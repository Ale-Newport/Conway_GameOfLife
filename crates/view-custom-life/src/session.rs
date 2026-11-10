//! Runner lifecycle, scenarios and data read from the simulation thread.
//!
//! The custom-life descriptor is generated from the running definition
//! (rules text, parameters, metrics); the runner publishes it again when it
//! changes, so the shell's Parameters, Rules and Metrics tabs always
//! describe what is simulated ([`served_descriptor`]). Parameter defaults
//! are the exception: they are the design as last loaded or applied
//! ([`CustomLifeSim::reference`]), so after a parameter change the shell
//! offers going back to it.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use bevy::prelude::*;
use interface::{ActiveSession, MetricsHistory, ModeScenarios, ScenarioEntry, ScenarioList, ScenarioSelected, Toasts};
use rule_engine::def::LifeSystem;
use rule_engine::scenarios::{self, Scenario};
use rule_engine::sim::entity::Entity;
use rule_engine::{CustomLifeCommand, CustomLifeSim, CustomLifeView, CustomLifeViewRequest};
use simulation_core::{Control, ModelDescriptor, ParamValue, Runner, RunnerConfig, Simulation, Snapshot};

use crate::MODE;
use crate::editor::ClEditor;
use crate::state::ClUi;

/// Samples per axis requested for field grids.
pub const FIELD_RESOLUTION: u32 = 256;

/// Seconds between two reads of the running definition.
pub const INFO_PERIOD: f32 = 0.5;

/// Seconds between two reads of the selected entity.
pub const INSPECT_PERIOD: f32 = 0.2;

/// The descriptor this mode serves for a running system: its `name` is the
/// system's name, so the top bar reads "Custom Life Creator · Producers &
/// consumers" instead of repeating the mode title. The model id is kept, so
/// save / load still matches. The rename is presentation: the model's own
/// name stays the model's ("Custom Life Creator") for the registry and the
/// CLI.
pub fn served_descriptor(mut descriptor: ModelDescriptor, system_name: &str) -> ModelDescriptor {
    if !system_name.trim().is_empty() {
        descriptor.name = system_name.to_string();
    }
    descriptor
}

/// The running session of this mode (inserted on enter, removed on exit).
#[derive(Resource, Clone)]
pub struct ClRunner(pub Arc<Runner<CustomLifeSim>>);

impl ClRunner {
    /// Sends a control message.
    pub fn send(&self, control: Control<CustomLifeSim>) {
        self.0.send(control);
    }

    /// Applies an undoable command.
    pub fn edit(&self, command: CustomLifeCommand) {
        self.0.send(Control::Edit(command));
    }
}

/// "Clear": removes every entity (carried resources drop into the fields).
fn clear_entities(sim: &mut CustomLifeSim) {
    let ids: Vec<u64> = sim.entities().map(|(id, _)| id.get()).collect();
    for id in ids {
        let _ = sim.apply(CustomLifeCommand::Remove { id });
    }
}

/// "Randomize": restarts from the reset target with a new seed (placement,
/// initial fields, decisions and events all follow the seed).
fn reseed(sim: &mut CustomLifeSim, seed: u64) {
    if sim.set_param("seed", ParamValue::Int((seed & i64::MAX as u64) as i64)).is_ok() {
        sim.reset();
    }
}

/// Spawns a runner for `sim` with the mode's descriptor, hooks and view
/// request.
pub fn spawn_session(sim: CustomLifeSim) -> Arc<Runner<CustomLifeSim>> {
    let runner = Runner::spawn_with_descriptor(sim, RunnerConfig::default(), |s| {
        served_descriptor(s.descriptor(), &s.system().name)
    })
    .with_clear(clear_entities)
    .with_randomize(reseed);
    runner.send(Control::SetViewRequest(view_request()));
    Arc::new(runner)
}

/// What the view asks the simulation for.
pub fn view_request() -> CustomLifeViewRequest {
    CustomLifeViewRequest {
        include_fields: true,
        field_resolution: FIELD_RESOLUTION,
        include_rules: true,
        recent_events: 30,
    }
}

