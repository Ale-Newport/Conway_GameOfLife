//! `HashlifeEngine`: the [`LifeEngine`] adapter over `hashlife::Universe`.

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
/// Views use cached node populations for texels whose block lies inside the
/// requested rectangle; texels on its border, whose blocks are only partly
/// inside, are counted from the live cells of the border strips alone (no
/// cell outside the request is read) as long as the strips hold at most
/// [`BORDER_CELL_BUDGET`] live cells. Beyond that budget — only reachable by
/// unaligned views cutting through enormous populations — a border texel is
/// estimated as its whole block's cached population times the fraction of
/// the block inside the request, so a view never enumerates more cells.
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

    /// Adds the live cells of `rect` inside the aligned square of side
    /// `2^level` at `(x, y)`, which holds `population` live cells (a cached
    /// quadtree population, saturated at `u32::MAX` by
    /// `Universe::density_grid`), to `count`, stopping once it reaches
    /// `limit`. A square inside `rect` adds its population; a square crossing
    /// the border lists its overlap when it holds at most
    /// [`LIST_UP_TO`] cells (every 8×8 leaf does) and otherwise descends into
    /// its four children, whose populations one 2×2 density grid gives. So
    /// only non-empty squares crossing the border are visited, plus, below
    /// `limit`, the squares inside `rect` holding more than `u32::MAX` cells.
    fn count_square(
        &self,
        rect: &CellRect,
        (x, y): (i128, i128),
        level: u32,
        population: u32,
        limit: u64,
        count: &mut u64,
    ) {
        let last = (1i128 << level) - 1;
        let (x0, y0) = (x.max(rect.x0.into()), y.max(rect.y0.into()));
        let (x1, y1) = ((x + last).min(rect.x1.into()), (y + last).min(rect.y1.into()));
        if population == 0 || *count >= limit || x0 > x1 || y0 > y1 {
            return;
        }
        let inside = (x0, y0, x1, y1) == (x, y, x + last, y + last);
        // A saturated population stands for at least u32::MAX cells.
        if inside && (population < u32::MAX || count.saturating_add(u64::from(u32::MAX)) >= limit) {
            *count += u64::from(population);
            return;
        }
        // The overlap lies inside the i64 plane.
        if population <= LIST_UP_TO {
            let cells = self.universe.cells_in_rect(x0 as i64, y0 as i64, x1 as i64, y1 as i64, LIST_UP_TO as usize);
            *count += cells.len() as u64;
            return;
        }
        // An 8×8 leaf holds at most LIST_UP_TO cells, so `level > 3` here and
        // a 2×2 grid of half-size squares is always a valid density grid.
        let Ok(children) = self.universe.density_grid(x as i64, y as i64, 2, 2, level - 1) else {
            return;
        };
        let half = 1i128 << (level - 1);
        for (i, &child) in children.iter().enumerate() {
            let corner = (x + half * (i % 2) as i128, y + half * (i / 2) as i128);
            self.count_square(rect, corner, level - 1, child, limit, count);
        }
    }

    fn bin(cells: &[(i64, i64)], grid: &TexelGrid, out: &mut [u32]) {
        for &(x, y) in cells {
            out[grid.index(x, y)] += 1;
        }
    }

    /// [`LifeEngine::count_live`] with an explicit budget of live cells the
    /// partly covered border strips may enumerate (see
    /// [`BORDER_CELL_BUDGET`]).
    fn count_live_with_budget(&self, grid: &TexelGrid, out: &mut [u32], mut budget: usize) {
        let Some(clip) = grid.clip else {
            return;
        };
        let k = 1i128 << grid.log2_k;
        // Full texel columns: blocks [ox + i·k, ox + (i+1)·k) inside the clip.
        let full = |origin: i64, lo: i64, hi: i64, len: u32| -> (i128, i128) {
            let first = (i128::from(lo) - i128::from(origin) + k - 1).div_euclid(k);
            let last = ((i128::from(hi) + 1 - i128::from(origin)).div_euclid(k) - 1).min(i128::from(len) - 1);
            (first, last)
        };
        let (i0, i1) = full(grid.origin_x, clip.x0, clip.x1, grid.width);
        let (j0, j1) = full(grid.origin_y, clip.y0, clip.y1, grid.height);
        if i0 > i1 || j0 > j1 {
            self.count_partial(clip, grid, out, &mut budget);
            return;
        }
        let (cols, rows) = ((i1 - i0 + 1) as usize, (j1 - j0 + 1) as usize);
        let x0 = (i128::from(grid.origin_x) + i0 * k) as i64;
        let y0 = (i128::from(grid.origin_y) + j0 * k) as i64;
        // The interior is at most the view budget, well under the density limit.
        if let Ok(counts) = self.universe.density_grid(x0, y0, cols, rows, grid.log2_k) {
            let width = grid.width as usize;
            for r in 0..rows {
                let base = (j0 as usize + r) * width + i0 as usize;
                for (o, &c) in out[base..base + cols].iter_mut().zip(&counts[r * cols..(r + 1) * cols]) {
                    *o = o.saturating_add(c);
                }
            }
        }
        // Border strips: the parts of the clip outside the full texels.
        let inner_x0 = x0;
        let inner_x1 = (i128::from(x0) + cols as i128 * k - 1) as i64;
        let inner_y0 = y0;
        let inner_y1 = (i128::from(y0) + rows as i128 * k - 1) as i64;
        let mut strips = Vec::new();
        if clip.x0 < inner_x0 {
            strips.push(CellRect { x0: clip.x0, y0: clip.y0, x1: inner_x0 - 1, y1: clip.y1 });
        }
        if clip.x1 > inner_x1 {
            strips.push(CellRect { x0: inner_x1 + 1, y0: clip.y0, x1: clip.x1, y1: clip.y1 });
        }
        if clip.y0 < inner_y0 {
            strips.push(CellRect { x0: inner_x0, y0: clip.y0, x1: inner_x1, y1: inner_y0 - 1 });
        }
        if clip.y1 > inner_y1 {
            strips.push(CellRect { x0: inner_x0, y0: inner_y1 + 1, x1: inner_x1, y1: clip.y1 });
        }
        for strip in strips {
            self.count_partial(strip, grid, out, &mut budget);
        }
    }

    /// Counts the live cells of `rect` (inside the clip, covering only partly
    /// covered texels) exactly by enumerating them while the budget lasts;
    /// beyond it, estimates every texel the rectangle touches as its whole
    /// aligned block's cached population times the covered fraction of the
    /// block.
    fn count_partial(&self, rect: CellRect, grid: &TexelGrid, out: &mut [u32], budget: &mut usize) {
        let cells = self.universe.cells_in_rect(rect.x0, rect.y0, rect.x1, rect.y1, budget.saturating_add(1));
        if cells.len() <= *budget {
            *budget -= cells.len();
            Self::bin(&cells, grid, out);
            return;
        }
        *budget = 0;
        let (c0, c1) = (grid.column(rect.x0), grid.column(rect.x1));
        let (r0, r1) = (grid.row(rect.y0), grid.row(rect.y1));
        let (cols, rows) = (c1 - c0 + 1, r1 - r0 + 1);
        let k = 1i128 << grid.log2_k;
        let bx = (i128::from(grid.origin_x) + c0 as i128 * k) as i64;
        let by = (i128::from(grid.origin_y) + r0 as i128 * k) as i64;
        // One texel row or column (two texels thick at most when the clip has
        // no full texel): far below the density-grid limit.
        let Ok(blocks) = self.universe.density_grid(bx, by, cols, rows, grid.log2_k) else {
            return;
        };
        let span = |origin: i64, t: usize, lo: i64, hi: i64| -> u128 {
            let start = i128::from(origin) + t as i128 * k;
            let (a, b) = (start.max(i128::from(lo)), (start + k - 1).min(i128::from(hi)));
            if a > b { 0 } else { (b - a + 1) as u128 }
        };
        let area = (k * k) as u128;
        let width = grid.width as usize;
        for r in 0..rows {
            let h = span(grid.origin_y, r0 + r, rect.y0, rect.y1);
            for c in 0..cols {
                let covered = span(grid.origin_x, c0 + c, rect.x0, rect.x1) * h;
                let estimate = (u128::from(blocks[r * cols + c]) * covered + area / 2) / area;
                let slot = &mut out[(r0 + r) * width + c0 + c];
                *slot = slot.saturating_add(u32::try_from(estimate).unwrap_or(u32::MAX));
            }
        }
    }
}

