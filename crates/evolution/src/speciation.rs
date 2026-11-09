//! The explicit, observable speciation criterion.
//!
//! Speciation is **never** inferred from anything but this rule, and the
//! model reports "not applicable" when its preconditions do not hold:
//!
//! 1. *Reproductive isolation is possible*: sexual reproduction with
//!    assortative mating — mates must be closer than the genetic distance
//!    limit `θ < 1` (otherwise there is nothing to isolate two groups).
//! 2. *Two clusters*: the living prey are split by deterministic 2-means
//!    in normalised heritable gene space (seeds: the individual farthest
//!    from the centroid, then the one farthest from it; ties to the lower
//!    store index; at most 25 Lloyd iterations). Each cluster must hold at
//!    least `max(5, min_cluster_fraction · N)` individuals.
//! 3. *Separation*: the mean genetic distance between members of different
//!    clusters exceeds `θ`, i.e. the average cross-cluster pair could not mate.
//! 4. *Persistence*: conditions 2–3 hold at every evaluation (every
//!    `check_interval` ticks) while the mean generation of the prey advances
//!    by at least `generations`. Then a speciation event is declared.
//!
//! When separation is lost the status returns to monitoring; declared events
//! stay in the history.

use organism_engine::{Genome, GenomeSchema};
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::config::{EvolutionConfig, ReproductionMode};
use crate::genes::HERITABLE_COUNT;
use crate::stats::mean_distance_between;

/// Lloyd iterations of the 2-means split.
const MAX_ITERATIONS: usize = 25;
/// Smallest population evaluated.
const MIN_POPULATION: usize = 20;
/// Smallest cluster accepted.
const MIN_CLUSTER: usize = 5;
/// Random stream tag for cross-cluster distance sampling.
const SPECIATION_STREAM: u64 = 0x5350_4543_0000_0001;
/// Cross-cluster pairs sampled.
const PAIR_BUDGET: usize = 2000;

/// A two-cluster split of the prey population.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClusterSplit {
    /// Cluster sizes.
    pub sizes: [u32; 2],
    /// Mean genetic distance between members of different clusters.
    pub between: f64,
    /// Normalised heritable centroids.
    pub centroids: [Vec<f32>; 2],
}

/// A declared speciation event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciationEvent {
    /// Tick of the declaration.
    pub tick: u64,
    /// Mean prey generation at the declaration.
    pub generation: f64,
    /// The split that satisfied the criterion.
    pub split: ClusterSplit,
}

/// Current state of the criterion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SpeciationStatus {
    /// Preconditions do not hold; the reason is shown to the user.
    NotApplicable(String),
    /// Evaluated; no persistent separation.
    Monitoring,
    /// Two separated clusters since `since_tick` / `since_generation`.
    Separated {
        /// First tick of the current separation.
        since_tick: u64,
        /// Mean generation at that tick.
        since_generation: f64,
        /// Generations elapsed since.
        generations: f64,
    },
    /// Separation persisted long enough; declared at the given event.
    Speciated(SpeciationEvent),
}

/// Evaluates the criterion over time (hysteresis state is part of snapshots).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeciationTracker {
    status: SpeciationStatus,
    last_split: Option<ClusterSplit>,
    history: Vec<SpeciationEvent>,
}

impl Default for SpeciationTracker {
    fn default() -> Self {
        Self { status: SpeciationStatus::Monitoring, last_split: None, history: Vec::new() }
    }
}

impl SpeciationTracker {
    /// Current status.
    pub fn status(&self) -> &SpeciationStatus {
        &self.status
    }

    /// The most recent two-cluster split (if the population was evaluated).
    pub fn last_split(&self) -> Option<&ClusterSplit> {
        self.last_split.as_ref()
    }

    /// Every declared event, oldest first.
    pub fn history(&self) -> &[SpeciationEvent] {
        &self.history
    }

    /// Why the criterion cannot apply under `config`, if it cannot.
    pub fn inapplicable(config: &EvolutionConfig) -> Option<String> {
        if !config.speciation.enabled {
            Some("the speciation criterion is switched off".into())
        } else if config.reproduction.mode != ReproductionMode::Sexual {
            Some("requires sexual reproduction".into())
        } else if config.reproduction.mate_distance_limit >= 1.0 {
            Some("requires assortative mating (mating distance limit below 1)".into())
        } else {
            None
        }
    }

