//! Storage-independent access to a voxel world (dense or sparse).

use std::collections::BTreeSet;

use crate::dense::DenseGrid;
use crate::error::VoxelError;
use crate::geometry::{Bounds, Pos, WorldKind};
use crate::rule::{Neighbourhood, Transition};
use crate::sparse::{ChunkKey, SparseGrid};
use crate::stats::Stats;

/// Most chunks a sparse world may evaluate in one generation, and most
/// chunks an edit or initial state may store (about 12 KiB each, so roughly
/// 400 MiB of chunks).
pub const MAX_SPARSE_CHUNKS: usize = 32_768;

/// Error for an edit or initial state that would store too many chunks.
pub(crate) fn too_many_chunks() -> VoxelError {
    VoxelError::Limit(format!(
        "the sparse world would store more than {MAX_SPARSE_CHUNKS} chunks of 16³ voxels (about 400 MiB); \
         use a smaller or more compact region"
    ))
}

/// Voxel storage of a world.
#[derive(Clone, Debug)]
pub(crate) enum Storage {
    Dense(DenseGrid),
    Sparse(SparseGrid),
}

impl Storage {
    /// Empty storage for a validated world kind and size.
    pub(crate) fn new(kind: WorldKind, size: [u32; 3]) -> Self {
        match kind {
            WorldKind::DenseBounded => Self::Dense(DenseGrid::new(size, false)),
            WorldKind::DenseTorus => Self::Dense(DenseGrid::new(size, true)),
            WorldKind::SparseBounded | WorldKind::SparseUnbounded => Self::Sparse(SparseGrid::new(kind.bounds(size))),
        }
    }

    /// State and age at `pos` (empty outside the world).
    pub(crate) fn get(&self, pos: Pos) -> (u8, u16) {
        match self {
            Self::Dense(grid) => grid.get(pos),
            Self::Sparse(grid) => grid.get(pos),
        }
    }

    /// State at `pos` (0 outside the world).
    #[inline]
    pub(crate) fn state(&self, pos: Pos) -> u8 {
        match self {
            Self::Dense(grid) => grid.state(pos),
            Self::Sparse(grid) => grid.state(pos),
        }
    }

    /// Writes a voxel (ignored outside the world) and resets its age.
    pub(crate) fn set(&mut self, pos: Pos, state: u8) {
        match self {
            Self::Dense(grid) => grid.set(pos, state),
            Self::Sparse(grid) => grid.set(pos, state),
        }
    }

    /// Empties the world.
    pub(crate) fn clear(&mut self) {
        match self {
            Self::Dense(grid) => grid.clear(),
            Self::Sparse(grid) => grid.clear(),
        }
    }

    /// Fails when writing non-empty voxels at `positions` would make a sparse
    /// world store more than [`MAX_SPARSE_CHUNKS`] chunks. Dense worlds have
    /// a fixed size and always pass.
    pub(crate) fn check_new_voxels(&self, positions: impl IntoIterator<Item = Pos>) -> Result<(), VoxelError> {
        let Self::Sparse(grid) = self else { return Ok(()) };
        let mut added = BTreeSet::new();
        for pos in positions {
            let key = ChunkKey::of(pos).0;
            if grid.bounds().contains(pos)
                && grid.chunk(key).is_none()
                && added.insert(key)
                && grid.chunk_count() + added.len() > MAX_SPARSE_CHUNKS
            {
                return Err(too_many_chunks());
            }
        }
        Ok(())
    }

