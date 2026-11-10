//! Dead organic matter (litter, dung, dead wood) and fungi.
//!
//! Fungi are **not plants**: saprotrophic mycelium grows only by
//! decomposing litter and dead wood, mushrooms fruit from the mycelium when
//! the soil is moist and mild, and mycorrhizal fungi live on plant carbon
//! and raise the nutrient uptake of their hosts. Per cell and hour:
//!
//! ```text
//! sat    = F / (F + K_F)                                  (fungal saturation)
//! f      = Q10^{(T − 20)/10} · hump(moisture) · decomposition multiplier
//! dL     = L · k_L (m + (1 − m) sat) f Δt                  (litter; m = microbial share;
//!                                                            in water L · k_aq · Q10 Δt, bacterial)
//! dW     = W · k_W sat f Δt                                (dead wood, white rot)
//! ΔF     = yield · (fungal share of dL + dW)               (mycelium growth)
//! humus  = h · (dL + dW)                                   → soil organic matter (h = 0 at sea:
//!                                                            marine detritus is remineralised)
//! N_free = nutrients(dL + dW) · (1 − h) − ΔF · n_F         → mineralised (> 0) or immobilised (< 0)
//! ```
//!
//! The rest of the decomposed energy is respired. Fungal mortality and
//! mushroom decay return mass to the litter; mycelium spreads to the four
//! neighbours by a conservative exchange.

use physics_world::{SurfaceKind, World};
use serde::{Deserialize, Serialize};
use simulation_core::Grid2;

use crate::ledger::{Flux, Ledgers};
use crate::species::plants::FungiParams;
use crate::units::{DETRITUS_KJ_PER_KG, FUNGI_KJ_PER_KG, moisture_hump, q10_factor};

/// Litter (incl. dung and animal remains, as dry-matter equivalents) and
/// dead wood of every cell, with their nutrient contents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Detritus {
    width: usize,
    height: usize,
    litter_kg: Vec<f32>,
    litter_g: Vec<f32>,
    wood_kg: Vec<f32>,
    wood_g: Vec<f32>,
}

impl Detritus {
    /// Empty pools.
    pub fn new(width: usize, height: usize) -> Self {
        let n = width * height;
        Self {
            width,
            height,
            litter_kg: vec![0.0; n],
            litter_g: vec![0.0; n],
            wood_kg: vec![0.0; n],
            wood_g: vec![0.0; n],
        }
    }

    /// Whether the buffers match the dimensions (snapshot check).
    pub(crate) fn is_consistent(&self, width: usize, height: usize) -> bool {
        let n = width * height;
        self.width == width
            && self.height == height
            && [&self.litter_kg, &self.litter_g, &self.wood_kg, &self.wood_g]
                .iter()
                .all(|v| v.len() == n && v.iter().all(|x| x.is_finite() && *x >= 0.0))
    }

    /// Adds litter (kg/m² dry-matter equivalent and g/m² nutrients).
    pub(crate) fn add_litter(&mut self, cell: usize, kg_m2: f32, g_m2: f32) {
        self.litter_kg[cell] += kg_m2.max(0.0);
        self.litter_g[cell] += g_m2.max(0.0);
    }

    /// Adds litter given as energy (kJ/m²): animal remains and dung keep
    /// their energy exactly; their mass is its dry-matter equivalent.
    pub(crate) fn add_litter_energy(&mut self, cell: usize, kj_m2: f64, g_m2: f32) {
        self.add_litter(cell, (kj_m2 / DETRITUS_KJ_PER_KG) as f32, g_m2);
    }

    /// Adds dead wood (kg/m², g/m²).
    pub(crate) fn add_wood(&mut self, cell: usize, kg_m2: f32, g_m2: f32) {
        self.wood_kg[cell] += kg_m2.max(0.0);
        self.wood_g[cell] += g_m2.max(0.0);
    }

    /// Litter mass of a cell (kg/m²).
    pub fn litter(&self, cell: usize) -> f32 {
        self.litter_kg[cell]
    }

    /// Dead-wood mass of a cell (kg/m²).
    pub fn wood(&self, cell: usize) -> f32 {
        self.wood_kg[cell]
    }

    /// Removes up to `kg_m2` of litter with its share of nutrients; returns `(kg, g)`.
    pub(crate) fn take_litter(&mut self, cell: usize, kg_m2: f32) -> (f32, f32) {
        take_share(&mut self.litter_kg[cell], &mut self.litter_g[cell], kg_m2)
    }