    /// Updates the status as soon as a parameter change makes the criterion
    /// inapplicable (`NotApplicable` with the reason) or applicable again
    /// (`Monitoring`), so the interface never shows a stale state. The
    /// population itself is evaluated only at the next check.
    pub(crate) fn refresh_applicability(&mut self, config: &EvolutionConfig) {
        match (Self::inapplicable(config), &self.status) {
            (Some(reason), _) => {
                self.status = SpeciationStatus::NotApplicable(reason);
                self.last_split = None;
            }
            (None, SpeciationStatus::NotApplicable(_)) => self.status = SpeciationStatus::Monitoring,
            (None, _) => {}
        }
    }

    /// Evaluates the criterion on the living prey. Returns a newly declared
    /// event, if any.
    pub fn update(
        &mut self,
        prey: &[&Genome],
        mean_generation: f64,
        config: &EvolutionConfig,
        schema: &GenomeSchema,
        tick: u64,
        seed: u64,
    ) -> Option<SpeciationEvent> {
        if let Some(reason) = Self::inapplicable(config) {
            self.status = SpeciationStatus::NotApplicable(reason);
            self.last_split = None;
            return None;
        }
        let limit = f64::from(config.reproduction.mate_distance_limit);
        let mut rng = SimRng::derive(seed ^ SPECIATION_STREAM, tick);
        self.last_split = two_means(prey, schema, &mut rng);
        let min_size =
            MIN_CLUSTER.max((f64::from(config.speciation.min_cluster_fraction) * prey.len() as f64) as usize);
        let separated = self.last_split.as_ref().filter(|s| {
            prey.len() >= MIN_POPULATION && s.sizes.iter().all(|&n| n as usize >= min_size) && s.between > limit
        });
        let Some(split) = separated.cloned() else {
            self.status = SpeciationStatus::Monitoring;
            return None;
        };
        let (since_tick, since_generation) = match &self.status {
            SpeciationStatus::Speciated(_) => return None,
            SpeciationStatus::Separated { since_tick, since_generation, .. } => (*since_tick, *since_generation),
            _ => (tick, mean_generation),
        };
        let generations = mean_generation - since_generation;
        if generations >= f64::from(config.speciation.generations) {
            let event = SpeciationEvent { tick, generation: mean_generation, split };
            self.history.push(event.clone());
            self.status = SpeciationStatus::Speciated(event.clone());
            return Some(event);
        }
        self.status = SpeciationStatus::Separated { since_tick, since_generation, generations };
        None
    }
}

