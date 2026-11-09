//! Snapshot payload (schema version 1).
//!
//! Voxel states and ages are stored run-length encoded: one `(value, run)`
//! list over the dense layout, or one per stored 16³ chunk (in sorted chunk
//! order) for sparse worlds. Decoding re-validates every invariant (sizes,
//! states below the rule's state count, chunk order and bounds), so a
//! corrupt payload is rejected instead of producing an inconsistent world.

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use super::VoxelLife;
use crate::dense::DenseGrid;
use crate::error::VoxelError;
use crate::geometry::WorldKind;
use crate::init::WorldConfig;
use crate::rule::Rule3D;
use crate::scenarios::Scenario;
use crate::sparse::{CHUNK_SIDE, Chunk, ChunkKey, SparseGrid};
use crate::world::Storage;

const CHUNK_VOLUME: usize = (CHUNK_SIDE * CHUNK_SIDE * CHUNK_SIDE) as usize;

/// One stored chunk.
#[derive(Serialize, Deserialize)]
struct SavedChunk {
    key: [i32; 3],
    cells: Vec<(u8, u32)>,
    ages: Vec<(u16, u32)>,
}

/// Voxel contents.
#[derive(Serialize, Deserialize)]
enum SavedStorage {
    Dense { cells: Vec<(u8, u32)>, ages: Vec<(u16, u32)> },
    Sparse { chunks: Vec<SavedChunk> },
}

/// Complete model state.
#[derive(Serialize, Deserialize)]
pub(super) struct SavedState {
    pub(super) scenario: Scenario,
    pub(super) rule: Rule3D,
    pub(super) active: WorldConfig,
    pub(super) pending: WorldConfig,
    storage: SavedStorage,
    pub(super) tick: u64,
    pub(super) rng: SimRng,
    pub(super) births: u64,
    pub(super) deaths: u64,
    pub(super) halted: Option<String>,
}

fn encode_runs<T: Copy + PartialEq>(data: &[T]) -> Vec<(T, u32)> {
    let mut runs: Vec<(T, u32)> = Vec::new();
    for &value in data {
        match runs.last_mut() {
            Some((last, len)) if *last == value && *len < u32::MAX => *len += 1,
            _ => runs.push((value, 1)),
        }
    }
    runs
}

fn decode_runs<T: Copy>(runs: &[(T, u32)], len: usize, what: &str) -> Result<Vec<T>, VoxelError> {
    let total: u64 = runs.iter().map(|&(_, n)| u64::from(n)).sum();
    if total != len as u64 {
        return Err(VoxelError::Snapshot(format!("{what} hold {total} values, expected {len}")));
    }
    let mut out = Vec::with_capacity(len);
    for &(value, n) in runs {
        out.extend(std::iter::repeat_n(value, n as usize));
    }
    Ok(out)
}

fn check_states(cells: &[u8], states: u16) -> Result<(), VoxelError> {
    match cells.iter().find(|&&c| u16::from(c) >= states) {
        Some(bad) => Err(VoxelError::Snapshot(format!("state {bad} does not exist in a {states}-state rule"))),
        None => Ok(()),
    }
}

impl SavedState {
    /// Captures the complete state of a model.
    pub(super) fn capture(life: &VoxelLife) -> Self {
        let storage = match &life.storage {
            Storage::Dense(grid) => {
                SavedStorage::Dense { cells: encode_runs(grid.cells()), ages: encode_runs(grid.ages()) }
            }
            Storage::Sparse(grid) => SavedStorage::Sparse {
                chunks: grid
                    .chunks()
                    .map(|(key, chunk)| SavedChunk {
                        key: key.0,
                        cells: encode_runs(&chunk.cells[..]),
                        ages: encode_runs(&chunk.ages[..]),
                    })
                    .collect(),
            },
        };
        Self {
            scenario: life.scenario.clone(),
            rule: life.rule,
            active: life.active.clone(),
            pending: life.pending.clone(),
            storage,
            tick: life.tick,
            rng: life.rng.clone(),
            births: life.stats.births(),
            deaths: life.stats.deaths(),
            halted: life.halted.clone(),
        }
    }

