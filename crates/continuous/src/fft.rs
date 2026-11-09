//! Two-dimensional complex FFT on a periodic grid built from `rustfft`
//! one-dimensional transforms.
//!
//! Layout conventions:
//!
//! * **Natural layout**: row-major, `height` rows of `width` values (the
//!   layout of every field in this crate).
//! * **Spectral layout**: transposed, `width` rows of `height` bins. The
//!   forward transform runs the row FFTs, transposes, and runs the column FFTs
//!   as contiguous rows; the inverse undoes the steps in reverse order. Kernel
//!   spectra are stored in the same transposed layout, so convolution never
//!   needs to transpose back before the pointwise product.
//!
//! The inverse transform is unnormalised (`rustfft` convention); callers fold
//! the `1 / (width·height)` factor into their precomputed kernel spectra.
//!
//! Rows are processed in blocks through
//! [`crate::parallel::for_row_blocks_with`], each block with its own FFT
//! scratch buffer from a pool that lives as long as the plan (allocated once),
//! and transposes are tiled. Every row is transformed by the same plan
//! whatever the block size, so results are bitwise identical for any number of
//! threads.

use std::fmt;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use crate::parallel::for_row_blocks_with;

/// Number of destination rows written per transpose task.
const TRANSPOSE_TILE: usize = 32;

/// Precomputed plans for 2-D transforms of one grid size.
#[derive(Clone)]
pub struct Fft2 {
    width: usize,
    height: usize,
    row_forward: Arc<dyn Fft<f32>>,
    row_inverse: Arc<dyn Fft<f32>>,
    col_forward: Arc<dyn Fft<f32>>,
    col_inverse: Arc<dyn Fft<f32>>,
    row_scratch: Vec<Vec<Complex32>>,
    col_scratch: Vec<Vec<Complex32>>,
}

impl fmt::Debug for Fft2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fft2").field("width", &self.width).field("height", &self.height).finish()
    }
}

impl Fft2 {
    /// Plans transforms for a `width × height` grid (both at least 1).
    pub fn new(width: usize, height: usize) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        Self {
            width,
            height,
            row_forward: planner.plan_fft_forward(width),
            row_inverse: planner.plan_fft_inverse(width),
            col_forward: planner.plan_fft_forward(height),
            col_inverse: planner.plan_fft_inverse(height),
            row_scratch: Vec::new(),
            col_scratch: Vec::new(),
        }
    }

    /// Grid width.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Grid height.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Number of complex values in a buffer of this grid.
    pub fn len(&self) -> usize {
        self.width * self.height
    }

    /// Whether the grid has no cells.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forward DFT of the real field `input` (natural layout) into
    /// `spectrum` (spectral layout). `work` is natural-layout scratch.
    pub fn forward_real(&mut self, input: &[f32], work: &mut [Complex32], spectrum: &mut [Complex32]) {
        debug_assert_eq!(input.len(), self.len());
        let w = self.width;
        let row_fft = self.row_forward.as_ref();
        let len = scratch_len(&self.row_forward, &self.row_inverse);
        for_row_blocks_with(
            work,
            w,
            &mut self.row_scratch,
            || vec![Complex32::default(); len],
            |row0, block, scratch| {
                let start = row0 * w;
                for (c, &v) in block.iter_mut().zip(&input[start..]) {
                    *c = Complex32::new(v, 0.0);
                }
                run_fft(row_fft, block, scratch);
            },
        );
        transpose(work, self.height, w, spectrum);
        let col_fft = self.col_forward.as_ref();
        let len = scratch_len(&self.col_forward, &self.col_inverse);
        for_row_blocks_with(
            spectrum,
            self.height,
            &mut self.col_scratch,
            || vec![Complex32::default(); len],
            |_, block, scratch| {
                run_fft(col_fft, block, scratch);
            },
        );
    }

    /// Unnormalised inverse DFT. `spectrum` (spectral layout) is overwritten
    /// with intermediate results; the output is written to `out` in natural
    /// layout.
    pub fn inverse(&mut self, spectrum: &mut [Complex32], out: &mut [Complex32]) {
        debug_assert_eq!(spectrum.len(), self.len());
        let col_fft = self.col_inverse.as_ref();
        let len = scratch_len(&self.col_forward, &self.col_inverse);
        for_row_blocks_with(
            spectrum,
            self.height,
            &mut self.col_scratch,
            || vec![Complex32::default(); len],
            |_, block, scratch| {
                run_fft(col_fft, block, scratch);
            },
        );
        transpose(spectrum, self.width, self.height, out);
        let row_fft = self.row_inverse.as_ref();
        let len = scratch_len(&self.row_forward, &self.row_inverse);
        for_row_blocks_with(
            out,
            self.width,
            &mut self.row_scratch,
            || vec![Complex32::default(); len],
            |_, block, scratch| {
                run_fft(row_fft, block, scratch);
            },
        );
    }
}

