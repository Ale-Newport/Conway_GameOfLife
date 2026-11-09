//! Needs and drives (hunger, thirst, fatigue...).
//!
//! A [`Drive`] is a deprivation level in `[0, 1]` (0 = fully satisfied, 1 =
//! maximal need) that grows linearly at `rate` per unit of time and becomes
//! *urgent* at `threshold`. The value is designed to be fed directly into a
//! utility [`crate::utility::Consideration`].
//!
//! ```text
//! update(dt):      value ← clamp(value + rate·dt, 0, 1)
//! satisfy(amount): value ← max(value − amount, 0)
//! urgency:         0 below threshold, (value − threshold) / (1 − threshold) above
//! ```
//!
//! A negative `rate` models a need that recovers on its own (e.g. fatigue
//! while resting). Models convert physical quantities (kJ eaten, hours slept)
//! into drive units themselves.

use serde::{Deserialize, Serialize};

use crate::error::OrganismError;

/// One need of an agent.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drive {
    /// Current deprivation in `[0, 1]`.
    pub value: f32,
    /// Change per unit of time (usually per tick).
    pub rate: f32,
    /// Value at which the drive becomes urgent, in `[0, 1]`.
    pub threshold: f32,
}

impl Drive {
    /// Fully satisfied drive. `rate` must be finite and `threshold` a number
    /// in `[0, 1]`.
    pub fn new(rate: f32, threshold: f32) -> Result<Self, OrganismError> {
        let drive = Self { value: 0.0, rate, threshold };
        drive.validate()?;
        Ok(drive)
    }

    /// Sets the starting value (clamped to `[0, 1]`, NaN becomes 0).
    pub fn with_value(mut self, value: f32) -> Self {
        self.value = clamp_unit(value);
        self
    }

    /// Checks a drive loaded from data.
    pub fn validate(&self) -> Result<(), OrganismError> {
        if !self.rate.is_finite() {
            return Err(OrganismError::InvalidDrive(format!("rate {} must be finite", self.rate)));
        }
        if !(0.0..=1.0).contains(&self.threshold) {
            return Err(OrganismError::InvalidDrive(format!("threshold {} must be in [0, 1]", self.threshold)));
        }
        if !(0.0..=1.0).contains(&self.value) {
            return Err(OrganismError::InvalidDrive(format!("value {} must be in [0, 1]", self.value)));
        }
        Ok(())
    }

    /// Advances the drive by `dt` time units (non-finite or negative `dt` is
    /// ignored).
    pub fn update(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.value = clamp_unit(self.value + self.rate * dt);
        }
    }

    /// Reduces the deprivation by `amount` (≥ 0) and returns the reduction
    /// actually achieved (smaller when the drive bottoms out at 0).
    pub fn satisfy(&mut self, amount: f32) -> f32 {
        if amount.is_nan() || amount <= 0.0 {
            return 0.0;
        }
        let before = self.value;
        self.value = clamp_unit(self.value - amount);
        before - self.value
    }

    /// Whether the drive is at or above its threshold.
    pub fn is_urgent(&self) -> bool {
        self.value >= self.threshold
    }

    /// Whether the drive is maxed out (starvation, exhaustion...).
    pub fn is_saturated(&self) -> bool {
        self.value >= 1.0
    }

    /// 0 below the threshold, rising linearly to 1 at a value of 1 (1 for any
    /// value at or above a threshold of 1).
    pub fn urgency(&self) -> f32 {
        if self.value < self.threshold {
            0.0
        } else if self.threshold >= 1.0 {
            1.0
        } else {
            clamp_unit((self.value - self.threshold) / (1.0 - self.threshold))
        }
    }

    /// Time until the threshold is reached at the current rate: `Some(0)` if
    /// already urgent, `None` if the drive is not growing.
    pub fn time_until_urgent(&self) -> Option<f32> {
        if self.is_urgent() {
            Some(0.0)
        } else if self.rate > 0.0 {
            Some((self.threshold - self.value) / self.rate)
        } else {
            None
        }
    }
}

