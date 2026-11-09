//! Render data: visible-surface voxel lists, 2D slices and state counts.
//!
//! **Surface rule.** For a [`SliceRange`] along an axis, a non-empty voxel
//! whose coordinate on that axis lies in `[min, max]` is emitted iff at least
//! one of its six faces is exposed. A face is exposed when the voxel across
//! it is empty, lies outside the world box (also on a torus: the renderer
//! shows the box, not its periodic images), or is sliced away (its coordinate
//! along the range axis is outside `[min, max]`). Fully enclosed voxels are
//! omitted. Voxels are listed in a deterministic order (z, y, x for dense
//! worlds; chunk by chunk for sparse ones).

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simulation_core::Grid2;

use crate::geometry::{Axis, Bounds, Pos};
use crate::sparse::{CHUNK_SIDE, Chunk};
use crate::world::Storage;

/// Default cap on the number of voxels of a surface view.
pub const DEFAULT_MAX_SURFACE_VOXELS: usize = 2_000_000;

/// Largest side of a slice view, in voxels.
pub const MAX_SLICE_SIDE: u32 = 4096;

/// Face bits of [`SurfaceVoxel::faces`].
pub mod faces {
    /// Face towards −x.
    pub const NEG_X: u8 = 1;
    /// Face towards +x.
    pub const POS_X: u8 = 2;
    /// Face towards −y.
    pub const NEG_Y: u8 = 4;
    /// Face towards +y.
    pub const POS_Y: u8 = 8;
    /// Face towards −z.
    pub const NEG_Z: u8 = 16;
    /// Face towards +z.
    pub const POS_Z: u8 = 32;
    /// All six faces.
    pub const ALL: u8 = 63;
}

/// Face directions in the order of the [`faces`] bits.
const FACE_OFFSETS: [[i32; 3]; 6] = [[-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1]];

/// The layers `[min, max]` (inclusive) along an axis that remain visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SliceRange {
    /// Axis the range applies to.
    pub axis: Axis,
    /// First visible layer.
    pub min: i32,
    /// Last visible layer.
    pub max: i32,
}

impl SliceRange {
    /// Every layer (nothing is sliced away).
    pub fn all() -> Self {
        Self { axis: Axis::Z, min: i32::MIN, max: i32::MAX }
    }

    /// Layers `min..=max` along `axis`.
    pub fn new(axis: Axis, min: i32, max: i32) -> Self {
        Self { axis, min, max }
    }

    /// Whether `pos` is inside the range.
    pub fn contains(&self, pos: Pos) -> bool {
        (self.min..=self.max).contains(&pos[self.axis.index()])
    }
}

impl Default for SliceRange {
    fn default() -> Self {
        Self::all()
    }
}

/// A rectangle of a slice plane: `u` and `v` are the in-plane axes given by
/// [`Axis::plane_axes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SliceWindow {
    /// First `u` coordinate.
    pub u0: i32,
    /// First `v` coordinate.
    pub v0: i32,
    /// Width along `u` (clamped to `1..=MAX_SLICE_SIDE`).
    pub width: u32,
    /// Height along `v` (clamped to `1..=MAX_SLICE_SIDE`).
    pub height: u32,
}

/// What a renderer asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoxelViewRequest {
    /// Visible voxels within a slice range.
    Surface {
        /// Visible layers.
        range: SliceRange,
        /// Cap on the number of listed voxels.
        max_voxels: usize,
    },
    /// One layer as a 2D grid of states.
    Slice {
        /// Axis perpendicular to the layer.
        axis: Axis,
        /// Coordinate of the layer along `axis`.
        layer: i32,
        /// Region of the layer; `None` picks the world face (dense) or the
        /// initialisation box joined with the population's bounding box
        /// (sparse).
        window: Option<SliceWindow>,
    },
    /// Number of voxels per state.
    Counts,
}

impl Default for VoxelViewRequest {
    fn default() -> Self {
        Self::Surface { range: SliceRange::all(), max_voxels: DEFAULT_MAX_SURFACE_VOXELS }
    }
}

/// A visible voxel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceVoxel {
    /// Position.
    pub pos: Pos,
    /// State (1 = alive, 2… = dying).
    pub state: u8,
    /// Generations since the voxel was born or placed.
    pub age: u16,
    /// Exposed faces (see [`faces`]).
    pub faces: u8,
}

/// Visible voxels of a world.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceView {
    /// Generation shown.
    pub tick: u64,
    /// Number of states of the rule.
    pub states: u16,
    /// Box of coordinates that can hold voxels.
    pub world: Bounds,
    /// Range that was requested.
    pub range: SliceRange,
    /// Listed voxels (at most the requested cap).
    pub voxels: Vec<SurfaceVoxel>,
    /// Number of visible voxels before applying the cap.
    pub visible: u64,
    /// Whether the cap cut the list.
    pub truncated: bool,
}

