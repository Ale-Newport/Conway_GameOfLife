//! Radially symmetric convolution kernels: Lenia shells (Chan 2019 eqs.
//! 10–12), SmoothLife's anti-aliased disc and annulus (Rafler 2011) and
//! Flow-Lenia's Gaussian rings (Plantec et al. 2023 eq. 1).
//!
//! Kernels are sampled at integer cell offsets `n` (cell centres), built
//! centred at offset (0, 0) with wrap-around — so FFT convolution needs no
//! `fftshift` — and normalised in `f64` so that `Σ K = 1`.

use serde::{Deserialize, Serialize};

use crate::error::ContinuousError;

/// Lenia kernel core `K_C : [0, 1] → [0, 1]` (Chan 2019 Table 1, α = 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelCore {
    /// `exp(4 − 1/(r(1 − r)))` ("bump4"), zero at `r = 0` and `r = 1`.
    Exponential,
    /// `(4r(1 − r))⁴` ("quad4"); the core that Chan's Python app executes for
    /// `kn = 1` in `animals.json`.
    Polynomial,
    /// `1` on `[¼, ¾]`, else `0` ("stpz1/4").
    Rectangular,
    /// `1` on `[¼, ¾]`, `½` on `[0, ¼)`, else `0` (Chan's Game-of-Life core).
    Staircase,
    /// `exp(−((r − ½)/0.15)²/2)`, the "bell" kernel of the Lenia tutorial
    /// (not zero at the ends; the shell masks `r ≥ 1`).
    GaussianBump,
}

impl KernelCore {
    /// Every core, in display order.
    pub const ALL: [KernelCore; 5] =
        [Self::Exponential, Self::Polynomial, Self::Rectangular, Self::Staircase, Self::GaussianBump];

    /// Stable key used in parameters and scenario files.
    pub fn key(self) -> &'static str {
        match self {
            Self::Exponential => "exponential",
            Self::Polynomial => "polynomial",
            Self::Rectangular => "rectangular",
            Self::Staircase => "staircase",
            Self::GaussianBump => "gaussian_bump",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Exponential => "Exponential (bump4)",
            Self::Polynomial => "Polynomial (quad4)",
            Self::Rectangular => "Rectangular",
            Self::Staircase => "Staircase",
            Self::GaussianBump => "Gaussian bump (tutorial)",
        }
    }

    /// Parses a [`KernelCore::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key() == key)
    }

    /// Core value at radial position `r ∈ [0, 1]` (0 outside).
    pub fn eval(self, r: f64) -> f64 {
        if !(0.0..=1.0).contains(&r) {
            return 0.0;
        }
        match self {
            Self::Exponential => {
                if r <= 0.0 || r >= 1.0 {
                    0.0
                } else {
                    (4.0 - 1.0 / (r * (1.0 - r))).exp()
                }
            }
            Self::Polynomial => (4.0 * r * (1.0 - r)).powi(4),
            Self::Rectangular => f64::from(u8::from((0.25..=0.75).contains(&r))),
            Self::Staircase => {
                if (0.25..=0.75).contains(&r) {
                    1.0
                } else if r < 0.25 {
                    0.5
                } else {
                    0.0
                }
            }
            Self::GaussianBump => (-((r - 0.5) / 0.15).powi(2) / 2.0).exp(),
        }
    }
}

/// Lenia kernel shell `K_S(r; β) = β_{⌊Br⌋} · K_C(Br mod 1)` for `r < 1`
/// (Chan 2019 eq. 11), where `r` is the distance in kernel radii and `B` the
/// number of rings.
pub fn lenia_shell(r: f64, beta: &[f64], core: KernelCore) -> f64 {
    if beta.is_empty() || !(0.0..1.0).contains(&r) {
        return 0.0;
    }
    let br = beta.len() as f64 * r;
    let ring = (br.floor() as usize).min(beta.len() - 1);
    beta[ring] * core.eval(br - br.floor())
}

/// Anti-aliased indicator of the disc of radius `radius` with a linear rim
/// of width `rim` (Rafler 2011 §2): 1 inside `radius − rim/2`, 0 outside
/// `radius + rim/2`, linear in between.
pub fn antialiased_disc(distance: f64, radius: f64, rim: f64) -> f64 {
    if rim <= 0.0 {
        return f64::from(u8::from(distance <= radius));
    }
    ((radius + rim / 2.0 - distance) / rim).clamp(0.0, 1.0)
}

/// One Gaussian ring of a Flow-Lenia kernel: `b · exp(−(D − a)²/(2w²))` with
/// `D` the distance in kernel radii.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GaussianRing {
    /// Ring centre `a ∈ [0, 1]` (kernel radii).
    pub center: f64,
    /// Ring height `b ∈ [0, 1]`.
    pub height: f64,
    /// Ring width `w > 0` (kernel radii).
    pub width: f64,
}

