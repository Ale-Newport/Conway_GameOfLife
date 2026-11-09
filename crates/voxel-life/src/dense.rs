//! Dense `u8` voxel grid with double buffering and rayon over z-planes.
//!
//! Layout: `index = (z · H + y) · W + x`. One generation is computed in two
//! parallel passes, each a pure function of read-only buffers:
//!
//! 1. (Moore only) every z-plane's 3×3 box sums of alive voxels (`sums`),
//!    computed separably along x then y;
//! 2. every output plane `z` reads `sums` of planes `z−1, z, z+1` (Moore) or
//!    the six face neighbours (von Neumann), applies the rule into the back
//!    buffer and updates ages in place (an age depends only on the voxel's
//!    own old and new state).
//!
//! The buffers are then swapped. On a torus, neighbour offsets are taken
//! modulo the size of each axis, so along an axis shorter than 3 the same
//! voxel is reached through several offsets and is counted once per offset.

use rayon::prelude::*;

use crate::geometry::Pos;
use crate::rule::{Neighbourhood, Transition};
use crate::stats::Stats;

#[inline]
fn alive(state: u8) -> u8 {
    u8::from(state == 1)
}

/// Dense voxel storage.
#[derive(Clone, Debug)]
pub(crate) struct DenseGrid {
    size: [usize; 3],
    torus: bool,
    cells: Vec<u8>,
    ages: Vec<u16>,
    back: Vec<u8>,
    sums: Vec<u8>,
}

impl DenseGrid {
    /// Empty grid. `size` must be validated by the caller.
    pub(crate) fn new(size: [u32; 3], torus: bool) -> Self {
        let size = size.map(|s| s as usize);
        let volume = size[0] * size[1] * size[2];
        Self { size, torus, cells: vec![0; volume], ages: vec![0; volume], back: vec![0; volume], sums: Vec::new() }
    }

    /// Grid with the given contents; lengths must equal the volume.
    pub(crate) fn from_parts(size: [u32; 3], torus: bool, cells: Vec<u8>, ages: Vec<u16>) -> Option<Self> {
        let mut grid = Self::new(size, torus);
        if cells.len() != grid.cells.len() || ages.len() != grid.ages.len() {
            return None;
        }
        grid.cells = cells;
        grid.ages = ages;
        Some(grid)
    }

    #[inline]
    fn index(&self, pos: Pos) -> usize {
        let [nx, ny, _] = self.size;
        (pos[2] as usize * ny + pos[1] as usize) * nx + pos[0] as usize
    }

    #[inline]
    fn inside(&self, pos: Pos) -> bool {
        (0..3).all(|a| pos[a] >= 0 && (pos[a] as usize) < self.size[a])
    }

    /// State and age at `pos` (empty outside the box).
    pub(crate) fn get(&self, pos: Pos) -> (u8, u16) {
        if self.inside(pos) {
            let i = self.index(pos);
            (self.cells[i], self.ages[i])
        } else {
            (0, 0)
        }
    }

    /// State at `pos` (0 outside the box).
    #[inline]
    pub(crate) fn state(&self, pos: Pos) -> u8 {
        if self.inside(pos) { self.cells[self.index(pos)] } else { 0 }
    }

    /// Writes a voxel inside the box and resets its age.
    pub(crate) fn set(&mut self, pos: Pos, state: u8) {
        if self.inside(pos) {
            let i = self.index(pos);
            self.cells[i] = state;
            self.ages[i] = 0;
        }
    }

    /// Empties every voxel.
    pub(crate) fn clear(&mut self) {
        self.cells.fill(0);
        self.ages.fill(0);
    }

    /// Empties voxels whose state is `states` or more.
    pub(crate) fn clamp_states(&mut self, states: u16) {
        for (cell, age) in self.cells.iter_mut().zip(&mut self.ages) {
            if u16::from(*cell) >= states {
                *cell = 0;
                *age = 0;
            }
        }
    }

