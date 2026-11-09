//! The `environment.world` model: descriptor, parameters, snapshots and
//! replay, views, commands, seasons and long-run rainfall.

mod common;

use physics_world::sim::biome_fuel_capacity;
use physics_world::{
    Biome, Disaster, EnvironmentSim, MODEL_ID, Season, SurfaceKind, World, WorldConfig, WorldError, WorldLayer,
    WorldViewRequest,
};
use simulation_core::{ApplyPolicy, ParamValue, SimError, Simulation, erase};

fn small_sim(seed: u64) -> EnvironmentSim {
    EnvironmentSim::new(WorldConfig::with_size(64, 64).with_seed(seed)).expect("valid config")
}

#[test]
fn descriptor_declares_params_and_metrics_consistently() {
    let sim = small_sim(1);
    let descriptor = sim.descriptor();
    assert_eq!(descriptor.id, MODEL_ID);
    assert!(descriptor.rules.contains("Alexandridis"));
    assert!((descriptor.dt - 6.0).abs() < 1e-9 && descriptor.tick_unit == "minute");
    for spec in &descriptor.params {
        assert_eq!(spec.validate(&spec.default).as_ref(), Ok(&spec.default), "default of {}", spec.key);
    }
    let params = EnvironmentSim::new(WorldConfig::default()).expect("default config").params();
    for spec in &descriptor.params {
        assert_eq!(params.get(&spec.key), Some(&spec.default), "a default world reports the default of {}", spec.key);
    }
    let metrics = sim.metrics();
    let declared: Vec<&str> = descriptor.metrics.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(metrics.keys().collect::<Vec<_>>(), declared);
    assert!(sim.memory_bytes() > 64 * 64 * 4);
}

#[test]
fn snapshot_round_trip_and_replay_identity() {
    let mut sim = small_sim(7);
    sim.apply(Disaster::Storm { duration_ticks: 60, intensity: 1.5 }).expect("valid storm");
    for _ in 0..100 {
        sim.step();
    }
    sim.apply(Disaster::Ignite { x: 20, y: 20 }).expect("in bounds");
    let snapshot = sim.snapshot().expect("encodes");
    assert_eq!(snapshot.model_id, MODEL_ID);
    assert_eq!(snapshot.tick, 100);

    let mut original_metrics = Vec::new();
    for _ in 0..150 {
        sim.step();
        original_metrics.push(sim.metrics());
    }
    let final_snapshot = sim.snapshot().expect("encodes");

    let mut restored = small_sim(99);
    restored.restore(&snapshot).expect("decodes");
    assert_eq!(restored.tick(), 100);
    assert_eq!(restored.snapshot().expect("encodes").payload, snapshot.payload, "round trip is lossless");
    for expected in &original_metrics {
        restored.step();
        assert_eq!(&restored.metrics(), expected, "replay diverged at tick {}", restored.tick());
    }
    assert_eq!(restored.snapshot().expect("encodes").payload, final_snapshot.payload);

    let wrong = simulation_core::Snapshot { model_id: "other".into(), ..snapshot.clone() };
    assert!(matches!(restored.restore(&wrong), Err(SimError::WrongModel { .. })));
    let newer = simulation_core::Snapshot { schema_version: 99, ..snapshot };
    assert!(matches!(restored.restore(&newer), Err(SimError::UnsupportedVersion { .. })));
}

#[test]
fn runs_are_deterministic_through_the_dyn_interface() {
    let mut a = erase(small_sim(3));
    let mut b = erase(small_sim(3));
    a.advance(200);
    b.advance(200);
    assert_eq!(a.metrics(), b.metrics());
    assert_eq!(a.snapshot().expect("encodes"), b.snapshot().expect("encodes"));
}

#[test]
fn parameters_report_their_apply_policy() {
    let mut sim = small_sim(2);
    assert_eq!(sim.set_param("rainfall_mm", ParamValue::Float(1500.0)), Ok(ApplyPolicy::NextTick));
    assert_eq!(sim.world().config().rainfall_mm, 1500.0, "runtime parameters reach the world");
    let before = sim.world().climate().mean_temperature.clone();
    assert_eq!(sim.set_param("base_temperature_c", ParamValue::Int(20)), Ok(ApplyPolicy::NextTick));
    let after = &sim.world().climate().mean_temperature;
    assert!(
        before.as_slice().iter().zip(after.as_slice()).all(|(b, a)| (a - b - 6.0).abs() < 1e-4),
        "raising the base temperature from 14 to 20 °C warms every cell's annual mean by 6 °C"
    );

    assert_eq!(sim.set_param("width", ParamValue::Int(32)), Ok(ApplyPolicy::RequiresReset));
    assert_eq!(sim.world().width(), 64, "structural changes wait for a reset");
    assert_eq!(sim.params().i64_or("width", 0), 32);
    sim.reset();
    assert_eq!(sim.world().width(), 32);
    assert_eq!(sim.world().config().rainfall_mm, 1500.0, "edits survive the reset");

    assert!(matches!(sim.set_param("rainfall_mm", ParamValue::Float(-3.0)), Err(SimError::InvalidParameter { .. })));
    // Values at the edge of a range are reported exactly, so the reported
    // set always validates against the specs (f32 storage must not leak).
    assert_eq!(sim.set_param("flood_threshold_m", ParamValue::Float(0.01)), Ok(ApplyPolicy::NextTick));
    assert_eq!(sim.params().get("flood_threshold_m"), Some(&ParamValue::Float(0.01)));
    assert!(sim.params().resolve(&sim.descriptor().params).is_ok());
    assert!(matches!(sim.set_param("nope", ParamValue::Bool(true)), Err(SimError::UnknownParameter(_))));
    let huge_seed = WorldConfig::with_size(16, 16).with_seed(u64::MAX);
    assert!(matches!(EnvironmentSim::new(huge_seed), Err(WorldError::InvalidConfig { field: "seed", .. })));

    assert_eq!(sim.set_param("preset", ParamValue::Text("savanna".into())), Ok(ApplyPolicy::RequiresReset));
    assert_eq!(sim.params().str_or("preset", ""), "savanna");
    assert_eq!(sim.params().f64_or("base_temperature_c", 0.0), 25.0);
    sim.set_param("rainfall_mm", ParamValue::Float(700.0)).expect("valid");
    assert_eq!(sim.params().str_or("preset", ""), "none", "editing a preset field makes it custom");
    assert_eq!(sim.params().f64_or("base_temperature_c", 0.0), 25.0, "preset values are kept");
    assert_eq!(sim.params().f64_or("rainfall_mm", 0.0), 700.0);
}

