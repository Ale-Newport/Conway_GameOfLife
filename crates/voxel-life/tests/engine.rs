//! Engine correctness: neighbour counts and whole generations against the
//! brute-force reference, hand-built rule checks, decay, wrap-around, ages
//! and dense/sparse agreement.

mod common;

use common::*;
use simulation_core::Simulation;
use voxel_life::{Neighbourhood, Pos, VoxelCommand, WorldKind};

/// One engine configuration exercised against the reference: world kind
/// and size, the matching reference edges, and the soup box `lo + [0, side)`.
struct Setup {
    kind: WorldKind,
    size: [u32; 3],
    edges: Edges,
    lo: Pos,
    side: [i32; 3],
}

fn finite(kind: WorldKind, size: [u32; 3]) -> Setup {
    let side = size.map(|s| s as i32);
    let edges = if kind == WorldKind::DenseTorus { Edges::Torus(side) } else { Edges::Bounded(side) };
    Setup { kind, size, edges, lo: [0, 0, 0], side }
}

fn configurations() -> Vec<Setup> {
    vec![
        finite(WorldKind::DenseBounded, [10, 9, 8]),
        finite(WorldKind::DenseTorus, [10, 9, 8]),
        // Axes shorter than 3: one voxel is reached through several offsets.
        finite(WorldKind::DenseTorus, [1, 2, 5]),
        finite(WorldKind::DenseTorus, [2, 3, 2]),
        finite(WorldKind::DenseBounded, [1, 7, 2]),
        // Sparse worlds crossing chunk boundaries (16) and negative chunks.
        finite(WorldKind::SparseBounded, [20, 18, 17]),
        Setup {
            kind: WorldKind::SparseUnbounded,
            size: [8, 8, 8],
            edges: Edges::Unbounded,
            lo: [-10, 6, -3],
            side: [20, 20, 20],
        },
    ]
}

#[test]
fn neighbour_counts_match_brute_force() {
    // With rule S{k}/B{k}/C5 a voxel is in state 1 next generation iff it is
    // empty or alive and has exactly k alive neighbours, so the state-1 set
    // reveals every count. Dying voxels (states 2-4) are in the soup and
    // must not be counted.
    for Setup { kind, size, edges, lo, side } in configurations() {
        let start = soup(7, lo, side, 0.45, 5);
        for neighbourhood in [Neighbourhood::Moore, Neighbourhood::VonNeumann] {
            let letter = if neighbourhood == Neighbourhood::Moore { 'M' } else { 'N' };
            for k in 1..=neighbourhood.size() {
                let text = format!("{k}/{k}/5/{letter}");
                let mut life = world(&text, kind, size);
                load(&mut life, &start);
                life.step();
                let got: Vec<Pos> = cells(&life).into_iter().filter(|(_, s)| *s == 1).map(|(p, _)| p).collect();
                let reference = reference_step(&start, &rule(&text), edges);
                let expected: Vec<Pos> = reference.into_iter().filter(|(_, s)| *s == 1).map(|(p, _)| p).collect();
                for p in &expected {
                    assert_eq!(reference_count(&start, &rule(&text), *p, edges), k);
                }
                assert_eq!(got, expected, "{kind:?} {size:?} rule {text}");
            }
        }
    }
}

#[test]
fn generations_match_brute_force() {
    let rules = [
        "5766",
        "4555",
        "4/4/5/M",
        "9-26/5-7,12-13,15/5/M",
        "2,6,9/4,6,8-9/10/M",
        "1-2/1,3/5/N",
        "0-6/1,3/2/N",
        "/1/2/M",
        "3,5,7,9,11,15,17,19,21,23-24,26/3,6,8-9,11,14-17,19,24/7/M",
    ];
    for Setup { kind, size, edges, lo, side } in configurations() {
        for text in rules {
            let r = rule(text);
            let mut expected = soup(11, lo, side, 0.35, r.states());
            let mut life = world(text, kind, size);
            load(&mut life, &expected);
            for generation in 1..=6 {
                expected = reference_step(&expected, &r, edges);
                life.step();
                assert_eq!(cells(&life), expected, "{kind:?} {size:?} rule {text} generation {generation}");
            }
        }
    }
}

#[test]
fn birth_and_survival_happen_exactly_at_listed_counts() {
    // Survive {4, 7}, born {5, 8, 13, 26}: check every possible count for a
    // centre voxel that is empty or alive, with exactly k alive neighbours.
    let r = rule("4,7/5,8,13,26/2/M");
    for k in 0..=26u32 {
        for centre_alive in [false, true] {
            let mut life = world("4,7/5,8,13,26/2/M", WorldKind::DenseBounded, [5, 5, 5]);
            let mut voxels: Vec<(Pos, u8)> = Neighbourhood::Moore.offsets()[..k as usize]
                .iter()
                .map(|d| ([2 + d[0], 2 + d[1], 2 + d[2]], 1))
                .collect();
            if centre_alive {
                voxels.push(([2, 2, 2], 1));
            }
            life.apply(VoxelCommand::SetVoxels(voxels)).unwrap();
            life.step();
            let expected = if centre_alive { r.survive().contains(k) } else { r.birth().contains(k) };
            assert_eq!(life.state_at([2, 2, 2]) == 1, expected, "k = {k}, alive = {centre_alive}");
        }
    }
    // Von Neumann: diagonal voxels are not neighbours.
    let mut life = world("/2/2/N", WorldKind::DenseBounded, [3, 3, 3]);
    life.apply(VoxelCommand::SetVoxels(vec![([0, 0, 0], 1), ([2, 2, 2], 1), ([1, 1, 0], 1), ([1, 1, 2], 1)])).unwrap();
    life.step();
    assert_eq!(life.state_at([1, 1, 1]), 1, "two face neighbours");
    assert_eq!(life.state_at([0, 0, 1]), 0, "only corner/edge contacts");
}

