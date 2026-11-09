//! Tests of the procedural poses: every action is visibly animated for
//! every species, planted feet do not slide, folded wings lie flat.

use super::*;
use crate::procedural::parts::Detail;
use crate::procedural::species::SpeciesTable;

/// Largest difference (rad + m relative to body length) between the
/// poses of two actions over a stride and a few seconds.
fn pose_difference(rig: &crate::procedural::rig::Rig, a: OrganismAction, b: OrganismAction) -> f32 {
    let m = &rig.motion;
    let mut worst = 0.0f32;
    for step in 0..24 {
        let t = 0.25 + step as f32 * 0.21;
        let make = |action: OrganismAction| {
            let action = effective_action(m, action);
            let moving = matches!(action, OrganismAction::Walk | OrganismAction::Run | OrganismAction::Swim);
            let speed = if moving { default_speed(m, action, 1.0) } else { 0.0 };
            let cycle = cycle_frequency(m, action, speed, 1.0) * t;
            PoseInput {
                action,
                speed,
                cycle,
                flap: display_flap_hz(m) * t,
                t,
                clock: 7.0 + t,
                size: 1.0,
                wing_rest: WingRest::FoldedBack,
            }
        };
        let (ia, ib) = (make(a), make(b));
        for part in &rig.parts {
            let pa = part_offset(m, part.role, &part.rest, &ia).apply(&part.rest);
            let pb = part_offset(m, part.role, &part.rest, &ib).apply(&part.rest);
            let d = pa.rotation.angle_between(pb.rotation)
                + pa.translation.distance(pb.translation) / m.body_length.max(1e-3)
                + (pa.scale - pb.scale).abs().max_element();
            worst = worst.max(d);
        }
    }
    worst
}

/// Largest deviation of an action's pose from the rest pose.
fn motion_amount(rig: &crate::procedural::rig::Rig, action: OrganismAction) -> f32 {
    let m = &rig.motion;
    let mut worst = 0.0f32;
    for step in 0..24 {
        let t = 0.25 + step as f32 * 0.21;
        let action = effective_action(m, action);
        let moving = matches!(action, OrganismAction::Walk | OrganismAction::Run | OrganismAction::Swim);
        let speed = if moving { default_speed(m, action, 1.0) } else { 0.0 };
        let input = PoseInput {
            action,
            speed,
            cycle: cycle_frequency(m, action, speed, 1.0) * t,
            flap: display_flap_hz(m) * t,
            t,
            clock: 7.0 + t,
            size: 1.0,
            wing_rest: WingRest::FoldedBack,
        };
        for part in &rig.parts {
            let p = part_offset(m, part.role, &part.rest, &input).apply(&part.rest);
            let d = p.rotation.angle_between(part.rest.rotation)
                + p.translation.distance(part.rest.translation) / m.body_length.max(1e-3)
                + (p.scale - part.rest.scale).abs().max_element();
            worst = worst.max(d);
        }
    }
    worst
}

#[test]
fn every_action_is_visibly_animated_for_every_species() {
    let table = SpeciesTable::builtin();
    let mut missing = Vec::new();
    for spec in table.all() {
        let rig = spec.build(Detail::Full);
        for action in OrganismAction::ALL {
            if action == OrganismAction::Idle || effective_action(&rig.motion, action) == OrganismAction::Idle {
                // Idle itself, or a documented "no visible behaviour" mapping.
                continue;
            }
            let vs_idle = pose_difference(&rig, action, OrganismAction::Idle);
            let moving = motion_amount(&rig, action);
            if vs_idle < 0.05 || moving < 0.05 {
                missing.push(format!("{}:{action} (vs idle {vs_idle:.3}, motion {moving:.3})", spec.id));
            }
        }
    }
    assert!(missing.is_empty(), "actions without visible motion: {missing:?}");
}

/// Rig-space position (relative to the root) of the tip of the lower
/// segment of leg (`pair`, left) for a pose input.
fn foot_in_body_frame(rig: &crate::procedural::rig::Rig, pair: u8, input: &PoseInput) -> Vec3 {
    use crate::procedural::rig::{LegSegment, Side};
    let lower = rig
        .find(PartRole::Leg { pair, side: Side::Left, segment: LegSegment::Lower })
        .unwrap_or_else(|| panic!("no lower leg {pair}"));
    let mut chain = vec![lower];
    let mut cur = lower;
    while let Some(p) = rig.parts[cur].parent {
        if p == 0 {
            break;
        }
        chain.push(p);
        cur = p;
    }
    let model = chain.iter().rev().fold(Transform::IDENTITY, |acc, &k| {
        let part = &rig.parts[k];
        acc * part_offset(&rig.motion, part.role, &part.rest, input).apply(&part.rest)
    });
    model.transform_point(Vec3::new(0.0, -rig.motion.leg_lengths[pair as usize].1, 0.0))
}

