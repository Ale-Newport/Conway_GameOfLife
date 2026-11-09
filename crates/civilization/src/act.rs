//! Movement along planned routes and individual actions: eating, drinking,
//! resting, gathering, farming, crafting, building and storing.
//!
//! Contested resources are claimed first and resolved afterwards with
//! organism-engine's deterministic intent resolution: wild stocks and group
//! stores are split proportionally among all claimants of a tile or store
//! ([`resolve_shared`]), while a farm plot or a free building site goes to a
//! single claimant ([`resolve_exclusive`], ranked by priority, distance, then
//! id).

use organism_engine::intent::{Claim, ShareRule, resolve_exclusive, resolve_shared};
use organism_engine::kinematics::{Bounds, MotionLimits, Movement};
use organism_engine::{AgentId, steering};
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent};

use crate::agent::{Activity, Agent, FOOD_ENERGY, Goal, Skill, Target};
use crate::buildings::{BuildingId, BuildingKind, Crop, SEED_FOOD};
use crate::economy::Good;
use crate::navigate::in_reach;
use crate::state::CivState;
use crate::terrain::Resource;

/// Movement points per tick (tiles per tick on cost-1 terrain).
pub const BASE_SPEED: f32 = 1.0;
/// Units gathered per tick by an agent with productivity 1.
pub const GATHER_RATE: f32 = 1.0;
/// Units harvested per tick by an agent with productivity 1.
pub const HARVEST_RATE: f32 = 3.0;
/// Tools made per tick by an agent with productivity 1 (each from one wood and one stone).
pub const CRAFT_RATE: f32 = 0.08;
/// Construction work per tick by an agent with productivity 1.
pub const BUILD_RATE: f32 = 0.06;
/// Tool units worn per tick of productive work.
pub const TOOL_WEAR: f32 = 0.01;
/// Food eaten per tick at most.
pub const EAT_RATE: f32 = 2.0;
/// Thirst removed per tick of drinking at a shore.
pub const DRINK_RATE: f32 = 0.25;

/// Wild goods gathered by one agent this tick (for territorial defence).
#[derive(Clone, Copy, Debug)]
pub struct Gathered {
    /// Gatherer.
    pub agent: AgentId,
    /// Tile the goods came from.
    pub tile: u32,
    /// Amount.
    pub amount: f32,
    /// Good.
    pub good: Good,
}

/// Per-tick bookkeeping shared by the phases of a step.
#[derive(Clone, Debug, Default)]
pub struct TickWork {
    /// Skill practised by each agent (store slot order).
    pub practised: Vec<Option<Skill>>,
    /// Whether each agent moved.
    pub moved: Vec<bool>,
    /// Whether each agent rested.
    pub rested: Vec<bool>,
    /// Wild goods gathered.
    pub gathered: Vec<Gathered>,
}

/// Ends the current goal so the agent decides again next tick.
pub fn finish(agent: &mut Agent) {
    agent.goal = Goal::default();
    agent.path.clear();
}

