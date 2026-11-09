//! Flow-Lenia (Plantec et al. 2023, arXiv:2212.07906; 2025,
//! arXiv:2506.08569): mass-conservative Lenia.
//!
//! Each tick, for channels `A_c ≥ 0` (unbounded):
//!
//! ```text
//! U_j   = Σ_{k: dst(k)=j} h_k · G_k(K_k ∗ A_{src(k)})            affinity (eq. 3)
//! A_Σ   = Σ_c A_c
//! α     = clip((A_Σ / θ_A)^n, 0, 1)
//! F_j   = (1 − α)·∇U_j − α·∇A_Σ                                    flow (eq. 5)
//! δ_j   = clip(dt·F_j, −(w − s), w − s)   per axis                 (window w)
//! A_j'(p) = Σ_{p'} A_j(p') · |Ω(p) ∩ square(p' + ½ + δ_j(p'), s)| / (2s)²   reintegration tracking (eqs. 6–7)
//! ```
//!
//! `K_k` are Gaussian-ring kernels `Σ b·exp(−(D − a)²/(2w²))` with
//! `D = |x|/(r_k·R)` masked to `D < 1` (paper eq. 1), `G_k` is the Gaussian
//! growth, and the square distribution has side `2s` ("temperature" `s`).
//!
//! Conventions (see `docs/models/continuous.md`): gradients use the
//! unnormalised 3×3 Sobel operator (≈ 8× the derivative, as in the reference
//! implementation, so published `dt`/`s` values keep their meaning) with
//! periodic boundaries; `α` uses the summed mass `A_Σ` as in the paper. The
//! reintegration step is written as a scatter: every source cell distributes
//! its mass over the at most `⌈2s⌉ + 1` cells per axis that its square
//! overlaps, with per-axis weights computed in source-local coordinates and
//! renormalised to sum to exactly 1, accumulated in `f64`. Mass is therefore
//! conserved up to the final rounding of each cell to `f32`.

use serde::{Deserialize, Serialize};
use simulation_core::Grid2;

use crate::convolve::{ConvolutionMethod, ConvolutionPlan, PlannedKernel};
use crate::error::ContinuousError;
use crate::growth::GrowthFunction;
use crate::kernel::{GaussianRing, KernelImage, gaussian_rings};
use crate::lenia::{MAX_CHANNELS, MAX_KERNELS, MAX_RINGS};
use crate::parallel::for_row_blocks;
use crate::stats::field_sum;

fn one() -> f64 {
    1.0
}

fn one_channel() -> usize {
    1
}

/// Largest number of cells a distribution square can overlap per axis.
const MAX_SPAN: usize = 8;

/// One Flow-Lenia kernel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowKernel {
    /// Relative radius `r`: the kernel spans `r·R` cells.
    #[serde(default = "one")]
    pub relative_radius: f64,
    /// Gaussian rings `(a, b, w)`.
    pub rings: Vec<GaussianRing>,
    /// Growth centre `μ`.
    pub mu: f64,
    /// Growth width `σ`.
    pub sigma: f64,
    /// Weight `h`.
    #[serde(default = "one")]
    pub weight: f64,
    /// Channel read.
    #[serde(default)]
    pub source: usize,
    /// Channel whose affinity receives the growth.
    #[serde(default)]
    pub destination: usize,
}

/// A Flow-Lenia rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowLeniaRule {
    /// Kernel radius `R` in cells.
    pub radius: f64,
    /// Time step `dt` (0.2 in the papers).
    pub dt: f64,
    /// Number of channels.
    #[serde(default = "one_channel")]
    pub channels: usize,
    /// Kernels.
    pub kernels: Vec<FlowKernel>,
    /// Critical mass `θ_A` at which diffusion fully replaces the affinity flow.
    pub critical_mass: f64,
    /// Exponent `n` of `α = (A_Σ/θ_A)^n` (2 in the papers).
    pub alpha_exponent: f64,
    /// Temperature `s`: half the side of the square each cell's mass is
    /// spread over (0.65 in the papers; 0.5 makes a still field an identity).
    pub temperature: f64,
    /// Displacement window `w` in cells (5 in the papers); displacements are
    /// clipped to `±(w − s)` per axis.
    pub window: u32,
}

