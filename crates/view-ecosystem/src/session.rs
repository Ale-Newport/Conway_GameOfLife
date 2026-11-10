//! The model as this view runs it: [`EcosystemSim`] wrapped in
//! [`EcoSession`], which publishes a [`Frame`] (the model's own
//! [`EcosystemView`] plus data the model keeps but does not put in its view:
//! counters, ledgers, territories, recent events) and records observations
//! for the food web (who killed whom, energy eaten per feeding mode).
//!
//! Everything else is delegated unchanged: parameters, commands, snapshots
//! (same model id, so saves are ordinary `ecosystem` snapshots). The
//! descriptor gains a `population` metric (living individual animals, shown
//! by the shell's bottom bar), cumulative deaths per cause and the births
//! suppressed by population caps — all read from the model's counters.
//!
//! Every rebuild of the run (new scenario, reset, restore, undo, history
//! jump) gets a fresh *epoch*: a renderer that asks with another epoch
//! receives the static terrain again even when the new world reuses the same
//! terrain version number (every world starts at version 1).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use ecosystem::ledger::Ledger;
use ecosystem::metrics::{TrophicLevel, trophic_level};
use ecosystem::species::AnimalTraits;
use ecosystem::view::DEATH_CAUSES;
use ecosystem::{
    Action, DeathCause, EcosystemCommand, EcosystemError, EcosystemSim, EcosystemView, EcosystemViewRequest, Flux,
    FluxKind, Pathogen, Scenario, SpeciesCatalog, Stocks,
};
use ecosystem::{EventAction, FieldMarker, ScheduledEvent};
use simulation_core::glam::Vec2;
use simulation_core::{
    ApplyPolicy, MetricSpec, Metrics, ModelDescriptor, ParamSet, ParamValue, SimError, SimEvent, Simulation, Snapshot,
};

/// Recent notable events carried by every frame.
pub const RECENT_EVENTS: usize = 40;
/// Ticks for which a successful attack stays in [`Frame::recent_kills`].
pub const KILL_MEMORY_TICKS: u64 = 4;

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

fn next_epoch() -> u64 {
    NEXT_EPOCH.fetch_add(1, Ordering::Relaxed)
}

/// Simulated hours between two frames that carry the trees (trees grow
/// daily; fire, felling and planting change them at any tick and are shown
/// within the hour).
pub const TREE_REFRESH_HOURS: u32 = 1;
/// Shortest real time (s) between two frames that carry the trees: at high
/// speeds every frame would otherwise carry tens of thousands of trees.
pub const TREE_REFRESH_MIN_SECONDS: f64 = 0.5;

/// Representative markers of the density fields wanted around the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkerRequest {
    /// Cell region `(x0, y0, x1, y1)`, end exclusive.
    pub region: (usize, usize, usize, usize),
    /// Largest number of markers.
    pub limit: usize,
}

/// What the view asks the session to publish.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameRequest {
    /// The model's own view request (its whole-map field markers are not
    /// used: see `markers`).
    pub view: EcosystemViewRequest,
    /// Epoch the renderer holds static terrain for (`None`: send it).
    pub known_epoch: Option<u64>,
    /// Include territory centres of territorial adults.
    pub territories: bool,
    /// Density-field markers of a region (the model fills its marker budget
    /// row by row, so a whole-map request would only cover the first rows).
    pub markers: Option<MarkerRequest>,
}

impl Default for FrameRequest {
    fn default() -> Self {
        Self {
            view: EcosystemViewRequest { field_markers: false, ..EcosystemViewRequest::default() },
            known_epoch: None,
            territories: false,
            markers: None,
        }
    }
}

/// Scenario header for the description card.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenarioMeta {
    /// Scenario id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// The scenario file's description.
    pub description: String,
    /// Seed the scenario file gives (Randomize or the `seed` parameter
    /// may have changed the run's seed: see [`StaticInfo::seed`]).
    pub seed: u64,
    /// Scheduled interventions `(day, description)`.
    pub events: Vec<(f32, String)>,
}

