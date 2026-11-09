//! SmoothLife (Rafler 2011, arXiv:1111.1567).
//!
//! State `f ∈ [0, 1]`. Each tick:
//!
//! ```text
//! m = (1/M) Σ_{|u| < r_i} w_i(u)·f(x+u)       inner filling (anti-aliased disc)
//! n = (1/N) Σ_{r_i < |u| < r_a} w_a(u)·f(x+u)  outer filling (anti-aliased annulus)
//! σ1(x, a, α)   = 1 / (1 + exp(−(x − a)·4/α))
//! σ2(x, a, b)   = σ1(x, a, α_n)·(1 − σ1(x, b, α_n))
//! σm(x, y, m)   = x·(1 − σ1(m, ½, α_m)) + y·σ1(m, ½, α_m)
//! s(n, m)       = σ2(n, σm(b1, d1, m), σm(b2, d2, m))         (paper eq. 6, "bounds")
//!              or σm(σ2(n, b1, b2), σ2(n, d1, d2), m)          (Ready's SmoothLifeL, "indicators")
//! ```
//!
//! Time stepping (Rafler's modes, then clip to `[0, 1]`):
//! mode 0 `f ← s`; mode 1 `f ← f + dt·(2s − 1)`; mode 2 `f ← f + dt·(s − f)`.
//! Modes 3 and 4 of a third-party port (integrating from `m`) are not
//! implemented because they could not be verified against Rafler's code.

use serde::{Deserialize, Serialize};
use simulation_core::Grid2;

use crate::convolve::{ConvolutionMethod, ConvolutionPlan, PlannedKernel};
use crate::error::ContinuousError;
use crate::kernel::{KernelImage, antialiased_disc};
use crate::parallel::for_row_blocks;
use crate::stats::field_sum;

/// How the living-ness `m` mixes birth and survival intervals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmoothLifeMixing {
    /// Interpolate the interval bounds (Rafler 2011 eq. 6).
    #[default]
    Bounds,
    /// Interpolate the birth and survival indicators (GollyGang Ready's
    /// `smoothlifeL`; probably one of Rafler's undocumented `mixtype` options).
    Indicators,
}

impl SmoothLifeMixing {
    /// Stable key used in parameters.
    pub fn key(self) -> &'static str {
        match self {
            Self::Bounds => "bounds",
            Self::Indicators => "indicators",
        }
    }

    /// Parses a [`SmoothLifeMixing::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        [Self::Bounds, Self::Indicators].into_iter().find(|m| m.key() == key)
    }
}

/// Rafler's time-stepping modes 0–2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmoothLifeStepping {
    /// Mode 0: `f ← s(n, m)` (`dt` is ignored).
    #[default]
    Discrete,
    /// Mode 1: `f ← f + dt·(2s − 1)`.
    Smooth,
    /// Mode 2: `f ← f + dt·(s − f)` (relaxation towards `s`).
    Relax,
}

impl SmoothLifeStepping {
    /// Stable key used in parameters.
    pub fn key(self) -> &'static str {
        match self {
            Self::Discrete => "discrete",
            Self::Smooth => "smooth",
            Self::Relax => "relax",
        }
    }

    /// Parses a [`SmoothLifeStepping::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        [Self::Discrete, Self::Smooth, Self::Relax].into_iter().find(|m| m.key() == key)
    }
}

/// A SmoothLife rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmoothLifeRule {
    /// Outer radius `r_a` in cells.
    pub outer_radius: f64,
    /// Ratio `r_a / r_i` (3 in Rafler's paper).
    pub radius_ratio: f64,
    /// Anti-aliasing rim width `b` in cells (1 in the paper; 0 = hard edges).
    pub rim: f64,
    /// Birth interval `[b1, b2]`.
    pub birth: [f64; 2],
    /// Survival ("death") interval `[d1, d2]`.
    pub survival: [f64; 2],
    /// Sigmoid width `α_n` for the outer filling.
    pub alpha_n: f64,
    /// Sigmoid width `α_m` for the inner filling.
    pub alpha_m: f64,
    /// Mixing of birth and survival.
    #[serde(default)]
    pub mixing: SmoothLifeMixing,
    /// Time stepping mode.
    #[serde(default)]
    pub stepping: SmoothLifeStepping,
    /// Time step for modes 1 and 2.
    pub dt: f64,
}

/// `σ1(x, a, α) = 1 / (1 + exp(−(x − a)·4/α))` (Rafler eq. 3).
pub fn sigma1(x: f64, a: f64, alpha: f64) -> f64 {
    1.0 / (1.0 + (-(x - a) * 4.0 / alpha).exp())
}

