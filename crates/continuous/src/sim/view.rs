//! Render data for continuous worlds: box-downsampled planes of a requested
//! rectangle plus per-channel statistics.

use simulation_core::Grid2;

use crate::stats::{ChannelStats, channel_stats};

/// Largest number of output samples per side.
pub const MAX_VIEW_SIDE: u32 = 4096;

/// Which field a view shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewLayer {
    /// Cell states (one plane per channel).
    #[default]
    State,
    /// Growth of the last tick (Lenia `D_j`, SmoothLife `2s − 1`, Flow-Lenia
    /// affinity `U_j`), one plane per channel, values in about `[−1, 1]`
    /// (Flow-Lenia affinity is a weighted sum and can exceed that range).
    Growth,
    /// Potentials of the last tick: Lenia and Flow-Lenia `K_0 ∗ A`;
    /// SmoothLife inner filling `m` and outer filling `n`.
    Potential,
}

/// What a renderer asks for. Coordinates wrap around the torus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContinuousViewRequest {
    /// Left column of the region.
    pub x0: i64,
    /// Top row of the region.
    pub y0: i64,
    /// Region width in cells (0 = whole world width).
    pub width: u32,
    /// Region height in cells (0 = whole world height).
    pub height: u32,
    /// Largest number of output columns (clamped to `1..=MAX_VIEW_SIDE`).
    pub max_cols: u32,
    /// Largest number of output rows (clamped to `1..=MAX_VIEW_SIDE`).
    pub max_rows: u32,
    /// Field to show.
    pub layer: ViewLayer,
}

impl Default for ContinuousViewRequest {
    fn default() -> Self {
        Self { x0: 0, y0: 0, width: 0, height: 0, max_cols: 1024, max_rows: 1024, layer: ViewLayer::State }
    }
}

/// Render-ready data.
#[derive(Clone, Debug, PartialEq)]
pub struct ContinuousView {
    /// Tick the view was built at.
    pub tick: u64,
    /// World width in cells.
    pub world_width: usize,
    /// World height in cells.
    pub world_height: usize,
    /// Left column of the region (wrapped into the world).
    pub x0: usize,
    /// Top row of the region (wrapped into the world).
    pub y0: usize,
    /// Region width in cells.
    pub width: usize,
    /// Region height in cells.
    pub height: usize,
    /// Cells per output sample along each axis (box filter size).
    pub cells_per_sample: usize,
    /// Output columns.
    pub cols: usize,
    /// Output rows.
    pub rows: usize,
    /// Layer shown.
    pub layer: ViewLayer,
    /// Name of each plane ("channel 0", "growth 1", "inner filling m"...).
    pub labels: Vec<String>,
    /// One `cols × rows` row-major plane per label.
    pub planes: Vec<Vec<f32>>,
    /// Natural display range of the values.
    pub value_range: (f32, f32),
    /// Statistics of each state channel over the whole world.
    pub channel_stats: Vec<ChannelStats>,
}

/// Box-averages the region `[x0, x0 + width) × [y0, y0 + height)` (given
/// as `(x0, y0, width, height)`) of a `world.0 × world.1` plane, wrapping
/// around the torus, into samples of `factor × factor` cells; partial blocks
/// at the far edges average the cells they contain.
pub fn downsample(
    plane: &[f32],
    world: (usize, usize),
    region: (usize, usize, usize, usize),
    factor: usize,
) -> Vec<f32> {
    let (world_width, world_height) = world;
    let (x0, y0, width, height) = region;
    let factor = factor.max(1);
    let cols = width.div_ceil(factor);
    let rows = height.div_ceil(factor);
    let mut out = vec![0.0f32; cols * rows];
    let mut col_sum = vec![0.0f64; cols];
    for (r, out_row) in out.chunks_mut(cols).enumerate() {
        col_sum.iter_mut().for_each(|v| *v = 0.0);
        let ry0 = r * factor;
        let ry1 = (ry0 + factor).min(height);
        for ry in ry0..ry1 {
            let y = (y0 + ry) % world_height;
            let row = &plane[y * world_width..(y + 1) * world_width];
            for rx in 0..width {
                col_sum[rx / factor] += f64::from(row[(x0 + rx) % world_width]);
            }
        }
        for (c, o) in out_row.iter_mut().enumerate() {
            let cx1 = ((c + 1) * factor).min(width);
            let count = (cx1 - c * factor) * (ry1 - ry0);
            *o = (col_sum[c] / count as f64) as f32;
        }
    }
    out
}