/// Moves every agent along its route. Each tick is split into
/// `⌈1/(1 − k)⌉` sub-steps (`k` = road discount) so that no sub-step covers
/// more than one tile; in each sub-step the agent steers with
/// [`steering::arrive`] towards its next waypoint at speed
/// `BASE_SPEED / cost(current tile)` and is integrated with
/// [`organism_engine::Kinematics::integrate_with`] (impassable destinations
/// are refused). Entering a tile deposits footfall on it.
pub fn move_agents(state: &mut CivState, work: &mut TickWork) {
    let CivState { world, agents, rules, .. } = state;
    let (w, h) = (world.width() as f32, world.height() as f32);
    let bounds = Bounds::Clamp { min: Vec2::ZERO, max: Vec2::new(w - 1e-3, h - 1e-3) };
    // Agents chasing a moving partner walk straight at it once their route ends.
    let chase: Vec<Option<u32>> = agents
        .as_slice()
        .iter()
        .map(|a| match a.goal.target {
            Target::Agent(other) if a.path.is_empty() => agents
                .get(other)
                .filter(|o| o.kin.position.distance(a.kin.position) <= 4.0)
                .and_then(|o| world.tile_at(o.kin.position)),
            _ => None,
        })
        .collect();
    let fastest = BASE_SPEED / (1.0 - rules.road_discount).max(0.1);
    let substeps = fastest.ceil().max(1.0) as u32;
    let dt = 1.0 / substeps as f32;
    work.moved = vec![false; agents.len()];
    for (slot, agent) in agents.as_mut_slice().iter_mut().enumerate() {
        if let Some(t) = chase[slot] {
            agent.path = vec![t];
        }
        let mut tile = world.tile_at(agent.kin.position);
        for _ in 0..substeps {
            let Some(&next) = agent.path.last() else { break };
            let target = world.center(next);
            let cost = tile.map(|t| world.move_cost(t, rules)).filter(|c| c.is_finite()).unwrap_or(1.0).max(0.1);
            let limits = MotionLimits::new(BASE_SPEED / cost, f32::INFINITY);
            let accel = steering::arrive(&agent.kin, target, limits.max_speed * dt, limits) / dt;
            if agent.kin.integrate_with(accel, dt, limits, bounds, |p| world.passable_at(p)) == Movement::Blocked {
                agent.path.clear();
                break;
            }
            let now = world.tile_at(agent.kin.position);
            if now != tile {
                if let Some(t) = now {
                    world.add_footfall(t);
                }
                tile = now;
                work.moved[slot] = true;
            }
            if agent.kin.position.distance(target) < 0.05 {
                agent.path.pop();
            }
        }
        if agent.path.is_empty() {
            agent.kin.velocity = Vec2::ZERO;
        }
    }
}

