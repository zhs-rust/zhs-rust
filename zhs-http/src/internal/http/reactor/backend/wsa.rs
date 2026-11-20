use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::raw::{c_int, c_short};
use std::os::windows::io::{AsRawSocket, RawSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use crate::internal::http::reactor::{ConnectionState, Event};

pub(crate) const NAME: &str = "WSAPoll";

const POLLRDNORM: c_short = 0x0100;
const POLLWRNORM: c_short = 0x0010;
const POLLERR: c_short = 0x0001;
const POLLHUP: c_short = 0x0002;
const POLLNVAL: c_short = 0x0004;

#[repr(C)]
struct WsaPollFd {
    socket: RawSocket,
    events: c_short,
    revents: c_short,
}

#[link(name = "ws2_32")]
unsafe extern "C" {
    fn WSAPoll(fds: *mut WsaPollFd, count: u32, timeout: c_int) -> c_int;
    fn WSAGetLastError() -> c_int;
}

pub(crate) struct Poller {
    wake_read: TcpStream,
    wake_write: TcpStream,

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
        socket: RawSocket,
        read: bool,
        write: bool,
    },
    Remove {
        token: u64,
        stream: TcpStream,
    },
}

struct Slots {
    fds: Vec<WsaPollFd>,

    tokens: Vec<u64>,
    index: HashMap<u64, usize>,
}

impl Poller {
    pub(crate) fn new() -> io::Result<Self> {
        let (wake_read, wake_write) = wakeup_pair()?;

        let mut fds = Vec::with_capacity(32);
        fds.push(WsaPollFd {
            socket: wake_read.as_raw_socket(),
            events: POLLRDNORM,
            revents: 0,
        });

        Ok(Self {
            wake_read,
            wake_write,
            entries: Mutex::new(HashMap::new()),
            commands: Mutex::new(Vec::new()),
            waiting: AtomicBool::new(false),
            slots: Mutex::new(Slots {
                fds,
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
        socket: RawSocket,
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
                socket,
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
                            socket,
                            read,
                            write,
                        } => set_slot(&mut slots, token, socket, read, write),
                        Command::Remove { token, stream } => {
                            remove_slot(&mut slots, token);

                            drop(stream);
                        }
                    }
                }
                self.waiting.store(true, Ordering::SeqCst);
            }

            let result = unsafe { WSAPoll(slots.fds.as_mut_ptr(), slots.fds.len() as u32, -1) };

            self.waiting.store(false, Ordering::SeqCst);

            if result < 0 {
                let code = unsafe { WSAGetLastError() };

                if code == 10004 {
                    continue;
                }
                return Err(io::Error::from_raw_os_error(code));
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
                    readable: revents & (POLLRDNORM | POLLHUP | POLLERR | POLLNVAL) != 0,
                    writable: revents & (POLLWRNORM | POLLHUP | POLLERR | POLLNVAL) != 0,
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

fn set_slot(slots: &mut Slots, token: u64, socket: RawSocket, read: bool, write: bool) {
    if !read && !write {
        remove_slot(slots, token);
        return;
    }

    if let Some(&index) = slots.index.get(&token) {
        slots.fds[index].events = interests(read, write);
        return;
    }

    let index = slots.fds.len();
    slots.fds.push(WsaPollFd {
        socket,
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
        events |= POLLRDNORM;
    }
    if write {
        events |= POLLWRNORM;
    }
    events
}

fn wakeup_pair() -> io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listener.local_addr()?;
    let writer = TcpStream::connect(address)?;
    let (reader, _) = listener.accept()?;

    reader.set_nonblocking(true)?;
    writer.set_nonblocking(true)?;
    Ok((reader, writer))
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
