//! # cellular-automata
//!
//! Engines and models for the **Classic Conway** mode and the **Cellular
//! Automata Laboratory** of Emergence Lab.
//!
//! * [`engine`] — four interchangeable two-state Life-like engines behind
//!   [`LifeEngine`]: [`DenseEngine`] (byte per cell), [`BitEngine`] (64 cells
//!   per word, SWAR), [`SparseEngine`] (64×64 tiles, active set) and
//!   [`HashlifeEngine`] (adapter over `hashlife::Universe`), plus the
//!   automatic choice [`select_engine`] and exact state transfer
//!   ([`switch_engine`]).
//! * Models implementing [`simulation_core::Simulation`]:
//!   * [`LifeSim`] — Conway's Life and every Life-like `B/S` rule (model ids
//!     `life.classic` and `ca.life_like`), with editing tools, symmetric
//!     random fills, pattern import and age/activity tracking;
//!   * [`GenerationsSim`] — multi-state Generations rules such as Brian's
//!     Brain and Star Wars (`ca.generations`);
//!   * [`WireworldSim`] — Wireworld circuits (`ca.wireworld`);
//!   * [`LangtonsAntSim`] — Langton's ant and multi-colour turmites
//!     (`ca.langtons_ant`);
//!   * [`WolframSim`] — elementary one-dimensional automata shown as a
//!     space-time diagram (`ca.wolfram`).
//! * [`view`] — the [`GridView`] / [`GridViewRequest`] contract consumed by
//!   the grid renderer (exact cells or level-of-detail density texels).
//! * [`scenario`] — RON scenario files (`scenarios/conway/`,
//!   `scenarios/ca-lab/`), also embedded in the crate.
//!
//! ```
//! use cellular_automata::{LifeCommand, LifeConfig, LifeMode, LifeSim, PatternLibrary, Rotation, PasteMode};
//! use patterns::Pattern;
//! use simulation_core::{Simulation, Topology};
//!
//! // An empty 64×64 torus running Conway's Life.
//! let config = LifeConfig { topology: Topology::Torus, width: 64, height: 64, init: vec![], ..LifeConfig::default() };
//! let mut life = LifeSim::new(&config, 1, &PatternLibrary::builtin(), LifeMode::Classic).unwrap();
//! let glider = Pattern::from_ascii(".O.\n..O\nOOO").unwrap();
//! life.apply(LifeCommand::Paste { pattern: glider, x: 10, y: 10, rotation: Rotation::None, mirror: false, mode: PasteMode::Or })
//!     .unwrap();
//! life.advance(4 * 64); // a glider crosses the torus diagonally in 256 generations
//! assert_eq!(life.population(), 5);
//! assert!(life.cell(11, 10) && life.cell(12, 11));
//! ```
//!
//! Coordinates follow the workspace convention: `x` grows to the right and
//! `y` downward. Everything is deterministic: randomness comes from the
//! [`simulation_core::SimRng`] stored in each model, parallel work (rayon)
//! only computes pure functions into separate buffers, and no result depends
//! on hash-map iteration order.

pub mod ant;
pub mod engine;
pub mod error;
pub mod generations;
pub mod geometry;
pub mod library;
pub mod life;
pub mod multistate;
pub mod scenario;
mod statemodel;
pub mod view;
pub mod wireworld;
pub mod wolfram;

pub use engine::{
    BitEngine, DenseEngine, EngineChoice, EngineKind, GenStats, HashlifeEngine, LifeEngine, SelectionInput,
    SparseEngine, create_engine, select_engine, switch_engine, tile_regularity,
};
pub use error::CaError;
pub use geometry::{CellRect, Rotation, Symmetry, World};
pub use life::{
    InitItem, LifeCommand, LifeConfig, LifeMode, LifeSim, MirrorAxis, PasteMode, PatternLibrary, PatternSource,
    RulePreset,
};
pub use view::{GridView, GridViewRequest};

pub use ant::{Ant, AntCommand, AntConfig, AntView, Heading, LangtonsAntSim, TurmiteRule};
pub use generations::{GenerationsConfig, GenerationsSim};
pub use multistate::{PaintCommand, StateInit};
pub use scenario::{BuiltSimulation, Scenario, ScenarioModel, builtin_scenarios};
pub use wireworld::{WireworldConfig, WireworldSim};
pub use wolfram::{Boundary, EcaStart, WolframCommand, WolframConfig, WolframSim};
