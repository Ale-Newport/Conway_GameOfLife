//! The running experiment: runner lifecycle, scenarios, and the data the
//! view reads from the simulation thread.
//!
//! The published [`EvolutionView`] carries positions, statistics and the
//! food field. Everything else is fetched with `Runner::request` closures
//! that run between ticks on the simulation thread and never change the
//! model: the genomes of all organisms (for the visible morphs), the
//! environment fields (temperature, capacity, regions), the selected
//! organism's full state and the genealogy.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use evolution::config::Terrain;
use evolution::genes::{GENE_COUNT, GENES, HERITABLE_COUNT};
use evolution::organism::{
    basal_cost, intake_capacity, movement_cost, perception_cost, senescence_hazard, speed_upkeep,
};
use evolution::organism::{storage, thermal_mismatch};
use evolution::{
    Activity, EvolutionCommand, EvolutionSim, EvolutionView, EvolutionViewRequest, FieldLayer, Scenario, Species,
};
use interface::{ActiveSession, ModeScenarios, ScenarioEntry, ScenarioList, SessionStatus, Toasts};
use organism_engine::{AgentId, Lineage};
use simulation_core::{Control, Runner, RunnerConfig, Simulation};

use crate::history::GeneHistory;
use crate::{MODE, ui_state::EvoUi};

/// Scenario opened when the mode starts.
pub const DEFAULT_SCENARIO: &str = "speed_selection";

/// Typed handle on the running experiment.
#[derive(Resource, Clone)]
pub struct EvoRunner(pub Arc<Runner<EvolutionSim>>);

/// The built-in experiment presets of the model crate.
#[derive(Resource, Clone, Debug, Default)]
pub struct EvoScenarios(pub Vec<Scenario>);

impl EvoScenarios {
    /// Loads `evolution::scenarios::builtin()`, logging a failure.
    pub fn load() -> Self {
        match evolution::scenarios::builtin() {
            Ok(list) => Self(list),
            Err(e) => {
                error!("evolution scenarios could not be loaded: {e}");
                Self(Vec::new())
            }
        }
    }

    /// Scenario by id.
    pub fn get(&self, id: &str) -> Option<&Scenario> {
        self.0.iter().find(|s| s.id == id)
    }
}

/// Identity of the loaded experiment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScenarioInfo {
    /// Scenario id (`custom` for a seed variant).
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the experiment shows.
    pub description: String,
    /// Seed of the run.
    pub seed: u64,
}

/// Normalised genome of one organism (all slots, heritable first).
pub type GenomeValues = [f32; GENE_COUNT];

/// Genomes of every living organism, keyed by id.
#[derive(Clone, Debug, Default)]
pub struct TraitsTable {
    /// Tick the table was read at.
    pub tick: u64,
    /// Seed of the run (a change means the world was rebuilt).
    pub seed: u64,
    /// `(id, species, normalised genome)` in id order.
    pub genomes: Vec<(AgentId, Species, GenomeValues)>,
}

/// Static and slowly changing environment fields.
#[derive(Clone, Debug, Default)]
pub struct EnvSnapshot {
    /// Tick the fields were read at.
    pub tick: u64,
    /// Grid width.
    pub width: usize,
    /// Grid height.
    pub height: usize,
    /// Terrain (row-major).
    pub terrain: Vec<Terrain>,
    /// Temperature at each cell centre (°C).
    pub temperature: Vec<f32>,
    /// Food carrying capacity per cell.
    pub capacity: Vec<f32>,
    /// Region label per cell (`u16::MAX` for impassable cells).
    pub regions: Vec<u16>,
    /// Number of regions.
    pub region_count: usize,
    /// Mean land temperature (°C).
    pub mean_temperature: f64,
    /// Directional climate shift (°C per 1000 ticks).
    pub shift_rate: f32,
    /// Seasonal amplitude (°C).
    pub seasonal_amplitude: f32,
}

impl EnvSnapshot {
    /// Largest carrying capacity of any cell (at least 1).
    pub fn max_capacity(&self) -> f32 {
        self.capacity.iter().copied().fold(1.0, f32::max)
    }

