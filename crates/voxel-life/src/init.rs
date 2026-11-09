//! World configuration and initial states.
//!
//! The *world centre* is `⌊size / 2⌋` on every axis. A random cube of side
//! `s` (clamped to the world on each axis) starts at `centre − ⌊s / 2⌋`, so
//! it always fits inside a finite world. With mirror symmetry along an axis
//! only the first half of the cube (rounded up) is drawn and every drawn
//! voxel is also placed at its reflection `s − 1 − l` in cube coordinates.
//! Voxels are drawn in z, y, x order from the generator handed in, one
//! `chance(density)` call per voxel of the drawn region.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::error::VoxelError;
use crate::geometry::{Bounds, MAX_SIDE, Pos, WorldKind, validate_size};
use crate::sparse::{CHUNK_SIDE, ChunkKey};
use crate::world::{MAX_SPARSE_CHUNKS, Storage, too_many_chunks};

/// Largest number of voxels a random cube may draw (256³).
pub const MAX_RANDOM_VOXELS: u64 = 1 << 24;

/// What the world contains after a reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InitKind {
    /// Random alive voxels in a centred cube.
    #[default]
    RandomCube,
    /// One alive voxel at the world centre.
    SingleVoxel,
    /// The scenario's voxel list, placed relative to the world centre.
    Pattern,
    /// No voxels.
    Empty,
}

impl InitKind {
    /// All kinds in display order.
    pub const ALL: [InitKind; 4] = [Self::RandomCube, Self::SingleVoxel, Self::Pattern, Self::Empty];

    /// Stable key used in parameters and scenario files.
    pub fn key(self) -> &'static str {
        match self {
            Self::RandomCube => "random_cube",
            Self::SingleVoxel => "single_voxel",
            Self::Pattern => "pattern",
            Self::Empty => "empty",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::RandomCube => "Random cube at the centre",
            Self::SingleVoxel => "Single voxel at the centre",
            Self::Pattern => "Scenario pattern",
            Self::Empty => "Empty",
        }
    }

    /// Parses an [`InitKind::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }
}

/// Mirror symmetry of a random cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mirror {
    /// No symmetry.
    #[default]
    None,
    /// Symmetric under x → −x (about the cube centre).
    X,
    /// Symmetric under y → −y.
    Y,
    /// Symmetric under z → −z.
    Z,
    /// Symmetric along x and y.
    Xy,
    /// Symmetric along x and z.
    Xz,
    /// Symmetric along y and z.
    Yz,
    /// Symmetric along all three axes.
    Xyz,
}

impl Mirror {
    /// All options in display order.
    pub const ALL: [Mirror; 8] = [Self::None, Self::X, Self::Y, Self::Z, Self::Xy, Self::Xz, Self::Yz, Self::Xyz];

    /// Stable key used in parameters and scenario files.
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
            Self::Xy => "xy",
            Self::Xz => "xz",
            Self::Yz => "yz",
            Self::Xyz => "xyz",
        }
    }

    /// Parses a [`Mirror::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }

    /// Which axes are mirrored.
    pub fn axes(self) -> [bool; 3] {
        let key = self.key();
        ['x', 'y', 'z'].map(|c| key.contains(c))
    }
}

/// One voxel of an initial pattern: offset from the world centre and state.
/// Written `(dx, dy, dz, state)` in scenario files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "(i32, i32, i32, u8)", into = "(i32, i32, i32, u8)")]
pub struct PatternVoxel {
    /// Offset from the world centre.
    pub offset: Pos,
    /// State (1 = alive; 2… = dying states of multi-state rules).
    pub state: u8,
}

impl From<(i32, i32, i32, u8)> for PatternVoxel {
    fn from((x, y, z, state): (i32, i32, i32, u8)) -> Self {
        Self { offset: [x, y, z], state }
    }
}

impl From<PatternVoxel> for (i32, i32, i32, u8) {
    fn from(v: PatternVoxel) -> Self {
        (v.offset[0], v.offset[1], v.offset[2], v.state)
    }
}

/// How the world is filled on reset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InitSpec {
    /// Kind of initial state.
    pub kind: InitKind,
    /// Side of the centred random cube (clamped to the world per axis).
    pub cube_side: u32,
    /// Probability that a drawn voxel of the random cube is alive.
    pub density: f64,
    /// Mirror symmetry of the random cube.
    pub mirror: Mirror,
    /// Voxels of [`InitKind::Pattern`].
    pub pattern: Vec<PatternVoxel>,
}

impl Default for InitSpec {
    fn default() -> Self {
        Self { kind: InitKind::RandomCube, cube_side: 8, density: 0.5, mirror: Mirror::None, pattern: Vec::new() }
    }
}

