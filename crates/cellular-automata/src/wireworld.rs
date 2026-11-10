//! `WireworldSim`: Brian Silverman's Wireworld (1987).

use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamValue, SimError, Simulation,
    Snapshot, Topology,
};

use crate::engine::Lut;
use crate::error::CaError;
use crate::geometry::World;
use crate::life::PatternLibrary;
use crate::multistate::{PaintCommand, StateInit};
use crate::statemodel::{SavedCore, StateCore};
use crate::view::{GridView, GridViewRequest};

/// Model id.
pub const WIREWORLD_MODEL_ID: &str = "ca.wireworld";
const SCHEMA_VERSION: u32 = 1;

/// Empty cell.
pub const EMPTY: u8 = 0;
/// Electron head.
pub const HEAD: u8 = 1;
/// Electron tail.
pub const TAIL: u8 = 2;
/// Conductor (wire).
pub const WIRE: u8 = 3;
const STATES: u16 = 4;

/// Configuration of a [`WireworldSim`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireworldConfig {
    /// Finite topology (circuits normally use the bounded plane).
    #[serde(default = "bounded")]
    pub topology: Topology,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Activity trails.
    #[serde(default)]
    pub track_activity: bool,
    /// Initial content (patterns use Golly's states: `A` head, `B` tail, `C` wire).
    #[serde(default)]
    pub init: Vec<StateInit>,
}

fn bounded() -> Topology {
    Topology::Bounded
}

impl Default for WireworldConfig {
    fn default() -> Self {
        Self { topology: Topology::Bounded, width: 128, height: 96, track_activity: false, init: Vec::new() }
    }
}

/// Wireworld as a [`Simulation`].
///
/// States (Golly numbering): 0 empty, 1 electron head, 2 electron tail,
/// 3 conductor. Every generation: empty stays empty, a head becomes a tail,
/// a tail becomes a conductor, and a conductor becomes a head iff exactly 1
/// or 2 of its 8 Moore neighbours are heads.
pub struct WireworldSim {
    core: StateCore,
    lut: Lut,
}

/// The Wireworld transition table.
fn wireworld_lut() -> Lut {
    Lut::from_fn(STATES, |s, n| match s {
        HEAD => TAIL,
        TAIL => WIRE,
        WIRE if n == 1 || n == 2 => HEAD,
        other => other,
    })
}

impl WireworldSim {
    /// Builds the model.
    ///
    /// # Errors
    /// Invalid world, patterns or states above 3.
    pub fn new(config: &WireworldConfig, seed: u64, library: &PatternLibrary) -> Result<Self, CaError> {
        let world = World::new(config.topology, config.width, config.height)?;
        let core = StateCore::new(world, &config.init, library, seed, STATES, config.track_activity)?;
        Ok(Self { core, lut: wireworld_lut() })
    }

    /// State of a cell.
    pub fn cell(&self, x: i64, y: i64) -> u8 {
        self.core.grid.get(x, y)
    }

    /// Non-zero cells `(x, y, state)`, row-major.
    pub fn cells(&self) -> Vec<(i64, i64, u8)> {
        self.core.grid.cells()
    }

    /// Activity of a cell: 255 when it changed in the last generation, then
    /// 16 less per generation (0 when tracking is off).
    pub fn activity(&self, x: i64, y: i64) -> u8 {
        self.core.grid.activity(x, y)
    }

    /// The world.
    pub fn world(&self) -> World {
        self.core.world()
    }

    /// Why the model stopped, if it did (a reset that could not be carried
    /// out; the state is then unchanged).
    pub fn halt_reason(&self) -> Option<&str> {
        self.core.halted.as_deref()
    }
}

impl Simulation for WireworldSim {
    type Command = PaintCommand;
    type ViewRequest = GridViewRequest;
    type View = GridView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(WIREWORLD_MODEL_ID, "Wireworld", ModelFamily::Cellular)
            .summary("Brian Silverman's four-state automaton for electronic circuits: wires, diodes, clocks and gates.")
            .rules(
                "## Wireworld\n\n\
                 States: 0 = empty, 1 = electron head, 2 = electron tail, 3 = conductor (Golly numbering; RLE \
                 letters `.`, `A`, `B`, `C`). Moore neighbourhood (8 cells; positions outside a finite world map \
                 through its topology). Every generation, simultaneously:\n\n\
                 * empty → empty;\n\
                 * electron head → electron tail;\n\
                 * electron tail → conductor;\n\
                 * conductor → electron head iff exactly 1 or 2 neighbours are electron heads, otherwise conductor.",
            )
            .params(StateCore::world_specs("bounded", 128))
            .metrics(vec![
                MetricSpec::gauge("heads", "Electron heads", "cells", "Cells in state 1."),
                MetricSpec::gauge("tails", "Electron tails", "cells", "Cells in state 2."),
                MetricSpec::gauge("conductors", "Conductors", "cells", "Cells in state 3 (wire without electron)."),
                MetricSpec::gauge("generation", "Generation", "generations", "Generations since the last reset."),
            ])
    }

    fn tick(&self) -> u64 {
        self.core.generation
    }

    fn step(&mut self) {
        self.core.step(&self.lut);
    }

    fn reset(&mut self) {
        self.core.reset();
    }

    fn apply(&mut self, command: PaintCommand) -> Result<(), SimError> {
        Ok(self.core.apply(command, STATES)?)
    }

    fn params(&self) -> ParamSet {
        let mut set = ParamSet::new();
        for (k, v) in self.core.world_values() {
            set.set(k, v);
        }
        set
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let spec = self.descriptor().param(key).cloned().ok_or_else(|| SimError::UnknownParameter(key.into()))?;
        let value = spec.validate(&value)?;
        match self.core.set_world_param(key, &value) {
            Some(result) => result.map(|()| spec.apply),
            None => Err(SimError::UnknownParameter(key.into())),
        }
    }

    fn metrics(&self) -> Metrics {
        let h = self.core.grid.histogram();
        Metrics::new(self.core.generation, self.core.generation as f64)
            .with("heads", h[usize::from(HEAD)] as f64)
            .with("tails", h[usize::from(TAIL)] as f64)
            .with("conductors", h[usize::from(WIRE)] as f64)
            .with("generation", self.core.generation as f64)
    }

    fn view(&self, request: &GridViewRequest) -> GridView {
        self.core.grid.view(request)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(WIREWORLD_MODEL_ID, SCHEMA_VERSION, self.core.generation, &self.core.save())
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let saved: SavedCore = snapshot.decode(WIREWORLD_MODEL_ID, SCHEMA_VERSION)?;
        self.core = StateCore::load_saved(saved, STATES)?;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.core.grid.memory_bytes()
    }

    fn is_finished(&self) -> bool {
        self.core.halted.is_some()
    }
}
