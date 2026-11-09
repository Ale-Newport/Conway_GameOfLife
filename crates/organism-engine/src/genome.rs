//! Genomes: normalised gene values interpreted through a schema.
//!
//! A [`GenomeSchema`] declares the genes of a species (name, phenotypic range,
//! default, mutation size, heritability). A [`Genome`] stores one value per
//! gene, *normalised* to `[0, 1]`; [`Genome::express`] maps it linearly to the
//! gene's `[min, max]` range. Working in normalised space makes mutation sizes
//! comparable across genes (`mutation_sd` is a fraction of the range) and lets
//! [`genetic_distance`] weigh all genes equally.
//!
//! **Heritability.** Non-heritable genes model acquired traits stored next to
//! the genes (e.g. learned skills): they are not mutated, not passed on by
//! crossover (offspring start from the gene default), not randomised by
//! [`Genome::random`], ignored by [`genetic_distance`], and may be changed
//! during life with [`Genome::set_normalised`].
//!
//! All operations draw randomness only from the [`SimRng`] passed in, in a
//! documented order, so offspring are reproducible.

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::error::OrganismError;

/// Default `mutation_sd` when a data file omits it (5 % of the range).
pub const DEFAULT_MUTATION_SD: f32 = 0.05;

fn default_mutation_sd() -> f32 {
    DEFAULT_MUTATION_SD
}

fn default_heritable() -> bool {
    true
}

/// Clamps to `[0, 1]`, mapping NaN to `0`.
fn unit(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// Declaration of one gene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeneSpec {
    /// Unique name, e.g. `"max_speed"`.
    pub name: String,
    /// Phenotypic value expressed by a normalised value of 0.
    pub min: f32,
    /// Phenotypic value expressed by a normalised value of 1 (`max >= min`;
    /// `max == min` declares a constant trait).
    pub max: f32,
    /// Phenotypic default in `[min, max]` (used for founders without
    /// variation and for non-heritable genes of offspring).
    pub default: f32,
    /// Standard deviation of a Gaussian mutation step, as a fraction of the
    /// range (`0.05` = 5 % of `max − min`). Defaults to [`DEFAULT_MUTATION_SD`].
    #[serde(default = "default_mutation_sd")]
    pub mutation_sd: f32,
    /// Whether the gene is inherited (defaults to `true`); see the module docs.
    #[serde(default = "default_heritable")]
    pub heritable: bool,
}

impl GeneSpec {
    /// Heritable gene with the default mutation size (not validated until it
    /// is added to a [`GenomeSchema`]).
    pub fn new(name: &str, min: f32, max: f32, default: f32) -> Self {
        Self { name: name.to_string(), min, max, default, mutation_sd: DEFAULT_MUTATION_SD, heritable: true }
    }

    /// Sets the mutation standard deviation (fraction of the range).
    pub fn with_mutation_sd(mut self, mutation_sd: f32) -> Self {
        self.mutation_sd = mutation_sd;
        self
    }

    /// Marks the gene as non-heritable (an acquired trait).
    pub fn non_heritable(mut self) -> Self {
        self.heritable = false;
        self
    }

    /// Checks the declaration.
    pub fn validate(&self) -> Result<(), OrganismError> {
        let fail = |reason: String| Err(OrganismError::InvalidGene { gene: self.name.clone(), reason });
        if self.name.trim().is_empty() {
            return fail("the gene name is empty".into());
        }
        if !(self.min.is_finite() && self.max.is_finite() && self.default.is_finite()) {
            return fail(format!("min {}, max {} and default {} must be finite", self.min, self.max, self.default));
        }
        if self.max < self.min {
            return fail(format!("max {} is below min {}", self.max, self.min));
        }
        if !(self.min..=self.max).contains(&self.default) {
            return fail(format!("default {} is outside [{}, {}]", self.default, self.min, self.max));
        }
        if !(self.mutation_sd.is_finite() && self.mutation_sd >= 0.0) {
            return fail(format!("mutation_sd {} must be a finite number >= 0", self.mutation_sd));
        }
        Ok(())
    }

    /// Width of the phenotypic range, `max − min`.
    pub fn range(&self) -> f32 {
        self.max - self.min
    }

