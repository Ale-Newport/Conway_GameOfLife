//! Periodic convolution of fields with many kernels, by FFT or directly.
//!
//! A [`ConvolutionPlan`] owns the kernels of one rule on one grid size:
//!
//! * **FFT path** — each *source channel* is transformed once; then, for
//!   every pair of kernels, the two spectral products are combined as
//!   `P_a + i·P_b` and inverted together. Because both potentials are real,
//!   the real part of the inverse is `K_a ∗ A` and the imaginary part is
//!   `K_b ∗ A` (exact by linearity), so `K` kernels cost `C + ⌈K/2⌉` 2-D
//!   transforms. Kernel spectra are precomputed with the `1/(W·H)` scale
//!   folded in. Every kernel of this crate is sampled from a function of the
//!   distance only, so it is exactly point-symmetric (`K(n) = K(−n)`) and its
//!   DFT is real: only the real part of each spectrum is stored (the
//!   imaginary part is rounding noise), which halves the spectrum memory and
//!   the work of the pointwise products.
//! * **Direct path** — `U(x) = Σ_n K(n)·A(x + n)` over the kernel's non-zero
//!   taps, using a horizontally wrap-padded copy of the field so the inner
//!   loop is a contiguous multiply-add. Used for tiny kernels and as the
//!   oracle the FFT path is tested against. (All kernels in this crate are
//!   radially symmetric, so correlation and convolution coincide.)

use rustfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};

use crate::error::ContinuousError;
use crate::fft::Fft2;
use crate::kernel::{KernelImage, Tap};
use crate::parallel::for_row_blocks;

/// How convolutions are computed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvolutionMethod {
    /// FFT unless the kernels are so small that direct summation is cheaper
    /// (see [`ConvolutionMethod::resolve`]).
    #[default]
    Auto,
    /// Always FFT.
    Fft,
    /// Always direct summation over kernel taps.
    Direct,
}

impl ConvolutionMethod {
    /// Every method, in display order.
    pub const ALL: [ConvolutionMethod; 3] = [Self::Auto, Self::Fft, Self::Direct];

    /// Stable key used in parameters.
    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Fft => "fft",
            Self::Direct => "direct",
        }
    }

    /// Parses a [`ConvolutionMethod::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }

    /// Chooses FFT or direct for the given work. The cost model counts one
    /// multiply-add per tap and cell for direct summation, and
    /// `DIRECT_TAPS_PER_FFT · log2(W·H)` per cell for each 2-D transform; the
    /// constant was calibrated with the `continuous` Criterion benchmark.
    pub fn resolve(self, width: usize, height: usize, total_taps: usize, transforms: usize) -> ResolvedMethod {
        match self {
            Self::Fft => ResolvedMethod::Fft,
            Self::Direct => ResolvedMethod::Direct,
            Self::Auto => {
                let log_cells = ((width * height).max(2) as f64).log2();
                let fft_cost = transforms as f64 * DIRECT_TAPS_PER_FFT * log_cells;
                if (total_taps as f64) < fft_cost { ResolvedMethod::Direct } else { ResolvedMethod::Fft }
            }
        }
    }
}

/// Direct-summation taps that cost as much as one 2-D transform per
/// `log2(cells)` (calibration constant of [`ConvolutionMethod::Auto`]).
/// Measured at 256² (serial): one FFT convolution (two transforms) costs
/// 0.47 ms whatever the radius, direct summation 0.095 ms with 9 taps,
/// 0.55 ms with 109 taps and 2.5 ms with 517 taps — about 49 taps per
/// transform, i.e. 3·log2(256²).
pub const DIRECT_TAPS_PER_FFT: f64 = 3.0;

/// Largest memory, in bytes, that one convolution plan may use on the FFT
/// path (kernel spectra, channel spectra and work buffers). Larger worlds or
/// rules are refused with [`ContinuousError::LimitExceeded`] instead of
/// exhausting memory.
pub const MAX_PLAN_BYTES: u64 = 4 << 30;

