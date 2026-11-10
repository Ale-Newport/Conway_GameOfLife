//! The running simulation: built-in scenarios, the [`Runner`] behind
//! [`ActiveSession`], scenario loading and a live copy of the rule fetched
//! from the simulation thread.
//!
//! Every scenario is loaded into the same runner (`Control::Replace`); the
//! runner publishes the new model's descriptor, so the shell's Parameters
//! and Rules tabs follow it, also after loading a save of another scenario.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use continuous::scenario::{self, LibraryCreature, Scenario, ScenarioHeader};
use continuous::{ContinuousSim, ModelKind, Rule};
use interface::{ActiveSession, ModeScenarios, ScenarioEntry, ScenarioList, ScenarioSelected, SessionStatus, Toasts};
use simulation_core::{
    Control, ModelDescriptor, ParamKind, ParamSet, ParamValue, Runner, RunnerConfig, Simulation, Speed,
};

use crate::MODE;
use crate::world::{CameraFit, LatestView, ViewRequestState, ViewSettings};

/// Scenario opened the first time the mode is entered.
pub const DEFAULT_SCENARIO: &str = "lenia.orbium";

/// Typed handle on the session's runner (the same runner as
/// [`ActiveSession`]).
#[derive(Resource, Clone)]
pub struct ContinuousRunner(pub Arc<Runner<ContinuousSim>>);

/// Model families offered by the mode (the user-facing grouping of the
/// scenarios).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// Single-channel, single-kernel Lenia.
    Lenia,
    /// Lenia with several channels and/or kernels (Chan 2020).
    ExpandedLenia,
    /// Rafler's SmoothLife.
    SmoothLife,
    /// Mass-conserving Flow-Lenia.
    FlowLenia,
}

impl Family {
    /// Every family, in display order.
    pub const ALL: [Family; 4] = [Family::Lenia, Family::ExpandedLenia, Family::SmoothLife, Family::FlowLenia];

    /// Family of a rule.
    pub fn of(rule: &Rule) -> Self {
        match rule {
            Rule::Lenia(r) if r.channels > 1 || r.kernels.len() > 1 => Family::ExpandedLenia,
            Rule::Lenia(_) => Family::Lenia,
            Rule::SmoothLife(_) => Family::SmoothLife,
            Rule::FlowLenia(_) => Family::FlowLenia,
        }
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Family::Lenia => "Lenia",
            Family::ExpandedLenia => "Expanded Lenia",
            Family::SmoothLife => "SmoothLife",
            Family::FlowLenia => "Flow-Lenia",
        }
    }

    /// Scenario loaded when the family is picked in the tools panel.
    pub fn default_scenario(self) -> &'static str {
        match self {
            Family::Lenia => "lenia.orbium",
            Family::ExpandedLenia => "lenia.tessellatium_gyrans",
            Family::SmoothLife => "smoothlife.glider",
            Family::FlowLenia => "flow_lenia.demo",
        }
    }

    /// One-sentence description for tooltips.
    pub fn blurb(self) -> &'static str {
        match self {
            Family::Lenia => "One channel, one ring kernel and a smooth growth function (Chan 2019).",
            Family::ExpandedLenia => {
                "Several channels and kernels, each kernel reading one channel and feeding another (Chan 2020)."
            }
            Family::SmoothLife => "Rafler's continuous Game of Life: inner disc and outer annulus fillings (2011).",
            Family::FlowLenia => {
                "Matter flows along affinity gradients and total mass is conserved (Plantec et al. 2023)."
            }
        }
    }
}

/// Built-in scenarios and the creature library, parsed once.
#[derive(Resource, Clone)]
pub struct Catalog {
    /// Scenarios in file order.
    pub scenarios: Vec<Scenario>,
    /// Creatures defined by the scenarios.
    pub creatures: Vec<LibraryCreature>,
}

impl Catalog {
    /// Parses the scenarios embedded in the `continuous` crate.
    pub fn builtin() -> Self {
        let scenarios = scenario::builtin();
        let creatures = scenarios.iter().filter_map(|s| s.creatures().ok()).flatten().collect();
        Self { scenarios, creatures }
    }

    /// Scenario by id.
    pub fn get(&self, id: &str) -> Option<&Scenario> {
        self.scenarios.iter().find(|s| s.header.id == id)
    }

