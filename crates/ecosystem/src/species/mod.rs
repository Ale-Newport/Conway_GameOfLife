//! Data-driven species definitions.
//!
//! Animals are described by [`Species`] records loaded from RON files in
//! `assets/species/animals/`; plant layers, tree species and fungal guilds
//! live in [`plants`] and are loaded from `assets/species/`. Every record is
//! validated with precise messages ([`Species::validate`]) and the whole
//! [`SpeciesCatalog`] checks cross references (prey ids, duplicate ids).
//! Numeric per-tick parameters are derived from the records by
//! [`derived::AnimalTraits`] (Kleiber metabolism, Hirt speeds...).

pub mod behaviour;
pub mod catalog;
pub mod derived;
pub mod lifestyle;
pub mod plants;
mod validate;

use physics_world::{Biome, Season};
use serde::{Deserialize, Serialize};

pub use behaviour::{
    ActionWeights, ActivityPattern, Behaviour, Climate, Combat, HuntingStyle, Social, SocialStructure,
};
pub use catalog::SpeciesCatalog;
pub use derived::AnimalTraits;
pub use lifestyle::{ColonyParams, FieldParams, Hibernation, Representation, Seasonal, WaterKind};
pub use plants::{FungiParams, LayerParams, PlantLayer, TreeSpecies, TreeStyle};

/// Taxonomic class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxonClass {
    /// Mammals.
    Mammal,
    /// Birds.
    Bird,
    /// Reptiles.
    Reptile,
    /// Amphibians.
    Amphibian,
    /// Fish.
    Fish,
    /// Insects.
    Insect,
    /// Spiders and relatives.
    Arachnid,
    /// Crustaceans.
    Crustacean,
    /// Earthworms and relatives.
    Annelid,
    /// Snails and relatives.
    Mollusc,
}

/// Ecological role (several per species).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Eats plants.
    Herbivore,
    /// Eats animals.
    Carnivore,
    /// Eats plants and animals.
    Omnivore,
    /// Kills other animals.
    Predator,
    /// Hunted by other animals.
    Prey,
    /// Top predator without natural predators.
    Apex,
    /// Eats carrion.
    Scavenger,
    /// Eats litter and dung.
    Detritivore,
    /// Feeds on nectar and pollinates flowers.
    Pollinator,
    /// Breaks down dead matter.
    Decomposer,
    /// Lives in groups.
    Social,
    /// Lives alone.
    Solitary,
}

/// A food item of a diet.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Food {
    /// Biomass of a vegetation layer (grazing).
    Plant(PlantLayer),
    /// Foliage of young trees within reach (browsing).
    Browse,
    /// Live animals of a species (by id).
    Prey(String),
    /// Carcasses of any species.
    Carrion,
    /// Nectar of the flower layer (pollinates it).
    Nectar,
    /// Litter and dung.
    Detritus,
    /// Mushrooms (fungal fruiting bodies).
    Fungi,
    /// Eggs of other species' clutches (nest predation).
    Eggs,
}

impl Food {
    /// Default assimilation efficiency (fraction of ingested energy and
    /// nutrients absorbed; the rest is egested). Carnivores ≈ 0.8,
    /// herbivores 0.2–0.6 depending on food, detritivores ≈ 0.3
    /// (research note §3.1).
    pub fn default_assimilation(&self) -> f32 {
        match self {
            Food::Plant(layer) => layer.default_assimilation(),
            Food::Browse => 0.35,
            Food::Prey(_) => 0.8,
            Food::Carrion => 0.75,
            Food::Nectar => 0.9,
            Food::Detritus => 0.3,
            Food::Fungi => 0.6,
            Food::Eggs => 0.8,
        }
    }
}

/// One diet entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DietEntry {
    /// What is eaten.
    pub food: Food,
    /// Preference in `(0, 1]` (relative weight when choosing food).
    pub preference: f32,
    /// Assimilation efficiency override in `(0, 1]`.
    #[serde(default)]
    pub assimilation: Option<f32>,
}

impl DietEntry {
    /// Effective assimilation efficiency.
    pub fn assimilation(&self) -> f32 {
        self.assimilation.unwrap_or_else(|| self.food.default_assimilation())
    }
}

/// Locomotion modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locomotion {
    /// Walks on land.
    Walk,
    /// Runs on land (sprint speed from the running model).
    Run,
    /// Flies (ignores terrain).
    Fly,
    /// Swims (crosses water).
    Swim,
    /// Digs burrows (refuge).
    Burrow,
    /// Jumps.
    Jump,
    /// Climbs trees (refuge).
    Climb,
}

/// Places where a species hides when threatened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refuge {
    /// Its own burrow (requires [`Locomotion::Burrow`]).
    Burrow,
    /// Cells with dense shrubs or canopy.
    DenseVegetation,
    /// Trees (requires [`Locomotion::Climb`] or [`Locomotion::Fly`]).
    Trees,
    /// Water bodies (requires [`Locomotion::Swim`]).
    Water,
}