fn clamp_unit(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// Advances every drive by `dt`.
pub fn update_all(drives: &mut [Drive], dt: f32) {
    for drive in drives {
        drive.update(dt);
    }
}

/// Index of the most urgent drive among those at or above their threshold
/// (largest [`Drive::urgency`], then largest value, ties to the lowest index),
/// or `None` when no drive is urgent.
pub fn most_urgent(drives: &[Drive]) -> Option<usize> {
    let mut best: Option<(usize, f32, f32)> = None;
    for (i, d) in drives.iter().enumerate().filter(|(_, d)| d.is_urgent()) {
        let key = (d.urgency(), d.value);
        if best.is_none_or(|(_, u, v)| key.0 > u || (key.0 == u && key.1 > v)) {
            best = Some((i, key.0, key.1));
        }
    }
    best.map(|(i, _, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_saturates_and_is_satisfied() {
        let mut hunger = Drive::new(0.1, 0.6).unwrap();
        for _ in 0..5 {
            hunger.update(1.0);
        }
        assert!((hunger.value - 0.5).abs() < 1e-6);
        assert!(!hunger.is_urgent());
        assert!((hunger.time_until_urgent().unwrap() - 1.0).abs() < 1e-5);
        hunger.update(2.0);
        assert!(hunger.is_urgent());
        assert!((hunger.urgency() - 0.25).abs() < 1e-5, "{}", hunger.urgency());
        for _ in 0..10 {
            hunger.update(1.0);
        }
        assert_eq!(hunger.value, 1.0);
        assert!(hunger.is_saturated());
        assert_eq!(hunger.urgency(), 1.0);
        assert!((hunger.satisfy(0.3) - 0.3).abs() < 1e-6);
        assert!((hunger.satisfy(5.0) - 0.7).abs() < 1e-6, "only what was missing");
        assert_eq!(hunger.value, 0.0);
        assert_eq!(hunger.satisfy(-1.0), 0.0);
        hunger.update(f32::NAN);
        assert_eq!(hunger.value, 0.0);
    }

    #[test]
    fn recovering_drives_and_validation() {
        let mut fatigue = Drive::new(-0.2, 0.8).unwrap().with_value(0.5);
        fatigue.update(1.0);
        assert!((fatigue.value - 0.3).abs() < 1e-6);
        assert_eq!(fatigue.time_until_urgent(), None);
        fatigue.update(10.0);
        assert_eq!(fatigue.value, 0.0);
        assert!(Drive::new(f32::INFINITY, 0.5).is_err());
        assert!(Drive::new(0.1, 1.5).is_err());
        let loaded = Drive { value: 2.0, rate: 0.1, threshold: 0.5 };
        assert!(loaded.validate().is_err());
        let always = Drive::new(0.0, 0.0).unwrap();
        assert!(always.is_urgent());
        assert_eq!(always.urgency(), 0.0);
    }

    #[test]
    fn most_urgent_picks_the_highest_urgency() {
        let drives = [
            Drive::new(0.1, 0.5).unwrap().with_value(0.4),
            Drive::new(0.1, 0.5).unwrap().with_value(0.75),
            Drive::new(0.1, 0.0).unwrap().with_value(0.5),
            Drive::new(0.1, 0.5).unwrap().with_value(0.75),
        ];
        // Indices 1, 2 and 3 all have urgency 0.5; 1 and 3 have the larger
        // value and 1 is the lower index. Index 0 is below its threshold.
        assert_eq!(most_urgent(&drives), Some(1));
        let mut calm = drives;
        for d in &mut calm {
            d.value = 0.0;
        }
        assert_eq!(most_urgent(&calm), Some(2), "a zero threshold is always urgent");
        calm[2].threshold = 0.5;
        assert_eq!(most_urgent(&calm), None);
        update_all(&mut calm, 2.0);
        assert!(calm.iter().all(|d| (d.value - 0.2).abs() < 1e-6));
        let json = serde_json::to_string(&drives[2]).unwrap();
        assert_eq!(serde_json::from_str::<Drive>(&json).unwrap(), drives[2]);
    }
}
