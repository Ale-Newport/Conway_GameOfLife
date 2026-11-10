//! Runtime IR produced by validation: names resolved to indices, types
//! checked, sensing radii fixed. Evaluation of this IR cannot fail (a missing
//! target makes the rule ineligible; division by zero yields 0).

use serde::{Deserialize, Serialize};

use crate::dsl::ast::RuleAst;

/// Maximum sensing radius accepted by `nearby`/`nearest` (world units).
pub const MAX_SENSE_RADIUS: f32 = 32.0;

/// Maximum number of entities one `spawn` action or product creates.
pub const MAX_SPAWN_PER_ACTION: u32 = 100;

/// A numeric property of an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PropRef {
    /// Energy (read-write).
    Energy,
    /// Ticks since birth (read-only).
    Age,
    /// Ticks since the entity entered its current type and state (read-only).
    StateAge,
    /// Diameter (read-write).
    Size,
    /// Movement speed (read-write).
    Speed,
    /// Position x (read-only).
    X,
    /// Position y (read-only).
    Y,
    /// Generation: 0 for initial entities, parent + 1 for offspring (read-only).
    Generation,
    /// Stable id (read-only).
    Id,
    /// User property by index in the type's property list (read-write).
    User(u16),
    /// Carried resource by index in the type's `carries` list (read-only;
    /// changed by `consume`, `emit` and `trade`).
    Carried(u16),
}

impl PropRef {
    /// Built-in property names, in documentation order.
    pub const BUILTIN: [(&'static str, PropRef); 9] = [
        ("energy", PropRef::Energy),
        ("age", PropRef::Age),
        ("state_age", PropRef::StateAge),
        ("size", PropRef::Size),
        ("speed", PropRef::Speed),
        ("x", PropRef::X),
        ("y", PropRef::Y),
        ("generation", PropRef::Generation),
        ("id", PropRef::Id),
    ];

    /// Looks up a built-in property.
    pub fn builtin(name: &str) -> Option<Self> {
        Self::BUILTIN.iter().find(|(n, _)| *n == name).map(|(_, p)| *p)
    }

    /// Whether rules may write the property with `set`/`add`/`cost`.
    pub fn is_writable(self) -> bool {
        matches!(self, Self::Energy | Self::Size | Self::Speed | Self::User(_))
    }
}

/// Whose property is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    /// The acting entity (`self`).
    Me,
    /// The touched entity of a collision rule (`other`).
    Other,
}

/// Entity selector of sensors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KindSel {
    /// Any entity type.
    Any,
    /// The acting entity's own type.
    Same,
    /// A specific type.
    Type(u16),
}

/// A readable field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldRef {
    /// Resource field index.
    Resource(u16),
    /// Environment field index.
    Env(u16),
}

/// Numeric comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CmpOp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `==`
    Eq,
    /// `!=`
    Ne,
}

impl CmpOp {
    /// Applies the comparison (false whenever a NaN is involved, except `!=`).
    pub fn apply(self, a: f64, b: f64) -> bool {
        match self {
            Self::Lt => a < b,
            Self::Le => a <= b,
            Self::Gt => a > b,
            Self::Ge => a >= b,
            Self::Eq => a == b,
            Self::Ne => a != b,
        }
    }
}

/// Arithmetic operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArithOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/` (x / 0 = 0)
    Div,
}

/// Typed runtime expression. Numeric, boolean and entity-valued variants
/// share one enum; validation guarantees each is used in the right place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RExpr {
    /// Number literal.
    Num(f64),
    /// Boolean literal.
    Bool(bool),
    /// Property of `self` or `other`.
    Prop(Subject, PropRef),
    /// Current tick.
    Tick,
    /// `subject.state == name` (`negate` for `!=`); `name` is a global state-name id.
    StateIs(Subject, u32, bool),
    /// `subject.type == T` (`negate` for `!=`).
    TypeIs(Subject, u16, bool),
    /// Number of matching entities within `radius` (excluding self).
    Nearby(KindSel, f32),
    /// Global population of matching entities at the start of the tick.
    Population(KindSel),
    /// Field value at the entity's cell.
    Field(FieldRef),
    /// Uniform number in `[0, 1)`.
    Random,
    /// `true` with the given probability.
    Chance(Box<RExpr>),
    /// Distance to an entity (`inf` when there is none).
    Distance(Box<RExpr>),
    /// Whether an entity expression found something.
    Exists(Box<RExpr>),
    /// `min(a, b)`
    Min(Box<RExpr>, Box<RExpr>),
    /// `max(a, b)`
    Max(Box<RExpr>, Box<RExpr>),
    /// `abs(a)`
    Abs(Box<RExpr>),
    /// `clamp(x, lo, hi)`
    Clamp(Box<RExpr>, Box<RExpr>, Box<RExpr>),
    /// `-x`
    Neg(Box<RExpr>),
    /// `not x`
    Not(Box<RExpr>),
    /// Arithmetic.
    Arith(ArithOp, Box<RExpr>, Box<RExpr>),
    /// Numeric comparison.
    Cmp(CmpOp, Box<RExpr>, Box<RExpr>),
    /// Boolean equality (`negate` for `!=`).
    BoolEq(Box<RExpr>, Box<RExpr>, bool),
    /// Short-circuit `and`.
    And(Box<RExpr>, Box<RExpr>),
    /// Short-circuit `or`.
    Or(Box<RExpr>, Box<RExpr>),
    /// The acting entity.
    SelfEntity,
    /// The touched entity of a collision rule.
    OtherEntity,
    /// Nearest matching entity within `radius` (excluding self).
    Nearest(KindSel, f32),
}

