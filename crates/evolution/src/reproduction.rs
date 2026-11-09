//! Breeding rules: readiness, inheritance and parental investment.
//!
//! * **Readiness**: `age ≥ maturity`, refractory period over, health ≥ ½ and
//!   energy ≥ `threshold · E_max`.
//! * **Inheritance** goes through [`organism_engine::Genome`]: asexual
//!   offspring copy the parent's heritable genes; sexual offspring recombine
//!   both parents ([`organism_engine::Crossover`]) over the heritable genes
//!   only ([`recombine`]); then every heritable gene mutates with probability
//!   `rate` by `N(0, size)` (normalised units, reflected or clamped at the
//!   bounds). Acquired traits always start at their defaults. Non-evolving
//!   predators clone their first parent exactly.
//! * **Investment** (the r/K trade-off is explicit): each parent spends
//!   `effort · E` of its reserve; the clutch (`n` = the parent's clutch gene,
//!   or the rounded mean of both parents') shares
//!   `birth_efficiency · Σ spent` equally, so more offspring means less
//!   energy for each. A newborn stores at most its own capacity `s · M`
//!   ([`provision`]); provisioning beyond it is lost, like any energy above
//!   `E_max`.

use organism_engine::{Crossover, Genome, GenomeSchema};
use simulation_core::SimRng;

use crate::config::{EvolutionConfig, PhysiologyConfig};
use crate::error::EvolutionError;
use crate::genes::Phenotype;
use crate::organism::{Organism, Species, storage};

/// Whether an organism may breed now (predators never breed under the
/// constant predation regime).
pub fn is_ready(o: &Organism, p: &Phenotype, config: &EvolutionConfig) -> bool {
    !(o.species == Species::Predator && config.predators.constant)
        && o.age >= p.maturity
        && o.refractory == 0
        && o.health >= 0.5
        && o.energy >= config.reproduction.threshold * storage(p, &config.physiology)
}

/// Copy of the heritable genes with acquired traits reset to their defaults.
pub fn clone_heritable(parent: &Genome, schema: &GenomeSchema) -> Result<Genome, EvolutionError> {
    let values = parent
        .values()
        .iter()
        .zip(schema.genes())
        .map(|(v, g)| if g.heritable { *v } else { g.normalise(g.default) })
        .collect();
    Ok(Genome::from_normalised(schema, values)?)
}

/// Genome of one offspring of `first` (and `second` in sexual reproduction).
/// Returns the genome and the number of mutated genes. With `evolve = false`
/// the offspring is an exact copy of `first`'s heritable genes and no random
/// numbers are drawn.
pub fn offspring_genome(
    first: &Genome,
    second: Option<&Genome>,
    schema: &GenomeSchema,
    config: &EvolutionConfig,
    evolve: bool,
    rng: &mut SimRng,
) -> Result<(Genome, usize), EvolutionError> {
    if !evolve {
        return Ok((clone_heritable(first, schema)?, 0));
    }
    let mut child = match second {
        Some(second) => recombine(first, second, schema, config.reproduction.crossover, rng)?,
        None => clone_heritable(first, schema)?,
    };
    let mutated = child.mutate_with(schema, config.mutation.rate, config.mutation.bounds, rng)?;
    Ok((child, mutated))
}

/// Recombines two parents with the organism engine's crossover `operator`
/// applied to the **heritable genes alone**; acquired traits start at their
/// defaults.
///
/// The projection matters for one-point crossover: the engine draws its cut
/// uniformly over all schema slots, so with the two acquired slots at the end
/// of the genome a cut falling among them would copy the first parent's
/// heritable genes unchanged (2 of 14 cuts, about one sexual offspring in
/// seven). On the heritable projection every cut separates two heritable
/// genes, uniformly over the `H − 1` gaps. Uniform crossover draws the same
/// numbers either way (one per heritable gene, in order).
pub fn recombine(
    first: &Genome,
    second: &Genome,
    schema: &GenomeSchema,
    operator: Crossover,
    rng: &mut SimRng,
) -> Result<Genome, EvolutionError> {
    first.check(schema)?;
    second.check(schema)?;
    let slots: Vec<usize> = schema.genes().iter().enumerate().filter(|(_, g)| g.heritable).map(|(i, _)| i).collect();
    let heritable = GenomeSchema::new(slots.iter().map(|&i| schema.genes()[i].clone()).collect())?;
    let project = |g: &Genome| Genome::from_normalised(&heritable, slots.iter().map(|&i| g.values()[i]).collect());
    let child = Genome::crossover(&project(first)?, &project(second)?, &heritable, operator, rng)?;
    let mut values = Genome::defaults(schema).values().to_vec();
    for (&slot, &value) in slots.iter().zip(child.values()) {
        values[slot] = value;
    }
    Ok(Genome::from_normalised(schema, values)?)
}

