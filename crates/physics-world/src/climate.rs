//! Static climate fields (annual means) and the per-cell atmospheric
//! formulas evaluated every tick.

use noise::NoiseFn;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simulation_core::{Grid2, SimRng};

use crate::biome::SurfaceKind;
use crate::cells::distance_to;
use crate::config::WorldConfig;
use crate::streams;
use crate::terrain::{fbm, noise_seed};

/// Scale height (m) of the rain shadow behind a barrier.
const RAIN_SHADOW_SCALE_M: f32 = 1200.0;

/// Annual-mean climate of every cell. Computed at generation and rebuilt
/// (temperature part) when climate parameters or terrain change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaticClimate {
    /// Annual mean temperature, °C.
    pub mean_temperature: Grid2<f32>,
    /// Relative precipitation (mean 1 over non-ocean cells); annual
    /// precipitation is `rainfall_mm × precip_factor`.
    pub precip_factor: Grid2<f32>,
    /// Damping of seasonal and diurnal amplitudes by nearby water,
    /// `[0.3, 1]` (1 = fully continental).
    pub continental: Grid2<f32>,
    /// Base relative humidity, `[0.1, 0.95]`.
    pub humidity_base: Grid2<f32>,
}

/// Relative precipitation pattern, normalised to a mean of 1 over
/// non-ocean cells:
///
/// `P = (1 + 0.35 n) × (0.55 + 0.45 e^{−d/(0.6 L)}) × (1 + 3 min(s_w, 0.5))
///      × e^{−max(0, z_b − z)/1200 m} × (1 + 0.4 min(z / 2000 m, 1))`
///
/// with `n` a moisture fBm, `d` the distance to the nearest open water and
/// `L` the map size (both in cells), `s_w` the terrain rise along the
/// prevailing wind over two cells (windward orographic lift), and `z_b` the
/// highest terrain within `min(48, L/3)` cells upwind (rain shadow).
pub(crate) fn precipitation_pattern(cfg: &WorldConfig, elevation: &[f32], ocean: &[bool], lake: &[bool]) -> Vec<f32> {
    let (w, h) = (cfg.width, cfg.height);
    let mut rng = SimRng::derive(cfg.seed, streams::CLIMATE);
    let moisture = fbm(noise_seed(&mut rng), 4, 2.0, 0.5);
    let (ox, oy) = (rng.range_f64(-500.0, 500.0), rng.range_f64(-500.0, 500.0));
    let water_dist = distance_to(w, h, |i| ocean[i] || lake[i]);
    let size = w.max(h) as f32;
    let reach = ((size / 3.0) as usize).clamp(1, 48);
    let [dx, dy] = cfg.prevailing_wind_dir();
    let cell = cfg.cell_size_m;
    let height_at = |x: f32, y: f32| -> Option<f32> {
        let (xi, yi) = (x.round(), y.round());
        (xi >= 0.0 && yi >= 0.0 && (xi as usize) < w && (yi as usize) < h)
            .then(|| elevation[yi as usize * w + xi as usize].max(0.0))
    };
    let scale = size as f64;
    let mut pattern = vec![0.0f32; w * h];
    pattern.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let z = elevation[i].max(0.0);
            let n = moisture.get([x as f64 / scale + ox, y as f64 / scale + oy]) as f32;
            let d = water_dist[i];
            let coast = if d == u32::MAX { 0.55 } else { 0.55 + 0.45 * (-(d as f32) / (0.6 * size)).exp() };
            let (fx, fy) = (x as f32, y as f32);
            let upwind2 = height_at(fx - 2.0 * dx, fy - 2.0 * dy).unwrap_or(z);
            let rise = ((z - upwind2) / (2.0 * cell)).max(0.0);
            let lift = 1.0 + 3.0 * rise.min(0.5);
            let mut barrier = 0.0f32;
            for s in 1..=reach {
                match height_at(fx - dx * s as f32, fy - dy * s as f32) {
                    Some(b) => barrier = barrier.max(b),
                    None => break,
                }
            }
            let shadow = (-(barrier - z).max(0.0) / RAIN_SHADOW_SCALE_M).exp();
            let altitude = 1.0 + 0.4 * (z / 2000.0).min(1.0);
            *out = ((1.0 + 0.35 * n) * coast * lift * shadow * altitude).max(0.02);
        }
    });
    let (sum, count) = pattern
        .iter()
        .zip(ocean)
        .filter(|(_, o)| !**o)
        .fold((0.0f64, 0usize), |(s, c), (p, _)| (s + f64::from(*p), c + 1));
    let mean = if count > 0 {
        sum / count as f64
    } else {
        pattern.iter().map(|&p| f64::from(p)).sum::<f64>() / pattern.len() as f64
    };
    if mean > 0.0 {
        for p in &mut pattern {
            *p = (f64::from(*p) / mean) as f32;
        }
    }
    pattern
}

