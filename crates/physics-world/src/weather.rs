//! Weather: a seeded hourly Markov chain over weather states, a calibrated
//! rain intensity, wind as an Ornstein–Uhlenbeck random walk around the
//! prevailing wind with gusts, and a wind-advected cloud/rain field.

use noise::NoiseFn;
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent, SimRng};

use crate::config::WorldConfig;
use crate::terrain::{fbm, noise_seed};

/// Weather condition reported to the ecosystem and renderers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherKind {
    /// No precipitation, few clouds.
    Clear,
    /// Overcast, no precipitation.
    Cloudy,
    /// Rain (precipitation multiplier 1).
    Rain,
    /// Thunderstorm: heavy rain (multiplier 3), strong gusty wind, lightning.
    Storm,
    /// Rain or storm while the mean land temperature is below 0 °C. Not a
    /// chain state: it is how precipitating states are reported when cold
    /// (each cell still decides rain vs. snow from its own temperature).
    Snow,
    /// Hot, dry, calm spell (+4.2 °C, rising to +7 °C in mid-summer;
    /// higher evaporation).
    Heatwave,
    /// Persistent dry spell (no rain, +2 °C, higher evaporation).
    Drought,
}

impl WeatherKind {
    /// States of the Markov chain, in matrix order.
    pub const CHAIN: [WeatherKind; 6] =
        [Self::Clear, Self::Cloudy, Self::Rain, Self::Storm, Self::Heatwave, Self::Drought];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear",
            Self::Cloudy => "Cloudy",
            Self::Rain => "Rain",
            Self::Storm => "Storm",
            Self::Snow => "Snow",
            Self::Heatwave => "Heatwave",
            Self::Drought => "Drought",
        }
    }

    /// Precipitation multiplier relative to the calibrated rain rate.
    fn precipitation_multiplier(self) -> f32 {
        match self {
            Self::Rain | Self::Snow => 1.0,
            Self::Storm => 3.0,
            Self::Clear | Self::Cloudy | Self::Heatwave | Self::Drought => 0.0,
        }
    }

    /// Fraction of the sky covered where the local cloud field is dense.
    fn cloud_cover(self) -> f32 {
        match self {
            Self::Clear => 0.1,
            Self::Cloudy => 0.8,
            Self::Rain | Self::Snow | Self::Storm => 1.0,
            Self::Heatwave | Self::Drought => 0.0,
        }
    }

    /// Light transmitted through full local cloud cover.
    fn cloud_transmission(self) -> f32 {
        match self {
            Self::Clear | Self::Heatwave | Self::Drought => 1.0,
            Self::Cloudy => 0.6,
            Self::Rain | Self::Snow => 0.45,
            Self::Storm => 0.25,
        }
    }

    /// Relative-humidity offset of the state.
    fn humidity_bonus(self) -> f32 {
        match self {
            Self::Clear => 0.0,
            Self::Cloudy => 0.1,
            Self::Rain | Self::Snow => 0.3,
            Self::Storm => 0.35,
            Self::Heatwave => -0.2,
            Self::Drought => -0.25,
        }
    }

    /// Target temperature offset (°C) given the seasonal signal.
    fn temperature_offset(self, season: f32) -> f32 {
        match self {
            Self::Clear => 0.0,
            Self::Cloudy => -0.7,
            Self::Rain | Self::Snow => -2.0,
            Self::Storm => -3.0,
            Self::Heatwave => 7.0 * (0.6 + 0.4 * season.max(0.0)),
            Self::Drought => 2.0,
        }
    }

    /// Multiplier on potential evaporation.
    fn evaporation_multiplier(self) -> f32 {
        match self {
            Self::Heatwave => 1.4,
            Self::Drought => 1.5,
            Self::Rain | Self::Snow | Self::Storm => 0.5,
            Self::Clear | Self::Cloudy => 1.0,
        }
    }

    /// Multiplier on the prevailing wind speed.
    fn wind_factor(self) -> f32 {
        match self {
            Self::Clear => 1.0,
            Self::Cloudy => 1.1,
            Self::Rain | Self::Snow => 1.3,
            Self::Storm => 2.5,
            Self::Heatwave => 0.6,
            Self::Drought => 0.9,
        }
    }
}

