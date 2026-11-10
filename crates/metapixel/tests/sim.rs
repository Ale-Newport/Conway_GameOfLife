//! The `life.metapixel` Simulation contract: descriptor, parameters and
//! apply policies, commands, views, snapshots, reset and scenarios. These
//! tests stay below one metageneration of physical time, so they are fast.

mod common;

use metapixel::spec::{PERIOD, TILE_PITCH};
use metapixel::{
    ACCELERATED_LABEL, CycleStage, GridViewRequest, MacroGrid, MetapixelCommand, MetapixelSim, MetapixelViewRequest,
    StepSize, TileReading, scenario,
};
use simulation_core::{ApplyPolicy, ModelFamily, ParamValue, SimError, Simulation};

fn blinker() -> MetapixelSim {
    common::sim(&["...", "OOO", "..."], "B3/S23", "moore")
}

#[test]
fn descriptor_states_the_executed_rules() {
    let sim = blinker();
    let d = sim.descriptor();
    assert_eq!(d.id, "life.metapixel");
    assert_eq!(d.family, ModelFamily::MetaCellular);
    assert_eq!(d.tick_unit, "generation");
    assert!(d.rules.contains("B3/S23") && d.rules.contains("35 328") && d.rules.contains("[250, 1750]²"));
    assert!(d.rules.contains("Accelerated macro representation — not a physical simulation"));
    for key in ["rule", "neighbourhood", "width", "height", "seed", "step", "node_limit"] {
        assert!(d.param(key).is_some(), "{key}");
    }
    let metrics = sim.metrics();
    for spec in &d.metrics {
        assert!(metrics.get(&spec.key).is_some(), "metric {} is declared and reported", spec.key);
    }
    assert_eq!(metrics.get("population"), Some(342_076.0));
}

#[test]
fn parameters_report_their_apply_policy() {
    let mut sim = blinker();
    assert_eq!(sim.set_param("rule", ParamValue::Text("B36/S23".into())), Ok(ApplyPolicy::RequiresReset));
    assert_eq!(sim.set_param("neighbourhood", ParamValue::Text("von_neumann".into())), Ok(ApplyPolicy::RequiresReset));
    assert_eq!(sim.set_param("width", ParamValue::Int(4)), Ok(ApplyPolicy::RequiresReset));
    assert_eq!(sim.set_param("step", ParamValue::Text("1024".into())), Ok(ApplyPolicy::Immediate));
    assert_eq!(sim.step_size(), StepSize::Pow2(10));
    // Command-line overrides (`ParamValue::parse_loose`) produce integers.
    assert_eq!(sim.set_param("step", ParamValue::parse_loose("35328")), Ok(ApplyPolicy::Immediate));
    assert_eq!(sim.step_size(), StepSize::Metagen);
    assert_eq!(sim.set_param("step", ParamValue::Int(1024)), Ok(ApplyPolicy::Immediate));
    assert_eq!(sim.params().get("step"), Some(&ParamValue::Text("1024".into())));
    // Pending values are reported, the running construction is unchanged.
    assert_eq!(sim.params().get("rule"), Some(&ParamValue::Text("B36/S23".into())));
    assert_eq!(sim.active_config().programming.rule.to_string(), "B3/S23");
    assert_eq!(sim.geometry().columns, 3);
    // Invalid values are rejected without side effects.
    assert!(matches!(sim.set_param("rule", ParamValue::Text("B9".into())), Err(SimError::InvalidParameter { .. })));
    assert!(sim.set_param("neighbourhood", ParamValue::Text("N,N".into())).is_err());
    assert!(sim.set_param("step", ParamValue::Text("1000".into())).is_err());
    assert!(sim.set_param("width", ParamValue::Int(0)).is_err());
    assert!(matches!(sim.set_param("nope", ParamValue::Int(1)), Err(SimError::UnknownParameter(_))));
    // Reset applies the pending configuration (pattern anchored top-left).
    sim.reset();
    assert_eq!(sim.active_config().programming.rule.to_string(), "B36/S23");
    assert_eq!(sim.active_config().programming.neighbourhood.to_string(), "von_neumann");
    assert_eq!(sim.initial_pattern().to_rows(), ["....", "OOO.", "...."]);
    assert_eq!(sim.tick(), 0);
    assert!(sim.descriptor().rules.contains("B36/S23"));
    // The macro rule is programmed into the tiles; the cells always run Life.
    assert_eq!(sim.universe().rule().to_string(), "B3/S23");
}

