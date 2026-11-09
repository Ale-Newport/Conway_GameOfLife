//! Population dynamics: growth rates, oscillation periods and extinction.

use serde::{Deserialize, Serialize};

use crate::error::{AnalyticsError, check_values};
use crate::fft;
use crate::series::{Point, TimeSeriesStore};
use crate::stats::linear_regression;

/// Exponential growth model `N(t) ≈ N₀ e^{r t}` fitted by least squares on
/// `ln N`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrowthFit {
    /// Intrinsic growth rate `r` per tick.
    pub rate: f64,
    /// Fitted `ln N₀` (intercept at tick 0).
    pub log_initial: f64,
    /// Coefficient of determination of the log-linear fit.
    pub r_squared: f64,
    /// Number of positive samples used.
    pub samples: usize,
}

impl GrowthFit {
    /// Finite rate of increase `λ = e^r` (multiplicative change per tick).
    pub fn finite_rate(&self) -> f64 {
        self.rate.exp()
    }

    /// Doubling time `ln 2 / r` for a growing population.
    pub fn doubling_time(&self) -> Option<f64> {
        (self.rate > 0.0).then(|| std::f64::consts::LN_2 / self.rate)
    }

    /// Halving time `ln 2 / −r` for a declining population.
    pub fn halving_time(&self) -> Option<f64> {
        (self.rate < 0.0).then(|| std::f64::consts::LN_2 / -self.rate)
    }
}

/// Fits exponential growth to the positive samples of a population series.
/// Samples with `N ≤ 0` are skipped (the logarithm is undefined); at least two
/// positive samples at different ticks are required.
pub fn exponential_growth(points: &[Point]) -> Result<GrowthFit, AnalyticsError> {
    let (ts, logs): (Vec<f64>, Vec<f64>) =
        points.iter().filter(|p| p.value > 0.0 && p.value.is_finite()).map(|p| (p.tick as f64, p.value.ln())).unzip();
    if ts.len() < 2 {
        return Err(AnalyticsError::NotEnoughData {
            what: "exponential growth fit (positive samples)",
            needed: 2,
            got: ts.len(),
        });
    }
    let fit = linear_regression(&ts, &logs)?;
    Ok(GrowthFit { rate: fit.slope, log_initial: fit.intercept, r_squared: fit.r_squared, samples: fit.samples })
}

/// Per-interval growth rates `r = ln(N₁ / N₀) / (t₁ − t₀)` between
/// consecutive samples, attributed to the later tick. Intervals touching a
/// non-positive population are skipped.
pub fn growth_rates(points: &[Point]) -> Vec<Point> {
    points
        .windows(2)
        .filter(|w| w[0].value > 0.0 && w[1].value > 0.0 && w[1].tick > w[0].tick)
        .map(|w| Point { tick: w[1].tick, value: (w[1].value / w[0].value).ln() / (w[1].tick - w[0].tick) as f64 })
        .collect()
}

/// Sample autocorrelation `r(k) = Σ (xₜ − x̄)(xₜ₊ₖ − x̄) / Σ (xₜ − x̄)²` for
/// lags `0..=max_lag` (biased estimator, `r(0) = 1`). Undefined for a
/// constant series.
///
/// The sums are computed directly (O(n · max_lag)) when that is cheap and
/// with a zero-padded FFT (O(n log n)) otherwise, so long series can be
/// analysed over all lags; both agree to rounding error.
pub fn autocorrelation(values: &[f64], max_lag: usize) -> Result<Vec<f64>, AnalyticsError> {
    check_values("autocorrelation", values, 2)?;
    if max_lag >= values.len() {
        return Err(AnalyticsError::invalid(
            "autocorrelation",
            format!("max_lag {max_lag} must be smaller than the series length {}", values.len()),
        ));
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let dev: Vec<f64> = values.iter().map(|x| x - mean).collect();
    let denom: f64 = dev.iter().map(|d| d * d).sum();
    if denom == 0.0 {
        return Err(AnalyticsError::undefined("autocorrelation", "the series is constant"));
    }
    let n = values.len();
    let fft_size = (n + max_lag + 1).next_power_of_two();
    let direct_cost = n.saturating_mul(max_lag + 1);
    let fft_cost = fft_size.saturating_mul(16 * fft_size.trailing_zeros().max(1) as usize);
    let sums: Vec<f64> = if direct_cost <= fft_cost {
        (0..=max_lag).map(|k| dev.iter().zip(&dev[k..]).map(|(a, b)| a * b).sum()).collect()
    } else {
        fft::autocovariance_sums(&dev, max_lag)
    };
    let mut acf: Vec<f64> = sums.iter().map(|s| s / denom).collect();
    acf[0] = 1.0;
    Ok(acf)
}

/// Settings of [`estimate_period`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodOptions {
    /// Smallest period considered (in samples, at least 2).
    pub min_lag: usize,
    /// Largest period considered; `None` uses half the series length.
    pub max_lag: Option<usize>,
    /// Minimum autocorrelation of the chosen peak (in `[-1, 1]`, normally
    /// between 0 and 1).
    pub min_strength: f64,
    /// A shorter period is preferred over the strongest peak when its
    /// autocorrelation is at least this fraction (in `(0, 1]`) of the
    /// strongest one, so the fundamental wins over its multiples.
    pub harmonic_tolerance: f64,
}

