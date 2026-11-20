use std::fmt;
use std::future::Future;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::internal::error::Result;
use zhs_mn::platform::TimerResolution;
use zhs_mn::scheduler::Scheduler;

use super::connection;
use super::parse::HttpLimits;
use super::reactor::Reactor;
use super::request::Request;
use super::response::Response;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfig {
    pub limits: HttpLimits,

    pub read_timeout: Duration,

    pub write_timeout: Duration,

    pub keep_alive_timeout: Duration,

    pub max_requests_per_connection: usize,

    pub max_workers: usize,

    pub acceptors: usize,

    pub max_connections: usize,

    pub reactor_interval: Duration,

    pub timer_resolution_millis: u32,

    pub shutdown_timeout: Duration,
}

impl ServerConfig {
    pub fn new() -> Self {
        let max_workers = thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(4);

        Self {
            limits: HttpLimits::new(),
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
            keep_alive_timeout: Duration::from_secs(5),
            max_requests_per_connection: 1_000,
            max_workers,
            acceptors: 1,
            max_connections: 0,
            reactor_interval: Duration::from_millis(1),
            timer_resolution_millis: 0,
            shutdown_timeout: Duration::from_secs(10),
        }
    }

    pub fn with_limits(mut self, limits: HttpLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    pub fn with_write_timeout(mut self, timeout: Duration) -> Self {
        self.write_timeout = timeout;
        self
    }

    pub fn with_keep_alive_timeout(mut self, timeout: Duration) -> Self {
        self.keep_alive_timeout = timeout;
        self
    }

    pub fn with_max_requests_per_connection(mut self, requests: usize) -> Self {
        self.max_requests_per_connection = requests;
        self
    }

    pub fn with_max_workers(mut self, workers: usize) -> Self {
        self.max_workers = workers;
        self
    }

    pub fn with_acceptors(mut self, acceptors: usize) -> Self {
        self.acceptors = acceptors.max(1);
        self
    }

    pub fn with_max_connections(mut self, connections: usize) -> Self {
        self.max_connections = connections;
        self
    }

    pub fn with_reactor_interval(mut self, interval: Duration) -> Self {
        self.reactor_interval = interval;
        self
    }

    pub fn with_timer_resolution_millis(mut self, millis: u32) -> Self {
        self.timer_resolution_millis = millis;
        self
    }

    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::new()
    }
}

pub trait Handler: Send + Sync {
    type Future: Future<Output = Result<Response>> + Send;

    fn handle(&self, request: Request) -> Self::Future;
}