#[test]
fn steps_stop_at_metageneration_boundaries() {
    let mut sim = common::sim(&["O"], "B3/S23", "moore");
    sim.set_param("step", ParamValue::Text("32768".into())).unwrap();
    sim.step();
    assert_eq!(sim.tick(), 32_768);
    assert_eq!(sim.history().total_readouts(), 0);
    let phase = sim.view(&MetapixelViewRequest { micro: None, macro_grid: false, ..Default::default() }).phase;
    assert_eq!(
        (phase.stage, phase.displayed_metagen, phase.next_readout_generation),
        (CycleStage::FirstCycle, 0, PERIOD)
    );
    assert_eq!(sim.advance(100), 100, "advance counts generations");
    assert_eq!(sim.tick(), 32_868);
}

#[test]
fn commands_rebuild_at_generation_zero() {
    let mut sim = blinker();
    sim.advance(1000);
    sim.apply(MetapixelCommand::ToggleTile { col: 1, row: 0 }).unwrap();
    assert_eq!(sim.tick(), 0, "edits rebuild the construction at generation 0");
    assert_eq!(sim.initial_pattern().to_rows(), [".O.", "OOO", "..."]);
    assert_eq!(sim.universe().population(), 342_076 + 66_206 - 23_923);
    // Errors leave everything unchanged.
    sim.advance(10);
    let error = sim.apply(MetapixelCommand::SetTile { col: 3, row: 0, on: true }).unwrap_err();
    assert!(error.to_string().contains("outside the 3×3 macro grid"), "{error}");
    assert_eq!(sim.tick(), 10);
    assert!(sim.apply(MetapixelCommand::Randomize { density: 2.0 }).is_err());
    assert!(sim.apply(MetapixelCommand::PlaceRle { rle: "3o!".into(), col: 1, row: 0 }).is_err(), "off the grid");
    assert!(sim.apply(MetapixelCommand::RestartFromLatestReadout).is_err(), "nothing read yet");
    // RLE placement and pattern replacement.
    sim.apply(MetapixelCommand::Clear).unwrap();
    sim.apply(MetapixelCommand::PlaceRle { rle: "bo$2bo$3o!".into(), col: 0, row: 0 }).unwrap();
    assert_eq!(sim.initial_pattern().to_rows(), [".O.", "..O", "OOO"]);
    sim.apply(MetapixelCommand::SetPattern(MacroGrid::from_rows(&["OO", "OO"]).unwrap())).unwrap();
    assert_eq!((sim.active_config().width, sim.pending_config().width), (2, 2));
    assert_eq!(sim.universe().population(), 264_796);
    // Randomize draws from the seeded generator: same seed, same soup.
    let mut a = blinker();
    let mut b = blinker();
    a.apply(MetapixelCommand::Randomize { density: 0.5 }).unwrap();
    b.apply(MetapixelCommand::Randomize { density: 0.5 }).unwrap();
    assert_eq!(a.initial_pattern(), b.initial_pattern());
}

