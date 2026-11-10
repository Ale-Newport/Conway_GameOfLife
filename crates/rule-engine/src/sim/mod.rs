//! The custom-life simulation ([`CustomLifeSim`]).
//!
//! **Representation.** Entities are agents with continuous positions
//! (`glam::Vec2`, one world unit = one field cell) stored in an
//! organism-engine [`AgentStore`] (stable ids, id order). Resource fields
//! are dense per-cell grids. Continuous positions were chosen over cells
//! so that sizes, contact distances, sensing radii and speeds are real
//! quantities and many entities can share a cell (bacteria on a nutrient
//! patch); the cost is that entities may overlap (there is no exclusion).
//!
//! **Tick pipeline** (one tick = one time unit):
//! 1. *Frame*: freeze the state and index positions.
//! 2. *Events and rules*: per entity in id order (in parallel when the
//!    population is large), `on_birth` / `on_state_enter` / `on_collision`
//!    and `on_tick` rules in priority order, then script rules, then the
//!    default movement (`decide`).
//! 3. *Resolve intents and apply* (`resolve`).
//! 4. *Fields*: decay, regrowth, diffusion (`fields`).
//! 5. *Random events*, then *end conditions* (`world`).

pub mod command;
pub(crate) mod decide;
pub mod entity;
pub(crate) mod eval;
pub(crate) mod fields;
pub(crate) mod frame;
pub(crate) mod intent;
pub mod params;
pub(crate) mod report;
pub(crate) mod resolve;
pub mod space;
pub mod view;
pub(crate) mod world;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use organism_engine::AgentStore;
use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, EventLog, EventSeverity, Grid2, Metrics, ModelDescriptor, ParamSet, ParamValue, SimError, SimEvent,
    SimRng, Simulation, Snapshot, SpatialGrid,
};

use crate::compile::{CompiledSystem, compile};
use crate::def::LifeSystem;
use crate::error::RuleEngineError;
use crate::scenarios::Scenario;
use crate::script::ScriptError;
use command::{CustomLifeCommand, EditReport, PaintMode, classify_edit};
use entity::Entity;
use frame::Frame;
use space::Space;
use view::{CustomLifeView, CustomLifeViewRequest};

/// Model id used in descriptors and snapshots.
pub const MODEL_ID: &str = "custom_life";
/// Snapshot schema version (2 added the reference design; version 1
/// snapshots are still restored, with their definition as the reference).
pub const SCHEMA_VERSION: u32 = 2;
/// Maximum number of script errors kept in the error list.
pub const MAX_SCRIPT_ERRORS: usize = 100;

/// Counters of the last tick (metrics).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TickStats {
    /// Births per type.
    pub births: Vec<u32>,
    /// Deaths per type.
    pub deaths: Vec<u32>,
    /// Firings per compiled rule.
    pub rule_fired: Vec<u32>,
    /// Amount consumed per resource field.
    pub consumed: Vec<f64>,
    /// Successful `eat` actions.
    pub eaten: u32,
    /// Entities killed by attacks.
    pub killed: u32,
    /// Completed trades.
    pub trades: u32,
    /// Firings cancelled because their interaction was refused.
    pub interactions_refused: u32,
    /// Births and spawns refused by `max_entities`.
    pub births_refused: u64,
    /// Random events that happened.
    pub random_events: u32,
}

impl TickStats {
    fn reset(&mut self, compiled: &CompiledSystem) {
        let types = compiled.types().len();
        *self = Self {
            births: vec![0; types],
            deaths: vec![0; types],
            rule_fired: vec![0; compiled.rules().len()],
            consumed: vec![0.0; compiled.source().world.resources.len()],
            ..Self::default()
        };
    }
}

