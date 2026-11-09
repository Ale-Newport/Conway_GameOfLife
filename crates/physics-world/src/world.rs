//! The [`World`]: static terrain, static climate and every dynamic layer,
//! with constructors, accessors and the hooks the ecosystem writes into.

use serde::{Deserialize, Serialize};
use simulation_core::{Grid2, SimRng};

use crate::biome::{Biome, SoilType, SurfaceKind};
use crate::calendar::{Calendar, CalendarTime, day_length_amplitude};
use crate::climate::StaticClimate;
use crate::config::WorldConfig;
use crate::drainage::ocean_mask;
use crate::error::WorldError;
use crate::fire::FireState;
use crate::hydrology::{DroughtState, GROUNDWATER_TIME_CONSTANT_H, NutrientBudget, WaterBudget};
use crate::landscape::{
    ClassInputs, classify_biomes, classify_soils, fertility, shape_landscape, slope_grid, soil_noise,
};
use crate::report::Pending;
use crate::streams;
use crate::terrain::generate_heightmap;
use crate::weather::Weather;

/// Mineral nutrients (g/m²) of a cell with fertility 1 at generation.
pub const NUTRIENTS_PER_FERTILITY: f32 = 50.0;
/// Organic matter (g/m²) of a cell with fertility 1 at generation.
pub const ORGANIC_PER_FERTILITY: f32 = 150.0;
/// Initial river water depth (m).
const INITIAL_RIVER_DEPTH_M: f32 = 0.3;

/// Independent random streams of the world dynamics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorldRngs {
    pub weather: SimRng,
    pub fire: SimRng,
    pub disasters: SimRng,
}

/// Reusable buffers (not part of the state).
#[derive(Clone, Debug, Default)]
pub(crate) struct Scratch {
    pub flux: Vec<[f64; 9]>,
}

/// The abiotic world: terrain, climate, weather, water, fire and nutrients.
///
/// Layers are row-major [`Grid2`]s of `width × height` cells. Static terrain
/// layers change only through disasters, which bump
/// [`World::terrain_version`]. The ecosystem writes [`World::fuel_mut`]
/// and [`World::canopy_mut`] each tick and exchanges nutrients through
/// [`World::add_nutrients`] / [`World::take_nutrients`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct World {
    pub(crate) config: WorldConfig,
    pub(crate) calendar: Calendar,
    pub(crate) tick: u64,
    pub(crate) terrain_version: u64,
    pub(crate) elevation: Grid2<f32>,
    pub(crate) slope: Grid2<f32>,
    pub(crate) surface: Grid2<SurfaceKind>,
    pub(crate) biome: Grid2<Biome>,
    pub(crate) soil: Grid2<SoilType>,
    pub(crate) receivers: Grid2<u8>,
    pub(crate) accumulation: Grid2<f32>,
    pub(crate) climate: StaticClimate,
    /// Surface water depth (m) in double precision: deep lakes receive
    /// increments far below the f32 resolution of their depth.
    pub(crate) water: Grid2<f64>,
    /// f32 mirror of `water` exposed to the ecosystem and renderers.
    pub(crate) water_depth: Grid2<f32>,
    pub(crate) snow: Grid2<f32>,
    pub(crate) soil_moisture: Grid2<f32>,
    pub(crate) temperature: Grid2<f32>,
    pub(crate) humidity: Grid2<f32>,
    pub(crate) light: Grid2<f32>,
    pub(crate) precipitation: Grid2<f32>,
    pub(crate) fertility: Grid2<f32>,
    pub(crate) nutrients: Grid2<f32>,
    pub(crate) organic: Grid2<f32>,
    pub(crate) fuel: Grid2<f32>,
    pub(crate) canopy: Grid2<f32>,
    pub(crate) fire: Grid2<FireState>,
    pub(crate) burning: Vec<u32>,
    pub(crate) weather: Weather,
    pub(crate) daylight: f32,
    pub(crate) groundwater_m3: f64,
    pub(crate) water_budget: WaterBudget,
    pub(crate) nutrient_budget: NutrientBudget,
    pub(crate) drought: DroughtState,
    pub(crate) flooded: u32,
    pub(crate) burned_total: u64,
    pub(crate) rngs: WorldRngs,
    pub(crate) pending: Pending,
    #[serde(skip)]
    pub(crate) scratch: Scratch,
}

