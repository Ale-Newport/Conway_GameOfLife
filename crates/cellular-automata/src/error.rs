//! Error type of the cellular-automata crate.

use hashlife::HashlifeError;
use patterns::{PatternError, RuleError};
use simulation_core::SimError;
use thiserror::Error;

/// Errors reported by the engines, models, editing tools and scenario
/// loader. Messages are written for end users: the desktop interface and the
/// command-line tool show them verbatim.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CaError {
    /// A rule the selected engine or topology cannot run (for example `B0`
    /// on the infinite plane).
    #[error("rule {rule} cannot be used here: {reason}")]
    UnsupportedRule {
        /// The rule in canonical notation.
        rule: String,
        /// Why it is rejected and what to do instead.
        reason: String,
    },
    /// An engine that cannot run the requested topology.
    #[error("the {engine} engine cannot run a {topology} world: {hint}")]
    UnsupportedTopology {
        /// Engine label.
        engine: &'static str,
        /// Topology label.
        topology: &'static str,
        /// Which engines can.
        hint: &'static str,
    },
    /// World dimensions outside the supported range.
    #[error("invalid world size {width}×{height}: {reason}")]
    InvalidSize {
        /// Requested width.
        width: u64,
        /// Requested height.
        height: u64,
        /// What is wrong.
        reason: String,
    },
    /// A request that would exceed a memory or work limit.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// An argument outside its documented range.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// A rule string that does not parse.
    #[error("invalid rule: {0}")]
    Rule(#[from] RuleError),
    /// A pattern that could not be read or written.
    #[error(transparent)]
    Pattern(#[from] PatternError),
    /// An error of the Hashlife engine (coordinate or generation overflow).
    #[error("Hashlife: {0}")]
    Hashlife(#[from] HashlifeError),
    /// A malformed scenario file.
    #[error("scenario `{source_name}`: {message}")]
    Scenario {
        /// File name or embedded scenario id.
        source_name: String,
        /// What is wrong.
        message: String,
    },
    /// A file could not be read.
    #[error("cannot read `{path}`: {message}")]
    Io {
        /// Path that failed.
        path: String,
        /// Operating-system message.
        message: String,
    },
}

impl CaError {
    /// Convenience constructor for [`CaError::InvalidArgument`].
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidArgument(message.into())
    }
}

impl From<CaError> for SimError {
    fn from(error: CaError) -> Self {
        match error {
            CaError::LimitExceeded(message) => SimError::LimitExceeded(message),
            CaError::Rule(e) => SimError::Parse(e.to_string()),
            CaError::Pattern(e) => SimError::from(e),
            CaError::Scenario { .. } | CaError::Io { .. } => SimError::InvalidConfig(error.to_string()),
            other => SimError::InvalidCommand(other.to_string()),
        }
    }
}