mod arc_serde {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(value: &Arc<T>, s: S) -> Result<S::Ok, S::Error> {
        (**value).serialize(s)
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Arc<T>, D::Error> {
        T::deserialize(d).map(Arc::new)
    }
}

/// Complete serialisable state (the snapshot payload).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct State {
    pub system: LifeSystem,
    pub seed: u64,
    pub tick: u64,
    pub rng: SimRng,
    #[serde(with = "arc_serde")]
    pub store: Arc<AgentStore<Entity>>,
    #[serde(with = "arc_serde")]
    pub fields: Arc<Vec<Grid2<f32>>>,
    pub fired_total: Vec<u64>,
    pub script_disabled: Vec<bool>,
    pub script_errors: Vec<ScriptError>,
    pub events: EventLog,
    pub finished: Option<String>,
    pub pause_on_end: bool,
    pub stats: TickStats,
}

impl State {
    /// Freezes the current state for a decision phase.
    pub(crate) fn frame(&self, compiled: &Arc<CompiledSystem>) -> Frame {
        let sys = compiled.source();
        let space = Space::new(sys.world.topology, sys.world.width, sys.world.height);
        let cell = compiled.types().iter().map(|t| t.def.perception).fold(2.0f32, f32::max).min(8.0);
        let mut grid = SpatialGrid::new(space.width, space.height, cell);
        grid.rebuild(self.store.as_slice().iter().enumerate().map(|(i, e)| (i as u32, e.kin.position)));
        let mut population = vec![0u32; compiled.types().len()];
        for e in self.store.as_slice() {
            population[usize::from(e.kind)] += 1;
        }
        Frame {
            tick: self.tick,
            seed: self.seed,
            compiled: Arc::clone(compiled),
            space,
            grid,
            store: Arc::clone(&self.store),
            fields: Arc::clone(&self.fields),
            population,
        }
    }

    /// Disables script `index` and records its first error.
    pub(crate) fn record_script_error(&mut self, index: usize, error: ScriptError) {
        let Some(flag) = self.script_disabled.get_mut(index) else { return };
        if *flag {
            return;
        }
        *flag = true;
        self.events.push(
            SimEvent::new(self.tick, "script_error", format!("{error}; the script was disabled"))
                .severity(EventSeverity::Major),
        );
        if self.script_errors.len() < MAX_SCRIPT_ERRORS {
            self.script_errors.push(error);
        }
    }
}

/// A running custom life system.
pub struct CustomLifeSim {
    state: State,
    compiled: Arc<CompiledSystem>,
    /// System and seed restored by `reset()`.
    initial: (LifeSystem, u64),
    /// The design as last loaded or applied: parameter defaults are its
    /// values ([`params::specs_with_defaults`]); parameter changes leave it
    /// alone.
    reference: LifeSystem,
    scratch: Grid2<f32>,
}

impl std::fmt::Debug for CustomLifeSim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomLifeSim")
            .field("system", &self.state.system.name)
            .field("tick", &self.state.tick)
            .field("entities", &self.state.store.len())
            .finish_non_exhaustive()
    }
}

impl CustomLifeSim {
    /// Compiles `system` and builds its initial state.
    pub fn new(system: LifeSystem, seed: u64) -> Result<Self, RuleEngineError> {
        let compiled = Arc::new(compile(&system)?);
        let state = world::initial_state(&compiled, seed, true);
        Ok(Self { state, compiled, initial: (system.clone(), seed), reference: system, scratch: Grid2::new(1, 1, 0.0) })
    }

    /// Builds the simulation of a scenario (its system and seed).
    pub fn from_scenario(scenario: &Scenario) -> Result<Self, RuleEngineError> {
        Self::new(scenario.system.clone(), scenario.seed)
    }

    /// The compiled system (types, rules incl. generated ones, warnings, graph).
    pub fn compiled(&self) -> &CompiledSystem {
        &self.compiled
    }

    /// The definition currently simulated.
    pub fn system(&self) -> &LifeSystem {
        &self.state.system
    }

    /// The definition and seed `reset()` will use.
    pub fn reset_target(&self) -> (&LifeSystem, u64) {
        (&self.initial.0, self.initial.1)
    }

    /// The design parameter defaults refer to: the system as last loaded
    /// ([`Self::new`]) or applied ([`Self::edit_system`]); snapshots carry it.
    pub fn reference(&self) -> &LifeSystem {
        &self.reference
    }

