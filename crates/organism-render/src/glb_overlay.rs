//! Procedural motion layered on authored GLB models for actions the model
//! has no clip for (see [`crate::library`] for the fallback rules):
//! whole-body ("root") offsets and bone-level overrides on rigs whose leg,
//! neck and head joints follow the Quaternius naming convention
//! (`FrontUpLeg.L`, `BackLowLeg.R`, `Neck`, `Head`...).
//!
//! Bone rotations are expressed about the model's lateral axis and
//! converted into each joint's local frame with its parent's rest
//! orientation in model space: `local' = P⁻¹ · R(axis, θ) · P · rest`.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::math::{Quat, Vec3};
use bevy::prelude::{Entity, Transform};

use crate::action::{GRAVITY, Gait, froude_number, gait_for_froude, stride_length};
use crate::library::MotionOverlay;
use crate::manifest::ModelEntry;
use crate::procedural::gait::{duty_factor, hop_height, leg_pose, quadruped_phase, sweep_amplitude};
use crate::procedural::pose::ease;
use crate::procedural::rig::{GallopStyle, Side};

/// Real-world dimensions of a model (adult, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayDims {
    /// Standing height.
    pub height: f32,
    /// Width.
    pub width: f32,
    /// Length.
    pub length: f32,
    /// Hip height (60 % of the height).
    pub hip: f32,
}

impl OverlayDims {
    /// Dimensions from the manifest's `real_size_m`, oriented by `forward`.
    pub fn from_model(model: &ModelEntry) -> Self {
        let [x, y, z] = model.real_size_m;
        let (width, length) = if model.forward.contains('X') { (z, x) } else { (x, z) };
        Self { height: y, width, length, hip: 0.6 * y }
    }
}

/// Model-space forward axis from the manifest `forward` field.
pub fn forward_axis(forward: &str) -> Vec3 {
    match forward.trim() {
        "-Z" => Vec3::NEG_Z,
        "+X" => Vec3::X,
        "-X" => Vec3::NEG_X,
        _ => Vec3::Z,
    }
}

/// Whole-body offset (organism frame: +Z forward, +Y up) for the overlays
/// of a plan. `t` is the time in the action, `cycle` the locomotion cycle,
/// `altitude` the organism's height above the ground (falls end there),
/// and `bones` whether neck/head joints are available (else head dips are
/// shown by pitching the whole body).
pub fn root_overlay(
    overlays: &[MotionOverlay],
    t: f32,
    cycle: f32,
    dims: &OverlayDims,
    altitude: f32,
    bones: bool,
) -> Transform {
    let mut translation = Vec3::ZERO;
    let mut rotation = Quat::IDENTITY;
    for overlay in overlays {
        match overlay {
            MotionOverlay::TipOverAndSink => {
                let k = ease(t / 0.8);
                rotation = Quat::from_rotation_z(FRAC_PI_2 * k) * rotation;
                translation += Vec3::new(
                    0.5 * dims.height * k,
                    0.5 * dims.width * k - 0.35 * dims.width * ease((t - 3.0) / 6.0),
                    0.0,
                );
            }
            MotionOverlay::BellyUpAndSink => {
                rotation = Quat::from_rotation_z(PI * ease(t / 1.5)) * rotation;
                translation.y -= (0.05 * dims.length * (t - 1.5).max(0.0)).min(1.5 * dims.length);
            }
            MotionOverlay::Tumble => {
                let fall = (0.5 * GRAVITY * t * t).min(altitude.max(0.0));
                let airborne = fall < altitude;
                let spin = if airborne { 2.5 * t } else { FRAC_PI_2 };
                rotation = Quat::from_rotation_z(spin) * Quat::from_rotation_x(0.3 * spin.sin()) * rotation;
                translation.y -= fall;
            }
            MotionOverlay::HeadDip if !bones => {
                rotation = Quat::from_rotation_x(0.22 * ease(t / 0.6) + 0.04 * (TAU * 2.2 * t).sin()) * rotation;
            }
            MotionOverlay::HeadDip => {}
            MotionOverlay::LieDown => translation.y -= 0.35 * dims.height * ease(t / 1.2),
            MotionOverlay::LegGait => translation.y -= 0.015 * dims.hip * (2.0 * TAU * cycle).cos(),
            MotionOverlay::Lunge => {
                let l = (TAU * 1.1 * t).sin().max(0.0);
                translation.z += 0.12 * dims.length * l;
                rotation = Quat::from_rotation_x(0.06 * l) * rotation;
            }
            MotionOverlay::Hop => translation.y += 0.35 * dims.height.max(0.3 * dims.length) * hop_height(t * 1.2, 0.6),
            MotionOverlay::Waterline => {
                translation.y -= 0.6 * dims.height - 0.02 * dims.height * (TAU * 0.7 * t).sin();
                rotation = Quat::from_rotation_x(-0.1) * rotation;
            }
            MotionOverlay::Glide => rotation = Quat::from_rotation_z(0.12 * (TAU * 0.15 * t).sin()) * rotation,
        }
    }
    Transform { translation, rotation, scale: Vec3::ONE }
}

