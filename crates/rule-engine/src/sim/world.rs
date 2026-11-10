//! World set-up, random events and end conditions.

use std::sync::Arc;

use organism_engine::AgentStore;
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent, SimRng};

use crate::compile::CompiledSystem;
use crate::def::{Distribution, EndCondition, EventEffect};
use crate::sim::State;
use crate::sim::entity::Entity;
use crate::sim::fields::{disc_cells, init_field};
use crate::sim::intent::HEADING_STREAM;
use crate::sim::resolve::{death_reactions, log_extinctions, queue_births};
use crate::sim::space::Space;

/// Stream of the set-up random generator (placement, initial fields).
const SETUP_STREAM: u64 = 1;
/// Stream of the world random generator (random events).
const WORLD_STREAM: u64 = 2;

/// Initial position of the `i`-th of `count` entities.
fn initial_position(d: &Distribution, i: u32, count: u32, space: &Space, rng: &mut SimRng) -> Vec2 {
    let p = match *d {
        Distribution::Uniform => Vec2::new(rng.range_f32(0.0, space.width), rng.range_f32(0.0, space.height)),
        Distribution::Cluster { x, y, radius } => Vec2::new(x, y) + rng.in_disc(radius),
        Distribution::Rect { x, y, width, height } => {
            Vec2::new(x + rng.range_f32(0.0, width.max(0.0)), y + rng.range_f32(0.0, height.max(0.0)))
        }
        Distribution::Grid => {
            let n = count.max(1) as f32;
            let cols = (n * space.width / space.height).sqrt().ceil().max(1.0);
            let rows = (n / cols).ceil().max(1.0);
            let (c, r) = ((i as f32) % cols, (i as f32 / cols).floor());
            Vec2::new((c + 0.5) * space.width / cols, (r + 0.5) * space.height / rows)
        }
    };
    space.place(p)
}

/// The state at tick 0 of `compiled` with `seed`.
pub(crate) fn initial_state(compiled: &CompiledSystem, seed: u64, pause_on_end: bool) -> State {
    let sys = compiled.source();
    let (w, h) = (sys.world.width as usize, sys.world.height as usize);
    let space = Space::new(sys.world.topology, sys.world.width, sys.world.height);
    let mut setup = SimRng::derive(seed, SETUP_STREAM);
    let fields: Vec<_> = sys.world.resources.iter().map(|r| init_field(r, w, h, &mut setup)).collect();
    let mut store = AgentStore::new();
    for (k, ty) in compiled.types().iter().enumerate() {
        let init = &ty.def.initial;
        for i in 0..init.count {
            let pos = initial_position(&init.distribution, i, init.count, &space, &mut setup);
            let heading = setup.f32() * std::f32::consts::TAU;
            store.queue_spawn(Entity::new(k as u16, ty, pos, heading, 0, ty.def.energy.initial));
        }
    }
    store.commit();
    let mut state = State {
        system: sys.clone(),
        seed,
        tick: 0,
        rng: SimRng::derive(seed, WORLD_STREAM),
        store: Arc::new(store),
        fields: Arc::new(fields),
        fired_total: vec![0; compiled.rules().len()],
        script_disabled: vec![false; compiled.scripts().scripts().len()],
        script_errors: Vec::new(),
        events: simulation_core::EventLog::new(1000),
        finished: None,
        pause_on_end,
        stats: Default::default(),
    };
    state.stats.reset(compiled);
    state
}

