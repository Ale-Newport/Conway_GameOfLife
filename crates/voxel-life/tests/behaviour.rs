//! Model-level behaviour: the exact Life embedding, determinism, snapshots,
//! parameters, commands, views and metrics.

mod common;

use common::*;
use simulation_core::{ApplyPolicy, ParamValue, SimError, Simulation};
use voxel_life::{
    Axis, InitKind, Mirror, SliceRange, VoxelCommand, VoxelLife, VoxelView, VoxelViewRequest, WorldKind, faces,
};

/// Conway's B3/S23 on a bounded plane, computed directly.
fn life_2d(cells: &[Vec<bool>]) -> Vec<Vec<bool>> {
    let (h, w) = (cells.len() as i32, cells[0].len() as i32);
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| {
                    let mut n = 0;
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let (nx, ny) = (x + dx, y + dy);
                            if (dx, dy) != (0, 0) && nx >= 0 && ny >= 0 && nx < w && ny < h {
                                n += i32::from(cells[ny as usize][nx as usize]);
                            }
                        }
                    }
                    n == 3 || (n == 2 && cells[y as usize][x as usize])
                })
                .collect()
        })
        .collect()
}

fn plane(life: &VoxelLife, z: i32) -> Vec<Vec<bool>> {
    let [w, h, _] = life.config().size;
    (0..h as i32).map(|y| (0..w as i32).map(|x| life.state_at([x, y, z]) == 1).collect()).collect()
}

#[test]
fn conway_life_runs_exactly_inside_5766_on_two_planes() {
    let mut life = builtin("life-in-5766-two-planes");
    let mut expected = plane(&life, 0);
    assert_eq!(plane(&life, 1), expected, "the mirrored soup fills both planes identically");
    assert!(life.stats().population() > 1000);
    for generation in 1..=60 {
        expected = life_2d(&expected);
        life.step();
        assert_eq!(plane(&life, 0), expected, "plane 0, generation {generation}");
        assert_eq!(plane(&life, 1), expected, "plane 1, generation {generation}");
    }
}

#[test]
fn runs_are_deterministic_and_snapshots_replay() {
    let mut a = builtin("softology-pyroclastic");
    let mut b = builtin("softology-pyroclastic");
    a.advance(15);
    b.advance(15);
    assert_eq!(a.voxels(), b.voxels());
    let snapshot = a.snapshot().unwrap();
    a.advance(10);
    a.apply(VoxelCommand::Randomize { seed: None }).unwrap();
    a.advance(5);
    let later = a.voxels();
    let later_metrics = a.metrics();
    b.restore(&snapshot).unwrap();
    assert_eq!(b.tick(), 15);
    b.advance(10);
    b.apply(VoxelCommand::Randomize { seed: None }).unwrap();
    b.advance(5);
    assert_eq!(b.voxels(), later, "the restored run, including the RNG, replays identically");
    assert_eq!(b.metrics(), later_metrics);
}

#[test]
fn sparse_snapshots_round_trip_and_bad_snapshots_are_rejected() {
    let mut life = builtin("crystal-growth-1");
    life.advance(20);
    let snapshot = life.snapshot().unwrap();
    let mut other = builtin("rokicki-ship");
    other.restore(&snapshot).unwrap();
    assert_eq!(other.voxels(), life.voxels());
    assert_eq!(other.config(), life.config());
    other.step();
    life.step();
    assert_eq!(other.voxels(), life.voxels());

    let mut wrong = snapshot.clone();
    wrong.model_id = "life.hashlife".into();
    assert!(matches!(other.restore(&wrong), Err(SimError::WrongModel { .. })));
    let mut newer = snapshot.clone();
    newer.schema_version = 99;
    assert!(matches!(other.restore(&newer), Err(SimError::UnsupportedVersion { .. })));
    let mut corrupt = snapshot;
    corrupt.payload.truncate(corrupt.payload.len() / 2);
    assert!(other.restore(&corrupt).is_err());
    assert_eq!(other.tick(), 21, "a failed restore leaves the model unchanged");
}

