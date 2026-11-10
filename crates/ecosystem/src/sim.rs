//! [`EcosystemSim`]: the ecosystem as a [`Simulation`] (model id `ecosystem`).

use organism_engine::AgentId;
use simulation_core::{
    ApplyPolicy, EventSeverity, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamValue, SimError, SimEvent,
    Simulation, Snapshot,
};

use crate::animal::OrganismRef;
use crate::command::EcosystemCommand;
use crate::config::{EcosystemConfig, Rules};
use crate::error::EcosystemError;
use crate::ledger::{Ledgers, Stocks};
use crate::metrics::metric_specs;
use crate::params::{RuntimeParams, current, param_specs, write};
use crate::scenario::Scenario;
use crate::species::SpeciesCatalog;
use crate::state::{Counters, EcoState, Runtime};
use crate::view::{EcosystemView, EcosystemViewRequest, OrganismInspection};

/// Model identifier.
pub const MODEL_ID: &str = "ecosystem";
/// Snapshot schema version (2: species life styles, density fields,
/// colonies, diseases, schedules and social state of part 2; 3: the
/// species' water economy `metabolism.food_water`).
pub const SCHEMA_VERSION: u32 = 3;

const ECOLOGICAL_RULES: &str = include_str!("rules_ecological.md");
const NETLOGO_RULES: &str = include_str!("rules_netlogo.md");

/// The ecosystem simulator.
pub struct EcosystemSim {
    state: EcoState,
    rt: Runtime,
    scenario_id: String,
}

impl EcosystemSim {
    /// Builds a run from a configuration, seed and species catalogue.
    /// Parameter overrides of the configuration are validated and applied.
    pub fn new(config: EcosystemConfig, seed: u64, catalog: SpeciesCatalog) -> Result<Self, EcosystemError> {
        let specs = param_specs(&config.rules);
        let overrides = config.params.resolve(&specs).map_err(|errors| {
            let list: Vec<String> = errors.iter().map(ToString::to_string).collect();
            EcosystemError::config("params", list.join("; "))
        })?;
        let mut config = config;
        // A `seed` override in the parameters wins over the argument.
        config.world.seed = seed;
        let mut params = RuntimeParams::default();
        for (key, value) in config.params.clone().iter() {
            let v = overrides.get(key).cloned().unwrap_or_else(|| value.clone());
            write(key, &v, &mut config, &mut params).map_err(|e| EcosystemError::config(key, e.to_string()))?;
        }
        let seed = config.world.seed;
        // The overrides now live in the parameters and the world
        // configuration; the state keeps no untagged values (snapshots use
        // a binary format).
        config.params = ParamSet::new();
        let (state, rt) = EcoState::build(config, seed, catalog, params)?;
        Ok(Self { state, rt, scenario_id: String::new() })
    }

    /// Builds the initial state of a scenario.
    pub fn from_scenario(scenario: &Scenario, catalog: SpeciesCatalog) -> Result<Self, EcosystemError> {
        let mut sim = Self::new(scenario.config.clone(), scenario.seed, catalog)?;
        sim.scenario_id = scenario.id.clone();
        Ok(sim)
    }

    /// Id of the scenario the run was built from (empty for ad-hoc configurations).
    pub fn scenario_id(&self) -> &str {
        &self.scenario_id
    }

    /// The species catalogue of the run.
    pub fn catalog(&self) -> &SpeciesCatalog {
        &self.state.catalog
    }

    /// The configuration (with `RequiresReset` changes that will apply at the next reset).
    pub fn config(&self) -> &EcosystemConfig {
        &self.state.config
    }

    /// Runtime parameters in effect.
    pub fn runtime_params(&self) -> &RuntimeParams {
        &self.state.params
    }

    /// The abiotic world.
    pub fn world(&self) -> &physics_world::World {
        &self.state.world
    }

    /// Living animals in id order.
    pub fn animals(&self) -> &organism_engine::AgentStore<crate::animal::Animal> {
        &self.state.animals
    }

    /// Trees in id order.
    pub fn trees(&self) -> &organism_engine::AgentStore<crate::trees::Tree> {
        &self.state.trees
    }

    /// Vegetation layers.
    pub fn vegetation(&self) -> &crate::vegetation::Vegetation {
        &self.state.vegetation
    }

    /// Litter and dead wood.
    pub fn detritus(&self) -> &crate::fungi::Detritus {
        &self.state.detritus
    }

    /// Fungi.
    pub fn fungi(&self) -> &crate::fungi::Fungi {
        &self.state.fungi
    }

    /// Carcasses and clutches.
    pub fn remains(&self) -> &crate::carcass::Remains {
        &self.state.remains
    }

    /// Energy and nutrient ledgers.
    pub fn ledgers(&self) -> &Ledgers {
        &self.state.ledgers
    }

