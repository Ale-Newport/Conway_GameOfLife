//! Typed reader for `assets/models/manifest.toml` (schema version 1).
//!
//! The manifest lists every installed glTF model with its provenance, the
//! scale and yaw that bring it to real-world size and the glTF facing
//! convention (+Z forward), and the mapping from our action vocabulary to the
//! exact clip names stored in the GLB. See `docs/research/assets-3d.md` §2.1.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Deserialize;

use crate::action::MANIFEST_ACTION_KEYS;

/// Manifest schema version this reader understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Errors produced while reading or validating the manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The file could not be read.
    #[error("cannot read model manifest `{path}`: {source}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// The text is not valid TOML or does not match the schema.
    #[error("model manifest is not valid: {0}")]
    Parse(#[from] toml::de::Error),
    /// The manifest declares a schema version this code does not support.
    #[error("unsupported model manifest schema_version {found} (this build reads version {supported})")]
    UnsupportedSchema {
        /// Version found in the file.
        found: u32,
        /// Version supported by this build.
        supported: u32,
    },
    /// Two models share an id.
    #[error("duplicate model id `{0}` in model manifest")]
    DuplicateId(String),
    /// A model entry is inconsistent.
    #[error("model `{model}`: {problem}")]
    InvalidModel {
        /// Model id.
        model: String,
        /// Human-readable description of the problem.
        problem: String,
    },
}

/// Broad classification of a model (`category` field).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub enum ModelCategory {
    /// `animal.mammal`
    Mammal,
    /// `animal.bird`
    Bird,
    /// `animal.fish`
    Fish,
    /// `animal.insect`
    Insect,
    /// `animal.arachnid`
    Arachnid,
    /// `animal.reptile`
    Reptile,
    /// `animal.amphibian`
    Amphibian,
    /// `plant.tree`
    Tree,
    /// `plant.shrub`
    Shrub,
    /// `plant.herb`
    Herb,
    /// `plant.flower`
    Flower,
    /// `plant.grass`
    Grass,
    /// `plant.crop`
    Crop,
    /// `plant.succulent`
    Succulent,
    /// `plant.aquatic`
    AquaticPlant,
    /// `fungus`
    Fungus,
    /// `deadwood`
    Deadwood,
    /// `rock`
    Rock,
}

impl ModelCategory {
    /// Every category with its manifest spelling.
    pub const ALL: [(ModelCategory, &'static str); 18] = [
        (Self::Mammal, "animal.mammal"),
        (Self::Bird, "animal.bird"),
        (Self::Fish, "animal.fish"),
        (Self::Insect, "animal.insect"),
        (Self::Arachnid, "animal.arachnid"),
        (Self::Reptile, "animal.reptile"),
        (Self::Amphibian, "animal.amphibian"),
        (Self::Tree, "plant.tree"),
        (Self::Shrub, "plant.shrub"),
        (Self::Herb, "plant.herb"),
        (Self::Flower, "plant.flower"),
        (Self::Grass, "plant.grass"),
        (Self::Crop, "plant.crop"),
        (Self::Succulent, "plant.succulent"),
        (Self::AquaticPlant, "plant.aquatic"),
        (Self::Fungus, "fungus"),
        (Self::Deadwood, "deadwood"),
        (Self::Rock, "rock"),
    ];

    /// Manifest spelling.
    pub fn as_str(self) -> &'static str {
        Self::ALL.iter().find(|(c, _)| *c == self).map_or("", |(_, s)| s)
    }

    /// Whether the model is an animal (animated organism).
    pub fn is_animal(self) -> bool {
        matches!(
            self,
            Self::Mammal | Self::Bird | Self::Fish | Self::Insect | Self::Arachnid | Self::Reptile | Self::Amphibian
        )
    }
}

impl FromStr for ModelCategory {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .find(|(_, name)| *name == s)
            .map(|(c, _)| *c)
            .ok_or_else(|| format!("unknown model category `{s}`"))
    }
}

impl TryFrom<String> for ModelCategory {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// Provenance of one asset pack.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSource {
    /// Stable key referenced by models.
    pub key: String,
    /// SPDX licence identifier.
    pub license: String,
    /// Author.
    pub author: String,
    /// Official page.
    pub page: String,
    /// Where the files were obtained.
    pub obtained_from: String,
    /// Retrieval date.
    pub retrieved: String,
}

