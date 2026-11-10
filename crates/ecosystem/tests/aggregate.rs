//! Aggregate representations: density fields of tiny organisms, colonies of
//! eusocial insects with forager squads and pheromone trails, pollination,
//! and the conservation of energy and nutrients through them.

mod common;

use common::{build, build_with, edit_species, grassland, with_params};
use ecosystem::species::Representation;
use ecosystem::{AnimalSetup, EcosystemCommand, EcosystemSim, EcosystemViewRequest, Flux};
use simulation_core::glam::Vec2;
use simulation_core::{ParamValue, Simulation};

fn population(sim: &EcosystemSim, species: &str) -> f64 {
    sim.metrics().get(&format!("population.{species}")).unwrap()
}

fn setup(species: &str, count: u32, region: Option<(f32, f32, f32, f32)>) -> AnimalSetup {
    AnimalSetup { species: species.into(), count, region, groups: None }
}

#[test]
fn fields_close_the_energy_and_nutrient_budgets() {
    let mut config = with_params(grassland(24), &[("external_fluxes", ParamValue::Bool(false))]);
    config.world.base_temperature_c = 18.0;
    config.world.seasonality = 0.0;
    config.animals = vec![
        setup("worm", 3_000_000, None),
        setup("beetle", 2_000_000, None),
        setup("butterfly", 300_000, None),
        setup("spider", 500_000, None),
        setup("wasp", 50_000, None),
        setup("lizard", 20, None),
    ];
    let mut sim = build(config, 2);
    let n0 = sim.stocks().nutrients();
    sim.advance(3 * 240);
    let m = sim.metrics();
    let n_err = (sim.stocks().nutrients() - n0).abs() / n0;
    assert!(n_err < 1e-5, "closed budget: nutrient stock constant ({n_err:e})");
    assert!(m.get("energy_balance_error").unwrap() < 1e-6, "{:?}", m.get("energy_balance_error"));
    assert!(m.get("nutrient_balance_error").unwrap() < 1e-5);
    assert!(sim.ledgers().energy.total(Flux::Detritivory) > 0.0, "worms and beetles eat litter");
    assert!(sim.ledgers().energy.total(Flux::Nectarivory) > 0.0, "butterflies drink nectar");
    assert!(sim.ledgers().energy.total(Flux::Predation) > 0.0, "spiders, wasps and lizards eat insects");
    let (kj, g) = sim.stocks().fields;
    assert!(kj > 0.0 && g > 0.0);
}

#[test]
fn field_populations_follow_their_food() {
    // Beetles on plenty of litter grow; on bare ground they starve.
    let run = |litter: f32| {
        let mut config = grassland(16);
        config.world.base_temperature_c = 20.0;
        config.world.seasonality = 0.0;
        config.soil.litter_kg_m2 = litter;
        config.vegetation.initial_fraction = if litter > 0.0 { 0.6 } else { 0.0 };
        config.vegetation.seed_bank = if litter > 0.0 { 1.0 } else { 0.0 };
        config.animals = vec![setup("beetle", 1_000_000, None)];
        let mut sim = build(with_params(config, &[("plant_growth", ParamValue::Float(0.0))]), 3);
        let start = population(&sim, "beetle");
        sim.advance(5 * 240);
        (start, population(&sim, "beetle"))
    };
    let (start, fed) = run(1.0);
    let (_, starved) = run(0.0);
    eprintln!("beetles: start {start:.0}, with litter {fed:.0}, without food {starved:.0}");
    // Litter is poor in nutrients: growth on it is nutrient-limited and slow.
    assert!(fed > 1.02 * start, "beetles multiply on litter");
    assert!(starved < 0.9 * start, "beetles starve without food");
    assert!(fed > 1.2 * starved);
}

