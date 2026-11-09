//! Schema migrations.
//!
//! Every document kind has a current schema version. When a file with an
//! older version is loaded, its payload is converted to the JSON data model
//! and passed through the registered steps `v → v + 1` one at a time until it
//! reaches the current version; only then is it decoded into the Rust type.
//! A missing step or a step that rejects the data aborts the load with an
//! error naming the kind and versions involved.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

use crate::envelope::FileKind;
use crate::error::PersistenceError;

/// One migration step: converts a payload of version `v` into version `v + 1`
/// or explains why it cannot.
pub type MigrationFn = dyn Fn(Value) -> Result<Value, String> + Send + Sync;

/// Registered migration steps, keyed by `(kind, from_version)`.
#[derive(Default)]
pub struct MigrationRegistry {
    steps: BTreeMap<(FileKind, u32), Box<MigrationFn>>,
}

impl fmt::Debug for MigrationRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.steps.keys().map(|(kind, from)| format!("{kind} v{from}→v{}", from + 1))).finish()
    }
}

impl MigrationRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the step converting `kind` payloads from `from_version` to
    /// `from_version + 1`. Registering the same step twice is an error.
    pub fn register<F>(&mut self, kind: FileKind, from_version: u32, step: F) -> Result<(), PersistenceError>
    where
        F: Fn(Value) -> Result<Value, String> + Send + Sync + 'static,
    {
        if from_version == u32::MAX {
            return Err(PersistenceError::InvalidData(format!(
                "{kind} schema version {from_version} cannot be upgraded"
            )));
        }
        if self.steps.contains_key(&(kind, from_version)) {
            return Err(PersistenceError::DuplicateMigration { kind, from: from_version });
        }
        self.steps.insert((kind, from_version), Box::new(step));
        Ok(())
    }

    /// Whether the step `from_version → from_version + 1` exists for `kind`.
    pub fn contains(&self, kind: FileKind, from_version: u32) -> bool {
        self.steps.contains_key(&(kind, from_version))
    }

    /// Versions in `from..to` that have no registered step (empty when the
    /// whole chain exists). Useful to validate a registry at start-up.
    pub fn missing_steps(&self, kind: FileKind, from: u32, to: u32) -> Vec<u32> {
        (from..to).filter(|&v| !self.contains(kind, v)).collect()
    }

    /// Applies the steps `from → from + 1 → … → to` in order. The whole
    /// chain is checked before any step runs.
    pub fn migrate(&self, kind: FileKind, from: u32, to: u32, payload: Value) -> Result<Value, PersistenceError> {
        if from > to {
            return Err(PersistenceError::NewerVersion { kind, found: from, current: to });
        }
        if let Some(&missing) = self.missing_steps(kind, from, to).first() {
            return Err(PersistenceError::MissingMigration { kind, from: missing, current: to });
        }
        let mut value = payload;
        for version in from..to {
            // The chain was checked above.
            let Some(step) = self.steps.get(&(kind, version)) else {
                return Err(PersistenceError::MissingMigration { kind, from: version, current: to });
            };
            value = step(value).map_err(|reason| PersistenceError::MigrationFailed { kind, from: version, reason })?;
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chain() -> MigrationRegistry {
        let mut r = MigrationRegistry::new();
        r.register(FileKind::Scenario, 1, |mut v| {
            let obj = v.as_object_mut().ok_or("payload is not an object")?;
            let size = obj.remove("size").ok_or("missing field `size`")?;
            obj.insert("width".into(), size.clone());
            obj.insert("height".into(), size);
            Ok(v)
        })
        .unwrap();
        r.register(FileKind::Scenario, 2, |mut v| {
            v.as_object_mut().ok_or("payload is not an object")?.insert("seed".into(), json!(0));
            Ok(v)
        })
        .unwrap();
        r
    }

    #[test]
    fn applies_steps_in_order() {
        let r = chain();
        let v = r.migrate(FileKind::Scenario, 1, 3, json!({"name": "x", "size": 8})).unwrap();
        assert_eq!(v, json!({"name": "x", "width": 8, "height": 8, "seed": 0}));
        let v = r.migrate(FileKind::Scenario, 2, 3, json!({"width": 1, "height": 2})).unwrap();
        assert_eq!(v, json!({"width": 1, "height": 2, "seed": 0}));
        assert_eq!(r.migrate(FileKind::Scenario, 3, 3, json!(5)).unwrap(), json!(5));
    }

    #[test]
    fn reports_missing_failing_and_duplicate_steps() {
        let mut r = chain();
        assert_eq!(r.missing_steps(FileKind::Scenario, 1, 5), vec![3, 4]);
        assert!(matches!(
            r.migrate(FileKind::Scenario, 1, 4, json!({"size": 1})),
            Err(PersistenceError::MissingMigration { from: 3, current: 4, .. })
        ));
        assert!(matches!(
            r.migrate(FileKind::Settings, 1, 2, json!({})),
            Err(PersistenceError::MissingMigration { .. })
        ));
        let err = r.migrate(FileKind::Scenario, 1, 3, json!([1, 2])).unwrap_err();
        assert!(matches!(&err, PersistenceError::MigrationFailed { from: 1, .. }));
        assert_eq!(
            err.to_string(),
            "upgrading scenario data from schema version 1 to 2 failed: payload is not an object"
        );
        assert!(matches!(r.register(FileKind::Scenario, 1, Ok), Err(PersistenceError::DuplicateMigration { .. })));
        assert!(matches!(r.migrate(FileKind::Scenario, 4, 3, json!({})), Err(PersistenceError::NewerVersion { .. })));
        assert_eq!(format!("{r:?}"), "[\"scenario v1→v2\", \"scenario v2→v3\"]");
    }
}
