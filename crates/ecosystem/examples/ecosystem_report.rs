//! Reproduces the population dynamics quoted in `docs/ECOSYSTEM.md`.
//!
//! ```text
//! cargo run --release -p ecosystem --example ecosystem_report -- netlogo [seeds] [ticks]
//! cargo run --release -p ecosystem --example ecosystem_report -- meadow [days] [seed]
//! cargo run --release -p ecosystem --example ecosystem_report -- scenario <id> [days] [every] [control]
//! cargo run --release -p ecosystem --example ecosystem_report -- species
//! ```
//!
//! * `netlogo` runs the grass and no-grass versions of the wolves, sheep and
//!   grass scenario for seeds `1..=seeds` (default 8 and 3000 ticks) and
//!   prints persistence, extinction ticks and post-burn-in statistics.
//! * `meadow` runs the balanced-meadow scenario for `days` simulated days
//!   (default 120, one year) and prints a row every 10 days.
//! * `scenario` runs any built-in scenario (`control` drops its scheduled
//!   events) and prints its species, vegetation, soil moisture, burned area
//!   and balance errors every `every` days, then births and deaths per
//!   species, by cause and by species and cause (from the event log).
//! * `species` prints the numbers derived from the built-in species for the
//!   default calendar (240 ticks/day, 120-day year), 20 m cells and the
//!   default `motion_seconds`.

use std::collections::BTreeMap;
use std::error::Error;
use std::time::Instant;

use ecosystem::scenario::find_builtin;
use ecosystem::units::TimeScale;
use ecosystem::{AnimalTraits, EcosystemSim, PlantLayer, Rules, RuntimeParams, SpeciesCatalog};
use simulation_core::Simulation;

/// Ticks discarded before computing NetLogo statistics.
const BURN_IN: usize = 500;

fn arg<T: std::str::FromStr>(args: &[String], index: usize, default: T) -> Result<T, Box<dyn Error>> {
    match args.get(index) {
        Some(text) => text.parse().map_err(|_| format!("argument {index} (`{text}`) is not a valid number").into()),
        None => Ok(default),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("netlogo") => netlogo(arg(&args, 1, 8)?, arg(&args, 2, 3000)?),
        Some("meadow") => meadow(arg(&args, 1, 120)?, arg(&args, 2, 7)?),
        Some("scenario") => {
            let id = args.get(1).ok_or("usage: ecosystem_report scenario <id> [days] [every] [control]")?;
            let control = args.get(4).is_some_and(|a| a == "control");
            scenario(id, arg(&args, 2, 60)?, arg(&args, 3, 10)?, None, control)
        }
        Some("species") => species(),
        _ => Err(
            "usage: ecosystem_report netlogo [seeds] [ticks] | meadow [days] [seed] | scenario <id> [days] [every] \
                  [control] | species"
                .into(),
        ),
    }
}

fn mean_sd(values: &[f64]) -> (f64, f64) {
    let n = values.len().max(1) as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    (mean, var.sqrt())
}

/// Sheep, wolves and green patches after each tick.
type Series = Vec<[f64; 3]>;

fn netlogo_run(seed: u64, ticks: u64, grass: bool) -> Result<(EcosystemSim, Series), Box<dyn Error>> {
    let mut scenario = find_builtin("wolf_sheep_grass")?;
    scenario.seed = seed;
    if let Rules::NetLogo(n) = &mut scenario.config.rules {
        n.grass = grass;
    }
    let mut sim = scenario.build()?;
    let mut series = Vec::new();
    for _ in 0..ticks {
        sim.step();
        let m = sim.metrics();
        let get = |key: &str| m.get(key).unwrap_or(0.0);
        series.push([get("population.sheep"), get("population.wolf"), get("green_patches")]);
        if sim.is_finished() {
            break;
        }
    }
    Ok((sim, series))
}

