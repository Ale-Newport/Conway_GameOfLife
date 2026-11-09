//! Procedural poses of arthropods, fish, snakes and worms, frogs, turtles
//! and snails (see [`super::pose`] for the shared types).

use std::f32::consts::{PI, TAU};

use bevy::math::{Quat, Vec3};
use bevy::prelude::Transform;

use super::arthropod::WingRest;
use super::gait::{
    duty_factor, hexapod_phase, hop_height, leg_pose, octopod_phase, quadruped_phase, sweep_amplitude,
    undulation_angle, wingbeat,
};
use super::herp::{FROG_PITCH, frog_hind_directions, frog_hind_rest};
use super::pose::{PartOffset, PoseInput, breathing, death_root, ease, stance_duty, stride_at, wander};
use super::rig::{GallopStyle, LegSegment, Locomotor, PartRole, RigMotion};
use crate::action::{Gait, OrganismAction};

/// Insects and spiders: tripod / tetrapod gait, wing beats, antenna motion,
/// sting, and the on-the-back death pose.
pub(super) fn arthropod(m: &RigMotion, role: PartRole, rest: &Transform, i: &PoseInput) -> PartOffset {
    use OrganismAction as A;
    let flying = matches!(i.action, A::Fly | A::Glide) && m.can_fly;
    match role {
        PartRole::Root => match i.action {
            A::Death => {
                // Dead insects end on their backs.
                let k = ease(i.t / 0.6);
                PartOffset {
                    pre: Quat::from_rotation_z(PI * k),
                    translation: Vec3::new(0.0, -0.4 * rest.translation.y * k, 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Rest => {
                PartOffset { translation: Vec3::new(0.0, -m.rest_drop * ease(i.t / 0.6), 0.0), ..PartOffset::IDENTITY }
            }
            A::Fly | A::Glide if flying => PartOffset {
                pre: Quat::from_rotation_x(-0.15),
                translation: Vec3::new(0.0, 0.05 * m.body_length * (TAU * 1.3 * i.clock).sin(), 0.0),
                ..PartOffset::IDENTITY
            },
            A::Walk | A::Run => {
                PartOffset { pre: Quat::from_rotation_y(0.04 * (TAU * i.cycle).sin()), ..PartOffset::IDENTITY }
            }
            A::Jump => PartOffset {
                translation: Vec3::new(0.0, 4.0 * m.body_length * hop_height(i.t * 1.2, 0.6), 0.0),
                ..PartOffset::IDENTITY
            },
            _ => PartOffset::IDENTITY,
        },
        PartRole::Leg { pair, side, segment } => {
            let s = side.sign();
            match i.action {
                A::Death => {
                    let k = ease(i.t / 0.6);
                    PartOffset::post(Quat::from_rotation_z(
                        s * if segment == LegSegment::Upper { 0.6 } else { 0.9 } * k,
                    ))
                }
                _ if flying => {
                    PartOffset::post(Quat::from_rotation_z(-s * if segment == LegSegment::Upper { 0.5 } else { 0.4 }))
                }
                A::Walk | A::Run | A::Swim => {
                    let phase = i.cycle
                        + if m.locomotor == Locomotor::Octopod {
                            octopod_phase(pair, side)
                        } else {
                            hexapod_phase(pair, side)
                        };
                    let duty = stance_duty(m, i.action, i.speed, i.size);
                    // The leg yaws about the coxa, so the foot's fore-aft
                    // travel is its lateral reach times the swing angle's
                    // chord; during stance it must cover the body's travel
                    // λ·β or the feet would slide.
                    let reach = m.swing_lever(pair) * i.size;
                    let stride = stride_at(m, i.action, i.speed, i.size);
                    let amp = sweep_amplitude(stride, duty, reach);
                    let p = leg_pose(phase, duty, amp);
                    match segment {
                        LegSegment::Upper => PartOffset {
                            pre: Quat::from_rotation_y(-s * p.swing),
                            post: Quat::from_rotation_z(s * 0.35 * p.lift),
                            ..PartOffset::IDENTITY
                        },
                        LegSegment::Lower => PartOffset::post(Quat::from_rotation_z(-s * 0.2 * p.lift)),
                    }
                }
                A::Jump if pair + 1 == m.leg_lengths.len() as u8 => {
                    let kick = hop_height(i.t * 1.2, 0.6);
                    PartOffset::post(Quat::from_rotation_z(
                        s * if segment == LegSegment::Upper { -0.5 } else { 1.2 } * kick,
                    ))
                }
                A::Rest => PartOffset::post(Quat::from_rotation_z(-s * 0.25 * ease(i.t / 0.6))),
                _ => PartOffset::IDENTITY,
            }
        }
        PartRole::Wing { pair, side, .. } => {
            let s = side.sign();
            if flying && i.action == A::Fly {
                let (elev, _) = wingbeat(i.flap + 0.25 * pair as f32, 0.5);
                let e = match i.wing_rest {
                    WingRest::Upright => 0.55 + 0.9 * elev,
                    _ => 0.1 + m.flap_amplitude * elev,
                };
                PartOffset::post(Quat::from_rotation_z(s * e))
            } else if flying {
                PartOffset::post(Quat::from_rotation_z(s * if i.wing_rest == WingRest::Upright { 0.25 } else { 0.05 }))
            } else {
                match i.wing_rest {
                    WingRest::FoldedBack => {
                        PartOffset::post(Quat::from_rotation_y(s * 1.3) * Quat::from_rotation_z(s * 0.08))
                    }
                    WingRest::Upright => PartOffset::post(Quat::from_rotation_z(
                        s * (1.45 - 0.08 * (TAU * 0.25 * i.clock).sin().max(0.0)),
                    )),
                    WingRest::Spread => PartOffset::IDENTITY,
                }
            }
        }
        PartRole::Antenna(side) => {
            let s = side.sign();
            PartOffset::post(
                Quat::from_rotation_x(0.25 * (TAU * 0.8 * i.clock + s).sin())
                    * Quat::from_rotation_y(s * 0.15 * wander(i.clock * 2.0, s)),
            )
        }
        PartRole::Head => match i.action {
            A::Eat | A::Drink => PartOffset::post(Quat::from_rotation_x(0.2 + 0.12 * (TAU * 3.0 * i.t).sin())),
            A::Attack => PartOffset::post(Quat::from_rotation_x(-0.15 * (TAU * 2.0 * i.t).sin())),
            _ => PartOffset::IDENTITY,
        },
        PartRole::Segment(_) => match i.action {
            // Abdomen curls under to sting / pumps while breathing.
            A::Attack => PartOffset::post(Quat::from_rotation_x(0.7 * (TAU * 1.5 * i.t).sin().max(0.0))),
            _ => PartOffset {
                scale: Vec3::new(1.0, 1.0 + 0.04 * (TAU * 1.2 * i.clock).sin(), 1.0),
                ..PartOffset::IDENTITY
            },
        },
        _ => PartOffset::IDENTITY,
    }
}

/// Arc position (0 = snout) of the joint at the front of segment `k`.
fn segment_position(m: &RigMotion, k: u8) -> f32 {
    let n = m.segments.max(2) as f32;
    match m.locomotor {
        Locomotor::Fish => 0.32 + 0.68 * (k as f32 - 1.0) / (n - 1.0).max(1.0),
        _ => k as f32 / n,
    }
}

/// Fraction of the full undulation (or ray fin-flap) amplitude shown for an
/// action: full while swimming, reduced while hovering, very small at rest.
fn wave_fraction(action: OrganismAction) -> f32 {
    use OrganismAction as A;
    match action {
        A::Swim | A::Walk | A::Run | A::Attack => 1.0,
        A::Death => 0.0,
        A::Rest => 0.12,
        _ => 0.4,
    }
}

fn wave_amplitude(m: &RigMotion, i: &PoseInput) -> f32 {
    m.wave_amplitude * wave_fraction(i.action)
}

/// Fish, cetaceans and rays: travelling-wave undulation, pectoral sculling
/// or ray-wing flapping, belly-up death.
pub(super) fn fish(m: &RigMotion, role: PartRole, _rest: &Transform, i: &PoseInput) -> PartOffset {
    use OrganismAction as A;
    let uniform = if m.wave_length < 0.8 { 1.0 } else { 0.0 };
    let amp = wave_amplitude(m, i);
    let theta = |s: f32| undulation_angle(s, i.cycle, m.wave_length, amp, uniform);
    // Cetaceans (horizontal flukes) undulate dorsoventrally.
    let bend = |a: f32| if m.vertical_undulation { Quat::from_rotation_x(a) } else { Quat::from_rotation_y(a) };
    match role {
        PartRole::Root => match i.action {
            A::Death => {
                let k = ease(i.t / 1.5);
                PartOffset {
                    pre: Quat::from_rotation_z(PI * k),
                    translation: Vec3::new(
                        0.0,
                        -(0.05 * m.body_length * (i.t - 1.5).max(0.0)).min(1.5 * m.body_length),
                        0.0,
                    ),
                    ..PartOffset::IDENTITY
                }
            }
            A::Jump => {
                // Breach: nose up on the way out, nose down on re-entry.
                let p = (i.t * 0.8).rem_euclid(1.0);
                let pitch = if p < 0.7 { -0.6 * (PI * p / 0.7).cos() } else { 0.0 };
                PartOffset {
                    pre: Quat::from_rotation_x(pitch),
                    translation: Vec3::new(0.0, 0.8 * m.body_length * hop_height(p, 0.7), 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Attack => {
                let lunge = (TAU * 0.8 * i.t).sin().max(0.0).powi(2);
                PartOffset {
                    pre: bend(-0.3 * theta(0.0)),
                    translation: Vec3::new(0.0, 0.0, 0.3 * m.body_length * lunge),
                    ..PartOffset::IDENTITY
                }
            }
            A::Eat => PartOffset {
                pre: Quat::from_rotation_x(0.25 * ease(i.t)) * bend(-0.3 * theta(0.0)),
                ..PartOffset::IDENTITY
            },
            // Resting fish settle lower, nose slightly down, almost still.
            A::Rest => PartOffset {
                pre: Quat::from_rotation_x(0.08 * ease(i.t / 2.0)) * bend(-0.3 * theta(0.0)),
                translation: Vec3::new(0.0, -0.15 * m.body_length * ease(i.t / 2.0), 0.0),
                ..PartOffset::IDENTITY
            },
            _ => PartOffset { pre: bend(-0.3 * theta(0.0)), ..PartOffset::IDENTITY },
        },
        PartRole::Segment(k) => {
            // Joint angle = absolute wave angle here minus the parent's
            // (the head carries a small recoil of −0.3·θ(0)).
            let s1 = segment_position(m, k);
            let rel = if k <= 1 { theta(s1) + 0.3 * theta(0.0) } else { theta(s1) - theta(segment_position(m, k - 1)) };
            PartOffset::post(bend(rel))
        }
        PartRole::Fin(side) => {
            let s = side.sign();
            if m.flap_hz > 0.0 {
                // Rays fly through the water with their pectoral fins: full
                // strokes when swimming, a gentle ripple when hovering.
                let f = m.flap_amplitude * wave_fraction(i.action) * (TAU * i.flap).sin();
                PartOffset::post(Quat::from_rotation_z(s * f))
            } else {
                let scull = match i.action {
                    A::Swim | A::Attack => -0.35,
                    A::Death => 0.6,
                    // Fins spread as brakes and stabilisers.
                    A::Rest => 0.45 + 0.08 * (TAU * 0.4 * i.clock).sin(),
                    _ => 0.3 * (TAU * 1.4 * i.clock).sin(),
                };
                PartOffset::post(Quat::from_rotation_y(s * scull))
            }
        }
        _ => PartOffset::IDENTITY,
    }
}

/// Gaussian bulge of half-width `w` centred at arc position `c`.
fn bulge(s: f32, c: f32, w: f32) -> f32 {
    (-((s - c) / w).powi(2)).exp()
}

/// Snakes (lateral undulation, coiling at rest, strike, swallowing prey,
/// drinking) and earthworms (peristaltic contraction waves, probing the
/// soil while feeding, the giant-fibre escape contraction).
///
/// The root is the head and the body segments hang behind it, so head-only
/// motions rotate the root and counter-rotate the first segment, keeping
/// the body in place.
pub(super) fn serpent(m: &RigMotion, role: PartRole, _rest: &Transform, i: &PoseInput) -> PartOffset {
    use OrganismAction as A;
    let worm = m.locomotor == Locomotor::Peristaltic;
    let amp = match i.action {
        // Feeding and drinking animals lie still apart from the head.
        A::Eat | A::Drink => 0.15 * m.wave_amplitude,
        _ => wave_amplitude(m, i),
    };
    let theta = |s: f32| undulation_angle(s, i.cycle, m.wave_length, amp, 1.0);
    let n = m.segments.max(2) as f32;
    // Head-only rotation (pitch, yaw) for feeding, drinking and probing.
    let head = match i.action {
        A::Eat if worm => (0.25 + 0.15 * (TAU * 0.5 * i.t).sin(), 0.45 * (TAU * 0.35 * i.t).sin()),
        // Swallowing: the jaws "walk" over the prey with alternating nods.
        A::Eat => (0.12 * (TAU * 0.8 * i.t).sin(), 0.1 * (TAU * 0.4 * i.t).sin()),
        // Head lowered to the water, throat pumping.
        A::Drink => (0.18 * ease(i.t / 0.6) + 0.04 * (TAU * 1.6 * i.t).sin(), 0.0),
        _ => (0.0, 0.0),
    };
    let head_q = Quat::from_rotation_y(head.1) * Quat::from_rotation_x(head.0);
    // Escape contraction of earthworms: fast shortening and thickening.
    let contract = if worm && matches!(i.action, A::Attack | A::Jump) { ease(i.t / 0.15) } else { 0.0 };
    match role {
        PartRole::Root => match i.action {
            A::Death => {
                let k = ease(i.t / 1.0);
                PartOffset { pre: Quat::from_rotation_z(PI * k), ..PartOffset::IDENTITY }
            }
            A::Attack | A::Jump if !worm => {
                let strike = (TAU * 0.7 * i.t).sin().max(0.0).powi(3);
                PartOffset {
                    pre: Quat::from_rotation_x(-0.5 * (1.0 - strike)),
                    translation: Vec3::new(
                        0.0,
                        2.0 * m.hip_height * (1.0 - strike),
                        0.6 * m.body_length * 0.25 * strike,
                    ),
                    ..PartOffset::IDENTITY
                }
            }
            A::Eat | A::Drink => PartOffset { pre: head_q, ..PartOffset::IDENTITY },
            _ => PartOffset { pre: Quat::from_rotation_y(-0.3 * theta(0.0)), ..PartOffset::IDENTITY },
        },
        PartRole::Segment(k) => {
            let s1 = k as f32 / n;
            let s0 = (k as f32 - 1.0) / n;
            let mut yaw = if k == 1 { theta(s1) + 0.3 * theta(0.0) } else { theta(s1) - theta(s0) };
            let mut pitch = 0.0;
            let mut counter = Quat::IDENTITY;
            let mut swell = 0.0;
            match i.action {
                A::Rest => yaw += ease(i.t / 2.0) * TAU * 1.15 / n,
                A::Attack | A::Jump if !worm && k <= 3 => {
                    let strike = (TAU * 0.7 * i.t).sin().max(0.0).powi(3);
                    yaw = if k % 2 == 0 { 0.9 } else { -0.9 } * (1.0 - strike);
                    pitch = if k == 1 { 0.6 * (1.0 - strike) } else { 0.0 };
                }
                A::Eat | A::Drink => {
                    if k == 1 {
                        counter = head_q.inverse();
                        yaw = 0.0;
                    }
                    if !worm && i.action == A::Eat {
                        // The swallowed prey travels down the body as a bulge.
                        let c = 0.08 + 0.55 * ((0.12 * i.t).rem_euclid(1.0));
                        swell = 0.45 * bulge(s1, c, 0.09);
                    }
                }
                A::Death => yaw *= 1.0 - ease(i.t),
                _ => {}
            }
            let scale = if worm {
                // Radial swelling travelling backwards, with axial shortening;
                // the escape reflex shortens every segment at once.
                let c = if contract > 0.0 { 0.0 } else { (TAU * (i.cycle - s1 / m.wave_length)).sin() };
                Vec3::new(
                    1.0 + 0.22 * c + 0.3 * contract,
                    1.0 + 0.22 * c + 0.3 * contract,
                    (1.0 - 0.18 * c) * (1.0 - 0.4 * contract),
                )
            } else {
                Vec3::new(1.0 + swell, 1.0 + swell, 1.0)
            };
            PartOffset {
                pre: counter,
                post: Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch),
                scale,
                ..PartOffset::IDENTITY
            }
        }
        PartRole::Jaw => {
            // Tongue flicks every couple of seconds (kept in while eating
            // or drinking).
            let phase = (i.clock * 0.6).rem_euclid(1.0);
            let flick =
                if phase < 0.15 && !matches!(i.action, A::Eat | A::Drink) { (PI * phase / 0.15).sin() } else { 0.0 };
            PartOffset {
                translation: Vec3::new(0.0, 0.0, -0.8 * m.hip_height * (1.0 - flick)),
                scale: Vec3::splat(0.2 + 0.8 * flick),
                ..PartOffset::IDENTITY
            }
        }
        _ => PartOffset::IDENTITY,
    }
}

/// Frogs: hop with synchronous hind-leg extension, frog-kick swimming.
pub(super) fn frog(m: &RigMotion, role: PartRole, rest: &Transform, i: &PoseInput) -> PartOffset {
    use OrganismAction as A;
    let hopping = matches!(i.action, A::Walk | A::Run | A::Jump);
    let hop_phase = if i.action == A::Jump { (i.t * 1.2).min(0.999) } else { i.cycle };
    let air = 0.55;
    let hop = if hopping { hop_height(hop_phase, air) } else { 0.0 };
    // Leg extension: fast push-off at the start of the hop, folding during flight.
    let extend = if i.action == A::Swim {
        (0.5 + 0.5 * (TAU * i.cycle).sin()).powi(2)
    } else if hopping {
        let p = hop_phase.rem_euclid(1.0);
        if p < air { (1.0 - p / air).powf(1.5) } else { 0.0 }
    } else {
        0.0
    };
    // Tongue strike: a fast forward lunge with the snout dipping onto the
    // prey (every ~1.4 s while feeding, once per ~0.9 s when attacking).
    let strike = |period: f32| {
        let p = (i.t / period).rem_euclid(1.0);
        if p < 0.25 { (PI * p / 0.25).sin() } else { 0.0 }
    };
    match role {
        PartRole::Root => match i.action {
            A::Death => death_root(m, rest, i.t, PI),
            A::Swim => PartOffset {
                pre: Quat::from_rotation_x(0.3),
                translation: Vec3::new(0.0, -rest.translation.y, 0.0),
                ..PartOffset::IDENTITY
            },
            A::Rest => PartOffset {
                translation: Vec3::new(0.0, -m.rest_drop * ease(i.t), 0.0),
                scale: breathing(m, i, 0.04),
                ..PartOffset::IDENTITY
            },
            A::Eat | A::Attack => {
                let (k, reach) = if i.action == A::Eat { (strike(1.4), 0.15) } else { (strike(0.9), 0.3) };
                // Swallowing squeezes the body briefly after each catch.
                let after = ((i.t / if i.action == A::Eat { 1.4 } else { 0.9 }).rem_euclid(1.0) - 0.25).max(0.0);
                let gulp = if after < 0.3 { (PI * after / 0.3).sin() } else { 0.0 };
                PartOffset {
                    pre: Quat::from_rotation_x(0.22 * k),
                    translation: Vec3::new(0.0, -0.1 * m.body_length * k, reach * m.body_length * k),
                    scale: Vec3::new(1.0, 1.0 - 0.06 * gulp, 1.0),
                    ..PartOffset::IDENTITY
                }
            }
            // Frogs do not drink by mouth: they press the vascular "seat
            // patch" of the belly onto wet ground and absorb water.
            A::Drink => PartOffset {
                pre: Quat::from_rotation_x(0.5 * FROG_PITCH * ease(i.t / 0.8)),
                translation: Vec3::new(0.0, -1.6 * m.rest_drop * ease(i.t / 0.8), 0.0),
                scale: Vec3::new(1.06, 0.9, 1.0) * breathing(m, i, 0.02),
                ..PartOffset::IDENTITY
            },
            _ if hopping => PartOffset {
                pre: Quat::from_rotation_x(-0.35 * hop),
                translation: Vec3::new(0.0, 1.6 * m.body_length * hop, 0.0),
                ..PartOffset::IDENTITY
            },
            // Throat / flank pumping.
            _ => PartOffset { scale: breathing(m, i, 0.05), ..PartOffset::IDENTITY },
        },
        PartRole::Leg { pair: 1, side, segment } => {
            // Unfold the Z-folded leg towards the straight take-off pose.
            let s = side.sign();
            let [d1, d2, e1, e2] = frog_hind_directions(s);
            let r1 = Quat::IDENTITY.slerp(Quat::from_rotation_arc(d1, e1), extend);
            let r2 = Quat::IDENTITY.slerp(Quat::from_rotation_arc(d2, e2), extend);
            let (_, q2) = frog_hind_rest(s);
            let root = Quat::from_rotation_x(-FROG_PITCH);
            match segment {
                LegSegment::Upper => PartOffset { pre: root.inverse() * r1 * root, ..PartOffset::IDENTITY },
                LegSegment::Lower => PartOffset::post(q2.inverse() * r1.inverse() * r2 * q2),
            }
        }
        PartRole::Leg { pair: 0, segment, .. } if hopping => {
            PartOffset::post(Quat::from_rotation_x(if segment == LegSegment::Upper { -0.7 * hop } else { 0.4 * hop }))
        }
        // Fore legs brace the lunge.
        PartRole::Leg { pair: 0, segment, .. } if matches!(i.action, A::Eat | A::Attack) => {
            let k = if i.action == A::Eat { strike(1.4) } else { strike(0.9) };
            PartOffset::post(Quat::from_rotation_x(if segment == LegSegment::Upper { -0.5 * k } else { 0.3 * k }))
        }
        _ => PartOffset::IDENTITY,
    }
}

/// Turtles: slow ambling walk with the head stretched out, flipper strokes
/// in water, withdrawal into the shell at rest, overturned when dead.
pub(super) fn turtle(m: &RigMotion, role: PartRole, rest: &Transform, i: &PoseInput) -> PartOffset {
    use OrganismAction as A;
    let retract = if i.action == A::Rest || i.action == A::Death { ease(i.t / 1.0) } else { 0.0 };
    match role {
        PartRole::Root => match i.action {
            A::Death => {
                let k = ease(i.t / 1.2);
                PartOffset {
                    pre: Quat::from_rotation_z(PI * k),
                    translation: Vec3::new(0.0, -0.4 * rest.translation.y * k, 0.0),
                    ..PartOffset::IDENTITY
                }
            }
            A::Rest => PartOffset { translation: Vec3::new(0.0, -m.rest_drop * retract, 0.0), ..PartOffset::IDENTITY },
            A::Swim => PartOffset { translation: Vec3::new(0.0, -rest.translation.y, 0.0), ..PartOffset::IDENTITY },
            _ => PartOffset::IDENTITY,
        },
        PartRole::Neck(_) => {
            let reach = match i.action {
                A::Walk | A::Run | A::Swim => 0.4,
                A::Eat | A::Drink => 0.5,
                A::Attack => 0.6 * (TAU * 1.0 * i.t).sin().max(0.0),
                _ => 0.0,
            };
            let pitch = match i.action {
                A::Eat | A::Drink => m.graze_pitch * ease(i.t / 0.8) + 0.1 * (TAU * 1.5 * i.t).sin(),
                A::Idle => 0.15 * wander(i.clock, 1.0),
                _ => 0.0,
            };
            let len = m.body_length * 0.25;
            PartOffset {
                post: Quat::from_rotation_y(if i.action == A::Idle { 0.4 * wander(i.clock, 2.0) } else { 0.0 })
                    * Quat::from_rotation_x(pitch),
                translation: Vec3::new(0.0, 0.0, len * (reach - 0.9 * retract)),
                ..PartOffset::IDENTITY
            }
        }
        PartRole::Leg { pair, side, segment } => {
            if retract > 0.0 {
                return PartOffset { scale: Vec3::splat(1.0 - 0.6 * retract), ..PartOffset::IDENTITY };
            }
            match i.action {
                A::Swim => {
                    let s = side.sign();
                    let stroke = (TAU * i.cycle + if pair == 0 { 0.0 } else { PI }).sin();
                    PartOffset::post(Quat::from_rotation_x(0.6 * stroke) * Quat::from_rotation_z(s * 0.3 * stroke))
                }
                A::Walk | A::Run => {
                    let gait = Gait::Walk;
                    let phase = i.cycle + quadruped_phase(gait, GallopStyle::Amble, pair, side);
                    let duty = duty_factor(gait, GallopStyle::Amble);
                    let amp =
                        sweep_amplitude(stride_at(m, i.action, i.speed, i.size), duty, m.leg_length(pair) * i.size);
                    let p = leg_pose(phase, duty, amp);
                    match segment {
                        LegSegment::Upper => PartOffset::post(Quat::from_rotation_x(-p.swing)),
                        LegSegment::Lower => {
                            PartOffset::post(Quat::from_rotation_x(if pair == 0 { 0.5 } else { -0.5 } * p.lift))
                        }
                    }
                }
                _ => PartOffset::IDENTITY,
            }
        }
        _ => PartOffset::IDENTITY,
    }
}

/// Snails: the foot stretches and carries pedal waves while crawling and
/// contracts into the shell when withdrawing (rest, death, alarm); eye
/// stalks wave, point forward while crawling and lower towards food.
pub(super) fn snail(_m: &RigMotion, role: PartRole, _rest: &Transform, i: &PoseInput) -> PartOffset {
    use OrganismAction as A;
    let withdraw = if matches!(i.action, A::Rest | A::Death | A::Attack | A::Jump) { ease(i.t / 1.5) } else { 0.0 };
    let crawling = matches!(i.action, A::Walk | A::Run | A::Swim);
    let feeding = matches!(i.action, A::Eat | A::Drink);
    match role {
        PartRole::Root => match i.action {
            // Rasping with the radula: the front of the animal dips rhythmically.
            _ if feeding => PartOffset::post(Quat::from_rotation_x(
                0.06 * ease(i.t / 0.8) + 0.02 * (TAU * 0.9 * i.t).sin().max(0.0),
            )),
            // A dead snail's shell rolls onto its side.
            A::Death => PartOffset::post(Quat::from_rotation_z(0.9 * ease((i.t - 1.5) / 2.0))),
            _ => PartOffset::IDENTITY,
        },
        PartRole::Segment(_) => {
            let stretch = if crawling { 0.12 + 0.05 * (TAU * i.cycle).sin() } else { 0.0 };
            PartOffset {
                scale: Vec3::new(
                    1.0 - 0.25 * withdraw,
                    (1.0 - 0.05 * stretch) * (1.0 - 0.3 * withdraw),
                    (1.0 + stretch) * (1.0 - 0.55 * withdraw),
                ),
                ..PartOffset::IDENTITY
            }
        }
        PartRole::Antenna(side) => {
            let s = side.sign();
            let len = (1.0 - 0.9 * withdraw) * (0.85 + 0.15 * wander(i.clock, s));
            let forward = if crawling {
                0.45
            } else if feeding {
                0.9 * ease(i.t / 0.8)
            } else {
                0.0
            };
            PartOffset {
                post: Quat::from_rotation_x(forward + 0.25 * wander(i.clock * 1.5, 3.0 + s))
                    * Quat::from_rotation_z(-s * 0.2 * wander(i.clock, 5.0 + s)),
                scale: Vec3::splat(len.max(0.05)),
                ..PartOffset::IDENTITY
            }
        }
        _ => PartOffset::IDENTITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(action: OrganismAction, cycle: f32) -> PoseInput {
        PoseInput {
            action,
            speed: 0.5,
            cycle,
            flap: cycle,
            t: 1.0,
            clock: 3.0,
            size: 1.0,
            wing_rest: WingRest::FoldedBack,
        }
    }

    fn fish_motion() -> RigMotion {
        RigMotion {
            locomotor: Locomotor::Fish,
            gallop: GallopStyle::Bound,
            mass_kg: 1.0,
            hip_height: 0.05,
            body_length: 0.4,
            rest_drop: 0.0,
            leg_lengths: vec![],
            leg_reach: Vec::new(),
            half_width: 0.02,
            half_span: 0.0,
            flap_hz: 0.0,
            flap_amplitude: 0.0,
            can_fly: false,
            aquatic: true,
            graze_pitch: 0.35,
            segments: 4,
            wave_length: 1.0,
            wave_amplitude: 0.4,
            vertical_undulation: false,
        }
    }

    #[test]
    fn fish_tail_swings_more_than_front_segments() {
        let m = fish_motion();
        let amp = |k: u8| {
            (0..40)
                .map(|c| {
                    let o = fish(
                        &m,
                        PartRole::Segment(k),
                        &Transform::IDENTITY,
                        &input(OrganismAction::Swim, c as f32 / 40.0),
                    );
                    o.post.to_euler(bevy::math::EulerRot::YXZ).0.abs()
                })
                .fold(0.0f32, f32::max)
        };
        assert!(amp(3) > amp(1), "{} vs {}", amp(3), amp(1));
    }

    #[test]
    fn dead_insect_lies_on_its_back() {
        let mut m = fish_motion();
        m.locomotor = Locomotor::Hexapod;
        let mut i = input(OrganismAction::Death, 0.0);
        i.t = 5.0;
        let o = arthropod(&m, PartRole::Root, &Transform::from_xyz(0.0, 0.01, 0.0), &i);
        let up = o.pre * Vec3::Y;
        assert!(up.y < -0.99, "{up}");
    }

    #[test]
    fn frog_extends_legs_at_take_off() {
        let mut m = fish_motion();
        m.locomotor = Locomotor::Anuran;
        let take_off = frog(
            &m,
            PartRole::Leg { pair: 1, side: crate::procedural::rig::Side::Left, segment: LegSegment::Lower },
            &Transform::IDENTITY,
            &input(OrganismAction::Walk, 0.02),
        );
        let landed = frog(
            &m,
            PartRole::Leg { pair: 1, side: crate::procedural::rig::Side::Left, segment: LegSegment::Lower },
            &Transform::IDENTITY,
            &input(OrganismAction::Walk, 0.8),
        );
        assert!(take_off.post.angle_between(Quat::IDENTITY) > 1.0);
        assert!(landed.post.angle_between(Quat::IDENTITY) < 1e-3);
    }

    #[test]
    fn display_flap_is_capped() {
        let mut m = fish_motion();
        m.flap_hz = 230.0;
        assert_eq!(crate::procedural::pose::display_flap_hz(&m), crate::procedural::pose::DISPLAY_FLAP_LIMIT_HZ);
    }
}
