mod console;
mod file;
mod format;
mod level;
mod record;
mod rotation;
mod sink;

use std::borrow::Cow;
use std::fmt;
use std::sync::{Arc, OnceLock};

pub use console::ConsoleSink;
pub use file::{DEFAULT_MAX_FILES, FileSink};
pub use format::{LogFormatter, TextFormatter};
pub use level::LogLevel;
pub use record::{LogRecord, Timestamp};
pub use rotation::Rotation;
pub use sink::LogSink;

use crate::LogError;

struct LoggerInner {
    level: LogLevel,
    sinks: Vec<Arc<dyn LogSink>>,
}

pub struct Logger {
    inner: Arc<LoggerInner>,
}

impl Logger {
    pub fn builder() -> LoggerBuilder {
        LoggerBuilder::new()
    }

    pub fn level(&self) -> LogLevel {
        self.inner.level
    }

    pub fn sink_count(&self) -> usize {
        self.inner.sinks.len()
    }

    pub fn enabled(&self, level: LogLevel) -> bool {
        level.is_enabled(self.inner.level)
    }

    pub fn log(&self, record: LogRecord) -> Result<(), LogError> {
        if !self.enabled(record.level()) {
            return Ok(());
        }

        let mut first_error = None;
        for sink in &self.inner.sinks {
            if let Err(error) = sink.write(&record)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn log_fmt(
        &self,
        level: LogLevel,
        target: impl Into<Cow<'static, str>>,
        arguments: fmt::Arguments<'_>,
    ) -> Result<(), LogError> {
        if !self.enabled(level) {
            return Ok(());
        }

        self.log(LogRecord::new(level, target, arguments.to_string()))
    }

    pub fn flush(&self) -> Result<(), LogError> {
        let mut first_error = None;
        for sink in &self.inner.sinks {
            if let Err(error) = sink.flush()
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Clone for Logger {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl fmt::Debug for Logger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Logger")
            .field("level", &self.inner.level)
            .field("sink_count", &self.inner.sinks.len())
            .finish()
    }
}

pub struct LoggerBuilder {
    level: LogLevel,
    sinks: Vec<Arc<dyn LogSink>>,
}

impl LoggerBuilder {
    pub fn new() -> Self {
        Self {
            level: LogLevel::Info,
            sinks: Vec::new(),
        }
    }

    pub fn with_level(mut self, level: LogLevel) -> Self {
        self.level = level;
        self
    }

    pub fn with_sink(mut self, sink: impl LogSink + 'static) -> Self {
        self.sinks.push(Arc::new(sink));
        self
    }

    pub fn with_shared_sink(mut self, sink: Arc<dyn LogSink>) -> Self {
        self.sinks.push(sink);
        self
    }

    pub fn build(self) -> Logger {
        Logger {
            inner: Arc::new(LoggerInner {
                level: self.level,
                sinks: self.sinks,
            }),
        }
    }
}

impl Default for LoggerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL: OnceLock<Logger> = OnceLock::new();

pub fn set_global(logger: Logger) -> Result<(), LogError> {
    GLOBAL.set(logger).map_err(|_| LogError::AlreadyInitialized)
}

pub fn global() -> Option<&'static Logger> {
    GLOBAL.get()
}

pub fn is_initialized() -> bool {
    GLOBAL.get().is_some()
}

#[macro_export]
macro_rules! log_with {
    ($level:expr, $logger:expr, target: $target:expr, $($arg:tt)+) => {{
        let logger = &$logger;
        if logger.enabled($level) {
            let _ = logger.log_fmt($level, $target, format_args!($($arg)+));
        }
    }};
    ($level:expr, $logger:expr, $($arg:tt)+) => {{
        let logger = &$logger;
        if logger.enabled($level) {
            let _ = logger.log_fmt($level, module_path!(), format_args!($($arg)+));
        }
    }};
    ($level:expr, target: $target:expr, $($arg:tt)+) => {{
        if let Some(logger) = $crate::runtime::global() {
            if logger.enabled($level) {
                let _ = logger.log_fmt($level, $target, format_args!($($arg)+));
            }
        }
    }};
    ($level:expr, $($arg:tt)+) => {{
        if let Some(logger) = $crate::runtime::global() {
            if logger.enabled($level) {
                let _ = logger.log_fmt($level, module_path!(), format_args!($($arg)+));
            }
        }
    }};
}

#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)+) => {
        $crate::log_with!($crate::runtime::LogLevel::Trace, $($arg)+)
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)+) => {
        $crate::log_with!($crate::runtime::LogLevel::Debug, $($arg)+)
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)+) => {
        $crate::log_with!($crate::runtime::LogLevel::Info, $($arg)+)
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)+) => {
        $crate::log_with!($crate::runtime::LogLevel::Warn, $($arg)+)
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)+) => {
        $crate::log_with!($crate::runtime::LogLevel::Error, $($arg)+)
    };
}