/// Energy a newborn with phenotype `child` actually stores out of the
/// `offered` provisioning: at most its storage capacity `s · M`.
pub fn provision(offered: f32, child: &Phenotype, phys: &PhysiologyConfig) -> f32 {
    offered.clamp(0.0, storage(child, phys))
}

/// Size and provisioning of one clutch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clutch {
    /// Number of offspring.
    pub count: u32,
    /// Energy given to each offspring.
    pub energy_each: f32,
    /// Energy spent by the first parent.
    pub cost_first: f32,
    /// Energy spent by the second parent (0 for asexual breeding).
    pub cost_second: f32,
}

/// Computes the clutch of one breeding event.
pub fn clutch(
    first: (&Organism, &Phenotype),
    second: Option<(&Organism, &Phenotype)>,
    config: &EvolutionConfig,
) -> Clutch {
    let spent = |(o, p): (&Organism, &Phenotype)| p.effort * o.energy.max(0.0);
    let cost_first = spent(first);
    let cost_second = second.map_or(0.0, spent);
    let count = match second {
        None => first.1.clutch,
        Some((_, q)) => ((first.1.clutch + q.clutch) as f32 / 2.0).round() as u32,
    }
    .max(1);
    let energy_each = (cost_first + cost_second) * config.reproduction.birth_efficiency / count as f32;
    Clutch { count, energy_each, cost_first, cost_second }
}

#[cfg(test)]
mod tests {
    use organism_engine::{AgentId, Lineage};
    use simulation_core::glam::Vec2;

    use super::*;
    use crate::genes::{self, ACCLIMATION, CLUTCH, EXPERIENCE, GENES, HERITABLE_COUNT, MATURITY};

    fn organism(genome: Genome, energy: f32, age: u32) -> Organism {
        Organism::new(Species::Prey, Vec2::ZERO, 0.0, genome, energy, age, Lineage::founder(AgentId(1), 0), AgentId(1))
    }

    #[test]
    fn acquired_traits_are_never_inherited() {
        let config = EvolutionConfig::default();
        let schema = genes::schema(config.mutation.size).unwrap();
        let mut parent = Genome::random(&schema, &mut SimRng::seed_from_u64(1));
        parent.set_normalised(ACCLIMATION, 1.0).unwrap();
        parent.set_normalised(EXPERIENCE, 0.9).unwrap();
        let other = Genome::random(&schema, &mut SimRng::seed_from_u64(2));
        let mut rng = SimRng::seed_from_u64(3);
        for (second, evolve) in [(None, false), (None, true), (Some(&other), true)] {
            for _ in 0..200 {
                let (child, _) = offspring_genome(&parent, second, &schema, &config, evolve, &mut rng).unwrap();
                assert_eq!(child.express(&schema, ACCLIMATION), 0.0, "acclimation resets to 0 °C");
                assert_eq!(child.express(&schema, EXPERIENCE), 0.0, "experience resets to 0");
            }
        }
        // Without evolution (or with rate 0) heritable genes are copied exactly.
        let (clone, mutated) = offspring_genome(&parent, Some(&other), &schema, &config, false, &mut rng).unwrap();
        assert_eq!(mutated, 0);
        assert_eq!(&clone.values()[..HERITABLE_COUNT], &parent.values()[..HERITABLE_COUNT]);
        let mut exact = config.clone();
        exact.mutation.rate = 0.0;
        let (copy, _) = offspring_genome(&parent, None, &schema, &exact, true, &mut rng).unwrap();
        assert_eq!(&copy.values()[..HERITABLE_COUNT], &parent.values()[..HERITABLE_COUNT]);
    }

    #[test]
    fn one_point_crossover_always_recombines_heritable_genes() {
        let schema = genes::schema(0.05).unwrap();
        let a = Genome::from_normalised(&schema, vec![0.0; genes::GENE_COUNT]).unwrap();
        let b = Genome::from_normalised(&schema, vec![1.0; genes::GENE_COUNT]).unwrap();
        let mut rng = SimRng::seed_from_u64(21);
        let mut cuts = [0u32; HERITABLE_COUNT + 1];
        let n = 12_000;
        for _ in 0..n {
            let child = recombine(&a, &b, &schema, Crossover::OnePoint, &mut rng).unwrap();
            let heritable = &child.values()[..HERITABLE_COUNT];
            // A prefix from `a` (0) and a non-empty suffix from `b` (1).
            let cut = heritable.iter().position(|v| *v == 1.0).expect("some genes come from the second parent");
            assert!(cut >= 1, "some genes come from the first parent");
            assert!(heritable[cut..].iter().all(|v| *v == 1.0), "a single cut");
            cuts[cut] += 1;
            assert_eq!(child.express(&schema, ACCLIMATION), 0.0);
            assert_eq!(child.express(&schema, EXPERIENCE), 0.0);
        }
        // Every one of the 12 gaps between the 13 heritable genes is equally likely.
        let expected = n as f64 / (HERITABLE_COUNT - 1) as f64;
        for (cut, &count) in cuts.iter().enumerate().take(HERITABLE_COUNT).skip(1) {
            assert!((f64::from(count) - expected).abs() < 0.12 * expected, "cut {cut}: {count} vs {expected}");
        }
        // Uniform crossover on the projection draws exactly what the engine
        // draws on the full genome (one number per heritable gene).
        let mut p = SimRng::seed_from_u64(5);
        let mut q = p.clone();
        let projected = recombine(&a, &b, &schema, Crossover::Uniform, &mut p).unwrap();
        let direct = Genome::crossover_uniform(&a, &b, &schema, &mut q).unwrap();
        assert_eq!((projected, p), (direct, q));
    }

