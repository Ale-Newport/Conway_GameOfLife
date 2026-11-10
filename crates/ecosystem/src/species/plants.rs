//! Data-driven parameters of the vegetation layers, tree species and fungal
//! guilds (`assets/species/vegetation.ron`, `assets/species/trees/*.ron`,
//! `assets/species/fungi.ron`).

use physics_world::{Biome, Season};
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::error::{EcosystemError, check_id, check_range};

/// Per-cell vegetation layers (biomass fields, not individuals).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlantLayer {
    /// Grasses and sedges (main grazing resource).
    Grass,
    /// Shrubs and berry bushes.
    Shrubs,
    /// Flowering herbs (nectar for pollinators).
    Flowers,
    /// Mosses and lichens of the ground layer.
    Moss,
    /// Algae / phytoplankton of water cells.
    Algae,
    /// Rooted aquatic plants (reeds, lilies).
    Aquatic,
    /// Succulents and dryland shrubs of deserts.
    Desert,
}

/// Number of vegetation layers.
pub const LAYER_COUNT: usize = 7;

impl PlantLayer {
    /// All layers in storage order.
    pub const ALL: [PlantLayer; LAYER_COUNT] =
        [Self::Grass, Self::Shrubs, Self::Flowers, Self::Moss, Self::Algae, Self::Aquatic, Self::Desert];

    /// Storage index.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Stable machine key.
    pub fn key(self) -> &'static str {
        match self {
            Self::Grass => "grass",
            Self::Shrubs => "shrubs",
            Self::Flowers => "flowers",
            Self::Moss => "moss",
            Self::Algae => "algae",
            Self::Aquatic => "aquatic",
            Self::Desert => "desert",
        }
    }

    /// Default assimilation efficiency of herbivores eating this layer.
    pub fn default_assimilation(self) -> f32 {
        match self {
            Self::Grass => 0.5,
            Self::Shrubs => 0.4,
            Self::Flowers => 0.6,
            Self::Moss => 0.3,
            Self::Algae => 0.6,
            Self::Aquatic => 0.5,
            Self::Desert => 0.4,
        }
    }
}

/// Parameters of one vegetation layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerParams {
    /// Which layer.
    pub layer: PlantLayer,
    /// Display name.
    pub name: String,
    /// Canopy height (m); taller layers shade shorter ones.
    pub height_m: f32,
    /// Grows in water cells instead of on land.
    #[serde(default)]
    pub aquatic: bool,
    /// Maximum relative growth rate `r` (per day, light-saturated).
    pub growth_per_day: f32,
    /// Carrying capacity `K` (kg dry matter / m²) per biome; missing biomes: 0.
    pub capacity_kg_m2: Vec<(Biome, f32)>,
    /// Relative light at which growth saturates (`(0, 1]`).
    pub light_saturation: f32,
    /// Beta-function temperatures `(min, optimum, max)` (°C).
    pub temperature_c: (f32, f32, f32),
    /// Soil moisture `(wilting, optimum)` (relative saturation).
    #[serde(default = "default_moisture")]
    pub moisture: (f32, f32),
    /// Nutrient content (g per kg dry matter).
    pub nutrient_g_per_kg: f32,
    /// Monod half-saturation of soil nutrients (g/m²).
    pub half_saturation_g_m2: f32,
    /// Basal turnover to litter (per day).
    pub turnover_per_day: f32,
    /// Extra turnover below the minimum temperature (winter dieback, per day).
    #[serde(default)]
    pub cold_dieback_per_day: f32,
    /// Extra turnover below the wilting point (drought, per day).
    #[serde(default)]
    pub drought_dieback_per_day: f32,
    /// Extra turnover of land layers under standing water (per day).
    #[serde(default)]
    pub flood_dieback_per_day: f32,
    /// Leaf area per kg (m²/kg): LAI = biomass × this.
    pub leaf_area_per_kg: f32,
    /// Fraction of the biomass counted as fire fuel.
    pub flammability: f32,
    /// Germination rate from a full seed bank (per day).
    pub germination_per_day: f32,
    /// Biomass created by germination (kg/m²).
    pub seedling_kg_m2: f32,
    /// Fraction of seed-bank presence spreading to each neighbour per day.
    pub dispersal_per_day: f32,
    /// Seed-bank presence added per day by a cell at carrying capacity
    /// (flowers: at full pollination). 1 saturates the bank at once.
    #[serde(default = "default_seed_production")]
    pub seed_production_per_day: f32,
    /// Associates with mycorrhizal fungi (nutrient uptake boost).
    #[serde(default)]
    pub mycorrhizal: bool,
    /// Biomass that grazers cannot remove (kg/m²).
    #[serde(default)]
    pub residual_kg_m2: f32,
    /// Model ids for instanced rendering (`assets/models/manifest.toml`).
    #[serde(default)]
    pub models: Vec<String>,
}

