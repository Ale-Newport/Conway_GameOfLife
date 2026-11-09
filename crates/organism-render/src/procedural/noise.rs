//! Small deterministic noise functions for procedural geometry and colour
//! patterns (lattice value noise with quintic interpolation, fBm and a
//! cellular "spots" field). They depend only on their inputs, so generated
//! meshes are identical on every run.

use bevy::math::{Vec2, Vec3};

/// 32-bit integer hash (lowbias32, C. Wellons) of three lattice coordinates
/// and a seed.
#[inline]
pub fn hash3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = seed
        ^ (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

/// Hash mapped to `[0, 1)`.
#[inline]
pub fn hash_unit(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    (hash3(x, y, z, seed) >> 8) as f32 / (1u32 << 24) as f32
}

#[inline]
fn quintic(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Value noise in `[-1, 1]`, smooth (C²) between integer lattice points.
pub fn value3(p: Vec3, seed: u32) -> f32 {
    let base = p.floor();
    let f = p - base;
    let (ix, iy, iz) = (base.x as i32, base.y as i32, base.z as i32);
    let (u, v, w) = (quintic(f.x), quintic(f.y), quintic(f.z));
    let corner = |dx: i32, dy: i32, dz: i32| hash_unit(ix + dx, iy + dy, iz + dz, seed) * 2.0 - 1.0;
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Fractional Brownian motion: `octaves` of [`value3`] with lacunarity 2 and
/// gain 0.5, normalised to roughly `[-1, 1]`.
pub fn fbm3(p: Vec3, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves.max(1) {
        sum += amp * value3(p * freq, seed.wrapping_add(o.wrapping_mul(0x9e37_79b9)));
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// Distance from `p` to the nearest jittered feature point of a 2D
/// cellular (Worley) field with unit cells, and that feature's hash. Used for
/// spots, rosettes and patches.
pub fn worley2(p: Vec2, seed: u32) -> (f32, u32) {
    let (f1, _, h) = worley2_f2(p, seed);
    (f1, h)
}

/// Distances to the nearest (F1) and second-nearest (F2) feature points
/// and the nearest feature's hash. `F2 − F1` is small along cell borders,
/// which draws polygonal networks (giraffe patches, tortoise scutes).
pub fn worley2_f2(p: Vec2, seed: u32) -> (f32, f32, u32) {
    let cell = p.floor();
    let (cx, cy) = (cell.x as i32, cell.y as i32);
    let (mut f1, mut f2, mut hash) = (f32::MAX, f32::MAX, 0u32);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (x, y) = (cx + dx, cy + dy);
            let jitter = Vec2::new(hash_unit(x, y, 1, seed), hash_unit(x, y, 2, seed));
            let feature = Vec2::new(x as f32, y as f32) + 0.15 + jitter * 0.7;
            let d = p.distance(feature);
            if d < f1 {
                f2 = f1;
                f1 = d;
                hash = hash3(x, y, 0, seed);
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        for i in 0..200 {
            let p = Vec3::new(i as f32 * 0.37, i as f32 * 0.11, -(i as f32) * 0.23);
            let a = value3(p, 3);
            assert_eq!(a, value3(p, 3));
            assert!((-1.0..=1.0).contains(&a));
            assert!((-1.0..=1.0).contains(&fbm3(p, 3, 4)));
        }
        assert_ne!(value3(Vec3::splat(0.5), 1), value3(Vec3::splat(0.5), 2));
    }

    #[test]
    fn continuous_across_lattice_cells() {
        let a = value3(Vec3::new(0.999_99, 0.3, 0.7), 9);
        let b = value3(Vec3::new(1.000_01, 0.3, 0.7), 9);
        assert!((a - b).abs() < 1e-3);
    }

    #[test]
    fn second_nearest_is_farther() {
        for i in 0..50 {
            let (f1, f2, _) = worley2_f2(Vec2::new(i as f32 * 0.23, i as f32 * 0.41), 5);
            assert!(f2 >= f1);
        }
    }

    #[test]
    fn worley_finds_near_features() {
        for i in 0..50 {
            let p = Vec2::new(i as f32 * 0.31, i as f32 * 0.17);
            let (d, _) = worley2(p, 4);
            assert!(d < 1.5);
        }
    }
}
