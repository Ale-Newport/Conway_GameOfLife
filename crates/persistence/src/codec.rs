//! Saving and loading enveloped documents with validation and migrations.

use std::fs::File;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::atomic::{atomic_write, read_limited};
use crate::binary::{self, BinaryImage};
use crate::envelope::{Document, DocumentSpec, Envelope, EnvelopeHeader, FORMAT_TAG, Loaded};
use crate::error::PersistenceError;
use crate::format::{self, Encoding};
use crate::migrate::MigrationRegistry;

/// Default upper bound on the size of files read by a [`Codec`] (1 GiB).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 1 << 30;

/// Reads only the `format` field, to recognise foreign files (a missing
/// field reads as an empty string).
#[derive(Deserialize)]
struct FormatProbe {
    #[serde(default)]
    format: String,
}

/// Encoder / decoder for Emergence Lab files: chooses the encoding from the
/// file extension, validates headers, applies registered migrations and
/// writes atomically.
#[derive(Debug)]
pub struct Codec {
    migrations: MigrationRegistry,
    max_file_bytes: u64,
}

impl Default for Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec {
    /// Codec without migrations and with [`DEFAULT_MAX_FILE_BYTES`].
    pub fn new() -> Self {
        Self::with_migrations(MigrationRegistry::new())
    }

    /// Codec using the given migrations.
    pub fn with_migrations(migrations: MigrationRegistry) -> Self {
        Self { migrations, max_file_bytes: DEFAULT_MAX_FILE_BYTES }
    }

    /// Registered migrations.
    pub fn migrations(&self) -> &MigrationRegistry {
        &self.migrations
    }

    /// Mutable access to the migrations (to register steps).
    pub fn migrations_mut(&mut self) -> &mut MigrationRegistry {
        &mut self.migrations
    }

    /// Largest file [`Codec::load`] accepts.
    pub fn max_file_bytes(&self) -> u64 {
        self.max_file_bytes
    }

    /// Changes the largest accepted file size.
    pub fn set_max_file_bytes(&mut self, limit: u64) {
        self.max_file_bytes = limit;
    }

    /// Encodes an envelope in memory.
    pub fn encode<P: Serialize>(
        &self,
        envelope: &Envelope<P>,
        encoding: Encoding,
    ) -> Result<Vec<u8>, PersistenceError> {
        if envelope.format != FORMAT_TAG {
            return Err(PersistenceError::InvalidData(format!(
                "the envelope format must be `{FORMAT_TAG}`, not `{}`",
                envelope.format
            )));
        }
        match encoding {
            Encoding::Binary => binary::encode(&envelope.header(), &envelope.payload),
            text => format::to_text(envelope, text).map(String::into_bytes),
        }
    }

    /// Decodes an envelope from memory: checks the header against `spec` and
    /// `expected_model`, migrates older text documents and decodes the payload.
    pub fn decode<P: DeserializeOwned>(
        &self,
        bytes: &[u8],
        encoding: Encoding,
        spec: DocumentSpec,
        expected_model: Option<&str>,
    ) -> Result<Loaded<P>, PersistenceError> {
        if encoding == Encoding::Binary {
            return decode_binary(bytes, spec, expected_model);
        }
        let text = std::str::from_utf8(bytes).map_err(|e| PersistenceError::Syntax {
            format: encoding,
            message: format!("the file is not valid UTF-8 text ({e})"),
        })?;
        let header = read_text_header(text, encoding)?;
        check_header(&header, spec, expected_model)?;
        if header.schema_version == spec.schema_version {
            let envelope: Envelope<P> = format::from_text(text, encoding).map_err(|message| {
                PersistenceError::InvalidPayload { kind: spec.kind, version: spec.schema_version, message }
            })?;
            return Ok(Loaded { envelope, migrated_from: None });
        }
        let mut document = format::to_json_value(text, encoding)?;
        let payload = document.get_mut("payload").map(Value::take).ok_or_else(|| PersistenceError::InvalidPayload {
            kind: spec.kind,
            version: header.schema_version,
            message: "the document has no payload".to_string(),
        })?;
        let migrated = self.migrations.migrate(spec.kind, header.schema_version, spec.schema_version, payload)?;
        let payload: P = serde_json::from_value(migrated).map_err(|e| PersistenceError::InvalidPayload {
            kind: spec.kind,
            version: spec.schema_version,
            message: format!("after upgrading from version {}: {e}", header.schema_version),
        })?;
        let from = header.schema_version;
        let header = EnvelopeHeader { schema_version: spec.schema_version, ..header };
        Ok(Loaded { envelope: Envelope::from_parts(header, payload), migrated_from: Some(from) })
    }