    /// Maps a phenotypic value to `[0, 1]` (clamped; constant genes map to 0).
    pub fn normalise(&self, value: f32) -> f32 {
        let range = self.range();
        if range > 0.0 { unit((value - self.min) / range) } else { 0.0 }
    }

    /// Maps a normalised value (clamped to `[0, 1]`) to `[min, max]`.
    pub fn express(&self, normalised: f32) -> f32 {
        // `max`/`min` instead of `clamp`: never panics on an unvalidated spec.
        (self.min + unit(normalised) * self.range()).max(self.min).min(self.max)
    }
}

/// Validated list of gene declarations (unique names, consistent ranges).
/// Serialised as a plain list of [`GeneSpec`]s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<GeneSpec>", into = "Vec<GeneSpec>")]
pub struct GenomeSchema {
    genes: Vec<GeneSpec>,
}

impl GenomeSchema {
    /// Validates every gene and the uniqueness of names.
    pub fn new(genes: Vec<GeneSpec>) -> Result<Self, OrganismError> {
        for (i, gene) in genes.iter().enumerate() {
            gene.validate()?;
            if genes[..i].iter().any(|g| g.name == gene.name) {
                return Err(OrganismError::DuplicateGene(gene.name.clone()));
            }
        }
        Ok(Self { genes })
    }

    /// The declarations, in gene index order.
    pub fn genes(&self) -> &[GeneSpec] {
        &self.genes
    }

    /// Number of genes.
    pub fn len(&self) -> usize {
        self.genes.len()
    }

    /// Whether the schema declares no genes.
    pub fn is_empty(&self) -> bool {
        self.genes.is_empty()
    }

    /// Index of the gene called `name`.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.genes.iter().position(|g| g.name == name)
    }

    /// Declaration of gene `index`.
    pub fn gene(&self, index: usize) -> Option<&GeneSpec> {
        self.genes.get(index)
    }
}

impl TryFrom<Vec<GeneSpec>> for GenomeSchema {
    type Error = OrganismError;

    fn try_from(genes: Vec<GeneSpec>) -> Result<Self, Self::Error> {
        Self::new(genes)
    }
}

impl From<GenomeSchema> for Vec<GeneSpec> {
    fn from(schema: GenomeSchema) -> Self {
        schema.genes
    }
}

/// What happens when a mutation step leaves the normalised range `[0, 1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationBounds {
    /// Clamp to the nearest bound. Simple, but probability mass piles up
    /// exactly on the bounds (values of exactly 0 or 1 become common).
    #[default]
    Clamp,
    /// Fold back into the range like a mirror (`−x` below 0, `2 − x` above 1,
    /// repeated for large steps). Avoids the pile-up at the bounds.
    Reflect,
}

impl MutationBounds {
    /// Maps any value into `[0, 1]` (NaN becomes 0).
    pub fn apply(self, x: f32) -> f32 {
        match self {
            Self::Clamp => unit(x),
            Self::Reflect if x.is_finite() => {
                let t = x.rem_euclid(2.0);
                unit(if t > 1.0 { 2.0 - t } else { t })
            }
            Self::Reflect => unit(x),
        }
    }
}

/// Recombination operator, for data-driven configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Crossover {
    /// [`Genome::crossover_uniform`].
    #[default]
    Uniform,
    /// [`Genome::crossover_one_point`].
    OnePoint,
}

/// Normalised gene values (one per schema gene, each a finite number in
/// `[0, 1]`). Serialised as a plain list; deserialisation rejects values
/// outside `[0, 1]`, and [`Genome::check`] verifies the length against a schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<f32>", into = "Vec<f32>")]
pub struct Genome {
    values: Vec<f32>,
}

impl TryFrom<Vec<f32>> for Genome {
    type Error = OrganismError;

    fn try_from(values: Vec<f32>) -> Result<Self, Self::Error> {
        if let Some((index, &value)) = values.iter().enumerate().find(|(_, v)| !(0.0..=1.0).contains(*v)) {
            return Err(OrganismError::InvalidGeneValue { index, value });
        }
        Ok(Self { values })
    }
}

impl From<Genome> for Vec<f32> {
    fn from(genome: Genome) -> Self {
        genome.values
    }
}

