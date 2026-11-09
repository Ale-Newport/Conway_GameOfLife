//! Mapping between integer cell coordinates and Bevy world space for 2D grids.
//!
//! Cell coordinates follow the workspace convention (x grows right, y grows
//! downward, `i64` for unbounded worlds). Bevy's 2D world has y growing upward,
//! so cell `(x, y)` occupies the world rectangle
//! `[(x - ax) s, (x - ax + 1) s] × [-(y - ay + 1) s, -(y - ay) s]`, where `s` is
//! [`GridSpace::cell_size`] and `(ax, ay)` is the floating-origin
//! [`GridSpace::anchor`].
//!
//! The anchor keeps world coordinates small: `f32` is exact for integers only
//! up to 2^24, while Hashlife patterns can spread over far larger coordinates.
//! The 2D camera controller re-anchors when the camera drifts far from the
//! anchor (see `camera2d`), so precision near the camera never degrades.

use bevy::math::I64Vec2;
use bevy::prelude::*;

/// Shared mapping between cell coordinates and world units for 2D grids.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct GridSpace {
    /// Size of one cell in world units (> 0).
    pub cell_size: f32,
    /// Cell coordinate placed at the world origin (floating origin).
    pub anchor: I64Vec2,
}

impl Default for GridSpace {
    fn default() -> Self {
        Self { cell_size: 1.0, anchor: I64Vec2::ZERO }
    }
}

impl GridSpace {
    /// Creates a space with the given cell size (non-positive or non-finite
    /// sizes fall back to 1.0) and the anchor at cell `(0, 0)`.
    pub fn new(cell_size: f32) -> Self {
        let cell_size = if cell_size.is_finite() && cell_size > 0.0 { cell_size } else { 1.0 };
        Self { cell_size, anchor: I64Vec2::ZERO }
    }

    /// World position of the top-left corner of a cell. Offsets from the
    /// anchor saturate at the `i64` range instead of overflowing.
    pub fn cell_corner_to_world(&self, cell: I64Vec2) -> Vec2 {
        let d = cell.saturating_sub(self.anchor);
        Vec2::new((d.x as f64 * self.cell_size as f64) as f32, (-(d.y as f64) * self.cell_size as f64) as f32)
    }

    /// World position of the centre of a cell.
    pub fn cell_center_to_world(&self, cell: I64Vec2) -> Vec2 {
        self.cell_corner_to_world(cell) + Vec2::new(0.5, -0.5) * self.cell_size
    }

    /// Fractional cell coordinates (x right, y down) of a world position.
    pub fn world_to_cell_f64(&self, world: Vec2) -> (f64, f64) {
        let s = self.cell_size as f64;
        (self.anchor.x as f64 + world.x as f64 / s, self.anchor.y as f64 - world.y as f64 / s)
    }

    /// Cell containing a world position.
    pub fn world_to_cell(&self, world: Vec2) -> I64Vec2 {
        let (x, y) = self.world_to_cell_f64(world);
        I64Vec2::new(saturating_floor(x), saturating_floor(y))
    }

    /// Cells visible through an orthographic 2D camera centred on
    /// `camera_world` with `scale` world units per logical pixel and a
    /// viewport of `viewport_px` logical pixels.
    pub fn visible_cells(&self, camera_world: Vec2, scale: f32, viewport_px: Vec2) -> VisibleCells {
        let half = viewport_px.max(Vec2::ZERO) * scale * 0.5;
        // Top-left and bottom-right world corners (world y up).
        let (min_x, min_y) = self.world_to_cell_f64(Vec2::new(camera_world.x - half.x, camera_world.y + half.y));
        let (max_x, max_y) = self.world_to_cell_f64(Vec2::new(camera_world.x + half.x, camera_world.y - half.y));
        // A right/bottom edge exactly on a cell boundary does not show that cell.
        let last = |v: f64| {
            let f = v.floor();
            if f == v { saturating_floor(v) - 1 } else { saturating_floor(v) }
        };
        let min_x = saturating_floor(min_x);
        let min_y = saturating_floor(min_y);
        VisibleCells {
            min_x,
            min_y,
            max_x: last(max_x).max(min_x),
            max_y: last(max_y).max(min_y),
            cells_per_pixel: scale / self.cell_size,
        }
    }
}

