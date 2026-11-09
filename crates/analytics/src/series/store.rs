//! [`TimeSeriesStore`]: the per-metric series of one run.

use std::collections::BTreeMap;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use simulation_core::Metrics;

use super::{Point, Series, StoreConfig};
use crate::downsample::M4Buckets;
use crate::error::AnalyticsError;

/// Per-metric time series of one run, filled from [`Metrics`] samples.
#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
#[serde(try_from = "StoreData")]
pub struct TimeSeriesStore {
    config: StoreConfig,
    series: Vec<Series>,
    index: BTreeMap<String, usize>,
    last_tick: Option<u64>,
    skipped_non_finite: u64,
}

/// Serialised form of a [`TimeSeriesStore`].
#[derive(Deserialize)]
struct StoreData {
    #[serde(default)]
    config: StoreConfig,
    series: Vec<Series>,
    #[serde(default)]
    skipped_non_finite: u64,
}

impl TryFrom<StoreData> for TimeSeriesStore {
    type Error = AnalyticsError;

    fn try_from(data: StoreData) -> Result<Self, Self::Error> {
        let mut store = TimeSeriesStore::with_config(data.config)?;
        for series in data.series {
            store.insert_series(series)?;
        }
        store.skipped_non_finite = data.skipped_non_finite;
        Ok(store)
    }
}

impl Serialize for TimeSeriesStore {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("TimeSeriesStore", 3)?;
        s.serialize_field("config", &self.config)?;
        s.serialize_field("series", &self.series)?;
        s.serialize_field("skipped_non_finite", &self.skipped_non_finite)?;
        s.end()
    }
}

impl TimeSeriesStore {
    /// Empty store with the default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Empty store with a validated configuration.
    pub fn with_config(config: StoreConfig) -> Result<Self, AnalyticsError> {
        config.validate()?;
        Ok(Self { config, ..Self::default() })
    }

    /// Current configuration.
    pub fn config(&self) -> StoreConfig {
        self.config
    }

    /// Records one measurement. Every finite value is appended to the series
    /// of its key (created on first sight). Non-finite values are not stored
    /// and are counted in [`TimeSeriesStore::skipped_non_finite`], so the
    /// series has no sample at that tick.
    ///
    /// A measurement for the most recent tick replaces the values it
    /// contains: finite values overwrite the samples of that tick and a
    /// non-finite value removes the key's sample at that tick; keys absent
    /// from the measurement keep their samples. A measurement for an earlier
    /// tick is rejected without changing the store (call
    /// [`TimeSeriesStore::truncate_after`] first when rewinding).
    pub fn record(&mut self, metrics: &Metrics) -> Result<(), AnalyticsError> {
        if let Some(last) = self.last_tick
            && metrics.tick < last
        {
            return Err(AnalyticsError::TickOutOfOrder { key: "*".to_string(), tick: metrics.tick, last });
        }
        for (key, value) in &metrics.values {
            if !value.is_finite() {
                self.skipped_non_finite += 1;
                // Only a sample recorded earlier for this same tick can be
                // removed: every series ends at or before `metrics.tick`.
                if let Some(&idx) = self.index.get(key.as_str()) {
                    self.series[idx].remove_last_at(metrics.tick);
                }
                continue;
            }
            let idx = self.series_index(key);
            // Every series ends at or before `last_tick <= metrics.tick`, so
            // this push cannot fail.
            self.series[idx].push(metrics.tick, *value)?;
        }
        self.last_tick = Some(metrics.tick);
        Ok(())
    }

    /// Records a single value for `key`, with the same ordering rules as
    /// [`TimeSeriesStore::record`] except that non-finite values are errors.
    pub fn record_value(&mut self, key: &str, tick: u64, value: f64) -> Result<(), AnalyticsError> {
        if let Some(last) = self.last_tick
            && tick < last
        {
            return Err(AnalyticsError::TickOutOfOrder { key: key.to_string(), tick, last });
        }
        if !value.is_finite() {
            return Err(AnalyticsError::NonFinite { what: "time series sample", index: 0, value });
        }
        let idx = self.series_index(key);
        self.series[idx].push(tick, value)?;
        self.last_tick = Some(tick);
        Ok(())
    }

    fn series_index(&mut self, key: &str) -> usize {
        if let Some(&i) = self.index.get(key) {
            return i;
        }
        let i = self.series.len();
        self.series.push(Series::with_config(key, self.config));
        self.index.insert(key.to_string(), i);
        i
    }

