//! # hashlife
//!
//! A standalone, safe-Rust implementation of Gosper's Hashlife (Gosper 1984;
//! Rokicki, *An Algorithm for Compressing Space and Time*, Dr. Dobb's 2006)
//! for every two-state outer-totalistic Moore-neighbourhood rule without
//! `B0` (Conway's `B3/S23`, HighLife `B36/S23`, Seeds `B2/S`, ...).
//!
//! * [`Universe`] — the engine: an unbounded `i64` plane stored as a
//!   hash-consed quadtree (`u32`-indexed node arena, open-addressing intern
//!   table, 8×8 bit-packed leaves) whose nodes memoise their future. It
//!   advances `2^k` generations per [`Universe::step_pow2`] call or any
//!   number with [`Universe::advance`], answers level-of-detail queries in
//!   time proportional to the visible nodes ([`Universe::density_grid`]),
//!   and garbage-collects automatically against a soft node limit.
//! * [`macrocell`] — Golly `.mc` reading and writing with Golly's exact
//!   coordinates.
//! * [`HashlifeSim`] — the engine behind the shared
//!   [`simulation_core::Simulation`] contract (model id `life.hashlife`).
//!
//! Results are exact and deterministic: the recursion is a pure function of
//! the pattern, and nothing depends on hash-table layout, garbage-collection
//! timing or memory limits.
//!
//! ```
//! use hashlife::{Rule, Universe};
//!
//! // Gosper glider gun, then a big jump in time.
//! let gun = [
//!     (24, 0), (22, 1), (24, 1), (12, 2), (13, 2), (20, 2), (21, 2), (34, 2), (35, 2),
//!     (11, 3), (15, 3), (20, 3), (21, 3), (34, 3), (35, 3), (0, 4), (1, 4), (10, 4),
//!     (16, 4), (20, 4), (21, 4), (0, 5), (1, 5), (10, 5), (14, 5), (16, 5), (17, 5),
//!     (22, 5), (24, 5), (10, 6), (16, 6), (24, 6), (11, 7), (15, 7), (12, 8), (13, 8),
//! ];
//! let mut life = Universe::with_rule(Rule::conway());
//! life.insert_cells(gun);
//! life.advance(30 * 1000).unwrap();
//! let before = life.population();
//! life.advance(30).unwrap();
//! assert_eq!(life.generation(), 30_030);
//! assert_eq!(life.population(), before + 5); // one more glider per period
//! ```

mod error;
mod leaf;
pub mod macrocell;
mod query;
mod rule;
mod sim;
mod step;
mod store;
mod universe;

pub use error::HashlifeError;
pub use query::{Cells, MAX_DENSITY_PIXELS};
pub use rule::Rule;
pub use sim::{
    HashlifeCommand, HashlifeSim, HashlifeView, HashlifeViewRequest, MAX_SOUP_CELLS, MAX_VIEW_SIDE, MODEL_ID,
    SCHEMA_VERSION,
};
pub use universe::{DEFAULT_NODE_LIMIT, MAX_STEP_EXPONENT, MIN_NODE_LIMIT, Universe};
