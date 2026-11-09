//! One tick of the civilization model.
//!
//! ```text
//! daily     (every 24 ticks) relationship decay, groups, building groups, territory
//! index     spatial grid of agent positions (slot = store index)
//! decide    agents whose goal ended, expired or is pre-empted choose a new one (utility AI, A*)
//! move      sub-stepped steering along routes; footfall builds trails
//! act       eat, drink, rest, craft, build, store; claims on tiles, plots, sites, stores resolved
//! interact  barter, sharing, socialising, familiarity, theft, territorial defence, births
//! life      metabolism, needs, health, skills, ageing; deaths and births committed; inheritance
//! world     crop growth, building decay, resource growback, trail and danger decay
//! ```
//!
//! Agents are always processed in id order and every random number comes
//! from the state's single generator, so a run is a pure function of the
//! scenario, the seed and the commands applied.

use crate::act::{self, TickWork};
use crate::agent::{Activity, Agent, TICKS_PER_DAY};
use crate::decide::{self, DECISION_INTERVAL, Decision, Env};
use crate::life;
use crate::navigate::in_reach;
use crate::social;
use crate::state::{CivState, Scratch, TickStats};

/// Whether `a` should choose a new goal this tick.
fn needs_decision(a: &Agent, env: &Env<'_>) -> bool {
    let goal = a.goal.activity;
    // Urgent needs pre-empt goals that do not address them; a search trip for
    // the missing resource (nothing remembered) is left to finish.
    let exploring = goal == Activity::Explore;
    let thirst_interrupt =
        a.needs.water.value >= 0.8 && goal != Activity::Drink && !(exploring && a.memory.water.is_none());
    let hunger_interrupt = a.hunger() >= 0.8
        && !matches!(goal, Activity::Eat | Activity::GatherFood | Activity::Farm | Activity::Steal)
        && !(exploring && a.memory.food.is_none());
    let stuck = a.path.is_empty() && !in_reach(env.world, env.agents, env.buildings, a, a.goal.target);
    goal == Activity::Idle || env.tick >= a.next_decision || thirst_interrupt || hunger_interrupt || stuck
}

/// Rebuilds the spatial index (slots are store indices; populations are
/// capped far below `u32::MAX`).
pub fn index_agents(state: &CivState, scratch: &mut Scratch) {
    scratch.grid.rebuild(state.agents.as_slice().iter().enumerate().map(|(i, a)| (i as u32, a.kin.position)));
}

/// Advances `state` by one tick.
pub fn step(state: &mut CivState, scratch: &mut Scratch) {
    state.stats = TickStats::default();
    if state.tick.is_multiple_of(TICKS_PER_DAY) {
        life::daily(state);
    }
    index_agents(state, scratch);
    let decisions: Vec<(usize, Decision)> = {
        let env = Env {
            world: &state.world,
            agents: &state.agents,
            buildings: &state.buildings,
            grid: &scratch.grid,
            rules: &state.rules,
            tick: state.tick,
        };
        let mut out = Vec::new();
        for (slot, a) in state.agents.as_slice().iter().enumerate() {
            if needs_decision(a, &env) {
                out.push((slot, decide::decide(&env, slot, &mut state.rng, &mut scratch.paths, &mut scratch.percepts)));
            }
        }
        out
    };
    for (slot, decision) in decisions {
        let a = &mut state.agents.as_mut_slice()[slot];
        // Commit to a trip at least until the expected arrival.
        a.next_decision = state.tick + DECISION_INTERVAL + decision.path.len() as u64;
        a.goal = decision.goal;
        a.path = decision.path;
        a.memory = decision.memory;
    }
    let mut work = TickWork::default();
    act::move_agents(state, &mut work);
    act::perform(state, &mut work);
    social::interact(state, scratch, &mut work);
    let deaths = life::physiology(state, &work);
    life::commit(state, &deaths);
    life::upkeep(state);
    state.world.update(&state.rules);
    state.tick += 1;
}
