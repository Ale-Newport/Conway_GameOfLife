//! [`VoxelLife`]: the 3D cellular automaton behind the shared
//! [`simulation_core::Simulation`] contract (model id `voxel.life`).

mod describe;
mod params;
mod snapshot;

use simulation_core::{
    ApplyPolicy, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamValue, SimError, SimRng, Simulation, Snapshot,
};

use crate::error::VoxelError;
use crate::geometry::{Axis, Bounds, Pos};
use crate::init::{MAX_RANDOM_VOXELS, WorldConfig};
use crate::rule::{Rule3D, Transition};
use crate::scenarios::Scenario;
use crate::stats::Stats;
use crate::view::{self, CountsView, SliceView, SurfaceView, VoxelView, VoxelViewRequest};
use crate::world::Storage;
use snapshot::SavedState;

/// Model identifier used in descriptors and snapshots.
pub const MODEL_ID: &str = "voxel.life";
/// Snapshot payload version written by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// Random stream of the initial state (see [`SimRng::derive`]).
const INIT_STREAM: u64 = 1;
/// Random stream of seedless [`VoxelCommand::Randomize`] commands.
const COMMAND_STREAM: u64 = 2;

/// A non-empty voxel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Voxel {
    /// Position.
    pub pos: Pos,
    /// State (1 = alive, 2… = dying).
    pub state: u8,
    /// Generations since the voxel was born or placed.
    pub age: u16,
}

/// User interventions accepted by [`VoxelLife`] through
/// [`Simulation::apply`]. Every command is validated before anything
/// changes; edited voxels get age 0.
#[derive(Clone, Debug, PartialEq)]
pub enum VoxelCommand {
    /// Sets one voxel (`state` 0 empties it).
    SetVoxel {
        /// Position (inside the world).
        pos: Pos,
        /// New state (below the rule's number of states).
        state: u8,
    },
    /// Empties one voxel.
    ClearVoxel {
        /// Position (inside the world).
        pos: Pos,
    },
    /// Sets several voxels; nothing changes if any entry is invalid.
    SetVoxels(Vec<(Pos, u8)>),
    /// Sets every voxel of the inclusive box `min..=max`, clipped to the
    /// world, to `state`.
    FillBox {
        /// One corner.
        min: Pos,
        /// The opposite corner (inclusive).
        max: Pos,
        /// State to write.
        state: u8,
    },
    /// Empties the world.
    Clear,
    /// Empties the world and draws a new centred random cube with the
    /// current cube side, density and mirror settings. With `seed: None` the
    /// model's own generator is used (and advances); with `Some(seed)` a
    /// fresh generator seeded with `seed`.
    Randomize {
        /// Optional explicit seed.
        seed: Option<u64>,
    },
}

/// Three-dimensional Life as a [`Simulation`].
///
/// Construct it from a [`Scenario`] (see [`crate::scenarios::builtin`]).
/// Rule changes apply at the next tick; world and initial-state parameters
/// are pending until [`Simulation::reset`], which rebuilds the world from
/// them and the seed. A sparse world that outgrows
/// [`crate::MAX_SPARSE_CHUNKS`] halts ([`Simulation::is_finished`]); any
/// successful command or a reset clears the halt, and restoring a snapshot
/// restores the halt state it was taken in.
pub struct VoxelLife {
    scenario: Scenario,
    rule: Rule3D,
    transition: Transition,
    active: WorldConfig,
    pending: WorldConfig,
    storage: Storage,
    stats: Stats,
    tick: u64,
    rng: SimRng,
    halted: Option<String>,
}

impl VoxelLife {
    /// Builds the scenario's world at generation 0.
    pub fn new(scenario: Scenario) -> Result<Self, VoxelError> {
        scenario.validate()?;
        let config = scenario.config();
        let rule = scenario.rule;
        let mut life = Self {
            transition: rule.transition(),
            rule,
            storage: Storage::new(config.kind, config.size),
            stats: Stats::new(rule.states()),
            active: config.clone(),
            pending: config,
            scenario,
            tick: 0,
            rng: SimRng::seed_from_u64(0),
            halted: None,
        };
        life.rebuild();
        Ok(life)
    }

    /// Recreates the world from the pending configuration (the reset).
    fn rebuild(&mut self) {
        self.active = self.pending.clone();
        self.storage = Storage::new(self.active.kind, self.active.size);
        let mut init_rng = SimRng::derive(self.active.seed, INIT_STREAM);
        self.active.populate(&mut self.storage, self.rule.states(), &mut init_rng);
        self.rng = SimRng::derive(self.active.seed, COMMAND_STREAM);
        self.tick = 0;
        self.halted = None;
        self.stats = self.storage.stats(self.rule.states());
    }

    /// The scenario the model was built from.
    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    /// The rule being executed.
    pub fn rule(&self) -> Rule3D {
        self.rule
    }

