//! Decision phase: for every entity (in id order, optionally in parallel)
//! evaluate its event rules and tick rules in priority order, then its
//! scripts, then fall back to its default movement.
//!
//! A rule is *eligible* when its event is active, its cooldown has expired,
//! its costs are affordable (counting costs reserved by rules that already
//! fired for this entity this tick), its condition holds, its probability
//! roll succeeds and all its actions can be carried out (targets exist,
//! `eat`/`attack`/`trade` targets touch the entity, reproduction is
//! possible). An eligible rule claims its movement/exclusive slots
//! atomically: if a slot is already taken by a higher-priority rule the
//! whole rule is skipped. Effects never conflict.
//!
//! Decisions read only the [`Frame`] and draw randomness from streams keyed
//! by (seed, tick, entity id, rule), so the outcome does not depend on
//! evaluation order or thread count.

use std::sync::Arc;

use organism_engine::kinematics::{MotionLimits, heading_vector};
use organism_engine::steering;
use rayon::prelude::*;
use simulation_core::SimRng;

use crate::compile::CompiledType;
use crate::compile::ir::{PropRef, RAction, REvent, RProduct};
use crate::script::ScriptError;
use crate::sim::eval::EvalCx;
use crate::sim::frame::{Frame, Target};
use crate::sim::intent::{
    Act, ActIntent, DEFAULT_MOVE_STREAM, Effect, Firing, Motion, Source, keyed_rng, rule_stream, script_realize_stream,
    script_stream,
};

/// Populations below this are decided sequentially (rayon overhead).
const PARALLEL_THRESHOLD: usize = 256;

/// Tolerance on contact distances (floating-point slack).
const CONTACT_SLACK: f32 = 1e-4;

/// Decisions of one entity.
#[derive(Debug, Default)]
pub struct AgentDecision {
    /// Firings in claim order.
    pub firings: Vec<Firing>,
    /// Script failures `(script index, error)`.
    pub errors: Vec<(u32, ScriptError)>,
}

/// Decides for every entity; `disabled[i]` marks disabled scripts.
pub fn decide_all(frame: &Arc<Frame>, disabled: &[bool]) -> Vec<AgentDecision> {
    if frame.len() < PARALLEL_THRESHOLD {
        (0..frame.len()).map(|slot| decide_agent(frame, slot, disabled)).collect()
    } else {
        (0..frame.len()).into_par_iter().map(|slot| decide_agent(frame, slot, disabled)).collect()
    }
}

fn finite_or(x: f64, default: f64) -> f64 {
    if x.is_finite() { x } else { default }
}

