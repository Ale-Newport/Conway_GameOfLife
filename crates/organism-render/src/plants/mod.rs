//! Plants: procedural trees with growth stages, instanced ground cover,
//! manifest GLB plants, wind sway, seasonal tint and burnt state.
//!
//! The view sets [`PlantSeason`] from the simulation calendar and [`Wind`]
//! from its weather; plant entities carry [`ProceduralTree`],
//! [`GroundCoverPlant`] or [`GlbPlant`].

pub mod glb_plants;
pub mod ground;
pub mod tree;

use std::collections::BTreeMap;

use bevy::color::ColorToComponents;
use bevy::prelude::*;

use crate::procedural::parts::Detail;
pub use glb_plants::{GlbPlant, spawn_glb_plant};
pub use ground::{FlowerColor, GroundCover};
pub use tree::TreeStyle;

/// Position in the plant year: 0 = start of spring, 0.25 summer, 0.5
/// autumn, 0.75 winter (temperate phenology; savanna styles read the same
/// cycle as wet → dry season).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PlantSeason {
    /// Fraction of the year in `[0, 1)`.
    pub year_fraction: f32,
}

impl Default for PlantSeason {
    fn default() -> Self {
        Self { year_fraction: 0.35 }
    }
}

fn lerp_srgb(stops: &[(f32, u32)], x: f32) -> Color {
    let to = |c: u32| Vec3::new(((c >> 16) & 0xff) as f32, ((c >> 8) & 0xff) as f32, (c & 0xff) as f32) / 255.0;
    let x = x.rem_euclid(1.0);
    let i = stops.iter().rposition(|(t, _)| *t <= x).unwrap_or(0);
    let (t0, c0) = stops[i];
    let (t1, c1) = stops.get(i + 1).copied().unwrap_or((1.0 + stops[0].0, stops[0].1));
    let f = ((x - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
    let c = to(c0).lerp(to(c1), f);
    Color::srgb(c.x, c.y, c.z)
}

impl PlantSeason {
    /// Season from a fraction of the year.
    pub fn new(year_fraction: f32) -> Self {
        Self { year_fraction: year_fraction.rem_euclid(1.0) }
    }

    /// Leaf colour of deciduous plants: fresh green in spring, deep green in
    /// summer, yellow → orange → red-brown in autumn.
    pub fn deciduous_leaf(&self) -> Color {
        lerp_srgb(
            &[
                (0.0, 0x8cc850),
                (0.2, 0x5a9a38),
                (0.5, 0x5a9032),
                (0.58, 0xd0b83a),
                (0.66, 0xd8701e),
                (0.74, 0x8a3a1a),
                (0.8, 0x6a4a2a),
            ],
            self.year_fraction,
        )
    }

    /// Needle / palm colour (slightly darker in winter).
    pub fn evergreen_leaf(&self) -> Color {
        lerp_srgb(&[(0.0, 0x3a7034), (0.4, 0x2e6430), (0.75, 0x2a4a2c), (0.95, 0x30562e)], self.year_fraction)
    }

    /// Grass colour: green, straw in late summer/autumn, dull in winter.
    pub fn grass(&self) -> Color {
        lerp_srgb(
            &[(0.0, 0x78b048), (0.3, 0x6a9a3a), (0.5, 0xb8a050), (0.75, 0x8a8a5a), (0.95, 0x6a9040)],
            self.year_fraction,
        )
    }

    /// Fraction of the canopy carrying leaves for deciduous plants: leaf
    /// fall through autumn, bare winter, leaf-out in early spring.
    pub fn leaf_cover(&self) -> f32 {
        let x = self.year_fraction;
        if x < 0.12 {
            x / 0.12
        } else if x < 0.6 {
            1.0
        } else if x < 0.76 {
            1.0 - (x - 0.6) / 0.16
        } else {
            0.0
        }
    }
}

/// Wind seen by plants: horizontal direction (x, z) and speed (m/s).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Wind {
    /// Direction the wind blows towards (normalised internally).
    pub direction: Vec2,
    /// Speed (m/s).
    pub speed: f32,
}

impl Default for Wind {
    fn default() -> Self {
        Self { direction: Vec2::X, speed: 4.0 }
    }
}

/// Sways an entity about its base in the wind. Bending grows with the
/// dynamic pressure (∝ speed²) and is limited for stiff plants; the
/// oscillation runs at the plant's natural frequency with a per-instance
/// phase. Purely visual.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct WindSway {
    /// Natural frequency (Hz).
    pub frequency: f32,
    /// Bend (rad) at 10 m/s.
    pub amplitude: f32,
    /// Phase offset (rad).
    pub phase: f32,
    /// Rotation without wind.
    pub rest: Quat,
}

