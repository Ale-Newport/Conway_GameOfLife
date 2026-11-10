//! Per-cell vegetation layers: logistic growth limited by light (Beer–Lambert
//! shading by trees and taller layers), temperature (beta function), water
//! and nutrients (Liebig minimum of the two), turnover and dieback into
//! litter, seed banks with dispersal, germination and herbivory.
//!
//! For layer `i` of a cell with biomass `B` (kg/m²) and capacity `K`:
//!
//! ```text
//! I_i   = I_ground · exp(−k Σ_{taller j} LAI_j),   LAI_j = B_j · SLA_j
//! G     = r B (1 − B/K) · min(1, I_i / L_sat) / LIGHT_DUTY · f_T · min(f_W, f_N) · Δt
//! f_N   = min(1, N/(K_N + N) · (1 + boost_myco))
//! R     = seedling · germination · seed bank · f_T · f_W · Δt   (while B < K/2)
//! loss  = B (turnover + cold + drought + flood dieback) · Δt  → litter
//! ```
//!
//! Growth takes `G · n_i` grams of nutrients from the soil; when the soil
//! holds less, growth is scaled down so mass is conserved.

use physics_world::{Biome, SurfaceKind, World};
use serde::{Deserialize, Serialize};
use simulation_core::Grid2;

use crate::fungi::Detritus;
use crate::ledger::{Flux, Ledgers};
use crate::species::SpeciesCatalog;
use crate::species::plants::{LAYER_COUNT, PlantLayer};
use crate::units::{LIGHT_DUTY, LIGHT_EXTINCTION, PLANT_KJ_PER_KG, beta_temperature, ramp};

/// Standing water (m) above which a land cell counts as flooded for land
/// plants and as habitat for aquatic layers of wetlands.
pub const FLOOD_DEPTH_M: f32 = 0.3;
/// Daily decay of seed-bank presence without new seed production.
const SEED_BANK_DECAY_PER_DAY: f32 = 0.02;
/// Daily decay of the pollination index.
const POLLINATION_DECAY_PER_DAY: f32 = 0.5;
/// Flowers per kg of flower-layer biomass (dry; flower heads of ≈ 0.2 g).
pub const FLOWERS_PER_KG: f64 = 5_000.0;
/// Nectar (kg dry) collected per flower visit.
pub const NECTAR_PER_VISIT_KG: f64 = 5e-7;
/// Seeds per kg of flower biomass per day at full pollination (designer
/// order of magnitude); the seed-set share without pollinators (wind and
/// self pollination) is [`SELF_POLLINATION`].
pub const FLOWER_SEEDS_PER_KG_DAY: f64 = 2_000.0;
/// Seed set of unvisited flowers relative to fully pollinated ones.
pub const SELF_POLLINATION: f32 = 0.3;

/// Vegetation biomass, seed banks and pollination of every cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vegetation {
    width: usize,
    height: usize,
    biomass: Vec<[f32; LAYER_COUNT]>,
    seeds: Vec<[f32; LAYER_COUNT]>,
    pollination: Vec<f32>,
    /// Flower seeds produced during the last daily update (whole map).
    flower_seeds_last_day: f64,
    /// Flower seeds produced since the start.
    flower_seeds_total: f64,
}

/// Inputs of one growth step.
pub(crate) struct GrowthStep<'a> {
    pub catalog: &'a SpeciesCatalog,
    /// Nutrient-uptake multiplier from mycorrhiza per cell (≥ 1).
    pub myco_boost: &'a [f32],
    /// Time step (days).
    pub dt_days: f32,
    /// Global growth multiplier (parameter).
    pub growth_multiplier: f32,
    /// Area of one cell (m²).
    pub cell_area: f64,
}

impl Vegetation {
    /// Empty vegetation.
    pub fn new(width: usize, height: usize) -> Self {
        let n = width * height;
        Self {
            width,
            height,
            biomass: vec![[0.0; LAYER_COUNT]; n],
            seeds: vec![[0.0; LAYER_COUNT]; n],
            pollination: vec![0.0; n],
            flower_seeds_last_day: 0.0,
            flower_seeds_total: 0.0,
        }
    }

