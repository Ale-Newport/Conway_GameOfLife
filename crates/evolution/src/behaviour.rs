//! The decision phase: every organism chooses an acceleration and an intent
//! from a read-only view of the world. Decisions are pure functions of that
//! view and of a per-organism random stream, so they run in parallel and are
//! deterministic.
//!
//! **Prey**, in priority order:
//! 1. *Flee* — if a predator in sight is within the flee distance
//!    `min(r, r (1 − boldness + ½ experience))` (`r` = perception radius):
//!    evade the nearest one at full speed.
//! 2. *Court* — sexual mode, ready to breed: approach the nearest ready,
//!    mutually acceptable mate in sight; stop when in contact.
//! 3. *Graze* — if the current cell holds at least half a full bite: brake and eat.
//! 4. *Forage* — march 8 rays (random rotation) cell by cell up to `r`,
//!    stopping at impassable cells; seek the point maximising
//!    `food / (1 + 0.15 d)` if it beats the current cell.
//! 5. *Wander* — Reynolds wander at cruising speed.
//!
//! Cruising speed is `min(v_max, cruise_speed)`: searching, grazing and
//! courting do not depend on the speed gene, which only matters for fleeing
//! and hunting. Acceleration is limited to `agility · v_max`.
//!
//! **Predators**: hunt (pursue the nearest prey in sight) while energy is
//! below `(0.3 + 0.6 boldness) · E_max` and no kill is being handled;
//! otherwise court (sexual mode, ready) or rest (wander at 60 % of cruising
//! speed). A hungry predator without prey in sight wanders at cruising speed.
//! *Interference* (Beddington–DeAngelis): a hungry predator with `n` rivals
//! within 3 units loses the tick to a contest with probability
//! `1 − e^{−c n}` (`c` = `interference`), so per-capita kill rates fall at
//! high predator density.
//!
//! Sight requires a passable straight line (sampled every cell). Whenever the
//! world contains obstacles, sampled feelers steer away from them first
//! (prioritised blending).

use std::f32::consts::FRAC_PI_4;

use organism_engine::perception::{self, Query};
use organism_engine::steering::{
    self, Feelers, WanderParams, WanderState, avoid_obstacles, combine_prioritized, evade, pursue,
};
use organism_engine::{AgentId, GenomeSchema, MotionLimits, genetic_distance};
use simulation_core::glam::Vec2;
use simulation_core::{SimRng, SpatialGrid};

use crate::config::{EvolutionConfig, ReproductionMode};
use crate::environment::Environment;
use crate::genes::Phenotype;
use crate::organism::{Activity, Organism, Species, intake_capacity, storage};

/// Prediction horizon (ticks) of pursuit and evasion.
const PREDICTION: f32 = 2.0;
/// Distance discount of the foraging score.
const DISTANCE_DISCOUNT: f32 = 0.15;
/// Rays marched when searching food.
const RAYS: usize = 8;
/// Distance within which predators interfere with each other's hunting.
pub const INTERFERENCE_RADIUS: f32 = 3.0;

/// The outcome of one organism's decision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Decision {
    /// Desired acceleration.
    pub accel: Vec2,
    /// Speed limit for this tick (cruising or full speed).
    pub speed_cap: f32,
    /// Updated wander state.
    pub wander: WanderState,
    /// Chosen behaviour.
    pub activity: Activity,
    /// Prey hunted, predator fled from or mate courted.
    pub target: Option<AgentId>,
    /// Whether the organism escaped a predator this tick.
    pub threatened: bool,
}

/// Read-only world view shared by all decisions of a tick.
pub(crate) struct Senses<'a> {
    pub env: &'a Environment,
    pub config: &'a EvolutionConfig,
    pub schema: &'a GenomeSchema,
    pub ids: &'a [AgentId],
    pub agents: &'a [Organism],
    pub phenotypes: &'a [Phenotype],
    pub ready: &'a [bool],
    pub prey_grid: &'a SpatialGrid,
    pub predator_grid: &'a SpatialGrid,
}

