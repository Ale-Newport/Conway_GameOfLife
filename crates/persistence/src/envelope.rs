//! The versioned envelope wrapped around every saved document.
//!
//! ```text
//! (
//!     format: "emergence-lab",
//!     kind: scenario,
//!     schema_version: 3,
//!     app_version: "0.1.0",
//!     model_id: Some("ecosystem"),
//!     created: "2026-11-09T10:00:00Z",
//!     payload: ( ... ),
//! )
//! ```
//!
//! The header fields are stable across all versions; only the payload
//! changes shape, and its `schema_version` tells the loader which
//! migrations to run (see [`crate::MigrationRegistry`]).

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Value of the `format` field identifying Emergence Lab files.
pub const FORMAT_TAG: &str = "emergence-lab";

/// Version of the application writing files (the workspace version).
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What a file contains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    /// Initial conditions and parameters of a model.
    Scenario,
    /// One saved simulation state ([`simulation_core::Snapshot`]).
    Snapshot,
    /// A sequence of snapshots (history export).
    SnapshotHistory,
    /// Rules of a rule-engine world or a custom automaton.
    RuleSet,
    /// Experiment definition or results (run records, comparisons).
    Experiment,
    /// Application or user settings.
    Settings,
    /// A cell pattern with metadata.
    Pattern,
}

impl FileKind {
    /// Every kind, in declaration order.
    pub const ALL: [FileKind; 7] = [
        FileKind::Scenario,
        FileKind::Snapshot,
        FileKind::SnapshotHistory,
        FileKind::RuleSet,
        FileKind::Experiment,
        FileKind::Settings,
        FileKind::Pattern,
    ];

    /// Human-readable name used in messages.
    pub fn label(self) -> &'static str {
        match self {
            FileKind::Scenario => "scenario",
            FileKind::Snapshot => "snapshot",
            FileKind::SnapshotHistory => "snapshot history",
            FileKind::RuleSet => "rule set",
            FileKind::Experiment => "experiment",
            FileKind::Settings => "settings",
            FileKind::Pattern => "pattern",
        }
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Header fields of an [`Envelope`] (everything except the payload).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeHeader {
    /// Always [`FORMAT_TAG`].
    pub format: String,
    /// Document kind.
    pub kind: FileKind,
    /// Schema version of the payload.
    pub schema_version: u32,
    /// Version of the application that wrote the file.
    pub app_version: String,
    /// Model the document belongs to, if any.
    #[serde(default)]
    pub model_id: Option<String>,
    /// Creation time as supplied by the caller (e.g. RFC 3339). The crate
    /// never reads the clock, so saved files are reproducible.
    pub created: String,
}

/// A document with its versioned header.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope<P> {
    /// Always [`FORMAT_TAG`].
    pub format: String,
    /// Document kind.
    pub kind: FileKind,
    /// Schema version of the payload.
    pub schema_version: u32,
    /// Version of the application that wrote the file.
    pub app_version: String,
    /// Model the document belongs to, if any.
    #[serde(default)]
    pub model_id: Option<String>,
    /// Creation time as supplied by the caller.
    pub created: String,
    /// The document itself.
    pub payload: P,
}

impl<P> Envelope<P> {
    /// Envelope written by this build ([`APP_VERSION`]) without a model id.
    pub fn new(kind: FileKind, schema_version: u32, created: &str, payload: P) -> Self {
        Self {
            format: FORMAT_TAG.to_string(),
            kind,
            schema_version,
            app_version: APP_VERSION.to_string(),
            model_id: None,
            created: created.to_string(),
            payload,
        }
    }

    /// Sets the model id.
    pub fn with_model(mut self, model_id: &str) -> Self {
        self.model_id = Some(model_id.to_string());
        self
    }

    /// Overrides the application version.
    pub fn with_app_version(mut self, app_version: &str) -> Self {
        self.app_version = app_version.to_string();
        self
    }

    /// Header fields.
    pub fn header(&self) -> EnvelopeHeader {
        EnvelopeHeader {
            format: self.format.clone(),
            kind: self.kind,
            schema_version: self.schema_version,
            app_version: self.app_version.clone(),
            model_id: self.model_id.clone(),
            created: self.created.clone(),
        }
    }

    /// Combines a header and a payload.
    pub fn from_parts(header: EnvelopeHeader, payload: P) -> Self {
        Self {
            format: header.format,
            kind: header.kind,
            schema_version: header.schema_version,
            app_version: header.app_version,
            model_id: header.model_id,
            created: header.created,
            payload,
        }
    }
}

/// Kind and current schema version of a document type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocumentSpec {
    /// Document kind.
    pub kind: FileKind,
    /// Schema version written by this build; older files are migrated to it.
    pub schema_version: u32,
}

impl DocumentSpec {
    /// New spec.
    pub fn new(kind: FileKind, schema_version: u32) -> Self {
        Self { kind, schema_version }
    }
}

/// A payload type with a fixed kind and current schema version, for the
/// typed convenience functions of [`crate::Codec`].
pub trait Document: Serialize + DeserializeOwned {
    /// Document kind.
    const KIND: FileKind;
    /// Current schema version.
    const SCHEMA_VERSION: u32;

    /// The spec built from the constants.
    fn spec() -> DocumentSpec {
        DocumentSpec::new(Self::KIND, Self::SCHEMA_VERSION)
    }
}

/// A loaded document and how it was obtained.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded<P> {
    /// The document, upgraded to the current schema version.
    pub envelope: Envelope<P>,
    /// Schema version stored in the file when migrations were applied.
    pub migrated_from: Option<u32>,
}

impl<P> Loaded<P> {
    /// The payload.
    pub fn into_payload(self) -> P {
        self.envelope.payload
    }
}