    /// Width in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in cells.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Whether the buffers match the dimensions (snapshot check).
    pub(crate) fn is_consistent(&self, width: usize, height: usize) -> bool {
        let n = width * height;
        self.width == width
            && self.height == height
            && self.biomass.len() == n
            && self.seeds.len() == n
            && self.pollination.len() == n
            && self.biomass.iter().flatten().chain(self.seeds.iter().flatten()).all(|v| v.is_finite() && *v >= 0.0)
            && self.pollination.iter().all(|p| (0.0..=1.0).contains(p))
            && self.flower_seeds_total.is_finite()
    }

    /// Biomass (kg/m²) of a layer in a cell.
    pub fn biomass(&self, cell: usize, layer: PlantLayer) -> f32 {
        self.biomass[cell][layer.index()]
    }

    /// All layers of a cell (kg/m², [`PlantLayer`] order).
    pub fn cell(&self, cell: usize) -> &[f32; LAYER_COUNT] {
        &self.biomass[cell]
    }

    /// Seed-bank presence of a layer in a cell (`[0, 1]`).
    pub fn seed_bank(&self, cell: usize, layer: PlantLayer) -> f32 {
        self.seeds[cell][layer.index()]
    }

    /// Pollination index of a cell (`[0, 1]`).
    pub fn pollination(&self, cell: usize) -> f32 {
        self.pollination[cell]
    }

    /// Full-resolution grid of one layer.
    pub fn layer_grid(&self, layer: PlantLayer) -> Grid2<f32> {
        Grid2::from_vec(self.width, self.height, self.biomass.iter().map(|b| b[layer.index()]).collect())
    }

    /// Sets the biomass of a layer (initialisation and NetLogo grass display).
    pub(crate) fn set(&mut self, cell: usize, layer: PlantLayer, kg_m2: f32) {
        self.biomass[cell][layer.index()] = kg_m2.max(0.0);
    }

    /// Sets the seed-bank presence of a layer.
    pub(crate) fn set_seed_bank(&mut self, cell: usize, layer: PlantLayer, presence: f32) {
        self.seeds[cell][layer.index()] = presence.clamp(0.0, 1.0);
    }

    /// Removes up to `kg_m2` of a layer and returns the amount removed.
    pub(crate) fn remove(&mut self, cell: usize, layer: PlantLayer, kg_m2: f32) -> f32 {
        let slot = &mut self.biomass[cell][layer.index()];
        let taken = kg_m2.max(0.0).min(*slot);
        *slot -= taken;
        taken
    }

    /// Records `visits` of pollinators to the flowers of a cell: the
    /// pollination index (fraction of flowers visited recently, decaying by
    /// half per day) rises by `visits / flowers` (capped at 1).
    pub(crate) fn visit_flowers(&mut self, cell: usize, visits: f64, cell_area: f64) {
        let flowers = FLOWERS_PER_KG * f64::from(self.biomass[cell][PlantLayer::Flowers.index()]) * cell_area;
        if flowers > 0.0 && visits > 0.0 {
            let p = f64::from(self.pollination[cell]) + visits / flowers;
            self.pollination[cell] = p.min(1.0) as f32;
        }
    }

    /// Pollination index grid.
    pub fn pollination_grid(&self) -> Grid2<f32> {
        Grid2::from_vec(self.width, self.height, self.pollination.clone())
    }

    /// Mean pollination index of the flowers (weighted by flower biomass).
    pub fn flower_pollination(&self) -> f64 {
        let fi = PlantLayer::Flowers.index();
        let (mut sum, mut weight) = (0.0, 0.0);
        for (b, p) in self.biomass.iter().zip(&self.pollination) {
            sum += f64::from(b[fi]) * f64::from(*p);
            weight += f64::from(b[fi]);
        }
        if weight > 0.0 { sum / weight } else { 0.0 }
    }

    /// Flower seeds produced on the last simulated day (whole map).
    pub fn flower_seed_production(&self) -> f64 {
        self.flower_seeds_last_day
    }

    /// Flower seeds produced since the start of the run.
    pub fn flower_seeds_total(&self) -> f64 {
        self.flower_seeds_total
    }

