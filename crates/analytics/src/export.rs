//! CSV and JSON export / import.
//!
//! CSV follows RFC 4180 field quoting: a field containing a comma, a double
//! quote, a carriage return or a line feed (or starting/ending with a space)
//! is wrapped in double quotes, and embedded quotes are doubled. Lines end
//! with `\n`. Numbers use Rust's shortest round-trip formatting, so parsing
//! an exported value gives back exactly the stored `f64`; empty cells mean
//! "no sample at this tick". The `write_*` functions stream rows to a writer
//! (memory independent of the run length); the `*_to_csv` functions return
//! the same text as a `String`.
//!
//! JSON documents carry a `schema_version` so that future builds can detect
//! and convert older exports.

use std::borrow::Cow;
use std::convert::Infallible;
use std::io::{BufWriter, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::AnalyticsError;
use crate::run::{AlignedSeries, ComparisonTable, RunRecord};
use crate::series::{Point, Series, TimeSeriesStore};

/// Schema version of the JSON documents written by this build.
pub const JSON_SCHEMA_VERSION: u32 = 1;

/// Quotes a CSV field when needed (RFC 4180).
pub fn escape_csv_field(field: &str) -> Cow<'_, str> {
    let needs_quotes = field.contains([',', '"', '\n', '\r']) || field.starts_with(' ') || field.ends_with(' ');
    if needs_quotes { Cow::Owned(format!("\"{}\"", field.replace('"', "\"\""))) } else { Cow::Borrowed(field) }
}

/// Formats a value so that it parses back to the same `f64`: integers
/// without a fractional part, moderate magnitudes in plain decimal notation,
/// very large or small magnitudes in scientific notation.
pub fn format_number(value: f64) -> String {
    let magnitude = value.abs();
    if value.fract() == 0.0 && magnitude < 1e15 {
        format!("{value:.0}")
    } else if magnitude == 0.0 || (1e-5..1e15).contains(&magnitude) {
        format!("{value}")
    } else {
        format!("{value:e}")
    }
}

fn push_record<'a>(out: &mut String, fields: impl IntoIterator<Item = Cow<'a, str>>) {
    for (i, field) in fields.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&escape_csv_field(&field));
    }
    out.push('\n');
}

/// Destination of CSV text, written line by line.
trait CsvSink {
    type Error;
    fn emit(&mut self, text: &str) -> Result<(), Self::Error>;
}

impl CsvSink for String {
    type Error = Infallible;

    fn emit(&mut self, text: &str) -> Result<(), Infallible> {
        self.push_str(text);
        Ok(())
    }
}

/// Buffered writer sink with user-facing I/O errors.
struct IoSink<W: Write>(BufWriter<W>);

impl<W: Write> IoSink<W> {
    fn new(out: W) -> Self {
        Self(BufWriter::new(out))
    }

    fn finish(mut self) -> Result<(), AnalyticsError> {
        self.0.flush().map_err(io_error)
    }
}

impl<W: Write> CsvSink for IoSink<W> {
    type Error = AnalyticsError;

    fn emit(&mut self, text: &str) -> Result<(), AnalyticsError> {
        self.0.write_all(text.as_bytes()).map_err(io_error)
    }
}

fn io_error(e: std::io::Error) -> AnalyticsError {
    AnalyticsError::Io(e.to_string())
}

/// Text accumulated in an infallible [`String`] sink.
fn into_text(out: String, result: Result<(), Infallible>) -> String {
    match result {
        Ok(()) => out,
        Err(never) => match never {},
    }
}

impl AlignedSeries {
    /// CSV with a `tick` column followed by one column per label.
    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        push_record(
            &mut out,
            std::iter::once(Cow::Borrowed("tick")).chain(self.labels.iter().map(|l| Cow::Borrowed(l.as_str()))),
        );
        for (r, tick) in self.ticks.iter().enumerate() {
            let cells =
                (0..self.labels.len()).map(|c| Cow::Owned(self.cell(c, r).map(format_number).unwrap_or_default()));
            push_record(&mut out, std::iter::once(Cow::Owned(tick.to_string())).chain(cells));
        }
        out
    }
}

