//! The `Simulation` contract: determinism, snapshots, parameters, commands,
//! views and scenario loading.

use continuous::scenario::{self, BUILTIN_FILES, builtin, builtin_by_id, creature_library, load_dir};
use continuous::{
    Blend, Brush, BrushMode, ContinuousCommand, ContinuousSim, ContinuousViewRequest, LENIA_ID, Placement, Region,
    ViewLayer,
};
use std::path::Path;
use std::time::Duration;

use simulation_core::{
    ApplyPolicy, ParamValue, Runner, RunnerConfig, SessionControl, SimError, Simulation, Snapshot, erase,
};

fn sim(id: &str) -> ContinuousSim {
    let scenario = builtin_by_id(id).unwrap_or_else(|| panic!("missing scenario {id}"));
    ContinuousSim::from_scenario(scenario).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn fields_of(s: &ContinuousSim) -> Vec<Vec<f32>> {
    s.fields().iter().map(|g| g.as_slice().to_vec()).collect()
}

#[test]
fn builtin_scenarios_match_the_scenario_folder() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/continuous");
    let loaded = load_dir(&dir).expect("scenario folder loads");
    assert_eq!(loaded.len(), BUILTIN_FILES.len());
    let embedded = builtin();
    for s in &embedded {
        assert!(loaded.iter().any(|l| l == s), "{} differs from its file", s.header.id);
        let sim = ContinuousSim::from_scenario(s.clone()).expect("builds");
        assert_eq!(sim.descriptor().id, sim.kind().model_id());
    }
    let mut ids: Vec<_> = embedded.iter().map(|s| s.header.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), embedded.len(), "scenario ids are unique");
    assert!(creature_library().len() >= 7);
}

#[test]
fn scenario_errors_are_reported_with_positions() {
    let err = scenario::parse("(header: (id: \"x\"", "broken.ron").expect_err("syntax error");
    assert!(err.to_string().contains("broken.ron"), "{err}");
    let mut text = BUILTIN_FILES[0].1.to_string();
    text = text.replace("sigma: 0.015", "sigma: -1.0");
    let err = scenario::parse(&text, "bad.ron").expect_err("invalid σ");
    assert!(err.to_string().contains("σ"), "{err}");
    let text = BUILTIN_FILES[0].1.replace("x: 0.5", "x: 1.5");
    assert!(scenario::parse(&text, "pos.ron").is_err());
    assert!(load_dir(Path::new("/definitely/not/here")).is_err());
}

#[test]
fn scenario_round_trips_through_ron() {
    for s in builtin() {
        let text = scenario::to_ron(&s).expect("serialises");
        let back = scenario::parse(&text, "round-trip").expect("parses");
        assert_eq!(back, s);
    }
}

#[test]
fn deterministic_replay_and_thread_count_independence() {
    // 384 × 256 cells is above the parallel threshold, so row blocks run on
    // several threads; one thread must give identical bits.
    let wide = || {
        let mut s = sim("lenia.soup");
        s.set_param("width", ParamValue::Int(384)).expect("valid width");
        s.reset();
        s
    };
    let mut a = wide();
    let mut b = wide();
    assert!(a.width() * a.height() > continuous::parallel::PARALLEL_MIN_CELLS);
    a.advance(40);
    b.advance(40);
    assert_eq!(fields_of(&a), fields_of(&b));
    assert_eq!(a.metrics(), b.metrics());
    let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("pool");
    let c = pool.install(|| {
        let mut c = wide();
        c.advance(40);
        c
    });
    assert_eq!(fields_of(&a), fields_of(&c));
    assert_eq!(a.metrics(), c.metrics());
}

#[test]
fn snapshot_round_trip_replays_identically() {
    for id in ["lenia.tessellatium_gyrans", "smoothlife.soup", "flow_lenia.demo"] {
        let mut s = sim(id);
        s.advance(10);
        s.apply(ContinuousCommand::Randomize {
            region: Region::Disc { x: 30.0, y: 30.0, radius: 8.0 },
            density: 0.5,
            amplitude: 1.0,
        })
        .expect("randomize");
        let snap = s.snapshot().expect("snapshot");
        let bytes = snap.to_bytes().expect("bytes");
        let snap = Snapshot::from_bytes(&bytes).expect("decode");
        assert_eq!(snap.model_id, s.kind().model_id());
        s.advance(15);
        // The generator state is part of the snapshot: randomising again replays.
        s.apply(ContinuousCommand::Randomize {
            region: Region::Rect { x: 0.0, y: 0.0, width: 9.0, height: 9.0 },
            density: 0.3,
            amplitude: 1.0,
        })
        .expect("randomize");
        s.advance(5);
        let expected = (fields_of(&s), s.metrics());
        s.restore(&snap).expect("restore");
        assert_eq!(s.tick(), 10);
        s.advance(15);
        s.apply(ContinuousCommand::Randomize {
            region: Region::Rect { x: 0.0, y: 0.0, width: 9.0, height: 9.0 },
            density: 0.3,
            amplitude: 1.0,
        })
        .expect("randomize");
        s.advance(5);
        assert_eq!((fields_of(&s), s.metrics()), expected, "{id}");
    }
}

