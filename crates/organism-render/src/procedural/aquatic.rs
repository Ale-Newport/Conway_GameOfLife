//! Undulating body plans: fish, cetaceans and rays (segmented fusiform
//! bodies with fins) and limbless serpentine bodies (snakes, worms).
//!
//! Bodies are split into rigid segments whose ends are rounded and overlap,
//! so moderate joint angles of a travelling wave read as one continuous body.

use std::f32::consts::PI;

use bevy::math::{Quat, Vec2, Vec3};
use bevy::prelude::Transform;

use super::mesh::{Cap, MeshData, Ring, SurfacePoint, loft, planform, tube};
use super::palette::{Coat, mix, shade, srgb};
use super::parts::{Detail, add_part, coat_color, eye, solid, wing_panel};
use super::rig::{GallopStyle, Locomotor, PartMaterial, PartRole, Rig, RigMotion, Side};

/// Caudal (tail) fin shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Caudal {
    /// Forked (trout, minnows).
    Forked,
    /// Rounded (catfish, goldfish-like).
    Rounded,
    /// Crescent (tuna, sharks).
    Lunate,
    /// Horizontal flukes (cetaceans).
    Fluke,
    /// Long ribbon-like fin continuous with dorsal/anal fins (eels).
    Ribbon,
    /// Thin whip (rays).
    Whip,
}

/// Parameters of a fish, cetacean or ray (adult, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FishParams {
    /// Total length.
    pub length: f32,
    /// Maximum body depth.
    pub depth: f32,
    /// Maximum body width.
    pub width: f32,
    /// Where the body is deepest (0 = snout .. 1 = tail).
    pub depth_at: f32,
    /// Caudal fin.
    pub caudal: Caudal,
    /// Caudal fin span relative to body depth.
    pub caudal_size: f32,
    /// Dorsal fin height relative to depth (0 = none).
    pub dorsal: f32,
    /// Pectoral fin length relative to depth.
    pub pectoral: f32,
    /// Barbel length (catfish), 0 = none.
    pub barbels: f32,
    /// Flattened disc with flapping pectoral "wings" (rays).
    pub disc: bool,
    /// Body coat.
    pub coat: Coat,
    /// Fin colour.
    pub fin: u32,
    /// Rigid segments.
    pub segments: u8,
    /// 0 = carangiform, 1 = anguilliform.
    pub uniform: f32,
    /// Iris colour.
    pub iris: u32,
}

impl FishParams {
    /// Generic small fusiform fish.
    pub const BASE: FishParams = FishParams {
        length: 0.25,
        depth: 0.06,
        width: 0.035,
        depth_at: 0.35,
        caudal: Caudal::Forked,
        caudal_size: 1.1,
        dorsal: 0.45,
        pectoral: 0.45,
        barbels: 0.0,
        disc: false,
        coat: Coat::countershaded(0x5a6a50, 0xe8e8d8),
        fin: 0x8a7a60,
        segments: 4,
        uniform: 0.0,
        iris: 0x1a1a10,
    };
}

/// Body half-depth, half-width profile at arc fraction `u` from the snout.
fn fish_profile(p: &FishParams, u: f32) -> (f32, f32) {
    let rise = (u / p.depth_at).clamp(0.0, 1.0);
    let fall = ((1.0 - u) / (1.0 - p.depth_at)).clamp(0.0, 1.0);
    let shape =
        if u < p.depth_at { (rise * PI * 0.5).sin().powf(0.6) } else { fall.powf(0.9) * 0.88 + 0.12 * fall.powf(0.2) };
    let peduncle = 0.1 + 0.9 * shape;
    (0.5 * p.depth * peduncle, 0.5 * p.width * peduncle)
}

