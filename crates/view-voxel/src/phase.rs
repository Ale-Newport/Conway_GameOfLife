//! "Phase" of the run, read from the recorded metrics: extinct, static,
//! periodic population (oscillator or ship candidate, with the centroid
//! displacement per period), growing, shrinking or fluctuating.
//!
//! Only consecutive generations are used (the runner records metrics every
//! generation). The classification is about the **metrics**: a periodic
//! population sequence does not prove that the configuration repeats; the
//! label says "candidate" for that reason. "Static" is exact: with no
//! births, no deaths and no dying voxels, no voxel changes state.

use bevy::prelude::*;
use interface::MetricsHistory;

/// One generation of metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Generation.
    pub tick: u64,
    /// Non-empty voxels.
    pub population: f64,
    /// Voxels in state 1.
    pub alive: f64,
    /// Births in the last generation.
    pub births: f64,
    /// Deaths in the last generation.
    pub deaths: f64,
    /// Centroid of the non-empty voxels (absent when empty).
    pub centroid: Option<[f64; 3]>,
}

/// Classification of the recent run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Not enough consecutive generations recorded yet.
    Unknown,
    /// No voxel left.
    Extinct,
    /// Nothing changed for this many generations (exact still life).
    Static {
        /// Generations without change.
        generations: u64,
    },
    /// Population, births and deaths repeat with this period.
    Periodic {
        /// Smallest period of the counts.
        period: u64,
        /// Centroid displacement and the number of generations it takes
        /// (a multiple of `period`), when `c(t) − c(t − k)` was the same,
        /// non-zero vector for every generation of the last `2k`.
        drift: Option<([f64; 3], u64)>,
    },
    /// Mean population growing by this fraction per generation.
    Growing {
        /// Relative growth per generation.
        rate: f64,
    },
    /// Mean population shrinking by this fraction per generation.
    Shrinking {
        /// Relative decline per generation (positive).
        rate: f64,
    },
    /// Changing without a clear trend.
    Fluctuating,
}

impl Phase {
    /// Short label.
    pub fn label(&self) -> String {
        match *self {
            Phase::Unknown => "Collecting data…".to_string(),
            Phase::Extinct => "Extinct".to_string(),
            Phase::Static { generations } => format!("Static for {generations} generations (still life)"),
            Phase::Periodic { period: 1, drift: None } => "Constant counts every generation".to_string(),
            Phase::Periodic { period, drift: None } => {
                format!("Counts repeat every {period} generations (oscillator candidate)")
            }
            Phase::Periodic { drift: Some((d, k)), .. } => format!(
                "Moves ({}, {}, {}) every {k} generations (ship candidate)",
                fmt_step(d[0]),
                fmt_step(d[1]),
                fmt_step(d[2])
            ),
            Phase::Growing { rate } => format!("Growing ({:+.1}% per generation)", rate * 100.0),
            Phase::Shrinking { rate } => format!("Shrinking ({:+.1}% per generation)", -rate * 100.0),
            Phase::Fluctuating => "Fluctuating (no clear trend)".to_string(),
        }
    }
}

fn fmt_step(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{v:.2}") }
}

/// Longest period searched.
pub const MAX_PERIOD: usize = 30;
/// Generations compared on each side for the trend.
pub const TREND_WINDOW: usize = 10;

