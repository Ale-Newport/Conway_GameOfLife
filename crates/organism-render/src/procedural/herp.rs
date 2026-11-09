//! Frogs and toads (squat body, bulging eyes, folded jumping legs),
//! turtles and tortoises (scuted shell, retractable neck, columnar legs or
//! flippers) and snails (logarithmic-spiral shell on a muscular foot).

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::math::{Quat, Vec3};
use bevy::prelude::Transform;

use super::mesh::{Cap, MeshData, Ring, SurfacePoint, loft, superellipsoid, tube};
use super::noise::{worley2, worley2_f2};
use super::palette::{Coat, mix, shade, srgb};
use super::parts::{Detail, add_part, basis_rotation, coat_color, eye, solid};
use super::rig::{GallopStyle, LegSegment, Locomotor, PartMaterial, PartRole, Rig, RigMotion, Side};

/// Parameters of a frog or toad (adult, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrogParams {
    /// Snout-vent length.
    pub length: f32,
    /// Body width.
    pub width: f32,
    /// Body depth.
    pub depth: f32,
    /// Hind-leg length (thigh + shank + foot).
    pub hind_leg: f32,
    /// Skin coat.
    pub coat: Coat,
    /// Warty skin (toads).
    pub warts: bool,
    /// Iris colour.
    pub iris: u32,
}

/// Body pitch of a sitting frog (rad, snout up).
pub const FROG_PITCH: f32 = 0.32;

/// Model-space directions of a frog's hind leg on side `s` (+1 left):
/// sitting thigh (forward, out, down), sitting shank (folded back),
/// extended thigh and extended shank (both straight back, as at take-off).
pub fn frog_hind_directions(s: f32) -> [Vec3; 4] {
    [
        Vec3::new(s * 0.55, -0.3, 0.75).normalize(),
        Vec3::new(s * 0.35, -0.25, -0.9).normalize(),
        Vec3::new(s * 0.25, -0.35, -0.9).normalize(),
        Vec3::new(s * 0.15, -0.3, -0.95).normalize(),
    ]
}

/// Model-space rest rotations (thigh, shank) of a frog's hind leg: each
/// segment hangs along its local −Y, with local +Z kept towards the sky.
pub fn frog_hind_rest(s: f32) -> (Quat, Quat) {
    let [d1, d2, ..] = frog_hind_directions(s);
    (basis_rotation(-d1, Vec3::Y), basis_rotation(-d2, Vec3::Y))
}