#[test]
fn decay_and_dying_voxels() {
    // A lone voxel of 445 decays 1 → 2 → 3 → 4 → 0 (research vector 1, 1, 1, 0).
    let mut life = world("4/4/5/M", WorldKind::DenseBounded, [3, 3, 3]);
    life.apply(VoxelCommand::SetVoxel { pos: [1, 1, 1], state: 1 }).unwrap();
    for expected in [2, 3, 4, 0] {
        life.step();
        assert_eq!(life.state_at([1, 1, 1]), expected);
    }
    // Dying voxels are not counted: 4 dying + 3 alive neighbours < B4.
    let mut life = world("4/4/5/M", WorldKind::DenseBounded, [5, 5, 5]);
    let offsets = Neighbourhood::Moore.offsets();
    let voxels =
        offsets[..7].iter().enumerate().map(|(i, d)| ([2 + d[0], 2 + d[1], 2 + d[2]], if i < 4 { 2 } else { 1 }));
    life.apply(VoxelCommand::SetVoxels(voxels.collect())).unwrap();
    life.step();
    assert_eq!(life.state_at([2, 2, 2]), 0);
    // A dying voxel with a birth count around it keeps decaying.
    let mut life = world("4/4/5/M", WorldKind::DenseBounded, [5, 5, 5]);
    let mut voxels: Vec<(Pos, u8)> = offsets[..4].iter().map(|d| ([2 + d[0], 2 + d[1], 2 + d[2]], 1)).collect();
    voxels.push(([2, 2, 2], 2));
    life.apply(VoxelCommand::SetVoxels(voxels)).unwrap();
    life.step();
    assert_eq!(life.state_at([2, 2, 2]), 3, "no rebirth before reaching 0");
}

#[test]
fn torus_wraps_every_face() {
    // A voxel at the corner sees alive voxels across all three faces.
    let mut life = world("/3/2/M", WorldKind::DenseTorus, [6, 7, 8]);
    life.apply(VoxelCommand::SetVoxels(vec![([5, 0, 0], 1), ([0, 6, 0], 1), ([0, 0, 7], 1)])).unwrap();
    life.step();
    assert_eq!(life.state_at([0, 0, 0]), 1, "born from three wrapped neighbours");
    let mut bounded = world("/3/2/M", WorldKind::DenseBounded, [6, 7, 8]);
    bounded.apply(VoxelCommand::SetVoxels(vec![([5, 0, 0], 1), ([0, 6, 0], 1), ([0, 0, 7], 1)])).unwrap();
    bounded.step();
    assert_eq!(bounded.state_at([0, 0, 0]), 0, "no wrapping in a bounded world");
}

#[test]
fn dense_and_sparse_agree_including_ages() {
    let start = soup(3, [4, 4, 4], [32, 32, 32], 0.3, 10);
    let mut dense = world("4-7/6-8/10/M", WorldKind::DenseBounded, [40, 40, 40]);
    let mut sparse = world("4-7/6-8/10/M", WorldKind::SparseBounded, [40, 40, 40]);
    load(&mut dense, &start);
    load(&mut sparse, &start);
    for generation in 0..25 {
        assert_eq!(dense.voxels(), sparse.voxels(), "generation {generation}");
        assert_eq!(dense.stats(), sparse.stats(), "generation {generation}");
        dense.step();
        sparse.step();
    }
}

#[test]
fn ages_count_generations_since_birth() {
    // A 2×2×2 block is a still life of 5766 (each voxel has 7 neighbours).
    let mut life = world("5766", WorldKind::SparseUnbounded, [8, 8, 8]);
    let block: Vec<(Pos, u8)> = (0..8).map(|i| ([i & 1, (i >> 1) & 1, (i >> 2) & 1], 1)).collect();
    life.apply(VoxelCommand::SetVoxels(block)).unwrap();
    for generation in 0..5u16 {
        assert!(life.voxels().iter().all(|v| v.age == generation), "generation {generation}");
        life.step();
    }
    // Decaying voxels keep ageing; newly born voxels start at 0.
    let mut life = world("4/4/5/M", WorldKind::DenseBounded, [3, 3, 3]);
    life.apply(VoxelCommand::SetVoxel { pos: [1, 1, 1], state: 1 }).unwrap();
    life.advance(2);
    assert_eq!(life.voxel([1, 1, 1]).age, 2);
    assert_eq!(life.voxel([1, 1, 1]).state, 3);
}

