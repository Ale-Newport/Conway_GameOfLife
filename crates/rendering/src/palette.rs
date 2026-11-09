//! Colour palettes for discrete cell states and continuous colormaps for
//! scalar fields.
//!
//! * [`GridPalette::States`] maps a cell state byte to one of up to
//!   [`MAX_PALETTE_COLORS`] colours (index 0 is the dead / empty state). States
//!   beyond the last colour use the last colour.
//! * [`GridPalette::Continuous`] maps a normalised value (`state / 255` for the
//!   grid renderer) through a [`Colormap`].
//!
//! Colormap provenance (documented approximations):
//! * `Viridis`, `Inferno`, `Magma`, `Plasma`: sixth-degree polynomial fits of
//!   the matplotlib colormaps (van der Walt & Smith 2015, CC0) published by
//!   Matt Zucker (CC0). Maximum deviation from the 256-entry matplotlib tables
//!   is about 0.02 per channel (checked in the tests at several anchors).
//! * `TurboLike`: a rainbow in the spirit of Google's Turbo (Mikhailov 2019),
//!   defined by nine Emergence Lab colour stops. It is *not* the exact Turbo
//!   table.
//! * `LeniaBlue`: an Emergence Lab navy → blue → cyan → white ramp used for
//!   Lenia fields, in the spirit of the blue renderings in Chan (2019).
//! * `Grayscale`: linear black → white in sRGB.
//!
//! The GPU receives up to 32 colour stops in linear RGB; the shader
//! interpolates linearly between neighbouring stops.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum number of colours (states or colormap stops) the GPU palette holds.
pub const MAX_PALETTE_COLORS: usize = 32;

/// Errors produced when building palettes from user input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteError {
    /// A discrete palette needs at least one colour.
    Empty,
    /// More colours than the GPU palette can hold.
    TooManyColors {
        /// Number of colours requested.
        requested: usize,
    },
    /// A Generations rule needs at least two states.
    TooFewStates {
        /// Number of states requested.
        requested: usize,
    },
}

impl fmt::Display for PaletteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "a state palette needs at least one colour"),
            Self::TooManyColors { requested } => {
                write!(f, "a state palette holds at most {MAX_PALETTE_COLORS} colours, {requested} were given")
            }
            Self::TooFewStates { requested } => {
                write!(f, "a Generations palette needs at least 2 states, {requested} were given")
            }
        }
    }
}

impl std::error::Error for PaletteError {}

/// A continuous colormap for scalar fields in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Colormap {
    /// Perceptually uniform blue → green → yellow (matplotlib).
    #[default]
    Viridis,
    /// Perceptually uniform black → purple → orange → pale yellow (matplotlib).
    Inferno,
    /// Perceptually uniform black → purple → pink → pale yellow (matplotlib).
    Magma,
    /// Perceptually uniform blue → magenta → yellow (matplotlib).
    Plasma,
    /// Rainbow similar to Google's Turbo (Emergence Lab stops, see module docs).
    TurboLike,
    /// Navy → blue → cyan → white ramp for Lenia fields.
    LeniaBlue,
    /// Black → white.
    Grayscale,
}

/// Polynomial coefficients `c0..c6` (per channel) of the matplotlib fits.
type Poly = [[f32; 3]; 7];

const VIRIDIS: Poly = [
    [0.277_727_33, 0.005_407_344_5, 0.334_099_8],
    [0.105_093_04, 1.404_613_5, 1.384_590_2],
    [-0.330_861_83, 0.214_847_56, 0.095_095_16],
    [-4.634_230_5, -5.799_101, -19.332_441],
    [6.228_27, 14.179_933, 56.690_55],
    [4.776_385, -13.745_145, -65.353_035],
    [-5.435_456, 4.645_852_6, 26.312_435],
];

const INFERNO: Poly = [
    [0.000_218_940_37, 0.001_651_004_6, -0.019_480_898],
    [0.106_513_42, 0.563_956_44, 3.932_712_4],
    [11.602_493, -3.972_854, -15.942_394],
    [-41.703_995, 17.436_4, 44.354_145],
    [77.162_94, -33.402_36, -81.807_31],
    [-71.319_43, 32.626_064, 73.209_52],
    [25.131_126, -12.242_669, -23.070_325],
];