/// Builds a frog; the root is the body, sitting with the snout raised.
pub fn build_frog(p: &FrogParams, mass_kg: f32, seed: u32, detail: Detail) -> Rig {
    let (l, w, d) = (p.length, p.width, p.depth);
    let thigh = 0.35 * p.hind_leg;
    let shank = 0.35 * p.hind_leg;
    let fore = 0.35 * l;
    let motion = RigMotion {
        locomotor: Locomotor::Anuran,
        gallop: GallopStyle::Bound,
        mass_kg,
        hip_height: 0.35 * p.hind_leg,
        body_length: l,
        rest_drop: 0.15 * d,
        leg_lengths: vec![(0.5 * fore, 0.5 * fore), (thigh, shank)],
        leg_reach: Vec::new(),
        half_width: 0.5 * w,
        half_span: 0.0,
        flap_hz: 0.0,
        flap_amplitude: 0.0,
        can_fly: false,
        aquatic: false,
        graze_pitch: 0.15,
        segments: 0,
        wave_length: 1.0,
        wave_amplitude: 0.0,
        vertical_undulation: false,
    };
    let root_rot = Quat::from_rotation_x(-FROG_PITCH);
    let mut rig = Rig::new(Transform::from_xyz(0.0, 0.55 * d, 0.0).with_rotation(root_rot), motion, srgb(p.coat.base));
    let n = detail.rings(9);
    let rings: Vec<Ring> = (0..=n)
        .map(|i| {
            let u = i as f32 / n as f32;
            let z = (u - 0.55) * l;
            let wide = (PI * (0.15 + 0.8 * u)).sin().powf(0.6);
            let head = if u > 0.7 { 1.0 - (u - 0.7) * 1.6 } else { 1.0 };
            Ring::asym(
                Vec3::new(0.0, 0.0, z),
                0.5 * w * wide.max(0.35) * head.max(0.45),
                0.42 * d * wide.max(0.3),
                0.5 * d * wide.max(0.3) * head.max(0.5),
            )
        })
        .collect();
    let warts = p.warts;
    let base = coat_color(p.coat, rig.rest_model_transform(0), seed);
    let skin = move |sp: &SurfacePoint| {
        let c = base(sp);
        if warts {
            let (dist, _) =
                worley2(bevy::math::Vec2::new(sp.position.z, sp.position.x + sp.dorsal * 0.01) * 180.0, seed ^ 13);
            if dist < 0.25 && sp.dorsal > 0.0 {
                return shade(c, 0.7);
            }
        }
        c
    };
    let mut body = MeshData::new();
    loft(&mut body, &rings, detail.around(14), Vec3::Y, Cap::Round, Cap::Round, &skin);
    rig.parts[0].mesh = body;
    // Bulging eyes on top of the head.
    let e = Vec3::new(0.3 * w, 0.42 * d, 0.27 * l);
    if detail.small_parts() {
        for s in [-1.0f32, 1.0] {
            add_part(
                &mut rig,
                0,
                PartRole::Static,
                Transform::from_translation(e * Vec3::new(s, 1.0, 1.0)),
                PartMaterial::Matte,
                |m, _| {
                    superellipsoid(m, Vec3::ZERO, Vec3::splat(0.15 * w), 1.0, 1.0, 5, 8, &solid(p.coat.base));
                },
            );
        }
    }
    eye(&mut rig, 0, e + Vec3::new(0.06 * w, 0.06 * w, 0.02 * l), 0.11 * w, p.iris, detail);
    eye(
        &mut rig,
        0,
        (e + Vec3::new(0.06 * w, 0.06 * w, 0.02 * l)) * Vec3::new(-1.0, 1.0, 1.0),
        0.11 * w,
        p.iris,
        detail,
    );
    let limb = |m: &mut MeshData, model: Transform, len: f32, r0: f32, r1: f32, foot: f32| {
        let c = coat_color(p.coat, model, seed);
        tube(
            m,
            &[Vec3::ZERO, Vec3::new(0.0, -len, 0.0)],
            &[r0, r1],
            detail.around(8),
            Vec3::Z,
            (Cap::Round, Cap::Round),
            &c,
        );
        if foot > 0.0 {
            for a in [-0.5f32, 0.0, 0.5] {
                let dir = Vec3::new(a.sin(), 0.0, a.cos());
                tube(
                    m,
                    &[Vec3::new(0.0, -len, 0.0), Vec3::new(0.0, -len, 0.0) + dir * foot],
                    &[r1 * 0.6, r1 * 0.4],
                    4,
                    Vec3::Y,
                    (Cap::Round, Cap::Round),
                    &c,
                );
            }
        }
    };
    for side in Side::BOTH {
        let s = side.sign();
        // Fore legs: short props under the chest.
        let fu = add_part(
            &mut rig,
            0,
            PartRole::Leg { pair: 0, side, segment: LegSegment::Upper },
            Transform::from_xyz(s * 0.35 * w, -0.25 * d, 0.2 * l)
                .with_rotation(Quat::from_rotation_x(0.32) * Quat::from_rotation_z(s * 0.35)),
            PartMaterial::Matte,
            |m, model| limb(m, model, 0.5 * fore, 0.07 * w, 0.05 * w, 0.0),
        );
        add_part(
            &mut rig,
            fu,
            PartRole::Leg { pair: 0, side, segment: LegSegment::Lower },
            Transform::from_xyz(0.0, -0.5 * fore, 0.0).with_rotation(Quat::from_rotation_z(-s * 0.5)),
            PartMaterial::Matte,
            |m, model| limb(m, model, 0.5 * fore, 0.05 * w, 0.04 * w, 0.18 * l),
        );
        // Hind legs folded in a Z: thigh forward-out, shank back, long
        // webbed foot forward on the ground.
        let (q1, q2) = frog_hind_rest(s);
        let hu = add_part(
            &mut rig,
            0,
            PartRole::Leg { pair: 1, side, segment: LegSegment::Upper },
            Transform::from_xyz(s * 0.32 * w, -0.1 * d, -0.4 * l).with_rotation(root_rot.inverse() * q1),
            PartMaterial::Matte,
            |m, model| limb(m, model, thigh, 0.13 * w, 0.08 * w, 0.0),
        );
        add_part(
            &mut rig,
            hu,
            PartRole::Leg { pair: 1, side, segment: LegSegment::Lower },
            Transform::from_xyz(0.0, -thigh, 0.0).with_rotation(q1.inverse() * q2),
            PartMaterial::Matte,
            |m, model| {
                limb(m, model, shank, 0.07 * w, 0.05 * w, 0.0);
                let c = coat_color(p.coat, model, seed);
                let ankle = Vec3::new(0.0, -shank, 0.0);
                for a in [-0.4f32, 0.0, 0.4] {
                    let dir = model.rotation.inverse() * Vec3::new(s * 0.35 + a, -0.08, 1.0).normalize();
                    tube(
                        m,
                        &[ankle, ankle + dir * 0.32 * p.hind_leg],
                        &[0.045 * w, 0.02 * w],
                        4,
                        Vec3::Y,
                        (Cap::Round, Cap::Round),
                        &c,
                    );
                }
            },
        );
    }
    rig
}

