use std::fmt;
use std::time::Duration;

use super::source::SourceKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthState {
    Healthy,

    Degraded,

    Stalled,

    Shutdown,
}

impl HealthState {
    pub fn is_healthy(self) -> bool {
        matches!(self, HealthState::Healthy | HealthState::Shutdown)
    }

    pub(crate) fn severity(self) -> u8 {
        match self {
            HealthState::Healthy | HealthState::Shutdown => 0,
            HealthState::Degraded => 1,
            HealthState::Stalled => 2,
        }
    }
}

impl fmt::Display for HealthState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            HealthState::Healthy => "healthy",
            HealthState::Degraded => "degraded",
            HealthState::Stalled => "stalled",
            HealthState::Shutdown => "shutdown",
        })
    }
}

#[derive(Clone, Debug)]
pub struct SourceHealth {
    pub name: String,

    pub kind: SourceKind,

    pub state: HealthState,

    pub stalled_ticks: u32,

    pub workers: usize,

    pub idle_workers: usize,

    pub queued: usize,

    pub running: usize,

    pub pending: usize,

    pub capacity: usize,

    pub progress: u64,

    pub shutdown: bool,
}

#[derive(Clone, Debug)]
pub struct WatchdogReport {
    pub sequence: u64,

    pub elapsed: Duration,

    pub state: HealthState,

    pub sources: Vec<SourceHealth>,
}

impl WatchdogReport {
    pub fn is_healthy(&self) -> bool {
        self.state == HealthState::Healthy
    }

    pub fn is_stalled(&self) -> bool {
        self.state == HealthState::Stalled
    }

    pub fn source(&self, name: &str) -> Option<&SourceHealth> {
        self.sources.iter().find(|source| source.name == name)
    }
}