/// Evaluates one action's arguments; `None` when a target is missing.
fn eval_action(cx: &mut EvalCx<'_>, a: &RAction, ty: &CompiledType) -> Option<Act> {
    let me_speed = f64::from(cx.frame.agent(cx.me).speed);
    let speed = |cx: &mut EvalCx<'_>, s: &Option<crate::compile::ir::RExpr>| match s {
        Some(e) => finite_or(cx.num(e), me_speed).clamp(0.0, 50.0) as f32,
        None => me_speed as f32,
    };
    Some(match a {
        RAction::MoveTowards { target, speed: s } => {
            let target = cx.entity(target)?;
            Act::MoveTowards { target, speed: speed(cx, s) }
        }
        RAction::MoveAway { target, speed: s } => {
            let target = cx.entity(target)?;
            Act::MoveAway { target, speed: speed(cx, s) }
        }
        RAction::Wander { turn, speed: s } => {
            let turn = match turn {
                Some(e) => finite_or(cx.num(e), 0.0).clamp(0.0, std::f64::consts::PI) as f32,
                None => ty.default_turn(),
            };
            Act::Wander { turn, speed: speed(cx, s) }
        }
        RAction::Stay => Act::Stay,
        RAction::Eat { target, efficiency } => {
            let target = cx.entity(target)?;
            let efficiency = efficiency.as_ref().map_or(1.0, |e| finite_or(cx.num(e), 1.0).clamp(0.0, 1.0));
            Act::Eat { target, efficiency }
        }
        RAction::Attack { target, damage } => {
            let target = cx.entity(target)?;
            let damage = damage.as_ref().map_or(5.0, |e| finite_or(cx.num(e), 0.0).max(0.0));
            Act::Attack { target, damage }
        }
        RAction::Reproduce { probability, partner } => {
            let probability = probability.as_ref().map_or(1.0, |e| finite_or(cx.num(e), 0.0).clamp(0.0, 1.0));
            let partner = match partner {
                Some(e) => Some(cx.entity(e)?),
                None => None,
            };
            Act::Reproduce { probability, partner }
        }
        RAction::Transform(t) => Act::Transform(*t),
        RAction::Die => Act::Die,
        RAction::Trade { target, give, amount, receive, receive_amount } => {
            let target = cx.entity(target)?;
            let amount = finite_or(cx.num(amount), 0.0).max(0.0);
            let receive_amount = finite_or(cx.num(receive_amount), 0.0).max(0.0);
            Act::Trade { target, give: *give, amount, receive: *receive, receive_amount }
        }
        RAction::Emit { resource, amount } => {
            Act::Emit { resource: *resource, amount: finite_or(cx.num(amount), 0.0).max(0.0) }
        }
        RAction::Consume { resource, amount } => {
            Act::Consume { resource: *resource, amount: finite_or(cx.num(amount), 0.0).max(0.0) }
        }
        RAction::SetProp { prop, value } => Act::SetProp { prop: *prop, value: cx.num(value) },
        RAction::AddProp { prop, value } => Act::AddProp { prop: *prop, value: cx.num(value) },
        RAction::SetState(s) => Act::SetState(*s),
        RAction::Spawn { ty, count } => {
            let n =
                finite_or(cx.num(count), 0.0).round().clamp(0.0, f64::from(crate::compile::ir::MAX_SPAWN_PER_ACTION));
            Act::Spawn { ty: *ty, count: n as u32 }
        }
    })
}

/// Movement/exclusive/effect parts of a firing.
#[derive(Default)]
struct Plan {
    motion: Option<Motion>,
    act: Option<ActIntent>,
    transform: Option<u16>,
    effects: Vec<Effect>,
}

fn limits(speed: f32) -> MotionLimits {
    MotionLimits::new(speed, f32::INFINITY)
}

fn touching(frame: &Frame, me: usize, t: &Target) -> bool {
    t.slot as usize != me && t.dist <= frame.contact_distance(me, t.slot as usize) + CONTACT_SLACK
}

/// Whether a trade can be carried out: both parties carry both resources,
/// the giver has `amount`, the partner has `receive_amount`, and both have
/// room for what they receive. Carried stocks do not change between the
/// decision and the moment trades are applied, so a trade that passes this
/// check succeeds (and one that fails is never charged or counted).
fn trade_possible(
    frame: &Frame,
    me: usize,
    other: usize,
    give: u16,
    amount: f64,
    receive: u16,
    receive_amount: f64,
) -> bool {
    let stock = |slot: usize, res: u16| -> Option<(f64, f64)> {
        let e = frame.agent(slot);
        let ty = &frame.compiled.types()[usize::from(e.kind)];
        let i = ty.carried.iter().position(|r| *r == res)?;
        Some((e.carried.get(i).copied().unwrap_or(0.0), ty.def.carries[i].capacity))
    };
    let (Some(my_give), Some(my_recv), Some(their_give), Some(their_recv)) =
        (stock(me, give), stock(me, receive), stock(other, give), stock(other, receive))
    else {
        return false;
    };
    give != receive
        && my_give.0 >= amount
        && their_recv.0 >= receive_amount
        && my_recv.0 + receive_amount <= my_recv.1
        && their_give.0 + amount <= their_give.1
}

/// Whether the entity in `slot` may reproduce now with `reserved` energy
/// already committed this tick: mature, off its reproduction cooldown and
/// with more energy than the cost (so the parent survives paying it).
fn can_reproduce(frame: &Frame, slot: usize, reserved: f64) -> bool {
    let a = frame.agent(slot);
    let ty = &frame.compiled.types()[usize::from(a.kind)];
    a.age >= ty.def.lifecycle.maturity
        && frame.tick >= a.next_reproduction
        && a.energy - reserved > ty.def.reproduction.cost
}

