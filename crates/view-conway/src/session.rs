//! The running simulation: the Classic Conway scenarios of the
//! `cellular-automata` crate, the [`Runner`] behind [`ActiveSession`] with a
//! live descriptor, scenario and pattern loading, and a periodic read of the
//! live world (engine, topology, exact generation and population, bounding
//! box, Hashlife memo statistics).
//!
//! **Why a session wrapper.** `simulation_core::Runner` captures the model
//! descriptor when it is spawned, but `LifeSim`'s rules text depends on the
//! running topology and generations per tick. [`ConwaySession`] serves a
//! descriptor that follows the simulation thread, so the shell's Rules tab
//! always describes what runs. Descriptors live in write-once slots (equal
//! descriptors reuse their slot), so `descriptor()` hands out plain
//! references without locks. The served descriptor omits the rule preset,
//! the rule text and the engine parameters: this mode runs B3/S23 only (the
//! Cellular Automata Lab runs other rules) and switches engines from its own
//! Engine card, which refuses switches that would copy astronomically many
//! cells out of Hashlife.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use cellular_automata::engine::GenStats;
use cellular_automata::{
    CellRect, EngineChoice, EngineKind, HashlifeEngine, InitItem, LifeCommand, LifeConfig, LifeMode, LifeSim,
    PatternSource, RulePreset, Scenario, ScenarioModel, Symmetry, builtin_scenarios,
};
use interface::{
    ActiveSession, MetricsHistory, ModeScenarios, ScenarioEntry, ScenarioList, ScenarioSelected, SessionStatus, Toasts,
};
use patterns::LifeRule;
use simulation_core::{
    Control, HistoryConfig, Metrics, ModelDescriptor, ParamSet, ParamValue, Runner, RunnerConfig, RunnerStatus,
    SessionControl, SimError, Simulation, Snapshot, Speed, Topology,
};

use crate::MODE;
use crate::labels::displayable;
use crate::library::ConwayLibrary;

/// Scenario opened the first time the mode is entered.
pub const DEFAULT_SCENARIO: &str = "glider-gun-showcase";
/// Write-once descriptor slots per session.
pub const DESCRIPTOR_SLOTS: usize = 1024;
/// Seconds between two reads of the live world while nothing changes.
pub const LIVE_PERIOD: f32 = 0.2;
/// Parameters of `LifeSim` that this mode does not offer in the shell's
/// Parameters tab (see the module docs).
pub const HIDDEN_PARAMS: [&str; 3] = ["preset", "rule", "engine"];
/// Default target speed.
pub const DEFAULT_SPEED: Speed = Speed::TicksPerSecond(30.0);

/// The Classic Conway scenarios embedded in `cellular-automata`.
#[derive(Resource, Clone, Debug)]
pub struct ConwayScenarios(pub Vec<Scenario>);

impl ConwayScenarios {
    /// Parses the embedded scenarios and keeps the `Classic` ones.
    pub fn builtin() -> Self {
        let all = builtin_scenarios().unwrap_or_else(|e| {
            error!("embedded scenarios do not parse: {e}");
            Vec::new()
        });
        Self(all.into_iter().filter(|s| matches!(s.model, ScenarioModel::Classic(_))).collect())
    }

    /// Scenario by id.
    pub fn get(&self, id: &str) -> Option<&Scenario> {
        self.0.iter().find(|s| s.id == id)
    }

    /// Entries for the top-bar picker.
    pub fn entries(&self) -> Vec<ScenarioEntry> {
        self.0
            .iter()
            .map(|s| ScenarioEntry { id: s.id.clone(), title: s.name.clone(), description: s.description.clone() })
            .collect()
    }
}

/// Builds a Classic Conway scenario with `library` resolving pattern ids.
pub fn build_scenario(scenario: &Scenario, library: &ConwayLibrary) -> Result<LifeSim, String> {
    match &scenario.model {
        ScenarioModel::Classic(config) => {
            LifeSim::new(config, scenario.seed, &library.resolver, LifeMode::Classic).map_err(|e| e.to_string())
        }
        _ => Err(format!("`{}` is not a Classic Conway scenario", scenario.id)),
    }
}

/// An empty infinite plane with one library pattern whose bounding box
/// starts at the origin (used by tutorial steps and "Open in a new plane").
pub fn pattern_world(id: &str, library: &ConwayLibrary) -> Result<LifeSim, String> {
    let config = LifeConfig {
        topology: Topology::Infinite,
        width: 128,
        height: 96,
        init: vec![InitItem::Pattern {
            source: PatternSource::Library(id.to_string()),
            x: 0,
            y: 0,
            rotation: Default::default(),
            mirror: false,
        }],
        ..LifeConfig::default()
    };
    LifeSim::new(&config, 1, &library.resolver, LifeMode::Classic).map_err(|e| e.to_string())
}

