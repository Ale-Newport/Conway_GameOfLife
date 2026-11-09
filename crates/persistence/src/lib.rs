//! # persistence
//!
//! Versioned files for Emergence Lab:
//!
//! * [`Envelope`] — the header every file carries (`format`, [`FileKind`],
//!   `schema_version`, `app_version`, `model_id`, `created`) around its
//!   payload.
//! * [`Codec`] — saves and loads envelopes as RON, JSON or TOML (chosen by
//!   file extension) or as the binary `.elsnap` container ([`binary`]),
//!   validating the header and upgrading old text documents through the
//!   [`MigrationRegistry`].
//! * Snapshot and snapshot-history files ([`snapshot_file`]) for
//!   [`simulation_core::Snapshot`].
//! * [`atomic_write`] (temporary file + rename) and size-limited reads.
//! * [`UserDataDirs`] — the per-user data folder and its `saves/`,
//!   `scenarios/`, `exports/` and `screenshots/` subfolders.
//!
//! Every error names the file involved and the reason
//! ([`PersistenceError`]). The crate never reads the clock: creation times
//! are supplied by the caller.

pub mod atomic;
pub mod binary;
pub mod codec;
pub mod dirs;
pub mod envelope;
pub mod error;
pub mod format;
pub mod migrate;
pub mod snapshot_file;

pub use atomic::{atomic_write, read_limited};
pub use codec::{Codec, DEFAULT_MAX_FILE_BYTES};
pub use dirs::{DATA_DIR_ENV, DataFolder, Platform, UserDataDirs};
pub use envelope::{APP_VERSION, Document, DocumentSpec, Envelope, EnvelopeHeader, FORMAT_TAG, FileKind, Loaded};
pub use error::PersistenceError;
pub use format::Encoding;
pub use migrate::{MigrationFn, MigrationRegistry};
pub use snapshot_file::{HISTORY_SCHEMA_VERSION, HistoryLimits, SNAPSHOT_SCHEMA_VERSION, validate_history};
