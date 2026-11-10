//! The seven presets load and show their intended qualitative dynamics.

mod common;

use common::{mean, preset, series, sim_of};
use rule_engine::{CustomLifeSim, scenarios};
use simulation_core::glam::Vec2;
use simulation_core::{Simulation, erase};

#[test]
fn builtin_presets_match_the_scenario_folder() {
    let builtin = scenarios::builtin().unwrap();
    assert_eq!(builtin.len(), 7);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/custom-life");
    let from_disk = scenarios::load_dir(&dir).unwrap();
    assert_eq!(from_disk.len(), 7);
    for s in &builtin {
        let disk = from_disk.iter().find(|d| d.id == s.id).expect("preset file on disk");
        assert_eq!(disk, s, "embedded copy of {} is stale", s.id);
        assert!(!s.description.is_empty() && !s.expected.is_empty());
        // Every preset compiles without warnings and runs.
        let mut sim = sim_of(s);
        assert!(sim.compiled().warnings().is_empty(), "{}: {:?}", s.id, sim.compiled().warnings());
        sim.advance(5);
        assert!(sim.script_errors().is_empty(), "{}: {:?}", s.id, sim.script_errors());
    }
}

#[test]
fn bacteria_grow_logistically_to_the_resource_limit() {
    let p = preset("bacterial-growth");
    let mut sim = sim_of(&p);
    let s = series(&mut sim, 800, 10, &["population.bacterium"]);
    let n = &s[0];
    // Carrying capacity predicted from the supply: regen × cells / maintenance.
    let w = &p.system.world;
    let k =
        f64::from(w.resources[0].regen) * f64::from(w.width * w.height) / p.system.entity_types[0].energy.metabolism;
    assert!(n[10] > 3.0 * n[0], "early growth: {} -> {}", n[0], n[10]);
    let plateau = mean(&n[60..]);
    assert!((plateau - k).abs() < 0.2 * k, "plateau {plateau:.0} vs K = {k:.0}");
    let peak = n.iter().copied().fold(0.0, f64::max);
    assert!(peak < 1.4 * k, "overshoot {peak} vs K {k}");
    // Per-capita growth falls with density (logistic signature).
    let rate = |i: usize| (n[i + 5] - n[i]) / (5.0 * n[i]);
    let early = rate(0);
    let late = mean(&(60..75).map(rate).collect::<Vec<_>>());
    assert!(early > 0.01 && late.abs() < early / 4.0, "per-capita growth early {early:.4}, late {late:.4}");
}

#[test]
fn virus_epidemics_crash_the_hosts_and_cycle() {
    let mut sim = sim_of(&preset("virus-and-cells"));
    let s = series(&mut sim, 1500, 10, &["population.cell", "population.virus", "population.infected"]);
    let (cells, virus) = (&s[0], &s[1]);
    let argmax = |v: &[f64]| v.iter().enumerate().fold((0, f64::MIN), |b, (i, &x)| if x > b.1 { (i, x) } else { b });
    let argmin = |v: &[f64]| v.iter().enumerate().fold((0, f64::MAX), |b, (i, &x)| if x < b.1 { (i, x) } else { b });
    let (peak_i, peak) = argmax(&cells[..40]);
    // The first epidemic: cells fall below half their peak; its trough is the
    // minimum of the following 300 ticks.
    let below =
        peak_i + cells[peak_i..].iter().position(|&c| c < 0.5 * peak).expect("the first epidemic crashes the hosts");
    let (t, crash) = argmin(&cells[below..(below + 30).min(cells.len())]);
    let crash_i = below + t;
    let (vpeak_i, _) = argmax(&virus[..crash_i.max(1)]);
    assert!(vpeak_i > peak_i, "the virus peaks after its hosts ({vpeak_i} vs {peak_i})");
    let recovery = cells[crash_i..].iter().copied().fold(0.0, f64::max);
    assert!(recovery > 2.0 * crash.max(1.0), "hosts recover after the crash ({crash} -> {recovery})");
    // The cycle repeats: count crashes below half the first peak, each
    // separated by a recovery above 60 % of it.
    let (mut epidemics, mut low) = (0, false);
    for &c in cells.iter() {
        if !low && c < 0.5 * peak {
            epidemics += 1;
            low = true;
        } else if low && c > 0.6 * peak {
            low = false;
        }
    }
    assert!(epidemics >= 2, "repeated epidemics within 1500 ticks, got {epidemics}");
    assert!(s[2].iter().any(|&i| i > 0.0), "infected cells appear");
    assert!(cells.iter().all(|&c| c > 0.0), "hosts persist");
}

