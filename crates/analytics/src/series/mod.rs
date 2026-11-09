//! Metric time series: per-key `(tick, value)` samples with display
//! decimation.
//!
//! [`TimeSeriesStore`] receives the [`simulation_core::Metrics`] a simulation
//! publishes every tick and splits them into one [`Series`] per metric key.
//! Keys are kept in the order they were first seen, which is the column order
//! of CSV exports.
//!
//! Every series keeps its samples at full resolution (optionally bounded by
//! [`StoreConfig::max_raw_points`]) and, alongside them, an incrementally
//! maintained M4 summary over equal tick spans used for plots:
//! [`Series::display`] returns every sample while the series has at most
//! [`StoreConfig::display_points`] samples and a min/max-preserving
//! decimation beyond that.

mod store;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

use crate::downsample::{self, DownsampleMethod, M4Buckets};
use crate::error::AnalyticsError;
use crate::stats::{Summary, summarize};

pub use store::TimeSeriesStore;

/// One sample of a metric.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Tick (generation) of the sample.
    pub tick: u64,
    /// Measured value (always finite inside a [`Series`]).
    pub value: f64,
}

impl Point {
    /// New sample.
    pub fn new(tick: u64, value: f64) -> Self {
        Self { tick, value }
    }
}

/// Resolution and memory settings shared by the series of a store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreConfig {
    /// Maximum number of points returned by display queries. Series with at
    /// most this many samples are displayed at full resolution. At least 4.
    pub display_points: usize,
    /// Optional cap on stored samples per series. When a series grows beyond
    /// it, its samples are decimated to at most half the cap with M4 over
    /// buckets of equal, power-of-two tick spans anchored at the first sample:
    /// the first and last samples and every bucket's first, last, minimum and
    /// maximum are kept, intermediate samples are dropped. Because the bucket
    /// grids of successive compactions nest, the stored samples are always the
    /// M4 summary of the original data, with an even time resolution over the
    /// whole run. Must be at least `2 × display_points`. `None` keeps every
    /// sample.
    pub max_raw_points: Option<usize>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self { display_points: 2_048, max_raw_points: None }
    }
}

impl StoreConfig {
    /// Checks the documented constraints.
    pub fn validate(&self) -> Result<(), AnalyticsError> {
        if self.display_points < 4 {
            return Err(AnalyticsError::invalid("store configuration", "display_points must be at least 4"));
        }
        if let Some(cap) = self.max_raw_points
            && cap < self.display_points.saturating_mul(2)
        {
            return Err(AnalyticsError::invalid(
                "store configuration",
                format!("max_raw_points ({cap}) must be at least twice display_points ({})", self.display_points),
            ));
        }
        Ok(())
    }
}

/// Samples of one metric in strictly increasing tick order.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(try_from = "SeriesData")]
pub struct Series {
    key: String,
    points: Vec<Point>,
    compacted: bool,
    config: StoreConfig,
    display: M4Buckets,
}

/// Serialised (columnar) form of a [`Series`].
#[derive(Deserialize)]
struct SeriesData {
    key: String,
    ticks: Vec<u64>,
    values: Vec<f64>,
    #[serde(default)]
    compacted: bool,
}

impl TryFrom<SeriesData> for Series {
    type Error = AnalyticsError;

    fn try_from(data: SeriesData) -> Result<Self, Self::Error> {
        if data.ticks.len() != data.values.len() {
            return Err(AnalyticsError::InvalidData(format!(
                "series `{}` has {} ticks but {} values",
                data.key,
                data.ticks.len(),
                data.values.len()
            )));
        }
        let points = data.ticks.into_iter().zip(data.values).map(|(tick, value)| Point { tick, value }).collect();
        let mut series = Series::from_points(&data.key, points)?;
        series.compacted = data.compacted;
        Ok(series)
    }
}