/// One layer of the world.
#[derive(Clone, Debug, PartialEq)]
pub struct SliceView {
    /// Generation shown.
    pub tick: u64,
    /// Axis perpendicular to the layer.
    pub axis: Axis,
    /// Layer coordinate.
    pub layer: i32,
    /// Region shown; cell `(i, j)` of `cells` is at `u = u0 + i`, `v = v0 + j`.
    pub window: SliceWindow,
    /// States, row-major over `(u, v)`.
    pub cells: Grid2<u8>,
    /// Whether the requested or automatic region was larger than
    /// [`MAX_SLICE_SIDE`] and was cut.
    pub truncated: bool,
}

/// Number of voxels per state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountsView {
    /// Generation shown.
    pub tick: u64,
    /// `counts[s]` = voxels in state `s`; `counts[0]` = empty voxels of a
    /// finite world (0 for the unbounded world).
    pub counts: Vec<u64>,
    /// Empty voxels of a finite world, `None` for the unbounded world.
    pub empty: Option<u64>,
    /// Non-empty voxels.
    pub population: u64,
}

/// Render data produced for a [`VoxelViewRequest`].
#[derive(Clone, Debug, PartialEq)]
pub enum VoxelView {
    /// See [`SurfaceView`].
    Surface(SurfaceView),
    /// See [`SliceView`].
    Slice(SliceView),
    /// See [`CountsView`].
    Counts(CountsView),
}

/// Exposed-face mask (0 = enclosed) of the non-empty voxel at `pos` whose
/// six face neighbours, in [`FACE_OFFSETS`] order, are `occupied` (false
/// for neighbours outside the world box).
#[inline]
fn face_mask(pos: Pos, range: &SliceRange, occupied: [bool; 6]) -> u8 {
    let axis = range.axis.index();
    let mut mask = 0;
    for (bit, d) in FACE_OFFSETS.iter().enumerate() {
        let n = i64::from(pos[axis]) + i64::from(d[axis]);
        if !occupied[bit] || n < i64::from(range.min) || n > i64::from(range.max) {
            mask |= 1 << bit;
        }
    }
    mask
}

/// Visible voxels in `range` (see the module documentation), with the total
/// number visible.
///
/// Neighbours are read by index arithmetic: within the dense layout, and
/// within a sparse chunk plus its six face-adjacent chunks (looked up once
/// per chunk), so no hash lookup is made per voxel.
pub(crate) fn surface(storage: &Storage, range: &SliceRange, max_voxels: usize) -> (Vec<SurfaceVoxel>, u64) {
    let parts: Vec<Vec<SurfaceVoxel>> = match storage {
        Storage::Dense(grid) => {
            let [nx, ny, nz] = grid.size();
            let plane = nx * ny;
            let (cells, ages) = (grid.cells(), grid.ages());
            (0..nz)
                .into_par_iter()
                .map(|z| {
                    let mut out = Vec::new();
                    if range.axis == Axis::Z && !(range.min..=range.max).contains(&(z as i32)) {
                        return out;
                    }
                    for y in 0..ny {
                        for x in 0..nx {
                            let i = z * plane + y * nx + x;
                            let pos = [x as i32, y as i32, z as i32];
                            if cells[i] == 0 || !range.contains(pos) {
                                continue;
                            }
                            let occupied = [
                                x > 0 && cells[i - 1] != 0,
                                x + 1 < nx && cells[i + 1] != 0,
                                y > 0 && cells[i - nx] != 0,
                                y + 1 < ny && cells[i + nx] != 0,
                                z > 0 && cells[i - plane] != 0,
                                z + 1 < nz && cells[i + plane] != 0,
                            ];
                            let faces = face_mask(pos, range, occupied);
                            if faces != 0 {
                                out.push(SurfaceVoxel { pos, state: cells[i], age: ages[i], faces });
                            }
                        }
                    }
                    out
                })
                .collect()
        }
        Storage::Sparse(grid) => {
            const SIDE: usize = CHUNK_SIDE as usize;
            const ROW: usize = SIDE;
            const PLANE: usize = SIDE * SIDE;
            let chunks: Vec<_> = grid.chunks().collect();
            chunks
                .par_iter()
                .map(|&(key, chunk)| {
                    let around = grid.face_neighbours(key);
                    let other = |c: Option<&Chunk>, j: usize| c.is_some_and(|c| c.cells[j] != 0);
                    let own = &chunk.cells;
                    let mut out = Vec::new();
                    for (i, (&state, &age)) in own.iter().zip(chunk.ages.iter()).enumerate() {
                        let pos = key.position(i);
                        if state == 0 || !range.contains(pos) {
                            continue;
                        }
                        let (x, y, z) = (i % SIDE, (i / ROW) % SIDE, i / PLANE);
                        let occupied = [
                            if x > 0 { own[i - 1] != 0 } else { other(around[0], i + SIDE - 1) },
                            if x + 1 < SIDE { own[i + 1] != 0 } else { other(around[1], i - (SIDE - 1)) },
                            if y > 0 { own[i - ROW] != 0 } else { other(around[2], i + (SIDE - 1) * ROW) },
                            if y + 1 < SIDE { own[i + ROW] != 0 } else { other(around[3], i - (SIDE - 1) * ROW) },
                            if z > 0 { own[i - PLANE] != 0 } else { other(around[4], i + (SIDE - 1) * PLANE) },
                            if z + 1 < SIDE { own[i + PLANE] != 0 } else { other(around[5], i - (SIDE - 1) * PLANE) },
                        ];
                        let faces = face_mask(pos, range, occupied);
                        if faces != 0 {
                            out.push(SurfaceVoxel { pos, state, age, faces });
                        }
                    }
                    out
                })
                .collect()
        }
    };
    let visible = parts.iter().map(|p| p.len() as u64).sum();
    let mut voxels = Vec::with_capacity(max_voxels.min(visible as usize));
    for part in parts {
        let room = max_voxels - voxels.len();
        voxels.extend(part.into_iter().take(room));
        if voxels.len() == max_voxels {
            break;
        }
    }
    (voxels, visible)
}