    /// Display name of a scenario id (the id itself when unknown).
    pub fn name_of<'a>(&'a self, id: &'a str) -> &'a str {
        self.get(id).map_or(id, |s| s.header.name.as_str())
    }

    /// Entries for the top-bar picker, grouped by family.
    pub fn entries(&self) -> Vec<ScenarioEntry> {
        let mut entries: Vec<(usize, ScenarioEntry)> = self
            .scenarios
            .iter()
            .map(|s| {
                let family = Family::of(&s.rule);
                let rank = Family::ALL.iter().position(|f| *f == family).unwrap_or(0);
                let entry = ScenarioEntry {
                    id: s.header.id.clone(),
                    title: if s.header.name.starts_with(family.label()) {
                        s.header.name.clone()
                    } else {
                        format!("{} · {}", family.label(), s.header.name)
                    },
                    description: s.header.description.clone(),
                };
                (rank, entry)
            })
            .collect();
        entries.sort_by_key(|(rank, _)| *rank);
        entries.into_iter().map(|(_, e)| e).collect()
    }
}

/// The world being shown: the scenario identity and the live rule and size
/// (refreshed from the simulation thread when parameters change).
#[derive(Resource, Clone, Debug)]
pub struct LoadedWorld {
    /// Scenario the world was built from.
    pub header: ScenarioHeader,
    /// Current rule (including parameter edits).
    pub rule: Rule,
    /// World width in cells.
    pub width: usize,
    /// World height in cells.
    pub height: usize,
}

impl LoadedWorld {
    /// World described by a freshly loaded scenario.
    pub fn from_scenario(s: &Scenario) -> Self {
        Self { header: s.header.clone(), rule: s.rule.clone(), width: s.width, height: s.height }
    }

    /// Model kind.
    pub fn kind(&self) -> ModelKind {
        self.rule.kind()
    }

    /// Number of channels.
    pub fn channels(&self) -> usize {
        self.rule.channels()
    }

    /// Family for display.
    pub fn family(&self) -> Family {
        Family::of(&self.rule)
    }

    /// Largest value an edit may write.
    pub fn edit_limit(&self) -> f32 {
        self.rule.edit_limit()
    }
}

/// Scenario ids waiting to be loaded (from the top-bar picker, the model
/// buttons and tutorial actions).
#[derive(Resource, Default, Debug)]
pub struct LoadRequests(pub Vec<String>);

/// What the simulation thread reports about the live world.
#[derive(Clone, Debug)]
pub(crate) struct LiveInfo {
    rule: Rule,
    header: ScenarioHeader,
    width: usize,
    height: usize,
}

/// Bookkeeping of the asynchronous requests to the simulation thread.
#[derive(Resource, Default)]
pub struct LiveSync {
    pending: Mutex<Option<Receiver<LiveInfo>>>,
    last_params: Option<ParamSet>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LiveSync {
    fn reset(&mut self) {
        *lock(&self.pending) = None;
        self.last_params = None;
    }
}

/// Target speed when a model is opened.
pub fn default_speed(kind: ModelKind) -> Speed {
    match kind {
        // T = 10 for most presets: 30 ticks/s are 3 time units per second.
        ModelKind::Lenia | ModelKind::FlowLenia => Speed::TicksPerSecond(30.0),
        // The smooth glider jumps about 5 cells per discrete step.
        ModelKind::SmoothLife => Speed::TicksPerSecond(10.0),
    }
}

/// A runner serving [`display_descriptor`], with the generic Clear and
/// Randomize controls enabled.
pub fn spawn_runner(sim: ContinuousSim, speed: Speed) -> Arc<Runner<ContinuousSim>> {
    Arc::new(
        Runner::spawn_with_descriptor(sim, RunnerConfig { speed, ..default() }, |sim| {
            display_descriptor(&sim.descriptor(), &sim.params())
        })
        .with_clear(|sim: &mut ContinuousSim| sim.clear())
        .with_randomize(|sim: &mut ContinuousSim, seed| sim.randomize(seed)),
    )
}

/// Mode resources that start over with a new session.
#[derive(SystemParam)]
pub struct SessionState<'w> {
    latest: ResMut<'w, LatestView>,
    request: ResMut<'w, ViewRequestState>,
    live: ResMut<'w, LiveSync>,
    fit: ResMut<'w, CameraFit>,
    settings: ResMut<'w, ViewSettings>,
}

