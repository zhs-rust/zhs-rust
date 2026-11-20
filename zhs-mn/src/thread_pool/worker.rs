use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle as ThreadHandle};

use crate::error::ThreadPoolError;

use super::ThreadPoolConfig;
use super::ThreadPoolStats;
use super::task::{Job, JobId, JoinHandle, SharedJob};

static NEXT_POOL_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {

    static CURRENT_WORKER: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
}

pub(crate) fn is_worker_of(pool: u64) -> bool {
    CURRENT_WORKER.with(|worker| matches!(worker.get(), Some((id, _)) if id == pool))
}

struct PoolState {
    workers: usize,

    idle: usize,
    shutdown: bool,

    thread_handles: Vec<(u64, ThreadHandle<()>)>,
}

pub(crate) struct Inner {
    id: u64,
    config: ThreadPoolConfig,

    queue: Mutex<VecDeque<SharedJob>>,
    state: Mutex<PoolState>,

    work_available: Condvar,
    next_job_id: AtomicU64,
    next_worker_id: AtomicU64,

    completed_jobs: AtomicU64,
    handles: AtomicUsize,
}

impl Inner {
    pub(crate) fn new(config: ThreadPoolConfig) -> Self {
        Self {
            id: NEXT_POOL_ID.fetch_add(1, Ordering::Relaxed),
            config,
            queue: Mutex::new(VecDeque::new()),
            state: Mutex::new(PoolState {
                workers: 0,
                idle: 0,
                shutdown: false,
                thread_handles: Vec::new(),
            }),
            work_available: Condvar::new(),
            next_job_id: AtomicU64::new(1),
            next_worker_id: AtomicU64::new(1),
            completed_jobs: AtomicU64::new(0),
            handles: AtomicUsize::new(1),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn config(&self) -> ThreadPoolConfig {
        self.config
    }

    pub(crate) fn worker_count(&self) -> usize {
        lock(&self.state).workers
    }

    pub(crate) fn idle_worker_count(&self) -> usize {
        lock(&self.state).idle
    }

    pub(crate) fn queued_job_count(&self) -> usize {
        lock(&self.queue).len()
    }

    pub(crate) fn stats(&self) -> ThreadPoolStats {
        let (workers, idle_workers, shutdown) = {
            let state = lock(&self.state);
            (state.workers, state.idle, state.shutdown)
        };
        let queued_jobs = lock(&self.queue).len();

        ThreadPoolStats {
            workers,
            idle_workers,
            queued_jobs,
            running_jobs: workers.saturating_sub(idle_workers),
            completed_jobs: self.completed_jobs.load(Ordering::Acquire),
            shutdown,
        }
    }

    pub(crate) fn record_finished_job(&self) {
        self.completed_jobs.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        lock(&self.state).shutdown
    }

    pub(crate) fn retain_handle(&self) {
        self.handles.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn release_handle(&self) -> usize {
        self.handles.fetch_sub(1, Ordering::SeqCst) - 1
    }

    pub(crate) fn start_min_workers(self: &Arc<Self>) -> Result<(), ThreadPoolError> {
        if self.config.min_workers == 0 {
            return Ok(());
        }

        let queue = lock(&self.queue);
        let mut state = lock(&self.state);
        while state.workers < self.config.min_workers {
            self.spawn_worker(&mut state)?;
        }
        drop(state);
        drop(queue);
        Ok(())
    }

    pub(crate) fn submit<F, T>(self: &Arc<Self>, job: F) -> Result<JoinHandle<T>, ThreadPoolError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let id = JobId::new(self.next_job_id.fetch_add(1, Ordering::SeqCst));
        let task = Arc::new(Job::new(id, Box::new(job)));
        let handle = JoinHandle::new(Arc::clone(&task));
        let task: SharedJob = task;

        let mut queue = lock(&self.queue);
        let mut state = lock(&self.state);
        if state.shutdown {
            return Err(ThreadPoolError::Closed);
        }

        if state.workers < self.config.max_workers && queue.len() + 1 > state.idle {
            self.spawn_worker(&mut state)?;
        }
        drop(state);

        queue.push_back(task);
        drop(queue);

        self.work_available.notify_one();
        Ok(handle)
    }

    pub(crate) fn initiate_shutdown(&self) {
        let jobs = {
            let mut queue = lock(&self.queue);
            let mut state = lock(&self.state);
            if state.shutdown {
                return;
            }
            state.shutdown = true;
            queue.drain(..).collect::<Vec<_>>()
        };

        self.work_available.notify_all();
        for job in jobs {
            job.cancel();
        }
    }

    pub(crate) fn join_workers(&self) {
        let handles = {
            let mut state = lock(&self.state);
            std::mem::take(&mut state.thread_handles)
        };

        for (_, handle) in handles {
            let _ = handle.join();
        }

        let mut state = lock(&self.state);
        state.workers = 0;
        state.idle = 0;
    }

    fn spawn_worker(self: &Arc<Self>, state: &mut PoolState) -> Result<(), ThreadPoolError> {
        let worker = self.next_worker_id.fetch_add(1, Ordering::Relaxed);
        let inner = Arc::clone(self);
        state.workers += 1;

        let spawned = thread::Builder::new()
            .name(format!("zhs-blocking-{worker}"))
            .spawn(move || run(inner, worker));

        match spawned {
            Ok(handle) => {
                state.thread_handles.push((worker, handle));
                Ok(())
            }
            Err(error) => {
                state.workers -= 1;
                Err(ThreadPoolError::worker_spawn(state.workers, error))
            }
        }
    }

    fn next_job(&self, worker: u64) -> Option<SharedJob> {
        let mut queue = lock(&self.queue);

        loop {
            if lock(&self.state).shutdown {
                return None;
            }

            if let Some(job) = queue.pop_front() {
                if !queue.is_empty() {
                    self.work_available.notify_one();
                }
                return Some(job);
            }

            let mut state = lock(&self.state);
            if state.shutdown {
                return None;
            }
            state.idle += 1;
            drop(state);

            let (guard, timeout) = self
                .work_available
                .wait_timeout(queue, self.config.keep_alive)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue = guard;

            let mut state = lock(&self.state);
            state.idle -= 1;

            let retire = timeout.timed_out()
                && queue.is_empty()
                && !state.shutdown
                && state.workers > self.config.min_workers;
            if retire {
                state.workers -= 1;
                state.thread_handles.retain(|(id, _)| *id != worker);
            }
            drop(state);

            if retire {
                return None;
            }
        }
    }
}

pub(crate) fn run(inner: Arc<Inner>, worker: u64) {
    CURRENT_WORKER.with(|current| current.set(Some((inner.id, worker))));

    while let Some(job) = inner.next_job(worker) {
        job.run();
        inner.record_finished_job();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
