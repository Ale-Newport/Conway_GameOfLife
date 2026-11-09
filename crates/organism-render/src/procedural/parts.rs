//! Helpers shared by the body-plan builders: level of detail, colour
//! closures in rig space, and small recurring parts (eyes, noses, ears,
//! horns, tails).

use bevy::math::{Quat, Vec2, Vec3};
use bevy::prelude::Transform;

use super::mesh::{Cap, MeshData, Rgba, Ring, SurfacePoint, loft, lump, planform, superellipsoid, tube};
use super::palette::{Coat, mix, shade, srgb};
use super::rig::{PartMaterial, PartRole, Rig};

/// Geometric detail of a generated organism.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Detail {
    /// Near view: smooth sections, eyes, jaws and small appendages.
    Full,
    /// Far view: few segments, no small parts; merged into one static mesh.
    Low,
}

impl Detail {
    /// Vertices around a section for a part that uses `full` at full detail.
    pub fn around(self, full: usize) -> usize {
        match self {
            Self::Full => full,
            Self::Low => (full / 3).max(5),
        }
    }

    /// Number of sections along a part that uses `full` at full detail.
    pub fn rings(self, full: usize) -> usize {
        match self {
            Self::Full => full,
            Self::Low => (full / 3).max(2),
        }
    }

    /// Whether small parts (eyes, noses, jaws, claws, antenna tips) are built.
    pub fn small_parts(self) -> bool {
        self == Self::Full
    }
}

/// Colours a surface with a coat evaluated at rig-space positions, so
/// patterns continue across part boundaries.
pub fn coat_color(coat: Coat, model: Transform, seed: u32) -> impl Fn(&SurfacePoint) -> Rgba {
    move |sp| {
        let p = model.transform_point(sp.position);
        coat.color_at(&SurfacePoint { position: p, ..*sp }, 1.0, seed)
    }
}

/// Uniform colour closure.
pub fn solid(hex: u32) -> impl Fn(&SurfacePoint) -> Rgba {
    let c = srgb(hex);
    move |_| c
}

/// Rotation whose local +Y maps to `y` and local +Z to (the part of) `z`
/// orthogonal to it.
pub fn basis_rotation(y: Vec3, z: Vec3) -> Quat {
    let y = y.normalize_or(Vec3::Y);
    let z = (z - y * z.dot(y)).normalize_or(y.any_orthonormal_vector());
    let x = y.cross(z);
    Quat::from_mat3(&bevy::math::Mat3::from_cols(x, y, z))
}

/// Adds a part whose geometry is generated in its own local frame.
pub fn add_part(
    rig: &mut Rig,
    parent: usize,
    role: PartRole,
    rest: Transform,
    material: PartMaterial,
    build: impl FnOnce(&mut MeshData, Transform),
) -> usize {
    let model = rig.rest_model_transform(parent) * rest;
    let mut mesh = MeshData::new();
    build(&mut mesh, model);
    rig.add(parent, role, rest, mesh, material)
}

/// Adds a glossy eye of radius `r` at `pos` in the parent's frame (full
/// detail only). A pale rim makes it readable on dark faces.
pub fn eye(rig: &mut Rig, parent: usize, pos: Vec3, r: f32, iris: u32, detail: Detail) {
    if !detail.small_parts() {
        return;
    }
    add_part(rig, parent, PartRole::Static, Transform::from_translation(pos), PartMaterial::Eye, |m, _| {
        let iris_c = srgb(iris);
        // The eye looks sideways and slightly forward: dark pupil there,
        // coloured iris ring around it, dark sclera behind.
        let gaze = Vec3::new(0.8 * pos.x.signum(), 0.05, 0.6).normalize();
        superellipsoid(m, Vec3::ZERO, Vec3::splat(r), 1.0, 1.0, 6, 8, &move |sp| {
            let d = sp.position.normalize_or_zero().dot(gaze);
            if d > 0.8 {
                srgb(0x050505)
            } else if d > 0.45 {
                iris_c
            } else {
                srgb(0x1a1612)
            }
        });
    });
}