    /// Replaces the reference design without touching the simulation (e.g.
    /// to rebuild a session around a restored state).
    pub fn set_reference(&mut self, reference: LifeSystem) {
        self.reference = reference;
    }

    /// Current tick.
    pub fn current_tick(&self) -> u64 {
        self.state.tick
    }

    /// Live entities with their ids, in id order.
    pub fn entities(&self) -> impl Iterator<Item = (organism_engine::AgentId, &Entity)> {
        self.state.store.iter()
    }

    /// Live population of a type (0 for unknown names).
    pub fn population(&self, entity: &str) -> usize {
        self.compiled
            .type_index(entity)
            .map_or(0, |t| self.state.store.as_slice().iter().filter(|e| e.kind == t).count())
    }

    /// Total population.
    pub fn total_population(&self) -> usize {
        self.state.store.len()
    }

    /// Sum of a resource field (0 for unknown names).
    pub fn resource_total(&self, resource: &str) -> f64 {
        self.state.system.resource_index(resource).map_or(0.0, |r| self.state.fields[r].sum())
    }

    /// Script failures (each disabled its script).
    pub fn script_errors(&self) -> &[ScriptError] {
        &self.state.script_errors
    }

    /// Notable events (extinctions, random events, script errors, edits).
    pub fn events(&self) -> &EventLog {
        &self.state.events
    }

    /// Counters of the last tick.
    pub fn last_tick_stats(&self) -> &TickStats {
        &self.state.stats
    }

    /// Firings per compiled rule since the last reset.
    pub fn rule_fire_totals(&self) -> &[u64] {
        &self.state.fired_total
    }

    /// Why the run ended, if an end condition was met.
    pub fn finished_reason(&self) -> Option<&str> {
        self.state.finished.as_deref()
    }

    /// Flag that aborts running scripts (each is then disabled and reported).
    pub fn script_cancel_handle(&self) -> Arc<AtomicBool> {
        self.compiled.scripts().cancel_handle()
    }

    /// Whether `reset()` would start from a different definition or seed
    /// than the running one (a structural edit or reset-only parameter is
    /// waiting).
    pub fn reset_pending(&self) -> bool {
        self.initial.1 != self.state.seed || self.initial.0 != self.state.system
    }

    /// What replacing the definition with `system` would do.
    pub fn classify_edit(&self, system: &LifeSystem) -> EditReport {
        classify_edit(&self.state.system, system)
    }

    /// Replaces the definition. Compatible edits apply from the next tick
    /// (live entities are kept, cooldowns restart, values are clamped);
    /// structural edits become the system of the next reset. Either way
    /// `system` becomes the reference design. Invalid systems are rejected
    /// with all diagnostics and change nothing.
    pub fn edit_system(&mut self, system: LifeSystem) -> Result<EditReport, RuleEngineError> {
        let report = self.redefine(system.clone())?;
        self.reference = system;
        Ok(report)
    }

    /// [`Self::edit_system`] without changing the reference design.
    fn redefine(&mut self, system: LifeSystem) -> Result<EditReport, RuleEngineError> {
        let compiled = Arc::new(compile(&system)?);
        let report = classify_edit(&self.state.system, &system);
        let seed = self.initial.1;
        if report.policy == ApplyPolicy::RequiresReset {
            self.initial = (system, seed);
        } else {
            self.hot_swap(compiled, system.clone());
            self.initial = (system, seed);
        }
        self.state.events.push(SimEvent::new(
            self.state.tick,
            "definition_edit",
            format!("definition edited ({})", report.policy.label()),
        ));
        Ok(report)
    }

