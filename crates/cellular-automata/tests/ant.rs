//! Langton's ant and turmites: OEIS A255938 (black cells after n moves),
//! the highway onset at step 9977 with period 104, LLRR symmetry, and the
//! model contract.

use std::collections::BTreeMap;

use cellular_automata::{Ant, AntCommand, AntConfig, CellRect, GridViewRequest, Heading, LangtonsAntSim, TurmiteRule};
use simulation_core::{ParamValue, Simulation, Topology};

fn langton() -> LangtonsAntSim {
    LangtonsAntSim::langton(1)
}

#[test]
fn black_cells_follow_oeis_a255938() {
    // a(0..=10) and sampled terms of OEIS A255938 (b-file by A. Zabolotskii).
    let mut s = langton();
    let mut first = vec![s.colored()];
    for _ in 0..10 {
        s.step();
        first.push(s.colored());
    }
    assert_eq!(first, [0, 1, 2, 3, 4, 3, 4, 5, 6, 7, 6]);
    let samples =
        [(9976u64, 716u64), (9977, 715), (9978, 714), (10000, 720), (11000, 834), (12000, 952), (15000, 1298)];
    let mut s = langton();
    for (n, expected) in samples {
        s.advance(n - s.steps());
        assert_eq!(s.colored(), expected, "a({n})");
    }
}

#[test]
fn highway_starts_at_step_9977_with_period_104() {
    let mut s = langton();
    let total = 9977 + 104 * 30;
    let mut read = Vec::with_capacity(total);
    let mut positions = Vec::with_capacity(total);
    for _ in 0..total {
        let ant = s.ants()[0];
        read.push(s.color(ant.x, ant.y));
        positions.push((ant.x, ant.y));
        s.step();
    }
    let periodic_from = |t0: usize| (t0..total - 104).all(|t| read[t] == read[t + 104]);
    assert!(periodic_from(9977), "the colours read repeat with period 104 from step 9977");
    assert!(!periodic_from(9976), "but not from step 9976");
    for p in 1..104 {
        if 104 % p == 0 {
            assert!(!(9977..total - p).all(|t| read[t] == read[t + p]), "104 is the minimal period (not {p})");
        }
    }
    let (dx, dy) = (positions[9977 + 104].0 - positions[9977].0, positions[9977 + 104].1 - positions[9977].1);
    assert_eq!((dx.abs(), dy.abs()), (2, 2), "the highway moves diagonally by 2 cells per period");
    for t in (9977..total - 104).step_by(37) {
        assert_eq!((positions[t + 104].0 - positions[t].0, positions[t + 104].1 - positions[t].1), (dx, dy));
    }
    let mut a = langton();
    a.advance(12_000);
    let before = a.colored();
    a.advance(104);
    assert_eq!(a.colored(), before + 12, "a(n + 104) = a(n) + 12 on the highway");
}

/// Whether the coloured cells are mirror-symmetric about a vertical or
/// horizontal axis through the centre of their bounding box.
fn mirror_symmetric(cells: &BTreeMap<(i64, i64), u8>) -> bool {
    if cells.is_empty() {
        return true;
    }
    let (x0, x1) = (cells.keys().map(|c| c.0).min().unwrap(), cells.keys().map(|c| c.0).max().unwrap());
    let (y0, y1) = (cells.keys().map(|c| c.1).min().unwrap(), cells.keys().map(|c| c.1).max().unwrap());
    let vertical = cells.iter().all(|(&(x, y), c)| cells.get(&(x0 + x1 - x, y)) == Some(c));
    let horizontal = cells.iter().all(|(&(x, y), c)| cells.get(&(x, y0 + y1 - y)) == Some(c));
    vertical || horizontal
}

#[test]
fn llrr_keeps_returning_to_mirror_symmetry() {
    let config = AntConfig {
        rule: "LLRR".into(),
        ants: vec![Ant { x: 0, y: 0, heading: Heading::North }],
        ..AntConfig::default()
    };
    let mut s = LangtonsAntSim::new(&config, 1).unwrap();
    let mut cells: BTreeMap<(i64, i64), u8> = BTreeMap::new();
    let mut symmetric_steps = Vec::new();
    for step in 1..=20_000u64 {
        let ant = s.ants()[0];
        s.step();
        let c = s.color(ant.x, ant.y);
        if c == 0 {
            cells.remove(&(ant.x, ant.y));
        } else {
            cells.insert((ant.x, ant.y), c);
        }
        if mirror_symmetric(&cells) {
            symmetric_steps.push(step);
        }
    }
    assert!(symmetric_steps.iter().any(|&s| s > 19_000), "still symmetric after step 19,000");
    assert!(symmetric_steps.contains(&19_812), "symmetric at step 19,812 (docs/research/automata-catalog.md §5.2)");
    assert_eq!(symmetric_steps.len(), 722, "symmetric at 722 of the first 20,000 steps");
}

#[test]
fn langtons_ant_loses_symmetry_for_good_after_step_52() {
    let mut s = langton();
    let mut cells: BTreeMap<(i64, i64), u8> = BTreeMap::new();
    let mut last = 0;
    for step in 1..=2_000u64 {
        let ant = s.ants()[0];
        s.step();
        if s.color(ant.x, ant.y) == 0 {
            cells.remove(&(ant.x, ant.y));
        } else {
            cells.insert((ant.x, ant.y), 1);
        }
        if mirror_symmetric(&cells) {
            last = step;
        }
    }
    assert_eq!(last, 52);
}