#[test]
fn snapshots_from_other_models_are_rejected() {
    let lenia = sim("lenia.orbium");
    let mut smooth = sim("smoothlife.soup");
    let snap = lenia.snapshot().expect("snapshot");
    assert!(matches!(smooth.restore(&snap), Err(SimError::WrongModel { .. })));
    let mut newer = snap.clone();
    newer.schema_version = 99;
    let mut lenia2 = sim("lenia.orbium");
    assert!(matches!(lenia2.restore(&newer), Err(SimError::UnsupportedVersion { .. })));
    let mut garbage = snap;
    garbage.payload.truncate(10);
    assert!(lenia2.restore(&garbage).is_err());
}

#[test]
fn inconsistent_snapshots_are_rejected_instead_of_breaking_reset() {
    // A one-channel Lenia state whose embedded scenario is the three-channel
    // Tessellatium: restoring it would make the next reset build three
    // fields for a one-channel engine.
    let orbium = sim("lenia.orbium");
    let tess = builtin_by_id("lenia.tessellatium_gyrans").expect("scenario");
    let mut payload = orbium.snapshot().expect("snapshot");
    let original = sim("lenia.orbium").scenario().clone();
    let forged_scenario = postcard_swap(&payload.payload, &original, &tess);
    payload.payload = forged_scenario;
    let mut target = sim("lenia.orbium");
    let err = target.restore(&payload);
    assert!(matches!(&err, Err(SimError::Serialization(m)) if m.contains("channels")), "{err:?}");
    assert_eq!(target.tick(), 0, "a rejected snapshot leaves the world untouched");
    target.reset();
    target.step();
}

/// Re-encodes a snapshot payload with `replacement` in place of the leading
/// `original` scenario (the scenario is the first field of the state).
fn postcard_swap(payload: &[u8], original: &continuous::Scenario, replacement: &continuous::Scenario) -> Vec<u8> {
    let head = postcard::to_stdvec(original).expect("encode");
    assert_eq!(&payload[..head.len()], head.as_slice(), "the state starts with its scenario");
    let mut out = postcard::to_stdvec(replacement).expect("encode");
    out.extend_from_slice(&payload[head.len()..]);
    out
}

#[test]
fn reset_restores_the_scenario_and_applies_pending_size() {
    let mut s = sim("lenia.orbium");
    let initial = fields_of(&s);
    s.advance(25);
    s.reset();
    assert_eq!(s.tick(), 0);
    assert_eq!(fields_of(&s), initial);
    assert_eq!(s.set_param("width", ParamValue::Int(96)), Ok(ApplyPolicy::RequiresReset));
    assert_eq!(s.width(), 128, "size changes wait for a reset");
    assert_eq!(s.params().i64_or("width", 0), 96);
    s.reset();
    assert_eq!((s.width(), s.height()), (96, 128));
    // The creature is re-placed at the same relative position with the same mass.
    assert!((s.mass() - 75.10).abs() < 0.01);
    // A kernel that does not fit the pending size is refused.
    assert!(s.set_param("height", ParamValue::Int(20)).is_err());
    assert!(s.set_param("radius", ParamValue::Float(60.0)).is_err());
}

