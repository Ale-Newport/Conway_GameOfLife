//! Operations that list live cells one by one stay within their limits
//! without listing anything to find out: the region tools count the region
//! first, engine switches check the population, and the age/activity tracker
//! counts an edited region before merging it and the population before
//! listing every live cell to start (turning it on, building the model, a
//! reset). A counting global allocator
//! measures what each refusal allocates; it also refuses any single
//! allocation above 4 GiB, so a regression aborts instead of exhausting the
//! machine. The tests of this binary run one at a time.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use cellular_automata::engine::MAX_SWITCH_CELLS;
use cellular_automata::geometry::MAX_EDIT_CELLS;
use cellular_automata::life::{LifeConfig, LifeMode, MAX_TRACKED_CELLS, PatternLibrary, PatternSource};
use cellular_automata::{
    CellRect, EngineChoice, EngineKind, HashlifeEngine, InitItem, LifeCommand, LifeEngine, LifeSim, MirrorAxis,
    Rotation, Symmetry, switch_engine,
};
use patterns::LifeRule;
use simulation_core::{ParamValue, SimError, Simulation, Topology};

/// Bytes currently allocated, and the most allocated at once since the last
/// [`peak`] started.
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Largest single allocation granted.
const MAX_ALLOCATION: usize = 4 << 30;
/// What a refusal may allocate: far below the 1 GiB of a list of 2^26 cells.
const SMALL: usize = 16 << 20;

struct Counting;

impl Counting {
    fn grew(bytes: usize) {
        let now = ALLOCATED.fetch_add(bytes, Ordering::Relaxed) + bytes;
        PEAK.fetch_max(now, Ordering::Relaxed);
    }

    fn shrank(bytes: usize) {
        ALLOCATED.fetch_sub(bytes, Ordering::Relaxed);
    }
}

