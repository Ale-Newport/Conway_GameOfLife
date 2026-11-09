//! Error type shared by simulation crates.

use thiserror::Error;

/// Errors produced while configuring, commanding or (de)serialising a
/// simulation. Messages are written for end users: they are shown verbatim in
/// the desktop interface and the command-line tool.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum SimError {
    /// A parameter key that the model does not declare.
    #[error("unknown parameter `{0}`")]
    UnknownParameter(String),
    /// A parameter value of the wrong type or outside its declared range.
    #[error("invalid value for parameter `{key}`: {reason}")]
    InvalidParameter { key: String, reason: String },
    /// A command that cannot be applied in the current state.
    #[error("invalid command: {0}")]
    InvalidCommand(String),
    /// A configuration (scenario, rule, world description) failed validation.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// A snapshot produced by a different model.
    #[error("snapshot belongs to model `{found}`, expected `{expected}`")]
    WrongModel { expected: String, found: String },
    /// A snapshot written with a schema version this build cannot read.
    #[error("snapshot schema version {found} is not supported (this build reads up to {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    /// Binary or text (de)serialisation failure.
    #[error("serialization error: {0}")]
    Serialization(String),
    /// Parsing of a user-provided text format (RLE, rule strings, scripts...).
    #[error("parse error: {0}")]
    Parse(String),
    /// The requested operation exceeds a configured resource limit.
    #[error("resource limit exceeded: {0}")]
    LimitExceeded(String),
}

impl SimError {
    /// Convenience constructor for [`SimError::InvalidParameter`].
    pub fn invalid_param(key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidParameter { key: key.into(), reason: reason.into() }
    }
}
