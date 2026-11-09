//! Scenario files: RON parsing, folder loading and the built-in presets.
//!
//! A scenario file is a RON [`Scenario`] with a header (`id`, `name`,
//! `description`, `seed`), a `world`, its `populations` and optional rule
//! `params`. The six presets in `scenarios/civilization/` are embedded at
//! compile time, so the application works without the folder.

use std::path::Path;

use ron::extensions::Extensions;

use crate::config::Scenario;
use crate::error::CivError;

/// Largest scenario file accepted, in bytes.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Embedded copies of `scenarios/civilization/*.ron` as `(file name, text)`.
pub const BUILTIN_FILES: [(&str, &str); 6] = [
    ("abundant.ron", include_str!("../../../scenarios/civilization/abundant.ron")),
    ("scarcity.ron", include_str!("../../../scenarios/civilization/scarcity.ron")),
    ("cooperation_vs_competition.ron", include_str!("../../../scenarios/civilization/cooperation_vs_competition.ron")),
    ("two_strategies.ron", include_str!("../../../scenarios/civilization/two_strategies.ron")),
    ("territorial_expansion.ron", include_str!("../../../scenarios/civilization/territorial_expansion.ron")),
    ("trade_routes.ron", include_str!("../../../scenarios/civilization/trade_routes.ron")),
];

/// Parses and validates scenario text; `origin` names it in error messages.
pub fn parse(text: &str, origin: &str) -> Result<Scenario, CivError> {
    let options = ron::Options::default()
        .with_default_extension(Extensions::IMPLICIT_SOME)
        .with_default_extension(Extensions::UNWRAP_VARIANT_NEWTYPES);
    let scenario: Scenario =
        options.from_str(text).map_err(|e| CivError::Parse { origin: origin.to_string(), message: e.to_string() })?;
    scenario.validate().map_err(|e| CivError::InvalidScenario(format!("{origin}: {e}")))?;
    Ok(scenario)
}

/// The six built-in presets, in a fixed order.
pub fn builtin() -> Result<Vec<Scenario>, CivError> {
    BUILTIN_FILES.iter().map(|(name, text)| parse(text, &format!("builtin:{name}"))).collect()
}

/// The built-in preset with the given id.
pub fn builtin_by_id(id: &str) -> Result<Scenario, CivError> {
    builtin()?.into_iter().find(|s| s.id == id).ok_or_else(|| {
        let ids: Vec<&str> = BUILTIN_FILES.iter().map(|(n, _)| n.trim_end_matches(".ron")).collect();
        CivError::InvalidScenario(format!("no built-in scenario `{id}` (available: {})", ids.join(", ")))
    })
}

/// Reads one scenario file.
pub fn load_file(path: &Path) -> Result<Scenario, CivError> {
    let io = |e: std::io::Error| CivError::Io { path: path.display().to_string(), message: e.to_string() };
    let size = std::fs::metadata(path).map_err(io)?.len();
    if size > MAX_FILE_BYTES {
        return Err(CivError::Io {
            path: path.display().to_string(),
            message: format!("file is {size} bytes; the limit is {MAX_FILE_BYTES}"),
        });
    }
    let text = std::fs::read_to_string(path).map_err(io)?;
    parse(&text, &path.display().to_string())
}

/// Reads every `*.ron` file of a folder, sorted by file name. The first
/// invalid file aborts loading with an error naming it.
pub fn load_dir(dir: &Path) -> Result<Vec<Scenario>, CivError> {
    let io = |e: std::io::Error| CivError::Io { path: dir.display().to_string(), message: e.to_string() };
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(io)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "ron"))
        .collect();
    files.sort();
    files.iter().map(|p| load_file(p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse_and_have_unique_ids() {
        let all = builtin().unwrap();
        assert_eq!(all.len(), 6);
        let mut ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 6);
        assert!(builtin_by_id("scarcity").is_ok());
        assert!(builtin_by_id("nope").unwrap_err().to_string().contains("abundant"));
    }

    #[test]
    fn parse_errors_carry_position_and_origin() {
        let err = parse("(id: \"x\", name: 3)", "bad.ron").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("bad.ron") && text.contains("1:"), "{text}");
    }

    #[test]
    fn folder_matches_embedded_copies() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/civilization");
        let loaded = load_dir(&dir).unwrap();
        let mut embedded = builtin().unwrap();
        embedded.sort_by(|a, b| a.id.cmp(&b.id));
        let mut loaded_sorted = loaded.clone();
        loaded_sorted.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(loaded_sorted, embedded);
        assert!(load_dir(&dir.join("missing")).is_err());
    }
}
