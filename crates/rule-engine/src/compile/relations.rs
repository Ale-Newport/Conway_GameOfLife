//! Relationship compiler: every relationship (and every entity type with a
//! reproduction `rate`) becomes ordinary DSL rules, which then go through
//! the same validation and lowering as hand-written rules. The generated
//! text is shown to the user (`CompiledSystem::rules`, the view's rule list).
//!
//! Templates (`r` = radius, `S`/`O` = subject/object, `p` = priority band +
//! `priority_shift`):
//!
//! | Relation | Generated rules (for the subject) |
//! |---|---|
//! | `S consumes O` (type) | forage: `when energy < hunger and nearby(O, radius=r) > 0 then move_towards(nearest(O, radius=r))` p500 (mobile subjects only); eat: `on_collision with O when energy < hunger then eat(other, efficiency=e)` p520 `cooldown handling` |
//! | `S consumes O` (resource) | feed: `when energy < hunger and field(O) > 0 then consume(O, bite)` p510 |
//! | `S protects O` | for each enemy `T` of `O` (types that consume or chase `O`, or that `O` avoids): defend: `when nearby(O, radius=r) > 0 and nearby(T, radius=r) > 0 then move_towards(nearest(T, radius=r))` p700; strike: `on_collision with T when nearby(O, radius=r) > 0 then attack(other, damage=d)` p710; and stay close: `when nearby(O, radius=2r) > 0 and distance(nearest(O, radius=2r)) > r then move_towards(nearest(O, radius=2r))` p150 |
//! | `S chases O` | `when nearby(O, radius=r) > 0 then move_towards(nearest(O, radius=r), speed=self.speed * f)` p450 |
//! | `S avoids O` | `when nearby(O, radius=r) > 0 then move_away(nearest(O, radius=r), speed=self.speed * f)` p900 |
//! | `S transforms O` | `[on_collision with T] [when cond] then transform_into(O) probability q` p600 |
//! | `S produces O` | `then emit(O, a)` / `then spawn(O, a)` with `probability q`, `cost energy c` p100 |
//! | `S reproduces_with O` | `when age >= maturity and energy >= max(cost, min_energy) and nearby(O, radius=r) > 0 then reproduce(partner=nearest(O, radius=r), probability=q)` p300 |
//! | `S competes_with O` | `on_collision with O then attack(other, damage=d) probability a` p400, and the mirror rule for `O` |
//! | `S trades_with O` | approach: `when self.G >= g and nearby(O, radius=r) > 0 then move_towards(nearest(O, radius=r))` p200 (mobile only); trade: `on_collision with O when self.G >= g then trade(other, give=G, amount=g, receive=R, receive_amount=h)` p210 |
//! | type reproduction `rate > 0` | `when age >= maturity and energy >= 2 * cost [and nearby(same, radius=perception) > 0] then reproduce([partner=nearest(same, radius=perception), ]probability=rate)` p300 |
//!
//! Generated rule names are `"S kind O: what"`; when several relationships
//! share subject, kind and object, the later ones get a ` #n` suffix (`n` =
//! the 1-based relationship index) so that every name stays unique and
//! stable. Warnings: `W103` unconditional transformation cycle, `W201`
//! contradictory edges, `W204` protection without enemies, `W205`
//! competition without shared food.

use crate::compile::ir::RuleOrigin;
use crate::def::{LifeSystem, RelationKind, Relationship, ReproductionMode, TransformTrigger};
use crate::dsl::ast::{Action, Arg, BinOp, Cost, Event, Expr, ExprKind, Name, RuleAst};
use crate::dsl::parser::parse_expr;
use crate::error::{Diagnostic, ItemRef, item_index};

fn name_expr(name: &str) -> Expr {
    Expr::new(ExprKind::Str(name.to_string()))
}

fn radius_args(kind: &str, radius: f64) -> Vec<Arg> {
    vec![Arg::pos(name_expr(kind)), Arg::named("radius", Expr::num(radius))]
}

