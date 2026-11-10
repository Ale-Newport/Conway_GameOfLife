//! `compare`: replicate runs over seeds and parameter values.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use analytics::{ComparisonTable, RunRecord};
use anyhow::{Context, bail};
use model_registry::{BuildRequest, ParamOverride, Registry};
use rayon::prelude::*;
use simulation_core::ParamValue;

use crate::cli::CompareArgs;
use crate::commands::build;
use crate::headless::{self, RunSettings};
use crate::output::{ExportFormat, write_file};
use crate::output::{out, outln};
use crate::text::{Align, Table, now_rfc3339, num};

/// Largest number of runs one comparison may start.
pub const MAX_RUNS: usize = 10_000;

/// Parses `a..b` (b excluded), `a..=b` or `a,b,c`.
///
/// # Errors
/// Malformed numbers, empty or oversized ranges.
pub fn parse_seeds(text: &str) -> anyhow::Result<Vec<u64>> {
    let number = |s: &str| s.trim().parse::<u64>().with_context(|| format!("`{}` is not a seed (0 or more)", s.trim()));
    let text = text.trim();
    let seeds: Vec<u64> = if let Some((a, b)) = text.split_once("..=") {
        let (a, b) = (number(a)?, number(b)?);
        if a > b {
            bail!("seed range {text} is empty");
        }
        if b - a >= MAX_RUNS as u64 {
            bail!("seed range {text} has more than {MAX_RUNS} seeds");
        }
        (a..=b).collect()
    } else if let Some((a, b)) = text.split_once("..") {
        let (a, b) = (number(a)?, number(b)?);
        if a >= b {
            bail!("seed range {text} is empty (the end is excluded; use a..=b to include it)");
        }
        if b - a > MAX_RUNS as u64 {
            bail!("seed range {text} has more than {MAX_RUNS} seeds");
        }
        (a..b).collect()
    } else {
        text.split(',').map(number).collect::<anyhow::Result<_>>()?
    };
    Ok(seeds)
}

/// Parses a sweep `key=v1,v2,...`.
///
/// # Errors
/// Missing `=`, empty key or no values.
pub fn parse_sweep(text: &str) -> anyhow::Result<(String, Vec<String>)> {
    let Some((key, values)) = text.split_once('=') else {
        bail!("parameter sweep `{text}` must be written as key=v1,v2,...");
    };
    let values: Vec<String> = values.split(',').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect();
    if key.trim().is_empty() || values.is_empty() {
        bail!("parameter sweep `{text}` needs a key and at least one value");
    }
    Ok((key.trim().to_string(), values))
}

/// One run of the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSpec {
    /// Seed (`None`: the scenario's).
    pub seed: Option<u64>,
    /// Parameter values of this run.
    pub params: Vec<ParamOverride>,
}

/// Every combination of seeds and sweep values (seeds vary slowest).
///
/// # Errors
/// Grids larger than [`MAX_RUNS`].
pub fn grid(seeds: &[Option<u64>], sweeps: &[(String, Vec<String>)]) -> anyhow::Result<Vec<RunSpec>> {
    let mut combos: Vec<Vec<ParamOverride>> = vec![Vec::new()];
    for (key, values) in sweeps {
        combos = combos
            .iter()
            .flat_map(|base| {
                values.iter().map(move |v| {
                    let mut next = base.clone();
                    next.push(ParamOverride::new(key, ParamValue::Text(v.clone())));
                    next
                })
            })
            .collect();
        if combos.len().saturating_mul(seeds.len()) > MAX_RUNS {
            bail!("the comparison would start more than {MAX_RUNS} runs");
        }
    }
    Ok(seeds
        .iter()
        .flat_map(|&seed| combos.iter().map(move |params| RunSpec { seed, params: params.clone() }))
        .collect())
}

/// Label of a finished run.
fn label(seed: u64, params: &[ParamOverride]) -> String {
    let mut parts = vec![format!("seed={seed}")];
    parts.extend(params.iter().map(|p| format!("{}={}", p.key, p.value)));
    parts.join(" ")
}