const MAGMA: Poly = [
    [-0.002_136_485, -0.000_749_655_05, -0.005_386_128],
    [0.251_660_54, 0.677_523_24, 2.494_026_6],
    [8.353_717, -3.577_719_5, 0.314_467_9],
    [-27.668_733, 14.264_731, -13.649_213],
    [52.176_14, -27.943_606, 12.944_169],
    [-50.768_524, 29.046_583, 4.234_153],
    [18.655_705, -11.489_773, -5.601_961_5],
];

const PLASMA: Poly = [
    [0.058_732_344, 0.023_336_709, 0.543_340_2],
    [2.176_514_6, 0.238_383_42, 0.753_960_45],
    [-2.689_460_5, -7.455_851, 3.110_8],
    [6.130_348, 42.346_188, -28.518_854],
    [-11.107_436, -82.666_31, 60.139_847],
    [10.023_066, 71.413_62, -54.072_186],
    [-3.658_713_8, -22.931_534, 18.191_908],
];

const TURBO_LIKE: [(f32, [f32; 3]); 9] = [
    (0.0, [0.19, 0.07, 0.23]),
    (0.125, [0.27, 0.36, 0.85]),
    (0.25, [0.16, 0.66, 0.98]),
    (0.375, [0.10, 0.89, 0.73]),
    (0.5, [0.43, 0.99, 0.36]),
    (0.625, [0.80, 0.92, 0.21]),
    (0.75, [0.99, 0.65, 0.15]),
    (0.875, [0.90, 0.30, 0.05]),
    (1.0, [0.48, 0.02, 0.01]),
];

const LENIA_BLUE: [(f32, [f32; 3]); 5] = [
    (0.0, [0.02, 0.02, 0.08]),
    (0.25, [0.03, 0.12, 0.35]),
    (0.5, [0.05, 0.35, 0.75]),
    (0.75, [0.35, 0.75, 0.98]),
    (1.0, [0.92, 0.98, 1.0]),
];

fn eval_poly(poly: &Poly, t: f32) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (channel, value) in out.iter_mut().enumerate() {
        // Horner evaluation from the highest coefficient down.
        let mut acc = poly[6][channel];
        for k in (0..6).rev() {
            acc = acc * t + poly[k][channel];
        }
        *value = acc.clamp(0.0, 1.0);
    }
    out
}

fn eval_stops(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
    let mut prev = stops[0];
    for &next in &stops[1..] {
        if t <= next.0 {
            let span = (next.0 - prev.0).max(f32::EPSILON);
            let f = ((t - prev.0) / span).clamp(0.0, 1.0);
            // Weighted form so both endpoints are reproduced exactly.
            return [0, 1, 2].map(|c| prev.1[c] * (1.0 - f) + next.1[c] * f);
        }
        prev = next;
    }
    prev.1
}

impl Colormap {
    /// Every colormap, in display order.
    pub const ALL: [Colormap; 7] =
        [Self::Viridis, Self::Inferno, Self::Magma, Self::Plasma, Self::TurboLike, Self::LeniaBlue, Self::Grayscale];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Viridis => "Viridis",
            Self::Inferno => "Inferno",
            Self::Magma => "Magma",
            Self::Plasma => "Plasma",
            Self::TurboLike => "Turbo-like",
            Self::LeniaBlue => "Lenia blue",
            Self::Grayscale => "Grayscale",
        }
    }

    /// Evaluates the colormap at `t` (clamped to `[0, 1]`) as sRGB components.
    pub fn sample_srgb(self, t: f32) -> [f32; 3] {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            Self::Viridis => eval_poly(&VIRIDIS, t),
            Self::Inferno => eval_poly(&INFERNO, t),
            Self::Magma => eval_poly(&MAGMA, t),
            Self::Plasma => eval_poly(&PLASMA, t),
            Self::TurboLike => eval_stops(&TURBO_LIKE, t),
            Self::LeniaBlue => eval_stops(&LENIA_BLUE, t),
            Self::Grayscale => [t, t, t],
        }
    }

    /// Evaluates the colormap at `t` (clamped to `[0, 1]`).
    pub fn sample(self, t: f32) -> Color {
        let [r, g, b] = self.sample_srgb(t);
        Color::srgb(r, g, b)
    }

    /// `count` evenly spaced colours from 0 to 1 (inclusive). `count` is
    /// clamped to `2..=MAX_PALETTE_COLORS`.
    pub fn stops(self, count: usize) -> Vec<Color> {
        let count = count.clamp(2, MAX_PALETTE_COLORS);
        (0..count).map(|i| self.sample(i as f32 / (count - 1) as f32)).collect()
    }
}

