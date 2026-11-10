//! The running construction: the OTCA tiles (loaded from the asset folder
//! on a background thread), the [`Runner`] behind [`ActiveSession`], the
//! built-in scenarios, scenario loading with `Control::Replace`, and data
//! read from the simulation thread without blocking a frame.
//!
//! **Why a session wrapper.** `simulation_core::Runner` captures the model
//! descriptor when it is spawned, but the metapixel descriptor's rules text
//! is generated from the running rule, neighbourhood, grid size and step.
//! [`MetapixelSession`] delegates every control to the runner and serves a
//! descriptor that follows the model (read from the simulation thread),
//! with the few glyphs egui's fonts cannot draw replaced. Descriptors live
//! in write-once slots, so `descriptor()` hands out plain references; when
//! the slots run out the session is rebuilt from a snapshot.
//!
//! **Leaving the mode.** The runner's thread is joined when the last handle
//! drops; a Hashlife jump can take a moment on large grids, so the handles
//! are dropped on a helper thread and the window never waits for it.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use bevy::prelude::*;
use interface::{
    ActiveSession, AssetRoot, MetricsHistory, ModeScenarios, ScenarioEntry, ScenarioList, ScenarioSelected,
    SessionStatus, Toasts,
};
use metapixel::scenario::{self, Scenario};
use metapixel::spec::PERIOD;
use metapixel::{
    AmbiguousTile, BuildStats, MacroGrid, MetapixelCommand, MetapixelSim, MetapixelView, StepSize, TileSet, WorldConfig,
};
use simulation_core::{
    Control, HistoryConfig, Metrics, ModelDescriptor, ParamValue, Runner, RunnerConfig, RunnerStatus, SessionControl,
    SimError, Simulation, Snapshot, Speed,
};

use crate::MODE;
use crate::golden::GoldenTable;

/// Scenario opened the first time the mode is entered.
pub const DEFAULT_SCENARIO: &str = "metapixel.blinker";

/// Write-once descriptor slots per session.
pub const DESCRIPTOR_SLOTS: usize = 1024;

/// Readout records copied from the simulation thread (besides the first
/// eight, which the golden table needs).
pub const RECORDS_COPIED: usize = 240;

/// Target speed of a new session (ticks per second).
pub const DEFAULT_TICKS_PER_SECOND: f64 = 4.0;

/// Shortest interval between two model-data reads triggered by new views
/// (the read runs on the simulation thread, between ticks).
pub const INFO_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Density of random soups made by the bottom bar's Randomize (and the
/// tools panel), as `f32` bits shared with the runner's hook.
pub type SharedDensity = Arc<AtomicU32>;

/// Default density of random soups.
pub const DEFAULT_DENSITY: f32 = 0.4;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Takes the value of a pending reply, if it arrived.
fn take<T>(slot: &Mutex<Option<Receiver<T>>>) -> Option<T> {
    let mut guard = lock(slot);
    let rx = guard.as_ref()?;
    match rx.try_recv() {
        Ok(value) => {
            *guard = None;
            Some(value)
        }
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => {
            *guard = None;
            None
        }
    }
}

fn is_pending<T>(slot: &Mutex<Option<Receiver<T>>>) -> bool {
    lock(slot).is_some()
}

/// Built-in scenarios (embedded copies of `scenarios/metapixel/`).
#[derive(Resource, Clone, Debug, Default)]
pub struct Catalog {
    /// Scenarios in file order.
    pub scenarios: Vec<Scenario>,
    /// Why they could not be parsed, if so.
    pub error: Option<String>,
}

impl Catalog {
    /// Parses the embedded scenarios.
    pub fn builtin() -> Self {
        match scenario::builtin() {
            Ok(scenarios) => Self { scenarios, error: None },
            Err(e) => Self { scenarios: Vec::new(), error: Some(e.to_string()) },
        }
    }

    /// Scenario by id.
    pub fn get(&self, id: &str) -> Option<&Scenario> {
        self.scenarios.iter().find(|s| s.id == id)
    }

    /// Display name of a scenario id (the id when unknown).
    pub fn name_of<'a>(&'a self, id: &'a str) -> &'a str {
        self.get(id).map_or(id, |s| s.name.as_str())
    }

    /// Entries of the top-bar picker.
    pub fn entries(&self) -> Vec<ScenarioEntry> {
        self.scenarios
            .iter()
            .map(|s| ScenarioEntry {
                id: s.id.clone(),
                title: format!("{} · {}×{} · {}", s.name, s.width, s.height, s.rule),
                description: displayable(&s.description),
            })
            .collect()
    }
}

/// Where the OTCA tiles are read from.
pub fn tile_dir(asset_root: &std::path::Path) -> PathBuf {
    asset_root.join("patterns").join("otca")
}

/// State of the OTCA tiles.
#[derive(Default)]
pub enum TilesState {
    /// Not requested yet.
    #[default]
    Idle,
    /// Being read and validated on a background thread.
    Loading(Mutex<Receiver<Result<TileSet, String>>>),
    /// Validated tiles.
    Ready(Arc<TileSet>),
    /// The files are missing or invalid.
    Failed(String),
}

/// Brice Due's unprogrammed OFF and ON tiles, read at run time from
/// `<assets>/patterns/otca/` (GPL-2.0-or-later data, never embedded).
#[derive(Resource, Default)]
pub struct Tiles {
    /// Folder the tiles are read from.
    pub dir: PathBuf,
    /// Loading state.
    pub state: TilesState,
    /// Seconds the load took.
    pub load_seconds: Option<f64>,
    started: Option<Instant>,
}

