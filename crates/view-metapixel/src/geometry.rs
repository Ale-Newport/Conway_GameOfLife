//! Pure coordinate mapping between physical cells, metapixel tiles and the
//! screen: the construction's extent, the tile under a cell, the micro view
//! request for the visible region, the conversion of the model's grid view
//! into a renderer frame, the zoom-dependent blend between the physical and
//! the macro layer, and the scale bar.
//!
//! Conventions (shared with `metapixel`): cell `x` grows right, `y` grows
//! down; tile `(col, row)` has its nominal square at
//! `[2048·col, 2048·col + 2047] × [2048·row, 2048·row + 2047]`; tile files
//! extend 5 cells beyond the nominal square on every side.

use bevy::math::I64Vec2;
use metapixel::spec::{CellRect, TILE_PITCH};
use metapixel::{GridView, GridViewRequest, MAX_LOD_LOG2, MAX_TEXTURE_SIDE};
use rendering::GridFrame;
use rendering::grid::{GridFrameError, VisibleCells, encode_unit};

/// Cells a tile occupies beyond its nominal square on every side.
pub const TILE_MARGIN: i64 = 5;

/// Fraction of the visible size requested beyond each edge of the view, so
/// small pans do not need a new view from the simulation thread.
pub const REQUEST_MARGIN: f64 = 0.25;

/// Screen size of a tile (logical pixels) at or below which the automatic
/// layer shows only the macro readout.
pub const MACRO_FULL_PX: f32 = 32.0;

/// Screen size of a tile (logical pixels) at or above which the automatic
/// layer shows only the physical cells.
pub const MICRO_FULL_PX: f32 = 128.0;

/// Smallest displayed density of an occupied texel (so a single live cell
/// in a large texel stays visible).
pub const DENSITY_FLOOR: f32 = 0.16;

/// Exponent of the density scale (below 1 brightens sparse texels: the
/// circuitry of a tile is mostly empty space).
pub const DENSITY_GAMMA: f32 = 0.4;

/// Displayed density of a fully live texel and of an exact live cell.
pub const DENSITY_TOP: f32 = 0.92;

/// Size of a rectangular grid of tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridShape {
    /// Tiles per row.
    pub columns: u32,
    /// Rows of tiles.
    pub rows: u32,
}

impl GridShape {
    /// A grid of `columns × rows` tiles (each at least 1).
    pub fn new(columns: u32, rows: u32) -> Self {
        Self { columns: columns.max(1), rows: rows.max(1) }
    }

    /// Number of tiles.
    pub fn tile_count(self) -> usize {
        self.columns as usize * self.rows as usize
    }

    /// Nominal squares of every tile, inclusive.
    pub fn nominal(self) -> CellRect {
        CellRect {
            x0: 0,
            y0: 0,
            x1: TILE_PITCH * i64::from(self.columns) - 1,
            y1: TILE_PITCH * i64::from(self.rows) - 1,
        }
    }

    /// Every cell the construction can occupy (nominal squares plus the
    /// 5-cell tile margin), inclusive.
    pub fn bounds(self) -> CellRect {
        let n = self.nominal();
        CellRect { x0: n.x0 - TILE_MARGIN, y0: n.y0 - TILE_MARGIN, x1: n.x1 + TILE_MARGIN, y1: n.y1 + TILE_MARGIN }
    }

    /// Side lengths of the nominal plane in cells.
    pub fn size_cells(self) -> (u64, u64) {
        (TILE_PITCH as u64 * u64::from(self.columns), TILE_PITCH as u64 * u64::from(self.rows))
    }

    /// The tile whose nominal square contains `cell`.
    pub fn tile_at(self, cell: I64Vec2) -> Option<(u32, u32)> {
        let col = cell.x.div_euclid(TILE_PITCH);
        let row = cell.y.div_euclid(TILE_PITCH);
        let col = u32::try_from(col).ok().filter(|&c| c < self.columns)?;
        let row = u32::try_from(row).ok().filter(|&r| r < self.rows)?;
        Some((col, row))
    }

    /// Nominal square of tile `(col, row)`, inclusive.
    pub fn tile_rect(self, col: u32, row: u32) -> CellRect {
        let (x, y) = (TILE_PITCH * i64::from(col), TILE_PITCH * i64::from(row));
        CellRect { x0: x, y0: y, x1: x + TILE_PITCH - 1, y1: y + TILE_PITCH - 1 }
    }