#[test]
fn the_colony_with_lower_maintenance_excludes_its_rival() {
    let p = preset("competitive-colonies");
    let mut sim = sim_of(&p);
    let s = series(&mut sim, 1300, 10, &["population.amber", "population.violet"]);
    let (amber, violet) = (&s[0], &s[1]);
    let violet_peak = violet.iter().copied().fold(0.0, f64::max);
    let last = amber.len() - 1;
    assert!(amber[last] > 4.0 * violet[last], "amber {} vs violet {}", amber[last], violet[last]);
    assert!(violet[last] < 0.5 * violet_peak, "violet declines from its peak {violet_peak} to {}", violet[last]);
    // Shared resource: the total stays below the better competitor's capacity (+ noise).
    let w = &p.system.world;
    let k_amber =
        f64::from(w.resources[0].regen) * f64::from(w.width * w.height) / p.system.entity_types[0].energy.metabolism;
    assert!(amber.iter().zip(violet).all(|(a, v)| a + v < 1.2 * k_amber));
    let fights: u64 = sim
        .compiled()
        .rules()
        .iter()
        .zip(sim.rule_fire_totals())
        .filter(|(r, _)| r.name.contains("fight"))
        .map(|(_, n)| *n)
        .sum();
    assert!(fights > 0, "interference competition happened");
}

#[test]
fn alien_predators_control_grazers_in_a_trophic_cascade() {
    // With about 30 predators the dynamics are noisy and bimodal across seeds:
    // either the medusae persist food-limited, or the spikemaws overshoot and
    // the medusae survive only as a few drifting immigrants for a while (the
    // default seed). Seeds 1 and 5 cover both regimes; the cascade holds in all.
    let p = preset("alien-ecosystem");
    let keys = ["population.lumen_moss", "population.medusa", "population.spikemaw"];
    for seed in [p.seed, 1, 5] {
        let mut q = p.clone();
        q.seed = seed;
        let w = series(&mut sim_of(&q), 1500, 10, &keys);
        for (k, v, min) in [(keys[0], &w[0], 0.9), (keys[1], &w[1], 0.75), (keys[2], &w[2], 0.9)] {
            let present = v.iter().filter(|&&x| x > 0.0).count() as f64 / v.len() as f64;
            assert!(present > min, "seed {seed}: {k} present only {present:.2} of the time");
        }
        let mut without_predators = q.clone();
        let sys = &mut without_predators.system;
        sys.entity_types[2].initial.count = 0;
        sys.world.random_events.retain(|e| e.name != "spikemaw arrival");
        let c = series(&mut sim_of(&without_predators), 1500, 10, &keys);
        assert!(c[2].iter().all(|&x| x == 0.0));
        let (grazers, moss) = (mean(&c[1]) / mean(&w[1]), mean(&c[0]) / mean(&w[0]));
        assert!(grazers > 1.15, "seed {seed}: grazers ×{grazers:.2} without predators");
        assert!(moss < 0.6, "seed {seed}: moss ×{moss:.2} without predators");
    }
}

#[test]
fn energy_forms_a_pyramid_up_the_food_chain() {
    let mut sim = sim_of(&preset("producers-consumers"));
    let keys = [
        "energy.plant",
        "energy.grazer",
        "energy.hunter",
        "population.plant",
        "population.grazer",
        "population.hunter",
    ];
    let s = series(&mut sim, 2000, 10, &keys);
    let e: Vec<f64> = s[..3].iter().map(|v| mean(v)).collect();
    assert!(e[0] > 2.0 * e[1] && e[1] > e[2], "standing energy plants/grazers/hunters = {e:?}");
    let n: Vec<f64> = s[3..].iter().map(|v| mean(v)).collect();
    assert!(n[0] > n[1] && n[1] > n[2], "numbers pyramid {n:?}");
    for (k, v) in keys[3..].iter().zip(&s[3..]) {
        let present = v.iter().filter(|&&x| x > 0.0).count() as f64 / v.len() as f64;
        assert!(present > 0.75, "{k} present only {present:.2} of the time");
    }
}

/// Runs the chemistry preset with `a`/`b` initial molecules (and the
/// catalyst unless `catalyst` is false), checking atom conservation every
/// tick. Returns the C count per tick.
fn react(a: u32, b: u32, catalyst: bool, ticks: u64) -> Vec<f64> {
    let mut p = preset("artificial-chemistry");
    let types = &mut p.system.entity_types;
    types[0].initial.count = a;
    types[1].initial.count = b;
    if !catalyst {
        types[3].initial.count = 0;
    }
    let mut sim = sim_of(&p);
    let k = sim.population("k");
    let mut c_series = Vec::new();
    for _ in 0..ticks {
        let (na, nb, nc) = (sim.population("a"), sim.population("b"), sim.population("c"));
        assert_eq!(na + nc, a as usize, "A atoms conserved at tick {}", sim.tick());
        assert_eq!(nb + nc, b as usize, "B atoms conserved at tick {}", sim.tick());
        assert_eq!(sim.population("k"), k, "the catalyst is never consumed");
        c_series.push(nc as f64);
        sim.step();
    }
    c_series
}

