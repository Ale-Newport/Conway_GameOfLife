//! Render data of [`super::MetapixelSim`] for multi-scale zoom: the micro
//! cell texture (grid-view contract), the macro grid, tile inspection data,
//! scale indicators and phase information.

use hashlife::Universe;
use serde::{Deserialize, Serialize};

use crate::annotations::Annotation;
use crate::readout::{LiveReading, TileGeometry, TileReading};
use crate::spec::{Direction, PERIOD, TILE_LOG2, TILE_PITCH, displayed_metagen, in_transition_window};

/// Largest texture side a [`GridViewRequest`] may ask for (wgpu's default
/// texture limit).
pub const MAX_TEXTURE_SIDE: u32 = 8192;
/// Largest `log2(cells_per_texel)` (`cells_per_texel` is a `u32`).
pub const MAX_LOD_LOG2: u32 = 31;
/// Largest grid for which live core counts are computed on request.
pub const MAX_LIVE_READOUT_TILES: usize = 1024;

/// A cell rectangle to render and a texture budget.
///
/// The rectangle is **inclusive**: `[x0, x1] × [y0, y1]` (reversed bounds
/// are swapped). The model picks the smallest power-of-two
/// `cells_per_texel` for which the texel-aligned cover of the rectangle
/// fits in `max_width × max_height` texels (each clamped to
/// `1..=MAX_TEXTURE_SIDE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridViewRequest {
    /// Left column.
    pub x0: i64,
    /// Top row.
    pub y0: i64,
    /// Right column (inclusive).
    pub x1: i64,
    /// Bottom row (inclusive).
    pub y1: i64,
    /// Texture width budget in texels.
    pub max_width: u32,
    /// Texture height budget in texels.
    pub max_height: u32,
}

/// RGBA8 texture of a cell region (the workspace's 2D grid-view contract).
///
/// * `origin_x`, `origin_y`: cell coordinates of texel `(0, 0)`'s top-left
///   corner, multiples of `cells_per_texel`.
/// * `cells_per_texel`: power of two; 1 = exact cells, larger = level of
///   detail.
/// * `rgba`: `width × height × 4` bytes, row 0 at the top. **R** = state
///   (1 if the texel holds a live cell, else 0); **G** = age (not tracked
///   by Hashlife: 0); **B** = activity (not tracked: 0); **A** = density,
///   `round(255 × live / covered)` where `covered` is the number of
///   requested cells of the texel (255/0 for exact texels).
///
/// The texels cover the requested rectangle exactly when it is aligned and
/// otherwise its aligned cover (at most `cells_per_texel − 1` extra cells on
/// each side), and every texel describes only the **requested** cells of
/// its block, as in the grid views of `cellular-automata`: no cell outside
/// the request is counted. Texels whose whole block is requested add the
/// cached populations of the quadtree nodes inside it
/// (`Universe::density_grid`, O(visible nodes)); the border texels of an
/// unaligned request count the requested part of their block with
/// `Universe::population_in_rect`, whose cost is bounded by the quadtree
/// nodes crossing that part's border (memoised per distinct node), never
/// by the population. No cell is iterated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridView {
    /// Cell x of texel (0, 0).
    pub origin_x: i64,
    /// Cell y of texel (0, 0).
    pub origin_y: i64,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Cells per texel side (power of two).
    pub cells_per_texel: u32,
    /// RGBA bytes, row-major.
    pub rgba: Vec<u8>,
}

/// Scale indicators for the micro view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScaleInfo {
    /// Cells per texel side.
    pub cells_per_texel: u32,
    /// `log2(cells_per_texel)`.
    pub log2_cells_per_texel: u32,
    /// Texels across one 2048-cell tile.
    pub texels_per_tile: f64,
    /// Grid columns intersecting the requested rectangle.
    pub tiles_visible_x: u32,
    /// Grid rows intersecting the requested rectangle.
    pub tiles_visible_y: u32,
}

/// Where the construction is inside its 35 328-generation cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CycleStage {
    /// `t < P`: proximity fuses burn and first signals are exchanged; the
    /// display keeps showing metagen 0.
    FirstCycle,
    /// `t mod P ∈ [4320, 7407]`, `t ≥ P`: displays may be switching.
    Transition,
    /// Displays are steady.
    Stable,
}

/// Which half of the cycle `t mod P` is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CycleHalf {
    /// `t mod P < P / 2`.
    First,
    /// `t mod P ≥ P / 2`.
    Second,
}

