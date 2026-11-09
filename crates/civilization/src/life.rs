//! Physiology, ageing, deaths and inheritance; crop growth and building
//! upkeep; the daily social bookkeeping (relationship decay, groups,
//! territory).

use organism_engine::AgentId;
use simulation_core::{EventSeverity, SimEvent};

use crate::act::TickWork;
use crate::agent::{Agent, DeathCause, SOCIAL_RATE, TICKS_PER_YEAR, Work};
use crate::buildings::{ABANDON_TICKS, BuildingKind, CROP_GROWTH_TICKS, CROP_SPOIL_TICKS, CROP_YIELD};
use crate::economy::Good;
use crate::state::{CivState, death_index};
use crate::terrain::Resource;
use crate::territory;
use crate::world::World;

/// Health lost per tick with zero energy.
pub const STARVATION_DAMAGE: f32 = 0.012;
/// Health lost per tick with saturated thirst.
pub const THIRST_DAMAGE: f32 = 0.015;
/// Health regained per tick when fed (energy > 0.4) and watered (thirst < 0.6).
pub const HEALING: f32 = 0.003;
/// Share of affinity kept per day (relationships fade without contact).
pub const RELATION_RETENTION: f32 = 0.98;
/// Share of learned selling stock kept per day.
pub const SALES_RETENTION: f32 = 0.97;
/// Share of carried food that spoils per tick (fresh food, half-life ≈ 29 days).
pub const CARRIED_SPOILAGE: f32 = 0.001;
/// Share of stored food that spoils per tick (stores, half-life ≈ 58 days).
pub const STORED_SPOILAGE: f32 = 0.0005;
/// Smoothing factor of the activity-share moving average.
const PRACTICE_SMOOTHING: f32 = 0.01;

/// Local threat in `[0, 1]`: half the danger summed over the 5×5 tiles
/// around the agent, plus 0.15 in another group's territory, damped by risk
/// tolerance (`× (1 − 0.6·risk)`).
fn threat(world: &World, a: &Agent) -> f32 {
    let Some(t) = world.tile_at(a.kin.position) else { return 0.0 };
    let (x, y) = world.coords(t);
    let mut danger = 0.0;
    for ny in y.saturating_sub(2)..=(y + 2).min(world.height() - 1) {
        for nx in x.saturating_sub(2)..=(x + 2).min(world.width() - 1) {
            danger += world.danger(world.index(nx, ny));
        }
    }
    let foreign = world.claim(t).owner.is_some_and(|g| Some(g) != a.group);
    let base = (0.5 * danger).min(1.0) + if foreign { 0.15 } else { 0.0 };
    (base * (1.0 - 0.6 * a.prefs.risk)).clamp(0.0, 1.0)
}

