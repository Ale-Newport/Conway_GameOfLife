//! Behavioural sections of a species record: group structure, daily rhythm,
//! hunting technique, utility-AI weights and response curves, fighting
//! ability and thermal niche.

use organism_engine::Curve;
use serde::{Deserialize, Serialize};

/// Group structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SocialStructure {
    /// Lives alone.
    #[default]
    Solitary,
    /// Breeding pairs.
    Pair,
    /// Herds of grazers.
    Herd,
    /// Cooperative hunting packs.
    Pack,
    /// Bird flocks.
    Flock,
    /// Fish schools.
    School,
    /// Colonies (warrens, roosts, nests).
    Colony,
}

/// Social behaviour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Social {
    /// Group structure.
    pub structure: SocialStructure,
    /// Typical group size `(min, max)`.
    pub group_size: (u16, u16),
    /// Weight of boids cohesion (steering towards nearby conspecifics) in `[0, 1]`.
    pub cohesion: f32,
    /// Weight of boids alignment (matching the heading of nearby
    /// conspecifics) in `[0, 1]`; flocks and schools align strongly, herds
    /// weakly. `None` uses the cohesion weight (part-1 behaviour).
    pub alignment: Option<f32>,
    /// Defends a territory against adult conspecifics of other groups.
    pub territorial: bool,
    /// Territory radius (m).
    pub territory_radius_m: f32,
    /// Gives alarm calls: an animal that flees from a predator calls, and
    /// conspecifics that hear it flee from the caller's threat although they
    /// have not seen it (and call in turn, so the alarm propagates).
    pub alarm_calls: bool,
}

impl Social {
    /// Effective alignment weight.
    pub fn alignment_weight(&self) -> f32 {
        self.alignment.unwrap_or(self.cohesion)
    }

    /// Whether group members follow a leader and share its hunting target
    /// (packs and prides).
    pub fn is_pack(&self) -> bool {
        self.structure == SocialStructure::Pack
    }
}

impl Default for Social {
    fn default() -> Self {
        Self {
            structure: SocialStructure::Solitary,
            group_size: (1, 1),
            cohesion: 0.0,
            alignment: None,
            territorial: false,
            territory_radius_m: 0.0,
            alarm_calls: false,
        }
    }
}

/// Daily activity rhythm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityPattern {
    /// Active by day.
    #[default]
    Diurnal,
    /// Active by night.
    Nocturnal,
    /// Active at dawn and dusk.
    Crepuscular,
    /// Active at any time.
    Cathemeral,
}

/// Hunting technique of predators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HuntingStyle {
    /// Does not hunt.
    #[default]
    None,
    /// Waits motionless until prey comes close, then strikes (surprise bonus).
    Ambush,
    /// Approaches slowly, then sprints over a short distance.
    Stalk,
    /// Chases over long distances (needs endurance).
    Coursing,
}

/// Weights of the utility-AI actions (0 disables an action).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ActionWeights {
    /// Grazing, browsing and other foraging.
    pub forage: f32,
    /// Hunting live prey.
    pub hunt: f32,
    /// Eating carrion.
    pub scavenge: f32,
    /// Fleeing threats.
    pub flee: f32,
    /// Drinking.
    pub drink: f32,
    /// Resting.
    pub rest: f32,
    /// Sleeping.
    pub sleep: f32,
    /// Seeking a mate.
    pub mate: f32,
    /// Wandering / exploring.
    pub wander: f32,
    /// Chasing intruders out of the territory (territorial species).
    pub defend: f32,
    /// Seasonal migration towards the preferred climate (migratory species).
    pub migrate: f32,
}

impl Default for ActionWeights {
    fn default() -> Self {
        Self {
            forage: 1.0,
            hunt: 1.0,
            scavenge: 0.7,
            flee: 3.0,
            drink: 1.2,
            rest: 0.5,
            sleep: 1.0,
            mate: 1.0,
            wander: 0.15,
            defend: 1.0,
            migrate: 1.5,
        }
    }
}

/// Species-specific decision making.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Behaviour {
    /// Daily rhythm.
    pub activity: ActivityPattern,
    /// Action weights.
    pub weights: ActionWeights,
    /// Response of foraging / hunting to hunger.
    pub hunger_curve: Curve,
    /// Response of drinking to thirst.
    pub thirst_curve: Curve,
    /// Response of fleeing to threat level.
    pub fear_curve: Curve,
    /// Response of sleeping to fatigue.
    pub fatigue_curve: Curve,
    /// Boldness in `[0, 1]`: lowers perceived threat.
    pub boldness: f32,
    /// Vigilance in `[0, 1]`: fraction of vision kept while foraging.
    pub vigilance: f32,
    /// Hunting technique.
    pub hunting: HuntingStyle,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self {
            activity: ActivityPattern::Diurnal,
            weights: ActionWeights::default(),
            hunger_curve: Curve::Logistic { k: 10.0, x0: 0.45 },
            thirst_curve: Curve::Logistic { k: 12.0, x0: 0.5 },
            fear_curve: Curve::Logistic { k: 10.0, x0: 0.3 },
            fatigue_curve: Curve::Logistic { k: 10.0, x0: 0.6 },
            boldness: 0.2,
            vigilance: 0.5,
            hunting: HuntingStyle::None,
        }
    }
}

/// Fighting ability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Combat {
    /// Attack strength (dimensionless, scaled by `M^{1/3}`).
    pub attack: f32,
    /// Defence strength (dimensionless, scaled by `M^{1/3}`).
    pub defence: f32,
    /// Aggression in `[0, 1]` (territorial and intraspecific disputes).
    pub aggression: f32,
}

impl Default for Combat {
    fn default() -> Self {
        Self { attack: 0.1, defence: 0.2, aggression: 0.0 }
    }
}

/// Thermal niche (air temperature, °C).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Climate {
    /// Comfort range `(low, high)`.
    pub preferred_c: (f32, f32),
    /// Survivable range `(low, high)`; stress outside, damage beyond.
    pub tolerance_c: (f32, f32),
}
