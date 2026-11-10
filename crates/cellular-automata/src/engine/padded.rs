//! Byte-per-cell grids with a one-cell ghost ring, stepped through a lookup
//! table. Shared by the dense Life engine and the multi-state models
//! (Generations, Wireworld).
//!
//! The grid stores `(width + 2) × (height + 2)` bytes. Before every step the
//! ghost ring is refilled from the topology (Golly's bounded-grid emulation:
//! each ghost cell is a copy of the cell [`Topology::wrap`] maps it to, or
//! dead outside a bounded plane), so the step kernel reads every neighbour
//! without bounds checks or wrap arithmetic. The next state of a cell is
//! `lut[state][n]`, where `n` counts the Moore neighbours in state 1.

use rayon::prelude::*;
use simulation_core::Topology;

/// Grids with at least this many cells are stepped in parallel row bands.
const PAR_MIN_CELLS: usize = 1 << 16;

/// Transition table: next state for `(state, live neighbours)`, where live
/// means state 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lut {
    table: Box<[[u8; 9]; 256]>,
    /// Whether every state is 0 or 1 (neighbour counts are then plain sums).
    binary: bool,
}

impl Lut {
    /// Table built from `next(state, n)` for states `0..states`; states at or
    /// above `states` (which models never store) map to 0.
    pub(crate) fn from_fn(states: u16, next: impl Fn(u8, u8) -> u8) -> Self {
        let mut table = Box::new([[0u8; 9]; 256]);
        for (s, row) in table.iter_mut().enumerate().take(usize::from(states)) {
            for (n, cell) in row.iter_mut().enumerate() {
                *cell = next(s as u8, n as u8);
            }
        }
        Self { table, binary: states <= 2 }
    }

    /// Two-state Life-like table for `birth`/`survive` masks.
    pub(crate) fn life(birth: u16, survive: u16) -> Self {
        Self::from_fn(2, |s, n| {
            let mask = if s == 1 { survive } else { birth };
            u8::from(mask >> n & 1 == 1)
        })
    }
}

/// Counts gathered while stepping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StepCounts {
    /// Cells in state 1 after the step.
    pub ones: u64,
    /// Non-zero cells after the step.
    pub nonzero: u64,
    /// Cells that entered state 1 (births for Life).
    pub entered_one: u64,
    /// Cells that left state 1 (deaths for Life).
    pub left_one: u64,
}

impl StepCounts {
    fn add(self, o: Self) -> Self {
        Self {
            ones: self.ones + o.ones,
            nonzero: self.nonzero + o.nonzero,
            entered_one: self.entered_one + o.entered_one,
            left_one: self.left_one + o.left_one,
        }
    }
}

/// A byte grid with a ghost ring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Padded {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl Padded {
    /// All-zero grid of `width × height` interior cells.
    pub(crate) fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![0; (width + 2) * (height + 2)] }
    }

    /// Interior width.
    pub(crate) fn width(&self) -> usize {
        self.width
    }

    /// Interior height.
    pub(crate) fn height(&self) -> usize {
        self.height
    }

    #[inline]
    fn idx(&self, x: usize, y: usize) -> usize {
        (y + 1) * (self.width + 2) + x + 1
    }

    /// State of interior cell `(x, y)`.
    #[inline]
    pub(crate) fn get(&self, x: usize, y: usize) -> u8 {
        self.data[self.idx(x, y)]
    }

    /// Writes interior cell `(x, y)` and returns its previous state.
    #[inline]
    pub(crate) fn set(&mut self, x: usize, y: usize, value: u8) -> u8 {
        let i = self.idx(x, y);
        std::mem::replace(&mut self.data[i], value)
    }

    /// Interior row `y` (`width` bytes).
    #[inline]
    pub(crate) fn row(&self, y: usize) -> &[u8] {
        let start = self.idx(0, y);
        &self.data[start..start + self.width]
    }

    /// Sets every cell (and ghost) to 0.
    pub(crate) fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Heap bytes.
    pub(crate) fn memory_bytes(&self) -> usize {
        self.data.capacity()
    }

    /// Refills the ghost ring from `topology` (the infinite plane behaves as
    /// a bounded plane: the ring is dead).
    pub(crate) fn fill_ghosts(&mut self, topology: Topology) {
        let (w, h) = (self.width as i64, self.height as i64);
        let ghost = |grid: &Self, x: i64, y: i64| -> u8 {
            if topology == Topology::Infinite {
                return 0;
            }
            topology.wrap(x, y, w, h).map_or(0, |(sx, sy)| grid.get(sx as usize, sy as usize))
        };
        let stride = self.width + 2;
        for x in -1..=w {
            let top = ghost(self, x, -1);
            let bottom = ghost(self, x, h);
            self.data[(x + 1) as usize] = top;
            self.data[(h as usize + 1) * stride + (x + 1) as usize] = bottom;
        }
        for y in 0..h {
            let left = ghost(self, -1, y);
            let right = ghost(self, w, y);
            self.data[(y as usize + 1) * stride] = left;
            self.data[(y as usize + 1) * stride + self.width + 1] = right;
        }
    }

    /// Computes the next generation into `out` (same size) using the ghost
    /// ring as it is (call [`Padded::fill_ghosts`] first). Row bands run in
    /// parallel on large grids; the result does not depend on the banding.
    pub(crate) fn step_into(&self, lut: &Lut, out: &mut Padded) -> StepCounts {
        debug_assert_eq!((self.width, self.height), (out.width, out.height));
        let (w, h) = (self.width, self.height);
        let stride = w + 2;
        let src = &self.data;
        let threads = rayon::current_num_threads().max(1);
        let rows_per_band = h.div_ceil(threads * 4).max(1);
        let body = &mut out.data[stride..stride * (h + 1)];
        let work = |(band, chunk): (usize, &mut [u8])| -> StepCounts {
            let mut counts = StepCounts::default();
            for (i, out_row) in chunk.chunks_mut(stride).enumerate() {
                let y = band * rows_per_band + i;
                let above = &src[y * stride..(y + 1) * stride];
                let mid = &src[(y + 1) * stride..(y + 2) * stride];
                let below = &src[(y + 2) * stride..(y + 3) * stride];
                if lut.binary {
                    row_kernel::<true>(lut, above, mid, below, out_row, &mut counts);
                } else {
                    row_kernel::<false>(lut, above, mid, below, out_row, &mut counts);
                }
            }
            counts
        };
        if w * h >= PAR_MIN_CELLS {
            body.par_chunks_mut(rows_per_band * stride)
                .enumerate()
                .map(work)
                .reduce(StepCounts::default, StepCounts::add)
        } else {
            body.chunks_mut(rows_per_band * stride).enumerate().map(work).fold(StepCounts::default(), StepCounts::add)
        }
    }
}

