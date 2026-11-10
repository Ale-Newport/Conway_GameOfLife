//! The running simulation: the bundled CA-lab scenarios, one
//! [`Runner<LabSim>`](simulation_core::Runner) for every family, the
//! session handed to the shell and scenario loading.
//!
//! * Loading a scenario replaces the model inside the same runner
//!   (`Control::Replace`, even across families: [`LabSim`] holds any of
//!   the five models). The shell then gets a new [`LabSession`] around the
//!   same runner, so the metric plots start afresh for the new model.
//! * The runner captures the descriptor when it is spawned, but the
//!   descriptor follows the model (rules text with the running rule, the
//!   family's parameters and metrics). [`LabSession`] serves the current
//!   one from write-once slots (plain references, no locks), refreshed from
//!   the simulation thread whenever the parameters or the running rule
//!   change. Texts are made drawable by egui's fonts ([`displayable`]).

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use bevy::prelude::*;
use cellular_automata::scenario::{Scenario, builtin_scenarios};
use cellular_automata::{LifeSim, PatternLibrary};
use interface::{
    ActiveSession, AssetRoot, ModeScenarios, ScenarioEntry, ScenarioList, ScenarioSelected, SessionStatus, Toasts,
};
use simulation_core::{
    Control, HistoryConfig, Metrics, ModelDescriptor, ParamKind, ParamSet, ParamValue, Runner, RunnerConfig,
    RunnerStatus, SessionControl, SimError, Simulation, Snapshot, Speed,
};

use crate::MODE;
use crate::family::Family;
use crate::sim::{FillSettings, LabInfo, LabModel, LabSim};
use crate::tools::ToolSettings;
use crate::world::{CameraFit, FitQuery, FitTarget, LatestView, ViewRequestState};

/// Write-once descriptor slots per session.
pub const DESCRIPTOR_SLOTS: usize = 512;

/// Typed handle on the session's runner (the runner behind [`ActiveSession`]).
#[derive(Resource, Clone)]
pub struct LabRunner(pub Arc<Runner<LabSim>>);

/// Target speed when a family's scenario is opened (ticks per second).
pub fn default_speed(family: Family) -> Speed {
    Speed::TicksPerSecond(match family {
        // Slow enough to follow single generations of small patterns.
        Family::LifeLike | Family::Generations => 20.0,
        // Electrons move one cell per generation: follow them by eye.
        Family::Wireworld => 8.0,
        // Ant scenarios advance 10 to 50 steps per tick.
        Family::Ant => 30.0,
        // One row per tick: the diagram grows by 60 rows per second.
        Family::Wolfram => 60.0,
    })
}

/// A runner with the shell's Clear and Randomize controls enabled.
pub fn spawn_runner(sim: LabSim, speed: Speed) -> Arc<Runner<LabSim>> {
    Arc::new(
        Runner::spawn(sim, RunnerConfig { speed, ..default() })
            .with_clear(|sim: &mut LabSim| {
                if let Err(e) = sim.clear() {
                    warn!("clear failed: {e}");
                }
            })
            .with_randomize(|sim: &mut LabSim, seed| {
                if let Err(e) = sim.randomize(seed) {
                    warn!("randomize failed: {e}");
                }
            }),
    )
}

// ------------------------------------------------------------ descriptor

/// Characters of the model texts that egui's built-in fonts cannot draw,
/// with replacements.
const GLYPH_FALLBACKS: &[(char, &str)] = &[('∈', "in"), ('↦', "->"), ('→', "->"), ('∧', "AND")];

/// `text` with the characters egui's built-in fonts cannot draw replaced.
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

/// The descriptor as the shell shows it: drawable texts.
pub fn display_descriptor(descriptor: &ModelDescriptor) -> ModelDescriptor {
    let mut d = descriptor.clone();
    d.name = displayable(&d.name);
    d.summary = displayable(&d.summary);
    d.rules = displayable(&d.rules);
    for spec in &mut d.params {
        spec.label = displayable(&spec.label);
        spec.description = displayable(&spec.description);
        spec.group = displayable(&spec.group);
        if let ParamKind::Choice { options } = &mut spec.kind {
            for option in options {
                option.label = displayable(&option.label);
            }
        }
    }
    for metric in &mut d.metrics {
        metric.label = displayable(&metric.label);
        metric.description = displayable(&metric.description);
        metric.unit = displayable(&metric.unit);
    }
    d
}

