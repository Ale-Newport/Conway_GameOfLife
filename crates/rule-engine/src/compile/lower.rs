//! Validation and lowering of DSL rules to the runtime IR.
//!
//! Names are resolved against [`Symbols`] built from the definitions, every
//! expression is type-checked (number, true/false, entity, state, type,
//! name), call arguments are bound against fixed signatures, and every
//! problem becomes a [`Diagnostic`] with the rule name and line:column.

use crate::compile::ir::{ArithOp, CmpOp, FieldRef, KindSel, MAX_SENSE_RADIUS, PropRef, RExpr, Subject};
use crate::compile::symbols::Symbols;
use crate::dsl::ast::{Arg, BinOp, Expr, ExprKind, UnOp};
use crate::error::{Diagnostic, Span, did_you_mean};

/// What `other` refers to in the rule being checked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum OtherInfo {
    Unavailable,
    Any,
    Type(u16),
}

/// Result of typing an expression.
pub(super) enum Typed {
    Num(RExpr),
    Bool(RExpr),
    Entity(RExpr),
    /// A bare name or string literal (meaning decided by the context).
    Name(String),
    State(Subject),
    Kind(Subject),
}

impl Typed {
    fn describe(&self) -> &'static str {
        match self {
            Self::Num(_) => "a number",
            Self::Bool(_) => "true/false",
            Self::Entity(_) => "an entity",
            Self::Name(_) => "a name",
            Self::State(_) => "a state",
            Self::Kind(_) => "an entity type",
        }
    }
}

use crate::compile::vocab::ParamSig;
pub(super) use crate::compile::vocab::{action_names, params, sensor_names};

pub(super) struct RuleCx<'a> {
    pub(super) sym: &'a Symbols,
    pub(super) me: u16,
    pub(super) other: OtherInfo,
    pub(super) context: String,
    pub(super) diags: Vec<Diagnostic>,
}

