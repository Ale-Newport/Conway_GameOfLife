//! CPU mesh construction shared by procedural organisms and plants.
//!
//! * [`loft`] sweeps superellipse cross-sections along a spine using
//!   rotation-minimising frames computed with the double-reflection method
//!   (Wang, Jüttler, Zheng & Liu 2008, *ACM TOG* 27(1)), so tubes do not
//!   twist along curved spines. Ends are closed with rounded, pointed or flat
//!   caps.
//! * [`superellipsoid`] builds Barr superquadrics (Barr 1981) as lofts.
//! * [`planform`] builds thin two-sided surfaces from a 2D outline (wings,
//!   fins, ears, leaves), subdivided in concentric rings so that colour
//!   patterns have vertices to live on.
//! * [`lump`] builds noise-displaced icospheres used for foliage clusters.
//!
//! All geometry carries linear-RGBA vertex colours that multiply a white
//! `StandardMaterial`, so every organism of a species shares one material.

use bevy::asset::RenderAssetUsages;
use bevy::math::{Mat3, Vec2, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::Transform;

pub use super::lump::lump;

/// Linear RGBA colour.
pub type Rgba = [f32; 4];

/// Indexed triangle mesh with vertex colours, independent of Bevy assets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Vertex normals (unit length).
    pub normals: Vec<[f32; 3]>,
    /// Vertex colours (linear RGBA).
    pub colors: Vec<Rgba>,
    /// Triangle list indices (counter-clockwise = front).
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Empty mesh.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether the mesh has no triangles.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Adds a vertex and returns its index.
    pub fn vertex(&mut self, position: Vec3, normal: Vec3, color: Rgba) -> u32 {
        let index = self.positions.len() as u32;
        self.positions.push(position.to_array());
        self.normals.push(normal.normalize_or(Vec3::Y).to_array());
        self.colors.push(color);
        index
    }

    /// Adds a triangle.
    pub fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Appends another mesh unchanged.
    pub fn append(&mut self, other: &MeshData) {
        self.append_transformed(other, &Transform::IDENTITY);
    }

    /// Appends another mesh transformed by `t` (normals use the inverse
    /// transpose so non-uniform scale stays correct).
    pub fn append_transformed(&mut self, other: &MeshData, t: &Transform) {
        let base = self.positions.len() as u32;
        let matrix = t.to_matrix();
        let normal_matrix = Mat3::from_mat4(matrix).inverse().transpose();
        let flips = matrix.determinant() < 0.0;
        for ((p, n), c) in other.positions.iter().zip(&other.normals).zip(&other.colors) {
            self.positions.push(matrix.transform_point3(Vec3::from(*p)).to_array());
            self.normals.push((normal_matrix * Vec3::from(*n)).normalize_or(Vec3::Y).to_array());
            self.colors.push(*c);
        }
        for tri in other.indices.as_chunks::<3>().0 {
            if flips {
                self.indices.extend_from_slice(&[base + tri[0], base + tri[2], base + tri[1]]);
            } else {
                self.indices.extend(tri.iter().map(|i| base + i));
            }
        }
    }

    /// Recomputes smooth, area-weighted vertex normals from the triangles.
    pub fn recompute_normals(&mut self) {
        let mut acc = vec![Vec3::ZERO; self.positions.len()];
        for tri in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            let pa = Vec3::from(self.positions[a]);
            let n = (Vec3::from(self.positions[b]) - pa).cross(Vec3::from(self.positions[c]) - pa);
            acc[a] += n;
            acc[b] += n;
            acc[c] += n;
        }
        for (dst, n) in self.normals.iter_mut().zip(acc) {
            if n.length_squared() > 1e-20 {
                *dst = n.normalize().to_array();
            }
        }
    }

    /// Axis-aligned bounds, or `None` for an empty mesh.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self.positions.iter().map(|p| Vec3::from(*p));
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    /// Converts to a Bevy mesh (positions, normals, colours, u32 indices).
    pub fn to_mesh(&self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals.clone());
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors.clone());
        mesh.insert_indices(Indices::U32(self.indices.clone()));
        mesh
    }
}

