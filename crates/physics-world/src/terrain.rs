//! Heightmap synthesis: fractal Brownian motion with domain warping, an
//! optional island falloff and lake basins, mapped to metres around a
//! quantile-based sea level.

use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use rayon::prelude::*;
use simulation_core::{Grid2, SimRng};

use crate::cells::{is_border, smoothstep};
use crate::config::{MAX_OCEAN_DEPTH_M, WorldConfig};
use crate::streams;

/// Shallowest ocean depth and lowest coastal land elevation (m), so that sea
/// and land are always separated by at least one metre.
const COAST_STEP_M: f32 = 0.5;

/// Draws a seed for a `noise` generator. The `noise` crate seeds octave `k`
/// with `seed + k` (checked arithmetic in debug builds), so seeds are kept
/// well below `u32::MAX`.
pub(crate) fn noise_seed(rng: &mut SimRng) -> u32 {
    rng.next_u32() & 0x3FFF_FFFF
}

/// Fractal Brownian motion over Perlin noise with lacunarity 2.
pub(crate) fn fbm(seed: u32, octaves: usize, frequency: f64, persistence: f64) -> Fbm<Perlin> {
    Fbm::<Perlin>::new(seed)
        .set_octaves(octaves)
        .set_frequency(frequency)
        .set_persistence(persistence)
        .set_lacunarity(2.0)
}

/// Raw (dimensionless) height field before sea-level mapping.
fn raw_heights(cfg: &WorldConfig) -> Vec<f32> {
    let mut rng = SimRng::derive(cfg.seed, streams::TERRAIN);
    let continent = fbm(noise_seed(&mut rng), 3, 1.3, 0.5);
    let detail = fbm(noise_seed(&mut rng), 7, 2.6, 0.30 + 0.35 * f64::from(cfg.roughness));
    let warp_x = fbm(noise_seed(&mut rng), 3, 1.8, 0.5);
    let warp_y = fbm(noise_seed(&mut rng), 3, 1.8, 0.5);
    let basins = Perlin::new(noise_seed(&mut rng));
    let o: [f64; 8] = std::array::from_fn(|_| rng.range_f64(-500.0, 500.0));

    let (w, h) = (cfg.width, cfg.height);
    let scale = w.max(h) as f64;
    let continent_weight = 0.15 + 0.6 * f64::from(cfg.continentality);
    let warp = 0.12 + 0.25 * f64::from(cfg.roughness);
    let basin_depth = 0.10 * f64::from(cfg.lake_tendency);
    let island = cfg.island;

    let mut data = vec![0.0f32; w * h];
    data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let v = (y as f64 + 0.5) / scale;
        for (x, out) in row.iter_mut().enumerate() {
            let u = (x as f64 + 0.5) / scale;
            // Domain warping (Quilez): displace the sample point by another fBm.
            let qx = warp_x.get([u + o[0], v + o[1]]);
            let qy = warp_y.get([u + o[2], v + o[3]]);
            let d = detail.get([u + warp * qx + o[4], v + warp * qy + o[5]]);
            let c = continent.get([u + o[6], v + o[7]]);
            let mut value = continent_weight * c + (1.0 - continent_weight) * d;
            if basin_depth > 0.0 {
                let b = basins.get([u * 7.0 + o[1], v * 7.0 + o[4]]);
                value -= basin_depth * (b - 0.05).max(0.0);
            }
            if island {
                let dx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
                let dy = (y as f32 + 0.5) / h as f32 * 2.0 - 1.0;
                let fall = f64::from(smoothstep(0.4, 0.95, (dx * dx + dy * dy).sqrt()));
                value = value * (1.0 - fall) - 0.8 * fall + 0.12 * (1.0 - fall);
            }
            *out = value as f32;
        }
    });
    data
}

/// Generates the elevation grid in metres (sea level = 0 m).
///
/// The sea threshold is the `sea_level` quantile of the raw height field, so
/// `sea_level` is the fraction of cells below sea level. In island mode the
/// threshold is raised above the highest border cell so that the whole border
/// is ocean. Land maps to `0.5 m + (peak − 0.5 m) t^1.6` and sea to
/// `−(0.5 m + (1500 m − 0.5 m) t^1.2)`, where `t ∈ [0, 1]` is the normalised
/// distance from the threshold; the exponents give broad lowlands and
/// shallow continental shelves.
pub(crate) fn generate_heightmap(cfg: &WorldConfig) -> Grid2<f32> {
    let (w, h) = (cfg.width, cfg.height);
    let raw = raw_heights(cfg);
    let mut sorted = raw.clone();
    sorted.sort_by(f32::total_cmp);
    let n = sorted.len();
    let lo = sorted[0];
    let hi = sorted[n - 1];
    let quantile_index = ((f64::from(cfg.sea_level) * n as f64).floor() as usize).min(n - 1);
    let mut threshold = sorted[quantile_index];
    if cfg.island {
        let border_max = (0..n).filter(|&i| is_border(i, w, h)).map(|i| raw[i]).fold(f32::NEG_INFINITY, f32::max);
        if border_max >= threshold {
            threshold = border_max + (hi - lo).max(1e-6) * 1e-4;
        }
    }
    let peak = cfg.max_land_elevation_m();
    let land_span = (hi - threshold).max(1e-9);
    let sea_span = (threshold - lo).max(1e-9);
    let data = raw
        .iter()
        .map(|&value| {
            if value < threshold {
                let t = ((threshold - value) / sea_span).clamp(0.0, 1.0);
                -(COAST_STEP_M + (MAX_OCEAN_DEPTH_M - COAST_STEP_M) * t.powf(1.2))
            } else {
                let t = ((value - threshold) / land_span).clamp(0.0, 1.0);
                COAST_STEP_M + (peak - COAST_STEP_M) * t.powf(1.6)
            }
        })
        .collect();
    Grid2::from_vec(w, h, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heightmap_is_deterministic_and_respects_sea_fraction() {
        let cfg = WorldConfig { width: 64, height: 48, sea_level: 0.4, ..WorldConfig::default() };
        let a = generate_heightmap(&cfg);
        let b = generate_heightmap(&cfg);
        assert_eq!(a, b);
        let below = a.as_slice().iter().filter(|&&z| z < 0.0).count() as f32 / (64.0 * 48.0);
        assert!((below - 0.4).abs() < 0.01, "sea fraction {below}");
        let max = a.as_slice().iter().copied().fold(f32::MIN, f32::max);
        assert!((max - cfg.max_land_elevation_m()).abs() < 1.0);
        let other = generate_heightmap(&cfg.clone().with_seed(2));
        assert_ne!(a, other);
    }

    #[test]
    fn island_mode_drowns_the_whole_border() {
        let cfg = WorldConfig { width: 64, height: 64, island: true, sea_level: 0.1, ..WorldConfig::default() };
        let z = generate_heightmap(&cfg);
        for i in 0..64 * 64 {
            if is_border(i, 64, 64) {
                assert!(z.as_slice()[i] < 0.0);
            }
        }
        assert!(z.as_slice().iter().any(|&v| v > 0.0), "island must have land");
    }
}