impl FlowLeniaRule {
    /// Time step.
    pub fn dt(&self) -> f64 {
        self.dt
    }

    /// Largest displacement per axis, `window − temperature`.
    pub fn max_displacement(&self) -> f64 {
        f64::from(self.window) - self.temperature
    }

    /// Checks ranges and channel references.
    pub fn validate(&self) -> Result<(), ContinuousError> {
        let bad = |m: String| Err(ContinuousError::rule(m));
        if !(self.radius.is_finite() && (1.0..=512.0).contains(&self.radius)) {
            return bad(format!("kernel radius R = {} is outside [1, 512]", self.radius));
        }
        if !(self.dt.is_finite() && (0.001..=1.0).contains(&self.dt)) {
            return bad(format!("time step dt = {} is outside [0.001, 1]", self.dt));
        }
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return bad(format!("channel count must be between 1 and {MAX_CHANNELS}"));
        }
        if self.kernels.is_empty() || self.kernels.len() > MAX_KERNELS {
            return bad(format!("kernel count must be between 1 and {MAX_KERNELS}"));
        }
        if !(self.critical_mass.is_finite() && (0.01..=100.0).contains(&self.critical_mass)) {
            return bad(format!("critical mass θ_A = {} is outside [0.01, 100]", self.critical_mass));
        }
        if !(self.alpha_exponent.is_finite() && (0.1..=10.0).contains(&self.alpha_exponent)) {
            return bad(format!("exponent n = {} is outside [0.1, 10]", self.alpha_exponent));
        }
        if !(1..=16).contains(&self.window) {
            return bad(format!("window {} is outside [1, 16]", self.window));
        }
        let s_max = (f64::from(self.window) - 0.05).min((MAX_SPAN - 2) as f64 / 2.0);
        if !(self.temperature.is_finite() && self.temperature >= 0.05 && self.temperature <= s_max) {
            return bad(format!("temperature s = {} is outside [0.05, {s_max}]", self.temperature));
        }
        for (i, k) in self.kernels.iter().enumerate() {
            let pre = format!("kernel {i}");
            if !(k.relative_radius.is_finite() && (0.01..=4.0).contains(&k.relative_radius)) {
                return bad(format!("{pre}: relative radius {} is outside [0.01, 4]", k.relative_radius));
            }
            if k.rings.is_empty() || k.rings.len() > MAX_RINGS {
                return bad(format!("{pre}: between 1 and {MAX_RINGS} rings are required"));
            }
            for g in &k.rings {
                let ok = g.center.is_finite()
                    && (0.0..=1.0).contains(&g.center)
                    && g.height.is_finite()
                    && (0.0..=1.0).contains(&g.height)
                    && g.width.is_finite()
                    && (0.001..=1.0).contains(&g.width);
                if !ok {
                    return bad(format!(
                        "{pre}: ring (a, b, w) = ({}, {}, {}) is out of range",
                        g.center, g.height, g.width
                    ));
                }
            }
            if !(k.mu.is_finite() && (0.0..=1.0).contains(&k.mu)) {
                return bad(format!("{pre}: μ = {} is outside [0, 1]", k.mu));
            }
            if !(k.sigma.is_finite() && (0.0001..=1.0).contains(&k.sigma)) {
                return bad(format!("{pre}: σ = {} is outside [0.0001, 1]", k.sigma));
            }
            if !(k.weight.is_finite() && (0.0..=10.0).contains(&k.weight)) {
                return bad(format!("{pre}: weight h = {} is outside [0, 10]", k.weight));
            }
            if k.source >= self.channels || k.destination >= self.channels {
                return bad(format!("{pre}: channels {} → {} do not exist", k.source, k.destination));
            }
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
                let image = KernelImage::radial(i, width, height, r, |d| gaussian_rings(d / r, &k.rings))?;
                Ok(PlannedKernel { source: k.source, image })
            })
            .collect()
    }
}

