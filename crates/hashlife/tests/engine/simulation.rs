//! The `Simulation` wrapper: parameters, commands, metrics, views, snapshots.

use hashlife::{HashlifeCommand, HashlifeSim, HashlifeViewRequest, MODEL_ID, macrocell};
use simulation_core::{ApplyPolicy, DynSimulation, ParamValue, SimError, Simulation, erase};

use crate::common::{GLIDER, GOSPER_GUN, LIFE, cells_of, naive_run};

#[test]
fn descriptor_declares_params_metrics_and_rules() {
    let sim = HashlifeSim::new(1);
    let descriptor = sim.descriptor();
    assert_eq!(descriptor.id, MODEL_ID);
    assert_eq!(descriptor.param("rule").unwrap().apply, ApplyPolicy::NextTick);
    assert_eq!(descriptor.param("step_exponent").unwrap().apply, ApplyPolicy::Immediate);
    assert_eq!(descriptor.param("node_limit").unwrap().apply, ApplyPolicy::Immediate);
    assert!(
        descriptor.rules.contains("B3/S23") && descriptor.rules.contains("{3}") && descriptor.rules.contains("{2, 3}")
    );
    let metric_keys: Vec<&str> = descriptor.metrics.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(metric_keys, ["population", "generation", "node_count", "memory_bytes", "root_level"]);
    let metrics = sim.metrics();
    for key in metric_keys {
        assert!(metrics.get(key).is_some(), "metric {key} is reported");
    }
}

#[test]
fn step_exponent_sets_generations_per_tick() {
    let mut sim = HashlifeSim::new(0);
    sim.apply(HashlifeCommand::LoadCells { cells: GLIDER.to_vec(), replace: true }).unwrap();
    assert_eq!(sim.set_param("step_exponent", ParamValue::Int(10)), Ok(ApplyPolicy::Immediate));
    sim.step();
    assert_eq!(sim.tick(), 1024);
    sim.step();
    assert_eq!(sim.tick(), 2048);
    assert_eq!(sim.universe().bounding_box(), Some((512, 512, 514, 514)));
    assert_eq!(Simulation::advance(&mut sim, 5), 5);
    assert_eq!(sim.metrics().get("generation"), Some(2053.0));
    assert!(sim.set_param("step_exponent", ParamValue::Int(62)).is_err());
    assert!(matches!(sim.set_param("nope", ParamValue::Int(1)), Err(SimError::UnknownParameter(_))));
}

#[test]
fn rule_parameter_is_validated_and_applied() {
    let mut sim = HashlifeSim::new(0);
    let cells = vec![(0, 0), (1, 0), (2, 0), (0, 1), (2, 2), (5, 5), (5, 6), (6, 5)];
    sim.apply(HashlifeCommand::LoadCells { cells: cells.clone(), replace: true }).unwrap();
    assert_eq!(sim.set_param("rule", ParamValue::Text("b36/s23".into())), Ok(ApplyPolicy::NextTick));
    assert_eq!(sim.params().str_or("rule", ""), "B36/S23");
    assert!(sim.set_param("rule", ParamValue::Text("B0/S8".into())).is_err());
    assert!(sim.set_param("rule", ParamValue::Text("B3/S23:T10,10".into())).is_err());
    assert_eq!(sim.params().str_or("rule", ""), "B36/S23", "rejected rules leave the rule unchanged");
    Simulation::advance(&mut sim, 9);
    let start = cells.into_iter().collect();
    assert_eq!(cells_of(sim.universe()), naive_run(&start, (1 << 3) | (1 << 6), LIFE.1, 9));
}

#[test]
fn commands_edit_the_pattern() {
    let mut sim = HashlifeSim::new(3);
    sim.apply(HashlifeCommand::SetCell { x: 4, y: -2, alive: true }).unwrap();
    assert!(sim.universe().get_cell(4, -2));
    sim.apply(HashlifeCommand::SetCell { x: 4, y: -2, alive: false }).unwrap();
    assert!(sim.universe().is_empty());
    sim.apply(HashlifeCommand::RandomSoup { x: -10, y: -10, width: 20, height: 20, density: 0.5 }).unwrap();
    let soup = cells_of(sim.universe());
    assert!(soup.len() > 100 && soup.len() < 300);
    assert!(soup.iter().all(|&(x, y)| (-10..10).contains(&x) && (-10..10).contains(&y)));
    // Same seed, same soup; a soup replaces what was in its rectangle.
    let mut other = HashlifeSim::new(3);
    other.apply(HashlifeCommand::LoadCells { cells: vec![(0, 0), (50, 50)], replace: false }).unwrap();
    other.apply(HashlifeCommand::RandomSoup { x: -10, y: -10, width: 20, height: 20, density: 0.5 }).unwrap();
    let mut expected = soup.clone();
    expected.insert((50, 50));
    assert_eq!(cells_of(other.universe()), expected);
    sim.apply(HashlifeCommand::ClearRect { x0: -10, y0: -10, x1: -1, y1: 9 }).unwrap();
    assert!(cells_of(sim.universe()).iter().all(|&(x, _)| x >= 0));
    sim.apply(HashlifeCommand::Clear).unwrap();
    assert!(sim.universe().is_empty());
    for bad in [
        HashlifeCommand::RandomSoup { x: 0, y: 0, width: 0, height: 5, density: 0.5 },
        HashlifeCommand::RandomSoup { x: 0, y: 0, width: 5, height: 5, density: 1.5 },
        HashlifeCommand::RandomSoup { x: 0, y: 0, width: 5, height: 5, density: f64::NAN },
        HashlifeCommand::RandomSoup { x: i64::MAX, y: 0, width: 5, height: 5, density: 0.5 },
        HashlifeCommand::RandomSoup { x: 0, y: 0, width: 1 << 13, height: 1 << 13, density: 0.5 },
        HashlifeCommand::ClearRect { x0: 1, y0: 0, x1: 0, y1: 0 },
    ] {
        assert!(matches!(sim.apply(bad.clone()), Err(SimError::InvalidCommand(_))), "{bad:?}");
    }
    let error = sim.apply(HashlifeCommand::LoadMacrocell("hello".into())).unwrap_err();
    assert!(matches!(error, SimError::Parse(_)));
}