/// Bend angle (rad) of a plant at wind `speed` and time `t`.
pub fn sway_angle(sway: &WindSway, speed: f32, t: f32) -> f32 {
    let pressure = (speed / 10.0).powi(2).min(2.5);
    let gust = 0.65
        + 0.35 * (std::f32::consts::TAU * sway.frequency * t + sway.phase).sin()
        + 0.1 * (std::f32::consts::TAU * 0.13 * t + 1.7 * sway.phase).sin();
    sway.amplitude * pressure * gust
}

fn apply_wind(time: Res<Time>, wind: Res<Wind>, mut q: Query<(&WindSway, &mut Transform)>) {
    let dir = Vec3::new(wind.direction.x, 0.0, wind.direction.y).normalize_or(Vec3::X);
    let axis = Vec3::Y.cross(dir).normalize_or(Vec3::Z);
    let t = time.elapsed_secs();
    let speed = wind.speed;
    q.par_iter_mut().for_each(|(sway, mut transform)| {
        let rotation = Quat::from_axis_angle(axis, -sway_angle(sway, speed, t)) * sway.rest;
        // In calm air nothing changes, so no transform propagation is triggered.
        if transform.rotation != rotation {
            transform.rotation = rotation;
        }
    });
}

/// Shared plant materials (vertex colours × seasonal base colour).
#[derive(Resource, Clone, Debug)]
pub struct PlantMaterials {
    /// Bark.
    pub wood: Handle<StandardMaterial>,
    /// Charred bark.
    pub charred: Handle<StandardMaterial>,
    /// Deciduous foliage (seasonal colour).
    pub deciduous: Handle<StandardMaterial>,
    /// Evergreen foliage.
    pub evergreen: Handle<StandardMaterial>,
    /// Grass and shrubs (seasonal colour).
    pub grass: Handle<StandardMaterial>,
    /// Flowers (own colours).
    pub flower: Handle<StandardMaterial>,
    /// Burnt ground cover.
    pub burnt: Handle<StandardMaterial>,
}

impl FromWorld for PlantMaterials {
    fn from_world(world: &mut World) -> Self {
        let mut m = world.resource_mut::<Assets<StandardMaterial>>();
        let mat = |c: Color, r: f32| StandardMaterial {
            base_color: c,
            perceptual_roughness: r,
            double_sided: true,
            cull_mode: None,
            ..default()
        };
        Self {
            wood: m.add(mat(Color::WHITE, 0.9)),
            charred: m.add(mat(Color::srgb(0.12, 0.1, 0.09), 0.95)),
            deciduous: m.add(mat(PlantSeason::default().deciduous_leaf(), 0.75)),
            evergreen: m.add(mat(PlantSeason::default().evergreen_leaf(), 0.75)),
            grass: m.add(mat(PlantSeason::default().grass(), 0.8)),
            flower: m.add(mat(Color::WHITE, 0.6)),
            burnt: m.add(mat(Color::srgb(0.09, 0.08, 0.07), 0.95)),
        }
    }
}

/// A procedural tree. Changing `growth` regrows it (meshes are cached per
/// 1/16 growth stage); `burnt` chars the bark and drops the foliage.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(Transform, Visibility)]
pub struct ProceduralTree {
    /// Architecture.
    pub style: TreeStyle,
    /// Growth stage 0 (sapling) .. 1 (mature).
    pub growth: f32,
    /// Variation seed (8 distinct shapes per style).
    pub seed: u64,
    /// Burnt by fire.
    pub burnt: bool,
}

