//! The instants and time zones the JSON API accepts.
//!
//! The page, the CLI and the export speak RFC 3339 UTC and epoch milliseconds. The JSON API
//! speaks one more dialect: `now`, relative windows such as `-1h` or `-2d`, local dates and
//! date-times without an offset, and a `tz` parameter that supplies the zone those local
//! values are read in. This module owns that grammar so every route resolves it the same way.
//!
//! The parsing is pure: the caller passes one `now` for the whole request, so `from`, `to`,
//! and their defaults are resolved against the same instant.

use chrono::{
    DateTime, Days, FixedOffset, LocalResult, Months, NaiveDate, NaiveDateTime, TimeZone, Utc,
};
use chrono_tz::Tz;

use crate::time::format_rfc3339;

use super::{Rejection, DOMAIN_MAX_MS, DOMAIN_MIN_MS};

/// The zone a local instant is read in.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Zone {
    Utc,
    Fixed(FixedOffset),
    Named(Tz),
}

impl Zone {
    /// Turn a wall-clock time in this zone into epoch milliseconds.
    fn resolve(self, naive: NaiveDateTime, name: &str) -> Result<i64, Rejection> {
        match self {
            Zone::Utc => Ok(Utc.from_utc_datetime(&naive).timestamp_millis()),
            Zone::Fixed(offset) => settle(offset.from_local_datetime(&naive), name),
            Zone::Named(tz) => settle(tz.from_local_datetime(&naive), name),
        }
    }

    /// Render an instant in this zone as RFC 3339: `Z` for UTC, a numeric offset otherwise.
    pub(crate) fn format(self, ms: i64) -> String {
        let instant = Utc
            .timestamp_millis_opt(ms)
            .single()
            .expect("the calendar domain keeps every instant representable");
        match self {
            Zone::Utc => crate::time::format_rfc3339(ms),
            Zone::Fixed(offset) => instant
                .with_timezone(&offset)
                .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
                .to_string(),
            Zone::Named(tz) => instant
                .with_timezone(&tz)
                .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
                .to_string(),
        }
    }

    /// The wall-clock time this instant reads in this zone.
    fn local(self, ms: i64) -> NaiveDateTime {
        let instant = Utc
            .timestamp_millis_opt(ms)
            .single()
            .expect("the calendar domain keeps every instant representable");
        match self {
            Zone::Utc => instant.naive_utc(),
            Zone::Fixed(offset) => instant.with_timezone(&offset).naive_local(),
            Zone::Named(tz) => instant.with_timezone(&tz).naive_local(),
        }
    }
}

/// Turn a local resolution into one instant: a repeated hour takes its earlier occurrence,
/// and a local time the zone skips does not exist.
fn settle<T: TimeZone>(local: LocalResult<DateTime<T>>, name: &str) -> Result<i64, Rejection> {
    match local {
        LocalResult::Single(instant) => Ok(instant.timestamp_millis()),
        LocalResult::Ambiguous(first, _second) => Ok(first.timestamp_millis()),
        LocalResult::None => Err(Rejection::parameter(
            name,
            format!("{name} names a local time that does not exist in the requested time zone"),
        )),
    }
}

/// A unit of a relative instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

/// Resolve the `tz` parameter: `UTC`, a fixed offset, or an IANA name. Absent means UTC.
pub(super) fn parse_zone(value: Option<&str>) -> Result<Zone, Rejection> {
    let Some(value) = value else {
        return Ok(Zone::Utc);
    };
    let value = with_plus(value);
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("utc") || value == "Z" {
        return Ok(Zone::Utc);
    }
    if let Some(offset) = parse_offset(value) {
        return Ok(Zone::Fixed(offset));
    }
    value.parse::<Tz>().map(Zone::Named).map_err(|_| {
        Rejection::parameter(
            "tz",
            "tz must be 'UTC', an offset such as '+02:00', or an IANA time zone such as \
             'Europe/Rome'"
                .to_string(),
        )
    })
}

/// Parse one instant, in the grammar the API documents.
pub(super) fn parse_instant(
    name: &str,
    value: &str,
    zone: Zone,
    now_ms: i64,
) -> Result<i64, Rejection> {
    let value = with_plus(value);
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid(name));
    }
    if value.eq_ignore_ascii_case("now") {
        return check_domain(name, now_ms);
    }
    if let Some((negative, amount, unit)) = parse_relative(value) {
        return check_domain(name, shift(zone, now_ms, negative, amount, unit, name)?);
    }
    if let Ok(millis) = value.parse::<i64>() {
        return check_domain(name, millis);
    }

    let (instant, explicit) = split_suffix(value);
    let naive = parse_naive(instant).ok_or_else(|| invalid(name))?;
    let resolved = explicit.unwrap_or(zone).resolve(naive, name)?;
    check_domain(name, resolved)
}

