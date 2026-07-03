//! Date/time helpers shared across the app: the local "now" clock, weekday and
//! month names, and the user-configurable formats used by the `/date` and
//! `/time` slash commands and the `{{date}}` / `{{time}}` template placeholders.
//!
//! The chosen formats live in a thread-local — the app is single-UI-threaded, so
//! every window sees one value — set from Settings on launch and on change, the
//! same pattern as the PDF render-quality multiplier in [`crate::pdf`]. The
//! defaults (`iso` / `24h`) reproduce the previously hardcoded behaviour.

use std::cell::RefCell;

/// Local now, falling back to UTC when the offset can't be determined.
pub fn now_local() -> time::OffsetDateTime {
    time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc())
}

pub fn weekday_name(w: time::Weekday) -> &'static str {
    use time::Weekday::*;
    match w {
        Monday => "Monday",
        Tuesday => "Tuesday",
        Wednesday => "Wednesday",
        Thursday => "Thursday",
        Friday => "Friday",
        Saturday => "Saturday",
        Sunday => "Sunday",
    }
}

pub fn month_name(m: time::Month) -> &'static str {
    use time::Month::*;
    match m {
        January => "January",
        February => "February",
        March => "March",
        April => "April",
        May => "May",
        June => "June",
        July => "July",
        August => "August",
        September => "September",
        October => "October",
        November => "November",
        December => "December",
    }
}

/// Selectable format ids, in the order Settings offers them. The first of each
/// is the default (and matches the old hardcoded output).
pub const DATE_FORMATS: &[&str] = &["iso", "us", "eu", "long", "long_weekday", "dmy"];
pub const TIME_FORMATS: &[&str] = &["24h", "12h"];

const DEFAULT_DATE: &str = "iso";
const DEFAULT_TIME: &str = "24h";

thread_local! {
    static DATE_FMT: RefCell<String> = RefCell::new(DEFAULT_DATE.to_string());
    static TIME_FMT: RefCell<String> = RefCell::new(DEFAULT_TIME.to_string());
}

/// Set the active date format (a [`DATE_FORMATS`] id). Unknown ids fall back to
/// the ISO default when formatting, so a stale persisted value can't break.
pub fn set_date_format(id: &str) {
    DATE_FMT.with(|f| *f.borrow_mut() = id.to_string());
}

pub fn set_time_format(id: &str) {
    TIME_FMT.with(|f| *f.borrow_mut() = id.to_string());
}

/// The active date-format id (for Settings to pre-select the dropdown).
pub fn date_format() -> String {
    DATE_FMT.with(|f| f.borrow().clone())
}

pub fn time_format() -> String {
    TIME_FMT.with(|f| f.borrow().clone())
}

/// Format `dt` as a date in style `id`.
pub fn fmt_date(id: &str, dt: time::OffsetDateTime) -> String {
    let (y, m, d) = (dt.year(), u8::from(dt.month()), dt.day());
    match id {
        "us" => format!("{m:02}/{d:02}/{y:04}"),
        "eu" => format!("{d:02}/{m:02}/{y:04}"),
        "long" => format!("{} {d}, {y}", month_name(dt.month())),
        "long_weekday" => format!(
            "{}, {} {d}, {y}",
            weekday_name(dt.weekday()),
            month_name(dt.month())
        ),
        "dmy" => format!("{d} {} {y}", month_name(dt.month())),
        _ => format!("{y:04}-{m:02}-{d:02}"), // "iso" (default)
    }
}

/// Format `dt` as a time in style `id`.
pub fn fmt_time(id: &str, dt: time::OffsetDateTime) -> String {
    match id {
        "12h" => {
            let (h, ampm) = match dt.hour() {
                0 => (12, "AM"),
                12 => (12, "PM"),
                h if h < 12 => (h, "AM"),
                h => (h - 12, "PM"),
            };
            format!("{h}:{:02} {ampm}", dt.minute())
        }
        _ => format!("{:02}:{:02}", dt.hour(), dt.minute()), // "24h" (default)
    }
}

/// Current local date in the configured format (used by `/date` and `{{date}}`).
pub fn current_date() -> String {
    fmt_date(&date_format(), now_local())
}

