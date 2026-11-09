//! Chunked voxel meshing: splits a volume into cubic chunks and rebuilds only
//! the chunks whose visible faces can have changed.
//!
//! A chunk's mesh depends on its own voxels and on the face-neighbours of its
//! border voxels that lie in other chunks. Both are hashed per chunk (64-bit
//! SipHash with fixed keys, so runs are reproducible); a chunk is re-meshed
//! when that hash changes, so editing a border voxel correctly re-meshes the
//! neighbouring chunk too. A hash collision (probability about 2^-64 per
//! changed chunk) would leave that chunk's previous mesh in place until its
//! next change; [`ChunkedVoxelMesher::invalidate`] forces a full rebuild. Changing the palette or voxel size
//! rebuilds everything. Chunk meshes are positioned relative to the chunk's
//! origin (`chunk × chunk_size × voxel_size`) to keep vertex coordinates small.

use bevy::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

use super::mesher::{
    FACES, Voxel, VoxelError, VoxelMeshes, VoxelPalette, check_voxels, mesh_sorted, occupancy, sorted_voxels,
};

/// Ordered map key for a chunk coordinate (`IVec3` is not `Ord`).
pub(crate) type ChunkKey = (i32, i32, i32);

/// Converts a chunk coordinate to its ordered key.
pub(crate) fn chunk_key(chunk: IVec3) -> ChunkKey {
    (chunk.x, chunk.y, chunk.z)
}

fn key_chunk(key: ChunkKey) -> IVec3 {
    IVec3::new(key.0, key.1, key.2)
}

/// One re-meshed chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkUpdate {
    /// Chunk coordinate.
    pub chunk: IVec3,
    /// Chunk origin in voxels (`chunk × chunk_size`).
    pub origin: IVec3,
    /// New meshes (positions relative to the origin).
    pub meshes: VoxelMeshes,
}

/// Result of [`ChunkedVoxelMesher::rebuild`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChunkDiff {
    /// Chunks that are new or whose mesh changed, sorted by chunk coordinate.
    pub updated: Vec<ChunkUpdate>,
    /// Chunks that no longer contain voxels, sorted.
    pub removed: Vec<IVec3>,
}

/// Incremental chunked mesher.
#[derive(Clone, Debug)]
pub struct ChunkedVoxelMesher {
    chunk_size: i32,
    hashes: BTreeMap<ChunkKey, u64>,
    palette: Option<VoxelPalette>,
    voxel_size: f32,
}

impl ChunkedVoxelMesher {
    /// Creates a mesher with cubic chunks of `chunk_size` voxels (1..=256).
    pub fn new(chunk_size: u32) -> Result<Self, VoxelError> {
        if !(1..=256).contains(&chunk_size) {
            return Err(VoxelError::ChunkSize(chunk_size));
        }
        Ok(Self { chunk_size: chunk_size as i32, hashes: BTreeMap::new(), palette: None, voxel_size: 0.0 })
    }

    /// Chunk edge length in voxels.
    pub fn chunk_size(&self) -> i32 {
        self.chunk_size
    }

    /// Chunk containing a voxel position.
    pub fn chunk_of(&self, pos: IVec3) -> IVec3 {
        IVec3::new(
            pos.x.div_euclid(self.chunk_size),
            pos.y.div_euclid(self.chunk_size),
            pos.z.div_euclid(self.chunk_size),
        )
    }

    /// Number of non-empty chunks after the last rebuild.
    pub fn chunk_count(&self) -> usize {
        self.hashes.len()
    }

    /// Forgets all chunk state so the next rebuild re-meshes everything.
    pub fn invalidate(&mut self) {
        self.hashes.clear();
    }

