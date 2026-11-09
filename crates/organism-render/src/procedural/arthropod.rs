//! Arthropod body plan: insects (head, thorax, abdomen, six legs, antennae,
//! membranous, scaled or sclerotised wings) and spiders (cephalothorax,
//! abdomen, eight legs). Segmentation, leg count and wing type are the
//! recognition cues at small sizes.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::math::{Quat, Vec2, Vec3};
use bevy::prelude::Transform;

use super::mesh::{Cap, MeshData, Rgba, SurfacePoint, lump, superellipsoid, tube};
use super::noise::worley2;
use super::palette::{mix, shade, srgb};
use super::parts::{Detail, add_part, smooth_outline, solid, wing_panel};
use super::rig::{GallopStyle, LegSegment, Locomotor, PartMaterial, PartRole, Rig, RigMotion, Side, WingSegment};

/// Antenna form.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Antennae {
    /// None (spiders).
    None,
    /// Thin, straight or gently curved (beetles, grasshoppers).
    Straight(f32),
    /// Elbowed (ants, bees).
    Elbowed(f32),
    /// Clubbed tips (butterflies).
    Clubbed(f32),
}

/// Wing equipment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wings {
    /// Wingless.
    None,
    /// Transparent membranous wings (bees, flies, dragonflies).
    Membrane {
        /// Pairs (1 or 2).
        pairs: u8,
        /// Fore-wing length.
        length: f32,
        /// Fore-wing chord.
        width: f32,
        /// Tint (`0xRRGGBB`).
        tint: u32,
    },
    /// Hardened elytra over the abdomen (beetles).
    Elytra {
        /// Elytra colour.
        color: u32,
        /// Spot colour (ladybirds), if any.
        spots: Option<u32>,
    },
    /// Scaled, patterned wings (butterflies, moths).
    Scaled {
        /// Fore-wing length.
        length: f32,
        /// Ground colour.
        base: u32,
        /// Vein and border colour.
        border: u32,
        /// Spot colour in the border.
        spots: u32,
    },
}

/// How wings are held when not flying.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WingRest {
    /// Folded back over the abdomen (bees, flies).
    FoldedBack,
    /// Closed vertically above the back (butterflies).
    Upright,
    /// Held flat and spread (dragonflies).
    Spread,
}

/// Parameters of an arthropod (adult, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArthropodParams {
    /// Head radii (x, y, z).
    pub head: Vec3,
    /// Thorax / cephalothorax radii.
    pub thorax: Vec3,
    /// Abdomen radii.
    pub abdomen: Vec3,
    /// Petiole (waist) length; 0 = broad junction.
    pub waist: f32,
    /// Leg pairs (3 insects, 4 spiders).
    pub leg_pairs: u8,
    /// Length of one leg (both segments).
    pub leg_length: f32,
    /// Leg radius.
    pub leg_radius: f32,
    /// Hind-leg length multiplier (grasshoppers).
    pub hind_legs: f32,
    /// Antennae.
    pub antennae: Antennae,
    /// Wings.
    pub wings: Wings,
    /// Wing posture at rest.
    pub wing_rest: WingRest,
    /// Mandible length (stag beetles, ants).
    pub mandibles: f32,
    /// Head colour.
    pub head_color: u32,
    /// Thorax colour.
    pub thorax_color: u32,
    /// Abdomen colour.
    pub abdomen_color: u32,
    /// Abdomen bands (colour, bands per metre).
    pub bands: Option<(u32, f32)>,
    /// Fuzzy thorax/abdomen (bees).
    pub fuzzy: bool,
    /// Leg colour.
    pub leg_color: u32,
    /// Compound-eye colour.
    pub eye_color: u32,
    /// Display wing-beat frequency (Hz).
    pub flap_hz: f32,
}