/// One installed model.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    /// Stable model id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Classification.
    pub category: ModelCategory,
    /// Ecosystem species ids this model can represent.
    pub species: Vec<String>,
    /// Source pack key.
    pub source: String,
    /// SPDX licence.
    pub license: String,
    /// Author.
    pub author: String,
    /// Path relative to the assets root.
    pub model_path: String,
    /// Uniform scale bringing the model to real-world size in metres.
    pub scale: f32,
    /// Axis-aligned size (x, y, z) in metres after `scale`.
    pub real_size_m: [f32; 3],
    /// Model-space facing axis before `yaw_offset_deg`.
    pub forward: String,
    /// Rotation about +Y (degrees) that makes the model face +Z.
    pub yaw_offset_deg: f32,
    /// Triangle count.
    pub triangles: u32,
    /// File size in bytes.
    pub file_bytes: u64,
    /// Actions with an authored clip.
    #[serde(default)]
    pub supported_actions: Vec<String>,
    /// Actions without an authored clip (procedural fallback).
    #[serde(default)]
    pub missing_actions: Vec<String>,
    /// Far-LOD stand-in model paths.
    #[serde(default)]
    pub lod_variants: Vec<String>,
    /// Action key → exact clip name in the GLB.
    #[serde(default)]
    pub animation_set: BTreeMap<String, String>,
    /// Additional useful clips not in the action vocabulary.
    #[serde(default)]
    pub extra_clips: BTreeMap<String, String>,
    /// Playback-rate multipliers per action key.
    #[serde(default)]
    pub anim_speed: BTreeMap<String, f32>,
    /// Free-text notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// Record of processing applied to the file.
    #[serde(default)]
    pub modifications: Option<String>,
    /// Origin URL (MegaKit items).
    #[serde(default)]
    pub origin_url: Option<String>,
}

impl ModelEntry {
    /// Clip name for a manifest action key, if the model has one.
    pub fn clip_for(&self, key: &str) -> Option<&str> {
        self.animation_set.get(key).map(String::as_str)
    }

    /// Playback multiplier for an action key (1.0 when not listed).
    pub fn speed_multiplier(&self, key: &str) -> f32 {
        self.anim_speed.get(key).copied().unwrap_or(1.0)
    }

    /// Whether the model is animated (has at least one clip).
    pub fn is_animated(&self) -> bool {
        !self.animation_set.is_empty()
    }

    /// Distinct clip names referenced by `animation_set`, sorted.
    pub fn clip_names(&self) -> Vec<&str> {
        let set: BTreeSet<&str> = self.animation_set.values().map(String::as_str).collect();
        set.into_iter().collect()
    }

    /// Largest real-world extent in metres.
    pub fn max_extent(&self) -> f32 {
        self.real_size_m.iter().copied().fold(0.0, f32::max)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        let bad = |problem: String| ManifestError::InvalidModel { model: self.id.clone(), problem };
        if self.id.trim().is_empty() {
            return Err(bad("empty id".into()));
        }
        if !(self.scale.is_finite() && self.scale > 0.0) {
            return Err(bad(format!("scale must be a positive number, found {}", self.scale)));
        }
        if self.real_size_m.iter().any(|v| !(v.is_finite() && *v >= 0.0)) {
            return Err(bad(format!("real_size_m must be non-negative, found {:?}", self.real_size_m)));
        }
        if !self.yaw_offset_deg.is_finite() {
            return Err(bad("yaw_offset_deg is not finite".into()));
        }
        if self.model_path.trim().is_empty() || self.model_path.contains("..") {
            return Err(bad(format!(
                "model_path `{}` must be a relative path inside the assets root",
                self.model_path
            )));
        }
        let known = |k: &str| MANIFEST_ACTION_KEYS.contains(&k);
        for key in self.animation_set.keys().chain(self.anim_speed.keys()) {
            if !known(key) {
                return Err(bad(format!("unknown action key `{key}`")));
            }
        }
        for (key, speed) in &self.anim_speed {
            if !(speed.is_finite() && *speed >= 0.0) {
                return Err(bad(format!("anim_speed.{key} must be a non-negative number")));
            }
        }
        for key in self.supported_actions.iter().chain(&self.missing_actions) {
            if !known(key) {
                return Err(bad(format!("unknown action `{key}` in supported/missing actions")));
            }
        }
        for key in &self.supported_actions {
            if self.missing_actions.contains(key) {
                return Err(bad(format!("action `{key}` is both supported and missing")));
            }
            if !self.animation_set.contains_key(key) {
                return Err(bad(format!("supported action `{key}` has no clip in animation_set")));
            }
        }
        for key in &self.missing_actions {
            if self.animation_set.contains_key(key) {
                return Err(bad(format!("missing action `{key}` nevertheless has a clip")));
            }
        }
        Ok(())
    }
}

