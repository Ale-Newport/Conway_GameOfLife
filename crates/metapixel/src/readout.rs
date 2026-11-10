//! Reading macro states from the physical construction.
//!
//! A tile's state is read by counting live cells in its nominal readout
//! rectangle `[250, 1750]²` (research note §5). At `t = (n + 1)·P` the
//! count is exactly `0` (OFF) or `29 795` (ON) for every tile; anything else
//! means the timing or the construction is wrong, and is reported as
//! ambiguous rather than rounded to a state.

use hashlife::Universe;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::MetapixelError;
use crate::spec::{CellRect, READOUT_ON_COUNT, READOUT_ON_STEADY_MIN, READOUT_RECT, TILE_PITCH};

/// Placement of a rectangular grid of tiles in the plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileGeometry {
    /// Nominal x of tile `(0, 0)`'s top-left cell.
    pub origin_x: i64,
    /// Nominal y of tile `(0, 0)`'s top-left cell.
    pub origin_y: i64,
    /// Tiles per row.
    pub columns: u32,
    /// Rows of tiles.
    pub rows: u32,
}

impl TileGeometry {
    /// A grid whose tile `(0, 0)` starts at the origin (the layout produced by
    /// [`crate::build_universe`]).
    pub fn at_origin(columns: u32, rows: u32) -> Self {
        Self { origin_x: 0, origin_y: 0, columns, rows }
    }

    /// Number of tiles.
    pub fn tile_count(&self) -> usize {
        self.columns as usize * self.rows as usize
    }

    /// Checks that every tile of the grid has 64-bit cell coordinates (the
    /// readout functions call this before counting).
    ///
    /// # Errors
    /// [`MetapixelError::LimitExceeded`] if the last tile would end beyond
    /// `i64::MAX`.
    pub fn validate(&self) -> Result<(), MetapixelError> {
        let fits =
            |origin: i64, count: u32| count == 0 || origin.checked_add(TILE_PITCH * i64::from(count) - 1).is_some();
        if fits(self.origin_x, self.columns) && fits(self.origin_y, self.rows) {
            Ok(())
        } else {
            Err(MetapixelError::LimitExceeded(format!(
                "a {}×{} tile grid at ({}, {}) does not fit in 64-bit cell coordinates",
                self.columns, self.rows, self.origin_x, self.origin_y
            )))
        }
    }

    /// Nominal top-left cell of tile `(col, row)` (saturating; exact for
    /// every tile of a geometry that passes [`TileGeometry::validate`]).
    pub fn tile_origin(&self, col: u32, row: u32) -> (i64, i64) {
        (
            self.origin_x.saturating_add(TILE_PITCH * i64::from(col)),
            self.origin_y.saturating_add(TILE_PITCH * i64::from(row)),
        )
    }

    /// The nominal square of tile `(col, row)` (saturating, like
    /// [`TileGeometry::tile_origin`]).
    pub fn tile_rect(&self, col: u32, row: u32) -> CellRect {
        let (x, y) = self.tile_origin(col, row);
        CellRect { x0: x, y0: y, x1: x.saturating_add(TILE_PITCH - 1), y1: y.saturating_add(TILE_PITCH - 1) }
    }

    /// The readout rectangle of tile `(col, row)` in plane coordinates.
    pub fn readout_rect(&self, col: u32, row: u32) -> CellRect {
        let (x, y) = self.tile_origin(col, row);
        READOUT_RECT.translated(x, y)
    }

    /// The tile whose nominal square contains the cell `(x, y)`, if any.
    pub fn tile_at(&self, x: i64, y: i64) -> Option<(u32, u32)> {
        let col = x.checked_sub(self.origin_x)?.div_euclid(TILE_PITCH);
        let row = y.checked_sub(self.origin_y)?.div_euclid(TILE_PITCH);
        let col = u32::try_from(col).ok().filter(|&c| c < self.columns)?;
        let row = u32::try_from(row).ok().filter(|&r| r < self.rows)?;
        Some((col, row))
    }

    /// Every nominal tile square, as one rectangle (saturating, like
    /// [`TileGeometry::tile_origin`]).
    pub fn bounds(&self) -> CellRect {
        CellRect {
            x0: self.origin_x,
            y0: self.origin_y,
            x1: self.origin_x.saturating_add(TILE_PITCH * i64::from(self.columns) - 1),
            y1: self.origin_y.saturating_add(TILE_PITCH * i64::from(self.rows) - 1),
        }
    }

    fn check_tile(&self, col: u32, row: u32) -> Result<(), MetapixelError> {
        if col < self.columns && row < self.rows {
            Ok(())
        } else {
            Err(MetapixelError::TileOutOfRange {
                col: i64::from(col),
                row: i64::from(row),
                columns: self.columns,
                rows: self.rows,
            })
        }
    }
}