/// Streams the table of a store: a header, then one row per tick recorded by
/// any metric (a k-way merge of the series, so no intermediate table is built).
fn write_store_rows<S: CsvSink>(store: &TimeSeriesStore, sink: &mut S) -> Result<(), S::Error> {
    let mut line = String::new();
    push_record(&mut line, std::iter::once("tick").chain(store.keys()).map(Cow::Borrowed));
    sink.emit(&line)?;
    let series: Vec<&[Point]> = store.iter().map(Series::points).collect();
    let mut cursors = vec![0usize; series.len()];
    while let Some(tick) = series.iter().zip(&cursors).filter_map(|(points, &i)| points.get(i).map(|p| p.tick)).min() {
        line.clear();
        line.push_str(&tick.to_string());
        for (points, cursor) in series.iter().zip(cursors.iter_mut()) {
            line.push(',');
            // Numbers never contain characters that need quoting.
            if let Some(p) = points.get(*cursor)
                && p.tick == tick
            {
                line.push_str(&format_number(p.value));
                *cursor += 1;
            }
        }
        line.push('\n');
        sink.emit(&line)?;
    }
    Ok(())
}

/// CSV of every metric of a store: `tick` plus one column per metric in
/// first-seen order, one row per tick recorded by any metric (empty cells
/// for metrics without a sample at that tick).
pub fn store_to_csv(store: &TimeSeriesStore) -> String {
    let mut out = String::new();
    let result = write_store_rows(store, &mut out);
    into_text(out, result)
}

/// Streams [`store_to_csv`] to a writer (buffered internally).
pub fn write_store_csv<W: Write>(store: &TimeSeriesStore, out: W) -> Result<(), AnalyticsError> {
    let mut sink = IoSink::new(out);
    write_store_rows(store, &mut sink)?;
    sink.finish()
}

impl ComparisonTable {
    /// Long-format CSV: one row per (metric, run) with `source` = `metric`,
    /// the run identity and the statistics of [`crate::MetricStats`],
    /// followed by one row per (summary key, run) with `source` = `summary`
    /// and the value in the `final` column. Runs without the metric or key
    /// have no row.
    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        let header = [
            "source",
            "key",
            "run_id",
            "model_id",
            "scenario",
            "seed",
            "samples",
            "first_tick",
            "final_tick",
            "final",
            "mean",
            "std_dev",
            "min",
            "min_tick",
            "max",
            "max_tick",
            "extinction_tick",
        ];
        push_record(&mut out, header.iter().map(|h| Cow::Borrowed(*h)));
        let num = |v: f64| Cow::Owned(format_number(v));
        let int = |v: u64| Cow::Owned(v.to_string());
        for metric in &self.metrics {
            for (run, stats) in self.runs.iter().zip(&metric.per_run) {
                let Some(s) = stats else { continue };
                let row = [
                    Cow::Borrowed("metric"),
                    Cow::Borrowed(metric.key.as_str()),
                    Cow::Borrowed(run.run_id.as_str()),
                    Cow::Borrowed(run.model_id.as_str()),
                    Cow::Borrowed(run.scenario.as_str()),
                    int(run.seed),
                    int(s.samples as u64),
                    int(s.first_tick),
                    int(s.final_tick),
                    num(s.final_value),
                    num(s.mean),
                    num(s.std_dev),
                    num(s.min),
                    int(s.min_tick),
                    num(s.max),
                    int(s.max_tick),
                    Cow::Owned(s.extinction_tick.map(|t| t.to_string()).unwrap_or_default()),
                ];
                push_record(&mut out, row);
            }
        }
        for summary in &self.summaries {
            for (run, value) in self.runs.iter().zip(&summary.per_run) {
                let Some(v) = value else { continue };
                let mut row = vec![
                    Cow::Borrowed("summary"),
                    Cow::Borrowed(summary.key.as_str()),
                    Cow::Borrowed(run.run_id.as_str()),
                    Cow::Borrowed(run.model_id.as_str()),
                    Cow::Borrowed(run.scenario.as_str()),
                    int(run.seed),
                    Cow::Borrowed(""),
                    Cow::Borrowed(""),
                    Cow::Borrowed(""),
                    num(*v),
                ];
                row.resize(header.len(), Cow::Borrowed(""));
                push_record(&mut out, row);
            }
        }
        out
    }
}

