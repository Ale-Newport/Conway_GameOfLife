//! Configuration of an evolution experiment (what scenario files contain).
//!
//! Every struct uses `#[serde(default)]`, so a scenario only lists the values
//! it changes, and `deny_unknown_fields`, so typos are reported instead of
//! silently ignored. [`EvolutionConfig::validate`] checks every value: the
//! fields exposed as parameters are checked against their
//! [`simulation_core::ParamSpec`] ranges (one source of truth), the others
//! (shapes, patches, trait overrides...) explicitly.

use organism_engine::{Crossover, MutationBounds};
use serde::{Deserialize, Serialize};

use crate::error::EvolutionError;
use crate::genes::{TraitValue, validate_traits};
use crate::params;

/// Surface type of a world cell. Only land is passable and grows food.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terrain {
    /// Passable, grows food.
    #[default]
    Land,
    /// Impassable water.
    Water,
    /// Impassable mountain.
    Mountain,
}

impl Terrain {
    /// Whether organisms can stand on the cell.
    pub fn is_passable(self) -> bool {
        self == Self::Land
    }
}

/// A region of the world in cell coordinates (a cell belongs to a shape when
/// its centre does).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// Axis-aligned rectangle `[x, x + w) × [y, y + h)`.
    Rect {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
    },
    /// Disc of radius `r` around `(x, y)`.
    Disc {
        /// Centre x.
        x: f32,
        /// Centre y.
        y: f32,
        /// Radius.
        r: f32,
    },
}

impl Shape {
    /// Whether the point lies inside the shape.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        match *self {
            Self::Rect { x, y, w, h } => px >= x && px < x + w && py >= y && py < y + h,
            Self::Disc { x, y, r } => (px - x) * (px - x) + (py - y) * (py - y) <= r * r,
        }
    }

    /// Checks that all numbers are finite and sizes positive.
    pub fn validate(&self, field: &str) -> Result<(), EvolutionError> {
        let (values, sizes): (Vec<f32>, Vec<f32>) = match *self {
            Self::Rect { x, y, w, h } => (vec![x, y, w, h], vec![w, h]),
            Self::Disc { x, y, r } => (vec![x, y, r], vec![r]),
        };
        if values.iter().any(|v| !v.is_finite()) {
            return Err(EvolutionError::config(field, "shape coordinates must be finite numbers"));
        }
        if sizes.iter().any(|s| *s <= 0.0) {
            return Err(EvolutionError::config(field, "shape sizes must be positive"));
        }
        Ok(())
    }
}

/// World dimensions in cells (one cell = one world unit). The world edges are
/// walls (positions are clamped).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldConfig {
    /// Width in cells.
    pub width: u32,
    /// Height in cells.
    pub height: u32,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self { width: 96, height: 64 }
    }
}

/// A fertile patch: cells inside `shape` have carrying capacity
/// `max(background, capacity)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoodPatch {
    /// Area of the patch.
    pub shape: Shape,
    /// Carrying capacity inside the patch (food units per cell).
    pub capacity: f32,
}

/// Food on land cells: logistic regrowth towards a per-cell capacity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FoodConfig {
    /// Background carrying capacity of a land cell (food units).
    pub capacity: f32,
    /// Richer patches.
    pub patches: Vec<FoodPatch>,
    /// Multiplier applied to every capacity (scarcity experiments).
    pub capacity_scale: f32,
    /// Logistic growth rate r per tick.
    pub regrowth_rate: f32,
    /// Seed bank: growth is computed from `max(F, seed_bank × K)` so grazed-out cells regrow.
    pub seed_bank: f32,
    /// Energy gained per food unit eaten.
    pub energy: f32,
    /// Initial food as a fraction of capacity.
    pub initial_fraction: f32,
}

impl Default for FoodConfig {
    fn default() -> Self {
        Self {
            capacity: 8.0,
            patches: Vec::new(),
            capacity_scale: 1.0,
            regrowth_rate: 0.05,
            seed_bank: 0.1,
            energy: 1.0,
            initial_fraction: 1.0,
        }
    }
}

