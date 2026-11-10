//! # ecosystem
//!
//! The biotic layer of the Emergence Lab Ecosystem Simulator, on top of the
//! abiotic world of `physics-world` (terrain, biomes, climate, seasons,
//! hydrology, fire, disasters) and the agent infrastructure of
//! `organism-engine` (stores, steering, perception, utility AI, intents,
//! genomes, genealogy):
//!
//! * [`species`] — data-driven species (RON; 41 built-in animals from
//!   elephants to earthworms): roles, diets, locomotion, habitat, Kleiber
//!   metabolism, perception, life history, reproduction, social behaviour,
//!   migration and hibernation, representation (individuals, density
//!   fields, colonies) and visual ids; vegetation layers, tree species and
//!   fungal guilds; validation with precise messages.
//! * [`animal`] — individual animals: utility AI with species-specific
//!   considerations, steering, hunting with probabilistic captures,
//!   fleeing to refuges, drinking, sleeping, mating, pregnancy and eggs,
//!   parental care, Kleiber/MTE energetics, drives, growth, ageing and
//!   death; flight altitude, herds/flocks/schools (boids), pack leaders
//!   with shared targets, territorial defence and fights, alarm calls,
//!   migration, hibernation and injuries.
//! * [`fields`] — tiny organisms (insects, spiders, worms) as density fields
//!   with representative display markers; [`colony`] — ant and bee colonies
//!   as super-agents with forager squads, pheromone trails and pollination;
//!   [`disease`] — SIR infections spreading by contact; [`schedule`] —
//!   scenario interventions at given days.
//! * [`vegetation`], [`trees`], [`fungi`] — logistic vegetation layers with
//!   Beer–Lambert light, Liebig limitation and seed banks; individual trees
//!   with light competition, phenology and seeding; decomposer and
//!   mycorrhizal fungi.
//! * [`ledger`] — explicit energy and nutrient accounting of every flux.
//! * [`environment`] — fuel and canopy written to the world, fire and wind
//!   effects read back, water and fire perception caches.
//! * [`netlogo`] — the rules of NetLogo's Wolf Sheep Predation.
//! * [`EcosystemSim`] — the `simulation_core::Simulation` (model id
//!   `ecosystem`, one tick = 6 simulated minutes by default) with
//!   parameters, commands, metrics, views and snapshots; [`scenario`] loads
//!   the presets of `scenarios/ecosystem/`.
//!
//! All randomness comes from streams derived from the scenario seed (see
//! [`rng`]); parallel phases only read shared state; contested resources are
//! resolved with `organism-engine` claims in a total order. Runs are
//! reproducible for a given seed, platform and build.

#![warn(missing_docs)]

pub mod animal;
pub mod carcass;
pub mod colony;
pub mod command;
pub mod config;
pub mod disease;
pub mod environment;
pub mod error;
pub mod fields;
mod food;
pub mod fungi;
mod introduce;
pub mod ledger;
pub mod metrics;
pub mod netlogo;
pub mod params;
pub mod rng;
pub mod scenario;
pub mod schedule;
pub mod sim;
pub mod species;
mod state;
mod step;
pub mod trees;
pub mod units;
pub mod vegetation;
pub mod view;

pub use animal::{Action, Animal, DeathCause, OrganismRef, ReproState, Sex, Target};
pub use colony::{Colonies, Colony, Forager, ForagerState};
pub use command::EcosystemCommand;
pub use config::{AnimalSetup, EcosystemConfig, NetLogoConfig, Rules, SoilSetup, Terrain, TreeSetup, VegetationSetup};
pub use disease::{Infection, InfectionStage, Pathogen};
pub use error::EcosystemError;
pub use fields::{FieldMarker, Fields};
pub use ledger::{Flux, FluxKind, Ledger, Ledgers, Stocks};
pub use params::RuntimeParams;
pub use scenario::Scenario;
pub use schedule::{EventAction, ScheduledEvent, WorldEvent};
pub use sim::{EcosystemSim, MODEL_ID, SCHEMA_VERSION};
pub use species::{AnimalTraits, PlantLayer, Species, SpeciesCatalog, TreeSpecies};
pub use state::{Counters, FollowState};
pub use view::{
    AnimalInspection, AnimalView, EcosystemView, EcosystemViewRequest, OrganismInspection, TreeInspection, TreeView,
};
