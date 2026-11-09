//! Coat and plumage colouring: countershading plus procedural patterns
//! (stripes, spots, rosettes, reticulated patches, bands, speckles) written
//! into vertex colours. Pattern scales are expressed per metre of the
//! *adult* body so a species looks the same at every age.

use bevy::color::{Color, ColorToComponents};
use bevy::math::{Vec2, Vec3};

use super::mesh::{Rgba, SurfacePoint};
use super::noise::{fbm3, worley2, worley2_f2};

/// Converts a `0xRRGGBB` sRGB colour to linear RGBA.
pub fn srgb(hex: u32) -> Rgba {
    let c = Color::srgb_u8(((hex >> 16) & 0xff) as u8, ((hex >> 8) & 0xff) as u8, (hex & 0xff) as u8);
    c.to_linear().to_f32_array()
}

/// Linear interpolation between two colours.
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// Multiplies the RGB channels by `k`.
pub fn shade(c: Rgba, k: f32) -> Rgba {
    [c[0] * k, c[1] * k, c[2] * k, c[3]]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Pattern drawn over the base colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pattern {
    /// No pattern.
    Plain,
    /// Dorso-ventral stripes (tiger, zebra): `per_m` stripes per metre along
    /// the body, `width` fraction of each period, irregular edges.
    Stripes {
        /// Stripe colour (`0xRRGGBB`).
        color: u32,
        /// Stripes per metre.
        per_m: f32,
        /// Dark fraction of each period (0..1).
        width: f32,
        /// How far towards the belly stripes reach (−1 = everywhere).
        reach: f32,
    },
    /// Solid round spots (cheetah, ladybird, dalmatian-like).
    Spots {
        /// Spot colour.
        color: u32,
        /// Cells per metre.
        per_m: f32,
        /// Spot radius as a fraction of a cell.
        radius: f32,
    },
    /// Ring-shaped rosettes (leopard, jaguar).
    Rosettes {
        /// Rosette rim colour.
        color: u32,
        /// Cells per metre.
        per_m: f32,
    },
    /// Polygonal patches separated by light lines (giraffe, tortoise scutes).
    Reticulated {
        /// Line colour.
        line: u32,
        /// Cells per metre.
        per_m: f32,
    },
    /// Rings around the body along its length (bee abdomen, raccoon tail,
    /// coral snake): alternating with `color` every `per_m`.
    Bands {
        /// Band colour.
        color: u32,
        /// Bands per metre.
        per_m: f32,
    },
    /// Fine dark speckles (trout, sparrow streaks).
    Speckles {
        /// Speckle colour.
        color: u32,
        /// Cells per metre.
        per_m: f32,
    },
    /// Darker dorsal saddle (jackal, German-shepherd-like coats).
    Saddle {
        /// Saddle colour.
        color: u32,
    },
}

/// Colouring of one body surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coat {
    /// Dorsal / main colour (`0xRRGGBB`).
    pub base: u32,
    /// Ventral colour.
    pub belly: u32,
    /// Where countershading starts on the dorso-ventral axis
    /// (1 = no belly colour, 0 = lower half, −1 = everywhere).
    pub countershade: f32,
    /// Overlay pattern.
    pub pattern: Pattern,
}

impl Coat {
    /// Uniform colour.
    pub const fn plain(color: u32) -> Self {
        Self { base: color, belly: color, countershade: 1.0, pattern: Pattern::Plain }
    }

    /// Two-tone countershaded coat.
    pub const fn countershaded(back: u32, belly: u32) -> Self {
        Self { base: back, belly, countershade: -0.2, pattern: Pattern::Plain }
    }

    /// Same coat with another pattern.
    pub const fn with(mut self, pattern: Pattern) -> Self {
        self.pattern = pattern;
        self
    }

