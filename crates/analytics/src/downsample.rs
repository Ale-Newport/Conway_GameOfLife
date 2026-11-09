//! Decimation of time series for plotting.
//!
//! Three classic algorithms are provided:
//!
//! * [`min_max`] — splits the tick range into equal-width buckets and keeps
//!   the minimum and the maximum of every bucket. The global extremes always
//!   survive, so spikes never disappear from a plot.
//! * [`m4`] — keeps the first, minimum, maximum and last sample of every
//!   equal-width bucket (Jugel et al., 2014). With one bucket per pixel
//!   column, a line chart drawn from the M4 points is pixel-identical to the
//!   full series. The first and last samples of the series are always kept.
//! * [`lttb`] — Largest-Triangle-Three-Buckets (Steinarsson, 2013): one point
//!   per equal-count bucket chosen to maximise the triangle area with its
//!   neighbours. Visually faithful for smooth data, but extremes are not
//!   guaranteed.
//!
//! Buckets of [`min_max`] and [`m4`] cover equal *tick* spans, so irregularly
//! sampled series (metrics recorded every few ticks, gaps, rewinds, data
//! thinned by [`crate::StoreConfig::max_raw_points`]) keep a correct time
//! axis. [`crate::Series`] maintains an incremental version of M4 over
//! power-of-two tick spans that is updated in O(1) amortised time per sample,
//! so a plot of a million-sample series is produced in time proportional to
//! the display size rather than the series length.

use serde::{Deserialize, Serialize};

use crate::series::Point;

/// Decimation algorithm used by [`downsample`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownsampleMethod {
    /// Minimum and maximum of each bucket ([`min_max`]).
    MinMax,
    /// First, minimum, maximum and last of each bucket ([`m4`]).
    #[default]
    M4,
    /// Largest-Triangle-Three-Buckets ([`lttb`]).
    Lttb,
}

/// Decimates `points` to at most `max_points` points with `method`. Series
/// with at most `max_points` points are returned unchanged.
pub fn downsample(points: &[Point], max_points: usize, method: DownsampleMethod) -> Vec<Point> {
    match method {
        DownsampleMethod::MinMax => min_max(points, max_points),
        DownsampleMethod::M4 => m4(points, max_points),
        DownsampleMethod::Lttb => lttb(points, max_points),
    }
}

/// Aggregate of the samples falling into one bucket.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bucket {
    /// Sample with the smallest tick.
    first: Point,
    /// Sample with the largest tick.
    last: Point,
    /// First sample with the smallest value.
    min: Point,
    /// First sample with the largest value.
    max: Point,
}

impl Bucket {
    fn new(p: Point) -> Self {
        Self { first: p, last: p, min: p, max: p }
    }

    /// Adds a sample. For samples in tick order (as in a series) ties keep
    /// the earliest minimum / maximum.
    fn add(&mut self, p: Point) {
        if p.tick < self.first.tick {
            self.first = p;
        }
        if p.tick >= self.last.tick {
            self.last = p;
        }
        if p.value < self.min.value {
            self.min = p;
        }
        if p.value > self.max.value {
            self.max = p;
        }
    }

    /// Aggregate of two buckets, `b` covering later ticks than `a`.
    fn merge(a: &Bucket, b: &Bucket) -> Bucket {
        Bucket {
            first: a.first,
            last: b.last,
            min: if b.min.value < a.min.value { b.min } else { a.min },
            max: if b.max.value > a.max.value { b.max } else { a.max },
        }
    }

    fn of(points: &[Point]) -> Option<Bucket> {
        let (first, rest) = points.split_first()?;
        let mut bucket = Bucket::new(*first);
        for p in rest {
            bucket.add(*p);
        }
        Some(bucket)
    }

