//! Behaviour of the Life-like rules of the CA Laboratory, checked against
//! the verified facts of `docs/research/automata-catalog.md` §2.

mod common;

use std::collections::BTreeSet;

use cellular_automata::wolfram::{Boundary, eca_step};
use cellular_automata::{EngineKind, World, create_engine};
use common::{Cells, engine, normalize, run, sorted, text_cells, translate};
use patterns::LifeRule;
use simulation_core::{SimRng, Topology};

fn rule(s: &str) -> LifeRule {
    LifeRule::parse(s).expect("valid rule")
}

const REPLICATOR: &str = "x = 5, y = 5, rule = B36/S23\n2b3o$bo2bo$o3bo$o2bo$3o!";

/// Live positions of rule 90 after `n` steps from a single cell at 0.
fn rule90_positions(n: u64) -> Vec<i64> {
    let width = 2 * n as usize + 3;
    let centre = width / 2;
    let mut row = vec![0u64; width.div_ceil(64)];
    row[centre / 64] |= 1 << (centre % 64);
    for _ in 0..n {
        row = eca_step(90, &row, width, Boundary::Fixed0);
    }
    (0..width).filter(|&x| row[x / 64] >> (x % 64) & 1 == 1).map(|x| x as i64 - centre as i64).collect()
}

#[test]
fn highlife_replicator_follows_rule_90() {
    let highlife = rule("B36/S23");
    let r = text_cells(REPLICATOR);
    let at = |g: u64| run(highlife, &r, g);
    let mut gen12: Cells = translate(&r, -2, -2);
    gen12.extend(translate(&r, 2, 2));
    assert_eq!(at(12), sorted(gen12), "two copies at generation 12");
    for n in 1..=16u64 {
        let mut expected = Vec::new();
        for k in rule90_positions(n) {
            expected.extend(translate(&r, 2 * k, 2 * k));
        }
        let expected = sorted(expected);
        assert_eq!(at(12 * n), expected, "generation {} is the rule-90 superposition", 12 * n);
        assert_eq!(expected.len() as u32, 12 * 2u32.pow(n.count_ones()), "2^popcount({n}) copies");
    }
    let mut e = engine(EngineKind::Sparse, highlife, &r);
    let mut pops = vec![e.population()];
    for _ in 0..24 {
        e.step().unwrap();
        pops.push(e.population());
    }
    assert_eq!(
        pops,
        [12, 20, 17, 30, 20, 28, 34, 38, 26, 17, 16, 22, 24, 40, 34, 60, 31, 39, 43, 42, 34, 24, 24, 22, 24]
    );
}

fn random_torus(seed: u64, size: u32, density: f64) -> (World, Cells) {
    let world = World::new(Topology::Torus, size, size).unwrap();
    let mut rng = SimRng::seed_from_u64(seed);
    let mut cells = Vec::new();
    for y in 0..i64::from(size) {
        for x in 0..i64::from(size) {
            if rng.f64() < density {
                cells.push((x, y));
            }
        }
    }
    (world, cells)
}

fn torus_step(world: World, r: LifeRule, cells: &[(i64, i64)], generations: u64) -> Cells {
    let mut e = create_engine(EngineKind::Bit, world, r).unwrap();
    e.load_cells(cells);
    e.advance(generations).unwrap();
    e.export_cells()
}

