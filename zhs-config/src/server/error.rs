use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    Read {
        path: PathBuf,

        kind: io::ErrorKind,

        message: String,
    },

    Parse {
        path: PathBuf,

        message: String,
    },

    InvalidRoot {
        source: String,

        type_name: &'static str,
    },

    MissingSection {
        section: String,
    },

    InvalidValue {
        key: String,

        expected: &'static str,

        found: &'static str,
    },

    InvalidOption {
        key: String,

        value: String,

        expected: &'static str,
    },

    Bind {
        section: String,

        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read {
                path,
                kind,
                message,
            } => write!(
                formatter,
                "failed to read the configuration file {} ({kind:?}): {message}",
                path.display()
            ),
            ConfigError::Parse { path, message } => write!(
                formatter,
                "failed to parse the configuration file {}: {message}",
                path.display()
            ),
            ConfigError::InvalidRoot { source, type_name } => write!(
                formatter,
                "configuration source `{source}` must be a JSON object, found {type_name}"
            ),
            ConfigError::MissingSection { section } => {
                write!(formatter, "configuration section `{section}` is missing")
            }
            ConfigError::InvalidValue {
                key,
                expected,
                found,
            } => write!(
                formatter,
                "configuration key `{key}` must be {expected}, found {found}"
            ),
            ConfigError::InvalidOption {
                key,
                value,
                expected,
            } => write!(
                formatter,
                "configuration key `{key}` has unsupported value `{value}`, expected {expected}"
            ),
            ConfigError::Bind { section, message } => write!(
                formatter,
                "failed to bind configuration section `{section}`: {message}"
            ),
        }
    }
}

impl StdError for ConfigError {}
