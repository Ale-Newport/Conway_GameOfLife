//! Plants: the model's trees drawn with `organism-render`'s tree generator
//! (growth stage from the tree's height, the model's foliage fraction —
//! leaf fall, browsing, fire — burnt bark, seasonal leaf tint and wind
//! sway), in full detail near the camera and as a static low-detail mesh
//! farther away; grass, shrubs and flowers near the camera with a density
//! from the model's vegetation biomass; mushroom clusters where the fungi
//! fruit.
//!
//! Trees change only when a frame carries them (at most hourly in
//! simulated time); level-of-detail swaps and new meshes run within a
//! per-frame budget.

use std::collections::{BTreeMap, BTreeSet};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use ecosystem::species::PlantLayer;
use ecosystem::view::TreeView;
use organism_render::TreeStyle as RenderTreeStyle;
use organism_render::plants::tree::generate_tree_with_leaf_cover;
use organism_render::plants::{
    FlowerColor, GROWTH_STAGES, GroundCover, GroundCoverPlant, LEAF_COVER_LEVELS, PlantMaterials, WindSway,
    leaf_cover_level,
};
use organism_render::procedural::parts::Detail;
use physics_world::{Biome, FireState, WorldLayer};
use simulation_core::SimRng;

use super::EcoCamera;
use super::terrain::MapState;
use crate::MODE;
use crate::mapping::{tree_growth, tree_scale, tree_style};
use crate::palette::{HERB_COVER_HALF, SHRUB_COVER_HALF, SNOW_COVER_HALF, cover};
use crate::state::{EcoData, EcoUi};

/// Distance (m) within which trees are drawn in full detail.
pub const TREE_NEAR_DISTANCE: f32 = 230.0;
/// Level-of-detail swaps per frame.
pub const TREE_SWAPS_PER_FRAME: usize = 150;
/// New full-detail tree meshes generated per frame (a few milliseconds each).
pub const NEAR_MESHES_PER_FRAME: usize = 2;
/// New low-detail tree meshes generated per frame.
pub const FAR_MESHES_PER_FRAME: usize = 6;
/// Radius (m) around the camera focus with grass, shrubs and flowers.
pub const COVER_RADIUS: f32 = 60.0;
/// Camera distance (m) beyond which ground cover is not drawn.
pub const COVER_MAX_CAMERA_DISTANCE: f32 = 260.0;
/// Ground-cover cells rebuilt per frame.
pub const COVER_CELLS_PER_FRAME: usize = 24;
/// Grass clumps per 100 m² at full cover.
pub const GRASS_PER_100M2: f32 = 45.0;
/// Shrubs per 100 m² at full cover.
pub const SHRUBS_PER_100M2: f32 = 1.6;
/// Flowers per 100 m² at full cover.
pub const FLOWERS_PER_100M2: f32 = 10.0;
/// Mushroom biomass (kg/m²) at which a cell shows its first mushroom.
pub const MUSHROOM_THRESHOLD: f32 = 2e-4;

/// A tree of the model.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct EcoTree {
    /// Model id.
    pub id: u64,
    /// Tree species index.
    pub species: u16,
    /// Renderer style.
    pub style: RenderTreeStyle,
    /// Growth stage (0..1, quantised to the renderer's stages).
    pub growth: f32,
    /// Charred by fire.
    pub burnt: bool,
    /// Height (m).
    pub height: f32,
    /// Crown radius (m).
    pub crown: f32,
    /// Drawn leaf level (0 bare .. [`LEAF_COVER_LEVELS`] full) from the
    /// model's foliage fraction.
    pub leaf_level: u8,
}

/// Which representation a tree currently shows.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct TreeLod {
    /// Full detail.
    pub near: bool,
    /// The representation entity (child), if built.
    pub child: Option<Entity>,
    /// The child shows an older shape (growth, leaves, fire): it stays
    /// until its replacement is built.
    pub stale: bool,
}