/// One cross-section of a loft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    /// Centre of the section.
    pub center: Vec3,
    /// Half extent along the lateral axis.
    pub half_width: f32,
    /// Extent above the spine (dorsal).
    pub top: f32,
    /// Extent below the spine (ventral).
    pub bottom: f32,
    /// Superellipse exponent: 2 = ellipse, > 2 boxier, < 2 diamond-like.
    pub exponent: f32,
}

impl Ring {
    /// Circular section of radius `r`.
    pub fn circle(center: Vec3, r: f32) -> Self {
        Self { center, half_width: r, top: r, bottom: r, exponent: 2.0 }
    }

    /// Elliptic section with half width `w` and half height `h`.
    pub fn ellipse(center: Vec3, w: f32, h: f32) -> Self {
        Self { center, half_width: w, top: h, bottom: h, exponent: 2.0 }
    }

    /// Section with different dorsal and ventral extents.
    pub fn asym(center: Vec3, w: f32, top: f32, bottom: f32) -> Self {
        Self { center, half_width: w, top, bottom, exponent: 2.0 }
    }

    /// Same section with another superellipse exponent.
    pub fn with_exponent(mut self, exponent: f32) -> Self {
        self.exponent = exponent;
        self
    }

    /// Mean radius.
    pub fn mean_radius(&self) -> f32 {
        (2.0 * self.half_width + self.top + self.bottom) * 0.25
    }

    fn scaled(&self, s: f32, center: Vec3) -> Self {
        Self {
            center,
            half_width: self.half_width * s,
            top: self.top * s,
            bottom: self.bottom * s,
            exponent: self.exponent,
        }
    }
}

/// How a loft end is closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cap {
    /// Left open.
    Open,
    /// Closed with a flat fan.
    Flat,
    /// Closed with a hemispherical dome of the end ring's size.
    Round,
    /// Closed with a cone whose tip lies this far beyond the end ring.
    Point(f32),
}

/// Where on a loft surface a vertex lies, passed to colour functions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfacePoint {
    /// Arc-length fraction along the spine (0 = first ring, 1 = last).
    pub along: f32,
    /// Angle fraction around the section (0 = dorsal midline, 0.5 = ventral).
    pub around: f32,
    /// −1 on the ventral midline, +1 on the dorsal midline.
    pub dorsal: f32,
    /// −1 on the right flank, +1 on the left flank.
    pub lateral: f32,
    /// Local position.
    pub position: Vec3,
}

#[inline]
fn signed_pow(v: f32, e: f32) -> f32 {
    v.signum() * v.abs().powf(e)
}

/// Rotation-minimising frames (tangent, reference normal) along `points`.
fn rmf_frames(points: &[Vec3], up_hint: Vec3) -> Vec<(Vec3, Vec3)> {
    let n = points.len();
    let tangent = |i: usize| -> Vec3 {
        let a = points[i.saturating_sub(1)];
        let b = points[(i + 1).min(n - 1)];
        (b - a).normalize_or(Vec3::Z)
    };
    let t0 = tangent(0);
    let mut r = (up_hint - t0 * up_hint.dot(t0)).normalize_or_zero();
    if r == Vec3::ZERO {
        r = t0.any_orthonormal_vector();
    }
    let mut frames = Vec::with_capacity(n);
    frames.push((t0, r));
    for i in 0..n.saturating_sub(1) {
        let (ti, ri) = frames[i];
        let t_next = tangent(i + 1);
        let v1 = points[i + 1] - points[i];
        let c1 = v1.dot(v1);
        let r_next = if c1 < 1e-12 {
            (ri - t_next * ri.dot(t_next)).normalize_or(ri)
        } else {
            let r_l = ri - (2.0 / c1) * v1.dot(ri) * v1;
            let t_l = ti - (2.0 / c1) * v1.dot(ti) * v1;
            let v2 = t_next - t_l;
            let c2 = v2.dot(v2);
            let r = if c2 < 1e-12 { r_l } else { r_l - (2.0 / c2) * v2.dot(r_l) * v2 };
            (r - t_next * r.dot(t_next)).normalize_or(ri)
        };
        frames.push((t_next, r_next));
    }
    frames
}