#[test]
fn commands_go_through_apply_with_typed_errors() {
    let mut sim = small_sim(4);
    sim.apply(Disaster::Flood { x: 30, y: 30, radius: 5.0, depth_m: 2.0 }).expect("valid flood");
    let err = sim.apply(Disaster::Ignite { x: 640, y: 0 }).unwrap_err();
    assert!(matches!(err, SimError::InvalidCommand(_)));
    sim.step();
    assert!(sim.last_report().events.iter().any(|e| e.kind == "disaster"));
    assert!(!sim.events().is_empty());
}

#[test]
fn views_downsample_and_cache_terrain() {
    let sim = small_sim(5);
    let full = sim.view(&WorldViewRequest::default());
    let terrain = full.terrain.as_ref().expect("first request includes terrain");
    assert_eq!((terrain.elevation.width(), terrain.elevation.height()), (64, 64));
    assert_eq!(full.layers.len(), 4);
    assert!(full.fire.is_some());
    let request = WorldViewRequest {
        layers: vec![WorldLayer::Temperature, WorldLayer::Light],
        fire: false,
        block: 4,
        known_terrain_version: Some(full.terrain_version),
    };
    let coarse = sim.view(&request);
    assert!(coarse.terrain.is_none(), "terrain is not resent when unchanged");
    let t = coarse.layer(WorldLayer::Temperature).expect("requested");
    assert_eq!((t.width(), t.height()), (16, 16));
    let mean_full = common::mean(sim.world().temperature().as_slice());
    let mean_coarse = common::mean(t.as_slice());
    assert!((mean_full - mean_coarse).abs() < 1e-3, "block means preserve the average");
    assert!(coarse.layer(WorldLayer::Snow).is_none());
}

#[test]
fn vegetation_proxy_regrows_fuel_and_can_be_disabled() {
    let mut sim = small_sim(6);
    let world = sim.world();
    let land = (0..64 * 64)
        .find(|&i| {
            world.surface().as_slice()[i] == SurfaceKind::Land && biome_fuel_capacity(world.biome().as_slice()[i]) > 0.5
        })
        .expect("vegetated land");
    let capacity = biome_fuel_capacity(sim.world().biome().as_slice()[land]);
    let before = sim.world().fuel().as_slice()[land];
    assert!((before - 0.6 * capacity).abs() < 1e-6);
    for _ in 0..240 * 5 {
        sim.step();
    }
    let after = sim.world().fuel().as_slice()[land];
    assert!(after > before && after <= capacity, "{before} → {after} (capacity {capacity})");
    assert_eq!(biome_fuel_capacity(Biome::Ocean), 0.0);
    sim.set_param("vegetation_proxy", ParamValue::Bool(false)).expect("valid");
    let frozen = sim.world().fuel().as_slice()[land];
    sim.step();
    assert_eq!(sim.world().fuel().as_slice()[land], frozen);
}

#[test]
fn seasons_change_temperature_with_the_right_sign() {
    let cfg =
        WorldConfig { ticks_per_day: 24, days_per_season: 4, seasonality: 12.0, ..WorldConfig::with_size(48, 48) };
    let mut world = World::generate(&cfg);
    let mut sums = [0.0f64; 4];
    let mut counts = [0usize; 4];
    for _ in 0..2 * 16 * 24 {
        let season = world.time().season;
        world.step();
        let index = Season::ALL.iter().position(|s| *s == season).expect("season");
        sums[index] += f64::from(world.mean_land_temperature());
        counts[index] += 1;
    }
    let mean: Vec<f64> = (0..4).map(|k| sums[k] / counts[k] as f64).collect();
    let (spring, summer, autumn, winter) = (mean[0], mean[1], mean[2], mean[3]);
    assert!(summer > spring && summer > autumn, "summer is the warmest: {mean:?}");
    assert!(winter < spring && winter < autumn, "winter is the coldest: {mean:?}");
    assert!(summer - winter > 8.0, "seasonal swing {mean:?}");
}

#[test]
fn long_run_rainfall_matches_the_configured_climate() {
    let cfg = WorldConfig { ticks_per_day: 24, rainfall_mm: 900.0, ..WorldConfig::with_size(16, 16).with_seed(21) };
    let mut world = World::generate(&cfg);
    let days = 6 * 120;
    let mut total = 0.0;
    for _ in 0..days * 24 {
        total += world.step().rain_total_mm;
    }
    let per_day = total / f64::from(days);
    let expected = 900.0 / 365.0;
    // The rain rate is calibrated on the weather chain and the cloud field
    // has a unit map mean, so only weather randomness (six compressed years)
    // remains.
    assert!((per_day / expected - 1.0).abs() < 0.2, "mean rain {per_day:.2} mm/day, expected {expected:.2}");
}
