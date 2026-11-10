//! Aquatic life: forage fish as a density field confined to water of its
//! kind, waders that only reach fish in the shallows, the dissolved
//! nutrients that feed algae, and the water economy of reptiles.

mod common;

use common::{build, build_with, coast, edit_species, grassland, pond, spawn, traits_of};
use ecosystem::{AnimalSetup, EcosystemSim, Flux, PlantLayer};
use organism_engine::AgentId;
use physics_world::SurfaceKind;
use simulation_core::{ParamValue, Simulation};

fn setup(species: &str, count: u32) -> AnimalSetup {
    AnimalSetup { species: species.into(), count, region: None, groups: None }
}

fn fish_slot(sim: &EcosystemSim) -> usize {
    sim.fields().slot(sim.catalog().animal_index("small_fish").unwrap()).expect("small fish are a density field")
}

/// Fish (individuals) in water cells and on land.
fn fish_by_surface(sim: &EcosystemSim) -> (f64, f64) {
    let slot = fish_slot(sim);
    let area = sim.world().config().cell_area_m2();
    let (mut water, mut land) = (0.0, 0.0);
    for cell in 0..sim.world().width() * sim.world().height() {
        let n = f64::from(sim.fields().density(slot, cell)) * area;
        let wet = sim.world().surface().as_slice()[cell] != SurfaceKind::Land
            || sim.world().water_depth().as_slice()[cell] > 0.3;
        if wet { water += n } else { land += n }
    }
    (water, land)
}

#[test]
fn forage_fish_live_only_in_water() {
    // A lake on a dry plain: the fish field fills the lake and nothing else.
    let mut config = pond(32, 9.0, 20.0);
    config.world.base_temperature_c = 16.0;
    config.world.seasonality = 0.0;
    config.animals = vec![setup("small_fish", 200_000)];
    let mut sim = build(config, 3);
    let (water, land) = fish_by_surface(&sim);
    assert!(land == 0.0 && (water - 200_000.0).abs() < 1.0, "fish start in the lake: {water} / {land}");
    sim.advance(3 * 240);
    let (water, land) = fish_by_surface(&sim);
    assert_eq!(land, 0.0, "no fish on land after dispersal");
    assert!(water > 150_000.0, "the fish persist and feed in the lake: {water}");
    assert!(sim.ledgers().energy.total(Flux::Herbivory) > 0.0, "they graze algae");
    // Markers swim below the surface.
    let view = sim.view(&ecosystem::EcosystemViewRequest { field_markers: true, ..Default::default() });
    assert!(!view.field_markers.is_empty() && view.field_markers.iter().all(|m| m.altitude_m < 0.0));
}

#[test]
fn freshwater_and_salt_water_fields_follow_their_water_kind() {
    // Sticklebacks are euryhaline; a field restricted to fresh water stays
    // out of the sea.
    let mut config = coast(32, 0.5);
    config.animals = vec![setup("small_fish", 100_000)];
    let sea = build(config.clone(), 4);
    assert!(fish_by_surface(&sea).0 > 99_000.0, "euryhaline fish live at sea");
    let mut catalog = ecosystem::SpeciesCatalog::builtin().unwrap();
    edit_species(&mut catalog, "small_fish", |s| s.habitat.water = ecosystem::species::WaterKind::Fresh);
    let err = EcosystemSim::new(config, 4, catalog).err().expect("a freshwater field has no habitat at sea");
    assert!(err.to_string().contains("no habitat"), "{err}");
}