/// Turns evaluated actions into a plan; `None` when an action cannot be
/// carried out (the rule is then not eligible).
fn realize(frame: &Frame, me: usize, acts: Vec<Act>, rng: &mut SimRng, reserved_energy: f64) -> Option<Plan> {
    let a = frame.agent(me);
    let ty = &frame.compiled.types()[usize::from(a.kind)];
    let kin = a.kin;
    let mut plan = Plan::default();
    for act in acts {
        match act {
            Act::MoveTowards { target, speed } => {
                let accel = steering::arrive(&kin, target.pos, speed.max(1e-6), limits(speed));
                plan.motion = Some(Motion { accel, speed });
            }
            Act::MoveAway { target, speed } => {
                plan.motion = Some(Motion { accel: steering::flee(&kin, target.pos, limits(speed)), speed });
            }
            Act::Wander { turn, speed } => {
                let heading = kin.heading + rng.range_f32(-turn, turn);
                let accel = heading_vector(heading) * speed - kin.velocity;
                plan.motion = Some(Motion { accel, speed });
            }
            Act::Stay => plan.motion = Some(Motion { accel: -kin.velocity, speed: 0.0 }),
            Act::Eat { target, efficiency } => {
                if !touching(frame, me, &target) {
                    return None;
                }
                plan.act = Some(ActIntent::Eat { target: target.slot, distance: target.dist, efficiency });
            }
            Act::Attack { target, damage } => {
                if !touching(frame, me, &target) {
                    return None;
                }
                plan.act = Some(ActIntent::Attack { target: target.slot, distance: target.dist, damage });
            }
            Act::Trade { target, give, amount, receive, receive_amount } => {
                if !touching(frame, me, &target)
                    || !trade_possible(frame, me, target.slot as usize, give, amount, receive, receive_amount)
                {
                    return None;
                }
                plan.act = Some(ActIntent::Trade {
                    target: target.slot,
                    distance: target.dist,
                    give,
                    amount,
                    receive,
                    receive_amount,
                });
            }
            Act::Reproduce { probability, partner } => {
                if !can_reproduce(frame, me, reserved_energy) || !rng.chance(probability) {
                    return None;
                }
                plan.act = Some(match partner {
                    None => ActIntent::Reproduce,
                    Some(t) => {
                        let p = t.slot as usize;
                        let mate_ok = p != me
                            && ty.mates.contains(&frame.agent(p).kind)
                            && can_reproduce(frame, p, 0.0)
                            && t.dist <= ty.def.perception.max(frame.contact_distance(me, p)) + CONTACT_SLACK;
                        if !mate_ok {
                            return None;
                        }
                        ActIntent::Mate { partner: t.slot, distance: t.dist }
                    }
                });
            }
            Act::Transform(t) => plan.transform = Some(t),
            Act::Die => plan.act = Some(ActIntent::Die),
            Act::Emit { resource, amount } => plan.effects.push(Effect::Emit { resource, amount }),
            Act::Consume { resource, amount } => plan.effects.push(Effect::Consume { resource, amount }),
            Act::SetProp { prop, value } => plan.effects.push(Effect::SetProp { prop, value }),
            Act::AddProp { prop, value } => plan.effects.push(Effect::AddProp { prop, value }),
            Act::SetState(s) => plan.effects.push(Effect::SetState(s)),
            Act::Spawn { ty, count } => plan.effects.push(Effect::Spawn { ty, count }),
        }
    }
    Some(plan)
}

fn product_effects(products: &[RProduct]) -> impl Iterator<Item = Effect> + '_ {
    products.iter().map(|p| match *p {
        RProduct::State(s) => Effect::SetState(s),
        RProduct::Spawn { ty, count } => Effect::Spawn { ty, count },
        RProduct::Resource { resource, amount } => Effect::Release { resource, amount },
    })
}

/// Lazily computed contacts per `with` filter.
struct Contacts {
    cache: Vec<(Option<u16>, Option<Target>)>,
}