impl Tiles {
    /// Starts reading the tiles on a background thread (once).
    pub fn start(&mut self, dir: PathBuf) {
        if !matches!(self.state, TilesState::Idle) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let path = dir.clone();
        let spawned = std::thread::Builder::new().name("otca-tiles".into()).spawn(move || {
            let _ = tx.send(TileSet::from_dir(&path).map_err(|e| e.to_string()));
        });
        self.dir = dir;
        self.started = Some(Instant::now());
        self.state = match spawned {
            Ok(_) => TilesState::Loading(Mutex::new(rx)),
            Err(e) => TilesState::Failed(format!("could not start the tile loader: {e}")),
        };
    }

    /// Picks up a finished load.
    pub fn poll(&mut self) {
        let TilesState::Loading(rx) = &self.state else { return };
        let result = match lock(rx).try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the tile loader stopped unexpectedly".into()),
        };
        self.load_seconds = self.started.map(|s| s.elapsed().as_secs_f64());
        self.state = match result {
            Ok(tiles) => TilesState::Ready(Arc::new(tiles)),
            Err(e) => TilesState::Failed(e),
        };
    }

    /// The tiles, once loaded.
    pub fn ready(&self) -> Option<&Arc<TileSet>> {
        match &self.state {
            TilesState::Ready(tiles) => Some(tiles),
            _ => None,
        }
    }

    /// Why the tiles are unavailable, if they failed.
    pub fn error(&self) -> Option<&str> {
        match &self.state {
            TilesState::Failed(e) => Some(e),
            _ => None,
        }
    }
}

/// `SessionControl` over the metapixel runner with a live descriptor.
pub struct MetapixelSession {
    runner: Runner<MetapixelSim>,
    slots: Box<[OnceLock<ModelDescriptor>]>,
    current: AtomicUsize,
}

/// What [`MetapixelSession::update_descriptor`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorUpdate {
    /// The new descriptor is served.
    Updated,
    /// It equals the current one.
    Unchanged,
    /// No slot is left: rebuild the session.
    Full,
}

impl MetapixelSession {
    /// Wraps a runner; its (display-ready) descriptor is served first.
    pub fn new(runner: Runner<MetapixelSim>) -> Self {
        let slots: Box<[OnceLock<ModelDescriptor>]> = (0..DESCRIPTOR_SLOTS).map(|_| OnceLock::new()).collect();
        let _ = slots[0].set(served_descriptor(runner.descriptor().clone()));
        Self { runner, slots, current: AtomicUsize::new(0) }
    }

    /// The typed runner.
    pub fn runner(&self) -> &Runner<MetapixelSim> {
        &self.runner
    }

    /// Sends a control message.
    pub fn send(&self, control: Control<MetapixelSim>) {
        self.runner.send(control);
    }

    /// The descriptor currently served.
    pub fn current_descriptor(&self) -> &ModelDescriptor {
        let i = self.current.load(Ordering::Acquire);
        self.slots.get(i).and_then(OnceLock::get).unwrap_or_else(|| self.runner.descriptor())
    }

    /// Serves the display-ready version of `descriptor` from now on.
    pub fn update_descriptor(&self, descriptor: ModelDescriptor) -> DescriptorUpdate {
        let descriptor = served_descriptor(descriptor);
        if *self.current_descriptor() == descriptor {
            return DescriptorUpdate::Unchanged;
        }
        let next = self.current.load(Ordering::Acquire) + 1;
        let Some(slot) = self.slots.get(next) else { return DescriptorUpdate::Full };
        if slot.set(descriptor).is_err() {
            return DescriptorUpdate::Full;
        }
        self.current.store(next, Ordering::Release);
        DescriptorUpdate::Updated
    }
}

impl SessionControl for MetapixelSession {
    fn descriptor(&self) -> &ModelDescriptor {
        self.current_descriptor()
    }
    fn status(&self) -> RunnerStatus {
        SessionControl::status(&self.runner)
    }
    fn play(&self) {
        SessionControl::play(&self.runner);
    }
    fn pause(&self) {
        SessionControl::pause(&self.runner);
    }
    fn toggle_play(&self) {
        SessionControl::toggle_play(&self.runner);
    }
    fn step(&self, n: u64) {
        SessionControl::step(&self.runner, n);
    }
    fn set_speed(&self, speed: Speed) {
        SessionControl::set_speed(&self.runner, speed);
    }
    fn reset(&self) {
        SessionControl::reset(&self.runner);
    }
    fn undo(&self) {
        SessionControl::undo(&self.runner);
    }
    fn redo(&self) {
        SessionControl::redo(&self.runner);
    }
    fn jump_to_history(&self, index: usize) {
        SessionControl::jump_to_history(&self.runner, index);
    }
    fn set_history(&self, config: HistoryConfig) {
        SessionControl::set_history(&self.runner, config);
    }
    fn set_param(&self, key: &str, value: ParamValue) {
        SessionControl::set_param(&self.runner, key, value);
    }
    fn drain_metrics(&self) -> Vec<Metrics> {
        SessionControl::drain_metrics(&self.runner)
    }
    fn take_errors(&self) -> Vec<String> {
        SessionControl::take_errors(&self.runner)
    }
    fn request_snapshot(&self) -> Receiver<Result<Snapshot, SimError>> {
        SessionControl::request_snapshot(&self.runner)
    }
    fn restore(&self, snapshot: Snapshot) {
        SessionControl::restore(&self.runner, snapshot);
    }
    fn can_clear(&self) -> bool {
        SessionControl::can_clear(&self.runner)
    }
    fn clear(&self) {
        SessionControl::clear(&self.runner);
    }
    fn can_randomize(&self) -> bool {
        SessionControl::can_randomize(&self.runner)
    }
    fn randomize(&self, seed: u64) {
        SessionControl::randomize(&self.runner, seed);
    }
}

/// Typed handle on the session (the same object as [`ActiveSession`]).
#[derive(Resource, Clone)]
pub struct MpSession(pub Arc<MetapixelSession>);

impl MpSession {
    /// Sends a control message.
    pub fn send(&self, control: Control<MetapixelSim>) {
        self.0.send(control);
    }

