//! Lowering of actions, events, products and whole rules.

use crate::compile::ir::{
    CompiledRule, MAX_SPAWN_PER_ACTION, PropRef, RAction, REvent, RExpr, RProduct, RuleOrigin, Slot,
};
use crate::compile::lower::{OtherInfo, RuleCx, action_names, params, sensor_names};
use crate::compile::symbols::Symbols;
use crate::def::ReproductionMode;
use crate::dsl::ast::{Action, Event, Expr, ExprKind, RuleAst};
use crate::dsl::parser::{MAX_EXPR_DEPTH, parse_rule};
use crate::dsl::printer::print_rule;
use crate::error::{Diagnostic, Span, did_you_mean};

/// Largest accepted `|priority|`. Priorities rank contested interactions as
/// `f32`, which represents every integer up to 2^24 exactly.
pub const MAX_PRIORITY: i32 = 1_000_000;

/// Deepest expression tree accepted from editors that build the AST
/// directly (the parser's own limit is stricter, see
/// [`crate::dsl::parser::MAX_EXPR_DEPTH`]).
const MAX_TREE_DEPTH: usize = 4 * MAX_EXPR_DEPTH;

impl RuleCx<'_> {
    pub(super) fn writable(&mut self, path: &[String], span: Span) -> Option<PropRef> {
        let name = match path {
            [n] => n,
            [s, n] if s == "self" || s == "organism" => n,
            [s, _] if s == "other" => {
                self.err("E008", "only the acting entity's own properties can be changed", span);
                return None;
            }
            _ => {
                self.err("E001", format!("cannot assign to `{}`", path.join(".")), span);
                return None;
            }
        };
        match self.sym.prop_of(self.me, name) {
            Some(p) if p.is_writable() => Some(p),
            Some(PropRef::Carried(_)) => {
                self.err_help(
                    "E008",
                    format!("carried resource `{name}` is read-only"),
                    span,
                    Some("change it with consume(), emit() or trade()".into()),
                );
                None
            }
            Some(_) => {
                self.err("E008", format!("`{name}` is read-only"), span);
                None
            }
            None => {
                let help = did_you_mean(name, self.sym.prop_names(self.me));
                let tname = self.sym.types[usize::from(self.me)].clone();
                self.err_help("E001", format!("unknown property `{name}` of `{tname}`"), span, help);
                None
            }
        }
    }

    fn require_mobile(&mut self, action: &str, span: Span) -> bool {
        if !self.sym.mobile[usize::from(self.me)] {
            let tname = self.sym.types[usize::from(self.me)].clone();
            self.err_help(
                "E017",
                format!("`{tname}` is sessile (movement: None) and cannot {action}"),
                span,
                Some("give the type a RandomWalk or Directed movement".into()),
            );
            return false;
        }
        true
    }

    fn action(&mut self, a: &Action, event: REvent) -> Option<RAction> {
        if event == REvent::Death && !matches!(a, Action::Call { .. }) {
            self.err_help(
                "E007",
                "set/add are not allowed in on_death rules (the entity is gone)",
                a.span(),
                Some("on_death rules may only emit() resources and spawn() entities".into()),
            );
            return None;
        }
        let (name, args, span) = match a {
            Action::Set { path, value, span } => {
                let prop = self.writable(path, *span);
                let value = self.num(value);
                return Some(RAction::SetProp { prop: prop?, value: value? });
            }
            Action::Add { path, value, subtract, span } => {
                let prop = self.writable(path, *span);
                let value = self.num(value)?;
                let value = if *subtract { RExpr::Neg(Box::new(value)) } else { value };
                return Some(RAction::AddProp { prop: prop?, value });
            }
            Action::Call { name, args, span } => (name.as_str(), args, *span),
        };
        if event == REvent::Death && !matches!(name, "emit" | "spawn") {
            self.err_help(
                "E007",
                format!("{name}() is not allowed in on_death rules"),
                span,
                Some("on_death rules may only emit() resources and spawn() entities".into()),
            );
            return None;
        }
        let b = |cx: &mut Self| cx.bind(name, params(name), args, span);
        Some(match name {
            "move_towards" | "move_away" => {
                let a = b(self)?;
                let mobile = self.require_mobile("move", span);
                let target = self.entity(a[0]?);
                let speed = self.opt_num(a[1]).ok()?;
                let target = target.filter(|_| mobile)?;
                if name == "move_towards" {
                    RAction::MoveTowards { target, speed }
                } else {
                    RAction::MoveAway { target, speed }
                }
            }
            "wander" => {
                let a = b(self)?;
                let mobile = self.require_mobile("wander", span);
                let turn = self.opt_num(a[0]).ok()?;
                let speed = self.opt_num(a[1]).ok()?;
                mobile.then_some(RAction::Wander { turn, speed })?
            }
            "stay" => {
                b(self)?;
                RAction::Stay
            }
            "eat" => {
                let a = b(self)?;
                let target = self.entity(a[0]?);
                let efficiency = self.opt_num(a[1]).ok()?;
                RAction::Eat { target: target?, efficiency }
            }
            "attack" => {
                let a = b(self)?;
                let target = self.entity(a[0]?);
                let damage = self.opt_num(a[1]).ok()?;
                RAction::Attack { target: target?, damage }
            }
            "reproduce" => {
                let a = b(self)?;
                if let Some(p) = a[0] {
                    self.probability_literal(p);
                }
                let probability = self.opt_num(a[0]).ok()?;
                let partner = match a[1] {
                    Some(e) => Some(self.entity(e)?),
                    None => None,
                };
                let tname = self.sym.types[usize::from(self.me)].clone();
                match (self.sym.reproduction[usize::from(self.me)], partner.is_some()) {
                    (ReproductionMode::None, _) => {
                        self.err_help(
                            "E018",
                            format!("`{tname}` cannot reproduce (reproduction mode None)"),
                            span,
                            Some("set the type's reproduction mode to Asexual or Sexual".into()),
                        );
                        return None;
                    }
                    (ReproductionMode::Asexual, true) => {
                        self.err("E004", format!("`{tname}` reproduces asexually; remove `partner=`"), span);
                        return None;
                    }
                    (ReproductionMode::Sexual, false) => {
                        self.err_help(
                            "E004",
                            format!("`{tname}` reproduces sexually and needs a partner"),
                            span,
                            Some(format!("reproduce(partner=nearest({tname}))")),
                        );
                        return None;
                    }
                    _ => {}
                }
                RAction::Reproduce { probability, partner }
            }
            "transform_into" => {
                let a = b(self)?;
                let e = a[0]?;
                let n = self.name_arg(e, "an entity type")?;
                RAction::Transform(self.type_name(&n, e.span)?)
            }
            "die" => {
                b(self)?;
                RAction::Die
            }
            "trade" => {
                let a = b(self)?;
                let target = self.entity(a[0]?);
                let give = self.carried_resource(a[1]?);
                let amount = self.num(a[2]?);
                let receive = self.carried_resource(a[3]?);
                let receive_amount = self.num(a[4]?);
                RAction::Trade {
                    target: target?,
                    give: give?,
                    amount: amount?,
                    receive: receive?,
                    receive_amount: receive_amount?,
                }
            }
            "emit" | "consume" => {
                let a = b(self)?;
                let resource = self.resource(a[0]?);
                let amount = self.num(a[1]?);
                let (resource, amount) = (resource?, amount?);
                if name == "emit" { RAction::Emit { resource, amount } } else { RAction::Consume { resource, amount } }
            }
            "set_state" => {
                let a = b(self)?;
                let e = a[0]?;
                let n = self.name_arg(e, "a state")?;
                match self.sym.local_state(self.me, &n) {
                    Some(s) => RAction::SetState(s),
                    None => {
                        let states = &self.sym.states[usize::from(self.me)];
                        let help = did_you_mean(&n, states.iter().map(String::as_str))
                            .or_else(|| Some(format!("states: {}", states.join(", "))));
                        self.err_help("E003", format!("unknown state `{n}`"), e.span, help);
                        return None;
                    }
                }
            }
            "spawn" => {
                let a = b(self)?;
                let e = a[0]?;
                let ty = self.name_arg(e, "an entity type").and_then(|n| self.type_name(&n, e.span));
                if let Some(c) = a[1]
                    && let ExprKind::Num(n) = c.kind
                    && !(0.0..=f64::from(MAX_SPAWN_PER_ACTION)).contains(&n)
                {
                    self.err("E012", format!("spawn count must be in [0, {MAX_SPAWN_PER_ACTION}] (got {n})"), c.span);
                }
                let count = match a[1] {
                    Some(c) => self.num(c)?,
                    None => RExpr::Num(1.0),
                };
                RAction::Spawn { ty: ty?, count }
            }
            _ => {
                let help = if sensor_names().any(|f| f == name) {
                    Some(format!("`{name}` is a sensor; use it in the `when` condition"))
                } else {
                    did_you_mean(name, action_names())
                };
                self.err_help("E014", format!("unknown action `{name}`"), span, help);
                return None;
            }
        })
    }

    fn carried_resource(&mut self, e: &Expr) -> Option<u16> {
        let r = self.resource(e)?;
        if self.sym.carried[usize::from(self.me)].contains(&r) {
            Some(r)
        } else {
            let tname = self.sym.types[usize::from(self.me)].clone();
            let rname = self.sym.resources[usize::from(r)].clone();
            self.err_help(
                "E003",
                format!("`{tname}` does not carry `{rname}`"),
                e.span,
                Some("add it to the type's `carries` list".into()),
            );
            None
        }
    }
}

