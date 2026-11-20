use std::cell::Cell;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle as ThreadHandle};
use std::time::Instant;

use crate::error::WatchdogError;

use super::report::{HealthState, SourceHealth, WatchdogReport};
use super::source::{HealthSource, SourceKind, SourceSample};
use super::{WatchdogConfig, WatchdogObserver};

thread_local! {

    static IS_MONITOR: Cell<bool> = const { Cell::new(false) };
}

fn is_monitor_thread() -> bool {
    IS_MONITOR.with(Cell::get)
}

struct TrackedSource {
    id: u64,
    name: String,
    source: Arc<dyn HealthSource>,

    progress: u64,

    stalled_ticks: u32,
}

struct ReportStore {
    latest: Option<Arc<WatchdogReport>>,
    history: VecDeque<Arc<WatchdogReport>>,
}

struct Control {
    shutdown: bool,
    thread: Option<ThreadHandle<()>>,
}

pub(crate) struct Inner {
    config: WatchdogConfig,
    started: Instant,

    sources: Mutex<Vec<TrackedSource>>,
    observers: Mutex<Vec<Arc<dyn WatchdogObserver>>>,
    reports: Mutex<ReportStore>,
    control: Mutex<Control>,

    tick: Condvar,
    next_source_id: AtomicU64,
    sequence: AtomicU64,
}

impl Inner {
    pub(crate) fn start(config: WatchdogConfig) -> Result<Arc<Self>, WatchdogError> {
        let inner = Arc::new(Self {
            config,
            started: Instant::now(),
            sources: Mutex::new(Vec::new()),
            observers: Mutex::new(Vec::new()),
            reports: Mutex::new(ReportStore {
                latest: None,
                history: VecDeque::with_capacity(config.history),
            }),
            control: Mutex::new(Control {
                shutdown: false,
                thread: None,
            }),
            tick: Condvar::new(),
            next_source_id: AtomicU64::new(1),
            sequence: AtomicU64::new(0),
        });

        let monitor = Arc::clone(&inner);
        let handle = thread::Builder::new()
            .name("zhs-watchdog".to_owned())
            .spawn(move || run(monitor))
            .map_err(WatchdogError::thread_spawn)?;

        lock(&inner.control).thread = Some(handle);
        Ok(inner)
    }

    pub(crate) fn config(&self) -> WatchdogConfig {
        self.config
    }

    pub(crate) fn watch(
        &self,
        name: String,
        source: Arc<dyn HealthSource>,
    ) -> Result<(), WatchdogError> {
        let mut sources = lock(&self.sources);
        if self.is_shutdown() {
            return Err(WatchdogError::Closed);
        }

        let id = self.next_source_id.fetch_add(1, Ordering::Relaxed);
        sources.push(TrackedSource {
            id,
            name,
            source,
            progress: 0,
            stalled_ticks: 0,
        });
        Ok(())
    }

    pub(crate) fn observe(&self, observer: Arc<dyn WatchdogObserver>) -> Result<(), WatchdogError> {
        let mut observers = lock(&self.observers);
        if self.is_shutdown() {
            return Err(WatchdogError::Closed);
        }

        observers.push(observer);
        Ok(())
    }

    pub(crate) fn latest(&self) -> Option<Arc<WatchdogReport>> {
        lock(&self.reports).latest.clone()
    }

    pub(crate) fn history(&self) -> Vec<Arc<WatchdogReport>> {
        lock(&self.reports)
            .history
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        lock(&self.control).shutdown
    }

