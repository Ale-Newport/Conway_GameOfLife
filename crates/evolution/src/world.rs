//! [`EvolutionWorld`]: the complete state of an experiment (environment,
//! organisms, genealogy, random generator, speciation tracker and counters),
//! its construction and its snapshot form. The tick itself lives in
//! [`crate::step`].

use std::f32::consts::TAU;

use organism_engine::{AgentId, AgentStore, Genealogy, GenomeSchema, Lineage};
use serde::{Deserialize, Serialize};
use simulation_core::{EventLog, EventSeverity, ParamValue, SimEvent, SimRng};

use crate::config::{EvolutionConfig, Shape};
use crate::environment::{Environment, EnvironmentState};
use crate::error::EvolutionError;
use crate::genes::{self, TraitValue, founder_genome, validate_traits};
use crate::organism::{Organism, Species, storage};
use crate::params;
use crate::speciation::SpeciationTracker;
use crate::stats::{self, PopulationStats, StatsInput};

/// Hard limit on living organisms; births beyond it are suppressed and
/// counted ([`TickReport::births_blocked`]).
pub const MAX_ORGANISMS: usize = 100_000;
/// Capacity of the event log.
const EVENT_CAPACITY: usize = 1000;
/// Random stream of the founding population.
const FOUNDER_STREAM: u64 = 1;
/// Random stream of the simulation.
const SIM_STREAM: u64 = 2;

/// Why an organism died.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeathCause {
    /// Energy reserve exhausted.
    Starvation,
    /// Killed by a predator.
    Predation,
    /// Health exhausted by thermal stress.
    Thermal,
    /// Gompertz senescence.
    Senescence,
    /// Age-independent background mortality.
    Background,
    /// Removed by a cull command.
    Culled,
}

impl DeathCause {
    /// Every cause, in counter order.
    pub const ALL: [DeathCause; 6] =
        [Self::Starvation, Self::Predation, Self::Thermal, Self::Senescence, Self::Background, Self::Culled];

    /// Counter slot.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Machine name (metric suffix).
    pub fn name(self) -> &'static str {
        match self {
            Self::Starvation => "starvation",
            Self::Predation => "predation",
            Self::Thermal => "thermal",
            Self::Senescence => "senescence",
            Self::Background => "background",
            Self::Culled => "culled",
        }
    }
}

/// Events of the last tick (and of commands applied since).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TickReport {
    /// Prey born.
    pub prey_births: u32,
    /// Predators born.
    pub predator_births: u32,
    /// Births suppressed by [`MAX_ORGANISMS`].
    pub births_blocked: u32,
    /// Deaths per [`DeathCause`].
    pub deaths: [u32; 6],
    /// Successful matings (sexual mode).
    pub matings: u32,
    /// Attacks attempted (contact strikes).
    pub attacks: u32,
    /// Food eaten.
    pub food_eaten: f64,
    /// Mutated genes in newborns.
    pub mutations: u32,
}

impl TickReport {
    /// All deaths.
    pub fn total_deaths(&self) -> u32 {
        self.deaths.iter().sum()
    }

    /// All births.
    pub fn births(&self) -> u32 {
        self.prey_births + self.predator_births
    }

    fn add(&mut self, other: &TickReport) {
        self.prey_births += other.prey_births;
        self.predator_births += other.predator_births;
        self.births_blocked += other.births_blocked;
        for (a, b) in self.deaths.iter_mut().zip(other.deaths) {
            *a += b;
        }
        self.matings += other.matings;
        self.attacks += other.attacks;
        self.food_eaten += other.food_eaten;
        self.mutations += other.mutations;
    }
}

/// Complete state of an evolution experiment.
#[derive(Clone, Debug)]
pub struct EvolutionWorld {
    pub(crate) config: EvolutionConfig,
    pub(crate) seed: u64,
    pub(crate) tick: u64,
    pub(crate) rng: SimRng,
    pub(crate) env: Environment,
    pub(crate) store: AgentStore<Organism>,
    pub(crate) genealogy: Genealogy,
    pub(crate) schema: GenomeSchema,
    pub(crate) speciation: SpeciationTracker,
    pub(crate) last: TickReport,
    pub(crate) pending: TickReport,
    pub(crate) totals: TickReport,
    pub(crate) events: EventLog,
    pub(crate) commands_applied: u64,
    pub(crate) stats: PopulationStats,
}