/// Needs, energy, health, skills and age for one tick. Returns the agents
/// that die, with the cause.
///
/// * energy: `E ← max(0, E − metabolism·(1 + effort))`, effort 0.6 when
///   working, 0.3 when walking, −0.3 when resting;
/// * thirst grows by `thirst_rate` (×1.3 when working), fatigue by
///   `fatigue_rate` (×1.5 working, ×1.2 walking) unless resting, loneliness by
///   [`SOCIAL_RATE`]; perceived danger relaxes towards the local threat
///   (`s ← s + 0.1·(threat − s)`);
/// * health falls by [`STARVATION_DAMAGE`] at zero energy and by
///   [`THIRST_DAMAGE`] at saturated thirst, and recovers by [`HEALING`] when
///   fed and watered; death at zero health or at the maximum age;
/// * skills learn/decay (see [`crate::agent::Skills::practice`]) and the
///   shares of the kinds of production ([`crate::agent::Work`]) follow an
///   exponential moving average over production ticks;
/// * carried food spoils at [`CARRIED_SPOILAGE`] per tick.
pub fn physiology(state: &mut CivState, work: &TickWork) -> Vec<(AgentId, DeathCause)> {
    let CivState { agents, world, rules, .. } = state;
    let mut deaths = Vec::new();
    for (slot, (id, a)) in agents.iter_mut().enumerate() {
        let practised = work.practised.get(slot).copied().flatten();
        a.skills.practice(practised, rules.learning_rate, rules.forgetting_rate);
        if let Some(kind) = practised.and_then(|skill| Work::of(skill, a.doing)).map(Work::index) {
            for (k, p) in a.practice.iter_mut().enumerate() {
                let hit = if k == kind { 1.0 } else { 0.0 };
                *p += PRACTICE_SMOOTHING * (hit - *p);
            }
        }
        let working = practised.is_some();
        let moved = work.moved.get(slot).copied().unwrap_or(false);
        let rested = work.rested.get(slot).copied().unwrap_or(false);
        let effort = if working {
            0.6
        } else if moved {
            0.3
        } else if rested {
            -0.3
        } else {
            0.0
        };
        a.energy = (a.energy - rules.metabolism * (1.0 + effort)).max(0.0);
        a.needs.water.rate = rules.thirst_rate * if working { 1.3 } else { 1.0 };
        a.needs.water.update(1.0);
        if !rested {
            a.needs.rest.rate = rules.fatigue_rate
                * if working {
                    1.5
                } else if moved {
                    1.2
                } else {
                    1.0
                };
            a.needs.rest.update(1.0);
        }
        a.needs.social.rate = SOCIAL_RATE;
        a.needs.social.update(1.0);
        a.needs.safety += 0.1 * (threat(world, a) - a.needs.safety);
        if a.energy <= 0.0 {
            a.health -= STARVATION_DAMAGE;
            a.last_damage = Some(DeathCause::Starvation);
        }
        if a.needs.water.is_saturated() {
            a.health -= THIRST_DAMAGE;
            a.last_damage = Some(DeathCause::Thirst);
        }
        if a.energy > 0.4 && a.needs.water.value < 0.6 {
            a.health = (a.health + HEALING).min(1.0);
        }
        a.health = a.health.max(0.0);
        let food = a.inventory.get(Good::Food);
        a.inventory.set(Good::Food, food * (1.0 - CARRIED_SPOILAGE));
        a.age += 1;
        if a.health <= 0.0 {
            deaths.push((id, a.last_damage.unwrap_or(DeathCause::Starvation)));
        } else if a.age >= a.max_age {
            deaths.push((id, DeathCause::Age));
        }
    }
    deaths
}

/// Removes the dead and commits births. Inheritance follows Sugarscape rule
/// I: a dead agent's goods are split equally among its living children; its
/// buildings pass to the eldest living child (or stay with its group).
pub fn commit(state: &mut CivState, deaths: &[(AgentId, DeathCause)]) {
    for (id, _) in deaths {
        state.agents.queue_despawn(*id);
    }
    let report = state.agents.commit();
    for (id, dead) in report.despawned {
        let cause = deaths.iter().find(|(d, _)| *d == id).map_or(DeathCause::Age, |(_, c)| *c);
        state.stats.deaths[death_index(cause)] += 1;
        let heirs: Vec<AgentId> =
            state.agents.iter().filter(|(_, a)| a.parents.contains(&Some(id))).map(|(i, _)| i).collect();
        if !heirs.is_empty() {
            let share = 1.0 / heirs.len() as f32;
            for heir in &heirs {
                if let Some(a) = state.agents.get_mut(*heir) {
                    for g in Good::ALL {
                        a.inventory.add(g, dead.inventory.get(g) * share);
                    }
                }
            }
        }
        let eldest = heirs.first().copied();
        for b in state.buildings.iter_mut().filter(|b| b.owner == Some(id)) {
            b.owner = eldest;
            if let Some(a) = eldest.and_then(|h| state.agents.get_mut(h)) {
                match b.kind {
                    BuildingKind::Hut if a.home.is_none() => a.home = Some(b.id),
                    BuildingKind::FarmPlot if a.farm.is_none() => a.farm = Some(b.id),
                    _ => {}
                }
            }
        }
        state.events.push(
            SimEvent::new(
                state.tick,
                "death",
                format!("{id} died of {} at age {}", cause.label(), dead.age / TICKS_PER_YEAR),
            )
            .subject(id.0),
        );
    }
}

