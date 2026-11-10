//! Engine benchmarks (Criterion). Results are recorded in
//! `docs/models/cellular-automata.md`.
//!
//! * Dense and Bit engines: one generation of a 30 % random soup on 512² and
//!   2048² tori.
//! * Sparse engine: one generation of a set of far-apart oscillators and
//!   spaceships placed around coordinates of ±2^40.
//! * Hashlife adapter: the Gosper glider gun advanced to generation 2^20 from
//!   an empty memo (one `step_pow2(20)` call per iteration).

use std::hint::black_box;
use std::time::Duration;

use cellular_automata::{CellRect, EngineKind, World, create_engine};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use patterns::{LifeRule, Pattern};
use simulation_core::{SimRng, Topology};

fn soup(size: u32, density: f64, seed: u64) -> Vec<(i64, i64)> {
    let mut rng = SimRng::seed_from_u64(seed);
    let mut cells = Vec::new();
    for y in 0..i64::from(size) {
        for x in 0..i64::from(size) {
            if rng.f64() < density {
                cells.push((x, y));
            }
        }
    }
    cells
}

fn finite_engines(c: &mut Criterion) {
    let mut group = c.benchmark_group("finite_soup_generation");
    group.sample_size(20).measurement_time(Duration::from_secs(4)).warm_up_time(Duration::from_secs(1));
    for size in [512u32, 2048] {
        let cells = soup(size, 0.3, 7);
        let world = World::new(Topology::Torus, size, size).expect("valid world");
        group.throughput(Throughput::Elements(u64::from(size) * u64::from(size)));
        for kind in [EngineKind::Dense, EngineKind::Bit] {
            let mut engine = create_engine(kind, world, LifeRule::conway()).expect("engine");
            engine.load_cells(&cells);
            // Skip the first explosive generations of the soup.
            engine.advance(16).expect("finite engines never fail");
            group.bench_with_input(BenchmarkId::new(kind.as_str(), size), &size, |b, _| {
                b.iter(|| black_box(engine.step().expect("finite engines never fail")));
            });
        }
    }
    group.finish();
}

fn sparse_far_apart(c: &mut Criterion) {
    let glider = Pattern::from_ascii(".O.\n..O\nOOO").expect("valid");
    let pulsar_rows = [
        "..OOO...OOO..",
        ".............",
        "O....O.O....O",
        "O....O.O....O",
        "O....O.O....O",
        "..OOO...OOO..",
        ".............",
        "..OOO...OOO..",
        "O....O.O....O",
        "O....O.O....O",
        "O....O.O....O",
        ".............",
        "..OOO...OOO..",
    ];
    let pulsar = Pattern::from_ascii(&pulsar_rows.join("\n")).expect("valid");
    let mut cells = Vec::new();
    let far = 1i64 << 40;
    for (i, (cx, cy)) in
        [(-far, -far), (far, -far), (-far, far), (far, far), (0, 0), (far / 3, -far / 7)].into_iter().enumerate()
    {
        let p = if i % 2 == 0 { &glider } else { &pulsar };
        for dy in 0..4 {
            for dx in 0..4 {
                cells.extend(p.live_cells().map(|(x, y)| (cx + 40 * dx + x, cy + 40 * dy + y)));
            }
        }
    }
    let mut engine =
        create_engine(EngineKind::Sparse, World::new(Topology::Infinite, 256, 256).expect("valid"), LifeRule::conway())
            .expect("engine");
    engine.load_cells(&cells);
    let mut group = c.benchmark_group("sparse_far_apart");
    group.sample_size(30).measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(engine.population()));
    group.bench_function("generation", |b| b.iter(|| black_box(engine.step().expect("in range"))));
    group.finish();
    assert!(engine.bounding_box().is_some_and(|r: CellRect| r.width() > 1 << 40));
}

fn hashlife_gun(c: &mut Criterion) {
    let gun = patterns::builtin_patterns()
        .iter()
        .find(|p| p.id == "gosper-glider-gun")
        .and_then(|p| p.pattern().ok())
        .expect("embedded Gosper gun");
    let cells: Vec<(i64, i64)> = gun.live_cells().collect();
    let world = World::new(Topology::Infinite, 256, 256).expect("valid");
    let mut group = c.benchmark_group("hashlife_gosper_gun");
    group.sample_size(10).measurement_time(Duration::from_secs(5));
    group.bench_function("advance_2^20_cold", |b| {
        b.iter(|| {
            let mut engine = create_engine(EngineKind::Hashlife, world, LifeRule::conway()).expect("engine");
            engine.load_cells(&cells);
            engine.advance(1 << 20).expect("in range");
            black_box(engine.population())
        });
    });
    group.finish();
}

criterion_group!(benches, finite_engines, sparse_far_apart, hashlife_gun);
criterion_main!(benches);
