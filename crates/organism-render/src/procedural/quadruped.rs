//! Quadruped body plan: mammals (upright legs) and lizards/crocodilians
//! (sprawling legs). Proportions are adult metres; silhouette features
//! (ears, horns, antlers, trunk, tusks, mane, hump, tail) are what makes a
//! species identifiable at a distance, so each is an explicit parameter.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::math::{Quat, Vec2, Vec3};
use bevy::prelude::Transform;

use super::mesh::{Cap, MeshData, Ring, SurfacePoint, loft, lump, superellipsoid, tube};
use super::palette::{Coat, Pattern, mix, shade, srgb};
use super::parts::{Detail, EarStyle, TailStyle, add_part, coat_color, solid};
use super::rig::{GallopStyle, LegSegment, Locomotor, PartMaterial, PartRole, Rig, RigMotion, Side};

/// Foot type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Foot {
    /// Hoof (ungulates).
    Hoof,
    /// Rounded paw (carnivores, rodents).
    Paw,
    /// Broad columnar pad (elephants, rhinos).
    Pad,
    /// Splayed clawed foot (lizards, crocodiles).
    Claw,
}

/// Head ornaments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Headgear {
    /// Nothing.
    None,
    /// Branched deer antlers.
    Antlers {
        /// Beam length (m).
        length: f32,
        /// Tines per beam.
        tines: u8,
    },
    /// Moose palmate antlers.
    Palmate {
        /// Half span (m).
        span: f32,
    },
    /// Bovid horns: out from the poll, curling by `curl` radians.
    Horns {
        /// Horn length (m).
        length: f32,
        /// Curl angle (rad); > 3 gives a ram's spiral.
        curl: f32,
        /// Outward spread 0..1.
        spread: f32,
    },
    /// Giraffe ossicones.
    Ossicones,
    /// Rhino nasal horns (front, rear lengths in m).
    NoseHorns {
        /// Front horn length.
        front: f32,
        /// Rear horn length.
        rear: f32,
    },
}

/// Mane style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mane {
    /// None.
    None,
    /// Lion ruff around head and neck (colour).
    Ruff(u32),
    /// Upright crest along the neck (horses, zebras, donkeys).
    Crest(u32),
}

/// Parameters of a quadruped (adult, metres and radians).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadrupedParams {
    /// Torso length.
    pub body_length: f32,
    /// Torso width.
    pub body_width: f32,
    /// Torso depth at the chest.
    pub body_depth: f32,
    /// Rump depth relative to the chest.
    pub rump: f32,
    /// Extra ventral depth at mid-body (fraction of half depth).
    pub belly: f32,
    /// Dorsal hump height (m).
    pub hump: f32,
    /// Hump position, 0 = rump .. 1 = withers.
    pub hump_at: f32,
    /// Hind hip height.
    pub leg_length: f32,
    /// Fore leg length relative to the hind leg.
    pub fore_ratio: f32,
    /// Leg radius at the top.
    pub leg_radius: f32,
    /// Foot radius relative to the top radius.
    pub foot_ratio: f32,
    /// Foot type.
    pub foot: Foot,
    /// Sprawling (lizard) instead of upright legs.
    pub sprawl: bool,
    /// Colour of the lower legs ("socks"), if different from the coat.
    pub socks: Option<u32>,
    /// Hoof / claw colour.
    pub hoof: u32,
    /// Neck length.
    pub neck_length: f32,
    /// Neck radius.
    pub neck_radius: f32,
    /// Neck elevation above horizontal (rad).
    pub neck_angle: f32,
    /// Bend of the upper neck (rad, positive = down); only for long necks.
    pub neck_bend: f32,
    /// Head length (back of skull to nose).
    pub head_length: f32,
    /// Head width.
    pub head_width: f32,
    /// Head depth.
    pub head_depth: f32,
    /// Fraction of the head that is snout.
    pub snout: f32,
    /// Snout width relative to the head width.
    pub snout_width: f32,
    /// Nose-down pitch of the head relative to horizontal (rad).
    pub head_pitch: f32,
    /// Eyes facing forward (1, predators) or sideways (0, prey).
    pub eyes_forward: f32,
    /// Ear style.
    pub ears: EarStyle,
    /// Ear length.
    pub ear_size: f32,
    /// Inner-ear colour.
    pub ear_inner: u32,
    /// Ear tip / back colour.
    pub ear_tip: Option<u32>,
    /// Horns or antlers.
    pub headgear: Headgear,
    /// Trunk length (0 = none).
    pub trunk: f32,
    /// Tusk length (0 = none).
    pub tusks: f32,
    /// Mane.
    pub mane: Mane,
    /// Tail style.
    pub tail: TailStyle,
    /// Tail length.
    pub tail_length: f32,
    /// Tail radius at its thickest.
    pub tail_radius: f32,
    /// Tail droop below horizontal (rad); negative curls it up.
    pub tail_droop: f32,
    /// Tail tip / tuft colour.
    pub tail_tip: Option<u32>,
    /// Body coat.
    pub coat: Coat,
    /// Muzzle / chin colour.
    pub muzzle: Option<u32>,
    /// Dark facial mask around the eyes (raccoon).
    pub mask: Option<u32>,
    /// Nose-pad colour.
    pub nose: u32,
    /// Iris colour.
    pub iris: u32,
    /// Dorsal scute ridge colour (crocodiles).
    pub ridge: Option<u32>,
    /// Head colour when different from the body (sheep faces).
    pub face: Option<u32>,
    /// Spiny dorsal coat colour (hedgehogs).
    pub spines: Option<u32>,
    /// Dark rings on the tail (raccoons, lemurs).
    pub tail_rings: Option<u32>,
    /// Fast gait.
    pub gallop: GallopStyle,
}

