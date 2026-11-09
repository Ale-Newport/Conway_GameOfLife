//! The genome of the evolution laboratory: which genes exist, how they are
//! expressed and how founders are generated.
//!
//! Genes are stored by [`organism_engine::Genome`] as values normalised to
//! `[0, 1]` and expressed linearly into each gene's phenotypic range. Thirteen
//! genes are **heritable** (copied, recombined and mutated at birth). Two
//! slots hold **acquired traits** — thermal acclimation and learned predator
//! wariness — declared non-heritable in the schema, so the organism engine
//! never mutates them, never passes them on (offspring start at the default)
//! and ignores them in genetic distances. They change during an organism's
//! life only.

use organism_engine::{GeneSpec, Genome, GenomeSchema, MutationBounds};
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::error::EvolutionError;

/// Maximum speed (world units per tick).
pub const SPEED: usize = 0;
/// Perception radius (world units).
pub const PERCEPTION: usize = 1;
/// Body size (mass units).
pub const SIZE: usize = 2;
/// Metabolic efficiency (yield; trades off against intake rate).
pub const EFFICIENCY: usize = 3;
/// Thermal optimum (°C).
pub const TEMP_OPTIMUM: usize = 4;
/// Thermal tolerance breadth (°C).
pub const TEMP_TOLERANCE: usize = 5;
/// Offspring per breeding event.
pub const CLUTCH: usize = 6;
/// Fraction of the energy reserve spent on one breeding event.
pub const EFFORT: usize = 7;
/// Age at maturity (ticks).
pub const MATURITY: usize = 8;
/// Boldness: prey flee later, predators hunt only when hungrier.
pub const BOLDNESS: usize = 9;
/// Mate choosiness (minimum condition demanded from a partner).
pub const CHOOSINESS: usize = 10;
/// Neutral marker A (no phenotypic effect).
pub const MARKER_A: usize = 11;
/// Neutral marker B (no phenotypic effect).
pub const MARKER_B: usize = 12;
/// Acquired: thermal acclimation offset (°C), non-heritable.
pub const ACCLIMATION: usize = 13;
/// Acquired: learned predator wariness, non-heritable.
pub const EXPERIENCE: usize = 14;
/// Number of gene slots (heritable and acquired).
pub const GENE_COUNT: usize = 15;
/// Number of heritable genes (slots `0..HERITABLE_COUNT`).
pub const HERITABLE_COUNT: usize = 13;

/// Static description of one gene slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneInfo {
    /// Machine name used in scenario files, commands and metric keys.
    pub name: &'static str,
    /// Display label.
    pub label: &'static str,
    /// Unit of the phenotypic value.
    pub unit: &'static str,
    /// Phenotype at normalised value 0.
    pub min: f32,
    /// Phenotype at normalised value 1.
    pub max: f32,
    /// Default phenotype (founders without overrides, acquired traits at birth).
    pub default: f32,
    /// Whether the slot is inherited.
    pub heritable: bool,
    /// What the gene does in the model.
    pub description: &'static str,
}

/// All gene slots, in index order.
pub const GENES: [GeneInfo; GENE_COUNT] = [
    GeneInfo {
        name: "speed",
        label: "Speed",
        unit: "units/tick",
        min: 0.2,
        max: 1.6,
        default: 0.7,
        heritable: true,
        description: "Maximum speed; movement cost grows with size × speed².",
    },
    GeneInfo {
        name: "perception",
        label: "Perception radius",
        unit: "units",
        min: 2.0,
        max: 14.0,
        default: 6.0,
        heritable: true,
        description: "Detection radius for food, mates, prey and predators; costs energy linearly.",
    },
    GeneInfo {
        name: "size",
        label: "Body size",
        unit: "mass",
        min: 0.5,
        max: 3.0,
        default: 1.0,
        heritable: true,
        description: "Mass: storage ∝ M, basal cost and intake ∝ M^0.75, movement cost ∝ M, refuge from predators.",
    },
    GeneInfo {
        name: "efficiency",
        label: "Metabolic efficiency",
        unit: "×",
        min: 0.5,
        max: 1.5,
        default: 1.0,
        heritable: true,
        description: "Divides basal cost but also maximum intake per tick (rate–yield trade-off).",
    },
    GeneInfo {
        name: "temp_optimum",
        label: "Thermal optimum",
        unit: "°C",
        min: -5.0,
        max: 40.0,
        default: 20.0,
        heritable: true,
        description: "Temperature of best performance.",
    },
    GeneInfo {
        name: "temp_tolerance",
        label: "Thermal tolerance",
        unit: "°C",
        min: 1.0,
        max: 12.0,
        default: 5.0,
        heritable: true,
        description: "Breadth of the thermal niche; broad niches raise basal cost (generalist cost).",
    },
    GeneInfo {
        name: "clutch",
        label: "Clutch size",
        unit: "offspring",
        min: 1.0,
        max: 8.0,
        default: 2.0,
        heritable: true,
        description: "Offspring per breeding event (rounded); the breeding budget is split among them.",
    },
    GeneInfo {
        name: "effort",
        label: "Reproductive effort",
        unit: "fraction",
        min: 0.2,
        max: 0.8,
        default: 0.5,
        heritable: true,
        description: "Fraction of the energy reserve spent on one breeding event.",
    },
    GeneInfo {
        name: "maturity",
        label: "Age at maturity",
        unit: "ticks",
        min: 30.0,
        max: 300.0,
        default: 100.0,
        heritable: true,
        description: "First breeding age; the senescence time scale is proportional to it.",
    },
    GeneInfo {
        name: "boldness",
        label: "Boldness",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        heritable: true,
        description: "Prey: flee distance = (1 − boldness) × perception. Predators: hunt below (0.3 + 0.6 boldness) of storage.",
    },
    GeneInfo {
        name: "choosiness",
        label: "Mate choosiness",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.2,
        heritable: true,
        description: "Minimum condition (energy fraction) demanded from a mate, between the breeding threshold and 1.",
    },
    GeneInfo {
        name: "marker_a",
        label: "Neutral marker A",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        heritable: true,
        description: "No phenotypic effect: changes only by drift, hitch-hiking and mutation.",
    },
    GeneInfo {
        name: "marker_b",
        label: "Neutral marker B",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        heritable: true,
        description: "No phenotypic effect: changes only by drift, hitch-hiking and mutation.",
    },
    GeneInfo {
        name: "acclimation",
        label: "Thermal acclimation",
        unit: "°C",
        min: -4.0,
        max: 4.0,
        default: 0.0,
        heritable: false,
        description: "Acquired: shifts the effective optimum towards experienced temperatures; reset to 0 at birth.",
    },
    GeneInfo {
        name: "experience",
        label: "Predator experience",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        heritable: false,
        description: "Acquired: grows after escapes, decays otherwise, widens the flee distance; reset to 0 at birth.",
    },
];

