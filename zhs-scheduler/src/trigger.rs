use std::time::{Duration, SystemTime};

use crate::{CronExpression, SchedulerError};

#[derive(Debug, Clone)]
pub enum Trigger {
    Simple(SimpleTrigger),
    Cron(CronTrigger),
}

impl Trigger {
    pub fn next_after(&self, after: SystemTime, fire_count: u64) -> Option<SystemTime> {
        match self {
            Trigger::Simple(trigger) => trigger.next_after(after, fire_count),
            Trigger::Cron(trigger) => trigger.next_after(after),
        }
    }

    pub fn remaining_after(&self, fire_count: u64) -> Option<u64> {
        match self {
            Trigger::Simple(trigger) => trigger.remaining_after(fire_count),
            Trigger::Cron(_) => None,
        }
    }
}

impl From<SimpleTrigger> for Trigger {
    fn from(trigger: SimpleTrigger) -> Self {
        Trigger::Simple(trigger)
    }
}

impl From<CronTrigger> for Trigger {
    fn from(trigger: CronTrigger) -> Self {
        Trigger::Cron(trigger)
    }
}

#[derive(Debug, Clone)]
pub struct SimpleTrigger {
    interval: Duration,
    repeat: Option<u64>,
    start_at: Option<SystemTime>,
}

impl SimpleTrigger {
    pub fn every(interval: Duration) -> Self {
        Self {
            interval,
            repeat: None,
            start_at: None,
        }
    }

    pub fn repeat(mut self, repeat: u64) -> Self {
        self.repeat = Some(repeat);
        self
    }

    pub fn start_at(mut self, start_at: SystemTime) -> Self {
        self.start_at = Some(start_at);
        self
    }

    fn next_after(&self, after: SystemTime, _fire_count: u64) -> Option<SystemTime> {
        if self.interval.is_zero() {
            return None;
        }

        let anchor = self.start_at.unwrap_or(after);
        if anchor > after {
            return Some(anchor);
        }

        let elapsed = after.duration_since(anchor).ok()?;
        let steps = elapsed.as_nanos() / self.interval.as_nanos();
        let next_steps = steps.checked_add(1)?;

        let total_nanos = self.interval.as_nanos().checked_mul(next_steps)?;
        let secs = total_nanos / 1_000_000_000;
        let nanos = (total_nanos % 1_000_000_000) as u32;
        let secs = u64::try_from(secs).ok()?;
        let offset = Duration::new(secs, nanos);

        anchor.checked_add(offset)
    }

    fn remaining_after(&self, fire_count: u64) -> Option<u64> {
        self.repeat
            .map(|total| total.saturating_add(1).saturating_sub(fire_count))
    }
}

#[derive(Debug, Clone)]
pub struct CronTrigger {
    expression: CronExpression,
}

impl CronTrigger {
    pub fn parse(expression: &str) -> Result<Self, SchedulerError> {
        Ok(Self {
            expression: CronExpression::parse(expression)?,
        })
    }

    pub fn new(expression: CronExpression) -> Self {
        Self { expression }
    }

    fn next_after(&self, after: SystemTime) -> Option<SystemTime> {
        self.expression.next_after(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn simple_trigger_advances_by_interval() {
        let trigger = SimpleTrigger::every(Duration::from_secs(10));

        assert_eq!(
            trigger.next_after(UNIX_EPOCH, 0),
            Some(UNIX_EPOCH + Duration::from_secs(10))
        );
    }

    #[test]
    fn simple_trigger_counts_remaining_fires() {
        let trigger = SimpleTrigger::every(Duration::from_secs(1)).repeat(2);

        assert_eq!(trigger.remaining_after(0), Some(3));
        assert_eq!(trigger.remaining_after(2), Some(1));
        assert_eq!(trigger.remaining_after(3), Some(0));
    }

    #[test]
    fn cron_trigger_parses() {
        assert!(CronTrigger::parse("0 0 3 * * ?").is_ok());
    }
}