/// Id → entity of drawn trees.
#[derive(Resource, Default)]
pub struct TreeIndex {
    /// Entities by tree id.
    pub map: BTreeMap<u64, Entity>,
    epoch: Option<u64>,
    seq: u64,
    shown: Option<bool>,
    /// Terrain version the trees stand on.
    terrain: Option<u64>,
}

/// Key of a tree mesh: detail, style, shape variant, stage, leaf level.
type TreeKey = (Detail, RenderTreeStyle, u64, u8, u8);

/// Wood and (optional) foliage meshes.
type TreeMeshPair = (Handle<Mesh>, Option<Handle<Mesh>>);

/// Generated tree meshes.
#[derive(Resource, Default)]
pub struct TreeMeshes {
    map: BTreeMap<TreeKey, TreeMeshPair>,
}

/// Stages and shape variants per detail level.
fn grid_of(detail: Detail) -> (u8, u64) {
    match detail {
        Detail::Full => (GROWTH_STAGES, 4),
        Detail::Low => (8, 4),
    }
}

impl TreeMeshes {
    /// The meshes of a tree; `None` when they are not cached yet and this
    /// frame's generation budget is spent.
    fn get(
        &mut self,
        detail: Detail,
        tree: &EcoTree,
        budget: &mut usize,
        meshes: &mut Assets<Mesh>,
    ) -> Option<TreeMeshPair> {
        let (stages, variants) = grid_of(detail);
        let stage = ((tree.growth.clamp(0.0, 1.0) * f32::from(stages)).round() as u8).clamp(1, stages);
        let key = (detail, tree.style, tree.id % variants, stage, tree.leaf_level);
        if !self.map.contains_key(&key) {
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
        }
        Some(
            self.map
                .entry(key)
                .or_insert_with(|| {
                    let cover = f32::from(key.4) / f32::from(LEAF_COVER_LEVELS);
                    let t = generate_tree_with_leaf_cover(
                        key.1,
                        f32::from(stage) / f32::from(stages),
                        key.2,
                        detail,
                        cover,
                    );
                    (meshes.add(t.wood.to_mesh()), (!t.foliage.is_empty()).then(|| meshes.add(t.foliage.to_mesh())))
                })
                .clone(),
        )
    }
}

/// Leaf level drawn for a tree: none when burnt, the model's foliage
/// fraction (leaf mass relative to a full crown) otherwise; evergreen
/// shapes ignore leaf cover, so they always use the full crown mesh.
pub fn leaf_level(style: RenderTreeStyle, foliage: f32, burnt: bool) -> u8 {
    if burnt || !style.has_foliage() {
        0
    } else if !style.deciduous() {
        LEAF_COVER_LEVELS
    } else {
        leaf_cover_level(foliage)
    }
}

fn tree_params(t: &TreeView, frame: &crate::session::Frame) -> EcoTree {
    let species = &frame.info.catalog.trees()[usize::from(t.species)];
    let style = tree_style(t);
    let growth = tree_growth(t.height_m, species);
    // Meshes exist per 1/16 growth stage; quantise so updates only rebuild
    // a tree when its drawn stage changes.
    let growth = (growth * f32::from(GROWTH_STAGES)).round().max(1.0) / f32::from(GROWTH_STAGES);
    EcoTree {
        id: t.id,
        species: t.species,
        style,
        growth,
        burnt: t.burnt,
        height: t.height_m,
        crown: t.crown_radius_m,
        leaf_level: leaf_level(style, t.foliage, t.burnt),
    }
}

