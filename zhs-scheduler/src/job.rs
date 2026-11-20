use std::sync::Arc;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JobKey {
    pub name: String,
    pub group: String,
}

impl JobKey {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            group: "default".to_owned(),
        }
    }

    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }
}

#[derive(Debug, Clone)]
pub struct JobContext {
    pub key: JobKey,
    pub scheduled_at: SystemTime,
    pub fire_time: SystemTime,
}

pub trait Job: Send + Sync {
    fn execute(&self, context: &JobContext) -> Result<(), super::SchedulerError>;
}

impl<F> Job for F
where
    F: Fn(&JobContext) -> Result<(), super::SchedulerError> + Send + Sync,
{
    fn execute(&self, context: &JobContext) -> Result<(), super::SchedulerError> {
        self(context)
    }
}

pub fn job_fn<F>(job: F) -> Arc<dyn Job>
where
    F: Fn(&JobContext) -> Result<(), super::SchedulerError> + Send + Sync + 'static,
{
    Arc::new(job)
}
