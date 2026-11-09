//! Population-genetic measurements, recomputed after every tick.
//!
//! Everything here *observes* the population; nothing feeds back into
//! survival or reproduction. Sampling uses a random stream derived from
//! `(seed, tick)` that is independent of the simulation's generator, so
//! measuring never changes a run.
//!
//! * Per gene and species: mean, standard deviation, range, a histogram over
//!   the gene's phenotypic range and **allele-like class frequencies** (the
//!   normalised range cut into `k` equal classes), with the expected
//!   heterozygosity `H = 1 − Σ pᵢ²` of those classes.
//! * **Genetic diversity**: mean pairwise genetic distance between prey (the
//!   organism engine's normalised RMS distance over heritable genes; exact for
//!   small populations, otherwise estimated from `sample_pairs` random pairs).
//! * **Regions** (islands or sectors): population counts and mean genomes;
//!   for the six most populous prey regions, mean between-region distance
//!   `d_AB` and the net divergence `d_AB − (d_A + d_B)/2` (Nei's D_A
//!   analogue); **F_ST** = `(H̄_T − H̄_S) / H̄_T` over heritable genes, with
//!   `H_S` the size-weighted within-region heterozygosity of the classes.
//! * **Lineages**: living organisms per founder (line of first parents).

use std::collections::BTreeMap;

use organism_engine::{AgentId, AgentStore, Genome, GenomeSchema, genetic_distance};
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::config::StatisticsConfig;
use crate::environment::Environment;
use crate::genes::{GENE_COUNT, HERITABLE_COUNT, SIZE};
use crate::organism::{Organism, Species};

/// Random stream tag for statistical sampling.
const STATS_STREAM: u64 = 0x5354_4154_5300_0001;
/// Prey regions compared pairwise.
const MAX_COMPARED_REGIONS: usize = 6;

/// Statistics of one gene slot in one species. The histogram bins split the
/// gene's phenotypic range `[min, max]` of [`crate::genes::GENES`] equally.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GeneStatistics {
    /// Gene index (see [`crate::genes::GENES`]).
    pub gene: usize,
    /// Mean phenotypic value.
    pub mean: f64,
    /// Population standard deviation.
    pub sd: f64,
    /// Smallest value.
    pub min: f64,
    /// Largest value.
    pub max: f64,
    /// Counts in equal bins over the gene's phenotypic range.
    pub histogram: Vec<u32>,
    /// Frequencies of the allele-like value classes.
    pub class_frequencies: Vec<f64>,
    /// Expected heterozygosity of the classes, `1 − Σ p²`.
    pub heterozygosity: f64,
}

/// Statistics of one species.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeciesStatistics {
    /// Living organisms.
    pub count: u32,
    /// One entry per gene slot (heritable genes first, then acquired traits).
    pub genes: Vec<GeneStatistics>,
    /// Mean energy as a fraction of storage.
    pub mean_condition: f64,
    /// Mean generation number.
    pub mean_generation: f64,
    /// Largest generation number.
    pub max_generation: u32,
}

impl SpeciesStatistics {
    /// Mean of gene `index` (0 when the species is absent).
    pub fn mean(&self, index: usize) -> f64 {
        self.genes.get(index).map_or(0.0, |g| g.mean)
    }

    /// Standard deviation of gene `index`.
    pub fn sd(&self, index: usize) -> f64 {
        self.genes.get(index).map_or(0.0, |g| g.sd)
    }
}

/// Population of one region.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionStatistics {
    /// Region label.
    pub region: u16,
    /// Living prey.
    pub prey: u32,
    /// Living predators.
    pub predators: u32,
    /// Mean normalised heritable genes of the region's prey.
    pub mean_genome: Vec<f64>,
}

/// Genetic divergence between the prey of two regions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionDivergence {
    /// First region.
    pub a: u16,
    /// Second region.
    pub b: u16,
    /// Mean genetic distance between an individual of `a` and one of `b`.
    pub between: f64,
    /// `between − (within_a + within_b) / 2`.
    pub net: f64,
}

/// Living descendants of one founder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageCount {
    /// The founder.
    pub founder: AgentId,
    /// Living organisms descending from it through first parents.
    pub living: u32,
}

