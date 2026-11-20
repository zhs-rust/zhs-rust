use std::error::Error as StdError;
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogError {
    Open {
        path: PathBuf,

        message: String,
    },

    Write {
        path: Option<PathBuf>,

        message: String,
    },

    AlreadyInitialized,
}

impl LogError {
    pub(crate) fn open(path: impl Into<PathBuf>, error: impl fmt::Display) -> Self {
        Self::Open {
            path: path.into(),
            message: error.to_string(),
        }
    }

    pub(crate) fn write_file(path: impl Into<PathBuf>, error: impl fmt::Display) -> Self {
        Self::Write {
            path: Some(path.into()),
            message: error.to_string(),
        }
    }

    pub(crate) fn write_console(error: impl fmt::Display) -> Self {
        Self::Write {
            path: None,
            message: error.to_string(),
        }
    }
}

impl fmt::Display for LogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::Open { path, message } => {
                write!(
                    formatter,
                    "failed to open the log file {}: {message}",
                    path.display()
                )
            }
            LogError::Write {
                path: None,
                message,
            } => {
                write!(formatter, "failed to write the log record: {message}")
            }
            LogError::Write {
                path: Some(path),
                message,
            } => {
                write!(
                    formatter,
                    "failed to write the log file {}: {message}",
                    path.display()
                )
            }
            LogError::AlreadyInitialized => {
                formatter.write_str("the global logger is already initialized")
            }
        }
    }
}

impl StdError for LogError {}