/// Resolves the event of a rule or script for entity type `me`.
pub(crate) fn lower_event(
    sym: &Symbols,
    me: u16,
    event: &Event,
    context: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<REvent> {
    let mut cx = RuleCx { sym, me, other: OtherInfo::Unavailable, context: context.to_string(), diags: Vec::new() };
    let r = match event {
        Event::Tick => Some(REvent::Tick),
        Event::Birth => Some(REvent::Birth),
        Event::Death => Some(REvent::Death),
        Event::Collision { with: None } => Some(REvent::Collision(None)),
        Event::Collision { with: Some(n) } => cx.type_name(&n.text, n.span).map(|t| REvent::Collision(Some(t))),
        Event::StateEnter { state } => match sym.local_state(me, &state.text) {
            Some(s) => Some(REvent::StateEnter(s)),
            None => {
                let states = &sym.states[usize::from(me)];
                cx.err_help(
                    "E003",
                    format!("unknown state `{}`", state.text),
                    state.span,
                    Some(format!("states: {}", states.join(", "))),
                );
                None
            }
        },
    };
    diags.append(&mut cx.diags);
    r
}

/// Validates and lowers one rule. Returns the rule (when there were no
/// errors) and all diagnostics, warnings included.
pub(crate) fn lower_rule(sym: &Symbols, ast: &RuleAst, origin: RuleOrigin) -> (Option<CompiledRule>, Vec<Diagnostic>) {
    let context = format!("rule `{}` ({})", ast.name, ast.entity.text);
    let mut diags = Vec::new();
    let Some(me) = sym.type_idx(&ast.entity.text) else {
        let help = did_you_mean(&ast.entity.text, sym.types.iter().map(String::as_str))
            .or_else(|| Some(format!("known entity types: {}", sym.types.join(", "))));
        let mut d = Diagnostic::error("E003", format!("unknown entity type `{}`", ast.entity.text))
            .context(context)
            .at(ast.entity.span);
        if let Some(h) = help {
            d = d.help(h);
        }
        return (None, vec![d]);
    };
    let event = lower_event(sym, me, &ast.event, &context, &mut diags);
    let other = match event {
        Some(REvent::Collision(None)) => OtherInfo::Any,
        Some(REvent::Collision(Some(t))) => OtherInfo::Type(t),
        _ => OtherInfo::Unavailable,
    };
    let mut cx = RuleCx { sym, me, other, context, diags };
    let condition = ast.condition.as_ref().map(|c| cx.boolean(c));
    let ev = event.unwrap_or(REvent::Tick);
    let actions: Vec<Option<RAction>> = ast.actions.iter().map(|a| cx.action(a, ev)).collect();
    let span = ast.span;

    let m = &ast.meta;
    if let Some(p) = m.probability
        && !(0.0..=1.0).contains(&p)
    {
        cx.err("E005", format!("probability must be between 0 and 1 (got {p})"), span);
    }
    if m.priority.unsigned_abs() > MAX_PRIORITY.unsigned_abs() {
        cx.err(
            "E012",
            format!("priority must be between -{MAX_PRIORITY} and {MAX_PRIORITY} (got {})", m.priority),
            span,
        );
    }
    if ev == REvent::Death && !m.costs.is_empty() {
        cx.err_help(
            "E007",
            "an on_death rule cannot have a cost (nothing is left to pay it)",
            m.costs[0].span,
            Some("remove the `cost` modifier".into()),
        );
    }
    let mut costs = Vec::new();
    for c in &m.costs {
        if !(c.amount.is_finite() && c.amount >= 0.0) {
            cx.err("E009", format!("cost must be a non-negative number (got {})", c.amount), c.span);
        }
        if let Some(p) = cx.writable(&c.path, c.span) {
            costs.push((p, c.amount));
        }
    }
    let mut products = Vec::new();
    for p in &m.produces {
        if let Some(prod) = lower_product(&mut cx, &p.name.text, p.amount, p.name.span, ev) {
            products.push(prod);
        }
    }

    let mut uses_move = false;
    let mut uses_act = false;
    let mut uses_transform = false;
    for (a, src) in actions.iter().zip(&ast.actions) {
        let Some(a) = a else { continue };
        match a.slot() {
            Slot::Move if uses_move => {
                cx.err("E010", "a rule can contain only one movement action", src.span());
            }
            Slot::Act if uses_act => cx.err_help(
                "E010",
                "a rule can do only one of eat/attack/reproduce/die/trade",
                src.span(),
                Some("split the rule in two".into()),
            ),
            Slot::Transform if uses_transform => {
                cx.err("E010", "a rule can transform the entity only once", src.span());
            }
            Slot::Move => uses_move = true,
            Slot::Act => uses_act = true,
            Slot::Transform => uses_transform = true,
            Slot::Effect => {}
        }
    }
    let spawns = actions.iter().flatten().any(|a| matches!(a, RAction::Spawn { .. }))
        || products.iter().any(|p| matches!(p, RProduct::Spawn { .. }));
    let bounded = m.cooldown.is_some_and(|c| c > 0)
        || m.probability.is_some_and(|p| p < 1.0)
        || m.once
        || !m.costs.is_empty()
        || ast.condition.is_some()
        || !matches!(ev, REvent::Tick);
    if spawns && !bounded {
        cx.diags.push(
            Diagnostic::warning("W105", "spawns every tick without condition, cooldown, probability or cost")
                .context(cx.context.clone())
                .at(span)
                .help("the population will grow without bound"),
        );
    }

    let diags = cx.diags;
    if diags.iter().any(Diagnostic::is_error) {
        return (None, diags);
    }
    let (Some(event), Some(actions)) = (event, actions.into_iter().collect::<Option<Vec<_>>>()) else {
        return (None, diags);
    };
    let condition = match condition {
        Some(Some(c)) => Some(c),
        Some(None) => return (None, diags),
        None => None,
    };
    let rule = CompiledRule {
        name: ast.name.clone(),
        entity: me,
        event,
        condition,
        actions,
        priority: m.priority,
        probability: m.probability,
        cooldown: m.cooldown.unwrap_or(0),
        once: m.once,
        costs,
        products,
        origin,
        ast: ast.clone(),
        text: print_rule(ast),
        uses_move,
        uses_act,
        uses_transform,
    };
    (Some(rule), diags)
}

/// Whether an expression tree is deeper than `limit` (iterative, so it is
/// safe on arbitrarily deep trees built by editors).
fn deeper_than(root: &Expr, limit: usize) -> bool {
    let mut stack = vec![(root, 1usize)];
    while let Some((e, depth)) = stack.pop() {
        if depth > limit {
            return true;
        }
        match &e.kind {
            ExprKind::Unary { expr, .. } => stack.push((expr, depth + 1)),
            ExprKind::Binary { lhs, rhs, .. } => {
                stack.push((lhs, depth + 1));
                stack.push((rhs, depth + 1));
            }
            ExprKind::Call { args, .. } => stack.extend(args.iter().map(|a| (&a.value, depth + 1))),
            ExprKind::Num(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Path(_) => {}
        }
    }
    false
}

fn rule_exprs(ast: &RuleAst) -> impl Iterator<Item = &Expr> {
    ast.condition.iter().chain(ast.actions.iter().flat_map(|a| -> Vec<&Expr> {
        match a {
            Action::Call { args, .. } => args.iter().map(|x| &x.value).collect(),
            Action::Set { value, .. } | Action::Add { value, .. } => vec![value],
        }
    }))
}

/// Validates and lowers a hand-written rule, which (unlike generated rules)
/// is stored as text: on top of [`lower_rule`] it bounds the depth of
/// editor-built trees, requires a name, and checks that the canonical text
/// parses back to the same rule, so that saving and loading (files,
/// snapshots) never changes or loses it.
pub(crate) fn check_manual_rule(
    sym: &Symbols,
    ast: &RuleAst,
    origin: RuleOrigin,
) -> (Option<CompiledRule>, Vec<Diagnostic>) {
    let context = format!("rule `{}` ({})", ast.name, ast.entity.text);
    if rule_exprs(ast).any(|e| deeper_than(e, MAX_TREE_DEPTH)) {
        let d = Diagnostic::error("E015", format!("expression nested deeper than {MAX_EXPR_DEPTH} levels"))
            .context(context)
            .at(ast.span);
        return (None, vec![d]);
    }
    let (rule, mut diags) = lower_rule(sym, ast, origin);
    if ast.name.trim().is_empty() {
        diags.push(
            Diagnostic::error("E020", "rule names must not be empty")
                .context(context.clone())
                .at(ast.span)
                .help("rule \"my rule\" for TYPE: ..."),
        );
        return (None, diags);
    }
    if diags.iter().any(Diagnostic::is_error) {
        return (rule, diags);
    }
    let text = print_rule(ast);
    match parse_rule(&text) {
        Ok(back) if back == *ast => (rule, diags),
        Ok(_) => {
            diags.push(
                Diagnostic::error(
                    "E015",
                    "this rule has no faithful text form: it would change when saved and reloaded",
                )
                .context(context)
                .at(ast.span)
                .help(format!("its text reads: {}", text.replace('\n', " "))),
            );
            (None, diags)
        }
        Err(errors) => {
            let first = errors.first().map_or_else(String::new, |e| e.message.clone());
            diags.push(
                Diagnostic::error("E015", format!("this rule cannot be written as rule text ({first})"))
                    .context(context)
                    .at(ast.span)
                    .help(
                        "names of properties, functions and arguments must be identifiers; text must fit on one line",
                    ),
            );
            (None, diags)
        }
    }
}

fn lower_product(cx: &mut RuleCx<'_>, name: &str, amount: f64, span: Span, event: REvent) -> Option<RProduct> {
    if !(amount.is_finite() && amount > 0.0) {
        cx.err("E012", format!("product amount must be positive (got {amount})"), span);
        return None;
    }
    if let Some(s) = cx.sym.local_state(cx.me, name) {
        if event == REvent::Death {
            cx.err("E007", "an on_death rule cannot produce a state", span);
            return None;
        }
        return Some(RProduct::State(s));
    }
    if let Some(t) = cx.sym.type_idx(name) {
        if amount.fract() != 0.0 || amount > f64::from(MAX_SPAWN_PER_ACTION) {
            cx.err(
                "E012",
                format!("number of produced `{name}` must be a whole number ≤ {MAX_SPAWN_PER_ACTION}"),
                span,
            );
            return None;
        }
        return Some(RProduct::Spawn { ty: t, count: amount as u32 });
    }
    if let Some(r) = cx.sym.resource_idx(name) {
        return Some(RProduct::Resource { resource: r, amount });
    }
    cx.err_help("E003", format!("`{name}` is neither a state of this type, an entity type nor a resource"), span, None);
    None
}
