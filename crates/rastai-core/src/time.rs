//! Time as Unix milliseconds, and RFC 3339 text for timestamps inside documents (contracts 5.1:
//! `2026-10-10T12:00:00Z`). In-house and UTC only: there are no time zones to get wrong.
//!
//! Dates are converted with the proleptic Gregorian calendar (Howard Hinnant's `days_from_civil` and
//! `civil_from_days`), counting days from 1970-01-01. Leap seconds don't exist in Unix time, so `:60` is refused.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::CoreError;

const MILLISECONDS_PER_SECOND: i64 = 1000;
const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 3600;
const SECONDS_PER_DAY: i64 = 86_400;
const MILLISECONDS_PER_DAY: i64 = SECONDS_PER_DAY * MILLISECONDS_PER_SECOND;
// The calendar repeats every 400 years, which have 146,097 days; 1970-01-01 is day 719,468 counted from
// 0000-03-01, the start of Hinnant's March-based year.
const DAYS_PER_ERA: i64 = 146_097;
const YEARS_PER_ERA: i64 = 400;
const EPOCH_DAY_FROM_MARCH_ZERO: i64 = 719_468;
// RFC 3339 has four-digit years.
const FIRST_YEAR: i64 = 0;
const LAST_YEAR: i64 = 9999;
// "YYYY-MM-DDTHH:MM:SS": the part before the fraction and the zone.
const DATE_TIME_LENGTH: usize = 19;
const MILLISECOND_DIGITS: usize = 3;

// Why a timestamp was refused; carried in `CoreError::InvalidTimestamp.reason`.
const BAD_LAYOUT: &str = "expected YYYY-MM-DDTHH:MM:SS, an optional fraction, then Z or an offset like +02:00";
const BAD_DATE: &str = "the month or day is out of range";
const BAD_CLOCK: &str = "the hour, minute or second is out of range (leap seconds aren't supported)";
const BAD_FRACTION: &str = "a '.' must be followed by at least one digit";
const BAD_OFFSET: &str = "the zone must be Z or an offset +HH:MM or -HH:MM within 23:59";

/// A calendar date and clock time in UTC, with milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DateTime {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    millisecond: i64,
}

/// Now, in milliseconds since 1970-01-01T00:00:00Z. A clock set before 1970 gives a negative number.
pub fn now_unix_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
        Err(before_epoch) => -i64::try_from(before_epoch.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

/// RFC 3339 text in UTC: `2026-10-10T12:00:00Z`, with `.mmm` only when the milliseconds aren't zero. Times outside
/// the years 0000 to 9999 are clamped to the nearest end, because RFC 3339 can't write them.
pub fn format_rfc3339(unix_ms: i64) -> String {
    let clamped = unix_ms.clamp(first_unix_ms(), last_unix_ms());
    let date_time = date_time_from_unix_ms(clamped);
    let mut text = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        date_time.year, date_time.month, date_time.day, date_time.hour, date_time.minute, date_time.second
    );
    if date_time.millisecond != 0 {
        text.push_str(&format!(".{:03}", date_time.millisecond));
    }
    text.push('Z');
    text
}

/// The Unix milliseconds of an RFC 3339 timestamp. `T` or `t` separates date and time; the zone is `Z`, `z` or a
/// numeric offset, which is subtracted to give UTC. Fraction digits past milliseconds are dropped.
pub fn parse_rfc3339(text: &str) -> Result<i64, CoreError> {
    let refuse = |reason| CoreError::InvalidTimestamp {
        text: text.to_string(),
        reason,
    };
    let bytes = text.as_bytes();
    if bytes.len() < DATE_TIME_LENGTH || !has_date_time_layout(&bytes[..DATE_TIME_LENGTH]) {
        return Err(refuse(BAD_LAYOUT));
    }
    let (millisecond, zone_start) = read_fraction(bytes).ok_or_else(|| refuse(BAD_FRACTION))?;
    let offset_seconds = read_offset(&bytes[zone_start..]).ok_or_else(|| refuse(BAD_OFFSET))?;
    let date_time = DateTime {
        year: digits_value(&bytes[0..4]),
        month: digits_value(&bytes[5..7]),
        day: digits_value(&bytes[8..10]),
        hour: digits_value(&bytes[11..13]),
        minute: digits_value(&bytes[14..16]),
        second: digits_value(&bytes[17..19]),
        millisecond,
    };
    if !is_valid_date(date_time.year, date_time.month, date_time.day) {
        return Err(refuse(BAD_DATE));
    }
    if date_time.hour > 23 || date_time.minute > 59 || date_time.second > 59 {
        return Err(refuse(BAD_CLOCK));
    }
    Ok(unix_ms_from_date_time(&date_time) - offset_seconds * MILLISECONDS_PER_SECOND)
}

/// `YYYY-MM-DDTHH:MM:SS`: digits where digits go, and the separators in their places.
fn has_date_time_layout(bytes: &[u8]) -> bool {
    bytes.iter().enumerate().all(|(index, byte)| match index {
        4 | 7 => *byte == b'-',
        10 => *byte == b'T' || *byte == b't',
        13 | 16 => *byte == b':',
        _ => byte.is_ascii_digit(),
    })
}