/// Executes the individual (non-social) actions of agents that reached their
/// targets, resolving contested resources deterministically.
pub fn perform(state: &mut CivState, work: &mut TickWork) {
    let n = state.agents.len();
    work.practised = vec![None; n];
    work.rested = vec![false; n];
    work.gathered.clear();
    let reach: Vec<bool> = state
        .agents
        .as_slice()
        .iter()
        .map(|a| in_reach(&state.world, &state.agents, &state.buildings, a, a.goal.target))
        .collect();
    let mut gather_claims = Vec::new();
    let mut farm_claims = Vec::new();
    let mut site_claims = Vec::new();
    let mut store_claims = Vec::new();
    let tool_bonus = state.rules.tool_bonus;
    let ids: Vec<AgentId> = state.agents.ids().to_vec();
    let CivState { world, agents, buildings, stats, events, tick, .. } = state;
    for (slot, agent) in agents.as_mut_slice().iter_mut().enumerate() {
        let id = ids[slot];
        let goal = agent.goal;
        agent.doing = goal.activity;
        if !reach[slot] {
            if agent.path.is_empty() && !matches!(goal.target, Target::Agent(_)) {
                finish(agent);
            }
            continue;
        }
        agent.path.clear();
        let pos = agent.kin.position;
        let distance = |t: u32| pos.distance(world.center(t));
        match (goal.activity, goal.target) {
            (Activity::Eat, Target::None) => {
                let food = agent.inventory.get(Good::Food);
                let amount = food.min((1.0 - agent.energy) / FOOD_ENERGY).clamp(0.0, EAT_RATE);
                agent.inventory.add(Good::Food, -amount);
                agent.energy = (agent.energy + amount * FOOD_ENERGY).min(1.0);
                if agent.energy >= 0.95 || agent.inventory.get(Good::Food) < 0.05 {
                    finish(agent);
                }
            }
            (Activity::Eat, Target::Building(b)) => match buildings.get(b).filter(|x| agent.may_use_store(x)) {
                Some(store) => {
                    let want = ((1.0 - agent.energy) / FOOD_ENERGY).min(EAT_RATE);
                    store_claims.push(
                        Claim::new(id, u64::from(b.0))
                            .amount(want)
                            .priority(agent.hunger())
                            .distance(distance(store.tile)),
                    );
                }
                None => finish(agent),
            },
            // Only a shore (land touching water) gives water; a remembered
            // shore can dry up when the terrain is edited.
            (Activity::Drink, Target::Tile(t)) if world.near_water(t) => {
                agent.needs.water.satisfy(DRINK_RATE);
                agent.memory.water = Some(t);
                if agent.needs.water.value < 0.05 {
                    finish(agent);
                }
            }
            (Activity::Drink, _) => {
                agent.memory.water = None;
                finish(agent);
            }
            (Activity::Rest, target) => {
                let at_home =
                    matches!(target, Target::Building(b) if buildings.get(b).is_some_and(|x| x.is_complete()));
                agent.needs.rest.satisfy(if at_home { 0.07 } else { 0.04 });
                agent.needs.safety *= if at_home { 0.8 } else { 0.95 };
                work.rested[slot] = true;
                if agent.needs.rest.value < 0.1 && agent.needs.safety < 0.3 {
                    finish(agent);
                }
            }
            (Activity::GatherFood | Activity::GatherWood | Activity::GatherStone, Target::Tile(t)) => {
                let request =
                    (GATHER_RATE * agent.productivity(Skill::Gathering, tool_bonus)).min(agent.free_capacity());
                if request <= 0.01 || world.stock(t) < 0.05 {
                    finish(agent);
                } else {
                    gather_claims.push(Claim::new(id, u64::from(t)).amount(request).distance(distance(t)));
                }
            }
            (Activity::Farm, Target::Building(b)) => {
                let d = buildings.get(b).map_or(f32::INFINITY, |x| distance(x.tile));
                farm_claims.push(Claim::new(id, u64::from(b.0)).priority(agent.skills.get(Skill::Farming)).distance(d));
            }
            (Activity::Craft, _) => {
                let wood = agent.inventory.get(Good::Wood);
                let stone = agent.inventory.get(Good::Stone);
                let amount = (CRAFT_RATE * agent.productivity(Skill::Crafting, tool_bonus)).min(wood).min(stone);
                agent.inventory.add(Good::Wood, -amount);
                agent.inventory.add(Good::Stone, -amount);
                wear_tools(agent);
                agent.inventory.add(Good::Tools, amount);
                work.practised[slot] = Some(Skill::Crafting);
                if agent.inventory.get(Good::Wood) < 0.05
                    || agent.inventory.get(Good::Stone) < 0.05
                    || agent.inventory.get(Good::Tools) >= 2.0
                {
                    finish(agent);
                }
            }
            (Activity::Build, Target::Building(b)) => {
                let productivity = agent.productivity(Skill::Building, tool_bonus);
                match buildings.get_mut(b) {
                    Some(site) if !site.is_complete() => {
                        site.progress = (site.progress + BUILD_RATE * productivity / site.kind.work()).min(1.0);
                        wear_tools(agent);
                        work.practised[slot] = Some(Skill::Building);
                        if site.is_complete() {
                            stats.completed += 1;
                            events.push(
                                SimEvent::new(*tick, "built", format!("{} {} completed", site.kind.label(), site.id.0))
                                    .subject(id.0),
                            );
                            finish(agent);
                        }
                    }
                    _ => finish(agent),
                }
            }
            (Activity::Build, Target::Tile(t)) => match goal.plan {
                Some(kind) => site_claims.push((Claim::new(id, u64::from(t)).distance(distance(t)), kind)),
                None => finish(agent),
            },
            (Activity::Store, Target::Building(b)) => {
                let extra = agent.inventory.get(Good::Food) - agent.food_to_carry();
                if let Some(store) = buildings.get_mut(b).filter(|s| agent.may_use_store(s))
                    && extra > 0.0
                {
                    store.store.add(Good::Food, extra);
                    agent.inventory.add(Good::Food, -extra);
                    // Only pooling in a group store counts as sharing.
                    if store.kind == BuildingKind::Storage {
                        stats.shared_food += extra;
                    }
                }
                finish(agent);
            }
            // Arrived at a remembered market place (or done exploring): look around again.
            (Activity::Explore | Activity::Trade, Target::Tile(_)) => finish(agent),
            // Social activities are handled by the social phase.
            (Activity::Trade | Activity::Socialize | Activity::Share | Activity::Steal | Activity::Mate, _) => {}
            _ => finish(agent),
        }
    }
    resolve_gathering(state, work, &gather_claims);
    resolve_farming(state, work, &farm_claims);
    resolve_sites(state, &site_claims);
    resolve_store_meals(state, &store_claims);
}

