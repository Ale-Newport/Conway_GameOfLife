//! Property test: every engine computes exactly the same generations as the
//! naive reference stepper (and therefore as every other engine), on random
//! soups and random rules, for every topology each engine supports.

use std::collections::BTreeSet;

use cellular_automata::engine::reference::reference_step;
use cellular_automata::view::{GridViewRequest, TexelGrid};
use cellular_automata::{CellRect, EngineKind, GenStats, LifeEngine, World, create_engine, switch_engine};
use patterns::LifeRule;
use simulation_core::{SimRng, Topology};

fn random_rule(rng: &mut SimRng, allow_b0: bool) -> LifeRule {
    loop {
        let birth = (rng.next_u64() & 0x1FF) as u16;
        let survive = (rng.next_u64() & 0x1FF) as u16;
        if allow_b0 || birth & 1 == 0 {
            // Favour Conway now and then so its fast kernel path is covered.
            if rng.chance(0.15) {
                return LifeRule::conway();
            }
            return LifeRule::new(birth, survive).expect("masks within 0..=8");
        }
    }
}

fn soup(rng: &mut SimRng, region: CellRect, density: f64) -> Vec<(i64, i64)> {
    let mut cells = Vec::new();
    for y in region.y0..=region.y1 {
        for x in region.x0..=region.x1 {
            if rng.f64() < density {
                cells.push((x, y));
            }
        }
    }
    cells
}

fn sorted(set: &BTreeSet<(i64, i64)>) -> Vec<(i64, i64)> {
    let mut v: Vec<_> = set.iter().copied().collect();
    v.sort_unstable_by_key(|&(x, y)| (y, x));
    v
}

fn reference_stats(before: &BTreeSet<(i64, i64)>, after: &BTreeSet<(i64, i64)>) -> GenStats {
    GenStats { births: after.difference(before).count() as u64, deaths: before.difference(after).count() as u64 }
}

/// Runs all `kinds` side by side with the reference for `generations`.
fn check(world: World, rule: LifeRule, cells: &[(i64, i64)], kinds: &[EngineKind], generations: usize, label: &str) {
    let mut engines: Vec<Box<dyn LifeEngine>> =
        kinds.iter().map(|&k| create_engine(k, world, rule).expect("supported combination")).collect();
    for e in &mut engines {
        e.load_cells(cells);
    }
    let mut reference: BTreeSet<(i64, i64)> = engines[0].export_cells().into_iter().collect();
    for generation in 1..=generations {
        let next = reference_step(&world, rule, &reference);
        let expected_stats = reference_stats(&reference, &next);
        reference = next;
        let expected = sorted(&reference);
        for e in &mut engines {
            let stats = e.step().expect("no limit reached");
            if let Some(stats) = stats {
                assert_eq!(stats, expected_stats, "{label}: {} births/deaths at generation {generation}", e.name());
            }
            assert_eq!(
                e.population(),
                expected.len() as u64,
                "{label}: {} population at generation {generation}",
                e.name()
            );
            assert_eq!(
                e.export_cells(),
                expected,
                "{label}: {} cells at generation {generation} (rule {rule})",
                e.name()
            );
        }
    }
    // Bounding boxes agree with the cell set.
    let bbox = CellRect::bounding(reference.iter().copied());
    for e in &engines {
        assert_eq!(e.bounding_box(), bbox, "{label}: {} bounding box", e.name());
    }
}

#[test]
fn finite_engines_agree_on_every_topology() {
    let mut rng = SimRng::seed_from_u64(0xC0FFEE);
    let topologies = [Topology::Bounded, Topology::Torus, Topology::KleinBottle, Topology::CrossSurface];
    let sizes =
        [(1, 1), (1, 7), (2, 3), (3, 3), (5, 9), (17, 13), (63, 4), (64, 5), (65, 6), (70, 9), (130, 11), (128, 64)];
    for (trial, &(w, h)) in sizes.iter().cycle().take(48).enumerate() {
        let topology = topologies[trial % topologies.len()];
        let world = World::new(topology, w, h).expect("valid size");
        let rule = random_rule(&mut rng, true);
        let density = rng.range_f64(0.1, 0.6);
        let cells = soup(&mut rng, world.home(), density);
        let label = format!("trial {trial}: {topology:?} {w}×{h}");
        check(world, rule, &cells, &[EngineKind::Dense, EngineKind::Bit], 40, &label);
    }
}

