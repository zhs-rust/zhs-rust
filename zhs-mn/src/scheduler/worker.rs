use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle as ThreadHandle, Thread};

use crate::error::SchedulerError;

use super::SchedulerStats;
use super::task::{JoinHandle, Runnable, SharedTask, Task, TaskId};

static NEXT_SCHEDULER_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {

    static CURRENT_WORKER: Cell<Option<(u64, usize)>> = const { Cell::new(None) };
}

pub(crate) fn is_worker_thread() -> bool {
    CURRENT_WORKER.with(|worker| worker.get().is_some())
}

pub(crate) fn is_worker_of(scheduler: u64) -> bool {
    CURRENT_WORKER.with(|worker| matches!(worker.get(), Some((id, _)) if id == scheduler))
}

fn current_worker_of(scheduler: u64) -> Option<usize> {
    CURRENT_WORKER.with(|worker| match worker.get() {
        Some((id, worker)) if id == scheduler => Some(worker),
        _ => None,
    })
}

pub(crate) struct Inner {
    id: u64,
    worker_count: usize,
    queues: Vec<Mutex<VecDeque<SharedTask>>>,

    injected: Mutex<VecDeque<SharedTask>>,

    tasks: Mutex<HashMap<TaskId, SharedTask>>,

    slots: Vec<WorkerSlot>,
    next_slot: AtomicUsize,
    shutdown: AtomicBool,
    next_id: AtomicU64,

    polls: AtomicU64,

    busy: AtomicUsize,

    runnable: AtomicUsize,
    worker_handles: Mutex<Vec<ThreadHandle<()>>>,
    handles: AtomicUsize,
}

struct WorkerSlot {
    thread: Mutex<Option<Thread>>,

    parked: AtomicBool,
}

