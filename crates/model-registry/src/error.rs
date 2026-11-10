//! Error type of the registry.

use std::path::PathBuf;

use thiserror::Error;

/// Errors returned by the registry. Messages are written for end users: the
/// command-line tool prints them verbatim.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum RegistryError {
    /// The id is neither a model id nor a scenario id.
    #[error("`{id}` is not a model or scenario id{}", hint(.suggestions))]
    UnknownTarget {
        /// What was asked for.
        id: String,
        /// Close matches.
        suggestions: Vec<String>,
    },
    /// The id names no model.
    #[error("unknown model `{id}`; known models: {}", .known.join(", "))]
    UnknownModel {
        /// What was asked for.
        id: String,
        /// Every model id.
        known: Vec<String>,
    },
    /// A bare scenario id that several folders define.
    #[error("scenario id `{id}` is ambiguous; use one of: {}", .candidates.join(", "))]
    AmbiguousScenario {
        /// What was asked for.
        id: String,
        /// Qualified ids that match.
        candidates: Vec<String>,
    },
    /// A scenario file that cannot be read or parsed.
    #[error("{}: {message}", .path.display())]
    ScenarioFile {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
    /// The model crate rejected the scenario or one of its inputs.
    #[error("cannot build `{target}`: {message}")]
    Build {
        /// Scenario or model being built.
        target: String,
        /// The model crate's message.
        message: String,
    },
    /// A required asset (pattern catalog, OTCA tiles, species files) is
    /// missing or invalid.
    #[error("asset problem: {0}")]
    Asset(String),
    /// A parameter override names no parameter of the model.
    #[error("model `{model}` has no parameter `{key}`; its parameters are: {}", .known.join(", "))]
    UnknownParameter {
        /// Model id.
        model: String,
        /// The key given.
        key: String,
        /// Declared keys.
        known: Vec<String>,
    },
    /// A parameter override that the model rejects.
    #[error("parameter `{key}`: {message}")]
    InvalidParameter {
        /// The key given.
        key: String,
        /// Why the value is rejected.
        message: String,
    },
    /// An override written without `=`.
    #[error("parameter override `{0}` must be written as key=value")]
    MalformedOverride(String),
    /// A seed that the model cannot represent.
    #[error("seed {seed} is not accepted by `{model}`: {message}")]
    InvalidSeed {
        /// Model id.
        model: String,
        /// The seed.
        seed: u64,
        /// Why.
        message: String,
    },
}

fn hint(suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        "; run `emergence-cli list models` or `emergence-cli list scenarios`".to_string()
    } else {
        format!("; did you mean {}?", suggestions.join(", "))
    }
}

impl RegistryError {
    /// Wraps a model crate error raised while building `target`.
    pub(crate) fn build(target: &str, error: impl std::fmt::Display) -> Self {
        Self::Build { target: target.to_string(), message: error.to_string() }
    }
}