/// Builds a fish, cetacean or ray. The rig is centred on the swimming axis
/// (aquatic organisms are placed in the water column by the simulation).
pub fn build_fish(p: &FishParams, mass_kg: f32, seed: u32, detail: Detail) -> Rig {
    let segs = if detail.small_parts() { p.segments.max(2) } else { 2 };
    let l = p.length;
    let motion = RigMotion {
        locomotor: Locomotor::Fish,
        gallop: GallopStyle::Bound,
        mass_kg,
        hip_height: 0.5 * p.depth,
        body_length: l,
        rest_drop: 0.0,
        leg_lengths: Vec::new(),
        leg_reach: Vec::new(),
        half_width: 0.5 * p.width,
        half_span: if p.disc { p.pectoral * p.length } else { 0.0 },
        flap_hz: if p.disc { 0.6 } else { 0.0 },
        flap_amplitude: if p.disc { 0.45 } else { 0.0 },
        can_fly: false,
        aquatic: true,
        graze_pitch: 0.35,
        segments: segs,
        wave_length: if p.uniform > 0.5 { 0.65 } else { 1.0 },
        wave_amplitude: if p.disc { 0.1 } else { 0.35 + 0.25 * p.uniform },
        vertical_undulation: p.caudal == Caudal::Fluke,
    };
    let mut rig = Rig::new(Transform::IDENTITY, motion, srgb(p.coat.base));
    // Segment boundaries along the body (0 = snout, 1 = tail); the root is
    // the head segment, every other segment hinges at its front boundary.
    let head_frac = 0.32;
    let bounds: Vec<f32> = (0..=segs)
        .map(|i| if i == 0 { 0.0 } else { head_frac + (1.0 - head_frac) * (i - 1) as f32 / (segs - 1).max(1) as f32 })
        .collect();
    let model_z = |u: f32| (0.5 - u) * l;
    let around = detail.around(14);
    let ring_at = |u: f32, z: f32| -> Ring {
        let (hd, hw) = fish_profile(p, u);
        if p.disc {
            Ring::asym(Vec3::new(0.0, 0.0, z), (hw * 0.6).max(0.02 * l), hd * 0.55, hd * 0.35)
        } else {
            Ring::asym(Vec3::new(0.0, 0.0, z), hw.max(0.006 * l), hd, hd * 0.92)
        }
    };
    let fin_c = srgb(p.fin);
    let mut parent = 0usize;
    for k in 0..segs as usize {
        let (u0, u1) = (bounds[k], bounds[k + 1]);
        let origin = if k == 0 { 0.0 } else { model_z(u0) };
        let n = detail.rings(if k == 0 { 6 } else { 4 });
        // Rings from the back of the segment to its front (+Z), so the
        // loft's dorsal side follows +Y.
        let rings: Vec<Ring> = (0..=n)
            .rev()
            .map(|i| {
                let u = u0 + (u1 - u0) * i as f32 / n as f32;
                ring_at(u, model_z(u) - origin)
            })
            .collect();
        let first = k == 0;
        let last = k + 1 == segs as usize;
        let build = |m: &mut MeshData, model: Transform| {
            let c = coat_color(p.coat, model, seed);
            loft(m, &rings, around, Vec3::Y, if last { Cap::Point(0.02 * l) } else { Cap::Round }, Cap::Round, &c);
            if last {
                caudal_fin(m, p, rings[0].center.z, fin_c, detail);
            }
            let (dz0, dz1) = (0.32, 0.6);
            if p.dorsal > 0.0 && !p.disc && u0 < dz1 && u1 > dz0 {
                let a = model_z(u0.max(dz0)) - origin;
                let b = model_z(u1.min(dz1)) - origin;
                let top = fish_profile(p, (u0 + u1) * 0.5).0;
                let h = p.dorsal * p.depth * if p.caudal == Caudal::Fluke { 0.6 } else { 1.0 };
                let span = (a - b).abs();
                let outline = [
                    Vec2::new(0.0, b),
                    Vec2::new(0.0, a),
                    Vec2::new(0.6 * h, a - 0.15 * span),
                    Vec2::new(h, b - 0.25 * span),
                ];
                planform(
                    m,
                    &outline,
                    Vec2::new(0.3 * h, 0.5 * (a + b)),
                    2,
                    0.05 * h,
                    Vec3::new(0.0, 0.85 * top, 0.0),
                    Vec3::Y,
                    Vec3::Z,
                    &move |_, f| shade(fin_c, 1.0 - 0.3 * f),
                );
            }
        };
        if first {
            let mut mesh = MeshData::new();
            build(&mut mesh, rig.rest_model_transform(0));
            rig.parts[0].mesh = mesh;
        } else {
            let pos = Vec3::new(0.0, 0.0, model_z(u0) - if k == 1 { 0.0 } else { model_z(bounds[k - 1]) });
            parent = add_part(
                &mut rig,
                parent,
                PartRole::Segment(k as u8),
                Transform::from_translation(pos),
                PartMaterial::Matte,
                build,
            );
        }
    }
    // Head features on the root: eyes, barbels, cetacean beak.
    let (hd, hw) = fish_profile(p, 0.08);
    let eye_r = (0.085 * p.depth).min(0.03 * l) * if p.caudal == Caudal::Fluke { 0.5 } else { 1.0 };
    let eye_pos = Vec3::new((0.82 * hw).max(hw - 0.5 * eye_r), hd * 0.3, model_z(0.08));
    eye(&mut rig, 0, eye_pos, eye_r, p.iris, detail);
    eye(&mut rig, 0, eye_pos * Vec3::new(-1.0, 1.0, 1.0), eye_r, p.iris, detail);
    if p.barbels > 0.0 && detail.small_parts() {
        add_part(
            &mut rig,
            0,
            PartRole::Static,
            Transform::from_xyz(0.0, -0.2 * hd, model_z(0.02)),
            PartMaterial::Matte,
            |m, _| {
                for s in [-1.0f32, 1.0] {
                    let b = p.barbels;
                    tube(
                        m,
                        &[
                            Vec3::new(s * 0.5 * hw, 0.0, 0.0),
                            Vec3::new(s * (0.5 * hw + 0.4 * b), -0.2 * b, 0.2 * b),
                            Vec3::new(s * (0.5 * hw + 0.6 * b), -0.5 * b, -0.2 * b),
                        ],
                        &[0.01 * l, 0.006 * l, 0.003 * l],
                        4,
                        Vec3::Y,
                        (Cap::Round, Cap::Round),
                        &solid(0x2a2420),
                    );
                }
            },
        );
    }
    // Pectoral fins / ray wings / cetacean flippers.
    for side in Side::BOTH {
        let s = side.sign();
        let (hd2, hw2) = fish_profile(p, 0.3);
        if p.disc {
            let span = p.pectoral * p.length;
            let outline = [
                Vec2::new(0.0, 0.25 * l),
                Vec2::new(span, -0.05 * l),
                Vec2::new(0.25 * span, -0.25 * l),
                Vec2::new(0.0, -0.3 * l),
            ];
            let coat = p.coat;
            add_part(
                &mut rig,
                0,
                PartRole::Fin(side),
                Transform::from_xyz(s * 0.4 * hw2, 0.0, 0.0),
                PartMaterial::Matte,
                |m, model| {
                    let c = coat_color(coat, model, seed);
                    wing_panel(m, &outline, s, 0.06 * l, detail.rings(3), &move |q, f| {
                        let sp = SurfacePoint {
                            along: f,
                            around: 0.0,
                            dorsal: 1.0,
                            lateral: 0.0,
                            position: Vec3::new(q.x, 0.0, q.y),
                        };
                        c(&sp)
                    });
                },
            );
        } else if p.pectoral > 0.0 {
            let len = p.pectoral * p.depth;
            let outline = [
                Vec2::new(0.0, 0.0),
                Vec2::new(0.35 * len, 0.15 * len),
                Vec2::new(len, -0.35 * len),
                Vec2::new(0.7 * len, -0.75 * len),
                Vec2::new(0.0, -0.4 * len),
            ];
            // Pectoral fins sweep back along the flank, tilted slightly down.
            let rest = Quat::from_rotation_z(-s * 0.3) * Quat::from_rotation_y(s * 0.7);
            add_part(
                &mut rig,
                0,
                PartRole::Fin(side),
                Transform::from_xyz(s * 0.8 * hw2, -0.45 * hd2, model_z(0.33)).with_rotation(rest),
                PartMaterial::Matte,
                |m, _| {
                    wing_panel(m, &outline, s, 0.08 * len, 2, &move |_, f| shade(fin_c, 1.0 - 0.25 * f));
                },
            );
        }
    }
    rig
}

