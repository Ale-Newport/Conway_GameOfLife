//! Scenario files (RON) for the Classic Conway mode (`scenarios/conway/`)
//! and the CA Laboratory (`scenarios/ca-lab/`).
//!
//! ```ron
//! (
//!     id: "glider-gun-showcase",
//!     name: "Glider gun showcase",
//!     description: "...",
//!     seed: 1,
//!     model: Classic(
//!         topology: infinite,
//!         init: [Pattern(source: Library("gosper-glider-gun"), x: 0, y: 0)],
//!     ),
//! )
//! ```
//!
//! `model` selects the simulation and holds its configuration:
//! `Classic` / `LifeLike` ([`LifeConfig`], model ids `life.classic` and
//! `ca.life_like`), `Generations` ([`GenerationsConfig`]), `Wireworld`
//! ([`WireworldConfig`]), `Ant` ([`AntConfig`]) or `Wolfram`
//! ([`WolframConfig`]). Every file of both folders is also embedded in the
//! crate ([`builtin_scenarios`]), so the application works without the
//! folder. Pattern ids are resolved by a [`PatternLibrary`].

use std::path::Path;

use serde::{Deserialize, Serialize};
use simulation_core::{DynSimulation, erase};

use crate::ant::{ANT_MODEL_ID, AntConfig, LangtonsAntSim};
use crate::error::CaError;
use crate::generations::{GENERATIONS_MODEL_ID, GenerationsConfig, GenerationsSim};
use crate::life::{LifeConfig, LifeMode, LifeSim, PatternLibrary};
use crate::wireworld::{WIREWORLD_MODEL_ID, WireworldConfig, WireworldSim};
use crate::wolfram::{WOLFRAM_MODEL_ID, WolframConfig, WolframSim};

/// Largest scenario file read from disk.
const MAX_FILE_BYTES: u64 = 4 << 20;

/// The model of a scenario and its configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ScenarioModel {
    /// Classic Conway mode ([`LifeSim`], `life.classic`).
    Classic(LifeConfig),
    /// Life-like rules of the CA Laboratory ([`LifeSim`], `ca.life_like`).
    LifeLike(LifeConfig),
    /// Generations rules ([`GenerationsSim`]).
    Generations(GenerationsConfig),
    /// Wireworld ([`WireworldSim`]).
    Wireworld(WireworldConfig),
    /// Langton's ant and turmites ([`LangtonsAntSim`]).
    Ant(AntConfig),
    /// Elementary cellular automata ([`WolframSim`]).
    Wolfram(WolframConfig),
}

/// A preset: model, configuration, seed and description.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Stable identifier (lowercase letters, digits, `-`, `_`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the scenario shows.
    pub description: String,
    /// Seed of random content and of seedless randomize commands.
    #[serde(default)]
    pub seed: u64,
    /// Model and configuration.
    pub model: ScenarioModel,
    /// Sources of the patterns and of the behaviour described.
    #[serde(default)]
    pub reference: String,
}

/// A built scenario, typed.
pub enum BuiltSimulation {
    /// [`LifeSim`].
    Life(LifeSim),
    /// [`GenerationsSim`].
    Generations(GenerationsSim),
    /// [`WireworldSim`].
    Wireworld(WireworldSim),
    /// [`LangtonsAntSim`].
    Ant(LangtonsAntSim),
    /// [`WolframSim`].
    Wolfram(WolframSim),
}

impl BuiltSimulation {
    /// The simulation behind the object-safe interface.
    pub fn into_dyn(self) -> Box<dyn DynSimulation> {
        match self {
            Self::Life(s) => erase(s),
            Self::Generations(s) => erase(s),
            Self::Wireworld(s) => erase(s),
            Self::Ant(s) => erase(s),
            Self::Wolfram(s) => erase(s),
        }
    }
}