/// Data that changes only when the run is rebuilt or its traits are
/// re-derived (e.g. after `motion_seconds` changed).
#[derive(Debug)]
pub struct StaticInfo {
    /// Scenario header.
    pub scenario: ScenarioMeta,
    /// Species catalogue of the run.
    pub catalog: SpeciesCatalog,
    /// Derived traits per animal species.
    pub traits: Vec<AnimalTraits>,
    /// Trophic level per animal species (the model's classification).
    pub trophic: Vec<TrophicLevel>,
    /// Pathogens of the scenario.
    pub pathogens: Vec<Pathogen>,
    /// NetLogo rule set: `(wolf, sheep)` species indices.
    pub netlogo: Option<(u16, u16)>,
    /// Cell size (m).
    pub cell_size_m: f32,
    /// World size in cells.
    pub size: (usize, usize),
    /// Seed of the running world.
    pub seed: u64,
    /// Simulated minutes per tick.
    pub minutes_per_tick: f64,
    /// Ticks per simulated day.
    pub ticks_per_day: u32,
}

/// One flux of a ledger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluxRow {
    /// Flux.
    pub flux: Flux,
    /// Role in this ledger.
    pub kind: FluxKind,
    /// Cumulative total since the start of accounting.
    pub total: f64,
    /// Total of the last tick.
    pub last_tick: f64,
}

/// One ledger as displayed: every used flux and the balance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LedgerView {
    /// Fluxes with a role in this ledger.
    pub rows: Vec<FluxRow>,
    /// Stock when accounting started.
    pub initial: f64,
    /// Σ inputs.
    pub inputs: f64,
    /// Σ outputs.
    pub outputs: f64,
    /// Current stock.
    pub stock: f64,
    /// `|stock − books| / stock` (the model's balance error).
    pub relative_error: f64,
}

impl LedgerView {
    fn of(ledger: &Ledger, energy: bool, stock: f64) -> Self {
        let rows = Flux::ALL
            .iter()
            .filter_map(|&flux| {
                let kind = if energy { flux.energy_kind() } else { flux.nutrient_kind() };
                (kind != FluxKind::Unused).then(|| FluxRow {
                    flux,
                    kind,
                    total: ledger.total(flux),
                    last_tick: ledger.last_tick(flux),
                })
            })
            .collect();
        Self {
            rows,
            initial: ledger.initial_stock,
            inputs: ledger.inputs(),
            outputs: ledger.outputs(),
            stock,
            relative_error: ledger.relative_error(stock),
        }
    }

    /// Cumulative total of a flux (0 when it has no role here).
    pub fn total(&self, flux: Flux) -> f64 {
        self.rows.iter().find(|r| r.flux == flux).map_or(0.0, |r| r.total)
    }
}

/// How an animal was feeding when it ate (its action during the meal).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FeedingMode {
    /// Grazing grass, herbs, moss or algae.
    Grazing,
    /// Browsing shrubs or tree foliage.
    Browsing,
    /// Foraging nectar, litter, mushrooms or gleaned insects.
    Foraging,
    /// Eating at a carcass (own kill or carrion).
    Carcass,
    /// Any other action.
    Other,
}

impl FeedingMode {
    /// Every mode in display order.
    pub const ALL: [FeedingMode; 5] = [Self::Grazing, Self::Browsing, Self::Foraging, Self::Carcass, Self::Other];

    /// The mode a meal eaten during `action` counts as.
    pub fn of(action: Action) -> Self {
        match action {
            Action::Graze => Self::Grazing,
            Action::Browse => Self::Browsing,
            Action::Forage => Self::Foraging,
            Action::Feed => Self::Carcass,
            _ => Self::Other,
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Grazing => "grazing",
            Self::Browsing => "browsing",
            Self::Foraging => "foraging",
            Self::Carcass => "at carcasses",
            Self::Other => "other actions",
        }
    }
}

/// Kills of one predator species on one prey species.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KillTally {
    /// Animals killed.
    pub count: u64,
    /// Body mass killed (kg).
    pub kg: f64,
}