/// Everything measured at one tick.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PopulationStats {
    /// Tick of the measurement.
    pub tick: u64,
    /// Prey statistics.
    pub prey: SpeciesStatistics,
    /// Predator statistics.
    pub predators: SpeciesStatistics,
    /// Mean pairwise genetic distance among prey.
    pub genetic_diversity: f64,
    /// Mean class heterozygosity over heritable prey genes.
    pub heterozygosity: f64,
    /// Fixation index between prey regions.
    pub fst: f64,
    /// Regions with at least one organism, by label.
    pub regions: Vec<RegionStatistics>,
    /// Pairwise divergence between the most populous prey regions.
    pub divergence: Vec<RegionDivergence>,
    /// Organisms standing outside every region (on a barrier switched on under them).
    pub unassigned: u32,
    /// Living organisms per founder, by founder id.
    pub lineages: Vec<LineageCount>,
}

impl PopulationStats {
    /// Largest net divergence between two prey regions (0 if fewer than two).
    pub fn max_net_divergence(&self) -> f64 {
        self.divergence.iter().map(|d| d.net).fold(0.0, f64::max)
    }
}

/// Inputs of [`compute`].
pub struct StatsInput<'a> {
    /// The organisms.
    pub store: &'a AgentStore<Organism>,
    /// The environment (regions).
    pub env: &'a Environment,
    /// Genome schema.
    pub schema: &'a GenomeSchema,
    /// Settings.
    pub config: &'a StatisticsConfig,
    /// Energy storage per unit body mass (for condition).
    pub storage: f32,
    /// Current tick.
    pub tick: u64,
    /// Run seed (for the sampling stream).
    pub seed: u64,
}

/// Measures the population.
pub fn compute(input: &StatsInput) -> PopulationStats {
    let mut rng = SimRng::derive(input.seed ^ STATS_STREAM, input.tick);
    let agents = input.store.as_slice();
    let slots_of = |sp: Species| agents.iter().enumerate().filter(|(_, o)| o.species == sp).map(|(i, _)| i).collect();
    let prey_slots: Vec<usize> = slots_of(Species::Prey);
    let predator_slots: Vec<usize> = slots_of(Species::Predator);
    let prey = species_statistics(agents, &prey_slots, input);
    let predators = species_statistics(agents, &predator_slots, input);
    let genomes: Vec<&Genome> = prey_slots.iter().map(|&i| &agents[i].genome).collect();
    let genetic_diversity = mean_distance_within(&genomes, input.schema, input.config.sample_pairs as usize, &mut rng);
    let heterozygosity = if prey.count == 0 {
        0.0
    } else {
        prey.genes[..HERITABLE_COUNT].iter().map(|g| g.heterozygosity).sum::<f64>() / HERITABLE_COUNT as f64
    };

    // Regions.
    let mut by_region: BTreeMap<u16, (Vec<usize>, u32)> = BTreeMap::new();
    let mut unassigned = 0;
    for (i, o) in agents.iter().enumerate() {
        match input.env.region_at(o.kinematics.position) {
            Some(r) => {
                let entry = by_region.entry(r).or_default();
                match o.species {
                    Species::Prey => entry.0.push(i),
                    Species::Predator => entry.1 += 1,
                }
            }
            None => unassigned += 1,
        }
    }
    let regions: Vec<RegionStatistics> = by_region
        .iter()
        .map(|(&region, (prey_in, predators_in))| RegionStatistics {
            region,
            prey: prey_in.len() as u32,
            predators: *predators_in,
            mean_genome: mean_genome(agents, prey_in),
        })
        .collect();
    let fst = fixation_index(agents, &by_region, input.config.allele_classes as usize);
    let divergence = region_divergence(agents, &by_region, input, &mut rng);

    let mut lineages: BTreeMap<AgentId, u32> = BTreeMap::new();
    for o in agents {
        *lineages.entry(o.founder).or_default() += 1;
    }
    PopulationStats {
        tick: input.tick,
        prey,
        predators,
        genetic_diversity,
        heterozygosity,
        fst,
        regions,
        divergence,
        unassigned,
        lineages: lineages.into_iter().map(|(founder, living)| LineageCount { founder, living }).collect(),
    }
}

/// Allele-like class of a normalised value.
fn class_of(normalised: f32, classes: usize) -> usize {
    ((normalised * classes as f32) as usize).min(classes - 1)
}

