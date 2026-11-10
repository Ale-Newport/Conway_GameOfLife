//! `DenseEngine`: one byte per cell, double buffered, rayon row bands.

use patterns::LifeRule;
use simulation_core::Topology;

use super::{EngineKind, GenStats, LifeEngine, Lut, MAX_DENSE_WINDOW_CELLS, Padded};
use crate::error::CaError;
use crate::geometry::{CellRect, World};
use crate::view::TexelGrid;

/// Smallest window side on the infinite plane.
const MIN_WINDOW: usize = 16;
/// The infinite-plane window is checked for shrinking every this many steps.
const SHRINK_EVERY: u64 = 64;

/// The simplest engine: a byte per cell (0 or 1) in a grid with a ghost
/// ring, stepped through a lookup table into a second buffer.
///
/// On finite worlds the grid is the world and the ghost ring implements the
/// topology. On the infinite plane the grid is a *window* at `origin` that
/// keeps every live cell at least one cell away from its border: before each
/// step the window grows (by a quarter of its size, at least 16 cells) when a
/// live cell touches the border, and every 64 steps it shrinks when the live
/// cells occupy less than a quarter of it. Outside the window everything is
/// dead, so the result is exactly the unbounded evolution. The window is
/// limited to [`MAX_DENSE_WINDOW_CELLS`]; beyond that `step` fails and
/// suggests the sparse or Hashlife engine.
#[derive(Clone, Debug)]
pub struct DenseEngine {
    world: World,
    rule: LifeRule,
    lut: Lut,
    grid: Padded,
    next: Padded,
    /// Cell coordinates of window cell (0, 0) (always (0, 0) on finite worlds).
    origin: (i64, i64),
    population: u64,
    steps: u64,
}

impl DenseEngine {
    /// Empty engine.
    ///
    /// # Errors
    /// See [`EngineKind::check`].
    pub fn new(world: World, rule: LifeRule) -> Result<Self, CaError> {
        EngineKind::Dense.check(&world, rule)?;
        let (w, h) =
            if world.is_finite() { (world.width as usize, world.height as usize) } else { (MIN_WINDOW, MIN_WINDOW) };
        Ok(Self {
            world,
            rule,
            lut: Lut::life(rule.birth, rule.survive),
            grid: Padded::new(w, h),
            next: Padded::new(w, h),
            origin: (0, 0),
            population: 0,
            steps: 0,
        })
    }

    fn infinite(&self) -> bool {
        !self.world.is_finite()
    }

    /// The window rectangle in cell coordinates.
    fn window(&self) -> CellRect {
        CellRect {
            x0: self.origin.0,
            y0: self.origin.1,
            x1: self.origin.0 + self.grid.width() as i64 - 1,
            y1: self.origin.1 + self.grid.height() as i64 - 1,
        }
    }

    /// Window-relative position of a cell, after topology mapping.
    fn locate(&self, x: i64, y: i64) -> Option<(usize, usize)> {
        let (x, y) = self.world.map(x, y)?;
        let (wx, wy) = (i128::from(x) - i128::from(self.origin.0), i128::from(y) - i128::from(self.origin.1));
        let inside = (0..self.grid.width() as i128).contains(&wx) && (0..self.grid.height() as i128).contains(&wy);
        inside.then_some((wx as usize, wy as usize))
    }

    /// Live cells of the window.
    fn live_bbox(&self) -> Option<CellRect> {
        let mut bbox: Option<CellRect> = None;
        for y in 0..self.grid.height() {
            let row = self.grid.row(y);
            let (Some(first), Some(last)) = (row.iter().position(|&c| c != 0), row.iter().rposition(|&c| c != 0))
            else {
                continue;
            };
            let (gy, x0, x1) = (self.origin.1 + y as i64, self.origin.0 + first as i64, self.origin.0 + last as i64);
            let r = CellRect { x0, y0: gy, x1, y1: gy };
            bbox = Some(bbox.map_or(r, |b| b.union(r)));
        }
        bbox
    }

