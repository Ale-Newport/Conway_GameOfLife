//! `BitEngine`: 64 cells per `u64`, SWAR full-adder counting, ghost border.

use patterns::LifeRule;
use rayon::prelude::*;
use simulation_core::Topology;

use super::swar::{Kernel, bit_range, count_word, last_word_mask, push_cells, reverse_row, shift_east, shift_west};
use super::{EngineKind, GenStats, LifeEngine};
use crate::error::CaError;
use crate::geometry::{CellRect, World};
use crate::view::TexelGrid;

/// Worlds with at least this many words are stepped in parallel row bands.
const PAR_MIN_WORDS: usize = 4096;

/// Bit-packed finite-world engine.
///
/// Row `y` occupies `ceil(width / 64)` words; bit `i` of word `k` is the cell
/// at column `64k + i`, and bits beyond `width` are kept at zero. Every step
/// first builds the *ghost border* from the topology — the rows at `y = −1`
/// and `y = height` and the single cells at `x = −1` and `x = width` of every
/// row, each the cell [`Topology::wrap`] maps it to (dead on the bounded
/// plane). Twisted edges (Klein bottle, cross-surface) use bit-reversed rows.
/// The rule is then evaluated 64 cells at a time (the SWAR kernel of `engine::swar`) in row bands
/// on rayon's persistent global pool. Births, deaths and the population are
/// counted with `popcount` while stepping.
#[derive(Clone, Debug)]
pub struct BitEngine {
    world: World,
    rule: LifeRule,
    kernel: Kernel,
    width: usize,
    height: usize,
    words_per_row: usize,
    last_mask: u64,
    cells: Vec<u64>,
    next: Vec<u64>,
    population: u64,
}

/// The ghost border of one step.
struct Ghosts {
    top: Vec<u64>,
    bottom: Vec<u64>,
    /// Cells at `x = −1` for rows `−1..=height` (index `y + 1`).
    left: Vec<bool>,
    /// Cells at `x = width` for rows `−1..=height`.
    right: Vec<bool>,
}

impl BitEngine {
    /// Empty engine for a finite world.
    ///
    /// # Errors
    /// See [`EngineKind::check`].
    pub fn new(world: World, rule: LifeRule) -> Result<Self, CaError> {
        EngineKind::Bit.check(&world, rule)?;
        let (width, height) = (world.width as usize, world.height as usize);
        let words_per_row = width.div_ceil(64);
        let len = words_per_row * height;
        Ok(Self {
            world,
            rule,
            kernel: Kernel::new(rule),
            width,
            height,
            words_per_row,
            last_mask: last_word_mask(width),
            cells: vec![0; len],
            next: vec![0; len],
            population: 0,
        })
    }

    fn row(&self, y: usize) -> &[u64] {
        &self.cells[y * self.words_per_row..(y + 1) * self.words_per_row]
    }

    #[inline]
    fn bit(&self, x: usize, y: usize) -> bool {
        self.cells[y * self.words_per_row + x / 64] >> (x % 64) & 1 == 1
    }

    /// Value of the (possibly out-of-range) cell `(x, y)` seen through the topology.
    fn mapped(&self, x: i64, y: i64) -> bool {
        self.world.map(x, y).is_some_and(|(mx, my)| self.bit(mx as usize, my as usize))
    }

    fn ghosts(&self) -> Ghosts {
        let n = self.words_per_row;
        let (w, h) = (self.width as i64, self.height as i64);
        let mut top = vec![0u64; n];
        let mut bottom = vec![0u64; n];
        match self.world.topology {
            Topology::Bounded | Topology::Infinite => {}
            Topology::Torus => {
                top.copy_from_slice(self.row(self.height - 1));
                bottom.copy_from_slice(self.row(0));
            }
            Topology::KleinBottle | Topology::CrossSurface => {
                reverse_row(self.row(self.height - 1), self.width, &mut top);
                reverse_row(self.row(0), self.width, &mut bottom);
            }
        }
        let left = (-1..=h).map(|y| self.mapped(-1, y)).collect();
        let right = (-1..=h).map(|y| self.mapped(w, y)).collect();
        Ghosts { top, bottom, left, right }
    }

    /// Recounts the population (after bulk edits).
    fn recount(&mut self) {
        self.population = self.cells.iter().map(|w| u64::from(w.count_ones())).sum();
    }

    /// Word range and masks of the columns `[x0, x1]` (inside the world).
    fn column_words(x0: i64, x1: i64) -> impl Iterator<Item = (usize, u64)> {
        let (k0, k1) = ((x0 / 64) as usize, (x1 / 64) as usize);
        (k0..=k1).map(move |k| {
            let lo = if k == k0 { (x0 % 64) as u32 } else { 0 };
            let hi = if k == k1 { (x1 % 64) as u32 } else { 63 };
            (k, bit_range(lo, hi))
        })
    }

