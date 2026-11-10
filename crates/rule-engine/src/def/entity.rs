//! Entity type definitions.

use serde::{Deserialize, Serialize};

fn default_color() -> [f32; 4] {
    [0.8, 0.8, 0.8, 1.0]
}

fn one_f32() -> f32 {
    1.0
}

fn default_perception() -> f32 {
    5.0
}

fn one_u32() -> u32 {
    1
}

/// A numeric property declared by an entity type (readable as `self.name`,
/// writable with `set`/`add`; values are clamped into `[min, max]`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertyDef {
    /// Property name (an identifier).
    pub name: String,
    /// Initial value of new entities.
    #[serde(default)]
    pub default: f64,
    /// Lower bound.
    #[serde(default = "PropertyDef::default_min")]
    pub min: f64,
    /// Upper bound.
    #[serde(default = "PropertyDef::default_max")]
    pub max: f64,
}

impl PropertyDef {
    fn default_min() -> f64 {
        -1.0e9
    }

    fn default_max() -> f64 {
        1.0e9
    }
}

/// A world resource an entity can hold (`consume` fills it instead of
/// producing energy, `emit` releases from it, death drops it back into the
/// field, `trade` exchanges it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CarriedResource {
    /// Name of a resource field of the world.
    pub resource: String,
    /// Amount carried by new entities (created from nothing at birth).
    #[serde(default)]
    pub initial: f64,
    /// Maximum amount carried.
    pub capacity: f64,
}

/// Default movement, used whenever no rule or script claimed the movement
/// slot of an entity in a tick.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum MovementDef {
    /// Sessile: never moves (movement actions are rejected by validation).
    #[default]
    None,
    /// Uncorrelated random walk: a uniformly random direction every tick.
    RandomWalk {
        /// Distance per tick in world units.
        speed: f32,
    },
    /// Correlated random walk: the heading turns by `U(-turn, turn)` radians per tick.
    Directed {
        /// Distance per tick in world units.
        speed: f32,
        /// Maximum turn per tick in radians.
        turn: f32,
    },
}

impl MovementDef {
    /// Speed of the default movement (0 when sessile).
    pub fn speed(&self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::RandomWalk { speed } | Self::Directed { speed, .. } => *speed,
        }
    }

    /// Whether the type can move at all.
    pub fn is_mobile(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Energy budget of an entity type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnergyDef {
    /// Energy of entities created at start-up or by `spawn`.
    #[serde(default = "EnergyDef::default_initial")]
    pub initial: f64,
    /// Maximum energy (gains above it are lost).
    #[serde(default = "EnergyDef::default_max")]
    pub max: f64,
    /// Energy lost every tick (basal metabolism).
    #[serde(default)]
    pub metabolism: f64,
    /// Whether entities die when their energy reaches 0.
    #[serde(default = "EnergyDef::default_starvation")]
    pub starvation: bool,
}

impl EnergyDef {
    fn default_initial() -> f64 {
        10.0
    }

    fn default_max() -> f64 {
        100.0
    }

    fn default_starvation() -> bool {
        true
    }
}

impl Default for EnergyDef {
    fn default() -> Self {
        Self { initial: 10.0, max: 100.0, metabolism: 0.0, starvation: true }
    }
}

/// How an entity type reproduces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReproductionMode {
    /// Cannot reproduce (`reproduce` is rejected by validation).
    #[default]
    None,
    /// One parent.
    Asexual,
    /// Two parents (`reproduce(partner=...)`).
    Sexual,
}

/// Reproduction capability. Offspring receive the energy the parents pay
/// (`cost` per parent, split evenly over the litter), so reproduction never
/// creates energy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReproductionDef {
    /// Mode.
    #[serde(default)]
    pub mode: ReproductionMode,
    /// Energy each parent pays; also the minimum energy needed.
    #[serde(default)]
    pub cost: f64,
    /// Minimum ticks between two reproductions of the same parent.
    #[serde(default)]
    pub cooldown: u64,
    /// Offspring per reproduction.
    #[serde(default = "one_u32")]
    pub litter: u32,
    /// Per-tick probability of reproducing when eligible, through a
    /// generated rule; 0 means "only when a rule says `reproduce`".
    #[serde(default)]
    pub rate: f64,
}