    /// Configuration of the current world.
    pub fn config(&self) -> &WorldConfig {
        &self.active
    }

    /// Configuration the next reset will use.
    pub fn pending_config(&self) -> &WorldConfig {
        &self.pending
    }

    /// Box of coordinates that can hold voxels.
    pub fn bounds(&self) -> Bounds {
        self.storage.bounds()
    }

    /// Statistics of the current generation.
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// State of the voxel at `pos` (0 outside the world).
    pub fn state_at(&self, pos: Pos) -> u8 {
        self.storage.state(pos)
    }

    /// The voxel at `pos` (state 0 and age 0 when empty or outside).
    pub fn voxel(&self, pos: Pos) -> Voxel {
        let (state, age) = self.storage.get(pos);
        Voxel { pos, state, age }
    }

    /// Every non-empty voxel, sorted by z, then y, then x.
    pub fn voxels(&self) -> Vec<Voxel> {
        self.storage.voxels().into_iter().map(|(pos, state, age)| Voxel { pos, state, age }).collect()
    }

    /// Why the model halted, if it did.
    pub fn halt_reason(&self) -> Option<&str> {
        self.halted.as_deref()
    }

    /// Replaces the rule; voxels in states the new rule lacks are emptied.
    fn set_rule(&mut self, rule: Rule3D) {
        self.rule = rule;
        self.transition = rule.transition();
        self.storage.clamp_states(rule.states());
        self.recount();
    }

    fn set_pending(&mut self, config: WorldConfig) {
        self.pending = config;
    }

    /// Recomputes statistics after an edit, keeping the last generation's
    /// birth and death counts.
    fn recount(&mut self) {
        let (births, deaths) = (self.stats.births(), self.stats.deaths());
        self.stats = self.storage.stats(self.rule.states()).with_transitions(births, deaths);
    }

    fn check_pos(&self, pos: Pos) -> Result<(), VoxelError> {
        let bounds = self.storage.bounds();
        if bounds.contains(pos) {
            Ok(())
        } else {
            Err(VoxelError::OutOfBounds { x: pos[0], y: pos[1], z: pos[2], bounds: bounds.to_string() })
        }
    }

    fn check_state(&self, state: u8) -> Result<(), VoxelError> {
        if u16::from(state) < self.rule.states() {
            Ok(())
        } else {
            Err(VoxelError::InvalidState { state, states: self.rule.states() })
        }
    }

    fn execute(&mut self, command: VoxelCommand) -> Result<(), VoxelError> {
        match command {
            VoxelCommand::SetVoxel { pos, state } => {
                self.check_pos(pos)?;
                self.check_state(state)?;
                if state != 0 {
                    self.storage.check_new_voxels([pos])?;
                }
                self.storage.set(pos, state);
            }
            VoxelCommand::ClearVoxel { pos } => {
                self.check_pos(pos)?;
                self.storage.set(pos, 0);
            }
            VoxelCommand::SetVoxels(voxels) => {
                for &(pos, state) in &voxels {
                    self.check_pos(pos)?;
                    self.check_state(state)?;
                }
                self.storage.check_new_voxels(voxels.iter().filter(|(_, s)| *s != 0).map(|(p, _)| *p))?;
                for (pos, state) in voxels {
                    self.storage.set(pos, state);
                }
            }
            VoxelCommand::FillBox { min, max, state } => {
                self.check_state(state)?;
                let corners = Bounds::point(min).including(max);
                let world = self.storage.bounds();
                let area = corners.intersection(&world).ok_or_else(|| VoxelError::BoxOutside {
                    requested: corners.to_string(),
                    world: world.to_string(),
                })?;
                if area.volume() > MAX_RANDOM_VOXELS {
                    let [x, y, z] = area.extent();
                    return Err(VoxelError::Limit(format!(
                        "the box {area} holds {x}×{y}×{z} voxels inside the world, more than the limit of \
                         {MAX_RANDOM_VOXELS} per command"
                    )));
                }
                if state != 0 {
                    self.storage.check_new_box(area)?;
                }
                for z in area.min[2]..=area.max[2] {
                    for y in area.min[1]..=area.max[1] {
                        for x in area.min[0]..=area.max[0] {
                            self.storage.set([x, y, z], state);
                        }
                    }
                }
            }
            VoxelCommand::Clear => self.storage.clear(),
            VoxelCommand::Randomize { seed } => {
                let config = WorldConfig { init: self.pending.init.clone(), ..self.active.clone() };
                config.check_cube()?;
                self.storage.clear();
                match seed {
                    Some(seed) => config.random_cube(&mut self.storage, &mut SimRng::seed_from_u64(seed)),
                    None => config.random_cube(&mut self.storage, &mut self.rng),
                }
            }
        }
        self.recount();
        self.halted = None;
        Ok(())
    }

