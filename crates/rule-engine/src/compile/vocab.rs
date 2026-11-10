//! The vocabulary of the rule language as data, for editors (palettes,
//! autocompletion, argument forms, tooltips).
//!
//! The validator binds call arguments against [`FUNCTIONS`] itself, so the
//! signatures listed here are exactly the ones that are accepted.

use crate::compile::ir::Slot;

/// Whether a function is read in conditions or performed after `then`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionKind {
    /// Reads the world; used in `when` conditions and in argument values.
    Sensor,
    /// Performed after `then`.
    Action,
}

/// What an argument expects or a sensor returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// A number expression.
    Number,
    /// A number literal in `(0, 32]` (sensing radius).
    Radius,
    /// A number expression in `[0, 1]` (checked when it is a literal).
    Probability,
    /// A true/false expression.
    Bool,
    /// An entity expression: `self`, `other`, `nearest(...)`.
    Entity,
    /// An entity-type name, `same_species` (aliases `same_type`, `same`) or `any`.
    Kind,
    /// An entity-type name.
    EntityType,
    /// A resource-field name.
    Resource,
    /// A resource-field or environment-field name.
    Field,
    /// A state name of the acting entity's type.
    State,
    /// Nothing (actions).
    Nothing,
}

/// One parameter of a function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamSig {
    /// Name (usable as `name=value`).
    pub name: &'static str,
    /// Whether it must be given.
    pub required: bool,
    /// Expected value.
    pub kind: ValueKind,
    /// Default or meaning when omitted / short explanation.
    pub help: &'static str,
}

/// Signature of a sensor or action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionSig {
    /// Name as written in rules.
    pub name: &'static str,
    /// Sensor or action.
    pub kind: FunctionKind,
    /// Parameters in positional order.
    pub params: &'static [ParamSig],
    /// Result of a sensor ([`ValueKind::Nothing`] for actions).
    pub returns: ValueKind,
    /// Slot an action occupies (`None` for sensors).
    pub slot: Option<Slot>,
    /// Allowed in `on_death` rules.
    pub on_death: bool,
    /// One-line description.
    pub help: &'static str,
}

const fn req(name: &'static str, kind: ValueKind, help: &'static str) -> ParamSig {
    ParamSig { name, required: true, kind, help }
}

const fn opt(name: &'static str, kind: ValueKind, help: &'static str) -> ParamSig {
    ParamSig { name, required: false, kind, help }
}

const fn sensor(
    name: &'static str,
    params: &'static [ParamSig],
    returns: ValueKind,
    help: &'static str,
) -> FunctionSig {
    FunctionSig { name, kind: FunctionKind::Sensor, params, returns, slot: None, on_death: false, help }
}

const fn action(name: &'static str, params: &'static [ParamSig], slot: Slot, help: &'static str) -> FunctionSig {
    FunctionSig {
        name,
        kind: FunctionKind::Action,
        params,
        returns: ValueKind::Nothing,
        slot: Some(slot),
        on_death: false,
        help,
    }
}

/// Marks an action as allowed in `on_death` rules.
const fn on_death(f: FunctionSig) -> FunctionSig {
    FunctionSig { on_death: true, ..f }
}

use ValueKind as V;

const KIND_RADIUS: &[ParamSig] = &[
    req("kind", V::Kind, "entity type, same_species or any"),
    opt("radius", V::Radius, "default: the type's perception"),
];
const TARGET: &[ParamSig] = &[req("target", V::Entity, "self, other or nearest(...)")];
const MOVE: &[ParamSig] = &[
    req("target", V::Entity, "entity to move towards / away from"),
    opt("speed", V::Number, "default: the entity's speed"),
];
const RESOURCE_AMOUNT: &[ParamSig] =
    &[req("resource", V::Resource, "resource field"), req("amount", V::Number, "amount (≥ 0)")];

