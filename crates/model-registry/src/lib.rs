//! # model-registry
//!
//! The catalogue of every Emergence Lab model and scenario, with factories
//! that turn *(model or scenario, seed, parameter overrides, asset folder)*
//! into a runnable [`simulation_core::DynSimulation`]. Headless tools (the
//! `emergence-cli` binary, batch experiments, tests) use it to handle all
//! models uniformly.
//!
//! * [`MODELS`] — every model of the workspace with the crate implementing
//!   it, its documentation, its scenario folder and its default scenario.
//!   [`Registry::model_info`] adds what the model's own
//!   [`simulation_core::ModelDescriptor`] says (name, family, summary,
//!   rules, parameters with ranges and apply policies, metrics,
//!   determinism).
//! * Scenarios ([`scenario`]) — the built-in scenarios embedded in each
//!   model crate, plus the RON files of scenario directories laid out like
//!   the repository's `scenarios/` folder (the repository copy and a user
//!   folder). Ids are qualified by folder (`conway/glider-gun-showcase`);
//!   a bare id works when it is unique. Invalid files are reported with the
//!   model crate's message ([`Registry::problems`]) without hiding the
//!   others.
//! * Factories ([`Registry::build`]) — each model crate's real constructor
//!   (`Scenario::build_with_seed`, `MetapixelSim::from_scenario`,
//!   `ContinuousSim::with_seed`, `VoxelLife::new`, `EcosystemSim::from_scenario`,
//!   ...). Parameter overrides are converted and validated against the
//!   model's live descriptor and applied with their
//!   [`simulation_core::ApplyPolicy`]: parameters that require a reset are
//!   put into effect by one reset before the first tick ([`params`]). For
//!   models that publish their seed as a `seed` parameter, a `seed`
//!   override is the seed of the run, and seeds outside that parameter's
//!   range are refused.
//! * Validation ([`checks`]) — determinism, snapshot round trips and the
//!   model-specific checks the crates' public APIs allow.
//!
//! ```
//! use model_registry::{BuildRequest, ParamOverride, Registry};
//!
//! let registry = Registry::builtin();
//! let request = BuildRequest::new("ca-lab/highlife-replicator")
//!     .seed(7)
//!     .param(ParamOverride::parse("rule=B36/S23").unwrap());
//! let mut built = registry.build(&request).unwrap();
//! built.sim.advance(12);
//! assert_eq!(built.sim.tick(), 12);
//! assert!(built.sim.metrics().get("population").is_some());
//! ```

pub mod assets;
mod build;
pub mod checks;
pub mod error;
pub mod models;
pub mod params;
mod registry;
pub mod scenario;

pub use assets::Assets;
pub use checks::{CheckResult, CheckStatus, ValidationReport, validate};
pub use error::RegistryError;
pub use models::{MODELS, ModelInfo, ModelSpec, model_spec};
pub use params::{AppliedOverride, ParamOverride, SEED_PARAM, apply_overrides, coerce};
pub use registry::{BuildRequest, Built, Registry, RegistryConfig};
pub use scenario::{Scenario, ScenarioInfo, ScenarioProblem, ScenarioSource};