    /// Temperature range over land cells.
    pub fn land_temperature_range(&self) -> Option<(f32, f32)> {
        let mut range: Option<(f32, f32)> = None;
        for (t, terrain) in self.temperature.iter().zip(&self.terrain) {
            if terrain.is_passable() {
                range = Some(range.map_or((*t, *t), |(lo, hi)| (lo.min(*t), hi.max(*t))));
            }
        }
        range
    }
}

/// One recorded ancestor of an organism.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AncestorView {
    /// The ancestor.
    pub id: AgentId,
    /// Generation.
    pub generation: u32,
    /// Birth tick.
    pub birth_tick: u64,
    /// Death tick (`None` while alive).
    pub death_tick: Option<u64>,
}

/// Energy budget terms of one organism this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnergyBudget {
    /// Basal metabolism (incl. generalist cost and thermal stress).
    pub basal: f32,
    /// Cost of the speed used this tick.
    pub movement: f32,
    /// Upkeep of locomotor capacity.
    pub upkeep: f32,
    /// Perception cost.
    pub perception: f32,
    /// Maximum food intake per tick.
    pub intake_capacity: f32,
}

/// Full state of one organism, read on the simulation thread.
#[derive(Clone, Debug)]
pub struct OrganismDetail {
    /// Tick of the reading.
    pub tick: u64,
    /// The organism.
    pub id: AgentId,
    /// Species.
    pub species: Species,
    /// Normalised genome.
    pub normalised: GenomeValues,
    /// Expressed values of every slot (gene units).
    pub expressed: GenomeValues,
    /// Energy reserve.
    pub energy: f32,
    /// Storage capacity.
    pub storage: f32,
    /// Health.
    pub health: f32,
    /// Age (ticks).
    pub age: u32,
    /// Age at maturity (ticks, rounded phenotype).
    pub maturity: u32,
    /// Ticks until it may breed again.
    pub refractory: u32,
    /// Ticks still spent handling a kill (predators).
    pub handling: u32,
    /// Offspring produced.
    pub offspring: u32,
    /// Last behaviour.
    pub activity: Activity,
    /// Target of the last behaviour.
    pub target: Option<AgentId>,
    /// Parents, generation, birth tick.
    pub lineage: Lineage,
    /// Founder of the line of first parents.
    pub founder: AgentId,
    /// Living organisms descending from the same founder.
    pub lineage_living: u32,
    /// Position (world units).
    pub position: Vec2,
    /// Current speed (units per tick).
    pub speed: f32,
    /// Temperature at its position (°C).
    pub temperature: f32,
    /// Thermal mismatch `(T − T_opt − acclimation) / tolerance`.
    pub mismatch: f32,
    /// Per-tick senescence hazard.
    pub senescence: f64,
    /// Whether it could breed now.
    pub ready: bool,
    /// Energy budget.
    pub budget: EnergyBudget,
    /// Line of first parents that is still recorded (closest first).
    pub ancestors: Vec<AncestorView>,
}

/// One genealogy node for the tree view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenealogyNodeData {
    /// The organism.
    pub id: AgentId,
    /// First and second parent.
    pub parents: [Option<AgentId>; 2],
    /// Generation.
    pub generation: u32,
    /// Birth tick.
    pub birth_tick: u64,
    /// Death tick (`None` while alive).
    pub death_tick: Option<u64>,
}

/// Pruned genealogy of the living population.
#[derive(Clone, Debug, Default)]
pub struct GenealogyData {
    /// Tick of the reading.
    pub tick: u64,
    /// Recorded nodes in id order.
    pub nodes: Vec<GenealogyNodeData>,
    /// Species of living organisms.
    pub living: BTreeMap<AgentId, Species>,
    /// Births recorded since the start (including pruned).
    pub births: u64,
    /// Nodes pruned because they left no living descendants.
    pub pruned: u64,
    /// Oldest nodes dropped by the node cap.
    pub truncated: u64,
}