impl World {
    /// Generates a world. Out-of-range configuration values are clamped
    /// ([`WorldConfig::sanitized`]); use [`World::try_generate`] to reject
    /// them instead. The preset in `biome_override`, if any, is applied.
    pub fn generate(config: &WorldConfig) -> World {
        let cfg = config.effective().sanitized();
        let elevation = generate_heightmap(&cfg);
        Self::build(cfg, elevation)
    }

    /// Generates a world after validating the (effective) configuration.
    pub fn try_generate(config: &WorldConfig) -> Result<World, WorldError> {
        let cfg = config.effective();
        cfg.validate()?;
        let elevation = generate_heightmap(&cfg);
        Ok(Self::build(cfg, elevation))
    }

    /// Builds a world from a user-provided heightmap in metres (sea level at
    /// 0 m) instead of noise; every other generation step is the same.
    pub fn from_heightmap(config: &WorldConfig, elevation: Grid2<f32>) -> Result<World, WorldError> {
        let cfg = config.effective();
        cfg.validate()?;
        if elevation.width() != cfg.width || elevation.height() != cfg.height {
            return Err(WorldError::HeightmapSize {
                found_width: elevation.width(),
                found_height: elevation.height(),
                width: cfg.width,
                height: cfg.height,
            });
        }
        if let Some((x, y, z)) = elevation.iter().find(|(_, _, z)| !z.is_finite() || z.abs() > 20_000.0) {
            return Err(WorldError::arg(
                "elevation",
                format!("cell ({x}, {y}) has elevation {z}, expected |z| ≤ 20000 m"),
            ));
        }
        Ok(Self::build(cfg, elevation))
    }

    /// Generation pipeline shared by all constructors (`cfg` is valid).
    fn build(cfg: WorldConfig, elevation: Grid2<f32>) -> World {
        let (w, h) = (cfg.width, cfg.height);
        let n = w * h;
        let land = shape_landscape(&cfg, elevation.into_vec());
        let slope = slope_grid(&land.elevation, w, h, cfg.cell_size_m);
        let surface_height = surface_heights(&land.elevation, &land.surface, &land.lake_depth);
        let climate = StaticClimate::build(&cfg, &surface_height, &land.surface, land.precipitation);
        let inputs = ClassInputs {
            cfg: &cfg,
            elevation: &land.elevation,
            slope: &slope,
            surface: &land.surface,
            mean_temperature: climate.mean_temperature.as_slice(),
            precip_factor: climate.precip_factor.as_slice(),
            accumulation: &land.accumulation,
        };
        let biome = classify_biomes(&inputs);
        let soil = classify_soils(&inputs, &biome, &soil_noise(&cfg));
        let fertility = fertility(&inputs, &soil);

        let water_depth: Vec<f32> = (0..n)
            .map(|i| match land.surface[i] {
                SurfaceKind::Ocean => -land.elevation[i],
                SurfaceKind::Lake => land.lake_depth[i],
                SurfaceKind::River => INITIAL_RIVER_DEPTH_M,
                SurfaceKind::Land => 0.0,
            })
            .collect();
        let snow: Vec<f32> = (0..n)
            .map(|i| {
                let cold = climate.mean_temperature.as_slice()[i] < -3.0;
                if cold && land.surface[i] == SurfaceKind::Land { 0.1 } else { 0.0 }
            })
            .collect();
        let moisture: Vec<f32> = (0..n)
            .map(|i| match land.surface[i] {
                SurfaceKind::Ocean => 0.0,
                SurfaceKind::Lake | SurfaceKind::River => 1.0,
                SurfaceKind::Land => {
                    let p = cfg.rainfall_mm * climate.precip_factor.as_slice()[i];
                    let wet = if biome[i] == Biome::Wetland { 0.3 } else { 0.0 };
                    (0.15 + 0.55 * (p / 1500.0).min(1.0) + wet).clamp(0.05, 1.0)
                }
            })
            .collect();
        let nutrients: Vec<f32> = fertility.iter().map(|f| f * NUTRIENTS_PER_FERTILITY).collect();
        let organic: Vec<f32> = fertility.iter().map(|f| f * ORGANIC_PER_FERTILITY).collect();

        let mut weather_rng = SimRng::derive(cfg.seed, streams::WEATHER);
        let weather = Weather::new(&cfg, &mut weather_rng);
        let calendar = Calendar::from_config(&cfg);
        let non_ocean = land.surface.iter().filter(|s| **s != SurfaceKind::Ocean).count();
        // Steady state of the linear reservoir (time constant τ) when 25 % of
        // the mean rainfall recharges it: G = 0.25 × R × τ per cell.
        let recharge_m_per_h = 0.25 * f64::from(cfg.rainfall_mm) / 1000.0 / (365.0 * 24.0);
        let groundwater = recharge_m_per_h * GROUNDWATER_TIME_CONSTANT_H * non_ocean as f64 * cfg.cell_area_m2();
        let rngs = WorldRngs {
            weather: weather_rng,
            fire: SimRng::derive(cfg.seed, streams::FIRE),
            disasters: SimRng::derive(cfg.seed, streams::DISASTERS),
        };
        let mut world = World {
            calendar,
            tick: 0,
            terrain_version: 1,
            elevation: Grid2::from_vec(w, h, land.elevation),
            slope: Grid2::from_vec(w, h, slope),
            surface: Grid2::from_vec(w, h, land.surface),
            biome: Grid2::from_vec(w, h, biome),
            soil: Grid2::from_vec(w, h, soil),
            receivers: Grid2::from_vec(w, h, land.receivers),
            accumulation: Grid2::from_vec(w, h, land.accumulation),
            climate,
            water: Grid2::from_vec(w, h, water_depth.iter().map(|&d| f64::from(d)).collect()),
            water_depth: Grid2::from_vec(w, h, water_depth),
            snow: Grid2::from_vec(w, h, snow),
            soil_moisture: Grid2::from_vec(w, h, moisture),
            temperature: Grid2::new(w, h, 0.0),
            humidity: Grid2::new(w, h, 0.0),
            light: Grid2::new(w, h, 0.0),
            precipitation: Grid2::new(w, h, 0.0),
            fertility: Grid2::from_vec(w, h, fertility),
            nutrients: Grid2::from_vec(w, h, nutrients),
            organic: Grid2::from_vec(w, h, organic),
            fuel: Grid2::new(w, h, 0.0),
            canopy: Grid2::new(w, h, 0.0),
            fire: Grid2::new(w, h, FireState::Unburnable),
            burning: Vec::new(),
            weather,
            daylight: 0.0,
            groundwater_m3: groundwater,
            water_budget: WaterBudget::default(),
            nutrient_budget: NutrientBudget::default(),
            drought: DroughtState::default(),
            flooded: 0,
            burned_total: 0,
            rngs,
            pending: Pending::default(),
            scratch: Scratch::default(),
            config: cfg,
        };
        world.update_atmosphere();
        world.water_budget.initial_storage_m3 = world.water_storage().total_m3();
        world.nutrient_budget.initial_g = world.nutrient_storage_g();
        world.drought = DroughtState {
            rain_average_mm_h: world.expected_rain_mm_h(),
            reference_moisture: world.mean_land_moisture(),
            index: 0.0,
        };
        world.flooded = world.count_flooded();
        world
    }

