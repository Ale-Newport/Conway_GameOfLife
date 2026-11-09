//! Seeded fractal value noise used to generate the tile world.
//!
//! Classic lattice *value noise*: every integer lattice point `(i, j)` carries
//! a pseudo-random value `h(seed, i, j) ∈ [0, 1)` obtained from a SplitMix64
//! style hash, and the value at a continuous point is the bilinear
//! interpolation of the four surrounding lattice values with the smoothstep
//! weight `s(t) = t²(3 − 2t)`. Octaves are summed with doubling frequency and
//! halving amplitude (fractional Brownian motion) and normalised by the total
//! amplitude, so the result stays in `[0, 1)`.
//!
//! The function is pure (no generator state), so terrain generation does not
//! depend on evaluation order.

/// Fractal value-noise field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueNoise {
    seed: u64,
    period: f32,
    octaves: u32,
}

impl ValueNoise {
    /// Field whose first octave has features about `period` tiles wide.
    /// `period` is clamped to at least 1 and `octaves` to `1..=8`.
    pub fn new(seed: u64, period: f32, octaves: u32) -> Self {
        let period = if period.is_finite() { period.max(1.0) } else { 1.0 };
        Self { seed, period, octaves: octaves.clamp(1, 8) }
    }

    /// Value at `(x, y)` in `[0, 1)`.
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let mut amplitude = 1.0;
        let mut frequency = 1.0 / self.period;
        let mut total = 0.0;
        let mut norm = 0.0;
        for octave in 0..u64::from(self.octaves) {
            let seed = self.seed.wrapping_add(octave.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            total += amplitude * lattice_noise(seed, x * frequency, y * frequency);
            norm += amplitude;
            amplitude *= 0.5;
            frequency *= 2.0;
        }
        total / norm
    }
}

/// Hash of a lattice point to `[0, 1)` (SplitMix64 finaliser).
fn hash2(seed: u64, ix: i64, iy: i64) -> f32 {
    let mut h = seed
        ^ (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F).rotate_left(31);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// One octave: smoothstep-weighted bilinear interpolation of lattice values.
fn lattice_noise(seed: u64, x: f32, y: f32) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (ix, iy) = (fx as i64, fy as i64);
    let (tx, ty) = (smoothstep(x - fx), smoothstep(y - fy));
    let v00 = hash2(seed, ix, iy);
    let v10 = hash2(seed, ix + 1, iy);
    let v01 = hash2(seed, ix, iy + 1);
    let v11 = hash2(seed, ix + 1, iy + 1);
    let top = v00 + (v10 - v00) * tx;
    let bottom = v01 + (v11 - v01) * tx;
    top + (bottom - top) * ty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_in_unit_interval_and_deterministic() {
        let a = ValueNoise::new(7, 12.0, 4);
        let b = ValueNoise::new(7, 12.0, 4);
        for i in 0..2000 {
            let (x, y) = (i as f32 * 0.37, i as f32 * 0.91 - 300.0);
            let v = a.sample(x, y);
            assert!((0.0..1.0).contains(&v), "{v}");
            assert_eq!(v, b.sample(x, y));
        }
    }

    #[test]
    fn interpolates_lattice_values_and_is_continuous() {
        let n = ValueNoise::new(3, 1.0, 1);
        // At lattice points the noise equals the hashed value.
        assert_eq!(n.sample(4.0, -2.0), hash2(3, 4, -2));
        // Small steps produce small changes (Lipschitz with smoothstep slope 1.5).
        let n = ValueNoise::new(3, 10.0, 3);
        for i in 0..500 {
            let x = i as f32 * 0.13;
            assert!((n.sample(x, 5.0) - n.sample(x + 0.01, 5.0)).abs() < 0.01);
        }
    }

    #[test]
    fn seeds_give_different_fields() {
        let a = ValueNoise::new(1, 8.0, 3);
        let b = ValueNoise::new(2, 8.0, 3);
        let differing = (0..100).filter(|&i| a.sample(i as f32, 3.0) != b.sample(i as f32, 3.0)).count();
        assert!(differing > 90);
    }
}
