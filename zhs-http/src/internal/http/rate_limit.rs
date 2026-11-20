use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use zhs_log::runtime::{LogLevel, LogRecord, Logger};

use super::auth::{normalise_prefix, path_matches};
use super::context::Context;
use super::pipeline::{Middleware, Next, problem_response};
use super::service::ServiceFuture;
use super::status::StatusCode;

pub const DEFAULT_MAX_ENTRIES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quota {
    rate: f64,
    burst: u32,
}

impl Quota {
    pub fn new(rate: f64, burst: u32) -> Self {
        assert!(
            rate.is_finite() && rate > 0.0,
            "rate limit must be a positive finite number of tokens per second"
        );
        assert!(burst >= 1, "rate limit burst must be at least 1");

        Self { rate, burst }
    }

    pub fn per_second(tokens: u32) -> Self {
        assert!(tokens >= 1, "rate limit must allow at least 1 per second");
        Self {
            rate: f64::from(tokens),
            burst: tokens,
        }
    }

    pub fn per_minute(tokens: u32) -> Self {
        assert!(tokens >= 1, "rate limit must allow at least 1 per minute");
        Self {
            rate: f64::from(tokens) / 60.0,
            burst: tokens,
        }
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    pub fn burst(&self) -> u32 {
        self.burst
    }
}

#[derive(Clone)]
enum KeySource {
    Peer,

    Forwarded(String),

    Custom(Arc<dyn Fn(&Context) -> String + Send + Sync>),
}

#[derive(Clone, Debug)]
struct Rule {
    prefix: String,
    quota: Option<Quota>,
}

#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Clone)]
pub struct RateLimit {
    rules: Vec<Rule>,
    key: KeySource,
    buckets: Arc<Mutex<HashMap<(usize, String), Bucket>>>,
    max_entries: usize,
    logger: Option<Logger>,
}

impl RateLimit {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            key: KeySource::Peer,
            buckets: Arc::new(Mutex::new(HashMap::new())),
            max_entries: DEFAULT_MAX_ENTRIES,
            logger: None,
        }
    }

    pub fn rule(mut self, prefix: impl AsRef<str>, quota: Quota) -> Self {
        self.set_rule(prefix.as_ref(), Some(quota));
        self
    }

    pub fn exempt(mut self, prefix: impl AsRef<str>) -> Self {
        self.set_rule(prefix.as_ref(), None);
        self
    }

    pub fn with_key<F>(mut self, key: F) -> Self
    where
        F: Fn(&Context) -> String + Send + Sync + 'static,
    {
        self.key = KeySource::Custom(Arc::new(key));
        self
    }

    pub fn trust_forwarded_for(mut self, header: impl Into<String>) -> Self {
        self.key = KeySource::Forwarded(header.into());
        self
    }

    pub fn with_max_entries(mut self, max: usize) -> Self {
        assert!(max >= 1, "rate limit must allow at least one bucket");
        self.max_entries = max;
        self
    }

    pub fn with_logger(mut self, logger: Logger) -> Self {
        self.logger = Some(logger);
        self
    }

    fn set_rule(&mut self, prefix: &str, quota: Option<Quota>) {
        assert!(
            prefix.starts_with('/'),
            "rate limit prefix must start with '/': {prefix}"
        );

        let prefix = normalise_prefix(prefix);
        match self.rules.iter_mut().find(|rule| rule.prefix == prefix) {
            Some(rule) => rule.quota = quota,
            None => self.rules.push(Rule { prefix, quota }),
        }
    }

    fn quota_for(&self, path: &str) -> Option<(usize, Quota)> {
        let mut matched: Option<usize> = None;

        for (index, rule) in self.rules.iter().enumerate() {
            if !path_matches(path, &rule.prefix) {
                continue;
            }

            let longer = matched
                .map(|current| rule.prefix.len() > self.rules[current].prefix.len())
                .unwrap_or(true);
            if longer {
                matched = Some(index);
            }
        }

        let index = matched?;
        self.rules[index].quota.map(|quota| (index, quota))
    }

    fn key(&self, context: &Context) -> String {
        let key = match &self.key {
            KeySource::Peer => None,
            KeySource::Forwarded(header) => forwarded_value(context, header),
            KeySource::Custom(select) => Some(select(context)),
        };

        let key = key.unwrap_or_else(|| peer_key(context));
        if key.is_empty() {
            "unknown".to_owned()
        } else {
            key
        }
    }

    fn acquire(&self, slot: usize, key: &str, quota: Quota, now: Instant) -> Result<(), u64> {
        let bucket_key = (slot, key.to_owned());
        let mut buckets = self.lock();

        if buckets.len() >= self.max_entries && !buckets.contains_key(&bucket_key) {
            evict_oldest(&mut buckets);
        }

        let bucket = buckets.entry(bucket_key).or_insert(Bucket {
            tokens: f64::from(quota.burst()),
            last: now,
        });

        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * quota.rate()).min(f64::from(quota.burst()));
        bucket.last = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let seconds = ((1.0 - bucket.tokens) / quota.rate()).ceil().min(86_400.0) as u64;
            Err(seconds.max(1))
        }
    }

    fn audit(&self, context: &Context, key: &str, quota: Quota, retry_after: u64) {
        let logger = match &self.logger {
            Some(logger) => logger,
            None => match zhs_log::runtime::global() {
                Some(logger) => logger,
                None => return,
            },
        };
        if !logger.enabled(LogLevel::Warn) {
            return;
        }

        let record = LogRecord::new(
            LogLevel::Warn,
            "zhs_framework::http",
            "request rate limit exceeded",
        )
        .with_field("event", "rate_limited")
        .with_field("key", key.to_owned())
        .with_field("path", context.path().to_owned())
        .with_field("retry_after_seconds", retry_after.to_string())
        .with_field("burst", quota.burst().to_string());

        let _ = logger.log(record);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(usize, String), Bucket>> {
        self.buckets
            .lock()
            .unwrap_or_else(crate::internal::recover_poison)
    }
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RateLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimit")
            .field("rules", &self.rules)
            .field("max_entries", &self.max_entries)
            .finish_non_exhaustive()
    }
}