    /// Adds a complete series (e.g. built with [`Series::from_points`]). The
    /// key must not exist yet.
    pub fn insert_series(&mut self, mut series: Series) -> Result<(), AnalyticsError> {
        if self.index.contains_key(&series.key) {
            return Err(AnalyticsError::InvalidData(format!("duplicate series `{}`", series.key)));
        }
        series.configure(self.config);
        if let Some(last) = series.last() {
            self.last_tick = Some(self.last_tick.map_or(last.tick, |t| t.max(last.tick)));
        }
        self.index.insert(series.key.clone(), self.series.len());
        self.series.push(series);
        Ok(())
    }

    /// Series of `key`.
    pub fn get(&self, key: &str) -> Option<&Series> {
        self.index.get(key).map(|&i| &self.series[i])
    }

    /// Metric keys in first-seen order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.series.iter().map(|s| s.key.as_str())
    }

    /// All series in first-seen order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Series> {
        self.series.iter()
    }

    /// Most recent sample of `key`.
    pub fn last(&self, key: &str) -> Option<Point> {
        self.get(key).and_then(Series::last)
    }

    /// Display points of `key` (see [`Series::display`]).
    pub fn display(&self, key: &str) -> Option<Vec<Point>> {
        self.get(key).map(Series::display)
    }

    /// A new store containing only the samples with `start <= tick <= end`
    /// (keys without samples in the range are kept, empty).
    pub fn window(&self, start: u64, end: u64) -> TimeSeriesStore {
        let mut out = TimeSeriesStore { config: self.config, ..TimeSeriesStore::default() };
        for s in &self.series {
            let mut copy = Series::with_config(&s.key, self.config);
            copy.points = s.window(start, end).to_vec();
            copy.compacted = s.compacted;
            copy.display = M4Buckets::from_points(self.config.display_points, &copy.points);
            if let Some(last) = copy.last() {
                out.last_tick = Some(out.last_tick.map_or(last.tick, |t| t.max(last.tick)));
            }
            out.index.insert(s.key.clone(), out.series.len());
            out.series.push(copy);
        }
        out
    }

    /// Number of series.
    pub fn len(&self) -> usize {
        self.series.len()
    }

    /// Whether no metric has been recorded.
    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// Tick of the most recent measurement.
    pub fn last_tick(&self) -> Option<u64> {
        self.last_tick
    }

    /// Total number of stored samples over all series.
    pub fn total_samples(&self) -> usize {
        self.series.iter().map(Series::len).sum()
    }

    /// Number of non-finite values ignored by [`TimeSeriesStore::record`].
    pub fn skipped_non_finite(&self) -> u64 {
        self.skipped_non_finite
    }

    /// Removes samples after `last_kept` from every series.
    pub fn truncate_after(&mut self, last_kept: u64) {
        for s in &mut self.series {
            s.truncate_after(last_kept);
        }
        self.last_tick = self.series.iter().filter_map(|s| s.last().map(|p| p.tick)).max();
    }

    /// Removes every series and sample (the configuration is kept).
    pub fn clear(&mut self) {
        *self = TimeSeriesStore { config: self.config, ..TimeSeriesStore::default() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(tick: u64, values: &[(&str, f64)]) -> Metrics {
        let mut m = Metrics::new(tick, tick as f64);
        for (k, v) in values {
            m.push(k, *v);
        }
        m
    }

    #[test]
    fn records_per_key_series_in_first_seen_order() {
        let mut store = TimeSeriesStore::new();
        store.record(&metrics(0, &[("population", 10.0), ("births", 0.0)])).unwrap();
        store.record(&metrics(1, &[("population", 12.0), ("deaths", 1.0), ("births", 3.0)])).unwrap();
        assert_eq!(store.keys().collect::<Vec<_>>(), vec!["population", "births", "deaths"]);
        assert_eq!(store.get("population").unwrap().points(), &[Point::new(0, 10.0), Point::new(1, 12.0)]);
        assert_eq!(store.get("deaths").unwrap().points(), &[Point::new(1, 1.0)]);
        assert_eq!(store.last("births"), Some(Point::new(1, 3.0)));
        assert_eq!(store.last("missing"), None);
        assert_eq!(store.total_samples(), 5);
        assert_eq!(store.last_tick(), Some(1));
    }

    #[test]
    fn rejects_earlier_ticks_without_partial_updates() {
        let mut store = TimeSeriesStore::new();
        store.record(&metrics(5, &[("a", 1.0)])).unwrap();
        let err = store.record(&metrics(4, &[("a", 2.0), ("b", 3.0)])).unwrap_err();
        assert!(matches!(err, AnalyticsError::TickOutOfOrder { tick: 4, last: 5, .. }));
        assert_eq!(store.len(), 1);
        assert_eq!(store.get("a").unwrap().len(), 1);
    }

    #[test]
    fn same_tick_replaces_values() {
        let mut store = TimeSeriesStore::new();
        store.record(&metrics(3, &[("a", 1.0)])).unwrap();
        store.record(&metrics(3, &[("a", 7.0)])).unwrap();
        assert_eq!(store.get("a").unwrap().points(), &[Point::new(3, 7.0)]);
    }

    #[test]
    fn non_finite_values_are_counted_not_stored() {
        let mut store = TimeSeriesStore::new();
        store.record(&metrics(0, &[("a", f64::NAN), ("b", 1.0)])).unwrap();
        store.record(&metrics(1, &[("a", 2.0), ("b", f64::INFINITY)])).unwrap();
        assert_eq!(store.skipped_non_finite(), 2);
        assert_eq!(store.get("a").unwrap().points(), &[Point::new(1, 2.0)]);
        assert_eq!(store.get("b").unwrap().points(), &[Point::new(0, 1.0)]);
        assert!(store.record_value("a", 2, f64::NAN).is_err());
    }

    #[test]
    fn display_is_full_resolution_then_decimated_with_extremes() {
        let config = StoreConfig { display_points: 64, max_raw_points: None };
        let mut store = TimeSeriesStore::with_config(config).unwrap();
        for t in 0..64 {
            store.record(&metrics(t, &[("v", (t as f64).cos())])).unwrap();
        }
        assert_eq!(store.display("v").unwrap(), store.get("v").unwrap().points());
        for t in 64..50_000 {
            let v = if t == 31_337 {
                500.0
            } else if t == 40_000 {
                -500.0
            } else {
                (t as f64 * 0.01).sin()
            };
            store.record(&metrics(t, &[("v", v)])).unwrap();
        }
        let shown = store.display("v").unwrap();
        assert!(shown.len() <= 64);
        assert!(shown.contains(&Point::new(31_337, 500.0)));
        assert!(shown.contains(&Point::new(40_000, -500.0)));
        assert_eq!(shown.last(), Some(&Point::new(49_999, (49_999.0f64 * 0.01).sin())));
        assert_eq!(store.get("v").unwrap().len(), 50_000, "raw data stays at full resolution");
    }

    #[test]
    fn non_finite_replacement_removes_the_sample_of_that_tick() {
        let config = StoreConfig { display_points: 8, max_raw_points: None };
        let mut store = TimeSeriesStore::with_config(config).unwrap();
        for t in 0..100 {
            store.record(&metrics(t, &[("a", t as f64), ("b", 1.0)])).unwrap();
        }
        // The paused simulation re-publishes tick 99: `a` became NaN, `b` is absent.
        store.record(&metrics(99, &[("a", f64::NAN)])).unwrap();
        assert_eq!(store.last("a"), Some(Point::new(98, 98.0)));
        assert_eq!(store.last("b"), Some(Point::new(99, 1.0)));
        assert_eq!(store.skipped_non_finite(), 1);
        let rebuilt = M4Buckets::from_points(8, store.get("a").unwrap().points()).points();
        assert_eq!(store.display("a").unwrap(), rebuilt);
        // A NaN for a tick that has no sample of that key changes nothing.
        store.record(&metrics(100, &[("a", f64::NAN), ("b", 2.0)])).unwrap();
        assert_eq!(store.last("a"), Some(Point::new(98, 98.0)));
        assert_eq!(store.last_tick(), Some(100));
    }

    #[test]
    fn display_tracks_replaced_last_value() {
        let config = StoreConfig { display_points: 8, max_raw_points: None };
        let mut store = TimeSeriesStore::with_config(config).unwrap();
        for t in 0..100 {
            store.record_value("v", t, 0.0).unwrap();
        }
        store.record_value("v", 99, 1_000.0).unwrap();
        assert!(store.display("v").unwrap().contains(&Point::new(99, 1_000.0)));
    }

    #[test]
    fn raw_cap_compacts_but_keeps_extremes() {
        let config = StoreConfig { display_points: 16, max_raw_points: Some(1_000) };
        let mut store = TimeSeriesStore::with_config(config).unwrap();
        for t in 0..10_000u64 {
            let v = if t == 1_234 {
                99.0
            } else if t == 8_765 {
                -99.0
            } else {
                (t % 7) as f64
            };
            store.record_value("v", t, v).unwrap();
        }
        let s = store.get("v").unwrap();
        assert!(s.len() <= 1_000);
        assert!(s.is_compacted());
        assert!(s.points().contains(&Point::new(1_234, 99.0)));
        assert!(s.points().contains(&Point::new(8_765, -99.0)));
        assert!(s.points().windows(2).all(|w| w[0].tick < w[1].tick));
        assert!(store.display("v").unwrap().len() <= 16);
    }

    #[test]
    fn raw_cap_keeps_latest_sample_and_even_time_resolution() {
        let config = StoreConfig { display_points: 16, max_raw_points: Some(100) };
        let mut store = TimeSeriesStore::with_config(config).unwrap();
        for t in 0..10_000u64 {
            let v = (t as f64 * 0.7).sin();
            store.record_value("v", t, v).unwrap();
            // The current value is never lost by compaction.
            assert_eq!(store.last("v"), Some(Point::new(t, v)), "tick {t}");
        }
        let s = store.get("v").unwrap();
        assert_eq!(s.first(), Some(Point::new(0, 0.0)));
        assert!(s.is_compacted() && s.len() <= 100);
        assert_eq!(store.display("v").unwrap(), M4Buckets::from_points(16, s.points()).points());
    }

    #[test]
    fn compaction_equals_m4_of_the_original_samples() {
        // Repeated compactions must not erode older data: right after every
        // compaction the stored samples are exactly the M4 summary (25
        // power-of-two buckets) of every sample recorded so far.
        let config = StoreConfig { display_points: 16, max_raw_points: Some(100) };
        let mut store = TimeSeriesStore::with_config(config).unwrap();
        let mut raw = Vec::new();
        let mut compactions = 0;
        for t in 0..20_000u64 {
            let v = (t as f64 * 0.37).sin() * (1.0 + (t % 13) as f64);
            raw.push(Point::new(t, v));
            let before = store.get("v").map_or(0, Series::len);
            store.record_value("v", t, v).unwrap();
            let s = store.get("v").unwrap();
            if s.len() < before {
                compactions += 1;
                assert_eq!(s.points(), M4Buckets::from_points(50, &raw).points(), "compaction at tick {t}");
            }
        }
        assert!(compactions > 100);
        // The start of the run keeps as many samples per tick as the middle.
        let s = store.get("v").unwrap();
        let early = s.window(0, 4_999).len();
        let middle = s.window(5_000, 9_999).len();
        assert!(early >= 8 && early.abs_diff(middle) <= 4, "early {early}, middle {middle}");
    }

    #[test]
    fn display_uses_equal_tick_spans_for_irregular_sampling() {
        let config = StoreConfig { display_points: 16, max_raw_points: None };
        let mut store = TimeSeriesStore::with_config(config).unwrap();
        // Dense samples on ticks 0..100, then one sample every 10 000 ticks.
        for t in 0..100 {
            store.record_value("v", t, 1.0).unwrap();
        }
        for k in 1..=9 {
            store.record_value("v", k * 10_000, k as f64).unwrap();
        }
        let shown = store.display("v").unwrap();
        assert!(shown.len() <= 16);
        // The sparse half of the time axis is not crowded out by the dense start.
        assert!(shown.iter().filter(|p| p.tick >= 10_000).count() >= 3, "{shown:?}");
        assert_eq!(shown.last(), Some(&Point::new(90_000, 9.0)));
    }

    #[test]
    fn invalid_configs_are_rejected() {
        assert!(TimeSeriesStore::with_config(StoreConfig { display_points: 2, max_raw_points: None }).is_err());
        assert!(TimeSeriesStore::with_config(StoreConfig { display_points: 100, max_raw_points: Some(150) }).is_err());
    }

    #[test]
    fn truncate_rewinds_all_series() {
        let mut store = TimeSeriesStore::new();
        for t in 0..10 {
            store.record(&metrics(t, &[("a", t as f64), ("b", 1.0)])).unwrap();
        }
        store.truncate_after(4);
        assert_eq!(store.last_tick(), Some(4));
        assert_eq!(store.get("a").unwrap().len(), 5);
        store.record(&metrics(5, &[("a", 50.0)])).unwrap();
        assert_eq!(store.last("a"), Some(Point::new(5, 50.0)));
    }

    #[test]
    fn store_window_restricts_every_series() {
        let mut store = TimeSeriesStore::new();
        for t in 0..10 {
            store.record(&metrics(t, &[("a", t as f64)])).unwrap();
        }
        store.record(&metrics(10, &[("late", 1.0)])).unwrap();
        let w = store.window(3, 6);
        assert_eq!(w.keys().collect::<Vec<_>>(), vec!["a", "late"]);
        assert_eq!(w.get("a").unwrap().len(), 4);
        assert!(w.get("late").unwrap().is_empty());
        assert_eq!(w.last_tick(), Some(6));
    }
}
