//! [`EnvironmentSim`]: the world alone as a [`Simulation`] (model id
//! `environment.world`), useful to study climate, hydrology and fire without
//! organisms. A simple built-in vegetation proxy regrows fuel towards a
//! per-biome capacity so that fires have something to burn.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, EventLog, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamSpec, ParamValue,
    SimError, Simulation, Snapshot,
};

use crate::biome::{Biome, SurfaceKind};
use crate::config::{BiomePreset, WorldConfig};
use crate::disasters::Disaster;
use crate::error::WorldError;
use crate::fire::FireState;
use crate::report::EnvironmentReport;
use crate::view::{WorldView, WorldViewRequest};
use crate::world::World;

/// Model identifier.
pub const MODEL_ID: &str = "environment.world";
/// Snapshot schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Fuel capacity (kg/m²) the standalone vegetation proxy regrows towards.
pub fn biome_fuel_capacity(biome: Biome) -> f32 {
    match biome {
        Biome::TropicalRainforest => 4.0,
        Biome::TemperateForest => 3.0,
        Biome::Savanna => 1.2,
        Biome::Wetland => 1.0,
        Biome::Grassland => 0.8,
        Biome::Tundra => 0.3,
        Biome::Mountain => 0.2,
        Biome::Beach => 0.1,
        Biome::Desert => 0.04,
        Biome::Ocean | Biome::Reef | Biome::Lake | Biome::River => 0.0,
    }
}

/// Standalone vegetation proxy: `fuel += (capacity − fuel)(1 − e^{−Δt/τ})`
/// on land cells that are not burning; burned cells regrow from what is left.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VegetationProxy {
    /// Whether the proxy writes fuel (disable when an ecosystem owns fuel).
    pub enabled: bool,
    /// Regrowth time constant τ in days.
    pub regrowth_days: f32,
}

impl Default for VegetationProxy {
    fn default() -> Self {
        Self { enabled: true, regrowth_days: 60.0 }
    }
}

const PRESET_FIELDS: [&str; 11] = [
    "base_temperature_c",
    "latitude_gradient",
    "rainfall_mm",
    "seasonality",
    "sea_level",
    "relief",
    "roughness",
    "continentality",
    "lake_tendency",
    "river_count",
    "island",
];

const RULES: &str = r#"### Environment world (abiotic)

One tick = `1440 / ticks_per_day` simulated minutes (default 6 min); a year has 4 seasons of `days_per_season` days.

**Generation.** Elevation = fBm(Perlin, 7 octaves) with domain warping, blended with a 3-octave continent fBm
(weight `0.15 + 0.6 × continentality`), minus Perlin lake basins (`0.1 × lake_tendency`), optionally faded to
ocean by a radial island mask. Sea level is the `sea_level` quantile; land maps to `0.5 m + (peak − 0.5) t^1.6`
(peak = `150 + 3850 × relief` m), sea to `−(0.5 + 1499.5 t^1.2)` m. Ocean = below 0 m and 8-connected to the
border. Priority-Flood (+5 mm ε) fills depressions; basins of ≥ 2 cells deeper than `0.5 + 10 (1 − t)²` m
(t = `lake_tendency`) stay lakes, deepest first, until lakes cover `0.25 t²` of the land; the rest are filled
(sediment). D8 steepest descent on the filled surface gives receivers;
accumulation is weighted by relative precipitation. `river_count` sources (weight `(z/zmax)² P √A`, spaced) are
traced downstream and carved 0.6–4 m deep, strictly descending (≥ 1 cm per cell).

