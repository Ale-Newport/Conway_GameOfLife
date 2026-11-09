//! Hydrology: per-cell water column (precipitation, snow, melt, evaporation,
//! infiltration, percolation, nutrient mineralisation and leaching), a
//! lumped groundwater reservoir feeding river baseflow, and surface-water
//! routing by water-surface equalisation over the 8-neighbourhood.
//!
//! Every transfer is computed from read-only state into separate buffers
//! (double buffering) and accounted in a [`WaterBudget`], so that
//! `storage − initial = inputs − evaporation − outflow` holds up to f32
//! round-off.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::biome::{SoilProperties, SurfaceKind};
use crate::cells::{STEP_LEN, is_border, neighbor, opposite};
use crate::world::World;

/// Degree-day snow-melt factor: 4 mm water equivalent per °C per day.
const MELT_M_PER_DEGC_H: f32 = 0.004 / 24.0;
/// Potential evaporation coefficient (m per hour per °C above −5 °C at zero
/// humidity and full sun).
const PET_M_PER_DEGC_H: f32 = 3.3e-5;
/// Time constant of the groundwater reservoir (hours): 40 days.
pub(crate) const GROUNDWATER_TIME_CONSTANT_H: f64 = 40.0 * 24.0;
/// Fraction of the equalising volume moved per routing sub-step.
const FLOW_FRACTION: f64 = 0.8;
/// Water depth below which a cell does not route (m).
const MIN_ROUTED_DEPTH: f64 = 1e-6;
/// Mineralisation rate of organic matter at 20 °C and optimal moisture (per hour).
const MINERALISATION_PER_H: f32 = 5e-4;
/// Fraction of mineral nutrients carried away per unit of relative
/// percolation (percolated depth / soil capacity).
const LEACHING_COEFFICIENT: f32 = 0.5;
/// Time constant of the drought precipitation average (hours): 10 days.
const DROUGHT_EMA_H: f64 = 240.0;

/// Cumulative water accounting since generation (all volumes in m³; ocean
/// cells are an infinite sink and excluded).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WaterBudget {
    /// Storage (surface + soil + snow + groundwater) at generation.
    pub initial_storage_m3: f64,
    /// Precipitation (rain and snow) that fell on non-ocean cells.
    pub precipitation_m3: f64,
    /// Water added by interventions: extra rain, floods, and sea water cut
    /// off from the ocean by uplift (it stays behind as a lake).
    pub added_m3: f64,
    /// Water evaporated from open water and soil (and boiled by lava).
    pub evaporated_m3: f64,
    /// Water that flowed into the ocean (including land that sank into it).
    pub outflow_ocean_m3: f64,
    /// Water that left the map across its border.
    pub outflow_edge_m3: f64,
}

impl WaterBudget {
    /// Total inputs.
    pub fn inputs_m3(&self) -> f64 {
        self.precipitation_m3 + self.added_m3
    }

    /// Total outputs.
    pub fn outputs_m3(&self) -> f64 {
        self.evaporated_m3 + self.outflow_ocean_m3 + self.outflow_edge_m3
    }

    /// Storage implied by the budget: initial + inputs − outputs.
    pub fn expected_storage_m3(&self) -> f64 {
        self.initial_storage_m3 + self.inputs_m3() - self.outputs_m3()
    }
}

/// Water currently stored in the world (m³), excluding the ocean.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WaterStorage {
    /// Surface water on land, lakes and rivers.
    pub surface_m3: f64,
    /// Soil water (relative saturation × capacity).
    pub soil_m3: f64,
    /// Snow pack (water equivalent).
    pub snow_m3: f64,
    /// Lumped groundwater reservoir.
    pub groundwater_m3: f64,
}

impl WaterStorage {
    /// Sum of all stores.
    pub fn total_m3(&self) -> f64 {
        self.surface_m3 + self.soil_m3 + self.snow_m3 + self.groundwater_m3
    }
}

