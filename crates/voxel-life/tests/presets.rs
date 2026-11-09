//! The "observed" and "documented" presets show, with their shipped seeds,
//! the behaviour their descriptions state. The research runs used a
//! different generator, so these numbers are this build's own measurements
//! (quoted in the scenario descriptions and docs/models/voxel-life.md); the
//! tests keep descriptions and presets from drifting apart.

mod common;

use common::*;
use simulation_core::Simulation;

/// Runs a built-in scenario for `generations` and reports the first
/// generation whose bounding box touches the world boundary, the generation
/// at which the world became empty, and the population after every
/// generation (index 0 = initial state).
fn run(id: &str, generations: u64) -> (Option<u64>, Option<u64>, Vec<u64>) {
    let mut life = builtin(id);
    let world = life.bounds();
    let mut touched = None;
    let mut extinct = None;
    let mut populations = vec![life.stats().population()];
    for generation in 1..=generations {
        life.step();
        populations.push(life.stats().population());
        match life.stats().bounds() {
            Some(b) if touched.is_none() && (0..3).any(|a| b.min[a] == world.min[a] || b.max[a] == world.max[a]) => {
                touched = Some(generation);
            }
            None if extinct.is_none() => extinct = Some(generation),
            _ => {}
        }
    }
    (touched, extinct, populations)
}

#[test]
fn softology_445_reaches_the_boundary_then_dies_out() {
    let (touched, extinct, _) = run("softology-445", 60);
    assert_eq!((touched, extinct), (Some(29), Some(47)));
}

#[test]
fn clouds_1_keeps_three_quarters_of_the_volume() {
    let (_, _, populations) = run("clouds-1", 60);
    let fraction = populations[60] as f64 / f64::from(64 * 64 * 64);
    assert!((0.75..0.77).contains(&fraction), "{fraction}");
}

#[test]
fn amoeba_and_pyroclastic_grow_slowly_to_the_boundary() {
    let (touched, extinct, populations) = run("softology-amoeba", 150);
    assert_eq!((touched, extinct), (Some(47), None));
    assert!(populations[150] > 200_000, "the amoeba fills most of the box: {}", populations[150]);
    let (touched, extinct, _) = run("softology-pyroclastic", 150);
    assert_eq!((touched, extinct), (Some(54), None));
}

#[test]
fn builder_1_grows_then_partially_decays() {
    let (_, extinct, populations) = run("builder-1", 150);
    let peak = populations.iter().copied().max().unwrap_or(0);
    assert_eq!(extinct, None);
    assert!(peak > 23_000 && (50..=70).contains(&populations.iter().position(|&p| p == peak).unwrap_or(0)));
    assert_eq!(populations[150], 14_471, "peak {peak}");
}

#[test]
fn bays_5766_soup_stabilises_with_bounded_growth() {
    let mut life = builtin("life-5766-soup");
    let mut previous = positions(&life);
    let mut settled = None;
    for generation in 1..=100u64 {
        life.step();
        let now = positions(&life);
        let bounds = life.stats().bounds().map(|b| (b.min, b.max));
        assert!(bounds.is_some_and(|(min, max)| min.iter().chain(&max).all(|c| (16..48).contains(c))), "{bounds:?}");
        match (now == previous, settled) {
            (true, None) => settled = Some(generation - 1),
            (false, _) => settled = None,
            _ => {}
        }
        previous = now;
    }
    assert_eq!(settled, Some(29), "a still life from generation 29 on");
    assert_eq!(previous.len(), 35);
}