    /// Applies an undoable macro edit.
    pub fn edit(&self, command: MetapixelCommand) {
        self.0.send(Control::Edit(command));
    }
}

/// Characters of the model's texts that egui's built-in fonts cannot draw,
/// with replacements.
const GLYPH_FALLBACKS: &[(char, &str)] =
    &[('∈', "in"), ('∅', "{}"), ('⌊', "floor("), ('⌋', ")"), ('→', "->"), ('∧', "and"), ('∨', "or"), ('¬', "not ")];

/// `text` with the characters egui's fonts cannot draw replaced.
pub fn displayable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match GLYPH_FALLBACKS.iter().find(|(x, _)| *x == c) {
            Some((_, replacement)) => out.push_str(replacement),
            None => out.push(c),
        }
    }
    out
}

/// The descriptor as the shell shows it (texts made displayable; ids,
/// keys and values unchanged, so save / load still match).
pub fn served_descriptor(mut d: ModelDescriptor) -> ModelDescriptor {
    d.summary = displayable(&d.summary);
    d.rules = displayable(&d.rules);
    for p in &mut d.params {
        p.description = displayable(&p.description);
    }
    for m in &mut d.metrics {
        m.description = displayable(&m.description);
    }
    d
}

/// Spawns a runner for `sim` (history snapshots once per metageneration,
/// Clear and Randomize enabled).
pub fn spawn_session(sim: MetapixelSim, density: SharedDensity) -> Arc<MetapixelSession> {
    let config = RunnerConfig {
        speed: Speed::TicksPerSecond(DEFAULT_TICKS_PER_SECOND),
        history: HistoryConfig { interval: PERIOD, ..HistoryConfig::default() },
        ..RunnerConfig::default()
    };
    let runner = Runner::spawn(sim, config)
        .with_clear(|sim: &mut MetapixelSim| {
            // Clearing a valid grid cannot fail; an error would leave the
            // construction unchanged.
            let _ = sim.apply(MetapixelCommand::Clear);
        })
        .with_randomize(move |sim: &mut MetapixelSim, seed| {
            // The seed becomes the model's seed (applied by the rebuild),
            // then the soup is drawn from the model's generator.
            let seed = i64::try_from(seed & (i64::MAX as u64)).unwrap_or(0);
            if sim.set_param("seed", ParamValue::Int(seed)).is_ok() {
                sim.reset();
            }
            let density = f64::from(f32::from_bits(density.load(Ordering::Relaxed))).clamp(0.0, 1.0);
            let _ = sim.apply(MetapixelCommand::Randomize { density });
        });
    Arc::new(MetapixelSession::new(runner))
}

/// One readout record, summarised.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordSummary {
    /// Metageneration read.
    pub metagen: u64,
    /// Generation of the readout.
    pub generation: u64,
    /// ON tiles read.
    pub exact_on: u64,
    /// ON tiles predicted by the accelerated model.
    pub predicted_on: u64,
    /// Tiles whose reading differs from the prediction (ambiguous included).
    pub mismatches: usize,
    /// The first mismatching tiles.
    pub mismatch_tiles: Vec<(u32, u32)>,
    /// The ambiguous tiles (at most eight).
    pub ambiguous: Vec<AmbiguousTile>,
    /// Number of ambiguous tiles.
    pub ambiguous_count: usize,
    /// Core counts of every tile (grids of at most nine tiles).
    pub core_counts: Option<Vec<u64>>,
}

/// Model data that is not part of the view, read from the simulation
/// thread when the state changes.
#[derive(Clone, Debug)]
pub struct SimInfo {
    /// Generation the data was read at.
    pub tick: u64,
    /// The generation-0 macro pattern.
    pub initial: MacroGrid,
    /// Configuration of the running construction.
    pub active: WorldConfig,
    /// Configuration the next rebuild uses.
    pub pending: WorldConfig,
    /// Generations per tick.
    pub step: StepSize,
    /// Statistics of the last build.
    pub build: BuildStats,
    /// Readout records (the first eight and the latest ones), oldest first.
    pub records: Vec<RecordSummary>,
    /// Readouts since the build.
    pub total_readouts: u64,
    /// Mismatches summed over every readout.
    pub total_mismatches: u64,
    /// Ambiguous tiles summed over every readout.
    pub total_ambiguous: u64,
    /// Why the model halted, if it did.
    pub halt: Option<String>,
    /// Accelerated prediction of the next metageneration (not physical).
    pub next_prediction: Option<(u64, MacroGrid)>,
    /// The model's descriptor (rules text follows the configuration).
    pub descriptor: ModelDescriptor,
    /// Live cells now.
    pub population: u64,
    /// Live Hashlife nodes.
    pub node_count: usize,
    /// Hashlife memory in bytes.
    pub memory_bytes: usize,
    /// Hashlife node limit.
    pub node_limit: usize,
    /// Populations of the programmed OFF and ON tiles.
    pub programmed_populations: (usize, usize),
}