/// A weather state imposed by an intervention until `until_tick`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForcedWeather {
    /// Imposed state (Storm, Heatwave or Drought).
    pub kind: WeatherKind,
    /// First tick at which the state is no longer imposed.
    pub until_tick: u64,
    /// Precipitation intensity multiplier (storms).
    pub intensity: f32,
}

/// Hourly transition weights (unnormalised) from `state`, given the seasonal
/// signal `season ∈ [−1, 1]` and the wetness `w = clamp(rainfall / 900 mm,
/// 0.05, 4)`. Columns follow [`WeatherKind::CHAIN`].
pub fn transition_weights(state: WeatherKind, season: f32, wetness: f32) -> [f64; 6] {
    let s = f64::from(season.clamp(-1.0, 1.0));
    let w = f64::from(wetness);
    let summer = s.max(0.0);
    match state {
        WeatherKind::Clear => [0.92, 0.06 * (0.5 + 0.5 * w), 0.0, 0.0, 0.003 * summer + 0.0002, 0.0004 * (1.0 + s) / w],
        WeatherKind::Cloudy => [0.06, 0.86, 0.06 * w, 0.008 * w * (1.0 + s), 0.0, 0.0],
        WeatherKind::Rain | WeatherKind::Snow => [0.02, 0.08, 0.89, 0.012 * (1.0 + s), 0.0, 0.0],
        WeatherKind::Storm => [0.0, 0.05, 0.25, 0.70, 0.0, 0.0],
        WeatherKind::Heatwave => [0.01, 0.0, 0.0, 0.003, 0.987, 0.0],
        WeatherKind::Drought => [0.004, 0.0, 0.0, 0.0, 0.0, 0.996],
    }
}

/// Wetness factor of a rainfall total.
fn wetness(rainfall_mm: f32) -> f32 {
    (rainfall_mm / 900.0).clamp(0.05, 4.0)
}

/// Stationary distribution of the chain at a fixed seasonal signal (power
/// iteration of the row-normalised transition matrix).
fn stationary(season: f32, wet: f32) -> [f64; 6] {
    let rows: Vec<[f64; 6]> = WeatherKind::CHAIN
        .iter()
        .map(|&s| {
            let w = transition_weights(s, season, wet);
            let total: f64 = w.iter().sum();
            w.map(|v| v / total)
        })
        .collect();
    let mut pi = [1.0 / 6.0; 6];
    for _ in 0..5000 {
        let mut next = [0.0; 6];
        for (i, row) in rows.iter().enumerate() {
            for (j, p) in row.iter().enumerate() {
                next[j] += pi[i] * p;
            }
        }
        pi = next;
    }
    pi
}

/// Rain-state precipitation rate (mm/h) such that the long-run mean over the
/// year equals `rainfall_mm / 365` per day: the expected precipitation
/// multiplier is averaged over eight seasonal phases of the stationary
/// distribution.
pub fn calibrated_rain_rate(rainfall_mm: f32) -> f32 {
    if rainfall_mm <= 0.0 {
        return 0.0;
    }
    let wet = wetness(rainfall_mm);
    let expected: f64 = (0..8)
        .map(|k| {
            let season = (std::f64::consts::TAU * (f64::from(k) / 8.0 - 0.125)).sin() as f32;
            let pi = stationary(season, wet);
            WeatherKind::CHAIN.iter().zip(pi).map(|(s, p)| p * f64::from(s.precipitation_multiplier())).sum::<f64>()
        })
        .sum::<f64>()
        / 8.0;
    (f64::from(rainfall_mm) / (365.0 * 24.0) / expected.max(1e-9)) as f32
}