/// Label of a requested run.
fn spec_label(spec: &RunSpec) -> String {
    match spec.seed {
        Some(seed) => label(seed, &spec.params),
        None => {
            let mut parts = vec!["scenario seed".to_string()];
            parts.extend(spec.params.iter().map(|p| format!("{}={}", p.key, p.value)));
            parts.join(" ")
        }
    }
}

struct Finished {
    record: RunRecord,
    end_tick: u64,
    elapsed: Duration,
    stopped: Option<String>,
}

fn run_one(registry: &Registry, target: &str, spec: &RunSpec, settings: RunSettings) -> anyhow::Result<Finished> {
    let request = BuildRequest { target: target.to_string(), seed: spec.seed, params: spec.params.clone() };
    let mut built = build(registry, &request)?;
    let descriptor = built.sim.descriptor();
    let mut record = RunRecord::new(
        &label(built.seed, &spec.params),
        &descriptor.id,
        &built.scenario.id,
        built.seed,
        built.sim.params(),
        &now_rfc3339(),
    );
    let summary = headless::run(built.sim.as_mut(), &mut record, settings, |_| {})?;
    Ok(Finished { record, end_tick: summary.end_tick, elapsed: summary.elapsed, stopped: summary.stopped })
}

/// Aggregate table: per metric, the distribution of final values across
/// runs and the mean over time averaged across runs.
fn metrics_table(comparison: &ComparisonTable, only: &[String]) -> anyhow::Result<Table> {
    for key in only {
        if comparison.metric(key).is_none() {
            let known: Vec<&str> = comparison.metrics.iter().map(|m| m.key.as_str()).collect();
            bail!("no metric `{key}` was recorded; recorded metrics: {}", known.join(", "));
        }
    }
    let mut table = Table::new(&[
        ("METRIC", Align::Left),
        ("RUNS", Align::Right),
        ("FINAL MEAN", Align::Right),
        ("FINAL SD", Align::Right),
        ("FINAL MIN", Align::Right),
        ("FINAL MEDIAN", Align::Right),
        ("FINAL MAX", Align::Right),
        ("MEAN OVER TIME", Align::Right),
    ]);
    for metric in comparison.metrics.iter().filter(|m| only.is_empty() || only.contains(&m.key)) {
        let Some(finals) = metric.final_across_runs else { continue };
        let means: Vec<f64> = metric.per_run.iter().flatten().map(|s| s.mean).collect();
        let mean_over_time = means.iter().sum::<f64>() / means.len().max(1) as f64;
        table.row(vec![
            metric.key.clone(),
            finals.count.to_string(),
            num(finals.mean),
            num(finals.std_dev),
            num(finals.min),
            num(finals.median),
            num(finals.max),
            num(mean_over_time),
        ]);
    }
    Ok(table)
}

