mod monitor;
mod report;
mod source;

use std::fmt;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::error::WatchdogError;

pub use report::{HealthState, SourceHealth, WatchdogReport};
pub use source::{HealthSource, SourceKind, SourceSample};

use monitor::Inner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchdogConfig {
    pub interval: Duration,

    pub stall_ticks: u32,

    pub degraded_queue: usize,

    pub history: usize,
}

impl WatchdogConfig {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            stall_ticks: 3,
            degraded_queue: 64,
            history: 120,
        }
    }

    pub fn with_stall_ticks(mut self, ticks: u32) -> Self {
        self.stall_ticks = ticks;
        self
    }

    pub fn with_degraded_queue(mut self, queued: usize) -> Self {
        self.degraded_queue = queued;
        self
    }

    pub fn with_history(mut self, history: usize) -> Self {
        self.history = history;
        self
    }
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self::new(Duration::from_secs(1))
    }
}

pub trait WatchdogObserver: Send + Sync {
    fn on_report(&self, report: &WatchdogReport);
}

impl<F> WatchdogObserver for F
where
    F: Fn(&WatchdogReport) + Send + Sync,
{
    fn on_report(&self, report: &WatchdogReport) {
        self(report);
    }
}

pub struct Watchdog {
    inner: Arc<Inner>,
}

impl Watchdog {
    pub fn new(config: WatchdogConfig) -> Result<Self, WatchdogError> {
        if config.interval.is_zero() {
            return Err(WatchdogError::ZeroInterval);
        }

        Ok(Self {
            inner: Inner::start(config)?,
        })
    }

    pub fn start_global(config: WatchdogConfig) -> Result<&'static Watchdog, WatchdogError> {
        if let Some(watchdog) = GLOBAL.get() {
            return Ok(watchdog);
        }

        let watchdog = Watchdog::new(config)?;
        if let Err(losing) = GLOBAL.set(watchdog) {
            drop(losing);
        }

        Ok(GLOBAL
            .get()
            .expect("the global watchdog was initialized above"))
    }

    pub fn global() -> Option<&'static Watchdog> {
        GLOBAL.get()
    }

    pub fn config(&self) -> WatchdogConfig {
        self.inner.config()
    }

    pub fn watch(
        &self,
        name: impl Into<String>,
        source: impl HealthSource + 'static,
    ) -> Result<(), WatchdogError> {
        self.inner.watch(name.into(), Arc::new(source))
    }

    pub fn observe(&self, observer: impl WatchdogObserver + 'static) -> Result<(), WatchdogError> {
        self.inner.observe(Arc::new(observer))
    }

    pub fn observe_fn<F>(&self, observer: F) -> Result<(), WatchdogError>
    where
        F: Fn(&WatchdogReport) + Send + Sync + 'static,
    {
        self.inner.observe(Arc::new(observer))
    }

    pub fn latest(&self) -> Option<Arc<WatchdogReport>> {
        self.inner.latest()
    }

    pub fn history(&self) -> Vec<Arc<WatchdogReport>> {
        self.inner.history()
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown()
    }

    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.inner.shutdown();
    }
}

impl fmt::Debug for Watchdog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Watchdog")
            .field("interval", &self.config().interval)
            .field("shutdown", &self.is_shutdown())
            .field(
                "latest_sequence",
                &self.latest().map(|report| report.sequence),
            )
            .finish()
    }
}

static GLOBAL: OnceLock<Watchdog> = OnceLock::new();
