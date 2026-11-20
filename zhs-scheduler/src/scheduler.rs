use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

use crate::{Job, JobContext, JobKey, SchedulerError, Trigger, job_fn};

/// Default upper bound on the number of job threads running concurrently.
const DEFAULT_MAX_CONCURRENT_JOBS: usize = 32;

/// Safety cap for the misfire coalescing loop so a pathological trigger
/// cannot spin forever.
const MAX_CATCHUP_ITERATIONS: usize = 10_000;

struct Entry {
    key: JobKey,
    job: Arc<dyn Job>,
    trigger: Trigger,
    next: SystemTime,
    fire_count: u64,
}

struct State {
    entries: Vec<Entry>,
    running: bool,
    shutdown: bool,
    skipped: u64,
}

struct Inner {
    state: Mutex<State>,
    wake: Condvar,
    worker: Mutex<Option<JoinHandle<()>>>,
    handles: AtomicUsize,
    running_jobs: Arc<AtomicUsize>,
    max_concurrent_jobs: AtomicUsize,
}

/// Decrements the running-job counter when a spawned job thread finishes.
struct RunningGuard(Arc<AtomicUsize>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Scheduler {
    inner: Arc<Inner>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    entries: Vec::new(),
                    running: false,
                    shutdown: false,
                    skipped: 0,
                }),
                wake: Condvar::new(),
                worker: Mutex::new(None),
                handles: AtomicUsize::new(1),
                running_jobs: Arc::new(AtomicUsize::new(0)),
                max_concurrent_jobs: AtomicUsize::new(DEFAULT_MAX_CONCURRENT_JOBS),
            }),
        }
    }

    /// Sets the maximum number of job executions allowed to run concurrently.
    /// Fires that would exceed the limit are skipped (counted in
    /// [`Scheduler::skipped_jobs`]) and the schedule advances as usual.
    /// A value of `0` means no limit is enforced.
    pub fn set_max_concurrent_jobs(&self, max: usize) {
        self.inner
            .max_concurrent_jobs
            .store(max, Ordering::SeqCst);
    }

    /// Returns the current maximum number of concurrent job executions.
    pub fn max_concurrent_jobs(&self) -> usize {
        self.inner.max_concurrent_jobs.load(Ordering::SeqCst)
    }

    /// Returns how many fires were skipped because the concurrency limit
    /// was reached.
    pub fn skipped_jobs(&self) -> u64 {
        match self.inner.state.lock() {
            Ok(state) => state.skipped,
            Err(poisoned) => poisoned.into_inner().skipped,
        }
    }

    pub fn start(&self) -> Result<(), SchedulerError> {
        let mut worker = self
            .inner
            .worker
            .lock()
            .map_err(|_| SchedulerError::Closed)?;

        if worker.is_some() {
            return Ok(());
        }

        {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| SchedulerError::Closed)?;
            state.running = true;
            state.shutdown = false;
        }

        let inner = Arc::clone(&self.inner);
        let handle = match thread::Builder::new()
            .name("zhs-scheduler".to_owned())
            .spawn(move || run(inner))
        {
            Ok(handle) => handle,
            Err(error) => {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .map_err(|_| SchedulerError::Closed)?;
                state.running = false;
                return Err(SchedulerError::Job(error.to_string()));
            }
        };

        *worker = Some(handle);

        Ok(())
    }

    pub fn schedule(
        &self,
        name: impl Into<String>,
        job: Arc<dyn Job>,
        trigger: impl Into<Trigger>,
    ) -> Result<JobKey, SchedulerError> {
        let key = JobKey::new(name);
        let trigger = trigger.into();
        let now = SystemTime::now();
        let next = trigger.next_after(now, 0).ok_or_else(|| {
            SchedulerError::InvalidTrigger("trigger has no next fire time".into())
        })?;

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| SchedulerError::Closed)?;
        if state.shutdown {
            return Err(SchedulerError::Closed);
        }

        state.entries.push(Entry {
            key: key.clone(),
            job,
            trigger,
            next,
            fire_count: 0,
        });
        drop(state);

        self.inner.wake.notify_all();
        Ok(key)
    }

    pub fn schedule_fn<F>(
        &self,
        name: impl Into<String>,
        job: F,
        trigger: impl Into<Trigger>,
    ) -> Result<JobKey, SchedulerError>
    where
        F: Fn(&JobContext) -> Result<(), SchedulerError> + Send + Sync + 'static,
    {
        self.schedule(name, job_fn(job), trigger)
    }

    pub fn is_running(&self) -> bool {
        self.inner
            .state
            .lock()
            .map(|state| state.running)
            .unwrap_or(false)
    }

    pub fn shutdown(&self) {
        {
            let mut state = match self.inner.state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            state.shutdown = true;
            state.running = false;
        }

        self.inner.wake.notify_all();

        let worker = self
            .inner
            .worker
            .lock()
            .ok()
            .and_then(|mut worker| worker.take());

        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Scheduler {
    fn clone(&self) -> Self {
        self.inner.handles.fetch_add(1, Ordering::SeqCst);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        if self.inner.handles.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.shutdown();
        }
    }
}

