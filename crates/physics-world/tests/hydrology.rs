//! Hydrology: water mass accounting, runoff, floods, drought, snow, baseflow
//! and nutrient conservation.

mod common;

use physics_world::{BiomePreset, Disaster, MAX_NUTRIENT_ADDITION_G_M2, SurfaceKind, World, WorldConfig, WorldError};
use simulation_core::Grid2;

fn assert_water_balance(world: &World) {
    let storage = world.water_storage().total_m3();
    let budget = world.water_budget();
    let scale = budget.inputs_m3().max(1.0);
    let error = (storage - budget.expected_storage_m3()).abs() / scale;
    assert!(
        error < 1e-3,
        "water not conserved: storage {storage}, expected {}, relative error {error}",
        budget.expected_storage_m3()
    );
}

#[test]
fn water_mass_is_conserved_through_rain_floods_and_routing() {
    let mut world = World::generate(&WorldConfig::with_size(96, 96).with_seed(11));
    world.storm(240, 2.0).expect("valid storm");
    for _ in 0..300 {
        world.step();
    }
    world.add_rain(25.0).expect("valid rain");
    let added = world.trigger(Disaster::Flood { x: 48, y: 48, radius: 10.0, depth_m: 2.0 }).expect("valid flood");
    assert!(added.water_added_m3 > 0.0);
    for _ in 0..300 {
        world.step();
    }
    let budget = world.water_budget();
    assert!(budget.precipitation_m3 > 0.0, "the storm must rain");
    assert!(budget.evaporated_m3 > 0.0);
    assert!(budget.outflow_ocean_m3 + budget.outflow_edge_m3 > 0.0, "runoff must leave the map");
    assert_water_balance(&world);
}

#[test]
fn water_balance_holds_for_snow_climates_and_disasters() {
    let mut world = World::generate(&WorldConfig::with_size(64, 64).with_seed(3).with_preset(BiomePreset::Tundra));
    world.storm(100, 1.0).expect("valid storm");
    for _ in 0..200 {
        world.step();
    }
    assert!(world.water_storage().snow_m3 > 0.0, "cold storms build a snow pack");
    world.trigger(Disaster::Earthquake { x: 32, y: 32, radius: 20.0, magnitude: 7.5 }).expect("valid quake");
    world.trigger(Disaster::Volcano { x: 20, y: 40, radius: 6.0, height_m: 400.0 }).expect("valid volcano");
    for _ in 0..200 {
        world.step();
    }
    assert_water_balance(&world);
}

#[test]
fn runoff_moves_water_downhill_and_off_the_map() {
    // A ramp falling towards x = 0 (2 m per cell): rain drains to the low edge.
    let cfg = WorldConfig { river_count: 0, lake_tendency: 0.0, ..common::flat_config(48, 1) };
    let ramp = Grid2::from_fn(48, 48, |x, _| 10.0 + 2.0 * x as f32);
    let mut world = World::from_heightmap(&cfg, ramp).expect("valid ramp");
    world.start_drought(10_000).expect("valid drought");
    world.add_rain(40.0).expect("valid rain");
    for _ in 0..30 {
        world.step();
    }
    let depth = world.water_depth();
    let column_mean = |x: usize| (1..47).map(|y| f64::from(*depth.get(x, y))).sum::<f64>() / 46.0;
    assert!(column_mean(1) > column_mean(40), "water collects at the low side");
    assert!(world.water_budget().outflow_edge_m3 > 0.0, "water leaves across the low edge");
}

#[test]
fn floods_are_reported_and_spread() {
    let mut world = common::flat_world(48, 2);
    world.flood(24, 24, 6.0, 3.0).expect("valid flood");
    let peak_before = f64::from(*world.water_depth().get(24, 24));
    let report = world.step();
    assert!(report.flooded_cells > 0);
    assert!(report.events.iter().any(|e| e.kind == "flood"));
    assert!(world.is_flooded(24, 24));
    for _ in 0..50 {
        world.step();
    }
    assert!(f64::from(*world.water_depth().get(24, 24)) < peak_before, "the flood wave spreads out");
    assert!(*world.water_depth().get(24, 33) > 0.0, "water reaches cells outside the initial radius");
}