/// Spawns, updates and removes tree roots from frames that carry trees.
pub fn sync_trees(
    mut commands: Commands,
    data: Res<EcoData>,
    map: Res<MapState>,
    ui: Res<EcoUi>,
    mut index: ResMut<TreeIndex>,
    mut trees: Query<(&mut EcoTree, &mut Transform, &mut TreeLod)>,
    mut visibility: Query<&mut Visibility, With<EcoTree>>,
) {
    let Some(geometry) = map.geometry.as_ref() else { return };
    let Some(frame) = data.trees_frame.as_deref() else { return };
    if map.epoch != Some(frame.epoch) {
        return;
    }
    if index.shown != Some(ui.layers.trees) {
        index.shown = Some(ui.layers.trees);
        let v = if ui.layers.trees { Visibility::Inherited } else { Visibility::Hidden };
        for mut vis in &mut visibility {
            vis.set_if_neq(v);
        }
    }
    if index.epoch != Some(frame.epoch) {
        for (_, e) in std::mem::take(&mut index.map) {
            commands.entity(e).despawn();
        }
        index.epoch = Some(frame.epoch);
        index.seq = 0;
    }
    // The terrain moved (earthquake, landslide, volcano): every tree is
    // put back on the ground, from the trees already received.
    let regrounded = index.terrain != map.terrain_version;
    if index.seq == data.trees_seq && !regrounded {
        return;
    }
    index.seq = data.trees_seq;
    index.terrain = map.terrain_version;
    let mut alive = BTreeSet::new();
    let vis = if ui.layers.trees { Visibility::Inherited } else { Visibility::Hidden };
    for t in &frame.view.trees {
        alive.insert(t.id);
        let params = tree_params(t, frame);
        let place = |params: &EcoTree| {
            let scale = tree_scale(params.style, params.growth, t.height_m);
            let yaw = SimRng::seed_from_u64(t.id).range_f32(0.0, std::f32::consts::TAU);
            Transform::from_translation(geometry.ground_point(t.position))
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::splat(scale))
        };
        match index.map.get(&t.id).copied() {
            Some(e) => {
                let Ok((mut tree, mut tf, mut lod)) = trees.get_mut(e) else { continue };
                // Most trees do not change between two refreshes.
                if *tree == params && !regrounded {
                    continue;
                }
                let rebuild = tree.style != params.style
                    || tree.growth != params.growth
                    || tree.burnt != params.burnt
                    || tree.leaf_level != params.leaf_level;
                let transform = place(&params);
                if *tree != params {
                    *tree = params;
                }
                // Trees grow a little every day: re-scale only when the
                // height changed noticeably (no daily transform churn).
                if tf.translation.distance(transform.translation) > 1e-3
                    || (tf.scale.x - transform.scale.x).abs() > 0.02 * transform.scale.x.max(1e-3)
                {
                    *tf = transform;
                }
                if rebuild {
                    lod.stale = true;
                }
            }
            None => {
                let e = commands
                    .spawn((Name::new("Tree"), params, place(&params), vis, TreeLod::default(), DespawnOnExit(MODE)))
                    .id();
                index.map.insert(t.id, e);
            }
        }
    }
    index.map.retain(|id, e| {
        let keep = alive.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
}

/// Assigns full detail (swaying in the wind) or the static low-detail mesh
/// by camera distance, within per-frame budgets, closest trees first.
#[allow(clippy::too_many_arguments)]
pub fn tree_lod(
    mut commands: Commands,
    camera: Option<Single<&GlobalTransform, With<EcoCamera>>>,
    materials: Option<Res<PlantMaterials>>,
    mut cache: ResMut<TreeMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut trees: Query<(Entity, &EcoTree, &GlobalTransform, &mut TreeLod)>,
) {
    let (Some(camera), Some(materials)) = (camera, materials) else { return };
    let eye = camera.translation();
    let mut budget = TREE_SWAPS_PER_FRAME;
    let mut near_budget = NEAR_MESHES_PER_FRAME;
    let mut far_budget = FAR_MESHES_PER_FRAME;
    let mut work: Vec<(f32, Entity)> = Vec::new();
    for (e, _, gt, lod) in &trees {
        let d = eye.distance(gt.translation());
        let near = if lod.near { d < TREE_NEAR_DISTANCE * 1.1 } else { d < TREE_NEAR_DISTANCE * 0.9 };
        if lod.child.is_none() || near != lod.near || lod.stale {
            work.push((d, e));
        }
    }
    // Closest first: what the viewer looks at is fixed first.
    work.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, e) in work {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let Ok((_, tree, _, mut lod)) = trees.get_mut(e) else { continue };
        let near = d < TREE_NEAR_DISTANCE;
        let detail = if near { Detail::Full } else { Detail::Low };
        // Keep what it shows until the meshes can be generated.
        let mesh_budget = if near { &mut near_budget } else { &mut far_budget };
        let Some((wood, foliage)) = cache.get(detail, tree, mesh_budget, &mut meshes) else { continue };
        if let Some(child) = lod.child.take() {
            commands.entity(child).despawn();
        }
        lod.near = near;
        lod.stale = false;
        let bark = if tree.burnt { materials.charred.clone() } else { materials.wood.clone() };
        let leaves = if tree.style.deciduous() { materials.deciduous.clone() } else { materials.evergreen.clone() };
        let root = commands.spawn((Mesh3d(wood), MeshMaterial3d(bark), Transform::IDENTITY, ChildOf(e))).id();
        if near {
            // Trunk, branches and crown sway together about the tree base.
            let phase = (tree.id % 1000) as f32 * 0.37;
            commands.entity(root).insert(WindSway { frequency: 0.25, amplitude: 0.015, phase, rest: Quat::IDENTITY });
        } else if tree.leaf_level > 0 {
            // Far away the crown's shadow is what shows; the trunk's would
            // only cost a shadow draw per tree.
            commands.entity(root).insert(NotShadowCaster);
        }
        if let Some(foliage) = foliage.filter(|_| tree.leaf_level > 0) {
            commands.spawn((Mesh3d(foliage), MeshMaterial3d(leaves), Transform::IDENTITY, ChildOf(root)));
        }
        lod.child = Some(root);
    }
}