/// Observations of the run since its epoch began (display data only; it
/// is not part of the model state and starts again after a time jump).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observed {
    /// Tick the observations started at.
    pub since_tick: u64,
    /// Ticks observed.
    pub ticks: u64,
    /// Kills `(predator species, prey species)`.
    pub kills: BTreeMap<(u16, u16), KillTally>,
    /// Gross energy eaten (kJ) per `(species, feeding mode)` by individual animals.
    pub eaten: BTreeMap<(u16, FeedingMode), f64>,
}

impl Observed {
    fn starting_at(tick: u64) -> Self {
        Self { since_tick: tick, ..Self::default() }
    }

    /// Energy (kJ) eaten by a species in a feeding mode.
    pub fn eaten_by(&self, species: u16, mode: FeedingMode) -> f64 {
        self.eaten.get(&(species, mode)).copied().unwrap_or(0.0)
    }

    /// Kills of `prey` by `predator`.
    pub fn kills_of(&self, predator: u16, prey: u16) -> KillTally {
        self.kills.get(&(predator, prey)).copied().unwrap_or_default()
    }
}

/// Territory of a territorial adult.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Territory {
    /// Animal id.
    pub id: u64,
    /// Species index.
    pub species: u16,
    /// Territory centre (cells).
    pub centre: Vec2,
    /// Radius (cells).
    pub radius: f32,
}

/// Counters of the model since the start of the run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CounterView {
    /// Deaths per cause, in [`DEATH_CAUSES`] order.
    pub deaths_by_cause: [u64; 10],
    /// Births per species.
    pub total_births: Vec<u64>,
    /// Deaths per species.
    pub total_deaths: Vec<u64>,
    /// Offspring not born because of a population cap.
    pub births_suppressed: u64,
    /// Trees germinated.
    pub trees_germinated: u64,
    /// Trees that died.
    pub trees_died: u64,
    /// Infections, recoveries and deaths per pathogen.
    pub disease: Vec<(u64, u64, u64)>,
}

/// A published frame.
#[derive(Debug)]
pub struct Frame {
    /// The model's render data.
    pub view: EcosystemView,
    /// World identity (see the module documentation).
    pub epoch: u64,
    /// Static data of the run.
    pub info: Arc<StaticInfo>,
    /// Counters.
    pub counters: CounterView,
    /// Energy ledger (kJ).
    pub energy: LedgerView,
    /// Nutrient ledger (g).
    pub nutrients: LedgerView,
    /// Stocks of every pool.
    pub stocks: Stocks,
    /// Food-web observations.
    pub observed: Arc<Observed>,
    /// Territories (when requested).
    pub territories: Vec<Territory>,
    /// Recent notable events, newest first.
    pub events: Vec<SimEvent>,
    /// Successful attacks of the last ticks `(predator, prey, tick)`.
    pub recent_kills: Vec<(u64, u64, u64)>,
    /// Whether `view.trees` holds the trees (else it is empty and the
    /// previous trees stand).
    pub trees_included: bool,
    /// Density-field markers of the requested region.
    pub markers: Vec<FieldMarker>,
}

/// [`EcosystemSim`] plus the view's observations.
pub struct EcoSession {
    sim: EcosystemSim,
    meta: ScenarioMeta,
    epoch: u64,
    info: Arc<StaticInfo>,
    observed: Arc<Observed>,
    recent_kills: Vec<(u64, u64, u64)>,
    /// `(id, gross energy eaten)` of every animal before the current step.
    scratch: Vec<(u64, f64)>,
    /// `(epoch, tick, tree count, real time)` of the last frame that
    /// carried trees.
    trees_sent: Cell<Option<(u64, u64, usize, Instant)>>,
    /// Fingerprint of the configuration `meta` describes.
    built_from: Fingerprint,
}

type Fingerprint = (usize, usize, u32, Vec<(String, u32)>);