/// Sum of Gaussian rings at distance `d` (kernel radii), masked to `d < 1`
/// (Plantec et al. 2023 eq. 1, the paper's `2w²` convention).
pub fn gaussian_rings(d: f64, rings: &[GaussianRing]) -> f64 {
    if !(0.0..1.0).contains(&d) {
        return 0.0;
    }
    rings.iter().map(|g| g.height * (-(d - g.center).powi(2) / (2.0 * g.width * g.width)).exp()).sum()
}

/// One non-zero kernel entry at offset `(dx, dy)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    /// Column offset.
    pub dx: i32,
    /// Row offset.
    pub dy: i32,
    /// Normalised weight.
    pub weight: f32,
}

/// A kernel sampled for a `width × height` torus and normalised to
/// `Σ K = 1`. Only the non-zero taps are stored (a few hundred to a few
/// thousand values); the dense wrapped image needed by the FFT is produced on
/// demand with [`KernelImage::write_dense`], so validating a kernel against a
/// large world costs no world-sized memory.
#[derive(Clone, Debug, PartialEq)]
pub struct KernelImage {
    width: usize,
    height: usize,
    taps: Vec<Tap>,
    raw_sum: f64,
    reach: usize,
}

impl KernelImage {
    /// Samples `profile(distance_in_cells)` at every integer offset within
    /// `support` cells of the origin and normalises in `f64` for a
    /// `width × height` torus. `index` only labels errors.
    ///
    /// Fails when every weight is zero (or not finite), or when a non-zero
    /// weight lies so far from the centre that the kernel would overlap
    /// itself on the torus (`2·reach + 1 > min(width, height)`).
    pub fn radial(
        index: usize,
        width: usize,
        height: usize,
        support: f64,
        profile: impl Fn(f64) -> f64,
    ) -> Result<Self, ContinuousError> {
        let limit = width.max(height) as f64;
        let extent = support.max(0.0).min(limit).ceil() as i64;
        let mut raw: Vec<(i32, i32, f64)> = Vec::new();
        let mut raw_sum = 0.0f64;
        let mut reach = 0usize;
        for dy in -extent..=extent {
            for dx in -extent..=extent {
                let d = ((dx * dx + dy * dy) as f64).sqrt();
                let w = profile(d);
                if !w.is_finite() || w < 0.0 {
                    return Err(ContinuousError::rule(format!(
                        "kernel {index} has an invalid weight {w} at distance {d}"
                    )));
                }
                if w > 0.0 {
                    raw.push((dx as i32, dy as i32, w));
                    raw_sum += w;
                    reach = reach.max(dx.unsigned_abs() as usize).max(dy.unsigned_abs() as usize);
                }
            }
        }
        if raw.is_empty() || !raw_sum.is_finite() || raw_sum <= 0.0 {
            return Err(ContinuousError::EmptyKernel(index));
        }
        let needed = 2 * reach + 1;
        if needed > width.min(height) {
            return Err(ContinuousError::KernelTooLarge { kernel: index, reach, needed, width, height });
        }
        let taps: Vec<Tap> = raw.iter().map(|&(dx, dy, w)| Tap { dx, dy, weight: (w / raw_sum) as f32 }).collect();
        Ok(Self { width, height, taps, raw_sum, reach })
    }

    /// Writes the normalised kernel onto the torus, centred at index 0 with
    /// wrap-around (row-major, `width × height`), as needed for FFT
    /// convolution without `fftshift`. Every other entry is set to zero.
    pub fn write_dense(&self, out: &mut [f32]) {
        debug_assert_eq!(out.len(), self.width * self.height);
        out.fill(0.0);
        for t in &self.taps {
            // The fit check guarantees that distinct taps land on distinct cells.
            let x = i64::from(t.dx).rem_euclid(self.width as i64) as usize;
            let y = i64::from(t.dy).rem_euclid(self.height as i64) as usize;
            out[y * self.width + x] = t.weight;
        }
    }

    /// The dense wrapped kernel as a new buffer (see [`KernelImage::write_dense`]).
    pub fn dense(&self) -> Vec<f32> {
        let mut out = vec![0.0; self.width * self.height];
        self.write_dense(&mut out);
        out
    }

    /// Grid width.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Grid height.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Non-zero entries, sorted by row offset then column offset.
    pub fn taps(&self) -> &[Tap] {
        &self.taps
    }

    /// Sum of the weights before normalisation (SmoothLife's `M` and `N`).
    pub fn raw_sum(&self) -> f64 {
        self.raw_sum
    }