    /// Removes a fraction of litter and of dead wood (fire); returns `(kg, g)`
    /// of litter and of wood.
    pub(crate) fn burn(&mut self, cell: usize, litter_fraction: f32, wood_fraction: f32) -> ((f32, f32), (f32, f32)) {
        let l = self.litter_kg[cell] * litter_fraction.clamp(0.0, 1.0);
        let w = self.wood_kg[cell] * wood_fraction.clamp(0.0, 1.0);
        (
            take_share(&mut self.litter_kg[cell], &mut self.litter_g[cell], l),
            take_share(&mut self.wood_kg[cell], &mut self.wood_g[cell], w),
        )
    }

    /// Energy (kJ) and nutrients (g) of the litter pool.
    pub fn litter_stock(&self, cell_area: f64) -> (f64, f64) {
        stock(&self.litter_kg, &self.litter_g, cell_area, DETRITUS_KJ_PER_KG)
    }

    /// Energy (kJ) and nutrients (g) of the dead-wood pool.
    pub fn wood_stock(&self, cell_area: f64) -> (f64, f64) {
        stock(&self.wood_kg, &self.wood_g, cell_area, DETRITUS_KJ_PER_KG)
    }

    /// Litter grid (kg/m²).
    pub fn litter_grid(&self) -> Grid2<f32> {
        Grid2::from_vec(self.width, self.height, self.litter_kg.clone())
    }

    /// Dead-wood grid (kg/m²).
    pub fn wood_grid(&self) -> Grid2<f32> {
        Grid2::from_vec(self.width, self.height, self.wood_kg.clone())
    }
}

fn take_share(kg: &mut f32, g: &mut f32, amount: f32) -> (f32, f32) {
    let taken = amount.max(0.0).min(*kg);
    if taken <= 0.0 {
        return (0.0, 0.0);
    }
    let share = if *kg > 0.0 { taken / *kg } else { 0.0 };
    let nutrients = if taken >= *kg { *g } else { *g * share };
    *kg -= taken;
    *g -= nutrients;
    if *kg <= 0.0 {
        *kg = 0.0;
    }
    *g = g.max(0.0);
    (taken, nutrients)
}

fn stock(kg: &[f32], g: &[f32], area: f64, kj_per_kg: f64) -> (f64, f64) {
    let m: f64 = kg.iter().map(|v| f64::from(*v)).sum();
    let n: f64 = g.iter().map(|v| f64::from(*v)).sum();
    (m * area * kj_per_kg, n * area)
}

/// Fungal biomass of every cell (kg dry matter / m²).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fungi {
    width: usize,
    height: usize,
    mycelium: Vec<f32>,
    mycorrhiza: Vec<f32>,
    mushrooms: Vec<f32>,
}

/// Inputs of one decomposition step.
pub(crate) struct DecayStep<'a> {
    pub params: &'a FungiParams,
    /// Time step (days).
    pub dt_days: f32,
    /// Decomposition multiplier (parameter).
    pub multiplier: f32,
    /// Area of one cell (m²).
    pub cell_area: f64,
    /// Mycorrhizal host foliage per cell (kg/m²: shrubs + tree leaves).
    pub host_kg_m2: &'a [f32],
}

impl Fungi {
    /// Empty pools.
    pub fn new(width: usize, height: usize) -> Self {
        let n = width * height;
        Self { width, height, mycelium: vec![0.0; n], mycorrhiza: vec![0.0; n], mushrooms: vec![0.0; n] }
    }

    /// Whether the buffers match the dimensions (snapshot check).
    pub(crate) fn is_consistent(&self, width: usize, height: usize) -> bool {
        let n = width * height;
        self.width == width
            && self.height == height
            && [&self.mycelium, &self.mycorrhiza, &self.mushrooms]
                .iter()
                .all(|v| v.len() == n && v.iter().all(|x| x.is_finite() && *x >= 0.0))
    }

    /// Saprotrophic mycelium (kg/m²).
    pub fn mycelium(&self, cell: usize) -> f32 {
        self.mycelium[cell]
    }

    /// Mycorrhizal biomass (kg/m²).
    pub fn mycorrhiza(&self, cell: usize) -> f32 {
        self.mycorrhiza[cell]
    }