/// The parsed manifest.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    /// Schema version.
    pub schema_version: u32,
    /// Length unit (always `"metre"`).
    pub units: String,
    /// Up axis (always `"+Y"`).
    pub up_axis: String,
    /// Asset packs.
    #[serde(default, rename = "source")]
    pub sources: Vec<ManifestSource>,
    /// Models.
    #[serde(default, rename = "model")]
    pub models: Vec<ModelEntry>,
}

impl ModelManifest {
    /// Parses and validates manifest text.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: ModelManifest = toml::from_str(text)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Reads and validates a manifest file.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let text =
            std::fs::read_to_string(path).map_err(|source| ManifestError::Io { path: path.to_path_buf(), source })?;
        Self::parse(&text)
    }

    /// Checks schema version, id uniqueness and per-model consistency.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchema {
                found: self.schema_version,
                supported: SUPPORTED_SCHEMA_VERSION,
            });
        }
        let mut ids = BTreeSet::new();
        for model in &self.models {
            if !ids.insert(model.id.as_str()) {
                return Err(ManifestError::DuplicateId(model.id.clone()));
            }
            model.validate()?;
        }
        let source_keys: BTreeSet<&str> = self.sources.iter().map(|s| s.key.as_str()).collect();
        for model in &self.models {
            if !source_keys.is_empty() && !source_keys.contains(model.source.as_str()) {
                return Err(ManifestError::InvalidModel {
                    model: model.id.clone(),
                    problem: format!("source `{}` is not declared in [[source]]", model.source),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
schema_version = 1
units = "metre"
up_axis = "+Y"

[[source]]
key = "pack"
license = "CC0-1.0"
author = "A"
page = "p"
obtained_from = "o"
retrieved = "2026-11-09"

[[model]]
id = "fox"
name = "Red fox"
category = "animal.mammal"
species = ["fox"]
source = "pack"
license = "CC0-1.0"
author = "A"
model_path = "models/fox.glb"
scale = 0.18
real_size_m = [0.2, 0.48, 1.0]
forward = "+Z"
yaw_offset_deg = 0
triangles = 10
file_bytes = 20
supported_actions = ["idle", "run"]
missing_actions = ["fly"]
lod_variants = []
[model.animation_set]
idle = "Idle"
run = "Gallop"
[model.anim_speed]
run = 1.5
"#;

    #[test]
    fn parses_sample() {
        let m = ModelManifest::parse(SAMPLE).unwrap();
        assert_eq!(m.models.len(), 1);
        let fox = &m.models[0];
        assert_eq!(fox.category, ModelCategory::Mammal);
        assert_eq!(fox.clip_for("run"), Some("Gallop"));
        assert_eq!(fox.speed_multiplier("run"), 1.5);
        assert_eq!(fox.speed_multiplier("idle"), 1.0);
        assert_eq!(fox.clip_names(), vec!["Gallop", "Idle"]);
    }

    #[test]
    fn rejects_inconsistent_entries() {
        let bad_scale = SAMPLE.replace("scale = 0.18", "scale = -1");
        assert!(matches!(ModelManifest::parse(&bad_scale), Err(ManifestError::InvalidModel { .. })));
        let bad_key = SAMPLE.replace("run = \"Gallop\"", "sprint = \"Gallop\"");
        let err = ModelManifest::parse(&bad_key).unwrap_err();
        assert!(err.to_string().contains("sprint"), "{err}");
        let bad_version = SAMPLE.replace("schema_version = 1", "schema_version = 7");
        assert!(matches!(ModelManifest::parse(&bad_version), Err(ManifestError::UnsupportedSchema { found: 7, .. })));
        let bad_category = SAMPLE.replace("animal.mammal", "animal.dragon");
        assert!(matches!(ModelManifest::parse(&bad_category), Err(ManifestError::Parse(_))));
        let escaping = SAMPLE.replace("models/fox.glb", "../secret.glb");
        assert!(ModelManifest::parse(&escaping).is_err());
        let missing_clip = SAMPLE
            .replace("supported_actions = [\"idle\", \"run\"]", "supported_actions = [\"idle\", \"run\", \"eat\"]");
        assert!(ModelManifest::parse(&missing_clip).is_err());
    }

    #[test]
    fn rejects_duplicate_ids() {
        let model = &SAMPLE[SAMPLE.find("[[model]]").unwrap_or(0)..];
        let doubled = format!("{SAMPLE}\n{model}");
        assert!(matches!(ModelManifest::parse(&doubled), Err(ManifestError::DuplicateId(id)) if id == "fox"));
    }

    #[test]
    fn missing_file_is_a_typed_error() {
        let err = ModelManifest::load(Path::new("/definitely/not/here/manifest.toml")).unwrap_err();
        assert!(matches!(err, ManifestError::Io { .. }));
    }
}