/// Complete description of a world and its initial state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldConfig {
    /// Storage and edges.
    pub kind: WorldKind,
    /// World size `[W, H, D]` (the initialisation box of unbounded worlds).
    pub size: [u32; 3],
    /// Initial state.
    pub init: InitSpec,
    /// Seed of the initialisation and command random streams.
    pub seed: u64,
}

impl WorldConfig {
    /// Checks sizes, density, the random cube (voxel and, for sparse worlds,
    /// chunk limits) and the pattern (presence and, for sparse worlds, chunk
    /// limit).
    pub fn validate(&self) -> Result<(), VoxelError> {
        validate_size(self.kind, self.size).map_err(VoxelError::Config)?;
        let init = &self.init;
        if !(0.0..=1.0).contains(&init.density) {
            return Err(VoxelError::Config(format!("density {} must be between 0 and 1", init.density)));
        }
        if init.cube_side == 0 || init.cube_side > MAX_SIDE {
            return Err(VoxelError::Config(format!(
                "random cube side {} must be between 1 and {MAX_SIDE}",
                init.cube_side
            )));
        }
        self.check_cube()?;
        if init.kind == InitKind::Pattern {
            if init.pattern.is_empty() {
                return Err(VoxelError::Config("the initial state is `pattern` but the pattern has no voxels".into()));
            }
            if self.kind.is_sparse() {
                let centre = self.centre();
                let chunks: BTreeSet<[i64; 3]> = init
                    .pattern
                    .iter()
                    .map(|v| {
                        [0, 1, 2]
                            .map(|a| (i64::from(centre[a]) + i64::from(v.offset[a])).div_euclid(i64::from(CHUNK_SIDE)))
                    })
                    .collect();
                if chunks.len() > MAX_SPARSE_CHUNKS {
                    return Err(too_many_chunks());
                }
            }
        }
        Ok(())
    }

    /// World centre `⌊size / 2⌋`.
    pub fn centre(&self) -> Pos {
        self.size.map(|s| (s / 2) as i32)
    }

    /// Box covered by the centred random cube (see the module documentation).
    pub(crate) fn cube_box(&self) -> Bounds {
        let side = self.size.map(|s| self.init.cube_side.clamp(1, s.max(1)));
        let min = [0, 1, 2].map(|a| (self.size[a] / 2 - side[a] / 2) as i32);
        Bounds { min, max: [0, 1, 2].map(|a| min[a] + side[a] as i32 - 1) }
    }

    /// Checks that the random cube covers at most [`MAX_RANDOM_VOXELS`] voxels
    /// and, in a sparse world, at most [`MAX_SPARSE_CHUNKS`] chunks.
    pub(crate) fn check_cube(&self) -> Result<(), VoxelError> {
        let cube = self.cube_box();
        let volume = cube.volume();
        if volume > MAX_RANDOM_VOXELS {
            return Err(VoxelError::Limit(format!(
                "a random cube of side {} covers {volume} voxels in this world, more than the limit of \
                 {MAX_RANDOM_VOXELS}",
                self.init.cube_side
            )));
        }
        let chunks: u64 = ChunkKey::span(&cube).iter().map(|&(lo, hi)| (hi - lo + 1) as u64).product();
        if self.kind.is_sparse() && chunks > MAX_SPARSE_CHUNKS as u64 {
            return Err(too_many_chunks());
        }
        Ok(())
    }

    /// Fills `storage` (assumed empty) with the initial state. Pattern voxels
    /// outside the world or in states the rule lacks are skipped.
    pub(crate) fn populate(&self, storage: &mut Storage, states: u16, rng: &mut SimRng) {
        let centre = self.centre();
        match self.init.kind {
            InitKind::Empty => {}
            InitKind::SingleVoxel => storage.set(centre, 1),
            InitKind::Pattern => {
                for voxel in &self.init.pattern {
                    let pos = [0, 1, 2].map(|a| centre[a].checked_add(voxel.offset[a]));
                    if let [Some(x), Some(y), Some(z)] = pos
                        && voxel.state != 0
                        && u16::from(voxel.state) < states
                    {
                        storage.set([x, y, z], voxel.state);
                    }
                }
            }
            InitKind::RandomCube => self.random_cube(storage, rng),
        }
    }

