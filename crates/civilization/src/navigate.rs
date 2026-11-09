//! Navigation helpers for decisions and actions: where to stand to act on
//! a tile, when a target is within reach, route planning and exploration
//! targets.

use organism_engine::AgentStore;
use simulation_core::SimRng;
use simulation_core::glam::Vec2;

use crate::agent::{Agent, Goal, Target};
use crate::buildings::Buildings;
use crate::decide::Env;
use crate::path::PathFinder;
use crate::world::World;

/// Distance from a target tile's centre within which tile actions happen.
pub const REACH: f32 = 1.6;
/// Distance within which agents interact with each other.
pub const AGENT_REACH: f32 = 2.0;
/// A* search budget (tiles expanded) of one route.
const SEARCH_BUDGET: usize = 4000;

/// Tile an agent at `from` should stand on to act on `t`: `t` itself when
/// passable, else its nearest passable neighbour — in both cases only a tile
/// the agent can walk to.
pub fn stand_tile(world: &World, t: u32, from: Vec2) -> Option<u32> {
    let here = world.tile_at(from)?;
    if world.passable(t) {
        return world.connected(here, t).then_some(t);
    }
    let mut best: Option<(u32, f32)> = None;
    for n in world.neighbors(t).filter(|&n| world.connected(here, n)) {
        let d = from.distance_squared(world.center(n));
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((n, d));
        }
    }
    best.map(|(n, _)| n)
}

/// Exploration target (up to 6 attempts, two draws each). Wandering picks a
/// random tile within vision; a survival search is a correlated random walk
/// that heads beyond vision (`vision·U[1, 2]` tiles away) within ±90° of the
/// current heading, which covers new ground much faster than wandering.
pub(crate) fn explore_tile(world: &World, me: &Agent, vision: u32, searching: bool, rng: &mut SimRng) -> Option<u32> {
    let pos = me.kin.position;
    let here = world.tile_at(pos)?;
    let r = vision.max(1) as f32;
    for _ in 0..6 {
        let offset = if searching {
            let angle = me.kin.heading + rng.range_f32(-0.5, 0.5) * std::f32::consts::PI;
            Vec2::new(angle.cos(), angle.sin()) * r * rng.range_f32(1.0, 2.0)
        } else {
            Vec2::new(rng.range_f32(-r, r), rng.range_f32(-r, r))
        };
        if let Some(t) = world.tile_at(pos + offset).filter(|&t| world.connected(here, t)) {
            return Some(t);
        }
    }
    None
}

/// Tile the agent should walk to for `goal`, if any.
pub fn destination(
    world: &World,
    agents: &AgentStore<Agent>,
    buildings: &Buildings,
    me: &Agent,
    goal: Goal,
) -> Option<u32> {
    let pos = me.kin.position;
    match goal.target {
        Target::None => None,
        Target::Tile(t) => stand_tile(world, t, pos),
        Target::Building(b) => buildings.get(b).map(|b| b.tile),
        Target::Agent(a) => agents.get(a).and_then(|other| world.tile_at(other.kin.position)),
    }
}

/// Whether the agent is close enough to its goal's target to act.
pub fn in_reach(world: &World, agents: &AgentStore<Agent>, buildings: &Buildings, me: &Agent, target: Target) -> bool {
    let pos = me.kin.position;
    match target {
        Target::None => true,
        Target::Tile(t) => pos.distance(world.center(t)) <= REACH,
        Target::Building(b) => buildings.get(b).is_some_and(|b| pos.distance(world.center(b.tile)) <= REACH),
        Target::Agent(a) => agents.get(a).is_some_and(|o| pos.distance(o.kin.position) <= AGENT_REACH),
    }
}

/// Route for `goal` (empty when already in reach), or `None` when the target
/// is unreachable (checked in O(1) with the world's region labels before
/// searching).
pub(crate) fn route(env: &Env<'_>, me: &Agent, goal: Goal, paths: &mut PathFinder) -> Option<Vec<u32>> {
    if in_reach(env.world, env.agents, env.buildings, me, goal.target) {
        return Some(Vec::new());
    }
    let dest = destination(env.world, env.agents, env.buildings, me, goal)?;
    let start = env.world.tile_at(me.kin.position)?;
    if !env.world.connected(start, dest) {
        return None;
    }
    paths.find(env.world, env.rules, start, dest, SEARCH_BUDGET)
}