/// Current local time in the configured format (used by `/time` and `{{time}}`).
pub fn current_time() -> String {
    fmt_time(&time_format(), now_local())
}

/// Local ISO date for a DB `datetime('now')` timestamp (UTC
/// `YYYY-MM-DD HH:MM:SS`). `None` when the string doesn't parse.
pub fn db_timestamp_local_date(ts: &str) -> Option<String> {
    let (d, t) = ts.split_once(' ')?;
    let mut dp = d.splitn(3, '-');
    let (y, m, day): (i32, u8, u8) = (
        dp.next()?.parse().ok()?,
        dp.next()?.parse().ok()?,
        dp.next()?.parse().ok()?,
    );
    let mut tp = t.splitn(3, ':');
    let (h, min, sec): (u8, u8, u8) = (
        tp.next()?.parse().ok()?,
        tp.next()?.parse().ok()?,
        tp.next()?.parse().ok()?,
    );
    let utc = time::Date::from_calendar_date(y, time::Month::try_from(m).ok()?, day)
        .ok()?
        .with_hms(h, min, sec)
        .ok()?
        .assume_utc();
    Some(fmt_date("iso", utc.to_offset(now_local().offset())))
}

/// Local ISO date for a file timestamp.
pub fn system_time_local_date(t: std::time::SystemTime) -> String {
    let dt = time::OffsetDateTime::from(t).to_offset(now_local().offset());
    fmt_date("iso", dt)
}

/// Human label for a date-format id, for the Settings dropdown.
pub fn date_format_label(id: &str) -> &'static str {
    match id {
        "us" => "US — MM/DD/YYYY",
        "eu" => "European — DD/MM/YYYY",
        "long" => "Long — Month D, YYYY",
        "long_weekday" => "Long with weekday",
        "dmy" => "Day Month Year",
        _ => "ISO — YYYY-MM-DD",
    }
}

/// Human label for a time-format id, for the Settings dropdown.
pub fn time_format_label(id: &str) -> &'static str {
    match id {
        "12h" => "12-hour — 2:30 PM",
        _ => "24-hour — 14:30",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> time::OffsetDateTime {
        // 2026-06-08 is a Monday; 14:30 local-naive, treated as UTC.
        time::Date::from_calendar_date(2026, time::Month::June, 8)
            .unwrap()
            .with_hms(14, 30, 0)
            .unwrap()
            .assume_utc()
    }

    #[test]
    fn date_formats_render_as_expected() {
        let dt = sample();
        assert_eq!(fmt_date("iso", dt), "2026-06-08");
        assert_eq!(fmt_date("us", dt), "06/08/2026");
        assert_eq!(fmt_date("eu", dt), "08/06/2026");
        assert_eq!(fmt_date("long", dt), "June 8, 2026");
        assert_eq!(fmt_date("long_weekday", dt), "Monday, June 8, 2026");
        assert_eq!(fmt_date("dmy", dt), "8 June 2026");
        // Unknown id falls back to ISO.
        assert_eq!(fmt_date("bogus", dt), "2026-06-08");
    }

    #[test]
    fn time_formats_render_as_expected() {
        let dt = sample();
        assert_eq!(fmt_time("24h", dt), "14:30");
        assert_eq!(fmt_time("12h", dt), "2:30 PM");
        // Midnight + noon edges.
        let midnight = dt.replace_hour(0).unwrap();
        let noon = dt.replace_hour(12).unwrap();
        assert_eq!(fmt_time("12h", midnight), "12:30 AM");
        assert_eq!(fmt_time("12h", noon), "12:30 PM");
    }

    #[test]
    fn db_timestamps_parse_or_bail() {
        // Midday: no plausible local offset shifts the date.
        let d = db_timestamp_local_date("2026-07-03 12:00:00").unwrap();
        assert!(d.starts_with("2026-07-0"), "{d}");
        assert!(db_timestamp_local_date("not a timestamp").is_none());
        assert!(db_timestamp_local_date("2026-13-99 12:00:00").is_none());
    }

    #[test]
    fn every_offered_id_has_a_label() {
        for id in DATE_FORMATS {
            assert!(!date_format_label(id).is_empty());
        }
        for id in TIME_FORMATS {
            assert!(!time_format_label(id).is_empty());
        }
    }
}
