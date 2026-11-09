//! Bird body plan (also used for bats with membrane wings): ovoid body,
//! neck, round head with a species-specific beak, two-segment wings (arm
//! and hand) that fold along the body, legs with toes and a tail fan.
//! Wing-beat frequency follows Pennycuick's relation from mass, span and
//! wing area (see [`crate::action::bird_wingbeat_frequency`]).

use std::f32::consts::PI;

use bevy::math::{Quat, Vec2, Vec3};
use bevy::prelude::Transform;

use super::mesh::{Cap, MeshData, Ring, SurfacePoint, loft, lump, superellipsoid, tube};
use super::palette::{Coat, mix, shade, srgb};
use super::parts::{Detail, EarStyle, add_part, coat_color, ear_mesh, eye, solid, wing_outlines, wing_panel};
use super::rig::{GallopStyle, LegSegment, Locomotor, PartMaterial, PartRole, Rig, RigMotion, Side, WingSegment};
use crate::action::bird_wingbeat_frequency;

/// Beak shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BeakStyle {
    /// Short seed-cracking cone (finches, sparrows).
    Cone,
    /// Hooked raptor beak.
    Hooked,
    /// Broad flat bill (ducks).
    Flat,
    /// Long spear (herons).
    Dagger,
    /// Bat muzzle instead of a beak.
    Muzzle,
}

/// Tail-fan outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TailShape {
    /// Square-ended.
    Square,
    /// Rounded fan (raptors, pigeons).
    Fan,
    /// Forked (swallows, kites).
    Forked,
    /// Short stub (ducks, owls).
    Short,
    /// Tail membrane between the legs (bats).
    Membrane,
}

/// Wing outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WingShape {
    /// Rounded (passerines, chickens).
    Rounded,
    /// Pointed (falcons, swifts, ducks).
    Pointed,
    /// Slotted primaries (eagles, vultures, crows).
    Slotted,
    /// Skin membrane between fingers (bats).
    Membrane,
}

/// Parameters of a bird or bat (adult, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BirdParams {
    /// Body length (breast to vent).
    pub body_length: f32,
    /// Body width.
    pub body_width: f32,
    /// Body depth.
    pub body_depth: f32,
    /// Body pitch, nose up (rad): 0.15 duck, 0.6 owl.
    pub posture: f32,
    /// Neck length.
    pub neck_length: f32,
    /// Neck elevation (rad).
    pub neck_angle: f32,
    /// Head radius.
    pub head_radius: f32,
    /// Beak length.
    pub beak_length: f32,
    /// Beak depth at the base.
    pub beak_depth: f32,
    /// Beak shape.
    pub beak: BeakStyle,
    /// Upper beak colour.
    pub beak_color: u32,
    /// Visible leg length (tibia + tarsus).
    pub leg_length: f32,
    /// Leg / foot colour.
    pub leg_color: u32,
    /// Full wingspan.
    pub span: f32,
    /// Wing area (m²) for the wing-beat frequency.
    pub wing_area: f32,
    /// Wing outline.
    pub wing: WingShape,
    /// Tail length.
    pub tail_length: f32,
    /// Tail outline.
    pub tail: TailShape,
    /// Body plumage (countershading and pattern).
    pub coat: Coat,
    /// Head colour.
    pub head: u32,
    /// Throat / breast colour, if distinct.
    pub breast: Option<u32>,
    /// Wing colour.
    pub wing_color: u32,
    /// Wing-tip (primaries) colour.
    pub wing_tip: Option<u32>,
    /// Coloured wing bar / speculum.
    pub wing_bar: Option<u32>,
    /// Tail colour.
    pub tail_color: u32,
    /// Neck ring colour (mallard).
    pub neck_ring: Option<u32>,
    /// Red comb and wattles (chickens).
    pub comb: Option<u32>,
    /// Owl facial disc colour.
    pub facial_disc: Option<u32>,
    /// Feather ruff at the neck base (vultures).
    pub ruff: Option<u32>,
    /// Ear size (bats; 0 for birds).
    pub ears: f32,
    /// Whether it flies.
    pub flies: bool,
    /// Iris colour.
    pub iris: u32,
}

