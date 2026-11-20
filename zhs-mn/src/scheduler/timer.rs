use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::error::{SchedulerError, TimeoutError};

pub struct Sleep {
    timer: Arc<Timer>,
    deadline: Option<Instant>,
    entry: Option<Arc<TimerEntry>>,
}

impl Sleep {
    pub(crate) fn new(timer: Arc<Timer>, deadline: Option<Instant>) -> Self {
        Self {
            timer,
            deadline,
            entry: None,
        }
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if let Some(entry) = &this.entry {
            return if entry.update_waker(context.waker()) {
                Poll::Ready(())
            } else {
                Poll::Pending
            };
        }

        let Some(deadline) = this.deadline else {
            return Poll::Pending;
        };

        if deadline <= Instant::now() {
            return Poll::Ready(());
        }

        match this.timer.register(deadline, context.waker()) {
            Some(entry) => {
                this.entry = Some(entry);
                Poll::Pending
            }

            None => Poll::Ready(()),
        }
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let Some(entry) = &self.entry {
            entry.cancel();
            self.timer.notify_cancelled(entry);
        }
    }
}

pub struct Timeout<F> {
    future: Pin<Box<F>>,
    sleep: Sleep,
}

impl<F> Timeout<F> {
    pub(crate) fn new(future: F, sleep: Sleep) -> Self {
        Self {
            future: Box::pin(future),
            sleep,
        }
    }
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, TimeoutError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if let Poll::Ready(value) = this.future.as_mut().poll(context) {
            return Poll::Ready(Ok(value));
        }

        match Pin::new(&mut this.sleep).poll(context) {
            Poll::Ready(()) => Poll::Ready(Err(TimeoutError)),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub(crate) struct Timer {
    inner: Arc<TimerInner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct TimerInner {
    state: Mutex<TimerState>,
    signal: Condvar,
}

struct TimerState {
    heap: BinaryHeap<ScheduledTimer>,
    sequence: u64,
    shutdown: bool,
}

struct ScheduledTimer {
    deadline: Instant,
    sequence: u64,
    entry: Arc<TimerEntry>,
}

impl PartialEq for ScheduledTimer {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline && self.sequence == other.sequence
    }
}

impl Eq for ScheduledTimer {}

impl PartialOrd for ScheduledTimer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledTimer {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

struct TimerEntry {
    state: Mutex<EntryState>,
}

struct EntryState {
    fired: bool,
    cancelled: bool,
    waker: Option<Waker>,
}

impl Timer {
    pub(crate) fn start() -> Result<Self, SchedulerError> {
        let inner = Arc::new(TimerInner {
            state: Mutex::new(TimerState {
                heap: BinaryHeap::new(),
                sequence: 0,
                shutdown: false,
            }),
            signal: Condvar::new(),
        });

        let thread_inner = Arc::clone(&inner);
        let thread = thread::Builder::new()
            .name("zhs-timer".to_owned())
            .spawn(move || timer_loop(thread_inner))
            .map_err(SchedulerError::timer_spawn)?;

        Ok(Self {
            inner,
            thread: Mutex::new(Some(thread)),
        })
    }

    fn register(&self, deadline: Instant, waker: &Waker) -> Option<Arc<TimerEntry>> {
        let entry = Arc::new(TimerEntry::new(waker));

        let mut state = lock(&self.inner.state);
        if state.shutdown {
            return None;
        }

        let wake = state
            .heap
            .peek()
            .is_none_or(|scheduled| deadline < scheduled.deadline);

        state.sequence = state.sequence.wrapping_add(1);
        let sequence = state.sequence;
        state.heap.push(ScheduledTimer {
            deadline,
            sequence,
            entry: Arc::clone(&entry),
        });
        drop(state);

        if wake {
            self.notify();
        }

        Some(entry)
    }

    fn notify_cancelled(&self, entry: &Arc<TimerEntry>) {
        let state = lock(&self.inner.state);
        let is_head = state
            .heap
            .peek()
            .is_some_and(|scheduled| Arc::ptr_eq(&scheduled.entry, entry));
        drop(state);

        if is_head {
            self.notify();
        }
    }

    fn notify(&self) {
        self.inner.signal.notify_one();
    }

    pub(crate) fn shutdown(&self) {
        let entries = {
            let mut state = lock(&self.inner.state);
            if state.shutdown {
                Vec::new()
            } else {
                state.shutdown = true;
                state
                    .heap
                    .drain()
                    .map(|scheduled| scheduled.entry)
                    .collect::<Vec<_>>()
            }
        };

        self.inner.signal.notify_all();

        let handle = lock(&self.thread).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }

        for entry in entries {
            entry.fire();
        }
    }
}

impl TimerEntry {
    fn new(waker: &Waker) -> Self {
        Self {
            state: Mutex::new(EntryState {
                fired: false,
                cancelled: false,
                waker: Some(waker.clone()),
            }),
        }
    }

    fn update_waker(&self, waker: &Waker) -> bool {
        let mut state = lock(&self.state);
        if state.fired {
            return true;
        }
        if state.cancelled {
            return false;
        }

        state.waker = Some(waker.clone());
        false
    }

    fn fire(&self) {
        let waker = {
            let mut state = lock(&self.state);
            if state.fired || state.cancelled {
                return;
            }
            state.fired = true;
            state.waker.take()
        };

        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn cancel(&self) {
        let waker = {
            let mut state = lock(&self.state);
            if state.fired || state.cancelled {
                return;
            }
            state.cancelled = true;
            state.waker.take()
        };

        drop(waker);
    }

    fn is_cancelled(&self) -> bool {
        lock(&self.state).cancelled
    }
}

fn timer_loop(inner: Arc<TimerInner>) {
    loop {
        let next = {
            let mut state = lock(&inner.state);

            loop {
                if state.shutdown {
                    return;
                }

                drop_cancelled_head(&mut state);
                compact(&mut state);

                let now = Instant::now();
                match state.heap.peek() {
                    None => {
                        state = inner
                            .signal
                            .wait(state)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                    Some(next) if next.deadline <= now => {
                        break state.heap.pop().expect("heap was non-empty after peek");
                    }
                    Some(next) => {
                        let wait = next.deadline.duration_since(now);
                        let (guard, _) = inner
                            .signal
                            .wait_timeout(state, wait)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        state = guard;
                    }
                }
            }
        };

        next.entry.fire();
    }
}

fn drop_cancelled_head(state: &mut TimerState) {
    while state
        .heap
        .peek()
        .is_some_and(|scheduled| scheduled.entry.is_cancelled())
    {
        state.heap.pop();
    }
}

fn compact(state: &mut TimerState) {
    if state.heap.len() < 64 {
        return;
    }

    let cancelled = state
        .heap
        .iter()
        .filter(|scheduled| scheduled.entry.is_cancelled())
        .count();
    if cancelled == 0 || cancelled * 2 < state.heap.len() {
        return;
    }

    let mut retained = BinaryHeap::with_capacity(state.heap.len() - cancelled);
    for scheduled in state.heap.drain() {
        if !scheduled.entry.is_cancelled() {
            retained.push(scheduled);
        }
    }
    state.heap = retained;
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
