//! # simulation-core
//!
//! Shared, renderer-independent building blocks used by every Emergence Lab
//! simulation model:
//!
//! * [`Simulation`] — the trait every model implements (fixed ticks, commands,
//!   parameters, metrics, views, snapshots) and its object-safe twin
//!   [`DynSimulation`] used by headless tools.
//! * [`SimRng`] — a self-contained, serialisable xoshiro256++ generator so that
//!   seeded experiments stay bit-for-bit reproducible across dependency updates.
//! * [`params`] — typed parameter schemas with explicit [`ApplyPolicy`]
//!   (immediate / next tick / requires reset).
//! * [`Metrics`], [`Snapshot`], [`history`] (periodic snapshots and undo/redo)
//!   and [`runner::Runner`], a worker-thread tick loop that decouples simulation
//!   speed from rendering frame rate.
//! * Spatial helpers: [`Grid2`], [`Topology`], [`SpatialGrid`].
//!
//! Nothing in this crate depends on a renderer; models built on it can run in
//! tests, benchmarks and the command-line tool exactly as they run in the
//! desktop application.

pub mod error;
pub mod events;
pub mod grid;
pub mod history;
pub mod metrics;
pub mod model;
pub mod params;
pub mod rng;
pub mod runner;
pub mod simulation;
pub mod snapshot;
pub mod spatial;

pub use error::SimError;
pub use events::{EventLog, EventSeverity, SimEvent};
pub use grid::{Grid2, Topology};
pub use history::{HistoryConfig, SnapshotHistory, UndoStack};
pub use metrics::{MetricKind, MetricSpec, Metrics};
pub use model::{ModelDescriptor, ModelFamily};
pub use params::{ApplyPolicy, ParamKind, ParamSet, ParamSpec, ParamValue};
pub use rng::SimRng;
pub use runner::{Control, Published, Runner, RunnerConfig, RunnerStatus, SessionControl, Speed};
pub use simulation::{DynSimulation, Erased, Simulation, erase};
pub use snapshot::Snapshot;
pub use spatial::SpatialGrid;

/// Re-export of the vector math types used across simulation crates. The
/// version matches the one used by Bevy so that positions can be handed to the
/// renderer without conversion.
pub use glam;