/// Habitat constraints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Habitat {
    /// Preferred biomes (spawn placement, wandering bias, comfort).
    pub biomes: Vec<Biome>,
    /// Lives in water only (fish).
    #[serde(default)]
    pub aquatic: bool,
    /// Steepest walkable slope (rise/run).
    #[serde(default = "default_max_slope")]
    pub max_slope: f32,
    /// Deepest water (m) a non-swimmer can wade through.
    #[serde(default = "default_wade_depth")]
    pub max_wade_depth_m: f32,
    /// Refuges used when fleeing.
    #[serde(default)]
    pub refuges: Vec<Refuge>,
    /// Water an aquatic species lives in (fresh, salt or both).
    #[serde(default)]
    pub water: WaterKind,
    /// Shallowest water (m) an aquatic species can live in.
    #[serde(default)]
    pub min_depth_m: f32,
    /// Eggs (or spawn) are laid in water: a gravid female heads for water
    /// and lays only within one cell of it (amphibians).
    #[serde(default)]
    pub breeds_in_water: bool,
}

fn default_max_slope() -> f32 {
    1.0
}

fn default_wade_depth() -> f32 {
    0.3
}

/// Body temperature regulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Thermoregulation {
    /// Constant body temperature (mammals, birds).
    Endotherm,
    /// Body temperature follows the air (reptiles, amphibians, fish, insects).
    Ectotherm,
}

/// Body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    /// Adult body mass (kg).
    pub mass_kg: f32,
    /// Adult body length (m), used for rendering size.
    pub length_m: f32,
    /// Temperature regulation.
    pub thermoregulation: Thermoregulation,
    /// Body temperature of endotherms (°C).
    #[serde(default = "default_body_temperature")]
    pub body_temperature_c: f32,
}

fn default_body_temperature() -> f32 {
    37.0
}

/// Energetics beyond the Kleiber basal rate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Metabolism {
    /// Field / basal metabolic ratio of an awake animal, excluding
    /// locomotion (paid separately per metre).
    pub field_multiplier: f32,
    /// Days of field metabolism stored as reserves (hunger rate = 1 /
    /// fasting days). `None` derives `5 · M^{1/4} / c_th` (Lindstedt & Boyce 1985).
    pub fasting_days: Option<f32>,
    /// Days from full hydration to dehydration (thirst rate = 1 / days).
    pub dehydration_days: f32,
    /// Largest ingestion per hour, as a fraction of the daily field
    /// metabolism (grazers ~0.15, gorging carnivores ~2).
    pub intake_per_hour: f32,
    /// Water economy: multiplier of the water an animal gains from its food
    /// (preformed and metabolic water, `animal::physiology`). 1 for most
    /// species; reptiles, whose evaporative and urinary losses are small,
    /// cover their need from prey (about 2: Nagy 1982) and rarely drink.
    pub food_water: f32,
}

impl Default for Metabolism {
    fn default() -> Self {
        Self {
            field_multiplier: 2.0,
            fasting_days: None,
            dehydration_days: 3.0,
            intake_per_hour: 0.15,
            food_water: 1.0,
        }
    }
}

/// Movement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Movement {
    /// Sprint speed (m/s); `None` uses Hirt et al. (2017) for the main mode.
    pub sprint_speed_m_s: Option<f32>,
    /// Cruising speed as a fraction of the sprint speed.
    pub cruise_fraction: f32,
    /// Seconds of sprinting before stamina is exhausted.
    pub endurance_s: f32,
    /// Acceleration in units of max speed per tick.
    pub agility: f32,
    /// Cruising altitude above the ground (m) of fliers while travelling;
    /// they land to feed, drink, rest and breed. Ignored for non-fliers.
    pub flight_altitude_m: f32,
}

impl Default for Movement {
    fn default() -> Self {
        Self { sprint_speed_m_s: None, cruise_fraction: 0.3, endurance_s: 60.0, agility: 2.0, flight_altitude_m: 30.0 }
    }
}

/// Perception.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Perception {
    /// Vision radius (m).
    pub vision_m: f32,
    /// Field of view (degrees, full aperture).
    pub fov_deg: f32,
    /// Omnidirectional hearing radius (m): moving animals are heard.
    pub hearing_m: f32,
    /// Chemoreception radius (m): animals within it are smelled whether
    /// they move or not (a snake's tongue flicking finds resting frogs).
    /// 0 for species that do not hunt by scent.
    #[serde(default)]
    pub smell_m: f32,
}

/// Life history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifeHistory {
    /// Lifespan (years): ≈ 1 % of animals reach it (Gompertz mortality).
    pub lifespan_years: f32,
    /// Age at sexual maturity (real days).
    pub maturity_days: f32,
}

/// Sexual or asexual reproduction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReproductionMode {
    /// Two parents (uniform crossover of their genomes).
    Sexual,
    /// One parent (clone with mutation).
    Asexual,
}

/// Live birth or eggs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Development {
    /// Gestation then live birth.
    Live {
        /// Gestation (real days).
        gestation_days: f32,
    },
    /// Egg formation, laying, then incubation of a clutch.
    Eggs {
        /// Egg formation before laying (real days).
        laying_days: f32,
        /// Incubation until hatching (real days).
        incubation_days: f32,
    },
}