#[test]
fn drought_dries_the_soil_and_raises_the_index() {
    let cfg = WorldConfig::with_size(64, 64).with_seed(5);
    let mut control = World::generate(&cfg);
    let mut dry = World::generate(&cfg);
    dry.start_drought(240 * 20).expect("valid drought");
    for _ in 0..240 * 20 {
        control.step();
        let report = dry.step();
        assert_eq!(report.rain_total_mm, 0.0, "no rain during a drought");
    }
    assert!(dry.mean_land_moisture() < control.mean_land_moisture());
    assert!(dry.drought_index() > control.drought_index());
    assert!(dry.drought_index() > 0.4, "index {}", dry.drought_index());
}

#[test]
fn rivers_keep_flowing_on_baseflow_without_rain() {
    let mut world = World::generate(&WorldConfig::with_size(96, 96).with_seed(8));
    world.start_drought(240 * 5).expect("valid drought");
    let groundwater_before = world.groundwater_m3();
    let mut last_day_outflow = 0.0;
    for tick in 0..240 * 5 {
        let report = world.step();
        assert_eq!(report.rain_volume_m3, 0.0);
        if tick >= 240 * 4 {
            last_day_outflow += report.outflow_m3;
        }
    }
    let rivers: Vec<usize> = (0..96 * 96).filter(|&i| world.surface().as_slice()[i] == SurfaceKind::River).collect();
    assert!(!rivers.is_empty());
    assert!(
        rivers.iter().all(|&i| world.water_depth().as_slice()[i] > 1e-5),
        "every river cell keeps flowing water after five dry days"
    );
    assert!(last_day_outflow > 1000.0, "rivers still discharge {last_day_outflow} m³/day");
    assert!(world.groundwater_m3() < groundwater_before, "baseflow drains the aquifer");
}

#[test]
fn nutrients_are_conserved_and_never_negative() {
    let mut world = World::generate(&WorldConfig::with_size(48, 48).with_seed(4));
    let (x, y) = (0..48 * 48)
        .map(|i| (i % 48, i / 48))
        .find(|&(x, y)| world.surface().get(x, y) == &SurfaceKind::Land)
        .expect("land cell");
    let available = *world.nutrients().get(x, y);
    let taken = world.take_nutrients(x, y, available + 100.0).expect("valid take");
    assert_eq!(taken, available, "cannot take more than is there");
    assert_eq!(*world.nutrients().get(x, y), 0.0);
    assert_eq!(world.add_nutrients(x, y, 12.5).expect("valid add"), 12.5);
    world.add_organic_matter(x, y, 40.0).expect("valid add");
    assert!(world.take_nutrients(x, y, -1.0).is_err());
    assert!(world.add_nutrients(x, y, f32::NAN).is_err());
    // Additions are capped so that the pools stay finite.
    assert!(world.add_nutrients(x, y, f32::MAX).is_err());
    assert!(world.add_organic_matter(x, y, 2.0 * MAX_NUTRIENT_ADDITION_G_M2).is_err());
    assert_eq!(*world.nutrients().get(x, y), 12.5, "rejected additions change nothing");
    assert!(matches!(world.add_nutrients(48, 0, 1.0), Err(WorldError::OutOfBounds { .. })));
    world.storm(400, 1.5).expect("valid storm");
    for _ in 0..800 {
        world.step();
    }
    assert!(world.nutrients().as_slice().iter().all(|&n| n >= 0.0));
    assert!(world.organic_matter().as_slice().iter().all(|&n| n >= 0.0));
    let budget = world.nutrient_budget();
    assert!(budget.leached_g > 0.0, "percolation leaches nutrients");
    let stored = world.nutrient_storage_g();
    assert!((stored - budget.expected_g()).abs() / budget.initial_g < 1e-4, "{stored} vs {}", budget.expected_g());
}