fn nearby_gt0(kind: &str, radius: f64) -> Expr {
    Expr::binary(BinOp::Gt, Expr::call("nearby", radius_args(kind, radius)), Expr::num(0.0))
}

fn nearest(kind: &str, radius: f64) -> Expr {
    Expr::call("nearest", radius_args(kind, radius))
}

fn energy_below(threshold: Option<f64>) -> Option<Expr> {
    threshold.map(|h| Expr::binary(BinOp::Lt, Expr::path(&["energy"]), Expr::num(h)))
}

struct Gen<'a> {
    rel: &'a Relationship,
    index: usize,
    /// ` #n` when an earlier relationship has the same subject, kind and object.
    suffix: String,
    out: Vec<(RuleAst, RuleOrigin)>,
}

impl Gen<'_> {
    fn rule(
        &mut self,
        subject: &str,
        what: &str,
        band: i32,
        event: Event,
        condition: Option<Expr>,
        actions: Vec<Action>,
    ) {
        let name = format!("{} {} {}{}: {what}", self.rel.subject, self.rel.kind.label(), self.rel.object, self.suffix);
        let mut rule = RuleAst::new(&name, subject, actions);
        rule.event = event;
        rule.condition = condition;
        rule.meta.priority = band + self.rel.priority_shift;
        let origin = RuleOrigin::Relationship { index: self.index, relation: self.rel.kind.label().to_string() };
        self.out.push((rule, origin));
    }

    fn last(&mut self) -> Option<&mut RuleAst> {
        self.out.last_mut().map(|(r, _)| r)
    }
}

fn collision_with(t: &str) -> Event {
    Event::Collision { with: Some(Name::new(t)) }
}

