//! Terrain classes, natural resources and world generation.
//!
//! The world is a grid of tiles. Each tile has one [`Terrain`] class that
//! decides whether agents can walk on it, how much walking costs, which
//! natural resource regrows on it and how fertile it is for farming.
//! Terrain is either generated from seeded value noise
//! ([`TerrainSpec::Generated`]) or drawn by hand as ASCII rows
//! ([`TerrainSpec::Map`]).

use serde::{Deserialize, Serialize};
use simulation_core::{Grid2, SimRng};

use crate::error::CivError;
use crate::noise::ValueNoise;

/// Smallest accepted world side, in tiles.
pub const MIN_SIDE: u32 = 8;
/// Largest accepted world side, in tiles.
pub const MAX_SIDE: u32 = 512;

/// Terrain class of a tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terrain {
    /// Open land; farmable (fertility 0.6), buildable.
    #[default]
    Grassland,
    /// Woodland with regrowing timber.
    Forest,
    /// Rocky hills with slowly replenished stone (quarrying); buildable.
    Hills,
    /// Lakes and rivers: impassable, drinkable from the shore, holds fish.
    Water,
    /// Fertile land next to water (fertility 1.0); buildable.
    Riverbank,
    /// Berry bushes with regrowing wild food.
    Berries,
    /// Impassable peaks.
    Mountain,
}

impl Terrain {
    /// All classes in display order.
    pub const ALL: [Terrain; 7] =
        [Self::Grassland, Self::Forest, Self::Hills, Self::Water, Self::Riverbank, Self::Berries, Self::Mountain];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Grassland => "Grassland",
            Self::Forest => "Forest",
            Self::Hills => "Hills / stone",
            Self::Water => "Water / fish",
            Self::Riverbank => "Fertile riverbank",
            Self::Berries => "Berry bushes",
            Self::Mountain => "Mountain",
        }
    }

    /// Whether agents can stand on the tile.
    pub fn passable(self) -> bool {
        !matches!(self, Self::Water | Self::Mountain)
    }

    /// Movement cost multiplier before road discounts (`∞` if impassable).
    pub fn move_cost(self) -> f32 {
        match self {
            Self::Grassland | Self::Riverbank => 1.0,
            Self::Berries => 1.2,
            Self::Forest => 1.6,
            Self::Hills => 2.0,
            Self::Water | Self::Mountain => f32::INFINITY,
        }
    }

    /// Natural resource that regrows on the tile.
    pub fn resource(self) -> Option<Resource> {
        match self {
            Self::Berries => Some(Resource::Berries),
            Self::Water => Some(Resource::Fish),
            Self::Forest => Some(Resource::Wood),
            Self::Hills => Some(Resource::Stone),
            _ => None,
        }
    }

    /// Farming fertility in `[0, 1]` (0 = cannot be farmed).
    pub fn fertility(self) -> f32 {
        match self {
            Self::Riverbank => 1.0,
            Self::Grassland => 0.6,
            _ => 0.0,
        }
    }

    /// Whether buildings may be placed on the tile.
    pub fn buildable(self) -> bool {
        matches!(self, Self::Grassland | Self::Riverbank | Self::Hills)
    }

    /// Map symbol used by [`TerrainSpec::Map`].
    pub fn symbol(self) -> char {
        match self {
            Self::Grassland => '.',
            Self::Forest => 'T',
            Self::Hills => 'h',
            Self::Water => '~',
            Self::Riverbank => 'r',
            Self::Berries => 'b',
            Self::Mountain => '^',
        }
    }

    /// Inverse of [`Terrain::symbol`].
    pub fn from_symbol(symbol: char) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.symbol() == symbol)
    }
}

/// Regenerating natural resources (and crops, which only grow on farm plots).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    /// Wild food on berry tiles.
    Berries,
    /// Food in water tiles, caught from the shore.
    Fish,
    /// Timber in forests.
    Wood,
    /// Stone in hills.
    Stone,
    /// Planted crops on farm plots (growth speed only; no wild stock).
    Crops,
}

impl Resource {
    /// All resources (index order of regeneration multipliers).
    pub const ALL: [Resource; 5] = [Self::Berries, Self::Fish, Self::Wood, Self::Stone, Self::Crops];