impl BirdParams {
    /// Generic songbird.
    pub const BASE: BirdParams = BirdParams {
        body_length: 0.1,
        body_width: 0.05,
        body_depth: 0.055,
        posture: 0.35,
        neck_length: 0.015,
        neck_angle: 0.6,
        head_radius: 0.02,
        beak_length: 0.012,
        beak_depth: 0.008,
        beak: BeakStyle::Cone,
        beak_color: 0x3a3028,
        leg_length: 0.03,
        leg_color: 0x8a6a5a,
        span: 0.24,
        wing_area: 0.0094,
        wing: WingShape::Rounded,
        tail_length: 0.055,
        tail: TailShape::Square,
        coat: Coat::countershaded(0x8a6440, 0xd8ccb8),
        head: 0x7a5a3a,
        breast: None,
        wing_color: 0x6a4a30,
        wing_tip: Some(0x3a2a20),
        wing_bar: None,
        tail_color: 0x5a4030,
        neck_ring: None,
        comb: None,
        facial_disc: None,
        ruff: None,
        ears: 0.0,
        flies: true,
        iris: 0x1a1008,
    };
}

/// Builds a bird (or bat) rig.
pub fn build(p: &BirdParams, mass_kg: f32, seed: u32, detail: Detail) -> Rig {
    let (bl, bw, bd) = (p.body_length, p.body_width, p.body_depth);
    let bat = p.wing == WingShape::Membrane;
    let flap_hz = if p.flies { bird_wingbeat_frequency(mass_kg, p.span, p.wing_area) } else { 0.0 };
    // Body pitch lowers the vent: make sure the body clears the ground and
    // lengthen the legs to reach it if necessary.
    let (sp, cp) = p.posture.sin_cos();
    let pitched_y = |y: f32, z: f32| y * cp + z * sp;
    let lowest_body = (0..=16)
        .map(|i| {
            let u = i as f32 / 16.0;
            let z = (u - 0.5) * bl;
            pitched_y(-0.58 * bd * (PI * u).sin().max(0.25), z)
        })
        .fold(f32::MAX, f32::min)
        .min(pitched_y(0.1 * bd, -0.42 * bl) - p.tail_length * (0.6 * p.posture).sin());
    let hip_y = pitched_y(-0.3 * bd, 0.05 * bl);
    let root_y = (p.leg_length - hip_y).max(-lowest_body + 0.02 * bl);
    let leg_total = root_y + hip_y;
    let tibia = 0.45 * leg_total;
    let tarsus = 0.55 * leg_total;
    let motion = RigMotion {
        locomotor: if bat { Locomotor::Bat } else { Locomotor::Bird },
        gallop: GallopStyle::Bound,
        mass_kg,
        hip_height: leg_total.max(0.2 * bd),
        body_length: bl + p.neck_length + 2.0 * p.head_radius + p.beak_length,
        rest_drop: (root_y + lowest_body).max(0.0),
        leg_lengths: vec![(tibia, tarsus)],
        leg_reach: Vec::new(),
        half_width: 0.5 * bw,
        half_span: 0.5 * p.span,
        flap_hz,
        flap_amplitude: if bat { 0.9 } else { 0.75 },
        can_fly: p.flies,
        aquatic: false,
        graze_pitch: 0.0,
        segments: 0,
        wave_length: 1.0,
        wave_amplitude: 0.0,
        vertical_undulation: false,
    };
    let mut rig = Rig::new(
        Transform::from_xyz(0.0, root_y, 0.0).with_rotation(Quat::from_rotation_x(-p.posture)),
        motion,
        srgb(p.coat.base),
    );

    // Body: egg-shaped, breast forward and deep.
    let n = detail.rings(10);
    let rings: Vec<Ring> = (0..=n)
        .map(|i| {
            let u = i as f32 / n as f32;
            let z = (u - 0.5) * bl;
            let s = (PI * u.powf(1.15)).sin().max(0.0).powf(0.7);
            Ring::asym(Vec3::new(0.0, 0.0, z), 0.5 * bw * s.max(0.25), 0.42 * bd * s.max(0.3), 0.58 * bd * s.max(0.25))
        })
        .collect();
    let breast = p.breast;
    let body_color = {
        let base = coat_color(p.coat, rig.rest_model_transform(0), seed);
        move |sp: &SurfacePoint| match breast {
            Some(b) if sp.dorsal < 0.1 && sp.position.z > -0.1 * bl => {
                mix(base(sp), srgb(b), ((sp.position.z / bl + 0.1) * 4.0).clamp(0.0, 1.0))
            }
            _ => base(sp),
        }
    };
    let mut body = MeshData::new();
    loft(&mut body, &rings, detail.around(14), Vec3::Y, Cap::Round, Cap::Round, &body_color);
    rig.parts[0].mesh = body;

    // Neck and head.
    let neck = add_part(
        &mut rig,
        0,
        PartRole::Neck(0),
        Transform::from_xyz(0.0, 0.2 * bd, 0.38 * bl).with_rotation(Quat::from_rotation_x(-(p.neck_angle - p.posture))),
        PartMaterial::Matte,
        |m, _| {
            let r = (0.55 * p.head_radius).max(0.16 * bw);
            let c = srgb(p.head);
            let ring = p.neck_ring.map(srgb);
            let len = p.neck_length.max(0.3 * p.head_radius);
            tube(
                m,
                &[Vec3::new(0.0, 0.0, -r), Vec3::new(0.0, 0.0, len)],
                &[r * 1.5, r],
                detail.around(10),
                Vec3::Y,
                (Cap::Round, Cap::Round),
                &move |sp| match ring {
                    Some(rc) if (sp.position.z - 0.1 * len).abs() < 0.12 * len.max(r) => rc,
                    _ => mix(c, srgb(p.coat.base), (1.0 - sp.position.z / len).clamp(0.0, 1.0) * 0.6),
                },
            );
            if let Some(ruff) = p.ruff {
                lump(
                    m,
                    Vec3::new(0.0, -0.1 * r, -0.2 * r),
                    Vec3::new(2.4 * r, 2.0 * r, 1.6 * r),
                    1,
                    0.3,
                    3.0,
                    seed ^ 0x77,
                    &move |_, k| shade(srgb(ruff), 0.7 + 0.3 * k),
                );
            }
        },
    );
    let neck_len = p.neck_length.max(0.3 * p.head_radius);
    let hr = p.head_radius;
    let head = add_part(
        &mut rig,
        neck,
        PartRole::Head,
        Transform::from_xyz(0.0, 0.0, neck_len).with_rotation(Quat::from_rotation_x(p.neck_angle)),
        PartMaterial::Matte,
        |m, _| {
            let hc = srgb(p.head);
            let disc = p.facial_disc.map(srgb);
            superellipsoid(
                m,
                Vec3::new(0.0, 0.25 * hr, 0.2 * hr),
                Vec3::new(0.85 * hr, 0.9 * hr, hr),
                1.0,
                1.0,
                detail.rings(8),
                detail.around(12),
                &move |sp| match disc {
                    Some(dc) if sp.position.z > 0.35 * hr => {
                        mix(hc, dc, ((sp.position.z - 0.35 * hr) / (0.5 * hr)).clamp(0.0, 1.0))
                    }
                    _ => hc,
                },
            );
            if let Some(comb) = p.comb {
                for k in 0..4 {
                    let z = (0.4 - 0.25 * k as f32) * hr;
                    superellipsoid(
                        m,
                        Vec3::new(0.0, 1.1 * hr - 0.05 * hr * k as f32, z),
                        Vec3::new(0.14 * hr, 0.42 * hr, 0.2 * hr),
                        1.0,
                        1.0,
                        4,
                        6,
                        &solid(comb),
                    );
                }
                superellipsoid(
                    m,
                    Vec3::new(0.0, -0.45 * hr, 0.75 * hr),
                    Vec3::new(0.15 * hr, 0.35 * hr, 0.15 * hr),
                    1.0,
                    1.0,
                    4,
                    6,
                    &solid(comb),
                );
            }
            beak(m, p, true, detail);
            if p.ears > 0.0 {
                for s in [-1.0f32, 1.0] {
                    let mut ear = MeshData::new();
                    ear_mesh(&mut ear, EarStyle::Pointed, p.ears, p.head, 0x6a4a40, None, detail);
                    m.append_transformed(
                        &ear,
                        &Transform::from_xyz(s * 0.45 * hr, 0.7 * hr, 0.0)
                            .with_rotation(Quat::from_rotation_z(-s * 0.45) * Quat::from_rotation_x(-0.2)),
                    );
                }
            }
        },
    );
    let eye_pos = if p.facial_disc.is_some() {
        Vec3::new(0.38 * hr, 0.35 * hr, 0.92 * hr)
    } else {
        Vec3::new(0.7 * hr, 0.35 * hr, 0.5 * hr)
    };
    eye(&mut rig, head, eye_pos, 0.2 * hr, p.iris, detail);
    eye(&mut rig, head, eye_pos * Vec3::new(-1.0, 1.0, 1.0), 0.2 * hr, p.iris, detail);
    if detail.small_parts() && p.beak != BeakStyle::Muzzle {
        add_part(
            &mut rig,
            head,
            PartRole::Jaw,
            Transform::from_xyz(0.0, 0.05 * hr, 0.85 * hr),
            PartMaterial::Glossy,
            |m, _| beak(m, p, false, detail),
        );
    }

    // Wings: arm (inner) and hand (outer), spread in the rest pose.
    let half = 0.5 * p.span - 0.4 * bw;
    let chord = (p.wing_area / p.span).max(0.2 * bl) * if bat { 1.1 } else { 1.0 };
    let split = if bat { 0.42 } else { 0.45 };
    let (inner_outline, outer_outline) =
        wing_outlines(half, chord, p.wing == WingShape::Slotted, p.wing == WingShape::Pointed, bat, split);
    let inner_span = half * split;
    let wing_material = if bat { PartMaterial::Translucent } else { PartMaterial::Matte };
    for side in Side::BOTH {
        let s = side.sign();
        let wc = srgb(p.wing_color);
        let tip = p.wing_tip.map(srgb);
        let bar = p.wing_bar.map(srgb);
        let shoulder = Vec3::new(s * 0.38 * bw, 0.25 * bd, 0.12 * bl);
        let inner = add_part(
            &mut rig,
            0,
            PartRole::Wing { pair: 0, side, segment: WingSegment::Inner },
            Transform::from_translation(shoulder),
            wing_material,
            |m, _| {
                wing_panel(m, &inner_outline, s, 0.12 * chord, detail.rings(3), &move |q, _| match bar {
                    Some(b) if q.y < -0.35 * chord && q.y > -0.7 * chord => b,
                    _ => shade(wc, 0.9 + 0.2 * (q.y / chord + 0.85)),
                });
                if bat {
                    // Arm bones along the leading edge.
                    tube(
                        m,
                        &[Vec3::ZERO, Vec3::new(s * inner_span, 0.0, 0.18 * chord)],
                        &[0.04 * chord, 0.03 * chord],
                        5,
                        Vec3::Y,
                        (Cap::Round, Cap::Round),
                        &solid(0x3a2a24),
                    );
                }
            },
        );
        let outer_span = half - inner_span;
        add_part(
            &mut rig,
            inner,
            PartRole::Wing { pair: 0, side, segment: WingSegment::Outer },
            Transform::from_xyz(s * inner_span, 0.0, 0.0),
            wing_material,
            |m, _| {
                wing_panel(m, &outer_outline, s, 0.08 * chord, detail.rings(3), &move |q, _| {
                    let t = (q.x / outer_span).clamp(0.0, 1.0);
                    match tip {
                        Some(tc) => mix(wc, tc, ((t - 0.35) * 2.5).clamp(0.0, 1.0)),
                        None => wc,
                    }
                });
                if bat {
                    for k in 0..3 {
                        let end = Vec2::new(outer_span * (1.0 - 0.18 * k as f32), chord * (0.05 - 0.45 * k as f32));
                        tube(
                            m,
                            &[Vec3::ZERO, Vec3::new(s * end.x, 0.0, end.y)],
                            &[0.025 * chord, 0.012 * chord],
                            4,
                            Vec3::Y,
                            (Cap::Round, Cap::Round),
                            &solid(0x3a2a24),
                        );
                    }
                }
            },
        );
    }

    // Tail fan.
    if p.tail_length > 0.0 {
        let tl = p.tail_length;
        let tw = match p.tail {
            TailShape::Fan => 0.55 * tl,
            TailShape::Membrane => 0.6 * tl,
            _ => 0.35 * tl,
        };
        let outline: Vec<Vec2> = match p.tail {
            TailShape::Forked => vec![
                Vec2::new(-0.3 * tw, 0.0),
                Vec2::new(0.3 * tw, 0.0),
                Vec2::new(tw, -tl),
                Vec2::new(0.0, -0.6 * tl),
                Vec2::new(-tw, -tl),
            ],
            TailShape::Fan | TailShape::Membrane => (0..=8)
                .map(|k| {
                    let a = PI * (1.0 + k as f32 / 8.0);
                    if k == 0 || k == 8 {
                        Vec2::new(0.3 * tw * a.cos().signum(), 0.0)
                    } else {
                        Vec2::new(tw * a.cos(), tl * a.sin())
                    }
                })
                .collect(),
            TailShape::Square | TailShape::Short => vec![
                Vec2::new(-0.3 * tw, 0.0),
                Vec2::new(0.3 * tw, 0.0),
                Vec2::new(tw * 0.8, -tl),
                Vec2::new(-tw * 0.8, -tl),
            ],
        };
        let tc = srgb(p.tail_color);
        add_part(
            &mut rig,
            0,
            PartRole::Tail(0),
            Transform::from_xyz(0.0, 0.1 * bd, -0.42 * bl).with_rotation(Quat::from_rotation_x(p.posture * 0.6)),
            wing_material,
            |m, _| {
                wing_panel(m, &outline, 1.0, 0.1 * tw.max(0.002), 2, &move |q, _| shade(tc, 1.0 + 0.4 * q.y / tl));
            },
        );
    }

    // Legs: tibia (feathered) and tarsus with toes; compensate the body pitch.
    let lc = srgb(p.leg_color);
    for side in Side::BOTH {
        let s = side.sign();
        let hip = Vec3::new(s * 0.22 * bw, -0.3 * bd, 0.05 * bl);
        let upper = add_part(
            &mut rig,
            0,
            PartRole::Leg { pair: 0, side, segment: LegSegment::Upper },
            Transform::from_translation(hip).with_rotation(Quat::from_rotation_x(p.posture)),
            PartMaterial::Matte,
            |m, model| {
                let c = coat_color(p.coat, model, seed);
                tube(
                    m,
                    &[Vec3::new(0.0, 0.1 * tibia, 0.0), Vec3::new(0.0, -tibia, 0.0)],
                    &[0.16 * bw, 0.06 * bw],
                    detail.around(8),
                    Vec3::Z,
                    (Cap::Round, Cap::Round),
                    &c,
                );
            },
        );
        add_part(
            &mut rig,
            upper,
            PartRole::Leg { pair: 0, side, segment: LegSegment::Lower },
            Transform::from_xyz(0.0, -tibia, 0.0),
            PartMaterial::Matte,
            |m, _| {
                let r = (0.035 * bw).max(0.0008);
                tube(
                    m,
                    &[Vec3::ZERO, Vec3::new(0.0, -tarsus, 0.0)],
                    &[r, r * 0.85],
                    detail.around(6),
                    Vec3::Z,
                    (Cap::Round, Cap::Round),
                    &move |_| lc,
                );
                if detail.small_parts() || bl > 0.3 {
                    let toe = 0.2 * tarsus + 0.25 * bw;
                    for a in [-0.45f32, 0.0, 0.45, PI] {
                        let dir = Vec3::new(a.sin(), 0.0, a.cos());
                        let len = if a == PI { 0.6 * toe } else { toe };
                        tube(
                            m,
                            &[Vec3::new(0.0, -tarsus, 0.0), Vec3::new(0.0, -tarsus - 0.2 * r, 0.0) + dir * len],
                            &[r * 0.8, r * 0.4],
                            4,
                            Vec3::Y,
                            (Cap::Round, Cap::Point(r * 0.6)),
                            &move |_| lc,
                        );
                    }
                }
            },
        );
    }
    rig
}