/// Classification of a core count taken at a readout time `t = k·P`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TileReading {
    /// Empty core.
    Off,
    /// Core population exactly 29 795.
    On,
    /// Any other count (wrong timing or a broken construction).
    Ambiguous(u64),
}

impl TileReading {
    /// Strict classification for readout times `t = k·P`.
    pub fn classify(count: u64) -> Self {
        match count {
            0 => Self::Off,
            READOUT_ON_COUNT => Self::On,
            other => Self::Ambiguous(other),
        }
    }

    /// `Some(true)` for ON, `Some(false)` for OFF, `None` if ambiguous.
    pub fn state(self) -> Option<bool> {
        match self {
            Self::Off => Some(false),
            Self::On => Some(true),
            Self::Ambiguous(_) => None,
        }
    }
}

/// Threshold classification of a core count at an arbitrary generation
/// (for live display between readout times).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LiveReading {
    /// Empty core.
    Off,
    /// At least 21 677 live cells: a steady ON display (any phase).
    On,
    /// In between: the display is switching.
    Transitioning(u64),
}

impl LiveReading {
    /// `0` → OFF, `≥ 21 677` → ON, otherwise transitioning.
    pub fn classify(count: u64) -> Self {
        match count {
            0 => Self::Off,
            c if c >= READOUT_ON_STEADY_MIN => Self::On,
            other => Self::Transitioning(other),
        }
    }
}

/// Dyadic decomposition of `[lo, hi]` into aligned blocks
/// `[start, start + 2^k)`, as `(start, k)` pairs in increasing order.
fn dyadic_blocks(lo: i64, hi: i64) -> Vec<(i64, u32)> {
    let mut blocks = Vec::new();
    let (mut start, end) = (i128::from(lo), i128::from(hi) + 1);
    while start < end {
        let mut k = if start == 0 { 62 } else { start.trailing_zeros().min(62) };
        while start + (1i128 << k) > end {
            k -= 1;
        }
        // `start` stays within i64 because it never exceeds `hi`.
        blocks.push((start as i64, k));
        start += 1i128 << k;
    }
    blocks
}

/// Largest pixel exponent used in one rectangle query (keeps per-pixel
/// counts far below `u32::MAX`).
const MAX_QUERY_LOG2: u32 = 15;
/// Largest rectangle side [`count_live_in_rect`] accepts.
pub const MAX_COUNT_SIDE: u64 = 1 << 20;

/// Exact number of live cells in the inclusive rectangle `rect`.
///
/// The rectangle is split into products of aligned dyadic intervals; each
/// product is one [`Universe::density_grid`] call with aligned origin, so the
/// cost is proportional to the quadtree nodes that touch the rectangle's
/// border, not to its area or population.
///
/// # Errors
/// [`MetapixelError::LimitExceeded`] if a side exceeds [`MAX_COUNT_SIDE`].
pub fn count_live_in_rect(universe: &Universe, rect: CellRect) -> Result<u64, MetapixelError> {
    if rect.width() == 0 || rect.height() == 0 {
        return Ok(0);
    }
    if rect.width() > MAX_COUNT_SIDE || rect.height() > MAX_COUNT_SIDE {
        return Err(MetapixelError::LimitExceeded(format!(
            "rectangle counts are limited to {MAX_COUNT_SIDE} cells per side (got {}×{})",
            rect.width(),
            rect.height()
        )));
    }
    let xs = dyadic_blocks(rect.x0, rect.x1);
    let ys = dyadic_blocks(rect.y0, rect.y1);
    let mut total = 0u64;
    for &(x, a) in &xs {
        for &(y, b) in &ys {
            let k = a.min(b).min(MAX_QUERY_LOG2);
            let (w, h) = (1usize << (a - k), 1usize << (b - k));
            let grid = universe.density_grid(x, y, w, h, k)?;
            total += grid.iter().map(|&c| u64::from(c)).sum::<u64>();
        }
    }
    Ok(total)
}

/// Live cells in the readout rectangle of tile `(col, row)`.
///
/// # Errors
/// [`MetapixelError::TileOutOfRange`] for a tile outside `geometry` and
/// [`MetapixelError::LimitExceeded`] for a geometry that fails
/// [`TileGeometry::validate`].
pub fn core_count(universe: &Universe, geometry: &TileGeometry, col: u32, row: u32) -> Result<u64, MetapixelError> {
    geometry.validate()?;
    geometry.check_tile(col, row)?;
    count_live_in_rect(universe, geometry.readout_rect(col, row))
}

/// Core counts of every tile, row-major. Tiles are counted in parallel
/// (pure reads of the immutable universe; results are collected in order).
///
/// # Errors
/// As [`core_count`].
pub fn core_counts(universe: &Universe, geometry: &TileGeometry) -> Result<Vec<u64>, MetapixelError> {
    geometry.validate()?;
    let columns = geometry.columns.max(1);
    (0..geometry.tile_count())
        .into_par_iter()
        .map(|i| count_live_in_rect(universe, geometry.readout_rect(i as u32 % columns, i as u32 / columns)))
        .collect()
}