fn species_statistics(agents: &[Organism], slots: &[usize], input: &StatsInput) -> SpeciesStatistics {
    if slots.is_empty() {
        return SpeciesStatistics::default();
    }
    let bins = input.config.histogram_bins.max(1) as usize;
    let classes = input.config.allele_classes.max(1) as usize;
    let n = slots.len() as f64;
    let genes = (0..GENE_COUNT)
        .map(|gene| {
            let mut histogram = vec![0u32; bins];
            let mut class_counts = vec![0u32; classes];
            let (mut sum, mut sum_sq, mut min, mut max) = (0.0f64, 0.0f64, f64::INFINITY, f64::NEG_INFINITY);
            for &i in slots {
                let normalised = agents[i].genome.values()[gene];
                let value = f64::from(agents[i].genome.express(input.schema, gene));
                sum += value;
                sum_sq += value * value;
                min = min.min(value);
                max = max.max(value);
                histogram[class_of(normalised, bins)] += 1;
                class_counts[class_of(normalised, classes)] += 1;
            }
            let mean = sum / n;
            let class_frequencies: Vec<f64> = class_counts.iter().map(|&c| f64::from(c) / n).collect();
            GeneStatistics {
                gene,
                mean,
                sd: (sum_sq / n - mean * mean).max(0.0).sqrt(),
                min,
                max,
                histogram,
                heterozygosity: 1.0 - class_frequencies.iter().map(|p| p * p).sum::<f64>(),
                class_frequencies,
            }
        })
        .collect();
    let mean_condition = slots
        .iter()
        .map(|&i| {
            let o = &agents[i];
            f64::from(o.energy) / f64::from(input.storage * o.genome.express(input.schema, SIZE))
        })
        .sum::<f64>()
        / n;
    SpeciesStatistics {
        count: slots.len() as u32,
        genes,
        mean_condition,
        mean_generation: slots.iter().map(|&i| f64::from(agents[i].lineage.generation)).sum::<f64>() / n,
        max_generation: slots.iter().map(|&i| agents[i].lineage.generation).max().unwrap_or(0),
    }
}

fn mean_genome(agents: &[Organism], slots: &[usize]) -> Vec<f64> {
    let mut mean = vec![0.0; HERITABLE_COUNT];
    if slots.is_empty() {
        return mean;
    }
    for &i in slots {
        for (m, v) in mean.iter_mut().zip(agents[i].genome.values()) {
            *m += f64::from(*v);
        }
    }
    mean.iter_mut().for_each(|m| *m /= slots.len() as f64);
    mean
}

/// Genetic distance; genomes are checked against the schema on creation and
/// restore, so the length check of the organism engine cannot fail here.
fn distance(a: &Genome, b: &Genome, schema: &GenomeSchema) -> f64 {
    genetic_distance(a, b, schema).map_or(0.0, f64::from)
}

/// Mean distance over all unordered pairs, exact when there are at most
/// `budget` pairs, otherwise estimated from `budget` random pairs.
pub(crate) fn mean_distance_within(genomes: &[&Genome], schema: &GenomeSchema, budget: usize, rng: &mut SimRng) -> f64 {
    let n = genomes.len();
    if n < 2 {
        return 0.0;
    }
    let pairs = n * (n - 1) / 2;
    if pairs <= budget {
        let mut sum = 0.0;
        for i in 0..n {
            for j in i + 1..n {
                sum += distance(genomes[i], genomes[j], schema);
            }
        }
        return sum / pairs as f64;
    }
    let mut sum = 0.0;
    for _ in 0..budget {
        let i = rng.index(n);
        let j = (i + 1 + rng.index(n - 1)) % n;
        sum += distance(genomes[i], genomes[j], schema);
    }
    sum / budget as f64
}

/// Mean distance between members of two groups (exact or sampled).
pub(crate) fn mean_distance_between(
    a: &[&Genome],
    b: &[&Genome],
    schema: &GenomeSchema,
    budget: usize,
    rng: &mut SimRng,
) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a.len() * b.len() <= budget {
        let sum: f64 = a.iter().flat_map(|x| b.iter().map(move |y| distance(x, y, schema))).sum();
        return sum / (a.len() * b.len()) as f64;
    }
    let sum: f64 = (0..budget).map(|_| distance(a[rng.index(a.len())], b[rng.index(b.len())], schema)).sum();
    sum / budget as f64
}

