//! Abstract syntax tree of the rule DSL.
//!
//! The AST is the *authoring* representation shared by the text editor and
//! a visual block editor: it derives serde with the default externally
//! tagged enum layout (round-trips through RON and JSON) and carries source
//! [`Span`]s that are neither serialised nor compared. After validation it is
//! lowered to the runtime IR in [`mod@crate::compile`].

use serde::{Deserialize, Serialize};

use crate::error::Span;

/// A name written in the source (entity type, state, ...), with its location.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct Name {
    /// The text of the name.
    pub text: String,
    /// Where it was written.
    pub span: Span,
}

impl Name {
    /// Name without a source location (generated code).
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), span: Span::default() }
    }
}

impl From<String> for Name {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<Name> for String {
    fn from(name: Name) -> Self {
        name.text
    }
}

/// When a rule is considered.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// Every tick (the default).
    #[default]
    Tick,
    /// On the first tick of the entity's life.
    Birth,
    /// When the entity dies (only `emit` and `spawn` are allowed).
    Death,
    /// While touching another entity, optionally of a given type. The
    /// touched entity is available as `other`.
    Collision {
        /// Entity type the other entity must have.
        with: Option<Name>,
    },
    /// On the tick after the entity entered the given state.
    StateEnter {
        /// The state.
        state: Name,
    },
}

/// Unary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnOp {
    /// Arithmetic negation `-x`.
    Neg,
    /// Logical negation `not x`.
    Not,
}

/// Binary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/` (division by zero yields 0)
    Div,
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
    /// `and`
    And,
    /// `or`
    Or,
}

impl BinOp {
    /// Source symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::And => "and",
            Self::Or => "or",
        }
    }

    /// Binding strength (higher binds tighter).
    pub fn precedence(self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
            Self::Lt | Self::Le | Self::Gt | Self::Ge | Self::Eq | Self::Ne => 4,
            Self::Add | Self::Sub => 5,
            Self::Mul | Self::Div => 6,
        }
    }

    /// Whether the operator is a comparison.
    pub fn is_comparison(self) -> bool {
        self.precedence() == 4
    }
}

/// An expression with its location.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Expr {
    /// The expression.
    pub kind: ExprKind,
    /// Where it was written.
    #[serde(skip)]
    pub span: Span,
}

impl Expr {
    /// Expression without a source location (generated code).
    pub fn new(kind: ExprKind) -> Self {
        Self { kind, span: Span::default() }
    }

    /// Numeric literal.
    pub fn num(value: f64) -> Self {
        Self::new(ExprKind::Num(value))
    }

    /// Dotted path such as `self.energy` or a bare name such as `virus`.
    pub fn path(segments: &[&str]) -> Self {
        Self::new(ExprKind::Path(segments.iter().map(|s| (*s).to_string()).collect()))
    }

    /// Function call with arguments.
    pub fn call(name: &str, args: Vec<Arg>) -> Self {
        Self::new(ExprKind::Call { name: name.to_string(), args })
    }

    /// Binary operation.
    pub fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Self {
        Self::new(ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) })
    }

    /// Conjunction of all `terms` (left-nested); `None` for an empty list.
    pub fn all(terms: Vec<Expr>) -> Option<Self> {
        terms.into_iter().reduce(|acc, term| Self::binary(BinOp::And, acc, term))
    }
}

/// Expression variants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ExprKind {
    /// Number literal.
    Num(f64),
    /// `true` / `false`.
    Bool(bool),
    /// String literal (names of types, states, resources).
    Str(String),
    /// `name` or `subject.name` (`self`, `organism`, `other`, `world`).
    Path(Vec<String>),
    /// Function call `name(args)`.
    Call {
        /// Function name.
        name: String,
        /// Positional arguments first, then named ones.
        args: Vec<Arg>,
    },
    /// Unary operation.
    Unary {
        /// Operator.
        op: UnOp,
        /// Operand.
        expr: Box<Expr>,
    },
    /// Binary operation.
    Binary {
        /// Operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
}

/// A call argument, optionally named (`radius=3`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arg {
    /// Argument name for named arguments.
    pub name: Option<String>,
    /// Value.
    pub value: Expr,
}

impl Arg {
    /// Positional argument.
    pub fn pos(value: Expr) -> Self {
        Self { name: None, value }
    }

    /// Named argument.
    pub fn named(name: &str, value: Expr) -> Self {
        Self { name: Some(name.to_string()), value }
    }
}

/// An action in a rule's `then` list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// Call-style action such as `eat(other)`.
    Call {
        /// Action name.
        name: String,
        /// Arguments.
        args: Vec<Arg>,
        /// Location.
        #[serde(skip)]
        span: Span,
    },
    /// `set path = value`.
    Set {
        /// Property path.
        path: Vec<String>,
        /// New value.
        value: Expr,
        /// Location.
        #[serde(skip)]
        span: Span,
    },
    /// `add path += value` or `add path -= value`.
    Add {
        /// Property path.
        path: Vec<String>,
        /// Amount.
        value: Expr,
        /// `-=` instead of `+=`.
        subtract: bool,
        /// Location.
        #[serde(skip)]
        span: Span,
    },
}

impl Action {
    /// Call-style action without a source location.
    pub fn call(name: &str, args: Vec<Arg>) -> Self {
        Self::Call { name: name.to_string(), args, span: Span::default() }
    }

    /// Location of the action.
    pub fn span(&self) -> Span {
        match self {
            Self::Call { span, .. } | Self::Set { span, .. } | Self::Add { span, .. } => *span,
        }
    }
}

/// `cost <property> <amount>`: charged when the rule fires.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// Property path (e.g. `energy`).
    pub path: Vec<String>,
    /// Amount subtracted.
    pub amount: f64,
    /// Location.
    #[serde(skip)]
    pub span: Span,
}

/// `produces <name> [amount]`: a product of the rule — a state the entity
/// enters, entities spawned, or a resource released at its position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Product {
    /// State, entity type or resource name.
    pub name: Name,
    /// Count or amount (1 by default).
    pub amount: f64,
}

/// Rule modifiers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleMeta {
    /// Higher priorities are tried first (default 0).
    pub priority: i32,
    /// Chance that an eligible rule fires.
    pub probability: Option<f64>,
    /// Minimum number of ticks between two firings.
    pub cooldown: Option<u64>,
    /// Costs charged on firing.
    pub costs: Vec<Cost>,
    /// Products of the rule.
    pub produces: Vec<Product>,
    /// Fires at most once per entity lifetime.
    pub once: bool,
}

/// One rule: `rule NAME for TYPE: [event] [when COND] then ACTIONS [modifiers]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleAst {
    /// Rule name (unique per entity type).
    pub name: String,
    /// Entity type the rule belongs to.
    pub entity: Name,
    /// Triggering event.
    pub event: Event,
    /// Optional condition.
    pub condition: Option<Expr>,
    /// Actions (at least one).
    pub actions: Vec<Action>,
    /// Modifiers.
    pub meta: RuleMeta,
    /// Location of the `rule` keyword.
    #[serde(skip)]
    pub span: Span,
}

impl RuleAst {
    /// Rule with the given name, entity type and actions; everything else default.
    pub fn new(name: &str, entity: &str, actions: Vec<Action>) -> Self {
        Self {
            name: name.to_string(),
            entity: Name::new(entity),
            event: Event::Tick,
            condition: None,
            actions,
            meta: RuleMeta::default(),
            span: Span::default(),
        }
    }
}