    pub(crate) fn shutdown(&self) {
        let handle = {
            let mut control = lock(&self.control);
            control.shutdown = true;
            self.tick.notify_all();

            if is_monitor_thread() {
                None
            } else {
                control.thread.take()
            }
        };

        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    fn sample(&self) -> Arc<WatchdogReport> {
        let tracked = {
            let sources = lock(&self.sources);
            sources
                .iter()
                .map(|source| TrackedSource {
                    id: source.id,
                    name: source.name.clone(),
                    source: Arc::clone(&source.source),
                    progress: source.progress,
                    stalled_ticks: source.stalled_ticks,
                })
                .collect::<Vec<_>>()
        };

        let mut health = Vec::with_capacity(tracked.len());
        let mut updates = Vec::with_capacity(tracked.len());

        for tracked in tracked {
            let kind = catch_unwind(AssertUnwindSafe(|| tracked.source.kind()))
                .unwrap_or(SourceKind::Custom);
            let sampled = catch_unwind(AssertUnwindSafe(|| tracked.source.sample()));

            let (sample, stalled_ticks, state) = match sampled {
                Ok(sample) => {
                    let (stalled_ticks, state) =
                        self.evaluate(tracked.progress, tracked.stalled_ticks, &sample);
                    (sample, stalled_ticks, state)
                }
                Err(_) => (
                    SourceSample::default(),
                    tracked.stalled_ticks.saturating_add(1),
                    HealthState::Stalled,
                ),
            };

            updates.push((tracked.id, sample.progress, stalled_ticks));
            health.push(SourceHealth {
                name: tracked.name,
                kind,
                state,
                stalled_ticks,
                workers: sample.workers,
                idle_workers: sample.idle_workers,
                queued: sample.queued,
                running: sample.running,
                pending: sample.pending,
                capacity: sample.capacity,
                progress: sample.progress,
                shutdown: sample.shutdown,
            });
        }

        if !updates.is_empty() {
            let mut sources = lock(&self.sources);
            for (id, progress, stalled_ticks) in updates {
                if let Some(source) = sources.iter_mut().find(|source| source.id == id) {
                    source.progress = progress;
                    source.stalled_ticks = stalled_ticks;
                }
            }
        }

        // Shutdown has severity 0, so it never wins the severity comparison;
        // report it only when every source is shut down, otherwise a fully
        // stopped system would be reported as healthy.
        let mut state = if !health.is_empty()
            && health
                .iter()
                .all(|source| source.state == HealthState::Shutdown)
        {
            HealthState::Shutdown
        } else {
            HealthState::Healthy
        };
        for source in &health {
            if source.state.severity() > state.severity() {
                state = source.state;
            }
        }

        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        Arc::new(WatchdogReport {
            sequence,
            elapsed: self.started.elapsed(),
            state,
            sources: health,
        })
    }

    fn evaluate(
        &self,
        previous_progress: u64,
        previous_stalled: u32,
        sample: &SourceSample,
    ) -> (u32, HealthState) {
        if sample.shutdown {
            return (0, HealthState::Shutdown);
        }

        let stalled_ticks = if sample.progress_expected && sample.progress == previous_progress {
            previous_stalled.saturating_add(1)
        } else {
            0
        };

        let state = if sample.progress_expected && stalled_ticks >= self.config.stall_ticks {
            HealthState::Stalled
        } else if self.config.degraded_queue > 0 && sample.queued >= self.config.degraded_queue {
            HealthState::Degraded
        } else {
            HealthState::Healthy
        };

        (stalled_ticks, state)
    }

    fn publish(&self, report: Arc<WatchdogReport>) {
        {
            let mut reports = lock(&self.reports);
            reports.latest = Some(Arc::clone(&report));
            if self.config.history > 0 {
                reports.history.push_back(Arc::clone(&report));
                while reports.history.len() > self.config.history {
                    reports.history.pop_front();
                }
            }
        }

        let observers = {
            let observers = lock(&self.observers);
            observers.clone()
        };

        for observer in observers {
            let _ = catch_unwind(AssertUnwindSafe(|| observer.on_report(&report)));
        }
    }

    fn wait_next_tick(&self) -> bool {
        let control = lock(&self.control);
        if control.shutdown {
            return true;
        }

        let (control, _) = self
            .tick
            .wait_timeout(control, self.config.interval)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        control.shutdown
    }
}

fn run(inner: Arc<Inner>) {
    IS_MONITOR.with(|flag| flag.set(true));

    loop {
        if inner.is_shutdown() {
            return;
        }

        let report = inner.sample();
        inner.publish(report);

        if inner.wait_next_tick() {
            return;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