/// Every sensor and action, sensors first.
pub const FUNCTIONS: &[FunctionSig] = &[
    sensor("nearby", KIND_RADIUS, V::Number, "number of matching entities within the radius (excluding self)"),
    sensor("nearest", KIND_RADIUS, V::Entity, "nearest matching entity within the radius (ties: lower id)"),
    sensor("distance", TARGET, V::Number, "distance to an entity (infinite when there is none)"),
    sensor("exists", TARGET, V::Bool, "whether an entity expression found something"),
    sensor(
        "field",
        &[req("name", V::Field, "resource or environment field")],
        V::Number,
        "field value at the entity's cell",
    ),
    sensor(
        "population",
        &[req("kind", V::Kind, "entity type, same_species or any")],
        V::Number,
        "global population at the start of the tick",
    ),
    sensor("random", &[], V::Number, "uniform number in [0, 1) from the rule's keyed random stream"),
    sensor("chance", &[req("p", V::Probability, "probability")], V::Bool, "true with probability p"),
    sensor("min", &[req("a", V::Number, ""), req("b", V::Number, "")], V::Number, "smaller of two numbers"),
    sensor("max", &[req("a", V::Number, ""), req("b", V::Number, "")], V::Number, "larger of two numbers"),
    sensor("abs", &[req("x", V::Number, "")], V::Number, "absolute value"),
    sensor(
        "clamp",
        &[req("x", V::Number, ""), req("lo", V::Number, "lower bound"), req("hi", V::Number, "upper bound")],
        V::Number,
        "x limited to [lo, hi] (x itself when lo > hi)",
    ),
    action("move_towards", MOVE, Slot::Move, "steer to the target and arrive without overshooting"),
    action("move_away", MOVE, Slot::Move, "flee from the target"),
    action(
        "wander",
        &[
            opt("turn", V::Number, "maximum turn in radians; default: the type's"),
            opt("speed", V::Number, "default: the entity's speed"),
        ],
        Slot::Move,
        "correlated random-walk step",
    ),
    action("stay", &[], Slot::Move, "stand still"),
    action(
        "eat",
        &[
            req("target", V::Entity, "a touching entity"),
            opt("efficiency", V::Number, "fraction of its energy gained, default 1"),
        ],
        Slot::Act,
        "the target dies and the eater gains efficiency × its energy (contested)",
    ),
    action(
        "attack",
        &[req("target", V::Entity, "a touching entity"), opt("damage", V::Number, "energy removed, default 5")],
        Slot::Act,
        "the target loses energy and dies at ≤ 0 (contested)",
    ),
    action(
        "reproduce",
        &[opt("probability", V::Probability, "default 1"), opt("partner", V::Entity, "required for sexual types")],
        Slot::Act,
        "offspring share the energy the parents pay (needs maturity, cooldown, energy > cost)",
    ),
    action(
        "transform_into",
        &[req("type", V::EntityType, "new type")],
        Slot::Transform,
        "become another type at the end of the tick",
    ),
    action("die", &[], Slot::Act, "die at the end of the tick"),
    action(
        "trade",
        &[
            req("target", V::Entity, "a touching entity"),
            req("give", V::Resource, "carried resource given"),
            req("amount", V::Number, "amount given"),
            req("receive", V::Resource, "carried resource received"),
            req("receive_amount", V::Number, "amount received"),
        ],
        Slot::Act,
        "atomic exchange of carried resources (contested)",
    ),
    on_death(action(
        "emit",
        RESOURCE_AMOUNT,
        Slot::Effect,
        "release a resource at the entity's cell (from its stock if carried)",
    )),
    action(
        "consume",
        RESOURCE_AMOUNT,
        Slot::Effect,
        "take a resource from the cell (shared proportionally when contested)",
    ),
    action("set_state", &[req("state", V::State, "state of the entity's type")], Slot::Effect, "enter a state"),
    on_death(action(
        "spawn",
        &[req("type", V::EntityType, "entity type"), opt("n", V::Number, "count in [0, 100], default 1")],
        Slot::Effect,
        "create entities next to the entity",
    )),
];

/// Looks up a sensor or action by name.
pub fn function(name: &str) -> Option<&'static FunctionSig> {
    FUNCTIONS.iter().find(|f| f.name == name)
}