/// Human-readable description of a scheduled event.
pub fn describe_event(event: &ScheduledEvent) -> String {
    match &event.action {
        EventAction::Ignite { x, y } => format!("ignition at cell ({x}, {y})"),
        EventAction::World(w) => {
            use ecosystem::schedule::WorldEvent as W;
            match w {
                W::Drought { days } => format!("{days:.0}-day drought"),
                W::Heatwave { days } => format!("{days:.0}-day heatwave"),
                W::Storm { days, intensity } => format!("{days:.0}-day storm (×{intensity:.1} rain)"),
                W::AddRain { amount_mm } => format!("{amount_mm:.0} mm of rain"),
                W::Flood { x, y, radius, depth_m } => {
                    format!("flood {depth_m:.1} m deep, radius {radius:.0} cells at ({x}, {y})")
                }
                W::Earthquake { x, y, magnitude, .. } => format!("magnitude {magnitude:.1} earthquake at ({x}, {y})"),
                W::Tornado { width, .. } => format!("tornado ({width:.0} cells wide)"),
                W::Landslide { x, y, .. } => format!("landslide at ({x}, {y})"),
                W::Volcano { x, y, height_m, .. } => format!("{height_m:.0} m volcano at ({x}, {y})"),
            }
        }
        EventAction::Introduce { species, count, .. } => format!("{count} {species} introduced"),
        EventAction::Remove { species, fraction } => format!("{:.0} % of {species} removed", fraction * 100.0),
        EventAction::Outbreak { pathogen, species, count } => format!("{pathogen} outbreak in {count} {species}"),
        EventAction::SetParam { key, value } => format!("parameter {key} set to {value}"),
    }
}

fn build_info(sim: &EcosystemSim, meta: &ScenarioMeta) -> StaticInfo {
    let catalog = sim.catalog().clone();
    let trophic = catalog.animals().iter().map(trophic_level).collect();
    let world = sim.world();
    StaticInfo {
        scenario: meta.clone(),
        traits: sim.traits().to_vec(),
        trophic,
        pathogens: sim.config().pathogens.clone(),
        netlogo: match &sim.config().rules {
            ecosystem::Rules::NetLogo(n) => {
                catalog.animal_index(&n.wolf_species).ok().zip(catalog.animal_index(&n.sheep_species).ok())
            }
            ecosystem::Rules::Ecological => None,
        },
        cell_size_m: world.config().cell_size_m,
        size: (world.width(), world.height()),
        seed: sim.config().world.seed,
        minutes_per_tick: world.calendar().minutes_per_tick(),
        ticks_per_day: world.calendar().ticks_per_day(),
        catalog,
    }
}

/// What identifies a scenario's world in a configuration (size, cell size
/// and founders), to recognise the scenario of a restored save.
fn fingerprint(config: &ecosystem::EcosystemConfig) -> (usize, usize, u32, Vec<(String, u32)>) {
    (
        config.world.width,
        config.world.height,
        config.world.cell_size_m.to_bits(),
        config.animals.iter().map(|a| (a.species.clone(), a.count)).collect(),
    )
}

/// Header of the built-in scenario a configuration was built from, if any.
fn builtin_meta_for(config: &ecosystem::EcosystemConfig) -> Option<ScenarioMeta> {
    let wanted = fingerprint(config);
    ecosystem::scenario::builtin().ok()?.into_iter().find(|s| fingerprint(&s.config) == wanted).map(|s| meta_of(&s))
}

fn meta_of(scenario: &Scenario) -> ScenarioMeta {
    ScenarioMeta {
        id: scenario.id.clone(),
        name: scenario.name.clone(),
        description: scenario.description.clone(),
        seed: scenario.seed,
        events: scenario.config.events.iter().map(|e| (e.day, describe_event(e))).collect(),
    }
}