impl<F, Fut> Handler for F
where
    F: Fn(Request) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Response>> + Send,
{
    type Future = Fut;

    fn handle(&self, request: Request) -> Self::Future {
        self(request)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerStats {
    pub accepted_connections: u64,

    pub active_connections: usize,

    pub requests: u64,

    pub failures: u64,

    pub shutdown: bool,
}

#[derive(Clone)]
pub struct ServerStatsSource {
    state: Arc<ServerState>,
}

impl ServerStatsSource {
    pub fn new() -> Self {
        Self {
            state: Arc::new(ServerState::new()),
        }
    }

    pub fn stats(&self) -> ServerStats {
        self.state.stats()
    }

    pub fn is_running(&self) -> bool {
        !self.state.is_shutdown()
    }
}

impl Default for ServerStatsSource {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ServerStatsSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerStatsSource")
            .field("stats", &self.stats())
            .finish()
    }
}

pub(crate) struct ServerState {
    shutdown: AtomicBool,
    accepted: AtomicU64,
    active: AtomicUsize,
    requests: AtomicU64,
    failures: AtomicU64,
}

impl ServerState {
    pub(crate) fn new() -> Self {
        Self {
            shutdown: AtomicBool::new(false),
            accepted: AtomicU64::new(0),
            active: AtomicUsize::new(0),
            requests: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        }
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    fn connection_started(&self) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
        self.active.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn connection_finished(&self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
    }

    pub(crate) fn request_completed(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    fn active_connections(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    pub(crate) fn stats(&self) -> ServerStats {
        ServerStats {
            accepted_connections: self.accepted.load(Ordering::Relaxed),
            active_connections: self.active.load(Ordering::Relaxed),
            requests: self.requests.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            shutdown: self.shutdown.load(Ordering::Acquire),
        }
    }
}

pub struct Server {
    listener: TcpListener,
    config: ServerConfig,
}

impl Server {
    pub fn bind(address: impl ToSocketAddrs) -> io::Result<Self> {
        Self::bind_with(address, ServerConfig::default())
    }

    pub fn bind_with(address: impl ToSocketAddrs, config: ServerConfig) -> io::Result<Self> {
        if config.max_workers == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an HTTP server needs at least one scheduler worker",
            ));
        }

        let listener = TcpListener::bind(address)?;
        Ok(Self { listener, config })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn run<H>(self, handler: H) -> io::Result<()>
    where
        H: Handler + 'static,
    {
        let scheduler = Scheduler::new(self.config.max_workers).map_err(io::Error::other)?;

        let _timer = zhs_mn::platform::raise(self.config.timer_resolution_millis);
        let reactor = Arc::new(Reactor::new()?);
        let state = Arc::new(ServerState::new());
        let address = self.listener.local_addr()?;

        let joins = start_acceptors(
            self.listener,
            self.config,
            Arc::new(handler),
            scheduler.clone(),
            Arc::clone(&reactor),
            Arc::clone(&state),
        )?;

        let mut result = Ok(());
        for join in joins {
            match join.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if result.is_ok() {
                        result = Err(error);
                    }
                    state.shutdown();
                    wake_listener(address);
                }
                Err(_) => {
                    if result.is_ok() {
                        result = Err(io::Error::other("an HTTP accept thread panicked"));
                    }
                    state.shutdown();
                    wake_listener(address);
                }
            }
        }

        reactor.shutdown();
        scheduler.shutdown();
        result
    }

    pub fn spawn<H>(self, handler: H) -> io::Result<ServerHandle>
    where
        H: Handler + 'static,
    {
        self.spawn_with_source(handler, ServerStatsSource::new())
    }

    pub fn spawn_with_source<H>(
        self,
        handler: H,
        source: ServerStatsSource,
    ) -> io::Result<ServerHandle>
    where
        H: Handler + 'static,
    {
        let local_addr = self.listener.local_addr()?;
        let scheduler = Scheduler::new(self.config.max_workers).map_err(io::Error::other)?;

        let timer = zhs_mn::platform::raise(self.config.timer_resolution_millis);
        let reactor = Arc::new(Reactor::new()?);
        let state = source.state;

        let shutdown_timeout = self.config.shutdown_timeout;
        let acceptors = self.config.acceptors.max(1);

        let joins = start_acceptors(
            self.listener,
            self.config,
            Arc::new(handler),
            scheduler.clone(),
            Arc::clone(&reactor),
            Arc::clone(&state),
        )?;

        Ok(ServerHandle {
            local_addr,
            acceptors,
            state,
            scheduler,
            reactor,
            shutdown_timeout,
            join: Mutex::new(Some(joins)),
            _timer: timer,
        })
    }
}

impl fmt::Debug for Server {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Server")
            .field("local_addr", &self.listener.local_addr().ok())
            .field("max_workers", &self.config.max_workers)
            .finish()
    }
}

pub struct ServerHandle {
    local_addr: SocketAddr,
    acceptors: usize,
    state: Arc<ServerState>,
    scheduler: Scheduler,
    reactor: Arc<Reactor>,
    shutdown_timeout: Duration,
    join: Mutex<Option<Vec<thread::JoinHandle<io::Result<()>>>>>,

    _timer: Option<TimerResolution>,
}

impl ServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn stats(&self) -> ServerStats {
        self.state.stats()
    }

    pub fn is_shutdown(&self) -> bool {
        self.state.is_shutdown()
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.state.shutdown();
        self.wake_acceptors();

        let result = self.join_accept_threads();

        self.reactor.shutdown();
        self.wait_for_connections();
        self.scheduler.shutdown();

        result
    }

    fn wake_acceptors(&self) {
        for _ in 0..self.acceptors {
            wake_listener(self.local_addr);
        }
    }

    fn join_accept_threads(&self) -> io::Result<()> {
        let joins = {
            let mut slot = self
                .join
                .lock()
                .unwrap_or_else(crate::internal::recover_poison);
            slot.take()
        };

        let Some(joins) = joins else {
            return Ok(());
        };

        let mut result = Ok(());
        for join in joins {
            match join.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if result.is_ok() {
                        result = Err(error);
                    }
                }
                Err(_) => {
                    if result.is_ok() {
                        result = Err(io::Error::other("an HTTP accept thread panicked"));
                    }
                }
            }
        }

        result
    }

    fn wait_for_connections(&self) {
        let deadline = Instant::now() + self.shutdown_timeout;

        while self.state.active_connections() > 0 {
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.state.shutdown();
        self.wake_acceptors();

        let slot = match self.join.get_mut() {
            Ok(slot) => slot,
            Err(poisoned) => crate::internal::recover_poison(poisoned),
        };
        if let Some(joins) = slot.take() {
            for join in joins {
                let _ = join.join();
            }
        }

        self.reactor.shutdown();
        self.scheduler.shutdown();
    }
}

