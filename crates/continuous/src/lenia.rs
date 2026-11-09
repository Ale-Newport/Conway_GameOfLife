//! Lenia (Chan 2019) and Expanded Lenia with several channels and kernels
//! (Chan 2020, arXiv:2005.03742 eq. 2).
//!
//! One tick with time step `dt = 1/T`:
//!
//! ```text
//! U_k   = K_k ∗ A_{src(k)}                                  (periodic convolution)
//! D_j   = Σ_{k: dst(k)=j} h_k · G_k(U_k)   [ / Σ_{k: dst(k)=j} h_k  if per-destination ]
//! A_j  ← clip(A_j + dt · D_j, 0, 1)
//! ```
//!
//! Kernel `k` has radius `R·r_k` cells and shell `β_k` with core `K_C`
//! (see [`crate::kernel`]); it is normalised so that `Σ K_k = 1`.

use serde::{Deserialize, Serialize};
use simulation_core::Grid2;

use crate::convolve::{ConvolutionMethod, ConvolutionPlan, PlannedKernel};
use crate::error::ContinuousError;
use crate::growth::GrowthFunction;
use crate::kernel::{KernelCore, KernelImage, lenia_shell};
use crate::parallel::for_row_blocks;
use crate::stats::field_sum;

/// Largest number of channels of a world.
pub const MAX_CHANNELS: usize = 8;
/// Largest number of kernels of a rule.
pub const MAX_KERNELS: usize = 64;
/// Largest number of rings `B` of a kernel shell.
pub const MAX_RINGS: usize = 8;

fn one() -> f64 {
    1.0
}

fn one_channel() -> usize {
    1
}

/// How the weighted growths of the kernels targeting a channel are combined.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrowthNormalization {
    /// `D_j = Σ h_k·G_k` (the Lenia tutorial's multi-channel code).
    #[default]
    Sum,
    /// `D_j = Σ h_k·G_k / Σ h_k` (Chan's `LeniaNDKC.py`, per destination
    /// channel; with all `h_k = 1` this is the mean growth).
    PerDestination,
}

impl GrowthNormalization {
    /// Stable key used in parameters.
    pub fn key(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::PerDestination => "per_destination",
        }
    }

    /// Parses a [`GrowthNormalization::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        [Self::Sum, Self::PerDestination].into_iter().find(|n| n.key() == key)
    }
}

/// One kernel of an Expanded Lenia rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeniaKernel {
    /// Relative radius `r_k`: the kernel spans `R·r_k` cells.
    #[serde(default = "one")]
    pub relative_radius: f64,
    /// Ring peaks `β` (`B = β.len()` rings), each in `[0, 1]`.
    pub beta: Vec<f64>,
    /// Kernel core.
    pub core: KernelCore,
    /// Growth mapping.
    pub growth: GrowthFunction,
    /// Growth centre `μ`.
    pub mu: f64,
    /// Growth width `σ`.
    pub sigma: f64,
    /// Weight `h_k`.
    #[serde(default = "one")]
    pub weight: f64,
    /// Channel the kernel reads.
    #[serde(default)]
    pub source: usize,
    /// Channel the growth is added to.
    #[serde(default)]
    pub destination: usize,
}

impl LeniaKernel {
    /// A single-ring kernel reading and writing channel 0.
    pub fn simple(core: KernelCore, growth: GrowthFunction, mu: f64, sigma: f64) -> Self {
        Self { relative_radius: 1.0, beta: vec![1.0], core, growth, mu, sigma, weight: 1.0, source: 0, destination: 0 }
    }

    fn validate(&self, index: usize, channels: usize) -> Result<(), ContinuousError> {
        let bad = |what: String| Err(ContinuousError::rule(format!("kernel {index}: {what}")));
        if !(self.relative_radius.is_finite() && (0.01..=4.0).contains(&self.relative_radius)) {
            return bad(format!("relative radius {} is outside [0.01, 4]", self.relative_radius));
        }
        validate_beta(&self.beta).or_else(bad)?;
        if !(self.mu.is_finite() && (0.0..=1.0).contains(&self.mu)) {
            return bad(format!("μ = {} is outside [0, 1]", self.mu));
        }
        if !(self.sigma.is_finite() && (0.0001..=1.0).contains(&self.sigma)) {
            return bad(format!("σ = {} is outside [0.0001, 1]", self.sigma));
        }
        if !(self.weight.is_finite() && (0.0..=10.0).contains(&self.weight)) {
            return bad(format!("weight h = {} is outside [0, 10]", self.weight));
        }
        if self.source >= channels || self.destination >= channels {
            return bad(format!(
                "channels {} → {} do not exist (the world has {channels})",
                self.source, self.destination
            ));
        }
        Ok(())
    }
}

