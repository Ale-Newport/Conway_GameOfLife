//! Ground-cover meshes (grass tufts, flowers, shrubs) built once and
//! instanced: every tuft of a kind shares one mesh and one material, so
//! Bevy batches them into a single instanced draw.

use std::f32::consts::{PI, TAU};

use bevy::math::{Vec2, Vec3};
use simulation_core::SimRng;

use crate::procedural::mesh::{Cap, MeshData, lump, planform, tube};
use crate::procedural::palette::{shade, srgb};

/// Ground-cover kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GroundCover {
    /// Short grass tuft (~25 cm).
    Grass,
    /// Tall grass / reeds (~70 cm).
    TallGrass,
    /// Wildflower with a coloured head.
    Flower(FlowerColor),
    /// Leafy shrub (~1 m).
    Shrub,
}

/// Flower head colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FlowerColor {
    /// Poppy red.
    Red,
    /// Buttercup yellow.
    Yellow,
    /// Cornflower blue.
    Blue,
    /// Daisy white.
    White,
}

impl FlowerColor {
    /// Petal colour (`0xRRGGBB`).
    pub fn petal(self) -> u32 {
        match self {
            Self::Red => 0xd8281c,
            Self::Yellow => 0xf0c820,
            Self::Blue => 0x3a64d8,
            Self::White => 0xf4f2ea,
        }
    }
}

impl GroundCover {
    /// Whether the kind takes the seasonal foliage tint (flowers keep their
    /// own colours).
    pub fn seasonal(self) -> bool {
        !matches!(self, Self::Flower(_))
    }

    /// Wind stiffness: natural sway frequency (Hz) of the plant.
    pub fn sway_frequency(self) -> f32 {
        match self {
            Self::Grass => 1.4,
            Self::TallGrass => 0.9,
            Self::Flower(_) => 1.1,
            Self::Shrub => 0.5,
        }
    }
}

