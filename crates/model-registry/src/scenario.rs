//! Scenario discovery: the built-in scenarios of every model crate plus
//! scenario files found in scenario directories.
//!
//! Every scenario has a **qualified id** `<folder>/<id>` (for example
//! `conway/glider-gun-showcase` or `ecosystem/island_evolution`), because
//! local ids are only unique within one folder (`island_evolution` exists in
//! both `ecosystem/` and `evolution/`). A scenario directory has the layout
//! of the repository's `scenarios/` folder: one sub-folder per model family
//! (`conway`, `ca-lab`, `metapixel`, `continuous`, `voxel`, `ecosystem`,
//! `custom-life`, `evolution`, `civilization`) holding RON files in the
//! format of the model crate that owns the folder.
//!
//! `custom-life/` also accepts the plain systems the Custom Life editor
//! saves in the user's library folder.
//!
//! Two folders have no files: `hashlife/` lists the patterns embedded in the
//! `patterns` crate (each loaded into a Hashlife universe) and
//! `environment/` lists the biome presets of `physics-world`.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::assets::Assets;
use crate::error::RegistryError;
use crate::models::{Backend, FILE_FOLDERS};

/// Where a scenario comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "detail")]
pub enum ScenarioSource {
    /// Embedded in the named crate.
    Builtin(String),
    /// Read from this file.
    File(PathBuf),
}

impl ScenarioSource {
    /// Short label (`builtin (crate)` or the path).
    pub fn label(&self) -> String {
        match self {
            Self::Builtin(krate) => format!("builtin ({krate})"),
            Self::File(path) => path.display().to_string(),
        }
    }
}

/// Description of a scenario.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScenarioInfo {
    /// Qualified id `<folder>/<id>`.
    pub id: String,
    /// Id inside its folder.
    pub local_id: String,
    /// Scenario folder.
    pub folder: String,
    /// Model the scenario builds.
    pub model_id: String,
    /// Display name.
    pub title: String,
    /// What the scenario shows.
    pub description: String,
    /// Seed used unless overridden.
    pub seed: u64,
    /// Origin.
    pub source: ScenarioSource,
    /// The source this scenario replaced (a file with the id of a built-in
    /// scenario or of a file in an earlier directory).
    pub replaces: Option<ScenarioSource>,
}

/// Parsed scenario data, in the format of the model crate.
#[derive(Clone, Debug)]
pub(crate) enum ScenarioData {
    Ca(Box<cellular_automata::Scenario>),
    Hashlife(&'static patterns::BuiltinPattern),
    Metapixel(Box<metapixel::Scenario>),
    Continuous(Box<continuous::Scenario>),
    Voxel(Box<voxel_life::Scenario>),
    Environment(physics_world::BiomePreset),
    Ecosystem(Box<ecosystem::Scenario>),
    CustomLife(Box<rule_engine::Scenario>),
    Evolution(Box<evolution::Scenario>),
    Civilization(Box<civilization::Scenario>),
}

/// A scenario ready to build.
#[derive(Clone, Debug)]
pub struct Scenario {
    /// Identity and origin.
    pub info: ScenarioInfo,
    pub(crate) data: ScenarioData,
}

/// A scenario file that could not be used.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScenarioProblem {
    /// The file (or folder).
    pub path: PathBuf,
    /// What is wrong.
    pub message: String,
}

impl std::fmt::Display for ScenarioProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

/// Header fields shared by every scenario format.
struct Header<'a> {
    id: &'a str,
    name: &'a str,
    description: String,
    seed: u64,
    model_id: &'a str,
}