/// Per-axis reintegration weights of a square `[c − s, c + s]` with
/// `c = ½ + δ` in source-local coordinates: returns the first destination
/// offset and the normalised overlaps with the cells `[t, t + 1)`.
pub fn axis_weights(delta: f64, s: f64) -> (i64, [f64; MAX_SPAN], usize) {
    let lo = 0.5 + delta - s;
    let hi = 0.5 + delta + s;
    let first = lo.floor() as i64;
    let last = (hi.ceil() as i64 - 1).max(first);
    let mut weights = [0.0; MAX_SPAN];
    let span = ((last - first + 1) as usize).min(MAX_SPAN);
    let mut total = 0.0;
    for (i, w) in weights.iter_mut().enumerate().take(span) {
        let t = (first + i as i64) as f64;
        *w = ((t + 1.0).min(hi) - t.max(lo)).max(0.0);
        total += *w;
    }
    if total > 0.0 {
        weights.iter_mut().take(span).for_each(|w| *w /= total);
    }
    (first, weights, span)
}

/// Unnormalised Sobel gradient with periodic boundaries:
/// `∂x f ≈ Σ_{dy} (1, 2, 1)·(f(x+1, y+dy) − f(x−1, y+dy))`, and likewise for `y`.
fn sobel(field: &[f32], width: usize, height: usize, out: &mut [[f32; 2]]) {
    for_row_blocks(out, width, |row0, block| {
        for (r, orow) in block.chunks_mut(width).enumerate() {
            let y = row0 + r;
            let up = &field[((y + height - 1) % height) * width..][..width];
            let mid = &field[y * width..][..width];
            let down = &field[((y + 1) % height) * width..][..width];
            for (x, g) in orow.iter_mut().enumerate() {
                let (l, rr) = ((x + width - 1) % width, (x + 1) % width);
                let gx = (up[rr] - up[l]) + 2.0 * (mid[rr] - mid[l]) + (down[rr] - down[l]);
                let gy = (down[l] - up[l]) + 2.0 * (down[x] - up[x]) + (down[rr] - up[rr]);
                *g = [gx, gy];
            }
        }
    });
}

/// Derived buffers of a Flow-Lenia world (never serialised).
#[derive(Debug)]
pub struct FlowEngine {
    plan: ConvolutionPlan,
    width: usize,
    height: usize,
    affinity: Vec<Vec<f32>>,
    potential: Vec<f32>,
    total: Vec<f32>,
    grad_total: Vec<[f32; 2]>,
    grad_affinity: Vec<[f32; 2]>,
    displacement: Vec<[f32; 2]>,
    accumulator: Vec<f64>,
}

impl FlowEngine {
    /// Prepares kernels and buffers.
    pub fn new(
        rule: &FlowLeniaRule,
        width: usize,
        height: usize,
        method: ConvolutionMethod,
    ) -> Result<Self, ContinuousError> {
        rule.validate()?;
        let plan = ConvolutionPlan::new(width, height, rule.channels, rule.kernel_images(width, height)?, method)?;
        let n = width * height;
        Ok(Self {
            plan,
            width,
            height,
            affinity: vec![vec![0.0; n]; rule.channels],
            potential: vec![0.0; n],
            total: vec![0.0; n],
            grad_total: vec![[0.0; 2]; n],
            grad_affinity: vec![[0.0; 2]; n],
            displacement: vec![[0.0; 2]; n],
            accumulator: vec![0.0; n],
        })
    }

    /// Convolution plan in use.
    pub fn plan(&self) -> &ConvolutionPlan {
        &self.plan
    }