/// Snapshot payload (owned form used for decoding).
#[derive(Deserialize)]
pub(crate) struct WorldState {
    config: EvolutionConfig,
    seed: u64,
    tick: u64,
    rng: SimRng,
    env: EnvironmentState,
    store: AgentStore<Organism>,
    genealogy: Genealogy,
    speciation: SpeciationTracker,
    last: TickReport,
    pending: TickReport,
    totals: TickReport,
    events: EventLog,
    commands_applied: u64,
}

/// Snapshot payload (borrowed form used for encoding; same field order).
#[derive(Serialize)]
pub(crate) struct WorldStateRef<'a> {
    config: &'a EvolutionConfig,
    seed: u64,
    tick: u64,
    rng: &'a SimRng,
    env: EnvironmentState,
    store: &'a AgentStore<Organism>,
    genealogy: &'a Genealogy,
    speciation: &'a SpeciationTracker,
    last: &'a TickReport,
    pending: &'a TickReport,
    totals: &'a TickReport,
    events: &'a EventLog,
    commands_applied: u64,
}

impl EvolutionWorld {
    /// Builds the initial state of an experiment.
    pub fn new(config: EvolutionConfig, seed: u64) -> Result<Self, EvolutionError> {
        config.validate()?;
        Self::check_founders(&config)?;
        let schema = genes::schema(config.mutation.size)?;
        let env = Environment::new(&config);
        let mut world = Self {
            seed,
            tick: 0,
            rng: SimRng::derive(seed, SIM_STREAM),
            env,
            store: AgentStore::new(),
            genealogy: Genealogy::with_max_nodes(config.statistics.genealogy_max_nodes as usize),
            schema,
            speciation: SpeciationTracker::default(),
            last: TickReport::default(),
            pending: TickReport::default(),
            totals: TickReport::default(),
            events: EventLog::new(EVENT_CAPACITY),
            commands_applied: 0,
            stats: PopulationStats::default(),
            config,
        };
        let mut founders = SimRng::derive(seed, FOUNDER_STREAM);
        let mut prey_groups = vec![world.config.prey.clone()];
        prey_groups.extend(world.config.extra_prey.iter().cloned());
        for group in &prey_groups {
            world.spawn_with(
                Species::Prey,
                group.initial,
                group.area.as_ref(),
                &group.traits,
                group.spread,
                &mut founders,
            )?;
        }
        let predators = world.config.predators.clone();
        world.spawn_with(
            Species::Predator,
            predators.initial,
            predators.area.as_ref(),
            &predators.traits,
            predators.spread,
            &mut founders,
        )?;
        world.pending = TickReport::default();
        world.refresh_speciation();
        world.refresh_stats();
        Ok(world)
    }

    /// Checks that every founding population has passable land to start on.
    pub fn check_founders(config: &EvolutionConfig) -> Result<(), EvolutionError> {
        let env = Environment::new(config);
        let mut populations = vec![
            ("prey".to_string(), config.prey.initial, &config.prey.area),
            ("predators".to_string(), config.predators.initial, &config.predators.area),
        ];
        populations.extend(
            config.extra_prey.iter().enumerate().map(|(i, p)| (format!("extra_prey[{i}]"), p.initial, &p.area)),
        );
        for (name, initial, area) in populations {
            if initial > 0 && env.passable_cells(area.as_ref()).is_empty() {
                return Err(EvolutionError::config(
                    format!("{name}.area"),
                    "contains no passable land for the founders",
                ));
            }
        }
        Ok(())
    }

    /// Applies an already validated parameter change to the running world
    /// (parameters that do not require a reset).
    pub(crate) fn reconfigure(&mut self, key: &str, value: &ParamValue) -> Result<(), EvolutionError> {
        let mut config = self.config.clone();
        params::set(&mut config, key, value)?;
        if key == "mutation_size" {
            self.schema = genes::schema(config.mutation.size)?;
        }
        self.config = config;
        match key {
            "food_capacity_scale" => self.env.refresh_capacity(&self.config.food),
            "histogram_bins" | "allele_classes" => self.refresh_stats(),
            "speciation_enabled" | "reproduction_mode" | "mate_distance_limit" => {
                self.speciation.refresh_applicability(&self.config);
            }
            _ => {}
        }
        Ok(())
    }

    /// The configuration in use.
    pub fn config(&self) -> &EvolutionConfig {
        &self.config
    }

    /// The run seed.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Ticks since the start.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// The environment.
    pub fn environment(&self) -> &Environment {
        &self.env
    }

    /// Living organisms in id order.
    pub fn organisms(&self) -> &AgentStore<Organism> {
        &self.store
    }

