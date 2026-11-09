//! Regenerates `scenarios/continuous/smoothlife-glider.ron` with this
//! crate's own engine, following the procedure of
//! `docs/research/continuous-life.md` §5.4:
//!
//! 1. Run Rafler's paper rule (r_a = 21, discrete stepping) on a 256² torus
//!    from the `smoothlife.soup` disc soup for 150 steps.
//! 2. Split the field into connected components (cells above 0.05,
//!    4-neighbourhood, periodic).
//! 3. Copy each component alone onto an empty torus and run it for 400
//!    steps; keep the first one whose mass stays within ±2 % over the last
//!    200 steps while its centroid moves at a steady speed above 2 cells
//!    per step (a glider).
//! 4. Crop the candidate (8-bit Chan RLE) and check that the quantised
//!    copy still glides, then write the scenario file.
//!
//! Usage: `cargo run -p continuous --release --example smoothlife_glider -- <output.ron> [first_seed] [seeds]`

use std::collections::VecDeque;
use std::process::ExitCode;

use continuous::scenario::{self, InitElement, Scenario, ScenarioHeader};
use continuous::{CellData, ContinuousSim, rle};
use simulation_core::{Grid2, Simulation};

const SIDE: usize = 256;
const THRESHOLD: f32 = 0.05;

struct Candidate {
    cells: Grid2<f32>,
    mass: f64,
    speed: f64,
}

fn components(field: &Grid2<f32>) -> Vec<Vec<(usize, usize)>> {
    let (w, h) = (field.width(), field.height());
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    for start in 0..w * h {
        if seen[start] || field.as_slice()[start] <= THRESHOLD {
            continue;
        }
        seen[start] = true;
        let mut queue = VecDeque::from([start]);
        let mut cells = Vec::new();
        while let Some(i) = queue.pop_front() {
            let (x, y) = (i % w, i / w);
            cells.push((x, y));
            for (nx, ny) in [((x + 1) % w, y), ((x + w - 1) % w, y), (x, (y + 1) % h), (x, (y + h - 1) % h)] {
                let j = ny * w + nx;
                if !seen[j] && field.as_slice()[j] > THRESHOLD {
                    seen[j] = true;
                    queue.push_back(j);
                }
            }
        }
        out.push(cells);
    }
    out
}

/// Copies a component into a small grid, unwrapping it around the torus.
/// Cells within 3 cells of the component (its soft rim) are kept; every
/// other cell of the bounding box is zeroed so neighbouring matter is not
/// copied along.
fn crop(field: &Grid2<f32>, cells: &[(usize, usize)]) -> Grid2<f32> {
    let (w, h) = (field.width() as i64, field.height() as i64);
    let (x0, y0) = (cells[0].0 as i64, cells[0].1 as i64);
    let rel: Vec<(i64, i64)> = cells
        .iter()
        .map(|&(x, y)| {
            let dx = (x as i64 - x0 + w / 2).rem_euclid(w) - w / 2;
            let dy = (y as i64 - y0 + h / 2).rem_euclid(h) - h / 2;
            (dx, dy)
        })
        .collect();
    let margin = 3;
    let min_x = rel.iter().map(|p| p.0).min().unwrap_or(0) - margin;
    let max_x = rel.iter().map(|p| p.0).max().unwrap_or(0) + margin;
    let min_y = rel.iter().map(|p| p.1).min().unwrap_or(0) - margin;
    let max_y = rel.iter().map(|p| p.1).max().unwrap_or(0) + margin;
    let (cw, ch) = ((max_x - min_x + 1) as usize, (max_y - min_y + 1) as usize);
    let mut keep = Grid2::new(cw, ch, false);
    for &(dx, dy) in &rel {
        for oy in -margin..=margin {
            for ox in -margin..=margin {
                keep.set((dx - min_x + ox) as usize, (dy - min_y + oy) as usize, true);
            }
        }
    }
    Grid2::from_fn(cw, ch, |cx, cy| {
        if !*keep.get(cx, cy) {
            return 0.0;
        }
        let x = (x0 + min_x + cx as i64).rem_euclid(w) as usize;
        let y = (y0 + min_y + cy as i64).rem_euclid(h) as usize;
        *field.get(x, y)
    })
}

fn scenario_with(cells: CellData, seed: u64) -> Scenario {
    let mut base = match scenario::builtin_by_id("smoothlife.soup") {
        Some(s) => s,
        None => unreachable!("smoothlife.soup is a built-in scenario"),
    };
    base.header = ScenarioHeader {
        id: "smoothlife.glider".into(),
        name: "SmoothLife smooth glider".into(),
        description: String::new(),
        seed,
    };
    base.init =
        vec![InitElement::Creature { name: "Smooth glider".into(), cells, x: 0.5, y: 0.5, rotation: 0.0, scale: 1.0 }];
    base
}