/// Definitions read from the simulation thread.
#[derive(Clone, Debug)]
pub struct SimInfo {
    /// The definition being simulated.
    pub running: LifeSystem,
    /// The definition and seed the next reset starts from.
    pub target: LifeSystem,
    /// Seed of the next reset.
    pub seed: u64,
}

impl SimInfo {
    fn read(sim: &CustomLifeSim) -> Self {
        let (target, seed) = sim.reset_target();
        Self { running: sim.system().clone(), target: target.clone(), seed }
    }
}

/// Full state of one entity (selection inspector).
#[derive(Clone, Debug)]
pub struct EntityDetail {
    /// Entity id.
    pub id: u64,
    /// Tick it was read at.
    pub tick: u64,
    /// The entity (`None` once it no longer exists).
    pub entity: Option<Entity>,
}

/// Parts needed to rebuild the simulation when the mode is entered again
/// (see [`ClResume`]).
struct RebuildParts {
    snapshot: Snapshot,
    running: LifeSystem,
    target: LifeSystem,
    seed: u64,
}

/// Data read from the runner, refreshed every frame.
#[derive(Resource, Default)]
pub struct ClData {
    /// Latest published view.
    pub view: Option<Arc<CustomLifeView>>,
    /// Publication counter of `view`.
    pub view_seq: u64,
    /// Running and reset definitions.
    pub info: Option<SimInfo>,
    /// Revision of `info` (increments on every read that changed it).
    pub info_rev: u64,
    /// Selected entity, read from the simulation.
    pub detail: Option<EntityDetail>,
    info_rx: Mutex<Option<Receiver<SimInfo>>>,
    detail_rx: Mutex<Option<Receiver<EntityDetail>>>,
    info_timer: f32,
    detail_timer: f32,
    info_wanted: bool,
    /// Start playing once a replaced simulation is confirmed (see [`ClData::play_when_ready`]).
    play_pending: bool,
}

impl ClData {
    /// Asks for the definitions on the next update (after an edit).
    pub fn refresh_info(&mut self) {
        self.info_wanted = true;
    }

    /// Plays as soon as the simulation that just replaced the old one is
    /// confirmed by the runner status (so the metric plots restart cleanly).
    pub fn play_when_ready(&mut self) {
        self.play_pending = true;
    }

    /// Forgets the definitions read so far and any read in flight (the
    /// simulation was replaced: the next read describes the new one and is
    /// not compared with the old one).
    pub fn forget_info(&mut self) {
        self.info = None;
        *self.info_rx.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.info_wanted = true;
    }

    /// Forgets everything (new session).
    pub fn reset(&mut self) {
        *self = Self { info_wanted: true, ..Self::default() };
    }

    /// Tick of the latest view.
    pub fn tick(&self) -> u64 {
        self.view.as_ref().map_or(0, |v| v.tick)
    }
}