    #[test]
    fn newborns_store_at_most_their_capacity() {
        let phys = PhysiologyConfig::default();
        let schema = genes::schema(0.05).unwrap();
        let p = Phenotype::express(&Genome::defaults(&schema), &schema);
        let capacity = storage(&p, &phys);
        assert_eq!(provision(0.4 * capacity, &p, &phys), 0.4 * capacity);
        assert_eq!(provision(1.7 * capacity, &p, &phys), capacity, "overflow is not stored");
        assert_eq!(provision(-1.0, &p, &phys), 0.0);
    }

    #[test]
    fn mutation_rate_and_size_have_the_configured_statistics() {
        let mut config = EvolutionConfig::default();
        config.mutation.rate = 0.25;
        config.mutation.size = 0.04;
        let schema = genes::schema(config.mutation.size).unwrap();
        let parent = Genome::from_normalised(&schema, vec![0.5; genes::GENE_COUNT]).unwrap();
        let mut rng = SimRng::seed_from_u64(11);
        let (mut mutated, mut steps) = (0usize, Vec::new());
        let n = 8000;
        for _ in 0..n {
            let (child, m) = offspring_genome(&parent, None, &schema, &config, true, &mut rng).unwrap();
            mutated += m;
            for i in 0..HERITABLE_COUNT {
                let step = child.values()[i] - parent.values()[i];
                if step != 0.0 {
                    steps.push(f64::from(step));
                }
            }
        }
        let fraction = mutated as f64 / (n * HERITABLE_COUNT) as f64;
        assert!((fraction - 0.25).abs() < 0.01, "mutated fraction {fraction}");
        let mean = steps.iter().sum::<f64>() / steps.len() as f64;
        let sd = (steps.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / steps.len() as f64).sqrt();
        assert!(mean.abs() < 0.002, "unbiased steps: {mean}");
        assert!((sd - 0.04).abs() < 0.002, "step sd {sd}");
    }

    #[test]
    fn readiness_requires_maturity_energy_and_rest() {
        let config = EvolutionConfig::default();
        let schema = genes::schema(config.mutation.size).unwrap();
        let g = Genome::defaults(&schema);
        let p = Phenotype::express(&g, &schema);
        let full = storage(&p, &config.physiology);
        let mut o = organism(g, full, p.maturity);
        assert!(is_ready(&o, &p, &config));
        o.age = p.maturity - 1;
        assert!(!is_ready(&o, &p, &config), "juvenile");
        o.age = p.maturity;
        o.refractory = 3;
        assert!(!is_ready(&o, &p, &config), "refractory");
        o.refractory = 0;
        o.energy = 0.5 * full;
        assert!(!is_ready(&o, &p, &config), "below the breeding threshold");
        assert_eq!(GENES[MATURITY].default as u32, p.maturity);
    }

    #[test]
    fn clutch_splits_the_budget_between_offspring() {
        let config = EvolutionConfig::default();
        let schema = genes::schema(config.mutation.size).unwrap();
        let mut g = Genome::defaults(&schema);
        g.set_normalised(CLUTCH, schema.genes()[CLUTCH].normalise(4.0)).unwrap();
        let p = Phenotype::express(&g, &schema);
        let o = organism(g, 40.0, 200);
        let c = clutch((&o, &p), None, &config);
        assert_eq!(c.count, 4);
        assert!((c.cost_first - 20.0).abs() < 1e-4);
        assert!((c.energy_each - 20.0 * 0.85 / 4.0).abs() < 1e-4);
        let q = Phenotype { clutch: 1, ..p };
        let mate = organism(Genome::defaults(&schema), 30.0, 200);
        let pair = clutch((&o, &p), Some((&mate, &q)), &config);
        assert_eq!(pair.count, 3, "rounded mean of 4 and 1");
        assert!((pair.energy_each * 3.0 - (20.0 + 15.0) * 0.85).abs() < 1e-3);
    }
}