impl Scenario {
    /// The model id the scenario builds.
    pub fn model_id(&self) -> &'static str {
        match &self.model {
            ScenarioModel::Classic(_) => LifeMode::Classic.model_id(),
            ScenarioModel::LifeLike(_) => LifeMode::Lab.model_id(),
            ScenarioModel::Generations(_) => GENERATIONS_MODEL_ID,
            ScenarioModel::Wireworld(_) => WIREWORLD_MODEL_ID,
            ScenarioModel::Ant(_) => ANT_MODEL_ID,
            ScenarioModel::Wolfram(_) => WOLFRAM_MODEL_ID,
        }
    }

    /// Builds the simulation with the scenario's seed.
    ///
    /// # Errors
    /// Invalid configuration or unknown pattern ids.
    pub fn build(&self, library: &PatternLibrary) -> Result<BuiltSimulation, CaError> {
        self.build_with_seed(library, self.seed)
    }

    /// Builds the simulation with another seed.
    ///
    /// # Errors
    /// See [`Scenario::build`].
    pub fn build_with_seed(&self, library: &PatternLibrary, seed: u64) -> Result<BuiltSimulation, CaError> {
        Ok(match &self.model {
            ScenarioModel::Classic(c) => BuiltSimulation::Life(LifeSim::new(c, seed, library, LifeMode::Classic)?),
            ScenarioModel::LifeLike(c) => BuiltSimulation::Life(LifeSim::new(c, seed, library, LifeMode::Lab)?),
            ScenarioModel::Generations(c) => BuiltSimulation::Generations(GenerationsSim::new(c, seed, library)?),
            ScenarioModel::Wireworld(c) => BuiltSimulation::Wireworld(WireworldSim::new(c, seed, library)?),
            ScenarioModel::Ant(c) => BuiltSimulation::Ant(LangtonsAntSim::new(c, seed)?),
            ScenarioModel::Wolfram(c) => BuiltSimulation::Wolfram(WolframSim::new(c, seed)?),
        })
    }

    /// Builds the simulation behind [`DynSimulation`].
    ///
    /// # Errors
    /// See [`Scenario::build`].
    pub fn build_dyn(&self, library: &PatternLibrary) -> Result<Box<dyn DynSimulation>, CaError> {
        Ok(self.build(library)?.into_dyn())
    }

    fn check_header(&self) -> Result<(), String> {
        let id_ok = !self.id.is_empty()
            && self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !id_ok {
            return Err(format!("id `{}` must be non-empty and use only a-z, 0-9, `-` and `_`", self.id));
        }
        if self.name.trim().is_empty() {
            return Err("the name is empty".into());
        }
        Ok(())
    }
}

fn ron_options() -> ron::Options {
    use ron::extensions::Extensions;
    ron::Options::default().with_default_extension(Extensions::UNWRAP_VARIANT_NEWTYPES | Extensions::IMPLICIT_SOME)
}

/// Parses a scenario and checks its header. `source_name` labels errors.
/// Building it ([`Scenario::build`]) validates the configuration.
///
/// # Errors
/// RON syntax errors (with line and column) and header problems.
pub fn parse(text: &str, source_name: &str) -> Result<Scenario, CaError> {
    let fail = |message: String| CaError::Scenario { source_name: source_name.to_string(), message };
    let scenario: Scenario = ron_options().from_str(text).map_err(|e| fail(e.to_string()))?;
    scenario.check_header().map_err(fail)?;
    Ok(scenario)
}

/// Serialises a scenario as pretty RON (readable by [`parse`]).
///
/// # Errors
/// Serialisation failures (none for well-formed scenarios).
pub fn to_ron(scenario: &Scenario) -> Result<String, CaError> {
    let config = ron::ser::PrettyConfig::default().struct_names(false);
    ron_options()
        .to_string_pretty(scenario, config)
        .map_err(|e| CaError::Scenario { source_name: scenario.id.clone(), message: e.to_string() })
}

