//! Sparse voxel storage: a hash map of 16³ chunks clipped to a box.
//!
//! Only chunks holding at least one non-empty voxel are stored. A generation
//! evaluates every stored chunk plus, for chunks holding alive voxels, their
//! 26 neighbouring chunks (births need an alive neighbour because B0 is
//! rejected, so no other chunk can change). Candidate chunks are sorted
//! (z, y, x) and computed in parallel, each as a pure function of the
//! read-only current map: an 18³ padded block of alive bits is gathered from
//! the chunk and its neighbours, counted separably (Moore) or directly (von
//! Neumann), and the rule is applied. The new map is built from the results
//! in sorted order, so nothing depends on hash-map iteration order.

use std::cmp::Ordering;
use std::collections::HashMap;

use rayon::prelude::*;

use crate::geometry::{Bounds, Pos};
use crate::rule::{Neighbourhood, Transition};
use crate::stats::Stats;

/// Chunk side in voxels.
pub const CHUNK_SIDE: i32 = 16;
const SIDE: usize = CHUNK_SIDE as usize;
const VOLUME: usize = SIDE * SIDE * SIDE;
const PAD: usize = SIDE + 2;

/// Chunk coordinates (voxel coordinate divided by 16, rounded down),
/// ordered by z, then y, then x.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChunkKey(pub(crate) [i32; 3]);

impl Ord for ChunkKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let [x, y, z] = self.0;
        let [ox, oy, oz] = other.0;
        (z, y, x).cmp(&(oz, oy, ox))
    }
}

impl PartialOrd for ChunkKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl ChunkKey {
    /// Chunk holding `pos` and the local index of `pos` inside it.
    pub(crate) fn of(pos: Pos) -> (Self, usize) {
        let key = pos.map(|c| c.div_euclid(CHUNK_SIDE));
        let [lx, ly, lz] = pos.map(|c| c.rem_euclid(CHUNK_SIDE) as usize);
        (Self(key), local_index(lx, ly, lz))
    }

    /// Inclusive range of chunk coordinates covering `bounds` on each axis.
    pub(crate) fn span(bounds: &Bounds) -> [(i32, i32); 3] {
        [0, 1, 2].map(|a| (bounds.min[a].div_euclid(CHUNK_SIDE), bounds.max[a].div_euclid(CHUNK_SIDE)))
    }

    fn offset(self, d: [i32; 3]) -> Self {
        Self([self.0[0] + d[0], self.0[1] + d[1], self.0[2] + d[2]])
    }

    /// Voxel box covered by the chunk.
    pub(crate) fn bounds(self) -> Bounds {
        let min = self.0.map(|c| c * CHUNK_SIDE);
        Bounds { min, max: min.map(|c| c + CHUNK_SIDE - 1) }
    }

    /// Global position of a local index.
    pub(crate) fn position(self, i: usize) -> Pos {
        let local = [i % SIDE, (i / SIDE) % SIDE, i / (SIDE * SIDE)];
        [0, 1, 2].map(|a| self.0[a] * CHUNK_SIDE + local[a] as i32)
    }
}

#[inline]
fn local_index(x: usize, y: usize, z: usize) -> usize {
    (z * SIDE + y) * SIDE + x
}

/// One stored chunk.
#[derive(Clone, Debug)]
pub(crate) struct Chunk {
    pub(crate) cells: Box<[u8; VOLUME]>,
    pub(crate) ages: Box<[u16; VOLUME]>,
    occupied: u32,
    alive: u32,
}

impl Chunk {
    fn empty() -> Self {
        Self { cells: Box::new([0; VOLUME]), ages: Box::new([0; VOLUME]), occupied: 0, alive: 0 }
    }

    /// Chunk from raw contents, or `None` when every voxel is empty.
    pub(crate) fn from_parts(cells: Box<[u8; VOLUME]>, ages: Box<[u16; VOLUME]>) -> Option<Self> {
        let occupied = cells.iter().filter(|&&c| c != 0).count() as u32;
        let alive = cells.iter().filter(|&&c| c == 1).count() as u32;
        (occupied > 0).then_some(Self { cells, ages, occupied, alive })
    }
}