/// Sweeps `rings` along their centres. `up_hint` orients the dorsal side of
/// the first section (it is then carried along without twist). Every ring
/// has `segments` vertices. Normals are smooth within the loft.
pub fn loft(
    out: &mut MeshData,
    rings: &[Ring],
    segments: usize,
    up_hint: Vec3,
    start: Cap,
    end: Cap,
    color: &dyn Fn(&SurfacePoint) -> Rgba,
) {
    if rings.len() < 2 || segments < 3 {
        return;
    }
    let mut piece = MeshData::new();
    let centers: Vec<Vec3> = rings.iter().map(|r| r.center).collect();
    let frames = rmf_frames(&centers, up_hint);
    let mut cumulative = vec![0.0f32; rings.len()];
    for i in 1..rings.len() {
        cumulative[i] = cumulative[i - 1] + centers[i].distance(centers[i - 1]);
    }
    let total = cumulative[rings.len() - 1].max(1e-6);

    // Expand caps into extra rings.
    let mut all: Vec<(Ring, Vec3, Vec3, f32)> = Vec::new(); // ring, tangent, normal, along
    // Coarse sections get a single intermediate dome ring, fine ones three.
    let dome_steps: usize = if segments <= 6 { 1 } else { 3 };
    let dome_angle = |k: usize| k as f32 / (dome_steps + 1) as f32 * std::f32::consts::FRAC_PI_2;
    let cap_rings =
        |ring: &Ring, t: Vec3, n: Vec3, dir: f32, along: f32, out: &mut Vec<(Ring, Vec3, Vec3, f32)>, reverse: bool| {
            let len = ring.mean_radius();
            let mut extra = Vec::new();
            for k in 1..=dome_steps {
                let phi = dome_angle(k);
                extra.push((ring.scaled(phi.cos(), ring.center + t * dir * len * phi.sin()), t, n, along));
            }
            if reverse {
                extra.reverse();
            }
            out.extend(extra);
        };
    if start == Cap::Round {
        cap_rings(&rings[0], frames[0].0, frames[0].1, -1.0, 0.0, &mut all, true);
    }
    for (i, r) in rings.iter().enumerate() {
        all.push((*r, frames[i].0, frames[i].1, cumulative[i] / total));
    }
    let last = rings.len() - 1;
    if end == Cap::Round {
        cap_rings(&rings[last], frames[last].0, frames[last].1, 1.0, 1.0, &mut all, false);
    }

    let mut ring_start = Vec::with_capacity(all.len());
    for (ring, t, n, along) in &all {
        let lat = n.cross(*t);
        ring_start.push(piece.vertex_count() as u32);
        let e = 2.0 / ring.exponent.max(0.1);
        for j in 0..segments {
            let theta = j as f32 / segments as f32 * std::f32::consts::TAU;
            let (s, c) = theta.sin_cos();
            let d = signed_pow(c, e);
            let l = signed_pow(s, e);
            let h = if d >= 0.0 { ring.top * d } else { ring.bottom * d };
            let p = ring.center + *n * h + lat * (ring.half_width * l);
            let sp =
                SurfacePoint { along: *along, around: j as f32 / segments as f32, dorsal: c, lateral: s, position: p };
            piece.vertex(p, *n * c + lat * s, color(&sp));
        }
    }
    let seg = segments as u32;
    for w in ring_start.windows(2) {
        let (a0, b0) = (w[0], w[1]);
        for j in 0..seg {
            let j1 = (j + 1) % seg;
            piece.triangle(a0 + j, b0 + j, a0 + j1);
            piece.triangle(a0 + j1, b0 + j, b0 + j1);
        }
    }
    // Poles.
    let pole = |piece: &mut MeshData, ring: &Ring, t: Vec3, offset: f32, along: f32| -> u32 {
        let p = ring.center + t * offset;
        let sp = SurfacePoint { along, around: 0.0, dorsal: 0.0, lateral: 0.0, position: p };
        piece.vertex(p, t * offset.signum(), color(&sp))
    };
    // The pole closes the dome above its last intermediate ring.
    let dome_rest = 1.0 - dome_angle(dome_steps).sin();
    let (first_ring, ft, _, _) = all[0];
    let (last_ring, lt, _, _) = all[all.len() - 1];
    let first_start = ring_start[0];
    let last_start = ring_start[ring_start.len() - 1];
    let start_offset = match start {
        Cap::Open => None,
        Cap::Flat => Some(-1e-4),
        Cap::Round => Some(-(dome_rest * rings[0].mean_radius()).max(1e-4)),
        Cap::Point(ext) => Some(-ext.max(1e-4)),
    };
    if let Some(off) = start_offset {
        let q = pole(&mut piece, &first_ring, ft, off, 0.0);
        for j in 0..seg {
            piece.triangle(q, first_start + j, first_start + (j + 1) % seg);
        }
    }
    let end_offset = match end {
        Cap::Open => None,
        Cap::Flat => Some(1e-4),
        Cap::Round => Some((dome_rest * rings[last].mean_radius()).max(1e-4)),
        Cap::Point(ext) => Some(ext.max(1e-4)),
    };
    if let Some(off) = end_offset {
        let p = pole(&mut piece, &last_ring, lt, off, 1.0);
        for j in 0..seg {
            piece.triangle(last_start + j, p, last_start + (j + 1) % seg);
        }
    }
    piece.recompute_normals();
    out.append(&piece);
}