    fn surface_view(&self, range: view::SliceRange, max_voxels: usize) -> SurfaceView {
        let (voxels, visible) = view::surface(&self.storage, &range, max_voxels);
        SurfaceView {
            tick: self.tick,
            states: self.rule.states(),
            world: self.storage.bounds(),
            range,
            truncated: (voxels.len() as u64) < visible,
            voxels,
            visible,
        }
    }

    fn slice_view(&self, axis: Axis, layer: i32, window: Option<view::SliceWindow>) -> SliceView {
        let world = self.storage.bounds();
        let nominal = Bounds::from_size(self.active.size);
        let automatic = if self.active.kind.is_sparse() {
            self.stats.bounds().map_or(nominal, |b| b.union(nominal))
        } else {
            world
        };
        let (window, truncated) = view::slice_window(axis, window, world, automatic);
        let cells = view::slice_cells(&self.storage, axis, layer, window);
        SliceView { tick: self.tick, axis, layer, window, cells, truncated }
    }

    fn counts_view(&self) -> CountsView {
        let population = self.stats.population();
        let empty = self.active.kind.is_finite().then(|| self.storage.bounds().volume() - population);
        let mut counts = self.stats.counts().to_vec();
        if let Some(slot) = counts.first_mut() {
            *slot = empty.unwrap_or(0);
        }
        CountsView { tick: self.tick, counts, empty, population }
    }
}

impl Simulation for VoxelLife {
    type Command = VoxelCommand;
    type ViewRequest = VoxelViewRequest;
    type View = VoxelView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(MODEL_ID, "Three-Dimensional Life", ModelFamily::Voxel)
            .summary(describe::SUMMARY)
            .rules(&describe::rules_text(&self.rule, self.active.kind, self.active.size))
            .params(params::param_specs(self))
            .metrics(describe::metric_specs(self.rule.states()))
            .time("generation", 1.0)
    }

    fn tick(&self) -> u64 {
        self.tick
    }

    fn step(&mut self) {
        if self.halted.is_some() {
            return;
        }
        match self.storage.step(&self.transition, self.rule.neighbourhood(), self.rule.states()) {
            Ok(stats) => {
                self.stats = stats;
                self.tick += 1;
            }
            Err(error) => self.halted = Some(error.to_string()),
        }
    }

    fn reset(&mut self) {
        self.rebuild();
    }

    fn apply(&mut self, command: VoxelCommand) -> Result<(), SimError> {
        self.execute(command).map_err(SimError::from)
    }

    fn params(&self) -> ParamSet {
        params::current(self)
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        params::set(self, key, value)
    }

    fn metrics(&self) -> Metrics {
        let stats = &self.stats;
        let mut metrics = Metrics::new(self.tick, self.tick as f64).with("population", stats.population() as f64);
        for (state, &count) in stats.counts().iter().enumerate().skip(1) {
            metrics.push(&format!("state_{state}"), count as f64);
        }
        metrics.push("births", stats.births() as f64).push("deaths", stats.deaths() as f64);
        if let (Some(bounds), Some(centroid)) = (stats.bounds(), stats.centroid()) {
            for (axis, name) in ["x", "y", "z"].iter().enumerate() {
                metrics.push(&format!("bbox_min_{name}"), f64::from(bounds.min[axis]));
                metrics.push(&format!("bbox_max_{name}"), f64::from(bounds.max[axis]));
            }
            for (axis, name) in ["x", "y", "z"].iter().enumerate() {
                metrics.push(&format!("centroid_{name}"), centroid[axis]);
            }
        }
        metrics
    }

    fn view(&self, request: &VoxelViewRequest) -> VoxelView {
        match *request {
            VoxelViewRequest::Surface { range, max_voxels } => VoxelView::Surface(self.surface_view(range, max_voxels)),
            VoxelViewRequest::Slice { axis, layer, window } => VoxelView::Slice(self.slice_view(axis, layer, window)),
            VoxelViewRequest::Counts => VoxelView::Counts(self.counts_view()),
        }
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(MODEL_ID, SCHEMA_VERSION, self.tick, &SavedState::capture(self))
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let saved: SavedState = snapshot.decode(MODEL_ID, SCHEMA_VERSION)?;
        let storage = saved.rebuild_storage().map_err(|e| SimError::Serialization(e.to_string()))?;
        let states = saved.rule.states();
        self.stats = storage.stats(states).with_transitions(saved.births, saved.deaths);
        self.storage = storage;
        self.scenario = saved.scenario;
        self.rule = saved.rule;
        self.transition = saved.rule.transition();
        self.active = saved.active;
        self.pending = saved.pending;
        self.tick = saved.tick;
        self.rng = saved.rng;
        self.halted = saved.halted;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.storage.memory_bytes() + std::mem::size_of::<Self>()
    }

    fn is_finished(&self) -> bool {
        self.halted.is_some()
    }
}
