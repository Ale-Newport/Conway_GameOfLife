//! Voxel colours: palette layout, state / age / height ramps and the
//! ambient-occlusion-like shade levels.
//!
//! The rendering mesher colours a voxel by a palette index (`u8`, 0 =
//! empty). This mode packs three things into the index:
//!
//! ```text
//! index = 1 + ((translucent · COLORS) + colour) · SHADES + shade
//! shade = 2 · occlusion level + parity
//! ```
//!
//! * `colour` — one of [`COLORS`] buckets of the active ramp (state, age or
//!   height);
//! * occlusion level — one of [`AO_LEVELS`] brightness levels: voxels with
//!   many visible neighbours (crevices, the inside of concave shapes) are
//!   darker, so depth reads at a glance even without shadows;
//! * parity — `x + y + z` odd voxels are slightly darker (a 3D checkerboard),
//!   so single voxels stay distinguishable on flat faces of one colour;
//! * `translucent` — whether the voxel uses the translucent material.
//!
//! Occlusion and parity are display aids only. Because they live in the
//! palette index, every change of them changes the voxel set the chunked
//! mesher hashes, so re-meshing stays exact.
//!
//! The palette therefore only changes when the ramp, transparency or
//! opacity changes, and the chunked mesher re-meshes only changed chunks
//! between generations.

use bevy::prelude::*;
use bevy_egui::egui::Color32;
use rendering::{Colormap, VoxelPalette};

use crate::settings::ColorMode;

/// Occlusion (depth-shading) levels.
pub const AO_LEVELS: u8 = 4;
/// Brightness variants per colour: occlusion levels × checker parity.
pub const SHADES: u8 = AO_LEVELS * 2;
/// Colour buckets per ramp.
pub const COLORS: u8 = 15;
/// Brightness of odd-parity voxels (linear factor).
pub const CHECKER: f32 = 0.84;
/// Palette length (index 0 + both opacity classes).
pub const PALETTE_LEN: usize = 1 + 2 * COLORS as usize * SHADES as usize;

/// sRGB colour of alive voxels in the state ramp.
pub const ALIVE_SRGB: [f32; 3] = [1.0, 0.80, 0.32];

/// Palette index of a voxel.
pub fn palette_index(translucent: bool, color: u8, shade: u8) -> u8 {
    let color = color.min(COLORS - 1);
    let shade = shade.min(SHADES - 1);
    1 + ((u8::from(translucent) * COLORS + color) * SHADES + shade)
}

/// Shade of an occlusion level and a checker parity.
pub fn tone(level: u8, odd: bool) -> u8 {
    level.min(AO_LEVELS - 1) * 2 + u8::from(odd)
}

/// Shade of a fully lit, even voxel.
pub const FULL_TONE: u8 = (AO_LEVELS - 1) * 2;

/// Inverse of [`palette_index`]: `(translucent, colour, shade)`.
#[cfg(test)]
pub fn decode_index(index: u8) -> Option<(bool, u8, u8)> {
    if index == 0 || usize::from(index) >= PALETTE_LEN {
        return None;
    }
    let i = index - 1;
    let shade = i % SHADES;
    let class = i / SHADES;
    Some((class >= COLORS, class % COLORS, shade))
}

/// Colour bucket of a state: alive is the top bucket; dying states
/// `2 … C−1` spread over the others, freshly dying brightest.
pub fn state_bucket(state: u8, states: u16) -> u8 {
    if state <= 1 {
        return COLORS - 1;
    }
    let dying_states = u32::from(states.saturating_sub(2)).max(1);
    let d = u32::from(state - 2).min(dying_states - 1);
    let top = u32::from(COLORS - 2);
    if dying_states == 1 {
        return COLORS - 2;
    }
    (top - d * top / (dying_states - 1)) as u8
}

/// Colour bucket of an age on a logarithmic scale topping out at `scale`.
pub fn age_bucket(age: u16, scale: u16) -> u8 {
    let scale = f32::from(scale.max(1));
    let t = (1.0 + f32::from(age)).ln() / (1.0 + scale).ln();
    (t.clamp(0.0, 1.0) * f32::from(COLORS - 1)).round() as u8
}

/// Colour bucket of a height `z` within `[lo, hi]`.
pub fn height_bucket(z: i32, lo: i32, hi: i32) -> u8 {
    if hi <= lo {
        return COLORS - 1;
    }
    let t = (i64::from(z) - i64::from(lo)) as f64 / (i64::from(hi) - i64::from(lo)) as f64;
    (t.clamp(0.0, 1.0) * f64::from(COLORS - 1)).round() as u8
}

