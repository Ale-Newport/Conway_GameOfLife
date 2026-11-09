//! Head and tail of the quadruped body plan: skull and snout loft, eyes,
//! jaw, ears, trunk, tusks, headgear (antlers, palmate antlers, horns,
//! ossicones, nasal horns), neck crest and the segmented tail.

use std::f32::consts::PI;

use bevy::math::{Quat, Vec2, Vec3};
use bevy::prelude::Transform;

use super::mesh::{Cap, MeshData, Ring, SurfacePoint, loft, planform, superellipsoid, tube};
use super::palette::{Coat, mix, srgb};
use super::parts::{
    Detail, EarStyle, TailStyle, add_part, basis_rotation, coat_color, ear_mesh, eye, horn, nose, solid, tail_segment,
};
use super::quadruped::{Headgear, QuadrupedParams};
use super::rig::{PartMaterial, PartRole, Rig, Side};

/// Builds the head (and its children) under `parent`; returns its index.
pub(super) fn build_head(
    rig: &mut Rig,
    parent: usize,
    rest: Transform,
    p: &QuadrupedParams,
    seed: u32,
    detail: Detail,
) -> usize {
    let (hl, hw, hd) = (p.head_length, p.head_width, p.head_depth);
    let cranium_end = 0.8 * hl - p.snout * hl;
    let tip_z = 0.8 * hl;
    let muzzle = p.muzzle;
    let mask = p.mask;
    let coat = match p.face {
        Some(f) => Coat { base: f, belly: f, ..p.coat },
        None => p.coat,
    };
    // Eyes sit on the skull surface: half width of the elliptic section at
    // the eye's height, pulled towards the face for forward-facing eyes.
    let eye_z = cranium_end - 0.1 * hl + 0.05 * hl * p.eyes_forward;
    let eye_y = 0.18 * hd;
    let section_w = 0.5 * hw * (1.0 - 0.25 * (0.5 - 0.5 * p.snout_width) * (eye_z / cranium_end).clamp(0.0, 1.0));
    let section_t = 0.45 * hd;
    let surface_x = section_w * (1.0 - (eye_y / section_t).powi(2)).max(0.0).sqrt();
    let eye_r = 0.11 * hw.min(1.2 * hd) + 0.002;
    let eye_pos = Vec3::new(
        (surface_x - 0.35 * eye_r) * (1.0 - 0.22 * p.eyes_forward),
        eye_y,
        eye_z + 0.06 * hl * p.eyes_forward,
    );
    let head = add_part(rig, parent, PartRole::Head, rest, PartMaterial::Matte, |m, model| {
        let base = coat_color(coat, model, seed);
        let color = move |sp: &SurfacePoint| {
            let mut c = base(sp);
            if let Some(mz) = muzzle {
                let t = ((sp.position.z - cranium_end) / (tip_z - cranium_end)).clamp(0.0, 1.0);
                if sp.dorsal < 0.2 || t > 0.7 {
                    c = mix(c, srgb(mz), (t * 1.5).min(1.0));
                }
            }
            if let Some(mk) = mask {
                // Bandit mask: a dark band across the eyes, not the muzzle.
                let band = (sp.position.y - eye_pos.y).abs() < 0.24 * hd
                    && sp.position.z > eye_pos.z - 0.3 * hl
                    && sp.position.z < eye_pos.z + 0.16 * hl
                    && sp.dorsal > -0.5;
                if band {
                    c = srgb(mk);
                }
            }
            c
        };
        let sw = p.snout_width;
        let rings = [
            Ring::asym(Vec3::new(0.0, 0.0, -0.22 * hl), 0.3 * hw, 0.32 * hd, 0.3 * hd),
            Ring::asym(Vec3::new(0.0, 0.02 * hd, -0.05 * hl), 0.48 * hw, 0.5 * hd, 0.45 * hd),
            Ring::asym(Vec3::new(0.0, 0.0, 0.5 * cranium_end), 0.5 * hw, 0.5 * hd, 0.45 * hd),
            Ring::asym(Vec3::new(0.0, -0.08 * hd, cranium_end), 0.5 * hw * (0.5 + 0.5 * sw), 0.36 * hd, 0.4 * hd),
            Ring::asym(
                Vec3::new(0.0, -0.14 * hd, cranium_end + 0.55 * (tip_z - cranium_end)),
                0.5 * hw * sw * 0.9,
                0.27 * hd,
                0.3 * hd,
            ),
            Ring::asym(Vec3::new(0.0, -0.17 * hd, tip_z), 0.5 * hw * sw * 0.75, 0.22 * hd, 0.24 * hd),
        ];
        loft(m, &rings, detail.around(14), Vec3::Y, Cap::Round, Cap::Round, &color);
        // Nose pad on the tip of the rounded snout cap.
        let tip_r = 0.25 * (2.0 * 0.5 * hw * sw * 0.75 + 0.22 * hd + 0.24 * hd);
        nose(m, Vec3::new(0.0, -0.12 * hd, tip_z + 0.8 * tip_r), 0.16 * hw * sw.max(0.4), p.nose, detail);
        // Tusks from the upper jaw.
        if p.tusks > 0.0 {
            for s in [-1.0f32, 1.0] {
                horn(
                    m,
                    Vec3::new(s * 0.3 * hw * sw, -0.3 * hd, tip_z - 0.1 * hl),
                    Vec3::new(s * 0.15, -0.9, 0.5),
                    Vec3::new(0.0, 0.3, 1.0),
                    p.tusks,
                    0.08 * hw + 0.05 * p.tusks,
                    0.9,
                    0xf0ead8,
                    0xe8e0c8,
                    detail,
                );
            }
        }
        headgear(m, p, detail);
    });
    eye(rig, head, eye_pos, eye_r, p.iris, detail);
    eye(rig, head, eye_pos * Vec3::new(-1.0, 1.0, 1.0), eye_r, p.iris, detail);
    if detail.small_parts() {
        let jaw_len = tip_z - 0.05 * hl;
        let jaw_color = muzzle.unwrap_or(coat.belly);
        add_part(rig, head, PartRole::Jaw, Transform::from_xyz(0.0, -0.3 * hd, 0.0), PartMaterial::Matte, |m, _| {
            let rings = [
                Ring::ellipse(Vec3::new(0.0, 0.0, 0.0), 0.35 * hw, 0.14 * hd),
                Ring::ellipse(Vec3::new(0.0, -0.02 * hd, 0.6 * jaw_len), 0.28 * hw * p.snout_width.max(0.4), 0.1 * hd),
                Ring::ellipse(Vec3::new(0.0, 0.0, jaw_len), 0.2 * hw * p.snout_width.max(0.4), 0.07 * hd),
            ];
            loft(m, &rings, 8, Vec3::Y, Cap::Round, Cap::Round, &solid(jaw_color));
        });
    }
    // Ears.
    if p.ears != EarStyle::None {
        for side in Side::BOTH {
            let s = side.sign();
            let (pos, rot) = match p.ears {
                // Elephant ears hang flat against the sides of the head
                // (direction given in the body frame, so undo the head pitch),
                // their broad face turned outwards.
                EarStyle::Fan => {
                    let unpitch = Quat::from_rotation_x(-p.head_pitch);
                    (
                        Vec3::new(s * 0.48 * hw, 0.15 * hd, -0.18 * hl),
                        basis_rotation(unpitch * Vec3::new(s * 0.2, -0.95, -0.25), unpitch * Vec3::new(s, 0.0, 0.2)),
                    )
                }
                EarStyle::Drooping => (
                    Vec3::new(s * 0.36 * hw, 0.4 * hd, -0.02 * hl),
                    Quat::from_rotation_z(-s * 1.9) * Quat::from_rotation_x(0.3),
                ),
                EarStyle::Long => (
                    Vec3::new(s * 0.22 * hw, 0.42 * hd, -0.1 * hl),
                    Quat::from_rotation_z(-s * 0.18) * Quat::from_rotation_x(-0.35 - p.head_pitch),
                ),
                _ => (
                    Vec3::new(s * 0.3 * hw, 0.42 * hd, -0.04 * hl),
                    Quat::from_rotation_z(-s * 0.35) * Quat::from_rotation_x(-0.15 - p.head_pitch),
                ),
            };
            let (style, size, inner, tip, outer) =
                (p.ears, p.ear_size, p.ear_inner, p.ear_tip, p.face.unwrap_or(p.coat.base));
            add_part(
                rig,
                head,
                PartRole::Ear(side),
                Transform::from_translation(pos).with_rotation(rot),
                PartMaterial::Matte,
                |m, _| {
                    ear_mesh(m, style, size, outer, inner, tip, detail);
                },
            );
        }
    }
    // Trunk.
    if p.trunk > 0.0 {
        let segs = if detail.small_parts() { 5 } else { 2 };
        let seg_len = p.trunk / segs as f32;
        let mut parent_idx = head;
        let base_r = 0.22 * hw;
        for i in 0..segs {
            let u0 = i as f32 / segs as f32;
            let u1 = (i + 1) as f32 / segs as f32;
            let pos = if i == 0 { Vec3::new(0.0, -0.12 * hd, tip_z - 0.05 * hl) } else { Vec3::new(0.0, 0.0, seg_len) };
            let droop = if i == 0 { 0.55 } else { 0.32 };
            parent_idx = add_part(
                rig,
                parent_idx,
                PartRole::Trunk(i as u8),
                Transform::from_translation(pos).with_rotation(Quat::from_rotation_x(droop)),
                PartMaterial::Matte,
                |m, model| {
                    let c = coat_color(coat, model, seed);
                    let r0 = base_r * (1.0 - 0.6 * u0);
                    let r1 = base_r * (1.0 - 0.6 * u1);
                    tube(
                        m,
                        &[Vec3::ZERO, Vec3::new(0.0, 0.0, seg_len)],
                        &[r0, r1],
                        detail.around(10),
                        Vec3::Y,
                        (Cap::Round, Cap::Round),
                        &c,
                    );
                },
            );
        }
    }
    head
}