/// Number of cached growth stages.
pub const GROWTH_STAGES: u8 = 16;
/// Number of distinct shapes per style.
pub const TREE_VARIANTS: u64 = 8;

/// A ground-cover plant (instanced).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(Transform, Visibility)]
pub struct GroundCoverPlant {
    /// Kind.
    pub kind: GroundCover,
    /// Mesh variant (0..3).
    pub variant: u8,
    /// Burnt by fire (drawn charred).
    pub burnt: bool,
}

/// Foliage entity of a tree (seasonal visibility).
#[derive(Component, Clone, Copy, Debug)]
pub struct TreeFoliage {
    /// Sheds leaves.
    pub deciduous: bool,
    /// Owning tree is burnt.
    pub burnt: bool,
}

/// Number of discrete leaf-cover levels a deciduous crown is drawn at
/// (level `k` keeps `k / LEAF_COVER_LEVELS` of its leaf clusters).
pub const LEAF_COVER_LEVELS: u8 = 4;

/// Leaf-cover level (0 = bare .. [`LEAF_COVER_LEVELS`] = full) for a cover
/// fraction; any remaining leaves keep at least the first level.
pub fn leaf_cover_level(cover: f32) -> u8 {
    if cover.is_nan() || cover <= 0.0 {
        0
    } else {
        ((cover.min(1.0) * LEAF_COVER_LEVELS as f32).ceil() as u8).clamp(1, LEAF_COVER_LEVELS)
    }
}

/// Which cached crown a foliage entity shows.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct FoliageSource {
    key: TreeKey,
    level: u8,
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct TreeBuilt {
    key: TreeKey,
    burnt: bool,
}

/// Cache key of a tree mesh: style, shape variant, growth stage.
type TreeKey = (TreeStyle, u64, u8);
/// Wood mesh and (optional) foliage mesh of a cached tree.
type TreeMeshHandles = (Handle<Mesh>, Option<Handle<Mesh>>);
/// Trees whose parameters changed since the last frame.
type ChangedTrees<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static ProceduralTree, Option<&'static TreeBuilt>, Option<&'static Children>),
    Changed<ProceduralTree>,
>;

/// Cached plant meshes.
#[derive(Resource, Default)]
pub struct PlantMeshCache {
    trees: BTreeMap<TreeKey, TreeMeshHandles>,
    thinned: BTreeMap<(TreeKey, u8), Option<Handle<Mesh>>>,
    cover: BTreeMap<(GroundCover, u8), Handle<Mesh>>,
}

impl PlantMeshCache {
    fn tree(&mut self, key: TreeKey, meshes: &mut Assets<Mesh>) -> TreeMeshHandles {
        self.trees
            .entry(key)
            .or_insert_with(|| {
                let t = tree::generate_tree(key.0, key.2 as f32 / GROWTH_STAGES as f32, key.1, Detail::Full);
                (meshes.add(t.wood.to_mesh()), (!t.foliage.is_empty()).then(|| meshes.add(t.foliage.to_mesh())))
            })
            .clone()
    }

    /// Crown of a cached tree at a leaf-cover level (`None` when bare).
    fn foliage(&mut self, key: TreeKey, level: u8, meshes: &mut Assets<Mesh>) -> Option<Handle<Mesh>> {
        if level >= LEAF_COVER_LEVELS {
            return self.tree(key, meshes).1;
        }
        if level == 0 {
            return None;
        }
        self.thinned
            .entry((key, level))
            .or_insert_with(|| {
                let cover = level as f32 / LEAF_COVER_LEVELS as f32;
                let growth = key.2 as f32 / GROWTH_STAGES as f32;
                let t = tree::generate_tree_with_leaf_cover(key.0, growth, key.1, Detail::Full, cover);
                (!t.foliage.is_empty()).then(|| meshes.add(t.foliage.to_mesh()))
            })
            .clone()
    }

