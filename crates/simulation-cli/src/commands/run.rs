//! `run`: one headless experiment.

use std::time::{Duration, Instant};

use analytics::{MetricStats, RunRecord};
use anyhow::{Context, bail};
use model_registry::{AppliedOverride, ParamOverride, Registry, RegistryError, SEED_PARAM};
use persistence::Codec;
use simulation_core::{ApplyPolicy, DynSimulation, ModelDescriptor, ParamValue};

use crate::cli::RunArgs;
use crate::commands::{build, request};
use crate::headless::{self, Progress, RunSettings};
use crate::output::{ExportFormat, flat, write_file};
use crate::output::{out, outln};
use crate::text::{Align, Table, now_rfc3339, num, param_value};

/// Overrides as `key=value (policy)`.
pub fn overrides_text(overrides: &[AppliedOverride]) -> String {
    if overrides.is_empty() {
        return "none".to_string();
    }
    overrides
        .iter()
        .map(|o| format!("{}={} ({})", o.key, param_value(&o.value), o.policy.label()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Table of every recorded metric: final value and statistics over the run.
pub fn metrics_table(record: &RunRecord, descriptor: &ModelDescriptor) -> Table {
    let mut table = Table::new(&[
        ("METRIC", Align::Left),
        ("FINAL", Align::Right),
        ("MEAN", Align::Right),
        ("MIN", Align::Right),
        ("MAX", Align::Right),
        ("UNIT", Align::Left),
    ]);
    for series in record.series.iter() {
        let Some(stats) = MetricStats::from_series(series, 0.0) else { continue };
        let unit = descriptor.metrics.iter().find(|m| m.key == series.key()).map_or("", |m| m.unit.as_str());
        table.row(vec![
            series.key().to_string(),
            num(stats.final_value),
            num(stats.mean),
            num(stats.min),
            num(stats.max),
            unit.to_string(),
        ]);
    }
    table
}

/// Progress printer: one line to stderr at most every second.
pub struct ProgressPrinter {
    last: Instant,
}

impl ProgressPrinter {
    /// A printer whose first line comes after one second.
    pub fn new() -> Self {
        Self { last: Instant::now() }
    }

    /// Prints `p` if a second has passed since the last line or the run is complete.
    pub fn update(&mut self, p: &Progress) {
        if self.last.elapsed() < Duration::from_secs(1) && p.done < p.total {
            return;
        }
        self.last = Instant::now();
        let secs = p.elapsed.as_secs_f64();
        let rate = if secs > 0.0 { p.done as f64 / secs } else { 0.0 };
        eprintln!(
            "tick {} | {}/{} ticks ({:.0}%) | {:.1} s | {} ticks/s",
            p.tick,
            p.done,
            p.total,
            100.0 * p.done as f64 / p.total.max(1) as f64,
            secs,
            num(rate)
        );
    }
}

/// Applies overrides to a restored state. Parameters that only take effect
/// on reset are refused: a reset would discard the restored state.
fn apply_after_restore(
    sim: &mut dyn DynSimulation,
    overrides: &[ParamOverride],
) -> anyhow::Result<Vec<AppliedOverride>> {
    let mut applied = Vec::new();
    for o in overrides {
        let descriptor = sim.descriptor();
        let spec = descriptor.param(&o.key).ok_or_else(|| RegistryError::UnknownParameter {
            model: descriptor.id.clone(),
            key: o.key.clone(),
            known: descriptor.params.iter().map(|p| p.key.clone()).collect(),
        })?;
        if spec.apply == ApplyPolicy::RequiresReset {
            bail!("parameter `{}` requires a reset, which would discard the resumed state", o.key);
        }
        let value = model_registry::coerce(spec, &o.value)?;
        let policy = sim.set_param(&o.key, value.clone()).with_context(|| format!("parameter `{}`", o.key))?;
        applied.push(AppliedOverride { key: o.key.clone(), value, policy });
    }
    Ok(applied)
}

/// `run TARGET`.
///
/// # Errors
/// Build, resume, export and snapshot errors.
pub fn run(registry: &Registry, args: &RunArgs) -> anyhow::Result<()> {
    let export = args.out.as_deref().map(ExportFormat::of).transpose()?;
    let mut request = request(&args.target, &args.build)?;
    let codec = Codec::new();
    let mut built = match &args.resume {
        None => build(registry, &request)?,
        Some(path) => {
            // The snapshot replaces the whole state; overrides apply afterwards.
            let overrides = std::mem::take(&mut request.params);
            let mut built = build(registry, &request)?;
            let model = built.sim.descriptor().id;
            let snapshot =
                codec.load_snapshot(path, Some(&model)).map_err(|e| flat(e).context("cannot resume"))?.into_payload();
            built.sim.restore(&snapshot).with_context(|| format!("cannot restore {}", path.display()))?;
            // The seed of the resumed run, when the model reports it as a parameter.
            let seed =
                built.sim.params().get(SEED_PARAM).and_then(ParamValue::as_i64).and_then(|s| u64::try_from(s).ok());
            match seed {
                Some(seed) => built.seed = seed,
                None if !args.quiet => eprintln!(
                    "note: `{model}` does not report the seed of the restored run; the record says seed {} \
                     (give the seed of the saved run with --seed)",
                    built.seed
                ),
                None => {}
            }
            built.overrides = apply_after_restore(built.sim.as_mut(), &overrides)?;
            built
        }
    };
    let descriptor = built.sim.descriptor();
    let s = &built.scenario;
    if !args.quiet {
        outln!("model     {} ({})", descriptor.id, descriptor.name);
        outln!("scenario  {} — {} (seed {})", s.id, s.title, built.seed);
        outln!("overrides {}", overrides_text(&built.overrides));
        if let Some(path) = &args.resume {
            outln!("resumed   {} at tick {}", path.display(), built.sim.tick());
        }
    }

    let started_at = now_rfc3339();
    let mut record = RunRecord::new(
        &format!("{}@{}", s.id, built.seed),
        &descriptor.id,
        &s.id,
        built.seed,
        built.sim.params(),
        &started_at,
    );
    let settings = RunSettings { ticks: args.ticks, metrics_every: args.metrics_every };
    let mut printer = ProgressPrinter::new();
    let summary = headless::run(built.sim.as_mut(), &mut record, settings, |p| {
        if !args.quiet {
            printer.update(p);
        }
    })?;

    if !args.quiet {
        outln!(
            "ran       {} ticks in {:.3} s ({} ticks/s): tick counter {} -> {}, simulated time {} -> {} {}",
            summary.ticks_run,
            summary.elapsed.as_secs_f64(),
            num(summary.ticks_per_second()),
            summary.start_tick,
            summary.end_tick,
            num(summary.start_time),
            num(summary.end_time),
            descriptor.tick_unit
        );
        outln!();
    }
    if let Some(reason) = &summary.stopped {
        eprintln!("note: stopped early: {reason}");
    }
    out!("{}", metrics_table(&record, &descriptor).render());

    if let (Some(path), Some(format)) = (&args.out, export) {
        let text = match format {
            ExportFormat::Csv => analytics::run_to_csv(&record),
            ExportFormat::Json => analytics::run_to_json(&record, true)?,
        };
        write_file(path, &text)?;
        if !args.quiet {
            outln!("metrics written to {} ({} values)", path.display(), record.series.total_samples());
        }
    }
    if let Some(path) = &args.snapshot_out {
        let snapshot = built.sim.snapshot().context("the model could not take a snapshot")?;
        codec.save_snapshot(path, &snapshot, &started_at).map_err(|e| flat(e).context("cannot save the snapshot"))?;
        if !args.quiet {
            outln!("snapshot of tick {} written to {}", snapshot.tick, path.display());
        }
    }
    Ok(())
}
