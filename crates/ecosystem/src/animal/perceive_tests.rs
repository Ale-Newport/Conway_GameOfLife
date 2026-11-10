//! Unit tests of perception: water search and reachability.

use physics_world::World;
use simulation_core::Grid2;
use simulation_core::glam::Vec2;

use super::perceive;
use crate::config::{AnimalSetup, EcosystemConfig, Terrain, TreeSetup};
use crate::environment::EnvCache;
use crate::params::RuntimeParams;
use crate::species::SpeciesCatalog;
use crate::state::{EcoState, Runtime};

/// One thirsty rabbit at (20, 20) on a 40 × 40 waterless map whose
/// terrain is replaced by `elevation(x, y)`.
fn thirsty_rabbit(elevation: impl FnMut(usize, usize) -> f32) -> (EcoState, Runtime) {
    let mut config = EcosystemConfig {
        terrain: Terrain::Flat { elevation_m: 50.0 },
        trees: TreeSetup { density_per_ha: 0.0, species: Vec::new() },
        animals: vec![AnimalSetup {
            species: "rabbit".into(),
            count: 1,
            region: Some((20.0, 20.0, 20.5, 20.5)),
            groups: None,
        }],
        ..EcosystemConfig::default()
    };
    config.world.width = 40;
    config.world.height = 40;
    config.world.river_count = 0;
    config.world.lake_tendency = 0.0;
    let catalog = SpeciesCatalog::builtin().unwrap();
    let (mut state, rt) = EcoState::build(config, 5, catalog, RuntimeParams::default()).unwrap();
    let cfg = state.world.config().clone();
    state.world = World::from_heightmap(&cfg, Grid2::from_fn(40, 40, elevation)).unwrap();
    state.env = EnvCache::new(&state.world);
    let rabbit = &mut state.animals.as_mut_slice()[0];
    rabbit.hydration = 0.1;
    rabbit.kin.heading = std::f32::consts::FRAC_PI_2; // facing +y
    (state, rt)
}

fn search_point(state: &EcoState, rt: &mut Runtime) -> Option<Vec2> {
    let positions = state.animals.as_slice().iter().enumerate().map(|(i, a)| (i as u32, a.kin.position));
    rt.grid.rebuild(positions);
    let senses = perceive(&state.ctx(rt), 0);
    assert!(senses.water.is_none(), "no water on the map");
    senses.water_search
}

#[test]
fn ground_hunters_cannot_reach_airborne_or_deep_prey() {
    let catalog = SpeciesCatalog::builtin().unwrap();
    let scale =
        crate::units::TimeScale { ticks_per_day: 240, days_per_year: 120, cell_size_m: 20.0, motion_seconds: 5.0 };
    let traits = |id: &str| crate::species::AnimalTraits::derive(catalog.animal(id).unwrap(), &scale);
    let (_, rt) = thirsty_rabbit(|_, _| 50.0);
    let mut prey = crate::animal::Animal::new(
        0,
        crate::animal::Sex::Female,
        organism_engine::Kinematics::new(Vec2::ZERO, 0.0),
        0,
        organism_engine::Genome::defaults(&rt.schema),
        organism_engine::Lineage::founder(organism_engine::AgentId(1), 0),
    );
    prey.altitude_m = 30.0;
    assert!(!super::reachable(&traits("fox"), &prey), "a fox cannot catch a flying duck");
    assert!(super::reachable(&traits("eagle"), &prey), "an eagle can");
    prey.altitude_m = -2.0;
    assert!(!super::reachable(&traits("heron"), &prey), "a wading heron cannot reach a fish 2 m deep");
    assert!(super::reachable(&traits("pike"), &prey), "a pike can");
    prey.altitude_m = -0.4;
    assert!(super::reachable(&traits("heron"), &prey), "nor a fish in the shallows");
}

#[test]
fn thirsty_animals_without_known_water_search_downhill() {
    // Terrain falls towards +x: the search heads east.
    let (state, mut rt) = thirsty_rabbit(|x, _| 100.0 - x as f32);
    let p = search_point(&state, &mut rt).expect("a thirsty rabbit searches for water");
    let from = state.animals.as_slice()[0].kin.position;
    assert!(p.x > from.x + 2.0 && (p.y - from.y).abs() < 0.5, "downhill is east: {from} → {p}");
    // On flat ground it keeps going straight ahead.
    let (state, mut rt) = thirsty_rabbit(|_, _| 50.0);
    let p = search_point(&state, &mut rt).expect("straight-ahead search on flat ground");
    let from = state.animals.as_slice()[0].kin.position;
    assert!(p.y > from.y + 2.0 && (p.x - from.x).abs() < 0.5, "ahead is +y: {from} → {p}");
    // A hydrated animal does not search.
    let (mut state, mut rt) = thirsty_rabbit(|x, _| 100.0 - x as f32);
    state.animals.as_mut_slice()[0].hydration = 1.0;
    assert!(search_point(&state, &mut rt).is_none());
}

#[test]
fn waders_reach_aquatic_field_prey_only_in_the_shallows() {
    let (mut state, mut rt) = thirsty_rabbit(|_, _| 50.0);
    // Two flooded cells: 0.6 m and 3 m deep, both full of sticklebacks.
    state.world.flood(5, 5, 0.5, 0.6).unwrap();
    state.world.flood(30, 30, 0.5, 3.0).unwrap();
    let (shallow, deep) = (5 * 40 + 5, 30 * 40 + 30);
    assert!((state.world.water_depth().as_slice()[shallow] - 0.6).abs() < 0.05);
    assert!(state.world.water_depth().as_slice()[deep] > 2.5);
    let fish = state.catalog.animal_index("small_fish").unwrap();
    let slot = state.fields.slot(fish).unwrap();
    state.fields.add(slot, shallow, 2.0);
    state.fields.add(slot, deep, 2.0);
    let positions = state.animals.as_slice().iter().enumerate().map(|(i, a)| (i as u32, a.kin.position));
    rt.grid.rebuild(positions);
    let ctx = state.ctx(&rt);
    let entry = |id: &str| {
        let s = usize::from(ctx.catalog.animal_index(id).unwrap());
        let k = ctx.catalog.animals()[s]
            .diet
            .iter()
            .position(|d| d.food == crate::species::Food::Prey("small_fish".into()));
        (s, k.unwrap())
    };
    let (heron, k) = entry("heron");
    assert!(super::available_entry(&ctx, heron, k, shallow) > 0.0, "a heron reaches fish in 0.6 m of water");
    assert_eq!(super::available_entry(&ctx, heron, k, deep), 0.0, "but not in 3 m");
    let (perch, k) = entry("perch");
    assert!(super::available_entry(&ctx, perch, k, deep) > 0.0, "a perch swims down to them");
    let (snake, k) = entry("snake");
    assert!(super::available_entry(&ctx, snake, k, deep) > 0.0, "a swimming grass snake too");
}