fn wear_tools(agent: &mut Agent) {
    agent.inventory.add(Good::Tools, -TOOL_WEAR);
}

fn resolve_gathering(state: &mut CivState, work: &mut TickWork, claims: &[Claim]) {
    let CivState { world, agents, stats, .. } = state;
    for alloc in resolve_shared(claims, |t| world.stock(t as u32), ShareRule::Proportional) {
        let tile = alloc.target as u32;
        let Some(slot) = agents.index_of(alloc.claimant) else { continue };
        let good = match world.terrain(tile).resource() {
            Some(Resource::Berries | Resource::Fish) => Good::Food,
            Some(Resource::Wood) => Good::Wood,
            Some(Resource::Stone) => Good::Stone,
            _ => continue,
        };
        let taken = world.take(tile, alloc.granted);
        let agent = &mut agents.as_mut_slice()[slot];
        agent.inventory.add(good, taken);
        wear_tools(agent);
        work.practised[slot] = Some(Skill::Gathering);
        stats.gathered += taken;
        work.gathered.push(Gathered { agent: alloc.claimant, tile, amount: taken, good });
        if world.stock(tile) < 0.5 || agent.free_capacity() < 0.5 {
            finish(agent);
        }
    }
}

fn resolve_farming(state: &mut CivState, work: &mut TickWork, claims: &[Claim]) {
    let tool_bonus = state.rules.tool_bonus;
    let CivState { agents, buildings, stats, .. } = state;
    let winners = resolve_exclusive(claims);
    for claim in claims.iter().filter(|c| !winners.iter().any(|w| w.claimant == c.claimant)) {
        if let Some(agent) = agents.get_mut(claim.claimant) {
            finish(agent);
        }
    }
    for claim in winners {
        let Some(slot) = agents.index_of(claim.claimant) else { continue };
        let agent = &mut agents.as_mut_slice()[slot];
        let Some(plot) = buildings.get_mut(BuildingId(claim.target as u32)) else {
            finish(agent);
            continue;
        };
        match plot.crop {
            None if agent.inventory.get(Good::Food) >= SEED_FOOD => {
                agent.inventory.add(Good::Food, -SEED_FOOD);
                plot.crop = Some(Crop {
                    growth: 0.0,
                    quality: 0.5 + agent.skills.get(Skill::Farming),
                    yield_left: 0.0,
                    ripe_ticks: 0,
                });
                work.practised[slot] = Some(Skill::Farming);
                finish(agent);
            }
            Some(mut crop) if crop.is_ripe() => {
                let amount = (HARVEST_RATE * agent.productivity(Skill::Farming, tool_bonus))
                    .min(crop.yield_left)
                    .min(agent.free_capacity());
                crop.yield_left -= amount;
                agent.inventory.add(Good::Food, amount);
                wear_tools(agent);
                stats.gathered += amount;
                work.practised[slot] = Some(Skill::Farming);
                plot.crop = (crop.yield_left > 0.01).then_some(crop);
                if plot.crop.is_none() || agent.free_capacity() < 0.5 {
                    finish(agent);
                }
            }
            _ => finish(agent),
        }
    }
}

