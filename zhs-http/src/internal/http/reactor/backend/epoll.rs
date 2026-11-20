use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use crate::internal::http::reactor::{ConnectionState, Event};

pub(crate) const NAME: &str = "epoll";

const EPOLLIN: u32 = 0x001;
const EPOLLOUT: u32 = 0x004;
const EPOLLERR: u32 = 0x008;
const EPOLLHUP: u32 = 0x010;
const EPOLLRDHUP: u32 = 0x2000;
const EPOLLONESHOT: u32 = 1 << 30;

const EPOLL_CTL_ADD: c_int = 1;
const EPOLL_CTL_DEL: c_int = 2;
const EPOLL_CTL_MOD: c_int = 3;

const EPOLL_CLOEXEC: c_int = 0o2000000;
const O_CLOEXEC: c_int = 0o2000000;
const O_NONBLOCK: c_int = 0o4000;

// The Linux kernel packs `struct epoll_event` only on x86_64 (via EPOLL_PACKED)
// to keep it 12 bytes, matching the 32-bit ABI. On 64-bit non-x86 (aarch64,
// riscv64, ...) the struct is the natural 16-byte layout. Mirror that here so
// `epoll_wait` writes into a correctly-sized buffer on every target.
#[cfg(target_arch = "x86_64")]
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct EpollEvent {
    events: u32,
    data: u64,
}

#[cfg(not(target_arch = "x86_64"))]
#[repr(C)]
#[derive(Clone, Copy)]
struct EpollEvent {
    events: u32,
    data: u64,
}

unsafe extern "C" {
    fn epoll_create1(flags: c_int) -> c_int;
    fn epoll_ctl(epfd: c_int, op: c_int, fd: c_int, event: *mut EpollEvent) -> c_int;
    fn epoll_wait(epfd: c_int, events: *mut EpollEvent, maxevents: c_int, timeout: c_int) -> c_int;
    fn pipe2(fds: *mut c_int, flags: c_int) -> c_int;
}

pub(crate) struct Poller {
    epoll: OwnedFd,
    wake_read: File,
    wake_write: File,
    state: Mutex<State>,
    closed: AtomicBool,
}

#[derive(Default)]
struct State {
    entries: HashMap<u64, Entry>,
}

struct Entry {
    stream: TcpStream,
    connection: Weak<ConnectionState>,
}

impl Poller {
    pub(crate) fn new() -> io::Result<Self> {
        let raw = unsafe { epoll_create1(EPOLL_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }

        let epoll = unsafe { OwnedFd::from_raw_fd(raw) };

        let mut fds = [0 as c_int; 2];

        if unsafe { pipe2(fds.as_mut_ptr(), O_CLOEXEC | O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let wake_read = unsafe { File::from_raw_fd(fds[0]) };

        let wake_write = unsafe { File::from_raw_fd(fds[1]) };

        let mut wake_event = EpollEvent {
            events: EPOLLIN,
            data: 0,
        };

        let result = unsafe {
            epoll_ctl(
                epoll.as_raw_fd(),
                EPOLL_CTL_ADD,
                wake_read.as_raw_fd(),
                &mut wake_event,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            epoll,
            wake_read,
            wake_write,
            state: Mutex::new(State::default()),
            closed: AtomicBool::new(false),
        })
    }

    pub(crate) fn register(
        &self,
        stream: TcpStream,
        token: u64,
        connection: &Arc<ConnectionState>,
    ) -> io::Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(closed_error());
        }

        let fd = stream.as_raw_fd();

        let mut state = lock(&self.state);
        if state.entries.contains_key(&token) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the connection token is already registered",
            ));
        }
        state.entries.insert(
            token,
            Entry {
                stream,
                connection: Arc::downgrade(connection),
            },
        );

        let mut event = EpollEvent {
            events: EPOLLONESHOT,
            data: token,
        };

        let result = unsafe { epoll_ctl(self.epoll.as_raw_fd(), EPOLL_CTL_ADD, fd, &mut event) };
        if result < 0 {
            state.entries.remove(&token);
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    pub(crate) fn set_interest(
        &self,
        fd: RawFd,
        token: u64,
        read: bool,
        write: bool,
        _notify: bool,
    ) -> io::Result<()> {
        if !read && !write {
            return Ok(());
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(closed_error());
        }

        let mut flags = EPOLLONESHOT;
        if read {
            flags |= EPOLLIN | EPOLLRDHUP;
        }
        if write {
            flags |= EPOLLOUT;
        }
        let mut event = EpollEvent {
            events: flags,
            data: token,
        };

        let result = unsafe { epoll_ctl(self.epoll.as_raw_fd(), EPOLL_CTL_MOD, fd, &mut event) };
        if result < 0 {
            let error = io::Error::last_os_error();

            return if error.kind() == io::ErrorKind::NotFound || error.raw_os_error() == Some(9) {
                Err(closed_error())
            } else {
                Err(error)
            };
        }

        Ok(())
    }

    pub(crate) fn deregister(&self, token: u64) {
        let mut state = lock(&self.state);
        let Some(entry) = state.entries.remove(&token) else {
            return;
        };

        unsafe {
            epoll_ctl(
                self.epoll.as_raw_fd(),
                EPOLL_CTL_DEL,
                entry.stream.as_raw_fd(),
                std::ptr::null_mut(),
            );
        }
    }

    pub(crate) fn connection(&self, token: u64) -> Option<Arc<ConnectionState>> {
        lock(&self.state)
            .entries
            .get(&token)
            .and_then(|entry| entry.connection.upgrade())
    }

    pub(crate) fn connections(&self) -> Vec<Arc<ConnectionState>> {
        lock(&self.state)
            .entries
            .values()
            .filter_map(|entry| entry.connection.upgrade())
            .collect()
    }

    pub(crate) fn wait(&self, events: &mut Vec<Event>) -> io::Result<()> {
        let mut buffer = [EpollEvent { events: 0, data: 0 }; 256];

        let count = unsafe {
            epoll_wait(
                self.epoll.as_raw_fd(),
                buffer.as_mut_ptr(),
                buffer.len() as c_int,
                -1,
            )
        };

        if count < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(())
            } else {
                Err(error)
            };
        }

        for index in 0..count as usize {
            let event = buffer[index];
            let token = event.data;
            let flags = event.events;

            if token == 0 {
                self.drain_wakeups();
                continue;
            }

            events.push(Event {
                token,
                readable: flags & (EPOLLIN | EPOLLRDHUP | EPOLLHUP | EPOLLERR) != 0,
                writable: flags & (EPOLLOUT | EPOLLHUP | EPOLLERR) != 0,
                error: flags & EPOLLERR != 0,
            });
        }

        Ok(())
    }

    pub(crate) fn wake(&self) {
        let _ = (&self.wake_write).write(&[1_u8]);
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn drain_wakeups(&self) {
        let mut buffer = [0_u8; 64];
        loop {
            match (&self.wake_read).read(&mut buffer) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }
}

fn closed_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "the reactor has been shut down",
    )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(crate::internal::recover_poison)
}
