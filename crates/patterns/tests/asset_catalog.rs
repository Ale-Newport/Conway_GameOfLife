//! Loads the workspace pattern library (`assets/patterns/catalog.toml`) with
//! the catalog loader and checks every entry against its pattern file.
//!
//! The library is data maintained outside this crate; when it is absent (for
//! example in a source package without assets) the test reports that and
//! passes without checking anything.

use std::path::PathBuf;

use patterns::{Catalog, LifeRule, Rule, builtin_patterns, rle};

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/patterns")
}

#[test]
fn workspace_catalog_matches_its_pattern_files() {
    let path = assets_dir().join("catalog.toml");
    if !path.exists() {
        eprintln!("{} not found; skipping", path.display());
        return;
    }
    let catalog = Catalog::load(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(!catalog.is_empty());
    let mut problems = Vec::new();
    for entry in catalog.entries() {
        let pattern = match catalog.load_pattern(entry) {
            Ok(pattern) => pattern,
            Err(error) => {
                problems.push(format!("{}: {error}", entry.id));
                continue;
            }
        };
        if let Some(population) = entry.population
            && pattern.population() as u64 != population
        {
            problems.push(format!(
                "{}: population {} in file, {population} in catalog",
                entry.id,
                pattern.population()
            ));
        }
        if let (Some(width), Some(height)) = (entry.width, entry.height)
            && (pattern.width(), pattern.height()) != (width, height)
        {
            problems.push(format!(
                "{}: size {}x{} in file, {width}x{height} in catalog",
                entry.id,
                pattern.width(),
                pattern.height()
            ));
        }
        match pattern.rule.as_deref().map(Rule::parse) {
            Some(Ok(_)) => {}
            Some(Err(error)) => problems.push(format!("{}: rule: {error}", entry.id)),
            None => problems.push(format!("{}: no rule in file or catalog", entry.id)),
        }
        // The canonical writer must preserve the pattern exactly.
        if rle::parse(&rle::write(&pattern)).map(|p| p.cells) != Ok(pattern.cells.clone()) {
            problems.push(format!("{}: RLE round trip changed the cells", entry.id));
        }
    }
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.join("\n"));
}

#[test]
fn builtin_library_agrees_with_workspace_catalog() {
    let path = assets_dir().join("catalog.toml");
    if !path.exists() {
        eprintln!("{} not found; skipping", path.display());
        return;
    }
    let catalog = Catalog::load(&path).unwrap_or_else(|e| panic!("{e}"));
    for builtin in builtin_patterns() {
        let Some(entry) = catalog.by_id(builtin.id) else { continue };
        let ours = builtin.pattern().unwrap().normalized();
        let theirs = catalog.load_pattern(entry).unwrap().normalized();
        assert_eq!(LifeRule::parse(theirs.rule.as_deref().unwrap()).unwrap(), LifeRule::conway(), "{}", builtin.id);
        // Same object up to rotation and reflection.
        let mut variants = Vec::new();
        let mut shape = ours.clone();
        for _ in 0..4 {
            variants.push(shape.normalized().cells);
            variants.push(shape.flip_horizontal().normalized().cells);
            shape = shape.rotate_cw();
        }
        assert!(variants.contains(&theirs.cells), "{} differs from the asset library", builtin.id);
        assert_eq!(entry.category.as_deref(), Some(builtin.category), "{}", builtin.id);
        assert_eq!(entry.kind.as_deref(), Some(builtin.kind), "{}", builtin.id);
    }
}

#[test]
fn every_rle_file_in_the_assets_parses_and_round_trips() {
    let root = assets_dir();
    if !root.is_dir() {
        eprintln!("{} not found; skipping", root.display());
        return;
    }
    let mut stack = vec![root];
    let mut files = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rle") {
                files.push(path);
            }
        }
    }
    files.sort();
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let pattern = patterns::parse_any(&text, path.to_str()).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!pattern.is_empty(), "{} has no cells", path.display());
        let back = rle::parse(&rle::write(&pattern)).unwrap();
        // RLE readers produce row-major cells, so the lists must be identical.
        assert_eq!(back.cells, pattern.cells, "{}", path.display());
    }
    eprintln!("parsed {} RLE files under assets/patterns", files.len());
}