fn headgear(m: &mut MeshData, p: &QuadrupedParams, detail: Detail) {
    let (hl, hw, hd) = (p.head_length, p.head_width, p.head_depth);
    let poll = Vec3::new(0.0, 0.42 * hd, -0.05 * hl);
    let up_back = Quat::from_rotation_x(-p.head_pitch);
    match p.headgear {
        Headgear::None => {}
        Headgear::Antlers { length, tines } => {
            for s in [-1.0f32, 1.0] {
                let start = poll + Vec3::new(s * 0.22 * hw, 0.0, 0.0);
                let dir = up_back * Vec3::new(s * 0.55, 1.0, -0.35);
                let bend = up_back * Vec3::new(0.0, 0.2, 1.0);
                horn(m, start, dir, bend, length, 0.035 * length + 0.01, 0.9, 0x6a5038, 0xd8c8a8, detail);
                let tines = if detail.small_parts() { tines } else { tines.min(2) };
                let axis = dir.normalize().cross(bend).normalize_or(Vec3::X);
                for t in 0..tines {
                    let u = 0.25 + 0.65 * t as f32 / tines.max(1) as f32;
                    let along = arc_point(start, dir.normalize(), axis, length, 0.9, u);
                    let tine_dir = up_back * Vec3::new(s * 0.15, 0.8, 0.6);
                    horn(
                        m,
                        along,
                        tine_dir,
                        Vec3::Y,
                        length * (0.38 - 0.1 * u),
                        0.022 * length + 0.006,
                        0.4,
                        0x8a7050,
                        0xe0d4b8,
                        detail,
                    );
                }
            }
        }
        Headgear::Palmate { span } => {
            for s in [-1.0f32, 1.0] {
                let start = poll + Vec3::new(s * 0.25 * hw, 0.0, 0.0);
                let beam_end = start + up_back * Vec3::new(s * 0.35 * span, 0.12 * span, -0.05 * span);
                tube(
                    m,
                    &[start, beam_end],
                    &[0.04 * span, 0.03 * span],
                    detail.around(7),
                    Vec3::Y,
                    (Cap::Flat, Cap::Round),
                    &solid(0x8a7656),
                );
                let mut outline = Vec::new();
                let points = if detail.small_parts() { 7 } else { 3 };
                for k in 0..=points {
                    let a = PI * (0.05 + 0.9 * k as f32 / points as f32);
                    let r = span * (0.55 + if k % 2 == 0 { 0.12 } else { 0.0 });
                    outline.push(Vec2::new(a.cos() * r * 0.75, a.sin() * r));
                }
                outline.push(Vec2::new(-0.1 * span, 0.0));
                // Broad palm rising up and out from the beam, facing forward.
                let u = up_back * Vec3::new(s, 0.2, -0.1).normalize();
                let v = up_back * Vec3::new(0.2 * s, 1.0, -0.35).normalize();
                planform(m, &outline, Vec2::new(0.0, 0.15 * span), 2, 0.05 * span, beam_end, u, v, &|_, f| {
                    mix(srgb(0x9a8664), srgb(0xc8b894), f)
                });
            }
        }
        Headgear::Horns { length, curl, spread } => {
            for s in [-1.0f32, 1.0] {
                let start = poll + Vec3::new(s * 0.3 * hw, -0.02 * hd, 0.02 * hl);
                let dir = up_back * Vec3::new(s * (0.4 + spread), 0.7 - 0.4 * spread, -0.25);
                let bend = if curl > 2.5 {
                    up_back * Vec3::new(0.0, -0.4, 1.0)
                } else {
                    up_back * Vec3::new(-s * 0.2, 1.0, 0.4)
                };
                horn(m, start, dir, bend, length, 0.06 * hw + 0.12 * length, curl, 0xd8ccb0, 0x3a3228, detail);
            }
        }
        Headgear::Ossicones => {
            for s in [-1.0f32, 1.0] {
                let start = poll + Vec3::new(s * 0.18 * hw, 0.0, 0.0);
                let top = start + up_back * Vec3::new(s * 0.05 * hl, 0.4 * hl, -0.08 * hl);
                tube(
                    m,
                    &[start, top],
                    &[0.07 * hw, 0.06 * hw],
                    detail.around(7),
                    Vec3::Z,
                    (Cap::Flat, Cap::Open),
                    &solid(0x8a6a40),
                );
                superellipsoid(m, top, Vec3::splat(0.085 * hw), 1.0, 1.0, 5, detail.around(7), &solid(0x2a2018));
            }
        }
        Headgear::NoseHorns { front, rear } => {
            let tip_z = 0.8 * hl;
            horn(
                m,
                Vec3::new(0.0, 0.05 * hd, tip_z - 0.18 * hl),
                up_back * Vec3::new(0.0, 1.0, 0.35),
                up_back * Vec3::new(0.0, 0.0, -1.0),
                front,
                0.13 * hw + 0.1 * front,
                0.5,
                0x9a9080,
                0x5a5448,
                detail,
            );
            if rear > 0.0 {
                horn(
                    m,
                    Vec3::new(0.0, 0.18 * hd, tip_z - 0.42 * hl),
                    up_back * Vec3::new(0.0, 1.0, 0.1),
                    up_back * Vec3::new(0.0, 0.0, -1.0),
                    rear,
                    0.1 * hw + 0.1 * rear,
                    0.3,
                    0x9a9080,
                    0x5a5448,
                    detail,
                );
            }
        }
    }
}

