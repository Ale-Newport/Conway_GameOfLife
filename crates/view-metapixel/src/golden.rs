//! Golden values of the OTCA metapixel from Golly's own engine (`bgolly`),
//! read at run time from `patterns/otca/otca-spec.json` in the asset folder
//! (project data, see `PROVENANCE.md` next to it), and their comparison
//! with the running construction.
//!
//! The table lists, for three B3/S23 / Moore tilings (a 3×3 blinker, a
//! single ON tile and a 2×2 block), the population at `t = k·P` and the core
//! count of every tile at each readout time.

use std::path::Path;

use metapixel::spec::{PERIOD, READOUT_ON_COUNT};
use serde_json::Value;

/// File name of the machine-readable specification.
pub const SPEC_FILE: &str = "otca-spec.json";

/// One row of a golden case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoldenRow {
    /// Physical generation.
    pub t: u64,
    /// Live cells of the whole construction at `t`.
    pub population: u64,
    /// Core count of every tile (row-major) at `t` (absent at `t = 0`).
    pub core_counts: Option<Vec<u64>>,
}

/// One golden tiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoldenCase {
    /// Key in the file (`blinker3x3`...).
    pub name: String,
    /// Macro pattern rows (`.` OFF, `O` ON).
    pub model: Vec<String>,
    /// Rows by generation.
    pub rows: Vec<GoldenRow>,
}

/// The golden table, or why it could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoldenTable {
    /// Parsed cases.
    Loaded(Vec<GoldenCase>),
    /// The file is missing or malformed.
    Unavailable(String),
}

impl GoldenTable {
    /// Reads `<asset root>/patterns/otca/otca-spec.json`.
    pub fn load(asset_root: &Path) -> Self {
        let path = asset_root.join("patterns").join("otca").join(SPEC_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => match parse(&text) {
                Ok(cases) => GoldenTable::Loaded(cases),
                Err(e) => GoldenTable::Unavailable(format!("{}: {e}", path.display())),
            },
            Err(e) => GoldenTable::Unavailable(format!("{}: {e}", path.display())),
        }
    }

    /// The case whose macro pattern is `rows`.
    pub fn case_for(&self, rows: &[String]) -> Option<&GoldenCase> {
        match self {
            GoldenTable::Loaded(cases) => cases.iter().find(|c| c.model == rows),
            GoldenTable::Unavailable(_) => None,
        }
    }
}

/// Parses the `golden_or_tiling_B3S23` section of the specification.
pub fn parse(text: &str) -> Result<Vec<GoldenCase>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if root.get("period").and_then(Value::as_u64) != Some(PERIOD) {
        return Err(format!("the file's period is not {PERIOD}"));
    }
    let section =
        root.get("golden_or_tiling_B3S23").and_then(Value::as_object).ok_or("no `golden_or_tiling_B3S23` section")?;
    let mut cases = Vec::new();
    for (name, case) in section {
        let model: Vec<String> = case
            .get("model")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{name}: no model"))?
            .iter()
            .map(|r| r.as_str().map(str::to_string).ok_or_else(|| format!("{name}: bad model row")))
            .collect::<Result<_, _>>()?;
        let rows = case
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{name}: no rows"))?
            .iter()
            .map(|row| {
                let t = row.get("t").and_then(Value::as_u64).ok_or_else(|| format!("{name}: row without t"))?;
                let population = row
                    .get("population")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| format!("{name}: row without population"))?;
                let core_counts = match row.get("core_counts") {
                    None => None,
                    Some(v) => Some(
                        v.as_array()
                            .ok_or_else(|| format!("{name}: bad core counts"))?
                            .iter()
                            .map(|c| c.as_u64().ok_or_else(|| format!("{name}: bad core count")))
                            .collect::<Result<Vec<u64>, String>>()?,
                    ),
                };
                Ok(GoldenRow { t, population, core_counts })
            })
            .collect::<Result<Vec<_>, String>>()?;
        cases.push(GoldenCase { name: name.clone(), model, rows });
    }
    Ok(cases)
}

/// Outcome of one golden comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// The run reproduces the golden value.
    Pass,
    /// The run differs.
    Fail,
    /// The run has not reached this generation yet.
    Pending,
    /// The generation was passed but no value was sampled at it.
    NotSampled,
}

