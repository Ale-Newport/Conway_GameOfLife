//! Lenia growth mappings `G : [0, 1] → [−1, 1]` (Chan 2019 eq. 13).

use serde::{Deserialize, Serialize};

/// Shape of a growth mapping with centre `μ` and width `σ`; `G(μ) = 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrowthFunction {
    /// `2·exp(−(u − μ)²/(2σ²)) − 1`.
    Gaussian,
    /// `2·1_[μ±3σ](u)·(1 − (u − μ)²/(9σ²))⁴ − 1` (α = 4); the function Chan's
    /// Python app executes for `gn = 1` in `animals.json`.
    Polynomial,
    /// `2·1_[μ±σ](u) − 1`.
    Rectangular,
}

impl GrowthFunction {
    /// Every growth function, in display order.
    pub const ALL: [GrowthFunction; 3] = [Self::Gaussian, Self::Polynomial, Self::Rectangular];

    /// Stable key used in parameters and scenario files.
    pub fn key(self) -> &'static str {
        match self {
            Self::Gaussian => "gaussian",
            Self::Polynomial => "polynomial",
            Self::Rectangular => "rectangular",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Gaussian => "Gaussian",
            Self::Polynomial => "Polynomial (quad4)",
            Self::Rectangular => "Rectangular",
        }
    }

    /// Parses a [`GrowthFunction::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.key() == key)
    }

    /// Reference evaluation in `f64`.
    pub fn eval(self, u: f64, mu: f64, sigma: f64) -> f64 {
        let d = u - mu;
        match self {
            Self::Gaussian => 2.0 * (-d * d / (2.0 * sigma * sigma)).exp() - 1.0,
            Self::Polynomial => {
                if d.abs() <= 3.0 * sigma {
                    2.0 * (1.0 - d * d / (9.0 * sigma * sigma)).powi(4) - 1.0
                } else {
                    -1.0
                }
            }
            Self::Rectangular => {
                if d.abs() <= sigma {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }

    /// Precomputes constants for fast per-cell `f32` evaluation.
    pub fn prepare(self, mu: f64, sigma: f64) -> PreparedGrowth {
        PreparedGrowth {
            function: self,
            mu: mu as f32,
            sigma: sigma as f32,
            inv_two_sigma2: (1.0 / (2.0 * sigma * sigma)) as f32,
            inv_nine_sigma2: (1.0 / (9.0 * sigma * sigma)) as f32,
        }
    }
}

/// A growth function with its `μ`, `σ` and derived constants, evaluated in
/// `f32` on every cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedGrowth {
    function: GrowthFunction,
    mu: f32,
    sigma: f32,
    inv_two_sigma2: f32,
    inv_nine_sigma2: f32,
}

impl PreparedGrowth {
    /// `G(u)` in `f32`.
    #[inline]
    pub fn eval(&self, u: f32) -> f32 {
        let d = u - self.mu;
        match self.function {
            GrowthFunction::Gaussian => 2.0 * (-d * d * self.inv_two_sigma2).exp() - 1.0,
            GrowthFunction::Polynomial => {
                let q = 1.0 - d * d * self.inv_nine_sigma2;
                if q >= 0.0 {
                    let q2 = q * q;
                    2.0 * q2 * q2 - 1.0
                } else {
                    -1.0
                }
            }
            GrowthFunction::Rectangular => {
                if d.abs() <= self.sigma {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 0.15;
    const SIGMA: f64 = 0.015;

    #[test]
    fn gaussian_reference_values() {
        let g = |u| GrowthFunction::Gaussian.eval(u, MU, SIGMA);
        assert!((g(MU) - 1.0).abs() < 1e-12);
        for s in [-1.0, 1.0] {
            assert!((g(MU + s * SIGMA) - 0.213_061_3).abs() < 1e-7);
            assert!((g(MU + s * 2.0 * SIGMA) + 0.729_329_4).abs() < 1e-7);
            assert!((g(MU + s * 3.0 * SIGMA) + 0.977_782_0).abs() < 1e-7);
        }
        assert!((g(0.0) + 1.0).abs() < 1e-12);
        // Zero crossing at |u − μ| = σ·√(2 ln 2) = 1.17741σ.
        assert!(g(MU + 1.177_41 * SIGMA).abs() < 1e-5);
    }

    #[test]
    fn polynomial_reference_values() {
        let g = |u| GrowthFunction::Polynomial.eval(u, MU, SIGMA);
        assert!((g(MU) - 1.0).abs() < 1e-12);
        assert!((g(MU + SIGMA) - 0.248_590_2).abs() < 1e-7);
        assert!((g(MU - 2.0 * SIGMA) + 0.809_480_3).abs() < 1e-7);
        assert!((g(MU + 3.0 * SIGMA) + 1.0).abs() < 1e-12);
        assert_eq!(g(MU + 4.0 * SIGMA), -1.0);
        assert!(g(MU + 1.196_63 * SIGMA).abs() < 1e-5);
    }

    #[test]
    fn rectangular_edges() {
        let g = |u| GrowthFunction::Rectangular.eval(u, 0.35, 0.07);
        assert_eq!(g(0.35), 1.0);
        assert_eq!(g(0.30), 1.0);
        assert_eq!(g(0.45), -1.0);
        assert_eq!(g(0.2), -1.0);
    }

    #[test]
    fn f32_path_matches_reference() {
        for f in GrowthFunction::ALL {
            let p = f.prepare(MU, SIGMA);
            for i in 0..=1000 {
                let u = i as f64 / 1000.0 * 0.4;
                let expected = f.eval(u, MU, SIGMA);
                let got = f64::from(p.eval(u as f32));
                // Rectangular may flip exactly at the edges due to rounding.
                if f != GrowthFunction::Rectangular || ((u - MU).abs() - SIGMA).abs() > 1e-6 {
                    assert!((got - expected).abs() < 2e-5, "{f:?} u={u}: {got} vs {expected}");
                }
            }
            assert_eq!(GrowthFunction::from_key(f.key()), Some(f));
        }
    }
}