fn run(inner: Arc<Inner>) {
    loop {
        let mut state = match inner.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };

        if state.shutdown {
            state.running = false;
            return;
        }

        if state.entries.is_empty() {
            state = match inner.wake.wait(state) {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            continue;
        }

        let now = SystemTime::now();
        let (index, next) = state
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.next)
            .map(|(index, entry)| (index, entry.next))
            .expect("entries is not empty");

        if next > now {
            let wait = next.duration_since(now).unwrap_or_default();
            let (guard, _) = match inner.wake.wait_timeout(state, wait) {
                Ok(result) => result,
                Err(poisoned) => poisoned.into_inner(),
            };
            state = guard;
            continue;
        }

        let (key, job, scheduled_at, fire_count) = {
            let entry = &mut state.entries[index];
            entry.fire_count += 1;
            (
                entry.key.clone(),
                Arc::clone(&entry.job),
                entry.next,
                entry.fire_count,
            )
        };

        // Compute the next fire time, shielding the worker thread from a
        // panicking trigger implementation.
        let trigger_next = {
            let trigger = &state.entries[index].trigger;
            catch_unwind(AssertUnwindSafe(|| trigger.next_after(scheduled_at, fire_count)))
        };
        let trigger_next = match trigger_next {
            Ok(next) => coalesce_past_fire_times(&state.entries[index].trigger, next, fire_count),
            Err(_) => {
                eprintln!(
                    "zhs-scheduler: trigger for job '{}' panicked while computing the next fire time; removing the job from the schedule",
                    key.name
                );
                state.entries.remove(index);
                drop(state);
                continue;
            }
        };
        let remaining = state.entries[index].trigger.remaining_after(fire_count);

        match (trigger_next, remaining) {
            (Some(next), Some(0)) => {
                state.entries.remove(index);
                let _ = next;
            }
            (Some(next), _) => {
                state.entries[index].next = next;
            }
            (None, _) => {
                state.entries.remove(index);
            }
        }

        // Enforce the bounded concurrency limit. When the limit is reached
        // the fire is skipped (the schedule already advanced above).
        let max = inner.max_concurrent_jobs.load(Ordering::SeqCst);
        let previous = inner.running_jobs.fetch_add(1, Ordering::SeqCst);
        if max > 0 && previous >= max {
            inner.running_jobs.fetch_sub(1, Ordering::SeqCst);
            state.skipped += 1;
            eprintln!(
                "zhs-scheduler: skipping fire of job '{}' (concurrency limit of {max} running jobs reached)",
                key.name
            );
            continue;
        }
        let guard = RunningGuard(Arc::clone(&inner.running_jobs));

        drop(state);

        thread::spawn(move || {
            let _guard = guard;
            let context = JobContext {
                key,
                scheduled_at,
                fire_time: SystemTime::now(),
            };
            if catch_unwind(AssertUnwindSafe(|| job.execute(&context))).is_err() {
                eprintln!(
                    "zhs-scheduler: job '{}' panicked during execution; continuing",
                    context.key.name
                );
            }
        });
    }
}