**Climate.** `T = T_base + G (y/(H−1) − ½) − Γ z/1000 + c (A_s S + A_d D (1 − 0.6 k)) + ΔT_weather` with
`S = sin 2π(f − 1/8)` (year fraction f), `D = cos 2π(h − 15)/24`, continental factor `c ∈ [0.3, 1]`, local
cloudiness `k`. Precipitation pattern: moisture fBm × distance-to-water decay × windward lift × rain shadow
`exp(−Δz_barrier/1200 m)` × altitude, normalised to mean 1 over land. Biomes: Whittaker table on annual mean T and
P plus ocean/reef (≤ 30 m, ≥ 20 °C), lake, river, beach, mountain (> 55 % of peak and > 600 m, or slope ≥ 1) and
wetland (flat, wet, high wetness index).

**Weather.** Hourly Markov chain over Clear, Cloudy, Rain, Storm, Heatwave, Drought (weights depend on season and
wetness); Snow is reported when it precipitates and the mean land temperature is below 0 °C. The Rain-state rate
is calibrated from the stationary distribution so that the long-run mean is `rainfall_mm / 365` per day (Storm ×3).
Wind: Ornstein–Uhlenbeck walk (12 h) around the prevailing wind × state factor, plus half-normal gusts. A 3-D fBm
cloud field `max(0, 1 + c n)` advected by the wind, divided by its map mean (so it redistributes rain without changing
the regional total), modulates local precipitation and light.

**Hydrology (per non-ocean cell).** Precipitation (snow if T < 0 °C) → degree-day melt (4 mm/°C/day) → open-water
evaporation `PET = s · 3.3e-5 m/h/°C · max(0, T+5)(1 − RH)(0.3 + 0.7 daylight)(1 + 0.08 |wind|) · m_weather · Δt`;
then on land only: infiltration (soil rate, 10 % when frozen, limited by pore space) → soil evaporation
`∝ saturation` → percolation of the excess over field capacity into a lumped groundwater store → mineralisation
and leaching. Lake and river beds are gaining reaches (no infiltration). Groundwater (40-day linear reservoir)
releases baseflow `G (1 − e^{−Δt/40 d})` evenly into river cells. Surface routing (one sub-step per started 6 min,
at most 16 per tick):
each cell releases `0.8 min(d, ½ Δmax)` split ∝ `Δ_k / L_k` among neighbours with lower water surface; the ocean
(surface 0 m) and the map edge (free outfall) are sinks. Flooded = dry land deeper than `flood_threshold_m`.

**Fire (Alexandridis et al. 2008).** Per sub-step (one per started 6 min, ≤ 16 per tick) each burning cell ignites
each fuel neighbour with
`p = 0.58 (1 + p_veg)(1 + p_den) e^{0.045 V} e^{0.131 V (cos θ − 1)} e^{0.078 θ_s} (1 − m/0.9) f_I · fire_spread`.
Burning consumes 35 % of the fuel per sub-step; it goes out below 0.05 kg/m², on standing water or snow, after 40
sub-steps, or by rain with probability `1 − e^{−0.35 r}` (r mm/h). Burned cells carry fire again after one day
with ≥ 0.3 kg/m² of fuel. Storms strike `Poisson(lightning_rate × map km² × Δt)` uniformly random cells (strikes
on the ocean have no effect); a strike on fuel ignites with probability `0.35 (1 − m)`.

**Landslides (hourly).** Soil-covered land steeper than its soil's critical slope and above 95 % saturation fails
with probability `0.005 (s − 0.95)/0.05`; half of the excess height (≤ 5 m) moves to the steepest downhill
neighbour with a proportional share of nutrients.

**Interventions** (commands). Drought / heatwave / storm impose a weather state for a duration; extra rain and flood
domes add booked water; earthquakes offset terrain across a straight fault by `±A (1 − (d/r)²)` with
`log₁₀ A = −4.80 + 0.69 M` m (Wells & Coppersmith 1994) and trigger rockfall and slope failures; tornadoes report
wind damage and cut canopy; volcanoes raise a cone, turn its core to lava for two days and drop ash (organic matter).
After any terrain change the ocean mask is recomputed: land that sinks into the ocean hands it its water, sea floor
cut off from the ocean by uplift keeps its water as a lake, and slope, temperature and biomes are re-derived.

