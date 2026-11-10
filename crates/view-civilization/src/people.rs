//! People: stylised articulated figures (head, torso, arms, legs) with a
//! walk cycle matched to their on-screen speed, work and rest poses by
//! activity, tunics in their group's colour and a bundle on the back
//! showing what they carry.

use std::collections::HashMap;

use bevy::prelude::*;
use civilization::view::AgentView;
use civilization::{Activity, Good};
use organism_engine::AgentId;

use crate::MODE;
use crate::coords::{angle_delta, heading_to_yaw, to_world};
use crate::meshes::{self, Carry, HAIR, HIP, HIP_X, SHOULDER, SHOULDER_X, SKIN};
use crate::scene::{CivMaterials, MapCache};
use crate::state::CurrentFrame;

/// Shared mesh handles of the mode.
#[derive(Resource, Clone)]
pub struct CivMeshes {
    /// Round hut.
    pub hut: Handle<Mesh>,
    /// Group store.
    pub storage: Handle<Mesh>,
    /// Farm plot.
    pub farm: Handle<Mesh>,
    /// Crops by stage.
    pub crops: Vec<Handle<Mesh>>,
    /// Construction scaffold.
    pub scaffold: Handle<Mesh>,
    /// Group pennant.
    pub pennant: Handle<Mesh>,
    /// Rock variants.
    pub rocks: Vec<Handle<Mesh>>,
    /// Bushes by berry level and variant.
    pub bushes: Vec<Vec<Handle<Mesh>>>,
    /// Tunic.
    pub torso: Handle<Mesh>,
    /// Sleeve.
    pub sleeve: Handle<Mesh>,
    /// Hands by skin tone.
    pub hands: Vec<Handle<Mesh>>,
    /// Heads by skin tone and hair colour.
    pub heads: Vec<Vec<Handle<Mesh>>>,
    /// Leg.
    pub leg: Handle<Mesh>,
    /// Carried bundles (food, wood, stone).
    pub carry: [Handle<Mesh>; 3],
    /// Hand axe.
    pub tool: Handle<Mesh>,
}

impl FromWorld for CivMeshes {
    fn from_world(world: &mut World) -> Self {
        let mut m = world.resource_mut::<Assets<Mesh>>();
        let mut add = |data: organism_render::procedural::mesh::MeshData| m.add(data.to_mesh());
        Self {
            hut: add(meshes::hut_mesh()),
            storage: add(meshes::storage_mesh()),
            farm: add(meshes::farm_mesh()),
            crops: (0..meshes::CROP_STAGES).map(|s| add(meshes::crop_mesh(s))).collect(),
            scaffold: add(meshes::scaffold_mesh()),
            pennant: add(meshes::pennant_mesh()),
            rocks: (0..4).map(|v| add(meshes::rock_mesh(v))).collect(),
            bushes: (0..meshes::BERRY_LEVELS).map(|l| (0..2).map(|v| add(meshes::bush_mesh(l, v))).collect()).collect(),
            torso: add(meshes::torso_mesh()),
            sleeve: add(meshes::sleeve_mesh()),
            hands: (0..SKIN.len()).map(|s| add(meshes::hand_mesh(s))).collect(),
            heads: (0..SKIN.len()).map(|s| (0..HAIR.len()).map(|h| add(meshes::head_mesh(s, h))).collect()).collect(),
            leg: add(meshes::leg_mesh()),
            carry: [
                add(meshes::carry_mesh(Carry::Food)),
                add(meshes::carry_mesh(Carry::Wood)),
                add(meshes::carry_mesh(Carry::Stone)),
            ],
            tool: add(meshes::tool_mesh()),
        }
    }
}

/// Entities of one figure's moving parts.
#[derive(Clone, Copy, Debug)]
pub struct Parts {
    body: Entity,
    leg_l: Entity,
    leg_r: Entity,
    arm_l: Entity,
    arm_r: Entity,
    carry: [Entity; 3],
    tool: Entity,
    tinted: [Entity; 3],
}