/// A reply awaited from the simulation thread (the receiver is behind a
/// mutex so the resource holding it is `Sync`).
struct Pending<T> {
    rx: Mutex<Receiver<T>>,
    epoch: u64,
}

impl<T> Pending<T> {
    fn new(rx: Receiver<T>, epoch: u64) -> Self {
        Self { rx: Mutex::new(rx), epoch }
    }

    /// `Ok(Some(value))` when the reply arrived, `Ok(None)` while waiting,
    /// `Err(())` when the runner was dropped.
    fn poll(&self) -> Result<Option<T>, ()> {
        let rx = self.rx.lock().map_err(|_| ())?;
        match rx.try_recv() {
            Ok(v) => Ok(Some(v)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(()),
        }
    }
}

/// Data shared by every view system.
#[derive(Resource, Default)]
pub struct EvoData {
    /// Latest published view.
    pub view: Option<Arc<EvolutionView>>,
    /// Publication counter of `view`.
    pub view_seq: u64,
    /// A new view arrived this frame.
    pub view_changed: bool,
    /// Bumped whenever the run jumps (new scenario, rewind): organism ids
    /// may then denote different organisms.
    pub epoch: u64,
    /// Genomes by id.
    pub traits: BTreeMap<AgentId, (Species, GenomeValues)>,
    /// Environment fields.
    pub env: Option<Arc<EnvSnapshot>>,
    /// Bumped when `env` is replaced.
    pub env_seq: u64,
    /// Selected organism's full state.
    pub detail: Option<OrganismDetail>,
    /// Genealogy (only fetched while the tree is shown).
    pub genealogy: Option<Arc<GenealogyData>>,
    /// Gene statistics over time.
    pub history: GeneHistory,
    /// Loaded experiment.
    pub scenario: ScenarioInfo,
    traits_pending: Option<Pending<TraitsTable>>,
    traits_at: f64,
    env_pending: Option<Pending<EnvSnapshot>>,
    env_at: f64,
    env_key: Option<EnvKey>,
    detail_pending: Option<Pending<Option<OrganismDetail>>>,
    detail_at: f64,
    genealogy_pending: Option<Pending<GenealogyData>>,
    genealogy_at: f64,
    last_tick: Option<u64>,
}

/// What the environment fields depend on; a change triggers a refresh.
#[derive(Clone, Debug, PartialEq)]
struct EnvKey {
    terrain_hash: u64,
    offset_bits: u64,
    tick_bucket: u64,
    params: String,
}

fn terrain_hash(terrain: &[Terrain]) -> u64 {
    terrain.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, t| (h ^ (*t as u64 + 1)).wrapping_mul(0x0100_0000_01b3))
}

impl EvoData {
    /// Forgets everything that belongs to the previous run.
    pub fn reset_for_new_run(&mut self) {
        self.epoch += 1;
        self.traits.clear();
        self.detail = None;
        self.genealogy = None;
        self.history.clear();
        self.env_key = None;
        // The fields of the previous run (possibly another scenario of the
        // same size) must not tint the new world.
        self.env = None;
        self.env_seq += 1;
        self.traits_pending = None;
        self.detail_pending = None;
        self.genealogy_pending = None;
        self.env_pending = None;
        self.last_tick = None;
        self.view = None;
    }

    /// Living organism count by species in the current view.
    pub fn counts(&self) -> (u32, u32) {
        self.view.as_ref().map_or((0, 0), |v| (v.stats.prey.count, v.stats.predators.count))
    }
}

/// Builds a runner for an experiment.
pub fn spawn_runner(sim: EvolutionSim) -> Arc<Runner<EvolutionSim>> {
    Arc::new(
        Runner::spawn(sim, RunnerConfig::default())
            // Clear: remove every organism (the world stays), e.g. before
            // seeding custom genomes with the Spawn tool.
            .with_clear(|sim: &mut EvolutionSim| {
                let cull = EvolutionCommand::Cull { fraction: 1.0, species: None, area: None };
                if let Err(e) = sim.apply(cull) {
                    warn!("clear failed: {e}");
                }
            })
            // Randomize: the same experiment with another seed (a replicate).
            .with_randomize(|sim: &mut EvolutionSim, seed| {
                if let Some(next) = reseeded(sim, seed) {
                    *sim = next;
                }
            }),
    )
}

