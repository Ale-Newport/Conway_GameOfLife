//! Individual animals: state, decisions and physiology.
//!
//! Each tick the animals go through the five phases supported by
//! `organism-engine` (the tick pipeline is described in `docs/ECOSYSTEM.md`):
//!
//! 1. **perceive + decide** ([`decide`]) — pure, in parallel: utility AI
//!    with species-specific considerations chooses an action and a steering
//!    acceleration;
//! 2. **move** — kinematics with sub-steps and passability per species;
//! 3. **interact** ([`interact`]) — claims for prey, carcasses, plants,
//!    mates, resolved deterministically (exclusive per prey, shared per food,
//!    one-to-one matching per mate);
//! 4. **physiology** ([`physiology`]) — metabolism, drives, growth, ageing,
//!    stress, pregnancy, births and deaths;
//! 5. **commit** — births appended, deaths removed (carcasses created).

pub mod decide;
pub mod genetics;
mod hazards;
pub mod interact;
pub mod perceive;
pub mod physiology;
pub mod reproduction;
pub mod seasonal;
pub mod social;

use organism_engine::steering::WanderState;
use organism_engine::{AgentId, Genome, Kinematics, Lineage};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

use crate::disease::Infection;
use crate::species::AnimalTraits;
use crate::units::{ANIMAL_NUTRIENT_G_PER_KG, TISSUE_KJ_PER_KG};

/// Biological sex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    /// Bears young (or lays eggs).
    Female,
    /// Fertilises females.
    Male,
}

/// What an animal is doing (also selects the animation clip).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// No decision yet.
    Idle,
    /// Exploring.
    Wander,
    /// Eating grass, herbs, moss or algae.
    Graze,
    /// Eating shrubs or tree foliage.
    Browse,
    /// Feeding on nectar, litter or mushrooms.
    Forage,
    /// Pursuing prey.
    Hunt,
    /// Eating a carcass.
    Feed,
    /// Escaping a predator, fire or flood.
    Flee,
    /// Drinking or walking to water.
    Drink,
    /// Resting.
    Rest,
    /// Sleeping.
    Sleep,
    /// Looking for or courting a mate.
    SeekMate,
    /// Young following its mother.
    FollowParent,
    /// Guarding / incubating a clutch.
    Incubate,
    /// Too cold to move (ectotherms).
    Torpor,
    /// Chasing an intruder out of its territory.
    Defend,
    /// Travelling towards its preferred climate.
    Migrate,
    /// Dormant in its den for the cold season.
    Hibernate,
}

impl Action {
    /// Stable machine key.
    pub fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Wander => "wander",
            Self::Graze => "graze",
            Self::Browse => "browse",
            Self::Forage => "forage",
            Self::Hunt => "hunt",
            Self::Feed => "feed",
            Self::Flee => "flee",
            Self::Drink => "drink",
            Self::Rest => "rest",
            Self::Sleep => "sleep",
            Self::SeekMate => "seek_mate",
            Self::FollowParent => "follow_parent",
            Self::Incubate => "incubate",
            Self::Torpor => "torpor",
            Self::Defend => "defend",
            Self::Migrate => "migrate",
            Self::Hibernate => "hibernate",
        }
    }
}

/// Target of the current action.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// None.
    #[default]
    None,
    /// Another animal (prey, threat, mate, mother).
    Animal(AgentId),
    /// A carcass.
    Carcass(u64),
    /// A cell (food, water).
    Cell(u32),
    /// A point (refuge, wander goal).
    Point(Vec2),
    /// A clutch of eggs.
    Clutch(u64),
}

/// Why an animal died.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeathCause {
    /// Killed by a predator.
    Predation,
    /// Energy reserves exhausted.
    Starvation,
    /// Hydration exhausted.
    Dehydration,
    /// Gompertz ageing.
    OldAge,
    /// Burned.
    Fire,
    /// Drowned (non-swimmer in deep water or flood).
    Drowning,
    /// Temperature outside the tolerance range.
    Exposure,
    /// Injuries from fights and failed hunts.
    Injury,
    /// Killed by a command.
    Killed,
    /// Killed by an infectious disease.
    Disease,
}