/// Sparse voxel storage clipped to `bounds`.
#[derive(Clone, Debug)]
pub(crate) struct SparseGrid {
    bounds: Bounds,
    chunks: HashMap<ChunkKey, Chunk>,
    keys: Vec<ChunkKey>,
}

impl SparseGrid {
    /// Empty storage accepting voxels inside `bounds`.
    pub(crate) fn new(bounds: Bounds) -> Self {
        Self { bounds, chunks: HashMap::new(), keys: Vec::new() }
    }

    /// Box of coordinates that can hold voxels.
    pub(crate) fn bounds(&self) -> Bounds {
        self.bounds
    }

    /// Stored chunks in (z, y, x) order.
    pub(crate) fn chunks(&self) -> impl Iterator<Item = (ChunkKey, &Chunk)> {
        self.keys.iter().filter_map(|key| self.chunks.get(key).map(|chunk| (*key, chunk)))
    }

    /// Number of stored chunks.
    pub(crate) fn chunk_count(&self) -> usize {
        self.keys.len()
    }

    /// The stored chunk `key`, if any.
    pub(crate) fn chunk(&self, key: ChunkKey) -> Option<&Chunk> {
        self.chunks.get(&key)
    }

    /// The stored chunks across the six faces of chunk `key`, in the order
    /// −x, +x, −y, +y, −z, +z.
    pub(crate) fn face_neighbours(&self, key: ChunkKey) -> [Option<&Chunk>; 6] {
        [[-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1]].map(|d| self.chunk(key.offset(d)))
    }

    /// Inserts a whole chunk (used when restoring snapshots); keys must be
    /// inserted in increasing order.
    pub(crate) fn push_chunk(&mut self, key: ChunkKey, chunk: Chunk) -> bool {
        if self.keys.last().is_some_and(|last| *last >= key) {
            return false;
        }
        self.keys.push(key);
        self.chunks.insert(key, chunk);
        true
    }

    /// State and age at `pos`.
    pub(crate) fn get(&self, pos: Pos) -> (u8, u16) {
        let (key, i) = ChunkKey::of(pos);
        self.chunks.get(&key).map_or((0, 0), |c| (c.cells[i], c.ages[i]))
    }

    /// State at `pos`.
    #[inline]
    pub(crate) fn state(&self, pos: Pos) -> u8 {
        let (key, i) = ChunkKey::of(pos);
        self.chunks.get(&key).map_or(0, |c| c.cells[i])
    }

    /// Writes a voxel inside the bounds and resets its age.
    pub(crate) fn set(&mut self, pos: Pos, state: u8) {
        if !self.bounds.contains(pos) {
            return;
        }
        let (key, i) = ChunkKey::of(pos);
        if state == 0 && !self.chunks.contains_key(&key) {
            return;
        }
        let chunk = self.chunks.entry(key).or_insert_with(|| {
            if let Err(slot) = self.keys.binary_search(&key) {
                self.keys.insert(slot, key);
            }
            Chunk::empty()
        });
        let old = chunk.cells[i];
        chunk.occupied = chunk.occupied + u32::from(state != 0) - u32::from(old != 0);
        chunk.alive = chunk.alive + u32::from(state == 1) - u32::from(old == 1);
        chunk.cells[i] = state;
        chunk.ages[i] = 0;
        if chunk.occupied == 0 {
            self.chunks.remove(&key);
            self.keys.retain(|k| *k != key);
        }
    }

    /// Removes every chunk.
    pub(crate) fn clear(&mut self) {
        self.chunks.clear();
        self.keys.clear();
    }

    /// Empties voxels whose state is `states` or more.
    pub(crate) fn clamp_states(&mut self, states: u16) {
        let keys = std::mem::take(&mut self.keys);
        let mut kept = Vec::with_capacity(keys.len());
        for key in keys {
            let Some(chunk) = self.chunks.remove(&key) else { continue };
            let Chunk { mut cells, mut ages, .. } = chunk;
            for (cell, age) in cells.iter_mut().zip(ages.iter_mut()) {
                if u16::from(*cell) >= states {
                    *cell = 0;
                    *age = 0;
                }
            }
            if let Some(chunk) = Chunk::from_parts(cells, ages) {
                self.chunks.insert(key, chunk);
                kept.push(key);
            }
        }
        self.keys = kept;
    }