    /// Affinity `U_j` of the last step, per channel.
    pub fn affinity(&self) -> &[Vec<f32>] {
        &self.affinity
    }

    /// Potential `K_0 ∗ A_{src(0)}` of the last step.
    pub fn potential(&self) -> &[f32] {
        &self.potential
    }

    /// Advances `fields` by one tick; returns the mean affinity over cells
    /// and channels.
    pub fn step(&mut self, rule: &FlowLeniaRule, fields: &mut [Grid2<f32>]) -> f64 {
        let (w, h) = (self.width, self.height);
        let prepared: Vec<_> = rule.kernels.iter().map(|k| GrowthFunction::Gaussian.prepare(k.mu, k.sigma)).collect();
        for a in &mut self.affinity {
            a.fill(0.0);
        }
        {
            let Self { plan, affinity, potential, .. } = self;
            let slices: Vec<&[f32]> = fields.iter().map(Grid2::as_slice).collect();
            plan.run(&slices, |k, u| {
                if k == 0 {
                    potential.copy_from_slice(u);
                }
                let kernel = &rule.kernels[k];
                let hk = kernel.weight as f32;
                let g = prepared[k];
                for_row_blocks(&mut affinity[kernel.destination], w, |row0, block| {
                    let start = row0 * w;
                    for (d, &x) in block.iter_mut().zip(&u[start..]) {
                        *d += hk * g.eval(x);
                    }
                });
            });
        }
        {
            let slices: Vec<&[f32]> = fields.iter().map(Grid2::as_slice).collect();
            for_row_blocks(&mut self.total, w, |row0, block| {
                let start = row0 * w;
                for (i, t) in block.iter_mut().enumerate() {
                    *t = slices.iter().map(|c| c[start + i]).sum();
                }
            });
        }
        sobel(&self.total, w, h, &mut self.grad_total);
        let theta = rule.critical_mass as f32;
        let n_exp = rule.alpha_exponent as f32;
        let dt = rule.dt as f32;
        let max_d = rule.max_displacement() as f32;
        let s = rule.temperature;
        let mut affinity_sum = 0.0;
        for (j, field) in fields.iter_mut().enumerate() {
            sobel(&self.affinity[j], w, h, &mut self.grad_affinity);
            let (total, gt, gu): (&[f32], &[[f32; 2]], &[[f32; 2]]) =
                (&self.total, &self.grad_total, &self.grad_affinity);
            for_row_blocks(&mut self.displacement, w, |row0, block| {
                let start = row0 * w;
                for (i, d) in block.iter_mut().enumerate() {
                    let p = start + i;
                    let alpha = (total[p] / theta).powf(n_exp).clamp(0.0, 1.0);
                    let fx = (1.0 - alpha) * gu[p][0] - alpha * gt[p][0];
                    let fy = (1.0 - alpha) * gu[p][1] - alpha * gt[p][1];
                    *d = [(dt * fx).clamp(-max_d, max_d), (dt * fy).clamp(-max_d, max_d)];
                }
            });
            reintegrate(field.as_slice(), &self.displacement, w, h, s, &mut self.accumulator);
            for (a, &acc) in field.as_mut_slice().iter_mut().zip(&self.accumulator) {
                *a = acc as f32;
            }
            affinity_sum += field_sum(&self.affinity[j], w);
        }
        affinity_sum / (fields.len() * w * h).max(1) as f64
    }
}