    /// The pruned genealogy.
    pub fn genealogy(&self) -> &Genealogy {
        &self.genealogy
    }

    /// The genome schema.
    pub fn schema(&self) -> &GenomeSchema {
        &self.schema
    }

    /// Speciation criterion state.
    pub fn speciation(&self) -> &SpeciationTracker {
        &self.speciation
    }

    /// Births and deaths of the last tick (including commands applied before it).
    pub fn last_report(&self) -> &TickReport {
        &self.last
    }

    /// Cumulative counters since the start.
    pub fn totals(&self) -> &TickReport {
        &self.totals
    }

    /// Notable events (speciation, extinctions, interventions).
    pub fn events(&self) -> &EventLog {
        &self.events
    }

    /// Statistics measured after the last tick or command.
    pub fn stats(&self) -> &PopulationStats {
        &self.stats
    }

    /// Living organisms of a species.
    pub fn count(&self, species: Species) -> usize {
        self.store.as_slice().iter().filter(|o| o.species == species).count()
    }

    /// Recomputes the statistics (after a tick, a command or a parameter change).
    pub(crate) fn refresh_stats(&mut self) {
        self.stats = stats::compute(&StatsInput {
            store: &self.store,
            env: &self.env,
            schema: &self.schema,
            config: &self.config.statistics,
            storage: self.config.physiology.storage,
            tick: self.tick,
            seed: self.seed,
        });
    }

    /// Evaluates the speciation criterion now, logging a declared event.
    pub(crate) fn refresh_speciation(&mut self) {
        let prey: Vec<_> =
            self.store.as_slice().iter().filter(|o| o.species == Species::Prey).map(|o| &o.genome).collect();
        let mean_generation = if prey.is_empty() {
            0.0
        } else {
            self.store
                .as_slice()
                .iter()
                .filter(|o| o.species == Species::Prey)
                .map(|o| f64::from(o.lineage.generation))
                .sum::<f64>()
                / prey.len() as f64
        };
        let event = self.speciation.update(&prey, mean_generation, &self.config, &self.schema, self.tick, self.seed);
        if let Some(event) = event {
            let message = format!(
                "speciation criterion met: clusters of {} and {} prey, mean cross-cluster distance {:.3} > {:.3}",
                event.split.sizes[0],
                event.split.sizes[1],
                event.split.between,
                self.config.reproduction.mate_distance_limit
            );
            self.events.push(SimEvent::new(self.tick, "speciation", message).severity(EventSeverity::Major));
        }
    }