impl ArthropodParams {
    /// Generic small beetle.
    pub const BASE: ArthropodParams = ArthropodParams {
        head: Vec3::new(0.0012, 0.001, 0.0012),
        thorax: Vec3::new(0.002, 0.0016, 0.002),
        abdomen: Vec3::new(0.0025, 0.002, 0.0035),
        waist: 0.0,
        leg_pairs: 3,
        leg_length: 0.006,
        leg_radius: 0.00025,
        hind_legs: 1.0,
        antennae: Antennae::Straight(0.004),
        wings: Wings::None,
        wing_rest: WingRest::FoldedBack,
        mandibles: 0.0,
        head_color: 0x1a1a1a,
        thorax_color: 0x202020,
        abdomen_color: 0x2a2a2a,
        bands: None,
        fuzzy: false,
        leg_color: 0x1a1410,
        eye_color: 0x101010,
        flap_hz: 0.0,
    };
}

#[allow(clippy::too_many_arguments)]
fn glossy_body(
    m: &mut MeshData,
    center: Vec3,
    radii: Vec3,
    color: u32,
    fuzzy: bool,
    bands: Option<(u32, f32)>,
    seed: u32,
    detail: Detail,
) {
    let c = srgb(color);
    let band = bands.map(|(b, per_m)| (srgb(b), per_m));
    let paint = move |z: f32| -> Rgba {
        match band {
            Some((bc, per_m)) => {
                if ((z * per_m).rem_euclid(1.0)) < 0.45 {
                    bc
                } else {
                    c
                }
            }
            None => c,
        }
    };
    if fuzzy {
        lump(m, center, radii, if detail.small_parts() { 2 } else { 1 }, 0.12, 9.0, seed, &move |d, k| {
            shade(paint(center.z + d.z * radii.z), 0.8 + 0.25 * k)
        });
    } else {
        superellipsoid(m, center, radii, 1.0, 1.0, detail.rings(9), detail.around(12), &move |sp: &SurfacePoint| {
            shade(paint(sp.position.z), 0.85 + 0.25 * sp.dorsal.max(0.0))
        });
    }
}