    /// Effective configuration (preset applied, runtime changes included).
    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    /// Width in cells.
    pub fn width(&self) -> usize {
        self.config.width
    }

    /// Height in cells.
    pub fn height(&self) -> usize {
        self.config.height
    }

    /// Ticks simulated since generation.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// The calendar.
    pub fn calendar(&self) -> &Calendar {
        &self.calendar
    }

    /// Calendar time of the current tick.
    pub fn time(&self) -> CalendarTime {
        self.calendar.at(self.tick)
    }

    /// Daylight factor of the current tick (sun-elevation proxy, `[0, 1]`).
    pub fn daylight(&self) -> f32 {
        self.daylight
    }

    /// Daylight factor at an arbitrary tick.
    pub fn daylight_at(&self, tick: u64) -> f32 {
        self.calendar.daylight(tick, day_length_amplitude(self.config.seasonality))
    }

    /// Weather state.
    pub fn weather(&self) -> &Weather {
        &self.weather
    }

    /// Version of the static terrain layers; bumped whenever elevation,
    /// surface kinds, biomes or soils change.
    pub fn terrain_version(&self) -> u64 {
        self.terrain_version
    }

    /// Elevation (m, sea level 0; river beds carved, lake bottoms below the water).
    pub fn elevation(&self) -> &Grid2<f32> {
        &self.elevation
    }

    /// Terrain slope (rise/run).
    pub fn slope(&self) -> &Grid2<f32> {
        &self.slope
    }

    /// Surface kind (land, ocean, lake, river).
    pub fn surface(&self) -> &Grid2<SurfaceKind> {
        &self.surface
    }

    /// Biome.
    pub fn biome(&self) -> &Grid2<Biome> {
        &self.biome
    }