/// Cumulative nutrient accounting since generation (grams).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NutrientBudget {
    /// Mineral + organic nutrients at generation.
    pub initial_g: f64,
    /// Added by the ecosystem (litter, carcasses, ash) through the API.
    pub added_g: f64,
    /// Taken by the ecosystem (plant uptake) through the API.
    pub taken_g: f64,
    /// Leached to groundwater with percolating water.
    pub leached_g: f64,
    /// Deposited by disasters (volcanic ash).
    pub deposited_g: f64,
}

impl NutrientBudget {
    /// Storage implied by the budget.
    pub fn expected_g(&self) -> f64 {
        self.initial_g + self.added_g + self.deposited_g - self.taken_g - self.leached_g
    }
}

/// Running drought indicator.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DroughtState {
    /// Exponential moving average (10-day time constant) of the land-mean
    /// precipitation rate, mm/h.
    pub rain_average_mm_h: f64,
    /// Land-mean soil moisture at generation.
    pub reference_moisture: f32,
    /// Current index in `[0, 1]`.
    pub index: f32,
}

impl DroughtState {
    /// Updates the index: `0.6 × precipitation deficit + 0.4 × soil-moisture
    /// deficit`, both relative to the climatological expectation and clamped
    /// to `[0, 1]`.
    pub(crate) fn update(&mut self, rain_mm_h: f64, expected_mm_h: f64, moisture: f32, dt_hours: f64) {
        let alpha = 1.0 - (-dt_hours / DROUGHT_EMA_H).exp();
        self.rain_average_mm_h += (rain_mm_h - self.rain_average_mm_h) * alpha;
        let rain_deficit =
            if expected_mm_h > 0.0 { (1.0 - self.rain_average_mm_h / expected_mm_h).clamp(0.0, 1.0) } else { 1.0 };
        let moisture_deficit = if self.reference_moisture > 0.0 {
            (1.0 - moisture / self.reference_moisture).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.index = (0.6 * rain_deficit + 0.4 * f64::from(moisture_deficit)) as f32;
    }
}

/// State of one soil–water column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Column {
    /// Surface water depth (m).
    pub water: f64,
    /// Snow water equivalent (m).
    pub snow: f32,
    /// Relative soil saturation `[0, 1]`.
    pub moisture: f32,
    /// Mineral nutrients (g/m²).
    pub nutrients: f32,
    /// Organic matter (g/m²).
    pub organic: f32,
}

/// Drivers of one column update.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ColumnDrivers {
    /// Precipitation depth this tick (m).
    pub precipitation: f32,
    /// Air temperature (°C).
    pub temperature: f32,
    /// Potential evaporation this tick (m).
    pub potential_evaporation: f32,
    /// Tick length (h).
    pub dt_hours: f32,
    /// Soil properties of the cell.
    pub soil: SoilProperties,
    /// Permanent lake or river: its bed is connected to the groundwater
    /// (gaining reach), so only precipitation, melt and open-water
    /// evaporation apply.
    pub water_body: bool,
}

/// Fluxes of one column update (m of water, g/m² of nutrients).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ColumnFlux {
    pub precipitation: f64,
    pub evaporated: f64,
    pub percolated: f64,
    pub leached: f64,
}

