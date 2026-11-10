//! Resolution phase: applies the decisions of one tick in a fixed order.
//!
//! 1. **Interactions** (`eat`, `attack`, sexual `reproduce`, `trade`) are
//!    claims on another entity, resolved with organism-engine's
//!    [`resolve_matching`] (priority, then distance, then id); winners are
//!    then accepted in rank order so that every entity takes part in at
//!    most one interaction per tick. A firing whose interaction is refused
//!    is cancelled as a whole: no costs, no cooldown, no effects.
//! 2. **Capacity**: firings that would create more entities than
//!    `max_entities` allows are cancelled as a whole.
//! 3. Surviving firings, in (entity id, rule order): costs are charged,
//!    cooldowns start, property/state effects apply.
//! 4. Interactions are applied in rank order; then asexual reproduction and
//!    `die` of entities that were not eaten or killed by an interaction.
//! 5. `consume` claims on a cell are shared proportionally
//!    ([`resolve_shared`]) so the field never goes negative; `emit`.
//! 6. Metabolism, movement (`Kinematics::integrate`, then the topology),
//!    transformations, births and spawns, deaths (explicit, eaten, killed,
//!    starvation, old age) with their `on_death` reactions, ageing, commit.

use std::sync::Arc;

use organism_engine::kinematics::{Bounds, MotionLimits};
use organism_engine::{AgentId, Claim, ShareRule, resolve_matching, resolve_shared};
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent};

use crate::compile::CompiledSystem;
use crate::compile::ir::REvent;
use crate::def::ReproductionMode;
use crate::sim::State;
use crate::sim::decide::{AgentDecision, decide_death};
use crate::sim::entity::Entity;
use crate::sim::frame::Frame;
use crate::sim::intent::{ActIntent, Effect, Firing, HEADING_STREAM, PLACEMENT_STREAM, Source, keyed_rng};
use crate::sim::space::Space;

/// Why an entity died.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cause {
    Rule,
    Eaten,
    Killed,
}

/// A queued birth: type, position, energy, generation, parent.
pub(crate) struct Birth {
    ty: u16,
    pos: Vec2,
    energy: f64,
    generation: u32,
    parent: Option<AgentId>,
}

/// Scratch data of one resolution.
struct Work {
    dead: Vec<Option<Cause>>,
    motion: Vec<Option<crate::sim::intent::Motion>>,
    state_changed: Vec<bool>,
    transform: Vec<Option<u16>>,
    consume: Vec<(u32, u16, f64)>,
    emit: Vec<(u32, u16, f64, bool)>,
    births: Vec<Birth>,
}

/// Position of the `k`-th entity created by `parent` this tick: uniform in
/// a disc of `radius` around `pos` (keyed stream, order independent).
fn place_near(seed: u64, tick: u64, space: &Space, parent: AgentId, pos: Vec2, radius: f32, k: u64) -> Vec2 {
    let mut rng = keyed_rng(seed, tick, parent, PLACEMENT_STREAM + k);
    space.place(pos + rng.in_disc(radius.max(0.1)))
}

