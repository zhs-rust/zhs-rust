mod backend;

use std::io;
use std::net::{Shutdown, TcpStream};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};

use backend::Poller;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    pub(crate) token: u64,

    pub(crate) readable: bool,

    pub(crate) writable: bool,

    pub(crate) error: bool,
}

pub(crate) struct Reactor {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct Inner {
    poller: Poller,
    next_token: AtomicU64,
    shutdown: AtomicBool,
}

impl Inner {
    fn register(self: &Arc<Self>, stream: &TcpStream) -> io::Result<Arc<ConnectionState>> {
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);

        let polled = stream.try_clone()?;
        let handle = raw_handle(&polled);
        let state = Arc::new(ConnectionState {
            token,
            handle,
            inner: Arc::clone(self),
            closed: AtomicBool::new(false),
            waiters: Mutex::new(Waiters::default()),
        });

        self.poller.register(polled, token, &state)?;
        Ok(state)
    }

    fn dispatch(&self, event: Event) {
        let Some(state) = self.poller.connection(event.token) else {
            return;
        };

        let (readable, writable) = {
            let mut waiters = lock(&state.waiters);

            let readable = if event.readable || event.error {
                let waker = waiters.read.take();
                waiters.read_ready = true;

                waiters.want_read = waker.is_some() && backend::KEEPS_READ_INTEREST;
                waker
            } else {
                None
            };
            let writable = if event.writable || event.error {
                waiters.want_write = false;
                waiters.write_ready = true;
                waiters.write.take()
            } else {
                None
            };

            let changed = waiters.armed_read != waiters.want_read
                || waiters.armed_write != waiters.want_write;
            if changed
                && self
                    .poller
                    .set_interest(
                        state.handle,
                        state.token,
                        waiters.want_read,
                        waiters.want_write,
                        false,
                    )
                    .is_ok()
            {
                waiters.armed_read = waiters.want_read;
                waiters.armed_write = waiters.want_write;
            }

            (readable, writable)
        };

        if let Some(waker) = readable {
            waker.wake();
        }
        if let Some(waker) = writable {
            waker.wake();
        }
    }

    fn close_connections(&self) {
        for state in self.poller.connections() {
            state.close();
        }
    }
}

pub(crate) struct ConnectionState {
    token: u64,

    handle: RawHandle,
    inner: Arc<Inner>,
    closed: AtomicBool,
    waiters: Mutex<Waiters>,
}

#[derive(Default)]
struct Waiters {
    want_read: bool,
    want_write: bool,
    armed_read: bool,
    armed_write: bool,

    read_ready: bool,
    write_ready: bool,
    read: Option<Waker>,
    write: Option<Waker>,
}

impl ConnectionState {
    fn poll_read(&self, waker: &Waker) -> io::Result<Poll<()>> {
        self.poll_io(true, waker)
    }

    fn poll_write(&self, waker: &Waker) -> io::Result<Poll<()>> {
        self.poll_io(false, waker)
    }

    fn poll_io(&self, read: bool, waker: &Waker) -> io::Result<Poll<()>> {
        let mut waiters = lock(&self.waiters);
        if self.closed.load(Ordering::Acquire) {
            return Err(connection_closed());
        }

        if read && waiters.read_ready {
            waiters.read_ready = false;
            return Ok(Poll::Ready(()));
        }
        if !read && waiters.write_ready {
            waiters.write_ready = false;
            return Ok(Poll::Ready(()));
        }

        if read {
            waiters.want_read = true;
            waiters.read = Some(waker.clone());
        } else {
            waiters.want_write = true;
            waiters.write = Some(waker.clone());
        }

        let changed = if read {
            !waiters.armed_read
        } else {
            !waiters.armed_write
        };
        if changed {
            self.inner.poller.set_interest(
                self.handle,
                self.token,
                waiters.want_read,
                waiters.want_write,
                true,
            )?;
            waiters.armed_read = waiters.want_read;
            waiters.armed_write = waiters.want_write;
        }
        Ok(Poll::Pending)
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        let (read, write) = {
            let mut waiters = lock(&self.waiters);
            if self.closed.swap(true, Ordering::AcqRel) {
                return;
            }
            (waiters.read.take(), waiters.write.take())
        };

        if let Some(waker) = read {
            waker.wake();
        }
        if let Some(waker) = write {
            waker.wake();
        }
    }
}

