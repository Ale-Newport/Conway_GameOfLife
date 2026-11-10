//! `HashlifeEngine`: the [`LifeEngine`] adapter over `hashlife::Universe`.

use std::ops::Range;

use hashlife::{MAX_STEP_EXPONENT, Rule, Universe};
use patterns::LifeRule;
use simulation_core::Topology;

use super::{EngineKind, GenStats, LifeEngine};
use crate::error::CaError;
use crate::geometry::{CellRect, World};
use crate::view::TexelGrid;

/// Hashlife on the infinite plane.
///
/// [`LifeEngine::advance`] by `2^k` generations is one memoised
/// `step_pow2(k)` (other counts are decomposed into powers of two by the
/// universe); [`LifeEngine::step`] is `step_pow2(0)`. Births and deaths are
/// not counted (the engine skips intermediate generations), so `step`
/// returns `None`. The `hashlife` crate keeps the population cached per
/// quadtree node, so [`LifeEngine::population`] is constant time.
///
/// Views and rectangle counts never list cells: texels whose block lies
/// inside the requested rectangle add cached node populations
/// (`Universe::density_grid`), and every other count — the border texels of
/// unaligned views, whose blocks are only partly inside, and
/// [`LifeEngine::count_in_rect`] — is `Universe::population_in_rect`, exact
/// for any population at a cost bounded by the quadtree nodes crossing the
/// rectangle's border.
#[derive(Clone, Debug)]
pub struct HashlifeEngine {
    universe: Universe,
    rule: LifeRule,
}

fn to_hashlife(rule: LifeRule) -> Result<Rule, CaError> {
    EngineKind::Hashlife.check(&HashlifeEngine::plane(), rule)?;
    Ok(Rule::new(rule.birth, rule.survive)?)
}

impl HashlifeEngine {
    /// Empty engine.
    ///
    /// # Errors
    /// Rules with `B0`.
    pub fn new(rule: LifeRule) -> Result<Self, CaError> {
        Ok(Self { universe: Universe::with_rule(to_hashlife(rule)?), rule })
    }

    fn plane() -> World {
        World { topology: Topology::Infinite, width: 256, height: 256 }
    }

    /// The underlying universe (read-only).
    pub fn universe(&self) -> &Universe {
        &self.universe
    }

    /// Mutable access to the underlying universe, e.g. to load a macrocell
    /// file. Callers must keep its rule equal to [`LifeEngine::rule`].
    pub fn universe_mut(&mut self) -> &mut Universe {
        &mut self.universe
    }

    /// Replaces the universe (its rule becomes the engine's rule).
    ///
    /// # Errors
    /// Never for universes built by the `hashlife` crate (their rules have no `B0`).
    pub fn set_universe(&mut self, universe: Universe) -> Result<(), CaError> {
        let rule = universe.rule();
        self.rule = LifeRule::new(rule.birth(), rule.survive())?;
        self.universe = universe;
        Ok(())
    }
}