impl SmoothLifeRule {
    /// The smooth glider of Rafler's Fig. 1: `r_a = 21`, `r_i = 7`,
    /// `b1 = 0.278`, `b2 = 0.365`, `d1 = 0.267`, `d2 = 0.445`,
    /// `α_n = 0.028`, `α_m = 0.147`, discrete stepping.
    pub fn paper_glider() -> Self {
        Self {
            outer_radius: 21.0,
            radius_ratio: 3.0,
            rim: 1.0,
            birth: [0.278, 0.365],
            survival: [0.267, 0.445],
            alpha_n: 0.028,
            alpha_m: 0.147,
            mixing: SmoothLifeMixing::Bounds,
            stepping: SmoothLifeStepping::Discrete,
            dt: 0.1,
        }
    }

    /// Inner radius `r_i = r_a / ratio`.
    pub fn inner_radius(&self) -> f64 {
        self.outer_radius / self.radius_ratio
    }

    /// Transition function `s(n, m)`.
    pub fn transition(&self, n: f64, m: f64) -> f64 {
        let alive = sigma1(m, 0.5, self.alpha_m);
        let mix = |x: f64, y: f64| x * (1.0 - alive) + y * alive;
        let sigma2 = |x: f64, a: f64, b: f64| sigma1(x, a, self.alpha_n) * (1.0 - sigma1(x, b, self.alpha_n));
        match self.mixing {
            SmoothLifeMixing::Bounds => {
                sigma2(n, mix(self.birth[0], self.survival[0]), mix(self.birth[1], self.survival[1]))
            }
            SmoothLifeMixing::Indicators => {
                mix(sigma2(n, self.birth[0], self.birth[1]), sigma2(n, self.survival[0], self.survival[1]))
            }
        }
    }

    /// Checks ranges.
    pub fn validate(&self) -> Result<(), ContinuousError> {
        let bad = |m: String| Err(ContinuousError::rule(m));
        if !(self.outer_radius.is_finite() && (1.0..=512.0).contains(&self.outer_radius)) {
            return bad(format!("outer radius r_a = {} is outside [1, 512]", self.outer_radius));
        }
        if !(self.radius_ratio.is_finite() && (1.01..=16.0).contains(&self.radius_ratio)) {
            return bad(format!("radius ratio r_a/r_i = {} is outside [1.01, 16]", self.radius_ratio));
        }
        if !(self.rim.is_finite() && (0.0..=4.0).contains(&self.rim)) {
            return bad(format!("rim width b = {} is outside [0, 4]", self.rim));
        }
        for (name, [lo, hi]) in [("birth", self.birth), ("survival", self.survival)] {
            if !(lo.is_finite() && hi.is_finite() && 0.0 <= lo && lo <= hi && hi <= 1.0) {
                return bad(format!("{name} interval [{lo}, {hi}] must satisfy 0 ≤ lower ≤ upper ≤ 1"));
            }
        }
        for (name, a) in [("α_n", self.alpha_n), ("α_m", self.alpha_m)] {
            if !(a.is_finite() && (0.0001..=1.0).contains(&a)) {
                return bad(format!("{name} = {a} is outside [0.0001, 1]"));
            }
        }
        if !(self.dt.is_finite() && (0.001..=1.0).contains(&self.dt)) {
            return bad(format!("time step dt = {} is outside [0.001, 1]", self.dt));
        }
        Ok(())
    }

    /// The disc (`m`) and annulus (`n`) kernels on a `width × height` torus.
    pub fn kernel_images(&self, width: usize, height: usize) -> Result<Vec<PlannedKernel>, ContinuousError> {
        let (ra, ri, b) = (self.outer_radius, self.inner_radius(), self.rim);
        let disc = KernelImage::radial(0, width, height, ri + b / 2.0, |d| antialiased_disc(d, ri, b))?;
        let annulus = KernelImage::radial(1, width, height, ra + b / 2.0, |d| {
            (antialiased_disc(d, ra, b) - antialiased_disc(d, ri, b)).max(0.0)
        })?;
        Ok(vec![PlannedKernel { source: 0, image: disc }, PlannedKernel { source: 0, image: annulus }])
    }
}

/// Derived buffers of a SmoothLife world (never serialised).
#[derive(Debug)]
pub struct SmoothLifeEngine {
    plan: ConvolutionPlan,
    width: usize,
    inner: Vec<f32>,
    outer: Vec<f32>,
    growth: Vec<f32>,
}

impl SmoothLifeEngine {
    /// Prepares kernels and buffers.
    pub fn new(
        rule: &SmoothLifeRule,
        width: usize,
        height: usize,
        method: ConvolutionMethod,
    ) -> Result<Self, ContinuousError> {
        rule.validate()?;
        let plan = ConvolutionPlan::new(width, height, 1, rule.kernel_images(width, height)?, method)?;
        let n = width * height;
        Ok(Self { plan, width, inner: vec![0.0; n], outer: vec![0.0; n], growth: vec![0.0; n] })
    }

    /// Convolution plan in use.
    pub fn plan(&self) -> &ConvolutionPlan {
        &self.plan
    }

    /// Inner filling `m` of the last step.
    pub fn inner_filling(&self) -> &[f32] {
        &self.inner
    }

