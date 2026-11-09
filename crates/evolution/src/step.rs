//! One tick of the evolution laboratory.
//!
//! ```text
//! 1 environment   climate offset += shift_rate/1000; food regrows (parallel, per cell)
//! 2 index         phenotypes and readiness (parallel); prey and predator spatial grids
//! 3 decide        every organism decides from read-only state (parallel; per-organism
//!                 random stream derived from (tick key, id))
//! 4 live          movement, energy costs, thermal health, acquired traits, ageing (parallel,
//!                 each organism writes only itself)
//! 5 predation     hunting predators in contact strike; one predator per prey (exclusive claims:
//!                 closest, then lowest id); kill with probability p₀·min(1, 2M_pred/(M_pred+M_prey));
//!                 a successful predator then handles its kill for handling_ticks ticks
//! 6 feeding       non-fleeing prey claim min(intake, room) from their cell; proportional
//!                 sharing when claims exceed the food
//! 7 reproduction  asexual: every ready organism breeds; sexual: courting organisms in contact
//!                 with a ready, mutually accepting partner form one-to-one pairs (greedy
//!                 matching: closest first, then lowest id)
//! 8 mortality     starvation (energy ≤ 0), thermal (health ≤ 0), background and Gompertz
//!                 senescence hazards (id order); predators under the constant regime are exempt
//! 9 commit        offspring appended, the dead removed, genealogy updated
//! 10 measure      statistics every tick, speciation criterion every check_interval ticks
//! ```
//!
//! Serial phases draw from the world's generator in id (or claim) order,
//! so a run is fully determined by its seed and commands.

use std::collections::BTreeSet;
use std::f32::consts::TAU;

use organism_engine::{AgentId, Claim, Lineage, ShareRule, resolve_exclusive, resolve_matching, resolve_shared};
use rayon::prelude::*;
use simulation_core::{EventSeverity, SimEvent, SimRng, SpatialGrid};

use crate::behaviour::{Decision, Senses, decide, mate_acceptable};
use crate::config::ReproductionMode;
use crate::genes::Phenotype;
use crate::organism::{Activity, Organism, Species, capture_radius, intake_capacity, kill_probability, storage};
use crate::reproduction::{clutch, is_ready, offspring_genome, provision};
use crate::world::{DeathCause, EvolutionWorld, MAX_ORGANISMS, TickReport};

/// Bucket size of the spatial grids (world units).
const GRID_CELL: f32 = 4.0;
/// Radius around the first parent in which offspring are placed.
const BIRTH_SCATTER: f32 = 0.8;

impl EvolutionWorld {
    /// Advances the experiment by one tick.
    pub fn step(&mut self) {
        self.tick += 1;
        let mut report = TickReport::default();
        let before = (self.count(Species::Prey), self.count(Species::Predator));
        self.env.advance(&self.config);
        let tick_key = self.rng.next_u64();

        let phenotypes: Vec<Phenotype> = self.store.as_slice().par_iter().map(|o| o.phenotype(&self.schema)).collect();
        let ready: Vec<bool> = self
            .store
            .as_slice()
            .par_iter()
            .zip(phenotypes.par_iter())
            .map(|(o, p)| is_ready(o, p, &self.config))
            .collect();
        let (prey_grid, predator_grid) = self.index();
        let decisions: Vec<Decision> = {
            let ids = self.store.ids();
            let senses = Senses {
                env: &self.env,
                config: &self.config,
                schema: &self.schema,
                ids,
                agents: self.store.as_slice(),
                phenotypes: &phenotypes,
                ready: &ready,
                prey_grid: &prey_grid,
                predator_grid: &predator_grid,
            };
            (0..ids.len())
                .into_par_iter()
                .map(|slot| decide(&senses, slot, SimRng::derive(tick_key, ids[slot].get())))
                .collect()
        };
        {
            let (env, config, schema, tick) = (&self.env, &self.config, &self.schema, self.tick);
            self.store
                .as_mut_slice()
                .par_iter_mut()
                .zip(decisions.par_iter())
                .for_each(|(o, d)| o.live(d, env, config, schema, tick));
        }

        let mut fate: Vec<Option<DeathCause>> = vec![None; self.store.len()];
        self.predation(&decisions, &phenotypes, &mut fate, &mut report);
        self.feeding(&phenotypes, &fate, &mut report);
        self.reproduction(&phenotypes, &fate, &mut report);
        self.mortality(&phenotypes, &mut fate);
        let deaths: Vec<(AgentId, DeathCause)> =
            fate.iter().zip(self.store.ids()).filter_map(|(cause, id)| cause.map(|c| (*id, c))).collect();
        self.bury(&deaths, &mut report);
        self.store.commit();

        let after = (self.count(Species::Prey), self.count(Species::Predator));
        for (species, was, is) in [(Species::Prey, before.0, after.0), (Species::Predator, before.1, after.1)] {
            if was > 0 && is == 0 {
                let message = format!("the {} population went extinct", species.label());
                self.events.push(SimEvent::new(self.tick, "extinction", message).severity(EventSeverity::Major));
            }
        }
        if self.tick.is_multiple_of(u64::from(self.config.speciation.check_interval.max(1))) {
            self.refresh_speciation();
        }
        self.close_report(report);
        self.refresh_stats();
    }

