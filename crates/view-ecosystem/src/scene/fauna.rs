//! Animals and small life.
//!
//! Every individual the model publishes is an entity with an
//! `organism_render::AnimatedOrganism`: its species' authored GLB model or
//! procedural rig, the animation chosen from its action and motion, its
//! physical speed (for strides and wing beats) and its size. The renderer
//! picks the level of detail (full rig, reduced update rate, static low
//! mesh; never its marker discs: [`no_lod_markers`]) and batches identical
//! meshes. Positions arrive with each
//! publication and are interpolated in between (display only).
//!
//! Carcasses are the dead species lying in their death pose; clutches are
//! nests with the model's egg count; density-field taxa (insects, spiders,
//! worms, forage fish) are drawn from the model's representative markers
//! around the camera; ant and bee colonies as nests and hives with their
//! forager squads.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use ecosystem::Sex;
use ecosystem::species::Locomotion;
use organism_render::{AnimatedOrganism, LodLevel, OrganismAction, OrganismCatalog, OrganismLod};

use super::EcoCamera;
use super::terrain::MapState;
use crate::MODE;
use crate::coords::{heading_to_yaw, lerp_angle};
use crate::mapping::{
    MotionContext, age_scale, animal_height, cruise_ratio, is_swimming, male_render_id, organism_action, render_id,
    speed_m_s,
};
use crate::state::{EcoData, EcoUi};

/// Height above which a flier counts as airborne (the model's threshold).
pub const AIRBORNE_M: f32 = 1.0;
/// Moves longer than this (m) between two frames are jumps, not glides.
pub const SNAP_DISTANCE: f32 = 400.0;
/// Forager squads farther than this from the camera focus are not drawn (m).
pub const FORAGER_RADIUS: f32 = 160.0;
/// Small-life slots moving farther than this (m) between frames snap.
pub const MARKER_SNAP_DISTANCE: f32 = 15.0;
/// Small life (field markers, forager squads) is drawn this many times its
/// size: each marker stands for many individuals.
pub const SMALL_LIFE_SCALE: f32 = 4.0;

/// An animal of the model.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcoAnimal {
    /// Model id.
    pub id: u64,
    /// Species index.
    pub species: u16,
}

/// A carcass of the model.
#[derive(Component, Debug, Clone, Copy)]
pub struct EcoCarcass {
    /// Carcass id.
    pub id: u64,
}

/// A clutch of eggs.
#[derive(Component, Debug, Clone, Copy)]
pub struct EcoClutch {
    /// Clutch id.
    pub id: u64,
}

/// A density-field marker (slot in the frame's marker list).
#[derive(Component, Debug, Clone, Copy)]
pub struct FieldMarkerSlot(pub usize);

/// An ant nest or bee hive.
#[derive(Component, Debug, Clone, Copy)]
pub struct EcoColony {
    /// Colony id.
    pub id: u64,
}

/// A forager squad (slot in the frame's list).
#[derive(Component, Debug, Clone, Copy)]
pub struct ForagerSlot(pub usize);

/// Interpolated motion between published positions (display only).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    /// Where the glide started.
    pub from: Vec3,
    /// Latest published position.
    pub to: Vec3,
    /// Yaw at the start.
    pub yaw_from: f32,
    /// Target yaw.
    pub yaw_to: f32,
    /// Real time the target arrived (s).
    pub start: f64,
    /// Expected time to the next publication (s).
    pub duration: f32,
}

impl Glide {
    /// At rest.
    pub fn at(p: Vec3, yaw: f32, now: f64) -> Self {
        Self { from: p, to: p, yaw_from: yaw, yaw_to: yaw, start: now, duration: 0.1 }
    }

    /// Progress in `[0, 1]`.
    pub fn progress(&self, now: f64) -> f32 {
        if self.duration <= 0.0 { 1.0 } else { (((now - self.start) as f32) / self.duration).clamp(0.0, 1.0) }
    }