    /// Mushrooms (kg/m²).
    pub fn mushrooms(&self, cell: usize) -> f32 {
        self.mushrooms[cell]
    }

    /// Mushroom grid (kg/m²).
    pub fn mushroom_grid(&self) -> Grid2<f32> {
        Grid2::from_vec(self.width, self.height, self.mushrooms.clone())
    }

    /// Mycelium grid (kg/m²).
    pub fn mycelium_grid(&self) -> Grid2<f32> {
        Grid2::from_vec(self.width, self.height, self.mycelium.clone())
    }

    /// Sets initial pools of a cell.
    pub(crate) fn seed(&mut self, cell: usize, mycelium: f32, mycorrhiza: f32) {
        self.mycelium[cell] = mycelium.max(0.0);
        self.mycorrhiza[cell] = mycorrhiza.max(0.0);
    }

    /// Removes up to `kg_m2` of mushrooms; returns the amount removed.
    pub(crate) fn take_mushrooms(&mut self, cell: usize, kg_m2: f32) -> f32 {
        let taken = kg_m2.max(0.0).min(self.mushrooms[cell]);
        self.mushrooms[cell] -= taken;
        taken
    }

    /// Nutrient-uptake multiplier of mycorrhizal plants in a cell:
    /// `1 + boost · M / (M + K_M)`.
    pub fn uptake_boost(&self, params: &FungiParams, cell: usize, multiplier: f32) -> f32 {
        let m = self.mycorrhiza[cell];
        let p = &params.mycorrhiza;
        1.0 + p.uptake_boost * multiplier * m / (m + p.half_saturation_kg_m2)
    }

    /// Energy (kJ) and nutrients (g) of all fungal pools.
    pub fn stock(&self, params: &FungiParams, cell_area: f64) -> (f64, f64) {
        let kg: f64 = self.mycelium.iter().chain(&self.mycorrhiza).chain(&self.mushrooms).map(|v| f64::from(*v)).sum();
        (kg * cell_area * FUNGI_KJ_PER_KG, kg * cell_area * f64::from(params.saprotroph.nutrient_g_per_kg))
    }

