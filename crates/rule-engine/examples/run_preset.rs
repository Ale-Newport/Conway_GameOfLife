//! Runs a built-in custom-life preset headless and prints population and
//! resource time series as CSV.
//!
//! ```text
//! cargo run -p rule-engine --release --example run_preset -- <preset-id | file.ron> [ticks=600] [every=20] [seed]
//! ```

use rule_engine::{CustomLifeSim, scenarios};
use simulation_core::Simulation;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(id) = args.get(1) else {
        eprintln!("usage: run_preset <preset-id> [ticks] [every] [seed]");
        match scenarios::builtin() {
            Ok(all) => all.iter().for_each(|s| eprintln!("  {} — {}", s.id, s.name)),
            Err(e) => eprintln!("{e}"),
        }
        std::process::exit(2);
    };
    let number = |i: usize, default: u64| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(default);
    let (ticks, every) = (number(2, 600), number(3, 20).max(1));
    let loaded = if id.ends_with(".ron") {
        scenarios::load_file(std::path::Path::new(id))
    } else {
        scenarios::builtin_by_id(id)
    };
    let mut preset = match loaded {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    preset.seed = number(4, preset.seed);
    let mut sim = match CustomLifeSim::from_scenario(&preset) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    sim.apply(rule_engine::CustomLifeCommand::SetPauseOnEnd(false)).unwrap_or_else(|e| eprintln!("{e}"));
    let keys: Vec<String> = sim
        .metrics()
        .keys()
        .filter(|k| k.starts_with("population") || k.starts_with("resource.") || k.starts_with("energy."))
        .map(str::to_string)
        .collect();
    println!("tick,{}", keys.join(","));
    for t in 0..=ticks {
        if t % every == 0 {
            let m = sim.metrics();
            let row: Vec<String> = keys.iter().map(|k| format!("{:.1}", m.get(k).unwrap_or(0.0))).collect();
            println!("{},{}", sim.tick(), row.join(","));
        }
        if t < ticks {
            sim.step();
        }
    }
    for e in sim.script_errors() {
        eprintln!("{e}");
    }
}