impl fmt::Debug for ServerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerHandle")
            .field("local_addr", &self.local_addr)
            .field("stats", &self.stats())
            .finish()
    }
}

struct ConnectionGuard(Arc<ServerState>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.connection_finished();
    }
}

fn start_acceptors<H>(
    listener: TcpListener,
    config: ServerConfig,
    handler: Arc<H>,
    scheduler: Scheduler,
    reactor: Arc<Reactor>,
    state: Arc<ServerState>,
) -> io::Result<Vec<thread::JoinHandle<io::Result<()>>>>
where
    H: Handler + 'static,
{
    let acceptors = config.acceptors.max(1);
    let mut joins = Vec::with_capacity(acceptors);

    for index in 0..acceptors {
        let thread_listener = listener.try_clone()?;
        let thread_config = config.clone();
        let thread_handler = Arc::clone(&handler);
        let thread_scheduler = scheduler.clone();
        let thread_reactor = Arc::clone(&reactor);
        let thread_state = Arc::clone(&state);

        let join = thread::Builder::new()
            .name(format!("zhs-http-accept-{index}"))
            .spawn(move || {
                run_accept_loop(
                    thread_listener,
                    thread_config,
                    thread_handler,
                    thread_scheduler,
                    thread_reactor,
                    thread_state,
                )
            })?;

        joins.push(join);
    }

    Ok(joins)
}

fn run_accept_loop<H>(
    listener: TcpListener,
    config: ServerConfig,
    handler: Arc<H>,
    scheduler: Scheduler,
    reactor: Arc<Reactor>,
    state: Arc<ServerState>,
) -> io::Result<()>
where
    H: Handler + 'static,
{
    while !state.is_shutdown() {
        match listener.accept() {
            Ok((stream, peer)) => {
                if state.is_shutdown() {
                    break;
                }

                if config.max_connections > 0
                    && state.active_connections() >= config.max_connections
                {
                    reject_connection(&stream, config.write_timeout);
                    state.record_failure();
                    continue;
                }

                state.connection_started();
                let guard = ConnectionGuard(Arc::clone(&state));
                let handler = Arc::clone(&handler);
                let task_scheduler = scheduler.clone();
                let task_reactor = Arc::clone(&reactor);
                let task_config = config.clone();
                let task_state = Arc::clone(&state);

                let task = async move {
                    let _guard = guard;
                    connection::serve(
                        stream,
                        peer,
                        handler.as_ref(),
                        &task_config,
                        &task_scheduler,
                        &task_reactor,
                        &task_state,
                    )
                    .await;
                };

                if scheduler.try_spawn(task).is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                state.record_failure();
                return Err(error);
            }
        }
    }

    Ok(())
}

fn reject_connection(stream: &TcpStream, timeout: Duration) {
    use std::io::{Read, Write};

    let _ = stream.set_write_timeout(Some(timeout));
    let _ = (&*stream).write_all(
        b"HTTP/1.1 503 Service Unavailable\r\n\
          Content-Length: 0\r\n\
          Connection: close\r\n\
          Retry-After: 1\r\n\r\n",
    );
    let _ = (&*stream).flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let mut sink = [0_u8; 1024];
    let _ = (&*stream).read(&mut sink);
}

fn wake_listener(address: SocketAddr) {
    let target = match address {
        SocketAddr::V4(address) if address.ip().is_unspecified() => {
            SocketAddr::from(([127, 0, 0, 1], address.port()))
        }
        SocketAddr::V6(address) if address.ip().is_unspecified() => {
            SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], address.port()))
        }
        address => address,
    };

    for _ in 0..3 {
        if TcpStream::connect_timeout(&target, Duration::from_millis(200)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