impl Serialize for Series {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct Ticks<'a>(&'a [Point]);
        impl Serialize for Ticks<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_seq(self.0.iter().map(|p| p.tick))
            }
        }
        struct Values<'a>(&'a [Point]);
        impl Serialize for Values<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_seq(self.0.iter().map(|p| p.value))
            }
        }
        let mut s = serializer.serialize_struct("Series", 4)?;
        s.serialize_field("key", &self.key)?;
        s.serialize_field("ticks", &Ticks(&self.points))?;
        s.serialize_field("values", &Values(&self.points))?;
        s.serialize_field("compacted", &self.compacted)?;
        s.end()
    }
}

impl Series {
    /// Empty series with the default [`StoreConfig`].
    pub fn new(key: &str) -> Self {
        Self::with_config(key, StoreConfig::default())
    }

    fn with_config(key: &str, config: StoreConfig) -> Self {
        Self {
            key: key.to_string(),
            points: Vec::new(),
            compacted: false,
            config,
            display: M4Buckets::new(config.display_points),
        }
    }

    /// Builds a series from samples, checking that ticks strictly increase
    /// and values are finite.
    pub fn from_points(key: &str, points: Vec<Point>) -> Result<Self, AnalyticsError> {
        for (i, p) in points.iter().enumerate() {
            if !p.value.is_finite() {
                return Err(AnalyticsError::InvalidData(format!(
                    "series `{key}`: value at tick {} is not a finite number",
                    p.tick
                )));
            }
            if i > 0 && points[i - 1].tick >= p.tick {
                return Err(AnalyticsError::TickOutOfOrder {
                    key: key.to_string(),
                    tick: p.tick,
                    last: points[i - 1].tick,
                });
            }
        }
        let config = StoreConfig::default();
        let display = M4Buckets::from_points(config.display_points, &points);
        Ok(Self { key: key.to_string(), points, compacted: false, config, display })
    }

    /// Re-applies a store configuration (rebuilding the display summary and
    /// compacting if the new cap requires it).
    fn configure(&mut self, config: StoreConfig) {
        self.config = config;
        self.enforce_cap();
        self.display = M4Buckets::from_points(config.display_points, &self.points);
    }

    /// Metric key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Stored samples in tick order.
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// Number of stored samples.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether the series has no samples.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// First sample.
    pub fn first(&self) -> Option<Point> {
        self.points.first().copied()
    }

    /// Most recent sample.
    pub fn last(&self) -> Option<Point> {
        self.points.last().copied()
    }