#[test]
fn rule_parameters_apply_next_tick_and_are_validated() {
    let mut s = sim("lenia.orbium");
    assert_eq!(s.set_param("k0.mu", ParamValue::Float(0.2)), Ok(ApplyPolicy::NextTick));
    assert_eq!(s.params().f64_or("k0.mu", 0.0), 0.2);
    assert_eq!(s.set_param("k0.beta", ParamValue::Text("1/2, 1".into())), Ok(ApplyPolicy::NextTick));
    assert_eq!(s.params().str_or("k0.beta", ""), "0.5,1");
    assert!(s.descriptor().rules.contains("0.5,1"));
    assert_eq!(s.set_param("k0.core", ParamValue::Text("exponential".into())), Ok(ApplyPolicy::NextTick));
    assert!(matches!(s.set_param("nope", ParamValue::Int(1)), Err(SimError::UnknownParameter(_))));
    assert!(matches!(s.set_param("k0.sigma", ParamValue::Float(5.0)), Err(SimError::InvalidParameter { .. })));
    assert!(matches!(
        s.set_param("time_resolution", ParamValue::Text("x".into())),
        Err(SimError::InvalidParameter { .. })
    ));
    // Changing T changes the descriptor time step.
    s.set_param("time_resolution", ParamValue::Float(20.0)).expect("valid T");
    assert!((s.descriptor().dt - 0.05).abs() < 1e-12);
    // Simulated time accumulates the time step actually used by each tick.
    s.advance(2);
    s.set_param("time_resolution", ParamValue::Float(10.0)).expect("valid T");
    s.advance(1);
    assert!((s.metrics().sim_time - 0.2).abs() < 1e-12);
    // Every declared parameter has a value and every default validates.
    let d = s.descriptor();
    let values = s.params();
    for spec in &d.params {
        assert!(values.get(&spec.key).is_some(), "{} has no value", spec.key);
        assert!(spec.validate(&spec.default).is_ok(), "{}", spec.key);
    }
    let mut smooth = sim("smoothlife.soup");
    assert_eq!(smooth.set_param("stepping", ParamValue::Text("smooth".into())), Ok(ApplyPolicy::NextTick));
    assert!(smooth.set_param("b1", ParamValue::Float(0.5)).is_err(), "b1 > b2 is rejected");
    let mut flow = sim("flow_lenia.demo");
    assert_eq!(flow.set_param("temperature", ParamValue::Float(0.5)), Ok(ApplyPolicy::NextTick));
    assert!(flow.set_param("k0.rings", ParamValue::Text("0.5,1".into())).is_err());
    assert!(flow.set_param("temperature", ParamValue::Float(2.9)).is_ok());
    assert!(flow.set_param("window", ParamValue::Int(2)).is_err(), "s must stay below the window");
}

#[test]
fn multi_channel_kernel_routing_is_a_parameter() {
    let mut s = sim("lenia.tessellatium_gyrans");
    assert_eq!(s.params().i64_or("k9.destination", -1), 1);
    assert_eq!(s.set_param("k9.destination", ParamValue::Int(2)), Ok(ApplyPolicy::NextTick));
    assert!(s.set_param("k9.destination", ParamValue::Int(3)).is_err());
    assert!(s.descriptor().params.iter().any(|p| p.group.contains("channel 0 → 2")));
}

#[test]
fn commands_edit_the_world() {
    let mut s = sim("lenia.orbium");
    s.apply(ContinuousCommand::Clear).expect("clear");
    assert_eq!(s.mass(), 0.0);
    let brush =
        Brush { x: 64.0, y: 64.0, radius: 5.0, value: 1.0, softness: 0.0, mode: BrushMode::Paint, channel: None };
    s.apply(ContinuousCommand::Brush(brush)).expect("paint");
    assert!((s.mass() - 81.0).abs() < 5.0, "disc of radius 5 ≈ 78.5 cells, got {}", s.mass());
    let c = s.centroid().expect("defined");
    assert!((c.0 - 64.0).abs() < 0.5 && (c.1 - 64.0).abs() < 0.5);
    s.apply(ContinuousCommand::Brush(Brush { mode: BrushMode::Erase, radius: 8.0, ..brush })).expect("erase");
    assert_eq!(s.mass(), 0.0);
    let bad = Brush { value: 2.0, ..brush };
    assert!(matches!(s.apply(ContinuousCommand::Brush(bad)), Err(SimError::InvalidCommand(_))));
    // Place the library Orbium rotated by 90° and check its mass.
    let orbium = creature_library().into_iter().find(|c| c.scenario_id == "lenia.orbium").expect("orbium");
    let placement = Placement { x: 40.0, y: 40.0, rotation_deg: 90.0, scale: 1.0, blend: Blend::Replace };
    s.apply(ContinuousCommand::PlaceCreature { cells: orbium.cells.clone(), placement }).expect("place");
    assert!((s.mass() - orbium.cells.mass()).abs() < 0.05, "{} vs {}", s.mass(), orbium.cells.mass());
    // A three-channel creature does not fit a one-channel world.
    let tess = creature_library().into_iter().find(|c| c.scenario_id == "lenia.tessellatium_gyrans").expect("tess");
    let err = s.apply(ContinuousCommand::PlaceCreature { cells: tess.cells, placement });
    assert!(matches!(err, Err(SimError::InvalidCommand(_))));
    let err = s.apply(ContinuousCommand::Randomize {
        region: Region::Disc { x: 0.0, y: 0.0, radius: -1.0 },
        density: 0.5,
        amplitude: 1.0,
    });
    assert!(err.is_err());
}

