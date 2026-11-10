//! Metapixel scenarios: RON files under `scenarios/metapixel/`, also
//! embedded in the crate so the mode works without the folder.
//!
//! ```text
//! (
//!     id: "metapixel.blinker",          // unique id
//!     name: "Meta-blinker",
//!     description: "...",
//!     rule: "B3/S23",                   // macro rule (default B3/S23)
//!     neighbourhood: "moore",           // see Neighbourhood::parse (default moore)
//!     width: 3, height: 3,              // tiles
//!     init: Rows(["...", "OOO", "..."]),// or Random(density: 0.4) or Empty
//!     seed: 1,                          // generator seed (default 1)
//!     step: "35328",                    // generations per tick (default one metagen)
//!     notes: ["..."],                   // optional
//! )
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::error::MetapixelError;
use crate::macro_grid::MacroGrid;
use crate::programming::Programming;
use crate::sim::{StepSize, WorldConfig};

/// Largest scenario file read from disk.
pub const MAX_FILE_BYTES: u64 = 1 << 20;

/// Initial macro pattern of a scenario.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ScenarioInit {
    /// Explicit rows of `.` (OFF) and `O` (ON); exactly `height` rows of
    /// `width` characters.
    Rows(Vec<String>),
    /// Each tile ON with probability `density` (seeded by `seed`).
    Random {
        /// Probability of an ON tile in `[0, 1]`.
        density: f64,
    },
    /// Every tile OFF.
    Empty,
}

fn default_rule() -> String {
    "B3/S23".into()
}

fn default_neighbourhood() -> String {
    "moore".into()
}

fn default_seed() -> u64 {
    1
}

fn default_step() -> String {
    crate::spec::PERIOD.to_string()
}

/// One metapixel scenario.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Unique identifier (`metapixel.<name>`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// One-paragraph description.
    pub description: String,
    /// Macro rule.
    #[serde(default = "default_rule")]
    pub rule: String,
    /// Macro neighbourhood.
    #[serde(default = "default_neighbourhood")]
    pub neighbourhood: String,
    /// Grid width in tiles.
    pub width: u32,
    /// Grid height in tiles.
    pub height: u32,
    /// Initial macro pattern.
    pub init: ScenarioInit,
    /// Random generator seed.
    #[serde(default = "default_seed")]
    pub seed: u64,
    /// Generations per tick: a power of two up to `"32768"`, or `"35328"`
    /// (one metageneration, the default).
    #[serde(default = "default_step")]
    pub step: String,
    /// Sources, credits and remarks.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl Scenario {
    fn error(&self, message: impl Into<String>) -> MetapixelError {
        MetapixelError::Scenario { source_name: self.id.clone(), message: message.into() }
    }

    /// The world configuration (validated).
    ///
    /// # Errors
    /// [`MetapixelError::Scenario`] for an invalid rule, neighbourhood or size.
    pub fn config(&self) -> Result<WorldConfig, MetapixelError> {
        let programming = Programming::parse(&self.rule, &self.neighbourhood).map_err(|e| self.error(e.to_string()))?;
        MacroGrid::new(self.width, self.height).map_err(|e| self.error(e.to_string()))?;
        Ok(WorldConfig { programming, width: self.width, height: self.height, seed: self.seed })
    }

    /// Generations per tick.
    ///
    /// # Errors
    /// [`MetapixelError::Scenario`] for an unsupported step.
    pub fn step_size(&self) -> Result<StepSize, MetapixelError> {
        StepSize::parse(&self.step).ok_or_else(|| {
            self.error(format!("step `{}` must be a power of two up to 32768, or 35328 (one metagen)", self.step))
        })
    }

    /// The generation-0 macro pattern; random patterns draw from `rng`.
    ///
    /// # Errors
    /// [`MetapixelError::Scenario`] if the rows do not match the size.
    pub fn initial_pattern(&self, rng: &mut SimRng) -> Result<MacroGrid, MetapixelError> {
        match &self.init {
            ScenarioInit::Rows(rows) => {
                if rows.len() != self.height as usize {
                    return Err(self.error(format!("{} rows given for height {}", rows.len(), self.height)));
                }
                if let Some((i, row)) = rows.iter().enumerate().find(|(_, r)| r.chars().count() != self.width as usize)
                {
                    return Err(self.error(format!(
                        "row {} has {} cells, expected width {}",
                        i + 1,
                        row.chars().count(),
                        self.width
                    )));
                }
                MacroGrid::from_rows(rows).map_err(|e| self.error(e.to_string()))
            }
            ScenarioInit::Random { density } => {
                MacroGrid::random(self.width, self.height, *density, rng).map_err(|e| self.error(e.to_string()))
            }
            ScenarioInit::Empty => MacroGrid::new(self.width, self.height).map_err(|e| self.error(e.to_string())),
        }
    }

    /// Checks everything that [`crate::MetapixelSim::from_scenario`] checks
    /// (except the tiles).
    ///
    /// # Errors
    /// The first problem found.
    pub fn validate(&self) -> Result<(), MetapixelError> {
        if !self.id.starts_with("metapixel.") {
            return Err(self.error("ids must start with `metapixel.`"));
        }
        self.config()?;
        self.step_size()?;
        self.initial_pattern(&mut SimRng::derive(self.seed, crate::sim::SCENARIO_STREAM))?;
        Ok(())
    }

    /// Pretty RON text of the scenario.
    ///
    /// # Errors
    /// [`MetapixelError::Scenario`] if serialisation fails.
    pub fn to_ron(&self) -> Result<String, MetapixelError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).map_err(|e| self.error(e.to_string()))
    }
}

