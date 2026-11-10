//! Error type of the ecosystem crate.

use std::path::PathBuf;

use organism_engine::OrganismError;
use physics_world::WorldError;
use simulation_core::SimError;
use thiserror::Error;

/// Errors raised while loading species and scenarios, building an
/// ecosystem or applying commands. Messages are written for end users.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum EcosystemError {
    /// A species definition is inconsistent.
    #[error("species `{species}`: `{field}` {reason}")]
    Species {
        /// Species id (or file name when the id is unknown).
        species: String,
        /// Offending field.
        field: String,
        /// What is wrong.
        reason: String,
    },
    /// A data file (species, scenario) could not be parsed.
    #[error("{source_name}: {reason}")]
    Parse {
        /// File or embedded resource name.
        source_name: String,
        /// Parser message (with line and column).
        reason: String,
    },
    /// A file could not be read.
    #[error("cannot read `{path}`: {reason}")]
    Io {
        /// Path of the file or directory.
        path: PathBuf,
        /// Operating-system message.
        reason: String,
    },
    /// A scenario or configuration value is invalid.
    #[error("invalid ecosystem configuration: `{field}` {reason}")]
    Config {
        /// Offending field.
        field: String,
        /// What is wrong.
        reason: String,
    },
    /// A species id that the catalogue does not contain.
    #[error("unknown species `{0}`")]
    UnknownSpecies(String),
    /// An organism id that does not exist (or no longer exists).
    #[error("no living organism with id {0}")]
    UnknownOrganism(u64),
    /// A command that cannot be applied.
    #[error("invalid command: {0}")]
    Command(String),
    /// Error from the physics world.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Error from the organism engine.
    #[error(transparent)]
    Organism(#[from] OrganismError),
}

impl EcosystemError {
    /// Convenience constructor for [`EcosystemError::Species`].
    pub fn species(species: &str, field: &str, reason: impl Into<String>) -> Self {
        Self::Species { species: species.to_string(), field: field.to_string(), reason: reason.into() }
    }

    /// Convenience constructor for [`EcosystemError::Config`].
    pub fn config(field: &str, reason: impl Into<String>) -> Self {
        Self::Config { field: field.to_string(), reason: reason.into() }
    }
}

impl From<EcosystemError> for SimError {
    fn from(error: EcosystemError) -> Self {
        let message = error.to_string();
        match error {
            EcosystemError::Species { .. } | EcosystemError::Config { .. } => SimError::InvalidConfig(message),
            EcosystemError::Parse { .. } => SimError::Parse(message),
            EcosystemError::Io { .. } => SimError::InvalidConfig(message),
            EcosystemError::UnknownSpecies(_) | EcosystemError::UnknownOrganism(_) | EcosystemError::Command(_) => {
                SimError::InvalidCommand(message)
            }
            EcosystemError::World(e) => match e {
                WorldError::InvalidConfig { .. } | WorldError::HeightmapSize { .. } => SimError::InvalidConfig(message),
                WorldError::OutOfBounds { .. } | WorldError::InvalidArgument { .. } => {
                    SimError::InvalidCommand(message)
                }
            },
            EcosystemError::Organism(e) => SimError::from(e),
        }
    }
}

/// Checks that `value` is finite and inside `[min, max]`.
pub(crate) fn check_range(species: &str, field: &str, value: f32, min: f32, max: f32) -> Result<(), EcosystemError> {
    if !value.is_finite() || value < min || value > max {
        return Err(EcosystemError::species(species, field, format!("{value} is outside [{min}, {max}]")));
    }
    Ok(())
}

/// Checks a `(low, high)` pair: finite, ordered and inside `[min, max]`.
pub(crate) fn check_pair(
    species: &str,
    field: &str,
    pair: (f32, f32),
    min: f32,
    max: f32,
) -> Result<(), EcosystemError> {
    check_range(species, field, pair.0, min, max)?;
    check_range(species, field, pair.1, min, max)?;
    if pair.0 > pair.1 {
        return Err(EcosystemError::species(species, field, format!("({}, {}) is not ordered", pair.0, pair.1)));
    }
    Ok(())
}

/// Checks an identifier: non-empty, lowercase ASCII letters, digits, `_`, `-`.
pub(crate) fn check_id(kind: &str, id: &str) -> Result<(), EcosystemError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(EcosystemError::species(id, "id", format!("{kind} id must be 1–64 lowercase letters, digits, `_` or `-`")))
    }
}
