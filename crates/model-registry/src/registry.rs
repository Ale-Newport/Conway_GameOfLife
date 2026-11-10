//! The [`Registry`]: models, discovered scenarios and the factory.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use simulation_core::DynSimulation;

use crate::assets::Assets;
use crate::build::construct;
use crate::error::RegistryError;
use crate::models::{MODELS, ModelInfo, ModelSpec, model_spec};
use crate::params::{AppliedOverride, ParamOverride, SEED_PARAM, apply_overrides};
use crate::scenario::{self, Scenario, ScenarioInfo, ScenarioProblem};

/// Where the registry looks for data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistryConfig {
    /// Asset folder (`assets/`): pattern catalog, OTCA tiles, species.
    pub assets: Option<PathBuf>,
    /// Scenario directories with the layout of the repository's
    /// `scenarios/` folder, in increasing priority (a file in a later
    /// directory replaces a scenario with the same qualified id).
    pub scenario_dirs: Vec<PathBuf>,
}

/// What to build.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildRequest {
    /// Model id (its default scenario), qualified or bare scenario id, or
    /// the path of a scenario file.
    pub target: String,
    /// Seed replacing the scenario's.
    pub seed: Option<u64>,
    /// Parameter overrides, applied in order before the first tick.
    pub params: Vec<ParamOverride>,
}

impl BuildRequest {
    /// Request for `target` with the scenario's seed and parameters.
    pub fn new(target: &str) -> Self {
        Self { target: target.to_string(), ..Self::default() }
    }

    /// Builder: seed override.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Builder: one more parameter override.
    pub fn param(mut self, o: ParamOverride) -> Self {
        self.params.push(o);
        self
    }
}

/// A built simulation and how it was configured.
pub struct Built {
    /// The simulation, at tick 0 with every override in effect.
    pub sim: Box<dyn DynSimulation>,
    /// The scenario it was built from.
    pub scenario: ScenarioInfo,
    /// The seed used.
    pub seed: u64,
    /// Applied parameter overrides.
    pub overrides: Vec<AppliedOverride>,
    /// Whether a reset was needed to apply reset-only parameters.
    pub reset_applied: bool,
}

/// Catalogue of every model and scenario with factories producing runnable
/// simulations.
#[derive(Debug)]
pub struct Registry {
    assets: Assets,
    scenarios: Vec<Scenario>,
    problems: Vec<ScenarioProblem>,
    scenario_dirs: Vec<PathBuf>,
}

impl Registry {
    /// Discovers the built-in scenarios and the files of every scenario
    /// directory of `config`. Unreadable or invalid files do not stop
    /// discovery; they are listed by [`Registry::problems`].
    pub fn new(config: RegistryConfig) -> Self {
        let assets = Assets::new(config.assets);
        let (mut scenarios, mut problems) = scenario::builtin();
        for dir in &config.scenario_dirs {
            if dir.is_dir() {
                scenario::scan_dir(dir, &assets, &mut scenarios, &mut problems);
            }
        }
        Self { assets, scenarios, problems, scenario_dirs: config.scenario_dirs }
    }

    /// Built-in scenarios only, without an asset folder (models needing
    /// asset files report it when built).
    pub fn builtin() -> Self {
        Self::new(RegistryConfig::default())
    }

    /// Shared model data.
    pub fn assets(&self) -> &Assets {
        &self.assets
    }

    /// Scenario directories that were scanned (as configured).
    pub fn scenario_dirs(&self) -> &[PathBuf] {
        &self.scenario_dirs
    }