    /// Spatial grids of prey and predators (slot = store index).
    fn index(&self) -> (SpatialGrid, SpatialGrid) {
        let (w, h) = (self.env.width() as f32, self.env.height() as f32);
        let agents = self.store.as_slice();
        // The store never exceeds MAX_ORGANISMS, so every index fits in u32.
        let of = |species: Species| {
            agents
                .iter()
                .enumerate()
                .filter(move |(_, o)| o.species == species)
                .map(|(i, o)| (i as u32, o.kinematics.position))
        };
        let mut prey = SpatialGrid::new(w, h, GRID_CELL);
        let mut predators = SpatialGrid::new(w, h, GRID_CELL);
        prey.rebuild(of(Species::Prey));
        predators.rebuild(of(Species::Predator));
        (prey, predators)
    }

    fn predation(
        &mut self,
        decisions: &[Decision],
        phenotypes: &[Phenotype],
        fate: &mut [Option<DeathCause>],
        report: &mut TickReport,
    ) {
        let agents = self.store.as_slice();
        let ids = self.store.ids();
        let claims: Vec<Claim> = decisions
            .iter()
            .enumerate()
            .filter(|(slot, d)| agents[*slot].species == Species::Predator && d.activity == Activity::Hunt)
            .filter_map(|(slot, d)| {
                let prey = self.store.index_of(d.target?)?;
                let distance = agents[slot].kinematics.position.distance(agents[prey].kinematics.position);
                (distance <= capture_radius(&phenotypes[slot]))
                    .then(|| Claim::new(ids[slot], ids[prey].get()).distance(distance))
            })
            .collect();
        let predators = &self.config.predators;
        for strike in resolve_exclusive(&claims) {
            let (Some(hunter), Some(prey)) =
                (self.store.index_of(strike.claimant), self.store.index_of(AgentId(strike.target)))
            else {
                continue;
            };
            report.attacks += 1;
            let p_kill = kill_probability(&phenotypes[hunter], &phenotypes[prey], predators.capture_probability);
            if self.rng.chance(p_kill) {
                fate[prey] = Some(DeathCause::Predation);
                let victim = &self.store.as_slice()[prey];
                let gain =
                    predators.assimilation * (predators.body_energy * phenotypes[prey].size + victim.energy.max(0.0));
                let cap = storage(&phenotypes[hunter], &self.config.physiology);
                let predator = &mut self.store.as_mut_slice()[hunter];
                predator.energy = (predator.energy + gain).min(cap);
                predator.handling = predators.handling_ticks;
            }
        }
    }

    fn feeding(&mut self, phenotypes: &[Phenotype], fate: &[Option<DeathCause>], report: &mut TickReport) {
        let phys = &self.config.physiology;
        let food_energy = self.config.food.energy;
        let agents = self.store.as_slice();
        let ids = self.store.ids();
        let claims: Vec<Claim> = (0..agents.len())
            .filter(|&s| {
                agents[s].species == Species::Prey && fate[s].is_none() && agents[s].activity != Activity::Flee
            })
            .filter_map(|s| {
                let cell = self.env.cell_of(agents[s].kinematics.position)?;
                let room = (storage(&phenotypes[s], phys) - agents[s].energy).max(0.0) / food_energy;
                let want = intake_capacity(&phenotypes[s], phys).min(room);
                (want > 0.0).then(|| Claim::new(ids[s], cell as u64).amount(want))
            })
            .collect();
        let food = self.env.food().as_slice();
        let allocations =
            resolve_shared(&claims, |cell| food.get(cell as usize).copied().unwrap_or(0.0), ShareRule::Proportional);
        for a in allocations {
            let taken = self.env.take_food(a.target as usize, a.granted);
            if let Some(slot) = self.store.index_of(a.claimant) {
                self.store.as_mut_slice()[slot].energy += taken * food_energy;
                report.food_eaten += f64::from(taken);
            }
        }
    }