#[test]
fn reset_restores_the_initial_state() {
    let mut life = builtin("softology-445");
    let initial = life.voxels();
    life.advance(12);
    life.apply(VoxelCommand::Clear).unwrap();
    life.reset();
    assert_eq!(life.tick(), 0);
    assert_eq!(life.voxels(), initial);
}

#[test]
fn rule_parameter_applies_next_tick_and_clears_missing_states() {
    let mut life = builtin("builder-1");
    life.advance(5);
    assert!(life.stats().counts().iter().skip(2).sum::<u64>() > 0, "dying voxels exist");
    let policy = life.set_param("rule", ParamValue::Text("B6/S567".into())).unwrap();
    assert_eq!(policy, ApplyPolicy::NextTick);
    assert_eq!(life.params().str_or("rule", ""), "5-7/6/2/M");
    assert!(life.voxels().iter().all(|v| v.state == 1), "states 2-9 no longer exist");
    assert_eq!(life.stats().counts().len(), 2);
    assert!(life.descriptor().rules.contains("Bays `5766`"));
    let error = life.set_param("rule", ParamValue::Text("4/27/2/M".into())).unwrap_err();
    assert!(error.to_string().contains("count 27"), "{error}");
    assert_eq!(life.rule().to_string(), "5-7/6/2/M", "a rejected rule changes nothing");
}

#[test]
fn world_parameters_wait_for_reset_and_are_validated() {
    let mut life = builtin("softology-445");
    assert_eq!(life.set_param("size_x", ParamValue::Int(32)).unwrap(), ApplyPolicy::RequiresReset);
    assert_eq!(life.config().size, [64, 64, 64], "unchanged until reset");
    assert_eq!(life.pending_config().size, [32, 64, 64]);
    assert_eq!(life.params().i64_or("size_x", 0), 32);
    let too_big = life.set_param("size_z", ParamValue::Int(65_536)).unwrap_err();
    assert!(too_big.to_string().contains("dense limit"), "{too_big}");
    life.set_param("world", ParamValue::Text("sparse_unbounded".into())).unwrap();
    life.set_param("size_z", ParamValue::Int(65_536)).unwrap();
    assert!(life.set_param("world", ParamValue::Text("dense_torus".into())).is_err());
    assert!(life.set_param("init", ParamValue::Text("pattern".into())).is_err(), "no pattern in this scenario");
    assert!(matches!(life.set_param("nope", ParamValue::Int(1)), Err(SimError::UnknownParameter(_))));
    assert!(life.set_param("density", ParamValue::Float(1.5)).is_err());
    life.set_param("init", ParamValue::Text("single_voxel".into())).unwrap();
    life.reset();
    assert_eq!(life.config().kind, WorldKind::SparseUnbounded);
    assert_eq!(life.voxels().iter().map(|v| v.pos).collect::<Vec<_>>(), vec![[16, 32, 32768]]);
}

#[test]
fn seeds_and_mirror_parameters_shape_the_random_cube() {
    let mut life = builtin("softology-amoeba");
    let first = life.voxels();
    life.set_param("seed", ParamValue::Int(8)).unwrap();
    life.reset();
    assert_ne!(life.voxels(), first, "a different seed draws a different cube");
    life.set_param("seed", ParamValue::Int(7)).unwrap();
    life.reset();
    assert_eq!(life.voxels(), first, "the same seed draws the same cube");
    life.set_param("mirror", ParamValue::Text(Mirror::Xyz.key().into())).unwrap();
    life.reset();
    let cells = positions(&life);
    // Cube of side 12 spans [26, 37] on each axis: reflection p → 63 − p.
    assert!(cells.iter().all(|p| cells.contains(&[63 - p[0], 63 - p[1], 63 - p[2]])));
    assert_eq!(life.pending_config().init.kind, InitKind::RandomCube);
}

