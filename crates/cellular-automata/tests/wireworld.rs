//! Wireworld rule and the circuits of `assets/patterns/wireworld/`.

use cellular_automata::wireworld::{HEAD, TAIL, WIRE};
use cellular_automata::{PaintCommand, PatternLibrary, WireworldConfig, WireworldSim};
use patterns::Pattern;
use simulation_core::{Simulation, Topology};

/// Embedded circuits plus the asset directory, which holds the separately
/// licensed (GPL) McIntosh AND gate that is not compiled in.
fn library() -> PatternLibrary {
    let catalog = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/patterns/catalog.toml");
    PatternLibrary::load(&catalog).unwrap_or_else(|e| panic!("asset catalog at {}: {e}", catalog.display()))
}

fn circuit(id: &str) -> Pattern {
    library().resolve(id).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn gpl_gate_is_read_from_the_assets_not_embedded() {
    let err = PatternLibrary::builtin().resolve("wireworld/and-gate-mcintosh").unwrap_err().to_string();
    assert!(err.contains("asset catalog"), "{err}");
    assert!(circuit("wireworld/and-gate-mcintosh").cells.iter().all(|c| (1..=3).contains(&c.state)));
    // Ids are plain relative paths: nothing outside the asset directory.
    assert!(library().resolve("../wireworld/diode").is_err());
    assert!(library().resolve("wireworld/../../Cargo").is_err());
}

fn world(pattern: &Pattern) -> WireworldSim {
    let config =
        WireworldConfig { topology: Topology::Bounded, width: 48, height: 16, track_activity: false, init: vec![] };
    let mut sim = WireworldSim::new(&config, 1, &PatternLibrary::builtin()).unwrap();
    sim.apply(PaintCommand::LoadPattern { pattern: pattern.clone(), x: 0, y: 0 }).unwrap();
    sim
}

/// Generations (1..=limit) at which `probe` holds an electron head.
fn pulses(sim: &mut WireworldSim, probe: (i64, i64), limit: u64) -> Vec<u64> {
    let mut out = Vec::new();
    for g in 1..=limit {
        sim.step();
        if sim.cell(probe.0, probe.1) == HEAD {
            out.push(g);
        }
    }
    out
}

/// Replaces the electron (head and tail) at the given cells by plain wire.
fn remove_electron(sim: &mut WireworldSim, cells: &[(i64, i64)]) {
    for &(x, y) in cells {
        sim.apply(PaintCommand::SetCell { x, y, state: WIRE }).unwrap();
    }
}

#[test]
fn electrons_travel_one_cell_per_generation() {
    let mut s = world(&Pattern::from_ascii("12333333333").unwrap());
    // from_ascii digits are states: 1 head, 2 tail, 3 wire; the electron moves left.
    let wire: Vec<_> = (0..11).map(|x| (x, 0)).collect();
    remove_electron(&mut s, &wire);
    s.apply(PaintCommand::SetCell { x: 1, y: 0, state: HEAD }).unwrap();
    s.apply(PaintCommand::SetCell { x: 0, y: 0, state: TAIL }).unwrap();
    for g in 1..=8 {
        s.step();
        assert_eq!(s.cell(1 + g, 0), HEAD, "head at x = {} after {g} generations", 1 + g);
        assert_eq!(s.cell(g, 0), TAIL);
    }
}

#[test]
fn diode_passes_left_to_right_only() {
    let diode = circuit("wireworld/diode");
    let mut forward = world(&diode);
    assert_eq!(pulses(&mut forward, (10, 1), 40), vec![9], "one electron arrives at the far end");
    let mut reverse = world(&diode);
    remove_electron(&mut reverse, &[(0, 1), (1, 1)]);
    reverse.apply(PaintCommand::SetCell { x: 10, y: 1, state: HEAD }).unwrap();
    let mut reached_middle = false;
    for _ in 0..40 {
        reverse.step();
        reached_middle |= reverse.cell(6, 1) == HEAD;
        assert_ne!(reverse.cell(0, 1), HEAD, "nothing passes from right to left");
        assert_ne!(reverse.cell(1, 1), HEAD);
    }
    assert!(reached_middle, "the reverse electron did travel up to the diode");
}

#[test]
fn clocks_have_periods_8_and_12() {
    let mut c8 = world(&circuit("wireworld/clock-8"));
    assert_eq!(pulses(&mut c8, (12, 1), 60), vec![10, 18, 26, 34, 42, 50, 58]);
    let mut c12 = world(&circuit("wireworld/clock-12"));
    assert_eq!(pulses(&mut c12, (10, 2), 60), vec![7, 19, 31, 43, 55]);
}

fn truth_table(id: &str, inputs: [&[(i64, i64)]; 2], output: (i64, i64), limit: u64) -> [[usize; 2]; 2] {
    let gate = circuit(id);
    let mut table = [[0; 2]; 2];
    for (a, row) in table.iter_mut().enumerate() {
        for (b, cell) in row.iter_mut().enumerate() {
            let mut sim = world(&gate);
            if a == 0 {
                remove_electron(&mut sim, inputs[0]);
            }
            if b == 0 {
                remove_electron(&mut sim, inputs[1]);
            }
            *cell = pulses(&mut sim, output, limit).len();
        }
    }
    table
}

#[test]
fn or_xor_and_truth_tables() {
    let or = truth_table("wireworld/or-gate", [&[(0, 0), (1, 0)], &[(0, 4), (1, 4)]], (11, 2), 40);
    assert_eq!(or, [[0, 1], [1, 1]], "OR");
    let xor = truth_table("wireworld/xor-gate", [&[(1, 0), (1, 1)], &[(1, 7), (1, 8)]], (11, 4), 40);
    assert_eq!(xor, [[0, 1], [1, 0]], "XOR");
    let and = truth_table("wireworld/and-gate-mcintosh", [&[(14, 0)], &[(10, 2)]], (33, 7), 80);
    assert_eq!(and, [[0, 0], [0, 1]], "AND");
}

#[test]
fn or_gate_does_not_leak_into_the_other_input() {
    let gate = circuit("wireworld/or-gate");
    let mut sim = world(&gate);
    remove_electron(&mut sim, &[(0, 4), (1, 4)]);
    for _ in 0..40 {
        sim.step();
        assert_ne!(sim.cell(0, 4), HEAD);
    }
}

#[test]
fn model_contract() {
    let mut s = world(&circuit("wireworld/clock-8"));
    let d = s.descriptor();
    assert_eq!(d.id, "ca.wireworld");
    assert!(d.rules.contains("exactly 1 or 2"));
    assert!(s.apply(PaintCommand::SetCell { x: 0, y: 0, state: 4 }).is_err(), "only states 0..=3");
    s.advance(5);
    let m = s.metrics();
    assert_eq!(
        m.get("heads").unwrap() + m.get("tails").unwrap() + m.get("conductors").unwrap(),
        16.0,
        "electrons move, cells are conserved"
    );
    let snap = s.snapshot().unwrap();
    s.advance(9);
    let expected = s.cells();
    let mut other = world(&circuit("wireworld/diode"));
    other.restore(&snap).unwrap();
    other.advance(9);
    assert_eq!(other.cells(), expected);
    s.reset();
    assert_eq!(s.tick(), 0);
    assert_eq!(s.cells(), world(&circuit("wireworld/clock-8")).cells(), "reset returns to the loaded circuit");
}
