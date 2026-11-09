//! Randomised differential testing: random rules, positions and sequences of
//! operations (steps of every kind, edits, collections, rule changes,
//! macrocell round trips), each checked against the naive stepper.

use hashlife::{Rule, Universe, macrocell};
use simulation_core::SimRng;

use crate::common::{CellSet, cells_of, naive_run, naive_step, soup};

/// A random rule without B0.
fn random_rule(rng: &mut SimRng) -> Rule {
    // Truncation keeps the nine count bits; bit 0 of birth is cleared.
    let birth = (rng.next_u64() as u16) & 0x1FE;
    let survive = (rng.next_u64() as u16) & 0x1FF;
    Rule::new(birth, survive).expect("B0 cleared, bits 0..=8 only")
}

/// Origins that put the soup across quadtree boundaries of many levels and
/// close to the edge of the steppable range (±2^61).
fn random_origin(rng: &mut SimRng) -> (i64, i64) {
    let candidates = [
        (-6, -6),
        (-1, -13),
        (1 << 20, -(1 << 33)),
        ((1 << 61) - 700, -(1 << 61) + 600),
        (-(1 << 61) + 600, (1 << 61) - 720),
        (-(1 << 40) - 5, (1 << 40) - 7),
    ];
    let (x, y) = candidates[rng.index(candidates.len())];
    (x + rng.range_i64(-8, 8), y + rng.range_i64(-8, 8))
}

fn rule_masks(rule: Rule) -> (u16, u16) {
    (rule.birth(), rule.survive())
}

/// Keeps cells within a window around the origin so that explosive rules do
/// not make the naive stepper (or the steppable range) the bottleneck.
fn crop(cells: &CellSet, (x0, y0): (i64, i64), radius: i64) -> CellSet {
    cells.iter().copied().filter(|&(x, y)| (x - x0).abs() <= radius && (y - y0).abs() <= radius).collect()
}

#[test]
fn random_operation_sequences_match_the_naive_stepper() {
    let mut rng = SimRng::seed_from_u64(0x5EED_F00D);
    for trial in 0..48u64 {
        let mut rule = random_rule(&mut rng);
        let origin = random_origin(&mut rng);
        let start = soup(1000 + trial, origin.0, origin.1, 12, 12, rng.range_f64(0.2, 0.6));
        let mut universe = Universe::with_rule(rule);
        universe.insert_cells(start.iter().copied());
        if rng.chance(0.5) {
            universe.set_node_limit(0);
        }
        let mut expected = start;
        for op in 0..12 {
            let context = format!("trial {trial}, op {op}, rule {rule}, origin {origin:?}");
            match rng.below(9) {
                0 | 1 => {
                    let k = rng.below(6) as u32;
                    universe.step_pow2(k).unwrap();
                    expected = naive_run(&expected, rule.birth(), rule.survive(), 1 << k);
                }
                2 | 3 => {
                    let n = 1 + rng.below(24);
                    universe.advance(n).unwrap();
                    expected = naive_run(&expected, rule.birth(), rule.survive(), n);
                }
                4 => {
                    let (x, y) = (origin.0 + rng.range_i64(-20, 20), origin.1 + rng.range_i64(-20, 20));
                    let alive = rng.chance(0.6);
                    universe.set_cell(x, y, alive);
                    if alive {
                        expected.insert((x, y));
                    } else {
                        expected.remove(&(x, y));
                    }
                }
                5 => {
                    let (x0, y0) = (origin.0 + rng.range_i64(-30, 10), origin.1 + rng.range_i64(-30, 10));
                    let (x1, y1) = (x0 + rng.range_i64(0, 25), y0 + rng.range_i64(0, 25));
                    universe.clear_rect(x0, y0, x1, y1);
                    expected.retain(|&(x, y)| !(x0..=x1).contains(&x) || !(y0..=y1).contains(&y));
                }
                6 => {
                    rule = random_rule(&mut rng);
                    universe.set_rule(rule);
                }
                7 => {
                    if rng.chance(0.5) {
                        universe.garbage_collect();
                    } else {
                        universe.clear_memo();
                    }
                }
                _ => {
                    let generation = universe.generation();
                    let text = macrocell::write(&universe);
                    if rng.chance(0.5) {
                        universe = macrocell::read(&text).unwrap();
                    } else {
                        // Into the same store: existing nodes and memo are reused.
                        macrocell::read_into(&mut universe, &text).unwrap();
                    }
                    assert_eq!(universe.rule(), rule, "{context}");
                    assert_eq!(universe.generation(), generation, "{context}");
                }
            }
            assert_eq!(universe.population(), expected.len() as u64, "{context}");
            assert_eq!(cells_of(&universe), expected, "{context}");
            // Explosive rules: crop both sides identically (after checking) so
            // runs stay small.
            if expected.len() > 250 {
                expected = crop(&expected, origin, 11);
                universe.clear();
                universe.insert_cells(expected.iter().copied());
            }
        }
    }
}

#[test]
fn every_rule_without_b0_round_trips_through_text_and_steps_like_the_reference() {
    // Parsing the canonical text gives the same rule back, for all 2^17 rules.
    for birth in (0..512u16).step_by(2) {
        for survive in 0..512u16 {
            let rule = Rule::new(birth, survive).unwrap();
            assert_eq!(Rule::parse(&rule.to_string()).unwrap(), rule);
        }
    }
    // A single 5-cell seed for a sample of rules: one generation at a time.
    let mut rng = SimRng::seed_from_u64(77);
    let seed: CellSet = [(0, 0), (1, 0), (2, 0), (0, 1), (1, 2)].into_iter().collect();
    for _ in 0..200 {
        let rule = random_rule(&mut rng);
        let (birth, survive) = rule_masks(rule);
        let mut universe = Universe::with_rule(rule);
        universe.insert_cells(seed.iter().copied());
        let mut expected = seed.clone();
        for generation in 1..=6 {
            universe.step_pow2(0).unwrap();
            expected = naive_step(&expected, birth, survive);
            assert_eq!(cells_of(&universe), expected, "rule {rule}, generation {generation}");
        }
    }
}