/// Streams the comment lines and the table of a run.
fn write_run_rows<S: CsvSink>(run: &RunRecord, sink: &mut S) -> Result<(), S::Error> {
    // Labels and values are kept on one line each, so every comment line
    // starts with `#` whatever the run identity or parameter keys contain.
    let one_line = |text: &str| text.replace(['\n', '\r'], " ");
    let mut comment = |label: &str, value: &str| sink.emit(&format!("# {}: {}\n", one_line(label), one_line(value)));
    comment("run_id", &run.run_id)?;
    comment("model_id", &run.model_id)?;
    comment("scenario", &run.scenario)?;
    comment("seed", &run.seed.to_string())?;
    comment("started_at", &run.started_at)?;
    comment("ticks", &run.ticks.to_string())?;
    for (key, value) in run.params.iter() {
        comment(&format!("param {key}"), &value.to_string())?;
    }
    write_store_rows(&run.series, sink)
}

/// CSV of a run's metrics preceded by `#`-prefixed comment lines with the
/// run identity (model, scenario, seed, parameters, start time, ticks).
/// Spreadsheet tools that do not understand comments can skip the first
/// lines; [`store_to_csv`] produces the bare table.
pub fn run_to_csv(run: &RunRecord) -> String {
    let mut out = String::new();
    let result = write_run_rows(run, &mut out);
    into_text(out, result)
}

/// Streams [`run_to_csv`] to a writer (buffered internally).
pub fn write_run_csv<W: Write>(run: &RunRecord, out: W) -> Result<(), AnalyticsError> {
    let mut sink = IoSink::new(out);
    write_run_rows(run, &mut sink)?;
    sink.finish()
}

/// Versioned JSON document.
#[derive(Serialize)]
struct VersionedRef<'a, T> {
    schema_version: u32,
    kind: &'static str,
    data: &'a T,
}

#[derive(Deserialize)]
struct Versioned<T> {
    data: T,
}

#[derive(Deserialize)]
struct VersionProbe {
    schema_version: u32,
    kind: String,
}

fn to_json<T: Serialize>(kind: &'static str, data: &T, pretty: bool) -> Result<String, AnalyticsError> {
    let doc = VersionedRef { schema_version: JSON_SCHEMA_VERSION, kind, data };
    let result = if pretty { serde_json::to_string_pretty(&doc) } else { serde_json::to_string(&doc) };
    result.map_err(|e| AnalyticsError::Json(e.to_string()))
}

fn from_json<T: DeserializeOwned>(kind: &'static str, text: &str) -> Result<T, AnalyticsError> {
    let probe: VersionProbe = serde_json::from_str(text)
        .map_err(|e| AnalyticsError::Json(format!("not an Emergence Lab analytics document: {e}")))?;
    if probe.kind != kind {
        return Err(AnalyticsError::InvalidData(format!("expected a `{kind}` document, found `{}`", probe.kind)));
    }
    if probe.schema_version > JSON_SCHEMA_VERSION {
        return Err(AnalyticsError::UnsupportedVersion {
            what: kind,
            found: probe.schema_version,
            supported: JSON_SCHEMA_VERSION,
        });
    }
    let doc: Versioned<T> = serde_json::from_str(text).map_err(|e| AnalyticsError::Json(e.to_string()))?;
    Ok(doc.data)
}

/// JSON document of a time-series store (columnar: per series a `ticks` and
/// a `values` array).
pub fn store_to_json(store: &TimeSeriesStore, pretty: bool) -> Result<String, AnalyticsError> {
    to_json("time_series", store, pretty)
}

