//! SmoothLife glider persistence and Flow-Lenia mass conservation.

use continuous::scenario::builtin_by_id;
use continuous::{Blend, ContinuousCommand, ContinuousSim, CreatureCells, Placement, Rule};
use simulation_core::Simulation;

fn sim(id: &str) -> ContinuousSim {
    let scenario = builtin_by_id(id).unwrap_or_else(|| panic!("missing scenario {id}"));
    ContinuousSim::from_scenario(scenario).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn smoothlife_paper_glider_persists() {
    // Reference (numpy f64 reproduction, research note §5.4): mass 799.4 ± 1.5,
    // speed 5.281 ± 0.008 cells/step.
    let mut s = sim("smoothlife.glider");
    s.advance(50);
    let start = s.centroid_track();
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for _ in 0..250 {
        s.step();
        lo = lo.min(s.mass());
        hi = hi.max(s.mass());
    }
    let end = s.centroid_track();
    let speed = (end.0 - start.0).hypot(end.1 - start.1) / 250.0;
    assert!(lo > 790.0 && hi < 810.0, "mass band [{lo}, {hi}]");
    assert!((speed - 5.28).abs() < 0.05, "speed {speed}");
    // Still a single compact object: a few percent of the world is active.
    let active = s.metrics().get("active_fraction").unwrap_or(1.0);
    assert!(active < 0.05, "active fraction {active}");
}

#[test]
fn smoothlife_soup_condenses_into_a_glider_whatever_the_rounding() {
    // Discrete SmoothLife soups are chaotic: the fate of a seed can flip with
    // a change in floating-point rounding (FFT vs direct convolution), so the
    // shipped seed was chosen for an outcome that does not depend on it: a
    // translated copy of the soup (different FFT rounding, same dynamics on
    // the torus) must also condense into isolated smooth gliders.
    let original = sim("smoothlife.soup");
    let mut shifted = sim("smoothlife.soup");
    let cells = CreatureCells::from_planes(shifted.fields().to_vec()).expect("valid cells");
    shifted.apply(ContinuousCommand::Clear).expect("clear");
    let placement =
        Placement { x: 128.0 + 77.0, y: 128.0 + 31.0, rotation_deg: 0.0, scale: 1.0, blend: Blend::Replace };
    shifted.apply(ContinuousCommand::PlaceCreature { cells, placement }).expect("exact copy");
    assert!((shifted.mass() - original.mass()).abs() < 1e-6);
    for (name, mut s) in [("original", original), ("translated", shifted)] {
        for _ in 0..400 {
            s.step();
            assert!(s.fields()[0].as_slice().iter().all(|&f| (0.0..=1.0).contains(&f)));
        }
        // Only isolated gliders remain: the mass is a small multiple of one
        // glider's (≈ 799) and few cells are active. (Measured: the original
        // keeps one glider, the translated copy two.)
        let mass = s.mass();
        let gliders = (mass / 799.4).round();
        let active = s.metrics().get("active_fraction").unwrap_or(1.0);
        assert!((1.0..=3.0).contains(&gliders), "{name}: mass {mass}");
        assert!((mass - gliders * 799.4).abs() < 6.0 * gliders, "{name}: mass {mass} is not {gliders} glider(s)");
        assert!(active < 0.02 * gliders, "{name}: active fraction {active}");
    }
}

#[test]
fn flow_lenia_conserves_mass_and_stays_localised() {
    let mut s = sim("flow_lenia.demo");
    assert!(matches!(s.rule(), Rule::FlowLenia(_)));
    let m0 = s.mass();
    assert!(m0 > 100.0);
    let mut worst = 0.0f64;
    for _ in 0..500 {
        s.step();
        worst = worst.max(((s.mass() - m0) / m0).abs());
        assert!(s.fields()[0].as_slice().iter().all(|&a| a >= 0.0 && a.is_finite()));
    }
    assert!(worst < 1e-4, "relative mass error {worst}");
    let drift = s.metrics().get("mass_drift").unwrap_or(1.0);
    assert!(drift.abs() < 1e-4);
    // The matter forms a localised creature rather than spreading out.
    let active = s.metrics().get("active_fraction").unwrap_or(1.0);
    assert!(active < 0.5, "active fraction {active}");
}

#[test]
fn flow_lenia_mass_conservation_is_tight_per_step() {
    let mut s = sim("flow_lenia.demo");
    let mut prev = s.mass();
    for _ in 0..50 {
        s.step();
        let m = s.mass();
        assert!(((m - prev) / prev).abs() < 1e-6, "per-step drift {}", (m - prev) / prev);
        prev = m;
    }
}
