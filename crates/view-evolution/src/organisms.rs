//! Organisms as animated procedural creatures.
//!
//! One entity per living organism (keyed by its model id) carries an
//! `organism_render::AnimatedOrganism` whose species is the organism's
//! [`Morph`]: the renderer builds and animates the body. Positions arrive
//! at the publication rate of the simulation; between publications the
//! creature glides from where it is drawn to its new position (a purely
//! visual interpolation that never feeds back into the model).

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use evolution::{Activity, Species};
use interface::SessionStatus;
use organism_engine::AgentId;
use organism_render::{AnimatedOrganism, OrganismAction, OrganismCatalog};

use crate::MODE;
use crate::morph::{Morph, PREDATOR_BASE, PREY_BASE, display_scale, morph_spec};
use crate::scene::{SceneState, heading_to_yaw, world_to_scene};
use crate::session::EvoData;
use crate::ui_state::EvoUi;

/// New morph species built per frame (each needs its meshes generated).
pub const MORPHS_PER_FRAME: usize = 3;
/// A move longer than this (cells) between two views is a jump, not a walk.
pub const SNAP_DISTANCE: f32 = 6.0;

/// A drawn organism.
#[derive(Component, Debug, Clone, Copy)]
pub struct EvoOrganism {
    /// Model id.
    pub id: AgentId,
    /// Species.
    pub species: Species,
    /// Display scale (body size).
    pub scale: f32,
}

/// Interpolated motion between two published positions.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// Where the glide started.
    pub from: Vec2,
    /// Latest published position.
    pub to: Vec2,
    /// Yaw at the start.
    pub yaw_from: f32,
    /// Target yaw.
    pub yaw_to: f32,
    /// Time the target arrived (s).
    pub start: f64,
    /// Expected time until the next publication (s).
    pub duration: f32,
}

impl Motion {
    /// At rest at `p`.
    pub fn at(p: Vec2, yaw: f32, now: f64) -> Self {
        Self { from: p, to: p, yaw_from: yaw, yaw_to: yaw, start: now, duration: 0.1 }
    }

    /// Progress in `[0, 1]` at time `now`.
    pub fn progress(&self, now: f64) -> f32 {
        if self.duration <= 0.0 { 1.0 } else { (((now - self.start) as f32) / self.duration).clamp(0.0, 1.0) }
    }

    /// Drawn position and yaw at `now`.
    pub fn sample(&self, now: f64) -> (Vec2, f32) {
        let t = self.progress(now);
        (self.from.lerp(self.to, t), lerp_angle(self.yaw_from, self.yaw_to, t))
    }

    /// Retargets to a new published position (snaps on long jumps).
    pub fn retarget(&mut self, to: Vec2, yaw: f32, now: f64, duration: f32) {
        let (here, here_yaw) = self.sample(now);
        if here.distance(to) > SNAP_DISTANCE {
            *self = Self::at(to, yaw, now);
            return;
        }
        self.from = here;
        self.yaw_from = here_yaw;
        self.to = to;
        self.yaw_to = yaw;
        self.start = now;
        self.duration = duration;
    }
}

/// Shortest-arc interpolation between two angles.
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let d = (b - a).rem_euclid(tau);
    let d = if d > std::f32::consts::PI { d - tau } else { d };
    a + d * t
}

/// The renderer action shown for a model activity.
pub fn activity_action(activity: Activity, playing: bool) -> OrganismAction {
    match (activity, playing) {
        (Activity::Graze, _) => OrganismAction::Eat,
        (Activity::Rest, _) => OrganismAction::Rest,
        (_, false) => OrganismAction::Idle,
        (Activity::Flee | Activity::Hunt, true) => OrganismAction::Run,
        (Activity::Contest, true) => OrganismAction::Attack,
        (Activity::Wander | Activity::Forage | Activity::Court, true) => OrganismAction::Walk,
    }
}

/// Closest candidate to a ray: `(index, distance along the ray)` of the
/// first candidate whose sphere `(centre, radius)` the ray passes through,
/// preferring the one whose centre is closest to the ray relative to its
/// radius.
pub fn pick_along_ray(origin: Vec3, direction: Vec3, candidates: &[(Vec3, f32)]) -> Option<usize> {
    let dir = direction.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let mut best: Option<(usize, f32)> = None;
    for (i, (centre, radius)) in candidates.iter().enumerate() {
        let along = (*centre - origin).dot(dir);
        if along <= 0.0 {
            continue;
        }
        let miss = (origin + dir * along).distance(*centre);
        if miss <= *radius {
            let score = miss / radius.max(1e-3) + along * 1e-4;
            if best.is_none_or(|(_, s)| score < s) {
                best = Some((i, score));
            }
        }
    }
    best.map(|(i, _)| i)
}