impl Senses<'_> {
    fn grid(&self, species: Species) -> &SpatialGrid {
        match species {
            Species::Prey => self.prey_grid,
            Species::Predator => self.predator_grid,
        }
    }

    /// Whether a straight passable line joins `a` and `b`.
    fn in_sight(&self, a: Vec2, b: Vec2) -> bool {
        !self.env.has_obstacles() || perception::line_of_sight(a, b, 1.0, |q| self.env.is_passable(q))
    }

    /// Whether `chooser` accepts `candidate` as a mate: the candidate's
    /// condition reaches `threshold + choosiness (1 − threshold)` and their
    /// genetic distance does not exceed the mating limit.
    pub(crate) fn accepts(&self, chooser: usize, candidate: usize) -> bool {
        mate_acceptable(
            self.config,
            self.schema,
            (&self.agents[chooser], &self.phenotypes[chooser]),
            (&self.agents[candidate], &self.phenotypes[candidate]),
        )
    }
}

/// Mate acceptance rule shared by the decision and the mating phases.
pub(crate) fn mate_acceptable(
    config: &EvolutionConfig,
    schema: &GenomeSchema,
    chooser: (&Organism, &Phenotype),
    candidate: (&Organism, &Phenotype),
) -> bool {
    let threshold = config.reproduction.threshold;
    let demanded = threshold + chooser.1.choosiness * (1.0 - threshold);
    if candidate.0.species != chooser.0.species || candidate.0.condition(candidate.1, &config.physiology) < demanded {
        return false;
    }
    let limit = config.reproduction.mate_distance_limit;
    limit >= 1.0 || genetic_distance(&chooser.0.genome, &candidate.0.genome, schema).is_ok_and(|d| d <= limit)
}