    /// Mesh of a ground-cover kind and variant.
    pub fn cover(&mut self, kind: GroundCover, variant: u8, meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
        self.cover
            .entry((kind, variant % 4))
            .or_insert_with(|| meshes.add(ground::ground_cover_mesh(kind, 0x51ed + variant as u64).to_mesh()))
            .clone()
    }
}

/// Leaf-cover level a tree's crown shows in `season`.
fn crown_level(style: TreeStyle, season: &PlantSeason) -> u8 {
    if style.deciduous() { leaf_cover_level(season.leaf_cover()) } else { LEAF_COVER_LEVELS }
}

fn sync_trees(
    mut commands: Commands,
    mut cache: ResMut<PlantMeshCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: Res<PlantMaterials>,
    season: Res<PlantSeason>,
    trees: ChangedTrees,
) {
    for (entity, tree, built, children) in &trees {
        let stage = (tree.growth.clamp(0.0, 1.0) * GROWTH_STAGES as f32).round().max(1.0) as u8;
        let key = (tree.style, tree.seed % TREE_VARIANTS, stage);
        if built.is_some_and(|b| b.key == key && b.burnt == tree.burnt) {
            continue;
        }
        if let Some(children) = children {
            for c in children.iter() {
                commands.entity(c).despawn();
            }
        }
        let (wood, full_crown) = cache.tree(key, &mut meshes);
        let bark = if tree.burnt { materials.charred.clone() } else { materials.wood.clone() };
        let phase = (tree.seed % 1000) as f32 * 0.37;
        // The crown is a child of the wood, so trunk, branches and leaves
        // sway together about the tree base.
        let wood = commands
            .spawn((
                Mesh3d(wood),
                MeshMaterial3d(bark),
                Transform::IDENTITY,
                WindSway { frequency: 0.25, amplitude: 0.015, phase, rest: Quat::IDENTITY },
                ChildOf(entity),
            ))
            .id();
        if full_crown.is_some() {
            let deciduous = tree.style.deciduous();
            let level = crown_level(tree.style, &season);
            let material = if deciduous { materials.deciduous.clone() } else { materials.evergreen.clone() };
            let crown = cache.foliage(key, level, &mut meshes);
            let visible = !tree.burnt && crown.is_some();
            let mut foliage = commands.spawn((
                MeshMaterial3d(material),
                Transform::IDENTITY,
                if visible { Visibility::Inherited } else { Visibility::Hidden },
                TreeFoliage { deciduous, burnt: tree.burnt },
                FoliageSource { key, level },
                ChildOf(wood),
            ));
            if let Some(mesh) = crown {
                foliage.insert(Mesh3d(mesh));
            }
        }
        commands.entity(entity).insert(TreeBuilt { key, burnt: tree.burnt });
    }
}

fn sync_ground_cover(
    mut commands: Commands,
    mut cache: ResMut<PlantMeshCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: Res<PlantMaterials>,
    plants: Query<(Entity, &GroundCoverPlant, &Transform, Option<&WindSway>), Changed<GroundCoverPlant>>,
) {
    for (entity, plant, transform, sway) in &plants {
        let mesh = cache.cover(plant.kind, plant.variant, &mut meshes);
        let material = if plant.burnt {
            materials.burnt.clone()
        } else if plant.kind.seasonal() {
            materials.grass.clone()
        } else {
            materials.flower.clone()
        };
        let phase = transform.translation.x * 1.7 + transform.translation.z * 2.3;
        // The wind system owns the rotation once swaying: keep the original
        // rest orientation when the plant is updated (e.g. burnt) later.
        let rest = sway.map_or(transform.rotation, |s| s.rest);
        commands.entity(entity).insert((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            WindSway {
                frequency: plant.kind.sway_frequency(),
                amplitude: if plant.kind == GroundCover::Shrub { 0.04 } else { 0.18 },
                phase,
                rest,
            },
        ));
    }
}