fn complement(world: World, cells: &[(i64, i64)]) -> Cells {
    let live: BTreeSet<(i64, i64)> = cells.iter().copied().collect();
    let mut out = Vec::new();
    for y in 0..i64::from(world.height) {
        for x in 0..i64::from(world.width) {
            if !live.contains(&(x, y)) {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn self_complementary_rules_commute_with_inversion_and_life_does_not() {
    for (name, r, symmetric) in [
        ("Day & Night", "B3678/S34678", true),
        ("Anneal", "B4678/S35678", true),
        ("Majority", "B5678/S45678", true),
        ("Fredkin", "B1357/S02468", true),
        ("Life", "B3/S23", false),
    ] {
        let (world, cells) = random_torus(3, 64, 0.5);
        let a = complement(world, &torus_step(world, rule(r), &cells, 1));
        let b = torus_step(world, rule(r), &complement(world, &cells), 1);
        assert_eq!(a == b, symmetric, "{name}: step(¬X) == ¬step(X)");
    }
}

#[test]
fn day_and_night_rocket() {
    let rocket = text_cells(
        "x = 43, y = 13, rule = B3678/S34678\n7b2o$8bo$4bo3bo3b2o$3b2ob8o9b2o$2b11obo3b2ob6o13b2o$b13o2b11o7bo4b3o$\
         26o10b5obo$b13o2b11o7bo4b3o$2b11obo3b2ob6o13b2o$3b2ob8o9b2o$4bo3bo3b2o$8bo$7b2o!",
    );
    assert_eq!(rocket.len(), 170);
    let dn = rule("B3678/S34678");
    assert_eq!(run(dn, &rocket, 40), translate(&rocket, -20, 0));
    for g in 1..40 {
        assert_ne!(normalize(&run(dn, &rocket, g)).0, normalize(&rocket).0, "no shorter period ({g})");
    }
}

#[test]
fn seeds_never_keeps_a_cell_alive_and_has_light_speed_photons() {
    let seeds = rule("B2/S");
    let (world, cells) = random_torus(9, 64, 0.3);
    let mut e = create_engine(EngineKind::Bit, world, seeds).unwrap();
    e.load_cells(&cells);
    let mut previous: BTreeSet<(i64, i64)> = cells.into_iter().collect();
    for _ in 0..20 {
        e.step().unwrap();
        let now: BTreeSet<(i64, i64)> = e.export_cells().into_iter().collect();
        assert!(now.is_disjoint(&previous), "no cell is alive in two consecutive generations");
        previous = now;
    }
    let duoplet = text_cells("x = 2, y = 2, rule = B2/S\no$bo!");
    assert_ne!(run(seeds, &duoplet, 1), duoplet);
    assert_eq!(run(seeds, &duoplet, 2), duoplet, "duoplet has period 2");
    let photon = text_cells("x = 4, y = 3, rule = B2/S\nb2o2$o2bo!");
    for g in 1..=5 {
        assert_eq!(run(seeds, &photon, g), translate(&photon, 0, -(g as i64)), "photon moves (0, −1) per generation");
    }
    let domino = text_cells("2o!");
    let pops: Vec<usize> = (1..=7).map(|g| run(seeds, &domino, g).len()).collect();
    assert_eq!(pops, [4, 6, 8, 14, 8, 8, 8]);
}

#[test]
fn life_without_death_never_loses_cells() {
    let lwod = rule("B3/S012345678");
    let (world, cells) = random_torus(4, 64, 0.1);
    let mut e = create_engine(EngineKind::Dense, world, lwod).unwrap();
    e.load_cells(&cells);
    let mut previous: BTreeSet<(i64, i64)> = cells.into_iter().collect();
    for _ in 0..50 {
        e.step().unwrap();
        let now: BTreeSet<(i64, i64)> = e.export_cells().into_iter().collect();
        assert!(previous.is_subset(&now), "cells never die");
        previous = now;
    }
}

#[test]
fn two_by_two_glider_and_block_lattice() {
    let two = rule("B36/S125");
    let glider = text_cells("x = 5, y = 4, rule = B36/S125\n3bo$obo2$2b3o!");
    assert_eq!(run(two, &glider, 8), translate(&glider, -1, 1), "c/8 glider: period 8, displacement (−1, +1)");
    // A pattern of 2×2 blocks on the even lattice is made of blocks on the odd lattice one generation later.
    let mut rng = SimRng::seed_from_u64(12);
    let mut blocks = Vec::new();
    for by in 0..10i64 {
        for bx in 0..10i64 {
            if rng.chance(0.4) {
                blocks.extend([(2 * bx, 2 * by), (2 * bx + 1, 2 * by), (2 * bx, 2 * by + 1), (2 * bx + 1, 2 * by + 1)]);
            }
        }
    }
    let mut cells = blocks;
    for g in 1..=12 {
        cells = run(two, &cells, 1);
        let offset = g % 2;
        let set: BTreeSet<(i64, i64)> = cells.iter().copied().collect();
        for &(x, y) in &cells {
            let (bx, by) = ((x - offset).div_euclid(2) * 2 + offset, (y - offset).div_euclid(2) * 2 + offset);
            for c in [(bx, by), (bx + 1, by), (bx, by + 1), (bx + 1, by + 1)] {
                assert!(set.contains(&c), "generation {g}: blocks on the lattice offset by {offset}");
            }
        }
    }
}

#[test]
fn replicator_and_fredkin_make_copies() {
    for (r, copies) in [("B1357/S1357", 8usize), ("B1357/S02468", 9)] {
        for k in 1..=5 {
            let d = 1i64 << k;
            let mut expected = vec![(-d, -d), (0, -d), (d, -d), (-d, 0), (d, 0), (-d, d), (0, d), (d, d)];
            if copies == 9 {
                expected.push((0, 0));
            }
            assert_eq!(run(rule(r), &[(0, 0)], d as u64), sorted(expected), "{r}: single cell after {d} generations");
        }
        // An arbitrary small pattern becomes disjoint copies of itself.
        let p = vec![(0, 0), (1, 0), (2, 1), (0, 2), (1, 2)];
        let mut expected = Vec::new();
        for (dx, dy) in [(-8, -8), (0, -8), (8, -8), (-8, 0), (8, 0), (-8, 8), (0, 8), (8, 8)] {
            expected.extend(translate(&p, dx, dy));
        }
        if copies == 9 {
            expected.extend(p.iter().copied());
        }
        assert_eq!(run(rule(r), &p, 8), sorted(expected), "{r}: copies after 8 generations");
    }
}

#[test]
fn maze_wickstretcher_grows_linearly() {
    let maze = rule("B3/S12345");
    let wick = text_cells("x = 6, y = 8, rule = B3/S12345\n2o$b3o$bob2o$3b3o$3b2o$bobo$b3o$2o!");
    for k in 0..6i64 {
        let cells = run(maze, &wick, 12 * (k as u64 + 1));
        let max_x = cells.iter().map(|c| c.0).max().unwrap();
        let min_x = cells.iter().map(|c| c.0).min().unwrap();
        assert_eq!((min_x, max_x), (0, 9 + 4 * k), "x-extent after {} generations", 12 * (k + 1));
        assert_eq!(cells.len() as i64, 37 + 16 * k);
    }
}

#[test]
fn morley_jellyfish_and_puffer() {
    let morley = rule("B368/S245");
    let jelly = text_cells("x = 3, y = 3, rule = B368/S245\n3o$b2o$2bo!");
    assert_eq!(run(morley, &jelly, 7), translate(&jelly, -1, 1), "c/7 diagonal");
    let puffer = text_cells("x = 2, y = 6, rule = B368/S245\no$bo$bo$bo$bo$o!");
    let mut e = engine(EngineKind::Hashlife, morley, &puffer);
    for k in 1..=6i64 {
        e.advance(170).unwrap();
        let cells = e.export_cells();
        assert_eq!(cells.iter().map(|c| c.0).min(), Some(-13 * k), "leftmost x at generation {}", 170 * k);
        assert_eq!(cells.len() as i64, 12 + 8 * k);
    }
}

#[test]
fn generations_rule_with_two_states_is_the_life_like_rule() {
    use cellular_automata::{
        GenerationsConfig, GenerationsSim, LifeConfig, LifeMode, LifeSim, PaintCommand, PatternLibrary,
    };
    use simulation_core::Simulation;
    let (_, cells) = random_torus(5, 20, 0.35);
    let mut g = GenerationsSim::new(
        &GenerationsConfig {
            rule: "23/3/2".into(),
            topology: Topology::Torus,
            width: 20,
            height: 20,
            track_activity: false,
            init: vec![],
        },
        1,
        &PatternLibrary::builtin(),
    )
    .unwrap();
    let mut l = LifeSim::new(
        &LifeConfig { topology: Topology::Torus, width: 20, height: 20, init: vec![], ..LifeConfig::default() },
        1,
        &PatternLibrary::builtin(),
        LifeMode::Lab,
    )
    .unwrap();
    for &(x, y) in &cells {
        g.apply(PaintCommand::SetCell { x, y, state: 1 }).unwrap();
        l.apply(cellular_automata::LifeCommand::SetCell { x, y, alive: true }).unwrap();
    }
    for generation in 0..30 {
        let gc: Vec<(i64, i64)> = g.cells().into_iter().map(|(x, y, _)| (x, y)).collect();
        assert_eq!(gc, l.live_cells(), "generation {generation}");
        g.step();
        l.step();
    }
}
