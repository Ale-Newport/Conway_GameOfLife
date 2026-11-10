//! Loading Brice Due's unprogrammed OFF/ON tiles and programming them.
//!
//! The tiles are third-party pattern data distributed under
//! GPL-2.0-or-later (see `assets/patterns/otca/PROVENANCE.md`). They are
//! **not** embedded in the binary: [`TileSet::from_dir`] reads them at run
//! time and reports a clear error when they are missing.

use std::path::{Path, PathBuf};

use crate::error::MetapixelError;
use crate::programming::Programming;
use crate::spec::{
    Direction, FILE_OFFSET, POPULATION_OFF_UNPROGRAMMED, POPULATION_ON_UNPROGRAMMED, RuleTable, TILE_BOUNDING_BOX,
    rule_bit_cell,
};

/// File name of the unprogrammed OFF tile.
pub const OFF_TILE_FILE: &str = "OTCAmetapixel-OFF.rle";
/// File name of the unprogrammed ON tile.
pub const ON_TILE_FILE: &str = "OTCAmetapixel-ON.rle";

/// A macro cell state: which tile is placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TileState {
    /// The OFF tile.
    Off,
    /// The ON tile.
    On,
}

impl TileState {
    /// `On` for `true`.
    pub fn from_bool(on: bool) -> Self {
        if on { Self::On } else { Self::Off }
    }

    /// Label used in error messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::On => "ON",
        }
    }
}

/// Live cells of one tile in tile-nominal coordinates, sorted by `(y, x)`.
pub type TileCells = Vec<(i64, i64)>;

/// The two **unprogrammed** base tiles (all 18 rule eaters broken, all 8
/// fuses present), validated against the published populations and
/// bounding box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileSet {
    off: TileCells,
    on: TileCells,
}

impl TileSet {
    /// Directory of the shipped tiles in this source tree
    /// (`<workspace>/assets/patterns/otca`). Applications should prefer
    /// their own asset root.
    pub fn source_tree_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/patterns/otca")
    }

    /// Loads [`OFF_TILE_FILE`] and [`ON_TILE_FILE`] from `dir`.
    ///
    /// # Errors
    /// [`MetapixelError::TileFileMissing`] if a file cannot be read, and the
    /// errors of [`TileSet::from_rle`].
    pub fn from_dir(dir: &Path) -> Result<Self, MetapixelError> {
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read_to_string(&path).map_err(|e| MetapixelError::TileFileMissing {
                path: path.display().to_string(),
                message: e.to_string(),
            })
        };
        Self::from_rle(&read(OFF_TILE_FILE)?, &read(ON_TILE_FILE)?)
    }

    /// Parses the two unprogrammed tiles from RLE text (file coordinates;
    /// shifted by −5 to tile-nominal coordinates) and validates them.
    ///
    /// # Errors
    /// [`MetapixelError::TileParse`] for malformed RLE and
    /// [`MetapixelError::TileMismatch`] if a population, the bounding box or
    /// a programming site differs from the published construction (for
    /// example when a *programmed* tile is passed by mistake).
    pub fn from_rle(off_rle: &str, on_rle: &str) -> Result<Self, MetapixelError> {
        let set = Self { off: parse_tile(off_rle, TileState::Off)?, on: parse_tile(on_rle, TileState::On)? };
        set.validate()?;
        Ok(set)
    }

    /// Unprogrammed cells of `state` (tile-nominal, sorted by `(y, x)`).
    pub fn cells(&self, state: TileState) -> &[(i64, i64)] {
        match state {
            TileState::Off => &self.off,
            TileState::On => &self.on,
        }
    }

    fn validate(&self) -> Result<(), MetapixelError> {
        for (state, expected) in
            [(TileState::Off, POPULATION_OFF_UNPROGRAMMED), (TileState::On, POPULATION_ON_UNPROGRAMMED)]
        {
            let cells = self.cells(state);
            let mismatch = |message: String| MetapixelError::TileMismatch { tile: state.name().into(), message };
            if cells.len() != expected {
                return Err(mismatch(format!(
                    "population is {} but the unprogrammed tile has {expected} cells (is it a programmed copy?)",
                    cells.len()
                )));
            }
            let bbox = bounding_box(cells);
            let want = TILE_BOUNDING_BOX;
            if bbox != Some((want.x0, want.y0, want.x1, want.y1)) {
                return Err(mismatch(format!(
                    "bounding box is {bbox:?} in tile-nominal coordinates, expected ({}, {}, {}, {})",
                    want.x0, want.y0, want.x1, want.y1
                )));
            }
            for table in [RuleTable::Birth, RuleTable::Survival] {
                for digit in 0..=8 {
                    let cell = rule_bit_cell(table, digit);
                    if contains(cells, cell) {
                        return Err(mismatch(format!(
                            "rule cell {}{digit} at {cell:?} is already present: the base tile must be unprogrammed",
                            table.letter()
                        )));
                    }
                }
            }
            for fuse in Direction::ALL {
                if let Some(cell) = fuse.fuse_cells().iter().find(|&&c| !contains(cells, c)) {
                    return Err(mismatch(format!("{} fuse cell {cell:?} is missing", fuse.name())));
                }
            }
        }
        Ok(())
    }

    /// Programs both tiles identically: adds one cell per rule digit
    /// (completing that broken eater) and removes the fuse object of every
    /// direction the tiles must not hear.
    pub fn program(&self, programming: Programming) -> ProgrammedTiles {
        let program = |base: &[(i64, i64)]| {
            let mut cells: Vec<(i64, i64)> = base.to_vec();
            for digit in 0..=8u8 {
                if programming.rule.births(digit) {
                    cells.push(rule_bit_cell(RuleTable::Birth, digit));
                }
                if programming.rule.survives(digit) {
                    cells.push(rule_bit_cell(RuleTable::Survival, digit));
                }
            }
            let removed: Vec<(i64, i64)> = Direction::ALL
                .into_iter()
                .filter(|&d| !programming.neighbourhood.listens(d))
                .flat_map(|d| d.fuse_silencing().fuse_cells().iter().copied())
                .collect();
            cells.retain(|c| !removed.contains(c));
            sort_cells(&mut cells);
            cells
        };
        ProgrammedTiles { programming, off: program(&self.off), on: program(&self.on) }
    }
}

