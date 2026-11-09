//! Deterministic, serialisable pseudo-random number generation.
//!
//! Emergence Lab implements its own small generator — xoshiro256++ (Blackman &
//! Vigna, 2018) seeded through SplitMix64 — instead of depending on the `rand`
//! crate family. Recorded experiments must replay identically years later, and
//! third-party generators have changed their output streams between major
//! versions. The full generator state is four `u64`s and is stored inside every
//! snapshot, so restoring a snapshot also restores the random stream.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// SplitMix64 step, used to expand seeds into well-mixed generator states.
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Seeded xoshiro256++ generator with convenience sampling helpers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimRng {
    s: [u64; 4],
}

impl SimRng {
    /// Creates a generator from a 64-bit seed. Equal seeds give equal streams.
    pub fn seed_from_u64(seed: u64) -> Self {
        let mut sm = seed;
        let mut s = [0u64; 4];
        for word in &mut s {
            *word = splitmix64(&mut sm);
        }
        // xoshiro must not start from the all-zero state; SplitMix64 cannot
        // produce four zero outputs in a row, but keep the guarantee explicit.
        if s == [0; 4] {
            s[0] = 1;
        }
        Self { s }
    }

    /// Creates a generator for an independent named stream of a seed, e.g. one
    /// stream for terrain generation and another for agent behaviour, so that
    /// changing one subsystem does not perturb the random numbers of another.
    pub fn derive(seed: u64, stream: u64) -> Self {
        let mut sm = stream ^ 0xD1B5_4A32_D192_ED03;
        let mixed = seed ^ splitmix64(&mut sm);
        Self::seed_from_u64(mixed)
    }

    /// Derives a child generator from the current state without advancing it.
    pub fn fork(&self, stream: u64) -> Self {
        let mut sm = self.s[0] ^ self.s[2].rotate_left(17) ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        Self::seed_from_u64(splitmix64(&mut sm) ^ self.s[1] ^ self.s[3].rotate_left(31))
    }

    /// Builds a generator directly from a raw state (mainly for tests).
    pub fn from_state(state: [u64; 4]) -> Self {
        assert!(state != [0; 4], "xoshiro256++ state must not be all zero");
        Self { s: state }
    }

    /// Raw generator state.
    pub fn state(&self) -> [u64; 4] {
        self.s
    }

