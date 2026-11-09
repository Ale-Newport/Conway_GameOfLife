//! Heritable versus acquired traits in a running population, and the
//! genealogy export.

use std::collections::BTreeSet;

use evolution::config::{EvolutionConfig, ReproductionMode};
use evolution::genes::{ACCLIMATION, EXPERIENCE, HERITABLE_COUNT};
use evolution::{EvolutionSim, EvolutionViewRequest, Species};
use simulation_core::Simulation;

/// Asexual, mutation-free population under a warming climate with predators,
/// so adults acquire acclimation and predator experience during life.
fn config() -> EvolutionConfig {
    let mut c = EvolutionConfig::default();
    c.world.width = 48;
    c.world.height = 40;
    c.prey.initial = 150;
    c.mutation.rate = 0.0;
    c.climate.shift_rate = 8.0;
    c.climate.gradient_x = 10.0;
    c.predators.initial = 10;
    c.predators.constant = true;
    c
}

#[test]
fn offspring_inherit_genes_exactly_but_never_acquired_traits() {
    let mut sim = EvolutionSim::new(config(), 4).unwrap();
    let mut checked_pairs = 0;
    let mut checked_newborns = 0;
    let mut acclimated_parents = 0;
    for _ in 0..40 {
        sim.advance(10);
        let w = sim.world();
        for (id, o) in w.organisms().iter() {
            if o.age == 0 {
                checked_newborns += 1;
                assert_eq!(o.genome.express(w.schema(), ACCLIMATION), 0.0, "newborn {id} starts unacclimated");
                assert_eq!(o.genome.express(w.schema(), EXPERIENCE), 0.0, "newborn {id} starts naive");
                assert!(o.health == 1.0 && o.offspring == 0 && o.refractory == 0, "fresh individual state");
            }
            let Some(parent) = o.lineage.parents[0].and_then(|p| w.organisms().get(p)) else {
                continue;
            };
            checked_pairs += 1;
            assert_eq!(
                &o.genome.values()[..HERITABLE_COUNT],
                &parent.genome.values()[..HERITABLE_COUNT],
                "without mutation, asexual offspring of {} carry exactly the parent's genes",
                o.lineage.parents[0].map_or(0, |p| p.0)
            );
            assert_eq!(o.founder, parent.founder, "lineage follows the first parent");
            if parent.genome.express(w.schema(), ACCLIMATION).abs() > 0.5 {
                acclimated_parents += 1;
            }
        }
    }
    assert!(checked_newborns > 50, "{checked_newborns} newborns checked");
    assert!(checked_pairs > 100, "{checked_pairs} parent/offspring pairs checked");
    assert!(acclimated_parents > 20, "parents did acclimate ({acclimated_parents}), yet offspring start at 0");
    let experienced = sim
        .world()
        .organisms()
        .iter()
        .filter(|(_, o)| o.species == Species::Prey && o.genome.values()[EXPERIENCE] > 0.0)
        .count();
    assert!(experienced > 0, "some prey learned from predator encounters");
}

#[test]
fn genealogy_export_covers_the_living_and_stays_pruned() {
    let mut c = config();
    c.reproduction.mode = ReproductionMode::Sexual;
    c.statistics.genealogy_max_nodes = 2000;
    let mut sim = EvolutionSim::new(c, 9).unwrap();
    sim.advance(600);
    let view = sim.view(&EvolutionViewRequest { genealogy: true, ..Default::default() });
    let export = view.genealogy.expect("genealogy requested");
    let recorded: BTreeSet<_> = export.nodes.iter().map(|n| n.id).collect();
    for o in &view.organisms {
        assert!(recorded.contains(&o.id), "living organism {} is in the tree", o.id);
    }
    for e in &export.edges {
        assert!(recorded.contains(&e.parent) && recorded.contains(&e.child));
        assert!(e.parent < e.child, "parents are older than their children");
    }
    let living: u32 = export.lineages.iter().map(|l| l.living).sum();
    assert_eq!(living as usize, view.organisms.len(), "lineage counts cover every organism");
    assert!(export.nodes.len() <= 2000.max(view.organisms.len()), "node cap respected");
    assert!(export.summary.births > export.nodes.len() as u64, "dead lines were pruned");
    let dead_recorded = export.nodes.iter().filter(|n| n.death_tick.is_some()).count();
    assert!(dead_recorded > 0, "ancestors of the living are kept");
    assert!(sim.metrics().get("max_generation").unwrap_or(0.0) >= 2.0);
}