/// A query value is form-decoded, so a literal `+` arrives as a space. Every `+` in this
/// grammar is a sign, and a real space is never accepted, so restoring them is unambiguous.
fn with_plus(value: &str) -> String {
    // A leading space is a decoded `+`, so only trailing whitespace is discarded here.
    value.trim_end().replace(' ', "+")
}

fn invalid(name: &str) -> Rejection {
    Rejection::parameter(
        name,
        format!(
            "{name} must be 'now', a relative instant such as '-1h' or '-2d', epoch \
             milliseconds, or a date such as '2026-11-14' or '2026-11-14T22:00:00' with an \
             optional 'Z' or offset"
        ),
    )
}

fn check_domain(name: &str, millis: i64) -> Result<i64, Rejection> {
    if !(DOMAIN_MIN_MS..=DOMAIN_MAX_MS).contains(&millis) {
        return Err(Rejection::parameter(
            name,
            format!(
                "{name} must resolve to an instant between {} and {}",
                format_rfc3339(DOMAIN_MIN_MS),
                format_rfc3339(DOMAIN_MAX_MS),
            ),
        ));
    }
    Ok(millis)
}

fn parse_offset(value: &str) -> Option<FixedOffset> {
    let bytes = value.as_bytes();
    if bytes.len() != 6 || (bytes[0] != b'+' && bytes[0] != b'-') || bytes[3] != b':' {
        return None;
    }
    let hours: i32 = value[1..3].parse().ok()?;
    let minutes: i32 = value[4..6].parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    let sign = if bytes[0] == b'-' { -1 } else { 1 };
    FixedOffset::east_opt(sign * (hours * 3_600 + minutes * 60))
}

/// `[+-]N` followed by a unit, longest suffix first so `mo` is never read as `m`.
fn parse_relative(value: &str) -> Option<(bool, u32, Unit)> {
    let (negative, rest) = match value.as_bytes().first()? {
        b'-' => (true, &value[1..]),
        b'+' => (false, &value[1..]),
        _ => return None,
    };
    const UNITS: [(&str, Unit); 7] = [
        ("mo", Unit::Month),
        ("s", Unit::Second),
        ("m", Unit::Minute),
        ("h", Unit::Hour),
        ("d", Unit::Day),
        ("w", Unit::Week),
        ("y", Unit::Year),
    ];
    for (suffix, unit) in UNITS {
        if let Some(number) = rest.strip_suffix(suffix) {
            if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
                if let Ok(amount) = number.parse::<u32>() {
                    return Some((negative, amount, unit));
                }
            }
        }
    }
    None
}

/// Move `now` by a relative amount: exact for seconds, minutes, and hours; calendar steps in
/// the requested zone for days, weeks, months, and years.
fn shift(
    zone: Zone,
    now_ms: i64,
    negative: bool,
    amount: u32,
    unit: Unit,
    name: &str,
) -> Result<i64, Rejection> {
    let exact_ms = match unit {
        Unit::Second => Some(1_000_i64),
        Unit::Minute => Some(60_000),
        Unit::Hour => Some(3_600_000),
        _ => None,
    };
    if let Some(unit_ms) = exact_ms {
        let delta = i64::from(amount)
            .checked_mul(unit_ms)
            .ok_or_else(|| out_of_domain(name))?;
        return if negative {
            now_ms.checked_sub(delta)
        } else {
            now_ms.checked_add(delta)
        }
        .ok_or_else(|| out_of_domain(name));
    }

    let local = zone.local(now_ms);
    let shifted = match unit {
        Unit::Day => add_days(local, amount, negative),
        Unit::Week => add_days(
            local,
            amount.checked_mul(7).ok_or_else(|| out_of_domain(name))?,
            negative,
        ),
        Unit::Month => add_months(local, amount, negative),
        Unit::Year => add_months(
            local,
            amount.checked_mul(12).ok_or_else(|| out_of_domain(name))?,
            negative,
        ),
        Unit::Second | Unit::Minute | Unit::Hour => unreachable!("handled above"),
    }
    .ok_or_else(|| out_of_domain(name))?;
    zone.resolve(shifted, name)
}

