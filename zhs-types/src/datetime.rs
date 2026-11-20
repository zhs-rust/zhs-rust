use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

const NANOS_PER_SECOND: u32 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const MAX_OFFSET_MINUTES: i16 = 14 * 60;

#[derive(Clone, Copy, Default)]
pub struct DateTimeOffset {
    unix_seconds: i64,
    nanos: u32,
    offset_minutes: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Components {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub nanosecond: u32,
    pub offset_minutes: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeError {
    message: String,
}

impl DateTimeError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DateTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DateTimeError {}

impl DateTimeOffset {
    pub fn from_unix_timestamp(
        unix_seconds: i64,
        nanos: u32,
        offset_minutes: i16,
    ) -> Result<Self, DateTimeError> {
        if nanos >= NANOS_PER_SECOND {
            return Err(DateTimeError::new("nanoseconds must be less than 1e9"));
        }
        if offset_minutes.unsigned_abs() > MAX_OFFSET_MINUTES as u16 {
            return Err(DateTimeError::new(
                "UTC offset must be between -14:00 and +14:00",
            ));
        }

        Ok(Self {
            unix_seconds,
            nanos,
            offset_minutes,
        })
    }

    pub fn now() -> Self {
        Self::now_with_offset(0).expect("UTC offset 0 is always valid")
    }

    pub fn now_with_offset(offset_minutes: i16) -> Result<Self, DateTimeError> {
        let (seconds, nanos) = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => (duration.as_secs() as i64, duration.subsec_nanos()),
            Err(error) => {
                let duration = error.duration();
                let seconds = -(duration.as_secs() as i64);
                let nanos = duration.subsec_nanos();

                if nanos == 0 {
                    (seconds, 0)
                } else {
                    (seconds - 1, NANOS_PER_SECOND - nanos)
                }
            }
        };

        Self::from_unix_timestamp(seconds, nanos, offset_minutes)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_components(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        nanosecond: u32,
        offset_minutes: i16,
    ) -> Result<Self, DateTimeError> {
        validate_date(year, month, day)?;
        validate_time(hour, minute, second, nanosecond)?;

        if offset_minutes.unsigned_abs() > MAX_OFFSET_MINUTES as u16 {
            return Err(DateTimeError::new(
                "UTC offset must be between -14:00 and +14:00",
            ));
        }

        let days = days_from_civil(year, month, day);
        let local_seconds = days
            .checked_mul(SECONDS_PER_DAY)
            .and_then(|value| value.checked_add(hour as i64 * 3_600))
            .and_then(|value| value.checked_add(minute as i64 * 60))
            .and_then(|value| value.checked_add(second as i64))
            .ok_or_else(|| DateTimeError::new("date/time is out of range"))?;

        let unix_seconds = local_seconds
            .checked_sub(offset_minutes as i64 * 60)
            .ok_or_else(|| DateTimeError::new("date/time is out of range"))?;

        Self::from_unix_timestamp(unix_seconds, nanosecond, offset_minutes)
    }

    pub const fn unix_timestamp(&self) -> i64 {
        self.unix_seconds
    }

    pub fn unix_timestamp_nanos(&self) -> i128 {
        self.unix_seconds as i128 * NANOS_PER_SECOND as i128 + self.nanos as i128
    }

    pub const fn nanosecond(&self) -> u32 {
        self.nanos
    }

    pub const fn offset_minutes(&self) -> i16 {
        self.offset_minutes
    }

    pub fn components(&self) -> Components {
        // Saturate instead of overflowing for extreme `unix_seconds` so that
        // Display/to_rfc3339 degrade gracefully rather than panic (debug
        // builds) or wrap around (release builds).
        let local_seconds = self
            .unix_seconds
            .saturating_add((self.offset_minutes as i64).saturating_mul(60));
        let days = local_seconds.div_euclid(SECONDS_PER_DAY);
        let seconds_of_day = local_seconds.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);

        Components {
            year,
            month,
            day,
            hour: (seconds_of_day / 3_600) as u32,
            minute: ((seconds_of_day % 3_600) / 60) as u32,
            second: (seconds_of_day % 60) as u32,
            nanosecond: self.nanos,
            offset_minutes: self.offset_minutes,
        }
    }

    pub fn to_offset(&self, offset_minutes: i16) -> Result<Self, DateTimeError> {
        Self::from_unix_timestamp(self.unix_seconds, self.nanos, offset_minutes)
    }

    pub fn checked_add_seconds(&self, seconds: i64) -> Option<Self> {
        Some(Self {
            unix_seconds: self.unix_seconds.checked_add(seconds)?,
            nanos: self.nanos,
            offset_minutes: self.offset_minutes,
        })
    }

    pub fn to_rfc3339(&self) -> String {
        self.to_string()
    }

    pub fn parse_rfc3339(input: &str) -> Result<Self, DateTimeError> {
        let input = input.trim();
        let bytes = input.as_bytes();

        if bytes.len() < 19 {
            return Err(DateTimeError::new("timestamp is too short"));
        }

        if bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(DateTimeError::new("expected YYYY-MM-DD"));
        }
        if !matches!(bytes[10], b'T' | b't' | b' ') {
            return Err(DateTimeError::new("expected 'T' between date and time"));
        }
        if bytes[13] != b':' || bytes[16] != b':' {
            return Err(DateTimeError::new("expected HH:MM:SS"));
        }

        let year = parse_digits(&bytes[0..4])? as i32;
        let month = parse_digits(&bytes[5..7])?;
        let day = parse_digits(&bytes[8..10])?;
        let hour = parse_digits(&bytes[11..13])?;
        let minute = parse_digits(&bytes[14..16])?;
        let second = parse_digits(&bytes[17..19])?;

        let mut index = 19;
        let mut nanos = 0_u32;

        if index < bytes.len() && bytes[index] == b'.' {
            index += 1;
            let start = index;

            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }

            let digits = index - start;
            if digits == 0 || digits > 9 {
                return Err(DateTimeError::new(
                    "fractional seconds must contain 1 to 9 digits",
                ));
            }

            let mut value = parse_digits(&bytes[start..index])?;
            for _ in digits..9 {
                value *= 10;
            }
            nanos = value;
        }