/// The experiment of `sim` rebuilt from its configuration with `seed`.
pub fn reseeded(sim: &EvolutionSim, seed: u64) -> Option<EvolutionSim> {
    let scenario = Scenario {
        id: sim.scenario_id().to_string(),
        name: sim.scenario_name().to_string(),
        description: String::new(),
        seed,
        config: sim.pending_config().clone(),
    };
    match EvolutionSim::from_scenario(&scenario) {
        Ok(next) => Some(next),
        Err(e) => {
            warn!("could not rebuild the experiment with seed {seed}: {e}");
            None
        }
    }
}

/// View request used by this mode: the food field (vegetation); the other
/// fields come from [`EnvSnapshot`].
pub fn view_request() -> EvolutionViewRequest {
    EvolutionViewRequest { field: FieldLayer::Food, color_gene: None, genealogy: false }
}

/// Fills the scenario picker.
pub fn fill_scenarios(scenarios: Res<EvoScenarios>, mut picker: ResMut<ModeScenarios>) {
    let entries = scenarios
        .0
        .iter()
        .map(|s| ScenarioEntry { id: s.id.clone(), title: s.name.clone(), description: s.description.clone() })
        .collect();
    let selected =
        scenarios.get(DEFAULT_SCENARIO).map(|s| s.id.clone()).or_else(|| scenarios.0.first().map(|s| s.id.clone()));
    picker.set(MODE, ScenarioList { entries, selected });
}

/// Builds the experiment of a scenario.
pub fn build_scenario(scenario: &Scenario) -> Result<EvolutionSim, String> {
    EvolutionSim::from_scenario(scenario).map_err(|e| e.to_string())
}

fn info_of(scenario: &Scenario) -> ScenarioInfo {
    ScenarioInfo {
        id: scenario.id.clone(),
        name: scenario.name.clone(),
        description: scenario.description.clone(),
        seed: scenario.seed,
    }
}

/// `OnEnter`: starts the selected (or default) scenario.
pub fn enter(
    mut commands: Commands,
    scenarios: Res<EvoScenarios>,
    picker: Res<ModeScenarios>,
    mut data: ResMut<EvoData>,
    mut ui: ResMut<EvoUi>,
    mut toasts: ResMut<Toasts>,
) {
    let wanted = picker.get(MODE).and_then(|l| l.selected.clone()).unwrap_or_else(|| DEFAULT_SCENARIO.into());
    let scenario = scenarios.get(&wanted).or_else(|| scenarios.0.first());
    let Some(scenario) = scenario else {
        toasts.error("Evolution Laboratory: no scenario could be loaded");
        return;
    };
    let sim = match build_scenario(scenario) {
        Ok(sim) => sim,
        Err(e) => {
            toasts.error(format!("Scenario `{}` failed to build: {e}", scenario.id));
            return;
        }
    };
    let runner = spawn_runner(sim);
    runner.send(Control::SetViewRequest(view_request()));
    commands.insert_resource(ActiveSession::new(runner.clone()));
    commands.insert_resource(EvoRunner(runner));
    data.reset_for_new_run();
    data.scenario = info_of(scenario);
    ui.on_new_run(scenario.seed);
}

/// `OnExit`: stops the run (the runner thread ends with its last handle).
pub fn exit(mut commands: Commands, mut data: ResMut<EvoData>, mut ui: ResMut<EvoUi>) {
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<EvoRunner>();
    data.reset_for_new_run();
    ui.selected = None;
    ui.hovered = None;
}

