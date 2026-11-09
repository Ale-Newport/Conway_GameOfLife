//! Goal selection with utility AI.
//!
//! An agent reconsiders its goal when the previous one finished or failed,
//! when an urgent need interrupts it, and otherwise every
//! [`DECISION_INTERVAL`] ticks. It scans the tiles within its vision once
//! (best food, wood, stone, shore, farm plot, store, construction site and
//! building site), perceives the agents within vision through the spatial
//! grid, scores every feasible action with organism-engine's
//! [`ActionScore`] (product of response-curve considerations with the
//! compensation factor) and picks the best with [`choose_best`] (5 %
//! multiplicative noise, one draw per option). The current goal gets a 15 %
//! momentum bonus. A route to the chosen target is planned with A*; if no
//! route exists the next-best option is tried.

use organism_engine::perception::{self, Query};
use organism_engine::utility::{ActionScore, Consideration, Curve, choose_best};
use organism_engine::{AgentId, AgentStore};
use simulation_core::{SimRng, SpatialGrid};

use crate::agent::{Activity, Agent, CARRY_CAPACITY, Goal, Memory, Skill, Target};
use crate::buildings::{BuildingId, BuildingKind, Buildings, SEED_FOOD};
use crate::config::Rules;
use crate::economy::{Good, Trader, expected_gain, marginal_need, meeting_steps, trade_potential};
use crate::navigate::{REACH, explore_tile, route, stand_tile};
use crate::path::PathFinder;
use crate::terrain::Resource;
use crate::world::World;

/// Ticks between regular goal reconsiderations.
pub const DECISION_INTERVAL: u64 = 8;
/// Smallest trade potential (`1 − e^{−|Δ ln MRS|}`, here an MRS ratio of
/// about 1.33) for a perceived agent to be considered as a partner.
pub const MIN_TRADE_POTENTIAL: f32 = 0.25;
/// Expected welfare gain `Δ ln W` at which a meeting's value reaches
/// `1 − 1/e` (value `1 − e^{−Δ ln W / GAIN_SCALE}`).
pub const GAIN_SCALE: f64 = 0.05;
/// Smallest meeting value worth walking to a partner for (a gain of
/// `Δ ln W ≈ 0.014`, i.e. 1.4 % more welfare).
pub const MIN_TRADE_VALUE: f32 = 0.25;
/// Partners (best by MRS gap and distance) whose meeting is simulated.
const TRADE_CANDIDATES: usize = 4;
const MOMENTUM: f32 = 1.15;
const NOISE: f32 = 0.05;

static EAT: Curve = Curve::Logistic { k: 12.0, x0: 0.45 };
static DRINK: Curve = Curve::Logistic { k: 12.0, x0: 0.55 };
static PARCHED: Curve = Curve::Logistic { k: 25.0, x0: 0.85 };
static TIRED: Curve = Curve::Logistic { k: 12.0, x0: 0.6 };
static AFRAID: Curve = Curve::Logistic { k: 10.0, x0: 0.55 };
static LONELY: Curve = Curve::Logistic { k: 10.0, x0: 0.5 };
static DESPERATE: Curve = Curve::Logistic { k: 10.0, x0: 0.65 };
static LINEAR: Curve = Curve::Linear;
static NEAR: Curve = Curve::InverseLinear;

/// Read-only view of the state used while deciding.
pub struct Env<'a> {
    /// Tile world.
    pub world: &'a World,
    /// All agents.
    pub agents: &'a AgentStore<Agent>,
    /// Buildings.
    pub buildings: &'a Buildings,
    /// Agent positions by store slot.
    pub grid: &'a SpatialGrid,
    /// Active rules.
    pub rules: &'a Rules,
    /// Current tick.
    pub tick: u64,
}

/// A chosen goal and the route to it (next waypoint last).
pub struct Decision {
    /// New goal.
    pub goal: Goal,
    /// Route.
    pub path: Vec<u32>,
    /// Updated memory of places.
    pub memory: Memory,
}

#[derive(Clone, Copy, Debug)]
struct Spot {
    tile: u32,
    dist: f32,
    value: f32,
}

#[derive(Default)]
struct Scan {
    food: Option<Spot>,
    wood: Option<Spot>,
    stone: Option<Spot>,
    water: Option<Spot>,
    plant: Option<(BuildingId, Spot)>,
    harvest: Option<(BuildingId, Spot)>,
    storage: Option<(BuildingId, Spot)>,
    home_store: Option<(BuildingId, Spot)>,
    site: Option<(BuildingId, Spot)>,
    new_site: Option<Spot>,
}

