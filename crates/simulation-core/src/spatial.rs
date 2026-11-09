//! Uniform-grid spatial index for neighbour queries among agents.
//!
//! [`SpatialGrid`] is rebuilt once per tick from agent positions with a
//! counting sort (O(n)), then answers radius queries by visiting only the
//! buckets overlapping the query circle. Iteration order is fully determined
//! by the insertion order and bucket layout, so simulations stay deterministic.

use glam::Vec2;

/// Bucketed spatial index over a rectangular world `[0, width) × [0, height)`.
/// Positions outside the world are clamped into the border buckets.
#[derive(Clone, Debug)]
pub struct SpatialGrid {
    cell_size: f32,
    cols: usize,
    rows: usize,
    /// Prefix offsets into `entries` per bucket (len = cols * rows + 1).
    starts: Vec<u32>,
    /// `(id, position)` sorted by bucket, insertion order within a bucket.
    entries: Vec<(u32, Vec2)>,
    scratch_bucket: Vec<u32>,
}

impl SpatialGrid {
    /// Index for a world of the given size with square buckets of `cell_size`.
    ///
    /// # Panics
    /// Panics if `cell_size` is not positive.
    pub fn new(width: f32, height: f32, cell_size: f32) -> Self {
        assert!(cell_size > 0.0, "cell size must be positive");
        let cols = ((width / cell_size).ceil() as usize).max(1);
        let rows = ((height / cell_size).ceil() as usize).max(1);
        Self {
            cell_size,
            cols,
            rows,
            starts: vec![0; cols * rows + 1],
            entries: Vec::new(),
            scratch_bucket: Vec::new(),
        }
    }

    /// Bucket size.
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// Number of indexed items.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    fn bucket_coords(&self, p: Vec2) -> (usize, usize) {
        let cx = ((p.x / self.cell_size).floor().max(0.0) as usize).min(self.cols - 1);
        let cy = ((p.y / self.cell_size).floor().max(0.0) as usize).min(self.rows - 1);
        (cx, cy)
    }

    /// Rebuilds the index from `(id, position)` pairs.
    pub fn rebuild(&mut self, items: impl IntoIterator<Item = (u32, Vec2)>) {
        self.entries.clear();
        self.scratch_bucket.clear();
        let buckets = self.cols * self.rows;
        let mut counts = vec![0u32; buckets];
        let mut staged: Vec<(u32, Vec2)> = Vec::new();
        for (id, p) in items {
            let (cx, cy) = self.bucket_coords(p);
            let b = cy * self.cols + cx;
            counts[b] += 1;
            self.scratch_bucket.push(b as u32);
            staged.push((id, p));
        }
        self.starts.clear();
        self.starts.reserve(buckets + 1);
        let mut acc = 0u32;
        for c in &counts {
            self.starts.push(acc);
            acc += c;
        }
        self.starts.push(acc);
        let mut cursor: Vec<u32> = self.starts[..buckets].to_vec();
        self.entries.resize(staged.len(), (0, Vec2::ZERO));
        for (item, &b) in staged.into_iter().zip(&self.scratch_bucket) {
            let slot = &mut cursor[b as usize];
            self.entries[*slot as usize] = item;
            *slot += 1;
        }
    }

    /// Calls `f(id, position, distance_squared)` for every item within
    /// `radius` of `center` (buckets visited row by row).
    pub fn for_each_within(&self, center: Vec2, radius: f32, mut f: impl FnMut(u32, Vec2, f32)) {
        if self.entries.is_empty() || radius < 0.0 {
            return;
        }
        let r2 = radius * radius;
        let (x0, y0) = self.bucket_coords(center - Vec2::splat(radius));
        let (x1, y1) = self.bucket_coords(center + Vec2::splat(radius));
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let b = cy * self.cols + cx;
                let (s, e) = (self.starts[b] as usize, self.starts[b + 1] as usize);
                for &(id, p) in &self.entries[s..e] {
                    let d2 = p.distance_squared(center);
                    if d2 <= r2 {
                        f(id, p, d2);
                    }
                }
            }
        }
    }

    /// Collects ids within `radius` of `center` into `out` (cleared first).
    pub fn query(&self, center: Vec2, radius: f32, out: &mut Vec<u32>) {
        out.clear();
        self.for_each_within(center, radius, |id, _, _| out.push(id));
    }

    /// Nearest item within `radius` accepted by `filter`; ties are broken by
    /// the lower id so the answer is deterministic.
    pub fn nearest(&self, center: Vec2, radius: f32, mut filter: impl FnMut(u32) -> bool) -> Option<(u32, Vec2, f32)> {
        let mut best: Option<(u32, Vec2, f32)> = None;
        self.for_each_within(center, radius, |id, p, d2| {
            let better = match best {
                None => true,
                Some((bid, _, bd2)) => d2 < bd2 || (d2 == bd2 && id < bid),
            };
            if better && filter(id) {
                best = Some((id, p, d2));
            }
        });
        best
    }

    /// Number of items within `radius` accepted by `filter`.
    pub fn count_within(&self, center: Vec2, radius: f32, mut filter: impl FnMut(u32) -> bool) -> usize {
        let mut n = 0;
        self.for_each_within(center, radius, |id, _, _| {
            if filter(id) {
                n += 1;
            }
        });
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SimRng;

    #[test]
    fn matches_brute_force() {
        let mut rng = SimRng::seed_from_u64(1);
        let points: Vec<(u32, Vec2)> =
            (0..2000).map(|i| (i, Vec2::new(rng.range_f32(0.0, 100.0), rng.range_f32(0.0, 60.0)))).collect();
        let mut grid = SpatialGrid::new(100.0, 60.0, 7.0);
        grid.rebuild(points.iter().copied());
        let mut out = Vec::new();
        for _ in 0..200 {
            let c = Vec2::new(rng.range_f32(-5.0, 105.0), rng.range_f32(-5.0, 65.0));
            let r = rng.range_f32(0.0, 20.0);
            grid.query(c, r, &mut out);
            out.sort_unstable();
            let mut expected: Vec<u32> =
                points.iter().filter(|(_, p)| p.distance_squared(c) <= r * r).map(|(i, _)| *i).collect();
            expected.sort_unstable();
            assert_eq!(out, expected);
        }
    }

    #[test]
    fn nearest_breaks_ties_by_id() {
        let mut grid = SpatialGrid::new(10.0, 10.0, 2.0);
        grid.rebuild([(5, Vec2::new(4.0, 5.0)), (2, Vec2::new(6.0, 5.0)), (9, Vec2::new(9.0, 9.0))]);
        let (id, _, _) = grid.nearest(Vec2::new(5.0, 5.0), 3.0, |_| true).unwrap();
        assert_eq!(id, 2);
        let (id, _, _) = grid.nearest(Vec2::new(5.0, 5.0), 3.0, |i| i != 2).unwrap();
        assert_eq!(id, 5);
        assert_eq!(grid.count_within(Vec2::new(5.0, 5.0), 10.0, |_| true), 3);
    }
}
