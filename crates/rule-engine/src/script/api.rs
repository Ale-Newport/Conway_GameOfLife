//! The simulation API visible to Rhai scripts.
//!
//! Scripts define `fn run(me)`. `me` and every entity returned by sensors
//! are `Entity` handles with read-only getters (`exists`, `id`, `kind`,
//! `state`, `energy`, `age`, `state_age`, `size`, `speed`, `x`, `y`,
//! `generation`, `distance`) and a `prop("name")` method.
//!
//! Free functions read the frozen frame of the current tick through the
//! call tag: `nearby(kind[, radius])`, `nearest(kind[, radius])`,
//! `field(name)`, `population(kind)`, `other()`, `tick()`, `random()`.
//!
//! Action functions only **record intents** (they never touch the world):
//! `move_towards(e[, speed])`, `move_away(e[, speed])`, `wander([turn])`,
//! `stay()`, `eat(e[, efficiency])`, `attack(e[, damage])`,
//! `reproduce([p])`, `reproduce_with(e[, p])`, `transform_into(type)`,
//! `die()`, `emit(res, amount)`, `consume(res, amount)`,
//! `set_prop(name, v)`, `add_prop(name, v)`, `set_state(s)`,
//! `spawn_entity(type[, n])` (`spawn` is a reserved word in Rhai). Within one call a later movement (or exclusive)
//! action replaces an earlier one. Acting on an entity that does not exist
//! records nothing. Invalid names or values raise a runtime error that
//! disables the script.

use std::sync::{Arc, Mutex, PoisonError};

use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString, NativeCallContext};
use simulation_core::SimRng;

use crate::compile::CompiledType;
use crate::compile::ir::{FieldRef, KindSel, MAX_SENSE_RADIUS, MAX_SPAWN_PER_ACTION, PropRef, Slot};
use crate::def::ReproductionMode;
use crate::sim::frame::{Frame, Target};
use crate::sim::intent::Act;

type R<T> = Result<T, Box<EvalAltResult>>;

/// Actions recorded during one call, and the call's random stream.
pub struct Recorder {
    /// Recorded actions in order.
    pub acts: Vec<Act>,
    /// Keyed random stream of this (tick, entity, script).
    pub rng: SimRng,
}

impl Recorder {
    fn push(&mut self, act: Act) {
        let slot = act.slot();
        if slot != Slot::Effect {
            self.acts.retain(|a| a.slot() != slot);
        }
        self.acts.push(act);
    }
}

/// Per-call context passed as the Rhai call tag.
#[derive(Clone)]
pub struct ScriptCtx {
    /// Frozen world of the tick.
    pub frame: Arc<Frame>,
    /// Acting entity's slot.
    pub me: u32,
    /// Touched entity (collision scripts).
    pub other: Option<Target>,
    /// Whether this is an `on_death` call (only emit/spawn allowed).
    pub death: bool,
    /// Recorded intents.
    pub rec: Arc<Mutex<Recorder>>,
}

impl ScriptCtx {
    fn me(&self) -> usize {
        self.me as usize
    }

    fn my_type(&self) -> &CompiledType {
        &self.frame.compiled.types()[usize::from(self.frame.agent(self.me()).kind)]
    }

    fn record(&self, act: Act, name: &str) -> R<()> {
        if self.death && !matches!(act, Act::Emit { .. } | Act::Spawn { .. }) {
            return Err(format!("{name}() is not allowed in on_death scripts (only emit and spawn_entity)").into());
        }
        if act.slot() == Slot::Move && !self.my_type().def.movement.is_mobile() {
            return Err(format!("{name}(): `{}` is sessile and cannot move", self.my_type().def.name).into());
        }
        self.rec.lock().unwrap_or_else(PoisonError::into_inner).push(act);
        Ok(())
    }