impl Inner {
    pub(crate) fn new(worker_count: usize) -> Self {
        let queues = (0..worker_count)
            .map(|_| Mutex::new(VecDeque::new()))
            .collect();
        let slots = (0..worker_count)
            .map(|_| WorkerSlot {
                thread: Mutex::new(None),
                parked: AtomicBool::new(false),
            })
            .collect();
        Self {
            id: NEXT_SCHEDULER_ID.fetch_add(1, Ordering::Relaxed),
            worker_count,
            queues,
            injected: Mutex::new(VecDeque::new()),
            tasks: Mutex::new(HashMap::new()),
            slots,
            next_slot: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            polls: AtomicU64::new(0),
            busy: AtomicUsize::new(0),
            runnable: AtomicUsize::new(0),
            worker_handles: Mutex::new(Vec::with_capacity(worker_count)),
            handles: AtomicUsize::new(1),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn worker_count(&self) -> usize {
        self.worker_count
    }

    pub(crate) fn stats(&self) -> SchedulerStats {
        SchedulerStats {
            workers: self.worker_count,
            busy_workers: self.busy.load(Ordering::Acquire),
            runnable_tasks: self.runnable.load(Ordering::Acquire),
            pending_tasks: lock(&self.tasks).len(),
            polls: self.polls.load(Ordering::Acquire),
            shutdown: self.shutdown.load(Ordering::Acquire),
        }
    }

    pub(crate) fn begin_poll(&self) {
        self.busy.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn end_poll(&self) {
        self.busy.fetch_sub(1, Ordering::SeqCst);
        self.polls.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn set_worker_handles(&self, handles: Vec<ThreadHandle<()>>) {
        *lock(&self.worker_handles) = handles;
    }

    pub(crate) fn retain_handle(&self) {
        self.handles.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn release_handle(&self) -> usize {
        self.handles.fetch_sub(1, Ordering::SeqCst) - 1
    }

    pub(crate) fn spawn<F, T>(self: &Arc<Self>, future: F) -> Result<JoinHandle<T>, SchedulerError>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let id = TaskId::new(self.next_id.fetch_add(1, Ordering::SeqCst));
        let task = Arc::new(Task::new(id, Box::pin(future)));
        let handle = JoinHandle::new(Arc::clone(&task), self);
        let task: SharedTask = task;

        let mut registry = lock(&self.tasks);
        if self.shutdown.load(Ordering::Acquire) {
            return Err(SchedulerError::Closed);
        }
        registry.insert(id, Arc::clone(&task));

        match current_worker_of(self.id) {
            Some(worker) => {
                lock(&self.queues[worker]).push_back(task);
            }
            None => {
                lock(&self.injected).push_back(task);
            }
        }
        self.runnable.fetch_add(1, Ordering::SeqCst);
        drop(registry);
        self.notify_one();

        Ok(handle)
    }

    pub(crate) fn enqueue_wake(&self, task: SharedTask) {
        if self.shutdown.load(Ordering::Acquire) {
            task.cancel();
            return;
        }

        lock(&self.injected).push_back(task);
        self.runnable.fetch_add(1, Ordering::SeqCst);
        self.notify_one();
    }

    pub(crate) fn enqueue_local(&self, task: SharedTask, worker: usize) {
        if self.shutdown.load(Ordering::Acquire) {
            task.cancel();
            return;
        }

        lock(&self.queues[worker % self.worker_count]).push_back(task);
        self.runnable.fetch_add(1, Ordering::SeqCst);
        self.notify_one();
    }

    pub(crate) fn unregister(&self, id: TaskId) {
        lock(&self.tasks).remove(&id);
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    pub(crate) fn initiate_shutdown(&self) {
        let tasks = {
            let mut registry = lock(&self.tasks);
            if self.shutdown.swap(true, Ordering::AcqRel) {
                return;
            }
            registry.drain().map(|(_, task)| task).collect::<Vec<_>>()
        };

        for queue in &self.queues {
            lock(queue).clear();
        }
        lock(&self.injected).clear();
        self.runnable.store(0, Ordering::SeqCst);

        self.notify_all();
        for task in tasks {
            task.cancel();
        }
    }

    pub(crate) fn join_workers(&self) {
        let handles = {
            let mut handles = lock(&self.worker_handles);
            std::mem::take(&mut *handles)
        };

        for handle in handles {
            let _ = handle.join();
        }
    }

    fn next_task(&self, worker: usize, prefer_injected: bool) -> Option<SharedTask> {
        loop {
            if self.is_shutdown() {
                return None;
            }

            if let Some(task) = self.try_take(worker, prefer_injected) {
                if self.has_work() {
                    self.notify_one();
                }
                return Some(task);
            }

            let slot = &self.slots[worker];

            slot.parked.store(true, Ordering::SeqCst);
            if self.is_shutdown() {
                slot.parked.store(false, Ordering::SeqCst);
                return None;
            }
            if self.has_work() {
                slot.parked.store(false, Ordering::SeqCst);
                continue;
            }

            thread::park();
            slot.parked.store(false, Ordering::SeqCst);
        }
    }

    fn try_take(&self, worker: usize, prefer_injected: bool) -> Option<SharedTask> {
        if prefer_injected {
            if let Some(task) = self.pop_injected() {
                return Some(task);
            }
            if let Some(task) = self.pop_local(worker) {
                return Some(task);
            }
        } else {
            if let Some(task) = self.pop_local(worker) {
                return Some(task);
            }
            if let Some(task) = self.pop_injected() {
                return Some(task);
            }
        }

        for offset in 1..self.queues.len() {
            let victim = (worker + offset) % self.queues.len();
            if let Some(task) = self.pop_steal(victim) {
                return Some(task);
            }
        }

        None
    }

    fn pop_injected(&self) -> Option<SharedTask> {
        let task = lock(&self.injected).pop_front();
        if task.is_some() {
            self.runnable.fetch_sub(1, Ordering::SeqCst);
        }
        task
    }

    fn pop_local(&self, worker: usize) -> Option<SharedTask> {
        let task = lock(&self.queues[worker]).pop_back();
        if task.is_some() {
            self.runnable.fetch_sub(1, Ordering::SeqCst);
        }
        task
    }

    fn pop_steal(&self, victim: usize) -> Option<SharedTask> {
        let task = lock(&self.queues[victim]).pop_front();
        if task.is_some() {
            self.runnable.fetch_sub(1, Ordering::SeqCst);
        }
        task
    }

    fn has_work(&self) -> bool {
        self.runnable.load(Ordering::SeqCst) > 0
    }

    fn notify_one(&self) {
        let count = self.slots.len();
        if count == 0 {
            return;
        }

        let start = self.next_slot.fetch_add(1, Ordering::Relaxed) % count;
        for offset in 0..count {
            let slot = &self.slots[(start + offset) % count];
            if slot.parked.swap(false, Ordering::SeqCst) {
                if let Some(thread) = lock(&slot.thread).as_ref() {
                    thread.unpark();
                }
                return;
            }
        }
    }

    fn notify_all(&self) {
        for slot in &self.slots {
            if slot.parked.swap(false, Ordering::SeqCst)
                && let Some(thread) = lock(&slot.thread).as_ref()
            {
                thread.unpark();
            }
        }
    }
}

pub(crate) fn run(inner: Arc<Inner>, worker: usize) {
    CURRENT_WORKER.with(|current| current.set(Some((inner.id, worker))));
    *lock(&inner.slots[worker].thread) = Some(thread::current());

    let mut prefer_injected = true;
    while let Some(task) = inner.next_task(worker, prefer_injected) {
        prefer_injected = !prefer_injected;
        inner.begin_poll();
        Runnable::run(task, &inner, worker);
        inner.end_poll();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
