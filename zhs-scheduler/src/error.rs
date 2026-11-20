use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    InvalidCron(String),
    InvalidTrigger(String),
    Closed,
    Job(String),
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchedulerError::InvalidCron(message) => write!(formatter, "invalid cron: {message}"),
            SchedulerError::InvalidTrigger(message) => {
                write!(formatter, "invalid trigger: {message}")
            }
            SchedulerError::Closed => formatter.write_str("the scheduler is closed"),
            SchedulerError::Job(message) => write!(formatter, "job failed: {message}"),
        }
    }
}

impl std::error::Error for SchedulerError {}
