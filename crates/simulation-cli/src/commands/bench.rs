//! `bench`: ticks per second, time per tick and model memory.
//!
//! Every measured tick is one `step()` call (see [`crate::headless`]) timed
//! on its own with the monotonic clock (`std::time::Instant`); warm-up ticks are run
//! first and not measured.

use analytics::stats::{quantile, summarize};
use model_registry::Registry;
use serde::Serialize;

use crate::cli::BenchArgs;
use crate::commands::{build, request};
use crate::headless;
use crate::machine::MachineInfo;
use crate::output::write_file;
use crate::output::{out, outln};
use crate::text::{Align, Table, bytes, now_rfc3339, num};

/// Wall-clock time of one tick (one `step()` call) in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TickTimes {
    /// Arithmetic mean.
    pub mean: f64,
    /// Median.
    pub median: f64,
    /// 95th percentile (linear interpolation).
    pub p95: f64,
    /// Fastest tick.
    pub min: f64,
    /// Slowest tick.
    pub max: f64,
    /// Population standard deviation.
    pub std_dev: f64,
}

/// Benchmark results.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BenchReport {
    /// When the benchmark ran (UTC).
    pub timestamp: String,
    /// Model id.
    pub model_id: String,
    /// Scenario id.
    pub scenario_id: String,
    /// Seed.
    pub seed: u64,
    /// Ticks run before measuring.
    pub warmup_ticks: u64,
    /// Ticks measured.
    pub measured_ticks: u64,
    /// The model's tick counter after warm-up and at the end.
    pub ticks_range: (u64, u64),
    /// Simulated time after warm-up and at the end, in `tick_unit`.
    pub sim_time_range: (f64, f64),
    /// Unit of simulated time (the descriptor's tick unit).
    pub tick_unit: String,
    /// Measured ticks per second (measured ticks / summed tick times).
    pub ticks_per_second: f64,
    /// Time per tick.
    pub ms_per_tick: TickTimes,
    /// `memory_bytes()` after warm-up and at the end (`None`: not reported by the model).
    pub memory_bytes: (Option<u64>, Option<u64>),
    /// Why fewer ticks than requested were measured.
    pub stopped: Option<String>,
    /// Machine.
    pub machine: MachineInfo,
}

fn memory(sim: &dyn simulation_core::DynSimulation) -> Option<u64> {
    let bytes = sim.memory_bytes();
    (bytes > 0).then_some(bytes as u64)
}

/// `bench TARGET`.
///
/// # Errors
/// Build failures, a model that cannot run one measured tick, export errors.
pub fn bench(registry: &Registry, args: &BenchArgs) -> anyhow::Result<()> {
    let mut built = build(registry, &request(&args.target, &args.build)?)?;
    let descriptor = built.sim.descriptor();
    let machine = MachineInfo::collect();
    let sim = built.sim.as_mut();
    let mut stopped = None;
    for _ in 0..args.warmup {
        if let Err(reason) = headless::tick(sim) {
            stopped = Some(format!("during the warm-up: {reason}"));
            break;
        }
    }
    let first_tick = sim.tick();
    let first_time = sim.metrics().sim_time;
    let memory_start = memory(sim);
    let mut times_ms = Vec::with_capacity(usize::try_from(args.ticks).unwrap_or(0).min(1 << 24));
    if stopped.is_none() {
        for _ in 0..args.ticks {
            match headless::tick(sim) {
                Ok(elapsed) => times_ms.push(elapsed.as_secs_f64() * 1e3),
                Err(reason) => {
                    stopped = Some(reason);
                    break;
                }
            }
        }
    }
    if times_ms.is_empty() {
        anyhow::bail!("no tick could be measured: {}", stopped.unwrap_or_default());
    }
    let summary = summarize(&times_ms)?;
    let total_ms: f64 = times_ms.iter().sum();
    let report = BenchReport {
        timestamp: now_rfc3339(),
        model_id: descriptor.id.clone(),
        scenario_id: built.scenario.id.clone(),
        seed: built.seed,
        warmup_ticks: args.warmup,
        measured_ticks: times_ms.len() as u64,
        ticks_range: (first_tick, sim.tick()),
        sim_time_range: (first_time, sim.metrics().sim_time),
        tick_unit: descriptor.tick_unit.clone(),
        ticks_per_second: if total_ms > 0.0 { times_ms.len() as f64 * 1e3 / total_ms } else { 0.0 },
        ms_per_tick: TickTimes {
            mean: summary.mean,
            median: summary.median,
            p95: quantile(&times_ms, 0.95)?,
            min: summary.min,
            max: summary.max,
            std_dev: summary.std_dev,
        },
        memory_bytes: (memory_start, memory(sim)),
        stopped,
        machine,
    };
    print_report(&report);
    if let Some(path) = &args.json {
        write_file(path, &serde_json::to_string_pretty(&report)?)?;
        outln!("results written to {}", path.display());
    }
    Ok(())
}

fn print_report(r: &BenchReport) {
    let m = &r.machine;
    outln!(
        "machine   {} | {} logical cores{} | {}{} | {} {}",
        m.cpu.as_deref().unwrap_or("unknown CPU"),
        m.logical_cores,
        m.physical_cores.map_or(String::new(), |p| format!(" ({p} physical)")),
        m.memory_bytes.map_or(String::new(), |b| format!("{} RAM | ", bytes(b))),
        m.os,
        m.arch,
        if m.debug_assertions {
            "| debug assertions on (build with --release for representative numbers)"
        } else {
            "| debug assertions off"
        },
    );
    outln!("rayon     {} threads", m.rayon_threads);
    outln!("target    {} ({}), seed {}", r.scenario_id, r.model_id, r.seed);
    outln!(
        "ticks     {} warm-up + {} measured (tick counter {} -> {}, simulated time {} -> {} {})",
        r.warmup_ticks,
        r.measured_ticks,
        r.ticks_range.0,
        r.ticks_range.1,
        num(r.sim_time_range.0),
        num(r.sim_time_range.1),
        r.tick_unit
    );
    if let Some(reason) = &r.stopped {
        outln!("note      {reason}");
    }
    outln!();
    let t = &r.ms_per_tick;
    let mut table = Table::new(&[("MEASURE", Align::Left), ("VALUE", Align::Right), ("UNIT", Align::Left)]);
    table.row(vec!["ticks per second".into(), num(r.ticks_per_second), "ticks/s".into()]);
    for (name, value) in [("mean", t.mean), ("median", t.median), ("p95", t.p95), ("min", t.min), ("max", t.max)] {
        table.row(vec![format!("time per tick, {name}"), num(value), "ms".into()]);
    }
    let mem = |v: Option<u64>| v.map_or_else(|| "not reported by the model".to_string(), bytes);
    table.row(vec!["model memory after warm-up".into(), mem(r.memory_bytes.0), String::new()]);
    table.row(vec!["model memory at the end".into(), mem(r.memory_bytes.1), String::new()]);
    out!("{}", table.render());
}
