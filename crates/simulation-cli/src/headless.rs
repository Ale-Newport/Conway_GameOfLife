//! The headless run loop shared by `run`, `compare` and `bench`.
//!
//! One tick is one call to the model's `step()`, exactly as the desktop
//! application's `Runner` advances a model: `generations_per_tick`
//! generations for Life, `2^step_exponent` generations for Hashlife, the
//! `step` parameter's generations for the metapixel (one metageneration by
//! default), one time step for the other models. Step-size parameters given
//! with `--param` therefore act in the command-line tool as they do in the
//! application.

use std::time::{Duration, Instant};

use analytics::RunRecord;
use anyhow::Context;
use simulation_core::DynSimulation;

/// How long and how densely to record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunSettings {
    /// Ticks to run.
    pub ticks: u64,
    /// Ticks between metric samples (at least 1).
    pub metrics_every: u64,
}

/// Progress after one batch of ticks.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    /// Ticks run so far.
    pub done: u64,
    /// Ticks requested.
    pub total: u64,
    /// The model's tick counter.
    pub tick: u64,
    /// Time since the start of the run.
    pub elapsed: Duration,
}

/// What a run did.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSummary {
    /// Ticks run (calls to `step()` that advanced the model).
    pub ticks_run: u64,
    /// The model's tick counter at the start.
    pub start_tick: u64,
    /// The model's tick counter at the end.
    pub end_tick: u64,
    /// Simulated time at the start, in the descriptor's tick unit.
    pub start_time: f64,
    /// Simulated time at the end.
    pub end_time: f64,
    /// Wall-clock time spent advancing and measuring.
    pub elapsed: Duration,
    /// Why the run stopped before the requested ticks, if it did.
    pub stopped: Option<String>,
}

impl RunSummary {
    /// Ticks per second of wall-clock time.
    pub fn ticks_per_second(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs > 0.0 { self.ticks_run as f64 / secs } else { 0.0 }
    }
}

/// Advances one tick and returns the wall-clock time of the `step()` call
/// alone (monotonic clock). Returns why the model could not advance, if it
/// could not: it reports that its run has finished, or its tick counter did
/// not move (a halted model).
pub fn tick(sim: &mut dyn DynSimulation) -> Result<Duration, String> {
    if sim.is_finished() {
        return Err(format!("the model reports that the run has finished at tick {}", sim.tick()));
    }
    let before = sim.tick();
    let start = Instant::now();
    sim.step();
    let elapsed = start.elapsed();
    if sim.tick() == before {
        return Err(format!("the model stopped advancing at tick {before} (halted or finished)"));
    }
    Ok(elapsed)
}

/// Runs `settings.ticks` ticks, recording the metrics before the first tick
/// and after every `metrics_every` ticks (and after the last one) into
/// `record`. `on_progress` is called after every recorded batch. Counter
/// metrics are sampled: a sample holds the value of its own tick.
///
/// # Errors
/// A metric sample the record rejects (ticks not increasing).
pub fn run(
    sim: &mut dyn DynSimulation,
    record: &mut RunRecord,
    settings: RunSettings,
    mut on_progress: impl FnMut(&Progress),
) -> anyhow::Result<RunSummary> {
    let start = Instant::now();
    let initial = sim.metrics();
    let start_tick = sim.tick();
    record.record(&initial).context("recording the initial metrics")?;
    let mut done = 0;
    let mut stopped = None;
    while done < settings.ticks && stopped.is_none() {
        let batch = settings.metrics_every.min(settings.ticks - done);
        let mut advanced = 0;
        while advanced < batch {
            if let Err(reason) = tick(sim) {
                stopped = Some(reason);
                break;
            }
            advanced += 1;
        }
        if advanced == 0 {
            break;
        }
        done += advanced;
        record.record(&sim.metrics()).with_context(|| format!("recording the metrics of tick {}", sim.tick()))?;
        on_progress(&Progress { done, total: settings.ticks, tick: sim.tick(), elapsed: start.elapsed() });
    }
    let last = sim.metrics();
    Ok(RunSummary {
        ticks_run: done,
        start_tick,
        end_tick: sim.tick(),
        start_time: initial.sim_time,
        end_time: last.sim_time,
        elapsed: start.elapsed(),
        stopped,
    })
}

#[cfg(test)]
mod tests {
    use model_registry::{BuildRequest, ParamOverride, Registry};
    use simulation_core::ParamSet;

    use super::*;

    #[test]
    fn records_initial_state_and_every_batch() {
        let registry = Registry::builtin();
        let mut built = registry.build(&BuildRequest::new("ca-lab/wolfram-rule-30")).unwrap();
        let mut record = RunRecord::new("r", "ca.wolfram", "ca-lab/wolfram-rule-30", 0, ParamSet::new(), "t");
        let mut calls = 0;
        let summary =
            run(built.sim.as_mut(), &mut record, RunSettings { ticks: 10, metrics_every: 4 }, |_| calls += 1).unwrap();
        assert_eq!((summary.ticks_run, summary.start_tick, summary.end_tick), (10, 0, 10));
        assert_eq!((summary.start_time, summary.end_time), (0.0, 10.0));
        assert_eq!(calls, 3, "batches of 4, 4 and 2");
        let ticks: Vec<u64> = record.series.iter().next().unwrap().ticks().collect();
        assert_eq!(ticks, [0, 4, 8, 10]);
        assert_eq!(record.ticks, 10);
        assert!(summary.stopped.is_none());
    }

    #[test]
    fn a_tick_is_one_step_with_the_models_step_size() {
        // Hashlife: one tick advances 2^step_exponent generations.
        let registry = Registry::builtin();
        let request = BuildRequest::new("hashlife/glider").param(ParamOverride::parse("step_exponent=3").unwrap());
        let mut built = registry.build(&request).unwrap();
        let mut record = RunRecord::new("r", "life.hashlife", "hashlife/glider", 0, ParamSet::new(), "t");
        let summary = run(built.sim.as_mut(), &mut record, RunSettings { ticks: 5, metrics_every: 1 }, |_| {}).unwrap();
        assert_eq!(summary.ticks_run, 5);
        assert_eq!(summary.end_tick, 40, "5 ticks of 8 generations");
        let ticks: Vec<u64> = record.series.iter().next().unwrap().ticks().collect();
        assert_eq!(ticks, [0, 8, 16, 24, 32, 40]);
    }
}