/// Timing of the construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseInfo {
    /// Physical generation `t`.
    pub generation: u64,
    /// Generations per metageneration (35 328).
    pub period: u64,
    /// `t mod P`.
    pub phase: u64,
    /// `⌊t / P⌋`.
    pub cycle: u64,
    /// `m(t)`, the metageneration on display.
    pub displayed_metagen: u64,
    /// Stage of the cycle.
    pub stage: CycleStage,
    /// Half of the cycle.
    pub half: CycleHalf,
    /// Metageneration of the latest exact readout.
    pub latest_readout: Option<u64>,
    /// Generation of the next exact readout.
    pub next_readout_generation: u64,
}

impl PhaseInfo {
    /// Timing at generation `t`.
    pub fn at(t: u64, latest_readout: Option<u64>) -> Self {
        let phase = t % PERIOD;
        let stage = if t < PERIOD {
            CycleStage::FirstCycle
        } else if in_transition_window(t) {
            CycleStage::Transition
        } else {
            CycleStage::Stable
        };
        Self {
            generation: t,
            period: PERIOD,
            phase,
            cycle: t / PERIOD,
            displayed_metagen: displayed_metagen(t),
            stage,
            half: if phase < PERIOD / 2 { CycleHalf::First } else { CycleHalf::Second },
            latest_readout,
            next_readout_generation: (t / PERIOD).saturating_add(1).saturating_mul(PERIOD),
        }
    }
}

/// One metacell in the macro view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroTile {
    /// Exact reading of the latest readout (`None` before `t = P`).
    pub exact: Option<TileReading>,
    /// Accelerated prediction for [`MacroView::accelerated_metagen`]
    /// (**not** a physical result).
    pub accelerated: bool,
    /// Exact reading and prediction disagree (latest readout).
    pub mismatch: bool,
    /// Live cells in the tile's nominal 2048×2048 square now.
    pub population: u64,
    /// Threshold reading of the core now (only when requested).
    pub live: Option<LiveReading>,
}

/// The macro grid: exact states, accelerated states and mismatches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MacroView {
    /// Grid width in tiles.
    pub columns: u32,
    /// Grid height in tiles.
    pub rows: u32,
    /// Metageneration of the exact states (`None` before the first readout).
    pub exact_metagen: Option<u64>,
    /// Generation at which the exact states were read.
    pub exact_generation: Option<u64>,
    /// Metageneration of the accelerated states.
    pub accelerated_metagen: u64,
    /// Row-major tiles.
    pub tiles: Vec<MacroTile>,
    /// Number of mismatching tiles in the latest readout.
    pub mismatch_count: usize,
    /// Number of ambiguous tiles in the latest readout.
    pub ambiguous_count: usize,
    /// Label to show with exact states.
    pub exact_label: String,
    /// Label to show with accelerated states.
    pub accelerated_label: String,
}

/// One neighbour channel of an inspected tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighbourChannel {
    /// Direction of the neighbour.
    pub direction: Direction,
    /// The neighbour's tile, if it exists.
    pub tile: Option<(u32, u32)>,
    /// Whether this tile listens to that direction.
    pub heard: bool,
}

/// Tile-local data for micro-level overlays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileInspection {
    /// Column.
    pub col: u32,
    /// Row.
    pub row: u32,
    /// Plane x of the tile's nominal origin (add to annotation rectangles).
    pub origin_x: i64,
    /// Plane y of the tile's nominal origin.
    pub origin_y: i64,
    /// Labelled components (tile-nominal coordinates).
    pub annotations: Vec<Annotation>,
    /// Live cells in the readout rectangle now.
    pub core_count: u64,
    /// Threshold reading of the core now.
    pub live: LiveReading,
    /// Live cells in the nominal square now.
    pub population: u64,
    /// Exact reading of the latest readout.
    pub exact: Option<TileReading>,
    /// Accelerated prediction for the same metageneration as the macro view.
    pub accelerated: bool,
    /// The eight neighbour channels.
    pub neighbours: Vec<NeighbourChannel>,
}

/// What a renderer asks for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetapixelViewRequest {
    /// Cell texture to build; `None` skips it (macro-only display).
    pub micro: Option<GridViewRequest>,
    /// Whether to build the macro grid.
    pub macro_grid: bool,
    /// Whether to compute live core readings for every tile (only for grids
    /// of at most [`MAX_LIVE_READOUT_TILES`] tiles).
    pub live_readings: bool,
    /// Tile to inspect.
    pub inspect: Option<(u32, u32)>,
}

impl Default for MetapixelViewRequest {
    /// The macro grid plus a 1024×1024-texel overview of a 3×3 construction.
    fn default() -> Self {
        Self {
            micro: Some(GridViewRequest {
                x0: -5,
                y0: -5,
                x1: 3 * TILE_PITCH + 4,
                y1: 3 * TILE_PITCH + 4,
                max_width: 1024,
                max_height: 1024,
            }),
            macro_grid: true,
            live_readings: false,
            inspect: None,
        }
    }
}