fn netlogo(seeds: u64, ticks: u64) -> Result<(), Box<dyn Error>> {
    println!("grass version, {ticks} ticks, statistics after a {BURN_IN}-tick burn-in");
    println!("seed  persisted  sheep mean (sd)   wolves mean (sd)  green mean");
    for seed in 1..=seeds {
        let (_, series) = netlogo_run(seed, ticks, true)?;
        let last = series.last().copied().unwrap_or([0.0; 3]);
        let persisted = series.len() as u64 == ticks && last[0] > 0.0 && last[1] > 0.0;
        let tail = &series[BURN_IN.min(series.len())..];
        let column = |k: usize| tail.iter().map(|row| row[k]).collect::<Vec<_>>();
        let ((s, s_sd), (w, w_sd), (g, _)) = (mean_sd(&column(0)), mean_sd(&column(1)), mean_sd(&column(2)));
        println!("{seed:>4}  {persisted:>9}  {s:>7.1} ({s_sd:>5.1})  {w:>7.1} ({w_sd:>5.1})  {g:>9.1}");
    }
    println!("\nno-grass version (stops when no animal is left or wolves are extinct and sheep exceed 30,000)");
    println!("seed  wolves extinct at  sheep extinct at  final tick");
    for seed in 1..=seeds {
        let (sim, series) = netlogo_run(seed, ticks, false)?;
        let first_zero = |k: usize| series.iter().position(|row| row[k] == 0.0).map(|t| (t + 1).to_string());
        let show = |v: Option<String>| v.unwrap_or_else(|| "-".to_string());
        println!("{seed:>4}  {:>17}  {:>16}  {:>10}", show(first_zero(1)), show(first_zero(0)), sim.tick());
    }
    Ok(())
}

fn meadow(days: u64, seed: u64) -> Result<(), Box<dyn Error>> {
    scenario("balanced_meadow", days, 10, Some(seed), false)
}

/// Runs a built-in scenario (optionally with another seed, or without its
/// scheduled events as a control) and prints a row every `every` days.
fn scenario(id: &str, days: u64, every: u64, seed: Option<u64>, control: bool) -> Result<(), Box<dyn Error>> {
    let mut scenario = find_builtin(id)?;
    if let Some(seed) = seed {
        scenario.seed = seed;
    }
    if control {
        scenario.config.events.clear();
    }
    let mut sim = scenario.build()?;
    let ticks_per_day = u64::from(sim.world().calendar().ticks_per_day());
    let species: Vec<String> = scenario.config.animals.iter().map(|a| a.species.clone()).collect();
    let mut header = String::from(" day season ");
    for s in &species {
        header.push_str(&format!("{:>9}", &s[..s.len().min(9)]));
    }
    header.push_str("  trees producers_t  herbs_t herbiv_kg soil_moist burned  N_err    E_err  ms/tick");
    let mode = if control { " (control: no scheduled events)" } else { "" };
    println!("{id}{mode}, seed {}, {days} days ({ticks_per_day} ticks/day)", scenario.seed);
    println!("{header}");
    let area = sim.world().config().cell_area_m2();
    let mut elapsed = 0.0f64;
    let mut tally: BTreeMap<(String, String), u64> = BTreeMap::new();
    let mut seen = sim.events().total();
    for day in 0..=days {
        if day > 0 {
            let mut busy = 0.0;
            for _ in 0..ticks_per_day {
                let start = Instant::now();
                sim.step();
                busy += start.elapsed().as_secs_f64();
                tally_deaths(&sim, &mut seen, &mut tally);
            }
            elapsed = busy * 1000.0 / ticks_per_day as f64;
        }
        if day % every.max(1) != 0 {
            continue;
        }
        let m = sim.metrics();
        let get = |key: &str| m.get(key).unwrap_or(f64::NAN);
        let herbs: f64 = [PlantLayer::Grass, PlantLayer::Shrubs, PlantLayer::Flowers, PlantLayer::Desert]
            .iter()
            .map(|l| sim.vegetation().total_kg(*l, area))
            .sum();
        let mut row = format!("{day:>4} {:>6} ", format!("{:?}", sim.world().time().season).to_lowercase());
        for s in &species {
            let p = get(&format!("population.{s}"));
            row.push_str(&if p >= 1e6 { format!("{:>8.1}M", p / 1e6) } else { format!("{p:>9.0}") });
        }
        row.push_str(&format!(
            "  {:>5} {:>11.1} {:>8.1} {:>9.0} {:>10.3} {:>6.3} {:>6.0e} {:>8.0e} {:>8.2}",
            get("trees"),
            get("biomass_producers") / 1000.0,
            herbs / 1000.0,
            get("biomass_primary_consumers"),
            get("soil_moisture"),
            get("burned_area"),
            get("nutrient_balance_error"),
            get("energy_balance_error"),
            elapsed,
        ));
        println!("{row}");
    }
    let c = sim.counters();
    let all: Vec<String> = sim.catalog().animals().iter().map(|s| s.id.clone()).collect();
    let pairs = |v: &[u64]| -> Vec<String> {
        all.iter().zip(v).filter(|(_, n)| **n > 0).map(|(s, n)| format!("{s}={n}")).collect()
    };
    println!("\nbirths: {}", pairs(&c.total_births).join(", "));
    println!("deaths: {}", pairs(&c.total_deaths).join(", "));
    let causes: Vec<String> = ecosystem::view::DEATH_CAUSES
        .iter()
        .map(|cause| format!("{}={}", cause.key(), c.deaths_by_cause[*cause as usize]))
        .collect();
    println!("deaths by cause: {}", causes.join(", "));
    let by_species: Vec<String> = tally.iter().map(|((s, cause), n)| format!("{s}/{cause}={n}")).collect();
    println!("deaths by species and cause: {}", by_species.join(", "));
    println!("trees germinated: {}, trees died: {}", c.trees_germinated, c.trees_died);
    Ok(())
}