impl SimInfo {
    /// Reads the model (on the simulation thread).
    pub fn read(sim: &MetapixelSim) -> Self {
        let history = sim.history();
        let count = history.records().len();
        let small = sim.initial_pattern().len() <= 9;
        let records = history
            .records()
            .enumerate()
            .filter(|(i, r)| r.metagen < 8 || *i + RECORDS_COPIED >= count)
            .map(|(_, r)| {
                let (columns, rows) = (r.exact.width(), r.exact.height());
                RecordSummary {
                    metagen: r.metagen,
                    generation: r.generation,
                    exact_on: r.exact.population(),
                    predicted_on: r.accelerated.population(),
                    mismatches: r.mismatches.len(),
                    mismatch_tiles: r.mismatches.iter().take(8).copied().collect(),
                    ambiguous: r.ambiguous.iter().take(8).copied().collect(),
                    ambiguous_count: r.ambiguous.len(),
                    core_counts: small.then(|| {
                        let readings: Vec<_> =
                            (0..rows).flat_map(|row| (0..columns).map(move |col| (col, row))).collect();
                        crate::golden::core_counts_of(
                            &readings.iter().map(|&(c, rw)| r.reading(c, rw)).collect::<Vec<_>>(),
                        )
                    }),
                }
            })
            .collect();
        let next_metagen = history.latest().map_or(0, |r| r.metagen + 1);
        let programmed = sim.programmed_tiles();
        let universe = sim.universe();
        Self {
            tick: sim.tick(),
            initial: sim.initial_pattern().clone(),
            active: sim.active_config(),
            pending: sim.pending_config(),
            step: sim.step_size(),
            build: sim.build_stats(),
            records,
            total_readouts: history.total_readouts(),
            total_mismatches: history.total_mismatches(),
            total_ambiguous: history.total_ambiguous(),
            halt: sim.halt_reason().map(str::to_string),
            next_prediction: sim.accelerated_prediction(next_metagen).map(|g| (next_metagen, g)),
            descriptor: sim.descriptor(),
            population: universe.population(),
            node_count: universe.node_count(),
            memory_bytes: universe.memory_bytes(),
            node_limit: universe.node_limit(),
            programmed_populations: (
                programmed.cells(metapixel::TileState::Off).len(),
                programmed.cells(metapixel::TileState::On).len(),
            ),
        }
    }
}

/// Identity of a construction (macro pattern and programming): golden
/// values and logged populations belong to one.
pub fn construction_key(initial: &MacroGrid, config: &WorldConfig) -> u64 {
    let mut h = DefaultHasher::new();
    initial.to_rows().hash(&mut h);
    config.programming.hash(&mut h);
    h.finish()
}

/// Populations seen at readout times (`t = k·P`), from the model data and
/// the metric samples, for the golden checks.
#[derive(Clone, Debug, Default)]
pub struct PopulationLog {
    key: Option<u64>,
    /// Generation -> population.
    pub by_generation: BTreeMap<u64, u64>,
}

impl PopulationLog {
    /// Records a population of construction `key` at generation `t` (only
    /// multiples of P are kept). A different construction starts over.
    pub fn record(&mut self, key: u64, t: u64, population: u64) {
        if self.key != Some(key) {
            self.key = Some(key);
            self.by_generation.clear();
        }
        if t.is_multiple_of(PERIOD) {
            self.by_generation.insert(t, population);
        }
    }

    /// Forgets generations after `t` (the run went back in time).
    pub fn truncate_after(&mut self, t: u64) {
        self.by_generation.retain(|&g, _| g <= t);
    }
}

/// A requested jump to the next readout.
pub struct Jump {
    rx: Mutex<Receiver<Metrics>>,
    /// Generation of the readout it advances to.
    pub target: u64,
    /// When it was sent.
    pub started: Instant,
}

/// What the mode knows about the running simulation.
#[derive(Resource, Default)]
pub struct SimData {
    /// Latest published view.
    pub view: Option<Arc<MetapixelView>>,
    /// Publication counter of `view`.
    pub view_seq: u64,
    /// Model data (see [`SimInfo`]).
    pub info: Option<SimInfo>,
    /// Increments whenever `info` changes.
    pub info_rev: u64,
    /// Populations at readout times.
    pub populations: PopulationLog,
    /// The golden table.
    pub golden: Option<GoldenTable>,
    /// Increments with every restart (publication counters start over with
    /// a new simulation, so drawn-state keys include it).
    pub epoch: u64,
    /// A jump to the next readout in flight.
    pub jump: Option<Jump>,
    /// Seconds the last jump took.
    pub last_jump_seconds: Option<f64>,
    fingerprint: Option<u64>,
    info_rx: Mutex<Option<Receiver<SimInfo>>>,
    info_sent: Option<Instant>,
    info_wanted: bool,
    info_forced: bool,
    last_tick: Option<u64>,
}

impl SimData {
    /// Forgets everything tied to the previous simulation.
    pub fn restart(&mut self) {
        let golden = self.golden.take();
        let populations = std::mem::take(&mut self.populations);
        let epoch = self.epoch.wrapping_add(1);
        *self = Self { golden, populations, epoch, info_wanted: true, info_forced: true, ..Self::default() };
    }

    /// Asks for fresh model data on the next update.
    pub fn refresh_info(&mut self) {
        self.info_wanted = true;
        self.info_forced = true;
    }

    /// Whether a model-data read may be sent now: one is wanted, none is
    /// in flight, and (unless it was asked for explicitly) the previous one
    /// was sent at least [`INFO_MIN_INTERVAL`] ago, so a running simulation
    /// is not read at every published view.
    fn info_due(&self, now: Instant) -> bool {
        self.info_wanted
            && !is_pending(&self.info_rx)
            && (self.info_forced || self.info_sent.is_none_or(|sent| now.duration_since(sent) >= INFO_MIN_INTERVAL))
    }

    /// Generation of the latest view.
    pub fn tick(&self) -> u64 {
        self.view.as_ref().map_or(0, |v| v.phase.generation)
    }

    /// Whether a model-data read has been waiting for more than `secs`
    /// (the simulation thread is busy with a long Hashlife jump).
    pub fn busy_for(&self, secs: f64) -> Option<f64> {
        if !is_pending(&self.info_rx) {
            return None;
        }
        let waited = self.info_sent?.elapsed().as_secs_f64();
        (waited >= secs).then_some(waited)
    }
}

/// What a session is rebuilt from when its descriptor slots run out.
type RebuildParts = (Snapshot, WorldConfig, MacroGrid);

