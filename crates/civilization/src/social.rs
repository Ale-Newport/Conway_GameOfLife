//! Interactions between agents: barter, sharing, socialising, familiarity,
//! theft and fights, territorial defence and births.
//!
//! Mutual interactions (trade, pairing) are matched with a greedy maximal
//! matching over candidate pairs sorted by score, then by the pair's ids, so
//! every agent takes part in at most one per tick and the result does not
//! depend on iteration order. One-sided contests (theft, defence of a
//! territory) are resolved with organism-engine's exclusive claims: each
//! victim or trespasser is confronted by at most one agent per tick, the
//! claim with the highest priority winning.
//!
//! **Fights** use a Tullock contest success function: agent `a` wins with
//! probability `s_a / (s_a + s_b)`, with strength
//! `s = (0.5 + health)(0.5 + energy)(1.3 if armed with tools)(0.4 if a child)`.
//! The loser loses 0.3 health and the winner 0.08; both lose 0.3 affinity
//! towards each other and danger is added to the surrounding tiles.

use organism_engine::AgentId;
use organism_engine::intent::{Claim, resolve_exclusive};
use simulation_core::{EventSeverity, SimEvent};

use crate::act::{TickWork, finish};
use crate::agent::{
    Activity, Agent, BIRTH_GIFT, DeathCause, Needs, Preferences, SOCIAL_RATE, Sex, Skill, TICKS_PER_YEAR, Target,
};
use crate::decide::are_kin;
use crate::economy::{Good, Trader, barter, meeting_steps, trade_potential};
use crate::navigate::AGENT_REACH;
use crate::state::{CivState, Scratch};
use crate::step::index_agents;

/// Radius within which agents get familiar with each other.
pub const FAMILIARITY_RADIUS: f32 = 2.0;
/// Radius within which group members notice a trespasser.
pub const DEFENCE_RADIUS: f32 = 5.0;
/// Per-tick probability scale of confronting a trespasser.
pub const DEFEND_CHANCE: f64 = 0.15;
/// Largest learned selling stock of one good.
pub const MAX_SALES_STOCK: f32 = 12.0;
/// Food taken by a successful theft at most.
pub const THEFT_AMOUNT: f32 = 3.0;

/// Greedy maximal matching on unordered pairs: edges are taken in
/// decreasing score (ties by the lower, then the higher id) and accepted
/// when neither agent is matched yet.
pub fn match_pairs(edges: &[(f32, AgentId, AgentId)]) -> Vec<(AgentId, AgentId)> {
    let mut sorted: Vec<(f32, AgentId, AgentId)> =
        edges.iter().filter(|(_, a, b)| a != b).map(|&(s, a, b)| (s, a.min(b), a.max(b))).collect();
    sorted.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let mut used: Vec<AgentId> = Vec::new();
    let mut pairs = Vec::new();
    for (_, a, b) in sorted {
        if !used.contains(&a) && !used.contains(&b) {
            used.push(a);
            used.push(b);
            pairs.push((a, b));
        }
    }
    pairs
}

/// Runs every interaction phase. The spatial grid is rebuilt first because
/// agents moved since decisions were made.
pub fn interact(state: &mut CivState, scratch: &mut Scratch, work: &mut TickWork) {
    index_agents(state, scratch);
    trade(state, work);
    share(state);
    socialize(state, scratch);
    if state.rules.conflict_enabled {
        theft(state, scratch);
        defend(state, scratch, work);
    }
    births(state);
}

/// The agent targeted by `agent`'s goal, if alive and within reach.
fn partner_in_reach(state: &CivState, agent: &Agent, activity: Activity) -> Option<AgentId> {
    if agent.goal.activity != activity {
        return None;
    }
    let Target::Agent(other) = agent.goal.target else { return None };
    let o = state.agents.get(other)?;
    (o.kin.position.distance(agent.kin.position) <= AGENT_REACH).then_some(other)
}

