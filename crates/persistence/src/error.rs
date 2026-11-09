//! Error type of the persistence crate.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::envelope::FileKind;
use crate::format::Encoding;

/// Errors produced while saving, loading or migrating files. Messages are
/// written for end users and always name the file involved (through
/// [`PersistenceError::AtPath`]) when one is.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// Another error, annotated with the file it concerns.
    #[error("{}: {source}", path.display())]
    AtPath {
        /// File or directory.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: Box<PersistenceError>,
    },
    /// Operating-system I/O failure.
    #[error("cannot {action}: {source}")]
    Io {
        /// What was being attempted ("read the file", "create the folder"...).
        action: &'static str,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// The file extension does not select a known encoding.
    #[error("unsupported file extension `{found}` (use .ron, .json, .toml or .elsnap)")]
    UnsupportedExtension {
        /// Extension found (empty when the file has none).
        found: String,
    },
    /// Text that is not valid RON / JSON / TOML.
    #[error("invalid {format} syntax: {message}")]
    Syntax {
        /// Encoding being parsed.
        format: Encoding,
        /// Parser message (with line and column where available).
        message: String,
    },
    /// A value that cannot be represented in the chosen encoding.
    #[error("cannot write {format}: {message}")]
    Encode {
        /// Target encoding.
        format: Encoding,
        /// Serializer message.
        message: String,
    },
    /// The data is not an Emergence Lab file envelope.
    #[error("not an Emergence Lab file: {reason}")]
    NotEmergenceLab {
        /// Explanation.
        reason: String,
    },
    /// The file holds a different kind of document.
    #[error("expected a {expected} file, but this is a {found} file")]
    WrongKind {
        /// Kind requested by the caller.
        expected: FileKind,
        /// Kind stored in the file.
        found: FileKind,
    },
    /// The file belongs to another model.
    #[error("the file belongs to model `{found}`, expected `{expected}`")]
    WrongModel {
        /// Model requested by the caller.
        expected: String,
        /// Model stored in the file (`none` when absent).
        found: String,
    },
    /// The file was written by a newer build.
    #[error(
        "{kind} schema version {found} was written by a newer Emergence Lab (this build reads up to version {current})"
    )]
    NewerVersion {
        /// Document kind.
        kind: FileKind,
        /// Version stored in the file.
        found: u32,
        /// Current version of this build.
        current: u32,
    },
    /// A migration step needed to read an old file is not registered.
    #[error("cannot upgrade {kind} data from schema version {from}: no migration to version {} is registered (target version {current})", from + 1)]
    MissingMigration {
        /// Document kind.
        kind: FileKind,
        /// Version without a registered step.
        from: u32,
        /// Version the data should reach.
        current: u32,
    },
    /// A migration step rejected the data.
    #[error("upgrading {kind} data from schema version {from} to {} failed: {reason}", from + 1)]
    MigrationFailed {
        /// Document kind.
        kind: FileKind,
        /// Version the failing step starts from.
        from: u32,
        /// Message returned by the step.
        reason: String,
    },
    /// The same migration step was registered twice.
    #[error("a migration for {kind} data from schema version {from} is already registered")]
    DuplicateMigration {
        /// Document kind.
        kind: FileKind,
        /// Source version.
        from: u32,
    },
    /// Binary files are decoded with the exact current schema and cannot be
    /// migrated.
    #[error(
        "{kind} binary files cannot be upgraded from schema version {found} to {current}; open the file with the version that wrote it and export it as RON or JSON"
    )]
    BinaryMigration {
        /// Document kind.
        kind: FileKind,
        /// Version stored in the file.
        found: u32,
        /// Current version of this build.
        current: u32,
    },
    /// The payload does not match the expected data structure.
    #[error("the {kind} data does not match schema version {version}: {message}")]
    InvalidPayload {
        /// Document kind.
        kind: FileKind,
        /// Schema version used for decoding.
        version: u32,
        /// Decoder message.
        message: String,
    },
    /// A binary file is truncated or damaged.
    #[error("damaged binary file: {0}")]
    CorruptBinary(String),
    /// A size limit was exceeded.
    #[error("{what} is {size} bytes, above the limit of {limit} bytes")]
    TooLarge {
        /// What was measured.
        what: &'static str,
        /// Measured size.
        size: u64,
        /// Configured limit.
        limit: u64,
    },
    /// Data that violates a documented constraint.
    #[error("invalid data: {0}")]
    InvalidData(String),
    /// The per-user data directory cannot be determined.
    #[error("cannot determine the user data folder: {0}")]
    DataDir(String),
}

impl PersistenceError {
    /// Annotates an error with the file it concerns (no-op if it already
    /// carries that path).
    pub fn at(path: &Path, error: PersistenceError) -> Self {
        match error {
            PersistenceError::AtPath { path: p, source } if p == path => PersistenceError::AtPath { path: p, source },
            other => PersistenceError::AtPath { path: path.to_path_buf(), source: Box::new(other) },
        }
    }

    /// I/O error constructor.
    pub(crate) fn io(action: &'static str, source: io::Error) -> Self {
        Self::Io { action, source }
    }

    /// The innermost error, without file annotations.
    pub fn root(&self) -> &PersistenceError {
        match self {
            PersistenceError::AtPath { source, .. } => source.root(),
            other => other,
        }
    }

    /// File the error concerns, if any.
    pub fn path(&self) -> Option<&Path> {
        match self {
            PersistenceError::AtPath { path, .. } => Some(path),
            _ => None,
        }
    }
}
