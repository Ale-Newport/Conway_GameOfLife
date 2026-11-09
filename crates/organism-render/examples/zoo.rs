//! Gallery of every organism and plant the renderer can draw.
//!
//! ```text
//! cargo run -p organism-render --example zoo
//! ```
//!
//! Environment variables:
//! * `EMERGENCE_ZOO_GROUP` — `all` (default), `glb`, `glb-far`, `mammals`,
//!   `birds`, `herps`, `bugs`, `fish`, `trees`, `plants`, `lod`.
//! * `EMERGENCE_ZOO_PAGE` — page of exhibits within the group (default:
//!   every exhibit of the group); `EMERGENCE_ZOO_PAGE_SIZE` sets the page
//!   size (default 8).
//! * `EMERGENCE_ZOO_ACTION` — show one action (`walk`, `fly`, `death`...)
//!   instead of cycling through all of them every 3 s.
//! * `EMERGENCE_ZOO_SEASON` — plant year fraction (0 spring … 0.66 autumn).
//! * `EMERGENCE_SMOKE_SCREENSHOT=<png>` — save a screenshot once every model
//!   is loaded and animating, then exit.
//!
//! Exhibits are scaled to a common display size so a bee and an elephant
//! can be compared side by side (the age scale doubles as display scale).

use std::path::PathBuf;

use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use organism_render::glb::{GlbInstance, GlbModelAssets};
use organism_render::plants::glb_plants::{GlbPlantScene, GlbPlantSetup};
use organism_render::plants::ground::{FlowerColor, scatter};
use organism_render::plants::{
    GroundCover, PlantSeason, ProceduralTree, TreeStyle, spawn_glb_plant, spawn_ground_cover,
};
use organism_render::procedural::species::BodyPlan;
use organism_render::{AnimatedOrganism, LodLevel, OrganismAction, OrganismCatalog, OrganismLod, OrganismRenderPlugin};
use rendering::WorldCamera;

/// Display size (m) of every exhibit.
const DISPLAY: f32 = 1.6;
/// Spacing between exhibits (m).
const CELL: f32 = 2.4;
/// Default exhibits per page.
const PAGE: usize = 8;

fn assets_root() -> PathBuf {
    match std::env::var_os("BEVY_ASSET_ROOT") {
        Some(root) => PathBuf::from(root).join("assets"),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"),
    }
}

#[derive(Resource, Clone, Debug)]
struct ZooConfig {
    group: String,
    page: Option<usize>,
    page_size: usize,
    action: Option<OrganismAction>,
    screenshot: Option<PathBuf>,
}

#[derive(Component)]
struct Exhibit {
    actions: Vec<OrganismAction>,
    index: usize,
    base_height: f32,
}

#[derive(Resource, Default)]
struct SmokeState {
    ready_frames: u32,
    requested: bool,
    wait_frames: u32,
}

fn main() {
    let root = assets_root();
    let root = root.canonicalize().unwrap_or(root);
    let action = std::env::var("EMERGENCE_ZOO_ACTION").ok().and_then(|a| match a.parse::<OrganismAction>() {
        Ok(a) => Some(a),
        Err(e) => {
            eprintln!("zoo: {e}; cycling actions instead");
            None
        }
    });
    let config = ZooConfig {
        group: std::env::var("EMERGENCE_ZOO_GROUP").unwrap_or_else(|_| "all".into()),
        page: std::env::var("EMERGENCE_ZOO_PAGE").ok().and_then(|p| p.parse().ok()),
        page_size: std::env::var("EMERGENCE_ZOO_PAGE_SIZE")
            .ok()
            .and_then(|p| p.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(PAGE),
        action,
        screenshot: std::env::var_os("EMERGENCE_SMOKE_SCREENSHOT").map(PathBuf::from),
    };
    let season = std::env::var("EMERGENCE_ZOO_SEASON")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .map_or_else(PlantSeason::default, PlantSeason::new);
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Emergence Lab — organism zoo".into(),
                        resolution: (1600, 900).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin { file_path: root.to_string_lossy().into_owned(), ..default() }),
        )
        .insert_resource(ClearColor(Color::srgb(0.62, 0.76, 0.9)))
        .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 450.0, ..default() })
        .insert_resource(config)
        .insert_resource(season)
        .init_resource::<SmokeState>()
        .add_plugins(OrganismRenderPlugin { assets_root: root })
        .add_systems(Startup, setup)
        .add_systems(Update, (cycle_actions, smoke_screenshot))
        .run();
}