**Standalone vegetation proxy.** Fuel regrows towards a biome capacity with time constant `fuel_regrowth_days`.
"#;

/// A configuration value as a float parameter: the shortest decimal that
/// round-trips to the stored `f32` (e.g. `0.35`, not `0.3499999940395355`),
/// so reported values equal what was set and stay inside the spec ranges.
fn decimal(value: f32) -> ParamValue {
    ParamValue::Float(value.to_string().parse::<f64>().unwrap_or(f64::from(value)))
}

/// Checks a base configuration of the model: the effective configuration
/// (preset applied) is valid and the seed fits the `seed` parameter.
fn check_config(config: &WorldConfig) -> Result<(), WorldError> {
    if i64::try_from(config.seed).is_err() {
        return Err(WorldError::config(
            "seed",
            format!("{} exceeds the largest seed parameter {}", config.seed, i64::MAX),
        ));
    }
    config.effective().validate()
}

/// The abiotic world as a stand-alone simulation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvironmentSim {
    config: WorldConfig,
    world: World,
    vegetation: VegetationProxy,
    last: EnvironmentReport,
    events: EventLog,
}

impl EnvironmentSim {
    /// Generates a world from a validated configuration. The seed must fit
    /// the `seed` parameter (`0..=2⁶³ − 1`) so that [`Simulation::params`]
    /// reports it exactly.
    pub fn new(config: WorldConfig) -> Result<Self, WorldError> {
        check_config(&config)?;
        let world = World::try_generate(&config)?;
        let mut sim = Self {
            config,
            world,
            vegetation: VegetationProxy::default(),
            last: EnvironmentReport::default(),
            events: EventLog::new(2000),
        };
        sim.seed_vegetation();
        Ok(sim)
    }

    /// The world.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Mutable world (direct interventions, fuel edits).
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// The configuration used by [`Simulation::reset`].
    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    /// Vegetation proxy settings.
    pub fn vegetation(&self) -> VegetationProxy {
        self.vegetation
    }

    /// Report of the last step.
    pub fn last_report(&self) -> &EnvironmentReport {
        &self.last
    }

    /// Event history.
    pub fn events(&self) -> &EventLog {
        &self.events
    }

    /// Initial fuel: 60 % of the biome capacity (when the proxy is enabled).
    fn seed_vegetation(&mut self) {
        if !self.vegetation.enabled {
            return;
        }
        let world = &mut self.world;
        for (fuel, b) in world.fuel.as_mut_slice().iter_mut().zip(world.biome.as_slice()) {
            *fuel = 0.6 * biome_fuel_capacity(*b);
        }
    }

    /// One regrowth step of the vegetation proxy.
    fn regrow_fuel(&mut self) {
        if !self.vegetation.enabled || self.vegetation.regrowth_days <= 0.0 {
            return;
        }
        let dt_days = 1.0 / self.world.calendar().ticks_per_day() as f32;
        let rate = 1.0 - (-dt_days / self.vegetation.regrowth_days).exp();
        let world = &mut self.world;
        let biome = world.biome.as_slice();
        let surface = world.surface.as_slice();
        let fire = world.fire.as_slice();
        world.fuel.as_mut_slice().par_iter_mut().enumerate().for_each(|(i, fuel)| {
            if surface[i] == SurfaceKind::Land && !fire[i].is_active() {
                *fuel += (biome_fuel_capacity(biome[i]) - *fuel) * rate;
            }
        });
    }

