//! Calibration checks of the six shipped presets: each test runs a preset
//! (with its own seed unless stated) long enough to show the outcome that
//! `docs/models/evolution.md` describes, and asserts it qualitatively.
//!
//! They take minutes in the dev profile, so they are ignored by default; run
//! them in release (or the `verify` profile):
//!
//! ```sh
//! cargo test --release -p evolution --test presets -- --ignored --nocapture
//! ```

use evolution::scenarios::find_builtin;
use evolution::{EvolutionSim, Scenario, SpeciationStatus, Species};
use simulation_core::Simulation;

fn preset(id: &str) -> Scenario {
    find_builtin(id).expect("built-in scenario")
}

fn build(scenario: &Scenario) -> EvolutionSim {
    EvolutionSim::from_scenario(scenario).expect("valid scenario")
}

fn metric(sim: &EvolutionSim, key: &str) -> f64 {
    sim.metrics().get(key).unwrap_or(f64::NAN)
}

/// Runs `ticks` ticks and returns the mean of each metric over the last
/// `window` ticks (sampled every 10 ticks).
fn window_means(sim: &mut EvolutionSim, ticks: u64, window: u64, keys: &[&str]) -> Vec<f64> {
    let mut sums = vec![0.0; keys.len()];
    let mut n = 0.0;
    for t in 1..=ticks {
        sim.step();
        if t > ticks - window && t % 10 == 0 {
            for (sum, key) in sums.iter_mut().zip(keys) {
                *sum += metric(sim, key);
            }
            n += 1.0;
        }
    }
    sums.into_iter().map(|s| s / n).collect()
}

#[test]
#[ignore = "calibration run (release): cargo test --release -p evolution --test presets -- --ignored"]
fn speed_selection_raises_prey_speed_above_a_predator_free_control() {
    let mut gains = Vec::new();
    for seed in [11, 13, 17, 19, 23, 29] {
        let mut with = preset("speed_selection");
        with.seed = seed;
        let mut without = with.clone();
        without.config.predators.initial = 0;
        let (mut a, mut b) = (build(&with), build(&without));
        let founders = metric(&a, "mean_speed");
        let p = window_means(&mut a, 2000, 300, &["mean_speed", "mean_perception"]);
        let c = window_means(&mut b, 2000, 300, &["mean_speed", "mean_perception"]);
        println!(
            "speed_selection seed {seed}: speed {founders:.3} -> {:.3} with predators, {:.3} without; \
             perception {:.2} / {:.2}; prey {} / {}",
            p[0],
            c[0],
            p[1],
            c[1],
            a.world().count(Species::Prey),
            b.world().count(Species::Prey)
        );
        assert!(p[0] > c[0], "seed {seed}: faster with predators than without");
        assert!(p[1] < 4.0 && c[1] < 4.0, "perception is costly and falls in both runs");
        gains.push((p[0] - c[0], p[0] - founders));
    }
    let n = gains.len() as f64;
    let (vs_control, vs_founders) = gains.iter().fold((0.0, 0.0), |(a, b), (x, y)| (a + x / n, b + y / n));
    println!("speed_selection: mean gain vs control {vs_control:+.3}, vs founders {vs_founders:+.3}");
    assert!(vs_control > 0.1 && vs_founders > 0.05);
}

#[test]
#[ignore = "calibration run (release): cargo test --release -p evolution --test presets -- --ignored"]
fn thermal_adaptation_tracks_warming_and_fast_warming_is_lethal() {
    let scenario = preset("thermal_adaptation");
    let mut sim = build(&scenario);
    let keys = ["mean_temperature", "mean_temp_optimum", "mean_acclimation", "mean_temp_tolerance"];
    let start: Vec<f64> = keys.iter().map(|k| metric(&sim, k)).collect();
    let mut thermal_deaths = 0.0;
    for _ in 0..4000 {
        sim.step();
        thermal_deaths += metric(&sim, "deaths_thermal");
    }
    let end: Vec<f64> = keys.iter().map(|k| metric(&sim, k)).collect();
    println!(
        "thermal_adaptation: T {:.1} -> {:.1}, optimum {:.1} -> {:.1}, acclimation {:.2}, tolerance {:.2} -> {:.2}, \
         thermal deaths {thermal_deaths}, prey {}",
        start[0],
        end[0],
        start[1],
        end[1],
        end[2],
        start[3],
        end[3],
        sim.world().count(Species::Prey)
    );
    assert!(sim.world().count(Species::Prey) > 50, "the population keeps up with 6 °C per 1000 ticks");
    assert!(end[1] > start[1] + 10.0, "the thermal optimum evolves upwards");
    let lag = end[0] - end[1];
    assert!(lag > 0.0 && lag < 10.0, "optimum lags the climate by {lag:.1} °C");
    assert!(end[2] > 1.0, "acclimation absorbs part of the mismatch");
    assert!(thermal_deaths > 0.0);

    let mut fast = scenario.clone();
    fast.config.climate.shift_rate = 15.0;
    let mut sim = build(&fast);
    let mut extinct_at = None;
    for t in 1..=4000 {
        sim.step();
        if sim.world().count(Species::Prey) == 0 {
            extinct_at = Some(t);
            break;
        }
    }
    println!("thermal_adaptation at 15 °C/1000 ticks: extinct at {extinct_at:?}");
    assert!(extinct_at.is_some(), "warming at 15 °C per 1000 ticks outpaces evolution");
}