fn squared_distance(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

fn centroid(points: &[&[f32]], members: impl Iterator<Item = usize>) -> Vec<f32> {
    let mut sum = vec![0.0f64; HERITABLE_COUNT];
    let mut n = 0usize;
    for i in members {
        for (s, v) in sum.iter_mut().zip(points[i]) {
            *s += f64::from(*v);
        }
        n += 1;
    }
    sum.into_iter().map(|s| if n == 0 { 0.0 } else { (s / n as f64) as f32 }).collect()
}

/// Index of the point farthest from `from` (ties to the lower index).
fn farthest(points: &[&[f32]], from: &[f32]) -> usize {
    let mut best = (0, -1.0f32);
    for (i, p) in points.iter().enumerate() {
        let d = squared_distance(p, from);
        if d > best.1 {
            best = (i, d);
        }
    }
    best.0
}

/// Deterministic 2-means split of the heritable genes; `None` for fewer than
/// two genomes or when every genome is identical.
fn two_means(genomes: &[&Genome], schema: &GenomeSchema, rng: &mut SimRng) -> Option<ClusterSplit> {
    if genomes.len() < 2 {
        return None;
    }
    let points: Vec<&[f32]> = genomes.iter().map(|g| &g.values()[..HERITABLE_COUNT]).collect();
    let all = centroid(&points, 0..points.len());
    let a = farthest(&points, &all);
    let b = farthest(&points, points[a]);
    if squared_distance(points[a], points[b]) == 0.0 {
        return None;
    }
    let mut centroids = [points[a].to_vec(), points[b].to_vec()];
    let mut assignment = vec![0u8; points.len()];
    for iteration in 0..MAX_ITERATIONS {
        let mut changed = false;
        for (slot, p) in assignment.iter_mut().zip(&points) {
            let cluster = u8::from(squared_distance(p, &centroids[1]) < squared_distance(p, &centroids[0]));
            changed |= *slot != cluster;
            *slot = cluster;
        }
        if !changed && iteration > 0 {
            break;
        }
        for (k, c) in centroids.iter_mut().enumerate() {
            let members = assignment.iter().enumerate().filter(|(_, a)| usize::from(**a) == k).map(|(i, _)| i);
            *c = centroid(&points, members);
        }
    }
    let groups: [Vec<&Genome>; 2] = [0u8, 1]
        .map(|k| genomes.iter().zip(&assignment).filter(|(_, a)| **a == k).map(|(g, _)| *g).collect::<Vec<_>>());
    let between = mean_distance_between(&groups[0], &groups[1], schema, PAIR_BUDGET, rng);
    Some(ClusterSplit { sizes: [groups[0].len() as u32, groups[1].len() as u32], between, centroids })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genes::{self, GENE_COUNT};

    fn population(centres: &[f32], per_cluster: usize, jitter: f32, rng: &mut SimRng) -> Vec<Genome> {
        let schema = genes::schema(0.05).unwrap();
        centres
            .iter()
            .flat_map(|&c| {
                (0..per_cluster)
                    .map(|_| {
                        let mut v = vec![0.5; GENE_COUNT];
                        for x in v.iter_mut().take(HERITABLE_COUNT) {
                            *x = (c + jitter * rng.normal() as f32).clamp(0.0, 1.0);
                        }
                        Genome::from_normalised(&schema, v).unwrap()
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn sexual(limit: f32, generations: u32) -> EvolutionConfig {
        let mut c = EvolutionConfig::default();
        c.reproduction.mode = ReproductionMode::Sexual;
        c.reproduction.mate_distance_limit = limit;
        c.speciation.generations = generations;
        c
    }

    #[test]
    fn two_means_recovers_planted_clusters() {
        let schema = genes::schema(0.05).unwrap();
        let mut rng = SimRng::seed_from_u64(1);
        let pop = population(&[0.2, 0.8], 40, 0.03, &mut rng);
        let refs: Vec<&Genome> = pop.iter().collect();
        let split = two_means(&refs, &schema, &mut rng).unwrap();
        assert_eq!(split.sizes, [40, 40]);
        assert!((split.between - 0.6).abs() < 0.03, "{}", split.between);
        let clones = vec![&pop[0]; 10];
        assert!(two_means(&clones, &schema, &mut rng).is_none());
    }

    #[test]
    fn declaration_needs_persistent_separation_for_n_generations() {
        let schema = genes::schema(0.05).unwrap();
        let mut rng = SimRng::seed_from_u64(2);
        let split_pop = population(&[0.2, 0.8], 30, 0.03, &mut rng);
        let split: Vec<&Genome> = split_pop.iter().collect();
        let config = sexual(0.3, 3);
        let mut t = SpeciationTracker::default();
        assert!(t.update(&split, 10.0, &config, &schema, 100, 7).is_none());
        assert!(matches!(t.status(), SpeciationStatus::Separated { .. }));
        assert!(t.update(&split, 12.5, &config, &schema, 200, 7).is_none());
        let event = t.update(&split, 13.0, &config, &schema, 300, 7).unwrap();
        assert_eq!((event.tick, event.generation), (300, 13.0));
        assert!(matches!(t.status(), SpeciationStatus::Speciated(_)));
        assert_eq!(t.history().len(), 1);
        // Losing the separation resets the clock but keeps the history.
        let mixed_pop = population(&[0.5], 60, 0.03, &mut rng);
        let mixed: Vec<&Genome> = mixed_pop.iter().collect();
        assert!(t.update(&mixed, 14.0, &config, &schema, 400, 7).is_none());
        assert_eq!(t.status(), &SpeciationStatus::Monitoring);
        assert_eq!(t.history().len(), 1);
        // A small distance limit is not exceeded by close clusters.
        let close_pop = population(&[0.45, 0.55], 30, 0.01, &mut rng);
        let close: Vec<&Genome> = close_pop.iter().collect();
        assert!(t.update(&close, 20.0, &config, &schema, 500, 7).is_none());
        assert_eq!(t.status(), &SpeciationStatus::Monitoring);
    }

    #[test]
    fn criterion_is_not_applied_without_its_preconditions() {
        let schema = genes::schema(0.05).unwrap();
        let mut rng = SimRng::seed_from_u64(3);
        let pop = population(&[0.1, 0.9], 30, 0.02, &mut rng);
        let refs: Vec<&Genome> = pop.iter().collect();
        for config in [EvolutionConfig::default(), sexual(1.0, 1)] {
            let mut t = SpeciationTracker::default();
            for generation in 0..10 {
                assert!(t.update(&refs, f64::from(generation) * 5.0, &config, &schema, 10, 1).is_none());
            }
            assert!(matches!(t.status(), SpeciationStatus::NotApplicable(_)));
        }
    }
}
