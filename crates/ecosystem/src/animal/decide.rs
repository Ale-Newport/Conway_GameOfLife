//! Utility-AI decisions with species-specific considerations, and the
//! steering that executes them.
//!
//! Each candidate action is scored with `organism-engine`'s compensated
//! product of considerations times the species' action weight:
//!
//! | Action | Considerations (curve) | Available when |
//! |---|---|---|
//! | forage | hunger (species curve), food availability | plant-like food in the diet and perceived |
//! | hunt | hunger, prey value, stamina (½ weight) | prey perceived |
//! | scavenge | hunger, proximity | a carcass it eats is perceived (×2 for its own kill), or a clutch of another species (nest predators; weight `max(forage, scavenge) · (½ + ½ egg preference)`) |
//! | flee | threat level − boldness (species fear curve) | predator, fire or deep water (within 3 cells of a fire: to the nearest burned-out, bare or water cell, through the front if need be) |
//! | drink | thirst (species curve), water distance (or 0.5 when searching) | water remembered, or thirst > 0.3 (search downhill) |
//! | rest | satiety, hydration, 1 − stamina | always |
//! | sleep | fatigue (species curve) × circadian factor × safety × (1 − thirst urgency²) | always |
//! | seek mate | condition, mate proximity | ready to mate and in season (without a mate in sight: species spawning in water head for water, others wander) |
//! | wander | constant; with nothing to eat in sight at least `½ max(hunt, forage, scavenge)` × hunger (searching, at cruise speed); pack members at least `cohesion · ramp(leader distance, ¼ vision, vision)` (regrouping) | always |
//! | follow parent | distance to the mother | dependent young |
//! | incubate | 1 − hunger | a clutch to guard |
//! | defend | aggression, intruder closeness to the territory centre, health | an intruder in the territory |
//! | migrate | discomfort / 10 °C, 1 − hunger (½ weight) | a migrant outside its preferred climate |
//! | hibernate | constant 2.5 | not summer, cold day and enough reserves (or already dormant) |
//!
//! Pack members join the hunt of their leader (prey value ≥ 0.8, hunger at
//! half weight) and travel with it; herds, flocks and schools steer with
//! boids cohesion and alignment (see `social`).
//! The current action gets a momentum bonus of 15 %; ties and near-ties are
//! broken by a 5 % multiplicative noise from the animal's own random stream
//! for this tick. Ectotherms below their tolerance are torpid, or dormant
//! (`hibernate`) when the conditions of their hibernation hold.

use organism_engine::steering::{self, Feelers, WanderParams, WanderState};
use organism_engine::{ActionScore, Consideration, Curve, Kinematics, MotionLimits, choose_best};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

use super::genetics::Phenotype;
use super::perceive::{Ctx, Senses, ThreatSource, perceive, ready_to_mate};
use super::{Action, ReproState, Target, seasonal, social};
use crate::environment::passable;
use crate::species::{ActivityPattern, Food, HuntingStyle, PlantLayer};
use crate::units::ramp;

/// Number of scored actions.
pub const ACTION_COUNT: usize = 14;

/// Scored actions in score order.
pub const ACTIONS: [ScoredAction; ACTION_COUNT] = [
    ScoredAction::Forage,
    ScoredAction::Hunt,
    ScoredAction::Scavenge,
    ScoredAction::Flee,
    ScoredAction::Drink,
    ScoredAction::Rest,
    ScoredAction::Sleep,
    ScoredAction::SeekMate,
    ScoredAction::Wander,
    ScoredAction::FollowParent,
    ScoredAction::Incubate,
    ScoredAction::Defend,
    ScoredAction::Migrate,
    ScoredAction::Hibernate,
];

/// The candidates of the utility AI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoredAction {
    /// Graze, browse or forage.
    Forage,
    /// Hunt live prey.
    Hunt,
    /// Eat a carcass.
    Scavenge,
    /// Flee a threat.
    Flee,
    /// Drink.
    Drink,
    /// Rest.
    Rest,
    /// Sleep.
    Sleep,
    /// Seek a mate.
    SeekMate,
    /// Wander.
    Wander,
    /// Follow the mother.
    FollowParent,
    /// Guard a clutch.
    Incubate,
    /// Chase an intruder out of the territory.
    Defend,
    /// Travel towards the preferred climate.
    Migrate,
    /// Stay dormant in the den.
    Hibernate,
}

