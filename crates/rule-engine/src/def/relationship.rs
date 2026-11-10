//! Relationships between entity types (and resources).
//!
//! A relationship is a typed directed edge `subject --kind--> object`. The
//! relationship compiler (in [`mod@crate::compile`]) turns every edge
//! into ordinary rules whose text is shown to the user; see
//! `docs/RULE_ENGINE.md` for the exact templates.

use serde::{Deserialize, Serialize};

fn default_radius() -> f64 {
    5.0
}

fn one() -> f64 {
    1.0
}

fn default_damage() -> f64 {
    5.0
}

fn default_speed_factor() -> f64 {
    1.0
}

/// What makes a `transforms` relationship fire.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum TransformTrigger {
    /// Every tick (subject to the probability).
    #[default]
    Always,
    /// When a DSL condition holds, e.g. `"energy > 50 and age > 20"`.
    Condition(String),
    /// When touching an entity of the given type.
    Contact(String),
    /// When the entity has spent at least this many ticks in its current
    /// type and state.
    AfterTicks(u64),
}

/// The kind of a relationship and its parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RelationKind {
    /// The subject eats the object (an entity type) or feeds on it (a
    /// resource field).
    Consumes {
        /// Fraction of the prey's energy the eater gains (entity objects).
        #[serde(default = "one")]
        efficiency: f64,
        /// The subject only forages/eats while its energy is below this.
        #[serde(default)]
        hunger: Option<f64>,
        /// Sensing radius used to find food.
        #[serde(default = "default_radius")]
        radius: f64,
        /// Amount taken per tick from a resource field.
        #[serde(default = "one")]
        bite: f64,
        /// Handling time: minimum ticks between two meals (the cooldown of
        /// the generated eat rule; Holling's type II functional response).
        #[serde(default)]
        handling: u64,
    },
    /// The subject defends the object against the object's enemies (types
    /// that consume or chase it, or that it avoids).
    Protects {
        /// Guarding radius.
        #[serde(default = "default_radius")]
        radius: f64,
        /// Energy removed from an attacked enemy.
        #[serde(default = "default_damage")]
        damage: f64,
    },
    /// The subject pursues the object without eating it.
    Chases {
        /// Sensing radius.
        #[serde(default = "default_radius")]
        radius: f64,
        /// Speed multiplier while chasing.
        #[serde(default = "default_speed_factor")]
        speed_factor: f64,
    },
    /// The subject flees from the object.
    Avoids {
        /// Sensing radius.
        #[serde(default = "default_radius")]
        radius: f64,
        /// Speed multiplier while fleeing.
        #[serde(default = "default_speed_factor")]
        speed_factor: f64,
    },
    /// The subject turns into the object type.
    Transforms {
        /// What triggers the transformation.
        #[serde(default)]
        trigger: TransformTrigger,
        /// Chance per tick once triggered.
        #[serde(default = "one")]
        probability: f64,
    },
    /// The subject releases a resource or spawns entities of the object.
    Produces {
        /// Resource amount or number of entities per firing.
        #[serde(default = "one")]
        amount: f64,
        /// Chance per tick.
        #[serde(default = "one")]
        probability: f64,
        /// Energy the subject pays per firing.
        #[serde(default)]
        cost: f64,
    },
    /// Sexual reproduction between subject and object types.
    ReproducesWith {
        /// Radius in which a partner is searched.
        #[serde(default = "default_radius")]
        radius: f64,
        /// Chance per tick when a partner is available.
        #[serde(default = "one")]
        probability: f64,
        /// Minimum energy of the initiating parent (in addition to the
        /// reproduction cost).
        #[serde(default)]
        min_energy: f64,
    },
    /// Interference competition: the subject attacks the object on contact
    /// (and vice versa; the relation is symmetric).
    CompetesWith {
        /// Probability of attacking on contact.
        #[serde(default = "one")]
        aggression: f64,
        /// Energy removed per attack.
        #[serde(default = "default_damage")]
        damage: f64,
    },
    /// The subject trades carried resources with the object type.
    TradesWith {
        /// Resource given.
        give: String,
        /// Amount given.
        #[serde(default = "one")]
        give_amount: f64,
        /// Resource received.
        receive: String,
        /// Amount received.
        #[serde(default = "one")]
        receive_amount: f64,
        /// Sensing radius used to find a partner.
        #[serde(default = "default_radius")]
        radius: f64,
    },
}

