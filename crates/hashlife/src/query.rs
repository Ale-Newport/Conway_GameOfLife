//! Read-only queries: live-cell iteration, exact bounding box, rectangle
//! extraction, level-of-detail density grids and exact populations (of the
//! plane and of rectangles).
//!
//! All of them descend the quadtree and skip empty nodes, so their cost is
//! proportional to the number of distinct visited nodes, not to the area.

use std::collections::HashMap;

use crate::error::HashlifeError;
use crate::store::{LEAF_LEVEL, NodeStore};
use crate::universe::{Universe, half_side};

/// Largest number of pixels [`Universe::density_grid`] produces in one call.
pub const MAX_DENSITY_PIXELS: usize = 1 << 26;

/// Inclusive integer rectangle in plane coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rect {
    x0: i128,
    y0: i128,
    x1: i128,
    y1: i128,
}

impl Rect {
    /// The rectangle `[x0, x1] × [y0, y1]` (callers ensure `x0 <= x1`, `y0 <= y1`).
    pub(crate) fn new(x0: i64, y0: i64, x1: i64, y1: i64) -> Self {
        Self { x0: x0.into(), y0: y0.into(), x1: x1.into(), y1: y1.into() }
    }

    /// Whether the square `[nx, nx + size) × [ny, ny + size)` meets the rectangle.
    pub(crate) fn intersects(&self, nx: i128, ny: i128, size: i128) -> bool {
        nx <= self.x1 && nx + size > self.x0 && ny <= self.y1 && ny + size > self.y0
    }

    /// Whether the square lies entirely inside the rectangle.
    pub(crate) fn contains_square(&self, nx: i128, ny: i128, size: i128) -> bool {
        nx >= self.x0 && ny >= self.y0 && nx + size - 1 <= self.x1 && ny + size - 1 <= self.y1
    }

    /// Bits of the 8×8 leaf at `(nx, ny)` that lie inside the rectangle.
    pub(crate) fn leaf_mask(&self, nx: i128, ny: i128) -> u64 {
        let span = |lo: i128, hi: i128| -> Option<(u32, u32)> {
            let (lo, hi) = (lo.max(0), hi.min(7));
            // Both bounds are in 0..=7 here.
            (lo <= hi).then_some((lo as u32, hi as u32))
        };
        let (Some((c0, c1)), Some((r0, r1))) = (span(self.x0 - nx, self.x1 - nx), span(self.y0 - ny, self.y1 - ny))
        else {
            return 0;
        };
        let row = u64::from(((1u16 << (c1 + 1)) - 1) & !((1u16 << c0) - 1));
        (r0..=r1).fold(0, |mask, r| mask | row << (8 * r))
    }
}

/// Which extreme row or column [`edge`] looks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

impl Side {
    fn wants_minimum(self) -> bool {
        matches!(self, Self::Top | Self::Left)
    }
}

/// Smallest (top/left) or largest (bottom/right) row or column, relative to
/// the node's top-left corner, that holds a live cell; `None` for an empty
/// node. `memo` caches per-node answers for one `side`.
pub(crate) fn edge(store: &NodeStore, n: u32, side: Side, memo: &mut HashMap<u32, u64>) -> Option<u64> {
    let node = store.node(n);
    if node.population == 0 {
        return None;
    }
    if node.level == LEAF_LEVEL {
        let bits = node.leaf_bits();
        // OR of all rows: bit x set when column x has a live cell.
        let columns = (0..8).fold(0u8, |acc, r| acc | (bits >> (8 * r)) as u8);
        return Some(u64::from(match side {
            Side::Top => bits.trailing_zeros() / 8,
            Side::Bottom => (63 - bits.leading_zeros()) / 8,
            Side::Left => columns.trailing_zeros(),
            Side::Right => 7 - columns.leading_zeros(),
        }));
    }
    if let Some(&known) = memo.get(&n) {
        return Some(known);
    }
    let half = 1u64 << (node.level - 1);
    let [nw, ne, sw, se] = node.children;
    let (near, near_offset, far, far_offset) = match side {
        Side::Top => ([nw, ne], 0, [sw, se], half),
        Side::Bottom => ([sw, se], half, [nw, ne], 0),
        Side::Left => ([nw, sw], 0, [ne, se], half),
        Side::Right => ([ne, se], half, [nw, sw], 0),
    };
    let value =
        extreme(store, near, near_offset, side, memo).or_else(|| extreme(store, far, far_offset, side, memo))?;
    memo.insert(n, value);
    Some(value)
}

