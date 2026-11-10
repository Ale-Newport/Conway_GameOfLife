//! The table of every model of the workspace.
//!
//! The table only records what a model's descriptor cannot tell: which
//! crate implements it, where its documentation and scenario files live and
//! which built-in scenario is its default. Name, family, summary, rules,
//! parameters and metrics come from the model's own
//! [`simulation_core::ModelDescriptor`] (see [`crate::Registry::model_info`]).

use serde::Serialize;
use simulation_core::{ModelDescriptor, ModelFamily};

/// Which model crate parses the scenario files of a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Backend {
    /// `cellular-automata` scenarios (Life, Life-like, Generations,
    /// Wireworld, Langton's ant, Wolfram).
    CellularAutomata,
    /// `metapixel::MetapixelSim`.
    Metapixel,
    /// `continuous::ContinuousSim` (Lenia, SmoothLife, Flow-Lenia).
    Continuous,
    /// `voxel_life::VoxelLife`.
    Voxel,
    /// `ecosystem::EcosystemSim`.
    Ecosystem,
    /// `rule_engine::CustomLifeSim`.
    CustomLife,
    /// `evolution::EvolutionSim`.
    Evolution,
    /// `civilization::CivilizationSim`.
    Civilization,
}

/// Static facts about one model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct ModelSpec {
    /// Model id, as reported by the model's descriptor and snapshots.
    pub id: &'static str,
    /// Crate implementing the model.
    pub crate_name: &'static str,
    /// Documentation, relative to the workspace root.
    pub docs: &'static str,
    /// Folder holding the model's scenarios, relative to a scenario root
    /// (`scenarios/<folder>/`). The `hashlife` and `environment` folders
    /// have no files: their scenarios are built from library data.
    pub folder: &'static str,
    /// Local id of the default scenario (within `folder`).
    pub default_scenario: &'static str,
}

const fn spec(
    id: &'static str,
    crate_name: &'static str,
    docs: &'static str,
    folder: &'static str,
    default_scenario: &'static str,
) -> ModelSpec {
    ModelSpec { id, crate_name, docs, folder, default_scenario }
}

/// Every model, in presentation order.
pub const MODELS: [ModelSpec; 17] = [
    spec("life.classic", "cellular-automata", "docs/models/cellular-automata.md", "conway", "glider-gun-showcase"),
    spec("ca.life_like", "cellular-automata", "docs/models/cellular-automata.md", "ca-lab", "highlife-replicator"),
    spec("ca.generations", "cellular-automata", "docs/models/cellular-automata.md", "ca-lab", "brians-brain"),
    spec("ca.wireworld", "cellular-automata", "docs/models/cellular-automata.md", "ca-lab", "wireworld-circuits"),
    spec("ca.langtons_ant", "cellular-automata", "docs/models/cellular-automata.md", "ca-lab", "langtons-ant-highway"),
    spec("ca.wolfram", "cellular-automata", "docs/models/cellular-automata.md", "ca-lab", "wolfram-rule-110"),
    spec("life.hashlife", "hashlife", "docs/models/hashlife.md", "hashlife", "gosper-glider-gun"),
    spec("life.metapixel", "metapixel", "docs/models/metapixel.md", "metapixel", "metapixel.blinker"),
    spec("continuous.lenia", "continuous", "docs/models/continuous.md", "continuous", "lenia.orbium"),
    spec("continuous.smoothlife", "continuous", "docs/models/continuous.md", "continuous", "smoothlife.glider"),
    spec("continuous.flow_lenia", "continuous", "docs/models/continuous.md", "continuous", "flow_lenia.demo"),
    spec("voxel.life", "voxel-life", "docs/models/voxel-life.md", "voxel", "life-5766-glider"),
    spec("environment.world", "physics-world", "docs/models/physics-world.md", "environment", "temperate"),
    spec("ecosystem", "ecosystem", "docs/models/ecosystem.md", "ecosystem", "wolf_sheep_grass"),
    spec("custom_life", "rule-engine", "docs/models/rule-engine.md", "custom-life", "bacterial-growth"),
    spec("evolution", "evolution", "docs/models/evolution.md", "evolution", "speed_selection"),
    spec("civilization", "civilization", "docs/models/civilization.md", "civilization", "abundant"),
];

/// Scenario folders whose files are read, with the backend that parses them.
pub(crate) const FILE_FOLDERS: [(&str, Backend); 9] = [
    ("conway", Backend::CellularAutomata),
    ("ca-lab", Backend::CellularAutomata),
    ("metapixel", Backend::Metapixel),
    ("continuous", Backend::Continuous),
    ("voxel", Backend::Voxel),
    ("ecosystem", Backend::Ecosystem),
    ("custom-life", Backend::CustomLife),
    ("evolution", Backend::Evolution),
    ("civilization", Backend::Civilization),
];

/// Looks a model up by id.
pub fn model_spec(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

/// Description of a model taken from its own descriptor plus the static
/// facts of [`ModelSpec`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelInfo {
    /// Model id.
    pub id: String,
    /// Display name (descriptor).
    pub name: String,
    /// Family (descriptor).
    pub family: ModelFamily,
    /// One-paragraph summary (descriptor).
    pub summary: String,
    /// Crate implementing the model.
    pub crate_name: String,
    /// Documentation path relative to the workspace root.
    pub docs: String,
    /// Scenario folder (`scenarios/<folder>/`).
    pub scenario_folder: String,
    /// Qualified id of the default scenario.
    pub default_scenario: String,
    /// Whether runs are reproducible for a fixed seed (descriptor).
    pub deterministic: bool,
    /// Unit of one tick (descriptor).
    pub tick_unit: String,
    /// Simulated time per tick (descriptor).
    pub dt: f64,
    /// The complete descriptor of the default scenario.
    pub descriptor: ModelDescriptor,
}

impl ModelInfo {
    pub(crate) fn new(spec: &ModelSpec, descriptor: ModelDescriptor) -> Self {
        Self {
            id: spec.id.to_string(),
            name: descriptor.name.clone(),
            family: descriptor.family,
            summary: descriptor.summary.clone(),
            crate_name: spec.crate_name.to_string(),
            docs: spec.docs.to_string(),
            scenario_folder: spec.folder.to_string(),
            default_scenario: format!("{}/{}", spec.folder, spec.default_scenario),
            deterministic: descriptor.deterministic,
            tick_unit: descriptor.tick_unit.clone(),
            dt: descriptor.dt,
            descriptor,
        }
    }
}
