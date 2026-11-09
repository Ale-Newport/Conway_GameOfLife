//! Generation pipeline from a heightmap to a classified landscape: drainage,
//! lakes, rivers, slope, biomes, soils and fertility.

use noise::{NoiseFn, Perlin};
use simulation_core::SimRng;

use crate::biome::{Biome, SoilType, SurfaceKind, whittaker};
use crate::cells::{distance_to, smoothstep};
use crate::climate::precipitation_pattern;
use crate::config::WorldConfig;
use crate::drainage::{
    RiverInputs, d8_receivers, flow_accumulation, ocean_mask, place_rivers, priority_flood, select_lakes,
};
use crate::streams;
use crate::terrain::noise_seed;

/// Hydrologically conditioned terrain.
pub(crate) struct Landscape {
    /// Elevation after depression filling and river carving (m).
    pub elevation: Vec<f32>,
    /// Surface kind of every cell.
    pub surface: Vec<SurfaceKind>,
    /// Initial lake water depth (m), zero elsewhere.
    pub lake_depth: Vec<f32>,
    /// D8 receivers on the filled surface.
    pub receivers: Vec<u8>,
    /// Precipitation-weighted flow accumulation.
    pub accumulation: Vec<f32>,
    /// Relative precipitation pattern.
    pub precipitation: Vec<f32>,
}

/// Runs the drainage pipeline on a raw heightmap: ocean mask → Priority-Flood
/// → lake selection (other depressions filled) → precipitation pattern → D8
/// receivers and precipitation-weighted accumulation → river placement and
/// carving → final ocean mask.
pub(crate) fn shape_landscape(cfg: &WorldConfig, raw_elevation: Vec<f32>) -> Landscape {
    let (w, h) = (cfg.width, cfg.height);
    let n = w * h;
    let mut elevation = raw_elevation;
    let ocean = ocean_mask(&elevation, w, h);
    let filled = priority_flood(&elevation, &ocean, w, h);
    let lake = select_lakes(&mut elevation, &filled, &ocean, w, h, cfg.lake_tendency);
    let precipitation = precipitation_pattern(cfg, &elevation, &ocean, &lake);
    let receivers = d8_receivers(&filled, &ocean, w, h);
    let accumulation = flow_accumulation(&filled, &receivers, &ocean, &precipitation, w, h);
    let mut rng = SimRng::derive(cfg.seed, streams::RIVERS);
    let inputs = RiverInputs {
        width: w,
        height: h,
        count: cfg.river_count,
        receivers: &receivers,
        accumulation: &accumulation,
        ocean: &ocean,
        lake: &lake,
        precipitation: &precipitation,
    };
    let river = place_rivers(&inputs, &mut elevation, &mut rng);
    let ocean = ocean_mask(&elevation, w, h);
    let surface: Vec<SurfaceKind> = (0..n)
        .map(|i| {
            if ocean[i] {
                SurfaceKind::Ocean
            } else if lake[i] {
                SurfaceKind::Lake
            } else if river[i] {
                SurfaceKind::River
            } else {
                SurfaceKind::Land
            }
        })
        .collect();
    let lake_depth =
        (0..n)
            .map(|i| {
                if surface[i] == SurfaceKind::Lake {
                    (filled[i] - f64::from(elevation[i])).max(0.0) as f32
                } else {
                    0.0
                }
            })
            .collect();
    Landscape { elevation, surface, lake_depth, receivers, accumulation, precipitation }
}

/// Slope magnitude (rise/run) by central differences (one-sided at the
/// border).
pub(crate) fn slope_grid(elevation: &[f32], width: usize, height: usize, cell_size_m: f32) -> Vec<f32> {
    let mut slope = vec![0.0f32; width * height];
    for y in 0..height {
        let (yu, yd) = (y.saturating_sub(1), (y + 1).min(height - 1));
        for x in 0..width {
            let (xl, xr) = (x.saturating_sub(1), (x + 1).min(width - 1));
            let gx = if xr > xl {
                (elevation[y * width + xr] - elevation[y * width + xl]) / ((xr - xl) as f32 * cell_size_m)
            } else {
                0.0
            };
            let gy = if yd > yu {
                (elevation[yd * width + x] - elevation[yu * width + x]) / ((yd - yu) as f32 * cell_size_m)
            } else {
                0.0
            };
            slope[y * width + x] = (gx * gx + gy * gy).sqrt();
        }
    }
    slope
}