    /// Appends the bucket's distinct representative samples in tick order:
    /// minimum and maximum, plus first and last when `with_ends`.
    fn emit(&self, with_ends: bool, out: &mut Vec<Point>) {
        let mut all = [self.first, self.min, self.max, self.last];
        let candidates: &mut [Point] = if with_ends { &mut all } else { &mut all[1..3] };
        candidates.sort_by_key(|p| p.tick);
        for &p in candidates.iter() {
            // Skip the same sample chosen twice (e.g. the first sample is also
            // the minimum); distinct samples sharing a tick are all kept.
            if out.last().is_none_or(|last| *last != p) {
                out.push(p);
            }
        }
    }
}

/// Groups `points` into at most `buckets` buckets of equal tick span covering
/// `[min tick, max tick]`; returns the non-empty buckets in tick order. Works
/// for any input order and never produces more than `buckets` buckets.
fn time_buckets(points: &[Point], buckets: usize) -> Vec<Bucket> {
    let Some(lo) = points.iter().map(|p| p.tick).min() else {
        return Vec::new();
    };
    let hi = points.iter().map(|p| p.tick).max().unwrap_or(lo);
    let span = u128::from(hi - lo) + 1;
    let count = buckets.max(1);
    let mut slots: Vec<Option<Bucket>> = vec![None; count];
    for p in points {
        // (tick − lo) < span, so the index is always below `count`.
        let index = (u128::from(p.tick - lo) * count as u128 / span) as usize;
        match &mut slots[index] {
            Some(bucket) => bucket.add(*p),
            slot => *slot = Some(Bucket::new(*p)),
        }
    }
    slots.into_iter().flatten().collect()
}