/// Builds the mesh of a ground-cover kind (base at the origin, +Y up).
/// Seasonal kinds carry luminance-only vertex colours.
pub fn ground_cover_mesh(kind: GroundCover, seed: u64) -> MeshData {
    let mut rng = SimRng::seed_from_u64(seed);
    let mut m = MeshData::new();
    match kind {
        GroundCover::Grass | GroundCover::TallGrass => {
            let (blades, height) = if kind == GroundCover::Grass { (11, 0.25) } else { (9, 0.75) };
            for k in 0..blades {
                let a = k as f32 * 2.4 + rng.range_f32(-0.3, 0.3);
                let lean = rng.range_f32(0.15, 0.55);
                let h = height * rng.range_f32(0.65, 1.1);
                let base = Vec3::new(a.cos(), 0.0, a.sin()) * 0.03 * rng.f32();
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                // Blade: thin ribbon bending outwards under its own weight.
                let pts: Vec<Vec2> = (0..=4)
                    .map(|i| {
                        let u = i as f32 / 4.0;
                        Vec2::new(lean * h * u * u, h * u)
                    })
                    .collect();
                let w = 0.012 * height.sqrt();
                let mut outline: Vec<Vec2> = pts.iter().map(|p| Vec2::new(p.x - w * (1.0 - p.y / h), p.y)).collect();
                outline.extend(pts.iter().rev().map(|p| Vec2::new(p.x + w * (1.0 - p.y / h), p.y)));
                planform(
                    &mut m,
                    &outline,
                    Vec2::new(0.4 * lean * h, 0.4 * h),
                    1,
                    0.002,
                    base,
                    out,
                    Vec3::Y,
                    &|q, _| {
                        let lit = 0.55 + 0.45 * (q.y / h).clamp(0.0, 1.0);
                        [lit, lit, lit, 1.0]
                    },
                );
            }
        }
        GroundCover::Flower(color) => {
            let h = rng.range_f32(0.25, 0.4);
            let stem = srgb(0x3a7a2a);
            tube(
                &mut m,
                &[Vec3::ZERO, Vec3::new(0.02, 0.5 * h, 0.0), Vec3::new(0.0, h, 0.01)],
                &[0.006, 0.005, 0.004],
                5,
                Vec3::Z,
                (Cap::Flat, Cap::Flat),
                &move |_| stem,
            );
            // Two leaves.
            for (k, y) in [(0, 0.25 * h), (1, 0.45 * h)] {
                let a = PI * k as f32 + 0.6;
                let dir = Vec3::new(a.cos(), 0.35, a.sin()).normalize();
                let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
                let leaf = [Vec2::new(0.0, 0.0), Vec2::new(0.012, 0.04), Vec2::new(0.0, 0.08), Vec2::new(-0.012, 0.04)];
                planform(
                    &mut m,
                    &leaf,
                    Vec2::new(0.0, 0.03),
                    1,
                    0.003,
                    Vec3::new(0.0, y, 0.0),
                    side,
                    dir,
                    &move |_, _| shade(stem, 1.1),
                );
            }
            let petal = srgb(color.petal());
            let petals = 6;
            for k in 0..petals {
                let a = k as f32 / petals as f32 * TAU;
                let dir = Vec3::new(a.cos(), 0.25, a.sin()).normalize();
                let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
                let shape =
                    [Vec2::new(0.0, 0.0), Vec2::new(0.018, 0.03), Vec2::new(0.0, 0.05), Vec2::new(-0.018, 0.03)];
                planform(
                    &mut m,
                    &shape,
                    Vec2::new(0.0, 0.025),
                    1,
                    0.004,
                    Vec3::new(0.0, h, 0.01),
                    side,
                    dir,
                    &move |_, f| shade(petal, 1.0 - 0.15 * f),
                );
            }
            let center = srgb(if color == FlowerColor::White { 0xf0c020 } else { 0x2a2010 });
            lump(
                &mut m,
                Vec3::new(0.0, h + 0.008, 0.01),
                Vec3::splat(0.012),
                1,
                0.1,
                3.0,
                rng.next_u32(),
                &move |_, _| center,
            );
        }
        GroundCover::Shrub => {
            let stems = 5;
            let wood = srgb(0x5a4430);
            for k in 0..stems {
                let a = k as f32 * 2.4;
                let tip = Vec3::new(0.25 * a.cos(), 0.55, 0.25 * a.sin());
                tube(&mut m, &[Vec3::ZERO, tip], &[0.025, 0.012], 5, Vec3::Z, (Cap::Flat, Cap::Round), &move |_| wood);
            }
            for k in 0..7 {
                let a = k as f32 * 2.4 + rng.range_f32(-0.4, 0.4);
                let r = if k == 0 { 0.0 } else { rng.range_f32(0.25, 0.4) };
                let c = Vec3::new(r * a.cos(), rng.range_f32(0.45, 0.75), r * a.sin());
                let size = rng.range_f32(0.25, 0.38);
                lump(&mut m, c, Vec3::new(size, 0.8 * size, size), 2, 0.3, 2.5, rng.next_u32(), &|dir, k| {
                    let lit = 0.6 + 0.25 * dir.y + 0.2 * (k - 1.0);
                    [lit, lit, lit, 1.0]
                });
            }
        }
    }
    m
}

/// Deterministic scatter of `count` positions in a disc of `radius` with a
/// minimum spacing (dart throwing; gives up on a point after 30 tries).
pub fn scatter(count: usize, radius: f32, spacing: f32, seed: u64) -> Vec<Vec2> {
    let mut rng = SimRng::seed_from_u64(seed);
    let mut pts: Vec<Vec2> = Vec::with_capacity(count);
    for _ in 0..count {
        for _ in 0..30 {
            let p = rng.in_disc(radius);
            if pts.iter().all(|q| q.distance_squared(p) >= spacing * spacing) {
                pts.push(p);
                break;
            }
        }
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meshes_have_expected_heights() {
        let h = |k| ground_cover_mesh(k, 3).bounds().unwrap().1.y;
        assert!((0.15..0.35).contains(&h(GroundCover::Grass)));
        assert!(h(GroundCover::TallGrass) > 0.5);
        assert!(h(GroundCover::Shrub) > 0.7);
        assert!(h(GroundCover::Flower(FlowerColor::Red)) > 0.2);
        assert!(!GroundCover::Flower(FlowerColor::Blue).seasonal());
    }

    #[test]
    fn scatter_respects_spacing_and_radius() {
        let pts = scatter(200, 5.0, 0.3, 9);
        assert!(pts.len() > 150);
        for (i, a) in pts.iter().enumerate() {
            assert!(a.length() <= 5.0 + 1e-4);
            for b in &pts[i + 1..] {
                assert!(a.distance(*b) >= 0.3 - 1e-5);
            }
        }
        assert_eq!(pts, scatter(200, 5.0, 0.3, 9));
    }
}