/// Occlusion level for a voxel with `neighbours` visible voxels among its
/// 26 neighbours: up to 4 neighbours is fully lit, 18 or more is darkest.
pub fn shade_level(neighbours: u8) -> u8 {
    let darkness = ((f32::from(neighbours) - 4.0) / 14.0).clamp(0.0, 1.0);
    ((1.0 - darkness) * f32::from(AO_LEVELS - 1)).round() as u8
}

/// Brightness factor of a shade (occlusion level and parity).
pub fn shade_factor(shade: u8) -> f32 {
    let shade = shade.min(SHADES - 1);
    let level = f32::from(shade / 2) / f32::from(AO_LEVELS - 1);
    let parity = if shade % 2 == 1 { CHECKER } else { 1.0 };
    (0.45 + 0.55 * level) * parity
}

/// sRGB colour of a bucket of a ramp.
pub fn bucket_srgb(mode: ColorMode, bucket: u8) -> [f32; 3] {
    let bucket = bucket.min(COLORS - 1);
    match mode {
        ColorMode::State => {
            if bucket == COLORS - 1 {
                ALIVE_SRGB
            } else {
                let t = f32::from(bucket) / f32::from(COLORS - 2);
                dying_srgb(t)
            }
        }
        ColorMode::Age => Colormap::Viridis.sample_srgb(0.08 + 0.92 * f32::from(bucket) / f32::from(COLORS - 1)),
        ColorMode::Height => Colormap::Plasma.sample_srgb(0.12 + 0.86 * f32::from(bucket) / f32::from(COLORS - 1)),
    }
}

/// Dying ramp: `t = 1` freshly dying (orange), `t = 0` about to vanish
/// (dark purple). An inferno segment, like embers cooling.
pub fn dying_srgb(t: f32) -> [f32; 3] {
    Colormap::Inferno.sample_srgb(0.22 + 0.55 * t.clamp(0.0, 1.0))
}

/// The voxel palette for a ramp; translucent entries use `opacity`.
pub fn build_palette(mode: ColorMode, opacity: f32) -> VoxelPalette {
    let opacity = opacity.clamp(0.02, 0.95);
    let mut colors = Vec::with_capacity(PALETTE_LEN);
    colors.push(Color::NONE);
    for translucent in [false, true] {
        for bucket in 0..COLORS {
            let [r, g, b] = bucket_srgb(mode, bucket);
            let linear = Color::srgb(r, g, b).to_linear();
            for shade in 0..SHADES {
                let f = shade_factor(shade);
                let alpha = if translucent { opacity } else { 1.0 };
                colors.push(Color::LinearRgba(LinearRgba::new(
                    linear.red * f,
                    linear.green * f,
                    linear.blue * f,
                    alpha,
                )));
            }
        }
    }
    // Infallible: the list is never empty.
    VoxelPalette::new(colors).unwrap_or_else(|_| unreachable_palette())
}

fn unreachable_palette() -> VoxelPalette {
    VoxelPalette::new(vec![Color::NONE, Color::WHITE]).expect("non-empty palette")
}

/// egui colour of an sRGB triple.
pub fn color32(srgb: [f32; 3]) -> Color32 {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(c(srgb[0]), c(srgb[1]), c(srgb[2]))
}

/// egui colour of a voxel state (state ramp, fully lit).
pub fn state_color32(state: u8, states: u16) -> Color32 {
    if state == 0 {
        return Color32::TRANSPARENT;
    }
    color32(bucket_srgb(ColorMode::State, state_bucket(state, states)))
}

/// Gradient stops of a ramp for legends (low → high).
pub fn ramp_stops(mode: ColorMode) -> Vec<Color32> {
    (0..COLORS).step_by(2).chain([COLORS - 1]).map(|b| color32(bucket_srgb(mode, b))).collect()
}