/// Builds an arthropod rig. The root is the thorax.
pub fn build(p: &ArthropodParams, mass_kg: f32, seed: u32, detail: Detail) -> Rig {
    let spider = p.leg_pairs >= 4;
    let flies = !matches!(p.wings, Wings::None) && p.flap_hz > 0.0;
    let leg_upper = 0.42 * p.leg_length;
    let leg_lower = 0.58 * p.leg_length;
    let motion = RigMotion {
        locomotor: if spider { Locomotor::Octopod } else { Locomotor::Hexapod },
        gallop: GallopStyle::Bound,
        mass_kg,
        hip_height: 0.45 * p.leg_length,
        body_length: 2.0 * (p.head.z + p.thorax.z + p.abdomen.z) + p.waist,
        rest_drop: 0.3 * p.leg_length,
        leg_lengths: (0..p.leg_pairs.max(1))
            .map(|i| {
                if i + 1 == p.leg_pairs && !spider {
                    (leg_upper * p.hind_legs, leg_lower * p.hind_legs)
                } else {
                    (leg_upper, leg_lower)
                }
            })
            .collect(),
        half_width: p.thorax.x.max(p.abdomen.x),
        leg_reach: Vec::new(),
        half_span: match p.wings {
            Wings::Membrane { length, .. } | Wings::Scaled { length, .. } => length,
            _ => 0.0,
        },
        flap_hz: p.flap_hz,
        flap_amplitude: if matches!(p.wings, Wings::Scaled { .. }) { 1.1 } else { 0.9 },
        can_fly: flies,
        aquatic: false,
        graze_pitch: 0.25,
        segments: 1,
        wave_length: 1.0,
        wave_amplitude: 0.0,
        vertical_undulation: false,
    };
    let mut rig = Rig::new(Transform::from_xyz(0.0, 0.4 * p.leg_length, 0.0), motion, srgb(p.abdomen_color));
    let mut thorax = MeshData::new();
    glossy_body(&mut thorax, Vec3::ZERO, p.thorax, p.thorax_color, p.fuzzy, None, seed, detail);
    rig.parts[0].mesh = thorax;

    // Head with compound eyes, mandibles and antennae.
    let head_pos = Vec3::new(0.0, 0.15 * p.thorax.y, p.thorax.z + 0.75 * p.head.z);
    let head =
        add_part(&mut rig, 0, PartRole::Head, Transform::from_translation(head_pos), PartMaterial::Glossy, |m, _| {
            glossy_body(m, Vec3::ZERO, p.head, p.head_color, false, None, seed, detail);
            let ec = srgb(p.eye_color);
            let eye_r = if spider { 0.18 * p.head.y } else { 0.55 * p.head.y };
            let eyes: &[(f32, f32, f32)] =
                if spider { &[(0.3, 0.6, 0.85), (0.12, 0.75, 0.9), (0.45, 0.4, 0.7)] } else { &[(0.75, 0.25, 0.35)] };
            for &(x, y, z) in eyes {
                for s in [-1.0f32, 1.0] {
                    superellipsoid(
                        m,
                        Vec3::new(s * x * p.head.x, y * p.head.y, z * p.head.z),
                        Vec3::splat(eye_r),
                        1.0,
                        1.0,
                        5,
                        detail.around(8),
                        &move |_| ec,
                    );
                }
            }
            if p.mandibles > 0.0 && detail.small_parts() {
                for s in [-1.0f32, 1.0] {
                    let base = Vec3::new(s * 0.4 * p.head.x, -0.3 * p.head.y, 0.85 * p.head.z);
                    let pts = [
                        base,
                        base + Vec3::new(s * 0.25, 0.05, 0.9) * p.mandibles,
                        base + Vec3::new(-s * 0.15, 0.08, 1.15) * p.mandibles,
                    ];
                    tube(
                        m,
                        &pts,
                        &[0.18 * p.head.x, 0.12 * p.head.x, 0.03 * p.head.x],
                        6,
                        Vec3::Y,
                        (Cap::Round, Cap::Point(0.02 * p.mandibles)),
                        &solid(p.head_color),
                    );
                }
            }
        });
    if p.antennae != Antennae::None {
        for side in Side::BOTH {
            let s = side.sign();
            add_part(
                &mut rig,
                head,
                PartRole::Antenna(side),
                Transform::from_xyz(s * 0.35 * p.head.x, 0.6 * p.head.y, 0.8 * p.head.z),
                PartMaterial::Matte,
                |m, _| {
                    antenna(m, p.antennae, s, p.head.x, p.head_color, detail);
                },
            );
        }
    }

    // Abdomen (with petiole for ants/wasps).
    let abd_pos =
        Vec3::new(0.0, if spider { 0.25 * p.thorax.y } else { 0.0 }, -p.thorax.z - p.waist - 0.85 * p.abdomen.z);
    add_part(
        &mut rig,
        0,
        PartRole::Segment(0),
        Transform::from_translation(abd_pos).with_rotation(Quat::from_rotation_x(if spider { -0.15 } else { 0.08 })),
        PartMaterial::Glossy,
        |m, _| {
            glossy_body(m, Vec3::ZERO, p.abdomen, p.abdomen_color, p.fuzzy, p.bands, seed ^ 0x55, detail);
            if p.waist > 0.0 {
                tube(
                    m,
                    &[
                        Vec3::new(0.0, 0.0, 0.8 * p.abdomen.z),
                        Vec3::new(0.0, 0.0, 0.85 * p.abdomen.z + p.waist + 0.3 * p.thorax.z),
                    ],
                    &[0.18 * p.thorax.y, 0.16 * p.thorax.y],
                    6,
                    Vec3::Y,
                    (Cap::Round, Cap::Round),
                    &solid(p.thorax_color),
                );
            }
            if let Wings::Elytra { color, spots } = p.wings {
                // Domed wing cases covering the abdomen, split along the midline.
                let ec = srgb(color);
                let spot = spots.map(srgb);
                for s in [-1.0f32, 1.0] {
                    let r = Vec3::new(0.55 * p.abdomen.x, 1.08 * p.abdomen.y, 1.08 * p.abdomen.z);
                    superellipsoid(
                        m,
                        Vec3::new(s * 0.47 * p.abdomen.x, 0.12 * p.abdomen.y, 0.12 * p.abdomen.z),
                        r,
                        1.0,
                        0.85,
                        detail.rings(9),
                        detail.around(10),
                        &move |sp: &SurfacePoint| {
                            let mut c = shade(ec, 0.8 + 0.35 * sp.dorsal.max(0.0));
                            if let Some(sc) = spot {
                                let (d, _) = worley2(Vec2::new(sp.position.z, sp.position.x) / r.x * 1.4, 21);
                                if d < 0.28 && sp.dorsal > -0.2 {
                                    c = sc;
                                }
                            }
                            c
                        },
                    );
                }
            }
        },
    );

    // Legs.
    let pairs = p.leg_pairs.max(1);
    for pair in 0..pairs {
        let u = if pairs == 1 { 0.5 } else { pair as f32 / (pairs - 1) as f32 };
        let z = p.thorax.z * (0.55 - 1.1 * u);
        let yaw = if spider { 0.95 - 1.9 * u } else { 0.7 - 1.4 * u };
        let (lu, ll) = rig.motion.leg_lengths[pair as usize];
        for side in Side::BOTH {
            let s = side.sign();
            // Positive yaw angles the fore legs forwards and negative the
            // hind legs backwards (a yaw of −s·α turns a leg on side s
            // towards +Z, the head).
            let rest = Quat::from_rotation_y(-s * yaw)
                * Quat::from_rotation_z(s * (FRAC_PI_2 + if spider { 0.75 } else { 0.45 }));
            let attach = Vec3::new(s * 0.75 * p.thorax.x, -0.35 * p.thorax.y, z);
            let lc = srgb(p.leg_color);
            let r = p.leg_radius * if pair + 1 == pairs && p.hind_legs > 1.0 { 1.8 } else { 1.0 };
            let upper = add_part(
                &mut rig,
                0,
                PartRole::Leg { pair, side, segment: LegSegment::Upper },
                Transform::from_translation(attach).with_rotation(rest),
                PartMaterial::Matte,
                |m, _| {
                    tube(
                        m,
                        &[Vec3::ZERO, Vec3::new(0.0, -lu, 0.0)],
                        &[r * 1.3, r],
                        detail.around(6),
                        Vec3::Z,
                        (Cap::Round, Cap::Round),
                        &move |_| lc,
                    );
                },
            );
            // Tibiae splay outwards (~35° from vertical) so the body hangs low.
            let knee = -s * (if spider { 1.85 } else { 1.42 });
            add_part(
                &mut rig,
                upper,
                PartRole::Leg { pair, side, segment: LegSegment::Lower },
                Transform::from_xyz(0.0, -lu, 0.0).with_rotation(Quat::from_rotation_z(knee)),
                PartMaterial::Matte,
                |m, _| {
                    tube(
                        m,
                        &[Vec3::ZERO, Vec3::new(0.0, -ll, 0.0)],
                        &[r, 0.6 * r],
                        detail.around(6),
                        Vec3::Z,
                        (Cap::Round, Cap::Round),
                        &move |_| shade(lc, 0.9),
                    );
                },
            );
        }
    }

    // Lever arm of each pair's yaw swing: lateral foot offset from the coxa.
    rig.motion.leg_reach = (0..pairs)
        .map(|pair| {
            let upper = rig.find(PartRole::Leg { pair, side: Side::Left, segment: LegSegment::Upper });
            let lower = rig.find(PartRole::Leg { pair, side: Side::Left, segment: LegSegment::Lower });
            match (upper, lower) {
                (Some(u), Some(l)) => {
                    let coxa = rig.rest_model_transform(u).translation;
                    let lower_len = rig.motion.leg_lengths[pair as usize].1;
                    let tip = rig.rest_model_transform(l).transform_point(Vec3::new(0.0, -lower_len, 0.0));
                    (tip.x - coxa.x).abs().max(1e-5)
                }
                _ => 0.8 * p.leg_length,
            }
        })
        .collect();

    // Wings.
    match p.wings {
        Wings::Membrane { pairs, length, width, tint } => {
            for pair in 0..pairs.clamp(1, 2) {
                let (len, wid) = if pair == 0 { (length, width) } else { (0.85 * length, 0.9 * width) };
                let outline: Vec<Vec2> = (0..20)
                    .map(|k| {
                        let a = k as f32 / 20.0 * 2.0 * PI;
                        Vec2::new(0.5 * len * (1.0 + a.cos()), 0.5 * wid * a.sin() * (0.75 + 0.25 * a.cos()))
                    })
                    .collect();
                let tc = srgb(tint);
                let z = 0.3 * p.thorax.z - 0.55 * pair as f32 * p.thorax.z;
                for side in Side::BOTH {
                    let s = side.sign();
                    add_part(
                        &mut rig,
                        0,
                        PartRole::Wing { pair, side, segment: WingSegment::Inner },
                        Transform::from_xyz(s * 0.4 * p.thorax.x, 0.85 * p.thorax.y, z),
                        PartMaterial::Translucent,
                        |m, _| {
                            wing_panel(m, &outline, s, 0.02 * wid, 2, &move |q, _| {
                                let vein = (q.y.abs() < 0.04 * wid) || (q.x < 0.05 * len);
                                if vein { [0.08, 0.07, 0.06, 0.9] } else { [tc[0], tc[1], tc[2], 0.32] }
                            });
                        },
                    );
                }
            }
        }
        Wings::Scaled { length, base, border, spots } => {
            let (bc, rc, sc) = (srgb(base), srgb(border), srgb(spots));
            for pair in 0..2u8 {
                let control: Vec<Vec2> = if pair == 0 {
                    vec![
                        Vec2::new(0.02 * length, 0.1 * length),
                        Vec2::new(0.55 * length, 0.27 * length),
                        Vec2::new(length, 0.22 * length),
                        Vec2::new(0.93 * length, -0.2 * length),
                        Vec2::new(0.5 * length, -0.33 * length),
                        Vec2::new(0.05 * length, -0.12 * length),
                    ]
                } else {
                    vec![
                        Vec2::new(0.03 * length, -0.05 * length),
                        Vec2::new(0.45 * length, -0.12 * length),
                        Vec2::new(0.7 * length, -0.42 * length),
                        Vec2::new(0.5 * length, -0.78 * length),
                        Vec2::new(0.18 * length, -0.72 * length),
                        Vec2::new(0.02 * length, -0.35 * length),
                    ]
                };
                let outline = smooth_outline(&control, if detail.small_parts() { 5 } else { 2 });
                for side in Side::BOTH {
                    let s = side.sign();
                    add_part(
                        &mut rig,
                        0,
                        PartRole::Wing { pair, side, segment: WingSegment::Inner },
                        Transform::from_xyz(s * 0.45 * p.thorax.x, 0.8 * p.thorax.y, 0.2 * p.thorax.z),
                        PartMaterial::Matte,
                        |m, _| {
                            wing_panel(m, &outline, s, 0.01 * length, detail.rings(9), &move |q, f| {
                                butterfly_color(q, f, length, bc, rc, sc)
                            });
                        },
                    );
                }
            }
        }
        Wings::None | Wings::Elytra { .. } => {}
    }
    rig
}

