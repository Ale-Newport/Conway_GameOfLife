//! Heritable variation of a few traits through an `organism-engine`
//! [`Genome`]. All species share one schema; genes are multipliers or
//! offsets applied to the species values, so selection acts on variation
//! around each species' data. Trade-offs come from the physiology: larger
//! bodies burn more (Kleiber), faster sprinters pay `speed²` more for
//! locomotion, keener vision costs basal metabolism, larger litters split a
//! fixed reproductive budget into smaller offspring.

use organism_engine::{GeneSpec, Genome, GenomeSchema, MutationBounds};
use simulation_core::SimRng;

use crate::error::EcosystemError;

/// Index of the body-size multiplier gene.
pub const SIZE: usize = 0;
/// Index of the speed multiplier gene.
pub const SPEED: usize = 1;
/// Index of the vision multiplier gene.
pub const VISION: usize = 2;
/// Index of the boldness offset gene.
pub const BOLDNESS: usize = 3;
/// Index of the thermal-optimum offset gene (°C).
pub const THERMAL: usize = 4;
/// Index of the fecundity multiplier gene.
pub const FECUNDITY: usize = 5;

/// The shared gene schema.
pub fn schema() -> GenomeSchema {
    let genes = vec![
        GeneSpec::new("size", 0.8, 1.2, 1.0).with_mutation_sd(0.05),
        GeneSpec::new("speed", 0.8, 1.2, 1.0).with_mutation_sd(0.05),
        GeneSpec::new("vision", 0.8, 1.2, 1.0).with_mutation_sd(0.05),
        GeneSpec::new("boldness", -0.3, 0.3, 0.0).with_mutation_sd(0.05),
        GeneSpec::new("thermal_shift_c", -4.0, 4.0, 0.0).with_mutation_sd(0.05),
        GeneSpec::new("fecundity", 0.8, 1.2, 1.0).with_mutation_sd(0.05),
    ];
    // The literal schema above is valid; `expect` documents an invariant
    // checked by the unit test, not user input.
    GenomeSchema::new(genes).expect("built-in gene schema is valid")
}

/// Expressed traits of a genome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phenotype {
    /// Adult mass multiplier.
    pub size: f32,
    /// Speed multiplier.
    pub speed: f32,
    /// Vision multiplier.
    pub vision: f32,
    /// Boldness offset.
    pub boldness: f32,
    /// Shift of the thermal niche (°C).
    pub thermal_shift: f32,
    /// Litter-size multiplier.
    pub fecundity: f32,
}

impl Phenotype {
    /// Expresses a genome (genomes are validated against the schema when
    /// created or restored, so the indices are in range).
    pub fn of(genome: &Genome, schema: &GenomeSchema) -> Self {
        let get = |i: usize| schema.gene(i).map_or(1.0, |g| g.express(genome.values().get(i).copied().unwrap_or(0.5)));
        Self {
            size: get(SIZE),
            speed: get(SPEED),
            vision: get(VISION),
            boldness: get(BOLDNESS),
            thermal_shift: get(THERMAL),
            fecundity: get(FECUNDITY),
        }
    }
}

/// A founder genome: defaults plus one mutation pass at rate 1 (natural
/// standing variation).
pub fn founder(schema: &GenomeSchema, rng: &mut SimRng) -> Result<Genome, EcosystemError> {
    let mut genome = Genome::defaults(schema);
    genome.mutate_with(schema, 1.0, MutationBounds::Reflect, rng)?;
    Ok(genome)
}

/// Offspring genome: uniform crossover of two parents (or a clone of one)
/// followed by Gaussian mutation with reflection at the bounds.
pub fn offspring(
    schema: &GenomeSchema,
    mother: &Genome,
    father: Option<&Genome>,
    mutation_rate: f32,
    rng: &mut SimRng,
) -> Result<Genome, EcosystemError> {
    let mut child = match father {
        Some(f) => Genome::crossover_uniform(mother, f, schema, rng)?,
        None => mother.clone(),
    };
    child.mutate_with(schema, mutation_rate, MutationBounds::Reflect, rng)?;
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_valid_and_defaults_express_neutral_values() {
        let s = schema();
        let p = Phenotype::of(&Genome::defaults(&s), &s);
        assert_eq!((p.size, p.speed, p.vision, p.fecundity), (1.0, 1.0, 1.0, 1.0));
        assert_eq!((p.boldness, p.thermal_shift), (0.0, 0.0));
    }

    #[test]
    fn offspring_genes_come_from_parents() {
        let s = schema();
        let mut rng = SimRng::seed_from_u64(1);
        let a = Genome::from_normalised(&s, vec![0.0; 6]).unwrap();
        let b = Genome::from_normalised(&s, vec![1.0; 6]).unwrap();
        let child = offspring(&s, &a, Some(&b), 0.0, &mut rng).unwrap();
        assert!(child.values().iter().all(|v| *v == 0.0 || *v == 1.0));
        let clone = offspring(&s, &a, None, 0.0, &mut rng).unwrap();
        assert_eq!(clone, a);
    }
}