    /// Validates trait overrides, then spawns `count` founders of `species`
    /// (new lineages) uniformly on land inside `area`, drawing from `rng`.
    pub(crate) fn spawn_with(
        &mut self,
        species: Species,
        count: u32,
        area: Option<&Shape>,
        traits: &[TraitValue],
        spread: f32,
        rng: &mut SimRng,
    ) -> Result<Vec<AgentId>, EvolutionError> {
        validate_traits(traits, "traits")?;
        if !(0.0..=0.5).contains(&spread) {
            return Err(EvolutionError::InvalidCommand(format!("gene spread {spread} is outside [0, 0.5]")));
        }
        if count == 0 {
            return Ok(Vec::new());
        }
        if self.store.len() + count as usize > MAX_ORGANISMS {
            return Err(EvolutionError::InvalidCommand(format!(
                "{count} more organisms would exceed the limit of {MAX_ORGANISMS} living organisms"
            )));
        }
        let cells = self.env.passable_cells(area);
        if cells.is_empty() {
            return Err(EvolutionError::InvalidCommand("there is no passable land in the requested area".into()));
        }
        let mut ids = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let genome = founder_genome(&self.schema, traits, spread, rng)?;
            let p = genes::Phenotype::express(&genome, &self.schema);
            let position = self.env.random_point_in_cell(cells[rng.index(cells.len())], rng);
            let heading = rng.f32() * TAU;
            let age = rng.below(u64::from(p.maturity)) as u32;
            let energy = 0.5 * storage(&p, &self.config.physiology);
            let id = self.store.next_id();
            let lineage = Lineage::founder(id, self.tick);
            let organism = Organism::new(species, position, heading, genome, energy, age, lineage, id);
            let spawned = self.store.queue_spawn(organism);
            debug_assert_eq!(spawned, id);
            self.genealogy.record_birth(lineage)?;
            match species {
                Species::Prey => self.pending.prey_births += 1,
                Species::Predator => self.pending.predator_births += 1,
            }
            ids.push(id);
        }
        self.store.commit();
        Ok(ids)
    }

    /// Queues the removal of the given organisms and records their deaths
    /// (the caller commits the store).
    pub(crate) fn bury(&mut self, deaths: &[(AgentId, DeathCause)], report: &mut TickReport) {
        for &(id, cause) in deaths {
            self.store.queue_despawn(id);
            if let Err(e) = self.genealogy.record_death(id, self.tick) {
                self.events.push(SimEvent::new(self.tick, "genealogy", e.to_string()).severity(EventSeverity::Major));
            }
            report.deaths[cause.index()] += 1;
        }
    }

    /// Folds a finished tick's report into the totals.
    pub(crate) fn close_report(&mut self, mut report: TickReport) {
        report.add(&std::mem::take(&mut self.pending));
        self.totals.add(&report);
        self.last = report;
    }

    /// Encodes the full state (including the random generator).
    pub(crate) fn state(&self) -> WorldStateRef<'_> {
        WorldStateRef {
            config: &self.config,
            seed: self.seed,
            tick: self.tick,
            rng: &self.rng,
            env: self.env.state(),
            store: &self.store,
            genealogy: &self.genealogy,
            speciation: &self.speciation,
            last: &self.last,
            pending: &self.pending,
            totals: &self.totals,
            events: &self.events,
            commands_applied: self.commands_applied,
        }
    }

    /// Rebuilds a world from a decoded snapshot, validating it.
    pub(crate) fn from_state(state: WorldState) -> Result<Self, EvolutionError> {
        state.config.validate()?;
        let schema = genes::schema(state.config.mutation.size)?;
        let env = Environment::from_state(&state.config, state.env)?;
        let corrupt = |reason: String| EvolutionError::config("snapshot", reason);
        if state.rng.state() == [0; 4] {
            // xoshiro256++ stays at zero forever from this state.
            return Err(corrupt("the random generator state is all zero".into()));
        }
        if state.store.len() > MAX_ORGANISMS {
            return Err(corrupt(format!("{} organisms exceed the limit of {MAX_ORGANISMS}", state.store.len())));
        }
        for (id, o) in state.store.iter() {
            o.genome.check(&schema)?;
            if !(o.energy.is_finite() && o.health.is_finite() && o.kinematics.position.is_finite()) {
                return Err(corrupt(format!("organism {id} has non-finite state")));
            }
            if o.lineage.id != id || !state.genealogy.get(id).is_some_and(|n| n.is_alive()) {
                return Err(corrupt(format!("organism {id} is not recorded alive in the genealogy")));
            }
        }
        if state.genealogy.summary().living != state.store.len() as u64 {
            return Err(corrupt("the genealogy and the organism store disagree on the living population".into()));
        }
        let mut world = Self {
            config: state.config,
            seed: state.seed,
            tick: state.tick,
            rng: state.rng,
            env,
            store: state.store,
            genealogy: state.genealogy,
            schema,
            speciation: state.speciation,
            last: state.last,
            pending: state.pending,
            totals: state.totals,
            events: state.events,
            commands_applied: state.commands_applied,
            stats: PopulationStats::default(),
        };
        world.refresh_stats();
        Ok(world)
    }
}

#[cfg(test)]
mod tests {
    use simulation_core::Snapshot;

    use super::*;
    use crate::sim::{MODEL_ID, SCHEMA_VERSION};

    #[test]
    fn restore_validates_the_saved_state() {
        let mut config = EvolutionConfig::default();
        config.world.width = 32;
        config.world.height = 24;
        config.prey.initial = 30;
        let world = EvolutionWorld::new(config, 1).unwrap();
        let snap = Snapshot::encode(MODEL_ID, SCHEMA_VERSION, 0, &world.state()).unwrap();
        let state: WorldState = snap.decode(MODEL_ID, SCHEMA_VERSION).unwrap();
        assert!(EvolutionWorld::from_state(state).is_ok());

        let mut dead_rng: WorldState = snap.decode(MODEL_ID, SCHEMA_VERSION).unwrap();
        dead_rng.rng = ron::from_str("(s: (0, 0, 0, 0))").unwrap();
        let e = EvolutionWorld::from_state(dead_rng).unwrap_err().to_string();
        assert!(e.contains("random generator"), "{e}");

        let mut orphan: WorldState = snap.decode(MODEL_ID, SCHEMA_VERSION).unwrap();
        orphan.genealogy = Genealogy::new();
        assert!(EvolutionWorld::from_state(orphan).is_err(), "store and genealogy must agree");
    }
}