/// Id → entity of every drawn organism.
#[derive(Resource, Default, Debug)]
pub struct OrganismIndex {
    /// Entities by id.
    pub map: BTreeMap<AgentId, Entity>,
    epoch: u64,
    last_view: Option<f64>,
    interval: f32,
    /// Organisms still drawn with a stand-in morph (or not yet drawn).
    waiting: usize,
}

/// Morph species registered in the organism catalog.
#[derive(Resource, Default, Debug)]
pub struct MorphRegistry {
    ready: BTreeSet<Morph>,
    failed: bool,
}

impl MorphRegistry {
    /// Ensures `morph` is registered within this frame's budget; returns the
    /// species id to draw (the morph itself, or the closest ready one while
    /// it waits), or `None` if nothing can be drawn yet.
    fn resolve(&mut self, morph: Morph, catalog: &mut ResMut<OrganismCatalog>, budget: &mut usize) -> Option<String> {
        if self.ready.contains(&morph) {
            return Some(morph.species_id());
        }
        if *budget > 0 && !self.failed {
            *budget -= 1;
            let base_id = if morph.species == Species::Prey { PREY_BASE } else { PREDATOR_BASE };
            let spec = catalog.species.get(base_id).and_then(|base| morph_spec(base, &morph));
            match spec.map(|s| catalog.register(s)) {
                Some(Ok(())) => {
                    self.ready.insert(morph);
                    return Some(morph.species_id());
                }
                Some(Err(e)) => {
                    error!("evolution: morph {morph:?} rejected: {e}");
                    self.failed = true;
                }
                None => {
                    error!("evolution: base species `{base_id}` is missing or not a quadruped");
                    self.failed = true;
                }
            }
        }
        self.ready
            .iter()
            .filter(|m| m.species == morph.species)
            .min_by_key(|m| m.distance(&morph))
            .map(Morph::species_id)
    }
}

/// Spawns, updates and removes organism entities from the latest view.
#[allow(clippy::too_many_arguments)]
pub fn sync_organisms(
    mut commands: Commands,
    data: Res<EvoData>,
    status: Res<SessionStatus>,
    time: Res<Time<Real>>,
    mut index: ResMut<OrganismIndex>,
    mut registry: ResMut<MorphRegistry>,
    mut catalog: ResMut<OrganismCatalog>,
    mut organisms: Query<(&mut EvoOrganism, &mut Motion, Option<&mut AnimatedOrganism>, &mut Visibility)>,
) {
    let now = time.elapsed_secs_f64();
    if index.epoch != data.epoch {
        for (_, e) in std::mem::take(&mut index.map) {
            commands.entity(e).try_despawn();
        }
        index.epoch = data.epoch;
        index.last_view = None;
    }
    let Some(view) = data.view.as_ref() else { return };
    let playing = status.get().is_some_and(|s| s.playing);
    // Between views only organisms waiting for their genome or morph need work.
    if !data.view_changed && index.waiting == 0 {
        return;
    }
    if data.view_changed {
        let interval = index.last_view.map_or(0.1, |t| (now - t) as f32).clamp(1.0 / 120.0, 0.5);
        index.interval = if index.interval == 0.0 { interval } else { index.interval * 0.7 + interval * 0.3 };
        index.last_view = Some(now);
    }
    let duration = index.interval.max(1.0 / 60.0);
    let mut budget = MORPHS_PER_FRAME;
    let mut waiting = 0;
    let mut seen: BTreeSet<AgentId> = BTreeSet::new();
    for o in &view.organisms {
        seen.insert(o.id);
        let position = Vec2::new(o.position.x, o.position.y);
        let yaw = heading_to_yaw(o.heading);
        let morph = data.traits.get(&o.id).filter(|(s, _)| *s == o.species).map(|(s, g)| Morph::from_genome(*s, g));
        let species_id = morph.and_then(|m| registry.resolve(m, &mut catalog, &mut budget));
        if morph.is_none_or(|m| species_id.as_deref() != Some(m.species_id().as_str())) {
            waiting += 1;
        }
        let scale = display_scale(o.species, o.size);
        let action = activity_action(o.activity, playing);
        match index.map.get(&o.id).copied() {
            Some(entity) => {
                let Ok((mut evo, mut motion, animated, mut visibility)) = organisms.get_mut(entity) else {
                    continue;
                };
                let step = motion.to.distance(position);
                if data.view_changed {
                    motion.retarget(position, yaw, now, duration);
                }
                evo.scale = scale;
                match (animated, species_id) {
                    (Some(mut animated), id) => {
                        if data.view_changed {
                            let speed = if playing { step / duration } else { 0.0 };
                            if (animated.speed - speed).abs() > 0.05 {
                                animated.speed = speed;
                            }
                        }
                        if (animated.age_scale - scale).abs() > 1e-4 {
                            animated.age_scale = scale;
                        }
                        if animated.action != action {
                            animated.action = action;
                        }
                        if let Some(id) = id
                            && animated.species != id
                        {
                            animated.species = id;
                        }
                    }
                    (None, Some(id)) => {
                        commands
                            .entity(entity)
                            .insert(AnimatedOrganism::new(id).with_action(action).with_age_scale(scale));
                        visibility.set_if_neq(Visibility::Inherited);
                    }
                    (None, None) => {}
                }
            }
            None => {
                let mut entity = commands.spawn((
                    EvoOrganism { id: o.id, species: o.species, scale },
                    Motion::at(position, yaw, now),
                    Transform::from_translation(world_to_scene(position, 0.0))
                        .with_rotation(Quat::from_rotation_y(yaw)),
                    DespawnOnExit(MODE),
                ));
                match species_id {
                    Some(id) => {
                        entity.insert((
                            AnimatedOrganism::new(id).with_action(action).with_age_scale(scale),
                            Visibility::Inherited,
                        ));
                    }
                    None => {
                        entity.insert(Visibility::Hidden);
                    }
                }
                index.map.insert(o.id, entity.id());
            }
        }
    }
    let gone: Vec<AgentId> = index.map.keys().filter(|id| !seen.contains(id)).copied().collect();
    for id in gone {
        if let Some(e) = index.map.remove(&id) {
            commands.entity(e).try_despawn();
        }
    }
    index.waiting = waiting;
}

