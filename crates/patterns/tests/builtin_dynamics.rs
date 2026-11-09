//! Verifies the dynamics claimed for the embedded patterns with a small,
//! independent reference simulator (unbounded plane, sparse set of cells).
//!
//! The simulator uses `LifeRule::next_state`, so these tests also exercise the
//! rule implementation on real evolutions.

use std::collections::{BTreeSet, HashMap};

use patterns::{LifeRule, Pattern, builtin_patterns};

type Cells = BTreeSet<(i64, i64)>;

fn step(cells: &Cells, rule: &LifeRule) -> Cells {
    let mut counts: HashMap<(i64, i64), u8> = HashMap::new();
    for &(x, y) in cells {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx != 0 || dy != 0 {
                    *counts.entry((x + dx, y + dy)).or_insert(0) += 1;
                }
            }
        }
    }
    // Cells with zero live neighbours never appear in `counts`; that is only
    // correct for rules without B0 and S0, which holds for B3/S23.
    assert!(!rule.has_b0() && !rule.survives(0));
    counts.into_iter().filter(|&(cell, n)| rule.next_state(cells.contains(&cell), n)).map(|(cell, _)| cell).collect()
}

fn run(cells: &Cells, generations: usize) -> Cells {
    let rule = LifeRule::conway();
    (0..generations).fold(cells.clone(), |state, _| step(&state, &rule))
}

fn cells_of(id: &str) -> (Pattern, Cells) {
    let builtin = builtin_patterns().iter().find(|b| b.id == id).unwrap_or_else(|| panic!("no builtin {id}"));
    let pattern = builtin.pattern().unwrap();
    assert_eq!(LifeRule::parse(pattern.rule.as_deref().unwrap()).unwrap(), LifeRule::conway());
    let cells = pattern.live_cells().collect();
    (pattern, cells)
}

fn translate(cells: &Cells, dx: i64, dy: i64) -> Cells {
    cells.iter().map(|&(x, y)| (x + dx, y + dy)).collect()
}

#[test]
fn still_lifes_do_not_change() {
    for id in ["block", "beehive", "loaf", "boat", "tub"] {
        let (_, cells) = cells_of(id);
        assert_eq!(run(&cells, 1), cells, "{id}");
    }
}

#[test]
fn oscillators_have_their_exact_period() {
    for builtin in builtin_patterns().iter().filter(|b| b.kind == "oscillator") {
        let (_, cells) = cells_of(builtin.id);
        let period = builtin.period as usize;
        let mut state = cells.clone();
        for generation in 1..=period {
            state = run(&state, 1);
            if generation < period {
                assert_ne!(state, cells, "{} repeats early at generation {generation}", builtin.id);
            }
        }
        assert_eq!(state, cells, "{} must repeat after {period} generations", builtin.id);
    }
}

#[test]
fn beacon_population_alternates() {
    let (_, cells) = cells_of("beacon");
    assert_eq!(cells.len(), 8);
    assert_eq!(run(&cells, 1).len(), 6, "the touching corners die");
    assert_eq!(run(&cells, 2).len(), 8);
}

#[test]
fn spaceships_move_by_their_displacement() {
    for builtin in builtin_patterns().iter().filter(|b| b.kind == "spaceship") {
        let (_, cells) = cells_of(builtin.id);
        let (dx, dy) = builtin.displacement.unwrap();
        let period = builtin.period as usize;
        for generation in 1..period {
            let state = run(&cells, generation);
            let moved = (-4..=4).any(|ox| (-4..=4).any(|oy| translate(&cells, ox, oy) == state));
            assert!(!moved, "{} has a shorter period ({generation})", builtin.id);
        }
        assert_eq!(run(&cells, period), translate(&cells, dx, dy), "{}", builtin.id);
        assert_eq!(run(&cells, 4 * period), translate(&cells, 4 * dx, 4 * dy), "{}", builtin.id);
    }
}

#[test]
fn gosper_gun_restores_itself_and_emits_a_glider_every_30_generations() {
    let (_, gun) = cells_of("gosper-glider-gun");
    let after = run(&gun, 30);
    assert!(gun.is_subset(&after), "the gun returns to its initial state after 30 generations");
    let emitted: Cells = after.difference(&gun).copied().collect();
    assert_eq!(emitted.len(), 5, "one glider per period");
    assert_eq!(run(&emitted, 4), translate(&emitted, 1, 1), "the emitted object is a glider moving down-right");
    let populations: Vec<usize> = (1..=4).map(|k| run(&gun, 30 * k).len()).collect();
    assert_eq!(populations, vec![41, 46, 51, 56]);
}

#[test]
fn methuselah_lifespans() {
    let (_, diehard) = cells_of("diehard");
    let at_129 = run(&diehard, 129);
    assert!(!at_129.is_empty(), "diehard is still alive at generation 129");
    assert!(run(&at_129, 1).is_empty(), "diehard vanishes at generation 130");

    let (_, r) = cells_of("r-pentomino");
    let at_1103 = run(&r, 1103);
    assert_eq!(at_1103.len(), 116);
    // From 1103 on the population is constant (blinkers keep 3 cells, gliders 5).
    assert_eq!(run(&at_1103, 2).len(), 116);
    assert_ne!(run(&r, 1102).len(), 116, "R-pentomino has not settled at 1102");

    let (_, acorn) = cells_of("acorn");
    let at_5206 = run(&acorn, 5206);
    assert_eq!(at_5206.len(), 633);
    assert_eq!(run(&at_5206, 2).len(), 633);
}
