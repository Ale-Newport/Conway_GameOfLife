//! Long verification runs behind the "How it was verified" table of
//! `docs/models/continuous.md`. They print the measured numbers and check
//! them against the published references (research note §4, §5.4, §8).
//!
//! ```text
//! cargo test -p continuous --release --test verification -- --ignored --nocapture
//! ```

use continuous::ContinuousSim;
use continuous::scenario::builtin_by_id;
use simulation_core::Simulation;

fn sim(id: &str) -> ContinuousSim {
    let scenario = builtin_by_id(id).unwrap_or_else(|| panic!("missing scenario {id}"));
    ContinuousSim::from_scenario(scenario).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Mass band, mean speed (net displacement per tick) and heading (degrees
/// from +y towards +x) over the ticks `[from, to)`.
struct Window {
    min_mass: f64,
    max_mass: f64,
    speed: f64,
    heading: f64,
}

fn window(s: &mut ContinuousSim, from: u64, to: u64) -> Window {
    s.advance(from - s.tick());
    let start = s.centroid_track();
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    while s.tick() < to {
        s.step();
        lo = lo.min(s.mass());
        hi = hi.max(s.mass());
    }
    let end = s.centroid_track();
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    Window { min_mass: lo, max_mass: hi, speed: dx.hypot(dy) / (to - from) as f64, heading: dx.atan2(dy).to_degrees() }
}

fn report(name: &str, from: u64, to: u64, w: &Window) {
    eprintln!(
        "{name}, ticks {from}–{to}: mass [{:.2}, {:.2}], speed {:.4} cells/tick, heading {:.1}° from +y",
        w.min_mass, w.max_mass, w.speed, w.heading
    );
}

#[test]
#[ignore = "long run; see docs/models/continuous.md"]
fn orbium_lives_5000_steps() {
    // Research note §8: mass ∈ [70, 76] for steps 100–5000, speed 0.602 ± 0.01.
    let mut s = sim("lenia.orbium");
    let origin = s.centroid_track();
    let early = window(&mut s, 100, 500);
    let steady = window(&mut s, 500, 1000);
    let at_1000 = s.centroid_track();
    let (dx, dy) = (at_1000.0 - origin.0, at_1000.1 - origin.1);
    eprintln!(
        "Orbium (tutorial), ticks 0–1000: displacement (x, y) = ({dx:.1}, {dy:.1}), {:.1} cells, {:.2}° from +y",
        dx.hypot(dy),
        dx.atan2(dy).to_degrees()
    );
    let late = window(&mut s, 1000, 5000);
    report("Orbium (tutorial)", 100, 500, &early);
    report("Orbium (tutorial)", 500, 1000, &steady);
    report("Orbium (tutorial)", 1000, 5000, &late);
    for w in [&early, &steady, &late] {
        assert!(w.min_mass > 70.0 && w.max_mass < 76.0);
    }
    assert!((steady.speed - 0.602).abs() < 0.01);
    assert!((late.speed - 0.602).abs() < 0.02);
}

#[test]
#[ignore = "long run; see docs/models/continuous.md"]
fn animals_json_presets_long_windows() {
    let mut chan = sim("lenia.orbium_chan");
    let w = window(&mut chan, 400, 1000);
    report("Orbium (animals.json, poly/poly)", 400, 1000, &w);
    assert!(w.min_mass > 72.0 && w.max_mass < 75.5 && (w.speed - 0.623).abs() < 0.015);

    let mut scutium = sim("lenia.scutium_solidus");
    let w = window(&mut scutium, 500, 2000);
    report("Scutium solidus", 500, 2000, &w);
    assert!((w.min_mass - 139.5).abs() < 3.0 && (w.max_mass - 139.5).abs() < 3.0);
    assert!((w.speed - 0.36).abs() < 0.02);

    let mut hydro = sim("lenia.hydrogeminium_natans");
    let m0 = hydro.mass();
    let w = window(&mut hydro, 400, 2000);
    report("Hydrogeminium natans", 400, 2000, &w);
    eprintln!("  initial mass {m0:.2}, final {:.2}", hydro.mass());
    assert!((m0 - 689.0).abs() < 1.0);
    assert!(w.min_mass > 650.0 && w.max_mass < 750.0 && (0.38..=0.45).contains(&w.speed));
}

#[test]
#[ignore = "long run; see docs/models/continuous.md"]
fn multi_kernel_presets_long_windows() {
    let mut tess = sim("lenia.tessellatium_gyrans");
    let m0 = tess.mass();
    let w = window(&mut tess, 200, 3000);
    report("Tessellatium gyrans", 200, 3000, &w);
    eprintln!("  initial mass {m0:.2}, active fraction {:?}", tess.metrics().get("active_fraction"));
    assert!(w.min_mass > 780.0 && w.max_mass < 840.0 && (0.5..0.7).contains(&w.speed));

    let mut fish = sim("lenia.fish");
    let w = window(&mut fish, 200, 3000);
    report("K=3 Fish", 200, 3000, &w);
    assert!(w.min_mass > 95.0 && w.max_mass < 125.0 && w.speed > 0.1);
}

#[test]
#[ignore = "long run; see docs/models/continuous.md"]
fn smoothlife_glider_1000_steps() {
    // Research note §5.4 (numpy f64): mass 799.4 ± 1.5, speed 5.281 ± 0.008.
    let mut s = sim("smoothlife.glider");
    let short = window(&mut s, 50, 300);
    let long = window(&mut s, 300, 1050);
    report("SmoothLife glider", 50, 300, &short);
    report("SmoothLife glider", 300, 1050, &long);
    for w in [&short, &long] {
        assert!(w.min_mass > 795.0 && w.max_mass < 803.0);
        assert!((w.speed - 5.28).abs() < 0.08);
    }
}

#[test]
#[ignore = "long run; see docs/models/continuous.md"]
fn flow_lenia_conserves_mass_over_1000_steps() {
    let mut s = sim("flow_lenia.demo");
    let m0 = s.mass();
    let mut worst = 0.0f64;
    for _ in 0..1000 {
        s.step();
        worst = worst.max(((s.mass() - m0) / m0).abs());
    }
    let active = s.metrics().get("active_fraction").unwrap_or(1.0);
    eprintln!("Flow-Lenia demo: mass {m0:.4}, worst relative error over 1000 ticks {worst:.2e}, active {active:.3}");
    assert!(worst < 1e-6);
    assert!(active < 0.25);
}
