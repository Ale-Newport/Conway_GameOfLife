//! Statistical selection test: predation selects for prey speed.
//!
//! Paired design on the `speed_selection` preset, scaled down to stay fast:
//! a 48 × 40 world, 250 founding prey and 30 fixed predators whose top speed
//! is set to 0.75, the middle of the founders' speed range (the preset's
//! larger world uses 40 predators of speed 0.85). For each seed the same
//! founding prey (same founder stream) evolve in three arms run in lockstep:
//!
//! * **predators**: the constant predation regime (predators neither breed
//!   nor die);
//! * **no predators**: the plain control asked for by the scope;
//! * **random deaths**: no predators, but every tick prey are removed *at
//!   random* (`Cull` command) at the per-capita rate at which the predators
//!   killed prey in the first arm. Predation also lowers prey density and so
//!   relaxes food competition, which by itself weakens selection against
//!   costly speed; this arm has the same mortality without the
//!   selectivity.
//!
//! No fitness is computed anywhere. The test checks the *mechanism* and the
//! *response*:
//!
//! 1. **Selection differential of predation** `S = mean(v_killed − v̄)`:
//!    on ticks where predation was the only cause of death, every prey that
//!    vanished was killed, so its speed gene is compared with the mean of
//!    the prey alive at the start of that tick. Pooled over all seeds, `S`
//!    must be negative with an approximate z-score below −3 (predators catch
//!    slower prey). The same measurement on the random culls is the negative
//!    control (|z| < 3), which shows the estimator is unbiased.
//! 2. **Response**: for every seed the mean speed over the last 300 ticks is
//!    higher with predators than without; averaged over the seeds it is
//!    above the founders' mean and above the random-deaths control by more
//!    than 0.05.
//!
//! Measured values (dev and release profiles give identical numbers) are
//! recorded in `docs/models/evolution.md`. Replicate runs differ a lot (drift
//! and hitch-hiking in populations of a few hundred), so the comparison with
//! the random-deaths control is made on the seed average, not per seed.

use std::collections::BTreeMap;

use evolution::genes::SPEED;
use evolution::scenarios::find_builtin;
use evolution::world::DeathCause;
use evolution::{EvolutionCommand, EvolutionSim, Species};
use organism_engine::AgentId;
use simulation_core::Simulation;

const TICKS: u64 = 1500;
const WINDOW_START: u64 = 1200;
const SEEDS: [u64; 4] = [1, 2, 3, 4];
/// Top speed of the fixed predators (the founders' speeds are about 0.4–0.8).
const PREDATOR_SPEED: f32 = 0.75;

fn build(seed: u64, predators: u32) -> EvolutionSim {
    let mut sc = find_builtin("speed_selection").expect("built-in scenario");
    sc.seed = seed;
    sc.config.world.width = 48;
    sc.config.world.height = 40;
    sc.config.prey.initial = 250;
    sc.config.predators.initial = predators;
    for t in &mut sc.config.predators.traits {
        if t.gene == "speed" {
            t.value = PREDATOR_SPEED;
        }
    }
    EvolutionSim::from_scenario(&sc).expect("valid scenario")
}

/// Speed gene of every living prey, by id.
fn prey_speeds(sim: &EvolutionSim) -> BTreeMap<AgentId, f64> {
    let w = sim.world();
    w.organisms()
        .iter()
        .filter(|(_, o)| o.species == Species::Prey)
        .map(|(id, o)| (id, f64::from(o.genome.express(w.schema(), SPEED))))
        .collect()
}

/// Appends `v − mean(before)` for every prey of `before` missing from `after`.
fn record_removed(before: &BTreeMap<AgentId, f64>, after: &BTreeMap<AgentId, f64>, out: &mut Vec<f64>) {
    let mean = before.values().sum::<f64>() / before.len() as f64;
    out.extend(before.iter().filter(|(id, _)| !after.contains_key(id)).map(|(_, v)| v - mean));
}

struct Outcome {
    founders: f64,
    /// Window means of `mean_speed`: predators, no predators, random deaths.
    arms: [f64; 3],
    /// Speed deviations of prey killed by predators (attributable ticks).
    killed: Vec<f64>,
    /// Speed deviations of prey removed at random.
    culled: Vec<f64>,
}

