use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use crate::error::JoinError;

type BlockingClosure<T> = Box<dyn FnOnce() -> T + Send + 'static>;

pub(crate) type SharedJob = Arc<dyn Runnable>;

pub(crate) trait Runnable: Send + Sync {
    fn cancel(&self);

    fn run(self: Arc<Self>);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(u64);

impl JobId {
    pub(crate) fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "job-{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobState {
    Queued,

    Running,

    Completed,

    Cancelled,
}

pub(super) struct Job<T> {
    id: JobId,
    state: Mutex<JobState>,
    work: Mutex<Option<BlockingClosure<T>>>,
    completion: Completion<T>,
}

impl<T> Job<T> {
    pub(super) fn new(id: JobId, work: BlockingClosure<T>) -> Self {
        Self {
            id,
            state: Mutex::new(JobState::Queued),
            work: Mutex::new(Some(work)),
            completion: Completion::new(),
        }
    }

    fn cancel_job(&self) {
        let work = {
            let mut state = lock(&self.state);
            if !matches!(*state, JobState::Queued) {
                return;
            }
            *state = JobState::Cancelled;

            lock(&self.work).take()
        };

        drop_silently(work);
        self.completion.complete(Err(JoinError::Cancelled));
    }

    fn execute(self: Arc<Self>)
    where
        T: Send + 'static,
    {
        {
            let mut state = lock(&self.state);
            if !matches!(*state, JobState::Queued) {
                return;
            }
            *state = JobState::Running;
        }

        let Some(work) = lock(&self.work).take() else {
            return;
        };

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        let result = match outcome {
            Ok(output) => Ok(output),
            Err(payload) => Err(JoinError::panicked(payload)),
        };

        {
            let mut state = lock(&self.state);
            if matches!(*state, JobState::Cancelled) {
                return;
            }
            *state = JobState::Completed;
        }

        self.completion.complete(result);
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        let work = {
            let slot = match self.work.get_mut() {
                Ok(slot) => slot,
                Err(poisoned) => poisoned.into_inner(),
            };
            slot.take()
        };
        drop_silently(work);
    }
}

impl<T: Send + 'static> Runnable for Job<T> {
    fn cancel(&self) {
        self.cancel_job();
    }

    fn run(self: Arc<Self>) {
        self.execute();
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
                panic!("JoinHandle polled after its job result was consumed")
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
                    state = self
                        .ready
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                CompletionState::Consumed => {
                    panic!("JoinHandle waited after its job result was consumed")
                }
                CompletionState::Ready(_) => unreachable!("state was checked above"),
            }
        }
    }
}

pub struct JoinHandle<T> {
    job: Arc<Job<T>>,
}

impl<T> JoinHandle<T> {
    pub(super) fn new(job: Arc<Job<T>>) -> Self {
        Self { job }
    }

    pub fn id(&self) -> JobId {
        self.job.id
    }

    pub fn abort(&self) {
        self.job.cancel_job();
    }

    pub fn is_finished(&self) -> bool {
        self.job.completion.is_finished()
    }

    pub fn wait(self) -> Result<T, JoinError> {
        self.job.completion.wait()
    }
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JoinHandle")
            .field("id", &self.job.id)
            .field("finished", &self.is_finished())
            .finish()
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().job.completion.poll(context)
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
