//! Error type for pattern parsing, writing and catalog loading.

use simulation_core::SimError;
use thiserror::Error;

use crate::format::Format;
use crate::rule::RuleError;

/// Errors produced by the pattern readers, writers and the catalog loader.
///
/// Messages are written for end users and include the line and column of the
/// offending character whenever the input is text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatternError {
    /// Malformed input in one of the text formats.
    #[error("{format} syntax error at line {line}, column {column}: {message}")]
    Syntax {
        /// Format being parsed.
        format: Format,
        /// 1-based line number.
        line: usize,
        /// 1-based column (in characters) within the line.
        column: usize,
        /// What went wrong.
        message: String,
    },
    /// An invalid rule string.
    #[error("invalid rule: {0}")]
    Rule(#[from] RuleError),
    /// A format, file type or pattern feature that is not supported (for
    /// example a multi-state pattern written as plaintext, or a macrocell file).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The input would exceed a resource limit such as [`crate::MAX_CELLS`].
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// The input contains no pattern data at all.
    #[error("no pattern data found (the input is empty or contains only whitespace)")]
    Empty,
    /// A file could not be read.
    #[error("cannot read `{path}`: {message}")]
    Io {
        /// Path that failed.
        path: String,
        /// Operating-system error message.
        message: String,
    },
    /// The pattern catalog is malformed.
    #[error("pattern catalog `{path}`: {message}")]
    Catalog {
        /// Catalog file.
        path: String,
        /// What went wrong, including the entry when known.
        message: String,
    },
}

impl PatternError {
    /// Convenience constructor for [`PatternError::Syntax`].
    pub fn syntax(format: Format, line: usize, column: usize, message: impl Into<String>) -> Self {
        Self::Syntax { format, line, column, message: message.into() }
    }

    /// Line number of a syntax error, if this is one.
    pub fn line(&self) -> Option<usize> {
        match self {
            Self::Syntax { line, .. } => Some(*line),
            _ => None,
        }
    }

    /// Column of a syntax error, if this is one.
    pub fn column(&self) -> Option<usize> {
        match self {
            Self::Syntax { column, .. } => Some(*column),
            _ => None,
        }
    }
}

impl From<PatternError> for SimError {
    fn from(error: PatternError) -> Self {
        match error {
            PatternError::LimitExceeded(message) => SimError::LimitExceeded(message),
            PatternError::Io { .. } | PatternError::Catalog { .. } | PatternError::Unsupported(_) => {
                SimError::InvalidConfig(error.to_string())
            }
            PatternError::Syntax { .. } | PatternError::Rule(_) | PatternError::Empty => {
                SimError::Parse(error.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_error_message_and_positions() {
        let err = PatternError::syntax(Format::Rle, 3, 7, "unexpected character '%'");
        assert_eq!(err.to_string(), "RLE syntax error at line 3, column 7: unexpected character '%'");
        assert_eq!(err.line(), Some(3));
        assert_eq!(err.column(), Some(7));
        assert_eq!(PatternError::Empty.line(), None);
    }

    #[test]
    fn converts_to_sim_error() {
        let parse: SimError = PatternError::syntax(Format::Plaintext, 1, 1, "bad").into();
        assert!(matches!(parse, SimError::Parse(_)));
        let limit: SimError = PatternError::LimitExceeded("too many cells".into()).into();
        assert_eq!(limit, SimError::LimitExceeded("too many cells".into()));
        let config: SimError = PatternError::Unsupported("macrocell".into()).into();
        assert!(matches!(config, SimError::InvalidConfig(_)));
    }
}