#[test]
fn herons_fish_the_shallows_of_a_lake() {
    // A 4 m deep lake whose fish field reaches the shore: a hungry heron
    // gleans fish where the water is at most 1 m deep (unit test of the
    // depth rule in `animal::perceive`).
    let mut config = pond(32, 9.0, 20.0);
    config.world.base_temperature_c = 16.0;
    config.world.seasonality = 0.0;
    config.animals = vec![setup("small_fish", 300_000)];
    let mut sim = build(config, 6);
    sim.advance(100); // daylight: herons are diurnal
    let heron = spawn(&mut sim, "heron", 16.0, 4.0, 1)[0];
    let traits = traits_of(&sim, "heron");
    if let Some(h) = sim.animal_mut(heron) {
        h.energy_kj = 0.2 * h.reserve_max_kj(&traits);
        h.hydration = 1.0;
        h.repro = ecosystem::ReproState::Recovering { until: u64::MAX };
    }
    let predation = sim.ledgers().energy.total(Flux::Predation);
    sim.advance(60);
    let eaten = sim.animals().get(AgentId(heron)).map_or(0.0, |h| h.stats.eaten_kj);
    eprintln!("a hungry heron ate {eaten:.0} kJ of fish in 6 hours");
    assert!(eaten > 0.0, "the heron fishes");
    assert!(sim.ledgers().energy.total(Flux::Predation) > predation, "fish eaten from the field are predation");
}

#[test]
fn dissolved_nutrients_keep_algae_growing() {
    // Water cells start with dissolved nutrients (`soil.water_nutrients_g_m2`)
    // and detritus in water is remineralised by bacteria, so a lake's algae
    // persist; without the dissolved stock they starve.
    let run = |nutrients: f32| {
        let mut config = pond(32, 9.0, 20.0);
        config.world.base_temperature_c = 18.0;
        config.world.seasonality = 0.0;
        config.soil.water_nutrients_g_m2 = nutrients;
        let mut sim = build(config, 8);
        let area = sim.world().config().cell_area_m2();
        let start = sim.vegetation().total_kg(PlantLayer::Algae, area);
        sim.advance(30 * 240);
        sim.vegetation().total_kg(PlantLayer::Algae, area) / start
    };
    let (fed, starved) = (run(2.0), run(0.0));
    eprintln!("algae after 30 days: {fed:.2} of the start with dissolved nutrients, {starved:.2} without");
    assert!(fed > 0.6, "algae persist on dissolved and recycled nutrients: {fed:.2}");
    assert!(starved < 0.6 * fed, "without nutrients they decline: {starved:.2} vs {fed:.2}");
}

/// Lizards on a plain without fresh water, with insects to eat; returns
/// their mean hydration and the deaths by dehydration after 12 days
/// (dehydration within 4 days without water or food, to shorten the test).
fn lizards_without_water(food_water: f32) -> (f32, u64) {
    let mut config = grassland(24);
    config.world.base_temperature_c = 22.0;
    config.world.seasonality = 0.0;
    config.world.rainfall_mm = 0.0;
    config.animals = vec![setup("beetle", 3_000_000), setup("spider", 500_000), setup("lizard", 30)];
    let mut sim = build_with(config, 10, |c| {
        edit_species(c, "lizard", |s| {
            s.metabolism.food_water = food_water;
            s.metabolism.dehydration_days = 4.0;
        })
    });
    sim.set_param("rainfall_mm", ParamValue::Float(0.0)).unwrap();
    sim.advance(12 * 240);
    let lizard = sim.catalog().animal_index("lizard").unwrap();
    let alive: Vec<f32> = sim.animals().iter().filter(|(_, a)| a.species == lizard).map(|(_, a)| a.hydration).collect();
    let mean = alive.iter().sum::<f32>() / alive.len().max(1) as f32;
    (mean, sim.counters().deaths_by_cause[ecosystem::DeathCause::Dehydration as usize])
}

#[test]
fn reptiles_get_their_water_from_prey() {
    let (reptile, reptile_deaths) = lizards_without_water(2.0);
    let (ordinary, ordinary_deaths) = lizards_without_water(1.0);
    eprintln!(
        "lizards without water for 12 days: mean hydration {reptile:.2} ({reptile_deaths} dehydrated) with the \
         reptile water economy, {ordinary:.2} ({ordinary_deaths} dehydrated) with the ordinary one"
    );
    assert!(reptile > 0.5, "insects cover a reptile's water need: hydration {reptile:.2}");
    assert!(reptile > ordinary + 0.1, "the reptile economy keeps them wetter: {reptile:.2} vs {ordinary:.2}");
    assert!(reptile_deaths <= 1 && reptile_deaths <= ordinary_deaths);
}
