//! Encodings and their text (de)serialisation.
//!
//! * **RON** (`.ron`) — default for scenarios, rule sets and patterns; written
//!   pretty-printed with the `unwrap_newtypes` extension so that newtype
//!   structs read like their inner value.
//! * **JSON** (`.json`) — for interchange with other tools.
//! * **TOML** (`.toml`) — for settings. TOML has no `null`: `None` fields are
//!   omitted, and unit values cannot be written.
//! * **Binary** (`.elsnap`) — see [`crate::binary`].
//!
//! Migrations operate on the JSON data model (`serde_json::Value`). RON
//! documents are converted with serde's externally tagged enum convention
//! (`Variant(x)` becomes `{"Variant": x}`, a unit variant becomes the string
//! `"Variant"`), which is how `serde_json` itself represents enums.

use std::fmt;
use std::path::Path;

use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::envelope::FileKind;
use crate::error::PersistenceError;

/// File encoding, selected by extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    /// Rusty Object Notation (`.ron`).
    Ron,
    /// JSON (`.json`).
    Json,
    /// TOML (`.toml`).
    Toml,
    /// Binary `.elsnap` container with a postcard payload.
    Binary,
}

impl Encoding {
    /// Encoding selected by the extension of `path` (case-insensitive).
    pub fn from_path(path: &Path) -> Result<Self, PersistenceError> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "ron" => Ok(Self::Ron),
            "json" => Ok(Self::Json),
            "toml" => Ok(Self::Toml),
            "elsnap" => Ok(Self::Binary),
            _ => Err(PersistenceError::UnsupportedExtension { found: ext }),
        }
    }

    /// Canonical file extension (without the dot).
    pub fn extension(self) -> &'static str {
        match self {
            Self::Ron => "ron",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Binary => "elsnap",
        }
    }

    /// Recommended encoding for a kind of document: TOML for settings,
    /// binary for snapshots, RON otherwise.
    pub fn preferred_for(kind: FileKind) -> Self {
        match kind {
            FileKind::Settings => Self::Toml,
            FileKind::Snapshot | FileKind::SnapshotHistory => Self::Binary,
            _ => Self::Ron,
        }
    }

    /// Whether the encoding is human-readable text.
    pub fn is_text(self) -> bool {
        self != Self::Binary
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ron => "RON",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Binary => "binary",
        })
    }
}

/// Serialises a value as pretty-printed text in a text encoding.
pub(crate) fn to_text<T: Serialize>(value: &T, format: Encoding) -> Result<String, PersistenceError> {
    let encode_err = |message: String| PersistenceError::Encode { format, message };
    match format {
        Encoding::Ron => {
            let config = ron::ser::PrettyConfig::default().extensions(ron::extensions::Extensions::UNWRAP_NEWTYPES);
            ron::ser::to_string_pretty(value, config).map_err(|e| encode_err(e.to_string()))
        }
        Encoding::Json => serde_json::to_string_pretty(value).map_err(|e| encode_err(e.to_string())),
        Encoding::Toml => toml::to_string_pretty(value).map_err(|e| encode_err(e.to_string())),
        Encoding::Binary => Err(encode_err("binary is not a text encoding".to_string())),
    }
}

/// Checks that `text` is well-formed in `format`, so that later decoding
/// failures can be reported as data errors rather than syntax errors.
pub(crate) fn check_syntax(text: &str, format: Encoding) -> Result<(), PersistenceError> {
    let syntax = |message: String| PersistenceError::Syntax { format, message };
    match format {
        Encoding::Ron => ron::from_str::<IgnoredAny>(text).map(drop).map_err(|e| syntax(e.to_string())),
        Encoding::Json => serde_json::from_str::<IgnoredAny>(text).map(drop).map_err(|e| syntax(e.to_string())),
        Encoding::Toml => toml::from_str::<toml::Table>(text).map(drop).map_err(|e| syntax(e.to_string())),
        Encoding::Binary => Err(syntax("binary is not a text encoding".to_string())),
    }
}

/// Deserialises a typed value from text that passed [`check_syntax`]; the
/// error message describes the mismatch between the data and `T`.
pub(crate) fn from_text<T: DeserializeOwned>(text: &str, format: Encoding) -> Result<T, String> {
    match format {
        Encoding::Ron => ron::from_str(text).map_err(|e| e.to_string()),
        Encoding::Json => serde_json::from_str(text).map_err(|e| e.to_string()),
        Encoding::Toml => toml::from_str(text).map_err(|e| e.to_string()),
        Encoding::Binary => Err("binary is not a text encoding".to_string()),
    }
}