#[test]
fn conway_on_a_large_bit_world_uses_parallel_bands_correctly() {
    // 300 × 256 cells = 1280 words: crosses PAR thresholds in both engines.
    let mut rng = SimRng::seed_from_u64(42);
    for topology in [Topology::Torus, Topology::KleinBottle, Topology::CrossSurface, Topology::Bounded] {
        let world = World::new(topology, 300, 256).expect("valid");
        let cells = soup(&mut rng, world.home(), 0.35);
        let mut dense = create_engine(EngineKind::Dense, world, LifeRule::conway()).unwrap();
        let mut bit = create_engine(EngineKind::Bit, world, LifeRule::conway()).unwrap();
        dense.load_cells(&cells);
        bit.load_cells(&cells);
        let mut reference: BTreeSet<(i64, i64)> = cells.iter().copied().collect();
        for generation in 0..12 {
            reference = reference_step(&world, LifeRule::conway(), &reference);
            let d = dense.step().unwrap();
            let b = bit.step().unwrap();
            assert_eq!(d, b, "{topology:?} stats at generation {generation}");
            let expected = sorted(&reference);
            assert_eq!(dense.export_cells(), expected, "{topology:?} dense generation {generation}");
            assert_eq!(bit.export_cells(), expected, "{topology:?} bit generation {generation}");
        }
    }
}

#[test]
fn infinite_engines_agree() {
    let mut rng = SimRng::seed_from_u64(0xBADC0DE);
    let world = World::new(Topology::Infinite, 64, 64).expect("valid");
    let offsets = [(0, 0), (-30, -30), (60, -5), (-64, 63), (1 << 40, -(1 << 41)), (-(1 << 50), 1 << 50)];
    for trial in 0..36 {
        let rule = random_rule(&mut rng, false);
        let (ox, oy) = offsets[trial % offsets.len()];
        let side = rng.range_i64(3, 30);
        let region = CellRect::new(ox, oy, ox + side - 1, oy + rng.range_i64(3, 30) - 1);
        let density = rng.range_f64(0.15, 0.55);
        let cells = soup(&mut rng, region, density);
        // Rules with B1 grow at light speed; keep their runs shorter.
        let generations = if rule.birth & 0b10 != 0 { 15 } else { 45 };
        let label = format!("trial {trial} at ({ox}, {oy})");
        check(world, rule, &cells, &[EngineKind::Dense, EngineKind::Sparse, EngineKind::Hashlife], generations, &label);
    }
}

#[test]
fn hashlife_jumps_match_sparse_generation_by_generation() {
    let mut rng = SimRng::seed_from_u64(77);
    let world = World::new(Topology::Infinite, 64, 64).expect("valid");
    for trial in 0..6 {
        let rule = if trial % 2 == 0 { LifeRule::conway() } else { LifeRule::highlife() };
        let cells = soup(&mut rng, CellRect::new(-20, -20, 20, 20), 0.35);
        let mut sparse = create_engine(EngineKind::Sparse, world, rule).unwrap();
        let mut hash = create_engine(EngineKind::Hashlife, world, rule).unwrap();
        sparse.load_cells(&cells);
        hash.load_cells(&cells);
        for jump in [1u64, 2, 5, 8, 13, 64, 100] {
            sparse.advance(jump).unwrap();
            hash.advance(jump).unwrap();
            assert_eq!(hash.export_cells(), sparse.export_cells(), "trial {trial}, after a jump of {jump}");
            assert_eq!(hash.population(), sparse.population());
        }
    }
}

#[test]
fn switching_engines_keeps_the_exact_state() {
    let mut rng = SimRng::seed_from_u64(5);
    let world = World::new(Topology::Infinite, 64, 64).expect("valid");
    let cells = soup(&mut rng, CellRect::new(0, 0, 40, 40), 0.4);
    let mut engine = create_engine(EngineKind::Sparse, world, LifeRule::conway()).unwrap();
    engine.load_cells(&cells);
    let mut reference: BTreeSet<(i64, i64)> = cells.iter().copied().collect();
    for (round, kind) in
        [EngineKind::Hashlife, EngineKind::Dense, EngineKind::Sparse, EngineKind::Hashlife].into_iter().enumerate()
    {
        engine = switch_engine(engine, kind).unwrap();
        for _ in 0..10 {
            engine.step().unwrap();
            reference = reference_step(&world, LifeRule::conway(), &reference);
        }
        assert_eq!(engine.export_cells(), sorted(&reference), "round {round} on {kind:?}");
    }
    let torus = World::new(Topology::Torus, 77, 33).expect("valid");
    let mut finite = create_engine(EngineKind::Dense, torus, LifeRule::day_and_night()).unwrap();
    finite.load_cells(&soup(&mut rng, torus.home(), 0.5));
    let before = finite.export_cells();
    let bit = switch_engine(finite, EngineKind::Bit).unwrap();
    assert_eq!(bit.export_cells(), before);
    assert!(switch_engine(bit, EngineKind::Sparse).is_err(), "sparse cannot run a torus");
}