fn trade(state: &mut CivState, work: &mut TickWork) {
    let mut edges = Vec::new();
    let mut initiators = Vec::new();
    for (id, a) in state.agents.iter() {
        let Some(other) = partner_in_reach(state, a, Activity::Trade) else { continue };
        initiators.push(id);
        let Some(b) = state.agents.get(other) else { continue };
        let willing = state.rules.trade_enabled
            && a.is_adult()
            && b.is_adult()
            && b.prefs.trade >= 0.1
            && b.relations.affinity(id) >= -0.3;
        if willing {
            let (wa, wb) = (a.demand_weights(), b.demand_weights());
            let p = trade_potential(
                Trader { holdings: &a.inventory, weights: &wa },
                Trader { holdings: &b.inventory, weights: &wb },
            );
            edges.push((p, id, other));
        }
    }
    let cap = state.rules.max_relationships as usize;
    for (x, y) in match_pairs(&edges) {
        let (Some(sx), Some(sy)) = (state.agents.index_of(x), state.agents.index_of(y)) else { continue };
        let Some((a, b)) = state.agents.get_pair_mut(x, y) else { continue };
        let (wa, wb) = (a.demand_weights(), b.demand_weights());
        let steps = meeting_steps(a.skills.get(Skill::Trading), b.skills.get(Skill::Trading));
        let outcome = barter(&mut a.inventory, &wa, &mut b.inventory, &wb, steps);
        let (ma, mb) = (a.memory, b.memory);
        a.memory.learn_from(&mb);
        b.memory.learn_from(&ma);
        if outcome.steps > 0 {
            // Sellers learn what is in demand; both remember the market place.
            for g in Good::ALL {
                let delta = outcome.delta_a.get(g);
                let seller = if delta < 0.0 { &mut *a } else { &mut *b };
                let sold = (seller.sales.get(g) + delta.abs()).min(MAX_SALES_STOCK);
                seller.sales.set(g, sold);
            }
            let market = state.world.tile_at(a.kin.position);
            a.memory.market = market.or(a.memory.market);
            b.memory.market = market.or(b.memory.market);
            a.relations.adjust(y, 0.04 * a.prefs.cooperation, cap);
            b.relations.adjust(x, 0.04 * b.prefs.cooperation, cap);
            state.stats.trades += 1;
            state.stats.trade_volume += outcome.volume;
            work.practised[sx] = Some(Skill::Trading);
            work.practised[sy] = Some(Skill::Trading);
        }
    }
    for id in initiators {
        if let Some(a) = state.agents.get_mut(id) {
            finish(a);
        }
    }
}

fn share(state: &mut CivState) {
    let cap = state.rules.max_relationships as usize;
    let gifts: Vec<(AgentId, AgentId)> = state
        .agents
        .iter()
        .filter_map(|(id, a)| partner_in_reach(state, a, Activity::Share).map(|r| (id, r)))
        .collect();
    for (giver, receiver) in gifts {
        let Some((g, r)) = state.agents.get_pair_mut(giver, receiver) else { continue };
        let give = (g.inventory.get(Good::Food) - 0.5 * g.prefs.food_reserve()).clamp(0.0, 3.0);
        if give > 0.05 {
            g.inventory.add(Good::Food, -give);
            r.inventory.add(Good::Food, give);
            r.relations.adjust(giver, 0.15, cap);
            g.relations.adjust(receiver, 0.05, cap);
            state.stats.shared_food += give;
        }
        finish(g);
    }
}

fn socialize(state: &mut CivState, scratch: &mut Scratch) {
    let cap = state.rules.max_relationships as usize;
    let mut changes: Vec<(AgentId, AgentId, f32)> = Vec::new();
    let mut comfort: Vec<(AgentId, f32)> = Vec::new();
    let mut talks: Vec<(AgentId, AgentId)> = Vec::new();
    let mut done: Vec<AgentId> = Vec::new();
    let ids = state.agents.ids();
    let mut near = Vec::new();
    for (slot, (id, a)) in state.agents.iter().enumerate() {
        if let Some(other) = partner_in_reach(state, a, Activity::Socialize)
            && let Some(o) = state.agents.get(other)
        {
            let gain = 0.04 * a.prefs.cooperation * o.prefs.cooperation;
            changes.push((id, other, gain));
            changes.push((other, id, gain));
            talks.push((id, other));
            comfort.push((id, 0.05));
            comfort.push((other, 0.05));
            if a.needs.social.value < 0.1 {
                done.push(id);
            }
        }
        // Familiarity with everyone close by, scaled by own cooperativeness.
        scratch.grid.query(a.kin.position, FAMILIARITY_RADIUS, &mut near);
        near.sort_unstable();
        let mut friend_near = false;
        for &j in near.iter().filter(|&&j| j as usize != slot) {
            let other = ids[j as usize];
            changes.push((id, other, 0.002 * a.prefs.cooperation));
            friend_near |= a.relations.affinity(other) > 0.2;
        }
        if friend_near {
            comfort.push((id, 0.006));
        }
    }
    for (who, towards, delta) in changes {
        if let Some(a) = state.agents.get_mut(who) {
            a.relations.adjust(towards, delta, cap);
        }
    }
    for (x, y) in talks {
        if let Some((a, b)) = state.agents.get_pair_mut(x, y) {
            let (ma, mb) = (a.memory, b.memory);
            a.memory.learn_from(&mb);
            b.memory.learn_from(&ma);
        }
    }
    for (who, amount) in comfort {
        if let Some(a) = state.agents.get_mut(who) {
            a.needs.social.satisfy(amount);
        }
    }
    for id in done {
        if let Some(a) = state.agents.get_mut(id) {
            finish(a);
        }
    }
}