/// Everything a frame needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetapixelView {
    /// Cell texture (when requested).
    pub micro: Option<GridView>,
    /// Scale of the cell texture (with `micro`).
    pub scale: Option<ScaleInfo>,
    /// Macro grid (when requested; also `None` if the tiles could not be
    /// counted, which does not happen for grids built by this crate).
    pub macro_grid: Option<MacroView>,
    /// Inspected tile (when requested and inside the grid).
    pub inspection: Option<TileInspection>,
    /// Timing.
    pub phase: PhaseInfo,
}

/// Number of texels covering `[lo, hi]` with texels of `2^k` cells aligned
/// to multiples of `2^k`, and the aligned start.
fn aligned_span(lo: i64, hi: i64, k: u32) -> (i64, u64) {
    let size = 1i128 << k;
    let first = i128::from(lo).div_euclid(size);
    let last = i128::from(hi).div_euclid(size);
    // `first * size` lies in [lo - size + 1, lo], inside i64 for k ≤ 31
    // except at the very bottom of the range, where it saturates.
    let start = (first * size).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
    (start, (last - first + 1) as u64)
}

/// Builds the micro texture for `request` (see [`GridView`]).
pub fn grid_view(universe: &Universe, request: &GridViewRequest) -> GridView {
    let (x0, x1) = (request.x0.min(request.x1), request.x0.max(request.x1));
    let (y0, y1) = (request.y0.min(request.y1), request.y0.max(request.y1));
    let max_w = u64::from(request.max_width.clamp(1, MAX_TEXTURE_SIDE));
    let max_h = u64::from(request.max_height.clamp(1, MAX_TEXTURE_SIDE));
    let mut k = 0;
    while k < MAX_LOD_LOG2 && (aligned_span(x0, x1, k).1 > max_w || aligned_span(y0, y1, k).1 > max_h) {
        k += 1;
    }
    let (origin_x, w) = aligned_span(x0, x1, k);
    let (origin_y, h) = aligned_span(y0, y1, k);
    // Beyond 2^31 cells per texel the budget may still be exceeded: keep the
    // top-left part of the request.
    let (width, height) = (w.min(max_w) as u32, h.min(max_h) as u32);
    let counts = universe
        .density_grid(origin_x, origin_y, width as usize, height as usize, k)
        // Unreachable: at most 8192² texels, below Hashlife's pixel limit.
        .unwrap_or_else(|_| vec![0; width as usize * height as usize]);
    // The requested cells of every texel column (and row), and whether they
    // are its whole block (all but the first and last of an unaligned request).
    let size = 1i128 << k;
    let covered = |origin: i64, len: u32, lo: i64, hi: i64| -> Vec<(i64, i64, bool)> {
        (0..i128::from(len))
            .map(|t| {
                let start = i128::from(origin) + t * size;
                let (a, b) = (start.max(i128::from(lo)), (start + size - 1).min(i128::from(hi)));
                // Requested cells lie in the i64 plane.
                (a as i64, b as i64, b - a + 1 == size)
            })
            .collect()
    };
    let columns = covered(origin_x, width, x0, x1);
    let rows = covered(origin_y, height, y0, y1);
    let area = (size * size) as f64;
    let mut rgba = Vec::with_capacity(counts.len() * 4);
    for (&(cy0, cy1, whole_row), row) in rows.iter().zip(counts.chunks(width as usize)) {
        for (&(cx0, cx1, whole_column), &block) in columns.iter().zip(row) {
            let (live, cells) = if whole_row && whole_column {
                (f64::from(block), area)
            } else {
                // A border texel: only the requested part of its block (none
                // of an empty block).
                let live = if block == 0 { 0 } else { universe.population_in_rect(cx0, cy0, cx1, cy1) };
                (live as f64, ((cx1 - cx0 + 1) as u64 * (cy1 - cy0 + 1) as u64) as f64)
            };
            let density = (255.0 * live / cells).round().clamp(0.0, 255.0) as u8;
            rgba.extend_from_slice(&[u8::from(live > 0.0), 0, 0, density]);
        }
    }
    GridView { origin_x, origin_y, width, height, cells_per_texel: 1 << k, rgba }
}