    /// Outer filling `n` of the last step.
    pub fn outer_filling(&self) -> &[f32] {
        &self.outer
    }

    /// `2·s(n, m) − 1` of the last step (the transition in growth form).
    pub fn growth(&self) -> &[f32] {
        &self.growth
    }

    /// Advances the field by one tick and returns the mean of `2s − 1`.
    pub fn step(&mut self, rule: &SmoothLifeRule, field: &mut Grid2<f32>) -> f64 {
        let w = self.width;
        {
            let Self { plan, inner, outer, .. } = self;
            plan.run(
                &[field.as_slice()],
                |k, u| if k == 0 { inner.copy_from_slice(u) } else { outer.copy_from_slice(u) },
            );
        }
        let (inner, outer): (&[f32], &[f32]) = (&self.inner, &self.outer);
        for_row_blocks(&mut self.growth, w, |row0, block| {
            let start = row0 * w;
            for (i, g) in block.iter_mut().enumerate() {
                let s = rule.transition(f64::from(outer[start + i]), f64::from(inner[start + i]));
                *g = (2.0 * s - 1.0) as f32;
            }
        });
        let growth: &[f32] = &self.growth;
        let dt = rule.dt as f32;
        let stepping = rule.stepping;
        for_row_blocks(field.as_mut_slice(), w, |row0, block| {
            let start = row0 * w;
            for (f, &g) in block.iter_mut().zip(&growth[start..]) {
                let s = 0.5 * (g + 1.0);
                let next = match stepping {
                    SmoothLifeStepping::Discrete => s,
                    SmoothLifeStepping::Smooth => *f + dt * g,
                    SmoothLifeStepping::Relax => *f + dt * (s - *f),
                };
                *f = next.clamp(0.0, 1.0);
            }
        });
        field_sum(growth, w) / growth.len().max(1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigmoid_reference_values() {
        assert!((sigma1(0.3, 0.3, 0.028) - 0.5).abs() < 1e-15);
        assert!((sigma1(0.3 + 0.014, 0.3, 0.028) - 0.880_797_1).abs() < 1e-7);
    }

    #[test]
    fn paper_transition_reference_values() {
        // docs/research/continuous-life.md §8 (computed independently).
        let r = SmoothLifeRule::paper_glider();
        let cases = [
            (0.3, 0.0, 0.958_538),
            (0.3, 1.0, 0.991_113),
            (0.4, 1.0, 0.998_388),
            (0.4, 0.0, 0.006_693),
            (0.2, 1.0, 7.0e-5),
            (0.5, 1.0, 3.87e-4),
        ];
        for (n, m, expected) in cases {
            let s = r.transition(n, m);
            // The reference values are rounded to the digits shown.
            assert!((s - expected).abs() < 1e-6, "s({n}, {m}) = {s}, expected {expected}");
        }
    }

    #[test]
    fn indicator_mixing_differs_from_bounds() {
        let mut r = SmoothLifeRule::paper_glider();
        let a = r.transition(0.35, 0.5);
        r.mixing = SmoothLifeMixing::Indicators;
        let b = r.transition(0.35, 0.5);
        assert!((0.0..=1.0).contains(&b));
        assert!((a - b).abs() > 1e-3);
    }

    #[test]
    fn stepping_modes() {
        let mut rule = SmoothLifeRule::paper_glider();
        rule.outer_radius = 6.0;
        let mut engine = SmoothLifeEngine::new(&rule, 32, 32, ConvolutionMethod::Fft).expect("engine");
        // Uniform 0.3: m = n = 0.3 everywhere.
        let mut f = Grid2::new(32, 32, 0.3f32);
        engine.step(&rule, &mut f);
        let s = rule.transition(0.3, 0.3) as f32;
        assert!(f.as_slice().iter().all(|&v| (v - s).abs() < 1e-5));
        for (stepping, expected) in [
            (SmoothLifeStepping::Smooth, (0.3 + 0.1 * (2.0 * s - 1.0)).clamp(0.0, 1.0)),
            (SmoothLifeStepping::Relax, 0.3 + 0.1 * (s - 0.3)),
        ] {
            rule.stepping = stepping;
            let mut f = Grid2::new(32, 32, 0.3f32);
            engine.step(&rule, &mut f);
            assert!(f.as_slice().iter().all(|&v| (v - expected).abs() < 1e-5), "{stepping:?}");
        }
    }

    #[test]
    fn validation() {
        let mut r = SmoothLifeRule::paper_glider();
        r.birth = [0.4, 0.3];
        assert!(r.validate().is_err());
        let mut r = SmoothLifeRule::paper_glider();
        r.radius_ratio = 1.0;
        assert!(r.validate().is_err());
        let r = SmoothLifeRule::paper_glider();
        assert!(SmoothLifeEngine::new(&r, 40, 40, ConvolutionMethod::Auto).is_err(), "r_a = 21 needs 44 cells");
    }
}