fn better(slot: Option<Spot>, value: f32) -> bool {
    slot.is_none_or(|s| value > s.value)
}

fn better_b(slot: Option<(BuildingId, Spot)>, value: f32) -> bool {
    slot.is_none_or(|(_, s)| value > s.value)
}

/// Attractiveness factor of working on tile `t`: foreign territory deters
/// risk-averse agents and recent danger deters everyone but the bold.
fn place_factor(world: &World, t: u32, me: &Agent) -> f32 {
    let claim = world.claim(t);
    let risk = me.prefs.risk;
    let territory = match claim.owner {
        Some(g) if Some(g) != me.group => {
            if claim.contested {
                0.6 + 0.4 * risk
            } else {
                0.35 + 0.65 * risk
            }
        }
        _ => 1.0,
    };
    territory * (1.0 - world.danger(t).min(1.0) * (1.0 - risk) * 0.8)
}

fn scan(env: &Env<'_>, me_id: AgentId, me: &Agent) -> Scan {
    let world = env.world;
    let pos = me.kin.position;
    let r = i64::from(env.rules.vision);
    let (cx, cy) = (pos.x.floor() as i64, pos.y.floor() as i64);
    let (w, h) = (i64::from(world.width()), i64::from(world.height()));
    let plan = me.building_plan().filter(|k| me.inventory.covers(&k.cost()));
    let mut s = Scan::default();
    let Some(here) = world.tile_at(pos) else { return s };
    for y in (cy - r).max(0)..=(cy + r).min(h - 1) {
        for x in (cx - r).max(0)..=(cx + r).min(w - 1) {
            if (x - cx).pow(2) + (y - cy).pow(2) > r * r {
                continue;
            }
            let t = world.index(x as u32, y as u32);
            let d = pos.distance(world.center(t));
            let terrain = world.terrain(t);
            if world.near_water(t) && better(s.water, -d) && world.connected(here, t) {
                s.water = Some(Spot { tile: t, dist: d, value: -d });
            }
            if let Some(resource) = terrain.resource()
                && world.stock(t) >= 1.0
            {
                let v = world.stock(t).min(4.0) / 4.0 * place_factor(world, t, me) / (1.0 + 0.12 * d);
                let slot = match resource {
                    Resource::Berries | Resource::Fish => &mut s.food,
                    Resource::Wood => &mut s.wood,
                    Resource::Stone => &mut s.stone,
                    Resource::Crops => continue,
                };
                if better(*slot, v) && stand_tile(world, t, pos).is_some() {
                    *slot = Some(Spot { tile: t, dist: d, value: v });
                }
            }
            let raw = world.building_at(t);
            if !world.connected(here, t) {
                continue;
            }
            if raw != 0 {
                if let Some(b) = env.buildings.get(BuildingId(raw)) {
                    let mine = b.owner == Some(me_id);
                    let ours = b.group.is_some() && b.group == me.group;
                    if !b.is_complete() {
                        let v = if mine { 2.0 } else { 1.0 } / (1.0 + 0.1 * d);
                        if (mine || (ours && me.prefs.cooperation > 0.4)) && better_b(s.site, v) {
                            s.site = Some((b.id, Spot { tile: t, dist: d, value: v }));
                        }
                    } else if b.kind == BuildingKind::FarmPlot && (mine || ours) {
                        let spot = Spot { tile: t, dist: d, value: -d };
                        match b.crop {
                            None if better_b(s.plant, -d) => s.plant = Some((b.id, spot)),
                            Some(c) if c.is_ripe() && better_b(s.harvest, -d) => s.harvest = Some((b.id, spot)),
                            _ => {}
                        }
                    } else if b.kind == BuildingKind::Storage && ours && better_b(s.storage, -d) {
                        s.storage = Some((b.id, Spot { tile: t, dist: d, value: -d }));
                    }
                }
            } else if let Some(kind) = plan {
                let claim = world.claim(t);
                let own = claim.owner.is_some() && claim.owner == me.group;
                let foreign = claim.owner.is_some() && !own;
                if terrain.buildable() && !foreign {
                    let near_water = me.memory.water.is_some_and(|w| world.center(w).distance(world.center(t)) <= 6.0);
                    let base = match kind {
                        BuildingKind::Hut => (1.0 + if own { 0.6 } else { 0.0 }) * if near_water { 1.5 } else { 1.0 },
                        BuildingKind::FarmPlot => terrain.fertility() * if own { 1.3 } else { 1.0 },
                        BuildingKind::Storage => {
                            if own {
                                2.0
                            } else {
                                1.0
                            }
                        }
                    };
                    let v = base / (1.0 + 0.15 * d);
                    if base > 0.0 && better(s.new_site, v) {
                        s.new_site = Some(Spot { tile: t, dist: d, value: v });
                    }
                }
            }
        }
    }
    // The household store in the agent's own hut, wherever it is.
    if let Some(b) = me.home.and_then(|id| env.buildings.get(id))
        && me.may_use_store(b)
        && world.connected(here, b.tile)
    {
        let d = pos.distance(world.center(b.tile));
        s.home_store = Some((b.id, Spot { tile: b.tile, dist: d, value: -d }));
    }
    // Remembered places beyond vision: own unfinished home/farm, last shore.
    for b in [me.home, me.farm].into_iter().flatten().filter_map(|id| env.buildings.get(id)) {
        if !b.is_complete() && s.site.is_none_or(|(id, _)| env.buildings.get(id).is_none_or(|x| x.owner != Some(me_id)))
        {
            let d = pos.distance(world.center(b.tile));
            s.site = Some((b.id, Spot { tile: b.tile, dist: d, value: 2.0 / (1.0 + 0.1 * d) }));
        }
    }
    s
}