/// Point at fraction `u` along the circular arc traced by [`horn`].
fn arc_point(start: Vec3, dir: Vec3, axis: Vec3, length: f32, curl: f32, u: f32) -> Vec3 {
    let steps = 24;
    let mut p = start;
    let target = (u * steps as f32) as usize;
    for i in 0..target {
        let t = i as f32 / steps as f32;
        p += (Quat::from_axis_angle(axis, curl * t) * dir) * (length / steps as f32);
    }
    p
}

/// Builds the segmented tail from `start` (rig-root frame). Long tails on
/// short animals droop less so that the tip (and tuft) stays at least
/// `clearance` above the ground.
pub(super) fn build_tail(rig: &mut Rig, p: &QuadrupedParams, start: Vec3, clearance: f32, seed: u32, detail: Detail) {
    let coat = p.coat;
    let segs = if detail.small_parts() { 3 } else { 1 };
    let seg_len = p.tail_length / segs as f32;
    // Keep the tail (and its tuft) above the ground: reduce the droop of
    // long tails on short animals until the tip clears the ground.
    let extra = |d0: f32| 0.25 * d0.signum() * d0.abs().min(0.6);
    let drop = |d0: f32| -> f32 {
        (0..segs).map(|i| seg_len * (d0 + extra(d0) * i as f32).sin()).sum::<f32>()
            + if matches!(p.tail, TailStyle::Tufted | TailStyle::Flowing) {
                0.5 * (d0 + extra(d0) * (segs - 1) as f32).sin()
                    * if p.tail == TailStyle::Flowing { 2.4 * p.tail_length } else { 0.18 * p.tail_length }
            } else {
                0.0
            }
    };
    let available = clearance;
    let mut droop0 = p.tail_droop;
    while droop0 > -0.5 && drop(droop0) > available {
        droop0 -= 0.02;
    }
    let mut parent = 0;
    for i in 0..segs {
        let u0 = i as f32 / segs as f32;
        let u1 = (i + 1) as f32 / segs as f32;
        let profile = |u: f32| match p.tail {
            TailStyle::Bushy => p.tail_radius * (0.55 + 0.45 * (PI * (0.15 + 0.8 * u)).sin()),
            TailStyle::Heavy => p.tail_radius * (1.0 - 0.9 * u),
            TailStyle::Stub => p.tail_radius * (1.0 - 0.4 * u),
            _ => p.tail_radius * (1.0 - 0.5 * u),
        };
        let droop = if i == 0 { droop0 } else { extra(droop0) };
        let pos = if i == 0 { start } else { Vec3::new(0.0, 0.0, -seg_len) };
        let last = i + 1 == segs;
        let tip = p.tail_tip;
        let rings_c = p.tail_rings;
        parent = add_part(
            rig,
            parent,
            PartRole::Tail(i as u8),
            Transform::from_translation(pos).with_rotation(Quat::from_rotation_x(-droop)),
            PartMaterial::Matte,
            |m, model| {
                let base = coat_color(coat, model, seed);
                let colorize = move |sp: &SurfacePoint| match (tip, rings_c) {
                    (Some(t), _) if last && sp.position.z < -0.55 * seg_len => srgb(t),
                    (_, Some(r)) if (sp.position.z / seg_len * 2.0).rem_euclid(1.0) < 0.45 => srgb(r),
                    _ => base(sp),
                };
                let tuft_len = if p.tail == TailStyle::Flowing { 2.4 * p.tail_length } else { 0.18 * p.tail_length };
                tail_segment(
                    m,
                    p.tail,
                    seg_len,
                    profile(u0),
                    profile(u1),
                    &colorize,
                    last,
                    (tip.unwrap_or(0x222222), tuft_len),
                    detail,
                );
            },
        );
    }
}

/// Upright hair crest along the top of a neck segment (horses, zebras).
pub(super) fn crest(m: &mut MeshData, len: f32, r: f32, color: u32, detail: Detail) {
    let rings: Vec<Ring> = (0..=4)
        .map(|i| {
            let u = i as f32 / 4.0;
            Ring::ellipse(Vec3::new(0.0, r * (0.95 + 0.25 * (1.0 - u)), u * len), 0.18 * r, 0.32 * r)
        })
        .collect();
    loft(m, &rings, detail.around(6), Vec3::Y, Cap::Round, Cap::Round, &solid(color));
}
