use std::borrow::Cow;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use super::LogLevel;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    unix_seconds: u64,
    subsec_nanos: u32,
}

impl Timestamp {
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    pub fn from_system_time(time: SystemTime) -> Self {
        let elapsed = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        Self {
            unix_seconds: elapsed.as_secs(),
            subsec_nanos: elapsed.subsec_nanos(),
        }
    }

    pub fn unix_seconds(self) -> u64 {
        self.unix_seconds
    }

    pub fn subsec_nanos(self) -> u32 {
        self.subsec_nanos
    }

    pub fn format(self, utc_offset_seconds: i32) -> String {
        let offset = i64::from(utc_offset_seconds);
        let shifted = self.unix_seconds as i64 + offset;
        let days = shifted.div_euclid(86_400);
        let seconds_of_day = shifted.rem_euclid(86_400);

        let (year, month, day) = civil_from_days(days);
        let hour = seconds_of_day / 3_600;
        let minute = (seconds_of_day % 3_600) / 60;
        let second = seconds_of_day % 60;
        let millisecond = self.subsec_nanos / 1_000_000;

        let suffix = if utc_offset_seconds == 0 {
            "Z".to_owned()
        } else {
            let sign = if utc_offset_seconds < 0 { '-' } else { '+' };
            let absolute = offset.abs();
            format!(
                "{sign}{:02}:{:02}",
                absolute / 3_600,
                (absolute % 3_600) / 60
            )
        };

        format!(
            "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millisecond:03}{suffix}"
        )
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.format(0))
    }
}

pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[derive(Clone, Debug)]
pub struct LogRecord {
    timestamp: Timestamp,
    level: LogLevel,
    target: Cow<'static, str>,
    message: Cow<'static, str>,
    fields: Vec<(Cow<'static, str>, Cow<'static, str>)>,
}

impl LogRecord {
    pub fn new(
        level: LogLevel,
        target: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::at(Timestamp::now(), level, target, message)
    }

    pub fn at(
        timestamp: Timestamp,
        level: LogLevel,
        target: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            timestamp,
            level,
            target: target.into(),
            message: message.into(),
            fields: Vec::new(),
        }
    }

    pub fn with_field(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.fields.push((key.into(), value.into()));
        self
    }

    pub fn timestamp(&self) -> Timestamp {
        self.timestamp
    }

    pub fn level(&self) -> LogLevel {
        self.level
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn fields(&self) -> &[(Cow<'static, str>, Cow<'static, str>)] {
        &self.fields
    }
}
