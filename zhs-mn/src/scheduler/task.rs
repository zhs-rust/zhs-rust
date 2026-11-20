use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll, Wake, Waker};

use crate::error::JoinError;

use super::worker::{Inner, is_worker_thread};

type TaskFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub(crate) type SharedTask = Arc<dyn Runnable>;

pub(crate) trait Runnable: Send + Sync {
    fn cancel(&self);

    fn run(self: Arc<Self>, scheduler: &Arc<Inner>, worker: usize);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(u64);

impl TaskId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "task-{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskState {
    Scheduled,

    Running { notified: bool },

    Waiting,

    Completed,

    Cancelled,
}

pub(super) struct Task<T> {
    id: TaskId,
    state: Mutex<TaskState>,
    future: Mutex<Option<TaskFuture<T>>>,
    completion: Completion<T>,
}

impl<T> Task<T> {
    pub(super) fn new(id: TaskId, future: TaskFuture<T>) -> Self {
        Self {
            id,
            state: Mutex::new(TaskState::Scheduled),
            future: Mutex::new(Some(future)),
            completion: Completion::new(),
        }
    }

    fn mark_scheduled(&self) -> bool {
        let mut state = lock(&self.state);
        match *state {
            TaskState::Waiting => {
                *state = TaskState::Scheduled;
                true
            }
            TaskState::Running { .. } => {
                *state = TaskState::Running { notified: true };
                false
            }
            TaskState::Scheduled | TaskState::Completed | TaskState::Cancelled => false,
        }
    }

    fn cancel_task(&self) {
        let (running, future) = {
            let mut state = lock(&self.state);
            if matches!(*state, TaskState::Completed | TaskState::Cancelled) {
                return;
            }
            let running = matches!(*state, TaskState::Running { .. });
            *state = TaskState::Cancelled;

            (running, lock(&self.future).take())
        };

        drop_silently(future);
        if !running {
            // A running task is completing right now on a worker; its poll
            // delivers the final result (or the cancellation) itself.
            self.completion.complete(Err(JoinError::Cancelled));
        }
    }

    fn execute(self: Arc<Self>, scheduler: &Arc<Inner>, worker: usize)
    where
        T: Send + 'static,
    {
        if scheduler.is_shutdown() {
            self.cancel_task();
            scheduler.unregister(self.id);
            return;
        }

        {
            let mut state = lock(&self.state);
            match *state {
                TaskState::Scheduled => {
                    *state = TaskState::Running { notified: false };
                }
                TaskState::Cancelled => {
                    drop(state);
                    scheduler.unregister(self.id);
                    return;
                }
                _ => return,
            }
        }

        let mut future = match lock(&self.future).take() {
            Some(future) => future,
            None => {
                self.cancel_task();
                scheduler.unregister(self.id);
                return;
            }
        };

        let waker = Waker::from(Arc::new(TaskWaker::<T> {
            task: Arc::downgrade(&self),
            scheduler: Arc::downgrade(scheduler),
        }));
        let mut context = Context::from_waker(&waker);

        let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            future.as_mut().poll(&mut context)
        }));

        match polled {
            Ok(Poll::Ready(output)) => {
                {
                    let mut state = lock(&self.state);
                    if !matches!(*state, TaskState::Cancelled) {
                        *state = TaskState::Completed;
                    }
                }

                // Deliver the result even when the task was cancelled while
                // running: nobody else completed it, and the work is done.
                scheduler.unregister(self.id);
                self.completion.complete(Ok(output));

                drop_silently(future);
            }
            Ok(Poll::Pending) => {
                let mut future = Some(future);
                let (cancelled, requeue) = {
                    let mut state = lock(&self.state);
                    if matches!(*state, TaskState::Cancelled) {
                        (true, false)
                    } else {
                        *lock(&self.future) = future.take();

                        match *state {
                            TaskState::Running { notified: true } => {
                                *state = TaskState::Scheduled;
                                (false, true)
                            }
                            _ => {
                                *state = TaskState::Waiting;
                                (false, false)
                            }
                        }
                    }
                };

                if let Some(future) = future {
                    drop_silently(future);
                }

                if cancelled {
                    // Cancelled mid-poll: the task will never be polled
                    // again, so resolve the completion here.
                    self.completion.complete(Err(JoinError::Cancelled));
                } else if requeue {
                    let task: SharedTask = self;
                    scheduler.enqueue_local(task, worker);
                }
            }
            Err(payload) => {
                {
                    let mut state = lock(&self.state);
                    if !matches!(*state, TaskState::Cancelled) {
                        *state = TaskState::Completed;
                    }
                }

                scheduler.unregister(self.id);
                self.completion.complete(Err(JoinError::panicked(payload)));

                drop_silently(future);
            }
        }
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        let future = {
            let slot = match self.future.get_mut() {
                Ok(slot) => slot,
                Err(poisoned) => poisoned.into_inner(),
            };
            slot.take()
        };
        drop_silently(future);
    }
}