    fn param_specs() -> Vec<ParamSpec> {
        let reset = ApplyPolicy::RequiresReset;
        let mut presets: Vec<(&str, &str)> = vec![("none", "None (custom)")];
        presets.extend(BiomePreset::ALL.iter().map(|p| (p.key(), p.label())));
        vec![
            ParamSpec::int("width", "Width", 16, 2048, 256).group("World").apply(reset).describe("Cells per row."),
            ParamSpec::int("height", "Height", 16, 2048, 256).group("World").apply(reset).describe("Rows."),
            ParamSpec::int("seed", "Seed", 0, i64::MAX, 1).group("World").apply(reset).describe("Master random seed."),
            ParamSpec::choice("preset", "Biome preset", &presets, "none")
                .group("World")
                .apply(reset)
                .describe("Overrides climate and terrain fields to favour a landscape."),
            ParamSpec::float("cell_size_m", "Cell size (m)", 1.0, 10_000.0, 50.0).group("World").apply(reset),
            ParamSpec::float("sea_level", "Sea fraction", 0.0, 0.95, 0.35)
                .group("Terrain")
                .apply(reset)
                .describe("Fraction of the map below sea level."),
            ParamSpec::float("relief", "Relief", 0.0, 1.0, 0.45).group("Terrain").apply(reset),
            ParamSpec::float("roughness", "Roughness", 0.0, 1.0, 0.5).group("Terrain").apply(reset),
            ParamSpec::float("continentality", "Continentality", 0.0, 1.0, 0.5).group("Terrain").apply(reset),
            ParamSpec::boolean("island", "Island", false).group("Terrain").apply(reset),
            ParamSpec::int("river_count", "Rivers", 0, 200, 8).group("Terrain").apply(reset),
            ParamSpec::float("lake_tendency", "Lake tendency", 0.0, 1.0, 0.35).group("Terrain").apply(reset),
            ParamSpec::int("ticks_per_day", "Ticks per day", 1, 14_400, 240).group("Calendar").apply(reset),
            ParamSpec::int("days_per_season", "Days per season", 1, 3650, 30).group("Calendar").apply(reset),
            ParamSpec::float("base_temperature_c", "Base temperature (°C)", -40.0, 45.0, 14.0).group("Climate"),
            ParamSpec::float("latitude_gradient", "Latitude gradient (°C)", -60.0, 60.0, 12.0).group("Climate"),
            ParamSpec::float("rainfall_mm", "Annual rainfall (mm)", 0.0, 8000.0, 900.0).group("Climate"),
            ParamSpec::float("seasonality", "Seasonal amplitude (°C)", 0.0, 40.0, 10.0).group("Climate"),
            ParamSpec::float("diurnal_range_c", "Day–night range (°C)", 0.0, 40.0, 10.0).group("Climate"),
            ParamSpec::float("lapse_rate_c_per_km", "Lapse rate (°C/km)", 0.0, 15.0, 6.5).group("Climate"),
            ParamSpec::float("wind_speed", "Prevailing wind (m/s)", 0.0, 60.0, 4.0).group("Climate"),
            ParamSpec::float("wind_from_deg", "Wind from (°)", 0.0, 360.0, 270.0).group("Climate"),
            ParamSpec::float("fire_spread", "Fire spread factor", 0.0, 5.0, 1.0).group("Dynamics"),
            ParamSpec::float("lightning_rate", "Lightning (strikes/km²/h)", 0.0, 10.0, 0.02).group("Dynamics"),
            ParamSpec::float("evaporation_scale", "Evaporation scale", 0.0, 10.0, 1.0).group("Dynamics"),
            ParamSpec::float("flood_threshold_m", "Flood threshold (m)", 0.01, 10.0, 0.5).group("Dynamics"),
            ParamSpec::boolean("vegetation_proxy", "Vegetation proxy", true)
                .group("Dynamics")
                .describe("Regrow fuel towards a per-biome capacity (stand-alone runs only)."),
            ParamSpec::float("fuel_regrowth_days", "Fuel regrowth (days)", 1.0, 3650.0, 60.0).group("Dynamics"),
        ]
    }

