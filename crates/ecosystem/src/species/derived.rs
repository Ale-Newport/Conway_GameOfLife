//! Per-tick numbers derived from a species record and the world's time and
//! space scales: Kleiber/MTE metabolism, Hirt sprint speeds, perception in
//! cells, life-history durations in ticks, Gompertz mortality.

use serde::{Deserialize, Serialize};

use super::{Development, Locomotion, Refuge, Species, Thermoregulation, WaterKind};
use crate::units::{
    SECONDS_PER_DAY, SpeedModel, TimeScale, basal_metabolic_w, cost_of_transport_j_per_m, hirt_max_speed_m_s,
};

/// Survival to the nominal lifespan under the Gompertz hazard (1 %).
const LIFESPAN_SURVIVAL: f64 = 0.01;
/// Mortality-rate doubling time as a fraction of the lifespan.
const MRDT_FRACTION: f64 = 1.0 / 6.0;

/// Numbers derived from a [`Species`] for a given [`TimeScale`]. Rates are
/// per tick, distances in cells, durations in ticks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimalTraits {
    /// Adult body mass (kg).
    pub adult_mass_kg: f32,
    /// Endotherm.
    pub endotherm: bool,
    /// Body temperature used for endotherm metabolism (°C).
    pub body_temperature_c: f32,
    /// Basal metabolic rate of an adult at its body temperature (or 20 °C
    /// for ectotherms), kJ per day.
    pub bmr_kj_per_day: f32,
    /// Field metabolic rate without locomotion (kJ/day).
    pub field_kj_per_day: f32,
    /// Energy reserve capacity of an adult (kJ).
    pub reserve_max_kj: f32,
    /// Largest energy ingested per tick by an adult (kJ, gross).
    pub intake_kj_per_tick: f32,
    /// Hydration lost per tick (fraction of full).
    pub thirst_per_tick: f32,
    /// Ticks per day of the scale the traits were derived for.
    pub ticks_per_day: f32,
    /// Sprint speed (m/s).
    pub sprint_m_s: f32,
    /// Sprint speed (cells per tick).
    pub sprint_cells: f32,
    /// Cruising speed (cells per tick).
    pub cruise_cells: f32,
    /// Acceleration limit (cells per tick²).
    pub accel_cells: f32,
    /// Ticks of sprinting before exhaustion.
    pub endurance_ticks: f32,
    /// Vision radius (cells).
    pub vision_cells: f32,
    /// Distance (cells) within which the animal remembers drinkable water
    /// (its home range around water): three vision radii (at least 12
    /// cells), or the distance it travels in three hours at cruising speed
    /// if that is farther.
    pub water_memory_cells: f32,
    /// Hearing radius (cells).
    pub hearing_cells: f32,
    /// Smell (chemoreception) radius (cells).
    pub smell_cells: f32,
    /// Field of view (radians, full aperture).
    pub fov_rad: f32,
    /// Distance from which an attack is resolved (cells): half a tick of sprinting, at least 0.3 cells.
    pub strike_cells: f32,
    /// Lifespan (ticks).
    pub lifespan_ticks: f64,
    /// Age at maturity (ticks).
    pub maturity_ticks: f64,
    /// Gestation or egg formation (ticks).
    pub gestation_ticks: f64,
    /// Incubation of eggs (ticks; 0 for live birth).
    pub incubation_ticks: f64,
    /// Minimum interval between births (ticks).
    pub interval_ticks: f64,
    /// Parental care (ticks).
    pub care_ticks: f64,
    /// Birth mass per offspring (kg).
    pub birth_mass_kg: f32,
    /// Gompertz baseline hazard per tick.
    pub gompertz_h0: f64,
    /// Gompertz ageing rate per tick.
    pub gompertz_gamma: f64,
    /// Net cost of transport of an adult walking or running (kJ per cell
    /// travelled); flying and swimming cost [`crate::units::FLYING_COT_FRACTION`]
    /// and [`crate::units::SWIMMING_COT_FRACTION`] of it.
    pub cot_kj_per_cell: f32,
    /// Can walk on land.
    pub walks: bool,
    /// Can swim.
    pub swims: bool,
    /// Can fly.
    pub flies: bool,
    /// Lives in water only.
    pub aquatic: bool,
    /// Uses a burrow as refuge.
    pub burrows: bool,
    /// Deepest wadeable water (m).
    pub wade_depth_m: f32,
    /// Steepest walkable slope.
    pub max_slope: f32,
    /// Territory radius (cells); 0 for non-territorial species.
    pub territory_cells: f32,
    /// Water an aquatic species lives in.
    pub water: WaterKind,
    /// Shallowest water (m) an aquatic species can live in.
    pub min_depth_m: f32,
    /// Cruising flight altitude (m); 0 for non-fliers.
    pub flight_altitude_m: f32,
    /// Distance (cells) at which migrants sample the climate.
    pub migration_cells: f32,
}

