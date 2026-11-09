//! The pattern catalog (`assets/patterns/catalog.toml`) and the embedded
//! fallback library.
//!
//! The catalog is a TOML file with an array of tables named `pattern` (or
//! `patterns`). Only `id`, `name` and `file` are required; every other field
//! is optional, and unknown fields are kept as text in
//! [`CatalogEntry::extra`]. `file` is relative to the catalog's directory and
//! must stay inside it. An optional `[library]` table may give the library
//! `name` and a default `rule` for files without one.
//!
//! ```toml
//! [library]
//! name = "Life patterns"
//! rule = "B3/S23"
//!
//! [[pattern]]
//! id = "glider"
//! name = "Glider"
//! category = "spaceships"
//! kind = "spaceship"
//! period = 4
//! dx = 1            # or: displacement = [1, 1]
//! dy = 1
//! file = "life/spaceships/glider.rle"
//! ```

mod builtin;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use toml::{Table, Value};

pub use builtin::{BuiltinPattern, builtin_patterns};

use crate::error::PatternError;
use crate::format::read_pattern_file;
use crate::pattern::Pattern;

/// Metadata of one catalog pattern.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Unique identifier (slug).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Pattern file, relative to the catalog directory.
    pub file: String,
    /// Category, e.g. `"spaceships"`.
    pub category: Option<String>,
    /// Kind, e.g. `"still_life"`, `"oscillator"`, `"spaceship"`, `"gun"`.
    pub kind: Option<String>,
    /// Short description.
    pub description: Option<String>,
    /// Discoverer.
    pub discoverer: Option<String>,
    /// Year of discovery.
    pub year: Option<i32>,
    /// Period as stated in the catalog (the bundled catalog uses 0 for still
    /// lifes and for patterns without a period).
    pub period: Option<u64>,
    /// Displacement per period `(dx, dy)` (from `dx`/`dy` or `displacement`).
    pub displacement: Option<(i64, i64)>,
    /// Bounding-box width of the file's cells.
    pub width: Option<u64>,
    /// Bounding-box height of the file's cells.
    pub height: Option<u64>,
    /// Initial population.
    pub population: Option<u64>,
    /// Human-readable summary of the expected behaviour.
    pub expected_behavior: Option<String>,
    /// Complexity rating (the bundled catalog uses 1 = tiny classic .. 5).
    pub complexity: Option<u32>,
    /// Where the cell data came from.
    pub source: Option<String>,
    /// Licence of the cell data / file.
    pub license: Option<String>,
    /// Rule for files that do not state one.
    pub rule: Option<String>,
    /// Any other fields, as text (strings verbatim, other values in TOML syntax).
    pub extra: BTreeMap<String, String>,
}

/// Where a catalog's pattern files live.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Origin {
    Directory(PathBuf),
    Builtin,
}

/// A loaded pattern catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    origin: Origin,
    name: Option<String>,
    default_rule: Option<String>,
    entries: Vec<CatalogEntry>,
}

/// Fields with a dedicated [`CatalogEntry`] member.
const KNOWN_FIELDS: &[&str] = &[
    "id",
    "name",
    "file",
    "category",
    "kind",
    "description",
    "discoverer",
    "year",
    "period",
    "dx",
    "dy",
    "displacement",
    "width",
    "height",
    "population",
    "expected_behavior",
    "complexity",
    "source",
    "license",
    "rule",
];

/// Typed access to one `[[pattern]]` table with precise error messages.
struct EntryFields<'a> {
    table: &'a Table,
    label: String,
}

