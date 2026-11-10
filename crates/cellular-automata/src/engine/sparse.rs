//! `SparseEngine`: 64×64 bit tiles in a hash map with an active-tile set.

use std::collections::{BTreeSet, HashMap};

use patterns::LifeRule;
use rayon::prelude::*;
use simulation_core::Topology;

use super::swar::{Kernel, bit_range, count_word, push_cells};
use super::{EngineKind, GenStats, LifeEngine};
use crate::error::CaError;
use crate::geometry::{CellRect, World};
use crate::view::TexelGrid;

/// Tile key `(tx, ty)`: the tile covering `[64·tx, 64·tx + 63] × [64·ty, 64·ty + 63]`.
type Key = (i64, i64);

/// Smallest and largest tile coordinate: tiles beyond them would hold cells
/// outside the `i64` plane.
const KEY_MIN: i64 = i64::MIN >> 6;
const KEY_MAX: i64 = i64::MAX >> 6;
/// Candidate sets at least this large are stepped in parallel.
const PAR_MIN_TILES: usize = 32;

/// 64 rows of 64 cells; bit `i` of `rows[r]` is the cell `(64·tx + i, 64·ty + r)`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tile {
    rows: [u64; 64],
    population: u32,
}

impl Tile {
    fn empty() -> Self {
        Self { rows: [0; 64], population: 0 }
    }

    fn recount(&mut self) {
        self.population = self.rows.iter().map(|r| r.count_ones()).sum();
    }
}

static EMPTY: Tile = Tile { rows: [0; 64], population: 0 };

fn split(x: i64, y: i64) -> (Key, usize, u32) {
    ((x.div_euclid(64), y.div_euclid(64)), y.rem_euclid(64) as usize, x.rem_euclid(64) as u32)
}

fn valid_key((tx, ty): Key) -> bool {
    (KEY_MIN..=KEY_MAX).contains(&tx) && (KEY_MIN..=KEY_MAX).contains(&ty)
}

/// Sort order used for every iteration: row-major by tile.
fn row_major(key: &Key) -> (i64, i64) {
    (key.1, key.0)
}

/// Unbounded engine storing only non-empty 64×64 tiles.
///
/// A tile is recomputed in a generation only if it or one of its eight
/// neighbours changed in the previous generation (or was edited): otherwise
/// its 3×3 tile neighbourhood is the same as one generation earlier, so its
/// next state equals its current state. Still lifes therefore cost nothing
/// and only the active area is stepped. Candidate tiles are computed in
/// parallel as pure functions of the old tiles and applied in sorted key
/// order; tiles that become empty are freed. Iteration (export, views) always
/// uses sorted keys, never hash-map order.
///
/// The plane is the whole `i64` range; cells that would be born beyond it
/// are discarded (the tiles at the edge of the range behave like a bounded
/// plane).
#[derive(Clone, Debug)]
pub struct SparseEngine {
    rule: LifeRule,
    kernel: Kernel,
    tiles: HashMap<Key, Box<Tile>>,
    /// Tiles changed in the last generation or edited since.
    dirty: BTreeSet<Key>,
    population: u64,
}

impl SparseEngine {
    /// Empty engine.
    ///
    /// # Errors
    /// Rules with `B0`.
    pub fn new(rule: LifeRule) -> Result<Self, CaError> {
        EngineKind::Sparse.check(&Self::plane(), rule)?;
        Ok(Self { rule, kernel: Kernel::new(rule), tiles: HashMap::new(), dirty: BTreeSet::new(), population: 0 })
    }

    fn plane() -> World {
        World { topology: Topology::Infinite, width: 256, height: 256 }
    }

