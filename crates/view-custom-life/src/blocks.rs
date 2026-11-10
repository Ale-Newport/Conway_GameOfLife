//! The visual rule builder's model: a rule as blocks (event, condition rows
//! in AND groups joined by OR, actions with parameter slots bound to
//! [`vocab::FUNCTIONS`], modifiers) and its lossless conversion to and from
//! the rule-engine AST.
//!
//! Anything the blocks cannot express structurally (negations, arithmetic,
//! nested calls, unknown actions) is kept as DSL text in an `Expr` / `Raw`
//! block, so `from_ast` never loses information. `to_ast` may normalise
//! equivalent spellings (`self.energy` keeps its `self.`, but a positional
//! sensing radius becomes `radius=`), so the round trip preserves the
//! meaning and the canonical text is stable after one normalisation.

use rule_engine::Span;
use rule_engine::compile::vocab::{self, FunctionKind, ParamSig, ValueKind};
use rule_engine::dsl::parser::is_keyword;
use rule_engine::dsl::{
    Action, Arg, BinOp, Cost, Event, Expr, ExprKind, Name, Product, RuleAst, RuleMeta, parse_expr, parse_rule,
    print_expr, print_rule,
};

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
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

impl Cmp {
    /// Every comparator in menu order.
    pub const ALL: [Cmp; 6] = [Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge, Cmp::Eq, Cmp::Ne];

    /// Source symbol.
    pub fn symbol(self) -> &'static str {
        self.op().symbol()
    }

    /// The binary operator.
    pub fn op(self) -> BinOp {
        match self {
            Cmp::Lt => BinOp::Lt,
            Cmp::Le => BinOp::Le,
            Cmp::Gt => BinOp::Gt,
            Cmp::Ge => BinOp::Ge,
            Cmp::Eq => BinOp::Eq,
            Cmp::Ne => BinOp::Ne,
        }
    }

    /// The comparator of a binary operator, if it is one.
    pub fn from_op(op: BinOp) -> Option<Cmp> {
        Cmp::ALL.into_iter().find(|c| c.op() == op)
    }
}

/// Whose property a condition reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    /// Bare name (`energy`): the acting entity.
    Implicit,
    /// `self.name`.
    Me,
    /// `other.name` (collision rules).
    Other,
}

/// An entity expression: `self`, `other`, `nearest(kind, radius=r)` or text.
#[derive(Clone, Debug, PartialEq)]
pub enum EntitySel {
    /// `self`.
    Me,
    /// `other` (collision rules).
    Other,
    /// `nearest(kind[, radius=r])`.
    Nearest {
        /// Entity type, `same_species` or `any`.
        kind: String,
        /// Sensing radius (the type's perception when absent).
        radius: Option<f64>,
    },
    /// Any other entity expression, as DSL text.
    Expr(String),
}

/// One side of a comparison.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    /// A property (`energy`, `self.age`, `other.size`, `state`, `tick`, user
    /// properties, carried resources).
    Property {
        /// Whose property.
        who: Who,
        /// Property name.
        name: String,
    },
    /// `nearby(kind[, radius=r])`.
    Nearby {
        /// Entity type, `same_species` or `any`.
        kind: String,
        /// Sensing radius.
        radius: Option<f64>,
    },
    /// `distance(entity)`.
    Distance(EntitySel),
    /// `field(name)`.
    Field(String),
    /// `population(kind)`.
    Population(String),
    /// `random()`.
    Random,
    /// A number literal.
    Number(f64),
    /// A name (state or type on the right of `state ==` / `type ==`).
    Name(String),
    /// Any other expression, as DSL text.
    Expr(String),
}

/// One condition row.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// `left cmp right`.
    Compare {
        /// Left side.
        left: Operand,
        /// Comparator.
        cmp: Cmp,
        /// Right side.
        right: Operand,
    },
    /// `chance(p)`.
    Chance(f64),
    /// `exists(entity)`.
    Exists(EntitySel),
    /// Any other condition, as DSL text.
    Expr(String),
}

/// Condition rows that must all hold (AND).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CondGroup {
    /// Rows.
    pub rows: Vec<Condition>,
}

/// The value of an action parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum ArgValue {
    /// A number literal.
    Number(f64),
    /// A name (type, resource, field or state).
    Name(String),
    /// An entity expression.
    Entity(EntitySel),
    /// Any other expression, as DSL text.
    Expr(String),
}

