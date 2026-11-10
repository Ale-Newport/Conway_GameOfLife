//! [`CivilizationSim`]: the `simulation_core::Simulation` implementation.

use organism_engine::AgentStore;
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{
    ApplyPolicy, EventLog, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamSpec, ParamValue, SimError, SimRng,
    Simulation, Snapshot,
};

use crate::agent::Agent;
use crate::buildings::Buildings;
use crate::command::{self, CivCommand};
use crate::config::{Region, Rules, Scenario};
use crate::error::CivError;
use crate::groups::Groups;
use crate::metrics;
use crate::state::{CivState, Scratch, TickStats};
use crate::step;
use crate::terrain;
use crate::view::{self, CivView, CivViewRequest};
use crate::world::World;

/// Model identifier.
pub const MODEL_ID: &str = "civilization";
/// Snapshot schema version. Version 2 added household food stores and the
/// seven-way work shares; version 1 snapshots cannot be migrated (their agent
/// layout differs) and are rejected.
pub const SCHEMA_VERSION: u32 = 3;
const STREAM_AGENTS: u64 = 0xC1_71;
/// Seeds are limited to `0..=i64::MAX` (the range of the `seed` parameter).
const SEED_MASK: u64 = i64::MAX as u64;
const EVENT_CAPACITY: usize = 2000;

/// Formal statement of the executed rules (shown in the rule panel).
pub const RULES: &str = include_str!("rules.md");

/// The Emergent Civilizations model.
#[derive(Debug)]
pub struct CivilizationSim {
    scenario: Scenario,
    rules: Rules,
    seed: u64,
    state: CivState,
    scratch: Scratch,
}

// The scenario travels as RON text: its rule overrides are untagged
// `ParamValue`s, which postcard cannot decode.
#[derive(Serialize)]
struct PayloadRef<'a> {
    scenario: String,
    rules: &'a Rules,
    seed: u64,
    state: &'a CivState,
}

#[derive(Deserialize)]
struct Payload {
    scenario: String,
    rules: Rules,
    seed: u64,
    state: CivState,
}

/// Random passable position inside `region` (200 tries, then a scan).
fn place(world: &World, region: Region, rng: &mut SimRng) -> Option<Vec2> {
    let (w, h) = (world.width() as f32, world.height() as f32);
    let x0 = (region.x0 * w).floor() as i64;
    let x1 = ((region.x1 * w).ceil() as i64).clamp(x0 + 1, w as i64);
    let y0 = (region.y0 * h).floor() as i64;
    let y1 = ((region.y1 * h).ceil() as i64).clamp(y0 + 1, h as i64);
    let jitter = |rng: &mut SimRng| Vec2::new(rng.range_f32(-0.3, 0.3), rng.range_f32(-0.3, 0.3));
    for _ in 0..200 {
        let t = world.index(rng.range_i64(x0, x1) as u32, rng.range_i64(y0, y1) as u32);
        if world.passable(t) {
            return Some(world.center(t) + jitter(rng));
        }
    }
    let t = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .map(|(x, y)| world.index(x as u32, y as u32))
        .find(|&t| world.passable(t))?;
    Some(world.center(t) + jitter(rng))
}

/// Builds the initial state of `scenario` with `rules` and `seed`.
pub(crate) fn build_state(scenario: &Scenario, rules: &Rules, seed: u64) -> Result<CivState, CivError> {
    let terrain = terrain::generate(&scenario.world.terrain, seed)?;
    let world = World::new(terrain, scenario.world.richness);
    let mut rng = SimRng::derive(seed, STREAM_AGENTS);
    let mut agents: AgentStore<Agent> = AgentStore::new();
    let mut bands: Vec<(f32, Vec<organism_engine::AgentId>)> = Vec::new();
    let shores = world.shore_tiles();
    for (index, population) in scenario.populations.iter().enumerate() {
        let region = population.region.unwrap_or(Region::WHOLE);
        let mut members = Vec::new();
        for _ in 0..population.count {
            let position = place(&world, region, &mut rng).ok_or_else(|| {
                CivError::InvalidScenario(format!(
                    "the region of population `{}` has no passable land",
                    population.label
                ))
            })?;
            let mut agent = command::make_adult(position, population, index as u8, rules, &mut rng);
            agent.memory.water = world.nearest_of(&shores, position);
            members.push(agents.queue_spawn(agent));
        }
        bands.push((population.kin_affinity, members));
    }
    agents.commit();
    // Founding bands: each founder knows its nearest fellow founders.
    let cap = rules.max_relationships as usize;
    for (affinity, members) in bands.iter().filter(|(a, _)| *a != 0.0) {
        for &id in members {
            let Some(me) = agents.get(id).map(|a| a.kin.position) else { continue };
            let mut others: Vec<(f32, organism_engine::AgentId)> = members
                .iter()
                .filter(|&&o| o != id)
                .filter_map(|&o| agents.get(o).map(|a| (a.kin.position.distance(me), o)))
                .collect();
            others.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some(agent) = agents.get_mut(id) {
                for (_, o) in others.into_iter().take(cap) {
                    agent.relations.adjust(o, *affinity, cap);
                }
            }
        }
    }
    Ok(CivState {
        tick: 0,
        rules: rules.clone(),
        world,
        agents,
        buildings: Buildings::default(),
        groups: Groups::default(),
        rng,
        stats: TickStats::default(),
        events: EventLog::new(EVENT_CAPACITY),
    })
}

