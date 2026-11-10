//! `LifeSim` against the `Simulation` contract: parameters and their apply
//! policies, the protected Conway preset, editing commands, engine switching,
//! tracking, views, metrics, reset and snapshots.

mod common;

use cellular_automata::life::{LifeConfig, LifeMode, PatternLibrary, PatternSource, RulePreset};
use cellular_automata::{
    CellRect, EngineKind, GridViewRequest, InitItem, LifeCommand, LifeSim, MirrorAxis, PasteMode, Rotation, Symmetry,
};
use patterns::{LifeRule, Pattern};
use simulation_core::{ApplyPolicy, ParamValue, SimError, Simulation, Topology};

fn sim(topology: Topology, width: u32, height: u32, init: Vec<InitItem>) -> LifeSim {
    let config = LifeConfig { topology, width, height, init, ..LifeConfig::default() };
    LifeSim::new(&config, 7, &PatternLibrary::builtin(), LifeMode::Classic).expect("valid configuration")
}

fn glider_at(x: i64, y: i64) -> InitItem {
    InitItem::Pattern { source: PatternSource::Library("glider".into()), x, y, rotation: Rotation::None, mirror: false }
}

fn text(s: &str) -> ParamValue {
    ParamValue::Text(s.into())
}

#[test]
fn defaults_and_descriptor() {
    let s = LifeSim::conway(1);
    let d = s.descriptor();
    assert_eq!(d.id, "life.classic");
    assert!(d.rules.contains("B3/S23") && d.rules.contains("Conway (B3/S23, unaltered)"));
    for key in [
        "preset",
        "rule",
        "topology",
        "width",
        "height",
        "engine",
        "generations_per_tick",
        "track_ages",
        "track_activity",
    ] {
        let spec = d.param(key).unwrap_or_else(|| panic!("param {key}"));
        assert!(spec.validate(&spec.default).is_ok(), "{key} default is valid");
    }
    assert_eq!(d.param("rule").unwrap().apply, ApplyPolicy::NextTick);
    assert_eq!(d.param("topology").unwrap().apply, ApplyPolicy::RequiresReset);
    assert_eq!(d.param("engine").unwrap().apply, ApplyPolicy::Immediate);
    assert_eq!(d.param("track_ages").unwrap().apply, ApplyPolicy::Immediate);
    assert_eq!(s.engine_kind(), EngineKind::Bit, "256×256 is above the dense threshold");
    assert!((s.population() as f64 / 65536.0 - 0.3).abs() < 0.02, "30 % soup");
    let lab = LifeSim::new(&LifeConfig::default(), 1, &PatternLibrary::builtin(), LifeMode::Lab).unwrap();
    assert_eq!(lab.descriptor().id, "ca.life_like");
}

#[test]
fn conway_preset_cannot_be_altered_by_rule_edits() {
    let mut s = sim(Topology::Torus, 32, 32, vec![]);
    assert_eq!(s.preset(), RulePreset::Conway);
    assert_eq!(s.set_param("rule", text("B36/S23")).unwrap(), ApplyPolicy::NextTick);
    assert_eq!(s.params().str_or("preset", ""), "highlife", "typing a preset's rule selects that preset");
    assert_eq!(s.rule(), LifeRule::conway(), "rule changes wait for the next tick");
    s.step();
    assert_eq!(s.rule(), LifeRule::highlife());
    s.set_param("rule", text("B3/S238")).unwrap();
    s.step();
    assert_eq!(s.preset(), RulePreset::Custom, "an edited rule is a custom rule");
    assert_eq!(RulePreset::Conway.rule(), Some(LifeRule::conway()), "the preset itself is unchanged");
    s.set_param("preset", text("conway")).unwrap();
    s.step();
    assert_eq!((s.rule(), s.preset()), (LifeRule::conway(), RulePreset::Conway));
    assert!(s.set_param("rule", text("B3/S2x")).is_err());
    assert!(matches!(s.set_param("nope", text("x")), Err(SimError::UnknownParameter(_))));
}

#[test]
fn b0_rules_only_on_finite_worlds() {
    let mut finite = sim(Topology::Torus, 8, 8, vec![]);
    finite.set_param("rule", text("B0/S")).unwrap();
    finite.step();
    assert_eq!(finite.population(), 64, "every cell of an empty torus is born");
    finite.step();
    assert_eq!(finite.population(), 0, "and dies (no survival)");
    let mut infinite = sim(Topology::Infinite, 64, 64, vec![glider_at(0, 0)]);
    let err = infinite.set_param("rule", text("B013/S23")).unwrap_err().to_string();
    assert!(err.contains("B0"), "{err}");
    // A B0 rule on a torus forbids switching the world to the infinite plane.
    finite.set_param("rule", text("B0/S8")).unwrap();
    assert!(finite.set_param("topology", text("infinite")).is_err());
}