/// Lepidopteran wing pattern: dark veins radiating from the wing base, a
/// dark margin with a row of pale spots, ground colour brightening outwards.
fn butterfly_color(q: Vec2, f: f32, length: f32, base: Rgba, border: Rgba, spots: Rgba) -> Rgba {
    if f > 0.8 {
        let k = ((q.x - q.y) / length * 7.0).rem_euclid(1.0);
        if (0.86..0.95).contains(&f) && (0.3..0.65).contains(&k) { spots } else { border }
    } else {
        let angle = q.y.atan2(q.x.max(1e-6));
        let vein = ((angle * 4.5).rem_euclid(1.0) - 0.5).abs() < 0.06 && f > 0.15;
        if vein { mix(base, border, 0.9) } else { shade(base, 0.8 + 0.3 * f) }
    }
}

fn antenna(m: &mut MeshData, kind: Antennae, s: f32, head: f32, color: u32, detail: Detail) {
    let r = (0.08 * head).max(0.00004);
    let c = srgb(color);
    let col = move |_: &SurfacePoint| c;
    match kind {
        Antennae::None => {}
        Antennae::Straight(len) => {
            let pts = [
                Vec3::ZERO,
                Vec3::new(s * 0.25 * len, 0.35 * len, 0.5 * len),
                Vec3::new(s * 0.5 * len, 0.5 * len, 0.85 * len),
            ];
            tube(m, &pts, &[r, 0.8 * r, 0.5 * r], 4, Vec3::Y, (Cap::Round, Cap::Round), &col);
        }
        Antennae::Elbowed(len) => {
            let elbow = Vec3::new(s * 0.15 * len, 0.45 * len, 0.1 * len);
            tube(m, &[Vec3::ZERO, elbow], &[r, 0.9 * r], 4, Vec3::Y, (Cap::Round, Cap::Round), &col);
            tube(
                m,
                &[elbow, elbow + Vec3::new(s * 0.25 * len, 0.05 * len, 0.5 * len)],
                &[0.9 * r, 0.8 * r],
                4,
                Vec3::Y,
                (Cap::Round, Cap::Round),
                &col,
            );
        }
        Antennae::Clubbed(len) => {
            let tip = Vec3::new(s * 0.35 * len, 0.6 * len, 0.75 * len);
            tube(m, &[Vec3::ZERO, tip], &[0.6 * r, 0.5 * r], 4, Vec3::Y, (Cap::Round, Cap::Round), &col);
            if detail.small_parts() {
                superellipsoid(m, tip, Vec3::new(2.0 * r, 2.0 * r, 3.0 * r), 1.0, 1.0, 4, 5, &col);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insect_has_six_legs_and_spider_eight() {
        let ant = build(&ArthropodParams::BASE, 1e-5, 1, Detail::Full);
        ant.validate().unwrap();
        let legs = |r: &Rig| {
            r.parts.iter().filter(|p| matches!(p.role, PartRole::Leg { segment: LegSegment::Lower, .. })).count()
        };
        assert_eq!(legs(&ant), 6);
        let spider = build(
            &ArthropodParams { leg_pairs: 4, antennae: Antennae::None, ..ArthropodParams::BASE },
            1e-4,
            1,
            Detail::Full,
        );
        assert_eq!(legs(&spider), 8);
        assert_eq!(spider.motion.locomotor, Locomotor::Octopod);
    }

    #[test]
    fn butterfly_wings_are_patterned() {
        let (b, r, s) = (srgb(0xe07010), srgb(0x101010), srgb(0xffffff));
        let inner = butterfly_color(Vec2::new(0.3, 0.05), 0.4, 0.05, b, r, s);
        let edge = butterfly_color(Vec2::new(0.04, 0.0), 0.9, 0.05, b, r, s);
        assert_ne!(inner, edge);
    }
}
