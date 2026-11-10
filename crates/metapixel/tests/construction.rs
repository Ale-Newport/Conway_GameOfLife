//! Building constructions: OR-tiling at pitch 2048, golden generation-0
//! states, sharing of quadtree squares, readout at generation 0, and
//! equivalence with Golly's own tiling of the galaxy demo.

mod common;

use std::time::Instant;

use metapixel::readout::{TileGeometry, core_counts, count_live_in_rect, tile_populations};
use metapixel::spec::{READOUT_ON_COUNT, TILE_PITCH};
use metapixel::{MacroGrid, Programming, TileState, build_universe, read_golly_tiling, scenario};

fn build(rows: &[&str]) -> hashlife::Universe {
    let pattern = MacroGrid::from_rows(rows).unwrap();
    let tiles = common::tiles().program(Programming::conway());
    build_universe(&tiles, &pattern).unwrap().0
}

#[test]
fn generation_zero_matches_the_golden_or_tilings() {
    // Research note §7.2 (bgolly): population and SHA-256 of the OR-tiling.
    let cases: [(&[&str], u64, &str); 3] = [
        (&["...", "OOO", "..."], 342_076, "cf4530b73d4fff3232ad87ba999a2b687ee31066476df403ab3f71774cc8987b"),
        (&["O"], 66_206, "d278172c42df6acd4b92e9ba484196e292fc078e5eb4922eb0edfebb939c928f"),
        (&["OO", "OO"], 264_796, "692ef87b74e8307d8f1087c4aa4cd206d5bc00dde5d05a561ed3e05f24e28eab"),
    ];
    for (rows, population, hash) in cases {
        let universe = build(rows);
        assert_eq!(universe.generation(), 0);
        assert_eq!(universe.population(), population, "{rows:?}");
        assert_eq!(common::cell_hash(&universe), hash, "{rows:?}");
        // Nothing beyond the 5-cell overlap of the border tiles.
        let (w, h) = (rows[0].len() as i64, rows.len() as i64);
        assert_eq!(universe.bounding_box(), Some((-5, -5, TILE_PITCH * w + 4, TILE_PITCH * h + 4)));
    }
}

#[test]
fn every_tile_is_the_programmed_tile_or_ed_at_its_nominal_origin() {
    let pattern = MacroGrid::from_rows(&["O.O", ".O.", "..O", "O.."]).unwrap();
    let tiles = common::tiles().program(Programming::parse("B36/S23", "N,E,S,W,NE").unwrap());
    let (universe, _) = build_universe(&tiles, &pattern).unwrap();
    // Expected plane: the union of every tile shifted to (2048·col, 2048·row).
    let mut expected = std::collections::BTreeSet::new();
    for row in 0..pattern.height() {
        for col in 0..pattern.width() {
            let state = TileState::from_bool(pattern.get(col, row));
            let (ox, oy) = (TILE_PITCH * i64::from(col), TILE_PITCH * i64::from(row));
            expected.extend(tiles.cells(state).iter().map(|&(x, y)| (x + ox, y + oy)));
        }
    }
    let actual: std::collections::BTreeSet<(i64, i64)> = universe.cells().collect();
    assert_eq!(actual.len(), expected.len());
    assert!(actual == expected, "the construction must equal the OR of the shifted tiles");
}

#[test]
fn readout_at_generation_zero_recovers_the_macro_pattern() {
    let pattern = MacroGrid::from_rows(&["O..O", ".OO.", "O.O."]).unwrap();
    let tiles = common::tiles().program(Programming::conway());
    let (universe, _) = build_universe(&tiles, &pattern).unwrap();
    let geometry = TileGeometry::at_origin(4, 3);
    let counts = core_counts(&universe, &geometry).unwrap();
    for (i, &count) in counts.iter().enumerate() {
        let on = pattern.get(i as u32 % 4, i as u32 / 4);
        assert_eq!(count, if on { READOUT_ON_COUNT } else { 0 }, "tile {i}");
    }
    let populations = tile_populations(&universe, &geometry).unwrap();
    let sum: u64 = populations.iter().sum();
    let inside = count_live_in_rect(&universe, geometry.bounds()).unwrap();
    assert_eq!(sum, inside, "per-tile populations partition the nominal squares");
}