impl ScenarioData {
    fn header(&self) -> Header<'_> {
        match self {
            Self::Ca(s) => Header {
                id: &s.id,
                name: &s.name,
                description: s.description.clone(),
                seed: s.seed,
                model_id: s.model_id(),
            },
            Self::Hashlife(p) => Header {
                id: p.id,
                name: p.name,
                description: format!("{} {}", p.description, p.expected_behavior),
                seed: 0,
                model_id: hashlife::MODEL_ID,
            },
            Self::Metapixel(s) => Header {
                id: &s.id,
                name: &s.name,
                description: s.description.clone(),
                seed: s.seed,
                model_id: metapixel::MODEL_ID,
            },
            Self::Continuous(s) => Header {
                id: &s.header.id,
                name: &s.header.name,
                description: s.header.description.clone(),
                seed: s.header.seed,
                model_id: s.rule.kind().model_id(),
            },
            Self::Voxel(s) => Header {
                id: &s.id,
                name: &s.name,
                description: s.description.clone(),
                seed: s.seed,
                model_id: voxel_life::MODEL_ID,
            },
            Self::Environment(p) => Header {
                id: p.key(),
                name: p.label(),
                description: format!(
                    "The abiotic world alone (terrain, climate, weather, hydrology, fire) generated with the \
                     `{}` biome preset.",
                    p.key()
                ),
                seed: physics_world::WorldConfig::default().seed,
                model_id: physics_world::MODEL_ID,
            },
            Self::Ecosystem(s) => Header {
                id: &s.id,
                name: &s.name,
                description: s.description.clone(),
                seed: s.seed,
                model_id: ecosystem::MODEL_ID,
            },
            Self::CustomLife(s) => Header {
                id: &s.id,
                name: &s.name,
                description: s.description.clone(),
                seed: s.seed,
                model_id: rule_engine::MODEL_ID,
            },
            Self::Evolution(s) => Header {
                id: &s.id,
                name: &s.name,
                description: s.description.clone(),
                seed: s.seed,
                model_id: evolution::MODEL_ID,
            },
            Self::Civilization(s) => Header {
                id: &s.id,
                name: &s.name,
                description: s.description.clone(),
                seed: s.seed,
                model_id: civilization::MODEL_ID,
            },
        }
    }
}

impl Scenario {
    pub(crate) fn new(folder: &str, data: ScenarioData, source: ScenarioSource) -> Self {
        let h = data.header();
        let info = ScenarioInfo {
            id: format!("{folder}/{}", h.id),
            local_id: h.id.to_string(),
            folder: folder.to_string(),
            model_id: h.model_id.to_string(),
            title: h.name.to_string(),
            description: h.description.split_whitespace().collect::<Vec<_>>().join(" "),
            seed: h.seed,
            source,
            replaces: None,
        };
        Self { info, data }
    }
}

/// The built-in scenarios of every model crate, in presentation order, and
/// the problems found (only possible for a broken build).
pub(crate) fn builtin() -> (Vec<Scenario>, Vec<ScenarioProblem>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let mut fail = |what: &str, e: &dyn std::fmt::Display| {
        problems.push(ScenarioProblem { path: PathBuf::from(what), message: e.to_string() });
    };
    let builtin = |krate: &str| ScenarioSource::Builtin(krate.to_string());

    let ca_files = [
        ("conway", &cellular_automata::scenario::CONWAY_SCENARIO_FILES[..]),
        ("ca-lab", &cellular_automata::scenario::CA_LAB_SCENARIO_FILES[..]),
    ];
    for (folder, files) in ca_files {
        for (name, text) in files {
            match cellular_automata::scenario::parse(text, name) {
                Ok(s) => out.push(Scenario::new(folder, ScenarioData::Ca(Box::new(s)), builtin("cellular-automata"))),
                Err(e) => fail(&format!("builtin {folder}/{name}"), &e),
            }
        }
    }
    for pattern in patterns::builtin_patterns() {
        out.push(Scenario::new("hashlife", ScenarioData::Hashlife(pattern), builtin("patterns")));
    }
    match metapixel::scenario::builtin() {
        Ok(list) => out.extend(
            list.into_iter()
                .map(|s| Scenario::new("metapixel", ScenarioData::Metapixel(Box::new(s)), builtin("metapixel"))),
        ),
        Err(e) => fail("builtin metapixel scenarios", &e),
    }
    out.extend(
        continuous::scenario::builtin()
            .into_iter()
            .map(|s| Scenario::new("continuous", ScenarioData::Continuous(Box::new(s)), builtin("continuous"))),
    );
    match voxel_life::scenarios::builtin() {
        Ok(list) => out.extend(
            list.into_iter().map(|s| Scenario::new("voxel", ScenarioData::Voxel(Box::new(s)), builtin("voxel-life"))),
        ),
        Err(e) => fail("builtin voxel scenarios", &e),
    }
    for preset in physics_world::BiomePreset::ALL {
        out.push(Scenario::new("environment", ScenarioData::Environment(preset), builtin("physics-world")));
    }
    match ecosystem::scenario::builtin() {
        Ok(list) => out.extend(
            list.into_iter()
                .map(|s| Scenario::new("ecosystem", ScenarioData::Ecosystem(Box::new(s)), builtin("ecosystem"))),
        ),
        Err(e) => fail("builtin ecosystem scenarios", &e),
    }
    match rule_engine::scenarios::builtin() {
        Ok(list) => out.extend(
            list.into_iter()
                .map(|s| Scenario::new("custom-life", ScenarioData::CustomLife(Box::new(s)), builtin("rule-engine"))),
        ),
        Err(e) => fail("builtin custom-life scenarios", &e),
    }
    match evolution::scenarios::builtin() {
        Ok(list) => out.extend(
            list.into_iter()
                .map(|s| Scenario::new("evolution", ScenarioData::Evolution(Box::new(s)), builtin("evolution"))),
        ),
        Err(e) => fail("builtin evolution scenarios", &e),
    }
    match civilization::scenarios::builtin() {
        Ok(list) => {
            out.extend(list.into_iter().map(|s| {
                Scenario::new("civilization", ScenarioData::Civilization(Box::new(s)), builtin("civilization"))
            }))
        }
        Err(e) => fail("builtin civilization scenarios", &e),
    }
    (out, problems)
}