    fn metric_specs() -> Vec<MetricSpec> {
        vec![
            MetricSpec::gauge(
                "mean_temperature",
                "Mean temperature",
                "°C",
                "Mean air temperature over non-ocean cells.",
            ),
            MetricSpec::gauge(
                "rainfall",
                "Rainfall",
                "mm/tick",
                "Mean precipitation depth over non-ocean cells this tick.",
            ),
            MetricSpec::gauge("water_volume", "Surface water", "m³", "Water on land, lakes and rivers."),
            MetricSpec::gauge("groundwater", "Groundwater", "m³", "Lumped groundwater storage."),
            MetricSpec::gauge("snow_volume", "Snow", "m³", "Snow water equivalent."),
            MetricSpec::ratio("soil_moisture", "Soil moisture", "Mean relative saturation of dry land."),
            MetricSpec::gauge("burning_cells", "Burning cells", "cells", "Cells burning or covered by lava."),
            MetricSpec::ratio("burned_area", "Burned area", "Fraction of non-ocean cells in the burned state."),
            MetricSpec::ratio("drought_index", "Drought index", "0.6 × rain deficit + 0.4 × soil-moisture deficit."),
            MetricSpec::gauge(
                "flooded_cells",
                "Flooded cells",
                "cells",
                "Dry land under more than the flood threshold.",
            ),
            MetricSpec::gauge("wind_speed", "Wind speed", "m/s", "Instantaneous wind speed including gusts."),
            MetricSpec::ratio("daylight", "Daylight", "Sun-elevation proxy."),
        ]
    }

    /// Applies a parameter value to the base configuration.
    fn set_config_field(cfg: &mut WorldConfig, key: &str, value: &ParamValue) -> Result<(), SimError> {
        let float =
            || value.as_f64().map(|v| v as f32).ok_or_else(|| SimError::invalid_param(key, "expected a number"));
        let int = || value.as_i64().ok_or_else(|| SimError::invalid_param(key, "expected an integer"));
        let size = |v: i64| usize::try_from(v).map_err(|_| SimError::invalid_param(key, "must not be negative"));
        let count = |v: i64| u32::try_from(v).map_err(|_| SimError::invalid_param(key, "out of range"));
        match key {
            "width" => cfg.width = size(int()?)?,
            "height" => cfg.height = size(int()?)?,
            "seed" => {
                cfg.seed = u64::try_from(int()?).map_err(|_| SimError::invalid_param(key, "must not be negative"))?
            }
            "cell_size_m" => cfg.cell_size_m = float()?,
            "sea_level" => cfg.sea_level = float()?,
            "relief" => cfg.relief = float()?,
            "roughness" => cfg.roughness = float()?,
            "continentality" => cfg.continentality = float()?,
            "island" => {
                cfg.island = value.as_bool().ok_or_else(|| SimError::invalid_param(key, "expected true/false"))?;
            }
            "river_count" => cfg.river_count = count(int()?)?,
            "lake_tendency" => cfg.lake_tendency = float()?,
            "ticks_per_day" => cfg.ticks_per_day = count(int()?)?,
            "days_per_season" => cfg.days_per_season = count(int()?)?,
            "base_temperature_c" => cfg.base_temperature_c = float()?,
            "latitude_gradient" => cfg.latitude_gradient = float()?,
            "rainfall_mm" => cfg.rainfall_mm = float()?,
            "seasonality" => cfg.seasonality = float()?,
            "diurnal_range_c" => cfg.diurnal_range_c = float()?,
            "lapse_rate_c_per_km" => cfg.lapse_rate_c_per_km = float()?,
            "wind_speed" => cfg.wind_speed = float()?,
            "wind_from_deg" => cfg.wind_from_deg = float()?,
            "fire_spread" => cfg.fire_spread = float()?,
            "lightning_rate" => cfg.lightning_rate = float()?,
            "evaporation_scale" => cfg.evaporation_scale = float()?,
            "flood_threshold_m" => cfg.flood_threshold_m = float()?,
            _ => return Err(SimError::UnknownParameter(key.to_string())),
        }
        Ok(())
    }
}

