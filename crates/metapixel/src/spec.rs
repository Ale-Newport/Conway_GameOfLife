//! The validated numbers of Brice Due's OTCA metapixel (2006).
//!
//! Every constant here comes from `docs/research/otca-metapixel.md` and its
//! machine-readable twin `assets/patterns/otca/otca-spec.json`, which were
//! established by running Golly's own engine (`bgolly`) on the original
//! tiles and on Golly's metapixel demos. The test suite cross-checks this
//! module against the JSON file.
//!
//! **Tile-nominal coordinates.** A tile's nominal square is `[0, 2047]²`;
//! its RLE files are 2058×2058 and load with their top-left at `(0, 0)`, so
//! *file coordinate = nominal + 5*. Tiles are OR-ed at pitch 2048, so each
//! one overlaps its neighbours' border region by 5 cells.

use serde::{Deserialize, Serialize};

/// Generations of one metageneration (2⁹·3·23).
pub const PERIOD: u64 = 35_328;
/// Distance between the nominal origins of neighbouring tiles.
pub const TILE_PITCH: i64 = 2048;
/// `log2(TILE_PITCH)`: a tile's nominal square is one level-11 quadtree node.
pub const TILE_LOG2: u32 = 11;
/// RLE file coordinate minus tile-nominal coordinate.
pub const FILE_OFFSET: i64 = 5;
/// Inclusive tile-nominal bounding box of an unprogrammed tile.
pub const TILE_BOUNDING_BOX: CellRect = CellRect { x0: -5, y0: -5, x1: 2052, y1: 2052 };
/// Live cells of the unprogrammed OFF tile (`OTCAmetapixel-OFF.rle`).
pub const POPULATION_OFF_UNPROGRAMMED: usize = 23_920;
/// Live cells of the unprogrammed ON tile (`OTCAmetapixel-ON.rle`).
pub const POPULATION_ON_UNPROGRAMMED: usize = 66_203;

/// Nominal readout rectangle ("core" of the display), inclusive.
pub const READOUT_RECT: CellRect = CellRect { x0: 250, y0: 250, x1: 1750, y1: 1750 };
/// Core population of an ON tile at `t ≡ 0 (mod 46)`, in particular at every
/// readout time `t = k·P`.
pub const READOUT_ON_COUNT: u64 = 29_795;
/// Smallest core population of a steady ON tile (any phase).
pub const READOUT_ON_STEADY_MIN: u64 = 21_677;
/// Period of the steady ON core population.
pub const READOUT_ON_PERIOD: u64 = 46;
/// First phase `t mod P` (for `t ≥ P`) at which a core may leave its steady
/// state.
pub const TRANSITION_START: u64 = 4_320;
/// Last phase `t mod P` (for `t ≥ P`) of the core transition window.
pub const TRANSITION_END: u64 = 7_407;

/// An inclusive rectangle of cells `[x0, x1] × [y0, y1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellRect {
    /// Left column.
    pub x0: i64,
    /// Top row.
    pub y0: i64,
    /// Right column (inclusive).
    pub x1: i64,
    /// Bottom row (inclusive).
    pub y1: i64,
}

impl CellRect {
    /// The rectangle shifted by `(dx, dy)` (saturating).
    pub fn translated(self, dx: i64, dy: i64) -> Self {
        Self {
            x0: self.x0.saturating_add(dx),
            y0: self.y0.saturating_add(dy),
            x1: self.x1.saturating_add(dx),
            y1: self.y1.saturating_add(dy),
        }
    }

    /// Whether `(x, y)` lies inside.
    pub fn contains(self, x: i64, y: i64) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }

    /// Width in cells (0 for an empty rectangle).
    pub fn width(self) -> u64 {
        if self.x1 < self.x0 { 0 } else { self.x1.abs_diff(self.x0) + 1 }
    }

    /// Height in cells (0 for an empty rectangle).
    pub fn height(self) -> u64 {
        if self.y1 < self.y0 { 0 } else { self.y1.abs_diff(self.y0) + 1 }
    }
}

/// The eight Moore directions (y grows downward, so `N` is `(0, -1)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// North, `(0, -1)`.
    N,
    /// North-east, `(1, -1)`.
    NE,
    /// East, `(1, 0)`.
    E,
    /// South-east, `(1, 1)`.
    SE,
    /// South, `(0, 1)`.
    S,
    /// South-west, `(-1, 1)`.
    SW,
    /// West, `(-1, 0)`.
    W,
    /// North-west, `(-1, -1)`.
    NW,
}

impl Direction {
    /// All directions, clockwise from north (also the bit order of
    /// [`crate::Neighbourhood`]).
    pub const ALL: [Direction; 8] = [Self::N, Self::NE, Self::E, Self::SE, Self::S, Self::SW, Self::W, Self::NW];