/// Summary served instead of the model's (which advertises every Life-like
/// rule; this mode runs B3/S23 only).
pub const SERVED_SUMMARY: &str = "Conway's Game of Life (B3/S23) on bounded, toroidal, Klein-bottle, cross-surface \
                                  or infinite worlds, with dense, bit-packed, sparse and Hashlife engines. Other \
                                  Life-like rules run in the Cellular Automata Lab.";

/// The model's rules text quotes the current generations per tick ("One
/// tick advances 64 generation(s)."). The served text names the setting
/// instead, so the descriptor only changes with the topology or the rule
/// and the write-once slots cannot run out when the value is scrubbed.
pub fn generic_ticks_sentence(rules: &str) -> String {
    const START: &str = "One tick advances ";
    const END: &str = " generation(s).";
    let Some(a) = rules.find(START) else { return rules.to_string() };
    let rest = &rules[a + START.len()..];
    match rest.find(END) {
        Some(b) if rest[..b].chars().all(|c| c.is_ascii_digit()) && b > 0 => format!(
            "{}One tick advances the number of generations set in *Generations per tick* (Tools › Time).{}",
            &rules[..a],
            &rest[b + END.len()..]
        ),
        _ => rules.to_string(),
    }
}

/// What `descriptor()` serves for a model descriptor: hidden parameters
/// removed, a Classic Conway summary, a rules text that does not depend on
/// the generations per tick, and every text drawable by egui's built-in fonts.
pub fn served_descriptor(descriptor: &ModelDescriptor) -> ModelDescriptor {
    let mut d = descriptor.clone();
    d.params.retain(|p| !HIDDEN_PARAMS.contains(&p.key.as_str()));
    d.summary = SERVED_SUMMARY.to_string();
    d.rules = displayable(&generic_ticks_sentence(&d.rules));
    for spec in &mut d.params {
        spec.description = displayable(&spec.description);
        spec.label = displayable(&spec.label);
    }
    for metric in &mut d.metrics {
        metric.description = displayable(&metric.description);
    }
    d
}

/// `SessionControl` over the Life runner with a live descriptor.
pub struct ConwaySession {
    runner: Runner<LifeSim>,
    slots: Box<[OnceLock<ModelDescriptor>]>,
    used: AtomicUsize,
    current: AtomicUsize,
}

impl ConwaySession {
    /// Wraps a runner; `first` is served until [`Self::update_descriptor`].
    pub fn new(runner: Runner<LifeSim>, first: ModelDescriptor) -> Self {
        let slots: Box<[OnceLock<ModelDescriptor>]> = (0..DESCRIPTOR_SLOTS).map(|_| OnceLock::new()).collect();
        let _ = slots[0].set(first);
        Self { runner, slots, used: AtomicUsize::new(1), current: AtomicUsize::new(0) }
    }

    /// The typed runner.
    pub fn runner(&self) -> &Runner<LifeSim> {
        &self.runner
    }

    /// The descriptor currently served.
    pub fn current_descriptor(&self) -> &ModelDescriptor {
        let i = self.current.load(Ordering::Acquire);
        self.slots.get(i).and_then(OnceLock::get).unwrap_or_else(|| self.runner.descriptor())
    }

    /// Serves `descriptor` from now on. Returns `false` when every slot is
    /// used by other descriptors (the previous one stays served).
    pub fn update_descriptor(&self, descriptor: ModelDescriptor) -> bool {
        if *self.current_descriptor() == descriptor {
            return true;
        }
        let used = self.used.load(Ordering::Acquire);
        if let Some(i) = (0..used).find(|&i| self.slots[i].get() == Some(&descriptor)) {
            self.current.store(i, Ordering::Release);
            return true;
        }
        let Some(slot) = self.slots.get(used) else { return false };
        if slot.set(descriptor).is_err() {
            return false;
        }
        self.used.store(used + 1, Ordering::Release);
        self.current.store(used, Ordering::Release);
        true
    }
}

impl SessionControl for ConwaySession {
    fn descriptor(&self) -> &ModelDescriptor {
        self.current_descriptor()
    }
    fn status(&self) -> RunnerStatus {
        self.runner.status()
    }
    fn play(&self) {
        self.runner.play();
    }
    fn pause(&self) {
        self.runner.pause();
    }
    fn toggle_play(&self) {
        SessionControl::toggle_play(&self.runner);
    }
    fn step(&self, n: u64) {
        self.runner.step(n);
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
        self.runner.drain_metrics()
    }
    fn take_errors(&self) -> Vec<String> {
        self.runner.take_errors()
    }
    fn request_snapshot(&self) -> Receiver<Result<Snapshot, SimError>> {
        SessionControl::request_snapshot(&self.runner)
    }
    fn restore(&self, snapshot: Snapshot) {
        SessionControl::restore(&self.runner, snapshot);
    }
    fn can_clear(&self) -> bool {
        self.runner.can_clear()
    }
    fn clear(&self) {
        SessionControl::clear(&self.runner);
    }
    fn can_randomize(&self) -> bool {
        self.runner.can_randomize()
    }
    fn randomize(&self, seed: u64) {
        SessionControl::randomize(&self.runner, seed);
    }
}

