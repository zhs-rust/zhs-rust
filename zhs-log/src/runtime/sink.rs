use super::{LogError, LogRecord};

pub trait LogSink: Send + Sync {
    fn write(&self, record: &LogRecord) -> Result<(), LogError>;

    fn flush(&self) -> Result<(), LogError> {
        Ok(())
    }
}

impl<F> LogSink for F
where
    F: Fn(&LogRecord) -> Result<(), LogError> + Send + Sync,
{
    fn write(&self, record: &LogRecord) -> Result<(), LogError> {
        self(record)
    }
}