impl ScoredAction {
    /// Stable machine key.
    pub fn key(self) -> &'static str {
        match self {
            Self::Forage => "forage",
            Self::Hunt => "hunt",
            Self::Scavenge => "scavenge",
            Self::Flee => "flee",
            Self::Drink => "drink",
            Self::Rest => "rest",
            Self::Sleep => "sleep",
            Self::SeekMate => "seek_mate",
            Self::Wander => "wander",
            Self::FollowParent => "follow_parent",
            Self::Incubate => "incubate",
            Self::Defend => "defend",
            Self::Migrate => "migrate",
            Self::Hibernate => "hibernate",
        }
    }
}

/// Momentum bonus of the current action.
const MOMENTUM: f32 = 1.15;
/// Multiplicative noise of `choose_best`.
const NOISE: f32 = 0.05;
/// Consideration value of searching for water whose location is unknown.
const WATER_SEARCH_ODDS: f32 = 0.5;

/// Outcome of a decision.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Decision {
    pub action: Action,
    pub target: Target,
    pub accel: Vec2,
    pub max_speed: f32,
    pub sprint: bool,
    pub scores: [f32; ACTION_COUNT],
    pub wander: WanderState,
    pub hidden: bool,
    /// Alarm call: threat position and strength (calling species fleeing a
    /// predator, strength 1, or relaying an alarm they heard).
    pub alarm: Option<(Vec2, f32)>,
}

/// Circadian sleep pressure multiplier: high in the species' rest window.
pub(crate) fn sleep_window(pattern: ActivityPattern, daylight: f32, hour: f32) -> f32 {
    let resting = match pattern {
        ActivityPattern::Diurnal => daylight <= 0.02,
        ActivityPattern::Nocturnal => daylight > 0.1,
        ActivityPattern::Crepuscular => (10.0..15.0).contains(&hour) || !(3.0..22.0).contains(&hour),
        ActivityPattern::Cathemeral => false,
    };
    if resting { 1.0 } else { 0.15 }
}

