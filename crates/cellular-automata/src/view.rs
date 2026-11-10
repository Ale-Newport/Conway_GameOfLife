//! The 2D grid view contract shared by every model of this crate and consumed
//! by the Bevy grid renderer.
//!
//! A renderer sends a [`GridViewRequest`]: an **inclusive** cell rectangle
//! `[x0, x1] × [y0, y1]` and a texel budget. The model chooses
//! `cells_per_texel` — the smallest power of two for which the texture fits
//! the budget — and returns a [`GridView`]. Texel boundaries are aligned to
//! multiples of `cells_per_texel` in absolute cell coordinates, so they stay
//! fixed while the camera pans; the view covers every requested cell.
//!
//! Every texel aggregates only the cells of its `k × k` block that lie inside
//! the requested rectangle (and inside the world, for finite worlds): models
//! never read cells outside the request. Channels (RGBA8, row 0 = top):
//!
//! | Channel | Exact texels (`k = 1`) | Level-of-detail texels (`k > 1`) |
//! |---|---|---|
//! | R | cell state (0 = dead) | most frequent non-zero state of the block (ties: lowest state), 0 if none |
//! | G | age in generations, saturating at 255; 0 when not tracked | mean age of the live cells, rounded |
//! | B | activity: 255 when the cell changed during the last tick, decaying afterwards; 0 when not tracked | maximum activity in the block |
//! | A | 255 if the cell is non-zero, else 0 | `round(255 × non-zero cells / covered cells)` |
//!
//! Texels that cover no existing cell (outside a finite world) are all zero.

use serde::{Deserialize, Serialize};

use crate::geometry::CellRect;

/// Largest texture side a view may have (wgpu's default 2D texture limit).
pub const MAX_VIEW_SIDE: u32 = 8192;
/// Largest level of detail: one texel covers at most `2^31 × 2^31` cells.
pub const MAX_LOG2_CELLS_PER_TEXEL: u32 = 31;

/// What a renderer asks for: an inclusive cell rectangle and a texel budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GridViewRequest {
    /// Left column (inclusive).
    pub x0: i64,
    /// Top row (inclusive).
    pub y0: i64,
    /// Right column (inclusive).
    pub x1: i64,
    /// Bottom row (inclusive).
    pub y1: i64,
    /// Largest acceptable texture width in texels (clamped to `1..=8192`).
    pub max_width: u32,
    /// Largest acceptable texture height in texels (clamped to `1..=8192`).
    pub max_height: u32,
}

impl Default for GridViewRequest {
    fn default() -> Self {
        Self { x0: 0, y0: 0, x1: 255, y1: 255, max_width: 1024, max_height: 1024 }
    }
}

impl GridViewRequest {
    /// Request for `rect` with a square budget of `max_side` texels.
    pub fn new(rect: CellRect, max_side: u32) -> Self {
        Self { x0: rect.x0, y0: rect.y0, x1: rect.x1, y1: rect.y1, max_width: max_side, max_height: max_side }
    }

    /// The requested rectangle with ordered corners.
    pub fn rect(&self) -> CellRect {
        CellRect::new(self.x0, self.y0, self.x1, self.y1)
    }
}

/// Owned render data for a [`GridViewRequest`] (see the module docs for the
/// channel layout). This struct is the renderer contract; keep its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridView {
    /// Cell x coordinate of the left edge of texel (0, 0).
    pub origin_x: i64,
    /// Cell y coordinate of the top edge of texel (0, 0).
    pub origin_y: i64,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Cells per texel side: a power of two, 1 = exact cells.
    pub cells_per_texel: u32,
    /// RGBA bytes, row-major, row 0 at the top; `width × height × 4` bytes.
    pub rgba: Vec<u8>,
}