impl EntryFields<'_> {
    fn error(&self, message: impl std::fmt::Display) -> String {
        format!("{}: {message}", self.label)
    }

    fn text(&self, key: &str) -> Result<Option<String>, String> {
        match self.table.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(v @ (Value::Integer(_) | Value::Float(_) | Value::Boolean(_))) => Ok(Some(v.to_string())),
            Some(_) => Err(self.error(format!("field `{key}` must be text"))),
        }
    }

    fn required_text(&self, key: &str) -> Result<String, String> {
        match self.text(key)? {
            Some(text) if !text.trim().is_empty() => Ok(text),
            _ => Err(self.error(format!("missing required field `{key}`"))),
        }
    }

    fn integer<T: TryFrom<i64>>(&self, key: &str) -> Result<Option<T>, String> {
        let value = match self.table.get(key) {
            None => return Ok(None),
            Some(Value::Integer(i)) => *i,
            Some(Value::String(s)) => {
                s.trim().parse().map_err(|_| self.error(format!("field `{key}` must be an integer, got \"{s}\"")))?
            }
            Some(other) => {
                return Err(self.error(format!("field `{key}` must be an integer, got {}", other.type_str())));
            }
        };
        T::try_from(value).map(Some).map_err(|_| self.error(format!("field `{key}` is out of range: {value}")))
    }

    fn displacement(&self) -> Result<Option<(i64, i64)>, String> {
        let pair = match (self.integer::<i64>("dx")?, self.integer::<i64>("dy")?) {
            (Some(dx), Some(dy)) => Some((dx, dy)),
            (None, None) => None,
            _ => return Err(self.error("`dx` and `dy` must be given together")),
        };
        let array = match self.table.get("displacement") {
            None => None,
            Some(Value::Array(items)) => match items.as_slice() {
                [Value::Integer(dx), Value::Integer(dy)] => Some((*dx, *dy)),
                _ => return Err(self.error("`displacement` must be an array of two integers [dx, dy]")),
            },
            Some(_) => return Err(self.error("`displacement` must be an array of two integers [dx, dy]")),
        };
        match (pair, array) {
            (Some(a), Some(b)) if a != b => {
                Err(self.error(format!("`dx`/`dy` {a:?} disagree with `displacement` {b:?}")))
            }
            (a, b) => Ok(a.or(b)),
        }
    }

    fn entry(&self) -> Result<CatalogEntry, String> {
        let file = self.required_text("file")?;
        validate_relative_path(&file).map_err(|reason| self.error(format!("`file` {reason}")))?;
        let extra = self
            .table
            .iter()
            .filter(|(key, _)| !KNOWN_FIELDS.contains(&key.as_str()))
            .map(|(key, value)| {
                let text = match value {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (key.clone(), text)
            })
            .collect();
        Ok(CatalogEntry {
            id: self.required_text("id")?,
            name: self.required_text("name")?,
            file,
            category: self.text("category")?,
            kind: self.text("kind")?,
            description: self.text("description")?,
            discoverer: self.text("discoverer")?,
            year: self.integer("year")?,
            period: self.integer("period")?,
            displacement: self.displacement()?,
            width: self.integer("width")?,
            height: self.integer("height")?,
            population: self.integer("population")?,
            expected_behavior: self.text("expected_behavior")?,
            complexity: self.integer("complexity")?,
            source: self.text("source")?,
            license: self.text("license")?,
            rule: self.text("rule")?,
            extra,
        })
    }
}

/// Checks that `file` is a non-empty relative path without `..`.
fn validate_relative_path(file: &str) -> Result<(), String> {
    let path = Path::new(file);
    if file.trim().is_empty() {
        return Err("is empty".into());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Err(format!("`{file}` must not contain `..`")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("`{file}` must be relative to the catalog directory"));
            }
        }
    }
    Ok(())
}

/// 1-based line and column of a byte offset.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

impl Catalog {
    /// Loads a catalog file. `path` may also be a directory containing
    /// `catalog.toml`. Pattern files are resolved relative to the catalog's
    /// directory.
    pub fn load(path: impl AsRef<Path>) -> Result<Catalog, PatternError> {
        let mut path = path.as_ref().to_path_buf();
        if path.is_dir() {
            path.push("catalog.toml");
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| PatternError::Io { path: path.display().to_string(), message: e.to_string() })?;
        let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Self::parse_toml(&text, root, &path.display().to_string())
    }