#[test]
fn world_changes_apply_on_reset() {
    let mut s =
        sim(Topology::Torus, 16, 16, vec![InitItem::Random { region: None, density: 0.5, symmetry: Symmetry::None }]);
    assert_eq!(s.set_param("width", ParamValue::Int(40)).unwrap(), ApplyPolicy::RequiresReset);
    assert_eq!(s.world().width, 16, "not before reset");
    assert_eq!(s.params().i64_or("width", 0), 40, "the pending value is shown");
    s.reset();
    assert_eq!(s.world().width, 40);
    s.set_param("topology", text("infinite")).unwrap();
    s.reset();
    assert_eq!(s.world().topology, Topology::Infinite);
    assert_eq!(s.engine_kind(), EngineKind::Sparse);
    assert!(s.set_param("width", ParamValue::Int(0)).is_err());
}

#[test]
fn switching_engines_keeps_every_cell_and_the_future() {
    let mut a = sim(
        Topology::Infinite,
        64,
        64,
        vec![InitItem::Random { region: None, density: 0.35, symmetry: Symmetry::None }],
    );
    let mut b = sim(
        Topology::Infinite,
        64,
        64,
        vec![InitItem::Random { region: None, density: 0.35, symmetry: Symmetry::None }],
    );
    for (i, engine) in ["hashlife", "dense", "sparse", "hashlife", "auto"].into_iter().enumerate() {
        assert_eq!(a.set_param("engine", text(engine)).unwrap(), ApplyPolicy::Immediate);
        assert_eq!(a.live_cells(), b.live_cells(), "switch {i} to {engine}");
        a.advance(7);
        b.advance(7);
        assert_eq!(a.live_cells(), b.live_cells(), "after switch {i} to {engine}");
    }
    assert!(a.set_param("engine", text("bit")).is_err(), "the bit engine needs a finite world");
    let mut t =
        sim(Topology::Torus, 100, 60, vec![InitItem::Random { region: None, density: 0.4, symmetry: Symmetry::None }]);
    let mut u =
        sim(Topology::Torus, 100, 60, vec![InitItem::Random { region: None, density: 0.4, symmetry: Symmetry::None }]);
    t.set_param("engine", text("bit")).unwrap();
    u.set_param("engine", text("dense")).unwrap();
    t.advance(50);
    u.advance(50);
    assert_eq!(t.live_cells(), u.live_cells());
}

#[test]
fn hashlife_ticks_and_power_of_two_steps() {
    let mut s = sim(Topology::Infinite, 64, 64, vec![glider_at(0, 0)]);
    s.set_param("engine", text("hashlife")).unwrap();
    assert!(s.set_param("generations_per_tick", ParamValue::Int(3)).is_err(), "Hashlife needs a power of two");
    s.set_param("generations_per_tick", ParamValue::Int(1024)).unwrap();
    s.step();
    assert_eq!(s.tick(), 1024, "tick() reports generations");
    assert_eq!(s.live_cells().first(), Some(&(257, 256)), "the glider moved 256 cells diagonally");
    assert_eq!(s.metrics().get("births"), None, "Hashlife does not count births when jumping");
    assert_eq!(s.advance(4), 4096);
    assert!(s.set_param("engine", text("sparse")).is_ok());
    assert!(s.set_param("generations_per_tick", ParamValue::Int(3)).is_ok());
    // Automatic choice: large power-of-two steps select Hashlife.
    s.set_param("engine", text("auto")).unwrap();
    s.set_param("generations_per_tick", ParamValue::Int(64)).unwrap();
    s.step();
    assert_eq!(s.engine_kind(), EngineKind::Hashlife);
}

