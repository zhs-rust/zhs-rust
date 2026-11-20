use std::fmt;
use std::time::Duration;

use super::record::civil_from_days;

const DAY_SECONDS: u64 = 86_400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    #[default]
    Never,

    Daily,

    Every(Duration),
}

impl Rotation {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        if text.is_empty() {
            return None;
        }

        match text.as_str() {
            "none" | "never" | "off" => return Some(Self::Never),
            "daily" | "day" => return Some(Self::Daily),
            "hourly" | "hour" => return Some(Self::Every(Duration::from_secs(3_600))),
            "minutely" | "minute" => return Some(Self::Every(Duration::from_secs(60))),
            _ => {}
        }

        let split = text
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(text.len());
        let (digits, unit) = text.split_at(split);
        let value: u64 = digits.parse().ok()?;
        if value == 0 {
            return None;
        }

        let seconds = match unit.trim() {
            "" | "s" | "sec" | "secs" => value,
            "m" | "min" | "mins" => value.checked_mul(60)?,
            "h" | "hour" | "hours" => value.checked_mul(3_600)?,
            "d" | "day" | "days" => value.checked_mul(DAY_SECONDS)?,
            _ => return None,
        };

        if seconds == 0 || seconds > 365 * DAY_SECONDS {
            return None;
        }

        Some(Self::Every(Duration::from_secs(seconds)))
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Never)
    }

    pub fn period_seconds(&self) -> Option<u64> {
        match self {
            Self::Never => None,
            Self::Daily => Some(DAY_SECONDS),
            Self::Every(period) => Some(period.as_secs().max(1)),
        }
    }

    pub(crate) fn period_start(&self, unix_seconds: u64, utc_offset_seconds: i32) -> Option<u64> {
        let period = self.period_seconds()?;
        let offset = i64::from(utc_offset_seconds);
        let shifted = unix_seconds as i64 + offset;
        let start = shifted.div_euclid(period as i64) * period as i64;

        Some((start - offset).max(0) as u64)
    }

    pub(crate) fn suffix(&self, period_start: u64, utc_offset_seconds: i32) -> String {
        let shifted = period_start as i64 + i64::from(utc_offset_seconds);
        let days = shifted.div_euclid(DAY_SECONDS as i64);
        let seconds = shifted.rem_euclid(DAY_SECONDS as i64) as u64;
        let (year, month, day) = civil_from_days(days);

        let period = self.period_seconds().unwrap_or(DAY_SECONDS);
        let date = format!("{year:04}-{month:02}-{day:02}");

        if period >= DAY_SECONDS {
            date
        } else if period >= 60 {
            format!("{date}-{:02}{:02}", seconds / 3_600, (seconds % 3_600) / 60)
        } else {
            format!(
                "{date}-{:02}{:02}{:02}",
                seconds / 3_600,
                (seconds % 3_600) / 60,
                seconds % 60
            )
        }
    }
}

impl fmt::Display for Rotation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Never => formatter.write_str("none"),
            Self::Daily => formatter.write_str("daily"),
            Self::Every(period) => write!(formatter, "every {}s", period.as_secs()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Rotation;

    fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
        let year = if month <= 2 { year - 1 } else { year };
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let month_prime = if month > 2 {
            i64::from(month) - 3
        } else {
            i64::from(month) + 9
        };
        let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

        era * 146_097 + day_of_era - 719_468
    }

    fn unix(year: i64, month: u32, day: u32, hour: u64, minute: u64, second: u64) -> u64 {
        (days_from_civil(year, month, day) as u64) * 86_400 + hour * 3_600 + minute * 60 + second
    }

    #[test]
    fn parses_the_supported_spellings() {
        assert_eq!(Rotation::parse("none"), Some(Rotation::Never));
        assert_eq!(Rotation::parse("off"), Some(Rotation::Never));
        assert_eq!(Rotation::parse(" daily "), Some(Rotation::Daily));
        assert_eq!(
            Rotation::parse("hourly"),
            Some(Rotation::Every(Duration::from_secs(3_600)))
        );
        assert_eq!(
            Rotation::parse("30m"),
            Some(Rotation::Every(Duration::from_secs(1_800)))
        );
        assert_eq!(
            Rotation::parse("6h"),
            Some(Rotation::Every(Duration::from_secs(21_600)))
        );
        assert_eq!(
            Rotation::parse("2d"),
            Some(Rotation::Every(Duration::from_secs(172_800)))
        );
        assert_eq!(
            Rotation::parse("90"),
            Some(Rotation::Every(Duration::from_secs(90)))
        );
    }

    #[test]
    fn rejects_unknown_or_out_of_range_periods() {
        assert_eq!(Rotation::parse(""), None);
        assert_eq!(Rotation::parse("weekly"), None);
        assert_eq!(Rotation::parse("0s"), None);
        assert_eq!(Rotation::parse("0"), None);
        assert_eq!(Rotation::parse("400d"), None);
        assert_eq!(Rotation::parse("5x"), None);
    }

    #[test]
    fn aligns_daily_boundaries_to_the_offset() {
        let midnight = unix(2026, 11, 13, 16, 0, 0);

        assert_eq!(
            Rotation::Daily.period_start(midnight - 1, 28_800),
            Some(midnight - 86_400)
        );
        assert_eq!(
            Rotation::Daily.period_start(midnight, 28_800),
            Some(midnight)
        );
        assert_eq!(
            Rotation::Daily.suffix(midnight, 28_800),
            "2026-11-14".to_owned()
        );

        assert_eq!(
            Rotation::Daily.suffix(Rotation::Daily.period_start(midnight, 0).unwrap(), 0),
            "2026-11-13".to_owned()
        );
    }

    #[test]
    fn aligns_interval_boundaries_to_the_epoch() {
        let rotation = Rotation::parse("30m").unwrap();
        let half_past_three = unix(2026, 11, 14, 15, 29, 59);

        assert_eq!(
            rotation.period_start(half_past_three, 0),
            Some(unix(2026, 11, 14, 15, 0, 0))
        );
        assert_eq!(
            rotation.suffix(unix(2026, 11, 14, 15, 0, 0), 0),
            "2026-11-14-1500".to_owned()
        );
        assert_eq!(
            rotation.suffix(unix(2026, 11, 14, 15, 30, 0), 0),
            "2026-11-14-1530".to_owned()
        );
    }

    #[test]
    fn names_sub_minute_periods_with_seconds() {
        let rotation = Rotation::parse("15s").unwrap();
        let instant = unix(2026, 11, 14, 16, 0, 7);

        let start = rotation.period_start(instant, 0).unwrap();
        assert_eq!(start, unix(2026, 11, 14, 16, 0, 0));
        assert_eq!(rotation.suffix(start, 0), "2026-11-14-160000".to_owned());
    }
}