    /// Size along each axis.
    pub(crate) fn size(&self) -> [usize; 3] {
        self.size
    }

    /// States in layout order.
    pub(crate) fn cells(&self) -> &[u8] {
        &self.cells
    }

    /// Ages in layout order.
    pub(crate) fn ages(&self) -> &[u16] {
        &self.ages
    }

    /// Bytes held by the buffers.
    pub(crate) fn memory_bytes(&self) -> usize {
        self.cells.capacity() + self.back.capacity() + self.sums.capacity() + 2 * self.ages.capacity()
    }

    /// Plane index `z + delta` with the grid's edge rule.
    fn neighbour_plane(&self, z: usize, delta: isize) -> Option<usize> {
        let nz = self.size[2];
        let target = z as isize + delta;
        if (0..nz as isize).contains(&target) {
            Some(target as usize)
        } else if self.torus {
            Some(target.rem_euclid(nz as isize) as usize)
        } else {
            None
        }
    }

    /// Advances one generation and returns the new statistics.
    pub(crate) fn step(&mut self, transition: &Transition, neighbourhood: Neighbourhood, states: u16) -> Stats {
        let [nx, ny, nz] = self.size;
        let plane = nx * ny;
        let torus = self.torus;
        let planes: Vec<(Option<usize>, Option<usize>)> =
            (0..nz).map(|z| (self.neighbour_plane(z, -1), self.neighbour_plane(z, 1))).collect();
        if neighbourhood == Neighbourhood::Moore {
            self.sums.resize(self.cells.len(), 0);
            let cells = &self.cells;
            self.sums.par_chunks_mut(plane).enumerate().for_each_init(
                || vec![0u8; plane],
                |row_sums, (z, out)| plane_box_sums(&cells[z * plane..(z + 1) * plane], nx, ny, torus, row_sums, out),
            );
        }
        let zeros = vec![0u8; plane];
        let Self { cells, ages, back, sums, .. } = self;
        let (cells, sums) = (&*cells, &*sums);
        let parts: Vec<Stats> = back
            .par_chunks_mut(plane)
            .zip(ages.par_chunks_mut(plane))
            .enumerate()
            .map(|(z, (next, ages))| {
                let (below, above) = planes[z];
                let own = &cells[z * plane..(z + 1) * plane];
                let ctx = PlaneStep { nx, ny, z, torus, transition };
                let mut stats = Stats::new(states);
                match neighbourhood {
                    Neighbourhood::Moore => {
                        let around = [
                            layer(sums, &zeros, plane, below),
                            layer(sums, &zeros, plane, Some(z)),
                            layer(sums, &zeros, plane, above),
                        ];
                        ctx.moore(own, around, next, ages, &mut stats);
                    }
                    Neighbourhood::VonNeumann => {
                        let around = [layer(cells, &zeros, plane, below), layer(cells, &zeros, plane, above)];
                        ctx.von_neumann(own, around, next, ages, &mut stats);
                    }
                }
                stats
            })
            .collect();
        std::mem::swap(&mut self.cells, &mut self.back);
        Stats::combine(states, parts)
    }

    /// Statistics of the current state (no transitions), scanned in parallel.
    pub(crate) fn stats(&self, states: u16) -> Stats {
        let [nx, ny, _] = self.size;
        let plane = nx * ny;
        let parts: Vec<Stats> = self
            .cells
            .par_chunks(plane)
            .enumerate()
            .map(|(z, cells)| {
                let mut stats = Stats::new(states);
                for (i, &state) in cells.iter().enumerate() {
                    if state != 0 {
                        stats.add([(i % nx) as i32, (i / nx) as i32, z as i32], state);
                    }
                }
                stats
            })
            .collect();
        Stats::combine(states, parts)
    }
}