/// What [`LabSession::update_descriptor`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorUpdate {
    /// The new descriptor is served from now on.
    Updated,
    /// It equals the served one.
    Unchanged,
    /// No slot is left: install a new session.
    Full,
}

/// The session handed to the shell: every control goes to the runner; the
/// descriptor follows the running model.
pub struct LabSession {
    runner: Arc<Runner<LabSim>>,
    slots: Box<[OnceLock<ModelDescriptor>]>,
    current: AtomicUsize,
}

impl LabSession {
    /// Wraps the runner, serving `first` until it is updated.
    pub fn new(runner: Arc<Runner<LabSim>>, first: ModelDescriptor) -> Self {
        let slots: Box<[OnceLock<ModelDescriptor>]> = (0..DESCRIPTOR_SLOTS).map(|_| OnceLock::new()).collect();
        let _ = slots[0].set(first);
        Self { runner, slots, current: AtomicUsize::new(0) }
    }

    /// The descriptor served now.
    pub fn current_descriptor(&self) -> &ModelDescriptor {
        let i = self.current.load(Ordering::Acquire);
        self.slots.get(i).and_then(OnceLock::get).unwrap_or_else(|| self.runner.descriptor())
    }

    /// Serves `descriptor` from now on if it differs from the current one.
    pub fn update_descriptor(&self, descriptor: ModelDescriptor) -> DescriptorUpdate {
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

impl SessionControl for LabSession {
    fn descriptor(&self) -> &ModelDescriptor {
        self.current_descriptor()
    }
    fn status(&self) -> RunnerStatus {
        SessionControl::status(&*self.runner)
    }
    fn play(&self) {
        SessionControl::play(&*self.runner);
    }
    fn pause(&self) {
        SessionControl::pause(&*self.runner);
    }
    fn toggle_play(&self) {
        self.runner.toggle_play();
    }
    fn step(&self, n: u64) {
        SessionControl::step(&*self.runner, n);
    }
    fn set_speed(&self, speed: Speed) {
        self.runner.set_speed(speed);
    }
    fn reset(&self) {
        self.runner.reset();
    }
    fn undo(&self) {
        self.runner.undo();
    }
    fn redo(&self) {
        self.runner.redo();
    }
    fn jump_to_history(&self, index: usize) {
        self.runner.jump_to_history(index);
    }
    fn set_history(&self, config: HistoryConfig) {
        self.runner.set_history(config);
    }
    fn set_param(&self, key: &str, value: ParamValue) {
        self.runner.set_param(key, value);
    }
    fn drain_metrics(&self) -> Vec<Metrics> {
        SessionControl::drain_metrics(&*self.runner)
    }
    fn take_errors(&self) -> Vec<String> {
        SessionControl::take_errors(&*self.runner)
    }
    fn request_snapshot(&self) -> Receiver<Result<Snapshot, SimError>> {
        self.runner.request_snapshot()
    }
    fn restore(&self, snapshot: Snapshot) {
        self.runner.restore(snapshot);
    }
    fn can_clear(&self) -> bool {
        self.runner.can_clear()
    }
    fn clear(&self) {
        self.runner.clear();
    }
    fn can_randomize(&self) -> bool {
        self.runner.can_randomize()
    }
    fn randomize(&self, seed: u64) {
        self.runner.randomize(seed);
    }
}

// ------------------------------------------------------------- scenarios

/// The CA-lab scenarios (every bundled scenario except Classic Conway's)
/// and the pattern library that builds them.
#[derive(Resource, Clone)]
pub struct Catalog {
    /// Scenarios in display order (grouped by family).
    pub scenarios: Vec<Scenario>,
    library: PatternLibrary,
}

impl Catalog {
    /// The bundled scenarios; patterns resolve against the asset catalog
    /// under `asset_root/patterns` when it can be read.
    pub fn load(asset_root: Option<&Path>) -> Self {
        let library = asset_root
            .and_then(|root| PatternLibrary::load(root.join("patterns")).ok())
            .unwrap_or_else(PatternLibrary::builtin);
        let mut scenarios: Vec<(usize, Scenario)> = builtin_scenarios()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| {
                let family = Family::of_model(&s.model)?;
                Some((Family::ALL.iter().position(|f| *f == family).unwrap_or(0), s))
            })
            .collect();
        scenarios.sort_by_key(|(rank, _)| *rank);
        Self { scenarios: scenarios.into_iter().map(|(_, s)| s).collect(), library }
    }

    /// Scenario by id.
    pub fn get(&self, id: &str) -> Option<&Scenario> {
        self.scenarios.iter().find(|s| s.id == id)
    }

    /// Family of a scenario.
    pub fn family_of(&self, id: &str) -> Option<Family> {
        self.get(id).and_then(|s| Family::of_model(&s.model))
    }

    /// Scenarios of a family.
    pub fn of_family(&self, family: Family) -> impl Iterator<Item = &Scenario> {
        self.scenarios.iter().filter(move |s| Family::of_model(&s.model) == Some(family))
    }

    /// Builds a scenario.
    ///
    /// # Errors
    /// Unknown ids and scenarios that fail to build (message for a toast).
    pub fn build(&self, id: &str) -> Result<LabSim, String> {
        let scenario = self.get(id).ok_or_else(|| format!("Unknown scenario `{id}`."))?;
        LabSim::from_scenario(scenario, &self.library).map_err(|e| format!("Could not build {}: {e}", scenario.name))
    }

    /// Entries for the top-bar picker: "Family · Name", grouped by family.
    pub fn entries(&self) -> Vec<ScenarioEntry> {
        self.scenarios
            .iter()
            .map(|s| {
                let family = Family::of_model(&s.model).unwrap_or_default();
                ScenarioEntry {
                    id: s.id.clone(),
                    title: format!("{} · {}", family.short_label(), s.name),
                    description: s.description.clone(),
                }
            })
            .collect()
    }
}

/// Scenario ids waiting to be loaded (top-bar picker, family buttons,
/// tutorial actions).
#[derive(Resource, Default, Debug)]
pub struct LoadRequests(pub Vec<String>);

/// The scenario shown (name and texts for the panels).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct LoadedScenario {
    /// Scenario id (empty for a loaded state of unknown origin).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the scenario shows.
    pub description: String,
    /// Sources.
    pub reference: String,
    /// Family.
    pub family: Family,
}

