//! Error types of the voxel-life crate.
//!
//! Messages are written for end users: the desktop interface and the
//! command-line tool show them verbatim.

use simulation_core::SimError;
use thiserror::Error;

/// Why a rule string was rejected by [`crate::Rule3D::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuleError {
    /// Nothing but whitespace was given.
    #[error("the rule string is empty")]
    Empty,
    /// The text matches none of the supported notations.
    #[error(
        "unrecognised rule `{0}`: use Softology `S/B/C/N` (e.g. `4/4/5/M`), letters `B5/S45` \
         (optionally `/C<states>` and `/N`), Bays digits `5766` or `(10 21 10 21)`, or Golly `3D5..7/6`"
    )]
    UnknownNotation(String),
    /// One field of the rule is malformed.
    #[error("{field}: {reason}")]
    Field {
        /// Which field ("survival list", "number of states"...).
        field: &'static str,
        /// What is wrong with it.
        reason: String,
    },
    /// A neighbour count larger than the neighbourhood size.
    #[error("{field}: count {count} is impossible in the {size}-voxel {neighbourhood} neighbourhood")]
    CountOutOfRange {
        /// "survival list" or "birth list".
        field: &'static str,
        /// The offending count.
        count: u32,
        /// Neighbourhood size.
        size: u32,
        /// Neighbourhood name.
        neighbourhood: &'static str,
    },
    /// The birth list contains 0.
    #[error(
        "birth on 0 neighbours is not supported: every empty voxel in the world would be born at once \
         (Golly's 3D rules reject it for the same reason)"
    )]
    BirthOnZero,
    /// The number of states is outside `2..=256`.
    #[error("the number of states must be an integer from 2 to 256, got `{0}`")]
    States(String),
    /// An unknown or unsupported neighbourhood letter.
    #[error("unsupported neighbourhood `{0}`: use M (Moore, 26 neighbours) or N (von Neumann, 6 neighbours)")]
    Neighbourhood(String),
}

/// Errors produced while configuring, editing or loading a voxel world.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum VoxelError {
    /// A rule string could not be parsed.
    #[error("invalid rule: {0}")]
    Rule(#[from] RuleError),
    /// A world configuration (size, edges, initialisation) is invalid.
    #[error("invalid world configuration: {0}")]
    Config(String),
    /// A voxel coordinate outside the world.
    #[error("voxel ({x}, {y}, {z}) lies outside the world {bounds}")]
    OutOfBounds {
        /// Column.
        x: i32,
        /// Row.
        y: i32,
        /// Layer.
        z: i32,
        /// Human-readable description of the world box.
        bounds: String,
    },
    /// A box that does not overlap the world.
    #[error("the box {requested} does not overlap the world {world}")]
    BoxOutside {
        /// The requested box.
        requested: String,
        /// The world box.
        world: String,
    },
    /// A state that the current rule does not have.
    #[error("state {state} does not exist: the rule has {states} states (0 to {max})", max = states - 1)]
    InvalidState {
        /// Requested state.
        state: u8,
        /// Number of states of the rule.
        states: u16,
    },
    /// An edit or step would exceed a memory limit.
    #[error("resource limit exceeded: {0}")]
    Limit(String),
    /// A scenario file is malformed or inconsistent.
    #[error("scenario `{source_name}`: {message}")]
    Scenario {
        /// File name or embedded-scenario id.
        source_name: String,
        /// What is wrong.
        message: String,
    },
    /// A file or directory could not be read.
    #[error("cannot read `{path}`: {message}")]
    Io {
        /// Path that failed.
        path: String,
        /// Operating-system message.
        message: String,
    },
    /// A snapshot payload is inconsistent.
    #[error("corrupt snapshot: {0}")]
    Snapshot(String),
}

impl From<VoxelError> for SimError {
    fn from(error: VoxelError) -> Self {
        match error {
            VoxelError::Rule(e) => SimError::Parse(e.to_string()),
            VoxelError::Config(_) | VoxelError::Scenario { .. } | VoxelError::Io { .. } => {
                SimError::InvalidConfig(error.to_string())
            }
            VoxelError::OutOfBounds { .. } | VoxelError::BoxOutside { .. } | VoxelError::InvalidState { .. } => {
                SimError::InvalidCommand(error.to_string())
            }
            VoxelError::Limit(message) => SimError::LimitExceeded(message),
            VoxelError::Snapshot(message) => SimError::Serialization(message),
        }
    }
}
