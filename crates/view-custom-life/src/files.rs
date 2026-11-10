//! Saving, loading, exporting and importing life systems.
//!
//! The library lives in `<data>/scenarios/custom-life/` (one
//! [`LifeSystem`] RON document per file, written atomically); exports go to
//! `<data>/exports/` as RON or JSON. Reading accepts a plain `LifeSystem`
//! document (RON or JSON) and the preset format of
//! `scenarios/custom-life/*.ron` (a header with `id`, `seed`... around the
//! system), so presets can be imported as they are. Reading never compiles
//! the system: a work-in-progress draft with errors loads and shows its
//! diagnostics in the editor.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use persistence::{UserDataDirs, atomic_write, read_limited};
use rule_engine::def::LifeSystem;
use rule_engine::scenarios::{MAX_SCENARIO_BYTES, Scenario};

/// Library sub-folder of `<data>/scenarios/`.
pub const LIBRARY_FOLDER: &str = "custom-life";

/// Document encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// RON (the library format).
    Ron,
    /// JSON.
    Json,
}

impl Format {
    /// File extension.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Ron => "ron",
            Format::Json => "json",
        }
    }

    /// Format implied by a file name (`.json` → JSON, anything else RON).
    pub fn of_path(path: &Path) -> Self {
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) { Format::Json } else { Format::Ron }
    }
}

/// A system read from a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    /// The system.
    pub system: LifeSystem,
    /// The seed, when the document carries one (preset format).
    pub seed: Option<u64>,
    /// How it was encoded.
    pub format: Format,
}

/// One file of the library.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    /// File stem.
    pub name: String,
    /// Full path.
    pub path: PathBuf,
    /// Size in bytes.
    pub bytes: u64,
    /// Last modification.
    pub modified: Option<SystemTime>,
}

/// The library folder.
pub fn library_dir(dirs: &UserDataDirs) -> PathBuf {
    dirs.scenarios().join(LIBRARY_FOLDER)
}

/// A file-system friendly version of a user-chosen name: letters, digits,
/// `-` and `_` are kept, spaces and other characters become `-`, runs of
/// `-` collapse, at most 64 characters; empty names become `untitled`.
pub fn sanitize_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        let keep = c.is_ascii_alphanumeric() || c == '_' || c == '-';
        let c = if keep { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
        if out.len() >= 64 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "untitled".to_string() } else { out }
}

/// Encodes a system.
pub fn encode(system: &LifeSystem, format: Format) -> Result<String, String> {
    match format {
        Format::Ron => system.to_ron(),
        Format::Json => system.to_json(),
    }
    .map_err(|e| e.to_string())
}

/// Decodes a document: a `LifeSystem` in the preferred format, then the
/// other format, then the preset (scenario) RON format.
pub fn decode(text: &str, preferred: Format) -> Result<Loaded, String> {
    let ron = |text: &str| LifeSystem::from_ron(text).map(|s| (s, Format::Ron));
    let json = |text: &str| LifeSystem::from_json(text).map(|s| (s, Format::Json));
    let (first, second) = match preferred {
        Format::Ron => (ron(text), json(text)),
        Format::Json => (json(text), ron(text)),
    };
    let first_error = match first {
        Ok((system, format)) => return Ok(Loaded { system, seed: None, format }),
        Err(e) => e.to_string(),
    };
    if let Ok((system, format)) = second {
        return Ok(Loaded { system, seed: None, format });
    }
    match ron::from_str::<Scenario>(text) {
        Ok(scenario) => Ok(Loaded { system: scenario.system, seed: Some(scenario.seed), format: Format::Ron }),
        Err(_) => Err(first_error),
    }
}

/// Reads a document from disk (at most [`MAX_SCENARIO_BYTES`]).
pub fn read_file(path: &Path) -> Result<Loaded, String> {
    let bytes = read_limited(path, MAX_SCENARIO_BYTES).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| format!("`{}` is not UTF-8 text", path.display()))?;
    decode(&text, Format::of_path(path))
}

/// Writes `system` to `<dir>/<sanitised name>.ron`. Refuses to replace an
/// existing file unless `overwrite` is set.
pub fn save_to_library(dir: &Path, name: &str, system: &LifeSystem, overwrite: bool) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create `{}`: {e}", dir.display()))?;
    let path = dir.join(format!("{}.ron", sanitize_name(name)));
    if path.exists() && !overwrite {
        return Err(format!("`{}` exists; tick “Overwrite” to replace it", path.display()));
    }
    let text = encode(system, Format::Ron)?;
    atomic_write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Writes an export copy `custom-life-<name>-<stamp>.<ext>` into `dir`.
