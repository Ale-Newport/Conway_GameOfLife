//! Criterion benchmarks of the evolution laboratory.
//!
//! * `speed_selection_100_ticks` — 100 ticks of the built-in speed-selection
//!   preset (64×48 world, ~300 prey, 24 constant-regime predators), starting from a state
//!   already run for 300 ticks so that the population is at working size.
//! * `island_evolution_100_ticks` — 100 ticks of the sexual, assortative
//!   island preset (90×40 world, obstacles, mate search).
//! * `view_with_genealogy` — building the full render view (organisms,
//!   food field, statistics, pruned genealogy) of the speed-selection state.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use evolution::scenarios::find_builtin;
use evolution::{EvolutionSim, EvolutionViewRequest};
use simulation_core::Simulation;

fn warmed(id: &str, ticks: u64) -> EvolutionSim {
    let scenario = find_builtin(id).expect("built-in scenarios are valid");
    let mut sim = scenario.build().expect("built-in scenarios build");
    sim.advance(ticks);
    sim
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("evolution");
    group.sample_size(10).measurement_time(Duration::from_secs(20));
    for id in ["speed_selection", "island_evolution"] {
        let start = warmed(id, 300);
        group.bench_function(format!("{id}_100_ticks"), |b| {
            b.iter_batched(
                || start.clone(),
                |mut sim| {
                    sim.advance(100);
                    black_box(sim.tick())
                },
                BatchSize::LargeInput,
            );
        });
    }
    let sim = warmed("speed_selection", 300);
    group.bench_function("view_with_genealogy", |b| {
        let request = EvolutionViewRequest { genealogy: true, ..Default::default() };
        b.iter(|| black_box(sim.view(&request).organisms.len()));
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