impl Default for PeriodOptions {
    fn default() -> Self {
        Self { min_lag: 2, max_lag: None, min_strength: 0.3, harmonic_tolerance: 0.85 }
    }
}

/// Dominant oscillation period found by [`estimate_period`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodEstimate {
    /// Lag (in samples) of the chosen autocorrelation peak.
    pub lag: usize,
    /// Peak position refined by parabolic interpolation (in samples).
    pub refined: f64,
    /// Autocorrelation at `lag` (1 = perfectly periodic).
    pub strength: f64,
}

/// Estimates the oscillation period of evenly sampled values from the
/// autocorrelation function: the local maxima of `r(k)` in
/// `[min_lag, max_lag]` with `r(k) ≥ min_strength` are candidates, and the
/// smallest lag whose peak reaches `harmonic_tolerance ×` the strongest peak
/// is returned. `Ok(None)` means no periodicity was detected (including a
/// constant series). Trends should be removed by the caller (e.g. by
/// analysing growth rates instead of raw populations).
pub fn estimate_period(values: &[f64], options: &PeriodOptions) -> Result<Option<PeriodEstimate>, AnalyticsError> {
    check_values("period estimate", values, 4)?;
    if !(-1.0..=1.0).contains(&options.min_strength) {
        return Err(AnalyticsError::invalid(
            "period estimate",
            format!("min_strength {} must be in [-1, 1]", options.min_strength),
        ));
    }
    if !(options.harmonic_tolerance > 0.0 && options.harmonic_tolerance <= 1.0) {
        return Err(AnalyticsError::invalid(
            "period estimate",
            format!("harmonic_tolerance {} must be in (0, 1]", options.harmonic_tolerance),
        ));
    }
    let n = values.len();
    let max_lag = options.max_lag.unwrap_or(n / 2).min(n - 2);
    let min_lag = options.min_lag.max(2);
    if min_lag > max_lag {
        return Err(AnalyticsError::invalid(
            "period estimate",
            format!("lag range [{min_lag}, {max_lag}] is empty for {n} samples"),
        ));
    }
    if values.iter().all(|&v| v == values[0]) {
        return Ok(None);
    }
    // One extra lag so a peak at `max_lag` can be confirmed.
    let acf = autocorrelation(values, max_lag + 1)?;
    let candidates: Vec<usize> = (min_lag..=max_lag)
        .filter(|&k| acf[k] > acf[k - 1] && acf[k] >= acf[k + 1] && acf[k] >= options.min_strength)
        .collect();
    let Some(best) = candidates.iter().map(|&k| acf[k]).reduce(f64::max) else {
        return Ok(None);
    };
    let lag =
        candidates.iter().copied().find(|&k| acf[k] >= options.harmonic_tolerance * best).unwrap_or(candidates[0]);
    let (a, b, c) = (acf[lag - 1], acf[lag], acf[lag + 1]);
    let curvature = a - 2.0 * b + c;
    let offset = if curvature < 0.0 { (0.5 * (a - c) / curvature).clamp(-0.5, 0.5) } else { 0.0 };
    Ok(Some(PeriodEstimate { lag, refined: lag as f64 + offset, strength: b }))
}

/// Oscillation period of a series in ticks. The samples must be evenly spaced
/// in ticks (the period in samples is multiplied by the spacing).
pub fn estimate_series_period(points: &[Point], options: &PeriodOptions) -> Result<Option<f64>, AnalyticsError> {
    if points.len() < 2 {
        return Err(AnalyticsError::NotEnoughData { what: "period estimate", needed: 4, got: points.len() });
    }
    let spacing = points[1].tick.saturating_sub(points[0].tick);
    if spacing == 0 || points.windows(2).any(|w| w[1].tick.checked_sub(w[0].tick) != Some(spacing)) {
        return Err(AnalyticsError::invalid("period estimate", "samples must be evenly spaced in ticks"));
    }
    let values: Vec<f64> = points.iter().map(|p| p.value).collect();
    Ok(estimate_period(&values, options)?.map(|e| e.refined * spacing as f64))
}