/// Names of every gene slot, in index order.
pub fn gene_names() -> Vec<&'static str> {
    GENES.iter().map(|g| g.name).collect()
}

/// Index of the gene called `name`.
pub fn gene_index(name: &str) -> Option<usize> {
    GENES.iter().position(|g| g.name == name)
}

/// Builds the genome schema; every heritable gene mutates with standard
/// deviation `mutation_size` (fraction of its range).
pub fn schema(mutation_size: f32) -> Result<GenomeSchema, EvolutionError> {
    let specs = GENES
        .iter()
        .map(|g| {
            let spec = GeneSpec::new(g.name, g.min, g.max, g.default);
            if g.heritable { spec.with_mutation_sd(mutation_size) } else { spec.with_mutation_sd(0.0).non_heritable() }
        })
        .collect();
    Ok(GenomeSchema::new(specs)?)
}

/// Expressed (phenotypic) values of one genome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phenotype {
    /// Maximum speed.
    pub speed: f32,
    /// Perception radius.
    pub perception: f32,
    /// Body size.
    pub size: f32,
    /// Metabolic efficiency.
    pub efficiency: f32,
    /// Thermal optimum (°C), without acclimation.
    pub temp_optimum: f32,
    /// Thermal tolerance breadth (°C).
    pub temp_tolerance: f32,
    /// Offspring per breeding event.
    pub clutch: u32,
    /// Fraction of the reserve spent per breeding event.
    pub effort: f32,
    /// Age at maturity in ticks.
    pub maturity: u32,
    /// Boldness.
    pub boldness: f32,
    /// Mate choosiness.
    pub choosiness: f32,
    /// Acquired thermal acclimation (°C).
    pub acclimation: f32,
    /// Acquired predator experience.
    pub experience: f32,
}

impl Phenotype {
    /// Expresses `genome` (whose length must match `schema`; genomes created
    /// by this crate and restored snapshots are checked).
    pub fn express(genome: &Genome, schema: &GenomeSchema) -> Self {
        let v = |i: usize| genome.express(schema, i);
        Self {
            speed: v(SPEED),
            perception: v(PERCEPTION),
            size: v(SIZE),
            efficiency: v(EFFICIENCY),
            temp_optimum: v(TEMP_OPTIMUM),
            temp_tolerance: v(TEMP_TOLERANCE),
            clutch: v(CLUTCH).round().clamp(1.0, GENES[CLUTCH].max) as u32,
            effort: v(EFFORT),
            maturity: v(MATURITY).round().max(1.0) as u32,
            boldness: v(BOLDNESS),
            choosiness: v(CHOOSINESS),
            acclimation: v(ACCLIMATION),
            experience: v(EXPERIENCE),
        }
    }

    /// Thermal optimum including acclimation.
    pub fn effective_optimum(&self) -> f32 {
        self.temp_optimum + self.acclimation
    }
}

/// A phenotypic value assigned to a heritable gene by name (scenario files,
/// spawn commands).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraitValue {
    /// Gene name (see [`GENES`]).
    pub gene: String,
    /// Phenotypic value within the gene's range.
    pub value: f32,
}

impl TraitValue {
    /// Convenience constructor.
    pub fn new(gene: &str, value: f32) -> Self {
        Self { gene: gene.to_string(), value }
    }