impl Drop for ConnectionState {
    fn drop(&mut self) {
        self.inner.poller.deregister(self.token);
    }
}

impl Reactor {
    pub(crate) fn new() -> io::Result<Self> {
        let poller = Poller::new()?;
        let inner = Arc::new(Inner {
            poller,
            next_token: AtomicU64::new(1),
            shutdown: AtomicBool::new(false),
        });

        let thread_inner = Arc::clone(&inner);
        let thread = thread::Builder::new()
            .name("zhs-http-reactor".to_owned())
            .spawn(move || run(thread_inner))?;

        Ok(Self {
            inner,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(crate) fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::Release);
        self.inner.poller.close();
        self.inner.poller.wake();

        let handle = lock(&self.thread).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(inner: Arc<Inner>) {
    let mut events = Vec::with_capacity(128);

    loop {
        events.clear();
        match inner.poller.wait(&mut events) {
            Ok(()) => {}
            Err(_) => {
                inner.shutdown.store(true, Ordering::Release);
                inner.poller.close();
                break;
            }
        }
        if inner.shutdown.load(Ordering::Acquire) {
            break;
        }

        for event in events.drain(..) {
            inner.dispatch(event);
        }
    }

    inner.close_connections();
}

pub(crate) struct AsyncTcpStream {
    stream: TcpStream,
    state: Arc<ConnectionState>,
}

impl AsyncTcpStream {
    pub(crate) fn new(stream: TcpStream, reactor: &Reactor) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        let _ = stream.set_nodelay(true);

        let state = reactor.inner.register(&stream)?;
        Ok(Self { stream, state })
    }

    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            stream: self.stream.try_clone()?,
            state: Arc::clone(&self.state),
        })
    }

    pub(crate) async fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        use std::io::Read;

        loop {
            match self.stream.read(buffer) {
                Ok(read) => return Ok(read),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    Readable::new(Arc::clone(&self.state)).await?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }

    pub(crate) async fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        let mut filled = 0;
        while filled < buffer.len() {
            let read = self.read(&mut buffer[filled..]).await?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the connection closed in the middle of the request body",
                ));
            }
            filled += read;
        }
        Ok(())
    }

    pub(crate) async fn write_all(&mut self, buffer: &[u8]) -> io::Result<()> {
        use std::io::Write;

        let mut written = 0;
        while written < buffer.len() {
            match self.stream.write(&buffer[written..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "the connection accepted no data",
                    ));
                }
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    Writable::new(Arc::clone(&self.state)).await?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Drop for AsyncTcpStream {
    fn drop(&mut self) {
        self.state.close();
    }
}

struct Readable {
    state: Arc<ConnectionState>,
}

impl Readable {
    fn new(state: Arc<ConnectionState>) -> Self {
        Self { state }
    }
}

impl std::future::Future for Readable {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.state.is_closed() {
            return Poll::Ready(Err(connection_closed()));
        }

        match this.state.poll_read(context.waker()) {
            Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
            Ok(Poll::Pending) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

struct Writable {
    state: Arc<ConnectionState>,
}

impl Writable {
    fn new(state: Arc<ConnectionState>) -> Self {
        Self { state }
    }
}

impl std::future::Future for Writable {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.state.is_closed() {
            return Poll::Ready(Err(connection_closed()));
        }

        match this.state.poll_write(context.waker()) {
            Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
            Ok(Poll::Pending) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }
}

fn connection_closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the connection or its reactor has been shut down",
    )
}

pub(crate) fn backend_name() -> &'static str {
    backend::NAME
}

#[cfg(unix)]
pub(crate) type RawHandle = std::os::fd::RawFd;
#[cfg(windows)]
pub(crate) type RawHandle = std::os::windows::io::RawSocket;

#[cfg(unix)]
fn raw_handle(stream: &TcpStream) -> RawHandle {
    use std::os::fd::AsRawFd;
    stream.as_raw_fd()
}

#[cfg(windows)]
fn raw_handle(stream: &TcpStream) -> RawHandle {
    use std::os::windows::io::AsRawSocket;
    stream.as_raw_socket()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(crate::internal::recover_poison)
}