/// One output row: `above`, `mid`, `below` and `out` are padded rows.
#[inline]
fn row_kernel<const BINARY: bool>(
    lut: &Lut,
    above: &[u8],
    mid: &[u8],
    below: &[u8],
    out: &mut [u8],
    counts: &mut StepCounts,
) {
    let w = out.len() - 2;
    let (above, mid, below) = (&above[..w + 2], &mid[..w + 2], &below[..w + 2]);
    let live = |v: u8| if BINARY { v } else { u8::from(v == 1) };
    for x in 0..w {
        let n = live(above[x])
            + live(above[x + 1])
            + live(above[x + 2])
            + live(mid[x])
            + live(mid[x + 2])
            + live(below[x])
            + live(below[x + 1])
            + live(below[x + 2]);
        let s = mid[x + 1];
        let next = lut.table[usize::from(s)][usize::from(n)];
        out[x + 1] = next;
        let (was_one, is_one) = (s == 1, next == 1);
        counts.ones += u64::from(is_one);
        counts.nonzero += u64::from(next != 0);
        counts.entered_one += u64::from(is_one && !was_one);
        counts.left_one += u64::from(was_one && !is_one);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blinker_on_torus_and_ghosts() {
        let lut = Lut::life(1 << 3, (1 << 2) | (1 << 3));
        let mut g = Padded::new(5, 5);
        for x in 1..4 {
            g.set(x, 2, 1);
        }
        let mut next = Padded::new(5, 5);
        g.fill_ghosts(Topology::Torus);
        let c = g.step_into(&lut, &mut next);
        assert_eq!(c, StepCounts { ones: 3, nonzero: 3, entered_one: 2, left_one: 2 });
        assert_eq!((next.get(2, 1), next.get(2, 2), next.get(2, 3), next.get(1, 2)), (1, 1, 1, 0));
    }

    #[test]
    fn ghost_ring_follows_topology() {
        let mut g = Padded::new(4, 3);
        g.set(0, 0, 1);
        g.set(3, 2, 2);
        g.fill_ghosts(Topology::Torus);
        let stride = 6;
        // Ghost (-1, -1) is cell (3, 2); ghost (4, 0) is cell (0, 0).
        assert_eq!(g.data[0], 2);
        assert_eq!(g.data[stride + 5], 1);
        g.fill_ghosts(Topology::KleinBottle);
        // Klein: (0, 3) -> mirrored x on crossing the bottom: (3, 0).
        assert_eq!(g.data[4 * stride + 1], 0);
        assert_eq!(g.data[4 * stride + 4], 1);
        g.fill_ghosts(Topology::Bounded);
        assert_eq!(g.data[0], 0);
    }

    #[test]
    fn multistate_lut_counts_state_one_only() {
        // Wireworld-like: wire (3) becomes head (1) with 1 or 2 head neighbours.
        let lut = Lut::from_fn(4, |s, n| match s {
            1 => 2,
            2 => 3,
            3 if n == 1 || n == 2 => 1,
            other => other,
        });
        let mut g = Padded::new(4, 1);
        g.set(0, 0, 1);
        g.set(1, 0, 3);
        g.set(2, 0, 3);
        g.set(3, 0, 2);
        let mut next = Padded::new(4, 1);
        g.fill_ghosts(Topology::Bounded);
        g.step_into(&lut, &mut next);
        assert_eq!(next.row(0), &[2, 1, 3, 3]);
    }
}