/// Exhibit list for a group: (species, pinned LOD).
fn exhibits(catalog: &OrganismCatalog, group: &str) -> Vec<(String, Option<LodLevel>)> {
    let glb: Vec<String> = catalog
        .library
        .models()
        .iter()
        .filter(|m| m.category.is_animal() && m.is_animated())
        .filter_map(|m| m.species.first().cloned())
        .collect();
    let procedural_only = |pred: fn(&BodyPlan) -> bool| -> Vec<(String, Option<LodLevel>)> {
        catalog
            .species
            .all()
            .iter()
            .filter(|s| {
                catalog.library.animal_model(&s.id).is_none()
                    && s.aliases.iter().all(|a| catalog.library.animal_model(a).is_none())
                    && pred(&s.plan)
            })
            .map(|s| (s.id.clone(), None))
            .collect()
    };
    match group {
        "glb" => glb.into_iter().map(|s| (s, Some(LodLevel::Near))).collect(),
        "glb-far" => glb.into_iter().map(|s| (s, Some(LodLevel::Far))).collect(),
        "mammals" => procedural_only(|p| matches!(p, BodyPlan::Quadruped(_))),
        "birds" => procedural_only(|p| matches!(p, BodyPlan::Bird(_))),
        "herps" => procedural_only(|p| {
            matches!(p, BodyPlan::Serpent(_) | BodyPlan::Frog(_) | BodyPlan::Turtle(_) | BodyPlan::Snail(_))
        }),
        "bugs" => procedural_only(|p| matches!(p, BodyPlan::Arthropod(..))),
        "fish" => procedural_only(|p| matches!(p, BodyPlan::Fish(_))),
        "lod" => [LodLevel::Near, LodLevel::Mid, LodLevel::Far, LodLevel::Marker]
            .into_iter()
            .flat_map(|l| [("fox".to_string(), Some(l)), ("elephant".to_string(), Some(l))])
            .collect(),
        _ => Vec::new(),
    }
}

fn action_cycle(single: Option<OrganismAction>) -> Vec<OrganismAction> {
    match single {
        Some(a) => vec![a],
        None => vec![
            OrganismAction::Idle,
            OrganismAction::Walk,
            OrganismAction::Run,
            OrganismAction::Eat,
            OrganismAction::Attack,
            OrganismAction::Fly,
            OrganismAction::Swim,
            OrganismAction::Jump,
            OrganismAction::Rest,
            OrganismAction::Death,
        ],
    }
}

fn setup(
    mut commands: Commands,
    catalog: Res<OrganismCatalog>,
    config: Res<ZooConfig>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let groups: Vec<&str> = if config.group == "all" {
        vec!["glb", "glb-far", "mammals", "birds", "herps", "bugs", "fish", "trees", "plants", "lod"]
    } else {
        vec![config.group.as_str()]
    };
    let mut placed: Vec<Vec3> = Vec::new();
    for (row, group) in groups.iter().enumerate() {
        let z = -(row as f32) * 3.0 * CELL;
        match *group {
            "trees" => {
                let stages = [0.15, 0.4, 0.7, 1.0];
                let mut col = 0;
                for style in TreeStyle::ALL {
                    for g in stages {
                        let x = col as f32 * 6.0;
                        commands.spawn((
                            ProceduralTree { style, growth: g, seed: 3 + col as u64, burnt: false },
                            Transform::from_xyz(x, 0.0, z),
                        ));
                        placed.push(Vec3::new(x, 0.0, z));
                        col += 1;
                    }
                }
                let x = col as f32 * 6.0;
                commands.spawn((
                    ProceduralTree { style: TreeStyle::Broadleaf, growth: 0.8, seed: 5, burnt: true },
                    Transform::from_xyz(x, 0.0, z),
                ));
                placed.push(Vec3::new(x, 0.0, z));
            }
            "plants" => {
                let ids = [
                    "tree_broadleaf_a",
                    "tree_pine_b",
                    "tree_twisted_c",
                    "tree_dead_a",
                    "kenney_palm_tall",
                    "bush_flowering",
                    "fern",
                    "grass_tall",
                    "flower_group_a",
                    "mushroom_cluster",
                    "kenney_cactus_tall",
                    "rock_medium_a",
                    "kenney_stump_old",
                    "kenney_crop_corn_d",
                ];
                for (i, id) in ids.iter().enumerate() {
                    let p = Vec3::new(i as f32 * 4.0, 0.0, z);
                    match spawn_glb_plant(&mut commands, &catalog.library, &server, id, Transform::from_translation(p))
                    {
                        Ok(_) => placed.push(p),
                        Err(e) => eprintln!("zoo: {e}"),
                    }
                }
                let patch = commands.spawn((Transform::from_xyz(-6.0, 0.0, z), Visibility::default())).id();
                let kinds = [
                    GroundCover::Grass,
                    GroundCover::TallGrass,
                    GroundCover::Flower(FlowerColor::Red),
                    GroundCover::Flower(FlowerColor::Yellow),
                    GroundCover::Flower(FlowerColor::Blue),
                    GroundCover::Shrub,
                ];
                for (k, kind) in kinds.iter().enumerate() {
                    let count = match kind {
                        GroundCover::Grass => 160,
                        GroundCover::Shrub => 4,
                        _ => 18,
                    };
                    let pts: Vec<Vec3> = scatter(count, 2.6, 0.12, 40 + k as u64)
                        .into_iter()
                        .map(|p| Vec3::new(p.x, 0.0, p.y))
                        .collect();
                    spawn_ground_cover(&mut commands, patch, *kind, &pts, k as u64);
                }
                placed.push(Vec3::new(-6.0, 0.0, z));
            }
            g => {
                let mut list = exhibits(&catalog, g);
                if let Some(page) = config.page {
                    list = list.into_iter().skip(page * config.page_size).take(config.page_size).collect();
                }
                for (i, (species, pinned)) in list.into_iter().enumerate() {
                    let extent = catalog.extent(&species);
                    let display = (DISPLAY / extent).max(0.05);
                    let aquatic = catalog.procedural(&species).is_some_and(|s| matches!(s.plan, BodyPlan::Fish(_)));
                    let p = Vec3::new(i as f32 * CELL, if aquatic { 0.45 * DISPLAY } else { 0.0 }, z);
                    let mut organism = AnimatedOrganism::new(species.clone());
                    organism.age_scale = display;
                    let actions = action_cycle(config.action);
                    organism.action = actions[0];
                    commands.spawn((
                        organism,
                        OrganismLod { pinned: pinned.or(Some(LodLevel::Near)), ..default() },
                        Exhibit { actions, index: 0, base_height: p.y },
                        // Three-quarter view: face the camera's left.
                        Transform::from_translation(p).with_rotation(Quat::from_rotation_y(0.9)),
                    ));
                    placed.push(p);
                }
            }
        }
    }
    // Ground.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(600.0, 600.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.5, 0.3),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.001, 0.0),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 9000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.6, -0.9, 0.0)),
        CascadeShadowConfigBuilder {
            num_cascades: 3,
            first_cascade_far_bound: 25.0,
            maximum_distance: 250.0,
            ..default()
        }
        .build(),
    ));
    // Frame every exhibit.
    let (lo, hi) =
        placed.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let (lo, hi) = if placed.is_empty() { (Vec3::ZERO, Vec3::ZERO) } else { (lo, hi) };
    let tall = matches!(config.group.as_str(), "trees" | "plants");
    let center = (lo + hi) * 0.5 + Vec3::Y * if tall { 4.0 } else { 0.7 };
    let span = (hi.x - lo.x).max(hi.z - lo.z).max(if tall { 20.0 } else { 3.0 }) + CELL;
    let eye = center + Vec3::new(0.0, 0.2 * span + 0.6, 0.66 * span + 0.8);
    commands.spawn((
        Camera3d::default(),
        WorldCamera,
        Msaa::Sample4,
        Transform::from_translation(eye).looking_at(center, Vec3::Y),
    ));
}

