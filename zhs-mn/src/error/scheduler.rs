use std::any::Any;
use std::error::Error as StdError;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchedulerError {
    ZeroWorkers,

    WorkerSpawn { worker: usize, message: String },

    TimerSpawn { message: String },

    Closed,
}

impl SchedulerError {
    pub(crate) fn worker_spawn(worker: usize, error: impl fmt::Display) -> Self {
        Self::WorkerSpawn {
            worker,
            message: error.to_string(),
        }
    }

    pub(crate) fn timer_spawn(error: impl fmt::Display) -> Self {
        Self::TimerSpawn {
            message: error.to_string(),
        }
    }
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchedulerError::ZeroWorkers => {
                formatter.write_str("a scheduler needs at least one worker")
            }
            SchedulerError::WorkerSpawn { worker, message } => {
                write!(
                    formatter,
                    "failed to start scheduler worker {worker}: {message}"
                )
            }
            SchedulerError::TimerSpawn { message } => {
                write!(formatter, "failed to start the scheduler timer: {message}")
            }
            SchedulerError::Closed => formatter.write_str("the scheduler is closed"),
        }
    }
}

impl StdError for SchedulerError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeoutError;

impl fmt::Display for TimeoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("deadline has elapsed")
    }
}

impl StdError for TimeoutError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinError {
    Cancelled,

    Panicked { message: Option<String> },
}

impl JoinError {
    pub(crate) fn panicked(payload: Box<dyn Any + Send>) -> Self {
        let message = if let Some(message) = payload.downcast_ref::<&'static str>() {
            Some((*message).to_owned())
        } else {
            payload.downcast_ref::<String>().cloned()
        };

        Self::Panicked { message }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, JoinError::Cancelled)
    }

    pub fn is_panic(&self) -> bool {
        matches!(self, JoinError::Panicked { .. })
    }

    pub fn panic_message(&self) -> Option<&str> {
        match self {
            JoinError::Panicked { message } => message.as_deref(),
            JoinError::Cancelled => None,
        }
    }
}

impl fmt::Display for JoinError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinError::Cancelled => formatter.write_str("task was cancelled"),
            JoinError::Panicked {
                message: Some(message),
            } => {
                write!(formatter, "task panicked: {message}")
            }
            JoinError::Panicked { message: None } => formatter.write_str("task panicked"),
        }
    }
}

impl StdError for JoinError {}