    /// Bit index in [`crate::Neighbourhood`] masks.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Offset `(dx, dy)` of the neighbour in this direction.
    pub fn offset(self) -> (i64, i64) {
        match self {
            Self::N => (0, -1),
            Self::NE => (1, -1),
            Self::E => (1, 0),
            Self::SE => (1, 1),
            Self::S => (0, 1),
            Self::SW => (-1, 1),
            Self::W => (-1, 0),
            Self::NW => (-1, -1),
        }
    }

    /// The opposite direction.
    pub fn opposite(self) -> Self {
        Self::ALL[(self.index() + 4) % 8]
    }

    /// Whether this is a diagonal direction.
    pub fn is_corner(self) -> bool {
        self.index() % 2 == 1
    }

    /// Upper-case name (`"NE"`).
    pub fn name(self) -> &'static str {
        match self {
            Self::N => "N",
            Self::NE => "NE",
            Self::E => "E",
            Self::SE => "SE",
            Self::S => "S",
            Self::SW => "SW",
            Self::W => "W",
            Self::NW => "NW",
        }
    }

    /// Parses a direction name, ignoring case.
    pub fn parse(text: &str) -> Option<Self> {
        let upper = text.trim().to_ascii_uppercase();
        Self::ALL.into_iter().find(|d| d.name() == upper)
    }

    /// The direction a tile becomes **deaf to** when the fuse object named
    /// by `self` is removed from every tile (research note §4.2): an edge
    /// block opens the neighbour's output toward this tile, so removing it
    /// silences that same edge; a corner LWSS gates the tile's *own* output
    /// toward that corner, so removing it silences the opposite corner.
    pub fn deaf_when_fuse_removed(self) -> Self {
        if self.is_corner() { self.opposite() } else { self }
    }

    /// The fuse object to remove so that every tile stops hearing its
    /// neighbour in direction `self` (inverse of
    /// [`Direction::deaf_when_fuse_removed`]).
    pub fn fuse_silencing(self) -> Self {
        if self.is_corner() { self.opposite() } else { self }
    }

    /// The tile-nominal cells of the neighbourhood fuse object named by this
    /// direction: the corner LWSS for diagonals, the edge block otherwise.
    pub fn fuse_cells(self) -> &'static [(i64, i64)] {
        match self {
            Self::NW => &[(8, 1), (8, 3), (9, 0), (10, 0), (11, 0), (11, 3), (12, 0), (12, 1), (12, 2)],
            Self::NE => &[
                (2044, 8),
                (2044, 11),
                (2045, 12),
                (2046, 8),
                (2046, 12),
                (2047, 9),
                (2047, 10),
                (2047, 11),
                (2047, 12),
            ],
            Self::SE => &[
                (2035, 2045),
                (2035, 2046),
                (2035, 2047),
                (2036, 2044),
                (2036, 2047),
                (2037, 2047),
                (2038, 2047),
                (2039, 2044),
                (2039, 2046),
            ],
            Self::SW => {
                &[(0, 2035), (0, 2036), (0, 2037), (0, 2038), (1, 2035), (1, 2039), (2, 2035), (3, 2036), (3, 2039)]
            }
            Self::N => &[(1001, 0), (1001, 1), (1002, 0), (1002, 1)],
            Self::E => &[(2046, 1001), (2046, 1002), (2047, 1001), (2047, 1002)],
            Self::S => &[(1045, 2046), (1045, 2047), (1046, 2046), (1046, 2047)],
            Self::W => &[(0, 1045), (0, 1046), (1, 1045), (1, 1046)],
        }
    }
}

/// Offsets `o[8 − d]` of the rule-table rows (research note §4.1).
const RULE_ROW_OFFSETS: [i64; 9] = [0, 10, 20, 46, 56, 66, 92, 102, 112];

/// Row offset of digit `d` (0..=8) in the rule table.
fn rule_row_offset(digit: u8) -> i64 {
    RULE_ROW_OFFSETS[8 - usize::from(digit.min(8))]
}

/// Which half of the rule table a programming cell belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RuleTable {
    /// Birth table (`B` column).
    Birth,
    /// Survival table (`S` column).
    Survival,
}

impl RuleTable {
    /// `"B"` or `"S"`.
    pub fn letter(self) -> char {
        match self {
            Self::Birth => 'B',
            Self::Survival => 'S',
        }
    }
}

