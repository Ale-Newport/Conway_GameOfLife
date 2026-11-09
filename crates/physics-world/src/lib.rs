//! # physics-world
//!
//! The abiotic world of Emergence Lab, shared by the ecosystem, evolution
//! and civilization models: procedurally generated terrain with drainage,
//! rivers and lakes; soils, biomes and fertility; a compressed calendar with
//! seasons and day/night; weather with wind; hydrology (rain, snow,
//! infiltration, evaporation, groundwater, runoff, floods, drought); a
//! probabilistic fire-spread automaton; a nutrient pool with mass
//! accounting; and disasters as explicit interventions.
//!
//! * [`World::generate`] builds a world from a [`WorldConfig`];
//!   [`World::step`] advances one tick and returns an [`EnvironmentReport`].
//! * The ecosystem writes [`World::fuel_mut`] and [`World::canopy_mut`]
//!   every tick, removes the biomass reported in
//!   [`EnvironmentReport::fuel_consumed`], and exchanges nutrients through
//!   [`World::add_nutrients`] / [`World::take_nutrients`].
//! * Interventions go through [`World::trigger`] with a [`Disaster`].
//! * [`EnvironmentSim`] implements [`simulation_core::Simulation`] for the
//!   world alone (model id `environment.world`).
//!
//! All randomness comes from [`simulation_core::SimRng`] streams stored in
//! the world; parallel loops only compute pure per-cell functions into
//! separate buffers, and reductions are summed in row order, so runs are
//! reproducible for a given seed, platform and build.

pub mod biome;
pub mod calendar;
mod cells;
pub mod climate;
pub mod config;
mod disasters;
mod drainage;
pub mod error;
mod exchange;
pub mod fire;
pub mod hydrology;
mod integrity;
mod landscape;
mod report;
pub mod sim;
mod step;
mod terrain;
pub mod view;
pub mod weather;
mod world;

pub use biome::{Biome, SoilProperties, SoilType, SurfaceKind, whittaker};
pub use calendar::{Calendar, CalendarTime, Season};
pub use climate::StaticClimate;
pub use config::{BiomePreset, WorldConfig};
pub use disasters::{Disaster, DisasterOutcome, fault_displacement_m};
pub use drainage::{NO_RECEIVER, OFF_MAP};
pub use error::WorldError;
pub use exchange::MAX_NUTRIENT_ADDITION_G_M2;
pub use fire::FireState;
pub use hydrology::{NutrientBudget, WaterBudget, WaterStorage};
pub use report::{CellDamage, EnvironmentReport, FuelBurn};
pub use sim::{EnvironmentSim, MODEL_ID};
pub use view::{TerrainView, WeatherInfo, WorldLayer, WorldView, WorldViewRequest};
pub use weather::{ForcedWeather, Weather, WeatherKind};
pub use world::World;

/// Identifiers of the independent random streams derived from the world
/// seed with [`simulation_core::SimRng::derive`].
pub(crate) mod streams {
    /// Heightmap noise seeds and offsets.
    pub const TERRAIN: u64 = 0x5445_5252;
    /// Precipitation-pattern noise.
    pub const CLIMATE: u64 = 0x434c_494d;
    /// River source selection.
    pub const RIVERS: u64 = 0x5249_5645;
    /// Soil-texture noise.
    pub const SOIL: u64 = 0x534f_494c;
    /// Weather chain, wind and cloud noise.
    pub const WEATHER: u64 = 0x5745_4154;
    /// Fire spread and lightning.
    pub const FIRE: u64 = 0x4649_5245;
    /// Disasters and landslides.
    pub const DISASTERS: u64 = 0x4449_5341;
}
