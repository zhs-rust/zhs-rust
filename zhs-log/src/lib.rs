mod error;
pub mod runtime;
pub use error::LogError;

use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Information,
    Warning,
    Error,
    Critical,
}

impl LogLevel {
    pub const ALL: [LogLevel; 6] = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Information,
        LogLevel::Warning,
        LogLevel::Error,
        LogLevel::Critical,
    ];

    #[allow(non_upper_case_globals)]
    pub const Info: LogLevel = LogLevel::Information;
    #[allow(non_upper_case_globals)]
    pub const Warn: LogLevel = LogLevel::Warning;

    pub const fn is_enabled(self, threshold: LogLevel) -> bool {
        self as u8 >= threshold as u8
    }

    pub fn parse(value: &str) -> Option<Self> {
        value.parse().ok()
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Information => "INFO",
            LogLevel::Warning => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRITICAL",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for LogLevel {
    type Err = ();

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_ascii_lowercase().as_str() {
            "trace" => Ok(LogLevel::Trace),
            "debug" => Ok(LogLevel::Debug),
            "info" | "information" => Ok(LogLevel::Information),
            "warn" | "warning" => Ok(LogLevel::Warning),
            "error" => Ok(LogLevel::Error),
            "critical" | "fatal" => Ok(LogLevel::Critical),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Log {
    level: LogLevel,
    category: String,
}

impl Log {
    pub fn new(level: LogLevel, category: impl Into<String>) -> Self {
        Self {
            level,
            category: category.into(),
        }
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn log(&self, level: LogLevel, message: impl fmt::Display) {
        if level < self.level {
            return;
        }

        let message = message.to_string();
        eprintln!(
            "[{}] {}: {}",
            level,
            escape_controls(&self.category),
            escape_controls(&message)
        );
    }

    pub fn trace(&self, message: impl fmt::Display) {
        self.log(LogLevel::Trace, message);
    }

    pub fn debug(&self, message: impl fmt::Display) {
        self.log(LogLevel::Debug, message);
    }

    pub fn info(&self, message: impl fmt::Display) {
        self.log(LogLevel::Information, message);
    }

    pub fn warn(&self, message: impl fmt::Display) {
        self.log(LogLevel::Warning, message);
    }

    pub fn error(&self, message: impl fmt::Display) {
        self.log(LogLevel::Error, message);
    }

    pub fn critical(&self, message: impl fmt::Display) {
        self.log(LogLevel::Critical, message);
    }
}

fn escape_controls(value: &str) -> String {
    let mut output = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => output.push(character),
        }
    }

    output
}

#[derive(Debug, Clone)]
pub struct LoggerFactory {
    level: LogLevel,
}

impl LoggerFactory {
    pub fn new(level: LogLevel) -> Self {
        Self { level }
    }

    pub fn level(&self) -> LogLevel {
        self.level
    }

    pub fn logger<T>(&self) -> Logger<T> {
        Logger {
            inner: Log::new(self.level, std::any::type_name::<T>()),
            _marker: PhantomData,
        }
    }
}

pub struct Logger<T> {
    inner: Log,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Logger<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> Logger<T> {
    pub fn from_factory(factory: &LoggerFactory) -> Self {
        factory.logger::<T>()
    }

    pub fn inner(&self) -> &Log {
        &self.inner
    }

    pub fn trace(&self, message: impl fmt::Display) {
        self.inner.trace(message);
    }

    pub fn debug(&self, message: impl fmt::Display) {
        self.inner.debug(message);
    }

    pub fn info(&self, message: impl fmt::Display) {
        self.inner.info(message);
    }

    pub fn warn(&self, message: impl fmt::Display) {
        self.inner.warn(message);
    }

    pub fn error(&self, message: impl fmt::Display) {
        self.inner.error(message);
    }

    pub fn critical(&self, message: impl fmt::Display) {
        self.inner.critical(message);
    }
}

#[derive(Debug, Clone)]
pub struct LoggingBuilder {
    level: LogLevel,
}

impl Default for LoggingBuilder {
    fn default() -> Self {
        Self {
            level: LogLevel::Information,
        }
    }
}

impl LoggingBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_minimum_level(&mut self, level: LogLevel) -> &mut Self {
        self.level = level;
        self
    }

    pub fn minimum_level(&self) -> LogLevel {
        self.level
    }

    pub fn build(&self) -> LoggerFactory {
        LoggerFactory::new(self.level)
    }
}

#[cfg(test)]
mod macro_tests {
    #[test]
    fn log_macros_expand() {
        crate::log_trace!("trace");
        crate::log_debug!("debug");
        crate::log_info!("info");
        crate::log_warn!("warn");
        crate::log_error!("error");
    }
}