/// Applies `decisions` (taken on `frame`) to `state`.
pub(crate) fn resolve(
    state: &mut State,
    compiled: &Arc<CompiledSystem>,
    frame: Arc<Frame>,
    decisions: Vec<AgentDecision>,
) {
    let space = frame.space;
    let n = frame.len();
    let ids: Vec<AgentId> = frame.store.ids().to_vec();
    // Release the frame so the store and fields are uniquely owned again.
    drop(frame);

    let mut firings: Vec<Firing> = Vec::new();
    for d in decisions {
        firings.extend(d.firings);
        for (si, err) in d.errors {
            state.record_script_error(si as usize, err);
        }
    }
    let mut alive = vec![true; firings.len()];

    // 1. Contested interactions.
    let mut claim_owner = Vec::new();
    let claims: Vec<Claim> = firings
        .iter()
        .enumerate()
        .filter_map(|(fi, f)| {
            let (target, distance) = f.act.as_ref()?.counterpart()?;
            claim_owner.push((ids[f.slot as usize], fi));
            Some(
                Claim::new(ids[f.slot as usize], ids[target as usize].get())
                    .priority(f.priority as f32)
                    .distance(distance),
            )
        })
        .collect();
    claim_owner.sort_by_key(|(id, _)| *id);
    let owner = |id: AgentId| claim_owner.binary_search_by_key(&id, |(i, _)| *i).ok().map(|k| claim_owner[k].1);
    let slot_of = |id: u64| ids.binary_search(&AgentId(id)).ok();
    let mut winners = resolve_matching(&claims);
    winners.sort_by(|a, b| {
        b.priority.total_cmp(&a.priority).then(a.distance.total_cmp(&b.distance)).then(a.claimant.cmp(&b.claimant))
    });
    let mut busy = vec![false; n];
    let mut accepted = 0usize;
    let mut rank = vec![usize::MAX; firings.len()];
    for w in &winners {
        let (Some(fi), Some(t)) = (owner(w.claimant), slot_of(w.target)) else { continue };
        let c = firings[fi].slot as usize;
        if busy[c] || busy[t] {
            continue;
        }
        busy[c] = true;
        busy[t] = true;
        rank[fi] = accepted;
        accepted += 1;
    }
    for (fi, f) in firings.iter().enumerate() {
        if f.act.as_ref().and_then(ActIntent::counterpart).is_some() && rank[fi] == usize::MAX {
            alive[fi] = false;
            state.stats.interactions_refused += 1;
        }
    }

    // 2. Capacity for new entities.
    let cap = u64::from(compiled.source().world.max_entities);
    let mut room = cap.saturating_sub(n as u64);
    for (fi, f) in firings.iter().enumerate() {
        if !alive[fi] {
            continue;
        }
        let litter =
            compiled.types()[usize::from(state.store.as_slice()[f.slot as usize].kind)].def.reproduction.litter;
        let b = f.births(litter);
        if b > room {
            alive[fi] = false;
            state.stats.births_refused += b;
        } else {
            room -= b;
        }
    }

    let tick = state.tick;
    let mut work = Work {
        dead: vec![None; n],
        motion: vec![None; n],
        state_changed: vec![false; n],
        transform: vec![None; n],
        consume: Vec::new(),
        emit: Vec::new(),
        births: Vec::new(),
    };
    let mut acts: Vec<(usize, ActIntent)> = Vec::new();
    {
        let store = Arc::make_mut(&mut state.store);
        let agents = store.as_mut_slice();
        // 3. Costs, cooldowns and own effects.
        for (fi, f) in firings.iter().enumerate() {
            if !alive[fi] {
                continue;
            }
            let slot = f.slot as usize;
            let e = &mut agents[slot];
            let ty = &compiled.types()[usize::from(e.kind)];
            for (p, c) in &f.costs {
                let v = e.prop(*p, ids[slot], tick);
                e.set_prop(*p, v - c, ty);
            }
            if let Some((local, next)) = f.cooldown
                && let Some(cd) = e.cooldowns.get_mut(local)
            {
                *cd = next;
            }
            if let Source::Rule(ri) = f.source {
                state.stats.rule_fired[ri as usize] += 1;
            }
            if let Some(m) = f.motion {
                work.motion[slot] = Some(m);
            }
            if let Some(t) = f.transform {
                work.transform[slot] = Some(t);
            }
            for eff in &f.effects {
                match *eff {
                    Effect::SetProp { prop, value } => e.set_prop(prop, value, ty),
                    Effect::AddProp { prop, value } => {
                        let v = e.prop(prop, ids[slot], tick);
                        e.set_prop(prop, v + value, ty);
                    }
                    Effect::SetState(s) => {
                        if e.state != s {
                            e.state = s;
                            e.state_since = tick + 1;
                            work.state_changed[slot] = true;
                        }
                    }
                    Effect::Consume { resource, amount } => work.consume.push((slot as u32, resource, amount)),
                    Effect::Emit { resource, amount } => work.emit.push((slot as u32, resource, amount, true)),
                    Effect::Release { resource, amount } => work.emit.push((slot as u32, resource, amount, false)),
                    Effect::Spawn { ty: t, count } => {
                        for k in 0..count {
                            let pos = place_near(
                                state.seed,
                                tick,
                                &space,
                                ids[slot],
                                e.kin.position,
                                e.size,
                                u64::from(k) + 1000 * fi as u64,
                            );
                            let energy = compiled.types()[usize::from(t)].def.energy.initial;
                            work.births.push(Birth { ty: t, pos, energy, generation: 0, parent: Some(ids[slot]) });
                        }
                    }
                }
            }
            if let Some(act) = &f.act {
                acts.push((fi, act.clone()));
            }
        }
    }
    // 4. Exclusive actions: interactions in rank order, then the rest in entity order.
    // An entity eaten or killed by an earlier interaction does nothing more:
    // in particular it does not reproduce (its energy already went to the eater).
    acts.sort_by_key(|(fi, _)| (rank[*fi], *fi));
    for (fi, act) in acts {
        let slot = firings[fi].slot as usize;
        if work.dead[slot].is_some() {
            continue;
        }
        apply_act(state, compiled, &space, &ids, &mut work, slot, fi, act);
    }

    // 5. Resources.
    let fields = Arc::make_mut(&mut state.fields);
    let store = Arc::make_mut(&mut state.store);
    let agents = store.as_mut_slice();
    if !work.consume.is_empty() {
        let cells = (space.width as usize) * (space.height as usize);
        let claims: Vec<Claim> = work
            .consume
            .iter()
            .map(|&(slot, r, amount)| {
                let e = &agents[slot as usize];
                let ty = &compiled.types()[usize::from(e.kind)];
                let request = match ty.carried.iter().position(|c| *c == r) {
                    Some(i) => amount.min(ty.def.carries[i].capacity - e.carried[i]).max(0.0),
                    None => amount,
                };
                let (x, y) = space.cell(e.kin.position);
                let target = usize::from(r) * cells + y * space.width as usize + x;
                Claim::new(ids[slot as usize], target as u64).amount(request as f32)
            })
            .collect();
        let grants = resolve_shared(
            &claims,
            |t| {
                let (r, cell) = (t as usize / cells, t as usize % cells);
                fields.get(r).map_or(0.0, |g| g.as_slice()[cell])
            },
            ShareRule::Proportional,
        );
        for g in grants {
            let (r, cell) = (g.target as usize / cells, g.target as usize % cells);
            let Some(slot) = ids.binary_search(&g.claimant).ok() else { continue };
            let Some(field) = fields.get_mut(r) else { continue };
            let e = &mut agents[slot];
            let ty = &compiled.types()[usize::from(e.kind)];
            let taken = match ty.carried.iter().position(|c| usize::from(*c) == r) {
                // Several requests of one entity may together exceed its
                // capacity; only what fits leaves the field.
                Some(i) => {
                    let fits = (ty.def.carries[i].capacity - e.carried[i]).max(0.0);
                    let taken = f64::from(g.granted).min(fits);
                    e.carried[i] += taken;
                    taken
                }
                None => {
                    let value = compiled.source().world.resources[r].energy_value;
                    e.energy = (e.energy + f64::from(g.granted) * value).min(ty.def.energy.max);
                    f64::from(g.granted)
                }
            };
            let v = &mut field.as_mut_slice()[cell];
            *v = (*v - taken as f32).max(0.0);
            state.stats.consumed[r] += taken;
        }
    }
    for &(slot, r, amount, from_stock) in &work.emit {
        let e = &mut agents[slot as usize];
        let ty = &compiled.types()[usize::from(e.kind)];
        let released = match ty.carried.iter().position(|c| *c == r).filter(|_| from_stock) {
            Some(i) => {
                let a = amount.min(e.carried[i]);
                e.carried[i] -= a;
                a
            }
            None => amount,
        };
        let (x, y) = space.cell(e.kin.position);
        if let Some(field) = fields.get_mut(usize::from(r)) {
            *field.get_mut(x, y) += released as f32;
        }
    }

    // 6. Metabolism and movement.
    for (slot, e) in agents.iter_mut().enumerate() {
        let ty = &compiled.types()[usize::from(e.kind)];
        e.energy -= ty.def.energy.metabolism;
        if let Some(m) = work.motion[slot] {
            e.kin.integrate(m.accel, 1.0, MotionLimits::new(m.speed, f32::INFINITY), Bounds::Unbounded);
            space.constrain(&mut e.kin);
        }
    }
    // Transformations.
    for (slot, e) in agents.iter_mut().enumerate() {
        let Some(t) = work.transform[slot] else { continue };
        if work.dead[slot].is_some() {
            continue;
        }
        let old = &compiled.types()[usize::from(e.kind)];
        let new = &compiled.types()[usize::from(t)];
        let (x, y) = space.cell(e.kin.position);
        for (r, amount) in e.transform(t, old, new, tick + 1) {
            if let Some(field) = fields.get_mut(usize::from(r)) {
                *field.get_mut(x, y) += amount as f32;
            }
        }
        work.state_changed[slot] = false;
    }
    // Natural deaths.
    for (slot, e) in agents.iter().enumerate() {
        if work.dead[slot].is_some() {
            continue;
        }
        let ty = &compiled.types()[usize::from(e.kind)];
        let starved = ty.def.energy.starvation && e.energy <= 0.0;
        let old = ty.def.lifecycle.max_age.is_some_and(|m| e.age + 1 >= m);
        if starved || old {
            work.dead[slot] = Some(Cause::Rule);
        }
    }
    finish(state, compiled, &space, &ids, work);
}