/// Replaces the running experiment with `scenario` (optionally with another
/// seed), keeping the same session.
pub fn load_scenario(
    runner: &EvoRunner,
    scenario: &Scenario,
    seed: Option<u64>,
    data: &mut EvoData,
    ui: &mut EvoUi,
    toasts: &mut Toasts,
    picker: &mut ModeScenarios,
) -> bool {
    picker.mark_selected(MODE, &scenario.id);
    let mut scenario = scenario.clone();
    if let Some(seed) = seed {
        scenario.seed = seed;
    }
    match build_scenario(&scenario) {
        Ok(sim) => {
            runner.0.send(Control::Replace(Box::new(sim)));
            runner.0.send(Control::SetViewRequest(view_request()));
            data.reset_for_new_run();
            data.scenario = info_of(&scenario);
            ui.on_new_run(scenario.seed);
            true
        }
        Err(e) => {
            toasts.error(format!("Scenario `{}` failed to build: {e}", scenario.id));
            false
        }
    }
}

/// Handles the top-bar scenario picker.
#[allow(clippy::too_many_arguments)]
pub fn handle_scenario_selection(
    mut picked: MessageReader<interface::ScenarioSelected>,
    runner: Option<Res<EvoRunner>>,
    scenarios: Res<EvoScenarios>,
    mut data: ResMut<EvoData>,
    mut ui: ResMut<EvoUi>,
    mut toasts: ResMut<Toasts>,
    mut picker: ResMut<ModeScenarios>,
    mut scene: ResMut<crate::scene::SceneState>,
) {
    let Some(runner) = runner else {
        picked.clear();
        return;
    };
    for event in picked.read() {
        if event.mode != MODE {
            continue;
        }
        if let Some(scenario) = scenarios.get(&event.id)
            && load_scenario(&runner, scenario, None, &mut data, &mut ui, &mut toasts, &mut picker)
        {
            scene.frame_request = true;
        }
    }
}