/// Live cells in each tile's nominal 2048×2048 square, row-major: one
/// level-11 density query (exact for any origin; O(visible nodes) when the
/// origin is a multiple of 2048).
///
/// # Errors
/// [`MetapixelError::LimitExceeded`] for a geometry that fails
/// [`TileGeometry::validate`] and [`MetapixelError::Hashlife`] for more
/// than [`hashlife::MAX_DENSITY_PIXELS`] tiles.
pub fn tile_populations(universe: &Universe, geometry: &TileGeometry) -> Result<Vec<u64>, MetapixelError> {
    geometry.validate()?;
    let (columns, rows) = (geometry.columns as usize, geometry.rows as usize);
    let grid = universe.density_grid(geometry.origin_x, geometry.origin_y, columns, rows, crate::spec::TILE_LOG2)?;
    // A 2048×2048 square holds at most 2^22 cells: no u32 count saturates.
    Ok(grid.into_iter().map(u64::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dyadic_blocks_cover_the_interval_exactly() {
        for (lo, hi) in [(250, 1750), (-7, 9), (0, 0), (5, 5), (-2048, -1), (2298, 3798)] {
            let blocks = dyadic_blocks(lo, hi);
            let mut next = lo;
            for &(start, k) in &blocks {
                assert_eq!(start, next);
                assert_eq!(start.rem_euclid(1 << k), 0, "aligned");
                next = start + (1 << k);
            }
            assert_eq!(next, hi + 1);
        }
        assert_eq!(dyadic_blocks(250, 1750).len(), 11);
    }

    #[test]
    fn rectangle_counts_match_brute_force() {
        let mut universe = Universe::conway();
        let mut rng = simulation_core::SimRng::seed_from_u64(3);
        let cells: Vec<(i64, i64)> =
            (0..2000).map(|_| (rng.below(300) as i64 - 150, rng.below(300) as i64 - 37)).collect();
        universe.insert_cells(cells.iter().copied());
        let mut unique = cells.clone();
        unique.sort_unstable();
        unique.dedup();
        for rect in [
            CellRect { x0: -150, y0: -37, x1: 149, y1: 262 },
            CellRect { x0: -13, y0: 5, x1: 77, y1: 6 },
            CellRect { x0: 3, y0: -30, x1: 3, y1: 200 },
            CellRect { x0: 10, y0: 10, x1: 9, y1: 20 },
        ] {
            let expected = unique.iter().filter(|&&(x, y)| rect.contains(x, y)).count() as u64;
            assert_eq!(count_live_in_rect(&universe, rect).unwrap(), expected, "{rect:?}");
        }
        let too_big = CellRect { x0: 0, y0: 0, x1: 1 << 30, y1: 1 };
        assert!(count_live_in_rect(&universe, too_big).is_err());
    }

    #[test]
    fn classification_thresholds() {
        assert_eq!(TileReading::classify(0), TileReading::Off);
        assert_eq!(TileReading::classify(29_795), TileReading::On);
        assert_eq!(TileReading::classify(29_794), TileReading::Ambiguous(29_794));
        assert_eq!(LiveReading::classify(21_677), LiveReading::On);
        assert_eq!(LiveReading::classify(21_676), LiveReading::Transitioning(21_676));
        let geometry = TileGeometry::at_origin(3, 2);
        assert_eq!(geometry.tile_at(2048 * 2 + 5, 2047), Some((2, 0)));
        assert_eq!(geometry.tile_at(-1, 0), None);
        assert_eq!(geometry.tile_at(0, 2048 * 2), None);
        assert_eq!(geometry.readout_rect(1, 0), CellRect { x0: 2298, y0: 250, x1: 3798, y1: 1750 });
    }

    #[test]
    fn counts_reject_tiles_and_geometries_they_cannot_read() {
        let universe = Universe::conway();
        let geometry = TileGeometry::at_origin(2, 2);
        assert_eq!(core_counts(&universe, &geometry).unwrap(), [0; 4]);
        assert!(matches!(core_count(&universe, &geometry, 2, 0), Err(MetapixelError::TileOutOfRange { .. })));
        let overflowing = TileGeometry { origin_x: i64::MAX - 4096, origin_y: 0, columns: 3, rows: 1 };
        assert!(matches!(overflowing.validate(), Err(MetapixelError::LimitExceeded(_))));
        assert!(core_counts(&universe, &overflowing).is_err());
        assert!(tile_populations(&universe, &overflowing).is_err());
        let edge = TileGeometry { origin_x: i64::MAX - 4095, origin_y: 0, columns: 2, rows: 1 };
        assert!(edge.validate().is_ok(), "the last cell may be i64::MAX");
        let huge = TileGeometry { origin_x: 0, origin_y: 0, columns: 1 << 14, rows: 1 << 14 };
        assert!(tile_populations(&universe, &huge).is_err(), "more tiles than one density query allows");
    }
}
