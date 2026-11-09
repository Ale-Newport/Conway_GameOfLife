//! Procedural poses: for every rig part, an offset from its rest transform
//! as a pure function of the action, the locomotion cycle and time. The
//! Bevy runtime ([`super::runtime`]) evaluates it each frame and crossfades
//! between actions.
//!
//! Locomotion uses the biomechanics in [`crate::action`] and
//! [`super::gait`]: the gait comes from the Froude number, stride frequency
//! and length from dynamic similarity, leg sweep from stride length and duty
//! factor; fish and snakes undulate with a travelling wave; birds flap at the
//! Pennycuick frequency (display-capped, see [`DISPLAY_FLAP_LIMIT_HZ`]).

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::math::{Quat, Vec3};
use bevy::prelude::Transform;

use super::arthropod::WingRest;
use super::gait::{LegPose, MAX_SWEEP, duty_factor, hop_height, leg_pose, quadruped_phase, sweep_amplitude, wingbeat};
use super::noise::value3;
use super::pose_other::{arthropod, fish, frog, serpent, snail, turtle};
use super::rig::{GallopStyle, LegSegment, Locomotor, PartRole, RigMotion, Side, WingSegment};
use crate::action::{
    FROUDE_DEFAULT_RUN, FROUDE_DEFAULT_WALK, Gait, OrganismAction, breathing_frequency, froude_number, gait_for_froude,
    speed_for_froude, stride_frequency, stride_length,
};

/// Wing beats faster than this are shown at this rate: at 60 frames per
/// second a faster beat has fewer than five frames per cycle and strobes.
/// The displayed rate is a visual approximation of the physiological one.
pub const DISPLAY_FLAP_LIMIT_HZ: f32 = 12.0;

/// Duration (s) of the crossfade between two actions.
pub const CROSSFADE_SECONDS: f32 = 0.25;

/// Everything a pose depends on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseInput {
    /// Effective action (see [`effective_action`]).
    pub action: OrganismAction,
    /// Movement speed (m/s), already defaulted for locomotion.
    pub speed: f32,
    /// Locomotion / undulation / hop cycle (cycles, unbounded).
    pub cycle: f32,
    /// Wing-beat cycle (cycles, unbounded).
    pub flap: f32,
    /// Seconds since the action started.
    pub t: f32,
    /// Per-instance clock (s) for idle variation and breathing.
    pub clock: f32,
    /// Age scale (1 = adult).
    pub size: f32,
    /// Wing posture at rest (arthropods).
    pub wing_rest: WingRest,
}

/// Offset applied to a part's rest transform:
/// `rotation = pre · rest · post`, `translation = rest + translation`,
/// `scale = rest · scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartOffset {
    /// Rotation in the parent frame.
    pub pre: Quat,
    /// Rotation in the part's own frame.
    pub post: Quat,
    /// Translation in the parent frame.
    pub translation: Vec3,
    /// Scale multiplier.
    pub scale: Vec3,
}

impl PartOffset {
    /// No offset.
    pub const IDENTITY: PartOffset =
        PartOffset { pre: Quat::IDENTITY, post: Quat::IDENTITY, translation: Vec3::ZERO, scale: Vec3::ONE };

    /// Offset rotating in the part frame.
    pub fn post(q: Quat) -> Self {
        Self { post: q, ..Self::IDENTITY }
    }

    /// Applies the offset to a rest transform.
    pub fn apply(&self, rest: &Transform) -> Transform {
        Transform {
            translation: rest.translation + self.translation,
            rotation: self.pre * rest.rotation * self.post,
            scale: rest.scale * self.scale,
        }
    }
}

