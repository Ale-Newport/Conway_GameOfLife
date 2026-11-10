//! Helpers shared by the integration tests.
#![allow(dead_code)]

use rule_engine::{CustomLifeSim, LifeSystem, Scenario, scenarios};
use simulation_core::Simulation;

/// A built-in preset by id.
pub fn preset(id: &str) -> Scenario {
    scenarios::builtin_by_id(id).unwrap_or_else(|e| panic!("{e}"))
}

/// Simulation of a preset (end conditions do not stop `step`).
pub fn sim_of(scenario: &Scenario) -> CustomLifeSim {
    CustomLifeSim::from_scenario(scenario).unwrap_or_else(|e| panic!("{e}"))
}

/// Runs `ticks` ticks and samples `keys` every `every` ticks (including tick 0).
pub fn series(sim: &mut CustomLifeSim, ticks: u64, every: u64, keys: &[&str]) -> Vec<Vec<f64>> {
    let mut out = vec![Vec::new(); keys.len()];
    for t in 0..=ticks {
        if t % every == 0 {
            let m = sim.metrics();
            for (k, key) in keys.iter().enumerate() {
                out[k].push(m.get(key).unwrap_or_else(|| panic!("missing metric {key}")));
            }
        }
        if t < ticks {
            sim.step();
        }
    }
    out
}

/// Mean of a slice.
pub fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// Parses a system from RON text.
pub fn system(text: &str) -> LifeSystem {
    LifeSystem::from_ron(text).unwrap_or_else(|e| panic!("{e}"))
}
