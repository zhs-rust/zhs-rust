use std::sync::Arc;

use crate::scheduler::Scheduler;
use crate::thread_pool::ThreadPool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Scheduler,

    ThreadPool,

    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceSample {
    pub workers: usize,

    pub idle_workers: usize,

    pub queued: usize,

    pub running: usize,

    pub pending: usize,

    pub capacity: usize,

    pub progress: u64,

    pub shutdown: bool,

    pub progress_expected: bool,
}

pub trait HealthSource: Send + Sync {
    fn kind(&self) -> SourceKind {
        SourceKind::Custom
    }

    fn sample(&self) -> SourceSample;
}

impl<T: HealthSource + ?Sized> HealthSource for Arc<T> {
    fn kind(&self) -> SourceKind {
        (**self).kind()
    }

    fn sample(&self) -> SourceSample {
        (**self).sample()
    }
}

impl HealthSource for Scheduler {
    fn kind(&self) -> SourceKind {
        SourceKind::Scheduler
    }

    fn sample(&self) -> SourceSample {
        let stats = self.stats();

        SourceSample {
            workers: stats.workers,
            idle_workers: stats.workers.saturating_sub(stats.busy_workers),
            queued: stats.runnable_tasks,
            running: stats.busy_workers,
            pending: stats.pending_tasks,
            capacity: stats.workers,
            progress: stats.polls,
            shutdown: stats.shutdown,

            progress_expected: stats.busy_workers > 0 || stats.runnable_tasks > 0,
        }
    }
}

impl HealthSource for ThreadPool {
    fn kind(&self) -> SourceKind {
        SourceKind::ThreadPool
    }

    fn sample(&self) -> SourceSample {
        let stats = self.stats();
        let capacity = self.config().max_workers;

        SourceSample {
            workers: stats.workers,
            idle_workers: stats.idle_workers,
            queued: stats.queued_jobs,
            running: stats.running_jobs,
            pending: stats.queued_jobs.saturating_add(stats.running_jobs),
            capacity,
            progress: stats.completed_jobs,
            shutdown: stats.shutdown,

            progress_expected: stats.queued_jobs > 0 && stats.workers < capacity,
        }
    }
}
