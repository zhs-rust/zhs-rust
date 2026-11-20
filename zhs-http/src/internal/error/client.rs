use std::error::Error as StdError;
use std::fmt;
use std::io;

use crate::internal::http::StatusCode;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientError {
    InvalidUrl {
        url: String,

        reason: String,
    },

    UnsupportedScheme {
        scheme: String,
    },

    InvalidHeader {
        name: String,

        reason: String,
    },

    Connect {
        host: String,

        port: u16,

        kind: io::ErrorKind,

        message: String,
    },

    Io {
        kind: io::ErrorKind,

        message: String,
    },

    Timeout {
        during: &'static str,
    },

    Incomplete,

    InvalidStatusLine,

    InvalidResponseHeader {
        reason: String,
    },

    HeadTooLarge,

    TooManyHeaders,

    BodyTooLarge {
        limit: usize,
    },

    InvalidChunk,

    UnsupportedTransferEncoding {
        coding: String,
    },

    TooManyRedirects {
        limit: usize,
    },

    Status {
        status: StatusCode,

        body: Vec<u8>,
    },

    InvalidBodyEncoding,
}

impl ClientError {
    pub fn status(&self) -> StatusCode {
        match self {
            ClientError::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            ClientError::Connect { .. }
            | ClientError::Io { .. }
            | ClientError::Incomplete
            | ClientError::InvalidStatusLine
            | ClientError::InvalidResponseHeader { .. }
            | ClientError::HeadTooLarge
            | ClientError::TooManyHeaders
            | ClientError::BodyTooLarge { .. }
            | ClientError::InvalidChunk
            | ClientError::UnsupportedTransferEncoding { .. }
            | ClientError::TooManyRedirects { .. }
            | ClientError::Status { .. } => StatusCode::BAD_GATEWAY,
            ClientError::InvalidUrl { .. }
            | ClientError::UnsupportedScheme { .. }
            | ClientError::InvalidHeader { .. }
            | ClientError::InvalidBodyEncoding => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub(crate) fn io(error: io::Error, during: &'static str) -> Self {
        match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => ClientError::Timeout { during },
            kind => ClientError::Io {
                kind,
                message: error.to_string(),
            },
        }
    }

    pub(crate) fn connect(host: &str, port: u16, error: io::Error) -> Self {
        if matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            return ClientError::Timeout { during: "connect" };
        }

        ClientError::Connect {
            host: host.to_owned(),
            port,
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::InvalidUrl { url, reason } => {
                write!(formatter, "invalid URL {url:?}: {reason}")
            }
            ClientError::UnsupportedScheme { scheme } => {
                write!(formatter, "unsupported URL scheme {scheme:?}")
            }
            ClientError::InvalidHeader { name, reason } => {
                write!(formatter, "invalid request header {name:?}: {reason}")
            }
            ClientError::Connect {
                host,
                port,
                kind,
                message,
            } => write!(
                formatter,
                "failed to connect to {host}:{port} ({kind:?}): {message}"
            ),
            ClientError::Io { kind, message } => {
                write!(formatter, "HTTP client I/O error ({kind:?}): {message}")
            }
            ClientError::Timeout { during } => {
                write!(formatter, "timed out while {during} the HTTP request")
            }
            ClientError::Incomplete => formatter
                .write_str("the server closed the connection before the response was complete"),
            ClientError::InvalidStatusLine => formatter.write_str("invalid HTTP status line"),
            ClientError::InvalidResponseHeader { reason } => {
                write!(formatter, "invalid HTTP response header: {reason}")
            }
            ClientError::HeadTooLarge => formatter.write_str("HTTP response head is too large"),
            ClientError::TooManyHeaders => formatter.write_str("too many HTTP response headers"),
            ClientError::BodyTooLarge { limit } => {
                write!(formatter, "HTTP response body exceeds {limit} bytes")
            }
            ClientError::InvalidChunk => formatter.write_str("invalid chunked response body"),
            ClientError::UnsupportedTransferEncoding { coding } => {
                write!(formatter, "unsupported transfer coding {coding:?}")
            }
            ClientError::TooManyRedirects { limit } => {
                write!(formatter, "more than {limit} redirects")
            }
            ClientError::Status { status, body } => write!(
                formatter,
                "the server returned {status} {} with a {} byte body",
                status.reason_phrase(),
                body.len()
            ),
            ClientError::InvalidBodyEncoding => {
                formatter.write_str("HTTP response body is not valid UTF-8")
            }
        }
    }
}

impl StdError for ClientError {}
