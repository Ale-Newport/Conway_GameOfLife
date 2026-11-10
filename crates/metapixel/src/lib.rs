//! # metapixel — Conway within Conway
//!
//! Brice Due's **OTCA metapixel** (2006) is a 2048×2048 pattern of Conway's
//! Life that behaves like one cell of *another* Life-like cellular
//! automaton: tiled at pitch 2048, ON and OFF tiles exchange gliders with
//! their neighbours and every 35 328 generations each tile shows the state
//! that the programmed rule gives it. This crate builds such tilings from
//! the original tiles and runs them **exactly**, cell by cell, under B3/S23
//! with Hashlife.
//!
//! * [`tile`] — loads the unprogrammed OFF/ON tiles (asset files, validated
//!   against the published populations) and programs them for any
//!   outer-totalistic rule and any subset of the Moore neighbourhood
//!   ([`Programming`]).
//! * [`build`] — composes an N×M construction from a [`MacroGrid`] with
//!   shared quadtree squares (a macrocell document read by Hashlife), tiles
//!   OR-ed at pitch 2048 exactly as specified.
//! * [`readout`] — counts live cells in each tile's readout core to recover
//!   macro states ([`TileReading`]); [`spec`] holds the validated numbers.
//! * [`accelerated`] — the **accelerated macro representation — not a
//!   physical simulation**: the same rule applied directly to the macro
//!   grid, used only to predict and flag mismatches.
//! * [`MetapixelSim`] — the `life.metapixel` [`simulation_core::Simulation`]
//!   with views for multi-scale zoom ([`GridView`] cell textures at any level
//!   of detail, [`MacroView`], [`TileInspection`] overlays, [`PhaseInfo`]).
//! * [`golly`] and [`gzip`] — reading Golly's own metapixel demos
//!   (`.mc.gz`) to verify them against the macro rule.
//! * [`scenario`] — RON scenarios (embedded copies of `scenarios/metapixel/`).
//!
//! ```
//! use std::sync::Arc;
//! use metapixel::{MacroGrid, MetapixelSim, TileSet, WorldConfig, spec::PERIOD};
//! use simulation_core::Simulation;
//!
//! let tiles = Arc::new(TileSet::from_dir(&TileSet::source_tree_dir())?);
//! let blinker = MacroGrid::from_rows(&["...", "OOO", "..."])?;
//! let mut sim = MetapixelSim::new(tiles, WorldConfig::default(), blinker)?;
//! sim.advance(2 * PERIOD); // 70 656 generations of a 6158×6158-cell pattern
//! let record = sim.history().latest().unwrap(); // metagen 1, read at t = 2P
//! assert_eq!(record.exact.to_rows(), [".O.", ".O.", ".O."]);
//! assert!(record.matches_prediction());
//! # Ok::<(), metapixel::MetapixelError>(())
//! ```
//!
//! Licensing: the tile and demo files are third-party GPL-2.0-or-later
//! *data* loaded at run time from `assets/patterns/otca/` (see
//! `PROVENANCE.md` there); this crate contains no GPL code and does not
//! embed them.

pub mod accelerated;
pub mod annotations;
pub mod build;
mod error;
pub mod golly;
pub mod gzip;
pub mod macro_grid;
pub mod programming;
mod quadtree;
pub mod readout;
pub mod scenario;
mod sim;
pub mod spec;
pub mod tile;

pub use accelerated::{ACCELERATED_LABEL, EXACT_LABEL, MacroModel};
pub use annotations::{Annotation, AnnotationKind, tile_annotations};
pub use build::{BuildStats, build_universe, construction_macrocell};
pub use error::MetapixelError;
pub use golly::{GollyTiling, read_golly_tiling};
pub use macro_grid::{MAX_GRID_SIDE, MacroGrid};
pub use programming::{Neighbourhood, Programming};
pub use readout::{LiveReading, TileGeometry, TileReading};
pub use scenario::Scenario;
pub use sim::{
    AmbiguousTile, CycleHalf, CycleStage, GridView, GridViewRequest, HISTORY_CAPACITY, MAX_LIVE_READOUT_TILES,
    MAX_LOD_LOG2, MAX_NODE_LIMIT, MAX_STEP_EXPONENT, MAX_TEXTURE_SIDE, MODEL_ID, MacroTile, MacroView,
    MetapixelCommand, MetapixelSim, MetapixelView, MetapixelViewRequest, NeighbourChannel, PhaseInfo, Predictor,
    ReadoutHistory, ReadoutRecord, SCENARIO_STREAM, SCHEMA_VERSION, ScaleInfo, StepSize, TileInspection, WorldConfig,
};
pub use spec::{CellRect, Direction, PERIOD, RuleTable};
pub use tile::{ProgrammedTiles, TileSet, TileState};
