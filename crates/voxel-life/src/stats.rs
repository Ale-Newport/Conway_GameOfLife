//! Population statistics gathered while stepping or scanning a world.

use crate::geometry::{Bounds, Pos};

/// Populations per state, last-generation transitions, bounding box and
/// centroid of the non-empty voxels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stats {
    counts: Vec<u64>,
    births: u64,
    deaths: u64,
    min: Pos,
    max: Pos,
    sum: [i128; 3],
}

impl Stats {
    /// Empty statistics for a rule with `states` states.
    pub(crate) fn new(states: u16) -> Self {
        Self {
            counts: vec![0; usize::from(states)],
            births: 0,
            deaths: 0,
            min: [i32::MAX; 3],
            max: [i32::MIN; 3],
            sum: [0; 3],
        }
    }

    /// Records a non-empty voxel.
    #[inline]
    pub(crate) fn add(&mut self, pos: Pos, state: u8) {
        self.counts[usize::from(state)] += 1;
        for (a, &c) in pos.iter().enumerate() {
            self.min[a] = self.min[a].min(c);
            self.max[a] = self.max[a].max(c);
            self.sum[a] += i128::from(c);
        }
    }

    /// Records one generation transition of a voxel.
    #[inline]
    pub(crate) fn transition(&mut self, old: u8, new: u8) {
        self.births += u64::from(old == 0 && new == 1);
        self.deaths += u64::from(old == 1 && new != 1);
    }

    /// Adds the statistics of a disjoint part of the world.
    pub(crate) fn merge(&mut self, other: &Stats) {
        for (a, b) in self.counts.iter_mut().zip(&other.counts) {
            *a += b;
        }
        self.births += other.births;
        self.deaths += other.deaths;
        for a in 0..3 {
            self.min[a] = self.min[a].min(other.min[a]);
            self.max[a] = self.max[a].max(other.max[a]);
            self.sum[a] += other.sum[a];
        }
    }

    /// Sum of disjoint parts, combined in the given (deterministic) order.
    pub(crate) fn combine(states: u16, parts: impl IntoIterator<Item = Stats>) -> Stats {
        parts.into_iter().fold(Stats::new(states), |mut acc, part| {
            acc.merge(&part);
            acc
        })
    }

    /// Replaces the transition counters (used after edits and restores).
    pub(crate) fn with_transitions(mut self, births: u64, deaths: u64) -> Self {
        self.births = births;
        self.deaths = deaths;
        self
    }

    /// Number of voxels per state, indexed by state. Index 0 is always 0:
    /// empty voxels are not counted here (see [`crate::CountsView`]).
    pub fn counts(&self) -> &[u64] {
        &self.counts
    }

    /// Number of non-empty voxels (all states except 0).
    pub fn population(&self) -> u64 {
        self.counts.iter().skip(1).sum()
    }

    /// Number of voxels in state 1 (alive).
    pub fn alive(&self) -> u64 {
        self.counts.get(1).copied().unwrap_or(0)
    }

    /// Empty voxels that became alive during the last generation.
    pub fn births(&self) -> u64 {
        self.births
    }

    /// Alive voxels that did not survive the last generation (they became
    /// empty, or entered the first dying state of a multi-state rule).
    pub fn deaths(&self) -> u64 {
        self.deaths
    }

    /// Smallest box containing every non-empty voxel.
    pub fn bounds(&self) -> Option<Bounds> {
        (self.population() > 0).then_some(Bounds { min: self.min, max: self.max })
    }

    /// Mean coordinate of the non-empty voxels (arithmetic mean, also on a
    /// torus).
    pub fn centroid(&self) -> Option<[f64; 3]> {
        let n = self.population();
        (n > 0).then(|| self.sum.map(|s| s as f64 / n as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_and_merges() {
        let mut a = Stats::new(3);
        a.add([0, 0, 0], 1);
        a.add([2, 4, 6], 2);
        a.transition(0, 1);
        let mut b = Stats::new(3);
        b.add([-2, 0, 0], 1);
        b.transition(1, 2);
        let total = Stats::combine(3, [a, b]);
        assert_eq!(total.counts(), &[0, 2, 1]);
        assert_eq!(total.population(), 3);
        assert_eq!(total.alive(), 2);
        assert_eq!((total.births(), total.deaths()), (1, 1));
        assert_eq!(total.bounds(), Some(Bounds { min: [-2, 0, 0], max: [2, 4, 6] }));
        assert_eq!(total.centroid(), Some([0.0, 4.0 / 3.0, 2.0]));
        assert_eq!(Stats::new(2).bounds(), None);
        assert_eq!(Stats::new(2).centroid(), None);
    }
}