/// Parameters of a turtle or tortoise (adult, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurtleParams {
    /// Carapace length.
    pub shell_length: f32,
    /// Carapace width.
    pub shell_width: f32,
    /// Carapace height (domed tortoises ≈ 0.5 of the length).
    pub shell_height: f32,
    /// Head radius.
    pub head: f32,
    /// Visible neck length.
    pub neck: f32,
    /// Leg length.
    pub leg: f32,
    /// Paddle-shaped flippers (sea turtles) instead of columnar legs.
    pub flippers: bool,
    /// Carapace colour.
    pub shell: u32,
    /// Scute seam colour.
    pub seams: u32,
    /// Skin colour.
    pub skin: u32,
    /// Plastron colour.
    pub plastron: u32,
}

/// Builds a turtle; the root carries the shell.
pub fn build_turtle(p: &TurtleParams, mass_kg: f32, seed: u32, detail: Detail) -> Rig {
    let (sl, sw, sh) = (p.shell_length, p.shell_width, p.shell_height);
    let motion = RigMotion {
        locomotor: Locomotor::Chelonian,
        gallop: GallopStyle::Amble,
        mass_kg,
        hip_height: p.leg,
        body_length: sl,
        rest_drop: 0.8 * p.leg,
        leg_lengths: vec![(0.5 * p.leg, 0.5 * p.leg), (0.5 * p.leg, 0.5 * p.leg)],
        leg_reach: Vec::new(),
        half_width: 0.5 * sw,
        half_span: 0.0,
        flap_hz: 0.0,
        flap_amplitude: 0.0,
        can_fly: false,
        aquatic: false,
        graze_pitch: 0.4,
        segments: 0,
        wave_length: 1.0,
        wave_amplitude: 0.0,
        vertical_undulation: false,
    };
    let mut rig = Rig::new(Transform::from_xyz(0.0, 0.85 * p.leg + 0.12 * sh, 0.0), motion, srgb(p.shell));
    let (shell_c, seam_c, plast_c) = (srgb(p.shell), srgb(p.seams), srgb(p.plastron));
    let mut shell = MeshData::new();
    let n = detail.rings(10);
    let rings: Vec<Ring> = (0..=n)
        .map(|i| {
            let u = i as f32 / n as f32;
            let s = (PI * u).sin().powf(0.55).max(0.18);
            Ring::asym(Vec3::new(0.0, 0.0, (u - 0.5) * sl), 0.5 * sw * s, sh * s.powf(0.8), 0.12 * sh * s)
                .with_exponent(2.2)
        })
        .collect();
    loft(&mut shell, &rings, detail.around(18), Vec3::Y, Cap::Round, Cap::Round, &move |sp: &SurfacePoint| {
        if sp.dorsal < -0.2 {
            return plast_c;
        }
        let q = bevy::math::Vec2::new(sp.position.z, sp.position.x) / (0.22 * sl);
        let (f1, f2, h) = worley2_f2(q, seed);
        let edge = (1.0 - (f2 - f1) / 0.12).clamp(0.0, 1.0);
        let tone = 0.85 + 0.25 * ((h & 0xff) as f32 / 255.0);
        mix(shade(shell_c, tone), seam_c, edge)
    });
    rig.parts[0].mesh = shell;
    // Neck + head.
    let skin = p.skin;
    let neck = add_part(
        &mut rig,
        0,
        PartRole::Neck(0),
        Transform::from_xyz(0.0, 0.05 * sh, 0.42 * sl).with_rotation(Quat::from_rotation_x(-0.3)),
        PartMaterial::Matte,
        |m, _| {
            tube(
                m,
                &[Vec3::new(0.0, 0.0, -0.1 * sl), Vec3::new(0.0, 0.0, p.neck)],
                &[0.75 * p.head, 0.6 * p.head],
                detail.around(10),
                Vec3::Y,
                (Cap::Round, Cap::Round),
                &solid(skin),
            );
        },
    );
    let head = add_part(
        &mut rig,
        neck,
        PartRole::Head,
        Transform::from_xyz(0.0, 0.0, p.neck).with_rotation(Quat::from_rotation_x(0.35)),
        PartMaterial::Matte,
        |m, _| {
            superellipsoid(
                m,
                Vec3::new(0.0, 0.1 * p.head, 0.55 * p.head),
                Vec3::new(0.85 * p.head, 0.8 * p.head, 1.15 * p.head),
                0.9,
                1.0,
                detail.rings(8),
                detail.around(10),
                &solid(skin),
            );
        },
    );
    let e = Vec3::new(0.62 * p.head, 0.35 * p.head, 0.95 * p.head);
    eye(&mut rig, head, e, 0.2 * p.head, 0x2a1a08, detail);
    eye(&mut rig, head, e * Vec3::new(-1.0, 1.0, 1.0), 0.2 * p.head, 0x2a1a08, detail);
    // Legs.
    for pair in 0..2u8 {
        for side in Side::BOTH {
            let s = side.sign();
            let z = if pair == 0 { 0.3 * sl } else { -0.32 * sl };
            let rest = if p.flippers {
                Quat::from_rotation_z(s * (FRAC_PI_2 - 0.1)) * Quat::from_rotation_x(if pair == 0 { -0.5 } else { 0.6 })
            } else {
                Quat::from_rotation_z(s * 0.45)
            };
            let len = if p.flippers && pair == 0 { 1.6 * p.leg } else { p.leg };
            let r = 0.45 * p.head;
            let flippers = p.flippers;
            let upper = add_part(
                &mut rig,
                0,
                PartRole::Leg { pair, side, segment: LegSegment::Upper },
                Transform::from_xyz(s * 0.36 * sw, -0.02 * sh, z).with_rotation(rest),
                PartMaterial::Matte,
                |m, _| {
                    tube(
                        m,
                        &[Vec3::ZERO, Vec3::new(0.0, -0.5 * len, 0.0)],
                        &[r, 0.9 * r],
                        detail.around(8),
                        Vec3::Z,
                        (Cap::Round, Cap::Round),
                        &solid(skin),
                    );
                },
            );
            add_part(
                &mut rig,
                upper,
                PartRole::Leg { pair, side, segment: LegSegment::Lower },
                Transform::from_xyz(0.0, -0.5 * len, 0.0)
                    .with_rotation(Quat::from_rotation_z(-s * if flippers { 0.0 } else { 0.45 })),
                PartMaterial::Matte,
                |m, _| {
                    if flippers {
                        let rings = [
                            Ring::ellipse(Vec3::ZERO, 0.9 * r, 0.4 * r),
                            Ring::ellipse(Vec3::new(0.0, -0.3 * len, 0.0), 1.6 * r, 0.25 * r),
                            Ring::ellipse(Vec3::new(0.0, -0.5 * len, 0.0), 0.5 * r, 0.15 * r),
                        ];
                        loft(m, &rings, detail.around(8), Vec3::Z, Cap::Round, Cap::Round, &solid(skin));
                    } else {
                        tube(
                            m,
                            &[Vec3::ZERO, Vec3::new(0.0, -0.5 * len, 0.0)],
                            &[0.9 * r, 1.05 * r],
                            detail.around(8),
                            Vec3::Z,
                            (Cap::Round, Cap::Flat),
                            &solid(skin),
                        );
                    }
                },
            );
        }
    }
    if detail.small_parts() {
        add_part(
            &mut rig,
            0,
            PartRole::Tail(0),
            Transform::from_xyz(0.0, 0.0, -0.45 * sl).with_rotation(Quat::from_rotation_x(-0.4)),
            PartMaterial::Matte,
            |m, _| {
                tube(
                    m,
                    &[Vec3::ZERO, Vec3::new(0.0, 0.0, -0.15 * sl)],
                    &[0.3 * p.head, 0.05 * p.head],
                    6,
                    Vec3::Y,
                    (Cap::Round, Cap::Point(0.02 * sl)),
                    &solid(skin),
                );
            },
        );
    }
    rig
}

