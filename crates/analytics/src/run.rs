//! Run records and multi-run comparison.
//!
//! A [`RunRecord`] captures everything needed to interpret and reproduce one
//! simulation run: identity (model, scenario, seed, parameters), the metric
//! time series, a free-form numeric summary and notable events.
//! [`compare_runs`] reduces several records to a [`ComparisonTable`] with
//! per-run statistics of every metric, and [`align_runs`] lines series up by
//! tick for side-by-side plots or CSV export.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use simulation_core::{Metrics, ParamSet, SimEvent};

use crate::dynamics::extinction_tick;
use crate::error::AnalyticsError;
use crate::series::{Point, Series, TimeSeriesStore};
use crate::stats::{RunningStats, Summary, summarize};

/// Complete record of one simulation run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Unique identifier of the run (chosen by the caller).
    pub run_id: String,
    /// Model identifier (`ModelDescriptor::id`).
    pub model_id: String,
    /// Scenario name or path.
    pub scenario: String,
    /// Seed of the run.
    pub seed: u64,
    /// Parameter values in effect at the start of the run.
    pub params: ParamSet,
    /// Start time as supplied by the caller (e.g. an RFC 3339 timestamp); the
    /// analytics crate never reads the clock, so records stay reproducible.
    pub started_at: String,
    /// Number of ticks simulated (the highest recorded tick).
    pub ticks: u64,
    /// Metric time series.
    pub series: TimeSeriesStore,
    /// Free-form numeric results (totals, scores...). Values must be finite.
    pub summary: BTreeMap<String, f64>,
    /// Notable events in chronological order.
    pub events: Vec<SimEvent>,
}

impl RunRecord {
    /// Empty record for a new run.
    pub fn new(run_id: &str, model_id: &str, scenario: &str, seed: u64, params: ParamSet, started_at: &str) -> Self {
        Self {
            run_id: run_id.to_string(),
            model_id: model_id.to_string(),
            scenario: scenario.to_string(),
            seed,
            params,
            started_at: started_at.to_string(),
            ticks: 0,
            series: TimeSeriesStore::new(),
            summary: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    /// Records a measurement (see [`TimeSeriesStore::record`]) and advances
    /// [`RunRecord::ticks`].
    pub fn record(&mut self, metrics: &Metrics) -> Result<(), AnalyticsError> {
        self.series.record(metrics)?;
        self.ticks = self.ticks.max(metrics.tick);
        Ok(())
    }

    /// Appends an event.
    pub fn push_event(&mut self, event: SimEvent) {
        self.events.push(event);
    }

    /// Sets a summary value (rejecting non-finite numbers).
    pub fn set_summary(&mut self, key: &str, value: f64) -> Result<(), AnalyticsError> {
        if !value.is_finite() {
            return Err(AnalyticsError::invalid("run summary", format!("value of `{key}` is not a finite number")));
        }
        self.summary.insert(key.to_string(), value);
        Ok(())
    }

    /// Checks invariants that the type system cannot express (finite summary
    /// values, `ticks` covering the series).
    pub fn validate(&self) -> Result<(), AnalyticsError> {
        if let Some((key, _)) = self.summary.iter().find(|(_, v)| !v.is_finite()) {
            return Err(AnalyticsError::InvalidData(format!(
                "run `{}`: summary value `{key}` is not finite",
                self.run_id
            )));
        }
        if let Some(last) = self.series.last_tick()
            && last > self.ticks
        {
            return Err(AnalyticsError::InvalidData(format!(
                "run `{}`: series reach tick {last} but the run has only {} ticks",
                self.run_id, self.ticks
            )));
        }
        Ok(())
    }

    /// Statistics of one metric of this run.
    pub fn metric_stats(&self, key: &str, extinction_threshold: f64) -> Option<MetricStats> {
        self.series.get(key).and_then(|s| MetricStats::from_series(s, extinction_threshold))
    }
}

/// Statistics of one metric in one run.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricStats {
    /// Number of samples.
    pub samples: usize,
    /// Tick of the first sample.
    pub first_tick: u64,
    /// Tick of the last sample.
    pub final_tick: u64,
    /// Value of the last sample.
    pub final_value: f64,
    /// Mean over all samples.
    pub mean: f64,
    /// Population standard deviation over all samples.
    pub std_dev: f64,
    /// Minimum value.
    pub min: f64,
    /// First tick at which the minimum was reached.
    pub min_tick: u64,
    /// Maximum value.
    pub max: f64,
    /// First tick at which the maximum was reached.
    pub max_tick: u64,
    /// Tick of permanent extinction (`value ≤ threshold` until the end).
    pub extinction_tick: Option<u64>,
}

impl MetricStats {
    /// Whether every floating-point statistic is finite.
    pub fn is_finite(&self) -> bool {
        [self.final_value, self.mean, self.std_dev, self.min, self.max].iter().all(|v| v.is_finite())
    }

