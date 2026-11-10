//! The 3D world: camera and lights, terrain, water, roads, territory,
//! vegetation and buildings, kept in sync with the published frames.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::color::ColorToPacked;
use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;
use bevy::render::view::Msaa;
use bevy_panorbit_camera::PanOrbitCamera;
use civilization::Terrain;
use civilization::buildings::{BuildingId, BuildingKind};
use organism_render::plants::{ProceduralTree, TreeStyle};
use rendering::water::WaterSettings;
use rendering::{FollowTarget, OrbitCameraRig, WaterMaterial};

use crate::MODE;
use crate::coords::{HeightField, tile_hash, to_world};
use crate::meshes::{BERRY_LEVELS, CROP_STAGES, berry_level, crop_stage};
use crate::people::PeopleIndex;
use crate::session::Selection;
use crate::state::{CurrentFrame, ViewState};
use crate::world_mesh::{road_mesh, scatter_on_tile, terrain_mesh, territory_mesh, water_mesh};

/// Marker of the mode's world camera.
#[derive(Component, Debug, Clone, Copy)]
pub struct CivCamera;

/// Marker of the sun.
#[derive(Component, Debug, Clone, Copy)]
pub struct Sun;

/// Which map layer an entity draws.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapLayer {
    /// Terrain surface and skirt.
    Terrain,
    /// Water surface.
    Water,
    /// Trails and roads.
    Roads,
    /// Territory overlay.
    Territory,
}

/// Shared materials.
#[derive(Resource, Clone)]
pub struct CivMaterials {
    /// White, vertex-coloured, matte (terrain, buildings, rocks, people).
    pub white: Handle<StandardMaterial>,
    /// Vertex-coloured, alpha-blended (roads).
    pub road: Handle<StandardMaterial>,
    /// Unlit, vertex-coloured, alpha-blended (territory).
    pub overlay: Handle<StandardMaterial>,
    /// Animated water.
    pub water: Handle<WaterMaterial>,
    groups: HashMap<[u8; 3], Handle<StandardMaterial>>,
}

impl CivMaterials {
    /// Creates the shared materials.
    pub fn new(materials: &mut Assets<StandardMaterial>, water: &mut Assets<WaterMaterial>) -> Self {
        Self {
            white: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.92,
                reflectance: 0.25,
                ..default()
            }),
            road: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 1.0,
                reflectance: 0.1,
                alpha_mode: AlphaMode::Blend,
                depth_bias: 4.0,
                ..default()
            }),
            overlay: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                depth_bias: 2.0,
                ..default()
            }),
            water: water.add(WaterMaterial {
                settings: WaterSettings {
                    shallow_color: Color::srgb(0.16, 0.52, 0.56).to_linear().to_vec4(),
                    deep_color: Color::srgb(0.04, 0.2, 0.34).to_linear().to_vec4(),
                    waves: Vec4::new(0.06, 3.0, 0.8, 0.35),
                    // Sparkling glints rather than white bands where the sun
                    // reflects towards a top-down camera.
                    sun_color: Vec4::new(1.0, 0.95, 0.85, 0.45),
                    // Softer sky reflection: a strong one shows as pale
                    // blotches on the lakes in the aerial view.
                    shading: Vec4::new(0.55, 0.94, 420.0, 0.6),
                    ..default()
                }
                .with_sun_from(&sun_transform()),
            }),
            groups: HashMap::new(),
        }
    }

    /// Material of a cloth / pennant colour (cached per 8-bit sRGB colour).
    pub fn group(&mut self, color: Color, materials: &mut Assets<StandardMaterial>) -> Handle<StandardMaterial> {
        let key = color.to_srgba().to_u8_array_no_alpha();
        self.groups
            .entry(key)
            .or_insert_with(|| {
                materials.add(StandardMaterial { base_color: color, perceptual_roughness: 0.85, ..default() })
            })
            .clone()
    }
}

/// Owners, contested flags and group colours the territory mesh was built from.
type TerritoryKey = (Vec<u32>, Vec<bool>, Vec<(u32, [u8; 3])>);

/// Parts of building visuals updated in place.
type BuildingParts<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Visibility,
        Option<&'static mut Mesh3d>,
        Option<&'static mut MeshMaterial3d<StandardMaterial>>,
    ),
    Without<BuildingVisual>,
>;

/// The mode's camera as driven by [`drive_camera`].
type CameraRig<'w, 's> = Single<
    'w,
    's,
    (
        Entity,
        &'static Camera,
        &'static mut OrbitCameraRig,
        &'static mut PanOrbitCamera,
        &'static Projection,
        Option<&'static FollowTarget>,
    ),
    With<CivCamera>,
>;

/// Map state kept between frames.
#[derive(Resource, Default)]
pub struct MapCache {
    /// Height field of the current terrain.
    pub field: Option<HeightField>,
    /// Map size in tiles.
    pub size: UVec2,
    terrain: Vec<Terrain>,
    roads: Vec<f32>,
    road_threshold: f32,
    roads_built_at: f64,
    territory: TerritoryKey,
    stocks_built_at: f64,
    vegetation: HashMap<u32, Vec<Entity>>,
    vegetation_terrain: Option<Vec<Terrain>>,
    /// Scenario id the map was built for (to re-frame the camera).
    pub scenario: String,
}

/// A tree, bush or rock standing on a tile.
#[derive(Component, Debug, Clone, Copy)]
pub struct Vegetation {
    /// Tile index.
    pub tile: u32,
    /// Kind and current level.
    pub kind: VegetationKind,
}

/// Kind of vegetation with its stock-dependent level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VegetationKind {
    /// Tree on a forest tile; growth follows the wood stock.
    Tree {
        /// Growth for the tile's stock (before `factor` and `cap`).
        growth: f32,
        /// Size factor of this tree relative to the tile's main tree.
        factor: f32,
        /// Largest growth drawn for the style (detail budget).
        cap: f32,
    },
    /// Berry bush; level follows the berry stock.
    Bush {
        /// Berry level (see [`berry_level`]).
        level: usize,
        /// Shape variant.
        variant: usize,
    },
    /// Rock outcrop; size follows the stone stock.
    Rock {
        /// Base scale.
        scale: f32,
    },
}

/// Visual of a building.
#[derive(Component, Debug, Clone)]
pub struct BuildingVisual {
    /// Building.
    pub id: BuildingId,
    /// Kind.
    pub kind: BuildingKind,
    /// Tile coordinates.
    pub tile: (u32, u32),
    progress: f32,
    group: Option<u32>,
    crop: Option<usize>,
    body: Entity,
    scaffold: Entity,
    pennant: Option<Entity>,
    crops: Option<Entity>,
}