/// Parameters of a function (empty for unknown names).
pub(crate) fn params(name: &str) -> &'static [ParamSig] {
    function(name).map_or(&[], |f| f.params)
}

/// Names of the sensors.
pub fn sensor_names() -> impl Iterator<Item = &'static str> {
    FUNCTIONS.iter().filter(|f| f.kind == FunctionKind::Sensor).map(|f| f.name)
}

/// Names of the actions.
pub fn action_names() -> impl Iterator<Item = &'static str> {
    FUNCTIONS.iter().filter(|f| f.kind == FunctionKind::Action).map(|f| f.name)
}

/// A built-in entity property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyInfo {
    /// Name (`energy`, `self.energy`, `other.energy`).
    pub name: &'static str,
    /// Writable with `set`/`add`/`cost`.
    pub writable: bool,
    /// Meaning.
    pub help: &'static str,
}

/// Built-in properties of every entity (user properties and carried
/// resources of a type come from its definition).
pub const BUILTIN_PROPERTIES: &[PropertyInfo] = &[
    PropertyInfo { name: "energy", writable: true, help: "energy (clamped to the type's maximum)" },
    PropertyInfo { name: "age", writable: false, help: "ticks since birth" },
    PropertyInfo { name: "state_age", writable: false, help: "ticks since entering the current type and state" },
    PropertyInfo { name: "size", writable: true, help: "diameter in world units" },
    PropertyInfo { name: "speed", writable: true, help: "movement speed" },
    PropertyInfo { name: "x", writable: false, help: "position x" },
    PropertyInfo { name: "y", writable: false, help: "position y" },
    PropertyInfo { name: "generation", writable: false, help: "0 for initial entities, parent + 1 for offspring" },
    PropertyInfo { name: "id", writable: false, help: "stable id" },
];

/// A keyword construct with its syntax and meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeywordInfo {
    /// Syntax as written.
    pub syntax: &'static str,
    /// Meaning.
    pub help: &'static str,
}

/// Rule events.
pub const EVENTS: &[KeywordInfo] = &[
    KeywordInfo { syntax: "on_tick", help: "every tick (the default)" },
    KeywordInfo { syntax: "on_birth", help: "on the first tick of the entity's life" },
    KeywordInfo { syntax: "on_death", help: "when the entity dies; only emit() and spawn()" },
    KeywordInfo { syntax: "on_collision [with TYPE]", help: "while touching another entity, bound to `other`" },
    KeywordInfo { syntax: "on_state_enter(STATE)", help: "on the tick after entering the state" },
];

/// Rule modifiers.
pub const MODIFIERS: &[KeywordInfo] = &[
    KeywordInfo { syntax: "priority N", help: "higher first; ties keep declaration order (|N| ≤ 1000000)" },
    KeywordInfo { syntax: "probability P", help: "chance that an eligible rule fires" },
    KeywordInfo { syntax: "cooldown N", help: "at least N ticks between firings by one entity" },
    KeywordInfo { syntax: "cost PROPERTY AMOUNT", help: "charged on firing; the rule needs it to be affordable" },
    KeywordInfo { syntax: "produces NAME [AMOUNT]", help: "a state entered, entities spawned or a resource released" },
    KeywordInfo { syntax: "once", help: "at most once per lifetime" },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        let mut names: Vec<&str> = FUNCTIONS.iter().map(|f| f.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), FUNCTIONS.len(), "unique names");
        let death: Vec<&str> = FUNCTIONS.iter().filter(|f| f.on_death).map(|f| f.name).collect();
        assert_eq!(death, vec!["emit", "spawn"]);
        for f in FUNCTIONS {
            assert_eq!(f.kind == FunctionKind::Action, f.slot.is_some(), "{}", f.name);
            let first_optional = f.params.iter().position(|p| !p.required).unwrap_or(f.params.len());
            assert!(f.params[first_optional..].iter().all(|p| !p.required), "{}: required after optional", f.name);
        }
    }
}