#[test]
fn micro_view_follows_the_grid_view_contract() {
    let sim = blinker();
    let whole = GridViewRequest { x0: -5, y0: -5, x1: 6148, y1: 6148, max_width: 700, max_height: 500 };
    let view = sim.view(&MetapixelViewRequest { micro: Some(whole), ..Default::default() });
    let micro = view.micro.unwrap();
    assert!(micro.cells_per_texel.is_power_of_two());
    assert!(micro.width <= 700 && micro.height <= 500);
    assert_eq!(micro.cells_per_texel, 16, "6158 cells into 500 texels needs 2^4 cells per texel");
    assert_eq!((micro.origin_x % 16, micro.origin_y % 16), (0, 0));
    assert!(micro.origin_x <= -5 && micro.origin_x + 16 > -5);
    assert_eq!(micro.rgba.len(), (micro.width * micro.height * 4) as usize);
    // Every texel encodes the exact block population of Hashlife's density grid.
    let counts = sim
        .universe()
        .density_grid(micro.origin_x, micro.origin_y, micro.width as usize, micro.height as usize, 4)
        .unwrap();
    for (texel, &count) in micro.rgba.chunks(4).zip(&counts) {
        let density = (255.0 * f64::from(count) / 256.0).round() as u8;
        assert_eq!(texel, [u8::from(count > 0), 0, 0, density]);
    }
    assert_eq!(counts.iter().map(|&c| u64::from(c)).sum::<u64>(), sim.universe().population());
    let scale = view.scale.unwrap();
    assert_eq!((scale.tiles_visible_x, scale.tiles_visible_y), (3, 3));
    assert_eq!(scale.texels_per_tile, 128.0);
    // Exact cells: a 64×32 window around the state boat of tile (1, 0), an OFF tile.
    let x0 = TILE_PITCH + 540;
    let y0 = 1955;
    let exact = GridViewRequest { x0, y0, x1: x0 + 63, y1: y0 + 31, max_width: 64, max_height: 64 };
    let micro = sim.view(&MetapixelViewRequest { micro: Some(exact), ..Default::default() }).micro.unwrap();
    assert_eq!((micro.width, micro.height, micro.cells_per_texel), (64, 32, 1));
    for (i, texel) in micro.rgba.chunks(4).enumerate() {
        let (x, y) = (x0 + (i % 64) as i64, y0 + (i / 64) as i64);
        let alive = sim.universe().get_cell(x, y);
        assert_eq!(texel, [u8::from(alive), 0, 0, if alive { 255 } else { 0 }]);
    }
}

#[test]
fn macro_view_and_inspection_label_their_sources() {
    let sim = blinker();
    let view = sim.view(&MetapixelViewRequest {
        micro: None,
        live_readings: true,
        inspect: Some((1, 1)),
        ..Default::default()
    });
    let macro_view = view.macro_grid.unwrap();
    assert_eq!((macro_view.columns, macro_view.rows, macro_view.exact_metagen), (3, 3, None));
    assert_eq!(macro_view.accelerated_label, ACCELERATED_LABEL);
    assert!(macro_view.exact_label.contains("Exact"));
    let accelerated: Vec<bool> = macro_view.tiles.iter().map(|t| t.accelerated).collect();
    assert_eq!(accelerated, [false, false, false, true, true, true, false, false, false]);
    assert!(macro_view.tiles.iter().all(|t| t.exact.is_none() && !t.mismatch));
    let live: Vec<_> = macro_view.tiles.iter().map(|t| t.live.unwrap()).collect();
    assert_eq!(live[4], metapixel::LiveReading::On);
    assert_eq!(live[0], metapixel::LiveReading::Off);
    let inspection = view.inspection.unwrap();
    assert_eq!((inspection.origin_x, inspection.origin_y), (TILE_PITCH, TILE_PITCH));
    assert_eq!(inspection.core_count, 29_795);
    assert!(inspection.annotations.iter().any(|a| a.label == "B3"));
    assert_eq!(inspection.neighbours.iter().filter(|n| n.tile.is_some()).count(), 8);
    let corner = sim.view(&MetapixelViewRequest { micro: None, inspect: Some((0, 0)), ..Default::default() });
    assert_eq!(corner.inspection.unwrap().neighbours.iter().filter(|n| n.tile.is_some()).count(), 3);
    let outside = sim.view(&MetapixelViewRequest { micro: None, inspect: Some((3, 0)), ..Default::default() });
    assert!(outside.inspection.is_none());
}