impl QuadrupedParams {
    /// Generic medium mammal; species override what differs.
    pub const BASE: QuadrupedParams = QuadrupedParams {
        body_length: 0.8,
        body_width: 0.3,
        body_depth: 0.35,
        rump: 0.9,
        belly: 0.05,
        hump: 0.0,
        hump_at: 0.8,
        leg_length: 0.5,
        fore_ratio: 1.0,
        leg_radius: 0.05,
        foot_ratio: 0.55,
        foot: Foot::Paw,
        sprawl: false,
        socks: None,
        hoof: 0x2a2420,
        neck_length: 0.25,
        neck_radius: 0.09,
        neck_angle: 0.6,
        neck_bend: 0.0,
        head_length: 0.25,
        head_width: 0.15,
        head_depth: 0.14,
        snout: 0.45,
        snout_width: 0.55,
        head_pitch: 0.2,
        eyes_forward: 0.3,
        ears: EarStyle::Pointed,
        ear_size: 0.08,
        ear_inner: 0xe8c8b8,
        ear_tip: None,
        headgear: Headgear::None,
        trunk: 0.0,
        tusks: 0.0,
        mane: Mane::None,
        tail: TailStyle::Thin,
        tail_length: 0.4,
        tail_radius: 0.03,
        tail_droop: 0.9,
        tail_tip: None,
        coat: Coat::plain(0x8a6a4a),
        muzzle: None,
        mask: None,
        nose: 0x1a1414,
        iris: 0x5a3a1a,
        ridge: None,
        face: None,
        spines: None,
        tail_rings: None,
        gallop: GallopStyle::Rotary,
    };
}

/// Inserts `k − 1` linearly interpolated sections between consecutive
/// key sections (denser lofts carry finer coat patterns).
fn subdivide(keys: &[Ring], k: usize) -> Vec<Ring> {
    let k = k.max(1);
    let mut out = Vec::with_capacity(keys.len() * k);
    for w in keys.windows(2) {
        for i in 0..k {
            let t = i as f32 / k as f32;
            let lerp = |a: f32, b: f32| a + (b - a) * t;
            out.push(Ring {
                center: w[0].center.lerp(w[1].center, t),
                half_width: lerp(w[0].half_width, w[1].half_width),
                top: lerp(w[0].top, w[1].top),
                bottom: lerp(w[0].bottom, w[1].bottom),
                exponent: lerp(w[0].exponent, w[1].exponent),
            });
        }
    }
    out.extend(keys.last().copied());
    out
}