/// Inputs of one weather update.
pub(crate) struct WeatherStep<'a> {
    pub cfg: &'a WorldConfig,
    pub tick: u64,
    pub hours_started: u64,
    pub dt_hours: f32,
    pub season: f32,
}

/// Weather state (part of the world state and its snapshots).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Weather {
    state: WeatherKind,
    hours_in_state: u32,
    mean_wind: Vec2,
    gust: f32,
    cloud_offset: [f64; 2],
    temp_offset: f32,
    forced: Option<ForcedWeather>,
    rain_rate_mm_h: f32,
    cloud_seed: u32,
    land_mean_temperature: f32,
}

impl Weather {
    pub(crate) fn new(cfg: &WorldConfig, rng: &mut SimRng) -> Self {
        let [dx, dy] = cfg.prevailing_wind_dir();
        Self {
            state: WeatherKind::Clear,
            hours_in_state: 0,
            mean_wind: Vec2::new(dx, dy) * cfg.wind_speed,
            gust: 1.0,
            cloud_offset: [rng.range_f64(-1e4, 1e4), rng.range_f64(-1e4, 1e4)],
            temp_offset: 0.0,
            forced: None,
            rain_rate_mm_h: calibrated_rain_rate(cfg.rainfall_mm),
            cloud_seed: noise_seed(rng),
            land_mean_temperature: cfg.base_temperature_c,
        }
    }

    /// Reported weather kind ([`WeatherKind::Snow`] when precipitating in
    /// the cold).
    pub fn kind(&self) -> WeatherKind {
        if matches!(self.state, WeatherKind::Rain | WeatherKind::Storm) && self.land_mean_temperature < 0.0 {
            WeatherKind::Snow
        } else {
            self.state
        }
    }

    /// Underlying Markov-chain state.
    pub fn chain_state(&self) -> WeatherKind {
        self.state
    }

    /// Hours spent in the current chain state.
    pub fn hours_in_state(&self) -> u32 {
        self.hours_in_state
    }

    /// Instantaneous wind velocity (m/s, `x` east, `y` south), gusts included.
    pub fn wind(&self) -> Vec2 {
        self.mean_wind * self.gust
    }

    /// Instantaneous wind speed (m/s).
    pub fn wind_speed(&self) -> f32 {
        self.wind().length()
    }

    /// Wind velocity without the current gust.
    pub fn mean_wind(&self) -> Vec2 {
        self.mean_wind
    }

    /// Current gust multiplier (≥ 1).
    pub fn gust(&self) -> f32 {
        self.gust
    }

    /// Imposed weather, if any.
    pub fn forced(&self) -> Option<ForcedWeather> {
        self.forced
    }

    /// Calibrated precipitation rate of the Rain state (mm/h).
    pub fn base_rain_rate_mm_h(&self) -> f32 {
        self.rain_rate_mm_h
    }

    /// Regional precipitation rate (mm/h) before the spatial pattern and the
    /// local cloud field are applied.
    pub fn precipitation_rate_mm_h(&self) -> f32 {
        let intensity = self.forced.map_or(1.0, |f| f.intensity);
        self.rain_rate_mm_h * self.state.precipitation_multiplier() * intensity
    }

    /// Sky cover of the state where the cloud field is dense.
    pub fn cloud_cover(&self) -> f32 {
        self.state.cloud_cover()
    }

    /// Light transmitted through full local cloud cover.
    pub fn cloud_transmission(&self) -> f32 {
        self.state.cloud_transmission()
    }

    /// Relative humidity offset of the state.
    pub fn humidity_bonus(&self) -> f32 {
        self.state.humidity_bonus()
    }

    /// Smoothed temperature offset (°C) applied to every cell.
    pub fn temperature_offset(&self) -> f32 {
        self.temp_offset
    }

    /// Multiplier on potential evaporation.
    pub fn evaporation_multiplier(&self) -> f32 {
        self.state.evaporation_multiplier()
    }

    /// Whether lightning can strike (thunderstorm).
    pub fn has_lightning(&self) -> bool {
        self.state == WeatherKind::Storm
    }