/// Typed handle on the running session (the same object as [`ActiveSession`]).
#[derive(Resource, Clone)]
pub struct ConwayRunner(pub Arc<ConwaySession>);

impl ConwayRunner {
    /// Sends a control message.
    pub fn send(&self, control: Control<LifeSim>) {
        self.0.runner().send(control);
    }

    /// Applies an undoable command.
    pub fn edit(&self, command: LifeCommand) {
        self.send(Control::Edit(command));
    }

    /// Applies a command without recording an undo step.
    pub fn command(&self, command: LifeCommand) {
        self.send(Control::Command(command));
    }

    /// Runs `f` on the simulation thread; poll the receiver with `try_recv`.
    pub fn request<R: Send + 'static>(&self, f: impl FnOnce(&mut LifeSim) -> R + Send + 'static) -> Receiver<R> {
        self.0.runner().request(f)
    }

    /// Sets a parameter.
    pub fn set_param(&self, key: &str, value: ParamValue) {
        self.send(Control::SetParam(key.to_string(), value));
    }
}

/// Density used by the shell's Randomize button (shared with the random
/// fill tool; `f64` bits).
#[derive(Resource, Clone, Debug)]
pub struct RandomizeDensity(pub Arc<AtomicU64>);

impl Default for RandomizeDensity {
    fn default() -> Self {
        Self(Arc::new(AtomicU64::new(0.3f64.to_bits())))
    }
}

impl RandomizeDensity {
    /// The density.
    pub fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }

    /// Sets the density (clamped to `[0, 1]`).
    pub fn set(&self, density: f64) {
        self.0.store(density.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

/// Keeps the shell's Randomize density equal to the Fill tool's density.
pub(crate) fn sync_randomize_density(tools: Res<crate::tools::ToolSettings>, density: Res<RandomizeDensity>) {
    if tools.is_changed() {
        density.set(tools.fill_density);
    }
}

/// A runner with Clear and Randomize (whole world, or the home region of the
/// infinite plane, with the random fill tool's density) enabled.
pub fn spawn_session(sim: LifeSim, speed: Speed, density: &RandomizeDensity) -> Arc<ConwaySession> {
    let first = served_descriptor(&sim.descriptor());
    let density = density.0.clone();
    let runner = Runner::spawn(sim, RunnerConfig { speed, ..default() })
        .with_clear(|sim: &mut LifeSim| {
            if let Err(e) = sim.apply(LifeCommand::Clear) {
                warn!("clear failed: {e}");
            }
        })
        .with_randomize(move |sim: &mut LifeSim, seed| {
            let density = f64::from_bits(density.load(Ordering::Relaxed));
            let command = LifeCommand::Randomize { density, seed: Some(seed), region: None, symmetry: Symmetry::None };
            if let Err(e) = sim.apply(command) {
                warn!("randomize failed: {e}");
            }
        });
    Arc::new(ConwaySession::new(runner, first))
}

/// Hashlife's memo statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashlifeStats {
    /// Distinct quadtree nodes in the store.
    pub nodes: usize,
    /// Level of the root (it covers `2^level` cells per side).
    pub root_level: u32,
    /// Allocated bytes of the node arena and intern table.
    pub memory_bytes: usize,
    /// Garbage collections so far.
    pub collections: u64,
}

/// What the simulation thread reports about the live world.
#[derive(Clone, Debug)]
pub struct LiveInfo {
    /// Session epoch the information belongs to (see [`LiveWorld::epoch`]).
    pub epoch: u64,
    /// Running world.
    pub world: cellular_automata::World,
    /// Running engine.
    pub engine: EngineKind,
    /// Engine choice.
    pub choice: EngineChoice,
    /// Generations per tick (pending value included).
    pub generations_per_tick: u64,
    /// Running rule.
    pub rule: LifeRule,
    /// Its preset.
    pub preset: RulePreset,
    /// Generation counter.
    pub generation: u64,
    /// Exact population.
    pub population: u128,
    /// Smallest rectangle holding every live cell.
    pub bbox: Option<CellRect>,
    /// Why the model stopped, if it did.
    pub halt: Option<String>,
    /// Births and deaths of the last tick.
    pub last_stats: Option<GenStats>,
    /// Hashlife memo statistics when Hashlife runs.
    pub hashlife: Option<HashlifeStats>,
    /// Approximate model memory.
    pub memory_bytes: usize,
    /// The model's descriptor.
    pub descriptor: ModelDescriptor,
}

impl LiveInfo {
    /// Reads the live world (runs on the simulation thread).
    pub fn read(sim: &LifeSim, epoch: u64) -> Self {
        let hashlife = sim.engine().as_any().downcast_ref::<HashlifeEngine>();
        let population = hashlife.map_or(u128::from(sim.population()), |h| h.universe().population_exact());
        Self {
            epoch,
            world: sim.world(),
            engine: sim.engine_kind(),
            choice: sim.engine_choice(),
            generations_per_tick: sim.generations_per_tick(),
            rule: sim.rule(),
            preset: sim.preset(),
            generation: sim.tick(),
            population,
            bbox: sim.engine().bounding_box(),
            halt: sim.halt_reason().map(str::to_string),
            last_stats: sim.last_stats(),
            hashlife: hashlife.map(|h| {
                let u = h.universe();
                HashlifeStats {
                    nodes: u.node_count(),
                    root_level: u.root_level(),
                    memory_bytes: u.memory_bytes(),
                    collections: u.gc_count(),
                }
            }),
            memory_bytes: sim.memory_bytes(),
            descriptor: sim.descriptor(),
        }
    }

    /// Whether the rule is exactly Conway's B3/S23.
    pub fn is_conway(&self) -> bool {
        self.rule == LifeRule::conway()
    }
}

/// Identity of what was loaded last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedOrigin {
    /// Scenario id, or `None` for a pattern opened on its own plane.
    pub scenario: Option<String>,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
}

