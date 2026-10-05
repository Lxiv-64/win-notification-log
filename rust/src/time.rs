//! Time helpers: FILETIME, Unix microseconds, the journal's ISO-8601 format.
//!
//! The journal stores timestamps exactly like the Python implementation
//! (`datetime.isoformat(timespec="microseconds")`), for example
//! `2026-09-29T11:07:26.676804+00:00`, so both implementations read the same
//! values and string ordering stays chronological.  The format is verified
//! against Python by `cargo run -- selftest`.
//!
//! No external crates: the civil-date conversion is the well-known days/era
//! algorithm and local time comes from `SystemTimeToTzSpecificLocalTime`.

use std::ffi::c_void;
use std::time::UNIX_EPOCH;

#[link(name = "kernel32")]
extern "system" {
    fn SystemTimeToTzSpecificLocalTime(
        time_zone: *const c_void,
        universal: *const WinSystemTime,
        local: *mut WinSystemTime,
    ) -> i32;
    fn TzSpecificLocalTimeToSystemTime(
        time_zone: *const c_void,
        local: *const WinSystemTime,
        universal: *mut WinSystemTime,
    ) -> i32;
}

/// Windows SYSTEMTIME layout (`std::time::SystemTime` is a different type).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct WinSystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

/// Microseconds between 1601-01-01 and 1970-01-01.
pub const FILETIME_UNIX_OFFSET_MICROS: i64 = 11_644_473_600_000_000;

/// Current time as microseconds since the Unix epoch.
pub fn now_micros() -> i64 {
    match std::time::SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_micros() as i64,
        Err(error) => -(error.duration().as_micros() as i64),
    }
}

/// Canonical journal timestamp for "now".
pub fn now_iso() -> String {
    micros_to_iso(now_micros())
}

/// Convert Windows FILETIME ticks (100 ns since 1601) into microseconds since 1970.
pub fn filetime_to_micros(ticks: i64) -> Option<i64> {
    if ticks <= 0 {
        return None;
    }
    Some(ticks / 10 - FILETIME_UNIX_OFFSET_MICROS)
}

/// Convert Windows FILETIME ticks into the journal timestamp format.
pub fn filetime_to_iso(ticks: i64) -> Option<String> {
    filetime_to_micros(ticks).map(micros_to_iso)
}

/// Format microseconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SS.ffffff+00:00`.
pub fn micros_to_iso(micros: i64) -> String {
    let seconds = micros.div_euclid(1_000_000);
    let fraction = micros.rem_euclid(1_000_000);
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = time_of_day / 3_600;
    let minute = (time_of_day % 3_600) / 60;
    let second = time_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{fraction:06}+00:00")
}

/// Parse the journal timestamp format (also accepts a trailing `Z`).
pub fn iso_to_micros(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.len() < 19 {
        return None;
    }
    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: u32 = text.get(5..7)?.parse().ok()?;
    let day: u32 = text.get(8..10)?.parse().ok()?;
    let hour: i64 = text.get(11..13)?.parse().ok()?;
    let minute: i64 = text.get(14..16)?.parse().ok()?;
    let second: i64 = text.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let fraction = text
        .get(19..)
        .and_then(|rest| rest.strip_prefix('.'))
        .map(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .unwrap_or_default();
    let mut micros: i64 = 0;
    for (index, character) in fraction.chars().take(6).enumerate() {
        let digit = character.to_digit(10)? as i64;
        micros += digit * 10i64.pow(5 - index as u32);
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 * 1_000_000 + (hour * 3_600 + minute * 60 + second) * 1_000_000 + micros)
}

/// Format a journal timestamp as local time `DD.MM.YYYY HH:MM:SS` (like the Python CLI).
pub fn format_local_iso(iso: Option<&str>) -> String {
    iso.and_then(iso_to_micros)
        .map(format_local)
        .unwrap_or_default()
}

/// Format microseconds as local time `DD.MM.YYYY HH:MM:SS`.
pub fn format_local(micros: i64) -> String {
    let utc = to_system_time(micros);
    let mut local = WinSystemTime::default();
    let ok = unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) };
    let value = if ok != 0 { local } else { utc };
    format!(
        "{:02}.{:02}.{:04} {:02}:{:02}:{:02}",
        value.day, value.month, value.year, value.hour, value.minute, value.second
    )
}