    fn reproduction(&mut self, phenotypes: &[Phenotype], fate: &[Option<DeathCause>], report: &mut TickReport) {
        let agents = self.store.as_slice();
        let ready: Vec<bool> = (0..agents.len())
            .map(|s| fate[s].is_none() && is_ready(&agents[s], &phenotypes[s], &self.config))
            .collect();
        let pairs: Vec<(usize, Option<usize>)> = match self.config.reproduction.mode {
            ReproductionMode::Asexual => (0..agents.len()).filter(|&s| ready[s]).map(|s| (s, None)).collect(),
            ReproductionMode::Sexual => {
                let ids = self.store.ids();
                let accepts = |a: usize, b: usize| {
                    mate_acceptable(
                        &self.config,
                        &self.schema,
                        (&agents[a], &phenotypes[a]),
                        (&agents[b], &phenotypes[b]),
                    )
                };
                let claims: Vec<Claim> = (0..agents.len())
                    .filter(|&s| ready[s] && agents[s].activity == Activity::Court)
                    .filter_map(|s| {
                        let mate = self.store.index_of(agents[s].target?)?;
                        let distance = agents[s].kinematics.position.distance(agents[mate].kinematics.position);
                        let ok = ready[mate]
                            && distance <= self.config.reproduction.mate_radius
                            && accepts(s, mate)
                            && accepts(mate, s);
                        ok.then(|| Claim::new(ids[s], ids[mate].get()).distance(distance))
                    })
                    .collect();
                let pairs: Vec<(usize, Option<usize>)> = mating_pairs(&claims)
                    .into_iter()
                    .filter_map(|(a, b)| Some((self.store.index_of(a)?, Some(self.store.index_of(b)?))))
                    .collect();
                report.matings += pairs.len() as u32;
                pairs
            }
        };
        for (first, second) in pairs {
            self.breed(first, second, phenotypes, report);
        }
    }

    /// One breeding event of `first` (and `second`): offspring are queued
    /// and recorded, the parents pay and enter their refractory period.
    fn breed(&mut self, first: usize, second: Option<usize>, phenotypes: &[Phenotype], report: &mut TickReport) {
        let agents = self.store.as_slice();
        let parent: Organism = agents[first].clone();
        let partner: Option<Organism> = second.map(|s| agents[s].clone());
        let brood =
            clutch((&parent, &phenotypes[first]), partner.as_ref().zip(second.map(|s| &phenotypes[s])), &self.config);
        let room = MAX_ORGANISMS.saturating_sub(self.store.len() + self.store.pending().spawns().len());
        let count = (brood.count as usize).min(room);
        report.births_blocked += brood.count - count as u32;
        let evolve = parent.species == Species::Prey || self.config.predators.evolve;
        let mut born = 0u32;
        for _ in 0..count {
            let genome = match offspring_genome(
                &parent.genome,
                partner.as_ref().map(|p| &p.genome),
                &self.schema,
                &self.config,
                evolve,
                &mut self.rng,
            ) {
                Ok((genome, mutated)) => {
                    report.mutations += mutated as u32;
                    genome
                }
                Err(e) => {
                    self.events
                        .push(SimEvent::new(self.tick, "inheritance", e.to_string()).severity(EventSeverity::Major));
                    break;
                }
            };
            let home = parent.kinematics.position;
            let scattered = home + self.rng.in_disc(BIRTH_SCATTER);
            let position = if self.env.is_passable(scattered) { scattered } else { home };
            let heading = self.rng.f32() * TAU;
            let id = self.store.next_id();
            let lineage = Lineage::offspring(id, &parent.lineage, partner.as_ref().map(|p| &p.lineage), self.tick);
            let energy =
                provision(brood.energy_each, &Phenotype::express(&genome, &self.schema), &self.config.physiology);
            let child = Organism::new(parent.species, position, heading, genome, energy, 0, lineage, parent.founder);
            self.store.queue_spawn(child);
            if let Err(e) = self.genealogy.record_birth(lineage) {
                self.events.push(SimEvent::new(self.tick, "genealogy", e.to_string()).severity(EventSeverity::Major));
            }
            born += 1;
            match parent.species {
                Species::Prey => report.prey_births += 1,
                Species::Predator => report.predator_births += 1,
            }
        }
        let refractory = self.config.reproduction.refractory;
        for (slot, cost) in [(Some(first), brood.cost_first), (second, brood.cost_second)] {
            if let Some(o) = slot.and_then(|s| self.store.as_mut_slice().get_mut(s)) {
                o.energy -= cost;
                o.refractory = refractory;
                o.offspring += born;
            }
        }
    }

