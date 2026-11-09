//! Criterion benchmarks on a 256×256 map: one world step in calm weather,
//! during a thunderstorm (rain on every cell, routing, lightning), and with
//! a wide active fire front; plus world generation.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use physics_world::{Disaster, SurfaceKind, World, WorldConfig};

fn config() -> WorldConfig {
    WorldConfig::with_size(256, 256).with_seed(7)
}

fn bench_step(c: &mut Criterion) {
    let mut world = World::generate(&config());
    world.fuel_mut().fill(1.5);
    c.bench_function("world_step_256x256", |b| b.iter(|| black_box(world.step())));

    let mut stormy = World::generate(&config());
    stormy.fuel_mut().fill(1.5);
    if let Err(error) = stormy.trigger(Disaster::Storm { duration_ticks: 1_000_000, intensity: 1.0 }) {
        panic!("storm intervention rejected: {error}");
    }
    c.bench_function("world_step_256x256_storm", |b| b.iter(|| black_box(stormy.step())));

    // A drought-dried world with a fire line across the map: every measured
    // step starts from a fresh copy, so the front is always active.
    let mut fire = World::generate(&config());
    fire.fuel_mut().fill(1.5);
    if let Err(error) = fire.start_drought(1_000_000) {
        panic!("drought intervention rejected: {error}");
    }
    for _ in 0..240 {
        fire.step();
    }
    let mut ignited = 0;
    for y in 0..256 {
        if fire.surface().get(128, y) == &SurfaceKind::Land {
            match fire.ignite(128, y) {
                Ok(true) => ignited += 1,
                Ok(false) => {}
                Err(error) => panic!("ignition rejected: {error}"),
            }
        }
    }
    for _ in 0..20 {
        fire.step();
    }
    assert!(ignited > 0 && !fire.burning_cells().is_empty(), "the fire benchmark needs an active front");
    c.bench_function("world_step_256x256_fire_front", |b| {
        b.iter_batched_ref(|| fire.clone(), |w| black_box(w.step()), BatchSize::LargeInput)
    });
}

fn bench_generate(c: &mut Criterion) {
    let mut group = c.benchmark_group("generation");
    group.sample_size(10);
    group.bench_function("generate_256x256", |b| {
        b.iter_batched(config, |cfg| black_box(World::generate(&cfg)), BatchSize::SmallInput)
    });
    group.finish();
}

criterion_group!(benches, bench_step, bench_generate);
criterion_main!(benches);