/// Updates the agent's memory with what it sees now (shores, and resource
/// patches holding at least 2 units) and falls back on remembered places for
/// goods not in sight. A remembered resource tile that is in sight but
/// exhausted, a remembered shore that no longer touches water, and any place
/// that can no longer be reached are forgotten. Remembered spots score as
/// uncertain (`0.25 / (1 + 0.05·d)`).
fn remember(world: &World, me: &Agent, vision: f32, s: &mut Scan) -> Memory {
    let pos = me.kin.position;
    let mut memory = me.memory;
    let slots = [
        (&mut s.water, &mut memory.water, true),
        (&mut s.food, &mut memory.food, false),
        (&mut s.wood, &mut memory.wood, false),
        (&mut s.stone, &mut memory.stone, false),
    ];
    for (seen, remembered, is_water) in slots {
        if let Some(spot) = seen {
            // Remember shores and rich resource patches (stock ≥ 2).
            if is_water || world.stock(spot.tile) >= 2.0 {
                *remembered = Some(spot.tile);
            }
            continue;
        }
        let Some(t) = *remembered else { continue };
        let d = pos.distance(world.center(t));
        // A patch seen exhausted, a shore that is no longer one (terrain
        // edits) and an unreachable place are forgotten.
        let exhausted = if is_water { !world.near_water(t) } else { d <= vision && world.stock(t) < 1.0 };
        if exhausted || stand_tile(world, t, pos).is_none() {
            *remembered = None;
            continue;
        }
        *seen = Some(Spot { tile: t, dist: d, value: 0.25 / (1.0 + 0.05 * d) });
    }
    memory
}

#[derive(Default)]
struct Options {
    goals: Vec<Goal>,
    scores: Vec<f32>,
}

impl Options {
    fn push(&mut self, activity: Activity, target: Target, score: ActionScore) {
        self.push_plan(activity, target, None, score);
    }

    fn push_plan(&mut self, activity: Activity, target: Target, plan: Option<BuildingKind>, score: ActionScore) {
        if score.value > 0.0 {
            self.goals.push(Goal { activity, target, plan });
            self.scores.push(score.value);
        }
    }
}

fn best_of(a: ActionScore, b: ActionScore) -> ActionScore {
    if b.value > a.value { b } else { a }
}

/// Distance consideration: falls linearly to half weight at twice the vision.
fn near(d: f32, vision: f32) -> Consideration<'static> {
    Consideration::new((d / (2.0 * vision)).min(1.0), &NEAR).weighted(0.5)
}

fn lin(x: f32) -> Consideration<'static> {
    Consideration::new(x, &LINEAR)
}

/// Whether `a` and `b` are parent and child.
pub fn are_kin(a_id: AgentId, a: &Agent, b_id: AgentId, b: &Agent) -> bool {
    a.parents.contains(&Some(b_id)) || b.parents.contains(&Some(a_id))
}