/// Lifecycle of the session: the scenario to build and builds in flight.
#[derive(Resource, Default)]
pub struct Lifecycle {
    /// Scenario to load as soon as the tiles are ready.
    pub wanted: Option<String>,
    /// Scenario being built.
    pub building: Option<String>,
    /// Problem of the last load.
    pub error: Option<String>,
    /// Shared density of random soups (bottom bar Randomize).
    pub density: Option<SharedDensity>,
    /// Start playing once the requested scenario is loaded.
    pub play_after_load: bool,
    /// Generation-0 pattern and configuration of the loaded scenario (to
    /// tell when the construction was edited since).
    pub baseline: Option<(MacroGrid, WorldConfig)>,
    build_rx: Mutex<Option<Receiver<Result<MetapixelSim, String>>>>,
    rebuild_rx: Mutex<Option<Receiver<Result<RebuildParts, String>>>>,
}

impl Lifecycle {
    /// The density shared with the runners' Randomize hook.
    pub fn density(&mut self) -> SharedDensity {
        self.density.get_or_insert_with(|| Arc::new(AtomicU32::new(DEFAULT_DENSITY.to_bits()))).clone()
    }

    /// Current random-soup density.
    pub fn density_value(&self) -> f32 {
        self.density.as_ref().map_or(DEFAULT_DENSITY, |d| f32::from_bits(d.load(Ordering::Relaxed)))
    }

    /// Sets the random-soup density.
    pub fn set_density(&mut self, value: f32) {
        self.density().store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Whether a build is in flight.
    pub fn is_building(&self) -> bool {
        is_pending(&self.build_rx) || is_pending(&self.rebuild_rx)
    }
}

/// Scenario ids waiting to be loaded (tools panel, tutorial actions).
#[derive(Resource, Default, Debug)]
pub struct LoadRequests(pub Vec<String>);

/// `Startup`: fills the top-bar picker and starts reading the tiles.
pub(crate) fn startup(
    catalog: Res<Catalog>,
    mut scenarios: ResMut<ModeScenarios>,
    root: Option<Res<AssetRoot>>,
    mut tiles: ResMut<Tiles>,
    mut data: ResMut<SimData>,
) {
    let selected = scenarios.get(MODE).and_then(|l| l.selected.clone()).or_else(|| Some(DEFAULT_SCENARIO.into()));
    scenarios.set(MODE, ScenarioList { entries: catalog.entries(), selected });
    let root = root.map_or_else(|| PathBuf::from("assets"), |r| r.0.clone());
    tiles.start(tile_dir(&root));
    data.golden = Some(GoldenTable::load(&root));
}

/// `OnEnter`: asks for the selected scenario.
pub(crate) fn enter_session(
    scenarios: Res<ModeScenarios>,
    mut lifecycle: ResMut<Lifecycle>,
    mut data: ResMut<SimData>,
    root: Option<Res<AssetRoot>>,
    mut tiles: ResMut<Tiles>,
) {
    let selected = scenarios.get(MODE).and_then(|l| l.selected.clone()).unwrap_or_else(|| DEFAULT_SCENARIO.into());
    lifecycle.wanted = Some(selected);
    lifecycle.error = None;
    data.restart();
    if matches!(tiles.state, TilesState::Idle) {
        let root = root.map_or_else(|| PathBuf::from("assets"), |r| r.0.clone());
        tiles.start(tile_dir(&root));
    }
}

/// `OnExit`: removes the session and drops it off the main thread.
pub(crate) fn exit_session(world: &mut World) {
    let active = world.remove_resource::<ActiveSession>();
    let typed = world.remove_resource::<MpSession>();
    world.resource_mut::<SimData>().restart();
    let mut lifecycle = world.resource_mut::<Lifecycle>();
    lifecycle.wanted = None;
    lifecycle.building = None;
    lifecycle.play_after_load = false;
    *lock(&lifecycle.build_rx) = None;
    *lock(&lifecycle.rebuild_rx) = None;
    if active.is_some() || typed.is_some() {
        drop_off_thread((active, typed));
    }
}

/// `Last`, when the application exits: the session is released on a
/// helper thread too, so quitting never waits for a Hashlife jump in
/// progress (the runner joins its worker when the last handle drops).
pub(crate) fn release_on_exit(mut exits: MessageReader<AppExit>, mut commands: Commands) {
    if exits.read().next().is_some() {
        commands.queue(|world: &mut World| {
            let handles = (world.remove_resource::<ActiveSession>(), world.remove_resource::<MpSession>());
            if handles.0.is_some() || handles.1.is_some() {
                drop_off_thread(handles);
            }
        });
    }
}

/// Queues scenarios picked in the top bar or requested by the panels.
pub(crate) fn collect_scenario_requests(
    mut picked: MessageReader<ScenarioSelected>,
    mut requests: ResMut<LoadRequests>,
    mut lifecycle: ResMut<Lifecycle>,
) {
    for pick in picked.read().filter(|p| p.mode == MODE) {
        requests.0.push(pick.id.clone());
    }
    if let Some(last) = requests.0.pop() {
        requests.0.clear();
        lifecycle.wanted = Some(last);
    }
}

/// Builds wanted scenarios on a background thread and installs them:
/// `Control::Replace` into the running session, or a new session.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_session(
    mut commands: Commands,
    mut tiles: ResMut<Tiles>,
    mut lifecycle: ResMut<Lifecycle>,
    catalog: Res<Catalog>,
    session: Option<Res<MpSession>>,
    mut data: ResMut<SimData>,
    mut scenarios: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    mut fit: ResMut<crate::world::CameraFit>,
) {
    tiles.poll();
    if let Some(error) = tiles.error() {
        if lifecycle.wanted.take().is_some() {
            let message = format!("The OTCA tiles could not be loaded: {error}");
            toasts.error(message.clone());
            lifecycle.error = Some(message);
        }
        return;
    }
    // Start a build when the tiles are ready.
    if let (Some(tiles), false) = (tiles.ready().cloned(), lifecycle.is_building())
        && let Some(id) = lifecycle.wanted.take()
    {
        match catalog.get(&id) {
            Some(scenario) => {
                let scenario = scenario.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                let spawned = std::thread::Builder::new().name("metapixel-build".into()).spawn(move || {
                    let _ = tx.send(MetapixelSim::from_scenario(tiles, &scenario).map_err(|e| e.to_string()));
                });
                match spawned {
                    Ok(_) => {
                        *lock(&lifecycle.build_rx) = Some(rx);
                        lifecycle.building = Some(id);
                    }
                    Err(e) => toasts.error(format!("could not start building `{id}`: {e}")),
                }
            }
            None => toasts.error(format!("Unknown metapixel scenario `{id}`.")),
        }
    }
    // A finished build (a scenario, or a session rebuilt from a snapshot).
    if let Some(result) = take(&lifecycle.build_rx) {
        let scenario = lifecycle.building.take();
        match (result, scenario) {
            (Ok(sim), Some(id)) => {
                lifecycle.baseline = Some((sim.initial_pattern().clone(), sim.active_config()));
                // The view published before the replacement belongs to the
                // previous construction: never show it again.
                let stale = session.as_ref().and_then(|s| s.0.runner().latest_view()).map(|p| p.seq);
                let target = match &session {
                    Some(session) => {
                        session.send(Control::Replace(Box::new(sim)));
                        session.0.clone()
                    }
                    None => {
                        let density = lifecycle.density();
                        install(&mut commands, sim, density)
                    }
                };
                if std::mem::take(&mut lifecycle.play_after_load) {
                    target.send(Control::Play);
                }
                scenarios.mark_selected(MODE, &id);
                lifecycle.error = None;
                data.restart();
                data.view_seq = stale.unwrap_or(0);
                fit.request(true);
            }
            (Ok(sim), None) => {
                let density = lifecycle.density();
                let _ = install(&mut commands, sim, density);
                data.restart();
            }
            (Err(e), Some(id)) => {
                lifecycle.play_after_load = false;
                let message = format!("Could not build `{}`: {e}", catalog.name_of(&id));
                toasts.error(message.clone());
                lifecycle.error = Some(message);
            }
            (Err(e), None) => toasts.error(format!("Could not rebuild the session: {e}")),
        }
    }
    // A snapshot taken to rebuild a session whose descriptor slots ran out.
    if let Some(result) = take(&lifecycle.rebuild_rx) {
        match (result, tiles.ready().cloned()) {
            (Ok((snapshot, config, initial)), Some(tiles)) => {
                let (tx, rx) = std::sync::mpsc::channel();
                let spawned = std::thread::Builder::new().name("metapixel-rebuild".into()).spawn(move || {
                    let rebuilt =
                        MetapixelSim::new(tiles, config, initial).map_err(|e| e.to_string()).and_then(|mut sim| {
                            sim.restore(&snapshot).map_err(|e| e.to_string())?;
                            Ok(sim)
                        });
                    let _ = tx.send(rebuilt);
                });
                match spawned {
                    Ok(_) => *lock(&lifecycle.build_rx) = Some(rx),
                    Err(e) => toasts.error(format!("Could not rebuild the session: {e}")),
                }
            }
            (Err(e), _) => toasts.error(format!("Could not rebuild the session: {e}")),
            (Ok(_), None) => toasts.error("Could not rebuild the session: the OTCA tiles are not loaded.".to_string()),
        }
    }
}