    /// Colour at a surface point. `scale` converts local mesh units back to
    /// adult metres (pattern scale); `seed` varies the pattern per species.
    pub fn color_at(&self, sp: &SurfacePoint, scale: f32, seed: u32) -> Rgba {
        let base = srgb(self.base);
        let belly = srgb(self.belly);
        let cs = smoothstep(self.countershade + 0.15, self.countershade - 0.15, sp.dorsal);
        let mut c = mix(base, belly, cs);
        let p = sp.position * scale;
        // Gentle fur-like value variation so large surfaces are not flat.
        let grain = 1.0 + 0.06 * fbm3(p * 9.0, seed ^ 0x51, 2);
        c = shade(c, grain);
        match self.pattern {
            Pattern::Plain => c,
            Pattern::Stripes { color, per_m, width, reach } => {
                let wobble = 0.35 * fbm3(Vec3::new(p.z * 2.0, sp.dorsal * 1.5, p.x), seed, 2);
                let phase = (p.z * per_m + wobble + 0.25 * sp.dorsal.abs()).rem_euclid(1.0);
                let taper = width * smoothstep(reach - 0.2, reach + 0.3, sp.dorsal);
                let stripe = smoothstep(taper, taper - 0.08, (phase - 0.5).abs() * 2.0);
                mix(c, srgb(color), stripe)
            }
            Pattern::Spots { color, per_m, radius } => {
                let (d, _) = worley2(Vec2::new(p.z, p.y + 0.5 * p.x) * per_m, seed);
                let s = smoothstep(radius, radius * 0.8, d);
                mix(c, srgb(color), s * smoothstep(-0.6, -0.2, sp.dorsal))
            }
            Pattern::Rosettes { color, per_m } => {
                let (d, _) = worley2(Vec2::new(p.z, p.y + 0.5 * p.x) * per_m, seed);
                let ring = smoothstep(0.08, 0.0, (d - 0.3).abs());
                mix(c, srgb(color), ring * smoothstep(-0.6, -0.2, sp.dorsal))
            }
            Pattern::Reticulated { line, per_m } => {
                let q = Vec2::new(p.z, p.y + 0.5 * p.x) * per_m;
                let (f1, f2, h) = worley2_f2(q, seed);
                let edge = smoothstep(0.2, 0.12, f2 - f1);
                let tone = 0.85 + 0.25 * ((h & 0xff) as f32 / 255.0);
                mix(shade(c, tone), srgb(line), edge)
            }
            Pattern::Bands { color, per_m } => {
                let phase = (p.z * per_m).rem_euclid(1.0);
                mix(c, srgb(color), smoothstep(0.45, 0.55, phase))
            }
            Pattern::Speckles { color, per_m } => {
                let (d, h) = worley2(Vec2::new(p.z, p.y + 0.7 * p.x) * per_m, seed);
                let keep = (h >> 9) & 1 == 1;
                let s = if keep { smoothstep(0.22, 0.16, d) } else { 0.0 };
                mix(c, srgb(color), s * smoothstep(-0.3, 0.1, sp.dorsal))
            }
            Pattern::Saddle { color } => mix(c, srgb(color), smoothstep(0.35, 0.75, sp.dorsal)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(z: f32, dorsal: f32) -> SurfacePoint {
        SurfacePoint { along: 0.5, around: 0.0, dorsal, lateral: 0.0, position: Vec3::new(0.0, dorsal * 0.2, z) }
    }

    #[test]
    fn countershading_lightens_the_belly() {
        let coat = Coat::countershaded(0x704020, 0xf0e8d8);
        let back = coat.color_at(&point(0.0, 1.0), 1.0, 1);
        let belly = coat.color_at(&point(0.0, -1.0), 1.0, 1);
        assert!(belly[0] + belly[1] + belly[2] > 2.0 * (back[0] + back[1] + back[2]));
    }

    #[test]
    fn stripes_alternate_along_the_body() {
        let coat =
            Coat::plain(0xe08020).with(Pattern::Stripes { color: 0x101010, per_m: 6.0, width: 0.35, reach: -1.0 });
        let lum: Vec<f32> = (0..60)
            .map(|i| {
                let c = coat.color_at(&point(i as f32 * 0.01, 0.8), 1.0, 2);
                c[0] + c[1] + c[2]
            })
            .collect();
        let dark = lum.iter().filter(|l| **l < 0.2).count();
        let light = lum.iter().filter(|l| **l > 0.5).count();
        assert!(dark > 5 && light > 5, "dark {dark} light {light}");
    }

    #[test]
    fn srgb_conversion_is_linear_space() {
        let mid = srgb(0x808080);
        assert!((mid[0] - 0.2158).abs() < 0.01, "{mid:?}");
        assert_eq!(srgb(0xffffff), [1.0, 1.0, 1.0, 1.0]);
    }
}