/// A tree hidden because its crown stands between the camera and what it
/// looks at.
#[derive(Component, Debug, Clone, Copy)]
pub struct Occluder;

/// Camera distance (m) below which occluding crowns are hidden.
pub const OCCLUSION_DISTANCE: f32 = 220.0;

/// Whether a tree hides the target from the eye: some point of the sight
/// line (short of the target, which may stand at the trunk) lies inside the
/// tree, taken as its trunk (`max(0.4 m, 6 %)` of the height thick) below
/// a crown cylinder of the crown radius (plus half a metre) from a third of
/// the height to the top: conical and layered crowns reach far down.
pub fn tree_blocks(eye: Vec3, target: Vec3, base: Vec3, height: f32, crown: f32) -> bool {
    if eye.distance_squared(target) < 1e-6 {
        return false;
    }
    let trunk = (0.06 * height).max(0.4);
    let crown_base = base.y + height / 3.0;
    let crown = crown.max(1.0) + 0.5;
    const SAMPLES: usize = 32;
    (0..SAMPLES).any(|k| {
        let p = eye.lerp(target, 0.92 * k as f32 / (SAMPLES - 1) as f32);
        let r = Vec2::new(p.x - base.x, p.z - base.z).length();
        p.y >= base.y && p.y <= base.y + height && r < if p.y >= crown_base { crown } else { trunk }
    })
}

/// Whether the eye stands inside (or just under) a tree: within its crown
/// radius plus a margin horizontally and below its top. Such a tree would
/// fill the whole view; it is hidden instead of lifting the camera, so
/// low and following views work inside a wood.
pub fn eye_in_tree(eye: Vec3, base: Vec3, height: f32, crown: f32) -> bool {
    Vec2::new(eye.x - base.x, eye.z - base.z).length() < crown.max(1.0) + 1.5 && eye.y < base.y + height + 1.0
}