#[test]
fn level_of_detail_counts_agree_between_engines() {
    let mut rng = SimRng::seed_from_u64(99);
    let world = World::new(Topology::Infinite, 64, 64).expect("valid");
    let cells = soup(&mut rng, CellRect::new(-150, -90, 170, 110), 0.3);
    let kinds = [EngineKind::Dense, EngineKind::Sparse, EngineKind::Hashlife];
    let engines: Vec<Box<dyn LifeEngine>> = kinds
        .iter()
        .map(|&k| {
            let mut e = create_engine(k, world, LifeRule::conway()).unwrap();
            e.load_cells(&cells);
            e
        })
        .collect();
    for _ in 0..40 {
        let x0 = rng.range_i64(-200, 100);
        let y0 = rng.range_i64(-200, 100);
        let req = GridViewRequest {
            x0,
            y0,
            x1: x0 + rng.range_i64(0, 400),
            y1: y0 + rng.range_i64(0, 400),
            max_width: rng.range_i64(1, 300) as u32,
            max_height: rng.range_i64(1, 300) as u32,
        };
        let grid = TexelGrid::plan(&req, None);
        let mut expected = vec![0u32; grid.len()];
        let clip = grid.clip.expect("infinite plane");
        for &(x, y) in &cells {
            if clip.contains(x, y) {
                expected[grid.index(x, y)] += 1;
            }
        }
        for e in &engines {
            let mut got = vec![0u32; grid.len()];
            e.count_live(&grid, &mut got);
            assert_eq!(got, expected, "{} for {req:?}", e.name());
        }
    }
    // Finite: the bit engine against the dense engine on a Klein bottle.
    let klein = World::new(Topology::KleinBottle, 200, 150).expect("valid");
    let cells = soup(&mut rng, klein.home(), 0.4);
    let mut dense = create_engine(EngineKind::Dense, klein, LifeRule::conway()).unwrap();
    let mut bit = create_engine(EngineKind::Bit, klein, LifeRule::conway()).unwrap();
    dense.load_cells(&cells);
    bit.load_cells(&cells);
    for max in [1u32, 3, 7, 64, 500] {
        let req = GridViewRequest { x0: -30, y0: -10, x1: 230, y1: 170, max_width: max, max_height: max };
        let grid = TexelGrid::plan(&req, klein.bounds());
        let (mut a, mut b) = (vec![0u32; grid.len()], vec![0u32; grid.len()]);
        dense.count_live(&grid, &mut a);
        bit.count_live(&grid, &mut b);
        assert_eq!(a, b, "budget {max}");
        assert_eq!(a.iter().map(|&c| u64::from(c)).sum::<u64>(), cells.len() as u64);
    }
}

#[test]
fn region_listings_agree_between_engines_and_stop_at_the_limit() {
    let mut rng = SimRng::seed_from_u64(17);
    let plane = World::new(Topology::Infinite, 64, 64).expect("valid");
    let torus = World::new(Topology::Torus, 150, 90).expect("valid");
    let cells = soup(&mut rng, torus.home(), 0.3);
    for (world, kind) in [
        (plane, EngineKind::Dense),
        (plane, EngineKind::Sparse),
        (plane, EngineKind::Hashlife),
        (torus, EngineKind::Dense),
        (torus, EngineKind::Bit),
    ] {
        let mut e = create_engine(kind, world, LifeRule::conway()).unwrap();
        e.load_cells(&cells);
        for _ in 0..20 {
            let (x0, y0) = (rng.range_i64(-20, 140), rng.range_i64(-20, 80));
            let rect = CellRect::new(x0, y0, x0 + rng.range_i64(0, 100), y0 + rng.range_i64(0, 60));
            let all = sorted(&cells.iter().copied().filter(|&(x, y)| rect.contains(x, y)).collect());
            assert_eq!(e.cells_in_rect(rect, usize::MAX), all, "{} in {rect:?}", e.name());
            for limit in [0, 1, all.len() / 2, all.len()] {
                let some = e.cells_in_rect(rect, limit);
                assert_eq!(some.len(), limit.min(all.len()), "{} in {rect:?}, limit {limit}", e.name());
                assert!(some.is_sorted_by_key(|&(x, y)| (y, x)) && some.iter().all(|c| all.contains(c)));
            }
        }
    }
}

#[test]
fn long_runs_agree() {
    // 300 generations against the reference on a small Klein bottle, Conway and HighLife.
    let mut rng = SimRng::seed_from_u64(2718);
    for rule in [LifeRule::conway(), LifeRule::highlife()] {
        let world = World::new(Topology::KleinBottle, 37, 29).expect("valid");
        let cells = soup(&mut rng, world.home(), 0.4);
        check(world, rule, &cells, &[EngineKind::Dense, EngineKind::Bit], 300, &format!("long Klein run, {rule}"));
    }
    // 2000 generations of a soup on the infinite plane: the three unbounded engines stay identical.
    let world = World::new(Topology::Infinite, 64, 64).expect("valid");
    let cells = soup(&mut rng, CellRect::new(0, 0, 47, 47), 0.4);
    let mut engines: Vec<Box<dyn LifeEngine>> = [EngineKind::Dense, EngineKind::Sparse, EngineKind::Hashlife]
        .into_iter()
        .map(|k| {
            let mut e = create_engine(k, world, LifeRule::conway()).unwrap();
            e.load_cells(&cells);
            e
        })
        .collect();
    for block in 0..20 {
        for e in &mut engines {
            e.advance(100).unwrap();
        }
        let reference = engines[1].export_cells();
        for e in &engines {
            assert_eq!(e.export_cells(), reference, "{} after {} generations", e.name(), 100 * (block + 1));
        }
    }
}
