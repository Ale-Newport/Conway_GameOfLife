//! Rectangular macro patterns: one ON/OFF bit per metapixel tile.

use std::fmt;

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::error::MetapixelError;

/// Largest macro grid side, in tiles. A 256×256 construction is
/// 524 288 cells wide; readout and memory costs grow with the tile count
/// (see `docs/OTCA_METAPIXEL.md`).
pub const MAX_GRID_SIDE: u32 = 256;

/// A `width × height` grid of macro cells (`true` = ON tile), row-major,
/// bit-packed. Column `col` and row `row` map to the tile whose nominal
/// origin is `(2048·col, 2048·row)`.
///
/// Deserialisation (snapshots) re-validates the size and the bit buffer, so
/// a corrupt file is rejected instead of producing an inconsistent grid.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawMacroGrid")]
pub struct MacroGrid {
    width: u32,
    height: u32,
    bits: Vec<u64>,
}

/// Unvalidated serialised form of [`MacroGrid`].
#[derive(Deserialize)]
struct RawMacroGrid {
    width: u32,
    height: u32,
    bits: Vec<u64>,
}

impl TryFrom<RawMacroGrid> for MacroGrid {
    type Error = MetapixelError;

    fn try_from(raw: RawMacroGrid) -> Result<Self, Self::Error> {
        check_size(u64::from(raw.width), u64::from(raw.height))?;
        let cells = raw.width as usize * raw.height as usize;
        let words = cells.div_ceil(64);
        if raw.bits.len() != words {
            return Err(MetapixelError::InvalidPattern(format!(
                "a {}×{} macro grid needs {words} words of cell bits, found {}",
                raw.width,
                raw.height,
                raw.bits.len()
            )));
        }
        let used = cells % 64;
        if used != 0 && raw.bits.last().is_some_and(|&w| w >> used != 0) {
            return Err(MetapixelError::InvalidPattern("bits beyond the last macro cell are set".into()));
        }
        Ok(Self { width: raw.width, height: raw.height, bits: raw.bits })
    }
}

impl MacroGrid {
    /// An all-OFF grid.
    ///
    /// # Errors
    /// [`MetapixelError::InvalidGridSize`] unless both sides are in
    /// `1..=MAX_GRID_SIDE`.
    pub fn new(width: u32, height: u32) -> Result<Self, MetapixelError> {
        check_size(u64::from(width), u64::from(height))?;
        let words = (width as usize * height as usize).div_ceil(64);
        Ok(Self { width, height, bits: vec![0; words] })
    }

    /// Parses rows of `.`/`O` (also `*`/`o` for ON and `b`/`-`/`_` for OFF);
    /// rows shorter than the longest are padded with OFF. Whitespace is not
    /// a macro cell (it is rejected rather than trimmed, so a stray space can
    /// never shift a row).
    ///
    /// # Errors
    /// [`MetapixelError::InvalidPattern`] for other characters or an empty
    /// pattern, [`MetapixelError::InvalidGridSize`] if too large.
    pub fn from_rows<S: AsRef<str>>(rows: &[S]) -> Result<Self, MetapixelError> {
        let width = rows.iter().map(|r| r.as_ref().chars().count()).max().unwrap_or(0);
        if width == 0 {
            return Err(MetapixelError::InvalidPattern("the pattern has no cells".into()));
        }
        check_size(width as u64, rows.len() as u64)?;
        let mut grid = Self::new(width as u32, rows.len() as u32)?;
        for (row, text) in rows.iter().enumerate() {
            for (col, ch) in text.as_ref().chars().enumerate() {
                let on = match ch {
                    'O' | 'o' | '*' => true,
                    '.' | 'b' | '-' | '_' => false,
                    other => {
                        return Err(MetapixelError::InvalidPattern(format!(
                            "row {}, column {}: `{other}` is not a macro cell (use `.` for OFF and `O` for ON)",
                            row + 1,
                            col + 1
                        )));
                    }
                };
                grid.set(col as u32, row as u32, on);
            }
        }
        Ok(grid)
    }

    /// A random grid: each tile is ON with probability `density`, drawn from
    /// `rng` in row-major order.
    ///
    /// # Errors
    /// As [`MacroGrid::new`], plus [`MetapixelError::InvalidPattern`] if
    /// `density` is not in `[0, 1]`.
    pub fn random(width: u32, height: u32, density: f64, rng: &mut SimRng) -> Result<Self, MetapixelError> {
        if !(0.0..=1.0).contains(&density) {
            return Err(MetapixelError::InvalidPattern(format!("density must be in [0, 1], got {density}")));
        }
        let mut grid = Self::new(width, height)?;
        for row in 0..height {
            for col in 0..width {
                grid.set(col, row, rng.chance(density));
            }
        }
        Ok(grid)
    }

    /// Width in tiles.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in tiles.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Number of tiles.
    pub fn len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Always false (grids have at least one tile); provided for API symmetry.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn index(&self, col: u32, row: u32) -> Option<usize> {
        (col < self.width && row < self.height).then(|| row as usize * self.width as usize + col as usize)
    }

    /// State of a tile (`false` outside the grid: the finite-grid semantics).
    pub fn get(&self, col: u32, row: u32) -> bool {
        self.index(col, row).is_some_and(|i| self.bits[i / 64] >> (i % 64) & 1 == 1)
    }