/// Live cells the partly covered border texels of one view may enumerate.
/// Texels fully inside a request are counted exactly from cached quadtree
/// populations; border texels (only when the request is not aligned to the
/// texel grid) are counted exactly by enumerating their cells up to this
/// budget, and estimated from their whole aligned block's cached population
/// beyond it, so a view never enumerates more than this many cells.
pub const BORDER_CELL_BUDGET: usize = 1 << 21;

/// Squares crossing the border of a counted rectangle that hold at most
/// this many live cells (every 8×8 quadtree leaf does) are counted by listing
/// their overlap with the rectangle (see [`HashlifeEngine::count_square`]).
const LIST_UP_TO: u32 = 64;

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
        // The whole plane is the aligned square of side 2^64 at (i64::MIN, i64::MIN).
        let corner = i64::MIN;
        let Ok(plane) = self.universe.density_grid(corner, corner, 1, 1, 64) else {
            return 0;
        };
        let mut count = 0;
        self.count_square(&rect, (corner.into(), corner.into()), 64, plane[0], limit, &mut count);
        count.min(limit)
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
        self.count_live_with_budget(grid, out, BORDER_CELL_BUDGET);
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
        HashlifeEngine::bin(&e.cells_in_rect(grid.clip.unwrap()), &grid, &mut slow);
        assert_eq!(fast, slow);
    }

    #[test]
    fn border_texels_beyond_the_budget_are_estimated_from_block_populations() {
        // A full 200×200 square: every block of the view below lies inside it,
        // so the proportional estimate of a border texel is exact.
        let mut e = HashlifeEngine::new(LifeRule::conway()).unwrap();
        let square: Vec<(i64, i64)> = (-100..100).flat_map(|y| (-100..100).map(move |x| (x, y))).collect();
        e.set_cells(&square, true);
        let req = GridViewRequest { x0: -45, y0: -37, x1: 61, y1: 29, max_width: 9, max_height: 9 };
        let grid = TexelGrid::plan(&req, None);
        let mut exact = vec![0u32; grid.len()];
        HashlifeEngine::bin(&e.cells_in_rect(grid.clip.unwrap()), &grid, &mut exact);
        for budget in [0, 50, 1 << 20] {
            let mut got = vec![0u32; grid.len()];
            e.count_live_with_budget(&grid, &mut got, budget);
            assert_eq!(got, exact, "budget {budget}");
        }
        // A sparse pattern: the estimate stays within the covered cells.
        let mut sparse = HashlifeEngine::new(LifeRule::conway()).unwrap();
        sparse.set_cells(&[(-47, 0), (-40, 3), (60, 60)], true);
        let mut got = vec![0u32; grid.len()];
        sparse.count_live_with_budget(&grid, &mut got, 0);
        let coverage = grid.coverage();
        assert!(got.iter().zip(&coverage).all(|(&n, &c)| u64::from(n) <= c));
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
        // A full 2^17 × 2^17 square (2^34 cells, built from a macrocell file):
        // the density grid saturates its blocks of 2^32 cells and more.
        let mut text = String::from("[M2]\n");
        text.push_str(&"********$".repeat(8));
        for level in 4..=17 {
            text.push_str(&format!("\n{level} {i} {i} {i} {i}", i = level - 3));
        }
        e.set_universe(hashlife::macrocell::read(&text).unwrap()).unwrap();
        let square = e.bounding_box().unwrap();
        assert_eq!((square.width(), square.height()), (1 << 17, 1 << 17));
        assert_eq!(e.count_in_rect(plane, u64::MAX), 1 << 34, "saturated blocks are split until exact");
        let part = CellRect::new(square.x0 + 3, square.y0 - 9, square.x1, square.y0 + (1 << 16));
        assert_eq!(e.count_in_rect(part, u64::MAX), ((1 << 17) - 3) * ((1 << 16) + 1));
        assert_eq!(e.count_in_rect(part, 1000), 1000, "a saturated block reaches the limit at once");
    }
}