    fn kind(&self, name: &str) -> R<KindSel> {
        match name {
            "any" => Ok(KindSel::Any),
            "same_species" | "same_type" | "same" => Ok(KindSel::Same),
            _ => self
                .frame
                .compiled
                .symbols()
                .type_idx(name)
                .map(KindSel::Type)
                .ok_or_else(|| format!("unknown entity type `{name}`").into()),
        }
    }

    fn type_idx(&self, name: &str) -> R<u16> {
        self.frame.compiled.symbols().type_idx(name).ok_or_else(|| format!("unknown entity type `{name}`").into())
    }

    fn resource(&self, name: &str) -> R<u16> {
        self.frame.compiled.symbols().resource_idx(name).ok_or_else(|| format!("unknown resource `{name}`").into())
    }

    fn speed(&self) -> f32 {
        self.frame.agent(self.me()).speed
    }
}

/// Script-visible entity handle (`Entity` in Rhai).
#[derive(Clone)]
pub struct EntityRef {
    frame: Arc<Frame>,
    target: Option<Target>,
}

impl EntityRef {
    /// Handle for `target` (or a non-existent entity).
    pub fn new(frame: Arc<Frame>, target: Option<Target>) -> Self {
        Self { frame, target }
    }

    fn num(&self, p: PropRef) -> f64 {
        self.target.map_or(0.0, |t| self.frame.prop(t.slot as usize, p))
    }

    fn kind_name(&self) -> String {
        self.target.map_or_else(String::new, |t| {
            let k = usize::from(self.frame.agent(t.slot as usize).kind);
            self.frame.compiled.types()[k].def.name.clone()
        })
    }

    fn state_name(&self) -> String {
        self.target.map_or_else(String::new, |t| {
            let a = self.frame.agent(t.slot as usize);
            self.frame.compiled.types()[usize::from(a.kind)]
                .states
                .get(usize::from(a.state))
                .cloned()
                .unwrap_or_default()
        })
    }
}

fn ctx_of(ctx: &NativeCallContext) -> R<ScriptCtx> {
    ctx.tag()
        .and_then(|t| t.read_lock::<ScriptCtx>())
        .map(|c| c.clone())
        .ok_or_else(|| "simulation functions can only be used inside a running simulation script".into())
}

fn number(v: &Dynamic, what: &str) -> R<f64> {
    let x = if let Ok(f) = v.as_float() {
        f
    } else if let Ok(i) = v.as_int() {
        i as f64
    } else {
        return Err(format!("{what} must be a number, got {}", v.type_name()).into());
    };
    if x.is_finite() { Ok(x) } else { Err(format!("{what} must be finite").into()) }
}

fn radius(v: &Dynamic) -> R<f32> {
    let r = number(v, "radius")?;
    if r > 0.0 && r <= f64::from(MAX_SENSE_RADIUS) {
        Ok(r as f32)
    } else {
        Err(format!("radius {r} must be in (0, {MAX_SENSE_RADIUS}]").into())
    }
}

fn probability(v: &Dynamic) -> R<f64> {
    let p = number(v, "probability")?;
    if (0.0..=1.0).contains(&p) { Ok(p) } else { Err(format!("probability must be in [0, 1] (got {p})").into()) }
}

fn writable(c: &ScriptCtx, name: &str) -> R<PropRef> {
    let me = c.frame.agent(c.me()).kind;
    match c.frame.compiled.symbols().prop_of(me, name) {
        Some(p) if p.is_writable() => Ok(p),
        Some(_) => Err(format!("property `{name}` is read-only").into()),
        None => Err(format!("unknown property `{name}`").into()),
    }
}