/// Updates one column in place. Order: precipitation (snow when T < 0 °C),
/// degree-day melt, open-water evaporation, then — on land only —
/// infiltration (10 % of the rate on frozen ground, limited by free pore
/// space), soil evaporation and transpiration (`∝ saturation`), percolation
/// of the excess above field capacity, mineralisation
/// (`k = 5·10⁻⁴ h⁻¹ × 2^{(T−20)/10} × min(1, 2s)`) and leaching
/// (`∝ percolated / capacity`).
pub(crate) fn column_step(c: &mut Column, d: &ColumnDrivers) -> ColumnFlux {
    let capacity = d.soil.capacity_m;
    let mut flux = ColumnFlux { precipitation: f64::from(d.precipitation), ..ColumnFlux::default() };
    if d.temperature < 0.0 {
        c.snow += d.precipitation;
    } else {
        c.water += f64::from(d.precipitation);
    }
    if d.temperature > 0.0 && c.snow > 0.0 {
        let melt = c.snow.min(MELT_M_PER_DEGC_H * d.temperature * d.dt_hours);
        c.snow -= melt;
        c.water += f64::from(melt);
    }
    let open = c.water.min(f64::from(d.potential_evaporation));
    c.water -= open;
    flux.evaporated += open;
    if d.water_body {
        return flux;
    }

    let frozen = if d.temperature < 0.0 { 0.1 } else { 1.0 };
    let room = (1.0 - c.moisture).max(0.0) * capacity;
    let infiltration = c.water.min(f64::from(d.soil.infiltration_m_per_h * d.dt_hours * frozen)).min(f64::from(room));
    c.water -= infiltration;
    c.moisture += (infiltration / f64::from(capacity)) as f32;

    let remaining = (f64::from(d.potential_evaporation) - open).max(0.0) as f32;
    let soil_demand = remaining * d.soil.evaporation_factor * c.moisture;
    let soil_evaporation = soil_demand.min(c.moisture * capacity);
    c.moisture -= soil_evaporation / capacity;
    flux.evaporated += f64::from(soil_evaporation);

    let excess = (c.moisture - d.soil.field_capacity).max(0.0) * capacity;
    let percolation = excess * (d.soil.percolation_per_h * d.dt_hours).min(1.0);
    c.moisture = (c.moisture - percolation / capacity).clamp(0.0, 1.0);
    flux.percolated = f64::from(percolation);

    let rate = MINERALISATION_PER_H * 2f32.powf((d.temperature - 20.0) / 10.0) * (2.0 * c.moisture).min(1.0);
    let mineralised = c.organic * (rate * d.dt_hours).min(1.0);
    c.organic -= mineralised;
    c.nutrients += mineralised;
    let leached = c.nutrients * (LEACHING_COEFFICIENT * percolation / capacity).min(1.0);
    c.nutrients -= leached;
    flux.leached = f64::from(leached);
    flux
}

/// Outflow of one cell to its 8 neighbours (indices 0–7) and off the map
/// (index 8), by water-surface equalisation: the cell releases
/// `0.8 × min(d, ½ Δmax)` where `Δmax` is the largest drop of water
/// surface to a neighbour, split in proportion to `Δ_k / L_k`. Ocean
/// neighbours have their surface at 0 m; a border cell sees a free outfall
/// of drop `d` off the map.
fn cell_outflow(
    i: usize,
    width: usize,
    height: usize,
    elevation: &[f32],
    water: &[f64],
    surface: &[SurfaceKind],
) -> [f64; 9] {
    let mut out = [0.0f64; 9];
    let d = water[i];
    if surface[i] == SurfaceKind::Ocean || d <= MIN_ROUTED_DEPTH {
        return out;
    }
    let head = f64::from(elevation[i]) + d;
    let mut weights = [0.0f64; 9];
    let mut drop_max = 0.0f64;
    for k in 0..8 {
        if let Some(j) = neighbor(i, k, width, height) {
            let other = if surface[j] == SurfaceKind::Ocean { 0.0 } else { f64::from(elevation[j]) + water[j] };
            let drop = head - other;
            if drop > 0.0 {
                weights[k] = drop / f64::from(STEP_LEN[k]);
                drop_max = drop_max.max(drop);
            }
        }
    }
    if is_border(i, width, height) {
        weights[8] = d;
        drop_max = drop_max.max(d);
    }
    let total: f64 = weights.iter().sum();
    if drop_max <= 0.0 || total <= 0.0 {
        return out;
    }
    let released = FLOW_FRACTION * d.min(0.5 * drop_max);
    for k in 0..9 {
        out[k] = released * weights[k] / total;
    }
    out
}

/// Per-row sums of the column pass.
#[derive(Clone, Copy, Default)]
struct RowSums {
    precipitation: f64,
    evaporated: f64,
    percolated: f64,
    leached: f64,
}