/// Validates ring peaks: 1 to [`MAX_RINGS`] values in `[0, 1]`, not all zero.
pub fn validate_beta(beta: &[f64]) -> Result<(), String> {
    if beta.is_empty() || beta.len() > MAX_RINGS {
        return Err(format!("β must have between 1 and {MAX_RINGS} ring peaks"));
    }
    if let Some(b) = beta.iter().find(|b| !(b.is_finite() && (0.0..=1.0).contains(*b))) {
        return Err(format!("ring peak {b} is outside [0, 1]"));
    }
    if beta.iter().all(|&b| b == 0.0) {
        return Err("at least one ring peak must be positive".into());
    }
    Ok(())
}

/// A complete (Expanded) Lenia rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeniaRule {
    /// Kernel radius `R` in cells (space resolution).
    pub radius: f64,
    /// Time resolution `T`; the time step is `dt = 1/T`.
    pub time_resolution: f64,
    /// Number of channels.
    #[serde(default = "one_channel")]
    pub channels: usize,
    /// Combination of the growths reaching one channel.
    #[serde(default)]
    pub normalization: GrowthNormalization,
    /// Kernels.
    pub kernels: Vec<LeniaKernel>,
}

impl LeniaRule {
    /// Orbium unicaudatus in the Lenia tutorial configuration: `R = 13`,
    /// `T = 10`, Gaussian-bump kernel, Gaussian growth `μ = 0.15`, `σ = 0.015`.
    pub fn orbium() -> Self {
        Self {
            radius: 13.0,
            time_resolution: 10.0,
            channels: 1,
            normalization: GrowthNormalization::Sum,
            kernels: vec![LeniaKernel::simple(KernelCore::GaussianBump, GrowthFunction::Gaussian, 0.15, 0.015)],
        }
    }

    /// Time step `1/T`.
    pub fn dt(&self) -> f64 {
        1.0 / self.time_resolution
    }

    /// Checks ranges and channel references.
    pub fn validate(&self) -> Result<(), ContinuousError> {
        if !(self.radius.is_finite() && (0.5..=512.0).contains(&self.radius)) {
            return Err(ContinuousError::rule(format!("kernel radius R = {} is outside [0.5, 512]", self.radius)));
        }
        if !(self.time_resolution.is_finite() && (1.0..=10_000.0).contains(&self.time_resolution)) {
            return Err(ContinuousError::rule(format!(
                "time resolution T = {} is outside [1, 10000]",
                self.time_resolution
            )));
        }
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return Err(ContinuousError::rule(format!("channel count must be between 1 and {MAX_CHANNELS}")));
        }
        if self.kernels.is_empty() || self.kernels.len() > MAX_KERNELS {
            return Err(ContinuousError::rule(format!("kernel count must be between 1 and {MAX_KERNELS}")));
        }
        for (i, k) in self.kernels.iter().enumerate() {
            k.validate(i, self.channels)?;
        }
        Ok(())
    }

    /// Samples every kernel on a `width × height` torus.
    pub fn kernel_images(&self, width: usize, height: usize) -> Result<Vec<PlannedKernel>, ContinuousError> {
        self.kernels
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let r = self.radius * k.relative_radius;
                let image = KernelImage::radial(i, width, height, r, |d| lenia_shell(d / r, &k.beta, k.core))?;
                Ok(PlannedKernel { source: k.source, image })
            })
            .collect()
    }
}

/// Derived buffers of a Lenia world (rebuilt whenever the rule or the size
/// changes; never serialised).
#[derive(Debug)]
pub struct LeniaEngine {
    plan: ConvolutionPlan,
    width: usize,
    growth: Vec<Vec<f32>>,
    potential: Vec<f32>,
}

impl LeniaEngine {
    /// Prepares kernels and buffers for `rule` on a `width × height` world.
    pub fn new(
        rule: &LeniaRule,
        width: usize,
        height: usize,
        method: ConvolutionMethod,
    ) -> Result<Self, ContinuousError> {
        rule.validate()?;
        let plan = ConvolutionPlan::new(width, height, rule.channels, rule.kernel_images(width, height)?, method)?;
        let n = width * height;
        Ok(Self { plan, width, growth: vec![vec![0.0; n]; rule.channels], potential: vec![0.0; n] })
    }

    /// Convolution plan in use.
    pub fn plan(&self) -> &ConvolutionPlan {
        &self.plan
    }

    /// Growth `D_j` of the last step, per channel (after normalisation).
    pub fn growth(&self) -> &[Vec<f32>] {
        &self.growth
    }

    /// Potential `U_0 = K_0 ∗ A_{src(0)}` of the last step.
    pub fn potential(&self) -> &[f32] {
        &self.potential
    }