    /// Statistics of a series (`None` when empty).
    pub fn from_series(series: &Series, extinction_threshold: f64) -> Option<Self> {
        let points = series.points();
        let (first, last) = (points.first()?, points.last()?);
        let mut stats = RunningStats::new();
        let (mut min, mut max) = (*first, *first);
        for p in points {
            // Series values are finite by construction.
            stats.add(p.value);
            if p.value < min.value {
                min = *p;
            }
            if p.value > max.value {
                max = *p;
            }
        }
        Some(Self {
            samples: points.len(),
            first_tick: first.tick,
            final_tick: last.tick,
            final_value: last.value,
            mean: stats.mean()?,
            std_dev: stats.std_dev()?,
            min: min.value,
            min_tick: min.tick,
            max: max.value,
            max_tick: max.tick,
            extinction_tick: extinction_tick(points, extinction_threshold),
        })
    }
}

/// Identity of a run in a [`ComparisonTable`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunHeader {
    /// Run identifier.
    pub run_id: String,
    /// Model identifier.
    pub model_id: String,
    /// Scenario.
    pub scenario: String,
    /// Seed.
    pub seed: u64,
    /// Ticks simulated.
    pub ticks: u64,
}

/// One metric across all compared runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricComparison {
    /// Metric key.
    pub key: String,
    /// Statistics per run, aligned with [`ComparisonTable::runs`] (`None`
    /// when the run has no samples of this metric).
    pub per_run: Vec<Option<MetricStats>>,
    /// Distribution of the final values over the runs that have the metric
    /// (useful for replicate runs with different seeds).
    pub final_across_runs: Option<Summary>,
}

/// One summary key across all compared runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SummaryComparison {
    /// Summary key.
    pub key: String,
    /// Value per run, aligned with [`ComparisonTable::runs`].
    pub per_run: Vec<Option<f64>>,
    /// Distribution over the runs that have the key.
    pub across_runs: Option<Summary>,
}

/// Result of [`compare_runs`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonTable {
    /// Compared runs, in input order.
    pub runs: Vec<RunHeader>,
    /// Metrics in order of first appearance over the runs.
    pub metrics: Vec<MetricComparison>,
    /// Summary keys in sorted order.
    pub summaries: Vec<SummaryComparison>,
    /// Threshold used for extinction detection.
    pub extinction_threshold: f64,
}

impl ComparisonTable {
    /// Comparison row of a metric.
    pub fn metric(&self, key: &str) -> Option<&MetricComparison> {
        self.metrics.iter().find(|m| m.key == key)
    }

    /// Comparison row of a summary key.
    pub fn summary(&self, key: &str) -> Option<&SummaryComparison> {
        self.summaries.iter().find(|m| m.key == key)
    }

    /// Checks invariants the public fields cannot enforce: every row has one
    /// entry per run and every number is finite (JSON cannot represent NaN or
    /// infinity).
    pub fn validate(&self) -> Result<(), AnalyticsError> {
        let runs = self.runs.len();
        let bad = |what: String| Err(AnalyticsError::InvalidData(format!("comparison table: {what}")));
        if !self.extinction_threshold.is_finite() {
            return bad("the extinction threshold is not finite".to_string());
        }
        for m in &self.metrics {
            if m.per_run.len() != runs {
                return bad(format!("metric `{}` has {} entries for {runs} runs", m.key, m.per_run.len()));
            }
            let stats = m.per_run.iter().flatten();
            if !stats.clone().all(MetricStats::is_finite) || !m.final_across_runs.iter().all(summary_is_finite) {
                return bad(format!("metric `{}` has a non-finite statistic", m.key));
            }
        }
        for s in &self.summaries {
            if s.per_run.len() != runs {
                return bad(format!("summary `{}` has {} entries for {runs} runs", s.key, s.per_run.len()));
            }
            if !s.per_run.iter().flatten().all(|v| v.is_finite()) || !s.across_runs.iter().all(summary_is_finite) {
                return bad(format!("summary `{}` has a non-finite value", s.key));
            }
        }
        Ok(())
    }
}

