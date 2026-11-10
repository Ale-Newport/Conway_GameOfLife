//! Exact verifications: the physical B3/S23 construction is run for whole
//! metagenerations with Hashlife and its readouts are checked against the
//! golden values of the research note (bgolly), against Golly's own
//! metapixel demos and against the macro rule.
//!
//! Hashlife makes most of these runs take well under a second, so they are
//! part of the normal suite. The 36-run rule-bit matrix, the per-generation
//! transition scan and the two large Golly demos (4096 and 4512 tiles) are
//! ignored by default and run with
//!
//! ```text
//! cargo test -p metapixel --profile verify -- --ignored
//! ```
//!
//! Each test prints its wall time (`--nocapture`); measured values are
//! recorded in `docs/OTCA_METAPIXEL.md`.

mod common;

use std::time::Instant;

use metapixel::readout::{TileGeometry, core_counts, tile_populations};
use metapixel::spec::{PERIOD, READOUT_ON_COUNT, READOUT_ON_STEADY_MIN};
use metapixel::{
    Direction, MacroGrid, MacroModel, MetapixelSim, Programming, TileReading, read_golly_tiling, scenario,
};
use simulation_core::{SimRng, Simulation};

/// Runs `metagens` metagenerations and returns the exact readouts (rows).
fn run_readouts(sim: &mut MetapixelSim, metagens: u64) -> Vec<Vec<String>> {
    sim.advance(metagens * PERIOD);
    sim.history()
        .records()
        .map(|r| {
            assert!(r.ambiguous.is_empty(), "metagen {}: ambiguous tiles {:?}", r.metagen, r.ambiguous);
            r.exact.to_rows()
        })
        .collect()
}

fn timed<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let out = f();
    eprintln!("[timing] {name}: {:.2} s", started.elapsed().as_secs_f64());
    out
}