/// Wrapper that makes RON switch to serde's externally tagged enum
/// representation while building a JSON value (RON only does so for
/// untagged/buffered content).
#[derive(Deserialize)]
#[serde(untagged)]
enum RonAsJson {
    Value(Value),
}

/// Parses text into the JSON data model used by migrations.
pub(crate) fn to_json_value(text: &str, format: Encoding) -> Result<Value, PersistenceError> {
    let syntax = |message: String| PersistenceError::Syntax { format, message };
    match format {
        Encoding::Ron => {
            ron::from_str::<RonAsJson>(text).map(|RonAsJson::Value(v)| v).map_err(|e| syntax(e.to_string()))
        }
        Encoding::Json => serde_json::from_str(text).map_err(|e| syntax(e.to_string())),
        Encoding::Toml => {
            let table: toml::Table = toml::from_str(text).map_err(|e| syntax(e.to_string()))?;
            serde_json::to_value(table).map_err(|e| syntax(e.to_string()))
        }
        Encoding::Binary => Err(syntax("binary is not a text encoding".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Meters(f64);

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum Shape {
        Circle(Meters),
        Rect { w: f64, h: f64 },
        Empty,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Doc {
        name: String,
        size: Meters,
        shapes: Vec<Shape>,
        tag: Option<String>,
    }

    fn doc() -> Doc {
        Doc {
            name: "x".into(),
            size: Meters(2.5),
            shapes: vec![Shape::Circle(Meters(1.0)), Shape::Rect { w: 1.0, h: 2.0 }, Shape::Empty],
            tag: Some("t".into()),
        }
    }

    #[test]
    fn extensions_select_encodings() {
        assert_eq!(Encoding::from_path(Path::new("a/b.RON")).unwrap(), Encoding::Ron);
        assert_eq!(Encoding::from_path(Path::new("b.json")).unwrap(), Encoding::Json);
        assert_eq!(Encoding::from_path(Path::new("settings.toml")).unwrap(), Encoding::Toml);
        assert_eq!(Encoding::from_path(Path::new("save.elsnap")).unwrap(), Encoding::Binary);
        assert!(matches!(
            Encoding::from_path(Path::new("notes.txt")),
            Err(PersistenceError::UnsupportedExtension { found }) if found == "txt"
        ));
        assert!(Encoding::from_path(Path::new("no_extension")).is_err());
        assert_eq!(Encoding::preferred_for(FileKind::Settings), Encoding::Toml);
        assert_eq!(Encoding::preferred_for(FileKind::Snapshot), Encoding::Binary);
        assert_eq!(Encoding::preferred_for(FileKind::Scenario), Encoding::Ron);
    }

    #[test]
    fn typed_round_trip_in_every_text_format() {
        for format in [Encoding::Ron, Encoding::Json, Encoding::Toml] {
            let text = to_text(&doc(), format).unwrap();
            check_syntax(&text, format).unwrap();
            let back: Doc = from_text(&text, format).unwrap_or_else(|e| panic!("{format} decode failed: {e}\n{text}"));
            assert_eq!(back, doc(), "{format}");
        }
    }

    #[test]
    fn ron_converts_to_externally_tagged_json() {
        let text = to_text(&doc(), Encoding::Ron).unwrap();
        assert!(text.starts_with("#![enable(unwrap_newtypes)]"));
        let value = to_json_value(&text, Encoding::Ron).unwrap();
        assert_eq!(value, serde_json::to_value(doc()).unwrap());
        let back: Doc = serde_json::from_value(value).unwrap();
        assert_eq!(back, doc());
    }

    #[test]
    fn toml_and_json_convert_to_json_values() {
        for format in [Encoding::Json, Encoding::Toml] {
            let text = to_text(&doc(), format).unwrap();
            let value = to_json_value(&text, format).unwrap();
            let back: Doc = serde_json::from_value(value).unwrap();
            assert_eq!(back, doc(), "{format}");
        }
    }

    #[test]
    fn syntax_and_data_errors_are_distinguished() {
        for (format, broken, wrong_shape) in [
            (Encoding::Json, "{\"name\": ", "{\"name\": 3}"),
            (Encoding::Ron, "(name: \"x\", size: [", "(name: 3)"),
            (Encoding::Toml, "name = [", "name = 3"),
        ] {
            assert!(matches!(check_syntax(broken, format), Err(PersistenceError::Syntax { .. })), "{format} syntax");
            assert!(check_syntax(wrong_shape, format).is_ok(), "{format} well-formed");
            assert!(from_text::<Doc>(wrong_shape, format).is_err(), "{format} data");
            assert!(matches!(to_json_value(broken, format), Err(PersistenceError::Syntax { .. })));
        }
    }
}