    /// Soil type.
    pub fn soil(&self) -> &Grid2<SoilType> {
        &self.soil
    }

    /// D8 receivers from generation (`0..8` = direction E, SE, S, SW, W, NW,
    /// N, NE; [`crate::NO_RECEIVER`] for ocean; [`crate::OFF_MAP`] for
    /// border outlets).
    pub fn flow_receivers(&self) -> &Grid2<u8> {
        &self.receivers
    }

    /// Precipitation-weighted flow accumulation from generation (cells).
    pub fn flow_accumulation(&self) -> &Grid2<f32> {
        &self.accumulation
    }

    /// Static climate (annual means, precipitation pattern).
    pub fn climate(&self) -> &StaticClimate {
        &self.climate
    }

    /// Surface water depth (m). Ocean cells hold their (constant) sea depth.
    pub fn water_depth(&self) -> &Grid2<f32> {
        &self.water_depth
    }

    /// Snow water equivalent (m).
    pub fn snow(&self) -> &Grid2<f32> {
        &self.snow
    }

    /// Relative soil saturation `[0, 1]`.
    pub fn soil_moisture(&self) -> &Grid2<f32> {
        &self.soil_moisture
    }

    /// Air temperature (°C).
    pub fn temperature(&self) -> &Grid2<f32> {
        &self.temperature
    }

    /// Relative humidity `[0, 1]`.
    pub fn humidity(&self) -> &Grid2<f32> {
        &self.humidity
    }

    /// Light at ground level `[0, 1]` (daylight × cloud transmission ×
    /// (1 − canopy)).
    pub fn light(&self) -> &Grid2<f32> {
        &self.light
    }

    /// Precipitation rate of the last tick (mm/h).
    pub fn precipitation(&self) -> &Grid2<f32> {
        &self.precipitation
    }

    /// Fertility index `[0, 1]` (soil × climate × slope).
    pub fn fertility(&self) -> &Grid2<f32> {
        &self.fertility
    }

    /// Mineral nutrients available to plants (g/m²).
    pub fn nutrients(&self) -> &Grid2<f32> {
        &self.nutrients
    }

    /// Soil organic matter (g/m²), slowly mineralised into nutrients.
    pub fn organic_matter(&self) -> &Grid2<f32> {
        &self.organic
    }

    /// Fuel load (kg/m²), written by the ecosystem.
    pub fn fuel(&self) -> &Grid2<f32> {
        &self.fuel
    }

    /// Mutable fuel load (kg/m²). Negative or non-finite values are treated
    /// as zero at the next step.
    pub fn fuel_mut(&mut self) -> &mut Grid2<f32> {
        &mut self.fuel
    }

    /// Canopy cover `[0, 1]`, written by the ecosystem (shades the light layer).
    pub fn canopy(&self) -> &Grid2<f32> {
        &self.canopy
    }

    /// Mutable canopy cover; values are clamped to `[0, 1]` at the next step.
    pub fn canopy_mut(&mut self) -> &mut Grid2<f32> {
        &mut self.canopy
    }

    /// Fire state.
    pub fn fire(&self) -> &Grid2<FireState> {
        &self.fire
    }

    /// Indices (`y × width + x`) of burning or lava cells, ascending.
    pub fn burning_cells(&self) -> &[u32] {
        &self.burning
    }

    /// Whether cell `(x, y)` is burning (or lava). Out-of-range cells are not.
    pub fn is_burning(&self, x: usize, y: usize) -> bool {
        self.fire.try_get(x as i64, y as i64).is_some_and(|s| s.is_active())
    }

    /// Cells currently in the `Burned` state.
    pub fn burned_area_cells(&self) -> usize {
        self.fire.as_slice().iter().filter(|s| matches!(s, FireState::Burned { .. })).count()
    }

    /// Cells that have finished burning since generation.
    pub fn total_burned_cells(&self) -> u64 {
        self.burned_total
    }

    /// Dry-land cells currently flooded.
    pub fn flooded_cells(&self) -> u32 {
        self.flooded
    }

    /// Whether `(x, y)` is dry land under more than the flood threshold.
    pub fn is_flooded(&self, x: usize, y: usize) -> bool {
        self.cell_index(x, y).is_ok_and(|i| {
            self.surface.as_slice()[i] == SurfaceKind::Land
                && self.water_depth.as_slice()[i] > self.config.flood_threshold_m
        })
    }

    /// Drought index `[0, 1]` (0.6 × precipitation deficit + 0.4 ×
    /// soil-moisture deficit).
    pub fn drought_index(&self) -> f32 {
        self.drought.index
    }

