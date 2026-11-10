//! Model ↔ scene coordinates and terrain sampling.
//!
//! One scene unit is one metre. Model positions are in cells (`x` to the
//! right, `y` down the rows); cell `(x, y)` maps to the scene point
//! `(x · s, height, y · s)` with the cell size `s` in metres, so the centre
//! of cell `(i, j)` is at `((i + ½) s, h, (j + ½) s)`. The terrain grid the
//! view receives may be a block mean of `b × b` cells; its sample `(i, j)`
//! then sits at the centre of the block, `((i + ½) b s, h, (j + ½) b s)`.
//! Heights between samples are bilinear (the same surface the terrain mesh
//! shows with two triangles per quad, up to the diagonal).

use bevy::math::{Vec2, Vec3};

/// Geometry of the terrain grid in the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct MapGeometry {
    /// Samples along x.
    pub width: usize,
    /// Samples along y.
    pub height: usize,
    /// Metres between two samples (`block × cell size`).
    pub spacing: f32,
    /// Model cells per sample.
    pub block: usize,
    /// Metres per model cell.
    pub cell_size: f32,
    /// Ground elevation per sample (m), row-major.
    pub elevation: Vec<f32>,
    /// Water surface level per sample (m): elevation + water depth.
    pub water_level: Vec<f32>,
}

impl MapGeometry {
    /// A grid of `width × height` samples (each `block` cells of `cell_size`
    /// metres); water surface at ground level until set.
    pub fn new(width: usize, height: usize, block: usize, cell_size: f32, elevation: Vec<f32>) -> Self {
        let block = block.max(1);
        assert_eq!(elevation.len(), width * height, "one elevation per sample");
        Self {
            width,
            height,
            spacing: block as f32 * cell_size,
            block,
            cell_size,
            water_level: elevation.clone(),
            elevation,
        }
    }

    /// Size of the mapped area in metres (model cells × cell size).
    pub fn extent(&self) -> Vec2 {
        Vec2::new(self.width as f32, self.height as f32) * self.spacing
    }

    /// Centre of the map on the ground plane (y = 0).
    pub fn centre(&self) -> Vec3 {
        let e = self.extent();
        Vec3::new(e.x * 0.5, 0.0, e.y * 0.5)
    }

    /// Scene x/z of a model position (cells).
    pub fn cell_to_plane(&self, p: Vec2) -> Vec2 {
        p * self.cell_size
    }

    /// Model position (cells) of a scene x/z.
    pub fn plane_to_cell(&self, x: f32, z: f32) -> Vec2 {
        Vec2::new(x, z) / self.cell_size
    }

    /// Fractional sample coordinates of a scene x/z.
    fn sample_coords(&self, x: f32, z: f32) -> Vec2 {
        Vec2::new(x / self.spacing - 0.5, z / self.spacing - 0.5)
    }

    fn bilinear(&self, values: &[f32], x: f32, z: f32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let s = self.sample_coords(x, z);
        let fx = s.x.clamp(0.0, (self.width - 1) as f32);
        let fz = s.y.clamp(0.0, (self.height - 1) as f32);
        let (x0, z0) = (fx.floor() as usize, fz.floor() as usize);
        let (x1, z1) = ((x0 + 1).min(self.width - 1), (z0 + 1).min(self.height - 1));
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let v = |x: usize, z: usize| values[z * self.width + x];
        let top = v(x0, z0) + (v(x1, z0) - v(x0, z0)) * tx;
        let bottom = v(x0, z1) + (v(x1, z1) - v(x0, z1)) * tx;
        top + (bottom - top) * tz
    }

    /// Ground height (m) under a scene x/z.
    pub fn ground(&self, x: f32, z: f32) -> f32 {
        self.bilinear(&self.elevation, x, z)
    }

    /// Water surface (m) at a scene x/z (the ground where it is dry).
    pub fn surface(&self, x: f32, z: f32) -> f32 {
        self.bilinear(&self.water_level, x, z).max(self.ground(x, z))
    }

    /// Water depth (m) at a scene x/z.
    pub fn water_depth(&self, x: f32, z: f32) -> f32 {
        (self.surface(x, z) - self.ground(x, z)).max(0.0)
    }