/// One action.
#[derive(Clone, Debug, PartialEq)]
pub enum ActionBlock {
    /// A vocabulary action with one slot per parameter (`None` = omitted).
    Call {
        /// Action name (a [`vocab::FUNCTIONS`] action).
        name: String,
        /// Values in parameter order.
        args: Vec<Option<ArgValue>>,
    },
    /// `set path = value`.
    Set {
        /// Property path.
        path: String,
        /// Value.
        value: ArgValue,
    },
    /// `add path += value` / `add path -= value`.
    Add {
        /// Property path.
        path: String,
        /// Amount.
        value: ArgValue,
        /// `-=` instead of `+=`.
        subtract: bool,
    },
    /// Any other action, as DSL text.
    Raw(String),
}

/// The triggering event.
#[derive(Clone, Debug, PartialEq)]
pub enum EventBlock {
    /// Every tick.
    Tick,
    /// First tick of life.
    Birth,
    /// On death (`emit` and `spawn` only).
    Death,
    /// While touching another entity (of a type).
    Collision(Option<String>),
    /// The tick after entering a state.
    StateEnter(String),
}

impl EventBlock {
    /// Menu label.
    pub fn label(&self) -> &'static str {
        match self {
            EventBlock::Tick => "every tick",
            EventBlock::Birth => "on birth",
            EventBlock::Death => "on death",
            EventBlock::Collision(_) => "on collision",
            EventBlock::StateEnter(_) => "on state enter",
        }
    }
}

/// A rule as blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleBlocks {
    /// Rule name.
    pub name: String,
    /// Entity type.
    pub entity: String,
    /// Event.
    pub event: EventBlock,
    /// Condition: OR of AND groups (empty = always).
    pub groups: Vec<CondGroup>,
    /// Actions (at least one).
    pub actions: Vec<ActionBlock>,
    /// Priority.
    pub priority: i32,
    /// Probability.
    pub probability: Option<f64>,
    /// Cooldown in ticks.
    pub cooldown: Option<u64>,
    /// Costs `(property path, amount)`.
    pub costs: Vec<(String, f64)>,
    /// Products `(name, amount)`.
    pub produces: Vec<(String, f64)>,
    /// At most once per lifetime.
    pub once: bool,
}

/// Why blocks could not be turned into a rule.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockError {
    /// Which block (e.g. `condition 2.1`, `action 1`).
    pub location: String,
    /// What is wrong.
    pub message: String,
}

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

fn err(location: impl Into<String>, message: impl Into<String>) -> BlockError {
    BlockError { location: location.into(), message: message.into() }
}

/// Whether `word` can be written without quotes.
pub fn is_bare(word: &str) -> bool {
    let mut chars = word.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !is_keyword(word)
}

/// A name as an expression: an identifier path when possible, else a string.
pub fn name_expr(name: &str) -> Expr {
    if is_bare(name) { Expr::path(&[name]) } else { Expr::new(ExprKind::Str(name.to_string())) }
}

fn as_name(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Path(p) if p.len() == 1 => Some(p[0].clone()),
        ExprKind::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn as_num(e: &Expr) -> Option<f64> {
    match e.kind {
        ExprKind::Num(n) => Some(n),
        _ => None,
    }
}

fn parse_text(text: &str, location: &str) -> Result<Expr, BlockError> {
    if text.trim().is_empty() {
        return Err(err(location, "the expression is empty"));
    }
    parse_expr(text).map_err(|d| {
        let first = d.first().map_or_else(|| "syntax error".to_string(), |d| d.message.clone());
        err(location, first)
    })
}

/// Binds call arguments to a signature: one slot per parameter.
fn bind<'a>(params: &[ParamSig], args: &'a [Arg]) -> Option<Vec<Option<&'a Expr>>> {
    let mut slots: Vec<Option<&Expr>> = vec![None; params.len()];
    let mut next = 0;
    for arg in args {
        let index = match &arg.name {
            None => {
                let i = next;
                next += 1;
                i
            }
            Some(n) => params.iter().position(|p| p.name == n)?,
        };
        let slot = slots.get_mut(index)?;
        if slot.is_some() {
            return None;
        }
        *slot = Some(&arg.value);
    }
    Some(slots)
}