/// Swaying pennant.
#[derive(Component, Debug, Clone, Copy)]
pub struct Pennant {
    phase: f32,
    base_yaw: f32,
}

/// Building visuals by id.
#[derive(Resource, Default)]
pub struct BuildingIndex(pub HashMap<BuildingId, Entity>);

/// Sun direction (light travels along −Z of this transform).
pub fn sun_transform() -> Transform {
    Transform::from_xyz(0.0, 0.0, 0.0).looking_to(Vec3::new(-0.55, -0.9, -0.35).normalize(), Vec3::Y)
}

/// Spawns the camera and lights (`OnEnter`).
pub fn spawn_environment(mut commands: Commands) {
    commands.spawn((
        Name::new("Civilization camera"),
        CivCamera,
        Camera3d::default(),
        OrbitCameraRig::new(rendering::OrbitPreset::Strategic).with_distance_limits(2.5, 260.0),
        // North (row 0) at the top of the screen: the camera looks along −Z.
        PanOrbitCamera { focus: Vec3::new(32.0, 0.0, 32.0), yaw: Some(0.0), radius: Some(60.0), ..default() },
        Transform::from_xyz(32.0, 50.0, 70.0).looking_at(Vec3::new(32.0, 0.0, 32.0), Vec3::Y),
        Msaa::Sample4,
        AmbientLight { color: Color::srgb(0.8, 0.87, 1.0), brightness: 380.0, ..default() },
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Civilization sun"),
        Sun,
        DirectionalLight {
            illuminance: 8_000.0,
            color: Color::srgb(1.0, 0.96, 0.88),
            shadow_maps_enabled: true,
            ..default()
        },
        sun_transform(),
        CascadeShadowConfigBuilder {
            num_cascades: 3,
            first_cascade_far_bound: 25.0,
            maximum_distance: 220.0,
            ..default()
        }
        .build(),
        DespawnOnExit(MODE),
    ));
}

/// Pulls the newest published frame from the runner.
pub fn pull_frame(runner: Option<Res<crate::state::CivRunner>>, mut current: ResMut<CurrentFrame>) {
    let Some(runner) = runner else { return };
    let Some(published) = runner.0.latest_view() else { return };
    if published.seq != current.seq || current.frame.is_none() {
        current.seq = published.seq;
        current.frame = Some(published.view);
    }
}

/// Sends the view request when the selection or trade window changed.
pub fn send_request(runner: Option<Res<crate::state::CivRunner>>, mut state: ResMut<ViewState>) {
    let Some(runner) = runner else { return };
    let request = state.request();
    if state.sent != Some(request) {
        runner.0.send(simulation_core::Control::SetViewRequest(request));
        state.sent = Some(request);
    }
}

fn spawn_mesh_layer(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mesh: Mesh,
    layer: MapLayer,
    material: impl Bundle,
    lift: f32,
) {
    commands.spawn((
        Name::new(format!("{layer:?}")),
        layer,
        Mesh3d(meshes.add(mesh)),
        material,
        Transform::from_xyz(0.0, lift, 0.0),
        DespawnOnExit(MODE),
    ));
}