// SAFETY: every call is forwarded unchanged to the system allocator (or
// fails with a null pointer, which callers must handle); only counters are
// updated besides.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > MAX_ALLOCATION {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's contract is passed on unchanged.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            Self::grew(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() > MAX_ALLOCATION {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's contract is passed on unchanged.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            Self::grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract is passed on unchanged.
        unsafe { System.dealloc(ptr, layout) };
        Self::shrank(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > MAX_ALLOCATION {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's contract is passed on unchanged.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            if new_size >= layout.size() {
                Self::grew(new_size - layout.size());
            } else {
                Self::shrank(layout.size() - new_size);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Serialises the tests, so that one test's allocations do not count in
/// another's measurement.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `f` and returns its result with the most bytes it had allocated at
/// once (beyond what was allocated before).
fn peak<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = ALLOCATED.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let result = f();
    (result, PEAK.load(Ordering::Relaxed) - before)
}

fn sim(config: LifeConfig) -> LifeSim {
    LifeSim::new(&config, 7, &PatternLibrary::builtin(), LifeMode::Classic).expect("valid configuration")
}

/// An empty torus (ages and activity tracked with `track`).
fn torus(width: u32, height: u32, track: bool) -> LifeSim {
    let config = LifeConfig {
        topology: Topology::Torus,
        width,
        height,
        init: vec![],
        track_ages: track,
        track_activity: track,
        ..LifeConfig::default()
    };
    sim(config)
}

/// Brings every cell of `region` (default: the whole world) to life.
fn fill(region: Option<CellRect>) -> LifeCommand {
    LifeCommand::Randomize { density: 1.0, seed: Some(1), region, symmetry: Symmetry::None }
}

/// The live cells of the Gosper gun at generation 0.
fn gun_cells() -> Vec<(i64, i64)> {
    let source = PatternSource::Library("gosper-glider-gun".into());
    PatternLibrary::builtin().load_source(&source).unwrap().live_cells().collect()
}

/// A Gosper gun after 2^40 generations on Hashlife: about 1.8 × 10^11 live
/// cells, far too many to list.
fn huge_gun() -> LifeSim {
    let gun = InitItem::Pattern {
        source: PatternSource::Library("gosper-glider-gun".into()),
        x: 0,
        y: 0,
        rotation: Rotation::None,
        mirror: false,
    };
    let config = LifeConfig {
        topology: Topology::Infinite,
        init: vec![gun],
        generations_per_tick: 1 << 30,
        ..LifeConfig::default()
    };
    let mut s = sim(config);
    s.advance(1 << 10);
    assert_eq!((s.engine_kind(), s.tick()), (EngineKind::Hashlife, 1 << 40));
    s
}

fn limit_exceeded(result: Result<(), SimError>, limit: u64) -> bool {
    matches!(result, Err(SimError::LimitExceeded(m)) if m.contains(&limit.to_string()))
}

/// The `track_ages` and `track_activity` parameters.
fn tracked(s: &LifeSim) -> [Option<bool>; 2] {
    ["track_ages", "track_activity"].map(|k| s.params().get(k).and_then(ParamValue::as_bool))
}

/// The description of parameter `key`, as the user interface shows it.
fn description(s: &LifeSim, key: &str) -> String {
    s.descriptor().params.into_iter().find(|p| p.key == key).expect("known parameter").description
}

/// A full 8,192 × 8,193 rectangle on the infinite plane (2^26 + 8,192 live
/// cells, filled as the scenario starts) on `engine`, tracked with `track`.
fn full_rectangle(engine: EngineChoice, track: bool) -> LifeConfig {
    let region = Some(CellRect::new(0, 0, 8191, 8192));
    LifeConfig {
        topology: Topology::Infinite,
        init: vec![InitItem::Random { region, density: 1.0, symmetry: Symmetry::None }],
        engine,
        track_ages: track,
        track_activity: track,
        ..LifeConfig::default()
    }
}

#[test]
fn region_tools_refuse_more_live_cells_than_the_edit_limit_without_listing_them() {
    let _serial = serial();
    // Every cell of an 8,192 × 8,193 torus alive: one row more than the limit.
    let mut s = torus(8192, 8193, false);
    s.apply(fill(None)).unwrap();
    let population = s.population();
    assert_eq!(population, MAX_EDIT_CELLS + 8192);
    let world = CellRect::new(0, 0, 8191, 8192);
    for command in [
        LifeCommand::MoveRegion { rect: world, dx: 1, dy: 0 },
        LifeCommand::RotateRegion { rect: world, rotation: Rotation::Half },
        LifeCommand::MirrorRegion { rect: world, axis: MirrorAxis::Horizontal },
    ] {
        let (result, bytes) = peak(|| s.apply(command.clone()));
        assert!(limit_exceeded(result, MAX_EDIT_CELLS), "{command:?} is refused");
        assert!(bytes < SMALL, "{command:?} allocated {bytes} bytes to refuse");
    }
    let (result, bytes) = peak(|| s.cut_region(world).map(|_| ()));
    assert!(limit_exceeded(result, MAX_EDIT_CELLS), "copy_region refuses, so nothing is cleared");
    assert!(bytes < SMALL, "the cut allocated {bytes} bytes to refuse");
    assert_eq!(s.population(), population);
    // The same on Hashlife, counted from cached quadtree populations.
    let mut gun = huge_gun();
    let bbox = gun.engine().bounding_box().expect("live cells");
    let (result, bytes) = peak(|| gun.apply(LifeCommand::MoveRegion { rect: bbox, dx: 1, dy: 0 }));
    assert!(limit_exceeded(result, MAX_EDIT_CELLS) && bytes < SMALL, "{bytes} bytes");
}

#[test]
fn engine_switches_refuse_more_live_cells_than_the_switch_limit_without_listing_them() {
    let _serial = serial();
    let mut s = huge_gun();
    assert!(description(&s, "engine").contains(&MAX_SWITCH_CELLS.to_string()), "the user interface states the limit");
    let population = s.population();
    for engine in ["sparse", "dense"] {
        let (result, bytes) = peak(|| s.set_param("engine", ParamValue::Text(engine.into())));
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("copies every live cell") && message.contains(&MAX_SWITCH_CELLS.to_string()),
            "{message}"
        );
        assert!(bytes < SMALL, "the switch to {engine} allocated {bytes} bytes to refuse");
        assert_eq!(
            (s.engine_kind(), s.engine_choice(), s.population()),
            (EngineKind::Hashlife, EngineChoice::Auto, population)
        );
    }
    // The engine-level API refuses too.
    let mut engine = HashlifeEngine::new(LifeRule::conway()).unwrap();
    engine.set_cells(&gun_cells(), true);
    engine.advance(1 << 40).unwrap();
    let (result, bytes) = peak(|| switch_engine(Box::new(engine), EngineKind::Sparse).map(|_| ()));
    assert!(matches!(result, Err(e) if e.to_string().contains("copies every live cell")) && bytes < SMALL);
    // For a large power-of-two tick the automatic choice would move a large
    // sparse pattern to Hashlife; above the limit it keeps the sparse engine.
    let config = LifeConfig {
        topology: Topology::Infinite,
        init: vec![],
        engine: EngineChoice::Sparse,
        generations_per_tick: 64,
        ..LifeConfig::default()
    };
    let mut sparse = sim(config);
    sparse.apply(fill(Some(CellRect::new(0, 0, 8191, 8192)))).unwrap();
    assert!(sparse.population() > MAX_SWITCH_CELLS);
    let (result, bytes) = peak(|| sparse.set_param("engine", ParamValue::Text("auto".into())));
    result.unwrap();
    assert!(bytes < SMALL, "the automatic choice allocated {bytes} bytes");
    assert_eq!((sparse.engine_kind(), sparse.engine_choice()), (EngineKind::Sparse, EngineChoice::Auto));
}

