//! Colours: the living ground (soil by biome under vegetation cover, snow,
//! ash and fire), the data overlays with their legends, the sky and the
//! sun from the model's calendar and weather.
//!
//! Ground colours are presentation; every input is a model field (biome,
//! vegetation biomass in kg/m², snow water equivalent, soil moisture, fire
//! state). Covers use `1 − e^{−B/B½}`, a display curve that shows sparse
//! vegetation and saturates on dense swards.

use bevy::color::{Color, ColorToComponents, LinearRgba, Mix};
use bevy::math::{Vec2, Vec3};
use physics_world::{Biome, FireState};
use rendering::Colormap;

/// Biomass (kg/m²) at which a layer covers `1 − 1/e` of the ground.
pub const HERB_COVER_HALF: f32 = 0.12;
/// Same for shrubs.
pub const SHRUB_COVER_HALF: f32 = 0.25;
/// Snow water equivalent (m) at which snow covers `1 − 1/e` of the ground.
pub const SNOW_COVER_HALF: f32 = 0.004;
/// Days over which ash fades by `1/e`.
pub const ASH_FADE_DAYS: f32 = 40.0;

fn srgb(r: f32, g: f32, b: f32) -> LinearRgba {
    Color::srgb(r, g, b).to_linear()
}

/// Bare ground colour of a biome (soil, sand, rock, sediment).
pub fn soil_color(biome: Biome) -> LinearRgba {
    match biome {
        Biome::Ocean | Biome::Reef => srgb(0.74, 0.68, 0.52),
        Biome::Lake | Biome::River => srgb(0.36, 0.32, 0.25),
        Biome::Beach => srgb(0.86, 0.80, 0.62),
        Biome::Wetland => srgb(0.30, 0.29, 0.21),
        Biome::Desert => srgb(0.82, 0.69, 0.47),
        Biome::Grassland => srgb(0.47, 0.40, 0.28),
        Biome::Savanna => srgb(0.66, 0.55, 0.36),
        Biome::TemperateForest => srgb(0.34, 0.27, 0.19),
        Biome::TropicalRainforest => srgb(0.29, 0.22, 0.15),
        Biome::Tundra => srgb(0.55, 0.54, 0.48),
        Biome::Mountain => srgb(0.50, 0.48, 0.45),
    }
}

/// Everything that colours one terrain sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundSample {
    /// Biome.
    pub biome: Biome,
    /// Herb layers: grass and flowers (kg/m²).
    pub herbs: f32,
    /// Mosses and lichens of the ground layer (kg/m²).
    pub moss: f32,
    /// Flowers alone (kg/m²).
    pub flowers: f32,
    /// Shrubs (kg/m²).
    pub shrubs: f32,
    /// Desert plants (kg/m²).
    pub desert: f32,
    /// Snow water equivalent (m).
    pub snow_m: f32,
    /// Relative soil saturation.
    pub moisture: f32,
    /// Fire state.
    pub fire: FireState,
}

/// Cover fraction of a biomass with half-cover `half`.
pub fn cover(biomass: f32, half: f32) -> f32 {
    if biomass <= 0.0 || half <= 0.0 { 0.0 } else { 1.0 - (-biomass / half).exp() }
}

fn mix(a: LinearRgba, b: LinearRgba, t: f32) -> LinearRgba {
    a.mix(&b, t.clamp(0.0, 1.0))
}

