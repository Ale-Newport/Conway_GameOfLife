//! Error type of the metapixel crate.

use simulation_core::SimError;
use thiserror::Error;

/// Errors reported while loading, programming, building, running or reading
/// an OTCA metapixel construction.
///
/// Messages are written for end users: the desktop interface and the
/// command-line tool show them verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MetapixelError {
    /// A tile file could not be read from disk.
    #[error(
        "cannot read OTCA tile file `{path}`: {message}. The tiles ship in assets/patterns/otca/ (see PROVENANCE.md)"
    )]
    TileFileMissing {
        /// File that could not be read.
        path: String,
        /// Operating-system message.
        message: String,
    },
    /// A tile file is not valid RLE.
    #[error("OTCA tile `{tile}` is not valid RLE: {message}")]
    TileParse {
        /// Which tile (`OFF` or `ON`).
        tile: String,
        /// Parser message (with line and column).
        message: String,
    },
    /// A tile file parsed but does not match the published construction.
    #[error("OTCA tile `{tile}` does not match the published construction: {message}")]
    TileMismatch {
        /// Which tile (`OFF` or `ON`).
        tile: String,
        /// What differs (population, bounding box, programming cells...).
        message: String,
    },
    /// A rule string that is not a two-state outer-totalistic rule.
    #[error("invalid macro rule `{rule}`: {message}")]
    InvalidRule {
        /// Rule text as written by the user.
        rule: String,
        /// Why it was rejected.
        message: String,
    },
    /// A neighbourhood description that cannot be parsed.
    #[error("invalid neighbourhood `{text}`: {message}")]
    InvalidNeighbourhood {
        /// Text as written by the user.
        text: String,
        /// Why it was rejected.
        message: String,
    },
    /// A macro grid with unsupported dimensions.
    #[error("invalid macro grid size {width}×{height}: each side must be in 1..={max} tiles")]
    InvalidGridSize {
        /// Requested width in tiles.
        width: u64,
        /// Requested height in tiles.
        height: u64,
        /// Largest supported side.
        max: u32,
    },
    /// A macro pattern that cannot be used (bad characters, wrong shape...).
    #[error("invalid macro pattern: {0}")]
    InvalidPattern(String),
    /// A tile coordinate outside the macro grid.
    #[error("tile ({col}, {row}) is outside the {columns}×{rows} macro grid")]
    TileOutOfRange {
        /// Requested column.
        col: i64,
        /// Requested row.
        row: i64,
        /// Grid width in tiles.
        columns: u32,
        /// Grid height in tiles.
        rows: u32,
    },
    /// The Hashlife engine reported an error.
    #[error("Hashlife: {0}")]
    Hashlife(String),
    /// A gzip stream could not be decompressed.
    #[error("gzip: {0}")]
    Gzip(String),
    /// A Golly metapixel tiling that cannot be interpreted.
    #[error("Golly tiling: {0}")]
    GollyTiling(String),
    /// A scenario file is malformed.
    #[error("metapixel scenario `{source_name}`: {message}")]
    Scenario {
        /// File or embedded scenario name.
        source_name: String,
        /// What went wrong (with line and column when known).
        message: String,
    },
    /// A request exceeds a documented limit.
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}

impl From<hashlife::HashlifeError> for MetapixelError {
    fn from(error: hashlife::HashlifeError) -> Self {
        Self::Hashlife(error.to_string())
    }
}

impl From<MetapixelError> for SimError {
    fn from(error: MetapixelError) -> Self {
        match error {
            MetapixelError::InvalidRule { .. }
            | MetapixelError::InvalidNeighbourhood { .. }
            | MetapixelError::InvalidGridSize { .. }
            | MetapixelError::InvalidPattern(_)
            | MetapixelError::TileOutOfRange { .. } => SimError::InvalidCommand(error.to_string()),
            MetapixelError::LimitExceeded(_) => SimError::LimitExceeded(error.to_string()),
            MetapixelError::TileParse { .. } | MetapixelError::Scenario { .. } | MetapixelError::Gzip(_) => {
                SimError::Parse(error.to_string())
            }
            _ => SimError::InvalidConfig(error.to_string()),
        }
    }
}