    /// Total biomass of a layer over the map (kg) for a cell area (m²).
    pub fn total_kg(&self, layer: PlantLayer, cell_area: f64) -> f64 {
        self.biomass.iter().map(|b| f64::from(b[layer.index()])).sum::<f64>() * cell_area
    }

    /// Total biomass of all layers (kg).
    pub fn all_total_kg(&self, cell_area: f64) -> f64 {
        self.biomass.iter().flatten().map(|v| f64::from(*v)).sum::<f64>() * cell_area
    }

    /// Energy (kJ) and nutrients (g) stored in all layers.
    pub fn stock(&self, catalog: &SpeciesCatalog, cell_area: f64) -> (f64, f64) {
        let mut kg = 0.0;
        let mut g = 0.0;
        for cell in &self.biomass {
            for (layer, &b) in PlantLayer::ALL.iter().zip(cell) {
                kg += f64::from(b);
                g += f64::from(b) * f64::from(catalog.layer(*layer).nutrient_g_per_kg);
            }
        }
        (kg * cell_area * PLANT_KJ_PER_KG, g * cell_area)
    }

    /// One growth step (hourly): growth with nutrient uptake, germination,
    /// turnover and dieback into litter. Cells are processed in index order
    /// and layers from the tallest down, so uptake order is deterministic.
    pub(crate) fn grow(
        &mut self,
        step: &GrowthStep<'_>,
        world: &mut World,
        litter: &mut Detritus,
        ledgers: &mut Ledgers,
    ) {
        let mut order: Vec<PlantLayer> = PlantLayer::ALL.to_vec();
        order.sort_by(|a, b| {
            let (ha, hb) = (step.catalog.layer(*a).height_m, step.catalog.layer(*b).height_m);
            hb.total_cmp(&ha).then(a.index().cmp(&b.index()))
        });
        let w = self.width;
        for cell in 0..self.biomass.len() {
            let (x, y) = (cell % w, cell / w);
            let env = CellEnv::read(world, cell);
            let mut light = env.light;
            for &layer in &order {
                let p = step.catalog.layer(layer);
                let li = layer.index();
                let habitat = if p.aquatic { env.aquatic } else { env.terrestrial };
                let capacity = if habitat { p.capacity(env.biome) } else { 0.0 };
                let b = self.biomass[cell][li];
                let f_t = beta_temperature(env.temperature, p.temperature_c.0, p.temperature_c.1, p.temperature_c.2);
                let f_w = if p.aquatic { 1.0 } else { ramp(env.moisture, p.moisture.0, p.moisture.1) };
                // Losses first (they do not depend on the soil).
                let mut loss_rate = p.turnover_per_day;
                if env.temperature < p.temperature_c.0 {
                    loss_rate += p.cold_dieback_per_day;
                }
                if !p.aquatic && env.moisture < p.moisture.0 {
                    loss_rate += p.drought_dieback_per_day;
                }
                if !p.aquatic && env.flooded {
                    loss_rate += p.flood_dieback_per_day;
                }
                if !habitat {
                    // Plants outside their habitat (e.g. on a cell that became
                    // ocean) die back quickly.
                    loss_rate = loss_rate.max(0.2);
                }
                let loss = b * (loss_rate * step.dt_days).min(1.0);
                // Logistic growth or germination.
                let f_light = (light / p.light_saturation).min(1.0) / LIGHT_DUTY;
                let soil = world.nutrients().as_slice()[cell];
                let f_n = (soil / (p.half_saturation_g_m2 + soil)
                    * if p.mycorrhizal { step.myco_boost[cell] } else { 1.0 })
                .min(1.0);
                let limitation = f_t * f_w.min(f_n);
                let mut growth = 0.0;
                if capacity > 0.0 && b > 0.0 {
                    growth = p.growth_per_day
                        * b
                        * (1.0 - b / capacity)
                        * f_light
                        * limitation
                        * step.dt_days
                        * step.growth_multiplier;
                }
                // Recruitment from the seed bank (and surviving roots) while
                // the layer is sparse: additive, so burned or grazed-out
                // cells recover.
                if capacity > 0.0 && b < 0.5 * capacity && self.seeds[cell][li] > 0.01 {
                    growth +=
                        p.seedling_kg_m2 * p.germination_per_day * self.seeds[cell][li] * f_t * f_w * step.dt_days;
                }
                growth = growth.clamp(0.0, (capacity - b).max(0.0));
                let mut gained = 0.0;
                if growth > 0.0 {
                    let demand = growth * p.nutrient_g_per_kg;
                    let taken = take_soil(world, x, y, demand, p.aquatic, w, self.height);
                    gained = if demand > 0.0 { growth * (taken / demand).min(1.0) } else { 0.0 };
                    let area = step.cell_area;
                    ledgers.book(
                        Flux::PrimaryProduction,
                        f64::from(gained) * area * PLANT_KJ_PER_KG,
                        f64::from(taken) * area,
                    );
                }
                let after = (b - loss + gained).max(0.0);
                self.biomass[cell][li] = after;
                if loss > 0.0 {
                    litter.add_litter(cell, loss, loss * p.nutrient_g_per_kg);
                    let area = step.cell_area;
                    ledgers.book(
                        Flux::Litterfall,
                        f64::from(loss) * area * PLANT_KJ_PER_KG,
                        f64::from(loss * p.nutrient_g_per_kg) * area,
                    );
                }
                if habitat {
                    light *= (-LIGHT_EXTINCTION * after * p.leaf_area_per_kg).exp();
                }
            }
        }
    }