    /// Origin and size of a window containing `want` with a margin of a
    /// quarter of its size (at least [`MIN_WINDOW`] cells) on every side.
    ///
    /// # Errors
    /// The window would leave the `i64` plane or exceed
    /// [`MAX_DENSE_WINDOW_CELLS`].
    fn window_for(want: CellRect) -> Result<(i64, i64, usize, usize), CaError> {
        let margin = |side: u64| (side / 4).max(MIN_WINDOW as u64) as i128;
        let (mw, mh) = (margin(want.width()), margin(want.height()));
        let x0 = i128::from(want.x0) - mw;
        let y0 = i128::from(want.y0) - mh;
        let w = (i128::from(want.x1) - x0 + 1 + mw).max(MIN_WINDOW as i128);
        let h = (i128::from(want.y1) - y0 + 1 + mh).max(MIN_WINDOW as i128);
        let fits = x0 >= i128::from(i64::MIN)
            && y0 >= i128::from(i64::MIN)
            && x0 + w - 1 <= i128::from(i64::MAX)
            && y0 + h - 1 <= i128::from(i64::MAX);
        if !fits {
            return Err(CaError::LimitExceeded(
                "the dense engine's window would leave the 64-bit coordinate plane; use the sparse engine".into(),
            ));
        }
        if (w as u128) * (h as u128) > u128::from(MAX_DENSE_WINDOW_CELLS) {
            return Err(CaError::LimitExceeded(format!(
                "the dense engine's window would need {w}×{h} cells (limit {MAX_DENSE_WINDOW_CELLS}); \
                 switch to the Sparse or Hashlife engine for large patterns on the infinite plane"
            )));
        }
        // Checked above: inside the i64 plane and at most 2^26 cells.
        Ok((x0 as i64, y0 as i64, w as usize, h as usize))
    }

    /// Whether live cells spread over `rect` fit in the window of the dense
    /// engine on the infinite plane (checked without allocating it).
    ///
    /// # Errors
    /// As [`DenseEngine::reserve`](LifeEngine::reserve).
    pub fn window_fits(rect: CellRect) -> Result<(), CaError> {
        Self::window_for(rect).map(|_| ())
    }

    /// Re-creates the window so that it contains `want` with a margin.
    fn rebuild_window(&mut self, want: CellRect) -> Result<(), CaError> {
        let (nx, ny, w, h) = Self::window_for(want)?;
        let mut grid = Padded::new(w, h);
        for y in 0..self.grid.height() {
            for (x, &c) in self.grid.row(y).iter().enumerate() {
                if c != 0 {
                    let gx = (self.origin.0 + x as i64 - nx) as usize;
                    let gy = (self.origin.1 + y as i64 - ny) as usize;
                    grid.set(gx, gy, 1);
                }
            }
        }
        self.grid = grid;
        self.next = Padded::new(w, h);
        self.origin = (nx, ny);
        Ok(())
    }

    /// Whether any live cell lies on the window border.
    fn touches_border(&self) -> bool {
        let (w, h) = (self.grid.width(), self.grid.height());
        let edge_rows = self.grid.row(0).iter().chain(self.grid.row(h - 1)).any(|&c| c != 0);
        edge_rows || (0..h).any(|y| self.grid.get(0, y) != 0 || self.grid.get(w - 1, y) != 0)
    }