fn resolve_sites(state: &mut CivState, claims: &[(Claim, BuildingKind)]) {
    let bare: Vec<Claim> = claims.iter().map(|(c, _)| *c).collect();
    let winners = resolve_exclusive(&bare);
    let CivState { world, agents, buildings, events, tick, .. } = state;
    for (claim, kind) in claims {
        let won = winners.iter().any(|w| w.claimant == claim.claimant && w.target == claim.target);
        let Some(agent) = agents.get_mut(claim.claimant) else { continue };
        let tile = claim.target as u32;
        if !won || world.building_at(tile) != 0 || !agent.inventory.covers(&kind.cost()) {
            finish(agent);
            continue;
        }
        agent.inventory.subtract(&kind.cost());
        let id = buildings.insert(*kind, tile, claim.claimant, agent.group);
        world.set_building(tile, id.0);
        match kind {
            BuildingKind::Hut if agent.home.is_none() => agent.home = Some(id),
            BuildingKind::FarmPlot if agent.farm.is_none() => agent.farm = Some(id),
            _ => {}
        }
        agent.goal = Goal { activity: Activity::Build, target: Target::Building(id), plan: None };
        events.push(
            SimEvent::new(*tick, "site", format!("{} site {} laid out", kind.label(), id.0))
                .subject(claim.claimant.0)
                .severity(EventSeverity::Trace),
        );
    }
}