/// `OnExit`: the organism entities are despawned with the mode; forget them.
pub fn clear_index(mut index: ResMut<OrganismIndex>) {
    *index = OrganismIndex::default();
}

/// Keeps idle/moving actions in step with play/pause between views.
pub fn update_actions(
    data: Res<EvoData>,
    status: Res<SessionStatus>,
    mut last: Local<Option<bool>>,
    mut organisms: Query<(&EvoOrganism, &mut AnimatedOrganism)>,
) {
    let playing = status.get().is_some_and(|s| s.playing);
    if *last == Some(playing) {
        return;
    }
    *last = Some(playing);
    let Some(view) = data.view.as_ref() else { return };
    let activity: BTreeMap<AgentId, Activity> = view.organisms.iter().map(|o| (o.id, o.activity)).collect();
    for (evo, mut animated) in &mut organisms {
        if let Some(a) = activity.get(&evo.id) {
            let action = activity_action(*a, playing);
            if animated.action != action {
                animated.action = action;
            }
            if !playing && animated.speed != 0.0 {
                animated.speed = 0.0;
            }
        }
    }
}

/// Moves every organism along its interpolated path, on the ground.
pub fn animate_motion(time: Res<Time<Real>>, scene: Res<SceneState>, mut organisms: Query<(&Motion, &mut Transform)>) {
    let now = time.elapsed_secs_f64();
    for (motion, mut transform) in &mut organisms {
        let (p, yaw) = motion.sample(now);
        let target = world_to_scene(p, scene.field.sample(p));
        if transform.translation != target {
            transform.translation = target;
        }
        let rotation = Quat::from_rotation_y(yaw);
        if transform.rotation != rotation {
            transform.rotation = rotation;
        }
    }
}

/// Ring under the selected organism.
#[derive(Component, Debug)]
pub struct SelectionRing;

/// Ring under the hovered organism.
#[derive(Component, Debug)]
pub struct HoverRing;

/// `OnEnter`: the selection and hover rings.
pub fn spawn_markers(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let ring = meshes.add(Torus::new(0.46, 0.54));
    let accent = MODE.default_accent();
    let accent = Color::srgb_u8(accent.r(), accent.g(), accent.b());
    let selected = materials.add(StandardMaterial {
        base_color: accent,
        emissive: accent.to_linear() * 2.5,
        unlit: true,
        ..default()
    });
    let hover = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.55),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    commands.spawn((
        Name::new("Selection ring"),
        SelectionRing,
        Mesh3d(ring.clone()),
        MeshMaterial3d(selected),
        Transform::default(),
        Visibility::Hidden,
        bevy::light::NotShadowCaster,
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Hover ring"),
        HoverRing,
        Mesh3d(ring),
        MeshMaterial3d(hover),
        Transform::default(),
        Visibility::Hidden,
        bevy::light::NotShadowCaster,
        DespawnOnExit(MODE),
    ));
}

