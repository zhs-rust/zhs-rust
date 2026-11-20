#![deny(unsafe_code)]

#[cfg(windows)]
#[allow(clippy::module_inception)]
mod platform;
#[cfg(not(windows))]
#[allow(clippy::module_inception)]
mod platform;

use std::fmt;

pub fn default_granularity_millis() -> f64 {
    platform::DEFAULT_GRANULARITY_MILLIS
}

pub fn raise(millis: u32) -> Option<TimerResolution> {
    if millis == 0 {
        return None;
    }
    if !platform::begin(millis) {
        return None;
    }
    Some(TimerResolution { millis })
}

#[must_use = "守卫丢弃即恢复定时器分辨率，应立即持有到不再需要低延迟为止"]
pub struct TimerResolution {
    millis: u32,
}

impl TimerResolution {
    pub fn millis(&self) -> u32 {
        self.millis
    }
}

impl fmt::Debug for TimerResolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TimerResolution")
            .field("millis", &self.millis)
            .finish()
    }
}

impl Drop for TimerResolution {
    fn drop(&mut self) {
        platform::end(self.millis);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_means_keep_the_system_default() {
        assert!(raise(0).is_none());
    }

    #[test]
    fn raise_matches_the_platform() {
        let guard = raise(1);
        #[cfg(windows)]
        {
            let guard = guard.expect("timeBeginPeriod should succeed on Windows");
            assert_eq!(guard.millis(), 1);
        }
        #[cfg(not(windows))]
        assert!(guard.is_none());
    }
}
