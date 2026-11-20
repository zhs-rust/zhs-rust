use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::DataError;

pub trait Connector: Send + Sync + 'static {
    type Connection: Send + 'static;

    fn connect(&self) -> Result<Self::Connection, DataError>;

    fn probe(&self, connection: &mut Self::Connection) -> Result<(), DataError>;
}

#[derive(Clone, Debug)]
pub struct PoolOptions {
    pub capacity: usize,

    pub acquire_timeout: Option<Duration>,

    pub idle_timeout: Option<Duration>,

    pub ping_on_checkout: bool,
}

impl PoolOptions {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            acquire_timeout: Some(Duration::from_secs(5)),
            idle_timeout: Some(Duration::from_secs(300)),
            ping_on_checkout: true,
        }
    }

    pub fn acquire_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.acquire_timeout = timeout.into();
        self
    }

    pub fn idle_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.idle_timeout = timeout.into();
        self
    }

    pub fn without_ping_on_checkout(mut self) -> Self {
        self.ping_on_checkout = false;
        self
    }
}

impl Default for PoolOptions {
    fn default() -> Self {
        Self::new(4)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    pub busy: usize,

    pub idle: usize,

    pub capacity: usize,
}

struct Idle<C: Connector> {
    connection: C::Connection,
    since: Instant,
}

struct State<C: Connector> {
    idle: Vec<Idle<C>>,
    busy: usize,
    closed: bool,
}

struct Inner<C: Connector> {
    connector: C,
    config: PoolOptions,
    state: Mutex<State<C>>,
    ready: Condvar,
}

impl<C: Connector> Inner<C> {
    fn take_idle(&self, state: &mut State<C>) -> Option<C::Connection> {
        while let Some(entry) = state.idle.pop() {
            let expired = self
                .config
                .idle_timeout
                .is_some_and(|timeout| entry.since.elapsed() >= timeout);
            if expired {
                continue;
            }
            return Some(entry.connection);
        }
        None
    }

    fn get(self: &Arc<Self>) -> Result<PooledConnection<C>, DataError> {
        let deadline = self
            .config
            .acquire_timeout
            .map(|timeout| Instant::now() + timeout);
        let mut state = lock(&self.state);

        loop {
            if state.closed {
                return Err(DataError::Closed);
            }

            if let Some(connection) = self.take_idle(&mut state) {
                state.busy += 1;
                drop(state);

                let connection = if self.config.ping_on_checkout {
                    match self.probe(connection) {
                        Ok(connection) => connection,
                        Err(_) => {
                            let mut guard = lock(&self.state);
                            guard.busy = guard.busy.saturating_sub(1);
                            drop(guard);
                            state = lock(&self.state);
                            continue;
                        }
                    }
                } else {
                    connection
                };

                return Ok(PooledConnection {
                    inner: Arc::clone(self),
                    connection: Some(connection),
                });
            }

            if state.busy < self.config.capacity {
                state.busy += 1;
                drop(state);

                return match self.connector.connect() {
                    Ok(connection) => Ok(PooledConnection {
                        inner: Arc::clone(self),
                        connection: Some(connection),
                    }),
                    Err(error) => {
                        let mut state = lock(&self.state);
                        state.busy -= 1;
                        drop(state);
                        self.ready.notify_one();
                        Err(error)
                    }
                };
            }

            state = match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(DataError::Timeout("waiting for a pooled connection"));
                    }
                    match self.ready.wait_timeout(state, remaining) {
                        Ok((guard, _)) => guard,
                        Err(poisoned) => {
                            eprintln!("zhs-entity pool condition variable poisoned; recovering");
                            poisoned.into_inner().0
                        }
                    }
                }
                None => match self.ready.wait(state) {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        eprintln!("zhs-entity pool condition variable poisoned; recovering");
                        poisoned.into_inner()
                    }
                },
            };
        }
    }

    fn probe(&self, mut connection: C::Connection) -> Result<C::Connection, DataError> {
        self.connector.probe(&mut connection)?;
        Ok(connection)
    }

    fn put(&self, connection: C::Connection) {
        let mut state = lock(&self.state);
        state.busy = state.busy.saturating_sub(1);

        if state.closed {
            drop(state);
            drop(connection);
            return;
        }

        state.idle.push(Idle {
            connection,
            since: Instant::now(),
        });
        drop(state);
        self.ready.notify_one();
    }

    fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        state.idle.clear();
        drop(state);
        self.ready.notify_all();
    }

    fn reap(&self) -> usize {
        let Some(timeout) = self.config.idle_timeout else {
            return 0;
        };

        let mut state = lock(&self.state);
        let before = state.idle.len();
        state.idle.retain(|entry| entry.since.elapsed() < timeout);
        before - state.idle.len()
    }

    fn stats(&self) -> PoolStats {
        let state = lock(&self.state);
        PoolStats {
            busy: state.busy,
            idle: state.idle.len(),
            capacity: self.config.capacity,
        }
    }
}

