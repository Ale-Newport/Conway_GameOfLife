//! Descriptive statistics: streaming moments (Welford), summaries,
//! quantiles, correlation and least-squares regression.

use serde::{Deserialize, Serialize};

use crate::error::{AnalyticsError, check_finite, check_values};

/// Streaming mean / variance / extremes using Welford's algorithm, which is
/// numerically stable for long runs (no catastrophic cancellation of
/// `Σx² − (Σx)²/n`). Two accumulators can be combined with
/// [`RunningStats::merge`] (Chan, Golub & LeVeque, 1979).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunningStats {
    count: u64,
    mean: f64,
    m2: f64,
    min: f64,
    max: f64,
}

impl Default for RunningStats {
    fn default() -> Self {
        Self::new()
    }
}

impl RunningStats {
    /// Empty accumulator.
    pub fn new() -> Self {
        Self { count: 0, mean: 0.0, m2: 0.0, min: f64::INFINITY, max: f64::NEG_INFINITY }
    }

    /// Accumulates every value of a slice.
    pub fn from_slice(values: &[f64]) -> Result<Self, AnalyticsError> {
        check_finite("running statistics", values)?;
        let mut s = Self::new();
        for &x in values {
            s.add(x);
        }
        Ok(s)
    }

    /// Adds one value; non-finite values are rejected.
    pub fn push(&mut self, x: f64) -> Result<(), AnalyticsError> {
        if !x.is_finite() {
            return Err(AnalyticsError::NonFinite { what: "running statistics", index: self.count as usize, value: x });
        }
        self.add(x);
        Ok(())
    }

    /// Adds a value known to be finite.
    pub(crate) fn add(&mut self, x: f64) {
        self.count += 1;
        let delta = x - self.mean;
        self.mean += delta / self.count as f64;
        self.m2 += delta * (x - self.mean);
        self.min = self.min.min(x);
        self.max = self.max.max(x);
    }

    /// Combines two accumulators as if all values had been pushed into one.
    pub fn merge(&mut self, other: &RunningStats) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = *other;
            return;
        }
        let n_a = self.count as f64;
        let n_b = other.count as f64;
        let n = n_a + n_b;
        let delta = other.mean - self.mean;
        self.mean += delta * n_b / n;
        self.m2 += other.m2 + delta * delta * n_a * n_b / n;
        self.count += other.count;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    /// Number of values.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Arithmetic mean.
    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then_some(self.mean)
    }

    /// Population variance `Σ(x − x̄)² / n`.
    pub fn variance(&self) -> Option<f64> {
        (self.count > 0).then(|| self.m2 / self.count as f64)
    }

    /// Sample variance `Σ(x − x̄)² / (n − 1)` (needs two values).
    pub fn sample_variance(&self) -> Option<f64> {
        (self.count > 1).then(|| self.m2 / (self.count - 1) as f64)
    }

    /// Population standard deviation.
    pub fn std_dev(&self) -> Option<f64> {
        self.variance().map(f64::sqrt)
    }

    /// Sample standard deviation.
    pub fn sample_std_dev(&self) -> Option<f64> {
        self.sample_variance().map(f64::sqrt)
    }

    /// Smallest value.
    pub fn min(&self) -> Option<f64> {
        (self.count > 0).then_some(self.min)
    }

    /// Largest value.
    pub fn max(&self) -> Option<f64> {
        (self.count > 0).then_some(self.max)
    }
}

/// Five-number summary plus mean and standard deviation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Number of values.
    pub count: usize,
    /// Arithmetic mean.
    pub mean: f64,
    /// Population standard deviation.
    pub std_dev: f64,
    /// Minimum.
    pub min: f64,
    /// First quartile (type-7 quantile 0.25).
    pub q1: f64,
    /// Median.
    pub median: f64,
    /// Third quartile (type-7 quantile 0.75).
    pub q3: f64,
    /// Maximum.
    pub max: f64,
}