/// Smoothstep easing of `x` over `[0, 1]`.
pub fn ease(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Maps an action to one the body plan can show:
///
/// * `Flee` → `Run`; flight for flightless species → their fastest
///   locomotion; walking or running for fish → swimming.
/// * `Drink` → `Idle` for animals with no visible drinking behaviour: fish
///   and cetaceans (water is taken in with food or across the gills) and
///   earthworms (water is absorbed through the skin).
/// * `Jump` → `Attack` for turtles and tortoises, which cannot jump: their
///   sudden lunge is shown instead. (Snakes show a jump as the springing
///   strike, earthworms as the escape contraction and snails as a startled
///   withdrawal; those are handled by their pose functions.)
pub fn effective_action(motion: &RigMotion, action: OrganismAction) -> OrganismAction {
    use OrganismAction as A;
    let a = action.canonical();
    if motion.aquatic {
        return match a {
            A::Walk | A::Run | A::Fly | A::Glide => A::Swim,
            A::Drink => A::Idle,
            other => other,
        };
    }
    match (a, motion.locomotor) {
        (A::Fly | A::Glide, _) if !motion.can_fly => A::Run,
        (A::Drink, Locomotor::Peristaltic) => A::Idle,
        (A::Jump, Locomotor::Chelonian) => A::Attack,
        (other, _) => other,
    }
}

/// Default speed (m/s) for a locomotion action when the simulation gives
/// none, scaled to the body: Froude 0.25 / 3 for legged walking / running,
/// body lengths per second for undulating and gliding bodies.
pub fn default_speed(motion: &RigMotion, action: OrganismAction, size: f32) -> f32 {
    let h = motion.hip_height * size;
    let l = motion.body_length * size;
    let run = action == OrganismAction::Run;
    match motion.locomotor {
        Locomotor::Fish => l * if run { 2.5 } else { 1.0 },
        Locomotor::Serpentine => l * if run { 0.6 } else { 0.25 },
        Locomotor::Peristaltic | Locomotor::Gastropod => l * 0.12,
        Locomotor::Anuran => l * if run { 8.0 } else { 3.0 },
        _ => {
            let fr = if run {
                if motion.gallop == GallopStyle::Amble { 1.0 } else { FROUDE_DEFAULT_RUN }
            } else {
                FROUDE_DEFAULT_WALK
            };
            speed_for_froude(fr, h)
        }
    }
}

/// Frequency (cycles/s) of the locomotion cycle at `speed` for an
/// individual of age scale `size`.
pub fn cycle_frequency(motion: &RigMotion, action: OrganismAction, speed: f32, size: f32) -> f32 {
    let h = motion.hip_height * size;
    let l = motion.body_length * size;
    let v = speed.max(0.0);
    match (motion.locomotor, action) {
        (Locomotor::Fish, OrganismAction::Swim | OrganismAction::Attack) => (v / (0.7 * l)).max(0.6),
        (Locomotor::Fish, _) => 0.5,
        (Locomotor::Serpentine, _) => {
            if v > 0.0 {
                v / (motion.wave_length * l * 0.7)
            } else {
                0.25
            }
        }
        (Locomotor::Peristaltic, _) => {
            if v > 0.0 {
                v / (0.13 * l)
            } else {
                0.3
            }
        }
        (Locomotor::Gastropod, _) => 0.8,
        (Locomotor::Anuran, OrganismAction::Walk | OrganismAction::Run) => (v / (5.0 * l)).clamp(0.4, 3.0),
        (Locomotor::Anuran, OrganismAction::Swim) => 1.2,
        (_, OrganismAction::Swim) => 0.7 * stride_frequency(v.max(speed_for_froude(0.2, h)), h),
        (_, OrganismAction::Walk | OrganismAction::Run) => {
            if v > 0.0 {
                v / stride_at(motion, action, v, size).max(1e-6)
            } else {
                0.0
            }
        }
        (_, OrganismAction::Jump) => 1.0 / (0.35 + 2.0 * (0.8 * h / crate::action::GRAVITY).sqrt()),
        _ => 0.0,
    }
}

/// Gait of a quadruped at `speed`: the Froude-number gait, with ambling
/// species (elephants, tortoises) never galloping.
pub fn quadruped_gait(motion: &RigMotion, speed: f32, size: f32) -> Gait {
    let gait = gait_at(motion, speed, size);
    if motion.gallop == GallopStyle::Amble && gait == Gait::Gallop { Gait::Trot } else { gait }
}

/// Duty factor (fraction of the stride a foot is on the ground) drawn for
/// a legged body plan.
pub fn stance_duty(motion: &RigMotion, action: OrganismAction, speed: f32, size: f32) -> f32 {
    let run = action == OrganismAction::Run;
    match motion.locomotor {
        Locomotor::Quadruped => duty_factor(quadruped_gait(motion, speed, size), motion.gallop),
        Locomotor::Hexapod | Locomotor::Octopod => {
            if run {
                0.5
            } else {
                0.6
            }
        }
        Locomotor::Bird | Locomotor::Bat => {
            if run {
                0.4
            } else {
                0.6
            }
        }
        _ => duty_factor(Gait::Walk, GallopStyle::Amble),
    }
}

/// Stride length (m) drawn at `speed`: Alexander's dynamically similar
/// stride `2.3·h·Fr^0.3`, shortened when the legs could not sweep that far
/// within [`MAX_SWEEP`] (short-legged, fast animals such as insects running
/// at high Froude numbers). The stride frequency `v / stride` then rises
/// instead, so planted feet never slide.
pub fn stride_at(motion: &RigMotion, action: OrganismAction, speed: f32, size: f32) -> f32 {
    let lambda = stride_length(speed, motion.hip_height * size);
    let pairs = motion.leg_lengths.len().min(u8::MAX as usize) as u8;
    if pairs == 0 {
        return lambda;
    }
    let lever = (0..pairs).map(|p| motion.swing_lever(p)).fold(f32::MAX, f32::min) * size;
    let duty = stance_duty(motion, action, speed, size).max(0.05);
    lambda.min(2.0 * lever * MAX_SWEEP.sin() / duty)
}

/// Displayed wing-beat frequency (Hz), see [`DISPLAY_FLAP_LIMIT_HZ`].
pub fn display_flap_hz(motion: &RigMotion) -> f32 {
    motion.flap_hz.min(DISPLAY_FLAP_LIMIT_HZ)
}

/// Gait at `speed` for the body plan.
pub fn gait_at(motion: &RigMotion, speed: f32, size: f32) -> Gait {
    gait_for_froude(froude_number(speed, motion.hip_height * size))
}

/// Slow, smooth pseudo-random signal in [-1, 1] for idle variation.
pub fn wander(clock: f32, channel: f32) -> f32 {
    value3(Vec3::new(clock * 0.35, channel * 7.3, 0.5), 0x7a11)
}

/// Offset of one part for the given input.
pub fn part_offset(motion: &RigMotion, role: PartRole, rest: &Transform, input: &PoseInput) -> PartOffset {
    match motion.locomotor {
        Locomotor::Quadruped => quadruped(motion, role, rest, input),
        Locomotor::Bird | Locomotor::Bat => bird(motion, role, rest, input),
        Locomotor::Hexapod | Locomotor::Octopod => arthropod(motion, role, rest, input),
        Locomotor::Fish => fish(motion, role, rest, input),
        Locomotor::Serpentine | Locomotor::Peristaltic => serpent(motion, role, rest, input),
        Locomotor::Anuran => frog(motion, role, rest, input),
        Locomotor::Chelonian => turtle(motion, role, rest, input),
        Locomotor::Gastropod => snail(motion, role, rest, input),
    }
}

/// Breathing scale on the root (chest expansion).
pub fn breathing(motion: &RigMotion, input: &PoseInput, depth: f32) -> Vec3 {
    let f = breathing_frequency(motion.mass_kg * input.size.powi(3))
        * if input.action == OrganismAction::Rest { 0.7 } else { 1.0 };
    let b = (TAU * f * input.clock).sin();
    Vec3::new(1.0 + 0.5 * depth * b, 1.0 + depth * b, 1.0)
}

/// Common death pose of the root: roll onto the side and lie at the body's
/// half width, then settle slowly into the ground (decay).
pub fn death_root(motion: &RigMotion, rest: &Transform, t: f32, roll: f32) -> PartOffset {
    let k = ease(t / 0.9);
    let lying = motion.half_width.max(0.02 * motion.body_length);
    let drop = (rest.translation.y - lying).max(0.0) * k;
    let settle = 0.35 * lying * ease((t - 3.0) / 6.0);
    PartOffset {
        pre: Quat::from_rotation_z(roll * k),
        translation: Vec3::new(0.0, -drop - settle, 0.0),
        ..PartOffset::IDENTITY
    }
}

/// Rotation of one leg segment for a point of its stride cycle.
fn leg_protraction(pair: u8, p: LegPose, flex: f32, segment: LegSegment, sprawl: bool, side: Side) -> PartOffset {
    let s = side.sign();
    match (segment, sprawl) {
        (LegSegment::Upper, false) => PartOffset::post(Quat::from_rotation_x(-p.swing)),
        (LegSegment::Lower, false) => {
            let dir = if pair == 0 { 1.0 } else { -1.0 };
            PartOffset::post(Quat::from_rotation_x(dir * flex * p.lift))
        }
        (LegSegment::Upper, true) => PartOffset {
            pre: Quat::from_rotation_y(-s * p.swing),
            post: Quat::from_rotation_z(s * 0.45 * p.lift),
            ..PartOffset::IDENTITY
        },
        (LegSegment::Lower, true) => PartOffset::post(Quat::from_rotation_z(-s * 0.3 * p.lift)),
    }
}

fn quadruped(m: &RigMotion, role: PartRole, rest: &Transform, i: &PoseInput) -> PartOffset {
    use OrganismAction as A;
    let sprawl = m.wave_amplitude > 0.0;
    let gait = quadruped_gait(m, i.speed, i.size);
    let duty = duty_factor(gait, m.gallop);
    let stride = stride_at(m, i.action, i.speed, i.size);
    let moving = matches!(i.action, A::Walk | A::Run | A::Swim);
    let lie = ease(i.t / 1.2);
    match role {
        PartRole::Root => match i.action {
            A::Death => death_root(m, rest, i.t, FRAC_PI_2),
            A::Rest => PartOffset {
                translation: Vec3::new(0.0, -m.rest_drop * lie, 0.0),
                scale: breathing(m, i, 0.025),
                ..PartOffset::IDENTITY
            },
            A::Walk | A::Run => {
                let (bob, pitch) = match gait {
                    Gait::Gallop => (0.04 * m.hip_height * (TAU * i.cycle).sin(), 0.09 * (TAU * i.cycle).cos()),
                    _ => (-0.018 * m.hip_height * (2.0 * TAU * i.cycle).cos(), 0.0),
                };
                let yaw = if sprawl { 0.22 * (TAU * i.cycle).sin() } else { 0.0 };
                PartOffset {
                    pre: Quat::from_rotation_x(pitch) * Quat::from_rotation_y(yaw),
                    translation: Vec3::new(0.0, bob, 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Swim => {
                let sink = rest.translation.y - 0.35 * (rest.translation.y - m.rest_drop);
                PartOffset {
                    pre: Quat::from_rotation_x(-0.12),
                    translation: Vec3::new(0.0, -sink + 0.01 * m.hip_height * (TAU * i.cycle).sin(), 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Jump => {
                let hop = hop_height(i.t * 1.1, 0.6);
                PartOffset {
                    pre: Quat::from_rotation_x(-0.25 * hop),
                    translation: Vec3::new(0.0, 0.6 * m.hip_height * hop, 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Attack => {
                let lunge = (TAU * 1.1 * i.t).sin().max(0.0);
                PartOffset {
                    pre: Quat::from_rotation_x(0.08 * lunge),
                    translation: Vec3::new(0.0, 0.0, 0.12 * m.body_length * lunge),
                    ..PartOffset::IDENTITY
                }
            }
            _ => PartOffset { scale: breathing(m, i, 0.02), ..PartOffset::IDENTITY },
        },
        PartRole::Leg { pair, side, segment } => {
            let leg = m.leg_length(pair) * i.size;
            match i.action {
                A::Walk | A::Run | A::Swim => {
                    let amp = sweep_amplitude(stride, duty, leg) * if i.action == A::Swim { 1.4 } else { 1.0 };
                    let phase = i.cycle + quadruped_phase(gait, m.gallop, pair, side);
                    let flex = if gait == Gait::Gallop { 1.3 } else { 0.9 };
                    leg_protraction(pair, leg_pose(phase, duty, amp), flex, segment, sprawl, side)
                }
                A::Rest if !sprawl => {
                    let (u, l) = if pair == 0 { (1.3, -2.3) } else { (-1.3, 2.4) };
                    PartOffset::post(Quat::from_rotation_x(lie * if segment == LegSegment::Upper { u } else { l }))
                }
                A::Death => {
                    let k = ease(i.t / 0.9);
                    let a = if segment == LegSegment::Upper { 0.35 } else { 0.25 } * if pair == 0 { -1.0 } else { 1.0 };
                    PartOffset::post(Quat::from_rotation_x(a * k))
                }
                A::Jump => {
                    let tuck = hop_height(i.t * 1.1, 0.6);
                    let a = if pair == 0 { 0.7 } else { -0.7 } * if segment == LegSegment::Upper { 1.0 } else { -1.2 };
                    PartOffset::post(Quat::from_rotation_x(a * tuck))
                }
                _ => PartOffset::IDENTITY,
            }
        }
        PartRole::Neck(k) => {
            let first = k == 0;
            let nod = if moving { 0.05 * (2.0 * TAU * i.cycle).sin() } else { 0.0 };
            let pitch = match i.action {
                A::Eat | A::Drink if first => m.graze_pitch * ease(i.t / 0.7) + 0.05 * (TAU * 0.8 * i.t).sin(),
                A::Rest if first => 0.35 * lie,
                A::Death if first => 0.5 * ease(i.t / 0.9),
                A::Attack if first => 0.25 * (TAU * 1.1 * i.t).sin().max(0.0),
                A::Swim if first => -0.35,
                A::Idle if first => 0.12 * wander(i.clock, 1.0),
                _ => 0.0,
            };
            let yaw = if i.action == A::Idle && first { 0.35 * wander(i.clock, 2.0) } else { 0.0 };
            PartOffset::post(Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch + if first { nod } else { 0.0 }))
        }
        PartRole::Head => match i.action {
            A::Idle => PartOffset::post(
                Quat::from_rotation_x(0.1 * wander(i.clock, 3.0)) * Quat::from_rotation_z(0.08 * wander(i.clock, 4.0)),
            ),
            A::Rest => PartOffset::post(Quat::from_rotation_x(-0.25 * lie)),
            _ => PartOffset::IDENTITY,
        },
        PartRole::Jaw => {
            let open = match i.action {
                A::Eat => 0.18 * (0.5 + 0.5 * (TAU * 2.2 * i.t).sin()),
                A::Drink => 0.08 * (0.5 + 0.5 * (TAU * 3.5 * i.t).sin()),
                A::Attack => 0.55 * (TAU * 1.1 * i.t).sin().max(0.0),
                A::Run if gait == Gait::Gallop => 0.12,
                A::Death => 0.2,
                _ => 0.0,
            };
            PartOffset::post(Quat::from_rotation_x(open))
        }
        PartRole::Ear(side) => {
            let s = side.sign();
            match i.action {
                A::Attack | A::Run => PartOffset::post(Quat::from_rotation_x(-0.45)),
                // Elephants fan their ears; others flick them now and then.
                _ if m.mass_kg > 1000.0 => {
                    PartOffset::post(Quat::from_rotation_y(s * 0.25 * (TAU * 0.35 * i.clock + s).sin()))
                }
                _ => {
                    let flick = (wander(i.clock * 2.0, 5.0 + s) - 0.6).max(0.0) * 2.5;
                    PartOffset::post(Quat::from_rotation_z(-s * 0.3 * flick))
                }
            }
        }
        PartRole::Tail(k) => {
            let kf = k as f32;
            let (pitch, yaw) = match i.action {
                A::Run => (0.25 + 0.05 * (TAU * i.cycle).sin(), 0.05 * (TAU * i.cycle + kf).sin()),
                A::Walk | A::Swim => (0.0, 0.18 * (TAU * i.cycle + 0.6 * kf).sin()),
                A::Death | A::Rest => (-0.15 * ease(i.t), 0.0),
                A::Attack => (0.3, 0.0),
                _ => (0.0, 0.22 * (TAU * 0.45 * i.clock + 0.7 * kf).sin() * (0.5 + 0.5 * wander(i.clock, 6.0))),
            };
            PartOffset::post(Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch))
        }
        PartRole::Trunk(k) => {
            let kf = k as f32;
            let curl = match i.action {
                A::Eat | A::Drink => -0.42 * (0.5 + 0.5 * (TAU * 0.4 * i.t).sin()),
                A::Attack => -0.55,
                _ => 0.0,
            };
            let sway = 0.12 * (TAU * 0.3 * i.clock + 0.8 * kf).sin();
            PartOffset::post(
                Quat::from_rotation_y(0.1 * (TAU * 0.23 * i.clock + 0.5 * kf).sin())
                    * Quat::from_rotation_x(curl * ease(i.t / 0.8) + sway),
            )
        }
        _ => PartOffset::IDENTITY,
    }
}

/// Rotation of a folded wing (rest pose: spread along ±X, leading edge
/// +Z): the span points backwards and slightly down along the flank, the
/// leading edge up, and the wing surface faces outwards.
pub fn folded_wing(side: f32, droop: f32) -> Quat {
    let span = side * Vec3::new(0.0, -droop, -1.0).normalize();
    // Leading edge up and towards the midline: the folded wing covers the
    // upper flank and back.
    let up_hint = Vec3::new(-0.45 * side, 1.0, 0.0);
    let up = (up_hint - span * up_hint.dot(span)).normalize();
    let normal = up.cross(span);
    Quat::from_mat3(&bevy::math::Mat3::from_cols(span, normal, up))
}

/// How far an outer (hand) wing folds back over the inner wing when the
/// span is long compared with the body (raptors fold the hand forward over
/// the arm; small birds lay it straight back).
fn outer_fold(m: &RigMotion) -> f32 {
    if m.half_span > 0.75 * m.body_length { 2.9 } else { 0.15 }
}

fn bird(m: &RigMotion, role: PartRole, rest: &Transform, i: &PoseInput) -> PartOffset {
    use OrganismAction as A;
    let flying = matches!(i.action, A::Fly | A::Glide);
    let flap = display_flap_hz(m) > 0.0 && i.action == A::Fly;
    let (elev, down) = wingbeat(i.flap, 0.58);
    let small = m.mass_kg < 0.12;
    let lie = ease(i.t / 0.8);
    match role {
        PartRole::Root => match i.action {
            A::Death => death_root(m, rest, i.t, FRAC_PI_2),
            A::Fly | A::Glide => {
                let level = Quat::IDENTITY.slerp(rest.rotation.inverse(), 0.85);
                let bob = if flap { -0.04 * m.body_length * elev } else { 0.0 };
                let bank = if i.action == A::Glide { 0.12 * (TAU * 0.15 * i.t).sin() } else { 0.0 };
                PartOffset {
                    pre: Quat::from_rotation_z(bank) * level,
                    translation: Vec3::new(0.0, bob, 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Rest => PartOffset {
                translation: Vec3::new(0.0, -m.rest_drop * lie, 0.0),
                scale: breathing(m, i, 0.03),
                ..PartOffset::IDENTITY
            },
            A::Swim => {
                let float = rest.translation.y - 0.3 * m.half_width;
                PartOffset {
                    pre: Quat::IDENTITY.slerp(rest.rotation.inverse(), 0.7),
                    translation: Vec3::new(0.0, -float + 0.01 * m.body_length * (TAU * 0.6 * i.clock).sin(), 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Walk | A::Run if small => {
                // Small passerines hop with both feet together.
                let hop = hop_height(i.cycle, 0.45);
                PartOffset { translation: Vec3::new(0.0, 0.5 * m.hip_height * hop, 0.0), ..PartOffset::IDENTITY }
            }
            A::Walk | A::Run => {
                let waddle = 0.07 * (TAU * i.cycle).sin();
                PartOffset {
                    pre: Quat::from_rotation_z(waddle),
                    translation: Vec3::new(0.0, -0.03 * m.hip_height * (2.0 * TAU * i.cycle).cos(), 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Jump => PartOffset {
                translation: Vec3::new(0.0, 0.8 * m.hip_height * hop_height(i.t * 1.3, 0.55), 0.0),
                ..PartOffset::IDENTITY
            },
            _ => PartOffset { scale: breathing(m, i, 0.03), ..PartOffset::IDENTITY },
        },
        PartRole::Wing { side, segment, .. } => {
            let s = side.sign();
            let bat = m.locomotor == Locomotor::Bat;
            let mantle = i.action == A::Attack;
            if flap {
                let e = 0.12 + m.flap_amplitude * elev;
                match segment {
                    WingSegment::Inner => {
                        PartOffset::post(Quat::from_rotation_y(s * 0.12 * elev) * Quat::from_rotation_z(s * e))
                    }
                    WingSegment::Outer => {
                        let fold =
                            if down { 0.0 } else { (PI * (i.flap.rem_euclid(1.0) - 0.58) / 0.42).sin().max(0.0) };
                        PartOffset::post(
                            Quat::from_rotation_y(s * 0.55 * fold)
                                * Quat::from_rotation_z(-s * (0.45 * fold - 0.15 * elev)),
                        )
                    }
                }
            } else if flying || mantle {
                let dihedral = if mantle { 0.35 } else { 0.1 };
                match segment {
                    WingSegment::Inner => PartOffset::post(
                        Quat::from_rotation_z(s * dihedral)
                            * Quat::from_rotation_y(s * if mantle { 0.35 } else { 0.0 }),
                    ),
                    WingSegment::Outer => PartOffset::post(Quat::from_rotation_z(s * 0.06)),
                }
            } else {
                let open = if i.action == A::Death { 0.6 * ease(i.t) } else { 0.0 };
                match segment {
                    // Folded flight feathers overlap: the folded wing is
                    // much narrower than the spread chord.
                    WingSegment::Inner => PartOffset {
                        post: Quat::IDENTITY.slerp(folded_wing(s, if bat { 0.5 } else { 0.15 }), 1.0 - open),
                        translation: Vec3::new(s * 0.15 * m.half_width * (1.0 - open), 0.0, 0.0),
                        scale: Vec3::new(1.0, 1.0, 1.0 - 0.55 * (1.0 - open)),
                        ..PartOffset::IDENTITY
                    },
                    WingSegment::Outer => PartOffset::post(Quat::from_rotation_y(-s * outer_fold(m) * (1.0 - open))),
                }
            }
        }
        PartRole::Leg { side, segment, .. } => {
            let tuck = if flying { 1.0 } else { 0.0 };
            if tuck > 0.0 {
                return PartOffset::post(Quat::from_rotation_x(if segment == LegSegment::Upper { 1.25 } else { 0.4 }));
            }
            match i.action {
                A::Walk | A::Run | A::Swim if !small || i.action == A::Swim => {
                    let duty = stance_duty(m, i.action, i.speed, i.size);
                    let phase = i.cycle + if side == Side::Left { 0.0 } else { 0.5 };
                    let amp = sweep_amplitude(stride_at(m, i.action, i.speed, i.size), duty, m.leg_length(0) * i.size);
                    match segment {
                        LegSegment::Upper => PartOffset::post(Quat::from_rotation_x(-leg_pose(phase, duty, amp).swing)),
                        LegSegment::Lower => {
                            PartOffset::post(Quat::from_rotation_x(0.9 * leg_pose(phase, duty, amp).lift))
                        }
                    }
                }
                A::Walk | A::Run => {
                    let crouch = 1.0 - hop_height(i.cycle, 0.45);
                    PartOffset::post(Quat::from_rotation_x(
                        0.35 * crouch * if segment == LegSegment::Upper { -1.0 } else { 1.6 },
                    ))
                }
                A::Rest => {
                    PartOffset::post(Quat::from_rotation_x(lie * if segment == LegSegment::Upper { -1.0 } else { 2.2 }))
                }
                A::Death => PartOffset::post(Quat::from_rotation_x(
                    0.5 * ease(i.t) * if segment == LegSegment::Upper { 1.0 } else { 1.5 },
                )),
                _ => PartOffset::IDENTITY,
            }
        }
        PartRole::Neck(_) => {
            let pitch = match i.action {
                // Pecking: rapid dips to the ground.
                A::Eat | A::Drink => 0.9 * ease(i.t / 0.3) * (0.5 + 0.5 * (TAU * 2.4 * i.t).sin()).powf(0.5),
                A::Rest => 0.5 * lie,
                A::Attack => -0.3 + 0.6 * (TAU * 1.5 * i.t).sin().max(0.0),
                A::Death => 0.9 * ease(i.t),
                _ => 0.08 * wander(i.clock, 1.0),
            };
            // Head-bobbing walk: the head is held still, then thrust forward.
            let bob = if matches!(i.action, A::Walk | A::Run) && !small {
                Vec3::new(0.0, 0.0, 0.25 * m.leg_length(0) * ((i.cycle * 2.0).rem_euclid(1.0) - 0.5))
            } else {
                Vec3::ZERO
            };
            PartOffset {
                post: Quat::from_rotation_y(if i.action == A::Idle { 0.6 * wander(i.clock, 2.0) } else { 0.0 })
                    * Quat::from_rotation_x(pitch),
                translation: bob,
                ..PartOffset::IDENTITY
            }
        }
        PartRole::Jaw => PartOffset::post(Quat::from_rotation_x(match i.action {
            A::Attack => 0.4 * (TAU * 1.5 * i.t).sin().max(0.0),
            A::Eat => 0.15 * (TAU * 2.4 * i.t).sin().max(0.0),
            _ => 0.0,
        })),
        PartRole::Tail(_) => {
            let fan = if flying {
                0.25
            } else if i.action == A::Idle {
                0.08 * (TAU * 0.4 * i.clock).sin().max(0.0)
            } else {
                0.0
            };
            PartOffset::post(Quat::from_rotation_x(-fan))
        }
        _ => PartOffset::IDENTITY,
    }
}

#[cfg(test)]
#[path = "pose_tests.rs"]
mod tests;