impl World {
    /// Runs the column pass over every non-ocean cell. Returns the summed
    /// fluxes (m·cells and g/m²·cells).
    fn column_pass(&mut self, pet_factor: f32, dt_hours: f32) -> (f64, f64, f64, f64) {
        let w = self.config.width;
        let temperature = self.temperature.as_slice();
        let humidity = self.humidity.as_slice();
        let precipitation = self.precipitation.as_slice();
        let surface = self.surface.as_slice();
        let soil = self.soil.as_slice();
        let rows: Vec<RowSums> = self
            .water
            .as_mut_slice()
            .par_chunks_mut(w)
            .zip(self.snow.as_mut_slice().par_chunks_mut(w))
            .zip(self.soil_moisture.as_mut_slice().par_chunks_mut(w))
            .zip(self.nutrients.as_mut_slice().par_chunks_mut(w))
            .zip(self.organic.as_mut_slice().par_chunks_mut(w))
            .enumerate()
            .map(|(y, ((((water, snow), moisture), nutrients), organic))| {
                let mut sums = RowSums::default();
                for x in 0..w {
                    let i = y * w + x;
                    if surface[i] == SurfaceKind::Ocean {
                        continue;
                    }
                    let t = temperature[i];
                    let drivers = ColumnDrivers {
                        precipitation: precipitation[i] * dt_hours / 1000.0,
                        temperature: t,
                        potential_evaporation: pet_factor * (t + 5.0).max(0.0) * (1.0 - humidity[i]),
                        dt_hours,
                        soil: soil[i].properties(),
                        water_body: surface[i] != SurfaceKind::Land,
                    };
                    let mut column = Column {
                        water: water[x],
                        snow: snow[x],
                        moisture: moisture[x],
                        nutrients: nutrients[x],
                        organic: organic[x],
                    };
                    let flux = column_step(&mut column, &drivers);
                    water[x] = column.water;
                    snow[x] = column.snow;
                    moisture[x] = column.moisture;
                    nutrients[x] = column.nutrients;
                    organic[x] = column.organic;
                    sums.precipitation += flux.precipitation;
                    sums.evaporated += flux.evaporated;
                    sums.percolated += flux.percolated;
                    sums.leached += flux.leached;
                }
                sums
            })
            .collect();
        rows.iter().fold((0.0, 0.0, 0.0, 0.0), |acc, r| {
            (acc.0 + r.precipitation, acc.1 + r.evaporated, acc.2 + r.percolated, acc.3 + r.leached)
        })
    }

    /// Releases baseflow from the groundwater reservoir (linear reservoir,
    /// 40-day time constant) evenly into river cells (or lake cells when the
    /// map has no rivers). Returns the volume released (m³).
    fn baseflow(&mut self, dt_hours: f32) -> f64 {
        let area = self.config.cell_area_m2();
        let mut targets = self.cells_of(SurfaceKind::River);
        if targets.is_empty() {
            targets = self.cells_of(SurfaceKind::Lake);
        }
        if targets.is_empty() || self.groundwater_m3 <= 0.0 {
            return 0.0;
        }
        let release = self.groundwater_m3 * (1.0 - (-f64::from(dt_hours) / GROUNDWATER_TIME_CONSTANT_H).exp());
        let depth = release / (targets.len() as f64 * area);
        let water = self.water.as_mut_slice();
        for &i in &targets {
            water[i] += depth;
        }
        self.groundwater_m3 -= release;
        release
    }

    fn cells_of(&self, kind: SurfaceKind) -> Vec<usize> {
        self.surface.as_slice().iter().enumerate().filter(|(_, s)| **s == kind).map(|(i, _)| i).collect()
    }

