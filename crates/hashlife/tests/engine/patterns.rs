//! Known pattern behaviour and query correctness.

use hashlife::Universe;
use simulation_core::SimRng;

use crate::common::{CellSet, GLIDER, GOSPER_GUN, LIFE, cells_of, naive_run, soup, translated, universe_with};

fn life_with(cells: &[(i64, i64)]) -> Universe {
    let mut universe = Universe::conway();
    universe.insert_cells(cells.iter().copied());
    universe
}

#[test]
fn glider_moves_one_cell_diagonally_every_four_generations() {
    let glider: CellSet = GLIDER.into_iter().collect();
    let mut universe = life_with(&GLIDER);
    universe.advance(4 * 1000).unwrap();
    assert_eq!(universe.generation(), 4000);
    assert_eq!(universe.population(), 5);
    assert_eq!(cells_of(&universe), translated(&glider, 1000, 1000));
    assert_eq!(universe.bounding_box(), Some((1000, 1000, 1002, 1002)));
}

#[test]
fn glider_after_two_to_the_forty_generations() {
    let glider: CellSet = GLIDER.into_iter().collect();
    let shift = 1i64 << 38; // 2^40 / 4 diagonal moves
    let mut one_step = life_with(&GLIDER);
    one_step.step_pow2(40).unwrap();
    assert_eq!(one_step.generation(), 1 << 40);
    assert_eq!(cells_of(&one_step), translated(&glider, shift, shift));
    // The same jump through `advance` and through a mixture of steps.
    let mut mixed = life_with(&GLIDER);
    mixed.advance((1 << 40) - 4).unwrap();
    mixed.advance(4).unwrap();
    assert_eq!(cells_of(&mixed), translated(&glider, shift, shift));
}

#[test]
fn blinker_has_period_two() {
    let horizontal: CellSet = [(-1, 0), (0, 0), (1, 0)].into_iter().collect();
    let vertical: CellSet = [(0, -1), (0, 0), (0, 1)].into_iter().collect();
    let mut universe = life_with(&[(-1, 0), (0, 0), (1, 0)]);
    for generation in 1..=20u64 {
        universe.step_pow2(0).unwrap();
        let expected = if generation % 2 == 1 { &vertical } else { &horizontal };
        assert_eq!(&cells_of(&universe), expected, "generation {generation}");
    }
    for k in 1..=30u32 {
        universe.step_pow2(k).unwrap();
        assert_eq!(cells_of(&universe), horizontal, "after a 2^{k} step");
    }
}

#[test]
fn gosper_gun_adds_one_glider_every_thirty_generations() {
    let mut universe = life_with(&GOSPER_GUN);
    assert_eq!(universe.population(), 36);
    let mut populations = vec![universe.population()];
    for _ in 0..1200 {
        universe.step_pow2(0).unwrap();
        populations.push(universe.population());
    }
    for t in 120..=1170usize {
        assert_eq!(populations[t + 30], populations[t] + 5, "generations {t} and {}", t + 30);
    }
    // The single-generation run agrees with the naive stepper at the end.
    let gun: CellSet = GOSPER_GUN.into_iter().collect();
    assert_eq!(cells_of(&universe), naive_run(&gun, LIFE.0, LIFE.1, 1200));
    // Far in the future the growth rate is unchanged.
    universe.advance(30 * 100_000 - 1200).unwrap();
    let before = universe.population();
    universe.advance(30).unwrap();
    assert_eq!(universe.population(), before + 5);
}

#[test]
fn methuselahs_reach_their_known_final_populations() {
    let r_pentomino = [(1, 0), (2, 0), (0, 1), (1, 1), (1, 2)];
    let mut universe = life_with(&r_pentomino);
    universe.advance(1103).unwrap();
    assert_eq!(universe.population(), 116);
    let acorn = [(1, 0), (3, 1), (0, 2), (1, 2), (4, 2), (5, 2), (6, 2)];
    let mut universe = life_with(&acorn);
    universe.advance(5206).unwrap();
    assert_eq!(universe.population(), 633);
}

/// A soup evolved for a while, with its cells from the naive stepper.
fn evolved_soup() -> (Universe, CellSet) {
    let start = soup(99, -30, -25, 60, 50, 0.4);
    let mut universe = universe_with(&start, LIFE.0, LIFE.1);
    universe.advance(100).unwrap();
    (universe, naive_run(&start, LIFE.0, LIFE.1, 100))
}

#[test]
fn bounding_box_is_exact() {
    let (universe, cells) = evolved_soup();
    let min_x = cells.iter().map(|c| c.0).min();
    let max_x = cells.iter().map(|c| c.0).max();
    let min_y = cells.iter().map(|c| c.1).min();
    let max_y = cells.iter().map(|c| c.1).max();
    let expected = min_x.zip(min_y).zip(max_x.zip(max_y)).map(|((a, b), (c, d))| (a, b, c, d));
    assert_eq!(universe.bounding_box(), expected);
    assert_eq!(life_with(&[(-7, 9)]).bounding_box(), Some((-7, 9, -7, 9)));
}

#[test]
fn cells_in_rect_matches_a_filter() {
    let (universe, cells) = evolved_soup();
    for &(x0, y0, x1, y1) in &[(-10, -10, 10, 10), (-100, -100, 100, 100), (3, -40, 3, 40), (5, 5, 4, 4)] {
        let mut expected: Vec<(i64, i64)> =
            cells.iter().copied().filter(|&(x, y)| x0 <= x && x <= x1 && y0 <= y && y <= y1).collect();
        expected.sort_unstable_by_key(|&(x, y)| (y, x));
        assert_eq!(universe.cells_in_rect(x0, y0, x1, y1, usize::MAX), expected);
        let limited = universe.cells_in_rect(x0, y0, x1, y1, 7);
        assert_eq!(limited.len(), expected.len().min(7));
        assert!(limited.iter().all(|c| expected.contains(c)));
    }
}