impl Default for ReproductionDef {
    fn default() -> Self {
        Self { mode: ReproductionMode::None, cost: 0.0, cooldown: 0, litter: 1, rate: 0.0 }
    }
}

/// Ageing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LifecycleDef {
    /// Entities die of old age when their age reaches this many ticks.
    #[serde(default)]
    pub max_age: Option<u64>,
    /// Age from which an entity may reproduce.
    #[serde(default)]
    pub maturity: u64,
}

/// Where the initial population is placed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Distribution {
    /// Uniformly over the whole world.
    #[default]
    Uniform,
    /// Uniformly inside a disc.
    Cluster {
        /// Centre x.
        x: f32,
        /// Centre y.
        y: f32,
        /// Radius.
        radius: f32,
    },
    /// Uniformly inside a rectangle.
    Rect {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Width.
        width: f32,
        /// Height.
        height: f32,
    },
    /// On a regular lattice covering the world (row-major, cell centres).
    Grid,
}

/// Initial population of a type.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InitialPopulation {
    /// Number of entities created at reset.
    #[serde(default)]
    pub count: u32,
    /// Placement.
    #[serde(default)]
    pub distribution: Distribution,
}

/// A kind of entity in a custom life system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityType {
    /// Unique name (an identifier, used in rules).
    pub name: String,
    /// Free text shown in the legend.
    #[serde(default)]
    pub description: String,
    /// RGBA colour in `[0, 1]`.
    #[serde(default = "default_color")]
    pub color: [f32; 4],
    /// Glyph or emoji shown in the legend and in icon render mode.
    #[serde(default)]
    pub icon: String,
    /// Sprite or 3D model id for renderers (empty = default shape).
    #[serde(default)]
    pub sprite: String,
    /// Diameter in world units; two entities touch when their centres are
    /// closer than the mean of their diameters.
    #[serde(default = "one_f32")]
    pub size: f32,
    /// User-declared numeric properties.
    #[serde(default)]
    pub properties: Vec<PropertyDef>,
    /// Named states; the first is the initial state. Empty means a single
    /// implicit state `default`.
    #[serde(default)]
    pub states: Vec<String>,
    /// Resources the entity can hold.
    #[serde(default)]
    pub carries: Vec<CarriedResource>,
    /// Default movement.
    #[serde(default)]
    pub movement: MovementDef,
    /// Energy budget.
    #[serde(default)]
    pub energy: EnergyDef,
    /// Reproduction capability.
    #[serde(default)]
    pub reproduction: ReproductionDef,
    /// Ageing.
    #[serde(default)]
    pub lifecycle: LifecycleDef,
    /// Default sensing radius of `nearby`/`nearest` (world units).
    #[serde(default = "default_perception")]
    pub perception: f32,
    /// Initial population.
    #[serde(default)]
    pub initial: InitialPopulation,
}

impl EntityType {
    /// A type with defaults for everything but the name.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            description: String::new(),
            color: default_color(),
            icon: String::new(),
            sprite: String::new(),
            size: 1.0,
            properties: Vec::new(),
            states: Vec::new(),
            carries: Vec::new(),
            movement: MovementDef::None,
            energy: EnergyDef::default(),
            reproduction: ReproductionDef::default(),
            lifecycle: LifecycleDef::default(),
            perception: default_perception(),
            initial: InitialPopulation::default(),
        }
    }

    /// States with the implicit `default` state filled in.
    pub fn state_names(&self) -> Vec<String> {
        if self.states.is_empty() { vec!["default".to_string()] } else { self.states.clone() }
    }
}