#[test]
fn fields_disperse_into_empty_habitat() {
    let mut config = grassland(30);
    config.world.base_temperature_c = 20.0;
    config.animals = vec![setup("butterfly", 200_000, Some((0.0, 0.0, 5.0, 5.0)))];
    let mut sim = build(config, 4);
    let slot = sim.fields().slot(sim.catalog().animal_index("butterfly").unwrap()).unwrap();
    let far = |sim: &EcosystemSim| {
        (10..30).flat_map(|y| (10..30).map(move |x| y * 30 + x)).map(|c| sim.fields().density(slot, c)).sum::<f32>()
    };
    assert_eq!(far(&sim), 0.0);
    sim.advance(10 * 240);
    assert!(far(&sim) > 0.0, "butterflies reach cells far from where they were released");
    // Representative markers stand for them in views.
    let view = sim.view(&EcosystemViewRequest::default());
    assert!(!view.field_markers.is_empty());
    assert!(view.field_markers.iter().all(|m| m.altitude_m > 0.0), "butterflies are drawn flying");
    let info = view.species.iter().find(|s| s.id == "butterfly").unwrap();
    assert_eq!((info.representation.as_str(), info.visual_id.as_str()), ("field", "proc:butterfly"));
}

#[test]
fn insectivores_eat_field_prey() {
    let run = |lizards: u32| {
        let mut config = grassland(16);
        config.world.base_temperature_c = 24.0;
        config.world.seasonality = 0.0;
        config.animals = vec![setup("beetle", 800_000, None), setup("spider", 300_000, None)];
        if lizards > 0 {
            config.animals.push(setup("lizard", lizards, None));
        }
        let mut sim = build(config, 5);
        let lizard = sim.catalog().animal_index("lizard").unwrap();
        let ids: Vec<u64> = sim.animals().iter().filter(|(_, a)| a.species == lizard).map(|(id, _)| id.get()).collect();
        for id in &ids {
            if let Some(a) = sim.animal_mut(*id) {
                a.energy_kj *= 0.3;
                a.hydration = 1.0;
            }
        }
        sim.advance(2 * 240);
        let eaten: f64 = sim.animals().iter().filter(|(_, a)| a.species == lizard).map(|(_, a)| a.stats.eaten_kj).sum();
        (sim.ledgers().energy.total(Flux::Predation), eaten)
    };
    let (with, eaten) = run(40);
    let (without, _) = run(0);
    assert!(eaten > 0.0, "lizards ate beetles and spiders");
    assert!(with > without, "predation on the fields grows with the lizards: {with:.0} vs {without:.0} kJ");
}

/// An ant colony on 1 m cells with a carcass 8 cells east of the nest
/// (bare ground: the carcass is the only food; ten squads of 50 workers);
/// returns the run after `ticks` and the nest position.
fn ants(ticks: u64) -> (EcosystemSim, Vec2) {
    let mut config = grassland(48);
    config.world.cell_size_m = 1.0;
    config.world.base_temperature_c = 22.0;
    config.world.seasonality = 0.0;
    config.vegetation.initial_fraction = 0.0;
    config.vegetation.seed_bank = 0.0;
    config.soil.litter_kg_m2 = 0.0;
    config.animals = vec![setup("ant", 1, Some((10.0, 23.0, 11.0, 24.0)))];
    let mut sim = build_with(config, 6, |c| {
        edit_species(c, "ant", |s| {
            if let Representation::Colony(p) = &mut s.representation {
                p.forager_fraction = 0.5;
                p.workers_per_forager = 50.0;
            }
        })
    });
    let nest = sim.colonies().colonies[0].nest;
    let rabbit = common::spawn(&mut sim, "rabbit", nest.x + 8.0, nest.y, 1)[0];
    sim.command(EcosystemCommand::KillAnimal { id: rabbit }).unwrap();
    sim.advance(ticks);
    (sim, nest)
}