    /// Saves an envelope to `path` atomically, in the encoding selected by
    /// the file extension. The parent folder must exist.
    pub fn save<P: Serialize>(&self, path: &Path, envelope: &Envelope<P>) -> Result<(), PersistenceError> {
        let bytes = Encoding::from_path(path)
            .and_then(|encoding| self.encode(envelope, encoding))
            .map_err(|e| PersistenceError::at(path, e))?;
        atomic_write(path, &bytes)
    }

    /// Loads, validates and (if needed) migrates the document at `path`.
    pub fn load<P: DeserializeOwned>(
        &self,
        path: &Path,
        spec: DocumentSpec,
        expected_model: Option<&str>,
    ) -> Result<Loaded<P>, PersistenceError> {
        let encoding = Encoding::from_path(path).map_err(|e| PersistenceError::at(path, e))?;
        let bytes = read_limited(path, self.max_file_bytes)?;
        self.decode(&bytes, encoding, spec, expected_model).map_err(|e| PersistenceError::at(path, e))
    }

    /// Reads only the header of the file at `path` (e.g. to list saves
    /// without decoding them). Nothing is validated beyond the format tag;
    /// for `.elsnap` files only the first bytes are read and the checksum is
    /// not verified.
    pub fn read_header(&self, path: &Path) -> Result<EnvelopeHeader, PersistenceError> {
        let read = || {
            let encoding = Encoding::from_path(path)?;
            if encoding == Encoding::Binary {
                let file = File::open(path).map_err(|e| PersistenceError::io("open the file", e))?;
                return binary::read_header(file);
            }
            let bytes = read_limited(path, self.max_file_bytes)?;
            let text = std::str::from_utf8(&bytes).map_err(|e| PersistenceError::Syntax {
                format: encoding,
                message: format!("the file is not valid UTF-8 text ({e})"),
            })?;
            read_text_header(text, encoding)
        };
        read().map_err(|e| PersistenceError::at(path, e))
    }

    /// Saves a [`Document`] with a fresh envelope.
    pub fn save_document<D: Document>(
        &self,
        path: &Path,
        payload: &D,
        created: &str,
        model_id: Option<&str>,
    ) -> Result<(), PersistenceError> {
        let mut envelope = Envelope::new(D::KIND, D::SCHEMA_VERSION, created, payload);
        envelope.model_id = model_id.map(str::to_string);
        self.save(path, &envelope)
    }

    /// Loads a [`Document`], migrating older versions.
    pub fn load_document<D: Document>(
        &self,
        path: &Path,
        expected_model: Option<&str>,
    ) -> Result<Loaded<D>, PersistenceError> {
        self.load(path, D::spec(), expected_model)
    }
}

/// Checks the syntax of a text document and decodes its header.
fn read_text_header(text: &str, encoding: Encoding) -> Result<EnvelopeHeader, PersistenceError> {
    format::check_syntax(text, encoding)?;
    let probe: FormatProbe = format::from_text(text, encoding).map_err(|message| {
        PersistenceError::NotEmergenceLab { reason: format!("the document is not a file envelope ({message})") }
    })?;
    match probe.format.as_str() {
        FORMAT_TAG => {}
        "" => {
            return Err(PersistenceError::NotEmergenceLab { reason: "the document has no `format` field".to_string() });
        }
        other => {
            return Err(PersistenceError::NotEmergenceLab { reason: format!("the format field is `{other}`") });
        }
    }
    // Unknown fields (the payload) are skipped by the header decoder.
    format::from_text(text, encoding)
        .map_err(|message| PersistenceError::NotEmergenceLab { reason: format!("invalid file header: {message}") })
}

