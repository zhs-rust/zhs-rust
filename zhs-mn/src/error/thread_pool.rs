use std::error::Error as StdError;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadPoolError {
    ZeroWorkers,

    MinGreaterThanMax { min: usize, max: usize },

    WorkerSpawn { worker: usize, message: String },

    Closed,
}

impl ThreadPoolError {
    pub(crate) fn worker_spawn(worker: usize, error: impl fmt::Display) -> Self {
        Self::WorkerSpawn {
            worker,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for ThreadPoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThreadPoolError::ZeroWorkers => {
                formatter.write_str("a thread pool needs at least one worker thread")
            }
            ThreadPoolError::MinGreaterThanMax { min, max } => write!(
                formatter,
                "min_workers ({min}) must not exceed max_workers ({max})"
            ),
            ThreadPoolError::WorkerSpawn { worker, message } => write!(
                formatter,
                "failed to start thread pool worker {worker}: {message}"
            ),
            ThreadPoolError::Closed => formatter.write_str("the thread pool is closed"),
        }
    }
}

impl StdError for ThreadPoolError {}
