//! Minimal radix-2 FFT used for fast autocorrelation of long series.
//!
//! The transform is the iterative Cooley–Tukey algorithm on power-of-two
//! lengths with a precomputed twiddle table (each factor computed directly
//! with `sin_cos`, so rounding errors do not accumulate across a stage).

use std::f64::consts::TAU;

/// In-place FFT of `re + i·im` (equal power-of-two lengths). The forward
/// transform uses `e^{−2πi jk/n}`; the inverse uses `e^{+2πi jk/n}` and is not
/// scaled by `1/n`.
fn transform(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let twiddles: Vec<(f64, f64)> = (0..n / 2)
        .map(|k| {
            let (s, c) = (TAU * k as f64 / n as f64).sin_cos();
            (c, sign * s)
        })
        .collect();
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let stride = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let (wr, wi) = twiddles[k * stride];
                let (a, b) = (start + k, start + k + half);
                let xr = re[b] * wr - im[b] * wi;
                let xi = re[b] * wi + im[b] * wr;
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len <<= 1;
    }
}

/// Raw autocovariance sums `c(k) = Σₜ xₜ xₜ₊ₖ` for `k = 0..=max_lag`
/// (`max_lag < x.len()`), computed with zero padding so the circular
/// correlation equals the linear one.
pub(crate) fn autocovariance_sums(x: &[f64], max_lag: usize) -> Vec<f64> {
    let size = (x.len() + max_lag + 1).next_power_of_two();
    let mut re = vec![0.0; size];
    re[..x.len()].copy_from_slice(x);
    let mut im = vec![0.0; size];
    transform(&mut re, &mut im, false);
    for (r, i) in re.iter_mut().zip(im.iter_mut()) {
        *r = *r * *r + *i * *i;
        *i = 0.0;
    }
    transform(&mut re, &mut im, true);
    let scale = 1.0 / size as f64;
    re[..=max_lag].iter().map(|v| v * scale).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_then_inverse_is_identity() {
        let mut rng = simulation_core::SimRng::seed_from_u64(5);
        let original: Vec<f64> = (0..64).map(|_| rng.range_f64(-1.0, 1.0)).collect();
        let mut re = original.clone();
        let mut im = vec![0.0; 64];
        transform(&mut re, &mut im, false);
        transform(&mut re, &mut im, true);
        for (a, b) in re.iter().zip(&original) {
            assert!((a / 64.0 - b).abs() < 1e-14);
        }
        assert!(im.iter().all(|v| v.abs() < 1e-12));
    }

    #[test]
    fn transform_of_impulse_and_cosine() {
        // DFT of a unit impulse is all ones.
        let mut re = vec![0.0; 8];
        re[0] = 1.0;
        let mut im = vec![0.0; 8];
        transform(&mut re, &mut im, false);
        assert!(re.iter().all(|v| (v - 1.0).abs() < 1e-15) && im.iter().all(|v| v.abs() < 1e-15));
        // cos(2π·3t/16) has energy n/2 at bins 3 and 13 only.
        let mut re: Vec<f64> = (0..16).map(|t| (TAU * 3.0 * t as f64 / 16.0).cos()).collect();
        let mut im = vec![0.0; 16];
        transform(&mut re, &mut im, false);
        for (k, (r, i)) in re.iter().zip(&im).enumerate() {
            let expected = if k == 3 || k == 13 { 8.0 } else { 0.0 };
            assert!((r - expected).abs() < 1e-12 && i.abs() < 1e-12, "bin {k}: {r} {i}");
        }
    }

    #[test]
    fn autocovariance_matches_direct_sums() {
        let mut rng = simulation_core::SimRng::seed_from_u64(9);
        for n in [1usize, 2, 7, 100, 1_000] {
            let x: Vec<f64> = (0..n).map(|_| rng.range_f64(-5.0, 5.0)).collect();
            let max_lag = n - 1;
            let fast = autocovariance_sums(&x, max_lag);
            for (k, value) in fast.iter().enumerate() {
                let direct: f64 = x.iter().zip(&x[k..]).map(|(a, b)| a * b).sum();
                assert!((value - direct).abs() < 1e-9 * (1.0 + direct.abs()), "n {n} lag {k}: {value} vs {direct}");
            }
        }
    }
}
