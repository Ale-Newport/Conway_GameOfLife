//! Compressed simulation calendar: ticks → year / season / day / hour, the
//! seasonal and diurnal signals and a daylight factor (sun-elevation proxy).

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::config::WorldConfig;
use crate::error::WorldError;

/// Upper bound of [`Calendar::substeps_per_tick`].
pub const MAX_SUBSTEPS: u32 = 16;

/// The four seasons, in calendar order. A year starts with spring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Season {
    /// First quarter of the year (warming).
    Spring,
    /// Second quarter (warmest, longest days).
    Summer,
    /// Third quarter (cooling).
    Autumn,
    /// Fourth quarter (coldest, shortest days).
    Winter,
}

impl Season {
    /// All seasons in order.
    pub const ALL: [Season; 4] = [Self::Spring, Self::Summer, Self::Autumn, Self::Winter];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Spring => "Spring",
            Self::Summer => "Summer",
            Self::Autumn => "Autumn",
            Self::Winter => "Winter",
        }
    }
}

/// A point in calendar time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalendarTime {
    /// Tick this time belongs to.
    pub tick: u64,
    /// Year, starting at 0.
    pub year: u64,
    /// Season of the year.
    pub season: Season,
    /// Day within the season, starting at 0.
    pub day_of_season: u32,
    /// Day within the year, starting at 0.
    pub day_of_year: u32,
    /// Hour of the day, `0..24`.
    pub hour: u32,
    /// Minute of the hour, `0..60`.
    pub minute: u32,
    /// Fraction of the year elapsed, `[0, 1)`.
    pub year_fraction: f64,
}

/// Maps ticks to calendar time. One tick lasts `1440 / ticks_per_day`
/// minutes; a year has `4 × days_per_season` days.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calendar {
    ticks_per_day: u32,
    days_per_season: u32,
}

impl Calendar {
    /// Creates a calendar; both arguments must be at least 1.
    pub fn new(ticks_per_day: u32, days_per_season: u32) -> Result<Self, WorldError> {
        if ticks_per_day == 0 {
            return Err(WorldError::config("ticks_per_day", "must be at least 1"));
        }
        if days_per_season == 0 {
            return Err(WorldError::config("days_per_season", "must be at least 1"));
        }
        Ok(Self { ticks_per_day, days_per_season })
    }

    /// Calendar of a configuration (values below 1 are raised to 1; a
    /// validated configuration never has them).
    pub fn from_config(cfg: &WorldConfig) -> Self {
        Self { ticks_per_day: cfg.ticks_per_day.max(1), days_per_season: cfg.days_per_season.max(1) }
    }

    /// Ticks per day.
    pub fn ticks_per_day(&self) -> u32 {
        self.ticks_per_day
    }

    /// Days per season.
    pub fn days_per_season(&self) -> u32 {
        self.days_per_season
    }

    /// Days per year (four seasons).
    pub fn days_per_year(&self) -> u32 {
        4 * self.days_per_season
    }

    /// Ticks per year.
    pub fn ticks_per_year(&self) -> u64 {
        u64::from(self.ticks_per_day) * u64::from(self.days_per_year())
    }

    /// Simulated duration of one tick in hours.
    pub fn dt_hours(&self) -> f32 {
        24.0 / self.ticks_per_day as f32
    }

    /// Simulated duration of one tick in minutes.
    pub fn minutes_per_tick(&self) -> f64 {
        1440.0 / f64::from(self.ticks_per_day)
    }

    /// Number of sub-steps the surface-routing and fire automata run per
    /// tick: one per started 6 simulated minutes, `clamp(⌈minutes / 6⌉, 1,
    /// 16)`. Ticks longer than 96 minutes therefore use sub-steps longer
    /// than 6 minutes (water and fire still move at most one cell per
    /// sub-step).
    pub fn substeps_per_tick(&self) -> u32 {
        (self.minutes_per_tick() / 6.0).ceil().clamp(1.0, f64::from(MAX_SUBSTEPS)) as u32
    }