#[test]
fn snapshot_restore_replays_identically_including_the_rng() {
    let mut sim = HashlifeSim::new(42);
    sim.apply(HashlifeCommand::RandomSoup { x: 0, y: 0, width: 64, height: 64, density: 0.4 }).unwrap();
    sim.set_param("step_exponent", ParamValue::Int(3)).unwrap();
    for _ in 0..5 {
        sim.step();
    }
    let snapshot = sim.snapshot().unwrap();
    assert_eq!(snapshot.model_id, MODEL_ID);
    let continue_run = |sim: &mut HashlifeSim| {
        sim.apply(HashlifeCommand::RandomSoup { x: 200, y: 0, width: 16, height: 16, density: 0.5 }).unwrap();
        for _ in 0..7 {
            sim.step();
        }
        (sim.tick(), cells_of(sim.universe()))
    };
    let first = continue_run(&mut sim);
    let mut restored = HashlifeSim::new(0);
    restored.restore(&snapshot).unwrap();
    assert_eq!(restored.tick(), 40);
    assert_eq!(restored.step_exponent(), 3);
    assert_eq!(continue_run(&mut restored), first);
    // Snapshots of another model are rejected.
    let mut foreign = snapshot.clone();
    foreign.model_id = "life.conway".into();
    assert!(matches!(restored.restore(&foreign), Err(SimError::WrongModel { .. })));
}

#[test]
fn reset_returns_to_the_pattern_the_run_started_from() {
    let mut sim = HashlifeSim::new(1);
    sim.apply(HashlifeCommand::LoadCells { cells: GOSPER_GUN.to_vec(), replace: true }).unwrap();
    let start = cells_of(sim.universe());
    Simulation::advance(&mut sim, 300);
    sim.apply(HashlifeCommand::SetCell { x: -50, y: -50, alive: true }).unwrap();
    Simulation::advance(&mut sim, 10);
    sim.reset();
    // The second run started from the edited generation-300 state.
    assert_eq!(sim.tick(), 300);
    sim.apply(HashlifeCommand::LoadCells { cells: GOSPER_GUN.to_vec(), replace: true }).unwrap();
    sim.reset();
    // No run since the last edit: reset goes back to the previous start point.
    assert_eq!(sim.tick(), 300);
    let mut fresh = HashlifeSim::new(1);
    fresh.apply(HashlifeCommand::LoadCells { cells: GOSPER_GUN.to_vec(), replace: true }).unwrap();
    fresh.step();
    fresh.reset();
    assert_eq!(fresh.tick(), 0);
    assert_eq!(cells_of(fresh.universe()), start);
}

#[test]
fn overflow_halts_the_model_until_the_next_command() {
    let mut sim = HashlifeSim::new(1);
    let text = macrocell::write(&{
        let mut u = hashlife::Universe::conway();
        u.insert_cells(GLIDER);
        u.set_generation(u64::MAX - 3);
        u
    });
    sim.apply(HashlifeCommand::LoadMacrocell(text)).unwrap();
    sim.set_param("step_exponent", ParamValue::Int(2)).unwrap();
    sim.step();
    assert!(sim.is_finished());
    assert!(sim.halt_reason().unwrap().contains("exceed"));
    assert_eq!(sim.tick(), u64::MAX - 3, "a failed step changes nothing");
    sim.step();
    assert_eq!(sim.tick(), u64::MAX - 3);
    sim.apply(HashlifeCommand::Clear).unwrap();
    assert!(!sim.is_finished());
}

#[test]
fn view_is_a_clamped_density_grid() {
    let mut sim = HashlifeSim::new(1);
    sim.apply(HashlifeCommand::LoadCells { cells: GOSPER_GUN.to_vec(), replace: true }).unwrap();
    let request = HashlifeViewRequest { x0: 0, y0: 0, width_px: 10, height_px: 3, log2_scale: 2 };
    let view = sim.view(&request);
    assert_eq!(view.populations, sim.universe().density_grid(0, 0, 10, 3, 2).unwrap());
    assert_eq!(view.populations.iter().sum::<u32>(), 36);
    let huge = sim.view(&HashlifeViewRequest { x0: 0, y0: 0, width_px: 100_000, height_px: 0, log2_scale: 99 });
    assert_eq!((huge.width_px, huge.height_px, huge.log2_scale), (4096, 1, 64));
    assert_eq!(huge.populations.len(), 4096);
    assert_eq!(sim.view(&HashlifeViewRequest::default()).populations.len(), 128 * 128);
}

#[test]
fn works_behind_the_object_safe_interface() {
    let mut sim: Box<dyn DynSimulation> = erase(HashlifeSim::new(9));
    sim.set_param("step_exponent", ParamValue::Int(4)).unwrap();
    sim.step();
    assert_eq!(sim.tick(), 16);
    let snapshot = sim.snapshot().unwrap();
    sim.advance(100);
    sim.restore(&snapshot).unwrap();
    assert_eq!(sim.tick(), 16);
    assert!(sim.memory_bytes() > 0);
}