/// Changes every exhibit's action every 3 s; flying exhibits are lifted
/// into the air (in a simulation, altitude comes from the model).
fn cycle_actions(
    time: Res<Time>,
    mut timer: Local<f32>,
    mut exhibits: Query<(&mut AnimatedOrganism, &mut Exhibit, &mut Transform)>,
) {
    *timer += time.delta_secs();
    let advance = *timer >= 3.0;
    if advance {
        *timer = 0.0;
    }
    for (mut organism, mut exhibit, mut transform) in &mut exhibits {
        if advance && exhibit.actions.len() > 1 {
            exhibit.index = (exhibit.index + 1) % exhibit.actions.len();
            organism.action = exhibit.actions[exhibit.index];
        }
        let airborne = matches!(organism.action, OrganismAction::Fly | OrganismAction::Glide);
        let target = if airborne { exhibit.base_height + 0.9 * DISPLAY } else { exhibit.base_height };
        transform.translation.y += (target - transform.translation.y) * (4.0 * time.delta_secs()).min(1.0);
    }
}

/// Waits until every GLB model is loaded and animating, lets animations
/// run for a second, captures the window and exits.
fn smoke_screenshot(
    mut commands: Commands,
    config: Res<ZooConfig>,
    mut state: ResMut<SmokeState>,
    glb: Res<GlbModelAssets>,
    instances: Query<&GlbInstance>,
    plants: Query<Option<&GlbPlantSetup>, With<GlbPlantScene>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(path) = &config.screenshot else { return };
    if state.requested {
        state.wait_frames += 1;
        if path.exists() || state.wait_frames > 600 {
            if !path.exists() {
                eprintln!("zoo: screenshot was not written to {}", path.display());
            }
            exit.write(AppExit::Success);
        }
        return;
    }
    let animating = instances.iter().all(|i| i.player.is_some() || i.broken);
    let plants_ready = plants.iter().all(|setup| setup.is_some());
    if glb.pending() == 0 && animating && plants_ready {
        state.ready_frames += 1;
    }
    if state.ready_frames > 90 {
        for (id, e) in glb.failures() {
            eprintln!("zoo: model `{id}` failed: {e}");
        }
        let _ = std::fs::remove_file(path);
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
        state.requested = true;
    }
}