    /// Stored ticks in order.
    pub fn ticks(&self) -> impl ExactSizeIterator<Item = u64> + '_ {
        self.points.iter().map(|p| p.tick)
    }

    /// Stored values in tick order.
    pub fn values(&self) -> impl ExactSizeIterator<Item = f64> + '_ {
        self.points.iter().map(|p| p.value)
    }

    /// Value recorded exactly at `tick`.
    pub fn value_at(&self, tick: u64) -> Option<f64> {
        self.points.binary_search_by_key(&tick, |p| p.tick).ok().map(|i| self.points[i].value)
    }

    /// Samples with `start <= tick <= end` (empty when `start > end`).
    pub fn window(&self, start: u64, end: u64) -> &[Point] {
        if start > end {
            return &[];
        }
        let lo = self.points.partition_point(|p| p.tick < start);
        let hi = self.points.partition_point(|p| p.tick <= end);
        &self.points[lo..hi]
    }

    /// Whether samples were dropped by the [`StoreConfig::max_raw_points`] cap.
    pub fn is_compacted(&self) -> bool {
        self.compacted
    }

    /// Points for plotting: every sample while the series has at most
    /// [`StoreConfig::display_points`] samples, otherwise the M4 decimation
    /// (first, minimum, maximum and last sample of buckets spanning equal,
    /// power-of-two numbers of ticks), which always contains the first and
    /// last samples and the global minimum and maximum.
    pub fn display(&self) -> Vec<Point> {
        if self.points.len() <= self.config.display_points { self.points.clone() } else { self.display.points() }
    }

    /// Decimates the samples in `[start, end]` to at most `max_points` points.
    pub fn downsample_window(&self, start: u64, end: u64, max_points: usize, method: DownsampleMethod) -> Vec<Point> {
        downsample::downsample(self.window(start, end), max_points, method)
    }

    /// Summary statistics of the stored values (`None` when empty).
    pub fn summary(&self) -> Option<Summary> {
        let values: Vec<f64> = self.values().collect();
        summarize(&values).ok()
    }

    /// Appends a sample. A sample for the tick already at the end of the
    /// series replaces that value; an earlier tick or a non-finite value is
    /// rejected.
    pub fn push(&mut self, tick: u64, value: f64) -> Result<(), AnalyticsError> {
        if !value.is_finite() {
            return Err(AnalyticsError::NonFinite { what: "time series sample", index: self.points.len(), value });
        }
        match self.points.last_mut() {
            Some(last) if last.tick > tick => {
                return Err(AnalyticsError::TickOutOfOrder { key: self.key.clone(), tick, last: last.tick });
            }
            Some(last) if last.tick == tick => {
                last.value = value;
                self.refresh_last_bucket();
                return Ok(());
            }
            _ => {}
        }
        let point = Point { tick, value };
        self.points.push(point);
        self.display.push(point);
        if self.enforce_cap() {
            self.display = M4Buckets::from_points(self.config.display_points, &self.points);
        }
        Ok(())
    }

    /// Removes the last sample if it was recorded at `tick`.
    fn remove_last_at(&mut self, tick: u64) {
        if self.points.last().is_some_and(|p| p.tick == tick) {
            self.points.pop();
            self.refresh_last_bucket();
        }
    }

    /// Re-summarises the last display bucket after its samples changed.
    fn refresh_last_bucket(&mut self) {
        if let Some(start) = self.display.last_bucket_start() {
            let from = self.points.partition_point(|p| p.tick < start);
            self.display.rebuild_last(&self.points[from..]);
        }
    }

    /// Applies the raw-sample cap; returns whether samples were dropped.
    fn enforce_cap(&mut self) -> bool {
        match self.config.max_raw_points {
            Some(cap) if self.points.len() > cap => {
                // Power-of-two buckets anchored at the first sample nest
                // across compactions, so the result is always the M4 summary
                // of the original full-resolution samples.
                self.points = M4Buckets::from_points(cap / 2, &self.points).points();
                self.compacted = true;
                true
            }
            _ => false,
        }
    }

    /// Removes samples with `tick > last_kept`, e.g. after rewinding the
    /// simulation to an earlier snapshot.
    pub fn truncate_after(&mut self, last_kept: u64) {
        let keep = self.points.partition_point(|p| p.tick <= last_kept);
        if keep < self.points.len() {
            self.points.truncate(keep);
            self.display = M4Buckets::from_points(self.config.display_points, &self.points);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_is_inclusive() {
        let s = Series::from_points("x", (0..10).map(|t| Point::new(t * 10, t as f64)).collect()).unwrap();
        let w = s.window(20, 50);
        assert_eq!(w.iter().map(|p| p.tick).collect::<Vec<_>>(), vec![20, 30, 40, 50]);
        assert!(s.window(51, 59).is_empty());
        assert!(s.window(50, 20).is_empty());
        assert_eq!(s.value_at(30), Some(3.0));
        assert_eq!(s.value_at(31), None);
    }

    #[test]
    fn series_validation() {
        assert!(Series::from_points("x", vec![Point::new(2, 1.0), Point::new(2, 2.0)]).is_err());
        assert!(Series::from_points("x", vec![Point::new(1, f64::NAN)]).is_err());
        let mut s = Series::new("x");
        s.push(5, 1.0).unwrap();
        assert!(matches!(s.push(4, 1.0), Err(AnalyticsError::TickOutOfOrder { .. })));
    }
}