/// The milliseconds of an optional `.digits` after the seconds, and where the zone starts. None when a `.` has no
/// digits after it.
fn read_fraction(bytes: &[u8]) -> Option<(i64, usize)> {
    if bytes.get(DATE_TIME_LENGTH) != Some(&b'.') {
        return Some((0, DATE_TIME_LENGTH));
    }
    let digits_start = DATE_TIME_LENGTH + 1;
    let digit_count = bytes[digits_start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }
    let mut millisecond = 0;
    for position in 0..MILLISECOND_DIGITS {
        let digit = if position < digit_count {
            i64::from(bytes[digits_start + position] - b'0')
        } else {
            0
        };
        millisecond = millisecond * 10 + digit;
    }
    Some((millisecond, digits_start + digit_count))
}

/// The zone's offset from UTC in seconds: `Z` or `z` is 0, `+HH:MM` and `-HH:MM` their signed value. None for
/// anything else, including text after the zone.
fn read_offset(zone: &[u8]) -> Option<i64> {
    if zone == b"Z" || zone == b"z" {
        return Some(0);
    }
    let is_offset_layout = zone.len() == 6
        && (zone[0] == b'+' || zone[0] == b'-')
        && zone[1..3].iter().all(u8::is_ascii_digit)
        && zone[3] == b':'
        && zone[4..6].iter().all(u8::is_ascii_digit);
    if !is_offset_layout {
        return None;
    }
    let hours = digits_value(&zone[1..3]);
    let minutes = digits_value(&zone[4..6]);
    if hours > 23 || minutes > 59 {
        return None;
    }
    let seconds = hours * SECONDS_PER_HOUR + minutes * SECONDS_PER_MINUTE;
    Some(if zone[0] == b'-' { -seconds } else { seconds })
}

/// The value of ASCII digits the caller already checked.
fn digits_value(digits: &[u8]) -> i64 {
    digits
        .iter()
        .fold(0, |value, digit| value * 10 + i64::from(digit - b'0'))
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_valid_date(year: i64, month: i64, day: i64) -> bool {
    (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month)
}

/// Days from 1970-01-01 to the given date (negative before it).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // Hinnant's years start in March, so the leap day is the last day of the year.
    let march_year = if month <= 2 { year - 1 } else { year };
    let era = march_year.div_euclid(YEARS_PER_ERA);
    let year_of_era = march_year - era * YEARS_PER_ERA;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * DAYS_PER_ERA + day_of_era - EPOCH_DAY_FROM_MARCH_ZERO
}

/// The date (year, month, day) that is `days` after 1970-01-01. The inverse of `days_from_civil`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days_from_march_zero = days + EPOCH_DAY_FROM_MARCH_ZERO;
    let era = days_from_march_zero.div_euclid(DAYS_PER_ERA);
    let day_of_era = days_from_march_zero - era * DAYS_PER_ERA;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * YEARS_PER_ERA + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

fn unix_ms_from_date_time(date_time: &DateTime) -> i64 {
    let days = days_from_civil(date_time.year, date_time.month, date_time.day);
    let seconds = date_time.hour * SECONDS_PER_HOUR + date_time.minute * SECONDS_PER_MINUTE + date_time.second;
    days * MILLISECONDS_PER_DAY + seconds * MILLISECONDS_PER_SECOND + date_time.millisecond
}