    /// Bytes held by stored chunks (approximate).
    pub(crate) fn memory_bytes(&self) -> usize {
        self.keys.len() * (3 * VOLUME + std::mem::size_of::<(ChunkKey, Chunk)>() + std::mem::size_of::<ChunkKey>())
    }

    /// Chunks that the next generation must evaluate, sorted and unique.
    pub(crate) fn candidates(&self) -> Vec<ChunkKey> {
        let mut candidates = Vec::with_capacity(self.keys.len() * 4);
        for (key, chunk) in self.chunks() {
            candidates.push(key);
            if chunk.alive > 0 {
                for d in Neighbourhood::Moore.offsets() {
                    let neighbour = key.offset(*d);
                    if neighbour.bounds().intersection(&self.bounds).is_some() {
                        candidates.push(neighbour);
                    }
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        candidates
    }

    /// Advances one generation over `candidates` (from [`Self::candidates`]).
    pub(crate) fn step(
        &mut self,
        candidates: &[ChunkKey],
        transition: &Transition,
        neighbourhood: Neighbourhood,
        states: u16,
    ) -> Stats {
        let results: Vec<(ChunkKey, Option<Chunk>, Stats)> = candidates
            .par_iter()
            .map(|&key| {
                let (chunk, stats) = self.next_chunk(key, transition, neighbourhood, states);
                (key, chunk, stats)
            })
            .collect();
        let mut chunks = HashMap::with_capacity(results.len());
        let mut keys = Vec::with_capacity(results.len());
        let mut parts = Vec::with_capacity(results.len());
        for (key, chunk, stats) in results {
            if let Some(chunk) = chunk {
                chunks.insert(key, chunk);
                keys.push(key);
            }
            parts.push(stats);
        }
        self.chunks = chunks;
        self.keys = keys;
        Stats::combine(states, parts)
    }

    /// Alive bits of the chunk and a one-voxel shell around it.
    fn padded_alive(&self, key: ChunkKey) -> Box<[u8; PAD * PAD * PAD]> {
        let mut padded = Box::new([0u8; PAD * PAD * PAD]);
        // For a neighbour offset d: (first source index, first padded index, length).
        let span = |d: i32| match d {
            -1 => (SIDE - 1, 0, 1),
            0 => (0, 1, SIDE),
            _ => (0, SIDE + 1, 1),
        };
        for d in Neighbourhood::Moore.offsets().iter().chain(std::iter::once(&[0, 0, 0])) {
            let Some(chunk) = self.chunks.get(&key.offset(*d)) else { continue };
            if chunk.alive == 0 {
                continue;
            }
            let [(sx, px, nx), (sy, py, ny), (sz, pz, nz)] = [span(d[0]), span(d[1]), span(d[2])];
            for z in 0..nz {
                for y in 0..ny {
                    let src = local_index(sx, sy + y, sz + z);
                    let dst = ((pz + z) * PAD + py + y) * PAD + px;
                    for x in 0..nx {
                        padded[dst + x] = u8::from(chunk.cells[src + x] == 1);
                    }
                }
            }
        }
        padded
    }

    fn next_chunk(
        &self,
        key: ChunkKey,
        transition: &Transition,
        neighbourhood: Neighbourhood,
        states: u16,
    ) -> (Option<Chunk>, Stats) {
        let padded = self.padded_alive(key);
        let counts = match neighbourhood {
            Neighbourhood::Moore => moore_counts(&padded),
            Neighbourhood::VonNeumann => von_neumann_counts(&padded),
        };
        let current = self.chunks.get(&key);
        let chunk_box = key.bounds();
        let clip = chunk_box.intersection(&self.bounds).filter(|inside| *inside != chunk_box);
        let mut cells = Box::new([0u8; VOLUME]);
        let mut ages = Box::new([0u16; VOLUME]);
        let mut stats = Stats::new(states);
        for i in 0..VOLUME {
            let (old, age) = current.map_or((0, 0), |c| (c.cells[i], c.ages[i]));
            let pos = key.position(i);
            if clip.is_some_and(|inside| !inside.contains(pos)) {
                continue;
            }
            let new = transition.apply(old, counts[i]);
            stats.transition(old, new);
            if new != 0 {
                cells[i] = new;
                ages[i] = if old == 0 { 0 } else { age.saturating_add(1) };
                stats.add(pos, new);
            }
        }
        (Chunk::from_parts(cells, ages), stats)
    }
}

/// Moore counts of the 16³ interior from the padded alive block (separable
/// 3×3×3 box sum minus the voxel itself).
fn moore_counts(p: &[u8; PAD * PAD * PAD]) -> Box<[u8; VOLUME]> {
    let mut sx = vec![0u8; PAD * PAD * SIDE];
    for z in 0..PAD {
        for y in 0..PAD {
            let row = (z * PAD + y) * PAD;
            for x in 0..SIDE {
                sx[(z * PAD + y) * SIDE + x] = p[row + x] + p[row + x + 1] + p[row + x + 2];
            }
        }
    }
    let mut sy = vec![0u8; PAD * SIDE * SIDE];
    for z in 0..PAD {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let at = |yy: usize| sx[(z * PAD + yy) * SIDE + x];
                sy[(z * SIDE + y) * SIDE + x] = at(y) + at(y + 1) + at(y + 2);
            }
        }
    }
    let mut counts = Box::new([0u8; VOLUME]);
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let at = |zz: usize| sy[(zz * SIDE + y) * SIDE + x];
                let own = p[((z + 1) * PAD + y + 1) * PAD + x + 1];
                counts[local_index(x, y, z)] = at(z) + at(z + 1) + at(z + 2) - own;
            }
        }
    }
    counts
}

