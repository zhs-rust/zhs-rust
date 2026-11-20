use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zhs_http::{HttpContext, Next, Response, StatusCode};

const MAX_ENTRIES: usize = 10_000;

struct Window {
    started: Instant,
    count: u64,
}

#[derive(Clone)]
pub struct RateLimiter {
    max_requests: u64,
    window: Duration,
    windows: Arc<Mutex<HashMap<String, Window>>>,
}

impl RateLimiter {
    pub fn new(max_requests: u64, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            windows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn per_second(max_requests: u64) -> Self {
        Self::new(max_requests, Duration::from_secs(1))
    }

    pub fn per_minute(max_requests: u64) -> Self {
        Self::new(max_requests, Duration::from_secs(60))
    }

    fn check(&self, key: &str) -> Result<(), u64> {
        let mut windows = self.windows.lock().unwrap_or_else(|poisoned| {
            eprintln!("rate limit mutex poisoned; recovering");
            poisoned.into_inner()
        });
        let now = Instant::now();

        if windows.len() >= MAX_ENTRIES {
            windows.retain(|_, window| now.duration_since(window.started) < self.window);
        }

        let window = windows.entry(key.to_owned()).or_insert(Window {
            started: now,
            count: 0,
        });

        if now.duration_since(window.started) >= self.window {
            window.started = now;
            window.count = 0;
        }

        if window.count >= self.max_requests {
            let elapsed = now.duration_since(window.started).as_secs();
            return Err(self.window.as_secs().saturating_sub(elapsed).max(1));
        }

        window.count += 1;

        if windows.len() > MAX_ENTRIES
            && let Some(oldest) = windows
                .iter()
                .min_by_key(|(_, window)| window.started)
                .map(|(key, _)| key.clone())
        {
            windows.remove(&oldest);
        }

        Ok(())
    }
}

pub fn apply(
    limiter: RateLimiter,
    context: HttpContext,
    next: Next,
) -> zhs_http::BoxFuture<Result<Response, zhs_http::HttpError>> {
    let key = context
        .request
        .peer_addr
        .map(|peer| peer.ip().to_string())
        .unwrap_or_else(|| "unknown".to_owned());

    match limiter.check(&key) {
        Ok(()) => next(context),
        Err(retry_after) => Box::pin(async move {
            Ok(Response::text("Too Many Requests")
                .with_status(StatusCode(429))
                .with_header("retry-after", retry_after.to_string()))
        }),
    }
}