    /// Daily seed-bank update: production from standing biomass (flowers
    /// need pollination for most of their seed set), decay, and dispersal to
    /// the 8 neighbours (computed from the previous state).
    pub(crate) fn update_seed_banks(&mut self, catalog: &SpeciesCatalog, world: &World, days: f32, cell_area: f64) {
        let (w, h) = (self.width, self.height);
        let previous = self.seeds.clone();
        let mut flower_seeds = 0.0f64;
        for cell in 0..self.seeds.len() {
            let biome = world.biome().as_slice()[cell];
            for layer in PlantLayer::ALL {
                let p = catalog.layer(layer);
                let li = layer.index();
                let capacity = p.capacity(biome).max(1e-6);
                let mut production = (self.biomass[cell][li] / capacity).min(1.0) * p.seed_production_per_day;
                if layer == PlantLayer::Flowers {
                    let seed_set = SELF_POLLINATION + (1.0 - SELF_POLLINATION) * self.pollination[cell];
                    production *= seed_set;
                    flower_seeds += f64::from(self.biomass[cell][li])
                        * cell_area
                        * FLOWER_SEEDS_PER_KG_DAY
                        * f64::from(seed_set)
                        * f64::from(days);
                }
                let mut s = previous[cell][li] * (1.0 - SEED_BANK_DECAY_PER_DAY * days).max(0.0) + production * days;
                // Incoming seeds from neighbours.
                let (x, y) = ((cell % w) as i64, (cell / w) as i64);
                let mut incoming = 0.0;
                for (dx, dy) in NEIGHBOURS {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        incoming += previous[ny as usize * w + nx as usize][li];
                    }
                }
                s += p.dispersal_per_day * days * incoming / 8.0;
                self.seeds[cell][li] = s.clamp(0.0, 1.0);
            }
            self.pollination[cell] *= (1.0 - POLLINATION_DECAY_PER_DAY * days).max(0.0);
        }
        self.flower_seeds_last_day = flower_seeds;
        self.flower_seeds_total += flower_seeds;
    }

    /// Removes a fraction of each layer (fire, wind) and returns the removed
    /// energy (kJ) and nutrients (g) per m², per layer weighting given by `weight`.
    pub(crate) fn remove_fraction(
        &mut self,
        catalog: &SpeciesCatalog,
        cell: usize,
        fraction: f32,
        weight: impl Fn(PlantLayer) -> f32,
    ) -> (f32, f32) {
        let mut kg = 0.0;
        let mut g = 0.0;
        for layer in PlantLayer::ALL {
            let li = layer.index();
            let removed = self.biomass[cell][li] * (fraction * weight(layer)).clamp(0.0, 1.0);
            self.biomass[cell][li] -= removed;
            kg += removed;
            g += removed * catalog.layer(layer).nutrient_g_per_kg;
        }
        (kg, g)
    }
}