/// Rebuilds terrain, water, roads and territory meshes when they changed.
#[allow(clippy::too_many_arguments)]
pub fn sync_map(
    mut commands: Commands,
    current: Res<CurrentFrame>,
    time: Res<Time<Real>>,
    state: Res<ViewState>,
    mut cache: ResMut<MapCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: Res<CivMaterials>,
    layers: Query<(Entity, &MapLayer)>,
    mut visibility: Query<(&MapLayer, &mut Visibility)>,
) {
    if state.is_changed() {
        for (layer, mut vis) in &mut visibility {
            let on = match layer {
                MapLayer::Roads => state.layers.roads,
                MapLayer::Territory => state.layers.territory,
                _ => true,
            };
            vis.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    if !current.is_changed() {
        return;
    }
    let Some(frame) = current.get() else { return };
    let view = &frame.view;
    let (w, h) = (view.world_width, view.world_height);
    if view.terrain.len() != (w * h) as usize || w < 2 || h < 2 {
        return;
    }
    let now = time.elapsed_secs_f64();
    let despawn = |commands: &mut Commands, which: MapLayer| {
        for (e, l) in &layers {
            if *l == which {
                commands.entity(e).despawn();
            }
        }
    };
    let rebuilt = cache.terrain != view.terrain || cache.size != UVec2::new(w, h);
    if rebuilt {
        let field = HeightField::from_terrain(&view.terrain, w, h);
        despawn(&mut commands, MapLayer::Terrain);
        despawn(&mut commands, MapLayer::Water);
        let terrain = terrain_mesh(&view.terrain, &field).to_mesh();
        spawn_mesh_layer(
            &mut commands,
            &mut meshes,
            terrain,
            MapLayer::Terrain,
            MeshMaterial3d(materials.white.clone()),
            0.0,
        );
        let water = water_mesh(&view.terrain, &field);
        if !water.positions.is_empty() {
            spawn_mesh_layer(
                &mut commands,
                &mut meshes,
                water.to_mesh(),
                MapLayer::Water,
                MeshMaterial3d(materials.water.clone()),
                0.0,
            );
        }
        cache.terrain = view.terrain.clone();
        cache.size = UVec2::new(w, h);
        cache.field = Some(field);
        // Force the overlays to follow the new surface.
        cache.roads.clear();
        cache.territory = Default::default();
    }
    let Some(field) = cache.field.clone() else { return };
    let roads_changed = cache.road_threshold != view.road_threshold
        || cache.roads.len() != view.roads.len()
        || cache.roads.iter().zip(&view.roads).any(|(a, b)| (a - b).abs() > 0.004);
    if roads_changed && (now - cache.roads_built_at > 0.25 || cache.roads.len() != view.roads.len()) {
        despawn(&mut commands, MapLayer::Roads);
        let mesh = road_mesh(&view.roads, view.road_threshold, &field);
        if !mesh.positions.is_empty() {
            commands.spawn((
                Name::new("Roads"),
                MapLayer::Roads,
                Mesh3d(meshes.add(mesh.to_mesh())),
                MeshMaterial3d(materials.road.clone()),
                Transform::from_xyz(0.0, 0.004, 0.0),
                if state.layers.roads { Visibility::Inherited } else { Visibility::Hidden },
                bevy::light::NotShadowCaster,
                DespawnOnExit(MODE),
            ));
        }
        cache.roads = view.roads.clone();
        cache.road_threshold = view.road_threshold;
        cache.roads_built_at = now;
    }
    let colours: Vec<(u32, [u8; 3])> = view
        .groups
        .iter()
        .map(|g| (g.id.0, crate::palette::group_color(Some(g.color)).to_srgba().to_u8_array_no_alpha()))
        .collect();
    let territory_key = (view.territory.clone(), view.contested.clone(), colours.clone());
    if cache.territory != territory_key {
        despawn(&mut commands, MapLayer::Territory);
        let lookup = |id: u32| {
            colours.iter().find(|(g, _)| *g == id).map(|(_, c)| {
                let l = Color::srgb_u8(c[0], c[1], c[2]).to_linear();
                [l.red, l.green, l.blue]
            })
        };
        let mesh = territory_mesh(&view.territory, &view.contested, &view.terrain, &field, &lookup);
        if !mesh.positions.is_empty() {
            commands.spawn((
                Name::new("Territory"),
                MapLayer::Territory,
                Mesh3d(meshes.add(mesh.to_mesh())),
                MeshMaterial3d(materials.overlay.clone()),
                Transform::IDENTITY,
                if state.layers.territory { Visibility::Inherited } else { Visibility::Hidden },
                bevy::light::NotShadowCaster,
                DespawnOnExit(MODE),
            ));
        }
        cache.territory = territory_key;
    }
}

/// Visible tree growth for a wood stock fraction (quantised so trees are
/// regrown rarely): saplings on logged-out tiles, mature trees on full ones.
pub fn tree_growth(stock: f32) -> f32 {
    let level = (stock.clamp(0.0, 1.0) * 6.0).round() / 6.0;
    0.22 + 0.5 * level
}

/// Rock scale for a stone stock fraction.
pub fn rock_scale(stock: f32) -> f32 {
    0.55 + 0.45 * ((stock.clamp(0.0, 1.0) * 4.0).round() / 4.0)
}

/// Tiles whose vegetation must be (re)built when the terrain changes from
/// `old` to `new` (`width` columns): every tile the first time or after a
/// size change, else the changed tiles and their neighbours (whose surface
/// heights follow the shared corners).
pub fn vegetation_dirty(old: Option<&[Terrain]>, new: &[Terrain], width: u32) -> Vec<usize> {
    let Some(old) = old.filter(|o| o.len() == new.len()) else { return (0..new.len()).collect() };
    let w = width.max(1) as i64;
    let h = new.len() as i64 / w;
    let mut dirty = vec![false; new.len()];
    for (i, _) in old.iter().zip(new).enumerate().filter(|(_, (a, b))| a != b) {
        let (x, y) = (i as i64 % w, i as i64 / w);
        for ny in (y - 1).max(0)..=(y + 1).min(h - 1) {
            for nx in (x - 1).max(0)..=(x + 1).min(w - 1) {
                dirty[(ny * w + nx) as usize] = true;
            }
        }
    }
    dirty.iter().enumerate().filter(|(_, d)| **d).map(|(i, _)| i).collect()
}

/// Trees, bushes or rocks of one tile, as their terrain and stock require.
#[allow(clippy::too_many_arguments)]
fn spawn_tile_vegetation(
    commands: &mut Commands,
    i: usize,
    terrain: Terrain,
    stock: f32,
    width: u32,
    field: &HeightField,
    assets: &crate::people::CivMeshes,
    materials: &CivMaterials,
    shown: Visibility,
) -> Vec<Entity> {
    let (x, y) = (i as u32 % width, i as u32 / width);
    let mut spawned = Vec::new();
    match terrain {
        Terrain::Forest => {
            let broadleaf = tile_hash(x, y, 21) < 0.3;
            let count = if tile_hash(x, y, 23) < 0.45 { 2 } else { 1 };
            for (k, p) in scatter_on_tile(x, y, count, 0.28, 5).into_iter().enumerate() {
                let factor = if k == 1 { 0.8 } else { 1.0 };
                // Mature broadleaf crowns are very detailed: they are drawn
                // young and scaled up instead.
                let (style, scale, cap) =
                    if broadleaf { (TreeStyle::Broadleaf, 0.12, 0.34) } else { (TreeStyle::Conifer, 0.072, 1.0) };
                let base = tree_growth(stock);
                let growth = (base * factor).min(cap);
                let seed = u64::from(x) * 7919 + u64::from(y) * 104_729 + k as u64;
                let e = commands
                    .spawn((
                        Vegetation { tile: i as u32, kind: VegetationKind::Tree { growth: base, factor, cap } },
                        ProceduralTree { style, growth, seed, burnt: false },
                        Transform::from_translation(to_world(p, field.height_at(p) - 0.01))
                            .with_rotation(Quat::from_rotation_y(tile_hash(x, y, 40 + k as u64) * TAU))
                            .with_scale(Vec3::splat(scale * (0.85 + 0.3 * tile_hash(x, y, 50 + k as u64)))),
                        shown,
                        DespawnOnExit(MODE),
                    ))
                    .id();
                spawned.push(e);
            }
        }
        Terrain::Berries => {
            for (k, p) in scatter_on_tile(x, y, 2, 0.24, 9).into_iter().enumerate() {
                let variant = (tile_hash(x, y, 60 + k as u64) * 2.0) as usize % 2;
                let level = berry_level(stock);
                let e = commands
                    .spawn((
                        Vegetation { tile: i as u32, kind: VegetationKind::Bush { level, variant } },
                        Mesh3d(assets.bushes[level][variant].clone()),
                        MeshMaterial3d(materials.white.clone()),
                        Transform::from_translation(to_world(p, field.height_at(p) - 0.01))
                            .with_rotation(Quat::from_rotation_y(tile_hash(x, y, 70 + k as u64) * TAU))
                            .with_scale(Vec3::splat(0.9 + 0.3 * tile_hash(x, y, 80 + k as u64))),
                        shown,
                        DespawnOnExit(MODE),
                    ))
                    .id();
                spawned.push(e);
            }
        }
        Terrain::Hills => {
            let count = 1 + (tile_hash(x, y, 90) * 2.0) as usize;
            for (k, p) in scatter_on_tile(x, y, count, 0.25, 13).into_iter().enumerate() {
                let variant = (tile_hash(x, y, 100 + k as u64) * 4.0) as usize % 4;
                let scale = 1.05 + 0.6 * tile_hash(x, y, 110 + k as u64);
                let e = commands
                    .spawn((
                        Vegetation { tile: i as u32, kind: VegetationKind::Rock { scale } },
                        Mesh3d(assets.rocks[variant].clone()),
                        MeshMaterial3d(materials.white.clone()),
                        Transform::from_translation(to_world(p, field.height_at(p) - 0.02))
                            .with_rotation(Quat::from_rotation_y(tile_hash(x, y, 120 + k as u64) * TAU))
                            .with_scale(Vec3::splat(scale * rock_scale(stock))),
                        shown,
                        DespawnOnExit(MODE),
                    ))
                    .id();
                spawned.push(e);
            }
        }
        _ => {}
    }
    spawned
}

/// Spawns, updates and removes trees, bushes and rocks.
#[allow(clippy::too_many_arguments)]
pub fn sync_vegetation(
    mut commands: Commands,
    current: Res<CurrentFrame>,
    time: Res<Time<Real>>,
    state: Res<ViewState>,
    mut cache: ResMut<MapCache>,
    assets: Res<crate::people::CivMeshes>,
    materials: Res<CivMaterials>,
    mut plants: Query<(&mut Vegetation, &mut Transform, Option<&mut ProceduralTree>, Option<&mut Mesh3d>)>,
    mut visibility: Query<&mut Visibility, With<Vegetation>>,
) {
    if state.is_changed() {
        let v = if state.layers.vegetation { Visibility::Inherited } else { Visibility::Hidden };
        for mut vis in &mut visibility {
            vis.set_if_neq(v);
        }
    }
    if !current.is_changed() {
        return;
    }
    let Some(frame) = current.get() else { return };
    let Some(field) = cache.field.clone() else { return };
    let view = &frame.view;
    let now = time.elapsed_secs_f64();
    if cache.terrain_changed_since_vegetation(&view.terrain) {
        // Only the tiles whose terrain (or a neighbour's) changed are rebuilt:
        // an "add resources" click does not respawn the whole forest.
        let resized = cache.vegetation_terrain.as_ref().is_none_or(|old| old.len() != view.terrain.len());
        if resized {
            for (_, list) in cache.vegetation.drain() {
                for e in list {
                    commands.entity(e).despawn();
                }
            }
        }
        let dirty = vegetation_dirty(cache.vegetation_terrain.as_deref(), &view.terrain, view.world_width);
        let shown = if state.layers.vegetation { Visibility::Inherited } else { Visibility::Hidden };
        for i in dirty {
            if let Some(list) = cache.vegetation.remove(&(i as u32)) {
                for e in list {
                    commands.entity(e).despawn();
                }
            }
            let stock = view.resources.get(i).copied().unwrap_or(0.0);
            let spawned = spawn_tile_vegetation(
                &mut commands,
                i,
                view.terrain[i],
                stock,
                view.world_width,
                &field,
                &assets,
                &materials,
                shown,
            );
            if !spawned.is_empty() {
                cache.vegetation.insert(i as u32, spawned);
            }
        }
        cache.mark_vegetation(&view.terrain);
        cache.stocks_built_at = now;
        return;
    }
    if now - cache.stocks_built_at < 0.5 {
        return;
    }
    cache.stocks_built_at = now;
    for (mut veg, mut transform, tree, mesh) in &mut plants {
        let stock = view.resources.get(veg.tile as usize).copied().unwrap_or(0.0);
        match &mut veg.kind {
            VegetationKind::Tree { growth, factor, cap } => {
                let next = tree_growth(stock);
                if (*growth - next).abs() > 1e-3 {
                    *growth = next;
                    if let Some(mut tree) = tree {
                        tree.growth = (next * *factor).min(*cap);
                    }
                }
            }
            VegetationKind::Bush { level, variant } => {
                let next = berry_level(stock).min(BERRY_LEVELS - 1);
                if next != *level {
                    *level = next;
                    if let Some(mut mesh) = mesh {
                        mesh.0 = assets.bushes[next][*variant].clone();
                    }
                }
            }
            VegetationKind::Rock { scale } => {
                let s = *scale * rock_scale(stock);
                if (transform.scale.x - s).abs() > 1e-3 {
                    transform.scale = Vec3::splat(s);
                }
            }
        }
    }
}

impl MapCache {
    fn terrain_changed_since_vegetation(&self, terrain: &[Terrain]) -> bool {
        self.vegetation_terrain.as_deref() != Some(terrain)
    }

    fn mark_vegetation(&mut self, terrain: &[Terrain]) {
        self.vegetation_terrain = Some(terrain.to_vec());
    }
}

/// Spawns, updates and removes building visuals.
#[allow(clippy::too_many_arguments)]
pub fn sync_buildings(
    mut commands: Commands,
    current: Res<CurrentFrame>,
    cache: Res<MapCache>,
    assets: Res<crate::people::CivMeshes>,
    mut materials: ResMut<CivMaterials>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut index: ResMut<BuildingIndex>,
    mut visuals: Query<&mut BuildingVisual>,
    mut parts: BuildingParts,
) {
    if !current.is_changed() {
        return;
    }
    let Some(frame) = current.get() else { return };
    let Some(field) = cache.field.as_ref() else { return };
    let view = &frame.view;
    let group_color = |g: Option<u32>| {
        g.and_then(|id| view.groups.iter().find(|x| x.id.0 == id)).map(|x| crate::palette::group_color(Some(x.color)))
    };
    let mut alive = Vec::with_capacity(view.buildings.len());
    for b in &view.buildings {
        alive.push(b.id);
        let group = b.group.map(|g| g.0);
        let crop = b.crop_growth.map(crop_stage);
        if let Some(&root) = index.0.get(&b.id)
            && let Ok(mut visual) = visuals.get_mut(root)
            && visual.kind == b.kind
            && visual.tile == b.tile
        {
            if (visual.progress - b.progress).abs() > 1e-3 {
                visual.progress = b.progress;
                apply_progress(&visual, &mut parts);
            }
            if visual.group != group {
                visual.group = group;
                if let Some(flag) = visual.pennant
                    && let Ok((_, mut vis, _, material)) = parts.get_mut(flag)
                {
                    match group_color(group) {
                        Some(c) => {
                            if let Some(mut m) = material {
                                m.0 = materials.group(c, &mut standard);
                            }
                            *vis = if visual.progress >= 1.0 { Visibility::Inherited } else { Visibility::Hidden };
                        }
                        None => *vis = Visibility::Hidden,
                    }
                }
            }
            if visual.crop != crop {
                visual.crop = crop;
                if let Some(crops) = visual.crops
                    && let Ok((_, mut vis, mesh, _)) = parts.get_mut(crops)
                {
                    match crop {
                        Some(stage) => {
                            if let Some(mut mesh) = mesh {
                                mesh.0 = assets.crops[stage.min(CROP_STAGES - 1)].clone();
                            }
                            *vis = Visibility::Inherited;
                        }
                        None => *vis = Visibility::Hidden,
                    }
                }
            }
            continue;
        }
        if let Some(old) = index.0.remove(&b.id) {
            commands.entity(old).despawn();
        }
        let (x, y) = b.tile;
        let centre = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
        let yaw = match b.kind {
            BuildingKind::Hut => tile_hash(x, y, 140) * TAU,
            _ => (tile_hash(x, y, 141) * 4.0).floor() * FRAC_PI_2,
        };
        let root = commands
            .spawn((
                Name::new(format!("{} #{}", b.kind.label(), b.id.0)),
                Transform::from_translation(to_world(centre, field.centre(x, y) - 0.005))
                    .with_rotation(Quat::from_rotation_y(yaw)),
                Visibility::Inherited,
                DespawnOnExit(MODE),
            ))
            .id();
        let mesh = match b.kind {
            BuildingKind::Hut => assets.hut.clone(),
            BuildingKind::Storage => assets.storage.clone(),
            BuildingKind::FarmPlot => assets.farm.clone(),
        };
        let body = commands
            .spawn((Mesh3d(mesh), MeshMaterial3d(materials.white.clone()), Transform::IDENTITY, ChildOf(root)))
            .id();
        let scaffold = commands
            .spawn((
                Mesh3d(assets.scaffold.clone()),
                MeshMaterial3d(materials.white.clone()),
                Transform::IDENTITY,
                Visibility::Hidden,
                ChildOf(root),
            ))
            .id();
        let pennant = (b.kind != BuildingKind::FarmPlot).then(|| {
            let (pos, phase) = match b.kind {
                BuildingKind::Hut => (Vec3::Y * crate::meshes::HUT_FLAG_HEIGHT, tile_hash(x, y, 150) * TAU),
                _ => (Vec3::new(0.3, crate::meshes::STORE_FLAG_HEIGHT, 0.0), tile_hash(x, y, 151) * TAU),
            };
            let colour = group_color(group).unwrap_or(Color::WHITE);
            let visible = group.is_some() && b.progress >= 1.0;
            commands
                .spawn((
                    Mesh3d(assets.pennant.clone()),
                    MeshMaterial3d(materials.group(colour, &mut standard)),
                    Transform::from_translation(pos),
                    if visible { Visibility::Inherited } else { Visibility::Hidden },
                    Pennant { phase, base_yaw: -yaw + 0.6 },
                    ChildOf(root),
                ))
                .id()
        });
        let crops = (b.kind == BuildingKind::FarmPlot).then(|| {
            let stage = crop.unwrap_or(0);
            commands
                .spawn((
                    Mesh3d(assets.crops[stage].clone()),
                    MeshMaterial3d(materials.white.clone()),
                    Transform::IDENTITY,
                    if crop.is_some() { Visibility::Inherited } else { Visibility::Hidden },
                    ChildOf(root),
                ))
                .id()
        });
        let visual = BuildingVisual {
            id: b.id,
            kind: b.kind,
            tile: b.tile,
            progress: b.progress,
            group,
            crop,
            body,
            scaffold,
            pennant,
            crops,
        };
        commands.entity(root).insert(visual.clone());
        // Initial progress pose, applied directly to the new children.
        let (body_scale, scaffold_visible) = progress_pose(b.kind, b.progress);
        commands.entity(body).insert(Transform::from_scale(body_scale));
        commands.entity(scaffold).insert(if scaffold_visible { Visibility::Inherited } else { Visibility::Hidden });
        index.0.insert(b.id, root);
    }
    index.0.retain(|id, root| {
        let keep = alive.contains(id);
        if !keep {
            commands.entity(*root).despawn();
        }
        keep
    });
}

/// Scale of a building body and scaffold visibility for a construction progress.
pub fn progress_pose(kind: BuildingKind, progress: f32) -> (Vec3, bool) {
    let p = progress.clamp(0.0, 1.0);
    let complete = p >= 1.0;
    match kind {
        BuildingKind::FarmPlot => (Vec3::new(1.0, 0.3 + 0.7 * p, 1.0), !complete && p < 0.5),
        _ => (Vec3::new(1.0, 0.12 + 0.88 * p, 1.0), !complete),
    }
}

fn apply_progress(visual: &BuildingVisual, parts: &mut BuildingParts) {
    let (scale, scaffold) = progress_pose(visual.kind, visual.progress);
    if let Ok((mut t, ..)) = parts.get_mut(visual.body) {
        t.scale = scale;
    }
    if let Ok((_, mut vis, ..)) = parts.get_mut(visual.scaffold) {
        *vis = if scaffold { Visibility::Inherited } else { Visibility::Hidden };
    }
    if let Some(flag) = visual.pennant
        && let Ok((_, mut vis, ..)) = parts.get_mut(flag)
    {
        *vis =
            if visual.group.is_some() && visual.progress >= 1.0 { Visibility::Inherited } else { Visibility::Hidden };
    }
}

/// Pennants flutter in the wind.
pub fn wave_pennants(time: Res<Time>, mut flags: Query<(&Pennant, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (p, mut transform) in &mut flags {
        let yaw = p.base_yaw + 0.35 * (t * 1.7 + p.phase).sin() + 0.12 * (t * 4.3 + 2.0 * p.phase).sin();
        transform.rotation = Quat::from_rotation_y(yaw);
    }
}

/// Applies the camera preset, following and framing requests.
///
/// The whole map is framed on entering, on a new scenario, on request (G,
/// "Frame map") and, while the camera still shows the framed map, when the
/// preset or the viewport's shape changes (a switch to the isometric
/// diamond, panels shown or hidden, a resized window); a camera the user
/// moved keeps its focus and distance.
#[allow(clippy::too_many_arguments)]
pub fn drive_camera(
    mut waited: Local<u32>,
    mut framed: Local<Option<(Vec3, f32, f32)>>,
    mut commands: Commands,
    mut state: ResMut<ViewState>,
    cache: Res<MapCache>,
    current: Res<CurrentFrame>,
    people: Res<PeopleIndex>,
    overlay: Res<crate::ui::overlay::OverlayRect>,
    layout: Option<Res<interface::ShellLayout>>,
    camera: Option<CameraRig>,
) {
    let Some(camera) = camera else { return };
    let (entity, cam, mut rig, mut pan, projection, follow) = camera.into_inner();
    let waited = &mut *waited;
    let switching = rig.preset != state.camera;
    let aspect = cam.logical_viewport_size().map_or(1.3, |s| (s.x / s.y.max(1.0)).clamp(0.3, 5.0));
    // Distance in world units whatever the projection (on a preset switch
    // the rig converts units later this frame).
    let distance_now = match projection {
        Projection::Orthographic(_) => rendering::orbit::ortho_scale_to_distance(pan.target_radius, rig.fov),
        _ => pan.target_radius,
    };
    let untouched = framed.is_some_and(|(focus, d, _)| {
        pan.target_focus.distance(focus) < 0.05 && (distance_now - d).abs() < 0.01 * d.max(1.0)
    });
    let reshaped = framed.is_some_and(|(.., a)| (a - aspect).abs() > 0.01 * a);
    if untouched && (switching || reshaped) && !state.follow && cam.viewport.is_some() {
        state.frame_world = true;
    }
    if switching {
        rig.preset = state.camera;
    }
    // A new scenario (other map) is framed automatically, once its map is built.
    if let Some(frame) = current.get()
        && cache.scenario != frame.info.id
        && cache.field.is_some()
        && cache.size == UVec2::new(frame.view.world_width, frame.view.world_height)
    {
        state.frame_world = true;
    }
    // Wait for the shell to give the camera its viewport (the aspect ratio
    // of the free area decides the framing distance) and for the overlay
    // panel to be laid out (the map is framed clear of it).
    let viewport_known = (cam.viewport.is_some() && overlay.0.is_some()) || *waited > 30;
    if state.frame_world && cache.field.is_some() && !viewport_known {
        *waited += 1;
    }
    if state.frame_world && cache.field.is_some() && viewport_known {
        *waited = 0;
        let cfg = state.camera.config();
        let pitch = if switching || !pan.initialized {
            cfg.pitch
        } else {
            pan.target_pitch.clamp(cfg.pitch_limits.0.unwrap_or(0.05), cfg.pitch_limits.1.unwrap_or(1.55))
        };
        let view = FramingView {
            yaw: cfg.yaw.unwrap_or(if pan.initialized { pan.target_yaw } else { pan.yaw.unwrap_or(0.0) }),
            pitch,
            fov: rig.fov,
            aspect,
            orthographic: cfg.orthographic,
        };
        let clear_of = layout.as_ref().and_then(|l| l.central).zip(overlay.0);
        let (centre, distance) = frame_map_clear_of(cache.size.as_vec2(), &view, FRAMING_MARGIN, clear_of);
        rig.focus_on(&mut pan, projection, centre, distance);
        *framed = Some((pan.target_focus, distance, aspect));
        state.frame_world = false;
        commands.entity(entity).remove::<FollowTarget>();
        state.follow = false;
    }
    let target = match (state.follow, state.selection) {
        (true, Selection::Agent(id)) => people.0.get(&id).copied(),
        _ => None,
    };
    match (target, follow) {
        (Some(e), Some(f)) if f.subject == rendering::FollowSubject::Entity(e) => {}
        (Some(e), _) => {
            commands.entity(entity).insert(FollowTarget::entity(e));
            let (focus, radius) = (pan.target_focus, pan.target_radius.min(18.0));
            rig.focus_on(&mut pan, projection, focus, radius);
        }
        (None, Some(_)) => {
            commands.entity(entity).remove::<FollowTarget>();
            if state.follow {
                state.follow = false;
            }
        }
        (None, None) => {}
    }
}

/// Largest normalised device coordinate (|x| or |y|) the map may reach
/// when framed: a margin of 8 % of the half-viewport on the tightest side.
pub const FRAMING_MARGIN: f32 = 0.92;

/// Camera parameters that decide how the map projects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramingView {
    /// Orbit yaw (radians, `bevy_panorbit_camera` convention).
    pub yaw: f32,
    /// Orbit pitch (radians above the horizon).
    pub pitch: f32,
    /// Vertical field of view (also converts distances to orthographic scales).
    pub fov: f32,
    /// Viewport width / height.
    pub aspect: f32,
    /// Orthographic projection (scale = `2·distance·tan(fov/2)`).
    pub orthographic: bool,
}

impl FramingView {
    /// Normalised device coordinates of `point` for a camera orbiting
    /// `focus` at `distance` (as `bevy_panorbit_camera` places it:
    /// rotation `Ry(yaw)·Rx(−pitch)`, position `focus + rotation·(0, 0, r)`);
    /// `None` behind or too close to the camera.
    pub fn project(&self, focus: Vec3, distance: f32, point: Vec3) -> Option<Vec2> {
        let rotation = Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(-self.pitch);
        let (right, up, forward) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::NEG_Z);
        let half = (self.fov * 0.5).tan();
        if self.orthographic {
            let half_height = distance * half;
            let rel = point - focus;
            return Some(Vec2::new(rel.dot(right) / (half_height * self.aspect), rel.dot(up) / half_height));
        }
        let eye = focus + rotation * Vec3::new(0.0, 0.0, distance);
        let rel = point - eye;
        let depth = rel.dot(forward);
        (depth > 0.1).then(|| Vec2::new(rel.dot(right) / (depth * half * self.aspect), rel.dot(up) / (depth * half)))
    }

    /// Bounds `(min, max)` of the projected map corners (surface and the
    /// bottom of the earth skirt), `None` if a corner is behind the camera.
    fn bounds(&self, size: Vec2, focus: Vec3, distance: f32) -> Option<(Vec2, Vec2)> {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for (x, z) in [(0.0, 0.0), (size.x, 0.0), (0.0, size.y), (size.x, size.y)] {
            for y in [crate::world_mesh::SKIRT_BOTTOM, 0.6] {
                let p = self.project(focus, distance, Vec3::new(x, y, z))?;
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        Some((lo, hi))
    }
}

/// Part of the viewport the map is framed in, in normalised device
/// coordinates (`x` right, `y` up, the whole viewport is `[-1, 1]²`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NdcWindow {
    /// Lower-left corner.
    pub min: Vec2,
    /// Upper-right corner.
    pub max: Vec2,
}

impl NdcWindow {
    /// The whole viewport.
    pub const FULL: Self = Self { min: Vec2::NEG_ONE, max: Vec2::ONE };

    /// Centre of the window.
    pub fn centre(&self) -> Vec2 {
        0.5 * (self.min + self.max)
    }

    /// Half extent of the window.
    pub fn half(&self) -> Vec2 {
        0.5 * (self.max - self.min)
    }

    /// Windows of a viewport `central` (logical px, y down) that leave out
    /// an `overlay` rectangle anchored at its top-left: the band below it and
    /// the band to its right (`None` if a band would be too thin to use).
    pub fn beside_overlay(central: Rect, overlay: Rect) -> [Option<Self>; 2] {
        let size = central.size();
        if size.x < 1.0 || size.y < 1.0 {
            return [None, None];
        }
        let below = 1.0 - 2.0 * ((overlay.max.y - central.min.y) / size.y).clamp(0.0, 1.0);
        let right = -1.0 + 2.0 * ((overlay.max.x - central.min.x) / size.x).clamp(0.0, 1.0);
        let band = |w: Self| (w.half().min_element() > 0.3).then_some(w);
        [
            band(Self { min: Vec2::NEG_ONE, max: Vec2::new(1.0, below) }),
            band(Self { min: Vec2::new(right, -1.0), max: Vec2::ONE }),
        ]
    }
}

/// Focus and distance that show a whole `size` map (tiles from the origin)
/// as large as possible inside `margin` (NDC), centred in the viewport.
pub fn frame_map(size: Vec2, view: &FramingView, margin: f32) -> (Vec3, f32) {
    frame_map_in(size, view, margin, NdcWindow::FULL)
}

/// [`frame_map`] inside a part of the viewport: the map fills at most
/// `margin` of the window's half extent on its tightest side and is centred
/// in it.
pub fn frame_map_in(size: Vec2, view: &FramingView, margin: f32, window: NdcWindow) -> (Vec3, f32) {
    let size = size.max(Vec2::ONE);
    let (centre, half) = (window.centre(), window.half() * margin);
    let mut focus = Vec3::new(size.x * 0.5, 0.0, size.y * 0.5);
    let fits = |focus: Vec3, d: f32| {
        view.bounds(size, focus, d).is_some_and(|(lo, hi)| {
            let reach = (lo - centre).abs().max((hi - centre).abs());
            reach.x <= half.x && reach.y <= half.y
        })
    };
    // Ground directions that move the view right and up the screen.
    let rotation = Quat::from_rotation_y(view.yaw);
    let (right, ahead) = (rotation * Vec3::X, rotation * Vec3::NEG_Z);
    let mut distance = 4.0 * size.max_element();
    for _ in 0..6 {
        // Shortest distance that fits (the projection shrinks with distance).
        let (mut near, mut far) = (0.5, 40.0 * size.max_element());
        if !fits(focus, far) {
            break;
        }
        for _ in 0..40 {
            let mid = 0.5 * (near + far);
            if fits(focus, mid) { far = mid } else { near = mid }
        }
        distance = far;
        // Centre the map in the window: shift the focus along the ground by
        // the offset of the projected centre (one NDC unit is
        // `d·tan(fov/2)` across the view, `/ sin(pitch)` along the ground).
        let Some((lo, hi)) = view.bounds(size, focus, distance) else { break };
        let offset = 0.5 * (lo + hi) - centre;
        if offset.abs().max_element() < 0.01 {
            break;
        }
        let unit = distance * (view.fov * 0.5).tan();
        focus += right * offset.x * unit * view.aspect * 0.9;
        focus += ahead * offset.y * unit / view.pitch.sin().max(0.2) * 0.9;
        focus.x = focus.x.clamp(0.0, size.x);
        focus.z = focus.z.clamp(0.0, size.y);
    }
    (focus, distance)
}

/// Framing of the whole map that keeps it clear of the overlay panel when
/// that costs at most 25 % of the map's size (the band below or beside the
/// overlay), else the framing of the whole viewport.
pub fn frame_map_clear_of(size: Vec2, view: &FramingView, margin: f32, viewport: Option<(Rect, Rect)>) -> (Vec3, f32) {
    let full = frame_map(size, view, margin);
    let Some((central, overlay)) = viewport else { return full };
    NdcWindow::beside_overlay(central, overlay)
        .into_iter()
        .flatten()
        .map(|w| frame_map_in(size, view, margin, w))
        .filter(|(_, d)| *d <= full.1 * 1.25)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or(full)
}

/// Records which scenario the map shows (after `drive_camera` framed it).
pub fn note_scenario(current: Res<CurrentFrame>, mut cache: ResMut<MapCache>) {
    if let Some(frame) = current.get()
        && cache.field.is_some()
        && cache.size == UVec2::new(frame.view.world_width, frame.view.world_height)
        && cache.scenario != frame.info.id
    {
        cache.scenario = frame.info.id.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stocks_map_to_visual_levels() {
        assert!(tree_growth(1.0) > tree_growth(0.0));
        assert_eq!(tree_growth(0.51), tree_growth(0.49), "quantised: small changes do not regrow trees");
        assert!(tree_growth(0.0) > 0.0, "logged-out tiles keep saplings");
        assert!(rock_scale(1.0) > rock_scale(0.0) && rock_scale(0.0) > 0.0);
        assert_eq!(rock_scale(f32::NAN.max(0.0)), rock_scale(0.0));
    }

    #[test]
    fn vegetation_is_rebuilt_only_around_changed_tiles() {
        use Terrain::{Forest, Grassland};
        let old = vec![Grassland; 25];
        assert_eq!(vegetation_dirty(None, &old, 5).len(), 25, "first build");
        assert_eq!(vegetation_dirty(Some(&old[..20]), &old, 5).len(), 25, "resized map");
        assert!(vegetation_dirty(Some(&old), &old, 5).is_empty(), "nothing changed");
        let mut new = old.clone();
        new[12] = Forest; // centre of a 5 × 5 map
        assert_eq!(vegetation_dirty(Some(&old), &new, 5), vec![6, 7, 8, 11, 12, 13, 16, 17, 18]);
        let mut corner = old.clone();
        corner[0] = Forest;
        assert_eq!(vegetation_dirty(Some(&old), &corner, 5), vec![0, 1, 5, 6], "clipped at the border");
    }

    #[test]
    fn framing_fits_the_map_tightly_and_centred() {
        let fov = std::f32::consts::FRAC_PI_4;
        for (size, aspect, pitch, yaw, ortho) in [
            (Vec2::new(80.0, 48.0), 1.14, 0.96, 0.0, false),
            (Vec2::new(64.0, 64.0), 1.6, 0.96, 0.4, false),
            (Vec2::new(96.0, 96.0), 1.14, rendering::orbit::ISOMETRIC_PITCH, std::f32::consts::FRAC_PI_4, true),
            (Vec2::new(72.0, 56.0), 0.7, rendering::orbit::AERIAL_PITCH, 0.0, false),
        ] {
            let view = FramingView { yaw, pitch, fov, aspect, orthographic: ortho };
            let (focus, d) = frame_map(size, &view, FRAMING_MARGIN);
            let (lo, hi) = view.bounds(size, focus, d).expect("in front of the camera");
            let extent = lo.abs().max(hi.abs()).max_element();
            assert!(extent <= FRAMING_MARGIN + 1e-3, "{size} {aspect}: {extent}");
            assert!(extent > FRAMING_MARGIN - 0.03, "{size} {aspect}: not tight ({extent})");
            assert!((lo.y + hi.y).abs() < 0.08, "{size} {aspect}: off centre {lo} {hi}");
        }
        // A wider viewport needs less distance.
        let view = |aspect| FramingView { yaw: 0.0, pitch: 0.96, fov, aspect, orthographic: false };
        assert!(
            frame_map(Vec2::new(80.0, 48.0), &view(2.5), 0.9).1 < frame_map(Vec2::new(80.0, 48.0), &view(1.0), 0.9).1
        );
    }

    #[test]
    fn framing_keeps_the_map_clear_of_the_overlay_when_cheap() {
        let fov = std::f32::consts::FRAC_PI_4;
        let central = Rect::new(240.0, 60.0, 1410.0, 1088.0);
        let overlay = Rect::new(252.0, 72.0, 500.0, 330.0);
        let view =
            FramingView { yaw: 0.0, pitch: 0.96, fov, aspect: central.width() / central.height(), orthographic: false };
        let [below, right] = NdcWindow::beside_overlay(central, overlay);
        let below = below.expect("room below the overlay");
        assert!(below.max.y < 0.6 && below.min == Vec2::NEG_ONE);
        // A wide map is limited by the width: it moves below the overlay
        // (or beside it) at a small cost in size.
        let wide = Vec2::new(80.0, 48.0);
        let (_, full) = frame_map(wide, &view, FRAMING_MARGIN);
        let (focus, d) = frame_map_clear_of(wide, &view, FRAMING_MARGIN, Some((central, overlay)));
        assert!(d <= full * 1.25, "{d} vs {full}");
        let (lo, hi) = view.bounds(wide, focus, d).expect("in front");
        let right = right.expect("room beside the overlay");
        assert!(hi.y <= below.max.y + 1e-3 || lo.x >= right.min.x - 1e-3, "overlaps: {lo} {hi}");
        assert!(lo.cmpge(Vec2::splat(-1.0)).all() && hi.cmple(Vec2::ONE).all(), "inside the viewport");
        // Framed in the band below the overlay, the map is centred in it.
        let (focus, d) = frame_map_in(wide, &view, FRAMING_MARGIN, below);
        let (lo, hi) = view.bounds(wide, focus, d).expect("in front");
        assert!(hi.y <= below.max.y && (0.5 * (lo.y + hi.y) - below.centre().y).abs() < 0.08, "{lo} {hi}");
        // A tall map has room beside the overlay.
        let tall = Vec2::new(48.0, 80.0);
        let (focus, d) = frame_map_clear_of(tall, &view, FRAMING_MARGIN, Some((central, overlay)));
        let (lo, hi) = view.bounds(tall, focus, d).expect("in front");
        assert!(lo.x >= right.min.x - 1e-3 || hi.y <= below.max.y + 1e-3, "overlaps: {lo} {hi}");
        assert!(d <= frame_map(tall, &view, FRAMING_MARGIN).1 * 1.25);
        // Avoiding a large overlay would shrink the map too much: the whole
        // viewport is used and the overlap accepted.
        let large = Rect::new(252.0, 72.0, 1000.0, 700.0);
        let square = Vec2::new(64.0, 64.0);
        let full = frame_map(square, &view, FRAMING_MARGIN);
        assert_eq!(frame_map_clear_of(square, &view, FRAMING_MARGIN, Some((central, large))), full);
        assert_eq!(frame_map_clear_of(square, &view, FRAMING_MARGIN, None), full);
        // No band is offered when the overlay leaves too little room.
        assert_eq!(NdcWindow::beside_overlay(central, Rect::new(240.0, 60.0, 1300.0, 1000.0)), [None, None]);
    }

    #[test]
    fn projection_follows_the_orbit_convention() {
        let view = FramingView { yaw: 0.0, pitch: 0.9, fov: 0.8, aspect: 1.0, orthographic: false };
        let focus = Vec3::new(10.0, 0.0, 10.0);
        assert!(view.project(focus, 20.0, focus).unwrap().length() < 1e-4, "the focus is at the centre");
        // Yaw 0: north (−Z) is up on the screen, east (+X) to the right.
        assert!(view.project(focus, 20.0, focus + Vec3::NEG_Z * 3.0).unwrap().y > 0.0);
        assert!(view.project(focus, 20.0, focus + Vec3::X * 3.0).unwrap().x > 0.0);
        assert!(view.project(focus, 20.0, focus + Vec3::new(0.0, 0.0, 40.0)).is_none(), "behind the camera");
        let ortho = FramingView { orthographic: true, ..view };
        let p = ortho.project(focus, 20.0, focus + Vec3::X * 20.0 * (0.4f32).tan()).unwrap();
        assert!((p.x - 1.0).abs() < 1e-4, "orthographic half width = d·tan(fov/2)·aspect");
    }

    #[test]
    fn construction_raises_the_building() {
        let (start, scaffold) = progress_pose(BuildingKind::Hut, 0.0);
        let (done, finished_scaffold) = progress_pose(BuildingKind::Hut, 1.0);
        assert!(start.y < done.y && scaffold && !finished_scaffold);
        assert_eq!(done, Vec3::ONE);
        assert_eq!(progress_pose(BuildingKind::Storage, 2.0).0, Vec3::ONE, "clamped");
        assert!(!progress_pose(BuildingKind::FarmPlot, 0.7).1, "farm plots need no scaffold late on");
    }
}
