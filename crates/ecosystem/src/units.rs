//! Physical constants, unit conversions and the allometric / thermal
//! response functions shared by every ecosystem module.
//!
//! Units used throughout the crate:
//!
//! * **mass** — plant, litter and fungal pools in kg of dry matter per m²;
//!   animal bodies and carcasses in kg of wet mass;
//! * **energy** — kJ (gross energy of tissue, food and reserves);
//! * **nutrients** — grams of the generic mineral nutrient of
//!   `physics-world` (one pool standing for N and P together);
//! * **space** — one world unit is one terrain cell of `cell_size_m` metres;
//! * **time** — one tick (6 simulated minutes by default). Physiological
//!   rates are given per simulated day; life-history durations (lifespans,
//!   gestation, maturity, tree growth per year) are given in real days or
//!   years and compressed by `days_per_year / 365` (see [`TimeScale`]).

use serde::{Deserialize, Serialize};

/// Gross energy of plant dry matter (kJ/kg).
pub const PLANT_KJ_PER_KG: f64 = 18_000.0;
/// Gross energy of litter / detritus dry matter (kJ/kg). Animal remains that
/// enter the litter pool are converted at this density, so energy is exact
/// while "mass" is a dry-matter equivalent.
pub const DETRITUS_KJ_PER_KG: f64 = 18_000.0;
/// Gross energy of fungal dry matter (kJ/kg).
pub const FUNGI_KJ_PER_KG: f64 = 18_000.0;
/// Gross energy of wet animal tissue (kJ/kg).
pub const TISSUE_KJ_PER_KG: f64 = 7_000.0;
/// Nutrient content of wet animal tissue (g/kg, ≈ 3 % N).
pub const ANIMAL_NUTRIENT_G_PER_KG: f32 = 30.0;
/// Extra energy spent (and respired) per kJ of tissue synthesised (growth
/// and embryo production).
pub const SYNTHESIS_OVERHEAD: f32 = 0.5;
/// Beer–Lambert light extinction coefficient `k` (Monsi & Saeki 1953).
pub const LIGHT_EXTINCTION: f32 = 0.5;
/// Mean of the light-saturation factor over a clear day; growth per tick is
/// divided by it so a clear, light-saturated day integrates to the daily rate.
pub const LIGHT_DUTY: f32 = 0.4;
/// Boltzmann constant (eV/K).
pub const BOLTZMANN_EV_K: f64 = 8.617e-5;
/// Activation energy of metabolism (eV), Brown et al. (2004).
pub const ACTIVATION_EV: f64 = 0.63;
/// Reference temperature of the metabolic normalisation (K, 20 °C).
pub const REFERENCE_K: f64 = 293.15;
/// Metabolic normalisation at 20 °C (W per kg^0.75), research note §3.2.
pub const MTE_NORMALISATION_W: f64 = 0.78;
/// Ectotherm / endotherm metabolic ratio at equal body temperature.
pub const ECTOTHERM_FACTOR: f64 = 0.16;
/// Seconds per day.
pub const SECONDS_PER_DAY: f64 = 86_400.0;

/// Boltzmann–Arrhenius factor `exp(−E/k (1/T − 1/T_ref))` relative to 20 °C.
pub fn boltzmann_factor(temperature_c: f64) -> f64 {
    let t = (temperature_c + 273.15).max(200.0);
    (-(ACTIVATION_EV / BOLTZMANN_EV_K) * (1.0 / t - 1.0 / REFERENCE_K)).exp()
}

/// Basal metabolic rate (W) from the Metabolic Theory of Ecology:
/// `B = 0.78 · c_th · M^{3/4} · e^{−E/kT}` (Kleiber scaling with the
/// Boltzmann temperature correction; `c_th = 1` for endotherms, 0.16 for
/// ectotherms). For endotherms at 37 °C this gives ≈ 3.1 W·M^{3/4}, close to
/// Kleiber's 3.39 W·M^{3/4}.
pub fn basal_metabolic_w(mass_kg: f64, endotherm: bool, body_temperature_c: f64) -> f64 {
    let c = if endotherm { 1.0 } else { ECTOTHERM_FACTOR };
    MTE_NORMALISATION_W * c * mass_kg.max(0.0).powf(0.75) * boltzmann_factor(body_temperature_c)
}