pub fn export(dir: &Path, system: &LifeSystem, format: Format, stamp: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create `{}`: {e}", dir.display()))?;
    let path = dir.join(format!("custom-life-{}-{stamp}.{}", sanitize_name(&system.name), format.extension()));
    let text = encode(system, format)?;
    atomic_write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Files written by [`export`] (`custom-life-*.ron` / `*.json`), newest
/// first, offered for import without typing a path.
pub fn list_exports(dir: &Path) -> Result<Vec<LibraryEntry>, String> {
    list_files(dir, |path| {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        name.starts_with("custom-life-") && (ext == "ron" || ext == "json")
    })
}

/// Library files (`*.ron`), newest first. A missing folder is an empty library.
pub fn list_library(dir: &Path) -> Result<Vec<LibraryEntry>, String> {
    list_files(dir, |path| path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ron")))
}

fn list_files(dir: &Path, keep: impl Fn(&Path) -> bool) -> Result<Vec<LibraryEntry>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read `{}`: {e}", dir.display())),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !keep(&path) {
            continue;
        }
        let meta = entry.metadata().ok();
        out.push(LibraryEntry {
            name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            bytes: meta.as_ref().map_or(0, std::fs::Metadata::len),
            modified: meta.and_then(|m| m.modified().ok()),
            path,
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::scenarios;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("view-custom-life-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn names_are_file_safe() {
        assert_eq!(sanitize_name("  My first ecosystem! "), "My-first-ecosystem");
        assert_eq!(sanitize_name("a/b\\c..d"), "a-b-c-d");
        assert_eq!(sanitize_name("***"), "untitled");
        assert_eq!(sanitize_name(&"x".repeat(200)).len(), 64);
        assert_eq!(sanitize_name("keep_this-one"), "keep_this-one");
    }

    #[test]
    fn documents_round_trip_in_both_formats_and_presets_import() {
        let preset = scenarios::builtin_by_id("robot-swarm").unwrap();
        for format in [Format::Ron, Format::Json] {
            let text = encode(&preset.system, format).unwrap();
            let loaded = decode(&text, format).unwrap();
            assert_eq!(loaded.system, preset.system);
            assert_eq!(loaded.format, format);
            // The other preference still finds it.
            let other = if format == Format::Ron { Format::Json } else { Format::Ron };
            assert_eq!(decode(&text, other).unwrap().system, preset.system);
        }
        let file = include_str!("../../../scenarios/custom-life/bacterial-growth.ron");
        let loaded = decode(file, Format::Ron).unwrap();
        assert_eq!(loaded.seed, Some(42));
        assert_eq!(loaded.system.entity_types[0].name, "bacterium");
        assert!(decode("(not a system)", Format::Ron).is_err());
    }

    #[test]
    fn library_save_list_load_and_export() {
        let dir = scratch("library");
        let sys = scenarios::builtin_by_id("bacterial-growth").unwrap().system;
        assert!(list_library(&dir).unwrap().is_empty(), "a missing folder is an empty library");
        let path = save_to_library(&dir, "my colony", &sys, false).unwrap();
        assert!(path.ends_with("my-colony.ron"));
        assert!(save_to_library(&dir, "my colony", &sys, false).is_err(), "no silent overwrite");
        save_to_library(&dir, "my colony", &sys, true).unwrap();
        let listed = list_library(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "my-colony");
        assert_eq!(read_file(&listed[0].path).unwrap().system, sys);
        let exported = export(&dir.join("exports"), &sys, Format::Json, "20261110-120000").unwrap();
        assert!(exported.to_string_lossy().ends_with("custom-life-Bacterial-growth-20261110-120000.json"));
        let back = read_file(&exported).unwrap();
        assert_eq!((back.system, back.format), (sys, Format::Json));
        std::fs::write(dir.join("exports").join("notes.txt"), "x").unwrap();
        let exports = list_exports(&dir.join("exports")).unwrap();
        assert_eq!(exports.len(), 1, "only exported systems are offered for import");
        assert_eq!(exports[0].path, exported);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