/// The 8-neighbourhood.
pub(crate) const NEIGHBOURS: [(i64, i64); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// Takes nutrients for plant growth: from the cell itself, and for aquatic
/// layers also from the neighbouring cells (inflow from the banks), in a
/// fixed order. Returns the amount taken (g/m² of the growing cell).
fn take_soil(world: &mut World, x: usize, y: usize, demand: f32, aquatic: bool, w: usize, h: usize) -> f32 {
    let mut taken = world.take_nutrients(x, y, demand).unwrap_or(0.0);
    if aquatic && taken < demand {
        for (dx, dy) in NEIGHBOURS {
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
            if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                continue;
            }
            taken += world.take_nutrients(nx as usize, ny as usize, demand - taken).unwrap_or(0.0);
            if taken >= demand {
                break;
            }
        }
    }
    taken
}

/// Environment of a cell for plant growth.
#[derive(Clone, Copy, Debug)]
struct CellEnv {
    temperature: f32,
    moisture: f32,
    light: f32,
    biome: Biome,
    aquatic: bool,
    terrestrial: bool,
    flooded: bool,
}

impl CellEnv {
    fn read(world: &World, cell: usize) -> Self {
        let surface = world.surface().as_slice()[cell];
        let depth = world.water_depth().as_slice()[cell];
        let land = surface == SurfaceKind::Land;
        let biome = world.biome().as_slice()[cell];
        Self {
            temperature: world.temperature().as_slice()[cell],
            moisture: world.soil_moisture().as_slice()[cell],
            light: world.light().as_slice()[cell],
            biome,
            aquatic: !land || (depth > FLOOD_DEPTH_M && biome == Biome::Wetland),
            terrestrial: land,
            flooded: land && depth > FLOOD_DEPTH_M,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use physics_world::WorldConfig;

    fn flat_world() -> World {
        let cfg = WorldConfig { width: 8, height: 8, river_count: 0, lake_tendency: 0.0, ..WorldConfig::default() };
        World::from_heightmap(&cfg, Grid2::new(8, 8, 50.0)).unwrap()
    }

    #[test]
    fn removal_never_goes_negative() {
        let mut v = Vegetation::new(2, 2);
        v.set(0, PlantLayer::Grass, 0.3);
        assert_eq!(v.remove(0, PlantLayer::Grass, 0.5), 0.3);
        assert_eq!(v.biomass(0, PlantLayer::Grass), 0.0);
    }

    #[test]
    fn growth_conserves_nutrients_and_is_logistic() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let mut world = flat_world();
        // Run the world to midday so there is light.
        for _ in 0..120 {
            world.step();
        }
        let mut v = Vegetation::new(8, 8);
        let biome = world.biome().as_slice()[0];
        let k = catalog.layer(PlantLayer::Grass).capacity(biome);
        assert!(k > 0.0, "flat default world is grassland/forest: {biome:?}");
        for c in 0..64 {
            v.set(c, PlantLayer::Grass, 0.1 * k);
        }
        let mut litter = Detritus::new(8, 8);
        let mut ledgers = Ledgers::default();
        let area = world.config().cell_area_m2();
        let boost = vec![1.0; 64];
        let soil_before = world.nutrient_storage_g();
        let (_, plant_before) = v.stock(&catalog, area);
        let step = GrowthStep {
            catalog: &catalog,
            myco_boost: &boost,
            dt_days: 1.0 / 24.0,
            growth_multiplier: 1.0,
            cell_area: area,
        };
        v.grow(&step, &mut world, &mut litter, &mut ledgers);
        let (_, plant_after) = v.stock(&catalog, area);
        let (_, litter_g) = litter.litter_stock(area);
        let total_before = soil_before + plant_before;
        let total_after = world.nutrient_storage_g() + plant_after + litter_g;
        assert!((total_after - total_before).abs() / total_before < 1e-5, "{total_before} → {total_after}");
        assert!(v.biomass(0, PlantLayer::Grass) > 0.1 * k * 0.99, "grass grows or stays at low density");
    }
}