/// Result of running a candidate alone.
struct Trial {
    /// (max − min)/mean of the mass over the second half.
    spread: f64,
    /// Mean mass over the second half.
    mass: f64,
    /// Mean centroid speed over the second half (cells/step).
    speed: f64,
    /// Final field.
    field: Grid2<f32>,
}

/// Runs `cells` alone on an empty 256² torus for `steps`.
fn trial(cells: &Grid2<f32>, steps: u64) -> Option<Trial> {
    let data = CellData::Rows(vec![(0..cells.height()).map(|y| cells.row(y).to_vec()).collect()]);
    let mut sim = ContinuousSim::from_scenario(scenario_with(data, 1)).ok()?;
    sim.advance(steps / 2);
    let start = sim.centroid_track();
    let (mut lo, mut hi, mut sum) = (f64::MAX, f64::MIN, 0.0);
    for _ in 0..steps / 2 {
        sim.step();
        let m = sim.mass();
        lo = lo.min(m);
        hi = hi.max(m);
        sum += m;
    }
    let end = sim.centroid_track();
    let speed = (end.0 - start.0).hypot(end.1 - start.1) / (steps / 2) as f64;
    let mean = sum / (steps / 2) as f64;
    Some(Trial { spread: (hi - lo) / mean, mass: mean, speed, field: sim.fields()[0].clone() })
}

fn search(seed: u64) -> Option<Candidate> {
    let mut sim = ContinuousSim::with_seed(scenario::builtin_by_id("smoothlife.soup")?, seed).ok()?;
    sim.advance(150);
    let field = sim.fields()[0].clone();
    for comp in components(&field) {
        if comp.len() < 200 {
            continue;
        }
        let cells = crop(&field, &comp);
        if cells.width() > SIDE / 2 || cells.height() > SIDE / 2 {
            continue;
        }
        let Some(t) = trial(&cells, 400) else { continue };
        if t.spread < 0.02 && t.speed > 2.0 {
            // Re-crop the glider after it has settled alone.
            let settled = components(&t.field).into_iter().max_by_key(Vec::len)?;
            return Some(Candidate { cells: crop(&t.field, &settled), mass: t.mass, speed: t.speed });
        }
    }
    None
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(out) = args.get(1) else {
        eprintln!("usage: smoothlife_glider <output.ron> [first_seed] [seeds]");
        return ExitCode::FAILURE;
    };
    let first: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let count: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
    for seed in first..first + count {
        let Some(c) = search(seed) else {
            eprintln!("seed {seed}: no glider");
            continue;
        };
        let text = rle::encode(&c.cells);
        let quantised = match rle::decode(&text) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("seed {seed}: {e}");
                continue;
            }
        };
        let Some(Trial { spread, mass, speed, .. }) = trial(&quantised, 1000) else { continue };
        eprintln!(
            "seed {seed}: candidate {}×{} mass {:.1} speed {:.3}; quantised: spread {spread:.4} mass {mass:.1} speed {speed:.3}",
            c.cells.width(),
            c.cells.height(),
            c.mass,
            c.speed
        );
        if spread >= 0.02 || speed <= 2.0 {
            continue;
        }
        let mut s = scenario_with(CellData::Rle(vec![text]), 1);
        s.header.description = format!(
            "Rafler's smooth glider (r_a = 21, r_i = 7, b1 = 0.278, b2 = 0.365, d1 = 0.267, d2 = 0.445, α_n = 0.028, \
             α_m = 0.147, discrete stepping), extracted with this engine from a disc soup (seed {seed}). It keeps a \
             mass of about {mass:.0} and moves about {speed:.2} cells per step."
        );
        s.notes = vec![
            "Parameters from S. Rafler, arXiv:1111.1567, Fig. 1 caption (facts, no code used).".into(),
            "Cells generated by crates/continuous/examples/smoothlife_glider.rs (8-bit Chan RLE).".into(),
        ];
        let body = match scenario::to_ron(&s) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let file = format!(
            "// Emergence Lab scenario (continuous). Generated by `cargo run -p continuous --release --example smoothlife_glider`.\n{body}\n"
        );
        if let Err(e) = std::fs::write(out, file) {
            eprintln!("cannot write {out}: {e}");
            return ExitCode::FAILURE;
        }
        eprintln!("wrote {out}");
        return ExitCode::SUCCESS;
    }
    eprintln!("no glider found");
    ExitCode::FAILURE
}