/// Min/max decimation to at most `max_points` points (values below 2 are
/// treated as 2): the tick range is split into `max_points / 2` equal-width
/// buckets and the minimum and maximum of each are emitted in tick order, so
/// the global minimum and maximum are always kept.
pub fn min_max(points: &[Point], max_points: usize) -> Vec<Point> {
    let max_points = max_points.max(2);
    if points.len() <= max_points {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(max_points);
    for bucket in time_buckets(points, max_points / 2) {
        bucket.emit(false, &mut out);
    }
    out
}

/// M4 decimation to at most `max_points` points (values below 4 are treated
/// as 4): the tick range is split into `max_points / 4` equal-width buckets
/// and the first, minimum, maximum and last sample of each are emitted in
/// tick order. The first and last samples of the series and its global
/// extremes are always kept.
pub fn m4(points: &[Point], max_points: usize) -> Vec<Point> {
    let max_points = max_points.max(4);
    if points.len() <= max_points {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(max_points);
    for bucket in time_buckets(points, max_points / 4) {
        bucket.emit(true, &mut out);
    }
    out
}

/// Largest-Triangle-Three-Buckets decimation to exactly `threshold` points
/// (values below 3 are treated as 3) of a series in tick order. The first and
/// last samples are always kept. The tick is used as the x coordinate.
pub fn lttb(points: &[Point], threshold: usize) -> Vec<Point> {
    let threshold = threshold.max(3);
    let n = points.len();
    if n <= threshold {
        return points.to_vec();
    }
    let every = (n - 2) as f64 / (threshold - 2) as f64;
    let mut out = Vec::with_capacity(threshold);
    out.push(points[0]);
    let mut a = 0usize;
    for i in 0..threshold - 2 {
        // Average of the next bucket (the third vertex of the triangle).
        let avg_start = ((i + 1) as f64 * every) as usize + 1;
        let avg_end = (((i + 2) as f64 * every) as usize + 1).min(n);
        let avg_slice = &points[avg_start..avg_end];
        let count = avg_slice.len() as f64;
        let avg_x = avg_slice.iter().map(|p| p.tick as f64).sum::<f64>() / count;
        let avg_y = avg_slice.iter().map(|p| p.value).sum::<f64>() / count;

        // Candidate points of the current bucket.
        let range_start = (i as f64 * every) as usize + 1;
        let range_end = ((i + 1) as f64 * every) as usize + 1;
        let (ax, ay) = (points[a].tick as f64, points[a].value);
        let mut best = range_start;
        let mut best_area = -1.0;
        for (j, p) in points[range_start..range_end].iter().enumerate() {
            let area = ((ax - avg_x) * (p.value - ay) - (ax - p.tick as f64) * (avg_y - ay)).abs();
            if area > best_area {
                best_area = area;
                best = range_start + j;
            }
        }
        out.push(points[best]);
        a = best;
    }
    out.push(points[n - 1]);
    out
}

/// Incrementally maintained M4 summary of a growing series.
///
/// Bucket `k` covers the ticks `[origin + k·width, origin + (k + 1)·width)`,
/// where `origin` is the tick of the first sample and `width` is a power of
/// two. When a sample falls beyond the last of the `max_buckets` buckets,
/// `width` doubles and neighbouring buckets are merged pairwise. Each bucket
/// emits at most four points, so `points()` never returns more than the
/// configured number of display points.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct M4Buckets {
    max_buckets: u128,
    origin: u64,
    width: u128,
    /// Non-empty buckets as `(index, aggregate)`, in increasing index order.
    buckets: Vec<(u128, Bucket)>,
}

impl M4Buckets {
    /// Empty summary emitting at most `display_points` points (at least 4).
    pub(crate) fn new(display_points: usize) -> Self {
        Self { max_buckets: (display_points / 4).max(1) as u128, origin: 0, width: 1, buckets: Vec::new() }
    }

    /// Summary of an existing series.
    pub(crate) fn from_points(display_points: usize, points: &[Point]) -> Self {
        let mut buckets = Self::new(display_points);
        for p in points {
            buckets.push(*p);
        }
        buckets
    }

    /// Adds the next sample (ticks must not decrease; the series checks).
    pub(crate) fn push(&mut self, p: Point) {
        if self.buckets.is_empty() {
            self.origin = p.tick;
            self.width = 1;
        }
        let offset = u128::from(p.tick.saturating_sub(self.origin));
        while offset / self.width >= self.max_buckets {
            self.coarsen();
        }
        let index = offset / self.width;
        match self.buckets.last_mut() {
            Some((last, bucket)) if *last == index => bucket.add(p),
            _ => self.buckets.push((index, Bucket::new(p))),
        }
    }

    /// Doubles the bucket width, merging buckets that now share an index.
    fn coarsen(&mut self) {
        self.width *= 2;
        let mut merged: Vec<(u128, Bucket)> = Vec::with_capacity(self.buckets.len() / 2 + 1);
        for (index, bucket) in self.buckets.drain(..) {
            let index = index / 2;
            match merged.last_mut() {
                Some((last, acc)) if *last == index => *acc = Bucket::merge(acc, &bucket),
                _ => merged.push((index, bucket)),
            }
        }
        self.buckets = merged;
    }

    /// First tick covered by the last bucket (`None` when empty). The samples
    /// of the series from this tick on are exactly the last bucket's samples.
    pub(crate) fn last_bucket_start(&self) -> Option<u64> {
        // index·width ≤ offset of the last sample, which fits in u64.
        self.buckets.last().map(|(index, _)| self.origin + (index * self.width) as u64)
    }

    /// Replaces the last bucket by a summary of `tail`, which must be the
    /// samples of the series from [`M4Buckets::last_bucket_start`] on (after
    /// the last sample was replaced or removed). An empty tail removes the
    /// bucket.
    pub(crate) fn rebuild_last(&mut self, tail: &[Point]) {
        match Bucket::of(tail) {
            Some(bucket) => {
                if let Some((_, last)) = self.buckets.last_mut() {
                    *last = bucket;
                }
            }
            None => {
                self.buckets.pop();
            }
        }
    }

    /// `(origin, width)` of the bucket grid.
    #[cfg(test)]
    pub(crate) fn grid(&self) -> (u64, u128) {
        (self.origin, self.width)
    }

    /// Decimated points in tick order.
    pub(crate) fn points(&self) -> Vec<Point> {
        let mut out = Vec::with_capacity(self.buckets.len() * 4);
        for (_, bucket) in &self.buckets {
            bucket.emit(true, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(n: u64) -> Vec<Point> {
        (0..n).map(|t| Point { tick: t, value: ((t as f64) * 0.37).sin() * (1.0 + (t % 17) as f64) }).collect()
    }

    fn with_spike(n: u64, at: u64, value: f64) -> Vec<Point> {
        let mut pts = wave(n);
        pts[at as usize].value = value;
        pts
    }

    /// First minimum and first maximum (brute force).
    fn global_extremes(points: &[Point]) -> (Point, Point) {
        let mut lo = points[0];
        let mut hi = points[0];
        for p in points {
            if p.value < lo.value {
                lo = *p;
            }
            if p.value > hi.value {
                hi = *p;
            }
        }
        (lo, hi)
    }

    /// Brute-force M4 over a fixed grid: groups samples by
    /// `(tick − origin) / width` and emits first / min / max / last.
    fn brute_force_m4(points: &[Point], origin: u64, width: u128) -> Vec<Point> {
        let mut out = Vec::new();
        let mut start = 0;
        while start < points.len() {
            let key = u128::from(points[start].tick - origin) / width;
            let mut end = start;
            while end < points.len() && u128::from(points[end].tick - origin) / width == key {
                end += 1;
            }
            let group = &points[start..end];
            let (lo, hi) = global_extremes(group);
            let mut chosen = vec![group[0], lo, hi, group[group.len() - 1]];
            chosen.sort_by_key(|p| p.tick);
            chosen.dedup();
            out.extend(chosen);
            start = end;
        }
        out
    }

    #[test]
    fn short_series_are_unchanged() {
        let pts = wave(10);
        assert_eq!(min_max(&pts, 20), pts);
        assert_eq!(m4(&pts, 20), pts);
        assert_eq!(lttb(&pts, 20), pts);
        assert!(m4(&[], 4).is_empty());
    }

    #[test]
    fn min_max_preserves_extremes_and_budget() {
        let mut pts = with_spike(10_007, 4_321, 1e6);
        pts[9_999].value = -1e6;
        let out = min_max(&pts, 100);
        assert!(out.len() <= 100);
        let (lo, hi) = global_extremes(&pts);
        assert!(out.contains(&lo) && out.contains(&hi));
        assert_eq!(hi.value, 1e6);
        assert_eq!(lo.value, -1e6);
        assert!(out.windows(2).all(|w| w[0].tick < w[1].tick), "ticks must stay ordered");
    }

    #[test]
    fn m4_keeps_first_last_and_extremes() {
        let pts = with_spike(5_000, 17, -42.0);
        let out = m4(&pts, 64);
        assert!(out.len() <= 64);
        assert_eq!(out.first(), pts.first());
        assert_eq!(out.last(), pts.last());
        let (lo, hi) = global_extremes(&pts);
        assert!(out.contains(&lo) && out.contains(&hi));
    }

    #[test]
    fn buckets_cover_equal_tick_spans() {
        // 1000 samples on ticks 0..1000 followed by 4 samples spread over
        // ticks 1000..1_000_000: equal-count buckets would spend almost the
        // whole budget on the first 0.1 % of the time axis.
        let mut pts: Vec<Point> = (0..1_000).map(|t| Point::new(t, (t % 10) as f64)).collect();
        pts.extend([250_000, 500_000, 750_000, 999_999].map(|t| Point::new(t, 5.0)));
        let out = m4(&pts, 40);
        assert!(out.len() <= 40);
        // 10 buckets of 100 000 ticks: the first holds the dense data, the
        // others hold one sparse sample each.
        assert_eq!(out.iter().filter(|p| p.tick >= 1_000).count(), 4);
        assert_eq!(out.last(), pts.last());
        // Brute force: M4 of the first bucket (ticks 0..100 000).
        let first_bucket: Vec<Point> = out.iter().copied().filter(|p| p.tick < 100_000).collect();
        assert_eq!(first_bucket, brute_force_m4(&pts[..1_000], 0, 100_000));
    }

    #[test]
    fn unordered_input_stays_within_budget() {
        let mut pts = wave(1_000);
        pts.reverse();
        let out = m4(&pts, 40);
        assert!(out.len() <= 40);
        assert!(out.windows(2).all(|w| w[0].tick <= w[1].tick));
        assert_eq!(out.first().map(|p| p.tick), Some(0));
        assert_eq!(out.last().map(|p| p.tick), Some(999));
    }

    #[test]
    fn lttb_returns_threshold_points_with_endpoints() {
        let pts = wave(1_000);
        let out = lttb(&pts, 50);
        assert_eq!(out.len(), 50);
        assert_eq!(out[0], pts[0]);
        assert_eq!(out[49], pts[999]);
        assert!(out.windows(2).all(|w| w[0].tick < w[1].tick));
    }

    #[test]
    fn lttb_picks_an_isolated_peak() {
        // A flat line with one spike: the spike forms the largest triangle.
        let mut pts: Vec<Point> = (0..100).map(|t| Point { tick: t, value: 0.0 }).collect();
        pts[50].value = 10.0;
        let out = lttb(&pts, 10);
        assert!(out.iter().any(|p| p.tick == 50 && p.value == 10.0));
    }

    #[test]
    fn incremental_m4_matches_brute_force() {
        let pts = with_spike(100_003, 77_777, 9e9);
        let buckets = M4Buckets::from_points(400, &pts);
        let out = buckets.points();
        assert!(out.len() <= 400, "{} points", out.len());
        let (origin, width) = buckets.grid();
        assert!(width.is_power_of_two());
        assert_eq!(out, brute_force_m4(&pts, origin, width));
        let (lo, hi) = global_extremes(&pts);
        assert!(out.contains(&lo) && out.contains(&hi));
        assert_eq!(out.first(), pts.first());
        assert_eq!(out.last(), pts.last());
        assert!(out.windows(2).all(|w| w[0].tick < w[1].tick));
    }

    #[test]
    fn incremental_m4_handles_irregular_ticks() {
        // Samples every 7 ticks, then a long gap, then every tick.
        let mut pts: Vec<Point> = (0..500).map(|i| Point::new(1_000 + i * 7, (i as f64).sin())).collect();
        pts.extend((0..500).map(|i| Point::new(1_000_000 + i, (i as f64).cos())));
        let buckets = M4Buckets::from_points(64, &pts);
        let (origin, width) = buckets.grid();
        assert_eq!(origin, 1_000);
        assert!(buckets.points().len() <= 64);
        assert_eq!(buckets.points(), brute_force_m4(&pts, origin, width));
    }

    #[test]
    fn incremental_m4_last_bucket_can_be_rebuilt() {
        let pts = wave(1_000);
        let mut buckets = M4Buckets::from_points(40, &pts);
        let start = buckets.last_bucket_start().unwrap();
        // Changing the last sample and rebuilding the tail matches a fresh build.
        let mut changed = pts.clone();
        changed[999].value = 1e3;
        let tail_start = changed.partition_point(|p| p.tick < start);
        buckets.rebuild_last(&changed[tail_start..]);
        assert_eq!(buckets, M4Buckets::from_points(40, &changed));
        // Removing samples down to an empty last bucket also matches.
        let start = buckets.last_bucket_start().unwrap();
        let shorter: Vec<Point> = changed.iter().copied().filter(|p| p.tick < start).collect();
        buckets.rebuild_last(&[]);
        assert_eq!(buckets.points(), M4Buckets::from_points(40, &shorter).points());
    }
}