#[test]
fn rules_ants_and_worlds() {
    assert_eq!(TurmiteRule::parse("rlnu").unwrap().to_string(), "RLNU");
    assert!(TurmiteRule::parse("R").is_err());
    assert!(TurmiteRule::parse("RX").unwrap_err().to_string().contains("character 2"));
    // Two ants on one torus move one after the other.
    let config = AntConfig {
        rule: "RL".into(),
        topology: Topology::Torus,
        width: 8,
        height: 8,
        steps_per_tick: 1,
        track_activity: true,
        ants: vec![Ant { x: 0, y: 0, heading: Heading::North }, Ant { x: 0, y: 0, heading: Heading::South }],
    };
    let mut s = LangtonsAntSim::new(&config, 1).unwrap();
    s.step();
    // Ant 1 turns right (east) and flips (0,0) to black; ant 2 then reads black, turns left (east) and flips it back.
    assert_eq!(s.color(0, 0), 0);
    assert_eq!(s.ants(), [Ant { x: 1, y: 0, heading: Heading::East }, Ant { x: 1, y: 0, heading: Heading::East }]);
    // On a bounded plane an ant walking off the world is removed.
    let bounded = AntConfig {
        topology: Topology::Bounded,
        width: 3,
        height: 3,
        ants: vec![Ant { x: 2, y: 0, heading: Heading::North }],
        ..config
    };
    let mut b = LangtonsAntSim::new(&bounded, 1).unwrap();
    b.step();
    b.step();
    assert!(b.ants().is_empty() && b.is_finished());
}

#[test]
fn model_contract() {
    let mut s = langton();
    assert_eq!(s.descriptor().id, "ca.langtons_ant");
    s.set_param("steps_per_tick", ParamValue::Int(100)).unwrap();
    s.step();
    assert_eq!(s.tick(), 100);
    s.apply(AntCommand::AddAnt(Ant { x: 50, y: 50, heading: Heading::West })).unwrap();
    s.apply(AntCommand::Randomize { density: 0.2, color: 1, seed: None, region: Some(CellRect::new(30, 30, 40, 40)) })
        .unwrap();
    assert!(s.apply(AntCommand::SetCell { x: 0, y: 0, color: 2 }).is_err(), "RL has colours 0 and 1");
    let snap = s.snapshot().unwrap();
    s.advance(7);
    let expected = (s.ants().to_vec(), s.colored());
    let mut other = LangtonsAntSim::langton(9);
    other.restore(&snap).unwrap();
    other.advance(7);
    assert_eq!((other.ants().to_vec(), other.colored()), expected);
    let view = s.view(&GridViewRequest { x0: -40, y0: -40, x1: 60, y1: 60, max_width: 128, max_height: 128 });
    assert_eq!(view.ants.len(), 2);
    let a = view.ants[0];
    assert_eq!(view.grid.texel_at_cell(a.x, a.y).map(|t| t[0]), Some(s.color(a.x, a.y)));
    s.set_param("rule", ParamValue::Text("LLRR".into())).unwrap();
    s.reset();
    assert_eq!((s.tick(), s.colored(), s.rule().to_string().as_str()), (0, 0, "LLRR"));
}

#[test]
fn snapshots_keep_the_running_rule_while_a_rule_change_waits_for_reset() {
    let mut s = langton();
    s.advance(500);
    s.set_param("rule", ParamValue::Text("LLRR".into())).unwrap();
    assert_eq!(s.rule().to_string(), "RL", "the new rule waits for the reset");
    let snap = s.snapshot().unwrap();
    s.advance(300);
    let expected = (s.ants().to_vec(), s.colored());
    let mut other = LangtonsAntSim::langton(5);
    other.restore(&snap).unwrap();
    assert_eq!(other.rule().to_string(), "RL", "the running rule is restored");
    assert_eq!(other.params().str_or("rule", ""), "LLRR", "and the pending one");
    other.advance(300);
    assert_eq!((other.ants().to_vec(), other.colored()), expected);
    other.reset();
    assert_eq!((other.tick(), other.rule().to_string().as_str()), (0, "LLRR"));
}

#[test]
fn the_trail_shows_every_flip_of_the_last_tick_and_fades_per_tick() {
    let mut s = langton();
    s.set_param("steps_per_tick", ParamValue::Int(20)).unwrap();
    s.step();
    let req = GridViewRequest { x0: -20, y0: -20, x1: 20, y1: 20, max_width: 64, max_height: 64 };
    let view = s.view(&req);
    // Every cell the ant stood on during the first 20 steps was flipped during this tick.
    let mut first = LangtonsAntSim::langton(1);
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..20 {
        let a = first.ants()[0];
        visited.insert((a.x, a.y));
        first.step();
    }
    for &(x, y) in &visited {
        assert_eq!(view.grid.texel_at_cell(x, y).map(|t| t[2]), Some(255), "({x}, {y})");
    }
    let lit = (-20..=20)
        .flat_map(|y| (-20..=20).map(move |x| (x, y)))
        .filter(|&(x, y)| view.grid.texel_at_cell(x, y).is_some_and(|t| t[2] > 0));
    assert_eq!(lit.count(), visited.len());
    s.set_param("steps_per_tick", ParamValue::Int(1)).unwrap();
    s.step();
    let later = s.view(&req);
    let (x, y) = *visited.iter().next().unwrap();
    assert!(later.grid.texel_at_cell(x, y).is_some_and(|t| t[2] == 255 || t[2] == 255 - 16));
}