/// Ear shapes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EarStyle {
    /// No external ears.
    None,
    /// Upright triangle (canids, felids, deer).
    Pointed,
    /// Small semicircle (bears, rodents).
    Round,
    /// Long upright ellipse (rabbits, donkeys).
    Long,
    /// Hanging flap (pigs, some cattle).
    Drooping,
    /// Huge fan held against the head (elephants).
    Fan,
    /// Pointed with a black tuft (lynx, caracal).
    Tufted,
}

/// Builds an ear in its local frame (+Y along the ear, +Z its front face).
/// `size` is the ear length; `inner` colours the concave side.
pub fn ear_mesh(
    m: &mut MeshData,
    style: EarStyle,
    size: f32,
    outer: u32,
    inner: u32,
    tip: Option<u32>,
    detail: Detail,
) {
    let outer_c = srgb(outer);
    let inner_c = srgb(inner);
    let tip_c = tip.map(srgb);
    let (width, thickness, profile): (f32, f32, fn(f32) -> f32) = match style {
        EarStyle::None => return,
        EarStyle::Pointed | EarStyle::Tufted => {
            (0.55, 0.12, |u| (1.0 - u).powf(0.9) * (0.35 + 0.65 * (u * 3.0).min(1.0)))
        }
        EarStyle::Round => {
            (0.9, 0.18, |u| (1.0 - (2.0 * u - 1.0).powi(2)).max(0.0).sqrt().max(if u < 0.5 { 0.6 } else { 0.0 }))
        }
        EarStyle::Long => {
            (0.32, 0.12, |u| (1.0 - (2.0 * u - 1.0).powi(2)).max(0.0).sqrt().max(if u < 0.3 { 0.7 } else { 0.0 }))
        }
        EarStyle::Drooping => {
            (0.6, 0.1, |u| (1.0 - (2.0 * u - 1.0).powi(2)).max(0.0).sqrt().max(if u < 0.3 { 0.7 } else { 0.0 }))
        }
        EarStyle::Fan => (0.85, 0.06, |u| (1.0 - (2.0 * u - 1.0).powi(2)).max(0.0).powf(0.35)),
    };
    let n = detail.rings(8);
    let rings: Vec<Ring> = (0..=n)
        .map(|i| {
            let u = i as f32 / n as f32;
            let w = (size * width * 0.5 * profile(u)).max(size * 0.02);
            // Slight cup: the section is pushed back in the middle.
            Ring {
                center: Vec3::new(0.0, u * size, -0.05 * size * (1.0 - u)),
                half_width: w,
                top: size * thickness * 0.5,
                bottom: size * thickness * 0.5,
                exponent: 2.0,
            }
        })
        .collect();
    loft(m, &rings, detail.around(8), Vec3::Z, Cap::Round, Cap::Point(size * 0.04), &move |sp| {
        let u = sp.position.y / size;
        let base = if sp.dorsal > 0.3 && u < 0.85 { mix(outer_c, inner_c, 0.85) } else { outer_c };
        match tip_c {
            Some(t) if u > 0.72 => t,
            _ => base,
        }
    });
    if style == EarStyle::Tufted && detail.small_parts() {
        let tuft = tip.unwrap_or(0x111111);
        tube(
            m,
            &[Vec3::new(0.0, size * 0.95, 0.0), Vec3::new(0.0, size * 1.35, -0.02 * size)],
            &[size * 0.05, size * 0.01],
            5,
            Vec3::Z,
            (Cap::Round, Cap::Point(size * 0.02)),
            &solid(tuft),
        );
    }
}

/// Curved, tapered horn along a planar arc in the local YZ/XY plane:
/// starts at the origin going up (+Y), bending by `curl` radians towards
/// `bend` (a unit vector). Used for bovid horns, tusks, rhino horns,
/// antler tines.
#[allow(clippy::too_many_arguments)]
pub fn horn(
    m: &mut MeshData,
    start: Vec3,
    dir: Vec3,
    bend: Vec3,
    length: f32,
    base_radius: f32,
    curl: f32,
    color: u32,
    tip: u32,
    detail: Detail,
) {
    let n = detail.rings(7);
    let dir = dir.normalize_or(Vec3::Y);
    let axis = dir.cross(bend).normalize_or(Vec3::X);
    let mut pts = Vec::with_capacity(n + 1);
    let mut radii = Vec::with_capacity(n + 1);
    let mut p = start;
    let step = length / n as f32;
    for i in 0..=n {
        let u = i as f32 / n as f32;
        pts.push(p);
        radii.push(base_radius * (1.0 - 0.85 * u));
        let d = Quat::from_axis_angle(axis, curl * u) * dir;
        p += d * step;
    }
    let (c0, c1) = (srgb(color), srgb(tip));
    tube(m, &pts, &radii, detail.around(8), bend, (Cap::Flat, Cap::Point(base_radius * 0.3)), &move |sp| {
        mix(c0, c1, sp.along.powf(1.5))
    });
}