impl SessionState<'_> {
    /// Forgets everything tied to the previous runner or world.
    fn restart(&mut self, refit: bool) {
        *self.latest = LatestView::default();
        self.request.last = None;
        self.live.reset();
        if refit {
            self.fit.request(true);
        }
    }
}

fn install(commands: &mut Commands, sim: ContinuousSim, speed: Speed) {
    let runner = spawn_runner(sim, speed);
    commands.insert_resource(ActiveSession::new(runner.clone()));
    commands.insert_resource(ContinuousRunner(runner));
}

/// Replacements for characters of the model's texts that egui's built-in
/// fonts cannot draw (they would render as boxes).
const GLYPH_FALLBACKS: &[(char, &str)] = &[
    ('ₐ', "_a"),
    ('ᵢ', "_i"),
    ('ₙ', "_n"),
    ('ₘ', "_m"),
    ('→', "->"),
    ('←', "<-"),
    ('∗', "*"),
    ('∇', "grad "),
    ('∈', "in"),
    ('∝', "~"),
    ('⌊', "floor("),
    ('⌋', ")"),
    ('∓', "-/+"),
    ('‖', "|"),
];

/// `text` with the characters egui cannot draw replaced (`∗` by `*`, `rₐ` by `r_a`...) and negative
/// superscript exponents (`10⁻⁶`) written as `10^-6`.
pub fn displayable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut exponent = false;
    for c in text.chars() {
        if c == '⁻' {
            out.push_str("^-");
            exponent = true;
            continue;
        }
        if exponent && let Some(d) = "⁰¹²³⁴⁵⁶⁷⁸⁹".chars().position(|x| x == c) {
            out.push(char::from(b'0' + d as u8));
            continue;
        }
        exponent = false;
        match GLYPH_FALLBACKS.iter().find(|(x, _)| *x == c) {
            Some((_, replacement)) => out.push_str(replacement),
            None => out.push(c),
        }
    }
    out
}

/// The descriptor as the shell should show it:
///
/// * texts use only characters egui's built-in fonts can draw
///   ([`displayable`]);
/// * the tick unit reads "time unit" (the shell pluralises it);
/// * linear float sliders whose current value lies off their step grid are
///   not stepped. egui re-applies a slider's step on every frame
///   (`SliderClamping::Always`), so drawing such a parameter would silently
///   change it (SmoothLife's `α_n = 0.028` on the grid `0.0001 + k·0.001`
///   would become 0.0281) and send it to the model.
pub fn display_descriptor(descriptor: &ModelDescriptor, params: &ParamSet) -> ModelDescriptor {
    let mut d = descriptor.clone();
    d.name = displayable(&d.name);
    d.summary = displayable(&d.summary);
    d.rules = displayable(&d.rules);
    if d.tick_unit.starts_with("time unit") {
        d.tick_unit = "time unit".to_string();
    }
    for spec in &mut d.params {
        spec.label = displayable(&spec.label);
        spec.description = displayable(&spec.description);
        spec.group = displayable(&spec.group);
        let value = params.get(&spec.key).and_then(ParamValue::as_f64).or_else(|| spec.default.as_f64());
        match &mut spec.kind {
            ParamKind::Float { min, step, logarithmic: false, .. } if *step > 0.0 => {
                if let Some(v) = value
                    && *min + ((v - *min) / *step).round() * *step != v
                {
                    *step = 0.0;
                }
            }
            ParamKind::Choice { options } => {
                for option in options {
                    option.label = displayable(&option.label);
                }
            }
            _ => {}
        }
    }
    for metric in &mut d.metrics {
        metric.label = displayable(&metric.label);
        metric.description = displayable(&metric.description);
        metric.unit = displayable(&metric.unit);
    }
    d
}

/// `OnEnter`: opens the selected (or default) scenario.
pub(crate) fn enter_session(
    mut commands: Commands,
    catalog: Res<Catalog>,
    mut scenarios: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    mut state: SessionState,
) {
    let wanted = scenarios.get(MODE).and_then(|l| l.selected.clone()).unwrap_or_else(|| DEFAULT_SCENARIO.into());
    let scenario = catalog.get(&wanted).or_else(|| catalog.get(DEFAULT_SCENARIO)).or(catalog.scenarios.first());
    let Some(scenario) = scenario.cloned() else {
        toasts.error("No continuous scenario is available.");
        return;
    };
    match ContinuousSim::from_scenario(scenario.clone()) {
        Ok(sim) => {
            install(&mut commands, sim, default_speed(scenario.rule.kind()));
            state.settings.adapt_to(&scenario.rule, true);
            commands.insert_resource(LoadedWorld::from_scenario(&scenario));
            scenarios.mark_selected(MODE, &scenario.header.id);
            state.restart(true);
        }
        Err(e) => toasts.error(format!("Could not build {}: {e}", scenario.header.name)),
    }
}