fn register_entity(engine: &mut Engine) {
    engine.register_type_with_name::<EntityRef>("Entity");
    engine.register_get("exists", |e: &mut EntityRef| e.target.is_some());
    engine.register_get("id", |e: &mut EntityRef| e.target.map_or(-1, |t| e.frame.id(t.slot as usize).get() as i64));
    engine.register_get("kind", |e: &mut EntityRef| e.kind_name());
    engine.register_get("state", |e: &mut EntityRef| e.state_name());
    engine.register_get("distance", |e: &mut EntityRef| e.target.map_or(f64::INFINITY, |t| f64::from(t.dist)));
    for (name, p) in PropRef::BUILTIN {
        if name == "id" {
            continue;
        }
        engine.register_get(name, move |e: &mut EntityRef| e.num(p));
    }
    engine.register_fn("prop", |e: &mut EntityRef, name: ImmutableString| -> R<f64> {
        let Some(t) = e.target else { return Ok(0.0) };
        let kind = e.frame.agent(t.slot as usize).kind;
        match e.frame.compiled.symbols().prop_of(kind, &name) {
            Some(p) => Ok(e.frame.prop(t.slot as usize, p)),
            None => Err(format!("unknown property `{name}`").into()),
        }
    });
}

fn register_sensors(engine: &mut Engine) {
    engine.register_fn("nearby", |ctx: NativeCallContext, kind: ImmutableString, r: Dynamic| -> R<i64> {
        let c = ctx_of(&ctx)?;
        Ok(c.frame.nearby(c.me(), c.kind(&kind)?, radius(&r)?) as i64)
    });
    engine.register_fn("nearby", |ctx: NativeCallContext, kind: ImmutableString| -> R<i64> {
        let c = ctx_of(&ctx)?;
        Ok(c.frame.nearby(c.me(), c.kind(&kind)?, c.my_type().def.perception) as i64)
    });
    engine.register_fn("nearest", |ctx: NativeCallContext, kind: ImmutableString, r: Dynamic| -> R<EntityRef> {
        let c = ctx_of(&ctx)?;
        let t = c.frame.nearest(c.me(), c.kind(&kind)?, radius(&r)?);
        Ok(EntityRef::new(c.frame.clone(), t))
    });
    engine.register_fn("nearest", |ctx: NativeCallContext, kind: ImmutableString| -> R<EntityRef> {
        let c = ctx_of(&ctx)?;
        let t = c.frame.nearest(c.me(), c.kind(&kind)?, c.my_type().def.perception);
        Ok(EntityRef::new(c.frame.clone(), t))
    });
    engine.register_fn("field", |ctx: NativeCallContext, name: ImmutableString| -> R<f64> {
        let c = ctx_of(&ctx)?;
        let sym = c.frame.compiled.symbols();
        let f = sym
            .resource_idx(&name)
            .map(FieldRef::Resource)
            .or_else(|| sym.env_idx(&name).map(FieldRef::Env))
            .ok_or_else(|| format!("unknown field `{name}`"))?;
        Ok(c.frame.field_at(f, c.frame.agent(c.me()).kin.position))
    });
    engine.register_fn("population", |ctx: NativeCallContext, kind: ImmutableString| -> R<i64> {
        let c = ctx_of(&ctx)?;
        Ok(c.frame.population_of(c.me(), c.kind(&kind)?) as i64)
    });
    engine.register_fn("other", |ctx: NativeCallContext| -> R<EntityRef> {
        let c = ctx_of(&ctx)?;
        Ok(EntityRef::new(c.frame.clone(), c.other))
    });
    engine.register_fn("tick", |ctx: NativeCallContext| -> R<i64> { Ok(ctx_of(&ctx)?.frame.tick as i64) });
    engine.register_fn("random", |ctx: NativeCallContext| -> R<f64> {
        let c = ctx_of(&ctx)?;
        let mut rec = c.rec.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(rec.rng.f64())
    });
}

