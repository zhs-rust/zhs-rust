use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::SchedulerError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronExpression {
    seconds: Vec<u32>,
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days_of_month: Vec<u32>,
    months: Vec<u32>,
    days_of_week: Vec<u32>,
    dom_restricted: bool,
    dow_restricted: bool,
}

impl CronExpression {
    pub fn parse(expression: &str) -> Result<Self, SchedulerError> {
        let fields = expression.split_whitespace().collect::<Vec<_>>();

        let (seconds, minutes, hours, days_of_month, months, days_of_week) = match fields.len() {
            5 => (
                vec![0],
                parse_field(fields[0], 0, 59, false)?,
                parse_field(fields[1], 0, 23, false)?,
                parse_field(fields[2], 1, 31, false)?,
                parse_field(fields[3], 1, 12, false)?,
                parse_field(fields[4], 0, 7, true)?,
            ),
            6 => (
                parse_field(fields[0], 0, 59, false)?,
                parse_field(fields[1], 0, 59, false)?,
                parse_field(fields[2], 0, 23, false)?,
                parse_field(fields[3], 1, 31, false)?,
                parse_field(fields[4], 1, 12, false)?,
                parse_field(fields[5], 0, 7, true)?,
            ),
            _ => {
                return Err(SchedulerError::InvalidCron(format!(
                    "expected 5 or 6 fields, got {}",
                    fields.len()
                )));
            }
        };

        Ok(Self {
            seconds: normalize(&seconds, 0, 59),
            minutes: normalize(&minutes, 0, 59),
            hours: normalize(&hours, 0, 23),
            days_of_month: normalize(&days_of_month, 1, 31),
            months: normalize(&months, 1, 12),
            days_of_week: normalize(&days_of_week, 0, 6),
            dom_restricted: !matches!(fields[if fields.len() == 5 { 2 } else { 3 }], "*" | "?"),
            dow_restricted: !matches!(fields[if fields.len() == 5 { 4 } else { 5 }], "*" | "?"),
        })
    }

    pub fn next_after(&self, after: SystemTime) -> Option<SystemTime> {
        let after_seconds = after.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
        let start_day = (after_seconds + 1).div_euclid(86_400);
        let start_seconds = ((after_seconds + 1).rem_euclid(86_400)) as u32;

        for offset in 0..=366 * 8 {
            let day = start_day + offset;
            let start_of_day = if offset == 0 { start_seconds } else { 0 };
            let (year, month, day_of_month) = civil_from_days(day);

            if self.matches_date(year, month, day_of_month)
                && let Some(seconds) = self.first_time_on_day(day, start_of_day)
            {
                return UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64));
            }
        }

        None
    }

    fn matches_date(&self, year: i64, month: u32, day: u32) -> bool {
        let _ = year;
        if !self.months.contains(&month) {
            return false;
        }

        let day_matches = self.days_of_month.contains(&day);
        let weekday = weekday_from_days(days_from_civil(year, month, day));
        let weekday_matches = self.days_of_week.contains(&weekday);

        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => day_matches || weekday_matches,
            (true, false) => day_matches,
            (false, true) => weekday_matches,
            (false, false) => true,
        }
    }

    fn first_time_on_day(&self, day: i64, start_of_day: u32) -> Option<i64> {
        let start_hour = start_of_day / 3_600;
        let start_minute = (start_of_day % 3_600) / 60;
        let start_second = start_of_day % 60;

        for &hour in &self.hours {
            if hour < start_hour {
                continue;
            }

            for &minute in &self.minutes {
                if hour == start_hour && minute < start_minute {
                    continue;
                }

                for &second in &self.seconds {
                    // The candidate at exactly `after + 1` must fire; only
                    // strictly earlier seconds are skipped.
                    if hour == start_hour && minute == start_minute && second < start_second {
                        continue;
                    }

                    return Some(day * 86_400 + (hour * 3_600 + minute * 60 + second) as i64);
                }
            }
        }

        None
    }
}

fn parse_field(
    field: &str,
    minimum: u32,
    maximum: u32,
    weekday: bool,
) -> Result<Vec<u32>, SchedulerError> {
    if field == "?" {
        return Ok((minimum..=maximum).collect());
    }

    let mut values = Vec::new();

    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(SchedulerError::InvalidCron("empty field".to_owned()));
        }

        let (base, step) = match part.split_once('/') {
            Some((base, step)) => {
                let step = step
                    .parse::<u32>()
                    .map_err(|_| SchedulerError::InvalidCron(format!("invalid step `{step}`")))?;
                if step == 0 {
                    return Err(SchedulerError::InvalidCron("step must be > 0".to_owned()));
                }
                (base, step)
            }
            None => (part, 1),
        };

        let (start, end) = if base == "*" {
            (minimum, maximum)
        } else if let Some((start, end)) = base.split_once('-') {
            let start = parse_number(start, minimum, maximum, weekday)?;
            let end = parse_number(end, minimum, maximum, weekday)?;
            (start, end)
        } else {
            let value = parse_number(base, minimum, maximum, weekday)?;
            (value, if part.contains('/') { maximum } else { value })
        };

        if start > end {
            return Err(SchedulerError::InvalidCron(format!(
                "range `{base}` starts after it ends"
            )));
        }

        let mut value = start;
        while value <= end {
            values.push(if weekday && value == 7 { 0 } else { value });
            value = match value.checked_add(step) {
                Some(value) => value,
                None => break,
            };
        }
    }

    Ok(values)
}

fn parse_number(
    text: &str,
    minimum: u32,
    maximum: u32,
    weekday: bool,
) -> Result<u32, SchedulerError> {
    let mut value = text
        .parse::<u32>()
        .map_err(|_| SchedulerError::InvalidCron(format!("invalid number `{text}`")))?;

    if weekday && value == 7 {
        value = 0;
    }

    if !(minimum..=maximum).contains(&value) {
        return Err(SchedulerError::InvalidCron(format!(
            "`{value}` is outside {minimum}..={maximum}"
        )));
    }

    Ok(value)
}

fn normalize(values: &[u32], minimum: u32, maximum: u32) -> Vec<u32> {
    let mut normalized = values.to_vec();
    normalized.retain(|value| (minimum..=maximum).contains(value));
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn weekday_from_days(days: i64) -> u32 {
    (days + 4).rem_euclid(7) as u32
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
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
    let year = if month <= 2 { year + 1 } else { year };

    (year, month as u32, day as u32)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day = day as i64;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn parses_six_fields_and_steps() {
        let cron = CronExpression::parse("*/5 * * * * *").unwrap();
        let next = cron.next_after(at(0)).unwrap();

        assert_eq!(next, at(5));
    }

    #[test]
    fn finds_next_noon() {
        let cron = CronExpression::parse("0 0 12 * * ?").unwrap();
        let next = cron.next_after(at(1_700_000_000)).unwrap();

        assert_eq!(
            next.duration_since(UNIX_EPOCH).unwrap().as_secs() % 86_400,
            43_200
        );
    }

    #[test]
    fn rejects_invalid_fields() {
        assert!(CronExpression::parse("0 0 99 * * ?").is_err());
        assert!(CronExpression::parse("0 0 *").is_err());
    }

    #[test]
    fn fires_on_the_immediately_following_second() {
        let cron = CronExpression::parse("* * * * * *").unwrap();
        assert_eq!(cron.next_after(at(0)), Some(at(1)));
        assert_eq!(cron.next_after(at(59)), Some(at(60)));
    }
}
