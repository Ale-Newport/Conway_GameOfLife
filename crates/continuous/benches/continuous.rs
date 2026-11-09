//! Criterion benchmarks of the continuous models.
//!
//! * `lenia_step/<side>`: one Orbium-rule step (R = 13, FFT) on a square world.
//! * `tessellatium_step/256`: three channels, 15 kernels.
//! * `smoothlife_step/256`: paper glider rule (r_a = 21).
//! * `flow_lenia_step/<side>`: the Flow-Lenia demo rule.
//! * `convolution/{fft,direct}/R<r>`: one single-kernel convolution on 256²,
//!   used to calibrate `ConvolutionMethod::Auto`.
//!
//! Run with `cargo bench -p continuous`.

use std::hint::black_box;
use std::time::Duration;

use continuous::convolve::{ConvolutionPlan, PlannedKernel};
use continuous::kernel::{KernelImage, lenia_shell};
use continuous::scenario::builtin_by_id;
use continuous::{ContinuousSim, ConvolutionMethod, KernelCore, Scenario};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use simulation_core::{ParamValue, SimRng, Simulation};

fn scenario(id: &str) -> Scenario {
    match builtin_by_id(id) {
        Some(s) => s,
        None => panic!("missing built-in scenario {id}"),
    }
}

/// A world from a built-in scenario, resized, reset and warmed up.
fn world(id: &str, side: usize, warmup: u64) -> ContinuousSim {
    let mut sim = match ContinuousSim::from_scenario(scenario(id)) {
        Ok(sim) => sim,
        Err(e) => panic!("{id}: {e}"),
    };
    for key in ["width", "height"] {
        if let Err(e) = sim.set_param(key, ParamValue::Int(side as i64)) {
            panic!("{id}: {e}");
        }
    }
    sim.reset();
    sim.advance(warmup);
    sim
}

fn bench_steps(c: &mut Criterion) {
    let mut group = c.benchmark_group("lenia_step");
    group.sample_size(20).measurement_time(Duration::from_secs(4));
    for side in [128usize, 256, 512, 1024] {
        let mut sim = world("lenia.orbium", side, 5);
        group.bench_function(BenchmarkId::from_parameter(side), |b| b.iter(|| sim.step()));
    }
    group.finish();

    let mut group = c.benchmark_group("tessellatium_step");
    group.sample_size(10).measurement_time(Duration::from_secs(4));
    let mut sim = world("lenia.tessellatium_gyrans", 256, 2);
    group.bench_function(BenchmarkId::from_parameter(256), |b| b.iter(|| sim.step()));
    group.finish();

    let mut group = c.benchmark_group("smoothlife_step");
    group.sample_size(20).measurement_time(Duration::from_secs(4));
    let mut sim = world("smoothlife.soup", 256, 2);
    group.bench_function(BenchmarkId::from_parameter(256), |b| b.iter(|| sim.step()));
    group.finish();

    let mut group = c.benchmark_group("flow_lenia_step");
    group.sample_size(20).measurement_time(Duration::from_secs(4));
    for side in [128usize, 256] {
        let mut sim = world("flow_lenia.demo", side, 5);
        group.bench_function(BenchmarkId::from_parameter(side), |b| b.iter(|| sim.step()));
    }
    group.finish();
}

fn bench_convolution(c: &mut Criterion) {
    let side = 256;
    let mut rng = SimRng::seed_from_u64(1);
    let field: Vec<f32> = (0..side * side).map(|_| rng.f32()).collect();
    let mut group = c.benchmark_group("convolution");
    group.sample_size(20).measurement_time(Duration::from_secs(3));
    for radius in [2.0f64, 3.0, 4.0, 6.0, 13.0] {
        let image = match KernelImage::radial(0, side, side, radius, |d| {
            lenia_shell(d / radius, &[1.0], KernelCore::GaussianBump)
        }) {
            Ok(k) => k,
            Err(e) => panic!("{e}"),
        };
        for method in [ConvolutionMethod::Fft, ConvolutionMethod::Direct] {
            let kernels = vec![PlannedKernel { source: 0, image: image.clone() }];
            let mut plan = match ConvolutionPlan::new(side, side, 1, kernels, method) {
                Ok(p) => p,
                Err(e) => panic!("{e}"),
            };
            let id = BenchmarkId::new(method.key(), format!("R{radius}_taps{}", image.taps().len()));
            group.bench_function(id, |b| {
                b.iter(|| {
                    plan.run(&[field.as_slice()], |_, u| {
                        black_box(u[0]);
                    })
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_steps, bench_convolution);
criterion_main!(benches);