fn fixation_index(agents: &[Organism], by_region: &BTreeMap<u16, (Vec<usize>, u32)>, classes: usize) -> f64 {
    let groups: Vec<&Vec<usize>> = by_region.values().map(|(prey, _)| prey).filter(|p| p.len() >= 2).collect();
    if groups.len() < 2 {
        return 0.0;
    }
    let total: usize = groups.iter().map(|g| g.len()).sum();
    let (mut h_t, mut h_s) = (0.0, 0.0);
    for gene in 0..HERITABLE_COUNT {
        let mut pooled = vec![0.0; classes];
        for group in &groups {
            let w = group.len() as f64 / total as f64;
            let mut counts = vec![0.0; classes];
            for &i in group.iter() {
                counts[class_of(agents[i].genome.values()[gene], classes)] += 1.0;
            }
            let freqs: Vec<f64> = counts.iter().map(|c| c / group.len() as f64).collect();
            h_s += w * (1.0 - freqs.iter().map(|p| p * p).sum::<f64>());
            for (p, f) in pooled.iter_mut().zip(&freqs) {
                *p += w * f;
            }
        }
        h_t += 1.0 - pooled.iter().map(|p| p * p).sum::<f64>();
    }
    if h_t <= 0.0 { 0.0 } else { ((h_t - h_s) / h_t).max(0.0) }
}

fn region_divergence(
    agents: &[Organism],
    by_region: &BTreeMap<u16, (Vec<usize>, u32)>,
    input: &StatsInput,
    rng: &mut SimRng,
) -> Vec<RegionDivergence> {
    let mut ranked: Vec<(u16, Vec<&Genome>)> = by_region
        .iter()
        .filter(|(_, (prey, _))| prey.len() >= 2)
        .map(|(&r, (prey, _))| (r, prey.iter().map(|&i| &agents[i].genome).collect()))
        .collect();
    // Most populous first; ties by label (stable sort keeps label order).
    ranked.sort_by_key(|(_, genomes)| std::cmp::Reverse(genomes.len()));
    ranked.truncate(MAX_COMPARED_REGIONS);
    ranked.sort_by_key(|(r, _)| *r);
    let k = ranked.len();
    if k < 2 {
        return Vec::new();
    }
    let budget = (input.config.sample_pairs as usize / (k + k * (k - 1) / 2)).max(20);
    let within: Vec<f64> = ranked.iter().map(|(_, g)| mean_distance_within(g, input.schema, budget, rng)).collect();
    let mut out = Vec::new();
    for x in 0..k {
        for y in x + 1..k {
            let between = mean_distance_between(&ranked[x].1, &ranked[y].1, input.schema, budget, rng);
            out.push(RegionDivergence {
                a: ranked[x].0,
                b: ranked[y].0,
                between,
                net: between - 0.5 * (within[x] + within[y]),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genes;

    #[test]
    fn exact_and_sampled_pairwise_distances_agree() {
        let schema = genes::schema(0.05).unwrap();
        let mut rng = SimRng::seed_from_u64(4);
        let genomes: Vec<Genome> = (0..60).map(|_| Genome::random(&schema, &mut rng)).collect();
        let refs: Vec<&Genome> = genomes.iter().collect();
        let exact = mean_distance_within(&refs, &schema, 10_000, &mut rng);
        let sampled = mean_distance_within(&refs, &schema, 3000, &mut rng);
        assert!((exact - sampled).abs() < 0.01, "{exact} vs {sampled}");
        // Uniform genes: E|x − y|² = 1/6 per gene, so the RMS distance is close to sqrt(1/6).
        assert!((exact - (1.0f64 / 6.0).sqrt()).abs() < 0.03, "{exact}");
        let clones = vec![&genomes[0]; 5];
        assert_eq!(mean_distance_within(&clones, &schema, 100, &mut rng), 0.0);
        let between = mean_distance_between(&refs[..30], &refs[30..], &schema, 2000, &mut rng);
        assert!((between - exact).abs() < 0.02);
    }

    #[test]
    fn classes_cover_the_unit_interval() {
        assert_eq!(class_of(0.0, 4), 0);
        assert_eq!(class_of(0.2499, 4), 0);
        assert_eq!(class_of(0.25, 4), 1);
        assert_eq!(class_of(1.0, 4), 3);
    }
}