/// Lengths of the two leg segments and their rest angles for an upright
/// leg whose joint sits at height `h`. Returns (upper, lower, a_upper,
/// a_lower) with angles as protraction (rad, + = foot forward).
fn upright_leg(h: f32, hind: bool) -> (f32, f32, f32, f32) {
    let (au, al): (f32, f32) = if hind { (0.22, -0.48) } else { (-0.1, 0.16) };
    let k = h / (0.5 * au.cos() + 0.5 * (au + al).cos());
    (0.5 * k, 0.5 * k, au, al)
}

/// Neck pitch (rad, downwards) that brings the mouth to the ground, found by
/// scanning the forward kinematics of the neck + head chain in the sagittal
/// plane. `base` is the neck base (z, y); `neck` and `head` are the rest
/// vectors (z, y) of the neck and of the head (to the mouth).
pub fn graze_pitch(base: Vec2, neck: Vec2, head: Vec2, ground_clearance: f32) -> f32 {
    let mut best = (f32::MAX, 0.0);
    let mut d = 0.0f32;
    while d <= 1.75 {
        let (s, c) = (-d).sin_cos();
        let rot = |v: Vec2| Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c);
        let tip = base + rot(neck) + rot(head);
        let err = (tip.y - ground_clearance).abs();
        if err < best.0 {
            best = (err, d);
        }
        if tip.y <= ground_clearance {
            return d;
        }
        d += 0.01;
    }
    best.1
}