    /// Position and yaw at `now`.
    pub fn sample(&self, now: f64) -> (Vec3, f32) {
        let t = self.progress(now);
        (self.from.lerp(self.to, t), lerp_angle(self.yaw_from, self.yaw_to, t))
    }

    /// Glides from the current pose to a new target (snaps on jumps).
    pub fn retarget(&mut self, to: Vec3, yaw: f32, now: f64, duration: f32) {
        let (here, here_yaw) = self.sample(now);
        if here.distance(to) > SNAP_DISTANCE {
            *self = Self::at(to, yaw, now);
            return;
        }
        *self = Self { from: here, to, yaw_from: here_yaw, yaw_to: yaw, start: now, duration };
    }
}

/// Id → entity of drawn animals, and the render id of every species.
#[derive(Resource, Default)]
pub struct AnimalIndex {
    map: BTreeMap<u64, Entity>,
    carcasses: BTreeMap<u64, Entity>,
    clutches: BTreeMap<u64, Entity>,
    colonies: BTreeMap<u64, Entity>,
    markers: Vec<Entity>,
    epoch: Option<u64>,
    seq: u64,
    layers: Option<crate::state::Layers>,
    /// `organism-render` id per species index.
    pub render_ids: Vec<String>,
    /// Distinct visual of adult males per species index, if any.
    pub male_ids: Vec<Option<String>>,
}

impl AnimalIndex {
    /// Entity of an animal.
    pub fn entity(&self, id: u64) -> Option<Entity> {
        self.map.get(&id).copied()
    }
}

/// Shared meshes of nests, hives and clutches.
#[derive(Resource)]
pub struct NestAssets {
    ant_mound: Handle<Mesh>,
    hive: Handle<Mesh>,
    nest: Handle<Mesh>,
    egg: Handle<Mesh>,
    soil: Handle<StandardMaterial>,
    straw: Handle<StandardMaterial>,
    twigs: Handle<StandardMaterial>,
    shell: Handle<StandardMaterial>,
}

impl FromWorld for NestAssets {
    fn from_world(world: &mut World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        let ant_mound = meshes.add(Cone { radius: 0.7, height: 0.45 }.mesh().resolution(20));
        let mut hive =
            Torus { minor_radius: 0.09, major_radius: 0.28 }.mesh().minor_resolution(8).major_resolution(20).build();
        for (k, (major, y)) in [(0.25f32, 0.18f32), (0.2, 0.33), (0.13, 0.46), (0.05, 0.56)].into_iter().enumerate() {
            let ring = Torus { minor_radius: 0.09 - 0.012 * k as f32, major_radius: major }
                .mesh()
                .minor_resolution(8)
                .major_resolution(20)
                .build()
                .transformed_by(Transform::from_xyz(0.0, y, 0.0));
            let _ = hive.merge(&ring);
        }
        hive.transform_by(Transform::from_xyz(0.0, 0.09, 0.0));
        let hive = meshes.add(hive);
        let nest = meshes.add(
            Torus { minor_radius: 0.05, major_radius: 0.14 }
                .mesh()
                .minor_resolution(8)
                .major_resolution(16)
                .build()
                .transformed_by(Transform::from_xyz(0.0, 0.04, 0.0)),
        );
        let egg = meshes.add(Sphere::new(0.025).mesh().uv(10, 6));
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        let mat = |m: &mut Assets<StandardMaterial>, c: Color, r: f32| {
            m.add(StandardMaterial { base_color: c, perceptual_roughness: r, ..default() })
        };
        Self {
            ant_mound,
            hive,
            nest,
            egg,
            soil: mat(&mut materials, Color::srgb(0.42, 0.31, 0.2), 1.0),
            straw: mat(&mut materials, Color::srgb(0.78, 0.64, 0.36), 0.9),
            twigs: mat(&mut materials, Color::srgb(0.38, 0.28, 0.18), 1.0),
            shell: mat(&mut materials, Color::srgb(0.93, 0.9, 0.82), 0.5),
        }
    }
}

