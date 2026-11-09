//! # analytics
//!
//! Measurement and analysis tools shared by every Emergence Lab model, the
//! desktop dashboards and the command-line tool:
//!
//! * [`TimeSeriesStore`] / [`Series`] — per-metric `(tick, value)` series
//!   filled from [`simulation_core::Metrics`], with windows and automatic
//!   min/max-preserving decimation over equal tick spans for plots
//!   ([`downsample`]).
//! * [`stats`] — streaming moments (Welford), summaries, quantiles,
//!   correlation and regression; [`Histogram`] with fixed or automatic bins.
//! * [`diversity`] — richness, Shannon, Simpson, Pielou; Gini coefficient
//!   and Lorenz curve.
//! * [`SurvivalCurve`] — Kaplan–Meier estimates from censored lifespans.
//! * [`dynamics`] — exponential growth fits, oscillation periods by
//!   autocorrelation, extinction detection.
//! * [`RunRecord`], [`compare_runs`] and series alignment for experiments.
//! * [`export`] — CSV and versioned JSON export / import.
//!
//! The crate never reads clocks or random sources: every result is a pure
//! function of its inputs, so analyses are reproducible.

pub mod diversity;
pub mod downsample;
pub mod dynamics;
pub mod error;
pub mod export;
mod fft;
pub mod histogram;
pub mod run;
pub mod series;
pub mod stats;
pub mod survival;

pub use diversity::{Diversity, gini, lorenz_curve};
pub use downsample::DownsampleMethod;
pub use dynamics::{GrowthFit, PeriodEstimate, PeriodOptions, estimate_period, exponential_growth, extinction_tick};
pub use error::AnalyticsError;
pub use export::{
    comparison_from_json, comparison_to_json, escape_csv_field, format_number, run_from_json, run_to_csv, run_to_json,
    runs_from_json, runs_to_json, store_from_json, store_to_csv, store_to_json, write_run_csv, write_store_csv,
};
pub use histogram::{BinRule, Histogram};
pub use run::{
    AlignedSeries, Alignment, ComparisonOptions, ComparisonTable, MetricComparison, MetricStats, RunHeader, RunRecord,
    SummaryComparison, align_runs, align_series, align_store, compare_runs, compare_runs_with,
};
pub use series::{Point, Series, StoreConfig, TimeSeriesStore};
pub use stats::{LinearFit, RunningStats, Summary, summarize};
pub use survival::{Lifespan, SurvivalCurve, SurvivalStep};
