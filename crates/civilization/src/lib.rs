//! # civilization — Emergent Civilizations
//!
//! An agent-based society on a self-contained, regenerating tile world.
//! People have needs (food via energy, water, rest, safety, company),
//! skills that improve with practice and fade without it, inventories,
//! preferences (risk, cooperation, trade propensity, time preference) and
//! bounded relationships. A utility AI (organism-engine `utility`) picks
//! their goals; they move by steering along A* routes, gather, farm, craft
//! tools, build huts, stores and farm plots, barter at marginal-rate-of-
//! substitution prices (Sugarscape rule T), keep surplus food in household
//! and group stores, share within groups, and steal or fight when desperate
//! or territorial. Nothing is scripted: roads emerge from footfall
//! (active-walker trails), groups from mutual affinity, territories from
//! group buildings (XTENT-style influence), specialisation from learning by
//! doing and trade, and population change from births and deaths.
//!
//! | Module | Contents |
//! |---|---|
//! | [`sim`] | [`CivilizationSim`] (`Simulation` implementation), descriptor, snapshots |
//! | [`config`], [`scenarios`] | [`Scenario`] files (RON), presets, runtime [`Rules`] and their `ParamSpec`s |
//! | [`terrain`], [`noise`], [`world`], [`path`] | terrain classes and generation, resources, trails, A* routing |
//! | [`agent`], [`skills`], [`economy`], [`buildings`], [`groups`], [`territory`] | agent state, skills and kinds of work, welfare and barter, buildings, groups, territory |
//! | [`command`], [`view`], [`metrics`] | interventions, render data, published metrics |
//!
//! ## One tick
//!
//! ```text
//! daily     (every 24 ticks) relationship decay, groups, building groups, territory
//! decide    agents whose goal ended, expired or is pre-empted choose a new one (utility AI, A*)
//! move      sub-stepped steering along routes; footfall builds trails
//! act       eat, drink, rest, craft, build, store; claims on tiles, plots, sites, stores resolved
//! interact  barter, sharing, socialising, familiarity, theft, territorial defence, births
//! life      metabolism, needs, health, skills, ageing; deaths and births committed; inheritance
//! world     crop growth, building decay, resource growback, trail and danger decay
//! ```
//!
//! Full model documentation: `docs/models/civilization.md`.

#![warn(missing_docs)]

mod act;
pub mod agent;
pub mod buildings;
pub mod command;
pub mod config;
mod decide;
pub mod economy;
pub mod error;
pub mod groups;
mod life;
pub mod metrics;
mod navigate;
pub mod noise;
pub mod path;
pub mod scenarios;
pub mod sim;
pub mod skills;
mod social;
mod state;
mod step;
pub mod terrain;
pub mod territory;
pub mod view;
pub mod world;

pub use agent::{Activity, Agent, Preferences, Sex, Skill, Work};
pub use buildings::{Building, BuildingId, BuildingKind};
pub use command::CivCommand;
pub use config::{Rules, Scenario};
pub use economy::{Bundle, Good};
pub use error::CivError;
pub use groups::{Group, GroupId};
pub use sim::{CivilizationSim, MODEL_ID};
pub use state::TickStats;
pub use terrain::{Resource, Terrain, TerrainSpec};
pub use view::{CivView, CivViewRequest};