impl GridView {
    /// The RGBA bytes of texel `(tx, ty)`, or `None` outside the view.
    pub fn texel(&self, tx: u32, ty: u32) -> Option<[u8; 4]> {
        if tx >= self.width || ty >= self.height {
            return None;
        }
        let i = (ty as usize * self.width as usize + tx as usize) * 4;
        Some([self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]])
    }

    /// The RGBA bytes of the texel covering cell `(x, y)`, or `None` outside.
    pub fn texel_at_cell(&self, x: i64, y: i64) -> Option<[u8; 4]> {
        let k = i128::from(self.cells_per_texel);
        let tx = (i128::from(x) - i128::from(self.origin_x)).div_euclid(k);
        let ty = (i128::from(y) - i128::from(self.origin_y)).div_euclid(k);
        if tx < 0 || ty < 0 {
            return None;
        }
        self.texel(u32::try_from(tx).ok()?, u32::try_from(ty).ok()?)
    }
}

/// The texel layout chosen for a request: which cells each texel covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexelGrid {
    /// Cells that may be read: the request clipped to the world.
    pub clip: Option<CellRect>,
    /// Cell x of texel column 0 (a multiple of `2^log2_k`).
    pub origin_x: i64,
    /// Cell y of texel row 0 (a multiple of `2^log2_k`).
    pub origin_y: i64,
    /// `log2(cells_per_texel)`.
    pub log2_k: u32,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
}

/// Number of aligned blocks of side `2^log2_k` touched by `[a, b]`.
fn blocks(a: i64, b: i64, log2_k: u32) -> i128 {
    (i128::from(b) >> log2_k) - (i128::from(a) >> log2_k) + 1
}

impl TexelGrid {
    /// Chooses the level of detail for `request`. `bounds` are the world's
    /// bounds (`None` on the infinite plane); cells outside them are never
    /// read. A request too large for `2^31` cells per texel within the budget
    /// is shrunk around its centre.
    pub fn plan(request: &GridViewRequest, bounds: Option<CellRect>) -> Self {
        let mut rect = request.rect();
        let max_w = request.max_width.clamp(1, MAX_VIEW_SIDE);
        let max_h = request.max_height.clamp(1, MAX_VIEW_SIDE);
        let fits = |r: &CellRect, k: u32| {
            blocks(r.x0, r.x1, k) <= i128::from(max_w) && blocks(r.y0, r.y1, k) <= i128::from(max_h)
        };
        let log2_k = match (0..=MAX_LOG2_CELLS_PER_TEXEL).find(|&k| fits(&rect, k)) {
            Some(k) => k,
            None => {
                let k = MAX_LOG2_CELLS_PER_TEXEL;
                rect = shrink(rect, max_w, max_h, k);
                k
            }
        };
        let origin_x = ((i128::from(rect.x0) >> log2_k) << log2_k) as i64;
        let origin_y = ((i128::from(rect.y0) >> log2_k) << log2_k) as i64;
        let width = blocks(rect.x0, rect.x1, log2_k) as u32;
        let height = blocks(rect.y0, rect.y1, log2_k) as u32;
        let clip = match bounds {
            Some(b) => rect.intersect(&b),
            None => Some(rect),
        };
        Self { clip, origin_x, origin_y, log2_k, width, height }
    }

    /// Cells per texel side.
    pub fn cells_per_texel(&self) -> u32 {
        1 << self.log2_k
    }

    /// Number of texels.
    pub fn len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Whether the grid has no texel (never true for a planned grid).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Texel column of cell column `x` (`x` inside the clip).
    #[inline]
    pub fn column(&self, x: i64) -> usize {
        ((i128::from(x) - i128::from(self.origin_x)) >> self.log2_k) as usize
    }

    /// Texel row of cell row `y` (`y` inside the clip).
    #[inline]
    pub fn row(&self, y: i64) -> usize {
        ((i128::from(y) - i128::from(self.origin_y)) >> self.log2_k) as usize
    }

    /// Texel index of cell `(x, y)` (inside the clip).
    #[inline]
    pub fn index(&self, x: i64, y: i64) -> usize {
        self.row(y) * self.width as usize + self.column(x)
    }