/// Largest number of multiply-adds per tick accepted on the direct path
/// (`taps × cells`; roughly a second per tick at the serial rate measured by
/// the benchmark, about 1.5·10¹⁰ multiply-adds per second). Larger direct
/// convolutions must use the FFT.
pub const MAX_DIRECT_WORK: u64 = 10_000_000_000;

/// The method a plan actually uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedMethod {
    /// FFT convolution.
    Fft,
    /// Direct summation.
    Direct,
}

/// A kernel together with the channel it reads.
#[derive(Clone, Debug)]
pub struct PlannedKernel {
    /// Source channel index.
    pub source: usize,
    /// Sampled, normalised kernel.
    pub image: KernelImage,
}

/// Kernels prepared for repeated convolution on one grid size.
#[derive(Debug)]
pub struct ConvolutionPlan {
    width: usize,
    height: usize,
    channels: usize,
    method: ResolvedMethod,
    kernels: Vec<PlannedKernel>,
    fft: Option<FftState>,
    out_a: Vec<f32>,
    out_b: Vec<f32>,
    padded: Vec<f32>,
}

#[derive(Debug)]
struct FftState {
    fft: Fft2,
    /// Real parts of the pre-scaled kernel spectra (spectral layout); the
    /// imaginary parts vanish because every kernel is point-symmetric.
    spectra: Vec<Vec<f32>>,
    /// Spectrum of each channel; empty for channels no kernel reads.
    channel_spectra: Vec<Vec<Complex32>>,
    work: Vec<Complex32>,
    product: Vec<Complex32>,
}

impl ConvolutionPlan {
    /// Checks `kernels` against the grid, resolves `method` and checks the
    /// resource limits ([`MAX_PLAN_BYTES`], [`MAX_DIRECT_WORK`]) without
    /// allocating anything world-sized. Returns the method a plan would use.
    pub fn check(
        width: usize,
        height: usize,
        channels: usize,
        kernels: &[PlannedKernel],
        method: ConvolutionMethod,
    ) -> Result<ResolvedMethod, ContinuousError> {
        for (i, k) in kernels.iter().enumerate() {
            if k.image.width() != width || k.image.height() != height {
                return Err(ContinuousError::rule(format!("kernel {i} was sampled for a different grid size")));
            }
            if k.source >= channels {
                return Err(ContinuousError::rule(format!(
                    "kernel {i} reads channel {} but the world has {channels} channel(s)",
                    k.source
                )));
            }
        }
        let sources = source_count(kernels, channels);
        let transforms = sources + kernels.len().div_ceil(2);
        let total_taps: usize = kernels.iter().map(|k| k.image.taps().len()).sum();
        let resolved = method.resolve(width, height, total_taps, transforms);
        let cells = (width * height) as u64;
        match resolved {
            ResolvedMethod::Fft => {
                let bytes = fft_bytes(cells, kernels.len() as u64, sources as u64);
                if bytes > MAX_PLAN_BYTES {
                    return Err(ContinuousError::LimitExceeded(format!(
                        "FFT convolution of {} kernel(s) on a {width}×{height} world needs {:.1} GiB \
                         (limit {:.0} GiB); use a smaller world or fewer kernels",
                        kernels.len(),
                        bytes as f64 / f64::from(1u32 << 30),
                        MAX_PLAN_BYTES as f64 / f64::from(1u32 << 30),
                    )));
                }
            }
            ResolvedMethod::Direct => {
                let work = total_taps as u64 * cells;
                if work > MAX_DIRECT_WORK {
                    return Err(ContinuousError::LimitExceeded(format!(
                        "direct summation of {total_taps} kernel taps on a {width}×{height} world needs \
                         {:.2e} multiply-adds per tick (limit {:.0e}); use the FFT method",
                        work as f64, MAX_DIRECT_WORK as f64,
                    )));
                }
            }
        }
        Ok(resolved)
    }