/// Tick at which a population went extinct for good: the first sample of
/// the final run of samples with `value ≤ threshold`. `None` when the last
/// sample is above the threshold, the series is empty or the threshold is
/// NaN (nothing compares as `≤ NaN`). Extinction is judged on the recorded
/// data only, and applies to whatever the series measures (meaningful for
/// populations and other gauges, not for per-tick counters).
pub fn extinction_tick(points: &[Point], threshold: f64) -> Option<u64> {
    if threshold.is_nan() {
        return None;
    }
    let last_alive = points.iter().rposition(|p| p.value > threshold);
    match last_alive {
        Some(i) if i + 1 == points.len() => None,
        Some(i) => Some(points[i + 1].tick),
        None => points.first().map(|p| p.tick),
    }
}

/// Every series of `store` that ends extinct (see [`extinction_tick`]), as
/// `(key, tick)` pairs in key order of first appearance.
pub fn extinctions(store: &TimeSeriesStore, threshold: f64) -> Vec<(String, u64)> {
    store.iter().filter_map(|s| extinction_tick(s.points(), threshold).map(|t| (s.key().to_string(), t))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(values: &[f64]) -> Vec<Point> {
        values.iter().enumerate().map(|(i, &v)| Point::new(i as u64, v)).collect()
    }

    #[test]
    fn exponential_growth_recovers_rate() {
        let pts: Vec<Point> = (0..50).map(|t| Point::new(t, 10.0 * (0.1 * t as f64).exp())).collect();
        let fit = exponential_growth(&pts).unwrap();
        assert!((fit.rate - 0.1).abs() < 1e-12);
        assert!((fit.log_initial - 10f64.ln()).abs() < 1e-10);
        assert!((fit.doubling_time().unwrap() - std::f64::consts::LN_2 / 0.1).abs() < 1e-9);
        assert_eq!(fit.halving_time(), None);
        assert!((fit.finite_rate() - 0.1f64.exp()).abs() < 1e-12);
        assert!(exponential_growth(&series(&[0.0, 0.0, 5.0])).is_err());
    }

    #[test]
    fn per_interval_growth_rates() {
        let pts = vec![Point::new(0, 100.0), Point::new(2, 400.0), Point::new(3, 0.0), Point::new(4, 10.0)];
        let rates = growth_rates(&pts);
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].tick, 2);
        assert!((rates[0].value - 4f64.ln() / 2.0).abs() < 1e-12);
    }

    #[test]
    fn autocorrelation_of_alternating_sequence() {
        let acf = autocorrelation(&[1.0, -1.0, 1.0, -1.0, 1.0, -1.0], 2).unwrap();
        assert_eq!(acf[0], 1.0);
        assert!((acf[1] + 5.0 / 6.0).abs() < 1e-12);
        assert!((acf[2] - 4.0 / 6.0).abs() < 1e-12);
        assert!(autocorrelation(&[2.0; 5], 1).is_err());
    }

    #[test]
    fn fast_and_direct_autocorrelation_agree() {
        // Long enough for the FFT path; checked against the defining sums.
        let mut rng = simulation_core::SimRng::seed_from_u64(21);
        let values: Vec<f64> = (0..4_000).map(|t| (t as f64 * 0.05).sin() * 10.0 + rng.normal() + 1e3).collect();
        let acf = autocorrelation(&values, 3_999).unwrap();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let dev: Vec<f64> = values.iter().map(|x| x - mean).collect();
        let denom: f64 = dev.iter().map(|d| d * d).sum();
        assert_eq!(acf[0], 1.0);
        for k in [1, 2, 63, 126, 1_000, 3_998, 3_999] {
            let direct: f64 = dev.iter().zip(&dev[k..]).map(|(a, b)| a * b).sum::<f64>() / denom;
            assert!((acf[k] - direct).abs() < 1e-10, "lag {k}: {} vs {direct}", acf[k]);
        }
    }

    #[test]
    fn long_series_period_is_found() {
        // 200 000 samples with the default lag range (100 000 lags): the
        // direct method would need 10¹⁰ operations.
        let mut rng = simulation_core::SimRng::seed_from_u64(8);
        let values: Vec<f64> =
            (0..200_000).map(|t| (std::f64::consts::TAU * t as f64 / 137.0).sin() + 0.3 * rng.normal()).collect();
        let est = estimate_period(&values, &PeriodOptions::default()).unwrap().unwrap();
        assert!((est.refined - 137.0).abs() < 0.5, "{est:?}");
    }

    #[test]
    fn detects_pulsar_like_period_three() {
        // Population of the pulsar oscillator over its period-3 cycle.
        let values: Vec<f64> = (0..60).map(|i| [48.0, 56.0, 72.0][i % 3]).collect();
        let est = estimate_period(&values, &PeriodOptions::default()).unwrap().unwrap();
        assert_eq!(est.lag, 3);
        assert!(est.strength > 0.9);
    }

    #[test]
    fn detects_noisy_sine_period_and_prefers_fundamental() {
        let mut rng = simulation_core::SimRng::seed_from_u64(7);
        let values: Vec<f64> =
            (0..400).map(|t| (std::f64::consts::TAU * t as f64 / 25.0).sin() + 0.2 * rng.normal()).collect();
        let est = estimate_period(&values, &PeriodOptions::default()).unwrap().unwrap();
        assert!((est.refined - 25.0).abs() < 1.0, "period {est:?}");
        let pts: Vec<Point> = values.iter().enumerate().map(|(i, &v)| Point::new(i as u64 * 4, v)).collect();
        let ticks = estimate_series_period(&pts, &PeriodOptions::default()).unwrap().unwrap();
        assert!((ticks - 100.0).abs() < 4.0);
    }

    #[test]
    fn no_period_for_constant_or_monotone_data() {
        assert_eq!(estimate_period(&[5.0; 20], &PeriodOptions::default()).unwrap(), None);
        let ramp: Vec<f64> = (0..50).map(f64::from).collect();
        assert_eq!(estimate_period(&ramp, &PeriodOptions::default()).unwrap(), None);
        let uneven = vec![Point::new(0, 1.0), Point::new(1, 2.0), Point::new(3, 1.0), Point::new(4, 2.0)];
        assert!(estimate_series_period(&uneven, &PeriodOptions::default()).is_err());
    }

    #[test]
    fn invalid_period_options_are_rejected() {
        let values: Vec<f64> = (0..40).map(|i| [1.0, 3.0][i % 2]).collect();
        let bad_strength = PeriodOptions { min_strength: f64::NAN, ..PeriodOptions::default() };
        assert!(matches!(estimate_period(&values, &bad_strength), Err(AnalyticsError::InvalidArgument { .. })));
        let bad_tolerance = PeriodOptions { harmonic_tolerance: 0.0, ..PeriodOptions::default() };
        assert!(estimate_period(&values, &bad_tolerance).is_err());
        let narrow = PeriodOptions { min_lag: 30, max_lag: Some(20), ..PeriodOptions::default() };
        assert!(estimate_period(&values, &narrow).is_err());
        assert_eq!(estimate_period(&values, &PeriodOptions::default()).unwrap().map(|e| e.lag), Some(2));
    }

    #[test]
    fn detects_longer_oscillator_periods() {
        // Population sequence of a period-15 oscillator (pentadecathlon-like
        // profile): the fundamental, not a multiple, must be reported.
        let cycle = [12.0, 22.0, 30.0, 34.0, 40.0, 34.0, 28.0, 22.0, 18.0, 26.0, 36.0, 30.0, 22.0, 18.0, 14.0];
        let values: Vec<f64> = (0..300).map(|i| cycle[i % 15]).collect();
        let est = estimate_period(&values, &PeriodOptions::default()).unwrap().unwrap();
        assert_eq!(est.lag, 15);
        assert!((est.refined - 15.0).abs() < 0.5);
    }

    #[test]
    fn extinction_is_permanent_drop() {
        assert_eq!(extinction_tick(&series(&[5.0, 3.0, 0.0, 0.0]), 0.0), Some(2));
        // Recovery after a temporary zero is not an extinction.
        assert_eq!(extinction_tick(&series(&[5.0, 0.0, 2.0]), 0.0), None);
        assert_eq!(extinction_tick(&series(&[5.0, 0.0, 2.0, 0.0]), 0.0), Some(3));
        assert_eq!(extinction_tick(&series(&[0.0, 0.0]), 0.0), Some(0));
        assert_eq!(extinction_tick(&series(&[5.0, 1.0]), 1.0), Some(1));
        assert_eq!(extinction_tick(&[], 0.0), None);
        // A NaN threshold cannot declare anything extinct.
        assert_eq!(extinction_tick(&series(&[0.0, 0.0]), f64::NAN), None);
    }

    #[test]
    fn extinctions_in_store() {
        let mut store = TimeSeriesStore::new();
        for (t, (wolves, deer)) in [(4.0, 10.0), (2.0, 12.0), (0.0, 15.0)].into_iter().enumerate() {
            let m = simulation_core::Metrics::new(t as u64, t as f64).with("wolves", wolves).with("deer", deer);
            store.record(&m).unwrap();
        }
        assert_eq!(extinctions(&store, 0.0), vec![("wolves".to_string(), 2)]);
    }
}