fn default_moisture() -> (f32, f32) {
    (0.1, 0.4)
}

fn default_seed_production() -> f32 {
    1.0
}

impl LayerParams {
    /// Carrying capacity in a biome (kg/m²).
    pub fn capacity(&self, biome: Biome) -> f32 {
        self.capacity_kg_m2.iter().find(|(b, _)| *b == biome).map_or(0.0, |(_, k)| *k)
    }

    /// Validates the record.
    pub fn validate(&self) -> Result<(), EcosystemError> {
        let id = format!("vegetation.{}", self.layer.key());
        let id = id.as_str();
        if self.name.trim().is_empty() {
            return Err(EcosystemError::species(id, "name", "is empty"));
        }
        check_range(id, "height_m", self.height_m, 0.0, 10.0)?;
        check_range(id, "growth_per_day", self.growth_per_day, 0.0, 10.0)?;
        for (biome, k) in &self.capacity_kg_m2 {
            check_range(id, &format!("capacity_kg_m2.{}", biome.key()), *k, 0.0, 50.0)?;
        }
        for (i, (b, _)) in self.capacity_kg_m2.iter().enumerate() {
            if self.capacity_kg_m2[..i].iter().any(|(o, _)| o == b) {
                return Err(EcosystemError::species(id, "capacity_kg_m2", format!("biome {} listed twice", b.key())));
            }
        }
        check_range(id, "light_saturation", self.light_saturation, 0.01, 1.0)?;
        let (t0, t1, t2) = self.temperature_c;
        if !(t0 < t1 && t1 < t2 && t0.is_finite() && t2.is_finite()) {
            return Err(EcosystemError::species(id, "temperature_c", "needs min < optimum < max"));
        }
        let (w0, w1) = self.moisture;
        check_range(id, "moisture.wilting", w0, 0.0, 1.0)?;
        check_range(id, "moisture.optimum", w1, w0, 1.0)?;
        check_range(id, "nutrient_g_per_kg", self.nutrient_g_per_kg, 0.01, 200.0)?;
        check_range(id, "half_saturation_g_m2", self.half_saturation_g_m2, 1e-4, 1000.0)?;
        for (name, v) in [
            ("turnover_per_day", self.turnover_per_day),
            ("cold_dieback_per_day", self.cold_dieback_per_day),
            ("drought_dieback_per_day", self.drought_dieback_per_day),
            ("flood_dieback_per_day", self.flood_dieback_per_day),
            ("germination_per_day", self.germination_per_day),
            ("dispersal_per_day", self.dispersal_per_day),
            ("seed_production_per_day", self.seed_production_per_day),
        ] {
            check_range(id, name, v, 0.0, 1.0)?;
        }
        check_range(id, "leaf_area_per_kg", self.leaf_area_per_kg, 0.0, 100.0)?;
        check_range(id, "flammability", self.flammability, 0.0, 1.0)?;
        check_range(id, "seedling_kg_m2", self.seedling_kg_m2, 0.0, 1.0)?;
        check_range(id, "residual_kg_m2", self.residual_kg_m2, 0.0, 10.0)?;
        Ok(())
    }
}