    fn hot_swap(&mut self, compiled: Arc<CompiledSystem>, system: LifeSystem) {
        let src = compiled.source();
        let space = Space::new(src.world.topology, src.world.width, src.world.height);
        let store = Arc::make_mut(&mut self.state.store);
        let fields = Arc::make_mut(&mut self.state.fields);
        for e in store.as_mut_slice() {
            let ty = &compiled.types()[usize::from(e.kind)];
            e.cooldowns = vec![0; ty.rules.len()];
            e.energy = e.energy.min(ty.def.energy.max);
            for (v, p) in e.props.iter_mut().zip(&ty.def.properties) {
                *v = v.clamp(p.min, p.max);
            }
            // Stock above a lowered capacity is released into the field, so
            // carried resources stay conserved across edits.
            let (x, y) = space.cell(e.kin.position);
            for ((v, c), &r) in e.carried.iter_mut().zip(&ty.def.carries).zip(&ty.carried) {
                let excess = *v - c.capacity;
                if excess > 0.0 {
                    *v = c.capacity;
                    if let Some(f) = fields.get_mut(usize::from(r)) {
                        *f.get_mut(x, y) += excess as f32;
                    }
                }
            }
        }
        self.state.system = system;
        self.state.fired_total = vec![0; compiled.rules().len()];
        self.state.script_disabled = vec![false; compiled.scripts().scripts().len()];
        self.state.script_errors.clear();
        self.state.finished = None;
        self.state.stats.reset(&compiled);
        self.compiled = compiled;
    }

    fn param_policy(&mut self, key: &str, value: &ParamValue) -> Result<ApplyPolicy, SimError> {
        let spec = params::specs_with_target(&self.state.system, &self.initial.0)
            .into_iter()
            .find(|s| s.key == key)
            .ok_or_else(|| SimError::UnknownParameter(key.to_string()))?;
        let value = spec.validate(value)?;
        if key == "pause_on_end" {
            self.state.pause_on_end = value.as_bool().unwrap_or(true);
            return Ok(spec.apply);
        }
        if key == "seed" {
            let seed = value.as_i64().unwrap_or(0) as u64;
            self.initial.1 = seed;
            return Ok(spec.apply);
        }
        // The reset target may already hold edits that wait for a reset, so
        // the parameter is written into it as well as (for next-tick
        // parameters) into the running system.
        let mut target = self.initial.0.clone();
        params::apply_param(&mut target, key, &value)?;
        compile(&target).map_err(SimError::from)?;
        if spec.apply != ApplyPolicy::RequiresReset {
            let mut current = self.state.system.clone();
            params::apply_param(&mut current, key, &value)?;
            self.redefine(current).map_err(SimError::from)?;
        }
        self.initial.0 = target;
        Ok(spec.apply)
    }
}

impl Simulation for CustomLifeSim {
    type Command = CustomLifeCommand;
    type ViewRequest = CustomLifeViewRequest;
    type View = CustomLifeView;

    fn descriptor(&self) -> ModelDescriptor {
        report::descriptor(&self.compiled, &self.initial.0, &self.reference)
    }

    fn tick(&self) -> u64 {
        self.state.tick
    }

    fn step(&mut self) {
        self.state.stats.reset(&self.compiled);
        let frame = Arc::new(self.state.frame(&self.compiled));
        let decisions = decide::decide_all(&frame, &self.state.script_disabled);
        resolve::resolve(&mut self.state, &self.compiled, frame, decisions);
        let sys = self.compiled.source();
        let fields = Arc::make_mut(&mut self.state.fields);
        fields::update_fields(fields, &sys.world.resources, sys.world.topology, &mut self.scratch);
        world::random_events(&mut self.state, &self.compiled);
        for (total, n) in self.state.fired_total.iter_mut().zip(&self.state.stats.rule_fired) {
            *total += u64::from(*n);
        }
        self.state.tick += 1;
        world::check_end(&mut self.state, &self.compiled);
        self.compiled.scripts().cancel_handle().store(false, Ordering::Relaxed);
    }

    fn reset(&mut self) {
        let (system, seed) = self.initial.clone();
        // The reset target was validated when it was stored.
        if let Ok(compiled) = compile(&system) {
            self.compiled = Arc::new(compiled);
        }
        self.state = world::initial_state(&self.compiled, seed, self.state.pause_on_end);
    }

