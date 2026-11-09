//! Snapshot and snapshot-history files.
//!
//! A snapshot file stores one [`Snapshot`] (model id, model schema version,
//! tick and the model's postcard state) in an envelope of kind
//! [`FileKind::Snapshot`] whose `model_id` must match the snapshot's. The
//! envelope's `schema_version` describes the container layout
//! ([`SNAPSHOT_SCHEMA_VERSION`]); the model's own state version travels inside
//! the snapshot and is checked by the model when it restores it.
//!
//! History files store a sequence of snapshots of one model (e.g. the
//! periodic snapshots kept for rewinding) under [`HistoryLimits`].

use std::path::Path;

use serde::{Deserialize, Serialize};
use simulation_core::Snapshot;

use crate::codec::Codec;
use crate::envelope::{DocumentSpec, Envelope, FileKind, Loaded};
use crate::error::PersistenceError;

/// Schema version of snapshot files written by this build.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Schema version of snapshot-history files written by this build.
pub const HISTORY_SCHEMA_VERSION: u32 = 1;

/// Size limits applied when exporting or importing a snapshot history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryLimits {
    /// Maximum number of snapshots.
    pub max_snapshots: usize,
    /// Maximum total size of the snapshot payloads in bytes.
    pub max_total_bytes: u64,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self { max_snapshots: 10_000, max_total_bytes: 512 * 1024 * 1024 }
    }
}

fn check_snapshot(snapshot: &Snapshot) -> Result<(), PersistenceError> {
    if snapshot.model_id.trim().is_empty() {
        return Err(PersistenceError::InvalidData("the snapshot has no model id".to_string()));
    }
    Ok(())
}

/// Checks that a history is non-empty, belongs to one model, is in tick
/// order and respects the limits.
pub fn validate_history(snapshots: &[Snapshot], limits: &HistoryLimits) -> Result<(), PersistenceError> {
    let Some(first) = snapshots.first() else {
        return Err(PersistenceError::InvalidData("the snapshot history is empty".to_string()));
    };
    if snapshots.len() > limits.max_snapshots {
        return Err(PersistenceError::TooLarge {
            what: "the number of snapshots",
            size: snapshots.len() as u64,
            limit: limits.max_snapshots as u64,
        });
    }
    let total: u64 = snapshots.iter().map(|s| s.payload.len() as u64).sum();
    if total > limits.max_total_bytes {
        return Err(PersistenceError::TooLarge {
            what: "the snapshot history",
            size: total,
            limit: limits.max_total_bytes,
        });
    }
    check_snapshot(first)?;
    for (i, pair) in snapshots.windows(2).enumerate() {
        if pair[1].model_id != first.model_id {
            return Err(PersistenceError::InvalidData(format!(
                "snapshot {} belongs to model `{}`, but the history is for `{}`",
                i + 1,
                pair[1].model_id,
                first.model_id
            )));
        }
        if pair[1].tick < pair[0].tick {
            return Err(PersistenceError::InvalidData(format!(
                "snapshot {} (tick {}) is earlier than the snapshot before it (tick {})",
                i + 1,
                pair[1].tick,
                pair[0].tick
            )));
        }
    }
    Ok(())
}

fn check_model(header_model: Option<&str>, found: &str) -> Result<(), PersistenceError> {
    if header_model != Some(found) {
        return Err(PersistenceError::InvalidData(format!(
            "the file header names model `{}` but the snapshot data belongs to `{found}`",
            header_model.unwrap_or("none")
        )));
    }
    Ok(())
}

impl Codec {
    /// Saves one snapshot (normally to a `.elsnap` file).
    pub fn save_snapshot(&self, path: &Path, snapshot: &Snapshot, created: &str) -> Result<(), PersistenceError> {
        check_snapshot(snapshot).map_err(|e| PersistenceError::at(path, e))?;
        let envelope = Envelope::new(FileKind::Snapshot, SNAPSHOT_SCHEMA_VERSION, created, snapshot)
            .with_model(&snapshot.model_id);
        self.save(path, &envelope)
    }

    /// Loads one snapshot, checking that the header and the snapshot agree on
    /// the model (and match `expected_model` when given).
    pub fn load_snapshot(
        &self,
        path: &Path,
        expected_model: Option<&str>,
    ) -> Result<Loaded<Snapshot>, PersistenceError> {
        let spec = DocumentSpec::new(FileKind::Snapshot, SNAPSHOT_SCHEMA_VERSION);
        let loaded: Loaded<Snapshot> = self.load(path, spec, expected_model)?;
        let envelope = &loaded.envelope;
        check_snapshot(&envelope.payload)
            .and_then(|()| check_model(envelope.model_id.as_deref(), &envelope.payload.model_id))
            .map_err(|e| PersistenceError::at(path, e))?;
        Ok(loaded)
    }

    /// Exports a snapshot history after [`validate_history`].
    pub fn save_history(
        &self,
        path: &Path,
        snapshots: &[Snapshot],
        created: &str,
        limits: &HistoryLimits,
    ) -> Result<(), PersistenceError> {
        validate_history(snapshots, limits).map_err(|e| PersistenceError::at(path, e))?;
        let model = snapshots[0].model_id.as_str();
        let envelope =
            Envelope::new(FileKind::SnapshotHistory, HISTORY_SCHEMA_VERSION, created, snapshots).with_model(model);
        self.save(path, &envelope)
    }

