//! Colour storage for the ant model: a dense array on finite worlds, 64×64
//! tiles in a `BTreeMap` (ordered, so iteration is deterministic) on the
//! infinite plane.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::CaError;
use crate::geometry::{CellRect, World};
use crate::view::{GridView, GridViewRequest, RowAccumulator, TexelGrid};

const TILE: i64 = 64;

/// Cell colours of the ant world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorGrid {
    /// Finite world: `width × height` colours, row-major.
    Dense {
        /// The world.
        world: World,
        /// Colours.
        cells: Vec<u8>,
        /// Non-zero cells.
        nonzero: u64,
    },
    /// Infinite plane: tiles keyed `(ty, tx)` (row-major order).
    Tiles {
        /// The world (home region size).
        world: World,
        /// Tile contents, `(y mod 64) · 64 + (x mod 64)`.
        tiles: BTreeMap<(i64, i64), Vec<u8>>,
        /// Non-zero cells.
        nonzero: u64,
    },
}

fn tile_of(x: i64, y: i64) -> ((i64, i64), usize) {
    ((y.div_euclid(TILE), x.div_euclid(TILE)), (y.rem_euclid(TILE) * TILE + x.rem_euclid(TILE)) as usize)
}

impl ColorGrid {
    /// All-zero grid for `world`.
    pub fn new(world: World) -> Result<Self, CaError> {
        if world.is_finite() {
            if world.cell_count() > crate::multistate::MAX_MULTISTATE_CELLS {
                return Err(CaError::InvalidSize {
                    width: u64::from(world.width),
                    height: u64::from(world.height),
                    reason: "ant worlds hold at most 2^28 cells".into(),
                });
            }
            Ok(Self::Dense { world, cells: vec![0; world.cell_count() as usize], nonzero: 0 })
        } else {
            Ok(Self::Tiles { world, tiles: BTreeMap::new(), nonzero: 0 })
        }
    }

    /// The world.
    pub fn world(&self) -> World {
        match self {
            Self::Dense { world, .. } | Self::Tiles { world, .. } => *world,
        }
    }

    /// Colour of a cell in world coordinates (already mapped).
    pub fn get(&self, x: i64, y: i64) -> u8 {
        match self {
            Self::Dense { world, cells, .. } => cells[(y * i64::from(world.width) + x) as usize],
            Self::Tiles { tiles, .. } => {
                let (key, i) = tile_of(x, y);
                tiles.get(&key).map_or(0, |t| t[i])
            }
        }
    }

    /// Sets the colour of a cell in world coordinates (already mapped).
    pub fn set(&mut self, x: i64, y: i64, color: u8) {
        let (slot, nonzero) = match self {
            Self::Dense { world, cells, nonzero } => (&mut cells[(y * i64::from(world.width) + x) as usize], nonzero),
            Self::Tiles { tiles, nonzero, .. } => {
                let (key, i) = tile_of(x, y);
                if color == 0 && !tiles.contains_key(&key) {
                    return;
                }
                (&mut tiles.entry(key).or_insert_with(|| vec![0; (TILE * TILE) as usize])[i], nonzero)
            }
        };
        match (*slot != 0, color != 0) {
            (false, true) => *nonzero += 1,
            (true, false) => *nonzero -= 1,
            _ => {}
        }
        *slot = color;
    }

    /// Number of non-zero cells.
    pub fn nonzero(&self) -> u64 {
        match self {
            Self::Dense { nonzero, .. } | Self::Tiles { nonzero, .. } => *nonzero,
        }
    }

    /// Sets every cell to 0.
    pub fn clear(&mut self) {
        match self {
            Self::Dense { cells, nonzero, .. } => {
                cells.fill(0);
                *nonzero = 0;
            }
            Self::Tiles { tiles, nonzero, .. } => {
                tiles.clear();
                *nonzero = 0;
            }
        }
    }

    /// Sets the cells of `rect` (clipped to finite worlds) to 0.
    pub fn clear_rect(&mut self, rect: CellRect) {
        let Some(rect) = self.world().clip(&rect) else {
            return;
        };
        let cells: Vec<(i64, i64)> = self.nonzero_in(&rect).into_iter().map(|(x, y, _)| (x, y)).collect();
        for (x, y) in cells {
            self.set(x, y, 0);
        }
    }

    /// Non-zero cells inside `rect`, row-major.
    fn nonzero_in(&self, rect: &CellRect) -> Vec<(i64, i64, u8)> {
        let mut out = Vec::new();
        match self {
            Self::Dense { world, cells, .. } => {
                if let Some(clip) = world.clip(rect) {
                    for y in clip.y0..=clip.y1 {
                        for x in clip.x0..=clip.x1 {
                            let c = cells[(y * i64::from(world.width) + x) as usize];
                            if c != 0 {
                                out.push((x, y, c));
                            }
                        }
                    }
                }
            }
            Self::Tiles { tiles, .. } => {
                let (ty0, ty1) = (rect.y0.div_euclid(TILE), rect.y1.div_euclid(TILE));
                let (tx0, tx1) = (rect.x0.div_euclid(TILE), rect.x1.div_euclid(TILE));
                for (&(ty, tx), tile) in tiles.range((ty0, i64::MIN)..=(ty1, i64::MAX)) {
                    if !(tx0..=tx1).contains(&tx) {
                        continue;
                    }
                    let (bx, by) = (tx * TILE, ty * TILE);
                    let (c0, c1) = ((rect.x0.max(bx) - bx) as usize, (rect.x1.min(bx + TILE - 1) - bx) as usize);
                    for r in (rect.y0.max(by) - by) as usize..=(rect.y1.min(by + TILE - 1) - by) as usize {
                        for (i, &c) in tile[r * TILE as usize + c0..=r * TILE as usize + c1].iter().enumerate() {
                            if c != 0 {
                                out.push((bx + (c0 + i) as i64, by + r as i64, c));
                            }
                        }
                    }
                }
                out.sort_unstable_by_key(|&(x, y, _)| (y, x));
            }
        }
        out
    }