#[test]
fn the_automatic_choice_does_not_measure_patterns_too_large_to_list() {
    use cellular_automata::engine::AUTO_REGULARITY_MAX_POPULATION;
    // A soup too large to measure keeps Hashlife (measured, it would leave it as irregular).
    let mut soup = sim(Topology::Infinite, 64, 64, vec![]);
    soup.set_param("engine", text("hashlife")).unwrap();
    let region = Some(CellRect::new(0, 0, 2047, 1199));
    soup.apply(LifeCommand::Randomize { density: 0.5, seed: Some(3), region, symmetry: Symmetry::None }).unwrap();
    assert!(soup.population() > AUTO_REGULARITY_MAX_POPULATION);
    soup.set_param("engine", text("auto")).unwrap();
    assert_eq!(soup.engine_kind(), EngineKind::Hashlife);
    // The other engines treat such a pattern as irregular: a full block of
    // identical tiles is measured up to the limit and stays on Sparse above it.
    for (rows, kind) in [(1024, EngineKind::Hashlife), (1025, EngineKind::Sparse)] {
        let mut full = sim(Topology::Infinite, 64, 64, vec![]);
        full.set_param("engine", text("sparse")).unwrap();
        let region = Some(CellRect::new(0, 0, 1023, rows - 1));
        full.apply(LifeCommand::Randomize { density: 1.0, seed: Some(1), region, symmetry: Symmetry::None }).unwrap();
        full.set_param("engine", text("auto")).unwrap();
        assert_eq!(full.engine_kind(), kind, "{} live cells", full.population());
    }
    // A Gosper gun after 2^40 generations has about 1.8e11 live cells: back
    // to one generation per tick, the choice keeps Hashlife without listing them.
    let gun = InitItem::Pattern {
        source: PatternSource::Library("gosper-glider-gun".into()),
        x: 0,
        y: 0,
        rotation: Rotation::None,
        mirror: false,
    };
    let mut s = sim(Topology::Infinite, 64, 64, vec![gun]);
    s.set_param("generations_per_tick", ParamValue::Int(1 << 30)).unwrap();
    s.advance(1 << 10);
    assert_eq!((s.engine_kind(), s.tick()), (EngineKind::Hashlife, 1 << 40));
    s.set_param("generations_per_tick", ParamValue::Int(1)).unwrap();
    s.step();
    assert_eq!((s.engine_kind(), s.tick()), (EngineKind::Hashlife, (1 << 40) + 1));
    assert!(s.population() > 1 << 37, "{}", s.population());
}

#[test]
fn metrics_count_births_and_deaths() {
    let blinker = Pattern::from_ascii("OOO").unwrap();
    let mut s = sim(Topology::Torus, 10, 10, vec![]);
    s.apply(LifeCommand::Paste {
        pattern: blinker,
        x: 3,
        y: 4,
        rotation: Rotation::None,
        mirror: false,
        mode: PasteMode::Or,
    })
    .unwrap();
    s.step();
    let m = s.metrics();
    assert_eq!((m.get("population"), m.get("births"), m.get("deaths")), (Some(3.0), Some(2.0), Some(2.0)));
    assert_eq!(m.get("density"), Some(0.03));
    assert_eq!((m.get("bbox_width"), m.get("bbox_height")), (Some(1.0), Some(3.0)));
    assert_eq!(m.get("engine"), Some(0.0), "dense");
    s.set_param("generations_per_tick", ParamValue::Int(4)).unwrap();
    s.step();
    assert_eq!(s.tick(), 5);
    assert_eq!(s.metrics().get("births"), Some(8.0), "summed over the four generations of the tick");
}

#[test]
fn drawing_commands() {
    let mut s = sim(Topology::Bounded, 40, 40, vec![]);
    s.apply(LifeCommand::SetCell { x: 1, y: 1, alive: true }).unwrap();
    s.apply(LifeCommand::SetCell { x: -1, y: 1, alive: true }).unwrap();
    assert_eq!(s.population(), 1, "outside a bounded world nothing is set");
    s.apply(LifeCommand::Line { from: (0, 10), to: (9, 10), alive: true }).unwrap();
    assert_eq!(s.population(), 11);
    s.apply(LifeCommand::Rectangle { rect: CellRect::new(20, 20, 24, 23), filled: false, alive: true }).unwrap();
    assert_eq!(s.population(), 11 + 14);
    s.apply(LifeCommand::Circle { centre: (30, 5), radius: 2, filled: true, alive: true }).unwrap();
    assert!(s.cell(30, 5) && s.cell(32, 5) && !s.cell(32, 7));
    s.apply(LifeCommand::Stroke { points: vec![(0, 10), (9, 10)], radius: 0, alive: false }).unwrap();
    assert!(!s.cell(5, 10), "eraser stroke");
    s.apply(LifeCommand::ClearRegion { rect: CellRect::new(0, 0, 39, 25) }).unwrap();
    assert!(s.live_cells().iter().all(|&(_, y)| y > 25) || s.population() == 0);
    s.apply(LifeCommand::Clear).unwrap();
    assert_eq!(s.population(), 0);
    // On a torus single cells wrap.
    let mut t = sim(Topology::Torus, 10, 10, vec![]);
    t.apply(LifeCommand::SetCell { x: -1, y: 12, alive: true }).unwrap();
    assert!(t.cell(9, 2));
    assert!(
        t.apply(LifeCommand::Line { from: (0, 0), to: (1 << 40, 0), alive: true }).is_err(),
        "huge shapes are refused"
    );
}