/// Hides the trees whose crowns block close views of the camera's focus
/// (a selected or followed animal in a wood) or that the eye stands in, and
/// shows them again after.
pub fn hide_occluding_trees(
    mut commands: Commands,
    ui: Res<EcoUi>,
    camera: Option<Single<(&GlobalTransform, &bevy_panorbit_camera::PanOrbitCamera), With<EcoCamera>>>,
    mut trees: Query<(Entity, &EcoTree, &GlobalTransform, &mut Visibility, Has<Occluder>)>,
) {
    let Some(camera) = camera else { return };
    let (eye, focus) = (camera.0.translation(), camera.1.focus);
    let close = ui.layers.trees && eye.distance(focus) < OCCLUSION_DISTANCE;
    let (lo, hi) = (eye.min(focus), eye.max(focus));
    for (e, tree, gt, mut vis, hidden) in &mut trees {
        let base = gt.translation();
        // Only trees near the eye–focus segment can hide anything.
        let reach = tree.crown.max(1.0) + 2.0;
        let near = base.x > lo.x - reach && base.x < hi.x + reach && base.z > lo.z - reach && base.z < hi.z + reach;
        let blocks = close
            && near
            && (tree_blocks(eye, focus, base, tree.height, tree.crown)
                || eye_in_tree(eye, base, tree.height, tree.crown));
        if blocks && !hidden {
            vis.set_if_neq(Visibility::Hidden);
            commands.entity(e).insert(Occluder);
        } else if !blocks && hidden {
            if ui.layers.trees {
                vis.set_if_neq(Visibility::Inherited);
            }
            commands.entity(e).remove::<Occluder>();
        }
    }
}

/// A ground-cover cell near the camera.
#[derive(Component, Debug, Clone, Copy)]
pub struct CoverCell;

/// Mushroom cluster mesh and material.
#[derive(Resource)]
pub struct MushroomAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

impl FromWorld for MushroomAssets {
    fn from_world(world: &mut World) -> Self {
        let mesh = mushroom_mesh();
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.7,
            ..default()
        });
        Self { mesh, material }
    }
}

/// A mushroom (cap on a stem, 12 cm tall) with vertex colours.
pub fn mushroom_mesh() -> Mesh {
    let colored = |mesh: Mesh, rgb: [f32; 3]| {
        let n = mesh.count_vertices();
        let c = Color::srgb(rgb[0], rgb[1], rgb[2]).to_linear().to_f32_array();
        mesh.with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![c; n])
    };
    let mut stem = colored(Cylinder::new(0.012, 0.08).mesh().resolution(10).build(), [0.88, 0.84, 0.74])
        .transformed_by(Transform::from_xyz(0.0, 0.04, 0.0));
    let cap = colored(Sphere::new(0.04).mesh().uv(14, 8), [0.62, 0.30, 0.16])
        .transformed_by(Transform::from_xyz(0.0, 0.085, 0.0).with_scale(Vec3::new(1.0, 0.5, 1.0)));
    // Both meshes carry the same attributes, so merging cannot fail.
    let _ = stem.merge(&cap);
    stem
}

/// What a ground-cover cell holds; it is rebuilt when this changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoverSignature {
    grass: u16,
    shrubs: u16,
    flowers: u16,
    mushrooms: u16,
    burnt: bool,
}

impl CoverSignature {
    /// Whether a cell must be rebuilt: burnt state or fungi changed, or a
    /// plant count moved by more than a quarter (biomass fluctuates hourly;
    /// small changes are not worth respawning hundreds of plants).
    pub fn differs(&self, new: &CoverSignature) -> bool {
        let far = |a: u16, b: u16, floor: u16| a.abs_diff(b) > floor.max(a / 4);
        self.burnt != new.burnt
            || self.mushrooms != new.mushrooms
            || far(self.grass, new.grass, 6)
            || far(self.shrubs, new.shrubs, 1)
            || far(self.flowers, new.flowers, 3)
    }
}

/// Cells near the camera with their ground cover.
#[derive(Resource, Default)]
pub struct CoverIndex {
    cells: BTreeMap<usize, (Entity, CoverSignature)>,
    epoch: Option<u64>,
    terrain: Option<u64>,
}

