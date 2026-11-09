//! Genetic drift: a neutral marker drifts towards fixation in small isolated
//! populations and the isolated populations drift apart, while a large
//! population keeps most of its variation.
//!
//! Sexual reproduction with free recombination (uniform crossover) and no
//! mutation, so the neutral marker changes only by sampling (who happens to
//! breed). Expected (Wright 1931): within-population variance decays as
//! `(1 − 1/N_e)` per generation, so much faster on small islands (≈ 20–30
//! prey) than on the mainland (≈ 550), and the islands' means wander apart.
//!
//! Small sexual island populations can also die out (demographic chance and
//! mate scarcity: 8 of 40 islands in seeds 1–10 over this run), so drift is
//! measured on the islands that persist, pooled over four seeds, each island
//! against its own founders. Differentiation is the neutral `Q_ST`/`F_ST`
//! analogue `among / (among + within)` of the marker's variance.

use evolution::config::{Barrier, EvolutionConfig, ReproductionMode, Shape, Terrain};
use evolution::genes::MARKER_A;
use evolution::{EvolutionSim, Species};
use simulation_core::Simulation;

const TICKS: u64 = 2500;
const SEEDS: [u64; 4] = [1, 2, 3, 4];

fn config(isolated: bool) -> EvolutionConfig {
    let mut c = EvolutionConfig::default();
    c.world.width = 48;
    c.world.height = 48;
    c.prey.initial = if isolated { 80 } else { 400 };
    c.prey.spread = 0.2;
    c.reproduction.mode = ReproductionMode::Sexual;
    c.mutation.rate = 0.0;
    c.speciation.enabled = false;
    c.barriers.push(Barrier {
        name: "ocean".into(),
        terrain: Terrain::Water,
        shapes: [(12.0, 12.0), (36.0, 12.0), (12.0, 36.0), (36.0, 36.0)]
            .into_iter()
            .map(|(x, y)| Shape::Disc { x, y, r: 7.0 })
            .collect(),
        outside: true,
        active: isolated,
    });
    c
}

fn variance(xs: &[f64]) -> f64 {
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / xs.len() as f64
}

/// Marker values of the living prey on each region, by region label.
fn marker_by_region(sim: &EvolutionSim) -> Vec<Vec<f64>> {
    let w = sim.world();
    let mut groups: Vec<Vec<f64>> = vec![Vec::new(); w.environment().region_count()];
    for (_, o) in w.organisms().iter().filter(|(_, o)| o.species == Species::Prey) {
        if let Some(r) = w.environment().region_at(o.kinematics.position) {
            groups[usize::from(r)].push(f64::from(o.genome.values()[MARKER_A]));
        }
    }
    groups
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Prey an island must still hold at the end to count as persisting.
const MIN_SURVIVORS: usize = 5;

/// Differentiation of the marker among `groups` (a neutral `Q_ST`/`F_ST`
/// analogue): variance among group means over itself plus the mean
/// within-group variance.
fn differentiation(groups: &[&Vec<f64>]) -> f64 {
    let among = variance(&groups.iter().map(|g| mean(g)).collect::<Vec<_>>());
    let within = groups.iter().map(|g| variance(g)).sum::<f64>() / groups.len() as f64;
    among / (among + within)
}

#[test]
fn neutral_marker_drifts_to_fixation_on_small_islands_and_islands_diverge() {
    let (mut island_ratios, mut mainland_ratios) = (Vec::new(), Vec::new());
    let mut differentiation_change = Vec::new();
    for seed in SEEDS {
        let mut islands = EvolutionSim::new(config(true), seed).expect("valid config");
        let mut mainland = EvolutionSim::new(config(false), seed).expect("valid config");
        let start = marker_by_region(&islands);
        assert_eq!(start.iter().filter(|g| g.len() >= 2).count(), 4, "founders on all four islands");
        let main_start = variance(&marker_by_region(&mainland).concat());
        islands.advance(TICKS);
        mainland.advance(TICKS);
        let end = marker_by_region(&islands);
        mainland_ratios.push(variance(&marker_by_region(&mainland).concat()) / main_start);
        // Small sexual populations can die out (demographic chance and mate
        // scarcity, as in real island populations); drift is measured on the
        // islands that persist, each against its own founders.
        let persisting: Vec<usize> = (0..end.len()).filter(|&r| end[r].len() >= MIN_SURVIVORS).collect();
        assert!(persisting.len() >= 2, "seed {seed}: at least two islands persist");
        let ratios: Vec<f64> = persisting.iter().map(|&r| variance(&end[r]) / variance(&start[r])).collect();
        let pick = |groups: &[Vec<f64>]| differentiation(&persisting.iter().map(|&r| &groups[r]).collect::<Vec<_>>());
        let (d_start, d_end) = (pick(&start), pick(&end));
        println!(
            "seed {seed}: prey per island {:?}, mainland prey {}, generations {:.1}; within-island variance ratios \
             {ratios:.3?}, mainland {:.3}; differentiation {d_start:.3} -> {d_end:.3}",
            end.iter().map(Vec::len).collect::<Vec<_>>(),
            mainland.world().count(Species::Prey),
            islands.metrics().get("mean_generation").unwrap_or(0.0),
            mainland_ratios[mainland_ratios.len() - 1],
        );
        island_ratios.extend(ratios);
        differentiation_change.push((d_start, d_end));
    }
    let island = mean(&island_ratios);
    let main = mean(&mainland_ratios);
    let fixed = island_ratios.iter().filter(|r| **r < 0.05).count();
    let d_start = mean(&differentiation_change.iter().map(|d| d.0).collect::<Vec<_>>());
    let d_end = mean(&differentiation_change.iter().map(|d| d.1).collect::<Vec<_>>());
    println!(
        "pooled: {} persisting islands, mean variance ratio {island:.3} ({fixed} near fixation), mainland {main:.3}; \
         differentiation {d_start:.3} -> {d_end:.3}",
        island_ratios.len()
    );
    // Wright–Fisher: E[V_t / V_0] = (1 − 1/N_e)^t, at most ≈ 0.5 for N_e ≈ 25 over 16 generations
    // (and less with overlapping generations), but ≈ 0.97 for the mainland's N ≈ 550.
    assert!(island < 0.5, "drift erodes variation on small islands ({island:.3})");
    assert!(island < 0.5 * main, "much faster on islands ({island:.3}) than on the mainland ({main:.3})");
    assert!(4 * fixed >= island_ratios.len(), "the marker fixes on many islands ({fixed} of {})", island_ratios.len());
    assert!(d_end > 0.3 && d_end > 3.0 * d_start, "isolated islands diverge: {d_start:.3} -> {d_end:.3}");
}