/// `OnExit`: stops the simulation thread.
pub(crate) fn exit_session(mut commands: Commands, mut state: SessionState) {
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<ContinuousRunner>();
    commands.remove_resource::<LoadedWorld>();
    state.restart(false);
}

/// Fills the top-bar scenario picker.
pub(crate) fn fill_scenarios(catalog: Res<Catalog>, mut scenarios: ResMut<ModeScenarios>) {
    let selected = scenarios.get(MODE).and_then(|l| l.selected.clone()).or_else(|| Some(DEFAULT_SCENARIO.into()));
    scenarios.set(MODE, ScenarioList { entries: catalog.entries(), selected });
}

/// Queues scenarios picked in the top bar.
pub(crate) fn collect_scenario_picks(mut picked: MessageReader<ScenarioSelected>, mut requests: ResMut<LoadRequests>) {
    for pick in picked.read().filter(|p| p.mode == MODE) {
        requests.0.push(pick.id.clone());
    }
}

/// Loads queued scenarios into the running runner (`Control::Replace`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn load_requested_scenarios(
    mut commands: Commands,
    mut requests: ResMut<LoadRequests>,
    catalog: Res<Catalog>,
    runner: Option<Res<ContinuousRunner>>,
    loaded: Option<Res<LoadedWorld>>,
    mut scenarios: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    mut state: SessionState,
) {
    // Only the last request matters when several arrive in one frame.
    let Some(id) = requests.0.drain(..).next_back() else { return };
    let Some(scenario) = catalog.get(&id).cloned() else {
        toasts.error(format!("Unknown continuous scenario `{id}`."));
        return;
    };
    let sim = match ContinuousSim::from_scenario(scenario.clone()) {
        Ok(sim) => sim,
        Err(e) => {
            toasts.error(format!("Could not build {}: {e}", scenario.header.name));
            return;
        }
    };
    let previous_kind = loaded.as_ref().map(|l| l.kind());
    let kind_changed = previous_kind != Some(scenario.rule.kind());
    match runner {
        Some(runner) => {
            runner.0.send(Control::Replace(Box::new(sim)));
            if kind_changed {
                runner.0.send(Control::SetSpeed(default_speed(scenario.rule.kind())));
            }
            runner.0.send(Control::RequestView);
            // Inserted again, the session clears the metric plots of the previous world.
            commands.insert_resource(ActiveSession::new(runner.0.clone()));
        }
        None => install(&mut commands, sim, default_speed(scenario.rule.kind())),
    }
    state.settings.adapt_to(&scenario.rule, kind_changed);
    commands.insert_resource(LoadedWorld::from_scenario(&scenario));
    scenarios.mark_selected(MODE, &scenario.header.id);
    state.restart(true);
}

