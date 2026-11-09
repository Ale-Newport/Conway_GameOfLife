//! Deterministic Softology test vectors from the research catalogue (§7.4):
//! a single state-1 voxel (or a 2×2×1 block) in empty space, populations of
//! all non-empty voxels for generations 1…12. Each vector is checked on the
//! dense bounded engine (41³, large enough for 12 generations at light
//! speed) and on the unbounded sparse engine.

mod common;

use common::*;
use simulation_core::Simulation;
use voxel_life::{Pos, VoxelCommand, VoxelLife, WorldKind};

fn engines(rule_text: &str, seed: &[Pos]) -> Vec<VoxelLife> {
    [(WorldKind::DenseBounded, [41, 41, 41]), (WorldKind::SparseUnbounded, [8, 8, 8])]
        .into_iter()
        .map(|(kind, size)| {
            let mut life = world(rule_text, kind, size);
            let centre = if kind == WorldKind::DenseBounded { 20 } else { 0 };
            let voxels = seed.iter().map(|p| ([p[0] + centre, p[1] + centre, p[2] + centre], 1)).collect();
            life.apply(VoxelCommand::SetVoxels(voxels)).unwrap();
            life
        })
        .collect()
}

fn assert_populations(rule_text: &str, seed: &[Pos], expected: &[u64], alive: Option<&[u64]>) {
    for mut life in engines(rule_text, seed) {
        let kind = life.config().kind;
        for (i, &population) in expected.iter().enumerate() {
            life.step();
            assert_eq!(life.stats().population(), population, "{rule_text} {kind:?} generation {}", i + 1);
            if let Some(alive) = alive {
                assert_eq!(life.stats().alive(), alive[i], "{rule_text} {kind:?} state-1 count, generation {}", i + 1);
            }
        }
    }
}

const SINGLE: &[Pos] = &[[0, 0, 0]];

#[test]
fn crystal_growth_1() {
    assert_populations("0-6/1,3/2/N", SINGLE, &[7, 13, 43, 49, 79, 133, 259, 313, 343, 397, 523, 625], None);
}

#[test]
fn crystal_growth_2_and_von_neumann_builder() {
    let populations = [7, 13, 43, 48, 72, 120, 216, 264, 264, 264, 264, 312];
    let alive = [6, 6, 30, 6, 30, 54, 126, 54, 30, 54, 126, 102];
    assert_populations("1-2/1,3/5/N", SINGLE, &populations, Some(&alive));
    assert_populations("1-3/1,4-5/5/N", SINGLE, &populations, None);
}

#[test]
fn diamond_growth() {
    assert_populations("5-6/1-3/7/N", SINGLE, &[7, 25, 63, 129, 231, 376, 568, 808, 1096, 1432, 1816, 2248], None);
}

#[test]
fn single_point_replication() {
    assert_populations("/1/2/M", SINGLE, &[26, 8, 208, 8, 208, 64, 1664, 8, 208, 64, 1664, 64], None);
    // After 2^k generations exactly 8 voxels at (±2^k, ±2^k, ±2^k).
    let mut life = builtin("single-point-replication");
    let centre = life.voxels()[0].pos;
    let mut generation = 0;
    for k in 1..=5 {
        let target = 1u64 << k;
        life.advance(target - generation);
        generation = target;
        let r = target as i32;
        let mut expected = Vec::new();
        for dz in [-r, r] {
            for dy in [-r, r] {
                for dx in [-r, r] {
                    expected.push([centre[0] + dx, centre[1] + dy, centre[2] + dz]);
                }
            }
        }
        expected.sort_by_key(|p| (p[2], p[1], p[0]));
        let got: Vec<Pos> = life.voxels().into_iter().map(|v| v.pos).collect();
        assert_eq!(got, expected, "k = {k}");
    }
}

#[test]
fn builder_2() {
    assert_populations("5-7/1/2/M", SINGLE, &[26, 16, 152, 88, 176, 400, 920, 544, 512, 1264, 1568, 2440], None);
}

#[test]
fn pulse_waves() {
    let expected = [27, 71, 235, 327, 827, 1087, 2643, 2879, 4674, 5940, 8828, 10596];
    assert_populations("3/1-3/10/M", SINGLE, &expected, None);
}

#[test]
fn lone_445_voxel_decays() {
    assert_populations("4/4/5/M", SINGLE, &[1, 1, 1, 0, 0], None);
}

#[test]
fn brain_3d_from_a_2x2x1_block() {
    let block = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0]];
    assert_populations("/4/2/M", &block, &[8, 16, 12, 24, 64, 32, 16, 32, 24, 48, 128, 64], None);
}

#[test]
fn crystal_growth_scenario_matches_the_vector() {
    let mut life = builtin("crystal-growth-1");
    let mut populations = Vec::new();
    for _ in 0..12 {
        life.step();
        populations.push(life.stats().population());
    }
    assert_eq!(populations, [7, 13, 43, 49, 79, 133, 259, 313, 343, 397, 523, 625]);
}