#[test]
fn births_and_deaths_match_the_reference() {
    let r = rule("9-26/5-7,12-13,15/5/M");
    let mut expected = soup(5, [0, 0, 0], [12, 12, 12], 0.4, 5);
    let mut life = world("9-26/5-7,12-13,15/5/M", WorldKind::DenseBounded, [12, 12, 12]);
    load(&mut life, &expected);
    for _ in 0..5 {
        let next = reference_step(&expected, &r, Edges::Bounded([12, 12, 12]));
        let births = next.iter().filter(|(p, s)| **s == 1 && !expected.contains_key(*p)).count() as u64;
        let deaths = expected.iter().filter(|(p, s)| **s == 1 && next.get(*p) != Some(&1)).count() as u64;
        life.step();
        assert_eq!((life.stats().births(), life.stats().deaths()), (births, deaths));
        let mut per_state = vec![0u64; 5];
        for s in next.values() {
            per_state[usize::from(*s)] += 1;
        }
        assert_eq!(life.stats().counts(), per_state.as_slice());
        expected = next;
    }
}

#[test]
fn sparse_world_halts_at_the_chunk_limit() {
    // 1,300 isolated alive voxels, 48 apart: 27 candidate chunks each.
    let mut life = world("5766", WorldKind::SparseUnbounded, [8, 8, 8]);
    let voxels: Vec<(Pos, u8)> =
        (0..1300).map(|i| ([(i % 20) * 48, ((i / 20) % 20) * 48, (i / 400) * 48], 1)).collect();
    life.apply(VoxelCommand::SetVoxels(voxels)).unwrap();
    let before = life.voxels();
    life.step();
    assert!(life.is_finished());
    assert!(life.halt_reason().is_some_and(|r| r.contains("chunks")));
    assert_eq!(life.tick(), 0);
    assert_eq!(life.voxels(), before, "the world is left unchanged");
    life.apply(VoxelCommand::Clear).unwrap();
    assert!(!life.is_finished());
}

#[test]
fn edits_cannot_exceed_the_sparse_chunk_limit() {
    let limit = voxel_life::MAX_SPARSE_CHUNKS as i32;
    let is_limit = |r: Result<(), simulation_core::SimError>| matches!(r, Err(simulation_core::SimError::LimitExceeded(ref m)) if m.contains("chunks"));
    // One voxel per chunk: exactly the limit is accepted, one more is not.
    let mut life = world("5766", WorldKind::SparseUnbounded, [8, 8, 8]);
    let scattered = |n: i32, y: i32| -> Vec<(Pos, u8)> { (0..n).map(|i| ([i * 16, y, 0], 1)).collect() };
    assert!(is_limit(life.apply(VoxelCommand::SetVoxels(scattered(limit + 1, 0)))));
    assert_eq!(life.stats().population(), 0, "a rejected batch changes nothing");
    life.apply(VoxelCommand::SetVoxels(scattered(limit - 1, 0))).unwrap();
    // Voxels in already stored chunks and empty voxels need no new chunk.
    life.apply(VoxelCommand::SetVoxels(scattered(limit - 1, 5))).unwrap();
    life.apply(VoxelCommand::SetVoxel { pos: [-16, 0, 0], state: 1 }).unwrap();
    assert!(is_limit(life.apply(VoxelCommand::SetVoxel { pos: [-32, 0, 0], state: 1 })));
    life.apply(VoxelCommand::SetVoxel { pos: [-32, 0, 0], state: 0 }).unwrap();
    life.apply(VoxelCommand::FillBox { min: [-64, 0, 0], max: [-48, 0, 0], state: 0 }).unwrap();
    assert!(is_limit(life.apply(VoxelCommand::FillBox { min: [-64, 0, 0], max: [-48, 0, 0], state: 1 })));
    assert_eq!(life.stats().population(), 2 * (limit as u64 - 1) + 1);

    // A FillBox of 2^24 voxels in a thin slab touches 4096 × 16 chunks.
    let mut slab = world("5766", WorldKind::SparseUnbounded, [8, 8, 8]);
    assert!(is_limit(slab.apply(VoxelCommand::FillBox { min: [0, 0, 0], max: [65_535, 255, 0], state: 1 })));
    slab.apply(VoxelCommand::FillBox { min: [0, 0, 0], max: [255, 255, 255], state: 1 }).unwrap();
    assert_eq!(slab.stats().population(), 1 << 24);

    // Randomize draws the pending cube in the current world: a cube that is
    // valid for the pending size can exceed the limit in the current one.
    let mut wide = world("5766", WorldKind::SparseBounded, [65_536, 256, 1]);
    wide.set_param("size_x", simulation_core::ParamValue::Int(1)).unwrap();
    wide.set_param("cube_side", simulation_core::ParamValue::Int(65_536)).unwrap();
    assert!(is_limit(wide.apply(VoxelCommand::Randomize { seed: Some(1) })));
}
