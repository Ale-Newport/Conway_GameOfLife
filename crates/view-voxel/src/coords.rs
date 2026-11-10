//! Model ↔ Bevy coordinates, box outlines, voxel ray casting and camera
//! framing.
//!
//! The model uses `(x, y, z)` with z as the layer axis; slices along z are
//! the "planes" of the model (`docs/models/voxel-life.md` §7). The view
//! shows z **up**. Bevy is y-up, so model points are rotated by −90° about
//! x — `(x, y, z) ↦ (x, z, −y)`, a proper rotation (no mirror image) — and
//! shifted so the world's footprint is centred on the origin with its floor
//! at Bevy `y = 0`. A voxel at integer position `p` occupies the unit cube
//! `[p, p + 1]` in model space.

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use voxel_life::{Axis, Bounds};

/// Rotation taking model axes (z up) to Bevy axes (y up).
pub fn model_rotation() -> Quat {
    Quat::from_rotation_x(-FRAC_PI_2)
}

/// Placement of the model in the Bevy world: `world = R · (model − anchor)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelFrame {
    /// Model point placed at the Bevy origin.
    pub anchor: Vec3,
}

impl Default for ModelFrame {
    fn default() -> Self {
        Self { anchor: Vec3::ZERO }
    }
}

impl ModelFrame {
    /// Frame for a world box: its footprint is centred and its floor
    /// (lowest z face) sits at Bevy `y = 0`.
    pub fn for_box(bounds: Bounds) -> Self {
        let (min, max) = box_corners(bounds);
        Self { anchor: Vec3::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, min.z) }
    }

    /// Model point → Bevy world point.
    pub fn to_world(self, model: Vec3) -> Vec3 {
        model_rotation() * (model - self.anchor)
    }

    /// Bevy world point → model point.
    pub fn to_model(self, world: Vec3) -> Vec3 {
        model_rotation().inverse() * world + self.anchor
    }

    /// Bevy direction → model direction.
    pub fn dir_to_model(self, world: Vec3) -> Vec3 {
        model_rotation().inverse() * world
    }

    /// Transform of the entity whose children are positioned in model
    /// coordinates.
    pub fn transform(self) -> Transform {
        let rotation = model_rotation();
        Transform { translation: -(rotation * self.anchor), rotation, scale: Vec3::ONE }
    }
}

/// Continuous corners `[min, max + 1]` of an inclusive voxel box.
pub fn box_corners(bounds: Bounds) -> (Vec3, Vec3) {
    let min = Vec3::new(bounds.min[0] as f32, bounds.min[1] as f32, bounds.min[2] as f32);
    let max = Vec3::new(bounds.max[0] as f32 + 1.0, bounds.max[1] as f32 + 1.0, bounds.max[2] as f32 + 1.0);
    (min, max)
}

/// The 12 edges of the axis-aligned box `[min, max]`.
pub fn box_edges(min: Vec3, max: Vec3) -> [(Vec3, Vec3); 12] {
    let c = |x: bool, y: bool, z: bool| {
        Vec3::new(if x { max.x } else { min.x }, if y { max.y } else { min.y }, if z { max.z } else { min.z })
    };
    [
        // Along x.
        (c(false, false, false), c(true, false, false)),
        (c(false, true, false), c(true, true, false)),
        (c(false, false, true), c(true, false, true)),
        (c(false, true, true), c(true, true, true)),
        // Along y.
        (c(false, false, false), c(false, true, false)),
        (c(true, false, false), c(true, true, false)),
        (c(false, false, true), c(false, true, true)),
        (c(true, false, true), c(true, true, true)),
        // Along z.
        (c(false, false, false), c(false, false, true)),
        (c(true, false, false), c(true, false, true)),
        (c(false, true, false), c(false, true, true)),
        (c(true, true, false), c(true, true, true)),
    ]
}