/// Plane `p` of `source`, or the all-empty plane `zeros` when `p` lies
/// outside a bounded world.
fn layer<'s>(source: &'s [u8], zeros: &'s [u8], plane: usize, p: Option<usize>) -> &'s [u8] {
    p.map_or(zeros, |p| &source[p * plane..(p + 1) * plane])
}

/// 3×3 box sums of alive voxels of one plane, separably along x then y.
fn plane_box_sums(src: &[u8], nx: usize, ny: usize, torus: bool, row_sums: &mut [u8], out: &mut [u8]) {
    for y in 0..ny {
        let row = &src[y * nx..(y + 1) * nx];
        let sums = &mut row_sums[y * nx..(y + 1) * nx];
        for x in 0..nx {
            let left = if x > 0 {
                alive(row[x - 1])
            } else if torus {
                alive(row[nx - 1])
            } else {
                0
            };
            let right = if x + 1 < nx {
                alive(row[x + 1])
            } else if torus {
                alive(row[0])
            } else {
                0
            };
            sums[x] = left + alive(row[x]) + right;
        }
    }
    for y in 0..ny {
        let up = if y > 0 {
            Some(y - 1)
        } else if torus {
            Some(ny - 1)
        } else {
            None
        };
        let down = if y + 1 < ny {
            Some(y + 1)
        } else if torus {
            Some(0)
        } else {
            None
        };
        for x in 0..nx {
            let neighbour = |row: Option<usize>| row.map_or(0, |r| row_sums[r * nx + x]);
            out[y * nx + x] = row_sums[y * nx + x] + neighbour(up) + neighbour(down);
        }
    }
}

/// Per-plane context of the second pass.
struct PlaneStep<'a> {
    nx: usize,
    ny: usize,
    z: usize,
    torus: bool,
    transition: &'a Transition,
}

impl PlaneStep<'_> {
    #[inline]
    fn finish(&self, i: usize, old: u8, count: u8, next: &mut [u8], ages: &mut [u16], stats: &mut Stats) {
        let new = self.transition.apply(old, count);
        next[i] = new;
        stats.transition(old, new);
        if new == 0 {
            ages[i] = 0;
        } else {
            ages[i] = if old == 0 { 0 } else { ages[i].saturating_add(1) };
            stats.add([(i % self.nx) as i32, (i / self.nx) as i32, self.z as i32], new);
        }
    }

    /// Moore count = box sums of planes z−1, z, z+1 minus the voxel itself.
    fn moore(&self, own: &[u8], sums: [&[u8]; 3], next: &mut [u8], ages: &mut [u16], stats: &mut Stats) {
        for i in 0..own.len() {
            let old = own[i];
            let count = sums[0][i] + sums[1][i] + sums[2][i] - alive(old);
            self.finish(i, old, count, next, ages, stats);
        }
    }

    /// Von Neumann count: the six face neighbours.
    fn von_neumann(&self, own: &[u8], planes: [&[u8]; 2], next: &mut [u8], ages: &mut [u16], stats: &mut Stats) {
        let (nx, ny, torus) = (self.nx, self.ny, self.torus);
        for y in 0..ny {
            let row = y * nx;
            let up = if y > 0 {
                Some(row - nx)
            } else if torus {
                Some((ny - 1) * nx)
            } else {
                None
            };
            let down = if y + 1 < ny {
                Some(row + nx)
            } else if torus {
                Some(0)
            } else {
                None
            };
            for x in 0..nx {
                let i = row + x;
                let left = if x > 0 {
                    alive(own[i - 1])
                } else if torus {
                    alive(own[row + nx - 1])
                } else {
                    0
                };
                let right = if x + 1 < nx {
                    alive(own[i + 1])
                } else if torus {
                    alive(own[row])
                } else {
                    0
                };
                let vertical = up.map_or(0, |r| alive(own[r + x])) + down.map_or(0, |r| alive(own[r + x]));
                let count = left + right + vertical + alive(planes[0][i]) + alive(planes[1][i]);
                self.finish(i, own[i], count, next, ages, stats);
            }
        }
    }
}