/// Tile-nominal cell that completes the broken eater of digit `digit`
/// (clamped to 0..=8) in `table`. Adding it puts the digit in the rule:
/// `B_d = (148, 1404 + o)`, `S_d = (162, 1406 + o)` with
/// `o = [0, 10, 20, 46, 56, 66, 92, 102, 112][8 − d]`.
pub fn rule_bit_cell(table: RuleTable, digit: u8) -> (i64, i64) {
    let o = rule_row_offset(digit);
    match table {
        RuleTable::Birth => (148, 1404 + o),
        RuleTable::Survival => (162, 1406 + o),
    }
}

/// Inclusive tile-nominal footprint of the eater of digit `digit` in
/// `table` (the broken eater plus its completion cell). The birth eater has
/// its origin at `(145, 1401 + o)`; the survival eater is the 180° rotated
/// copy with origin `(165, 1409 + o)` (the research note writes it as
/// `(165, 1521 − o')` with `o' = 112 − o`).
pub fn rule_eater_rect(table: RuleTable, digit: u8) -> CellRect {
    let o = rule_row_offset(digit);
    match table {
        RuleTable::Birth => CellRect { x0: 145, y0: 1401 + o, x1: 148, y1: 1404 + o },
        RuleTable::Survival => CellRect { x0: 162, y0: 1406 + o, x1: 165, y1: 1409 + o },
    }
}

/// Metageneration displayed at generation `t`:
/// `m(t) = max(0, ⌊(t − 7408) / P⌋)` (valid outside the transition windows).
pub fn displayed_metagen(t: u64) -> u64 {
    t.saturating_sub(TRANSITION_END + 1) / PERIOD
}

/// Whether the readout cores may be changing at generation `t`
/// (`t ≥ P` and `t mod P ∈ [4320, 7407]`). The first cycle never changes the
/// display.
pub fn in_transition_window(t: u64) -> bool {
    t >= PERIOD && (TRANSITION_START..=TRANSITION_END).contains(&(t % PERIOD))
}

/// Generation at which metageneration `n` is cleanly readable:
/// `(n + 1)·P`, or `None` on overflow.
pub fn readout_generation(metagen: u64) -> Option<u64> {
    metagen.checked_add(1)?.checked_mul(PERIOD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_bits_follow_the_published_table() {
        assert_eq!(rule_bit_cell(RuleTable::Birth, 8), (148, 1404));
        assert_eq!(rule_bit_cell(RuleTable::Birth, 3), (148, 1470));
        assert_eq!(rule_bit_cell(RuleTable::Birth, 0), (148, 1516));
        assert_eq!(rule_bit_cell(RuleTable::Survival, 2), (162, 1498));
        assert_eq!(rule_bit_cell(RuleTable::Survival, 3), (162, 1472));
        assert_eq!(rule_bit_cell(RuleTable::Survival, 0), (162, 1518));
        for d in 0..=8 {
            let (bx, by) = rule_bit_cell(RuleTable::Birth, d);
            let rect = rule_eater_rect(RuleTable::Birth, d);
            assert_eq!((rect.x1, rect.y1), (bx, by), "the completion cell is the eater's far corner");
            let (sx, sy) = rule_bit_cell(RuleTable::Survival, d);
            let rect = rule_eater_rect(RuleTable::Survival, d);
            assert_eq!((rect.x0, rect.y0), (sx, sy));
        }
    }

    #[test]
    fn deafness_mapping_is_an_involution_matching_the_table() {
        let expected = [("NW", "SE"), ("NE", "SW"), ("SE", "NW"), ("SW", "NE"), ("N", "N"), ("E", "E"), ("S", "S")];
        for (fuse, deaf) in expected {
            let fuse = Direction::parse(fuse).unwrap();
            assert_eq!(fuse.deaf_when_fuse_removed().name(), deaf);
            assert_eq!(fuse.deaf_when_fuse_removed().fuse_silencing(), fuse);
        }
        for d in Direction::ALL {
            assert_eq!(d.opposite().opposite(), d);
            let (dx, dy) = d.offset();
            assert_eq!(d.opposite().offset(), (-dx, -dy));
            assert_eq!(d.fuse_cells().len(), if d.is_corner() { 9 } else { 4 });
        }
    }

    #[test]
    fn displayed_metagen_formula() {
        assert_eq!(displayed_metagen(0), 0);
        assert_eq!(displayed_metagen(PERIOD + 7407), 0);
        assert_eq!(displayed_metagen(PERIOD + 7408), 1);
        assert_eq!(displayed_metagen(2 * PERIOD), 1);
        assert!(!in_transition_window(4320), "the first cycle never changes the display");
        assert!(in_transition_window(PERIOD + 4320));
        assert!(in_transition_window(PERIOD + 7407));
        assert!(!in_transition_window(PERIOD + 7408));
        assert_eq!(readout_generation(0), Some(PERIOD));
        assert_eq!(readout_generation(u64::MAX), None);
    }
}