/// The live world as last read from the simulation thread.
#[derive(Resource, Default, Debug)]
pub struct LiveWorld {
    /// Latest information (of any epoch).
    pub info: Option<LiveInfo>,
    /// Incremented whenever the simulation is replaced; information of an
    /// older epoch describes the previous world.
    pub epoch: u64,
    /// What was loaded.
    pub origin: Option<LoadedOrigin>,
    /// Frames since information about the current world first arrived.
    pub settled_frames: u32,
}

impl LiveWorld {
    /// The information if it describes the current world.
    pub fn current(&self) -> Option<&LiveInfo> {
        self.info.as_ref().filter(|i| i.epoch == self.epoch)
    }

    /// The information to display: the current world's, or during the frames
    /// of a load (until the new world's first read arrives) the previous
    /// one, so that the overlay and the Properties tab do not blink.
    pub fn shown(&self) -> Option<&LiveInfo> {
        self.current().or(self.info.as_ref())
    }

    /// Generation, population and births / deaths to show (see
    /// [`newest_counts`]), or `None` before the first live read.
    pub fn counts(&self, history: &MetricsHistory, status: &SessionStatus) -> Option<Counts> {
        let info = self.shown()?;
        let store = history.store();
        let at = |key: &str, tick: u64| store.get(key).and_then(|s| s.value_at(tick));
        let sample = status.get().and_then(|s| Some((s.tick, at("population", s.tick)?)));
        let settled = self.current().is_some() && self.settled_frames >= 2;
        let (generation, population, from_metrics) = newest_counts((info.generation, info.population), sample, settled);
        let births_deaths = if from_metrics {
            match (at("births", generation), at("deaths", generation)) {
                (Some(b), Some(d)) => Some((b.max(0.0) as u64, d.max(0.0) as u64)),
                _ => None,
            }
        } else {
            info.last_stats.map(|s| (s.births, s.deaths))
        };
        Some(Counts { generation, population, births_deaths })
    }
}

/// Counters of one state of the world, as shown by the overlay and the
/// Properties tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    /// Generation.
    pub generation: u64,
    /// Exact population.
    pub population: u128,
    /// Births and deaths of the tick that ended at `generation`, if counted.
    pub births_deaths: Option<(u64, u64)>,
}

/// Largest integer every `f64` below represents exactly (2^53).
const EXACT_F64: f64 = 9_007_199_254_740_992.0;

/// Generation and population to show, and whether they come from the
/// metric sample. The live read (`info`) is refreshed every
/// [`LIVE_PERIOD`] seconds while running; the shell records a `population`
/// sample after every tick, so the sample at the generation of this frame's
/// status (the generation the bottom bar shows) follows a running world
/// frame by frame, and it keeps the overlay, the Tools panel and the bottom
/// bar on the same generation. The sample is used when it is exact (below
/// 2^53; Hashlife populations can be larger) and `settled` (the live read of
/// the current world has been seen for a couple of frames: right after a
/// load the store can still hold samples of the previous world for a frame).
pub fn newest_counts(info: (u64, u128), sample: Option<(u64, f64)>, settled: bool) -> (u64, u128, bool) {
    match sample {
        Some((tick, value)) if settled && (0.0..EXACT_F64).contains(&value) => (tick, value as u128, true),
        _ => (info.0, info.1, false),
    }
}