    /// Number of whole hours that have *started* strictly before tick time
    /// `tick` (i.e. `ceil(tick × 24 / ticks_per_day)`).
    fn hours_begun_before(&self, tick: u64) -> u64 {
        let tpd = u128::from(self.ticks_per_day);
        (u128::from(tick) * 24).div_ceil(tpd) as u64
    }

    /// Number of hours whose start falls inside tick `tick` (the interval
    /// `[tick, tick + 1)`): 1 or 0 for sub-hour ticks, more when a tick is
    /// longer than an hour. Hourly processes run this many times per tick, so
    /// they run exactly once per simulated hour whatever the tick length.
    pub fn hours_started(&self, tick: u64) -> u64 {
        self.hours_begun_before(tick + 1) - self.hours_begun_before(tick)
    }

    /// Hour of day (fractional) at the start of `tick`.
    pub fn hour_of_day(&self, tick: u64) -> f64 {
        let in_day = tick % u64::from(self.ticks_per_day);
        in_day as f64 * 24.0 / f64::from(self.ticks_per_day)
    }

    /// Fraction of the year elapsed at the start of `tick`.
    pub fn year_fraction(&self, tick: u64) -> f64 {
        let in_year = tick % self.ticks_per_year();
        in_year as f64 / self.ticks_per_year() as f64
    }

    /// Calendar breakdown of `tick`.
    pub fn at(&self, tick: u64) -> CalendarTime {
        let tpd = u64::from(self.ticks_per_day);
        let day_index = tick / tpd;
        let days_per_year = u64::from(self.days_per_year());
        let year = day_index / days_per_year;
        let day_of_year = (day_index % days_per_year) as u32;
        let season = Season::ALL[(day_of_year / self.days_per_season) as usize];
        let minutes = (tick % tpd) * 1440 / tpd;
        CalendarTime {
            tick,
            year,
            season,
            day_of_season: day_of_year % self.days_per_season,
            day_of_year,
            hour: (minutes / 60) as u32,
            minute: (minutes % 60) as u32,
            year_fraction: self.year_fraction(tick),
        }
    }

    /// Seasonal signal in `[-1, 1]`: `sin(2π(f − 1/8))` where `f` is the
    /// year fraction, so it is 0 in mid-spring, +1 in mid-summer, 0 in
    /// mid-autumn and −1 in mid-winter.
    pub fn seasonal_signal(&self, tick: u64) -> f32 {
        (TAU * (self.year_fraction(tick) - 0.125)).sin() as f32
    }

    /// Diurnal signal in `[-1, 1]`: `cos(2π(h − 15)/24)`, maximal at 15:00
    /// (afternoon temperature peak) and minimal at 03:00.
    pub fn diurnal_signal(&self, tick: u64) -> f32 {
        (TAU * (self.hour_of_day(tick) - 15.0) / 24.0).cos() as f32
    }

    /// Day length in hours: `12 + amplitude × seasonal_signal`, clamped to
    /// `[0.5, 23.5]`.
    pub fn day_length_hours(&self, tick: u64, amplitude_h: f32) -> f32 {
        (12.0 + amplitude_h * self.seasonal_signal(tick)).clamp(0.5, 23.5)
    }

    /// Daylight factor in `[0, 1]` (sun-elevation proxy): zero at night,
    /// `peak × sin(π (h − sunrise) / L)` during the day, where the day of
    /// length `L` is centred on noon and the noon peak
    /// `0.75 + 0.25 × seasonal_signal` is higher in summer.
    pub fn daylight(&self, tick: u64, amplitude_h: f32) -> f32 {
        let length = f64::from(self.day_length_hours(tick, amplitude_h));
        let sunrise = 12.0 - length / 2.0;
        let hour = self.hour_of_day(tick);
        if hour < sunrise || hour > sunrise + length {
            return 0.0;
        }
        let peak = 0.75 + 0.25 * f64::from(self.seasonal_signal(tick));
        (peak * (std::f64::consts::PI * (hour - sunrise) / length).sin()).clamp(0.0, 1.0) as f32
    }
}

