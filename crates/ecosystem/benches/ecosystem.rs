//! Criterion benchmarks of the ecosystem step.
//!
//! * `meadow_tick` — one ordinary tick of the balanced-meadow scenario
//!   (100 × 100 cells) one day in: world step, colonies and animals (the
//!   tick starts no simulated hour or day);
//! * `meadow/meadow_day` — 240 ticks (one simulated day) from the same
//!   state, including the 24 hourly vegetation / fungi / field phases and
//!   the daily tree update; divide by 240 for the mean cost of a tick;
//! * `wolf_sheep_grass_tick` — one tick of the NetLogo rules (51 × 51,
//!   200–250 animals after 200 ticks);
//! * `agents/1000`, `agents/10000` and `agents/100000` — one ordinary tick
//!   of a 160 × 160 grassland (3.2 km side, rivers and lakes) holding 1,000,
//!   10,000 or 100,000 individual animals of five species (rabbits, deer,
//!   sheep, foxes, wolves) after a 50-tick warm-up;
//! * `fields/100k` and `fields/10M` — one simulated hour (10 ticks, one
//!   field update) of a 100 × 100 grassland without individual animals
//!   whose beetles, worms, spiders and butterflies number 10⁵ or 10⁷ in
//!   total: the aggregate representation costs `O(cells)`, not `O(organisms)`.
//!
//! Each iteration restores the same snapshot first (untimed), so every
//! sample measures the same work.

use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ecosystem::scenario::find_builtin;
use ecosystem::{AnimalSetup, EcosystemConfig, EcosystemSim, SpeciesCatalog, Terrain, TreeSetup};
use simulation_core::{ParamSet, ParamValue, Simulation, Snapshot};

/// A run advanced by `ticks` and its snapshot.
fn prepared(mut sim: EcosystemSim, ticks: u64) -> (EcosystemSim, Snapshot) {
    sim.advance(ticks);
    let snapshot = sim.snapshot().expect("snapshot");
    (sim, snapshot)
}

fn scenario(id: &str) -> EcosystemSim {
    find_builtin(id).expect("built-in scenario").build().expect("scenario builds")
}

/// Times `ticks` steps after restoring `snapshot`, `iters` times.
fn timed(sim: &mut EcosystemSim, snapshot: &Snapshot, ticks: u64, iters: u64) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        sim.restore(snapshot).expect("restore");
        let start = Instant::now();
        sim.advance(ticks);
        total += start.elapsed();
    }
    total
}

fn setup(species: &str, count: u32) -> AnimalSetup {
    AnimalSetup { species: species.into(), count, region: None, groups: None }
}

/// A 160 × 160 grassland of 20 m cells with `animals` individuals.
fn crowd(animals: u32) -> EcosystemSim {
    let mut config = EcosystemConfig::default();
    config.world.width = 160;
    config.world.height = 160;
    config.world.river_count = 4;
    config.world.lake_tendency = 0.7;
    config.world.lightning_rate = 0.0;
    config.terrain = Terrain::Generated;
    config.trees = TreeSetup { density_per_ha: 10.0, species: vec!["oak".into(), "birch".into()] };
    let share = |f: f32| (animals as f32 * f) as u32;
    config.animals = vec![
        setup("rabbit", share(0.6)),
        setup("deer", share(0.15)),
        setup("sheep", share(0.15)),
        setup("fox", share(0.05)),
        setup("wolf", share(0.05)),
    ];
    let mut params = ParamSet::new();
    params.set("max_population_per_species", ParamValue::Int(100_000));
    params.set("max_population_total", ParamValue::Int(200_000));
    config.params = params;
    EcosystemSim::new(config, 3, SpeciesCatalog::builtin().expect("species")).expect("valid configuration")
}

/// A 100 × 100 grassland without individual animals and `organisms`
/// field organisms of four species.
fn fields(organisms: u32) -> EcosystemSim {
    let mut config = EcosystemConfig::default();
    config.world.width = 100;
    config.world.height = 100;
    config.world.lightning_rate = 0.0;
    config.terrain = Terrain::Flat { elevation_m: 60.0 };
    config.trees = TreeSetup { density_per_ha: 0.0, species: Vec::new() };
    let share = |f: f32| (organisms as f32 * f) as u32;
    config.animals = vec![
        setup("beetle", share(0.4)),
        setup("worm", share(0.4)),
        setup("spider", share(0.15)),
        setup("butterfly", share(0.05)),
    ];
    EcosystemSim::new(config, 5, SpeciesCatalog::builtin().expect("species")).expect("valid configuration")
}

fn bench_steps(c: &mut Criterion) {
    // Tick 241: hours start every 10 ticks and days every 240.
    let (mut meadow, tick_241) = prepared(scenario("balanced_meadow"), 241);
    c.bench_function("meadow_tick", |b| b.iter_custom(|iters| timed(&mut meadow, &tick_241, 1, iters)));
    let (mut meadow, day_1) = prepared(scenario("balanced_meadow"), 240);
    let mut group = c.benchmark_group("meadow");
    group.sample_size(10);
    group.bench_function("meadow_day", |b| b.iter_custom(|iters| timed(&mut meadow, &day_1, 240, iters)));
    group.finish();
    let (mut netlogo, tick_200) = prepared(scenario("wolf_sheep_grass"), 200);
    c.bench_function("wolf_sheep_grass_tick", |b| b.iter_custom(|iters| timed(&mut netlogo, &tick_200, 1, iters)));

    let mut group = c.benchmark_group("agents");
    group.sample_size(10);
    for n in [1_000u32, 10_000, 100_000] {
        // Tick 51: an ordinary tick (no hour or day starts).
        let (mut sim, snap) = prepared(crowd(n), 51);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter_custom(|iters| timed(&mut sim, &snap, 1, iters))
        });
    }
    group.finish();

    let mut group = c.benchmark_group("fields");
    group.sample_size(10);
    for (label, n) in [("100k", 100_000u32), ("10M", 10_000_000)] {
        let (mut sim, snap) = prepared(fields(n), 0);
        group.bench_function(label, |b| b.iter_custom(|iters| timed(&mut sim, &snap, 10, iters)));
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(30);
    targets = bench_steps
}
criterion_main!(benches);
