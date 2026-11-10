//! Decision-phase outputs: evaluated actions ([`Act`]) and the per-rule
//! bundles ([`Firing`]) the resolution phase applies.

use organism_engine::AgentId;
use simulation_core::SimRng;
use simulation_core::glam::Vec2;

use crate::compile::ir::{PropRef, Slot};
use crate::sim::frame::Target;

/// An action with its arguments evaluated, produced by a DSL rule or
/// recorded by a script.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Steer towards a target, arriving without overshoot.
    MoveTowards {
        /// Target.
        target: Target,
        /// Speed.
        speed: f32,
    },
    /// Flee from a target.
    MoveAway {
        /// Target.
        target: Target,
        /// Speed.
        speed: f32,
    },
    /// Correlated random-walk step.
    Wander {
        /// Maximum turn (radians).
        turn: f32,
        /// Speed.
        speed: f32,
    },
    /// Stand still.
    Stay,
    /// Eat a touched entity.
    Eat {
        /// Prey.
        target: Target,
        /// Fraction of the prey's energy gained.
        efficiency: f64,
    },
    /// Damage a touched entity.
    Attack {
        /// Victim.
        target: Target,
        /// Energy removed.
        damage: f64,
    },
    /// Reproduce.
    Reproduce {
        /// Chance.
        probability: f64,
        /// Partner for sexual reproduction.
        partner: Option<Target>,
    },
    /// Become another type.
    Transform(u16),
    /// Die.
    Die,
    /// Exchange carried resources.
    Trade {
        /// Partner.
        target: Target,
        /// Resource field given.
        give: u16,
        /// Amount given.
        amount: f64,
        /// Resource field received.
        receive: u16,
        /// Amount received.
        receive_amount: f64,
    },
    /// Release a resource.
    Emit {
        /// Resource field.
        resource: u16,
        /// Amount.
        amount: f64,
    },
    /// Take a resource from the cell.
    Consume {
        /// Resource field.
        resource: u16,
        /// Amount.
        amount: f64,
    },
    /// Set a property.
    SetProp {
        /// Property.
        prop: PropRef,
        /// Value.
        value: f64,
    },
    /// Add to a property.
    AddProp {
        /// Property.
        prop: PropRef,
        /// Amount.
        value: f64,
    },
    /// Enter a state.
    SetState(u16),
    /// Create entities.
    Spawn {
        /// Type.
        ty: u16,
        /// Count.
        count: u32,
    },
}

impl Act {
    /// Slot occupied by the action.
    pub fn slot(&self) -> Slot {
        match self {
            Self::MoveTowards { .. } | Self::MoveAway { .. } | Self::Wander { .. } | Self::Stay => Slot::Move,
            Self::Eat { .. } | Self::Attack { .. } | Self::Reproduce { .. } | Self::Die | Self::Trade { .. } => {
                Slot::Act
            }
            Self::Transform(_) => Slot::Transform,
            _ => Slot::Effect,
        }
    }
}

/// Where a firing comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// DSL rule index.
    Rule(u32),
    /// Script index.
    Script(u32),
    /// The type's default movement.
    DefaultMove,
}

/// Desired motion: an acceleration for `Kinematics::integrate` and the
/// speed limit to integrate with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// Desired acceleration.
    pub accel: Vec2,
    /// Speed limit.
    pub speed: f32,
}

/// Exclusive action after decision.
#[derive(Clone, Debug, PartialEq)]
pub enum ActIntent {
    /// Eat (contested on the prey).
    Eat {
        /// Prey slot.
        target: u32,
        /// Distance (claim ranking).
        distance: f32,
        /// Energy fraction gained.
        efficiency: f64,
    },
    /// Attack (contested on the victim).
    Attack {
        /// Victim slot.
        target: u32,
        /// Distance.
        distance: f32,
        /// Damage.
        damage: f64,
    },
    /// Sexual reproduction (contested on the partner).
    Mate {
        /// Partner slot.
        partner: u32,
        /// Distance.
        distance: f32,
    },
    /// Trade (contested on the partner).
    Trade {
        /// Partner slot.
        target: u32,
        /// Distance.
        distance: f32,
        /// Given resource.
        give: u16,
        /// Given amount.
        amount: f64,
        /// Received resource.
        receive: u16,
        /// Received amount.
        receive_amount: f64,
    },
    /// Asexual reproduction.
    Reproduce,
    /// Die.
    Die,
}