impl Contacts {
    fn get(&mut self, frame: &Frame, me: usize, with: Option<u16>) -> Option<Target> {
        if let Some((_, t)) = self.cache.iter().find(|(w, _)| *w == with) {
            return *t;
        }
        let t = frame.contact(me, with);
        self.cache.push((with, t));
        t
    }
}

/// Whether an event is active for the entity; `Err(())` when inactive,
/// otherwise the bound `other`.
fn event_binding(frame: &Frame, me: usize, event: REvent, contacts: &mut Contacts) -> Result<Option<Target>, ()> {
    let a = frame.agent(me);
    match event {
        REvent::Tick => Ok(None),
        REvent::Birth if a.birth_pending => Ok(None),
        REvent::StateEnter(s) if a.state_enter_pending && a.state == s => Ok(None),
        REvent::Collision(with) => contacts.get(frame, me, with).map(Some).ok_or(()),
        _ => Err(()),
    }
}

fn reserved(costs: &[(PropRef, f64)], prop: PropRef) -> f64 {
    costs.iter().filter(|(p, _)| *p == prop).map(|(_, c)| c).sum()
}

fn decide_agent(frame: &Arc<Frame>, me: usize, disabled: &[bool]) -> AgentDecision {
    let compiled = &frame.compiled;
    let a = frame.agent(me);
    let id = frame.id(me);
    let ty = &compiled.types()[usize::from(a.kind)];
    let tick = frame.tick;
    let mut out = AgentDecision::default();
    let mut contacts = Contacts { cache: Vec::new() };
    let (mut move_taken, mut act_taken, mut transform_taken) = (false, false, false);
    let mut committed: Vec<(PropRef, f64)> = Vec::new();

    for (local, &ri) in ty.rules.iter().enumerate() {
        let rule = &compiled.rules()[ri as usize];
        if (rule.uses_move && move_taken) || (rule.uses_act && act_taken) || (rule.uses_transform && transform_taken) {
            continue;
        }
        if a.cooldowns.get(local).copied().unwrap_or(0) > tick {
            continue;
        }
        let Ok(other) = event_binding(frame, me, rule.event, &mut contacts) else { continue };
        let affordable = rule
            .costs
            .iter()
            .all(|(p, _)| frame.prop(me, *p) - reserved(&committed, *p) - reserved(&rule.costs, *p) >= 0.0);
        if !affordable {
            continue;
        }
        let mut cx = EvalCx { frame, me, other, rng: keyed_rng(frame.seed, tick, id, rule_stream(ri)) };
        if let Some(cond) = &rule.condition
            && !cx.boolean(cond)
        {
            continue;
        }
        if let Some(p) = rule.probability
            && !cx.rng.chance(p)
        {
            continue;
        }
        let Some(acts) = rule.actions.iter().map(|act| eval_action(&mut cx, act, ty)).collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let energy_reserved = reserved(&committed, PropRef::Energy) + reserved(&rule.costs, PropRef::Energy);
        let mut rng = cx.rng;
        let Some(mut plan) = realize(frame, me, acts, &mut rng, energy_reserved) else { continue };
        plan.effects.extend(product_effects(&rule.products));
        move_taken |= plan.motion.is_some();
        act_taken |= plan.act.is_some();
        transform_taken |= plan.transform.is_some();
        committed.extend(rule.costs.iter().copied());
        let cooldown = if rule.once {
            Some((local, u64::MAX))
        } else if rule.cooldown > 0 {
            Some((local, tick.saturating_add(rule.cooldown)))
        } else {
            None
        };
        out.firings.push(Firing {
            slot: me as u32,
            source: Source::Rule(ri),
            priority: rule.priority,
            costs: rule.costs.clone(),
            cooldown,
            motion: plan.motion,
            act: plan.act,
            transform: plan.transform,
            effects: plan.effects,
        });
    }

    let host = compiled.scripts();
    for &si in &ty.scripts {
        let script = &host.scripts()[si as usize];
        if !script.enabled || disabled.get(si as usize).copied().unwrap_or(false) || script.event == REvent::Death {
            continue;
        }
        let Ok(other) = event_binding(frame, me, script.event, &mut contacts) else { continue };
        let rng = keyed_rng(frame.seed, tick, id, script_stream(si));
        let acts = match host.run(si as usize, frame, me, other, rng) {
            Ok(acts) => acts,
            Err(e) => {
                out.errors.push((si, e));
                continue;
            }
        };
        if acts.is_empty() {
            continue;
        }
        // Independent of the draws the script made with random().
        let mut rng = keyed_rng(frame.seed, tick, id, script_realize_stream(si));
        let Some(plan) = realize(frame, me, acts, &mut rng, reserved(&committed, PropRef::Energy)) else { continue };
        if (plan.motion.is_some() && move_taken)
            || (plan.act.is_some() && act_taken)
            || (plan.transform.is_some() && transform_taken)
        {
            continue;
        }
        move_taken |= plan.motion.is_some();
        act_taken |= plan.act.is_some();
        transform_taken |= plan.transform.is_some();
        out.firings.push(Firing {
            slot: me as u32,
            source: Source::Script(si),
            priority: script.priority,
            costs: Vec::new(),
            cooldown: None,
            motion: plan.motion,
            act: plan.act,
            transform: plan.transform,
            effects: plan.effects,
        });
    }

    if !move_taken && ty.def.movement.is_mobile() {
        let mut rng = keyed_rng(frame.seed, tick, id, DEFAULT_MOVE_STREAM);
        let act = Act::Wander { turn: ty.default_turn(), speed: a.speed };
        if let Some(plan) = realize(frame, me, vec![act], &mut rng, 0.0) {
            out.firings.push(Firing {
                slot: me as u32,
                source: Source::DefaultMove,
                priority: i32::MIN,
                costs: Vec::new(),
                cooldown: None,
                motion: plan.motion,
                act: None,
                transform: None,
                effects: Vec::new(),
            });
        }
    }
    out
}