    /// Largest non-zero offset along either axis.
    pub fn reach(&self) -> usize {
        self.reach
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_reference_values() {
        // Values from docs/research/continuous-life.md §8 (computed independently).
        let e = KernelCore::Exponential;
        assert!((e.eval(0.5) - 1.0).abs() < 1e-12);
        assert!((e.eval(0.25) - (-4.0f64 / 3.0).exp()).abs() < 1e-12);
        assert!((e.eval(0.25) - 0.263_597_1).abs() < 1e-7);
        assert!((e.eval(0.75) - 0.263_597_1).abs() < 1e-7);
        assert!((e.eval(0.1) - 8.159_88e-4).abs() < 1e-8);
        assert_eq!(e.eval(0.0), 0.0);
        assert_eq!(e.eval(1.0), 0.0);
        assert!((KernelCore::Polynomial.eval(0.25) - 0.316_406_25).abs() < 1e-15);
        assert!((KernelCore::GaussianBump.eval(0.25) - 0.249_352_2).abs() < 1e-7);
        assert_eq!(KernelCore::Rectangular.eval(0.2), 0.0);
        assert_eq!(KernelCore::Rectangular.eval(0.5), 1.0);
        assert_eq!(KernelCore::Staircase.eval(0.1), 0.5);
        assert_eq!(KernelCore::Staircase.eval(0.8), 0.0);
        for c in KernelCore::ALL {
            assert_eq!(KernelCore::from_key(c.key()), Some(c));
        }
    }

    #[test]
    fn shell_rings_peak_at_ring_centres() {
        let beta = [0.5, 1.0, 2.0 / 3.0];
        for (i, &b) in beta.iter().enumerate() {
            let r = (i as f64 + 0.5) / 3.0;
            assert!((lenia_shell(r, &beta, KernelCore::Exponential) - b).abs() < 1e-12);
        }
        assert_eq!(lenia_shell(1.0, &beta, KernelCore::GaussianBump), 0.0);
    }

    #[test]
    fn kernels_are_normalised_and_wrapped() {
        let k = KernelImage::radial(0, 64, 48, 13.0, |d| lenia_shell(d / 13.0, &[1.0], KernelCore::Exponential))
            .expect("valid kernel");
        let dense = k.dense();
        let sum: f64 = dense.iter().map(|&v| f64::from(v)).sum();
        assert!((sum - 1.0).abs() < 1e-6, "sum {sum}");
        // 516 non-zero taps for the exponential core at R = 13 (the centre is 0).
        assert_eq!(k.taps().len(), 516);
        assert_eq!(dense.iter().filter(|&&v| v > 0.0).count(), 516);
        assert_eq!(dense[0], 0.0);
        // Symmetry on the torus: K(1, 0) == K(−1, 0) and K(0, 1) == K(0, −1).
        assert_eq!(dense[1], dense[63]);
        assert_eq!(dense[64], dense[47 * 64]);
        // The kernel is invariant under the eight symmetries of the square
        // grid (exactly, since it only depends on dx² + dy²).
        let at = |dx: i32, dy: i32| k.taps().iter().find(|t| (t.dx, t.dy) == (dx, dy)).map(|t| t.weight);
        for t in k.taps() {
            for (sx, sy) in [(-t.dx, t.dy), (t.dx, -t.dy), (t.dy, t.dx), (-t.dy, -t.dx)] {
                assert_eq!(at(sx, sy), Some(t.weight));
            }
        }
        let bell = KernelImage::radial(0, 64, 64, 13.0, |d| lenia_shell(d / 13.0, &[1.0], KernelCore::GaussianBump))
            .expect("valid kernel");
        assert_eq!(bell.taps().len(), 517);
    }

    #[test]
    fn kernel_errors() {
        assert!(matches!(
            KernelImage::radial(2, 20, 20, 13.0, |d| lenia_shell(d / 13.0, &[1.0], KernelCore::Polynomial)),
            Err(ContinuousError::KernelTooLarge { kernel: 2, .. })
        ));
        assert_eq!(KernelImage::radial(1, 20, 20, 3.0, |_| 0.0), Err(ContinuousError::EmptyKernel(1)));
    }

    #[test]
    fn smoothlife_areas_match_rafler_normalisation() {
        // M and N for r_a = 21, r_i = 7, b = 1 (research note §5.1).
        let disc = KernelImage::radial(0, 64, 64, 7.5, |d| antialiased_disc(d, 7.0, 1.0)).expect("disc");
        let ring =
            KernelImage::radial(1, 64, 64, 21.5, |d| antialiased_disc(d, 21.0, 1.0) - antialiased_disc(d, 7.0, 1.0))
                .expect("annulus");
        assert!((disc.raw_sum() - 154.55).abs() < 0.01, "M = {}", disc.raw_sum());
        assert!((ring.raw_sum() - 1231.23).abs() < 0.01, "N = {}", ring.raw_sum());
    }

    #[test]
    fn gaussian_rings_are_masked() {
        let rings = [GaussianRing { center: 0.5, height: 1.0, width: 0.1 }];
        assert!((gaussian_rings(0.5, &rings) - 1.0).abs() < 1e-12);
        assert!((gaussian_rings(0.6, &rings) - (-0.5f64).exp()).abs() < 1e-12);
        assert_eq!(gaussian_rings(1.0, &rings), 0.0);
    }
}
