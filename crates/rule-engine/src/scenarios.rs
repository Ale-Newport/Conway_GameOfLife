//! Preset scenarios (`scenarios/custom-life/*.ron`).
//!
//! A scenario file is RON with a header and a [`LifeSystem`]:
//!
//! ```ron
//! (
//!     id: "bacterial-growth",
//!     name: "Bacterial growth",
//!     description: "...",
//!     seed: 42,
//!     expected: "...",          // optional: the dynamics to look for
//!     tags: ["ecology"],        // optional
//!     system: ( name: "...", entity_types: [...], world: (...), ... ),
//! )
//! ```
//!
//! [`builtin`] returns copies embedded at compile time (the application
//! works without the folder); [`load_dir`] reads a folder. Loading
//! validates the system by compiling it, so a broken file is reported with
//! its name and every diagnostic.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::compile::compile;
use crate::def::LifeSystem;
use crate::error::RuleEngineError;

/// Maximum accepted scenario file size.
pub const MAX_SCENARIO_BYTES: u64 = 4 * 1024 * 1024;

/// A preset: header plus system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Stable identifier (file stem by convention).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the scenario shows.
    pub description: String,
    /// Default seed.
    pub seed: u64,
    /// The qualitative dynamics to look for.
    #[serde(default)]
    pub expected: String,
    /// Free tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The system.
    pub system: LifeSystem,
}

impl Scenario {
    /// Parses and validates a scenario document; `origin` names it in errors.
    pub fn from_ron(text: &str, origin: &str) -> Result<Self, RuleEngineError> {
        let scenario: Scenario = ron::from_str(text)
            .map_err(|e| RuleEngineError::Scenario { file: origin.to_string(), message: e.to_string() })?;
        compile(&scenario.system)
            .map_err(|e| RuleEngineError::Scenario { file: origin.to_string(), message: e.to_string() })?;
        Ok(scenario)
    }

    /// Writes the scenario as pretty RON.
    pub fn to_ron(&self) -> Result<String, RuleEngineError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| RuleEngineError::Ron(e.to_string()))
    }
}

const BUILTIN: [(&str, &str); 7] = [
    ("bacterial-growth.ron", include_str!("../../../scenarios/custom-life/bacterial-growth.ron")),
    ("virus-and-cells.ron", include_str!("../../../scenarios/custom-life/virus-and-cells.ron")),
    ("competitive-colonies.ron", include_str!("../../../scenarios/custom-life/competitive-colonies.ron")),
    ("alien-ecosystem.ron", include_str!("../../../scenarios/custom-life/alien-ecosystem.ron")),
    ("producers-consumers.ron", include_str!("../../../scenarios/custom-life/producers-consumers.ron")),
    ("artificial-chemistry.ron", include_str!("../../../scenarios/custom-life/artificial-chemistry.ron")),
    ("robot-swarm.ron", include_str!("../../../scenarios/custom-life/robot-swarm.ron")),
];

/// The seven built-in presets (embedded copies), in a fixed order.
pub fn builtin() -> Result<Vec<Scenario>, RuleEngineError> {
    BUILTIN.iter().map(|(file, text)| Scenario::from_ron(text, file)).collect()
}

/// One built-in preset by id.
pub fn builtin_by_id(id: &str) -> Result<Scenario, RuleEngineError> {
    builtin()?.into_iter().find(|s| s.id == id).ok_or_else(|| RuleEngineError::Scenario {
        file: id.to_string(),
        message: format!(
            "no built-in custom-life scenario with this id (known: {})",
            BUILTIN.iter().map(|(f, _)| f.trim_end_matches(".ron")).collect::<Vec<_>>().join(", ")
        ),
    })
}

/// Reads one scenario file.
pub fn load_file(path: &Path) -> Result<Scenario, RuleEngineError> {
    let shown = path.display().to_string();
    let io = |e: std::io::Error| RuleEngineError::Io { path: shown.clone(), reason: e.to_string() };
    let size = std::fs::metadata(path).map_err(io)?.len();
    if size > MAX_SCENARIO_BYTES {
        return Err(RuleEngineError::Scenario {
            file: shown,
            message: format!("file is {size} bytes; the limit is {MAX_SCENARIO_BYTES}"),
        });
    }
    let text = std::fs::read_to_string(path).map_err(io)?;
    Scenario::from_ron(&text, &shown)
}

/// Reads every `*.ron` file of a folder, sorted by file name. The first
/// invalid file aborts loading with its name and diagnostics.
pub fn load_dir(dir: &Path) -> Result<Vec<Scenario>, RuleEngineError> {
    let shown = dir.display().to_string();
    let entries =
        std::fs::read_dir(dir).map_err(|e| RuleEngineError::Io { path: shown.clone(), reason: e.to_string() })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| RuleEngineError::Io { path: shown.clone(), reason: e.to_string() })?;
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "ron") && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    paths.iter().map(|p| load_file(p)).collect()
}
