//! Every scenario file parses, matches its embedded copy, builds with the
//! asset pattern library and runs; selected scenarios behave as described.

mod common;

use std::path::PathBuf;

use cellular_automata::scenario::{
    BuiltSimulation, CA_LAB_SCENARIO_FILES, CONWAY_SCENARIO_FILES, builtin_scenario, builtin_scenarios, load_dir,
};
use cellular_automata::{EngineKind, PatternLibrary};
use simulation_core::Simulation;

fn library() -> PatternLibrary {
    PatternLibrary::with_catalog(common::catalog())
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios")
}

#[test]
fn files_on_disk_equal_the_embedded_copies() {
    for (folder, files) in [("conway", &CONWAY_SCENARIO_FILES[..]), ("ca-lab", &CA_LAB_SCENARIO_FILES[..])] {
        let dir = root().join(folder);
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".ron"))
            .collect();
        names.sort();
        let embedded: Vec<String> = files.iter().map(|(n, _)| (*n).to_string()).collect();
        assert_eq!(names, embedded, "{folder}: every file is embedded");
        for (name, text) in files {
            assert_eq!(std::fs::read_to_string(dir.join(name)).unwrap(), *text, "{folder}/{name}");
        }
        assert_eq!(load_dir(&dir).unwrap().len(), files.len());
    }
}

#[test]
fn every_scenario_builds_and_runs() {
    let library = library();
    let all = builtin_scenarios().unwrap();
    assert_eq!(all.len(), 29);
    for scenario in &all {
        let mut sim = scenario.build_dyn(&library).unwrap_or_else(|e| panic!("{}: {e}", scenario.id));
        assert_eq!(sim.descriptor().id, scenario.model_id(), "{}", scenario.id);
        let before = sim.snapshot().unwrap();
        sim.advance(3);
        assert!(sim.tick() >= 3, "{} advanced", scenario.id);
        let after = sim.metrics();
        sim.restore(&before).unwrap();
        sim.advance(3);
        assert_eq!(sim.metrics(), after, "{}: deterministic replay", scenario.id);
    }
    // Scenarios using only embedded patterns also build without the asset catalog.
    let builtin = PatternLibrary::builtin();
    for id in ["glider-gun-showcase", "random-soup-torus", "highlife-replicator", "wolfram-rule-30"] {
        assert!(builtin_scenario(id).unwrap().build(&builtin).is_ok(), "{id}");
    }
    // Catalog patterns and the GPL Wireworld gate are read from the assets.
    for id in ["infinite-puffer-world", "wireworld-circuits"] {
        let err = builtin_scenario(id).unwrap().build(&builtin).err().unwrap().to_string();
        assert!(err.contains("asset catalog"), "{id}: {err}");
    }
}

#[test]
fn glider_gun_showcase_streams_diverge() {
    let BuiltSimulation::Life(mut sim) = builtin_scenario("glider-gun-showcase").unwrap().build(&library()).unwrap()
    else {
        panic!("a Life scenario");
    };
    assert_eq!(sim.engine_kind(), EngineKind::Sparse);
    sim.advance(300);
    let p0 = sim.population();
    sim.advance(300);
    assert_eq!(sim.population(), p0 + 2 * 10 * 5, "two guns, ten gliders each, nothing collides");
}

#[test]
fn engine_choices_of_scenarios() {
    let library = library();
    let kind = |id: &str| match builtin_scenario(id).unwrap().build(&library).unwrap() {
        BuiltSimulation::Life(mut s) => {
            s.step();
            s.engine_kind()
        }
        _ => panic!("{id} is a Life scenario"),
    };
    assert_eq!(kind("large-bit-soup"), EngineKind::Bit);
    assert_eq!(kind("bounded-r-pentomino"), EngineKind::Dense, "128 × 128 is the largest automatic dense world");
    assert_eq!(kind("infinite-puffer-world"), EngineKind::Hashlife);
    assert_eq!(kind("breeder-hashlife"), EngineKind::Hashlife);
    assert_eq!(kind("methuselah-gallery"), EngineKind::Sparse);
}

#[test]
fn corrupted_snapshots_are_rejected_without_panicking() {
    // Bytes spread over snapshots of every model are altered in turn; restoring
    // must either succeed or fail with an error, never panic.
    let library = library();
    for id in [
        "bounded-r-pentomino",
        "glider-gun-showcase",
        "infinite-puffer-world",
        "brians-brain",
        "wireworld-circuits",
        "langtons-ant-highway",
        "wolfram-rule-30",
    ] {
        let scenario = builtin_scenario(id).unwrap();
        let mut sim = scenario.build_dyn(&library).unwrap();
        sim.advance(2);
        let snap = sim.snapshot().unwrap();
        let positions: Vec<usize> = if snap.payload.len() <= 200 {
            (0..snap.payload.len()).collect()
        } else {
            (0..200).map(|i| i * snap.payload.len() / 200).collect()
        };
        for &i in &positions {
            for flip in [0x01u8, 0xFF] {
                let mut bad = snap.clone();
                bad.payload[i] ^= flip;
                let mut target = scenario.build_dyn(&library).unwrap();
                if target.restore(&bad).is_ok() {
                    target.advance(1);
                    target.reset();
                }
            }
        }
        let mut truncated = snap.clone();
        truncated.payload.truncate(snap.payload.len() / 2);
        assert!(sim.restore(&truncated).is_err(), "{id}: truncated snapshot");
    }
}