impl AnimalTraits {
    /// Derives the traits of `species` for the given scale.
    pub fn derive(species: &Species, scale: &TimeScale) -> Self {
        let mass = f64::from(species.body.mass_kg);
        let endotherm = species.body.thermoregulation == Thermoregulation::Endotherm;
        let body_t = if endotherm { species.body.body_temperature_c } else { 20.0 };
        let bmr_kj_day = basal_metabolic_w(mass, endotherm, f64::from(body_t)) * SECONDS_PER_DAY / 1000.0;
        let field = bmr_kj_day * f64::from(species.metabolism.field_multiplier);
        let c_th = if endotherm { 1.0 } else { 0.16 };
        let fasting = species.metabolism.fasting_days.map_or(5.0 * mass.powf(0.25) / c_th, f64::from);
        let ticks_per_hour = f64::from(scale.ticks_per_day) / 24.0;
        let intake = field * f64::from(species.metabolism.intake_per_hour) / ticks_per_hour.max(1e-9);

        let flies = species.can(Locomotion::Fly);
        let swims = species.can(Locomotion::Swim);
        let walks = species.can(Locomotion::Walk) || species.can(Locomotion::Run);
        let aquatic = species.habitat.aquatic;
        let sprint = species.movement.sprint_speed_m_s.map_or_else(
            || {
                let model = if flies {
                    SpeedModel::Flying
                } else if aquatic {
                    SpeedModel::Swimming
                } else {
                    SpeedModel::Running
                };
                let v = hirt_max_speed_m_s(model, mass);
                // Walkers that cannot run reach 40 % of the running model.
                if model == SpeedModel::Running && !species.can(Locomotion::Run) { 0.4 * v } else { v }
            },
            f64::from,
        ) as f32;
        let sprint_cells = scale.cells_per_tick(sprint);
        let cruise_cells = sprint_cells * species.movement.cruise_fraction;
        let endurance_ticks = (species.movement.endurance_s / scale.motion_seconds.max(1e-3)).max(0.5);

        let lifespan_ticks = scale.life_ticks_from_years(species.life.lifespan_years).max(1.0);
        let gamma = std::f64::consts::LN_2 / (MRDT_FRACTION * lifespan_ticks);
        // S(L) = exp(−h0/γ (e^{γL} − 1)) = 1 % → h0 = γ ln(100) / (e^{γL} − 1).
        let h0 = gamma * (1.0 / LIFESPAN_SURVIVAL).ln() / ((gamma * lifespan_ticks).exp() - 1.0);
        let (gestation, incubation) = match species.reproduction.development {
            Development::Live { gestation_days } => (scale.life_ticks_from_days(gestation_days), 0.0),
            Development::Eggs { laying_days, incubation_days } => {
                (scale.life_ticks_from_days(laying_days), scale.life_ticks_from_days(incubation_days))
            }
        };
        let litter_mean = 0.5 * f64::from(species.reproduction.litter.0 + species.reproduction.litter.1);
        let birth_fraction = species
            .reproduction
            .birth_mass_fraction
            .map_or_else(|| 0.1 * mass.powf(-0.08) / litter_mean.max(1.0), f64::from)
            .clamp(1e-6, 0.5);
        let cot_kj_per_m = cost_of_transport_j_per_m(mass) / 1000.0;
        Self {
            adult_mass_kg: species.body.mass_kg,
            endotherm,
            body_temperature_c: body_t,
            bmr_kj_per_day: bmr_kj_day as f32,
            field_kj_per_day: field as f32,
            reserve_max_kj: (fasting * field) as f32,
            intake_kj_per_tick: intake as f32,
            thirst_per_tick: scale.dt_days() / species.metabolism.dehydration_days,
            ticks_per_day: scale.ticks_per_day as f32,
            sprint_m_s: sprint,
            sprint_cells,
            cruise_cells,
            accel_cells: sprint_cells * species.movement.agility,
            endurance_ticks,
            vision_cells: scale.cells(species.perception.vision_m),
            water_memory_cells: (3.0 * scale.cells(species.perception.vision_m).max(4.0))
                .max(cruise_cells * 3.0 * scale.ticks_per_day as f32 / 24.0),
            hearing_cells: scale.cells(species.perception.hearing_m),
            smell_cells: scale.cells(species.perception.smell_m),
            fov_rad: species.perception.fov_deg.to_radians(),
            strike_cells: (0.5 * sprint_cells).max(0.3),
            lifespan_ticks,
            maturity_ticks: scale.life_ticks_from_days(species.life.maturity_days),
            gestation_ticks: gestation.max(1.0),
            incubation_ticks: incubation,
            interval_ticks: scale.life_ticks_from_days(species.reproduction.interval_days),
            care_ticks: scale.life_ticks_from_days(species.reproduction.parental_care_days),
            birth_mass_kg: (mass * birth_fraction) as f32,
            gompertz_h0: h0,
            gompertz_gamma: gamma,
            cot_kj_per_cell: (cot_kj_per_m * f64::from(scale.cell_size_m)) as f32,
            walks,
            swims,
            flies,
            aquatic,
            burrows: species.can(Locomotion::Burrow) && species.habitat.refuges.contains(&Refuge::Burrow),
            wade_depth_m: species.habitat.max_wade_depth_m,
            max_slope: species.habitat.max_slope,
            territory_cells: if species.social.territorial {
                scale.cells(species.social.territory_radius_m)
            } else {
                0.0
            },
            water: species.habitat.water,
            min_depth_m: species.habitat.min_depth_m,
            flight_altitude_m: if flies { species.movement.flight_altitude_m } else { 0.0 },
            migration_cells: scale.cells(species.seasonal.migration_range_m),
        }
    }

