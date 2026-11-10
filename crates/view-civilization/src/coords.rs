//! Coordinates and the terrain height field.
//!
//! Convention (workspace grid convention): tile `(x, y)` covers
//! `[x, x + 1) × [y, y + 1)` in the model's tile units and maps to Bevy
//! `X = x`, `Z = y`, `Y` = height; agent positions (tile units) map the same
//! way, `(p.x, h, p.y)`. One tile is one world unit.
//!
//! The landscape is purely visual: every tile gets a height from its
//! terrain class plus a small seeded jitter; corners take the mean of the
//! tiles that touch them and each tile is drawn as a fan of four triangles
//! around its centre, so mountains and hills rise as peaks and ridges, and
//! land next to water slopes down below the water level ([`WATER_LEVEL`]).

use bevy::math::{Vec2, Vec3};
use civilization::Terrain;

/// Height of the water surface.
pub const WATER_LEVEL: f32 = 0.0;

/// Base height of a terrain class (world units).
pub fn base_height(terrain: Terrain) -> f32 {
    match terrain {
        Terrain::Water => -0.34,
        Terrain::Riverbank => 0.03,
        Terrain::Grassland => 0.07,
        Terrain::Berries => 0.08,
        Terrain::Forest => 0.1,
        Terrain::Hills => 0.36,
        Terrain::Mountain => 1.0,
    }
}

/// Amplitude of the seeded height jitter of a terrain class.
fn jitter_amplitude(terrain: Terrain) -> f32 {
    match terrain {
        Terrain::Mountain => 0.5,
        Terrain::Hills => 0.14,
        Terrain::Water => 0.06,
        _ => 0.025,
    }
}

/// Deterministic hash of a tile to `[0, 1)` (SplitMix64 finaliser).
pub fn tile_hash(x: u32, y: u32, salt: u64) -> f32 {
    let mut z = (u64::from(x) << 32 | u64::from(y)) ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// Heights of tile centres and tile corners.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeightField {
    width: u32,
    height: u32,
    centres: Vec<f32>,
    corners: Vec<f32>,
}