impl DeathCause {
    /// Stable machine key.
    pub fn key(self) -> &'static str {
        match self {
            Self::Predation => "predation",
            Self::Starvation => "starvation",
            Self::Dehydration => "dehydration",
            Self::OldAge => "old_age",
            Self::Fire => "fire",
            Self::Drowning => "drowning",
            Self::Exposure => "exposure",
            Self::Injury => "injury",
            Self::Killed => "killed",
            Self::Disease => "disease",
        }
    }
}

/// Reproductive state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReproState {
    /// Not yet mature.
    Juvenile,
    /// Mature and available.
    Ready,
    /// Pregnant (live birth) or forming eggs.
    Gestating(Box<Pregnancy>),
    /// Waiting for the inter-birth interval to pass.
    Recovering {
        /// Tick at which the animal is ready again.
        until: u64,
    },
}

/// An ongoing pregnancy (or egg formation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pregnancy {
    /// Tick of conception.
    pub conceived: u64,
    /// Tick of birth (or laying).
    pub due: u64,
    /// Planned litter / clutch size.
    pub litter: u16,
    /// Father (if sexual).
    pub father: Option<AgentId>,
    /// Father's lineage (kept for the genealogy even if he dies).
    pub father_lineage: Option<Lineage>,
    /// Father's genome.
    pub father_genome: Option<Genome>,
    /// Energy invested in the embryos so far (kJ).
    pub embryo_kj: f64,
    /// Nutrients invested in the embryos so far (g).
    pub embryo_g: f64,
}

/// Counters of an animal's life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LifeStats {
    /// Prey killed.
    pub kills: u32,
    /// Offspring born (or hatched).
    pub offspring: u32,
    /// Distance travelled (cells).
    pub distance_cells: f32,
    /// Gross energy eaten (kJ).
    pub eaten_kj: f64,
    /// Successful escapes from attacks.
    pub escapes: u32,
}

/// One animal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Animal {
    /// Species index into [`crate::SpeciesCatalog::animals`].
    pub species: u16,
    /// Sex.
    pub sex: Sex,
    /// Position, velocity and heading (cells, cells per tick).
    pub kin: Kinematics,
    /// Wander behaviour state.
    pub wander: WanderState,
    /// Tick of birth; founders created with an age have a negative birth tick.
    pub birth_tick: i64,
    /// Body mass (kg wet).
    pub mass_kg: f32,
    /// Energy reserve (kJ). In the NetLogo rule variant: NetLogo energy units.
    pub energy_kj: f32,
    /// Nutrient store beyond the structural content of the body (g).
    pub nutrient_store_g: f32,
    /// Hydration in `[0, 1]` (1 = fully hydrated).
    pub hydration: f32,
    /// Sleep pressure in `[0, 1]`.
    pub fatigue: f32,
    /// Sprint stamina in `[0, 1]`.
    pub stamina: f32,
    /// Health in `[0, 1]`; the animal dies at 0.
    pub health: f32,
    /// Thermal comfort of the last tick in `[0, 1]`.
    pub comfort: f32,
    /// Reproductive state.
    pub repro: ReproState,
    /// Heritable traits.
    pub genome: Genome,
    /// Ancestry.
    pub lineage: Lineage,
    /// Current action.
    pub action: Action,
    /// Current target.
    pub target: Target,
    /// Tick at which the current action started.
    pub action_since: u64,
    /// Burrow / den position (burrowing species).
    pub home: Option<Vec2>,
    /// Hidden in its burrow (invisible to predators).
    pub hidden: bool,
    /// Ticks spent in water deeper than it can wade.
    pub drowning_ticks: u16,
    /// Consecutive ticks in which it barely moved although it wanted to
    /// (blocked by terrain); decisions then add an escape manoeuvre.
    pub stuck_ticks: u16,
    /// Utility scores of the last decision (see [`decide::ACTIONS`]).
    pub last_scores: [f32; decide::ACTION_COUNT],
    /// Life counters.
    pub stats: LifeStats,
    /// Height above the ground (m): fliers climb to their cruising altitude
    /// while travelling and land to feed, drink, rest and breed.
    pub altitude_m: f32,
    /// Injury severity in `[0, 1]` (failed hunts, fights): slows the animal
    /// by `1 − 0.6 injury` and heals while it is fed and watered.
    pub injury: f32,
    /// Infection with a pathogen (infected or recovered and immune).
    pub infection: Option<Infection>,
    /// Social group (herd, pack, flock, school): founders placed together
    /// share one id and young inherit their mother's.
    pub group: Option<u64>,
    /// Alarm call of this animal: the position of the threat and the
    /// strength of the call (1 for a predator seen first-hand, less for a
    /// relayed call).
    pub alarm: Option<(Vec2, f32)>,
}