fn strength(a: &Agent) -> f64 {
    let armed = if a.inventory.get(Good::Tools) > 0.0 { 1.3 } else { 1.0 };
    let maturity = if a.is_adult() { 1.0 } else { 0.4 };
    f64::from((0.5 + a.health) * (0.5 + a.energy) * armed * maturity)
}

/// Fight between `a` and `b`; returns the winner (one random draw).
fn fight(state: &mut CivState, a: AgentId, b: AgentId) -> Option<AgentId> {
    let (sa, sb) = (strength(state.agents.get(a)?), strength(state.agents.get(b)?));
    let a_wins = state.rng.chance(sa / (sa + sb));
    let (winner, loser) = if a_wins { (a, b) } else { (b, a) };
    let cap = state.rules.max_relationships as usize;
    let (w, l) = state.agents.get_pair_mut(winner, loser)?;
    l.health = (l.health - 0.3).max(0.0);
    l.last_damage = Some(DeathCause::Conflict);
    l.needs.safety = (l.needs.safety + 0.4).min(1.0);
    w.health = (w.health - 0.08).max(0.0);
    w.last_damage = Some(DeathCause::Conflict);
    l.relations.adjust(winner, -0.3, cap);
    w.relations.adjust(loser, -0.3, cap);
    let place = state.world.tile_at(l.kin.position);
    if let Some(t) = place {
        state.world.add_danger(t, 1.0);
    }
    state.stats.conflicts += 1;
    state.events.push(
        SimEvent::new(state.tick, "fight", format!("{winner} beat {loser} in a fight"))
            .subject(winner.0)
            .object(loser.0)
            .severity(EventSeverity::Trace),
    );
    Some(winner)
}

fn theft(state: &mut CivState, scratch: &mut Scratch) {
    let claims: Vec<Claim> = state
        .agents
        .iter()
        .filter_map(|(id, a)| {
            partner_in_reach(state, a, Activity::Steal).map(|v| Claim::new(id, v.0).priority(a.hunger()))
        })
        .collect();
    let cap = state.rules.max_relationships as usize;
    let mut near = Vec::new();
    for claim in resolve_exclusive(&claims) {
        let (thief, victim) = (claim.claimant, AgentId(claim.target));
        let Some(v) = state.agents.get(victim) else { continue };
        let resists = v.is_adult() && state.rng.chance(f64::from(0.2 + 0.6 * v.prefs.risk));
        let victim_group = v.group;
        let victim_pos = v.kin.position;
        let thief_wins = if resists { fight(state, thief, victim) == Some(thief) } else { true };
        if let Some((t, v)) = state.agents.get_pair_mut(thief, victim) {
            if thief_wins {
                let amount = v.inventory.get(Good::Food).min(THEFT_AMOUNT);
                v.inventory.add(Good::Food, -amount);
                t.inventory.add(Good::Food, amount);
                state.stats.thefts += 1;
            }
            v.relations.adjust(thief, -0.5, cap);
            t.relations.adjust(victim, -0.1, cap);
        }
        if let Some(tile) = state.world.tile_at(victim_pos) {
            state.world.add_danger(tile, 0.5);
        }
        // Group members who see it resent the thief.
        if let Some(g) = victim_group {
            scratch.grid.query(victim_pos, state.rules.vision as f32, &mut near);
            near.sort_unstable();
            let witnesses: Vec<AgentId> = near
                .iter()
                .map(|&s| state.agents.ids()[s as usize])
                .filter(|&w| w != thief && w != victim && state.agents.get(w).is_some_and(|a| a.group == Some(g)))
                .collect();
            for w in witnesses {
                if let Some(a) = state.agents.get_mut(w) {
                    a.relations.adjust(thief, -0.2, cap);
                }
            }
        }
    }
    for claim in claims {
        if let Some(a) = state.agents.get_mut(claim.claimant) {
            finish(a);
        }
    }
}