#[test]
fn density_grid_matches_bucketed_counts_and_sums_to_the_population() {
    let (universe, cells) = evolved_soup();
    for k in 0..=6u32 {
        for &(x0, y0) in &[(-64i64, -64i64), (-61, -70), (-40, -33)] {
            let side = (256usize >> k).max(1);
            let grid = universe.density_grid(x0, y0, side, side, k).unwrap();
            let mut expected = vec![0u32; side * side];
            for &(x, y) in &cells {
                let (px, py) = ((x - x0) >> k, (y - y0) >> k);
                if (0..side as i64).contains(&px) && (0..side as i64).contains(&py) {
                    expected[py as usize * side + px as usize] += 1;
                }
            }
            assert_eq!(grid, expected, "k = {k}, origin ({x0}, {y0})");
        }
        // A grid covering the whole pattern sums to the population.
        let total: u64 =
            universe.density_grid(-128, -128, 256 >> k, 256 >> k, k).unwrap().iter().map(|&p| u64::from(p)).sum();
        assert_eq!(total, universe.population(), "k = {k}");
    }
    // Zoomed far out, everything lands in one pixel.
    assert_eq!(universe.density_grid(-(1 << 20), -(1 << 20), 1, 1, 21).unwrap(), vec![cells.len() as u32]);
}

/// One coordinate of a random rectangle corner: near a live cell, near the
/// origin, at or near the ends of the `i64` range, or anywhere.
fn random_coordinate(rng: &mut SimRng, cells: &[i64]) -> i64 {
    match rng.below(4) {
        0 if !cells.is_empty() => cells[rng.index(cells.len())].saturating_add(rng.range_i64(-9, 10)),
        1 => rng.range_i64(-40, 40),
        2 => *rng.choose(&[i64::MIN, i64::MAX, i64::MIN + 17, i64::MAX - 17, 1 << 40, -(1 << 40)]).expect("non-empty"),
        _ => rng.next_u64() as i64,
    }
}

#[test]
fn rectangle_populations_match_brute_force_counts() {
    let mut rng = SimRng::seed_from_u64(0x00C0_FFEE_7EC7);
    // Top-left corners of 24×24 soups: across the origin, across the 2^40
    // and 2^62 node boundaries, and in three corners of the i64 plane.
    let corners = [
        (-12, -12),
        (-(1 << 40) - 11, (1 << 40) - 13),
        (-(1 << 62) - 9, 3),
        (i64::MIN, i64::MIN),
        (i64::MAX - 24, i64::MIN + 5),
        (i64::MAX - 24, i64::MAX - 24),
    ];
    for trial in 0..40u64 {
        // Half of the universes start from an evolved soup, whose ash
        // repeats small nodes all over it, half from a random tile repeated
        // on an aligned 6×6 grid: identical quadtree nodes that one rectangle
        // cuts with different edges.
        let mut cells = if trial % 2 == 0 {
            naive_run(&soup(trial, -30, -20, 60, 40, 0.4), LIFE.0, LIFE.1, rng.below(80))
        } else {
            let side = 8 << rng.below(3);
            let tile = soup(rng.next_u64(), 0, 0, side, side, rng.range_f64(0.2, 0.7));
            (0..36)
                .flat_map(|i| {
                    let (dx, dy) = ((i % 6 - 3) * side, (i / 6 - 3) * side);
                    tile.iter().map(move |&(x, y)| (x + dx, y + dy))
                })
                .collect()
        };
        for _ in 0..1 + rng.below(3) {
            let (x0, y0) = corners[rng.index(corners.len())];
            cells.extend(soup(rng.next_u64(), x0, y0, 24, 24, rng.range_f64(0.2, 0.7)));
        }
        if rng.chance(0.5) {
            cells.extend([(i64::MIN, i64::MAX), (i64::MAX, i64::MAX), (i64::MAX, i64::MIN)]);
        }
        let universe = universe_with(&cells, LIFE.0, LIFE.1);
        let xs: Vec<i64> = cells.iter().map(|c| c.0).collect();
        let ys: Vec<i64> = cells.iter().map(|c| c.1).collect();
        for query in 0..60 {
            let (mut x0, mut x1) = (random_coordinate(&mut rng, &xs), random_coordinate(&mut rng, &xs));
            let (mut y0, mut y1) = (random_coordinate(&mut rng, &ys), random_coordinate(&mut rng, &ys));
            // Mostly ordered corners; reversed ones must count nothing.
            if rng.chance(0.9) {
                (x0, x1, y0, y1) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
            }
            let expected = cells.iter().filter(|&&(x, y)| x0 <= x && x <= x1 && y0 <= y && y <= y1).count();
            assert_eq!(
                universe.population_in_rect(x0, y0, x1, y1),
                expected as u128,
                "trial {trial}, query {query}: [{x0}, {x1}] × [{y0}, {y1}]"
            );
        }
        assert_eq!(universe.population_in_rect(i64::MIN, i64::MIN, i64::MAX, i64::MAX), cells.len() as u128);
    }
}

#[test]
fn cells_iterator_lists_every_live_cell_once() {
    let (universe, cells) = evolved_soup();
    let listed: Vec<(i64, i64)> = universe.cells().collect();
    assert_eq!(listed.len(), cells.len());
    assert_eq!(listed.into_iter().collect::<CellSet>(), cells);
}