    /// Current stocks of every pool.
    pub fn stocks(&self) -> Stocks {
        self.state.stocks()
    }

    /// Counters (births, deaths, causes...).
    pub fn counters(&self) -> &Counters {
        &self.state.counters
    }

    /// Infection, recovery and death counts per pathogen.
    pub fn disease(&self) -> &crate::disease::DiseaseCounters {
        &self.state.disease
    }

    /// Density fields of tiny organisms.
    pub fn fields(&self) -> &crate::fields::Fields {
        &self.state.fields
    }

    /// Colonies of eusocial insects.
    pub fn colonies(&self) -> &crate::colony::Colonies {
        &self.state.colonies
    }

    /// The global event log.
    pub fn events(&self) -> &simulation_core::EventLog {
        &self.state.events
    }

    /// Genealogy of the animals.
    pub fn genealogy(&self) -> &organism_engine::Genealogy {
        &self.state.genealogy
    }

    /// Derived traits of every species (index = species index).
    pub fn traits(&self) -> &[crate::species::AnimalTraits] {
        &self.rt.traits
    }

    /// Population of a species by id.
    pub fn population(&self, species: &str) -> usize {
        self.state
            .catalog
            .animal_index(species)
            .map_or(0, |i| self.state.animals.as_slice().iter().filter(|a| a.species == i).count())
    }

    /// Full state and history of an organism.
    pub fn inspect(&self, organism: OrganismRef) -> Result<OrganismInspection, EcosystemError> {
        self.state.inspect(&self.rt, organism)
    }

    /// Mutable access to an animal (tests and tools; prefer commands).
    pub fn animal_mut(&mut self, id: u64) -> Option<&mut crate::animal::Animal> {
        self.state.animals.get_mut(AgentId(id))
    }

    /// Applies a command and returns the typed error (see also [`Simulation::apply`]).
    pub fn command(&mut self, command: EcosystemCommand) -> Result<(), EcosystemError> {
        self.state.apply_command(&mut self.rt, command)
    }

    fn descriptor_rules(&self) -> &'static str {
        match self.state.config.rules {
            Rules::Ecological => ECOLOGICAL_RULES,
            Rules::NetLogo(_) => NETLOGO_RULES,
        }
    }

    /// Validates a decoded state before it replaces the current one.
    fn check(state: &mut EcoState) -> Result<(), String> {
        state.catalog.rebuild().map_err(|e| e.to_string())?;
        let world = &state.world;
        let cfg = world.config();
        cfg.validate().map_err(|e| e.to_string())?;
        if *world.calendar() != physics_world::Calendar::from_config(cfg) {
            return Err("world calendar does not match its configuration".into());
        }
        let (w, h) = (cfg.width, cfg.height);
        let dims_ok = [
            (world.elevation().width(), world.elevation().height()),
            (world.water_depth().width(), world.water_depth().height()),
            (world.temperature().width(), world.temperature().height()),
            (world.soil_moisture().width(), world.soil_moisture().height()),
            (world.light().width(), world.light().height()),
            (world.nutrients().width(), world.nutrients().height()),
            (world.organic_matter().width(), world.organic_matter().height()),
            (world.fuel().width(), world.fuel().height()),
            (world.canopy().width(), world.canopy().height()),
            (world.surface().width(), world.surface().height()),
            (world.biome().width(), world.biome().height()),
            (world.slope().width(), world.slope().height()),
            (world.fire().width(), world.fire().height()),
        ]
        .iter()
        .all(|&d| d == (w, h));
        if !dims_ok || world.burning_cells().iter().any(|&i| i as usize >= w * h) {
            return Err("world layers do not match the world size".into());
        }
        if !state.vegetation.is_consistent(w, h)
            || !state.detritus.is_consistent(w, h)
            || !state.fungi.is_consistent(w, h)
        {
            return Err("vegetation, litter or fungal grids do not match the world size".into());
        }
        let n = w * h;
        let env = &state.env;
        let env_ok = env.width == w
            && env.height == h
            && env.water_nearest.len() == n
            && env.water_distance.len() == n
            && env.cover.len() == n
            && env.day_temperature.len() == n
            && env.water_nearest.iter().all(|&c| c == crate::environment::NO_WATER || (c as usize) < n);
        if !env_ok {
            return Err("environment caches do not match the world size".into());
        }
        let canopy = &state.canopy;
        let canopy_ok = canopy.lai.len() == n
            && canopy.leaf_kg_m2.len() == n
            && canopy.crown_fuel.len() == n
            && canopy.browse_kg_m2.len() == n
            && canopy.count.len() == n
            && canopy.starts.len() == n + 1
            && canopy.starts.windows(2).all(|p| p[0] <= p[1])
            && canopy.starts.last().is_some_and(|&e| e as usize == canopy.items.len())
            && canopy.items.iter().all(|&i| (i as usize) < state.trees.len());
        if !canopy_ok && !state.canopy_dirty {
            return Err("canopy cache is inconsistent with the trees".into());
        }
        let species = state.catalog.animals().len();
        let schema_len = crate::animal::genetics::schema().len();
        if state.animals.as_slice().iter().any(|a| usize::from(a.species) >= species || a.genome.len() != schema_len) {
            return Err("an animal refers to an unknown species or has a malformed genome".into());
        }
        if state.trees.as_slice().iter().any(|t| usize::from(t.species) >= state.catalog.trees().len()) {
            return Err("a tree refers to an unknown species".into());
        }
        let remains_ok = state.remains.is_consistent()
            && state.remains.carcasses.iter().all(|c| usize::from(c.species) < species)
            && state
                .remains
                .clutches
                .iter()
                .all(|c| usize::from(c.species) < species && c.mother_genome.len() == schema_len);
        if !remains_ok {
            return Err("carcass or clutch data is inconsistent".into());
        }
        let c = &state.counters;
        if [c.births.len(), c.deaths.len(), c.total_births.len(), c.total_deaths.len()].iter().any(|&l| l != species) {
            return Err("counters do not match the species catalogue".into());
        }
        if !state.ledgers.energy.is_consistent() || !state.ledgers.nutrients.is_consistent() {
            return Err("ledgers have the wrong number of fluxes".into());
        }
        if !state.fields.is_consistent(&state.catalog, w, h) {
            return Err("density fields do not match the species catalogue or the world size".into());
        }
        if !state.colonies.is_consistent(&state.catalog, w, h) {
            return Err("colony data is inconsistent".into());
        }
        state.config.validate(&state.catalog).map_err(|e| e.to_string())?;
        let pathogens = state.config.pathogens.len();
        if !state.disease.is_consistent(pathogens)
            || state.schedule_done > state.config.events.len()
            || state.animals.as_slice().iter().any(|a| {
                a.infection.is_some_and(|i| usize::from(i.pathogen) >= pathogens)
                    || !(0.0..=1.0).contains(&a.injury)
                    || !a.altitude_m.is_finite()
            })
        {
            return Err("disease, schedule or animal condition data is inconsistent".into());
        }
        if let Some(nl) = &state.netlogo
            && (nl.green.len() != n
                || nl.countdown.len() != n
                || usize::from(nl.sheep) >= species
                || usize::from(nl.wolf) >= species)
        {
            return Err("NetLogo grass does not match the world size".into());
        }
        Ok(())
    }
}

