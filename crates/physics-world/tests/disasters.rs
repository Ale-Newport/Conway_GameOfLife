//! Disasters change the world physically and validate their inputs.

mod common;

use physics_world::{BiomePreset, Disaster, FireState, SurfaceKind, WeatherKind, World, WorldConfig, WorldError};
use simulation_core::Grid2;

fn cone_world() -> World {
    // A steep cone (slope ≈ 1.2) on a plain: ideal for slope failures.
    let cfg = WorldConfig { cell_size_m: 50.0, ..common::flat_config(48, 9) };
    let cone = Grid2::from_fn(48, 48, |x, y| {
        let d = ((x as f32 - 24.0).powi(2) + (y as f32 - 24.0).powi(2)).sqrt();
        (1200.0 - 60.0 * d).max(20.0)
    });
    World::from_heightmap(&cfg, cone).expect("valid cone")
}

#[test]
fn earthquake_displaces_terrain_and_bumps_the_version() {
    let mut world = World::generate(&WorldConfig::with_size(96, 96).with_seed(6).with_preset(BiomePreset::Mountains));
    let before = world.elevation().clone();
    let version = world.terrain_version();
    let outcome = world.earthquake(48, 48, 25.0, 7.5).expect("valid quake");
    assert!(outcome.terrain_changed);
    assert!(outcome.max_elevation_change_m > 1.0, "M7.5 displaces metres: {}", outcome.max_elevation_change_m);
    assert!(world.terrain_version() > version);
    let changed = before.as_slice().iter().zip(world.elevation().as_slice()).filter(|(a, b)| a != b).count();
    assert!(changed > 1000);
    // Cells far outside the radius are untouched.
    assert_eq!(before.get(2, 2), world.elevation().get(2, 2));
    let report = world.step();
    assert!(report.terrain_changed);
    assert!(report.events.iter().any(|e| e.kind == "disaster"));
}

#[test]
fn landslides_move_soil_downhill_and_conserve_it() {
    let mut world = cone_world();
    let total_before: f64 = world.elevation().as_slice().iter().map(|&z| f64::from(z)).sum();
    let peak_before = *world.elevation().get(24, 20);
    let outcome = world.landslide(24, 24, 10.0).expect("valid landslide");
    assert!(outcome.landslides > 0 && outcome.terrain_changed);
    let total_after: f64 = world.elevation().as_slice().iter().map(|&z| f64::from(z)).sum();
    assert!((total_after - total_before).abs() < 1e-3 * total_before.abs(), "soil volume is conserved");
    assert!(*world.elevation().get(24, 20) <= peak_before);
}

#[test]
fn volcano_raises_a_cone_pours_lava_and_drops_ash() {
    let mut world = common::flat_world(64, 2);
    world.fuel_mut().fill(1.0);
    let organic_before = *world.organic_matter().get(50, 32);
    let outcome = world.volcano(32, 32, 10.0, 600.0).expect("valid volcano");
    assert!(outcome.ignited > 0 && outcome.terrain_changed);
    assert!(*world.elevation().get(32, 30) > 400.0, "the cone rises");
    assert!(matches!(world.fire().get(32, 32), FireState::Lava { .. }));
    assert!(*world.organic_matter().get(50, 32) > organic_before, "ash falls downwind and around");
    assert!(world.nutrient_budget().deposited_g > 0.0);
    let report = world.step();
    assert!(report.fuel_consumed.iter().any(|b| (b.x, b.y) == (32, 32)), "lava consumed the fuel");
    for _ in 0..10 {
        world.step();
    }
    assert!(world.total_burned_cells() > 0 || world.burning_cells().len() > outcome.ignited as usize);
}

#[test]
fn tornado_reports_a_damage_swath() {
    let mut world = common::flat_world(64, 3);
    world.canopy_mut().fill(0.8);
    let outcome = world.tornado(vec![[5.0, 5.0], [58.0, 40.0]], 6.0).expect("valid tornado");
    assert!(outcome.affected_cells > 50);
    let on_path = outcome.wind_damage.iter().find(|d| (d.x, d.y) == (5, 5)).expect("start of the path");
    assert!((on_path.damage - 1.0).abs() < 1e-6);
    assert!(*world.canopy().get(5, 5) < 1e-6);
    assert_eq!(*world.canopy().get(60, 5), 0.8, "cells away from the path are untouched");
    let report = world.step();
    assert_eq!(report.wind_damage.len(), outcome.wind_damage.len());
}

#[test]
fn imposed_weather_spells_change_the_climate() {
    let cfg = WorldConfig::with_size(48, 48).with_seed(12);
    let mut control = World::generate(&cfg);
    let mut hot = World::generate(&cfg);
    hot.heatwave(480).expect("valid heatwave");
    let mut stormy = World::generate(&cfg);
    stormy.storm(480, 2.0).expect("valid storm");
    let (mut hot_t, mut control_t, mut rain_storm, mut rain_control) = (0.0, 0.0, 0.0, 0.0);
    for _ in 0..480 {
        rain_control += control.step().rain_total_mm;
        hot.step();
        rain_storm += stormy.step().rain_total_mm;
        hot_t += f64::from(hot.mean_land_temperature());
        control_t += f64::from(control.mean_land_temperature());
    }
    assert_eq!(hot.weather().kind(), WeatherKind::Heatwave);
    assert!(hot_t / 480.0 > control_t / 480.0 + 3.0, "heatwave warms the land");
    assert!(rain_storm > rain_control + 10.0, "storm rain {rain_storm} vs {rain_control} mm");
    assert!(matches!(stormy.weather().kind(), WeatherKind::Storm | WeatherKind::Snow));
}