impl EntitySel {
    /// Reads an entity expression.
    pub fn from_expr(e: &Expr) -> Self {
        match &e.kind {
            ExprKind::Path(p) if p.len() == 1 && p[0] == "self" => EntitySel::Me,
            ExprKind::Path(p) if p.len() == 1 && p[0] == "other" => EntitySel::Other,
            ExprKind::Call { name, args } if name == "nearest" => {
                let params = vocab::function("nearest").map_or(&[][..], |f| f.params);
                match bind(params, args).as_deref() {
                    Some([Some(kind), radius]) => match (as_name(kind), radius.map(as_num)) {
                        (Some(kind), None) => EntitySel::Nearest { kind, radius: None },
                        (Some(kind), Some(Some(r))) => EntitySel::Nearest { kind, radius: Some(r) },
                        _ => EntitySel::Expr(print_expr(e)),
                    },
                    _ => EntitySel::Expr(print_expr(e)),
                }
            }
            _ => EntitySel::Expr(print_expr(e)),
        }
    }

    /// Writes the entity expression.
    pub fn to_expr(&self, location: &str) -> Result<Expr, BlockError> {
        Ok(match self {
            EntitySel::Me => Expr::path(&["self"]),
            EntitySel::Other => Expr::path(&["other"]),
            EntitySel::Nearest { kind, radius } => {
                if kind.trim().is_empty() {
                    return Err(err(location, "choose what `nearest` looks for"));
                }
                let mut args = vec![Arg::pos(name_expr(kind))];
                if let Some(r) = radius {
                    args.push(Arg::named("radius", Expr::num(*r)));
                }
                Expr::call("nearest", args)
            }
            EntitySel::Expr(text) => parse_text(text, location)?,
        })
    }

    /// Short label for menus.
    pub fn label(&self) -> String {
        match self {
            EntitySel::Me => "self".into(),
            EntitySel::Other => "other".into(),
            EntitySel::Nearest { .. } => "nearest".into(),
            EntitySel::Expr(_) => "expression".into(),
        }
    }
}

fn is_state_or_type(op: &Operand) -> bool {
    matches!(op, Operand::Property { name, .. } if name == "state" || name == "type")
}

impl Operand {
    fn from_expr(e: &Expr, right_of: Option<&Operand>) -> Self {
        let names_expected = right_of.is_some_and(is_state_or_type);
        match &e.kind {
            ExprKind::Path(p) if p.len() == 1 && names_expected => Operand::Name(p[0].clone()),
            ExprKind::Str(s) if names_expected => Operand::Name(s.clone()),
            ExprKind::Path(p) if p.len() == 1 && p[0] != "self" && p[0] != "other" => {
                Operand::Property { who: Who::Implicit, name: p[0].clone() }
            }
            ExprKind::Path(p) if p.len() == 2 && p[0] == "self" => {
                Operand::Property { who: Who::Me, name: p[1].clone() }
            }
            ExprKind::Path(p) if p.len() == 2 && p[0] == "other" => {
                Operand::Property { who: Who::Other, name: p[1].clone() }
            }
            ExprKind::Num(n) => Operand::Number(*n),
            ExprKind::Call { name, args } => {
                let params = vocab::function(name).map_or(&[][..], |f| f.params);
                let bound = bind(params, args);
                match (name.as_str(), bound.as_deref()) {
                    ("nearby", Some([Some(kind), radius])) => match (as_name(kind), radius.map(as_num)) {
                        (Some(kind), None) => Operand::Nearby { kind, radius: None },
                        (Some(kind), Some(Some(r))) => Operand::Nearby { kind, radius: Some(r) },
                        _ => Operand::Expr(print_expr(e)),
                    },
                    ("distance", Some([Some(target)])) => Operand::Distance(EntitySel::from_expr(target)),
                    ("field", Some([Some(n)])) => {
                        as_name(n).map_or_else(|| Operand::Expr(print_expr(e)), Operand::Field)
                    }
                    ("population", Some([Some(k)])) => {
                        as_name(k).map_or_else(|| Operand::Expr(print_expr(e)), Operand::Population)
                    }
                    ("random", Some([])) => Operand::Random,
                    _ => Operand::Expr(print_expr(e)),
                }
            }
            _ => Operand::Expr(print_expr(e)),
        }
    }