/// `compare TARGET --seeds ... --param key=v1,v2`.
///
/// # Errors
/// Invalid seeds or sweeps, failing runs, export errors.
pub fn compare(registry: &Registry, args: &CompareArgs) -> anyhow::Result<()> {
    let export = args.out.as_deref().map(ExportFormat::of).transpose()?;
    if args.seeds.is_none() && args.params.is_empty() {
        bail!("give --seeds and/or at least one --param key=v1,v2,... to compare");
    }
    let seeds: Vec<Option<u64>> = match &args.seeds {
        Some(text) => parse_seeds(text)?.into_iter().map(Some).collect(),
        None => vec![None],
    };
    let sweeps = args.params.iter().map(|p| parse_sweep(p)).collect::<anyhow::Result<Vec<_>>>()?;
    let specs = grid(&seeds, &sweeps)?;
    // Resolve the target once so that unknown ids fail before any run starts.
    registry.resolve(&args.target).map_err(anyhow::Error::new)?;

    let settings = RunSettings { ticks: args.ticks, metrics_every: args.metrics_every };
    let threads = args.jobs.map_or(0, |j| j as usize);
    let pool =
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().context("cannot start the worker threads")?;
    let finished_count = AtomicUsize::new(0);
    let total = specs.len();
    if !args.quiet {
        eprintln!("{total} runs of {} ticks on {} threads", args.ticks, pool.current_num_threads());
    }
    // After a failure, runs that have not started yet are skipped (`None`).
    let failed = AtomicBool::new(false);
    let results: Vec<Option<anyhow::Result<Finished>>> = pool.install(|| {
        specs
            .par_iter()
            .map(|spec| {
                if failed.load(Ordering::Relaxed) {
                    return None;
                }
                let result = run_one(registry, &args.target, spec, settings);
                let n = finished_count.fetch_add(1, Ordering::Relaxed) + 1;
                match &result {
                    Ok(f) if !args.quiet => {
                        eprintln!("run {n}/{total} done: {} ({:.2} s)", f.record.run_id, f.elapsed.as_secs_f64());
                    }
                    Ok(_) => {}
                    Err(_) => failed.store(true, Ordering::Relaxed),
                }
                Some(result)
            })
            .collect()
    });
    if let Some((spec, error)) =
        specs.iter().zip(&results).find_map(|(spec, r)| r.as_ref().and_then(|r| r.as_ref().err()).map(|e| (spec, e)))
    {
        let skipped = results.iter().filter(|r| r.is_none()).count();
        let note = if skipped > 0 { format!(" ({skipped} other run(s) were not started)") } else { String::new() };
        bail!("run `{}` failed{note}: {error:#}", spec_label(spec));
    }
    let runs: Vec<Finished> = results.into_iter().flatten().collect::<anyhow::Result<_>>()?;

    let mut table = Table::new(&[
        ("RUN", Align::Left),
        ("SCENARIO", Align::Left),
        ("END TICK", Align::Right),
        ("TIME S", Align::Right),
        ("NOTE", Align::Left),
    ]);
    for f in &runs {
        table.row(vec![
            f.record.run_id.clone(),
            f.record.scenario.clone(),
            f.end_tick.to_string(),
            format!("{:.3}", f.elapsed.as_secs_f64()),
            f.stopped.clone().unwrap_or_default(),
        ]);
    }
    out!("{}", table.render());
    outln!();
    let records: Vec<RunRecord> = runs.into_iter().map(|f| f.record).collect();
    let comparison = analytics::compare_runs(&records);
    out!("{}", metrics_table(&comparison, &args.metrics)?.render());

    if let (Some(path), Some(format)) = (&args.out, export) {
        let text = match format {
            ExportFormat::Csv => comparison.to_csv(),
            ExportFormat::Json => analytics::comparison_to_json(&comparison, true)?,
        };
        write_file(path, &text)?;
        outln!("summary of {} runs written to {}", records.len(), path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_lists() {
        assert_eq!(parse_seeds("1..4").unwrap(), [1, 2, 3]);
        assert_eq!(parse_seeds("1..=4").unwrap(), [1, 2, 3, 4]);
        assert_eq!(parse_seeds("7, 2,9").unwrap(), [7, 2, 9]);
        assert!(parse_seeds("4..4").unwrap_err().to_string().contains("empty"));
        assert!(parse_seeds("0..=20000").is_err());
        assert!(parse_seeds("a..3").unwrap_err().to_string().contains("`a` is not a seed"));
    }

    #[test]
    fn grids_combine_seeds_and_sweeps() {
        let sweeps = vec![parse_sweep("rule=B3/S23,B36/S23").unwrap(), parse_sweep("width=32").unwrap()];
        let runs = grid(&[Some(1), Some(2)], &sweeps).unwrap();
        assert_eq!(runs.len(), 4);
        assert_eq!(runs[1].seed, Some(1));
        assert_eq!(runs[1].params[0].value, ParamValue::Text("B36/S23".into()));
        assert_eq!(runs[1].params[1].key, "width");
        assert!(parse_sweep("rule=").is_err());
        assert!(parse_sweep("rule").is_err());
    }
}