    /// Hourly decomposition, fungal growth and death, fruiting and
    /// mycorrhizal dynamics (see the module documentation).
    pub(crate) fn decompose(
        &mut self,
        step: &DecayStep<'_>,
        world: &mut World,
        detritus: &mut Detritus,
        ledgers: &mut Ledgers,
    ) {
        let s = &step.params.saprotroph;
        let fr = &step.params.fruiting;
        let my = &step.params.mycorrhiza;
        let n_f = s.nutrient_g_per_kg;
        let area = step.cell_area;
        let dt = step.dt_days;
        let w = self.width;
        for cell in 0..self.mycelium.len() {
            let land = world.surface().as_slice()[cell] == SurfaceKind::Land;
            let t = world.temperature().as_slice()[cell];
            let moisture = if land { world.soil_moisture().as_slice()[cell] } else { 1.0 };
            let q10 = q10_factor(t, s.q10, 20.0) * step.multiplier;
            let f = q10 * moisture_hump(moisture, s.moisture_optimum);
            let fungi = self.mycelium[cell];
            let sat = fungi / (fungi + s.half_saturation_kg_m2);
            // In water, bacteria decompose labile detritus without fungi.
            let k_litter = if land {
                s.litter_decay_per_day * (s.microbial_fraction + (1.0 - s.microbial_fraction) * sat) * f
            } else {
                s.aquatic_decay_per_day * q10
            };
            let k_wood = s.wood_decay_per_day * sat * f;
            let (dl, dl_g) = detritus.take_litter(cell, detritus.litter_kg[cell] * (k_litter * dt).min(1.0));
            let wood = detritus.wood_kg[cell] * (k_wood * dt).min(1.0);
            let (dw, dw_g) = take_share(&mut detritus.wood_kg[cell], &mut detritus.wood_g[cell], wood);
            let decomposed = dl + dw;
            if decomposed > 0.0 {
                let fungal_share_litter = if k_litter > 0.0 {
                    (1.0 - s.microbial_fraction) * sat / (s.microbial_fraction + (1.0 - s.microbial_fraction) * sat)
                } else {
                    0.0
                };
                // No soil forms at sea, and the world does not mineralise
                // organic matter in ocean cells: marine detritus is
                // remineralised in the water column instead of humified.
                let ocean = world.surface().as_slice()[cell] == SurfaceKind::Ocean;
                let humification = if ocean { 0.0 } else { s.humification };
                let humus_kg = humification * decomposed;
                let humus_g = humification * (dl_g + dw_g);
                let free_g = dl_g + dw_g - humus_g;
                let mut growth = if land { s.growth_yield * (fungal_share_litter * dl + dw) } else { 0.0 };
                let need = growth * n_f;
                let (x, y) = (cell % w, cell / w);
                if need <= free_g {
                    let surplus = free_g - need;
                    add_soil(world, x, y, surplus);
                    ledgers.book(Flux::Mineralisation, 0.0, f64::from(surplus) * area);
                } else {
                    let taken = world.take_nutrients(x, y, need - free_g).unwrap_or(0.0);
                    ledgers.book(Flux::Immobilisation, 0.0, f64::from(taken) * area);
                    growth = (free_g + taken) / n_f;
                }
                if humus_g > 0.0 {
                    add_organic(world, x, y, humus_g);
                }
                self.mycelium[cell] += growth;
                let respired = (decomposed - growth - humus_kg).max(0.0);
                ledgers.book(
                    Flux::FungalGrowth,
                    f64::from(growth) * area * FUNGI_KJ_PER_KG,
                    f64::from(growth * n_f) * area,
                );
                ledgers.book(
                    Flux::Humification,
                    f64::from(humus_kg) * area * DETRITUS_KJ_PER_KG,
                    f64::from(humus_g) * area,
                );
                ledgers.book(Flux::DecomposerRespiration, f64::from(respired) * area * DETRITUS_KJ_PER_KG, 0.0);
            }
            // Mortality of mycelium and mycorrhiza, decay of mushrooms → litter.
            let dead = self.mycelium[cell] * (s.mortality_per_day * dt).min(1.0)
                + self.mycorrhiza[cell] * (my.mortality_per_day * dt).min(1.0);
            let rotten = self.mushrooms[cell] * (fr.decay_per_day * dt).min(1.0);
            self.mycelium[cell] -= self.mycelium[cell] * (s.mortality_per_day * dt).min(1.0);
            self.mycorrhiza[cell] -= self.mycorrhiza[cell] * (my.mortality_per_day * dt).min(1.0);
            self.mushrooms[cell] -= rotten;
            let returned = dead + rotten;
            if returned > 0.0 {
                detritus.add_litter_energy(cell, f64::from(returned) * FUNGI_KJ_PER_KG, returned * n_f);
                ledgers.book(
                    Flux::FungalMortality,
                    f64::from(returned) * area * FUNGI_KJ_PER_KG,
                    f64::from(returned * n_f) * area,
                );
            }
            // Fruiting.
            if land
                && self.mycelium[cell] > fr.min_mycelium_kg_m2
                && moisture >= fr.min_moisture
                && t > fr.temperature_c.0
                && t < fr.temperature_c.1
            {
                let fruit = self.mycelium[cell] * (fr.rate_per_day * dt).min(1.0);
                self.mycelium[cell] -= fruit;
                self.mushrooms[cell] += fruit;
                ledgers.book(Flux::Fruiting, f64::from(fruit) * area * FUNGI_KJ_PER_KG, f64::from(fruit * n_f) * area);
            }
            // Mycorrhiza: logistic growth on host carbon, nutrients from the soil.
            let host = step.host_kg_m2[cell];
            let capacity = my.capacity_per_host * host;
            if land && capacity > 0.0 {
                let m = self.mycorrhiza[cell].max(capacity * 1e-3);
                let want = (my.growth_per_day * m * (1.0 - m / capacity) * dt).max(0.0);
                if want > 0.0 {
                    let (x, y) = (cell % w, cell / w);
                    let taken = world.take_nutrients(x, y, want * n_f).unwrap_or(0.0);
                    let grown = taken / n_f;
                    self.mycorrhiza[cell] += grown;
                    ledgers.book(
                        Flux::MycorrhizalCarbon,
                        f64::from(grown) * area * FUNGI_KJ_PER_KG,
                        f64::from(taken) * area,
                    );
                }
            }
        }
    }