        let offset_minutes = if index == bytes.len() {
            0
        } else {
            match bytes[index] {
                b'Z' | b'z' => {
                    index += 1;
                    0
                }
                sign @ (b'+' | b'-') => {
                    index += 1;

                    if index + 5 > bytes.len() || bytes[index + 2] != b':' {
                        return Err(DateTimeError::new("expected offset like +08:00"));
                    }

                    let offset_hour = parse_digits(&bytes[index..index + 2])?;
                    let offset_minute = parse_digits(&bytes[index + 3..index + 5])?;
                    index += 5;

                    if offset_hour > 23 || offset_minute > 59 {
                        return Err(DateTimeError::new("invalid UTC offset"));
                    }

                    let magnitude = (offset_hour * 60 + offset_minute) as i16;
                    if sign == b'-' { -magnitude } else { magnitude }
                }
                _ => return Err(DateTimeError::new("invalid UTC offset")),
            }
        };

        if index != bytes.len() {
            return Err(DateTimeError::new("trailing characters after timestamp"));
        }

        Self::from_components(
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanos,
            offset_minutes,
        )
    }
}

impl fmt::Display for DateTimeOffset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let components = self.components();

        write!(
            formatter,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            components.year,
            components.month,
            components.day,
            components.hour,
            components.minute,
            components.second
        )?;

        if components.nanosecond != 0 {
            let mut fraction = format!("{:09}", components.nanosecond);
            while fraction.ends_with('0') {
                fraction.pop();
            }
            write!(formatter, ".{fraction}")?;
        }

        if components.offset_minutes == 0 {
            formatter.write_str("Z")
        } else {
            let sign = if components.offset_minutes < 0 {
                '-'
            } else {
                '+'
            };
            let offset = components.offset_minutes.unsigned_abs();
            write!(formatter, "{sign}{:02}:{:02}", offset / 60, offset % 60)
        }
    }
}

impl fmt::Debug for DateTimeOffset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "DateTimeOffset(\"{self}\")")
    }
}

impl PartialEq for DateTimeOffset {
    fn eq(&self, other: &Self) -> bool {
        self.unix_seconds == other.unix_seconds && self.nanos == other.nanos
    }
}

impl Eq for DateTimeOffset {}

impl PartialOrd for DateTimeOffset {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DateTimeOffset {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.unix_seconds, self.nanos).cmp(&(other.unix_seconds, other.nanos))
    }
}

impl Hash for DateTimeOffset {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.unix_seconds.hash(state);
        self.nanos.hash(state);
    }
}

impl FromStr for DateTimeOffset {
    type Err = DateTimeError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse_rfc3339(input)
    }
}