/// How cell states are coloured by the grid renderer.
#[derive(Clone, Debug, PartialEq)]
pub enum GridPalette {
    /// One colour per state (index 0 = dead / empty). At most
    /// [`MAX_PALETTE_COLORS`] colours; larger states use the last colour.
    States(Vec<Color>),
    /// The state byte is a normalised value (`state / 255`) shown through a colormap.
    Continuous(Colormap),
}

impl Default for GridPalette {
    fn default() -> Self {
        Self::life()
    }
}

fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color::srgb_u8(r, g, b)
}

impl GridPalette {
    /// Builds a discrete palette, validating the number of colours.
    pub fn states(colors: Vec<Color>) -> Result<Self, PaletteError> {
        match colors.len() {
            0 => Err(PaletteError::Empty),
            n if n > MAX_PALETTE_COLORS => Err(PaletteError::TooManyColors { requested: n }),
            _ => Ok(Self::States(colors)),
        }
    }

    /// Two-state palette (dead, alive).
    pub fn two_state(dead: Color, alive: Color) -> Self {
        Self::States(vec![dead, alive])
    }

    /// Default Life palette: near-black background, warm white cells.
    pub fn life() -> Self {
        Self::two_state(rgb8(14, 16, 22), rgb8(236, 232, 214))
    }

    /// Generations palette for `states` states (Golly numbering: 0 dead,
    /// 1 alive, 2..states-1 dying). Dying states fade from orange-red to dark
    /// blue. More than [`MAX_PALETTE_COLORS`] states are compressed so that
    /// the last dying states share the final colour.
    pub fn generations(states: usize) -> Result<Self, PaletteError> {
        if states < 2 {
            return Err(PaletteError::TooFewStates { requested: states });
        }
        let len = states.min(MAX_PALETTE_COLORS);
        let mut colors = vec![rgb8(10, 10, 16), rgb8(255, 244, 150)];
        let dying = len.saturating_sub(2);
        let start = Vec3::new(1.0, 0.36, 0.12);
        let end = Vec3::new(0.16, 0.12, 0.45);
        for i in 0..dying {
            let f = if dying == 1 { 0.0 } else { i as f32 / (dying - 1) as f32 };
            let c = start.lerp(end, f);
            colors.push(Color::srgb(c.x, c.y, c.z));
        }
        Ok(Self::States(colors))
    }

    /// Wireworld with Golly's state numbering and colours: 0 empty (dark
    /// grey), 1 electron head (blue), 2 electron tail (white), 3 conductor
    /// (orange).
    pub fn wireworld() -> Self {
        Self::States(vec![rgb8(48, 48, 48), rgb8(0, 128, 255), rgb8(255, 255, 255), rgb8(255, 128, 0)])
    }

    /// Brian's Brain (`B2/S/C3`): 0 ready (black), 1 firing (white),
    /// 2 refractory (blue).
    pub fn brians_brain() -> Self {
        Self::States(vec![rgb8(6, 8, 14), rgb8(235, 242, 255), rgb8(40, 92, 220)])
    }

    /// Saturated colours on black, intended for the Neon style.
    pub fn neon() -> Self {
        Self::States(vec![
            rgb8(4, 4, 10),
            rgb8(40, 255, 230),
            rgb8(255, 60, 200),
            rgb8(255, 230, 60),
            rgb8(90, 140, 255),
            rgb8(140, 255, 90),
        ])
    }

    /// Light, quiet palette intended for the Minimal style.
    pub fn minimal() -> Self {
        Self::two_state(rgb8(242, 240, 233), rgb8(40, 44, 52))
    }

