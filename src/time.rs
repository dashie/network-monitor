//! UTC time helpers.
//!
//! Timestamps are stored as epoch milliseconds (INV-6) and rendered as RFC 3339 in
//! UTC. The conversion is small enough to live here rather than in a date crate.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current time as UTC epoch milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Format epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_rfc3339(ms: i64) -> String {
    let seconds = ms.div_euclid(1_000);
    let millis = ms.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);

    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Parse a `--since` value into an absolute epoch-millisecond cut-off.
///
/// Accepted forms: a duration with a unit suffix (`30s`, `15m`, `24h`, `7d`),
/// relative to `now_ms`, or an absolute epoch-millisecond value.
pub fn parse_since(value: &str, now_ms: i64) -> Result<i64, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("empty value".to_string());
    }

    if let Some(number) = trimmed.strip_suffix('s') {
        return parse_amount(number).map(|amount| now_ms - amount * 1_000);
    }
    if let Some(number) = trimmed.strip_suffix('m') {
        return parse_amount(number).map(|amount| now_ms - amount * 60_000);
    }
    if let Some(number) = trimmed.strip_suffix('h') {
        return parse_amount(number).map(|amount| now_ms - amount * 3_600_000);
    }
    if let Some(number) = trimmed.strip_suffix('d') {
        return parse_amount(number).map(|amount| now_ms - amount * 86_400_000);
    }

    trimmed
        .parse::<i64>()
        .map_err(|_| "expected a duration such as 24h or an epoch-millisecond value".to_string())
}

/// Parse `YYYY-MM-DDTHH:MM:SS.mmmZ` into epoch milliseconds.
///
/// Only the format this binary writes is accepted: a UTC instant with millisecond
/// precision. Returns `None` for anything else, including an impossible date.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    if !text.is_ascii() || text.len() < 20 || text.len() > 24 {
        return None;
    }
    let bytes = text.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }

    let year: i64 = text[0..4].parse().ok()?;
    let month: u32 = text[5..7].parse().ok()?;
    let day: u32 = text[8..10].parse().ok()?;
    let hour: i64 = text[11..13].parse().ok()?;
    let minute: i64 = text[14..16].parse().ok()?;
    let second: i64 = text[17..19].parse().ok()?;

    let millis: i64 = match &text[19..] {
        "Z" => 0,
        rest => {
            let fraction = rest.strip_prefix('.')?.strip_suffix('Z')?;
            if fraction.len() != 3 {
                return None;
            }
            fraction.parse().ok()?
        }
    };

    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds_of_day = hour * 3_600 + minute * 60 + second;
    Some(days * 86_400_000 + seconds_of_day * 1_000 + millis)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

/// A civil date to a day count since 1970-01-01, after Howard Hinnant's
/// `days_from_civil`.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_index = if month > 2 {
        month as i64 - 3
    } else {
        month as i64 + 9
    };
    let day_of_year = (153 * month_index + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn parse_amount(number: &str) -> Result<i64, String> {
    number
        .trim()
        .parse::<i64>()
        .map_err(|_| format!("'{number}' is not a whole number"))
}

/// Convert a day count since the Unix epoch to a civil date.
///
/// Howard Hinnant's `civil_from_days`, valid for the whole range we can reach from
/// an `i64` of milliseconds.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_epoch() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn formats_a_known_instant() {
        // 2026-11-08T00:00:00Z
        assert_eq!(
            format_rfc3339(1_794_096_000_000),
            "2026-11-08T00:00:00.000Z"
        );
    }

    #[test]
    fn formats_before_the_epoch() {
        assert_eq!(format_rfc3339(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn parses_relative_durations() {
        let now = 1_000_000_000_000;
        assert_eq!(parse_since("24h", now).unwrap(), now - 86_400_000);
        assert_eq!(parse_since("90m", now).unwrap(), now - 5_400_000);
        assert_eq!(parse_since("30s", now).unwrap(), now - 30_000);
        assert_eq!(parse_since("7d", now).unwrap(), now - 604_800_000);
    }

    #[test]
    fn parses_absolute_milliseconds() {
        assert_eq!(parse_since("1762560000000", 0).unwrap(), 1_762_560_000_000);
    }

    #[test]
    fn parses_the_timestamps_of_the_export() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            parse_rfc3339("2026-11-08T00:00:00.000Z"),
            Some(1_794_096_000_000)
        );
        assert_eq!(
            parse_rfc3339("2026-11-08T00:00:00Z"),
            Some(1_794_096_000_000)
        );
        assert_eq!(parse_rfc3339("1969-12-31T23:59:59.999Z"), Some(-1));
    }

    #[test]
    fn a_parsed_timestamp_formats_back_to_the_same_text() {
        for text in [
            "1970-01-01T00:00:00.000Z",
            "2026-11-08T12:34:56.789Z",
            "2024-02-29T23:59:59.001Z",
        ] {
            assert_eq!(format_rfc3339(parse_rfc3339(text).unwrap()), text);
        }
    }

    #[test]
    fn impossible_or_malformed_timestamps_are_rejected() {
        for text in [
            "",
            "2026-11-08",
            "2026-13-01T00:00:00.000Z",
            "2026-02-30T00:00:00.000Z",
            "2023-02-29T00:00:00.000Z",
            "2026-11-08T24:00:00.000Z",
            "2026-11-08T00:60:00.000Z",
            "2026-11-08T00:00:00+02:00",
            "2026-11-08T00:00:00.1Z",
            "oggi",
        ] {
            assert!(parse_rfc3339(text).is_none(), "accepted {text}");
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_since("yesterday", 0).is_err());
        assert!(parse_since("", 0).is_err());
        assert!(parse_since("h", 0).is_err());
    }
}
