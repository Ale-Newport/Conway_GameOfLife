//! Face-culled voxel meshing.
//!
//! Every voxel is a unit cube `[x, x+1] × [y, y+1] × [z, z+1]` scaled by the
//! voxel size; voxel axes map directly to Bevy axes (y up). A face is emitted
//! only when it can be seen:
//!
//! * an **opaque** voxel hides its face if the neighbour is opaque;
//! * a **translucent** voxel (palette alpha < 1) hides its face if the
//!   neighbour is opaque or translucent with the same state (no internal faces
//!   inside a block of glass).
//!
//! Opaque and translucent faces go to separate meshes so they can use
//! different materials. Output order is deterministic (voxels sorted by
//! position), independent of input order. State 0 means empty. Coordinates
//! must lie within `±`[`MAX_VOXEL_COORD`]; meshing rejects other inputs with
//! [`VoxelError::OutOfRange`].

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Largest absolute voxel coordinate accepted (2^24). Vertex positions are
/// `f32`, which holds integers exactly only up to 2^24, and the limit keeps
/// neighbour and chunk arithmetic far from `i32` overflow.
pub const MAX_VOXEL_COORD: i32 = 1 << 24;

/// One voxel of a volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Voxel {
    /// Column (Bevy x).
    pub x: i32,
    /// Height (Bevy y).
    pub y: i32,
    /// Depth (Bevy z).
    pub z: i32,
    /// State; 0 = empty (never drawn).
    pub state: u8,
}

impl Voxel {
    /// Creates a voxel.
    pub const fn new(x: i32, y: i32, z: i32, state: u8) -> Self {
        Self { x, y, z, state }
    }

    /// Position as a vector.
    pub fn pos(&self) -> IVec3 {
        IVec3::new(self.x, self.y, self.z)
    }
}

/// Errors from voxel configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoxelError {
    /// The palette has no colours.
    EmptyPalette,
    /// Chunk size outside `1..=256`.
    ChunkSize(u32),
    /// A voxel coordinate exceeds [`MAX_VOXEL_COORD`] in absolute value.
    OutOfRange {
        /// The offending voxel.
        voxel: Voxel,
    },
}

impl fmt::Display for VoxelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPalette => write!(f, "a voxel palette needs at least one colour"),
            Self::ChunkSize(size) => write!(f, "voxel chunk size must be between 1 and 256, got {size}"),
            Self::OutOfRange { voxel } => write!(
                f,
                "voxel ({}, {}, {}) is outside the supported range ±{MAX_VOXEL_COORD}",
                voxel.x, voxel.y, voxel.z
            ),
        }
    }
}

impl std::error::Error for VoxelError {}

/// Colours per voxel state (index 0 is unused: empty). States beyond the last
/// colour use the last colour. Alpha < 1 marks a state as translucent.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxelPalette {
    colors: Vec<LinearRgba>,
}

impl VoxelPalette {
    /// Builds a palette; `colors[s]` is the colour of state `s`.
    pub fn new(colors: Vec<Color>) -> Result<Self, VoxelError> {
        if colors.is_empty() {
            return Err(VoxelError::EmptyPalette);
        }
        Ok(Self { colors: colors.into_iter().map(|c| c.to_linear()).collect() })
    }

    /// Linear colour of a state.
    pub fn color(&self, state: u8) -> LinearRgba {
        let i = (state as usize).min(self.colors.len() - 1);
        self.colors[i]
    }

    /// Whether a state renders translucent.
    pub fn is_translucent(&self, state: u8) -> bool {
        self.color(state).alpha < 1.0
    }

    /// Number of colours.
    pub fn len(&self) -> usize {
        self.colors.len()
    }

    /// Always false for a constructed palette.
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }
}

/// Vertex buffers of one voxel mesh (positions relative to the mesh origin).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoxelMeshData {
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Vertex normals (one per face direction).
    pub normals: Vec<[f32; 3]>,
    /// Linear RGBA vertex colours.
    pub colors: Vec<[f32; 4]>,
    /// Triangle indices (two triangles per face, counter-clockwise outside).
    pub indices: Vec<u32>,
}

impl VoxelMeshData {
    /// Number of emitted faces.
    pub fn face_count(&self) -> usize {
        self.indices.len() / 6
    }