/// Chooses a new goal for the agent in store slot `slot`.
pub fn decide(
    env: &Env<'_>,
    slot: usize,
    rng: &mut SimRng,
    paths: &mut PathFinder,
    percepts: &mut Vec<perception::Percept>,
) -> Decision {
    let me_id = env.agents.ids()[slot];
    let me = &env.agents.as_slice()[slot];
    let pos = me.kin.position;
    let vision = env.rules.vision as f32;
    let mut s = scan(env, me_id, me);
    let memory = remember(env.world, me, vision, &mut s);
    let mut o = Options::default();

    let hunger = me.hunger();
    let food = me.inventory.get(Good::Food);
    let reserve = me.prefs.food_reserve();
    let coop = me.prefs.cooperation;
    let patience = 1.0 - me.prefs.time;
    let weights = me.demand_weights();
    // Holdings as valued for production decisions: carried goods plus the
    // food in the household store. Needs are absolute marginal utilities, so
    // they fall as holdings grow and gathering saturates.
    let mut valued = me.inventory;
    if let Some((b, _)) = s.home_store
        && let Some(store) = env.buildings.get(b)
    {
        valued.add(Good::Food, store.store.get(Good::Food));
    }
    let need = marginal_need(&valued, &weights);
    let need_of = |g: Good| need.get(g);
    let skill = |k: Skill| me.skills.get(k);

    // Survival: eat carried food, else from the nearest stocked store the
    // agent may use (group store in sight or its household store).
    if food >= 0.25 {
        o.push(Activity::Eat, Target::None, ActionScore::evaluate(3.0, &[Consideration::new(hunger, &EAT)]));
    } else if let Some((b, spot)) = [s.storage, s.home_store]
        .into_iter()
        .flatten()
        .filter(|(b, _)| env.buildings.get(*b).is_some_and(|x| x.store.get(Good::Food) >= 1.0))
        .min_by(|x, y| x.1.dist.total_cmp(&y.1.dist))
    {
        let score = ActionScore::evaluate(3.0, &[Consideration::new(hunger, &EAT), near(spot.dist, vision)]);
        o.push(Activity::Eat, Target::Building(b), score);
    }
    if let Some(spot) = s.water {
        // Critical thirst kills fastest, so it outranks everything regardless of distance.
        let score = best_of(
            ActionScore::evaluate(3.0, &[Consideration::new(me.needs.water.value, &DRINK), near(spot.dist, vision)]),
            ActionScore::evaluate(3.2, &[Consideration::new(me.needs.water.value, &PARCHED)]),
        );
        o.push(Activity::Drink, Target::Tile(spot.tile), score);
    }
    let rest = best_of(
        ActionScore::evaluate(2.0, &[Consideration::new(me.needs.rest.value, &TIRED)]),
        ActionScore::evaluate(1.6, &[Consideration::new(me.needs.safety, &AFRAID)]),
    );
    let home = me.home.and_then(|h| env.buildings.get(h)).filter(|b| b.is_complete());
    let rest_target = home
        .filter(|b| pos.distance(env.world.center(b.tile)) <= 2.0 * vision)
        .map_or(Target::None, |b| Target::Building(b.id));
    o.push(Activity::Rest, rest_target, rest);

    // Production.
    let free = me.free_capacity();
    if free >= 1.0 {
        let shortage = 1.0 - (food / reserve).min(1.0);
        // Saving up for the current building plan: the missing share of its
        // cost (discounted by impatience for anything but a first home).
        let plan = me.building_plan();
        let commitment = if plan == Some(BuildingKind::Hut) { 1.0 } else { 0.4 + 0.6 * patience };
        let deficit = |g: Good| {
            plan.map_or(0.0, |k| {
                let cost = k.cost().get(g);
                if cost > 0.0 { ((cost - me.inventory.get(g)) / cost).clamp(0.0, 1.0) * commitment } else { 0.0 }
            })
        };
        // Producing for sale: refilling the stock others have been buying.
        let sale_targets = me.sale_targets();
        let for_sale = |g: Good| {
            let target = sale_targets.get(g);
            if target >= 1.0 { me.prefs.trade * ((target - me.inventory.get(g)) / target).clamp(0.0, 1.0) } else { 0.0 }
        };
        let gather = [
            (
                Activity::GatherFood,
                s.food,
                need_of(Good::Food).max(shortage).max(hunger).max(for_sale(Good::Food)),
                1.6,
            ),
            (Activity::GatherWood, s.wood, need_of(Good::Wood).max(deficit(Good::Wood)).max(for_sale(Good::Wood)), 1.3),
            (
                Activity::GatherStone,
                s.stone,
                need_of(Good::Stone).max(deficit(Good::Stone)).max(for_sale(Good::Stone)),
                1.3,
            ),
        ];
        // Without food to eat, getting food is a survival action scored like eating.
        let survival = |spot: Spot| {
            ActionScore::evaluate(
                2.6,
                &[Consideration::new(hunger, &EAT), lin(spot.value.min(1.0)).weighted(0.3), near(spot.dist, vision)],
            )
        };
        let starving = food < 1.0;
        for (activity, spot, need, weight) in gather {
            if let Some(spot) = spot {
                let mut score = ActionScore::evaluate(
                    weight,
                    &[
                        lin(need),
                        lin(spot.value.min(1.0)).weighted(0.5),
                        lin(skill(Skill::Gathering)).weighted(0.25 + 0.5 * me.prefs.trade),
                    ],
                );
                if activity == Activity::GatherFood && starving {
                    score = best_of(score, survival(spot));
                }
                o.push(activity, Target::Tile(spot.tile), score);
            }
        }
        if let Some((b, spot)) = s.harvest {
            let mut score =
                ActionScore::evaluate(2.0, &[lin(0.5 + 0.5 * need_of(Good::Food)), near(spot.dist, vision)]);
            if starving {
                score = best_of(score, survival(Spot { value: 1.0, ..spot }));
            }
            o.push(Activity::Farm, Target::Building(b), score);
        }
    }
    if let Some((b, spot)) = s.plant
        && food >= SEED_FOOD
    {
        // Planting is an investment: worth more to the patient, and to those
        // whose food (carried and stored at home) is running low.
        let score = ActionScore::evaluate(
            1.2,
            &[
                lin(patience),
                lin(0.2 + 0.8 * need_of(Good::Food)),
                lin(skill(Skill::Farming)).weighted(0.5),
                near(spot.dist, vision),
            ],
        );
        o.push(Activity::Farm, Target::Building(b), score);
    }
    if me.inventory.get(Good::Wood) >= 1.0
        && me.inventory.get(Good::Stone) >= 1.0
        && me.inventory.get(Good::Tools) < 1.0
    {
        let score = ActionScore::evaluate(
            1.2,
            &[lin(need_of(Good::Tools)), lin(patience).weighted(0.6), lin(skill(Skill::Crafting)).weighted(0.4)],
        );
        o.push(Activity::Craft, Target::None, score);
    }
    if me.is_adult() {
        if let Some((b, spot)) = s.site {
            let own = env.buildings.get(b).is_some_and(|x| x.owner == Some(me_id));
            let score = ActionScore::evaluate(
                1.4,
                &[
                    lin(if own { 1.0 } else { coop }),
                    lin(skill(Skill::Building)).weighted(0.3),
                    near(spot.dist, vision),
                ],
            );
            o.push(Activity::Build, Target::Building(b), score);
        } else if let (Some(kind), Some(spot)) = (me.building_plan(), s.new_site)
            && me.inventory.covers(&kind.cost())
            && (kind != BuildingKind::Storage || s.storage.is_none())
        {
            let urgency = if kind == BuildingKind::Hut { 1.0 } else { patience };
            let score = ActionScore::evaluate(
                1.4,
                &[lin(urgency), lin(skill(Skill::Building)).weighted(0.3), near(spot.dist, vision)],
            );
            o.push_plan(Activity::Build, Target::Tile(spot.tile), Some(kind), score);
        }
    }

    // Social options from perceived agents.
    perception::within(env.grid, &Query::new(pos, vision), |slot_j, _| slot_j as usize != slot, percepts);
    social_options(env, me_id, me, &s, percepts, &mut o);

    // Exploration: a weak default, and a survival search when no water or
    // (for an agent without food) no food is known.
    let idle = ActionScore::evaluate(0.1, &[]);
    let thirst_search = ActionScore::evaluate(2.8, &[Consideration::new(me.needs.water.value, &DRINK)]);
    let food_search = ActionScore::evaluate(2.4, &[Consideration::new(hunger, &EAT)]);
    let mut explore = idle;
    if s.water.is_none() {
        explore = best_of(explore, thirst_search);
    }
    if s.food.is_none() && food < 1.0 {
        explore = best_of(explore, food_search);
    }
    let searching = explore.value > idle.value;
    if let Some(t) = explore_tile(env.world, me, env.rules.vision, searching, rng) {
        o.push(Activity::Explore, Target::Tile(t), explore);
    }

    for (goal, score) in o.goals.iter().zip(o.scores.iter_mut()) {
        if goal.activity == me.goal.activity && goal.target == me.goal.target {
            *score *= MOMENTUM;
        }
    }
    // Pick the best, falling back to the next best when no route exists.
    let mut scores = o.scores.clone();
    for _ in 0..3 {
        let Some(i) = choose_best(&scores, Some(&mut *rng), NOISE) else { break };
        let goal = o.goals[i];
        if let Some(path) = route(env, me, goal, paths) {
            return Decision { goal, path, memory };
        }
        scores[i] = 0.0;
    }
    Decision { goal: Goal::default(), path: Vec::new(), memory }
}