fn defend(state: &mut CivState, scratch: &mut Scratch, work: &TickWork) {
    let mut claims = Vec::new();
    let mut near = Vec::new();
    for g in &work.gathered {
        let claim = state.world.claim(g.tile);
        let Some(owner) = claim.owner else { continue };
        let Some(gatherer) = state.agents.get(g.agent) else { continue };
        if gatherer.group == Some(owner) {
            continue;
        }
        let pos = gatherer.kin.position;
        scratch.grid.query(pos, DEFENCE_RADIUS, &mut near);
        near.sort_unstable();
        for &slot in &near {
            let defender_id = state.agents.ids()[slot as usize];
            let d = &state.agents.as_slice()[slot as usize];
            if d.group != Some(owner) || !d.is_adult() || defender_id == g.agent {
                continue;
            }
            let aggression =
                (1.0 - d.prefs.cooperation) * (0.3 + 0.7 * d.prefs.risk) * if claim.contested { 1.5 } else { 1.0 };
            let distance = d.kin.position.distance(pos);
            if state.rng.chance(DEFEND_CHANCE * f64::from(aggression)) {
                claims.push(Claim::new(defender_id, g.agent.0).priority(aggression).distance(distance));
            }
        }
    }
    for claim in resolve_exclusive(&claims) {
        let trespasser = AgentId(claim.target);
        if fight(state, claim.claimant, trespasser) == Some(trespasser) {
            continue;
        }
        // The loser drops what it gathered here this tick.
        for g in work.gathered.iter().filter(|g| g.agent == trespasser) {
            let Some(t) = state.agents.get_mut(trespasser) else { break };
            let dropped = t.inventory.get(g.good).min(g.amount);
            t.inventory.add(g.good, -dropped);
            finish(t);
            let stock = state.world.stock(g.tile);
            state.world.set_stock(g.tile, stock + dropped);
        }
    }
}

