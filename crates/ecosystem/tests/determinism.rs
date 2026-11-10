//! Deterministic replay and snapshots.

mod common;

use common::{build, grassland, with_animals};
use ecosystem::scenario::find_builtin;
use simulation_core::{DynSimulation, Simulation, Snapshot, erase};

fn small(seed: u64) -> ecosystem::EcosystemSim {
    let mut config = with_animals(grassland(32), &[("rabbit", 30), ("deer", 6), ("fox", 3), ("wolf", 2), ("raven", 2)]);
    config.trees.density_per_ha = 25.0;
    config.world.rainfall_mm = 700.0;
    build(config, seed)
}

#[test]
fn same_seed_gives_bit_identical_runs() {
    let mut a = small(5);
    let mut b = small(5);
    a.advance(300);
    b.advance(300);
    assert_eq!(a.snapshot().unwrap(), b.snapshot().unwrap());
    let mut c = small(6);
    c.advance(300);
    assert_ne!(a.snapshot().unwrap().payload, c.snapshot().unwrap().payload, "another seed differs");
}

#[test]
fn snapshot_round_trip_replays_exactly() {
    let mut sim = small(7);
    sim.advance(150);
    let mid = sim.snapshot().unwrap();
    assert_eq!(mid.schema_version, ecosystem::SCHEMA_VERSION);
    assert_eq!(mid.model_id, ecosystem::MODEL_ID);
    sim.advance(150);
    let expected = sim.snapshot().unwrap();
    // Through the byte encoding and the object-safe interface.
    let mut other: Box<dyn DynSimulation> = erase(small(99));
    other.restore(&Snapshot::from_bytes(&mid.to_bytes().unwrap()).unwrap()).unwrap();
    assert_eq!(other.tick(), 150);
    other.advance(150);
    assert_eq!(other.snapshot().unwrap(), expected);
}

#[test]
fn netlogo_and_meadow_scenarios_replay() {
    for id in ["wolf_sheep_grass", "balanced_meadow"] {
        let mut sim = find_builtin(id).unwrap().build().unwrap();
        sim.advance(60);
        let snap = sim.snapshot().unwrap();
        sim.advance(60);
        let after = sim.metrics();
        sim.restore(&snap).unwrap();
        sim.advance(60);
        assert_eq!(sim.metrics(), after, "{id}");
    }
}

#[test]
fn restore_rejects_foreign_and_newer_snapshots() {
    let mut sim = small(1);
    let good = sim.snapshot().unwrap();
    let foreign = Snapshot { model_id: "environment.world".into(), ..good.clone() };
    assert!(sim.restore(&foreign).is_err());
    let newer = Snapshot { schema_version: ecosystem::SCHEMA_VERSION + 1, ..good.clone() };
    assert!(sim.restore(&newer).is_err());
    let garbage = Snapshot { payload: vec![1, 2, 3], ..good.clone() };
    assert!(sim.restore(&garbage).is_err());
    sim.restore(&good).unwrap();
}

#[test]
fn scenarios_with_fields_colonies_diseases_and_schedules_replay() {
    for id in ["temperate_forest", "forest_fire"] {
        let mut a = find_builtin(id).unwrap().build().unwrap();
        let mut b = find_builtin(id).unwrap().build().unwrap();
        a.advance(120);
        b.advance(120);
        assert_eq!(a.snapshot().unwrap(), b.snapshot().unwrap(), "{id}: same seed, same state");
        let snap = a.snapshot().unwrap();
        a.advance(120);
        let expected = a.snapshot().unwrap();
        b.restore(&snap).unwrap();
        b.advance(120);
        assert_eq!(b.snapshot().unwrap(), expected, "{id}: a restored run replays exactly");
    }
}

#[test]
fn older_snapshots_are_rejected_with_a_clear_message() {
    let mut sim = small(2);
    for version in 1..ecosystem::SCHEMA_VERSION {
        let old = Snapshot { schema_version: version, ..sim.snapshot().unwrap() };
        let err = sim.restore(&old).unwrap_err().to_string();
        assert!(err.contains(&format!("schema version {version}")), "{err}");
    }
}
