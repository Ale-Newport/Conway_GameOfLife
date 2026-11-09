//! Error type of the evolution laboratory.

use std::path::PathBuf;

use organism_engine::OrganismError;
use simulation_core::SimError;
use thiserror::Error;

/// Errors raised while configuring, commanding or loading the evolution
/// model. Messages are written for end users: they name the offending field,
/// gene, command or file.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum EvolutionError {
    /// A configuration value is outside its admissible range or inconsistent.
    #[error("invalid evolution configuration: {field}: {reason}")]
    InvalidConfig {
        /// Dotted path of the field, e.g. `food.regrowth_rate`.
        field: String,
        /// What is wrong with it.
        reason: String,
    },
    /// A trait override names a gene the model does not have.
    #[error("unknown gene `{0}` (known genes: {known})", known = crate::genes::gene_names().join(", "))]
    UnknownGene(String),
    /// A trait override targets an acquired (non-heritable) trait.
    #[error("`{0}` is an acquired trait: it starts at its default in every organism and cannot be set")]
    AcquiredTrait(String),
    /// A trait value outside the gene's phenotypic range.
    #[error("value {value} for gene `{gene}` is outside its range [{min}, {max}]")]
    TraitOutOfRange {
        /// Gene name.
        gene: String,
        /// Rejected value.
        value: f32,
        /// Lower bound of the range.
        min: f32,
        /// Upper bound of the range.
        max: f32,
    },
    /// A command that cannot be applied.
    #[error("invalid command: {0}")]
    InvalidCommand(String),
    /// A scenario file could not be parsed.
    #[error("scenario {source_name}: {reason}")]
    Scenario {
        /// File name or built-in scenario name.
        source_name: String,
        /// Parse or validation message (RON errors include line and column).
        reason: String,
    },
    /// Reading a scenario file or directory failed.
    #[error("cannot read {path}: {reason}")]
    Io {
        /// The path involved.
        path: PathBuf,
        /// Operating-system message.
        reason: String,
    },
    /// An error reported by the organism engine (genomes, genealogy, store).
    #[error(transparent)]
    Organism(#[from] OrganismError),
}

impl EvolutionError {
    /// Convenience constructor for [`EvolutionError::InvalidConfig`].
    pub fn config(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidConfig { field: field.into(), reason: reason.into() }
    }
}

impl From<EvolutionError> for SimError {
    fn from(error: EvolutionError) -> Self {
        let message = error.to_string();
        match error {
            EvolutionError::InvalidCommand(_) => SimError::InvalidCommand(message),
            EvolutionError::Scenario { .. } => SimError::Parse(message),
            EvolutionError::Organism(inner) => inner.into(),
            EvolutionError::InvalidConfig { .. }
            | EvolutionError::UnknownGene(_)
            | EvolutionError::AcquiredTrait(_)
            | EvolutionError::TraitOutOfRange { .. }
            | EvolutionError::Io { .. } => SimError::InvalidConfig(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_problem() {
        let e = EvolutionError::config("food.regrowth_rate", "2 is outside [0, 0.5]");
        assert_eq!(e.to_string(), "invalid evolution configuration: food.regrowth_rate: 2 is outside [0, 0.5]");
        let unknown = EvolutionError::UnknownGene("wings".into()).to_string();
        assert!(unknown.contains("`wings`") && unknown.contains("speed"));
        let sim: SimError = EvolutionError::InvalidCommand("no such barrier".into()).into();
        assert_eq!(sim, SimError::InvalidCommand("invalid command: no such barrier".into()));
    }
}