/// Classifies the samples (oldest first).
pub fn analyze(samples: &[Sample]) -> Phase {
    // Keep the trailing run of consecutive generations.
    let mut start = samples.len();
    while start > 0 && (start == samples.len() || samples[start - 1].tick + 1 == samples[start].tick) {
        start -= 1;
    }
    let s = &samples[start..];
    let Some(last) = s.last() else { return Phase::Unknown };
    if last.population == 0.0 {
        return Phase::Extinct;
    }
    let unchanged = s
        .iter()
        .rev()
        .take_while(|x| {
            x.births == 0.0 && x.deaths == 0.0 && x.alive == x.population && x.population == last.population
        })
        .count();
    if unchanged >= 2 {
        return Phase::Static { generations: unchanged as u64 };
    }
    for period in 1..=MAX_PERIOD {
        let needed = (3 * period).max(8);
        if s.len() < needed {
            break;
        }
        let tail = &s[s.len() - needed..];
        let repeats = (period..tail.len()).all(|i| {
            let (a, b) = (tail[i], tail[i - period]);
            a.population == b.population && a.births == b.births && a.deaths == b.deaths
        });
        if repeats {
            let drift = (period..=MAX_PERIOD)
                .step_by(period)
                .take_while(|k| s.len() >= 3 * k)
                .find_map(|k| translation(s, k).map(|d| (d, k as u64)));
            return Phase::Periodic { period: period as u64, drift };
        }
    }
    if s.len() < 2 * TREND_WINDOW {
        return Phase::Unknown;
    }
    let mean = |w: &[Sample]| w.iter().map(|x| x.population).sum::<f64>() / w.len() as f64;
    let recent = mean(&s[s.len() - TREND_WINDOW..]);
    let before = mean(&s[s.len() - 2 * TREND_WINDOW..s.len() - TREND_WINDOW]);
    if before <= 0.0 {
        return Phase::Growing { rate: 1.0 };
    }
    let rate = (recent / before).powf(1.0 / TREND_WINDOW as f64) - 1.0;
    if rate > 0.002 {
        Phase::Growing { rate }
    } else if rate < -0.002 {
        Phase::Shrinking { rate: -rate }
    } else {
        Phase::Fluctuating
    }
}