/// Installs a new session for `sim`, replacing (and dropping on a helper
/// thread) any previous one.
fn install(commands: &mut Commands, sim: MetapixelSim, density: SharedDensity) -> Arc<MetapixelSession> {
    let session = spawn_session(sim, density);
    let installed = session.clone();
    commands.queue(move |world: &mut World| {
        let old = (world.remove_resource::<ActiveSession>(), world.remove_resource::<MpSession>());
        world.insert_resource(ActiveSession::new(session.clone()));
        world.insert_resource(MpSession(session));
        drop_off_thread(old);
    });
    installed
}

/// Drops session handles on a helper thread (the runner joins its worker
/// when the last handle drops, which may wait for a Hashlife jump).
fn drop_off_thread<T: Send + 'static>(value: T) {
    let spawned = std::thread::Builder::new().name("metapixel-shutdown".into()).spawn(move || drop(value));
    if let Err(e) = spawned {
        warn!("could not drop the metapixel session on a helper thread: {e}");
    }
}

/// Hash of everything in a view and status that model data depends on.
fn fingerprint(view: &MetapixelView, status: Option<&RunnerStatus>) -> u64 {
    let mut h = DefaultHasher::new();
    view.phase.generation.hash(&mut h);
    view.phase.latest_readout.hash(&mut h);
    if let Some(m) = &view.macro_grid {
        (m.columns, m.rows, m.exact_metagen, m.mismatch_count).hash(&mut h);
        for t in &m.tiles {
            let exact = match t.exact {
                None => 0,
                Some(metapixel::TileReading::Off) => 1,
                Some(metapixel::TileReading::On) => 2,
                Some(metapixel::TileReading::Ambiguous(c)) => 3 + c,
            };
            (t.population, t.accelerated, exact).hash(&mut h);
        }
    }
    if let Some(status) = status {
        for (k, v) in status.params.iter() {
            k.hash(&mut h);
            format!("{v:?}").hash(&mut h);
        }
        status.finished.hash(&mut h);
    }
    h.finish()
}

