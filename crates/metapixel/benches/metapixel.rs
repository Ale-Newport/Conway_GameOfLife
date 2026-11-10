//! Criterion benchmarks of the OTCA metapixel construction.
//!
//! * `program_tiles_b3s23` — programming both unprogrammed tiles.
//! * `build_64x64_random` — building a 64×64 construction (2.7·10⁸ cells
//!   wide area, 4096 tiles) and reading it into Hashlife.
//! * `blinker_3x3_one_metagen_cold` — a fresh 3×3 blinker advanced one full
//!   metageneration (35 328 generations) with an empty memo, including the
//!   readout of all 9 tiles at `t = P`.
//! * `readout_galaxy_15x15` — counting the readout cores of all 225 tiles
//!   of the galaxy construction.
//! * `micro_view_galaxy_1024` — the level-of-detail cell texture of the
//!   whole galaxy construction in a 1024×1024-texel budget.
//! * `accelerated_step_256x256` — one step of the accelerated macro model
//!   on the largest grid.
//!
//! Results are recorded in `docs/OTCA_METAPIXEL.md`.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use metapixel::readout::{TileGeometry, core_counts};
use metapixel::spec::PERIOD;
use metapixel::{
    GridViewRequest, MacroGrid, MacroModel, MetapixelSim, MetapixelViewRequest, Programming, TileSet, WorldConfig,
    build_universe, scenario,
};
use simulation_core::{SimRng, Simulation};

fn tiles() -> Arc<TileSet> {
    Arc::new(TileSet::from_dir(&TileSet::source_tree_dir()).expect("tiles in assets/patterns/otca"))
}

fn benches(c: &mut Criterion) {
    let tiles = tiles();
    c.bench_function("program_tiles_b3s23", |b| b.iter(|| black_box(tiles.program(Programming::conway()))));

    let programmed = tiles.program(Programming::conway());
    let soup = MacroGrid::random(64, 64, 0.5, &mut SimRng::seed_from_u64(1)).unwrap();
    c.bench_function("build_64x64_random", |b| b.iter(|| black_box(build_universe(&programmed, &soup).unwrap())));

    let blinker = MacroGrid::from_rows(&["...", "OOO", "..."]).unwrap();
    c.bench_function("blinker_3x3_one_metagen_cold", |b| {
        b.iter_batched(
            || MetapixelSim::new(tiles.clone(), WorldConfig::default(), blinker.clone()).unwrap(),
            |mut sim| {
                sim.advance(PERIOD);
                black_box(sim.history().latest().map(|r| r.exact.population()))
            },
            BatchSize::LargeInput,
        )
    });

    let galaxy = scenario::builtin_by_id("metapixel.galaxy").unwrap();
    let mut sim = MetapixelSim::from_scenario(tiles.clone(), &galaxy).unwrap();
    sim.advance(PERIOD);
    let geometry = TileGeometry::at_origin(15, 15);
    c.bench_function("readout_galaxy_15x15", |b| b.iter(|| black_box(core_counts(sim.universe(), &geometry).unwrap())));

    // The renderer's request for the whole galaxy (30 730 cells square) in a
    // 1024×1024-texel budget: 32 cells per texel, 962×962 texels.
    let whole =
        GridViewRequest { x0: -5, y0: -5, x1: 15 * 2048 + 4, y1: 15 * 2048 + 4, max_width: 1024, max_height: 1024 };
    let request = MetapixelViewRequest { micro: Some(whole), macro_grid: false, live_readings: false, inspect: None };
    c.bench_function("micro_view_galaxy_1024", |b| b.iter(|| black_box(sim.view(&request).micro)));

    let model = MacroModel::new(Programming::conway());
    let big = MacroGrid::random(256, 256, 0.4, &mut SimRng::seed_from_u64(2)).unwrap();
    c.bench_function("accelerated_step_256x256", |b| b.iter(|| black_box(model.step(&big))));
}

criterion_group! {
    name = metapixel;
    config = Criterion::default().sample_size(10).measurement_time(Duration::from_secs(8));
    targets = benches
}
criterion_main!(metapixel);