impl Animal {
    /// A new animal with neutral condition (full hydration, health and
    /// stamina, no reserves, juvenile); callers set mass, reserves and the
    /// reproductive state.
    pub fn new(species: u16, sex: Sex, kin: Kinematics, birth_tick: i64, genome: Genome, lineage: Lineage) -> Self {
        Self {
            species,
            sex,
            kin,
            wander: WanderState::default(),
            birth_tick,
            mass_kg: 0.0,
            energy_kj: 0.0,
            nutrient_store_g: 0.0,
            hydration: 1.0,
            fatigue: 0.0,
            stamina: 1.0,
            health: 1.0,
            comfort: 1.0,
            repro: ReproState::Juvenile,
            genome,
            lineage,
            action: Action::Idle,
            target: Target::None,
            action_since: birth_tick.max(0) as u64,
            home: None,
            hidden: false,
            drowning_ticks: 0,
            stuck_ticks: 0,
            last_scores: [0.0; decide::ACTION_COUNT],
            stats: LifeStats::default(),
            altitude_m: 0.0,
            injury: 0.0,
            infection: None,
            group: None,
            alarm: None,
        }
    }

    /// Speed multiplier of injuries and sickness.
    pub fn impairment(&self) -> f32 {
        (1.0 - 0.6 * self.injury.clamp(0.0, 1.0)).max(0.2)
    }

    /// Age in ticks.
    pub fn age_ticks(&self, tick: u64) -> u64 {
        (tick as i64).saturating_sub(self.birth_tick).max(0) as u64
    }

    /// Whether the animal is an adult.
    pub fn is_adult(&self) -> bool {
        !matches!(self.repro, ReproState::Juvenile)
    }

    /// Whether the animal is pregnant (or forming eggs).
    pub fn is_pregnant(&self) -> bool {
        matches!(self.repro, ReproState::Gestating(_))
    }

    /// Energy reserve capacity at the current mass (kJ).
    pub fn reserve_max_kj(&self, traits: &AnimalTraits) -> f32 {
        traits.reserve_max_kj * (self.mass_kg / traits.adult_mass_kg).max(0.01).powf(0.75)
    }

    /// Field metabolic rate at the current mass (kJ/day): the adult rate
    /// scaled by `(m/M)^{3/4}`. Water gained from food and milk is measured
    /// against this own daily need.
    pub fn field_kj_per_day(&self, traits: &AnimalTraits) -> f32 {
        traits.field_kj_per_day * (self.mass_kg / traits.adult_mass_kg).max(0.01).powf(0.75)
    }

    /// Hunger in `[0, 1]`: `1 − reserve / capacity`.
    pub fn hunger(&self, traits: &AnimalTraits) -> f32 {
        (1.0 - self.energy_kj / self.reserve_max_kj(traits).max(1e-6)).clamp(0.0, 1.0)
    }

    /// Thirst in `[0, 1]`.
    pub fn thirst(&self) -> f32 {
        (1.0 - self.hydration).clamp(0.0, 1.0)
    }

    /// Energy (kJ) of the body, reserves and embryos.
    pub fn energy_stock_kj(&self) -> f64 {
        let embryo = match &self.repro {
            ReproState::Gestating(p) => p.embryo_kj,
            _ => 0.0,
        };
        f64::from(self.mass_kg) * TISSUE_KJ_PER_KG + f64::from(self.energy_kj.max(0.0)) + embryo
    }

    /// Nutrients (g) of the body, the store and embryos.
    pub fn nutrient_stock_g(&self) -> f64 {
        let embryo = match &self.repro {
            ReproState::Gestating(p) => p.embryo_g,
            _ => 0.0,
        };
        f64::from(self.mass_kg * ANIMAL_NUTRIENT_G_PER_KG) + f64::from(self.nutrient_store_g.max(0.0)) + embryo
    }

    /// Nutrient store capacity (g): 10 % of the structural content.
    pub fn store_capacity_g(&self) -> f32 {
        0.1 * self.mass_kg * ANIMAL_NUTRIENT_G_PER_KG
    }
}

/// Reference to an organism for selection, following and inspection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganismRef {
    /// An animal id.
    Animal(u64),
    /// A tree id.
    Tree(u64),
}