/// One person as drawn.
#[derive(Component, Clone, Debug)]
pub struct Person {
    /// Identifier.
    pub id: AgentId,
    /// Latest simulated position (tiles).
    pub target: Vec2,
    /// Latest simulated heading.
    pub heading: f32,
    /// Activity of the last tick.
    pub activity: Activity,
    /// Adult or child.
    pub adult: bool,
    /// Group id (0 = none).
    pub group: u32,
    /// Carried goods.
    pub carrying: [f32; 4],
    /// Drawn position (tiles), easing towards `target`.
    pub rendered: Vec2,
    /// Drawn height of the feet.
    pub ground: f32,
    /// Drawn yaw.
    pub yaw: f32,
    /// Smoothed on-screen speed (tiles per second).
    pub speed: f32,
    /// Walk-cycle phase (radians).
    pub phase: f32,
    /// Visual enlargement when the camera is far (see [`figure_boost`]).
    pub boost: f32,
    /// Constant visual displacement from the simulated position (see
    /// [`personal_offset`]).
    pub offset: Vec2,
    parts: Parts,
}

impl Person {
    /// Drawn scale of the figure: children are smaller, and every figure is
    /// enlarged when the camera is far away so people stay visible.
    pub fn scale(&self) -> f32 {
        (if self.adult { 1.0 } else { 0.68 }) * self.boost
    }

    /// Where the figure stands (tiles): the eased position plus its
    /// personal offset.
    pub fn drawn(&self) -> Vec2 {
        self.rendered + self.offset
    }

    /// Point at chest height (for picking and badges).
    pub fn chest(&self) -> Vec3 {
        to_world(self.drawn(), self.ground + 0.33 * self.scale())
    }

    /// Point just above the head.
    pub fn head_top(&self) -> Vec3 {
        to_world(self.drawn(), self.ground + 0.56 * self.scale())
    }
}

/// Small constant displacement of a figure from its simulated position
/// (0.08–0.2 tiles in a direction fixed per person). People working the
/// same farm plot or drinking at the same shore share one position in the
/// model; without it their figures and badges would be drawn on top of
/// each other.
pub fn personal_offset(id: AgentId) -> Vec2 {
    let a = crate::coords::tile_hash(id.0 as u32, (id.0 >> 32) as u32, 91) * std::f32::consts::TAU;
    let r = 0.08 + 0.12 * crate::coords::tile_hash(id.0 as u32, (id.0 >> 32) as u32, 92);
    Vec2::new(a.cos(), a.sin()) * r
}

/// Marker of animated parts.
#[derive(Component, Debug, Clone, Copy)]
pub struct Limb;

/// Figures by person id.
#[derive(Resource, Default)]
pub struct PeopleIndex(pub HashMap<AgentId, Entity>);

/// Joint angles and offsets of one frame of animation (radians; positive
/// pitch swings a hanging limb backwards, towards −Z).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    /// Vertical offset of the hips.
    pub body_drop: f32,
    /// Forward lean of the upper body.
    pub lean: f32,
    /// Left and right leg pitch.
    pub legs: [f32; 2],
    /// Left and right arm pitch.
    pub arms: [f32; 2],
    /// Left and right arm roll (outwards).
    pub arms_out: [f32; 2],
}