    /// Number of colours (discrete) or GPU stops (continuous).
    pub fn len(&self) -> usize {
        match self {
            Self::States(colors) => colors.len(),
            Self::Continuous(_) => MAX_PALETTE_COLORS,
        }
    }

    /// Whether the palette has no colours (never true for validated palettes).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Colour used for a state byte, exactly as the GPU looks it up
    /// (continuous palettes interpolate between their GPU stops).
    pub fn color(&self, state: u8) -> Color {
        match self {
            Self::States(colors) => {
                // The GPU table holds the first MAX_PALETTE_COLORS entries.
                let used = colors.len().min(MAX_PALETTE_COLORS);
                let index = (state as usize).min(used.saturating_sub(1));
                colors.get(index).copied().unwrap_or(Color::BLACK)
            }
            Self::Continuous(map) => {
                let stops = gpu_stops(&map.stops(MAX_PALETTE_COLORS));
                let color = lerp_table(&stops, MAX_PALETTE_COLORS, state as f32 / 255.0);
                Color::LinearRgba(LinearRgba::new(color.x, color.y, color.z, color.w))
            }
        }
    }

    /// Colour of the dead / zero state.
    pub fn background(&self) -> Color {
        self.color(0)
    }
}

/// Converts colours to the GPU table (linear RGBA), padded to 32 entries.
pub(crate) fn gpu_stops(colors: &[Color]) -> [Vec4; MAX_PALETTE_COLORS] {
    let mut table = [Vec4::ZERO; MAX_PALETTE_COLORS];
    for (slot, color) in table.iter_mut().zip(colors) {
        *slot = color.to_linear().to_vec4();
    }
    table
}