    /// Sample index holding a model cell, if inside the grid.
    pub fn sample_of_cell(&self, cell: Vec2) -> Option<usize> {
        if !(cell.x >= 0.0 && cell.y >= 0.0) {
            return None;
        }
        let (i, j) = ((cell.x as usize) / self.block, (cell.y as usize) / self.block);
        (i < self.width && j < self.height).then_some(j * self.width + i)
    }

    /// Scene point on the ground for a model position.
    pub fn ground_point(&self, cell: Vec2) -> Vec3 {
        let p = self.cell_to_plane(cell);
        Vec3::new(p.x, self.ground(p.x, p.y), p.y)
    }

    /// Updates the water surface from a per-sample depth grid (same size).
    pub fn set_water(&mut self, depth: &[f32]) {
        if depth.len() == self.elevation.len() {
            for ((w, e), d) in self.water_level.iter_mut().zip(&self.elevation).zip(depth) {
                *w = e + d.max(0.0);
            }
        }
    }

    /// Whether a scene x/z lies over the mapped area.
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let e = self.extent();
        (0.0..=e.x).contains(&x) && (0.0..=e.y).contains(&z)
    }

    /// First intersection of a ray with the terrain surface (water counts
    /// as surface), by marching and refining with bisection. `None` when the
    /// ray misses the map.
    pub fn raycast(&self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        let dir = direction.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        let e = self.extent();
        // Clip the ray to the vertical box over the map.
        let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
        for (o, d, max) in [(origin.x, dir.x, e.x), (origin.z, dir.z, e.y)] {
            if d.abs() < 1e-9 {
                if !(0.0..=max).contains(&o) {
                    return None;
                }
            } else {
                let (a, b) = ((0.0 - o) / d, (max - o) / d);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
        }
        // Clip to the height band of the surface.
        let lowest = self.elevation.iter().copied().fold(f32::INFINITY, f32::min);
        let highest = self.water_level.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if dir.y < 0.0 {
            t1 = t1.min((origin.y - lowest) / -dir.y + 1e-3);
            t0 = t0.max((origin.y - highest) / -dir.y - 1e-3);
        } else if origin.y > highest {
            return None;
        }
        if t0 > t1 || !t1.is_finite() {
            return None;
        }
        let above = |t: f32| {
            let p = origin + dir * t;
            p.y - self.surface(p.x, p.z)
        };
        let step = (self.spacing * 0.25).max(0.05);
        let mut prev = t0;
        if above(prev) < 0.0 {
            return Some(origin + dir * prev);
        }
        let mut t = t0;
        while t < t1 {
            t = (t + step).min(t1);
            if above(t) <= 0.0 {
                let (mut lo, mut hi) = (prev, t);
                for _ in 0..24 {
                    let mid = 0.5 * (lo + hi);
                    if above(mid) > 0.0 {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                return Some(origin + dir * hi);
            }
            prev = t;
        }
        None
    }
}

/// Scene yaw (rotation about +Y) of a model heading `atan2(y, x)` for a
/// model that faces local +Z.
///
/// The model's x maps to scene x and its y to scene z, so the facing
/// direction is `(cos h, 0, sin h)`; rotating +Z by `θ` about +Y gives
/// `(sin θ, 0, cos θ)`, hence `θ = π/2 − h`.
pub fn heading_to_yaw(heading: f32) -> f32 {
    std::f32::consts::FRAC_PI_2 - heading
}

/// Shortest-arc interpolation between two angles.
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let d = (b - a).rem_euclid(tau);
    let d = if d > std::f32::consts::PI { d - tau } else { d };
    a + d * t
}

/// Block size that keeps a `width × height` grid at most `max_side`
/// samples per side.
pub fn block_for(width: usize, height: usize, max_side: usize) -> usize {
    width.max(height).div_ceil(max_side.max(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Quat;

    fn ramp() -> MapGeometry {
        // 4 × 3 samples, height = 2 × sample column, cells of 10 m.
        let elevation = (0..12).map(|i| 2.0 * (i % 4) as f32).collect();
        MapGeometry::new(4, 3, 1, 10.0, elevation)
    }

    #[test]
    fn cell_centres_and_bilinear_heights() {
        let m = ramp();
        assert_eq!(m.extent(), Vec2::new(40.0, 30.0));
        // Centre of cell (2, 1) is sample (2, 1) exactly.
        assert_eq!(m.ground_point(Vec2::new(2.5, 1.5)), Vec3::new(25.0, 4.0, 15.0));
        // Half way between samples 1 and 2 along x.
        assert!((m.ground(20.0, 15.0) - 3.0).abs() < 1e-5);
        // Clamped outside the sample lattice.
        assert_eq!(m.ground(-50.0, 0.0), 0.0);
        assert_eq!(m.ground(1000.0, 0.0), 6.0);
        assert_eq!(m.plane_to_cell(25.0, 15.0), Vec2::new(2.5, 1.5));
    }

    #[test]
    fn blocks_scale_the_lattice() {
        let m = MapGeometry::new(2, 2, 4, 20.0, vec![0.0, 8.0, 0.0, 8.0]);
        assert_eq!(m.spacing, 80.0);
        assert_eq!(m.extent(), Vec2::new(160.0, 160.0));
        assert_eq!(m.sample_of_cell(Vec2::new(5.0, 1.0)), Some(1));
        assert_eq!(m.sample_of_cell(Vec2::new(9.0, 0.0)), None);
        assert_eq!(m.sample_of_cell(Vec2::new(-1.0, 0.0)), None);
        // Sample centres at 40 m and 120 m.
        assert!((m.ground(80.0, 40.0) - 4.0).abs() < 1e-5);
        assert_eq!(block_for(100, 80, 256), 1);
        assert_eq!(block_for(1024, 300, 256), 4);
        assert_eq!(block_for(257, 1, 256), 2);
    }

    #[test]
    fn water_surface_and_depth() {
        let mut m = ramp();
        let mut depth = vec![0.0; 12];
        depth[0] = 3.0;
        m.set_water(&depth);
        assert_eq!(m.surface(5.0, 5.0), 3.0);
        assert_eq!(m.water_depth(5.0, 5.0), 3.0);
        assert_eq!(m.water_depth(35.0, 25.0), 0.0);
        m.set_water(&[1.0; 3]);
        assert_eq!(m.surface(5.0, 5.0), 3.0, "a grid of the wrong size is ignored");
    }

    #[test]
    fn raycast_hits_the_surface() {
        let m = MapGeometry::new(4, 4, 1, 10.0, vec![5.0; 16]);
        let hit = m.raycast(Vec3::new(20.0, 100.0, 20.0), Vec3::NEG_Y).expect("straight down");
        assert!((hit - Vec3::new(20.0, 5.0, 20.0)).length() < 1e-3);
        // Oblique ray entering from outside the map.
        let hit = m.raycast(Vec3::new(-30.0, 40.0, 20.0), Vec3::new(1.0, -0.6, 0.0)).expect("oblique");
        assert!((hit.y - 5.0).abs() < 1e-3 && (hit.x - (-30.0 + 35.0 / 0.6)).abs() < 1e-2, "{hit:?}");
        assert!(m.raycast(Vec3::new(20.0, 100.0, 20.0), Vec3::Y).is_none(), "pointing up");
        assert!(m.raycast(Vec3::new(-10.0, 100.0, 20.0), Vec3::NEG_X).is_none(), "away from the map");
        // A ray that starts underground reports its origin.
        assert_eq!(m.raycast(Vec3::new(20.0, 1.0, 20.0), Vec3::X), Some(Vec3::new(20.0, 1.0, 20.0)));
    }

    #[test]
    fn heading_maps_to_facing_direction() {
        for h in [0.0f32, 0.7, 2.0, -1.3] {
            let facing = Quat::from_rotation_y(heading_to_yaw(h)) * Vec3::Z;
            let expected = Vec3::new(h.cos(), 0.0, h.sin());
            assert!((facing - expected).length() < 1e-5, "{h}");
        }
        assert!((lerp_angle(3.0, -3.0, 0.5).abs() - std::f32::consts::PI).abs() < 0.1, "shortest arc");
        assert_eq!(lerp_angle(0.0, 1.0, 0.25), 0.25);
    }
}
