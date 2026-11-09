//! Garbage collection, node limits and rule changes never change results.

use hashlife::{MIN_NODE_LIMIT, Rule, Universe};

use crate::common::{GOSPER_GUN, LIFE, cells_of, naive_run, soup, universe_with};

#[test]
fn explicit_collection_between_steps_changes_nothing() {
    let start = soup(5, -24, -24, 48, 48, 0.4);
    let mut reference = universe_with(&start, LIFE.0, LIFE.1);
    let mut collected = universe_with(&start, LIFE.0, LIFE.1);
    for k in [0u32, 3, 1, 4, 2, 5, 0, 3] {
        reference.step_pow2(k).unwrap();
        collected.step_pow2(k).unwrap();
        let before = collected.node_count();
        let freed = collected.garbage_collect();
        assert_eq!(collected.node_count(), before - freed);
        assert_eq!(cells_of(&collected), cells_of(&reference), "after a 2^{k} step");
    }
    assert_eq!(collected.generation(), reference.generation());
    assert_eq!(cells_of(&collected), naive_run(&start, LIFE.0, LIFE.1, collected.generation()));
}

#[test]
fn collection_during_steps_with_a_tiny_node_limit_changes_nothing() {
    let start = soup(6, -32, -32, 64, 64, 0.35);
    let mut reference = universe_with(&start, LIFE.0, LIFE.1);
    let mut tight = universe_with(&start, LIFE.0, LIFE.1);
    for k in [6u32, 2, 7, 0, 5] {
        reference.step_pow2(k).unwrap();
        // Re-arming the (minimum) limit makes collections run as soon as the
        // arena reaches it and again whenever the live set doubles.
        tight.set_node_limit(0);
        assert_eq!(tight.node_limit(), MIN_NODE_LIMIT);
        let collections = tight.gc_count();
        tight.step_pow2(k).unwrap();
        if k >= 6 {
            // A step collects at most once before its recursion starts, so
            // more than one collection proves collections ran mid-step.
            assert!(tight.gc_count() >= collections + 2, "a 2^{k} step should collect while recursing");
        }
        assert_eq!(cells_of(&tight), cells_of(&reference), "after a 2^{k} step");
    }
    assert_eq!(cells_of(&tight), naive_run(&start, LIFE.0, LIFE.1, tight.generation()));
}

#[test]
fn memoised_results_survive_collection_and_stay_valid() {
    // A gun stepped with a fixed exponent reuses results heavily; collecting
    // between steps keeps results of live nodes (and their nodes) alive.
    let mut reference = Universe::conway();
    reference.insert_cells(GOSPER_GUN);
    let mut collected = reference.clone();
    for _ in 0..40 {
        reference.step_pow2(8).unwrap();
        collected.step_pow2(8).unwrap();
        collected.garbage_collect();
        assert_eq!(collected.population(), reference.population());
    }
    assert_eq!(cells_of(&collected), cells_of(&reference));
    // After a collection, stepping again reuses kept results: far fewer
    // new nodes are needed than for a cold start.
    let mut cold = Universe::conway();
    cold.insert_cells(cells_of(&collected));
    cold.set_generation(collected.generation());
    let warm_before = collected.node_count();
    collected.step_pow2(8).unwrap();
    cold.step_pow2(8).unwrap();
    assert_eq!(cells_of(&collected), cells_of(&cold));
    assert!(collected.node_count() - warm_before < cold.node_count(), "kept results must be reused");
}

#[test]
fn changing_the_rule_discards_memoised_results() {
    let start = soup(9, -16, -16, 32, 32, 0.4);
    let mut universe = universe_with(&start, LIFE.0, LIFE.1);
    universe.step_pow2(4).unwrap();
    let after_life = naive_run(&start, LIFE.0, LIFE.1, 16);
    assert_eq!(cells_of(&universe), after_life);
    let highlife = Rule::highlife();
    universe.set_rule(highlife);
    universe.step_pow2(4).unwrap();
    assert_eq!(cells_of(&universe), naive_run(&after_life, highlife.birth(), highlife.survive(), 16));
    // And back: Life results computed before must not be reused blindly.
    universe.set_rule(Rule::conway());
    let before = cells_of(&universe);
    universe.step_pow2(4).unwrap();
    assert_eq!(cells_of(&universe), naive_run(&before, LIFE.0, LIFE.1, 16));
}

#[test]
fn node_count_and_memory_are_reported() {
    let mut universe = Universe::conway();
    let empty_nodes = universe.node_count();
    assert!(empty_nodes >= 62, "canonical empty nodes for levels 3..=64");
    universe.insert_cells(GOSPER_GUN);
    universe.advance(1000).unwrap();
    assert!(universe.node_count() > empty_nodes);
    assert!(universe.memory_bytes() >= universe.node_count() * 32);
    let peak = universe.memory_bytes();
    universe.clear();
    universe.garbage_collect();
    assert_eq!(universe.node_count(), empty_nodes);
    // Explicit collections compact the arena and give the memory back.
    assert!(universe.memory_bytes() < peak / 4, "{} bytes after clearing, {peak} before", universe.memory_bytes());
    universe.insert_cells(GOSPER_GUN);
    universe.advance(1000).unwrap();
    let mut reference = Universe::conway();
    reference.insert_cells(GOSPER_GUN);
    reference.advance(1000).unwrap();
    assert_eq!(cells_of(&universe), cells_of(&reference), "the compacted store keeps working");
}

#[test]
fn clear_memo_frees_results_without_changing_the_future() {
    let mut kept = Universe::conway();
    kept.insert_cells(GOSPER_GUN);
    kept.step_pow2(10).unwrap();
    let mut cleared = kept.clone();
    let before = cleared.node_count();
    let freed = cleared.clear_memo();
    assert!(freed > 0);
    assert_eq!(cleared.node_count(), before - freed);
    kept.garbage_collect();
    assert!(cleared.node_count() < kept.node_count(), "the memo is what clear_memo releases");
    kept.step_pow2(10).unwrap();
    cleared.step_pow2(10).unwrap();
    assert_eq!(cells_of(&cleared), cells_of(&kept));
}