impl Simulation for EnvironmentSim {
    type Command = Disaster;
    type ViewRequest = WorldViewRequest;
    type View = WorldView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(MODEL_ID, "Environment (abiotic world)", ModelFamily::Continuous)
            .summary(
                "Procedural terrain with rivers and lakes, biomes, seasons and day/night, weather and wind, \
                 hydrology with floods and droughts, probabilistic fire spread and disasters — without organisms.",
            )
            .rules(RULES)
            .params(Self::param_specs())
            .metrics(Self::metric_specs())
            .time("minute", self.world.calendar().minutes_per_tick())
    }

    fn tick(&self) -> u64 {
        self.world.tick()
    }

    fn step(&mut self) {
        self.regrow_fuel();
        let report = self.world.step();
        for event in &report.events {
            self.events.push(event.clone());
        }
        self.last = report;
    }

    fn reset(&mut self) {
        self.world = World::generate(&self.config);
        self.last = EnvironmentReport::default();
        self.events.clear();
        self.seed_vegetation();
    }

    fn apply(&mut self, command: Disaster) -> Result<(), SimError> {
        self.world.trigger(command).map(|_| ()).map_err(SimError::from)
    }

    fn params(&self) -> ParamSet {
        let c = self.config.effective();
        let preset = self.config.biome_override.map_or("none", BiomePreset::key);
        ParamSet::new()
            .with("width", ParamValue::Int(c.width as i64))
            .with("height", ParamValue::Int(c.height as i64))
            .with("seed", ParamValue::Int(i64::try_from(c.seed).unwrap_or(i64::MAX)))
            .with("preset", ParamValue::Text(preset.to_string()))
            .with("cell_size_m", decimal(c.cell_size_m))
            .with("sea_level", decimal(c.sea_level))
            .with("relief", decimal(c.relief))
            .with("roughness", decimal(c.roughness))
            .with("continentality", decimal(c.continentality))
            .with("island", ParamValue::Bool(c.island))
            .with("river_count", ParamValue::Int(i64::from(c.river_count)))
            .with("lake_tendency", decimal(c.lake_tendency))
            .with("ticks_per_day", ParamValue::Int(i64::from(c.ticks_per_day)))
            .with("days_per_season", ParamValue::Int(i64::from(c.days_per_season)))
            .with("base_temperature_c", decimal(c.base_temperature_c))
            .with("latitude_gradient", decimal(c.latitude_gradient))
            .with("rainfall_mm", decimal(c.rainfall_mm))
            .with("seasonality", decimal(c.seasonality))
            .with("diurnal_range_c", decimal(c.diurnal_range_c))
            .with("lapse_rate_c_per_km", decimal(c.lapse_rate_c_per_km))
            .with("wind_speed", decimal(c.wind_speed))
            .with("wind_from_deg", decimal(c.wind_from_deg))
            .with("fire_spread", decimal(c.fire_spread))
            .with("lightning_rate", decimal(c.lightning_rate))
            .with("evaporation_scale", decimal(c.evaporation_scale))
            .with("flood_threshold_m", decimal(c.flood_threshold_m))
            .with("vegetation_proxy", ParamValue::Bool(self.vegetation.enabled))
            .with("fuel_regrowth_days", decimal(self.vegetation.regrowth_days))
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let specs = Self::param_specs();
        let spec = specs.iter().find(|s| s.key == key).ok_or_else(|| SimError::UnknownParameter(key.to_string()))?;
        let value = spec.validate(&value)?;
        match key {
            "vegetation_proxy" => {
                self.vegetation.enabled = value.as_bool().unwrap_or(self.vegetation.enabled);
                return Ok(spec.apply);
            }
            "fuel_regrowth_days" => {
                self.vegetation.regrowth_days = value.as_f64().map_or(self.vegetation.regrowth_days, |v| v as f32);
                return Ok(spec.apply);
            }
            "preset" => {
                let name = value.as_str().unwrap_or("none");
                self.config.biome_override = BiomePreset::from_key(name);
                return Ok(spec.apply);
            }
            _ => {}
        }
        // Editing a field a preset controls turns the preset into explicit
        // values first, so the edit is not overwritten at the next reset.
        let mut next = self.config.clone();
        if next.biome_override.is_some() && PRESET_FIELDS.contains(&key) {
            next = next.effective();
            next.biome_override = None;
        }
        Self::set_config_field(&mut next, key, &value)?;
        next.effective().validate().map_err(SimError::from)?;
        if spec.apply != ApplyPolicy::RequiresReset {
            let mut runtime = self.world.config().clone();
            Self::set_config_field(&mut runtime, key, &value)?;
            self.world.set_runtime_config(&runtime).map_err(SimError::from)?;
        }
        self.config = next;
        Ok(spec.apply)
    }

    fn metrics(&self) -> Metrics {
        let world = &self.world;
        let storage = world.water_storage();
        let non_ocean = world.surface().as_slice().iter().filter(|s| **s != SurfaceKind::Ocean).count().max(1);
        let burned = world.fire().as_slice().iter().filter(|s| matches!(s, FireState::Burned { .. })).count();
        let minutes = world.calendar().minutes_per_tick();
        Metrics::new(world.tick(), world.tick() as f64 * minutes)
            .with("mean_temperature", f64::from(world.mean_land_temperature()))
            .with("rainfall", self.last.rain_total_mm)
            .with("water_volume", storage.surface_m3)
            .with("groundwater", storage.groundwater_m3)
            .with("snow_volume", storage.snow_m3)
            .with("soil_moisture", f64::from(world.mean_land_moisture()))
            .with("burning_cells", world.burning_cells().len() as f64)
            .with("burned_area", burned as f64 / non_ocean as f64)
            .with("drought_index", f64::from(world.drought_index()))
            .with("flooded_cells", f64::from(world.flooded_cells()))
            .with("wind_speed", f64::from(world.weather().wind_speed()))
            .with("daylight", f64::from(world.daylight()))
    }

    fn view(&self, request: &WorldViewRequest) -> WorldView {
        self.world.view(request)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(MODEL_ID, SCHEMA_VERSION, self.world.tick(), self)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let restored: EnvironmentSim = snapshot.decode(MODEL_ID, SCHEMA_VERSION)?;
        check_config(&restored.config).map_err(SimError::from)?;
        restored.world.check_layout().map_err(|e| SimError::Serialization(format!("inconsistent snapshot: {e}")))?;
        *self = restored;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.world.memory_bytes()
    }
}

