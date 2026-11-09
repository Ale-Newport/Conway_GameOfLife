//! Criterion benchmarks of the Hashlife engine.
//!
//! * `gosper_gun_2^20` — a Gosper gun from a cold start (empty memo) to
//!   generation 2^20 in one `step_pow2(20)` call.
//! * `soup_256x256_step_2^10` — a 256×256 random soup (density 0.375) to
//!   generation 1024 in one `step_pow2(10)` call (chaotic: Hashlife's worst
//!   case).
//! * `soup_256x256_1024_single_generations` — the same soup advanced one
//!   generation at a time, 1024 times (comparable with dense engines).
//! * `metapixel_grid_empty_expansion` — 256 blocks on the corners of a 16×16
//!   grid of 2048-cell OTCA-sized tiles (a 32768-cell-wide, almost empty
//!   universe) advanced one OTCA period (35328 generations): measures root
//!   expansion and the empty-space shortcut.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use hashlife::Universe;
use simulation_core::SimRng;

const GOSPER_GUN: [(i64, i64); 36] = [
    (24, 0),
    (22, 1),
    (24, 1),
    (12, 2),
    (13, 2),
    (20, 2),
    (21, 2),
    (34, 2),
    (35, 2),
    (11, 3),
    (15, 3),
    (20, 3),
    (21, 3),
    (34, 3),
    (35, 3),
    (0, 4),
    (1, 4),
    (10, 4),
    (16, 4),
    (20, 4),
    (21, 4),
    (0, 5),
    (1, 5),
    (10, 5),
    (14, 5),
    (16, 5),
    (17, 5),
    (22, 5),
    (24, 5),
    (10, 6),
    (16, 6),
    (24, 6),
    (11, 7),
    (15, 7),
    (12, 8),
    (13, 8),
];

fn gun() -> Universe {
    let mut universe = Universe::conway();
    universe.insert_cells(GOSPER_GUN);
    universe
}

fn soup() -> Universe {
    let mut rng = SimRng::seed_from_u64(2024);
    let mut cells = Vec::new();
    for y in -128..128i64 {
        for x in -128..128i64 {
            if rng.chance(0.375) {
                cells.push((x, y));
            }
        }
    }
    let mut universe = Universe::conway();
    universe.insert_cells(cells);
    universe
}

fn metapixel_grid() -> Universe {
    let mut universe = Universe::conway();
    let mut cells = Vec::new();
    for ty in 0..16i64 {
        for tx in 0..16i64 {
            let (x, y) = (tx * 2048 - 16 * 1024, ty * 2048 - 16 * 1024);
            cells.extend([(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)]);
        }
    }
    universe.insert_cells(cells);
    universe
}

fn benches(c: &mut Criterion) {
    let mut group = c.benchmark_group("hashlife");
    group.sample_size(10).warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));
    group.bench_function("gosper_gun_2^20", |b| {
        b.iter_batched(
            gun,
            |mut u| {
                u.step_pow2(20).expect("in range");
                black_box(u.population())
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("soup_256x256_step_2^10", |b| {
        b.iter_batched(
            soup,
            |mut u| {
                u.step_pow2(10).expect("in range");
                black_box(u.population())
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("soup_256x256_1024_single_generations", |b| {
        b.iter_batched(
            soup,
            |mut u| {
                for _ in 0..1024 {
                    u.step_pow2(0).expect("in range");
                }
                black_box(u.population())
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("metapixel_grid_empty_expansion", |b| {
        b.iter_batched(
            metapixel_grid,
            |mut u| {
                u.advance(35_328).expect("in range");
                black_box(u.population())
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(hashlife_benches, benches);
criterion_main!(hashlife_benches);