impl HeightField {
    /// Builds the field of a `width × height` terrain grid (row-major).
    pub fn from_terrain(terrain: &[Terrain], width: u32, height: u32) -> Self {
        let (w, h) = (width as usize, height as usize);
        assert_eq!(terrain.len(), w * h, "terrain grid has the wrong size");
        let centres: Vec<f32> = terrain
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                let (x, y) = ((i % w) as u32, (i / w) as u32);
                base_height(t) + jitter_amplitude(t) * tile_hash(x, y, 7)
            })
            .collect();
        let mut corners = vec![0.0; (w + 1) * (h + 1)];
        for cy in 0..=h {
            for cx in 0..=w {
                let mut sum = 0.0;
                let mut n = 0.0;
                for (dx, dy) in [(0usize, 0usize), (1, 0), (0, 1), (1, 1)] {
                    if cx >= dx && cy >= dy && cx - dx < w && cy - dy < h {
                        let t = (cy - dy) * w + (cx - dx);
                        sum += centres[t];
                        n += 1.0;
                    }
                }
                corners[cy * (w + 1) + cx] = if n > 0.0 { sum / n } else { 0.0 };
            }
        }
        Self { width, height, centres, corners }
    }

    /// Width in tiles.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height (rows) in tiles.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Height of the centre of tile `(x, y)`.
    pub fn centre(&self, x: u32, y: u32) -> f32 {
        self.centres[(y * self.width + x) as usize]
    }

    /// Height of the corner `(cx, cy)` (`0..=width`, `0..=height`).
    pub fn corner(&self, cx: u32, cy: u32) -> f32 {
        self.corners[(cy * (self.width + 1) + cx) as usize]
    }

    /// Height of the drawn surface at a point (tile units), clamped to the map.
    pub fn height_at(&self, p: Vec2) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let px = p.x.clamp(0.0, self.width as f32 - 1e-4);
        let py = p.y.clamp(0.0, self.height as f32 - 1e-4);
        let (ix, iy) = (px.floor() as u32, py.floor() as u32);
        let (u, v) = (px - ix as f32, py - iy as f32);
        let c = self.centre(ix, iy);
        let h00 = self.corner(ix, iy);
        let h10 = self.corner(ix + 1, iy);
        let h01 = self.corner(ix, iy + 1);
        let h11 = self.corner(ix + 1, iy + 1);
        let (du, dv) = (u - 0.5, v - 0.5);
        // Fan triangle containing (u, v), as (corner a, corner b) with their
        // local positions; interpolate barycentrically with the centre.
        let (a, pa, b, pb) = if dv <= -du.abs() {
            (h00, Vec2::new(0.0, 0.0), h10, Vec2::new(1.0, 0.0))
        } else if du >= dv.abs() {
            (h10, Vec2::new(1.0, 0.0), h11, Vec2::new(1.0, 1.0))
        } else if dv >= du.abs() {
            (h11, Vec2::new(1.0, 1.0), h01, Vec2::new(0.0, 1.0))
        } else {
            (h01, Vec2::new(0.0, 1.0), h00, Vec2::new(0.0, 0.0))
        };
        let pc = Vec2::splat(0.5);
        let q = Vec2::new(u, v);
        let det = (pa.x - pc.x) * (pb.y - pc.y) - (pb.x - pc.x) * (pa.y - pc.y);
        let wa = ((q.x - pc.x) * (pb.y - pc.y) - (pb.x - pc.x) * (q.y - pc.y)) / det;
        let wb = ((pa.x - pc.x) * (q.y - pc.y) - (q.x - pc.x) * (pa.y - pc.y)) / det;
        c + wa * (a - c) + wb * (b - c)
    }

    /// Smooth normal at a corner, from central differences of the corner grid.
    pub fn corner_normal(&self, cx: u32, cy: u32) -> Vec3 {
        let x0 = cx.saturating_sub(1);
        let x1 = (cx + 1).min(self.width);
        let y0 = cy.saturating_sub(1);
        let y1 = (cy + 1).min(self.height);
        let sx = (self.corner(x1, cy) - self.corner(x0, cy)) / (x1 - x0).max(1) as f32;
        let sy = (self.corner(cx, y1) - self.corner(cx, y0)) / (y1 - y0).max(1) as f32;
        Vec3::new(-sx, 1.0, -sy).normalize()
    }

    /// Normal at a tile centre from the slopes towards its corners.
    pub fn centre_normal(&self, x: u32, y: u32) -> Vec3 {
        let c = self.centre(x, y);
        let east = 0.5 * (self.corner(x + 1, y) + self.corner(x + 1, y + 1)) - c;
        let west = 0.5 * (self.corner(x, y) + self.corner(x, y + 1)) - c;
        let south = 0.5 * (self.corner(x, y + 1) + self.corner(x + 1, y + 1)) - c;
        let north = 0.5 * (self.corner(x, y) + self.corner(x + 1, y)) - c;
        // Slopes over half a tile each way.
        let sx = (east - west) / 1.0;
        let sy = (south - north) / 1.0;
        Vec3::new(-sx, 1.0, -sy).normalize()
    }

    /// First intersection of a ray with the drawn surface inside the map,
    /// found by marching in steps of 0.2 world units and bisecting.
    pub fn ray_hit(&self, origin: Vec3, direction: Vec3) -> Option<Vec3> {
        let dir = direction.normalize_or_zero();
        if dir == Vec3::ZERO || self.width == 0 {
            return None;
        }
        // Clip to the map's bounding box.
        let lo = Vec3::new(0.0, -1.0, 0.0);
        let hi = Vec3::new(self.width as f32, 3.5, self.height as f32);
        let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
        for axis in 0..3 {
            let (o, d) = (origin[axis], dir[axis]);
            if d.abs() < 1e-9 {
                if o < lo[axis] || o > hi[axis] {
                    return None;
                }
            } else {
                let (a, b) = ((lo[axis] - o) / d, (hi[axis] - o) / d);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
        }
        if t0 > t1 {
            return None;
        }
        let above = |t: f32| {
            let p = origin + dir * t;
            p.y - self.height_at(Vec2::new(p.x, p.z))
        };
        let step = 0.2;
        let mut prev = t0;
        if above(prev) <= 0.0 {
            return Some(origin + dir * prev);
        }
        let mut t = t0 + step;
        while t <= t1 + step {
            let now = above(t.min(t1));
            if now <= 0.0 {
                let (mut a, mut b) = (prev, t.min(t1));
                for _ in 0..24 {
                    let m = 0.5 * (a + b);
                    if above(m) > 0.0 {
                        a = m;
                    } else {
                        b = m;
                    }
                }
                return Some(origin + dir * b);
            }
            prev = t;
            t += step;
        }
        None
    }
}

/// World position of a point in tile units at height `h`.
pub fn to_world(p: Vec2, h: f32) -> Vec3 {
    Vec3::new(p.x, h, p.y)
}

/// Rotation about +Y that turns a model facing +Z to the model heading
/// (`atan2` angle in the tile plane, x towards +X, y towards +Z).
pub fn heading_to_yaw(heading: f32) -> f32 {
    std::f32::consts::FRAC_PI_2 - heading
}

/// Smallest signed difference `to − from` between two angles (radians).
pub fn angle_delta(from: f32, to: f32) -> f32 {
    let d = (to - from).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI { d - std::f32::consts::TAU } else { d }
}

/// Distance from a point to a ray (origin, unit direction), and the ray
/// parameter of the closest point (negative = behind the origin).
pub fn ray_point_distance(origin: Vec3, dir: Vec3, point: Vec3) -> (f32, f32) {
    let t = (point - origin).dot(dir);
    let closest = origin + dir * t.max(0.0);
    (closest.distance(point), t)
}