/// Coalesces misfired (past) fire times into a single upcoming one: if the
/// computed next fire time is still in the past, advance it from the previous
/// candidate until it lands in the future or the safety iteration cap is hit.
/// The job fires only once for all coalesced misfires.
fn coalesce_past_fire_times(
    trigger: &Trigger,
    next: Option<SystemTime>,
    fire_count: u64,
) -> Option<SystemTime> {
    let mut next = next?;
    let mut iterations = 0;
    while next <= SystemTime::now() && iterations < MAX_CATCHUP_ITERATIONS {
        let candidate = next;
        match catch_unwind(AssertUnwindSafe(|| trigger.next_after(candidate, fire_count))) {
            Ok(Some(further)) if further > candidate => next = further,
            _ => break,
        }
        iterations += 1;
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;
    use crate::SimpleTrigger;
    use std::time::Duration;

    #[test]
    fn runs_a_repeating_job() {
        let scheduler = Scheduler::new();
        let count = Arc::new(AtomicUsize::new(0));
        let captured = Arc::clone(&count);

        scheduler
            .schedule_fn(
                "tick",
                move |_| {
                    captured.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                SimpleTrigger::every(Duration::from_millis(10)).repeat(2),
            )
            .unwrap();

        scheduler.start().unwrap();
        thread::sleep(Duration::from_millis(80));
        scheduler.shutdown();

        assert!(count.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn default_concurrency_limit_is_32() {
        let scheduler = Scheduler::new();
        assert_eq!(scheduler.max_concurrent_jobs(), 32);
        scheduler.set_max_concurrent_jobs(4);
        assert_eq!(scheduler.max_concurrent_jobs(), 4);
    }

    #[test]
    fn panicking_job_does_not_stop_the_scheduler() {
        let scheduler = Scheduler::new();
        let panicked = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));

        let captured_panic = Arc::clone(&panicked);
        scheduler
            .schedule_fn(
                "boom",
                move |_| {
                    if !captured_panic.swap(true, Ordering::SeqCst) {
                        panic!("job panic");
                    }
                    Ok(())
                },
                SimpleTrigger::every(Duration::from_millis(10)),
            )
            .unwrap();

        let captured_count = Arc::clone(&count);
        scheduler
            .schedule_fn(
                "steady",
                move |_| {
                    captured_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                SimpleTrigger::every(Duration::from_millis(10)),
            )
            .unwrap();

        scheduler.start().unwrap();
        thread::sleep(Duration::from_millis(120));
        let running = scheduler.is_running();
        scheduler.shutdown();

        assert!(panicked.load(Ordering::SeqCst), "job should have panicked");
        assert!(running, "scheduler worker must survive a panicking job");
        assert!(count.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn concurrency_limit_skips_excess_fires() {
        let scheduler = Scheduler::new();
        scheduler.set_max_concurrent_jobs(1);
        let count = Arc::new(AtomicUsize::new(0));
        let captured = Arc::clone(&count);

        scheduler
            .schedule_fn(
                "slow",
                move |_| {
                    captured.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(300));
                    Ok(())
                },
                SimpleTrigger::every(Duration::from_millis(10)),
            )
            .unwrap();

        scheduler.start().unwrap();
        thread::sleep(Duration::from_millis(120));
        let skipped = scheduler.skipped_jobs();
        let started = count.load(Ordering::SeqCst);
        scheduler.shutdown();

        assert_eq!(started, 1, "only one job thread should run concurrently");
        assert!(skipped >= 1, "excess fires should be skipped, got {skipped}");
    }

    #[test]
    fn coalesces_past_fire_times_into_one_future_time() {
        let trigger = Trigger::from(SimpleTrigger::every(Duration::from_millis(20)));
        let before = SystemTime::now();
        let past = before - Duration::from_secs(1);

        let next = coalesce_past_fire_times(&trigger, Some(past), 0)
            .expect("coalescing should yield a next fire time");

        // The loop only stops once the candidate is past its own `now` sample,
        // which is never earlier than `before`.
        assert!(next > before);
        // A single coalesced jump, not a replay of every missed fire.
        assert!(next <= before + Duration::from_millis(500));
    }

    #[test]
    fn coalescing_passes_through_future_fire_times() {
        let trigger = Trigger::from(SimpleTrigger::every(Duration::from_secs(60)));
        let future = SystemTime::now() + Duration::from_secs(30);

        assert_eq!(coalesce_past_fire_times(&trigger, Some(future), 0), Some(future));
        assert_eq!(coalesce_past_fire_times(&trigger, None, 0), None);
    }
}