/// Bookkeeping of the live-world reads.
#[derive(Resource, Default)]
pub struct LiveSync {
    pending: Mutex<Option<Receiver<LiveInfo>>>,
    since: f32,
    last_params: Option<ParamSet>,
    last_tick: Option<u64>,
    /// Status tick of the previous frame.
    status_tick: Option<u64>,
    /// The run went back in time (reset, undo) since the last read was sent.
    rewound: bool,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LiveSync {
    /// Forgets the pending read and asks for a new one on the next frame.
    pub fn restart(&mut self) {
        *lock(&self.pending) = None;
        self.since = f32::INFINITY;
        self.last_params = None;
        self.last_tick = None;
        self.status_tick = None;
        self.rewound = false;
    }
}

/// Something to load into the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadRequest {
    /// A Classic Conway scenario by id.
    Scenario(String),
    /// A library pattern alone on an empty infinite plane.
    Pattern(String),
}

/// Loads waiting for the next frame (scenario picker, tutorial, browser).
#[derive(Resource, Default, Debug)]
pub struct LoadRequests(pub Vec<LoadRequest>);

/// Emitted (as a resource flag) when a new world was loaded, so the camera
/// frames it once its first live information arrives.
#[derive(Resource, Default, Debug)]
pub struct WorldLoaded {
    /// Epoch of the load that still needs framing.
    pub frame_epoch: Option<u64>,
}

/// Mode resources that start over with a new world.
#[derive(SystemParam)]
pub struct SessionState<'w> {
    live: ResMut<'w, LiveWorld>,
    sync: ResMut<'w, LiveSync>,
    loaded: ResMut<'w, WorldLoaded>,
    view: ResMut<'w, crate::world::ViewRequestState>,
    latest: ResMut<'w, crate::world::LatestView>,
    selection: ResMut<'w, crate::tools::Selection>,
    drag: ResMut<'w, crate::tools::DragState>,
    jump: ResMut<'w, crate::commands::JumpState>,
    clipboard: ResMut<'w, crate::tools::Clipboard>,
    fit: ResMut<'w, crate::world::CameraFit>,
    analysis: ResMut<'w, crate::analysis::Analysis>,
}

impl SessionState<'_> {
    /// Forgets everything tied to the previous world, including answers
    /// still pending from it (a jump prepared for the old world must not
    /// run on the new one).
    fn restart(&mut self, origin: Option<LoadedOrigin>) {
        self.live.epoch += 1;
        self.live.origin = origin;
        self.live.settled_frames = 0;
        self.sync.restart();
        self.loaded.frame_epoch = Some(self.live.epoch);
        self.view.last = None;
        *self.latest = crate::world::LatestView::default();
        self.selection.clear();
        *self.drag = crate::tools::DragState::default();
        self.jump.forget();
        *lock(&self.clipboard.pending) = None;
        self.fit.forget_fresh();
        self.analysis.cancel();
        self.analysis.report = None;
    }
}

fn install(commands: &mut Commands, sim: LifeSim, speed: Speed, density: &RandomizeDensity) {
    let session = spawn_session(sim, speed, density);
    commands.insert_resource(ActiveSession::new(session.clone()));
    commands.insert_resource(ConwayRunner(session));
}

fn origin_of(scenario: &Scenario) -> LoadedOrigin {
    LoadedOrigin {
        scenario: Some(scenario.id.clone()),
        name: scenario.name.clone(),
        description: scenario.description.clone(),
    }
}