    /// Next 64 random bits.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Next 32 random bits (upper half of a 64-bit output).
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform `f64` in `[0, 1)` with 53 bits of precision.
    #[inline]
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform `f32` in `[0, 1)` with 24 bits of precision.
    #[inline]
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    /// Uniform `f32` in `[lo, hi)`.
    #[inline]
    pub fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    /// Uniform `f64` in `[lo, hi)`.
    #[inline]
    pub fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }

    /// Unbiased uniform integer in `[0, n)` (Lemire's multiply-shift method).
    ///
    /// # Panics
    /// Panics if `n == 0`.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "SimRng::below called with n = 0");
        let mut m = u128::from(self.next_u64()) * u128::from(n);
        let mut low = m as u64;
        if low < n {
            let threshold = n.wrapping_neg() % n;
            while low < threshold {
                m = u128::from(self.next_u64()) * u128::from(n);
                low = m as u64;
            }
        }
        (m >> 64) as u64
    }

    /// Uniform index in `[0, len)`.
    #[inline]
    pub fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    /// Uniform integer in the half-open range `[lo, hi)`.
    ///
    /// # Panics
    /// Panics if `hi <= lo`.
    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        assert!(hi > lo, "empty range {lo}..{hi}");
        let span = hi.wrapping_sub(lo) as u64;
        lo.wrapping_add(self.below(span) as i64)
    }

    /// Returns `true` with probability `p` (clamped to `[0, 1]`).
    #[inline]
    pub fn chance(&mut self, p: f64) -> bool {
        if p <= 0.0 {
            false
        } else if p >= 1.0 {
            true
        } else {
            self.f64() < p
        }
    }

    /// Standard normal sample (Box–Muller transform).
    pub fn normal(&mut self) -> f64 {
        // 1 - u keeps the logarithm argument in (0, 1].
        let u1 = 1.0 - self.f64();
        let u2 = self.f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Normal sample with the given mean and standard deviation.
    #[inline]
    pub fn normal_with(&mut self, mean: f64, std_dev: f64) -> f64 {
        mean + std_dev * self.normal()
    }

    /// Uniformly distributed unit vector in the plane.
    pub fn unit_vec2(&mut self) -> Vec2 {
        let angle = self.f32() * std::f32::consts::TAU;
        Vec2::new(angle.cos(), angle.sin())
    }

    /// Uniform point inside a disc of the given radius.
    pub fn in_disc(&mut self, radius: f32) -> Vec2 {
        self.unit_vec2() * radius * self.f32().sqrt()
    }

    /// Uniformly distributed unit vector in 3D (Marsaglia's method).
    pub fn unit_vec3(&mut self) -> Vec3 {
        loop {
            let a = self.range_f32(-1.0, 1.0);
            let b = self.range_f32(-1.0, 1.0);
            let s = a * a + b * b;
            if s < 1.0 && s > 0.0 {
                let f = 2.0 * (1.0 - s).sqrt();
                return Vec3::new(a * f, b * f, 1.0 - 2.0 * s);
            }
        }
    }

    /// Fisher–Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.index(i + 1);
            items.swap(i, j);
        }
    }

    /// Uniformly chosen element, or `None` for an empty slice.
    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() { None } else { Some(&items[self.index(items.len())]) }
    }

    /// Index chosen with probability proportional to its (non-negative) weight.
    /// Returns `None` when all weights are zero or the slice is empty.
    pub fn weighted_index(&mut self, weights: &[f64]) -> Option<usize> {
        let total: f64 = weights.iter().filter(|w| w.is_finite() && **w > 0.0).sum();
        if total <= 0.0 {
            return None;
        }
        let mut target = self.f64() * total;
        let mut last_positive = None;
        for (i, &w) in weights.iter().enumerate() {
            if !(w.is_finite() && w > 0.0) {
                continue;
            }
            last_positive = Some(i);
            if target < w {
                return Some(i);
            }
            target -= w;
        }
        last_positive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoshiro256pp_reference_output() {
        // For state {1, 2, 3, 4} the first output is rotl(1 + 4, 23) + 1.
        let mut rng = SimRng::from_state([1, 2, 3, 4]);
        assert_eq!(rng.next_u64(), 41_943_041);
    }

    #[test]
    fn same_seed_same_stream() {
        let mut a = SimRng::seed_from_u64(42);
        let mut b = SimRng::seed_from_u64(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = SimRng::seed_from_u64(43);
        assert_ne!(SimRng::seed_from_u64(42).next_u64(), c.next_u64());
    }

    #[test]
    fn derived_streams_differ_and_are_stable() {
        let mut a = SimRng::derive(7, 1);
        let mut b = SimRng::derive(7, 2);
        assert_ne!(a.next_u64(), b.next_u64());
        assert_eq!(SimRng::derive(7, 1), SimRng::derive(7, 1));
    }

    #[test]
    fn floats_in_unit_interval() {
        let mut rng = SimRng::seed_from_u64(1);
        for _ in 0..10_000 {
            let x = rng.f64();
            assert!((0.0..1.0).contains(&x));
            let y = rng.f32();
            assert!((0.0..1.0).contains(&y));
        }
    }

    #[test]
    fn below_is_in_range_and_roughly_uniform() {
        let mut rng = SimRng::seed_from_u64(9);
        let mut counts = [0u32; 6];
        for _ in 0..60_000 {
            counts[rng.below(6) as usize] += 1;
        }
        for c in counts {
            assert!((9_000..11_000).contains(&c), "counts {counts:?}");
        }
    }

    #[test]
    fn normal_has_expected_moments() {
        let mut rng = SimRng::seed_from_u64(3);
        let n = 50_000;
        let samples: Vec<f64> = (0..n).map(|_| rng.normal()).collect();
        let mean = samples.iter().sum::<f64>() / n as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.02, "mean {mean}");
        assert!((var - 1.0).abs() < 0.03, "variance {var}");
    }

    #[test]
    fn serde_round_trip_preserves_stream() {
        let mut rng = SimRng::seed_from_u64(5);
        rng.next_u64();
        let json = serde_json::to_string(&rng).unwrap();
        let mut restored: SimRng = serde_json::from_str(&json).unwrap();
        assert_eq!(rng.next_u64(), restored.next_u64());
    }

    #[test]
    fn weighted_index_respects_zero_weights() {
        let mut rng = SimRng::seed_from_u64(11);
        for _ in 0..1000 {
            let i = rng.weighted_index(&[0.0, 2.0, 0.0, 1.0]).unwrap();
            assert!(i == 1 || i == 3);
        }
        assert_eq!(rng.weighted_index(&[0.0, 0.0]), None);
    }
}
