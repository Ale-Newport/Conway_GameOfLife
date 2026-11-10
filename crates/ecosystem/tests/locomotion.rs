//! Locomotion and habitat: flight altitude, fish confined to their water,
//! amphibious animals, the reach of ground hunters.

mod common;

use common::{build, coast, content, ids_of, pond, position, spawn};
use ecosystem::{Action, EcosystemCommand, EcosystemError};
use organism_engine::AgentId;
use physics_world::SurfaceKind;
use simulation_core::Simulation;
use simulation_core::glam::Vec2;

fn surface_at(sim: &ecosystem::EcosystemSim, p: Vec2) -> (SurfaceKind, f32) {
    let w = sim.world();
    let (x, y) = w.cell_at(p).expect("inside the map");
    let i = y * w.width() + x;
    (w.surface().as_slice()[i], w.water_depth().as_slice()[i])
}

#[test]
fn fliers_climb_while_travelling_land_to_feed_and_cross_water() {
    // A 40 × 40 plain of 20 m cells with a lake of radius 10 cells.
    let mut sim = build(pond(40, 10.0, 20.0), 3);
    sim.advance(60); // 06:00, daylight: eagles are diurnal
    let eagles = spawn(&mut sim, "eagle", 4.0, 20.0, 2);
    for &id in &eagles {
        content(&mut sim, id);
    }
    let mut highest = 0.0f32;
    let mut over_water = false;
    for _ in 0..240 {
        sim.step();
        for &id in &eagles {
            if let Some(a) = sim.animals().get(AgentId(id)) {
                highest = highest.max(a.altitude_m);
                if a.action == Action::Wander {
                    assert!(a.altitude_m > 0.0 || a.kin.speed() < 1e-3, "a wandering eagle flies");
                }
                if matches!(surface_at(&sim, a.kin.position).0, SurfaceKind::Lake) {
                    over_water = true;
                    assert!(a.altitude_m >= 1.0, "an eagle over the lake stays airborne");
                }
            }
        }
    }
    let cruise = sim.catalog().animal("eagle").unwrap().movement.flight_altitude_m;
    assert!(highest >= 0.9 * cruise, "eagles reach their cruising altitude: {highest} m of {cruise} m");
    assert!(over_water, "fliers cross the lake");
    for &id in &eagles {
        assert!(sim.animals().contains(AgentId(id)), "no eagle drowned");
    }
    assert_eq!(sim.counters().deaths_by_cause[ecosystem::DeathCause::Drowning as usize], 0);
}

#[test]
fn fish_stay_in_water_of_their_kind_and_depth() {
    // Fresh water: perch and bream in a lake.
    let mut sim = build(pond(40, 10.0, 20.0), 5);
    let perch = spawn(&mut sim, "perch", 20.0, 20.0, 6);
    let bream = spawn(&mut sim, "bream", 20.0, 20.0, 10);
    let perch_min = sim.catalog().animal("perch").unwrap().habitat.min_depth_m;
    for _ in 0..480 {
        sim.step();
        for &id in perch.iter().chain(&bream) {
            if let Some(a) = sim.animals().get(AgentId(id)) {
                let (surface, depth) = surface_at(&sim, a.kin.position);
                assert!(surface == SurfaceKind::Lake || depth > 0.3, "fish {id} left the water: {surface:?} {depth}");
                assert!(a.altitude_m < 0.0, "fish swim below the surface");
            }
        }
        for &id in &perch {
            if let Some(a) = sim.animals().get(AgentId(id)) {
                assert!(surface_at(&sim, a.kin.position).1 >= perch_min, "perch keep to deep enough water");
            }
        }
    }
    // A freshwater fish cannot be put into the sea, a reef fish can.
    let mut sea = build(coast(40, 0.5), 5);
    let err = sea
        .command(EcosystemCommand::SpawnAnimals { species: "perch".into(), x: 5.0, y: 20.0, radius: 3.0, count: 1 })
        .unwrap_err();
    assert!(matches!(err, EcosystemError::Command(_)), "{err}");
    let reef = spawn(&mut sea, "reef_fish", 5.0, 20.0, 8);
    sea.advance(240);
    for &id in &reef {
        if let Some(a) = sea.animals().get(AgentId(id)) {
            assert_eq!(surface_at(&sea, a.kin.position).0, SurfaceKind::Ocean, "reef fish stay at sea");
        }
    }
}

#[test]
fn amphibians_use_land_and_water() {
    let mut config = pond(40, 8.0, 20.0);
    config.animals =
        vec![ecosystem::AnimalSetup { species: "beetle".into(), count: 2_000_000, region: None, groups: None }];
    let mut sim = build(config, 9);
    sim.advance(1200); // a few days of evening and night activity
    let frogs = spawn(&mut sim, "frog", 20.0, 9.0, 4);
    let mut on_land = false;
    let mut in_water = false;
    for t in 0..960 {
        if t % 240 == 0 {
            // Dry skin sends frogs back to the water every day.
            for &id in &frogs {
                if let Some(a) = sim.animal_mut(id) {
                    a.hydration = 0.2;
                }
            }
        }
        sim.step();
        for &id in &frogs {
            if sim.animals().contains(AgentId(id)) {
                match surface_at(&sim, position(&sim, id)).0 {
                    SurfaceKind::Lake => in_water = true,
                    SurfaceKind::Land => on_land = true,
                    _ => {}
                }
            }
        }
    }
    assert!(on_land && in_water, "frogs move between land ({on_land}) and water ({in_water})");
    assert!(!ids_of(&sim, "frog").is_empty(), "frogs survive");
    assert_eq!(sim.counters().deaths_by_cause[ecosystem::DeathCause::Drowning as usize], 0);
}
