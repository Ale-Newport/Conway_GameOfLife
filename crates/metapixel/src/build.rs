//! Building the physical B3/S23 construction for a macro pattern.
//!
//! **Placement (research note §2.2, method A).** The programmed ON or OFF
//! tile of macro cell `(col, row)` is OR-ed into the plane with its nominal
//! origin at `(2048·col, 2048·row)`. Each 2058×2058 tile reaches 5 cells
//! into its neighbours, so the content of any 2048×2048 square depends on
//! the 3×3 block of macro cells around it (and on which of them exist).
//!
//! **Sharing.** Every distinct 3×3 neighbourhood is turned into a quadtree
//! square exactly once; equal squares and sub-squares are hash-consed, the
//! whole grid becomes a small DAG, and that DAG is handed to Hashlife as a
//! macrocell document. A 64×64 grid of 2.7·10⁸ cells costs a few dozen
//! distinct squares, never a cell-by-cell insertion.
//!
//! **Frame.** Golly's macrocell origin puts the root's south-east child at
//! `(0, 1)`. The document is therefore written one row *up* (each written
//! square holds nominal rows `2048·row + 1 ..= 2048·row + 2048`), so that
//! after `hashlife::macrocell::read` every tile lands exactly on its nominal
//! origin and on one level-11 node of the engine's quadtree.

use std::collections::BTreeMap;

use hashlife::Universe;
use serde::{Deserialize, Serialize};

use crate::error::MetapixelError;
use crate::macro_grid::MacroGrid;
use crate::quadtree::{DocBuilder, EMPTY};
use crate::spec::{TILE_LOG2, TILE_PITCH};
use crate::tile::{ProgrammedTiles, TileState};

/// What occupies one position of a 3×3 neighbourhood of macro cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Slot {
    /// No tile (outside the macro grid).
    Absent,
    /// A tile whose overlap into neighbouring squares does not depend on
    /// its state (used for the 8 outer positions when the OFF and ON rims
    /// are identical, which is the case for Brice Due's tiles).
    Present,
    /// A tile in a known state.
    Tile(TileState),
}

/// The 3×3 neighbourhood of a square, row-major from `(-1, -1)`.
type SquareKey = [Slot; 9];

/// Side of a square in leaves.
const LEAVES_PER_SIDE: usize = 256;

/// Statistics of one construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildStats {
    /// Distinct 2048×2048 squares (3×3 neighbourhood classes) built.
    pub distinct_squares: usize,
    /// Distinct non-empty quadtree nodes written to the macrocell document.
    pub document_nodes: usize,
    /// Size of the macrocell document in bytes.
    pub document_bytes: usize,
}

/// Builds the construction of `pattern` as a Hashlife universe at
/// generation 0, with tile `(col, row)` at nominal origin
/// `(2048·col, 2048·row)`.
///
/// # Errors
/// [`MetapixelError::Hashlife`] if the engine rejects the document (not
/// expected for grids within [`crate::MAX_GRID_SIDE`]).
pub fn build_universe(tiles: &ProgrammedTiles, pattern: &MacroGrid) -> Result<(Universe, BuildStats), MetapixelError> {
    let (text, stats) = construction_macrocell(tiles, pattern);
    let universe = hashlife::macrocell::read(&text)?;
    Ok((universe, stats))
}

/// The construction of `pattern` as Golly macrocell text (rule `B3/S23`,
/// generation 0) in Golly's coordinates, plus statistics.
pub fn construction_macrocell(tiles: &ProgrammedTiles, pattern: &MacroGrid) -> (String, BuildStats) {
    let mut squares = SquareBuilder::new(tiles);
    let (width, height) = (i64::from(pattern.width()), i64::from(pattern.height()));
    // Squares exist for columns -1..=width and rows -1..=height (the outer
    // ring only holds the 5-cell overlap of the border tiles). A root with
    // `half` squares per half side covers squares -half..half.
    let half = (width.max(height) + 1).unsigned_abs().next_power_of_two();
    let root_level = TILE_LOG2 as u8 + 1 + half.trailing_zeros() as u8;
    let half = half as i64;
    let slot = |col: i64, row: i64| {
        if (0..width).contains(&col) && (0..height).contains(&row) {
            Slot::Tile(TileState::from_bool(pattern.get_signed(col, row)))
        } else {
            Slot::Absent
        }
    };
    let mut grid = GridWalk { squares: &mut squares, slot, width, height, half };
    let root = grid.node(root_level, -half, -half);
    let header = format!(
        "[M2] (emergence-lab metapixel {})\n#R B3/S23\n#C OTCA metapixel construction by Brice Due (2006); {}×{} tiles, rule {} ({})\n",
        env!("CARGO_PKG_VERSION"),
        width,
        height,
        tiles.programming().rule,
        tiles.programming().neighbourhood,
    );
    let text = squares.builder.to_macrocell(root, &header);
    let stats = BuildStats {
        distinct_squares: squares.memo.len(),
        document_nodes: squares.builder.node_count(),
        document_bytes: text.len(),
    };
    (text, stats)
}