/// Von Neumann counts of the 16³ interior from the padded alive block.
fn von_neumann_counts(p: &[u8; PAD * PAD * PAD]) -> Box<[u8; VOLUME]> {
    let at = |x: usize, y: usize, z: usize| p[(z * PAD + y) * PAD + x];
    let mut counts = Box::new([0u8; VOLUME]);
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let (px, py, pz) = (x + 1, y + 1, z + 1);
                counts[local_index(x, y, z)] = at(px - 1, py, pz)
                    + at(px + 1, py, pz)
                    + at(px, py - 1, pz)
                    + at(px, py + 1, pz)
                    + at(px, py, pz - 1)
                    + at(px, py, pz + 1);
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_local_indices() {
        let (key, i) = ChunkKey::of([-1, 16, 33]);
        assert_eq!(key, ChunkKey([-1, 1, 2]));
        assert_eq!(key.position(i), [-1, 16, 33]);
        assert!(ChunkKey([5, 0, 0]) < ChunkKey([0, 1, 0]));
        assert!(ChunkKey([0, 5, 0]) < ChunkKey([0, 0, 1]));
    }

    #[test]
    fn set_get_and_chunk_bookkeeping() {
        let mut grid = SparseGrid::new(Bounds::unbounded());
        grid.set([20, -3, 7], 2);
        grid.set([-40, 0, 0], 1);
        assert_eq!(grid.get([20, -3, 7]), (2, 0));
        assert_eq!(grid.state([-40, 0, 0]), 1);
        assert_eq!(grid.chunk_count(), 2);
        let keys: Vec<ChunkKey> = grid.chunks().map(|(k, _)| k).collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        grid.set([20, -3, 7], 0);
        assert_eq!(grid.chunk_count(), 1);
        grid.clamp_states(1);
        assert_eq!(grid.chunk_count(), 0);
        let mut bounded = SparseGrid::new(Bounds::from_size([4, 4, 4]));
        bounded.set([4, 0, 0], 1);
        assert_eq!(bounded.chunk_count(), 0, "voxels outside the bounds are ignored");
    }
}