    /// One routing sub-step. Returns `(to ocean, off map)` in m·cells.
    fn route_once(&mut self) -> (f64, f64) {
        let (w, h) = (self.config.width, self.config.height);
        let n = w * h;
        if self.scratch.flux.len() != n {
            self.scratch.flux = vec![[0.0; 9]; n];
        }
        let elevation = self.elevation.as_slice();
        let surface = self.surface.as_slice();
        let water = self.water.as_slice();
        let sums: Vec<(f64, f64)> = self
            .scratch
            .flux
            .par_chunks_mut(w)
            .enumerate()
            .map(|(y, row)| {
                let (mut to_ocean, mut off_map) = (0.0f64, 0.0f64);
                for (x, out) in row.iter_mut().enumerate() {
                    let i = y * w + x;
                    *out = cell_outflow(i, w, h, elevation, water, surface);
                    for (k, &flow) in out.iter().take(8).enumerate() {
                        if flow > 0.0
                            && let Some(j) = neighbor(i, k, w, h)
                            && surface[j] == SurfaceKind::Ocean
                        {
                            to_ocean += flow;
                        }
                    }
                    off_map += out[8];
                }
                (to_ocean, off_map)
            })
            .collect();
        let flux = &self.scratch.flux;
        self.water.as_mut_slice().par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, d) in row.iter_mut().enumerate() {
                let i = y * w + x;
                if surface[i] == SurfaceKind::Ocean {
                    continue;
                }
                let out: f64 = flux[i].iter().sum();
                let mut inflow = 0.0f64;
                for k in 0..8 {
                    if let Some(j) = neighbor(i, k, w, h) {
                        inflow += flux[j][opposite(k)];
                    }
                }
                *d = (*d - out).max(0.0) + inflow;
            }
        });
        sums.iter().fold((0.0, 0.0), |acc, s| (acc.0 + s.0, acc.1 + s.1))
    }

    /// Full hydrology update of one tick. Returns `(precipitation m³,
    /// evaporated m³, outflow m³)` for the report.
    pub(crate) fn hydrology_step(&mut self, pet_factor: f32) -> (f64, f64, f64) {
        let dt_hours = self.calendar.dt_hours();
        let area = self.config.cell_area_m2();
        let (precip, evaporated, percolated, leached) = self.column_pass(pet_factor, dt_hours);
        self.groundwater_m3 += percolated * area;
        self.water_budget.precipitation_m3 += precip * area;
        self.water_budget.evaporated_m3 += evaporated * area;
        self.nutrient_budget.leached_g += leached * area;
        self.baseflow(dt_hours);
        let substeps = self.calendar.substeps_per_tick();
        let (mut to_ocean, mut off_map) = (0.0, 0.0);
        for _ in 0..substeps {
            let (o, e) = self.route_once();
            to_ocean += o;
            off_map += e;
        }
        self.water_budget.outflow_ocean_m3 += to_ocean * area;
        self.water_budget.outflow_edge_m3 += off_map * area;
        self.sync_water_depth();
        (precip * area, evaporated * area, (to_ocean + off_map) * area)
    }

    /// Recounts flooded cells (dry land under more than the flood threshold).
    pub(crate) fn count_flooded(&self) -> u32 {
        let threshold = f64::from(self.config.flood_threshold_m);
        self.surface
            .as_slice()
            .iter()
            .zip(self.water.as_slice())
            .filter(|(s, d)| **s == SurfaceKind::Land && **d > threshold)
            .count() as u32
    }

    /// Current water storage (m³), excluding the ocean.
    pub fn water_storage(&self) -> WaterStorage {
        let area = self.config.cell_area_m2();
        let mut storage = WaterStorage { groundwater_m3: self.groundwater_m3, ..WaterStorage::default() };
        for (i, kind) in self.surface.as_slice().iter().enumerate() {
            if *kind == SurfaceKind::Ocean {
                continue;
            }
            storage.surface_m3 += self.water.as_slice()[i] * area;
            storage.snow_m3 += f64::from(self.snow.as_slice()[i]) * area;
            let capacity = self.soil.as_slice()[i].properties().capacity_m;
            storage.soil_m3 += f64::from(self.soil_moisture.as_slice()[i] * capacity) * area;
        }
        storage
    }

    /// Potential-evaporation factor of the tick, to be multiplied per cell by
    /// `max(0, T + 5) × (1 − RH)`:
    /// `PET = s × 3.3·10⁻⁵ m/h/°C × (0.3 + 0.7 daylight) × (1 + 0.08 |wind|) × m_weather × Δt`.
    pub(crate) fn potential_evaporation_factor(&self, daylight: f32) -> f32 {
        self.config.evaporation_scale
            * PET_M_PER_DEGC_H
            * (0.3 + 0.7 * daylight)
            * (1.0 + 0.08 * self.weather.wind_speed())
            * self.weather.evaporation_multiplier()
            * self.calendar.dt_hours()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::SoilType;

    fn drivers(precipitation: f32, temperature: f32, pet: f32) -> ColumnDrivers {
        ColumnDrivers {
            precipitation,
            temperature,
            potential_evaporation: pet,
            dt_hours: 1.0,
            soil: SoilType::Loam.properties(),
            water_body: false,
        }
    }

    fn total(c: &Column) -> f32 {
        c.water as f32 + c.snow + c.moisture * SoilType::Loam.properties().capacity_m
    }

    #[test]
    fn column_conserves_water() {
        let mut c = Column { water: 0.01, snow: 0.02, moisture: 0.6, nutrients: 5.0, organic: 50.0 };
        let before = total(&c);
        let flux = column_step(&mut c, &drivers(0.004, 5.0, 0.0005));
        let after = total(&c);
        let expected = before + (flux.precipitation - flux.evaporated - flux.percolated) as f32;
        assert!((after - expected).abs() < 1e-6, "{after} vs {expected}");
        assert!(c.snow < 0.02, "snow melts above 0 °C");
    }

    #[test]
    fn cold_precipitation_accumulates_as_snow_and_soil_infiltrates_slowly() {
        let mut c = Column { water: 0.0, snow: 0.0, moisture: 0.2, nutrients: 0.0, organic: 0.0 };
        column_step(&mut c, &drivers(0.005, -5.0, 0.0));
        assert!((c.snow - 0.005).abs() < 1e-7);
        assert_eq!(c.water, 0.0);
        let mut sand = Column { water: 0.02, snow: 0.0, moisture: 0.1, nutrients: 0.0, organic: 0.0 };
        let mut clay = sand;
        let mut d = drivers(0.0, 15.0, 0.0);
        d.soil = SoilType::Sand.properties();
        column_step(&mut sand, &d);
        d.soil = SoilType::Clay.properties();
        column_step(&mut clay, &d);
        assert!(sand.water < clay.water, "sand infiltrates faster than clay");
    }

    #[test]
    fn water_bodies_do_not_drain_into_their_bed() {
        let mut c = Column { water: 0.5, snow: 0.0, moisture: 1.0, nutrients: 0.0, organic: 0.0 };
        let mut d = drivers(0.001, 15.0, 0.0002);
        d.water_body = true;
        let flux = column_step(&mut c, &d);
        assert!((c.water - (0.5 + 0.001 - 0.0002)).abs() < 1e-9);
        assert_eq!(flux.percolated, 0.0);
    }

    #[test]
    fn mineralisation_moves_organic_to_mineral() {
        let mut c = Column { water: 0.0, snow: 0.0, moisture: 0.5, nutrients: 0.0, organic: 100.0 };
        let flux = column_step(&mut c, &drivers(0.0, 20.0, 0.0));
        assert!(c.nutrients > 0.0);
        assert!((f64::from(c.nutrients + c.organic) + flux.leached - 100.0).abs() < 1e-4);
    }

    #[test]
    fn outflow_never_exceeds_depth_and_goes_downhill() {
        // 3x3, centre high with water, everything else dry and lower.
        let elevation = vec![0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 1.0, 0.0];
        let water = vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.0, 0.0, 0.0, 0.0];
        let surface = vec![SurfaceKind::Land; 9];
        let out = cell_outflow(4, 3, 3, &elevation, &water, &surface);
        let sum: f64 = out.iter().sum();
        assert!(sum <= 0.3 + 1e-7 && sum > 0.0);
        assert!(out[2] < out[0], "less flow towards the higher southern neighbour");
        assert_eq!(out[8], 0.0, "interior cell has no edge outflow");
    }

    #[test]
    fn drought_index_rises_without_rain() {
        let mut d = DroughtState { rain_average_mm_h: 0.1, reference_moisture: 0.5, index: 0.0 };
        d.update(0.1, 0.1, 0.5, 1.0);
        assert!(d.index < 0.01);
        for _ in 0..2000 {
            d.update(0.0, 0.1, 0.25, 1.0);
        }
        assert!(d.index > 0.7);
    }
}
