//! Scenario files (`scenarios/ecosystem/*.ron`).
//!
//! A scenario is a RON struct with a header and an [`EcosystemConfig`]:
//!
//! ```ron
//! (
//!     id: "my_meadow",
//!     name: "My meadow",
//!     description: "What it shows.",
//!     seed: 7,
//!     config: (
//!         world: (width: 64, height: 64, cell_size_m: 10.0),
//!         animals: [(species: "rabbit", count: 40)],
//!         params: {"mutation_rate": 0.05},
//!     ),
//! )
//! ```
//!
//! [`builtin`] returns the presets shipped with the crate (embedded with
//! `include_str!`); [`load_file`] and [`load_dir`] read user files.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::EcosystemConfig;
use crate::error::EcosystemError;
use crate::sim::EcosystemSim;
use crate::species::SpeciesCatalog;
use crate::species::catalog::{parse_ron, read_text, ron_files};

/// The built-in scenarios: `(file name, contents)`, in presentation order
/// (the two classic models, then the biomes, then the experiments).
pub const BUILTIN_FILES: [(&str, &str); 14] = [
    ("wolf_sheep_grass.ron", include_str!("../../../scenarios/ecosystem/wolf_sheep_grass.ron")),
    ("balanced_meadow.ron", include_str!("../../../scenarios/ecosystem/balanced_meadow.ron")),
    ("temperate_forest.ron", include_str!("../../../scenarios/ecosystem/temperate_forest.ron")),
    ("african_savanna.ron", include_str!("../../../scenarios/ecosystem/african_savanna.ron")),
    ("freshwater_lake.ron", include_str!("../../../scenarios/ecosystem/freshwater_lake.ron")),
    ("ocean_ecosystem.ron", include_str!("../../../scenarios/ecosystem/ocean_ecosystem.ron")),
    ("tropical_rainforest.ron", include_str!("../../../scenarios/ecosystem/tropical_rainforest.ron")),
    ("isolated_island.ron", include_str!("../../../scenarios/ecosystem/isolated_island.ron")),
    ("forest_fire.ron", include_str!("../../../scenarios/ecosystem/forest_fire.ron")),
    ("prolonged_drought.ron", include_str!("../../../scenarios/ecosystem/prolonged_drought.ron")),
    ("invasive_species.ron", include_str!("../../../scenarios/ecosystem/invasive_species.ron")),
    ("apex_predator_extinction.ron", include_str!("../../../scenarios/ecosystem/apex_predator_extinction.ron")),
    ("island_evolution.ron", include_str!("../../../scenarios/ecosystem/island_evolution.ron")),
    ("extreme_seasons.ron", include_str!("../../../scenarios/ecosystem/extreme_seasons.ron")),
];

/// An ecosystem preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Stable identifier (lowercase letters, digits, `_` and `-`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the scenario shows and what to watch.
    pub description: String,
    /// Seed of the run.
    pub seed: u64,
    /// Configuration (defaults for omitted fields).
    #[serde(default)]
    pub config: EcosystemConfig,
}

impl Scenario {
    /// Parses a scenario and validates it against the built-in species;
    /// `source` names the file in errors.
    pub fn parse(text: &str, source: &str) -> Result<Self, EcosystemError> {
        Self::parse_with(text, source, &SpeciesCatalog::builtin()?)
    }

    /// Parses a scenario and validates it against a catalogue.
    pub fn parse_with(text: &str, source: &str, catalog: &SpeciesCatalog) -> Result<Self, EcosystemError> {
        let scenario: Scenario = parse_ron(text, source)?;
        let fail = |reason: String| EcosystemError::Parse { source_name: source.to_string(), reason };
        let id_ok = !scenario.id.is_empty()
            && scenario.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if !id_ok {
            return Err(fail(format!("id `{}` must use lowercase letters, digits, `_` or `-`", scenario.id)));
        }
        if scenario.name.trim().is_empty() {
            return Err(fail("the name is empty".into()));
        }
        scenario.config.validate(catalog).map_err(|e| fail(e.to_string()))?;
        let specs = crate::params::param_specs(&scenario.config.rules);
        if let Err(errors) = scenario.config.params.resolve(&specs) {
            let list: Vec<String> = errors.iter().map(ToString::to_string).collect();
            return Err(fail(list.join("; ")));
        }
        Ok(scenario)
    }

    /// Builds the initial state with the built-in species.
    pub fn build(&self) -> Result<EcosystemSim, EcosystemError> {
        EcosystemSim::from_scenario(self, SpeciesCatalog::builtin()?)
    }
}

/// The built-in scenarios, in a fixed order.
pub fn builtin() -> Result<Vec<Scenario>, EcosystemError> {
    let catalog = SpeciesCatalog::builtin()?;
    BUILTIN_FILES.iter().map(|(name, text)| Scenario::parse_with(text, name, &catalog)).collect()
}

/// The built-in scenario with the given id.
pub fn find_builtin(id: &str) -> Result<Scenario, EcosystemError> {
    builtin()?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| EcosystemError::config("scenario", format!("no built-in ecosystem scenario has id `{id}`")))
}

/// Reads and validates one scenario file against a catalogue.
pub fn load_file(path: &Path, catalog: &SpeciesCatalog) -> Result<Scenario, EcosystemError> {
    let text = read_text(path)?;
    Scenario::parse_with(&text, &path.display().to_string(), catalog)
}

/// Reads every `*.ron` file of a directory (sorted by name); duplicate ids
/// are rejected.
pub fn load_dir(dir: &Path, catalog: &SpeciesCatalog) -> Result<Vec<Scenario>, EcosystemError> {
    let mut scenarios: Vec<Scenario> = Vec::new();
    for path in ron_files(dir)? {
        let scenario = load_file(&path, catalog)?;
        if scenarios.iter().any(|s| s.id == scenario.id) {
            return Err(EcosystemError::Parse {
                source_name: path.display().to_string(),
                reason: format!("duplicate scenario id `{}`", scenario.id),
            });
        }
        scenarios.push(scenario);
    }
    Ok(scenarios)
}