/// Scale indicators for a micro view built for `request`.
pub fn scale_info(view: &GridView, request: &GridViewRequest, geometry: &TileGeometry) -> ScaleInfo {
    let (x0, x1) = (request.x0.min(request.x1), request.x0.max(request.x1));
    let (y0, y1) = (request.y0.min(request.y1), request.y0.max(request.y1));
    let visible = |lo: i64, hi: i64, origin: i64, count: u32| -> u32 {
        let first = (lo.saturating_sub(origin)).div_euclid(TILE_PITCH).max(0);
        let last = (hi.saturating_sub(origin)).div_euclid(TILE_PITCH).min(i64::from(count) - 1);
        if last < first { 0 } else { (last - first + 1) as u32 }
    };
    let k = view.cells_per_texel.trailing_zeros();
    ScaleInfo {
        cells_per_texel: view.cells_per_texel,
        log2_cells_per_texel: k,
        texels_per_tile: (1u64 << TILE_LOG2) as f64 / f64::from(view.cells_per_texel),
        tiles_visible_x: visible(x0, x1, geometry.origin_x, geometry.columns),
        tiles_visible_y: visible(y0, y1, geometry.origin_y, geometry.rows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_is_chosen_to_fit_the_budget() {
        let mut universe = Universe::conway();
        universe.insert_cells([(0, 0), (1, 0), (2, 0), (100, 100)]);
        let exact = grid_view(&universe, &GridViewRequest { x0: 0, y0: 0, x1: 3, y1: 1, max_width: 8, max_height: 8 });
        assert_eq!((exact.width, exact.height, exact.cells_per_texel), (4, 2, 1));
        assert_eq!(&exact.rgba[..12], &[1, 0, 0, 255, 1, 0, 0, 255, 1, 0, 0, 255]);
        assert_eq!(&exact.rgba[12..16], &[0, 0, 0, 0]);
        let lod =
            grid_view(&universe, &GridViewRequest { x0: -1, y0: -1, x1: 101, y1: 101, max_width: 16, max_height: 16 });
        assert_eq!(lod.cells_per_texel, 8);
        assert_eq!((lod.origin_x, lod.origin_y), (-8, -8), "origins snap to the texel grid");
        assert!(lod.width <= 16 && lod.height <= 16);
        assert_eq!(lod.rgba.len(), (lod.width * lod.height * 4) as usize);
        // Texel (1, 1) covers [0, 8)²: 3 live cells of 64.
        let i = ((lod.width + 1) * 4) as usize;
        assert_eq!(&lod.rgba[i..i + 4], &[1, 0, 0, 12]);
        let total: u32 = lod.rgba.chunks(4).map(|t| u32::from(t[0])).sum();
        assert_eq!(total, 2, "two occupied texels");
    }

    #[test]
    fn border_texels_count_only_the_requested_cells() {
        let mut universe = Universe::conway();
        let cells: Vec<(i64, i64)> = (0..400).map(|i| (i * 37 % 131 - 20, i * 59 % 127 - 15)).collect();
        universe.insert_cells(cells.iter().copied());
        let requests = [
            GridViewRequest { x0: -5, y0: -7, x1: 100, y1: 98, max_width: 16, max_height: 16 },
            GridViewRequest { x0: 60, y0: 2, x1: 3, y1: 1, max_width: 4, max_height: 4 },
            GridViewRequest { x0: -13, y0: -11, x1: 9, y1: 13, max_width: 64, max_height: 64 },
        ];
        for request in requests {
            let view = grid_view(&universe, &request);
            let (x0, x1) = (request.x0.min(request.x1), request.x0.max(request.x1));
            let (y0, y1) = (request.y0.min(request.y1), request.y0.max(request.y1));
            let k = i64::from(view.cells_per_texel);
            for (i, texel) in view.rgba.chunks(4).enumerate() {
                let (tx, ty) = ((i % view.width as usize) as i64, (i / view.width as usize) as i64);
                let (bx, by) = (view.origin_x + tx * k, view.origin_y + ty * k);
                let (cx0, cx1) = (bx.max(x0), (bx + k - 1).min(x1));
                let (cy0, cy1) = (by.max(y0), (by + k - 1).min(y1));
                let live = cells.iter().filter(|&&(x, y)| (cx0..=cx1).contains(&x) && (cy0..=cy1).contains(&y)).count();
                let density = (255.0 * live as f64 / ((cx1 - cx0 + 1) * (cy1 - cy0 + 1)) as f64).round() as u8;
                assert_eq!(texel, [u8::from(live > 0), 0, 0, density], "{request:?}, texel ({tx}, {ty})");
            }
        }
    }

    #[test]
    fn phase_info_stages() {
        assert_eq!(PhaseInfo::at(10, None).stage, CycleStage::FirstCycle);
        let t = PERIOD + 5000;
        let info = PhaseInfo::at(t, Some(0));
        assert_eq!(
            (info.stage, info.half, info.cycle, info.phase),
            (CycleStage::Transition, CycleHalf::First, 1, 5000)
        );
        assert_eq!(info.next_readout_generation, 2 * PERIOD);
        assert_eq!(PhaseInfo::at(PERIOD + 30_000, None).half, CycleHalf::Second);
        assert_eq!(PhaseInfo::at(PERIOD + 7408, None).stage, CycleStage::Stable);
    }
}