#[allow(clippy::too_many_arguments)]
fn apply_act(
    state: &mut State,
    compiled: &CompiledSystem,
    space: &Space,
    ids: &[AgentId],
    work: &mut Work,
    slot: usize,
    fi: usize,
    act: ActIntent,
) {
    let tick = state.tick;
    let store = Arc::make_mut(&mut state.store);
    let agents = store.as_mut_slice();
    let kind = agents[slot].kind;
    let ty = &compiled.types()[usize::from(kind)];
    match act {
        ActIntent::Eat { target, efficiency, .. } => {
            let t = target as usize;
            let gain = agents[t].energy.max(0.0) * efficiency;
            work.dead[t] = Some(Cause::Eaten);
            let e = &mut agents[slot];
            e.energy = (e.energy + gain).min(ty.def.energy.max);
            state.stats.eaten += 1;
        }
        ActIntent::Attack { target, damage, .. } => {
            let t = target as usize;
            agents[t].energy -= damage;
            if agents[t].energy <= 0.0 {
                work.dead[t] = Some(Cause::Killed);
            }
        }
        ActIntent::Trade { target, give, amount, receive, receive_amount, .. } => {
            let t = target as usize;
            let other_ty = &compiled.types()[usize::from(agents[t].kind)];
            let (Some(gi), Some(ri)) =
                (ty.carried.iter().position(|r| *r == give), ty.carried.iter().position(|r| *r == receive))
            else {
                return;
            };
            let (Some(ogi), Some(ori)) =
                (other_ty.carried.iter().position(|r| *r == give), other_ty.carried.iter().position(|r| *r == receive))
            else {
                return;
            };
            let ok = agents[slot].carried[gi] >= amount
                && agents[t].carried[ori] >= receive_amount
                && agents[slot].carried[ri] + receive_amount <= ty.def.carries[ri].capacity
                && agents[t].carried[ogi] + amount <= other_ty.def.carries[ogi].capacity;
            if ok {
                agents[slot].carried[gi] -= amount;
                agents[slot].carried[ri] += receive_amount;
                agents[t].carried[ogi] += amount;
                agents[t].carried[ori] -= receive_amount;
                state.stats.trades += 1;
            }
        }
        ActIntent::Reproduce | ActIntent::Mate { .. } => {
            let rep = &ty.def.reproduction;
            let mut paid = rep.cost;
            agents[slot].energy -= rep.cost;
            agents[slot].next_reproduction = tick + rep.cooldown.max(1);
            if let ActIntent::Mate { partner, .. } = act {
                let p = partner as usize;
                let pty = &compiled.types()[usize::from(agents[p].kind)];
                let pcost =
                    if pty.def.reproduction.mode == ReproductionMode::None { 0.0 } else { pty.def.reproduction.cost };
                agents[p].energy -= pcost;
                agents[p].next_reproduction = tick + pty.def.reproduction.cooldown.max(1);
                paid += pcost;
            }
            let litter = rep.litter.max(1);
            let parent = &agents[slot];
            for k in 0..litter {
                let pos = place_near(
                    state.seed,
                    tick,
                    space,
                    ids[slot],
                    parent.kin.position,
                    parent.size,
                    u64::from(k) + 1000 * fi as u64,
                );
                work.births.push(Birth {
                    ty: kind,
                    pos,
                    energy: paid / f64::from(litter),
                    generation: parent.generation + 1,
                    parent: Some(ids[slot]),
                });
            }
        }
        ActIntent::Die => work.dead[slot] = Some(Cause::Rule),
    }
}

