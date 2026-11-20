mod task;
mod timer;
mod worker;

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::SchedulerError;

pub use task::{JoinHandle, TaskId};
pub use timer::{Sleep, Timeout};

use timer::Timer;
use worker::Inner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedulerStats {
    pub workers: usize,

    pub busy_workers: usize,

    pub runnable_tasks: usize,

    pub pending_tasks: usize,

    pub polls: u64,

    pub shutdown: bool,
}

pub struct Scheduler {
    inner: Arc<Inner>,
    timer: Arc<Timer>,
}

impl Scheduler {
    pub fn new(worker_count: usize) -> Result<Self, SchedulerError> {
        if worker_count == 0 {
            return Err(SchedulerError::ZeroWorkers);
        }

        let timer = Arc::new(Timer::start()?);
        let inner = Arc::new(Inner::new(worker_count));
        let mut handles = Vec::with_capacity(worker_count);

        for worker in 0..worker_count {
            let thread_worker = Arc::clone(&inner);
            let spawned = thread::Builder::new()
                .name(format!("zhs-worker-{worker}"))
                .spawn(move || worker::run(thread_worker, worker));

            match spawned {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    inner.initiate_shutdown();
                    timer.shutdown();
                    for handle in handles {
                        let _ = handle.join();
                    }
                    return Err(SchedulerError::worker_spawn(worker, error));
                }
            }
        }

        inner.set_worker_handles(handles);
        Ok(Self { inner, timer })
    }

    pub fn with_available_parallelism() -> Result<Self, SchedulerError> {
        let workers = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1);
        Self::new(workers)
    }

    pub fn worker_count(&self) -> usize {
        self.inner.worker_count()
    }

    pub fn stats(&self) -> SchedulerStats {
        self.inner.stats()
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown()
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.try_spawn(future)
            .unwrap_or_else(|error| panic!("failed to spawn task: {error}"))
    }

    pub fn try_spawn<F>(&self, future: F) -> Result<JoinHandle<F::Output>, SchedulerError>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.inner.spawn(future)
    }

    pub fn sleep(&self, duration: Duration) -> Sleep {
        let deadline = Instant::now().checked_add(duration);
        Sleep::new(Arc::clone(&self.timer), deadline)
    }

    pub fn sleep_until(&self, deadline: Instant) -> Sleep {
        Sleep::new(Arc::clone(&self.timer), Some(deadline))
    }

    pub fn timeout<F>(&self, duration: Duration, future: F) -> Timeout<F> {
        Timeout::new(future, self.sleep(duration))
    }

    pub fn timeout_at<F>(&self, deadline: Instant, future: F) -> Timeout<F> {
        Timeout::new(future, self.sleep_until(deadline))
    }

    pub fn shutdown(&self) {
        self.inner.initiate_shutdown();
        self.timer.shutdown();
        if !worker::is_worker_of(self.inner.id()) {
            self.inner.join_workers();
        }
    }
}

impl Clone for Scheduler {
    fn clone(&self) -> Self {
        self.inner.retain_handle();
        Self {
            inner: Arc::clone(&self.inner),
            timer: Arc::clone(&self.timer),
        }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        if self.inner.release_handle() == 0 {
            self.inner.initiate_shutdown();
            self.timer.shutdown();

            if !worker::is_worker_of(self.inner.id()) {
                self.inner.join_workers();
            }
        }
    }
}

impl fmt::Debug for Scheduler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Scheduler")
            .field("workers", &self.worker_count())
            .field("shutdown", &self.is_shutdown())
            .finish()
    }
}

pub fn yield_now() -> YieldNow {
    YieldNow { yielded: false }
}

#[derive(Debug)]
pub struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.yielded {
            Poll::Ready(())
        } else {
            this.yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