/// The OFF and ON tiles programmed for one [`Programming`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgrammedTiles {
    programming: Programming,
    off: TileCells,
    on: TileCells,
}

impl ProgrammedTiles {
    /// The programming both tiles carry.
    pub fn programming(&self) -> Programming {
        self.programming
    }

    /// Programmed cells of `state` (tile-nominal, sorted by `(y, x)`).
    pub fn cells(&self, state: TileState) -> &[(i64, i64)] {
        match state {
            TileState::Off => &self.off,
            TileState::On => &self.on,
        }
    }

    /// The programmed tile as RLE in *file* coordinates (nominal + 5), the
    /// same frame as the shipped `*-B3S23.rle` files.
    pub fn to_rle(&self, state: TileState) -> String {
        let pattern = patterns::Pattern::from_live_cells(
            self.cells(state).iter().map(|&(x, y)| (x + FILE_OFFSET, y + FILE_OFFSET)),
        )
        .with_rule("B3/S23");
        patterns::rle::write(&pattern)
    }
}

/// Parses one tile RLE (file coordinates) into sorted tile-nominal cells.
///
/// # Errors
/// [`MetapixelError::TileParse`] for malformed RLE and
/// [`MetapixelError::TileMismatch`] for a multi-state pattern.
pub fn parse_tile(text: &str, state: TileState) -> Result<TileCells, MetapixelError> {
    let pattern = patterns::rle::parse(text)
        .map_err(|e| MetapixelError::TileParse { tile: state.name().into(), message: e.to_string() })?;
    if pattern.max_state() > 1 {
        return Err(MetapixelError::TileMismatch {
            tile: state.name().into(),
            message: "the tile must be a two-state pattern".into(),
        });
    }
    let mut cells: TileCells = pattern.live_cells().map(|(x, y)| (x - FILE_OFFSET, y - FILE_OFFSET)).collect();
    sort_cells(&mut cells);
    cells.dedup();
    Ok(cells)
}

fn sort_cells(cells: &mut [(i64, i64)]) {
    cells.sort_unstable_by_key(|&(x, y)| (y, x));
}

fn contains(sorted: &[(i64, i64)], (x, y): (i64, i64)) -> bool {
    sorted.binary_search_by_key(&(y, x), |&(cx, cy)| (cy, cx)).is_ok()
}

fn bounding_box(cells: &[(i64, i64)]) -> Option<(i64, i64, i64, i64)> {
    let first = cells.first()?;
    Some(cells.iter().fold((first.0, first.1, first.0, first.1), |(x0, y0, x1, y1), &(x, y)| {
        (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
    }))
}