impl Simulation for EcosystemSim {
    type Command = EcosystemCommand;
    type ViewRequest = EcosystemViewRequest;
    type View = EcosystemView;

    fn descriptor(&self) -> ModelDescriptor {
        let minutes = self.state.world.calendar().minutes_per_tick();
        ModelDescriptor::new(MODEL_ID, "Ecosystem", ModelFamily::Hybrid)
            .summary(
                "Animals with individual physiology and species-specific utility AI, vegetation layers, trees, fungi and \
                 a closed nutrient cycle on top of the physics world (terrain, climate, water, fire, disasters).",
            )
            .rules(self.descriptor_rules())
            .params(param_specs(&self.state.config.rules))
            .metrics(metric_specs(&self.state.catalog, &self.state.config))
            .time("minute", minutes)
    }

    fn tick(&self) -> u64 {
        self.state.tick
    }

    fn step(&mut self) {
        if let Err(error) = self.state.step(&mut self.rt) {
            // Internal invariant violations are reported, never panicked on.
            let tick = self.state.tick;
            self.state.events.push(SimEvent::new(tick, "error", error.to_string()).severity(EventSeverity::Major));
            self.state.tick += 1;
        }
    }

    fn reset(&mut self) {
        let (config, params) = (*self.state.reset_point).clone();
        let catalog = self.state.catalog.clone();
        // The configured seed (a `seed` change applies here).
        let seed = config.world.seed;
        match EcoState::build(config, seed, catalog, params) {
            Ok((state, rt)) => {
                self.state = state;
                self.rt = rt;
            }
            Err(error) => {
                let tick = self.state.tick;
                self.state.events.push(
                    SimEvent::new(tick, "error", format!("reset failed: {error}")).severity(EventSeverity::Major),
                );
            }
        }
    }

    fn apply(&mut self, command: EcosystemCommand) -> Result<(), SimError> {
        self.command(command).map_err(SimError::from)
    }

    fn params(&self) -> ParamSet {
        let mut config = self.state.config.clone();
        let mut params = self.state.params.clone();
        for (key, value) in &self.state.pending {
            // Queued values were validated when they were set.
            let _ = write(key, &ParamValue::from(value), &mut config, &mut params);
        }
        current(&config, &params)
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let policy = self.state.set_param(key, value.clone())?;
        // User changes also apply to the configuration `reset()` rebuilds from.
        let (config, params) = &mut *self.state.reset_point;
        write(key, &value, config, params)?;
        Ok(policy)
    }

