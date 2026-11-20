use std::error::Error as StdError;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchdogError {
    ZeroInterval,

    ThreadSpawn { message: String },

    Closed,
}

impl WatchdogError {
    pub(crate) fn thread_spawn(error: impl fmt::Display) -> Self {
        Self::ThreadSpawn {
            message: error.to_string(),
        }
    }
}

impl fmt::Display for WatchdogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchdogError::ZeroInterval => {
                formatter.write_str("the watchdog interval must be greater than zero")
            }
            WatchdogError::ThreadSpawn { message } => {
                write!(formatter, "failed to start the watchdog thread: {message}")
            }
            WatchdogError::Closed => formatter.write_str("the watchdog is closed"),
        }
    }
}

impl StdError for WatchdogError {}