/// Reads a document written by [`store_to_json`], validating every series.
pub fn store_from_json(text: &str) -> Result<TimeSeriesStore, AnalyticsError> {
    from_json("time_series", text)
}

/// JSON document of a run record.
pub fn run_to_json(run: &RunRecord, pretty: bool) -> Result<String, AnalyticsError> {
    run.validate()?;
    to_json("run", run, pretty)
}

/// Reads a document written by [`run_to_json`].
pub fn run_from_json(text: &str) -> Result<RunRecord, AnalyticsError> {
    let run: RunRecord = from_json("run", text)?;
    run.validate()?;
    Ok(run)
}

/// JSON document of several run records (e.g. the replicates of an experiment).
pub fn runs_to_json(runs: &[RunRecord], pretty: bool) -> Result<String, AnalyticsError> {
    for run in runs {
        run.validate()?;
    }
    to_json("runs", &runs, pretty)
}

/// Reads a document written by [`runs_to_json`].
pub fn runs_from_json(text: &str) -> Result<Vec<RunRecord>, AnalyticsError> {
    let runs: Vec<RunRecord> = from_json("runs", text)?;
    for run in &runs {
        run.validate()?;
    }
    Ok(runs)
}

/// JSON document of a comparison table (after [`ComparisonTable::validate`]).
pub fn comparison_to_json(table: &ComparisonTable, pretty: bool) -> Result<String, AnalyticsError> {
    table.validate()?;
    to_json("comparison", table, pretty)
}