/// Adds the deaths logged since `seen` (events are `Name #id died (cause)`)
/// to the per-species, per-cause tally.
fn tally_deaths(sim: &EcosystemSim, seen: &mut u64, tally: &mut BTreeMap<(String, String), u64>) {
    let log = sim.events();
    let new = (log.total() - *seen).min(log.len() as u64) as usize;
    *seen = log.total();
    for event in log.iter().rev().take(new).filter(|e| e.kind == "death") {
        let name = event.message.split(" #").next().unwrap_or_default();
        let cause = event.message.rsplit('(').next().unwrap_or_default().trim_end_matches(')');
        let id = sim.catalog().animals().iter().find(|s| s.name == name).map_or(name, |s| s.id.as_str());
        *tally.entry((id.to_string(), cause.to_string())).or_insert(0) += 1;
    }
}

/// `v` with `decimals` decimals, or in scientific notation when that
/// would round it to zero.
fn short(v: f64, decimals: usize) -> String {
    let smallest = 10f64.powi(-(decimals as i32));
    if v == 0.0 || v.abs() >= smallest { format!("{v:.decimals$}") } else { format!("{v:.1e}") }
}

fn species() -> Result<(), Box<dyn Error>> {
    let catalog = SpeciesCatalog::builtin()?;
    let scale = TimeScale {
        ticks_per_day: 240,
        days_per_year: 120,
        cell_size_m: 20.0,
        motion_seconds: RuntimeParams::default().motion_seconds,
    };
    println!("20 m cells, 240 ticks/day, 120-day year, motion_seconds = {}", scale.motion_seconds);
    println!(
        "species  mass_kg  BMR_kJ/d  field_kJ/d  reserve_kJ  sprint_m/s  sprint_cells/tick  vision_cells  lifespan_d  maturity_d  gestation_d"
    );
    for s in catalog.animals() {
        let t = AnimalTraits::derive(s, &scale);
        let days = |ticks: f64| ticks / f64::from(scale.ticks_per_day);
        println!(
            "{:<8} {:>7} {:>9} {:>11} {:>11} {:>11.1} {:>18.2} {:>13.1} {:>11.0} {:>11.1} {:>12.1}",
            s.id,
            short(f64::from(t.adult_mass_kg), 2),
            short(f64::from(t.bmr_kj_per_day), 1),
            short(f64::from(t.field_kj_per_day), 1),
            short(f64::from(t.reserve_max_kj), 0),
            t.sprint_m_s,
            t.sprint_cells,
            t.vision_cells,
            days(t.lifespan_ticks),
            days(t.maturity_ticks),
            days(t.gestation_ticks),
        );
    }
    Ok(())
}