impl LifeEngine for HashlifeEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Hashlife
    }

    fn world(&self) -> World {
        Self::plane()
    }

    fn rule(&self) -> LifeRule {
        self.rule
    }

    fn set_rule(&mut self, rule: LifeRule) -> Result<(), CaError> {
        self.universe.set_rule(to_hashlife(rule)?);
        self.rule = rule;
        Ok(())
    }

    fn get_cell(&self, x: i64, y: i64) -> bool {
        self.universe.get_cell(x, y)
    }

    fn set_cell(&mut self, x: i64, y: i64, alive: bool) {
        self.universe.set_cell(x, y, alive);
    }

    fn set_cells(&mut self, cells: &[(i64, i64)], alive: bool) {
        if alive {
            self.universe.insert_cells(cells.iter().copied());
        } else {
            for &(x, y) in cells {
                self.universe.set_cell(x, y, false);
            }
        }
    }

    fn clear(&mut self) {
        self.universe.clear();
    }

    fn clear_rect(&mut self, rect: CellRect) {
        self.universe.clear_rect(rect.x0, rect.y0, rect.x1, rect.y1);
    }

    fn export_cells(&self) -> Vec<(i64, i64)> {
        let mut cells: Vec<(i64, i64)> = self.universe.cells().collect();
        cells.sort_unstable_by_key(|&(x, y)| (y, x));
        cells
    }

    fn cells_in_rect(&self, rect: CellRect) -> Vec<(i64, i64)> {
        self.universe.cells_in_rect(rect.x0, rect.y0, rect.x1, rect.y1, usize::MAX)
    }

    fn count_in_rect(&self, rect: CellRect, limit: u64) -> u64 {
        let count = self.universe.population_in_rect(rect.x0, rect.y0, rect.x1, rect.y1);
        u64::try_from(count).map_or(limit, |count| count.min(limit))
    }

    fn step(&mut self) -> Result<Option<GenStats>, CaError> {
        self.universe.step_pow2(0)?;
        Ok(None)
    }

    /// One memoised `step_pow2(k)` when `n = 2^k` with `k ≤ MAX_STEP_EXPONENT`
    /// (atomic: unchanged on error); otherwise the universe's decomposition
    /// into powers of two, which keeps the steps already taken when a later
    /// one fails.
    fn advance(&mut self, n: u64) -> Result<(), CaError> {
        if n.is_power_of_two() && n.trailing_zeros() <= MAX_STEP_EXPONENT {
            self.universe.step_pow2(n.trailing_zeros())?;
        } else {
            self.universe.advance(n)?;
        }
        Ok(())
    }

    fn population(&self) -> u64 {
        self.universe.population()
    }

    fn bounding_box(&self) -> Option<CellRect> {
        self.universe.bounding_box().map(|(x0, y0, x1, y1)| CellRect { x0, y0, x1, y1 })
    }

    fn count_live(&self, grid: &TexelGrid, out: &mut [u32]) {
        let Some(clip) = grid.clip else {
            return;
        };
        let k = 1i128 << grid.log2_k;
        // Texels whose whole block lies inside the clip, along one axis.
        let full = |origin: i64, lo: i64, hi: i64, len: u32| -> Range<usize> {
            let first = (i128::from(lo) - i128::from(origin) + k - 1).div_euclid(k).min(i128::from(len));
            let end = (i128::from(hi) + 1 - i128::from(origin)).div_euclid(k).clamp(first, i128::from(len));
            first as usize..end as usize
        };
        let cols = full(grid.origin_x, clip.x0, clip.x1, grid.width);
        let rows = full(grid.origin_y, clip.y0, clip.y1, grid.height);
        let width = grid.width as usize;
        let block_start = |origin: i64, t: usize| i128::from(origin) + t as i128 * k;
        if !cols.is_empty() && !rows.is_empty() {
            // Both corners lie in the clip, so in the i64 plane; the interior is
            // at most the view budget, far below the density-grid limit.
            let (x0, y0) =
                (block_start(grid.origin_x, cols.start) as i64, block_start(grid.origin_y, rows.start) as i64);
            if let Ok(counts) = self.universe.density_grid(x0, y0, cols.len(), rows.len(), grid.log2_k) {
                for (r, row) in rows.clone().zip(counts.chunks(cols.len())) {
                    for (o, &c) in out[r * width + cols.start..r * width + cols.end].iter_mut().zip(row) {
                        *o = o.saturating_add(c);
                    }
                }
            }
        }
        // Border texels: the part of their block inside the clip.
        for r in 0..grid.height as usize {
            let skip = if rows.contains(&r) { cols.clone() } else { 0..0 };
            let y = block_start(grid.origin_y, r);
            for c in (0..skip.start).chain(skip.end..width) {
                let x = block_start(grid.origin_x, c);
                let (x0, x1) = (x.max(clip.x0.into()), (x + k - 1).min(clip.x1.into()));
                let (y0, y1) = (y.max(clip.y0.into()), (y + k - 1).min(clip.y1.into()));
                if x0 <= x1 && y0 <= y1 {
                    // A non-empty overlap with the clip lies in the i64 plane.
                    let count = self.universe.population_in_rect(x0 as i64, y0 as i64, x1 as i64, y1 as i64);
                    let slot = &mut out[r * width + c];
                    *slot = slot.saturating_add(u32::try_from(count).unwrap_or(u32::MAX));
                }
            }
        }
    }

    fn memory_bytes(&self) -> usize {
        self.universe.memory_bytes()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::GridViewRequest;

    fn bin(cells: &[(i64, i64)], grid: &TexelGrid, out: &mut [u32]) {
        for &(x, y) in cells {
            out[grid.index(x, y)] += 1;
        }
    }

    /// An engine holding a full `2^level × 2^level` square, built from a
    /// macrocell file (one node per level). Golly's origin puts it at
    /// `[-2^(level-1), 2^(level-1)) × [-2^(level-1) + 1, 2^(level-1) + 1)`.
    fn full_square(level: u32) -> HashlifeEngine {
        let mut text = String::from("[M2]\n");
        text.push_str(&"********$".repeat(8));
        for level in 4..=level {
            text.push_str(&format!("\n{level} {i} {i} {i} {i}", i = level - 3));
        }
        let mut e = HashlifeEngine::new(LifeRule::conway()).unwrap();
        e.set_universe(hashlife::macrocell::read(&text).unwrap()).unwrap();
        e
    }

    #[test]
    fn rejects_b0_and_runs_big_steps() {
        assert!(HashlifeEngine::new(LifeRule::parse("B013/S23").unwrap()).is_err());
        let mut e = HashlifeEngine::new(LifeRule::conway()).unwrap();
        e.set_cells(&[(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)], true);
        e.advance(1 << 12).unwrap();
        assert_eq!(e.bounding_box(), Some(CellRect::new(1024, 1024, 1026, 1026)));
        assert_eq!(e.population(), 5);
    }

    #[test]
    fn lod_counts_with_unaligned_borders_match_direct_counts() {
        let mut e = HashlifeEngine::new(LifeRule::conway()).unwrap();
        let cells: Vec<(i64, i64)> = (0..200).flat_map(|i| [(i * 7 % 101 - 50, i * 13 % 97 - 40), (i, -i)]).collect();
        e.set_cells(&cells, true);
        let req = GridViewRequest { x0: -45, y0: -37, x1: 61, y1: 29, max_width: 9, max_height: 9 };
        let grid = TexelGrid::plan(&req, None);
        assert!(grid.log2_k >= 3);
        let mut fast = vec![0u32; grid.len()];
        e.count_live(&grid, &mut fast);
        let mut slow = vec![0u32; grid.len()];
        bin(&e.cells_in_rect(grid.clip.unwrap()), &grid, &mut slow);
        assert_eq!(fast, slow);
    }

    #[test]
    fn border_texels_are_exact_for_any_population() {
        // A full 2^14 × 2^14 square (2^28 live cells) and a view inside it at
        // 2^10 cells per texel: its unaligned border strips hold millions of
        // live cells, and the top row of blocks has one empty row (the square
        // starts one row below a block boundary), so a texel estimated from
        // its block's population would be wrong there.
        let e = full_square(14);
        let s = e.bounding_box().unwrap();
        assert_eq!((s.x0, s.y0, s.width(), s.height()), (-(1 << 13), 1 - (1 << 13), 1 << 14, 1 << 14));
        let req = GridViewRequest {
            x0: s.x0 + 300,
            y0: s.y0 + 191,
            x1: s.x1 - 77,
            y1: s.y1 - 500,
            max_width: 16,
            max_height: 16,
        };
        let grid = TexelGrid::plan(&req, None);
        assert_eq!(grid.log2_k, 10);
        let mut got = vec![0u32; grid.len()];
        e.count_live(&grid, &mut got);
        // Every requested cell is alive: each texel counts its covered cells.
        let covered: Vec<u32> = grid.coverage().into_iter().map(|c| c as u32).collect();
        assert_eq!(got, covered);
    }

    #[test]
    fn advances_beyond_the_largest_single_step() {
        let mut e = HashlifeEngine::new(LifeRule::conway()).unwrap();
        let block = [(0, 0), (1, 0), (0, 1), (1, 1)];
        e.set_cells(&block, true);
        e.advance(1 << 62).unwrap();
        assert_eq!(e.export_cells(), block.to_vec(), "2^62 generations of a still life");
        assert_eq!(e.universe().generation(), 1 << 62);
    }

    #[test]
    fn rectangle_counts_are_exact_up_to_the_limit() {
        let mut e = HashlifeEngine::new(LifeRule::conway()).unwrap();
        let mut cells: Vec<(i64, i64)> = (0..3000).map(|i| (i * 37 % 211 - 90, i * 53 % 173 - 70)).collect();
        cells.extend([(i64::MIN, i64::MIN), (i64::MAX, i64::MAX), (i64::MIN, 5), (1 << 40, -(1 << 40))]);
        e.set_cells(&cells, true);
        let plane = CellRect::new(i64::MIN, i64::MIN, i64::MAX, i64::MAX);
        let rects = [
            plane,
            CellRect::new(-90, -70, 120, 102),
            CellRect::new(-37, -11, 58, 13),
            CellRect::new(3, 3, 3, 90),
            CellRect::new(i64::MIN, -100, 0, i64::MAX),
            CellRect::new(-(1 << 41), -(1 << 41), 1 << 41, 0),
        ];
        for rect in rects {
            let listed = e.cells_in_rect(rect).len() as u64;
            assert_eq!(e.count_in_rect(rect, u64::MAX), listed, "{rect:?}");
            for limit in [0, 1, listed / 2] {
                assert_eq!(e.count_in_rect(rect, limit), limit.min(listed), "{rect:?}, limit {limit}");
            }
        }
        assert_eq!(e.count_in_rect(plane, u64::MAX), e.population());
        // A full 2^33 × 2^33 square: 2^66 live cells, more than a u64 holds.
        // Its blocks hold more than u32::MAX cells and every border runs
        // through 2^30 leaves, so only cached populations and the per-node
        // memo of the universe keep this instantaneous.
        let e = full_square(33);
        let square = e.bounding_box().unwrap();
        assert_eq!((square.width(), square.height()), (1 << 33, 1 << 33));
        assert_eq!(e.count_in_rect(plane, u64::MAX), u64::MAX, "2^66 cells saturate the count");
        let part = CellRect::new(square.x0 + 3, square.y0 - 9, square.x1 - 7, square.y0 + (1 << 30) + 4);
        assert_eq!(e.count_in_rect(part, u64::MAX), ((1 << 33) - 10) * ((1 << 30) + 5));
        assert_eq!(e.count_in_rect(part, 1000), 1000);
    }
}