#[test]
fn planted_feet_do_not_slide() {
    // During stance a planted foot must move backwards relative to the
    // body by the distance the body travels, stride × duty factor.
    let table = SpeciesTable::builtin();
    for id in ["ant", "beetle", "spider", "bear", "lion"] {
        let spec = table.get(id).unwrap_or_else(|| panic!("{id}"));
        let rig = spec.build(Detail::Full);
        let m = &rig.motion;
        for action in [OrganismAction::Walk, OrganismAction::Run] {
            let speed = default_speed(m, action, 1.0);
            let f = cycle_frequency(m, action, speed, 1.0);
            let duty = stance_duty(m, action, speed, 1.0);
            let phase0 = match m.locomotor {
                Locomotor::Quadruped => quadruped_phase(quadruped_gait(m, speed, 1.0), m.gallop, 0, Side::Left),
                _ => 0.0,
            };
            let at = |stride_phase: f32| {
                let cycle = stride_phase - phase0;
                let input = PoseInput {
                    action,
                    speed,
                    cycle,
                    flap: 0.0,
                    t: 3.0,
                    clock: 1.0,
                    size: 1.0,
                    wing_rest: WingRest::FoldedBack,
                };
                foot_in_body_frame(&rig, 0, &input)
            };
            let (start, end) = (at(0.001), at(duty - 0.001));
            let travelled = start.z - end.z;
            let expected = speed / f * duty;
            assert!(
                (travelled / expected - 1.0).abs() < 0.1,
                "{id} {action}: foot moved {travelled} m during stance, body {expected} m"
            );
        }
    }
}

#[test]
fn fore_legs_reach_forward_and_hind_legs_back() {
    use crate::procedural::rig::LegSegment;
    let table = SpeciesTable::builtin();
    for id in ["ant", "spider", "beetle", "grasshopper"] {
        let spec = table.get(id).unwrap_or_else(|| panic!("{id}"));
        let rig = spec.build(Detail::Full);
        let pairs = rig.motion.leg_lengths.len() as u8;
        let tip_minus_coxa = |pair: u8| {
            let u = rig.find(PartRole::Leg { pair, side: Side::Left, segment: LegSegment::Upper }).unwrap();
            let l = rig.find(PartRole::Leg { pair, side: Side::Left, segment: LegSegment::Lower }).unwrap();
            let tip = rig.rest_model_transform(l).transform_point(Vec3::new(
                0.0,
                -rig.motion.leg_lengths[pair as usize].1,
                0.0,
            ));
            tip - rig.rest_model_transform(u).translation
        };
        assert!(tip_minus_coxa(0).z > 0.0, "{id}: fore legs must point forwards");
        assert!(tip_minus_coxa(pairs - 1).z < 0.0, "{id}: hind legs must point backwards");
    }
}

#[test]
fn fast_short_legged_animals_shorten_strides_instead_of_sliding() {
    let rig = SpeciesTable::builtin().get("spider").unwrap().build(Detail::Full);
    let m = &rig.motion;
    let v = default_speed(m, OrganismAction::Run, 1.0);
    let natural = stride_length(v, m.hip_height);
    let drawn = stride_at(m, OrganismAction::Run, v, 1.0);
    assert!(drawn < natural, "the spider's reach limits its stride");
    let f = cycle_frequency(m, OrganismAction::Run, v, 1.0);
    assert!((f * drawn - v).abs() < 1e-4 * v.max(1.0), "speed = stride × frequency");
    // Large animals walk with Alexander's stride unchanged.
    let bear = SpeciesTable::builtin().get("bear").unwrap().build(Detail::Full);
    let vb = default_speed(&bear.motion, OrganismAction::Walk, 1.0);
    assert_eq!(stride_at(&bear.motion, OrganismAction::Walk, vb, 1.0), stride_length(vb, bear.motion.hip_height));
}

#[test]
fn idle_mappings_are_limited_to_documented_cases() {
    let table = SpeciesTable::builtin();
    for spec in table.all() {
        let rig = spec.build(Detail::Low);
        for action in OrganismAction::ALL {
            if action != OrganismAction::Idle && effective_action(&rig.motion, action) == OrganismAction::Idle {
                let documented = action == OrganismAction::Drink
                    && (rig.motion.aquatic || rig.motion.locomotor == Locomotor::Peristaltic);
                assert!(documented, "{}: {action} is drawn as idle", spec.id);
            }
        }
    }
}

#[test]
fn folded_wings_lie_along_the_flanks() {
    for side in [1.0f32, -1.0] {
        let q = folded_wing(side, 0.15);
        // The span (side·X in the rest pose) points backwards.
        assert!((q * (side * Vec3::X)).z < -0.9);
        // The leading edge (+Z) points up; the surface faces outwards.
        assert!((q * Vec3::Z).y > 0.8);
        assert!(((q * Vec3::NEG_Y).x * side) > 0.9 || ((q * Vec3::Y).x * side) > 0.9);
    }
}