/// Upper (`upper = true`) or lower beak in the head (or jaw) frame.
fn beak(m: &mut MeshData, p: &BirdParams, upper: bool, detail: Detail) {
    let (len, depth, hr) = (p.beak_length, p.beak_depth, p.head_radius);
    let color = if upper { srgb(p.beak_color) } else { shade(srgb(p.beak_color), 0.85) };
    let (base_z, half_w) = if upper { (0.75 * hr, 0.5 * depth) } else { (0.0, 0.42 * depth) };
    let c = move |_: &SurfacePoint| color;
    match (p.beak, upper) {
        (BeakStyle::Muzzle, true) => {
            superellipsoid(
                m,
                Vec3::new(0.0, 0.0, 0.95 * hr),
                Vec3::new(0.45 * hr, 0.4 * hr, 0.4 * hr),
                1.0,
                1.0,
                5,
                detail.around(8),
                &solid(0x4a3028),
            );
        }
        (BeakStyle::Muzzle, false) => {}
        (BeakStyle::Flat, _) => {
            let y = if upper { 0.0 } else { -0.18 * depth };
            let rings = [
                Ring::ellipse(Vec3::new(0.0, y, base_z), 1.1 * half_w, (if upper { 0.45 } else { 0.25 }) * depth),
                Ring::ellipse(
                    Vec3::new(0.0, y - 0.15 * depth, base_z + 0.6 * len),
                    1.0 * half_w,
                    (if upper { 0.25 } else { 0.15 }) * depth,
                ),
                Ring::ellipse(Vec3::new(0.0, y - 0.2 * depth, base_z + len), 1.05 * half_w, 0.12 * depth),
            ];
            loft(m, &rings, detail.around(10), Vec3::Y, Cap::Flat, Cap::Round, &c);
        }
        (style, _) => {
            let (curve, len) = match style {
                BeakStyle::Hooked => (if upper { 0.9 } else { 0.0 }, if upper { len } else { 0.7 * len }),
                _ => (0.0, if upper { len } else { 0.92 * len }),
            };
            let n = detail.rings(5);
            let pts: Vec<Vec3> = (0..=n)
                .map(|i| {
                    let u = i as f32 / n as f32;
                    let a = curve * u * u;
                    Vec3::new(
                        0.0,
                        -(a.sin() * len * 0.5) * u - if upper { 0.0 } else { 0.25 * depth },
                        base_z + len * u * (1.0 - 0.3 * curve * u),
                    )
                })
                .collect();
            let radii: Vec<f32> = (0..=n).map(|i| half_w * (1.0 - i as f32 / n as f32).powf(0.8).max(0.05)).collect();
            tube(m, &pts, &radii, detail.around(8), Vec3::Y, (Cap::Flat, Cap::Point(0.1 * len)), &c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn songbird_rig_is_valid_and_grounded() {
        let rig = build(&BirdParams::BASE, 0.03, 3, Detail::Full);
        rig.validate().unwrap();
        let (lo, hi) = rig.rest_bounds().unwrap();
        assert!(lo.y > -0.02 && lo.y < 0.01, "toes at the ground: {lo}");
        // Spread wings reach close to the half span.
        assert!(hi.x > 0.09 && lo.x < -0.09, "{lo} {hi}");
        assert!((10.0..17.0).contains(&rig.motion.flap_hz), "{}", rig.motion.flap_hz);
        for side in Side::BOTH {
            assert!(rig.find(PartRole::Wing { pair: 0, side, segment: WingSegment::Outer }).is_some());
        }
    }

    #[test]
    fn flightless_bird_does_not_flap() {
        let hen = BirdParams { flies: false, ..BirdParams::BASE };
        assert_eq!(build(&hen, 2.0, 1, Detail::Low).motion.flap_hz, 0.0);
    }
}