    /// Re-meshes the chunks affected by the new voxel set and reports
    /// removals. Fails (changing nothing) if a coordinate is out of range.
    pub fn rebuild(
        &mut self,
        voxels: &[Voxel],
        palette: &VoxelPalette,
        voxel_size: f32,
    ) -> Result<ChunkDiff, VoxelError> {
        check_voxels(voxels)?;
        if self.palette.as_ref() != Some(palette) || self.voxel_size != voxel_size {
            self.hashes.clear();
            self.palette = Some(palette.clone());
            self.voxel_size = voxel_size;
        }
        let lookup = occupancy(voxels);
        let mut by_chunk: BTreeMap<ChunkKey, Vec<Voxel>> = BTreeMap::new();
        for voxel in sorted_voxels(&lookup) {
            by_chunk.entry(chunk_key(self.chunk_of(voxel.pos()))).or_default().push(voxel);
        }
        let mut diff = ChunkDiff::default();
        let mut next_hashes = BTreeMap::new();
        for (key, chunk_voxels) in &by_chunk {
            let chunk = key_chunk(*key);
            let hash = self.chunk_hash(chunk, chunk_voxels, &lookup);
            next_hashes.insert(*key, hash);
            if self.hashes.get(key) == Some(&hash) {
                continue;
            }
            let origin = chunk * self.chunk_size;
            let meshes = mesh_sorted(chunk_voxels, &lookup, palette, voxel_size, origin);
            diff.updated.push(ChunkUpdate { chunk, origin, meshes });
        }
        diff.removed = self.hashes.keys().filter(|k| !next_hashes.contains_key(*k)).map(|k| key_chunk(*k)).collect();
        self.hashes = next_hashes;
        Ok(diff)
    }

