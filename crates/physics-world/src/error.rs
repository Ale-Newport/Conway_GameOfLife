//! Error type of the physics world.

use simulation_core::SimError;
use thiserror::Error;

/// Errors produced while configuring the world or applying interventions.
/// Messages are written for end users.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum WorldError {
    /// A configuration field is outside its admissible range.
    #[error("invalid world configuration: `{field}` {reason}")]
    InvalidConfig {
        /// Name of the offending field.
        field: &'static str,
        /// Why the value was rejected.
        reason: String,
    },
    /// A cell coordinate outside the world.
    #[error("cell ({x}, {y}) is outside the {width}x{height} world")]
    OutOfBounds {
        /// Requested column.
        x: i64,
        /// Requested row.
        y: i64,
        /// World width in cells.
        width: usize,
        /// World height in cells.
        height: usize,
    },
    /// An intervention argument that is not finite or outside its range.
    #[error("invalid value for `{name}`: {reason}")]
    InvalidArgument {
        /// Argument name.
        name: &'static str,
        /// Why the value was rejected.
        reason: String,
    },
    /// A user-provided heightmap whose size does not match the configuration.
    #[error("heightmap is {found_width}x{found_height} but the configuration asks for {width}x{height}")]
    HeightmapSize {
        /// Width of the provided heightmap.
        found_width: usize,
        /// Height of the provided heightmap.
        found_height: usize,
        /// Configured width.
        width: usize,
        /// Configured height.
        height: usize,
    },
}

impl WorldError {
    /// Convenience constructor for [`WorldError::InvalidArgument`].
    pub(crate) fn arg(name: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidArgument { name, reason: reason.into() }
    }

    /// Convenience constructor for [`WorldError::InvalidConfig`].
    pub(crate) fn config(field: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidConfig { field, reason: reason.into() }
    }
}

impl From<WorldError> for SimError {
    fn from(error: WorldError) -> Self {
        match error {
            WorldError::InvalidConfig { .. } | WorldError::HeightmapSize { .. } => {
                SimError::InvalidConfig(error.to_string())
            }
            WorldError::OutOfBounds { .. } | WorldError::InvalidArgument { .. } => {
                SimError::InvalidCommand(error.to_string())
            }
        }
    }
}

/// Checks that `value` is finite and inside `[min, max]`.
pub(crate) fn check_range(name: &'static str, value: f32, min: f32, max: f32) -> Result<f32, WorldError> {
    if !value.is_finite() {
        return Err(WorldError::arg(name, format!("{value} is not a finite number")));
    }
    if value < min || value > max {
        return Err(WorldError::arg(name, format!("{value} is outside [{min}, {max}]")));
    }
    Ok(value)
}