    /// Number of allocated (non-empty) tiles.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    fn tile(&self, key: Key) -> &Tile {
        self.tiles.get(&key).map_or(&EMPTY, |t| t)
    }

    /// Sorted keys of the tiles intersecting `rect`.
    fn keys_in(&self, rect: &CellRect) -> Vec<Key> {
        let (kx0, ky0) = (rect.x0.div_euclid(64), rect.y0.div_euclid(64));
        let (kx1, ky1) = (rect.x1.div_euclid(64), rect.y1.div_euclid(64));
        let span = (i128::from(kx1) - i128::from(kx0) + 1) * (i128::from(ky1) - i128::from(ky0) + 1);
        let mut keys: Vec<Key> = if span <= self.tiles.len() as i128 {
            (ky0..=ky1)
                .flat_map(|ty| (kx0..=kx1).map(move |tx| (tx, ty)))
                .filter(|k| self.tiles.contains_key(k))
                .collect()
        } else {
            self.tiles
                .keys()
                .copied()
                .filter(|&(tx, ty)| (kx0..=kx1).contains(&tx) && (ky0..=ky1).contains(&ty))
                .collect()
        };
        keys.sort_unstable_by_key(row_major);
        keys
    }

    /// Next state of the tile `key` (a pure function of the 3×3 old tiles).
    fn compute(&self, key: Key) -> Option<Tile> {
        let (tx, ty) = key;
        let get = |dx: i64, dy: i64| self.tile((tx + dx, ty + dy));
        let around =
            [get(-1, -1), get(0, -1), get(1, -1), get(-1, 0), get(0, 0), get(1, 0), get(-1, 1), get(0, 1), get(1, 1)];
        if around.iter().all(|t| t.population == 0) {
            return None;
        }
        // Rows −1..=64 of the neighbourhood: centre word plus west/east shifts.
        let mut west = [0u64; 66];
        let mut centre = [0u64; 66];
        let mut east = [0u64; 66];
        for r in 0..66usize {
            let (row, band) = match r {
                0 => (63, 0),
                65 => (0, 2),
                _ => (r - 1, 1),
            };
            let (w, c, e) =
                (around[band * 3].rows[row], around[band * 3 + 1].rows[row], around[band * 3 + 2].rows[row]);
            centre[r] = c;
            west[r] = (c << 1) | (w >> 63);
            east[r] = (c >> 1) | (e << 63);
        }
        let mut out = Tile::empty();
        for r in 0..64 {
            out.rows[r] = self.kernel.word(
                [west[r], centre[r], east[r]],
                [west[r + 1], centre[r + 1], east[r + 1]],
                [west[r + 2], centre[r + 2], east[r + 2]],
            );
        }
        out.recount();
        Some(out)
    }

    fn mark(&mut self, key: Key) {
        self.dirty.insert(key);
    }
}

/// Outcome of stepping one candidate tile.
struct Update {
    key: Key,
    tile: Option<Tile>,
    births: u64,
    deaths: u64,
}