/// Ground colour (linear) of a sample. `grass` is the seasonal grass
/// colour (`organism_render::PlantSeason::grass`), `tick` the current tick
/// and `ticks_per_day` the calendar's (for fading ash).
pub fn ground_color(s: &GroundSample, grass: LinearRgba, tick: u64, ticks_per_day: u32) -> LinearRgba {
    let mut c = soil_color(s.biome);
    // Wet soil is darker.
    c = mix(c, c * 0.62, s.moisture.clamp(0.0, 1.0) * 0.45);
    // Mosses and lichens: an olive mat under the herbs.
    c = mix(c, srgb(0.34, 0.37, 0.19), cover(s.moss, HERB_COVER_HALF) * 0.8);
    let dry_grass = matches!(s.biome, Biome::Savanna | Biome::Desert);
    let herb_color = if dry_grass { mix(grass, srgb(0.78, 0.68, 0.40), 0.4) } else { grass };
    c = mix(c, herb_color, cover(s.herbs, HERB_COVER_HALF) * 0.95);
    c = mix(c, srgb(0.86, 0.74, 0.34), cover(s.flowers, HERB_COVER_HALF * 2.0) * 0.18);
    c = mix(c, mix(herb_color, srgb(0.13, 0.25, 0.10), 0.65), cover(s.shrubs, SHRUB_COVER_HALF) * 0.75);
    c = mix(c, srgb(0.52, 0.53, 0.37), cover(s.desert, HERB_COVER_HALF) * 0.7);
    // Ash and char lie on the burned ground and on what is left of its
    // plants, and fade as the cell recovers (regrowth shows through).
    let ash = srgb(0.07, 0.065, 0.06);
    match s.fire {
        FireState::Burned { since } => {
            let days = tick.saturating_sub(since) as f32 / ticks_per_day.max(1) as f32;
            c = mix(c, ash, 0.92 * (-days / ASH_FADE_DAYS).exp());
        }
        // Smouldering cells (low intensity) stay mostly ash with a red glow.
        FireState::Burning { intensity, .. } => {
            c = mix(c, ash, 0.8);
            c = mix(c, srgb(0.95, 0.30, 0.05), 0.2 + 0.6 * intensity.clamp(0.0, 1.0));
        }
        FireState::Lava { .. } => c = srgb(1.0, 0.28, 0.03),
        _ => {}
    }
    mix(c, srgb(0.93, 0.95, 0.98), cover(s.snow_m, SNOW_COVER_HALF))
}

/// A data overlay draped over the terrain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Overlay {
    /// No overlay.
    #[default]
    None,
    /// Air temperature.
    Temperature,
    /// Relative soil saturation.
    Moisture,
    /// Surface water depth.
    SurfaceWater,
    /// Soil mineral nutrients.
    Nutrients,
    /// Plant biomass (all vegetation layers).
    PlantBiomass,
    /// Fire spread risk.
    FireRisk,
    /// Territories of territorial adults (circles).
    Territories,
    /// Share of animals that are infectious.
    Disease,
    /// Ant trail pheromone.
    Pheromone,
    /// Flower pollination index.
    Pollination,
}

impl Overlay {
    /// Every overlay in display order.
    pub const ALL: [Overlay; 11] = [
        Self::None,
        Self::Temperature,
        Self::Moisture,
        Self::SurfaceWater,
        Self::Nutrients,
        Self::PlantBiomass,
        Self::FireRisk,
        Self::Territories,
        Self::Disease,
        Self::Pheromone,
        Self::Pollination,
    ];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Temperature => "Temperature",
            Self::Moisture => "Soil moisture",
            Self::SurfaceWater => "Surface water",
            Self::Nutrients => "Soil nutrients",
            Self::PlantBiomass => "Plant biomass",
            Self::FireRisk => "Fire risk",
            Self::Territories => "Territories",
            Self::Disease => "Disease prevalence",
            Self::Pheromone => "Ant trails",
            Self::Pollination => "Pollination",
        }
    }

    /// What the overlay shows and where the numbers come from.
    pub fn explanation(self) -> &'static str {
        match self {
            Self::None => "The landscape as it looks: soil, vegetation cover, snow, ash and water.",
            Self::Temperature => {
                "Air temperature of each cell (°C) from the physics world: climate, season, hour, \
                                  elevation and weather."
            }
            Self::Moisture => {
                "Relative saturation of the soil (0 = dry, 1 = saturated). Plants wilt below their \
                               wilting point; drought dieback turns forage into litter."
            }
            Self::SurfaceWater => "Depth of standing and flowing water (m): rivers, lakes, the sea and floods.",
            Self::Nutrients => {
                "Mineral nutrients in the soil (g/m²), the pool plants take up and decomposers \
                                replenish."
            }
            Self::PlantBiomass => {
                "Dry plant matter of all vegetation layers (kg/m²): grass, shrubs, flowers, moss, \
                                   desert plants and aquatic plants (trees not included)."
            }
            Self::FireRisk => {
                "Probability that the cell catches fire in one fire sub-step from a burning upwind \
                               neighbour, from the physics world's spread rule with the current wind, fuel load and \
                               fuel moisture (0 where the fuel is below the burnable minimum)."
            }
            Self::Territories => "Territories of territorial adults: centre and radius as kept by the model.",
            Self::Disease => {
                "Share of the pathogens' host animals that are infectious, per block of about 100 m \
                 (blocks without hosts are clear)."
            }
            Self::Pheromone => "Trail pheromone laid by loaded ant foragers; it evaporates and diffuses.",
            Self::Pollination => "Pollination index of the flower layer (visits per flower, halving daily).",
        }
    }

    /// Whether the overlay is drawn on the terrain grid (not as shapes).
    pub fn is_grid(self) -> bool {
        !matches!(self, Self::None | Self::Territories)
    }
}

