//! Formatting helpers for readouts, file names and dates.
//!
//! Dates are computed from the system clock in UTC with the proleptic
//! Gregorian calendar (Howard Hinnant's `civil_from_days` algorithm), so no
//! time-zone database is needed.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::InterfaceError;

/// Bytes with binary prefixes: `512 B`, `1.5 KiB`, `12.3 MiB`, `1.20 GiB`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Integer with thousands separators: `1,234,567`.
pub fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Compact number for readouts: integers in full up to 99,999, then SI
/// suffixes (`1.23k`, `45.6M`, `7.8G`); fractions with 3 significant digits.
pub fn format_compact(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_string();
    }
    let magnitude = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    if magnitude >= 1e5 {
        let (scaled, suffix) = if magnitude >= 1e12 {
            (magnitude / 1e12, "T")
        } else if magnitude >= 1e9 {
            (magnitude / 1e9, "G")
        } else if magnitude >= 1e6 {
            (magnitude / 1e6, "M")
        } else {
            (magnitude / 1e3, "k")
        };
        return format!("{sign}{}{suffix}", three_significant(scaled));
    }
    if value.fract() == 0.0 {
        return format!("{sign}{}", format_count(magnitude as u64));
    }
    format!("{sign}{}", three_significant(magnitude))
}

fn three_significant(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else if v >= 1.0 {
        format!("{v:.2}")
    } else if v == 0.0 {
        "0".to_string()
    } else {
        // Three significant digits for small magnitudes.
        let decimals = (2 - v.log10().floor() as i32).clamp(3, 9) as usize;
        format!("{v:.decimals$}")
    }
}

/// Milliseconds: `0.042 ms`, `12.3 ms`, `1.20 s`.
pub fn format_ms(ms: f64) -> String {
    if !ms.is_finite() || ms < 0.0 {
        "—".to_string()
    } else if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else if ms >= 10.0 {
        format!("{ms:.1} ms")
    } else if ms >= 0.1 {
        format!("{ms:.2} ms")
    } else {
        format!("{ms:.3} ms")
    }
}

/// Rate per second: `0/s`, `12.5/s`, `1.23k/s`.
pub fn format_rate(per_second: f64) -> String {
    if !per_second.is_finite() {
        return "—".to_string();
    }
    let rounded = if per_second < 100.0 { (per_second * 10.0).round() / 10.0 } else { per_second.round() };
    format!("{}/s", format_compact(rounded))
}

/// Capitalises the first letter of a tick unit (`generation` → `Generation`).
pub fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A UTC calendar date and time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtcDateTime {
    /// Year (proleptic Gregorian).
    pub year: i64,
    /// Month 1–12.
    pub month: u32,
    /// Day 1–31.
    pub day: u32,
    /// Hour 0–23.
    pub hour: u32,
    /// Minute 0–59.
    pub minute: u32,
    /// Second 0–59.
    pub second: u32,
}

impl UtcDateTime {
    /// Date and time of `seconds` since the Unix epoch.
    pub fn from_unix_seconds(seconds: i64) -> Self {
        let days = seconds.div_euclid(86_400);
        let secs = seconds.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);
        Self { year, month, day, hour: secs / 3600, minute: secs % 3600 / 60, second: secs % 60 }
    }

    /// Current UTC time from the system clock (the epoch if the clock is
    /// before 1970).
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    /// Converts a [`SystemTime`].
    pub fn from_system_time(time: SystemTime) -> Self {
        let seconds = match time.duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX),
        };
        Self::from_unix_seconds(seconds)
    }

    /// ISO 8601: `2026-11-09T14:03:12Z` (stored in file envelopes).
    pub fn iso8601(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// Compact stamp for file names: `20261109-140312`.
    pub fn file_stamp(&self) -> String {
        format!("{:04}{:02}{:02}-{:02}{:02}{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second)
    }
}

/// `(year, month, day)` of a day count since 1970-01-01 (Hinnant's
/// `civil_from_days`, exact for the whole `i64` day range used here).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Shows an ISO 8601 UTC timestamp (`2026-11-09T14:03:12Z`) as
/// `2026-11-09 14:03 UTC`; other strings are returned unchanged.
pub fn pretty_timestamp(iso: &str) -> String {
    let b = iso.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    let shaped = b.len() == 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19)
        && b[19] == b'Z';
    if shaped { format!("{} {} UTC", &iso[0..10], &iso[11..16]) } else { iso.to_string() }
}

/// Maximum length of a user-chosen save name (characters).
pub const MAX_NAME_CHARS: usize = 80;