/// Temperature field `T(x, y, t)`; see [`crate::environment`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClimateConfig {
    /// Temperature at the world centre at tick 0 (°C).
    pub base_temperature: f32,
    /// Temperature difference from the left to the right edge (°C).
    pub gradient_x: f32,
    /// Temperature difference from the top to the bottom edge (°C).
    pub gradient_y: f32,
    /// Directional shift in °C per 1000 ticks (climate change).
    pub shift_rate: f32,
    /// Amplitude of a sinusoidal seasonal oscillation (°C).
    pub seasonal_amplitude: f32,
    /// Period of the seasonal oscillation (ticks).
    pub seasonal_period: u32,
}

impl Default for ClimateConfig {
    fn default() -> Self {
        Self {
            base_temperature: 20.0,
            gradient_x: 0.0,
            gradient_y: 0.0,
            shift_rate: 0.0,
            seasonal_amplitude: 0.0,
            seasonal_period: 1000,
        }
    }
}

/// A named terrain overlay that can be switched on and off (land bridges,
/// rivers, mountain ranges). Barriers are painted in list order onto a land
/// world; a later barrier overrides earlier ones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Barrier {
    /// Display name.
    pub name: String,
    /// Terrain painted (use `land` to open a corridor through an earlier barrier).
    pub terrain: Terrain,
    /// Painted area (union of shapes).
    pub shapes: Vec<Shape>,
    /// Paint everything *outside* the shapes instead (e.g. an ocean around islands).
    #[serde(default)]
    pub outside: bool,
    /// Whether the barrier is present at the start.
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_true() -> bool {
    true
}

/// How cells are grouped into regions for population and divergence statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionMode {
    /// Connected components of passable land (4-neighbourhood): islands.
    #[default]
    Islands,
    /// A `cols × rows` grid of sectors (passable cells only), e.g. bands along a gradient.
    Sectors {
        /// Columns.
        cols: u32,
        /// Rows.
        rows: u32,
    },
}

/// Founding population of a species.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PopulationConfig {
    /// Number of founders.
    pub initial: u32,
    /// Standard deviation of founder genes around the means (normalised units).
    pub spread: f32,
    /// Founder gene means (phenotypic values); unspecified genes use defaults.
    pub traits: Vec<TraitValue>,
    /// Founders are placed uniformly on land inside this shape (whole world if absent).
    pub area: Option<Shape>,
}

impl Default for PopulationConfig {
    fn default() -> Self {
        Self { initial: 200, spread: 0.12, traits: Vec::new(), area: None }
    }
}

/// Predators: founders and hunting parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PredatorConfig {
    /// Founding predators (0 = no predators until introduced by command).
    pub initial: u32,
    /// Founder gene spread (normalised units).
    pub spread: f32,
    /// Founder gene means.
    pub traits: Vec<TraitValue>,
    /// Founder area.
    pub area: Option<Shape>,
    /// Whether predator offspring recombine and mutate (co-evolution). When
    /// false, offspring are exact clones of the first parent.
    pub evolve: bool,
    /// Constant predation regime (an experimental treatment): predators
    /// neither breed nor die (their energy and health are floored at 0, so
    /// hunger still drives hunting); their number changes only by command.
    pub constant: bool,
    /// Probability that a contact attack kills an equally sized prey.
    pub capture_probability: f32,
    /// Fraction of prey energy assimilated by the predator.
    pub assimilation: f32,
    /// Energy content of prey tissue per unit body mass (added to the prey's reserve).
    pub body_energy: f32,
    /// Ticks a predator spends handling a kill before it can hunt again
    /// (gives a saturating, Holling type II functional response).
    pub handling_ticks: u32,
    /// Interference coefficient `c`: a hungry predator with `n` rivals within
    /// 3 units loses the tick with probability `1 − e^{−c n}` (0 = none).
    pub interference: f32,
}

impl Default for PredatorConfig {
    fn default() -> Self {
        Self {
            initial: 0,
            spread: 0.0,
            traits: Vec::new(),
            area: None,
            evolve: false,
            constant: false,
            capture_probability: 0.5,
            assimilation: 0.5,
            body_energy: 20.0,
            handling_ticks: 20,
            interference: 0.0,
        }
    }
}