fn run(seed: u64) -> Outcome {
    let mut arms = [build(seed, 30), build(seed, 0), build(seed, 0)];
    let founders = arms.each_ref().map(|s| s.metrics().get("mean_speed").unwrap_or(f64::NAN));
    assert!(founders.iter().all(|f| *f == founders[0]), "paired design: identical founders");
    let (mut sums, mut n) = ([0.0; 3], 0.0);
    let (mut killed, mut culled) = (Vec::new(), Vec::new());
    // Expected random deaths not yet applied, so that rounding to whole
    // organisms keeps the matched rate unbiased.
    let mut owed = 0.0f64;
    for t in 1..=TICKS {
        let exposed = prey_speeds(&arms[0]);
        for sim in &mut arms {
            sim.step();
        }
        let report = arms[0].world().last_report();
        let kills = report.deaths[DeathCause::Predation.index()];
        if kills > 0 && report.total_deaths() == kills {
            record_removed(&exposed, &prey_speeds(&arms[0]), &mut killed);
        }
        let available = arms[2].world().count(Species::Prey);
        if !exposed.is_empty() && available > 0 {
            owed += f64::from(kills) / exposed.len() as f64 * available as f64;
            let k = (owed.floor() as usize).min(available);
            if k > 0 {
                let before = prey_speeds(&arms[2]);
                let fraction = k as f32 / available as f32;
                let cull = EvolutionCommand::Cull { fraction, species: Some(Species::Prey), area: None };
                arms[2].apply(cull).expect("valid cull");
                record_removed(&before, &prey_speeds(&arms[2]), &mut culled);
                owed -= k as f64;
            }
        }
        for (i, sim) in arms.iter().enumerate() {
            assert!(sim.world().count(Species::Prey) > 50, "prey must persist (seed {seed}, arm {i}, tick {t})");
        }
        if t > WINDOW_START && t % 10 == 0 {
            for (sum, sim) in sums.iter_mut().zip(&arms) {
                *sum += sim.metrics().get("mean_speed").unwrap_or(f64::NAN);
            }
            n += 1.0;
        }
    }
    Outcome { founders: founders[0], arms: sums.map(|s| s / n), killed, culled }
}

/// Mean and approximate z-score (mean / standard error) of a sample.
fn z_score(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, mean / (var / n).sqrt())
}

#[test]
fn predation_selects_for_prey_speed() {
    let outcomes: Vec<(u64, Outcome)> = SEEDS.iter().map(|&seed| (seed, run(seed))).collect();
    for (seed, o) in &outcomes {
        let [predators, none, random] = o.arms;
        println!(
            "seed {seed}: founders {:.3}, predators {predators:.3}, no predators {none:.3}, random deaths {random:.3}; \
             S(killed) {:+.4} over {} kills, S(culled) {:+.4} over {} culls",
            o.founders,
            z_score(&o.killed).0,
            o.killed.len(),
            z_score(&o.culled).0,
            o.culled.len()
        );
    }
    let pooled = |f: fn(&Outcome) -> &Vec<f64>| outcomes.iter().flat_map(|(_, o)| f(o).iter().copied()).collect();
    let killed: Vec<f64> = pooled(|o| &o.killed);
    let culled: Vec<f64> = pooled(|o| &o.culled);
    let (s_kill, z_kill) = z_score(&killed);
    let (s_cull, z_cull) = z_score(&culled);
    println!(
        "pooled: S(killed) {s_kill:+.4} (z {z_kill:.1}, n {}), S(culled) {s_cull:+.4} (z {z_cull:.1}, n {})",
        killed.len(),
        culled.len()
    );
    assert!(killed.len() > 300 && culled.len() > 2000, "enough deaths to measure");
    assert!(z_kill < -3.0, "predators catch slower than average prey: S {s_kill:+.4}, z {z_kill:.1}");
    assert!(z_cull.abs() < 3.0, "random culls are unselective: S {s_cull:+.4}, z {z_cull:.1}");

    let mean = |f: fn(&Outcome) -> f64| outcomes.iter().map(|(_, o)| f(o)).sum::<f64>() / outcomes.len() as f64;
    for (seed, o) in &outcomes {
        let [predators, none, _] = o.arms;
        assert!(predators > none, "seed {seed}: {predators:.3} with predators vs {none:.3} without");
    }
    let vs_none = mean(|o| o.arms[0] - o.arms[1]);
    let vs_random = mean(|o| o.arms[0] - o.arms[2]);
    let vs_founders = mean(|o| o.arms[0] - o.founders);
    println!(
        "mean differences: vs no predators {vs_none:+.3}, vs random deaths {vs_random:+.3}, vs founders {vs_founders:+.3}"
    );
    assert!(vs_none > 0.05, "vs no predators: {vs_none:+.3}");
    assert!(vs_random > 0.05, "vs random deaths at the same rate: {vs_random:+.3}");
    assert!(vs_founders > 0.05, "a response to selection, not only relaxed costs: {vs_founders:+.3}");
}