fn caudal_fin(m: &mut MeshData, p: &FishParams, z_end: f32, fin: [f32; 4], detail: Detail) {
    let h = 0.5 * p.depth * p.caudal_size;
    let len = 0.45 * h + 0.05 * p.length;
    let outline: Vec<Vec2> = match p.caudal {
        Caudal::Forked => vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(h, -len),
            Vec2::new(0.35 * h, -0.55 * len),
            Vec2::new(0.0, -0.45 * len),
            Vec2::new(-0.35 * h, -0.55 * len),
            Vec2::new(-h, -len),
        ],
        Caudal::Lunate | Caudal::Fluke => vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.6 * h, -0.4 * len),
            Vec2::new(1.2 * h, -0.95 * len),
            Vec2::new(0.5 * h, -0.6 * len),
            Vec2::new(0.0, -0.5 * len),
            Vec2::new(-0.5 * h, -0.6 * len),
            Vec2::new(-1.2 * h, -0.95 * len),
            Vec2::new(-0.6 * h, -0.4 * len),
        ],
        Caudal::Rounded => (0..=8)
            .map(|k| {
                let a = PI * (1.0 + k as f32 / 8.0);
                Vec2::new(0.8 * h * a.cos(), 0.9 * len * a.sin())
            })
            .collect(),
        Caudal::Ribbon => vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5 * h, -0.4 * len),
            Vec2::new(0.0, -1.3 * len),
            Vec2::new(-0.5 * h, -0.4 * len),
        ],
        Caudal::Whip => {
            tube(
                m,
                &[Vec3::new(0.0, 0.0, z_end), Vec3::new(0.0, 0.0, z_end - 0.6 * p.length)],
                &[0.01 * p.length, 0.002 * p.length],
                detail.around(5),
                Vec3::Y,
                (Cap::Round, Cap::Point(0.01)),
                &solid(p.coat.base),
            );
            return;
        }
    };
    // Vertical fin in the YZ plane, except horizontal cetacean flukes.
    let (u, v) = if p.caudal == Caudal::Fluke { (Vec3::X, Vec3::Z) } else { (Vec3::Y, Vec3::Z) };
    let pivot = Vec2::new(0.0, -0.3 * len);
    planform(
        m,
        &outline,
        pivot,
        detail.rings(3),
        0.04 * h,
        Vec3::new(0.0, 0.0, z_end + 0.02 * len),
        u,
        v,
        &move |_, f| mix(fin, shade(fin, 0.7), f),
    );
}