fn check_len(schema: &GenomeSchema, genome: &Genome) -> Result<(), OrganismError> {
    if genome.values.len() == schema.len() {
        Ok(())
    } else {
        Err(OrganismError::GenomeLength { expected: schema.len(), found: genome.values.len() })
    }
}

impl Genome {
    /// Every gene at its default.
    pub fn defaults(schema: &GenomeSchema) -> Self {
        Self { values: schema.genes.iter().map(|g| g.normalise(g.default)).collect() }
    }

    /// Heritable genes uniform in `[0, 1]` (one draw each, in gene order);
    /// non-heritable genes at their default.
    pub fn random(schema: &GenomeSchema, rng: &mut SimRng) -> Self {
        let values =
            schema.genes.iter().map(|g| if g.heritable { rng.f32() } else { g.normalise(g.default) }).collect();
        Self { values }
    }

    /// Genome from normalised values, validated against `schema`.
    pub fn from_normalised(schema: &GenomeSchema, values: Vec<f32>) -> Result<Self, OrganismError> {
        let genome = Self::try_from(values)?;
        check_len(schema, &genome)?;
        Ok(genome)
    }

    /// The normalised values in gene order.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Number of genes.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the genome is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Verifies that the genome has one value per gene of `schema` (use after
    /// deserialising a genome stored separately from its schema).
    pub fn check(&self, schema: &GenomeSchema) -> Result<(), OrganismError> {
        check_len(schema, self)
    }

    /// Phenotypic value of gene `index`, in `[min, max]`.
    ///
    /// # Panics
    /// Panics if `index` is out of range for the schema or the genome, like
    /// slice indexing. Gene indices are normally constants of the model; use
    /// [`Genome::express_named`] for names coming from data.
    pub fn express(&self, schema: &GenomeSchema, index: usize) -> f32 {
        schema.genes[index].express(self.values[index])
    }

    /// Phenotypic value of the gene called `name`, if the schema has it and
    /// the genome is long enough.
    pub fn express_named(&self, schema: &GenomeSchema, name: &str) -> Option<f32> {
        let index = schema.index_of(name)?;
        Some(schema.genes[index].express(*self.values.get(index)?))
    }

    /// Overwrites gene `index` with a normalised value (e.g. an acquired trait
    /// or an edit from the inspector).
    pub fn set_normalised(&mut self, index: usize, value: f32) -> Result<(), OrganismError> {
        if !(0.0..=1.0).contains(&value) {
            return Err(OrganismError::InvalidGeneValue { index, value });
        }
        let len = self.values.len();
        let slot = self.values.get_mut(index).ok_or(OrganismError::GeneIndex { index, len })?;
        *slot = value;
        Ok(())
    }

    /// Gaussian point mutation with clamping (see [`Genome::mutate_with`]):
    /// for each heritable gene, with probability `rate`,
    /// `v ← clamp(v + N(0, mutation_sd), 0, 1)`. Returns the number of
    /// mutated genes.
    pub fn mutate(&mut self, schema: &GenomeSchema, rate: f32, rng: &mut SimRng) -> Result<usize, OrganismError> {
        self.mutate_with(schema, rate, MutationBounds::Clamp, rng)
    }

    /// Gaussian point mutation. For each heritable gene in order, with
    /// probability `rate` (clamped to `[0, 1]`, NaN counts as 0):
    /// `v ← bounds(v + N(0, mutation_sd))`. Draws one uniform number per
    /// heritable gene when `0 < rate < 1`, plus two (Box–Muller) for each
    /// mutated gene. Returns the number of mutated genes.
    pub fn mutate_with(
        &mut self,
        schema: &GenomeSchema,
        rate: f32,
        bounds: MutationBounds,
        rng: &mut SimRng,
    ) -> Result<usize, OrganismError> {
        check_len(schema, self)?;
        // `SimRng::chance` draws nothing for p <= 0 or p >= 1, but would draw for NaN.
        let rate = if rate.is_nan() { 0.0 } else { f64::from(rate) };
        let mut mutated = 0;
        for (value, gene) in self.values.iter_mut().zip(&schema.genes) {
            if gene.heritable && rng.chance(rate) {
                let step = gene.mutation_sd * rng.normal() as f32;
                *value = bounds.apply(*value + step);
                mutated += 1;
            }
        }
        Ok(mutated)
    }

