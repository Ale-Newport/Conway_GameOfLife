//! The built-in scenarios: every one loads and runs without violating the
//! model's invariants; the experiments produce their qualitative outcomes
//! (long runs, `#[ignore]`d: `cargo test --profile verify -p ecosystem --test
//! scenarios -- --ignored`).

mod common;

use ecosystem::scenario::{self, Scenario, find_builtin};
use ecosystem::species::Representation;
use ecosystem::{EcosystemSim, EventAction, PlantLayer};
use simulation_core::{EventSeverity, Simulation};

/// Asserts the invariants of a running ecosystem.
fn check_invariants(sim: &EcosystemSim, id: &str) {
    let m = sim.metrics();
    for (key, value) in &m.values {
        assert!(value.is_finite(), "{id}: metric {key} = {value}");
        if key.starts_with("population.") || key.starts_with("biomass") || key.starts_with("nutrients.") {
            assert!(*value >= 0.0, "{id}: {key} = {value}");
        }
    }
    if let Some(e) = m.get("energy_balance_error") {
        assert!(e < 1e-4, "{id}: energy balance error {e:e}");
    }
    if let Some(e) = m.get("nutrient_balance_error") {
        assert!(e < 1e-3, "{id}: nutrient balance error {e:e}");
    }
    let (w, h) = (sim.world().width() as f32, sim.world().height() as f32);
    for (id_a, a) in sim.animals().iter() {
        let p = a.kin.position;
        assert!(p.x >= 0.0 && p.y >= 0.0 && p.x <= w && p.y <= h, "{id}: animal {} outside the map at {p}", id_a.get());
        assert!((0.0..=1.0).contains(&a.health) && (0.0..=1.0).contains(&a.hydration), "{id}: condition out of range");
        assert!(
            a.energy_kj >= 0.0 && a.mass_kg > 0.0 && a.nutrient_store_g >= 0.0 && a.altitude_m.is_finite(),
            "{id}: body of animal {} out of range: {} kJ, {} kg, {} g, {} m",
            id_a.get(),
            a.energy_kj,
            a.mass_kg,
            a.nutrient_store_g,
            a.altitude_m
        );
    }
    for c in &sim.colonies().colonies {
        assert!(c.workers >= 0.0 && c.store_kj >= -1e-6, "{id}: colony {} has negative pools", c.id);
    }
    let errors: Vec<_> = sim.events().iter().filter(|e| e.kind == "error").map(|e| e.message.clone()).collect();
    assert!(errors.is_empty(), "{id}: internal errors {errors:?}");
}

#[test]
fn there_are_fourteen_scenarios_covering_the_experiments() {
    let all = scenario::builtin().unwrap();
    let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(all.len(), 14, "{ids:?}");
    for id in [
        "balanced_meadow",
        "wolf_sheep_grass",
        "temperate_forest",
        "african_savanna",
        "freshwater_lake",
        "ocean_ecosystem",
        "tropical_rainforest",
        "isolated_island",
        "forest_fire",
        "prolonged_drought",
        "invasive_species",
        "apex_predator_extinction",
        "island_evolution",
        "extreme_seasons",
    ] {
        assert!(ids.contains(&id), "{id} is a built-in scenario");
    }
}

#[test]
fn every_scenario_runs_a_day_without_violating_invariants() {
    for s in scenario::builtin().unwrap() {
        let mut sim = s.build().unwrap_or_else(|e| panic!("{}: {e}", s.id));
        check_invariants(&sim, &s.id);
        for _ in 0..4 {
            sim.advance(60);
            check_invariants(&sim, &s.id);
        }
        // Every species of the scenario is present at the start of the run
        // (individuals, fields or colonies).
        let catalog = sim.catalog();
        for setup in &s.config.animals {
            let species = catalog.animal(&setup.species).unwrap();
            let present = match species.representation {
                Representation::Individual => sim.population(&setup.species) > 0 || setup.count < 3,
                _ => sim.metrics().get(&format!("population.{}", setup.species)).unwrap() > 0.0,
            };
            assert!(present, "{}: {} present after a day", s.id, setup.species);
        }
    }
}

#[test]
fn scheduled_events_run_on_their_day() {
    let s = find_builtin("apex_predator_extinction").unwrap();
    let day =
        s.config.events.iter().find_map(|e| matches!(e.action, EventAction::Remove { .. }).then_some(e.day)).unwrap();
    let mut sim = s.build().unwrap();
    let tick = (f64::from(day) * 240.0).round() as u64;
    sim.advance(tick);
    assert!(sim.population("wolf") > 0, "wolves live until the event");
    sim.step();
    assert_eq!(sim.population("wolf"), 0, "the removal runs at the start of its tick");
    assert!(sim.events().iter().any(|e| e.kind == "scheduled" && e.severity == EventSeverity::Major));
    // A reset rebuilds the scenario with its schedule.
    sim.reset();
    assert!(sim.population("wolf") > 0);
}

/// The scenario without its scheduled events (the control of an experiment).
fn control(id: &str) -> Scenario {
    let mut s = find_builtin(id).unwrap();
    s.config.events.clear();
    s
}

