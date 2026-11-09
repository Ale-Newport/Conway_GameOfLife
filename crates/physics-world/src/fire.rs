//! Probabilistic fire-spread cellular automaton after Alexandridis et al.
//! (2008), with fuel consumption, rain/water extinction, lava sources and
//! lightning ignition.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent, SimRng};

use crate::biome::{Biome, SurfaceKind};
use crate::cells::{DIRS, STEP_LEN, neighbor};
use crate::report::{EnvironmentReport, FuelBurn};
use crate::world::World;

/// Fuel load (kg/m²) below which a cell cannot burn.
pub const MIN_FUEL_KG_M2: f32 = 0.05;
/// Fuel load (kg/m²) a burned cell needs (after at least one day) to carry
/// fire again.
pub const RECOVERY_FUEL_KG_M2: f32 = 0.3;
/// Standing water (m) that makes a cell unburnable and extinguishes fire.
pub const WET_DEPTH_M: f32 = 0.005;
/// Snow water equivalent (m) that makes a cell unburnable.
pub const SNOW_COVER_M: f32 = 0.005;
/// Fraction of the remaining fuel consumed per fire sub-step.
pub const BURN_FRACTION: f32 = 0.35;
/// Fuel load (kg/m²) of a "normal" fire: intensity 1 when it burns.
pub const REFERENCE_FUEL_KG_M2: f32 = 2.0;
/// Sub-steps after which a cell burns out regardless of fuel.
pub const MAX_BURN_STEPS: u32 = 40;
/// Spread intensity of lava.
pub const LAVA_INTENSITY: f32 = 2.0;
/// Intensity of a freshly ignited cell.
const IGNITION_INTENSITY: f32 = 0.5;
/// Alexandridis et al. (2008) constants: base probability `p_h`, wind
/// coefficients `c1`, `c2` (per m/s) and slope coefficient `a` (per degree).
pub const P_H: f32 = 0.58;
/// See [`P_H`].
pub const C1: f32 = 0.045;
/// See [`P_H`].
pub const C2: f32 = 0.131;
/// See [`P_H`].
pub const A_SLOPE: f32 = 0.078;
/// Fuel moisture of extinction (fraction): no spread into fuel this wet.
pub const MOISTURE_OF_EXTINCTION: f32 = 0.9;
/// Upper bound on the lightning strikes simulated in one tick (it only
/// matters for huge maps with extreme strike rates).
pub const MAX_STRIKES_PER_TICK: u64 = 10_000;

/// Fire state of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum FireState {
    /// Water, bare ground, snow or too little fuel.
    #[default]
    Unburnable,
    /// Enough dry fuel to ignite.
    Fuel,
    /// Burning with a relative `intensity` for `steps` fire sub-steps.
    Burning {
        /// Relative fire intensity (1 = consuming 35 % of 2 kg/m² per step).
        intensity: f32,
        /// Sub-steps spent burning.
        steps: u32,
    },
    /// Burned out at tick `since`.
    Burned {
        /// Tick at which the fire went out.
        since: u64,
    },
    /// Molten lava (volcano): ignites neighbours, ignores rain.
    Lava {
        /// Sub-steps until it solidifies.
        steps_left: u32,
    },
}

impl FireState {
    /// Whether the cell is an active heat source (burning or lava).
    pub fn is_active(self) -> bool {
        matches!(self, Self::Burning { .. } | Self::Lava { .. })
    }

    /// Display priority used when downsampling (higher wins).
    pub fn priority(self) -> u8 {
        match self {
            Self::Unburnable => 0,
            Self::Fuel => 1,
            Self::Burned { .. } => 2,
            Self::Burning { .. } => 3,
            Self::Lava { .. } => 4,
        }
    }
}

/// Alexandridis vegetation factor `p_veg` by biome (forests carry fire
/// better, humid or sparse biomes worse).
pub fn vegetation_factor(biome: Biome) -> f32 {
    match biome {
        Biome::TemperateForest => 0.4,
        Biome::Savanna | Biome::Grassland => 0.0,
        Biome::Beach | Biome::Desert | Biome::Tundra | Biome::TropicalRainforest => -0.3,
        Biome::Mountain => -0.2,
        Biome::Wetland => -0.4,
        Biome::Ocean | Biome::Reef | Biome::Lake | Biome::River => -1.0,
    }
}