#[test]
fn chemistry_conserves_atoms_and_obeys_mass_action() {
    let equal = react(300, 300, true, 1200);
    let skewed = react(450, 150, true, 1200);
    // Equilibrium constant K = [C] / ([A][B]) from the late part of each run.
    let k_of = |c: &[f64], a: f64, b: f64| {
        let c = mean(&c[600..]);
        c / ((a - c) * (b - c))
    };
    let (k1, k2) = (k_of(&equal, 300.0, 300.0), k_of(&skewed, 450.0, 150.0));
    assert!(mean(&equal[600..]) > 50.0, "C forms");
    assert!((k1 - k2).abs() < 0.3 * k1, "law of mass action: K = {k1:.2e} vs {k2:.2e}");
}

#[test]
fn the_catalyst_speeds_up_equilibration_without_shifting_it() {
    let fast = react(300, 300, true, 1000);
    let slow = react(300, 300, false, 2500);
    let (eq_fast, eq_slow) = (mean(&fast[500..]), mean(&slow[1500..]));
    assert!((eq_fast - eq_slow).abs() < 0.3 * eq_fast, "equilibria {eq_fast:.0} vs {eq_slow:.0}");
    // Time to get half-way to equilibrium.
    let half_time = |c: &[f64], eq: f64| c.iter().position(|&x| x >= 0.5 * eq).unwrap_or(c.len());
    let (t_fast, t_slow) = (half_time(&fast, eq_fast), half_time(&slow, eq_slow));
    assert!(2 * t_fast < t_slow, "half-way after {t_fast} ticks with vs {t_slow} without catalyst");
}

/// Clark–Evans aggregation index on a torus: mean nearest-neighbour distance
/// divided by its expectation 0.5/√density under complete spatial randomness.
fn clark_evans(points: &[Vec2], w: f32, h: f32) -> f64 {
    let d = |a: Vec2, b: Vec2| {
        let dx = (a.x - b.x).abs().min(w - (a.x - b.x).abs());
        let dy = (a.y - b.y).abs().min(h - (a.y - b.y).abs());
        f64::from((dx * dx + dy * dy).sqrt())
    };
    let nn: Vec<f64> = points
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            points.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, &q)| d(p, q)).fold(f64::MAX, f64::min)
        })
        .collect();
    let density = points.len() as f64 / f64::from(w * h);
    mean(&nn) / (0.5 / density.sqrt())
}

fn robot_index(sim: &CustomLifeSim) -> f64 {
    let pts: Vec<Vec2> = sim.entities().map(|(_, e)| e.kin.position).collect();
    let w = &sim.system().world;
    clark_evans(&pts, w.width as f32, w.height as f32)
}

#[test]
fn robots_aggregate_through_local_rules() {
    let p = preset("robot-swarm");
    let mut sim = sim_of(&p);
    let start = robot_index(&sim);
    sim.advance(400);
    let end = robot_index(&sim);
    assert!(sim.script_errors().is_empty(), "{:?}", sim.script_errors());
    let mut control = p.clone();
    control.system.rules.clear();
    control.system.scripts.clear();
    let mut walkers = sim_of(&control);
    walkers.advance(400);
    let random = robot_index(&walkers);
    assert!((0.8..1.2).contains(&start), "uniform start ≈ 1, got {start:.2}");
    assert!((0.8..1.2).contains(&random), "pure walkers stay random, got {random:.2}");
    assert!(end < 0.7, "aggregated swarm index {end:.2}");
    let waiting = sim.view(&Default::default()).entities.iter().filter(|e| e.state == 1).count();
    assert!(waiting > 75, "most robots wait in clusters ({waiting})");
}

#[test]
fn presets_replay_deterministically_and_restore_from_snapshots() {
    let p = preset("virus-and-cells");
    let mut a = erase(sim_of(&p));
    let mut b = erase(sim_of(&p));
    a.advance(120);
    b.advance(120);
    assert_eq!(a.metrics(), b.metrics());
    let snap = a.snapshot().unwrap();
    a.advance(80);
    let later = a.metrics();
    b.restore(&snap).unwrap();
    b.advance(80);
    assert_eq!(b.metrics(), later, "restored run must replay identically");
}