/// Reads one scenario file.
///
/// # Errors
/// I/O errors, oversized files and [`parse`] errors.
pub fn load_file(path: &Path) -> Result<Scenario, CaError> {
    let io = |e: std::io::Error| CaError::Io { path: path.display().to_string(), message: e.to_string() };
    if std::fs::metadata(path).map_err(io)?.len() > MAX_FILE_BYTES {
        return Err(CaError::Scenario {
            source_name: path.display().to_string(),
            message: format!("the file is larger than {MAX_FILE_BYTES} bytes"),
        });
    }
    let text = std::fs::read_to_string(path).map_err(io)?;
    parse(&text, &path.display().to_string())
}

/// Reads every `*.ron` file of a directory, sorted by file name.
///
/// # Errors
/// The first unreadable or invalid file, or duplicate ids.
pub fn load_dir(dir: &Path) -> Result<Vec<Scenario>, CaError> {
    let io = |e: std::io::Error| CaError::Io { path: dir.display().to_string(), message: e.to_string() };
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "ron") {
            paths.push(path);
        }
    }
    paths.sort();
    let scenarios = paths.iter().map(|p| load_file(p)).collect::<Result<Vec<_>, _>>()?;
    check_unique(&scenarios, &dir.display().to_string())?;
    Ok(scenarios)
}

fn check_unique(scenarios: &[Scenario], source: &str) -> Result<(), CaError> {
    let mut ids: Vec<&str> = scenarios.iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    match ids.windows(2).find(|w| w[0] == w[1]) {
        Some(w) => {
            Err(CaError::Scenario { source_name: source.to_string(), message: format!("duplicate id `{}`", w[0]) })
        }
        None => Ok(()),
    }
}

/// Embedded copies of `scenarios/conway/*.ron`: `(file name, text)`.
pub const CONWAY_SCENARIO_FILES: [(&str, &str); 9] = [
    ("bounded-r-pentomino.ron", include_str!("../../../scenarios/conway/bounded-r-pentomino.ron")),
    ("breeder-hashlife.ron", include_str!("../../../scenarios/conway/breeder-hashlife.ron")),
    ("glider-gun-showcase.ron", include_str!("../../../scenarios/conway/glider-gun-showcase.ron")),
    ("infinite-puffer-world.ron", include_str!("../../../scenarios/conway/infinite-puffer-world.ron")),
    ("klein-bottle-symmetric-soup.ron", include_str!("../../../scenarios/conway/klein-bottle-symmetric-soup.ron")),
    ("large-bit-soup.ron", include_str!("../../../scenarios/conway/large-bit-soup.ron")),
    ("methuselah-gallery.ron", include_str!("../../../scenarios/conway/methuselah-gallery.ron")),
    ("pattern-collisions.ron", include_str!("../../../scenarios/conway/pattern-collisions.ron")),
    ("random-soup-torus.ron", include_str!("../../../scenarios/conway/random-soup-torus.ron")),
];

