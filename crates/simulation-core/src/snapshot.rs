//! Serialised simulation states.
//!
//! A [`Snapshot`] is an opaque, versioned binary image of a model's complete
//! state — including its random generator — so that restoring it reproduces
//! the exact same future. Payloads are encoded with `postcard`, a compact and
//! stable serde format.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::SimError;

/// Versioned binary image of a simulation state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Identifier of the model that produced the snapshot.
    pub model_id: String,
    /// Schema version of the payload, owned by the model.
    pub schema_version: u32,
    /// Tick at which the snapshot was taken.
    pub tick: u64,
    /// `postcard`-encoded state.
    pub payload: Vec<u8>,
}

impl Snapshot {
    /// Encodes `state` into a snapshot.
    pub fn encode<T: Serialize>(model_id: &str, schema_version: u32, tick: u64, state: &T) -> Result<Self, SimError> {
        let payload = postcard::to_stdvec(state).map_err(|e| SimError::Serialization(e.to_string()))?;
        Ok(Self { model_id: model_id.to_string(), schema_version, tick, payload })
    }

    /// Decodes the payload, checking the model identifier and that the schema
    /// version is not newer than `supported_version`. Models that changed
    /// their schema should match on `schema_version` and migrate older data
    /// before calling this with the old type.
    pub fn decode<T: DeserializeOwned>(&self, expected_model: &str, supported_version: u32) -> Result<T, SimError> {
        if self.model_id != expected_model {
            return Err(SimError::WrongModel { expected: expected_model.to_string(), found: self.model_id.clone() });
        }
        if self.schema_version > supported_version {
            return Err(SimError::UnsupportedVersion { found: self.schema_version, supported: supported_version });
        }
        postcard::from_bytes(&self.payload).map_err(|e| SimError::Serialization(e.to_string()))
    }

    /// Approximate memory footprint in bytes.
    pub fn size_bytes(&self) -> usize {
        self.payload.len() + self.model_id.len() + std::mem::size_of::<Self>()
    }

    /// Encodes the whole snapshot (header and payload) to bytes for storage.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SimError> {
        postcard::to_stdvec(self).map_err(|e| SimError::Serialization(e.to_string()))
    }

    /// Decodes a snapshot previously produced by [`Snapshot::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SimError> {
        postcard::from_bytes(bytes).map_err(|e| SimError::Serialization(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SimRng;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct State {
        cells: Vec<u8>,
        rng: SimRng,
    }

    #[test]
    fn round_trip() {
        let state = State { cells: vec![1, 0, 1, 1], rng: SimRng::seed_from_u64(4) };
        let snap = Snapshot::encode("test", 1, 12, &state).unwrap();
        let back: State = snap.decode("test", 1).unwrap();
        assert_eq!(back, state);
        let bytes = snap.to_bytes().unwrap();
        assert_eq!(Snapshot::from_bytes(&bytes).unwrap(), snap);
    }

    #[test]
    fn rejects_wrong_model_and_newer_version() {
        let snap = Snapshot::encode("a", 3, 0, &1u32).unwrap();
        assert!(matches!(snap.decode::<u32>("b", 3), Err(SimError::WrongModel { .. })));
        assert!(matches!(snap.decode::<u32>("a", 2), Err(SimError::UnsupportedVersion { .. })));
    }
}