/// Recursive walk over the grid of squares, in units of squares.
struct GridWalk<'a, 't, F> {
    squares: &'a mut SquareBuilder<'t>,
    slot: F,
    width: i64,
    height: i64,
    half: i64,
}

impl<F: Fn(i64, i64) -> Slot> GridWalk<'_, '_, F> {
    /// Node of `level` whose top-left square is `(col, row)` (square units).
    fn node(&mut self, level: u8, col: i64, row: i64) -> u32 {
        let span = 1i64 << (level - TILE_LOG2 as u8);
        // Squares that can hold cells: -1..=width × -1..=height.
        if col > self.width || row > self.height || col + span <= -1 || row + span <= -1 {
            return EMPTY;
        }
        if level == TILE_LOG2 as u8 {
            let mut key = [Slot::Absent; 9];
            for (i, slot) in key.iter_mut().enumerate() {
                let (dx, dy) = (i as i64 % 3 - 1, i as i64 / 3 - 1);
                *slot = (self.slot)(col + dx, row + dy);
            }
            return self.squares.square(key);
        }
        let h = span / 2;
        debug_assert!(-self.half <= col && col + span <= self.half);
        let children = [
            self.node(level - 1, col, row),
            self.node(level - 1, col + h, row),
            self.node(level - 1, col, row + h),
            self.node(level - 1, col + h, row + h),
        ];
        self.squares.builder.join(level, children)
    }
}

/// Builds (once per distinct neighbourhood) the quadtree of a square.
struct SquareBuilder<'t> {
    tiles: &'t ProgrammedTiles,
    builder: DocBuilder,
    memo: BTreeMap<SquareKey, u32>,
    /// Cells of each tile state that can land in a neighbouring square.
    rims: [Vec<(i64, i64)>; 2],
    /// Whether the OFF and ON rims are identical (neighbour state irrelevant).
    rims_equal: bool,
    leaves: Vec<u64>,
}

impl<'t> SquareBuilder<'t> {
    fn new(tiles: &'t ProgrammedTiles) -> Self {
        // A cell at tile-nominal (x, y) lands in a neighbouring written
        // square iff x < 0, x > 2047, y < 1 or y > 2048 (rows shifted by one).
        let rim = |state| -> Vec<(i64, i64)> {
            tiles
                .cells(state)
                .iter()
                .copied()
                .filter(|&(x, y)| !(0..TILE_PITCH).contains(&x) || !(1..=TILE_PITCH).contains(&y))
                .collect()
        };
        let rims = [rim(TileState::Off), rim(TileState::On)];
        let rims_equal = rims[0] == rims[1];
        Self {
            tiles,
            builder: DocBuilder::new(),
            memo: BTreeMap::new(),
            rims,
            rims_equal,
            leaves: vec![0; LEAVES_PER_SIDE * LEAVES_PER_SIDE],
        }
    }

    fn normalise(&self, mut key: SquareKey) -> SquareKey {
        if self.rims_equal {
            for (i, slot) in key.iter_mut().enumerate() {
                if i != 4 && matches!(slot, Slot::Tile(_)) {
                    *slot = Slot::Present;
                }
            }
        }
        key
    }

    fn square(&mut self, key: SquareKey) -> u32 {
        if key.iter().all(|&s| s == Slot::Absent) {
            return EMPTY;
        }
        let key = self.normalise(key);
        if let Some(&id) = self.memo.get(&key) {
            return id;
        }
        self.leaves.fill(0);
        for (i, &slot) in key.iter().enumerate() {
            let (dx, dy) = (i as i64 % 3 - 1, i as i64 / 3 - 1);
            let cells: &[(i64, i64)] = match (slot, i == 4) {
                (Slot::Absent, _) => continue,
                (Slot::Tile(state), true) => self.tiles.cells(state),
                (Slot::Tile(state), false) => &self.rims[usize::from(state == TileState::On)],
                (Slot::Present, _) => &self.rims[0],
            };
            for &(cx, cy) in cells {
                // Written-square coordinates (one row up from nominal).
                let x = TILE_PITCH * dx + cx;
                let y = TILE_PITCH * dy + cy - 1;
                if (0..TILE_PITCH).contains(&x) && (0..TILE_PITCH).contains(&y) {
                    let (x, y) = (x as usize, y as usize);
                    self.leaves[(y / 8) * LEAVES_PER_SIDE + x / 8] |= 1 << ((y % 8) * 8 + x % 8);
                }
            }
        }
        let leaves = std::mem::take(&mut self.leaves);
        let id = self.builder.square_from_leaves(&leaves);
        self.leaves = leaves;
        self.memo.insert(key, id);
        id
    }
}