    /// Cells of texel column `tx` (or row) that lie inside the clip, along
    /// one axis.
    fn covered_1d(&self, origin: i64, t: u32, lo: i64, hi: i64) -> u64 {
        let k = 1i128 << self.log2_k;
        let start = i128::from(origin) + i128::from(t) * k;
        let end = start + k - 1;
        let a = start.max(i128::from(lo));
        let b = end.min(i128::from(hi));
        if a > b { 0 } else { (b - a + 1) as u64 }
    }

    /// Number of readable cells covered by every texel (row-major).
    pub fn coverage(&self) -> Vec<u64> {
        let mut out = vec![0u64; self.len()];
        let Some(clip) = self.clip else {
            return out;
        };
        let cols: Vec<u64> = (0..self.width).map(|t| self.covered_1d(self.origin_x, t, clip.x0, clip.x1)).collect();
        for ty in 0..self.height {
            let rows = self.covered_1d(self.origin_y, ty, clip.y0, clip.y1);
            let base = ty as usize * self.width as usize;
            for (tx, &c) in cols.iter().enumerate() {
                out[base + tx] = c.saturating_mul(rows);
            }
        }
        out
    }

    /// Builds a view from per-texel channel values. `occupied[i]` is the
    /// number of non-zero cells of texel `i`; `state`, `age` and `activity`
    /// give the R, G and B channels.
    pub fn assemble(&self, occupied: &[u32], state: &[u8], age: &[u8], activity: &[u8]) -> GridView {
        let coverage = self.coverage();
        let mut rgba = vec![0u8; self.len() * 4];
        let (texels, _) = rgba.as_chunks_mut::<4>();
        for (i, texel) in texels.iter_mut().enumerate() {
            let covered = coverage[i];
            if covered == 0 {
                continue;
            }
            let count = u64::from(occupied[i]).min(covered);
            texel[0] = state[i];
            texel[1] = age[i];
            texel[2] = activity[i];
            texel[3] = ((u128::from(count) * 255 + u128::from(covered) / 2) / u128::from(covered)) as u8;
        }
        GridView {
            origin_x: self.origin_x,
            origin_y: self.origin_y,
            width: self.width,
            height: self.height,
            cells_per_texel: self.cells_per_texel(),
            rgba,
        }
    }
}

/// Shrinks `rect` around its centre so that it spans exactly the budget at
/// level `log2_k`.
fn shrink(rect: CellRect, max_w: u32, max_h: u32, log2_k: u32) -> CellRect {
    let axis = |a: i64, b: i64, budget: u32| -> (i64, i64) {
        if blocks(a, b, log2_k) <= i128::from(budget) {
            return (a, b);
        }
        let k = 1i128 << log2_k;
        let centre = (i128::from(a) + i128::from(b)) / 2;
        let start = ((centre - i128::from(budget / 2) * k) >> log2_k) << log2_k;
        let start = start.clamp(i128::from(i64::MIN), i128::from(i64::MAX) - i128::from(budget) * k + 1);
        let end = start + i128::from(budget) * k - 1;
        (start as i64, end as i64)
    };
    let (x0, x1) = axis(rect.x0, rect.x1, max_w);
    let (y0, y1) = axis(rect.y0, rect.y1, max_h);
    CellRect { x0, y0, x1, y1 }
}

/// Accumulates one row of texels for multi-state views: the number of
/// non-zero cells and the most frequent non-zero state of every texel. Models
/// fill one texel row at a time, so memory stays proportional to the view
/// width.
#[derive(Debug)]
pub struct RowAccumulator {
    occupied: Vec<u32>,
    histograms: Vec<Vec<(u8, u32)>>,
}

impl RowAccumulator {
    /// Empty accumulator for a row of `width` texels.
    pub fn new(width: usize) -> Self {
        Self { occupied: vec![0; width], histograms: vec![Vec::new(); width] }
    }

    /// Records one cell of `state` in texel column `column` (state 0 is ignored).
    #[inline]
    pub fn add(&mut self, column: usize, state: u8) {
        if state == 0 {
            return;
        }
        self.occupied[column] += 1;
        let hist = &mut self.histograms[column];
        match hist.iter_mut().find(|(s, _)| *s == state) {
            Some(entry) => entry.1 += 1,
            None => hist.push((state, 1)),
        }
    }