    /// Position in [`Resource::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Berries => "Berries",
            Self::Fish => "Fish",
            Self::Wood => "Wood",
            Self::Stone => "Stone",
            Self::Crops => "Crops",
        }
    }

    /// Terrain that carries this resource as a wild stock (none for crops).
    pub fn terrain(self) -> Option<Terrain> {
        match self {
            Self::Berries => Some(Terrain::Berries),
            Self::Fish => Some(Terrain::Water),
            Self::Wood => Some(Terrain::Forest),
            Self::Stone => Some(Terrain::Hills),
            Self::Crops => None,
        }
    }

    /// Stock capacity of one tile before the world richness factor.
    pub fn base_capacity(self) -> f32 {
        match self {
            Self::Berries => 8.0,
            Self::Fish => 6.0,
            Self::Wood => 20.0,
            Self::Stone => 30.0,
            Self::Crops => 0.0,
        }
    }

    /// Growback per tick before richness and regeneration multipliers
    /// (Sugarscape rule G_α: `stock ← min(capacity, stock + α)`).
    pub fn base_regen(self) -> f32 {
        match self {
            Self::Berries => 0.02,
            Self::Fish => 0.012,
            Self::Wood => 0.01,
            Self::Stone => 0.003,
            Self::Crops => 0.0,
        }
    }
}

/// How resource classes are distributed over a generated map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum ResourceZones {
    /// Every class can appear anywhere.
    #[default]
    Mixed,
    /// Building materials (forest, hills) concentrate in the west and food
    /// (berries) in the east; `strength` in `[0, 2]` is the bias added to the
    /// classification noise across the map.
    Split {
        /// Gradient strength.
        strength: f32,
    },
}

/// Parameters of noise-generated terrain. Fractions are shares of all tiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenParams {
    /// Width in tiles.
    pub width: u32,
    /// Height in tiles.
    pub height: u32,
    /// Share of water tiles (lowest elevations).
    pub water: f32,
    /// Share of impassable mountains (highest elevations).
    pub mountains: f32,
    /// Share of hills (high elevations below the mountains).
    pub hills: f32,
    /// Share of forest.
    pub forest: f32,
    /// Share of berry bushes.
    pub berries: f32,
    /// Approximate size of terrain features in tiles.
    #[serde(default = "default_feature_size")]
    pub feature_size: f32,
    /// Spatial arrangement of resources.
    #[serde(default)]
    pub zones: ResourceZones,
}

fn default_feature_size() -> f32 {
    16.0
}

/// Source of a world's terrain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TerrainSpec {
    /// Generated from seeded value noise.
    Generated(GenParams),
    /// Hand-drawn rows of [`Terrain::symbol`] characters (`.` grassland,
    /// `T` forest, `h` hills, `~` water, `r` riverbank, `b` berries,
    /// `^` mountain), top row first.
    Map {
        /// Equal-length rows.
        rows: Vec<String>,
    },
}

impl TerrainSpec {
    /// Checks sizes and fractions without generating anything.
    pub fn validate(&self) -> Result<(), CivError> {
        match self {
            Self::Generated(p) => p.validate(),
            Self::Map { rows } => parse_map(rows).map(|_| ()),
        }
    }
}

impl GenParams {
    /// Checks sizes and fractions.
    pub fn validate(&self) -> Result<(), CivError> {
        let bad = |m: String| Err(CivError::InvalidScenario(m));
        for (name, side) in [("width", self.width), ("height", self.height)] {
            if !(MIN_SIDE..=MAX_SIDE).contains(&side) {
                return bad(format!("world {name} {side} is outside [{MIN_SIDE}, {MAX_SIDE}]"));
            }
        }
        let fractions = [
            ("water", self.water),
            ("mountains", self.mountains),
            ("hills", self.hills),
            ("forest", self.forest),
            ("berries", self.berries),
        ];
        for (name, f) in fractions {
            if !(0.0..=1.0).contains(&f) {
                return bad(format!("terrain fraction `{name}` = {f} must be in [0, 1]"));
            }
        }
        let total: f32 = fractions.iter().map(|(_, f)| f).sum();
        if total > 0.9 {
            return bad(format!("terrain fractions add up to {total:.2}; at most 0.9 is allowed so land remains"));
        }
        if !(4.0..=256.0).contains(&self.feature_size) {
            return bad(format!("feature_size {} must be in [4, 256]", self.feature_size));
        }
        if let ResourceZones::Split { strength } = self.zones
            && !(0.0..=2.0).contains(&strength)
        {
            return bad(format!("zone strength {strength} must be in [0, 2]"));
        }
        Ok(())
    }
}

/// Builds the terrain grid described by `spec`.
pub fn generate(spec: &TerrainSpec, seed: u64) -> Result<Grid2<Terrain>, CivError> {
    match spec {
        TerrainSpec::Generated(params) => {
            params.validate()?;
            Ok(generate_noise(params, seed))
        }
        TerrainSpec::Map { rows } => parse_map(rows),
    }
}