    /// Imports a snapshot history, validating it against `limits`.
    pub fn load_history(
        &self,
        path: &Path,
        expected_model: Option<&str>,
        limits: &HistoryLimits,
    ) -> Result<Loaded<Vec<Snapshot>>, PersistenceError> {
        let spec = DocumentSpec::new(FileKind::SnapshotHistory, HISTORY_SCHEMA_VERSION);
        let loaded: Loaded<Vec<Snapshot>> = self.load(path, spec, expected_model)?;
        let envelope = &loaded.envelope;
        validate_history(&envelope.payload, limits)
            .and_then(|()| check_model(envelope.model_id.as_deref(), &envelope.payload[0].model_id))
            .map_err(|e| PersistenceError::at(path, e))?;
        Ok(loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic::test_dir::TestDir;
    use crate::format::Encoding;
    use simulation_core::SimRng;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct State {
        cells: Vec<u8>,
        rng: SimRng,
    }

    fn snapshot(model: &str, tick: u64) -> Snapshot {
        let state = State { cells: (0..=255).collect(), rng: SimRng::seed_from_u64(tick) };
        Snapshot::encode(model, 2, tick, &state).unwrap()
    }

    #[test]
    fn snapshot_round_trip_restores_state_and_rng() {
        let dir = TestDir::new("snap");
        let codec = Codec::new();
        let path = dir.path().join("save.elsnap");
        let snap = snapshot("life.conway", 120);
        codec.save_snapshot(&path, &snap, "2026-11-09T10:00:00Z").unwrap();
        let loaded = codec.load_snapshot(&path, Some("life.conway")).unwrap();
        assert_eq!(loaded.envelope.model_id.as_deref(), Some("life.conway"));
        assert_eq!(loaded.envelope.kind, FileKind::Snapshot);
        let restored = loaded.into_payload();
        assert_eq!(restored, snap);
        let mut state: State = restored.decode("life.conway", 2).unwrap();
        let mut original: State = snap.decode("life.conway", 2).unwrap();
        assert_eq!(state.cells, original.cells);
        assert_eq!(state.rng.next_u64(), original.rng.next_u64());
        // Also readable as JSON (useful for debugging).
        let json = dir.path().join("save.json");
        codec.save_snapshot(&json, &snap, "now").unwrap();
        assert_eq!(codec.load_snapshot(&json, None).unwrap().into_payload(), snap);
    }

    #[test]
    fn snapshot_header_is_validated() {
        let dir = TestDir::new("snap-validate");
        let codec = Codec::new();
        let path = dir.path().join("save.elsnap");
        codec.save_snapshot(&path, &snapshot("life.conway", 1), "now").unwrap();
        let err = codec.load_snapshot(&path, Some("ecosystem")).unwrap_err();
        assert!(matches!(err.root(), PersistenceError::WrongModel { .. }));
        assert!(matches!(
            codec.load_history(&path, None, &HistoryLimits::default()).unwrap_err().root(),
            PersistenceError::WrongKind { expected: FileKind::SnapshotHistory, found: FileKind::Snapshot }
        ));
        // Header and data disagreeing about the model.
        let forged = Envelope::new(FileKind::Snapshot, SNAPSHOT_SCHEMA_VERSION, "now", snapshot("life.conway", 1))
            .with_model("ecosystem");
        codec.save(&path, &forged).unwrap();
        let err = codec.load_snapshot(&path, None).unwrap_err();
        assert!(matches!(err.root(), PersistenceError::InvalidData(m) if m.contains("ecosystem")));
        // Bit rot is detected.
        codec.save_snapshot(&path, &snapshot("life.conway", 1), "now").unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0x40;
        std::fs::write(&path, &bytes).unwrap();
        assert!(matches!(codec.load_snapshot(&path, None).unwrap_err().root(), PersistenceError::CorruptBinary(_)));
        let nameless = Snapshot { model_id: " ".into(), ..snapshot("x", 0) };
        assert!(codec.save_snapshot(&path, &nameless, "now").is_err());
    }

    #[test]
    fn history_round_trip_and_limits() {
        let dir = TestDir::new("history");
        let codec = Codec::new();
        let path = dir.path().join(format!("history.{}", Encoding::Binary.extension()));
        let history: Vec<Snapshot> = (0..5).map(|i| snapshot("ecosystem", i * 100)).collect();
        let limits = HistoryLimits::default();
        codec.save_history(&path, &history, "now", &limits).unwrap();
        let loaded = codec.load_history(&path, Some("ecosystem"), &limits).unwrap();
        assert_eq!(loaded.into_payload(), history);

        let few = HistoryLimits { max_snapshots: 4, ..limits };
        let err = codec.save_history(&path, &history, "now", &few).unwrap_err();
        assert!(matches!(err.root(), PersistenceError::TooLarge { size: 5, limit: 4, .. }));
        assert!(matches!(codec.load_history(&path, None, &few).unwrap_err().root(), PersistenceError::TooLarge { .. }));
        let small = HistoryLimits { max_total_bytes: 1_000, ..limits };
        assert!(matches!(
            codec.save_history(&path, &history, "now", &small).unwrap_err().root(),
            PersistenceError::TooLarge { what: "the snapshot history", .. }
        ));
    }

    #[test]
    fn history_consistency_rules() {
        let limits = HistoryLimits::default();
        assert!(validate_history(&[], &limits).is_err());
        let mixed = vec![snapshot("a", 0), snapshot("b", 1)];
        assert!(
            matches!(validate_history(&mixed, &limits), Err(PersistenceError::InvalidData(m)) if m.contains("`b`"))
        );
        let backwards = vec![snapshot("a", 5), snapshot("a", 4)];
        assert!(validate_history(&backwards, &limits).is_err());
        let same_tick = vec![snapshot("a", 5), snapshot("a", 5)];
        assert!(validate_history(&same_tick, &limits).is_ok());
    }
}