/// Spawns `positions.len()` instanced ground-cover plants as children of
/// `parent`, with seeded yaw, size and mesh-variant variation.
pub fn spawn_ground_cover(commands: &mut Commands, parent: Entity, kind: GroundCover, positions: &[Vec3], seed: u64) {
    let mut rng = simulation_core::SimRng::seed_from_u64(seed);
    for p in positions {
        let yaw = rng.range_f32(0.0, std::f32::consts::TAU);
        let scale = rng.range_f32(0.75, 1.25);
        let variant = rng.index(4) as u8;
        commands.spawn((
            GroundCoverPlant { kind, variant, burnt: false },
            Transform::from_translation(*p).with_rotation(Quat::from_rotation_y(yaw)).with_scale(Vec3::splat(scale)),
            ChildOf(parent),
        ));
    }
}

/// Seasonal tint of the shared foliage materials and leaf fall: deciduous
/// crowns switch to a thinned mesh (fewer leaf clusters, see
/// [`tree::generate_tree_with_leaf_cover`]) and are hidden when bare.
fn apply_season(
    mut commands: Commands,
    season: Res<PlantSeason>,
    materials: Res<PlantMaterials>,
    mut assets: ResMut<Assets<StandardMaterial>>,
    mut cache: ResMut<PlantMeshCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut foliage: Query<(Entity, &TreeFoliage, &mut FoliageSource, &mut Visibility)>,
) {
    if !season.is_changed() {
        return;
    }
    let set = |assets: &mut Assets<StandardMaterial>, h: &Handle<StandardMaterial>, c: Color| {
        if let Some(mut m) = assets.get_mut(h) {
            m.base_color = c;
        }
    };
    set(&mut assets, &materials.deciduous, season.deciduous_leaf());
    set(&mut assets, &materials.evergreen, season.evergreen_leaf());
    set(&mut assets, &materials.grass, season.grass());
    for (entity, f, mut source, mut vis) in &mut foliage {
        if f.burnt || !f.deciduous {
            continue;
        }
        let level = crown_level(source.key.0, &season);
        if level == source.level {
            continue;
        }
        source.level = level;
        match cache.foliage(source.key, level, &mut meshes) {
            Some(mesh) => {
                commands.entity(entity).insert(Mesh3d(mesh));
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

/// Converts a colour to its linear RGB vector (used for tint ratios).
pub(crate) fn linear_rgb(c: Color) -> Vec3 {
    Vec3::from_slice(&c.to_linear().to_f32_array()[..3])
}

/// Registers plant resources and systems.
pub(crate) fn build(app: &mut App) {
    app.init_resource::<PlantSeason>()
        .init_resource::<Wind>()
        .init_resource::<PlantMaterials>()
        .init_resource::<PlantMeshCache>()
        .init_resource::<glb_plants::GlbPlantMaterials>()
        .add_systems(
            Update,
            (
                sync_trees,
                sync_ground_cover,
                glb_plants::setup_glb_plants,
                apply_season,
                glb_plants::apply_glb_plant_season,
                apply_wind,
            )
                .chain()
                .in_set(crate::OrganismSystems::Animate),
        )
        .add_observer(glb_plants::on_plant_scene_ready);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasons_cycle_colours_and_leaf_cover() {
        let summer = PlantSeason::new(0.35);
        let autumn = PlantSeason::new(0.66);
        let winter = PlantSeason::new(0.85);
        assert_eq!(summer.leaf_cover(), 1.0);
        assert!(autumn.leaf_cover() > 0.0 && autumn.leaf_cover() < 1.0);
        assert_eq!(winter.leaf_cover(), 0.0);
        let s = linear_rgb(summer.deciduous_leaf());
        let a = linear_rgb(autumn.deciduous_leaf());
        assert!(s.y > s.x, "summer is green");
        assert!(a.x > a.y, "autumn is orange/red");
        assert!((PlantSeason::new(1.0).year_fraction - 0.0).abs() < 1e-6);
    }

    #[test]
    fn leaf_cover_levels_cover_the_year() {
        assert_eq!(leaf_cover_level(1.0), LEAF_COVER_LEVELS);
        assert_eq!(leaf_cover_level(0.0), 0);
        assert_eq!(leaf_cover_level(0.01), 1, "a few leaves left are still drawn");
        assert_eq!(leaf_cover_level(0.5), LEAF_COVER_LEVELS / 2);
        assert_eq!(leaf_cover_level(f32::NAN), 0);
        let levels: Vec<u8> =
            (0..100).map(|i| leaf_cover_level(PlantSeason::new(i as f32 / 100.0).leaf_cover())).collect();
        assert!(levels.contains(&0) && levels.contains(&LEAF_COVER_LEVELS) && levels.contains(&2));
    }

    fn plant_app(season: f32) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(PlantSeason::new(season))
            .init_resource::<Wind>()
            .init_resource::<PlantMaterials>()
            .init_resource::<PlantMeshCache>()
            .add_systems(Update, (sync_trees, sync_ground_cover, apply_season, apply_wind).chain());
        app
    }

    fn crown(app: &mut App) -> (Visibility, u8, Option<Entity>) {
        let mut q = app.world_mut().query::<(&Visibility, &FoliageSource, &ChildOf)>();
        let (v, s, parent) = q.single(app.world()).unwrap();
        (*v, s.level, Some(parent.parent()))
    }

    #[test]
    fn trees_planted_in_winter_are_bare_and_leaf_out_in_spring() {
        let mut app = plant_app(0.85);
        let tree = app
            .world_mut()
            .spawn(ProceduralTree { style: TreeStyle::Broadleaf, growth: 1.0, seed: 1, burnt: false })
            .id();
        app.update();
        let (vis, level, parent) = crown(&mut app);
        assert_eq!((vis, level), (Visibility::Hidden, 0), "no leaves in winter");
        // The crown hangs from the swaying wood, not from the tree root.
        assert_ne!(parent, Some(tree));
        assert!(app.world().get::<WindSway>(parent.unwrap()).is_some());
        app.world_mut().insert_resource(PlantSeason::new(0.06));
        app.update();
        let (vis, level, _) = crown(&mut app);
        assert_eq!(vis, Visibility::Inherited);
        assert_eq!(level, leaf_cover_level(PlantSeason::new(0.06).leaf_cover()));
        assert!(level > 0 && level < LEAF_COVER_LEVELS, "leaf-out is gradual: {level}");
        app.world_mut().insert_resource(PlantSeason::new(0.35));
        app.update();
        assert_eq!(crown(&mut app).1, LEAF_COVER_LEVELS);
    }

    #[test]
    fn ground_cover_keeps_its_rest_orientation_when_updated() {
        let mut app = plant_app(0.35);
        app.insert_resource(Wind { direction: Vec2::X, speed: 15.0 });
        let rest = Quat::from_rotation_y(1.0);
        let plant = app
            .world_mut()
            .spawn((
                GroundCoverPlant { kind: GroundCover::TallGrass, variant: 0, burnt: false },
                Transform::from_rotation(rest),
            ))
            .id();
        for _ in 0..3 {
            app.update();
        }
        app.world_mut().get_mut::<GroundCoverPlant>(plant).unwrap().burnt = true;
        app.update();
        let sway = app.world().get::<WindSway>(plant).unwrap();
        assert!(sway.rest.angle_between(rest) < 1e-5, "rest drifted to the swayed pose");
    }

    #[test]
    fn sway_grows_with_wind_pressure() {
        let s = WindSway { frequency: 1.0, amplitude: 0.2, phase: 0.0, rest: Quat::IDENTITY };
        let calm = (0..100).map(|i| sway_angle(&s, 2.0, i as f32 * 0.05)).fold(0.0f32, f32::max);
        let storm = (0..100).map(|i| sway_angle(&s, 20.0, i as f32 * 0.05)).fold(0.0f32, f32::max);
        assert!(storm > 10.0 * calm);
        assert_eq!(sway_angle(&s, 0.0, 1.0), 0.0);
    }
}