/// Annual mean temperature (°C) of a cell at row `y` whose surface (land or
/// water surface) is `surface_m` metres above sea level:
/// `T = T_base + G (y/(H−1) − ½) − Γ max(z, 0) / 1000`.
pub fn annual_mean_temperature(cfg: &WorldConfig, y: usize, surface_m: f32) -> f32 {
    let latitude = if cfg.height > 1 { y as f32 / (cfg.height - 1) as f32 - 0.5 } else { 0.0 };
    cfg.base_temperature_c + cfg.latitude_gradient * latitude - cfg.lapse_rate_c_per_km * surface_m.max(0.0) / 1000.0
}

impl StaticClimate {
    /// Builds every static field. `surface_height` is the terrain or water
    /// surface elevation (m) of each cell.
    pub(crate) fn build(
        cfg: &WorldConfig,
        surface_height: &[f32],
        surface: &[SurfaceKind],
        precip_factor: Vec<f32>,
    ) -> Self {
        let (w, h) = (cfg.width, cfg.height);
        let ocean_dist = distance_to(w, h, |i| surface[i] == SurfaceKind::Ocean);
        let water_dist = distance_to(w, h, |i| surface[i].is_water());
        let size = w.max(h) as f32;
        let continental = (0..w * h)
            .map(|i| match surface[i] {
                SurfaceKind::Ocean => 0.3,
                kind => {
                    let d = ocean_dist[i];
                    let c = if d == u32::MAX { 1.0 } else { 0.35 + 0.65 * (1.0 - (-(d as f32) / (0.25 * size)).exp()) };
                    if kind == SurfaceKind::Lake { 0.7 * c } else { c }
                }
            })
            .collect();
        let humidity_base = (0..w * h)
            .map(|i| {
                if surface[i] == SurfaceKind::Ocean {
                    return 0.85;
                }
                let p = cfg.rainfall_mm * precip_factor[i];
                let near = if water_dist[i] == u32::MAX { 0.0 } else { (-(water_dist[i] as f32) / 8.0).exp() };
                (0.35 + 0.35 * (p / 2500.0).min(1.0) + 0.2 * near).clamp(0.1, 0.95)
            })
            .collect();
        let mut climate = Self {
            mean_temperature: Grid2::new(w, h, 0.0),
            precip_factor: Grid2::from_vec(w, h, precip_factor),
            continental: Grid2::from_vec(w, h, continental),
            humidity_base: Grid2::from_vec(w, h, humidity_base),
        };
        climate.rebuild_temperature(cfg, surface_height);
        climate
    }

    /// Recomputes the annual mean temperature (after a change of base
    /// temperature, latitude gradient, lapse rate or terrain).
    pub(crate) fn rebuild_temperature(&mut self, cfg: &WorldConfig, surface_height: &[f32]) {
        let w = cfg.width;
        self.mean_temperature.as_mut_slice().par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, t) in row.iter_mut().enumerate() {
                *t = annual_mean_temperature(cfg, y, surface_height[y * w + x]);
            }
        });
    }
}