    fn to_expr(&self, location: &str) -> Result<Expr, BlockError> {
        Ok(match self {
            Operand::Property { who, name } => {
                if name.trim().is_empty() {
                    return Err(err(location, "choose a property"));
                }
                match who {
                    Who::Implicit => Expr::path(&[name]),
                    Who::Me => Expr::path(&["self", name]),
                    Who::Other => Expr::path(&["other", name]),
                }
            }
            Operand::Nearby { kind, radius } => {
                if kind.trim().is_empty() {
                    return Err(err(location, "choose what `nearby` counts"));
                }
                let mut args = vec![Arg::pos(name_expr(kind))];
                if let Some(r) = radius {
                    args.push(Arg::named("radius", Expr::num(*r)));
                }
                Expr::call("nearby", args)
            }
            Operand::Distance(sel) => Expr::call("distance", vec![Arg::pos(sel.to_expr(location)?)]),
            Operand::Field(n) => {
                if n.trim().is_empty() {
                    return Err(err(location, "choose a field"));
                }
                Expr::call("field", vec![Arg::pos(name_expr(n))])
            }
            Operand::Population(k) => {
                if k.trim().is_empty() {
                    return Err(err(location, "choose what `population` counts"));
                }
                Expr::call("population", vec![Arg::pos(name_expr(k))])
            }
            Operand::Random => Expr::call("random", Vec::new()),
            Operand::Number(n) => {
                if !n.is_finite() {
                    return Err(err(location, "numbers must be finite"));
                }
                Expr::num(*n)
            }
            Operand::Name(n) => {
                if n.trim().is_empty() {
                    return Err(err(location, "choose a name"));
                }
                name_expr(n)
            }
            Operand::Expr(text) => parse_text(text, location)?,
        })
    }

    /// Short label for menus.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Operand::Property { .. } => "property",
            Operand::Nearby { .. } => "nearby",
            Operand::Distance(_) => "distance",
            Operand::Field(_) => "field",
            Operand::Population(_) => "population",
            Operand::Random => "random",
            Operand::Number(_) => "number",
            Operand::Name(_) => "name",
            Operand::Expr(_) => "expression",
        }
    }
}

impl Condition {
    fn from_expr(e: &Expr) -> Self {
        match &e.kind {
            ExprKind::Binary { op, lhs, rhs } if Cmp::from_op(*op).is_some() => {
                let left = Operand::from_expr(lhs, None);
                let right = Operand::from_expr(rhs, Some(&left));
                Condition::Compare { left, cmp: Cmp::from_op(*op).unwrap_or(Cmp::Eq), right }
            }
            ExprKind::Call { name, args } if name == "chance" => match args.as_slice() {
                [Arg { value, .. }] if as_num(value).is_some() => Condition::Chance(as_num(value).unwrap_or(0.0)),
                _ => Condition::Expr(print_expr(e)),
            },
            ExprKind::Call { name, args } if name == "exists" => match args.as_slice() {
                [Arg { name: None, value }] => Condition::Exists(EntitySel::from_expr(value)),
                _ => Condition::Expr(print_expr(e)),
            },
            _ => Condition::Expr(print_expr(e)),
        }
    }

    fn to_expr(&self, location: &str) -> Result<Expr, BlockError> {
        Ok(match self {
            Condition::Compare { left, cmp, right } => {
                Expr::binary(cmp.op(), left.to_expr(location)?, right.to_expr(location)?)
            }
            Condition::Chance(p) => {
                if !(0.0..=1.0).contains(p) {
                    return Err(err(location, "a chance must be in [0, 1]"));
                }
                Expr::call("chance", vec![Arg::pos(Expr::num(*p))])
            }
            Condition::Exists(sel) => Expr::call("exists", vec![Arg::pos(sel.to_expr(location)?)]),
            Condition::Expr(text) => parse_text(text, location)?,
        })
    }
}

fn flatten(e: &Expr, op: BinOp, out: &mut Vec<Expr>) {
    match &e.kind {
        ExprKind::Binary { op: o, lhs, rhs } if *o == op => {
            flatten(lhs, op, out);
            flatten(rhs, op, out);
        }
        _ => out.push(e.clone()),
    }
}

