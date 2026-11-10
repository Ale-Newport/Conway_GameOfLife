//! Helpers shared by the integration tests.
#![allow(dead_code)]

use ecosystem::{
    AnimalSetup, EcosystemCommand, EcosystemConfig, EcosystemSim, SoilSetup, SpeciesCatalog, Terrain, TreeSetup,
    VegetationSetup,
};
use organism_engine::AgentId;
use physics_world::WorldConfig;
use simulation_core::{ParamSet, ParamValue};

/// A flat grassland of `size × size` cells of 20 m without trees or animals.
pub fn grassland(size: usize) -> EcosystemConfig {
    EcosystemConfig {
        world: WorldConfig {
            width: size,
            height: size,
            cell_size_m: 20.0,
            sea_level: 0.0,
            relief: 0.0,
            river_count: 0,
            lake_tendency: 0.0,
            rainfall_mm: 400.0,
            base_temperature_c: 14.0,
            latitude_gradient: 0.0,
            ..WorldConfig::default()
        },
        terrain: Terrain::Flat { elevation_m: 50.0 },
        trees: TreeSetup { density_per_ha: 0.0, species: Vec::new() },
        vegetation: VegetationSetup::default(),
        soil: SoilSetup::default(),
        animals: Vec::new(),
        ..EcosystemConfig::default()
    }
}

/// A flat plain of `size × size` cells of `cell_m` metres with a round
/// freshwater lake of `radius` cells in the middle (4 m deep at the centre).
/// The world keeps lakes up to a quarter of the land: `π r² ≤ size² / 4`.
pub fn pond(size: usize, radius: f32, cell_m: f32) -> EcosystemConfig {
    let mut config = grassland(size);
    config.world.cell_size_m = cell_m;
    config.world.lake_tendency = 1.0;
    config.terrain = Terrain::Basin { elevation_m: 50.0, depth_m: 4.0, radius_cells: radius };
    config
}

/// A coast: the western `sea_fraction` of a `size × size` map is sea 20 m
/// deep, the rest a flat plain.
pub fn coast(size: usize, sea_fraction: f32) -> EcosystemConfig {
    let mut config = grassland(size);
    config.world.base_temperature_c = 25.0;
    config.terrain = Terrain::Coast { elevation_m: 10.0, sea_depth_m: 20.0, sea_fraction };
    config
}

/// Adds initial animals.
pub fn with_animals(mut config: EcosystemConfig, animals: &[(&str, u32)]) -> EcosystemConfig {
    config.animals = animals
        .iter()
        .map(|(s, n)| AnimalSetup { species: (*s).to_string(), count: *n, region: None, groups: None })
        .collect();
    config
}

/// Adds parameter overrides.
pub fn with_params(mut config: EcosystemConfig, params: &[(&str, ParamValue)]) -> EcosystemConfig {
    let mut set = ParamSet::new();
    for (k, v) in params {
        set.set(*k, v.clone());
    }
    config.params = set;
    config
}

/// Builds a run with the built-in species.
pub fn build(config: EcosystemConfig, seed: u64) -> EcosystemSim {
    EcosystemSim::new(config, seed, SpeciesCatalog::builtin().expect("built-in species")).expect("valid configuration")
}

/// Spawns `count` animals of `species` near `(x, y)` and returns their ids.
pub fn spawn(sim: &mut EcosystemSim, species: &str, x: f32, y: f32, count: u32) -> Vec<u64> {
    let before: Vec<AgentId> = sim.animals().ids().to_vec();
    sim.command(EcosystemCommand::SpawnAnimals { species: species.to_string(), x, y, radius: 0.5, count })
        .expect("spawn succeeds");
    sim.animals().ids().iter().filter(|id| !before.contains(id)).map(|id| id.get()).collect()
}

/// Ids of the living animals of a species.
pub fn ids_of(sim: &EcosystemSim, species: &str) -> Vec<u64> {
    let index = sim.catalog().animal_index(species).expect("known species");
    sim.animals().iter().filter(|(_, a)| a.species == index).map(|(id, _)| id.get()).collect()
}

/// Sets the condition of an animal so that it only wanders: fed, watered,
/// rested and not ready to mate.
pub fn content(sim: &mut EcosystemSim, id: u64) {
    let traits = traits_of(sim, &species_of(sim, id));
    if let Some(a) = sim.animal_mut(id) {
        a.energy_kj = 0.95 * a.reserve_max_kj(&traits);
        a.hydration = 1.0;
        a.fatigue = 0.0;
        a.repro = ecosystem::ReproState::Recovering { until: u64::MAX };
    }
}

/// Species id of an animal.
pub fn species_of(sim: &EcosystemSim, id: u64) -> String {
    let a = sim.animals().get(AgentId(id)).expect("alive");
    sim.catalog().animals()[usize::from(a.species)].id.clone()
}

/// Position of a living animal.
pub fn position(sim: &EcosystemSim, id: u64) -> simulation_core::glam::Vec2 {
    sim.animals().get(AgentId(id)).expect("alive").kin.position
}

/// Builds a run with a modified built-in catalogue.
pub fn build_with(config: EcosystemConfig, seed: u64, edit: impl FnOnce(&mut SpeciesCatalog)) -> EcosystemSim {
    let mut catalog = SpeciesCatalog::builtin().expect("built-in species");
    edit(&mut catalog);
    EcosystemSim::new(config, seed, catalog).expect("valid configuration")
}

/// Replaces a species of a catalogue after editing it.
pub fn edit_species(catalog: &mut SpeciesCatalog, id: &str, edit: impl FnOnce(&mut ecosystem::Species)) {
    let mut s = catalog.animal(id).expect("known species").clone();
    edit(&mut s);
    catalog.insert_animal(s).expect("valid species");
}

/// Derived traits of a species.
pub fn traits_of(sim: &EcosystemSim, species: &str) -> ecosystem::AnimalTraits {
    sim.traits()[usize::from(sim.catalog().animal_index(species).expect("known species"))].clone()
}