/// Parses and validates scenario text; `source_name` appears in errors.
///
/// # Errors
/// [`MetapixelError::Scenario`] with the RON position or the validation
/// problem.
pub fn parse(text: &str, source_name: &str) -> Result<Scenario, MetapixelError> {
    let scenario: Scenario = ron::from_str(text)
        .map_err(|e| MetapixelError::Scenario { source_name: source_name.to_string(), message: e.to_string() })?;
    scenario.validate()?;
    Ok(scenario)
}

/// Reads and parses one scenario file.
///
/// # Errors
/// [`MetapixelError::Scenario`] for unreadable, oversized or invalid files.
pub fn load_file(path: &Path) -> Result<Scenario, MetapixelError> {
    let name = path.display().to_string();
    let fail = |message: String| MetapixelError::Scenario { source_name: name.clone(), message };
    let size = std::fs::metadata(path).map_err(|e| fail(e.to_string()))?.len();
    if size > MAX_FILE_BYTES {
        return Err(fail(format!("file is {size} bytes, the limit is {MAX_FILE_BYTES}")));
    }
    let text = std::fs::read_to_string(path).map_err(|e| fail(e.to_string()))?;
    parse(&text, &name)
}

/// Reads every `*.ron` file of a directory, sorted by file name.
///
/// # Errors
/// The first unreadable or invalid file.
pub fn load_dir(dir: &Path) -> Result<Vec<Scenario>, MetapixelError> {
    let fail = |message: String| MetapixelError::Scenario { source_name: dir.display().to_string(), message };
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| fail(e.to_string()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .collect();
    paths.sort();
    paths.iter().map(|p| load_file(p)).collect()
}

/// The scenario files shipped in `scenarios/metapixel/`, embedded.
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    ("blinker.ron", include_str!("../../../scenarios/metapixel/blinker.ron")),
    ("block.ron", include_str!("../../../scenarios/metapixel/block.ron")),
    ("glider.ron", include_str!("../../../scenarios/metapixel/glider.ron")),
    ("galaxy.ron", include_str!("../../../scenarios/metapixel/galaxy.ron")),
    ("soup.ron", include_str!("../../../scenarios/metapixel/soup.ron")),
    ("parity-von-neumann.ron", include_str!("../../../scenarios/metapixel/parity-von-neumann.ron")),
    ("b0-single-tile.ron", include_str!("../../../scenarios/metapixel/b0-single-tile.ron")),
];

/// Every embedded scenario, in [`BUILTIN_FILES`] order.
///
/// # Errors
/// Only if an embedded file is invalid (checked by the test suite).
pub fn builtin() -> Result<Vec<Scenario>, MetapixelError> {
    BUILTIN_FILES.iter().map(|(name, text)| parse(text, name)).collect()
}

/// The embedded scenario with `id`.
///
/// # Errors
/// [`MetapixelError::Scenario`] if no embedded scenario has that id.
pub fn builtin_by_id(id: &str) -> Result<Scenario, MetapixelError> {
    builtin()?.into_iter().find(|s| s.id == id).ok_or_else(|| MetapixelError::Scenario {
        source_name: id.to_string(),
        message: "no built-in metapixel scenario has this id".into(),
    })
}
