//! Error type of the civilization model.

use simulation_core::SimError;
use thiserror::Error;

/// Errors produced while loading scenarios, building worlds or applying
/// commands. Messages are written for end users and shown verbatim.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum CivError {
    /// A scenario failed validation (bad fraction, empty population list...).
    #[error("invalid scenario: {0}")]
    InvalidScenario(String),
    /// A hand-drawn terrain map is malformed.
    #[error("invalid map: {0}")]
    InvalidMap(String),
    /// A scenario file could not be parsed as RON.
    #[error("cannot parse scenario {origin}: {message}")]
    Parse {
        /// File name or `builtin:<id>` of the parsed text.
        origin: String,
        /// Parser message including line and column.
        message: String,
    },
    /// A scenario file or folder could not be read.
    #[error("cannot read {path}: {message}")]
    Io {
        /// Path that failed.
        path: String,
        /// Operating-system message.
        message: String,
    },
    /// A command was rejected (coordinates outside the world, bad amount...).
    #[error("invalid command: {0}")]
    InvalidCommand(String),
}

impl From<CivError> for SimError {
    fn from(error: CivError) -> Self {
        match error {
            CivError::InvalidCommand(message) => SimError::InvalidCommand(message),
            CivError::Parse { .. } => SimError::Parse(error.to_string()),
            other => SimError::InvalidConfig(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_to_sim_errors_with_readable_messages() {
        let e: SimError = CivError::InvalidCommand("radius must be positive".into()).into();
        assert_eq!(e.to_string(), "invalid command: radius must be positive");
        let e: SimError = CivError::Parse { origin: "a.ron".into(), message: "1:2: oops".into() }.into();
        assert!(matches!(e, SimError::Parse(_)));
        assert!(e.to_string().contains("a.ron"));
        let e: SimError = CivError::InvalidScenario("no populations".into()).into();
        assert!(matches!(e, SimError::InvalidConfig(_)));
    }
}
