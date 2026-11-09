//! Documented moving objects: periods, displacements and populations.

mod common;

use std::collections::BTreeSet;

use common::*;
use simulation_core::Simulation;
use voxel_life::{Pos, VoxelCommand, VoxelLife, WorldKind};

/// Pattern translated so that its bounding box starts at the origin.
fn normalised(set: &BTreeSet<Pos>) -> BTreeSet<Pos> {
    let min = [0, 1, 2].map(|a| set.iter().map(|p| p[a]).min().unwrap_or(0));
    shifted(set, [-min[0], -min[1], -min[2]])
}

/// Checks that the object has exactly `period` and moves by `displacement`
/// per period. `phase_population` is asserted in every phase when the
/// source documents a constant population.
fn assert_spaceship(
    life: &mut VoxelLife,
    period: u64,
    displacement: [i32; 3],
    population: usize,
    phase_population: Option<usize>,
) -> Vec<usize> {
    let start = positions(life);
    assert_eq!(start.len(), population);
    let mut populations = vec![population];
    for generation in 1..period {
        life.step();
        let phase = positions(life);
        populations.push(phase.len());
        if let Some(expected) = phase_population {
            assert_eq!(phase.len(), expected, "population in phase {generation}");
        }
        assert_ne!(normalised(&phase), normalised(&start), "phase {generation} repeats the start: period < {period}");
    }
    life.step();
    assert_eq!(positions(life), shifted(&start, displacement), "displacement after one period");
    populations
}

#[test]
fn bays_5766_glider() {
    let mut life = builtin("life-5766-glider");
    assert_spaceship(&mut life, 4, [1, 1, 0], 10, Some(10));
    // On the 24×24 torus it is back where it started after 24 periods.
    let mut life = builtin("life-5766-glider");
    let start = positions(&life);
    life.advance(96);
    assert_eq!(positions(&life), start);
    life.advance(48);
    assert_ne!(positions(&life), start);
}

#[test]
fn bays_4555_glider() {
    let mut life = builtin("life-4555-glider");
    assert_spaceship(&mut life, 4, [-1, -1, 0], 10, Some(10));
    // 24 periods on the 24×24 torus bring it back to its starting voxels.
    let mut far = builtin("life-4555-glider");
    let initial = positions(&far);
    far.advance(4 * 24);
    assert_eq!(positions(&far), initial);
}

#[test]
fn rokicki_light_speed_ship() {
    let mut life = builtin("rokicki-ship");
    let populations = assert_spaceship(&mut life, 10, [10, 0, 0], 14, None);
    // Measured here (the source documents only the period and speed).
    assert_eq!(populations, [14, 16, 18, 20, 20, 14, 16, 18, 20, 20]);
    // Also in an unbounded sparse world over many periods.
    let mut sparse = world("3D4,7/5,8", WorldKind::SparseUnbounded, [8, 8, 8]);
    let ship: Vec<(Pos, u8)> = builtin("rokicki-ship").voxels().into_iter().map(|v| (v.pos, 1)).collect();
    sparse.apply(VoxelCommand::SetVoxels(ship)).unwrap();
    let start = positions(&sparse);
    sparse.advance(500);
    assert_eq!(positions(&sparse), shifted(&start, [500, 0, 0]));
}

#[test]
fn gliders_survive_crossing_chunk_and_sign_boundaries() {
    // Run the 5766 glider backwards in direction (-1, -1, 0) by mirroring it,
    // through negative coordinates of the unbounded sparse world.
    let mut sparse = world("5766", WorldKind::SparseUnbounded, [8, 8, 8]);
    let glider: Vec<(Pos, u8)> =
        builtin("life-5766-glider").voxels().into_iter().map(|v| ([-v.pos[0], -v.pos[1], v.pos[2]], 1)).collect();
    sparse.apply(VoxelCommand::SetVoxels(glider)).unwrap();
    let start = positions(&sparse);
    sparse.advance(4 * 30);
    assert_eq!(positions(&sparse), shifted(&start, [-30, -30, 0]));
}