/// Extra metric keys published by the session (beyond the model's own).
pub fn extra_metric_specs() -> Vec<MetricSpec> {
    let mut specs = vec![
        MetricSpec::gauge(
            "population",
            "Population",
            "",
            "Living individual animals (the same count as Animals; shown in the bottom bar).",
        ),
        MetricSpec::gauge(
            "births_suppressed",
            "Births suppressed",
            "",
            "Offspring not born because of a population cap, since the start (model counter).",
        ),
    ];
    for cause in DEATH_CAUSES {
        specs.push(MetricSpec::gauge(
            &format!("deaths_by.{}", cause.key()),
            &format!("Deaths: {}", cause_label(cause)),
            "",
            &format!("Animals dead of {} since the start (model counter).", cause_label(cause).to_lowercase()),
        ));
    }
    specs
}

/// Display label of a cause of death.
pub fn cause_label(cause: DeathCause) -> &'static str {
    match cause {
        DeathCause::Predation => "Predation",
        DeathCause::Starvation => "Starvation",
        DeathCause::Dehydration => "Dehydration",
        DeathCause::OldAge => "Old age",
        DeathCause::Fire => "Fire",
        DeathCause::Drowning => "Drowning",
        DeathCause::Exposure => "Exposure",
        DeathCause::Injury => "Injury",
        DeathCause::Killed => "Killed (command)",
        DeathCause::Disease => "Disease",
    }
}

impl EcoSession {
    /// Builds a scenario's initial state.
    pub fn new(scenario: &Scenario) -> Result<Self, EcosystemError> {
        let sim = scenario.build()?;
        Ok(Self::wrap(sim, meta_of(scenario), fingerprint(&scenario.config)))
    }

    fn wrap(sim: EcosystemSim, meta: ScenarioMeta, built_from: Fingerprint) -> Self {
        let info = Arc::new(build_info(&sim, &meta));
        let tick = sim.tick();
        Self {
            sim,
            meta,
            epoch: next_epoch(),
            info,
            observed: Arc::new(Observed::starting_at(tick)),
            recent_kills: Vec::new(),
            scratch: Vec::new(),
            trees_sent: Cell::new(None),
            built_from,
        }
    }

    /// The wrapped model.
    pub fn sim(&self) -> &EcosystemSim {
        &self.sim
    }

    /// Scenario header.
    pub fn scenario(&self) -> &ScenarioMeta {
        &self.meta
    }

    /// Current epoch.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Observations since the epoch began.
    pub fn observed(&self) -> &Observed {
        &self.observed
    }

    /// Applies a command and returns the typed error.
    pub fn command(&mut self, command: EcosystemCommand) -> Result<(), EcosystemError> {
        self.sim.command(command)
    }

    /// Starts a new epoch (the run jumped in time or was rebuilt).
    fn new_epoch(&mut self) {
        self.epoch = next_epoch();
        self.info = Arc::new(build_info(&self.sim, &self.meta));
        self.observed = Arc::new(Observed::starting_at(self.sim.tick()));
        self.recent_kills.clear();
    }

    fn counters(&self) -> CounterView {
        let c = self.sim.counters();
        let d = self.sim.disease();
        CounterView {
            deaths_by_cause: c.deaths_by_cause,
            total_births: c.total_births.clone(),
            total_deaths: c.total_deaths.clone(),
            births_suppressed: c.births_suppressed,
            trees_germinated: c.trees_germinated,
            trees_died: c.trees_died,
            disease: (0..d.total_infections.len())
                .map(|p| {
                    (
                        d.total_infections[p],
                        d.recoveries.get(p).copied().unwrap_or(0),
                        d.deaths.get(p).copied().unwrap_or(0),
                    )
                })
                .collect(),
        }
    }

    fn territories(&self) -> Vec<Territory> {
        let traits = self.sim.traits();
        self.sim
            .animals()
            .iter()
            .filter_map(|(id, a)| {
                let radius = traits.get(usize::from(a.species))?.territory_cells;
                let centre = a.home?;
                (radius > 0.0 && a.is_adult()).then_some(Territory { id: id.get(), species: a.species, centre, radius })
            })
            .collect()
    }