/// Deaths with `on_death` reactions, births, ageing and commit.
/// Runs the `on_death` rules and scripts of the entities in `dying` (store
/// slots, evaluated on the current state, before they are removed): counts
/// firings, records script errors, releases emitted resources and returns
/// the entities to spawn. `room` is the number of entities that may still
/// be created this tick (further spawns are refused and counted).
pub(crate) fn death_reactions(
    state: &mut State,
    compiled: &Arc<CompiledSystem>,
    space: &Space,
    dying: &[usize],
    mut room: u64,
) -> Vec<Birth> {
    let mut births = Vec::new();
    let needs_reaction = dying.iter().any(|&s| {
        let ty = &compiled.types()[usize::from(state.store.as_slice()[s].kind)];
        ty.rules.iter().any(|&r| compiled.rules()[r as usize].event == REvent::Death)
            || ty.scripts.iter().any(|&s| compiled.scripts().scripts()[s as usize].event == REvent::Death)
    });
    if !needs_reaction {
        return births;
    }
    let tick = state.tick;
    let frame = Arc::new(state.frame(compiled));
    let disabled = state.script_disabled.clone();
    let mut releases = Vec::new();
    for &s in dying {
        let (effects, fired, errors) = decide_death(&frame, s, &disabled);
        for ri in fired {
            state.stats.rule_fired[ri as usize] += 1;
        }
        for (si, e) in errors {
            state.record_script_error(si as usize, e);
        }
        let (e, id) = (frame.agent(s), frame.id(s));
        for (k, eff) in effects.into_iter().enumerate() {
            match eff {
                Effect::Spawn { ty, count } => {
                    for j in 0..count {
                        if room == 0 {
                            state.stats.births_refused += 1;
                            continue;
                        }
                        room -= 1;
                        let key = (k as u64) << 20 | u64::from(j);
                        let pos = place_near(state.seed, tick, space, id, e.kin.position, e.size, key);
                        let energy = compiled.types()[usize::from(ty)].def.energy.initial;
                        births.push(Birth { ty, pos, energy, generation: 0, parent: Some(id) });
                    }
                }
                Effect::Emit { resource, amount } | Effect::Release { resource, amount } => {
                    releases.push((e.kin.position, resource, amount));
                }
                _ => {}
            }
        }
    }
    drop(frame);
    let fields = Arc::make_mut(&mut state.fields);
    for (pos, r, amount) in releases {
        let (x, y) = space.cell(pos);
        if let Some(f) = fields.get_mut(usize::from(r)) {
            *f.get_mut(x, y) += amount as f32;
        }
    }
    births
}