    fn apply(&mut self, command: CustomLifeCommand) -> Result<(), SimError> {
        let sys = self.compiled.source();
        let space = Space::new(sys.world.topology, sys.world.width, sys.world.height);
        match command {
            CustomLifeCommand::Spawn { entity, x, y } => {
                let t = self.compiled.type_index(&entity).ok_or_else(|| {
                    let known: Vec<&str> = self.compiled.types().iter().map(|t| t.def.name.as_str()).collect();
                    SimError::InvalidCommand(format!("unknown entity type `{entity}` (known: {})", known.join(", ")))
                })?;
                if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 || x > space.width || y > space.height {
                    return Err(SimError::InvalidCommand(format!(
                        "position ({x}, {y}) is outside the {}×{} world",
                        space.width, space.height
                    )));
                }
                if self.state.store.len() >= sys.world.max_entities as usize {
                    return Err(SimError::LimitExceeded(format!("max_entities ({}) reached", sys.world.max_entities)));
                }
                let ty = &self.compiled.types()[usize::from(t)];
                let e = Entity::new(
                    t,
                    ty,
                    space.place(simulation_core::glam::Vec2::new(x, y)),
                    0.0,
                    self.state.tick,
                    ty.def.energy.initial,
                );
                let store = Arc::make_mut(&mut self.state.store);
                store.queue_spawn(e);
                store.commit();
                Ok(())
            }
            CustomLifeCommand::Remove { id } => {
                let id = organism_engine::AgentId(id);
                let Some(e) = self.state.store.get(id).cloned() else {
                    return Err(SimError::InvalidCommand(format!("no live entity with id {}", id.get())));
                };
                world::drop_carried(&mut self.state.fields, &self.compiled, &space, &e);
                let store = Arc::make_mut(&mut self.state.store);
                store.queue_despawn(id);
                store.commit();
                Ok(())
            }
            CustomLifeCommand::PaintResource { resource, x, y, radius, amount, mode } => {
                let r = sys
                    .resource_index(&resource)
                    .ok_or_else(|| SimError::InvalidCommand(format!("unknown resource `{resource}`")))?;
                if ![x, y, radius, amount].iter().all(|v| v.is_finite()) || radius < 0.0 {
                    return Err(SimError::InvalidCommand("brush values must be finite, radius ≥ 0".into()));
                }
                let (w, h) = (sys.world.width as usize, sys.world.height as usize);
                let topology = sys.world.topology;
                let field = &mut Arc::make_mut(&mut self.state.fields)[r];
                fields::disc_cells(x, y, radius, w, h, topology, |i| {
                    let v = &mut field.as_mut_slice()[i];
                    *v = match mode {
                        PaintMode::Add => (*v + amount).max(0.0),
                        PaintMode::Set => amount.max(0.0),
                    };
                });
                Ok(())
            }
            CustomLifeCommand::ReplaceSystem(system) => self.edit_system(*system).map(|_| ()).map_err(SimError::from),
            CustomLifeCommand::SetPauseOnEnd(on) => {
                self.state.pause_on_end = on;
                Ok(())
            }
            CustomLifeCommand::ReenableScripts => {
                self.state.script_disabled.iter_mut().for_each(|d| *d = false);
                self.state.script_errors.clear();
                Ok(())
            }
        }
    }

    fn params(&self) -> ParamSet {
        params::params_with_target(&self.state.system, &self.initial.0, self.initial.1, self.state.pause_on_end)
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        self.param_policy(key, &value)
    }

    fn metrics(&self) -> Metrics {
        report::metrics(&self.state, &self.compiled)
    }

