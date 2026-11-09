//! Owned, render-ready views of the world: static terrain (re-sent only when
//! its version changes), requested dynamic layers downsampled by an integer
//! block size, and weather / calendar information.

use serde::{Deserialize, Serialize};
use simulation_core::Grid2;
use simulation_core::glam::Vec2;

use crate::biome::{Biome, SoilType, SurfaceKind};
use crate::calendar::CalendarTime;
use crate::fire::FireState;
use crate::weather::{ForcedWeather, WeatherKind};
use crate::world::World;

/// Scalar layers a renderer can request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldLayer {
    /// Surface water depth (m; ocean cells carry their sea depth).
    WaterDepth,
    /// Snow water equivalent (m).
    Snow,
    /// Relative soil saturation.
    SoilMoisture,
    /// Air temperature (°C).
    Temperature,
    /// Relative humidity.
    Humidity,
    /// Ground-level light.
    Light,
    /// Precipitation rate (mm/h).
    Precipitation,
    /// Mineral nutrients (g/m²).
    Nutrients,
    /// Fertility index.
    Fertility,
    /// Fuel load (kg/m²).
    Fuel,
}

impl WorldLayer {
    /// All layers.
    pub const ALL: [WorldLayer; 10] = [
        Self::WaterDepth,
        Self::Snow,
        Self::SoilMoisture,
        Self::Temperature,
        Self::Humidity,
        Self::Light,
        Self::Precipitation,
        Self::Nutrients,
        Self::Fertility,
        Self::Fuel,
    ];
}

/// What a renderer asks for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldViewRequest {
    /// Dynamic scalar layers to include.
    pub layers: Vec<WorldLayer>,
    /// Include the fire-state grid.
    pub fire: bool,
    /// Downsampling block size in cells (1 = full resolution; clamped to
    /// `[1, max(width, height)]`).
    pub block: usize,
    /// Terrain version the renderer already has; static layers are included
    /// only when it differs from the current one.
    pub known_terrain_version: Option<u64>,
}

impl Default for WorldViewRequest {
    fn default() -> Self {
        Self {
            layers: vec![WorldLayer::WaterDepth, WorldLayer::Snow, WorldLayer::SoilMoisture, WorldLayer::Temperature],
            fire: true,
            block: 1,
            known_terrain_version: None,
        }
    }
}

/// Static terrain layers at the view resolution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainView {
    /// Version these layers belong to.
    pub version: u64,
    /// Block-mean elevation (m).
    pub elevation: Grid2<f32>,
    /// Most frequent biome of each block.
    pub biome: Grid2<Biome>,
    /// Most frequent soil type of each block.
    pub soil: Grid2<SoilType>,
    /// Most frequent surface kind of each block.
    pub surface: Grid2<SurfaceKind>,
}

/// Weather summary for displays.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeatherInfo {
    /// Reported weather.
    pub kind: WeatherKind,
    /// Wind velocity (m/s, gust included).
    pub wind: Vec2,
    /// Regional precipitation rate before spatial patterns (mm/h).
    pub precipitation_rate_mm_h: f32,
    /// Sky cover of the state.
    pub cloud_cover: f32,
    /// Smoothed weather temperature offset (°C).
    pub temperature_offset: f32,
    /// Imposed weather, if any.
    pub forced: Option<ForcedWeather>,
}

/// Render data produced by [`World::view`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldView {
    /// Tick of the data.
    pub tick: u64,
    /// Calendar time.
    pub time: CalendarTime,
    /// Daylight factor.
    pub daylight: f32,
    /// Weather summary.
    pub weather: WeatherInfo,
    /// Full-resolution width.
    pub width: usize,
    /// Full-resolution height.
    pub height: usize,
    /// Block size used (view cell = `block × block` world cells).
    pub block: usize,
    /// Current terrain version.
    pub terrain_version: u64,
    /// Static terrain, present when the requested version was stale.
    pub terrain: Option<TerrainView>,
    /// Requested scalar layers (block means), in request order.
    pub layers: Vec<(WorldLayer, Grid2<f32>)>,
    /// Fire state (highest-priority state of each block), when requested.
    pub fire: Option<Grid2<FireState>>,
    /// Drought index.
    pub drought_index: f32,
    /// Flooded cells.
    pub flooded_cells: u32,
    /// Burning cells.
    pub burning_cells: u32,
}

impl WorldView {
    /// Looks up a requested layer.
    pub fn layer(&self, layer: WorldLayer) -> Option<&Grid2<f32>> {
        self.layers.iter().find(|(l, _)| *l == layer).map(|(_, g)| g)
    }
}

/// Downsamples by the mean of each `block × block` tile (edge tiles may be
/// partial).
pub fn downsample_mean(grid: &Grid2<f32>, block: usize) -> Grid2<f32> {
    let block = block.max(1);
    let (w, h) = (grid.width(), grid.height());
    Grid2::from_fn(w.div_ceil(block), h.div_ceil(block), |bx, by| {
        let (mut sum, mut count) = (0.0f64, 0u32);
        for y in by * block..((by + 1) * block).min(h) {
            for &v in &grid.row(y)[bx * block..((bx + 1) * block).min(w)] {
                sum += f64::from(v);
                count += 1;
            }
        }
        (sum / f64::from(count.max(1))) as f32
    })
}

