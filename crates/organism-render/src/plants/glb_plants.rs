//! Manifest GLB plants, rocks and fungi (MegaKit and Kenney models), with
//! seasonal tint on their foliage materials, leaf fall for deciduous
//! models, wind sway and a burnt (charred) state.
//!
//! Asset fix-up: the Kenney Nature Kit files store sRGB colour values in
//! glTF's linear `baseColorFactor` (e.g. bark `0.886, 0.514, 0.341` is the
//! sRGB orange-brown `#E28357`); read as linear they render pink and cyan.
//! Their material colours are reinterpreted as sRGB at load time.

use bevy::gltf::{GltfAssetLabel, GltfMaterialName};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};

use super::{PlantSeason, WindSway, linear_rgb};
use crate::library::ModelLibrary;
use crate::manifest::ModelCategory;

/// Errors when spawning a GLB plant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlantSpawnError {
    /// No model with that id in the manifest.
    #[error("no model `{0}` in the model manifest")]
    UnknownModel(String),
    /// The model is an animal.
    #[error("model `{0}` is an animal, not a plant, rock or fungus")]
    NotAPlant(String),
}

/// A plant drawn from a manifest GLB.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(Transform, Visibility)]
pub struct GlbPlant {
    /// Manifest model id.
    pub model: String,
    /// Burnt by fire.
    pub burnt: bool,
}

/// How a foliage material changes with the seasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TintKind {
    /// Deciduous leaves.
    Deciduous,
    /// Needles, palms, cacti.
    Evergreen,
    /// Grasses and herbs.
    Grass,
}

/// Scene root of a GLB plant.
#[derive(Component, Clone, Copy, Debug)]
pub struct GlbPlantScene {
    /// The [`GlbPlant`] entity.
    pub plant: Entity,
    /// Seasonal behaviour of its foliage.
    pub tint: TintKind,
    /// Whether material colours were authored as sRGB in the linear field
    /// (Kenney Nature Kit).
    pub srgb_colors: bool,
}

#[derive(Component)]
pub(super) struct GlbPlantReady;

