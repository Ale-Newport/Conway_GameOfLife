//! Criterion benchmarks: one tick of two presets after a warm-up, a large
//! area edit (one batch relabelling of land regions) and building a preset
//! from scratch.

use std::hint::black_box;

use civilization::{CivCommand, CivilizationSim, Resource, scenarios};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use simulation_core::Simulation;

fn warmed(id: &str, ticks: u64) -> CivilizationSim {
    let scenario = match scenarios::builtin_by_id(id) {
        Ok(s) => s,
        Err(e) => panic!("built-in scenario `{id}` failed to load: {e}"),
    };
    let mut sim = match CivilizationSim::new(scenario) {
        Ok(s) => s,
        Err(e) => panic!("built-in scenario `{id}` is invalid: {e}"),
    };
    sim.advance(ticks);
    sim
}

fn bench_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("step");
    group.sample_size(30);
    for id in ["abundant", "trade_routes"] {
        let base = warmed(id, 500);
        group.bench_function(id, |b| {
            b.iter_batched_ref(
                || {
                    let mut sim = warmed(id, 0);
                    // Clone the warmed state through a snapshot so each batch starts equal.
                    if let Ok(snap) = base.snapshot()
                        && sim.restore(&snap).is_ok()
                    {
                        return sim;
                    }
                    panic!("snapshot round trip failed");
                },
                |sim| {
                    sim.step();
                    black_box(sim.tick())
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn bench_area_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("command");
    group.sample_size(20);
    let base = warmed("territorial_expansion", 0);
    group.bench_function("add_resources_r30_96x96", |b| {
        b.iter_batched_ref(
            || {
                let mut sim = warmed("territorial_expansion", 0);
                if let Ok(snap) = base.snapshot()
                    && sim.restore(&snap).is_ok()
                {
                    return sim;
                }
                panic!("snapshot round trip failed");
            },
            |sim| {
                let command = CivCommand::AddResources { x: 48, y: 48, radius: 30.0, resource: Resource::Wood };
                black_box(sim.apply(command).is_ok())
            },
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

fn bench_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("build");
    group.sample_size(20);
    group.bench_function("territorial_expansion", |b| {
        b.iter(|| black_box(warmed("territorial_expansion", 0).agents().len()))
    });
    group.finish();
}

criterion_group!(benches, bench_step, bench_area_edit, bench_build);
criterion_main!(benches);