/// Energy budget, thermal physiology and mortality (see [`crate::organism`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PhysiologyConfig {
    /// Basal cost per tick of a unit-size, unit-efficiency organism.
    pub basal_cost: f32,
    /// Movement cost coefficient: `c × size × speed²` per tick (speed actually used).
    pub movement_cost: f32,
    /// Upkeep of locomotor capacity: `c × size × max_speed²` per tick, paid
    /// whether or not the organism runs (muscle is expensive tissue).
    pub speed_upkeep: f32,
    /// Perception cost per unit of radius per tick.
    pub perception_cost: f32,
    /// Basal cost increase per °C of tolerance above 1 °C (generalist cost).
    pub tolerance_cost: f32,
    /// Basal cost multiplier per squared normalised thermal mismatch.
    pub thermal_stress_cost: f32,
    /// Health lost per tick per unit of mismatch beyond the tolerance.
    pub thermal_damage: f32,
    /// Health regained per tick inside the tolerance.
    pub health_recovery: f32,
    /// Energy storage capacity per unit body mass.
    pub storage: f32,
    /// Maximum food intake per tick of a unit-size, unit-efficiency organism.
    pub intake_rate: f32,
    /// Foraging, searching and courting speed (world units per tick, capped
    /// by the speed gene); full speed is used only to flee or hunt.
    pub cruise_speed: f32,
    /// Maximum acceleration as a fraction of maximum speed per tick.
    pub agility: f32,
    /// Age-independent death probability per tick.
    pub background_mortality: f32,
    /// Senescence hazard per tick at birth (Gompertz initial mortality).
    pub senescence_onset: f32,
    /// Age, in units of the maturity age, at which the senescence hazard has grown 1000-fold.
    pub longevity_ratio: f32,
    /// Fraction of the thermal mismatch absorbed by acclimation per tick.
    pub acclimation_rate: f32,
    /// Experience gained per tick of escape: `e += rate × (1 − e)`.
    pub learning_rate: f32,
    /// Experience lost per tick without threat: `e −= rate × e`.
    pub forgetting_rate: f32,
}

impl Default for PhysiologyConfig {
    fn default() -> Self {
        Self {
            basal_cost: 0.2,
            movement_cost: 0.15,
            speed_upkeep: 0.05,
            perception_cost: 0.006,
            tolerance_cost: 0.03,
            thermal_stress_cost: 0.5,
            thermal_damage: 0.05,
            health_recovery: 0.01,
            storage: 60.0,
            intake_rate: 1.5,
            cruise_speed: 0.4,
            agility: 0.5,
            background_mortality: 0.0005,
            senescence_onset: 1e-4,
            longevity_ratio: 4.0,
            acclimation_rate: 0.01,
            learning_rate: 0.3,
            forgetting_rate: 0.005,
        }
    }
}

/// Asexual (clonal) or sexual (two-parent) reproduction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReproductionMode {
    /// One parent; offspring are mutated clones.
    #[default]
    Asexual,
    /// Two hermaphroditic parents in contact; offspring genomes recombine.
    Sexual,
}

/// Breeding rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReproductionConfig {
    /// Asexual or sexual.
    pub mode: ReproductionMode,
    /// Recombination operator in sexual mode.
    pub crossover: Crossover,
    /// Energy fraction of storage required to breed.
    pub threshold: f32,
    /// Ticks after breeding before an organism may breed again.
    pub refractory: u32,
    /// Fraction of the breeding budget that ends up in the offspring.
    pub birth_efficiency: f32,
    /// Contact distance for mating.
    pub mate_radius: f32,
    /// Maximum genetic distance (0–1) between mates; 1 = no restriction.
    pub mate_distance_limit: f32,
}

impl Default for ReproductionConfig {
    fn default() -> Self {
        Self {
            mode: ReproductionMode::Asexual,
            crossover: Crossover::Uniform,
            threshold: 0.6,
            refractory: 20,
            birth_efficiency: 0.85,
            mate_radius: 1.5,
            mate_distance_limit: 1.0,
        }
    }
}

/// Mutation of heritable genes at birth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MutationConfig {
    /// Per-gene mutation probability per birth.
    pub rate: f32,
    /// Standard deviation of a mutation step (fraction of the gene range).
    pub size: f32,
    /// Boundary handling of mutated values.
    pub bounds: MutationBounds,
}

impl Default for MutationConfig {
    fn default() -> Self {
        Self { rate: 0.1, size: 0.05, bounds: MutationBounds::Reflect }
    }
}