/// `OnEnter`: opens the selected (or default) scenario.
#[allow(clippy::too_many_arguments)]
pub(crate) fn enter_session(
    mut commands: Commands,
    scenarios: Res<ConwayScenarios>,
    library: Res<ConwayLibrary>,
    density: Res<RandomizeDensity>,
    mut picker: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    mut state: SessionState,
) {
    let wanted = picker.get(MODE).and_then(|l| l.selected.clone()).unwrap_or_else(|| DEFAULT_SCENARIO.into());
    let candidates = [scenarios.get(&wanted), scenarios.get(DEFAULT_SCENARIO), scenarios.0.first()];
    for scenario in candidates.into_iter().flatten() {
        match build_scenario(scenario, &library) {
            Ok(sim) => {
                install(&mut commands, sim, DEFAULT_SPEED, &density);
                picker.mark_selected(MODE, &scenario.id);
                state.restart(Some(origin_of(scenario)));
                return;
            }
            Err(e) => toasts.error(format!("Could not build {}: {e}", scenario.name)),
        }
    }
    // Nothing could be built (e.g. no scenario at all): an empty torus.
    let config = LifeConfig { init: vec![], ..LifeConfig::default() };
    match LifeSim::new(&config, 1, &library.resolver, LifeMode::Classic) {
        Ok(sim) => {
            install(&mut commands, sim, DEFAULT_SPEED, &density);
            state.restart(Some(LoadedOrigin {
                scenario: None,
                name: "Empty torus".into(),
                description: "An empty 256 × 256 torus.".into(),
            }));
        }
        Err(e) => toasts.error(format!("Could not start Classic Conway: {e}")),
    }
}

/// `OnExit`: drops the session; the runner's `Drop` stops and joins its thread.
pub(crate) fn exit_session(mut commands: Commands, mut state: SessionState) {
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<ConwayRunner>();
    state.restart(None);
    state.loaded.frame_epoch = None;
}

/// `Startup`: fills the top-bar scenario picker.
pub(crate) fn fill_scenarios(scenarios: Res<ConwayScenarios>, mut picker: ResMut<ModeScenarios>) {
    let selected = picker.get(MODE).and_then(|l| l.selected.clone()).or_else(|| Some(DEFAULT_SCENARIO.into()));
    picker.set(MODE, ScenarioList { entries: scenarios.entries(), selected });
}

/// Queues scenarios picked in the top bar.
pub(crate) fn collect_scenario_picks(mut picked: MessageReader<ScenarioSelected>, mut requests: ResMut<LoadRequests>) {
    for pick in picked.read().filter(|p| p.mode == MODE) {
        requests.0.push(LoadRequest::Scenario(pick.id.clone()));
    }
}

/// Loads the last queued request into the running session
/// (`Control::Replace`, so the plots restart but the session stays).
#[allow(clippy::too_many_arguments)]
pub(crate) fn load_requests(
    mut commands: Commands,
    mut requests: ResMut<LoadRequests>,
    scenarios: Res<ConwayScenarios>,
    library: Res<ConwayLibrary>,
    density: Res<RandomizeDensity>,
    runner: Option<Res<ConwayRunner>>,
    status: Res<SessionStatus>,
    mut picker: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    mut state: SessionState,
) {
    let Some(request) = requests.0.drain(..).next_back() else { return };
    let built = match &request {
        LoadRequest::Scenario(id) => match scenarios.get(id) {
            Some(s) => build_scenario(s, &library).map(|sim| (sim, origin_of(s))),
            None => Err(format!("unknown scenario `{id}`")),
        },
        LoadRequest::Pattern(id) => pattern_world(id, &library).map(|sim| {
            let entry = library.entry(id);
            let name = entry.map_or_else(|| id.clone(), |e| e.name.clone());
            let description = entry.and_then(|e| e.description.clone()).unwrap_or_default();
            (sim, LoadedOrigin { scenario: None, name, description })
        }),
    };
    let (sim, origin) = match built {
        Ok(built) => built,
        Err(e) => {
            toasts.error(format!("Could not load: {e}"));
            return;
        }
    };
    match &runner {
        Some(runner) => {
            runner.send(Control::Replace(Box::new(sim)));
            runner.send(Control::RequestView);
        }
        None => {
            let speed = status.get().map_or(DEFAULT_SPEED, |s| s.speed);
            install(&mut commands, sim, speed, &density);
        }
    }
    match &origin.scenario {
        Some(id) => {
            picker.mark_selected(MODE, id);
        }
        None => {
            let entries = scenarios.entries();
            picker.set(MODE, ScenarioList { entries, selected: None });
        }
    }
    state.restart(Some(origin));
}