fn score(weight: f32, considerations: &[Consideration<'_>]) -> f32 {
    ActionScore::evaluate(weight, considerations).value
}

/// Perceives and decides for the animal at store index `index` (pure).
pub(crate) fn decide(ctx: &Ctx<'_>, index: usize) -> Decision {
    let senses = perceive(ctx, index);
    decide_with(ctx, index, &senses)
}

/// Scores the actions of an animal from its senses (used by [`decide`] and
/// by the inspector).
pub(crate) fn scores(ctx: &Ctx<'_>, index: usize, senses: &Senses) -> [f32; ACTION_COUNT] {
    let me = &ctx.animals.as_slice()[index];
    let s = usize::from(me.species);
    let species = &ctx.catalog.animals()[s];
    let traits = &ctx.traits[s];
    let pheno = Phenotype::of(&me.genome, ctx.schema);
    let b = &species.behaviour;
    let w = &b.weights;
    let hunger = me.hunger(traits);
    let thirst = me.thirst();
    let linear = Curve::Linear;
    let inverse = Curve::InverseLinear;
    let mut out = [0.0f32; ACTION_COUNT];

    if let Some(food) = &senses.food {
        out[0] =
            score(w.forage, &[Consideration::new(hunger, &b.hunger_curve), Consideration::new(food.value, &linear)]);
    }
    if let Some(prey) = senses.prey {
        // Pack members join their leader's hunt even when not very hungry.
        let appetite = if senses.pack_hunt { 0.5 } else { 1.0 };
        out[1] = score(
            w.hunt,
            &[
                Consideration::new(hunger, &b.hunger_curve).weighted(appetite),
                Consideration::new(prey.value, &linear),
                Consideration::new(me.stamina, &linear).weighted(0.5),
            ],
        );
    }
    if let Some((_, _, distance)) = senses.eggs.filter(|e| senses.carcass.is_none_or(|c| e.2 < c.2)) {
        // Raiding a nest: a rich, defenceless food found while foraging,
        // valued by the species' preference for eggs.
        let reach = traits.vision_cells.max(1.0);
        let preference = species.diet_entry(&Food::Eggs).map_or(0.0, |d| d.preference);
        out[2] = score(
            w.forage.max(w.scavenge) * (0.5 + 0.5 * preference),
            &[
                Consideration::new(hunger, &b.hunger_curve),
                Consideration::new(distance / reach, &inverse).weighted(0.5),
            ],
        );
    } else if let Some((id, _, distance)) = senses.carcass {
        let own = ctx.remains.carcass(id).is_some_and(|c| c.killer == Some(ctx.animals.ids()[index]));
        let reach = traits.vision_cells.max(1.0);
        out[2] = score(
            w.scavenge.max(if own { w.hunt } else { 0.0 }) * if own { 2.0 } else { 1.0 },
            &[
                Consideration::new(hunger, &b.hunger_curve),
                Consideration::new(distance / reach, &inverse).weighted(0.5),
            ],
        );
    }
    if let Some(threat) = senses.threat {
        let level = match threat.source {
            ThreatSource::Predator(_) | ThreatSource::Alarm | ThreatSource::Rival(_) => {
                (threat.level - (b.boldness + pheno.boldness).clamp(0.0, 1.0) * 0.5).max(0.0)
            }
            ThreatSource::Fire | ThreatSource::Water => threat.level,
        };
        let weight = match threat.source {
            ThreatSource::Predator(_) | ThreatSource::Alarm | ThreatSource::Rival(_) => w.flee,
            // Every animal escapes fire and drowning, whatever its boldness.
            ThreatSource::Fire | ThreatSource::Water => w.flee.max(3.0),
        };
        out[3] = score(weight, &[Consideration::new(level, &b.fear_curve)]);
    }
    // A gravid female that spawns in water heads for it when her eggs are due.
    let spawning = species.habitat.breeds_in_water
        && matches!(&me.repro, ReproState::Gestating(p) if ctx.tick + traits.gestation_ticks as u64 / 4 >= p.due);
    let thirst = if spawning { thirst.max(0.8) } else { thirst };
    if let Some((_, distance)) = senses.water {
        let memory = traits.water_memory_cells;
        out[4] = score(
            w.drink,
            &[
                Consideration::new(thirst, &b.thirst_curve),
                Consideration::new(distance / memory, &inverse).weighted(0.3),
            ],
        );
    } else if senses.water_search.is_some() {
        // Searching for unknown water: as urgent as thirst, discounted for
        // the uncertainty of finding it.
        out[4] = score(
            w.drink,
            &[Consideration::new(thirst, &b.thirst_curve), Consideration::new(WATER_SEARCH_ODDS, &linear)],
        );
    }
    out[5] = score(
        w.rest,
        &[
            Consideration::new(1.0 - hunger, &linear),
            Consideration::new(1.0 - thirst, &linear),
            Consideration::new(1.0 - me.stamina, &linear).weighted(0.5),
        ],
    );
    let safety = 1.0 - senses.threat.map_or(0.0, |t| t.level);
    // Acute thirst keeps an animal awake (dehydration kills within a day).
    let urgency = b.thirst_curve.evaluate(thirst);
    out[6] = score(
        w.sleep,
        &[
            Consideration::new(me.fatigue, &b.fatigue_curve),
            Consideration::new(sleep_window(b.activity, ctx.daylight, ctx.hour), &linear),
            Consideration::new(safety, &linear),
            Consideration::new(1.0 - urgency * urgency, &linear),
        ],
    );
    if ready_to_mate(ctx.animals.ids()[index], me, ctx) {
        let proximity = senses.mate.map_or(0.3, |(_, _, d)| 1.0 - (d / traits.vision_cells.max(1.0)).min(0.7));
        out[7] = score(
            w.mate,
            &[Consideration::new(1.0 - hunger, &linear).weighted(0.5), Consideration::new(proximity, &linear)],
        );
    }
    out[8] = w.wander;
    if searching(senses) {
        // Nothing to eat in sight: a hungry animal goes searching. The
        // wander score rises with hunger to half the weight of its main way
        // of feeding, so a hungry wolf pack prowls instead of resting.
        let weight = 0.5 * w.hunt.max(w.forage).max(w.scavenge);
        out[8] = out[8].max(score(weight, &[Consideration::new(hunger, &b.hunger_curve)]));
    }
    if let Some((_, d)) = senses.leader {
        // Pack members that have strayed from the alpha regroup, so the
        // pack travels and rests together.
        let (near, far) = social::regroup_range(traits);
        out[8] = out[8].max(species.social.cohesion * ramp(d, near, far));
    }
    let dependent = matches!(me.repro, ReproState::Juvenile)
        && (me.age_ticks(ctx.tick) as f64) < traits.care_ticks
        && senses.mother.is_some();
    if let Some((_, d)) = senses.mother.filter(|_| dependent) {
        out[9] = 2.0 * (d / 3.0).min(1.0);
    }
    if senses.clutch.is_some() {
        out[10] = score(1.5, &[Consideration::new(1.0 - hunger, &linear)]);
    }
    if let (Some((_, p, _)), Some(home)) = (senses.intruder, me.home) {
        let closeness = 1.0 - (home.distance(p) / traits.territory_cells.max(1e-3)).min(1.0);
        out[11] = score(
            w.defend,
            &[
                Consideration::new(species.combat.aggression, &linear),
                Consideration::new(closeness, &linear),
                Consideration::new(me.health * (1.0 - me.injury), &linear),
            ],
        );
    }
    if let Some((_, discomfort)) = senses.migration {
        out[12] = score(
            w.migrate,
            &[
                Consideration::new((discomfort / 10.0).min(1.0), &linear),
                Consideration::new(1.0 - hunger, &linear).weighted(0.5),
            ],
        );
    }
    if senses.dormancy {
        out[13] = seasonal::HIBERNATION_SCORE;
    }
    out
}

/// Chooses the action and computes the steering for given senses.
pub(crate) fn decide_with(ctx: &Ctx<'_>, index: usize, senses: &Senses) -> Decision {
    let me = &ctx.animals.as_slice()[index];
    let id = ctx.animals.ids()[index];
    let s = usize::from(me.species);
    let species = &ctx.catalog.animals()[s];
    let traits = &ctx.traits[s];
    let pheno = Phenotype::of(&me.genome, ctx.schema);
    let mut rng = crate::rng::stream(ctx.seed, crate::rng::DECIDE, ctx.tick, id.get());
    let raw = scores(ctx, index, senses);
    let mut weighted = raw;
    if me.stuck_ticks < 3
        && let Some(i) = ACTIONS.iter().position(|a| matches_current(*a, me.action))
    {
        weighted[i] *= MOMENTUM;
    }
    let speed_factor = pheno.speed * (0.5 + 0.5 * me.health) * me.impairment() * (1.0 - 0.5 * ctx.epi.virulence(me));
    let sprint_speed = traits.sprint_cells
        * speed_factor
        * if me.stamina > 0.15 { 1.0 } else { traits.cruise_cells / traits.sprint_cells };
    let cruise = traits.cruise_cells * speed_factor;
    let limits = |speed: f32| MotionLimits::new(speed, traits.accel_cells * pheno.speed);
    let mut wander = me.wander;
    let torpid = !traits.endotherm && {
        let t = crate::environment::felt_temperature(ctx.world, traits, ctx.cell_of(me.kin.position));
        t < species.climate.tolerance_c.0 + pheno.thermal_shift
    };
    // A torpid ectotherm that meets the conditions of dormancy goes into
    // brumation instead of sitting exposed in the cold.
    let choice = if torpid {
        senses.dormancy.then(|| ACTIONS.iter().position(|a| *a == ScoredAction::Hibernate)).flatten()
    } else {
        choose_best(&weighted, Some(&mut rng), NOISE)
    };
    let pos = me.kin.position;
    let kin = &me.kin;
    // Territorial adults keep to their territory: beyond its radius the
    // exploratory steering is pulled back towards the territory centre.
    let territory = |speed: f32| match me.home {
        Some(home) if traits.territory_cells > 0.0 && me.is_adult() => {
            let r = traits.territory_cells;
            steering::seek(kin, home, limits(speed)) * ramp(home.distance(pos), r, 2.0 * r)
        }
        _ => Vec2::ZERO,
    };
    let (action, target, desired, max_speed, sprint, mut hidden) = match choice.map(|i| ACTIONS[i]) {
        None if torpid => (Action::Torpor, Target::None, brake(kin, cruise), cruise, false, me.hidden),
        Some(ScoredAction::Flee) => {
            let threat = senses.threat.unwrap_or(super::perceive::Threat {
                source: ThreatSource::Fire,
                position: pos,
                velocity: Vec2::ZERO,
                distance: 0.0,
                level: 0.0,
            });
            let threat_kin = Kinematics { position: threat.position, velocity: threat.velocity, heading: 0.0 };
            let refuge = senses.refuge.filter(|(p, d)| {
                *d < threat.distance.max(1.0) * 2.0 && (*p - pos).dot(pos - threat.position) > -0.2 * d * d
            });
            // From a fire, burrowers dive into a nearby burrow (the fire
            // passes over it); others, close to the fire, run to safe ground
            // (burned out, bare or water), even behind the front: they dash
            // through the flames instead of being cornered ahead of it.
            let fire_escape = if threat.source == ThreatSource::Fire {
                match me.home.filter(|h| traits.burrows && h.distance(pos) <= 2.0 * sprint_speed.max(0.5)) {
                    Some(h) => Some((h, h.distance(pos), true)),
                    None => senses.fire_refuge.map(|(p, d)| (p, d, false)),
                }
            } else {
                None
            };
            match fire_escape {
                Some((p, d, true)) if d < 0.75 => {
                    (Action::Flee, Target::Point(p), brake(kin, sprint_speed), sprint_speed, false, true)
                }
                Some((_, d, false)) if d < 0.5 => {
                    (Action::Flee, Target::Point(pos), brake(kin, sprint_speed), sprint_speed, false, false)
                }
                Some((p, _, _)) => (
                    Action::Flee,
                    Target::Point(p),
                    steering::seek(kin, p, limits(sprint_speed)),
                    sprint_speed,
                    true,
                    false,
                ),
                None => match refuge {
                    Some((p, d)) if d < 0.75 && traits.burrows && me.home.is_some_and(|h| h.distance(p) < 0.5) => {
                        (Action::Flee, Target::Point(p), brake(kin, sprint_speed), sprint_speed, false, true)
                    }
                    Some((p, _)) => (
                        Action::Flee,
                        Target::Point(p),
                        steering::seek(kin, p, limits(sprint_speed)),
                        sprint_speed,
                        true,
                        false,
                    ),
                    None => {
                        let target = match threat.source {
                            ThreatSource::Predator(pid) | ThreatSource::Rival(pid) => Target::Animal(pid),
                            _ => Target::Point(threat.position),
                        };
                        // A distant fire is walked away from at cruising
                        // speed: stamina is kept for the dash through the front.
                        let calm =
                            threat.source == ThreatSource::Fire && threat.distance > super::hazards::FIRE_ESCAPE_CELLS;
                        let speed = if calm { cruise } else { sprint_speed };
                        let accel = steering::evade(kin, &threat_kin, 2.0, limits(speed));
                        // Herd animals bias their escape towards the herd (selfish herd).
                        let accel = match senses.herd {
                            Some(h) if species.social.cohesion > 0.0 => {
                                accel + steering::seek(kin, h.centroid, limits(speed)) * 0.3 * species.social.cohesion
                            }
                            _ => accel,
                        };
                        (Action::Flee, target, accel, speed, !calm, false)
                    }
                },
            }
        }
        Some(ScoredAction::Hunt) => {
            let prey = senses.prey.map_or((Target::None, pos, Vec2::ZERO, 0.0), |p| {
                (Target::Animal(p.id), p.position, p.velocity, p.distance)
            });
            let (target, prey_pos, prey_vel, distance) = prey;
            let quarry = Kinematics { position: prey_pos, velocity: prey_vel, heading: 0.0 };
            let (speed, sprinting) = match species.behaviour.hunting {
                HuntingStyle::Ambush if distance > 3.0 * traits.strike_cells => (0.0, false),
                HuntingStyle::Stalk if distance > 4.0 * traits.strike_cells => (0.5 * cruise, false),
                HuntingStyle::Coursing if distance > traits.vision_cells * 0.6 => (cruise, false),
                _ => (sprint_speed, true),
            };
            let accel =
                if speed > 0.0 { steering::pursue(kin, &quarry, 2.0, limits(speed)) } else { brake(kin, cruise) };
            (Action::Hunt, target, accel, speed.max(0.01), sprinting, false)
        }
        Some(ScoredAction::Scavenge) => match senses.eggs.filter(|e| senses.carcass.is_none_or(|c| e.2 < c.2)) {
            Some((cid, p, _)) => {
                (Action::Feed, Target::Clutch(cid), steering::arrive(kin, p, 0.5, limits(cruise)), cruise, false, false)
            }
            None => {
                let (id, p, _) = senses.carcass.unwrap_or((0, pos, 0.0));
                (Action::Feed, Target::Carcass(id), steering::arrive(kin, p, 1.0, limits(cruise)), cruise, false, false)
            }
        },
        Some(ScoredAction::Forage) => {
            let food = senses.food.as_ref();
            let (cell, p) = food.map_or((0, pos), |f| (f.cell, f.position));
            let action = match food.map(|f| &species.diet[f.entry].food) {
                Some(Food::Plant(PlantLayer::Shrubs)) | Some(Food::Browse) => Action::Browse,
                Some(Food::Plant(_)) => Action::Graze,
                _ => Action::Forage,
            };
            let forage_speed = 0.4 * cruise;
            let accel = if p.distance(pos) < 0.6 {
                steering::wander(
                    kin,
                    &mut wander,
                    &WanderParams { distance: 1.0, radius: 0.5, jitter: 0.6 },
                    &mut rng,
                    limits(0.1 * cruise),
                )
            } else {
                steering::arrive(kin, p, 1.0, limits(forage_speed.max(0.05)))
            };
            // Herds, flocks and schools keep together while they feed.
            let accel = accel + 0.5 * social::boids(kin, senses, species, limits(forage_speed.max(0.05)));
            (action, Target::Cell(cell), accel, forage_speed.max(0.05), false, false)
        }
        Some(ScoredAction::Drink) => match senses.water {
            Some((p, _)) => {
                (Action::Drink, Target::Point(p), steering::arrive(kin, p, 1.0, limits(cruise)), cruise, false, false)
            }
            None => {
                let p = senses.water_search.unwrap_or(pos);
                (Action::Drink, Target::Point(p), steering::seek(kin, p, limits(cruise)), cruise, false, false)
            }
        },
        Some(ScoredAction::Rest) | Some(ScoredAction::Sleep) => {
            let action =
                if choice.map(|i| ACTIONS[i]) == Some(ScoredAction::Sleep) { Action::Sleep } else { Action::Rest };
            match me.home.filter(|h| traits.burrows && h.distance(pos) < 2.0 * traits.vision_cells.max(4.0)) {
                Some(home) if home.distance(pos) > 0.5 => (
                    action,
                    Target::Point(home),
                    steering::arrive(kin, home, 1.0, limits(cruise)),
                    cruise,
                    false,
                    false,
                ),
                Some(home) => (action, Target::Point(home), brake(kin, cruise), cruise, false, true),
                None => (action, Target::None, brake(kin, cruise), cruise, false, false),
            }
        }
        Some(ScoredAction::SeekMate) => match senses.mate {
            Some((mid, p, _)) => (
                Action::SeekMate,
                Target::Animal(mid),
                steering::arrive(kin, p, 0.5, limits(cruise)),
                cruise,
                false,
                false,
            ),
            // Species that spawn in water gather at the nearest water to
            // find mates (frog choruses at breeding ponds).
            None => match senses.water.filter(|(_, d)| species.habitat.breeds_in_water && *d > 0.5) {
                Some((p, _)) => (
                    Action::SeekMate,
                    Target::Point(p),
                    steering::arrive(kin, p, 1.0, limits(cruise)),
                    cruise,
                    false,
                    false,
                ),
                None => {
                    let a = steering::wander(kin, &mut wander, &WanderParams::default(), &mut rng, limits(cruise))
                        + territory(cruise);
                    (Action::SeekMate, Target::None, a, cruise, false, false)
                }
            },
        },
        Some(ScoredAction::FollowParent) => {
            let p = senses.mother.map_or(pos, |(p, _)| p);
            let mother = me.lineage.parents[0].map_or(Target::None, Target::Animal);
            (Action::FollowParent, mother, steering::arrive(kin, p, 1.5, limits(cruise)), cruise, false, false)
        }
        Some(ScoredAction::Incubate) => {
            let (cid, p, _) = senses.clutch.unwrap_or((0, pos, 0.0));
            (Action::Incubate, Target::Clutch(cid), steering::arrive(kin, p, 0.5, limits(cruise)), cruise, false, false)
        }
        Some(ScoredAction::Defend) => {
            let (iid, p, _) = senses.intruder.unwrap_or((id, pos, 0.0));
            let speed = 0.8 * sprint_speed;
            let quarry = Kinematics { position: p, velocity: Vec2::ZERO, heading: 0.0 };
            (
                Action::Defend,
                Target::Animal(iid),
                steering::pursue(kin, &quarry, 2.0, limits(speed)),
                speed,
                true,
                false,
            )
        }
        Some(ScoredAction::Migrate) => {
            let p = senses.migration.map_or(pos, |(p, _)| p);
            let accel = steering::seek(kin, p, limits(cruise)) + social::boids(kin, senses, species, limits(cruise));
            (Action::Migrate, Target::Point(p), accel, cruise, false, false)
        }
        Some(ScoredAction::Hibernate) => (Action::Hibernate, Target::None, brake(kin, cruise), cruise, false, true),
        Some(ScoredAction::Wander) | None => {
            let own = if senses.flock.is_empty() { 1.0 } else { 1.0 - 0.5 * species.social.alignment_weight() };
            let mut accel =
                steering::wander(kin, &mut wander, &WanderParams::default(), &mut rng, limits(0.5 * cruise)) * own;
            accel += social::boids(kin, senses, species, limits(0.5 * cruise));
            accel += social::follow_leader(kin, senses, limits(cruise));
            accel += territory(0.5 * cruise);
            // Regrouping pack members and hungry animals searching for
            // food travel at cruise speed.
            let regrouping = senses.leader.is_some_and(|(_, d)| d > social::regroup_range(traits).0);
            let hungry_search = searching(senses) && raw[8] > species.behaviour.weights.wander;
            let speed = if regrouping || hungry_search { cruise } else { 0.5 * cruise };
            (Action::Wander, Target::None, accel, speed, false, false)
        }
    };
    if !matches!(action, Action::Rest | Action::Sleep | Action::Flee | Action::Hibernate) {
        hidden = false;
    }
    let alarm = match (action, senses.threat) {
        (Action::Flee, Some(t)) if species.social.alarm_calls => match t.source {
            ThreatSource::Predator(_) => Some((t.position, 1.0)),
            ThreatSource::Alarm if t.level >= social::MIN_RELAY => Some((t.position, t.level)),
            _ => None,
        },
        _ => None,
    };
    // Collision and terrain avoidance first, then the action, then separation.
    let lim = limits(max_speed);
    let avoid = steering::avoid_obstacles(
        kin,
        &Feelers { length: (2.0 * max_speed).max(1.0), spread: 0.6, samples: 4 },
        lim,
        |p| passable(ctx.world, traits, p),
    );
    let separation = if senses.crowd.is_empty() {
        Vec2::ZERO
    } else {
        let crowd: Vec<Kinematics> = senses.crowd.iter().map(|p| Kinematics::new(*p, 0.0)).collect();
        steering::separation(kin, crowd.iter(), 1.0, lim)
    };
    // Blocked for several ticks (cliff, lake shore): add a random escape
    // direction so the animal can get around the obstacle.
    let escape = if me.stuck_ticks >= 3 { rng.unit_vec2() * lim.max_accel } else { Vec2::ZERO };
    let accel =
        steering::combine_prioritized(&[(avoid, 1.0), (escape, 1.0), (desired, 1.0), (separation, 0.3)], lim.max_accel);
    Decision { action, target, accel, max_speed, sprint, scores: raw, wander, hidden, alarm }
}

/// Whether the animal perceives nothing to eat (no prey, forage, carcass or
/// clutch), so that hunger drives it to search.
fn searching(senses: &Senses) -> bool {
    senses.prey.is_none() && senses.food.is_none() && senses.carcass.is_none() && senses.eggs.is_none()
}

/// Whether a scored action corresponds to the current action (momentum).
fn matches_current(scored: ScoredAction, action: Action) -> bool {
    match scored {
        ScoredAction::Forage => matches!(action, Action::Graze | Action::Browse | Action::Forage),
        ScoredAction::Hunt => action == Action::Hunt,
        ScoredAction::Scavenge => action == Action::Feed,
        ScoredAction::Flee => action == Action::Flee,
        ScoredAction::Drink => action == Action::Drink,
        ScoredAction::Rest => action == Action::Rest,
        ScoredAction::Sleep => action == Action::Sleep,
        ScoredAction::SeekMate => action == Action::SeekMate,
        ScoredAction::Wander => action == Action::Wander,
        ScoredAction::FollowParent => action == Action::FollowParent,
        ScoredAction::Incubate => action == Action::Incubate,
        ScoredAction::Defend => action == Action::Defend,
        ScoredAction::Migrate => action == Action::Migrate,
        ScoredAction::Hibernate => action == Action::Hibernate,
    }
}

/// Acceleration that stops the animal.
fn brake(kin: &Kinematics, speed: f32) -> Vec2 {
    steering::arrive(kin, kin.position, 1.0, MotionLimits::new(speed, speed.max(0.01) * 4.0))
}
