//! # voxel-life
//!
//! Three-dimensional cellular automata ("Three-Dimensional Life" mode):
//! outer-totalistic rules on the cubic lattice with Generations-style decay
//! states, as studied by Carter Bays (Life 4555, 5766) and popularised by
//! Softology's *Visions of Chaos* (`S/B/C/N` rules such as `4/4/5/M`).
//!
//! * [`Rule3D`] — canonical rule: survival and birth count sets, number of
//!   states `C`, neighbourhood (Moore 26 or von Neumann 6). Parses Softology
//!   `S/B/C/N`, letters `B5/S45[/C<n>][/N]`, Bays `5766` / `(10 21 10 21)`
//!   and Golly `3D5..7/6[F|V]`; prints all of them.
//! * Engines: a dense `u8` grid (bounded or toroidal) with double buffering
//!   and rayon over z-planes, and sparse 16³ chunks in a hash map (bounded
//!   or unbounded) iterated in sorted order. Both count only state-1
//!   neighbours, exactly as Golly's Generations algorithm.
//! * [`VoxelLife`] — the [`simulation_core::Simulation`] implementation:
//!   parameters with apply policies, commands (set / clear voxels, fill box,
//!   clear, randomize), metrics (population per state, births, deaths,
//!   bounding box, centroid), versioned snapshots including the RNG, and
//!   views ([`VoxelViewRequest`]): visible surface voxels within a slice
//!   range, a single 2D layer, and counts per state.
//! * [`scenarios`] — RON presets (`scenarios/voxel/`, also embedded).
//!
//! ```
//! use simulation_core::Simulation;
//! use voxel_life::{VoxelLife, scenarios};
//!
//! // Bays' 5766 glider: period 4, displacement (1, 1, 0).
//! let mut life = VoxelLife::new(scenarios::builtin_by_id("life-5766-glider")?)?;
//! let start: Vec<_> = life.voxels().iter().map(|v| v.pos).collect();
//! life.advance(4);
//! let moved: Vec<_> = life.voxels().iter().map(|v| [v.pos[0] - 1, v.pos[1] - 1, v.pos[2]]).collect();
//! assert_eq!(start, moved);
//! # Ok::<(), voxel_life::VoxelError>(())
//! ```

mod dense;
mod error;
mod geometry;
mod init;
mod rule;
pub mod scenarios;
mod sim;
mod sparse;
mod stats;
mod view;
mod world;

pub use error::{RuleError, VoxelError};
pub use geometry::{Axis, Bounds, COORD_LIMIT, MAX_DENSE_VOXELS, MAX_SIDE, Pos, WorldKind};
pub use init::{InitKind, InitSpec, MAX_RANDOM_VOXELS, Mirror, PatternVoxel, WorldConfig};
pub use rule::{CountSet, MAX_COUNT, Neighbourhood, Rule3D};
pub use scenarios::{Evidence, Scenario};
pub use sim::{MODEL_ID, SCHEMA_VERSION, Voxel, VoxelCommand, VoxelLife};
pub use sparse::CHUNK_SIDE;
pub use stats::Stats;
pub use view::{
    CountsView, DEFAULT_MAX_SURFACE_VOXELS, MAX_SLICE_SIDE, SliceRange, SliceView, SliceWindow, SurfaceView,
    SurfaceVoxel, VoxelView, VoxelViewRequest, faces,
};
pub use world::MAX_SPARSE_CHUNKS;