fn summary_is_finite(s: &Summary) -> bool {
    [s.mean, s.std_dev, s.min, s.q1, s.median, s.q3, s.max].iter().all(|v| v.is_finite())
}

/// Options of [`compare_runs_with`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComparisonOptions {
    /// A metric is extinct once it stays at or below this value until the
    /// end of the run (default 0; must be finite). Extinction is computed for
    /// every metric but is only meaningful for populations and other gauges.
    pub extinction_threshold: f64,
}

/// Compares runs with default options (extinction threshold 0).
pub fn compare_runs(runs: &[RunRecord]) -> ComparisonTable {
    build_comparison(runs, 0.0)
}

/// Compares runs metric by metric. Metrics appear in order of first
/// appearance over the runs, summary keys in sorted order. Non-finite summary
/// values (which [`RunRecord::set_summary`] rejects but the public field
/// allows) are treated as missing. A non-finite extinction threshold is an
/// error.
pub fn compare_runs_with(runs: &[RunRecord], options: &ComparisonOptions) -> Result<ComparisonTable, AnalyticsError> {
    if !options.extinction_threshold.is_finite() {
        return Err(AnalyticsError::invalid(
            "run comparison",
            format!("extinction threshold {} is not a finite number", options.extinction_threshold),
        ));
    }
    Ok(build_comparison(runs, options.extinction_threshold))
}