pub struct Pool<C: Connector> {
    inner: Arc<Inner<C>>,
}

impl<C: Connector> Pool<C> {
    pub fn new(connector: C, config: PoolOptions) -> Result<Self, DataError> {
        if config.capacity == 0 {
            return Err(DataError::query(
                "a connection pool needs at least one slot",
            ));
        }

        let inner = Arc::new(Inner {
            connector,
            config,
            state: Mutex::new(State {
                idle: Vec::new(),
                busy: 0,
                closed: false,
            }),
            ready: Condvar::new(),
        });

        let connection = inner.connector.connect()?;
        inner.put(connection);

        Ok(Self { inner })
    }

    pub fn get(&self) -> Result<PooledConnection<C>, DataError> {
        self.inner.get()
    }

    pub fn capacity(&self) -> usize {
        self.inner.config.capacity
    }

    pub fn stats(&self) -> PoolStats {
        self.inner.stats()
    }

    pub fn reap(&self) -> usize {
        self.inner.reap()
    }

    pub fn close(&self) {
        self.inner.close();
    }
}

impl<C: Connector> Drop for Pool<C> {
    fn drop(&mut self) {
        self.close();
    }
}

pub struct PooledConnection<C: Connector> {
    inner: Arc<Inner<C>>,
    connection: Option<C::Connection>,
}

impl<C: Connector> PooledConnection<C> {
    pub fn discard(mut self) {
        drop(self.connection.take());
        let mut state = lock(&self.inner.state);
        state.busy = state.busy.saturating_sub(1);
        drop(state);
        self.inner.ready.notify_one();
    }

    pub fn connection(&self) -> &C::Connection {
        self.connection
            .as_ref()
            .expect("a pooled connection always holds a connection")
    }
}

impl<C: Connector> Deref for PooledConnection<C> {
    type Target = C::Connection;

    fn deref(&self) -> &Self::Target {
        self.connection()
    }
}

impl<C: Connector> DerefMut for PooledConnection<C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.connection
            .as_mut()
            .expect("a pooled connection always holds a connection")
    }
}

impl<C: Connector> Drop for PooledConnection<C> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            self.inner.put(connection);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("zhs-entity pool mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestConnector;

    impl Connector for TestConnector {
        type Connection = usize;

        fn connect(&self) -> Result<Self::Connection, DataError> {
            Ok(0)
        }

        fn probe(&self, _connection: &mut Self::Connection) -> Result<(), DataError> {
            Ok(())
        }
    }

    #[test]
    fn discard_releases_the_busy_slot() {
        let pool = Pool::new(
            TestConnector,
            PoolOptions::new(1).acquire_timeout(Duration::from_millis(200)),
        )
        .unwrap();

        pool.get().unwrap().discard();

        assert_eq!(pool.stats().busy, 0);

        // Before the fix, this acquire timed out because the discarded slot
        // was never returned to the pool.
        let connection = pool.get().unwrap();
        assert_eq!(pool.stats().busy, 1);
        drop(connection);
    }
}