    /// Seed of the cloud noise.
    pub(crate) fn cloud_seed(&self) -> u32 {
        self.cloud_seed
    }

    /// Accumulated wind advection of the cloud field (cells).
    pub(crate) fn cloud_offset(&self) -> [f64; 2] {
        self.cloud_offset
    }

    pub(crate) fn set_land_mean_temperature(&mut self, t: f32) {
        self.land_mean_temperature = t;
    }

    /// Recomputes the rain-rate calibration after a rainfall change.
    pub(crate) fn recalibrate(&mut self, rainfall_mm: f32) {
        self.rain_rate_mm_h = calibrated_rain_rate(rainfall_mm);
    }

    /// Imposes a state until `until_tick` (interventions).
    pub(crate) fn force(&mut self, kind: WeatherKind, until_tick: u64, intensity: f32) {
        self.forced = Some(ForcedWeather { kind, until_tick, intensity });
        if self.state != kind {
            self.state = kind;
            self.hours_in_state = 0;
        }
    }

    /// Advances the weather by one tick: forced-state bookkeeping, one
    /// Markov transition per started hour, wind random walk and gusts, the
    /// smoothed temperature offset and cloud advection. Returns an event when
    /// the reported state changes.
    pub(crate) fn advance(&mut self, step: &WeatherStep<'_>, rng: &mut SimRng) -> Option<SimEvent> {
        let before = self.kind();
        if let Some(forced) = self.forced {
            if step.tick >= forced.until_tick {
                self.forced = None;
                self.state = if forced.kind == WeatherKind::Storm { WeatherKind::Rain } else { WeatherKind::Clear };
                self.hours_in_state = 0;
            } else {
                self.state = forced.kind;
            }
        }
        let wet = wetness(step.cfg.rainfall_mm);
        for _ in 0..step.hours_started {
            if self.forced.is_none() {
                let weights = transition_weights(self.state, step.season, wet);
                let next = rng.weighted_index(&weights).map_or(self.state, |i| WeatherKind::CHAIN[i]);
                if next != self.state {
                    self.state = next;
                    self.hours_in_state = 0;
                    continue;
                }
            }
            self.hours_in_state = self.hours_in_state.saturating_add(1);
        }

        // Ornstein–Uhlenbeck wind: relax towards the state's target with a
        // 12 h time scale, plus Gaussian increments.
        let dt = step.dt_hours;
        let [dx, dy] = step.cfg.prevailing_wind_dir();
        let target = Vec2::new(dx, dy) * step.cfg.wind_speed * self.state.wind_factor();
        let relax = 1.0 - (-dt / 12.0).exp();
        let sigma = (0.5 + 0.1 * step.cfg.wind_speed) * dt.sqrt();
        let noise = Vec2::new(rng.normal() as f32, rng.normal() as f32) * sigma;
        self.mean_wind += (target - self.mean_wind) * relax + noise;
        let gustiness = if self.state == WeatherKind::Storm { 0.5 } else { 0.2 };
        self.gust = 1.0 + (rng.normal() as f32 * gustiness).abs();

        let target_offset = self.state.temperature_offset(step.season);
        self.temp_offset += (target_offset - self.temp_offset) * (1.0 - (-dt / 3.0).exp());

        let seconds = f64::from(dt) * 3600.0;
        let cell = f64::from(step.cfg.cell_size_m);
        self.cloud_offset[0] += f64::from(self.mean_wind.x) * seconds / cell;
        self.cloud_offset[1] += f64::from(self.mean_wind.y) * seconds / cell;

        let after = self.kind();
        (after != before).then(|| {
            let severity = match after {
                WeatherKind::Storm | WeatherKind::Heatwave | WeatherKind::Drought => EventSeverity::Info,
                _ => EventSeverity::Trace,
            };
            SimEvent::new(step.tick, "weather", format!("Weather changed to {}", after.label())).severity(severity)
        })
    }
}