/// A "nice" scale-bar length (1, 2 or 5 × 10ⁿ tiles) of at most `max_tiles`.
pub fn nice_length(max_tiles: f32) -> f32 {
    if !(max_tiles.is_finite() && max_tiles > 0.0) {
        return 1.0;
    }
    let exp = max_tiles.log10().floor();
    let base = 10f32.powf(exp);
    for m in [5.0, 2.0, 1.0] {
        if m * base <= max_tiles {
            return m * base;
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    fn field(rows: &[&str]) -> HeightField {
        let terrain: Vec<Terrain> =
            rows.iter().flat_map(|r| r.chars().map(|c| Terrain::from_symbol(c).expect("symbol"))).collect();
        HeightField::from_terrain(&terrain, rows[0].len() as u32, rows.len() as u32)
    }

    #[test]
    fn heights_follow_terrain_and_water_lies_below_the_surface() {
        let f = field(&["....", ".~~.", ".^h.", "...."]);
        assert!(f.centre(1, 1) < WATER_LEVEL, "water bed is below the water level");
        assert!(f.centre(1, 2) > f.centre(2, 2), "mountains rise above hills");
        assert!(f.centre(2, 2) > f.centre(0, 0), "hills rise above grassland");
        // Corners are means of the touching tiles.
        let expected = (f.centre(0, 0) + f.centre(1, 0) + f.centre(0, 1) + f.centre(1, 1)) / 4.0;
        assert!((f.corner(1, 1) - expected).abs() < 1e-6);
        assert!((f.corner(0, 0) - f.centre(0, 0)).abs() < 1e-6, "map corners touch one tile");
    }

    #[test]
    fn height_at_matches_vertices_and_is_continuous() {
        let f = field(&["..h.", ".^h.", "~~..", "...."]);
        for y in 0..4 {
            for x in 0..4 {
                let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                assert!((f.height_at(c) - f.centre(x, y)).abs() < 1e-5);
                let corner = Vec2::new(x as f32 + 1e-4, y as f32 + 1e-4);
                assert!((f.height_at(corner) - f.corner(x, y)).abs() < 2e-3);
            }
        }
        // Across a tile border the surface does not jump.
        let a = f.height_at(Vec2::new(1.9999, 1.3));
        let b = f.height_at(Vec2::new(2.0001, 1.3));
        assert!((a - b).abs() < 1e-2, "{a} vs {b}");
        // Edge midpoints lie between the two corners.
        let mid = f.height_at(Vec2::new(1.5, 1.0001));
        assert!((mid - 0.5 * (f.corner(1, 1) + f.corner(2, 1))).abs() < 2e-3);
        // Outside the map the height is clamped, never NaN.
        assert!(f.height_at(Vec2::new(-5.0, 99.0)).is_finite());
    }

    #[test]
    fn rays_hit_the_surface() {
        let f = field(&["....", "....", "....", "...."]);
        let origin = Vec3::new(2.3, 10.0, 1.7);
        let hit = f.ray_hit(origin, Vec3::NEG_Y).expect("straight down hits");
        assert!((hit.x - 2.3).abs() < 1e-4 && (hit.z - 1.7).abs() < 1e-4);
        assert!((hit.y - f.height_at(Vec2::new(2.3, 1.7))).abs() < 1e-3);
        let slanted = f.ray_hit(Vec3::new(-3.0, 4.0, 2.0), Vec3::new(1.0, -1.0, 0.0)).expect("slanted hit");
        assert!((slanted.x - (4.0 - slanted.y - 3.0)).abs() < 1e-3, "{slanted:?}");
        assert!(f.ray_hit(origin, Vec3::Y).is_none(), "looking up misses");
        assert!(f.ray_hit(Vec3::new(50.0, 5.0, 50.0), Vec3::NEG_Y).is_none(), "outside the map");
    }

    #[test]
    fn heading_maps_to_model_yaw() {
        use bevy::math::Quat;
        for heading in [0.0f32, FRAC_PI_2, PI, -FRAC_PI_2, 0.7] {
            let forward = Quat::from_rotation_y(heading_to_yaw(heading)) * Vec3::Z;
            let expected = Vec3::new(heading.cos(), 0.0, heading.sin());
            assert!((forward - expected).length() < 1e-5, "{heading}: {forward:?}");
        }
        assert!((angle_delta(3.0, -3.0) - (2.0 * PI - 6.0)).abs() < 1e-5);
        assert!((angle_delta(-3.0, 3.0) + (2.0 * PI - 6.0)).abs() < 1e-5);
        assert_eq!(to_world(Vec2::new(3.0, 4.0), 1.0), Vec3::new(3.0, 1.0, 4.0));
    }

    #[test]
    fn ray_point_distance_and_nice_lengths() {
        let (d, t) = ray_point_distance(Vec3::ZERO, Vec3::X, Vec3::new(3.0, 4.0, 0.0));
        assert!((d - 4.0).abs() < 1e-6 && (t - 3.0).abs() < 1e-6);
        assert_eq!(nice_length(7.0), 5.0);
        assert_eq!(nice_length(19.0), 10.0);
        assert_eq!(nice_length(25.0), 20.0);
        assert_eq!(nice_length(0.3), 0.2);
        assert_eq!(nice_length(-1.0), 1.0);
        let h = tile_hash(3, 4, 1);
        assert!((0.0..1.0).contains(&h) && h != tile_hash(4, 3, 1));
    }
}
