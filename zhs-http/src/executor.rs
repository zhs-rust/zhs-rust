use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

struct Parker {
    thread: Thread,
    notified: AtomicBool,
}

impl Wake for Parker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.notified.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let parker = Arc::new(Parker {
        thread: thread::current(),
        notified: AtomicBool::new(false),
    });
    let waker = Waker::from(parker.clone());
    let mut context = Context::from_waker(&waker);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                if !parker.notified.swap(false, Ordering::SeqCst) {
                    thread::park();
                }
            }
        }
    }
}
