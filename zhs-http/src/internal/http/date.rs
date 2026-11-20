use std::time::{Duration, SystemTime, UNIX_EPOCH};

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn format_http_date(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    format_timestamp(seconds)
}

fn format_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let clock = seconds % 86_400;
    let hour = clock / 3_600;
    let minute = (clock % 3_600) / 60;
    let second = clock % 60;

    let weekday = (days + 4).rem_euclid(7) as usize;
    let (year, month, day) = civil_from_days(days);

    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[weekday],
        day,
        MONTHS[(month - 1) as usize],
        year,
        hour,
        minute,
        second
    )
}

pub(crate) fn parse_http_date(value: &str) -> Option<SystemTime> {
    let rest = value.trim().strip_suffix(" GMT")?;
    let (weekday, rest) = rest.split_once(", ")?;
    if !WEEKDAYS.contains(&weekday) {
        return None;
    }

    let mut parts = rest.split(' ');
    let day: u32 = parse_digits(parts.next()?)?;
    let month_name = parts.next()?;
    let month = MONTHS
        .iter()
        .position(|name| *name == month_name)
        .map(|index| index as u32 + 1)?;
    let year: i64 = parse_digits(parts.next()?)?;
    if !(0..=9999).contains(&year) {
        return None;
    }
    let clock = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let mut clock = clock.split(':');
    let hour: u32 = parse_digits(clock.next()?)?;
    let minute: u32 = parse_digits(clock.next()?)?;
    let second: u32 = parse_digits(clock.next()?)?;
    if clock.next().is_some()
        || hour > 23
        || minute > 59
        || second > 59
        || day == 0
        || month == 0
        || day > days_in_month(year, month)
    {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds = days.checked_mul(86_400)?
        + i64::from(hour) * 3_600
        + i64::from(minute) * 60
        + i64::from(second);
    if seconds < 0 {
        return None;
    }

    Some(UNIX_EPOCH + Duration::from_secs(seconds as u64))
}

// IMF-fixdate numeric fields must be plain decimal digits: reject the '+'
// sign that Rust's integer FromStr would otherwise accept.
fn parse_digits<T: std::str::FromStr>(field: &str) -> Option<T> {
    if field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    field.parse().ok()
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
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
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };

    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::{format_timestamp, parse_http_date};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_timestamp(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(
            format_timestamp(784_111_777),
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
        assert_eq!(
            format_timestamp(951_782_400),
            "Tue, 29 Feb 2000 00:00:00 GMT"
        );
    }

    #[test]
    fn parses_known_instants() {
        assert_eq!(
            parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"),
            Some(UNIX_EPOCH)
        );
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(UNIX_EPOCH + Duration::from_secs(784_111_777))
        );
        assert_eq!(
            parse_http_date("Tue, 29 Feb 2000 00:00:00 GMT"),
            Some(UNIX_EPOCH + Duration::from_secs(951_782_400))
        );
    }

    #[test]
    fn rejects_invalid_dates() {
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37"), None);
        assert_eq!(parse_http_date("Sun, 31 Feb 2000 00:00:00 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 Foo 1994 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 24:00:00 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49 GMT"), None);
        assert_eq!(parse_http_date("Wed, 31 Dec 1969 23:59:59 GMT"), None);
        assert_eq!(parse_http_date("Sun, +6 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 +8:49:37 GMT"), None);
    }
}