/// Decides for the organism in store slot `slot`.
pub(crate) fn decide(s: &Senses, slot: usize, mut rng: SimRng) -> Decision {
    let me = &s.agents[slot];
    let p = &s.phenotypes[slot];
    let phys = &s.config.physiology;
    let accel = p.speed * phys.agility;
    let full = MotionLimits::new(p.speed, accel);
    let cruise = MotionLimits::new(p.speed.min(phys.cruise_speed), accel);
    let mut wander = me.wander;
    let avoid = |limits: MotionLimits| {
        if s.env.has_obstacles() {
            avoid_obstacles(&me.kinematics, &Feelers::default(), limits, |q| s.env.is_passable(q))
        } else {
            Vec2::ZERO
        }
    };
    let decision = |steer: Vec2, limits: MotionLimits, activity, target, wander, threatened| Decision {
        accel: combine_prioritized(&[(avoid(limits), 1.0), (steer, 1.0)], limits.max_accel),
        speed_cap: limits.max_speed,
        wander,
        activity,
        target,
        threatened,
    };
    let pos = me.kinematics.position;
    match me.species {
        Species::Prey => {
            let flee_radius = (p.perception * (1.0 - p.boldness + 0.5 * p.experience)).min(p.perception);
            let query = Query::new(pos, flee_radius);
            if let Some(threat) = perception::nearest(s.predator_grid, &query, |_, q| s.in_sight(pos, q)) {
                let predator = &s.agents[threat.slot as usize];
                let steer = evade(&me.kinematics, &predator.kinematics, PREDICTION, full);
                let target = Some(s.ids[threat.slot as usize]);
                return decision(steer, full, Activity::Flee, target, wander, true);
            }
            if let Some((steer, target)) = court(s, slot, p, cruise) {
                return decision(steer, cruise, Activity::Court, Some(target), wander, false);
            }
            let bite = intake_capacity(p, phys);
            let here = s.env.food_at(pos);
            if here >= 0.5 * bite {
                let steer = steering::arrive(&me.kinematics, pos, 1.0, cruise);
                return decision(steer, cruise, Activity::Graze, None, wander, false);
            }
            if let Some(spot) = best_food(s, pos, p.perception, here, &mut rng) {
                let steer = steering::seek(&me.kinematics, spot, cruise);
                return decision(steer, cruise, Activity::Forage, None, wander, false);
            }
            let steer = steering::wander(&me.kinematics, &mut wander, &WanderParams::default(), &mut rng, cruise);
            decision(steer, cruise, Activity::Wander, None, wander, false)
        }
        Species::Predator => {
            let hungry = me.handling == 0 && me.energy < (0.3 + 0.6 * p.boldness) * storage(p, phys);
            let interference = s.config.predators.interference;
            if hungry && interference > 0.0 {
                let rivals = perception::count(s.predator_grid, &Query::new(pos, INTERFERENCE_RADIUS), |other, _| {
                    other as usize != slot
                });
                if rivals > 0 && rng.chance(1.0 - (-f64::from(interference) * rivals as f64).exp()) {
                    let limits = MotionLimits::new(cruise.max_speed * 0.6, accel);
                    let steer =
                        steering::wander(&me.kinematics, &mut wander, &WanderParams::default(), &mut rng, limits);
                    return decision(steer, limits, Activity::Contest, None, wander, false);
                }
            }
            if hungry {
                let query = Query::new(pos, p.perception);
                let quarry = perception::nearest(s.prey_grid, &query, |_, q| s.in_sight(pos, q));
                if let Some(quarry) = quarry {
                    let prey = &s.agents[quarry.slot as usize];
                    let steer = pursue(&me.kinematics, &prey.kinematics, PREDICTION, full);
                    return decision(steer, full, Activity::Hunt, Some(s.ids[quarry.slot as usize]), wander, false);
                }
            } else if let Some((steer, target)) = court(s, slot, p, cruise) {
                return decision(steer, cruise, Activity::Court, Some(target), wander, false);
            }
            let (limits, activity) = if hungry {
                (cruise, Activity::Wander)
            } else {
                (MotionLimits::new(cruise.max_speed * 0.6, accel), Activity::Rest)
            };
            let steer = steering::wander(&me.kinematics, &mut wander, &WanderParams::default(), &mut rng, limits);
            decision(steer, limits, activity, None, wander, false)
        }
    }
}

/// Mate search (sexual mode, organism ready): nearest ready, mutually
/// acceptable partner of the same species in sight. Returns the steering
/// (brake when in contact, otherwise seek) and the partner.
fn court(s: &Senses, slot: usize, p: &Phenotype, cruise: MotionLimits) -> Option<(Vec2, AgentId)> {
    if s.config.reproduction.mode != ReproductionMode::Sexual || !s.ready[slot] {
        return None;
    }
    let me = &s.agents[slot];
    let pos = me.kinematics.position;
    let query = Query::new(pos, p.perception);
    let mate = perception::nearest(s.grid(me.species), &query, |other, q| {
        let other = other as usize;
        other != slot && s.ready[other] && s.accepts(slot, other) && s.accepts(other, slot) && s.in_sight(pos, q)
    })?;
    let steer = if mate.distance <= s.config.reproduction.mate_radius {
        steering::arrive(&me.kinematics, pos, 1.0, cruise)
    } else {
        steering::seek(&me.kinematics, mate.position, cruise)
    };
    Some((steer, s.ids[mate.slot as usize]))
}