/// Scratch length that serves both directions of one axis.
fn scratch_len(forward: &Arc<dyn Fft<f32>>, inverse: &Arc<dyn Fft<f32>>) -> usize {
    forward.get_inplace_scratch_len().max(inverse.get_inplace_scratch_len())
}

/// Transforms every FFT-length chunk of `block` in place.
fn run_fft(fft: &dyn Fft<f32>, block: &mut [Complex32], scratch: &mut [Complex32]) {
    let needed = fft.get_inplace_scratch_len();
    fft.process_with_scratch(block, &mut scratch[..needed]);
}

/// Writes the transpose of `src` (`rows × cols`, row-major) into `dst`
/// (`cols × rows`, row-major), in tiles of 32 destination rows.
pub fn transpose<T: Copy + Send + Sync>(src: &[T], rows: usize, cols: usize, dst: &mut [T]) {
    debug_assert_eq!(src.len(), rows * cols);
    debug_assert_eq!(dst.len(), rows * cols);
    if rows == 0 || cols == 0 {
        return;
    }
    // Each destination "row" is a source column; tiles of destination rows
    // read short contiguous runs from every source row.
    let tile_len = TRANSPOSE_TILE * rows;
    let tiles = |tile_index: usize, tile: &mut [T]| {
        let c0 = tile_index * TRANSPOSE_TILE;
        let ncols = tile.len() / rows;
        for r in 0..rows {
            let src_row = &src[r * cols + c0..r * cols + c0 + ncols];
            for (j, &v) in src_row.iter().enumerate() {
                tile[j * rows + r] = v;
            }
        }
    };
    let tile_count = cols.div_ceil(TRANSPOSE_TILE);
    if tile_count == 1 || rows * cols < crate::parallel::PARALLEL_MIN_CELLS {
        for (i, tile) in dst.chunks_mut(tile_len).enumerate() {
            tiles(i, tile);
        }
    } else {
        use rayon::prelude::*;
        dst.par_chunks_mut(tile_len).enumerate().for_each(|(i, tile)| tiles(i, tile));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_dft(input: &[f32], w: usize, h: usize) -> Vec<Complex32> {
        let mut out = vec![Complex32::default(); w * h];
        for ky in 0..h {
            for kx in 0..w {
                let mut acc = (0.0f64, 0.0f64);
                for y in 0..h {
                    for x in 0..w {
                        let phase = -std::f64::consts::TAU * ((kx * x) as f64 / w as f64 + (ky * y) as f64 / h as f64);
                        let v = f64::from(input[y * w + x]);
                        acc.0 += v * phase.cos();
                        acc.1 += v * phase.sin();
                    }
                }
                // spectral layout: row kx, column ky
                out[kx * h + ky] = Complex32::new(acc.0 as f32, acc.1 as f32);
            }
        }
        out
    }

    #[test]
    fn forward_matches_naive_dft_in_transposed_layout() {
        let (w, h) = (6, 5);
        let input: Vec<f32> = (0..w * h).map(|i| ((i * 7919) % 13) as f32 / 13.0).collect();
        let mut fft = Fft2::new(w, h);
        let mut work = vec![Complex32::default(); w * h];
        let mut spec = vec![Complex32::default(); w * h];
        fft.forward_real(&input, &mut work, &mut spec);
        let expected = naive_dft(&input, w, h);
        for (a, b) in spec.iter().zip(&expected) {
            assert!((a - b).norm() < 1e-4, "{a} vs {b}");
        }
    }

    #[test]
    fn inverse_round_trip() {
        let (w, h) = (16, 12);
        let input: Vec<f32> = (0..w * h).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut fft = Fft2::new(w, h);
        let mut work = vec![Complex32::default(); w * h];
        let mut spec = vec![Complex32::default(); w * h];
        fft.forward_real(&input, &mut work, &mut spec);
        let mut out = vec![Complex32::default(); w * h];
        fft.inverse(&mut spec, &mut out);
        let scale = 1.0 / (w * h) as f32;
        for (o, i) in out.iter().zip(&input) {
            assert!((o.re * scale - i).abs() < 1e-5);
            assert!((o.im * scale).abs() < 1e-5);
        }
    }

    #[test]
    fn transpose_large_parallel_matches_definition() {
        let (rows, cols) = (300, 257);
        let src: Vec<u32> = (0..rows * cols).map(|i| i as u32).collect();
        let mut dst = vec![0u32; rows * cols];
        transpose(&src, rows, cols, &mut dst);
        for r in 0..rows {
            for c in 0..cols {
                assert_eq!(dst[c * rows + r], src[r * cols + c]);
            }
        }
    }
}