/// Builds a quadruped rig.
pub fn build(p: &QuadrupedParams, mass_kg: f32, seed: u32, detail: Detail) -> Rig {
    let coat = p.coat;
    let marker = srgb(coat.base);
    let (lb, w, d) = (p.body_length, p.body_width, p.body_depth);
    let hind_h;
    let fore_h;
    let hind_leg;
    let fore_leg;
    if p.sprawl {
        // Upper leg nearly horizontal, lower leg vertical: joint at lower length.
        let upper = p.leg_length * 0.55;
        let lower = p.leg_length;
        hind_h = lower * 0.9 + upper * 0.25f32.sin();
        fore_h = hind_h * p.fore_ratio;
        hind_leg = (upper, lower, 0.0, 0.0);
        fore_leg = (upper * p.fore_ratio, lower * p.fore_ratio, 0.0, 0.0);
    } else {
        hind_h = p.leg_length;
        fore_h = p.leg_length * p.fore_ratio;
        hind_leg = upright_leg(hind_h, true);
        fore_leg = upright_leg(fore_h, false);
    }
    let lift = if p.sprawl { 0.05 * d } else { 0.22 * d };
    let root_y = 0.5 * (hind_h + fore_h) + lift;
    let rear_y = hind_h + lift - root_y;
    let front_y = fore_h + lift - root_y;
    let rest_drop = (hind_h.min(fore_h) + lift - 0.5 * d * p.rump * 0.95).max(0.0);
    let motion = RigMotion {
        locomotor: Locomotor::Quadruped,
        gallop: p.gallop,
        mass_kg,
        hip_height: hind_h,
        body_length: lb + p.neck_length + p.head_length,
        rest_drop,
        leg_lengths: vec![(fore_leg.0, fore_leg.1), (hind_leg.0, hind_leg.1)],
        leg_reach: Vec::new(),
        half_width: 0.5 * w,
        half_span: 0.0,
        flap_hz: 0.0,
        flap_amplitude: 0.0,
        can_fly: false,
        aquatic: false,
        graze_pitch: 0.0,
        segments: 0,
        wave_length: 1.0,
        wave_amplitude: if p.sprawl { 0.25 } else { 0.0 },
        vertical_undulation: false,
    };
    let mut rig = Rig::new(Transform::from_xyz(0.0, root_y, 0.0), motion, marker);

    // --- torso ---------------------------------------------------------------
    // Vertex colours carry the coat pattern: patterned coats need a denser
    // torso so stripes, spots and patch networks are resolved.
    let patterned = !matches!(coat.pattern, Pattern::Plain | Pattern::Saddle { .. });
    let n = detail.rings(if patterned { 26 } else { 12 });
    let around = detail.around(if patterned { 30 } else { 16 });
    // Rounded caps add about one end radius to each end: inset the end
    // rings so the torso keeps its nominal length.
    let inset_rear = 0.8 * 0.25 * (2.0 * 0.5 * w * 0.72 + 0.5 * d * p.rump * 1.65);
    let inset_front = 0.8 * 0.25 * (2.0 * 0.5 * w * 0.72 + 0.5 * d * 1.85);
    let rings: Vec<Ring> = (0..=n)
        .map(|i| {
            let u = i as f32 / n as f32;
            let z = -0.5 * lb + inset_rear + u * (lb - inset_rear - inset_front);
            let y = rear_y + (front_y - rear_y) * u;
            let swell = (PI * u.powf(0.85)).sin();
            let half_w = 0.5 * w * (0.72 + 0.28 * swell);
            let depth = 0.5 * d * (p.rump + (1.0 - p.rump) * u);
            let hump = p.hump * (-((u - p.hump_at) / 0.2).powi(2)).exp();
            let top = depth * (0.85 + 0.15 * swell) + hump;
            let bottom = depth * (0.8 + 0.2 * u) + 0.5 * d * p.belly * swell * swell;
            Ring { center: Vec3::new(0.0, y, z), half_width: half_w, top, bottom, exponent: 2.3 }
        })
        .collect();
    let model0 = rig.rest_model_transform(0);
    let body_color = coat_color(coat, model0, seed);
    let mut body = MeshData::new();
    loft(&mut body, &rings, around, Vec3::Y, Cap::Round, Cap::Round, &body_color);
    if let Some(ridge) = p.ridge
        && detail.small_parts()
    {
        // Paired dorsal scute rows.
        for i in 0..14 {
            let u = 0.08 + 0.84 * i as f32 / 13.0;
            let r = &rings[((u * n as f32) as usize).min(n)];
            for side in [-1.0, 1.0] {
                let c = Vec3::new(side * r.half_width * 0.35, r.center.y + r.top * 0.92, r.center.z);
                superellipsoid(&mut body, c, Vec3::new(0.02, 0.025, 0.035) * (d / 0.3), 1.0, 1.0, 4, 5, &solid(ridge));
            }
        }
    }
    rig.parts[0].mesh = body;
    if let Some(spines) = p.spines {
        let c = srgb(spines);
        let mid = &rings[n / 2];
        add_part(&mut rig, 0, PartRole::Static, Transform::IDENTITY, PartMaterial::Matte, |m, _| {
            let level = if detail.small_parts() { 3 } else { 1 };
            lump(
                m,
                Vec3::new(0.0, mid.center.y + 0.25 * mid.top, -0.05 * lb),
                Vec3::new(1.08 * mid.half_width, 1.0 * mid.top, 0.62 * lb),
                level,
                0.18,
                22.0,
                seed ^ 0x5b,
                &move |dir, k| {
                    let tip = ((k - 1.0) * 6.0).clamp(0.0, 1.0);
                    shade(mix(c, srgb(0xe8dcc8), tip), 0.75 + 0.25 * dir.y.max(0.0))
                },
            );
        });
    }

    // --- legs ----------------------------------------------------------------
    let leg_top = |pair: u8| -> Vec3 {
        let (z, y) = if pair == 0 {
            (0.5 * lb - 1.4 * p.leg_radius, front_y - lift)
        } else {
            (-0.5 * lb + 1.6 * p.leg_radius, rear_y - lift)
        };
        Vec3::new(0.5 * w - 0.9 * p.leg_radius, y, z)
    };
    for pair in 0..2u8 {
        let (upper, lower, au, al) = if pair == 0 { fore_leg } else { hind_leg };
        let hind = pair == 1;
        for side in Side::BOTH {
            let s = side.sign();
            let top = leg_top(pair) * Vec3::new(s, 1.0, 1.0);
            let (rest_u, rest_l) = if p.sprawl {
                let roll = s * (FRAC_PI_2 - 0.25);
                (Quat::from_rotation_z(roll), Quat::from_rotation_z(-roll))
            } else {
                (Quat::from_rotation_x(-au), Quat::from_rotation_x(-al))
            };
            let r0 = p.leg_radius * if hind { 1.25 } else { 1.0 };
            let r_knee = p.leg_radius * 0.72;
            let r_foot = p.leg_radius * p.foot_ratio;
            let socks = p.socks;
            let upper_idx = add_part(
                &mut rig,
                0,
                PartRole::Leg { pair, side, segment: LegSegment::Upper },
                Transform::from_translation(top).with_rotation(rest_u),
                PartMaterial::Matte,
                |m, model| {
                    let c = coat_color(coat, model, seed);
                    let pts = [
                        Vec3::new(0.0, 0.25 * r0, 0.0),
                        Vec3::new(0.0, -0.45 * upper, if hind { -0.15 * r0 } else { 0.1 * r0 }),
                        Vec3::new(0.0, -upper, 0.0),
                    ];
                    tube(
                        m,
                        &pts,
                        &[r0 * 1.15, r0 * 0.95, r_knee],
                        detail.around(10),
                        Vec3::Z,
                        (Cap::Round, Cap::Round),
                        &c,
                    );
                },
            );
            add_part(
                &mut rig,
                upper_idx,
                PartRole::Leg { pair, side, segment: LegSegment::Lower },
                Transform::from_xyz(0.0, -upper, 0.0).with_rotation(rest_l),
                PartMaterial::Matte,
                |m, model| {
                    let base = coat_color(coat, model, seed);
                    let colorize = move |sp: &SurfacePoint| {
                        let c = base(sp);
                        match socks {
                            Some(sock) => {
                                let f = ((sp.position.y + 0.75 * lower) / (0.45 * lower)).clamp(0.0, 1.0);
                                mix(srgb(sock), c, f * f * (3.0 - 2.0 * f))
                            }
                            None => c,
                        }
                    };
                    let pts = [Vec3::ZERO, Vec3::new(0.0, -0.55 * lower, 0.0), Vec3::new(0.0, -lower + r_foot, 0.0)];
                    tube(
                        m,
                        &pts,
                        &[r_knee, r_knee * 0.85, r_foot],
                        detail.around(9),
                        Vec3::Z,
                        (Cap::Round, Cap::Open),
                        &colorize,
                    );
                    let hoof = srgb(p.hoof);
                    match p.foot {
                        Foot::Hoof => tube(
                            m,
                            &[Vec3::new(0.0, -lower + r_foot * 1.4, 0.0), Vec3::new(0.0, -lower, 0.12 * r_foot)],
                            &[r_foot * 1.02, r_foot * 1.18],
                            detail.around(9),
                            Vec3::Z,
                            (Cap::Flat, Cap::Flat),
                            &move |_| hoof,
                        ),
                        Foot::Paw => superellipsoid(
                            m,
                            Vec3::new(0.0, -lower + 0.55 * r_foot, 0.5 * r_foot),
                            Vec3::new(r_foot * 1.15, r_foot * 0.7, r_foot * 1.45),
                            0.8,
                            1.0,
                            6,
                            detail.around(9),
                            &|sp| colorize(sp),
                        ),
                        Foot::Pad => tube(
                            m,
                            &[Vec3::new(0.0, -lower + r_foot, 0.0), Vec3::new(0.0, -lower, 0.0)],
                            &[r_foot * 1.02, r_foot * 1.12],
                            detail.around(12),
                            Vec3::Z,
                            (Cap::Flat, Cap::Flat),
                            &move |_| hoof,
                        ),
                        Foot::Claw => {
                            let toes = if detail.small_parts() { 4 } else { 2 };
                            for t in 0..toes {
                                let a = (t as f32 / (toes - 1).max(1) as f32 - 0.5) * 1.6;
                                let dir = Vec3::new(a.sin() * s, 0.0, a.cos());
                                tube(
                                    m,
                                    &[
                                        Vec3::new(0.0, -lower + r_foot * 0.5, 0.0),
                                        Vec3::new(0.0, -lower + r_foot * 0.3, 0.0) + dir * r_foot * 3.2,
                                    ],
                                    &[r_foot * 0.45, r_foot * 0.18],
                                    5,
                                    Vec3::Y,
                                    (Cap::Round, Cap::Point(r_foot * 0.3)),
                                    &|sp| shade(colorize(sp), 0.85),
                                );
                            }
                        }
                    }
                },
            );
        }
    }

    // --- neck & head -----------------------------------------------------------
    let neck_base = Vec3::new(0.0, front_y + 0.25 * d, 0.5 * lb - 0.8 * inset_front);
    let two_segment = p.neck_length > 1.4 * p.head_length;
    let seg_len = if two_segment { 0.5 * p.neck_length } else { p.neck_length };
    let neck0 = add_part(
        &mut rig,
        0,
        PartRole::Neck(0),
        Transform::from_translation(neck_base).with_rotation(Quat::from_rotation_x(-p.neck_angle)),
        PartMaterial::Matte,
        |m, model| {
            let c = coat_color(coat, model, seed);
            let r = p.neck_radius;
            let keys = [
                Ring::asym(Vec3::new(0.0, -0.2 * r, -0.6 * r), 1.25 * r, 1.0 * r, 1.6 * r),
                Ring::asym(Vec3::new(0.0, 0.0, 0.4 * seg_len), 0.95 * r, 1.0 * r, 1.15 * r),
                Ring::asym(Vec3::new(0.0, 0.0, seg_len), 0.85 * r, 0.95 * r, 1.0 * r),
            ];
            let rings = subdivide(&keys, if patterned { 6 } else { 1 });
            loft(m, &rings, detail.around(if patterned { 18 } else { 12 }), Vec3::Y, Cap::Round, Cap::Round, &c);
            if let Mane::Crest(color) = p.mane {
                super::quadruped_head::crest(m, seg_len, r, color, detail);
            }
        },
    );
    let mut head_parent = neck0;
    let mut neck_end = Vec3::new(0.0, 0.0, seg_len);
    if two_segment {
        head_parent = add_part(
            &mut rig,
            neck0,
            PartRole::Neck(1),
            Transform::from_translation(neck_end).with_rotation(Quat::from_rotation_x(p.neck_bend)),
            PartMaterial::Matte,
            |m, model| {
                let c = coat_color(coat, model, seed);
                let r = p.neck_radius * 0.85;
                let keys = [
                    Ring::circle(Vec3::new(0.0, 0.0, -0.3 * r), r * 1.05),
                    Ring::circle(Vec3::new(0.0, 0.0, seg_len), r * 0.85),
                ];
                let rings = subdivide(&keys, if patterned { 10 } else { 1 });
                loft(m, &rings, detail.around(if patterned { 18 } else { 12 }), Vec3::Y, Cap::Round, Cap::Round, &c);
                if let Mane::Crest(color) = p.mane {
                    super::quadruped_head::crest(m, seg_len, r, color, detail);
                }
            },
        );
        neck_end = Vec3::new(0.0, 0.0, seg_len);
    }
    if let Mane::Ruff(color) = p.mane {
        let (hw, hd) = (p.head_width, p.head_depth);
        add_part(&mut rig, neck0, PartRole::Static, Transform::IDENTITY, PartMaterial::Matte, |m, _| {
            let c = srgb(color);
            let level = if detail.small_parts() { 2 } else { 1 };
            lump(
                m,
                Vec3::new(0.0, 0.15 * hd, 0.72 * seg_len),
                Vec3::new(1.25 * hw, 1.45 * hd, 0.85 * seg_len.max(hd)),
                level,
                0.22,
                3.5,
                seed ^ 0x3a,
                &move |dir, k| shade(c, 0.75 + 0.35 * k + 0.1 * dir.y),
            );
            lump(
                m,
                Vec3::new(0.0, -0.8 * hd, 0.25 * seg_len),
                Vec3::new(0.9 * hw, 1.1 * hd, 0.8 * seg_len.max(hd)),
                level,
                0.25,
                3.0,
                seed ^ 0x3b,
                &move |_, k| shade(c, 0.7 + 0.3 * k),
            );
        });
    }
    let head_rest = Quat::from_rotation_x(p.neck_angle + p.head_pitch - if two_segment { p.neck_bend } else { 0.0 });
    let head = super::quadruped_head::build_head(
        &mut rig,
        head_parent,
        Transform::from_translation(neck_end).with_rotation(head_rest),
        p,
        seed,
        detail,
    );

    // Graze pitch from the rest kinematics.
    let base_model = rig.rest_model_transform(neck0).translation;
    let head_model = rig.rest_model_transform(head);
    let mouth = head_model.transform_point(Vec3::new(0.0, -0.3 * p.head_depth, 0.8 * p.head_length + p.trunk * 0.3));
    let neck_vec = head_model.translation - base_model;
    let head_vec = mouth - head_model.translation;
    rig.motion.graze_pitch = graze_pitch(
        Vec2::new(base_model.z, base_model.y),
        Vec2::new(neck_vec.z, neck_vec.y),
        Vec2::new(head_vec.z, head_vec.y),
        0.03 * hind_h,
    );

    // --- tail ------------------------------------------------------------------
    if p.tail != TailStyle::None && p.tail_length > 0.0 {
        let start = Vec3::new(0.0, rear_y + 0.5 * d * p.rump * 0.55, -0.5 * lb + 0.05 * lb);
        let clearance = root_y + start.y - 0.04 * hind_h - p.tail_radius;
        super::quadruped_head::build_tail(&mut rig, p, start, clearance, seed, detail);
    }
    rig
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feet_touch_the_ground_in_rest_pose() {
        let rig = build(&QuadrupedParams::BASE, 20.0, 1, Detail::Full);
        rig.validate().unwrap();
        let (lo, hi) = rig.rest_bounds().unwrap();
        assert!(lo.y.abs() < 0.02, "lowest point {lo}");
        assert!(hi.y > QuadrupedParams::BASE.leg_length);
        assert!(hi.z > 0.4 && lo.z < -0.4, "extends forward (head) and back (tail)");
    }

    #[test]
    fn grazing_reaches_the_ground() {
        let rig = build(&QuadrupedParams::BASE, 20.0, 1, Detail::Full);
        assert!(rig.motion.graze_pitch > 0.3 && rig.motion.graze_pitch < 1.75, "{}", rig.motion.graze_pitch);
        // Direct check: rotating a neck of length 1 from 1 m height reaches the ground at 90°.
        let pitch = graze_pitch(Vec2::new(0.0, 1.0), Vec2::new(1.0, 0.0), Vec2::ZERO, 0.0);
        assert!((pitch - FRAC_PI_2).abs() < 0.02, "{pitch}");
    }

    #[test]
    fn low_detail_is_much_cheaper() {
        let full = build(&QuadrupedParams::BASE, 20.0, 1, Detail::Full);
        let low = build(&QuadrupedParams::BASE, 20.0, 1, Detail::Low);
        assert!(
            low.triangle_count() * 3 < full.triangle_count(),
            "{} vs {}",
            low.triangle_count(),
            full.triangle_count()
        );
        assert!(full.find(PartRole::Jaw).is_some() && low.find(PartRole::Jaw).is_none());
    }

    #[test]
    fn sprawling_legs_keep_body_low() {
        let lizard = QuadrupedParams {
            sprawl: true,
            body_length: 0.1,
            body_width: 0.03,
            body_depth: 0.022,
            leg_length: 0.022,
            leg_radius: 0.005,
            neck_length: 0.008,
            neck_radius: 0.012,
            head_length: 0.032,
            head_width: 0.022,
            head_depth: 0.014,
            ears: EarStyle::None,
            tail: TailStyle::Heavy,
            tail_length: 0.2,
            tail_radius: 0.012,
            tail_droop: 0.1,
            ..QuadrupedParams::BASE
        };
        let mut rig = build(&lizard, 0.03, 2, Detail::Full);
        rig.ground();
        // The belly hangs close to the ground: less than a body depth above it.
        let root_y = rig.parts[0].rest.translation.y;
        assert!(root_y < 0.03, "{root_y}");
    }
}
