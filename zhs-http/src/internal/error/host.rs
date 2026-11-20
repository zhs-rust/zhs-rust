use std::error::Error as StdError;
use std::fmt;
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostError {
    MissingRouter,

    Bind {
        address: String,

        kind: io::ErrorKind,

        message: String,
    },

    Server {
        kind: io::ErrorKind,

        message: String,
    },
}

impl HostError {
    pub fn bind(address: impl Into<String>, error: io::Error) -> Self {
        Self::Bind {
            address: address.into(),
            kind: error.kind(),
            message: error.to_string(),
        }
    }

    pub fn server(error: io::Error) -> Self {
        Self::Server {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::MissingRouter => {
                formatter.write_str("the host needs a router to serve requests")
            }
            HostError::Bind {
                address,
                kind,
                message,
            } => write!(
                formatter,
                "failed to bind the host address {address} ({kind:?}): {message}"
            ),
            HostError::Server { kind, message } => {
                write!(formatter, "the HTTP server failed ({kind:?}): {message}")
            }
        }
    }
}

impl StdError for HostError {}
