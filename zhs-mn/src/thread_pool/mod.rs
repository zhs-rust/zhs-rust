mod task;
mod worker;

use std::fmt;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::error::ThreadPoolError;

pub use task::{JobId, JoinHandle};

use worker::Inner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadPoolConfig {
    pub min_workers: usize,

    pub max_workers: usize,

    pub keep_alive: Duration,
}

impl ThreadPoolConfig {
    pub fn new(min_workers: usize, max_workers: usize) -> Self {
        Self {
            min_workers,
            max_workers,
            keep_alive: Duration::from_secs(60),
        }
    }

    pub fn with_keep_alive(mut self, keep_alive: Duration) -> Self {
        self.keep_alive = keep_alive;
        self
    }
}

impl Default for ThreadPoolConfig {
    fn default() -> Self {
        Self::new(0, 512)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadPoolStats {
    pub workers: usize,

    pub idle_workers: usize,

    pub queued_jobs: usize,

    pub running_jobs: usize,

    pub completed_jobs: u64,

    pub shutdown: bool,
}

pub struct ThreadPool {
    inner: Arc<Inner>,
}

impl ThreadPool {
    pub fn new(max_workers: usize) -> Result<Self, ThreadPoolError> {
        Self::with_config(ThreadPoolConfig::new(0, max_workers))
    }

    pub fn with_config(config: ThreadPoolConfig) -> Result<Self, ThreadPoolError> {
        if config.max_workers == 0 {
            return Err(ThreadPoolError::ZeroWorkers);
        }
        if config.min_workers > config.max_workers {
            return Err(ThreadPoolError::MinGreaterThanMax {
                min: config.min_workers,
                max: config.max_workers,
            });
        }

        let inner = Arc::new(Inner::new(config));
        if let Err(error) = inner.start_min_workers() {
            inner.initiate_shutdown();
            inner.join_workers();
            return Err(error);
        }

        Ok(Self { inner })
    }

    pub fn with_available_parallelism() -> Result<Self, ThreadPoolError> {
        let workers = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1);
        Self::new(workers)
    }

    pub fn config(&self) -> ThreadPoolConfig {
        self.inner.config()
    }

    pub fn worker_count(&self) -> usize {
        self.inner.worker_count()
    }

    pub fn idle_worker_count(&self) -> usize {
        self.inner.idle_worker_count()
    }

    pub fn queued_job_count(&self) -> usize {
        self.inner.queued_job_count()
    }

    pub fn stats(&self) -> ThreadPoolStats {
        self.inner.stats()
    }

    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown()
    }

    pub fn spawn_blocking<F, T>(&self, job: F) -> JoinHandle<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.try_spawn_blocking(job)
            .unwrap_or_else(|error| panic!("failed to spawn blocking job: {error}"))
    }

    pub fn try_spawn_blocking<F, T>(&self, job: F) -> Result<JoinHandle<T>, ThreadPoolError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.inner.submit(job)
    }

    pub fn shutdown(&self) {
        self.inner.initiate_shutdown();
        if !worker::is_worker_of(self.inner.id()) {
            self.inner.join_workers();
        }
    }
}

impl Clone for ThreadPool {
    fn clone(&self) -> Self {
        self.inner.retain_handle();
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        if self.inner.release_handle() == 0 {
            self.inner.initiate_shutdown();

            if !worker::is_worker_of(self.inner.id()) {
                self.inner.join_workers();
            }
        }
    }
}

impl fmt::Debug for ThreadPool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ThreadPool")
            .field("workers", &self.worker_count())
            .field("idle", &self.idle_worker_count())
            .field("queued", &self.queued_job_count())
            .field("shutdown", &self.is_shutdown())
            .finish()
    }
}