fn extreme(store: &NodeStore, pair: [u32; 2], offset: u64, side: Side, memo: &mut HashMap<u32, u64>) -> Option<u64> {
    let a = edge(store, pair[0], side, memo).map(|v| v + offset);
    let b = edge(store, pair[1], side, memo).map(|v| v + offset);
    match (a, b) {
        (Some(a), Some(b)) => Some(if side.wants_minimum() { a.min(b) } else { a.max(b) }),
        (a, b) => a.or(b),
    }
}

/// Iterator over live cells in quadtree order (north-west quadrant first,
/// row-major inside each 8×8 leaf). Created by [`Universe::cells`].
#[derive(Clone, Debug)]
pub struct Cells<'a> {
    store: &'a NodeStore,
    stack: Vec<(u32, i64, i64)>,
    leaf: u64,
    leaf_x: i64,
    leaf_y: i64,
}

impl Iterator for Cells<'_> {
    type Item = (i64, i64);

    fn next(&mut self) -> Option<(i64, i64)> {
        loop {
            if self.leaf != 0 {
                let bit = self.leaf.trailing_zeros();
                self.leaf &= self.leaf - 1;
                return Some((self.leaf_x + i64::from(bit % 8), self.leaf_y + i64::from(bit / 8)));
            }
            let (n, x, y) = self.stack.pop()?;
            let node = self.store.node(n);
            if node.population == 0 {
                continue;
            }
            if node.level == LEAF_LEVEL {
                (self.leaf, self.leaf_x, self.leaf_y) = (node.leaf_bits(), x, y);
                continue;
            }
            // Child corners stay inside the i64 plane, so the wrapping sums
            // (needed for the 2^63 half-side of a level-64 root) are exact.
            let half = 1u64 << (node.level - 1);
            let (xh, yh) = (x.wrapping_add_unsigned(half), y.wrapping_add_unsigned(half));
            let [nw, ne, sw, se] = node.children;
            self.stack.extend([(se, xh, yh), (sw, x, yh), (ne, xh, y), (nw, x, y)]);
        }
    }
}

/// Collects live cells inside a rectangle, up to a limit.
struct RectCollector<'a> {
    store: &'a NodeStore,
    rect: Rect,
    limit: usize,
    out: Vec<(i64, i64)>,
}

impl RectCollector<'_> {
    fn visit(&mut self, n: u32, nx: i128, ny: i128) {
        let node = self.store.node(n);
        let size = 1i128 << node.level;
        if self.out.len() >= self.limit || node.population == 0 || !self.rect.intersects(nx, ny, size) {
            return;
        }
        if node.level == LEAF_LEVEL {
            let mut bits = node.leaf_bits() & self.rect.leaf_mask(nx, ny);
            while bits != 0 && self.out.len() < self.limit {
                let bit = i128::from(bits.trailing_zeros());
                bits &= bits - 1;
                // Inside the rectangle, hence inside the i64 range.
                self.out.push(((nx + bit % 8) as i64, (ny + bit / 8) as i64));
            }
            return;
        }
        let half = size / 2;
        let [nw, ne, sw, se] = node.children;
        self.visit(nw, nx, ny);
        self.visit(ne, nx + half, ny);
        self.visit(sw, nx, ny + half);
        self.visit(se, nx + half, ny + half);
    }
}

/// Accumulates populations of `2^k × 2^k` blocks into a pixel grid.
struct DensityGrid<'a> {
    store: &'a NodeStore,
    x0: i128,
    y0: i128,
    x_end: i128,
    y_end: i128,
    width: i128,
    k: u32,
    out: &'a mut [u32],
}