#[test]
fn paste_modes_rotation_and_mirror() {
    let l = Pattern::from_ascii("O..\nOOO").unwrap();
    let mut s = sim(Topology::Infinite, 64, 64, vec![]);
    s.apply(LifeCommand::Paste {
        pattern: l.clone(),
        x: 10,
        y: 10,
        rotation: Rotation::Cw90,
        mirror: false,
        mode: PasteMode::Or,
    })
    .unwrap();
    assert_eq!(s.live_cells(), vec![(10, 10), (11, 10), (10, 11), (10, 12)]);
    s.apply(LifeCommand::Clear).unwrap();
    s.apply(LifeCommand::Paste {
        pattern: l.clone(),
        x: 0,
        y: 0,
        rotation: Rotation::None,
        mirror: true,
        mode: PasteMode::Or,
    })
    .unwrap();
    assert_eq!(s.live_cells(), vec![(2, 0), (0, 1), (1, 1), (2, 1)]);
    s.apply(LifeCommand::Paste {
        pattern: l.clone(),
        x: 0,
        y: 0,
        rotation: Rotation::None,
        mirror: true,
        mode: PasteMode::Xor,
    })
    .unwrap();
    assert_eq!(s.population(), 0, "xor with itself clears");
    s.apply(LifeCommand::SetCell { x: 1, y: 0, alive: true }).unwrap();
    s.apply(LifeCommand::Paste {
        pattern: l,
        x: 0,
        y: 0,
        rotation: Rotation::None,
        mirror: false,
        mode: PasteMode::Copy,
    })
    .unwrap();
    assert!(!s.cell(1, 0), "copy mode clears the bounding box first");
}

#[test]
fn region_tools_and_copy() {
    let mut s = sim(Topology::Infinite, 64, 64, vec![glider_at(10, 10)]);
    let rect = CellRect::new(10, 10, 12, 12);
    let copy = s.copy_region(rect).unwrap();
    assert_eq!(copy.population(), 5);
    assert_eq!(copy.rule.as_deref(), Some("B3/S23"));
    s.apply(LifeCommand::MoveRegion { rect, dx: 100, dy: -5 }).unwrap();
    assert_eq!(s.copy_region(CellRect::new(110, 5, 112, 7)).unwrap(), copy, "moved intact");
    assert_eq!(s.population(), 5);
    let moved = CellRect::new(110, 5, 112, 7);
    s.apply(LifeCommand::RotateRegion { rect: moved, rotation: Rotation::Half }).unwrap();
    s.apply(LifeCommand::RotateRegion { rect: moved, rotation: Rotation::Half }).unwrap();
    assert_eq!(s.copy_region(moved).unwrap(), copy, "two half turns are the identity");
    s.apply(LifeCommand::MirrorRegion { rect: moved, axis: MirrorAxis::Vertical }).unwrap();
    let mirrored: Vec<(i64, i64)> = s.live_cells();
    s.apply(LifeCommand::MirrorRegion { rect: moved, axis: MirrorAxis::Vertical }).unwrap();
    assert_ne!(mirrored, s.live_cells());
    assert_eq!(s.copy_region(moved).unwrap(), copy, "mirroring twice is the identity");
    // Cut = copy + clear.
    let cut = s.copy_region(moved).unwrap();
    s.apply(LifeCommand::ClearRegion { rect: moved }).unwrap();
    assert_eq!(s.population(), 0);
    s.apply(LifeCommand::Paste {
        pattern: cut,
        x: moved.x0,
        y: moved.y0,
        rotation: Rotation::None,
        mirror: false,
        mode: PasteMode::Or,
    })
    .unwrap();
    assert_eq!(s.copy_region(moved).unwrap(), copy);
}

