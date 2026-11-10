//! # rule-engine — Custom Life Creator
//!
//! Lets users define their own artificial life without writing Rust:
//!
//! * [`def`] — serialisable, versioned definitions: [`LifeSystem`] with
//!   entity types, relationships, DSL rules, Rhai script rules, the world
//!   (size, topology, resource and environment fields, random events) and
//!   end conditions. RON and JSON.
//! * [`dsl`] — the no-code rule language: lexer, parser with line/column
//!   spans, AST, pretty-printer that round-trips.
//! * [`mod@compile`] — validation with precise diagnostics, relationship
//!   compilation into ordinary (visible) rules, lowering to a typed runtime
//!   IR.
//! * [`script`] — sandboxed Rhai 1.27 scripts that read the world and only
//!   record intents.
//! * [`sim`] — [`CustomLifeSim`], the `simulation_core::Simulation`
//!   implementation: continuous-position agents on a grid world with
//!   resource fields; deterministic decisions and intent resolution built
//!   on organism-engine.
//! * [`scenarios`] — the seven presets in `scenarios/custom-life/`.
//!
//! **For editors**: [`validate`] lists every diagnostic of a system (each
//! anchored to its [`ItemRef`]); [`check_rule_text`] / [`check_rule`] give
//! live feedback on one rule; [`compile::vocab`] describes every sensor and
//! action; [`classify_edit`] tells whether an edit applies next tick or at
//! the next reset; [`CustomLifeCommand::ReplaceSystem`] applies it to a
//! running model; [`RelationshipGraph`] (in every view) feeds a graph view.
//! See `docs/RULE_ENGINE.md` §11.
//!
//! See `docs/RULE_ENGINE.md` for the user and developer documentation and
//! `docs/models/rule-engine.md` for the model description.
//!
//! ```
//! use rule_engine::{CustomLifeSim, scenarios};
//! use simulation_core::Simulation;
//!
//! let preset = scenarios::builtin_by_id("bacterial-growth").unwrap();
//! let mut sim = CustomLifeSim::from_scenario(&preset).unwrap();
//! sim.advance(20);
//! assert!(sim.population("bacterium") > 0);
//! ```

#![warn(missing_docs)]

pub mod compile;
pub mod def;
pub mod dsl;
pub mod error;
pub mod scenarios;
pub mod script;
pub mod sim;

pub use compile::{
    CompiledSystem, GraphEdge, GraphNode, RelationshipGraph, check_rule, check_rule_text, compile, validate,
};
pub use def::{EntityType, LifeSystem, RelationKind, Relationship, ScriptRule, WorldDef};
pub use error::{Diagnostic, ItemRef, RuleEngineError, Severity, Span};
pub use scenarios::Scenario;
pub use script::{ScriptError, ScriptErrorKind};
pub use sim::command::{CustomLifeCommand, EditReport, PaintMode, classify_edit};
pub use sim::view::{CustomLifeView, CustomLifeViewRequest};
pub use sim::{CustomLifeSim, MODEL_ID};