impl DensityGrid<'_> {
    fn add(&mut self, px: i128, py: i128, amount: u64) {
        // (px, py) is inside the grid, so the index fits in usize.
        let i = (py * self.width + px) as usize;
        let amount = amount.min(u64::from(u32::MAX)) as u32;
        self.out[i] = self.out[i].saturating_add(amount);
    }

    fn visit(&mut self, n: u32, nx: i128, ny: i128) {
        let node = self.store.node(n);
        let size = 1i128 << node.level;
        if node.population == 0 || nx >= self.x_end || ny >= self.y_end || nx + size <= self.x0 || ny + size <= self.y0
        {
            return;
        }
        let inside = nx >= self.x0 && ny >= self.y0 && nx + size <= self.x_end && ny + size <= self.y_end;
        if inside {
            let (px, py) = ((nx - self.x0) >> self.k, (ny - self.y0) >> self.k);
            if (nx + size - 1 - self.x0) >> self.k == px && (ny + size - 1 - self.y0) >> self.k == py {
                self.add(px, py, node.population);
                return;
            }
        }
        if node.level == LEAF_LEVEL {
            let mut bits = node.leaf_bits();
            while bits != 0 {
                let bit = i128::from(bits.trailing_zeros());
                bits &= bits - 1;
                let (cx, cy) = (nx + bit % 8, ny + bit / 8);
                if (self.x0..self.x_end).contains(&cx) && (self.y0..self.y_end).contains(&cy) {
                    self.add((cx - self.x0) >> self.k, (cy - self.y0) >> self.k, 1);
                }
            }
            return;
        }
        let half = size / 2;
        let [nw, ne, sw, se] = node.children;
        self.visit(nw, nx, ny);
        self.visit(ne, nx + half, ny);
        self.visit(sw, nx, ny + half);
        self.visit(se, nx + half, ny + half);
    }
}

impl Universe {
    /// Iterates over all live cells in quadtree order (see [`Cells`]).
    pub fn cells(&self) -> Cells<'_> {
        // The root corner -2^(L-1) is at least i64::MIN.
        let corner = (-half_side(self.level())) as i64;
        Cells { store: self.store(), stack: vec![(self.root(), corner, corner)], leaf: 0, leaf_x: 0, leaf_y: 0 }
    }

    /// Exact bounding box `(min_x, min_y, max_x, max_y)` (inclusive) of the
    /// live cells, or `None` when the universe is empty. Found by descending
    /// towards each edge and skipping empty nodes, with per-node memoisation.
    pub fn bounding_box(&self) -> Option<(i64, i64, i64, i64)> {
        let origin = -half_side(self.level());
        let mut memo = HashMap::new();
        let mut extent = |side: Side| {
            memo.clear();
            // Relative offsets are inside the root, so the sums fit in i64.
            edge(self.store(), self.root(), side, &mut memo).map(|v| (origin + i128::from(v)) as i64)
        };
        Some((extent(Side::Left)?, extent(Side::Top)?, extent(Side::Right)?, extent(Side::Bottom)?))
    }

    /// Live cells inside the inclusive rectangle `[x0, x1] × [y0, y1]`,
    /// sorted row-major (by `y`, then `x`). At most `limit` cells are
    /// returned; when the rectangle holds more, the subset kept is the first
    /// `limit` cells in quadtree order (deterministic), then sorted.
    pub fn cells_in_rect(&self, x0: i64, y0: i64, x1: i64, y1: i64, limit: usize) -> Vec<(i64, i64)> {
        if x0 > x1 || y0 > y1 || limit == 0 {
            return Vec::new();
        }
        let mut collector =
            RectCollector { store: self.store(), rect: Rect::new(x0, y0, x1, y1), limit, out: Vec::new() };
        let origin = -half_side(self.level());
        collector.visit(self.root(), origin, origin);
        let mut out = collector.out;
        out.sort_unstable_by_key(|&(x, y)| (y, x));
        out
    }

    /// Level-of-detail population grid for zoomed-out rendering.
    ///
    /// Pixel `(i, j)` (row-major, `width_px` per row) holds the number of
    /// live cells in the block `[x0 + i·2^k, x0 + (i+1)·2^k) × [y0 + j·2^k,
    /// y0 + (j+1)·2^k)` with `k = log2_cells_per_px`, saturating at
    /// `u32::MAX`. Whole quadtree nodes inside one pixel contribute their
    /// cached population, so when `x0` and `y0` are multiples of `2^k` the
    /// cost is O(visible non-empty nodes of level ≥ k), independent of the
    /// population. Unaligned origins are exact too, but descend further.
    ///
    /// # Errors
    /// [`HashlifeError::InvalidArgument`] if `log2_cells_per_px > 64` or the
    /// grid has more than [`MAX_DENSITY_PIXELS`] pixels.
    pub fn density_grid(
        &self,
        x0: i64,
        y0: i64,
        width_px: usize,
        height_px: usize,
        log2_cells_per_px: u32,
    ) -> Result<Vec<u32>, HashlifeError> {
        if log2_cells_per_px > 64 {
            return Err(HashlifeError::InvalidArgument(format!(
                "log2 cells per pixel must be at most 64 (got {log2_cells_per_px})"
            )));
        }
        let pixels = width_px.checked_mul(height_px).filter(|&p| p <= MAX_DENSITY_PIXELS).ok_or_else(|| {
            HashlifeError::InvalidArgument(format!(
                "a {width_px}×{height_px} density grid exceeds the limit of {MAX_DENSITY_PIXELS} pixels"
            ))
        })?;
        let mut out = vec![0u32; pixels];
        self.density_into(x0, y0, width_px, height_px, log2_cells_per_px, &mut out);
        Ok(out)
    }

    /// Fills `out` (length `width_px * height_px`, already validated) with the
    /// density grid described in [`Universe::density_grid`].
    pub(crate) fn density_into(&self, x0: i64, y0: i64, width_px: usize, height_px: usize, k: u32, out: &mut [u32]) {
        out.fill(0);
        let (width, height) = (width_px as i128, height_px as i128);
        let (x0, y0) = (i128::from(x0), i128::from(y0));
        let mut grid = DensityGrid {
            store: self.store(),
            x0,
            y0,
            x_end: x0 + (width << k),
            y_end: y0 + (height << k),
            width,
            k,
            out,
        };
        let origin = -half_side(self.level());
        grid.visit(self.root(), origin, origin);
    }

    /// Exact number of live cells as a `u128` (saturating only for a
    /// completely full 2^64 × 2^64 plane). [`Universe::population`] is the
    /// constant-time `u64` version, which saturates above `u64::MAX`.
    pub fn population_exact(&self) -> u128 {
        self.population_in_rect(i64::MIN, i64::MIN, i64::MAX, i64::MAX)
    }

    /// Exact number of live cells in the inclusive rectangle `[x0, x1] ×
    /// [y0, y1]` (0 if `x0 > x1` or `y0 > y1`), as a `u128` saturating only
    /// for a completely full plane, like [`Universe::population_exact`].
    ///
    /// No cell is listed: a quadtree node inside the rectangle adds its
    /// cached population (or, above `u64::MAX` cells, its children's), a
    /// node outside it or empty adds nothing, and only the non-empty nodes
    /// crossing the rectangle's border are split, down to 8×8 leaves that
    /// are masked and popcounted. The count of a split node no larger than
    /// the rectangle is memoised per node and set of rectangle edges
    /// crossing it: every copy of a node is aligned to its side, so the
    /// edges cut all copies at the same offsets. For a `w × h` rectangle the
    /// cost is therefore at most four nodes per level for nodes larger than
    /// the rectangle, plus O(non-empty nodes crossing the border) — about
    /// `2(w + h) / 2^j` of each smaller level `j` — and also at most
    /// O(distinct nodes): a border through a periodic region costs as much
    /// as its distinct structure, whatever its length or population.
    pub fn population_in_rect(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> u128 {
        if x0 > x1 || y0 > y1 {
            return 0;
        }
        let rect = Rect::new(x0, y0, x1, y1);
        let memo_side = (rect.x1 - rect.x0).max(rect.y1 - rect.y0) + 1;
        let mut counter = RectCounter { store: self.store(), rect, memo_side, memo: HashMap::new() };
        let origin = -half_side(self.level());
        counter.count(self.root(), origin, origin)
    }
}

