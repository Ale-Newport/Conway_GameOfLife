//! Shared helpers for the integration tests: an independent naive Life
//! stepper over a sorted cell set, seeded soups and well-known patterns.

use std::collections::{BTreeMap, BTreeSet};

use hashlife::Universe;
use simulation_core::SimRng;

/// A set of live cells.
pub type CellSet = BTreeSet<(i64, i64)>;

/// One generation of an outer-totalistic rule on the unbounded plane,
/// computed by counting neighbours of every live cell (independent of the
/// engine under test).
pub fn naive_step(cells: &CellSet, birth: u16, survive: u16) -> CellSet {
    let mut counts: BTreeMap<(i64, i64), u32> = BTreeMap::new();
    for &(x, y) in cells {
        counts.entry((x, y)).or_insert(0);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx, dy) != (0, 0) {
                    *counts.entry((x + dx, y + dy)).or_insert(0) += 1;
                }
            }
        }
    }
    counts
        .into_iter()
        .filter(|&(p, n)| {
            let mask = if cells.contains(&p) { survive } else { birth };
            mask >> n & 1 == 1
        })
        .map(|(p, _)| p)
        .collect()
}

/// `generations` naive steps.
pub fn naive_run(cells: &CellSet, birth: u16, survive: u16, generations: u64) -> CellSet {
    let mut current = cells.clone();
    for _ in 0..generations {
        current = naive_step(&current, birth, survive);
    }
    current
}

/// A `width × height` random soup at `(x0, y0)` drawn from `SimRng`.
pub fn soup(seed: u64, x0: i64, y0: i64, width: i64, height: i64, density: f64) -> CellSet {
    let mut rng = SimRng::seed_from_u64(seed);
    let mut cells = CellSet::new();
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            if rng.chance(density) {
                cells.insert((x, y));
            }
        }
    }
    cells
}

/// The live cells of a universe as a sorted set.
pub fn cells_of(universe: &Universe) -> CellSet {
    universe.cells().collect()
}

/// A universe holding `cells` under the given masks.
pub fn universe_with(cells: &CellSet, birth: u16, survive: u16) -> Universe {
    let mut universe = Universe::new(birth, survive).expect("test rules are valid");
    universe.insert_cells(cells.iter().copied());
    universe
}

/// `B3/S23` masks.
pub const LIFE: (u16, u16) = (1 << 3, (1 << 2) | (1 << 3));

/// Gosper glider gun (36 cells), top-left at (0, 0).
pub const GOSPER_GUN: [(i64, i64); 36] = [
    (24, 0),
    (22, 1),
    (24, 1),
    (12, 2),
    (13, 2),
    (20, 2),
    (21, 2),
    (34, 2),
    (35, 2),
    (11, 3),
    (15, 3),
    (20, 3),
    (21, 3),
    (34, 3),
    (35, 3),
    (0, 4),
    (1, 4),
    (10, 4),
    (16, 4),
    (20, 4),
    (21, 4),
    (0, 5),
    (1, 5),
    (10, 5),
    (14, 5),
    (16, 5),
    (17, 5),
    (22, 5),
    (24, 5),
    (10, 6),
    (16, 6),
    (24, 6),
    (11, 7),
    (15, 7),
    (12, 8),
    (13, 8),
];

/// A glider moving towards +x, +y (south-east) by one cell every 4 generations.
pub const GLIDER: [(i64, i64); 5] = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];

/// Translates a cell set.
pub fn translated(cells: &CellSet, dx: i64, dy: i64) -> CellSet {
    cells.iter().map(|&(x, y)| (x + dx, y + dy)).collect()
}