/// Keeps [`LoadedWorld`] in step with the simulation thread: after any
/// parameter change (including undo and loading a save) the live rule,
/// size and scenario are fetched.
pub(crate) fn sync_live_world(
    runner: Option<Res<ContinuousRunner>>,
    status: Res<SessionStatus>,
    loaded: Option<ResMut<LoadedWorld>>,
    mut scenarios: ResMut<ModeScenarios>,
    mut state: SessionState,
) {
    let (Some(runner), Some(mut loaded)) = (runner, loaded) else { return };
    let Some(current) = status.get() else { return };

    let received = lock(&state.live.pending).as_ref().map(Receiver::try_recv);
    match received {
        Some(Ok(info)) => {
            *lock(&state.live.pending) = None;
            let resized = (info.width, info.height) != (loaded.width, loaded.height);
            if info.header.id != loaded.header.id && scenarios.mark_selected(MODE, &info.header.id) {
                debug!("live scenario is now {}", info.header.id);
            }
            loaded.header = info.header;
            loaded.rule = info.rule;
            loaded.width = info.width;
            loaded.height = info.height;
            if resized {
                state.fit.request(true);
            }
        }
        Some(Err(TryRecvError::Disconnected)) => *lock(&state.live.pending) = None,
        Some(Err(TryRecvError::Empty)) => {}
        None => {
            if state.live.last_params.as_ref() != Some(&current.params) {
                state.live.last_params = Some(current.params.clone());
                *lock(&state.live.pending) = Some(runner.0.request(|sim| LiveInfo {
                    rule: sim.rule().clone(),
                    header: sim.scenario().header.clone(),
                    width: sim.width(),
                    height: sim.height(),
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Weak;

    #[test]
    fn catalog_lists_every_family() {
        let catalog = Catalog::builtin();
        assert_eq!(catalog.scenarios.len(), scenario::BUILTIN_FILES.len());
        for family in Family::ALL {
            let s = catalog.get(family.default_scenario()).expect("default scenario exists");
            assert_eq!(Family::of(&s.rule), family, "{}", s.header.id);
        }
        let entries = catalog.entries();
        assert_eq!(entries.len(), catalog.scenarios.len());
        assert!(entries[0].title.starts_with("Lenia · "));
        assert_eq!(entries.last().unwrap().title, "Flow-Lenia membrane", "no repeated family name");
        // Families appear in display order.
        let ranks: Vec<usize> = entries
            .iter()
            .map(|e| {
                let f = Family::of(&catalog.get(&e.id).unwrap().rule);
                Family::ALL.iter().position(|x| *x == f).unwrap()
            })
            .collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]));
        assert!(catalog.creatures.iter().any(|c| c.name.starts_with("Orbium")));
        assert_eq!(catalog.name_of("lenia.fish"), "K=3 Fish");
        assert_eq!(catalog.name_of("nope"), "nope");
    }

    #[test]
    fn display_descriptors_draw_and_do_not_snap() {
        let catalog = Catalog::builtin();
        let ctx = bevy_egui::egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.label("x");
        });
        output.textures_delta.clear();
        let font = bevy_egui::egui::FontId::proportional(14.0);
        let mut missing: Vec<char> = Vec::new();
        for scenario in &catalog.scenarios {
            let sim = ContinuousSim::from_scenario(scenario.clone()).unwrap();
            let params = sim.params();
            let d = display_descriptor(&sim.descriptor(), &params);
            let mut texts = vec![d.name.clone(), d.summary.clone(), d.rules.clone(), d.tick_unit.clone()];
            for spec in &d.params {
                texts.extend([spec.label.clone(), spec.description.clone(), spec.group.clone()]);
                if let ParamKind::Choice { options } = &spec.kind {
                    texts.extend(options.iter().map(|o| o.label.clone()));
                }
                // No slider would move its value when drawn.
                if let (ParamKind::Float { min, step, logarithmic: false, .. }, Some(v)) =
                    (&spec.kind, params.get(&spec.key).and_then(ParamValue::as_f64))
                    && *step > 0.0
                {
                    assert_eq!(min + ((v - min) / step).round() * step, v, "{} {}", scenario.header.id, spec.key);
                }
            }
            for m in &d.metrics {
                texts.extend([m.label.clone(), m.description.clone(), m.unit.clone()]);
            }
            for text in texts {
                for c in text.chars().filter(|c| !c.is_ascii()) {
                    if !ctx.fonts_mut(|f| f.has_glyphs(&font, &c.to_string())) && !missing.contains(&c) {
                        missing.push(c);
                    }
                }
            }
        }
        assert!(missing.is_empty(), "characters that cannot be drawn: {missing:?}");
        // The SmoothLife sigmoid widths are off the 0.001 grid counted from 0.0001.
        let sl = ContinuousSim::from_scenario(catalog.get("smoothlife.glider").unwrap().clone()).unwrap();
        let d = display_descriptor(&sl.descriptor(), &sl.params());
        let step = |key: &str| match &d.param(key).unwrap().kind {
            ParamKind::Float { step, .. } => *step,
            _ => f64::NAN,
        };
        assert_eq!(step("alpha_n"), 0.0);
        assert_eq!(step("b1"), 0.001, "values on the grid keep their step");
        assert_eq!(d.param("outer_radius").unwrap().label, "Outer radius r_a");
        assert_eq!(displayable("~10⁻⁶ and K ∗ A"), "~10^-6 and K * A");
        assert_eq!(displayable("10⁻¹²·x²"), "10^-12·x²");
    }

    fn lifecycle_world() -> World {
        let mut world = World::new();
        world.insert_resource(Catalog::builtin());
        world.init_resource::<ModeScenarios>();
        world.init_resource::<Toasts>();
        world.init_resource::<SessionStatus>();
        world.init_resource::<LoadRequests>();
        world.init_resource::<LatestView>();
        world.init_resource::<ViewRequestState>();
        world.init_resource::<LiveSync>();
        world.init_resource::<CameraFit>();
        world.init_resource::<ViewSettings>();
        bevy::ecs::system::RunSystemOnce::run_system_once(&mut world, fill_scenarios).unwrap();
        world
    }

    fn runner_of(world: &World) -> std::sync::Weak<Runner<ContinuousSim>> {
        Arc::downgrade(&world.resource::<ContinuousRunner>().0)
    }

    /// Enter, leave and enter again: every exit drops the last handle on the
    /// runner (its `Drop` stops the simulation thread).
    #[test]
    fn exiting_releases_the_runner_and_reentering_starts_a_new_one() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = lifecycle_world();
        world.run_system_once(enter_session).unwrap();
        let first = runner_of(&world);
        assert!(world.contains_resource::<ActiveSession>() && world.contains_resource::<LoadedWorld>());
        assert_eq!(world.resource::<LoadedWorld>().header.id, DEFAULT_SCENARIO);
        world.run_system_once(exit_session).unwrap();
        assert!(first.upgrade().is_none(), "the runner (and its thread) is gone after OnExit");
        assert!(!world.contains_resource::<ActiveSession>() && !world.contains_resource::<ContinuousRunner>());
        // The selected scenario is reopened on the next visit.
        world.resource_mut::<ModeScenarios>().set(
            MODE,
            ScenarioList { entries: Catalog::builtin().entries(), selected: Some("smoothlife.glider".into()) },
        );
        world.run_system_once(enter_session).unwrap();
        assert_eq!(world.resource::<LoadedWorld>().header.id, "smoothlife.glider");
        let second = runner_of(&world);
        world.run_system_once(exit_session).unwrap();
        assert!(second.upgrade().is_none());
    }

    /// Every scenario, of any model, is loaded into the same runner
    /// (`Control::Replace`), which then serves the new model's descriptor
    /// and speed; the session is inserted again, so the plots start afresh.
    #[test]
    fn scenario_loads_reuse_the_runner() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = lifecycle_world();
        world.run_system_once(enter_session).unwrap();
        let first = runner_of(&world);
        let session_changed = |world: &World| world.get_resource_change_ticks::<ActiveSession>().unwrap().changed;
        let inserted = session_changed(&world);
        world.resource_mut::<LoadRequests>().0.extend(["nope".to_string(), "smoothlife.glider".to_string()]);
        world.run_system_once(load_requested_scenarios).unwrap();
        assert!(Weak::ptr_eq(&first, &runner_of(&world)), "Control::Replace keeps the runner");
        assert_ne!(session_changed(&world), inserted, "the session is inserted again: the plots start afresh");
        assert_eq!(world.resource::<LoadedWorld>().kind(), ModelKind::SmoothLife);
        assert_eq!(
            world.resource::<ModeScenarios>().get(MODE).and_then(|l| l.selected.clone()).as_deref(),
            Some("smoothlife.glider")
        );
        assert!(world.resource::<LoadRequests>().0.is_empty());
        assert_eq!(
            world.resource::<ViewSettings>().colormap,
            crate::world::default_colormap(&world.resource::<LoadedWorld>().rule)
        );
        let runner = first.upgrade().unwrap();
        let glider =
            ContinuousSim::from_scenario(Catalog::builtin().get("smoothlife.glider").unwrap().clone()).unwrap();
        let expected = display_descriptor(&glider.descriptor(), &glider.params());
        let start = std::time::Instant::now();
        while runner.status().descriptor_seq == 0 || runner.status().speed != default_speed(ModelKind::SmoothLife) {
            assert!(start.elapsed().as_secs() < 10, "the new model's descriptor and speed were not published");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(*runner.descriptor(), expected);
    }

    #[test]
    fn default_speeds_per_model() {
        assert_eq!(default_speed(ModelKind::Lenia), Speed::TicksPerSecond(30.0));
        assert_eq!(default_speed(ModelKind::SmoothLife), Speed::TicksPerSecond(10.0));
    }
}
