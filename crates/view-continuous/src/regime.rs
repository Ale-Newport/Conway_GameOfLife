//! A coarse, honest description of what the world is doing, from the recent
//! metric history: empty, space-filling, a steady travelling or stationary
//! structure, growing, shrinking or fluctuating. It is a heuristic shown in
//! the overlay (with its criteria in the tooltip), not a measurement.

/// Number of recent ticks the classification looks at.
pub const WINDOW: usize = 100;
/// Fewer samples than this give [`Regime::Unknown`].
pub const MIN_SAMPLES: usize = 20;
/// Active fraction above which the world is [`Regime::SpaceFilling`].
pub const SPACE_FILLING: f64 = 0.15;

/// Dynamical regime of the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    /// Not enough history yet.
    Unknown,
    /// No matter left.
    Empty,
    /// More than 15 % of the cells are active (chaotic matter spread over
    /// the world; single creatures of the built-in scenarios cover 1–4 %,
    /// the Flow-Lenia membrane about 10 %, the Lenia soup about 23 %).
    SpaceFilling,
    /// Steady mass, moving centroid (gliders).
    Travelling,
    /// Steady mass, centroid at rest.
    Stationary,
    /// Mass rising by more than 10 % over the window.
    Growing,
    /// Mass falling by more than 10 % over the window.
    Shrinking,
    /// Mass varying without a trend.
    Fluctuating,
}

impl Regime {
    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Regime::Unknown => "Warming up",
            Regime::Empty => "Empty",
            Regime::SpaceFilling => "Space-filling",
            Regime::Travelling => "Travelling",
            Regime::Stationary => "Stationary",
            Regime::Growing => "Growing",
            Regime::Shrinking => "Shrinking",
            Regime::Fluctuating => "Fluctuating",
        }
    }
}

/// Criteria, for the tooltip.
pub const CRITERIA: &str = "Heuristic over the last 100 ticks: empty when the mass is below 0.001; space-filling when \
more than 15 % of the cells are active; growing or shrinking when the mass changed by more than 10 %; otherwise \
steady when the mass varies by less than 3 % (travelling if the centroid moves faster than 0.05 cells per tick, \
else stationary) and fluctuating when it varies more.";

/// Classifies the recent history: `mass` and `speed` are the last samples
/// (oldest first), `active_fraction` the latest active fraction.
pub fn classify(mass: &[f64], speed: &[f64], active_fraction: f64) -> Regime {
    let Some(&last) = mass.last() else { return Regime::Unknown };
    if last < 1e-3 {
        return Regime::Empty;
    }
    if active_fraction > SPACE_FILLING {
        return Regime::SpaceFilling;
    }
    if mass.len() < MIN_SAMPLES {
        return Regime::Unknown;
    }
    let first = mass[0];
    if first > 0.0 {
        let change = (last - first) / first;
        if change > 0.10 {
            return Regime::Growing;
        }
        if change < -0.10 {
            return Regime::Shrinking;
        }
    }
    let n = mass.len() as f64;
    let mean = mass.iter().sum::<f64>() / n;
    let var = mass.iter().map(|m| (m - mean).powi(2)).sum::<f64>() / n;
    if mean <= 0.0 || var.sqrt() / mean >= 0.03 {
        return Regime::Fluctuating;
    }
    let mean_speed = if speed.is_empty() { 0.0 } else { speed.iter().sum::<f64>() / speed.len() as f64 };
    if mean_speed > 0.05 { Regime::Travelling } else { Regime::Stationary }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regimes() {
        let steady = vec![73.0; 50];
        assert_eq!(classify(&steady, &[0.6; 50], 0.012), Regime::Travelling);
        assert_eq!(classify(&steady, &[0.01; 50], 0.012), Regime::Stationary);
        assert_eq!(classify(&steady[..5], &[0.6; 5], 0.012), Regime::Unknown);
        assert_eq!(classify(&[], &[], 0.0), Regime::Unknown);
        assert_eq!(classify(&[3.0, 0.0], &[], 0.0), Regime::Empty);
        assert_eq!(classify(&steady, &[0.6; 50], 0.3), Regime::SpaceFilling);
        // The Lenia soup's chaos (23 % active, jittering centroid) is not a glider.
        assert_eq!(classify(&[13_520.0; 50], &[4.2; 50], 0.226), Regime::SpaceFilling);
        // The Flow-Lenia membrane covers about 10 % of its world.
        assert_eq!(classify(&[804.0; 50], &[0.02; 50], 0.10), Regime::Stationary);
        let rising: Vec<f64> = (0..50).map(|i| 100.0 + i as f64).collect();
        assert_eq!(classify(&rising, &[], 0.05), Regime::Growing);
        let falling: Vec<f64> = (0..50).map(|i| 100.0 - i as f64).collect();
        assert_eq!(classify(&falling, &[], 0.05), Regime::Shrinking);
        let noisy: Vec<f64> = (0..51).map(|i| if i % 2 == 0 { 90.0 } else { 110.0 }).collect();
        assert_eq!(classify(&noisy, &[0.6; 50], 0.05), Regime::Fluctuating);
        assert_eq!(Regime::Travelling.label(), "Travelling");
    }
}