/// Builds a view of `planes` (each `world_width × world_height`).
pub(crate) fn build(
    request: &ContinuousViewRequest,
    tick: u64,
    state: &[Grid2<f32>],
    planes: Vec<(String, &[f32])>,
    value_range: (f32, f32),
) -> ContinuousView {
    let (ww, wh) = (state[0].width(), state[0].height());
    let width = if request.width == 0 { ww } else { (request.width as usize).min(ww) };
    let height = if request.height == 0 { wh } else { (request.height as usize).min(wh) };
    let max_cols = request.max_cols.clamp(1, MAX_VIEW_SIDE) as usize;
    let max_rows = request.max_rows.clamp(1, MAX_VIEW_SIDE) as usize;
    let factor = width.div_ceil(max_cols).max(height.div_ceil(max_rows)).max(1);
    let x0 = request.x0.rem_euclid(ww as i64) as usize;
    let y0 = request.y0.rem_euclid(wh as i64) as usize;
    let (labels, data): (Vec<String>, Vec<Vec<f32>>) = planes
        .into_iter()
        .map(|(label, plane)| (label, downsample(plane, (ww, wh), (x0, y0, width, height), factor)))
        .unzip();
    ContinuousView {
        tick,
        world_width: ww,
        world_height: wh,
        x0,
        y0,
        width,
        height,
        cells_per_sample: factor,
        cols: width.div_ceil(factor),
        rows: height.div_ceil(factor),
        layer: request.layer,
        labels,
        planes: data,
        value_range,
        channel_stats: state.iter().map(|g| channel_stats(g.as_slice(), ww)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_averages_blocks_with_wrap() {
        // 4 × 4 world, values = x + 10·y.
        let plane: Vec<f32> = (0..16).map(|i| (i % 4) as f32 + 10.0 * (i / 4) as f32).collect();
        let full = downsample(&plane, (4, 4), (0, 0, 4, 4), 1);
        assert_eq!(full, plane);
        let half = downsample(&plane, (4, 4), (0, 0, 4, 4), 2);
        assert_eq!(half, vec![5.5, 7.5, 25.5, 27.5]);
        // Region starting at (3, 3) wraps: cells (3,3), (0,3), (3,0), (0,0).
        let wrapped = downsample(&plane, (4, 4), (3, 3, 2, 2), 2);
        assert_eq!(wrapped, vec![(33.0 + 30.0 + 3.0 + 0.0) / 4.0]);
        // Partial edge block: width 3 with factor 2 → columns {0,1} and {2}.
        let partial = downsample(&plane, (4, 4), (0, 0, 3, 1), 2);
        assert_eq!(partial, vec![0.5, 2.0]);
    }

    #[test]
    fn request_clamps_and_wraps() {
        let state = vec![Grid2::new(64, 32, 0.5f32)];
        let req = ContinuousViewRequest {
            x0: -10,
            y0: 40,
            width: 1000,
            height: 0,
            max_cols: 16,
            max_rows: 0,
            ..Default::default()
        };
        let v = build(&req, 3, &state, vec![("channel 0".into(), state[0].as_slice())], (0.0, 1.0));
        assert_eq!((v.x0, v.y0, v.width, v.height), (54, 8, 64, 32));
        assert_eq!(v.cells_per_sample, 32);
        assert_eq!((v.cols, v.rows), (2, 1));
        assert!(v.planes[0].iter().all(|&x| (x - 0.5).abs() < 1e-6));
        assert!((v.channel_stats[0].mass - 1024.0).abs() < 1e-6);
    }
}