impl LifeEngine for SparseEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Sparse
    }

    fn world(&self) -> World {
        Self::plane()
    }

    fn rule(&self) -> LifeRule {
        self.rule
    }

    fn set_rule(&mut self, rule: LifeRule) -> Result<(), CaError> {
        EngineKind::Sparse.check(&Self::plane(), rule)?;
        self.rule = rule;
        self.kernel = Kernel::new(rule);
        // Every tile may now evolve differently.
        let keys: Vec<Key> = self.tiles.keys().copied().collect();
        self.dirty.extend(keys);
        Ok(())
    }

    fn get_cell(&self, x: i64, y: i64) -> bool {
        let (key, row, bit) = split(x, y);
        self.tile(key).rows[row] >> bit & 1 == 1
    }

    fn set_cell(&mut self, x: i64, y: i64, alive: bool) {
        let (key, row, bit) = split(x, y);
        let mask = 1u64 << bit;
        if alive {
            let tile = self.tiles.entry(key).or_insert_with(|| Box::new(Tile::empty()));
            if tile.rows[row] & mask == 0 {
                tile.rows[row] |= mask;
                tile.population += 1;
                self.population += 1;
                self.mark(key);
            }
        } else if let Some(tile) = self.tiles.get_mut(&key)
            && tile.rows[row] & mask != 0
        {
            tile.rows[row] &= !mask;
            tile.population -= 1;
            self.population -= 1;
            if tile.population == 0 {
                self.tiles.remove(&key);
            }
            self.mark(key);
        }
    }

    fn clear(&mut self) {
        self.tiles.clear();
        self.dirty.clear();
        self.population = 0;
    }

    fn clear_rect(&mut self, rect: CellRect) {
        for key in self.keys_in(&rect) {
            let Some(tile) = self.tiles.get_mut(&key) else {
                continue;
            };
            let (bx, by) = (key.0 * 64, key.1 * 64);
            let lo = (rect.x0.max(bx) - bx) as u32;
            let hi = (rect.x1.min(bx + 63) - bx) as u32;
            let mask = bit_range(lo, hi);
            let (r0, r1) = ((rect.y0.max(by) - by) as usize, (rect.y1.min(by + 63) - by) as usize);
            for row in &mut tile.rows[r0..=r1] {
                *row &= !mask;
            }
            let before = tile.population;
            tile.recount();
            self.population -= u64::from(before - tile.population);
            if tile.population == 0 {
                self.tiles.remove(&key);
            }
            self.mark(key);
        }
    }

    fn export_cells(&self) -> Vec<(i64, i64)> {
        let mut keys: Vec<Key> = self.tiles.keys().copied().collect();
        keys.sort_unstable_by_key(row_major);
        let mut out = Vec::with_capacity(self.population as usize);
        for band in keys.chunk_by(|a, b| a.1 == b.1) {
            for r in 0..64 {
                for &key in band {
                    push_cells(self.tile(key).rows[r], key.0 * 64, key.1 * 64 + r as i64, &mut out);
                }
            }
        }
        out
    }

    fn cells_in_rect(&self, rect: CellRect) -> Vec<(i64, i64)> {
        let keys = self.keys_in(&rect);
        let mut out = Vec::new();
        for band in keys.chunk_by(|a, b| a.1 == b.1) {
            let by = band[0].1 * 64;
            let (r0, r1) = ((rect.y0.max(by) - by) as usize, (rect.y1.min(by + 63) - by) as usize);
            for r in r0..=r1 {
                for &key in band {
                    let bx = key.0 * 64;
                    let mask = bit_range((rect.x0.max(bx) - bx) as u32, (rect.x1.min(bx + 63) - bx) as u32);
                    push_cells(self.tile(key).rows[r] & mask, bx, by + r as i64, &mut out);
                }
            }
        }
        out
    }

    fn count_in_rect(&self, rect: CellRect, limit: u64) -> u64 {
        let mut count = 0;
        for key in self.keys_in(&rect) {
            if count >= limit {
                break;
            }
            let (bx, by) = (key.0 * 64, key.1 * 64);
            let tile = self.tile(key);
            count += if rect.contains(bx, by) && rect.contains(bx + 63, by + 63) {
                u64::from(tile.population)
            } else {
                let mask = bit_range((rect.x0.max(bx) - bx) as u32, (rect.x1.min(bx + 63) - bx) as u32);
                let (r0, r1) = ((rect.y0.max(by) - by) as usize, (rect.y1.min(by + 63) - by) as usize);
                tile.rows[r0..=r1].iter().map(|&row| u64::from((row & mask).count_ones())).sum()
            };
        }
        count.min(limit)
    }

    fn step(&mut self) -> Result<Option<GenStats>, CaError> {
        let mut candidates: BTreeSet<Key> = BTreeSet::new();
        for &(tx, ty) in &self.dirty {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if let (Some(x), Some(y)) = (tx.checked_add(dx), ty.checked_add(dy))
                        && valid_key((x, y))
                    {
                        candidates.insert((x, y));
                    }
                }
            }
        }
        let candidates: Vec<Key> = candidates.into_iter().collect();
        let this = &*self;
        let work = |&key: &Key| -> Option<Update> {
            let old = this.tile(key);
            let new = this.compute(key);
            let new_rows = new.as_ref().map_or(&EMPTY.rows, |t| &t.rows);
            if *new_rows == old.rows {
                return None;
            }
            let (mut births, mut deaths) = (0u64, 0u64);
            for (n, o) in new_rows.iter().zip(&old.rows) {
                births += u64::from((n & !o).count_ones());
                deaths += u64::from((o & !n).count_ones());
            }
            Some(Update { key, tile: new.filter(|t| t.population > 0), births, deaths })
        };
        let updates: Vec<Update> = if candidates.len() >= PAR_MIN_TILES {
            candidates.par_iter().filter_map(work).collect()
        } else {
            candidates.iter().filter_map(work).collect()
        };
        let mut stats = GenStats::default();
        let mut dirty = BTreeSet::new();
        for update in updates {
            stats.births += update.births;
            stats.deaths += update.deaths;
            dirty.insert(update.key);
            match update.tile {
                Some(tile) => {
                    self.tiles.insert(update.key, Box::new(tile));
                }
                None => {
                    self.tiles.remove(&update.key);
                }
            }
        }
        self.population = self.population + stats.births - stats.deaths;
        self.dirty = dirty;
        Ok(Some(stats))
    }

    fn population(&self) -> u64 {
        self.population
    }

    fn bounding_box(&self) -> Option<CellRect> {
        let mut bbox: Option<CellRect> = None;
        for (&(tx, ty), tile) in &self.tiles {
            let (Some(first), Some(last)) =
                (tile.rows.iter().position(|&r| r != 0), tile.rows.iter().rposition(|&r| r != 0))
            else {
                continue;
            };
            let ors = tile.rows.iter().fold(0u64, |a, &r| a | r);
            let r = CellRect {
                x0: tx * 64 + i64::from(ors.trailing_zeros()),
                y0: ty * 64 + first as i64,
                x1: tx * 64 + 63 - i64::from(ors.leading_zeros()),
                y1: ty * 64 + last as i64,
            };
            // Order-independent: the union of rectangles is commutative.
            bbox = Some(bbox.map_or(r, |b| b.union(r)));
        }
        bbox
    }

    fn count_live(&self, grid: &TexelGrid, out: &mut [u32]) {
        let Some(clip) = grid.clip else {
            return;
        };
        let width = grid.width as usize;
        for key in self.keys_in(&clip) {
            let (bx, by) = (key.0 * 64, key.1 * 64);
            let mask = bit_range((clip.x0.max(bx) - bx) as u32, (clip.x1.min(bx + 63) - bx) as u32);
            let (r0, r1) = ((clip.y0.max(by) - by) as usize, (clip.y1.min(by + 63) - by) as usize);
            let tile = self.tile(key);
            for r in r0..=r1 {
                let base = grid.row(by + r as i64) * width;
                count_word(tile.rows[r] & mask, bx, grid, &mut out[base..base + width]);
            }
        }
    }

    fn memory_bytes(&self) -> usize {
        let per_tile = std::mem::size_of::<Tile>() + std::mem::size_of::<(Key, Box<Tile>)>() + 8;
        self.tiles.capacity() * per_tile + self.dirty.len() * std::mem::size_of::<Key>() * 2
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
    fn glider_crosses_tiles_and_old_tiles_are_freed() {
        let mut e = SparseEngine::new(LifeRule::conway()).unwrap();
        let glider = [(61, 61), (62, 62), (60, 63), (61, 63), (62, 63)];
        e.set_cells(&glider, true);
        for _ in 0..4 * 64 {
            e.step().unwrap();
        }
        let mut expected: Vec<_> = glider.iter().map(|&(x, y)| (x + 64, y + 64)).collect();
        expected.sort_unstable_by_key(|&(x, y)| (y, x));
        assert_eq!(e.export_cells(), expected);
        assert!(e.tile_count() <= 4, "tiles left behind are freed");
    }

    #[test]
    fn still_lifes_go_quiet() {
        let mut e = SparseEngine::new(LifeRule::conway()).unwrap();
        e.set_cells(&[(-1, -1), (0, -1), (-1, 0), (0, 0)], true);
        e.step().unwrap();
        assert!(e.dirty.is_empty(), "a block changes nothing, so nothing stays active");
        assert_eq!(e.population(), 4);
        assert_eq!(e.bounding_box(), Some(CellRect::new(-1, -1, 0, 0)));
    }

    #[test]
    fn negative_coordinates_and_rect_queries() {
        let mut e = SparseEngine::new(LifeRule::conway()).unwrap();
        let cells = [(-65, -3), (-64, -3), (0, 0), (63, 64), (1 << 40, -(1 << 40))];
        e.set_cells(&cells, true);
        assert_eq!(e.cells_in_rect(CellRect::new(-70, -5, 0, 0)), vec![(-65, -3), (-64, -3), (0, 0)]);
        e.clear_rect(CellRect::new(-64, -64, 63, 63));
        assert_eq!(e.population(), 3);
        assert!(!e.get_cell(0, 0) && e.get_cell(-65, -3) && e.get_cell(63, 64));
    }
}
