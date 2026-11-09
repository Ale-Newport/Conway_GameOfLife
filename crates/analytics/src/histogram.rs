//! Histograms with fixed or automatically chosen bins.

use serde::{Deserialize, Serialize};

use crate::error::{AnalyticsError, check_values};
use crate::stats::{RunningStats, quantiles};

/// Upper bound on the number of bins chosen by [`Histogram::auto`], so that a
/// single extreme outlier cannot allocate millions of empty bins.
pub const MAX_AUTO_BINS: usize = 1_000;

/// Upper bound on the number of bins of any histogram (8 MB of counts).
pub const MAX_BINS: usize = 1_000_000;

/// Rule for choosing the number of bins automatically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinRule {
    /// `k = ⌈log₂ n⌉ + 1` (Sturges, 1926). Good for small, roughly normal samples.
    Sturges,
    /// `k = ⌈√n⌉`.
    SquareRoot,
    /// Bin width `h = 3.49 σ n^(−1/3)` (Scott, 1979).
    Scott,
    /// Bin width `h = 2 IQR n^(−1/3)` (Freedman & Diaconis, 1981). Robust to
    /// outliers; the default.
    #[default]
    FreedmanDiaconis,
}

/// Equal-width histogram over `[lo, hi]`. Values below `lo` / above `hi` are
/// counted separately as underflow / overflow; a value equal to `hi` falls in
/// the last bin (NumPy convention).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "HistogramData")]
pub struct Histogram {
    lo: f64,
    hi: f64,
    counts: Vec<u64>,
    underflow: u64,
    overflow: u64,
}

/// Serialised form of a [`Histogram`], validated on import.
#[derive(Deserialize)]
struct HistogramData {
    lo: f64,
    hi: f64,
    counts: Vec<u64>,
    underflow: u64,
    overflow: u64,
}

impl TryFrom<HistogramData> for Histogram {
    type Error = AnalyticsError;

    fn try_from(data: HistogramData) -> Result<Self, Self::Error> {
        check_layout(data.lo, data.hi, data.counts.len())?;
        Ok(Self { lo: data.lo, hi: data.hi, counts: data.counts, underflow: data.underflow, overflow: data.overflow })
    }
}

/// Checks a bin count and range (finite, non-empty, finite width).
fn check_layout(lo: f64, hi: f64, bins: usize) -> Result<(), AnalyticsError> {
    if bins == 0 || bins > MAX_BINS {
        return Err(AnalyticsError::invalid("histogram", format!("the number of bins must be in 1..={MAX_BINS}")));
    }
    if !(lo.is_finite() && hi.is_finite() && lo < hi && (hi - lo).is_finite()) {
        return Err(AnalyticsError::invalid(
            "histogram",
            format!("range [{lo}, {hi}] is empty, not finite or too wide"),
        ));
    }
    Ok(())
}

impl Histogram {
    /// Empty histogram with `bins` equal bins over `[lo, hi]` (at most
    /// [`MAX_BINS`] bins; `hi − lo` must be finite and positive).
    pub fn with_range(lo: f64, hi: f64, bins: usize) -> Result<Self, AnalyticsError> {
        check_layout(lo, hi, bins)?;
        Ok(Self { lo, hi, counts: vec![0; bins], underflow: 0, overflow: 0 })
    }

    /// Histogram of `values` with `bins` equal bins spanning the data range.
    /// When all values are equal to `x` the range is widened to
    /// `[x − d, x + d]` with `d = max(0.5, |x|·10⁻⁹)` (so that the bounds
    /// stay distinct for large `|x|`).
    pub fn fixed(values: &[f64], bins: usize) -> Result<Self, AnalyticsError> {
        check_values("histogram", values, 1)?;
        let stats = RunningStats::from_slice(values)?;
        let (lo, hi) = data_range(&stats);
        let mut h = Self::with_range(lo, hi, bins)?;
        for &x in values {
            h.insert(x);
        }
        Ok(h)
    }

    /// Histogram of `values` with the number of bins chosen by `rule` (at most
    /// [`MAX_AUTO_BINS`]). Width-based rules fall back to Sturges when the
    /// spread estimate is zero.
    pub fn auto(values: &[f64], rule: BinRule) -> Result<Self, AnalyticsError> {
        check_values("histogram", values, 1)?;
        let stats = RunningStats::from_slice(values)?;
        let n = values.len() as f64;
        let sturges = n.log2().ceil() as usize + 1;
        let (lo, hi) = data_range(&stats);
        let from_width = |h: f64| if h > 0.0 { ((hi - lo) / h).ceil() as usize } else { sturges };
        let bins = match rule {
            BinRule::Sturges => sturges,
            BinRule::SquareRoot => n.sqrt().ceil() as usize,
            BinRule::Scott => from_width(3.49 * stats.std_dev().unwrap_or(0.0) * n.powf(-1.0 / 3.0)),
            BinRule::FreedmanDiaconis => {
                let q = quantiles(values, &[0.25, 0.75])?;
                from_width(2.0 * (q[1] - q[0]) * n.powf(-1.0 / 3.0))
            }
        };
        Self::fixed(values, bins.clamp(1, MAX_AUTO_BINS))
    }

