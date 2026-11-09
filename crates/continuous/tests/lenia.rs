//! Behavioural tests of Lenia presets against the reference numbers of
//! `docs/research/continuous-life.md` §4 and §8 (numpy f64 and an
//! independent Rust f32 implementation).

use continuous::scenario::builtin_by_id;
use continuous::{ContinuousSim, ResolvedMethod};
use simulation_core::{ParamValue, Simulation};

fn sim(id: &str) -> ContinuousSim {
    let scenario = builtin_by_id(id).unwrap_or_else(|| panic!("missing scenario {id}"));
    ContinuousSim::from_scenario(scenario).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Mass band and mean centroid speed (cells/tick) over `[from, to)`.
struct Run {
    min_mass: f64,
    max_mass: f64,
    speed: f64,
    heading_change_deg: f64,
}

fn run(sim: &mut ContinuousSim, from: u64, to: u64) -> Run {
    sim.advance(from - sim.tick());
    let start = sim.centroid_track();
    let mid_tick = (from + to) / 2;
    let mut mid = start;
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    while sim.tick() < to {
        sim.step();
        let m = sim.mass();
        lo = lo.min(m);
        hi = hi.max(m);
        if sim.tick() == mid_tick {
            mid = sim.centroid_track();
        }
    }
    let end = sim.centroid_track();
    let speed = (end.0 - start.0).hypot(end.1 - start.1) / (to - from) as f64;
    let h1 = (mid.1 - start.1).atan2(mid.0 - start.0);
    let h2 = (end.1 - mid.1).atan2(end.0 - mid.0);
    Run { min_mass: lo, max_mass: hi, speed, heading_change_deg: (h2 - h1).to_degrees().abs() }
}

#[test]
fn orbium_tutorial_matches_reference_mass_trajectory_and_speed() {
    let mut s = sim("lenia.orbium");
    assert_eq!(s.resolved_method(), ResolvedMethod::Fft);
    // Reference (research note §7.4/§8: independent Rust f32 run, numpy f64
    // agrees to ±0.0003): Σ A = 75.10 (t = 0), 73.5506 (10), 73.0977 (100),
    // 72.9739 (200), 72.9989 (500). The kernel, growth, placement and update
    // must all match for the trajectory to agree to 10⁻³.
    let start = s.centroid_track();
    assert!((s.mass() - 75.10).abs() < 1e-3, "t=0 mass {}", s.mass());
    for (t, expected) in [(10, 73.5506), (100, 73.0977), (200, 72.9739), (500, 72.9989)] {
        s.advance(t - s.tick());
        assert!((s.mass() - expected).abs() < 1e-3, "t={t}: mass {} vs {expected}", s.mass());
    }
    // Steady state over steps 500–1000: mass in [72.4, 73.3] (we allow
    // [70, 76]) and speed 0.602 ± 0.01 cells/step (we allow [0.58, 0.63]).
    let r = run(&mut s, 500, 1000);
    assert!(r.min_mass > 70.0 && r.max_mass < 76.0, "mass band [{}, {}]", r.min_mass, r.max_mass);
    assert!((0.58..=0.63).contains(&r.speed), "speed {}", r.speed);
    assert!(r.heading_change_deg < 2.0, "heading changed by {}°", r.heading_change_deg);
    // Direction (research note §8): displacement (rows, columns) ≈ (560.0,
    // 222.2) after 1000 steps for the cells as given, i.e. down and to the
    // right at 21.6° from the +y axis. A mirrored or rotated placement, or a
    // transposed kernel/FFT layout, fails this.
    let end = s.centroid_track();
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let heading = dx.atan2(dy).to_degrees();
    assert!((heading - 21.6).abs() < 0.5, "heading {heading}° from +y, displacement ({dx}, {dy})");
    assert!((dx.hypot(dy) - 602.5).abs() < 6.0, "distance {} after 1000 steps", dx.hypot(dy));
    // Keeps living well beyond 1000 steps.
    let r = run(&mut s, 1000, 1500);
    assert!(r.min_mass > 70.0 && r.max_mass < 76.0);
}

#[test]
fn orbium_animals_json_polynomial_functions() {
    // Reference: mass ≈ 73.6 (relative range 1.1 %), speed 0.623 cells/step.
    let mut s = sim("lenia.orbium_chan");
    assert!((s.mass() - 19_600.0 / 255.0).abs() < 1e-3);
    let r = run(&mut s, 400, 1000);
    assert!(r.min_mass > 72.0 && r.max_mass < 75.5, "mass band [{}, {}]", r.min_mass, r.max_mass);
    assert!((0.60..=0.645).contains(&r.speed), "speed {}", r.speed);
}

#[test]
fn scutium_solidus_survives() {
    // Reference (polynomial/polynomial, 2000 steps): mass 139.5, speed 0.36.
    let mut s = sim("lenia.scutium_solidus");
    let r = run(&mut s, 500, 1200);
    assert!(r.min_mass > 125.0 && r.max_mass < 155.0, "mass band [{}, {}]", r.min_mass, r.max_mass);
    assert!((0.30..=0.42).contains(&r.speed), "speed {}", r.speed);
}

#[test]
fn hydrogeminium_natans_survives() {
    // Reference (polynomial/polynomial, 256², 2000 steps): mass 689 → 699, speed ≈ 0.39–0.42.
    let mut s = sim("lenia.hydrogeminium_natans");
    let m0 = s.mass();
    assert!((m0 - 689.0).abs() < 1.0, "initial mass {m0}");
    let r = run(&mut s, 400, 1000);
    assert!(r.min_mass > 620.0 && r.max_mass < 760.0, "mass band [{}, {}]", r.min_mass, r.max_mass);
    assert!((0.3..=0.5).contains(&r.speed), "speed {}", r.speed);
}

#[test]
fn tessellatium_three_channel_creature_survives() {
    // Measured with this engine (no published reference at R = 12, scale 1):
    // the tutorial cells grow within ~100 ticks into a compact three-channel
    // glider of mass ≈ 810 moving ≈ 0.6 cells/tick.
    let mut s = sim("lenia.tessellatium_gyrans");
    assert_eq!(s.fields().len(), 3);
    let r = run(&mut s, 200, 1000);
    let mean = 0.5 * (r.min_mass + r.max_mass);
    assert!((r.max_mass - r.min_mass) / mean < 0.02, "mass band [{}, {}]", r.min_mass, r.max_mass);
    assert!((780.0..840.0).contains(&mean), "mean mass {mean}");
    assert!((0.5..0.7).contains(&r.speed), "speed {}", r.speed);
    // Every channel stays populated and the creature stays compact.
    for (c, g) in s.fields().iter().enumerate() {
        assert!(g.sum() > 150.0, "channel {c} mass {}", g.sum());
    }
    let active = s.metrics().get("active_fraction").unwrap_or(1.0);
    assert!(active < 0.1, "active fraction {active}");
}

#[test]
fn fish_multi_kernel_creature_survives() {
    let mut s = sim("lenia.fish");
    let m0 = s.mass();
    let r = run(&mut s, 200, 1000);
    assert!(r.min_mass > 0.5 * m0 && r.max_mass < 2.0 * m0, "mass band [{}, {}] from {m0}", r.min_mass, r.max_mass);
    assert!(r.speed > 0.05, "speed {}", r.speed);
}

#[test]
fn fft_and_direct_paths_agree_on_a_running_creature() {
    let mut fft = sim("lenia.orbium");
    let mut direct = sim("lenia.orbium");
    let policy = direct.set_param("convolution", ParamValue::Text("direct".into()));
    assert!(policy.is_ok());
    assert_eq!(direct.resolved_method(), ResolvedMethod::Direct);
    for _ in 0..30 {
        fft.step();
        direct.step();
    }
    let max = fft.fields()[0]
        .as_slice()
        .iter()
        .zip(direct.fields()[0].as_slice())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(max < 1e-3, "max |fft − direct| after 30 steps = {max}");
}

#[test]
fn lenia_values_stay_in_unit_interval() {
    let mut s = sim("lenia.soup");
    for _ in 0..60 {
        s.step();
        assert!(s.fields()[0].as_slice().iter().all(|&a| (0.0..=1.0).contains(&a)));
    }
}