/// Reads the live world from the simulation thread: when a parameter or the
/// generation changed, and every [`LIVE_PERIOD`] seconds. Keeps the served
/// descriptor in step with the model's.
pub(crate) fn poll_live_world(
    time: Res<Time>,
    runner: Option<Res<ConwayRunner>>,
    status: Res<SessionStatus>,
    mut live: ResMut<LiveWorld>,
    mut sync: ResMut<LiveSync>,
    mut loaded: ResMut<WorldLoaded>,
) {
    let Some(runner) = runner else { return };
    sync.since += time.delta_secs();
    if live.current().is_some() {
        live.settled_frames = live.settled_frames.saturating_add(1);
    }
    let status_tick = status.get().map(|s| s.tick);
    if let (Some(now), Some(before)) = (status_tick, sync.status_tick)
        && now < before
    {
        sync.rewound = true;
    }
    sync.status_tick = status_tick;
    let received = lock(&sync.pending).as_ref().map(Receiver::try_recv);
    match received {
        Some(Ok(info)) => {
            *lock(&sync.pending) = None;
            if info.epoch == live.epoch {
                if !runner.0.update_descriptor(served_descriptor(&info.descriptor)) {
                    warn!("all {DESCRIPTOR_SLOTS} descriptor slots are used; the Rules tab keeps the previous text");
                }
                // A reset that applied a new topology or size, or a loaded
                // save: frame the new world.
                if live.current().is_some_and(|old| old.world != info.world) {
                    loaded.frame_epoch = Some(live.epoch);
                }
            }
            live.info = Some(info);
        }
        Some(Err(TryRecvError::Disconnected)) => *lock(&sync.pending) = None,
        Some(Err(TryRecvError::Empty)) => {}
        None => {
            let current = status.get();
            // Parameter changes are read at once; generation changes at once
            // while paused (steps, undo) or when the run went back in time
            // (reset, undo while running), else at most every LIVE_PERIOD.
            let changed = sync.rewound
                || current.is_some_and(|s| {
                    sync.last_params.as_ref() != Some(&s.params) || (!s.playing && sync.last_tick != Some(s.tick))
                });
            if changed || sync.since >= LIVE_PERIOD {
                sync.rewound = false;
                if let Some(s) = current {
                    sync.last_params = Some(s.params.clone());
                    sync.last_tick = Some(s.tick);
                }
                sync.since = 0.0;
                let epoch = live.epoch;
                *lock(&sync.pending) = Some(runner.request(move |sim| LiveInfo::read(sim, epoch)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use std::sync::Weak;

    #[test]
    fn scenarios_are_the_classic_ones() {
        let scenarios = ConwayScenarios::builtin();
        assert_eq!(scenarios.0.len(), 9);
        assert!(scenarios.get(DEFAULT_SCENARIO).is_some());
        assert!(scenarios.0.iter().all(|s| s.model_id() == "life.classic"));
        let library = ConwayLibrary::builtin("test".into());
        assert!(build_scenario(scenarios.get(DEFAULT_SCENARIO).unwrap(), &library).is_ok());
        let err = build_scenario(scenarios.get("breeder-hashlife").unwrap(), &library).err().unwrap();
        assert!(err.contains("asset catalog"), "{err}");
        let world = pattern_world("glider", &library).unwrap();
        assert_eq!((world.population(), world.world().topology), (5, Topology::Infinite));
    }

    #[test]
    fn served_descriptor_hides_rule_and_engine() {
        let sim = LifeSim::conway(1);
        let d = served_descriptor(&sim.descriptor());
        let keys: Vec<&str> = d.params.iter().map(|p| p.key.as_str()).collect();
        assert!(!keys.contains(&"preset") && !keys.contains(&"rule") && !keys.contains(&"engine"));
        assert!(keys.contains(&"topology") && keys.contains(&"generations_per_tick"));
        assert!(!d.rules.contains('∈'), "drawable characters only");
        assert_eq!(d.id, "life.classic");
        assert_eq!(d.summary, SERVED_SUMMARY);
    }

    /// The served descriptor does not change with the generations per tick,
    /// so scrubbing that value cannot use up the descriptor slots.
    #[test]
    fn served_rules_do_not_depend_on_the_generations_per_tick() {
        let mut sim = LifeSim::conway(1);
        let before = served_descriptor(&sim.descriptor());
        sim.set_param("generations_per_tick", ParamValue::Int(64)).unwrap();
        sim.step(); // the value applies on the next tick
        let model = sim.descriptor();
        assert!(model.rules.contains("One tick advances 64 generation(s)."), "the model's sentence changed");
        let after = served_descriptor(&model);
        assert_eq!(before, after);
        assert!(after.rules.contains("set in *Generations per tick*") && after.rules.contains("Births and deaths"));
        assert_eq!(generic_ticks_sentence("no such sentence"), "no such sentence");
    }

    #[test]
    fn shown_counts_follow_the_newest_exact_state() {
        // A newer exact sample wins once the live read has settled.
        assert_eq!(newest_counts((87, 26_531), Some((96, 25_836.0)), true), (96, 25_836, true));
        assert_eq!(newest_counts((87, 26_531), Some((96, 25_836.0)), false), (87, 26_531, false));
        // After a rewind (reset, undo) the status is older than the live read.
        assert_eq!(newest_counts((100, 5), Some((40, 9.0)), true), (40, 9, true));
        // Populations beyond 2^53 are taken from the exact live read.
        let huge = 2f64.powi(60);
        assert_eq!(newest_counts((1, 1 << 60), Some((2, huge)), true), (1, 1 << 60, false));
        assert_eq!(newest_counts((3, 7), None, true), (3, 7, false));
    }

    #[test]
    fn descriptor_slots_reuse_equal_descriptors() {
        let sim = LifeSim::conway(1);
        let first = served_descriptor(&sim.descriptor());
        let session = ConwaySession::new(Runner::spawn(LifeSim::conway(1), RunnerConfig::default()), first.clone());
        let mut other = first.clone();
        other.rules.push_str("\n\nchanged");
        assert!(session.update_descriptor(other.clone()));
        assert_eq!(session.descriptor().rules, other.rules);
        assert!(session.update_descriptor(first.clone()));
        assert_eq!(*session.descriptor(), first);
        assert_eq!(session.used.load(Ordering::Acquire), 2, "the first slot was reused");
    }

    fn lifecycle_world() -> World {
        let mut world = World::new();
        world.insert_resource(ConwayScenarios::builtin());
        world.insert_resource(ConwayLibrary::builtin("test".into()));
        world.init_resource::<RandomizeDensity>();
        world.init_resource::<ModeScenarios>();
        world.init_resource::<Toasts>();
        world.init_resource::<SessionStatus>();
        world.init_resource::<LoadRequests>();
        world.init_resource::<LiveWorld>();
        world.init_resource::<LiveSync>();
        world.init_resource::<WorldLoaded>();
        world.init_resource::<crate::world::ViewRequestState>();
        world.init_resource::<crate::world::LatestView>();
        world.init_resource::<crate::tools::Selection>();
        world.init_resource::<crate::tools::DragState>();
        world.init_resource::<crate::tools::Clipboard>();
        world.init_resource::<crate::commands::JumpState>();
        world.init_resource::<crate::world::CameraFit>();
        world.init_resource::<crate::analysis::Analysis>();
        world.run_system_once(fill_scenarios).unwrap();
        world
    }

    fn runner_of(world: &World) -> Weak<ConwaySession> {
        Arc::downgrade(&world.resource::<ConwayRunner>().0)
    }

    /// Enter, leave, enter again: every exit drops the last handle on the
    /// runner, whose `Drop` stops and joins the simulation thread.
    #[test]
    fn exit_releases_the_runner_and_reentry_starts_a_new_one() {
        let mut world = lifecycle_world();
        world.run_system_once(enter_session).unwrap();
        let first = runner_of(&world);
        assert!(world.contains_resource::<ActiveSession>());
        let runner = world.resource::<ConwayRunner>().clone();
        runner.0.play();
        drop(runner);
        world.run_system_once(exit_session).unwrap();
        assert!(first.upgrade().is_none(), "the runner (and its thread) is gone after OnExit");
        assert!(!world.contains_resource::<ActiveSession>() && !world.contains_resource::<ConwayRunner>());
        world.run_system_once(enter_session).unwrap();
        let second = runner_of(&world);
        assert!(second.upgrade().is_some());
        world.run_system_once(exit_session).unwrap();
        assert!(second.upgrade().is_none());
    }

    /// Loading keeps the session (Control::Replace) and bumps the epoch.
    #[test]
    fn loads_replace_the_simulation_in_the_same_session() {
        let mut world = lifecycle_world();
        world.run_system_once(enter_session).unwrap();
        let first = runner_of(&world);
        let epoch = world.resource::<LiveWorld>().epoch;
        world
            .resource_mut::<LoadRequests>()
            .0
            .extend([LoadRequest::Scenario("nope".into()), LoadRequest::Scenario("random-soup-torus".into())]);
        world.run_system_once(load_requests).unwrap();
        assert!(Weak::ptr_eq(&first, &runner_of(&world)), "same session");
        assert_eq!(world.resource::<LiveWorld>().epoch, epoch + 1);
        assert_eq!(
            world.resource::<ModeScenarios>().get(MODE).and_then(|l| l.selected.clone()).as_deref(),
            Some("random-soup-torus")
        );
        world.resource_mut::<LoadRequests>().0.push(LoadRequest::Pattern("glider".into()));
        world.run_system_once(load_requests).unwrap();
        let live = world.resource::<LiveWorld>();
        assert_eq!(live.origin.as_ref().map(|o| o.name.as_str()), Some("Glider"));
        assert_eq!(world.resource::<ModeScenarios>().get(MODE).and_then(|l| l.selected.clone()), None);
        // The replaced simulation is the one the runner now runs.
        let rx = world.resource::<ConwayRunner>().request(|sim| (sim.population(), sim.world().topology));
        assert_eq!(rx.recv().unwrap(), (5, Topology::Infinite));
        world.run_system_once(exit_session).unwrap();
        assert!(first.upgrade().is_none());
    }
}