/// Colour ramp of a grid overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ramp {
    /// Blue – white – red, centred on `centre`.
    Diverging,
    /// A `rendering` colormap.
    Map(Colormap),
    /// Pale yellow to deep green.
    Greens,
    /// Pale cream to deep red.
    Reds,
    /// Yellow through orange to dark red (heat, fire).
    Heat,
}

impl Ramp {
    /// Colour at `t` in `[0, 1]` (sRGB components).
    pub fn sample(self, t: f32) -> [f32; 3] {
        let t = t.clamp(0.0, 1.0);
        let lerp = |stops: &[[f32; 3]]| {
            let x = t * (stops.len() - 1) as f32;
            let i = (x.floor() as usize).min(stops.len() - 2);
            let f = x - i as f32;
            let (a, b) = (stops[i], stops[i + 1]);
            [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
        };
        match self {
            Self::Diverging => lerp(&[
                [0.13, 0.25, 0.62],
                [0.36, 0.60, 0.86],
                [0.95, 0.95, 0.93],
                [0.94, 0.58, 0.32],
                [0.70, 0.12, 0.13],
            ]),
            Self::Map(map) => map.sample_srgb(t),
            Self::Greens => lerp(&[[0.97, 0.96, 0.78], [0.62, 0.82, 0.42], [0.20, 0.55, 0.24], [0.04, 0.28, 0.12]]),
            Self::Reds => lerp(&[[0.98, 0.90, 0.80], [0.96, 0.55, 0.40], [0.78, 0.12, 0.12], [0.45, 0.02, 0.05]]),
            Self::Heat => lerp(&[[1.0, 0.93, 0.45], [0.99, 0.66, 0.20], [0.90, 0.28, 0.08], [0.55, 0.04, 0.04]]),
        }
    }

    /// `n` colours along the ramp (legend).
    pub fn stops(self, n: usize) -> Vec<[f32; 3]> {
        (0..n.max(2)).map(|i| self.sample(i as f32 / (n.max(2) - 1) as f32)).collect()
    }
}

/// How a grid overlay maps values to colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayScale {
    /// Value at the low end.
    pub min: f32,
    /// Value at the high end.
    pub max: f32,
    /// Colour ramp.
    pub ramp: Ramp,
    /// Unit shown in the legend.
    pub unit: &'static str,
    /// Sparse quantity (zero almost everywhere): zero is drawn clear and
    /// small values faintly, so the landscape shows where there is nothing.
    pub fade: bool,
}

impl OverlayScale {
    /// Opacity of a value for an overlay drawn at `opacity`.
    pub fn alpha(&self, value: f32, opacity: f32) -> f32 {
        if !self.fade {
            return opacity;
        }
        let t = self.position(value);
        if t <= 0.0 { 0.0 } else { opacity * (0.3 + 0.7 * t.sqrt()) }
    }