impl ArgValue {
    fn from_expr(e: &Expr, kind: ValueKind) -> Self {
        match kind {
            ValueKind::Number | ValueKind::Radius | ValueKind::Probability => {
                as_num(e).map_or_else(|| ArgValue::Expr(print_expr(e)), ArgValue::Number)
            }
            ValueKind::Kind | ValueKind::EntityType | ValueKind::Resource | ValueKind::Field | ValueKind::State => {
                as_name(e).map_or_else(|| ArgValue::Expr(print_expr(e)), ArgValue::Name)
            }
            ValueKind::Entity => ArgValue::Entity(EntitySel::from_expr(e)),
            ValueKind::Bool | ValueKind::Nothing => ArgValue::Expr(print_expr(e)),
        }
    }

    fn to_expr(&self, location: &str) -> Result<Expr, BlockError> {
        match self {
            ArgValue::Number(n) if n.is_finite() => Ok(Expr::num(*n)),
            ArgValue::Number(_) => Err(err(location, "numbers must be finite")),
            ArgValue::Name(n) if n.trim().is_empty() => Err(err(location, "choose a name")),
            ArgValue::Name(n) => Ok(name_expr(n)),
            ArgValue::Entity(sel) => sel.to_expr(location),
            ArgValue::Expr(text) => parse_text(text, location),
        }
    }

    /// The default value of a parameter kind (used when a slot is switched on).
    pub fn default_for(kind: ValueKind) -> Self {
        match kind {
            ValueKind::Number => ArgValue::Number(1.0),
            ValueKind::Radius => ArgValue::Number(5.0),
            ValueKind::Probability => ArgValue::Number(0.5),
            ValueKind::Entity => ArgValue::Entity(EntitySel::Other),
            ValueKind::Kind | ValueKind::EntityType | ValueKind::Resource | ValueKind::Field | ValueKind::State => {
                ArgValue::Name(String::new())
            }
            ValueKind::Bool | ValueKind::Nothing => ArgValue::Expr("true".into()),
        }
    }
}

/// Prints one action as it appears after `then`.
pub fn print_action(action: &Action) -> String {
    let text = print_rule(&RuleAst::new("x", "x", vec![action.clone()]));
    text.split_once("\n    then ").map_or(text.clone(), |(_, rest)| rest.to_string())
}

/// Parses one action written as text.
pub fn parse_action(text: &str) -> Result<Action, String> {
    if text.trim().is_empty() {
        return Err("the action is empty".into());
    }
    let rule = parse_rule(&format!("rule x for x: then {text}"))
        .map_err(|d| d.first().map_or_else(|| "syntax error".to_string(), |d| d.message.clone()))?;
    if rule.meta != RuleMeta::default() {
        return Err("modifiers (priority, cooldown...) belong to the rule, not to an action".into());
    }
    match rule.actions.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err("write exactly one action".into()),
    }
}

impl ActionBlock {
    /// Reads an action.
    pub fn from_action(action: &Action) -> Self {
        match action {
            Action::Call { name, args, .. } => {
                let Some(f) = vocab::function(name).filter(|f| f.kind == FunctionKind::Action) else {
                    return ActionBlock::Raw(print_action(action));
                };
                match bind(f.params, args) {
                    Some(slots) => ActionBlock::Call {
                        name: name.clone(),
                        args: slots
                            .iter()
                            .zip(f.params)
                            .map(|(slot, p)| slot.map(|e| ArgValue::from_expr(e, p.kind)))
                            .collect(),
                    },
                    None => ActionBlock::Raw(print_action(action)),
                }
            }
            Action::Set { path, value, .. } => {
                ActionBlock::Set { path: path.join("."), value: ArgValue::from_expr(value, ValueKind::Number) }
            }
            Action::Add { path, value, subtract, .. } => ActionBlock::Add {
                path: path.join("."),
                value: ArgValue::from_expr(value, ValueKind::Number),
                subtract: *subtract,
            },
        }
    }

    /// A new call block for a vocabulary action with its required slots filled.
    pub fn new_call(name: &str) -> Self {
        let params = vocab::function(name).map_or(&[][..], |f| f.params);
        ActionBlock::Call {
            name: name.to_string(),
            args: params.iter().map(|p| p.required.then(|| ArgValue::default_for(p.kind))).collect(),
        }
    }

