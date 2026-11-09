//! Per-step reports and the effects of interventions waiting to be reported.

use serde::{Deserialize, Serialize};
use simulation_core::SimEvent;

/// Fuel consumed by fire (or lava) in one cell during one tick.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FuelBurn {
    /// Column.
    pub x: u32,
    /// Row.
    pub y: u32,
    /// Fuel consumed, kg/m² (the ecosystem removes this much biomass and may
    /// return ash nutrients with [`World::add_nutrients`](crate::World::add_nutrients)).
    pub amount: f32,
}

/// Wind damage of a cell (tornado), in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellDamage {
    /// Column.
    pub x: u32,
    /// Row.
    pub y: u32,
    /// Fraction of standing vegetation destroyed.
    pub damage: f32,
}

/// Summary of one [`World::step`](crate::World::step) (plus effects of interventions applied
/// since the previous step).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentReport {
    /// Tick that was simulated.
    pub tick: u64,
    /// Notable events (weather changes, fires, floods, disasters...).
    pub events: Vec<SimEvent>,
    /// Mean precipitation depth over non-ocean cells this tick (mm).
    pub rain_total_mm: f64,
    /// Precipitation volume on non-ocean cells this tick (m³).
    pub rain_volume_m3: f64,
    /// Evaporated volume this tick (m³).
    pub evaporated_m3: f64,
    /// Volume that left to the ocean or across the border this tick (m³).
    pub outflow_m3: f64,
    /// Dry-land cells under more than the flood threshold.
    pub flooded_cells: u32,
    /// Cells burning (or lava) at the end of the tick.
    pub burning_cells: u32,
    /// Cells that finished burning this tick.
    pub burned_cells: u32,
    /// New ignitions this tick (spread, lightning and interventions).
    pub ignitions: u32,
    /// Cloud-to-ground lightning strikes that hit non-ocean cells this tick
    /// (whether or not they started a fire).
    pub lightning_strikes: u32,
    /// Fuel consumed per cell, sorted by row then column, one entry per cell.
    pub fuel_consumed: Vec<FuelBurn>,
    /// Wind damage from tornadoes since the previous step.
    pub wind_damage: Vec<CellDamage>,
    /// Number of landslides this tick.
    pub landslides: u32,
    /// Whether terrain (elevation, surface, biome) changed; renderers should
    /// refetch static layers ([`World::terrain_version`](crate::World::terrain_version) was bumped).
    pub terrain_changed: bool,
}

impl EnvironmentReport {
    /// Merges duplicate cells of `fuel_consumed` and sorts it.
    pub(crate) fn normalize_fuel(&mut self) {
        self.fuel_consumed.sort_by_key(|b| (b.y, b.x));
        let mut merged: Vec<FuelBurn> = Vec::with_capacity(self.fuel_consumed.len());
        for burn in self.fuel_consumed.drain(..) {
            match merged.last_mut() {
                Some(last) if last.x == burn.x && last.y == burn.y => last.amount += burn.amount,
                _ => merged.push(burn),
            }
        }
        self.fuel_consumed = merged;
    }
}

/// Effects of interventions waiting to be reported by the next step.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Pending {
    pub events: Vec<SimEvent>,
    pub wind_damage: Vec<CellDamage>,
    pub fuel_consumed: Vec<FuelBurn>,
    pub ignitions: u32,
    pub landslides: u32,
    pub terrain_changed: bool,
}