fn register_moves(engine: &mut Engine) {
    fn towards(ctx: &NativeCallContext, e: &EntityRef, speed: Option<&Dynamic>, away: bool) -> R<()> {
        let c = ctx_of(ctx)?;
        let speed = match speed {
            Some(s) => number(s, "speed")?.clamp(0.0, 50.0) as f32,
            None => c.speed(),
        };
        let name = if away { "move_away" } else { "move_towards" };
        let Some(target) = e.target else { return Ok(()) };
        let act = if away { Act::MoveAway { target, speed } } else { Act::MoveTowards { target, speed } };
        c.record(act, name)
    }
    engine.register_fn("move_towards", |ctx: NativeCallContext, e: EntityRef| towards(&ctx, &e, None, false));
    engine.register_fn("move_towards", |ctx: NativeCallContext, e: EntityRef, s: Dynamic| {
        towards(&ctx, &e, Some(&s), false)
    });
    engine.register_fn("move_away", |ctx: NativeCallContext, e: EntityRef| towards(&ctx, &e, None, true));
    engine
        .register_fn("move_away", |ctx: NativeCallContext, e: EntityRef, s: Dynamic| towards(&ctx, &e, Some(&s), true));
    fn wander(ctx: &NativeCallContext, turn: Option<&Dynamic>) -> R<()> {
        let c = ctx_of(ctx)?;
        let turn = match turn {
            Some(t) => number(t, "turn")?.clamp(0.0, std::f64::consts::PI) as f32,
            None => c.my_type().default_turn(),
        };
        c.record(Act::Wander { turn, speed: c.speed() }, "wander")
    }
    engine.register_fn("wander", |ctx: NativeCallContext| wander(&ctx, None));
    engine.register_fn("wander", |ctx: NativeCallContext, t: Dynamic| wander(&ctx, Some(&t)));
    engine.register_fn("stay", |ctx: NativeCallContext| -> R<()> { ctx_of(&ctx)?.record(Act::Stay, "stay") });
}

fn register_acts(engine: &mut Engine) {
    fn eat(ctx: &NativeCallContext, e: &EntityRef, efficiency: f64) -> R<()> {
        let c = ctx_of(ctx)?;
        match e.target {
            Some(target) => c.record(Act::Eat { target, efficiency: efficiency.clamp(0.0, 1.0) }, "eat"),
            None => Ok(()),
        }
    }
    engine.register_fn("eat", |ctx: NativeCallContext, e: EntityRef| eat(&ctx, &e, 1.0));
    engine.register_fn("eat", |ctx: NativeCallContext, e: EntityRef, eff: Dynamic| {
        eat(&ctx, &e, number(&eff, "efficiency")?)
    });
    fn attack(ctx: &NativeCallContext, e: &EntityRef, damage: f64) -> R<()> {
        let c = ctx_of(ctx)?;
        match e.target {
            Some(target) => c.record(Act::Attack { target, damage: damage.max(0.0) }, "attack"),
            None => Ok(()),
        }
    }
    engine.register_fn("attack", |ctx: NativeCallContext, e: EntityRef| attack(&ctx, &e, 5.0));
    engine.register_fn("attack", |ctx: NativeCallContext, e: EntityRef, d: Dynamic| {
        attack(&ctx, &e, number(&d, "damage")?)
    });
    fn reproduce(ctx: &NativeCallContext, partner: Option<&EntityRef>, p: f64) -> R<()> {
        let c = ctx_of(ctx)?;
        let ty = c.my_type();
        match (ty.def.reproduction.mode, partner.is_some()) {
            (ReproductionMode::None, _) => return Err(format!("`{}` cannot reproduce", ty.def.name).into()),
            (ReproductionMode::Asexual, true) => {
                return Err(format!("`{}` reproduces asexually; use reproduce()", ty.def.name).into());
            }
            (ReproductionMode::Sexual, false) => {
                return Err(format!("`{}` reproduces sexually; use reproduce_with(partner)", ty.def.name).into());
            }
            _ => {}
        }
        let partner = match partner {
            Some(e) => match e.target {
                Some(t) => Some(t),
                None => return Ok(()),
            },
            None => None,
        };
        c.record(Act::Reproduce { probability: p, partner }, "reproduce")
    }
    engine.register_fn("reproduce", |ctx: NativeCallContext| reproduce(&ctx, None, 1.0));
    engine.register_fn("reproduce", |ctx: NativeCallContext, p: Dynamic| reproduce(&ctx, None, probability(&p)?));
    engine.register_fn("reproduce_with", |ctx: NativeCallContext, e: EntityRef| reproduce(&ctx, Some(&e), 1.0));
    engine.register_fn("reproduce_with", |ctx: NativeCallContext, e: EntityRef, p: Dynamic| {
        reproduce(&ctx, Some(&e), probability(&p)?)
    });
    engine.register_fn("transform_into", |ctx: NativeCallContext, ty: ImmutableString| -> R<()> {
        let c = ctx_of(&ctx)?;
        let t = c.type_idx(&ty)?;
        c.record(Act::Transform(t), "transform_into")
    });
    engine.register_fn("die", |ctx: NativeCallContext| -> R<()> { ctx_of(&ctx)?.record(Act::Die, "die") });
}