#[test]
fn clear_and_randomize_hooks() {
    let mut s = sim("lenia.orbium");
    s.randomize(42);
    let a = fields_of(&s);
    assert!(s.mass() > 100.0);
    s.randomize(42);
    assert_eq!(fields_of(&s), a);
    s.randomize(43);
    assert_ne!(fields_of(&s), a);
    s.clear();
    assert_eq!(s.mass(), 0.0);
    let m = s.metrics();
    assert_eq!(m.get("centroid_x"), Some(0.0));
    assert_eq!(m.get("speed"), Some(0.0));
    let mut smooth = sim("smoothlife.soup");
    smooth.randomize(1);
    assert!(smooth.mass() > 1000.0);
}

#[test]
fn metrics_and_views() {
    let mut s = sim("lenia.orbium");
    s.advance(3);
    let m = s.metrics();
    for key in ["mass", "active_fraction", "centroid_x", "centroid_y", "speed", "mean_growth"] {
        let v = m.get(key).unwrap_or_else(|| panic!("missing {key}"));
        assert!(v.is_finite(), "{key} = {v}");
    }
    assert_eq!(m.keys().count(), s.descriptor().metrics.len());
    assert!((m.sim_time - 0.3).abs() < 1e-12);
    let speed = m.get("speed").unwrap_or_default();
    assert!(speed > 0.1 && speed < 2.0, "speed {speed}");
    let full = s.view(&ContinuousViewRequest::default());
    assert_eq!((full.cols, full.rows, full.cells_per_sample), (128, 128, 1));
    assert!((full.planes[0].iter().map(|&v| f64::from(v)).sum::<f64>() - s.mass()).abs() < 1e-3);
    assert!((full.channel_stats[0].mass - s.mass()).abs() < 1e-6);
    let small = s.view(&ContinuousViewRequest { max_cols: 32, max_rows: 32, ..Default::default() });
    assert_eq!((small.cols, small.rows, small.cells_per_sample), (32, 32, 4));
    let avg: f64 = small.planes[0].iter().map(|&v| f64::from(v)).sum::<f64>() * 16.0;
    assert!((avg - s.mass()).abs() < 1e-2, "box filter preserves the mean");
    let growth = s.view(&ContinuousViewRequest { layer: ViewLayer::Growth, ..Default::default() });
    assert_eq!(growth.value_range, (-1.0, 1.0));
    assert!(growth.planes[0].iter().all(|&g| (-1.0..=1.0).contains(&g)));
    let potential = s.view(&ContinuousViewRequest {
        layer: ViewLayer::Potential,
        x0: -10,
        y0: 5,
        width: 40,
        height: 20,
        ..Default::default()
    });
    assert_eq!((potential.x0, potential.y0, potential.cols, potential.rows), (118, 5, 40, 20));
    let smooth = sim("smoothlife.soup");
    let v = smooth.view(&ContinuousViewRequest { layer: ViewLayer::Potential, ..Default::default() });
    assert_eq!(v.labels, vec!["inner filling m".to_string(), "outer filling n".to_string()]);
    let flow = sim("flow_lenia.demo");
    assert!(flow.metrics().get("mass_drift").is_some());
    assert_eq!(flow.descriptor().metrics.len(), 7);
}

#[test]
fn descriptor_is_complete_and_runs_through_dyn_interface() {
    let s = sim("lenia.tessellatium_gyrans");
    let d = s.descriptor();
    assert_eq!(d.id, LENIA_ID);
    assert!(d.rules.contains("clip(A_j + dt · D_j, 0, 1)"));
    assert_eq!(d.rules.matches("\n| ").count(), 16, "header + 15 kernel rows");
    let mut dynamic = erase(s);
    assert_eq!(dynamic.advance(3), 3);
    assert!(dynamic.memory_bytes() > 128 * 128 * 4 * 3);
}

