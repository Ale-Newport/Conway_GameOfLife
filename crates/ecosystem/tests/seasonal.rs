//! Seasonal behaviour: migration towards the preferred climate and
//! hibernation in the cold season.

mod common;

use common::{build_with, content, edit_species, grassland, ids_of, pond, position, spawn, traits_of};
use ecosystem::{Action, EcosystemSim};
use organism_engine::AgentId;
use simulation_core::{ParamValue, Simulation};

/// Ducks released at the cold (top) edge of a tall map whose temperature
/// rises by 40 °C from top to bottom; returns their mean row after a day.
fn ducks_after_a_day(migrates: bool) -> (f32, f32) {
    let mut config = grassland(24);
    config.world.height = 120;
    config.world.base_temperature_c = 5.0;
    config.world.latitude_gradient = 40.0;
    config.world.seasonality = 0.0;
    let mut sim = build_with(config, 4, |c| edit_species(c, "duck", |s| s.seasonal.migrates = migrates));
    sim.advance(60); // morning
    let ducks = spawn(&mut sim, "duck", 12.0, 8.0, 12);
    for &id in &ducks {
        content(&mut sim, id);
    }
    let start = ducks.iter().map(|&id| position(&sim, id).y).sum::<f32>() / ducks.len() as f32;
    for _ in 0..12 {
        sim.advance(20);
        for &id in &ducks {
            if let Some(a) = sim.animal_mut(id) {
                a.hydration = 1.0; // no water on this map
            }
        }
    }
    let alive: Vec<u64> = ducks.into_iter().filter(|id| sim.animals().contains(AgentId(*id))).collect();
    let end = alive.iter().map(|&id| position(&sim, id).y).sum::<f32>() / alive.len().max(1) as f32;
    (start, end)
}

#[test]
fn migrants_travel_towards_their_preferred_climate() {
    let (start, migrated) = ducks_after_a_day(true);
    let (_, control) = ducks_after_a_day(false);
    eprintln!("mean row of the ducks: start {start:.1}, migrants {migrated:.1}, residents {control:.1}");
    assert!(migrated > start + 20.0, "migrants fly towards the warm side: {start:.1} → {migrated:.1}");
    assert!(migrated > control + 15.0, "residents stay closer: {control:.1}");
}

/// A fed bear by a pond on a cold plain whose seasons last 2 days; returns
/// the run, the bear and its energy use on day 8 (winter) in days of field
/// metabolism.
fn bear_winter(hibernates: bool) -> (EcosystemSim, u64, f32) {
    let mut config = pond(24, 5.0, 20.0);
    config.world.base_temperature_c = -8.0;
    config.world.seasonality = 4.0;
    config.world.days_per_season = 2;
    let mut sim = build_with(config, 6, |c| {
        edit_species(c, "bear", |s| {
            if !hibernates {
                s.seasonal.hibernation = None;
            }
        })
    });
    let bear = spawn(&mut sim, "bear", 4.0, 12.0, 1)[0];
    content(&mut sim, bear);
    sim.advance(7 * 240); // spring, summer, autumn and the first winter day
    let traits = traits_of(&sim, "bear");
    let before = sim.animals().get(AgentId(bear)).map_or(0.0, |b| b.energy_kj);
    sim.advance(240);
    let after = sim.animals().get(AgentId(bear)).map_or(0.0, |b| b.energy_kj);
    let used = (before - after) / traits.field_kj_per_day;
    (sim, bear, used)
}

#[test]
fn bears_hibernate_in_the_cold_season_and_save_energy() {
    let (sim, bear, dormant_use) = bear_winter(true);
    let b = sim.animals().get(AgentId(bear)).expect("the bear survives the winter");
    assert_eq!(b.action, Action::Hibernate, "the bear is dormant in winter");
    assert!(b.hidden, "a hibernating bear is hidden in its den");
    let (_, _, active_use) = bear_winter(false);
    eprintln!("energy used on a winter day (field-metabolism days): dormant {dormant_use:.3}, active {active_use:.3}");
    assert!(dormant_use < 0.4 * active_use, "dormancy saves energy: {dormant_use:.3} vs {active_use:.3}");
    assert!(dormant_use >= 0.0);
}

#[test]
fn hibernators_wake_when_it_gets_warm() {
    let (mut sim, bear, _) = bear_winter(true);
    assert_eq!(sim.animals().get(AgentId(bear)).unwrap().action, Action::Hibernate);
    sim.set_param("base_temperature_c", ParamValue::Float(22.0)).unwrap();
    let mut woke = false;
    for _ in 0..3 * 240 {
        sim.step();
        if sim.animals().get(AgentId(bear)).is_some_and(|b| b.action != Action::Hibernate) {
            woke = true;
            break;
        }
    }
    assert!(woke, "the bear leaves its den once the daily mean exceeds its waking temperature");
    assert!(!ids_of(&sim, "bear").is_empty());
}

#[test]
fn torpid_ectotherms_brumate_in_a_cold_spring() {
    // Early spring on a cold plain (daily mean far below the frogs' 5 °C):
    // instead of sitting torpid in the frost, frogs with enough reserves go
    // into dormancy, which is now possible outside autumn and winter.
    let mut config = pond(24, 4.0, 20.0);
    config.world.base_temperature_c = -2.0;
    config.world.seasonality = 20.0;
    let mut sim = build_with(config, 12, |_| {});
    let frogs = spawn(&mut sim, "frog", 3.0, 12.0, 6);
    for &id in &frogs {
        content(&mut sim, id);
    }
    sim.advance(2 * 240);
    let dormant = frogs
        .iter()
        .filter(|&&id| sim.animals().get(AgentId(id)).is_some_and(|a| a.action == Action::Hibernate && a.hidden))
        .count();
    assert_eq!(dormant, frogs.len(), "every frog brumates in its shelter");
    assert_eq!(sim.counters().deaths_by_cause[ecosystem::DeathCause::Exposure as usize], 0);
}

#[test]
fn hibernators_rouse_to_drink_when_their_water_runs_low() {
    let (mut sim, bear, _) = bear_winter(true);
    assert_eq!(sim.animals().get(AgentId(bear)).unwrap().action, Action::Hibernate);
    sim.animal_mut(bear).unwrap().hydration = 0.2;
    let mut roused = false;
    let mut drank = false;
    for _ in 0..240 {
        sim.step();
        let Some(b) = sim.animals().get(AgentId(bear)) else { break };
        roused |= b.action != Action::Hibernate;
        drank |= b.hydration > 0.3;
        if drank {
            break;
        }
    }
    assert!(roused, "a dehydrated bear leaves its torpor (periodic arousal)");
    assert!(drank, "and drinks at the pond");
}
