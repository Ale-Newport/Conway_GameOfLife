//! World configuration and biome presets.

use serde::{Deserialize, Serialize};

use crate::error::WorldError;

/// Highest land elevation (metres) reached with `relief = 1`.
pub const MAX_RELIEF_M: f32 = 4000.0;
/// Lowest land peak (metres) used with `relief = 0`.
pub const MIN_RELIEF_M: f32 = 150.0;
/// Depth of the deepest ocean cell in metres.
pub const MAX_OCEAN_DEPTH_M: f32 = 1500.0;

/// Parameters of world generation, climate and dynamics.
///
/// Every field has a documented range; [`WorldConfig::validate`] rejects
/// values outside it and [`WorldConfig::sanitized`] clamps them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldConfig {
    /// Width in cells, `8..=4096`.
    pub width: usize,
    /// Height in cells, `8..=4096`.
    pub height: usize,
    /// Master seed; every subsystem derives an independent stream from it.
    pub seed: u64,
    /// Horizontal size of one cell in metres, `1..=10000`.
    pub cell_size_m: f32,
    /// Approximate fraction of the map below sea level, `0..=0.95`. With
    /// `island = true` the whole border is additionally forced under water.
    pub sea_level: f32,
    /// Vertical relief, `0..=1`: the highest peak reaches
    /// `150 m + relief × 3850 m`.
    pub relief: f32,
    /// Terrain roughness, `0..=1` (fBm persistence and domain-warp strength).
    pub roughness: f32,
    /// Continentality, `0..=1`: weight of the low-frequency continent noise
    /// (large, coherent land masses) against detail noise.
    pub continentality: f32,
    /// Apply a radial falloff so that land forms a central island.
    pub island: bool,
    /// Number of river sources to place, `0..=200`.
    pub river_count: u32,
    /// Lake tendency, `0..=1`: deepens closed basins and lowers the minimum
    /// depth a depression needs to be kept as a lake instead of being filled.
    pub lake_tendency: f32,
    /// Annual mean sea-level temperature at the middle row, °C (`-40..=45`).
    pub base_temperature_c: f32,
    /// Temperature difference between the bottom (warm) and top (cold) rows,
    /// °C (`-60..=60`; negative puts the cold side at the bottom).
    pub latitude_gradient: f32,
    /// Mean annual precipitation over land, mm (`0..=8000`), expressed as a
    /// rate of `rainfall_mm / 365` per simulated day.
    pub rainfall_mm: f32,
    /// Seasonal temperature amplitude (half of summer–winter range), °C (`0..=40`).
    pub seasonality: f32,
    /// Diurnal temperature range (day–night), °C (`0..=40`).
    pub diurnal_range_c: f32,
    /// Environmental lapse rate, °C per km of elevation (`0..=15`, default 6.5).
    pub lapse_rate_c_per_km: f32,
    /// Prevailing wind speed, m/s (`0..=60`).
    pub wind_speed: f32,
    /// Direction the prevailing wind blows *from*, degrees clockwise from
    /// north (top of the map); 270 = westerly wind blowing towards +x.
    pub wind_from_deg: f32,
    /// Ticks per simulated day (`1..=14400`, default 240 = 6-minute ticks).
    pub ticks_per_day: u32,
    /// Days per season (`1..=3650`); a year has four seasons.
    pub days_per_season: u32,
    /// Multiplier on the fire-spread probability (`0..=5`).
    pub fire_spread: f32,
    /// Cloud-to-ground lightning strikes per km² per hour of storm (`0..=10`).
    pub lightning_rate: f32,
    /// Multiplier on potential evaporation (`0..=10`).
    pub evaporation_scale: f32,
    /// Surface water depth (m) above which a dry-land cell counts as flooded.
    pub flood_threshold_m: f32,
    /// Optional preset whose climate/terrain values replace the fields above
    /// when the world is generated (see [`WorldConfig::effective`]).
    pub biome_override: Option<BiomePreset>,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            width: 256,
            height: 256,
            seed: 1,
            cell_size_m: 50.0,
            sea_level: 0.35,
            relief: 0.45,
            roughness: 0.5,
            continentality: 0.5,
            island: false,
            river_count: 8,
            lake_tendency: 0.35,
            base_temperature_c: 14.0,
            latitude_gradient: 12.0,
            rainfall_mm: 900.0,
            seasonality: 10.0,
            diurnal_range_c: 10.0,
            lapse_rate_c_per_km: 6.5,
            wind_speed: 4.0,
            wind_from_deg: 270.0,
            ticks_per_day: 240,
            days_per_season: 30,
            fire_spread: 1.0,
            lightning_rate: 0.02,
            evaporation_scale: 1.0,
            flood_threshold_m: 0.5,
            biome_override: None,
        }
    }
}