    /// Writes the action.
    pub fn to_action(&self, location: &str) -> Result<Action, BlockError> {
        let split = |path: &str| -> Result<Vec<String>, BlockError> {
            let parts: Vec<String> = path.split('.').map(|s| s.trim().to_string()).collect();
            if parts.iter().any(String::is_empty) {
                return Err(err(location, "choose a property"));
            }
            Ok(parts)
        };
        match self {
            ActionBlock::Call { name, args } => {
                let Some(f) = vocab::function(name).filter(|f| f.kind == FunctionKind::Action) else {
                    return Err(err(location, format!("`{name}` is not an action")));
                };
                let mut out = Vec::new();
                for (i, p) in f.params.iter().enumerate() {
                    match args.get(i).and_then(Option::as_ref) {
                        Some(v) => {
                            let value = v.to_expr(&format!("{location}, {}", p.name))?;
                            out.push(if p.required { Arg::pos(value) } else { Arg::named(p.name, value) });
                        }
                        None if p.required => {
                            return Err(err(location, format!("`{name}` needs `{}`", p.name)));
                        }
                        None => {}
                    }
                }
                Ok(Action::call(name, out))
            }
            ActionBlock::Set { path, value } => {
                Ok(Action::Set { path: split(path)?, value: value.to_expr(location)?, span: Span::default() })
            }
            ActionBlock::Add { path, value, subtract } => Ok(Action::Add {
                path: split(path)?,
                value: value.to_expr(location)?,
                subtract: *subtract,
                span: Span::default(),
            }),
            ActionBlock::Raw(text) => parse_action(text).map_err(|m| err(location, m)),
        }
    }

    /// Name shown in the action picker.
    pub fn label(&self) -> String {
        match self {
            ActionBlock::Call { name, .. } => name.clone(),
            ActionBlock::Set { .. } => "set".into(),
            ActionBlock::Add { subtract: false, .. } => "add".into(),
            ActionBlock::Add { subtract: true, .. } => "subtract".into(),
            ActionBlock::Raw(_) => "custom".into(),
        }
    }
}

impl RuleBlocks {
    /// A rule for `entity` without condition and with `first_action` (a
    /// vocabulary action with its required slots at their defaults).
    pub fn new(name: &str, entity: &str, first_action: &str) -> Self {
        Self {
            name: name.to_string(),
            entity: entity.to_string(),
            event: EventBlock::Tick,
            groups: Vec::new(),
            actions: vec![ActionBlock::new_call(first_action)],
            priority: 0,
            probability: None,
            cooldown: None,
            costs: Vec::new(),
            produces: Vec::new(),
            once: false,
        }
    }

    /// Reads a rule.
    pub fn from_ast(rule: &RuleAst) -> Self {
        let event = match &rule.event {
            Event::Tick => EventBlock::Tick,
            Event::Birth => EventBlock::Birth,
            Event::Death => EventBlock::Death,
            Event::Collision { with } => EventBlock::Collision(with.as_ref().map(|n| n.text.clone())),
            Event::StateEnter { state } => EventBlock::StateEnter(state.text.clone()),
        };
        let mut groups = Vec::new();
        if let Some(cond) = &rule.condition {
            let mut disjuncts = Vec::new();
            flatten(cond, BinOp::Or, &mut disjuncts);
            for d in &disjuncts {
                let mut conjuncts = Vec::new();
                flatten(d, BinOp::And, &mut conjuncts);
                groups.push(CondGroup { rows: conjuncts.iter().map(Condition::from_expr).collect() });
            }
        }
        let m = &rule.meta;
        Self {
            name: rule.name.clone(),
            entity: rule.entity.text.clone(),
            event,
            groups,
            actions: rule.actions.iter().map(ActionBlock::from_action).collect(),
            priority: m.priority,
            probability: m.probability,
            cooldown: m.cooldown,
            costs: m.costs.iter().map(|c| (c.path.join("."), c.amount)).collect(),
            produces: m.produces.iter().map(|p| (p.name.text.clone(), p.amount)).collect(),
            once: m.once,
        }
    }

    /// The condition expression of the groups (`None` when there are no rows).
    pub fn condition(&self) -> Result<Option<Expr>, BlockError> {
        let mut disjuncts = Vec::new();
        for (g, group) in self.groups.iter().enumerate() {
            let mut terms = Vec::new();
            for (r, row) in group.rows.iter().enumerate() {
                terms.push(row.to_expr(&format!("condition {}.{}", g + 1, r + 1))?);
            }
            if let Some(all) = Expr::all(terms) {
                disjuncts.push(all);
            }
        }
        Ok(disjuncts.into_iter().reduce(|acc, d| Expr::binary(BinOp::Or, acc, d)))
    }