/// Reintegration tracking as a scatter: each source cell's mass is spread
/// over the cells overlapped by its displaced square (side `2s`). The
/// accumulation order is fixed (row-major sources), so the result is
/// deterministic.
pub fn reintegrate(mass: &[f32], displacement: &[[f32; 2]], width: usize, height: usize, s: f64, acc: &mut [f64]) {
    acc.fill(0.0);
    let (wi, hi) = (width as i64, height as i64);
    for y in 0..height {
        for x in 0..width {
            let p = y * width + x;
            let m = f64::from(mass[p]);
            if m == 0.0 {
                continue;
            }
            let [dx, dy] = displacement[p];
            let (fx, wx, nx) = axis_weights(f64::from(dx), s);
            let (fy, wy, ny) = axis_weights(f64::from(dy), s);
            for (j, &wyj) in wy.iter().enumerate().take(ny) {
                let ty = (y as i64 + fy + j as i64).rem_euclid(hi) as usize;
                let row = &mut acc[ty * width..(ty + 1) * width];
                let my = m * wyj;
                for (i, &wxi) in wx.iter().enumerate().take(nx) {
                    let tx = (x as i64 + fx + i as i64).rem_euclid(wi) as usize;
                    row[tx] += my * wxi;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::SimRng;

    #[test]
    fn axis_weights_reference() {
        // s = 0.65, no flow: [0.15, 1.0, 0.15] / 1.3 (research note §6.7).
        let (first, w, n) = axis_weights(0.0, 0.65);
        assert_eq!((first, n), (-1, 3));
        for (got, want) in w.iter().zip([0.115_384_6, 0.769_230_8, 0.115_384_6]) {
            assert!((got - want).abs() < 1e-7);
        }
        // s = 0.5, no flow: identity.
        let (first, w, n) = axis_weights(0.0, 0.5);
        assert_eq!((first, n, w[0]), (0, 1, 1.0));
        // Any displacement: weights sum to one.
        for d in [-4.35, -1.3, 0.2, 2.75] {
            let (_, w, n) = axis_weights(d, 0.65);
            assert!((w[..n].iter().sum::<f64>() - 1.0).abs() < 1e-15);
        }
    }

    #[test]
    fn reintegration_identity_and_conservation() {
        let (w, h) = (17, 13);
        let mut rng = SimRng::seed_from_u64(4);
        let mass: Vec<f32> = (0..w * h).map(|_| rng.f32() * 2.0).collect();
        let still = vec![[0.0f32; 2]; w * h];
        let mut acc = vec![0.0; w * h];
        reintegrate(&mass, &still, w, h, 0.5, &mut acc);
        assert!(acc.iter().zip(&mass).all(|(a, &m)| (*a - f64::from(m)).abs() < 1e-12));
        let flow: Vec<[f32; 2]> =
            (0..w * h).map(|_| [rng.range_f32(-4.35, 4.35), rng.range_f32(-4.35, 4.35)]).collect();
        reintegrate(&mass, &flow, w, h, 0.65, &mut acc);
        let before: f64 = mass.iter().map(|&m| f64::from(m)).sum();
        let after: f64 = acc.iter().sum();
        assert!(((after - before) / before).abs() < 1e-12);
        assert!(acc.iter().all(|&a| a >= 0.0));
    }

    #[test]
    fn uniform_translation_moves_mass() {
        let (w, h) = (16, 16);
        let mut mass = vec![0.0f32; w * h];
        mass[5 * w + 5] = 1.0;
        let flow = vec![[2.0f32, -1.0]; w * h];
        let mut acc = vec![0.0; w * h];
        reintegrate(&mass, &flow, w, h, 0.5, &mut acc);
        assert!((acc[4 * w + 7] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn sobel_of_a_ramp() {
        let (w, h) = (8, 6);
        // f = x on interior columns: unnormalised Sobel gives 8·∂x.
        let f: Vec<f32> = (0..w * h).map(|i| (i % w) as f32).collect();
        let mut g = vec![[0.0f32; 2]; w * h];
        sobel(&f, w, h, &mut g);
        assert_eq!(g[2 * w + 3], [8.0, 0.0]);
        // Periodic wrap at column 0: neighbours are columns 7 and 1.
        assert_eq!(g[2 * w][0], 4.0 * (1.0 - 7.0));
    }
}