    /// Parses catalog TOML whose pattern files live under `root`.
    pub fn from_toml_str(text: &str, root: impl Into<PathBuf>) -> Result<Catalog, PatternError> {
        let root = root.into();
        let label = root.join("catalog.toml").display().to_string();
        Self::parse_toml(text, root, &label)
    }

    fn parse_toml(text: &str, root: PathBuf, path_label: &str) -> Result<Catalog, PatternError> {
        let fail = |message: String| PatternError::Catalog { path: path_label.to_string(), message };
        let document: Table = toml::from_str(text).map_err(|e| {
            let (line, column) = e.span().map_or((0, 0), |span| line_column(text, span.start));
            fail(format!("TOML syntax error at line {line}, column {column}: {}", e.message()))
        })?;
        let array = match (document.get("pattern"), document.get("patterns")) {
            (Some(_), Some(_)) => return Err(fail("use either [[pattern]] or [[patterns]], not both".into())),
            (Some(value), None) | (None, Some(value)) => value,
            (None, None) => return Err(fail("no [[pattern]] entries found".into())),
        };
        let Value::Array(items) = array else {
            return Err(fail("`pattern` must be an array of tables ([[pattern]])".into()));
        };
        let mut entries = Vec::with_capacity(items.len());
        let mut ids = BTreeSet::new();
        for (index, item) in items.iter().enumerate() {
            let Value::Table(table) = item else {
                return Err(fail(format!("entry {} is not a table", index + 1)));
            };
            let label = match table.get("id").and_then(Value::as_str) {
                Some(id) => format!("entry {} (`{id}`)", index + 1),
                None => format!("entry {}", index + 1),
            };
            let entry = EntryFields { table, label }.entry().map_err(fail)?;
            if !ids.insert(entry.id.clone()) {
                return Err(fail(format!("entry {}: duplicate id `{}`", index + 1, entry.id)));
            }
            entries.push(entry);
        }
        let library = document.get("library").and_then(Value::as_table);
        let library_text = |key: &str| library.and_then(|t| t.get(key)).and_then(Value::as_str).map(str::to_string);
        Ok(Catalog {
            origin: Origin::Directory(root),
            name: library_text("name"),
            default_rule: library_text("rule"),
            entries,
        })
    }

    /// The embedded library of classic Life patterns (see
    /// [`builtin_patterns`]); it needs no files.
    pub fn builtin() -> Catalog {
        Catalog {
            origin: Origin::Builtin,
            name: Some("Built-in classic patterns".to_string()),
            default_rule: Some("B3/S23".to_string()),
            entries: builtin_patterns().iter().map(BuiltinPattern::entry).collect(),
        }
    }

    /// All entries, in file order.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entry with the given id.
    pub fn by_id(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Distinct categories in order of first appearance.
    pub fn categories(&self) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        self.entries
            .iter()
            .filter_map(|entry| entry.category.as_deref())
            .filter(|category| seen.insert(*category))
            .collect()
    }