/// Coarse, wind-advected cloud field: a multiplier `m ≥ 0` of the regional
/// precipitation rate, sampled bilinearly per cell and normalised to a mean
/// of exactly 1 over the map's cells.
pub(crate) struct CloudField {
    block: usize,
    columns: usize,
    rows: usize,
    data: Vec<f32>,
}

/// Coarse-grid spacing of the cloud field, in cells.
const CLOUD_BLOCK: usize = 8;

impl CloudField {
    /// Evaluates 3-D fBm `n` (x, y advected by the wind; time evolving
    /// slowly) on a grid every [`CLOUD_BLOCK`] cells and takes
    /// `max(0, 1 + c n)` (contrast `c` = 4, storms 6). Rain cells have a
    /// characteristic size of about 6 km.
    ///
    /// The clipping at 0 would raise the mean above 1 (and with it the
    /// long-run rainfall), so the field is divided by its exact cell mean
    /// (bilinear sampling is linear in the node values): the cloud field
    /// redistributes the regional precipitation over the map without
    /// changing its map mean. A field that is zero everywhere stays zero.
    pub(crate) fn compute(weather: &Weather, cfg: &WorldConfig, hours: f64) -> Self {
        let columns = cfg.width.div_ceil(CLOUD_BLOCK) + 1;
        let rows = cfg.height.div_ceil(CLOUD_BLOCK) + 1;
        let feature = (6000.0 / f64::from(cfg.cell_size_m)).clamp(8.0, 4.0 * cfg.width.max(cfg.height) as f64);
        let noise = fbm(weather.cloud_seed(), 3, 1.0, 0.5);
        let contrast = if weather.chain_state() == WeatherKind::Storm { 6.0 } else { 4.0 };
        let [ox, oy] = weather.cloud_offset();
        let mut data: Vec<f64> = (0..rows * columns)
            .map(|k| {
                let cx = ((k % columns) * CLOUD_BLOCK) as f64;
                let cy = ((k / columns) * CLOUD_BLOCK) as f64;
                // The field moves with the wind: sample upwind of the cell.
                let n = noise.get([(cx - ox) / feature, (cy - oy) / feature, hours * 0.03]);
                (1.0 + contrast * n).max(0.0)
            })
            .collect();
        let wx = node_weights(cfg.width, columns);
        let wy = node_weights(cfg.height, rows);
        let weighted: f64 = data.iter().enumerate().map(|(k, v)| v * wx[k % columns] * wy[k / columns]).sum();
        let mean = weighted / (cfg.width * cfg.height) as f64;
        if mean > 0.0 {
            for v in &mut data {
                *v /= mean;
            }
        }
        Self { block: CLOUD_BLOCK, columns, rows, data: data.into_iter().map(|v| v as f32).collect() }
    }

