//! The per-tick update: weather, atmosphere, hydrology, drought, floods,
//! fire and landslides, in that order.

use rayon::prelude::*;
use simulation_core::{EventSeverity, SimEvent};

use crate::biome::SurfaceKind;
use crate::calendar::day_length_amplitude;
use crate::climate::{Drivers, cell_humidity, cell_light, cell_temperature};
use crate::report::EnvironmentReport;
use crate::weather::{CloudField, WeatherStep};
use crate::world::World;

impl World {
    /// Advances the world by one tick and reports what happened.
    ///
    /// Order: (1) effects of interventions since the last step are moved
    /// into the report; (2) ecosystem inputs (fuel, canopy) are sanitised;
    /// (3) weather (one Markov transition per started hour, wind, cloud
    /// advection); (4) temperature, humidity, light and precipitation
    /// fields; (5) hydrology (columns, groundwater/baseflow, routing);
    /// (6) drought index and flood count; (7) fire (passive states,
    /// lightning, spread sub-steps); (8) hourly landslide checks; (9) terrain
    /// refresh when elevation changed. The tick counter then increments.
    pub fn step(&mut self) -> EnvironmentReport {
        let tick = self.tick;
        let pending = std::mem::take(&mut self.pending);
        let mut report = EnvironmentReport {
            tick,
            events: pending.events,
            wind_damage: pending.wind_damage,
            fuel_consumed: pending.fuel_consumed,
            ignitions: pending.ignitions,
            landslides: pending.landslides,
            terrain_changed: pending.terrain_changed,
            ..EnvironmentReport::default()
        };
        self.sanitize_inputs();

        let hours = self.calendar.hours_started(tick);
        let weather_step = WeatherStep {
            cfg: &self.config,
            tick,
            hours_started: hours,
            dt_hours: self.calendar.dt_hours(),
            season: self.calendar.seasonal_signal(tick),
        };
        if let Some(event) = self.weather.advance(&weather_step, &mut self.rngs.weather) {
            report.events.push(event);
        }
        self.update_atmosphere();

        let pet = self.potential_evaporation_factor(self.daylight);
        let (rain, evaporated, outflow) = self.hydrology_step(pet);
        report.rain_volume_m3 = rain;
        report.evaporated_m3 = evaporated;
        report.outflow_m3 = outflow;
        let non_ocean = self.surface.as_slice().iter().filter(|s| **s != SurfaceKind::Ocean).count();
        if non_ocean > 0 {
            report.rain_total_mm = rain / (non_ocean as f64 * self.config.cell_area_m2()) * 1000.0;
        }
        let dt_hours = f64::from(self.calendar.dt_hours());
        let expected = self.expected_rain_mm_h();
        let moisture = self.mean_land_moisture();
        self.drought.update(report.rain_total_mm / dt_hours, expected, moisture, dt_hours);

        let flooded = self.count_flooded();
        if flooded > 0 && self.flooded == 0 {
            report.events.push(
                SimEvent::new(tick, "flood", format!("Flooding: {flooded} cells under water"))
                    .severity(EventSeverity::Info),
            );
        }
        self.flooded = flooded;
        report.flooded_cells = flooded;

        self.fire_step(&mut report);

        let mut slid = 0;
        for _ in 0..hours {
            slid += self.natural_landslides(tick, &mut report);
        }
        if slid > 0 {
            report.landslides += slid;
            report.terrain_changed = true;
            self.refresh_terrain();
        }
        report.normalize_fuel();
        self.tick += 1;
        report
    }

    /// Clamps ecosystem-written layers: fuel to finite non-negative values,
    /// canopy to `[0, 1]` (non-finite values become 0).
    fn sanitize_inputs(&mut self) {
        for f in self.fuel.as_mut_slice() {
            if !f.is_finite() || *f < 0.0 {
                *f = 0.0;
            }
        }
        for c in self.canopy.as_mut_slice() {
            *c = if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 };
        }
    }

    /// Recomputes temperature, humidity, light and precipitation for the
    /// current tick from the static climate, the calendar and the weather.
    /// Local cloudiness is `cover(state) × min(1, m)` and precipitation
    /// `rate(state) × P(x, y) × m`, where `m` is the advected cloud field.
    pub(crate) fn update_atmosphere(&mut self) {
        let tick = self.tick;
        let daylight = self.calendar.daylight(tick, day_length_amplitude(self.config.seasonality));
        self.daylight = daylight;
        let drivers = Drivers {
            season: self.calendar.seasonal_signal(tick),
            diurnal: self.calendar.diurnal_signal(tick),
            seasonality: self.config.seasonality,
            diurnal_half_range: self.config.diurnal_range_c / 2.0,
            weather_offset: self.weather.temperature_offset(),
            humidity_bonus: self.weather.humidity_bonus(),
            daylight,
            transmission: self.weather.cloud_transmission(),
            cover: self.weather.cloud_cover(),
        };
        let rate = self.weather.precipitation_rate_mm_h();
        let hours = tick as f64 * f64::from(self.calendar.dt_hours());
        let clouds = (drivers.cover > 0.0).then(|| CloudField::compute(&self.weather, &self.config, hours));
        let w = self.config.width;
        let mean_t = self.climate.mean_temperature.as_slice();
        let continental = self.climate.continental.as_slice();
        let humidity_base = self.climate.humidity_base.as_slice();
        let precip_factor = self.climate.precip_factor.as_slice();
        let water = self.water_depth.as_slice();
        let canopy = self.canopy.as_slice();
        let surface = self.surface.as_slice();
        self.temperature
            .as_mut_slice()
            .par_chunks_mut(w)
            .zip(self.humidity.as_mut_slice().par_chunks_mut(w))
            .zip(self.light.as_mut_slice().par_chunks_mut(w))
            .zip(self.precipitation.as_mut_slice().par_chunks_mut(w))
            .enumerate()
            .for_each(|(y, (((temperature, humidity), light), precipitation))| {
                for x in 0..w {
                    let i = y * w + x;
                    let m = clouds.as_ref().map_or(0.0, |c| c.sample(x, y));
                    let cloudiness = drivers.cover * m.min(1.0);
                    let t = cell_temperature(mean_t[i], continental[i], cloudiness, &drivers);
                    let wet = if surface[i] == SurfaceKind::Land { (water[i] / 0.05).min(1.0) } else { 1.0 };
                    temperature[x] = t;
                    humidity[x] = cell_humidity(humidity_base[i], cloudiness, wet, t, &drivers);
                    light[x] = cell_light(cloudiness, canopy[i], &drivers);
                    precipitation[x] = rate * precip_factor[i] * m;
                }
            });
        let mean = self.mean_land_temperature();
        self.weather.set_land_mean_temperature(mean);
    }
}