/// Pose for an activity at walk amount `walk` (0 standing … 1 walking),
/// walk phase `phase` and time `t` (seconds); `seed` desynchronises people.
pub fn pose(activity: Activity, walk: f32, phase: f32, t: f32, seed: f32) -> Pose {
    let walk = walk.clamp(0.0, 1.0);
    let s = phase.sin();
    let mut p = Pose {
        body_drop: -0.012 * s.abs() * walk,
        lean: 0.06 * walk,
        legs: [0.62 * s * walk, -0.62 * s * walk],
        arms: [-0.5 * s * walk, 0.5 * s * walk],
        arms_out: [0.08, 0.08],
    };
    let still = 1.0 - walk;
    if still < 0.05 {
        return p;
    }
    let w = t + seed * 7.0;
    let blend = |a: f32, b: f32| a * walk + b * still;
    let work = match activity {
        Activity::GatherWood | Activity::GatherStone | Activity::Build | Activity::Craft => {
            // Chopping / hammering with the right arm.
            let swing = (w * 5.5).sin();
            Some(Pose {
                body_drop: 0.0,
                lean: 0.18 + 0.06 * swing,
                legs: [-0.12, 0.1],
                arms: [-0.5, -1.55 - 0.85 * swing],
                arms_out: [0.12, 0.05],
            })
        }
        Activity::Farm | Activity::GatherFood => {
            // Bent over, picking with both hands.
            let pick = (w * 3.2).sin();
            Some(Pose {
                body_drop: -0.03,
                lean: 0.55,
                legs: [-0.25, -0.05],
                arms: [-0.9 - 0.3 * pick, -0.9 + 0.3 * pick],
                arms_out: [0.1, 0.1],
            })
        }
        Activity::Rest => Some(Pose {
            // Sitting on the ground.
            body_drop: -0.15,
            lean: -0.08,
            legs: [-1.45, -1.4],
            arms: [-0.35, -0.3],
            arms_out: [0.25, 0.25],
        }),
        Activity::Eat | Activity::Drink => {
            let sip = (w * 2.4).sin();
            Some(Pose {
                body_drop: if activity == Activity::Drink { -0.04 } else { 0.0 },
                lean: if activity == Activity::Drink { 0.35 } else { 0.05 },
                legs: [0.0, 0.0],
                arms: [-0.2, -2.1 - 0.2 * sip],
                arms_out: [0.08, -0.15],
            })
        }
        Activity::Trade | Activity::Socialize | Activity::Share => {
            let talk = (w * 2.8).sin();
            Some(Pose {
                body_drop: 0.0,
                lean: 0.02,
                legs: [0.0, 0.0],
                arms: [-0.6 - 0.25 * talk, -0.9 + 0.35 * talk],
                arms_out: [0.25, 0.35],
            })
        }
        Activity::Steal => {
            Some(Pose { body_drop: -0.04, lean: 0.4, legs: [-0.3, 0.2], arms: [-1.2, -1.0], arms_out: [0.05, 0.05] })
        }
        Activity::Mate => {
            Some(Pose { body_drop: 0.0, lean: 0.0, legs: [0.0, 0.0], arms: [-0.4, -0.4], arms_out: [0.5, 0.5] })
        }
        Activity::Store => {
            Some(Pose { body_drop: -0.02, lean: 0.3, legs: [-0.1, 0.0], arms: [-1.1, -1.1], arms_out: [0.1, 0.1] })
        }
        Activity::Idle | Activity::Explore => {
            let breathe = 0.04 * (w * 1.3).sin();
            Some(Pose { body_drop: 0.0, lean: 0.0, legs: [0.0, 0.0], arms: [breathe, -breathe], arms_out: [0.1, 0.1] })
        }
    };
    if let Some(q) = work {
        p = Pose {
            body_drop: blend(p.body_drop, q.body_drop),
            lean: blend(p.lean, q.lean),
            legs: [blend(p.legs[0], q.legs[0]), blend(p.legs[1], q.legs[1])],
            arms: [blend(p.arms[0], q.arms[0]), blend(p.arms[1], q.arms[1])],
            arms_out: [blend(p.arms_out[0], q.arms_out[0]), blend(p.arms_out[1], q.arms_out[1])],
        };
    }
    p
}

/// Bundle shown on the back: the bulk good carried most, if at least 1.5
/// units, with a size factor growing with the load (carry limit 24).
pub fn carried_bundle(carrying: &[f32; 4]) -> Option<(Carry, f32)> {
    let goods = [(Carry::Food, Good::Food), (Carry::Wood, Good::Wood), (Carry::Stone, Good::Stone)];
    let (kind, amount) = goods.iter().map(|(k, g)| (*k, carrying[g.index()])).max_by(|a, b| a.1.total_cmp(&b.1))?;
    let load: f32 = carrying[..3].iter().sum();
    (amount >= 1.5).then(|| (kind, 0.75 + 0.6 * (load / civilization::agent::CARRY_CAPACITY).clamp(0.0, 1.0)))
}

fn person_seed(id: AgentId) -> (usize, usize, f32) {
    let h = crate::coords::tile_hash((id.0 & 0xffff_ffff) as u32, (id.0 >> 32) as u32, 77);
    let skin = (h * SKIN.len() as f32) as usize % SKIN.len();
    let hair = (crate::coords::tile_hash(id.0 as u32, 1, 78) * HAIR.len() as f32) as usize % HAIR.len();
    (skin, hair, h)
}

fn group_colour(frame: &crate::session::Frame, group: u32) -> Color {
    let colour = (group != 0).then(|| frame.view.groups.iter().find(|g| g.id.0 == group).map(|g| g.color)).flatten();
    crate::palette::group_color(colour)
}