/// Alexandridis density factor `p_den` from fuel load:
/// `clamp(0.3 ln(fuel / 1 kg m⁻²) / ln 3, −0.4, 0.3)`.
pub fn density_factor(fuel_kg_m2: f32) -> f32 {
    (0.3 * (fuel_kg_m2.max(1e-3)).ln() / 3f32.ln()).clamp(-0.4, 0.3)
}

/// Fuel moisture proxy in `[0, 1]`: `0.5 s + 0.35 RH + 0.15 min(1, d / 1 cm)`
/// with soil saturation `s`, relative humidity `RH` and surface water `d`.
pub fn fuel_moisture(soil_moisture: f32, humidity: f32, water_depth: f32) -> f32 {
    (0.5 * soil_moisture + 0.35 * humidity + 0.15 * (water_depth / 0.01).min(1.0)).clamp(0.0, 1.0)
}

/// Inputs of [`spread_probability`].
#[derive(Clone, Copy, Debug)]
pub struct SpreadInputs {
    /// Wind velocity (m/s).
    pub wind: Vec2,
    /// Unit vector from the burning cell to the target cell.
    pub direction: Vec2,
    /// Target elevation minus source elevation (m).
    pub rise_m: f32,
    /// Horizontal distance between cell centres (m).
    pub distance_m: f32,
    /// Fuel load of the target (kg/m²).
    pub target_fuel: f32,
    /// Biome of the target.
    pub target_biome: Biome,
    /// Fuel moisture of the target.
    pub target_moisture: f32,
    /// Intensity of the source.
    pub source_intensity: f32,
    /// Global multiplier (`fire_spread`).
    pub spread_factor: f32,
}

/// Probability that a burning cell ignites a neighbour in one sub-step:
///
/// `p = p_h (1 + p_veg)(1 + p_den) p_w p_s f_m f_i k`, clamped to `[0, 1]`,
/// with `p_w = exp(c1 V) exp(c2 V (cos θ − 1))` (θ = angle between wind and
/// spread direction), `p_s = exp(a θ_s)` (θ_s = slope angle in degrees,
/// positive uphill), `f_m = max(0, 1 − m / 0.9)` (fuel moisture `m`),
/// `f_i = clamp(0.6 + 0.4 I, 0.6, 1.8)` (source intensity `I`) and the
/// user multiplier `k`.
pub fn spread_probability(s: &SpreadInputs) -> f32 {
    let speed = s.wind.length();
    let cos_theta = if speed > 1e-6 { s.wind.dot(s.direction) / speed } else { 1.0 };
    let p_wind = (C1 * speed).exp() * (C2 * speed * (cos_theta - 1.0)).exp();
    let slope_deg = (s.rise_m / s.distance_m.max(1e-3)).atan().to_degrees();
    let p_slope = (A_SLOPE * slope_deg).exp();
    let p_veg = 1.0 + vegetation_factor(s.target_biome);
    let p_den = 1.0 + density_factor(s.target_fuel);
    let f_moisture = (1.0 - s.target_moisture / MOISTURE_OF_EXTINCTION).max(0.0);
    let f_intensity = (0.6 + 0.4 * s.source_intensity).clamp(0.6, 1.8);
    (P_H * p_veg * p_den * p_wind * p_slope * f_moisture * f_intensity * s.spread_factor).clamp(0.0, 1.0)
}

/// Re-derives the passive states (`Unburnable` / `Fuel` / recovery from
/// `Burned`) of a cell; active fires are left untouched.
pub(crate) fn refreshed_state(state: FireState, burnable: bool, fuel: f32, tick: u64, ticks_per_day: u64) -> FireState {
    match state {
        FireState::Burning { .. } | FireState::Lava { .. } => state,
        FireState::Burned { since } => {
            if burnable && fuel >= RECOVERY_FUEL_KG_M2 && tick.saturating_sub(since) >= ticks_per_day {
                FireState::Fuel
            } else {
                state
            }
        }
        FireState::Unburnable | FireState::Fuel => {
            if burnable {
                FireState::Fuel
            } else {
                FireState::Unburnable
            }
        }
    }
}