/// Measurement settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatisticsConfig {
    /// Histogram bins per gene.
    pub histogram_bins: u32,
    /// Number of equal-width value classes per gene for allele-like frequencies.
    pub allele_classes: u32,
    /// Random pairs sampled for genetic-distance estimates.
    pub sample_pairs: u32,
    /// Node cap of the genealogy (oldest dead ancestors are truncated first).
    pub genealogy_max_nodes: u32,
}

impl Default for StatisticsConfig {
    fn default() -> Self {
        Self { histogram_bins: 12, allele_classes: 4, sample_pairs: 1000, genealogy_max_nodes: 20_000 }
    }
}

/// The explicit, observable speciation criterion (see [`crate::speciation`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpeciationConfig {
    /// Whether the criterion is evaluated.
    pub enabled: bool,
    /// Minimum share of the population in each of the two clusters.
    pub min_cluster_fraction: f32,
    /// Generations the separation must persist before a split is declared.
    pub generations: u32,
    /// Ticks between evaluations.
    pub check_interval: u32,
}

impl Default for SpeciationConfig {
    fn default() -> Self {
        Self { enabled: true, min_cluster_fraction: 0.1, generations: 5, check_interval: 20 }
    }
}

/// Complete experiment configuration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EvolutionConfig {
    /// World size.
    pub world: WorldConfig,
    /// Food.
    pub food: FoodConfig,
    /// Temperature field.
    pub climate: ClimateConfig,
    /// Terrain overlays.
    pub barriers: Vec<Barrier>,
    /// Region definition for statistics.
    pub regions: RegionMode,
    /// Prey founders.
    pub prey: PopulationConfig,
    /// Additional prey founder groups with their own gene means (e.g. two
    /// competing life-history strategies); each founder starts its own lineage.
    pub extra_prey: Vec<PopulationConfig>,
    /// Predators.
    pub predators: PredatorConfig,
    /// Physiology.
    pub physiology: PhysiologyConfig,
    /// Reproduction.
    pub reproduction: ReproductionConfig,
    /// Mutation.
    pub mutation: MutationConfig,
    /// Statistics.
    pub statistics: StatisticsConfig,
    /// Speciation criterion.
    pub speciation: SpeciationConfig,
    /// Stop after this many ticks (0 = run until extinction).
    pub max_ticks: u64,
}

/// Checks `lo <= value <= hi` for a non-parameter field.
fn check_range(field: &str, value: f32, lo: f32, hi: f32) -> Result<(), EvolutionError> {
    if value.is_finite() && (lo..=hi).contains(&value) {
        Ok(())
    } else {
        Err(EvolutionError::config(field, format!("{value} is outside [{lo}, {hi}]")))
    }
}