/// Local date `YYYY-MM-DD` for today (used when a time without a date is given).
pub fn today_local_date() -> String {
    let now = to_system_time(now_micros());
    let mut local = WinSystemTime::default();
    let ok = unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &now, &mut local) };
    let value = if ok != 0 { local } else { now };
    format!("{:04}-{:02}-{:02}", value.year, value.month, value.day)
}

/// Local midnight of the day `now_micros` falls on, as a journal timestamp.
///
/// The journal compares UTC strings, so "today" has to be turned into the UTC
/// instant of the local midnight - otherwise a filter would start at the wrong
/// moment for everybody east or west of Greenwich.
pub fn local_day_start_iso(now_micros: i64) -> String {
    let utc = to_system_time(now_micros);
    let mut local = WinSystemTime::default();
    let ok = unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) };
    let mut midnight = if ok != 0 { local } else { utc };
    midnight.hour = 0;
    midnight.minute = 0;
    midnight.second = 0;
    midnight.milliseconds = 0;
    local_to_utc_micros(&midnight)
        .map(micros_to_iso)
        .unwrap_or_else(|| micros_to_iso(now_micros))
}

/// Convert a local calendar date (`YYYY-MM-DD`, optionally with a time) into a
/// UTC journal timestamp; `end_of_day` picks 23:59:59 instead of 00:00:00.
pub fn local_date_to_iso(date: &str, end_of_day: bool) -> Option<String> {
    let (year, month, day) = split_date(date)?;
    let (hour, minute, second) = if end_of_day { (23, 59, 59) } else { (0, 0, 0) };
    let local = WinSystemTime {
        year: year as u16,
        month: month as u16,
        day_of_week: 0,
        day: day as u16,
        hour,
        minute,
        second,
        milliseconds: 0,
    };
    local_to_utc_micros(&local).map(micros_to_iso)
}

/// Accept the date spellings a user actually types and return `YYYY-MM-DD`.
///
/// `29.09.2026` (how the journal shows dates), `29.09.26`, `2026-09-29` and
/// `29/09/2026` all mean the same day; anything else is rejected so that a typo
/// filters nothing instead of filtering wrongly.
pub fn parse_local_date(text: &str) -> Option<String> {
    let (year, month, day) = split_date(text)?;
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Split a date in one of the accepted spellings into year, month, day.
fn split_date(text: &str) -> Option<(i32, i64, i64)> {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return None;
    }
    let separator = if cleaned.contains('-') {
        '-'
    } else if cleaned.contains('.') {
        '.'
    } else if cleaned.contains('/') {
        '/'
    } else {
        return None;
    };
    let parts: Vec<&str> = cleaned.split(separator).collect();
    if parts.len() != 3 {
        return None;
    }
    let numbers: Option<Vec<i64>> = parts
        .iter()
        .map(|part| part.trim().parse::<i64>().ok())
        .collect();
    let numbers = numbers?;
    let (year, month, day) = if parts[0].len() == 4 {
        (numbers[0], numbers[1], numbers[2])
    } else {
        let year = if numbers[2] < 100 {
            2000 + numbers[2]
        } else {
            numbers[2]
        };
        (year, numbers[1], numbers[0])
    };
    if year < 1601 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year as i32, month, day))
}

/// Turn a local SYSTEMTIME into UTC microseconds since 1970.
fn local_to_utc_micros(local: &WinSystemTime) -> Option<i64> {
    let mut universal = WinSystemTime::default();
    let ok = unsafe { TzSpecificLocalTimeToSystemTime(std::ptr::null(), local, &mut universal) };
    if ok == 0 {
        return None;
    }
    let days = days_from_civil(
        i64::from(universal.year),
        u32::from(universal.month),
        u32::from(universal.day),
    );
    let seconds = days * 86_400
        + i64::from(universal.hour) * 3_600
        + i64::from(universal.minute) * 60
        + i64::from(universal.second);
    Some(seconds * 1_000_000 + i64::from(universal.milliseconds) * 1_000)
}