    /// Bilinear sample at cell `(x, y)`.
    pub(crate) fn sample(&self, x: usize, y: usize) -> f32 {
        let (x0, x1, tx) = interpolation(x, self.block, self.columns);
        let (y0, y1, ty) = interpolation(y, self.block, self.rows);
        let at = |cx: usize, cy: usize| self.data[cy * self.columns + cx];
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

/// Nodes and weight `t` of the linear interpolation at cell coordinate
/// `c` on a node grid of spacing `block` with `nodes` nodes: the value is
/// `(1 − t) v[n0] + t v[n1]`.
fn interpolation(c: usize, block: usize, nodes: usize) -> (usize, usize, f32) {
    let f = c as f32 / block as f32;
    let n0 = (f.floor() as usize).min(nodes - 1);
    let n1 = (n0 + 1).min(nodes - 1);
    (n0, n1, f - n0 as f32)
}

/// Total interpolation weight of every node over the `cells` cells of one
/// axis, so that the cell mean of the bilinear field is
/// `Σ v[a, b] wx[a] wy[b] / (W H)`.
fn node_weights(cells: usize, nodes: usize) -> Vec<f64> {
    let mut weights = vec![0.0f64; nodes];
    for c in 0..cells {
        let (n0, n1, t) = interpolation(c, CLOUD_BLOCK, nodes);
        weights[n0] += f64::from(1.0 - t);
        weights[n1] += f64::from(t);
    }
    weights
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_scales_with_rainfall() {
        let dry = calibrated_rain_rate(300.0);
        let wet = calibrated_rain_rate(2000.0);
        assert!(dry > 0.0 && wet > 0.0);
        assert_eq!(calibrated_rain_rate(0.0), 0.0);
        // Rain falls more often in wet climates, so the per-event rate grows
        // less than proportionally.
        assert!(wet / dry < 2000.0 / 300.0);
    }

    #[test]
    fn summer_favours_storms_and_heatwaves() {
        let summer = transition_weights(WeatherKind::Cloudy, 1.0, 1.0);
        let winter = transition_weights(WeatherKind::Cloudy, -1.0, 1.0);
        assert!(summer[3] > winter[3]);
        let summer = transition_weights(WeatherKind::Clear, 1.0, 1.0);
        let winter = transition_weights(WeatherKind::Clear, -1.0, 1.0);
        assert!(summer[4] > winter[4]);
    }

    #[test]
    fn forced_state_holds_then_releases() {
        let cfg = WorldConfig::default();
        let mut rng = SimRng::seed_from_u64(3);
        let mut weather = Weather::new(&cfg, &mut rng);
        weather.force(WeatherKind::Drought, 50, 1.0);
        for tick in 0..50 {
            let step = WeatherStep { cfg: &cfg, tick, hours_started: 1, dt_hours: 1.0, season: 0.0 };
            weather.advance(&step, &mut rng);
            assert_eq!(weather.kind(), WeatherKind::Drought);
            assert_eq!(weather.precipitation_rate_mm_h(), 0.0);
        }
        let step = WeatherStep { cfg: &cfg, tick: 50, hours_started: 1, dt_hours: 1.0, season: 0.0 };
        weather.advance(&step, &mut rng);
        assert!(weather.forced().is_none());
        assert_ne!(weather.chain_state(), WeatherKind::Drought);
    }

    #[test]
    fn cloud_field_has_unit_cell_mean() {
        // Map sizes that are not multiples of the node spacing exercise the
        // partial edge weights.
        let cfg = WorldConfig { width: 100, height: 60, cell_size_m: 400.0, ..WorldConfig::default() };
        let mut rng = SimRng::seed_from_u64(8);
        let mut weather = Weather::new(&cfg, &mut rng);
        for (kind, hours) in [(WeatherKind::Rain, 3.0), (WeatherKind::Storm, 40.0)] {
            weather.force(kind, 10, 1.0);
            let field = CloudField::compute(&weather, &cfg, hours);
            let samples: Vec<f64> = (0..60)
                .flat_map(|y| (0..100).map(move |x| (x, y)))
                .map(|(x, y)| f64::from(field.sample(x, y)))
                .collect();
            let mean = samples.iter().sum::<f64>() / samples.len() as f64;
            assert!((mean - 1.0).abs() < 1e-4, "{kind:?}: cell mean {mean}");
            assert!(samples.iter().all(|&m| m >= 0.0));
            let spread =
                samples.iter().copied().fold(0.0f64, f64::max) - samples.iter().copied().fold(f64::MAX, f64::min);
            assert!(spread > 0.1, "the field must vary in space on a 40 km map");
        }
    }

    #[test]
    fn snow_is_reported_when_cold() {
        let cfg = WorldConfig::default();
        let mut rng = SimRng::seed_from_u64(1);
        let mut weather = Weather::new(&cfg, &mut rng);
        weather.force(WeatherKind::Storm, 10, 1.0);
        weather.set_land_mean_temperature(-4.0);
        assert_eq!(weather.kind(), WeatherKind::Snow);
        weather.set_land_mean_temperature(4.0);
        assert_eq!(weather.kind(), WeatherKind::Storm);
    }
}