#[test]
fn parameter_defaults_are_the_scenario_values() {
    // "Restore default" in a property panel must restore the preset, not a
    // generic value: Chan's Orbium runs the polynomial core and growth.
    let chan = sim("lenia.orbium_chan");
    let d = chan.descriptor();
    let default = |key: &str| d.param(key).map(|p| p.default.clone());
    assert_eq!(default("k0.core"), Some(ParamValue::Text("polynomial".into())));
    assert_eq!(default("k0.growth"), Some(ParamValue::Text("polynomial".into())));
    assert_eq!(default("width"), Some(ParamValue::Int(128)));
    let tess = sim("lenia.tessellatium_gyrans").descriptor();
    assert_eq!(tess.param("k3.beta").map(|p| p.default.clone()), Some(ParamValue::Text("0,1".into())));
    assert_eq!(tess.param("k9.destination").map(|p| p.default.clone()), Some(ParamValue::Int(1)));
    assert_eq!(tess.param("time_resolution").map(|p| p.default.clone()), Some(ParamValue::Float(2.0)));
    let flow = sim("flow_lenia.demo").descriptor();
    assert_eq!(flow.param("radius").map(|p| p.default.clone()), Some(ParamValue::Float(33.0)));
    assert_eq!(flow.param("seed").map(|p| p.default.clone()), Some(ParamValue::Int(3)));
    // Defaults stay the preset's after the user changes a value.
    let mut s = sim("lenia.orbium_chan");
    s.set_param("k0.mu", ParamValue::Float(0.2)).expect("valid");
    assert_eq!(s.descriptor().param("k0.mu").map(|p| p.default.clone()), Some(ParamValue::Float(0.15)));
    for spec in &s.descriptor().params {
        assert!(spec.validate(&spec.default).is_ok(), "{}", spec.key);
    }
}

#[test]
fn growth_view_range_covers_summed_growths() {
    // Tessellatium sums h_k·G_k per destination channel: |D_j| ≤ max_j Σ h_k.
    let mut s = sim("lenia.tessellatium_gyrans");
    s.advance(5);
    let v = s.view(&ContinuousViewRequest { layer: ViewLayer::Growth, ..Default::default() });
    let bound = 0.138 + 0.48 + 0.284 + 0.67 + 0.43; // kernels writing channel 0
    assert!((v.value_range.1 - bound).abs() < 1e-5, "range {:?}", v.value_range);
    assert_eq!(v.value_range.0, -v.value_range.1);
    let (lo, hi) = v.planes.iter().flatten().fold((0.0f32, 0.0f32), |(a, b), &x| (a.min(x), b.max(x)));
    assert!(lo >= v.value_range.0 && hi <= v.value_range.1, "[{lo}, {hi}] outside {:?}", v.value_range);
    assert!(lo < -1.0, "summed growth goes below −1 here ({lo}), so (−1, 1) would clip it");
    // Weighted means stay in [−1, 1].
    let fish = sim("lenia.fish").view(&ContinuousViewRequest { layer: ViewLayer::Growth, ..Default::default() });
    assert_eq!(fish.value_range, (-1.0, 1.0));
}

#[test]
fn flow_lenia_edits_are_bounded() {
    let mut s = sim("flow_lenia.demo");
    let brush =
        Brush { x: 10.0, y: 10.0, radius: 3.0, value: 1.0e6, softness: 0.0, mode: BrushMode::Paint, channel: None };
    assert!(matches!(s.apply(ContinuousCommand::Brush(brush)), Err(SimError::InvalidCommand(_))));
    s.apply(ContinuousCommand::Brush(Brush { value: 5.0, ..brush })).expect("paint");
    assert!(s.fields()[0].as_slice().contains(&5.0), "values above 1 are allowed in Flow-Lenia");
    s.apply(ContinuousCommand::Randomize {
        region: Region::Disc { x: 64.0, y: 64.0, radius: 5.0 },
        density: 1.0,
        amplitude: 1.0e30,
    })
    .expect("capped");
    let max = s.fields()[0].as_slice().iter().copied().fold(0.0f32, f32::max);
    assert!(max <= continuous::FLOW_EDIT_LIMIT, "max {max}");
    s.advance(3);
    assert!(s.fields()[0].as_slice().iter().all(|v| v.is_finite()));
    assert!(s.metrics().get("mass_drift").is_some_and(|d| d.abs() < 1e-6));
    let nan = ContinuousCommand::Randomize {
        region: Region::Disc { x: 64.0, y: 64.0, radius: 5.0 },
        density: 1.0,
        amplitude: f32::NAN,
    };
    assert!(s.apply(nan).is_err());
}