    /// Checks restored data: sizes and colours below `colors`.
    pub fn validate(&self, colors: u8) -> Result<(), CaError> {
        let bad = |what: &str| Err(CaError::invalid(format!("snapshot colour grid: {what}")));
        match self {
            Self::Dense { world, cells, nonzero } => {
                if !world.is_finite() || cells.len() as u64 != world.cell_count() {
                    return bad("size mismatch");
                }
                if cells.iter().any(|&c| c >= colors) || cells.iter().filter(|&&c| c != 0).count() as u64 != *nonzero {
                    return bad("invalid colours");
                }
            }
            Self::Tiles { world, tiles, nonzero } => {
                if world.is_finite() {
                    return bad("tiles on a finite world");
                }
                let mut count = 0u64;
                for tile in tiles.values() {
                    if tile.len() != (TILE * TILE) as usize || tile.iter().any(|&c| c >= colors) {
                        return bad("invalid tile");
                    }
                    count += tile.iter().filter(|&&c| c != 0).count() as u64;
                }
                if count != *nonzero {
                    return bad("population mismatch");
                }
            }
        }
        Ok(())
    }

    /// Heap bytes.
    pub fn memory_bytes(&self) -> usize {
        match self {
            Self::Dense { cells, .. } => cells.capacity(),
            Self::Tiles { tiles, .. } => tiles.len() * ((TILE * TILE) as usize + 48),
        }
    }

    /// The grid view: R = colour (most frequent non-zero colour for LOD
    /// texels), B = the strongest trail value in the texel, A = non-zero fraction.
    pub fn view(&self, request: &GridViewRequest, trail: &[(i64, i64, u8)]) -> GridView {
        let world = self.world();
        let grid = TexelGrid::plan(request, world.bounds());
        let len = grid.len();
        let (mut occupied, mut state, mut activity) = (vec![0u32; len], vec![0u8; len], vec![0u8; len]);
        if let Some(clip) = grid.clip {
            let width = grid.width as usize;
            let mut acc = RowAccumulator::new(width);
            let k = 1i64 << grid.log2_k;
            for ty in grid.row(clip.y0)..=grid.row(clip.y1) {
                let top = grid.origin_y + ty as i64 * k;
                let band = CellRect { x0: clip.x0, y0: clip.y0.max(top), x1: clip.x1, y1: clip.y1.min(top + k - 1) };
                for (x, _, c) in self.nonzero_in(&band) {
                    acc.add(grid.column(x), c);
                }
                let base = ty * width;
                acc.flush(&mut occupied[base..base + width], &mut state[base..base + width]);
            }
            for &(x, y, v) in trail {
                if clip.contains(x, y) {
                    let t = grid.index(x, y);
                    activity[t] = activity[t].max(v);
                }
            }
        }
        grid.assemble(&occupied, &state, &vec![0u8; len], &activity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::Topology;

    #[test]
    fn tiles_and_dense_agree() {
        let mut t = ColorGrid::new(World::new(Topology::Infinite, 8, 8).unwrap()).unwrap();
        let mut d = ColorGrid::new(World::new(Topology::Torus, 200, 200).unwrap()).unwrap();
        for (i, (x, y)) in [(0, 0), (63, 64), (64, 63), (5, 199), (199, 5)].into_iter().enumerate() {
            t.set(x, y, (i % 3 + 1) as u8);
            d.set(x, y, (i % 3 + 1) as u8);
        }
        t.set(500, 500, 0);
        assert_eq!(t.nonzero(), 5);
        assert_eq!(d.nonzero(), 5);
        let r = CellRect::new(0, 0, 199, 199);
        assert_eq!(t.nonzero_in(&r), d.nonzero_in(&r));
        t.clear_rect(CellRect::new(60, 60, 70, 70));
        assert_eq!(t.nonzero(), 3);
        assert!(t.validate(4).is_ok());
        assert!(t.validate(2).is_err());
        let req = GridViewRequest { x0: 0, y0: 0, x1: 199, y1: 199, max_width: 200, max_height: 200 };
        assert_eq!(d.view(&req, &[]).texel(5, 199), Some([1, 0, 0, 255]), "cell 3 got colour 3 % 3 + 1");
        assert_eq!(d.view(&req, &[(5, 199, 200)]).texel(5, 199), Some([1, 0, 200, 255]), "trail in channel B");
    }
}