/// Checks format tag, kind, model and version of a header.
fn check_header(
    header: &EnvelopeHeader,
    spec: DocumentSpec,
    expected_model: Option<&str>,
) -> Result<(), PersistenceError> {
    if header.format != FORMAT_TAG {
        return Err(PersistenceError::NotEmergenceLab { reason: format!("the format field is `{}`", header.format) });
    }
    if header.kind != spec.kind {
        return Err(PersistenceError::WrongKind { expected: spec.kind, found: header.kind });
    }
    if let Some(expected) = expected_model
        && header.model_id.as_deref() != Some(expected)
    {
        return Err(PersistenceError::WrongModel {
            expected: expected.to_string(),
            found: header.model_id.clone().unwrap_or_else(|| "none".to_string()),
        });
    }
    if header.schema_version > spec.schema_version {
        return Err(PersistenceError::NewerVersion {
            kind: spec.kind,
            found: header.schema_version,
            current: spec.schema_version,
        });
    }
    Ok(())
}

fn decode_binary<P: DeserializeOwned>(
    bytes: &[u8],
    spec: DocumentSpec,
    expected_model: Option<&str>,
) -> Result<Loaded<P>, PersistenceError> {
    let image = BinaryImage::parse(bytes)?;
    check_header(&image.header, spec, expected_model)?;
    if image.header.schema_version < spec.schema_version {
        return Err(PersistenceError::BinaryMigration {
            kind: spec.kind,
            found: image.header.schema_version,
            current: spec.schema_version,
        });
    }
    let payload: P = image.decode_payload()?;
    Ok(Loaded { envelope: Envelope::from_parts(image.header, payload), migrated_from: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic::test_dir::TestDir;
    use crate::envelope::FileKind;
    use serde_json::json;

    /// Current (v3) scenario schema used by the migration tests.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct ScenarioV3 {
        name: String,
        width: u32,
        height: u32,
        seed: u64,
        topology: Topology,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum Topology {
        Bounded,
        Torus { wrap_x: bool, wrap_y: bool },
    }

    /// v1 → v2: `size` split into `width` and `height`.
    /// v2 → v3: `seed` added (default 0), `wrap: bool` replaced by `topology`.
    fn registry() -> MigrationRegistry {
        let mut r = MigrationRegistry::new();
        r.register(FileKind::Scenario, 1, |mut v| {
            let obj = v.as_object_mut().ok_or("expected a map")?;
            let size = obj.remove("size").ok_or("missing field `size`")?;
            obj.insert("width".into(), size.clone());
            obj.insert("height".into(), size);
            Ok(v)
        })
        .unwrap();
        r.register(FileKind::Scenario, 2, |mut v| {
            let obj = v.as_object_mut().ok_or("expected a map")?;
            obj.insert("seed".into(), json!(0));
            let wrap = obj.remove("wrap").and_then(|w| w.as_bool()).ok_or("missing boolean field `wrap`")?;
            let topology = if wrap { json!({"Torus": {"wrap_x": true, "wrap_y": true}}) } else { json!("Bounded") };
            obj.insert("topology".into(), topology);
            Ok(v)
        })
        .unwrap();
        r
    }

    const SPEC: DocumentSpec = DocumentSpec { kind: FileKind::Scenario, schema_version: 3 };

    fn v3() -> ScenarioV3 {
        ScenarioV3 {
            name: "soup".into(),
            width: 64,
            height: 32,
            seed: 9,
            topology: Topology::Torus { wrap_x: true, wrap_y: false },
        }
    }

    #[test]
    fn current_documents_round_trip_in_every_encoding() {
        let dir = TestDir::new("codec-roundtrip");
        let codec = Codec::new();
        let envelope = Envelope::new(FileKind::Scenario, 3, "2026-11-09T10:00:00Z", v3()).with_model("life.conway");
        for ext in ["ron", "json", "toml", "elsnap"] {
            let path = dir.path().join(format!("scenario.{ext}"));
            codec.save(&path, &envelope).unwrap();
            let loaded: Loaded<ScenarioV3> = codec.load(&path, SPEC, Some("life.conway")).unwrap();
            assert_eq!(loaded.envelope, envelope, "{ext}");
            assert_eq!(loaded.migrated_from, None);
            let header = codec.read_header(&path).unwrap();
            assert_eq!(header, envelope.header(), "{ext}");
        }
    }

    #[test]
    fn migrates_v1_and_v2_files_step_by_step() {
        let dir = TestDir::new("codec-migrate");
        let codec = Codec::with_migrations(registry());
        let v1 = Envelope::new(FileKind::Scenario, 1, "old", json!({"name": "soup", "size": 16, "wrap": true}));
        let v2 = Envelope::new(
            FileKind::Scenario,
            2,
            "old",
            json!({"name": "soup", "width": 8, "height": 4, "wrap": false}),
        );
        for ext in ["json", "ron", "toml"] {
            let p1 = dir.path().join(format!("v1.{ext}"));
            codec.save(&p1, &v1).unwrap();
            let loaded: Loaded<ScenarioV3> = codec.load(&p1, SPEC, None).unwrap();
            assert_eq!(loaded.migrated_from, Some(1), "{ext}");
            assert_eq!(loaded.envelope.schema_version, 3);
            assert_eq!(loaded.envelope.created, "old");
            assert_eq!(
                loaded.envelope.payload,
                ScenarioV3 {
                    name: "soup".into(),
                    width: 16,
                    height: 16,
                    seed: 0,
                    topology: Topology::Torus { wrap_x: true, wrap_y: true }
                }
            );
            let p2 = dir.path().join(format!("v2.{ext}"));
            codec.save(&p2, &v2).unwrap();
            let loaded: Loaded<ScenarioV3> = codec.load(&p2, SPEC, None).unwrap();
            assert_eq!(loaded.migrated_from, Some(2));
            assert_eq!(
                loaded.envelope.payload,
                ScenarioV3 { name: "soup".into(), width: 8, height: 4, seed: 0, topology: Topology::Bounded }
            );
        }
    }

    #[test]
    fn ron_enums_survive_migration() {
        // A v2 RON file written by hand, with enum syntax in the payload.
        let text = r#"(
            format: "emergence-lab",
            kind: scenario,
            schema_version: 2,
            app_version: "0.0.9",
            model_id: None,
            created: "2025",
            payload: (name: "x", width: 3, height: 3, wrap: true, extra: Some(Torus(wrap_x: true, wrap_y: false))),
        )"#;
        let mut r = registry();
        // A step that must see RON enums in serde's JSON representation.
        let mut codec = Codec::new();
        r.register(FileKind::Pattern, 1, |v| {
            if v["extra"] == json!({"Torus": {"wrap_x": true, "wrap_y": false}}) {
                Ok(v)
            } else {
                Err(format!("unexpected {v}"))
            }
        })
        .unwrap();
        *codec.migrations_mut() = r;
        let loaded: Loaded<ScenarioV3> = codec.decode(text.as_bytes(), Encoding::Ron, SPEC, None).unwrap();
        assert_eq!(loaded.envelope.payload.topology, Topology::Torus { wrap_x: true, wrap_y: true });
        let pattern = text.replace("kind: scenario", "kind: pattern").replace("schema_version: 2", "schema_version: 1");
        let spec = DocumentSpec::new(FileKind::Pattern, 2);
        let loaded: Loaded<Value> = codec.decode(pattern.as_bytes(), Encoding::Ron, spec, None).unwrap();
        assert_eq!(loaded.envelope.payload["extra"], json!({"Torus": {"wrap_x": true, "wrap_y": false}}));
    }

    #[test]
    fn hand_written_ron_with_struct_names_and_comments_loads() {
        // Scenario authors often write struct names and comments; the header
        // probe must skip such payloads and the typed decode must accept them.
        let text = r#"// Soup on a torus.
        (
            format: "emergence-lab",
            kind: scenario,
            schema_version: 3,
            app_version: "0.1.0",
            model_id: Some("life.conway"),
            created: "2026-11-09",
            payload: ScenarioV3(
                name: "soup", /* inline comment */
                width: 64,
                height: 32,
                seed: 9,
                topology: Torus(wrap_x: true, wrap_y: false),
            ),
        )"#;
        let codec = Codec::new();
        let loaded: Loaded<ScenarioV3> =
            codec.decode(text.as_bytes(), Encoding::Ron, SPEC, Some("life.conway")).unwrap();
        assert_eq!(loaded.envelope.payload, v3());
        assert_eq!(loaded.envelope.created, "2026-11-09");
    }

    #[test]
    fn header_validation_errors() {
        let codec = Codec::with_migrations(registry());
        let envelope = Envelope::new(FileKind::Scenario, 3, "now", v3()).with_model("life.conway");
        let json = codec.encode(&envelope, Encoding::Json).unwrap();
        let wrong_kind =
            codec.decode::<ScenarioV3>(&json, Encoding::Json, DocumentSpec::new(FileKind::Settings, 3), None);
        assert!(matches!(
            wrong_kind,
            Err(PersistenceError::WrongKind { expected: FileKind::Settings, found: FileKind::Scenario })
        ));
        let wrong_model = codec.decode::<ScenarioV3>(&json, Encoding::Json, SPEC, Some("ecosystem"));
        assert!(matches!(wrong_model, Err(PersistenceError::WrongModel { .. })));
        let older_build =
            codec.decode::<ScenarioV3>(&json, Encoding::Json, DocumentSpec::new(FileKind::Scenario, 2), None);
        assert!(matches!(older_build, Err(PersistenceError::NewerVersion { found: 3, current: 2, .. })));
        let missing_step =
            codec.decode::<ScenarioV3>(&json, Encoding::Json, DocumentSpec::new(FileKind::Scenario, 4), None);
        assert!(matches!(missing_step, Err(PersistenceError::MissingMigration { from: 3, current: 4, .. })));
        let foreign = br#"{"name": "not ours"}"#;
        assert!(matches!(
            codec.decode::<Value>(foreign, Encoding::Json, SPEC, None),
            Err(PersistenceError::NotEmergenceLab { .. })
        ));
        let other_tag = br#"{"format": "golly", "kind": "scenario"}"#;
        let err = codec.decode::<Value>(other_tag, Encoding::Json, SPEC, None).unwrap_err();
        assert_eq!(err.to_string(), "not an Emergence Lab file: the format field is `golly`");
        assert!(matches!(
            codec.decode::<Value>(b"{\"format\": ", Encoding::Json, SPEC, None),
            Err(PersistenceError::Syntax { .. })
        ));
        assert!(matches!(
            codec.decode::<Value>(&[0xff, 0xfe], Encoding::Ron, SPEC, None),
            Err(PersistenceError::Syntax { .. })
        ));
        let mut bad = envelope;
        bad.format = "other".into();
        assert!(codec.encode(&bad, Encoding::Json).is_err());
    }

    #[test]
    fn payload_errors_name_the_schema() {
        let codec = Codec::with_migrations(registry());
        let wrong = Envelope::new(FileKind::Scenario, 3, "now", json!({"name": "x"}));
        let bytes = codec.encode(&wrong, Encoding::Ron).unwrap();
        let err = codec.decode::<ScenarioV3>(&bytes, Encoding::Ron, SPEC, None).unwrap_err();
        assert!(matches!(err, PersistenceError::InvalidPayload { version: 3, .. }), "{err}");
        // A v1 file whose data cannot be migrated.
        let broken = Envelope::new(FileKind::Scenario, 1, "now", json!({"name": "x"}));
        let bytes = codec.encode(&broken, Encoding::Json).unwrap();
        let err = codec.decode::<ScenarioV3>(&bytes, Encoding::Json, SPEC, None).unwrap_err();
        assert_eq!(err.to_string(), "upgrading scenario data from schema version 1 to 2 failed: missing field `size`");
    }

    #[test]
    fn binary_files_are_not_migrated() {
        let codec = Codec::with_migrations(registry());
        let old = Envelope::new(FileKind::Scenario, 2, "now", v3());
        let bytes = codec.encode(&old, Encoding::Binary).unwrap();
        let err = codec.decode::<ScenarioV3>(&bytes, Encoding::Binary, SPEC, None).unwrap_err();
        assert!(matches!(err, PersistenceError::BinaryMigration { found: 2, current: 3, .. }));
        let truncated =
            &codec.encode(&Envelope::new(FileKind::Scenario, 3, "now", v3()), Encoding::Binary).unwrap()[..40];
        assert!(matches!(
            codec.decode::<ScenarioV3>(truncated, Encoding::Binary, SPEC, None),
            Err(PersistenceError::CorruptBinary(_))
        ));
    }

    #[test]
    fn file_errors_carry_the_path() {
        let dir = TestDir::new("codec-paths");
        let mut codec = Codec::new();
        let missing = dir.path().join("missing.ron");
        let err = codec.load::<ScenarioV3>(&missing, SPEC, None).unwrap_err();
        assert_eq!(err.path(), Some(missing.as_path()));
        assert!(err.to_string().starts_with(&missing.display().to_string()), "{err}");
        let txt = dir.path().join("notes.txt");
        let err = codec.save(&txt, &Envelope::new(FileKind::Scenario, 3, "now", v3())).unwrap_err();
        assert!(matches!(err.root(), PersistenceError::UnsupportedExtension { .. }));
        assert_eq!(err.path(), Some(txt.as_path()));
        let big = dir.path().join("big.json");
        codec.save(&big, &Envelope::new(FileKind::Scenario, 3, "now", v3())).unwrap();
        codec.set_max_file_bytes(10);
        let err = codec.load::<ScenarioV3>(&big, SPEC, None).unwrap_err();
        assert!(matches!(err.root(), PersistenceError::TooLarge { limit: 10, .. }));
        let corrupt = dir.path().join("corrupt.json");
        std::fs::write(&corrupt, "{ not json").unwrap();
        codec.set_max_file_bytes(DEFAULT_MAX_FILE_BYTES);
        let err = codec.load::<ScenarioV3>(&corrupt, SPEC, None).unwrap_err();
        assert!(matches!(err.root(), PersistenceError::Syntax { format: Encoding::Json, .. }));
        assert!(err.to_string().contains("corrupt.json"));
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Settings {
        theme: String,
        ui_scale: f32,
        recent: Vec<String>,
        autosave_minutes: Option<u32>,
    }

    impl Document for Settings {
        const KIND: FileKind = FileKind::Settings;
        const SCHEMA_VERSION: u32 = 1;
    }

    #[test]
    fn typed_documents_and_toml_settings() {
        let dir = TestDir::new("codec-settings");
        let codec = Codec::new();
        let path = dir.path().join("settings.toml");
        let settings =
            Settings { theme: "dark".into(), ui_scale: 1.25, recent: vec!["a.ron".into()], autosave_minutes: None };
        codec.save_document(&path, &settings, "now", None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("format = \"emergence-lab\""), "{text}");
        assert!(text.contains("kind = \"settings\""), "{text}");
        let loaded = codec.load_document::<Settings>(&path, None).unwrap();
        assert_eq!(loaded.into_payload(), settings);
    }
}