fn saturating_floor(v: f64) -> i64 {
    // `as` saturates for out-of-range floats and maps NaN to 0.
    v.floor() as i64
}

/// The inclusive cell rectangle covered by the 2D world camera this frame,
/// plus the zoom level. Modes use it to request only the visible region and
/// to choose a level of detail.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct VisibleCells {
    /// Leftmost visible cell column.
    pub min_x: i64,
    /// Topmost visible cell row.
    pub min_y: i64,
    /// Rightmost visible cell column (inclusive).
    pub max_x: i64,
    /// Bottom visible cell row (inclusive).
    pub max_y: i64,
    /// Cells per logical pixel (> 1 means several cells share one pixel).
    pub cells_per_pixel: f32,
}

impl VisibleCells {
    /// Number of visible columns (saturating at `u64::MAX`).
    pub fn width(&self) -> u64 {
        self.max_x.abs_diff(self.min_x).saturating_add(1)
    }

    /// Number of visible rows (saturating at `u64::MAX`).
    pub fn height(&self) -> u64 {
        self.max_y.abs_diff(self.min_y).saturating_add(1)
    }

    /// Whether a cell is inside the visible rectangle.
    pub fn contains(&self, cell: I64Vec2) -> bool {
        (self.min_x..=self.max_x).contains(&cell.x) && (self.min_y..=self.max_y).contains(&cell.y)
    }

    /// The rectangle grown by `margin` cells on every side (saturating).
    pub fn expanded(&self, margin: i64) -> Self {
        Self {
            min_x: self.min_x.saturating_sub(margin),
            min_y: self.min_y.saturating_sub(margin),
            max_x: self.max_x.saturating_add(margin),
            max_y: self.max_y.saturating_add(margin),
            ..*self
        }
    }

    /// Suggested level of detail as a power of two: `log2(cells_per_texel)`
    /// such that one texel is no smaller than one pixel. 0 means exact cells.
    pub fn lod_log2(&self) -> u32 {
        if self.cells_per_pixel.is_nan() || self.cells_per_pixel <= 1.0 {
            return 0;
        }
        (self.cells_per_pixel.log2().ceil() as u32).min(62)
    }

