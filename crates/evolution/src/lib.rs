//! # evolution — the Evolution Laboratory
//!
//! A population of prey (and optionally predators) with heritable genomes
//! lives in a self-contained 2D world: food patches that regrow
//! logistically, a temperature field with a spatial gradient and a temporal
//! shift, and switchable barriers that cut the land into islands.
//! **Selection is never computed**: there is no fitness function anywhere in
//! the model. Gene frequencies change only because some organisms survive
//! and breed more than others, through drift in small populations, and
//! through mutation.
//!
//! | Module | Contents |
//! |---|---|
//! | [`genes`] | the 13 heritable genes and 2 acquired (non-heritable) traits, expression, founders |
//! | [`config`], [`params`] | experiment configuration, parameter specs with apply policies |
//! | [`environment`] | terrain, barriers, regions, logistic food, temperature field |
//! | [`organism`] | individual state and physiology (allometric costs, thermal stress, senescence) |
//! | [`behaviour`] | parallel decision phase (flee, court, graze, forage, hunt) |
//! | [`reproduction`] | readiness, inheritance through the organism engine, parental investment |
//! | [`step`] | the tick: decide, live, predation, feeding, reproduction, mortality |
//! | [`stats`] | gene statistics, allele-like classes, diversity, regional divergence, F_ST, lineages |
//! | [`speciation`] | the explicit, observable speciation criterion |
//! | [`commands`], [`view`] | interventions and render data |
//! | [`sim`] | [`EvolutionSim`], the [`simulation_core::Simulation`] implementation |
//! | [`scenarios`] | RON presets: [`scenarios::builtin`], [`scenarios::load_dir`] |
//!
//! Determinism: all randomness comes from [`simulation_core::SimRng`]
//! streams stored in (or derived from) the world state; parallel phases are
//! pure and conflicts are resolved by the organism engine's intent
//! resolution in a fixed order. Snapshots include the generator.

#![warn(missing_docs)]

pub mod behaviour;
pub mod commands;
pub mod config;
pub mod environment;
pub mod error;
pub mod genes;
pub mod organism;
pub mod params;
pub mod reproduction;
pub mod scenarios;
pub mod sim;
pub mod speciation;
pub mod stats;
pub mod step;
pub mod view;
pub mod world;

pub use commands::EvolutionCommand;
pub use config::{EvolutionConfig, ReproductionMode, Shape, Terrain};
pub use error::EvolutionError;
pub use genes::{GENES, Phenotype, TraitValue};
pub use organism::{Activity, Organism, Species};
pub use scenarios::Scenario;
pub use sim::{EvolutionSim, MODEL_ID};
pub use speciation::SpeciationStatus;
pub use stats::PopulationStats;
pub use view::{EvolutionView, EvolutionViewRequest, FieldLayer};
pub use world::{DeathCause, EvolutionWorld, TickReport};