    /// Entries of one category, in file order.
    pub fn in_category<'a>(&'a self, category: &'a str) -> impl Iterator<Item = &'a CatalogEntry> + 'a {
        self.entries.iter().filter(move |entry| entry.category.as_deref() == Some(category))
    }

    /// Library name from the `[library]` table.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Directory the pattern files are resolved against (`None` for the
    /// built-in catalog).
    pub fn root(&self) -> Option<&Path> {
        match &self.origin {
            Origin::Directory(root) => Some(root),
            Origin::Builtin => None,
        }
    }

    /// Whether this is the embedded catalog.
    pub fn is_builtin(&self) -> bool {
        self.origin == Origin::Builtin
    }

    /// Path of an entry's pattern file (`None` for the built-in catalog).
    pub fn pattern_path(&self, entry: &CatalogEntry) -> Option<PathBuf> {
        self.root().map(|root| root.join(&entry.file))
    }

    /// Reads and parses an entry's pattern. Metadata missing from the file is
    /// filled from the entry: name, description, author (discoverer) and rule
    /// (the entry's `rule`, else the library default).
    pub fn load_pattern(&self, entry: &CatalogEntry) -> Result<Pattern, PatternError> {
        let mut pattern = match &self.origin {
            Origin::Builtin => {
                let builtin =
                    builtin_patterns().iter().find(|b| b.id == entry.id).ok_or_else(|| PatternError::Catalog {
                        path: "<built-in>".into(),
                        message: format!("no built-in pattern with id `{}`", entry.id),
                    })?;
                builtin.pattern()?
            }
            Origin::Directory(root) => {
                validate_relative_path(&entry.file).map_err(|reason| PatternError::Catalog {
                    path: root.display().to_string(),
                    message: format!("entry `{}`: `file` {reason}", entry.id),
                })?;
                read_pattern_file(root.join(&entry.file))?
            }
        };
        if pattern.name.is_empty() {
            pattern.name = entry.name.clone();
        }
        if pattern.description.is_empty() {
            pattern.description = entry.description.clone().unwrap_or_default();
        }
        if pattern.author.is_empty() {
            pattern.author = entry.discoverer.clone().unwrap_or_default();
        }
        if pattern.rule.is_none() {
            pattern.rule = entry.rule.clone().or_else(|| self.default_rule.clone());
        }
        Ok(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[library]
name = "Test library"
rule = "B3/S23"

[[pattern]]
id = "glider"
name = "Glider"
category = "spaceships"
kind = "spaceship"
period = 4
dx = 1
dy = 1
speed = "c/4 diagonal"
year = 1970
complexity = 1
population = 5
file = "life/glider.rle"
verified = true

[[pattern]]
id = "block"
name = "Block"
category = "still-lifes"
period = "0"
displacement = [0, 0]
file = "block.cells"

[[pattern]]
id = "lwss"
name = "LWSS"
category = "spaceships"
file = "lwss.rle"
"#;

    #[test]
    fn parses_entries_and_tolerates_unknown_fields() {
        let catalog = Catalog::from_toml_str(SAMPLE, "/tmp/catalog-root").unwrap();
        assert_eq!(catalog.len(), 3);
        assert_eq!(catalog.name(), Some("Test library"));
        let glider = catalog.by_id("glider").unwrap();
        assert_eq!(glider.period, Some(4));
        assert_eq!(glider.displacement, Some((1, 1)));
        assert_eq!(glider.year, Some(1970));
        assert_eq!(glider.complexity, Some(1));
        assert_eq!(glider.extra.get("speed").map(String::as_str), Some("c/4 diagonal"));
        assert_eq!(glider.extra.get("verified").map(String::as_str), Some("true"));
        let block = catalog.by_id("block").unwrap();
        assert_eq!(block.period, Some(0), "numeric strings are accepted");
        assert_eq!(block.displacement, Some((0, 0)));
        assert_eq!(block.kind, None);
        assert_eq!(catalog.categories(), vec!["spaceships", "still-lifes"]);
        assert_eq!(catalog.in_category("spaceships").count(), 2);
        assert_eq!(catalog.pattern_path(glider), Some(PathBuf::from("/tmp/catalog-root/life/glider.rle")));
        assert!(catalog.by_id("missing").is_none());
    }

    #[test]
    fn accepts_plural_table_name() {
        let text = SAMPLE.replace("[[pattern]]", "[[patterns]]");
        assert_eq!(Catalog::from_toml_str(&text, "x").unwrap().len(), 3);
    }

    #[test]
    fn rejects_malformed_catalogs() {
        let cases: &[(&str, &str)] = &[
            ("[[pattern]]\nid = \"a\"\nname = \"A\"\n", "entry 1 (`a`): missing required field `file`"),
            ("[[pattern]]\nname = \"A\"\nfile = \"a.rle\"\n", "missing required field `id`"),
            ("[[pattern]]\nid = \"a\"\nname = \"A\"\nfile = \"../a.rle\"\n", "must not contain `..`"),
            ("[[pattern]]\nid = \"a\"\nname = \"A\"\nfile = \"/etc/a.rle\"\n", "must be relative"),
            ("[[pattern]]\nid = \"a\"\nname = \"A\"\nfile = \"a\"\nperiod = \"x\"\n", "`period` must be an integer"),
            ("[[pattern]]\nid = \"a\"\nname = \"A\"\nfile = \"a\"\nperiod = -1\n", "out of range"),
            ("[[pattern]]\nid = \"a\"\nname = \"A\"\nfile = \"a\"\ndx = 1\n", "together"),
            (
                "[[pattern]]\nid = \"a\"\nname = \"A\"\nfile = \"a\"\ndx = 1\ndy = 0\ndisplacement = [0, 1]\n",
                "disagree",
            ),
            ("[[pattern]]\nid = \"a\"\nname = \"A\"\nfile = \"a\"\ndisplacement = [1]\n", "two integers"),
            (
                "[[pattern]]\nid = \"a\"\nname = \"A\"\nfile = \"a\"\n[[pattern]]\nid = \"a\"\nname = \"B\"\nfile = \"b\"\n",
                "duplicate id `a`",
            ),
            ("[library]\nname = \"x\"\n", "no [[pattern]] entries"),
            ("pattern = 3\n", "array of tables"),
            ("[[pattern]]\nid = \"a\"\nname = \n", "TOML syntax error at line 3"),
            ("[[pattern]]\nid = \"a\"\nname = [1]\nfile = \"a\"\n", "`name` must be text"),
        ];
        for (text, message) in cases {
            let err = Catalog::from_toml_str(text, "root").expect_err(text);
            assert!(err.to_string().contains(message), "{text:?}: got `{err}`, expected `{message}`");
        }
    }

    #[test]
    fn loads_patterns_from_disk_and_fills_metadata() {
        let dir = std::env::temp_dir().join(format!("patterns-catalog-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("life")).unwrap();
        std::fs::write(dir.join("life/glider.rle"), "x = 3, y = 3\nbo$2bo$3o!\n").unwrap();
        std::fs::write(dir.join("block.cells"), "!Name: Block from file\nOO\nOO\n").unwrap();
        std::fs::write(dir.join("catalog.toml"), SAMPLE).unwrap();

        let catalog = Catalog::load(&dir).unwrap();
        assert_eq!(catalog.root(), Some(dir.as_path()));
        let glider = catalog.load_pattern(catalog.by_id("glider").unwrap()).unwrap();
        assert_eq!(glider.population(), 5);
        assert_eq!(glider.name, "Glider", "name filled from the entry");
        assert_eq!(glider.rule.as_deref(), Some("B3/S23"), "rule filled from [library]");
        let block = catalog.load_pattern(catalog.by_id("block").unwrap()).unwrap();
        assert_eq!(block.name, "Block from file", "file metadata wins");
        assert_eq!(block.population(), 4);
        let missing = catalog.load_pattern(catalog.by_id("lwss").unwrap()).unwrap_err();
        assert!(matches!(missing, PatternError::Io { .. }), "{missing}");

        let direct = Catalog::load(dir.join("catalog.toml")).unwrap();
        assert_eq!(direct.entries(), catalog.entries());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(Catalog::load(&dir), Err(PatternError::Io { .. })));
    }

    #[test]
    fn builtin_catalog_loads_every_pattern() {
        let catalog = Catalog::builtin();
        assert!(catalog.is_builtin());
        assert!(catalog.root().is_none());
        assert!(catalog.len() >= 15);
        for entry in catalog.entries() {
            let pattern = catalog.load_pattern(entry).unwrap();
            assert_eq!(pattern.population() as u64, entry.population.unwrap(), "{}", entry.id);
            assert_eq!(pattern.name, entry.name);
            assert_eq!(pattern.rule.as_deref(), Some("B3/S23"));
        }
        assert_eq!(catalog.categories(), vec!["still-lifes", "oscillators", "spaceships", "guns", "methuselahs"]);
        let unknown = CatalogEntry { id: "nope".into(), ..CatalogEntry::default() };
        assert!(catalog.load_pattern(&unknown).is_err());
    }
}