/// Reproduction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reproduction {
    /// Sexual or asexual.
    pub mode: ReproductionMode,
    /// Live birth or eggs.
    pub development: Development,
    /// Litter / clutch size range `(min, max)`.
    pub litter: (u16, u16),
    /// Seasons in which mating (or asexual conception) can start; empty = all year.
    #[serde(default)]
    pub breeding_seasons: Vec<Season>,
    /// Minimum interval between births (real days).
    #[serde(default = "default_interval")]
    pub interval_days: f32,
    /// Days the young depend on the mother (suckling / feeding); 0 = none.
    #[serde(default)]
    pub parental_care_days: f32,
    /// Birth mass per offspring as a fraction of adult mass; `None` derives
    /// `0.1 · M^{−0.08} / litter` (mammal-like).
    #[serde(default)]
    pub birth_mass_fraction: Option<f32>,
    /// Energy reserve fraction required to conceive (energy-gated breeding).
    #[serde(default = "default_min_energy")]
    pub min_energy_fraction: f32,
}

fn default_interval() -> f32 {
    365.0
}

fn default_min_energy() -> f32 {
    0.5
}

/// Procedural body plan used when no authored model exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BodyPlan {
    /// Four legs.
    #[default]
    Quadruped,
    /// Two legs.
    Biped,
    /// Bird.
    Bird,
    /// Fish.
    Fish,
    /// Snake.
    Serpent,
    /// Frog-like.
    Amphibian,
    /// Six-legged insect.
    Insect,
    /// Eight-legged arachnid.
    Arachnid,
    /// Worm.
    Worm,
}

/// How renderers draw the species.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visual {
    /// Model id in `assets/models/manifest.toml`, if an authored model exists.
    #[serde(default)]
    pub model: Option<String>,
    /// Id of the procedural rig in `organism-render`'s species table (e.g.
    /// `bear`, `eagle`, `bee`); used when no authored model exists and for
    /// distant level of detail.
    #[serde(default)]
    pub procedural: Option<String>,
    /// Procedural body plan (fallback and clip fallback).
    #[serde(default)]
    pub body_plan: BodyPlan,
    /// Base colour (linear RGB in `[0, 1]`).
    pub color: (f32, f32, f32),
    /// Extra scale factor on top of the body length.
    #[serde(default = "default_scale")]
    pub scale: f32,
}

fn default_scale() -> f32 {
    1.0
}

impl Visual {
    /// The id renderers use: the manifest model id if there is one,
    /// otherwise `proc:<procedural rig id>`.
    pub fn visual_id(&self) -> String {
        match (&self.model, &self.procedural) {
            (Some(model), _) => model.clone(),
            (None, Some(rig)) => format!("proc:{rig}"),
            (None, None) => String::new(),
        }
    }
}

/// An animal species.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Species {
    /// Stable identifier (lowercase letters, digits, `_`, `-`).
    pub id: String,
    /// Common name.
    pub name: String,
    /// Scientific name.
    #[serde(default)]
    pub scientific_name: String,
    /// Taxonomic class.
    pub class: TaxonClass,
    /// Ecological roles.
    pub roles: Vec<Role>,
    /// Diet entries.
    pub diet: Vec<DietEntry>,
    /// Locomotion modes.
    pub locomotion: Vec<Locomotion>,
    /// Habitat constraints.
    pub habitat: Habitat,
    /// Body.
    pub body: Body,
    /// Energetics.
    #[serde(default)]
    pub metabolism: Metabolism,
    /// Movement.
    #[serde(default)]
    pub movement: Movement,
    /// Perception.
    pub perception: Perception,
    /// Life history.
    pub life: LifeHistory,
    /// Reproduction.
    pub reproduction: Reproduction,
    /// Social behaviour.
    #[serde(default)]
    pub social: Social,
    /// Decision making.
    #[serde(default)]
    pub behaviour: Behaviour,
    /// Fighting ability.
    #[serde(default)]
    pub combat: Combat,
    /// Thermal niche.
    pub climate: Climate,
    /// Migration and hibernation.
    #[serde(default)]
    pub seasonal: Seasonal,
    /// Individual agents, density field or colonies.
    #[serde(default)]
    pub representation: Representation,
    /// Rendering.
    pub visual: Visual,
}

impl Species {
    /// Whether the species has a role.
    pub fn has_role(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }

    /// Whether the species can move in a mode.
    pub fn can(&self, mode: Locomotion) -> bool {
        self.locomotion.contains(&mode)
    }

    /// Diet entry for a food, if any.
    pub fn diet_entry(&self, food: &Food) -> Option<&DietEntry> {
        self.diet.iter().find(|d| &d.food == food)
    }

    /// Whether the species eats any live prey (individual animals or
    /// density-field organisms).
    pub fn hunts(&self) -> bool {
        self.diet.iter().any(|d| matches!(d.food, Food::Prey(_)))
    }

    /// Whether the species is simulated as individual agents.
    pub fn is_individual(&self) -> bool {
        self.representation.is_individual()
    }
}

#[cfg(test)]
mod tests;
