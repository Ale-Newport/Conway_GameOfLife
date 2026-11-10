//! The NetLogo Wolf Sheep Predation rule variant.

use ecosystem::scenario::find_builtin;
use ecosystem::{EcosystemSim, Rules};
use simulation_core::Simulation;

fn run(seed: u64, ticks: u64, grass: bool, max_sheep: u32) -> (EcosystemSim, Vec<(f64, f64)>) {
    let mut scenario = find_builtin("wolf_sheep_grass").unwrap();
    scenario.seed = seed;
    if let Rules::NetLogo(n) = &mut scenario.config.rules {
        n.grass = grass;
        n.max_sheep = max_sheep;
    }
    let mut sim = scenario.build().unwrap();
    let mut series = Vec::new();
    for _ in 0..ticks {
        sim.step();
        let m = sim.metrics();
        series.push((m.get("population.sheep").unwrap(), m.get("population.wolf").unwrap()));
        if sim.is_finished() {
            break;
        }
    }
    (sim, series)
}

#[test]
fn setup_matches_the_netlogo_defaults() {
    let sim = find_builtin("wolf_sheep_grass").unwrap().build().unwrap();
    assert_eq!(sim.population("sheep"), 100);
    assert_eq!(sim.population("wolf"), 50);
    let green = sim.metrics().get("green_patches").unwrap();
    // Each of the 2601 patches is green with probability 1/2.
    assert!((green - 1300.5).abs() < 4.0 * (2601.0f64 * 0.25).sqrt(), "{green}");
    let sheep = sim.catalog().animal_index("sheep").unwrap();
    for (_, a) in sim.animals().iter() {
        let max = if a.species == sheep { 8.0 } else { 40.0 };
        assert!(a.energy_kj >= 0.0 && a.energy_kj < max && a.energy_kj.fract() == 0.0, "random(2 × gain)");
    }
}

#[test]
fn grass_version_persists_with_fluctuating_populations() {
    // Recorded runs (docs/ECOSYSTEM.md §13.1, `ecosystem_report netlogo`):
    // seeds 1–8 all keep both species for 3,000 ticks, sheep ≈ 160 ± 25 and
    // wolves ≈ 76 ± 14 after the burn-in.
    let mut persisted = 0;
    for seed in 1..=4 {
        let (_, series) = run(seed, 1500, true, 30_000);
        let (sheep, wolves) = *series.last().unwrap();
        if series.len() == 1500 && sheep > 0.0 && wolves > 0.0 {
            persisted += 1;
        }
        let mean = series.iter().map(|s| s.0).sum::<f64>() / series.len() as f64;
        let sd = (series.iter().map(|s| (s.0 - mean).powi(2)).sum::<f64>() / series.len() as f64).sqrt();
        assert!(sd > 10.0, "seed {seed}: sheep numbers fluctuate (sd {sd})");
    }
    assert!(persisted >= 3, "both species persisted in {persisted} of 4 seeds");
}

#[test]
fn no_grass_version_collapses() {
    // Without grass, sheep never starve: the system is neutrally unstable and
    // one species goes extinct (or sheep overrun the map) quickly.
    for seed in 1..=3 {
        let (sim, series) = run(seed, 3000, false, 2000);
        let (sheep, wolves) = *series.last().unwrap();
        assert!(
            sim.is_finished() || sheep == 0.0 || wolves == 0.0,
            "seed {seed} still has {sheep} sheep and {wolves} wolves"
        );
    }
}

#[test]
fn wolves_eat_sheep_and_both_reproduce() {
    let (sim, _) = run(1, 200, true, 30_000);
    let c = sim.counters();
    let sheep = usize::from(sim.catalog().animal_index("sheep").unwrap());
    let wolf = usize::from(sim.catalog().animal_index("wolf").unwrap());
    assert!(c.total_births[sheep] > 0 && c.total_births[wolf] > 0);
    assert!(c.deaths_by_cause[ecosystem::DeathCause::Predation as usize] > 0);
    // Lambs inherit nothing but their lineage: generation 1 for the first ones.
    assert!(sim.animals().iter().any(|(_, a)| a.lineage.generation >= 1));
}