impl LoadedScenario {
    fn from_scenario(s: &Scenario) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            description: s.description.clone(),
            reference: s.reference.clone(),
            family: Family::of_model(&s.model).unwrap_or_default(),
        }
    }
}

/// Where the camera should look first for a freshly built model.
pub fn initial_fit(sim: &LabSim) -> FitTarget {
    let info = sim.info();
    if let Some(bounds) = info.world().and_then(|w| w.bounds()) {
        return FitTarget::Rect(bounds);
    }
    match sim.model() {
        LabModel::Life(life) => FitTarget::Rect(content_frame(life)),
        LabModel::Ant(ant) => {
            let (mut x0, mut y0, mut x1, mut y1) = (0i64, 0i64, 0i64, 0i64);
            for (i, a) in ant.ants().iter().enumerate() {
                if i == 0 {
                    (x0, y0, x1, y1) = (a.x, a.y, a.x, a.y);
                }
                (x0, y0, x1, y1) = (x0.min(a.x), y0.min(a.y), x1.max(a.x), y1.max(a.y));
            }
            FitTarget::Rect(cellular_automata::CellRect::new(x0 - 40, y0 - 40, x1 + 40, y1 + 40))
        }
        LabModel::Wolfram(_) => match info.detail {
            crate::sim::FamilyInfo::Wolfram { width, first_row, .. } => {
                FitTarget::DiagramTop { width, top: i64::try_from(first_row).unwrap_or(i64::MAX) }
            }
            _ => FitTarget::DiagramTop { width: 1, top: 0 },
        },
        _ => FitTarget::Rect(cellular_automata::CellRect::new(0, 0, 63, 63)),
    }
}