/// Inputs of the biome, soil and fertility classifications.
pub(crate) struct ClassInputs<'a> {
    pub cfg: &'a WorldConfig,
    pub elevation: &'a [f32],
    pub slope: &'a [f32],
    pub surface: &'a [SurfaceKind],
    pub mean_temperature: &'a [f32],
    pub precip_factor: &'a [f32],
    pub accumulation: &'a [f32],
}

/// Biome of every cell:
///
/// 1. Ocean cells: **Reef** if depth ≤ 30 m and annual mean ≥ 20 °C, else **Ocean**.
/// 2. Lake / river cells: **Lake** / **River**.
/// 3. Land within 2 cells of the ocean, below `3 m + 0.002 × peak` and with
///    slope < 0.25: **Beach**.
/// 4. Land above 55 % of the highest land and above 600 m, or with slope ≥ 1:
///    **Mountain**.
/// 5. **Wetland** on land with T ≥ −1 °C and either (a) slope < 0.03,
///    P ≥ 450 mm and a high wetness index `ln((A + 1)/(slope + 0.002)) ≥ 9.5`
///    (A = accumulation), (b) slope < 0.01, P ≥ 450 mm and fresh water within
///    one cell, or (c) poorly drained flats (slope < 0.002, i.e. filled
///    basins and plains) with P ≥ 600 mm.
/// 6. Otherwise the Whittaker table ([`whittaker`]).
pub(crate) fn classify_biomes(c: &ClassInputs<'_>) -> Vec<Biome> {
    let (w, h) = (c.cfg.width, c.cfg.height);
    let ocean_dist = distance_to(w, h, |i| c.surface[i] == SurfaceKind::Ocean);
    let fresh_dist = distance_to(w, h, |i| matches!(c.surface[i], SurfaceKind::Lake | SurfaceKind::River));
    let peak = (0..w * h).filter(|&i| c.surface[i] == SurfaceKind::Land).map(|i| c.elevation[i]).fold(0.0f32, f32::max);
    (0..w * h)
        .map(|i| {
            let t = c.mean_temperature[i];
            let z = c.elevation[i];
            match c.surface[i] {
                SurfaceKind::Ocean => {
                    if -z <= 30.0 && t >= 20.0 {
                        Biome::Reef
                    } else {
                        Biome::Ocean
                    }
                }
                SurfaceKind::Lake => Biome::Lake,
                SurfaceKind::River => Biome::River,
                SurfaceKind::Land => {
                    let s = c.slope[i];
                    let p = c.cfg.rainfall_mm * c.precip_factor[i];
                    let wetness = ((c.accumulation[i] + 1.0) / (s + 0.002)).ln();
                    if ocean_dist[i] <= 2 && z < 3.0 + 0.002 * peak && s < 0.25 {
                        Biome::Beach
                    } else if (z >= 0.55 * peak && z >= 600.0) || s >= 1.0 {
                        Biome::Mountain
                    } else if t >= -1.0
                        && ((s < 0.03 && p >= 450.0 && wetness >= 9.5)
                            || (s < 0.01 && p >= 450.0 && fresh_dist[i] <= 1)
                            || (s < 0.002 && p >= 600.0))
                    {
                        Biome::Wetland
                    } else {
                        whittaker(t, p)
                    }
                }
            }
        })
        .collect()
}

/// Low-frequency noise in about `[-1, 1]` used to vary soil texture.
pub(crate) fn soil_noise(cfg: &WorldConfig) -> Vec<f32> {
    let mut rng = SimRng::derive(cfg.seed, streams::SOIL);
    let perlin = Perlin::new(noise_seed(&mut rng));
    let (ox, oy) = (rng.range_f64(-500.0, 500.0), rng.range_f64(-500.0, 500.0));
    let scale = cfg.width.max(cfg.height) as f64;
    (0..cfg.width * cfg.height)
        .map(|i| {
            let (x, y) = ((i % cfg.width) as f64, (i / cfg.width) as f64);
            (perlin.get([x / scale * 5.0 + ox, y / scale * 5.0 + oy]) * 1.6) as f32
        })
        .collect()
}