/// Parses ASCII rows into a terrain grid.
pub fn parse_map(rows: &[String]) -> Result<Grid2<Terrain>, CivError> {
    let height = rows.len();
    let width = rows.first().map_or(0, |r| r.chars().count());
    let in_range = |n: usize| (MIN_SIDE as usize..=MAX_SIDE as usize).contains(&n);
    if !in_range(width) || !in_range(height) {
        return Err(CivError::InvalidMap(format!(
            "map is {width}×{height}; both sides must be in [{MIN_SIDE}, {MAX_SIDE}]"
        )));
    }
    let mut cells = Vec::with_capacity(width * height);
    for (y, row) in rows.iter().enumerate() {
        let len = row.chars().count();
        if len != width {
            return Err(CivError::InvalidMap(format!("row {} has {len} columns, expected {width}", y + 1)));
        }
        for (x, symbol) in row.chars().enumerate() {
            let terrain = Terrain::from_symbol(symbol).ok_or_else(|| {
                CivError::InvalidMap(format!(
                    "unknown symbol `{symbol}` at row {}, column {} (use . T h ~ r b ^)",
                    y + 1,
                    x + 1
                ))
            })?;
            cells.push(terrain);
        }
    }
    if !cells.iter().any(|t| t.passable()) {
        return Err(CivError::InvalidMap("the map has no passable tile".into()));
    }
    Ok(Grid2::from_vec(width, height, cells))
}

/// Indices of the `share · n_total` highest-scoring candidates (ties to the
/// lower index), so class fractions are exact quantiles of the noise.
fn top_fraction(candidates: &[usize], score: impl Fn(usize) -> f32, share: f32, n_total: usize) -> Vec<usize> {
    let wanted = ((share * n_total as f32).round() as usize).min(candidates.len());
    let mut ranked: Vec<(f32, usize)> = candidates.iter().map(|&i| (score(i), i)).collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.truncate(wanted);
    ranked.into_iter().map(|(_, i)| i).collect()
}