/// Frame around the live cells of an infinite Life world: their bounding
/// box with a margin of half its size (at least 24 cells), at least 64
/// cells wide; the home region when nothing is alive.
pub fn content_frame(life: &LifeSim) -> cellular_automata::CellRect {
    let cells = life.live_cells();
    let Some(bbox) = cellular_automata::CellRect::bounding(cells) else {
        return life.world().home();
    };
    let pad = |span: u64| ((span / 2).max(24) as i64).max(64u64.saturating_sub(span).div_ceil(2) as i64);
    let (px, py) = (pad(bbox.width()), pad(bbox.height()));
    cellular_automata::CellRect::new(bbox.x0 - px, bbox.y0 - py, bbox.x1 + px, bbox.y1 + py)
}

// --------------------------------------------------------------- systems

/// Requests to the simulation thread for the current descriptor.
#[derive(Resource, Default)]
pub struct DescriptorSync {
    pending: Mutex<Option<Receiver<ModelDescriptor>>>,
    last_signature: Option<String>,
    last_params: Option<ParamSet>,
}

impl DescriptorSync {
    fn reset(&mut self) {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.last_signature = None;
        self.last_params = None;
    }
}

/// What, besides the parameters, changes the descriptor: the running rule
/// and world (the rules text quotes them) and the family.
pub fn descriptor_signature(info: &LabInfo) -> String {
    format!(
        "{:?}|{}|{:?}|{}",
        info.family(),
        info.rule_text(),
        info.world(),
        match &info.detail {
            crate::sim::FamilyInfo::Life { generations_per_tick, engine, .. } =>
                format!("{generations_per_tick} {engine:?}"),
            _ => String::new(),
        }
    )
}

/// The [`LabSession`] currently handed to the shell as [`ActiveSession`].
#[derive(Resource, Clone)]
pub struct ServedSession(pub Arc<LabSession>);

fn install(commands: &mut Commands, runner: &Arc<Runner<LabSim>>, descriptor: ModelDescriptor) {
    let session = Arc::new(LabSession::new(runner.clone(), descriptor));
    commands.insert_resource(ActiveSession::new(session.clone()));
    commands.insert_resource(ServedSession(session));
}

/// Resources reset whenever another model or scenario is shown.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct ViewState<'w> {
    latest: ResMut<'w, LatestView>,
    request: ResMut<'w, ViewRequestState>,
    fit: ResMut<'w, CameraFit>,
    query: Res<'w, FitQuery>,
    sync: ResMut<'w, DescriptorSync>,
}

impl ViewState<'_> {
    /// After a load (`scenario`) the previous view stays shown until the new
    /// model publishes; after leaving the mode (`None`) everything is cleared.
    fn restart(&mut self, scenario: Option<&str>, fit: Option<FitTarget>) {
        match scenario {
            Some(id) => self.latest.await_scenario(id),
            None => *self.latest = LatestView::default(),
        }
        self.request.last = None;
        self.sync.reset();
        // A framing measured on the previous model must not override the new one.
        self.query.clear();
        self.fit.refresh = false;
        if let Some(target) = fit {
            self.fit.request(target);
        }
    }
}

/// `Startup`: reads the scenarios and fills the top-bar picker.
pub(crate) fn init_catalog(mut commands: Commands, root: Option<Res<AssetRoot>>, mut scenarios: ResMut<ModeScenarios>) {
    let root = root.as_deref().map(|r| r.0.as_path());
    commands.insert_resource(crate::tools::Circuits::load(root));
    let catalog = Catalog::load(root);
    let selected = scenarios
        .get(MODE)
        .and_then(|l| l.selected.clone())
        .or_else(|| Some(Family::LifeLike.default_scenario().to_string()));
    scenarios.set(MODE, ScenarioList { entries: catalog.entries(), selected });
    commands.insert_resource(catalog);
}