/// Summary statistics of a non-empty slice of finite values.
pub fn summarize(values: &[f64]) -> Result<Summary, AnalyticsError> {
    check_values("summary", values, 1)?;
    let stats = RunningStats::from_slice(values)?;
    let sorted = sorted_copy(values);
    Ok(Summary {
        count: values.len(),
        mean: stats.mean,
        std_dev: (stats.m2 / values.len() as f64).sqrt(),
        min: sorted[0],
        q1: quantile_sorted(&sorted, 0.25),
        median: quantile_sorted(&sorted, 0.5),
        q3: quantile_sorted(&sorted, 0.75),
        max: sorted[sorted.len() - 1],
    })
}

/// Arithmetic mean of a non-empty slice.
pub fn mean(values: &[f64]) -> Result<f64, AnalyticsError> {
    check_values("mean", values, 1)?;
    Ok(RunningStats::from_slice(values)?.mean)
}

/// Population variance (`n` denominator) of a non-empty slice.
pub fn variance(values: &[f64]) -> Result<f64, AnalyticsError> {
    check_values("variance", values, 1)?;
    let s = RunningStats::from_slice(values)?;
    Ok(s.m2 / s.count as f64)
}

/// Sample variance (`n − 1` denominator); needs at least two values.
pub fn sample_variance(values: &[f64]) -> Result<f64, AnalyticsError> {
    check_values("sample variance", values, 2)?;
    let s = RunningStats::from_slice(values)?;
    Ok(s.m2 / (s.count - 1) as f64)
}

/// Population standard deviation of a non-empty slice.
pub fn std_dev(values: &[f64]) -> Result<f64, AnalyticsError> {
    variance(values).map(f64::sqrt)
}

/// `(min, max)` of a non-empty slice.
pub fn min_max(values: &[f64]) -> Result<(f64, f64), AnalyticsError> {
    check_values("min/max", values, 1)?;
    Ok(values.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &x| (lo.min(x), hi.max(x))))
}

fn sorted_copy(values: &[f64]) -> Vec<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted
}

/// Type-7 quantile (linear interpolation between order statistics, the
/// default of R and NumPy) of an already sorted non-empty slice.
fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    let h = (sorted.len() - 1) as f64 * q;
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    sorted[lo] + (h - lo as f64) * (sorted[hi] - sorted[lo])
}

fn check_probability(q: f64) -> Result<(), AnalyticsError> {
    if (0.0..=1.0).contains(&q) {
        Ok(())
    } else {
        Err(AnalyticsError::invalid("quantile", format!("probability {q} is outside [0, 1]")))
    }
}

/// Quantile `q ∈ [0, 1]` with linear interpolation (Hyndman & Fan type 7):
/// with sorted values `x₀ ≤ … ≤ x₍ₙ₋₁₎` and `h = (n − 1)q`, the result is
/// `x⌊h⌋ + (h − ⌊h⌋)(x⌊h⌋₊₁ − x⌊h⌋)`.
pub fn quantile(values: &[f64], q: f64) -> Result<f64, AnalyticsError> {
    check_values("quantile", values, 1)?;
    check_probability(q)?;
    Ok(quantile_sorted(&sorted_copy(values), q))
}

/// Several quantiles at once (the values are sorted only once).
pub fn quantiles(values: &[f64], qs: &[f64]) -> Result<Vec<f64>, AnalyticsError> {
    check_values("quantile", values, 1)?;
    for &q in qs {
        check_probability(q)?;
    }
    let sorted = sorted_copy(values);
    Ok(qs.iter().map(|&q| quantile_sorted(&sorted, q)).collect())
}

/// Median (type-7 quantile 0.5).
pub fn median(values: &[f64]) -> Result<f64, AnalyticsError> {
    quantile(values, 0.5)
}

fn check_pairs(what: &'static str, xs: &[f64], ys: &[f64]) -> Result<(), AnalyticsError> {
    if xs.len() != ys.len() {
        return Err(AnalyticsError::LengthMismatch { what, left: xs.len(), right: ys.len() });
    }
    check_values(what, xs, 2)?;
    check_finite(what, ys)
}

