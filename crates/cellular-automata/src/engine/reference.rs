//! A deliberately naive reference stepper, used by the tests to check every
//! engine. It applies the definition literally: for every candidate cell,
//! count the live cells among its eight neighbour positions — each position
//! mapped through the world's topology, so on small tori the same cell can be
//! counted more than once, exactly like a ghost ring — and apply the rule.

use std::collections::BTreeSet;

use patterns::LifeRule;

use crate::geometry::World;

/// One generation of `rule` on `world`, from and to sets of live cells (in
/// world coordinates; on finite worlds every cell must lie inside).
pub fn reference_step(world: &World, rule: LifeRule, live: &BTreeSet<(i64, i64)>) -> BTreeSet<(i64, i64)> {
    let count = |x: i64, y: i64| -> u8 {
        let mut n = 0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx, dy) != (0, 0) && world.map(x + dx, y + dy).is_some_and(|c| live.contains(&c)) {
                    n += 1;
                }
            }
        }
        n
    };
    let mut next = BTreeSet::new();
    if let Some(bounds) = world.bounds() {
        for y in bounds.y0..=bounds.y1 {
            for x in bounds.x0..=bounds.x1 {
                if rule.next_state(live.contains(&(x, y)), count(x, y)) {
                    next.insert((x, y));
                }
            }
        }
    } else {
        // Without B0 only live cells and their neighbours can be alive next.
        let mut candidates = BTreeSet::new();
        for &(x, y) in live {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    candidates.insert((x + dx, y + dy));
                }
            }
        }
        for (x, y) in candidates {
            if rule.next_state(live.contains(&(x, y)), count(x, y)) {
                next.insert((x, y));
            }
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::Topology;

    #[test]
    fn blinker_and_b0_on_a_small_torus() {
        let world = World::new(Topology::Infinite, 1, 1).unwrap();
        let blinker: BTreeSet<_> = [(0, 0), (1, 0), (2, 0)].into_iter().collect();
        let next = reference_step(&world, LifeRule::conway(), &blinker);
        assert_eq!(next, [(1, -1), (1, 0), (1, 1)].into_iter().collect());
        // B0 on a 2×2 torus: an empty world comes alive everywhere.
        let torus = World::new(Topology::Torus, 2, 2).unwrap();
        let all = reference_step(&torus, LifeRule::parse("B0/S").unwrap(), &BTreeSet::new());
        assert_eq!(all.len(), 4);
    }
}