/// Reads the latest view, keeps the model data fresh (requests run on the
/// simulation thread and are polled, never awaited) and follows the
/// model's descriptor.
#[allow(clippy::too_many_arguments)]
pub(crate) fn poll_data(
    session: Option<Res<MpSession>>,
    status: Res<SessionStatus>,
    mut data: ResMut<SimData>,
    mut history: ResMut<MetricsHistory>,
    lifecycle: Res<Lifecycle>,
    mut toasts: ResMut<Toasts>,
) {
    let Some(session) = session else { return };
    if let Some(published) = session.0.runner().latest_view().filter(|p| p.seq != data.view_seq) {
        data.view_seq = published.seq;
        let print = fingerprint(&published.view, status.get());
        if data.fingerprint != Some(print) {
            data.fingerprint = Some(print);
            data.info_wanted = true;
        }
        data.view = Some(published.view);
    }
    // The run went back in time (reset, undo, history jump, restore).
    let tick = data.tick();
    if data.last_tick.is_some_and(|last| tick < last) {
        data.populations.truncate_after(tick);
    }
    data.last_tick = Some(tick);

    let now = Instant::now();
    if data.info_due(now) {
        data.info_wanted = false;
        data.info_forced = false;
        let rx = session.0.runner().request(|sim| SimInfo::read(sim));
        *lock(&data.info_rx) = Some(rx);
        data.info_sent = Some(now);
    }
    if let Some(info) = take(&data.info_rx) {
        let key = construction_key(&info.initial, &info.active);
        let (t, population) = (info.tick, info.population);
        data.populations.record(key, t, population);
        // Metric samples at earlier readout times (every tick records one).
        // The metric store may already hold samples of a newer construction
        // (this data was read before an edit or a new run): back-fill only
        // when its sample at `t` is this construction's population.
        if let Some(series) = history.store().get("population")
            && series.value_at(t).is_some_and(|v| v as u64 == population)
        {
            for k in 0..=8u64 {
                let g = k * PERIOD;
                if g > t {
                    break;
                }
                if let Some(value) = series.value_at(g) {
                    data.populations.record(key, g, value as u64);
                }
            }
        }
        match session.0.update_descriptor(info.descriptor.clone()) {
            DescriptorUpdate::Full if !is_pending(&lifecycle.rebuild_rx) => {
                let rx = session.0.runner().request(|sim| {
                    sim.snapshot()
                        .map(|s| (s, sim.active_config(), sim.initial_pattern().clone()))
                        .map_err(|e| e.to_string())
                });
                *lock(&lifecycle.rebuild_rx) = Some(rx);
                toasts.info("Refreshing the session so the Rules tab follows the model.".to_string());
            }
            _ => {}
        }
        data.info = Some(info);
        data.info_rev += 1;
    }
    // A finished jump to the next readout: record its metric sample (the
    // runner only samples regular ticks) unless newer samples exist.
    let finished = data.jump.as_ref().and_then(|j| match lock(&j.rx).try_recv() {
        Ok(metrics) => Some(Some(metrics)),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(None),
    });
    if let Some(result) = finished {
        if let Some(jump) = data.jump.take() {
            data.last_jump_seconds = Some(jump.started.elapsed().as_secs_f64());
        }
        if let Some(metrics) = result
            && history.store().last_tick().is_none_or(|last| last < metrics.tick)
        {
            history.record(&[metrics]);
        }
        data.refresh_info();
    }
}

/// Advances exactly to the next readout (`t = (⌊t/P⌋ + 1)·P`) on the
/// simulation thread.
pub fn jump_to_next_readout(session: &MpSession, data: &mut SimData) {
    jump(session, data, crate::timing::next_readout);
}