/// Convert microseconds since 1970 into a SYSTEMTIME (UTC).
fn to_system_time(micros: i64) -> WinSystemTime {
    let seconds = micros.div_euclid(1_000_000);
    let fraction = micros.rem_euclid(1_000_000);
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    WinSystemTime {
        year: year as u16,
        month: month as u16,
        day_of_week: 0,
        day: day as u16,
        hour: (time_of_day / 3_600) as u16,
        minute: ((time_of_day % 3_600) / 60) as u16,
        second: (time_of_day % 60) as u16,
        milliseconds: (fraction / 1_000) as u16,
    }
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
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

/// Civil date to days since 1970-01-01.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = (year - era * 400) as u64;
    let month_prime = if month > 2 { month - 3 } else { month + 9 } as u64;
    let day_of_year = (153 * month_prime + 2) / 5 + day as u64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_unix_epoch() {
        assert_eq!(micros_to_iso(0), "1970-01-01T00:00:00.000000+00:00");
        assert_eq!(micros_to_iso(1_000_000), "1970-01-01T00:00:01.000000+00:00");
        assert_eq!(
            micros_to_iso(-1_000_000),
            "1969-12-31T23:59:59.000000+00:00"
        );
        // A record without a timestamp shows an empty line, not "1970".
        assert_eq!(format_local_iso(None), "");
        assert_eq!(
            format_local_iso(Some("2026-09-29T11:07:26.000000+00:00")).len(),
            19
        );
        assert_eq!(today_local_date().len(), 10);
    }

    #[test]
    fn handles_leap_days_and_month_ends() {
        // 2000-02-29 (leap year, century divisible by 400)
        let micros = iso_to_micros("2000-02-29T12:00:00.000000+00:00").expect("parsed");
        assert_eq!(micros_to_iso(micros), "2000-02-29T12:00:00.000000+00:00");
        // 2100-02-28 is followed by 2100-03-01 (2100 is not a leap year)
        let last_of_february = iso_to_micros("2100-02-28T23:59:59.999999+00:00").expect("parsed");
        assert_eq!(
            micros_to_iso(last_of_february + 1),
            "2100-03-01T00:00:00.000000+00:00"
        );
    }

    #[test]
    fn round_trips_microsecond_precision() {
        for text in [
            "2026-09-29T11:07:26.676804+00:00",
            "1970-01-01T00:00:00.000001+00:00",
            "2038-01-19T03:14:07.000000+00:00",
        ] {
            let micros = iso_to_micros(text).expect("parsed");
            assert_eq!(micros_to_iso(micros), text);
        }
    }

    #[test]
    fn converts_filetime() {
        let ticks = FILETIME_UNIX_OFFSET_MICROS * 10;
        assert_eq!(
            filetime_to_iso(ticks).as_deref(),
            Some("1970-01-01T00:00:00.000000+00:00")
        );
        assert_eq!(filetime_to_iso(0), None);
        assert_eq!(filetime_to_iso(-5), None);
    }

    #[test]
    fn rejects_invalid_timestamps() {
        assert!(iso_to_micros("nonsense").is_none());
        assert!(iso_to_micros("2026-99-29T11:07:26").is_none());
    }

    #[test]
    fn dates_are_accepted_in_the_spellings_people_use() {
        assert_eq!(
            parse_local_date("29.09.2026").as_deref(),
            Some("2026-09-29")
        );
        assert_eq!(parse_local_date("29.09.26").as_deref(), Some("2026-09-29"));
        assert_eq!(
            parse_local_date("2026-09-29").as_deref(),
            Some("2026-09-29")
        );
        assert_eq!(
            parse_local_date(" 29/09/2026 ").as_deref(),
            Some("2026-09-29")
        );
        assert_eq!(parse_local_date("1.10.2026").as_deref(), Some("2026-10-01"));

        assert_eq!(parse_local_date(""), None);
        assert_eq!(parse_local_date("вчера"), None, "a typo filters nothing");
        assert_eq!(parse_local_date("32.09.2026"), None, "no such day");
        assert_eq!(parse_local_date("29.13.2026"), None, "no such month");
        assert_eq!(parse_local_date("2026-09"), None, "not a full date");
    }

    #[test]
    fn a_local_day_becomes_a_utc_range() {
        let start = local_date_to_iso("29.09.2026", false).expect("start");
        let end = local_date_to_iso("29.09.2026", true).expect("end");
        assert!(start.starts_with("2026-09-2"), "{start}");
        assert!(end > start, "the end of the day comes after its start");
        let length = iso_to_micros(&end).expect("end") - iso_to_micros(&start).expect("start");
        assert!(
            (86_399_000_000..=90_000_000_000).contains(&length),
            "a day is 24 hours plus or minus an offset: {length}"
        );

        assert_eq!(local_date_to_iso("вчера", false), None);

        // Midnight of the local day around "now" is never in the future.
        let now = now_micros();
        let midnight = local_day_start_iso(now);
        let midnight_micros = iso_to_micros(&midnight).expect("midnight");
        assert!(midnight_micros <= now);
        assert!(
            now - midnight_micros < 86_400_000_000 + 3_600_000_000,
            "midnight belongs to today"
        );
    }
}