impl EvolutionConfig {
    /// Validates every value; the error names the first offending field.
    pub fn validate(&self) -> Result<(), EvolutionError> {
        for spec in params::specs() {
            let value = params::get(self, &spec.key)?;
            spec.validate(&value).map_err(|e| EvolutionError::config(params::field_path(&spec.key), e.to_string()))?;
        }
        let f = &self.food;
        check_range("food.capacity", f.capacity, 0.0, 1000.0)?;
        check_range("food.seed_bank", f.seed_bank, 0.0, 1.0)?;
        check_range("food.initial_fraction", f.initial_fraction, 0.0, 1.0)?;
        for (i, patch) in f.patches.iter().enumerate() {
            patch.shape.validate(&format!("food.patches[{i}].shape"))?;
            check_range(&format!("food.patches[{i}].capacity"), patch.capacity, 0.0, 1000.0)?;
        }
        for (i, barrier) in self.barriers.iter().enumerate() {
            if barrier.name.trim().is_empty() {
                return Err(EvolutionError::config(format!("barriers[{i}].name"), "a barrier needs a name"));
            }
            for (j, shape) in barrier.shapes.iter().enumerate() {
                shape.validate(&format!("barriers[{i}].shapes[{j}]"))?;
            }
        }
        if let RegionMode::Sectors { cols, rows } = self.regions
            && !((1..=64).contains(&cols) && (1..=64).contains(&rows))
        {
            return Err(EvolutionError::config("regions", "sector columns and rows must be in [1, 64]"));
        }
        let predators = self.population_of_predators();
        let mut populations = vec![("prey".to_string(), &self.prey), ("predators".to_string(), &predators)];
        populations.extend(self.extra_prey.iter().enumerate().map(|(i, p)| (format!("extra_prey[{i}]"), p)));
        for (name, pop) in populations {
            if pop.initial > 5000 {
                return Err(EvolutionError::config(format!("{name}.initial"), "at most 5000 founders per group"));
            }
            check_range(&format!("{name}.spread"), pop.spread, 0.0, 0.5)?;
            validate_traits(&pop.traits, &format!("{name}.traits"))?;
            if let Some(area) = &pop.area {
                area.validate(&format!("{name}.area"))?;
            }
        }
        check_range("predators.body_energy", self.predators.body_energy, 0.0, 1000.0)?;
        let p = &self.physiology;
        check_range("physiology.thermal_damage", p.thermal_damage, 0.0, 1.0)?;
        check_range("physiology.health_recovery", p.health_recovery, 0.0, 1.0)?;
        check_range("physiology.storage", p.storage, 1.0, 10_000.0)?;
        check_range("physiology.intake_rate", p.intake_rate, 0.0, 100.0)?;
        check_range("physiology.cruise_speed", p.cruise_speed, 0.05, 2.0)?;
        check_range("physiology.agility", p.agility, 0.05, 2.0)?;
        check_range("physiology.senescence_onset", p.senescence_onset, 0.0, 0.1)?;
        check_range("physiology.acclimation_rate", p.acclimation_rate, 0.0, 1.0)?;
        check_range("physiology.learning_rate", p.learning_rate, 0.0, 1.0)?;
        check_range("physiology.forgetting_rate", p.forgetting_rate, 0.0, 1.0)?;
        let r = &self.reproduction;
        check_range("reproduction.birth_efficiency", r.birth_efficiency, 0.0, 1.0)?;
        check_range("reproduction.mate_radius", r.mate_radius, 0.1, 20.0)?;
        let s = &self.statistics;
        if !(10..=100_000).contains(&s.sample_pairs) {
            return Err(EvolutionError::config("statistics.sample_pairs", "must be in [10, 100000]"));
        }
        let sp = &self.speciation;
        check_range("speciation.min_cluster_fraction", sp.min_cluster_fraction, 0.01, 0.5)?;
        if !(1..=10_000).contains(&sp.check_interval) {
            return Err(EvolutionError::config("speciation.check_interval", "must be in [1, 10000]"));
        }
        Ok(())
    }

    /// The predator founders seen as a [`PopulationConfig`] (shared validation).
    fn population_of_predators(&self) -> PopulationConfig {
        PopulationConfig {
            initial: self.predators.initial,
            spread: self.predators.spread,
            traits: self.predators.traits.clone(),
            area: self.predators.area,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        EvolutionConfig::default().validate().unwrap();
    }

    #[test]
    fn errors_name_the_field() {
        let mut c = EvolutionConfig::default();
        c.food.regrowth_rate = 3.0;
        let e = c.validate().unwrap_err().to_string();
        assert!(e.contains("food.regrowth_rate"), "{e}");
        let mut c = EvolutionConfig::default();
        c.barriers.push(Barrier {
            name: "river".into(),
            terrain: Terrain::Water,
            shapes: vec![Shape::Rect { x: 0.0, y: 0.0, w: -1.0, h: 3.0 }],
            outside: false,
            active: true,
        });
        assert!(c.validate().unwrap_err().to_string().contains("barriers[0].shapes[0]"));
        let mut c = EvolutionConfig::default();
        c.prey.traits.push(TraitValue::new("speed", 99.0));
        assert!(matches!(c.validate(), Err(EvolutionError::TraitOutOfRange { .. })));
    }

    #[test]
    fn shapes_contain_cell_centres() {
        let r = Shape::Rect { x: 2.0, y: 2.0, w: 3.0, h: 1.0 };
        assert!(r.contains(2.5, 2.5) && r.contains(4.5, 2.5) && !r.contains(5.5, 2.5));
        let d = Shape::Disc { x: 0.0, y: 0.0, r: 2.0 };
        assert!(d.contains(1.0, 1.0) && !d.contains(1.5, 1.5));
    }
}
