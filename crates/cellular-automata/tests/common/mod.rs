//! Shared helpers for the integration tests: running engines, normalising
//! cell sets, splitting patterns into objects and recognising spaceships.

#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use cellular_automata::{EngineKind, LifeEngine, World, create_engine};
use patterns::{Catalog, LifeRule, Pattern};
use simulation_core::Topology;

pub type Cells = Vec<(i64, i64)>;

/// The workspace asset catalog (`assets/patterns/catalog.toml`).
pub fn catalog() -> Catalog {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/patterns/catalog.toml");
    Catalog::load(&path).unwrap_or_else(|e| panic!("asset catalog at {}: {e}", path.display()))
}

/// Live cells of a catalog pattern.
pub fn catalog_cells(catalog: &Catalog, id: &str) -> Cells {
    let entry = catalog.by_id(id).unwrap_or_else(|| panic!("catalog entry {id}"));
    let pattern = catalog.load_pattern(entry).unwrap_or_else(|e| panic!("{id}: {e}"));
    sorted(pattern.live_cells().collect())
}

/// Live cells of a pattern given as text.
pub fn text_cells(text: &str) -> Cells {
    let p: Pattern = patterns::parse_any(text, None).expect("valid pattern text");
    sorted(p.live_cells().collect())
}

pub fn sorted(mut cells: Cells) -> Cells {
    cells.sort_unstable_by_key(|&(x, y)| (y, x));
    cells.dedup();
    cells
}

/// An engine on the infinite plane loaded with `cells`.
pub fn engine(kind: EngineKind, rule: LifeRule, cells: &[(i64, i64)]) -> Box<dyn LifeEngine> {
    let world = World::new(Topology::Infinite, 256, 256).expect("valid");
    let mut e = create_engine(kind, world, rule).expect("engine");
    e.load_cells(cells);
    e
}

/// Cells after `generations` (Sparse engine).
pub fn run(rule: LifeRule, cells: &[(i64, i64)], generations: u64) -> Cells {
    let mut e = engine(EngineKind::Sparse, rule, cells);
    e.advance(generations).expect("in range");
    e.export_cells()
}

pub fn translate(cells: &[(i64, i64)], dx: i64, dy: i64) -> Cells {
    sorted(cells.iter().map(|&(x, y)| (x + dx, y + dy)).collect())
}

/// Shape (cells moved so the bounding box starts at the origin) and offset.
pub fn normalize(cells: &[(i64, i64)]) -> (Cells, (i64, i64)) {
    let x0 = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let y0 = cells.iter().map(|c| c.1).min().unwrap_or(0);
    (translate(cells, -x0, -y0), (x0, y0))
}

/// The eight symmetries of the square applied to a shape (normalised).
pub fn orientations(cells: &[(i64, i64)]) -> Vec<Cells> {
    let mut out = Vec::new();
    for flip in [false, true] {
        for turns in 0..4 {
            let t: Cells = cells
                .iter()
                .map(|&(x, y)| {
                    let (mut a, mut b) = if flip { (-x, y) } else { (x, y) };
                    for _ in 0..turns {
                        (a, b) = (-b, a);
                    }
                    (a, b)
                })
                .collect();
            out.push(normalize(&t).0);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Objects: groups of cells connected through cells at Chebyshev distance
/// at most 2 (so gliders and spaceships with one-cell gaps stay whole).
pub fn components(cells: &[(i64, i64)]) -> Vec<Cells> {
    let index: HashMap<(i64, i64), usize> = cells.iter().enumerate().map(|(i, &c)| (c, i)).collect();
    let mut parent: Vec<usize> = (0..cells.len()).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut j = i;
        while p[j] != r {
            let next = p[j];
            p[j] = r;
            j = next;
        }
        r
    }
    for (i, &(x, y)) in cells.iter().enumerate() {
        for dy in -2..=2 {
            for dx in -2..=2 {
                if let Some(&j) = index.get(&(x + dx, y + dy)) {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
            }
        }
    }
    let mut groups: HashMap<usize, Cells> = HashMap::new();
    for (i, &c) in cells.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(c);
    }
    let mut out: Vec<Cells> = groups.into_values().map(sorted).collect();
    out.sort();
    out
}

/// A spaceship: every phase in every orientation, as normalised shapes.
pub struct Ship {
    pub name: &'static str,
    pub period: u64,
    pub shapes: BTreeSet<Cells>,
}

/// Life spaceships used to recognise gun and rake output.
pub fn ship_library(catalog: &Catalog) -> Vec<Ship> {
    let ships = [("glider", 4u64), ("lwss", 4), ("mwss", 4), ("hwss", 4), ("loafer", 7)];
    ships
        .iter()
        .map(|&(name, period)| {
            let mut cells = catalog_cells(catalog, name);
            let mut shapes = BTreeSet::new();
            for _ in 0..period {
                for o in orientations(&cells) {
                    shapes.insert(o);
                }
                cells = run(LifeRule::conway(), &cells, 1);
            }
            Ship { name, period, shapes }
        })
        .collect()
}

/// Name of the spaceship `component` is, if any.
pub fn classify<'a>(ships: &'a [Ship], component: &[(i64, i64)]) -> Option<&'a Ship> {
    let shape = normalize(component).0;
    ships.iter().find(|s| s.shapes.contains(&shape))
}

/// The cells with every free-flying spaceship removed.
pub fn without_ships(ships: &[Ship], cells: &[(i64, i64)]) -> Cells {
    let mut out = Vec::new();
    for c in components(cells) {
        if classify(ships, &c).is_none() {
            out.extend(c);
        }
    }
    sorted(out)
}

/// Displacement of a free spaceship after `period` generations.
pub fn ship_velocity(component: &[(i64, i64)], period: u64) -> (i64, i64) {
    let later = run(LifeRule::conway(), component, period);
    let (_, (x0, y0)) = normalize(component);
    let (_, (x1, y1)) = normalize(&later);
    (x1 - x0, y1 - y0)
}