/// Counts of plants in a cell of `area_m2` from the model's biomass of
/// grass, shrubs, flowers and mushrooms (kg/m²).
#[allow(clippy::too_many_arguments)]
pub fn cover_signature(
    area_m2: f32,
    grass: f32,
    shrubs: f32,
    flowers: f32,
    mushrooms: f32,
    snow_m: f32,
    burnt: bool,
) -> CoverSignature {
    let snowed = 1.0 - cover(snow_m, SNOW_COVER_HALF);
    let per = area_m2 / 100.0;
    let n = |density: f32, c: f32| (density * per * c * snowed).round().clamp(0.0, 400.0) as u16;
    let mushrooms = if mushrooms > MUSHROOM_THRESHOLD {
        (mushrooms / MUSHROOM_THRESHOLD).log2().clamp(0.0, 4.0) as u16 + 1
    } else {
        0
    };
    CoverSignature {
        grass: n(GRASS_PER_100M2, cover(grass, HERB_COVER_HALF)),
        shrubs: n(SHRUBS_PER_100M2, cover(shrubs, SHRUB_COVER_HALF)),
        flowers: n(FLOWERS_PER_100M2, cover(flowers, HERB_COVER_HALF)),
        mushrooms: if snowed > 0.5 { mushrooms } else { 0 },
        burnt,
    }
}