/// Crop growth (`growth += fertility · crop multiplier / CROP_GROWTH_TICKS`;
/// at maturity the yield is `CROP_YIELD · fertility · quality`; ripe crops rot
/// after [`CROP_SPOIL_TICKS`]), spoilage of stored food ([`STORED_SPOILAGE`])
/// and decay of abandoned buildings.
pub fn upkeep(state: &mut CivState) {
    let CivState { world, buildings, agents, rules, events, tick, .. } = state;
    let crop_multiplier = rules.regen_of(Resource::Crops);
    for b in buildings.iter_mut() {
        if let Some(crop) = &mut b.crop {
            if crop.growth < 1.0 {
                let fertility = world.terrain(b.tile).fertility();
                crop.growth = (crop.growth + fertility * crop_multiplier / CROP_GROWTH_TICKS).min(1.0);
                if crop.growth >= 1.0 {
                    crop.yield_left = CROP_YIELD * fertility * crop.quality;
                }
            } else {
                crop.ripe_ticks += 1;
            }
        }
        if b.crop.is_some_and(|c| c.growth >= 1.0 && c.ripe_ticks > CROP_SPOIL_TICKS) {
            b.crop = None;
        }
        let stored = b.store.get(Good::Food);
        b.store.set(Good::Food, stored * (1.0 - STORED_SPOILAGE));
        if b.owner.is_none() && b.group.is_none() {
            b.condition -= 1.0 / ABANDON_TICKS;
        }
    }
    for b in buildings.remove_where(|b| b.condition > 0.0) {
        world.set_building(b.tile, 0);
        for a in agents.as_mut_slice() {
            if a.home == Some(b.id) {
                a.home = None;
            }
            if a.farm == Some(b.id) {
                a.farm = None;
            }
        }
        events.push(
            SimEvent::new(*tick, "ruin", format!("abandoned {} {} fell apart", b.kind.label(), b.id.0))
                .severity(EventSeverity::Trace),
        );
    }
}

/// Daily bookkeeping: relationships fade (`a ← 0.98·a`, dead agents
/// forgotten, memory truncated), learned selling stocks fade (×0.97), groups
/// are recomputed, buildings follow
/// their owner's group, and territory is recomputed.
pub fn daily(state: &mut CivState) {
    let cap = state.rules.max_relationships as usize;
    let alive: Vec<AgentId> = state.agents.ids().to_vec();
    for a in state.agents.as_mut_slice() {
        a.relations.decay(RELATION_RETENTION, |id| alive.binary_search(&id).is_ok());
        a.relations.truncate(cap);
        for g in Good::ALL {
            a.sales.set(g, a.sales.get(g) * SALES_RETENTION);
        }
    }
    let CivState { agents, groups, buildings, world, rules, events, tick, .. } = state;
    groups.update(agents, rules, *tick, events);
    for b in buildings.iter_mut() {
        b.group = match b.owner.and_then(|o| agents.get(o)) {
            Some(owner) => owner.group,
            None => b.group.filter(|g| groups.get(*g).is_some()),
        };
    }
    let stores: Vec<crate::groups::GroupId> =
        buildings.iter().filter(|b| b.kind == BuildingKind::Storage).filter_map(|b| b.group).collect();
    for a in agents.as_mut_slice() {
        a.group_storage = a.group.is_some_and(|g| stores.contains(&g));
    }
    territory::update(world, buildings, groups, rules);
}
