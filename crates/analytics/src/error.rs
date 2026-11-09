//! Error type of the analytics crate.

use thiserror::Error;

/// Errors produced by analytics functions. Messages are written for end users
/// (they are shown by the desktop interface and the command-line tool).
#[derive(Debug, Error, Clone, PartialEq)]
pub enum AnalyticsError {
    /// Not enough samples for the requested statistic.
    #[error("{what} needs at least {needed} value(s), got {got}")]
    NotEnoughData {
        /// Name of the statistic.
        what: &'static str,
        /// Minimum number of values.
        needed: usize,
        /// Number of values supplied.
        got: usize,
    },
    /// An input value is NaN or infinite.
    #[error("{what}: value {value} at position {index} is not a finite number")]
    NonFinite {
        /// Name of the computation.
        what: &'static str,
        /// Position of the offending value.
        index: usize,
        /// The offending value.
        value: f64,
    },
    /// An argument is outside its admissible range.
    #[error("invalid argument for {what}: {reason}")]
    InvalidArgument {
        /// Name of the computation.
        what: &'static str,
        /// Explanation.
        reason: String,
    },
    /// Paired inputs have different lengths.
    #[error("{what}: inputs have different lengths ({left} and {right})")]
    LengthMismatch {
        /// Name of the computation.
        what: &'static str,
        /// Length of the first input.
        left: usize,
        /// Length of the second input.
        right: usize,
    },
    /// The statistic is mathematically undefined for this input.
    #[error("{what} is undefined: {reason}")]
    Undefined {
        /// Name of the statistic.
        what: &'static str,
        /// Explanation.
        reason: String,
    },
    /// A sample arrived with a tick earlier than (or, for a single series,
    /// equal to) the previously recorded one.
    #[error("metric `{key}`: tick {tick} is not after the last recorded tick {last}")]
    TickOutOfOrder {
        /// Metric key (or `*` for a whole measurement).
        key: String,
        /// Tick of the rejected sample.
        tick: u64,
        /// Last tick already stored.
        last: u64,
    },
    /// Imported or constructed data violates an invariant.
    #[error("invalid data: {0}")]
    InvalidData(String),
    /// A document has a schema version this build cannot read.
    #[error("unsupported {what} schema version {found} (this build reads up to version {supported})")]
    UnsupportedVersion {
        /// Document type.
        what: &'static str,
        /// Version found in the document.
        found: u32,
        /// Highest version this build reads.
        supported: u32,
    },
    /// JSON encoding or decoding failed.
    #[error("JSON error: {0}")]
    Json(String),
    /// Writing an export failed.
    #[error("write error: {0}")]
    Io(String),
}

impl AnalyticsError {
    pub(crate) fn invalid(what: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidArgument { what, reason: reason.into() }
    }

    pub(crate) fn undefined(what: &'static str, reason: impl Into<String>) -> Self {
        Self::Undefined { what, reason: reason.into() }
    }
}

/// Checks that `values` is non-empty (at least `needed` values) and finite.
pub(crate) fn check_values(what: &'static str, values: &[f64], needed: usize) -> Result<(), AnalyticsError> {
    if values.len() < needed {
        return Err(AnalyticsError::NotEnoughData { what, needed, got: values.len() });
    }
    check_finite(what, values)
}

/// Checks that every value is finite.
pub(crate) fn check_finite(what: &'static str, values: &[f64]) -> Result<(), AnalyticsError> {
    match values.iter().position(|v| !v.is_finite()) {
        Some(index) => Err(AnalyticsError::NonFinite { what, index, value: values[index] }),
        None => Ok(()),
    }
}
