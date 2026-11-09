//! Reads real-world pattern files from a local Golly source checkout.
//!
//! Golly's pattern collection is GPL-licensed, so no file is copied into this
//! crate: the test reads them at run time and is ignored by default. Run it
//! with
//!
//! ```text
//! GOLLY_DIR=/path/to/golly cargo test -p patterns --test golly_files -- --ignored
//! ```
//!
//! When `GOLLY_DIR` is unset or does not contain a `Patterns` folder the
//! tests report that and pass without checking anything.

use std::path::{Path, PathBuf};

use patterns::{BoundedGrid, Format, GridKind, LifeRule, Pattern, Rule, detect_format, parse_any, rle};

fn golly_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("GOLLY_DIR")?);
    dir.join("Patterns").is_dir().then_some(dir)
}

/// Pattern files below `dir` with one of `extensions`, sorted for a stable order.
fn files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| extensions.contains(&e)) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn read(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
#[ignore = "needs a local Golly checkout (set GOLLY_DIR)"]
fn parses_and_round_trips_every_golly_pattern() {
    let Some(golly) = golly_dir() else {
        eprintln!("Golly checkout not found; set GOLLY_DIR to run this test");
        return;
    };
    let paths = files(&golly.join("Patterns"), &["rle", "lif", "cells"]);
    assert!(paths.len() > 400, "expected the full Golly collection, found {}", paths.len());
    let mut failures = Vec::new();
    let mut total_cells = 0usize;
    for path in &paths {
        let text = read(path);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let pattern = match parse_any(&text, Some(&name)) {
            Ok(pattern) => pattern,
            Err(error) => {
                failures.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        total_cells += pattern.population();
        // Writing RLE and reading it back preserves every cell and state.
        let back = rle::parse(&rle::write(&pattern)).unwrap();
        if back.normalized().cells != pattern.normalized().cells {
            failures.push(format!("{}: RLE round trip changed the cells", path.display()));
        }
        if back.bounding_box() != pattern.bounding_box() {
            failures.push(format!("{}: RLE round trip moved the pattern", path.display()));
        }
    }
    eprintln!("parsed {} Golly files with {total_cells} cells in total", paths.len());
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
#[ignore = "needs a local Golly checkout (set GOLLY_DIR)"]
fn life_folder_rules_and_known_patterns() {
    let Some(golly) = golly_dir() else {
        eprintln!("Golly checkout not found; set GOLLY_DIR to run this test");
        return;
    };
    let life = golly.join("Patterns/Life");
    let mut rle_files = 0;
    for path in files(&life, &["rle"]) {
        let text = read(&path);
        assert_eq!(detect_format(&text, path.to_str()).unwrap(), Format::Rle, "{}", path.display());
        let document = rle::parse_document(&text).unwrap();
        rle_files += 1;
        let rule = document.pattern.rule.clone().unwrap_or_else(|| "B3/S23".into());
        let (body, suffix) = rule.split_once(':').map_or((rule.as_str(), None), |(b, s)| (b, Some(s)));
        if body != "LifeHistory" {
            let (parsed, grid) = Rule::parse_with_grid(&rule).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(parsed, Rule::Life(LifeRule::conway()), "{}", path.display());
            assert_eq!(grid.is_some(), suffix.is_some_and(|s| s != "T0,0"), "{}", path.display());
        }
        if let Some(suffix) = suffix {
            BoundedGrid::parse(suffix).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
        assert!(!document.pattern.is_empty(), "{} has no cells", path.display());
    }
    assert!(rle_files >= 150, "found only {rle_files} RLE files");

    let acorn = parse_any(&read(&life.join("Methuselahs/acorn.lif")), Some("acorn.lif")).unwrap();
    assert_eq!(acorn.population(), 7);
    assert_eq!(acorn.comments.first().map(String::as_str), Some("Acorn"));
    let rabbits = parse_any(&read(&life.join("Methuselahs/rabbits.lif")), Some("rabbits.lif")).unwrap();
    assert_eq!(rabbits.population(), 9);
    assert_eq!((rabbits.width(), rabbits.height()), (7, 3));
    let breeder = parse_any(&read(&life.join("Breeders/breeder.lif")), Some("breeder.lif")).unwrap();
    assert_eq!(breeder.rule.as_deref(), Some("B3/S23"), "#N means normal Life rules");
    assert!(breeder.population() > 1000);

    let klein = rle::parse_document(&read(&life.join("Bounded-Grids/Klein-bottle.rle"))).unwrap();
    assert_eq!(klein.position, Some((-10, -3)));
    let (_, grid) =
        Rule::parse_with_grid(klein.pattern.rule.as_deref().unwrap().replace("LifeHistory", "B3/S23").as_str())
            .unwrap();
    let grid = grid.unwrap();
    assert_eq!((grid.kind, grid.width, grid.height, grid.horizontal_twist), (GridKind::KleinBottle, 40, 20, true));
    assert!(klein.pattern.comments.iter().any(|c| c.contains("Klein bottle")));

    let torus = rle::parse(&read(&life.join("Bounded-Grids/torus-with-shift.rle"))).unwrap();
    let suffix = torus.rule.as_deref().unwrap().split_once(':').unwrap().1;
    assert_eq!(BoundedGrid::parse(suffix).unwrap().unwrap().to_string(), ":T30+7,20");
    assert!(torus.cells.iter().all(|c| c.state >= 1));
}

/// Golly places a pattern read into a bounded grid without `#CXRLE Pos` in
/// the grid (centred, or at its top-left corner when it is too large) and
/// drops the cells outside it. With the reader's placement every cell of a
/// pattern whose header size fits must therefore lie inside the grid.
#[test]
#[ignore = "needs a local Golly checkout (set GOLLY_DIR)"]
fn bounded_grid_patterns_are_placed_inside_their_grid() {
    let Some(golly) = golly_dir() else {
        eprintln!("Golly checkout not found; set GOLLY_DIR to run this test");
        return;
    };
    let mut checked = 0;
    for path in files(&golly.join("Patterns"), &["rle"]) {
        let document = rle::parse_document(&read(&path)).unwrap();
        let Some(grid) =
            document.pattern.rule.as_deref().and_then(|rule| rule.split_once(':')).and_then(|(_, suffix)| {
                BoundedGrid::parse(suffix).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            })
        else {
            continue;
        };
        let (width, height) = document.declared_size.unwrap();
        let fits = |size: u64, limit: u32| limit == 0 || size <= u64::from(limit);
        if !(fits(width, grid.width) && fits(height, grid.height)) {
            continue;
        }
        let outside = document.pattern.cells.iter().filter(|c| !grid.contains(c.x, c.y)).count();
        assert_eq!(outside, 0, "{}: {outside} cells outside {grid}", path.display());
        checked += 1;
    }
    eprintln!("checked the placement of {checked} bounded-grid patterns");
    assert!(checked >= 10, "expected Golly's bounded-grid examples, found {checked}");
}

#[test]
#[ignore = "needs a local Golly checkout (set GOLLY_DIR)"]
fn generations_folder_rules_parse() {
    let Some(golly) = golly_dir() else {
        eprintln!("Golly checkout not found; set GOLLY_DIR to run this test");
        return;
    };
    let mut checked = 0;
    for path in files(&golly.join("Patterns/Generations"), &["rle"]) {
        let pattern: Pattern = rle::parse(&read(&path)).unwrap();
        let Some(rule) = pattern.rule.as_deref() else { continue };
        match Rule::parse(rule) {
            Ok(parsed) => {
                assert!(pattern.max_state() < parsed.states(), "{}: state above the rule's range", path.display());
                checked += 1;
            }
            // Only notations this crate does not support (isotropic letters,
            // H/V neighbourhoods) may be rejected.
            Err(error) => assert!(rule.chars().any(|c| c.is_ascii_alphabetic()), "{}: {rule}: {error}", path.display()),
        }
    }
    eprintln!("checked {checked} Generations rules");
    assert!(checked > 0);
}