/// Plant scenes that are spawned but not yet set up.
type PendingPlantScenes<'w, 's> =
    Query<'w, 's, (Entity, &'static GlbPlantScene), (With<GlbPlantReady>, Without<GlbPlantSetup>)>;

/// Mesh entities of a set-up plant scene with their original materials.
#[derive(Component, Clone, Debug)]
pub struct GlbPlantSetup {
    foliage: Vec<(Entity, Handle<StandardMaterial>)>,
    other: Vec<(Entity, Handle<StandardMaterial>)>,
    burnt: bool,
    /// Whether the foliage is currently hidden (`None` until first applied).
    foliage_hidden: Option<bool>,
}

/// Seasonal and charred material clones of GLB plant materials.
#[derive(Resource, Default)]
pub struct GlbPlantMaterials {
    tinted: HashMap<AssetId<StandardMaterial>, (Handle<StandardMaterial>, Color, TintKind)>,
    charred: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
    srgb_fixed: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
}

/// Reinterprets a colour whose linear components were authored as sRGB.
pub fn reinterpret_as_srgb(c: Color) -> Color {
    let l = c.to_linear();
    Color::srgba(l.red, l.green, l.blue, l.alpha)
}

fn tint_kind(id: &str, category: ModelCategory) -> TintKind {
    let lower = id.to_ascii_lowercase();
    if category == ModelCategory::Grass || category == ModelCategory::Herb || category == ModelCategory::Crop {
        TintKind::Grass
    } else if lower.contains("pine") || lower.contains("palm") || category == ModelCategory::Succulent {
        TintKind::Evergreen
    } else {
        TintKind::Deciduous
    }
}

/// Whether a glTF material name denotes foliage.
pub fn is_foliage_material(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["leaf", "leaves", "leafs", "grass", "needle", "fern", "clover"].iter().any(|k| n.contains(k))
}

/// Spawns a manifest plant (or rock/fungus/deadwood) at `transform`; the
/// manifest scale and yaw are applied to a child scene root.
pub fn spawn_glb_plant(
    commands: &mut Commands,
    library: &ModelLibrary,
    server: &AssetServer,
    model_id: &str,
    transform: Transform,
) -> Result<Entity, PlantSpawnError> {
    let model = library.model(model_id).ok_or_else(|| PlantSpawnError::UnknownModel(model_id.to_string()))?;
    if model.category.is_animal() {
        return Err(PlantSpawnError::NotAPlant(model_id.to_string()));
    }
    let plant = commands.spawn((GlbPlant { model: model.id.clone(), burnt: false }, transform)).id();
    let rest = Quat::from_rotation_y(model.yaw_offset_deg.to_radians());
    let phase = transform.translation.x * 0.9 + transform.translation.z * 1.3;
    let stiff = matches!(model.category, ModelCategory::Rock | ModelCategory::Deadwood | ModelCategory::Fungus);
    let mut scene = commands.spawn((
        WorldAssetRoot(server.load(GltfAssetLabel::Scene(0).from_asset(model.model_path.clone()))),
        Transform::from_rotation(rest).with_scale(Vec3::splat(model.scale)),
        GlbPlantScene {
            plant,
            tint: tint_kind(&model.id, model.category),
            srgb_colors: model.source.starts_with("kenney/"),
        },
        ChildOf(plant),
    ));
    if !stiff {
        let (frequency, amplitude) = if model.category == ModelCategory::Tree { (0.28, 0.015) } else { (1.0, 0.08) };
        scene.insert(WindSway { frequency, amplitude, phase, rest });
    }
    Ok(plant)
}

/// Observer: marks plant scenes whose instance is ready.
pub(super) fn on_plant_scene_ready(
    ready: On<WorldInstanceReady>,
    scenes: Query<(), With<GlbPlantScene>>,
    mut commands: Commands,
) {
    if scenes.contains(ready.entity) {
        commands.entity(ready.entity).insert(GlbPlantReady);
    }
}

/// Replaces foliage materials with shared seasonal clones.
pub(super) fn setup_glb_plants(
    mut commands: Commands,
    mut cache: ResMut<GlbPlantMaterials>,
    season: Res<PlantSeason>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    scenes: PendingPlantScenes,
    children: Query<&Children>,
    meshes: Query<(&MeshMaterial3d<StandardMaterial>, Option<&GltfMaterialName>)>,
) {
    for (root, scene) in &scenes {
        let mut foliage = Vec::new();
        let mut other = Vec::new();
        for d in children.iter_descendants(root) {
            let Ok((mat, name)) = meshes.get(d) else { continue };
            let mut original = mat.0.clone();
            if scene.srgb_colors {
                original = cache
                    .srgb_fixed
                    .entry(original.id())
                    .or_insert_with(|| {
                        let mut m = materials.get(&original).cloned().unwrap_or_default();
                        m.base_color = reinterpret_as_srgb(m.base_color);
                        materials.add(m)
                    })
                    .clone();
                commands.entity(d).insert(MeshMaterial3d(original.clone()));
            }
            if name.is_some_and(|n| is_foliage_material(n)) {
                let entry = cache.tinted.entry(original.id()).or_insert_with(|| {
                    let base = materials.get(&original).map_or(Color::WHITE, |m| m.base_color);
                    let clone =
                        materials.get(&original).cloned().map_or_else(|| original.clone(), |m| materials.add(m));
                    (clone, base, scene.tint)
                });
                if let Some(mut m) = materials.get_mut(&entry.0) {
                    m.base_color = seasonal_color(entry.1, entry.2, &season);
                }
                commands.entity(d).insert(MeshMaterial3d(entry.0.clone()));
                foliage.push((d, original));
            } else {
                other.push((d, original));
            }
        }
        commands.entity(root).insert(GlbPlantSetup { foliage, other, burnt: false, foliage_hidden: None });
    }
}

/// Original colour shifted by the ratio of the seasonal colour to the
/// summer colour, so textured leaves keep their detail.
fn seasonal_color(original: Color, kind: TintKind, season: &PlantSeason) -> Color {
    let summer = PlantSeason::new(0.35);
    let (now, reference) = match kind {
        TintKind::Deciduous => (season.deciduous_leaf(), summer.deciduous_leaf()),
        TintKind::Evergreen => (season.evergreen_leaf(), summer.evergreen_leaf()),
        TintKind::Grass => (season.grass(), summer.grass()),
    };
    let ratio = (linear_rgb(now) / linear_rgb(reference).max(Vec3::splat(1e-3))).clamp(Vec3::ZERO, Vec3::splat(3.0));
    let o = original.to_linear();
    Color::linear_rgba(o.red * ratio.x, o.green * ratio.y, o.blue * ratio.z, o.alpha)
}

/// Whether a GLB plant's foliage is hidden: burnt plants and bare
/// deciduous plants (leaf cover below 5 %) show no leaves. Authored models
/// have one merged canopy, so leaf fall is all-or-nothing for them.
fn foliage_hidden(burnt: bool, tint: TintKind, leaf_cover: f32) -> bool {
    burnt || (tint == TintKind::Deciduous && leaf_cover < 0.05)
}

/// Applies season changes (tint, leaf fall) and burnt-state changes.
pub(super) fn apply_glb_plant_season(
    mut commands: Commands,
    season: Res<PlantSeason>,
    mut cache: ResMut<GlbPlantMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    plants: Query<&GlbPlant>,
    mut scenes: Query<(&GlbPlantScene, &mut GlbPlantSetup)>,
    mut visibility: Query<&mut Visibility>,
) {
    if season.is_changed() {
        let GlbPlantMaterials { tinted, .. } = &mut *cache;
        for (handle, original, kind) in tinted.values() {
            if let Some(mut m) = materials.get_mut(handle) {
                m.base_color = seasonal_color(*original, *kind, &season);
            }
        }
    }
    let cover = season.leaf_cover();
    for (scene, mut setup) in &mut scenes {
        let Ok(plant) = plants.get(scene.plant) else { continue };
        let burnt_changed = plant.burnt != setup.burnt;
        if burnt_changed {
            let entries: Vec<(Entity, Handle<StandardMaterial>)> =
                setup.foliage.iter().chain(&setup.other).cloned().collect();
            for (e, original) in entries {
                let handle = if plant.burnt {
                    cache
                        .charred
                        .entry(original.id())
                        .or_insert_with(|| {
                            let mut m = materials.get(&original).cloned().unwrap_or_default();
                            m.base_color = Color::srgb(0.1, 0.09, 0.08);
                            m.base_color_texture = None;
                            materials.add(m)
                        })
                        .clone()
                } else if setup.foliage.iter().any(|(f, _)| *f == e) {
                    cache.tinted.get(&original.id()).map_or(original.clone(), |t| t.0.clone())
                } else {
                    original.clone()
                };
                commands.entity(e).insert(MeshMaterial3d(handle));
            }
            setup.burnt = plant.burnt;
        }
        // Applied on the first frame after setup too, so a deciduous model
        // planted in winter appears bare.
        let hide = foliage_hidden(plant.burnt, scene.tint, cover);
        if setup.foliage_hidden != Some(hide) {
            for (e, _) in &setup.foliage {
                if let Ok(mut v) = visibility.get_mut(*e) {
                    let want = if hide { Visibility::Hidden } else { Visibility::Inherited };
                    if *v != want {
                        *v = want;
                    }
                }
            }
            setup.foliage_hidden = Some(hide);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foliage_names_and_tint_kinds() {
        assert!(is_foliage_material("Leaves_NormalTree"));
        assert!(is_foliage_material("leafsGreen"));
        assert!(is_foliage_material("Grass"));
        assert!(!is_foliage_material("Bark_NormalTree"));
        assert!(!is_foliage_material("woodBark"));
        assert_eq!(tint_kind("tree_pine_a", ModelCategory::Tree), TintKind::Evergreen);
        assert_eq!(tint_kind("tree_broadleaf_a", ModelCategory::Tree), TintKind::Deciduous);
        assert_eq!(tint_kind("grass_tall", ModelCategory::Grass), TintKind::Grass);
    }

    #[test]
    fn foliage_hidden_when_burnt_or_bare() {
        assert!(foliage_hidden(true, TintKind::Evergreen, 1.0));
        assert!(foliage_hidden(false, TintKind::Deciduous, PlantSeason::new(0.85).leaf_cover()));
        assert!(!foliage_hidden(false, TintKind::Evergreen, 0.0));
        assert!(!foliage_hidden(false, TintKind::Deciduous, PlantSeason::new(0.35).leaf_cover()));
    }

    #[test]
    fn srgb_reinterpretation_darkens_mid_tones() {
        let c = reinterpret_as_srgb(Color::linear_rgb(0.886, 0.514, 0.341)).to_srgba();
        assert!((c.red - 0.886).abs() < 1e-3 && (c.green - 0.514).abs() < 1e-3);
    }

    #[test]
    fn summer_tint_is_identity_and_autumn_reddens() {
        let green = Color::srgb(0.2, 0.6, 0.2);
        let same = seasonal_color(green, TintKind::Deciduous, &PlantSeason::new(0.35)).to_linear();
        let g = green.to_linear();
        assert!((same.green - g.green).abs() < 1e-4 && (same.red - g.red).abs() < 1e-4);
        let autumn = seasonal_color(green, TintKind::Deciduous, &PlantSeason::new(0.66)).to_linear();
        assert!(autumn.red / autumn.green > g.red / g.green);
    }
}