/// Resolves the region of a slice; returns the window and whether it was cut.
pub(crate) fn slice_window(
    axis: Axis,
    requested: Option<SliceWindow>,
    world: Bounds,
    automatic: Bounds,
) -> (SliceWindow, bool) {
    let (u, v) = axis.plane_axes();
    let wanted = requested.unwrap_or_else(|| {
        let area = automatic.intersection(&world).unwrap_or(automatic);
        let extent = area.extent();
        SliceWindow {
            u0: area.min[u.index()],
            v0: area.min[v.index()],
            width: u32::try_from(extent[u.index()]).unwrap_or(u32::MAX),
            height: u32::try_from(extent[v.index()]).unwrap_or(u32::MAX),
        }
    });
    let width = wanted.width.clamp(1, MAX_SLICE_SIDE);
    let height = wanted.height.clamp(1, MAX_SLICE_SIDE);
    let truncated = width != wanted.width || height != wanted.height;
    (SliceWindow { width, height, ..wanted }, truncated)
}

/// States of one layer over a window.
///
/// Dense worlds are read voxel by voxel; sparse worlds copy only the stored
/// chunks that cross the layer, so the cost does not depend on hash lookups
/// per pixel.
pub(crate) fn slice_cells(storage: &Storage, axis: Axis, layer: i32, window: SliceWindow) -> Grid2<u8> {
    let (u, v) = (axis.plane_axes().0.index(), axis.plane_axes().1.index());
    let (width, height) = (window.width as usize, window.height as usize);
    // Grid cell of a position in the layer, if it falls inside the window.
    let cell = |pos: Pos| {
        let i = i64::from(pos[u]) - i64::from(window.u0);
        let j = i64::from(pos[v]) - i64::from(window.v0);
        ((0..width as i64).contains(&i) && (0..height as i64).contains(&j)).then_some((i as usize, j as usize))
    };
    match storage {
        Storage::Dense(_) => Grid2::from_fn(width, height, |i, j| {
            let cu = i32::try_from(i64::from(window.u0) + i as i64);
            let cv = i32::try_from(i64::from(window.v0) + j as i64);
            match (cu, cv) {
                (Ok(cu), Ok(cv)) => {
                    let mut pos = [layer; 3];
                    pos[u] = cu;
                    pos[v] = cv;
                    storage.state(pos)
                }
                _ => 0,
            }
        }),
        Storage::Sparse(grid) => {
            let mut out = Grid2::new(width, height, 0u8);
            let a = axis.index();
            for (key, chunk) in grid.chunks() {
                let b = key.bounds();
                if !(b.min[a]..=b.max[a]).contains(&layer) {
                    continue;
                }
                for (i, &state) in chunk.cells.iter().enumerate() {
                    let pos = key.position(i);
                    if state != 0
                        && pos[a] == layer
                        && let Some((x, y)) = cell(pos)
                    {
                        out.set(x, y, state);
                    }
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::WorldKind;

    fn solid_cube(kind: WorldKind, side: i32) -> Storage {
        let mut storage = Storage::new(kind, [8, 8, 8]);
        for z in 1..=side {
            for y in 1..=side {
                for x in 1..=side {
                    storage.set([x, y, z], 1);
                }
            }
        }
        storage
    }

    #[test]
    fn interior_voxels_are_omitted() {
        for kind in [WorldKind::DenseBounded, WorldKind::SparseUnbounded] {
            let storage = solid_cube(kind, 5);
            let (voxels, visible) = surface(&storage, &SliceRange::all(), usize::MAX);
            assert_eq!(visible, 125 - 27, "{kind:?}: a 5³ cube shows its 98 shell voxels");
            assert_eq!(voxels.len(), 98);
            assert!(voxels.iter().all(|v| v.pos.iter().any(|&c| c == 1 || c == 5)), "only shell voxels");
            let corner = voxels.iter().find(|v| v.pos == [1, 1, 1]).map(|v| v.faces);
            assert_eq!(corner, Some(faces::NEG_X | faces::NEG_Y | faces::NEG_Z));
            let face_centre = voxels.iter().find(|v| v.pos == [3, 3, 5]).map(|v| v.faces);
            assert_eq!(face_centre, Some(faces::POS_Z));
        }
    }

    #[test]
    fn slicing_exposes_the_cut_layer() {
        let storage = solid_cube(WorldKind::DenseBounded, 5);
        let range = SliceRange::new(Axis::Z, 1, 3);
        let (voxels, visible) = surface(&storage, &range, usize::MAX);
        // Layers 1..=3: the whole cut layer z = 3 (25) plus the side walls
        // of z = 1 (25, bottom face) and z = 2 (16 border voxels).
        assert_eq!(visible, 25 + 25 + 16);
        let centre = voxels.iter().find(|v| v.pos == [3, 3, 3]).map(|v| v.faces);
        assert_eq!(centre, Some(faces::POS_Z), "the voxel below the cut shows its top face");
        assert!(voxels.iter().all(|v| (1..=3).contains(&v.pos[2])));
        let (capped, total) = surface(&storage, &range, 10);
        assert_eq!((capped.len(), total), (10, visible));
    }

    #[test]
    fn lone_voxel_and_world_edges() {
        let mut storage = Storage::new(WorldKind::DenseTorus, [3, 3, 3]);
        storage.set([0, 0, 0], 2);
        storage.set([1, 0, 0], 1);
        let (voxels, _) = surface(&storage, &SliceRange::all(), usize::MAX);
        assert_eq!(voxels.len(), 2);
        assert_eq!(voxels[0].faces, faces::ALL & !faces::POS_X, "torus edges count as exposed");
        assert_eq!(voxels[0].state, 2);
    }

    #[test]
    fn slices_and_windows() {
        let storage = solid_cube(WorldKind::DenseBounded, 2);
        let world = storage.bounds();
        let (window, cut) = slice_window(Axis::X, None, world, world);
        assert_eq!((window.width, window.height, cut), (8, 8, false));
        let cells = slice_cells(&storage, Axis::X, 1, window);
        // Axis x: u = y, v = z.
        assert_eq!(*cells.get(1, 2), 1);
        assert_eq!(*cells.get(3, 1), 0);
        assert_eq!(cells.as_slice().iter().filter(|&&c| c == 1).count(), 4);
        let huge = SliceWindow { u0: i32::MAX - 1, v0: 0, width: 10_000, height: 1 };
        let (window, cut) = slice_window(Axis::Z, Some(huge), world, world);
        assert!(cut && window.width == MAX_SLICE_SIDE);
        assert!(slice_cells(&storage, Axis::Z, 1, window).as_slice().iter().all(|&c| c == 0));
    }

    #[test]
    fn sparse_and_dense_slices_agree() {
        let mut dense = Storage::new(WorldKind::DenseBounded, [40, 40, 40]);
        let mut sparse = Storage::new(WorldKind::SparseBounded, [40, 40, 40]);
        let mut rng = simulation_core::SimRng::seed_from_u64(4);
        for z in 0..40 {
            for y in 0..40 {
                for x in 0..40 {
                    if rng.chance(0.2) {
                        let state = 1 + rng.below(3) as u8;
                        dense.set([x, y, z], state);
                        sparse.set([x, y, z], state);
                    }
                }
            }
        }
        let full = SliceWindow { u0: 0, v0: 0, width: 40, height: 40 };
        let offset = SliceWindow { u0: -5, v0: 10, width: 30, height: 33 };
        for axis in Axis::ALL {
            for layer in [0, 15, 16, 39, 40] {
                for window in [full, offset] {
                    let a = slice_cells(&dense, axis, layer, window);
                    assert_eq!(a, slice_cells(&sparse, axis, layer, window), "{axis:?} layer {layer}");
                }
            }
        }
    }
}
