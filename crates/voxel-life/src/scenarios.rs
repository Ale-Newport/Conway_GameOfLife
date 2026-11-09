//! Scenario (preset) files: RON documents under `scenarios/voxel/`.
//!
//! ```ron
//! (
//!     id: "bays-5766-glider",
//!     name: "Bays 5766 glider",
//!     description: "...",
//!     seed: 1,
//!     rule: "5-7/6/2/M",          // any notation accepted by Rule3D::parse
//!     world: dense_torus,         // dense_bounded | dense_torus | sparse_bounded | sparse_unbounded
//!     size: (24, 24, 12),
//!     init: (kind: pattern, pattern: [(1, 0, 0, 1), ...]),
//!     evidence: tested,           // tested | documented | observed | unverified
//!     reference: "Bays (1987), Theorem 7",
//! )
//! ```
//!
//! The same files are embedded in the crate ([`builtin`]) so the application
//! works without the folder.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::VoxelError;
use crate::geometry::WorldKind;
use crate::init::{InitKind, InitSpec, WorldConfig};
use crate::rule::Rule3D;

/// How well the behaviour a scenario describes is established.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// Exact behaviour asserted by this crate's tests (periods,
    /// displacements, population sequences, theorems).
    Tested,
    /// Behaviour stated in a primary source and not reproduced exactly here.
    Documented,
    /// Qualitative behaviour seen in the research simulations for particular
    /// seeds and boundaries; other seeds may behave differently.
    Observed,
    /// No verified behaviour.
    Unverified,
}

impl Evidence {
    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tested => "behaviour verified by tests",
            Self::Documented => "behaviour documented in the literature",
            Self::Observed => "behaviour observed for some seeds (not guaranteed)",
            Self::Unverified => "behaviour unverified",
        }
    }
}

/// A preset world: rule, world configuration and initial state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Stable identifier (lowercase letters, digits, `-`, `_`, `.`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the scenario shows.
    pub description: String,
    /// Seed of the random initial state and of seedless randomize commands.
    pub seed: u64,
    /// The rule (any accepted notation; stored canonically).
    pub rule: Rule3D,
    /// Storage and edges.
    pub world: WorldKind,
    /// World size `(W, H, D)`.
    pub size: [u32; 3],
    /// Initial state.
    #[serde(default)]
    pub init: InitSpec,
    /// How well the described behaviour is established.
    pub evidence: Evidence,
    /// Source of the rule and of the described behaviour.
    #[serde(default)]
    pub reference: String,
}

impl Scenario {
    /// World configuration of the scenario.
    pub fn config(&self) -> WorldConfig {
        WorldConfig { kind: self.world, size: self.size, init: self.init.clone(), seed: self.seed }
    }

    /// Checks identifiers, world configuration and pattern voxels.
    pub fn validate(&self) -> Result<(), VoxelError> {
        let fail = |message: String| VoxelError::Scenario { source_name: self.id.clone(), message };
        if self.id.is_empty()
            || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c))
        {
            return Err(fail(format!(
                "id `{}` must be non-empty and use only lowercase letters, digits, `-`, `_` and `.`",
                self.id
            )));
        }
        if self.name.trim().is_empty() {
            return Err(fail("the name is empty".into()));
        }
        if i64::try_from(self.seed).is_err() {
            return Err(fail(format!("seed {} must be at most 2^63 − 1", self.seed)));
        }
        let config = self.config();
        config.validate().map_err(|e| fail(e.to_string()))?;
        if self.init.kind == InitKind::Pattern {
            let bounds = self.world.bounds(self.size);
            let centre = config.centre();
            for voxel in &self.init.pattern {
                let [dx, dy, dz] = voxel.offset;
                if voxel.state == 0 || u16::from(voxel.state) >= self.rule.states() {
                    return Err(fail(format!(
                        "pattern voxel ({dx}, {dy}, {dz}) has state {}, but rule {} has states 1 to {}",
                        voxel.state,
                        self.rule,
                        self.rule.states() - 1
                    )));
                }
                let pos = [0, 1, 2].map(|a| i64::from(centre[a]) + i64::from(voxel.offset[a]));
                let inside = (0..3).all(|a| (i64::from(bounds.min[a])..=i64::from(bounds.max[a])).contains(&pos[a]));
                if !inside {
                    return Err(fail(format!("pattern voxel ({dx}, {dy}, {dz}) lies outside the world {bounds}")));
                }
            }
        }
        Ok(())
    }
}

/// Parses and validates a scenario. `source_name` is used in messages.
pub fn parse(text: &str, source_name: &str) -> Result<Scenario, VoxelError> {
    let scenario: Scenario = ron::from_str(text)
        .map_err(|e| VoxelError::Scenario { source_name: source_name.to_string(), message: e.to_string() })?;
    scenario.validate().map_err(|e| match e {
        VoxelError::Scenario { message, .. } => VoxelError::Scenario { source_name: source_name.to_string(), message },
        other => other,
    })?;
    Ok(scenario)
}

/// Reads one scenario file.
pub fn load_file(path: &Path) -> Result<Scenario, VoxelError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| VoxelError::Io { path: path.display().to_string(), message: e.to_string() })?;
    parse(&text, &path.display().to_string())
}

/// Reads every `*.ron` file of a directory, in file-name order. Fails on the
/// first invalid file or on duplicate ids.
pub fn load_dir(dir: &Path) -> Result<Vec<Scenario>, VoxelError> {
    let io = |e: std::io::Error| VoxelError::Io { path: dir.display().to_string(), message: e.to_string() };
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "ron") {
            paths.push(path);
        }
    }
    paths.sort();
    let scenarios = paths.iter().map(|p| load_file(p)).collect::<Result<Vec<_>, _>>()?;
    check_unique(&scenarios)?;
    Ok(scenarios)
}

fn check_unique(scenarios: &[Scenario]) -> Result<(), VoxelError> {
    for (i, a) in scenarios.iter().enumerate() {
        if scenarios[..i].iter().any(|b| b.id == a.id) {
            return Err(VoxelError::Scenario { source_name: a.id.clone(), message: "duplicate scenario id".into() });
        }
    }
    Ok(())
}

macro_rules! embedded {
    ($($file:literal),* $(,)?) => {
        &[$(($file, include_str!(concat!("../../../scenarios/voxel/", $file)))),*]
    };
}

/// Embedded copies of `scenarios/voxel/*.ron`, in file-name order.
const BUILTIN: &[(&str, &str)] = embedded![
    "builder-1.ron",
    "clouds-1.ron",
    "crystal-growth-1.ron",
    "life-4555-glider.ron",
    "life-5766-glider.ron",
    "life-5766-soup.ron",
    "life-in-5766-two-planes.ron",
    "rokicki-ship.ron",
    "single-point-replication.ron",
    "softology-445.ron",
    "softology-amoeba.ron",
    "softology-pyroclastic.ron",
];

/// The scenarios shipped with the crate (embedded copies of
/// `scenarios/voxel/`).
pub fn builtin() -> Result<Vec<Scenario>, VoxelError> {
    let scenarios = BUILTIN.iter().map(|(file, text)| parse(text, file)).collect::<Result<Vec<_>, _>>()?;
    check_unique(&scenarios)?;
    Ok(scenarios)
}

/// The embedded scenario with the given id.
pub fn builtin_by_id(id: &str) -> Result<Scenario, VoxelError> {
    builtin()?.into_iter().find(|s| s.id == id).ok_or_else(|| VoxelError::Scenario {
        source_name: id.to_string(),
        message: "no built-in scenario has this id".into(),
    })
}