    /// Every model, in presentation order.
    pub fn models(&self) -> &'static [ModelSpec] {
        &MODELS
    }

    /// One model.
    ///
    /// # Errors
    /// [`RegistryError::UnknownModel`].
    pub fn model(&self, id: &str) -> Result<&'static ModelSpec, RegistryError> {
        model_spec(id).ok_or_else(|| RegistryError::UnknownModel {
            id: id.to_string(),
            known: MODELS.iter().map(|m| m.id.to_string()).collect(),
        })
    }

    /// Information about a model, from the descriptor of its default
    /// scenario (built with the scenario's seed).
    ///
    /// # Errors
    /// Unknown ids and build failures (for example missing asset files).
    pub fn model_info(&self, id: &str) -> Result<ModelInfo, RegistryError> {
        let spec = self.model(id)?;
        let built = self.build(&BuildRequest::new(spec.id))?;
        Ok(ModelInfo::new(spec, built.sim.descriptor()))
    }

    /// Every scenario: built-in ones in presentation order, then files.
    pub fn scenarios(&self) -> &[Scenario] {
        &self.scenarios
    }

    /// Scenarios building `model_id`.
    pub fn scenarios_of<'a>(&'a self, model_id: &'a str) -> impl Iterator<Item = &'a Scenario> + 'a {
        self.scenarios.iter().filter(move |s| s.info.model_id == model_id)
    }

    /// Scenario files that could not be used.
    pub fn problems(&self) -> &[ScenarioProblem] {
        &self.problems
    }

    /// The default scenario of a model.
    ///
    /// # Errors
    /// Unknown model, or a default scenario that is missing.
    pub fn default_scenario(&self, model_id: &str) -> Result<&Scenario, RegistryError> {
        let spec = self.model(model_id)?;
        let id = format!("{}/{}", spec.folder, spec.default_scenario);
        self.scenarios
            .iter()
            .find(|s| s.info.id == id && s.info.model_id == spec.id)
            .ok_or_else(|| RegistryError::build(spec.id, format!("its default scenario `{id}` is not available")))
    }

    /// A scenario by qualified id, or by bare id when only one folder has it.
    ///
    /// # Errors
    /// [`RegistryError::AmbiguousScenario`] and [`RegistryError::UnknownTarget`].
    pub fn scenario(&self, id: &str) -> Result<&Scenario, RegistryError> {
        if let Some(s) = self.scenarios.iter().find(|s| s.info.id == id) {
            return Ok(s);
        }
        let matches: Vec<&Scenario> = self.scenarios.iter().filter(|s| s.info.local_id == id).collect();
        match matches.as_slice() {
            [one] => Ok(one),
            [] => Err(RegistryError::UnknownTarget { id: id.to_string(), suggestions: self.suggestions(id) }),
            many => Err(RegistryError::AmbiguousScenario {
                id: id.to_string(),
                candidates: many.iter().map(|s| s.info.id.clone()).collect(),
            }),
        }
    }

    /// Resolves a model id (its default scenario), a scenario id or the path
    /// of a scenario file (whose parent folder names the format).
    ///
    /// # Errors
    /// Unknown or ambiguous ids and invalid files.
    pub fn resolve(&self, target: &str) -> Result<Cow<'_, Scenario>, RegistryError> {
        if model_spec(target).is_some() {
            return self.default_scenario(target).map(Cow::Borrowed);
        }
        match self.scenario(target) {
            Ok(s) => Ok(Cow::Borrowed(s)),
            Err(e) => {
                let path = Path::new(target);
                if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("ron")) && path.is_file() {
                    scenario::load_path(path, &self.assets).map(Cow::Owned)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Ids close to `id`: those containing it, or contained in it.
    fn suggestions(&self, id: &str) -> Vec<String> {
        let needle = id.to_ascii_lowercase();
        let ids = MODELS.iter().map(|m| m.id.to_string()).chain(self.scenarios.iter().map(|s| s.info.id.clone()));
        let mut out: Vec<String> = ids
            .filter(|candidate| {
                let c = candidate.to_ascii_lowercase();
                !needle.is_empty() && (c.contains(&needle) || (c.len() > 3 && needle.contains(&c)))
            })
            .collect();
        out.truncate(5);
        out
    }

    /// Builds a simulation: resolves the target, constructs the model with
    /// the seed, applies the parameter overrides (resetting once if a
    /// parameter requires it) and returns it at tick 0.
    ///
    /// A `seed` parameter override (models that publish their seed as a
    /// parameter) sets the seed of the run like [`BuildRequest::seed`];
    /// giving both with different values is an error.
    ///
    /// # Errors
    /// Resolution, construction, asset and parameter errors.
    pub fn build(&self, request: &BuildRequest) -> Result<Built, RegistryError> {
        let scenario = self.resolve(&request.target)?;
        let seed = request.seed.unwrap_or(scenario.info.seed);
        let mut sim = construct(&self.assets, &scenario, seed)?;
        let (overrides, reset_applied) = apply_overrides(sim.as_mut(), &request.params)?;
        let seed = match overrides.iter().rev().find(|o| o.key == SEED_PARAM).and_then(|o| o.value.as_i64()) {
            None => seed,
            Some(param) => {
                // Validated against the parameter's range, which starts at 0.
                let param = u64::try_from(param).map_err(|_| RegistryError::InvalidParameter {
                    key: SEED_PARAM.to_string(),
                    message: format!("{param} is not a seed"),
                })?;
                if request.seed.is_some_and(|s| s != param) {
                    return Err(RegistryError::InvalidParameter {
                        key: SEED_PARAM.to_string(),
                        message: format!(
                            "the seed is given twice with different values ({seed} and {param}); give it once"
                        ),
                    });
                }
                param
            }
        };
        Ok(Built { sim, scenario: scenario.info.clone(), seed, overrides, reset_applied })
    }
}
