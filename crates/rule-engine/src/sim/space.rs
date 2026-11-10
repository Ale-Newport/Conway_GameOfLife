//! Continuous positions on a finite world with any [`Topology`].
//!
//! Entities live at `Vec2` positions in world units (one unit = one field
//! cell). Movement constraints and neighbour queries follow the topology:
//!
//! * **Bounded** — positions are reflected at the edges
//!   ([`Bounds::Reflect`] from organism-engine); queries see no images.
//! * **Torus** — positions wrap ([`Bounds::Wrap`]); queries use
//!   organism-engine's wrap-aware perception (nearest periodic image).
//! * **Klein bottle / cross-surface** — positions wrap with the same edge
//!   identifications as [`Topology::wrap`] on cells (crossing the top/bottom
//!   edge mirrors x; on the cross-surface crossing the left/right edge also
//!   mirrors y), and velocities/headings are mirrored with them. Queries
//!   visit the eight neighbouring images `g(p)` of the fundamental square,
//!   where each `g` is an isometry (translation plus mirror), by querying
//!   the spatial grid around `g⁻¹(origin)`; an entity seen through several
//!   images is reported once, at its nearest image.

use organism_engine::kinematics::{Bounds, Kinematics};
use organism_engine::perception::{self, Percept, Query};
use simulation_core::glam::Vec2;
use simulation_core::{SpatialGrid, Topology};

/// World geometry for continuous positions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Space {
    /// Edge identification.
    pub topology: Topology,
    /// Width in world units.
    pub width: f32,
    /// Height in world units.
    pub height: f32,
}

/// One of the nine images of the fundamental square: `p ↦ (fx(p.x) + kx·W,
/// fy(p.y) + ky·H)` where `fx`/`fy` mirror when `flip_x`/`flip_y`.
#[derive(Clone, Copy, Debug)]
struct Image {
    shift: Vec2,
    flip_x: bool,
    flip_y: bool,
}

impl Space {
    /// Geometry of a `width × height` world.
    pub fn new(topology: Topology, width: u32, height: u32) -> Self {
        Self { topology, width: width as f32, height: height as f32 }
    }

    /// organism-engine bounds for the simple topologies (twisted ones are
    /// handled by [`Space::constrain`] itself).
    pub fn bounds(&self) -> Bounds {
        let max = Vec2::new(self.width, self.height);
        match self.topology {
            Topology::Bounded => Bounds::Reflect { min: Vec2::ZERO, max },
            Topology::Torus => Bounds::Wrap { min: Vec2::ZERO, max },
            _ => Bounds::Unbounded,
        }
    }

    fn twisted(&self) -> bool {
        matches!(self.topology, Topology::KleinBottle | Topology::CrossSurface)
    }

    /// Maps an integrated position back into the world, mirroring velocity
    /// and heading when a twisted edge was crossed.
    pub fn constrain(&self, kin: &mut Kinematics) {
        if !self.twisted() {
            let (p, v) = self.bounds().apply(kin.position, kin.velocity);
            kin.position = p;
            kin.velocity = v;
            return;
        }
        let (w, h) = (self.width, self.height);
        let kx = (kin.position.x / w).floor();
        let ky = (kin.position.y / h).floor();
        let mut x = kin.position.x - kx * w;
        let mut y = kin.position.y - ky * h;
        let odd = |k: f32| (k.rem_euclid(2.0) - 1.0).abs() < 0.5;
        let flip_x = odd(ky);
        let flip_y = self.topology == Topology::CrossSurface && odd(kx);
        if flip_x {
            x = w - x;
            kin.velocity.x = -kin.velocity.x;
            kin.heading = std::f32::consts::PI - kin.heading;
        }
        if flip_y {
            y = h - y;
            kin.velocity.y = -kin.velocity.y;
            kin.heading = -kin.heading;
        }
        // Rounding can land exactly on the far edge, which is the near edge.
        if !(0.0..w).contains(&x) {
            x = 0.0;
        }
        if !(0.0..h).contains(&y) {
            y = 0.0;
        }
        kin.position = Vec2::new(x, y);
    }