/// Region-wide atmospheric drivers of one tick.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Drivers {
    /// Seasonal signal `S ∈ [−1, 1]`.
    pub season: f32,
    /// Diurnal signal `D ∈ [−1, 1]`.
    pub diurnal: f32,
    /// Seasonal amplitude (°C).
    pub seasonality: f32,
    /// Half of the diurnal range (°C).
    pub diurnal_half_range: f32,
    /// Smoothed weather temperature offset (°C).
    pub weather_offset: f32,
    /// Relative humidity offset of the weather state.
    pub humidity_bonus: f32,
    /// Daylight factor (sun-elevation proxy).
    pub daylight: f32,
    /// Light transmitted through full local cloud cover.
    pub transmission: f32,
    /// Sky cover of the state where clouds are dense.
    pub cover: f32,
}

/// Instantaneous temperature:
/// `T = T_mean + c (A_s S + A_d D (1 − 0.6 k)) + ΔT_weather`, where `c` is
/// the continental factor and `k` the local cloudiness (clouds damp the
/// diurnal cycle).
#[inline]
pub(crate) fn cell_temperature(mean: f32, continental: f32, cloudiness: f32, d: &Drivers) -> f32 {
    mean + continental * (d.seasonality * d.season + d.diurnal_half_range * d.diurnal * (1.0 - 0.6 * cloudiness))
        + d.weather_offset
}

/// Relative humidity in `[0.05, 1]`: base + weather bonus (scaled by local
/// cloudiness for positive bonuses) + 0.1 at night − 0.1 at the afternoon
/// peak + 0.15 over standing water − 0.01 per °C above 25 °C.
#[inline]
pub(crate) fn cell_humidity(base: f32, cloudiness: f32, wet_surface: f32, temperature: f32, d: &Drivers) -> f32 {
    let bonus = if d.humidity_bonus > 0.0 { d.humidity_bonus * (0.4 + 0.6 * cloudiness) } else { d.humidity_bonus };
    (base + bonus - 0.1 * d.diurnal + 0.15 * wet_surface - 0.01 * (temperature - 25.0).max(0.0)).clamp(0.05, 1.0)
}

/// Light reaching the ground in `[0, 1]`:
/// `daylight × (1 − (1 − τ) k) × (1 − canopy)`.
#[inline]
pub(crate) fn cell_light(cloudiness: f32, canopy: f32, d: &Drivers) -> f32 {
    d.daylight * (1.0 - (1.0 - d.transmission) * cloudiness) * (1.0 - canopy.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drivers(season: f32, diurnal: f32) -> Drivers {
        Drivers {
            season,
            diurnal,
            seasonality: 10.0,
            diurnal_half_range: 5.0,
            weather_offset: 0.0,
            humidity_bonus: 0.0,
            daylight: 1.0,
            transmission: 0.5,
            cover: 1.0,
        }
    }

    #[test]
    fn lapse_rate_and_latitude() {
        let cfg =
            WorldConfig { height: 101, base_temperature_c: 15.0, latitude_gradient: 10.0, ..WorldConfig::default() };
        assert!((annual_mean_temperature(&cfg, 50, 0.0) - 15.0).abs() < 1e-5);
        assert!((annual_mean_temperature(&cfg, 50, 1000.0) - 8.5).abs() < 1e-5);
        assert!(annual_mean_temperature(&cfg, 0, 0.0) < annual_mean_temperature(&cfg, 100, 0.0));
    }

    #[test]
    fn seasons_days_clouds_and_canopy() {
        let summer = cell_temperature(10.0, 1.0, 0.0, &drivers(1.0, 0.0));
        let winter = cell_temperature(10.0, 1.0, 0.0, &drivers(-1.0, 0.0));
        assert_eq!((summer, winter), (20.0, 0.0));
        let maritime = cell_temperature(10.0, 0.3, 0.0, &drivers(1.0, 0.0));
        assert!(maritime < summer);
        let clear_day = cell_temperature(10.0, 1.0, 0.0, &drivers(0.0, 1.0));
        let cloudy_day = cell_temperature(10.0, 1.0, 1.0, &drivers(0.0, 1.0));
        assert!(clear_day > cloudy_day);
        let d = drivers(0.0, 0.0);
        assert_eq!(cell_light(0.0, 0.0, &d), 1.0);
        assert_eq!(cell_light(1.0, 0.0, &d), 0.5);
        assert!((cell_light(0.0, 0.75, &d) - 0.25).abs() < 1e-6);
    }
}