#[test]
fn randomize_with_seed_symmetry_and_region() {
    let mut a = sim(Topology::Torus, 64, 64, vec![]);
    let mut b = sim(Topology::Torus, 64, 64, vec![]);
    let cmd = LifeCommand::Randomize {
        density: 0.5,
        seed: Some(99),
        region: Some(CellRect::new(8, 8, 39, 39)),
        symmetry: Symmetry::Rotate8,
    };
    a.apply(cmd.clone()).unwrap();
    b.apply(cmd).unwrap();
    assert_eq!(a.live_cells(), b.live_cells(), "seeded fills are reproducible");
    assert!(a.live_cells().iter().all(|&(x, y)| (8..=39).contains(&x) && (8..=39).contains(&y)));
    for &(x, y) in &a.live_cells() {
        assert!(a.cell(47 - x, y) && a.cell(y, x), "8-fold symmetric about the region centre");
    }
    assert!(
        a.apply(LifeCommand::Randomize {
            density: 0.5,
            seed: None,
            region: Some(CellRect::new(0, 0, 9, 4)),
            symmetry: Symmetry::Rotate4
        })
        .is_err()
    );
    // Without a seed the model's generator is used and advances.
    a.apply(LifeCommand::Randomize { density: 0.5, seed: None, region: None, symmetry: Symmetry::None }).unwrap();
    let first = a.live_cells();
    a.apply(LifeCommand::Randomize { density: 0.5, seed: None, region: None, symmetry: Symmetry::None }).unwrap();
    assert_ne!(first, a.live_cells());
}

#[test]
fn load_and_import_patterns() {
    let mut s = sim(Topology::Infinite, 64, 64, vec![glider_at(0, 0)]);
    s.advance(10);
    s.apply(LifeCommand::ImportText {
        text: "#N Replicator\nx = 5, y = 5, rule = B36/S23\n2b3o$bo2bo$o3bo$o2bo$3o!".into(),
        file_name: Some("replicator.rle".into()),
        x: 0,
        y: 0,
        replace: true,
    })
    .unwrap();
    assert_eq!(s.tick(), 0, "loading restarts the generation count");
    assert_eq!(s.rule(), LifeRule::highlife(), "the file's rule is adopted");
    assert_eq!(s.preset(), RulePreset::HighLife);
    assert_eq!(s.population(), 12);
    s.advance(12);
    assert_eq!(s.population(), 24, "the replicator doubled");
    s.reset();
    assert_eq!((s.tick(), s.population()), (0, 12), "the loaded pattern is the reset point");
    // Pasting text keeps the rule; a Generations rule cannot be loaded.
    s.apply(LifeCommand::ImportText { text: "3o!".into(), file_name: None, x: 50, y: 50, replace: false }).unwrap();
    assert_eq!(s.population(), 15);
    let bad = LifeCommand::LoadPattern {
        pattern: Pattern::from_ascii("OO").unwrap().with_rule("/2/3"),
        x: 0,
        y: 0,
        use_pattern_rule: true,
    };
    assert!(s.apply(bad).is_err());
    assert!(
        s.apply(LifeCommand::ImportText {
            text: "x = 2, y = 1\n2o%!".into(),
            file_name: None,
            x: 0,
            y: 0,
            replace: true
        })
        .is_err()
    );
}

#[test]
fn reset_returns_to_generation_zero_including_edits() {
    let mut s =
        sim(Topology::Torus, 32, 32, vec![InitItem::Random { region: None, density: 0.3, symmetry: Symmetry::None }]);
    let start = s.live_cells();
    s.apply(LifeCommand::SetCell { x: 0, y: 0, alive: !s.cell(0, 0) }).unwrap();
    let edited = s.live_cells();
    assert_ne!(edited, start);
    s.advance(25);
    s.apply(LifeCommand::Clear).unwrap();
    s.reset();
    assert_eq!((s.tick(), s.live_cells()), (0, edited), "edits at generation 0 are part of the reset point");
    // After a world change reset rebuilds the scenario from the seed.
    s.set_param("height", ParamValue::Int(33)).unwrap();
    s.reset();
    assert_eq!(s.world().height, 33);
    s.set_param("height", ParamValue::Int(32)).unwrap();
    s.reset();
    assert_eq!(s.live_cells(), start);
}