    /// Maps an arbitrary point (e.g. a command position or an offspring
    /// offset) into the world.
    pub fn place(&self, p: Vec2) -> Vec2 {
        let mut kin = Kinematics::new(p, 0.0);
        self.constrain(&mut kin);
        kin.position
    }

    /// Integer cell of a position (clamped into the grid).
    pub fn cell(&self, p: Vec2) -> (usize, usize) {
        let cx = (p.x.floor().max(0.0) as usize).min(self.width as usize - 1);
        let cy = (p.y.floor().max(0.0) as usize).min(self.height as usize - 1);
        (cx, cy)
    }

    fn images(&self) -> Vec<Image> {
        let mut out = Vec::with_capacity(9);
        for ky in -1i32..=1 {
            for kx in -1i32..=1 {
                out.push(Image {
                    shift: Vec2::new(kx as f32 * self.width, ky as f32 * self.height),
                    flip_x: ky != 0,
                    flip_y: self.topology == Topology::CrossSurface && kx != 0,
                });
            }
        }
        out
    }

    fn apply_image(&self, img: &Image, p: Vec2) -> Vec2 {
        let x = if img.flip_x { self.width - p.x } else { p.x };
        let y = if img.flip_y { self.height - p.y } else { p.y };
        Vec2::new(x, y) + img.shift
    }

    fn invert_image(&self, img: &Image, q: Vec2) -> Vec2 {
        let d = q - img.shift;
        Vec2::new(if img.flip_x { self.width - d.x } else { d.x }, if img.flip_y { self.height - d.y } else { d.y })
    }

    /// All items within `radius` of `origin`, each once at its nearest
    /// image, sorted by distance then slot.
    pub fn within(&self, grid: &SpatialGrid, origin: Vec2, radius: f32, out: &mut Vec<Percept>) {
        out.clear();
        if !self.twisted() {
            let query = Query::new(origin, radius).in_world(self.bounds());
            perception::within(grid, &query, |_, _| true, out);
            return;
        }
        if radius.is_nan() || radius < 0.0 {
            return;
        }
        for img in self.images() {
            let centre = self.invert_image(&img, origin);
            // Skip images whose query disc misses the world rectangle.
            let near_x = centre.x.clamp(0.0, self.width);
            let near_y = centre.y.clamp(0.0, self.height);
            if Vec2::new(near_x, near_y).distance_squared(centre) > radius * radius {
                continue;
            }
            grid.for_each_within(centre, radius, |slot, p, d2| {
                out.push(Percept { slot, position: self.apply_image(&img, p), distance: d2.sqrt() });
            });
        }
        out.sort_unstable_by(|a, b| a.slot.cmp(&b.slot).then(a.distance.total_cmp(&b.distance)));
        out.dedup_by_key(|p| p.slot);
        out.sort_unstable_by(|a, b| a.distance.total_cmp(&b.distance).then(a.slot.cmp(&b.slot)));
    }

    /// Nearest item within `radius` accepted by `filter(slot, image
    /// position)`; ties go to the lower slot.
    pub fn nearest(
        &self,
        grid: &SpatialGrid,
        origin: Vec2,
        radius: f32,
        mut filter: impl FnMut(u32, Vec2) -> bool,
    ) -> Option<Percept> {
        if !self.twisted() {
            let query = Query::new(origin, radius).in_world(self.bounds());
            return perception::nearest(grid, &query, filter);
        }
        let mut found = Vec::new();
        self.within(grid, origin, radius, &mut found);
        found.into_iter().find(|p| filter(p.slot, p.position))
    }

    /// Number of items within `radius` accepted by `filter`.
    pub fn count(
        &self,
        grid: &SpatialGrid,
        origin: Vec2,
        radius: f32,
        mut filter: impl FnMut(u32, Vec2) -> bool,
    ) -> usize {
        if !self.twisted() {
            let query = Query::new(origin, radius).in_world(self.bounds());
            return perception::count(grid, &query, filter);
        }
        let mut found = Vec::new();
        self.within(grid, origin, radius, &mut found);
        found.into_iter().filter(|p| filter(p.slot, p.position)).count()
    }