/// Dying-state stops for the legend (freshly dying → about to vanish).
pub fn dying_stops() -> Vec<Color32> {
    (0..=6).rev().map(|i| color32(dying_srgb(i as f32 / 6.0))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_round_trip_and_fit_a_byte() {
        assert_eq!(PALETTE_LEN, 241);
        const { assert!(PALETTE_LEN <= 256, "indices fit a byte") };
        for translucent in [false, true] {
            for color in 0..COLORS {
                for shade in 0..SHADES {
                    let index = palette_index(translucent, color, shade);
                    assert_ne!(index, 0, "0 is the empty state");
                    assert_eq!(decode_index(index), Some((translucent, color, shade)));
                }
            }
        }
        assert_eq!(decode_index(0), None);
        assert_eq!(decode_index(241), None);
        // Out-of-range inputs saturate instead of overflowing.
        assert_eq!(palette_index(true, 200, 200), palette_index(true, COLORS - 1, SHADES - 1));
    }

    #[test]
    fn palette_classes_have_the_right_opacity() {
        let palette = build_palette(ColorMode::State, 0.3);
        assert_eq!(palette.len(), PALETTE_LEN);
        assert!(!palette.is_translucent(palette_index(false, 3, 2)));
        assert!(palette.is_translucent(palette_index(true, 3, 2)));
        assert!((palette.color(palette_index(true, 0, 0)).alpha - 0.3).abs() < 1e-6);
        // Darker shades are darker.
        let dark = palette.color(palette_index(false, COLORS - 1, tone(0, false)));
        let lit = palette.color(palette_index(false, COLORS - 1, FULL_TONE));
        let odd = palette.color(palette_index(false, COLORS - 1, tone(AO_LEVELS - 1, true)));
        assert!(dark.red < lit.red && dark.green < lit.green);
        assert!(odd.red < lit.red && odd.red > dark.red, "the checker is subtler than occlusion");
    }

    #[test]
    fn state_buckets_order_alive_then_dying_by_freshness() {
        assert_eq!(state_bucket(1, 2), COLORS - 1);
        assert_eq!(state_bucket(1, 10), COLORS - 1);
        // Ten states: dying 2..=9; 2 is brightest, 9 darkest, monotonic.
        let buckets: Vec<u8> = (2..=9).map(|s| state_bucket(s, 10)).collect();
        assert_eq!(buckets.first(), Some(&(COLORS - 2)));
        assert_eq!(buckets.last(), Some(&0));
        assert!(buckets.windows(2).all(|w| w[0] > w[1]), "{buckets:?}");
        // One dying state; many states (C = 256) stay in range.
        assert_eq!(state_bucket(2, 3), COLORS - 2);
        assert!((2..=255).all(|s| state_bucket(s, 256) < COLORS - 1));
        assert_eq!(state_bucket(255, 256), 0);
    }

    #[test]
    fn age_and_height_buckets_are_monotonic_and_clamped() {
        assert_eq!(age_bucket(0, 100), 0);
        assert_eq!(age_bucket(100, 100), COLORS - 1);
        assert_eq!(age_bucket(65_535, 100), COLORS - 1);
        let ages: Vec<u8> = [0u16, 1, 3, 10, 30, 100].iter().map(|&a| age_bucket(a, 100)).collect();
        assert!(ages.windows(2).all(|w| w[0] < w[1]), "{ages:?}");
        assert_eq!(age_bucket(5, 0), COLORS - 1, "a zero scale saturates");
        assert_eq!(height_bucket(0, 0, 63), 0);
        assert_eq!(height_bucket(63, 0, 63), COLORS - 1);
        assert_eq!(height_bucket(-5, 0, 63), 0);
        assert_eq!(height_bucket(7, 7, 7), COLORS - 1);
        assert_eq!(height_bucket(i32::MAX, i32::MIN, i32::MAX), COLORS - 1);
    }

    #[test]
    fn shading_darkens_crowded_voxels() {
        assert_eq!(shade_level(0), AO_LEVELS - 1);
        assert_eq!(shade_level(4), AO_LEVELS - 1);
        assert_eq!(shade_level(18), 0);
        assert_eq!(shade_level(26), 0);
        let levels: Vec<u8> = (4..=18).map(shade_level).collect();
        assert!(levels.windows(2).all(|w| w[0] >= w[1]));
        assert!((shade_factor(FULL_TONE) - 1.0).abs() < 1e-6);
        assert!((shade_factor(tone(AO_LEVELS - 1, true)) - CHECKER).abs() < 1e-6);
        assert!(shade_factor(0) > 0.3, "even the darkest shade stays readable");
        assert_eq!(tone(9, true), SHADES - 1, "levels saturate");
    }

    #[test]
    fn ramps_have_distinct_ends() {
        for mode in ColorMode::ALL {
            let stops = ramp_stops(mode);
            assert!(stops.len() >= 2);
            assert_ne!(stops.first(), stops.last(), "{mode:?}");
        }
        assert_eq!(state_color32(0, 5), Color32::TRANSPARENT);
        assert_eq!(state_color32(1, 5), color32(ALIVE_SRGB));
        assert_eq!(dying_stops().len(), 7);
    }
}