/// Downsamples by the most frequent value of each tile (ties → smallest).
pub fn downsample_mode<T: Copy + Ord>(grid: &Grid2<T>, block: usize) -> Grid2<T> {
    let block = block.max(1);
    let (w, h) = (grid.width(), grid.height());
    let mut values = Vec::with_capacity(block * block);
    Grid2::from_fn(w.div_ceil(block), h.div_ceil(block), |bx, by| {
        values.clear();
        for y in by * block..((by + 1) * block).min(h) {
            values.extend_from_slice(&grid.row(y)[bx * block..((bx + 1) * block).min(w)]);
        }
        values.sort_unstable();
        let mut best = values[0];
        let mut best_run = 0;
        let mut start = 0;
        for i in 1..=values.len() {
            if i == values.len() || values[i] != values[start] {
                if i - start > best_run {
                    best_run = i - start;
                    best = values[start];
                }
                start = i;
            }
        }
        best
    })
}

/// Downsamples fire states keeping the highest display priority per tile.
fn downsample_fire(grid: &Grid2<FireState>, block: usize) -> Grid2<FireState> {
    let block = block.max(1);
    let (w, h) = (grid.width(), grid.height());
    Grid2::from_fn(w.div_ceil(block), h.div_ceil(block), |bx, by| {
        let mut best = FireState::Unburnable;
        for y in by * block..((by + 1) * block).min(h) {
            for &s in &grid.row(y)[bx * block..((bx + 1) * block).min(w)] {
                if s.priority() > best.priority() {
                    best = s;
                }
            }
        }
        best
    })
}

impl World {
    /// Builds an owned view for renderers.
    pub fn view(&self, request: &WorldViewRequest) -> WorldView {
        let block = request.block.clamp(1, self.config.width.max(self.config.height));
        let terrain = (request.known_terrain_version != Some(self.terrain_version)).then(|| TerrainView {
            version: self.terrain_version,
            elevation: downsample_mean(&self.elevation, block),
            biome: downsample_mode(&self.biome, block),
            soil: downsample_mode(&self.soil, block),
            surface: downsample_mode(&self.surface, block),
        });
        let layers = request
            .layers
            .iter()
            .map(|&layer| {
                let grid = match layer {
                    WorldLayer::WaterDepth => &self.water_depth,
                    WorldLayer::Snow => &self.snow,
                    WorldLayer::SoilMoisture => &self.soil_moisture,
                    WorldLayer::Temperature => &self.temperature,
                    WorldLayer::Humidity => &self.humidity,
                    WorldLayer::Light => &self.light,
                    WorldLayer::Precipitation => &self.precipitation,
                    WorldLayer::Nutrients => &self.nutrients,
                    WorldLayer::Fertility => &self.fertility,
                    WorldLayer::Fuel => &self.fuel,
                };
                (layer, downsample_mean(grid, block))
            })
            .collect();
        WorldView {
            tick: self.tick,
            time: self.time(),
            daylight: self.daylight,
            weather: WeatherInfo {
                kind: self.weather.kind(),
                wind: self.weather.wind(),
                precipitation_rate_mm_h: self.weather.precipitation_rate_mm_h(),
                cloud_cover: self.weather.cloud_cover(),
                temperature_offset: self.weather.temperature_offset(),
                forced: self.weather.forced(),
            },
            width: self.config.width,
            height: self.config.height,
            block,
            terrain_version: self.terrain_version,
            terrain,
            layers,
            fire: request.fire.then(|| downsample_fire(&self.fire, block)),
            drought_index: self.drought.index,
            flooded_cells: self.flooded,
            burning_cells: self.burning.len() as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_and_mode_downsampling() {
        let g = Grid2::from_fn(5, 3, |x, y| (x + 10 * y) as f32);
        let d = downsample_mean(&g, 2);
        assert_eq!((d.width(), d.height()), (3, 2));
        assert_eq!(*d.get(0, 0), (0.0 + 1.0 + 10.0 + 11.0) / 4.0);
        assert_eq!(*d.get(2, 1), 24.0, "partial corner tile");
        let m = Grid2::from_vec(2, 2, vec![3u8, 1, 3, 1]);
        assert_eq!(*downsample_mode(&m, 2).get(0, 0), 1, "tie goes to the smallest");
        let m = Grid2::from_vec(2, 2, vec![3u8, 3, 3, 1]);
        assert_eq!(*downsample_mode(&m, 2).get(0, 0), 3);
    }

    #[test]
    fn fire_downsampling_keeps_active_fire() {
        let mut g = Grid2::new(4, 4, FireState::Fuel);
        g.set(3, 3, FireState::Burning { intensity: 1.0, steps: 0 });
        let d = downsample_fire(&g, 4);
        assert!(d.get(0, 0).is_active());
    }
}