/// Counts the live cells of a rectangle; see [`Universe::population_in_rect`].
struct RectCounter<'a> {
    store: &'a NodeStore,
    rect: Rect,
    /// Side of the largest nodes whose counts are memoised. Larger nodes
    /// meet the rectangle at most four per level, so they are not.
    memo_side: i128,
    /// Count of a node split by the edges of `rect` that cross it (bits:
    /// left, right, top, bottom; none for a node inside `rect` whose cached
    /// population saturates).
    memo: HashMap<(u32, u8), u128>,
}

impl RectCounter<'_> {
    fn count(&mut self, n: u32, nx: i128, ny: i128) -> u128 {
        let node = self.store.node(n);
        let size = 1i128 << node.level;
        let rect = &self.rect;
        if node.population == 0 || !rect.intersects(nx, ny, size) {
            return 0;
        }
        let edges = u8::from(nx < rect.x0)
            | u8::from(nx + size - 1 > rect.x1) << 1
            | u8::from(ny < rect.y0) << 2
            | u8::from(ny + size - 1 > rect.y1) << 3;
        if edges == 0 && node.population < u64::MAX {
            return u128::from(node.population);
        }
        if node.level == LEAF_LEVEL {
            return u128::from((node.leaf_bits() & rect.leaf_mask(nx, ny)).count_ones());
        }
        // Every node but the root (the only node of its level) lies at a
        // multiple of its side, so its count depends only on it and `edges`.
        let memoised = size <= self.memo_side;
        if memoised && let Some(&known) = self.memo.get(&(n, edges)) {
            return known;
        }
        let half = size / 2;
        let [nw, ne, sw, se] = node.children;
        let total = [(nw, nx, ny), (ne, nx + half, ny), (sw, nx, ny + half), (se, nx + half, ny + half)]
            .into_iter()
            .fold(0u128, |acc, (c, x, y)| acc.saturating_add(self.count(c, x, y)));
        if memoised {
            self.memo.insert((n, edges), total);
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_mask_selects_the_overlap() {
        let rect = Rect::new(2, 3, 4, 3);
        assert_eq!(rect.leaf_mask(0, 0), 0b0001_1100 << 24);
        assert_eq!(rect.leaf_mask(10, 0), 0);
        let all = Rect::new(-100, -100, 100, 100);
        assert_eq!(all.leaf_mask(0, 0), u64::MAX);
    }

    #[test]
    fn population_exact_beyond_u64() {
        let mut u = Universe::conway();
        let store = u.store_mut();
        let mut full = store.leaf(u64::MAX);
        for _ in 4..=33 {
            full = store.join([full; 4]);
        }
        let empty = store.empty(33);
        let root = store.join([empty, empty, empty, full]);
        u.set_root(root);
        assert_eq!(u.population(), u64::MAX, "u64 population saturates");
        assert_eq!(u.population_exact(), 1u128 << 66);
        assert_eq!(u.bounding_box(), Some((0, 0, (1 << 33) - 1, (1 << 33) - 1)));
        // Rectangles over the saturated nodes are exact too.
        let side = 1i64 << 33;
        assert_eq!(u.population_in_rect(i64::MIN, i64::MIN, i64::MAX, i64::MAX), 1 << 66);
        assert_eq!(u.population_in_rect(-5, -5, side, side / 2 - 1), 1 << 65);
        assert_eq!(u.population_in_rect(1, 0, side + 9, side - 1), (1 << 66) - (1 << 33));
        assert_eq!(u.population_in_rect(3, 7, side - 2, side - 4), (side as u128 - 4) * (side as u128 - 10));
    }

    #[test]
    fn queries_on_an_empty_universe() {
        let u = Universe::conway();
        assert_eq!(u.cells().count(), 0);
        assert_eq!(u.bounding_box(), None);
        assert!(u.cells_in_rect(-10, -10, 10, 10, 100).is_empty());
        assert_eq!(u.population_in_rect(i64::MIN, i64::MIN, i64::MAX, i64::MAX), 0);
        assert_eq!(u.density_grid(-8, -8, 4, 4, 2).unwrap(), vec![0; 16]);
        assert!(u.density_grid(0, 0, 1 << 14, 1 << 14, 0).is_err());
        assert!(u.density_grid(0, 0, 1, 1, 65).is_err());
    }

    #[test]
    fn extreme_coordinates_round_trip_through_queries() {
        let mut u = Universe::conway();
        let corners = [(i64::MIN, i64::MIN), (i64::MAX, i64::MAX), (i64::MIN, i64::MAX), (0, 0)];
        u.insert_cells(corners);
        let mut cells: Vec<_> = u.cells().collect();
        cells.sort_unstable();
        let mut expected = corners.to_vec();
        expected.sort_unstable();
        assert_eq!(cells, expected);
        assert_eq!(u.bounding_box(), Some((i64::MIN, i64::MIN, i64::MAX, i64::MAX)));
        assert_eq!(u.cells_in_rect(i64::MIN, i64::MIN, i64::MAX, i64::MAX, 10).len(), 4);
        assert_eq!(u.population_in_rect(i64::MIN, i64::MIN, i64::MAX, i64::MAX), 4);
        assert_eq!(u.population_in_rect(i64::MIN, 0, 0, i64::MAX), 2, "(MIN, MAX) and (0, 0)");
        assert_eq!(u.population_in_rect(i64::MAX, i64::MAX, i64::MAX, i64::MAX), 1);
        assert_eq!(u.population_in_rect(i64::MIN + 1, i64::MIN, i64::MAX, i64::MAX - 1), 1, "only (0, 0)");
        assert_eq!(u.population_in_rect(1, 1, 0, 0), 0, "reversed bounds");
        let grid = u.density_grid(i64::MIN, i64::MIN, 2, 2, 63).unwrap();
        assert_eq!(grid, vec![1, 0, 1, 2]);
    }
}