    /// Daily spread of mycelium to the four land neighbours: each pair of
    /// land cells exchanges `spread · Δt · (F_a − F_b) / 4`, computed from the
    /// previous state, so total mycelium is conserved.
    pub(crate) fn spread(&mut self, params: &FungiParams, world: &World, days: f32) {
        let (w, h) = (self.width, self.height);
        let rate = (params.saprotroph.spread_per_day * days).min(0.2) / 4.0;
        let previous = self.mycelium.clone();
        let land = |i: usize| world.surface().as_slice()[i] == SurfaceKind::Land;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if !land(i) {
                    continue;
                }
                let mut delta = 0.0;
                for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if land(j) {
                        delta += rate * (previous[j] - previous[i]);
                    }
                }
                self.mycelium[i] = (previous[i] + delta).max(0.0);
            }
        }
    }
}

fn add_soil(world: &mut World, x: usize, y: usize, g_m2: f32) {
    if g_m2 > 0.0 {
        // Amounts are far below the per-call limit; a failure would mean a
        // coordinate bug, which cannot happen for cells of the world itself.
        let _ = world.add_nutrients(x, y, g_m2);
    }
}

fn add_organic(world: &mut World, x: usize, y: usize, g_m2: f32) {
    if g_m2 > 0.0 {
        let _ = world.add_organic_matter(x, y, g_m2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::SpeciesCatalog;
    use physics_world::WorldConfig;

    fn world() -> World {
        // Warm enough for decomposition at night in early spring.
        let cfg = WorldConfig {
            width: 8,
            height: 8,
            river_count: 0,
            lake_tendency: 0.0,
            base_temperature_c: 25.0,
            ..WorldConfig::default()
        };
        World::from_heightmap(&cfg, Grid2::new(8, 8, 50.0)).unwrap()
    }

    #[test]
    fn decomposition_conserves_nutrients_and_fungi_speed_it_up() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let params = catalog.fungi();
        let mut w = world();
        let area = w.config().cell_area_m2();
        let mut detritus = Detritus::new(8, 8);
        let mut fungi = Fungi::new(8, 8);
        for c in 0..64 {
            detritus.add_litter(c, 0.5, 5.0);
            detritus.add_wood(c, 1.0, 1.5);
        }
        // Left half has fungi, right half none.
        for c in 0..64 {
            if c % 8 < 4 {
                fungi.seed(c, 0.05, 0.0);
            }
        }
        let host = vec![0.0; 64];
        let mut ledgers = Ledgers::default();
        let total = |w: &World, d: &Detritus, f: &Fungi| {
            w.nutrient_storage_g() + d.litter_stock(area).1 + d.wood_stock(area).1 + f.stock(params, area).1
        };
        let before = total(&w, &detritus, &fungi);
        let step = DecayStep { params, dt_days: 1.0, multiplier: 1.0, cell_area: area, host_kg_m2: &host };
        fungi.decompose(&step, &mut w, &mut detritus, &mut ledgers);
        assert!(detritus.litter(0) < detritus.litter(7), "fungi accelerate litter decay");
        for _ in 0..29 {
            fungi.decompose(&step, &mut w, &mut detritus, &mut ledgers);
        }
        let after = total(&w, &detritus, &fungi);
        assert!((after - before).abs() / before < 1e-5, "{before} → {after}");
        // Dead mycelium returns to the litter, but dead organic matter as a
        // whole disappears faster where fungi live.
        let organic = |c: usize| detritus.litter(c) + detritus.wood(c) + fungi.mycelium(c);
        assert!(organic(0) < organic(7), "{} vs {}", organic(0), organic(7));
        assert!(detritus.wood(0) < 1.0 && (detritus.wood(7) - 1.0).abs() < 1e-6, "wood decays only with fungi");
        assert!(ledgers.nutrients.total(Flux::Mineralisation) > 0.0);
    }

    #[test]
    fn spread_conserves_mycelium() {
        let w = world();
        let catalog = SpeciesCatalog::builtin().unwrap();
        let mut fungi = Fungi::new(8, 8);
        fungi.seed(27, 1.0, 0.0);
        let before: f32 = fungi.mycelium.iter().sum();
        for _ in 0..10 {
            fungi.spread(catalog.fungi(), &w, 1.0);
        }
        let after: f32 = fungi.mycelium.iter().sum();
        assert!((after - before).abs() < 1e-5);
        assert!(fungi.mycelium(28) > 0.0 && fungi.mycelium(27) < 1.0);
    }
}