    /// Resolves the gene and normalises the value, rejecting unknown genes,
    /// acquired traits and out-of-range values.
    pub fn resolve(&self) -> Result<(usize, f32), EvolutionError> {
        let index = gene_index(&self.gene).ok_or_else(|| EvolutionError::UnknownGene(self.gene.clone()))?;
        let info = &GENES[index];
        if !info.heritable {
            return Err(EvolutionError::AcquiredTrait(self.gene.clone()));
        }
        if !self.value.is_finite() || self.value < info.min || self.value > info.max {
            return Err(EvolutionError::TraitOutOfRange {
                gene: self.gene.clone(),
                value: self.value,
                min: info.min,
                max: info.max,
            });
        }
        Ok((index, (self.value - info.min) / (info.max - info.min)))
    }
}

/// Validates a list of trait overrides (each gene at most once).
pub fn validate_traits(traits: &[TraitValue], field: &str) -> Result<(), EvolutionError> {
    for (i, t) in traits.iter().enumerate() {
        t.resolve()?;
        if traits[..i].iter().any(|o| o.gene == t.gene) {
            return Err(EvolutionError::config(field, format!("gene `{}` is listed more than once", t.gene)));
        }
    }
    Ok(())
}

/// Founder genome: every heritable gene starts at its override (or default)
/// plus `N(0, spread)` in normalised units, reflected into `[0, 1]`; acquired
/// traits start at their defaults. Draws two numbers per heritable gene when
/// `spread > 0`, none otherwise.
pub fn founder_genome(
    schema: &GenomeSchema,
    traits: &[TraitValue],
    spread: f32,
    rng: &mut SimRng,
) -> Result<Genome, EvolutionError> {
    let mut values: Vec<f32> = Genome::defaults(schema).values().to_vec();
    for t in traits {
        let (index, normalised) = t.resolve()?;
        values[index] = normalised;
    }
    if spread > 0.0 {
        for (value, info) in values.iter_mut().zip(GENES.iter()) {
            if info.heritable {
                *value = MutationBounds::Reflect.apply(*value + spread * rng.normal() as f32);
            }
        }
    }
    Ok(Genome::from_normalised(schema, values)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        assert_eq!(GENES.iter().filter(|g| g.heritable).count(), HERITABLE_COUNT);
        assert!(GENES[..HERITABLE_COUNT].iter().all(|g| g.heritable), "heritable genes come first");
        for g in &GENES {
            assert!(g.min < g.max && (g.min..=g.max).contains(&g.default), "{}", g.name);
        }
        assert_eq!(gene_index("speed"), Some(SPEED));
        assert_eq!(gene_index("experience"), Some(EXPERIENCE));
        let s = schema(0.05).unwrap();
        assert_eq!(s.len(), GENE_COUNT);
        assert!(!s.genes()[ACCLIMATION].heritable && !s.genes()[EXPERIENCE].heritable);
        assert!(schema(-1.0).is_err());
    }

    #[test]
    fn defaults_express_the_declared_values() {
        let s = schema(0.05).unwrap();
        let p = Phenotype::express(&Genome::defaults(&s), &s);
        assert!((p.speed - 0.7).abs() < 1e-6 && (p.perception - 6.0).abs() < 1e-5);
        assert_eq!((p.clutch, p.maturity), (2, 100));
        assert_eq!((p.acclimation, p.experience), (0.0, 0.0));
    }

    #[test]
    fn trait_overrides_are_validated() {
        assert!(TraitValue::new("speed", 1.0).resolve().is_ok());
        assert!(matches!(TraitValue::new("wings", 1.0).resolve(), Err(EvolutionError::UnknownGene(_))));
        assert!(matches!(TraitValue::new("acclimation", 1.0).resolve(), Err(EvolutionError::AcquiredTrait(_))));
        assert!(matches!(TraitValue::new("speed", 9.0).resolve(), Err(EvolutionError::TraitOutOfRange { .. })));
        let dup = [TraitValue::new("size", 1.0), TraitValue::new("size", 2.0)];
        assert!(validate_traits(&dup, "prey.traits").is_err());
    }

    #[test]
    fn founders_scatter_around_the_overrides() {
        let s = schema(0.05).unwrap();
        let mut rng = SimRng::seed_from_u64(3);
        let traits = [TraitValue::new("speed", 1.3)];
        let exact = founder_genome(&s, &traits, 0.0, &mut rng).unwrap();
        assert!((exact.express(&s, SPEED) - 1.3).abs() < 1e-5);
        let n = 4000;
        let mean: f32 =
            (0..n).map(|_| founder_genome(&s, &traits, 0.05, &mut rng).unwrap().values()[SPEED]).sum::<f32>()
                / n as f32;
        assert!((mean - exact.values()[SPEED]).abs() < 0.01, "{mean}");
        let g = founder_genome(&s, &traits, 0.3, &mut rng).unwrap();
        assert_eq!(g.values()[ACCLIMATION], 0.5, "acquired traits start at their default (0 °C)");
    }
}