/// Noise-based generation: elevation quantiles give water (lowest), mountains
/// (highest) and hills; land next to water becomes riverbank; independent
/// noise layers (optionally biased by [`ResourceZones::Split`]) place forest
/// and berries; the rest is grassland.
fn generate_noise(p: &GenParams, seed: u64) -> Grid2<Terrain> {
    let (w, h) = (p.width as usize, p.height as usize);
    let n = w * h;
    let layer = |stream: u64, period: f32| ValueNoise::new(SimRng::derive(seed, stream).next_u64(), period, 4);
    let elevation = layer(1, p.feature_size * 2.0);
    let woods = layer(2, p.feature_size);
    let bushes = layer(3, p.feature_size * 0.75);
    let coord = |i: usize| ((i % w) as f32, (i / w) as f32);
    let elev: Vec<f32> = (0..n).map(|i| elevation.sample(coord(i).0, coord(i).1)).collect();
    let east = |i: usize| coord(i).0 / (w.max(2) - 1) as f32 - 0.5;
    let (material_bias, food_bias) = match p.zones {
        ResourceZones::Mixed => (0.0, 0.0),
        ResourceZones::Split { strength } => (-strength, strength),
    };

    let mut terrain = vec![Terrain::Grassland; n];
    let all: Vec<usize> = (0..n).collect();
    for i in top_fraction(&all, |i| -elev[i], p.water, n) {
        terrain[i] = Terrain::Water;
    }
    let dry: Vec<usize> = all.iter().copied().filter(|&i| terrain[i] == Terrain::Grassland).collect();
    for i in top_fraction(&dry, |i| elev[i], p.mountains, n) {
        terrain[i] = Terrain::Mountain;
    }
    let rest: Vec<usize> = all.iter().copied().filter(|&i| terrain[i] == Terrain::Grassland).collect();
    for i in top_fraction(&rest, |i| elev[i] + 0.5 * material_bias * east(i), p.hills, n) {
        terrain[i] = Terrain::Hills;
    }
    // Riverbanks: grassland touching water in the 8-neighbourhood.
    let near_water = |i: usize| {
        let (x, y) = (i % w, i / w);
        (y.saturating_sub(1)..=(y + 1).min(h - 1))
            .any(|ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).any(|nx| terrain[ny * w + nx] == Terrain::Water))
    };
    let banks: Vec<usize> =
        all.iter().copied().filter(|&i| terrain[i] == Terrain::Grassland && near_water(i)).collect();
    for i in banks {
        terrain[i] = Terrain::Riverbank;
    }
    let open: Vec<usize> = all.iter().copied().filter(|&i| terrain[i] == Terrain::Grassland).collect();
    for i in top_fraction(&open, |i| woods.sample(coord(i).0, coord(i).1) + material_bias * east(i), p.forest, n) {
        terrain[i] = Terrain::Forest;
    }
    let open: Vec<usize> = all.iter().copied().filter(|&i| terrain[i] == Terrain::Grassland).collect();
    for i in top_fraction(&open, |i| bushes.sample(coord(i).0, coord(i).1) + food_bias * east(i), p.berries, n) {
        terrain[i] = Terrain::Berries;
    }
    Grid2::from_vec(w, h, terrain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> GenParams {
        GenParams {
            width: 64,
            height: 48,
            water: 0.15,
            mountains: 0.05,
            hills: 0.1,
            forest: 0.2,
            berries: 0.1,
            feature_size: 12.0,
            zones: ResourceZones::Mixed,
        }
    }

    fn count(g: &Grid2<Terrain>, t: Terrain) -> usize {
        g.as_slice().iter().filter(|&&c| c == t).count()
    }

    #[test]
    fn generated_fractions_match_quantiles() {
        let g = generate(&TerrainSpec::Generated(params()), 5).unwrap();
        let n = 64 * 48;
        assert_eq!(count(&g, Terrain::Water), (0.15 * n as f32).round() as usize);
        assert_eq!(count(&g, Terrain::Mountain), (0.05 * n as f32).round() as usize);
        assert_eq!(count(&g, Terrain::Hills), (0.1 * n as f32).round() as usize);
        assert_eq!(count(&g, Terrain::Forest), (0.2 * n as f32).round() as usize);
        assert_eq!(count(&g, Terrain::Berries), (0.1 * n as f32).round() as usize);
        // Every riverbank touches water.
        for (x, y, t) in g.iter() {
            if *t == Terrain::Riverbank {
                let touches = (-1..=1)
                    .any(|dy| (-1..=1).any(|dx| g.try_get(x as i64 + dx, y as i64 + dy) == Some(&Terrain::Water)));
                assert!(touches, "riverbank at {x},{y} is dry");
            }
        }
        assert_eq!(g, generate(&TerrainSpec::Generated(params()), 5).unwrap(), "same seed, same map");
        assert_ne!(g, generate(&TerrainSpec::Generated(params()), 6).unwrap());
    }

    #[test]
    fn split_zones_separate_food_from_materials() {
        let mut p = params();
        p.zones = ResourceZones::Split { strength: 1.5 };
        let g = generate(&TerrainSpec::Generated(p), 9).unwrap();
        let mean_x = |t: Terrain| {
            let xs: Vec<f32> = g.iter().filter(|(_, _, c)| **c == t).map(|(x, _, _)| x as f32).collect();
            xs.iter().sum::<f32>() / xs.len() as f32
        };
        assert!(mean_x(Terrain::Forest) < 20.0, "forest west: {}", mean_x(Terrain::Forest));
        assert!(mean_x(Terrain::Berries) > 44.0, "berries east: {}", mean_x(Terrain::Berries));
    }

    #[test]
    fn rejects_bad_parameters_and_maps() {
        let mut p = params();
        p.water = 0.8;
        assert!(matches!(generate(&TerrainSpec::Generated(p), 1), Err(CivError::InvalidScenario(_))));
        let mut p = params();
        p.width = 4;
        assert!(generate(&TerrainSpec::Generated(p), 1).is_err());
        let rows: Vec<String> = vec!["........".into(); 7];
        assert!(matches!(parse_map(&rows), Err(CivError::InvalidMap(_))));
        let mut rows: Vec<String> = vec!["........".into(); 8];
        rows[3] = "...x....".into();
        let err = parse_map(&rows).unwrap_err().to_string();
        assert!(err.contains("row 4, column 4"), "{err}");
        rows[3] = "......".into();
        assert!(parse_map(&rows).unwrap_err().to_string().contains("row 4 has 6 columns"));
    }

    #[test]
    fn map_symbols_round_trip() {
        for t in Terrain::ALL {
            assert_eq!(Terrain::from_symbol(t.symbol()), Some(t));
        }
        let rows: Vec<String> = (0..8).map(|_| ".Th~rb^.".to_string()).collect();
        let g = parse_map(&rows).unwrap();
        assert_eq!(*g.get(3, 2), Terrain::Water);
        assert_eq!(*g.get(6, 7), Terrain::Mountain);
    }
}