    /// Bin index of `x`, or `None` when it is outside `[lo, hi]` or not finite.
    pub fn bin_of(&self, x: f64) -> Option<usize> {
        if !(x >= self.lo && x <= self.hi) {
            return None;
        }
        let bins = self.counts.len();
        let i = ((x - self.lo) / (self.hi - self.lo) * bins as f64) as usize;
        Some(i.min(bins - 1))
    }

    fn insert(&mut self, x: f64) {
        match self.bin_of(x) {
            Some(i) => self.counts[i] += 1,
            None if x < self.lo => self.underflow += 1,
            None => self.overflow += 1,
        }
    }

    /// Adds a value; non-finite values are rejected.
    pub fn add(&mut self, x: f64) -> Result<(), AnalyticsError> {
        if !x.is_finite() {
            return Err(AnalyticsError::NonFinite { what: "histogram", index: self.total() as usize, value: x });
        }
        self.insert(x);
        Ok(())
    }

    /// Counts per bin.
    pub fn counts(&self) -> &[u64] {
        &self.counts
    }

    /// Number of bins.
    pub fn bins(&self) -> usize {
        self.counts.len()
    }

    /// `(lo, hi)` range covered by the bins.
    pub fn range(&self) -> (f64, f64) {
        (self.lo, self.hi)
    }

    /// Width of every bin.
    pub fn bin_width(&self) -> f64 {
        (self.hi - self.lo) / self.counts.len() as f64
    }

    /// The `bins + 1` bin edges.
    pub fn edges(&self) -> Vec<f64> {
        let w = self.bin_width();
        (0..=self.counts.len()).map(|i| if i == self.counts.len() { self.hi } else { self.lo + w * i as f64 }).collect()
    }

    /// Bin centres.
    pub fn centers(&self) -> Vec<f64> {
        let w = self.bin_width();
        (0..self.counts.len()).map(|i| self.lo + w * (i as f64 + 0.5)).collect()
    }

    /// Values below the range.
    pub fn underflow(&self) -> u64 {
        self.underflow
    }

    /// Values above the range.
    pub fn overflow(&self) -> u64 {
        self.overflow
    }

    /// Values inside the range.
    pub fn in_range(&self) -> u64 {
        self.counts.iter().sum()
    }

    /// All values added, including under- and overflow.
    pub fn total(&self) -> u64 {
        self.in_range() + self.underflow + self.overflow
    }

    /// Probability density per bin (`count / (in_range · width)`), integrating
    /// to 1 over the range. All zeros when the histogram is empty.
    pub fn density(&self) -> Vec<f64> {
        let n = self.in_range();
        if n == 0 {
            return vec![0.0; self.counts.len()];
        }
        let norm = n as f64 * self.bin_width();
        self.counts.iter().map(|&c| c as f64 / norm).collect()
    }
}