/// Runs `days` days and returns a metric sampled at the end of every day.
fn daily(sim: &mut EcosystemSim, days: u64, key: &str) -> Vec<f64> {
    (0..days)
        .map(|_| {
            sim.advance(240);
            sim.metrics().get(key).unwrap_or(0.0)
        })
        .collect()
}

fn herbivores(sim: &EcosystemSim, ids: &[&str]) -> usize {
    ids.iter().map(|id| sim.population(id)).sum()
}

/// Body mass (kg) of the living animals of the given species.
fn biomass_kg(sim: &EcosystemSim, ids: &[&str]) -> f64 {
    let species: Vec<u16> = ids.iter().filter_map(|id| sim.catalog().animal_index(id).ok()).collect();
    sim.animals().iter().filter(|(_, a)| species.contains(&a.species)).map(|(_, a)| f64::from(a.mass_kg)).sum()
}

/// Biomass of the herb and shrub layers (kg): what grazers and browsers eat.
fn herbs(sim: &EcosystemSim) -> f64 {
    let area = sim.world().config().cell_area_m2();
    [PlantLayer::Grass, PlantLayer::Shrubs, PlantLayer::Flowers]
        .iter()
        .map(|l| sim.vegetation().total_kg(*l, area))
        .sum()
}

#[test]
#[ignore = "long run (≈ 90 simulated days); run with --profile verify -- --ignored"]
fn forest_fire_burns_then_vegetation_recovers() {
    let mut sim = find_builtin("forest_fire").unwrap().build().unwrap();
    sim.advance(22 * 240);
    let pre_fire = herbs(&sim);
    let mut after = Vec::new();
    for _ in 0..70 {
        sim.advance(240);
        after.push(herbs(&sim));
    }
    let burned = sim.world().burned_area_cells();
    let combusted = sim.ledgers().energy.total(ecosystem::Flux::Combustion);
    let (low_day, low) =
        after.iter().copied().enumerate().fold((0, f64::MAX), |b, (d, v)| if v < b.1 { (d, v) } else { b });
    let end = *after.last().unwrap();
    eprintln!(
        "forest fire: herbs and shrubs {pre_fire:.0} kg before, minimum {low:.0} kg {} days after ignition, {end:.0} kg 70 days after; {burned} cells burned, {combusted:.3e} kJ combusted",
        low_day + 1
    );
    check_invariants(&sim, "forest_fire");
    assert!(combusted > 0.0 && burned > 100, "the fire spreads ({burned} cells)");
    assert!(low < 0.5 * pre_fire, "the fire removes most of the herb layer: {low:.0} of {pre_fire:.0} kg");
    assert!(end > 1.5 * low, "the vegetation regrows from the seed bank: {low:.0} → {end:.0} kg");
}

#[test]
#[ignore = "long run (≈ 100 simulated days, two runs); run with --profile verify -- --ignored"]
fn drought_reduces_plants_and_herbivores() {
    let herbivore_ids = ["sheep", "horse", "deer", "rabbit"];
    let mut dry = find_builtin("prolonged_drought").unwrap().build().unwrap();
    let mut wet = control("prolonged_drought").build().unwrap();
    let dry_plants = daily(&mut dry, 100, "biomass_producers");
    let wet_plants = daily(&mut wet, 100, "biomass_producers");
    let counts = |s: &EcosystemSim| herbivore_ids.map(|id| s.population(id));
    let starved = |s: &EcosystemSim| s.counters().deaths_by_cause[ecosystem::DeathCause::Starvation as usize];
    let (d, w) = (herbivores(&dry, &herbivore_ids), herbivores(&wet, &herbivore_ids));
    eprintln!(
        "drought, day 100: producers {:.0} vs {:.0} kg, herbivores {d} vs {w} ({:?} vs {:?} sheep, horses, deer, rabbits; {:.0} vs {:.0} kg), {} vs {} starved, soil moisture {:.3} vs {:.3}",
        dry_plants[99],
        wet_plants[99],
        counts(&dry),
        counts(&wet),
        biomass_kg(&dry, &herbivore_ids),
        biomass_kg(&wet, &herbivore_ids),
        starved(&dry),
        starved(&wet),
        dry.metrics().get("soil_moisture").unwrap(),
        wet.metrics().get("soil_moisture").unwrap()
    );
    check_invariants(&dry, "prolonged_drought");
    assert!(dry_plants[99] < 0.9 * wet_plants[99], "the drought reduces plant biomass");
    assert!(starved(&dry) > starved(&wet), "herbivores starve in the drought");
    assert!((d as f64) < 0.9 * w as f64, "and decline: {d} vs {w}");
}