/// Architecture and rendering style of a tree species.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeStyle {
    /// Broadleaf trees (oak, beech, birch).
    Broadleaf,
    /// Conifers (pine, spruce).
    Conifer,
    /// Palms.
    Palm,
    /// Umbrella-shaped savanna trees.
    Acacia,
}

/// Seed dispersal kernel (research note §4.4).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispersalKernel {
    /// Negative exponential `k(r) ∝ e^{−r/α}` with mean distance `2α`.
    Exponential {
        /// Mean dispersal distance (m).
        mean_m: f32,
    },
    /// Fat-tailed 2Dt kernel (Clark et al. 1999) with scale `u` (m²) and shape `p > 1/2`.
    TwoDt {
        /// Scale parameter (m²).
        scale_m2: f32,
        /// Shape parameter.
        shape: f32,
    },
}

impl DispersalKernel {
    /// Samples a dispersal distance (m) from the radial density `2πr·k(r)`.
    ///
    /// * Exponential: the radial density of the 2-D kernel with scale
    ///   `α = mean / 2` is Gamma(2, α), sampled exactly as `−α ln(u₁u₂)`
    ///   (two draws).
    /// * 2Dt: the radial CDF is `F(r) = 1 − (1 + r²/u)^{−p}`, inverted with
    ///   one draw.
    pub fn sample_distance(self, rng: &mut SimRng) -> f32 {
        match self {
            DispersalKernel::Exponential { mean_m } => {
                let alpha = f64::from(mean_m) / 2.0;
                let u1 = 1.0 - rng.f64();
                let u2 = 1.0 - rng.f64();
                (-alpha * (u1 * u2).ln()) as f32
            }
            DispersalKernel::TwoDt { scale_m2, shape } => {
                let p = f64::from(shape);
                let survival = 1.0 - rng.f64();
                let r2 = f64::from(scale_m2) * (survival.powf(-1.0 / p) - 1.0);
                r2.max(0.0).sqrt() as f32
            }
        }
    }
}

/// Visual model ids of a tree species per growth stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct TreeModels {
    /// Seedlings and saplings.
    pub young: Vec<String>,
    /// Mature trees.
    pub mature: Vec<String>,
    /// Old trees.
    pub old: Vec<String>,
    /// Dead / burnt standing trees.
    pub dead: Vec<String>,
}

/// A tree species (individual trees are simulated).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeSpecies {
    /// Stable identifier.
    pub id: String,
    /// Common name.
    pub name: String,
    /// Scientific name.
    #[serde(default)]
    pub scientific_name: String,
    /// Architecture / rendering style.
    pub style: TreeStyle,
    /// Drops its leaves in autumn.
    pub deciduous: bool,
    /// Maximum height (m).
    pub max_height_m: f32,
    /// Above-ground woody biomass at maximum height (kg).
    pub mature_biomass_kg: f32,
    /// Crown radius / height.
    pub crown_ratio: f32,
    /// Leaf area index inside the crown.
    pub leaf_area_index: f32,
    /// Specific leaf area (m² per kg of leaves).
    pub leaf_area_per_kg: f32,
    /// Height growth of a small, unshaded tree (m per year).
    pub height_growth_m_per_year: f32,
    /// Age at first seeding (years).
    pub maturity_years: f32,
    /// Lifespan (years; Gompertz mortality, ≈ 1 % reach it).
    pub lifespan_years: f32,
    /// Shade tolerance in `[0, 1]`.
    pub shade_tolerance: f32,
    /// Drought tolerance in `[0, 1]`.
    pub drought_tolerance: f32,
    /// Beta-function temperatures `(min, optimum, max)` (°C).
    pub temperature_c: (f32, f32, f32),
    /// Biomes where seedlings establish.
    pub biomes: Vec<Biome>,
    /// Successfully dispersed seeds per mature tree per year.
    pub seeds_per_year: f32,
    /// Seasons in which seeds are released; empty = all year.
    #[serde(default)]
    pub seeding_seasons: Vec<Season>,
    /// Seed dispersal kernel.
    pub dispersal: DispersalKernel,
    /// Nutrient content of wood (g/kg).
    pub wood_nutrient_g_per_kg: f32,
    /// Nutrient content of leaves (g/kg).
    pub leaf_nutrient_g_per_kg: f32,
    /// Resistance of the bark to surface fire in `[0, 1]`.
    pub bark_resistance: f32,
    /// Flammability of the foliage in `[0, 1]` (resinous conifers are high).
    pub flammability: f32,
    /// Resprouts from the root collar after being killed by fire.
    #[serde(default)]
    pub resprouts: bool,
    /// Visual models per stage.
    #[serde(default)]
    pub models: TreeModels,
}