impl RelationKind {
    /// Short label (`"consumes"`, `"avoids"`...).
    pub fn label(&self) -> &'static str {
        match self {
            Self::Consumes { .. } => "consumes",
            Self::Protects { .. } => "protects",
            Self::Chases { .. } => "chases",
            Self::Avoids { .. } => "avoids",
            Self::Transforms { .. } => "transforms",
            Self::Produces { .. } => "produces",
            Self::ReproducesWith { .. } => "reproduces_with",
            Self::CompetesWith { .. } => "competes_with",
            Self::TradesWith { .. } => "trades_with",
        }
    }

    /// Parameters as short text for graph labels and tooltips, e.g.
    /// `"efficiency 0.5, radius 6, handling 10"` (defaults included).
    pub fn summary(&self) -> String {
        let n = crate::dsl::printer::print_number;
        match self {
            Self::Consumes { efficiency, hunger, radius, bite, handling } => {
                let hunger = hunger.map_or_else(|| "always".to_string(), |h| format!("below {}", n(h)));
                format!(
                    "efficiency {}, hunger {hunger}, radius {}, bite {}, handling {handling}",
                    n(*efficiency),
                    n(*radius),
                    n(*bite)
                )
            }
            Self::Protects { radius, damage } => format!("radius {}, damage {}", n(*radius), n(*damage)),
            Self::Chases { radius, speed_factor } | Self::Avoids { radius, speed_factor } => {
                format!("radius {}, speed ×{}", n(*radius), n(*speed_factor))
            }
            Self::Transforms { trigger, probability } => {
                let trigger = match trigger {
                    TransformTrigger::Always => "always".to_string(),
                    TransformTrigger::Condition(c) => format!("when {c}"),
                    TransformTrigger::Contact(t) => format!("on contact with {t}"),
                    TransformTrigger::AfterTicks(t) => format!("after {t} ticks"),
                };
                format!("{trigger}, probability {}", n(*probability))
            }
            Self::Produces { amount, probability, cost } => {
                format!("amount {}, probability {}, cost {}", n(*amount), n(*probability), n(*cost))
            }
            Self::ReproducesWith { radius, probability, min_energy } => {
                format!("radius {}, probability {}, min energy {}", n(*radius), n(*probability), n(*min_energy))
            }
            Self::CompetesWith { aggression, damage } => {
                format!("aggression {}, damage {}", n(*aggression), n(*damage))
            }
            Self::TradesWith { give, give_amount, receive, receive_amount, radius } => {
                format!("gives {} {give} for {} {receive}, radius {}", n(*give_amount), n(*receive_amount), n(*radius))
            }
        }
    }

    /// Whether the relation acts in both directions (drawn without an arrow head).
    pub fn is_symmetric(&self) -> bool {
        matches!(self, Self::CompetesWith { .. })
    }

    /// Base priority band of the main generated rule.
    pub fn priority_band(&self) -> i32 {
        match self {
            Self::Avoids { .. } => 900,
            Self::Protects { .. } => 700,
            Self::Transforms { .. } => 600,
            Self::Consumes { .. } => 500,
            Self::Chases { .. } => 450,
            Self::CompetesWith { .. } => 400,
            Self::ReproducesWith { .. } => 300,
            Self::TradesWith { .. } => 200,
            Self::Produces { .. } => 100,
        }
    }
}

/// A typed edge `subject --kind--> object`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    /// Entity type that acts.
    pub subject: String,
    /// Kind and parameters.
    pub kind: RelationKind,
    /// Entity type or resource acted upon.
    pub object: String,
    /// Shift of the generated rules' priorities within their band (±99).
    #[serde(default)]
    pub priority_shift: i32,
}