    /// Prepares `kernels` (all sampled on the same `width × height` grid)
    /// for fields with `channels` channels. Fails with the errors of
    /// [`ConvolutionPlan::check`].
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        kernels: Vec<PlannedKernel>,
        method: ConvolutionMethod,
    ) -> Result<Self, ContinuousError> {
        let resolved = Self::check(width, height, channels, &kernels, method)?;
        let n = width * height;
        let fft = match resolved {
            ResolvedMethod::Fft => Some(FftState::new(width, height, channels, &kernels)),
            ResolvedMethod::Direct => None,
        };
        Ok(Self {
            width,
            height,
            channels,
            method: resolved,
            kernels,
            fft,
            out_a: vec![0.0; n],
            out_b: vec![0.0; n],
            padded: Vec::new(),
        })
    }

    /// Method in use.
    pub fn method(&self) -> ResolvedMethod {
        self.method
    }

    /// Number of kernels.
    pub fn kernel_count(&self) -> usize {
        self.kernels.len()
    }

    /// Bytes held by the plan's buffers (spectra, work and output buffers).
    pub fn memory_bytes(&self) -> usize {
        let n = self.width * self.height;
        let outputs = (self.out_a.len() + self.out_b.len() + self.padded.len()) * 4;
        let taps: usize = self.kernels.iter().map(|k| std::mem::size_of_val(k.image.taps())).sum();
        let fft = self.fft.as_ref().map_or(0, |state| {
            let spectra: usize = state.spectra.iter().map(Vec::len).sum::<usize>() * 4;
            let channels: usize = state.channel_spectra.iter().map(Vec::len).sum::<usize>() * 8;
            spectra + channels + 2 * n * 8
        });
        outputs + taps + fft
    }

    /// Convolves every kernel with its source channel and calls
    /// `sink(kernel_index, potential)` in kernel order. `fields` holds one
    /// row-major `width × height` buffer per channel.
    pub fn run<F: AsRef<[f32]>>(&mut self, fields: &[F], mut sink: impl FnMut(usize, &[f32])) {
        debug_assert_eq!(fields.len(), self.channels);
        let (w, h) = (self.width, self.height);
        match &mut self.fft {
            Some(state) => {
                for (c, spectrum) in state.channel_spectra.iter_mut().enumerate() {
                    if !spectrum.is_empty() {
                        state.fft.forward_real(fields[c].as_ref(), &mut state.work, spectrum);
                    }
                }
                let mut k = 0;
                while k < self.kernels.len() {
                    let paired = k + 1 < self.kernels.len();
                    state.multiply(&self.kernels, k, paired);
                    state.fft.inverse(&mut state.product, &mut state.work);
                    split_complex(&state.work, &mut self.out_a, &mut self.out_b, w);
                    sink(k, &self.out_a);
                    if paired {
                        sink(k + 1, &self.out_b);
                    }
                    k += 2;
                }
            }
            None => {
                for (k, kernel) in self.kernels.iter().enumerate() {
                    convolve_direct(
                        fields[kernel.source].as_ref(),
                        w,
                        h,
                        kernel.image.taps(),
                        &mut self.padded,
                        &mut self.out_a,
                    );
                    sink(k, &self.out_a);
                }
            }
        }
    }
}

/// Which channels some kernel reads.
fn source_mask(kernels: &[PlannedKernel], channels: usize) -> Vec<bool> {
    let mut sources = vec![false; channels];
    for k in kernels {
        if let Some(s) = sources.get_mut(k.source) {
            *s = true;
        }
    }
    sources
}

/// Number of distinct channels read by `kernels`.
fn source_count(kernels: &[PlannedKernel], channels: usize) -> usize {
    source_mask(kernels, channels).iter().filter(|&&s| s).count()
}