/// Embedded copies of `scenarios/ca-lab/*.ron`: `(file name, text)`.
pub const CA_LAB_SCENARIO_FILES: [(&str, &str); 20] = [
    ("anneal-coarsening.ron", include_str!("../../../scenarios/ca-lab/anneal-coarsening.ron")),
    ("brians-brain.ron", include_str!("../../../scenarios/ca-lab/brians-brain.ron")),
    ("day-and-night-rocket.ron", include_str!("../../../scenarios/ca-lab/day-and-night-rocket.ron")),
    ("diamoeba.ron", include_str!("../../../scenarios/ca-lab/diamoeba.ron")),
    ("highlife-replicator.ron", include_str!("../../../scenarios/ca-lab/highlife-replicator.ron")),
    ("langtons-ant-highway.ron", include_str!("../../../scenarios/ca-lab/langtons-ant-highway.ron")),
    ("life-without-death.ron", include_str!("../../../scenarios/ca-lab/life-without-death.ron")),
    ("maze.ron", include_str!("../../../scenarios/ca-lab/maze.ron")),
    ("morley-jellyfish.ron", include_str!("../../../scenarios/ca-lab/morley-jellyfish.ron")),
    ("replicator-rule.ron", include_str!("../../../scenarios/ca-lab/replicator-rule.ron")),
    ("seeds.ron", include_str!("../../../scenarios/ca-lab/seeds.ron")),
    ("star-wars.ron", include_str!("../../../scenarios/ca-lab/star-wars.ron")),
    ("turmite-four-ants.ron", include_str!("../../../scenarios/ca-lab/turmite-four-ants.ron")),
    ("turmite-llrr.ron", include_str!("../../../scenarios/ca-lab/turmite-llrr.ron")),
    ("two-by-two.ron", include_str!("../../../scenarios/ca-lab/two-by-two.ron")),
    ("wireworld-circuits.ron", include_str!("../../../scenarios/ca-lab/wireworld-circuits.ron")),
    ("wolfram-rule-110.ron", include_str!("../../../scenarios/ca-lab/wolfram-rule-110.ron")),
    ("wolfram-rule-184.ron", include_str!("../../../scenarios/ca-lab/wolfram-rule-184.ron")),
    ("wolfram-rule-30.ron", include_str!("../../../scenarios/ca-lab/wolfram-rule-30.ron")),
    ("wolfram-rule-90.ron", include_str!("../../../scenarios/ca-lab/wolfram-rule-90.ron")),
];

/// Every embedded scenario (Classic Conway first, then the CA Laboratory).
///
/// # Errors
/// Never for a correct build (the embedded files are checked by tests); a
/// parse failure is reported with the file name.
pub fn builtin_scenarios() -> Result<Vec<Scenario>, CaError> {
    let parse_all = |files: &[(&str, &str)], folder: &str| -> Result<Vec<Scenario>, CaError> {
        files.iter().map(|(name, text)| parse(text, &format!("{folder}/{name}"))).collect()
    };
    let mut all = parse_all(&CONWAY_SCENARIO_FILES, "scenarios/conway")?;
    all.extend(parse_all(&CA_LAB_SCENARIO_FILES, "scenarios/ca-lab")?);
    check_unique(&all, "embedded scenarios")?;
    Ok(all)
}

/// Looks up an embedded scenario by id.
///
/// # Errors
/// See [`builtin_scenarios`]; unknown ids give [`CaError::Scenario`].
pub fn builtin_scenario(id: &str) -> Result<Scenario, CaError> {
    builtin_scenarios()?.into_iter().find(|s| s.id == id).ok_or_else(|| CaError::Scenario {
        source_name: "embedded scenarios".into(),
        message: format!("no scenario with id `{id}`"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_errors_are_reported() {
        let text = r#"(id: "Bad Id", name: "x", description: "", model: Wolfram(rule: 30))"#;
        let err = parse(text, "inline").unwrap_err().to_string();
        assert!(err.contains("inline") && err.contains("Bad Id"), "{err}");
        let text = r#"(id: "ok", name: "x", description: "", model: Wolfram(rule: 30), extra: 1)"#;
        assert!(parse(text, "inline").is_err(), "unknown fields are rejected");
        let text = r#"(id: "ok", name: "x", description: "", model: Wolfram(rule: 30, width: 2))"#;
        let s = parse(text, "inline").unwrap();
        assert!(s.build(&PatternLibrary::builtin()).is_err(), "width 2 is rejected when building");
    }

    #[test]
    fn round_trip_through_ron() {
        let s = Scenario {
            id: "t".into(),
            name: "T".into(),
            description: "d".into(),
            seed: 3,
            model: ScenarioModel::Classic(LifeConfig::default()),
            reference: String::new(),
        };
        let text = to_ron(&s).unwrap();
        assert_eq!(parse(&text, "rt").unwrap(), s);
    }
}