#[test]
fn commands_are_validated() {
    let mut life = world("4/4/5/M", WorldKind::DenseBounded, [8, 8, 8]);
    let out = life.apply(VoxelCommand::SetVoxel { pos: [8, 0, 0], state: 1 }).unwrap_err();
    assert!(matches!(out, SimError::InvalidCommand(ref m) if m.contains("outside")), "{out}");
    let state = life.apply(VoxelCommand::SetVoxel { pos: [0, 0, 0], state: 5 }).unwrap_err();
    assert!(state.to_string().contains("state 5 does not exist"), "{state}");
    let batch = vec![([1, 1, 1], 1), ([2, 2, 2], 9)];
    assert!(life.apply(VoxelCommand::SetVoxels(batch)).is_err());
    assert_eq!(life.stats().population(), 0, "an invalid batch changes nothing");

    life.apply(VoxelCommand::FillBox { min: [6, 6, 6], max: [20, 7, -3], state: 3 }).unwrap();
    // Corners in any order; x 6..=7, y 6..=7, z 0..=6 after clipping.
    assert_eq!(life.stats().counts()[3], 2 * 2 * 7, "clipped to the world");
    let outside = life.apply(VoxelCommand::FillBox { min: [9, 9, 9], max: [12, 12, 12], state: 1 }).unwrap_err();
    assert!(outside.to_string().contains("does not overlap the world"), "{outside}");
    life.apply(VoxelCommand::ClearVoxel { pos: [6, 6, 0] }).unwrap();
    assert_eq!(life.stats().population(), 27);
    life.apply(VoxelCommand::Clear).unwrap();
    assert_eq!(life.stats().population(), 0);

    let mut a = builtin("softology-445");
    let mut b = builtin("softology-445");
    a.apply(VoxelCommand::Randomize { seed: Some(99) }).unwrap();
    b.apply(VoxelCommand::Randomize { seed: Some(99) }).unwrap();
    assert_eq!(a.voxels(), b.voxels());
    assert!(a.voxels().iter().all(|v| v.state == 1 && v.age == 0));
    let mut huge = world("5766", WorldKind::SparseBounded, [4096, 4096, 4096]);
    huge.set_param("cube_side", ParamValue::Int(200)).unwrap();
    huge.set_param("size_x", ParamValue::Int(512)).unwrap();
    let limit = huge.set_param("cube_side", ParamValue::Int(4096)).unwrap_err();
    assert!(limit.to_string().contains("random cube"), "{limit}");
}

#[test]
fn views_report_surface_slice_and_counts() {
    let mut life = world("5766", WorldKind::DenseBounded, [10, 10, 10]);
    life.apply(VoxelCommand::FillBox { min: [2, 2, 2], max: [6, 6, 6], state: 1 }).unwrap();
    let VoxelView::Surface(surface) = life.view(&VoxelViewRequest::default()) else { panic!("surface view") };
    assert_eq!(surface.visible, 98);
    assert!(!surface.truncated);
    assert!(surface.voxels.iter().all(|v| v.faces != 0 && v.pos != [4, 4, 4]));
    let cut = VoxelViewRequest::Surface { range: SliceRange::new(Axis::X, 4, 4), max_voxels: 7 };
    let VoxelView::Surface(cut) = life.view(&cut) else { panic!("surface view") };
    assert_eq!((cut.visible, cut.voxels.len(), cut.truncated), (25, 7, true));
    assert!(cut.voxels.iter().all(|v| v.faces & (faces::NEG_X | faces::POS_X) == faces::NEG_X | faces::POS_X));

    let VoxelView::Slice(slice) = life.view(&VoxelViewRequest::Slice { axis: Axis::Y, layer: 2, window: None }) else {
        panic!("slice view")
    };
    assert_eq!((slice.cells.width(), slice.cells.height()), (10, 10));
    assert_eq!(slice.cells.as_slice().iter().filter(|&&c| c == 1).count(), 25);
    assert_eq!(*slice.cells.get(3, 6), 1, "u = x, v = z for a slice across y");

    let VoxelView::Counts(counts) = life.view(&VoxelViewRequest::Counts) else { panic!("counts view") };
    assert_eq!(counts.counts, vec![1000 - 125, 125]);
    assert_eq!(counts.empty, Some(875));

    let unbounded = builtin("single-point-replication");
    let VoxelView::Counts(counts) = unbounded.view(&VoxelViewRequest::Counts) else { panic!("counts view") };
    assert_eq!((counts.empty, counts.population), (None, 1));
    let VoxelView::Slice(slice) = unbounded.view(&VoxelViewRequest::Slice { axis: Axis::Z, layer: 32, window: None })
    else {
        panic!("slice view")
    };
    assert_eq!((slice.window.u0, slice.window.v0, slice.window.width), (0, 0, 64));
    assert_eq!(*slice.cells.get(32, 32), 1);
}