/// Centred sums `(Sxx, Syy, Sxy, x̄, ȳ)` computed in two passes.
fn centred_sums(xs: &[f64], ys: &[f64]) -> (f64, f64, f64, f64, f64) {
    let n = xs.len() as f64;
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for (&x, &y) in xs.iter().zip(ys) {
        let (dx, dy) = (x - mx, y - my);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    (sxx, syy, sxy, mx, my)
}

/// Pearson product-moment correlation `r = Sxy / √(Sxx·Syy)`. Undefined when
/// either variable is constant.
pub fn pearson(xs: &[f64], ys: &[f64]) -> Result<f64, AnalyticsError> {
    check_pairs("Pearson correlation", xs, ys)?;
    let (sxx, syy, sxy, _, _) = centred_sums(xs, ys);
    if sxx == 0.0 || syy == 0.0 {
        return Err(AnalyticsError::undefined("Pearson correlation", "one of the variables is constant"));
    }
    Ok((sxy / (sxx * syy).sqrt()).clamp(-1.0, 1.0))
}

/// Ranks starting at 1, ties receiving the average of their ranks.
fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut ranks = vec![0.0; values.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && values[order[j + 1]] == values[order[i]] {
            j += 1;
        }
        let rank = (i + j) as f64 / 2.0 + 1.0;
        for &k in &order[i..=j] {
            ranks[k] = rank;
        }
        i = j + 1;
    }
    ranks
}

/// Spearman rank correlation: Pearson correlation of the ranks (average
/// ranks for ties). Measures monotonic rather than linear association.
pub fn spearman(xs: &[f64], ys: &[f64]) -> Result<f64, AnalyticsError> {
    check_pairs("Spearman correlation", xs, ys)?;
    pearson(&ranks(xs), &ranks(ys))
        .map_err(|_| AnalyticsError::undefined("Spearman correlation", "one of the variables is constant"))
}

/// Ordinary least-squares line `y = intercept + slope·x`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinearFit {
    /// Slope.
    pub slope: f64,
    /// Intercept.
    pub intercept: f64,
    /// Coefficient of determination `1 − SS_res / SS_tot` (1 for a constant
    /// `y`, which the line fits exactly).
    pub r_squared: f64,
    /// Number of points used.
    pub samples: usize,
}

impl LinearFit {
    /// Value of the fitted line at `x`.
    pub fn predict(&self, x: f64) -> f64 {
        self.intercept + self.slope * x
    }
}