    /// Uniform crossover: each heritable gene comes from `a` or `b` with
    /// probability ½ (one 64-bit draw per heritable gene, in gene order);
    /// non-heritable genes start at their default.
    pub fn crossover_uniform(
        a: &Genome,
        b: &Genome,
        schema: &GenomeSchema,
        rng: &mut SimRng,
    ) -> Result<Genome, OrganismError> {
        check_len(schema, a)?;
        check_len(schema, b)?;
        let values = schema
            .genes
            .iter()
            .enumerate()
            .map(|(i, g)| {
                if !g.heritable {
                    g.normalise(g.default)
                } else if rng.next_u64() >> 63 == 0 {
                    a.values[i]
                } else {
                    b.values[i]
                }
            })
            .collect();
        Ok(Genome { values })
    }

    /// One-point crossover: a cut `c` is drawn uniformly from `1..n` (one
    /// draw, only when `n >= 2`); heritable genes before `c` come from `a`,
    /// the rest from `b`. With fewer than two genes the child copies `a`.
    /// Non-heritable genes start at their default.
    pub fn crossover_one_point(
        a: &Genome,
        b: &Genome,
        schema: &GenomeSchema,
        rng: &mut SimRng,
    ) -> Result<Genome, OrganismError> {
        check_len(schema, a)?;
        check_len(schema, b)?;
        let n = schema.len();
        let cut = if n >= 2 { 1 + rng.index(n - 1) } else { n };
        let values = schema
            .genes
            .iter()
            .enumerate()
            .map(|(i, g)| match (g.heritable, i < cut) {
                (false, _) => g.normalise(g.default),
                (true, true) => a.values[i],
                (true, false) => b.values[i],
            })
            .collect();
        Ok(Genome { values })
    }

    /// Dispatches to the chosen [`Crossover`] operator.
    pub fn crossover(
        a: &Genome,
        b: &Genome,
        schema: &GenomeSchema,
        operator: Crossover,
        rng: &mut SimRng,
    ) -> Result<Genome, OrganismError> {
        match operator {
            Crossover::Uniform => Self::crossover_uniform(a, b, schema, rng),
            Crossover::OnePoint => Self::crossover_one_point(a, b, schema, rng),
        }
    }
}

/// Gaussian point mutation, `genome::mutate` of the cross-crate contract
/// (`ARCHITECTURE.md` §4). Same as [`Genome::mutate_with`]: each heritable
/// gene mutates with probability `rate`, `v ← bounds(v + N(0, mutation_sd))`;
/// returns the number of mutated genes.
pub fn mutate(
    genome: &mut Genome,
    schema: &GenomeSchema,
    rate: f32,
    bounds: MutationBounds,
    rng: &mut SimRng,
) -> Result<usize, OrganismError> {
    genome.mutate_with(schema, rate, bounds, rng)
}

/// Recombination of two parents, `genome::crossover` of the cross-crate
/// contract (`ARCHITECTURE.md` §4). Same as [`Genome::crossover`].
pub fn crossover(
    a: &Genome,
    b: &Genome,
    schema: &GenomeSchema,
    operator: Crossover,
    rng: &mut SimRng,
) -> Result<Genome, OrganismError> {
    Genome::crossover(a, b, schema, operator, rng)
}

/// Normalised Euclidean distance over the heritable genes:
/// `sqrt(Σ (a_i − b_i)² / m)` where `m` is the number of heritable genes.
/// The result is in `[0, 1]` (0 = identical, 1 = opposite corners of the
/// gene space) and is `0` when the schema has no heritable genes.
pub fn genetic_distance(a: &Genome, b: &Genome, schema: &GenomeSchema) -> Result<f32, OrganismError> {
    check_len(schema, a)?;
    check_len(schema, b)?;
    let (sum, m) = schema
        .genes
        .iter()
        .zip(a.values.iter().zip(&b.values))
        .filter(|(g, _)| g.heritable)
        .fold((0.0f32, 0u32), |(s, m), (_, (x, y))| (s + (x - y) * (x - y), m + 1));
    Ok(if m == 0 { 0.0 } else { (sum / m as f32).sqrt() })
}

#[cfg(test)]
mod tests;