#[test]
fn snapshots_replay_identically_on_every_engine() {
    for engine in ["dense", "bit", "sparse", "hashlife"] {
        let topology = if matches!(engine, "sparse" | "hashlife") { Topology::Infinite } else { Topology::Torus };
        let mut s =
            sim(topology, 48, 48, vec![InitItem::Random { region: None, density: 0.4, symmetry: Symmetry::None }]);
        s.set_param("engine", text(engine)).unwrap();
        s.set_param("track_ages", ParamValue::Bool(true)).unwrap();
        s.set_param("track_activity", ParamValue::Bool(true)).unwrap();
        s.advance(5);
        let snap = s.snapshot().unwrap();
        assert_eq!(snap.model_id, "life.classic");
        let continue_run = |s: &mut LifeSim| {
            s.apply(LifeCommand::Randomize {
                density: 0.2,
                seed: None,
                region: Some(CellRect::new(0, 0, 9, 9)),
                symmetry: Symmetry::None,
            })
            .unwrap();
            s.advance(9);
            (s.live_cells(), s.age(5, 5), s.activity(5, 5), s.metrics())
        };
        let expected = continue_run(&mut s);
        let mut restored = sim(Topology::Bounded, 4, 4, vec![]);
        restored.restore(&snap).unwrap();
        assert_eq!(restored.engine_kind(), s.engine_kind());
        assert_eq!(continue_run(&mut restored), expected, "{engine}: restored run replays identically");
    }
    let mut lab = LifeSim::new(&LifeConfig::default(), 1, &PatternLibrary::builtin(), LifeMode::Lab).unwrap();
    let snap = LifeSim::conway(1).snapshot().unwrap();
    assert!(lab.restore(&snap).is_err(), "a classic snapshot is not a lab snapshot");
}

#[test]
fn ages_and_activity_are_tracked_per_generation() {
    let mut s = sim(Topology::Torus, 16, 16, vec![]);
    s.apply(LifeCommand::Rectangle { rect: CellRect::new(4, 4, 5, 5), filled: true, alive: true }).unwrap(); // block
    s.apply(LifeCommand::Line { from: (10, 8), to: (12, 8), alive: true }).unwrap(); // blinker
    s.set_param("track_ages", ParamValue::Bool(true)).unwrap();
    s.set_param("track_activity", ParamValue::Bool(true)).unwrap();
    s.set_param("generations_per_tick", ParamValue::Int(3)).unwrap();
    s.step();
    assert_eq!(s.age(4, 4), 4, "block cells: tracked from age 1, then three generations");
    assert_eq!(s.age(11, 8), 4, "the blinker centre never dies");
    assert_eq!((s.age(11, 7), s.age(10, 8)), (1, 0), "after three generations the blinker is vertical");
    assert_eq!(s.activity(11, 7), 255, "changed in the last generation");
    assert_eq!(s.activity(4, 4), 0, "block cells never changed");
    // Views carry the channels.
    let view = s.view(&GridViewRequest { x0: 0, y0: 0, x1: 15, y1: 15, max_width: 16, max_height: 16 });
    assert_eq!(view.texel_at_cell(4, 4), Some([1, 4, 0, 255]));
    assert_eq!(view.texel_at_cell(11, 7), Some([1, 1, 255, 255]));
    assert_eq!(view.texel_at_cell(10, 8).map(|t| t[2]), Some(255), "the cell that died this generation glows");
}

#[test]
fn views_respect_the_budget_and_report_density() {
    let mut s = sim(Topology::Infinite, 64, 64, vec![]);
    s.apply(LifeCommand::Rectangle { rect: CellRect::new(0, 0, 63, 63), filled: true, alive: true }).unwrap();
    let exact = s.view(&GridViewRequest { x0: -10, y0: -10, x1: 9, y1: 9, max_width: 64, max_height: 64 });
    assert_eq!((exact.cells_per_texel, exact.width, exact.height, exact.origin_x), (1, 20, 20, -10));
    assert_eq!(exact.rgba.len(), 20 * 20 * 4);
    assert_eq!(exact.texel_at_cell(-1, 0), Some([0, 0, 0, 0]));
    assert_eq!(exact.texel_at_cell(0, 0), Some([1, 0, 0, 255]));
    let lod = s.view(&GridViewRequest { x0: -32, y0: -32, x1: 95, y1: 95, max_width: 8, max_height: 8 });
    assert_eq!((lod.cells_per_texel, lod.width, lod.height), (16, 8, 8));
    assert_eq!(lod.texel_at_cell(-1, -1).map(|t| t[3]), Some(0));
    assert_eq!(lod.texel_at_cell(0, 0).map(|t| t[3]), Some(255), "a full block");
    // Unaligned request: border texels only count the requested cells.
    let edge = s.view(&GridViewRequest { x0: 60, y0: 0, x1: 67, y1: 7, max_width: 1, max_height: 1 });
    assert_eq!((edge.cells_per_texel, edge.width, edge.height), (128, 1, 1), "x 60..=67 spans two 64-cell blocks");
    assert_eq!(edge.rgba[3], 128, "4 live columns of the 8 requested");
    for engine in ["dense", "hashlife"] {
        s.set_param("engine", text(engine)).unwrap();
        let again = s.view(&GridViewRequest { x0: 60, y0: 0, x1: 67, y1: 7, max_width: 1, max_height: 1 });
        assert_eq!(again, edge, "{engine}");
    }
}