/// Admissible range of every float field: `(name, getter, min, max)`.
type FloatField = (&'static str, fn(&mut WorldConfig) -> &mut f32, f32, f32);

const FLOAT_FIELDS: [FloatField; 17] = [
    ("cell_size_m", |c| &mut c.cell_size_m, 1.0, 10_000.0),
    ("sea_level", |c| &mut c.sea_level, 0.0, 0.95),
    ("relief", |c| &mut c.relief, 0.0, 1.0),
    ("roughness", |c| &mut c.roughness, 0.0, 1.0),
    ("continentality", |c| &mut c.continentality, 0.0, 1.0),
    ("lake_tendency", |c| &mut c.lake_tendency, 0.0, 1.0),
    ("base_temperature_c", |c| &mut c.base_temperature_c, -40.0, 45.0),
    ("latitude_gradient", |c| &mut c.latitude_gradient, -60.0, 60.0),
    ("rainfall_mm", |c| &mut c.rainfall_mm, 0.0, 8000.0),
    ("seasonality", |c| &mut c.seasonality, 0.0, 40.0),
    ("diurnal_range_c", |c| &mut c.diurnal_range_c, 0.0, 40.0),
    ("lapse_rate_c_per_km", |c| &mut c.lapse_rate_c_per_km, 0.0, 15.0),
    ("wind_speed", |c| &mut c.wind_speed, 0.0, 60.0),
    ("wind_from_deg", |c| &mut c.wind_from_deg, 0.0, 360.0),
    ("fire_spread", |c| &mut c.fire_spread, 0.0, 5.0),
    ("lightning_rate", |c| &mut c.lightning_rate, 0.0, 10.0),
    ("evaporation_scale", |c| &mut c.evaporation_scale, 0.0, 10.0),
];

/// Range of `flood_threshold_m` (kept separate so the table above stays a
/// plain list of climate/terrain factors).
const FLOOD_THRESHOLD_RANGE: (f32, f32) = (0.01, 10.0);
/// Range of the world dimensions.
pub const SIZE_RANGE: (usize, usize) = (8, 4096);
/// Maximum number of river sources.
pub const MAX_RIVERS: u32 = 200;

impl WorldConfig {
    /// Default configuration with a different size.
    pub fn with_size(width: usize, height: usize) -> Self {
        Self { width, height, ..Self::default() }
    }

    /// Builder: sets the seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Builder: selects a preset (applied by [`WorldConfig::effective`]).
    pub fn with_preset(mut self, preset: BiomePreset) -> Self {
        self.biome_override = Some(preset);
        self
    }

    /// The configuration actually used for generation: a copy of `self`
    /// with the preset's values written over the fields it controls.
    pub fn effective(&self) -> WorldConfig {
        let mut cfg = self.clone();
        if let Some(preset) = self.biome_override {
            preset.apply(&mut cfg);
        }
        cfg
    }

    /// Checks every field, returning the first violation.
    pub fn validate(&self) -> Result<(), WorldError> {
        let (lo, hi) = SIZE_RANGE;
        if !(lo..=hi).contains(&self.width) {
            return Err(WorldError::config("width", format!("{} is outside [{lo}, {hi}]", self.width)));
        }
        if !(lo..=hi).contains(&self.height) {
            return Err(WorldError::config("height", format!("{} is outside [{lo}, {hi}]", self.height)));
        }
        let mut probe = self.clone();
        for (name, field, min, max) in FLOAT_FIELDS {
            let value = *field(&mut probe);
            if !value.is_finite() || value < min || value > max {
                return Err(WorldError::config(name, format!("{value} is outside [{min}, {max}]")));
            }
        }
        let (fmin, fmax) = FLOOD_THRESHOLD_RANGE;
        if !self.flood_threshold_m.is_finite() || self.flood_threshold_m < fmin || self.flood_threshold_m > fmax {
            return Err(WorldError::config(
                "flood_threshold_m",
                format!("{} is outside [{fmin}, {fmax}]", self.flood_threshold_m),
            ));
        }
        if self.river_count > MAX_RIVERS {
            return Err(WorldError::config("river_count", format!("{} exceeds {MAX_RIVERS}", self.river_count)));
        }
        if !(1..=14_400).contains(&self.ticks_per_day) {
            return Err(WorldError::config("ticks_per_day", format!("{} is outside [1, 14400]", self.ticks_per_day)));
        }
        if !(1..=3650).contains(&self.days_per_season) {
            return Err(WorldError::config(
                "days_per_season",
                format!("{} is outside [1, 3650]", self.days_per_season),
            ));
        }
        Ok(())
    }

    /// A copy with every field clamped into its admissible range (non-finite
    /// values are replaced by the default).
    pub fn sanitized(&self) -> WorldConfig {
        let defaults = WorldConfig::default();
        let mut cfg = self.clone();
        let mut default_probe = defaults.clone();
        for (_, field, min, max) in FLOAT_FIELDS {
            let fallback = *field(&mut default_probe);
            let slot = field(&mut cfg);
            *slot = if slot.is_finite() { slot.clamp(min, max) } else { fallback };
        }
        let (fmin, fmax) = FLOOD_THRESHOLD_RANGE;
        cfg.flood_threshold_m = if cfg.flood_threshold_m.is_finite() {
            cfg.flood_threshold_m.clamp(fmin, fmax)
        } else {
            defaults.flood_threshold_m
        };
        cfg.width = cfg.width.clamp(SIZE_RANGE.0, SIZE_RANGE.1);
        cfg.height = cfg.height.clamp(SIZE_RANGE.0, SIZE_RANGE.1);
        cfg.river_count = cfg.river_count.min(MAX_RIVERS);
        cfg.ticks_per_day = cfg.ticks_per_day.clamp(1, 14_400);
        cfg.days_per_season = cfg.days_per_season.clamp(1, 3650);
        cfg
    }

    /// Highest land elevation in metres implied by `relief`.
    pub fn max_land_elevation_m(&self) -> f32 {
        MIN_RELIEF_M + self.relief * (MAX_RELIEF_M - MIN_RELIEF_M)
    }

    /// Area of one cell in square metres.
    pub fn cell_area_m2(&self) -> f64 {
        f64::from(self.cell_size_m) * f64::from(self.cell_size_m)
    }

    /// Unit vector of the direction the prevailing wind blows *towards*
    /// (`x` right/east, `y` down/south).
    pub fn prevailing_wind_dir(&self) -> [f32; 2] {
        let theta = self.wind_from_deg.to_radians();
        // "From" vector points to the source; the wind blows the other way.
        [-theta.sin(), theta.cos()]
    }
}

/// Presets that bias generation towards a dominant landscape. A preset is a
/// bundle of field values (climate and terrain shape); it never adds hidden
/// behaviour, so a preset world can always be reproduced with plain fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiomePreset {
    /// Mild, moderately wet: forests and grasslands.
    Temperate,
    /// Hot with a long dry season and low relief: savanna dominated.
    Savanna,
    /// Hot and very wet: tropical rainforest with many rivers.
    Rainforest,
    /// A single island surrounded by ocean (warm, reefs possible).
    Island,
    /// Low relief, deep basins and no sea: a lake district.
    Lakes,
    /// Mostly ocean with an archipelago.
    Ocean,
    /// Cold and dry: tundra.
    Tundra,
    /// Hot and arid: desert.
    Desert,
    /// Flat and wet: marshes, peat and many lakes.
    Wetland,
    /// High relief: alpine terrain and rock.
    Mountains,
}