#[test]
fn invalid_interventions_are_rejected_without_side_effects() {
    let mut world = common::flat_world(32, 1);
    let water = world.water_storage();
    let cases = [
        Disaster::Ignite { x: 32, y: 0 },
        Disaster::Drought { duration_ticks: 0 },
        Disaster::Storm { duration_ticks: 10, intensity: f32::NAN },
        Disaster::AddRain { amount_mm: -5.0 },
        Disaster::Flood { x: 4, y: 4, radius: 0.0, depth_m: 1.0 },
        Disaster::Earthquake { x: 4, y: 4, radius: 5.0, magnitude: 12.0 },
        Disaster::Tornado { path: vec![[1.0, 1.0]], width: 3.0 },
        Disaster::Tornado { path: vec![[1.0, 1.0], [f32::INFINITY, 2.0]], width: 3.0 },
        Disaster::Volcano { x: 4, y: 4, radius: 1.0, height_m: 100.0 },
    ];
    for case in cases {
        let kind = case.kind();
        let result = world.trigger(case);
        assert!(
            matches!(result, Err(WorldError::InvalidArgument { .. } | WorldError::OutOfBounds { .. })),
            "{kind} should be rejected, got {result:?}"
        );
    }
    assert_eq!(world.water_storage(), water);
    assert!(world.weather().forced().is_none());
    let message = world.trigger(Disaster::Ignite { x: 99, y: 3 }).unwrap_err().to_string();
    assert!(message.contains("outside"), "{message}");
}

#[test]
fn sinking_land_hands_its_water_to_the_ocean() {
    // A low coastal plain next to the sea; a strong quake drops part of it
    // below sea level.
    let cfg = common::flat_config(48, 5);
    let coast = Grid2::from_fn(48, 48, |x, _| if x < 8 { -20.0 } else { 0.8 });
    let mut world = World::from_heightmap(&cfg, coast).expect("valid coast");
    world.add_rain(20.0).expect("valid rain");
    let ocean_before = world.surface().as_slice().iter().filter(|s| **s == SurfaceKind::Ocean).count();
    world.trigger(Disaster::Earthquake { x: 10, y: 24, radius: 15.0, magnitude: 9.0 }).expect("valid quake");
    let ocean_after = world.surface().as_slice().iter().filter(|s| **s == SurfaceKind::Ocean).count();
    assert!(ocean_after > ocean_before, "subsided coast floods: {ocean_before} → {ocean_after}");
    let storage = world.water_storage().total_m3();
    let expected = world.water_budget().expected_storage_m3();
    assert!((storage - expected).abs() < 1e-6 * expected.max(1.0));
}

#[test]
fn uplift_that_cuts_off_a_bay_leaves_a_lake() {
    // Open sea for x < 5, a 1-cell channel (y = 15, x = 5..9) into a bay
    // (x 10..20, y 10..20), land at 5 m elsewhere. A volcano on the channel
    // lifts it above sea level and cuts the bay off.
    let cfg = common::flat_config(32, 7);
    let terrain = Grid2::from_fn(32, 32, |x, y| {
        let sea = x < 5;
        let channel = y == 15 && (5..10).contains(&x);
        let bay = (10..=20).contains(&x) && (10..=20).contains(&y);
        if sea || channel || bay { -10.0 } else { 5.0 }
    });
    let mut world = World::from_heightmap(&cfg, terrain).expect("valid terrain");
    assert_eq!(world.surface().get(15, 15), &SurfaceKind::Ocean, "the bay starts as open sea");
    world.volcano(7, 15, 3.0, 200.0).expect("valid volcano");
    let bay: Vec<(usize, usize)> = (10..=20).flat_map(|y| (10..=20).map(move |x| (x, y))).collect();
    for &(x, y) in &bay {
        assert_eq!(world.surface().get(x, y), &SurfaceKind::Lake, "({x}, {y}) is cut off from the sea");
        let depth = f64::from(*world.water_depth().get(x, y));
        let floor = f64::from(*world.elevation().get(x, y));
        assert!((depth + floor).abs() < 1e-3, "the lake surface stays at sea level: depth {depth}, floor {floor}");
    }
    assert_eq!(world.surface().get(2, 15), &SurfaceKind::Ocean);
    let budget = world.water_budget();
    assert!(budget.added_m3 > 0.0, "captured sea water is booked as an input");
    let storage = world.water_storage().total_m3();
    assert!((storage - budget.expected_storage_m3()).abs() < 1e-6 * storage);
    for _ in 0..50 {
        world.step();
    }
    assert!(*world.water_depth().get(15, 15) > 9.0, "the lake keeps its water");
}