fn seed_spec() -> ParamSpec {
    ParamSpec::int("seed", "Seed", 0, i64::MAX, 0)
        .describe("Seed of terrain, founders and behaviour; applied on reset.")
        .group("World")
        .apply(ApplyPolicy::RequiresReset)
}

/// Static model description.
pub fn descriptor() -> ModelDescriptor {
    let mut params = Rules::param_specs();
    params.push(seed_spec());
    ModelDescriptor::new(MODEL_ID, "Emergent Civilizations", ModelFamily::AgentBased)
        .summary(
            "People with needs, skills, inventories, preferences and relationships gather, farm, craft, build, \
             barter at marginal-rate-of-substitution prices, share, steal and fight on a regenerating tile world. \
             Roads emerge from footfall, groups from mutual affinity and territories from group buildings.",
        )
        .rules(RULES)
        .params(params)
        .metrics(metrics::metric_specs())
        .time("hour", 1.0)
}

impl CivilizationSim {
    /// Validates `scenario` and builds its initial state.
    pub fn new(scenario: Scenario) -> Result<Self, CivError> {
        scenario.validate()?;
        let rules = Rules::from_params(&scenario.params)?;
        let seed = scenario.seed;
        let state = build_state(&scenario, &rules, seed)?;
        let scratch = Scratch::new(&state.world);
        Ok(Self { scenario, rules, seed, state, scratch })
    }

    /// Scenario the model was built from.
    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    /// Active rule parameters.
    pub fn rules(&self) -> &Rules {
        &self.state.rules
    }

    /// Seed used by the next reset.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Tile world.
    pub fn world(&self) -> &World {
        &self.state.world
    }

    /// Living agents.
    pub fn agents(&self) -> &AgentStore<Agent> {
        &self.state.agents
    }

    /// Buildings and construction sites.
    pub fn buildings(&self) -> &Buildings {
        &self.state.buildings
    }

    /// Groups.
    pub fn groups(&self) -> &Groups {
        &self.state.groups
    }

    /// Notable events (births, deaths, groups, buildings, fights, disasters).
    pub fn events(&self) -> &EventLog {
        &self.state.events
    }

    /// Event counts of the last tick.
    pub fn last_tick_stats(&self) -> &TickStats {
        &self.state.stats
    }

    /// Sets the seed and resets (used by the generic "Randomize" control).
    /// The seed is reduced to its low 63 bits so that it stays representable
    /// by the `seed` parameter (`0..=i64::MAX`).
    pub fn reseed(&mut self, seed: u64) -> Result<(), CivError> {
        let seed = seed & SEED_MASK;
        let state = build_state(&self.scenario, &self.rules, seed)?;
        self.seed = seed;
        self.scratch = Scratch::new(&state.world);
        self.state = state;
        Ok(())
    }

