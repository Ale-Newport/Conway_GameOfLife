//! Surface kinds, biomes (Whittaker-style classification) and soil types
//! with their hydraulic and agronomic properties.

use serde::{Deserialize, Serialize};

/// What occupies the surface of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// Dry land (may hold temporary surface water).
    #[default]
    Land,
    /// Sea connected to the map border: an infinite sink at 0 m.
    Ocean,
    /// Permanent lake in a closed depression.
    Lake,
    /// Carved river channel.
    River,
}

impl SurfaceKind {
    /// Whether the cell is permanent fresh or salt water.
    pub fn is_water(self) -> bool {
        !matches!(self, Self::Land)
    }
}

/// Biome of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Biome {
    /// Open ocean.
    Ocean,
    /// Warm shallow ocean (depth ≤ 30 m, annual mean ≥ 20 °C).
    Reef,
    /// Low, gentle land next to the ocean.
    Beach,
    /// Permanent lake.
    Lake,
    /// River channel.
    River,
    /// Waterlogged flats (marsh, bog, swamp).
    Wetland,
    /// Hot or cold desert (very low precipitation).
    Desert,
    /// Temperate grassland / steppe.
    Grassland,
    /// Tropical grassland with scattered trees.
    Savanna,
    /// Temperate deciduous / mixed forest.
    TemperateForest,
    /// Tropical rainforest.
    TropicalRainforest,
    /// Cold treeless plain (annual mean below −1 °C).
    Tundra,
    /// Alpine terrain and rock (high or very steep).
    Mountain,
}

impl Biome {
    /// All biomes in declaration order.
    pub const ALL: [Biome; 13] = [
        Self::Ocean,
        Self::Reef,
        Self::Beach,
        Self::Lake,
        Self::River,
        Self::Wetland,
        Self::Desert,
        Self::Grassland,
        Self::Savanna,
        Self::TemperateForest,
        Self::TropicalRainforest,
        Self::Tundra,
        Self::Mountain,
    ];

    /// Stable machine key.
    pub fn key(self) -> &'static str {
        match self {
            Self::Ocean => "ocean",
            Self::Reef => "reef",
            Self::Beach => "beach",
            Self::Lake => "lake",
            Self::River => "river",
            Self::Wetland => "wetland",
            Self::Desert => "desert",
            Self::Grassland => "grassland",
            Self::Savanna => "savanna",
            Self::TemperateForest => "temperate_forest",
            Self::TropicalRainforest => "tropical_rainforest",
            Self::Tundra => "tundra",
            Self::Mountain => "mountain",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ocean => "Ocean",
            Self::Reef => "Reef",
            Self::Beach => "Coast / beach",
            Self::Lake => "Lake",
            Self::River => "River",
            Self::Wetland => "Wetland",
            Self::Desert => "Desert",
            Self::Grassland => "Grassland",
            Self::Savanna => "Savanna",
            Self::TemperateForest => "Temperate forest",
            Self::TropicalRainforest => "Tropical rainforest",
            Self::Tundra => "Tundra",
            Self::Mountain => "Mountain",
        }
    }

    /// Whether the biome is a water body.
    pub fn is_water(self) -> bool {
        matches!(self, Self::Ocean | Self::Reef | Self::Lake | Self::River)
    }

    /// Suggested map colour (sRGB).
    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Ocean => [28, 64, 120],
            Self::Reef => [64, 190, 190],
            Self::Beach => [226, 210, 160],
            Self::Lake => [52, 110, 170],
            Self::River => [70, 140, 200],
            Self::Wetland => [90, 120, 80],
            Self::Desert => [220, 190, 120],
            Self::Grassland => [150, 190, 90],
            Self::Savanna => [190, 180, 90],
            Self::TemperateForest => [50, 120, 60],
            Self::TropicalRainforest => [20, 95, 40],
            Self::Tundra => [170, 180, 170],
            Self::Mountain => [130, 120, 110],
        }
    }
}

/// Whittaker-style land biome from annual mean temperature (°C) and annual
/// precipitation (mm):
///
/// | Temperature | Precipitation | Biome |
/// |---|---|---|
/// | T < −1 | any | Tundra |
/// | −1 ≤ T < 20 | P < 250 | Desert |
/// | −1 ≤ T < 20 | 250 ≤ P < 650 | Grassland |
/// | −1 ≤ T < 20 | P ≥ 650 | Temperate forest |
/// | T ≥ 20 | P < 400 | Desert |
/// | T ≥ 20 | 400 ≤ P < 1700 | Savanna |
/// | T ≥ 20 | P ≥ 1700 | Tropical rainforest |
pub fn whittaker(mean_temperature_c: f32, precipitation_mm: f32) -> Biome {
    let (t, p) = (mean_temperature_c, precipitation_mm);
    if t < -1.0 {
        Biome::Tundra
    } else if t < 20.0 {
        if p < 250.0 {
            Biome::Desert
        } else if p < 650.0 {
            Biome::Grassland
        } else {
            Biome::TemperateForest
        }
    } else if p < 400.0 {
        Biome::Desert
    } else if p < 1700.0 {
        Biome::Savanna
    } else {
        Biome::TropicalRainforest
    }
}