/// Corners (in order around the rectangle) of the plane `axis = at` across
/// the box `[min, max]`.
pub fn plane_rect(min: Vec3, max: Vec3, axis: Axis, at: f32) -> [Vec3; 4] {
    match axis {
        Axis::X => [
            Vec3::new(at, min.y, min.z),
            Vec3::new(at, max.y, min.z),
            Vec3::new(at, max.y, max.z),
            Vec3::new(at, min.y, max.z),
        ],
        Axis::Y => [
            Vec3::new(min.x, at, min.z),
            Vec3::new(max.x, at, min.z),
            Vec3::new(max.x, at, max.z),
            Vec3::new(min.x, at, max.z),
        ],
        Axis::Z => [
            Vec3::new(min.x, min.y, at),
            Vec3::new(max.x, min.y, at),
            Vec3::new(max.x, max.y, at),
            Vec3::new(min.x, max.y, at),
        ],
    }
}

/// First occupied voxel along a ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelHit {
    /// The voxel hit.
    pub voxel: IVec3,
    /// Outward normal of the face the ray entered through (zero when the ray
    /// starts inside the voxel).
    pub normal: IVec3,
    /// Ray parameter at the entry point.
    pub distance: f32,
}

/// Parameter interval where the ray `origin + t·dir` is inside `[min, max]`.
pub fn ray_box(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<(f32, f32, usize)> {
    let mut t_enter = 0.0f32;
    let mut t_exit = f32::INFINITY;
    let mut enter_axis = usize::MAX;
    for a in 0..3 {
        if dir[a].abs() < 1e-12 {
            if origin[a] < min[a] || origin[a] > max[a] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[a];
        let (t0, t1) = {
            let t0 = (min[a] - origin[a]) * inv;
            let t1 = (max[a] - origin[a]) * inv;
            if t0 <= t1 { (t0, t1) } else { (t1, t0) }
        };
        if t0 > t_enter {
            t_enter = t0;
            enter_axis = a;
        }
        t_exit = t_exit.min(t1);
        if t_enter > t_exit {
            return None;
        }
    }
    Some((t_enter, t_exit, enter_axis))
}

/// Walks the voxels pierced by the ray inside the inclusive box
/// `[lo, hi]` (Amanatides–Woo) and returns the first one `occupied`
/// accepts. At most `max_steps` voxels are visited.
pub fn raycast_voxels(
    origin: Vec3,
    dir: Vec3,
    lo: IVec3,
    hi: IVec3,
    max_steps: usize,
    occupied: impl Fn(IVec3) -> bool,
) -> Option<VoxelHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let (bmin, bmax) = (lo.as_vec3(), hi.as_vec3() + Vec3::ONE);
    let (t_enter, t_exit, enter_axis) = ray_box(origin, dir, bmin, bmax)?;
    let start = origin + dir * (t_enter + 1e-4);
    let mut voxel = start.floor().as_ivec3().clamp(lo, hi);
    let mut normal = IVec3::ZERO;
    if enter_axis < 3 {
        normal[enter_axis] = if dir[enter_axis] > 0.0 { -1 } else { 1 };
    }
    let step = IVec3::new(sign(dir.x), sign(dir.y), sign(dir.z));
    let mut t_max = Vec3::ZERO;
    let mut t_delta = Vec3::ZERO;
    for a in 0..3 {
        if step[a] == 0 {
            t_max[a] = f32::INFINITY;
            t_delta[a] = f32::INFINITY;
        } else {
            let boundary = if step[a] > 0 { voxel[a] as f32 + 1.0 } else { voxel[a] as f32 };
            t_max[a] = (boundary - origin[a]) / dir[a];
            t_delta[a] = 1.0 / dir[a].abs();
        }
    }
    let mut t = t_enter;
    for _ in 0..max_steps {
        if occupied(voxel) {
            return Some(VoxelHit { voxel, normal, distance: t });
        }
        let a = if t_max.x < t_max.y {
            if t_max.x < t_max.z { 0 } else { 2 }
        } else if t_max.y < t_max.z {
            1
        } else {
            2
        };
        t = t_max[a];
        if t > t_exit {
            return None;
        }
        voxel[a] += step[a];
        if voxel[a] < lo[a] || voxel[a] > hi[a] {
            return None;
        }
        t_max[a] += t_delta[a];
        normal = IVec3::ZERO;
        normal[a] = -step[a];
    }
    None
}

fn sign(v: f32) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

/// Voxel of layer `layer` (along `axis`) where the ray crosses the middle
/// of that layer, if it does so in front of the origin.
pub fn ray_layer_voxel(origin: Vec3, dir: Vec3, axis: Axis, layer: i32) -> Option<IVec3> {
    let a = axis.index();
    if dir[a].abs() < 1e-6 {
        return None;
    }
    let t = (layer as f32 + 0.5 - origin[a]) / dir[a];
    if t <= 0.0 {
        return None;
    }
    let mut voxel = (origin + dir * t).floor().as_ivec3();
    voxel[a] = layer;
    Some(voxel)
}

/// Whether `voxel` lies in the inclusive box.
pub fn box_contains(bounds: Bounds, voxel: IVec3) -> bool {
    (0..3).all(|a| (bounds.min[a]..=bounds.max[a]).contains(&voxel[a]))
}

/// Camera distance at which a sphere around a box of size `extent` fits the
/// view (vertical field of view `fov`, width / height `aspect`), with 15 %
/// margin.
pub fn framing_distance(extent: Vec3, fov: f32, aspect: f32) -> f32 {
    let radius = (extent.length() * 0.5).max(0.5);
    let half_v = (fov * 0.5).clamp(0.05, 1.5);
    let half_h = (half_v.tan() * aspect.max(0.1)).atan();
    let half = half_v.min(half_h);
    radius / half.sin() * 1.15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_puts_model_z_up_without_mirroring() {
        let r = model_rotation();
        assert!((r * Vec3::Z - Vec3::Y).length() < 1e-6, "model z is Bevy up");
        assert!((r * Vec3::Y - Vec3::NEG_Z).length() < 1e-6);
        assert!((r * Vec3::X - Vec3::X).length() < 1e-6);
        // A proper rotation keeps handedness: (r x) × (r y) = r z.
        assert!(((r * Vec3::X).cross(r * Vec3::Y) - r * Vec3::Z).length() < 1e-6);
    }

    #[test]
    fn frame_centres_the_footprint_on_the_floor() {
        let frame = ModelFrame::for_box(Bounds { min: [0, 0, 0], max: [63, 31, 7] });
        assert_eq!(frame.anchor, Vec3::new(32.0, 16.0, 0.0));
        // The model floor centre is the Bevy origin; the top is 8 voxels up.
        assert!(frame.to_world(Vec3::new(32.0, 16.0, 0.0)).length() < 1e-5);
        assert!((frame.to_world(Vec3::new(32.0, 16.0, 8.0)) - Vec3::new(0.0, 8.0, 0.0)).length() < 1e-5);
        let p = Vec3::new(3.0, -7.5, 12.25);
        assert!((frame.to_model(frame.to_world(p)) - p).length() < 1e-4);
        // The entity transform agrees with `to_world`.
        let t = frame.transform();
        assert!((t.transform_point(p) - frame.to_world(p)).length() < 1e-4);
    }

    #[test]
    fn box_outline_has_twelve_axis_aligned_unit_edges() {
        let edges = box_edges(Vec3::ZERO, Vec3::ONE);
        for (a, b) in edges {
            let d = (b - a).abs();
            assert!((d.x + d.y + d.z - 1.0).abs() < 1e-6, "edge {a:?} -> {b:?}");
        }
        let mut seen: Vec<(IVec3, IVec3)> = edges.iter().map(|(a, b)| (a.as_ivec3(), b.as_ivec3())).collect();
        seen.sort_by_key(|(a, b)| (a.to_array(), b.to_array()));
        seen.dedup();
        assert_eq!(seen.len(), 12);
        assert_eq!(
            box_corners(Bounds { min: [1, 2, 3], max: [1, 2, 3] }),
            (Vec3::new(1.0, 2.0, 3.0), Vec3::splat(2.0) + Vec3::new(0.0, 1.0, 2.0))
        );
    }

    #[test]
    fn plane_rect_spans_the_other_axes() {
        let rect = plane_rect(Vec3::ZERO, Vec3::new(4.0, 5.0, 6.0), Axis::Y, 2.0);
        assert!(rect.iter().all(|p| p.y == 2.0));
        assert_eq!(rect[2], Vec3::new(4.0, 2.0, 6.0));
    }

    #[test]
    fn raycast_finds_the_first_voxel_and_entry_face() {
        let occupied = |v: IVec3| v == IVec3::new(5, 2, 2) || v == IVec3::new(7, 2, 2);
        let hit = raycast_voxels(Vec3::new(-3.0, 2.5, 2.5), Vec3::X, IVec3::ZERO, IVec3::splat(9), 100, occupied)
            .expect("hit");
        assert_eq!(hit.voxel, IVec3::new(5, 2, 2));
        assert_eq!(hit.normal, IVec3::NEG_X, "entered through the -x face");
        assert!((hit.distance - 8.0).abs() < 1e-3);
        // From the other side the +x face of the far voxel is hit first.
        let hit = raycast_voxels(Vec3::new(20.0, 2.5, 2.5), Vec3::NEG_X, IVec3::ZERO, IVec3::splat(9), 100, occupied)
            .expect("hit");
        assert_eq!((hit.voxel, hit.normal), (IVec3::new(7, 2, 2), IVec3::X));
        // Diagonal ray from above hits the top face.
        let hit = raycast_voxels(
            Vec3::new(5.5, 2.5, 30.0),
            Vec3::new(0.0, 0.0, -1.0),
            IVec3::ZERO,
            IVec3::splat(9),
            100,
            occupied,
        )
        .expect("hit");
        assert_eq!((hit.voxel, hit.normal), (IVec3::new(5, 2, 2), IVec3::Z));
        // Misses: pointing away, or nothing occupied on the path.
        assert!(
            raycast_voxels(Vec3::new(-3.0, 2.5, 2.5), Vec3::NEG_X, IVec3::ZERO, IVec3::splat(9), 100, occupied)
                .is_none()
        );
        assert!(
            raycast_voxels(Vec3::new(-3.0, 8.5, 2.5), Vec3::X, IVec3::ZERO, IVec3::splat(9), 100, occupied).is_none()
        );
    }

    #[test]
    fn raycast_walks_every_voxel_along_a_diagonal() {
        // A ray through the box diagonal must visit voxels face-connected:
        // record the visited cells and check each step changes one axis by 1.
        let visited = std::cell::RefCell::new(Vec::new());
        let _ = raycast_voxels(
            Vec3::new(-1.0, -0.8, -0.6),
            Vec3::new(1.0, 0.93, 0.87),
            IVec3::ZERO,
            IVec3::splat(7),
            500,
            |v| {
                visited.borrow_mut().push(v);
                false
            },
        );
        let visited = visited.into_inner();
        assert!(visited.len() > 8);
        for w in visited.windows(2) {
            let d = (w[1] - w[0]).abs();
            assert_eq!(d.x + d.y + d.z, 1, "{:?} -> {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn layer_intersection() {
        let v = ray_layer_voxel(Vec3::new(2.2, 3.7, 10.0), Vec3::new(0.0, 0.0, -1.0), Axis::Z, 0).expect("crosses");
        assert_eq!(v, IVec3::new(2, 3, 0));
        assert!(ray_layer_voxel(Vec3::new(0.0, 0.0, 10.0), Vec3::Z, Axis::Z, 0).is_none(), "behind the origin");
        assert!(box_contains(Bounds { min: [0; 3], max: [3; 3] }, IVec3::new(3, 0, 2)));
        assert!(!box_contains(Bounds { min: [0; 3], max: [3; 3] }, IVec3::new(4, 0, 2)));
    }

    #[test]
    fn framing_distance_fits_the_bounding_sphere() {
        let fov = std::f32::consts::FRAC_PI_4;
        let d = framing_distance(Vec3::splat(64.0), fov, 1.5);
        let radius = Vec3::splat(64.0).length() * 0.5;
        // The sphere subtends exactly the vertical half-angle (plus margin).
        assert!((radius / (d / 1.15) - (fov * 0.5).sin()).abs() < 1e-4);
        // A narrow viewport needs more distance.
        assert!(framing_distance(Vec3::splat(64.0), fov, 0.5) > d);
        assert!(framing_distance(Vec3::ZERO, fov, 1.0) > 0.0);
    }
}