    /// Draws the centred random cube (see the module documentation).
    pub(crate) fn random_cube(&self, storage: &mut Storage, rng: &mut SimRng) {
        let InitSpec { density, mirror, .. } = self.init;
        let cube = self.cube_box();
        let origin = cube.min;
        let side = cube.extent().map(|e| e as u32);
        let mirrored = mirror.axes();
        let drawn = [0, 1, 2].map(|a| if mirrored[a] { side[a].div_ceil(2) } else { side[a] });
        let images = |a: usize, l: u32| {
            let reflected = side[a] - 1 - l;
            if mirrored[a] && reflected != l { vec![l, reflected] } else { vec![l] }
        };
        for z in 0..drawn[2] {
            for y in 0..drawn[1] {
                for x in 0..drawn[0] {
                    if !rng.chance(density) {
                        continue;
                    }
                    for &iz in &images(2, z) {
                        for &iy in &images(1, y) {
                            for &ix in &images(0, x) {
                                let pos = [origin[0] + ix as i32, origin[1] + iy as i32, origin[2] + iz as i32];
                                storage.set(pos, 1);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(size: [u32; 3], init: InitSpec) -> WorldConfig {
        WorldConfig { kind: WorldKind::DenseBounded, size, init, seed: 1 }
    }

    fn voxels(config: &WorldConfig) -> Vec<Pos> {
        let mut storage = Storage::new(config.kind, config.size);
        config.populate(&mut storage, 2, &mut SimRng::seed_from_u64(config.seed));
        storage.voxels().into_iter().map(|(p, _, _)| p).collect()
    }

    #[test]
    fn random_cube_is_centred_and_clamped() {
        let init = InitSpec { cube_side: 4, density: 1.0, ..InitSpec::default() };
        let cells = voxels(&config([10, 9, 3], init));
        assert_eq!(cells.len(), 4 * 4 * 3, "the cube is clamped to the 3-voxel depth");
        assert!(cells.iter().all(|p| (3..7).contains(&p[0]) && (2..6).contains(&p[1]) && (0..3).contains(&p[2])));
    }

    #[test]
    fn mirror_symmetry_holds() {
        for mirror in Mirror::ALL {
            let init = InitSpec { cube_side: 7, density: 0.4, mirror, ..InitSpec::default() };
            let cfg = config([16, 16, 16], init);
            let cells = voxels(&cfg);
            assert!(!cells.is_empty());
            // Cube spans [5, 11] on every axis; reflection l → 16 − l keeps it.
            for (axis, on) in mirror.axes().into_iter().enumerate() {
                if on {
                    for p in &cells {
                        let mut q = *p;
                        q[axis] = 5 + 11 - p[axis];
                        assert!(cells.contains(&q), "{mirror:?}: {p:?} lacks its image {q:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn single_voxel_and_pattern() {
        let single = voxels(&config([5, 6, 7], InitSpec { kind: InitKind::SingleVoxel, ..InitSpec::default() }));
        assert_eq!(single, vec![[2, 3, 3]]);
        let pattern = vec![
            PatternVoxel { offset: [0, 0, 0], state: 1 },
            PatternVoxel { offset: [-9, 0, 0], state: 1 },
            PatternVoxel { offset: [1, 0, 0], state: 3 },
        ];
        let cells = voxels(&config([4, 4, 4], InitSpec { kind: InitKind::Pattern, pattern, ..InitSpec::default() }));
        assert_eq!(cells, vec![[2, 2, 2]], "outside voxels and missing states are skipped");
    }

    #[test]
    fn validation() {
        let mut cfg = config([8, 8, 8], InitSpec::default());
        assert!(cfg.validate().is_ok());
        cfg.init.density = 1.5;
        assert!(cfg.validate().is_err());
        cfg.init.density = 0.5;
        cfg.init.kind = InitKind::Pattern;
        assert!(cfg.validate().is_err());
        cfg.init.kind = InitKind::RandomCube;
        cfg.kind = WorldKind::SparseBounded;
        cfg.size = [4096, 4096, 4096];
        cfg.init.cube_side = 4096;
        assert!(matches!(cfg.validate(), Err(VoxelError::Limit(m)) if m.contains("random cube")));
        // 2^24 voxels in a thin slab touch 4096 × 16 chunks: over the sparse chunk limit.
        cfg.size = [65_536, 256, 1];
        cfg.init.cube_side = 65_536;
        assert!(matches!(cfg.validate(), Err(VoxelError::Limit(m)) if m.contains("chunks")));
        cfg.kind = WorldKind::DenseBounded;
        cfg.size = [256, 256, 256];
        cfg.init.cube_side = 256;
        assert!(cfg.validate().is_ok(), "dense worlds have no chunk limit");
        // A sparse pattern spread over more chunks than the limit.
        cfg.kind = WorldKind::SparseUnbounded;
        cfg.init.kind = InitKind::Pattern;
        cfg.init.pattern = (0..40_000).map(|i| PatternVoxel { offset: [i * 16, 0, 0], state: 1 }).collect();
        assert!(matches!(cfg.validate(), Err(VoxelError::Limit(m)) if m.contains("chunks")));
        cfg.init.pattern.truncate(1_000);
        assert!(cfg.validate().is_ok());
    }
}