    /// Whether no face was emitted.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Converts to a Bevy mesh (kept in the main world so it can be replaced).
    pub fn to_mesh(&self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions.clone())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals.clone())
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors.clone())
            .with_inserted_indices(Indices::U32(self.indices.clone()))
    }

    fn push_face(&mut self, origin: Vec3, face: &Face, size: f32, color: LinearRgba) {
        let base = self.positions.len() as u32;
        let n = face.normal.as_vec3();
        let corner0 = origin + face.base.as_vec3();
        let (u, v) = (face.u.as_vec3(), face.v.as_vec3());
        for offset in [Vec3::ZERO, u, u + v, v] {
            self.positions.push(((corner0 + offset) * size).to_array());
            self.normals.push(n.to_array());
            self.colors.push(color.to_f32_array());
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Opaque and translucent parts of a voxel mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoxelMeshes {
    /// Faces of opaque states.
    pub opaque: VoxelMeshData,
    /// Faces of translucent states.
    pub translucent: VoxelMeshData,
}

impl VoxelMeshes {
    /// Total number of faces.
    pub fn face_count(&self) -> usize {
        self.opaque.face_count() + self.translucent.face_count()
    }
}

/// A cube face: outward normal, the corner where it starts and its edge
/// vectors with `u × v = normal` (so corners are counter-clockwise outside).
pub(crate) struct Face {
    pub normal: IVec3,
    pub base: IVec3,
    pub u: IVec3,
    pub v: IVec3,
}

pub(crate) const FACES: [Face; 6] = [
    Face { normal: IVec3::X, base: IVec3::new(1, 0, 0), u: IVec3::Y, v: IVec3::Z },
    Face { normal: IVec3::NEG_X, base: IVec3::ZERO, u: IVec3::Z, v: IVec3::Y },
    Face { normal: IVec3::Y, base: IVec3::new(0, 1, 0), u: IVec3::Z, v: IVec3::X },
    Face { normal: IVec3::NEG_Y, base: IVec3::ZERO, u: IVec3::X, v: IVec3::Z },
    Face { normal: IVec3::Z, base: IVec3::new(0, 0, 1), u: IVec3::X, v: IVec3::Y },
    Face { normal: IVec3::NEG_Z, base: IVec3::ZERO, u: IVec3::Y, v: IVec3::X },
];

/// Checks that every voxel lies within `±`[`MAX_VOXEL_COORD`].
pub fn check_voxels(voxels: &[Voxel]) -> Result<(), VoxelError> {
    let ok = |c: i32| (-MAX_VOXEL_COORD..=MAX_VOXEL_COORD).contains(&c);
    match voxels.iter().find(|v| !(ok(v.x) && ok(v.y) && ok(v.z))) {
        Some(&voxel) => Err(VoxelError::OutOfRange { voxel }),
        None => Ok(()),
    }
}

/// Occupancy lookup: position → non-zero state. Duplicates keep the last.
pub(crate) fn occupancy(voxels: &[Voxel]) -> HashMap<IVec3, u8> {
    voxels.iter().filter(|v| v.state != 0).map(|v| (v.pos(), v.state)).collect()
}

/// Whether the face of a voxel with `state` towards a neighbour with
/// `neighbour` (None = empty) is visible.
pub fn face_visible(palette: &VoxelPalette, state: u8, neighbour: Option<u8>) -> bool {
    let Some(other) = neighbour else { return true };
    if !palette.is_translucent(other) {
        return false;
    }
    // Neighbour is translucent: opaque voxels show the face; translucent ones
    // hide it only against the same material.
    !(palette.is_translucent(state) && other == state)
}

/// Meshes `voxels` (positions relative to `origin`, in voxels) using `lookup`
/// for neighbour occupancy. `voxels` must be sorted and free of duplicates
/// and empty states.
pub(crate) fn mesh_sorted(
    voxels: &[Voxel],
    lookup: &HashMap<IVec3, u8>,
    palette: &VoxelPalette,
    voxel_size: f32,
    origin: IVec3,
) -> VoxelMeshes {
    let mut out = VoxelMeshes::default();
    for voxel in voxels {
        let pos = voxel.pos();
        let translucent = palette.is_translucent(voxel.state);
        let color = palette.color(voxel.state);
        let local = (pos - origin).as_vec3();
        for face in &FACES {
            let neighbour = lookup.get(&(pos + face.normal)).copied();
            if face_visible(palette, voxel.state, neighbour) {
                let target = if translucent { &mut out.translucent } else { &mut out.opaque };
                target.push_face(local, face, voxel_size, color);
            }
        }
    }
    out
}

/// Sorted, de-duplicated (last wins), non-empty voxels of an occupancy map.
pub(crate) fn sorted_voxels(lookup: &HashMap<IVec3, u8>) -> Vec<Voxel> {
    let mut sorted: Vec<Voxel> = lookup.iter().map(|(p, &state)| Voxel::new(p.x, p.y, p.z, state)).collect();
    sorted.sort_unstable();
    sorted
}

/// Meshes a list of voxels with per-face culling (positions in world voxel
/// coordinates times `voxel_size`). Fails if a coordinate is out of range.
pub fn mesh_voxels(voxels: &[Voxel], palette: &VoxelPalette, voxel_size: f32) -> Result<VoxelMeshes, VoxelError> {
    check_voxels(voxels)?;
    let lookup = occupancy(voxels);
    let sorted = sorted_voxels(&lookup);
    Ok(mesh_sorted(&sorted, &lookup, palette, voxel_size, IVec3::ZERO))
}

/// Which side of a slice plane is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceKeep {
    /// Keep voxels with coordinate `<= position` (cutaway from above).
    Below,
    /// Keep voxels with coordinate `>= position`.
    Above,
    /// Keep only the layer `== position`.
    Layer,
}

/// Axis of a slice plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliceAxis {
    /// x = const.
    X,
    /// y = const.
    Y,
    /// z = const.
    Z,
}