#[cfg(test)]
mod tests {
    use simulation_core::Grid2;

    use super::*;

    #[test]
    fn restore_rejects_structurally_inconsistent_snapshots() {
        let mut sim = EnvironmentSim::new(WorldConfig::with_size(16, 16)).expect("valid config");
        let good = sim.snapshot().expect("encodes");

        let mut wrong_layer = sim.clone();
        wrong_layer.world.water = Grid2::new(3, 3, 0.0);
        let err = sim.restore(&wrong_layer.snapshot().expect("encodes")).unwrap_err();
        assert!(matches!(&err, SimError::Serialization(m) if m.contains("`water`")), "{err}");

        let mut wrong_fire_list = sim.clone();
        wrong_fire_list.world.burning = vec![5, 400];
        let err = sim.restore(&wrong_fire_list.snapshot().expect("encodes")).unwrap_err();
        assert!(matches!(&err, SimError::Serialization(m) if m.contains("burning")), "{err}");

        let mut wrong_calendar = sim.clone();
        wrong_calendar.world.config.ticks_per_day = 24;
        assert!(sim.restore(&wrong_calendar.snapshot().expect("encodes")).is_err());

        // Failed restores leave the simulation untouched; a valid one works.
        assert_eq!(sim.snapshot().expect("encodes"), good);
        sim.restore(&good).expect("a consistent snapshot restores");
    }
}