fn resolve_store_meals(state: &mut CivState, claims: &[Claim]) {
    let CivState { agents, buildings, .. } = state;
    let available = |b: u64| buildings.get(BuildingId(b as u32)).map_or(0.0, |x| x.store.get(Good::Food));
    let allocations = resolve_shared(claims, available, ShareRule::Proportional);
    for alloc in allocations {
        let Some(agent) = agents.get_mut(alloc.claimant) else { continue };
        let Some(store) = buildings.get_mut(BuildingId(alloc.target as u32)) else {
            finish(agent);
            continue;
        };
        let eaten = alloc.granted.min(store.store.get(Good::Food));
        store.store.add(Good::Food, -eaten);
        agent.energy = (agent.energy + eaten * FOOD_ENERGY).min(1.0);
        if agent.energy >= 0.95 || store.store.get(Good::Food) < 0.05 || eaten <= 0.0 {
            finish(agent);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use simulation_core::ParamSet;

    use super::*;
    use crate::config::{Dist, PopulationSpec, PreferenceDist, Rules, Scenario, WorldSpec};
    use crate::sim::build_state;
    use crate::terrain::TerrainSpec;

    /// An 8×8 map with a lake along the top row and `n` settlers.
    pub(crate) fn state(n: u32) -> CivState {
        let rows: Vec<String> = (0..8).map(|y| if y == 0 { "~".repeat(8) } else { ".".repeat(8) }).collect();
        let d = |mean| Dist { mean, sd: 0.0 };
        let scenario = Scenario {
            id: "t".into(),
            name: "t".into(),
            description: String::new(),
            seed: 1,
            world: WorldSpec { terrain: TerrainSpec::Map { rows }, richness: 1.0 },
            populations: vec![PopulationSpec {
                label: "p".into(),
                count: n,
                preferences: PreferenceDist { risk: d(0.5), cooperation: d(0.5), trade: d(0.5), time: d(0.5) },
                region: None,
                kin_affinity: 0.0,
                starting_food: 0.0,
            }],
            params: ParamSet::new(),
        };
        match build_state(&scenario, &Rules::default(), 1) {
            Ok(s) => s,
            Err(e) => panic!("test world: {e}"),
        }
    }

    /// Finished hut of the first agent at tile (3, 4); everyone stands on it.
    fn with_hut(state: &mut CivState) -> BuildingId {
        let tile = state.world.index(3, 4);
        let owner = state.agents.ids()[0];
        let hut = state.buildings.insert(BuildingKind::Hut, tile, owner, None);
        if let Some(b) = state.buildings.get_mut(hut) {
            b.progress = 1.0;
        }
        state.world.set_building(tile, hut.0);
        let centre = state.world.center(tile);
        for a in state.agents.as_mut_slice() {
            a.kin.position = centre;
        }
        state.agents.as_mut_slice()[0].home = Some(hut);
        hut
    }

    fn set_goal(state: &mut CivState, slot: usize, activity: Activity, target: Target) {
        state.agents.as_mut_slice()[slot].goal = Goal { activity, target, plan: None };
    }

    #[test]
    fn household_store_takes_surplus_from_residents_only() {
        let mut s = state(2);
        let hut = with_hut(&mut s);
        for slot in 0..2 {
            s.agents.as_mut_slice()[slot].inventory.set(Good::Food, 20.0);
            set_goal(&mut s, slot, Activity::Store, Target::Building(hut));
        }
        let keep = s.agents.as_slice()[0].food_to_carry();
        assert_eq!(keep, 5.0, "reserve 2 + 6·(1 − 0.5)");
        perform(&mut s, &mut TickWork::default());
        let stored = s.buildings.get(hut).map_or(0.0, |b| b.store.get(Good::Food));
        assert!((stored - 15.0).abs() < 1e-5, "the resident deposits food above what it keeps: {stored}");
        assert!((s.agents.as_slice()[0].inventory.get(Good::Food) - keep).abs() < 1e-5);
        assert_eq!(s.agents.as_slice()[1].inventory.get(Good::Food), 20.0, "a stranger cannot use the hut");
        assert_eq!(s.stats.shared_food, 0.0, "household deposits are not sharing");
        // Food kept for sale stays in the pocket; stocks for sale fill at
        // most half the carrying capacity together.
        let a = &mut s.agents.as_mut_slice()[0];
        a.sales.set(Good::Food, 11.0);
        assert_eq!(a.food_to_carry(), 11.0);
        a.sales = crate::economy::Bundle::new(12.0, 12.0, 12.0, 3.0);
        let t = a.sale_targets();
        assert!((t.get(Good::Food) + t.get(Good::Wood) + t.get(Good::Stone) - 12.0).abs() < 1e-5);
        assert_eq!((t.get(Good::Food), t.get(Good::Tools)), (4.0, 3.0));
        assert_eq!(a.food_to_carry(), 5.0, "the reserve wins over a scaled-down sale target");
    }

    #[test]
    fn residents_eat_from_the_household_store() {
        let mut s = state(2);
        let hut = with_hut(&mut s);
        if let Some(b) = s.buildings.get_mut(hut) {
            b.store.set(Good::Food, 10.0);
        }
        for slot in 0..2 {
            let a = &mut s.agents.as_mut_slice()[slot];
            a.energy = 0.3;
            a.inventory.set(Good::Food, 0.0);
            set_goal(&mut s, slot, Activity::Eat, Target::Building(hut));
        }
        perform(&mut s, &mut TickWork::default());
        let resident = &s.agents.as_slice()[0];
        assert!((resident.energy - (0.3 + EAT_RATE * FOOD_ENERGY)).abs() < 1e-5, "{}", resident.energy);
        let left = s.buildings.get(hut).map_or(0.0, |b| b.store.get(Good::Food));
        assert!((left - (10.0 - EAT_RATE)).abs() < 1e-5);
        let stranger = &s.agents.as_slice()[1];
        assert_eq!(stranger.energy, 0.3, "no access for non-residents");
        assert_eq!(stranger.goal.activity, Activity::Idle);
    }

    #[test]
    fn drinking_needs_a_shore() {
        let mut s = state(2);
        let shore = s.world.index(3, 1);
        let dry = s.world.index(3, 5);
        assert!(s.world.near_water(shore) && !s.world.near_water(dry));
        for (slot, tile) in [(0, shore), (1, dry)] {
            let centre = s.world.center(tile);
            let a = &mut s.agents.as_mut_slice()[slot];
            a.kin.position = centre;
            a.needs.water.value = 0.9;
            a.memory.water = Some(tile);
            set_goal(&mut s, slot, Activity::Drink, Target::Tile(tile));
        }
        perform(&mut s, &mut TickWork::default());
        let (wet, parched) = (&s.agents.as_slice()[0], &s.agents.as_slice()[1]);
        assert!((wet.needs.water.value - (0.9 - DRINK_RATE)).abs() < 1e-6);
        assert_eq!(parched.needs.water.value, 0.9, "no water away from the shore");
        assert_eq!(parched.memory.water, None, "a dry 'shore' is forgotten");
        assert_eq!(parched.goal.activity, Activity::Idle);
    }
}