#[test]
fn large_grids_share_squares_instead_of_inserting_cells() {
    let mut rng = simulation_core::SimRng::seed_from_u64(5);
    let pattern = MacroGrid::random(64, 64, 0.5, &mut rng).unwrap();
    let tiles = common::tiles().program(Programming::conway());
    let started = Instant::now();
    let (universe, stats) = build_universe(&tiles, &pattern).unwrap();
    let elapsed = started.elapsed();
    // 2 states × 9 border classes inside the grid plus 16 classes of the
    // outer ring (4 corners, 3 per side): a few dozen squares, whatever the
    // number of tiles.
    assert!(stats.distinct_squares <= 2 * 9 + 16, "{stats:?}");
    let ons = pattern.population();
    let expected = ons * 66_206 + (4096 - ons) * 23_923;
    // Overlapping cells (corner blocks and tubs) are counted once.
    assert!(universe.population() < expected && universe.population() > expected - 4096 * 30);
    eprintln!("64×64 build: {elapsed:?}, {stats:?}, population {}", universe.population());
}

#[test]
fn golly_galaxy_demo_is_our_construction_plus_method_b_trimming() {
    let bytes = std::fs::read(common::otca_dir().join("metapixel-galaxy.mc.gz")).unwrap();
    let golly = read_golly_tiling(&bytes).unwrap();
    assert_eq!(golly.geometry, TileGeometry { origin_x: -16_384, origin_y: -16_383, columns: 15, rows: 15 });
    assert_eq!(golly.programming, Programming::conway());
    assert_eq!(golly.universe.population(), 7_408_195, "research note §7");
    // The macro pattern of the demo at generation 0 is our galaxy scenario.
    let counts = core_counts(&golly.universe, &golly.geometry).unwrap();
    let galaxy = scenario::builtin_by_id("metapixel.galaxy").unwrap();
    let pattern = galaxy.initial_pattern(&mut simulation_core::SimRng::seed_from_u64(1)).unwrap();
    for (i, &count) in counts.iter().enumerate() {
        let on = pattern.get(i as u32 % 15, i as u32 / 15);
        assert_eq!(count, if on { READOUT_ON_COUNT } else { 0 }, "tile {i}");
    }
    assert_eq!(pattern.population(), 48);
    // Method A (ours) = method B (Golly) plus the 4 corner cells of every
    // tile, tile for tile.
    let tiles = common::tiles().program(Programming::conway());
    let (ours, _) = build_universe(&tiles, &pattern).unwrap();
    let ours_pop = tile_populations(&ours, &TileGeometry::at_origin(15, 15)).unwrap();
    let golly_pop = tile_populations(&golly.universe, &golly.geometry).unwrap();
    for (i, (a, b)) in ours_pop.iter().zip(&golly_pop).enumerate() {
        assert_eq!(*a, b + 4, "tile {i}");
    }
    for (col, row) in [(0, 0), (14, 0), (7, 0), (0, 7), (3, 3), (7, 7), (6, 4), (14, 14)] {
        let theirs = golly.geometry.tile_rect(col, row);
        let mine = TileGeometry::at_origin(15, 15).tile_rect(col, row);
        let shift = (mine.x0 - theirs.x0, mine.y0 - theirs.y0);
        let corners = [(0, 0), (2047, 0), (0, 2047), (2047, 2047)];
        let a: Vec<(i64, i64)> = ours
            .cells_in_rect(mine.x0, mine.y0, mine.x1, mine.y1, usize::MAX)
            .into_iter()
            .map(|(x, y)| (x - mine.x0, y - mine.y0))
            .filter(|c| !corners.contains(c))
            .collect();
        let b: Vec<(i64, i64)> = golly
            .universe
            .cells_in_rect(theirs.x0, theirs.y0, theirs.x1, theirs.y1, usize::MAX)
            .into_iter()
            .map(|(x, y)| (x + shift.0 - mine.x0, y + shift.1 - mine.y0))
            .collect();
        assert!(a == b, "tile ({col}, {row}) differs from Golly's beyond the corner cells");
    }
}

#[test]
fn sha256_reference_vectors() {
    assert_eq!(common::hex(&common::sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(
        common::hex(&common::sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    assert_eq!(common::hex(&common::sha256(long)), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
}