/// Locomotion mode of the Hirt et al. (2017) speed model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedModel {
    /// Running animals.
    Running,
    /// Flying animals.
    Flying,
    /// Swimming animals.
    Swimming,
}

/// Maximum speed (m/s) from Hirt et al. (2017):
/// `v = a M^b (1 − e^{−h M^i})` km/h with the published parameters.
pub fn hirt_max_speed_m_s(model: SpeedModel, mass_kg: f64) -> f64 {
    let (a, b, h, i) = match model {
        SpeedModel::Running => (25.5, 0.26, 22.0, -0.6),
        SpeedModel::Flying => (142.8, 0.24, 2.4, -0.72),
        SpeedModel::Swimming => (11.2, 0.36, 19.5, -0.56),
    };
    let m = mass_kg.max(1e-9);
    let km_h = a * m.powf(b) * (1.0 - (-h * m.powf(i)).exp());
    km_h / 3.6
}

/// Net cost of transport of running animals (J/m), Taylor et al. (1982):
/// `10.7 · M^{0.68}` (from `10.7 M^{−0.32}` J kg⁻¹ m⁻¹).
pub fn cost_of_transport_j_per_m(mass_kg: f64) -> f64 {
    10.7 * mass_kg.max(0.0).powf(0.68)
}

/// Net cost of transport of flapping flight as a fraction of running at
/// the same mass: `3.7 M^{−0.32}` J kg⁻¹ m⁻¹, close to Tucker's (1975)
/// `3.6 M^{−0.31}` for birds (Schmidt-Nielsen 1972: flying is about a third
/// as costly per metre as running).
pub const FLYING_COT_FRACTION: f64 = 0.35;
/// Net cost of transport of swimming as a fraction of running at the same
/// mass (`1.3 M^{−0.32}` J kg⁻¹ m⁻¹): swimming fish pay about an order of
/// magnitude less per metre than runners (Schmidt-Nielsen 1972). An order
/// of magnitude estimate, documented as an approximation.
pub const SWIMMING_COT_FRACTION: f64 = 0.12;

/// Beta temperature response (Yan & Hunt 1999): 0 outside `(min, max)`,
/// 1 at `opt`.
pub fn beta_temperature(t: f32, min: f32, opt: f32, max: f32) -> f32 {
    if !(t > min && t < max) || !(min < opt && opt < max) {
        return 0.0;
    }
    let exponent = (opt - min) / (max - opt);
    let value = ((max - t) / (max - opt)) * ((t - min) / (opt - min)).powf(exponent);
    value.clamp(0.0, 1.0)
}

/// Q10 temperature factor `q10^{(T − T_ref)/10}`, zero below −2 °C (frozen
/// litter does not decompose).
pub fn q10_factor(t: f32, q10: f32, reference_c: f32) -> f32 {
    if t < -2.0 { 0.0 } else { q10.powf((t - reference_c) / 10.0) }
}

/// Moisture response of decomposition: a hump peaking at `optimum` relative
/// saturation (Linn & Doran 1984), floored at 0.05.
pub fn moisture_hump(moisture: f32, optimum: f32) -> f32 {
    let optimum = optimum.max(0.05);
    (1.0 - ((moisture - optimum) / optimum).powi(2)).clamp(0.05, 1.0)
}