/// Folder names whose files are read, in scanning order.
pub fn file_folders() -> impl Iterator<Item = &'static str> {
    FILE_FOLDERS.iter().map(|(folder, _)| *folder)
}

fn backend_of(folder: &str) -> Option<Backend> {
    FILE_FOLDERS.iter().find(|(f, _)| *f == folder).map(|(_, b)| *b)
}

/// Parses one scenario file of `folder` with the model crate owning the
/// folder.
///
/// # Errors
/// [`RegistryError::ScenarioFile`] naming the file, for unknown folders,
/// unreadable files and every parse or validation error of the crate.
pub(crate) fn load_file(folder: &str, path: &Path, assets: &Assets) -> Result<Scenario, RegistryError> {
    let fail = |message: String| RegistryError::ScenarioFile { path: path.to_path_buf(), message };
    let backend = backend_of(folder).ok_or_else(|| {
        let known: Vec<&str> = file_folders().collect();
        fail(format!(
            "cannot tell which model reads files of folder `{folder}`; scenario files must be in a folder named \
             one of: {}",
            known.join(", ")
        ))
    })?;
    let source = ScenarioSource::File(path.to_path_buf());
    let data = match backend {
        Backend::CellularAutomata => {
            ScenarioData::Ca(Box::new(cellular_automata::scenario::load_file(path).map_err(|e| fail(e.to_string()))?))
        }
        Backend::Metapixel => {
            ScenarioData::Metapixel(Box::new(metapixel::scenario::load_file(path).map_err(|e| fail(e.to_string()))?))
        }
        Backend::Continuous => {
            ScenarioData::Continuous(Box::new(continuous::scenario::load_file(path).map_err(|e| fail(e.to_string()))?))
        }
        Backend::Voxel => {
            ScenarioData::Voxel(Box::new(voxel_life::scenarios::load_file(path).map_err(|e| fail(e.to_string()))?))
        }
        Backend::Ecosystem => {
            let catalog = assets.species().map_err(|e| fail(e.to_string()))?;
            ScenarioData::Ecosystem(Box::new(
                ecosystem::scenario::load_file(path, catalog).map_err(|e| fail(e.to_string()))?,
            ))
        }
        Backend::CustomLife => ScenarioData::CustomLife(Box::new(custom_life_file(path).map_err(fail)?)),
        Backend::Evolution => {
            ScenarioData::Evolution(Box::new(evolution::scenarios::load_file(path).map_err(|e| fail(e.to_string()))?))
        }
        Backend::Civilization => ScenarioData::Civilization(Box::new(
            civilization::scenarios::load_file(path).map_err(|e| fail(e.to_string()))?,
        )),
    };
    Ok(Scenario::new(folder, data, source))
}