/// Keeps grass, shrubs, flowers and mushrooms around the camera focus.
#[allow(clippy::too_many_arguments)]
pub fn sync_ground_cover(
    mut commands: Commands,
    data: Res<EcoData>,
    map: Res<MapState>,
    ui: Res<EcoUi>,
    mushrooms: Res<MushroomAssets>,
    mut index: ResMut<CoverIndex>,
    camera: Option<Single<&bevy_panorbit_camera::PanOrbitCamera, With<EcoCamera>>>,
) {
    let (Some(frame), Some(geometry), Some(camera)) = (data.get(), map.geometry.as_ref(), camera) else { return };
    // A new world or a moved terrain: the plants are placed again.
    if index.epoch != map.epoch || index.terrain != map.terrain_version {
        for (_, (e, _)) in std::mem::take(&mut index.cells) {
            commands.entity(e).despawn();
        }
        index.epoch = map.epoch;
        index.terrain = map.terrain_version;
    }
    let enabled = ui.layers.ground_cover && camera.target_radius < COVER_MAX_CAMERA_DISTANCE;
    let focus = camera.target_focus;
    let veg = |layer: PlantLayer| frame.view.vegetation.iter().find(|(l, _)| *l == layer).map(|(_, g)| g.as_slice());
    let (grass, flowers, shrubs) = (veg(PlantLayer::Grass), veg(PlantLayer::Flowers), veg(PlantLayer::Shrubs));
    let snow = frame.view.world.layer(WorldLayer::Snow).map(|g| g.as_slice());
    let fire = frame.view.world.fire.as_ref().map(|g| g.as_slice());
    let fungi = frame.view.mushrooms.as_ref().map(|g| g.as_slice());
    let at = |s: Option<&[f32]>, i: usize| s.and_then(|v| v.get(i)).copied().unwrap_or(0.0);
    let s = geometry.spacing;
    let r = COVER_RADIUS + s;
    // Cells wanted now.
    let mut wanted = BTreeSet::new();
    if enabled {
        let (i0, i1) = (((focus.x - r) / s).floor().max(0.0) as usize, ((focus.x + r) / s).ceil() as usize);
        let (j0, j1) = (((focus.z - r) / s).floor().max(0.0) as usize, ((focus.z + r) / s).ceil() as usize);
        for j in j0..j1.min(geometry.height) {
            for i in i0..i1.min(geometry.width) {
                let centre = Vec2::new((i as f32 + 0.5) * s, (j as f32 + 0.5) * s);
                if centre.distance(Vec2::new(focus.x, focus.z)) <= r {
                    wanted.insert(j * geometry.width + i);
                }
            }
        }
    }
    index.cells.retain(|cell, (e, _)| {
        let keep = wanted.contains(cell);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    let mut budget = COVER_CELLS_PER_FRAME;
    let tick = frame.view.tick;
    let day = u64::from(frame.info.ticks_per_day.max(1));
    for cell in wanted {
        let burnt = match fire.and_then(|f| f.get(cell)) {
            Some(FireState::Burned { since }) => tick.saturating_sub(*since) < 20 * day,
            Some(FireState::Burning { .. }) => true,
            _ => false,
        };
        let sig = cover_signature(
            s * s,
            at(grass, cell),
            at(shrubs, cell),
            at(flowers, cell),
            at(fungi, cell),
            at(snow, cell),
            burnt,
        );
        if index.cells.get(&cell).is_some_and(|(_, old)| !old.differs(&sig)) {
            continue;
        }
        if budget == 0 {
            break;
        }
        budget -= 1;
        if let Some((e, _)) = index.cells.remove(&cell) {
            commands.entity(e).despawn();
        }
        let parent = commands.spawn((CoverCell, Transform::IDENTITY, Visibility::Inherited, DespawnOnExit(MODE))).id();
        let (ci, cj) = (cell % geometry.width, cell / geometry.width);
        let mut rng = SimRng::seed_from_u64(cell as u64 * 7919 + 17);
        let points = |count: u16, rng: &mut SimRng| -> Vec<Vec3> {
            (0..count)
                .filter_map(|_| {
                    let x = (ci as f32 + rng.range_f32(0.02, 0.98)) * s;
                    let z = (cj as f32 + rng.range_f32(0.02, 0.98)) * s;
                    (geometry.water_depth(x, z) < 0.05).then(|| Vec3::new(x, geometry.ground(x, z), z))
                })
                .collect()
        };
        let biome = map.biome.get(cell).copied().unwrap_or(Biome::Grassland);
        let tall = matches!(biome, Biome::Savanna | Biome::Wetland);
        let kinds: [(GroundCover, u16); 3] = [
            (if tall { GroundCover::TallGrass } else { GroundCover::Grass }, sig.grass),
            (GroundCover::Shrub, sig.shrubs),
            (GroundCover::Flower(flower_color(cell)), sig.flowers),
        ];
        for (kind, count) in kinds {
            // Each drawn plant is a clump: grass tufts are drawn about twice
            // the renderer's single-tuft size.
            let (lo, hi) = match kind {
                GroundCover::Grass | GroundCover::TallGrass => (1.4, 2.4),
                GroundCover::Shrub => (0.8, 1.4),
                GroundCover::Flower(_) => (1.1, 1.7),
            };
            for p in points(count, &mut rng) {
                let yaw = rng.range_f32(0.0, std::f32::consts::TAU);
                let scale = rng.range_f32(lo, hi);
                commands.spawn((
                    GroundCoverPlant { kind, variant: rng.index(4) as u8, burnt: sig.burnt },
                    Transform::from_translation(p)
                        .with_rotation(Quat::from_rotation_y(yaw))
                        .with_scale(Vec3::splat(scale)),
                    ChildOf(parent),
                ));
            }
        }
        for p in points(sig.mushrooms * 3, &mut rng) {
            let scale = rng.range_f32(0.8, 1.6);
            commands.spawn((
                Mesh3d(mushrooms.mesh.clone()),
                MeshMaterial3d(mushrooms.material.clone()),
                Transform::from_translation(p).with_scale(Vec3::splat(scale)),
                ChildOf(parent),
            ));
        }
        index.cells.insert(cell, (parent, sig));
    }
}

fn flower_color(cell: usize) -> FlowerColor {
    match (cell.wrapping_mul(2_654_435_761) >> 7) & 3 {
        0 => FlowerColor::Red,
        1 => FlowerColor::Yellow,
        2 => FlowerColor::Blue,
        _ => FlowerColor::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_follows_biomass_snow_and_fungi() {
        let area = 400.0;
        let bare = cover_signature(area, 0.0, 0.0, 0.0, 0.0, 0.0, false);
        assert_eq!((bare.grass, bare.shrubs, bare.flowers, bare.mushrooms), (0, 0, 0, 0));
        let lush = cover_signature(area, 0.5, 1.0, 0.2, 0.01, 0.0, false);
        assert!(lush.grass >= 60 && lush.shrubs >= 4 && lush.flowers >= 10, "{lush:?}");
        assert!(lush.mushrooms >= 1);
        let snowed = cover_signature(area, 0.5, 1.0, 0.2, 0.01, 0.05, false);
        assert!(snowed.grass < lush.grass / 10, "snow buries the herbs");
        assert_eq!(snowed.mushrooms, 0);
        let sparse = cover_signature(area, 0.02, 0.0, 0.0, 0.0, 0.0, false);
        assert!(sparse.grass > 0 && sparse.grass < lush.grass);
    }

    #[test]
    fn small_changes_do_not_rebuild_cover() {
        let a = cover_signature(400.0, 0.3, 0.2, 0.05, 0.0, 0.0, false);
        let b = cover_signature(400.0, 0.31, 0.2, 0.05, 0.0, 0.0, false);
        assert!(!a.differs(&b), "{a:?} {b:?}");
        let grazed = cover_signature(400.0, 0.05, 0.2, 0.05, 0.0, 0.0, false);
        assert!(a.differs(&grazed));
        let burnt = CoverSignature { burnt: true, ..a };
        assert!(a.differs(&burnt));
    }

    #[test]
    fn trees_between_eye_and_target_occlude() {
        let (eye, target) = (Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, 0.0, 40.0));
        // A 12 m tree half way: the line passes 5 m up, through the crown.
        assert!(tree_blocks(eye, target, Vec3::new(0.5, 0.0, 20.0), 12.0, 3.0));
        assert!(!tree_blocks(eye, target, Vec3::new(10.0, 0.0, 20.0), 12.0, 3.0), "beside the line");
        assert!(!tree_blocks(eye, target, Vec3::new(0.0, 0.0, 50.0), 12.0, 3.0), "behind the target");
        assert!(!tree_blocks(eye, target, Vec3::new(0.0, 0.0, -5.0), 12.0, 3.0), "behind the eye");
        assert!(!tree_blocks(eye, eye, Vec3::ZERO, 12.0, 3.0));
        // The line passes over a low tree.
        assert!(!tree_blocks(eye, target, Vec3::new(0.0, 0.0, 10.0), 3.0, 2.0));
        // A trunk on a low sight line hides the animal behind it; a thin
        // crown base far above the line does not.
        let (low_eye, animal) = (Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 0.5, 30.0));
        assert!(tree_blocks(low_eye, animal, Vec3::new(0.2, 0.0, 15.0), 20.0, 4.0));
        assert!(
            !tree_blocks(low_eye, animal, Vec3::new(2.0, 0.0, 15.0), 20.0, 4.0),
            "beside the trunk, under the crown"
        );
        // An eye inside a crown, or under it, hides the tree; one above or beside it does not.
        let base = Vec3::new(0.0, 2.0, 0.0);
        assert!(eye_in_tree(Vec3::new(1.0, 8.0, 0.0), base, 15.0, 4.0));
        assert!(eye_in_tree(Vec3::new(3.0, 3.0, 2.0), base, 15.0, 4.0));
        assert!(!eye_in_tree(Vec3::new(1.0, 19.0, 0.0), base, 15.0, 4.0), "above the top");
        assert!(!eye_in_tree(Vec3::new(9.0, 8.0, 0.0), base, 15.0, 4.0), "beside the crown");
    }

    #[test]
    fn mushroom_mesh_merges_cap_and_stem() {
        let m = mushroom_mesh();
        assert!(m.count_vertices() > 100);
        assert!(m.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
    }

    #[test]
    fn leaf_levels_follow_the_model_foliage() {
        assert_eq!(leaf_level(RenderTreeStyle::Broadleaf, 0.0, false), 0, "bare in winter");
        assert_eq!(leaf_level(RenderTreeStyle::Broadleaf, 1.0, false), LEAF_COVER_LEVELS);
        assert_eq!(leaf_level(RenderTreeStyle::Broadleaf, 0.3, false), 2);
        assert_eq!(leaf_level(RenderTreeStyle::Conifer, 0.3, false), LEAF_COVER_LEVELS, "evergreen shape");
        assert_eq!(leaf_level(RenderTreeStyle::Conifer, 1.0, true), 0, "burnt crowns are gone");
        assert_eq!(leaf_level(RenderTreeStyle::Dead, 1.0, false), 0);
    }
}