#[test]
fn region_tools_clear_the_wrapped_destination_on_a_torus() {
    let mut s = sim(Topology::Torus, 10, 10, vec![]);
    // Debris where the moved cells will land after wrapping around both edges.
    s.apply(LifeCommand::Rectangle { rect: CellRect::new(0, 0, 2, 2), filled: true, alive: true }).unwrap();
    let glider = Pattern::from_ascii(".O.\n..O\nOOO").unwrap();
    s.apply(LifeCommand::Paste {
        pattern: glider.clone(),
        x: 6,
        y: 6,
        rotation: Rotation::None,
        mirror: false,
        mode: PasteMode::Or,
    })
    .unwrap();
    s.apply(LifeCommand::MoveRegion { rect: CellRect::new(6, 6, 8, 8), dx: 4, dy: 4 }).unwrap();
    assert_eq!(s.live_cells(), vec![(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)], "the destination is replaced, not merged");
    // Copy-mode paste across the edge clears the wrapped bounding box too.
    s.apply(LifeCommand::Line { from: (0, 7), to: (0, 9), alive: true }).unwrap();
    s.apply(LifeCommand::Paste {
        pattern: glider,
        x: 8,
        y: 7,
        rotation: Rotation::None,
        mirror: false,
        mode: PasteMode::Copy,
    })
    .unwrap();
    let pasted: Vec<(i64, i64)> = s.live_cells().into_iter().filter(|&(_, y)| y >= 7).collect();
    assert_eq!(pasted, vec![(9, 7), (0, 8), (0, 9), (8, 9), (9, 9)], "the debris at (0, 7) is cleared");
    // Rotating a 1×3 region at x = 8 gives a 3×1 region over x = 8..=10: its
    // last column wraps to x = 0, which is cleared although no cell lands there.
    let mut t = sim(Topology::Torus, 10, 10, vec![]);
    t.apply(LifeCommand::Line { from: (8, 8), to: (8, 9), alive: true }).unwrap();
    t.apply(LifeCommand::SetCell { x: 0, y: 7, alive: true }).unwrap(); // debris in the wrapped image
    t.apply(LifeCommand::RotateRegion { rect: CellRect::new(8, 7, 8, 9), rotation: Rotation::Cw90 }).unwrap();
    assert_eq!(t.live_cells(), vec![(8, 7), (9, 7)]);
}

#[test]
fn the_dense_engine_refuses_cells_it_cannot_hold_instead_of_dropping_them() {
    let mut s = sim(Topology::Infinite, 64, 64, vec![glider_at(0, 0)]);
    s.set_param("engine", text("dense")).unwrap();
    let far = LifeCommand::SetCell { x: 1 << 30, y: 1 << 30, alive: true };
    assert!(s.apply(far).is_err(), "the window cannot span 2^30 cells");
    assert_eq!(s.population(), 5, "nothing was written");
    // Switching to the dense engine with far-apart cells is refused too.
    let mut t = sim(Topology::Infinite, 64, 64, vec![glider_at(0, 0), glider_at(1 << 30, 0)]);
    let before = t.live_cells();
    assert!(t.set_param("engine", text("dense")).is_err());
    assert_eq!((t.engine_kind(), t.live_cells()), (EngineKind::Sparse, before), "the state is kept");
    // A dense engine cannot be chosen when the next reset could not rebuild the scenario.
    let mut u =
        sim(Topology::Torus, 64, 64, vec![InitItem::Random { region: None, density: 0.2, symmetry: Symmetry::None }]);
    u.set_param("engine", text("dense")).unwrap();
    assert!(u.set_param("topology", text("infinite")).is_ok(), "a 64×64 home region fits the window");
    assert!(u.set_param("height", ParamValue::Int(4_096)).is_ok());
    let err = u.set_param("width", ParamValue::Int(65_536)).unwrap_err().to_string();
    assert!(err.contains("Sparse or Hashlife"), "a 65,536 × 4,096 home region does not fit: {err}");
}