    /// Like [`Storage::check_new_voxels`] for every voxel of `area` (which
    /// must lie inside the world).
    pub(crate) fn check_new_box(&self, area: Bounds) -> Result<(), VoxelError> {
        let Self::Sparse(grid) = self else { return Ok(()) };
        let span = ChunkKey::span(&area);
        let covered: u64 = span.iter().map(|&(lo, hi)| (i64::from(hi) - i64::from(lo) + 1) as u64).product();
        let stored = grid.chunk_count() as u64;
        if stored + covered <= MAX_SPARSE_CHUNKS as u64 {
            return Ok(());
        }
        let mut total = stored;
        for z in span[2].0..=span[2].1 {
            for y in span[1].0..=span[1].1 {
                for x in span[0].0..=span[0].1 {
                    total += u64::from(grid.chunk(ChunkKey([x, y, z])).is_none());
                    if total > MAX_SPARSE_CHUNKS as u64 {
                        return Err(too_many_chunks());
                    }
                }
            }
        }
        Ok(())
    }

    /// Empties voxels in states that a rule with `states` states lacks.
    pub(crate) fn clamp_states(&mut self, states: u16) {
        match self {
            Self::Dense(grid) => grid.clamp_states(states),
            Self::Sparse(grid) => grid.clamp_states(states),
        }
    }

    /// Advances one generation. A sparse world that would have to evaluate
    /// more than [`MAX_SPARSE_CHUNKS`] chunks is left unchanged and an error
    /// is returned.
    pub(crate) fn step(
        &mut self,
        transition: &Transition,
        neighbourhood: Neighbourhood,
        states: u16,
    ) -> Result<Stats, VoxelError> {
        match self {
            Self::Dense(grid) => Ok(grid.step(transition, neighbourhood, states)),
            Self::Sparse(grid) => {
                let candidates = grid.candidates();
                if candidates.len() > MAX_SPARSE_CHUNKS {
                    return Err(VoxelError::Limit(format!(
                        "the next generation would evaluate {} chunks of 16³ voxels, more than the limit of \
                         {MAX_SPARSE_CHUNKS}; the pattern has grown too large for sparse storage",
                        candidates.len()
                    )));
                }
                Ok(grid.step(&candidates, transition, neighbourhood, states))
            }
        }
    }

    /// Statistics of the current state (transition counters are zero).
    pub(crate) fn stats(&self, states: u16) -> Stats {
        match self {
            Self::Dense(grid) => grid.stats(states),
            Self::Sparse(grid) => {
                let parts = grid.chunks().map(|(key, chunk)| {
                    let mut stats = Stats::new(states);
                    for (i, &state) in chunk.cells.iter().enumerate() {
                        if state != 0 {
                            stats.add(key.position(i), state);
                        }
                    }
                    stats
                });
                Stats::combine(states, parts)
            }
        }
    }

    /// Every non-empty voxel as `(position, state, age)`, sorted by z, then
    /// y, then x.
    pub(crate) fn voxels(&self) -> Vec<(Pos, u8, u16)> {
        let mut out = Vec::new();
        match self {
            Self::Dense(grid) => {
                let [nx, ny, _] = grid.size();
                for (i, (&state, &age)) in grid.cells().iter().zip(grid.ages()).enumerate() {
                    if state != 0 {
                        let pos = [(i % nx) as i32, ((i / nx) % ny) as i32, (i / (nx * ny)) as i32];
                        out.push((pos, state, age));
                    }
                }
            }
            Self::Sparse(grid) => {
                for (key, chunk) in grid.chunks() {
                    for (i, (&state, &age)) in chunk.cells.iter().zip(chunk.ages.iter()).enumerate() {
                        if state != 0 {
                            out.push((key.position(i), state, age));
                        }
                    }
                }
                out.sort_unstable_by_key(|&([x, y, z], _, _)| (z, y, x));
            }
        }
        out
    }

    /// Approximate memory held by the storage.
    pub(crate) fn memory_bytes(&self) -> usize {
        match self {
            Self::Dense(grid) => grid.memory_bytes(),
            Self::Sparse(grid) => grid.memory_bytes(),
        }
    }

    /// Box of coordinates that can hold voxels.
    pub(crate) fn bounds(&self) -> Bounds {
        match self {
            Self::Dense(grid) => {
                let size = grid.size().map(|s| s as u32);
                Bounds::from_size(size)
            }
            Self::Sparse(grid) => grid.bounds(),
        }
    }
}