#[test]
fn tracking_stops_instead_of_listing_an_edited_region_above_the_edit_limit() {
    let _serial = serial();
    let mut s = torus(8192, 8193, true);
    let (result, bytes) = peak(|| s.apply(fill(None)));
    result.unwrap();
    assert_eq!(s.population(), MAX_EDIT_CELLS + 8192, "the edit is applied");
    assert!(bytes < SMALL, "the fill allocated {bytes} bytes");
    assert_eq!(tracked(&s), [Some(false); 2], "tracking stopped");
    assert!(s.tracking_off_reason().is_some_and(|r| r.contains(&MAX_EDIT_CELLS.to_string())));
    // Edits of regions within the limit keep tracking.
    let mut t = torus(256, 256, true);
    t.apply(fill(Some(CellRect::new(0, 0, 255, 99)))).unwrap();
    assert_eq!(tracked(&t), [Some(true); 2]);
    assert_eq!((t.population(), t.age(5, 5), t.activity(5, 5)), (256 * 100, 1, 255));
    assert_eq!(t.tracking_off_reason(), None);
}

#[test]
fn tracking_does_not_start_above_the_tracking_limit_without_listing() {
    let _serial = serial();
    // Turning tracking on would list the gun's 1.8 × 10^11 live cells.
    let mut gun = huge_gun();
    let population = gun.population().to_string();
    for key in ["track_ages", "track_activity"] {
        let text = description(&gun, key);
        let limits = [format!("above {MAX_TRACKED_CELLS} live cells"), format!("more than {MAX_EDIT_CELLS} in")];
        assert!(limits.iter().all(|limit| text.contains(limit.as_str())), "{text}");
        let (result, bytes) = peak(|| gun.set_param(key, ParamValue::Bool(true)).map(|_| ()));
        let message = result.unwrap_err().to_string();
        assert!(message.contains(&MAX_TRACKED_CELLS.to_string()) && message.contains(&population), "{message}");
        assert!(bytes < SMALL, "turning {key} on allocated {bytes} bytes to refuse");
        assert_eq!(tracked(&gun), [Some(false); 2]);
        assert!(gun.tracking_off_reason().is_some_and(|r| message.ends_with(r)));
    }
    gun.set_param("track_ages", ParamValue::Bool(false)).unwrap();
    assert_eq!(gun.tracking_off_reason(), None, "setting a tracking parameter clears the reason");
    // A model configured with tracking and built with more live cells starts
    // without it, allocating no more than without tracking.
    let (untracked, plain) = peak(|| sim(full_rectangle(EngineChoice::Sparse, false)));
    drop(untracked);
    let (s, bytes) = peak(|| sim(full_rectangle(EngineChoice::Sparse, true)));
    assert_eq!(s.population(), MAX_TRACKED_CELLS + 8192);
    assert!(bytes < plain + SMALL, "building with tracking allocated {bytes} bytes, {plain} without");
    assert_eq!(tracked(&s), [Some(false); 2]);
    assert!(s.tracking_off_reason().is_some_and(|r| r.contains(&MAX_TRACKED_CELLS.to_string())));
}

#[test]
fn resets_stop_tracking_above_the_tracking_limit_without_listing() {
    let _serial = serial();
    // Return to a reset point of 2^26 + 8,192 live cells (a macrocell file on
    // Hashlife): one generation empties the rectangle but for a frame of
    // births around it, which tracking can list.
    let mut s = sim(full_rectangle(EngineChoice::Hashlife, false));
    s.step();
    assert!(s.population() < 1 << 16);
    s.set_param("track_ages", ParamValue::Bool(true)).unwrap();
    s.set_param("track_activity", ParamValue::Bool(true)).unwrap();
    let (_, bytes) = peak(|| s.reset());
    assert_eq!((s.tick(), s.population(), s.halt_reason()), (0, MAX_TRACKED_CELLS + 8192, None));
    assert!(bytes < SMALL, "the reset allocated {bytes} bytes");
    assert_eq!(tracked(&s), [Some(false); 2]);
    assert!(s.tracking_off_reason().is_some_and(|r| r.contains(&MAX_TRACKED_CELLS.to_string())));
    // After a world change the reset rebuilds the scenario instead (a
    // streamed random fill, here on the sparse engine): compared with the
    // same rebuild untracked.
    let mut s = sim(full_rectangle(EngineChoice::Sparse, false));
    let rebuild = |s: &mut LifeSim, width: i64, track: bool| {
        s.apply(LifeCommand::Clear).unwrap();
        s.set_param("track_ages", ParamValue::Bool(track)).unwrap();
        s.set_param("width", ParamValue::Int(width)).unwrap();
        peak(|| s.reset()).1
    };
    let plain = rebuild(&mut s, 300, false);
    let bytes = rebuild(&mut s, 301, true);
    assert_eq!((s.world().width, s.population(), s.halt_reason()), (301, MAX_TRACKED_CELLS + 8192, None));
    assert!(bytes < plain + SMALL, "the tracked rebuild allocated {bytes} bytes, {plain} without");
    assert_eq!(tracked(&s), [Some(false); 2]);
    assert!(s.tracking_off_reason().is_some_and(|r| r.contains(&MAX_TRACKED_CELLS.to_string())));
}