/// A `custom-life` file: a preset (header and system), or a plain
/// [`rule_engine::LifeSystem`] as the Custom Life editor saves it in the
/// user's `scenarios/custom-life/` library. A plain system becomes a
/// scenario with the file stem as id, the system's name and description and
/// the default of the model's `seed` parameter (the file stores no seed).
fn custom_life_file(path: &Path) -> Result<rule_engine::Scenario, String> {
    let preset_error = match rule_engine::scenarios::load_file(path) {
        Ok(scenario) => return Ok(scenario),
        Err(e) => e.to_string(),
    };
    // `load_file` already reported unreadable and oversized files.
    let readable = std::fs::metadata(path).is_ok_and(|m| m.len() <= rule_engine::scenarios::MAX_SCENARIO_BYTES);
    let Some(text) = readable.then(|| std::fs::read_to_string(path).ok()).flatten() else {
        return Err(preset_error);
    };
    let system = rule_engine::LifeSystem::from_ron(&text).map_err(|system_error| {
        format!("neither a Custom Life preset ({preset_error}) nor a saved Custom Life system ({system_error})")
    })?;
    rule_engine::compile::compile(&system).map_err(|e| e.to_string())?;
    let seed = rule_engine::sim::params::param_specs(&system)
        .into_iter()
        .find(|p| p.key == crate::params::SEED_PARAM)
        .and_then(|p| p.default.as_i64())
        .and_then(|s| u64::try_from(s).ok())
        .ok_or("the custom_life model declares no default seed")?;
    let id = path.file_stem().and_then(|s| s.to_str()).ok_or("the file name is not UTF-8")?.to_string();
    Ok(rule_engine::Scenario {
        id,
        name: system.name.clone(),
        description: system.description.clone(),
        seed,
        expected: String::new(),
        tags: Vec::new(),
        system,
    })
}

/// Reads one scenario file whose folder is its parent directory's name.
///
/// # Errors
/// See [`load_file`].
pub(crate) fn load_path(path: &Path, assets: &Assets) -> Result<Scenario, RegistryError> {
    let folder = path.parent().and_then(Path::file_name).and_then(|n| n.to_str()).unwrap_or_default();
    load_file(folder, path, assets)
}

/// `*.ron` files of a folder, sorted by name.
fn ron_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("cannot list the folder: {e}"))? {
        let path = entry.map_err(|e| format!("cannot list the folder: {e}"))?.path();
        if path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ron")) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Adds the scenario files of the scenario directory `root` to `scenarios`.
/// A file whose qualified id is already known replaces the earlier entry
/// (built-in or from an earlier directory); two files of one folder with the
/// same id are a problem and the second is skipped. Missing folders are
/// skipped silently (a scenario directory may hold only some folders).
pub(crate) fn scan_dir(
    root: &Path,
    assets: &Assets,
    scenarios: &mut Vec<Scenario>,
    problems: &mut Vec<ScenarioProblem>,
) {
    for folder in file_folders() {
        let dir = root.join(folder);
        if !dir.is_dir() {
            continue;
        }
        let files = match ron_files(&dir) {
            Ok(files) => files,
            Err(message) => {
                problems.push(ScenarioProblem { path: dir, message });
                continue;
            }
        };
        let mut seen: Vec<String> = Vec::new();
        for path in files {
            let mut scenario = match load_file(folder, &path, assets) {
                Ok(s) => s,
                Err(e) => {
                    let message = match e {
                        RegistryError::ScenarioFile { message, .. } => message,
                        other => other.to_string(),
                    };
                    problems.push(ScenarioProblem { path, message });
                    continue;
                }
            };
            if seen.contains(&scenario.info.id) {
                problems.push(ScenarioProblem {
                    path,
                    message: format!("duplicate scenario id `{}` in {}", scenario.info.local_id, dir.display()),
                });
                continue;
            }
            seen.push(scenario.info.id.clone());
            match scenarios.iter_mut().find(|s| s.info.id == scenario.info.id) {
                Some(existing) => {
                    scenario.info.replaces = Some(existing.info.source.clone());
                    *existing = scenario;
                }
                None => {
                    // Keep scenarios grouped by folder.
                    let at = scenarios.iter().rposition(|s| s.info.folder == folder).map_or(scenarios.len(), |i| i + 1);
                    scenarios.insert(at, scenario);
                }
            }
        }
    }
}