    /// Advances `fields` by one tick and returns the mean growth `D` over
    /// cells and channels.
    pub fn step(&mut self, rule: &LeniaRule, fields: &mut [Grid2<f32>]) -> f64 {
        let w = self.width;
        let prepared: Vec<_> = rule.kernels.iter().map(|k| k.growth.prepare(k.mu, k.sigma)).collect();
        for g in &mut self.growth {
            g.fill(0.0);
        }
        {
            let Self { plan, growth, potential, .. } = self;
            let slices: Vec<&[f32]> = fields.iter().map(Grid2::as_slice).collect();
            plan.run(&slices, |k, u| {
                if k == 0 {
                    potential.copy_from_slice(u);
                }
                let kernel = &rule.kernels[k];
                let h = kernel.weight as f32;
                let g = prepared[k];
                for_row_blocks(&mut growth[kernel.destination], w, |row0, block| {
                    let start = row0 * w;
                    for (d, &x) in block.iter_mut().zip(&u[start..]) {
                        *d += h * g.eval(x);
                    }
                });
            });
        }
        let dt = rule.dt() as f32;
        let mut total = 0.0;
        for (j, field) in fields.iter_mut().enumerate() {
            let scale = match rule.normalization {
                GrowthNormalization::Sum => 1.0,
                GrowthNormalization::PerDestination => {
                    let h: f64 = rule.kernels.iter().filter(|k| k.destination == j).map(|k| k.weight).sum();
                    if h > 0.0 { (1.0 / h) as f32 } else { 0.0 }
                }
            };
            let growth = &mut self.growth[j];
            if scale != 1.0 {
                for_row_blocks(growth, w, |_, block| block.iter_mut().for_each(|d| *d *= scale));
            }
            let growth: &[f32] = growth;
            for_row_blocks(field.as_mut_slice(), w, |row0, block| {
                let start = row0 * w;
                for (a, &d) in block.iter_mut().zip(&growth[start..]) {
                    *a = (*a + dt * d).clamp(0.0, 1.0);
                }
            });
            total += field_sum(growth, w);
        }
        total / (fields.len() * fields.first().map_or(0, |f| f.as_slice().len())).max(1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_messages() {
        let mut rule = LeniaRule::orbium();
        assert!(rule.validate().is_ok());
        rule.kernels[0].sigma = 0.0;
        assert!(rule.validate().unwrap_err().to_string().contains("σ"));
        let mut rule = LeniaRule::orbium();
        rule.kernels[0].destination = 1;
        assert!(rule.validate().is_err());
        let mut rule = LeniaRule::orbium();
        rule.kernels[0].beta = vec![0.0, 0.0];
        assert!(rule.validate().is_err());
        let mut rule = LeniaRule::orbium();
        rule.time_resolution = 0.5;
        assert!(rule.validate().is_err());
    }

    #[test]
    fn empty_world_decays_and_stays_bounded() {
        let rule = LeniaRule::orbium();
        let mut engine = LeniaEngine::new(&rule, 64, 64, ConvolutionMethod::Fft).expect("engine");
        let mut fields = vec![Grid2::new(64, 64, 0.0f32)];
        let g = engine.step(&rule, &mut fields);
        // U = 0 everywhere, G(0) = 2·exp(−μ²/2σ²) − 1 ≈ −1.
        assert!((g + 1.0).abs() < 1e-6);
        assert_eq!(fields[0].sum(), 0.0);
        let mut full = vec![Grid2::new(64, 64, 1.0f32)];
        engine.step(&rule, &mut full);
        // U = 1 ⇒ G ≈ −1 ⇒ A = 1 − 0.1.
        assert!(full[0].as_slice().iter().all(|&a| (a - 0.9).abs() < 1e-5));
    }

    #[test]
    fn per_destination_normalisation_divides_by_weight_sum() {
        let mut rule = LeniaRule::orbium();
        rule.kernels.push(LeniaKernel { weight: 3.0, ..rule.kernels[0].clone() });
        rule.kernels[0].weight = 1.0;
        let mut fields = vec![Grid2::new(32, 32, 0.0f32)];
        rule.normalization = GrowthNormalization::PerDestination;
        let mut engine = LeniaEngine::new(&rule, 32, 32, ConvolutionMethod::Direct).expect("engine");
        engine.step(&rule, &mut fields);
        // D = (1·G + 3·G)/4 = G(0) ≈ −1.
        assert!(engine.growth()[0].iter().all(|&d| (d + 1.0).abs() < 1e-5));
        rule.normalization = GrowthNormalization::Sum;
        let mut engine = LeniaEngine::new(&rule, 32, 32, ConvolutionMethod::Direct).expect("engine");
        engine.step(&rule, &mut fields);
        assert!(engine.growth()[0].iter().all(|&d| (d + 4.0).abs() < 1e-4));
    }
}