/// Tail styles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TailStyle {
    /// No visible tail.
    None,
    /// Thin whip (cats, rats, cattle without tuft).
    Thin,
    /// Thick, furry tail (foxes, wolves, squirrels).
    Bushy,
    /// Thin with a dark terminal tuft (lion, cattle, elephant).
    Tufted,
    /// Short stub (bears, deer, rabbits).
    Stub,
    /// Broad flat paddle (beaver).
    Paddle,
    /// Thick at the base and tapering (lizards, crocodiles, kangaroos).
    Heavy,
    /// Long flowing hair (horses, zebras use Tufted).
    Flowing,
}

/// Builds one tail segment in its local frame, extending along −Z by
/// `len`, with radius tapering from `r0` to `r1`. On the last segment of a
/// tufted or flowing tail, a hair tuft of length `tuft_len` and colour
/// `tuft` is added.
#[allow(clippy::too_many_arguments)]
pub fn tail_segment(
    m: &mut MeshData,
    style: TailStyle,
    len: f32,
    r0: f32,
    r1: f32,
    color: &dyn Fn(&SurfacePoint) -> Rgba,
    last: bool,
    tuft: (u32, f32),
    detail: Detail,
) {
    let around = detail.around(9);
    match style {
        TailStyle::None => {}
        TailStyle::Paddle if last => {
            let rings: Vec<Ring> = (0..=4)
                .map(|i| {
                    let u = i as f32 / 4.0;
                    let w = r0 * (1.2 + 2.2 * (1.0 - (2.0 * u - 1.0).powi(2)).sqrt());
                    Ring::ellipse(Vec3::new(0.0, 0.0, -u * len), w, r0 * 0.35)
                })
                .collect();
            loft(m, &rings, around, Vec3::Y, Cap::Round, Cap::Round, &|sp| shade(color(sp), 0.6));
        }
        _ => {
            tube(
                m,
                &[Vec3::ZERO, Vec3::new(0.0, 0.0, -len)],
                &[r0, r1],
                around,
                Vec3::Y,
                (Cap::Round, Cap::Round),
                color,
            );
            if last && matches!(style, TailStyle::Tufted | TailStyle::Flowing) {
                let tc = srgb(tuft.0);
                let width = if style == TailStyle::Flowing { 2.2 * r0 } else { 2.6 * r0.max(r1) };
                let size = Vec3::new(width, width, 0.5 * tuft.1);
                lump(m, Vec3::new(0.0, 0.0, -len - 0.8 * size.z), size, 1, 0.25, 3.0, 11, &move |_, k| {
                    shade(tc, 0.8 + 0.3 * k)
                });
            }
        }
    }
}