/// Spawns, updates and removes animals, carcasses, clutches, colonies,
/// field markers and forager squads from new frames.
#[allow(clippy::too_many_arguments)]
pub fn sync_fauna(
    mut commands: Commands,
    data: Res<EcoData>,
    map: Res<MapState>,
    ui: Res<EcoUi>,
    catalog: Res<OrganismCatalog>,
    nests: Res<NestAssets>,
    time: Res<Time<Real>>,
    mut index: ResMut<AnimalIndex>,
    camera: Option<Single<&bevy_panorbit_camera::PanOrbitCamera, With<EcoCamera>>>,
    mut animals: Query<(&mut AnimatedOrganism, &mut Glide, &mut Visibility), Without<FieldMarkerSlot>>,
    mut carcasses: Query<&mut Transform, With<EcoCarcass>>,
) {
    let (Some(frame), Some(geometry)) = (data.get(), map.geometry.as_ref()) else { return };
    if map.epoch != Some(frame.epoch) {
        return;
    }
    if index.epoch != Some(frame.epoch) {
        let old: Vec<Entity> = index
            .map
            .values()
            .chain(index.carcasses.values())
            .chain(index.clutches.values())
            .chain(index.colonies.values())
            .chain(index.markers.iter())
            .copied()
            .collect();
        for e in old {
            commands.entity(e).despawn();
        }
        let render_ids: Vec<String> = frame.info.catalog.animals().iter().map(|s| render_id(s, &catalog)).collect();
        let male_ids = render_ids.iter().map(|id| male_render_id(id, &catalog)).collect();
        *index = AnimalIndex { epoch: Some(frame.epoch), render_ids, male_ids, ..AnimalIndex::default() };
    }
    if index.seq == data.seq {
        return;
    }
    index.seq = data.seq;
    let now = time.elapsed_secs_f64();
    let duration = data.interval.clamp(0.03, 0.6);
    let view = &frame.view;
    let species = frame.info.catalog.animals();
    let traits = &frame.info.traits;
    let attacking: BTreeSet<u64> = frame.recent_kills.iter().filter(|k| k.2 + 1 >= view.tick).map(|k| k.0).collect();
    let shown = if ui.layers.animals { Visibility::Inherited } else { Visibility::Hidden };
    // Individual animals.
    let mut alive = BTreeSet::new();
    for a in &view.animals {
        let s = usize::from(a.species);
        let (Some(sp), Some(tr)) = (species.get(s), traits.get(s)) else { continue };
        alive.insert(a.id);
        let p = geometry.cell_to_plane(a.position);
        let (ground, surface) = (geometry.ground(p.x, p.y), geometry.surface(p.x, p.y));
        let swimming = is_swimming(sp, surface - ground);
        let airborne = a.altitude_m > AIRBORNE_M;
        let position = Vec3::new(p.x, animal_height(a.altitude_m, ground, surface, swimming), p.y);
        let v = a.velocity.length();
        let ctx = MotionContext {
            cruise_ratio: cruise_ratio(v, tr),
            airborne,
            swimming,
            attacking: attacking.contains(&a.id),
        };
        // Adult males of dimorphic species have their own model.
        let male = (a.sex == Sex::Male && !a.juvenile).then(|| index.male_ids[s].as_ref()).flatten();
        let organism = AnimatedOrganism {
            species: male.unwrap_or(&index.render_ids[s]).clone(),
            action: organism_action(a.action, ctx),
            speed: speed_m_s(v, tr),
            age_scale: age_scale(a, sp),
        };
        let yaw = heading_to_yaw(a.heading);
        let visibility = if a.hidden { Visibility::Hidden } else { shown };
        match index.map.get(&a.id).copied() {
            Some(e) => {
                if let Ok((mut org, mut glide, mut vis)) = animals.get_mut(e) {
                    if *org != organism {
                        *org = organism;
                    }
                    glide.retarget(position, yaw, now, duration);
                    vis.set_if_neq(visibility);
                }
            }
            None => {
                let e = commands
                    .spawn((
                        Name::new(format!("{} #{}", sp.name, a.id)),
                        EcoAnimal { id: a.id, species: a.species },
                        organism,
                        Glide::at(position, yaw, now),
                        Transform::from_translation(position).with_rotation(Quat::from_rotation_y(yaw)),
                        visibility,
                        DespawnOnExit(MODE),
                    ))
                    .id();
                index.map.insert(a.id, e);
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
    // Carcasses: the species lying in its death pose, sinking into the
    // ground as it is eaten and decays (the model's remaining fraction).
    let mut present = BTreeSet::new();
    for c in &view.carcasses {
        let s = usize::from(c.species);
        let Some(sp) = species.get(s) else { continue };
        present.insert(c.id);
        let extent = catalog.extent(&index.render_ids[s]) * sp.visual.scale;
        let p = geometry.ground_point(c.position) - Vec3::Y * carcass_sink(extent, c.remaining);
        if let Some(&e) = index.carcasses.get(&c.id) {
            if let Ok(mut tf) = carcasses.get_mut(e)
                && tf.translation.distance(p) > 0.01
            {
                tf.translation = p;
            }
            continue;
        }
        let e = commands
            .spawn((
                Name::new(format!("{} carcass", sp.name)),
                EcoCarcass { id: c.id },
                AnimatedOrganism::new(index.render_ids[s].clone())
                    .with_action(OrganismAction::Death)
                    .with_age_scale(sp.visual.scale),
                Transform::from_translation(p).with_rotation(Quat::from_rotation_y(c.id as f32 * 2.399)),
                shown,
                DespawnOnExit(MODE),
            ))
            .id();
        index.carcasses.insert(c.id, e);
    }
    index.carcasses.retain(|id, e| {
        let keep = present.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    // Clutches: a nest with the model's egg count.
    present.clear();
    for c in &view.clutches {
        present.insert(c.id);
        if index.clutches.contains_key(&c.id) {
            continue;
        }
        let p = geometry.ground_point(c.position);
        let eggs = usize::from(c.eggs.min(8));
        let e = commands
            .spawn((
                Name::new("Clutch"),
                EcoClutch { id: c.id },
                Mesh3d(nests.nest.clone()),
                MeshMaterial3d(nests.twigs.clone()),
                Transform::from_translation(p).with_scale(Vec3::splat(1.6)),
                shown,
                DespawnOnExit(MODE),
            ))
            .with_children(|n| {
                for k in 0..eggs {
                    let a = k as f32 / eggs.max(1) as f32 * std::f32::consts::TAU;
                    let r = if eggs == 1 { 0.0 } else { 0.05 };
                    n.spawn((
                        Mesh3d(nests.egg.clone()),
                        MeshMaterial3d(nests.shell.clone()),
                        Transform::from_xyz(r * a.cos(), 0.05, r * a.sin()).with_scale(Vec3::new(1.0, 1.3, 1.0)),
                    ));
                }
            })
            .id();
        index.clutches.insert(c.id, e);
    }
    index.clutches.retain(|id, e| {
        let keep = present.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    // Colonies: ant mounds and bee hives sized by their workers.
    let small = if ui.layers.small_life { Visibility::Inherited } else { Visibility::Hidden };
    present.clear();
    for c in &view.colonies {
        let Some(sp) = species.get(usize::from(c.species)) else { continue };
        present.insert(c.id);
        let flies = sp.can(Locomotion::Fly);
        let size = (c.workers / if flies { 20_000.0 } else { 5_000.0 }).max(0.05).cbrt().clamp(0.5, 2.5);
        let p = geometry.ground_point(c.nest);
        let transform = Transform::from_translation(p).with_scale(Vec3::splat(size));
        match index.colonies.get(&c.id).copied() {
            Some(e) => {
                commands.entity(e).insert((transform, small));
            }
            None => {
                let (mesh, material) = if flies {
                    (nests.hive.clone(), nests.straw.clone())
                } else {
                    (nests.ant_mound.clone(), nests.soil.clone())
                };
                let e = commands
                    .spawn((
                        Name::new(format!("{} colony", sp.name)),
                        EcoColony { id: c.id },
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        transform,
                        small,
                        DespawnOnExit(MODE),
                    ))
                    .id();
                index.colonies.insert(c.id, e);
            }
        }
    }
    index.colonies.retain(|id, e| {
        let keep = present.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    // Density-field markers and forager squads near the camera focus.
    let focus = camera.map_or(Vec3::ZERO, |c| c.target_focus);
    let mut wanted: Vec<(Vec3, f32, AnimatedOrganism)> = Vec::new();
    if ui.layers.small_life {
        for m in &frame.markers {
            let s = usize::from(m.species);
            let Some(sp) = species.get(s) else { continue };
            let p = geometry.cell_to_plane(m.position);
            let (ground, surface) = (geometry.ground(p.x, p.y), geometry.surface(p.x, p.y));
            let swimming = sp.habitat.aquatic;
            let y = animal_height(m.altitude_m, ground, surface, swimming);
            let action = if m.altitude_m > AIRBORNE_M {
                OrganismAction::Fly
            } else if swimming {
                OrganismAction::Swim
            } else {
                OrganismAction::Walk
            };
            let organism = AnimatedOrganism::new(index.render_ids[s].clone())
                .with_action(action)
                .with_age_scale(sp.visual.scale * SMALL_LIFE_SCALE);
            wanted.push((Vec3::new(p.x, y, p.y), heading_to_yaw(m.heading), organism));
        }
        for f in &view.foragers {
            let s = usize::from(f.species);
            let Some(sp) = species.get(s) else { continue };
            let p = geometry.cell_to_plane(f.position);
            if Vec2::new(focus.x, focus.z).distance(p) > FORAGER_RADIUS {
                continue;
            }
            let y = geometry.ground(p.x, p.y) + f.altitude_m.max(0.0);
            let action = if f.altitude_m > AIRBORNE_M { OrganismAction::Fly } else { OrganismAction::Walk };
            let organism = AnimatedOrganism::new(index.render_ids[s].clone())
                .with_action(action)
                .with_age_scale(sp.visual.scale * SMALL_LIFE_SCALE);
            wanted.push((Vec3::new(p.x, y, p.y), heading_to_yaw(f.heading), organism));
        }
    }
    let pool = &mut index.markers;
    while pool.len() < wanted.len() {
        let slot = pool.len();
        let e = commands
            .spawn((
                Name::new("Small life"),
                FieldMarkerSlot(slot),
                AnimatedOrganism::new("bee"),
                // Always the animated model (reduced update rate), never a disc.
                OrganismLod { pinned: Some(LodLevel::Mid), ..default() },
                Glide::at(Vec3::ZERO, 0.0, now),
                Transform::default(),
                Visibility::Hidden,
                DespawnOnExit(MODE),
            ))
            .id();
        pool.push(e);
    }
    for (slot, &e) in pool.iter().enumerate() {
        match wanted.get(slot) {
            Some((p, yaw, organism)) => {
                let (p, yaw, organism) = (*p, *yaw, organism.clone());
                commands.queue(move |world: &mut World| {
                    let Ok(mut entity) = world.get_entity_mut(e) else { return };
                    if let Some(mut glide) = entity.get_mut::<Glide>() {
                        // Slots are reused: a marker of another place snaps there.
                        if glide.to.distance(p) > MARKER_SNAP_DISTANCE {
                            *glide = Glide::at(p, yaw, now);
                        } else {
                            glide.retarget(p, yaw, now, duration);
                        }
                    }
                    if let Some(mut org) = entity.get_mut::<AnimatedOrganism>()
                        && *org != organism
                    {
                        *org = organism;
                    }
                    if let Some(mut vis) = entity.get_mut::<Visibility>() {
                        vis.set_if_neq(Visibility::Inherited);
                    }
                });
            }
            None => {
                commands.entity(e).insert(Visibility::Hidden);
            }
        }
    }
}

/// Depth (m) a carcass of drawn length `extent` has sunk to when
/// `remaining` of its body is left: a lying body is about a third of its
/// length high, and it settles by up to most of that as it is eaten and
/// decays.
pub fn carcass_sink(extent: f32, remaining: f32) -> f32 {
    0.25 * extent.max(0.0) * (1.0 - remaining.clamp(0.0, 1.0))
}

/// Moves animals and small life along their glides.
pub fn animate_glides(time: Res<Time<Real>>, mut q: Query<(&Glide, &mut Transform)>) {
    let now = time.elapsed_secs_f64();
    q.par_iter_mut().for_each(|(glide, mut transform)| {
        if glide.progress(now) >= 1.0 && transform.translation == glide.to {
            return;
        }
        let (p, yaw) = glide.sample(now);
        let rotation = Quat::from_rotation_y(yaw);
        if transform.translation != p || transform.rotation != rotation {
            transform.translation = p;
            transform.rotation = rotation;
        }
    });
}

/// Carcasses and clutches.
type Remains = Or<(With<EcoCarcass>, With<EcoClutch>)>;

/// Animals far from the camera keep their static low-detail body instead
/// of `organism-render`'s marker discs: a disc of constant screen size is a
/// blob several metres wide at map scale, larger than any animal of the
/// model. Distant animals therefore look as small as they are (the
/// Populations tab and the overlays show where they are).
pub fn no_lod_markers(mut organisms: Query<&mut OrganismLod, With<AnimatedOrganism>>) {
    for mut lod in &mut organisms {
        if lod.pinned.is_none() && lod.level == LodLevel::Marker {
            lod.level = LodLevel::Far;
        }
    }
}

/// Applies the layer switches: carcasses and clutches directly, animals
/// and small life through a re-sync of the current frame (burrowed animals
/// stay hidden).
pub fn layer_visibility(ui: Res<EcoUi>, mut index: ResMut<AnimalIndex>, mut remains: Query<&mut Visibility, Remains>) {
    if index.layers == Some(ui.layers) {
        return;
    }
    index.layers = Some(ui.layers);
    let v = if ui.layers.animals { Visibility::Inherited } else { Visibility::Hidden };
    for mut vis in &mut remains {
        vis.set_if_neq(v);
    }
    index.seq = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glides_interpolate_and_snap() {
        let mut g = Glide::at(Vec3::ZERO, 0.0, 0.0);
        g.retarget(Vec3::new(10.0, 0.0, 0.0), 1.0, 0.0, 1.0);
        let (p, yaw) = g.sample(0.5);
        assert!((p.x - 5.0).abs() < 1e-5 && (yaw - 0.5).abs() < 1e-5);
        // Retargeting mid-way starts from the drawn pose.
        g.retarget(Vec3::new(20.0, 0.0, 0.0), 1.0, 0.5, 1.0);
        assert!((g.from.x - 5.0).abs() < 1e-5);
        // A jump snaps.
        g.retarget(Vec3::new(5000.0, 0.0, 0.0), 0.0, 0.6, 1.0);
        assert_eq!(g.sample(0.6).0, Vec3::new(5000.0, 0.0, 0.0));
        assert_eq!(Glide::at(Vec3::ONE, 0.0, 0.0).progress(10.0), 1.0);
    }

    #[test]
    fn carcasses_settle_as_they_are_eaten() {
        assert_eq!(carcass_sink(2.0, 1.0), 0.0, "a fresh carcass lies on the ground");
        assert!(carcass_sink(2.0, 0.5) > 0.0 && carcass_sink(2.0, 0.5) < carcass_sink(2.0, 0.0));
        assert_eq!(carcass_sink(2.0, 0.0), 0.5);
        assert_eq!(carcass_sink(2.0, 7.0), 0.0, "clamped");
    }
}