/// Circular tube through `points` with per-point radii.
pub fn tube(
    out: &mut MeshData,
    points: &[Vec3],
    radii: &[f32],
    segments: usize,
    up_hint: Vec3,
    caps: (Cap, Cap),
    color: &dyn Fn(&SurfacePoint) -> Rgba,
) {
    let rings: Vec<Ring> = points.iter().zip(radii).map(|(p, r)| Ring::circle(*p, r.max(1e-4))).collect();
    loft(out, &rings, segments, up_hint, caps.0, caps.1, color);
}

/// Superellipsoid centred at `center` with semi-axes `radii` (x lateral,
/// y dorsal, z along). `e_ns` shapes the profile along z (1 = ellipsoid,
/// < 1 boxy, > 1 pinched); `e_ew` shapes the cross-section (1 = round).
#[allow(clippy::too_many_arguments)]
pub fn superellipsoid(
    out: &mut MeshData,
    center: Vec3,
    radii: Vec3,
    e_ns: f32,
    e_ew: f32,
    stacks: usize,
    segments: usize,
    color: &dyn Fn(&SurfacePoint) -> Rgba,
) {
    let stacks = stacks.max(3);
    let mut rings = Vec::with_capacity(stacks);
    for k in 1..stacks {
        let phi = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * k as f32 / stacks as f32;
        let z = radii.z * signed_pow(phi.sin(), e_ns);
        let s = signed_pow(phi.cos(), e_ns).max(1e-3);
        rings.push(Ring {
            center: center + Vec3::Z * z,
            half_width: radii.x * s,
            top: radii.y * s,
            bottom: radii.y * s,
            exponent: 2.0 / e_ew.max(0.05),
        });
    }
    let first_gap = (rings[0].center.z - (center.z - radii.z)).max(1e-4);
    let last_gap = ((center.z + radii.z) - rings[rings.len() - 1].center.z).max(1e-4);
    loft(out, &rings, segments, Vec3::Y, Cap::Point(first_gap), Cap::Point(last_gap), color);
}