/// Feather-tipped or membrane wing outline (half wing, spanning +X from the
/// shoulder, chord along −Z). Returns (inner, outer) outlines in planform
/// coordinates (x = span, y = −chord direction), split at `split` of the span.
pub fn wing_outlines(
    span: f32,
    chord: f32,
    slotted: bool,
    pointed: bool,
    membrane: bool,
    split: f32,
) -> (Vec<Vec2>, Vec<Vec2>) {
    let s = span * split;
    // Arm wing: rounded leading edge, slightly concave trailing edge.
    let inner = vec![
        Vec2::new(0.0, 0.22 * chord),
        Vec2::new(0.5 * s, 0.3 * chord),
        Vec2::new(s, 0.22 * chord),
        Vec2::new(s, -0.82 * chord),
        Vec2::new(0.5 * s, -0.72 * chord),
        Vec2::new(0.0, -0.8 * chord),
    ];
    let o = span - s;
    let outer = if membrane {
        vec![
            Vec2::new(0.0, 0.2 * chord),
            Vec2::new(0.55 * o, 0.25 * chord),
            Vec2::new(o, 0.05 * chord),
            Vec2::new(0.8 * o, -0.25 * chord),
            Vec2::new(0.62 * o, -0.15 * chord),
            Vec2::new(0.48 * o, -0.62 * chord),
            Vec2::new(0.3 * o, -0.5 * chord),
            Vec2::new(0.12 * o, -0.95 * chord),
            Vec2::new(0.0, -0.85 * chord),
        ]
    } else if slotted {
        let mut v = vec![Vec2::new(0.0, 0.2 * chord), Vec2::new(0.6 * o, 0.18 * chord)];
        for k in 0..5 {
            let t = k as f32 / 4.0;
            let x = o * (0.98 - 0.12 * t);
            let y = chord * (0.1 - 0.85 * t);
            v.push(Vec2::new(x, y));
            v.push(Vec2::new(x - 0.12 * o, y - 0.1 * chord));
        }
        v.push(Vec2::new(0.0, -0.85 * chord));
        v
    } else if pointed {
        vec![
            Vec2::new(0.0, 0.2 * chord),
            Vec2::new(0.7 * o, 0.05 * chord),
            Vec2::new(o, -0.15 * chord),
            Vec2::new(0.55 * o, -0.6 * chord),
            Vec2::new(0.0, -0.85 * chord),
        ]
    } else {
        vec![
            Vec2::new(0.0, 0.2 * chord),
            Vec2::new(0.75 * o, 0.12 * chord),
            Vec2::new(o, -0.25 * chord),
            Vec2::new(0.85 * o, -0.62 * chord),
            Vec2::new(0.0, -0.85 * chord),
        ]
    };
    (inner, outer)
}

/// Closed Catmull-Rom resampling of a control polygon: `per_edge` points
/// per control edge give smooth, rounded wing and fin outlines with enough
/// vertices for colour patterns.
pub fn smooth_outline(control: &[Vec2], per_edge: usize) -> Vec<Vec2> {
    let n = control.len();
    if n < 3 {
        return control.to_vec();
    }
    let mut out = Vec::with_capacity(n * per_edge.max(1));
    for i in 0..n {
        let p0 = control[(i + n - 1) % n];
        let p1 = control[i];
        let p2 = control[(i + 1) % n];
        let p3 = control[(i + 2) % n];
        for k in 0..per_edge.max(1) {
            let t = k as f32 / per_edge.max(1) as f32;
            let (t2, t3) = (t * t, t * t * t);
            out.push(
                0.5 * ((2.0 * p1)
                    + (p2 - p0) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3),
            );
        }
    }
    out
}

/// Builds a wing (or fin) panel in the part's local frame from an outline
/// whose x runs along `span_dir` (±X) and y along +Z.
#[allow(clippy::too_many_arguments)]
pub fn wing_panel(
    m: &mut MeshData,
    outline: &[Vec2],
    span_dir: f32,
    thickness: f32,
    rings: usize,
    color: &dyn Fn(Vec2, f32) -> Rgba,
) {
    let mirrored: Vec<Vec2> =
        if span_dir < 0.0 { outline.iter().rev().map(|p| Vec2::new(-p.x, p.y)).collect() } else { outline.to_vec() };
    let pivot = mirrored.iter().copied().sum::<Vec2>() / mirrored.len() as f32;
    let pivot = Vec2::new(pivot.x * 0.6, pivot.y);
    planform(m, &mirrored, pivot, rings, thickness, Vec3::ZERO, Vec3::X, Vec3::Z, &|p, f| {
        color(Vec2::new(p.x * span_dir, p.y), f)
    });
}

/// Small dark nose pad at the tip of a mammal snout (full detail only).
pub fn nose(m: &mut MeshData, at: Vec3, r: f32, color: u32, detail: Detail) {
    if detail.small_parts() {
        superellipsoid(m, at, Vec3::new(r, r * 0.75, r * 0.6), 1.0, 1.0, 5, 8, &solid(color));
    }
}