impl RuleCx<'_> {
    pub(super) fn err(&mut self, code: &str, message: impl Into<String>, span: Span) {
        self.diags.push(Diagnostic::error(code, message).context(self.context.clone()).at(span));
    }

    pub(super) fn err_help(&mut self, code: &str, message: impl Into<String>, span: Span, help: Option<String>) {
        let mut d = Diagnostic::error(code, message).context(self.context.clone()).at(span);
        if let Some(h) = help {
            d = d.help(h);
        }
        self.diags.push(d);
    }

    // --------------------------------------------------------------- typing

    pub(super) fn expr(&mut self, e: &Expr) -> Option<Typed> {
        match &e.kind {
            ExprKind::Num(n) if !n.is_finite() => {
                self.err("E002", format!("numbers must be finite (got {n})"), e.span);
                None
            }
            ExprKind::Num(n) => Some(Typed::Num(RExpr::Num(*n))),
            ExprKind::Bool(b) => Some(Typed::Bool(RExpr::Bool(*b))),
            ExprKind::Str(s) => Some(Typed::Name(s.clone())),
            ExprKind::Path(segs) => self.path(segs, e.span),
            ExprKind::Call { name, args } => self.sensor(name, args, e.span),
            ExprKind::Unary { op: UnOp::Neg, expr } => Some(Typed::Num(RExpr::Neg(Box::new(self.num(expr)?)))),
            ExprKind::Unary { op: UnOp::Not, expr } => Some(Typed::Bool(RExpr::Not(Box::new(self.boolean(expr)?)))),
            ExprKind::Binary { op, lhs, rhs } => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                    let (l, r) = (self.num(lhs), self.num(rhs));
                    let op = match op {
                        BinOp::Add => ArithOp::Add,
                        BinOp::Sub => ArithOp::Sub,
                        BinOp::Mul => ArithOp::Mul,
                        _ => ArithOp::Div,
                    };
                    Some(Typed::Num(RExpr::Arith(op, Box::new(l?), Box::new(r?))))
                }
                BinOp::And | BinOp::Or => {
                    let (l, r) = (self.boolean(lhs), self.boolean(rhs));
                    let (l, r) = (Box::new(l?), Box::new(r?));
                    Some(Typed::Bool(if *op == BinOp::And { RExpr::And(l, r) } else { RExpr::Or(l, r) }))
                }
                _ => self.compare(*op, lhs, rhs, e.span),
            },
        }
    }

    /// Reports a name used where a value is expected: an unknown property
    /// for bare words, a type mismatch for string literals.
    pub(super) fn misplaced_name(&mut self, name: &str, e: &Expr, expected: &str) {
        if matches!(e.kind, ExprKind::Str(_)) {
            self.err("E002", format!("expected {expected}, found the text \"{name}\""), e.span);
        } else {
            self.unknown_name(name, e.span);
        }
    }

    pub(super) fn unknown_name(&mut self, name: &str, span: Span) {
        let mut candidates = self.sym.prop_names(self.me);
        candidates.extend(["tick", "state", "type", "self", "other"]);
        let help = did_you_mean(name, candidates).or_else(|| {
            Some("names of types, states and resources must be passed to a function, e.g. nearby(name)".into())
        });
        self.err_help("E001", format!("unknown property `{name}`"), span, help);
    }

    pub(super) fn num(&mut self, e: &Expr) -> Option<RExpr> {
        match self.expr(e)? {
            Typed::Num(r) => Some(r),
            Typed::Name(n) => {
                self.misplaced_name(&n, e, "a number");
                None
            }
            other => {
                self.err("E002", format!("expected a number, found {}", other.describe()), e.span);
                None
            }
        }
    }

    pub(super) fn boolean(&mut self, e: &Expr) -> Option<RExpr> {
        match self.expr(e)? {
            Typed::Bool(r) => Some(r),
            Typed::Name(n) => {
                self.misplaced_name(&n, e, "true/false");
                None
            }
            other => {
                let help = matches!(other, Typed::Num(_)).then(|| "compare it, e.g. `energy > 10`".to_string());
                self.err_help("E002", format!("expected true/false, found {}", other.describe()), e.span, help);
                None
            }
        }
    }

    pub(super) fn entity(&mut self, e: &Expr) -> Option<RExpr> {
        match self.expr(e)? {
            Typed::Entity(r) => Some(r),
            other => {
                self.err_help(
                    "E002",
                    format!("expected an entity, found {}", other.describe()),
                    e.span,
                    Some("use `other`, `self` or `nearest(TYPE)`".into()),
                );
                None
            }
        }
    }

    pub(super) fn require_other(&mut self, span: Span) -> bool {
        if self.other == OtherInfo::Unavailable {
            self.err_help(
                "E013",
                "`other` is only available in on_collision rules",
                span,
                Some("add `on_collision with TYPE` after the rule header".into()),
            );
            return false;
        }
        true
    }

    pub(super) fn path(&mut self, segs: &[String], span: Span) -> Option<Typed> {
        match segs {
            [n] => match n.as_str() {
                "tick" => Some(Typed::Num(RExpr::Tick)),
                "self" | "organism" => Some(Typed::Entity(RExpr::SelfEntity)),
                "other" => self.require_other(span).then_some(Typed::Entity(RExpr::OtherEntity)),
                "state" => Some(Typed::State(Subject::Me)),
                "type" => Some(Typed::Kind(Subject::Me)),
                _ => match self.sym.prop_of(self.me, n) {
                    Some(p) => Some(Typed::Num(RExpr::Prop(Subject::Me, p))),
                    None => Some(Typed::Name(n.clone())),
                },
            },
            [s, p] => match s.as_str() {
                "self" | "organism" => self.member(Subject::Me, p, span),
                "other" => {
                    if self.require_other(span) {
                        self.member(Subject::Other, p, span)
                    } else {
                        None
                    }
                }
                "world" if p == "tick" => Some(Typed::Num(RExpr::Tick)),
                "world" => {
                    self.err("E001", format!("unknown world property `{p}` (only `world.tick` exists)"), span);
                    None
                }
                _ => {
                    self.err_help(
                        "E001",
                        format!("unknown subject `{s}`"),
                        span,
                        Some("properties are read from `self`, `other` or `world`".into()),
                    );
                    None
                }
            },
            _ => {
                self.err("E001", format!("`{}` has too many parts", segs.join(".")), span);
                None
            }
        }
    }

    pub(super) fn member(&mut self, subject: Subject, name: &str, span: Span) -> Option<Typed> {
        match name {
            "state" => return Some(Typed::State(subject)),
            "type" => return Some(Typed::Kind(subject)),
            _ => {}
        }
        if let Some(p) = PropRef::builtin(name) {
            return Some(Typed::Num(RExpr::Prop(subject, p)));
        }
        let ty = match (subject, self.other) {
            (Subject::Me, _) => self.me,
            (Subject::Other, OtherInfo::Type(t)) => t,
            (Subject::Other, _) => {
                self.err_help(
                    "E001",
                    format!("`other.{name}`: `other` can be any type here, so only built-in properties can be read"),
                    span,
                    Some("write `on_collision with TYPE` to read that type's properties".into()),
                );
                return None;
            }
        };
        match self.sym.prop_of(ty, name) {
            Some(p) => Some(Typed::Num(RExpr::Prop(subject, p))),
            None => {
                let help = did_you_mean(name, self.sym.prop_names(ty));
                let tname = self.sym.types[usize::from(ty)].clone();
                self.err_help("E001", format!("unknown property `{name}` of `{tname}`"), span, help);
                None
            }
        }
    }

    pub(super) fn compare(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) -> Option<Typed> {
        let l = self.expr(lhs);
        let r = self.expr(rhs);
        let (l, r) = (l?, r?);
        let negate = op == BinOp::Ne;
        let equality = matches!(op, BinOp::Eq | BinOp::Ne);
        match (l, r) {
            (Typed::State(s), Typed::Name(n)) | (Typed::Name(n), Typed::State(s)) => {
                if !equality {
                    self.err("E002", "states can only be compared with == or !=", span);
                    return None;
                }
                let id = self.state_id(s, &n, span)?;
                Some(Typed::Bool(RExpr::StateIs(s, id, negate)))
            }
            (Typed::Kind(s), Typed::Name(n)) | (Typed::Name(n), Typed::Kind(s)) => {
                if !equality {
                    self.err("E002", "entity types can only be compared with == or !=", span);
                    return None;
                }
                let t = self.type_name(&n, span)?;
                Some(Typed::Bool(RExpr::TypeIs(s, t, negate)))
            }
            (Typed::Num(a), Typed::Num(b)) => {
                let op = match op {
                    BinOp::Lt => CmpOp::Lt,
                    BinOp::Le => CmpOp::Le,
                    BinOp::Gt => CmpOp::Gt,
                    BinOp::Ge => CmpOp::Ge,
                    BinOp::Eq => CmpOp::Eq,
                    _ => CmpOp::Ne,
                };
                Some(Typed::Bool(RExpr::Cmp(op, Box::new(a), Box::new(b))))
            }
            (Typed::Bool(a), Typed::Bool(b)) if equality => {
                Some(Typed::Bool(RExpr::BoolEq(Box::new(a), Box::new(b), negate)))
            }
            (Typed::Name(n), Typed::Num(_)) => {
                self.misplaced_name(&n, lhs, "a number");
                None
            }
            (Typed::Num(_), Typed::Name(n)) => {
                self.misplaced_name(&n, rhs, "a number");
                None
            }
            (a, b) => {
                self.err("E002", format!("cannot compare {} with {}", a.describe(), b.describe()), span);
                None
            }
        }
    }

    pub(super) fn state_id(&mut self, subject: Subject, name: &str, span: Span) -> Option<u32> {
        let ty = match (subject, self.other) {
            (Subject::Me, _) => Some(self.me),
            (Subject::Other, OtherInfo::Type(t)) => Some(t),
            _ => None,
        };
        let found = match ty {
            Some(t) => self.sym.local_state(t, name).map(|l| self.sym.state_ids[usize::from(t)][usize::from(l)]),
            None => self.sym.state_names.iter().position(|s| s == name).map(|i| i as u32),
        };
        if found.is_none() {
            let candidates: Vec<&str> = match ty {
                Some(t) => self.sym.states[usize::from(t)].iter().map(String::as_str).collect(),
                None => self.sym.state_names.iter().map(String::as_str).collect(),
            };
            let help = did_you_mean(name, candidates.iter().copied())
                .or_else(|| Some(format!("known states: {}", candidates.join(", "))));
            self.err_help("E003", format!("unknown state `{name}`"), span, help);
        }
        found
    }

    pub(super) fn type_name(&mut self, name: &str, span: Span) -> Option<u16> {
        let t = self.sym.type_idx(name);
        if t.is_none() {
            let help = did_you_mean(name, self.sym.types.iter().map(String::as_str))
                .or_else(|| Some(format!("known entity types: {}", self.sym.types.join(", "))));
            self.err_help("E003", format!("unknown entity type `{name}`"), span, help);
        }
        t
    }

    /// A name argument: a string literal or a bare single-word name.
    pub(super) fn name_arg(&mut self, e: &Expr, what: &str) -> Option<String> {
        match &e.kind {
            ExprKind::Str(s) => Some(s.clone()),
            ExprKind::Path(segs) if segs.len() == 1 => Some(segs[0].clone()),
            _ => {
                self.err("E002", format!("expected {what} name"), e.span);
                None
            }
        }
    }

    pub(super) fn kind(&mut self, e: &Expr) -> Option<KindSel> {
        let name = self.name_arg(e, "an entity type")?;
        match name.as_str() {
            "any" => Some(KindSel::Any),
            "same_species" | "same_type" | "same" => Some(KindSel::Same),
            _ => self.type_name(&name, e.span).map(KindSel::Type),
        }
    }

    pub(super) fn resource(&mut self, e: &Expr) -> Option<u16> {
        let name = self.name_arg(e, "a resource")?;
        let r = self.sym.resource_idx(&name);
        if r.is_none() {
            let help = did_you_mean(&name, self.sym.resources.iter().map(String::as_str))
                .or_else(|| Some(format!("resources: {}", self.sym.resources.join(", "))));
            self.err_help("E003", format!("unknown resource `{name}`"), e.span, help);
        }
        r
    }

    pub(super) fn field(&mut self, e: &Expr) -> Option<FieldRef> {
        let name = self.name_arg(e, "a field")?;
        if let Some(r) = self.sym.resource_idx(&name) {
            return Some(FieldRef::Resource(r));
        }
        if let Some(v) = self.sym.env_idx(&name) {
            return Some(FieldRef::Env(v));
        }
        let all = self.sym.resources.iter().chain(&self.sym.env).map(String::as_str);
        let help = did_you_mean(&name, all);
        self.err_help("E003", format!("unknown field `{name}`"), e.span, help);
        None
    }

    pub(super) fn radius(&mut self, e: Option<&Expr>) -> Option<f32> {
        let Some(e) = e else { return Some(self.sym.perception[usize::from(self.me)]) };
        match e.kind {
            ExprKind::Num(r) if r > 0.0 && r <= f64::from(MAX_SENSE_RADIUS) => Some(r as f32),
            ExprKind::Num(r) => {
                let shown = crate::dsl::printer::print_number(r);
                self.err("E006", format!("radius {shown} must be in (0, {MAX_SENSE_RADIUS}]"), e.span);
                None
            }
            _ => {
                self.err("E006", "radius must be a number literal", e.span);
                None
            }
        }
    }

    pub(super) fn probability_literal(&mut self, e: &Expr) {
        if let ExprKind::Num(p) = e.kind
            && !(0.0..=1.0).contains(&p)
        {
            self.err("E005", format!("probability must be between 0 and 1 (got {p})"), e.span);
        }
    }

    pub(super) fn bind<'e>(
        &mut self,
        func: &str,
        params: &[ParamSig],
        args: &'e [Arg],
        span: Span,
    ) -> Option<Vec<Option<&'e Expr>>> {
        let mut out: Vec<Option<&Expr>> = vec![None; params.len()];
        let mut ok = true;
        let mut seen_named = false;
        let mut next_pos = 0;
        let signature = || params.iter().map(|p| p.name).collect::<Vec<_>>().join(", ");
        for arg in args {
            match &arg.name {
                None if seen_named => {
                    self.err("E004", format!("{func}(): positional argument after a named one"), arg.value.span);
                    ok = false;
                }
                None => {
                    if next_pos >= params.len() {
                        self.err_help(
                            "E004",
                            format!("{func}() takes at most {} argument(s)", params.len()),
                            arg.value.span,
                            Some(format!("{func}({})", signature())),
                        );
                        ok = false;
                    } else {
                        out[next_pos] = Some(&arg.value);
                        next_pos += 1;
                    }
                }
                Some(name) => {
                    seen_named = true;
                    match params.iter().position(|p| p.name == name) {
                        Some(i) if out[i].is_some() => {
                            self.err("E004", format!("{func}(): argument `{name}` given twice"), arg.value.span);
                            ok = false;
                        }
                        Some(i) => out[i] = Some(&arg.value),
                        None => {
                            self.err_help(
                                "E004",
                                format!("{func}() has no argument `{name}`"),
                                arg.value.span,
                                Some(format!("{func}({})", signature())),
                            );
                            ok = false;
                        }
                    }
                }
            }
        }
        for (p, v) in params.iter().zip(&out) {
            if p.required && v.is_none() {
                self.err_help(
                    "E004",
                    format!("{func}() needs the argument `{}`", p.name),
                    span,
                    Some(format!("{func}({})", signature())),
                );
                ok = false;
            }
        }
        ok.then_some(out)
    }

    pub(super) fn opt_num(&mut self, e: Option<&Expr>) -> Result<Option<RExpr>, ()> {
        match e {
            None => Ok(None),
            Some(e) => self.num(e).map(Some).ok_or(()),
        }
    }

    pub(super) fn sensor(&mut self, name: &str, args: &[Arg], span: Span) -> Option<Typed> {
        let b = |cx: &mut Self| cx.bind(name, params(name), args, span);
        let boxed = |r: RExpr| Box::new(r);
        Some(match name {
            "nearby" | "nearest" => {
                let a = b(self)?;
                let (kind, radius) = (self.kind(a[0]?), self.radius(a[1]));
                let (kind, radius) = (kind?, radius?);
                if name == "nearby" {
                    Typed::Num(RExpr::Nearby(kind, radius))
                } else {
                    Typed::Entity(RExpr::Nearest(kind, radius))
                }
            }
            "distance" | "exists" => {
                let a = b(self)?;
                let target = boxed(self.entity(a[0]?)?);
                if name == "distance" {
                    Typed::Num(RExpr::Distance(target))
                } else {
                    Typed::Bool(RExpr::Exists(target))
                }
            }
            "field" => {
                let a = b(self)?;
                Typed::Num(RExpr::Field(self.field(a[0]?)?))
            }
            "random" => {
                b(self)?;
                Typed::Num(RExpr::Random)
            }
            "chance" => {
                let a = b(self)?;
                let p = a[0]?;
                self.probability_literal(p);
                Typed::Bool(RExpr::Chance(boxed(self.num(p)?)))
            }
            "min" | "max" => {
                let a = b(self)?;
                let (x, y) = (self.num(a[0]?), self.num(a[1]?));
                let (x, y) = (boxed(x?), boxed(y?));
                Typed::Num(if name == "min" { RExpr::Min(x, y) } else { RExpr::Max(x, y) })
            }
            "abs" => {
                let a = b(self)?;
                Typed::Num(RExpr::Abs(boxed(self.num(a[0]?)?)))
            }
            "clamp" => {
                let a = b(self)?;
                let (x, lo, hi) = (self.num(a[0]?), self.num(a[1]?), self.num(a[2]?));
                Typed::Num(RExpr::Clamp(boxed(x?), boxed(lo?), boxed(hi?)))
            }
            "population" => {
                let a = b(self)?;
                Typed::Num(RExpr::Population(self.kind(a[0]?)?))
            }
            _ => {
                let help = if action_names().any(|a| a == name) {
                    Some(format!("`{name}` is an action; actions go after `then`"))
                } else {
                    did_you_mean(name, sensor_names())
                };
                self.err_help("E014", format!("unknown function `{name}`"), span, help);
                return None;
            }
        })
    }
}
