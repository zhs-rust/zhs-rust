use std::error::Error as StdError;
use std::fmt;

use crate::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonErrorKind {
    UnexpectedEnd,

    ExpectedValue,

    InvalidLiteral,

    InvalidNumber,

    NumberOutOfRange,

    ControlCharacterInString(char),

    InvalidEscape(char),

    InvalidUnicodeEscape,

    LoneSurrogate,

    ExpectedObjectKey,

    ExpectedColon,

    ExpectedCommaOrEnd,

    TrailingCharacters,

    RecursionLimitExceeded,

    InvalidUtf8,
}

impl fmt::Display for JsonErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonErrorKind::UnexpectedEnd => formatter.write_str("unexpected end of input"),
            JsonErrorKind::ExpectedValue => formatter.write_str("expected a JSON value"),
            JsonErrorKind::InvalidLiteral => {
                formatter.write_str("invalid literal; expected 'true', 'false' or 'null'")
            }
            JsonErrorKind::InvalidNumber => formatter.write_str("invalid number"),
            JsonErrorKind::NumberOutOfRange => formatter.write_str("number out of range"),
            JsonErrorKind::ControlCharacterInString(character) => {
                write!(
                    formatter,
                    "unescaped control character {character:?} in string"
                )
            }
            JsonErrorKind::InvalidEscape(character) => {
                write!(formatter, "invalid escape sequence '\\{character}'")
            }
            JsonErrorKind::InvalidUnicodeEscape => {
                formatter.write_str("invalid \\u escape; expected four hexadecimal digits")
            }
            JsonErrorKind::LoneSurrogate => {
                formatter.write_str("unpaired UTF-16 surrogate in \\u escape")
            }
            JsonErrorKind::ExpectedObjectKey => formatter.write_str("expected a string object key"),
            JsonErrorKind::ExpectedColon => formatter.write_str("expected ':' after object key"),
            JsonErrorKind::ExpectedCommaOrEnd => {
                formatter.write_str("expected ',' or the closing bracket")
            }
            JsonErrorKind::TrailingCharacters => {
                formatter.write_str("trailing characters after the JSON value")
            }
            JsonErrorKind::RecursionLimitExceeded => {
                formatter.write_str("maximum nesting depth exceeded")
            }
            JsonErrorKind::InvalidUtf8 => formatter.write_str("input is not valid UTF-8"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    kind: JsonErrorKind,
    index: usize,
    line: usize,
    column: usize,
}

impl JsonError {
    pub(crate) fn at(kind: JsonErrorKind, index: usize, source: &str) -> Self {
        let bytes = source.as_bytes();
        let index = index.min(bytes.len());
        let line = 1 + bytes[..index].iter().filter(|&&byte| byte == b'\n').count();
        let line_start = bytes[..index]
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |position| position + 1);

        let column = if source.is_char_boundary(index) {
            let start = source[..index]
                .rfind('\n')
                .map_or(0, |position| position + 1);
            1 + source[start..index].chars().count()
        } else {
            1 + index - line_start
        };
        Self {
            kind,
            index,
            line,
            column,
        }
    }

    pub(crate) fn at_bytes(kind: JsonErrorKind, index: usize, bytes: &[u8]) -> Self {
        let index = index.min(bytes.len());
        let line = 1 + bytes[..index].iter().filter(|&&byte| byte == b'\n').count();
        let line_start = bytes[..index]
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |position| position + 1);
        let column = 1 + index - line_start;
        Self {
            kind,
            index,
            line,
            column,
        }
    }

    pub(crate) fn from_utf8(error: std::str::Utf8Error, bytes: &[u8]) -> Self {
        Self::at_bytes(JsonErrorKind::InvalidUtf8, error.valid_up_to(), bytes)
    }

    pub fn kind(&self) -> JsonErrorKind {
        self.kind
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn line(&self) -> usize {
        self.line
    }

    pub fn column(&self) -> usize {
        self.column
    }
}

impl fmt::Display for JsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "JSON error at line {}, column {}: {}",
            self.line, self.column, self.kind
        )
    }
}

impl StdError for JsonError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonDecodeError {
    path: String,
    message: String,
}

impl JsonDecodeError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn expected(expected: &str, value: &Value) -> Self {
        Self::new(
            "$",
            format!("expected {expected}, found {}", value.type_name()),
        )
    }

    pub fn missing(field: &str) -> Self {
        Self::new(format!("$.{field}"), format!("missing field `{field}`"))
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn prepend_field(mut self, field: &str) -> Self {
        let rest = self.path.strip_prefix('$').unwrap_or(&self.path);
        self.path = format!("$.{field}{rest}");
        self
    }

    pub fn prepend_index(mut self, index: usize) -> Self {
        let rest = self.path.strip_prefix('$').unwrap_or(&self.path);
        self.path = format!("$[{index}]{rest}");
        self
    }
}

impl fmt::Display for JsonDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "JSON decode error at {}: {}",
            self.path, self.message
        )
    }
}

impl StdError for JsonDecodeError {}

#[derive(Debug)]
pub enum Error {
    Json(JsonError),
    JsonDecode(JsonDecodeError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Json(error) => error.fmt(formatter),
            Error::JsonDecode(error) => error.fmt(formatter),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::Json(error) => Some(error),
            Error::JsonDecode(error) => Some(error),
        }
    }
}

impl From<JsonError> for Error {
    fn from(error: JsonError) -> Self {
        Error::Json(error)
    }
}

impl From<JsonDecodeError> for Error {
    fn from(error: JsonDecodeError) -> Self {
        Error::JsonDecode(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