/// Soil type of every cell:
///
/// * ocean → Sand; lake and river beds → Silt; beach → Sand;
/// * slope > 0.8 or mountain biome → Rock (mountain cells with soil noise
///   < −0.3 and slope ≤ 0.7 → Loam);
/// * desert → Sand (Rock where soil noise < −0.4);
/// * wetland → Peat when T < 16 °C, else Clay;
/// * within 2 cells of fresh water and slope < 0.05 → Silt (alluvium);
/// * tundra with P > 300 mm and slope < 0.05 → Peat;
/// * otherwise soil noise > 0.3 → Clay, < −0.35 → Sand, else Loam.
pub(crate) fn classify_soils(c: &ClassInputs<'_>, biomes: &[Biome], noise: &[f32]) -> Vec<SoilType> {
    let (w, h) = (c.cfg.width, c.cfg.height);
    let fresh_dist = distance_to(w, h, |i| matches!(c.surface[i], SurfaceKind::Lake | SurfaceKind::River));
    (0..w * h)
        .map(|i| {
            let s = c.slope[i];
            let n = noise[i];
            let t = c.mean_temperature[i];
            let p = c.cfg.rainfall_mm * c.precip_factor[i];
            match biomes[i] {
                Biome::Ocean | Biome::Reef | Biome::Beach => SoilType::Sand,
                Biome::Lake | Biome::River => SoilType::Silt,
                _ if s > 0.8 => SoilType::Rock,
                Biome::Mountain => {
                    if n < -0.3 && s <= 0.7 {
                        SoilType::Loam
                    } else {
                        SoilType::Rock
                    }
                }
                Biome::Desert => {
                    if n < -0.4 {
                        SoilType::Rock
                    } else {
                        SoilType::Sand
                    }
                }
                Biome::Wetland => {
                    if t < 16.0 {
                        SoilType::Peat
                    } else {
                        SoilType::Clay
                    }
                }
                _ if fresh_dist[i] <= 2 && s < 0.05 => SoilType::Silt,
                Biome::Tundra if p > 300.0 && s < 0.05 => SoilType::Peat,
                _ if n > 0.3 => SoilType::Clay,
                _ if n < -0.35 => SoilType::Sand,
                _ => SoilType::Loam,
            }
        })
        .collect()
}

/// Fertility index `[0, 1]` of every cell:
/// `base(soil) × (0.3 + 0.7 f_T f_P) × (1 − 0.6 min(slope, 1))` with
/// `f_T = smoothstep(−8, 12, T)` and `f_P = 0.2 + 0.8 smoothstep(100, 900, P)`;
/// zero on water.
pub(crate) fn fertility(c: &ClassInputs<'_>, soils: &[SoilType]) -> Vec<f32> {
    (0..c.cfg.width * c.cfg.height)
        .map(|i| {
            if c.surface[i].is_water() {
                return 0.0;
            }
            let t = c.mean_temperature[i];
            let p = c.cfg.rainfall_mm * c.precip_factor[i];
            let f_t = smoothstep(-8.0, 12.0, t);
            let f_p = 0.2 + 0.8 * smoothstep(100.0, 900.0, p);
            soils[i].properties().base_fertility * (0.3 + 0.7 * f_t * f_p) * (1.0 - 0.6 * c.slope[i].min(1.0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slope_of_a_ramp() {
        // z = 2x on a 4x3 grid with 10 m cells: slope 0.2 everywhere.
        let z: Vec<f32> = (0..12).map(|i| 2.0 * (i % 4) as f32).collect();
        let s = slope_grid(&z, 4, 3, 10.0);
        assert!(s.iter().all(|&v| (v - 0.2).abs() < 1e-6));
    }
}