/// Queues the entities of `births` (born at the end of `tick`, so their
/// `on_birth` rules run on the next tick); `stream` keys their headings.
pub(crate) fn queue_births(
    store: &mut organism_engine::AgentStore<Entity>,
    compiled: &CompiledSystem,
    seed: u64,
    tick: u64,
    stream: u64,
    births: Vec<Birth>,
) {
    for (k, b) in births.into_iter().enumerate() {
        let ty = &compiled.types()[usize::from(b.ty)];
        let key = stream + k as u64;
        let heading = keyed_rng(seed, tick, b.parent.unwrap_or(AgentId(0)), key).f32() * std::f32::consts::TAU;
        let mut e = Entity::new(b.ty, ty, b.pos, heading, tick + 1, b.energy);
        e.generation = b.generation;
        e.parent = b.parent;
        store.queue_spawn(e);
    }
}

/// Logs an extinction event for every type in `types` that has no live entity.
pub(crate) fn log_extinctions(state: &mut State, compiled: &CompiledSystem, types: impl IntoIterator<Item = usize>) {
    let mut counts = vec![0u32; compiled.types().len()];
    for e in state.store.as_slice() {
        counts[usize::from(e.kind)] += 1;
    }
    for t in types {
        if counts.get(t) == Some(&0) {
            let name = &compiled.types()[t].def.name;
            state.events.push(
                SimEvent::new(state.tick, "extinction", format!("`{name}` went extinct"))
                    .severity(EventSeverity::Major),
            );
        }
    }
}