    /// Texel-aligned frame region for `cells_per_texel` (power of two or
    /// not): returns `(origin, width_texels, height_texels)` where `origin`
    /// is a multiple of `cells_per_texel`, so texel boundaries stay fixed in
    /// cell space while the camera pans (no shimmering). The region covers
    /// every visible cell.
    pub fn aligned_region(&self, cells_per_texel: u32) -> (I64Vec2, u32, u32) {
        let k = i64::from(cells_per_texel.max(1));
        let (tx0, ty0) = (self.min_x.div_euclid(k), self.min_y.div_euclid(k));
        let (tx1, ty1) = (self.max_x.div_euclid(k), self.max_y.div_euclid(k));
        let span = |a: i64, b: i64| b.saturating_sub(a).saturating_add(1).clamp(1, u32::MAX as i64) as u32;
        (I64Vec2::new(tx0.saturating_mul(k), ty0.saturating_mul(k)), span(tx0, tx1), span(ty0, ty1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_and_center_mapping_flips_y() {
        let space = GridSpace::new(2.0);
        assert_eq!(space.cell_corner_to_world(I64Vec2::new(3, 4)), Vec2::new(6.0, -8.0));
        assert_eq!(space.cell_center_to_world(I64Vec2::new(3, 4)), Vec2::new(7.0, -9.0));
        assert_eq!(space.world_to_cell(Vec2::new(7.0, -9.0)), I64Vec2::new(3, 4));
        // Just inside the top-left corner and just inside the bottom-right corner.
        assert_eq!(space.world_to_cell(Vec2::new(6.001, -8.001)), I64Vec2::new(3, 4));
        assert_eq!(space.world_to_cell(Vec2::new(7.999, -9.999)), I64Vec2::new(3, 4));
        assert_eq!(space.world_to_cell(Vec2::new(-0.5, 0.5)), I64Vec2::new(-1, -1));
    }

    #[test]
    fn anchor_round_trips_huge_coordinates() {
        let far = I64Vec2::new(1_000_000_000_007, -9_000_000_000_001);
        let space = GridSpace { cell_size: 1.0, anchor: far - I64Vec2::new(10, 20) };
        let world = space.cell_center_to_world(far);
        assert_eq!(world, Vec2::new(10.5, -20.5));
        assert_eq!(space.world_to_cell(world), far);
    }

    #[test]
    fn invalid_cell_size_falls_back() {
        assert_eq!(GridSpace::new(0.0).cell_size, 1.0);
        assert_eq!(GridSpace::new(f32::NAN).cell_size, 1.0);
        assert_eq!(GridSpace::new(-3.0).cell_size, 1.0);
    }

    #[test]
    fn visible_cells_cover_viewport_exactly() {
        let space = GridSpace::new(1.0);
        // 100x50 px viewport, 1 world unit per px, centred at (50, -25):
        // world x in [0, 100], y in [-50, 0] → cells x 0..=99, y 0..=49.
        let v = space.visible_cells(Vec2::new(50.0, -25.0), 1.0, Vec2::new(100.0, 50.0));
        assert_eq!((v.min_x, v.min_y, v.max_x, v.max_y), (0, 0, 99, 49));
        assert_eq!((v.width(), v.height()), (100, 50));
        assert_eq!(v.cells_per_pixel, 1.0);
        // Half a cell shift exposes one more column on each side.
        let v = space.visible_cells(Vec2::new(50.5, -25.0), 1.0, Vec2::new(100.0, 50.0));
        assert_eq!((v.min_x, v.max_x), (0, 100));
    }

    #[test]
    fn visible_cells_zoomed_out() {
        let space = GridSpace::new(4.0);
        let v = space.visible_cells(Vec2::ZERO, 16.0, Vec2::new(10.0, 10.0));
        // 160 world units = 40 cells wide, centred on the origin.
        assert_eq!((v.min_x, v.max_x), (-20, 19));
        assert_eq!((v.min_y, v.max_y), (-20, 19));
        assert_eq!(v.cells_per_pixel, 4.0);
        assert_eq!(v.lod_log2(), 2);
    }

    #[test]
    fn lod_is_zero_when_zoomed_in() {
        let v = VisibleCells { cells_per_pixel: 0.25, ..Default::default() };
        assert_eq!(v.lod_log2(), 0);
        let v = VisibleCells { cells_per_pixel: 1.0, ..Default::default() };
        assert_eq!(v.lod_log2(), 0);
        let v = VisibleCells { cells_per_pixel: 5.0, ..Default::default() };
        assert_eq!(v.lod_log2(), 3);
        let v = VisibleCells { cells_per_pixel: f32::NAN, ..Default::default() };
        assert_eq!(v.lod_log2(), 0);
    }

    #[test]
    fn aligned_region_covers_visible_cells() {
        let v = VisibleCells { min_x: -5, min_y: 3, max_x: 12, max_y: 9, cells_per_pixel: 4.0 };
        let (origin, w, h) = v.aligned_region(4);
        assert_eq!(origin, I64Vec2::new(-8, 0));
        // Texels cover x in [-8, 16) and y in [0, 12).
        assert_eq!((w, h), (6, 3));
        assert!(origin.x <= v.min_x && origin.x + 4 * w as i64 > v.max_x);
        assert!(origin.y <= v.min_y && origin.y + 4 * h as i64 > v.max_y);
        let (origin, w, h) = v.aligned_region(1);
        assert_eq!((origin, w, h), (I64Vec2::new(-5, 3), 18, 7));
    }

    #[test]
    fn contains_and_expand() {
        let v = VisibleCells { min_x: 0, min_y: 0, max_x: 9, max_y: 9, cells_per_pixel: 1.0 };
        assert!(v.contains(I64Vec2::new(9, 0)));
        assert!(!v.contains(I64Vec2::new(10, 0)));
        let e = v.expanded(2);
        assert_eq!((e.min_x, e.max_y), (-2, 11));
        let huge = VisibleCells { min_x: i64::MIN, max_x: i64::MAX, ..v };
        assert_eq!(huge.expanded(5).min_x, i64::MIN);
        assert_eq!(huge.width(), u64::MAX, "saturates instead of overflowing");
    }

    #[test]
    fn extreme_cells_do_not_overflow() {
        let space = GridSpace { cell_size: 1.0, anchor: I64Vec2::new(i64::MIN, i64::MAX) };
        let world = space.cell_corner_to_world(I64Vec2::new(i64::MAX, i64::MIN));
        assert!(world.x > 9.0e18 && world.y > 9.0e18, "{world:?}");
    }
}