/// `OnEnter`: opens the selected (or default) scenario on a new runner.
pub(crate) fn enter_session(
    mut commands: Commands,
    catalog: Option<Res<Catalog>>,
    mut scenarios: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    tools: Res<ToolSettings>,
    mut state: ViewState,
) {
    let catalog = match catalog {
        Some(c) => c.clone(),
        None => {
            let c = Catalog::load(None);
            commands.insert_resource(c.clone());
            c
        }
    };
    let wanted = scenarios
        .get(MODE)
        .and_then(|l| l.selected.clone())
        .unwrap_or_else(|| Family::LifeLike.default_scenario().to_string());
    let id = if catalog.get(&wanted).is_some() { wanted } else { Family::LifeLike.default_scenario().to_string() };
    match catalog.build(&id) {
        Ok(mut sim) => {
            sim.set_fill(FillSettings { density: tools.density, state: tools.state });
            let family = sim.family();
            let descriptor = display_descriptor(&sim.descriptor());
            let fit = initial_fit(&sim);
            let runner = spawn_runner(sim, default_speed(family));
            install(&mut commands, &runner, descriptor);
            commands.insert_resource(LabRunner(runner));
            if let Some(s) = catalog.get(&id) {
                commands.insert_resource(LoadedScenario::from_scenario(s));
            }
            scenarios.mark_selected(MODE, &id);
            state.restart(Some(&id), Some(fit));
        }
        Err(e) => toasts.error(e),
    }
}

/// `OnExit`: drops the runner (its thread stops when the last handle goes).
pub(crate) fn exit_session(mut commands: Commands, mut state: ViewState) {
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<ServedSession>();
    commands.remove_resource::<LabRunner>();
    commands.remove_resource::<LoadedScenario>();
    state.restart(None, None);
}

/// Queues scenarios picked in the top bar.
pub(crate) fn collect_scenario_picks(mut picked: MessageReader<ScenarioSelected>, mut requests: ResMut<LoadRequests>) {
    for pick in picked.read().filter(|p| p.mode == MODE) {
        requests.0.push(pick.id.clone());
    }
}

/// Loads the last queued scenario into the running runner.
#[allow(clippy::too_many_arguments)]
pub(crate) fn load_requested_scenarios(
    mut commands: Commands,
    mut requests: ResMut<LoadRequests>,
    catalog: Option<Res<Catalog>>,
    runner: Option<Res<LabRunner>>,
    mut scenarios: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    tools: Res<ToolSettings>,
    mut state: ViewState,
) {
    let Some(id) = requests.0.drain(..).next_back() else { return };
    let (Some(catalog), Some(runner)) = (catalog, runner) else { return };
    let mut sim = match catalog.build(&id) {
        Ok(sim) => sim,
        Err(e) => {
            toasts.error(e);
            return;
        }
    };
    sim.set_fill(FillSettings { density: tools.density, state: tools.state });
    let family = sim.family();
    let descriptor = display_descriptor(&sim.descriptor());
    let fit = initial_fit(&sim);
    runner.0.send(Control::Replace(Box::new(sim)));
    runner.0.send(Control::SetSpeed(default_speed(family)));
    runner.0.send(Control::RequestView);
    install(&mut commands, &runner.0, descriptor);
    if let Some(s) = catalog.get(&id) {
        commands.insert_resource(LoadedScenario::from_scenario(s));
    }
    scenarios.mark_selected(MODE, &id);
    state.restart(Some(&id), Some(fit));
}