impl TreeSpecies {
    /// Validates the record.
    pub fn validate(&self) -> Result<(), EcosystemError> {
        check_id("tree species", &self.id)?;
        let id = self.id.as_str();
        if self.name.trim().is_empty() {
            return Err(EcosystemError::species(id, "name", "is empty"));
        }
        check_range(id, "max_height_m", self.max_height_m, 0.5, 150.0)?;
        check_range(id, "mature_biomass_kg", self.mature_biomass_kg, 0.1, 1e6)?;
        check_range(id, "crown_ratio", self.crown_ratio, 0.01, 2.0)?;
        check_range(id, "leaf_area_index", self.leaf_area_index, 0.1, 15.0)?;
        check_range(id, "leaf_area_per_kg", self.leaf_area_per_kg, 0.1, 100.0)?;
        check_range(id, "height_growth_m_per_year", self.height_growth_m_per_year, 0.001, 10.0)?;
        check_range(id, "lifespan_years", self.lifespan_years, 1.0, 10_000.0)?;
        check_range(id, "maturity_years", self.maturity_years, 0.1, self.lifespan_years)?;
        check_range(id, "shade_tolerance", self.shade_tolerance, 0.0, 1.0)?;
        check_range(id, "drought_tolerance", self.drought_tolerance, 0.0, 1.0)?;
        let (t0, t1, t2) = self.temperature_c;
        if !(t0 < t1 && t1 < t2 && t0.is_finite() && t2.is_finite()) {
            return Err(EcosystemError::species(id, "temperature_c", "needs min < optimum < max"));
        }
        if self.biomes.is_empty() {
            return Err(EcosystemError::species(id, "biomes", "must list at least one biome"));
        }
        check_range(id, "seeds_per_year", self.seeds_per_year, 0.0, 1e5)?;
        match self.dispersal {
            DispersalKernel::Exponential { mean_m } => check_range(id, "dispersal.mean_m", mean_m, 0.01, 1e5)?,
            DispersalKernel::TwoDt { scale_m2, shape } => {
                check_range(id, "dispersal.scale_m2", scale_m2, 0.01, 1e9)?;
                check_range(id, "dispersal.shape", shape, 0.51, 100.0)?;
            }
        }
        check_range(id, "wood_nutrient_g_per_kg", self.wood_nutrient_g_per_kg, 0.01, 100.0)?;
        check_range(id, "leaf_nutrient_g_per_kg", self.leaf_nutrient_g_per_kg, 0.01, 200.0)?;
        check_range(id, "bark_resistance", self.bark_resistance, 0.0, 1.0)?;
        check_range(id, "flammability", self.flammability, 0.0, 1.0)?;
        Ok(())
    }

    /// Woody biomass (kg) of a tree of height `h`: `B_max (h / H_max)^{2.5}`.
    pub fn wood_at(&self, height_m: f32) -> f32 {
        self.mature_biomass_kg * (height_m.max(0.0) / self.max_height_m).powf(2.5)
    }