/// Per-entity action slots: one movement, one exclusive action and one
/// transformation per tick, any number of effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    /// `move_towards`, `move_away`, `wander`, `stay`.
    Move,
    /// `eat`, `attack`, `reproduce`, `die`, `trade`.
    Act,
    /// `transform_into` (separate so that a reaction can eat and transform).
    Transform,
    /// Everything else.
    Effect,
}

/// Runtime action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RAction {
    /// Steer towards an entity (arrives without overshooting).
    MoveTowards {
        /// Entity expression.
        target: RExpr,
        /// Speed override.
        speed: Option<RExpr>,
    },
    /// Flee from an entity.
    MoveAway {
        /// Entity expression.
        target: RExpr,
        /// Speed override.
        speed: Option<RExpr>,
    },
    /// Correlated random walk step.
    Wander {
        /// Maximum turn in radians (default: the type's movement turn, π for random walks).
        turn: Option<RExpr>,
        /// Speed override.
        speed: Option<RExpr>,
    },
    /// Stand still.
    Stay,
    /// Eat a touched entity.
    Eat {
        /// Entity expression.
        target: RExpr,
        /// Fraction of the prey's energy gained (default 1).
        efficiency: Option<RExpr>,
    },
    /// Damage a touched entity.
    Attack {
        /// Entity expression.
        target: RExpr,
        /// Energy removed (default 5).
        damage: Option<RExpr>,
    },
    /// Reproduce (asexually, or with a partner).
    Reproduce {
        /// Chance (default 1).
        probability: Option<RExpr>,
        /// Partner entity (sexual types).
        partner: Option<RExpr>,
    },
    /// Become another type.
    Transform(u16),
    /// Die.
    Die,
    /// Exchange carried resources with a touched entity.
    Trade {
        /// Partner.
        target: RExpr,
        /// Resource given (field index).
        give: u16,
        /// Amount given.
        amount: RExpr,
        /// Resource received (field index).
        receive: u16,
        /// Amount received.
        receive_amount: RExpr,
    },
    /// Release a resource at the entity's cell.
    Emit {
        /// Resource field index.
        resource: u16,
        /// Amount.
        amount: RExpr,
    },
    /// Take a resource from the entity's cell.
    Consume {
        /// Resource field index.
        resource: u16,
        /// Amount requested.
        amount: RExpr,
    },
    /// `set prop = value`.
    SetProp {
        /// Property.
        prop: PropRef,
        /// Value.
        value: RExpr,
    },
    /// `add prop += value` / `add prop -= value`.
    AddProp {
        /// Property.
        prop: PropRef,
        /// Amount.
        value: RExpr,
    },
    /// Enter a state (type-local index).
    SetState(u16),
    /// Create entities of a type next to the entity.
    Spawn {
        /// Entity type.
        ty: u16,
        /// Count (rounded, clamped to [0, 100]).
        count: RExpr,
    },
}

impl RAction {
    /// The slot this action occupies.
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

/// A rule product (`produces`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RProduct {
    /// Enter a state (type-local index).
    State(u16),
    /// Spawn entities.
    Spawn {
        /// Entity type.
        ty: u16,
        /// Count.
        count: u32,
    },
    /// Release a resource.
    Resource {
        /// Resource field index.
        resource: u16,
        /// Amount.
        amount: f64,
    },
}

/// Resolved event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum REvent {
    /// Every tick.
    Tick,
    /// First tick of life.
    Birth,
    /// At death.
    Death,
    /// While touching an entity (of a type).
    Collision(Option<u16>),
    /// Tick after entering a state (type-local index).
    StateEnter(u16),
}

/// Where a rule comes from (shown next to the rule in the UI).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RuleOrigin {
    /// Written by the user (index in `LifeSystem::rules`).
    Manual(usize),
    /// Generated from a relationship (index in `LifeSystem::relationships`).
    Relationship {
        /// Relationship index.
        index: usize,
        /// Relation label, e.g. `consumes`.
        relation: String,
    },
    /// Generated from an entity type's reproduction settings.
    EntityType {
        /// Type name.
        entity: String,
    },
}

impl RuleOrigin {
    /// Whether the rule was generated (read-only in editors).
    pub fn is_generated(&self) -> bool {
        !matches!(self, Self::Manual(_))
    }
}

/// A validated, lowered rule.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledRule {
    /// Rule name.
    pub name: String,
    /// Entity type index.
    pub entity: u16,
    /// Event.
    pub event: REvent,
    /// Condition.
    pub condition: Option<RExpr>,
    /// Actions in source order.
    pub actions: Vec<RAction>,
    /// Priority.
    pub priority: i32,
    /// Rule-level probability.
    pub probability: Option<f64>,
    /// Cooldown in ticks (0 = none).
    pub cooldown: u64,
    /// Fires at most once per lifetime.
    pub once: bool,
    /// Costs charged on firing.
    pub costs: Vec<(PropRef, f64)>,
    /// Products.
    pub products: Vec<RProduct>,
    /// Provenance.
    pub origin: RuleOrigin,
    /// The source AST.
    pub ast: RuleAst,
    /// Canonical DSL text.
    pub text: String,
    /// Whether the rule claims the movement slot.
    pub uses_move: bool,
    /// Whether the rule claims the exclusive-action slot.
    pub uses_act: bool,
    /// Whether the rule claims the transformation slot.
    pub uses_transform: bool,
}