    fn validate_restored(scenario: &Scenario, p: &Payload) -> Result<(), CivError> {
        let bad = |m: String| CivError::InvalidScenario(format!("corrupt snapshot: {m}"));
        scenario.validate().map_err(|e| bad(e.to_string()))?;
        if p.seed > SEED_MASK {
            return Err(bad(format!("seed {} is larger than {}", p.seed, SEED_MASK)));
        }
        p.state.buildings.validate().map_err(bad)?;
        p.state.groups.validate().map_err(bad)?;
        for rules in [&p.rules, &p.state.rules] {
            for spec in Rules::param_specs() {
                let value = rules.value(&spec.key).ok_or_else(|| bad(format!("missing rule {}", spec.key)))?;
                spec.validate(&value).map_err(|e| bad(e.to_string()))?;
            }
        }
        let n = p.state.world.len() as u32;
        let tile_ok = |t: u32| t < n;
        for (id, a) in p.state.agents.iter() {
            let target_ok = match a.goal.target {
                crate::agent::Target::Tile(t) => tile_ok(t),
                _ => true,
            };
            if !target_ok || !a.path.iter().all(|&t| tile_ok(t)) || !a.memory.tiles().all(tile_ok) {
                return Err(bad(format!("agent {id} refers to a tile outside the world")));
            }
            if !(a.kin.position.is_finite() && a.energy.is_finite() && a.health.is_finite()) {
                return Err(bad(format!("agent {id} has non-finite state")));
            }
        }
        if p.state.buildings.iter().any(|b| !tile_ok(b.tile)) {
            return Err(bad("a building lies outside the world".into()));
        }
        Ok(())
    }
}

impl Simulation for CivilizationSim {
    type Command = CivCommand;
    type ViewRequest = CivViewRequest;
    type View = CivView;

    fn descriptor(&self) -> ModelDescriptor {
        descriptor()
    }

    fn tick(&self) -> u64 {
        self.state.tick
    }

    fn step(&mut self) {
        step::step(&mut self.state, &mut self.scratch);
    }

    fn reset(&mut self) {
        // The scenario and seed were validated when they were set, so building
        // cannot fail; keep the current state if it ever does.
        if let Ok(state) = build_state(&self.scenario, &self.rules, self.seed) {
            self.scratch = Scratch::new(&state.world);
            self.state = state;
        }
    }

    fn apply(&mut self, command: CivCommand) -> Result<(), SimError> {
        command::apply(&mut self.state, &self.scenario.populations, &mut self.rules, command).map_err(SimError::from)
    }

    fn params(&self) -> ParamSet {
        self.state.rules.values().with("seed", ParamValue::Int(i64::try_from(self.seed).unwrap_or(i64::MAX)))
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        if key == "seed" {
            let spec = seed_spec();
            let seed = spec.validate(&value)?.as_i64().and_then(|v| u64::try_from(v).ok()).unwrap_or(self.seed);
            build_state(&self.scenario, &self.rules, seed)
                .map_err(|e| SimError::invalid_param("seed", e.to_string()))?;
            self.seed = seed;
            return Ok(spec.apply);
        }
        let policy = self.rules.set(key, &value)?;
        self.state.rules.set(key, &value)?;
        Ok(policy)
    }

    fn metrics(&self) -> Metrics {
        metrics::measure(&self.state, self.state.tick as f64)
    }

    fn view(&self, request: &CivViewRequest) -> CivView {
        view::build(&self.state, request)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        let scenario = ron::to_string(&self.scenario).map_err(|e| SimError::Serialization(e.to_string()))?;
        let payload = PayloadRef { scenario, rules: &self.rules, seed: self.seed, state: &self.state };
        Snapshot::encode(MODEL_ID, SCHEMA_VERSION, self.state.tick, &payload)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        if snapshot.model_id == MODEL_ID && snapshot.schema_version < SCHEMA_VERSION {
            return Err(SimError::UnsupportedVersion { found: snapshot.schema_version, supported: SCHEMA_VERSION });
        }
        let mut payload: Payload = snapshot.decode(MODEL_ID, SCHEMA_VERSION)?;
        let scenario: Scenario =
            ron::from_str(&payload.scenario).map_err(|e| SimError::Serialization(e.to_string()))?;
        payload.state.world.restore_derived().map_err(SimError::from)?;
        Self::validate_restored(&scenario, &payload)?;
        self.scratch = Scratch::new(&payload.state.world);
        self.scenario = scenario;
        self.rules = payload.rules;
        self.seed = payload.seed;
        self.state = payload.state;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        let agents: usize = self
            .state
            .agents
            .as_slice()
            .iter()
            .map(|a| {
                std::mem::size_of::<Agent>()
                    + a.relations.len() * std::mem::size_of::<crate::agent::Relationship>()
                    + a.path.capacity() * 4
            })
            .sum();
        let tiles = self.state.world.len() * 32;
        let buildings = self.state.buildings.len() * std::mem::size_of::<crate::buildings::Building>();
        agents + tiles + buildings
    }

    fn is_finished(&self) -> bool {
        self.state.agents.is_empty()
    }
}