    /// Prepares the infinite-plane window before a step.
    fn adjust_window(&mut self) -> Result<(), CaError> {
        if self.touches_border() {
            let bbox = self.live_bbox().unwrap_or(self.window());
            return self.rebuild_window(bbox);
        }
        if self.steps.is_multiple_of(SHRINK_EVERY) {
            let area = (self.grid.width() * self.grid.height()) as u128;
            match self.live_bbox() {
                Some(b) if b.area() * 4 < area && area > (4 * MIN_WINDOW * MIN_WINDOW) as u128 => {
                    self.rebuild_window(b)?
                }
                None if area > (MIN_WINDOW * MIN_WINDOW) as u128 => {
                    self.grid = Padded::new(MIN_WINDOW, MIN_WINDOW);
                    self.next = Padded::new(MIN_WINDOW, MIN_WINDOW);
                    self.origin = (0, 0);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Grows the infinite-plane window to include `rect`.
    fn ensure_contains(&mut self, rect: CellRect) -> Result<(), CaError> {
        let window = self.window();
        let inner = CellRect { x0: window.x0 + 1, y0: window.y0 + 1, x1: window.x1 - 1, y1: window.y1 - 1 };
        if inner.x0 <= rect.x0 && inner.y0 <= rect.y0 && rect.x1 <= inner.x1 && rect.y1 <= inner.y1 {
            return Ok(());
        }
        let want = match self.live_bbox() {
            Some(b) => b.union(rect),
            None => rect,
        };
        self.rebuild_window(want)
    }
}

impl LifeEngine for DenseEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::Dense
    }

    fn world(&self) -> World {
        self.world
    }

    fn rule(&self) -> LifeRule {
        self.rule
    }

    fn set_rule(&mut self, rule: LifeRule) -> Result<(), CaError> {
        EngineKind::Dense.check(&self.world, rule)?;
        self.rule = rule;
        self.lut = Lut::life(rule.birth, rule.survive);
        Ok(())
    }

    fn get_cell(&self, x: i64, y: i64) -> bool {
        self.locate(x, y).is_some_and(|(wx, wy)| self.grid.get(wx, wy) != 0)
    }

    fn reserve(&mut self, rect: CellRect) -> Result<(), CaError> {
        if self.infinite() { self.ensure_contains(rect) } else { Ok(()) }
    }

    fn set_cell(&mut self, x: i64, y: i64, alive: bool) {
        if self.infinite() && alive && self.locate(x, y).is_none() {
            // A failure leaves the window as it was and the cell is dropped;
            // callers that must not lose cells call `reserve` first.
            if self.ensure_contains(CellRect::point(x, y)).is_err() {
                return;
            }
        }
        if let Some((wx, wy)) = self.locate(x, y) {
            let old = self.grid.set(wx, wy, u8::from(alive));
            match (old != 0, alive) {
                (false, true) => self.population += 1,
                (true, false) => self.population -= 1,
                _ => {}
            }
        }
    }

    fn set_cells(&mut self, cells: &[(i64, i64)], alive: bool) {
        if self.infinite()
            && alive
            && let Some(rect) = CellRect::bounding(cells.iter().copied())
        {
            // On failure the cells outside the current window are dropped.
            let _ = self.ensure_contains(rect);
        }
        for &(x, y) in cells {
            self.set_cell(x, y, alive);
        }
    }

    fn clear(&mut self) {
        self.grid.clear();
        self.population = 0;
        if self.infinite() {
            self.grid = Padded::new(MIN_WINDOW, MIN_WINDOW);
            self.next = Padded::new(MIN_WINDOW, MIN_WINDOW);
            self.origin = (0, 0);
        }
    }

    fn clear_rect(&mut self, rect: CellRect) {
        let Some(clip) = rect.intersect(&self.window()) else {
            return;
        };
        for y in clip.y0..=clip.y1 {
            for x in clip.x0..=clip.x1 {
                let (wx, wy) = ((x - self.origin.0) as usize, (y - self.origin.1) as usize);
                if self.grid.set(wx, wy, 0) != 0 {
                    self.population -= 1;
                }
            }
        }
    }

    fn export_cells(&self) -> Vec<(i64, i64)> {
        self.cells_in_rect(self.window())
    }

    fn cells_in_rect(&self, rect: CellRect) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        let Some(clip) = rect.intersect(&self.window()) else {
            return out;
        };
        for y in clip.y0..=clip.y1 {
            let row = self.grid.row((y - self.origin.1) as usize);
            let (a, b) = ((clip.x0 - self.origin.0) as usize, (clip.x1 - self.origin.0) as usize);
            for (i, &c) in row[a..=b].iter().enumerate() {
                if c != 0 {
                    out.push((clip.x0 + i as i64, y));
                }
            }
        }
        out
    }

    fn step(&mut self) -> Result<Option<GenStats>, CaError> {
        if self.infinite() {
            self.adjust_window()?;
        }
        let topology = if self.infinite() { Topology::Bounded } else { self.world.topology };
        self.grid.fill_ghosts(topology);
        let counts = self.grid.step_into(&self.lut, &mut self.next);
        std::mem::swap(&mut self.grid, &mut self.next);
        self.population = counts.ones;
        self.steps += 1;
        Ok(Some(GenStats { births: counts.entered_one, deaths: counts.left_one }))
    }

    fn population(&self) -> u64 {
        self.population
    }

    fn bounding_box(&self) -> Option<CellRect> {
        if self.population == 0 { None } else { self.live_bbox() }
    }

    fn count_live(&self, grid: &TexelGrid, out: &mut [u32]) {
        let Some(clip) = grid.clip.and_then(|c| c.intersect(&self.window())) else {
            return;
        };
        let width = grid.width as usize;
        for y in clip.y0..=clip.y1 {
            let row = self.grid.row((y - self.origin.1) as usize);
            let base = grid.row(y) * width;
            let a = (clip.x0 - self.origin.0) as usize;
            for (i, &c) in row[a..=(clip.x1 - self.origin.0) as usize].iter().enumerate() {
                if c != 0 {
                    out[base + grid.column(clip.x0 + i as i64)] += 1;
                }
            }
        }
    }

    fn memory_bytes(&self) -> usize {
        self.grid.memory_bytes() + self.next.memory_bytes()
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
    fn glider_on_the_infinite_plane_grows_the_window() {
        let world = World::new(Topology::Infinite, 16, 16).unwrap();
        let mut e = DenseEngine::new(world, LifeRule::conway()).unwrap();
        let glider = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
        e.set_cells(&glider, true);
        assert_eq!(e.population(), 5);
        for _ in 0..400 {
            e.step().unwrap();
        }
        let moved: Vec<_> = glider.iter().map(|&(x, y)| (x + 100, y + 100)).collect();
        let mut expected = moved.clone();
        expected.sort_unstable_by_key(|&(x, y)| (y, x));
        assert_eq!(e.export_cells(), expected);
        assert!(e.window().area() < 400 * 400, "the window shrinks behind the glider");
    }

    #[test]
    fn far_apart_cells_are_refused_instead_of_dropped() {
        let world = World::new(Topology::Infinite, 16, 16).unwrap();
        let mut e = DenseEngine::new(world, LifeRule::conway()).unwrap();
        e.set_cell(0, 0, true);
        assert!(e.reserve(CellRect::new(100, 100, 110, 110)).is_ok());
        let err = e.reserve(CellRect::point(1 << 20, 1 << 20)).unwrap_err().to_string();
        assert!(err.contains("Sparse or Hashlife"), "{err}");
        assert_eq!(e.population(), 1, "a failed reservation keeps the state");
        // Switching a far-apart pattern to the dense engine fails instead of losing cells.
        let mut sparse = super::super::create_engine(EngineKind::Sparse, world, LifeRule::conway()).unwrap();
        sparse.set_cells(&[(0, 0), (1 << 20, 1 << 20)], true);
        assert!(super::super::switch_engine(sparse, EngineKind::Dense).is_err());
    }

    #[test]
    fn bounded_plane_kills_at_the_edge() {
        let world = World::new(Topology::Bounded, 4, 4).unwrap();
        let mut e = DenseEngine::new(world, LifeRule::conway()).unwrap();
        e.set_cells(&[(0, 1), (0, 2), (0, 3)], true);
        e.set_cell(-1, 0, true); // outside: ignored
        assert_eq!(e.population(), 3);
        let stats = e.step().unwrap().unwrap();
        assert_eq!(e.export_cells(), vec![(0, 2), (1, 2)]);
        assert_eq!(stats, GenStats { births: 1, deaths: 2 });
    }
}