    /// Validates the configurations and the generator state, and rebuilds
    /// the voxel storage.
    pub(super) fn rebuild_storage(&self) -> Result<Storage, VoxelError> {
        self.scenario.validate()?;
        self.active.validate()?;
        self.pending.validate()?;
        if self.rng.state() == [0; 4] {
            return Err(VoxelError::Snapshot("the random generator state is all zero".into()));
        }
        let states = self.rule.states();
        let WorldConfig { kind, size, .. } = self.active;
        match (&self.storage, kind.is_sparse()) {
            (SavedStorage::Dense { cells, ages }, false) => {
                let volume = size.iter().map(|&s| s as usize).product();
                let cells = decode_runs(cells, volume, "dense states")?;
                let ages = decode_runs(ages, volume, "dense ages")?;
                check_states(&cells, states)?;
                let grid = DenseGrid::from_parts(size, kind == WorldKind::DenseTorus, cells, ages)
                    .ok_or_else(|| VoxelError::Snapshot("dense buffers do not match the world size".into()))?;
                Ok(Storage::Dense(grid))
            }
            (SavedStorage::Sparse { chunks }, true) => {
                let bounds = kind.bounds(size);
                let span = ChunkKey::span(&bounds);
                let mut grid = SparseGrid::new(bounds);
                for saved in chunks {
                    // Checked first: voxel positions of keys outside this range overflow `i32`.
                    if (0..3).any(|a| saved.key[a] < span[a].0 || saved.key[a] > span[a].1) {
                        return Err(VoxelError::Snapshot(format!("chunk {:?} lies outside the world", saved.key)));
                    }
                    let key = ChunkKey(saved.key);
                    let cells = decode_runs(&saved.cells, CHUNK_VOLUME, "chunk states")?;
                    let ages = decode_runs(&saved.ages, CHUNK_VOLUME, "chunk ages")?;
                    check_states(&cells, states)?;
                    if cells.iter().enumerate().any(|(i, &c)| c != 0 && !bounds.contains(key.position(i))) {
                        return Err(VoxelError::Snapshot(format!(
                            "chunk {:?} has voxels outside the world",
                            saved.key
                        )));
                    }
                    let wrong_size = || VoxelError::Snapshot("chunk buffers have the wrong size".into());
                    let cells: Box<[u8; CHUNK_VOLUME]> =
                        cells.into_boxed_slice().try_into().map_err(|_| wrong_size())?;
                    let ages: Box<[u16; CHUNK_VOLUME]> =
                        ages.into_boxed_slice().try_into().map_err(|_| wrong_size())?;
                    let chunk = Chunk::from_parts(cells, ages)
                        .ok_or_else(|| VoxelError::Snapshot(format!("chunk {:?} is stored but empty", saved.key)))?;
                    if !grid.push_chunk(key, chunk) {
                        return Err(VoxelError::Snapshot("chunks are not in increasing (z, y, x) order".into()));
                    }
                }
                Ok(Storage::Sparse(grid))
            }
            _ => Err(VoxelError::Snapshot(format!("the stored voxels do not match world kind `{}`", kind.key()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_length_round_trip() {
        let data = [0u8, 0, 0, 1, 1, 2, 0, 0];
        let runs = encode_runs(&data);
        assert_eq!(runs, vec![(0, 3), (1, 2), (2, 1), (0, 2)]);
        assert_eq!(decode_runs(&runs, data.len(), "test").ok(), Some(data.to_vec()));
        assert!(decode_runs(&runs, 9, "test").is_err());
        assert!(encode_runs::<u8>(&[]).is_empty());
    }

    fn sparse_state() -> SavedState {
        let scenario = crate::scenarios::builtin_by_id("crystal-growth-1").expect("embedded scenario");
        SavedState::capture(&VoxelLife::new(scenario).expect("valid scenario"))
    }

    fn one_voxel_chunk(key: [i32; 3]) -> SavedStorage {
        let cells = vec![(1, 1), (0, CHUNK_VOLUME as u32 - 1)];
        SavedStorage::Sparse { chunks: vec![SavedChunk { key, cells, ages: vec![(0, CHUNK_VOLUME as u32)] }] }
    }

    #[test]
    fn corrupt_payloads_are_rejected_without_panicking() {
        // Chunk keys whose voxel coordinates would overflow i32, or that lie
        // just outside the unbounded world's chunk range.
        let limit = crate::COORD_LIMIT / CHUNK_SIDE;
        for key in [[i32::MAX, 0, 0], [0, i32::MIN, 0], [0, 0, limit], [-limit - 1, 0, 0]] {
            let state = SavedState { storage: one_voxel_chunk(key), ..sparse_state() };
            let error = state.rebuild_storage().err().map(|e| e.to_string()).unwrap_or_default();
            assert!(error.contains("outside the world"), "{key:?}: {error}");
        }
        let edge = SavedState { storage: one_voxel_chunk([limit - 1, -limit, 0]), ..sparse_state() };
        assert!(edge.rebuild_storage().is_ok(), "the outermost chunks of the unbounded world are valid");
        let zero_rng = SavedState { rng: SimRng::from_state([1, 0, 0, 0]), ..sparse_state() };
        assert!(zero_rng.rebuild_storage().is_ok());
        let mut stuck = sparse_state();
        stuck.rng = ron::from_str("(s: (0, 0, 0, 0))").expect("a SimRng state in RON");
        assert!(stuck.rebuild_storage().is_err(), "an all-zero xoshiro state never changes");
        let dense_in_sparse =
            SavedState { storage: SavedStorage::Dense { cells: vec![(0, 1)], ages: vec![(0, 1)] }, ..sparse_state() };
        assert!(dense_in_sparse.rebuild_storage().is_err());
    }
}