    fn metrics(&self) -> Metrics {
        self.state.measure()
    }

    fn view(&self, request: &EcosystemViewRequest) -> EcosystemView {
        self.state.build_view(&self.rt, request)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(MODEL_ID, SCHEMA_VERSION, self.state.tick, &self.state)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        if snapshot.model_id == MODEL_ID && snapshot.schema_version < SCHEMA_VERSION {
            return Err(SimError::Serialization(format!(
                "ecosystem snapshots of schema version {} predate the current state layout (version {SCHEMA_VERSION}) \
                 and cannot be migrated; re-run the scenario",
                snapshot.schema_version
            )));
        }
        let mut state: EcoState = snapshot.decode(MODEL_ID, SCHEMA_VERSION)?;
        Self::check(&mut state).map_err(|e| SimError::Serialization(format!("inconsistent snapshot: {e}")))?;
        self.rt = Runtime::new(&state);
        self.state = state;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        let cells = self.state.world.width() * self.state.world.height();
        self.state.world.memory_bytes()
            + cells * (2 * 7 * 4 + 4 + 4 * 4 + 3 * 4 + 6 * 4 + 6 + 8)
            + self.state.animals.len() * std::mem::size_of::<crate::animal::Animal>()
            + self.state.trees.len() * std::mem::size_of::<crate::trees::Tree>()
            + self.state.remains.carcasses.len() * std::mem::size_of::<crate::carcass::Carcass>()
    }

    fn is_finished(&self) -> bool {
        match (&self.state.config.rules, &self.state.netlogo) {
            (Rules::NetLogo(cfg), Some(nl)) => {
                let wolves = self.state.animals.as_slice().iter().filter(|a| a.species == nl.wolf).count();
                let sheep = self.state.animals.as_slice().iter().filter(|a| a.species == nl.sheep).count();
                self.state.animals.is_empty() || (wolves == 0 && sheep > cfg.max_sheep as usize)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AnimalSetup, Terrain};

    fn small() -> EcosystemSim {
        let mut config = EcosystemConfig {
            terrain: Terrain::Flat { elevation_m: 40.0 },
            animals: vec![AnimalSetup { species: "rabbit".into(), count: 4, region: None, groups: None }],
            ..EcosystemConfig::default()
        };
        config.world.width = 16;
        config.world.height = 16;
        config.world.rainfall_mm = 400.0;
        EcosystemSim::new(config, 1, SpeciesCatalog::builtin().unwrap()).unwrap()
    }

    #[test]
    fn restore_rejects_inconsistent_states_without_side_effects() {
        let mut sim = small();
        sim.advance(5);
        let good = sim.snapshot().unwrap();
        let corrupt = |edit: &dyn Fn(&mut EcoState)| {
            let mut state = sim.state.clone();
            edit(&mut state);
            Snapshot::encode(MODEL_ID, SCHEMA_VERSION, state.tick, &state).unwrap()
        };
        let cases = [
            corrupt(&|s| s.animals.as_mut_slice()[0].species = 999),
            corrupt(&|s| s.vegetation = crate::vegetation::Vegetation::new(3, 3)),
            corrupt(&|s| s.env.water_nearest[0] = 1_000_000),
            corrupt(&|s| s.counters.births.push(1)),
            corrupt(&|s| {
                s.remains.carcasses.push(crate::carcass::Carcass {
                    id: 0,
                    species: 0,
                    position: Default::default(),
                    meat_kg: 1.0,
                    initial_kg: 1.0,
                    energy_kj: 1.0,
                    nutrients_g: 1.0,
                    created_tick: 0,
                    cause: crate::animal::DeathCause::Killed,
                    killer: None,
                })
            }),
        ];
        for (i, snap) in cases.iter().enumerate() {
            let err = sim.restore(snap).unwrap_err();
            assert!(matches!(err, SimError::Serialization(_)), "case {i}: {err}");
        }
        assert_eq!(sim.snapshot().unwrap(), good, "failed restores leave the model unchanged");
        sim.restore(&good).unwrap();
    }

    #[test]
    fn descriptor_rules_follow_the_rule_set() {
        let sim = small();
        let d = sim.descriptor();
        assert!(d.rules.contains("Ecosystem (ecological rules)"));
        let netlogo = crate::scenario::find_builtin("wolf_sheep_grass").unwrap().build().unwrap();
        assert!(netlogo.descriptor().rules.contains("NetLogo"));
        assert!(netlogo.descriptor().param("sheep_gain_from_food").is_some());
        assert!(d.param("sheep_gain_from_food").is_none());
    }
}