    fn mortality(&mut self, phenotypes: &[Phenotype], fate: &mut [Option<DeathCause>]) {
        let phys = &self.config.physiology;
        let constant = self.config.predators.constant;
        for (slot, o) in self.store.as_mut_slice().iter_mut().enumerate() {
            if fate[slot].is_some() {
                continue;
            }
            if constant && o.species == Species::Predator {
                o.energy = o.energy.max(0.0);
                o.health = o.health.max(0.0);
                continue;
            }
            let cause = if o.energy <= 0.0 {
                DeathCause::Starvation
            } else if o.health <= 0.0 {
                DeathCause::Thermal
            } else if self.rng.chance(f64::from(phys.background_mortality)) {
                DeathCause::Background
            } else if self.rng.chance(crate::organism::senescence_hazard(o.age, &phenotypes[slot], phys)) {
                DeathCause::Senescence
            } else {
                continue;
            };
            fate[slot] = Some(cause);
        }
    }
}

/// One-to-one mating pairs `(suitor, partner)` from courtship claims, closest
/// pair first (then lowest suitor id, then lowest partner id).
///
/// The organism engine's greedy matching gives every suitor at most one
/// partner and every partner at most one suitor. A pair is unordered,
/// though: an organism can win once as a suitor and once as a partner (A
/// courts B while B courts C). The matched claims are therefore visited again
/// in the same rank order and a pair is kept only if neither member is already
/// paired, so the closest pair wins whatever the ids.
pub(crate) fn mating_pairs(claims: &[Claim]) -> Vec<(AgentId, AgentId)> {
    let mut matched = resolve_matching(claims);
    matched.sort_by(|a, b| {
        a.distance.total_cmp(&b.distance).then(a.claimant.cmp(&b.claimant)).then(a.target.cmp(&b.target))
    });
    let mut paired = BTreeSet::new();
    let mut pairs = Vec::with_capacity(matched.len());
    for claim in matched {
        let (a, b) = (claim.claimant, AgentId(claim.target));
        if a != b && !paired.contains(&a) && !paired.contains(&b) {
            paired.insert(a);
            paired.insert(b);
            pairs.push((a, b));
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn court(suitor: u64, partner: u64, distance: f32) -> Claim {
        Claim::new(AgentId(suitor), partner).distance(distance)
    }

    #[test]
    fn mating_pairs_are_one_to_one_and_closest_first() {
        // 1 courts 2 from afar while 2 courts 3 at close range: 2 and 3 pair,
        // whatever the id order of the claims.
        let claims = [court(1, 2, 1.2), court(2, 3, 0.4)];
        assert_eq!(mating_pairs(&claims), vec![(AgentId(2), AgentId(3))]);
        // Mutual courtship forms a single pair.
        let mutual = [court(5, 6, 0.8), court(6, 5, 0.8)];
        assert_eq!(mating_pairs(&mutual), vec![(AgentId(5), AgentId(6))]);
        // Two suitors of one partner: the closer one wins, the other stays single.
        let rivals = [court(7, 9, 1.0), court(8, 9, 0.5), court(10, 11, 1.4)];
        assert_eq!(mating_pairs(&rivals), vec![(AgentId(8), AgentId(9)), (AgentId(10), AgentId(11))]);
        let mut members: Vec<AgentId> = mating_pairs(&rivals).into_iter().flat_map(|(a, b)| [a, b]).collect();
        let n = members.len();
        members.sort_unstable();
        members.dedup();
        assert_eq!(members.len(), n, "nobody breeds twice in a tick");
    }
}