/// Parameters of a snail (adult, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnailParams {
    /// Foot length.
    pub foot_length: f32,
    /// Shell radius (outermost whorl).
    pub shell_radius: f32,
    /// Number of whorls.
    pub whorls: f32,
    /// Shell colour.
    pub shell: u32,
    /// Spiral band colour.
    pub band: u32,
    /// Body colour.
    pub body: u32,
}

/// Builds a snail: a logarithmic-spiral shell (whorl radius `r = a·e^{bθ}`)
/// coiled in the sagittal plane, its largest (body) whorl and aperture
/// resting on the back of a gliding foot with two eye stalks.
///
/// Rig: the root is a pivot on the ground under the shell; the foot is
/// `Segment(0)` (it stretches while crawling and contracts into the shell
/// when withdrawing) and carries the eye stalks; the shell is a rigid child
/// of the root so foot deformations never distort it.
pub fn build_snail(p: &SnailParams, mass_kg: f32, detail: Detail) -> Rig {
    let fl = p.foot_length;
    let motion = RigMotion {
        locomotor: Locomotor::Gastropod,
        gallop: GallopStyle::Bound,
        mass_kg,
        hip_height: 0.1 * fl,
        body_length: fl,
        rest_drop: 0.0,
        leg_lengths: Vec::new(),
        leg_reach: Vec::new(),
        half_width: 0.14 * fl,
        half_span: 0.0,
        flap_hz: 0.0,
        flap_amplitude: 0.0,
        can_fly: false,
        aquatic: false,
        graze_pitch: 0.2,
        segments: 1,
        wave_length: 0.5,
        wave_amplitude: 0.05,
        vertical_undulation: false,
    };
    // The shell sits slightly behind mid-foot; the foot contracts towards it.
    let shell_z = -0.1 * fl;
    let foot_y = 0.08 * fl;
    let foot_top = foot_y + 0.1 * fl;
    let mut rig = Rig::new(Transform::IDENTITY, motion, srgb(p.shell));
    let body = srgb(p.body);
    let mut foot = MeshData::new();
    let rings: Vec<Ring> = (0..=6)
        .map(|i| {
            let u = i as f32 / 6.0;
            Ring::asym(
                Vec3::new(0.0, 0.0, (u - 0.55) * fl - shell_z),
                0.14 * fl * (1.0 - 0.6 * (2.0 * u - 1.2).abs().min(1.0)).max(0.35),
                0.1 * fl,
                0.08 * fl,
            )
        })
        .collect();
    loft(&mut foot, &rings, detail.around(10), Vec3::Y, Cap::Point(0.08 * fl), Cap::Round, &move |sp| {
        shade(body, 0.85 + 0.2 * sp.dorsal)
    });
    let foot_index =
        rig.add(0, PartRole::Segment(0), Transform::from_xyz(0.0, foot_y, shell_z), foot, PartMaterial::Matte);

    // Shell: a tube swept along a logarithmic spiral in the sagittal (y, z)
    // plane around a lateral coiling axis, with a small helical offset that
    // raises the spire to the left (dextral shell). Growth runs over the top
    // and down the back so the last, largest whorl lies along the bottom and
    // the aperture opens forwards onto the foot.
    let (shell_c, band_c) = (srgb(p.shell), srgb(p.band));
    let mut shell = MeshData::new();
    let turns = p.whorls.clamp(1.5, 8.0);
    let steps = (detail.rings(28) as f32 * turns / 2.0).ceil().max(8.0) as usize;
    let b = 0.14f32;
    let total = turns * TAU;
    // The last whorl's centreline radius; with the tube radius (0.42 r) the
    // shell's height is about 1.8 shell radii.
    let a = p.shell_radius * 0.8 / (b * total).exp();
    let psi_end = -FRAC_PI_2 + 0.2;
    let phase = psi_end - total;
    let (mut pts, mut radii) = (Vec::new(), Vec::new());
    for i in 0..=steps {
        let theta = total * i as f32 / steps as f32;
        let r = a * (b * theta).exp();
        let psi = theta + phase;
        pts.push(Vec3::new(0.3 * p.shell_radius * (1.0 - theta / total), r * psi.sin(), r * psi.cos()));
        radii.push(0.42 * r + 0.002 * p.shell_radius);
    }
    tube(&mut shell, &pts, &radii, detail.around(10), Vec3::X, (Cap::Round, Cap::Flat), &move |sp: &SurfacePoint| {
        let band = (sp.around - 0.25).abs() < 0.06;
        if band { band_c } else { shade(shell_c, 0.75 + 0.35 * sp.along) }
    });
    // Rest the body whorl on the foot, sunk by a fifth of the aperture radius.
    let aperture_r = radii.last().copied().unwrap_or(0.0);
    let lowest = shell.bounds().map_or(0.0, |(lo, _)| lo.y);
    let shell_y = foot_top - 0.2 * aperture_r - lowest;
    rig.add(0, PartRole::Static, Transform::from_xyz(0.0, shell_y, shell_z), shell, PartMaterial::Glossy);

    for side in Side::BOTH {
        let s = side.sign();
        add_part(
            &mut rig,
            foot_index,
            PartRole::Antenna(side),
            Transform::from_xyz(s * 0.04 * fl, 0.06 * fl, 0.38 * fl - shell_z),
            PartMaterial::Matte,
            |m, _| {
                let tip = Vec3::new(s * 0.1 * fl, 0.32 * fl, 0.15 * fl);
                tube(
                    m,
                    &[Vec3::ZERO, tip],
                    &[0.025 * fl, 0.015 * fl],
                    5,
                    Vec3::Z,
                    (Cap::Round, Cap::Open),
                    &move |_| body,
                );
                superellipsoid(m, tip, Vec3::splat(0.025 * fl), 1.0, 1.0, 4, 6, &solid(0x1a1410));
            },
        );
    }
    rig
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frog_turtle_snail_build() {
        let frog = build_frog(
            &FrogParams {
                length: 0.07,
                width: 0.04,
                depth: 0.025,
                hind_leg: 0.1,
                coat: Coat::countershaded(0x3a8a30, 0xe8e0a0),
                warts: false,
                iris: 0xd0a020,
            },
            0.03,
            1,
            Detail::Full,
        );
        frog.validate().unwrap();
        let turtle = build_turtle(
            &TurtleParams {
                shell_length: 0.25,
                shell_width: 0.18,
                shell_height: 0.09,
                head: 0.025,
                neck: 0.04,
                leg: 0.05,
                flippers: false,
                shell: 0x4a4a30,
                seams: 0xd0c080,
                skin: 0x5a6a40,
                plastron: 0xc0a060,
            },
            1.0,
            1,
            Detail::Full,
        );
        turtle.validate().unwrap();
        let snail = build_snail(
            &SnailParams {
                foot_length: 0.04,
                shell_radius: 0.012,
                whorls: 3.0,
                shell: 0x8a6030,
                band: 0x3a2010,
                body: 0xb0a080,
            },
            0.01,
            Detail::Full,
        );
        snail.validate().unwrap();
        assert!(snail.triangle_count() > 100);
    }

    /// Rig-space bounds of one part.
    fn part_bounds(rig: &Rig, i: usize) -> (Vec3, Vec3) {
        let t = rig.rest_model_transform(i);
        rig.parts[i]
            .mesh
            .positions
            .iter()
            .map(|p| t.transform_point(Vec3::from(*p)))
            .fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)))
    }

    #[test]
    fn snail_shell_rests_on_the_foot_and_coils_sideways() {
        let p = SnailParams {
            foot_length: 0.05,
            shell_radius: 0.016,
            whorls: 3.5,
            shell: 0x9a6a3a,
            band: 0x3a2414,
            body: 0xa89878,
        };
        let mut rig = build_snail(&p, 0.01, Detail::Full);
        rig.ground();
        let foot = rig.find(PartRole::Segment(0)).unwrap();
        let shell = rig.parts.iter().position(|q| q.role == PartRole::Static && q.parent == Some(0)).unwrap();
        let (flo, fhi) = part_bounds(&rig, foot);
        let (slo, shi) = part_bounds(&rig, shell);
        // The shell touches the foot's back: no gap, and not buried in it.
        assert!(slo.y < fhi.y && slo.y > flo.y + 0.5 * (fhi.y - flo.y), "shell {slo} foot {flo}..{fhi}");
        // Seen from the side the spiral is visible: the coiling plane is
        // sagittal, so the shell is much thinner across than along/up.
        let size = shi - slo;
        assert!(size.x < 0.8 * size.y.max(size.z) && size.x < size.z, "{size}");
        // Real proportions: the shell is about as tall as its diameter
        // (1.5-2 shell radii) and covers over half the foot length.
        assert!((1.5 * p.shell_radius..2.0 * p.shell_radius).contains(&size.z.max(size.y)), "{size}");
        assert!(size.z > 0.4 * p.foot_length, "{size}");
        // The shell is a child of the root, never of the deforming foot.
        assert_eq!(rig.parts[shell].parent, Some(0));
        assert!(rig.parts.iter().all(|q| !matches!(q.role, PartRole::Antenna(_)) || q.parent == Some(foot)));
    }
}
