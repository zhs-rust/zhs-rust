use std::fmt;
use std::io::{self, Write};

use super::{LogError, LogFormatter, LogLevel, LogRecord, LogSink, TextFormatter};

pub struct ConsoleSink {
    formatter: Box<dyn LogFormatter>,
    color: bool,
    flush: bool,
}

impl ConsoleSink {
    pub fn new() -> Self {
        Self {
            formatter: Box::new(TextFormatter::new()),
            color: false,
            flush: true,
        }
    }

    pub fn with_formatter(formatter: impl LogFormatter + 'static) -> Self {
        Self {
            formatter: Box::new(formatter),
            color: false,
            flush: true,
        }
    }

    pub fn color(&self) -> bool {
        self.color
    }

    pub fn flush(&self) -> bool {
        self.flush
    }

    pub fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    pub fn with_flush(mut self, flush: bool) -> Self {
        self.flush = flush;
        self
    }
}

impl Default for ConsoleSink {
    fn default() -> Self {
        Self::new()
    }
}

impl LogSink for ConsoleSink {
    fn write(&self, record: &LogRecord) -> Result<(), LogError> {
        let line = self.formatter.format(record);
        let color = self.color.then(|| color_for(record.level()));

        if record.level() >= LogLevel::Warning {
            write_line(io::stderr().lock(), &line, color, self.flush)
        } else {
            write_line(io::stdout().lock(), &line, color, self.flush)
        }
        .map_err(LogError::write_console)
    }

    fn flush(&self) -> Result<(), LogError> {
        io::stdout()
            .lock()
            .flush()
            .and_then(|()| io::stderr().lock().flush())
            .map_err(LogError::write_console)
    }
}

impl fmt::Debug for ConsoleSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleSink")
            .field("color", &self.color)
            .field("flush", &self.flush)
            .finish_non_exhaustive()
    }
}

fn write_line(
    mut stream: impl Write,
    line: &str,
    color: Option<&str>,
    flush: bool,
) -> io::Result<()> {
    match color {
        Some(color) => writeln!(stream, "{color}{line}\x1b[0m")?,
        None => writeln!(stream, "{line}")?,
    }

    if flush {
        stream.flush()?;
    }

    Ok(())
}

const fn color_for(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace => "\x1b[90m",
        LogLevel::Debug => "\x1b[36m",
        LogLevel::Information => "\x1b[32m",
        LogLevel::Warning => "\x1b[33m",
        LogLevel::Error => "\x1b[31m",
        LogLevel::Critical => "\x1b[35m",
    }
}
