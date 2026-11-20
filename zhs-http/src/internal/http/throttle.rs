use std::sync::OnceLock;
use std::time::{Duration, Instant};

use zhs_mn::scheduler::Scheduler;

use super::body::{BodyChunkFuture, BodyStream};

fn timer() -> &'static Scheduler {
    static TIMER: OnceLock<Scheduler> = OnceLock::new();

    TIMER.get_or_init(|| Scheduler::new(1).expect("the throttle scheduler should start"))
}

pub struct Throttle<S> {
    inner: S,
    bytes_per_second: u64,
    started: Option<Instant>,
    sent: u64,
    scheduler: Scheduler,
}

impl<S> Throttle<S>
where
    S: BodyStream,
{
    pub fn new(inner: S, bytes_per_second: u64) -> Self {
        Self {
            inner,
            bytes_per_second: bytes_per_second.max(1),
            started: None,
            sent: 0,
            scheduler: timer().clone(),
        }
    }

    pub fn bytes_per_second(&self) -> u64 {
        self.bytes_per_second
    }

    fn delay(&mut self, bytes: usize, now: Instant) -> Option<Duration> {
        let started = *self.started.get_or_insert(now);
        self.sent += bytes as u64;

        let target = Duration::from_secs_f64(self.sent as f64 / self.bytes_per_second as f64);
        let elapsed = now.saturating_duration_since(started);

        (target > elapsed).then(|| target - elapsed)
    }
}

impl<S> BodyStream for Throttle<S>
where
    S: BodyStream,
{
    fn next_chunk(&mut self) -> BodyChunkFuture<'_> {
        Box::pin(async move {
            let chunk = self.inner.next_chunk().await;

            if let Some(Ok(bytes)) = &chunk
                && let Some(delay) = self.delay(bytes.len(), Instant::now())
            {
                self.scheduler.sleep(delay).await;
            }

            chunk
        })
    }
}

impl<S> std::fmt::Debug for Throttle<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Throttle")
            .field("bytes_per_second", &self.bytes_per_second)
            .field("sent", &self.sent)
            .finish_non_exhaustive()
    }
}