/// Keeps the served descriptor and [`LoadedScenario`] in step with the
/// running model (parameter edits, rule changes taking effect, undo, a
/// loaded save of another family).
#[allow(clippy::too_many_arguments)]
pub(crate) fn sync_live_model(
    mut commands: Commands,
    runner: Option<Res<LabRunner>>,
    session: Option<Res<ServedSession>>,
    status: Res<SessionStatus>,
    latest: Res<LatestView>,
    catalog: Option<Res<Catalog>>,
    loaded: Option<ResMut<LoadedScenario>>,
    mut scenarios: ResMut<ModeScenarios>,
    mut sync: ResMut<DescriptorSync>,
) {
    let (Some(runner), Some(session)) = (runner, session) else { return };
    // A finished descriptor request.
    let received = sync.pending.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(Receiver::try_recv);
    match received {
        Some(Ok(descriptor)) => {
            *sync.pending.lock().unwrap_or_else(PoisonError::into_inner) = None;
            // Update the served session in place, or install a new one when
            // its slots are used up.
            if session.0.update_descriptor(descriptor.clone()) == DescriptorUpdate::Full {
                install(&mut commands, &runner.0, descriptor);
            }
        }
        Some(Err(TryRecvError::Disconnected)) => {
            *sync.pending.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
        Some(Err(TryRecvError::Empty)) => return,
        None => {}
    }
    let Some(view) = latest.view.as_ref() else { return };
    let signature = descriptor_signature(&view.info);
    let params = status.get().map(|s| s.params.clone());
    if sync.last_signature.as_deref() != Some(signature.as_str()) || sync.last_params != params {
        sync.last_signature = Some(signature);
        sync.last_params = params;
        *sync.pending.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(runner.0.request(|sim| display_descriptor(&sim.descriptor())));
    }
    // A state restored from elsewhere (a save, undo across a load) may come from another scenario.
    if let (Some(mut loaded), Some(catalog)) = (loaded, catalog) {
        let info = &view.info;
        let family = info.family();
        if loaded.id != info.scenario || loaded.family != family {
            match catalog.get(&info.scenario).filter(|s| Family::of_model(&s.model) == Some(family)) {
                Some(s) => {
                    *loaded = LoadedScenario::from_scenario(s);
                    scenarios.mark_selected(MODE, &s.id);
                }
                None => {
                    *loaded = LoadedScenario {
                        id: info.scenario.clone(),
                        name: format!("{} (loaded state)", family.label()),
                        description: String::new(),
                        reference: String::new(),
                        family,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use std::sync::Weak;

    fn assets() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    fn world() -> World {
        let mut world = World::new();
        world.insert_resource(AssetRoot(assets()));
        world.init_resource::<ModeScenarios>();
        world.init_resource::<Toasts>();
        world.init_resource::<SessionStatus>();
        world.init_resource::<LoadRequests>();
        world.init_resource::<LatestView>();
        world.init_resource::<ViewRequestState>();
        world.init_resource::<CameraFit>();
        world.init_resource::<FitQuery>();
        world.init_resource::<DescriptorSync>();
        world.init_resource::<crate::tools::ToolSettings>();
        world.run_system_once(init_catalog).unwrap();
        world
    }

    fn runner_of(world: &World) -> Weak<Runner<LabSim>> {
        Arc::downgrade(&world.resource::<LabRunner>().0)
    }

    #[test]
    fn catalog_holds_every_ca_lab_scenario_grouped_by_family() {
        let catalog = Catalog::load(Some(&assets()));
        assert_eq!(catalog.scenarios.len(), cellular_automata::scenario::CA_LAB_SCENARIO_FILES.len());
        for s in &catalog.scenarios {
            assert!(catalog.build(&s.id).is_ok(), "{} builds", s.id);
        }
        let entries = catalog.entries();
        assert_eq!(entries[0].title, "Life-like · Anneal (twisted majority)");
        let ranks: Vec<usize> = catalog
            .scenarios
            .iter()
            .map(|s| Family::ALL.iter().position(|f| Some(*f) == Family::of_model(&s.model)).unwrap())
            .collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]));
        assert!(catalog.get("glider-gun-showcase").is_none(), "Classic Conway scenarios belong to their mode");
        assert!(catalog.build("nope").is_err());
        // Without the asset folder the AND gate is missing: the circuits scenario cannot be built.
        assert!(Catalog::load(None).build("wireworld-circuits").is_err());
    }

    /// Enter, leave and enter again (also while running): every exit drops
    /// the last handle on the runner, whose `Drop` stops and joins its thread.
    #[test]
    fn reentering_never_leaks_runners() {
        let mut world = world();
        world.run_system_once(enter_session).unwrap();
        let first = runner_of(&world);
        first.upgrade().unwrap().play();
        assert!(world.contains_resource::<ActiveSession>() && world.contains_resource::<ServedSession>());
        assert_eq!(world.resource::<LoadedScenario>().id, "highlife-replicator");
        world.run_system_once(exit_session).unwrap();
        assert!(first.upgrade().is_none(), "the runner (and its thread) is gone after OnExit");
        assert!(!world.contains_resource::<ActiveSession>() && !world.contains_resource::<LabRunner>());
        world.resource_mut::<ModeScenarios>().mark_selected(MODE, "turmite-llrr");
        world.run_system_once(enter_session).unwrap();
        assert_eq!(world.resource::<LoadedScenario>().family, Family::Ant);
        let second = runner_of(&world);
        world.run_system_once(exit_session).unwrap();
        assert!(second.upgrade().is_none());
    }

    /// Scenarios of any family replace the model inside the same runner and
    /// install a new session (fresh plots) serving the new descriptor.
    #[test]
    fn scenario_loads_reuse_the_runner() {
        let mut world = world();
        world.run_system_once(enter_session).unwrap();
        let first = runner_of(&world);
        let session_before = Arc::as_ptr(&world.resource::<ServedSession>().0);
        world.resource_mut::<LoadRequests>().0.extend(["nope".to_string(), "wolfram-rule-110".to_string()]);
        world.run_system_once(load_requested_scenarios).unwrap();
        assert!(Weak::ptr_eq(&first, &runner_of(&world)), "Control::Replace keeps the runner");
        let served = world.resource::<ServedSession>().0.clone();
        assert_ne!(Arc::as_ptr(&served), session_before, "a new session clears the plots");
        assert_eq!(served.current_descriptor().name, "Elementary cellular automata");
        assert_eq!(world.resource::<LoadedScenario>().family, Family::Wolfram);
        assert_eq!(
            world.resource::<ModeScenarios>().get(MODE).and_then(|l| l.selected.clone()).as_deref(),
            Some("wolfram-rule-110")
        );
        assert!(world.resource::<LoadRequests>().0.is_empty());
        // The runner now runs rule 110.
        let rx = first.upgrade().unwrap().request(|sim| (sim.family(), sim.info().rule_text()));
        let reply = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(reply, (Family::Wolfram, "Rule 110".to_string()));
        drop(served);
        world.run_system_once(exit_session).unwrap();
        assert!(first.upgrade().is_none());
    }

    #[test]
    fn served_descriptors_follow_the_model() {
        let catalog = Catalog::load(Some(&assets()));
        let sim = catalog.build("seeds").unwrap();
        let runner = spawn_runner(sim, Speed::TicksPerSecond(10.0));
        let first = display_descriptor(&runner.descriptor().clone());
        let session = LabSession::new(runner.clone(), first.clone());
        assert_eq!(session.update_descriptor(first.clone()), DescriptorUpdate::Unchanged);
        let mut changed = first.clone();
        changed.rules.push_str("\nchanged");
        assert_eq!(session.update_descriptor(changed.clone()), DescriptorUpdate::Updated);
        assert_eq!(SessionControl::descriptor(&session).rules, changed.rules);
        let mut last = DescriptorUpdate::Updated;
        for i in 0..DESCRIPTOR_SLOTS {
            let mut d = first.clone();
            d.summary = format!("v{i}");
            last = session.update_descriptor(d);
        }
        assert_eq!(last, DescriptorUpdate::Full, "slots run out: a new session is installed");
        assert_eq!(first.id, crate::sim::LAB_MODEL_ID);
    }

    #[test]
    fn signatures_ignore_the_tick() {
        let catalog = Catalog::load(Some(&assets()));
        let mut sim = catalog.build("wolfram-rule-30").unwrap();
        let before = descriptor_signature(&sim.info());
        sim.step();
        assert_eq!(descriptor_signature(&sim.info()), before);
        sim.set_param("rule", ParamValue::Int(90)).unwrap();
        sim.step();
        assert_ne!(descriptor_signature(&sim.info()), before, "the running rule changed");
    }

    #[test]
    fn content_frames_surround_the_pattern() {
        let catalog = Catalog::load(Some(&assets()));
        let sim = catalog.build("highlife-replicator").unwrap();
        let FitTarget::Rect(r) = initial_fit(&sim) else { panic!() };
        // The 5 × 5 replicator at (120, 120): at least 64 cells around it.
        assert!(r.contains(120, 120) && r.contains(124, 124));
        assert!(r.width() >= 64 && r.height() >= 64, "{r:?}");
        assert!(matches!(
            initial_fit(&catalog.build("wolfram-rule-30").unwrap()),
            FitTarget::DiagramTop { width: 1025, top: 0 }
        ));
        assert!(matches!(
            initial_fit(&catalog.build("wireworld-circuits").unwrap()),
            FitTarget::Rect(r) if r == cellular_automata::CellRect::new(0, 0, 63, 71)
        ));
    }
}
