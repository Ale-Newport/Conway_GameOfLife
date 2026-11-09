//! Hashlife must agree exactly with the naive stepper.

use hashlife::{Rule, Universe};

use crate::common::{CellSet, LIFE, cells_of, naive_run, naive_step, soup, universe_with};

/// Rules exercised: Life, HighLife, Seeds, Day & Night, Life without death
/// (S0 present), Replicator.
fn rules() -> Vec<(u16, u16)> {
    ["B3/S23", "B36/S23", "B2/S", "B3678/S34678", "B3/S012345678", "B1357/S1357"]
        .iter()
        .map(|text| {
            let rule = Rule::parse(text).unwrap();
            (rule.birth(), rule.survive())
        })
        .collect()
}

#[test]
fn random_soups_match_generation_by_generation() {
    for seed in 0..6u64 {
        let density = 0.25 + 0.05 * seed as f64;
        let start = soup(seed, -16, -16, 32, 32, density);
        let mut expected = start.clone();
        let mut universe = universe_with(&start, LIFE.0, LIFE.1);
        for generation in 1..=64u64 {
            universe.step_pow2(0).unwrap();
            expected = naive_step(&expected, LIFE.0, LIFE.1);
            assert_eq!(universe.generation(), generation);
            assert_eq!(universe.population(), expected.len() as u64, "seed {seed}, generation {generation}");
            assert_eq!(cells_of(&universe), expected, "seed {seed}, generation {generation}");
        }
    }
}

#[test]
fn arbitrary_advance_matches_for_many_rules() {
    for (r, (birth, survive)) in rules().into_iter().enumerate() {
        let start = soup(100 + r as u64, -10, -12, 24, 20, 0.35);
        let mut previous = 0;
        let mut expected = start.clone();
        for n in [1u64, 2, 3, 5, 7, 13, 31, 37, 63, 64] {
            // Fresh universe advanced by n in one call.
            let mut fresh = universe_with(&start, birth, survive);
            fresh.advance(n).unwrap();
            expected = naive_run(&expected, birth, survive, n - previous);
            previous = n;
            assert_eq!(fresh.generation(), n);
            assert_eq!(cells_of(&fresh), expected, "rule #{r}, n = {n}");
        }
    }
}

#[test]
fn power_of_two_steps_match() {
    let start = soup(7, -20, -20, 40, 40, 0.4);
    for k in 0..=6u32 {
        let mut universe = universe_with(&start, LIFE.0, LIFE.1);
        universe.step_pow2(k).unwrap();
        assert_eq!(cells_of(&universe), naive_run(&start, LIFE.0, LIFE.1, 1 << k), "k = {k}");
    }
}

#[test]
fn interleaved_step_sizes_reuse_memoised_results_correctly() {
    let start = soup(11, -16, -16, 32, 32, 0.4);
    let mut universe = universe_with(&start, LIFE.0, LIFE.1);
    let mut expected = start;
    // Changing k between steps must reuse or recompute results correctly.
    for &k in &[3u32, 0, 5, 1, 2, 4, 0, 6, 3, 3, 1, 5] {
        universe.step_pow2(k).unwrap();
        expected = naive_run(&expected, LIFE.0, LIFE.1, 1 << k);
        assert_eq!(cells_of(&universe), expected, "after a 2^{k} step");
    }
}

#[test]
fn highlife_agrees_with_naive() {
    let highlife = Rule::highlife();
    for seed in 20..24u64 {
        let start = soup(seed, -12, -12, 24, 24, 0.4);
        let mut universe = Universe::with_rule(highlife);
        universe.insert_cells(start.iter().copied());
        let mut expected = start;
        for n in [1u64, 4, 9, 16, 50] {
            universe.advance(n).unwrap();
            expected = naive_run(&expected, highlife.birth(), highlife.survive(), n);
            assert_eq!(cells_of(&universe), expected, "seed {seed}, after +{n}");
        }
    }
}

#[test]
fn far_from_origin_and_across_quadrant_borders() {
    let offsets = [(1i64 << 40, -(1i64 << 40)), (-5, -3), ((1 << 61) - 40, 7), (-(1 << 61) + 40, -(1 << 61) + 40)];
    for (i, &(x0, y0)) in offsets.iter().enumerate() {
        let start = soup(40 + i as u64, x0, y0, 20, 20, 0.45);
        let mut universe = universe_with(&start, LIFE.0, LIFE.1);
        universe.advance(17).unwrap();
        assert_eq!(cells_of(&universe), naive_run(&start, LIFE.0, LIFE.1, 17), "offset {i}");
    }
}

#[test]
fn empty_universe_stays_empty() {
    let mut universe = Universe::conway();
    universe.advance(1 << 50).unwrap();
    assert_eq!(universe.generation(), 1 << 50);
    assert!(universe.is_empty());
    assert_eq!(cells_of(&universe), CellSet::new());
}