fn parse_digits(bytes: &[u8]) -> Result<u32, DateTimeError> {
    if bytes.is_empty() {
        return Err(DateTimeError::new("expected a digit"));
    }

    let mut value = 0_u32;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return Err(DateTimeError::new("expected a digit"));
        }
        value = value * 10 + (byte - b'0') as u32;
    }

    Ok(value)
}

fn validate_date(year: i32, month: u32, day: u32) -> Result<(), DateTimeError> {
    if !(0..=9999).contains(&year) {
        return Err(DateTimeError::new("year must be between 0000 and 9999"));
    }
    if !(1..=12).contains(&month) {
        return Err(DateTimeError::new("month must be between 1 and 12"));
    }

    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => unreachable!(),
    };

    if !(1..=days_in_month).contains(&day) {
        return Err(DateTimeError::new("day is out of range for the month"));
    }

    Ok(())
}

fn validate_time(
    hour: u32,
    minute: u32,
    second: u32,
    nanosecond: u32,
) -> Result<(), DateTimeError> {
    if hour > 23 {
        return Err(DateTimeError::new("hour must be between 0 and 23"));
    }
    if minute > 59 {
        return Err(DateTimeError::new("minute must be between 0 and 59"));
    }
    if second > 59 {
        return Err(DateTimeError::new("second must be between 0 and 59"));
    }
    if nanosecond >= NANOS_PER_SECOND {
        return Err(DateTimeError::new("nanoseconds must be less than 1e9"));
    }

    Ok(())
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year } as i64;
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month as i64 + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };

    (
        if month <= 2 { year + 1 } else { year } as i32,
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_utc() {
        let value = DateTimeOffset::parse_rfc3339("2026-11-19T13:30:00Z").unwrap();
        assert_eq!(value.to_string(), "2026-11-19T13:30:00Z");
        assert_eq!(value.offset_minutes(), 0);
    }

    #[test]
    fn offsets_compare_by_instant() {
        let utc = DateTimeOffset::parse_rfc3339("2026-11-19T13:30:00Z").unwrap();
        let shanghai = DateTimeOffset::parse_rfc3339("2026-11-19T21:30:00+08:00").unwrap();

        assert_eq!(utc, shanghai);
        assert_eq!(utc.cmp(&shanghai), Ordering::Equal);
        assert_eq!(shanghai.to_string(), "2026-11-19T21:30:00+08:00");
        assert_eq!(shanghai.components().hour, 21);
    }

    #[test]
    fn converts_offsets_without_changing_instant() {
        let value = DateTimeOffset::parse_rfc3339("2026-11-19T21:30:00+08:00").unwrap();
        let utc = value.to_offset(0).unwrap();

        assert_eq!(value, utc);
        assert_eq!(utc.to_string(), "2026-11-19T13:30:00Z");
    }

    #[test]
    fn handles_leap_years() {
        assert!(DateTimeOffset::parse_rfc3339("2024-02-29T00:00:00Z").is_ok());
        assert!(DateTimeOffset::parse_rfc3339("2023-02-29T00:00:00Z").is_err());
        assert!(DateTimeOffset::parse_rfc3339("2000-02-29T00:00:00Z").is_ok());
        assert!(DateTimeOffset::parse_rfc3339("1900-02-29T00:00:00Z").is_err());
    }

    #[test]
    fn handles_fractional_seconds() {
        let value = DateTimeOffset::parse_rfc3339("2026-11-19T13:30:00.123456789Z").unwrap();
        assert_eq!(value.nanosecond(), 123_456_789);
        assert_eq!(value.to_string(), "2026-11-19T13:30:00.123456789Z");

        let value = DateTimeOffset::parse_rfc3339("2026-11-19T13:30:00.120Z").unwrap();
        assert_eq!(value.to_string(), "2026-11-19T13:30:00.12Z");
    }

    #[test]
    fn rejects_invalid_values() {
        assert!(DateTimeOffset::parse_rfc3339("2026-13-01T00:00:00Z").is_err());
        assert!(DateTimeOffset::parse_rfc3339("2026-11-19T24:00:00Z").is_err());
        assert!(DateTimeOffset::parse_rfc3339("2026-11-19T13:30:00+15:00").is_err());
        assert!(DateTimeOffset::parse_rfc3339("not-a-date").is_err());
    }

    #[test]
    fn now_is_utc() {
        let now = DateTimeOffset::now();
        assert_eq!(now.offset_minutes(), 0);
        assert!(now.unix_timestamp() > 1_700_000_000);
    }
}