/// Reads a document written by [`comparison_to_json`], validating it.
pub fn comparison_from_json(text: &str) -> Result<ComparisonTable, AnalyticsError> {
    let table: ComparisonTable = from_json("comparison", text)?;
    table.validate()?;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::compare_runs;
    use crate::series::Point;
    use simulation_core::{Metrics, ParamSet, ParamValue, SimEvent};

    /// Minimal RFC 4180 reader used to check that exports parse back.
    fn parse_csv(text: &str) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        let mut row = Vec::new();
        let mut field = String::new();
        let mut chars = text.chars().peekable();
        let mut quoted = false;
        while let Some(c) = chars.next() {
            match (quoted, c) {
                (true, '"') if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                (true, '"') => quoted = false,
                (true, c) => field.push(c),
                (false, '"') => quoted = true,
                (false, ',') => row.push(std::mem::take(&mut field)),
                (false, '\n') => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                (false, c) => field.push(c),
            }
        }
        rows
    }

    #[test]
    fn csv_escaping_follows_rfc_4180() {
        assert_eq!(escape_csv_field("plain"), "plain");
        assert_eq!(escape_csv_field("a,b"), "\"a,b\"");
        assert_eq!(escape_csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(escape_csv_field("two\nlines"), "\"two\nlines\"");
        assert_eq!(escape_csv_field("cr\rhere"), "\"cr\rhere\"");
        assert_eq!(escape_csv_field(" padded"), "\" padded\"");
        assert_eq!(escape_csv_field(""), "");
    }

    #[test]
    fn numbers_round_trip() {
        for v in [0.0, -0.0, 1.0, -3.0, 0.1, 1.0 / 3.0, 1e-7, 6.02e23, 123_456_789.25, f64::MAX, f64::MIN_POSITIVE] {
            let s = format_number(v);
            assert_eq!(s.parse::<f64>().unwrap(), v, "{s}");
        }
        assert_eq!(format_number(42.0), "42");
        assert_eq!(format_number(0.5), "0.5");
        assert_eq!(format_number(1e-7), "1e-7");
    }

    #[test]
    fn store_csv_has_consistent_columns_and_gaps() {
        let mut store = TimeSeriesStore::new();
        store.record(&Metrics::new(0, 0.0).with("population", 10.0).with("weird, \"key\"", 0.5)).unwrap();
        store.record(&Metrics::new(1, 1.0).with("weird, \"key\"", 0.25).with("population", 11.0)).unwrap();
        store.record(&Metrics::new(2, 2.0).with("population", 12.0).with("late", 1.0)).unwrap();
        let csv = store_to_csv(&store);
        let rows = parse_csv(&csv);
        assert_eq!(rows[0], vec!["tick", "population", "weird, \"key\"", "late"]);
        assert_eq!(rows[1], vec!["0", "10", "0.5", ""]);
        assert_eq!(rows[2], vec!["1", "11", "0.25", ""]);
        assert_eq!(rows[3], vec!["2", "12", "", "1"]);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.len() == 4));
        let mut buffer = Vec::new();
        write_store_csv(&store, &mut buffer).unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), csv);
    }

    fn sample_run() -> RunRecord {
        let params = ParamSet::new()
            .with("rule", ParamValue::Text("B3/S23".into()))
            .with("density", ParamValue::Float(0.35))
            .with("size", ParamValue::Int(128));
        let mut run = RunRecord::new("run-1", "life.conway", "random soup", 42, params, "2026-11-09T10:00:00Z");
        for t in 0..20u64 {
            run.record(&Metrics::new(t, t as f64).with("population", (t * t) as f64 * 0.1)).unwrap();
        }
        run.set_summary("max_population", 36.1).unwrap();
        run.push_event(SimEvent::new(19, "stable", "pattern stabilised"));
        run
    }

    #[test]
    fn run_json_round_trip() {
        let run = sample_run();
        for pretty in [false, true] {
            let json = run_to_json(&run, pretty).unwrap();
            let back = run_from_json(&json).unwrap();
            assert_eq!(back, run);
            assert_eq!(
                back.series.get("population").unwrap().display(),
                run.series.get("population").unwrap().display()
            );
        }
        let many = runs_to_json(&[run.clone(), run.clone()], false).unwrap();
        assert_eq!(runs_from_json(&many).unwrap().len(), 2);
    }

    #[test]
    fn store_json_round_trip_and_validation() {
        let run = sample_run();
        let json = store_to_json(&run.series, false).unwrap();
        assert!(json.contains("\"ticks\":[0,1,2"));
        assert_eq!(store_from_json(&json).unwrap(), run.series);
        // Corrupted documents are rejected with explanations.
        let bad_order = json.replacen("\"ticks\":[0,1,2", "\"ticks\":[0,2,1", 1);
        assert!(matches!(store_from_json(&bad_order), Err(AnalyticsError::Json(msg)) if msg.contains("is not after")));
        let wrong_kind = run_to_json(&run, false).unwrap();
        assert!(matches!(store_from_json(&wrong_kind), Err(AnalyticsError::InvalidData(_))));
        let newer = json.replacen("\"schema_version\":1", "\"schema_version\":99", 1);
        assert!(matches!(store_from_json(&newer), Err(AnalyticsError::UnsupportedVersion { found: 99, .. })));
        assert!(store_from_json("[1, 2, 3]").is_err());
    }

    #[test]
    fn invalid_runs_are_not_exported() {
        let mut run = sample_run();
        run.summary.insert("broken".into(), f64::NAN);
        assert!(run_to_json(&run, false).is_err());
    }

    #[test]
    fn run_csv_has_metadata_comments() {
        let csv = run_to_csv(&sample_run());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "# run_id: run-1");
        assert!(lines.contains(&"# param rule: B3/S23"));
        assert!(lines.contains(&"tick,population"));
    }

    #[test]
    fn comparison_exports() {
        let mut a = sample_run();
        let mut b = sample_run();
        b.run_id = "run-2".into();
        b.seed = 7;
        b.record(&Metrics::new(20, 20.0).with("population", 0.0)).unwrap();
        a.set_summary("score", 2.5).unwrap();
        let table = compare_runs(&[a, b]);
        let rows = parse_csv(&table.to_csv());
        assert_eq!(rows[0][..3], ["source", "key", "run_id"]);
        assert!(rows.iter().all(|r| r.len() == rows[0].len()));
        let column = |name: &str| rows[0].iter().position(|h| h == name).unwrap();
        let run2 = rows.iter().find(|r| r[0] == "metric" && r[1] == "population" && r[2] == "run-2").unwrap();
        assert_eq!(run2[column("final")], "0");
        assert_eq!(run2[column("extinction_tick")], "20");
        let max_row = rows.iter().find(|r| r[0] == "summary" && r[1] == "max_population" && r[2] == "run-1").unwrap();
        assert_eq!(max_row[column("final")], "36.1");
        assert_eq!(max_row[column("mean")], "");
        let json = comparison_to_json(&table, true).unwrap();
        assert_eq!(comparison_from_json(&json).unwrap(), table);
    }

    #[test]
    fn streamed_csv_matches_aligned_table() {
        // The streaming writer must equal the union alignment of the store.
        let mut store = TimeSeriesStore::new();
        for t in 0..300u64 {
            let mut m = Metrics::new(t, t as f64);
            if t % 2 == 0 {
                m.push("even", t as f64 / 3.0);
            }
            if t % 3 == 0 {
                m.push("third", -(t as f64));
            }
            if t % 7 == 0 {
                m.push("sparse", f64::NAN);
            }
            store.record(&m).unwrap();
        }
        let csv = store_to_csv(&store);
        assert_eq!(csv, crate::run::align_store(&store, crate::run::Alignment::Union).to_csv());
        let mut streamed = Vec::new();
        write_store_csv(&store, &mut streamed).unwrap();
        assert_eq!(String::from_utf8(streamed).unwrap(), csv);
        // Values parse back bit-exactly.
        let rows = parse_csv(&csv);
        let row_3 = rows.iter().find(|r| r[0] == "3").unwrap();
        assert_eq!(row_3[1], "");
        assert_eq!(row_3[2].parse::<f64>().unwrap(), -3.0);
        let row_4 = rows.iter().find(|r| r[0] == "4").unwrap();
        assert_eq!(row_4[1].parse::<f64>().unwrap(), 4.0 / 3.0);
        // Ticks where only non-finite values arrived have no row.
        assert!(!rows.iter().any(|r| r[0] == "7"));
    }

    #[test]
    fn run_csv_comments_stay_single_line() {
        let mut run = sample_run();
        run.params.set("evil\nkey", ParamValue::Text("two\nlines".into()));
        run.scenario = "line one\r\nline two".into();
        let csv = run_to_csv(&run);
        let lines: Vec<&str> = csv.lines().collect();
        let table_start = lines.iter().position(|l| !l.starts_with('#')).unwrap();
        assert_eq!(lines[table_start], "tick,population");
        assert!(lines.contains(&"# param evil key: two lines"));
        let mut streamed = Vec::new();
        write_run_csv(&run, &mut streamed).unwrap();
        assert_eq!(String::from_utf8(streamed).unwrap(), csv);
    }

    #[test]
    fn write_errors_are_reported() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("disk full"))
            }
        }
        let err = write_store_csv(&sample_run().series, Broken).unwrap_err();
        assert!(matches!(&err, AnalyticsError::Io(m) if m.contains("disk full")), "{err}");
    }

    #[test]
    fn invalid_comparisons_are_not_exported() {
        let mut table = compare_runs(&[sample_run()]);
        table.extinction_threshold = f64::NAN;
        assert!(comparison_to_json(&table, false).is_err());
        table.extinction_threshold = 0.0;
        let json = comparison_to_json(&table, false).unwrap();
        let truncated = json.replacen("\"per_run\":[{", "\"per_run\":[null,{", 1);
        assert!(comparison_from_json(&truncated).is_err());
    }

    #[test]
    fn imported_series_keep_points() {
        let json = r#"{"schema_version":1,"kind":"time_series","data":{"series":[{"key":"x","ticks":[1,5],"values":[2.5,3]}]}}"#;
        let store = store_from_json(json).unwrap();
        assert_eq!(store.get("x").unwrap().points(), &[Point::new(1, 2.5), Point::new(5, 3.0)]);
        let mismatched =
            r#"{"schema_version":1,"kind":"time_series","data":{"series":[{"key":"x","ticks":[1],"values":[]}]}}"#;
        assert!(store_from_json(mismatched).is_err());
    }
}