    /// Writes the row (occupied counts and the most frequent non-zero state,
    /// ties broken towards the lowest state) and clears the accumulator.
    pub fn flush(&mut self, occupied: &mut [u32], state: &mut [u8]) {
        for (i, hist) in self.histograms.iter_mut().enumerate() {
            occupied[i] = self.occupied[i];
            state[i] = hist.iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map_or(0, |&(s, _)| s);
            hist.clear();
            self.occupied[i] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_view_when_budget_allows() {
        let req = GridViewRequest { x0: -3, y0: 2, x1: 4, y1: 5, max_width: 8, max_height: 8 };
        let g = TexelGrid::plan(&req, None);
        assert_eq!((g.log2_k, g.width, g.height, g.origin_x, g.origin_y), (0, 8, 4, -3, 2));
        assert!(g.coverage().iter().all(|&c| c == 1));
    }

    #[test]
    fn lod_is_aligned_and_clipped() {
        // 100 cells wide, budget 16 texels: k = 8 (13 blocks once aligned).
        let req = GridViewRequest { x0: 3, y0: 0, x1: 102, y1: 7, max_width: 16, max_height: 16 };
        let g = TexelGrid::plan(&req, None);
        assert_eq!(g.log2_k, 3);
        assert_eq!(g.origin_x, 0);
        assert_eq!(g.width, 13);
        let cov = g.coverage();
        assert_eq!(cov[0], 5 * 8, "first texel covers x = 3..=7 only");
        assert_eq!(cov[1], 64);
        assert_eq!(cov[12], 7 * 8, "last texel covers x = 96..=102");
        assert_eq!(g.column(3), 0);
        assert_eq!(g.column(102), 12);
    }

    #[test]
    fn finite_bounds_limit_coverage() {
        let req = GridViewRequest { x0: -4, y0: -4, x1: 3, y1: 3, max_width: 8, max_height: 8 };
        let g = TexelGrid::plan(&req, Some(CellRect::new(0, 0, 9, 9)));
        let cov = g.coverage();
        assert_eq!(cov.iter().sum::<u64>(), 16);
        let view = g.assemble(&[1; 64], &[1; 64], &[0; 64], &[0; 64]);
        assert_eq!(view.texel(0, 0), Some([0, 0, 0, 0]), "outside the world");
        assert_eq!(view.texel_at_cell(0, 0), Some([1, 0, 0, 255]));
    }

    #[test]
    fn huge_requests_are_shrunk() {
        let req =
            GridViewRequest { x0: i64::MIN, y0: i64::MIN, x1: i64::MAX, y1: i64::MAX, max_width: 64, max_height: 32 };
        let g = TexelGrid::plan(&req, None);
        assert_eq!(g.log2_k, MAX_LOG2_CELLS_PER_TEXEL);
        assert_eq!((g.width, g.height), (64, 32));
        assert_eq!(g.origin_x % (1 << 31), 0);
    }

    #[test]
    fn density_rounding() {
        let g = TexelGrid::plan(&GridViewRequest { x0: 0, y0: 0, x1: 3, y1: 3, max_width: 1, max_height: 1 }, None);
        assert_eq!(g.log2_k, 2);
        let v = g.assemble(&[8], &[1], &[0], &[0]);
        assert_eq!(v.rgba, vec![1, 0, 0, 128]);
    }

    #[test]
    fn state_histogram_prefers_frequent_then_lowest() {
        let mut acc = RowAccumulator::new(2);
        for s in [3, 3, 1] {
            acc.add(0, s);
        }
        for s in [2, 1, 0] {
            acc.add(1, s);
        }
        let (mut occ, mut st) = ([0u32; 2], [0u8; 2]);
        acc.flush(&mut occ, &mut st);
        assert_eq!(st, [3, 1]);
        assert_eq!(occ, [3, 2]);
        acc.flush(&mut occ, &mut st);
        assert_eq!((occ, st), ([0, 0], [0, 0]), "flush clears");
    }
}