    /// Whether `(col, row)` is a tile of the grid.
    pub fn contains(self, col: u32, row: u32) -> bool {
        col < self.columns && row < self.rows
    }

    /// Tiles (inclusive column and row ranges) that intersect the visible
    /// cells, or `None` when no tile is visible.
    pub fn visible_tiles(self, visible: &VisibleCells) -> Option<((u32, u32), (u32, u32))> {
        let n = self.nominal();
        let x0 = visible.min_x.max(n.x0);
        let x1 = visible.max_x.min(n.x1);
        let y0 = visible.min_y.max(n.y0);
        let y1 = visible.max_y.min(n.y1);
        if x0 > x1 || y0 > y1 {
            return None;
        }
        let tile = |v: i64| (v.div_euclid(TILE_PITCH)) as u32;
        Some(((tile(x0), tile(x1)), (tile(y0), tile(y1))))
    }
}

/// Number of texels covering `[lo, hi]` with texels of `2^k` cells aligned
/// to multiples of `2^k` (the rule the model applies to view requests).
pub fn aligned_texels(lo: i64, hi: i64, k: u32) -> u64 {
    let size = 1i128 << k.min(62);
    let first = i128::from(lo.min(hi)).div_euclid(size);
    let last = i128::from(lo.max(hi)).div_euclid(size);
    (last - first + 1) as u64
}

/// `log2(cells_per_texel)` the model will choose for `request` (smallest
/// power of two whose aligned cover fits the texel budget).
pub fn expected_lod(request: &GridViewRequest) -> u32 {
    let max_w = u64::from(request.max_width.clamp(1, MAX_TEXTURE_SIDE));
    let max_h = u64::from(request.max_height.clamp(1, MAX_TEXTURE_SIDE));
    let mut k = 0;
    while k < MAX_LOD_LOG2
        && (aligned_texels(request.x0, request.x1, k) > max_w || aligned_texels(request.y0, request.y1, k) > max_h)
    {
        k += 1;
    }
    k
}

/// The visible rectangle clipped to the construction, or `None` when the
/// view shows nothing of it.
pub fn visible_in_bounds(visible: &VisibleCells, shape: GridShape) -> Option<CellRect> {
    let b = shape.bounds();
    let rect = CellRect {
        x0: visible.min_x.max(b.x0),
        y0: visible.min_y.max(b.y0),
        x1: visible.max_x.min(b.x1),
        y1: visible.max_y.min(b.y1),
    };
    (rect.x0 <= rect.x1 && rect.y0 <= rect.y1).then_some(rect)
}

/// Micro view request for the visible cells: the visible rectangle grown by
/// `margin` (fraction of its size) on every side and clipped to the
/// construction, with a texel budget that keeps about one texel per logical
/// pixel (`lod_log2` of the camera) and at most `max_side` texels per side.
pub fn micro_request(visible: &VisibleCells, shape: GridShape, margin: f64, max_side: u32) -> Option<GridViewRequest> {
    visible_in_bounds(visible, shape)?;
    let b = shape.bounds();
    let grow = |extent: u64| (extent as f64 * margin.max(0.0)).ceil().min(i64::MAX as f64 / 4.0) as i64;
    let (mx, my) = (grow(visible.width()), grow(visible.height()));
    let x0 = visible.min_x.saturating_sub(mx).max(b.x0);
    let x1 = visible.max_x.saturating_add(mx).min(b.x1);
    let y0 = visible.min_y.saturating_sub(my).max(b.y0);
    let y1 = visible.max_y.saturating_add(my).min(b.y1);
    let k = visible.lod_log2().min(MAX_LOD_LOG2);
    let max_side = max_side.clamp(1, MAX_TEXTURE_SIDE);
    let budget = |lo: i64, hi: i64| aligned_texels(lo, hi, k).clamp(1, u64::from(max_side)) as u32;
    Some(GridViewRequest { x0, y0, x1, y1, max_width: budget(x0, x1), max_height: budget(y0, y1) })
}