/// Pauses and advances on the simulation thread to `goal(t)` (never
/// earlier than the current generation `t`), without blocking a frame;
/// [`SimData::jump`] tracks it until it is done. Ignored while another
/// jump is in flight.
pub fn jump(session: &MpSession, data: &mut SimData, goal: fn(u64) -> u64) {
    if data.jump.is_some() {
        return;
    }
    let target = goal(data.tick());
    let (tx, rx) = std::sync::mpsc::channel();
    session.send(Control::Pause);
    session.send(Control::Exec(Box::new(move |sim: &mut MetapixelSim| {
        let now = sim.tick();
        sim.advance(goal(now).saturating_sub(now));
        let _ = tx.send(sim.metrics());
    })));
    data.jump = Some(Jump { rx: Mutex::new(rx), target, started: Instant::now() });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_every_builtin_scenario() {
        let catalog = Catalog::builtin();
        assert!(catalog.error.is_none());
        let ids: Vec<&str> = catalog.scenarios.iter().map(|s| s.id.as_str()).collect();
        for id in ["metapixel.blinker", "metapixel.glider", "metapixel.galaxy", "metapixel.b0-single-tile"] {
            assert!(ids.contains(&id), "{id}");
        }
        assert!(catalog.get(DEFAULT_SCENARIO).is_some());
        let entries = catalog.entries();
        assert_eq!(entries.len(), catalog.scenarios.len());
        assert!(entries.iter().any(|e| e.title == "Kok's galaxy of metacells · 15×15 · B3/S23"));
        assert_eq!(catalog.name_of("nope"), "nope");
    }

    #[test]
    fn descriptors_are_made_displayable() {
        assert_eq!(displayable("N ∈ {3} and ⌊t/P⌋ → ∅"), "N in {3} and floor(t/P) -> {}");
        let mut d = ModelDescriptor::new("x", "X", simulation_core::ModelFamily::Cellular).rules("a ∈ b");
        d.summary = "⌊x⌋".into();
        let served = served_descriptor(d);
        assert_eq!((served.rules.as_str(), served.summary.as_str()), ("a in b", "floor(x)"));
        assert_eq!(served.id, "x", "ids are kept for save / load");
    }

    #[test]
    fn population_log_keeps_readout_times_of_one_construction() {
        let mut log = PopulationLog::default();
        log.record(1, 0, 10);
        log.record(1, 5, 11);
        log.record(1, PERIOD, 12);
        log.record(1, 2 * PERIOD, 13);
        assert_eq!(log.by_generation.len(), 3, "only multiples of P");
        log.truncate_after(PERIOD);
        assert_eq!(log.by_generation.keys().copied().collect::<Vec<_>>(), [0, PERIOD]);
        log.record(2, 0, 99);
        assert_eq!(log.by_generation.get(&0), Some(&99), "a new construction starts over");
        assert_eq!(log.by_generation.len(), 1);
    }

    #[test]
    fn construction_keys_distinguish_patterns_and_rules() {
        let a = MacroGrid::from_rows(&["O."]).unwrap();
        let b = MacroGrid::from_rows(&[".O"]).unwrap();
        let life = WorldConfig { width: 2, height: 1, ..WorldConfig::default() };
        let mut high = life;
        high.programming = metapixel::Programming::parse("B36/S23", "moore").unwrap();
        assert_eq!(construction_key(&a, &life), construction_key(&a, &life));
        assert_ne!(construction_key(&a, &life), construction_key(&b, &life));
        assert_ne!(construction_key(&a, &life), construction_key(&a, &high));
    }

    fn blinker() -> MetapixelSim {
        let tiles = Arc::new(TileSet::from_dir(&TileSet::source_tree_dir()).unwrap());
        MetapixelSim::new(tiles, WorldConfig::default(), MacroGrid::from_rows(&["...", "OOO", "..."]).unwrap()).unwrap()
    }

    /// Leaving the mode while running must not leak the runner's thread:
    /// dropping the last handle stops and joins the worker.
    #[test]
    fn dropping_a_running_session_stops_its_worker() {
        let session = spawn_session(blinker(), Arc::new(AtomicU32::new(DEFAULT_DENSITY.to_bits())));
        session.send(Control::SetSpeed(Speed::Unlimited));
        session.send(Control::Play);
        // Wait for a first tick, then drop while the next one runs.
        let start = Instant::now();
        while session.runner().status().tick == 0 {
            assert!(start.elapsed().as_secs() < 60, "the runner never ticked");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(session.runner().status().playing);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(session);
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(30)).expect("the worker stopped and was joined");
    }

    /// The descriptor served to the shell follows the model (rules text of
    /// the running rule), and identical descriptors use no slot.
    #[test]
    fn descriptor_follows_the_model() {
        let session = spawn_session(blinker(), Arc::new(AtomicU32::new(DEFAULT_DENSITY.to_bits())));
        assert!(session.descriptor().rules.contains("B3/S23"));
        let mut sim = blinker();
        sim.set_param("rule", ParamValue::Text("B36/S23".into())).unwrap();
        sim.reset();
        assert_eq!(session.update_descriptor(sim.descriptor()), DescriptorUpdate::Updated);
        assert!(session.descriptor().rules.contains("B36/S23"));
        assert_eq!(session.update_descriptor(sim.descriptor()), DescriptorUpdate::Unchanged);
        assert!(!session.descriptor().rules.contains('∈'), "displayable glyphs");
    }

    #[test]
    fn model_data_reports_records_and_the_next_prediction() {
        let mut sim = blinker();
        sim.advance(2 * PERIOD);
        let info = SimInfo::read(&sim);
        assert_eq!(info.records.len(), 2);
        assert_eq!(info.records[1].core_counts.as_deref(), Some(&[0, 29_795, 0, 0, 29_795, 0, 0, 29_795, 0][..]));
        assert_eq!((info.total_readouts, info.total_mismatches), (2, 0));
        let (n, grid) = info.next_prediction.unwrap();
        assert_eq!((n, grid.to_rows()), (2, vec!["...".to_string(), "OOO".into(), "...".into()]));
        assert_eq!(info.programmed_populations, (23_923, 66_206), "the shipped B3/S23 tiles");
    }

    /// A tick crossing a readout time reads the tiles there and then
    /// completes its step (what the Time card says).
    #[test]
    fn ticks_pause_at_readouts_without_ending_there() {
        let mut sim = blinker();
        sim.set_param("step", ParamValue::Text("1024".into())).unwrap();
        for _ in 0..crate::timing::ticks_to_next_readout(0, StepSize::Pow2(10)) {
            sim.step();
        }
        assert_eq!(sim.tick(), 35 * 1024);
        let record = sim.history().latest().expect("metagen 0 was read");
        assert_eq!((record.metagen, record.generation), (0, PERIOD));
    }

    /// The transition the tutorial jumps to really starts 224 generations
    /// after the lead-in: the dying cores of the meta-blinker still hold
    /// their steady counts there and are switching right after the window
    /// opens.
    #[test]
    fn the_watched_transition_follows_the_lead_in() {
        let mut sim = blinker();
        let lead_in = crate::timing::transition_lead_in(0);
        sim.advance(lead_in);
        let steady = sim.live_readings().unwrap();
        assert!(steady.iter().all(|r| !matches!(r, metapixel::LiveReading::Transitioning(_))), "{steady:?}");
        sim.advance(metapixel::spec::TRANSITION_END + 1 - (lead_in % PERIOD));
        let after = sim.live_readings().unwrap();
        // Metagen 1 (vertical) is on display once the window has closed.
        let on: Vec<bool> = after.iter().map(|r| matches!(r, metapixel::LiveReading::On)).collect();
        assert_eq!(on, [false, true, false, false, true, false, false, true, false]);
    }

    #[test]
    fn model_data_reads_are_throttled_unless_forced() {
        let mut data = SimData::default();
        let now = Instant::now();
        assert!(!data.info_due(now), "nothing wanted");
        data.refresh_info();
        assert!(data.info_due(now));
        // A read was sent; a new view wants another one right away.
        data.info_forced = false;
        data.info_sent = Some(now);
        assert!(!data.info_due(now), "too soon after the last read");
        assert!(data.info_due(now + INFO_MIN_INTERVAL));
        data.refresh_info();
        assert!(data.info_due(now), "explicit refreshes are not delayed");
    }

    #[test]
    fn density_is_shared_with_the_hook() {
        let mut lifecycle = Lifecycle::default();
        assert_eq!(lifecycle.density_value(), DEFAULT_DENSITY);
        let shared = lifecycle.density();
        lifecycle.set_density(0.25);
        assert_eq!(f32::from_bits(shared.load(Ordering::Relaxed)), 0.25);
        lifecycle.set_density(3.0);
        assert_eq!(lifecycle.density_value(), 1.0);
    }
}
