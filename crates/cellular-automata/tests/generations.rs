//! Generations semantics (Golly's `generationsalgo`): Brian's Brain, Star
//! Wars and the model contract.

use cellular_automata::{
    GenerationsConfig, GenerationsSim, GridViewRequest, PaintCommand, PatternLibrary, StateInit, Symmetry,
};
use simulation_core::{ApplyPolicy, ParamValue, Simulation, Topology};

fn brain(width: u32, init: Vec<StateInit>) -> GenerationsSim {
    let config = GenerationsConfig {
        rule: "/2/3".into(),
        topology: Topology::Torus,
        width,
        height: width,
        track_activity: false,
        init,
    };
    GenerationsSim::new(&config, 3, &PatternLibrary::builtin()).expect("valid")
}

fn load(sim: &mut GenerationsSim, rle: &str, x: i64, y: i64) {
    sim.apply(PaintCommand::ImportText { text: rle.into(), file_name: None, x, y, replace: true }).unwrap();
}

#[test]
fn default_model_is_brians_brain_on_a_torus() {
    let s = GenerationsSim::brians_brain(1);
    assert_eq!(s.rule().to_string(), "/2/3");
    assert_eq!((s.world().width, s.world().topology), (256, Topology::Torus));
    assert!(!s.cells().is_empty() && s.cells().iter().all(|c| c.2 == 1), "a soup of firing cells");
}

#[test]
fn brians_brain_fires_once_then_rests() {
    let mut s = brain(64, vec![StateInit::Random { region: None, density: 0.3, state: 1, symmetry: Symmetry::None }]);
    for _ in 0..30 {
        let before = s.cells();
        s.step();
        for (x, y, state) in before {
            let next = s.cell(x, y);
            match state {
                1 => assert_eq!(next, 2, "a firing cell becomes refractory"),
                2 => assert_eq!(next, 0, "a refractory cell becomes ready"),
                other => panic!("Brian's Brain has no state {other}"),
            }
        }
    }
}

#[test]
fn brians_brain_has_no_surviving_cells() {
    let mut s = brain(64, vec![StateInit::Random { region: None, density: 0.25, state: 1, symmetry: Symmetry::None }]);
    for _ in 0..40 {
        let firing: Vec<(i64, i64)> = s.cells().into_iter().filter(|c| c.2 == 1).map(|c| (c.0, c.1)).collect();
        s.step();
        assert!(firing.iter().all(|&(x, y)| s.cell(x, y) != 1), "a firing cell never fires again next generation");
        let m = s.metrics();
        assert_eq!(m.get("deaths"), Some(firing.len() as f64), "every firing cell stops firing");
    }
}

#[test]
fn domino_spaceship_and_block_explosion() {
    let mut s = brain(64, vec![]);
    load(&mut s, "x = 2, y = 2, rule = /2/3\nBA$BA!", 10, 10);
    let start = s.cells();
    for g in 1..=8 {
        s.step();
        let moved: Vec<(i64, i64, u8)> = start.iter().map(|&(x, y, st)| (x + g, y, st)).collect();
        assert_eq!(s.cells(), moved, "the domino moves one cell right per generation");
    }
    let mut b = brain(64, vec![]);
    load(&mut b, "x = 2, y = 2, rule = /2/3\n2A$2A!", 30, 30);
    let mut pops = Vec::new();
    for _ in 0..3 {
        b.step();
        pops.push(b.cells().len());
    }
    assert_eq!(pops, [12, 20, 28], "a 2×2 firing block explodes");
}

#[test]
fn decay_chain_of_a_many_state_rule() {
    // Star Wars: S345/B2/C4 — a lone firing cell has no neighbours, so it dies through states 2 and 3.
    let config = GenerationsConfig {
        rule: "Star Wars".into(),
        topology: Topology::Bounded,
        width: 8,
        height: 8,
        track_activity: true,
        init: vec![],
    };
    let mut s = GenerationsSim::new(&config, 1, &PatternLibrary::builtin()).unwrap();
    assert_eq!(s.rule().to_string(), "345/2/4");
    s.apply(PaintCommand::SetCell { x: 4, y: 4, state: 1 }).unwrap();
    let mut states = Vec::new();
    for _ in 0..4 {
        s.step();
        states.push(s.cell(4, 4));
    }
    assert_eq!(states, [2, 3, 0, 0]);
    assert_eq!(s.activity(4, 4), 255 - 16, "last change (3 -> 0) one generation before the last");
    assert!(s.apply(PaintCommand::SetCell { x: 0, y: 0, state: 4 }).is_err(), "Star Wars has states 0..=3");
}

#[test]
fn parameters_snapshot_and_view() {
    let mut s = brain(32, vec![StateInit::Random { region: None, density: 0.3, state: 1, symmetry: Symmetry::None }]);
    let d = s.descriptor();
    assert_eq!(d.id, "ca.generations");
    assert!(d.rules.contains("B2/S/C3") && d.rules.contains("Only cells in state 1 count"));
    assert_eq!(s.set_param("preset", ParamValue::Text("star_wars".into())).unwrap(), ApplyPolicy::NextTick);
    assert_eq!(s.params().str_or("rule", ""), "345/2/4");
    s.step();
    assert_eq!(s.rule().to_string(), "345/2/4");
    assert!(s.set_param("rule", ParamValue::Text("B0/S/C3".into())).is_err(), "B0 is rejected");
    assert!(s.set_param("topology", ParamValue::Text("infinite".into())).is_err());
    s.set_param("rule", ParamValue::Text("B2/S/C3".into())).unwrap();
    s.advance(5);
    let snap = s.snapshot().unwrap();
    s.advance(7);
    let expected = s.cells();
    let mut other = brain(8, vec![]);
    other.restore(&snap).unwrap();
    other.advance(7);
    assert_eq!(other.cells(), expected);
    let view = s.view(&GridViewRequest { x0: 0, y0: 0, x1: 31, y1: 31, max_width: 32, max_height: 32 });
    for (x, y, state) in s.cells() {
        assert_eq!(view.texel_at_cell(x, y), Some([state, 0, 0, 255]));
    }
    s.reset();
    assert_eq!(s.tick(), 0);
}

#[test]
fn large_worlds_and_world_changes_rebuild_or_are_refused_up_front() {
    // More cells than a random-fill list may hold (2^26): the fill is streamed.
    let config = GenerationsConfig {
        rule: "/2/3".into(),
        topology: Topology::Torus,
        width: 8_192,
        height: 8_200,
        track_activity: false,
        init: vec![StateInit::Random { region: None, density: 0.001, state: 1, symmetry: Symmetry::None }],
    };
    let s = GenerationsSim::new(&config, 1, &PatternLibrary::builtin()).expect("large worlds can be filled");
    let alive = s.metrics().get("alive").unwrap();
    assert!((alive - 0.001 * 8_192.0 * 8_200.0).abs() < 0.05 * 0.001 * 8_192.0 * 8_200.0, "{alive}");
    drop(s);
    // A world on which the scenario's square-symmetric fill is impossible is refused by set_param,
    // so reset never fails silently.
    let mut b =
        brain(32, vec![StateInit::Random { region: None, density: 0.3, state: 1, symmetry: Symmetry::Rotate8 }]);
    let err = b.set_param("height", ParamValue::Int(40)).unwrap_err().to_string();
    assert!(err.contains("square"), "{err}");
    b.set_param("width", ParamValue::Int(48)).unwrap_err();
    b.reset();
    assert!(b.halt_reason().is_none() && !b.is_finished());
}