/// Whether the previous micro request still serves the current view: same
/// level of detail and the visible part of the construction lies inside it.
pub fn request_still_serves(
    last: &GridViewRequest,
    wanted: &GridViewRequest,
    visible: &VisibleCells,
    shape: GridShape,
) -> bool {
    if expected_lod(last) != expected_lod(wanted) {
        return false;
    }
    match visible_in_bounds(visible, shape) {
        Some(v) => last.x0 <= v.x0 && last.y0 <= v.y0 && last.x1 >= v.x1 && last.y1 >= v.y1,
        None => true,
    }
}

/// Displayed density (0..=1) of a texel holding `live` cells out of
/// `cells` (power scale with exponent [`DENSITY_GAMMA`] between
/// [`DENSITY_FLOOR`] and [`DENSITY_TOP`]; 0 for an empty texel).
pub fn display_density(live_fraction: f32) -> f32 {
    if live_fraction.is_nan() || live_fraction <= 0.0 {
        return 0.0;
    }
    DENSITY_FLOOR + (DENSITY_TOP - DENSITY_FLOOR) * live_fraction.min(1.0).powf(DENSITY_GAMMA)
}

#[cfg(test)]
/// Live fraction shown by a displayed density (inverse of
/// [`display_density`] for values in `[DENSITY_FLOOR, DENSITY_TOP]`).
pub fn live_fraction_of_display(display: f32) -> f32 {
    let t = ((display - DENSITY_FLOOR) / (DENSITY_TOP - DENSITY_FLOOR)).clamp(0.0, 1.0);
    t.powf(1.0 / DENSITY_GAMMA)
}

/// Converts the model's micro grid view into a renderer frame. R (state)
/// is kept; A is rewritten as the displayed density: the live fraction of
/// each texel through [`display_density`], with occupied texels whose
/// rounded density is 0 shown at the floor.
pub fn micro_frame(view: &GridView) -> Result<GridFrame, GridFrameError> {
    let cpt = view.cells_per_texel.max(1);
    let area = f64::from(cpt) * f64::from(cpt);
    let mut data = Vec::with_capacity(view.rgba.len());
    for texel in view.rgba.as_chunks::<4>().0 {
        let alive = texel[0] > 0;
        let fraction = if !alive {
            0.0
        } else if cpt == 1 {
            1.0
        } else if texel[3] == 0 {
            (1.0 / area) as f32
        } else {
            f32::from(texel[3]) / 255.0
        };
        data.extend_from_slice(&[u8::from(alive), 0, 0, encode_unit(display_density(fraction))]);
    }
    let frame = GridFrame {
        origin: I64Vec2::new(view.origin_x, view.origin_y),
        width: view.width,
        height: view.height,
        cells_per_texel: cpt,
        data,
    };
    frame.validate()?;
    Ok(frame)
}

/// Screen size of one tile in logical pixels at `cells_per_pixel`.
pub fn pixels_per_tile(cells_per_pixel: f32) -> f32 {
    if !cells_per_pixel.is_finite() || cells_per_pixel <= 0.0 {
        return f32::INFINITY;
    }
    TILE_PITCH as f32 / cells_per_pixel
}

/// Opacity of the macro layer in automatic mode: 0 (physical cells only)
/// when a tile covers at least [`MICRO_FULL_PX`] pixels, 1 (macro readout
/// only) at [`MACRO_FULL_PX`] or less, a smooth blend in between (linear in
/// the logarithm of the zoom).
pub fn auto_macro_opacity(pixels_per_tile: f32) -> f32 {
    if pixels_per_tile.is_nan() {
        return 0.0;
    }
    let (lo, hi) = (MACRO_FULL_PX.log2(), MICRO_FULL_PX.log2());
    let t = ((hi - pixels_per_tile.max(1e-6).log2()) / (hi - lo)).clamp(0.0, 1.0);
    let o = t * t * (3.0 - 2.0 * t);
    // Snap the ends so "0%" and "100%" blends are never shown.
    if o < 0.01 {
        0.0
    } else if o > 0.99 {
        1.0
    } else {
        o
    }
}

/// A rectangle of the world viewport in logical pixels (origin at the
/// viewport's top-left corner, `y` down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    /// Top-left corner.
    pub min: bevy::math::Vec2,
    /// Bottom-right corner.
    pub max: bevy::math::Vec2,
}

impl ScreenRect {
    /// Width and height (0 when inverted).
    pub fn size(self) -> bevy::math::Vec2 {
        (self.max - self.min).max(bevy::math::Vec2::ZERO)
    }
}