    /// Whether the next frame should carry the trees: the renderer has no
    /// trees of this epoch yet or the tick went back; or, at most every
    /// [`TREE_REFRESH_MIN_SECONDS`], an hour passed or the number of trees
    /// changed.
    fn trees_due(&self, renderer_knows_epoch: bool) -> bool {
        let tick = self.sim.tick();
        let hour = u64::from((self.info.ticks_per_day / 24).max(1) * TREE_REFRESH_HOURS);
        match self.trees_sent.get() {
            Some((epoch, sent, count, at)) if renderer_knows_epoch && epoch == self.epoch => {
                let changed = tick >= sent + hour || count != self.sim.trees().len();
                tick < sent || (changed && at.elapsed().as_secs_f64() >= TREE_REFRESH_MIN_SECONDS)
            }
            _ => true,
        }
    }

    /// Records the meals and kills of the step that just ran.
    fn observe_step(&mut self, tick: u64, events_before: u64) {
        let observed = Arc::make_mut(&mut self.observed);
        observed.ticks += 1;
        // Meals: the gross energy each animal ate during the step, by the
        // action it took (meals happen within the decided action).
        let before = &self.scratch;
        for (id, a) in self.sim.animals().iter() {
            let Ok(i) = before.binary_search_by_key(&id.get(), |(k, _)| *k) else { continue };
            let delta = a.stats.eaten_kj - before[i].1;
            if delta > 0.0 {
                *observed.eaten.entry((a.species, FeedingMode::of(a.action))).or_insert(0.0) += delta;
            }
        }
        // Kills: carcasses of this tick killed by a predator that is still alive.
        for c in self.sim.remains().carcasses.iter().rev() {
            if c.created_tick != tick {
                if c.created_tick < tick {
                    break;
                }
                continue;
            }
            if c.cause != DeathCause::Predation {
                continue;
            }
            let Some(killer) = c.killer.and_then(|k| self.sim.animals().get(k)) else { continue };
            let tally = observed.kills.entry((killer.species, c.species)).or_default();
            tally.count += 1;
            tally.kg += f64::from(c.initial_kg);
        }
        // The NetLogo rules remove eaten sheep without a carcass; the model
        // counts those kills (wolves are the only predator there).
        if let Some((wolf, sheep)) = self.info.netlogo {
            let n = u64::from(self.sim.counters().kills);
            if n > 0 {
                observed.kills.entry((wolf, sheep)).or_default().count += n;
            }
        }
        // Successful attacks of this tick (the model logs them as events).
        let events = self.sim.events();
        let new = events.total().saturating_sub(events_before).min(events.len() as u64) as usize;
        for e in events.recent(new) {
            if e.kind == "attack"
                && let (Some(p), Some(q)) = (e.subject, e.object)
            {
                self.recent_kills.push((p, q, e.tick));
            }
        }
        let horizon = tick.saturating_sub(KILL_MEMORY_TICKS);
        self.recent_kills.retain(|k| k.2 >= horizon);
    }
}

impl Simulation for EcoSession {
    type Command = EcosystemCommand;
    type ViewRequest = FrameRequest;
    type View = Frame;

    fn descriptor(&self) -> ModelDescriptor {
        let mut d = self.sim.descriptor();
        d.metrics.extend(extra_metric_specs());
        d
    }

    fn tick(&self) -> u64 {
        self.sim.tick()
    }

    fn step(&mut self) {
        let tick = self.sim.tick();
        self.scratch.clear();
        self.scratch.extend(self.sim.animals().iter().map(|(id, a)| (id.get(), a.stats.eaten_kj)));
        self.scratch.sort_unstable_by_key(|(id, _)| *id);
        let events_before = self.sim.events().total();
        self.sim.step();
        self.observe_step(tick, events_before);
        if self.info.traits.as_slice() != self.sim.traits() {
            self.info = Arc::new(build_info(&self.sim, &self.meta));
        }
    }

    fn reset(&mut self) {
        self.sim.reset();
        self.new_epoch();
    }

    fn apply(&mut self, command: EcosystemCommand) -> Result<(), SimError> {
        self.sim.apply(command)
    }