/// A joint the overlay can drive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointRef {
    /// Joint entity.
    pub entity: Entity,
    /// Rest local rotation.
    pub rest: Quat,
    /// Rest rotation of the joint's parent in model space.
    pub parent_model: Quat,
}

impl JointRef {
    /// Local rotation after rotating the joint by `angle` about the
    /// model-space `axis`, given the parent's current model rotation.
    pub fn rotated(&self, parent_model: Quat, axis: Vec3, angle: f32) -> Quat {
        parent_model.inverse() * Quat::from_axis_angle(axis, angle) * parent_model * self.rest
    }
}

/// Leg joints of one leg.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegJoints {
    /// 0 = fore, 1 = hind.
    pub pair: u8,
    /// Side.
    pub side: Side,
    /// Hip / shoulder joint.
    pub upper: JointRef,
    /// Knee / elbow joint.
    pub lower: Option<JointRef>,
}

/// Overridable joints of a GLB rig.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlbRig {
    /// Legs found by name.
    pub legs: Vec<LegJoints>,
    /// Neck joint.
    pub neck: Option<JointRef>,
    /// Head joint.
    pub head: Option<JointRef>,
    /// Model-space lateral axis (`up × forward`).
    pub lateral: Vec3,
}

/// Name of a leg joint in the Quaternius convention.
pub fn leg_joint_name(pair: u8, side: Side, upper: bool) -> String {
    format!(
        "{}{}Leg.{}",
        if pair == 0 { "Front" } else { "Back" },
        if upper { "Up" } else { "Low" },
        if side == Side::Left { "L" } else { "R" }
    )
}