/// Turns a user-chosen name into a safe file stem: path separators and
/// characters reserved on common file systems become `-`, surrounding
/// whitespace and dots are trimmed. Empty or over-long names are errors.
pub fn sanitize_file_stem(name: &str) -> Result<String, InterfaceError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(InterfaceError::InvalidName("the name is empty".to_string()));
    }
    if trimmed.chars().count() > MAX_NAME_CHARS {
        return Err(InterfaceError::InvalidName(format!("the name is longer than {MAX_NAME_CHARS} characters")));
    }
    let mapped: String = trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let stem = mapped.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if stem.is_empty() || stem.chars().all(|c| c == '-') {
        return Err(InterfaceError::InvalidName(format!("`{trimmed}` contains no usable characters")));
    }
    Ok(stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_counts() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.50 KiB");
        assert_eq!(format_bytes(12 * 1024 * 1024 + 300 * 1024), "12.3 MiB");
        assert_eq!(format_bytes(300 * 1024 * 1024 * 1024), "300 GiB");
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1,000");
        assert_eq!(format_count(1_234_567), "1,234,567");
    }

    #[test]
    fn compact_numbers_rates_and_durations() {
        assert_eq!(format_compact(42.0), "42");
        assert_eq!(format_compact(99_999.0), "99,999");
        assert_eq!(format_compact(123_456.0), "123k");
        assert_eq!(format_compact(4_560_000.0), "4.56M");
        assert_eq!(format_compact(-2.5), "-2.50");
        assert_eq!(format_compact(0.012_34), "0.0123");
        assert_eq!(format_compact(f64::NAN), "—");
        assert_eq!(format_rate(12.46), "12.5/s");
        assert_eq!(format_rate(1234.4), "1,234/s");
        assert_eq!(format_ms(0.0421), "0.042 ms");
        assert_eq!(format_ms(12.34), "12.3 ms");
        assert_eq!(format_ms(1500.0), "1.50 s");
        assert_eq!(title_case("generation"), "Generation");
        assert_eq!(title_case(""), "");
    }

    #[test]
    fn calendar_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // 2000-02-29 (leap day of a year divisible by 400) is day 11 016.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        // 2026-11-09 is day 20 766.
        assert_eq!(civil_from_days(20_766), (2026, 11, 9));
        let t = UtcDateTime::from_unix_seconds(20_766 * 86_400 + 14 * 3600 + 3 * 60 + 12);
        assert_eq!(t.iso8601(), "2026-11-09T14:03:12Z");
        assert_eq!(t.file_stamp(), "20261109-140312");
        assert_eq!(pretty_timestamp(&t.iso8601()), "2026-11-09 14:03 UTC");
        assert_eq!(pretty_timestamp("yesterday"), "yesterday");
        assert_eq!(UtcDateTime::from_unix_seconds(-1).iso8601(), "1969-12-31T23:59:59Z");
    }

    /// Independent Gregorian month lengths (leap years: divisible by 4,
    /// except centuries not divisible by 400).
    fn days_in_month(year: i64, month: u32) -> u32 {
        match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
            _ => 28,
        }
    }

    #[test]
    fn calendar_is_consistent_over_four_centuries() {
        let mut previous = civil_from_days(-1);
        for day in 0..146_097 {
            let (y, m, d) = civil_from_days(day);
            let (py, pm, pd) = previous;
            let expected = if pd < days_in_month(py, pm) {
                (py, pm, pd + 1)
            } else if pm < 12 {
                (py, pm + 1, 1)
            } else {
                (py + 1, 1, 1)
            };
            assert_eq!((y, m, d), expected, "day {day} after {previous:?}");
            previous = (y, m, d);
        }
        // 400 Gregorian years are exactly 146,097 days.
        assert_eq!(civil_from_days(146_097), (2370, 1, 1));
    }

    #[test]
    fn file_stems_are_sanitized() {
        assert_eq!(sanitize_file_stem("  My glider gun ").unwrap(), "My glider gun");
        assert_eq!(sanitize_file_stem("a/b\\c:d").unwrap(), "a-b-c-d");
        assert_eq!(sanitize_file_stem("..hidden..").unwrap(), "hidden");
        assert_eq!(sanitize_file_stem("Lenia ørbium").unwrap(), "Lenia ørbium");
        assert!(sanitize_file_stem("   ").is_err());
        assert!(sanitize_file_stem("///").is_err());
        assert!(sanitize_file_stem(&"x".repeat(81)).is_err());
    }
}