/// Parameters of a limbless serpentine body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SerpentParams {
    /// Total length.
    pub length: f32,
    /// Body radius.
    pub radius: f32,
    /// Head length.
    pub head_length: f32,
    /// Head width relative to the body radius.
    pub head_width: f32,
    /// Rigid segments.
    pub segments: u8,
    /// Coat (pattern along the body).
    pub coat: Coat,
    /// Earthworm (no eyes, clitellum band, peristalsis) instead of a snake.
    pub worm: bool,
    /// Clitellum / tongue colour.
    pub accent: u32,
    /// Iris colour.
    pub iris: u32,
}

/// Builds a snake or worm resting on the ground along −Z from the head.
pub fn build_serpent(p: &SerpentParams, mass_kg: f32, seed: u32, detail: Detail) -> Rig {
    let segs = if detail.small_parts() { p.segments.max(3) } else { 4 };
    let r = p.radius;
    let body_len = p.length - p.head_length;
    let seg_len = body_len / (segs - 1) as f32;
    let motion = RigMotion {
        locomotor: if p.worm { Locomotor::Peristaltic } else { Locomotor::Serpentine },
        gallop: GallopStyle::Bound,
        mass_kg,
        hip_height: r,
        body_length: p.length,
        rest_drop: 0.0,
        leg_lengths: Vec::new(),
        leg_reach: Vec::new(),
        half_width: r,
        half_span: 0.0,
        flap_hz: 0.0,
        flap_amplitude: 0.0,
        can_fly: false,
        aquatic: false,
        graze_pitch: 0.2,
        segments: segs,
        wave_length: if p.worm { 0.5 } else { 0.7 },
        wave_amplitude: if p.worm { 0.12 } else { 0.55 },
        vertical_undulation: false,
    };
    let mut rig = Rig::new(Transform::from_xyz(0.0, r, 0.5 * p.length - p.head_length), motion, srgb(p.coat.base));
    let around = detail.around(10);
    // Head on the root, extending forward.
    let hw = p.head_width * r;
    let mut head = MeshData::new();
    let head_color = coat_color(p.coat, rig.rest_model_transform(0), seed);
    if p.worm {
        tube(
            &mut head,
            &[Vec3::new(0.0, 0.0, -0.4 * seg_len), Vec3::new(0.0, 0.0, p.head_length)],
            &[r, 0.6 * r],
            around,
            Vec3::Y,
            (Cap::Round, Cap::Round),
            &head_color,
        );
    } else {
        let rings = [
            Ring::asym(Vec3::new(0.0, 0.0, -0.4 * seg_len), r, r, r),
            Ring::asym(Vec3::new(0.0, 0.05 * r, 0.3 * p.head_length), hw, 0.75 * hw, 0.6 * hw),
            Ring::asym(Vec3::new(0.0, 0.0, 0.75 * p.head_length), 0.8 * hw, 0.55 * hw, 0.5 * hw),
            Ring::asym(Vec3::new(0.0, -0.05 * r, p.head_length), 0.4 * hw, 0.3 * hw, 0.3 * hw),
        ];
        loft(&mut head, &rings, around, Vec3::Y, Cap::Round, Cap::Round, &head_color);
    }
    rig.parts[0].mesh = head;
    if !p.worm {
        let e = Vec3::new(0.62 * hw, 0.35 * hw, 0.62 * p.head_length);
        eye(&mut rig, 0, e, 0.2 * hw, p.iris, detail);
        eye(&mut rig, 0, e * Vec3::new(-1.0, 1.0, 1.0), 0.2 * hw, p.iris, detail);
        if detail.small_parts() {
            let accent = p.accent;
            add_part(
                &mut rig,
                0,
                PartRole::Jaw,
                Transform::from_xyz(0.0, -0.2 * hw, p.head_length),
                PartMaterial::Matte,
                |m, _| {
                    let t = 0.8 * p.head_length;
                    for s in [-1.0f32, 1.0] {
                        tube(
                            m,
                            &[Vec3::ZERO, Vec3::new(0.0, 0.0, 0.6 * t), Vec3::new(s * 0.12 * t, 0.0, t)],
                            &[0.06 * hw, 0.05 * hw, 0.02 * hw],
                            4,
                            Vec3::Y,
                            (Cap::Round, Cap::Point(0.03 * t)),
                            &solid(accent),
                        );
                    }
                },
            );
        }
    }
    let mut parent = 0;
    for k in 1..segs {
        let u = k as f32 / segs as f32;
        let taper = |t: f32| if p.worm { 1.0 - 0.35 * t * t } else { (1.0 - t.powf(2.2)).max(0.12) };
        let (r0, r1) = (r * taper(u - 1.0 / segs as f32), r * taper(u));
        let pos = if k == 1 { Vec3::new(0.0, 0.0, -0.0) } else { Vec3::new(0.0, 0.0, -seg_len) };
        let last = k + 1 == segs;
        let worm = p.worm;
        let accent = srgb(p.accent);
        let coat = p.coat;
        parent = add_part(
            &mut rig,
            parent,
            PartRole::Segment(k),
            Transform::from_translation(pos),
            PartMaterial::Matte,
            |m, model| {
                let base = coat_color(coat, model, seed);
                let colorize = move |sp: &SurfacePoint| {
                    if worm && k == 2 {
                        accent
                    } else if worm {
                        shade(base(sp), 0.9 + 0.1 * ((sp.position.z / r * 3.0).sin()))
                    } else {
                        base(sp)
                    }
                };
                tube(
                    m,
                    &[Vec3::ZERO, Vec3::new(0.0, 0.0, -seg_len)],
                    &[r0, r1],
                    around,
                    Vec3::Y,
                    (Cap::Round, if last { Cap::Point(r1 * 2.0) } else { Cap::Round }),
                    &colorize,
                );
            },
        );
    }
    rig
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fish_is_centred_and_segmented() {
        let rig = build_fish(&FishParams::BASE, 0.2, 1, Detail::Full);
        rig.validate().unwrap();
        let segs = rig.parts.iter().filter(|p| matches!(p.role, PartRole::Segment(_))).count();
        assert_eq!(segs + 1, FishParams::BASE.segments as usize);
        let (lo, hi) = rig.rest_bounds().unwrap();
        assert!((hi.z - lo.z) > 0.9 * FishParams::BASE.length, "{lo} {hi}");
        assert!(lo.y < 0.0 && hi.y > 0.0, "centred on the swimming axis");
        assert!(rig.motion.aquatic);
    }

    #[test]
    fn snake_lies_on_the_ground_and_tapers() {
        let p = SerpentParams {
            length: 1.2,
            radius: 0.025,
            head_length: 0.05,
            head_width: 1.3,
            segments: 12,
            coat: Coat::plain(0x506030),
            worm: false,
            accent: 0xc02020,
            iris: 0xc0a020,
        };
        let rig = build_serpent(&p, 0.5, 2, Detail::Full);
        rig.validate().unwrap();
        let (lo, hi) = rig.rest_bounds().unwrap();
        assert!(lo.y.abs() < 0.01, "{lo}");
        assert!((hi.z - lo.z - 1.2).abs() < 0.15, "{lo} {hi}");
    }
}