/// Linear ramp from 0 at `lo` to 1 at `hi`.
pub fn ramp(x: f32, lo: f32, hi: f32) -> f32 {
    if hi <= lo {
        return if x >= hi { 1.0 } else { 0.0 };
    }
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Conversion between biological and simulated time and space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeScale {
    /// Ticks per simulated day (from the world calendar).
    pub ticks_per_day: u32,
    /// Days per simulated year (from the world calendar).
    pub days_per_year: u32,
    /// Cell size in metres.
    pub cell_size_m: f32,
    /// Seconds of continuous locomotion represented by one tick (see the
    /// `motion_seconds` parameter).
    pub motion_seconds: f32,
}

impl TimeScale {
    /// Length of a tick in days.
    pub fn dt_days(&self) -> f32 {
        1.0 / self.ticks_per_day.max(1) as f32
    }

    /// Length of a tick in hours.
    pub fn dt_hours(&self) -> f32 {
        24.0 / self.ticks_per_day.max(1) as f32
    }

    /// Ratio of the simulated year to a real year (`days_per_year / 365`).
    pub fn year_compression(&self) -> f64 {
        f64::from(self.days_per_year) / 365.0
    }

    /// Ticks corresponding to a biological duration given in real days.
    pub fn life_ticks_from_days(&self, real_days: f32) -> f64 {
        f64::from(real_days.max(0.0)) * self.year_compression() * f64::from(self.ticks_per_day)
    }

    /// Ticks corresponding to a biological duration given in years.
    pub fn life_ticks_from_years(&self, years: f32) -> f64 {
        f64::from(years.max(0.0)) * f64::from(self.days_per_year) * f64::from(self.ticks_per_day)
    }

    /// Speed in cells per tick for a speed in m/s.
    pub fn cells_per_tick(&self, speed_m_s: f32) -> f32 {
        speed_m_s * self.motion_seconds / self.cell_size_m.max(1e-3)
    }

    /// Length in cells.
    pub fn cells(&self, metres: f32) -> f32 {
        metres / self.cell_size_m.max(1e-3)
    }

    /// Area of one cell (m²).
    pub fn cell_area_m2(&self) -> f32 {
        self.cell_size_m * self.cell_size_m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kleiber_scaling_and_temperature_correction() {
        // An endotherm at 37 °C: ≈ 3.06 W per kg^0.75 (Kleiber: 3.39).
        let one = basal_metabolic_w(1.0, true, 37.0);
        assert!((one - 3.06).abs() < 0.05, "{one}");
        // Doubling the mass multiplies the rate by 2^0.75.
        let ratio = basal_metabolic_w(2.0, true, 37.0) / one;
        assert!((ratio - 2f64.powf(0.75)).abs() < 1e-9);
        // Ectotherms burn 16 % of an endotherm at equal temperature.
        let ecto = basal_metabolic_w(1.0, false, 37.0);
        assert!((ecto / one - 0.16).abs() < 1e-9);
        // +10 °C raises the rate by ≈ 2.2 (E = 0.63 eV).
        let warm = boltzmann_factor(30.0) / boltzmann_factor(20.0);
        assert!((2.0..2.5).contains(&warm), "{warm}");
    }

    #[test]
    fn hirt_speed_is_hump_shaped() {
        let wolf = hirt_max_speed_m_s(SpeedModel::Running, 30.0) * 3.6;
        assert!((55.0..70.0).contains(&wolf), "wolf {wolf} km/h");
        let mouse = hirt_max_speed_m_s(SpeedModel::Running, 0.02) * 3.6;
        let elephant = hirt_max_speed_m_s(SpeedModel::Running, 4000.0) * 3.6;
        let deer = hirt_max_speed_m_s(SpeedModel::Running, 100.0) * 3.6;
        assert!(mouse < deer && elephant < deer, "{mouse} {deer} {elephant}");
    }

    #[test]
    fn response_functions() {
        assert!((beta_temperature(20.0, 0.0, 20.0, 40.0) - 1.0).abs() < 1e-6);
        assert_eq!(beta_temperature(-1.0, 0.0, 20.0, 40.0), 0.0);
        assert_eq!(beta_temperature(41.0, 0.0, 20.0, 40.0), 0.0);
        assert!(beta_temperature(10.0, 0.0, 20.0, 40.0) > 0.4);
        assert!((q10_factor(30.0, 2.0, 20.0) - 2.0).abs() < 1e-6);
        assert_eq!(q10_factor(-5.0, 2.0, 20.0), 0.0);
        assert!((moisture_hump(0.6, 0.6) - 1.0).abs() < 1e-6);
        assert!(moisture_hump(0.05, 0.6) < 0.3);
        assert_eq!(ramp(0.5, 0.0, 1.0), 0.5);
    }

    #[test]
    fn time_scale_conversions() {
        let ts = TimeScale { ticks_per_day: 240, days_per_year: 120, cell_size_m: 10.0, motion_seconds: 10.0 };
        assert!((ts.dt_hours() - 0.1).abs() < 1e-6);
        // 365 real days are one simulated year.
        assert!((ts.life_ticks_from_days(365.0) - 120.0 * 240.0).abs() < 1e-6);
        assert!((ts.life_ticks_from_years(2.0) - 2.0 * 120.0 * 240.0).abs() < 1e-6);
        assert!((ts.cells_per_tick(5.0) - 5.0).abs() < 1e-6);
    }
}
