//! Criterion benchmarks of the custom-life engine.
//!
//! * `step/<preset>`: one tick of a preset after a warm-up run that brings
//!   the population to its typical size.
//! * `compile/all_presets`: parsing, validation, relationship compilation
//!   and Rhai compilation of the seven presets.
//! * `dsl/parse_print`: parsing and pretty-printing every preset rule.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use rule_engine::dsl::{parse_rule, print_rule};
use rule_engine::{CustomLifeSim, compile, scenarios};
use simulation_core::Simulation;

fn bench_steps(c: &mut Criterion) {
    let presets = scenarios::builtin().expect("built-in presets are valid");
    let mut group = c.benchmark_group("step");
    for (id, warmup) in
        [("bacterial-growth", 400), ("producers-consumers", 300), ("artificial-chemistry", 300), ("robot-swarm", 200)]
    {
        let Some(p) = presets.iter().find(|p| p.id == id) else { continue };
        let mut sim = CustomLifeSim::from_scenario(p).expect("preset compiles");
        sim.advance(warmup);
        let n = sim.total_population();
        group.bench_function(format!("{id} ({n} entities)"), |b| b.iter(|| sim.step()));
    }
    group.finish();
}

fn bench_compile(c: &mut Criterion) {
    let presets = scenarios::builtin().expect("built-in presets are valid");
    c.bench_function("compile/all_presets", |b| {
        b.iter(|| {
            for p in &presets {
                black_box(compile(&p.system).expect("valid"));
            }
        })
    });
    let texts: Vec<String> = presets.iter().flat_map(|p| p.system.rules.iter().map(print_rule)).collect();
    c.bench_function("dsl/parse_print", |b| {
        b.iter(|| {
            for t in &texts {
                let rule = parse_rule(black_box(t)).expect("valid");
                black_box(print_rule(&rule));
            }
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20).measurement_time(Duration::from_secs(4)).warm_up_time(Duration::from_secs(1));
    targets = bench_steps, bench_compile
}
criterion_main!(benches);