fn births(state: &mut CivState) {
    if !state.rules.births_enabled {
        return;
    }
    let tick = state.tick;
    let mut edges = Vec::new();
    for (id, a) in state.agents.iter() {
        let Some(other) = partner_in_reach(state, a, Activity::Mate) else { continue };
        let Some(b) = state.agents.get(other) else { continue };
        let affinity = a.relations.affinity(other).min(b.relations.affinity(id));
        if a.sex != b.sex
            && affinity >= 0.2
            && !are_kin(id, a, other, b)
            && a.can_reproduce(tick, &state.rules)
            && b.can_reproduce(tick, &state.rules)
        {
            edges.push((affinity, id, other));
        }
    }
    let cap = state.rules.max_relationships as usize;
    for (x, y) in match_pairs(&edges) {
        if state.agents.len() + state.agents.pending().spawns().len() >= state.rules.max_population as usize {
            break;
        }
        let (Some(a), Some(b)) = (state.agents.get(x), state.agents.get(y)) else { continue };
        let (mother_id, father_id) = if a.sex == Sex::Female { (x, y) } else { (y, x) };
        let (mother, father) = if a.sex == Sex::Female { (a, b) } else { (b, a) };
        let prefs = Preferences::inherit(&mother.prefs, &father.prefs, 0.05, &mut state.rng);
        let sex = if state.rng.chance(0.5) { Sex::Female } else { Sex::Male };
        let max_age = state.rng.range_i64(55 * TICKS_PER_YEAR as i64, 75 * TICKS_PER_YEAR as i64 + 1) as u64;
        let needs = Needs::new(state.rules.thirst_rate, state.rules.fatigue_rate, SOCIAL_RATE);
        let mut child = Agent::new(mother.kin.position, sex, 0, max_age, prefs, needs);
        child.inventory.set(Good::Food, 2.0 * BIRTH_GIFT);
        child.group = mother.group;
        child.home = mother.home.or(father.home);
        child.population = mother.population;
        child.parents = [Some(mother_id), Some(father_id)];
        child.generation = mother.generation.max(father.generation) + 1;
        child.memory = mother.memory;
        child.memory.learn_from(&father.memory);
        child.relations.adjust(mother_id, 0.6, cap);
        child.relations.adjust(father_id, 0.6, cap);
        let child_id = state.agents.queue_spawn(child);
        if let Some((m, f)) = state.agents.get_pair_mut(mother_id, father_id) {
            for parent in [m, f] {
                parent.inventory.add(Good::Food, -BIRTH_GIFT);
                parent.last_birth = Some(tick);
                parent.relations.adjust(child_id, 0.6, cap);
                if parent.goal.activity == Activity::Mate {
                    finish(parent);
                }
            }
        }
        state.stats.births += 1;
        state.events.push(
            SimEvent::new(tick, "birth", format!("{child_id} was born to {mother_id} and {father_id}"))
                .subject(child_id.0)
                .object(mother_id.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::tests::state;
    use crate::agent::Goal;

    #[test]
    fn theft_from_a_child_moves_food_and_breeds_resentment() {
        let mut s = state(2);
        let mut scratch = Scratch::new(&s.world);
        let (thief, victim) = (s.agents.ids()[0], s.agents.ids()[1]);
        let spot = s.world.center(s.world.index(4, 4));
        for (slot, food) in [(0, 0.0), (1, 5.0)] {
            let a = &mut s.agents.as_mut_slice()[slot];
            a.kin.position = spot;
            a.inventory.set(Good::Food, food);
        }
        // Children never resist, so the outcome is deterministic.
        s.agents.as_mut_slice()[1].age = 0;
        s.agents.as_mut_slice()[0].goal = Goal { activity: Activity::Steal, target: Target::Agent(victim), plan: None };
        index_agents(&s, &mut scratch);
        theft(&mut s, &mut scratch);
        let (t, v) = (&s.agents.as_slice()[0], &s.agents.as_slice()[1]);
        assert_eq!(t.inventory.get(Good::Food), THEFT_AMOUNT);
        assert_eq!(v.inventory.get(Good::Food), 5.0 - THEFT_AMOUNT);
        assert_eq!(v.relations.affinity(thief), -0.5);
        assert_eq!(t.relations.affinity(victim), -0.1);
        assert_eq!((s.stats.thefts, s.stats.conflicts), (1, 0));
        assert!(s.world.danger(s.world.index(4, 4)) > 0.0, "the place feels dangerous");
        assert_eq!(t.goal.activity, Activity::Idle, "one attempt per decision");
    }

    #[test]
    fn stronger_agents_win_fights_more_often() {
        let mut s = state(2);
        let (a, b) = (s.agents.ids()[0], s.agents.ids()[1]);
        let mut wins = 0;
        for _ in 0..400 {
            for (slot, health) in [(0, 1.0), (1, 0.2)] {
                let x = &mut s.agents.as_mut_slice()[slot];
                x.health = health;
                x.energy = 0.8;
            }
            wins += u32::from(fight(&mut s, a, b) == Some(a));
        }
        // P(a wins) = s_a/(s_a + s_b) = 1.5/(1.5 + 0.7) ≈ 0.68.
        let p = f64::from(wins) / 400.0;
        assert!((p - 1.5 / 2.2).abs() < 0.07, "{p}");
        assert_eq!(s.stats.conflicts, 400);
    }

    #[test]
    fn matching_is_maximal_and_order_independent() {
        let e = |s: f32, a: u64, b: u64| (s, AgentId(a), AgentId(b));
        let edges = vec![e(0.5, 1, 2), e(0.9, 2, 3), e(0.5, 3, 4), e(0.2, 1, 4), e(0.9, 3, 2)];
        let pairs = match_pairs(&edges);
        assert_eq!(pairs, vec![(AgentId(2), AgentId(3)), (AgentId(1), AgentId(4))]);
        let mut reversed = edges.clone();
        reversed.reverse();
        assert_eq!(match_pairs(&reversed), pairs);
    }
}