/// An axis-aligned cut through a volume. Faces exposed by the cut appear
/// automatically because the removed voxels no longer occlude them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VoxelSlice {
    /// Plane axis.
    pub axis: SliceAxis,
    /// Plane coordinate (in voxels).
    pub position: i32,
    /// Kept side.
    pub keep: SliceKeep,
}

impl VoxelSlice {
    /// Whether a voxel survives the cut.
    pub fn contains(&self, voxel: &Voxel) -> bool {
        let c = match self.axis {
            SliceAxis::X => voxel.x,
            SliceAxis::Y => voxel.y,
            SliceAxis::Z => voxel.z,
        };
        match self.keep {
            SliceKeep::Below => c <= self.position,
            SliceKeep::Above => c >= self.position,
            SliceKeep::Layer => c == self.position,
        }
    }

    /// The voxels that survive the cut.
    pub fn apply(&self, voxels: &[Voxel]) -> Vec<Voxel> {
        voxels.iter().filter(|v| self.contains(v)).copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> VoxelPalette {
        VoxelPalette::new(vec![
            Color::NONE,
            Color::srgb(1.0, 0.0, 0.0),
            Color::srgb(0.0, 1.0, 0.0),
            Color::srgba(0.5, 0.5, 1.0, 0.4),
            Color::srgba(1.0, 1.0, 0.5, 0.4),
        ])
        .expect("non-empty")
    }

    #[test]
    fn single_voxel_has_six_faces() {
        let m = mesh_voxels(&[Voxel::new(0, 0, 0, 1)], &palette(), 1.0).expect("in range");
        assert_eq!(m.opaque.face_count(), 6);
        assert_eq!(m.opaque.positions.len(), 24);
        assert!(m.translucent.is_empty());
    }

    #[test]
    fn adjacent_voxels_share_hidden_faces() {
        let m = mesh_voxels(&[Voxel::new(0, 0, 0, 1), Voxel::new(1, 0, 0, 2)], &palette(), 1.0).expect("in range");
        assert_eq!(m.face_count(), 10);
        // Diagonal neighbours do not occlude.
        let m = mesh_voxels(&[Voxel::new(0, 0, 0, 1), Voxel::new(1, 1, 0, 1)], &palette(), 1.0).expect("in range");
        assert_eq!(m.face_count(), 12);
    }

    #[test]
    fn solid_cube_only_has_surface_faces() {
        let mut voxels = Vec::new();
        for x in 0..3 {
            for y in 0..3 {
                for z in 0..3 {
                    voxels.push(Voxel::new(x, y, z, 1));
                }
            }
        }
        assert_eq!(mesh_voxels(&voxels, &palette(), 1.0).expect("in range").face_count(), 54);
        // Hollow out the centre: interior faces stay hidden (centre is enclosed
        // but its neighbours now expose 6 inner faces).
        voxels.retain(|v| v.pos() != IVec3::ONE);
        assert_eq!(mesh_voxels(&voxels, &palette(), 1.0).expect("in range").face_count(), 60);
    }

    #[test]
    fn empty_states_and_duplicates_are_ignored() {
        let voxels = [Voxel::new(0, 0, 0, 1), Voxel::new(0, 0, 0, 1), Voxel::new(1, 0, 0, 0)];
        assert_eq!(mesh_voxels(&voxels, &palette(), 1.0).expect("in range").face_count(), 6);
        assert_eq!(mesh_voxels(&[], &palette(), 1.0).expect("in range").face_count(), 0);
    }

    #[test]
    fn translucent_culling_rules() {
        let p = palette();
        // Opaque next to glass: opaque keeps all 6 faces, glass hides the shared one.
        let m = mesh_voxels(&[Voxel::new(0, 0, 0, 1), Voxel::new(1, 0, 0, 3)], &p, 1.0).expect("in range");
        assert_eq!(m.opaque.face_count(), 6);
        assert_eq!(m.translucent.face_count(), 5);
        // Same glass: internal faces removed.
        let m = mesh_voxels(&[Voxel::new(0, 0, 0, 3), Voxel::new(1, 0, 0, 3)], &p, 1.0).expect("in range");
        assert_eq!(m.translucent.face_count(), 10);
        // Different glass: both faces kept.
        let m = mesh_voxels(&[Voxel::new(0, 0, 0, 3), Voxel::new(1, 0, 0, 4)], &p, 1.0).expect("in range");
        assert_eq!(m.translucent.face_count(), 12);
    }

    #[test]
    fn faces_wind_counter_clockwise_outward() {
        let m = mesh_voxels(&[Voxel::new(2, -1, 5, 1)], &palette(), 0.5).expect("in range");
        let pos = |i: u32| Vec3::from_array(m.opaque.positions[i as usize]);
        let centre = (Vec3::new(2.0, -1.0, 5.0) + Vec3::splat(0.5)) * 0.5;
        for tri in m.opaque.indices.as_chunks::<3>().0 {
            let (a, b, c) = (pos(tri[0]), pos(tri[1]), pos(tri[2]));
            let normal = (b - a).cross(c - a);
            let stored = Vec3::from_array(m.opaque.normals[tri[0] as usize]);
            assert!(normal.dot(stored) > 0.0, "winding must match the normal");
            assert!(stored.dot(a - centre) > 0.0, "normal must point outward");
        }
    }

    #[test]
    fn voxel_size_scales_positions() {
        let m = mesh_voxels(&[Voxel::new(1, 2, 3, 2)], &palette(), 2.0).expect("in range");
        let max = m.opaque.positions.iter().fold(Vec3::splat(f32::MIN), |acc, p| acc.max(Vec3::from_array(*p)));
        let min = m.opaque.positions.iter().fold(Vec3::splat(f32::MAX), |acc, p| acc.min(Vec3::from_array(*p)));
        assert_eq!(min, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(max, Vec3::new(4.0, 6.0, 8.0));
        assert_eq!(m.opaque.colors[0], palette().color(2).to_f32_array());
    }

    #[test]
    fn output_is_independent_of_input_order() {
        let a = [Voxel::new(0, 0, 0, 1), Voxel::new(0, 1, 0, 2), Voxel::new(5, 0, 0, 3)];
        let b = [a[2], a[0], a[1]];
        assert_eq!(
            mesh_voxels(&a, &palette(), 1.0).expect("in range"),
            mesh_voxels(&b, &palette(), 1.0).expect("in range")
        );
    }

    #[test]
    fn slice_plane_filters_and_exposes_faces() {
        let voxels: Vec<Voxel> = (0..4).map(|y| Voxel::new(0, y, 0, 1)).collect();
        let below = VoxelSlice { axis: SliceAxis::Y, position: 1, keep: SliceKeep::Below };
        assert_eq!(below.apply(&voxels).len(), 2);
        let layer = VoxelSlice { axis: SliceAxis::Y, position: 2, keep: SliceKeep::Layer };
        assert_eq!(layer.apply(&voxels), vec![Voxel::new(0, 2, 0, 1)]);
        let above = VoxelSlice { axis: SliceAxis::X, position: 1, keep: SliceKeep::Above };
        assert!(above.apply(&voxels).is_empty());
        // Column of 4 has 18 faces; cut to 2 it has 10 (top face exposed).
        assert_eq!(mesh_voxels(&voxels, &palette(), 1.0).expect("in range").face_count(), 18);
        assert_eq!(mesh_voxels(&below.apply(&voxels), &palette(), 1.0).expect("in range").face_count(), 10);
    }

    #[test]
    fn out_of_range_coordinates_are_rejected() {
        let edge = Voxel::new(MAX_VOXEL_COORD, -MAX_VOXEL_COORD, 0, 1);
        assert_eq!(mesh_voxels(&[edge], &palette(), 1.0).map(|m| m.face_count()), Ok(6));
        let beyond = Voxel::new(i32::MAX, 0, 0, 1);
        assert_eq!(mesh_voxels(&[beyond], &palette(), 1.0), Err(VoxelError::OutOfRange { voxel: beyond }));
        assert!(VoxelError::OutOfRange { voxel: beyond }.to_string().contains("2147483647"));
    }

    #[test]
    fn palette_validation_and_clamping() {
        assert_eq!(VoxelPalette::new(vec![]), Err(VoxelError::EmptyPalette));
        let p = palette();
        assert_eq!(p.color(200), p.color(4));
        assert!(p.is_translucent(3));
        assert!(!p.is_translucent(1));
    }
}