/// Effects of the `on_death` rules and scripts of a dying entity, evaluated
/// on `frame` (built from the end-of-tick state, before the dead are
/// removed). Returns the effects (only emit/spawn/release), the rules that
/// fired and script failures.
pub fn decide_death(
    frame: &Arc<Frame>,
    me: usize,
    disabled: &[bool],
) -> (Vec<Effect>, Vec<u32>, Vec<(u32, ScriptError)>) {
    let compiled = &frame.compiled;
    let a = frame.agent(me);
    let id = frame.id(me);
    let ty = &compiled.types()[usize::from(a.kind)];
    let mut effects = Vec::new();
    let mut fired = Vec::new();
    let mut errors = Vec::new();
    for &ri in &ty.rules {
        let rule = &compiled.rules()[ri as usize];
        if rule.event != REvent::Death {
            continue;
        }
        let mut cx = EvalCx { frame, me, other: None, rng: keyed_rng(frame.seed, frame.tick, id, rule_stream(ri)) };
        if rule.condition.as_ref().is_some_and(|c| !cx.boolean(c))
            || rule.probability.is_some_and(|p| !cx.rng.chance(p))
        {
            continue;
        }
        let Some(acts) = rule.actions.iter().map(|act| eval_action(&mut cx, act, ty)).collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let mut rng = cx.rng;
        if let Some(plan) = realize(frame, me, acts, &mut rng, 0.0) {
            effects.extend(plan.effects);
            effects.extend(product_effects(&rule.products));
            fired.push(ri);
        }
    }
    let host = compiled.scripts();
    for &si in &ty.scripts {
        let script = &host.scripts()[si as usize];
        if !script.enabled || disabled.get(si as usize).copied().unwrap_or(false) || script.event != REvent::Death {
            continue;
        }
        match host.run(si as usize, frame, me, None, keyed_rng(frame.seed, frame.tick, id, script_stream(si))) {
            Ok(acts) => {
                let mut rng = keyed_rng(frame.seed, frame.tick, id, script_realize_stream(si));
                if let Some(plan) = realize(frame, me, acts, &mut rng, 0.0) {
                    effects.extend(plan.effects);
                }
            }
            Err(e) => errors.push((si, e)),
        }
    }
    (effects, fired, errors)
}