/// Soil texture class of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoilType {
    /// Coarse, fast-draining, low fertility.
    Sand,
    /// Balanced texture, fertile.
    Loam,
    /// Fine, slow-draining, holds water.
    Clay,
    /// Bare rock or rubble.
    Rock,
    /// Organic, waterlogged.
    Peat,
    /// Alluvial deposits, very fertile.
    Silt,
}

/// Hydraulic and agronomic properties of a soil type.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoilProperties {
    /// Maximum infiltration rate (saturated conductivity), m/h.
    pub infiltration_m_per_h: f32,
    /// Water held by the root zone at saturation, m.
    pub capacity_m: f32,
    /// Relative saturation above which water percolates to groundwater.
    pub field_capacity: f32,
    /// Fraction of the excess above field capacity percolating per hour.
    pub percolation_per_h: f32,
    /// Fertility of the soil under ideal climate, `[0, 1]`.
    pub base_fertility: f32,
    /// Slope (rise/run) above which wet soil can slide.
    pub critical_slope: f32,
    /// Multiplier on evaporation and transpiration from the soil.
    pub evaporation_factor: f32,
}

impl SoilType {
    /// All soil types in declaration order.
    pub const ALL: [SoilType; 6] = [Self::Sand, Self::Loam, Self::Clay, Self::Rock, Self::Peat, Self::Silt];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sand => "Sand",
            Self::Loam => "Loam",
            Self::Clay => "Clay",
            Self::Rock => "Rock",
            Self::Peat => "Peat",
            Self::Silt => "Silt",
        }
    }

    /// Properties used by hydrology, fertility and landslides. Infiltration
    /// rates follow typical saturated hydraulic conductivities (sand ≈ 30
    /// mm/h, loam ≈ 13, silt ≈ 7, clay ≈ 2 mm/h).
    pub fn properties(self) -> SoilProperties {
        let (infiltration, capacity, field, percolation, fertility, critical, evaporation) = match self {
            Self::Sand => (0.030, 0.06, 0.35, 0.08, 0.30, 0.65, 0.6),
            Self::Loam => (0.013, 0.15, 0.55, 0.03, 0.85, 0.75, 1.0),
            Self::Clay => (0.002, 0.20, 0.70, 0.005, 0.65, 0.50, 0.9),
            Self::Rock => (0.001, 0.01, 0.30, 0.10, 0.05, 1.40, 0.3),
            Self::Peat => (0.006, 0.30, 0.75, 0.004, 0.50, 0.45, 1.1),
            Self::Silt => (0.007, 0.18, 0.60, 0.015, 0.95, 0.55, 1.0),
        };
        SoilProperties {
            infiltration_m_per_h: infiltration,
            capacity_m: capacity,
            field_capacity: field,
            percolation_per_h: percolation,
            base_fertility: fertility,
            critical_slope: critical,
            evaporation_factor: evaporation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whittaker_regions() {
        assert_eq!(whittaker(-5.0, 2000.0), Biome::Tundra);
        assert_eq!(whittaker(10.0, 100.0), Biome::Desert);
        assert_eq!(whittaker(10.0, 400.0), Biome::Grassland);
        assert_eq!(whittaker(10.0, 1200.0), Biome::TemperateForest);
        assert_eq!(whittaker(26.0, 200.0), Biome::Desert);
        assert_eq!(whittaker(26.0, 900.0), Biome::Savanna);
        assert_eq!(whittaker(26.0, 3000.0), Biome::TropicalRainforest);
    }

    #[test]
    fn soil_properties_are_ordered_sensibly() {
        let sand = SoilType::Sand.properties();
        let clay = SoilType::Clay.properties();
        assert!(sand.infiltration_m_per_h > clay.infiltration_m_per_h);
        assert!(sand.capacity_m < clay.capacity_m);
        assert!(SoilType::Silt.properties().base_fertility > SoilType::Rock.properties().base_fertility);
        assert!(Biome::ALL.iter().filter(|b| b.is_water()).count() == 4);
    }
}