/// Bytes of an FFT plan: real kernel spectra (4 bytes per bin), complex
/// channel spectra, two complex work buffers and two real output buffers.
fn fft_bytes(cells: u64, kernels: u64, sources: u64) -> u64 {
    cells * (4 * kernels + 8 * sources + 2 * 8 + 2 * 4)
}

impl FftState {
    fn new(width: usize, height: usize, channels: usize, kernels: &[PlannedKernel]) -> Self {
        let mut fft = Fft2::new(width, height);
        let n = width * height;
        let scale = 1.0 / n as f32;
        let mut work = vec![Complex32::default(); n];
        let mut product = vec![Complex32::default(); n];
        let mut dense = vec![0.0f32; n];
        let spectra = kernels
            .iter()
            .map(|k| {
                k.image.write_dense(&mut dense);
                fft.forward_real(&dense, &mut work, &mut product);
                product.iter().map(|v| v.re * scale).collect()
            })
            .collect();
        let channel_spectra = source_mask(kernels, channels)
            .iter()
            .map(|&s| if s { vec![Complex32::default(); n] } else { Vec::new() })
            .collect();
        Self { fft, spectra, channel_spectra, work, product }
    }

    /// `product = S_src(a)·K_a (+ i·S_src(b)·K_b when paired)`.
    fn multiply(&mut self, kernels: &[PlannedKernel], a: usize, paired: bool) {
        let spec_a = &self.channel_spectra[kernels[a].source];
        let k_a = &self.spectra[a];
        let h = self.fft.height();
        if paired {
            let spec_b = &self.channel_spectra[kernels[a + 1].source];
            let k_b = &self.spectra[a + 1];
            for_row_blocks(&mut self.product, h, |row0, block| {
                let start = row0 * h;
                for (i, p) in block.iter_mut().enumerate() {
                    let j = start + i;
                    let pa = spec_a[j] * k_a[j];
                    let pb = spec_b[j] * k_b[j];
                    // pa + i·pb
                    *p = Complex32::new(pa.re - pb.im, pa.im + pb.re);
                }
            });
        } else {
            for_row_blocks(&mut self.product, h, |row0, block| {
                let start = row0 * h;
                for (i, p) in block.iter_mut().enumerate() {
                    *p = spec_a[start + i] * k_a[start + i];
                }
            });
        }
    }
}

/// Splits a natural-layout complex buffer into its real and imaginary parts.
fn split_complex(src: &[Complex32], re: &mut [f32], im: &mut [f32], width: usize) {
    for_row_blocks(re, width, |row0, block| {
        let start = row0 * width;
        for (o, c) in block.iter_mut().zip(&src[start..]) {
            *o = c.re;
        }
    });
    for_row_blocks(im, width, |row0, block| {
        let start = row0 * width;
        for (o, c) in block.iter_mut().zip(&src[start..]) {
            *o = c.im;
        }
    });
}