fn spawn_person(
    commands: &mut Commands,
    a: &AgentView,
    ground: f32,
    assets: &CivMeshes,
    white: &Handle<StandardMaterial>,
    cloth: &Handle<StandardMaterial>,
) -> Entity {
    let (skin, hair, _) = person_seed(a.id);
    let offset = personal_offset(a.id);
    let root = commands
        .spawn((
            Name::new(format!("Person #{}", a.id.0)),
            Transform::from_translation(to_world(a.position + offset, ground)),
            Visibility::Inherited,
            DespawnOnExit(MODE),
        ))
        .id();
    let mesh = |h: &Handle<Mesh>, m: &Handle<StandardMaterial>| (Mesh3d(h.clone()), MeshMaterial3d(m.clone()));
    let body = commands.spawn((Limb, Transform::from_xyz(0.0, HIP, 0.0), Visibility::Inherited, ChildOf(root))).id();
    let torso = commands.spawn((mesh(&assets.torso, cloth), Transform::IDENTITY, ChildOf(body))).id();
    commands.spawn((mesh(&assets.heads[skin][hair], white), Transform::IDENTITY, ChildOf(body)));
    let arm = |commands: &mut Commands, side: f32| {
        let pivot = commands
            .spawn((Limb, Transform::from_xyz(side * SHOULDER_X, SHOULDER, 0.0), Visibility::Inherited, ChildOf(body)))
            .id();
        let sleeve = commands.spawn((mesh(&assets.sleeve, cloth), Transform::IDENTITY, ChildOf(pivot))).id();
        commands.spawn((mesh(&assets.hands[skin], white), Transform::IDENTITY, ChildOf(pivot)));
        (pivot, sleeve)
    };
    let (arm_l, sleeve_l) = arm(commands, 1.0);
    let (arm_r, sleeve_r) = arm(commands, -1.0);
    let leg = |commands: &mut Commands, side: f32| {
        commands
            .spawn((mesh(&assets.leg, white), Limb, Transform::from_xyz(side * HIP_X, HIP, 0.0), ChildOf(root)))
            .id()
    };
    let leg_l = leg(commands, 1.0);
    let leg_r = leg(commands, -1.0);
    let carry = [0, 1, 2].map(|k| {
        commands
            .spawn((mesh(&assets.carry[k], white), Limb, Transform::IDENTITY, Visibility::Hidden, ChildOf(body)))
            .id()
    });
    let tool = commands
        .spawn((
            mesh(&assets.tool, white),
            Limb,
            Transform::from_xyz(-0.07, 0.03, 0.02),
            Visibility::Hidden,
            ChildOf(body),
        ))
        .id();
    let parts = Parts { body, leg_l, leg_r, arm_l, arm_r, carry, tool, tinted: [torso, sleeve_l, sleeve_r] };
    commands.entity(root).insert(Person {
        id: a.id,
        target: a.position,
        heading: a.heading,
        activity: a.activity,
        adult: a.adult,
        group: a.group.map_or(0, |g| g.0),
        carrying: a.carrying.0,
        rendered: a.position,
        ground,
        yaw: heading_to_yaw(a.heading),
        speed: 0.0,
        phase: 0.0,
        boost: 1.0,
        offset,
        parts,
    });
    root
}