#[test]
fn snapshot_restore_replays_identically() {
    let mut sim = common::sim(&["O."], "B3/S23", "moore");
    sim.apply(MetapixelCommand::Randomize { density: 0.5 }).unwrap();
    sim.advance(3000);
    let snapshot = sim.snapshot().unwrap();
    assert_eq!(snapshot.schema_version, metapixel::SCHEMA_VERSION);
    sim.advance(2000);
    sim.apply(MetapixelCommand::Randomize { density: 0.5 }).unwrap();
    let after_pattern = sim.initial_pattern().clone();
    let mut other = common::sim(&["..."], "B36/S23", "vn");
    other.restore(&snapshot).unwrap();
    assert_eq!(other.tick(), 3000);
    assert_eq!(other.active_config(), sim.active_config());
    other.advance(2000);
    let mut replay = blinker();
    replay.restore(&snapshot).unwrap();
    replay.advance(2000);
    assert_eq!(common::cell_hash(other.universe()), common::cell_hash(replay.universe()));
    other.apply(MetapixelCommand::Randomize { density: 0.5 }).unwrap();
    assert_eq!(other.initial_pattern(), &after_pattern, "the RNG is part of the snapshot");
    // Wrong model / version are rejected.
    let mut wrong = snapshot.clone();
    wrong.model_id = "life.hashlife".into();
    assert!(matches!(other.restore(&wrong), Err(SimError::WrongModel { .. })));
    let mut newer = snapshot.clone();
    newer.schema_version += 1;
    assert!(matches!(other.restore(&newer), Err(SimError::UnsupportedVersion { .. })));
}

#[test]
fn builtin_scenarios_are_valid_and_match_the_files() {
    let builtin = scenario::builtin().unwrap();
    assert_eq!(builtin.len(), scenario::BUILTIN_FILES.len());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/metapixel");
    let on_disk = scenario::load_dir(&dir).unwrap();
    assert_eq!(on_disk.len(), builtin.len());
    for s in &on_disk {
        assert!(builtin.contains(s), "{} differs from its embedded copy", s.id);
    }
    let soup = scenario::builtin_by_id("metapixel.soup").unwrap();
    let sim = MetapixelSim::from_scenario(common::tiles(), &soup).unwrap();
    assert_eq!((sim.geometry().columns, sim.geometry().rows), (6, 6));
    let again = MetapixelSim::from_scenario(common::tiles(), &soup).unwrap();
    assert_eq!(sim.initial_pattern(), again.initial_pattern(), "seeded");
    assert!(scenario::builtin_by_id("metapixel.nope").is_err());
    for s in &builtin {
        assert_eq!(&scenario::parse(&s.to_ron().unwrap(), "round trip").unwrap(), s, "{}", s.id);
    }
    // Reset restores the generator of a freshly built scenario.
    let mut fresh = MetapixelSim::from_scenario(common::tiles(), &soup).unwrap();
    let mut reset = MetapixelSim::from_scenario(common::tiles(), &soup).unwrap();
    reset.advance(10);
    reset.reset();
    fresh.apply(MetapixelCommand::Randomize { density: 0.5 }).unwrap();
    reset.apply(MetapixelCommand::Randomize { density: 0.5 }).unwrap();
    assert_eq!(fresh.initial_pattern(), reset.initial_pattern());
    let bad = scenario::parse(
        "(id: \"metapixel.bad\", name: \"x\", description: \"x\", width: 2, height: 1, init: Rows([\"O\"]))",
        "bad.ron",
    );
    assert!(bad.unwrap_err().to_string().contains("row 1 has 1 cells, expected width 2"));
    let typo = scenario::parse("(id: \"metapixel.t\", nme: \"x\")", "typo.ron").unwrap_err();
    assert!(typo.to_string().contains("typo.ron"), "{typo}");
}

#[test]
fn readings_are_classified_strictly() {
    assert_eq!(TileReading::classify(29_795).state(), Some(true));
    assert_eq!(TileReading::classify(0).state(), Some(false));
    assert_eq!(TileReading::classify(42).state(), None);
}