/// Gap kept between the overlay card and a framed rectangle (logical px).
pub const OCCLUDER_GAP: f32 = 10.0;

/// A framed rectangle may shrink at most by this factor to stay clear of
/// the overlay card; beyond it the card is allowed to cover it.
pub const MAX_CLEAR_SHRINK: f32 = 1.6;

/// Where a framed rectangle goes: the viewport area it is centred in and
/// its scale in cells per logical pixel.
fn placement(target: CellRect, area: ScreenRect) -> Option<f32> {
    let size = area.size();
    if size.x < 1.0 || size.y < 1.0 {
        return None;
    }
    let (w, h) = ((target.x1 - target.x0 + 1) as f32, (target.y1 - target.y0 + 1) as f32);
    Some((w / size.x).max(h / size.y))
}

/// The cell rectangle to pass to `GridCameraController::fit_to_rect` (with
/// no margin) so that `target` is shown as large as possible, centred,
/// `margin_px` inside the viewport and, when possible, clear of the
/// `occluder` (the overlay card in the viewport's top-left corner): the
/// target goes below the card or to its right, whichever keeps it larger,
/// unless that would shrink it by more than [`MAX_CLEAR_SHRINK`].
///
/// The result has the viewport's aspect ratio, so framing it maps it onto
/// the whole viewport and `target` onto the chosen area.
pub fn fit_clear_of(
    target: CellRect,
    viewport: bevy::math::Vec2,
    margin_px: f32,
    occluder: Option<ScreenRect>,
) -> CellRect {
    use bevy::math::Vec2;
    if viewport.x < 1.0 || viewport.y < 1.0 {
        return target;
    }
    let m = margin_px.clamp(0.0, viewport.min_element() * 0.45);
    let full = ScreenRect { min: Vec2::splat(m), max: viewport - Vec2::splat(m) };
    let mut best = placement(target, full).map(|s| (full, s));
    if let (Some(o), Some((_, full_scale))) = (occluder.filter(|o| o.size().min_element() > 0.0), best) {
        let below = ScreenRect { min: Vec2::new(m, o.max.y + OCCLUDER_GAP), max: full.max };
        let right = ScreenRect { min: Vec2::new(o.max.x + OCCLUDER_GAP, m), max: full.max };
        let clear = [below, right]
            .into_iter()
            .filter_map(|a| placement(target, a).map(|s| (a, s)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((area, scale)) = clear.filter(|(_, s)| *s <= full_scale * MAX_CLEAR_SHRINK) {
            best = Some((area, scale));
        }
    }
    let Some((area, scale)) = best else { return target };
    let centre = (area.min + area.max) * 0.5;
    let (tcx, tcy) = ((target.x0 + target.x1 + 1) as f64 * 0.5, (target.y0 + target.y1 + 1) as f64 * 0.5);
    let s = f64::from(scale);
    let x0 = (tcx - f64::from(centre.x) * s).round();
    let y0 = (tcy - f64::from(centre.y) * s).round();
    let (w, h) = ((f64::from(viewport.x) * s).round().max(1.0), (f64::from(viewport.y) * s).round().max(1.0));
    let clamp = |v: f64| v.clamp(i64::MIN as f64 / 4.0, i64::MAX as f64 / 4.0) as i64;
    CellRect { x0: clamp(x0), y0: clamp(y0), x1: clamp(x0 + w - 1.0), y1: clamp(y0 + h - 1.0) }
}

/// A scale bar: its length in cells, in logical pixels, and its label.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleBar {
    /// Length in cells.
    pub cells: u64,
    /// Length on screen (logical pixels).
    pub pixels: f32,
    /// Text such as `"100 cells"` or `"3 tiles"`.
    pub label: String,
}

/// The longest "nice" length (1, 2 or 5 × 10ⁿ cells, or whole tiles once a
/// tile fits) that is at most `max_px` logical pixels long.
pub fn scale_bar(cells_per_pixel: f32, max_px: f32) -> Option<ScaleBar> {
    if !cells_per_pixel.is_finite() || cells_per_pixel <= 0.0 || max_px <= 0.0 {
        return None;
    }
    let max_cells = f64::from(max_px) * f64::from(cells_per_pixel);
    if max_cells < 1.0 {
        return None;
    }
    let tile = TILE_PITCH as f64;
    let (cells, label) = if max_cells >= tile {
        let tiles = [1u64, 2, 5, 10, 20, 50, 100, 200, 500]
            .into_iter()
            .take_while(|&n| n as f64 * tile <= max_cells)
            .last()
            .unwrap_or(1);
        (tiles * TILE_PITCH as u64, format!("{tiles} tile{}", if tiles == 1 { "" } else { "s" }))
    } else {
        let mut best = 1u64;
        let mut base = 1u64;
        'outer: loop {
            for m in [1u64, 2, 5] {
                let n = m * base;
                if n as f64 > max_cells {
                    break 'outer;
                }
                best = n;
            }
            base *= 10;
        }
        (best, format!("{best} cell{}", if best == 1 { "" } else { "s" }))
    };
    Some(ScaleBar { cells, pixels: (cells as f64 / f64::from(cells_per_pixel)) as f32, label })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(min_x: i64, min_y: i64, max_x: i64, max_y: i64, cells_per_pixel: f32) -> VisibleCells {
        VisibleCells { min_x, min_y, max_x, max_y, cells_per_pixel }
    }

    #[test]
    fn tiles_map_to_nominal_squares() {
        let shape = GridShape::new(3, 2);
        assert_eq!(shape.tile_at(I64Vec2::new(0, 0)), Some((0, 0)));
        assert_eq!(shape.tile_at(I64Vec2::new(2047, 2047)), Some((0, 0)));
        assert_eq!(shape.tile_at(I64Vec2::new(2048, 4095)), Some((1, 1)));
        assert_eq!(shape.tile_at(I64Vec2::new(-1, 0)), None, "the margin belongs to no tile");
        assert_eq!(shape.tile_at(I64Vec2::new(3 * 2048, 0)), None);
        assert_eq!(shape.tile_at(I64Vec2::new(0, 2 * 2048)), None);
        assert_eq!(shape.tile_rect(2, 1), CellRect { x0: 4096, y0: 2048, x1: 6143, y1: 4095 });
        assert_eq!(shape.bounds(), CellRect { x0: -5, y0: -5, x1: 6148, y1: 4100 });
        assert_eq!(shape.size_cells(), (6144, 4096));
        assert_eq!(GridShape::new(0, 0), GridShape::new(1, 1), "a grid has at least one tile");
        // Agrees with the model's own geometry.
        let geometry = metapixel::TileGeometry::at_origin(3, 2);
        for (x, y) in [(5, 5), (2047, 0), (2048, 2048), (6143, 4095), (-3, 4), (6144, 1)] {
            assert_eq!(shape.tile_at(I64Vec2::new(x, y)), geometry.tile_at(x, y), "({x}, {y})");
        }
    }

    #[test]
    fn visible_tile_ranges_are_clipped() {
        let shape = GridShape::new(4, 4);
        let v = visible(1000, -500, 5000, 3000, 8.0);
        assert_eq!(shape.visible_tiles(&v), Some(((0, 2), (0, 1))));
        let outside = visible(-900, -900, -10, -10, 1.0);
        assert_eq!(shape.visible_tiles(&outside), None);
    }

    #[test]
    fn micro_requests_cover_the_view_with_a_margin_and_fit_the_budget() {
        let shape = GridShape::new(3, 3);
        // Zoomed in: 400 × 200 cells at 0.5 cells per pixel -> exact cells.
        let v = visible(1000, 1000, 1399, 1199, 0.5);
        let r = micro_request(&v, shape, REQUEST_MARGIN, 4096).unwrap();
        assert_eq!((r.x0, r.x1, r.y0, r.y1), (900, 1499, 950, 1249));
        assert_eq!(expected_lod(&r), 0);
        assert_eq!((r.max_width, r.max_height), (600, 300));
        // Zoomed out over the whole construction: clipped to its bounds.
        let v = visible(-3000, -3000, 9000, 9000, 12.0);
        let r = micro_request(&v, shape, REQUEST_MARGIN, 4096).unwrap();
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (-5, -5, 6148, 6148));
        assert_eq!(expected_lod(&r), 4, "16 cells per texel keeps a texel per pixel");
        // Same rule as the model: the budget is never exceeded.
        let tight = micro_request(&v, shape, 0.0, 64).unwrap();
        let k = expected_lod(&tight);
        assert!(aligned_texels(tight.x0, tight.x1, k) <= 64);
        // Outside the construction: nothing to request.
        assert!(micro_request(&visible(-900, -900, -10, -10, 1.0), shape, 0.25, 4096).is_none());
    }

    #[test]
    fn requests_are_reused_while_they_cover_the_view() {
        let shape = GridShape::new(3, 3);
        let v = visible(1000, 1000, 1399, 1199, 0.5);
        let first = micro_request(&v, shape, REQUEST_MARGIN, 4096).unwrap();
        let panned = visible(1050, 1020, 1449, 1219, 0.5);
        let wanted = micro_request(&panned, shape, REQUEST_MARGIN, 4096).unwrap();
        assert!(request_still_serves(&first, &wanted, &panned, shape), "small pans keep the request");
        let far = visible(3000, 1000, 3399, 1199, 0.5);
        let wanted = micro_request(&far, shape, REQUEST_MARGIN, 4096).unwrap();
        assert!(!request_still_serves(&first, &wanted, &far, shape));
        let zoomed = visible(800, 900, 1599, 1299, 4.0);
        let wanted = micro_request(&zoomed, shape, REQUEST_MARGIN, 4096).unwrap();
        assert!(!request_still_serves(&first, &wanted, &zoomed, shape), "a new level of detail needs a new view");
    }

    #[test]
    fn frames_keep_geometry_and_remap_density() {
        let view = GridView {
            origin_x: -8,
            origin_y: 16,
            width: 3,
            height: 1,
            cells_per_texel: 8,
            rgba: vec![0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 255],
        };
        let frame = micro_frame(&view).unwrap();
        assert_eq!((frame.origin, frame.width, frame.height, frame.cells_per_texel), (I64Vec2::new(-8, 16), 3, 1, 8));
        assert_eq!(&frame.data[0..4], &[0, 0, 0, 0], "empty texels stay empty");
        assert_eq!(frame.data[4], 1);
        assert_eq!(frame.data[7], encode_unit(display_density(1.0 / 64.0)), "occupied texels never vanish");
        assert!(frame.data[7] >= encode_unit(DENSITY_FLOOR));
        assert_eq!(frame.data[11], encode_unit(DENSITY_TOP));
        let exact = GridView { cells_per_texel: 1, rgba: vec![1, 0, 0, 255, 0, 0, 0, 0], width: 2, ..view };
        let frame = micro_frame(&exact).unwrap();
        assert_eq!((frame.data[3], frame.data[7]), (encode_unit(DENSITY_TOP), 0));
        let broken = GridView { rgba: vec![1, 2, 3], ..exact };
        assert!(micro_frame(&broken).is_err());
    }

    #[test]
    fn density_scale_round_trips() {
        assert_eq!(display_density(0.0), 0.0);
        assert_eq!(display_density(f32::NAN), 0.0);
        assert!((display_density(1.0) - DENSITY_TOP).abs() < 1e-6);
        for f in [0.01f32, 0.1, 0.25, 0.5, 0.9] {
            assert!((live_fraction_of_display(display_density(f)) - f).abs() < 1e-5, "{f}");
        }
        assert!(display_density(0.04) < display_density(0.05), "monotonic");
    }

    #[test]
    fn automatic_layer_blends_between_the_thresholds() {
        assert_eq!(auto_macro_opacity(MICRO_FULL_PX), 0.0);
        assert_eq!(auto_macro_opacity(1000.0), 0.0);
        assert_eq!(auto_macro_opacity(MACRO_FULL_PX), 1.0);
        assert_eq!(auto_macro_opacity(2.0), 1.0);
        let mid = auto_macro_opacity(64.0);
        assert!((mid - 0.5).abs() < 1e-6, "geometric mean of the thresholds is half way");
        assert!(auto_macro_opacity(40.0) > auto_macro_opacity(80.0));
        assert_eq!(auto_macro_opacity(MICRO_FULL_PX * 0.99), 0.0, "near the ends the layer snaps");
        assert_eq!(auto_macro_opacity(MACRO_FULL_PX * 1.01), 1.0);
        assert_eq!(pixels_per_tile(32.0), 64.0);
        assert!(pixels_per_tile(0.0).is_infinite());
    }

    /// Screen rectangle of `target` when `framed` fills the viewport.
    fn on_screen(target: CellRect, framed: CellRect, viewport: bevy::math::Vec2) -> ScreenRect {
        let s = (framed.x1 - framed.x0 + 1) as f32 / viewport.x;
        let to = |x: i64, y: i64| bevy::math::Vec2::new((x - framed.x0) as f32 / s, (y - framed.y0) as f32 / s);
        ScreenRect { min: to(target.x0, target.y0), max: to(target.x1 + 1, target.y1 + 1) }
    }

    #[test]
    fn framing_keeps_the_target_clear_of_the_overlay() {
        use bevy::math::Vec2;
        let viewport = Vec2::new(845.0, 815.0);
        let grid = GridShape::new(3, 3).bounds();
        // Without an occluder: centred, as large as the margin allows.
        let framed = fit_clear_of(grid, viewport, 20.0, None);
        let aspect = (framed.x1 - framed.x0 + 1) as f32 / (framed.y1 - framed.y0 + 1) as f32;
        assert!((aspect - viewport.x / viewport.y).abs() < 1e-3, "the framed rectangle has the viewport's shape");
        let r = on_screen(grid, framed, viewport);
        assert!((r.min.y - 20.0).abs() < 1.0 && (r.max.y - 795.0).abs() < 1.0, "{r:?}");
        assert!(((r.min.x + r.max.x) * 0.5 - viewport.x * 0.5).abs() < 1.0);
        // A card in the top-left corner: the grid moves below it.
        let card = ScreenRect { min: Vec2::new(12.0, 12.0), max: Vec2::new(392.0, 200.0) };
        let framed = fit_clear_of(grid, viewport, 20.0, Some(card));
        let r = on_screen(grid, framed, viewport);
        assert!(r.min.y >= card.max.y + OCCLUDER_GAP - 1.0, "{r:?}");
        assert!(r.max.y <= viewport.y - 19.0 && r.min.x >= 19.0 && r.max.x <= viewport.x - 19.0, "{r:?}");
        // A tall, narrow target goes to the right of the card instead.
        // (Framing works in whole cells: allow one cell of rounding, here
        // about 4 pixels.)
        let tall = CellRect { x0: 100, y0: 1380, x1: 210, y1: 1540 };
        let framed = fit_clear_of(tall, viewport, 20.0, Some(card));
        let cell_px = viewport.x / (framed.x1 - framed.x0 + 1) as f32;
        assert!(cell_px > 3.0 && cell_px < 5.0);
        let r = on_screen(tall, framed, viewport);
        assert!(r.min.x >= card.max.x + OCCLUDER_GAP - cell_px, "{r:?}");
        let inside = |v: f32, lo: f32, hi: f32| v >= lo - cell_px && v <= hi + cell_px;
        assert!(inside(r.max.x, 0.0, viewport.x - 20.0) && inside(r.min.y, 20.0, viewport.y), "{r:?}");
        assert!(inside(r.max.y, 0.0, viewport.y - 20.0), "{r:?}");
        // A card covering most of the viewport is accepted rather than
        // shrinking the target to nothing.
        let huge = ScreenRect { min: Vec2::new(12.0, 12.0), max: Vec2::new(800.0, 700.0) };
        let r = on_screen(grid, fit_clear_of(grid, viewport, 20.0, Some(huge)), viewport);
        assert!((r.min.y - 20.0).abs() < 1.0, "{r:?}");
    }

    #[test]
    fn scale_bars_pick_round_lengths() {
        let bar = scale_bar(0.25, 120.0).unwrap();
        assert_eq!((bar.cells, bar.label.as_str()), (20, "20 cells"));
        assert_eq!(bar.pixels, 80.0);
        let bar = scale_bar(4.0, 120.0).unwrap();
        assert_eq!(bar.cells, 200);
        let bar = scale_bar(20.0, 120.0).unwrap();
        assert_eq!((bar.cells, bar.label.as_str()), (2048, "1 tile"));
        assert_eq!(scale_bar(40.0, 120.0).unwrap().label, "2 tiles");
        let bar = scale_bar(400.0, 120.0).unwrap();
        assert_eq!(bar.label, "20 tiles");
        assert!(bar.pixels <= 120.0);
        assert!(scale_bar(0.001, 0.5).is_none(), "shorter than one cell");
        assert!(scale_bar(f32::NAN, 100.0).is_none());
    }
}