#[test]
fn random_fills_cover_worlds_larger_than_the_list_limit() {
    // 16,384 × 8,200 cells (more than 2^27): streamed into the bit engine.
    let config = LifeConfig {
        topology: Topology::Torus,
        width: 16_384,
        height: 8_200,
        init: vec![InitItem::Random { region: None, density: 0.01, symmetry: Symmetry::None }],
        ..LifeConfig::default()
    };
    let s =
        LifeSim::new(&config, 3, &PatternLibrary::builtin(), LifeMode::Classic).expect("large worlds can be filled");
    let expected = 0.01 * 16_384.0 * 8_200.0;
    assert!((s.population() as f64 - expected).abs() < 0.02 * expected, "{}", s.population());
    assert_eq!(s.engine_kind(), EngineKind::Bit);
    // Changing the world size so that a symmetric fill becomes impossible is refused up front.
    let mut sym = sim(
        Topology::Torus,
        32,
        32,
        vec![InitItem::Random { region: None, density: 0.3, symmetry: Symmetry::Rotate8 }],
    );
    let err = sym.set_param("width", ParamValue::Int(40)).unwrap_err().to_string();
    assert!(err.contains("square"), "{err}");
}

#[test]
fn activity_marks_every_change_of_the_tick_and_decays_per_tick() {
    let mut s = sim(Topology::Torus, 16, 16, vec![]);
    // An L-tromino becomes a block in one generation, then nothing changes.
    for (x, y) in [(4, 4), (5, 4), (4, 5)] {
        s.apply(LifeCommand::SetCell { x, y, alive: true }).unwrap();
    }
    s.set_param("track_activity", ParamValue::Bool(true)).unwrap();
    s.set_param("generations_per_tick", ParamValue::Int(4)).unwrap();
    s.step();
    assert_eq!(s.activity(5, 5), 255, "born in the first generation of this tick");
    s.step();
    assert_eq!(s.activity(5, 5), 255 - 16, "one tick later");
    let view = s.view(&GridViewRequest { x0: 0, y0: 0, x1: 15, y1: 15, max_width: 16, max_height: 16 });
    assert_eq!(view.texel_at_cell(5, 5).map(|t| t[2]), Some(239));
}

#[test]
fn births_and_deaths_are_those_of_the_last_tick_after_advance() {
    let mut s = sim(Topology::Torus, 10, 10, vec![]);
    s.apply(LifeCommand::Line { from: (3, 4), to: (5, 4), alive: true }).unwrap();
    assert_eq!(s.advance(3), 3);
    let m = s.metrics();
    assert_eq!((m.get("births"), m.get("deaths")), (Some(2.0), Some(2.0)), "one tick of a blinker, not three");
}

#[test]
fn hashlife_jumps_keep_the_generation_exact_when_a_later_jump_fails() {
    // A glider just inside the ±2^61 range Hashlife can step: the first jump
    // of 16 generations carries it out of the range, the next one fails.
    let edge = (1i64 << 61) - 3;
    let config = LifeConfig {
        topology: Topology::Infinite,
        engine: cellular_automata::EngineChoice::Hashlife,
        generations_per_tick: 16,
        init: vec![InitItem::Pattern {
            source: PatternSource::Text("x = 3, y = 3\nbo$2bo$3o!".into()),
            x: edge,
            y: edge,
            rotation: Rotation::None,
            mirror: false,
        }],
        ..LifeConfig::default()
    };
    let mut s = LifeSim::new(&config, 1, &PatternLibrary::builtin(), LifeMode::Classic).unwrap();
    let advanced = s.advance(3);
    assert!(s.halt_reason().is_some(), "the second jump leaves the 64-bit plane");
    assert_eq!((advanced, s.tick()), (16, 16), "the completed jump is counted");
    let hl = s.engine().as_any().downcast_ref::<cellular_automata::HashlifeEngine>().unwrap();
    assert_eq!(hl.universe().generation(), s.tick(), "model and universe agree");
    assert_eq!(s.population(), 5);
}

#[test]
fn cut_copies_then_clears() {
    let mut s = sim(Topology::Infinite, 64, 64, vec![glider_at(10, 10)]);
    let rect = CellRect::new(10, 10, 12, 12);
    let copy = s.copy_region(rect).unwrap();
    let cut = s.cut_region(rect).unwrap();
    assert_eq!(cut, copy);
    assert_eq!(s.population(), 0);
}
