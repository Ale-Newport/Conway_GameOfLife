//! Shared helpers: worlds built from the public API and a brute-force
//! reference implementation written directly from the rule definition
//! (neighbour offsets, per-voxel loop), independent of the engines'
//! separable sums, padding and chunking.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use simulation_core::{SimRng, Simulation};
use voxel_life::{Evidence, InitKind, InitSpec, Pos, Rule3D, Scenario, VoxelCommand, VoxelLife, WorldKind, scenarios};

/// Voxel states by position (non-empty voxels only).
pub type Cells = BTreeMap<Pos, u8>;

pub fn rule(text: &str) -> Rule3D {
    Rule3D::parse(text).unwrap_or_else(|e| panic!("rule `{text}`: {e}"))
}

/// A scenario with an empty initial state.
pub fn empty_scenario(rule_text: &str, kind: WorldKind, size: [u32; 3]) -> Scenario {
    Scenario {
        id: "test".into(),
        name: "Test".into(),
        description: String::new(),
        seed: 1,
        rule: rule(rule_text),
        world: kind,
        size,
        init: InitSpec { kind: InitKind::Empty, ..InitSpec::default() },
        evidence: Evidence::Unverified,
        reference: String::new(),
    }
}

/// An empty world.
pub fn world(rule_text: &str, kind: WorldKind, size: [u32; 3]) -> VoxelLife {
    VoxelLife::new(empty_scenario(rule_text, kind, size)).unwrap_or_else(|e| panic!("world: {e}"))
}

/// A built-in scenario.
pub fn builtin(id: &str) -> VoxelLife {
    let scenario = scenarios::builtin_by_id(id).unwrap_or_else(|e| panic!("{e}"));
    VoxelLife::new(scenario).unwrap_or_else(|e| panic!("{e}"))
}

/// Writes `cells` into the world.
pub fn load(life: &mut VoxelLife, cells: &Cells) {
    let voxels = cells.iter().map(|(p, s)| (*p, *s)).collect();
    life.apply(VoxelCommand::SetVoxels(voxels)).unwrap_or_else(|e| panic!("load: {e}"));
}

/// The world's non-empty voxels.
pub fn cells(life: &VoxelLife) -> Cells {
    life.voxels().into_iter().map(|v| (v.pos, v.state)).collect()
}

/// Positions of the non-empty voxels.
pub fn positions(life: &VoxelLife) -> BTreeSet<Pos> {
    life.voxels().into_iter().map(|v| v.pos).collect()
}

/// Random voxels in `[lo, lo + side)³` with states in `1..states`.
pub fn soup(seed: u64, lo: Pos, side: [i32; 3], density: f64, states: u16) -> Cells {
    let mut rng = SimRng::seed_from_u64(seed);
    let mut out = Cells::new();
    for z in 0..side[2] {
        for y in 0..side[1] {
            for x in 0..side[0] {
                if rng.chance(density) {
                    let state = 1 + rng.below(u64::from(states - 1)) as u8;
                    out.insert([lo[0] + x, lo[1] + y, lo[2] + z], state);
                }
            }
        }
    }
    out
}

/// Edge behaviour of the reference.
#[derive(Clone, Copy)]
pub enum Edges {
    Bounded([i32; 3]),
    Torus([i32; 3]),
    Unbounded,
}

/// Neighbour position under `edges`, or `None` outside a bounded world.
fn neighbour(pos: Pos, d: [i32; 3], edges: Edges) -> Option<Pos> {
    let n = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
    match edges {
        Edges::Unbounded => Some(n),
        Edges::Bounded(size) => (0..3).all(|a| n[a] >= 0 && n[a] < size[a]).then_some(n),
        Edges::Torus(size) => Some([0, 1, 2].map(|a| n[a].rem_euclid(size[a]))),
    }
}

/// Alive (state-1) neighbours of `pos`, one per neighbour offset.
pub fn reference_count(cells: &Cells, rule: &Rule3D, pos: Pos, edges: Edges) -> u32 {
    rule.neighbourhood()
        .offsets()
        .iter()
        .filter_map(|d| neighbour(pos, *d, edges))
        .filter(|n| cells.get(n) == Some(&1))
        .count() as u32
}

/// One generation computed voxel by voxel from the definition.
pub fn reference_step(cells: &Cells, rule: &Rule3D, edges: Edges) -> Cells {
    let candidates: BTreeSet<Pos> = match edges {
        Edges::Bounded(size) | Edges::Torus(size) => {
            let mut all = BTreeSet::new();
            for z in 0..size[2] {
                for y in 0..size[1] {
                    for x in 0..size[0] {
                        all.insert([x, y, z]);
                    }
                }
            }
            all
        }
        Edges::Unbounded => cells
            .keys()
            .flat_map(|p| rule.neighbourhood().offsets().iter().map(move |d| [p[0] + d[0], p[1] + d[1], p[2] + d[2]]))
            .chain(cells.keys().copied())
            .collect(),
    };
    let mut next = Cells::new();
    for pos in candidates {
        let state = cells.get(&pos).copied().unwrap_or(0);
        let new = rule.next_state(state, reference_count(cells, rule, pos, edges));
        if new != 0 {
            next.insert(pos, new);
        }
    }
    next
}

/// Positions translated by `d`.
pub fn shifted(set: &BTreeSet<Pos>, d: [i32; 3]) -> BTreeSet<Pos> {
    set.iter().map(|p| [p[0] + d[0], p[1] + d[1], p[2] + d[2]]).collect()
}
