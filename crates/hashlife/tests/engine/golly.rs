//! Validation against Golly's own macrocell files. The files are GPL data
//! shipped with Golly and are **not** part of this repository: the tests read
//! them at runtime from the directory named by `GOLLY_PATTERNS` (Golly's
//! `Patterns` folder) and are ignored by default. Run them with
//!
//! ```text
//! GOLLY_PATTERNS=/path/to/golly/Patterns \
//!   cargo test -p hashlife --profile verify -- --ignored golly
//! ```
//!
//! `.mc.gz` files are decompressed with the system `gzip` (skipped if absent).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hashlife::{Universe, macrocell};

use crate::common::{CellSet, LIFE, cells_of, naive_run, naive_step};

fn golly_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("GOLLY_PATTERNS")?);
    dir.is_dir().then_some(dir)
}

fn collect_macrocells(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let name = path.to_string_lossy().to_string();
        if path.is_dir() {
            collect_macrocells(&path, out);
        } else if name.ends_with(".mc") || name.ends_with(".mc.gz") {
            out.push(path);
        }
    }
}

fn load(path: &Path) -> Option<String> {
    if path.to_string_lossy().ends_with(".gz") {
        let output = Command::new("gzip").arg("-dc").arg(path).output().ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        fs::read_to_string(path).ok()
    }
}

fn load_universe(relative: &str) -> Option<Universe> {
    let Some(dir) = golly_dir() else {
        eprintln!("GOLLY_PATTERNS is not set or not a directory; skipping");
        return None;
    };
    let text = load(&dir.join(relative))?;
    Some(macrocell::read(&text).unwrap_or_else(|e| panic!("{relative}: {e}")))
}

/// Strips the header lines so files at different generations can be compared.
fn body(text: &str) -> String {
    text.lines().filter(|l| !l.starts_with('[') && !l.starts_with('#')).collect::<Vec<_>>().join("\n")
}

#[test]
#[ignore = "needs GOLLY_PATTERNS pointing at Golly's Patterns directory"]
fn golly_files_parse_round_trip_and_evolve_like_the_naive_stepper() {
    let Some(dir) = golly_dir() else {
        eprintln!("GOLLY_PATTERNS is not set or not a directory; skipping");
        return;
    };
    let mut files = Vec::new();
    collect_macrocells(&dir, &mut files);
    assert!(!files.is_empty(), "no .mc files under {}", dir.display());
    let (mut accepted, mut rejected, mut compared) = (0, 0, 0);
    for path in &files {
        let name = path.strip_prefix(&dir).unwrap_or(path).display().to_string();
        let Some(text) = load(path) else {
            eprintln!("skipped (unreadable or no gzip): {name}");
            continue;
        };
        let rule_line = text.lines().find(|l| l.starts_with("#R")).map(|l| l[2..].trim().to_string());
        let universe = match macrocell::read(&text) {
            Ok(universe) => universe,
            Err(error) => {
                // Only rules Hashlife here cannot run may be rejected.
                let rule = rule_line.unwrap_or_default();
                assert!(
                    !rule.is_empty() && hashlife::Rule::parse(&rule).is_err(),
                    "{name}: rejected although its rule `{rule}` is supported: {error}"
                );
                eprintln!("rejected as expected: {name}: {error}");
                rejected += 1;
                continue;
            }
        };
        accepted += 1;
        let written = macrocell::write(&universe);
        let back = macrocell::read(&written).unwrap_or_else(|e| panic!("{name}: re-reading our output failed: {e}"));
        assert_eq!(back.population_exact(), universe.population_exact(), "{name}");
        assert_eq!(macrocell::write(&back), written, "{name}: round trip is not stable");
        let small = universe.population() <= 20_000
            && universe.bounding_box().is_some_and(|(x0, y0, x1, y1)| x1 - x0 < 4096 && y1 - y0 < 4096);
        if small {
            let start: CellSet = cells_of(&universe);
            let rule = universe.rule();
            let mut evolved = universe.clone();
            evolved.advance(100).unwrap();
            assert_eq!(cells_of(&evolved), naive_run(&start, rule.birth(), rule.survive(), 100), "{name}");
            compared += 1;
        }
        eprintln!("ok: {name} (population {}, root level {})", universe.population(), universe.root_level());
    }
    eprintln!("{accepted} accepted, {rejected} rejected, {compared} compared with the naive stepper");
    assert!(accepted >= 20, "expected most Golly HashLife files to load");
}

#[test]
#[ignore = "needs GOLLY_PATTERNS pointing at Golly's Patterns directory"]
fn golly_gotts_dots_matches_its_documented_populations() {
    // From the file's own comments: 41 cells; at t = 215643 the population
    // is 316387 and at t = 3662092278363 it is 5742718768151.
    let Some(mut universe) = load_universe("HashLife/Eventful/gotts-dots.mc") else { return };
    assert_eq!(universe.population(), 41);
    universe.advance(215_643).unwrap();
    assert_eq!(universe.population(), 316_387);
    universe.advance(3_662_092_278_363 - 215_643).unwrap();
    assert_eq!(universe.population(), 5_742_718_768_151);
}

#[test]
#[ignore = "needs GOLLY_PATTERNS pointing at Golly's Patterns directory"]
fn golly_metapixel_galaxy_meta_grid_follows_life() {
    // OTCA metapixels: 2048×2048-cell tiles with period 35328. A tile showing
    // an ON meta-cell holds ~64k live cells, an OFF one ~22k. At meta-phase 0
    // a tile displays the meta-state computed one period earlier, so the
    // decoded meta-grid repeats once (the file starts at the display of
    // meta-generation 0) and then follows B3/S23.
    const PERIOD: u64 = 35_328;
    let Some(mut universe) = load_universe("HashLife/Metacell/metapixel-galaxy.mc.gz") else { return };
    let (x0, y0, _, _) = universe.bounding_box().expect("non-empty");
    // Tiles are aligned on Golly's quadtree: x ≡ 0 and y ≡ 1 (mod 2048).
    let (gx, gy) = (x0.div_euclid(2048) * 2048, (y0 - 1).div_euclid(2048) * 2048 + 1);
    let decode = |u: &Universe| -> CellSet {
        let tiles = u.density_grid(gx, gy, 16, 16, 11).expect("small grid");
        assert!(tiles.iter().all(|&p| p == 0 || !(30_000..60_000).contains(&p)), "ambiguous tile: {tiles:?}");
        tiles
            .iter()
            .enumerate()
            .filter(|&(_, &p)| p >= 60_000)
            .map(|(i, _)| ((i % 16) as i64, (i / 16) as i64))
            .collect()
    };
    let mut states = Vec::new();
    let mut bodies = Vec::new();
    for _ in 0..=17 {
        states.push(decode(&universe));
        bodies.push(body(&macrocell::write(&universe)));
        universe.advance(PERIOD).unwrap();
    }
    assert_eq!(states[0].len(), 48, "Kok's galaxy: four 2×6 bars");
    assert_eq!(states[1], states[0], "display delay of one metapixel period");
    for k in 1..17 {
        assert_eq!(states[k + 1], naive_step(&states[k], LIFE.0, LIFE.1), "meta-generation {k} -> {}", k + 1);
    }
    assert_eq!(states[9], states[1], "the galaxy has period 8");
    // Cell level: exactly periodic with period 16 × 35328 after a one-period
    // transient (the LWSS fill inside long-lived ON tiles needs two galaxy
    // periods to return to the same phase).
    assert_eq!(bodies[17], bodies[1]);
    assert_ne!(bodies[9], bodies[1]);
}