/// Thin two-sided surface. `outline` is a closed, star-shaped polygon (as
/// seen from `pivot`) in the plane spanned by `u` and `v` around `origin`;
/// the surface bulges by up to `thickness / 2` on each side along
/// `u × v`. `rings` concentric subdivisions give interior vertices; the
/// colour function receives the outline-space point and the radial fraction
/// (0 at the pivot, 1 on the outline).
#[allow(clippy::too_many_arguments)]
pub fn planform(
    out: &mut MeshData,
    outline: &[Vec2],
    pivot: Vec2,
    rings: usize,
    thickness: f32,
    origin: Vec3,
    u: Vec3,
    v: Vec3,
    color: &dyn Fn(Vec2, f32) -> Rgba,
) {
    if outline.len() < 3 {
        return;
    }
    let rings = rings.max(1);
    let normal = u.cross(v).normalize_or(Vec3::Y);
    let n = outline.len() as u32;
    let mut piece = MeshData::new();
    for side in [1.0f32, -1.0] {
        let base = piece.vertex_count() as u32;
        let center = origin + u * pivot.x + v * pivot.y + normal * side * thickness * 0.5;
        piece.vertex(center, normal * side, color(pivot, 0.0));
        for k in 1..=rings {
            let f = k as f32 / rings as f32;
            for q in outline {
                let p2 = pivot + (*q - pivot) * f;
                let bulge = thickness * 0.5 * (1.0 - f * f);
                let p = origin + u * p2.x + v * p2.y + normal * side * bulge;
                piece.vertex(p, normal * side, color(p2, f));
            }
        }
        let ring_index = |k: u32, i: u32| base + 1 + (k - 1) * n + (i % n);
        for i in 0..n {
            let (a, b) = (ring_index(1, i), ring_index(1, i + 1));
            if side > 0.0 { piece.triangle(base, a, b) } else { piece.triangle(base, b, a) }
        }
        for k in 1..rings as u32 {
            for i in 0..n {
                let (a, b) = (ring_index(k, i), ring_index(k, i + 1));
                let (c, d) = (ring_index(k + 1, i), ring_index(k + 1, i + 1));
                if side > 0.0 {
                    piece.triangle(a, c, b);
                    piece.triangle(b, c, d);
                } else {
                    piece.triangle(a, b, c);
                    piece.triangle(b, d, c);
                }
            }
        }
    }
    // Orientation check: outline order decides the winding; make the first
    // side face `normal`.
    let area: f32 = outline.iter().zip(outline.iter().cycle().skip(1)).map(|(a, b)| a.perp_dot(*b)).sum();
    if area < 0.0 {
        for tri in piece.indices.as_chunks_mut::<3>().0 {
            tri.swap(1, 2);
        }
    }
    out.append(&piece);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white(_: &SurfacePoint) -> Rgba {
        [1.0; 4]
    }

    /// Signed volume via the divergence theorem; positive for outward winding.
    fn signed_volume(m: &MeshData) -> f32 {
        m.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(m.positions[i as usize]));
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    fn assert_valid(m: &MeshData) {
        assert_eq!(m.positions.len(), m.normals.len());
        assert_eq!(m.positions.len(), m.colors.len());
        assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()));
        assert!(m.positions.iter().flatten().all(|v| v.is_finite()));
        assert!(m.normals.iter().all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-3));
    }

    #[test]
    fn closed_tube_is_outward_and_has_cylinder_volume() {
        let mut m = MeshData::new();
        let pts: Vec<Vec3> = (0..=10).map(|i| Vec3::new(0.0, 0.0, i as f32 * 0.2)).collect();
        tube(&mut m, &pts, &[0.5; 11], 32, Vec3::Y, (Cap::Flat, Cap::Flat), &white);
        assert_valid(&m);
        let v = signed_volume(&m);
        let expected = std::f32::consts::PI * 0.25 * 2.0;
        assert!((v - expected).abs() / expected < 0.02, "volume {v} vs {expected}");
    }

    #[test]
    fn rounded_caps_add_hemispheres() {
        let mut m = MeshData::new();
        let pts = [Vec3::ZERO, Vec3::Z];
        tube(&mut m, &pts, &[0.5, 0.5], 32, Vec3::Y, (Cap::Round, Cap::Round), &white);
        let v = signed_volume(&m);
        let expected = std::f32::consts::PI * 0.25 + 4.0 / 3.0 * std::f32::consts::PI * 0.125;
        assert!((v - expected).abs() / expected < 0.06, "volume {v} vs {expected}");
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo.z + 0.5).abs() < 1e-3 && (hi.z - 1.5).abs() < 1e-3);
    }

    #[test]
    fn rmf_does_not_twist_on_a_planar_curve() {
        // A quarter circle in the XZ plane: the reference normal must stay +Y.
        let pts: Vec<Vec3> = (0..=16)
            .map(|i| {
                let a = i as f32 / 16.0 * std::f32::consts::FRAC_PI_2;
                Vec3::new(a.cos(), 0.0, a.sin())
            })
            .collect();
        for (_, r) in rmf_frames(&pts, Vec3::Y) {
            assert!(r.dot(Vec3::Y) > 0.999, "{r}");
        }
    }

    #[test]
    fn dorsal_side_follows_up_hint() {
        let mut m = MeshData::new();
        let rings = [Ring::asym(Vec3::ZERO, 0.2, 0.5, 0.1), Ring::asym(Vec3::Z, 0.2, 0.5, 0.1)];
        loft(&mut m, &rings, 16, Vec3::Y, Cap::Open, Cap::Open, &|sp| [sp.dorsal, 0.0, 0.0, 1.0]);
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi.y - 0.5).abs() < 1e-4 && (lo.y + 0.1).abs() < 1e-4);
        // The top-most vertex is coloured as dorsal.
        let top = m.positions.iter().enumerate().max_by(|a, b| a.1[1].total_cmp(&b.1[1])).unwrap().0;
        assert!(m.colors[top][0] > 0.99);
    }

    #[test]
    fn superellipsoid_volume_matches_ellipsoid() {
        let mut m = MeshData::new();
        superellipsoid(&mut m, Vec3::ZERO, Vec3::new(1.0, 0.5, 2.0), 1.0, 1.0, 24, 32, &white);
        assert_valid(&m);
        let v = signed_volume(&m);
        let expected = 4.0 / 3.0 * std::f32::consts::PI * 1.0;
        assert!((v - expected).abs() / expected < 0.03, "{v} vs {expected}");
    }

    #[test]
    fn planform_is_two_sided_and_flat_volume_small() {
        let mut m = MeshData::new();
        let outline = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.2), Vec2::new(1.2, 0.6), Vec2::new(0.2, 0.5)];
        planform(&mut m, &outline, Vec2::new(0.4, 0.3), 3, 0.02, Vec3::ZERO, Vec3::X, Vec3::Z, &|_, _| [1.0; 4]);
        assert_valid(&m);
        assert!(signed_volume(&m) > 0.0, "outward winding");
        // Both faces exist: as many triangles facing +n as -n.
        let normal = Vec3::X.cross(Vec3::Z);
        let (mut up, mut down) = (0, 0);
        for t in m.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(m.positions[i as usize]));
            if (b - a).cross(c - a).dot(normal) > 0.0 { up += 1 } else { down += 1 }
        }
        assert_eq!(up, down);
    }

    #[test]
    fn mirrored_append_keeps_outward_winding() {
        let mut src = MeshData::new();
        tube(&mut src, &[Vec3::ZERO, Vec3::Z], &[0.3, 0.3], 12, Vec3::Y, (Cap::Flat, Cap::Flat), &white);
        let mut mirrored = MeshData::new();
        mirrored.append_transformed(&src, &Transform::from_scale(Vec3::new(-1.0, 1.0, 1.0)));
        assert!(signed_volume(&mirrored) > 0.0);
        assert!((signed_volume(&mirrored) - signed_volume(&src)).abs() < 1e-4);
    }
}
