//! Error type of the continuous-automata crate.

use std::path::PathBuf;

use simulation_core::SimError;
use thiserror::Error;

/// Errors produced while validating rules, decoding cell data, loading
/// scenarios or applying commands. Messages are written for end users.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum ContinuousError {
    /// A rule (kernel, growth, time step...) failed validation.
    #[error("invalid rule: {0}")]
    InvalidRule(String),
    /// World dimensions outside the supported range.
    #[error("invalid world size {width}×{height}: {reason}")]
    InvalidWorld {
        /// Requested width in cells.
        width: usize,
        /// Requested height in cells.
        height: usize,
        /// Why the size was rejected.
        reason: String,
    },
    /// A kernel does not fit on the torus (it would overlap itself).
    #[error(
        "kernel {kernel} reaches {reach} cells from its centre; the world needs at least {needed} cells per side (has {width}×{height})"
    )]
    KernelTooLarge {
        /// Index of the offending kernel.
        kernel: usize,
        /// Largest non-zero offset of the kernel, in cells.
        reach: usize,
        /// Minimum world side for this kernel (`2·reach + 1`).
        needed: usize,
        /// World width.
        width: usize,
        /// World height.
        height: usize,
    },
    /// The requested world and rule would need more memory or work per tick
    /// than this crate allows (see [`crate::convolve::MAX_PLAN_BYTES`] and
    /// [`crate::convolve::MAX_DIRECT_WORK`]).
    #[error("{0}")]
    LimitExceeded(String),
    /// A kernel whose weights are all zero (cannot be normalised).
    #[error("kernel {0} has no positive weight inside its radius (check the radius and the ring peaks β)")]
    EmptyKernel(usize),
    /// Malformed Lenia cell RLE.
    #[error("cell RLE error at character {position}: {message}")]
    Rle {
        /// Zero-based character offset in the RLE text.
        position: usize,
        /// Description of the problem.
        message: String,
    },
    /// Invalid creature cell data (sizes, values, channels).
    #[error("invalid cell data: {0}")]
    InvalidCells(String),
    /// A command that cannot be applied.
    #[error("{0}")]
    InvalidCommand(String),
    /// Scenario text failed to parse or validate.
    #[error("scenario `{source_name}`: {message}")]
    Scenario {
        /// File name or identifier of the scenario.
        source_name: String,
        /// Description of the problem (with line/column for syntax errors).
        message: String,
    },
    /// File-system failure while loading scenarios.
    #[error("cannot read `{path}`: {message}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Operating-system message.
        message: String,
    },
}

impl ContinuousError {
    /// Convenience constructor for [`ContinuousError::InvalidRule`].
    pub fn rule(message: impl Into<String>) -> Self {
        Self::InvalidRule(message.into())
    }

    /// Convenience constructor for [`ContinuousError::InvalidCommand`].
    pub fn command(message: impl Into<String>) -> Self {
        Self::InvalidCommand(message.into())
    }
}

impl From<ContinuousError> for SimError {
    fn from(error: ContinuousError) -> Self {
        match error {
            ContinuousError::InvalidCommand(message) => SimError::InvalidCommand(message),
            ContinuousError::Rle { .. } => SimError::Parse(error.to_string()),
            ContinuousError::LimitExceeded(message) => SimError::LimitExceeded(message),
            other => SimError::InvalidConfig(other.to_string()),
        }
    }
}