impl Check {
    /// Short label.
    pub fn label(self) -> &'static str {
        match self {
            Check::Pass => "match",
            Check::Fail => "DIFFERS",
            Check::Pending => "pending",
            Check::NotSampled => "not sampled",
        }
    }
}

/// Compares a golden value with a measurement taken by generation `now`.
pub fn compare(expected: u64, measured: Option<u64>, t: u64, now: u64) -> Check {
    match measured {
        Some(m) if m == expected => Check::Pass,
        Some(_) => Check::Fail,
        None if t > now => Check::Pending,
        None => Check::NotSampled,
    }
}

/// Compares core counts.
pub fn compare_cores(expected: &[u64], measured: Option<&[u64]>, t: u64, now: u64) -> Check {
    match measured {
        Some(m) if m == expected => Check::Pass,
        Some(_) => Check::Fail,
        None if t > now => Check::Pending,
        None => Check::NotSampled,
    }
}

/// Core counts implied by an exact readout: 29,795 for ON, 0 for OFF and
/// the measured count for ambiguous tiles (row-major).
pub fn core_counts_of(readings: &[metapixel::TileReading]) -> Vec<u64> {
    readings
        .iter()
        .map(|r| match r {
            metapixel::TileReading::On => READOUT_ON_COUNT,
            metapixel::TileReading::Off => 0,
            metapixel::TileReading::Ambiguous(c) => *c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_text() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/patterns/otca").join(SPEC_FILE);
        std::fs::read_to_string(path).expect("the specification ships with the workspace")
    }

    #[test]
    fn the_shipped_table_parses() {
        let cases = parse(&spec_text()).unwrap();
        let names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"blinker3x3") && names.contains(&"single1x1") && names.contains(&"block2x2"));
        let blinker = cases.iter().find(|c| c.name == "blinker3x3").unwrap();
        assert_eq!(blinker.model, ["...", "OOO", "..."]);
        // Values quoted in scenarios/metapixel/blinker.ron.
        assert_eq!(blinker.rows[0], GoldenRow { t: 0, population: 342_076, core_counts: None });
        assert_eq!(blinker.rows[1].t, PERIOD);
        assert_eq!(blinker.rows[1].population, 330_244);
        assert_eq!(blinker.rows[1].core_counts.as_deref(), Some(&[0, 0, 0, 29_795, 29_795, 29_795, 0, 0, 0][..]));
        let table = GoldenTable::Loaded(cases);
        assert!(table.case_for(&["OO".into(), "OO".into()]).is_some());
        assert!(table.case_for(&["O.".into()]).is_none());
    }

    #[test]
    fn malformed_files_are_reported() {
        assert!(parse("{").is_err());
        assert!(parse(r#"{"period": 7}"#).unwrap_err().contains("period"));
        assert!(parse(r#"{"period": 35328}"#).unwrap_err().contains("golden"));
        let bad = r#"{"period": 35328, "golden_or_tiling_B3S23": {"x": {"model": ["O"], "rows": [{"t": 0}]}}}"#;
        assert!(parse(bad).unwrap_err().contains("population"));
        let missing = GoldenTable::load(std::path::Path::new("/nonexistent"));
        assert!(matches!(missing, GoldenTable::Unavailable(ref e) if e.contains("otca-spec.json")));
        assert!(missing.case_for(&["O".into()]).is_none());
    }

    #[test]
    fn comparisons() {
        assert_eq!(compare(5, Some(5), 10, 20), Check::Pass);
        assert_eq!(compare(5, Some(6), 10, 20), Check::Fail);
        assert_eq!(compare(5, None, 30, 20), Check::Pending);
        assert_eq!(compare(5, None, 10, 20), Check::NotSampled);
        assert_eq!(compare_cores(&[0, 1], Some(&[0, 1]), 1, 1), Check::Pass);
        assert_eq!(compare_cores(&[0, 1], Some(&[1, 1]), 1, 1), Check::Fail);
        use metapixel::TileReading::*;
        assert_eq!(core_counts_of(&[On, Off, Ambiguous(12)]), [29_795, 0, 12]);
    }
}
