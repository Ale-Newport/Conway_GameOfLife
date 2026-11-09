//! # organism-engine
//!
//! Generic, deterministic agent infrastructure shared by the ecosystem,
//! evolution, civilization and custom-life (rule-engine) models. Nothing here
//! is a model by itself; the crate provides the building blocks every
//! agent-based model needs, all free of hidden randomness and of
//! `HashMap`-iteration-order effects:
//!
//! | Module | Contents |
//! |---|---|
//! | [`id`], [`store`] | [`AgentId`], [`AgentStore`]: stable ids, id-ordered storage, deferred spawn/despawn |
//! | [`kinematics`] | [`Kinematics`] point-mass integration with speed/acceleration limits, world bounds and a passability hook |
//! | [`steering`] | seek, flee, arrive, pursue, evade, wander, boids separation/alignment/cohesion, sampled obstacle avoidance, prioritised blending |
//! | [`perception`] | radius / field-of-view queries on [`simulation_core::SpatialGrid`] (wrap-aware on a torus), sorted results, sampled line of sight |
//! | [`utility`], [`fsm`] | utility-AI response curves, considerations, compensated action scores, [`choose_best`]; a small state machine |
//! | [`intent`] | [`Claim`]s and deterministic exclusive / one-to-one / shared conflict resolution |
//! | [`genome`], [`lineage`] | gene schemas, normalised genomes, mutation, crossover, genetic distance; lineage and a pruned genealogy |
//! | [`drive`] | needs such as hunger, thirst and fatigue |
//!
//! ## Determinism contract
//!
//! * Randomness is only drawn from a [`simulation_core::SimRng`] passed in by
//!   the caller (stored in the model state and in snapshots); each function
//!   documents how many numbers it draws.
//! * Agents are iterated in [`AgentId`] order; every tie is broken by the
//!   lower id (or the lower index for id-less inputs).
//! * Conflicting decisions are collected as [`Claim`]s and resolved serially
//!   by [`intent`] in a total order that does not depend on input order.
//! * All public data types implement `serde` so models can put them in
//!   snapshots; deserialisation validates invariants.
//!
//! Floating-point results are reproducible on the same platform and build;
//! bit-exactness across platforms is not promised (see `ARCHITECTURE.md` §3.2).

#![warn(missing_docs)]

pub mod drive;
pub mod error;
pub mod fsm;
pub mod genome;
pub mod id;
pub mod intent;
pub mod kinematics;
pub mod lineage;
pub mod perception;
pub mod steering;
pub mod store;
pub mod utility;

pub use drive::Drive;
pub use error::OrganismError;
pub use fsm::StateMachine;
pub use genome::{Crossover, GeneSpec, Genome, GenomeSchema, MutationBounds, genetic_distance};
pub use id::AgentId;
pub use intent::{
    Allocation, Claim, Resolution, ShareRule, resolve_claims, resolve_exclusive, resolve_matching, resolve_shared,
};
pub use kinematics::{Bounds, Kinematics, MotionLimits, Movement};
pub use lineage::{Genealogy, Lineage};
pub use store::AgentStore;
pub use utility::{ActionScore, Consideration, Curve, choose_best};