fn register_effects(engine: &mut Engine) {
    engine.register_fn("emit", |ctx: NativeCallContext, res: ImmutableString, amount: Dynamic| -> R<()> {
        let c = ctx_of(&ctx)?;
        let resource = c.resource(&res)?;
        let amount = number(&amount, "amount")?.max(0.0);
        c.record(Act::Emit { resource, amount }, "emit")
    });
    engine.register_fn("consume", |ctx: NativeCallContext, res: ImmutableString, amount: Dynamic| -> R<()> {
        let c = ctx_of(&ctx)?;
        let resource = c.resource(&res)?;
        let amount = number(&amount, "amount")?.max(0.0);
        c.record(Act::Consume { resource, amount }, "consume")
    });
    engine.register_fn("set_prop", |ctx: NativeCallContext, name: ImmutableString, v: Dynamic| -> R<()> {
        let c = ctx_of(&ctx)?;
        let prop = writable(&c, &name)?;
        c.record(Act::SetProp { prop, value: number(&v, "value")? }, "set_prop")
    });
    engine.register_fn("add_prop", |ctx: NativeCallContext, name: ImmutableString, v: Dynamic| -> R<()> {
        let c = ctx_of(&ctx)?;
        let prop = writable(&c, &name)?;
        c.record(Act::AddProp { prop, value: number(&v, "value")? }, "add_prop")
    });
    engine.register_fn("set_state", |ctx: NativeCallContext, state: ImmutableString| -> R<()> {
        let c = ctx_of(&ctx)?;
        let ty = c.my_type();
        let s = ty.states.iter().position(|s| *s == *state).ok_or_else(|| {
            format!("unknown state `{state}` of `{}` (states: {})", ty.def.name, ty.states.join(", "))
        })?;
        c.record(Act::SetState(s as u16), "set_state")
    });
    fn spawn(ctx: &NativeCallContext, ty: &str, n: f64) -> R<()> {
        let c = ctx_of(ctx)?;
        let t = c.type_idx(ty)?;
        if !(0.0..=f64::from(MAX_SPAWN_PER_ACTION)).contains(&n) {
            return Err(format!("spawn count must be in [0, {MAX_SPAWN_PER_ACTION}] (got {n})").into());
        }
        c.record(Act::Spawn { ty: t, count: n.round() as u32 }, "spawn_entity")
    }
    engine.register_fn("spawn_entity", |ctx: NativeCallContext, ty: ImmutableString| spawn(&ctx, &ty, 1.0));
    engine.register_fn("spawn_entity", |ctx: NativeCallContext, ty: ImmutableString, n: Dynamic| {
        spawn(&ctx, &ty, number(&n, "count")?)
    });
}

/// Registers the whole simulation API on `engine`.
pub fn register_api(engine: &mut Engine) {
    register_entity(engine);
    register_sensors(engine);
    register_moves(engine);
    register_acts(engine);
    register_effects(engine);
}