/// Spawns, updates and removes figures from the latest frame.
#[allow(clippy::too_many_arguments)]
pub fn sync_people(
    mut commands: Commands,
    current: Res<CurrentFrame>,
    cache: Res<MapCache>,
    assets: Res<CivMeshes>,
    mut materials: ResMut<CivMaterials>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut index: ResMut<PeopleIndex>,
    mut people: Query<&mut Person>,
    mut tinted: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    if !current.is_changed() {
        return;
    }
    let Some(frame) = current.get() else { return };
    let Some(field) = cache.field.as_ref() else { return };
    let mut seen = HashMap::with_capacity(frame.view.agents.len());
    for a in &frame.view.agents {
        seen.insert(a.id, ());
        let group = a.group.map_or(0, |g| g.0);
        if let Some(&e) = index.0.get(&a.id)
            && let Ok(mut p) = people.get_mut(e)
        {
            p.target = a.position;
            p.heading = a.heading;
            p.activity = a.activity;
            p.adult = a.adult;
            p.carrying = a.carrying.0;
            if p.group != group {
                p.group = group;
                let cloth = materials.group(group_colour(frame, group), &mut standard);
                for part in p.parts.tinted {
                    if let Ok(mut m) = tinted.get_mut(part) {
                        m.0 = cloth.clone();
                    }
                }
            }
            continue;
        }
        let cloth = materials.group(group_colour(frame, group), &mut standard);
        let ground = field.height_at(a.position + personal_offset(a.id));
        let e = spawn_person(&mut commands, a, ground, &assets, &materials.white, &cloth);
        index.0.insert(a.id, e);
    }
    index.0.retain(|id, e| {
        let keep = seen.contains_key(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
}

/// Enlargement of the figures for a camera distance: 1 up close, growing
/// to 1.9 when the whole map is in view (a purely visual exaggeration, as
/// in strategy maps; positions are unchanged).
pub fn figure_boost(distance: f32) -> f32 {
    if distance.is_finite() { (distance / 32.0).clamp(1.0, 1.9) } else { 1.0 }
}

/// Current camera distance (world units) of the mode's camera.
#[derive(Resource, Clone, Copy, Debug)]
pub struct CameraDistance(pub f32);

impl Default for CameraDistance {
    fn default() -> Self {
        Self(30.0)
    }
}

/// Measures the camera distance (perspective radius or the distance that
/// matches an orthographic scale).
pub fn measure_camera(
    camera: Option<Single<(&bevy_panorbit_camera::PanOrbitCamera, &Projection), With<crate::scene::CivCamera>>>,
    mut distance: ResMut<CameraDistance>,
) {
    let Some(camera) = camera else { return };
    let (pan, projection) = *camera;
    let radius = pan.radius.unwrap_or(pan.target_radius);
    let d = match projection {
        Projection::Orthographic(_) => rendering::orbit::ortho_scale_to_distance(radius, std::f32::consts::FRAC_PI_4),
        _ => radius,
    };
    if (distance.0 - d).abs() > 1e-3 {
        distance.0 = d;
    }
}

/// Eases figures towards their simulated positions and animates them.
pub fn animate_people(
    time: Res<Time<Real>>,
    cache: Res<MapCache>,
    camera: Res<CameraDistance>,
    mut people: Query<(&mut Person, &mut Transform), Without<Limb>>,
    mut limbs: Query<(&mut Transform, &mut Visibility), With<Limb>>,
) {
    let Some(field) = cache.field.as_ref() else { return };
    let dt = time.delta_secs().min(0.1);
    let t = time.elapsed_secs();
    let follow = rendering::orbit::half_life_factor(dt, 0.07);
    let boost = figure_boost(camera.0);
    for (mut p, mut root) in &mut people {
        p.boost = boost;
        let before = p.rendered;
        let gap = p.target - p.rendered;
        let next = if gap.length() > 3.0 { p.target } else { p.rendered + gap * follow };
        let moved = (next - before).length();
        p.rendered = next;
        let instant = if dt > 0.0 { moved / dt } else { 0.0 };
        p.speed += (instant - p.speed) * rendering::orbit::half_life_factor(dt, 0.15);
        let walk = (p.speed / 0.35).clamp(0.0, 1.0);
        // One full cycle per two strides of about 0.16 tiles each.
        p.phase = (p.phase + moved / 0.16 * std::f32::consts::PI) % std::f32::consts::TAU;
        let desired = if moved > 1e-4 && gap.length() > 0.02 {
            heading_to_yaw(gap.y.atan2(gap.x))
        } else {
            heading_to_yaw(p.heading)
        };
        let turn = angle_delta(p.yaw, desired);
        p.yaw += turn * rendering::orbit::half_life_factor(dt, 0.08);
        p.ground = field.height_at(p.drawn());
        root.translation = to_world(p.drawn(), p.ground);
        root.rotation = Quat::from_rotation_y(p.yaw);
        root.scale = Vec3::splat(p.scale());
        let seed = (p.id.0 % 1000) as f32 * 0.137;
        let q = pose(p.activity, walk, p.phase, t, seed);
        let parts = p.parts;
        if let Ok((mut tr, _)) = limbs.get_mut(parts.body) {
            tr.translation = Vec3::new(0.0, HIP + q.body_drop, 0.0);
            tr.rotation = Quat::from_rotation_x(q.lean);
        }
        for (entity, side, pitch) in [(parts.leg_l, 1.0, q.legs[0]), (parts.leg_r, -1.0, q.legs[1])] {
            if let Ok((mut tr, _)) = limbs.get_mut(entity) {
                tr.translation = Vec3::new(side * HIP_X, HIP + q.body_drop, 0.0);
                tr.rotation = Quat::from_rotation_x(pitch);
            }
        }
        for (entity, side, pitch, out) in
            [(parts.arm_l, 1.0, q.arms[0], q.arms_out[0]), (parts.arm_r, -1.0, q.arms[1], q.arms_out[1])]
        {
            if let Ok((mut tr, _)) = limbs.get_mut(entity) {
                tr.rotation = Quat::from_rotation_z(side * out) * Quat::from_rotation_x(pitch);
            }
        }
        let bundle = carried_bundle(&p.carrying);
        for (k, entity) in parts.carry.into_iter().enumerate() {
            if let Ok((mut tr, mut vis)) = limbs.get_mut(entity) {
                let shown = bundle.filter(|(kind, _)| *kind as usize == k);
                vis.set_if_neq(if shown.is_some() { Visibility::Inherited } else { Visibility::Hidden });
                if let Some((_, size)) = shown {
                    // Scale about the bundle's centre on the back.
                    let anchor = Vec3::new(0.0, 0.1, -0.075);
                    tr.scale = Vec3::splat(size);
                    tr.translation = anchor - anchor * size;
                }
            }
        }
        if let Ok((_, mut vis)) = limbs.get_mut(parts.tool) {
            let armed = p.carrying[Good::Tools.index()] > 0.05;
            vis.set_if_neq(if armed { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_swings_limbs_in_opposition() {
        let p = pose(Activity::Explore, 1.0, std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        assert!(p.legs[0] > 0.3 && p.legs[1] < -0.3, "{p:?}");
        assert!(p.arms[0] < 0.0 && p.arms[1] > 0.0, "arms counter-swing the legs");
        let half = pose(Activity::Explore, 1.0, 3.0 * std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        assert!(half.legs[0] < 0.0, "legs alternate over the cycle");
    }

    #[test]
    fn standing_poses_follow_the_activity() {
        let rest = pose(Activity::Rest, 0.0, 0.0, 1.0, 0.0);
        assert!(rest.body_drop < -0.1 && rest.legs[0] < -1.0, "sitting: {rest:?}");
        let chop = pose(Activity::GatherWood, 0.0, 0.0, 1.0, 0.0);
        assert!(chop.arms[1] < -0.6, "the working arm is raised: {chop:?}");
        let drink = pose(Activity::Drink, 0.0, 0.0, 1.0, 0.0);
        assert!(drink.arms[1] < -1.8 && drink.lean > 0.2, "{drink:?}");
        let idle = pose(Activity::Idle, 0.0, 0.0, 1.0, 0.0);
        assert!(idle.legs == [0.0, 0.0] && idle.body_drop == 0.0);
        // Walking overrides the work pose.
        let walking = pose(Activity::Rest, 1.0, 1.0, 1.0, 0.0);
        assert!(walking.body_drop > -0.02);
    }

    #[test]
    fn figures_grow_only_when_the_camera_is_far() {
        assert_eq!(figure_boost(10.0), 1.0);
        assert!(figure_boost(50.0) > 1.0);
        assert_eq!(figure_boost(500.0), 1.9);
        assert_eq!(figure_boost(f32::NAN), 1.0);
    }

    #[test]
    fn bundles_show_the_main_bulk_good() {
        assert_eq!(carried_bundle(&[0.5, 0.2, 0.0, 3.0]), None, "tools alone are worn, not carried");
        let (kind, small) = carried_bundle(&[2.0, 6.0, 1.0, 0.0]).expect("wood");
        assert_eq!(kind, Carry::Wood);
        let (_, big) = carried_bundle(&[2.0, 20.0, 1.0, 0.0]).expect("wood");
        assert!(big > small, "heavier loads look bigger");
        assert_eq!(carried_bundle(&[9.0, 2.0, 3.0, 0.0]).map(|b| b.0), Some(Carry::Food));
    }

    #[test]
    fn personal_offsets_separate_people_sharing_a_spot() {
        for i in 0..200 {
            let o = personal_offset(AgentId(i));
            assert!((0.079..=0.201).contains(&o.length()), "{o}");
            assert_eq!(o, personal_offset(AgentId(i)), "stable");
        }
        // Most pairs of people at one spot end up visibly apart.
        let apart = (0..100u64)
            .filter(|i| personal_offset(AgentId(2 * i)).distance(personal_offset(AgentId(2 * i + 1))) > 0.08)
            .count();
        assert!(apart >= 75, "{apart}");
    }

    #[test]
    fn people_get_varied_but_stable_looks() {
        let a = person_seed(AgentId(3));
        assert_eq!(a, person_seed(AgentId(3)));
        let looks: std::collections::HashSet<(usize, usize)> =
            (0..200).map(|i| person_seed(AgentId(i))).map(|(s, h, _)| (s, h)).collect();
        assert!(looks.len() >= 8, "{looks:?}");
    }
}