#[test]
fn pending_sizes_are_checked_against_creatures_and_resource_limits() {
    // A world one column narrower than the glider fixture still fits the
    // kernel (2·21 + 1 = 43 cells) but not the creature: the change is
    // refused instead of making the next reset fail.
    let creature = creature_library().into_iter().find(|c| c.scenario_id == "smoothlife.glider").expect("glider");
    let narrow = creature.cells.width() - 1;
    assert!(narrow >= 43, "the fixture is {} cells wide", creature.cells.width());
    let mut glider = sim("smoothlife.glider");
    let err = glider.set_param("width", ParamValue::Int(narrow as i64));
    assert!(
        matches!(&err, Err(SimError::InvalidParameter { reason, .. }) if reason.contains("does not fit")),
        "{err:?}"
    );
    assert_eq!(glider.set_param("width", ParamValue::Int(64)), Ok(ApplyPolicy::RequiresReset));
    glider.reset();
    assert_eq!(glider.width(), 64);
    // Forced direct summation on a 4096² world exceeds the work limit.
    let mut soup = sim("smoothlife.soup");
    assert_eq!(soup.set_param("convolution", ParamValue::Text("direct".into())), Ok(ApplyPolicy::NextTick));
    soup.set_param("width", ParamValue::Int(4096)).expect("width alone is fine");
    let err = soup.set_param("height", ParamValue::Int(4096));
    assert!(matches!(&err, Err(SimError::InvalidParameter { reason, .. }) if reason.contains("FFT")), "{err:?}");
    // Switching back to the FFT makes the pending size acceptable.
    assert_eq!(soup.set_param("convolution", ParamValue::Text("auto".into())), Ok(ApplyPolicy::NextTick));
    assert_eq!(soup.set_param("height", ParamValue::Int(4096)), Ok(ApplyPolicy::RequiresReset));
    assert!(soup.set_param("convolution", ParamValue::Text("direct".into())).is_err());
}

#[test]
fn scenario_init_elements_are_validated_when_parsed() {
    let soup = BUILTIN_FILES.iter().find(|(name, _)| *name == "smoothlife-soup.ron").expect("file").1;
    let err = scenario::parse(&soup.replace("count: 74", "count: 1000000"), "discs.ron").expect_err("too many");
    assert!(err.to_string().contains("discs"), "{err}");
    let err = scenario::parse(&soup.replace("min_radius: 7.0", "min_radius: 30.0"), "radii.ron").expect_err("radii");
    assert!(err.to_string().contains("radii"), "{err}");
    let lenia = BUILTIN_FILES.iter().find(|(name, _)| *name == "lenia-soup.ron").expect("file").1;
    let err = scenario::parse(&lenia.replace("amplitude: 1.0", "amplitude: 2.0"), "amp.ron").expect_err("amplitude");
    assert!(err.to_string().contains("amplitude"), "{err}");
    let orbium = BUILTIN_FILES[0].1.replace("y: 0.5,\n        ),", "y: 0.5,\n            scale: 0.0,\n        ),");
    assert!(scenario::parse(&orbium, "scale.ron").is_err());
}

#[test]
fn runs_behind_the_threaded_runner_with_generic_controls() {
    let runner = Runner::spawn(sim("lenia.orbium"), RunnerConfig::default())
        .with_clear(|s| s.clear())
        .with_randomize(|s, seed| s.randomize(seed));
    let control: &dyn SessionControl = &runner;
    assert!(control.can_clear() && control.can_randomize());
    control.step(20);
    let probe = runner.query(|s| (s.tick(), s.mass()));
    let (tick, mass) = probe.recv_timeout(Duration::from_secs(60)).expect("runner answers");
    assert_eq!(tick, 20);
    assert!((mass - 73.0).abs() < 2.0, "mass {mass}");
    control.clear();
    let probe = runner.query(|s| s.mass());
    assert_eq!(probe.recv_timeout(Duration::from_secs(60)).expect("runner answers"), 0.0);
    control.randomize(9);
    let probe = runner.query(|s| s.mass());
    assert!(probe.recv_timeout(Duration::from_secs(60)).expect("runner answers") > 100.0);
    control.set_param("k0.mu", ParamValue::Float(0.16));
    let probe = runner.query(|s| s.params().f64_or("k0.mu", 0.0));
    assert_eq!(probe.recv_timeout(Duration::from_secs(60)).expect("runner answers"), 0.16);
    assert!(control.take_errors().is_empty());
}
