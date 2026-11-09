//! Checks the installed model assets against `assets/models/manifest.toml`:
//! every model file exists and is a valid GLB, every clip named in an
//! `animation_set` exists in its file, every external texture exists, and
//! every action of every species resolves to an authored clip or a
//! procedural fallback.

use std::path::{Path, PathBuf};

use organism_render::OrganismAction;
use organism_render::catalog::{NearSource, OrganismCatalog};
use organism_render::library::{ModelLibrary, PlanSource, plan_action};
use organism_render::manifest::ModelManifest;
use organism_render::procedural::parts::Detail;
use organism_render::procedural::pose::effective_action;

fn assets_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// JSON chunk of a binary glTF (GLB) file.
fn glb_json(path: &Path) -> Result<serde_json::Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err(format!("{}: not a GLB file", path.display()));
    }
    let word = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
    let (version, total, chunk_len, chunk_type) = (word(4), word(8), word(12), word(16));
    if version != 2 || total != bytes.len() {
        return Err(format!("{}: GLB version {version}, declared length {total} vs {}", path.display(), bytes.len()));
    }
    if chunk_type != 0x4E4F_534A || 20 + chunk_len > bytes.len() {
        return Err(format!("{}: first chunk is not JSON", path.display()));
    }
    serde_json::from_slice(&bytes[20..20 + chunk_len]).map_err(|e| format!("{}: {e}", path.display()))
}

fn manifest() -> ModelManifest {
    ModelManifest::load(&assets_root().join("models/manifest.toml")).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn every_model_file_and_clip_exists() {
    let root = assets_root();
    let m = manifest();
    assert!(m.models.len() >= 100, "{} models", m.models.len());
    let mut problems = Vec::new();
    for model in &m.models {
        let path = root.join(&model.model_path);
        let json = match glb_json(&path) {
            Ok(j) => j,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        let clips: Vec<&str> = json["animations"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c["name"].as_str()).collect())
            .unwrap_or_default();
        for (action, clip) in &model.animation_set {
            if !clips.contains(&clip.as_str()) {
                problems.push(format!("{}: {action} clip `{clip}` not in {:?}", model.id, clips));
            }
        }
        for (key, clip) in &model.extra_clips {
            if !clips.contains(&clip.as_str()) {
                problems.push(format!("{}: extra clip {key} `{clip}` missing", model.id));
            }
        }
        if model.category.is_animal() && json["skins"].as_array().is_none_or(|s| s.is_empty()) && model.is_animated() {
            problems.push(format!("{}: animated animal without a skin", model.id));
        }
        // External textures (MegaKit) must sit next to the GLB.
        for image in json["images"].as_array().into_iter().flatten() {
            if let Some(uri) = image["uri"].as_str() {
                let tex = path.parent().unwrap_or(&root).join(uri);
                if !tex.is_file() {
                    problems.push(format!("{}: texture {} missing", model.id, tex.display()));
                }
            }
        }
        for lod in &model.lod_variants {
            if !root.join(lod).is_file() {
                problems.push(format!("{}: LOD variant {lod} missing", model.id));
            }
        }
    }
    assert!(problems.is_empty(), "asset problems:\n{}", problems.join("\n"));
}

#[test]
fn every_action_of_every_glb_species_is_drawable() {
    let lib = ModelLibrary::from_manifest(manifest());
    let mut checked = 0;
    for species in lib.animal_species() {
        let model = lib.animal_model(species).unwrap_or_else(|| panic!("{species} has no animated model"));
        for action in OrganismAction::ALL {
            let plan = plan_action(model, action);
            assert!(plan.is_drawable(), "{species}/{action}: nothing to draw");
            if let Some(clip) = &plan.clip {
                assert!(
                    model.animation_set.values().any(|c| *c == clip.name),
                    "{species}/{action}: clip `{}` not in the manifest set",
                    clip.name
                );
            }
            // Authored only when the manifest lists the action as supported.
            if plan.source == PlanSource::Authored
                && let Some(key) = action.manifest_key()
            {
                assert!(
                    model.supported_actions.iter().any(|a| a == key),
                    "{species}/{action}: authored but not supported"
                );
            }
            if let Some(key) = action.manifest_key()
                && model.missing_actions.iter().any(|a| a == key)
            {
                assert_eq!(plan.source, PlanSource::Fallback, "{species}/{action}: listed missing but authored");
            }
            checked += 1;
        }
    }
    assert!(checked > 30 * OrganismAction::ALL.len(), "{checked}");
}

#[test]
fn every_species_has_a_procedural_far_lod_and_full_action_set() {
    let catalog = OrganismCatalog::load(&assets_root());
    assert!(catalog.manifest_error.is_none(), "{:?}", catalog.manifest_error);
    let lib_species: Vec<String> = catalog.library.animal_species().into_iter().map(String::from).collect();
    for species in &lib_species {
        assert!(matches!(catalog.near_source(species), Some(NearSource::Glb(_))), "{species}");
        let spec =
            catalog.procedural(species).unwrap_or_else(|| panic!("{species} has no procedural far-LOD stand-in"));
        let rig = spec.build(Detail::Low);
        assert!(rig.triangle_count() > 0, "{species}");
    }
    // Procedural species: every action maps to something the body plan can show.
    for spec in catalog.species.all() {
        let rig = spec.build(Detail::Low);
        for action in OrganismAction::ALL {
            let shown = effective_action(&rig.motion, action);
            assert_ne!(shown, OrganismAction::Flee, "{}: flee is drawn as run", spec.id);
            if !rig.motion.can_fly {
                assert!(!matches!(shown, OrganismAction::Fly | OrganismAction::Glide), "{} cannot fly", spec.id);
            }
        }
    }
}
