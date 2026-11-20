#[cfg(any(target_os = "linux", target_os = "android"))]
mod epoll;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) use epoll::{NAME, Poller};

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
mod poll;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
pub(crate) use poll::{NAME, Poller};

#[cfg(windows)]
mod wsa;
#[cfg(windows)]
pub(crate) use wsa::{NAME, Poller};

pub(crate) const KEEPS_READ_INTEREST: bool = cfg!(any(
    windows,
    all(unix, not(any(target_os = "linux", target_os = "android")))
));

#[cfg(not(any(unix, windows)))]
compile_error!(
    "zhs 的 HTTP reactor 需要 epoll（Linux）、WSAPoll（Windows）或 poll(2)（其他 Unix）"
);