    /// Hash of everything that determines a chunk's mesh: its voxels and the
    /// states of face-neighbours outside the chunk.
    fn chunk_hash(&self, chunk: IVec3, voxels: &[Voxel], lookup: &HashMap<IVec3, u8>) -> u64 {
        let mut hasher = DefaultHasher::new();
        for voxel in voxels {
            voxel.hash(&mut hasher);
            for face in &FACES {
                let n = voxel.pos() + face.normal;
                if self.chunk_of(n) != chunk {
                    lookup.get(&n).copied().unwrap_or(0).hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::mesher::mesh_voxels;

    fn palette() -> VoxelPalette {
        VoxelPalette::new(vec![Color::NONE, Color::WHITE, Color::srgb(0.2, 0.4, 0.9), Color::srgba(1.0, 1.0, 1.0, 0.3)])
            .expect("non-empty")
    }

    /// Deterministic pseudo-random blob (xorshift).
    fn blob(n: usize, seed: u64) -> Vec<Voxel> {
        let mut s = seed.max(1);
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        (0..n)
            .map(|_| {
                let r = next();
                Voxel::new(
                    (r % 23) as i32 - 11,
                    ((r >> 8) % 19) as i32 - 9,
                    ((r >> 16) % 17) as i32 - 8,
                    (1 + (r >> 24) % 3) as u8,
                )
            })
            .collect()
    }

    #[test]
    fn rejects_bad_chunk_size() {
        assert_eq!(ChunkedVoxelMesher::new(0).err(), Some(VoxelError::ChunkSize(0)));
        assert_eq!(ChunkedVoxelMesher::new(300).err(), Some(VoxelError::ChunkSize(300)));
    }

    #[test]
    fn out_of_range_rebuild_changes_nothing() {
        let mut mesher = ChunkedVoxelMesher::new(3).expect("valid");
        mesher.rebuild(&[Voxel::new(0, 0, 0, 1)], &palette(), 1.0).expect("in range");
        // i32::MIN is not a multiple of 3: its chunk origin would overflow.
        let bad = Voxel::new(i32::MIN, 0, 0, 1);
        assert_eq!(mesher.rebuild(&[bad], &palette(), 1.0), Err(VoxelError::OutOfRange { voxel: bad }));
        assert_eq!(mesher.chunk_count(), 1);
    }

    #[test]
    fn chunk_of_handles_negative_coordinates() {
        let m = ChunkedVoxelMesher::new(8).expect("valid");
        assert_eq!(m.chunk_of(IVec3::new(0, 7, 8)), IVec3::new(0, 0, 1));
        assert_eq!(m.chunk_of(IVec3::new(-1, -8, -9)), IVec3::new(-1, -1, -2));
    }

    #[test]
    fn chunked_faces_equal_unchunked_faces() {
        let voxels = blob(1500, 7);
        let p = palette();
        let whole = mesh_voxels(&voxels, &p, 1.0).expect("in range");
        for size in [1, 4, 5, 16] {
            let mut mesher = ChunkedVoxelMesher::new(size).expect("valid");
            let diff = mesher.rebuild(&voxels, &p, 1.0).expect("in range");
            let opaque: usize = diff.updated.iter().map(|u| u.meshes.opaque.face_count()).sum();
            let translucent: usize = diff.updated.iter().map(|u| u.meshes.translucent.face_count()).sum();
            assert_eq!(opaque, whole.opaque.face_count(), "chunk size {size}");
            assert_eq!(translucent, whole.translucent.face_count(), "chunk size {size}");
            assert!(diff.removed.is_empty());
        }
    }

    #[test]
    fn unchanged_volume_rebuilds_nothing() {
        let voxels = blob(400, 3);
        let mut mesher = ChunkedVoxelMesher::new(8).expect("valid");
        let first = mesher.rebuild(&voxels, &palette(), 1.0).expect("in range");
        assert_eq!(first.updated.len(), mesher.chunk_count());
        let second = mesher.rebuild(&voxels, &palette(), 1.0).expect("in range");
        assert!(second.updated.is_empty() && second.removed.is_empty());
        // A palette change forces a full rebuild.
        let other = VoxelPalette::new(vec![Color::NONE, Color::BLACK]).expect("non-empty");
        assert_eq!(mesher.rebuild(&voxels, &other, 1.0).expect("in range").updated.len(), mesher.chunk_count());
    }

    #[test]
    fn interior_edit_rebuilds_one_chunk() {
        let mut voxels: Vec<Voxel> = (0..16).map(|x| Voxel::new(x, 0, 0, 1)).collect();
        let mut mesher = ChunkedVoxelMesher::new(4).expect("valid");
        assert_eq!(mesher.rebuild(&voxels, &palette(), 1.0).expect("in range").updated.len(), 4);
        voxels[5].state = 2; // x = 5: interior of chunk 1 (x 4..8), not on its border
        let diff = mesher.rebuild(&voxels, &palette(), 1.0).expect("in range");
        assert_eq!(diff.updated.iter().map(|u| u.chunk).collect::<Vec<_>>(), vec![IVec3::new(1, 0, 0)]);
    }

    #[test]
    fn border_edit_rebuilds_neighbour_chunk() {
        let mut voxels: Vec<Voxel> = (0..8).map(|x| Voxel::new(x, 0, 0, 1)).collect();
        let mut mesher = ChunkedVoxelMesher::new(4).expect("valid");
        mesher.rebuild(&voxels, &palette(), 1.0).expect("in range");
        // Removing x = 4 (first voxel of chunk 1) exposes a face of x = 3 in chunk 0.
        voxels.retain(|v| v.x != 4);
        let diff = mesher.rebuild(&voxels, &palette(), 1.0).expect("in range");
        let chunks: Vec<IVec3> = diff.updated.iter().map(|u| u.chunk).collect();
        assert_eq!(chunks, vec![IVec3::new(0, 0, 0), IVec3::new(1, 0, 0)]);
        let chunk0 = &diff.updated[0].meshes;
        assert_eq!(chunk0.face_count(), 4 * 4 + 2, "row of 4 voxels with both ends exposed");
    }

    #[test]
    fn emptied_chunks_are_removed_and_positions_are_local() {
        let mut mesher = ChunkedVoxelMesher::new(4).expect("valid");
        let diff =
            mesher.rebuild(&[Voxel::new(9, 1, 2, 1), Voxel::new(-1, 0, 0, 1)], &palette(), 2.0).expect("in range");
        let update = diff.updated.iter().find(|u| u.chunk == IVec3::new(2, 0, 0)).expect("chunk (2,0,0)");
        assert_eq!(update.origin, IVec3::new(8, 0, 0));
        let min = update.meshes.opaque.positions.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(Vec3::from_array(*p)));
        assert_eq!(min, Vec3::new(2.0, 2.0, 4.0), "local voxel (1,1,2) × size 2");
        let diff = mesher.rebuild(&[Voxel::new(-1, 0, 0, 1)], &palette(), 2.0).expect("in range");
        assert_eq!(diff.removed, vec![IVec3::new(2, 0, 0)]);
        assert!(diff.updated.is_empty());
    }
}