/// Least-squares regression of `ys` on `xs` (at least two points, `xs` not
/// all equal).
pub fn linear_regression(xs: &[f64], ys: &[f64]) -> Result<LinearFit, AnalyticsError> {
    check_pairs("linear regression", xs, ys)?;
    let (sxx, syy, sxy, mx, my) = centred_sums(xs, ys);
    if sxx == 0.0 {
        return Err(AnalyticsError::undefined("linear regression", "all x values are equal"));
    }
    let slope = sxy / sxx;
    let intercept = my - slope * mx;
    let r_squared = if syy == 0.0 { 1.0 } else { (sxy * sxy / (sxx * syy)).clamp(0.0, 1.0) };
    Ok(LinearFit { slope, intercept, r_squared, samples: xs.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn welford_matches_textbook_values() {
        // Classic example: 2, 4, 4, 4, 5, 5, 7, 9 has mean 5 and population sd 2.
        let s = RunningStats::from_slice(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]).unwrap();
        assert_eq!(s.mean(), Some(5.0));
        assert_eq!(s.std_dev(), Some(2.0));
        assert!(close(s.sample_variance().unwrap(), 32.0 / 7.0, 1e-12));
        assert_eq!((s.min(), s.max()), (Some(2.0), Some(9.0)));
        assert_eq!(RunningStats::new().mean(), None);
    }

    #[test]
    fn welford_is_stable_with_large_offset() {
        // Naive Σx² − (Σx)²/n loses all precision here.
        let values: Vec<f64> = (0..1000).map(|i| 1e9 + (i % 2) as f64).collect();
        let s = RunningStats::from_slice(&values).unwrap();
        assert!(close(s.variance().unwrap(), 0.25, 1e-9));
    }

    #[test]
    fn merge_equals_sequential() {
        let a: Vec<f64> = (0..37).map(|i| (i as f64).sin() * 10.0).collect();
        let b: Vec<f64> = (0..53).map(|i| (i as f64).cos() + 3.0).collect();
        let mut merged = RunningStats::from_slice(&a).unwrap();
        merged.merge(&RunningStats::from_slice(&b).unwrap());
        let all: Vec<f64> = a.iter().chain(&b).copied().collect();
        let seq = RunningStats::from_slice(&all).unwrap();
        assert_eq!(merged.count(), seq.count());
        assert!(close(merged.mean().unwrap(), seq.mean().unwrap(), 1e-12));
        assert!(close(merged.variance().unwrap(), seq.variance().unwrap(), 1e-10));
        assert_eq!(merged.min(), seq.min());
    }

    #[test]
    fn quantiles_use_linear_interpolation() {
        let v = [7.0, 1.0, 3.0, 5.0];
        // Sorted 1, 3, 5, 7: h = 3q.
        assert_eq!(quantile(&v, 0.0).unwrap(), 1.0);
        assert_eq!(quantile(&v, 1.0).unwrap(), 7.0);
        assert_eq!(median(&v).unwrap(), 4.0);
        assert_eq!(quantiles(&v, &[0.25, 0.75]).unwrap(), vec![2.5, 5.5]);
        assert!(quantile(&v, 1.5).is_err());
        assert!(quantile(&[], 0.5).is_err());
        assert!(matches!(quantile(&[1.0, f64::NAN], 0.5), Err(AnalyticsError::NonFinite { index: 1, .. })));
    }

    #[test]
    fn summary_of_known_data() {
        let s = summarize(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        assert_eq!((s.count, s.mean, s.min, s.median, s.max), (5, 3.0, 1.0, 3.0, 5.0));
        assert_eq!((s.q1, s.q3), (2.0, 4.0));
        assert!(close(s.std_dev, 2f64.sqrt(), 1e-12));
        assert_eq!(min_max(&[3.0, -1.0, 2.0]).unwrap(), (-1.0, 3.0));
        assert_eq!(mean(&[1.0, 2.0]).unwrap(), 1.5);
        assert_eq!(sample_variance(&[1.0, 3.0]).unwrap(), 2.0);
    }

    #[test]
    fn correlations() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        let y: Vec<f64> = x.iter().map(|v| 2.0 * v + 1.0).collect();
        assert!(close(pearson(&x, &y).unwrap(), 1.0, 1e-12));
        let neg: Vec<f64> = x.iter().map(|v| -v).collect();
        assert!(close(pearson(&x, &neg).unwrap(), -1.0, 1e-12));
        // Monotonic but non-linear: Spearman is exactly 1, Pearson is below 1.
        let cubic: Vec<f64> = x.iter().map(|v: &f64| v.powi(5)).collect();
        assert!(close(spearman(&x, &cubic).unwrap(), 1.0, 1e-12));
        assert!(pearson(&x, &cubic).unwrap() < 0.99);
        assert!(matches!(pearson(&x, &[1.0; 5]), Err(AnalyticsError::Undefined { .. })));
        assert!(matches!(pearson(&x, &[1.0; 4]), Err(AnalyticsError::LengthMismatch { .. })));
        // Known value: x = 1..5, y = 2,4,5,4,5 gives r = 0.7745966692.
        assert!(close(pearson(&x, &[2.0, 4.0, 5.0, 4.0, 5.0]).unwrap(), 0.774_596_669_2, 1e-9));
    }

    #[test]
    fn ranks_average_ties() {
        assert_eq!(ranks(&[10.0, 20.0, 10.0, 30.0]), vec![1.5, 3.0, 1.5, 4.0]);
    }

    #[test]
    fn regression_recovers_line() {
        let x = [0.0, 1.0, 2.0, 3.0];
        let y = [1.0, 3.0, 5.0, 7.0];
        let fit = linear_regression(&x, &y).unwrap();
        assert!(close(fit.slope, 2.0, 1e-12) && close(fit.intercept, 1.0, 1e-12));
        assert!(close(fit.r_squared, 1.0, 1e-12));
        assert_eq!(fit.predict(10.0), 21.0);
        assert!(linear_regression(&[1.0, 1.0], &[1.0, 2.0]).is_err());
    }
}