#[test]
#[ignore = "long run (≈ 120 simulated days, two runs); run with --profile verify -- --ignored"]
fn removing_the_apex_predator_releases_herbivores_then_vegetation_declines() {
    let ids = ["deer", "stag", "sheep", "rabbit"];
    let mut removed = find_builtin("apex_predator_extinction").unwrap().build().unwrap();
    let mut kept = control("apex_predator_extinction").build().unwrap();
    let mut rows = Vec::new();
    for day in (10..=120).step_by(10) {
        removed.advance(10 * 240);
        kept.advance(10 * 240);
        rows.push((day, herbivores(&removed, &ids), herbivores(&kept, &ids), herbs(&removed), herbs(&kept)));
    }
    for r in &rows {
        eprintln!(
            "day {:3}: herbivores {} (no wolves) vs {} (wolves); herbs {:.0} vs {:.0} kg",
            r.0, r.1, r.2, r.3, r.4
        );
    }
    let grazed = |s: &EcosystemSim| s.ledgers().energy.total(ecosystem::Flux::Herbivory);
    let (g_removed, g_kept) = (grazed(&removed), grazed(&kept));
    eprintln!("herbivory over 120 days: {g_removed:.4e} kJ (no wolves) vs {g_kept:.4e} kJ (wolves)");
    check_invariants(&removed, "apex_predator_extinction");
    let (_, h_removed, h_kept, _, _) = rows[5]; // day 60
    assert!(h_removed > h_kept, "without wolves more herbivores survive: {h_removed} vs {h_kept}");
    assert!(g_removed > g_kept, "more herbivores graze more: {g_removed:.3e} vs {g_kept:.3e} kJ");
    let (_, _, _, v_removed, v_kept) = *rows.last().unwrap();
    assert!(v_removed < v_kept, "more herbivores eat more vegetation: {v_removed:.0} vs {v_kept:.0} kg");
}

#[test]
#[ignore = "long run (≈ 180 simulated days, two runs); run with --profile verify -- --ignored"]
fn an_invasive_species_changes_the_native_populations() {
    let mut invaded = find_builtin("invasive_species").unwrap().build().unwrap();
    let mut pristine = control("invasive_species").build().unwrap();
    invaded.advance(180 * 240);
    pristine.advance(180 * 240);
    let natives = |s: &EcosystemSim| (s.population("lizard"), s.population("frog"), s.population("songbird"));
    let (a, b) = (natives(&invaded), natives(&pristine));
    eprintln!(
        "invasion: rats {}; lizards, frogs, songbirds {a:?} with rats vs {b:?} without",
        invaded.population("rat")
    );
    check_invariants(&invaded, "invasive_species");
    assert!(invaded.population("rat") > 24, "the rats establish and multiply");
    assert!(a.0 < b.0, "the native lizards decline: {} vs {}", a.0, b.0);
}

#[test]
#[ignore = "long run (one simulated year); run with --profile verify -- --ignored"]
fn the_balanced_meadow_keeps_its_vertebrates_for_a_year() {
    let s = find_builtin("balanced_meadow").unwrap();
    let mut sim = s.build().unwrap();
    for _ in 0..12 {
        sim.advance(10 * 240);
        check_invariants(&sim, "balanced_meadow");
    }
    let alive: Vec<&str> = ["rabbit", "deer", "fox", "wolf", "raven", "songbird"]
        .into_iter()
        .filter(|id| sim.population(id) > 0)
        .collect();
    eprintln!("balanced meadow after a year: {alive:?} alive");
    assert!(alive.len() >= 3, "at least three vertebrate species persist: {alive:?}");
}

#[test]
#[ignore = "long run (13 scenarios × 30 simulated days); run with --profile verify -- --ignored"]
fn no_scenario_loses_a_founding_species_within_ten_days() {
    // Every species a scenario starts with (four or more individuals, or a
    // density field or colonies) is still present ten days in, unless the
    // scenario removes it on purpose; the run keeps its invariants for a
    // month. Catches species that cannot feed, drink, keep warm or breed in
    // the world they were put in.
    for s in scenario::builtin().unwrap() {
        if matches!(s.config.rules, ecosystem::Rules::NetLogo(_)) {
            continue; // the NetLogo rule set has its own tests
        }
        let removed: Vec<&str> = s
            .config
            .events
            .iter()
            .filter_map(|e| match &e.action {
                EventAction::Remove { species, .. } => Some(species.as_str()),
                _ => None,
            })
            .collect();
        let mut sim = s.build().unwrap();
        let catalog = sim.catalog().clone();
        let tracked: Vec<&str> = s
            .config
            .animals
            .iter()
            .filter(|a| {
                let individual = catalog.animal(&a.species).is_some_and(|sp| sp.is_individual());
                !removed.contains(&a.species.as_str()) && (!individual || a.count >= 4)
            })
            .map(|a| a.species.as_str())
            .collect();
        let alive = |sim: &EcosystemSim, id: &str| sim.metrics().get(&format!("population.{id}")).unwrap_or(0.0);
        for day in 1..=30 {
            sim.advance(240);
            if day % 5 == 0 {
                check_invariants(&sim, &s.id);
            }
            if day == 10 {
                let lost: Vec<&str> = tracked.iter().copied().filter(|id| alive(&sim, id) <= 0.0).collect();
                assert!(lost.is_empty(), "{}: {lost:?} died out within ten days", s.id);
            }
        }
        let summary: Vec<String> = tracked.iter().map(|id| format!("{id}={}", alive(&sim, id))).collect();
        eprintln!("{} after 30 days: {}", s.id, summary.join(", "));
    }
}