#[test]
fn metrics_and_descriptor() {
    let mut life = builtin("life-5766-glider");
    let descriptor = life.descriptor();
    assert_eq!(descriptor.id, "voxel.life");
    for spec in &descriptor.params {
        assert_eq!(spec.validate(&spec.default).as_ref(), Ok(&spec.default), "default of {}", spec.key);
        let expected = if spec.key == "rule" { ApplyPolicy::NextTick } else { ApplyPolicy::RequiresReset };
        assert_eq!(spec.apply, expected, "{}", spec.key);
    }
    let metrics = life.metrics();
    let keys: Vec<&str> = metrics.keys().collect();
    let declared: Vec<&str> = descriptor.metrics.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, declared, "every declared metric is reported for a non-empty world");
    assert_eq!(metrics.get("population"), Some(10.0));
    assert_eq!(metrics.get("bbox_min_z"), Some(4.0));
    assert_eq!(metrics.get("bbox_max_z"), Some(5.0));
    assert_eq!(metrics.get("centroid_z"), Some(4.5));
    life.advance(4);
    assert_eq!(life.metrics().get("bbox_min_x"), Some(13.0), "moved one voxel along +x");
    life.apply(VoxelCommand::Clear).unwrap();
    assert_eq!(life.metrics().get("centroid_x"), None, "absent for an empty world");
    let multi = builtin("builder-1");
    assert!(multi.metrics().get("state_9").is_some());
    assert!(multi.descriptor().rules.contains("Decay"));
}

#[test]
fn built_in_scenarios_load_and_match_the_folder() {
    let builtin = voxel_life::scenarios::builtin().unwrap();
    assert_eq!(builtin.len(), 12);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/voxel");
    let folder = voxel_life::scenarios::load_dir(&dir).unwrap();
    assert_eq!(folder, builtin, "the embedded copies match scenarios/voxel");
    for scenario in builtin {
        let id = scenario.id.clone();
        let mut life = VoxelLife::new(scenario).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(life.stats().population() > 0, "{id} starts non-empty");
        life.advance(3);
        assert_eq!(life.tick(), 3, "{id}");
    }
}

#[test]
fn scenario_errors_are_reported() {
    use voxel_life::scenarios::parse;
    let base = include_str!("../../../scenarios/voxel/life-5766-glider.ron");
    let bad_rule = base.replace("\"5766\"", "\"5776\"");
    let error = parse(&bad_rule, "bad.ron").unwrap_err().to_string();
    assert!(error.contains("bad.ron") && error.contains("birth interval"), "{error}");
    let unknown = base.replace("seed: 1,", "seed: 1, colour: 3,");
    assert!(parse(&unknown, "x.ron").unwrap_err().to_string().contains("colour"));
    let far = base.replace("(2, 2, 1, 1)", "(40, 2, 1, 1)");
    assert!(parse(&far, "x.ron").unwrap_err().to_string().contains("outside the world"));
    let state = base.replace("(2, 2, 1, 1)", "(2, 2, 1, 2)");
    assert!(parse(&state, "x.ron").unwrap_err().to_string().contains("state 2"));
    let missing = std::path::Path::new("/nonexistent/voxel");
    assert!(voxel_life::scenarios::load_dir(missing).is_err());
    assert!(voxel_life::scenarios::builtin_by_id("nope").is_err());
}