/// Draws and applies the random events of one tick (in definition order).
pub(crate) fn random_events(state: &mut State, compiled: &Arc<CompiledSystem>) {
    let sys = compiled.source();
    let space = Space::new(sys.world.topology, sys.world.width, sys.world.height);
    let (w, h) = (sys.world.width as usize, sys.world.height as usize);
    for ev in &sys.world.random_events {
        if !state.rng.chance(ev.probability) {
            continue;
        }
        let at = Vec2::new(state.rng.range_f32(0.0, space.width), state.rng.range_f32(0.0, space.height));
        state.stats.random_events += 1;
        let message = match &ev.effect {
            EventEffect::ResourcePulse { resource, amount, radius } => {
                if let Some(r) = sys.resource_index(resource) {
                    let field = &mut Arc::make_mut(&mut state.fields)[r];
                    disc_cells(at.x, at.y, *radius, w, h, sys.world.topology, |i| {
                        let v = &mut field.as_mut_slice()[i];
                        *v = (*v + amount).max(0.0);
                    });
                }
                format!("{}: {resource} pulse at ({:.0}, {:.0})", ev.name, at.x, at.y)
            }
            EventEffect::ScaleResource { resource, factor } => {
                if let Some(r) = sys.resource_index(resource) {
                    for v in Arc::make_mut(&mut state.fields)[r].as_mut_slice() {
                        *v *= factor;
                    }
                }
                format!("{}: {resource} ×{factor}", ev.name)
            }
            EventEffect::Catastrophe { radius, kill_probability, entity } => {
                let only = entity.as_deref().and_then(|t| compiled.type_index(t));
                // Victims in id (= slot) order; one roll per entity in the disc.
                let mut victims = Vec::new();
                for (slot, e) in state.store.as_slice().iter().enumerate() {
                    if only.is_none_or(|t| e.kind == t)
                        && space.delta(at, e.kin.position).length() <= *radius
                        && state.rng.chance(*kill_probability)
                    {
                        victims.push(slot);
                    }
                }
                // Victims die like any other entity: on_death rules and scripts
                // run, carried resources are dropped.
                let room = (sys.world.max_entities as usize).saturating_sub(state.store.len()) as u64;
                let births = death_reactions(state, compiled, &space, &victims, room);
                let ids: Vec<_> = victims.iter().map(|&s| state.store.ids()[s]).collect();
                for &s in &victims {
                    let e = state.store.as_slice()[s].clone();
                    drop_carried(&mut state.fields, compiled, &space, &e);
                }
                let store = Arc::make_mut(&mut state.store);
                for id in ids {
                    store.queue_despawn(id);
                }
                queue_births(store, compiled, state.seed, state.tick, HEADING_STREAM + (1 << 20), births);
                let report = store.commit();
                let mut hit = Vec::new();
                for (_, e) in &report.despawned {
                    state.stats.deaths[usize::from(e.kind)] += 1;
                    hit.push(usize::from(e.kind));
                }
                for id in &report.spawned {
                    if let Some(e) = state.store.get(*id) {
                        state.stats.births[usize::from(e.kind)] += 1;
                    }
                }
                hit.sort_unstable();
                hit.dedup();
                log_extinctions(state, compiled, hit);
                format!("{}: catastrophe at ({:.0}, {:.0}) killed {}", ev.name, at.x, at.y, victims.len())
            }
            EventEffect::Immigration { entity, count } => {
                let Some(t) = compiled.type_index(entity) else { continue };
                let ty = &compiled.types()[usize::from(t)];
                let room = (sys.world.max_entities as usize).saturating_sub(state.store.len());
                let n = (*count as usize).min(room);
                let store = Arc::make_mut(&mut state.store);
                for _ in 0..n {
                    let pos = space.place(at + state.rng.in_disc(3.0));
                    let heading = state.rng.f32() * std::f32::consts::TAU;
                    store.queue_spawn(Entity::new(t, ty, pos, heading, state.tick + 1, ty.def.energy.initial));
                }
                store.commit();
                state.stats.births[usize::from(t)] += n as u32;
                format!("{}: {n} {entity} arrived at ({:.0}, {:.0})", ev.name, at.x, at.y)
            }
        };
        state.events.push(SimEvent::new(state.tick, "random_event", message).severity(EventSeverity::Major));
    }
}

/// Releases an entity's carried resources at its position.
pub(crate) fn drop_carried(
    fields: &mut Arc<Vec<simulation_core::Grid2<f32>>>,
    compiled: &CompiledSystem,
    space: &Space,
    e: &Entity,
) {
    let ty = &compiled.types()[usize::from(e.kind)];
    if ty.carried.is_empty() {
        return;
    }
    let fields = Arc::make_mut(fields);
    let (x, y) = space.cell(e.kin.position);
    for (i, &r) in ty.carried.iter().enumerate() {
        if let Some(f) = fields.get_mut(usize::from(r)) {
            *f.get_mut(x, y) += e.carried.get(i).copied().unwrap_or(0.0) as f32;
        }
    }
}

/// Checks the end conditions (first met condition wins) and logs it.
pub(crate) fn check_end(state: &mut State, compiled: &CompiledSystem) {
    if state.finished.is_some() {
        return;
    }
    let mut counts = vec![0u32; compiled.types().len()];
    for e in state.store.as_slice() {
        counts[usize::from(e.kind)] += 1;
    }
    let total: u32 = counts.iter().sum();
    let pop = |entity: &Option<String>| match entity {
        Some(t) => compiled.type_index(t).map_or(0, |i| counts[usize::from(i)]),
        None => total,
    };
    let label = |entity: &Option<String>| entity.clone().unwrap_or_else(|| "all entities".into());
    for c in &compiled.source().end_conditions {
        let reason = match c {
            EndCondition::TickLimit(n) if state.tick >= *n => Some(format!("tick limit {n} reached")),
            EndCondition::Extinction(t) if compiled.type_index(t).is_some_and(|i| counts[usize::from(i)] == 0) => {
                Some(format!("`{t}` went extinct"))
            }
            EndCondition::AllExtinct if total == 0 => Some("every entity died".to_string()),
            EndCondition::PopulationAbove { entity, count } if pop(entity) > *count => {
                Some(format!("population of {} exceeded {count}", label(entity)))
            }
            EndCondition::PopulationBelow { entity, count } if pop(entity) < *count => {
                Some(format!("population of {} fell below {count}", label(entity)))
            }
            _ => None,
        };
        if let Some(reason) = reason {
            state.events.push(
                SimEvent::new(state.tick, "end_condition", format!("end condition met: {reason}"))
                    .severity(EventSeverity::Major),
            );
            state.finished = Some(reason);
            return;
        }
    }
}