/// The centroid displacement over `k` generations when it is the same
/// non-zero vector for each of the last `2k` generations (needs `3k` samples).
fn translation(s: &[Sample], k: usize) -> Option<[f64; 3]> {
    let n = s.len();
    let step = |i: usize| -> Option<[f64; 3]> {
        let (a, b) = (s[i].centroid?, s[i - k].centroid?);
        Some([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
    };
    let d = step(n - 1)?;
    if d.iter().all(|v| v.abs() < 1e-6) {
        return None;
    }
    for i in n - 2 * k..n {
        let e = step(i)?;
        if (0..3).any(|a| (e[a] - d[a]).abs() > 1e-6) {
            return None;
        }
    }
    Some(d)
}

/// The latest samples (up to `max`) from the recorded metrics.
pub fn recent_samples(history: &MetricsHistory, max: usize) -> Vec<Sample> {
    let store = history.store();
    let Some(population) = store.get("population") else { return Vec::new() };
    let points = population.points();
    let from = points.len().saturating_sub(max);
    let value = |key: &str, tick: u64| store.get(key).and_then(|s| s.value_at(tick));
    points[from..]
        .iter()
        .map(|p| {
            let centroid = match (value("centroid_x", p.tick), value("centroid_y", p.tick), value("centroid_z", p.tick))
            {
                (Some(x), Some(y), Some(z)) => Some([x, y, z]),
                _ => None,
            };
            Sample {
                tick: p.tick,
                population: p.value,
                alive: value("state_1", p.tick).unwrap_or(p.value),
                births: value("births", p.tick).unwrap_or(0.0),
                deaths: value("deaths", p.tick).unwrap_or(0.0),
                centroid,
            }
        })
        .collect()
}

/// Cached phase of the run.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct PhaseInfo {
    /// Classification.
    pub phase: Phase,
    last_tick: Option<u64>,
}

impl Default for PhaseInfo {
    fn default() -> Self {
        Self { phase: Phase::Unknown, last_tick: None }
    }
}

/// Re-classifies when a new generation was recorded.
pub(crate) fn update_phase(history: Res<MetricsHistory>, mut info: ResMut<PhaseInfo>) {
    let last = history.store().last_tick();
    if last == info.last_tick {
        return;
    }
    info.last_tick = last;
    info.phase = analyze(&recent_samples(&history, 4 * MAX_PERIOD + 2 * TREND_WINDOW));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tick: u64, population: f64, births: f64, deaths: f64) -> Sample {
        Sample { tick, population, alive: population, births, deaths, centroid: Some([0.0; 3]) }
    }

    #[test]
    fn extinct_and_unknown() {
        assert_eq!(analyze(&[]), Phase::Unknown);
        assert_eq!(analyze(&[sample(5, 0.0, 0.0, 3.0)]), Phase::Extinct);
        assert_eq!(analyze(&[sample(1, 10.0, 1.0, 2.0)]), Phase::Unknown);
    }

    #[test]
    fn static_needs_no_change_and_no_dying_voxels() {
        let mut s: Vec<Sample> = (1..=5).map(|t| sample(t, 35.0, 0.0, 0.0)).collect();
        assert_eq!(analyze(&s), Phase::Static { generations: 5 });
        // Dying voxels keep decaying even without births or deaths.
        for x in &mut s {
            x.alive = 30.0;
        }
        assert_ne!(analyze(&s), Phase::Static { generations: 5 });
    }

    #[test]
    fn glider_is_periodic_with_drift() {
        // A period-4 ship moving (1, 1, 0) per period with a population cycle.
        let pops = [10.0, 10.0, 10.0, 10.0];
        let births = [3.0, 2.0, 3.0, 2.0];
        let samples: Vec<Sample> = (0..24)
            .map(|t| {
                let phase = t % 4;
                let shift = (t / 4) as f64;
                Sample {
                    tick: t as u64 + 1,
                    population: pops[phase],
                    alive: pops[phase],
                    births: births[phase],
                    deaths: births[phase],
                    centroid: Some([shift + phase as f64 * 0.1, shift, 4.0]),
                }
            })
            .collect();
        match analyze(&samples) {
            Phase::Periodic { period: 2, drift } => assert_eq!(drift, Some(([1.0, 1.0, 0.0], 4))),
            other => panic!("{other:?}"),
        }
        // With a pure period-4 signature the drift is (1, 1, 0).
        let births4 = [3.0, 2.0, 1.0, 2.0];
        let samples: Vec<Sample> = (0..24)
            .map(|t| {
                let phase = t % 4;
                let shift = (t / 4) as f64;
                Sample {
                    tick: t as u64 + 1,
                    population: 10.0,
                    alive: 10.0,
                    births: births4[phase],
                    deaths: births4[phase],
                    centroid: Some([shift + phase as f64 * 0.25, shift, 4.0]),
                }
            })
            .collect();
        assert_eq!(analyze(&samples), Phase::Periodic { period: 4, drift: Some(([1.0, 1.0, 0.0], 4)) });
        assert!(Phase::Periodic { period: 4, drift: Some(([1.0, 1.0, 0.0], 4)) }.label().contains("(1, 1, 0) every 4"));
        // Constant counts (period 1) whose shape repeats every 4 generations:
        // the displacement is found at the first consistent multiple.
        let samples: Vec<Sample> = (0..24)
            .map(|t| Sample {
                tick: t as u64 + 1,
                population: 10.0,
                alive: 10.0,
                births: 4.0,
                deaths: 4.0,
                centroid: Some([(t / 4) as f64 + [0.0, 0.3, 0.1, 0.6][t % 4], 0.0, 1.0]),
            })
            .collect();
        assert_eq!(analyze(&samples), Phase::Periodic { period: 1, drift: Some(([1.0, 0.0, 0.0], 4)) });
    }

    #[test]
    fn trends_and_gaps() {
        let growing: Vec<Sample> = (1..=30).map(|t| sample(t, 100.0 * 1.05f64.powi(t as i32), t as f64, 0.0)).collect();
        match analyze(&growing) {
            Phase::Growing { rate } => assert!((rate - 0.05).abs() < 1e-6, "{rate}"),
            other => panic!("{other:?}"),
        }
        let shrinking: Vec<Sample> =
            (1..=30).map(|t| sample(t, 1000.0 * 0.9f64.powi(t as i32), 0.0, t as f64)).collect();
        assert!(matches!(analyze(&shrinking), Phase::Shrinking { rate } if (rate - 0.1).abs() < 1e-6));
        // A gap in the ticks (reset or history jump) restarts the analysis.
        let mut gapped = growing.clone();
        gapped.push(sample(100, 50.0, 1.0, 1.0));
        assert_eq!(analyze(&gapped), Phase::Unknown);
        let noisy: Vec<Sample> = (1..=30)
            .map(|t| sample(t, 500.0 + if t % 3 == 0 { 7.0 } else { -3.0 } + (t % 7) as f64, 9.0, 9.0))
            .collect();
        assert!(matches!(analyze(&noisy), Phase::Fluctuating | Phase::Periodic { .. }));
    }
}
