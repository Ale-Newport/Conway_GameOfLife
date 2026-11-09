//! Error type of the Hashlife engine.

use thiserror::Error;

/// Errors reported by the Hashlife engine and its file formats.
///
/// Messages are written for end users: they are shown verbatim by the desktop
/// interface and the command-line tool.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HashlifeError {
    /// The rule makes empty space come alive (`B0`). Hashlife relies on the
    /// empty square staying empty, so such rules cannot be run.
    #[error(
        "rule `{rule}` contains B0 (birth with 0 neighbours): empty space would not stay empty, \
         which Hashlife requires; use a dense engine for B0 rules"
    )]
    BirthOnZero {
        /// The rejected rule, as written by the user.
        rule: String,
    },
    /// The rule text or masks are not a valid two-state outer-totalistic rule.
    #[error("invalid rule `{rule}`: {reason}")]
    InvalidRule {
        /// The rejected rule, as written by the user.
        rule: String,
        /// Why it was rejected.
        reason: String,
    },
    /// Advancing would overflow the 64-bit generation counter.
    #[error("cannot advance {requested} generations from generation {generation}: the counter would exceed 2^64 - 1")]
    GenerationOverflow {
        /// Generation before the request.
        generation: u64,
        /// Number of generations requested.
        requested: u64,
    },
    /// A power-of-two step larger than the engine supports in one call.
    #[error("step exponent {exponent} is too large: one step can advance at most 2^{max} generations")]
    StepTooLarge {
        /// Requested exponent.
        exponent: u32,
        /// Largest supported exponent.
        max: u32,
    },
    /// The pattern is too close to the edge of the 64-bit coordinate plane to
    /// be advanced (or represented) without leaving it.
    #[error("{0}")]
    CoordinateOverflow(String),
    /// A macrocell (`.mc`) file could not be parsed.
    #[error("macrocell line {line}: {message}")]
    Macrocell {
        /// 1-based line number.
        line: usize,
        /// What is wrong with the line.
        message: String,
    },
    /// An argument outside its documented range.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}