    fn params(&self) -> ParamSet {
        self.sim.params()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        self.sim.set_param(key, value)
    }

    fn metrics(&self) -> Metrics {
        let mut m = self.sim.metrics();
        let counters = self.sim.counters();
        m.push("population", self.sim.animals().len() as f64);
        m.push("births_suppressed", counters.births_suppressed as f64);
        for (cause, n) in DEATH_CAUSES.iter().zip(counters.deaths_by_cause) {
            m.push(&format!("deaths_by.{}", cause.key()), n as f64);
        }
        m
    }

    fn view(&self, request: &FrameRequest) -> Frame {
        let mut view_request = request.view.clone();
        if request.known_epoch != Some(self.epoch) {
            view_request.world.known_terrain_version = None;
        }
        let trees_included = view_request.trees && self.trees_due(request.known_epoch == Some(self.epoch));
        view_request.trees = trees_included;
        if trees_included {
            self.trees_sent.set(Some((self.epoch, self.sim.tick(), self.sim.trees().len(), Instant::now())));
        }
        let markers = request.markers.map_or_else(Vec::new, |m| {
            self.sim.fields().markers(self.sim.catalog(), self.sim.world(), m.region, self.sim.tick(), m.limit)
        });
        let stocks = self.sim.stocks();
        let ledgers = self.sim.ledgers();
        Frame {
            view: self.sim.view(&view_request),
            epoch: self.epoch,
            info: Arc::clone(&self.info),
            counters: self.counters(),
            energy: LedgerView::of(&ledgers.energy, true, stocks.energy()),
            nutrients: LedgerView::of(&ledgers.nutrients, false, stocks.nutrients()),
            stocks,
            observed: Arc::clone(&self.observed),
            territories: if request.territories { self.territories() } else { Vec::new() },
            events: self.sim.events().recent(RECENT_EVENTS).cloned().collect(),
            recent_kills: self.recent_kills.clone(),
            trees_included,
            markers,
        }
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        self.sim.snapshot()
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        self.sim.restore(snapshot)?;
        // A loaded save may come from another scenario (undo and history
        // jumps never do): describe the scenario the restored world is from.
        let restored = fingerprint(self.sim.config());
        if restored != self.built_from {
            self.meta = builtin_meta_for(self.sim.config()).unwrap_or_else(|| ScenarioMeta {
                id: String::new(),
                name: "Restored state".into(),
                description: "A saved state whose configuration matches no built-in scenario.".into(),
                seed: self.sim.config().world.seed,
                events: Vec::new(),
            });
            self.built_from = restored;
        }
        self.new_epoch();
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.sim.memory_bytes() + self.scratch.capacity() * 16
    }

    fn is_finished(&self) -> bool {
        self.sim.is_finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> EcoSession {
        EcoSession::new(&ecosystem::scenario::find_builtin(id).expect("builtin")).expect("valid")
    }

    #[test]
    fn wrapper_delegates_and_adds_metrics() {
        let s = session("balanced_meadow");
        let d = s.descriptor();
        assert_eq!(d.id, ecosystem::MODEL_ID, "saves stay ordinary ecosystem snapshots");
        for key in ["population", "births_suppressed", "deaths_by.predation", "deaths_by.disease"] {
            assert!(d.metrics.iter().any(|m| m.key == key), "{key} is described");
        }
        let m = s.metrics();
        assert_eq!(m.get("population"), m.get("animals"));
        assert_eq!(m.get("deaths_by.old_age"), Some(0.0));
        let keys: Vec<&str> = d.metrics.iter().map(|m| m.key.as_str()).collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "metric keys are unique");
    }