    /// Lumped groundwater storage (m³).
    pub fn groundwater_m3(&self) -> f64 {
        self.groundwater_m3
    }

    /// Cumulative water budget.
    pub fn water_budget(&self) -> &WaterBudget {
        &self.water_budget
    }

    /// Cumulative nutrient budget.
    pub fn nutrient_budget(&self) -> &NutrientBudget {
        &self.nutrient_budget
    }

    /// Climatological mean precipitation rate over land (mm/h).
    pub(crate) fn expected_rain_mm_h(&self) -> f64 {
        f64::from(self.config.rainfall_mm) / (365.0 * 24.0)
    }

    /// Mean soil saturation over non-ocean cells.
    pub fn mean_land_moisture(&self) -> f32 {
        let (sum, count) = self
            .surface
            .as_slice()
            .iter()
            .zip(self.soil_moisture.as_slice())
            .filter(|(s, _)| **s == SurfaceKind::Land)
            .fold((0.0f64, 0usize), |(s, c), (_, m)| (s + f64::from(*m), c + 1));
        if count == 0 { 0.0 } else { (sum / count as f64) as f32 }
    }

    /// Mean air temperature over non-ocean cells (°C).
    pub fn mean_land_temperature(&self) -> f32 {
        let (sum, count) = self
            .surface
            .as_slice()
            .iter()
            .zip(self.temperature.as_slice())
            .filter(|(s, _)| **s != SurfaceKind::Ocean)
            .fold((0.0f64, 0usize), |(s, c), (_, t)| (s + f64::from(*t), c + 1));
        if count == 0 {
            (self.temperature.sum() / self.temperature.as_slice().len().max(1) as f64) as f32
        } else {
            (sum / count as f64) as f32
        }
    }

    /// Applies runtime-adjustable configuration fields (climate, wind, fire,
    /// evaporation, flood threshold). Fields that shape the terrain or the
    /// calendar must stay equal; changing them requires a new world.
    /// Biomes keep their generated classification (call
    /// [`World::reclassify_biomes`] to re-derive them from the new climate).
    pub fn set_runtime_config(&mut self, config: &WorldConfig) -> Result<(), WorldError> {
        config.validate()?;
        if let Some(field) = structural_difference(&self.config, config) {
            return Err(WorldError::config(field, "can only be changed by regenerating the world"));
        }
        let rain_changed = config.rainfall_mm != self.config.rainfall_mm;
        self.config = config.clone();
        let surface_height = self.surface_heights();
        let precip = self.climate.precip_factor.as_slice().to_vec();
        self.climate = StaticClimate::build(&self.config, &surface_height, self.surface.as_slice(), precip);
        if rain_changed {
            self.weather.recalibrate(self.config.rainfall_mm);
        }
        Ok(())
    }

    /// Re-derives every biome from the current static climate and terrain
    /// and bumps the terrain version.
    pub fn reclassify_biomes(&mut self) {
        let biome = {
            let inputs = self.class_inputs();
            classify_biomes(&inputs)
        };
        self.biome = Grid2::from_vec(self.config.width, self.config.height, biome);
        self.terrain_version += 1;
    }

