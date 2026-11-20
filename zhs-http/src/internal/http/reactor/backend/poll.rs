use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::raw::{c_int, c_short};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use crate::internal::http::reactor::{ConnectionState, Event};

pub(crate) const NAME: &str = "poll";

const POLLIN: c_short = 0x0001;
const POLLOUT: c_short = 0x0004;
const POLLERR: c_short = 0x0008;
const POLLHUP: c_short = 0x0010;
const POLLNVAL: c_short = 0x0020;

const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
#[cfg(any(target_os = "solaris", target_os = "illumos"))]
const O_NONBLOCK: c_int = 0x80;
#[cfg(not(any(target_os = "solaris", target_os = "illumos")))]
const O_NONBLOCK: c_int = 0x0004;

#[cfg(any(target_os = "solaris", target_os = "illumos"))]
type Nfds = u64;
#[cfg(not(any(target_os = "solaris", target_os = "illumos")))]
type Nfds = u32;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

unsafe extern "C" {
    fn poll(fds: *mut PollFd, count: Nfds, timeout: c_int) -> c_int;
    fn pipe(fds: *mut c_int) -> c_int;
    fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
}

pub(crate) struct Poller {
    wake_read: File,
    wake_write: File,

    entries: Mutex<HashMap<u64, Entry>>,

    commands: Mutex<Vec<Command>>,

    waiting: AtomicBool,

    slots: Mutex<Slots>,
    closed: AtomicBool,
}

struct Entry {
    stream: TcpStream,
    connection: Weak<ConnectionState>,
}

enum Command {
    Interest {
        token: u64,
        fd: RawFd,
        read: bool,
        write: bool,
    },
    Remove {
        token: u64,
        stream: TcpStream,
    },
}

struct Slots {
    fds: Vec<PollFd>,

    tokens: Vec<u64>,
    index: HashMap<u64, usize>,
}

impl Poller {
    pub(crate) fn new() -> io::Result<Self> {
        let mut fds = [0 as c_int; 2];

        if unsafe { pipe(fds.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let wake_read = unsafe { File::from_raw_fd(fds[0]) };

        let wake_write = unsafe { File::from_raw_fd(fds[1]) };

        set_nonblocking(wake_read.as_raw_fd())?;
        set_nonblocking(wake_write.as_raw_fd())?;
        let wake_fd = wake_read.as_raw_fd();

        Ok(Self {
            wake_read,
            wake_write,
            entries: Mutex::new(HashMap::new()),
            commands: Mutex::new(Vec::new()),
            waiting: AtomicBool::new(false),
            slots: Mutex::new(Slots {
                fds: vec![PollFd {
                    fd: wake_fd,
                    events: POLLIN,
                    revents: 0,
                }],
                tokens: vec![0],
                index: HashMap::new(),
            }),
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

        let mut entries = lock(&self.entries);
        if entries.contains_key(&token) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the connection token is already registered",
            ));
        }
        entries.insert(
            token,
            Entry {
                stream,
                connection: Arc::downgrade(connection),
            },
        );
        Ok(())
    }

    pub(crate) fn set_interest(
        &self,
        fd: RawFd,
        token: u64,
        read: bool,
        write: bool,
        notify: bool,
    ) -> io::Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(closed_error());
        }

        {
            let mut commands = lock(&self.commands);
            commands.push(Command::Interest {
                token,
                fd,
                read,
                write,
            });

            if !notify || !self.waiting.load(Ordering::SeqCst) {
                return Ok(());
            }
        }

        self.wake();
        Ok(())
    }

    pub(crate) fn deregister(&self, token: u64) {
        let mut entries = lock(&self.entries);
        let Some(entry) = entries.remove(&token) else {
            return;
        };

        let mut commands = lock(&self.commands);
        commands.push(Command::Remove {
            token,
            stream: entry.stream,
        });
        if self.waiting.load(Ordering::SeqCst) {
            self.wake();
        }
    }

    pub(crate) fn connection(&self, token: u64) -> Option<Arc<ConnectionState>> {
        lock(&self.entries)
            .get(&token)
            .and_then(|entry| entry.connection.upgrade())
    }

    pub(crate) fn connections(&self) -> Vec<Arc<ConnectionState>> {
        lock(&self.entries)
            .values()
            .filter_map(|entry| entry.connection.upgrade())
            .collect()
    }

    pub(crate) fn wait(&self, events: &mut Vec<Event>) -> io::Result<()> {
        loop {
            let mut slots = lock(&self.slots);
            {
                let mut commands = lock(&self.commands);
                for command in commands.drain(..) {
                    match command {
                        Command::Interest {
                            token,
                            fd,
                            read,
                            write,
                        } => set_slot(&mut slots, token, fd, read, write),
                        Command::Remove { token, stream } => {
                            remove_slot(&mut slots, token);
                            drop(stream);
                        }
                    }
                }
                self.waiting.store(true, Ordering::SeqCst);
            }

            let result = unsafe { poll(slots.fds.as_mut_ptr(), slots.fds.len() as Nfds, -1) };

            self.waiting.store(false, Ordering::SeqCst);

            if result < 0 {
                let error = io::Error::last_os_error();
                return if error.kind() == io::ErrorKind::Interrupted {
                    Ok(())
                } else {
                    Err(error)
                };
            }

            if result == 0 {
                continue;
            }

            if slots.fds[0].revents != 0 {
                slots.fds[0].revents = 0;
                self.drain_wakeups();
            }

            for index in 1..slots.fds.len() {
                let revents = slots.fds[index].revents;
                if revents == 0 {
                    continue;
                }
                slots.fds[index].revents = 0;

                events.push(Event {
                    token: slots.tokens[index],
                    readable: revents & (POLLIN | POLLHUP | POLLERR | POLLNVAL) != 0,
                    writable: revents & (POLLOUT | POLLHUP | POLLERR | POLLNVAL) != 0,
                    error: revents & (POLLERR | POLLNVAL) != 0,
                });
            }

            return Ok(());
        }
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

fn set_slot(slots: &mut Slots, token: u64, fd: RawFd, read: bool, write: bool) {
    if !read && !write {
        remove_slot(slots, token);
        return;
    }

    if let Some(&index) = slots.index.get(&token) {
        slots.fds[index].events = interests(read, write);
        return;
    }

    let index = slots.fds.len();
    slots.fds.push(PollFd {
        fd,
        events: interests(read, write),
        revents: 0,
    });
    slots.tokens.push(token);
    slots.index.insert(token, index);
}

fn remove_slot(slots: &mut Slots, token: u64) {
    let Some(index) = slots.index.remove(&token) else {
        return;
    };
    slots.fds.swap_remove(index);
    slots.tokens.swap_remove(index);
    if let Some(&moved) = slots.tokens.get(index) {
        slots.index.insert(moved, index);
    }
}

fn interests(read: bool, write: bool) -> c_short {
    let mut events = 0;
    if read {
        events |= POLLIN;
    }
    if write {
        events |= POLLOUT;
    }
    events
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { fcntl(fd, F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }

    if unsafe { fcntl(fd, F_SETFL, flags | O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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