fn take<T>(slot: &Mutex<Option<Receiver<T>>>) -> Option<T> {
    let mut guard = slot.lock().unwrap_or_else(PoisonError::into_inner);
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

fn pending<T>(slot: &Mutex<Option<Receiver<T>>>) -> bool {
    slot.lock().unwrap_or_else(PoisonError::into_inner).is_some()
}

fn store<T>(slot: &Mutex<Option<Receiver<T>>>, rx: Receiver<T>) {
    *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(rx);
}

/// Built-in presets, loaded once.
#[derive(Resource, Clone, Debug, Default)]
pub struct ClScenarios {
    /// Presets in their fixed order.
    pub presets: Vec<Scenario>,
    /// Why they could not be loaded, if so.
    pub error: Option<String>,
}

impl ClScenarios {
    /// Loads the embedded presets.
    pub fn load() -> Self {
        match scenarios::builtin() {
            Ok(presets) => Self { presets, error: None },
            Err(e) => Self { presets: Vec::new(), error: Some(e.to_string()) },
        }
    }

    /// A preset by id.
    pub fn get(&self, id: &str) -> Option<&Scenario> {
        self.presets.iter().find(|s| s.id == id)
    }

    /// The preset opened when the mode starts.
    pub fn default_preset(&self) -> Option<&Scenario> {
        self.get(DEFAULT_SCENARIO).or_else(|| self.presets.first())
    }
}

/// Preset opened on enter.
pub const DEFAULT_SCENARIO: &str = "producers-consumers";

/// Fills the top-bar scenario picker.
pub fn fill_scenarios(presets: Res<ClScenarios>, mut list: ResMut<ModeScenarios>) {
    let entries = presets
        .presets
        .iter()
        .map(|s| ScenarioEntry { id: s.id.clone(), title: s.name.clone(), description: s.description.clone() })
        .collect();
    list.set(MODE, ScenarioList { entries, selected: presets.default_preset().map(|s| s.id.clone()) });
}

/// A blank system used when no preset is available.
fn fallback_system() -> LifeSystem {
    crate::draft::blank_system("New system")
}

/// What the mode was running when it was left, restored when it is
/// entered again (so leaving for the dashboard never discards a design).
#[derive(Resource, Default)]
pub struct ClResume(Option<Resume>);

enum Resume {
    /// The complete state (snapshot, running and reset definitions, seed).
    State(Box<RebuildParts>),
    /// The system and seed of the next reset (the snapshot was not ready).
    Reset(Box<LifeSystem>, u64),
}

impl ClResume {
    /// Whether a previous visit left something to restore.
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
}

/// How long leaving the mode waits for the simulation thread's snapshot.
pub const RESUME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Starts the session on entering the mode. After a previous visit the
/// world continues where it was left (same state, draft and selection of
/// the editor); otherwise the selected preset (or the default one) starts
/// and the draft becomes its definition.
#[allow(clippy::too_many_arguments)]
pub fn enter(
    mut commands: Commands,
    presets: Res<ClScenarios>,
    list: Res<ModeScenarios>,
    mut resume: ResMut<ClResume>,
    mut editor: ResMut<ClEditor>,
    mut data: ResMut<ClData>,
    mut ui: ResMut<ClUi>,
    mut toasts: ResMut<Toasts>,
) {
    data.reset();
    ui.clear_selection();
    ui.fit_requested = true;
    if let Some(previous) = resume.0.take() {
        let resumed = match previous {
            Resume::State(parts) => rebuild(*parts),
            Resume::Reset(system, seed) => {
                CustomLifeSim::new(*system, seed).map(spawn_session).map_err(|e| e.to_string())
            }
        };
        match resumed {
            Ok(session) => {
                commands.insert_resource(ActiveSession::new(session.clone()));
                commands.insert_resource(ClRunner(session));
                return;
            }
            Err(e) => toasts.error(format!("The previous world could not be restored: {e}")),
        }
    }
    let selected = list.get(MODE).and_then(|l| l.selected.clone());
    let scenario = selected.as_deref().and_then(|id| presets.get(id)).or_else(|| presets.default_preset()).cloned();
    if let Some(error) = &presets.error {
        toasts.error(format!("Custom Life presets could not be loaded: {error}"));
    }
    let (system, seed, label) = match &scenario {
        Some(s) => (s.system.clone(), s.seed, format!("Preset: {}", s.name)),
        None => (fallback_system(), 1, "New system".to_string()),
    };
    let sim = match CustomLifeSim::new(system.clone(), seed) {
        Ok(sim) => sim,
        Err(e) => {
            toasts.error(format!("Could not start {label}: {e}"));
            match CustomLifeSim::new(fallback_system(), 1) {
                Ok(sim) => sim,
                Err(e) => {
                    error!("the blank custom-life system does not compile: {e}");
                    return;
                }
            }
        }
    };
    let session = spawn_session(sim);
    commands.insert_resource(ActiveSession::new(session.clone()));
    commands.insert_resource(ClRunner(session));
    editor.load(system, seed, label);
}

/// Stops the session on leaving the mode, keeping what it ran for the next
/// visit (a snapshot taken on the simulation thread, or — if it does not
/// answer within [`RESUME_TIMEOUT`] — the system of the next reset). The
/// runner thread stops when the last reference is dropped.
pub fn exit(
    mut commands: Commands,
    runner: Option<Res<ClRunner>>,
    mut data: ResMut<ClData>,
    mut resume: ResMut<ClResume>,
) {
    if let Some(runner) = runner {
        let rx = runner.0.request(rebuild_parts);
        resume.0 = match rx.recv_timeout(RESUME_TIMEOUT) {
            Ok(Ok(parts)) => Some(Resume::State(Box::new(parts))),
            _ => data.info.as_ref().map(|i| Resume::Reset(Box::new(i.target.clone()), i.seed)),
        };
    }
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<ClRunner>();
    data.reset();
}

/// Everything needed to rebuild a simulation elsewhere (runs on the
/// simulation thread).
fn rebuild_parts(sim: &mut CustomLifeSim) -> Result<RebuildParts, String> {
    let (target, seed) = sim.reset_target();
    let (target, seed) = (target.clone(), seed);
    sim.snapshot()
        .map(|snapshot| RebuildParts { snapshot, running: sim.system().clone(), target, seed })
        .map_err(|e| e.to_string())
}

/// Marks no preset as loaded in the top-bar picker (the running system is
/// not one of them).
pub fn clear_scenario(list: &mut ModeScenarios) {
    if let Some(current) = list.get(MODE)
        && current.selected.is_some()
    {
        let entries = current.entries.clone();
        list.set(MODE, ScenarioList { entries, selected: None });
    }
}

/// Replaces the simulation with a new one built from `system` (undo history
/// and plots restart; the draft is not touched). The top-bar picker shows
/// no preset afterwards ([`open_preset`] marks its own).
pub fn start_system(
    runner: &ClRunner,
    system: LifeSystem,
    seed: u64,
    data: &mut ClData,
    ui: &mut ClUi,
    history: &mut MetricsHistory,
    list: &mut ModeScenarios,
) -> Result<(), String> {
    let sim = CustomLifeSim::new(system, seed).map_err(|e| e.to_string())?;
    let tick = sim.current_tick();
    runner.send(Control::Replace(Box::new(sim)));
    // Samples the old run produces until the replacement lands are dropped:
    // the plots restart when the status shows the new run paused at `tick`.
    history.expect_restore(tick);
    data.view = None;
    data.detail = None;
    data.forget_info();
    clear_scenario(list);
    ui.clear_selection();
    ui.fit_requested = true;
    Ok(())
}

/// Opens a preset: the draft becomes its definition and the simulation
/// restarts from it.
#[allow(clippy::too_many_arguments)]
pub fn open_preset(
    id: &str,
    presets: &ClScenarios,
    runner: &ClRunner,
    editor: &mut ClEditor,
    data: &mut ClData,
    ui: &mut ClUi,
    history: &mut MetricsHistory,
    list: &mut ModeScenarios,
) -> Result<(), String> {
    let scenario = presets.get(id).ok_or_else(|| format!("no preset `{id}`"))?;
    start_system(runner, scenario.system.clone(), scenario.seed, data, ui, history, list)?;
    editor.load(scenario.system.clone(), scenario.seed, format!("Preset: {}", scenario.name));
    list.mark_selected(MODE, id);
    Ok(())
}

/// Reacts to the top-bar scenario picker.
#[allow(clippy::too_many_arguments)]
pub fn handle_scenario_selection(
    mut picked: MessageReader<ScenarioSelected>,
    presets: Res<ClScenarios>,
    runner: Option<Res<ClRunner>>,
    mut editor: ResMut<ClEditor>,
    mut data: ResMut<ClData>,
    mut ui: ResMut<ClUi>,
    mut history: ResMut<MetricsHistory>,
    mut list: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
) {
    let ids: Vec<String> = picked.read().filter(|s| s.mode == MODE).map(|s| s.id.clone()).collect();
    let Some(runner) = runner else { return };
    for id in ids {
        let had_edits = editor.is_edited(data.info.as_ref());
        match open_preset(&id, &presets, &runner, &mut editor, &mut data, &mut ui, &mut history, &mut list) {
            Ok(()) if had_edits => toasts.warning("The previous draft had unapplied edits; they were replaced."),
            Ok(()) => {}
            Err(e) => toasts.error(e),
        }
    }
}

/// Reads the latest view, the definitions (periodically and after edits)
/// and the selected entity.
pub fn poll_data(
    runner: Option<Res<ClRunner>>,
    mut data: ResMut<ClData>,
    mut editor: ResMut<ClEditor>,
    mut ui: ResMut<ClUi>,
    time: Res<Time<Real>>,
    history: Res<MetricsHistory>,
) {
    let Some(runner) = runner else { return };
    let dt = time.delta_secs();
    if data.play_pending && !history.awaiting_restore() {
        data.play_pending = false;
        runner.send(Control::Play);
    }
    if let Some(published) = runner.0.latest_view()
        && published.seq != data.view_seq
    {
        let went_back = data.view.as_ref().is_some_and(|v| published.tick < v.tick);
        data.view_seq = published.seq;
        data.view = Some(published.view);
        if went_back {
            data.info_wanted = true;
        }
    }
    if let Some(info) = take(&data.info_rx) {
        if let Some(old) = data.info.as_ref()
            && (old.target != info.target || old.seed != info.seed)
        {
            // Parameters tab, undo/redo, load: keep the draft in step.
            editor.follow_target(&old.target, old.seed, &info.target, info.seed);
        }
        let changed = data
            .info
            .as_ref()
            .is_none_or(|old| old.running != info.running || old.target != info.target || old.seed != info.seed);
        if changed {
            data.info_rev += 1;
            let names = |s: &LifeSystem| s.entity_types.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
            let resources: Vec<String> = info.running.world.resources.iter().map(|r| r.name.clone()).collect();
            let fields: Vec<String> = resources
                .iter()
                .cloned()
                .chain(info.running.world.environment.iter().map(|e| e.name.clone()))
                .collect();
            ui.sync_names(&names(&info.running), &resources, &fields);
        }
        data.info = Some(info);
    }
    data.info_timer += dt;
    if (data.info_wanted || data.info_timer >= INFO_PERIOD) && !pending(&data.info_rx) {
        data.info_wanted = false;
        data.info_timer = 0.0;
        let rx = runner.0.request(|sim| SimInfo::read(sim));
        store(&data.info_rx, rx);
    }
    if let Some(detail) = take(&data.detail_rx)
        && ui.selected == Some(detail.id)
    {
        data.detail = Some(detail);
    }
    data.detail_timer += dt;
    match ui.selected {
        Some(id) => {
            let stale = data.detail.as_ref().is_none_or(|d| d.id != id);
            if (stale || data.detail_timer >= INSPECT_PERIOD) && !pending(&data.detail_rx) {
                data.detail_timer = 0.0;
                let rx = runner.0.request(move |sim| EntityDetail {
                    id,
                    tick: sim.current_tick(),
                    entity: sim.entities().find(|(i, _)| i.get() == id).map(|(_, e)| e.clone()),
                });
                store(&data.detail_rx, rx);
            }
        }
        None => data.detail = None,
    }
}

fn rebuild(parts: RebuildParts) -> Result<Arc<Runner<CustomLifeSim>>, String> {
    let mut sim = CustomLifeSim::new(parts.running.clone(), parts.seed).map_err(|e| e.to_string())?;
    sim.restore(&parts.snapshot).map_err(|e| e.to_string())?;
    if parts.target != parts.running {
        // Re-establishing the pending edits is not an Apply: the design the
        // parameter defaults refer to stays the snapshot's.
        let reference = sim.reference().clone();
        sim.edit_system(parts.target).map_err(|e| e.to_string())?;
        sim.set_reference(reference);
    }
    if sim.reset_target().1 != parts.seed {
        sim.set_param("seed", ParamValue::Int((parts.seed & i64::MAX as u64) as i64)).map_err(|e| e.to_string())?;
    }
    Ok(spawn_session(sim))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for<T>(rx: &Receiver<T>) -> T {
        rx.recv_timeout(Duration::from_secs(10)).expect("the runner answers")
    }

    fn sim(id: &str) -> CustomLifeSim {
        CustomLifeSim::from_scenario(&scenarios::builtin_by_id(id).unwrap()).unwrap()
    }

    /// The runner serves the running system's descriptor, named after it,
    /// from the start and after the system is replaced.
    #[test]
    fn descriptor_follows_the_running_system() {
        let session = spawn_session(sim("bacterial-growth"));
        let first = session.descriptor();
        assert!(first.rules.contains("rule feed for bacterium"));
        assert_eq!(first.name, sim("bacterial-growth").system().name);
        let robots = sim("robot-swarm");
        let expected = served_descriptor(robots.descriptor(), &robots.system().name);
        session.send(Control::Replace(Box::new(robots)));
        let start = Instant::now();
        while session.status().descriptor_seq == 0 {
            assert!(start.elapsed() < Duration::from_secs(10), "the descriptor was not published again");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(*session.descriptor(), expected);
    }

    #[test]
    fn clear_and_randomize_hooks() {
        use simulation_core::SessionControl;
        let session = spawn_session(sim("bacterial-growth"));
        assert!(session.can_clear() && session.can_randomize());
        session.clear();
        let count = wait_for(&session.request(|s| s.total_population()));
        assert_eq!(count, 0, "clear removes every entity");
        session.randomize(12345);
        let (count, seed) = wait_for(&session.request(|s| (s.total_population(), s.reset_target().1)));
        assert_eq!(count, 20, "randomize restarts the preset's initial population");
        assert_eq!(seed, 12345);
    }

    #[test]
    fn rebuild_keeps_state_and_pending_edits() {
        let mut s = sim("bacterial-growth");
        s.advance(15);
        let mut target = s.system().clone();
        target.world.width = 64; // requires a reset
        s.edit_system(target.clone()).unwrap();
        let parts = RebuildParts {
            snapshot: s.snapshot().unwrap(),
            running: s.system().clone(),
            target: target.clone(),
            seed: 42,
        };
        let session = rebuild(parts).unwrap();
        let start = Instant::now();
        let (tick, pending, width) = loop {
            let r = wait_for(&session.request(|s| (s.current_tick(), s.reset_pending(), s.system().world.width)));
            if r.0 == 15 || start.elapsed() > Duration::from_secs(5) {
                break r;
            }
        };
        assert_eq!((tick, pending, width), (15, true, 48));
    }

    fn eventually(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting until {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The shell offers going back to a parameter's default (↺) while the
    /// value differs from it: the default is the design as last loaded or
    /// applied, which parameter changes leave alone.
    #[test]
    fn parameter_defaults_are_the_design_as_last_loaded_or_applied() {
        use simulation_core::SessionControl;
        const KEY: &str = "entity.bacterium.metabolism";
        let runner = ClRunner(spawn_session(sim("bacterial-growth")));
        let session = runner.0.clone();
        let default = || session.descriptor().param(KEY).unwrap().default.clone();
        let value = || session.status().params.get(KEY).cloned();
        let designed = default();
        session.set_param(KEY, ParamValue::Float(0.5));
        eventually("the change is applied", || value() == Some(ParamValue::Float(0.5)));
        assert_eq!(default(), designed, "parameter changes leave the default alone");
        assert_ne!(value(), Some(default()), "so the shell offers going back (↺)");
        session.set_param(KEY, default()); // what ↺ sends
        eventually("the value is back at the design's", || value() == Some(designed.clone()));
        // Applying from the editor makes the draft the design.
        let info = wait_for(&session.request(|s| SimInfo::read(s)));
        let mut editor = ClEditor::default();
        editor.load(info.target.clone(), info.seed, "test");
        rule_engine::sim::params::apply_param(&mut editor.draft, KEY, &ParamValue::Float(0.25)).unwrap();
        let mut data = ClData::default();
        crate::editor::apply_draft(&runner, &editor, &info, false, &mut data);
        eventually("the applied value is the default", || default() == ParamValue::Float(0.25));
        // Loading a scenario makes it the design.
        let presets = ClScenarios::load();
        let (mut ui, mut history, mut list) = (ClUi::default(), MetricsHistory::default(), ModeScenarios::default());
        let id = "bacterial-growth";
        open_preset(id, &presets, &runner, &mut editor, &mut data, &mut ui, &mut history, &mut list).unwrap();
        eventually("the preset's value is the default", || default() == designed);
    }

    #[test]
    fn rebuild_keeps_the_parameter_defaults() {
        let mut s = sim("bacterial-growth");
        let designed = served_descriptor(s.descriptor(), &s.system().name);
        s.set_param("width", ParamValue::Int(64)).unwrap();
        s.set_param("entity.bacterium.metabolism", ParamValue::Float(0.5)).unwrap();
        let (target, seed) = s.reset_target();
        let parts =
            RebuildParts { snapshot: s.snapshot().unwrap(), running: s.system().clone(), target: target.clone(), seed };
        assert_eq!(rebuild(parts).unwrap().descriptor().params, designed.params);
    }

    #[test]
    fn served_descriptors_name_the_system() {
        for preset in scenarios::builtin().unwrap() {
            let s = CustomLifeSim::from_scenario(&preset).unwrap();
            let d = served_descriptor(s.descriptor(), &s.system().name);
            assert_eq!(d.name, preset.system.name);
            assert_eq!(d.id, rule_engine::MODEL_ID, "the id stays stable for save / load");
            assert_eq!(d.params, s.descriptor().params, "parameters are served unchanged");
        }
    }

    #[test]
    fn replacing_the_system_clears_the_picker() {
        let mut list = ModeScenarios::default();
        list.set(MODE, ScenarioList { entries: vec![], selected: None });
        let presets = ClScenarios::load();
        let entries = presets
            .presets
            .iter()
            .map(|s| ScenarioEntry { id: s.id.clone(), title: s.name.clone(), description: String::new() })
            .collect();
        list.set(MODE, ScenarioList { entries, selected: Some(DEFAULT_SCENARIO.into()) });
        clear_scenario(&mut list);
        let l = list.get(MODE).unwrap();
        assert_eq!((l.selected.as_deref(), l.entries.len()), (None, 7));
    }

    #[test]
    fn leaving_and_entering_resumes_the_world_and_keeps_the_draft() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(ClScenarios::load());
        world.insert_resource(ModeScenarios::default());
        world.insert_resource(ClResume::default());
        world.insert_resource(ClEditor::default());
        world.insert_resource(ClData::default());
        world.insert_resource(ClUi::default());
        world.insert_resource(Toasts::default());
        world.run_system_once(enter).unwrap();
        assert!(world.contains_resource::<ActiveSession>());
        {
            let runner = world.resource::<ClRunner>().clone();
            runner.0.step(25);
            let start = Instant::now();
            while wait_for(&runner.0.request(|s| s.current_tick())) < 25 {
                assert!(start.elapsed() < Duration::from_secs(10));
            }
        }
        world.resource_mut::<ClEditor>().draft.description = "my unapplied edit".into();
        let old_session = Arc::downgrade(&world.resource::<ClRunner>().0);
        world.run_system_once(exit).unwrap();
        assert!(!world.contains_resource::<ClRunner>() && !world.contains_resource::<ActiveSession>());
        assert!(old_session.upgrade().is_none(), "no reference keeps the old runner thread alive");
        assert!(world.resource::<ClResume>().is_some());
        world.run_system_once(enter).unwrap();
        assert!(!world.resource::<ClResume>().is_some());
        let runner = world.resource::<ClRunner>().clone();
        let (tick, population) = wait_for(&runner.0.request(|s| (s.current_tick(), s.total_population())));
        assert_eq!(tick, 25, "the world continues where it was left");
        assert!(population > 0);
        assert_eq!(world.resource::<ClEditor>().draft.description, "my unapplied edit");
    }

    #[test]
    fn presets_fill_the_picker() {
        let presets = ClScenarios::load();
        assert_eq!(presets.presets.len(), 7);
        assert_eq!(presets.default_preset().map(|s| s.id.as_str()), Some(DEFAULT_SCENARIO));
    }
}
