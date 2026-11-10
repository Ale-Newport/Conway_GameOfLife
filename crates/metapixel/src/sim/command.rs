//! Commands accepted by [`super::MetapixelSim`].

use crate::macro_grid::MacroGrid;

/// Edits of the **macro pattern**. Every command rebuilds the physical
/// construction at generation 0 (and clears the readout history), because
/// changing tiles of a running construction is not meaningful: tiles must
/// share the same phase and their fuses burn during the first cycle
/// (research note §2.3). The active rule, neighbourhood and size are kept;
/// pending parameter changes still wait for a reset.
#[derive(Clone, Debug, PartialEq)]
pub enum MetapixelCommand {
    /// Sets one macro cell.
    SetTile {
        /// Column.
        col: u32,
        /// Row.
        row: u32,
        /// ON (`true`) or OFF.
        on: bool,
    },
    /// Flips one macro cell.
    ToggleTile {
        /// Column.
        col: u32,
        /// Row.
        row: u32,
    },
    /// Replaces the whole macro pattern; its size becomes the grid size
    /// (also for later resets).
    SetPattern(MacroGrid),
    /// Turns ON the live cells of an RLE pattern (macro cells), with the
    /// pattern's bounding box placed at `(col, row)`.
    PlaceRle {
        /// RLE text (`#` comments allowed).
        rle: String,
        /// Column of the pattern's left edge.
        col: i64,
        /// Row of the pattern's top edge.
        row: i64,
    },
    /// Fills the grid at random: each tile ON with probability `density`,
    /// drawn from the model's seeded generator.
    Randomize {
        /// Probability of an ON tile, in `[0, 1]`.
        density: f64,
    },
    /// Turns every tile OFF.
    Clear,
    /// Rebuilds from the latest exact readout (fails if a tile was
    /// ambiguous or nothing has been read yet).
    RestartFromLatestReadout,
}