fn add_days(local: NaiveDateTime, days: u32, negative: bool) -> Option<NaiveDateTime> {
    if negative {
        local.checked_sub_days(Days::new(u64::from(days)))
    } else {
        local.checked_add_days(Days::new(u64::from(days)))
    }
}

fn add_months(local: NaiveDateTime, months: u32, negative: bool) -> Option<NaiveDateTime> {
    if negative {
        local.checked_sub_months(Months::new(months))
    } else {
        local.checked_add_months(Months::new(months))
    }
}

/// Split an explicit `Z` or `±HH:MM` suffix from the wall-clock text.
fn split_suffix(text: &str) -> (&str, Option<Zone>) {
    if let Some(rest) = text.strip_suffix('Z') {
        return (rest, Some(Zone::Utc));
    }
    if text.len() >= 6 {
        let tail = &text[text.len() - 6..];
        if let Some(offset) = parse_offset(tail) {
            return (&text[..text.len() - 6], Some(Zone::Fixed(offset)));
        }
    }
    (text, None)
}

/// `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM`, or `YYYY-MM-DDTHH:MM:SS[.fff]`.
fn parse_naive(text: &str) -> Option<NaiveDateTime> {
    if text.len() == 10 {
        return NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(0, 0, 0);
    }
    if text.len() < 16 {
        return None;
    }
    if let Some(position) = text.find('.') {
        let fraction = &text[position + 1..];
        if fraction.is_empty()
            || fraction.len() > 3
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M"))
        .ok()
}

fn out_of_domain(name: &str) -> Rejection {
    Rejection::parameter(
        name,
        format!(
            "{name} must resolve to an instant between {} and {}",
            format_rfc3339(DOMAIN_MIN_MS),
            format_rfc3339(DOMAIN_MAX_MS),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::parse_rfc3339;

    /// 2026-11-14T22:00:00.000Z
    const NOW: i64 = 1_794_693_600_000;

    fn instant(name: &str, value: &str, zone: Zone) -> i64 {
        parse_instant(name, value, zone, NOW)
            .ok()
            .expect("the value is accepted")
    }

    fn refused(name: &str, value: &str, zone: Zone) {
        let error = parse_instant(name, value, zone, NOW).expect_err("the value is refused");
        assert_eq!(error.status, 400);
        assert_eq!(error.code, "INVALID_PARAMETER");
        assert_eq!(error.parameter.as_deref(), Some(name));
    }

    #[test]
    fn now_is_the_request_instant() {
        assert_eq!(instant("from", "now", Zone::Utc), NOW);
        assert_eq!(instant("from", "NOW", Zone::Utc), NOW);
    }

    #[test]
    fn relative_exact_units() {
        assert_eq!(instant("from", "-30s", Zone::Utc), NOW - 30_000);
        assert_eq!(instant("from", "-15m", Zone::Utc), NOW - 15 * 60_000);
        assert_eq!(instant("to", "+2h", Zone::Utc), NOW + 2 * 3_600_000);
    }

    #[test]
    fn relative_calendar_units_follow_the_zone() {
        // 2026-11-14T22:00:00Z is 2026-11-14T23:00:00 in Europe/Rome (CET, +01:00).
        let rome = parse_zone(Some("Europe/Rome")).ok().expect("known zone");
        assert_eq!(
            instant("from", "-1d", rome),
            parse_rfc3339("2026-11-13T22:00:00.000Z").unwrap()
        );
        assert_eq!(
            instant("from", "-1mo", rome),
            parse_rfc3339("2026-10-14T21:00:00.000Z").unwrap()
        );
        assert_eq!(
            instant("from", "-1y", rome),
            parse_rfc3339("2025-11-14T22:00:00.000Z").unwrap()
        );
        assert_eq!(
            instant("from", "-1w", rome),
            parse_rfc3339("2026-11-07T22:00:00.000Z").unwrap()
        );
    }

    #[test]
    fn a_calendar_month_clamps_the_day() {
        // 2026-03-31T12:00:00Z - 1 month = 2026-02-28T12:00:00Z.
        let now = parse_rfc3339("2026-03-31T12:00:00.000Z").unwrap();
        let result = parse_instant("from", "-1mo", Zone::Utc, now).ok().unwrap();
        assert_eq!(result, parse_rfc3339("2026-02-28T12:00:00.000Z").unwrap());
    }

    #[test]
    fn epoch_milliseconds_are_taken_as_they_are() {
        assert_eq!(instant("from", "1794693600000", Zone::Utc), NOW);
    }

    #[test]
    fn absolute_forms() {
        assert_eq!(
            instant("from", "2026-11-14", Zone::Utc),
            parse_rfc3339("2026-11-14T00:00:00.000Z").unwrap()
        );
        assert_eq!(
            instant("from", "2026-11-14T22:00", Zone::Utc),
            parse_rfc3339("2026-11-14T22:00:00.000Z").unwrap()
        );
        assert_eq!(instant("from", "2026-11-14T22:00:00", Zone::Utc), NOW);
        assert_eq!(
            instant("from", "2026-11-14T22:00:00.5", Zone::Utc),
            NOW + 500
        );
        assert_eq!(instant("from", "2026-11-14T22:00:00Z", Zone::Utc), NOW);
    }

    #[test]
    fn a_local_instant_is_read_in_the_zone() {
        let rome = parse_zone(Some("Europe/Rome")).ok().expect("known zone");
        assert_eq!(
            instant("from", "2026-11-14T23:00:00", rome),
            NOW,
            "23:00 in Rome is 22:00 UTC"
        );
        let plus_two = parse_zone(Some("+02:00")).ok().expect("fixed offset");
        assert_eq!(instant("from", "2026-11-15T00:00:00", plus_two), NOW);
    }

    #[test]
    fn an_explicit_suffix_wins_over_the_zone() {
        let rome = parse_zone(Some("Europe/Rome")).ok().expect("known zone");
        assert_eq!(instant("from", "2026-11-14T22:00:00Z", rome), NOW);
    }

    #[test]
    fn an_instant_renders_in_the_zone() {
        assert_eq!(Zone::Utc.format(NOW), "2026-11-14T22:00:00.000Z");
        let rome = parse_zone(Some("Europe/Rome")).ok().expect("known zone");
        assert_eq!(rome.format(NOW), "2026-11-14T23:00:00.000+01:00");
        let plus_two = parse_zone(Some("+02:00")).ok().expect("fixed offset");
        assert_eq!(plus_two.format(NOW), "2026-11-15T00:00:00.000+02:00");
    }

    #[test]
    fn the_zone_reads_offsets_and_names() {
        assert!(matches!(parse_zone(None), Ok(Zone::Utc)));
        assert!(matches!(parse_zone(Some("")), Ok(Zone::Utc)));
        assert!(matches!(parse_zone(Some("UTC")), Ok(Zone::Utc)));
        assert!(matches!(parse_zone(Some("-05:00")), Ok(Zone::Fixed(_))));
        assert!(matches!(
            parse_zone(Some("Europe/Rome")),
            Ok(Zone::Named(_))
        ));
        let error = parse_zone(Some("Mars/Olympus")).expect_err("unknown zone");
        assert_eq!(error.parameter.as_deref(), Some("tz"));
    }

    #[test]
    fn a_plus_arrives_as_a_space_and_is_restored() {
        assert_eq!(
            instant("from", "2026-11-14T00:00:00 02:00", Zone::Utc),
            parse_rfc3339("2026-11-13T22:00:00.000Z").unwrap()
        );
        assert_eq!(instant("to", " 2h", Zone::Utc), NOW + 2 * 3_600_000);
    }

    #[test]
    fn the_dst_gap_is_refused_and_the_ambiguous_hour_is_the_earlier_one() {
        let rome = parse_zone(Some("Europe/Rome")).ok().expect("known zone");
        // 2026-03-29T02:30 does not exist in Rome: the clock jumps from 02:00 to 03:00.
        refused("from", "2026-03-29T02:30:00", rome);
        // 2026-10-25T02:30 happens twice; the earlier one is CEST (+02:00).
        assert_eq!(
            instant("from", "2026-10-25T02:30:00", rome),
            parse_rfc3339("2026-10-25T00:30:00.000Z").unwrap()
        );
    }

    #[test]
    fn refusals() {
        refused("from", "", Zone::Utc);
        refused("from", "yesterday", Zone::Utc);
        refused("from", "-1x", Zone::Utc);
        refused("from", "2026-13-01", Zone::Utc);
        refused("from", "2026-02-30", Zone::Utc);
        refused("from", "2026-11-14T25:00:00", Zone::Utc);
        refused("from", "2026-11-14T22:00:00.1234", Zone::Utc);
        refused("from", "1969-12-31T00:00:00", Zone::Utc);
        refused("from", "2101-01-01T00:00:00", Zone::Utc);
    }
}