impl Middleware for RateLimit {
    fn call(&self, context: Context, next: Next) -> ServiceFuture {
        let Some((slot, quota)) = self.quota_for(context.path()) else {
            return next.run(context);
        };

        let key = self.key(&context);
        match self.acquire(slot, &key, quota, Instant::now()) {
            Ok(()) => next.run(context),
            Err(retry_after) => {
                self.audit(&context, &key, quota, retry_after);
                let response =
                    problem_response(StatusCode::TOO_MANY_REQUESTS, Some("rate limit exceeded"))
                        .with_header("Retry-After", retry_after.to_string());
                Box::pin(async move { Ok(response) })
            }
        }
    }
}

fn peer_key(context: &Context) -> String {
    context
        .peer_addr()
        .map(|peer| peer.ip().to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn forwarded_value(context: &Context, header: &str) -> Option<String> {
    let value = context.headers().get(header)?;
    let first = value.split(',').next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_owned())
    }
}

fn evict_oldest(buckets: &mut HashMap<(usize, String), Bucket>) {
    let oldest = buckets
        .iter()
        .min_by_key(|(_, bucket)| bucket.last)
        .map(|(key, _)| (key.0, key.1.clone()));

    if let Some(key) = oldest {
        buckets.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Quota, RateLimit};

    fn limit(quota: Quota) -> RateLimit {
        RateLimit::new().rule("/", quota)
    }

    #[test]
    fn allows_burst_then_rejects_with_retry_after() {
        let limit = limit(Quota::new(1.0, 2));
        let now = Instant::now();

        assert_eq!(limit.acquire(0, "client", Quota::new(1.0, 2), now), Ok(()));
        assert_eq!(limit.acquire(0, "client", Quota::new(1.0, 2), now), Ok(()));
        assert_eq!(limit.acquire(0, "client", Quota::new(1.0, 2), now), Err(1));
    }

    #[test]
    fn refills_over_time() {
        let quota = Quota::new(1.0, 1);
        let limit = limit(quota);
        let now = Instant::now();

        assert_eq!(limit.acquire(0, "client", quota, now), Ok(()));
        assert_eq!(limit.acquire(0, "client", quota, now), Err(1));
        assert_eq!(
            limit.acquire(0, "client", quota, now + Duration::from_secs(1)),
            Ok(())
        );
    }

    #[test]
    fn buckets_are_separate_per_key_and_rule() {
        let quota = Quota::new(1.0, 1);
        let limit = limit(quota);
        let now = Instant::now();

        assert_eq!(limit.acquire(0, "a", quota, now), Ok(()));
        assert_eq!(limit.acquire(0, "b", quota, now), Ok(()));
        assert_eq!(limit.acquire(1, "a", quota, now), Ok(()));
        assert_eq!(limit.acquire(0, "a", quota, now), Err(1));
    }

    #[test]
    fn longest_prefix_wins_and_exemptions_apply() {
        let limit = RateLimit::new()
            .rule("/", Quota::per_second(100))
            .rule("/auth/login", Quota::per_minute(5))
            .exempt("/health");

        let (_, login_quota) = limit.quota_for("/auth/login").expect("login rule");
        assert_eq!(login_quota, Quota::per_minute(5));

        let (_, other_quota) = limit.quota_for("/auth/login-extra").expect("root rule");
        assert_eq!(other_quota, Quota::per_second(100));

        assert!(limit.quota_for("/health").is_none());
        assert!(limit.quota_for("/health/ready").is_none());

        assert!(RateLimit::new().quota_for("/anything").is_none());
    }

    #[test]
    fn evicts_when_the_bucket_limit_is_reached() {
        let limit = RateLimit::new()
            .rule("/", Quota::new(1.0, 1))
            .with_max_entries(2);
        let quota = Quota::new(1.0, 1);
        let now = Instant::now();

        assert_eq!(limit.acquire(0, "a", quota, now), Ok(()));
        assert_eq!(limit.acquire(0, "b", quota, now), Ok(()));
        assert_eq!(limit.acquire(0, "c", quota, now), Ok(()));
        assert!(limit.lock().len() <= 2);
    }
}