/// Day-length amplitude (hours) implied by a seasonal temperature amplitude:
/// stronger seasons stand for higher latitudes and longer summer days.
pub fn day_length_amplitude(seasonality_c: f32) -> f32 {
    (seasonality_c / 4.0).clamp(0.0, 6.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakdown_matches_compressed_calendar() {
        let cal = Calendar::new(240, 30).unwrap();
        let t = cal.at(0);
        assert_eq!((t.year, t.season, t.day_of_year, t.hour, t.minute), (0, Season::Spring, 0, 0, 0));
        // 6-minute ticks: tick 125 is 12:30 on day 0.
        let t = cal.at(125);
        assert_eq!((t.hour, t.minute), (12, 30));
        // Day 30 starts summer; day 120 starts year 1.
        assert_eq!(cal.at(30 * 240).season, Season::Summer);
        assert_eq!(cal.at(95 * 240).season, Season::Winter);
        let t = cal.at(120 * 240 + 10);
        assert_eq!((t.year, t.season, t.day_of_year, t.minute), (1, Season::Spring, 0, 0));
        assert!((cal.dt_hours() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn seasonal_and_diurnal_signals_have_the_right_phase() {
        let cal = Calendar::new(24, 10).unwrap();
        let mid_summer = 15 * 24; // day 15 of 40 = 0.375 of the year
        let mid_winter = 35 * 24;
        assert!((cal.seasonal_signal(mid_summer) - 1.0).abs() < 1e-6);
        assert!((cal.seasonal_signal(mid_winter) + 1.0).abs() < 1e-6);
        assert!((cal.diurnal_signal(15) - 1.0).abs() < 1e-6);
        assert!((cal.diurnal_signal(3) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn daylight_is_zero_at_night_and_longer_in_summer() {
        let cal = Calendar::new(240, 30).unwrap();
        assert_eq!(cal.daylight(0, 3.0), 0.0); // midnight
        // Noon in mid-spring (seasonal signal 0): peak 0.75.
        assert!((cal.daylight(15 * 240 + 120, 3.0) - 0.75).abs() < 0.01);
        let summer_day = 45 * 240;
        let winter_day = 105 * 240;
        assert!(cal.day_length_hours(summer_day, 3.0) > cal.day_length_hours(winter_day, 3.0));
        // 06:30 is lit in summer (sunrise ≈ 4:30) but dark in winter (≈ 7:30).
        assert!(cal.daylight(summer_day + 65, 3.0) > 0.0);
        assert_eq!(cal.daylight(winter_day + 65, 3.0), 0.0);
    }

    #[test]
    fn hourly_boundaries_are_counted_once() {
        let cal = Calendar::new(240, 30).unwrap();
        let total: u64 = (0..2400).map(|t| cal.hours_started(t)).sum();
        assert_eq!(total, 240); // ten days
        let coarse = Calendar::new(4, 30).unwrap(); // six-hour ticks
        assert_eq!(coarse.hours_started(1), 6);
        assert!(Calendar::new(0, 1).is_err());
    }

    #[test]
    fn substeps_cover_each_started_six_minutes_up_to_the_cap() {
        let substeps = |tpd| Calendar::new(tpd, 30).unwrap().substeps_per_tick();
        assert_eq!(substeps(240), 1); // 6 min
        assert_eq!(substeps(480), 1); // 3 min
        assert_eq!(substeps(100), 3); // 14.4 min
        assert_eq!(substeps(24), 10); // 1 h
        assert_eq!(substeps(1), MAX_SUBSTEPS); // 1 day
    }
}