/// Enemies of `target`: types that consume or chase it, or that it avoids.
fn enemies_of(sys: &LifeSystem, target: &str) -> Vec<String> {
    let mut out: Vec<String> = sys
        .relationships
        .iter()
        .filter_map(|r| match r.kind {
            RelationKind::Consumes { .. } | RelationKind::Chases { .. } if r.object == target => {
                Some(r.subject.clone())
            }
            RelationKind::Avoids { .. } if r.subject == target => Some(r.object.clone()),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The cycle `from → to → … → from` through `transforms` relationships with
/// the `Always` trigger, if `to` leads back to `from` that way.
fn unconditional_cycle(sys: &LifeSystem, from: &str, to: &str) -> Option<Vec<String>> {
    let always = |a: &str| -> Vec<&str> {
        sys.relationships
            .iter()
            .filter(|r| {
                r.subject == a && matches!(r.kind, RelationKind::Transforms { trigger: TransformTrigger::Always, .. })
            })
            .map(|r| r.object.as_str())
            .collect()
    };
    // Breadth-first search from `to`, remembering predecessors.
    let mut prev: Vec<(&str, &str)> = vec![(to, from)];
    let mut queue = std::collections::VecDeque::from([to]);
    while let Some(n) = queue.pop_front() {
        if n == from {
            let mut path = vec![from.to_string()];
            let mut cur = from;
            loop {
                let p = prev.iter().find(|(x, _)| *x == cur).map(|(_, p)| *p)?;
                path.push(p.to_string());
                if p == from {
                    break;
                }
                cur = p;
                if path.len() > sys.entity_types.len() + 2 {
                    break;
                }
            }
            path.reverse();
            return Some(path);
        }
        for next in always(n) {
            if !prev.iter().any(|(x, _)| *x == next) {
                prev.push((next, n));
                queue.push_back(next);
            }
        }
    }
    None
}

fn is_mobile(sys: &LifeSystem, name: &str) -> bool {
    sys.entity_types.iter().find(|t| t.name == name).is_some_and(|t| t.movement.is_mobile())
}

/// Generates the rules of all relationships and type-level reproduction,
/// with warnings for contradictory or ineffective relationships. The input
/// definitions must already be valid.
pub(crate) fn generate(sys: &LifeSystem) -> (Vec<(RuleAst, RuleOrigin)>, Vec<Diagnostic>) {
    let mut rules = Vec::new();
    let mut diags = Vec::new();
    for (index, rel) in sys.relationships.iter().enumerate() {
        let repeated = sys.relationships[..index]
            .iter()
            .any(|r| r.subject == rel.subject && r.object == rel.object && r.kind.label() == rel.kind.label());
        let suffix = if repeated { format!(" #{}", index + 1) } else { String::new() };
        let mut g = Gen { rel, index, suffix, out: Vec::new() };
        let (s, o) = (rel.subject.as_str(), rel.object.as_str());
        let ctx = format!("relationship #{} ({s} {} {o})", index + 1, rel.kind.label());
        let first_diag = diags.len();
        let object_is_resource = sys.resource_index(o).is_some();
        match &rel.kind {
            RelationKind::Consumes { efficiency, hunger, radius, bite, handling } => {
                if object_is_resource {
                    let cond = Expr::all(
                        energy_below(*hunger)
                            .into_iter()
                            .chain([Expr::binary(
                                BinOp::Gt,
                                Expr::call("field", vec![Arg::pos(name_expr(o))]),
                                Expr::num(0.0),
                            )])
                            .collect(),
                    );
                    let act = Action::call("consume", vec![Arg::pos(name_expr(o)), Arg::pos(Expr::num(*bite))]);
                    g.rule(s, "feed", 510, Event::Tick, cond, vec![act]);
                } else {
                    if is_mobile(sys, s) {
                        let cond =
                            Expr::all(energy_below(*hunger).into_iter().chain([nearby_gt0(o, *radius)]).collect());
                        let act = Action::call("move_towards", vec![Arg::pos(nearest(o, *radius))]);
                        g.rule(s, "forage", 500, Event::Tick, cond, vec![act]);
                    }
                    let act = Action::call(
                        "eat",
                        vec![Arg::pos(Expr::path(&["other"])), Arg::named("efficiency", Expr::num(*efficiency))],
                    );
                    g.rule(s, "eat", 520, collision_with(o), energy_below(*hunger), vec![act]);
                    if *handling > 0
                        && let Some(r) = g.last()
                    {
                        r.meta.cooldown = Some(*handling);
                    }
                }
                let protects = sys
                    .relationships
                    .iter()
                    .any(|r| matches!(r.kind, RelationKind::Protects { .. }) && r.subject == s && r.object == o);
                if protects {
                    diags.push(
                        Diagnostic::warning("W201", format!("`{s}` both consumes and protects `{o}`")).context(&ctx),
                    );
                }
            }
            RelationKind::Protects { radius, damage } => {
                let enemies = enemies_of(sys, o);
                if enemies.is_empty() {
                    diags.push(
                        Diagnostic::warning("W204", format!("`{o}` has no enemies, so protecting it has no effect"))
                            .context(&ctx)
                            .help("enemies are types that consume or chase it, or that it avoids"),
                    );
                }
                for t in &enemies {
                    let cond = Expr::all(vec![nearby_gt0(o, *radius), nearby_gt0(t, *radius)]);
                    if is_mobile(sys, s) {
                        let act = Action::call("move_towards", vec![Arg::pos(nearest(t, *radius))]);
                        g.rule(s, &format!("defend against {t}"), 700, Event::Tick, cond, vec![act]);
                    }
                    let act = Action::call(
                        "attack",
                        vec![Arg::pos(Expr::path(&["other"])), Arg::named("damage", Expr::num(*damage))],
                    );
                    g.rule(s, &format!("strike {t}"), 710, collision_with(t), Some(nearby_gt0(o, *radius)), vec![act]);
                }
                if is_mobile(sys, s) {
                    let far = 2.0 * radius;
                    let cond = Expr::all(vec![
                        nearby_gt0(o, far),
                        Expr::binary(
                            BinOp::Gt,
                            Expr::call("distance", vec![Arg::pos(nearest(o, far))]),
                            Expr::num(*radius),
                        ),
                    ]);
                    let act = Action::call("move_towards", vec![Arg::pos(nearest(o, far))]);
                    g.rule(s, "stay close", 150, Event::Tick, cond, vec![act]);
                }
            }
            RelationKind::Chases { radius, speed_factor } | RelationKind::Avoids { radius, speed_factor } => {
                let chase = matches!(rel.kind, RelationKind::Chases { .. });
                let speed = Expr::binary(BinOp::Mul, Expr::path(&["self", "speed"]), Expr::num(*speed_factor));
                let action = if chase { "move_towards" } else { "move_away" };
                let act = Action::call(action, vec![Arg::pos(nearest(o, *radius)), Arg::named("speed", speed)]);
                let (what, band) = if chase { ("chase", 450) } else { ("flee", 900) };
                g.rule(s, what, band, Event::Tick, Some(nearby_gt0(o, *radius)), vec![act]);
                let opposite = sys.relationships.iter().any(|r| {
                    r.subject == s
                        && r.object == o
                        && matches!(
                            (&r.kind, chase),
                            (RelationKind::Avoids { .. }, true) | (RelationKind::Chases { .. }, false)
                        )
                });
                if opposite && chase {
                    diags
                        .push(Diagnostic::warning("W201", format!("`{s}` both chases and avoids `{o}`")).context(&ctx));
                }
            }
            RelationKind::Transforms { trigger, probability } => {
                let act = Action::call("transform_into", vec![Arg::pos(name_expr(o))]);
                let (event, cond) = match trigger {
                    TransformTrigger::Always => (Event::Tick, None),
                    TransformTrigger::Contact(t) => (collision_with(t), None),
                    TransformTrigger::AfterTicks(n) => {
                        (Event::Tick, Some(Expr::binary(BinOp::Ge, Expr::path(&["state_age"]), Expr::num(*n as f64))))
                    }
                    TransformTrigger::Condition(text) => match parse_expr(text) {
                        Ok(e) => (Event::Tick, Some(e)),
                        Err(errs) => {
                            for d in errs {
                                diags.push(d.context(format!("{ctx}, trigger condition")));
                            }
                            continue;
                        }
                    },
                };
                g.rule(s, "transform", 600, event, cond, vec![act]);
                if *probability < 1.0
                    && let Some(r) = g.last()
                {
                    r.meta.probability = Some(*probability);
                }
                if matches!(trigger, TransformTrigger::Always)
                    && let Some(cycle) = unconditional_cycle(sys, s, o)
                {
                    diags.push(
                        Diagnostic::warning(
                            "W103",
                            format!(
                                "unconditional transformation cycle {}: entities keep changing type",
                                cycle.join(" → ")
                            ),
                        )
                        .context(&ctx)
                        .help("give one of the transformations a condition, a contact or an AfterTicks trigger"),
                    );
                }
            }
            RelationKind::Produces { amount, probability, cost } => {
                let act = if object_is_resource {
                    Action::call("emit", vec![Arg::pos(name_expr(o)), Arg::pos(Expr::num(*amount))])
                } else {
                    Action::call("spawn", vec![Arg::pos(name_expr(o)), Arg::pos(Expr::num(*amount))])
                };
                g.rule(s, "produce", 100, Event::Tick, None, vec![act]);
                if let Some(r) = g.last() {
                    if *probability < 1.0 {
                        r.meta.probability = Some(*probability);
                    }
                    if *cost > 0.0 {
                        r.meta.costs.push(Cost {
                            path: vec!["energy".into()],
                            amount: *cost,
                            span: Default::default(),
                        });
                        r.condition = Some(Expr::binary(BinOp::Ge, Expr::path(&["energy"]), Expr::num(*cost)));
                    }
                }
            }
            RelationKind::ReproducesWith { radius, probability, min_energy } => {
                let Some(t) = sys.entity_types.iter().find(|t| t.name == s) else { continue };
                let need = t.reproduction.cost.max(*min_energy);
                let cond = Expr::all(vec![
                    Expr::binary(BinOp::Ge, Expr::path(&["age"]), Expr::num(t.lifecycle.maturity as f64)),
                    Expr::binary(BinOp::Ge, Expr::path(&["energy"]), Expr::num(need)),
                    nearby_gt0(o, *radius),
                ]);
                let act = Action::call(
                    "reproduce",
                    vec![
                        Arg::named("probability", Expr::num(*probability)),
                        Arg::named("partner", nearest(o, *radius)),
                    ],
                );
                g.rule(s, "mate", 300, Event::Tick, cond, vec![act]);
            }
            RelationKind::CompetesWith { aggression, damage } => {
                let shares_food = |a: &str| -> Vec<&str> {
                    sys.relationships
                        .iter()
                        .filter(|r| r.subject == a && matches!(r.kind, RelationKind::Consumes { .. }))
                        .map(|r| r.object.as_str())
                        .collect()
                };
                let (fs, fo) = (shares_food(s), shares_food(o));
                if !fs.iter().any(|f| fo.contains(f)) {
                    diags.push(
                        Diagnostic::warning("W205", format!("`{s}` and `{o}` compete but share no food relationship"))
                            .context(&ctx),
                    );
                }
                let pairs: &[(&str, &str)] = if s == o { &[(s, o)] } else { &[(s, o), (o, s)] };
                for &(a, b) in pairs {
                    let act = Action::call(
                        "attack",
                        vec![Arg::pos(Expr::path(&["other"])), Arg::named("damage", Expr::num(*damage))],
                    );
                    g.rule(a, &format!("fight {b}"), 400, collision_with(b), None, vec![act]);
                    if *aggression < 1.0
                        && let Some(r) = g.last()
                    {
                        r.meta.probability = Some(*aggression);
                    }
                }
            }
            RelationKind::TradesWith { give, give_amount, receive, receive_amount, radius } => {
                let has_enough = Expr::binary(BinOp::Ge, Expr::path(&["self", give]), Expr::num(*give_amount));
                if is_mobile(sys, s) {
                    let cond = Expr::all(vec![has_enough.clone(), nearby_gt0(o, *radius)]);
                    let act = Action::call("move_towards", vec![Arg::pos(nearest(o, *radius))]);
                    g.rule(s, "approach", 200, Event::Tick, cond, vec![act]);
                }
                let act = Action::call(
                    "trade",
                    vec![
                        Arg::pos(Expr::path(&["other"])),
                        Arg::named("give", name_expr(give)),
                        Arg::named("amount", Expr::num(*give_amount)),
                        Arg::named("receive", name_expr(receive)),
                        Arg::named("receive_amount", Expr::num(*receive_amount)),
                    ],
                );
                g.rule(s, "trade", 210, collision_with(o), Some(has_enough), vec![act]);
            }
        }
        for d in &mut diags[first_diag..] {
            d.item.get_or_insert(ItemRef::Relationship(item_index(index)));
        }
        rules.append(&mut g.out);
    }
    for t in &sys.entity_types {
        let r = &t.reproduction;
        if r.rate <= 0.0 || r.mode == ReproductionMode::None {
            continue;
        }
        // Reproduce when the parent can pay the cost and keep as much as it gives.
        let mut terms = vec![
            Expr::binary(BinOp::Ge, Expr::path(&["age"]), Expr::num(t.lifecycle.maturity as f64)),
            Expr::binary(BinOp::Ge, Expr::path(&["energy"]), Expr::num(2.0 * r.cost)),
        ];
        let mut args = vec![Arg::named("probability", Expr::num(r.rate))];
        if r.mode == ReproductionMode::Sexual {
            let radius = f64::from(t.perception);
            terms.push(nearby_gt0("same_species", radius));
            args.push(Arg::named("partner", nearest("same_species", radius)));
        }
        let mut rule = RuleAst::new(&format!("{}: reproduce", t.name), &t.name, vec![Action::call("reproduce", args)]);
        rule.condition = Expr::all(terms);
        rule.meta.priority = 300;
        rules.push((rule, RuleOrigin::EntityType { entity: t.name.clone() }));
    }
    (rules, diags)
}