/// Reads the latest view and polls / issues the data requests.
pub fn poll_data(
    runner: Option<Res<EvoRunner>>,
    status: Res<SessionStatus>,
    time: Res<Time<Real>>,
    mut ui: ResMut<EvoUi>,
    mut data: ResMut<EvoData>,
) {
    let Some(runner) = runner else { return };
    let now = time.elapsed_secs_f64();
    let data = &mut *data;
    data.view_changed = false;
    if let Some(published) = runner.0.latest_view()
        && published.seq != data.view_seq
    {
        data.view_seq = published.seq;
        let view = published.view;
        if data.last_tick.is_some_and(|t| view.tick < t) {
            // The run went back in time (reset, undo, history jump, load):
            // ids may denote other organisms now.
            data.epoch += 1;
            data.traits.clear();
            data.detail = None;
            data.history.truncate_after(view.tick);
        }
        data.last_tick = Some(view.tick);
        data.history.record(&view);
        data.view = Some(view);
        data.view_changed = true;
    }
    let Some(view) = data.view.clone() else { return };

    // Genomes: needed for every organism that has none yet, and refreshed
    // regularly (undo can give an id back to a different organism).
    if let Some(pending) = &data.traits_pending {
        match pending.poll() {
            Ok(Some(table)) => {
                if pending.epoch == data.epoch {
                    if table.seed != data.scenario.seed {
                        // Rebuilt with another seed (e.g. Randomize in the bottom bar):
                        // the same ids now denote other organisms.
                        data.scenario.seed = table.seed;
                        data.epoch += 1;
                        data.detail = None;
                        data.history.clear();
                        ui.on_new_run(table.seed);
                    }
                    data.traits = table.genomes.into_iter().map(|(id, s, g)| (id, (s, g))).collect();
                }
                data.traits_pending = None;
            }
            Ok(None) => {}
            Err(()) => data.traits_pending = None,
        }
    }
    let missing = view.organisms.iter().any(|o| !data.traits.contains_key(&o.id));
    if data.traits_pending.is_none() && ((missing && now - data.traits_at > 0.05) || now - data.traits_at > 2.0) {
        data.traits_at = now;
        data.traits_pending = Some(Pending::new(runner.0.query(read_traits), data.epoch));
    }

    // Environment fields.
    if let Some(pending) = &data.env_pending {
        match pending.poll() {
            Ok(Some(env)) => {
                if pending.epoch == data.epoch {
                    data.env = Some(Arc::new(env));
                    data.env_seq += 1;
                } else {
                    // Read before a rewind or rebuild: ask again.
                    data.env_key = None;
                }
                data.env_pending = None;
            }
            Ok(None) => {}
            Err(()) => data.env_pending = None,
        }
    }
    // What the fields depend on is checked at most four times per second
    // (hashing the terrain and formatting the parameters every frame would
    // be wasted work).
    if data.env_pending.is_none() && now - data.env_at > 0.25 {
        let params = status.get().map(|s| format!("{:?}", s.params)).unwrap_or_default();
        let key = EnvKey {
            terrain_hash: terrain_hash(&view.terrain),
            offset_bits: view.temperature_offset.to_bits(),
            // Temperature drifts with the climate trend and seasons: refresh at
            // most every 25 ticks.
            tick_bucket: view.tick / 25,
            params,
        };
        data.env_at = now;
        if data.env_key.as_ref() != Some(&key) {
            data.env_key = Some(key);
            data.env_pending = Some(Pending::new(runner.0.query(read_environment), data.epoch));
        }
    }

    // Selected organism.
    if let Some(pending) = &data.detail_pending {
        match pending.poll() {
            Ok(Some(detail)) => {
                if pending.epoch == data.epoch {
                    data.detail = detail;
                }
                data.detail_pending = None;
            }
            Ok(None) => {}
            Err(()) => data.detail_pending = None,
        }
    }
    match ui.selected {
        Some(id) => {
            let stale = data.detail.as_ref().is_none_or(|d| d.id != id || d.tick != view.tick);
            if data.detail_pending.is_none() && stale && now - data.detail_at > 0.15 {
                data.detail_at = now;
                data.detail_pending = Some(Pending::new(runner.0.query(move |sim| read_detail(sim, id)), data.epoch));
            }
        }
        None => data.detail = None,
    }

    // Genealogy (only while the tree is visible).
    if let Some(pending) = &data.genealogy_pending {
        match pending.poll() {
            Ok(Some(g)) => {
                if pending.epoch == data.epoch {
                    data.genealogy = Some(Arc::new(g));
                }
                data.genealogy_pending = None;
            }
            Ok(None) => {}
            Err(()) => data.genealogy_pending = None,
        }
    }
    if ui.wants_genealogy() {
        let stale = data.genealogy.as_ref().is_none_or(|g| g.tick != view.tick);
        let interval = if data.genealogy.is_none() { 0.0 } else { 0.5 };
        if data.genealogy_pending.is_none() && stale && now - data.genealogy_at > interval {
            data.genealogy_at = now;
            data.genealogy_pending = Some(Pending::new(runner.0.query(read_genealogy), data.epoch));
        }
    }
}

fn genome_array(values: &[f32]) -> GenomeValues {
    std::array::from_fn(|i| values.get(i).copied().unwrap_or(0.5))
}

/// Genomes of every organism (simulation thread).
pub fn read_traits(sim: &EvolutionSim) -> TraitsTable {
    let world = sim.world();
    TraitsTable {
        tick: world.tick(),
        seed: world.seed(),
        genomes: world.organisms().iter().map(|(id, o)| (id, o.species, genome_array(o.genome.values()))).collect(),
    }
}