/// CPU twin of the shader's table interpolation (`len` used stops, value in
/// `[0, 1]`).
pub(crate) fn lerp_table(table: &[Vec4; MAX_PALETTE_COLORS], len: usize, value: f32) -> Vec4 {
    let n = len.clamp(1, MAX_PALETTE_COLORS);
    let x = value.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (x.floor() as usize).min(n - 1);
    let j = (i + 1).min(n - 1);
    table[i].lerp(table[j], x - i as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
    }

    /// Reference values from matplotlib's 256-entry `_cm_listed` tables.
    #[test]
    fn polynomial_colormaps_match_matplotlib_anchors() {
        let cases: [(Colormap, f32, [f32; 3]); 12] = [
            (Colormap::Viridis, 0.0, [0.267_004, 0.004_874, 0.329_415]),
            (Colormap::Viridis, 128.0 / 255.0, [0.127_568, 0.566_949, 0.550_556]),
            (Colormap::Viridis, 1.0, [0.993_248, 0.906_157, 0.143_936]),
            (Colormap::Inferno, 0.0, [0.001_462, 0.000_466, 0.013_866]),
            (Colormap::Inferno, 128.0 / 255.0, [0.735_683, 0.215_906, 0.330_245]),
            (Colormap::Inferno, 1.0, [0.988_362, 0.998_364, 0.644_924]),
            (Colormap::Magma, 0.0, [0.001_462, 0.000_466, 0.013_866]),
            (Colormap::Magma, 128.0 / 255.0, [0.716_387, 0.214_982, 0.474_720]),
            (Colormap::Magma, 1.0, [0.987_053, 0.991_438, 0.749_504]),
            (Colormap::Plasma, 0.0, [0.050_383, 0.029_803, 0.527_975]),
            (Colormap::Plasma, 128.0 / 255.0, [0.798_216, 0.280_197, 0.469_538]),
            (Colormap::Plasma, 1.0, [0.940_015, 0.975_158, 0.131_326]),
        ];
        for (map, t, expected) in cases {
            let got = map.sample_srgb(t);
            assert!(close(got, expected, 0.025), "{map:?}({t}) = {got:?}, expected {expected:?}");
        }
    }

    #[test]
    fn colormaps_clamp_and_handle_nan() {
        for map in Colormap::ALL {
            assert_eq!(map.sample_srgb(-3.0), map.sample_srgb(0.0));
            assert_eq!(map.sample_srgb(7.0), map.sample_srgb(1.0));
            assert_eq!(map.sample_srgb(f32::NAN), map.sample_srgb(0.0));
            for i in 0..=20 {
                let c = map.sample_srgb(i as f32 / 20.0);
                assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{map:?} out of gamut: {c:?}");
            }
        }
    }

    #[test]
    fn stop_colormaps_hit_their_stops() {
        assert_eq!(Colormap::TurboLike.sample_srgb(0.5), [0.43, 0.99, 0.36]);
        assert_eq!(Colormap::LeniaBlue.sample_srgb(1.0), [0.92, 0.98, 1.0]);
        let mid = Colormap::LeniaBlue.sample_srgb(0.125);
        assert!(close(mid, [0.025, 0.07, 0.215], 1e-5));
        assert_eq!(Colormap::Grayscale.sample_srgb(0.25), [0.25, 0.25, 0.25]);
    }

    #[test]
    fn viridis_luminance_is_monotonic() {
        let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let mut prev = -1.0;
        for i in 0..=64 {
            let l = lum(Colormap::Viridis.sample_srgb(i as f32 / 64.0));
            assert!(l > prev, "viridis luminance must increase");
            prev = l;
        }
    }

    #[test]
    fn state_palette_validation() {
        assert_eq!(GridPalette::states(vec![]), Err(PaletteError::Empty));
        assert_eq!(GridPalette::states(vec![Color::BLACK; 33]), Err(PaletteError::TooManyColors { requested: 33 }));
        assert!(GridPalette::states(vec![Color::BLACK; 32]).is_ok());
        assert_eq!(GridPalette::generations(1), Err(PaletteError::TooFewStates { requested: 1 }));
    }

    #[test]
    fn state_lookup_clamps_to_last_colour() {
        let palette = GridPalette::wireworld();
        assert_eq!(palette.len(), 4);
        assert_eq!(palette.color(1), Color::srgb_u8(0, 128, 255));
        assert_eq!(palette.color(3), Color::srgb_u8(255, 128, 0));
        assert_eq!(palette.color(200), palette.color(3));
        assert_eq!(palette.background(), Color::srgb_u8(48, 48, 48));
        // An unvalidated oversized palette is looked up like the GPU table,
        // which only holds the first 32 colours.
        let oversized = GridPalette::States((0..40u8).map(|i| Color::srgb_u8(i, 0, 0)).collect());
        assert_eq!(oversized.color(35), Color::srgb_u8(31, 0, 0));
        let gpu = gpu_stops(&[Color::srgb_u8(31, 0, 0)]);
        assert_eq!(oversized.color(35).to_linear().to_vec4(), gpu[0]);
    }

    #[test]
    fn generations_palette_sizes() {
        let p = GridPalette::generations(3).expect("valid");
        assert_eq!(p.len(), 3);
        let p = GridPalette::generations(100).expect("valid");
        assert_eq!(p.len(), MAX_PALETTE_COLORS);
        // Dying colours fade monotonically towards dark blue (red channel decreases).
        let reds: Vec<f32> = (2..MAX_PALETTE_COLORS as u8).map(|s| p.color(s).to_srgba().red).collect();
        assert!(reds.windows(2).all(|w| w[1] < w[0]));
    }

    #[test]
    fn continuous_palette_uses_gpu_interpolation() {
        let palette = GridPalette::Continuous(Colormap::Grayscale);
        let mid = palette.color(128).to_srgba();
        assert!((mid.red - 128.0 / 255.0).abs() < 0.01, "got {mid:?}");
        let end = palette.color(255).to_srgba();
        assert!((end.red - 1.0).abs() < 1e-4);
    }

    #[test]
    fn table_interpolation_matches_endpoints() {
        let colors = [Color::BLACK, Color::WHITE];
        let table = gpu_stops(&colors);
        assert_eq!(lerp_table(&table, 2, 0.0), Vec4::new(0.0, 0.0, 0.0, 1.0));
        assert_eq!(lerp_table(&table, 2, 1.0), Vec4::ONE);
        assert_eq!(lerp_table(&table, 2, 0.5), Vec4::new(0.5, 0.5, 0.5, 1.0));
        assert_eq!(lerp_table(&table, 1, 0.9), table[0]);
    }
}