    /// State at signed coordinates (`false` outside the grid).
    pub fn get_signed(&self, col: i64, row: i64) -> bool {
        match (u32::try_from(col), u32::try_from(row)) {
            (Ok(c), Ok(r)) => self.get(c, r),
            _ => false,
        }
    }

    /// Sets a tile; coordinates outside the grid are ignored.
    pub fn set(&mut self, col: u32, row: u32, on: bool) {
        if let Some(i) = self.index(col, row) {
            let mask = 1u64 << (i % 64);
            if on {
                self.bits[i / 64] |= mask;
            } else {
                self.bits[i / 64] &= !mask;
            }
        }
    }

    /// Number of ON tiles.
    pub fn population(&self) -> u64 {
        self.bits.iter().map(|w| u64::from(w.count_ones())).sum()
    }

    /// ON tiles as `(col, row)`, row-major.
    pub fn on_tiles(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (0..self.height).flat_map(move |r| (0..self.width).filter(move |&c| self.get(c, r)).map(move |c| (c, r)))
    }

    /// Tiles where `self` and `other` differ, row-major (both grids must
    /// have the same size; otherwise every tile of the larger extent that
    /// differs under the outside-is-OFF convention is listed).
    pub fn differences(&self, other: &MacroGrid) -> Vec<(u32, u32)> {
        let (w, h) = (self.width.max(other.width), self.height.max(other.height));
        (0..h).flat_map(|r| (0..w).map(move |c| (c, r))).filter(|&(c, r)| self.get(c, r) != other.get(c, r)).collect()
    }

    /// A grid of a new size with this pattern anchored at the top-left
    /// (cropped or padded with OFF).
    ///
    /// # Errors
    /// As [`MacroGrid::new`].
    pub fn resized(&self, width: u32, height: u32) -> Result<Self, MetapixelError> {
        let mut grid = Self::new(width, height)?;
        for (c, r) in self.on_tiles() {
            grid.set(c, r, true);
        }
        Ok(grid)
    }

    /// Rows of `.`/`O`.
    pub fn to_rows(&self) -> Vec<String> {
        (0..self.height).map(|r| (0..self.width).map(|c| if self.get(c, r) { 'O' } else { '.' }).collect()).collect()
    }
}

impl fmt::Debug for MacroGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "MacroGrid {}×{}", self.width, self.height)?;
        for row in self.to_rows() {
            writeln!(f, "  {row}")?;
        }
        Ok(())
    }
}

fn check_size(width: u64, height: u64) -> Result<(), MetapixelError> {
    let ok = |side: u64| (1..=u64::from(MAX_GRID_SIDE)).contains(&side);
    if ok(width) && ok(height) {
        Ok(())
    } else {
        Err(MetapixelError::InvalidGridSize { width, height, max: MAX_GRID_SIDE })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_round_trip_and_bounds() {
        let grid = MacroGrid::from_rows(&["...", "OOO", ".*"]).unwrap();
        assert_eq!((grid.width(), grid.height()), (3, 3));
        assert_eq!(grid.to_rows(), ["...", "OOO", ".O."]);
        assert_eq!(grid.population(), 4);
        assert!(!grid.get_signed(-1, 1) && !grid.get(3, 1), "outside is OFF");
        assert_eq!(grid.on_tiles().collect::<Vec<_>>(), [(0, 1), (1, 1), (2, 1), (1, 2)]);
        assert!(MacroGrid::from_rows(&[".x."]).is_err());
        assert!(MacroGrid::from_rows(&[" O."]).is_err(), "whitespace is rejected, not trimmed");
        assert!(MacroGrid::from_rows::<&str>(&[]).is_err());
        assert!(MacroGrid::new(0, 3).is_err());
        assert!(MacroGrid::new(MAX_GRID_SIDE + 1, 3).is_err());
    }

    #[test]
    fn random_grids_are_seeded() {
        let a = MacroGrid::random(6, 6, 0.5, &mut SimRng::seed_from_u64(9)).unwrap();
        let b = MacroGrid::random(6, 6, 0.5, &mut SimRng::seed_from_u64(9)).unwrap();
        assert_eq!(a, b);
        assert!(MacroGrid::random(6, 6, 1.5, &mut SimRng::seed_from_u64(9)).is_err());
        assert_eq!(MacroGrid::random(4, 4, 1.0, &mut SimRng::seed_from_u64(1)).unwrap().population(), 16);
    }

    #[test]
    fn deserialisation_validates_the_bit_buffer() {
        let grid = MacroGrid::from_rows(&["O.O", ".O."]).unwrap();
        let text = ron::to_string(&grid).unwrap();
        assert_eq!(ron::from_str::<MacroGrid>(&text).unwrap(), grid);
        for corrupt in [
            "(width: 3, height: 3, bits: [])",
            "(width: 3, height: 3, bits: [1, 2])",
            "(width: 0, height: 3, bits: [])",
            "(width: 300, height: 1, bits: [0, 0, 0, 0, 0])",
            "(width: 3, height: 1, bits: [8])",
        ] {
            assert!(ron::from_str::<MacroGrid>(corrupt).is_err(), "{corrupt}");
        }
    }

    #[test]
    fn differences_and_resizing() {
        let a = MacroGrid::from_rows(&["O.", ".O"]).unwrap();
        let b = MacroGrid::from_rows(&["OO", ".."]).unwrap();
        assert_eq!(a.differences(&b), [(1, 0), (1, 1)]);
        let big = a.resized(3, 1).unwrap();
        assert_eq!(big.to_rows(), ["O.."]);
    }
}