impl BiomePreset {
    /// All presets in display order.
    pub const ALL: [BiomePreset; 10] = [
        Self::Temperate,
        Self::Savanna,
        Self::Rainforest,
        Self::Island,
        Self::Lakes,
        Self::Ocean,
        Self::Tundra,
        Self::Desert,
        Self::Wetland,
        Self::Mountains,
    ];

    /// Stable machine key (used by parameter choices and scenario files).
    pub fn key(self) -> &'static str {
        match self {
            Self::Temperate => "temperate",
            Self::Savanna => "savanna",
            Self::Rainforest => "rainforest",
            Self::Island => "island",
            Self::Lakes => "lakes",
            Self::Ocean => "ocean",
            Self::Tundra => "tundra",
            Self::Desert => "desert",
            Self::Wetland => "wetland",
            Self::Mountains => "mountains",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Temperate => "Temperate",
            Self::Savanna => "Savanna",
            Self::Rainforest => "Tropical rainforest",
            Self::Island => "Island",
            Self::Lakes => "Lake district",
            Self::Ocean => "Ocean / archipelago",
            Self::Tundra => "Tundra",
            Self::Desert => "Desert",
            Self::Wetland => "Wetland",
            Self::Mountains => "Mountains",
        }
    }

    /// Looks a preset up by [`BiomePreset::key`].
    pub fn from_key(key: &str) -> Option<BiomePreset> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    /// Writes the preset's values into `cfg`.
    pub fn apply(self, cfg: &mut WorldConfig) {
        // (base °C, latitude gradient, rainfall mm, seasonality, sea level,
        //  relief, roughness, continentality, lake tendency, rivers, island)
        let (t, lat, rain, seas, sea, relief, rough, cont, lakes, rivers, island) = match self {
            Self::Temperate => (12.0, 10.0, 900.0, 10.0, 0.30, 0.45, 0.5, 0.5, 0.35, 8, false),
            Self::Savanna => (25.0, 3.0, 950.0, 4.0, 0.12, 0.22, 0.45, 0.7, 0.2, 6, false),
            Self::Rainforest => (26.0, 2.0, 3200.0, 2.0, 0.18, 0.40, 0.55, 0.5, 0.3, 14, false),
            Self::Island => (23.0, 3.0, 1400.0, 4.0, 0.55, 0.50, 0.55, 0.6, 0.2, 6, true),
            Self::Lakes => (10.0, 6.0, 1000.0, 10.0, 0.0, 0.22, 0.6, 0.4, 1.0, 6, false),
            Self::Ocean => (21.0, 4.0, 1200.0, 5.0, 0.78, 0.35, 0.6, 0.2, 0.1, 2, false),
            Self::Tundra => (-9.0, 6.0, 320.0, 14.0, 0.25, 0.30, 0.45, 0.5, 0.6, 6, false),
            Self::Desert => (25.0, 6.0, 120.0, 10.0, 0.12, 0.35, 0.5, 0.6, 0.05, 2, false),
            Self::Wetland => (14.0, 4.0, 1500.0, 8.0, 0.20, 0.06, 0.3, 0.4, 0.9, 10, false),
            Self::Mountains => (8.0, 8.0, 1100.0, 10.0, 0.08, 1.0, 0.75, 0.5, 0.4, 10, false),
        };
        cfg.base_temperature_c = t;
        cfg.latitude_gradient = lat;
        cfg.rainfall_mm = rain;
        cfg.seasonality = seas;
        cfg.sea_level = sea;
        cfg.relief = relief;
        cfg.roughness = rough;
        cfg.continentality = cont;
        cfg.lake_tendency = lakes;
        cfg.river_count = rivers;
        cfg.island = island;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid_and_presets_are_valid() {
        assert_eq!(WorldConfig::default().validate(), Ok(()));
        for preset in BiomePreset::ALL {
            let cfg = WorldConfig::default().with_preset(preset).effective();
            assert_eq!(cfg.validate(), Ok(()), "{preset:?}");
            assert_eq!(BiomePreset::from_key(preset.key()), Some(preset));
        }
    }

    #[test]
    fn validation_reports_field_and_sanitize_clamps() {
        let cfg = WorldConfig { sea_level: 2.0, rainfall_mm: f32::NAN, width: 3, ..WorldConfig::default() };
        assert!(matches!(cfg.validate(), Err(WorldError::InvalidConfig { field: "width", .. })));
        let fixed = cfg.sanitized();
        assert_eq!(fixed.validate(), Ok(()));
        assert_eq!(fixed.sea_level, 0.95);
        assert_eq!(fixed.rainfall_mm, WorldConfig::default().rainfall_mm);
        assert_eq!(fixed.width, 8);
        let bad_rain = WorldConfig { rainfall_mm: -1.0, ..WorldConfig::default() };
        assert!(matches!(bad_rain.validate(), Err(WorldError::InvalidConfig { field: "rainfall_mm", .. })));
    }

    #[test]
    fn prevailing_wind_points_downwind() {
        let westerly = WorldConfig { wind_from_deg: 270.0, ..WorldConfig::default() }.prevailing_wind_dir();
        assert!((westerly[0] - 1.0).abs() < 1e-6 && westerly[1].abs() < 1e-6);
        let northerly = WorldConfig { wind_from_deg: 0.0, ..WorldConfig::default() }.prevailing_wind_dir();
        assert!(northerly[0].abs() < 1e-6 && (northerly[1] - 1.0).abs() < 1e-6);
    }
}