impl ActIntent {
    /// The other party of a contested interaction and the distance to it.
    pub fn counterpart(&self) -> Option<(u32, f32)> {
        match self {
            Self::Eat { target, distance, .. }
            | Self::Attack { target, distance, .. }
            | Self::Trade { target, distance, .. } => Some((*target, *distance)),
            Self::Mate { partner, distance } => Some((*partner, *distance)),
            _ => None,
        }
    }
}

/// Non-exclusive effect.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Release from carried stock (if carried) or create.
    Emit {
        /// Resource field.
        resource: u16,
        /// Amount.
        amount: f64,
    },
    /// Create a resource at the entity's cell (rule products).
    Release {
        /// Resource field.
        resource: u16,
        /// Amount.
        amount: f64,
    },
    /// Take from the entity's cell (shared proportionally when contested).
    Consume {
        /// Resource field.
        resource: u16,
        /// Amount.
        amount: f64,
    },
    /// Set a property.
    SetProp {
        /// Property.
        prop: PropRef,
        /// Value.
        value: f64,
    },
    /// Add to a property.
    AddProp {
        /// Property.
        prop: PropRef,
        /// Amount.
        value: f64,
    },
    /// Enter a state.
    SetState(u16),
    /// Create entities.
    Spawn {
        /// Type.
        ty: u16,
        /// Count.
        count: u32,
    },
}

/// Everything one rule (or script call) decided for one entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Firing {
    /// Acting entity's slot.
    pub slot: u32,
    /// Origin.
    pub source: Source,
    /// Priority (claim ranking).
    pub priority: i32,
    /// Costs charged if the firing survives resolution.
    pub costs: Vec<(PropRef, f64)>,
    /// `(index in the type's rule list, next allowed tick)`.
    pub cooldown: Option<(usize, u64)>,
    /// Movement.
    pub motion: Option<Motion>,
    /// Exclusive action.
    pub act: Option<ActIntent>,
    /// Type to turn into at the end of the tick.
    pub transform: Option<u16>,
    /// Effects in action order.
    pub effects: Vec<Effect>,
}

impl Firing {
    /// Entities this firing creates (offspring and spawns), for capacity checks.
    pub fn births(&self, litter: u32) -> u64 {
        let spawns: u64 =
            self.effects.iter().map(|e| if let Effect::Spawn { count, .. } = e { u64::from(*count) } else { 0 }).sum();
        let offspring = match self.act {
            Some(ActIntent::Reproduce | ActIntent::Mate { .. }) => u64::from(litter),
            _ => 0,
        };
        spawns + offspring
    }
}

/// splitmix64 finaliser.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Random stream keyed by (seed, tick, entity, stream): decisions do not
/// depend on evaluation order or thread count.
pub fn keyed_rng(seed: u64, tick: u64, id: AgentId, stream: u64) -> SimRng {
    SimRng::seed_from_u64(mix(mix(mix(seed) ^ tick) ^ id.get()) ^ mix(stream))
}

/// Stream key of DSL rule `index`.
pub fn rule_stream(index: u32) -> u64 {
    1 + u64::from(index)
}

/// Stream key of script `index` (what the script's `random()` draws).
pub fn script_stream(index: u32) -> u64 {
    (1 << 32) + u64::from(index)
}

/// Stream key used to carry out the intents recorded by script `index`
/// (wander turns, reproduction rolls). Separate from [`script_stream`] so
/// that these draws are independent of the script's own `random()` calls.
pub fn script_realize_stream(index: u32) -> u64 {
    (1 << 33) + u64::from(index)
}

/// Stream key of the default movement.
pub const DEFAULT_MOVE_STREAM: u64 = 1 << 40;

/// Stream key used to place offspring and spawned entities.
pub const PLACEMENT_STREAM: u64 = 1 << 41;

/// Stream key of the initial headings of entities born in a tick (plus the
/// birth's index among the parent's births of that tick).
pub const HEADING_STREAM: u64 = 1 << 42;