fn build_comparison(runs: &[RunRecord], extinction_threshold: f64) -> ComparisonTable {
    let headers = runs
        .iter()
        .map(|r| RunHeader {
            run_id: r.run_id.clone(),
            model_id: r.model_id.clone(),
            scenario: r.scenario.clone(),
            seed: r.seed,
            ticks: r.ticks,
        })
        .collect();

    let mut keys: Vec<&str> = Vec::new();
    for run in runs {
        for key in run.series.keys() {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    let metrics = keys
        .into_iter()
        .map(|key| {
            let per_run: Vec<Option<MetricStats>> =
                runs.iter().map(|r| r.metric_stats(key, extinction_threshold)).collect();
            let finals: Vec<f64> = per_run.iter().flatten().map(|s| s.final_value).collect();
            MetricComparison { key: key.to_string(), per_run, final_across_runs: summarize(&finals).ok() }
        })
        .collect();

    let summary_keys: std::collections::BTreeSet<&str> =
        runs.iter().flat_map(|r| r.summary.keys().map(String::as_str)).collect();
    let summaries = summary_keys
        .into_iter()
        .map(|key| {
            let per_run: Vec<Option<f64>> =
                runs.iter().map(|r| r.summary.get(key).copied().filter(|v| v.is_finite())).collect();
            let present: Vec<f64> = per_run.iter().flatten().copied().collect();
            SummaryComparison { key: key.to_string(), per_run, across_runs: summarize(&present).ok() }
        })
        .collect();

    ComparisonTable { runs: headers, metrics, summaries, extinction_threshold }
}

/// How [`align_series`] combines the tick axes of several series.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    /// Every tick present in at least one series (missing cells are `None`).
    #[default]
    Union,
    /// Only ticks present in every series.
    Intersection,
}

/// Several series on a common tick axis: `columns[c][r]` is the value of
/// column `c` at `ticks[r]`. Imported documents are validated (one column
/// per label, one cell per tick, strictly increasing ticks); for values built
/// by hand, a missing cell reads as `None`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "AlignedData")]
pub struct AlignedSeries {
    /// Common tick axis in increasing order.
    pub ticks: Vec<u64>,
    /// Column labels.
    pub labels: Vec<String>,
    /// One column per label.
    pub columns: Vec<Vec<Option<f64>>>,
}

/// Serialised form of [`AlignedSeries`], validated on import.
#[derive(Deserialize)]
struct AlignedData {
    ticks: Vec<u64>,
    labels: Vec<String>,
    columns: Vec<Vec<Option<f64>>>,
}

impl TryFrom<AlignedData> for AlignedSeries {
    type Error = AnalyticsError;

    fn try_from(data: AlignedData) -> Result<Self, Self::Error> {
        if data.columns.len() != data.labels.len() {
            return Err(AnalyticsError::InvalidData(format!(
                "aligned series has {} labels but {} columns",
                data.labels.len(),
                data.columns.len()
            )));
        }
        if let Some((label, column)) = data.labels.iter().zip(&data.columns).find(|(_, c)| c.len() != data.ticks.len())
        {
            return Err(AnalyticsError::InvalidData(format!(
                "column `{label}` has {} cells for {} ticks",
                column.len(),
                data.ticks.len()
            )));
        }
        if let Some(w) = data.ticks.windows(2).find(|w| w[0] >= w[1]) {
            return Err(AnalyticsError::InvalidData(format!(
                "aligned series ticks are not increasing ({} then {})",
                w[0], w[1]
            )));
        }
        Ok(Self { ticks: data.ticks, labels: data.labels, columns: data.columns })
    }
}

impl AlignedSeries {
    /// Number of rows (ticks).
    pub fn rows(&self) -> usize {
        self.ticks.len()
    }

    /// Value of column `c` at row `r` (`None` for a gap or a missing cell).
    pub fn cell(&self, c: usize, r: usize) -> Option<f64> {
        self.columns.get(c).and_then(|column| column.get(r)).copied().flatten()
    }

    /// Values of row `r` in label order.
    pub fn row(&self, r: usize) -> Option<Vec<Option<f64>>> {
        (r < self.ticks.len()).then(|| (0..self.labels.len()).map(|c| self.cell(c, r)).collect())
    }
}

/// Aligns labelled series (each in strictly increasing tick order) on a
/// common tick axis.
pub fn align_series(series: &[(&str, &[Point])], alignment: Alignment) -> AlignedSeries {
    let mut ticks: Vec<u64> = series.iter().flat_map(|(_, pts)| pts.iter().map(|p| p.tick)).collect();
    ticks.sort_unstable();
    ticks.dedup();
    if alignment == Alignment::Intersection {
        ticks.retain(|t| series.iter().all(|(_, pts)| pts.binary_search_by_key(t, |p| p.tick).is_ok()));
    }
    let columns = series
        .iter()
        .map(|(_, pts)| {
            let mut column = Vec::with_capacity(ticks.len());
            let mut i = 0;
            for &t in &ticks {
                while i < pts.len() && pts[i].tick < t {
                    i += 1;
                }
                column.push((i < pts.len() && pts[i].tick == t).then(|| pts[i].value));
            }
            column
        })
        .collect();
    AlignedSeries { ticks, labels: series.iter().map(|(l, _)| l.to_string()).collect(), columns }
}

/// All metrics of a store on a common tick axis (columns in first-seen order).
pub fn align_store(store: &TimeSeriesStore, alignment: Alignment) -> AlignedSeries {
    let series: Vec<(&str, &[Point])> = store.iter().map(|s| (s.key(), s.points())).collect();
    align_series(&series, alignment)
}

/// One metric of several runs on a common tick axis (one column per run,
/// labelled with the run id; runs without the metric give empty columns).
pub fn align_runs(runs: &[RunRecord], key: &str, alignment: Alignment) -> AlignedSeries {
    let series: Vec<(&str, &[Point])> =
        runs.iter().map(|r| (r.run_id.as_str(), r.series.get(key).map_or(&[][..], |s| s.points()))).collect();
    align_series(&series, alignment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::ParamValue;

    fn run(id: &str, seed: u64, populations: &[f64]) -> RunRecord {
        let params = ParamSet::new().with("growth", ParamValue::Float(0.5));
        let mut r = RunRecord::new(id, "test.model", "basic", seed, params, "2026-01-01T00:00:00Z");
        for (t, &p) in populations.iter().enumerate() {
            r.record(&Metrics::new(t as u64, t as f64).with("population", p).with("energy", p * 2.0)).unwrap();
        }
        r
    }

    #[test]
    fn record_tracks_ticks_and_summary() {
        let mut r = run("a", 1, &[1.0, 2.0, 3.0]);
        assert_eq!(r.ticks, 2);
        assert!(r.set_summary("births", f64::NAN).is_err());
        r.set_summary("births", 4.0).unwrap();
        r.push_event(SimEvent::new(1, "birth", "first birth"));
        assert_eq!(r.events.len(), 1);
        r.validate().unwrap();
        r.ticks = 1;
        assert!(r.validate().is_err());
    }

    #[test]
    fn metric_stats_are_correct() {
        let r = run("a", 1, &[4.0, 9.0, 1.0, 0.0, 0.0]);
        let s = r.metric_stats("population", 0.0).unwrap();
        assert_eq!((s.samples, s.first_tick, s.final_tick, s.final_value), (5, 0, 4, 0.0));
        assert_eq!((s.min, s.min_tick, s.max, s.max_tick), (0.0, 3, 9.0, 1));
        assert_eq!(s.mean, 14.0 / 5.0);
        assert_eq!(s.extinction_tick, Some(3));
        assert!(r.metric_stats("missing", 0.0).is_none());
    }

    #[test]
    fn comparison_table_covers_all_runs_and_metrics() {
        let mut a = run("a", 1, &[10.0, 20.0, 30.0]);
        let mut b = run("b", 2, &[10.0, 5.0, 0.0]);
        b.record(&Metrics::new(3, 3.0).with("predators", 2.0)).unwrap();
        a.set_summary("score", 1.0).unwrap();
        b.set_summary("score", 3.0).unwrap();
        b.set_summary("bonus", 7.0).unwrap();
        let table = compare_runs(&[a, b]);
        assert_eq!(table.runs.iter().map(|r| r.run_id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(
            table.metrics.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            vec!["population", "energy", "predators"]
        );
        let pop = table.metric("population").unwrap();
        assert_eq!(pop.per_run[0].unwrap().final_value, 30.0);
        assert_eq!(pop.per_run[0].unwrap().extinction_tick, None);
        assert_eq!(pop.per_run[1].unwrap().extinction_tick, Some(2));
        assert_eq!(pop.final_across_runs.unwrap().mean, 15.0);
        let predators = table.metric("predators").unwrap();
        assert!(predators.per_run[0].is_none() && predators.per_run[1].is_some());
        let score = table.summary("score").unwrap();
        assert_eq!(score.per_run, vec![Some(1.0), Some(3.0)]);
        assert_eq!(score.across_runs.unwrap().mean, 2.0);
        assert_eq!(table.summary("bonus").unwrap().per_run, vec![None, Some(7.0)]);
        assert_eq!(table.summaries.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), vec!["bonus", "score"]);
    }

    #[test]
    fn comparison_threshold_option() {
        let r = run("a", 1, &[10.0, 3.0, 2.0]);
        let table =
            compare_runs_with(std::slice::from_ref(&r), &ComparisonOptions { extinction_threshold: 5.0 }).unwrap();
        assert_eq!(table.metric("population").unwrap().per_run[0].unwrap().extinction_tick, Some(1));
        table.validate().unwrap();
        let nan = ComparisonOptions { extinction_threshold: f64::NAN };
        assert!(matches!(compare_runs_with(&[r], &nan), Err(AnalyticsError::InvalidArgument { .. })));
    }

    #[test]
    fn comparison_validation_catches_inconsistent_tables() {
        let mut table = compare_runs(&[run("a", 1, &[1.0, 2.0]), run("b", 2, &[3.0])]);
        table.validate().unwrap();
        table.metrics[0].per_run.pop();
        assert!(table.validate().is_err());
        let mut table = compare_runs(&[run("a", 1, &[1.0, 2.0])]);
        table.extinction_threshold = f64::INFINITY;
        assert!(table.validate().is_err());
    }

    #[test]
    fn alignment_union_and_intersection() {
        let a = [Point::new(0, 1.0), Point::new(2, 3.0), Point::new(4, 5.0)];
        let b = [Point::new(2, 30.0), Point::new(3, 40.0), Point::new(4, 50.0)];
        let u = align_series(&[("a", &a), ("b", &b)], Alignment::Union);
        assert_eq!(u.ticks, vec![0, 2, 3, 4]);
        assert_eq!(u.columns[0], vec![Some(1.0), Some(3.0), None, Some(5.0)]);
        assert_eq!(u.columns[1], vec![None, Some(30.0), Some(40.0), Some(50.0)]);
        assert_eq!(u.row(2), Some(vec![None, Some(40.0)]));
        assert_eq!(u.row(9), None);
        assert_eq!(u.cell(1, 0), None);
        assert_eq!(u.cell(5, 0), None);
        let i = align_series(&[("a", &a), ("b", &b)], Alignment::Intersection);
        assert_eq!(i.ticks, vec![2, 4]);
        assert_eq!(i.columns[1], vec![Some(30.0), Some(50.0)]);
    }

    #[test]
    fn alignment_matches_brute_force_lookup() {
        let a: Vec<Point> = (0..50).filter(|t| t % 3 != 0).map(|t| Point::new(t, t as f64)).collect();
        let b: Vec<Point> = (0..50).filter(|t| t % 4 == 1).map(|t| Point::new(t, -(t as f64))).collect();
        let c: Vec<Point> = (20..30).map(|t| Point::new(t, 0.5)).collect();
        let input: [(&str, &[Point]); 3] = [("a", &a), ("b", &b), ("c", &c)];
        for alignment in [Alignment::Union, Alignment::Intersection] {
            let aligned = align_series(&input, alignment);
            let lookup = |pts: &[Point], t: u64| pts.iter().find(|p| p.tick == t).map(|p| p.value);
            let expected_ticks: Vec<u64> = (0..50)
                .filter(|&t| {
                    let present = input.iter().filter(|(_, pts)| lookup(pts, t).is_some()).count();
                    if alignment == Alignment::Union { present > 0 } else { present == input.len() }
                })
                .collect();
            assert_eq!(aligned.ticks, expected_ticks, "{alignment:?}");
            for (r, &t) in aligned.ticks.iter().enumerate() {
                for (c, (_, pts)) in input.iter().enumerate() {
                    assert_eq!(aligned.cell(c, r), lookup(pts, t));
                }
            }
        }
    }

    #[test]
    fn imported_alignments_are_validated() {
        let ok = r#"{"ticks":[1,2],"labels":["a"],"columns":[[1.0,null]]}"#;
        let aligned: AlignedSeries = serde_json::from_str(ok).unwrap();
        assert_eq!(aligned.row(1), Some(vec![None]));
        let short = r#"{"ticks":[1,2],"labels":["a"],"columns":[[1.0]]}"#;
        assert!(serde_json::from_str::<AlignedSeries>(short).is_err());
        let unlabeled = r#"{"ticks":[1],"labels":[],"columns":[[1.0]]}"#;
        assert!(serde_json::from_str::<AlignedSeries>(unlabeled).is_err());
        let unordered = r#"{"ticks":[2,1],"labels":["a"],"columns":[[1.0,2.0]]}"#;
        assert!(serde_json::from_str::<AlignedSeries>(unordered).is_err());
        // Hand-built inconsistent values do not panic.
        let manual = AlignedSeries { ticks: vec![0, 1], labels: vec!["a".into()], columns: vec![vec![Some(1.0)]] };
        assert_eq!(manual.row(1), Some(vec![None]));
    }

    #[test]
    fn runs_align_by_tick() {
        let a = run("a", 1, &[1.0, 2.0]);
        let b = run("b", 2, &[5.0, 6.0, 7.0]);
        let aligned = align_runs(&[a, b], "population", Alignment::Union);
        assert_eq!(aligned.labels, vec!["a", "b"]);
        assert_eq!(aligned.ticks, vec![0, 1, 2]);
        assert_eq!(aligned.columns[0], vec![Some(1.0), Some(2.0), None]);
        assert_eq!(aligned.rows(), 3);
    }
}