    fn view(&self, request: &CustomLifeViewRequest) -> CustomLifeView {
        report::view(&self.state, &self.compiled, request, self.reset_pending())
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(MODEL_ID, SCHEMA_VERSION, self.state.tick, &(&self.state, &self.reference))
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let (state, reference): (State, LifeSystem) = if snapshot.schema_version < 2 {
            let state: State = snapshot.decode(MODEL_ID, 1)?;
            let reference = state.system.clone();
            (state, reference)
        } else {
            snapshot.decode(MODEL_ID, SCHEMA_VERSION)?
        };
        let compiled = Arc::new(compile(&state.system).map_err(SimError::from)?);
        let world = &compiled.source().world;
        let (w, h) = (world.width as usize, world.height as usize);
        let consistent = state.fired_total.len() == compiled.rules().len()
            && state.script_disabled.len() == compiled.scripts().scripts().len()
            && state.fields.len() == world.resources.len()
            && state.fields.iter().all(|f| f.width() == w && f.height() == h)
            && state.store.as_slice().iter().all(|e| {
                let p = e.kin.position;
                let inside = p.is_finite() && (0.0..=w as f32).contains(&p.x) && (0.0..=h as f32).contains(&p.y);
                inside
                    && e.energy.is_finite()
                    && compiled.types().get(usize::from(e.kind)).is_some_and(|t| {
                        e.cooldowns.len() == t.rules.len()
                            && e.props.len() == t.def.properties.len()
                            && e.carried.len() == t.carried.len()
                            && usize::from(e.state) < t.states.len()
                    })
            });
        if !consistent {
            return Err(SimError::InvalidConfig("snapshot does not match its own definition".into()));
        }
        self.initial = (state.system.clone(), state.seed);
        self.reference = reference;
        self.state = state;
        self.compiled = compiled;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        let fields: usize = self.state.fields.iter().map(|g| g.as_slice().len() * 4).sum();
        let per_entity = std::mem::size_of::<Entity>() + 64;
        fields + self.state.store.len() * per_entity
    }

    fn is_finished(&self) -> bool {
        self.state.pause_on_end && self.state.finished.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenarios;

    fn tampered(edit: impl FnOnce(&mut State)) -> Snapshot {
        let mut sim = CustomLifeSim::from_scenario(&scenarios::builtin_by_id("bacterial-growth").unwrap()).unwrap();
        edit(&mut sim.state);
        sim.snapshot().unwrap()
    }

    #[test]
    fn inconsistent_snapshots_are_rejected_instead_of_panicking_later() {
        let mut sim = CustomLifeSim::from_scenario(&scenarios::builtin_by_id("bacterial-growth").unwrap()).unwrap();
        let bad: [fn(&mut State); 4] = [
            |s| Arc::make_mut(&mut s.fields)[0] = Grid2::new(3, 3, 0.0),
            |s| Arc::make_mut(&mut s.store).as_mut_slice()[0].state = 7,
            |s| Arc::make_mut(&mut s.store).as_mut_slice()[0].kin.position.x = f32::NAN,
            |s| Arc::make_mut(&mut s.store).as_mut_slice()[0].kin.position.y = 1.0e6,
        ];
        for edit in bad {
            let snap = tampered(edit);
            assert!(matches!(sim.restore(&snap), Err(SimError::InvalidConfig(_))));
        }
        // The simulation is untouched and keeps running.
        sim.advance(3);
        assert_eq!(sim.tick(), 3);
        let good = tampered(|_| {});
        sim.restore(&good).unwrap();
        assert_eq!(sim.tick(), 0);
    }

    #[test]
    fn version_1_snapshots_restore_with_their_definition_as_the_reference() {
        let mut old = CustomLifeSim::from_scenario(&scenarios::builtin_by_id("bacterial-growth").unwrap()).unwrap();
        old.set_param("entity.bacterium.metabolism", ParamValue::Float(0.5)).unwrap();
        old.advance(4);
        let v1 = Snapshot::encode(MODEL_ID, 1, old.state.tick, &old.state).unwrap();
        let mut sim = CustomLifeSim::from_scenario(&scenarios::builtin_by_id("robot-swarm").unwrap()).unwrap();
        sim.restore(&v1).unwrap();
        assert_eq!((sim.tick(), sim.system()), (4, old.system()));
        assert_eq!(sim.reference(), old.system());
    }
}