type RingItem = (&'static mut Transform, &'static mut Visibility);
type SelectionRingFilter = (With<SelectionRing>, Without<EvoOrganism>, Without<HoverRing>);
type HoverRingFilter = (With<HoverRing>, Without<EvoOrganism>, Without<SelectionRing>);

/// Places the rings under the selected and hovered organisms.
pub fn update_markers(
    ui: Res<EvoUi>,
    index: Res<OrganismIndex>,
    time: Res<Time<Real>>,
    organisms: Query<(&EvoOrganism, &Transform)>,
    mut selected: Query<RingItem, SelectionRingFilter>,
    mut hovered: Query<RingItem, HoverRingFilter>,
) {
    let target = |id: Option<AgentId>, pulse: f32| -> Option<(Vec3, f32)> {
        let (evo, t) = id.and_then(|id| index.map.get(&id)).and_then(|e| organisms.get(*e).ok())?;
        let size = evo.scale * if evo.species == Species::Prey { 0.55 } else { 0.95 } * pulse;
        Some((t.translation + Vec3::Y * 0.04, size))
    };
    let pulse = 1.0 + 0.06 * (time.elapsed_secs() * 4.0).sin();
    let hover = ui.hovered.filter(|h| Some(*h) != ui.selected);
    for (placement, (mut transform, mut visibility)) in
        [(target(ui.selected, pulse), selected.iter_mut().next()), (target(hover, 1.0), hovered.iter_mut().next())]
            .into_iter()
            .filter_map(|(p, ring)| ring.map(|r| (p, r)))
    {
        match placement {
            Some((position, size)) => {
                transform.translation = position;
                transform.scale = Vec3::new(size, 0.35, size);
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_glides_and_snaps() {
        let mut m = Motion::at(Vec2::ZERO, 0.0, 0.0);
        m.retarget(Vec2::new(1.0, 0.0), 1.0, 0.0, 0.1);
        let (p, yaw) = m.sample(0.05);
        assert!((p.x - 0.5).abs() < 1e-5 && (yaw - 0.5).abs() < 1e-5);
        assert_eq!(m.sample(1.0).0, Vec2::new(1.0, 0.0), "holds the target after arriving");
        // Retargeting mid-glide starts from where it is drawn.
        m.retarget(Vec2::new(2.0, 0.0), 1.0, 0.05, 0.1);
        assert!((m.from.x - 0.5).abs() < 1e-5);
        m.retarget(Vec2::new(50.0, 0.0), 0.0, 0.2, 0.1);
        assert_eq!(m.sample(0.2).0, Vec2::new(50.0, 0.0), "long jumps snap");
    }

    #[test]
    fn angles_take_the_short_way() {
        let a = lerp_angle(3.0, -3.0, 0.5);
        assert!((a.rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs() < 0.01, "{a}");
        assert!((lerp_angle(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn activities_map_to_visible_actions() {
        assert_eq!(activity_action(Activity::Flee, true), OrganismAction::Run);
        assert_eq!(activity_action(Activity::Hunt, true), OrganismAction::Run);
        assert_eq!(activity_action(Activity::Forage, true), OrganismAction::Walk);
        assert_eq!(activity_action(Activity::Graze, false), OrganismAction::Eat);
        assert_eq!(activity_action(Activity::Flee, false), OrganismAction::Idle, "paused runs stand still");
        assert_eq!(activity_action(Activity::Contest, true), OrganismAction::Attack);
        assert_eq!(activity_action(Activity::Rest, true), OrganismAction::Rest);
    }

    #[test]
    fn picking_prefers_the_organism_nearest_the_ray() {
        let origin = Vec3::new(0.0, 10.0, 0.0);
        let down = Vec3::NEG_Y;
        let candidates =
            [(Vec3::new(0.4, 0.0, 0.0), 0.5), (Vec3::new(0.1, 0.0, 0.0), 0.5), (Vec3::new(3.0, 0.0, 0.0), 0.5)];
        assert_eq!(pick_along_ray(origin, down, &candidates), Some(1));
        assert_eq!(pick_along_ray(origin, Vec3::Y, &candidates), None, "behind the camera");
        assert_eq!(pick_along_ray(Vec3::new(10.0, 10.0, 0.0), down, &candidates), None, "missed");
        assert_eq!(pick_along_ray(origin, Vec3::ZERO, &candidates), None);
    }
}