fn date_time_from_unix_ms(unix_ms: i64) -> DateTime {
    let days = unix_ms.div_euclid(MILLISECONDS_PER_DAY);
    let millisecond_of_day = unix_ms.rem_euclid(MILLISECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let second_of_day = millisecond_of_day / MILLISECONDS_PER_SECOND;
    DateTime {
        year,
        month,
        day,
        hour: second_of_day / SECONDS_PER_HOUR,
        minute: second_of_day % SECONDS_PER_HOUR / SECONDS_PER_MINUTE,
        second: second_of_day % SECONDS_PER_MINUTE,
        millisecond: millisecond_of_day % MILLISECONDS_PER_SECOND,
    }
}

/// 0000-01-01T00:00:00.000Z.
fn first_unix_ms() -> i64 {
    days_from_civil(FIRST_YEAR, 1, 1) * MILLISECONDS_PER_DAY
}

/// 9999-12-31T23:59:59.999Z.
fn last_unix_ms() -> i64 {
    days_from_civil(LAST_YEAR + 1, 1, 1) * MILLISECONDS_PER_DAY - 1
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // 2000-03-01T00:00:00Z, a day after a leap day in a year divisible by 400.
    const MARCH_FIRST_2000_MS: i64 = 951_868_800_000;
    const GARBAGE_SEED: u64 = 0xD1B5_4A32_D192_ED03;
    const GARBAGE_CASES: usize = 3000;

    fn parse(text: &str) -> i64 {
        parse_rfc3339(text).unwrap()
    }

    fn reason_for(text: &str) -> &'static str {
        match parse_rfc3339(text) {
            Err(CoreError::InvalidTimestamp { reason, .. }) => reason,
            other => panic!("{text}: expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn epoch_edges() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(-1), "1969-12-31T23:59:59.999Z");
        assert_eq!(format_rfc3339(1), "1970-01-01T00:00:00.001Z");
        assert_eq!(parse("1970-01-01T00:00:00Z"), 0);
        assert_eq!(parse("1969-12-31T23:59:59.999Z"), -1);
    }

    #[test]
    fn leap_years() {
        assert_eq!(
            parse("2000-02-29T00:00:00Z"),
            MARCH_FIRST_2000_MS - MILLISECONDS_PER_DAY
        );
        assert_eq!(parse("2000-03-01T00:00:00Z"), MARCH_FIRST_2000_MS);
        assert_eq!(
            format_rfc3339(parse("2024-02-29T12:30:45.250Z")),
            "2024-02-29T12:30:45.250Z"
        );
        // 1900 and 2100 are divisible by 100 but not 400: no 29 February.
        assert_eq!(reason_for("1900-02-29T00:00:00Z"), BAD_DATE);
        assert_eq!(reason_for("2100-02-29T00:00:00Z"), BAD_DATE);
        assert_eq!(reason_for("2023-02-29T00:00:00Z"), BAD_DATE);
        assert_eq!(
            format_rfc3339(parse("2100-02-28T23:59:59Z") + 1000),
            "2100-03-01T00:00:00Z"
        );
    }

    #[test]
    fn known_instants() {
        assert_eq!(parse("2026-10-10T12:00:00Z"), 1_791_633_600_000);
        assert_eq!(format_rfc3339(1_791_633_600_000), "2026-10-10T12:00:00Z");
        assert_eq!(format_rfc3339(first_unix_ms()), "0000-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(last_unix_ms()), "9999-12-31T23:59:59.999Z");
        assert_eq!(format_rfc3339(i64::MAX), "9999-12-31T23:59:59.999Z");
        assert_eq!(format_rfc3339(i64::MIN), "0000-01-01T00:00:00Z");
    }

    #[test]
    fn zones_fractions_and_lower_case() {
        let noon = parse("2026-10-10T12:00:00Z");
        assert_eq!(parse("2026-10-10t12:00:00z"), noon);
        assert_eq!(parse("2026-10-10T14:30:00+02:30"), noon);
        assert_eq!(parse("2026-10-10T07:00:00-05:00"), noon);
        assert_eq!(parse("2026-10-10T12:00:00-00:00"), noon);
        assert_eq!(parse("2026-10-10T12:00:00.5Z"), noon + 500);
        assert_eq!(parse("2026-10-10T12:00:00.123456789Z"), noon + 123);
    }

    #[test]
    fn every_day_of_four_centuries_round_trips() {
        // 1900 to 2300 covers both kinds of century year and the epoch.
        let first_day = days_from_civil(1900, 1, 1);
        let last_day = days_from_civil(2300, 1, 1);
        for day in first_day..last_day {
            let (year, month, day_of_month) = civil_from_days(day);
            assert!(is_valid_date(year, month, day_of_month), "{day}");
            assert_eq!(days_from_civil(year, month, day_of_month), day);
            let unix_ms = day * MILLISECONDS_PER_DAY + 45_296_789;
            assert_eq!(parse(&format_rfc3339(unix_ms)), unix_ms);
        }
    }

    #[test]
    fn refusals_say_why() {
        let cases = [
            ("", BAD_LAYOUT),
            ("2026-10-10", BAD_LAYOUT),
            ("2026-10-10 12:00:00Z", BAD_LAYOUT),
            ("2026/10/10T12:00:00Z", BAD_LAYOUT),
            ("+026-10-10T12:00:00Z", BAD_LAYOUT),
            ("2026-13-10T12:00:00Z", BAD_DATE),
            ("2026-04-31T12:00:00Z", BAD_DATE),
            ("2026-10-00T12:00:00Z", BAD_DATE),
            ("2026-10-10T24:00:00Z", BAD_CLOCK),
            ("2016-12-31T23:59:60Z", BAD_CLOCK),
            ("2026-10-10T12:00:00.Z", BAD_FRACTION),
            ("2026-10-10T12:00:00", BAD_OFFSET),
            ("2026-10-10T12:00:00+0200", BAD_OFFSET),
            ("2026-10-10T12:00:00+24:00", BAD_OFFSET),
            ("2026-10-10T12:00:00Zjunk", BAD_OFFSET),
        ];
        for (text, reason) in cases {
            assert_eq!(reason_for(text), reason, "{text:?}");
        }
    }

    // Timestamps come from user files: no input may panic the parser.
    #[test]
    fn garbage_never_panics() {
        let mut state = GARBAGE_SEED;
        let alphabet = b"0123456789-:T.Z+z ";
        for _ in 0..GARBAGE_CASES {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let length = (state % 32) as usize;
            let text: String = (0..length)
                .map(|index| char::from(alphabet[((state >> (index % 60)) as usize + index) % alphabet.len()]))
                .collect();
            let _ = parse_rfc3339(&text);
            let _ = parse_rfc3339(&format!("2026-10-10T12:00:00{text}"));
        }
    }
}