    /// Basal metabolic rate (kJ per tick) of an animal of `mass_kg` whose
    /// body is at `body_t` (°C); endotherms use their constant body temperature.
    pub fn bmr_kj_per_tick(&self, mass_kg: f32, air_temperature_c: f32, scale: &TimeScale) -> f32 {
        let t = if self.endotherm { self.body_temperature_c } else { air_temperature_c };
        let w = basal_metabolic_w(f64::from(mass_kg), self.endotherm, f64::from(t));
        (w * SECONDS_PER_DAY / 1000.0) as f32 * scale.dt_days()
    }

    /// Gompertz hazard (probability of death from ageing) during one tick at
    /// `age_ticks`.
    pub fn ageing_hazard(&self, age_ticks: f64) -> f64 {
        let rate = self.gompertz_h0 * (self.gompertz_gamma * age_ticks).exp();
        1.0 - (-rate).exp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::SpeciesCatalog;

    fn scale() -> TimeScale {
        TimeScale { ticks_per_day: 240, days_per_year: 120, cell_size_m: 10.0, motion_seconds: 10.0 }
    }

    #[test]
    fn metabolism_follows_kleiber() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let rabbit = AnimalTraits::derive(catalog.animal("rabbit").unwrap(), &scale());
        let wolf = AnimalTraits::derive(catalog.animal("wolf").unwrap(), &scale());
        let ratio = f64::from(wolf.bmr_kj_per_day / rabbit.bmr_kj_per_day);
        let mass_ratio = f64::from(wolf.adult_mass_kg / rabbit.adult_mass_kg);
        // Same body temperature class: exactly (M_w / M_r)^{3/4} up to the
        // small body-temperature difference.
        let expected = mass_ratio.powf(0.75);
        assert!((ratio / expected - 1.0).abs() < 0.1, "ratio {ratio}, expected {expected}");
        // A 32 kg wolf: Kleiber ≈ 3.39 W × 32^0.75 ≈ 4,000 kJ/day.
        assert!((3000.0..5000.0).contains(&wolf.bmr_kj_per_day), "{}", wolf.bmr_kj_per_day);
        assert!(wolf.reserve_max_kj > wolf.field_kj_per_day);
    }

    #[test]
    fn gompertz_survival_to_lifespan_is_one_percent() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let t = AnimalTraits::derive(catalog.animal("deer").unwrap(), &scale());
        // Integrate the per-tick survival in coarse steps.
        let steps = 10_000;
        let dt = t.lifespan_ticks / steps as f64;
        let mut log_s = 0.0;
        for k in 0..steps {
            let age = (k as f64 + 0.5) * dt;
            log_s -= t.gompertz_h0 * (t.gompertz_gamma * age).exp() * dt;
        }
        assert!((log_s.exp() - 0.01).abs() < 0.002, "survival {}", log_s.exp());
    }

    #[test]
    fn durations_are_compressed_with_the_year() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let deer = catalog.animal("deer").unwrap();
        let t = AnimalTraits::derive(deer, &scale());
        let expected = f64::from(deer.life.maturity_days) * 120.0 / 365.0 * 240.0;
        assert!((t.maturity_ticks - expected).abs() < 1e-6);
        assert!(t.sprint_cells > t.cruise_cells && t.strike_cells >= 0.3);
    }
}