fn social_options(
    env: &Env<'_>,
    me_id: AgentId,
    me: &Agent,
    s: &Scan,
    percepts: &[perception::Percept],
    o: &mut Options,
) {
    let vision = env.rules.vision as f32;
    let food = me.inventory.get(Good::Food);
    let coop = me.prefs.cooperation;
    let hunger = me.hunger();
    let mut trade_candidates: Vec<(f32, AgentId, f32)> = Vec::new();
    let mut best_friend: Option<(AgentId, f32, f32)> = None;
    let mut neediest: Option<(AgentId, f32, f32)> = None;
    let mut victim: Option<(AgentId, f32, f32)> = None;
    let mut partner: Option<(AgentId, f32, f32)> = None;
    let weights = me.demand_weights();
    let surplus = food - 0.5 * me.prefs.food_reserve();
    let can_mate = me.can_reproduce(env.tick, env.rules);
    let here = env.world.tile_at(me.kin.position);
    for p in percepts {
        let other_id = env.agents.ids()[p.slot as usize];
        let other = &env.agents.as_slice()[p.slot as usize];
        let reachable = here.zip(env.world.tile_at(other.kin.position)).is_some_and(|(a, b)| env.world.connected(a, b));
        if !reachable {
            continue;
        }
        let affinity = me.relations.affinity(other_id);
        let d = p.distance;
        let kin = are_kin(me_id, me, other_id, other);
        let same_group = me.group.is_some() && me.group == other.group;
        if env.rules.trade_enabled && me.is_adult() && other.is_adult() && affinity >= -0.3 {
            let w2 = other.demand_weights();
            let potential = trade_potential(
                Trader { holdings: &me.inventory, weights: &weights },
                Trader { holdings: &other.inventory, weights: &w2 },
            );
            if potential >= MIN_TRADE_POTENTIAL {
                trade_candidates.push((potential / (1.0 + 0.05 * d), other_id, d));
            }
        }
        let friendliness = affinity + 0.01 / (1.0 + d);
        if best_friend.is_none_or(|(_, bv, _)| friendliness > bv) {
            best_friend = Some((other_id, friendliness, d));
        }
        if surplus >= 1.0 && (kin || same_group) && other.hunger() > 0.5 && other.inventory.get(Good::Food) < 1.0 {
            let v = other.hunger();
            if neediest.is_none_or(|(_, bv, _)| v > bv) {
                neediest = Some((other_id, v, d));
            }
        }
        if env.rules.conflict_enabled
            && me.is_adult()
            && !kin
            && (!same_group || coop < 0.5)
            && other.inventory.get(Good::Food) >= 1.5
        {
            let v = other.inventory.get(Good::Food).min(6.0) / 6.0 * (1.0 - affinity.max(0.0)) / (1.0 + 0.1 * d);
            if victim.is_none_or(|(_, bv, _)| v > bv) {
                victim = Some((other_id, v, d));
            }
        }
        if can_mate && other.sex != me.sex && !kin && affinity >= 0.2 && other.can_reproduce(env.tick, env.rules) {
            let v = affinity.min(other.relations.affinity(me_id));
            if v >= 0.2 && partner.is_none_or(|(_, bv, _)| v > bv) {
                partner = Some((other_id, v, d));
            }
        }
    }
    // A meeting is worth its expected welfare gain (a barter simulated on
    // copies of both holdings), checked for the most promising partners.
    trade_candidates.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)));
    let mut best_trade: Option<(AgentId, f32, f32)> = None;
    for &(_, id, d) in trade_candidates.iter().take(TRADE_CANDIDATES) {
        let Some(other) = env.agents.get(id) else { continue };
        let steps = meeting_steps(me.skills.get(Skill::Trading), other.skills.get(Skill::Trading));
        let gain = expected_gain(
            Trader { holdings: &me.inventory, weights: &weights },
            Trader { holdings: &other.inventory, weights: &other.demand_weights() },
            steps,
        );
        let value = (1.0 - (-gain / GAIN_SCALE).exp()) as f32;
        if value >= MIN_TRADE_VALUE
            && best_trade.is_none_or(|(_, bv, bd)| value / (1.0 + 0.05 * d) > bv / (1.0 + 0.05 * bd))
        {
            best_trade = Some((id, value, d));
        }
    }
    if let Some((id, value, d)) = best_trade {
        let score = ActionScore::evaluate(
            1.4,
            &[lin(value), lin(me.prefs.trade), lin(me.skills.get(Skill::Trading)).weighted(0.3), near(d, vision)],
        );
        o.push(Activity::Trade, Target::Agent(id), score);
    } else if env.rules.trade_enabled
        && me.is_adult()
        && let Some(market) = me.memory.market
    {
        // No partner worth meeting in sight: carry goods for sale to the market place.
        let targets = me.sale_targets();
        let for_sale: f32 = Good::ALL.iter().map(|&g| me.inventory.get(g).min(targets.get(g))).sum();
        let d = me.kin.position.distance(env.world.center(market));
        if for_sale >= 2.0 && d > REACH {
            let score =
                ActionScore::evaluate(1.2, &[lin(me.prefs.trade), lin(for_sale / (for_sale + 4.0)), near(d, vision)]);
            o.push(Activity::Trade, Target::Tile(market), score);
        }
    }
    if let Some((id, _, d)) = best_friend {
        let score = ActionScore::evaluate(
            1.0,
            &[Consideration::new(me.needs.social.value, &LONELY), lin(coop).weighted(0.6), near(d, vision)],
        );
        o.push(Activity::Socialize, Target::Agent(id), score);
    }
    if let Some((id, their_hunger, d)) = neediest {
        let score = ActionScore::evaluate(
            1.6,
            &[lin(coop), lin(their_hunger), lin((surplus / 3.0).min(1.0)).weighted(0.5), near(d, vision)],
        );
        o.push(Activity::Share, Target::Agent(id), score);
    }
    // Food above what the agent keeps to eat and to sell can be put away.
    let extra = food - me.food_to_carry();
    if extra >= 2.0 {
        let surplus = lin((extra / 8.0).min(1.0));
        // Pooling in the group store is driven by cooperation.
        if let Some((b, spot)) = s.storage {
            let score = ActionScore::evaluate(1.0, &[lin(coop), surplus, near(spot.dist, vision)]);
            o.push(Activity::Store, Target::Building(b), score);
        }
        // Putting surplus away at home frees carrying capacity for the
        // materials of building plans and for harvests.
        if let Some((b, spot)) = s.home_store {
            let crowding = 1.0 - me.free_capacity() / CARRY_CAPACITY;
            let score = ActionScore::evaluate(1.0, &[surplus, lin(crowding), near(spot.dist, vision)]);
            o.push(Activity::Store, Target::Building(b), score);
        }
    }
    if let Some((id, value, d)) = victim
        && hunger > 0.55
        && food < 0.5
    {
        // Theft is tempting when a victim is closer than any known food:
        // convenience = d_food / (d_food + d_victim) (1 without known food).
        let convenience = s.food.map_or(1.0, |f| f.dist / (f.dist + d).max(f32::EPSILON));
        let score = ActionScore::evaluate(
            2.6,
            &[
                Consideration::new(hunger, &DESPERATE),
                lin(1.0 - coop),
                lin(me.prefs.risk).weighted(0.7),
                lin(value).weighted(0.5),
                lin(convenience),
            ],
        );
        o.push(Activity::Steal, Target::Agent(id), score);
    }
    if let Some((id, affinity, d)) = partner {
        let score = ActionScore::evaluate(1.2, &[lin(affinity), lin(me.energy), near(d, vision)]);
        o.push(Activity::Mate, Target::Agent(id), score);
    }
}