    #[test]
    fn epochs_change_on_time_jumps_and_force_terrain() {
        let mut s = session("wolf_sheep_grass");
        let first = s.epoch();
        let snap = s.snapshot().unwrap();
        let frame = s.view(&FrameRequest::default());
        assert_eq!(frame.epoch, first);
        let terrain = frame.view.world.terrain.as_ref().expect("terrain sent to an unknown epoch");
        let mut request = FrameRequest { known_epoch: Some(first), ..FrameRequest::default() };
        request.view.world.known_terrain_version = Some(terrain.version);
        assert!(s.view(&request).view.world.terrain.is_none(), "known terrain is not re-sent");
        for _ in 0..3 {
            s.step();
        }
        s.restore(&snap).unwrap();
        assert_ne!(s.epoch(), first);
        assert_eq!(s.tick(), 0);
        assert!(s.view(&request).view.world.terrain.is_some(), "a new epoch re-sends the terrain");
        let before = s.epoch();
        s.reset();
        assert_ne!(s.epoch(), before);
        assert_eq!(s.observed().ticks, 0);
    }

    #[test]
    fn observations_record_meals_and_kills() {
        let mut s = session("wolf_sheep_grass");
        for _ in 0..200 {
            s.step();
        }
        let obs = s.observed().clone();
        assert_eq!(obs.ticks, 200);
        let sheep = s.sim().catalog().animal_index("sheep").unwrap();
        let wolf = s.sim().catalog().animal_index("wolf").unwrap();
        let kills = obs.kills_of(wolf, sheep);
        // NetLogo wolves eat sheep on their patch; the model books a predation death per sheep eaten.
        let killed = s.sim().counters().deaths_by_cause[0];
        assert!(killed > 0, "wolves eat sheep within 200 ticks");
        assert_eq!(kills.count, killed, "every predation death is observed as a wolf kill");
        let frame = s.view(&FrameRequest::default());
        assert_eq!(frame.counters.deaths_by_cause[0], killed);
        assert!(frame.events.len() <= RECENT_EVENTS);
    }

    #[test]
    fn ecological_meals_are_attributed_to_feeding_modes() {
        let mut s = session("apex_predator_extinction");
        for _ in 0..240 {
            s.step();
        }
        let deer = s.sim().catalog().animal_index("deer").unwrap();
        let grazed =
            s.observed().eaten_by(deer, FeedingMode::Grazing) + s.observed().eaten_by(deer, FeedingMode::Browsing);
        assert!(grazed > 0.0, "deer graze or browse within a day");
        let frame = s.view(&FrameRequest { territories: true, ..FrameRequest::default() });
        let wolf = s.sim().catalog().animal_index("wolf").unwrap();
        assert!(frame.territories.iter().all(|t| t.radius > 0.0));
        assert!(
            frame
                .territories
                .iter()
                .all(|t| t.species == wolf || frame.info.traits[usize::from(t.species)].territory_cells > 0.0)
        );
        // The ledger view mirrors the model's balance.
        assert!(frame.energy.relative_error < 1e-3);
        assert!(frame.nutrients.rows.iter().all(|r| r.kind != FluxKind::Unused));
    }

    #[test]
    fn feeding_modes_follow_actions() {
        assert_eq!(FeedingMode::of(Action::Graze), FeedingMode::Grazing);
        assert_eq!(FeedingMode::of(Action::Browse), FeedingMode::Browsing);
        assert_eq!(FeedingMode::of(Action::Feed), FeedingMode::Carcass);
        assert_eq!(FeedingMode::of(Action::Flee), FeedingMode::Other);
    }

    #[test]
    fn restoring_a_save_of_another_scenario_renames_the_run() {
        let fire = session("forest_fire").snapshot().unwrap();
        let mut s = session("balanced_meadow");
        s.restore(&fire).unwrap();
        assert_eq!(s.scenario().id, "forest_fire");
        let frame = s.view(&FrameRequest::default());
        assert_eq!(frame.info.scenario.name, s.scenario().name);
    }

    #[test]
    fn scheduled_events_are_described() {
        let fire = ecosystem::scenario::find_builtin("forest_fire").unwrap();
        let s = EcoSession::new(&fire).unwrap();
        let events = &s.info.scenario.events;
        assert!(events.iter().any(|(_, d)| d.contains("drought")), "{events:?}");
        assert!(events.iter().any(|(_, d)| d.contains("ignition")), "{events:?}");
    }
}