    /// Crown radius (m) at height `h`.
    pub fn crown_radius(&self, height_m: f32) -> f32 {
        self.crown_ratio * height_m.max(0.0)
    }

    /// Full-season leaf mass (kg) at height `h`: `LAI · π r² / SLA`.
    pub fn full_leaf_mass(&self, height_m: f32) -> f32 {
        let r = self.crown_radius(height_m);
        self.leaf_area_index * std::f32::consts::PI * r * r / self.leaf_area_per_kg
    }
}

/// Saprotroph (decomposer) parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaprotrophParams {
    /// Litter decay rate (per day) at 20 °C, optimal moisture and saturating fungi.
    pub litter_decay_per_day: f32,
    /// Fraction of that rate achieved by bacteria alone (no fungi).
    pub microbial_fraction: f32,
    /// Dead-wood decay rate (per day, fungi only: white rot).
    pub wood_decay_per_day: f32,
    /// Fungal biomass at which decomposition is half-saturated (kg/m²).
    pub half_saturation_kg_m2: f32,
    /// Growth yield: fungal biomass formed per kg decomposed by fungi.
    pub growth_yield: f32,
    /// Fungal mortality (per day).
    pub mortality_per_day: f32,
    /// Q10 of decomposition.
    pub q10: f32,
    /// Optimal relative soil moisture.
    pub moisture_optimum: f32,
    /// Fraction of decomposed matter humified into soil organic matter.
    pub humification: f32,
    /// Fraction of the biomass difference exchanged with each neighbour per day.
    pub spread_per_day: f32,
    /// Nutrient content (g/kg).
    pub nutrient_g_per_kg: f32,
    /// Decay rate (per day at 20 °C) of litter in water cells (dead algae,
    /// dung, remains), decomposed by bacteria without fungi. Aquatic
    /// detritus is labile: phytoplankton decays at 0.05–0.3 per day
    /// (Enríquez, Duarte & Sand-Jensen 1993), far faster than leaf litter.
    pub aquatic_decay_per_day: f32,
}

/// Mushroom (fruiting body) parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FruitingParams {
    /// Fraction of mycelium converted to mushrooms per day when conditions are met.
    pub rate_per_day: f32,
    /// Soil moisture needed to fruit.
    pub min_moisture: f32,
    /// Temperature window `(min, max)` (°C).
    pub temperature_c: (f32, f32),
    /// Mushroom decay to litter (per day).
    pub decay_per_day: f32,
    /// Mycelium needed to fruit (kg/m²).
    pub min_mycelium_kg_m2: f32,
}

/// Mycorrhizal parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MycorrhizaParams {
    /// Logistic growth rate (per day).
    pub growth_per_day: f32,
    /// Capacity per kg of host foliage per m² (kg fungus / kg host).
    pub capacity_per_host: f32,
    /// Maximum relative increase of nutrient uptake of mycorrhizal plants.
    pub uptake_boost: f32,
    /// Biomass at which the boost is half-saturated (kg/m²).
    pub half_saturation_kg_m2: f32,
    /// Mortality (per day).
    pub mortality_per_day: f32,
}

/// Fungal guild parameters (fungi are decomposers and symbionts, not plants).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FungiParams {
    /// Decomposers.
    pub saprotroph: SaprotrophParams,
    /// Fruiting bodies.
    pub fruiting: FruitingParams,
    /// Root symbionts.
    pub mycorrhiza: MycorrhizaParams,
    /// Mushroom model ids.
    #[serde(default)]
    pub mushroom_models: Vec<String>,
    /// Bracket-fungus model ids (on dead wood).
    #[serde(default)]
    pub bracket_models: Vec<String>,
}

