//! World definition: size, topology, resource and environment fields,
//! random events and end conditions.

use serde::{Deserialize, Serialize};
use simulation_core::Topology;

fn default_topology() -> Topology {
    Topology::Torus
}

fn default_max_entities() -> u32 {
    20_000
}

fn default_field_color() -> [f32; 4] {
    [0.2, 0.7, 0.3, 1.0]
}

fn one() -> f64 {
    1.0
}

/// Axis of a gradient.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    /// Varies along x (left to right).
    #[default]
    X,
    /// Varies along y (top to bottom).
    Y,
}

/// Initial content of a resource field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FieldInit {
    /// The same amount in every cell.
    Uniform(f32),
    /// Independent uniform random amounts in `[min, max)`.
    Random {
        /// Lower bound.
        min: f32,
        /// Upper bound.
        max: f32,
    },
    /// `count` discs of radius `radius` filled with `amount` at random places.
    Patches {
        /// Number of patches.
        count: u32,
        /// Patch radius in cells.
        radius: f32,
        /// Amount per cell inside a patch.
        amount: f32,
    },
    /// Linear ramp from `from` to `to` along an axis.
    Gradient {
        /// Value at the start of the axis.
        from: f32,
        /// Value at the end of the axis.
        to: f32,
        /// Axis.
        axis: Axis,
    },
}

impl Default for FieldInit {
    fn default() -> Self {
        Self::Uniform(0.0)
    }
}

/// A consumable, regrowing, diffusing resource stored per cell.
///
/// Per tick and cell: `R ← R·(1 − decay)`, then `R ← min(max, R + regen)`
/// for cells below `max`, then explicit diffusion
/// `R ← R + diffusion·Σ_neighbours (R_n − R)` over the four neighbours
/// (topology-aware; zero flux at bounded edges).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceField {
    /// Unique name (used by `consume`, `emit`, `field`).
    pub name: String,
    /// Display colour.
    #[serde(default = "default_field_color")]
    pub color: [f32; 4],
    /// Initial content.
    #[serde(default)]
    pub initial: FieldInit,
    /// Amount added per cell and tick while below `max`.
    #[serde(default)]
    pub regen: f32,
    /// Regrowth ceiling per cell.
    #[serde(default = "ResourceField::default_max")]
    pub max: f32,
    /// Diffusion coefficient in `[0, 0.25]` (explicit-scheme stability limit).
    #[serde(default)]
    pub diffusion: f32,
    /// Fraction lost per tick in `[0, 1]`.
    #[serde(default)]
    pub decay: f32,
    /// Energy gained per unit consumed by entities that do not carry it.
    #[serde(default = "one")]
    pub energy_value: f64,
}

impl ResourceField {
    fn default_max() -> f32 {
        1.0
    }

    /// A field with the given name and defaults.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            color: default_field_color(),
            initial: FieldInit::default(),
            regen: 0.0,
            max: 1.0,
            diffusion: 0.0,
            decay: 0.0,
            energy_value: 1.0,
        }
    }
}

/// Spatial pattern of an environment field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EnvPattern {
    /// Same value everywhere.
    Constant(f64),
    /// Linear ramp along an axis.
    Gradient {
        /// Value at the start of the axis.
        from: f64,
        /// Value at the end of the axis.
        to: f64,
        /// Axis.
        axis: Axis,
    },
    /// `peak` at the centre falling linearly to `edge` at `radius` and beyond.
    Radial {
        /// Centre x.
        x: f64,
        /// Centre y.
        y: f64,
        /// Radius.
        radius: f64,
        /// Value at the centre.
        peak: f64,
        /// Value at and beyond the radius.
        edge: f64,
    },
}

/// Periodic modulation of an environment field:
/// `value · (1 + amplitude · sin(2π · tick / period))`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seasonality {
    /// Relative amplitude.
    pub amplitude: f64,
    /// Period in ticks (> 0).
    pub period: f64,
}

/// A read-only environmental quantity (light, temperature...) computed from
/// position and time; readable with `field("name")`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentField {
    /// Unique name.
    pub name: String,
    /// Spatial pattern.
    pub pattern: EnvPattern,
    /// Optional periodic modulation.
    #[serde(default)]
    pub seasonal: Option<Seasonality>,
}

/// Effect of a random event (placed at a uniformly random location).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EventEffect {
    /// Adds `amount` to every cell of a resource within `radius`.
    ResourcePulse {
        /// Resource field.
        resource: String,
        /// Amount per cell.
        amount: f32,
        /// Radius in cells.
        radius: f32,
    },
    /// Multiplies a whole resource field by `factor` (drought, bloom).
    ScaleResource {
        /// Resource field.
        resource: String,
        /// Factor (≥ 0).
        factor: f32,
    },
    /// Kills entities (optionally of one type) within `radius` with probability `kill_probability`.
    Catastrophe {
        /// Radius.
        radius: f32,
        /// Per-entity kill probability.
        kill_probability: f64,
        /// Restrict to one entity type.
        #[serde(default)]
        entity: Option<String>,
    },
    /// Creates `count` entities of a type around the location.
    Immigration {
        /// Entity type.
        entity: String,
        /// Number of entities.
        count: u32,
    },
}

/// An event that happens with a fixed probability every tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RandomEvent {
    /// Name shown in the event log.
    pub name: String,
    /// Chance per tick.
    pub probability: f64,
    /// What happens.
    pub effect: EventEffect,
}

/// World definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldDef {
    /// Width in cells (= world units).
    pub width: u32,
    /// Height in cells.
    pub height: u32,
    /// How the edges connect (`infinite` is not supported: fields need a size).
    #[serde(default = "default_topology")]
    pub topology: Topology,
    /// Consumable fields.
    #[serde(default)]
    pub resources: Vec<ResourceField>,
    /// Read-only environmental fields.
    #[serde(default)]
    pub environment: Vec<EnvironmentField>,
    /// Random events.
    #[serde(default)]
    pub random_events: Vec<RandomEvent>,
    /// Safety cap on the number of live entities; births and spawns beyond
    /// it are refused (and counted).
    #[serde(default = "default_max_entities")]
    pub max_entities: u32,
}

impl WorldDef {
    /// World of the given size with defaults for everything else.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            topology: default_topology(),
            resources: Vec::new(),
            environment: Vec::new(),
            random_events: Vec::new(),
            max_entities: default_max_entities(),
        }
    }
}

/// Condition that ends a run (the runner pauses when one is met, unless
/// pausing on end conditions is switched off).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EndCondition {
    /// The run reached this tick.
    TickLimit(u64),
    /// No entity of this type is left.
    Extinction(String),
    /// No entity at all is left.
    AllExtinct,
    /// The population (of a type, or total) exceeds `count`.
    PopulationAbove {
        /// Type, or `None` for the total.
        #[serde(default)]
        entity: Option<String>,
        /// Threshold.
        count: u32,
    },
    /// The population (of a type, or total) falls below `count`.
    PopulationBelow {
        /// Type, or `None` for the total.
        #[serde(default)]
        entity: Option<String>,
        /// Threshold.
        count: u32,
    },
}
