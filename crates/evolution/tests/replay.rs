//! Deterministic replay: equal seeds and commands give bit-identical runs
//! (despite parallel decision phases), and a restored snapshot continues
//! exactly like the original run.

use evolution::scenarios::find_builtin;
use evolution::{EvolutionCommand, EvolutionSim, Shape, Species, TraitValue};
use simulation_core::{DynSimulation, Simulation, erase};

fn sim(id: &str) -> EvolutionSim {
    find_builtin(id).expect("built-in scenario").build().expect("valid scenario")
}

fn interventions(sim: &mut EvolutionSim, tick: u64) {
    match tick {
        40 => sim
            .apply(EvolutionCommand::Spawn {
                species: Species::Prey,
                count: 15,
                area: Some(Shape::Disc { x: 20.0, y: 20.0, r: 6.0 }),
                traits: vec![TraitValue::new("speed", 1.4)],
                spread: 0.05,
            })
            .expect("valid spawn"),
        60 => sim
            .apply(EvolutionCommand::Cull { fraction: 0.2, species: Some(Species::Prey), area: None })
            .expect("valid cull"),
        80 => sim.apply(EvolutionCommand::ChangeTemperature { delta: 2.0 }).expect("valid change"),
        _ => {}
    }
}

#[test]
fn equal_seeds_and_commands_replay_bit_for_bit() {
    for id in ["arms_race", "island_evolution"] {
        let mut a = sim(id);
        let mut b = sim(id);
        for t in 0..150 {
            interventions(&mut a, t);
            interventions(&mut b, t);
            a.step();
            b.step();
        }
        assert_eq!(a.snapshot().unwrap(), b.snapshot().unwrap(), "{id}: identical state");
        assert_eq!(a.metrics(), b.metrics());
    }
    let mut other = find_builtin("arms_race").unwrap();
    other.seed += 1;
    let mut c = other.build().unwrap();
    let mut a = sim("arms_race");
    c.advance(30);
    a.advance(30);
    assert_ne!(a.snapshot().unwrap(), c.snapshot().unwrap(), "different seeds diverge");
}

#[test]
fn restored_snapshots_continue_identically_including_the_rng() {
    let mut original = sim("speed_selection");
    for t in 0..100 {
        interventions(&mut original, t);
        original.step();
    }
    let snap = original.snapshot().unwrap();
    for t in 100..220 {
        interventions(&mut original, t);
        original.step();
    }
    let expected = original.snapshot().unwrap();

    // Restore into a sim built from a different scenario: everything,
    // including the configuration and the generator, comes from the snapshot.
    let mut restored: Box<dyn DynSimulation> = erase(sim("food_scarcity"));
    restored.restore(&snap).unwrap();
    assert_eq!(restored.tick(), 100);
    restored.advance(120);
    assert_eq!(restored.snapshot().unwrap(), expected);

    // Snapshots survive serialisation to bytes.
    let bytes = snap.to_bytes().unwrap();
    let mut again = sim("speed_selection");
    again.restore(&simulation_core::Snapshot::from_bytes(&bytes).unwrap()).unwrap();
    again.advance(120);
    assert_eq!(again.snapshot().unwrap(), expected);
}

#[test]
fn reset_restores_the_initial_state() {
    let mut s = sim("r_vs_k");
    let initial = s.snapshot().unwrap();
    s.advance(50);
    s.apply(EvolutionCommand::ToggleBarrier { index: 0 }).unwrap_err();
    s.reset();
    assert_eq!(s.snapshot().unwrap(), initial);
}