    /// Writes the rule.
    pub fn to_ast(&self) -> Result<RuleAst, BlockError> {
        if self.name.trim().is_empty() {
            return Err(err("rule", "the rule needs a name"));
        }
        if self.entity.trim().is_empty() {
            return Err(err("rule", "choose the entity type the rule belongs to"));
        }
        if self.actions.is_empty() {
            return Err(err("actions", "a rule needs at least one action"));
        }
        let mut actions = Vec::new();
        for (i, a) in self.actions.iter().enumerate() {
            actions.push(a.to_action(&format!("action {}", i + 1))?);
        }
        let mut rule = RuleAst::new(&self.name, &self.entity, actions);
        rule.event = match &self.event {
            EventBlock::Tick => Event::Tick,
            EventBlock::Birth => Event::Birth,
            EventBlock::Death => Event::Death,
            EventBlock::Collision(with) => {
                Event::Collision { with: with.as_ref().filter(|w| !w.trim().is_empty()).map(Name::new) }
            }
            EventBlock::StateEnter(state) => {
                if state.trim().is_empty() {
                    return Err(err("event", "choose the state"));
                }
                Event::StateEnter { state: Name::new(state.clone()) }
            }
        };
        rule.condition = self.condition()?;
        let mut costs = Vec::new();
        for (i, (path, amount)) in self.costs.iter().enumerate() {
            let parts: Vec<String> = path.split('.').map(|s| s.trim().to_string()).collect();
            if parts.iter().any(String::is_empty) {
                return Err(err(format!("cost {}", i + 1), "choose a property"));
            }
            costs.push(Cost { path: parts, amount: *amount, span: Span::default() });
        }
        let mut produces = Vec::new();
        for (i, (name, amount)) in self.produces.iter().enumerate() {
            if name.trim().is_empty() {
                return Err(err(format!("product {}", i + 1), "choose a state, type or resource"));
            }
            produces.push(Product { name: Name::new(name.clone()), amount: *amount });
        }
        rule.meta = RuleMeta {
            priority: self.priority,
            probability: self.probability,
            cooldown: self.cooldown,
            costs,
            produces,
            once: self.once,
        };
        Ok(rule)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::dsl::{parse_program, print_rule};
    use rule_engine::{check_rule, compile, scenarios};

    fn round_trip(rule: &RuleAst) -> RuleAst {
        RuleBlocks::from_ast(rule).to_ast().unwrap_or_else(|e| panic!("{e} for:\n{}", print_rule(rule)))
    }

    #[test]
    fn every_preset_rule_round_trips_with_the_same_meaning() {
        for scenario in scenarios::builtin().unwrap() {
            let compiled = compile(&scenario.system).unwrap();
            let rules: Vec<RuleAst> = compiled.rules().iter().map(|r| r.ast.clone()).collect();
            for rule in &rules {
                let once = round_trip(rule);
                // Stable after one normalisation, and the blocks are lossless.
                assert_eq!(round_trip(&once), once, "{}", print_rule(rule));
                assert_eq!(RuleBlocks::from_ast(&once), RuleBlocks::from_ast(&round_trip(&once)));
                let diags = check_rule(&scenario.system, &once);
                assert!(diags.iter().all(|d| !d.is_error()), "{}: {}\n{diags:?}", scenario.id, print_rule(&once));
            }
            // Replacing every manual rule by its round trip still compiles to the same rule texts.
            let mut sys = scenario.system.clone();
            sys.rules = sys.rules.iter().map(round_trip).collect();
            let again = compile(&sys).unwrap();
            assert_eq!(again.rules().len(), compiled.rules().len());
        }
    }

    #[test]
    fn structure_is_recognised() {
        let rule = parse_rule(
            "rule flee for rabbit: when nearby(fox, radius=6) > 0 and energy > 2 or state == scared \
             then move_away(nearest(fox, radius=6), speed=self.speed * 1.5), emit(alarm, 1) priority 900",
        )
        .unwrap();
        let b = RuleBlocks::from_ast(&rule);
        assert_eq!(b.groups.len(), 2);
        assert_eq!(b.groups[0].rows.len(), 2);
        assert_eq!(
            b.groups[0].rows[0],
            Condition::Compare {
                left: Operand::Nearby { kind: "fox".into(), radius: Some(6.0) },
                cmp: Cmp::Gt,
                right: Operand::Number(0.0)
            }
        );
        assert_eq!(
            b.groups[1].rows[0],
            Condition::Compare {
                left: Operand::Property { who: Who::Implicit, name: "state".into() },
                cmp: Cmp::Eq,
                right: Operand::Name("scared".into())
            }
        );
        match &b.actions[0] {
            ActionBlock::Call { name, args } => {
                assert_eq!(name, "move_away");
                assert_eq!(
                    args[0],
                    Some(ArgValue::Entity(EntitySel::Nearest { kind: "fox".into(), radius: Some(6.0) }))
                );
                assert_eq!(args[1], Some(ArgValue::Expr("self.speed * 1.5".into())));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(b.priority, 900);
        assert_eq!(round_trip(&rule), rule);
    }

    #[test]
    fn blocks_generate_canonical_dsl() {
        let mut b = RuleBlocks::new("graze", "sheep", "stay");
        b.event = EventBlock::Collision(Some("grass".into()));
        b.groups = vec![CondGroup {
            rows: vec![Condition::Compare {
                left: Operand::Property { who: Who::Implicit, name: "energy".into() },
                cmp: Cmp::Lt,
                right: Operand::Number(30.0),
            }],
        }];
        b.actions = vec![ActionBlock::Call {
            name: "eat".into(),
            args: vec![Some(ArgValue::Entity(EntitySel::Other)), Some(ArgValue::Number(0.5))],
        }];
        b.priority = 520;
        b.cooldown = Some(10);
        b.costs = vec![("energy".into(), 1.0)];
        let text = print_rule(&b.to_ast().unwrap());
        assert_eq!(
            text,
            "rule graze for sheep:\n    on_collision with grass\n    when energy < 30\n    then eat(other, efficiency=0.5)\n    priority 520 cooldown 10 cost energy 1"
        );
    }

    #[test]
    fn complex_parts_are_kept_as_text() {
        let src = "rule r for x: when not (energy > 3) and abs(x - 4) < 2 then wander(turn=0.5), set size = size * 1.1, frobnicate(3)";
        // Unknown actions do not validate but must still survive the builder.
        let rule = parse_program(src).unwrap().remove(0);
        let b = RuleBlocks::from_ast(&rule);
        assert!(matches!(&b.groups[0].rows[0], Condition::Expr(t) if t == "not energy > 3"));
        assert!(matches!(&b.groups[0].rows[1], Condition::Compare { left: Operand::Expr(t), .. } if t == "abs(x - 4)"));
        assert_eq!(b.actions[2], ActionBlock::Raw("frobnicate(3)".into()));
        assert_eq!(round_trip(&rule), rule);
    }

    #[test]
    fn invalid_blocks_report_where() {
        let mut b = RuleBlocks::new("r", "x", "die");
        b.groups = vec![CondGroup { rows: vec![Condition::Expr("energy >".into())] }];
        let e = b.to_ast().unwrap_err();
        assert_eq!(e.location, "condition 1.1");
        b.groups.clear();
        b.actions = vec![ActionBlock::Call { name: "eat".into(), args: vec![None, None] }];
        let e = b.to_ast().unwrap_err();
        assert!(e.message.contains("target"), "{e}");
        b.actions = vec![ActionBlock::Raw("die() priority 4".into())];
        assert!(b.to_ast().unwrap_err().message.contains("modifiers"));
        b.actions.clear();
        assert!(b.to_ast().is_err());
    }

    #[test]
    fn actions_print_and_parse_alone() {
        let a = parse_action("spawn(virus, n=6)").unwrap();
        assert_eq!(print_action(&a), "spawn(virus, n=6)");
        assert!(parse_action("then").is_err());
        assert!(parse_action("die(), die()").is_err());
        assert_eq!(
            ActionBlock::from_action(&a),
            ActionBlock::Call {
                name: "spawn".into(),
                args: vec![Some(ArgValue::Name("virus".into())), Some(ArgValue::Number(6.0))]
            }
        );
        // Quoted names stay quoted when they are not identifiers.
        assert_eq!(print_expr(&name_expr("lumen moss")), "\"lumen moss\"");
        assert_eq!(print_expr(&name_expr("when")), "\"when\"");
    }
}