#[test]
fn ant_foragers_lay_a_trail_between_nest_and_food() {
    let (sim, nest) = ants(480);
    let colony = &sim.colonies().colonies[0];
    assert!(colony.store_kj > 0.0);
    assert!(sim.ledgers().energy.total(Flux::Scavenging) > 0.0, "ants carried meat home");
    let w = sim.world().width();
    let p = &sim.colonies().pheromone;
    let cell = |x: f32, y: f32| (y as usize) * w + x as usize;
    // Mean pheromone on the segment nest → carcass against cells as far
    // from the nest in the opposite direction.
    let on: f32 = (2..7).map(|k| p[cell(nest.x + k as f32, nest.y)]).sum::<f32>() / 5.0;
    let off: f32 = (2..7).map(|k| p[cell(nest.x, nest.y + k as f32)]).sum::<f32>() / 5.0;
    eprintln!("pheromone on the trail {on:.3}, off the trail {off:.3}");
    assert!(on > 3.0 * off.max(1e-6), "the trail is marked: {on:.3} vs {off:.3}");
}

#[test]
fn pheromone_evaporates_with_its_half_life() {
    let (mut sim, _) = ants(240);
    sim.set_param("pheromone_half_life_h", ParamValue::Float(1.0)).unwrap();
    sim.set_param("pheromone_diffusion", ParamValue::Float(0.0)).unwrap();
    sim.step();
    // Remove the foragers' source of new pheromone by night-time cold.
    sim.set_param("base_temperature_c", ParamValue::Float(-20.0)).unwrap();
    sim.advance(30);
    let before = sim.metrics().get("pheromone").unwrap();
    sim.advance(10); // one hour
    let after = sim.metrics().get("pheromone").unwrap();
    assert!(before > 0.0);
    let ratio = after / before;
    assert!((0.45..0.56).contains(&ratio), "half of the pheromone evaporates in one half-life: {ratio:.3}");
}

/// Flower seeds produced over `days` with and without two bee hives on a
/// 20 × 20 meadow (16 ha).
fn flower_seeds(bees: bool, days: u64) -> (f64, f64) {
    let mut config = grassland(20);
    config.world.base_temperature_c = 20.0;
    config.world.seasonality = 0.0;
    config.world.rainfall_mm = 900.0;
    if bees {
        config.animals = vec![setup("bee", 2, Some((8.0, 8.0, 12.0, 12.0)))];
    }
    let mut sim = build(config, 8);
    sim.advance(days * 240);
    (sim.vegetation().flower_seeds_total(), sim.vegetation().flower_pollination())
}

#[test]
fn pollinators_raise_flower_seed_production() {
    let (with_bees, pollination) = flower_seeds(true, 4);
    let (without, base) = flower_seeds(false, 4);
    eprintln!(
        "flower seeds over 4 days: with bees {with_bees:.3e}, without {without:.3e}; pollination {pollination:.3} vs {base:.3}"
    );
    assert_eq!(base, 0.0, "nothing pollinates without pollinators");
    assert!(pollination > 0.05, "bees pollinate the meadow: {pollination:.3}");
    assert!(with_bees > 1.05 * without, "pollination raises seed production");
}

#[test]
fn pollination_index_raises_seed_set_linearly() {
    // Unit-level check through two worlds that differ only by the visits
    // of butterflies (nectar feeders with 20 visits per hour).
    let run = |butterflies: u32| {
        let mut config = grassland(16);
        config.world.base_temperature_c = 20.0;
        config.world.seasonality = 0.0;
        if butterflies > 0 {
            config.animals = vec![setup("butterfly", butterflies, None)];
        }
        let mut sim = build(config, 9);
        sim.advance(2 * 240 + 1);
        (sim.vegetation().flower_seed_production(), sim.vegetation().flower_pollination())
    };
    let (seeds_0, p0) = run(0);
    let (seeds_1, p1) = run(200_000);
    assert_eq!(p0, 0.0);
    assert!(p1 > 0.0);
    // seed set = 0.3 + 0.7 p: production rises at most by (0.3 + 0.7)/0.3.
    let gain = seeds_1 / seeds_0;
    assert!(gain > 1.0 && gain <= 1.0 / 0.3 + 1e-6, "seed production gain {gain:.3}");
}