impl FungiParams {
    /// Validates the record.
    pub fn validate(&self) -> Result<(), EcosystemError> {
        let s = &self.saprotroph;
        let id = "fungi";
        check_range(id, "saprotroph.litter_decay_per_day", s.litter_decay_per_day, 0.0, 1.0)?;
        check_range(id, "saprotroph.aquatic_decay_per_day", s.aquatic_decay_per_day, 0.0, 1.0)?;
        check_range(id, "saprotroph.microbial_fraction", s.microbial_fraction, 0.0, 1.0)?;
        check_range(id, "saprotroph.wood_decay_per_day", s.wood_decay_per_day, 0.0, 1.0)?;
        check_range(id, "saprotroph.half_saturation_kg_m2", s.half_saturation_kg_m2, 1e-6, 10.0)?;
        check_range(id, "saprotroph.growth_yield", s.growth_yield, 0.0, 0.9)?;
        check_range(id, "saprotroph.mortality_per_day", s.mortality_per_day, 0.0, 1.0)?;
        check_range(id, "saprotroph.q10", s.q10, 1.0, 5.0)?;
        check_range(id, "saprotroph.moisture_optimum", s.moisture_optimum, 0.05, 1.0)?;
        check_range(id, "saprotroph.humification", s.humification, 0.0, 0.9)?;
        check_range(id, "saprotroph.spread_per_day", s.spread_per_day, 0.0, 0.2)?;
        check_range(id, "saprotroph.nutrient_g_per_kg", s.nutrient_g_per_kg, 0.1, 200.0)?;
        let f = &self.fruiting;
        check_range(id, "fruiting.rate_per_day", f.rate_per_day, 0.0, 1.0)?;
        check_range(id, "fruiting.min_moisture", f.min_moisture, 0.0, 1.0)?;
        if f.temperature_c.0.is_nan() || f.temperature_c.1.is_nan() || f.temperature_c.0 >= f.temperature_c.1 {
            return Err(EcosystemError::species(id, "fruiting.temperature_c", "needs min < max"));
        }
        check_range(id, "fruiting.decay_per_day", f.decay_per_day, 0.0, 1.0)?;
        check_range(id, "fruiting.min_mycelium_kg_m2", f.min_mycelium_kg_m2, 0.0, 10.0)?;
        let m = &self.mycorrhiza;
        check_range(id, "mycorrhiza.growth_per_day", m.growth_per_day, 0.0, 1.0)?;
        check_range(id, "mycorrhiza.capacity_per_host", m.capacity_per_host, 0.0, 10.0)?;
        check_range(id, "mycorrhiza.uptake_boost", m.uptake_boost, 0.0, 5.0)?;
        check_range(id, "mycorrhiza.half_saturation_kg_m2", m.half_saturation_kg_m2, 1e-6, 10.0)?;
        check_range(id, "mycorrhiza.mortality_per_day", m.mortality_per_day, 0.0, 1.0)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_kernel_mean() {
        let kernel = DispersalKernel::Exponential { mean_m: 30.0 };
        let mut rng = SimRng::seed_from_u64(3);
        let n = 40_000;
        let mean: f64 = (0..n).map(|_| f64::from(kernel.sample_distance(&mut rng))).sum::<f64>() / n as f64;
        assert!((mean - 30.0).abs() < 0.6, "mean {mean}");
    }

    #[test]
    fn two_dt_kernel_is_fat_tailed() {
        let kernel = DispersalKernel::TwoDt { scale_m2: 400.0, shape: 1.0 };
        let mut rng = SimRng::seed_from_u64(5);
        let samples: Vec<f32> = (0..20_000).map(|_| kernel.sample_distance(&mut rng)).collect();
        let far = samples.iter().filter(|&&r| r > 200.0).count();
        // CDF: P(r > 200) = (1 + 40000/400)^{-1} ≈ 0.0099.
        let p = far as f64 / samples.len() as f64;
        assert!((p - 0.0099).abs() < 0.004, "tail {p}");
    }
}