    /// Position of a value on the ramp.
    pub fn position(&self, value: f32) -> f32 {
        if self.max > self.min { ((value - self.min) / (self.max - self.min)).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Colour of a value (sRGB).
    pub fn color(&self, value: f32) -> [f32; 3] {
        self.ramp.sample(self.position(value))
    }
}

/// Scale of an overlay for the values currently shown (`observed` is the
/// smallest and largest value on the map; fixed ranges ignore it).
/// Temperature spans the map's own range (at least 6 °C) so that the
/// pattern of a mild day is visible.
pub fn overlay_scale(overlay: Overlay, observed: (f32, f32)) -> OverlayScale {
    let observed_max = observed.1;
    let dynamic = |floor: f32| observed_max.max(floor);
    match overlay {
        Overlay::Temperature => {
            let (lo, hi) = if observed.0.is_finite() && observed.1.is_finite() && observed.0 <= observed.1 {
                observed
            } else {
                (-20.0, 40.0)
            };
            let (centre, half) = (0.5 * (lo + hi), (0.5 * (hi - lo)).max(3.0));
            OverlayScale { min: centre - half, max: centre + half, ramp: Ramp::Diverging, unit: "°C", fade: false }
        }
        Overlay::Moisture => {
            OverlayScale { min: 0.0, max: 1.0, ramp: Ramp::Map(Colormap::Viridis), unit: "", fade: false }
        }
        Overlay::SurfaceWater => OverlayScale {
            min: 0.0,
            max: dynamic(0.5).min(10.0),
            ramp: Ramp::Map(Colormap::Viridis),
            unit: "m",
            fade: true,
        },
        Overlay::Nutrients => {
            OverlayScale { min: 0.0, max: dynamic(0.1), ramp: Ramp::Map(Colormap::Magma), unit: "g/m²", fade: false }
        }
        Overlay::PlantBiomass => {
            OverlayScale { min: 0.0, max: dynamic(0.05), ramp: Ramp::Greens, unit: "kg/m²", fade: false }
        }
        // A probability: most of a wet or burned map is at zero.
        Overlay::FireRisk => OverlayScale { min: 0.0, max: 1.0, ramp: Ramp::Heat, unit: "", fade: true },
        // Blocks with healthy hosts stay visible (pale): prevalence 0 is information.
        Overlay::Disease => OverlayScale { min: 0.0, max: 1.0, ramp: Ramp::Reds, unit: "", fade: false },
        Overlay::Pheromone => {
            OverlayScale { min: 0.0, max: dynamic(1e-6), ramp: Ramp::Map(Colormap::Plasma), unit: "", fade: true }
        }
        Overlay::Pollination => {
            OverlayScale { min: 0.0, max: dynamic(1e-3), ramp: Ramp::Map(Colormap::Plasma), unit: "", fade: true }
        }
        Overlay::None | Overlay::Territories => {
            OverlayScale { min: 0.0, max: 1.0, ramp: Ramp::Map(Colormap::Viridis), unit: "", fade: false }
        }
    }
}

/// Lighting of the scene for a moment of the model's calendar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyState {
    /// Unit vector towards the sun (or the moon at night).
    pub light_direction: Vec3,
    /// Directional light illuminance (lux).
    pub illuminance: f32,
    /// Directional light colour (sRGB).
    pub light_color: [f32; 3],
    /// Ambient brightness.
    pub ambient: f32,
    /// Ambient colour (sRGB).
    pub ambient_color: [f32; 3],
    /// Sky / clear colour (sRGB).
    pub sky: [f32; 3],
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Sky and light from the model's daylight factor (a sun-elevation proxy in
/// `[0, 1]`), hour of day and cloud cover. The sun rises in the east (+x),
/// culminates in the south (+z, rows grow southwards) and sets in the
/// west; its elevation is `asin(daylight)`. At night a dim, bluish
/// moonlight keeps the landscape readable.
pub fn sky_state(daylight: f32, hour: f32, cloud_cover: f32) -> SkyState {
    let d = daylight.clamp(0.0, 1.0);
    let clouds = cloud_cover.clamp(0.0, 1.0);
    let azimuth = std::f32::consts::PI * (hour - 6.0) / 12.0;
    let horizontal = Vec2::new(azimuth.cos(), azimuth.sin());
    if d > 0.0 {
        let elevation = d.asin().max(0.06);
        let dir = Vec3::new(horizontal.x * elevation.cos(), elevation.sin(), horizontal.y * elevation.cos());
        // Low sun: warm light and an orange horizon.
        let warmth = (1.0 - d / 0.35).clamp(0.0, 1.0);
        let day_sky = lerp3([0.50, 0.68, 0.90], [0.66, 0.70, 0.74], clouds);
        let sky = lerp3(day_sky, [0.86, 0.56, 0.38], warmth * 0.8);
        SkyState {
            light_direction: dir.normalize(),
            illuminance: (1_500.0 + 10_500.0 * d.sqrt()) * (1.0 - 0.55 * clouds),
            light_color: lerp3([1.0, 0.97, 0.90], [1.0, 0.70, 0.45], warmth),
            ambient: 220.0 + 260.0 * d,
            ambient_color: lerp3([0.80, 0.86, 1.0], [0.95, 0.80, 0.70], warmth * 0.5),
            sky,
        }
    } else {
        // The moon stands opposite the sun's night position.
        let dir = Vec3::new(-horizontal.x * 0.6, 0.75, -horizontal.y * 0.6 + 0.2);
        SkyState {
            light_direction: dir.normalize(),
            illuminance: 1_400.0 * (1.0 - 0.6 * clouds),
            light_color: [0.62, 0.72, 1.0],
            ambient: 210.0,
            ambient_color: [0.45, 0.55, 0.85],
            sky: lerp3([0.035, 0.05, 0.11], [0.08, 0.09, 0.12], clouds),
        }
    }
}

/// Linear RGBA of an sRGB triple.
pub fn linear(rgb: [f32; 3], alpha: f32) -> [f32; 4] {
    let c = Color::srgba(rgb[0], rgb[1], rgb[2], alpha).to_linear();
    c.to_f32_array()
}

/// Distinct colour for the `i`-th of many categories (species, groups):
/// golden-angle hues at fixed saturation and value (sRGB).
pub fn category_color(i: usize) -> [f32; 3] {
    let hue = (i as f32 * 137.508).rem_euclid(360.0);
    let c = Color::hsv(hue, 0.62, 0.92).to_srgba();
    [c.red, c.green, c.blue]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(biome: Biome) -> GroundSample {
        GroundSample {
            biome,
            herbs: 0.0,
            moss: 0.0,
            flowers: 0.0,
            shrubs: 0.0,
            desert: 0.0,
            snow_m: 0.0,
            moisture: 0.0,
            fire: FireState::Fuel,
        }
    }

    fn green() -> LinearRgba {
        srgb(0.3, 0.6, 0.2)
    }

    #[test]
    fn vegetation_greens_and_snow_whitens_the_ground() {
        let bare = ground_color(&sample(Biome::Grassland), green(), 0, 240);
        let grassy = ground_color(&GroundSample { herbs: 0.4, ..sample(Biome::Grassland) }, green(), 0, 240);
        assert!(grassy.green > bare.green && grassy.green > grassy.red, "{grassy:?}");
        let snowy = ground_color(&GroundSample { snow_m: 0.05, ..sample(Biome::Grassland) }, green(), 0, 240);
        assert!(snowy.red > 0.75 && snowy.blue > 0.75, "{snowy:?}");
        let wet = ground_color(&GroundSample { moisture: 1.0, ..sample(Biome::Grassland) }, green(), 0, 240);
        assert!(wet.red < bare.red, "wet soil is darker");
    }

    #[test]
    fn ash_fades_and_fire_glows() {
        let burned = |since: u64, tick: u64| {
            ground_color(
                &GroundSample { fire: FireState::Burned { since }, ..sample(Biome::Grassland) },
                green(),
                tick,
                240,
            )
        };
        let fresh = burned(100, 100);
        let old = burned(0, 240 * 200);
        // Fresh ash covers the few plants left; regrowth shows as it fades.
        let stubble = |since: u64, tick: u64| {
            ground_color(
                &GroundSample {
                    fire: FireState::Burned { since },
                    herbs: 0.03,
                    moss: 0.02,
                    ..sample(Biome::Grassland)
                },
                green(),
                tick,
                240,
            )
        };
        assert!(stubble(100, 100).green < 0.03, "{:?}", stubble(100, 100));
        assert!(stubble(0, 240 * 60).green > 2.0 * stubble(100, 100).green);
        let mossy = ground_color(&GroundSample { moss: 0.3, ..sample(Biome::Grassland) }, green(), 0, 240);
        let grassy = ground_color(&GroundSample { herbs: 0.3, ..sample(Biome::Grassland) }, green(), 0, 240);
        assert!(mossy.green < grassy.green, "moss is an olive mat, darker than grass");
        let bare = ground_color(&sample(Biome::Grassland), green(), 0, 240);
        assert!(fresh.red < 0.05, "{fresh:?}");
        assert!((old.red - bare.red).abs() < 0.01, "ash fades after months");
        let fire = ground_color(
            &GroundSample { fire: FireState::Burning { intensity: 1.0, steps: 1 }, ..sample(Biome::Grassland) },
            green(),
            0,
            240,
        );
        assert!(fire.red > 0.5 && fire.red > 3.0 * fire.blue, "{fire:?}");
        assert!(cover(0.0, 0.1) == 0.0 && cover(10.0, 0.1) > 0.99);
    }

    #[test]
    fn overlay_scales_and_ramps() {
        let t = overlay_scale(Overlay::Temperature, (-20.0, 40.0));
        assert_eq!(t.position(10.0), 0.5);
        assert_eq!(t.position(-100.0), 0.0);
        let mid = t.color(10.0);
        assert!(mid.iter().all(|c| *c > 0.85), "diverging ramp is white in the middle: {mid:?}");
        assert!(t.color(-20.0)[2] > t.color(-20.0)[0], "cold is blue");
        assert!(t.color(40.0)[0] > t.color(40.0)[2], "hot is red");
        let mild = overlay_scale(Overlay::Temperature, (10.0, 11.0));
        assert_eq!((mild.min, mild.max), (7.5, 13.5), "at least 6 °C around the map's range");
        let b = overlay_scale(Overlay::PlantBiomass, (0.0, 1.2));
        assert_eq!(b.max, 1.2, "dynamic range follows the map");
        assert_eq!(overlay_scale(Overlay::PlantBiomass, (0.0, 0.0)).max, 0.05, "with a floor");
        for o in Overlay::ALL {
            assert!(!o.label().is_empty() && !o.explanation().is_empty());
        }
        assert!(!Overlay::Territories.is_grid() && Overlay::FireRisk.is_grid());
        assert_eq!(Ramp::Greens.stops(5).len(), 5);
        // Sparse overlays are clear at zero and opaque at the top of their range.
        let fire = overlay_scale(Overlay::FireRisk, (0.0, 1.0));
        assert_eq!(fire.alpha(0.0, 0.7), 0.0);
        assert!(fire.alpha(0.05, 0.7) > 0.0 && fire.alpha(0.05, 0.7) < fire.alpha(0.8, 0.7));
        assert_eq!(fire.alpha(1.0, 0.7), 0.7);
        assert!(fire.color(0.0).iter().sum::<f32>() > 1.5, "no risk is pale, not black");
        assert_eq!(overlay_scale(Overlay::Moisture, (0.0, 1.0)).alpha(0.0, 0.7), 0.7, "fields stay opaque");
    }

    #[test]
    fn sky_follows_the_day() {
        let noon = sky_state(0.9, 12.0, 0.0);
        assert!(noon.light_direction.y > 0.8 && noon.light_direction.z > 0.0, "high in the south");
        let morning = sky_state(0.2, 7.0, 0.0);
        assert!(morning.light_direction.x > 0.5, "sun in the east in the morning");
        assert!(morning.light_color[2] < noon.light_color[2], "warmer light when low");
        let night = sky_state(0.0, 1.0, 0.0);
        assert!(night.illuminance < noon.illuminance / 5.0);
        assert!(night.sky[2] < 0.2);
        let cloudy = sky_state(0.9, 12.0, 1.0);
        assert!(cloudy.illuminance < noon.illuminance);
        assert_ne!(category_color(0), category_color(1));
    }
}