/// Local rotations for every overridden joint at this frame, or an empty
/// list when no bone overlay is active.
pub fn bone_overlay(
    rig: &GlbRig,
    overlays: &[MotionOverlay],
    t: f32,
    cycle: f32,
    speed: f32,
    dims: &OverlayDims,
) -> Vec<(Entity, Quat)> {
    let mut out = Vec::new();
    let axis = rig.lateral;
    let gait_active = overlays.contains(&MotionOverlay::LegGait);
    let lying = overlays.contains(&MotionOverlay::LieDown);
    if gait_active || lying {
        let gait = gait_for_froude(froude_number(speed, dims.hip));
        let duty = duty_factor(gait, GallopStyle::Transverse);
        let amp = sweep_amplitude(stride_length(speed, dims.hip), duty, dims.hip);
        let flex = if gait == Gait::Gallop { 1.1 } else { 0.7 };
        let fold = ease(t / 1.2);
        for leg in &rig.legs {
            let (upper_angle, lower_angle) = if lying {
                if leg.pair == 0 { (1.3 * fold, -2.3 * fold) } else { (-1.3 * fold, 2.4 * fold) }
            } else {
                let p = leg_pose(cycle + quadruped_phase(gait, GallopStyle::Transverse, leg.pair, leg.side), duty, amp);
                // The hock flexes less than the carpus in a walk.
                (-p.swing, if leg.pair == 0 { flex * p.lift } else { -0.6 * flex * p.lift })
            };
            out.push((leg.upper.entity, leg.upper.rotated(leg.upper.parent_model, axis, upper_angle)));
            if let Some(lower) = leg.lower {
                // The knee's parent (the thigh) has just been rotated too.
                let parent_now = Quat::from_axis_angle(axis, upper_angle) * lower.parent_model;
                out.push((lower.entity, lower.rotated(parent_now, axis, lower_angle)));
            }
        }
    }
    if overlays.contains(&MotionOverlay::HeadDip) {
        let k = ease(t / 0.6);
        let nod = 0.06 * (TAU * 2.2 * t).sin();
        if let Some(neck) = rig.neck {
            out.push((neck.entity, neck.rotated(neck.parent_model, axis, 0.7 * k + nod)));
        }
        if let Some(head) = rig.head {
            let parent_now = Quat::from_axis_angle(axis, 0.7 * k + nod) * head.parent_model;
            out.push((head.entity, head.rotated(parent_now, axis, 0.35 * k)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims() -> OverlayDims {
        OverlayDims { height: 1.0, width: 0.4, length: 1.5, hip: 0.6 }
    }

    #[test]
    fn tip_over_rolls_and_rests_on_the_side() {
        let t = root_overlay(&[MotionOverlay::TipOverAndSink], 2.0, 0.0, &dims(), 0.0, true);
        let up = t.rotation * Vec3::Y;
        assert!(up.y.abs() < 1e-3, "lies on its side: {up}");
        assert!((t.translation.y - 0.2).abs() < 1e-4, "{}", t.translation);
        let later = root_overlay(&[MotionOverlay::TipOverAndSink], 20.0, 0.0, &dims(), 0.0, true);
        assert!(later.translation.y < t.translation.y, "sinks with time");
    }

    #[test]
    fn tumble_stops_at_the_ground() {
        let t = root_overlay(&[MotionOverlay::Tumble], 10.0, 0.0, &dims(), 7.0, true);
        assert!((t.translation.y + 7.0).abs() < 1e-4);
    }

    #[test]
    fn joint_rotation_about_model_axis() {
        // Parent rotated 90° about Y: the model lateral axis X is the
        // parent's local Z. Rotating the joint about model X must equal a
        // local rotation about Z.
        let p = Quat::from_rotation_y(FRAC_PI_2);
        let j = JointRef { entity: Entity::PLACEHOLDER, rest: Quat::IDENTITY, parent_model: p };
        let local = j.rotated(p, Vec3::X, 0.3);
        let world = p * local;
        let expected = Quat::from_rotation_x(0.3) * p;
        for v in [Vec3::X, Vec3::Y, Vec3::Z] {
            assert!((world * v).distance(expected * v) < 1e-5);
        }
    }

    #[test]
    fn leg_gait_moves_diagonal_feet_and_names_match_quaternius() {
        assert_eq!(leg_joint_name(0, Side::Left, true), "FrontUpLeg.L");
        assert_eq!(leg_joint_name(1, Side::Right, false), "BackLowLeg.R");
        let mut world = bevy::prelude::World::new();
        let mut joint =
            || JointRef { entity: world.spawn_empty().id(), rest: Quat::IDENTITY, parent_model: Quat::IDENTITY };
        let rig = GlbRig {
            legs: (0..4u8)
                .map(|k| LegJoints {
                    pair: k / 2,
                    side: if k % 2 == 0 { Side::Left } else { Side::Right },
                    upper: joint(),
                    lower: None,
                })
                .collect(),
            neck: None,
            head: None,
            lateral: Vec3::X,
        };
        let out = bone_overlay(&rig, &[MotionOverlay::LegGait], 0.0, 0.1, 1.0, &dims());
        assert_eq!(out.len(), 4);
        assert!(out.iter().any(|(_, q)| q.angle_between(Quat::IDENTITY) > 0.05));
        assert!(bone_overlay(&rig, &[MotionOverlay::Lunge], 0.0, 0.1, 1.0, &dims()).is_empty());
    }
}