/// Environment fields (simulation thread).
pub fn read_environment(sim: &EvolutionSim) -> EnvSnapshot {
    let world = sim.world();
    let env = world.environment();
    let climate = &world.config().climate;
    let (w, h) = (env.width(), env.height());
    let temperature = (0..w * h)
        .map(|i| {
            let centre = simulation_core::glam::Vec2::new((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            env.temperature_at(climate, centre, world.tick())
        })
        .collect();
    EnvSnapshot {
        tick: world.tick(),
        width: w,
        height: h,
        terrain: env.terrain().as_slice().to_vec(),
        temperature,
        capacity: env.capacity().as_slice().to_vec(),
        regions: env.regions().as_slice().to_vec(),
        region_count: env.region_count(),
        mean_temperature: env.mean_land_temperature(climate, world.tick()),
        shift_rate: climate.shift_rate,
        seasonal_amplitude: climate.seasonal_amplitude,
    }
}

/// Full state of organism `id` (simulation thread); `None` if it is gone.
pub fn read_detail(sim: &EvolutionSim, id: AgentId) -> Option<OrganismDetail> {
    let world = sim.world();
    let o = world.organisms().get(id)?;
    let schema = world.schema();
    let config = world.config();
    let phys = &config.physiology;
    let p = o.phenotype(schema);
    let normalised = genome_array(o.genome.values());
    let expressed: GenomeValues = std::array::from_fn(|i| o.genome.express(schema, i));
    let temperature = world.environment().temperature_at(&config.climate, o.kinematics.position, world.tick());
    let mismatch = thermal_mismatch(&p, temperature);
    let speed = o.kinematics.speed();
    let mut ancestors = Vec::new();
    let mut next = o.lineage.parents[0];
    while let Some(parent) = next {
        let Some(node) = world.genealogy().get(parent) else { break };
        ancestors.push(AncestorView {
            id: parent,
            generation: node.lineage.generation,
            birth_tick: node.lineage.birth_tick,
            death_tick: node.death_tick,
        });
        if ancestors.len() >= 64 {
            break;
        }
        next = node.lineage.parents[0];
    }
    let lineage_living = world.stats().lineages.iter().find(|l| l.founder == o.founder).map_or(0, |l| l.living);
    Some(OrganismDetail {
        tick: world.tick(),
        id,
        species: o.species,
        normalised,
        expressed,
        energy: o.energy,
        storage: storage(&p, phys),
        health: o.health,
        age: o.age,
        maturity: p.maturity,
        refractory: o.refractory,
        handling: o.handling,
        offspring: o.offspring,
        activity: o.activity,
        target: o.target,
        lineage: o.lineage,
        founder: o.founder,
        lineage_living,
        position: Vec2::new(o.kinematics.position.x, o.kinematics.position.y),
        speed,
        temperature,
        mismatch,
        senescence: senescence_hazard(o.age, &p, phys),
        ready: evolution::reproduction::is_ready(o, &p, config),
        budget: EnergyBudget {
            basal: basal_cost(&p, phys, mismatch),
            movement: movement_cost(&p, phys, speed),
            upkeep: speed_upkeep(&p, phys),
            perception: perception_cost(&p, phys),
            intake_capacity: intake_capacity(&p, phys) * config.food.energy,
        },
        ancestors,
    })
}

/// Pruned genealogy (simulation thread).
pub fn read_genealogy(sim: &EvolutionSim) -> GenealogyData {
    let world = sim.world();
    let g = world.genealogy();
    let summary = g.summary();
    GenealogyData {
        tick: world.tick(),
        nodes: g
            .nodes()
            .map(|n| GenealogyNodeData {
                id: n.lineage.id,
                parents: n.lineage.parents,
                generation: n.lineage.generation,
                birth_tick: n.lineage.birth_tick,
                death_tick: n.death_tick,
            })
            .collect(),
        living: world.organisms().iter().map(|(id, o)| (id, o.species)).collect(),
        births: summary.births,
        pruned: summary.pruned,
        truncated: summary.truncated,
    }
}

/// Number of heritable genes shown in gene pickers.
pub const HERITABLE: usize = HERITABLE_COUNT;

/// Phenotypic value of normalised `t` for gene `index`.
pub fn express(index: usize, t: f32) -> f32 {
    let g = &GENES[index.min(GENE_COUNT - 1)];
    g.min + (g.max - g.min) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> EvolutionSim {
        let mut scenario = evolution::scenarios::find_builtin("speed_selection").unwrap();
        scenario.config.world.width = 32;
        scenario.config.world.height = 24;
        scenario.config.prey.initial = 40;
        scenario.config.predators.initial = 4;
        EvolutionSim::from_scenario(&scenario).unwrap()
    }

    #[test]
    fn readers_report_the_world_without_changing_it() {
        let mut sim = small();
        sim.advance(5);
        let before = sim.snapshot().unwrap();
        let traits = read_traits(&sim);
        assert_eq!(traits.genomes.len(), sim.world().organisms().len());
        let env = read_environment(&sim);
        assert_eq!(env.temperature.len(), 32 * 24);
        assert_eq!(env.terrain.len(), env.capacity.len());
        assert!(env.max_capacity() >= 1.0);
        let id = traits.genomes[0].0;
        let detail = read_detail(&sim, id).expect("organism exists");
        assert_eq!(detail.id, id);
        assert!(detail.storage > 0.0 && detail.budget.basal > 0.0);
        assert!((0.0..=1.0).contains(&detail.health));
        assert!(read_detail(&sim, AgentId(u64::MAX)).is_none());
        let genealogy = read_genealogy(&sim);
        assert!(genealogy.nodes.len() >= genealogy.living.len());
        assert_eq!(sim.snapshot().unwrap(), before, "reading never changes the run");
    }

    /// The panels' reads (genomes, environment, the selected organism, the
    /// genealogy) change nothing: once a later speed change shows in the
    /// status, no metric sample was recorded and no view was published.
    #[test]
    fn panel_reads_are_queries() {
        use bevy::ecs::system::RunSystemOnce;
        use simulation_core::Speed;
        use std::time::{Duration, Instant};
        let sim = small();
        let id = sim.world().organisms().iter().next().expect("organisms").0;
        let scenario = ScenarioInfo { seed: sim.world().seed(), ..ScenarioInfo::default() };
        let runner = EvoRunner(spawn_runner(sim));
        let fence = |speed: Speed| {
            runner.0.send(Control::SetSpeed(speed));
            let deadline = Instant::now() + Duration::from_secs(10);
            while runner.0.status().speed != speed {
                assert!(Instant::now() < deadline, "the runner did not answer");
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let mut world = World::new();
        world.insert_resource(runner.clone());
        world.init_resource::<SessionStatus>();
        world.init_resource::<Time<Real>>();
        let tab = crate::ui_state::PropertiesTab::Genealogy;
        world.insert_resource(EvoUi { selected: Some(id), tab, ..EvoUi::default() });
        world.insert_resource(EvoData {
            traits_at: -10.0,
            env_at: -10.0,
            detail_at: -10.0,
            genealogy_at: -10.0,
            scenario,
            ..EvoData::default()
        });
        fence(Speed::TicksPerSecond(3.0));
        runner.0.drain_metrics();
        let seq = runner.0.latest_view().map(|v| v.seq);
        world.run_system_once(poll_data).unwrap();
        fence(Speed::TicksPerSecond(5.0));
        assert!(runner.0.drain_metrics().is_empty(), "a read recorded a metric sample");
        assert_eq!(runner.0.latest_view().map(|v| v.seq), seq, "a read published a view");
        world.run_system_once(poll_data).unwrap();
        let data = world.resource::<EvoData>();
        assert!(!data.traits.is_empty() && data.env.is_some() && data.genealogy.is_some());
        assert_eq!(data.detail.as_ref().map(|d| d.id), Some(id));
    }

    #[test]
    fn reseeding_keeps_the_experiment_and_changes_the_seed() {
        let sim = small();
        let next = reseeded(&sim, 999).unwrap();
        assert_eq!(next.world().seed(), 999);
        assert_eq!(next.scenario_id(), sim.scenario_id());
        assert_eq!(next.pending_config(), sim.pending_config());
    }

    #[test]
    fn expression_follows_gene_ranges() {
        let speed = &GENES[evolution::genes::SPEED];
        assert_eq!(express(evolution::genes::SPEED, 0.0), speed.min);
        assert_eq!(express(evolution::genes::SPEED, 1.0), speed.max);
        assert_eq!(terrain_hash(&[Terrain::Land]), terrain_hash(&[Terrain::Land]));
        assert_ne!(terrain_hash(&[Terrain::Land]), terrain_hash(&[Terrain::Water]));
    }
}