/// One row of the golden table: generation multiple k, population, SHA-256.
type Golden = (u64, u64, &'static str);

const BLINKER_P: (u64, &str) = (330_244, "4e4bfc8180dd75f9b9565a3298343657247801f3b3589ed9447ed0775f645242");
const BLINKER_2P: (u64, &str) = (330_268, "309afbb13d1a03d2dd2fe3e22ef4be00a387106c7fb6da80c1c0595e771eff3f");
const BLINKER_3P: (u64, &str) = (330_244, "77a05b9ac8ebbcd9fee39eab2ef1fdfdab9f2a7e717b8912a8988ac285abe56d");
const BLINKER_4P: (u64, &str) = (330_220, "ba4826bf393200c2de688853d3a580a6b87cd89395e026bb5a444a0b7789e0f1");
const SINGLE_DEAD: (u64, &str) = (22_815, "7d5de0a54891a27aa2709665c8690735d66622ea8176951a3d3212384179200b");
const BLOCK_STEADY: (u64, &str) = (259_832, "21d3c6b0922d1ede4baa65a2bcfab7bf0f83f240947221c14775d56d35aa5318");

/// Runs to every `t = k·P` of `golden` and checks population and hash.
fn check_golden(sim: &mut MetapixelSim, golden: &[Golden], name: &str) {
    for &(k, population, hash) in golden {
        sim.advance(k * PERIOD - sim.tick());
        assert_eq!(sim.universe().population(), population, "{name}: population at t = {k}P");
        assert_eq!(common::cell_hash(sim.universe()), hash, "{name}: SHA-256 at t = {k}P");
    }
}

#[test]
fn golden_table_of_the_research_note_is_reproduced() {
    timed("golden table: blinker, single tile, block to t = 8P", || {
        let mut sim = common::sim(&["...", "OOO", "..."], "B3/S23", "moore");
        assert_eq!(sim.universe().population(), 342_076);
        assert_eq!(
            common::cell_hash(sim.universe()),
            "cf4530b73d4fff3232ad87ba999a2b687ee31066476df403ab3f71774cc8987b"
        );
        let rows: Vec<Golden> = [BLINKER_P, BLINKER_2P, BLINKER_3P, BLINKER_4P]
            .iter()
            .cycle()
            .take(8)
            .enumerate()
            .map(|(i, &(p, h))| (i as u64 + 1, p, h))
            .collect();
        check_golden(&mut sim, &rows, "blinker (period 4P from t = P)");
        let horizontal = ["...", "OOO", "..."];
        let vertical = [".O.", ".O.", ".O."];
        for record in sim.history().records() {
            let expected: &[&str] = if record.metagen % 2 == 0 { &horizontal } else { &vertical };
            assert_eq!(
                record.exact.to_rows(),
                expected,
                "metagen {} read at t = {}",
                record.metagen,
                record.generation
            );
            assert_eq!(record.generation, (record.metagen + 1) * PERIOD);
            assert!(record.matches_prediction());
        }
        assert_eq!(sim.history().total_readouts(), 8);

        let mut single = common::sim(&["O"], "B3/S23", "moore");
        assert_eq!(
            common::cell_hash(single.universe()),
            "d278172c42df6acd4b92e9ba484196e292fc078e5eb4922eb0edfebb939c928f"
        );
        let mut rows = vec![(1, 65_115, "41720dcbfb683543cbb41192e7cc3668315de3e358f018792a326accc09a5aef")];
        rows.extend((2..=8).map(|k| (k, SINGLE_DEAD.0, SINGLE_DEAD.1)));
        check_golden(&mut single, &rows, "single ON tile");
        let reads: Vec<_> = single.history().records().map(|r| r.exact.population()).collect();
        assert_eq!(reads, [1, 0, 0, 0, 0, 0, 0, 0], "a lone ON tile dies at metagen 1");

        let mut block = common::sim(&["OO", "OO"], "B3/S23", "moore");
        assert_eq!(
            common::cell_hash(block.universe()),
            "692ef87b74e8307d8f1087c4aa4cd206d5bc00dde5d05a561ed3e05f24e28eab"
        );
        let rows: Vec<Golden> = (1..=8).map(|k| (k, BLOCK_STEADY.0, BLOCK_STEADY.1)).collect();
        check_golden(&mut block, &rows, "block (period P from t = P)");
        assert!(block.history().records().all(|r| r.exact.population() == 4 && r.matches_prediction()));
        // Every ON core holds exactly 29 795 cells at t = kP.
        let counts = core_counts(block.universe(), &block.geometry()).unwrap();
        assert_eq!(counts, [READOUT_ON_COUNT; 4]);
    });
}

#[test]
fn life_rules_hold_for_metacells() {
    timed("B3/S23 rule configurations", || {
        let model = MacroModel::new(Programming::conway());
        // (initial, expected metagen 1, expected metagen 2, what it shows)
        type Case<'a> = (&'a [&'a str], &'a [&'a str], &'a [&'a str], &'a str);
        let cases: [Case; 5] = [
            (&["O..", "...", "..."], &["...", "...", "..."], &["...", "...", "..."], "no birth with 1, death with 0"),
            (&["O.O", "...", "..."], &["...", "...", "..."], &["...", "...", "..."], "no birth with 2"),
            (
                &["OO.", "O..", "..."],
                &["OO.", "OO.", "..."],
                &["OO.", "OO.", "..."],
                "birth with 3; survival with 2 and 3",
            ),
            (&["O..", ".O.", "..O"], &["...", ".O.", "..."], &["...", "...", "..."], "survival with 2 (diagonal)"),
            (&["O.O", ".O.", "O.O"], &[".O.", "O.O", ".O."], &[".O.", "O.O", ".O."], "death by overpopulation (4)"),
        ];
        for (initial, first, second, what) in cases {
            let mut sim = common::sim(initial, "B3/S23", "moore");
            let readouts = run_readouts(&mut sim, 3);
            assert_eq!(readouts[0], initial, "{what}: metagen 0");
            assert_eq!(readouts[1], first, "{what}: metagen 1");
            assert_eq!(readouts[2], second, "{what}: metagen 2");
            let pattern = MacroGrid::from_rows(initial).unwrap();
            assert_eq!(model.step(&pattern).to_rows(), first, "{what}: the macro rule agrees");
            assert!(sim.history().records().all(|r| r.matches_prediction()), "{what}");
        }
    });
}

#[test]
fn metacells_change_only_through_heard_neighbours() {
    timed("B1/S communication and deaf fuses", || {
        // B1/S: a tile is born iff exactly one heard neighbour is ON.
        let cases: [(&str, &[&str]); 4] = [
            ("moore", &[".O.", "OO.", "..."]),
            ("N,NE,E,SE,S,SW,W", &[".O.", "O..", "..."]), // deaf to NW: SE corner LWSS removed
            ("von_neumann", &[".O.", "O..", "..."]),
            ("NE,E,SE,S,SW,NW", &["...", ".O.", "..."]), // deaf to N and W: only the diagonal hears (0, 0)
        ];
        for (neighbourhood, expected) in cases {
            let mut sim = common::sim(&["O..", "...", "..."], "B1/S", neighbourhood);
            let readouts = run_readouts(&mut sim, 2);
            assert_eq!(readouts[0], ["O..", "...", "..."], "{neighbourhood}: metagen 0");
            assert_eq!(readouts[1], expected, "{neighbourhood}: metagen 1");
            assert!(sim.history().records().all(|r| r.matches_prediction()), "{neighbourhood}");
        }
    });
}

#[test]
fn macro_glider_moves_one_metacell_diagonally_every_four_metagens() {
    timed("glider 10×10, 12 metagens", || {
        let glider = scenario::builtin_by_id("metapixel.glider").unwrap();
        let mut sim = MetapixelSim::from_scenario(common::tiles(), &glider).unwrap();
        let initial = sim.initial_pattern().clone();
        let readouts = run_readouts(&mut sim, 12);
        assert_eq!(readouts.len(), 12);
        for k in 0..3u32 {
            let mut shifted = MacroGrid::new(10, 10).unwrap();
            for (c, r) in initial.on_tiles() {
                shifted.set(c + k, r + k, true);
            }
            assert_eq!(readouts[4 * k as usize], shifted.to_rows(), "metagen {}", 4 * k);
        }
        assert!(sim.history().records().all(|r| r.matches_prediction()));
    });
}

#[test]
fn random_soups_match_the_accelerated_model() {
    timed("4 random 6×6 soups, 5 metagens each", || {
        for seed in [11u64, 12, 13, 14] {
            let mut rng = SimRng::seed_from_u64(seed);
            let pattern = MacroGrid::random(6, 6, 0.45, &mut rng).unwrap();
            let rows = pattern.to_rows();
            let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
            let mut sim = common::sim(&rows, "B3/S23", "moore");
            run_readouts(&mut sim, 5);
            let model = MacroModel::new(Programming::conway());
            for record in sim.history().records() {
                assert_eq!(
                    record.exact,
                    model.run(&pattern, record.metagen),
                    "seed {seed}, metagen {}",
                    record.metagen
                );
                assert!(record.matches_prediction());
            }
            assert_eq!(sim.history().total_mismatches(), 0);
        }
    });
}

#[test]
fn other_rules_b0_and_finite_borders() {
    timed("B0/S single tile, 1×3 border, HighLife", || {
        // Brice Due's B0/S single tile blinks: born from nothing, then dies.
        let mut sim = common::sim(&["."], "B0/S", "moore");
        let readouts = run_readouts(&mut sim, 4);
        assert_eq!(readouts, [["."], ["O"], ["."], ["O"]]);
        // Outside the grid is permanently OFF: a 1×3 blinker dies.
        let mut sim = common::sim(&["OOO"], "B3/S23", "moore");
        assert_eq!(run_readouts(&mut sim, 3), [["OOO"], [".O."], ["..."]]);
        assert_eq!(sim.universe().bounding_box(), Some((-5, -5, 3 * 2048 + 4, 2048 + 4)), "nothing escapes");
        // HighLife (B36/S23): six neighbours give birth.
        let initial = ["OOO", "...", "OOO"];
        let mut sim = common::sim(&initial, "B36/S23", "moore");
        let readouts = run_readouts(&mut sim, 2);
        let model = MacroModel::new(Programming::parse("B36/S23", "moore").unwrap());
        let expected = model.step(&MacroGrid::from_rows(&initial).unwrap());
        assert!(expected.get(1, 1), "the centre has six ON neighbours");
        assert_eq!(readouts[1], expected.to_rows());
    });
}

#[test]
#[ignore = "36 exact runs (one per rule digit and table); run with --ignored"]
fn every_rule_bit_is_programmed_where_the_spec_says() {
    timed("rule-bit matrix (36 runs)", || {
        // Centre tile of a 3×3 grid with exactly d ON neighbours: OFF for Bd,
        // ON for Sd. It is ON at metagen 1 iff the digit is in the rule.
        let neighbours = [(0, 0), (1, 0), (2, 0), (2, 1), (2, 2), (1, 2), (0, 2), (0, 1)];
        for digit in 0..=8usize {
            for birth in [true, false] {
                for included in [true, false] {
                    let mut rows = vec![vec!['.'; 3]; 3];
                    for &(c, r) in &neighbours[..digit] {
                        rows[r][c] = 'O';
                    }
                    if !birth {
                        rows[1][1] = 'O';
                    }
                    let rows: Vec<String> = rows.into_iter().map(|r| r.into_iter().collect()).collect();
                    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
                    let digits: String = (0..=8).filter(|&d| (d == digit) == included).map(|d| d.to_string()).collect();
                    let rule = if birth { format!("B{digits}/S") } else { format!("B/S{digits}") };
                    let mut sim = common::sim(&rows, &rule, "moore");
                    sim.advance(2 * PERIOD);
                    let record = sim.history().latest().unwrap();
                    let want = if included { TileReading::On } else { TileReading::Off };
                    assert_eq!(record.reading(1, 1), want, "{rule}, {digit} neighbours");
                    assert!(record.matches_prediction(), "{rule}");
                }
            }
        }
    });
}

#[test]
fn golly_galaxy_demo_follows_life_and_matches_our_construction() {
    timed("galaxy demo vs Life and vs our construction, 9 metagens", || {
        let bytes = std::fs::read(common::otca_dir().join("metapixel-galaxy.mc.gz")).unwrap();
        let mut golly = read_golly_tiling(&bytes).unwrap();
        let read = |universe: &hashlife::Universe, geometry: &TileGeometry| -> MacroGrid {
            let mut grid = MacroGrid::new(geometry.columns, geometry.rows).unwrap();
            for (i, count) in core_counts(universe, geometry).unwrap().into_iter().enumerate() {
                let state = TileReading::classify(count).state();
                assert!(state.is_some(), "tile {i}: ambiguous core count {count}");
                grid.set(i as u32 % geometry.columns, i as u32 / geometry.columns, state == Some(true));
            }
            grid
        };
        let initial = read(&golly.universe, &golly.geometry);
        let galaxy = scenario::builtin_by_id("metapixel.galaxy").unwrap();
        let mut ours = MetapixelSim::from_scenario(common::tiles(), &galaxy).unwrap();
        assert_eq!(&initial, ours.initial_pattern());
        let model = MacroModel::new(golly.programming);
        let mut expected = initial.clone();
        for metagen in 0..9u64 {
            golly.universe.advance(PERIOD).unwrap();
            ours.advance(PERIOD);
            let theirs = read(&golly.universe, &golly.geometry);
            assert_eq!(theirs, expected, "Golly's demo, metagen {metagen}");
            let record = ours.history().latest().unwrap();
            assert_eq!(record.metagen, metagen);
            assert_eq!(record.exact, expected, "our construction, metagen {metagen}");
            assert!(record.matches_prediction());
            expected = model.step(&expected);
        }
        let last = ours.history().latest().unwrap();
        assert_eq!(last.exact, initial, "Kok's galaxy has period 8: metagen 8 = metagen 0");
        assert_eq!(last.exact.population(), 48);
    });
}

#[test]
#[ignore = "exact run of Golly's 64×64 parity demo (4096 tiles); run with --ignored"]
fn golly_parity_demo_follows_the_von_neumann_parity_rule() {
    timed("parity64 demo, 5 metagens", || {
        let bytes = std::fs::read(common::otca_dir().join("metapixel-parity64.mc.gz")).unwrap();
        let mut golly = read_golly_tiling(&bytes).unwrap();
        assert_eq!(golly.programming, Programming::parse("B135/S135", "von_neumann").unwrap());
        assert_eq!((golly.geometry.columns, golly.geometry.rows), (64, 64));
        let read = |universe: &hashlife::Universe| -> MacroGrid {
            let mut grid = MacroGrid::new(64, 64).unwrap();
            for (i, count) in core_counts(universe, &golly.geometry).unwrap().into_iter().enumerate() {
                let state = TileReading::classify(count).state();
                assert!(state.is_some(), "tile {i}: ambiguous core count {count}");
                grid.set(i as u32 % 64, i as u32 / 64, state == Some(true));
            }
            grid
        };
        let model = MacroModel::new(golly.programming);
        let mut expected = read(&golly.universe);
        for metagen in 0..5 {
            golly.universe.advance(PERIOD).unwrap();
            assert_eq!(read(&golly.universe), expected, "metagen {metagen}");
            expected = model.step(&expected);
        }
    });
}

#[test]
#[ignore = "exact run of Golly's sparse p216-gun demo (4512 tiles, B35/S236); run with --ignored"]
fn golly_p216_demo_follows_its_rule_with_missing_tiles_never_born() {
    timed("p216 demo (sparse), 4 metagens", || {
        let bytes = std::fs::read(common::otca_dir().join("metapixel-p216-gun.mc.gz")).unwrap();
        let mut golly = read_golly_tiling(&bytes).unwrap();
        assert_eq!(golly.programming, Programming::parse("B35/S236", "moore").unwrap());
        assert_eq!(golly.universe.population(), 128_116_349, "research note §7");
        let geometry = golly.geometry;
        // A tile exists iff its nominal square holds cells at generation 0.
        let present: Vec<bool> = tile_populations(&golly.universe, &geometry).unwrap().iter().map(|&p| p > 0).collect();
        assert_eq!(present.iter().filter(|&&p| p).count(), 4512);
        let (w, h) = (geometry.columns as i64, geometry.rows as i64);
        let read = |universe: &hashlife::Universe| -> Vec<bool> {
            core_counts(universe, &geometry)
                .unwrap()
                .into_iter()
                .enumerate()
                .map(|(i, count)| {
                    let state = TileReading::classify(count).state();
                    assert!(state.is_some(), "tile {i}: ambiguous core count {count}");
                    assert!(present[i] || state == Some(false), "a missing tile cannot be ON");
                    state == Some(true)
                })
                .collect()
        };
        // The macro rule with holes: missing tiles never send or get born.
        let step = |grid: &[bool]| -> Vec<bool> {
            (0..grid.len())
                .map(|i| {
                    let (c, r) = (i as i64 % w, i as i64 / w);
                    let n = Direction::ALL
                        .iter()
                        .filter(|d| {
                            let (dx, dy) = d.offset();
                            let (x, y) = (c + dx, r + dy);
                            (0..w).contains(&x) && (0..h).contains(&y) && grid[(y * w + x) as usize]
                        })
                        .count() as u8;
                    present[i] && golly.programming.rule.next_state(grid[i], n)
                })
                .collect()
        };
        let mut expected = read(&golly.universe);
        let on_at_start = expected.iter().filter(|&&on| on).count();
        assert!(on_at_start > 0);
        for metagen in 0..4 {
            golly.universe.advance(PERIOD).unwrap();
            assert_eq!(read(&golly.universe), expected, "metagen {metagen}");
            expected = step(&expected);
        }
    });
}

#[test]
#[ignore = "per-generation scan of the first transition window of the 3×3 blinker; run with --ignored"]
fn transition_window_timing_matches_the_research_note() {
    timed("per-generation core scan, t mod P in [4250, 7500]", || {
        let mut sim = common::sim(&["...", "OOO", "..."], "B3/S23", "moore");
        let geometry = sim.geometry();
        let (first, last) = (PERIOD + 4250, PERIOD + 7500);
        sim.advance(first);
        let mut counts: Vec<Vec<u64>> = Vec::new();
        for t in first..=last {
            if t > first {
                sim.advance(1);
            }
            counts.push(core_counts(sim.universe(), &geometry).unwrap());
        }
        let at = |phase: u64, tile: usize| counts[(PERIOD + phase - first) as usize][tile];
        let periodic = |phase: u64, tile: usize| at(phase, tile) == at(phase - 46, tile);
        // Row-major tiles: 3 and 5 die, 1 and 7 are born, 4 stays ON, corners stay OFF.
        for phase in 4250..=7500 {
            for tile in [0, 2, 6, 8] {
                assert_eq!(at(phase, tile), 0, "corner tile {tile} at phase {phase}");
            }
            assert!(at(phase, 4) >= READOUT_ON_STEADY_MIN, "the centre stays ON at phase {phase}");
            for tile in [3, 5] {
                assert_eq!(at(phase, tile) > 0, phase <= 7361, "dying tile {tile} at phase {phase}");
            }
            for tile in [1, 7] {
                assert_eq!(at(phase, tile) > 0, phase >= 4366, "born tile {tile} at phase {phase}");
            }
        }
        for phase in 4296..=7500 {
            assert!(periodic(phase, 4), "steady ON core has period 46 (phase {phase})");
        }
        for phase in 4296..4320 {
            assert!(periodic(phase, 3) && periodic(phase, 5), "dying cores are still steady at phase {phase}");
        }
        let left = (4296..=7400).find(|&p| !periodic(p, 3)).unwrap();
        eprintln!("dying core count first deviates from its period-46 sequence at t mod P = {left}");
        assert_eq!(left, 4320, "ON→OFF cores leave their steady state at t mod P = 4320");
        for phase in 7408 + 46..=7500 {
            assert!(periodic(phase, 1) && periodic(phase, 7), "born cores are steady at phase {phase}");
        }
        // ... and not one generation earlier: at 7407 the newborn core still
        // differs from the steady sequence it follows from 7408 on.
        assert!(!periodic(7407 + 46, 1) && !periodic(7407 + 46, 7), "born cores become steady exactly at 7408");
        for phase in [7452, 7498] {
            assert_eq!(at(phase, 1), READOUT_ON_COUNT, "t ≡ 0 (mod 46)");
        }
        let phase = sim.view(&Default::default()).phase;
        assert_eq!(phase.displayed_metagen, 1);
    });
}
