use std::error::Error as StdError;
use std::fmt;
use std::io;

use crate::internal::http::StatusCode;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpError {
    Incomplete,

    IdleTimeout,

    Timeout,

    InvalidRequestLine,

    RequestLineTooLong,

    InvalidTarget,

    UnsupportedVersion,

    InvalidHeader,

    InvalidHeaderValue,

    ObsoleteLineFolding,

    HeadTooLarge,

    TooManyHeaders,

    InvalidHost,

    InvalidContentLength,

    InvalidTransferEncoding,

    UnsupportedTransferEncoding,

    BodyTooLarge,

    InvalidBodyEncoding,

    InvalidChunk,

    UnsupportedExpectation,

    InvalidQuery,

    InvalidForm,

    UnsupportedContentType {
        actual: String,

        expected: &'static str,
    },

    InvalidField {
        field: String,

        reason: String,
    },

    InvalidMultipart {
        reason: String,
    },

    InvalidWebSocket {
        reason: String,
    },

    Io {
        kind: io::ErrorKind,

        message: String,
    },
}

impl HttpError {
    pub fn status(&self) -> StatusCode {
        match self {
            HttpError::Incomplete => StatusCode::BAD_REQUEST,
            HttpError::IdleTimeout | HttpError::Timeout => StatusCode::REQUEST_TIMEOUT,
            HttpError::InvalidRequestLine
            | HttpError::InvalidTarget
            | HttpError::InvalidHeader
            | HttpError::InvalidHeaderValue
            | HttpError::ObsoleteLineFolding
            | HttpError::InvalidHost
            | HttpError::InvalidContentLength
            | HttpError::InvalidTransferEncoding
            | HttpError::InvalidChunk => StatusCode::BAD_REQUEST,
            HttpError::RequestLineTooLong => StatusCode::URI_TOO_LONG,
            HttpError::UnsupportedVersion => StatusCode::HTTP_VERSION_NOT_SUPPORTED,
            HttpError::HeadTooLarge | HttpError::TooManyHeaders => {
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
            }
            HttpError::UnsupportedTransferEncoding => StatusCode::NOT_IMPLEMENTED,
            HttpError::BodyTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            HttpError::InvalidBodyEncoding
            | HttpError::InvalidQuery
            | HttpError::InvalidForm
            | HttpError::InvalidField { .. }
            | HttpError::InvalidMultipart { .. }
            | HttpError::InvalidWebSocket { .. } => StatusCode::BAD_REQUEST,
            HttpError::UnsupportedExpectation => StatusCode::EXPECTATION_FAILED,
            HttpError::UnsupportedContentType { .. } => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            HttpError::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub(crate) fn from_io(error: io::Error) -> Self {
        HttpError::Io {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Incomplete => {
                formatter.write_str("the connection closed before the request was complete")
            }
            HttpError::IdleTimeout => formatter.write_str("timed out while waiting for a request"),
            HttpError::Timeout => formatter.write_str("timed out while reading the request"),
            HttpError::InvalidRequestLine => formatter.write_str("invalid HTTP request line"),
            HttpError::RequestLineTooLong => formatter.write_str("HTTP request line is too long"),
            HttpError::InvalidTarget => formatter.write_str("invalid HTTP request target"),
            HttpError::UnsupportedVersion => formatter.write_str("unsupported HTTP version"),
            HttpError::InvalidHeader => formatter.write_str("invalid HTTP header line"),
            HttpError::InvalidHeaderValue => formatter.write_str("invalid HTTP header field value"),
            HttpError::ObsoleteLineFolding => {
                formatter.write_str("obsolete HTTP header line folding is not accepted")
            }
            HttpError::HeadTooLarge => formatter.write_str("HTTP request head is too large"),
            HttpError::TooManyHeaders => formatter.write_str("too many HTTP header fields"),
            HttpError::InvalidHost => {
                formatter.write_str("an HTTP/1.1 request requires exactly one Host header")
            }
            HttpError::InvalidContentLength => {
                formatter.write_str("invalid or conflicting Content-Length header")
            }
            HttpError::InvalidTransferEncoding => {
                formatter.write_str("invalid Transfer-Encoding header")
            }
            HttpError::UnsupportedTransferEncoding => {
                formatter.write_str("unsupported transfer coding")
            }
            HttpError::BodyTooLarge => formatter.write_str("HTTP request body is too large"),
            HttpError::InvalidBodyEncoding => {
                formatter.write_str("HTTP request body is not valid UTF-8")
            }
            HttpError::InvalidChunk => formatter.write_str("invalid chunked request body"),
            HttpError::UnsupportedExpectation => formatter.write_str("unsupported Expect header"),
            HttpError::InvalidQuery => {
                formatter.write_str("invalid percent-encoding in the query string")
            }
            HttpError::InvalidForm => {
                formatter.write_str("invalid percent-encoding in the form body")
            }
            HttpError::UnsupportedContentType { actual, expected } => {
                if actual.is_empty() {
                    write!(formatter, "a Content-Type of {expected} is required")
                } else {
                    write!(
                        formatter,
                        "unsupported Content-Type {actual:?} (expected {expected})"
                    )
                }
            }
            HttpError::InvalidField { field, reason } => {
                write!(formatter, "invalid field {field:?}: {reason}")
            }
            HttpError::InvalidMultipart { reason } => {
                write!(formatter, "invalid multipart body: {reason}")
            }
            HttpError::InvalidWebSocket { reason } => {
                write!(formatter, "invalid WebSocket: {reason}")
            }
            HttpError::Io { kind, message } => {
                write!(formatter, "HTTP I/O error ({kind:?}): {message}")
            }
        }
    }
}

impl StdError for HttpError {}

impl From<io::Error> for HttpError {
    fn from(error: io::Error) -> Self {
        HttpError::from_io(error)
    }
}