fn data_range(stats: &RunningStats) -> (f64, f64) {
    let (lo, hi) = (stats.min().unwrap_or(0.0), stats.max().unwrap_or(0.0));
    if lo < hi {
        (lo, hi)
    } else {
        let pad = 0.5f64.max(lo.abs() * 1e-9);
        (lo - pad, hi + pad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_bins_count_correctly() {
        let h = Histogram::fixed(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0], 5).unwrap();
        // Bins [0,2) [2,4) [4,6) [6,8) [8,10]; 10 lands in the last bin.
        assert_eq!(h.counts(), &[2, 2, 2, 2, 3]);
        assert_eq!(h.edges(), vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(h.centers()[0], 1.0);
        assert_eq!(h.total(), 11);
        let density_integral: f64 = h.density().iter().map(|d| d * h.bin_width()).sum();
        assert!((density_integral - 1.0).abs() < 1e-12);
    }

    #[test]
    fn explicit_range_tracks_under_and_overflow() {
        let mut h = Histogram::with_range(0.0, 1.0, 4).unwrap();
        for x in [-0.1, 0.0, 0.3, 0.99, 1.0, 1.5] {
            h.add(x).unwrap();
        }
        assert_eq!(h.counts(), &[1, 1, 0, 2]);
        assert_eq!((h.underflow(), h.overflow()), (1, 1));
        assert!(h.add(f64::NAN).is_err());
        assert!(Histogram::with_range(1.0, 1.0, 4).is_err());
        assert!(Histogram::with_range(0.0, 1.0, 0).is_err());
    }

    #[test]
    fn constant_data_gets_one_unit_range() {
        let h = Histogram::fixed(&[3.0, 3.0, 3.0], 3).unwrap();
        assert_eq!(h.range(), (2.5, 3.5));
        assert_eq!(h.counts(), &[0, 3, 0]);
        let auto = Histogram::auto(&[3.0; 10], BinRule::FreedmanDiaconis).unwrap();
        assert_eq!(auto.in_range(), 10);
    }

    #[test]
    fn auto_rules_choose_expected_bin_counts() {
        let values: Vec<f64> = (0..100).map(f64::from).collect();
        // ⌈log₂ 100⌉ + 1 = 8.
        assert_eq!(Histogram::auto(&values, BinRule::Sturges).unwrap().bins(), 8);
        assert_eq!(Histogram::auto(&values, BinRule::SquareRoot).unwrap().bins(), 10);
        // IQR = 49.5, h = 99 / 100^(1/3) = 21.33, range 99 → 5 bins.
        assert_eq!(Histogram::auto(&values, BinRule::FreedmanDiaconis).unwrap().bins(), 5);
        // σ = 28.866, h = 3.49 σ / 100^(1/3) = 21.70 → 5 bins.
        assert_eq!(Histogram::auto(&values, BinRule::Scott).unwrap().bins(), 5);
        for rule in [BinRule::Sturges, BinRule::SquareRoot, BinRule::Scott, BinRule::FreedmanDiaconis] {
            assert_eq!(Histogram::auto(&values, rule).unwrap().in_range(), 100);
        }
    }

    #[test]
    fn constant_large_values_get_a_relative_range() {
        let h = Histogram::fixed(&[1e17; 4], 2).unwrap();
        let (lo, hi) = h.range();
        assert!(lo < 1e17 && hi > 1e17);
        assert_eq!(h.in_range(), 4);
    }

    #[test]
    fn invalid_layouts_are_rejected() {
        assert!(Histogram::with_range(0.0, 1.0, MAX_BINS + 1).is_err());
        assert!(Histogram::with_range(-f64::MAX, f64::MAX, 4).is_err());
        assert!(Histogram::with_range(0.0, f64::INFINITY, 4).is_err());
        // Imported histograms are validated instead of panicking later.
        let empty = r#"{"lo":0.0,"hi":1.0,"counts":[],"underflow":0,"overflow":0}"#;
        assert!(serde_json::from_str::<Histogram>(empty).is_err());
        let reversed = r#"{"lo":2.0,"hi":1.0,"counts":[1],"underflow":0,"overflow":0}"#;
        assert!(serde_json::from_str::<Histogram>(reversed).is_err());
        let h = Histogram::fixed(&[1.0, 2.0, 2.5], 3).unwrap();
        let back: Histogram = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(back, h);
    }

    #[test]
    fn bins_match_brute_force_assignment() {
        // Each value lands in the bin whose half-open interval contains it.
        let mut rng = simulation_core::SimRng::seed_from_u64(11);
        let values: Vec<f64> = (0..2_000).map(|_| rng.range_f64(-3.0, 7.0)).collect();
        let h = Histogram::with_range(-2.0, 6.0, 16).unwrap();
        let mut filled = h.clone();
        let edges = h.edges();
        let mut expected = [0u64; 16];
        let (mut under, mut over) = (0, 0);
        for &x in &values {
            filled.add(x).unwrap();
            if x < -2.0 {
                under += 1;
            } else if x > 6.0 {
                over += 1;
            } else {
                let bin = (0..16).find(|&i| x < edges[i + 1]).unwrap_or(15);
                expected[bin] += 1;
            }
        }
        assert_eq!(filled.counts(), &expected[..]);
        assert_eq!((filled.underflow(), filled.overflow()), (under, over));
        assert_eq!(filled.total(), 2_000);
    }

    #[test]
    fn auto_bins_are_capped() {
        let mut values = vec![0.0; 1_000];
        values.extend((0..1_000).map(|i| i as f64 * 1e-6));
        values.push(1e9);
        assert!(Histogram::auto(&values, BinRule::FreedmanDiaconis).unwrap().bins() <= MAX_AUTO_BINS);
    }
}