#[test]
#[ignore = "calibration run (release): cargo test --release -p evolution --test presets -- --ignored"]
fn arms_race_populations_coexist_and_both_get_faster() {
    for seed in [37, 38, 39] {
        let mut scenario = preset("arms_race");
        scenario.seed = seed;
        let mut sim = build(&scenario);
        let prey_start = metric(&sim, "mean_speed");
        let predator_start = metric(&sim, "predator_mean_speed");
        let means = window_means(&mut sim, 4000, 500, &["mean_speed", "predator_mean_speed", "mean_size"]);
        let counts = (sim.world().count(Species::Prey), sim.world().count(Species::Predator));
        println!(
            "arms_race seed {seed}: prey speed {prey_start:.3} -> {:.3}, predator speed {predator_start:.3} -> {:.3}, \
             prey size {:.2}, populations {counts:?}",
            means[0], means[1], means[2]
        );
        assert!(counts.0 > 0 && counts.1 > 0, "seed {seed}: both populations persist 4000 ticks");
        assert!(means[0] > prey_start + 0.15, "seed {seed}: prey get faster");
        assert!(means[1] > predator_start, "seed {seed}: predators get faster");
    }
}

#[test]
#[ignore = "calibration run (release): cargo test --release -p evolution --test presets -- --ignored"]
fn food_scarcity_favours_search_and_efficiency() {
    let keys = ["mean_perception", "mean_efficiency", "mean_temp_tolerance", "mean_maturity"];
    let (mut scarce, mut abundant) = (vec![0.0; keys.len()], vec![0.0; keys.len()]);
    for seed in [41, 42, 43] {
        for (scale, sums) in [(0.3, &mut scarce), (2.0, &mut abundant)] {
            let mut scenario = preset("food_scarcity");
            scenario.seed = seed;
            scenario.config.food.capacity_scale = scale;
            let mut sim = build(&scenario);
            let means = window_means(&mut sim, 3000, 300, &keys);
            println!("food_scarcity seed {seed}, capacity scale {scale}: {keys:?} = {means:.3?}");
            for (s, m) in sums.iter_mut().zip(means) {
                *s += m / 3.0;
            }
        }
    }
    println!("food_scarcity mean of 3 seeds: scarce {scarce:.3?}, abundant {abundant:.3?}");
    assert!(scarce[0] > abundant[0], "scarcity favours a longer search radius");
    assert!(scarce[1] > abundant[1], "scarcity favours metabolic efficiency (yield over rate)");
}

#[test]
#[ignore = "calibration run (release): cargo test --release -p evolution --test presets -- --ignored"]
fn island_evolution_diverges_and_meets_the_speciation_criterion() {
    let mut sim = build(&preset("island_evolution"));
    let mut fst_late = Vec::new();
    let mut declared = None;
    for t in 1..=9000u64 {
        sim.step();
        if t >= 1500 && t % 100 == 0 {
            fst_late.push(metric(&sim, "fst"));
        }
        if declared.is_none() && matches!(sim.world().speciation().status(), SpeciationStatus::Speciated(_)) {
            declared = Some((t, metric(&sim, "mean_generation")));
        }
        if t % 1500 == 0 {
            println!(
                "island_evolution tick {t}: fst {:.3}, region divergence {:.3}, cluster distance {:.3}, \
                 mean generation {:.1}, prey {}",
                metric(&sim, "fst"),
                metric(&sim, "region_divergence"),
                metric(&sim, "cluster_distance"),
                metric(&sim, "mean_generation"),
                sim.world().count(Species::Prey)
            );
        }
    }
    let mean_fst = fst_late.iter().sum::<f64>() / fst_late.len() as f64;
    println!("island_evolution: mean F_ST from tick 1500 {mean_fst:.3}; speciation declared at {declared:?}");
    assert!(mean_fst > 0.2, "isolated islands are strongly differentiated");
    assert!(declared.is_some(), "the explicit criterion is met within 9000 ticks");
}

#[test]
#[ignore = "calibration run (release): cargo test --release -p evolution --test presets -- --ignored"]
fn r_vs_k_outcome_depends_on_mortality_and_food_distribution() {
    let scenario = preset("r_vs_k");
    let mut high_mortality = scenario.clone();
    high_mortality.config.physiology.background_mortality = 0.008;
    // The same total capacity spread evenly: 4 discs of 154 cells at 10 plus
    // 2456 cells at 0.5 ≈ 2.4 per cell.
    let mut uniform = scenario.clone();
    uniform.config.food.patches.clear();
    uniform.config.food.capacity = 2.4;
    let cases = [
        ("patchy food", &scenario, true),
        ("background mortality 0.008", &high_mortality, false),
        ("uniform food", &uniform, false),
    ];
    for (label, s, k_wins) in cases {
        let mut sim = build(s);
        let means = window_means(&mut sim, 5000, 500, &["mean_marker_a", "mean_clutch", "mean_maturity"]);
        println!(
            "r_vs_k ({label}): K marker {:.3}, clutch {:.2}, maturity {:.0}, prey {}",
            means[0],
            means[1],
            means[2],
            sim.world().count(Species::Prey)
        );
        if k_wins {
            assert!(means[0] > 0.7 && means[1] < 2.5, "{label}: the K strategy takes over");
        } else {
            assert!(means[0] < 0.3 && means[1] > 4.0, "{label}: the r strategy takes over");
        }
    }
}