impl<T: Send + 'static> Runnable for Task<T> {
    fn cancel(&self) {
        self.cancel_task();
    }

    fn run(self: Arc<Self>, scheduler: &Arc<Inner>, worker: usize) {
        self.execute(scheduler, worker);
    }
}

struct TaskWaker<T> {
    task: Weak<Task<T>>,
    scheduler: Weak<Inner>,
}

impl<T: Send + 'static> Wake for TaskWaker<T> {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let (Some(task), Some(scheduler)) = (self.task.upgrade(), self.scheduler.upgrade()) else {
            return;
        };

        if task.mark_scheduled() {
            let task: SharedTask = task;
            scheduler.enqueue_wake(task);
        }
    }
}

enum CompletionState<T> {
    Pending(Option<Waker>),
    Ready(Result<T, JoinError>),
    Consumed,
}

struct Completion<T> {
    state: Mutex<CompletionState<T>>,
    ready: Condvar,
}

impl<T> Completion<T> {
    fn new() -> Self {
        Self {
            state: Mutex::new(CompletionState::Pending(None)),
            ready: Condvar::new(),
        }
    }

    fn complete(&self, result: Result<T, JoinError>) {
        let waker = {
            let mut state = lock(&self.state);
            let waker = match &mut *state {
                CompletionState::Pending(waker) => waker.take(),
                CompletionState::Ready(_) | CompletionState::Consumed => return,
            };
            *state = CompletionState::Ready(result);
            waker
        };

        self.ready.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn poll(&self, context: &mut Context<'_>) -> Poll<Result<T, JoinError>> {
        let mut state = lock(&self.state);

        if matches!(*state, CompletionState::Ready(_)) {
            let previous = std::mem::replace(&mut *state, CompletionState::Consumed);
            return match previous {
                CompletionState::Ready(result) => Poll::Ready(result),
                _ => unreachable!("state was checked above"),
            };
        }

        match &mut *state {
            CompletionState::Pending(waker) => {
                *waker = Some(context.waker().clone());
                Poll::Pending
            }
            CompletionState::Consumed => {
                panic!("JoinHandle polled after its task result was consumed")
            }
            CompletionState::Ready(_) => unreachable!("state was checked above"),
        }
    }

    fn is_finished(&self) -> bool {
        !matches!(*lock(&self.state), CompletionState::Pending(_))
    }

    fn wait(&self) -> Result<T, JoinError> {
        let mut state = lock(&self.state);

        loop {
            if matches!(*state, CompletionState::Ready(_)) {
                let previous = std::mem::replace(&mut *state, CompletionState::Consumed);
                return match previous {
                    CompletionState::Ready(result) => result,
                    _ => unreachable!("state was checked above"),
                };
            }

            match &mut *state {
                CompletionState::Pending(_) => {
                    if is_worker_thread() {
                        panic!(
                            "cannot block on a JoinHandle from a scheduler worker; use .await instead"
                        );
                    }

                    state = self
                        .ready
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                CompletionState::Consumed => {
                    panic!("JoinHandle waited after its task result was consumed")
                }
                CompletionState::Ready(_) => unreachable!("state was checked above"),
            }
        }
    }
}

pub struct JoinHandle<T> {
    task: Arc<Task<T>>,
    scheduler: Weak<Inner>,
}

impl<T> JoinHandle<T> {
    pub(super) fn new(task: Arc<Task<T>>, scheduler: &Arc<Inner>) -> Self {
        Self {
            task,
            scheduler: Arc::downgrade(scheduler),
        }
    }

    pub fn id(&self) -> TaskId {
        self.task.id
    }

    pub fn abort(&self) {
        self.task.cancel_task();
        if let Some(scheduler) = self.scheduler.upgrade() {
            scheduler.unregister(self.task.id);
        }
    }

    pub fn is_finished(&self) -> bool {
        self.task.completion.is_finished()
    }

    pub fn wait(self) -> Result<T, JoinError> {
        self.task.completion.wait()
    }
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JoinHandle")
            .field("id", &self.task.id)
            .field("finished", &self.is_finished())
            .finish()
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().task.completion.poll(context)
    }
}

fn drop_silently<T>(value: T) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(value)));
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