    fn class_inputs(&self) -> ClassInputs<'_> {
        ClassInputs {
            cfg: &self.config,
            elevation: self.elevation.as_slice(),
            slope: self.slope.as_slice(),
            surface: self.surface.as_slice(),
            mean_temperature: self.climate.mean_temperature.as_slice(),
            precip_factor: self.climate.precip_factor.as_slice(),
            accumulation: self.accumulation.as_slice(),
        }
    }

    /// Terrain or water-surface height of every cell (m).
    fn surface_heights(&self) -> Vec<f32> {
        let lake_depth: Vec<f32> = self
            .surface
            .as_slice()
            .iter()
            .zip(self.water_depth.as_slice())
            .map(|(s, d)| if *s == SurfaceKind::Lake { *d } else { 0.0 })
            .collect();
        surface_heights(self.elevation.as_slice(), self.surface.as_slice(), &lake_depth)
    }

    /// Re-derives everything that depends on elevation after a disaster
    /// changed it: ocean connectivity (land that sank into the ocean loses its
    /// water to it; sea floor that emerged above 0 m becomes dry land; sea
    /// floor still below 0 m but cut off from the open ocean keeps its sea
    /// water as a lake whose surface is at 0 m, booked as added water),
    /// slope, annual mean temperature and biomes. Bumps the terrain version.
    pub(crate) fn refresh_terrain(&mut self) {
        let (w, h) = (self.config.width, self.config.height);
        let area = self.config.cell_area_m2();
        let ocean = ocean_mask(self.elevation.as_slice(), w, h);
        for (i, &is_ocean) in ocean.iter().enumerate() {
            let kind = self.surface.as_slice()[i];
            let z = f64::from(self.elevation.as_slice()[i]);
            if is_ocean {
                if kind != SurfaceKind::Ocean {
                    let capacity = self.soil.as_slice()[i].properties().capacity_m;
                    let stored = self.water.as_slice()[i]
                        + f64::from(self.snow.as_slice()[i])
                        + f64::from(self.soil_moisture.as_slice()[i] * capacity);
                    self.water_budget.outflow_ocean_m3 += stored * area;
                    self.snow.as_mut_slice()[i] = 0.0;
                    self.soil_moisture.as_mut_slice()[i] = 0.0;
                    self.fire.as_mut_slice()[i] = FireState::Unburnable;
                    self.surface.as_mut_slice()[i] = SurfaceKind::Ocean;
                }
                self.water.as_mut_slice()[i] = -z;
            } else if kind == SurfaceKind::Ocean {
                // Ocean cells hold no snow or soil water, so only the sea
                // water itself has to be decided.
                if z < 0.0 {
                    self.surface.as_mut_slice()[i] = SurfaceKind::Lake;
                    self.water.as_mut_slice()[i] = -z;
                    self.water_budget.added_m3 += -z * area;
                } else {
                    self.surface.as_mut_slice()[i] = SurfaceKind::Land;
                    self.water.as_mut_slice()[i] = 0.0;
                }
            }
        }
        self.sync_water_depth();
        self.burning.retain(|&i| self.surface.as_slice()[i as usize] != SurfaceKind::Ocean);
        self.slope = Grid2::from_vec(w, h, slope_grid(self.elevation.as_slice(), w, h, self.config.cell_size_m));
        let surface_height = self.surface_heights();
        self.climate.rebuild_temperature(&self.config, &surface_height);
        self.reclassify_biomes();
    }

    /// Copies the double-precision water store into the public f32 layer.
    pub(crate) fn sync_water_depth(&mut self) {
        for (mirror, d) in self.water_depth.as_mut_slice().iter_mut().zip(self.water.as_slice()) {
            *mirror = *d as f32;
        }
    }

    /// Approximate memory used by the state (bytes).
    pub fn memory_bytes(&self) -> usize {
        let n = self.config.width * self.config.height;
        n * (22 * std::mem::size_of::<f32>()
            + std::mem::size_of::<FireState>()
            + std::mem::size_of::<SurfaceKind>()
            + std::mem::size_of::<Biome>()
            + std::mem::size_of::<SoilType>()
            + 1)
            + self.scratch.flux.len() * std::mem::size_of::<[f64; 9]>()
            + self.burning.len() * 4
    }
}

/// Terrain height, or water-surface height for lakes (bottom + depth) and
/// 0 m for the ocean.
fn surface_heights(elevation: &[f32], surface: &[SurfaceKind], lake_depth: &[f32]) -> Vec<f32> {
    elevation
        .iter()
        .zip(surface)
        .zip(lake_depth)
        .map(|((z, s), d)| match s {
            SurfaceKind::Ocean => 0.0,
            SurfaceKind::Lake => (z + d).max(0.0),
            _ => z.max(0.0),
        })
        .collect()
}

/// First field that differs between two configurations and cannot change at
/// runtime.
fn structural_difference(a: &WorldConfig, b: &WorldConfig) -> Option<&'static str> {
    let checks: [(&'static str, bool); 13] = [
        ("width", a.width == b.width),
        ("height", a.height == b.height),
        ("seed", a.seed == b.seed),
        ("cell_size_m", a.cell_size_m == b.cell_size_m),
        ("sea_level", a.sea_level == b.sea_level),
        ("relief", a.relief == b.relief),
        ("roughness", a.roughness == b.roughness),
        ("continentality", a.continentality == b.continentality),
        ("island", a.island == b.island),
        ("river_count", a.river_count == b.river_count),
        ("lake_tendency", a.lake_tendency == b.lake_tendency),
        ("ticks_per_day", a.ticks_per_day == b.ticks_per_day),
        ("days_per_season", a.days_per_season == b.days_per_season),
    ];
    checks.into_iter().find(|(_, same)| !same).map(|(name, _)| name)
}
