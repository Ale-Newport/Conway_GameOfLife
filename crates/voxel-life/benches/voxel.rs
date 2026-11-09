//! Criterion benchmarks of the voxel engines (one generation per iteration).
//!
//! * `clouds1_128_dense` — Clouds 1 (`13-26/13-14,17-19/2/M`) on a 128³
//!   bounded dense grid filled at 60%, after 10 warm-up generations.
//! * `clouds1_128_sparse` — the same world in bounded sparse 16³ chunks.
//! * `crystal2_128_dense_von_neumann` — Crystal Growth 2 (`1-2/1,3/5/N`)
//!   on a 128³ torus from a 32³ soup at 30%, after 10 warm-up generations.
//!
//! The worlds keep evolving between iterations; Clouds 1 settles into an
//! almost static state within the warm-up and the dense engine's cost does
//! not depend on the contents, so the measurements are stable.
//!
//! `voxel_view/surface_clouds1_128_{dense,sparse}` build the default surface
//! view (every visible voxel, about 0.95 M of them) of the Clouds 1 worlds,
//! which the runner does up to 60 times per second.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use simulation_core::Simulation;
use voxel_life::{InitKind, InitSpec, Rule3D, Scenario, VoxelLife, VoxelView, VoxelViewRequest, WorldKind, scenarios};

fn world(id: &str, rule: &str, kind: WorldKind, side: u32, init: InitSpec) -> Option<VoxelLife> {
    let base = scenarios::builtin_by_id("clouds-1").ok()?;
    let scenario =
        Scenario { id: id.to_string(), rule: Rule3D::parse(rule).ok()?, world: kind, size: [side; 3], init, ..base };
    let mut life = VoxelLife::new(scenario).ok()?;
    life.advance(10);
    Some(life)
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("voxel_step");
    group.sample_size(10).measurement_time(Duration::from_secs(10)).warm_up_time(Duration::from_secs(2));
    let clouds = InitSpec { kind: InitKind::RandomCube, cube_side: 128, density: 0.6, ..InitSpec::default() };
    let soup = InitSpec { kind: InitKind::RandomCube, cube_side: 32, density: 0.3, ..InitSpec::default() };
    let cases = [
        ("clouds1_128_dense", "13-26/13-14,17-19/2/M", WorldKind::DenseBounded, clouds.clone()),
        ("clouds1_128_sparse", "13-26/13-14,17-19/2/M", WorldKind::SparseBounded, clouds.clone()),
        ("crystal2_128_dense_von_neumann", "1-2/1,3/5/N", WorldKind::DenseTorus, soup),
    ];
    for (name, rule, kind, init) in cases {
        let Some(mut life) = world(name, rule, kind, 128, init) else {
            eprintln!("benchmark `{name}`: could not build the world");
            continue;
        };
        group.bench_function(name, |b| {
            b.iter(|| {
                life.step();
                black_box(life.stats().population())
            })
        });
    }
    group.finish();

    let mut group = c.benchmark_group("voxel_view");
    group.sample_size(10).measurement_time(Duration::from_secs(10)).warm_up_time(Duration::from_secs(2));
    let request = VoxelViewRequest::default();
    for (name, kind) in [
        ("surface_clouds1_128_dense", WorldKind::DenseBounded),
        ("surface_clouds1_128_sparse", WorldKind::SparseBounded),
    ] {
        let rule = "13-26/13-14,17-19/2/M";
        let Some(life) = world(name, rule, kind, 128, clouds.clone()) else {
            eprintln!("benchmark `{name}`: could not build the world");
            continue;
        };
        group.bench_function(name, |b| {
            b.iter(|| match life.view(&request) {
                VoxelView::Surface(surface) => black_box(surface.visible),
                _ => 0,
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
