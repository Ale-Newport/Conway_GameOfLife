//! Experiment presets stored as RON files in `scenarios/evolution/`.
//!
//! A scenario file is a RON struct with a header (`id`, `name`,
//! `description`, `seed`) and an [`EvolutionConfig`] that lists only the
//! values differing from the defaults:
//!
//! ```ron
//! (
//!     id: "my_experiment",
//!     name: "My experiment",
//!     description: "What it shows.",
//!     seed: 7,
//!     config: (world: (width: 64, height: 48), predators: (initial: 20)),
//! )
//! ```
//!
//! [`builtin`] returns the six presets shipped with the crate (embedded with
//! `include_str!`, so the application works without the folder);
//! [`load_dir`] reads every `*.ron` file of a directory.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::EvolutionConfig;
use crate::error::EvolutionError;
use crate::sim::EvolutionSim;

/// Largest scenario file read from disk.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// The built-in presets: `(file name, contents)`.
pub const BUILTIN_FILES: [(&str, &str); 6] = [
    ("speed_selection.ron", include_str!("../../../scenarios/evolution/speed_selection.ron")),
    ("thermal_adaptation.ron", include_str!("../../../scenarios/evolution/thermal_adaptation.ron")),
    ("arms_race.ron", include_str!("../../../scenarios/evolution/arms_race.ron")),
    ("food_scarcity.ron", include_str!("../../../scenarios/evolution/food_scarcity.ron")),
    ("island_evolution.ron", include_str!("../../../scenarios/evolution/island_evolution.ron")),
    ("r_vs_k.ron", include_str!("../../../scenarios/evolution/r_vs_k.ron")),
];

/// An experiment preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Stable identifier (lowercase letters, digits, `_` and `-`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the experiment demonstrates and what to watch.
    pub description: String,
    /// Seed of the run.
    pub seed: u64,
    /// Experiment configuration (defaults for omitted fields).
    #[serde(default)]
    pub config: EvolutionConfig,
}

impl Scenario {
    /// Parses and validates a scenario; `source` names the file in errors.
    pub fn parse(text: &str, source: &str) -> Result<Self, EvolutionError> {
        let fail = |reason: String| EvolutionError::Scenario { source_name: source.to_string(), reason };
        let scenario: Scenario = ron::from_str(text).map_err(|e| fail(e.to_string()))?;
        let id_ok = !scenario.id.is_empty()
            && scenario.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if !id_ok {
            return Err(fail(format!("id `{}` must use lowercase letters, digits, `_` or `-`", scenario.id)));
        }
        if scenario.name.trim().is_empty() {
            return Err(fail("the name is empty".into()));
        }
        scenario.config.validate().map_err(|e| fail(e.to_string()))?;
        Ok(scenario)
    }

    /// Builds the experiment's initial state.
    pub fn build(&self) -> Result<EvolutionSim, EvolutionError> {
        EvolutionSim::from_scenario(self)
    }
}

/// The six built-in presets, in a fixed order.
pub fn builtin() -> Result<Vec<Scenario>, EvolutionError> {
    BUILTIN_FILES.iter().map(|(name, text)| Scenario::parse(text, name)).collect()
}

/// The built-in preset with the given id.
pub fn find_builtin(id: &str) -> Result<Scenario, EvolutionError> {
    builtin()?.into_iter().find(|s| s.id == id).ok_or_else(|| EvolutionError::Scenario {
        source_name: id.to_string(),
        reason: "no built-in evolution scenario has this id".into(),
    })
}

/// Reads and validates one scenario file.
pub fn load_file(path: &Path) -> Result<Scenario, EvolutionError> {
    let io = |e: std::io::Error| EvolutionError::Io { path: path.to_path_buf(), reason: e.to_string() };
    let size = std::fs::metadata(path).map_err(io)?.len();
    if size > MAX_FILE_BYTES {
        return Err(EvolutionError::Io {
            path: path.to_path_buf(),
            reason: format!("{size} bytes exceed the {MAX_FILE_BYTES}-byte limit for scenario files"),
        });
    }
    let text = std::fs::read_to_string(path).map_err(io)?;
    Scenario::parse(&text, &path.display().to_string())
}

/// Reads every `*.ron` file of `dir` (sorted by file name). Duplicate ids
/// are rejected.
pub fn load_dir(dir: &Path) -> Result<Vec<Scenario>, EvolutionError> {
    let io = |e: std::io::Error| EvolutionError::Io { path: dir.to_path_buf(), reason: e.to_string() };
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(io)?
        .map(|entry| entry.map(|e| e.path()).map_err(io))
        .collect::<Result<_, _>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "ron") && p.is_file());
    paths.sort();
    let mut scenarios: Vec<Scenario> = Vec::with_capacity(paths.len());
    for path in &paths {
        let scenario = load_file(path)?;
        if scenarios.iter().any(|s| s.id == scenario.id) {
            return Err(EvolutionError::Scenario {
                source_name: path.display().to_string(),
                reason: format!("duplicate scenario id `{}`", scenario.id),
            });
        }
        scenarios.push(scenario);
    }
    Ok(scenarios)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReproductionMode;
    use simulation_core::Simulation;

    #[test]
    fn builtin_presets_parse_build_and_run() {
        let all = builtin().unwrap();
        assert_eq!(all.len(), 6);
        let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            ["speed_selection", "thermal_adaptation", "arms_race", "food_scarcity", "island_evolution", "r_vs_k"]
        );
        for s in &all {
            let mut sim = s.build().unwrap();
            sim.advance(3);
            assert_eq!(sim.tick(), 3, "{}", s.id);
            assert!(!sim.world().organisms().is_empty(), "{}", s.id);
        }
        assert_eq!(find_builtin("island_evolution").unwrap().config.reproduction.mode, ReproductionMode::Sexual);
        assert!(find_builtin("nope").is_err());
    }

    #[test]
    fn the_scenario_folder_matches_the_embedded_copies() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/evolution");
        let loaded = load_dir(&dir).unwrap();
        let mut embedded = builtin().unwrap();
        embedded.sort_by(|a, b| a.id.cmp(&b.id));
        let mut loaded_sorted = loaded;
        loaded_sorted.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(loaded_sorted, embedded);
    }

    #[test]
    fn errors_name_the_file_and_position() {
        let e = Scenario::parse(
            "(id: \"x\", name: \"X\", description: \"\", seed: 1, config: (world: (width: 9000)))",
            "bad.ron",
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("bad.ron") && e.contains("world.width"), "{e}");
        let e =
            Scenario::parse("(id: \"x\", name: \"X\", description: \"\", seed: 1, colour: 3)", "typo.ron").unwrap_err();
        assert!(e.to_string().contains("typo.ron") && e.to_string().contains("colour"), "{e}");
        let e = Scenario::parse("(id: \"Bad Id\", name: \"X\", description: \"\", seed: 1)", "id.ron").unwrap_err();
        assert!(e.to_string().contains("lowercase"));
        let e = Scenario::parse("(id: \"x\",\n name: ", "cut.ron").unwrap_err().to_string();
        assert!(e.contains("cut.ron") && e.contains('2'), "line number reported: {e}");
        assert!(load_dir(Path::new("/definitely/not/here")).is_err());
    }
}
