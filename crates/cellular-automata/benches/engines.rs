//! Engine benchmarks (Criterion). Results are recorded in
//! `docs/models/cellular-automata.md`.
//!
//! * Dense and Bit engines: one generation of a 30 % random soup on 512² and
//!   2048² tori.
//! * Sparse engine: one generation of a set of far-apart oscillators and
//!   spaceships placed around coordinates of ±2^40.
//! * Hashlife adapter: the Gosper glider gun advanced to generation 2^20 from
//!   an empty memo (one `step_pow2(20)` call per iteration), and the texel
//!   counts of views of Breeder 1 at generation 2^24 (3.7·10^11 live cells):
//!   framing the whole pattern with the desktop view's margin, and two
//!   windows inside the triangle of glider streams whose unaligned borders
//!   cut through it (2^11 and 2^7 cells per texel).

use std::hint::black_box;
use std::path::Path;
use std::time::Duration;

use cellular_automata::view::TexelGrid;
use cellular_automata::{CellRect, EngineKind, GridViewRequest, World, create_engine};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use patterns::{Catalog, LifeRule, Pattern};
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

/// A view request for `[x0, x1] × [y0, y1]` with a 1920×1080-texel budget.
fn breeder_request(x0: i64, y0: i64, x1: i64, y1: i64) -> GridViewRequest {
    GridViewRequest { x0, y0, x1, y1, max_width: 1920, max_height: 1080 }
}

fn hashlife_breeder_view(c: &mut Criterion) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/patterns/catalog.toml");
    let catalog = Catalog::load(&path).expect("asset catalog");
    let breeder = catalog.by_id("breeder-1").and_then(|e| catalog.load_pattern(e).ok()).expect("breeder-1");
    let world = World::new(Topology::Infinite, 256, 256).expect("valid");
    let mut engine = create_engine(EngineKind::Hashlife, world, LifeRule::conway()).expect("engine");
    engine.load_cells(&breeder.live_cells().collect::<Vec<_>>());
    engine.advance(1 << 24).expect("in range");
    // A triangle of glider streams: base (0, 0)–(8 389 356, 0), apex near (4.19·10^6, −4.19·10^6).
    let b = engine.bounding_box().expect("not empty");
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (0, -4_194_138, 8_389_356, 337));
    let margin = b.width().max(b.height()) as i64 / 8;
    let views = [
        ("fit", breeder_request(b.x0 - margin, b.y0 - margin, b.x1 + margin, b.y1 + margin)),
        ("inside_2^11", breeder_request(2_700_007, -2_349_997, 5_699_995, -650_011)),
        ("inside_2^7", breeder_request(4_100_003, -1_600_005, 4_299_997, -1_489_999)),
    ];
    let mut group = c.benchmark_group("hashlife_breeder_2^24_view");
    group.sample_size(10).measurement_time(Duration::from_secs(5));
    for (name, request) in views {
        let grid = TexelGrid::plan(&request, None);
        let mut out = vec![0u32; grid.len()];
        group.bench_function(name, |b| {
            b.iter(|| {
                out.fill(0);
                engine.count_live(&grid, &mut out);
                black_box(&out);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, finite_engines, sparse_far_apart, hashlife_gun, hashlife_breeder_view);
criterion_main!(benches);