/// Deaths with `on_death` reactions, births, ageing and commit.
fn finish(state: &mut State, compiled: &Arc<CompiledSystem>, space: &Space, ids: &[AgentId], mut work: Work) {
    let tick = state.tick;
    let dying: Vec<usize> = (0..work.dead.len()).filter(|&s| work.dead[s].is_some()).collect();
    let room =
        u64::from(compiled.source().world.max_entities).saturating_sub(ids.len() as u64 + work.births.len() as u64);
    let mut reactions = death_reactions(state, compiled, space, &dying, room);
    work.births.append(&mut reactions);
    // Dropped carried resources and despawns.
    let fields = Arc::make_mut(&mut state.fields);
    let store = Arc::make_mut(&mut state.store);
    for &s in &dying {
        let e = &store.as_slice()[s];
        let ty = &compiled.types()[usize::from(e.kind)];
        let (x, y) = space.cell(e.kin.position);
        for (i, &r) in ty.carried.iter().enumerate() {
            if let Some(f) = fields.get_mut(usize::from(r)) {
                *f.get_mut(x, y) += e.carried[i] as f32;
            }
        }
        store.queue_despawn(ids[s]);
    }
    for (slot, e) in store.as_mut_slice().iter_mut().enumerate() {
        e.age += 1;
        e.birth_pending = false;
        e.state_enter_pending = work.state_changed[slot];
    }
    queue_births(store, compiled, state.seed, tick, HEADING_STREAM, work.births);
    let report = store.commit();
    for id in &report.spawned {
        if let Some(e) = store.get(*id) {
            state.stats.births[usize::from(e.kind)] += 1;
        }
    }
    for (id, e) in &report.despawned {
        state.stats.deaths[usize::from(e.kind)] += 1;
        let cause = ids.binary_search(id).ok().and_then(|s| work.dead[s]);
        if cause == Some(Cause::Killed) {
            state.stats.killed += 1;
        }
    }
    let died: Vec<usize> = (0..compiled.types().len()).filter(|&t| state.stats.deaths[t] > 0).collect();
    log_extinctions(state, compiled, died);
}