/// Poisson sample (Knuth's method; normal approximation for large means).
pub(crate) fn poisson(rng: &mut SimRng, mean: f64) -> u64 {
    if mean <= 0.0 {
        return 0;
    }
    if mean > 30.0 {
        return rng.normal_with(mean, mean.sqrt()).round().max(0.0) as u64;
    }
    let limit = (-mean).exp();
    let mut k = 0;
    let mut p = rng.f64();
    while p > limit {
        k += 1;
        p *= rng.f64();
    }
    k
}

impl World {
    /// Whether cell `i` can currently carry fire.
    pub(crate) fn burnable(&self, i: usize) -> bool {
        self.surface.as_slice()[i] == SurfaceKind::Land
            && self.fuel.as_slice()[i] >= MIN_FUEL_KG_M2
            && self.water_depth.as_slice()[i] < WET_DEPTH_M
            && self.snow.as_slice()[i] < SNOW_COVER_M
    }

    /// Recomputes passive fire states from the current fuel, water and snow.
    pub(crate) fn refresh_fire_states(&mut self) {
        let w = self.config.width;
        let tick = self.tick;
        let tpd = u64::from(self.calendar.ticks_per_day());
        let surface = self.surface.as_slice();
        let fuel = self.fuel.as_slice();
        let water = self.water_depth.as_slice();
        let snow = self.snow.as_slice();
        self.fire.as_mut_slice().par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, state) in row.iter_mut().enumerate() {
                let i = y * w + x;
                let burnable = surface[i] == SurfaceKind::Land
                    && fuel[i] >= MIN_FUEL_KG_M2
                    && water[i] < WET_DEPTH_M
                    && snow[i] < SNOW_COVER_M;
                *state = refreshed_state(*state, burnable, fuel[i], tick, tpd);
            }
        });
    }

    /// Sets cell `i` burning if it currently can. Returns whether it ignited.
    pub(crate) fn ignite_index(&mut self, i: usize) -> bool {
        let tick = self.tick;
        let tpd = u64::from(self.calendar.ticks_per_day());
        let burnable = self.burnable(i);
        let state = refreshed_state(self.fire.as_slice()[i], burnable, self.fuel.as_slice()[i], tick, tpd);
        if state != FireState::Fuel {
            self.fire.as_mut_slice()[i] = state;
            return false;
        }
        self.fire.as_mut_slice()[i] = FireState::Burning { intensity: IGNITION_INTENSITY, steps: 0 };
        if let Err(pos) = self.burning.binary_search(&(i as u32)) {
            self.burning.insert(pos, i as u32);
        }
        true
    }

    /// Fuel moisture of cell `i`.
    fn cell_fuel_moisture(&self, i: usize) -> f32 {
        fuel_moisture(self.soil_moisture.as_slice()[i], self.humidity.as_slice()[i], self.water_depth.as_slice()[i])
    }

    /// One synchronous fire sub-step: spread from every active cell, then
    /// fuel consumption and extinction, then the new ignitions take effect.
    fn fire_substep(&mut self, consumed: &mut Vec<(u32, f32)>, report: &mut EnvironmentReport) {
        let (w, h) = (self.config.width, self.config.height);
        let cell = self.config.cell_size_m;
        let wind = self.weather.wind();
        let spread_factor = self.config.fire_spread;
        let mut ignitions = Vec::new();
        for &src in &self.burning {
            let i = src as usize;
            let intensity = match self.fire.as_slice()[i] {
                FireState::Burning { intensity, .. } => intensity,
                FireState::Lava { .. } => LAVA_INTENSITY,
                _ => continue,
            };
            for (k, &(dx, dy)) in DIRS.iter().enumerate() {
                let Some(j) = neighbor(i, k, w, h) else { continue };
                if self.fire.as_slice()[j] != FireState::Fuel {
                    continue;
                }
                let inputs = SpreadInputs {
                    wind,
                    direction: Vec2::new(dx as f32, dy as f32).normalize(),
                    rise_m: self.elevation.as_slice()[j] - self.elevation.as_slice()[i],
                    distance_m: STEP_LEN[k] * cell,
                    target_fuel: self.fuel.as_slice()[j],
                    target_biome: self.biome.as_slice()[j],
                    target_moisture: self.cell_fuel_moisture(j),
                    source_intensity: intensity,
                    spread_factor,
                };
                if self.rngs.fire.chance(f64::from(spread_probability(&inputs))) {
                    ignitions.push(j as u32);
                }
            }
        }

        let tick = self.tick;
        let mut still_active = Vec::with_capacity(self.burning.len() + ignitions.len());
        for idx in 0..self.burning.len() {
            let i = self.burning[idx] as usize;
            let next = match self.fire.as_slice()[i] {
                FireState::Burning { steps, .. } => {
                    let fuel = self.fuel.as_slice()[i];
                    let burn = fuel * BURN_FRACTION;
                    self.fuel.as_mut_slice()[i] = fuel - burn;
                    if burn > 0.0 {
                        consumed.push((i as u32, burn));
                    }
                    let rain = self.precipitation.as_slice()[i];
                    let rained_out = rain > 0.0 && self.rngs.fire.chance(f64::from(1.0 - (-0.35 * rain).exp()));
                    let wet = self.water_depth.as_slice()[i] >= WET_DEPTH_M || self.snow.as_slice()[i] >= SNOW_COVER_M;
                    if fuel - burn < MIN_FUEL_KG_M2 || wet || rained_out || steps + 1 >= MAX_BURN_STEPS {
                        FireState::Burned { since: tick }
                    } else {
                        let intensity = (burn / (BURN_FRACTION * REFERENCE_FUEL_KG_M2)).clamp(0.1, 3.0);
                        FireState::Burning { intensity, steps: steps + 1 }
                    }
                }
                FireState::Lava { steps_left } if steps_left > 1 => FireState::Lava { steps_left: steps_left - 1 },
                FireState::Lava { .. } => FireState::Burned { since: tick },
                other => other,
            };
            self.fire.as_mut_slice()[i] = next;
            if next.is_active() {
                still_active.push(i as u32);
            } else if matches!(next, FireState::Burned { .. }) {
                report.burned_cells += 1;
            }
        }
        ignitions.sort_unstable();
        ignitions.dedup();
        for &j in &ignitions {
            if self.fire.as_slice()[j as usize] == FireState::Fuel {
                self.fire.as_mut_slice()[j as usize] = FireState::Burning { intensity: IGNITION_INTENSITY, steps: 0 };
                still_active.push(j);
                report.ignitions += 1;
            }
        }
        still_active.sort_unstable();
        still_active.dedup();
        self.burning = still_active;
    }

    /// Lightning: during thunderstorms, `Poisson(rate × map km² × Δt)`
    /// cloud-to-ground strikes (at most [`MAX_STRIKES_PER_TICK`]) hit
    /// uniformly random cells of the whole map, so every cell receives the
    /// configured strike density; strikes on the ocean have no effect. A
    /// strike on fuel ignites it with probability `0.35 (1 − m)` (fuel
    /// moisture `m`).
    fn lightning(&mut self, report: &mut EnvironmentReport) {
        if !self.weather.has_lightning() || self.config.lightning_rate <= 0.0 {
            return;
        }
        let n = self.config.width * self.config.height;
        let km2 = n as f64 * self.config.cell_area_m2() / 1e6;
        let mean = f64::from(self.config.lightning_rate) * km2 * f64::from(self.calendar.dt_hours());
        let strikes = poisson(&mut self.rngs.fire, mean).min(MAX_STRIKES_PER_TICK);
        for _ in 0..strikes {
            let i = self.rngs.fire.index(n);
            if self.surface.as_slice()[i] == SurfaceKind::Ocean {
                continue;
            }
            report.lightning_strikes += 1;
            if self.fire.as_slice()[i] != FireState::Fuel {
                continue;
            }
            let moisture = self.cell_fuel_moisture(i);
            if self.rngs.fire.chance(f64::from(0.35 * (1.0 - moisture))) && self.ignite_index(i) {
                report.ignitions += 1;
                let (x, y) = (i % self.config.width, i / self.config.width);
                report.events.push(
                    SimEvent::new(self.tick, "fire", format!("Lightning started a fire at ({x}, {y})"))
                        .severity(EventSeverity::Info),
                );
            }
        }
    }

    /// Fire update of one tick: passive-state refresh, lightning, then
    /// [`Calendar::substeps_per_tick`](crate::Calendar::substeps_per_tick)
    /// synchronous sub-steps.
    pub(crate) fn fire_step(&mut self, report: &mut EnvironmentReport) {
        self.refresh_fire_states();
        self.lightning(report);
        let substeps = self.calendar.substeps_per_tick();
        let mut consumed = Vec::new();
        for _ in 0..substeps {
            if self.burning.is_empty() {
                break;
            }
            self.fire_substep(&mut consumed, report);
        }
        consumed.sort_unstable_by_key(|&(i, _)| i);
        let w = self.config.width as u32;
        for (i, amount) in consumed {
            match report.fuel_consumed.last_mut() {
                Some(last) if last.x == i % w && last.y == i / w => last.amount += amount,
                _ => report.fuel_consumed.push(FuelBurn { x: i % w, y: i / w, amount }),
            }
        }
        report.burning_cells = self.burning.len() as u32;
        self.burned_total += u64::from(report.burned_cells);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(direction: Vec2, rise: f32) -> SpreadInputs {
        SpreadInputs {
            wind: Vec2::new(8.0, 0.0),
            direction,
            rise_m: rise,
            distance_m: 50.0,
            target_fuel: 1.0,
            target_biome: Biome::Grassland,
            target_moisture: 0.2,
            source_intensity: 1.0,
            spread_factor: 1.0,
        }
    }

    #[test]
    fn wind_and_slope_bias_spread() {
        let downwind = spread_probability(&inputs(Vec2::X, 0.0));
        let upwind = spread_probability(&inputs(-Vec2::X, 0.0));
        let crosswind = spread_probability(&inputs(Vec2::Y, 0.0));
        assert!(downwind > crosswind && crosswind > upwind);
        let uphill = spread_probability(&inputs(Vec2::Y, 10.0));
        let downhill = spread_probability(&inputs(Vec2::Y, -10.0));
        assert!(uphill > crosswind && crosswind > downhill);
        // Saturated fuel does not burn.
        let mut wet = inputs(Vec2::X, 0.0);
        wet.target_moisture = 0.95;
        assert_eq!(spread_probability(&wet), 0.0);
    }

    #[test]
    fn alexandridis_wind_factor_matches_formula() {
        // V = 8 m/s, θ = 0: p_w = exp(0.045 × 8).
        let mut s = inputs(Vec2::X, 0.0);
        s.target_moisture = 0.0;
        s.target_fuel = 1.0;
        s.source_intensity = 1.0;
        let expected = (P_H * (0.045f32 * 8.0).exp()).min(1.0);
        assert!((spread_probability(&s) - expected).abs() < 1e-6);
    }

    #[test]
    fn passive_state_rules() {
        assert_eq!(refreshed_state(FireState::Unburnable, true, 1.0, 0, 240), FireState::Fuel);
        assert_eq!(refreshed_state(FireState::Fuel, false, 1.0, 0, 240), FireState::Unburnable);
        let burned = FireState::Burned { since: 100 };
        assert_eq!(refreshed_state(burned, true, 1.0, 200, 240), burned, "needs a day");
        assert_eq!(refreshed_state(burned, true, 0.1, 400, 240), burned, "needs regrown fuel");
        assert_eq!(refreshed_state(burned, true, 1.0, 400, 240), FireState::Fuel);
        let active = FireState::Burning { intensity: 1.0, steps: 2 };
        assert_eq!(refreshed_state(active, false, 0.0, 0, 240), active);
    }

    #[test]
    fn poisson_mean_is_right() {
        let mut rng = SimRng::seed_from_u64(9);
        let n = 20_000;
        let mean = (0..n).map(|_| poisson(&mut rng, 2.5)).sum::<u64>() as f64 / f64::from(n);
        assert!((mean - 2.5).abs() < 0.08, "{mean}");
        assert_eq!(poisson(&mut rng, 0.0), 0);
    }
}