/// Direct periodic convolution `out(x) = Σ_taps w·input(x + (dx, dy))` on a
/// `width × height` torus. `padded` is scratch reused between calls.
pub fn convolve_direct(
    input: &[f32],
    width: usize,
    height: usize,
    taps: &[Tap],
    padded: &mut Vec<f32>,
    out: &mut [f32],
) {
    debug_assert_eq!(input.len(), width * height);
    debug_assert_eq!(out.len(), width * height);
    let pad = taps.iter().map(|t| t.dx.unsigned_abs() as usize).max().unwrap_or(0);
    let pw = width + 2 * pad;
    padded.clear();
    padded.resize(pw * height, 0.0);
    for (y, row) in input.chunks(width).enumerate() {
        let dst = &mut padded[y * pw..(y + 1) * pw];
        for (i, v) in dst.iter_mut().enumerate() {
            let x = (i as i64 - pad as i64).rem_euclid(width as i64) as usize;
            *v = row[x];
        }
    }
    // Group taps by row offset (taps are sorted by dy, then dx).
    let mut rows: Vec<(i32, &[Tap])> = Vec::new();
    let mut start = 0;
    while start < taps.len() {
        let dy = taps[start].dy;
        let end = start + taps[start..].iter().take_while(|t| t.dy == dy).count();
        rows.push((dy, &taps[start..end]));
        start = end;
    }
    let padded: &[f32] = padded;
    for_row_blocks(out, width, |row0, block| {
        for (r, orow) in block.chunks_mut(width).enumerate() {
            let y = row0 + r;
            orow.fill(0.0);
            for &(dy, row_taps) in &rows {
                let sy = (y as i64 + i64::from(dy)).rem_euclid(height as i64) as usize;
                let src = &padded[sy * pw..(sy + 1) * pw];
                for t in row_taps {
                    let off = (pad as i64 + i64::from(t.dx)) as usize;
                    let w = t.weight;
                    for (o, s) in orow.iter_mut().zip(&src[off..off + width]) {
                        *o += w * s;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{KernelCore, lenia_shell};
    use simulation_core::SimRng;

    fn random_field(w: usize, h: usize, seed: u64) -> Vec<f32> {
        let mut rng = SimRng::seed_from_u64(seed);
        (0..w * h).map(|_| rng.f32()).collect()
    }

    fn shell_kernel(w: usize, h: usize, radius: f64, beta: &[f64], core: KernelCore) -> KernelImage {
        KernelImage::radial(0, w, h, radius, |d| lenia_shell(d / radius, beta, core)).expect("kernel")
    }

    fn run_all(plan: &mut ConvolutionPlan, fields: &[Vec<f32>]) -> Vec<Vec<f32>> {
        let mut out = vec![Vec::new(); plan.kernel_count()];
        plan.run(fields, |k, u| out[k] = u.to_vec());
        out
    }

    #[test]
    fn fft_matches_direct_on_random_fields() {
        // Odd, even and non-power-of-two sizes; three kernels (one pair + one
        // single inverse) reading two channels.
        for &(w, h) in &[(64, 64), (90, 54), (37, 41)] {
            let kernels = vec![
                PlannedKernel { source: 0, image: shell_kernel(w, h, 13.0, &[1.0], KernelCore::Exponential) },
                PlannedKernel { source: 1, image: shell_kernel(w, h, 9.5, &[0.5, 1.0], KernelCore::Polynomial) },
                PlannedKernel { source: 0, image: shell_kernel(w, h, 4.0, &[1.0], KernelCore::GaussianBump) },
            ];
            let fields = vec![random_field(w, h, 1), random_field(w, h, 2)];
            let mut fft = ConvolutionPlan::new(w, h, 2, kernels.clone(), ConvolutionMethod::Fft).expect("plan");
            let mut direct = ConvolutionPlan::new(w, h, 2, kernels, ConvolutionMethod::Direct).expect("plan");
            assert_eq!(fft.method(), ResolvedMethod::Fft);
            assert_eq!(direct.method(), ResolvedMethod::Direct);
            let a = run_all(&mut fft, &fields);
            let b = run_all(&mut direct, &fields);
            for (ua, ub) in a.iter().zip(&b) {
                let max = ua.iter().zip(ub).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
                assert!(max < 1e-4, "{w}x{h}: max |fft − direct| = {max}");
            }
        }
    }

    #[test]
    fn constant_field_gives_constant_potential() {
        let (w, h) = (48, 40);
        let kernels =
            vec![PlannedKernel { source: 0, image: shell_kernel(w, h, 10.0, &[1.0], KernelCore::Exponential) }];
        let fields = vec![vec![0.7f32; w * h]];
        let mut plan = ConvolutionPlan::new(w, h, 1, kernels, ConvolutionMethod::Fft).expect("plan");
        let u = run_all(&mut plan, &fields);
        assert!(u[0].iter().all(|&v| (v - 0.7).abs() < 1e-5));
    }

    #[test]
    fn parallel_fft_path_is_bitwise_deterministic() {
        // 512 × 256 cells is above the parallel threshold.
        let (w, h) = (512, 256);
        let make = || {
            vec![
                PlannedKernel { source: 0, image: shell_kernel(w, h, 13.0, &[1.0], KernelCore::Exponential) },
                PlannedKernel { source: 0, image: shell_kernel(w, h, 7.0, &[1.0], KernelCore::Polynomial) },
            ]
        };
        let fields = vec![random_field(w, h, 9)];
        let mut plan = ConvolutionPlan::new(w, h, 1, make(), ConvolutionMethod::Fft).expect("plan");
        let a = run_all(&mut plan, &fields);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("pool");
        let b = pool.install(|| {
            let mut plan = ConvolutionPlan::new(w, h, 1, make(), ConvolutionMethod::Fft).expect("plan");
            run_all(&mut plan, &fields)
        });
        assert_eq!(a, b);
    }

    #[test]
    fn kernel_spectra_are_real() {
        // The plan keeps only real parts; check that the dropped imaginary
        // parts are rounding noise for odd, even and non-square grids.
        for &(w, h) in &[(64, 64), (90, 54), (37, 41)] {
            let image = shell_kernel(w, h, 9.5, &[0.5, 1.0], KernelCore::Polynomial);
            let mut fft = Fft2::new(w, h);
            let mut work = vec![Complex32::default(); w * h];
            let mut spectrum = vec![Complex32::default(); w * h];
            fft.forward_real(&image.dense(), &mut work, &mut spectrum);
            let max_re = spectrum.iter().map(|c| c.re.abs()).fold(0.0f32, f32::max);
            let max_im = spectrum.iter().map(|c| c.im.abs()).fold(0.0f32, f32::max);
            assert!((spectrum[0].re - 1.0).abs() < 1e-6, "DC bin = Σ K = {}", spectrum[0].re);
            assert!(max_im < 1e-6 * max_re, "{w}x{h}: max |Im| = {max_im}");
        }
    }

    #[test]
    fn resource_limits_are_checked_without_world_sized_allocations() {
        let side = 4096;
        let orbium = shell_kernel(side, side, 13.0, &[1.0], KernelCore::GaussianBump);
        let many: Vec<_> = (0..64).map(|_| PlannedKernel { source: 0, image: orbium.clone() }).collect();
        assert!(matches!(
            ConvolutionPlan::check(side, side, 1, &many, ConvolutionMethod::Fft),
            Err(ContinuousError::LimitExceeded(_))
        ));
        // One kernel fits easily (≈ 0.5 GiB).
        assert_eq!(ConvolutionPlan::check(side, side, 1, &many[..1], ConvolutionMethod::Fft), Ok(ResolvedMethod::Fft));
        // Forced direct summation: 3 × 517 taps × 4096² ≈ 2.6·10¹⁰ multiply-adds.
        let err = ConvolutionPlan::check(side, side, 1, &many[..3], ConvolutionMethod::Direct);
        assert!(matches!(&err, Err(ContinuousError::LimitExceeded(m)) if m.contains("FFT")), "{err:?}");
        // Auto picks the FFT there instead.
        assert_eq!(ConvolutionPlan::check(side, side, 1, &many[..3], ConvolutionMethod::Auto), Ok(ResolvedMethod::Fft));
    }

    #[test]
    fn auto_prefers_direct_for_tiny_kernels() {
        assert_eq!(ConvolutionMethod::Auto.resolve(256, 256, 5, 2), ResolvedMethod::Direct);
        assert_eq!(ConvolutionMethod::Auto.resolve(256, 256, 517, 2), ResolvedMethod::Fft);
        let k = vec![PlannedKernel { source: 3, image: shell_kernel(32, 32, 3.0, &[1.0], KernelCore::Polynomial) }];
        assert!(ConvolutionPlan::new(32, 32, 1, k, ConvolutionMethod::Auto).is_err());
    }
}
