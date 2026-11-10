//! Plain-text tables, number formatting and timestamps.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use simulation_core::ParamValue;

/// Column alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Text.
    Left,
    /// Numbers.
    Right,
}

/// A table printed with aligned columns.
#[derive(Clone, Debug)]
pub struct Table {
    headers: Vec<(String, Align)>,
    rows: Vec<Vec<String>>,
}

impl Table {
    /// Table with these column headers.
    pub fn new(headers: &[(&str, Align)]) -> Self {
        Self { headers: headers.iter().map(|(h, a)| ((*h).to_string(), *a)).collect(), rows: Vec::new() }
    }

    /// Appends a row (missing cells are blank).
    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    /// The text, one line per row, columns separated by two spaces and
    /// trailing spaces removed.
    pub fn render(&self) -> String {
        let n = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|(h, _)| h.chars().count()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate().take(n) {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        let line = |cells: &mut dyn Iterator<Item = &str>| {
            let mut out = String::new();
            for (i, cell) in cells.enumerate().take(n) {
                if i > 0 {
                    out.push_str("  ");
                }
                let pad = widths[i].saturating_sub(cell.chars().count());
                match self.headers[i].1 {
                    Align::Left => {
                        out.push_str(cell);
                        out.push_str(&" ".repeat(pad));
                    }
                    Align::Right => {
                        out.push_str(&" ".repeat(pad));
                        out.push_str(cell);
                    }
                }
            }
            out.trim_end().to_string()
        };
        let mut text = line(&mut self.headers.iter().map(|(h, _)| h.as_str()));
        text.push('\n');
        for row in &self.rows {
            let mut cells = row.iter().map(String::as_str).chain(std::iter::repeat(""));
            text.push_str(&line(&mut cells));
            text.push('\n');
        }
        text
    }
}

/// A number for display: integers in full, others with six significant
/// digits (scientific notation outside `[1e-3, 1e9)`).
pub fn num(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        // `+ 0.0` turns a negative zero into 0.
        return format!("{:.0}", value + 0.0);
    }
    let magnitude = value.abs();
    if !(1e-3..1e9).contains(&magnitude) {
        return format!("{value:.4e}");
    }
    let decimals = (5 - magnitude.log10().floor() as i32).max(0) as usize;
    let text = format!("{value:.decimals$}");
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

/// A parameter value for display. Models that store a float parameter as
/// `f32` report it widened to `f64` (`0.006000000052154064`); such values
/// are printed with the shortest decimal that reads back as the same `f32`
/// (`0.006`), which is exactly the value the model holds.
pub fn param_value(value: &ParamValue) -> String {
    match *value {
        ParamValue::Float(x) if x.is_finite() && f64::from(x as f32) == x => (x as f32).to_string(),
        _ => value.to_string(),
    }
}

/// A path relative to the current directory when it lies inside it.
pub fn short_path(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(|p| p.display().to_string()))
        .unwrap_or_else(|| path.display().to_string())
}

/// A byte count with a binary unit.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{n} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// The current time as an RFC 3339 UTC timestamp (seconds).
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    rfc3339(secs)
}

/// RFC 3339 UTC timestamp of a Unix time.
pub fn rfc3339(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let rem = unix_seconds % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Proleptic Gregorian date of a day count since 1970-01-01 (H. Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_align_columns() {
        let mut t = Table::new(&[("KEY", Align::Left), ("VALUE", Align::Right)]);
        t.row(vec!["population".into(), "12".into()]);
        t.row(vec!["d".into(), "0.5".into()]);
        assert_eq!(t.render(), "KEY         VALUE\npopulation     12\nd             0.5\n");
    }

    #[test]
    fn numbers_are_compact() {
        assert_eq!(num(12.0), "12");
        assert_eq!(num(0.125), "0.125");
        assert_eq!(num(1234.56789), "1234.57");
        assert_eq!(num(2.0 / 3.0), "0.666667");
        assert_eq!(num(1e-7), "1.0000e-7");
        assert_eq!(num(-0.0), "0");
        assert_eq!(param_value(&ParamValue::Float(f64::from(0.006f32))), "0.006");
        assert_eq!(param_value(&ParamValue::Float(0.1)), "0.1");
        assert_eq!(param_value(&ParamValue::Float(1.0 / 3.0)), (1.0f64 / 3.0).to_string());
        assert_eq!(param_value(&ParamValue::Int(-3)), "-3");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn timestamps() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_762_819_199), "2025-11-10T23:59:59Z");
    }
}