    fn clip(&self, rect: &CellRect) -> Option<CellRect> {
        self.world.clip(rect)
    }
}

/// Input row `y` (`−1..=height`, ghost rows at both ends) with its ghost
/// cells at `x = −1` and `x = width`.
fn input_row<'a>(cells: &'a [u64], ghosts: &'a Ghosts, n: usize, height: usize, y: i64) -> (&'a [u64], bool, bool) {
    let i = (y + 1) as usize;
    let row = if y < 0 {
        &ghosts.top[..]
    } else if y as usize >= height {
        &ghosts.bottom[..]
    } else {
        &cells[y as usize * n..(y as usize + 1) * n]
    };
    (row, ghosts.left[i], ghosts.right[i])
}

/// West-shifted, centre and east-shifted words of one input row.
struct Shifted {
    west: Vec<u64>,
    east: Vec<u64>,
}

impl Shifted {
    fn new(n: usize) -> Self {
        Self { west: vec![0; n], east: vec![0; n] }
    }

    fn fill(&mut self, row: &[u64], left: bool, right: bool, width: usize) {
        shift_west(row, left, &mut self.west);
        shift_east(row, right, width, &mut self.east);
    }
}

impl LifeEngine for BitEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Bit
    }

    fn world(&self) -> World {
        self.world
    }

    fn rule(&self) -> LifeRule {
        self.rule
    }

    fn set_rule(&mut self, rule: LifeRule) -> Result<(), CaError> {
        EngineKind::Bit.check(&self.world, rule)?;
        self.rule = rule;
        self.kernel = Kernel::new(rule);
        Ok(())
    }

    fn get_cell(&self, x: i64, y: i64) -> bool {
        self.mapped(x, y)
    }

    fn set_cell(&mut self, x: i64, y: i64, alive: bool) {
        let Some((mx, my)) = self.world.map(x, y) else {
            return;
        };
        let (mx, my) = (mx as usize, my as usize);
        let word = &mut self.cells[my * self.words_per_row + mx / 64];
        let bit = 1u64 << (mx % 64);
        let was = *word & bit != 0;
        if alive {
            *word |= bit;
        } else {
            *word &= !bit;
        }
        match (was, alive) {
            (false, true) => self.population += 1,
            (true, false) => self.population -= 1,
            _ => {}
        }
    }

    fn clear(&mut self) {
        self.cells.fill(0);
        self.population = 0;
    }

    fn clear_rect(&mut self, rect: CellRect) {
        let Some(clip) = self.clip(&rect) else {
            return;
        };
        for y in clip.y0..=clip.y1 {
            let base = y as usize * self.words_per_row;
            for (k, mask) in Self::column_words(clip.x0, clip.x1) {
                self.cells[base + k] &= !mask;
            }
        }
        self.recount();
    }

    fn load_cells(&mut self, cells: &[(i64, i64)]) {
        self.cells.fill(0);
        for &(x, y) in cells {
            if let Some((mx, my)) = self.world.map(x, y) {
                self.cells[my as usize * self.words_per_row + mx as usize / 64] |= 1 << (mx % 64);
            }
        }
        self.recount();
    }

    fn export_cells(&self) -> Vec<(i64, i64)> {
        let mut out = Vec::with_capacity(self.population as usize);
        for y in 0..self.height {
            for (k, &w) in self.row(y).iter().enumerate() {
                push_cells(w, 64 * k as i64, y as i64, &mut out);
            }
        }
        out
    }

    fn cells_in_rect(&self, rect: CellRect) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        let Some(clip) = self.clip(&rect) else {
            return out;
        };
        for y in clip.y0..=clip.y1 {
            let row = self.row(y as usize);
            for (k, mask) in Self::column_words(clip.x0, clip.x1) {
                push_cells(row[k] & mask, 64 * k as i64, y, &mut out);
            }
        }
        out
    }

    fn step(&mut self) -> Result<Option<GenStats>, CaError> {
        let ghosts = self.ghosts();
        let (n, width, height) = (self.words_per_row, self.width, self.height);
        let (kernel, last_mask, cells) = (&self.kernel, self.last_mask, &self.cells);
        let threads = rayon::current_num_threads().max(1);
        let rows_per_band = height.div_ceil(threads * 4).max(1);
        let input = |y: i64| input_row(cells, &ghosts, n, height, y);
        let work = |(band, chunk): (usize, &mut [u64])| -> (u64, u64, u64) {
            let first = (band * rows_per_band) as i64;
            let mut rows = [Shifted::new(n), Shifted::new(n), Shifted::new(n)];
            let mut centres: [&[u64]; 3] = [&[], &[], &[]];
            for (slot, y) in [first - 1, first].into_iter().enumerate() {
                let (row, l, r) = input(y);
                rows[slot].fill(row, l, r, width);
                centres[slot] = row;
            }
            let (mut pop, mut births, mut deaths) = (0u64, 0u64, 0u64);
            for (i, out) in chunk.chunks_mut(n).enumerate() {
                let y = first + i as i64;
                let (below, l, r) = input(y + 1);
                let (a, m, b) = (i % 3, (i + 1) % 3, (i + 2) % 3);
                rows[b].fill(below, l, r, width);
                centres[b] = below;
                for k in 0..n {
                    let mut word = kernel.word(
                        [rows[a].west[k], centres[a][k], rows[a].east[k]],
                        [rows[m].west[k], centres[m][k], rows[m].east[k]],
                        [rows[b].west[k], centres[b][k], rows[b].east[k]],
                    );
                    if k + 1 == n {
                        word &= last_mask;
                    }
                    let old = centres[m][k];
                    pop += u64::from(word.count_ones());
                    births += u64::from((word & !old).count_ones());
                    deaths += u64::from((old & !word).count_ones());
                    out[k] = word;
                }
            }
            (pop, births, deaths)
        };
        let sum = |a: (u64, u64, u64), b: (u64, u64, u64)| (a.0 + b.0, a.1 + b.1, a.2 + b.2);
        let (pop, births, deaths) = if self.cells.len() >= PAR_MIN_WORDS {
            self.next.par_chunks_mut(rows_per_band * n).enumerate().map(work).reduce(|| (0, 0, 0), sum)
        } else {
            self.next.chunks_mut(rows_per_band * n).enumerate().map(work).fold((0, 0, 0), sum)
        };
        std::mem::swap(&mut self.cells, &mut self.next);
        self.population = pop;
        Ok(Some(GenStats { births, deaths }))
    }

    fn population(&self) -> u64 {
        self.population
    }

    fn bounding_box(&self) -> Option<CellRect> {
        if self.population == 0 {
            return None;
        }
        let mut bbox: Option<CellRect> = None;
        for y in 0..self.height {
            let row = self.row(y);
            let (Some(first), Some(last)) = (row.iter().position(|&w| w != 0), row.iter().rposition(|&w| w != 0))
            else {
                continue;
            };
            let x0 = 64 * first as i64 + i64::from(row[first].trailing_zeros());
            let x1 = 64 * last as i64 + 63 - i64::from(row[last].leading_zeros());
            let r = CellRect { x0, y0: y as i64, x1, y1: y as i64 };
            bbox = Some(bbox.map_or(r, |b| b.union(r)));
        }
        bbox
    }

    fn count_live(&self, grid: &TexelGrid, out: &mut [u32]) {
        let Some(clip) = grid.clip.and_then(|c| self.clip(&c)) else {
            return;
        };
        let width = grid.width as usize;
        for y in clip.y0..=clip.y1 {
            let row = self.row(y as usize);
            let base = grid.row(y) * width;
            let texels = &mut out[base..base + width];
            for (k, mask) in Self::column_words(clip.x0, clip.x1) {
                count_word(row[k] & mask, 64 * k as i64, grid, texels);
            }
        }
    }

    fn memory_bytes(&self) -> usize {
        (self.cells.capacity() + self.next.capacity()) * 8
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

    #[test]
    fn glider_wraps_around_a_torus_of_odd_width() {
        let world = World::new(Topology::Torus, 70, 9).unwrap();
        let mut e = BitEngine::new(world, LifeRule::conway()).unwrap();
        let glider = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
        e.load_cells(&glider);
        // 70 × 9 torus: after lcm(70, 9) × 4 generations the glider is home.
        for _ in 0..4 * 630 {
            e.step().unwrap();
        }
        let mut expected = glider.to_vec();
        expected.sort_unstable_by_key(|&(x, y)| (y, x));
        assert_eq!(e.export_cells(), expected);
        assert_eq!(e.population(), 5);
    }

    #[test]
    fn rect_operations_are_clipped() {
        let world = World::new(Topology::Bounded, 130, 3).unwrap();
        let mut e = BitEngine::new(world, LifeRule::conway()).unwrap();
        e.load_cells(&(0..130).map(|x| (x, 1)).collect::<Vec<_>>());
        assert_eq!(e.cells_in_rect(CellRect::new(60, -5, 70, 5)).len(), 11);
        e.clear_rect(CellRect::new(62, 0, 200, 2));
        assert_eq!(e.population(), 62);
        assert_eq!(e.bounding_box(), Some(CellRect::new(0, 1, 61, 1)));
    }
}
