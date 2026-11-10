//! Life-style sections of a species record: seasonal behaviour (migration
//! and hibernation), the kind of water aquatic species live in, and how the
//! species is represented in the simulation — individual agents, a density
//! field of tiny organisms, or a colony super-agent with forager squads.

use serde::{Deserialize, Serialize};

/// Water bodies an aquatic species can live in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaterKind {
    /// Fresh and salt water (euryhaline species).
    #[default]
    Any,
    /// Lakes, rivers and flooded land.
    Fresh,
    /// The ocean.
    Salt,
}

/// Dormancy in the cold season (hibernation of endotherms, brumation of
/// ectotherms).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hibernation {
    /// Daily mean air temperature (°C) below which the animal enters its
    /// den outside summer (autumn, winter or a still-cold spring).
    pub below_c: f32,
    /// Daily mean air temperature (°C) above which it wakes up (must be
    /// above `below_c`; the gap prevents flickering).
    pub wake_above_c: f32,
    /// Metabolic rate while dormant as a fraction of the basal rate
    /// (endotherms: at body temperature; ectotherms: at air temperature).
    pub metabolic_fraction: f32,
    /// Energy reserve (fraction of capacity) needed to start hibernating;
    /// leaner animals keep foraging.
    pub min_reserve_fraction: f32,
}

/// Seasonal behaviour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Seasonal {
    /// Migrates towards its preferred climate when the daily mean
    /// temperature where it stands leaves the preferred range.
    pub migrates: bool,
    /// Distance (m) at which a migrant samples the climate to choose its
    /// direction.
    pub migration_range_m: f32,
    /// Dormancy in the cold season, if any.
    pub hibernation: Option<Hibernation>,
}

impl Default for Seasonal {
    fn default() -> Self {
        Self { migrates: false, migration_range_m: 2000.0, hibernation: None }
    }
}

/// Parameters of a species simulated as a density field (tiny organisms in
/// very large numbers: insects, spiders, worms). Each cell holds a density
/// (individuals per m²) that feeds with a Holling type II functional
/// response, respires with the MTE rate of one individual, reproduces from
/// its net assimilation, dies and disperses. The renderer receives a few
/// representative markers per cell; they are not simulated individually.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldParams {
    /// Highest density (individuals per m²) in a cell of a preferred biome;
    /// other biomes allow a tenth of it, water none (density-dependent
    /// mortality above it).
    pub max_density_m2: f32,
    /// Largest per-capita birth rate per (real) day; assimilated energy
    /// beyond what this allows is respired.
    pub max_birth_per_day: f32,
    /// Background mortality per (real) day; dead bodies go to the litter.
    pub mortality_per_day: f32,
    /// Fraction of each cell's population exchanged with its 8 neighbours
    /// per day.
    pub dispersal_per_day: f32,
    /// Food (kg/m², all foods weighted by preference) at which intake is
    /// half the maximum (Holling type II).
    pub half_saturation_kg_m2: f32,
    /// Flower visits per individual per hour of feeding (pollinators only).
    #[serde(default)]
    pub flower_visits_per_hour: f32,
    /// Individuals represented by one display marker.
    pub individuals_per_marker: f32,
}

/// Parameters of a eusocial colony (ants, honey bees). A colony is a
/// super-agent at its nest with a worker count, developing brood and a food
/// store; a few forager squads (each standing for `workers_per_forager`
/// workers) leave the nest, collect food and bring it back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColonyParams {
    /// Workers represented by one forager squad.
    pub workers_per_forager: f32,
    /// Workers of a newly founded colony (scenario founders and swarms).
    pub founding_workers: f32,
    /// Largest number of workers (the queen stops laying above it).
    pub max_workers: f32,
    /// Fraction of the workers that forage.
    pub forager_fraction: f32,
    /// Largest number of forager squads of one colony (performance cap).
    pub max_foragers: u16,
    /// Distance (m) from the nest within which squads forage.
    pub foraging_range_m: f32,
    /// Fraction of a tick a squad spends travelling (the rest is spent
    /// handling food); with the species' cruising speed it sets the squad's
    /// displacement per tick.
    pub travel_fraction: f32,
    /// Food a worker carries home per trip (kg dry matter).
    pub load_kg: f32,
    /// Worker lifespan (real days).
    pub worker_lifespan_days: f32,
    /// Development from egg to worker (real days).
    pub development_days: f32,
    /// Largest number of eggs the queen lays per (real) day.
    pub max_eggs_per_day: f32,
    /// Temperature of the nest (°C) if the colony regulates it (honey-bee
    /// hives); `None` follows the air temperature buffered by the soil
    /// (clamped to 4–30 °C).
    #[serde(default)]
    pub nest_temperature_c: Option<f32>,
    /// Foragers mark the way home with a trail pheromone that others follow
    /// (ants).
    #[serde(default)]
    pub pheromone: bool,
    /// Returning foragers report the best food patch and idle foragers are
    /// recruited to it (honey-bee dance).
    #[serde(default)]
    pub recruitment: bool,
    /// Flowers visited per worker per trip (pollinators).
    #[serde(default)]
    pub flower_visits_per_trip: f32,
    /// Workers above which a well-fed colony splits in its breeding season:
    /// half of the workers and of the store found a new colony nearby.
    pub swarm_workers: f32,
}

/// How a species is represented.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    /// Individual agents with their own physiology and decisions.
    #[default]
    Individual,
    /// A density field per cell (aggregate representation of tiny organisms).
    Field(FieldParams),
    /// Colony super-agents with forager squads (eusocial insects).
    Colony(ColonyParams),
}

impl Representation {
    /// Whether the species is simulated as individual agents.
    pub fn is_individual(&self) -> bool {
        matches!(self, Self::Individual)
    }

    /// Field parameters, if the species is a density field.
    pub fn field(&self) -> Option<&FieldParams> {
        match self {
            Self::Field(p) => Some(p),
            _ => None,
        }
    }

    /// Colony parameters, if the species lives in colonies.
    pub fn colony(&self) -> Option<&ColonyParams> {
        match self {
            Self::Colony(p) => Some(p),
            _ => None,
        }
    }
}