    /// Shortest displacement from `from` to `to` (nearest image).
    pub fn delta(&self, from: Vec2, to: Vec2) -> Vec2 {
        if !self.twisted() {
            return self.bounds().delta(from, to);
        }
        self.images()
            .iter()
            .map(|img| self.apply_image(img, to) - from)
            .min_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
            .unwrap_or(to - from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_of(space: &Space, points: &[Vec2]) -> SpatialGrid {
        let mut grid = SpatialGrid::new(space.width, space.height, 2.0);
        grid.rebuild(points.iter().enumerate().map(|(i, p)| (i as u32, *p)));
        grid
    }

    #[test]
    fn klein_bottle_wrap_matches_cell_topology() {
        let s = Space::new(Topology::KleinBottle, 10, 8);
        // Leaving through the top at x = 2.25 re-enters at the bottom mirrored.
        let mut k = Kinematics { position: Vec2::new(2.25, -0.5), velocity: Vec2::new(1.0, -1.0), heading: 0.0 };
        s.constrain(&mut k);
        assert!((k.position - Vec2::new(7.75, 7.5)).length() < 1e-5, "{:?}", k.position);
        assert_eq!(k.velocity, Vec2::new(-1.0, -1.0));
        // The cell agrees with Topology::wrap on the integer cell (2, -1).
        let (cx, cy) = s.cell(k.position);
        assert_eq!(Topology::KleinBottle.wrap(2, -1, 10, 8), Some((cx as i64, cy as i64)));
        // Left/right crossing does not mirror.
        let mut k = Kinematics { position: Vec2::new(-0.5, 3.0), velocity: Vec2::X, heading: 0.0 };
        s.constrain(&mut k);
        assert!((k.position - Vec2::new(9.5, 3.0)).length() < 1e-5);
    }

    #[test]
    fn cross_surface_mirrors_both_axes() {
        let s = Space::new(Topology::CrossSurface, 10, 8);
        let mut k = Kinematics { position: Vec2::new(10.5, 2.5), velocity: Vec2::X, heading: 0.0 };
        s.constrain(&mut k);
        assert!((k.position - Vec2::new(0.5, 5.5)).length() < 1e-5, "{:?}", k.position);
        let (cx, cy) = s.cell(k.position);
        assert_eq!(Topology::CrossSurface.wrap(10, 2, 10, 8), Some((cx as i64, cy as i64)));
    }

    #[test]
    fn twisted_queries_see_across_edges_once() {
        let s = Space::new(Topology::KleinBottle, 20, 20);
        // An entity near the bottom edge at x = 3 is seen from the top edge at x = 17.
        let points = [Vec2::new(3.0, 19.5), Vec2::new(10.0, 10.0)];
        let grid = grid_of(&s, &points);
        let mut out = Vec::new();
        s.within(&grid, Vec2::new(17.0, 0.5), 2.0, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slot, 0);
        assert!((out[0].distance - 1.0).abs() < 1e-4, "{}", out[0].distance);
        assert!((s.delta(Vec2::new(17.0, 0.5), points[0]) - Vec2::new(0.0, -1.0)).length() < 1e-4);
        // On a torus the same pair is far apart.
        let t = Space::new(Topology::Torus, 20, 20);
        let grid = grid_of(&t, &points);
        t.within(&grid, Vec2::new(17.0, 0.5), 2.0, &mut out);
        assert!(out.is_empty());
        t.within(&grid, Vec2::new(3.0, 0.5), 2.0, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn bounded_reflects_and_never_wraps() {
        let s = Space::new(Topology::Bounded, 10, 10);
        let mut k = Kinematics { position: Vec2::new(-1.0, 5.0), velocity: Vec2::new(-1.0, 0.0), heading: 0.0 };
        s.constrain(&mut k);
        assert_eq!(k.position, Vec2::new(1.0, 5.0));
        assert_eq!(k.velocity, Vec2::new(1.0, 0.0));
        let grid = grid_of(&s, &[Vec2::new(9.5, 5.0)]);
        let mut out = Vec::new();
        s.within(&grid, Vec2::new(0.5, 5.0), 2.0, &mut out);
        assert!(out.is_empty());
        assert_eq!(s.cell(Vec2::new(10.0, 10.0)), (9, 9));
    }
}