/// Ray-marching food search; returns the best point if it beats `here`.
fn best_food(s: &Senses, pos: Vec2, radius: f32, here: f32, rng: &mut SimRng) -> Option<Vec2> {
    let offset = rng.f32() * FRAC_PI_4;
    let steps = radius.floor().max(1.0) as u32;
    let mut best: Option<(f32, Vec2)> = None;
    for k in 0..RAYS {
        let dir = Vec2::from_angle(offset + k as f32 * FRAC_PI_4);
        for step in 1..=steps {
            let d = step as f32;
            let point = pos + dir * d;
            if !s.env.is_passable(point) {
                break;
            }
            let score = s.env.food_at(point) / (1.0 + DISTANCE_DISCOUNT * d);
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, point));
            }
        }
    }
    best.filter(|(score, _)| *score > here).map(|(_, point)| point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Barrier, Shape, Terrain};
    use crate::genes;
    use organism_engine::{Genome, Lineage};

    struct Fixture {
        env: Environment,
        config: EvolutionConfig,
        schema: GenomeSchema,
        agents: Vec<Organism>,
        ids: Vec<AgentId>,
    }

    impl Fixture {
        fn new(config: EvolutionConfig) -> Self {
            let schema = genes::schema(config.mutation.size).unwrap();
            Self { env: Environment::new(&config), config, schema, agents: Vec::new(), ids: Vec::new() }
        }

        fn add(&mut self, species: Species, x: f32, y: f32, energy_fraction: f32) {
            let genome = Genome::defaults(&self.schema);
            let p = Phenotype::express(&genome, &self.schema);
            let id = AgentId(self.agents.len() as u64 + 1);
            let energy = energy_fraction * storage(&p, &self.config.physiology);
            let age = p.maturity;
            self.agents.push(Organism::new(
                species,
                Vec2::new(x, y),
                0.0,
                genome,
                energy,
                age,
                Lineage::founder(id, 0),
                id,
            ));
            self.ids.push(id);
        }

        fn decide(&self, slot: usize) -> Decision {
            let phenotypes: Vec<Phenotype> = self.agents.iter().map(|a| a.phenotype(&self.schema)).collect();
            let ready: Vec<bool> = vec![true; self.agents.len()];
            let mut prey_grid = SpatialGrid::new(self.env.width() as f32, self.env.height() as f32, 4.0);
            let mut predator_grid = prey_grid.clone();
            let of = |sp: Species| {
                self.agents
                    .iter()
                    .enumerate()
                    .filter(move |(_, a)| a.species == sp)
                    .map(|(i, a)| (i as u32, a.kinematics.position))
            };
            prey_grid.rebuild(of(Species::Prey));
            predator_grid.rebuild(of(Species::Predator));
            let senses = Senses {
                env: &self.env,
                config: &self.config,
                schema: &self.schema,
                ids: &self.ids,
                agents: &self.agents,
                phenotypes: &phenotypes,
                ready: &ready,
                prey_grid: &prey_grid,
                predator_grid: &predator_grid,
            };
            decide(&senses, slot, SimRng::seed_from_u64(1))
        }
    }

    fn small() -> EvolutionConfig {
        let mut c = EvolutionConfig::default();
        c.world.width = 30;
        c.world.height = 30;
        c
    }

    #[test]
    fn prey_flee_from_close_predators_and_ignore_far_ones() {
        let mut f = Fixture::new(small());
        f.add(Species::Prey, 10.0, 10.0, 0.5);
        f.add(Species::Predator, 12.0, 10.0, 0.1);
        let d = f.decide(0);
        assert_eq!(d.activity, Activity::Flee);
        assert!(d.threatened && d.accel.x < 0.0, "accelerates away: {:?}", d.accel);
        // Default flee distance is 6 × (1 − 0.5) = 3 units.
        f.agents[1].kinematics.position = Vec2::new(14.0, 10.0);
        assert_ne!(f.decide(0).activity, Activity::Flee);
    }

    #[test]
    fn hungry_predators_hunt_and_satiated_ones_rest() {
        let mut f = Fixture::new(small());
        f.add(Species::Prey, 10.0, 10.0, 0.5);
        f.add(Species::Predator, 14.0, 10.0, 0.1);
        let d = f.decide(1);
        assert_eq!((d.activity, d.target), (Activity::Hunt, Some(AgentId(1))));
        assert!(d.accel.x < 0.0);
        f.agents[1].energy = storage(&f.agents[1].phenotype(&f.schema), &f.config.physiology);
        assert_eq!(f.decide(1).activity, Activity::Rest);
    }

    #[test]
    fn prey_graze_on_food_and_forage_towards_it_otherwise() {
        let mut c = small();
        c.food.initial_fraction = 0.0;
        let mut f = Fixture::new(c);
        let rich = f.env.cell_of(Vec2::new(14.5, 10.5)).unwrap();
        // Rays are 45° apart, so the patch must be wider than one cell to be seen 4 cells away.
        f.env.add_food(&Shape::Disc { x: 15.0, y: 10.5, r: 2.5 }, 5.0);
        assert!(f.env.food().as_slice()[rich] > 0.0);
        f.add(Species::Prey, 10.5, 10.5, 0.5);
        let d = f.decide(0);
        assert_eq!(d.activity, Activity::Forage);
        assert!(d.accel.x > 0.0, "{:?}", d.accel);
        f.agents[0].kinematics.position = Vec2::new(14.5, 10.5);
        assert_eq!(f.decide(0).activity, Activity::Graze);
    }

    #[test]
    fn walls_block_sight_of_food_and_prey() {
        let mut c = small();
        c.food.initial_fraction = 0.0;
        c.barriers.push(Barrier {
            name: "wall".into(),
            terrain: Terrain::Mountain,
            shapes: vec![Shape::Rect { x: 12.0, y: 0.0, w: 1.0, h: 30.0 }],
            outside: false,
            active: true,
        });
        let mut f = Fixture::new(c);
        f.env.add_food(&Shape::Rect { x: 14.0, y: 10.0, w: 1.0, h: 1.0 }, 5.0);
        f.add(Species::Prey, 10.5, 10.5, 0.5);
        f.add(Species::Predator, 15.0, 10.5, 0.1);
        assert_eq!(f.decide(0).activity, Activity::Wander, "food behind the wall is not seen");
        assert_ne!(f.decide(1).activity, Activity::Hunt, "prey behind the wall is not seen");
        // A predator 2.5 units away (inside the 3-unit flee distance) but
        // behind the wall is not seen either; on the same side it is.
        f.agents[1].kinematics.position = Vec2::new(13.0, 10.5);
        assert_ne!(f.decide(0).activity, Activity::Flee, "predator behind the wall is not seen");
        f.agents[1].kinematics.position = Vec2::new(10.5, 12.5);
        assert_eq!(f.decide(0).activity, Activity::Flee);
    }

    #[test]
    fn courtship_requires_sexual_mode_and_mutual_acceptance() {
        let mut c = small();
        c.reproduction.mode = ReproductionMode::Sexual;
        let mut f = Fixture::new(c);
        f.add(Species::Prey, 10.0, 10.0, 0.9);
        f.add(Species::Prey, 13.0, 10.0, 0.9);
        let d = f.decide(0);
        assert_eq!((d.activity, d.target), (Activity::Court, Some(AgentId(2))));
        // A partner in poor condition is rejected by a choosy organism.
        f.agents[1].energy = 0.3 * storage(&f.agents[1].phenotype(&f.schema), &f.config.physiology);
        assert_ne!(f.decide(0).activity, Activity::Court);
        // Assortative mating: genetically distant partners are refused.
        f.agents[1].energy = f.agents[0].energy;
        f.config.reproduction.mate_distance_limit = 0.05;
        // Neutral markers at opposite ends: distance sqrt(0.5 / 13) ≈ 0.2, same phenotype.
        f.agents[1].genome.set_normalised(genes::MARKER_A, 1.0).unwrap();
        f.agents[1].genome.set_normalised(genes::MARKER_B, 0.0).unwrap();
        assert_ne!(f.decide(0).activity, Activity::Court);
    }
}
