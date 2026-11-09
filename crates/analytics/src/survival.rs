//! Kaplan–Meier survival curves from (possibly censored) lifespans.
//!
//! An individual is *censored* when it was still alive when observation
//! stopped (end of the run, emigration...): its lifespan is only known to be
//! at least the recorded duration. At each distinct time `tᵢ` with `dᵢ`
//! deaths among `nᵢ` individuals still at risk (lifespan ≥ `tᵢ`), the
//! product-limit estimator is
//!
//! ```text
//! S(t) = Π_{tᵢ ≤ t} (1 − dᵢ / nᵢ)
//! ```
//!
//! and Greenwood's formula gives its variance
//! `Var S(t) = S(t)² Σ_{tᵢ ≤ t} dᵢ / (nᵢ (nᵢ − dᵢ))`. Individuals censored at
//! a death time are counted as at risk at that time (censoring is assumed to
//! happen just after the deaths), the usual convention.

use serde::{Deserialize, Serialize};

use crate::error::AnalyticsError;

/// Observed lifespan of one individual.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lifespan {
    /// Time from birth to death or to the end of observation (any unit,
    /// e.g. ticks or days; must be finite and non-negative).
    pub duration: f64,
    /// `true` when the individual was still alive at the end of observation.
    pub censored: bool,
}

impl Lifespan {
    /// Observed death after `duration`.
    pub fn death(duration: f64) -> Self {
        Self { duration, censored: false }
    }

    /// Still alive after `duration` (right-censored).
    pub fn censored(duration: f64) -> Self {
        Self { duration, censored: true }
    }
}

/// One distinct time of the survival table.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurvivalStep {
    /// Time of the step.
    pub time: f64,
    /// Individuals at risk just before `time`.
    pub at_risk: usize,
    /// Deaths at `time`.
    pub deaths: usize,
    /// Individuals censored at `time`.
    pub censored: usize,
    /// Estimated survival probability just after `time`.
    pub survival: f64,
    /// Greenwood standard error of `survival` (`None` once survival reaches 0,
    /// where the formula is undefined).
    pub std_error: Option<f64>,
}

/// Kaplan–Meier survival curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurvivalCurve {
    subjects: usize,
    steps: Vec<SurvivalStep>,
}

impl SurvivalCurve {
    /// Product-limit estimate from lifespans (at least one record).
    pub fn kaplan_meier(records: &[Lifespan]) -> Result<Self, AnalyticsError> {
        if records.is_empty() {
            return Err(AnalyticsError::NotEnoughData { what: "Kaplan–Meier estimate", needed: 1, got: 0 });
        }
        if let Some((i, r)) = records.iter().enumerate().find(|(_, r)| !(r.duration.is_finite() && r.duration >= 0.0)) {
            return Err(AnalyticsError::invalid(
                "Kaplan–Meier estimate",
                format!("lifespan {} at position {i} must be a finite, non-negative number", r.duration),
            ));
        }
        let mut sorted = records.to_vec();
        sorted.sort_by(|a, b| a.duration.total_cmp(&b.duration));

        let mut steps = Vec::new();
        let mut at_risk = sorted.len();
        let mut survival = 1.0;
        let mut greenwood_sum = 0.0;
        let mut greenwood_defined = true;
        let mut i = 0;
        while i < sorted.len() {
            let time = sorted[i].duration;
            let mut deaths = 0;
            let mut censored = 0;
            while i < sorted.len() && sorted[i].duration == time {
                if sorted[i].censored {
                    censored += 1;
                } else {
                    deaths += 1;
                }
                i += 1;
            }
            if deaths > 0 {
                survival *= 1.0 - deaths as f64 / at_risk as f64;
                if deaths < at_risk {
                    greenwood_sum += deaths as f64 / (at_risk as f64 * (at_risk - deaths) as f64);
                } else {
                    greenwood_defined = false;
                }
            }
            let std_error = greenwood_defined.then(|| survival * greenwood_sum.sqrt());
            steps.push(SurvivalStep { time, at_risk, deaths, censored, survival, std_error });
            at_risk -= deaths + censored;
        }
        Ok(Self { subjects: records.len(), steps })
    }

    /// Number of individuals.
    pub fn subjects(&self) -> usize {
        self.subjects
    }

    /// Survival table, one row per distinct observed time (deaths or
    /// censoring), in increasing time order.
    pub fn steps(&self) -> &[SurvivalStep] {
        &self.steps
    }

    /// Total number of observed deaths.
    pub fn deaths(&self) -> usize {
        self.steps.iter().map(|s| s.deaths).sum()
    }

    /// `S(t)`: 1 before the first death, then the right-continuous step function.
    pub fn survival_at(&self, t: f64) -> f64 {
        let n = self.steps.partition_point(|s| s.time <= t);
        if n == 0 { 1.0 } else { self.steps[n - 1].survival }
    }

    /// Smallest time at which survival drops to `p` or below (`p ∈ (0, 1)`),
    /// or `None` if the curve never gets that low.
    pub fn time_to_survival(&self, p: f64) -> Option<f64> {
        self.steps.iter().find(|s| s.survival <= p).map(|s| s.time)
    }

    /// Median survival time: smallest time with `S(t) ≤ 0.5`.
    pub fn median(&self) -> Option<f64> {
        self.time_to_survival(0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Remission times (weeks) of the 6-MP group of Freireich et al. (1963),
    /// the standard textbook example (Kleinbaum & Klein, *Survival
    /// Analysis*, ch. 2). Negative numbers mark censored observations.
    fn freireich_6mp() -> Vec<Lifespan> {
        [6, 6, 6, -6, 7, -9, 10, -10, -11, 13, 16, -17, -19, -20, 22, 23, -25, -32, -32, -34, -35]
            .iter()
            .map(|&t: &i32| if t < 0 { Lifespan::censored(f64::from(-t)) } else { Lifespan::death(f64::from(t)) })
            .collect()
    }

    fn round3(x: f64) -> f64 {
        (x * 1000.0).round() / 1000.0
    }

    #[test]
    fn textbook_kaplan_meier_example() {
        let km = SurvivalCurve::kaplan_meier(&freireich_6mp()).unwrap();
        assert_eq!(km.subjects(), 21);
        assert_eq!(km.deaths(), 9);
        let expected =
            [(6.0, 0.857), (7.0, 0.807), (10.0, 0.753), (13.0, 0.690), (16.0, 0.627), (22.0, 0.538), (23.0, 0.448)];
        for (t, s) in expected {
            assert_eq!(round3(km.survival_at(t)), s, "S({t})");
        }
        // Risk sets of the textbook table.
        let at = |t: f64| km.steps().iter().find(|s| s.time == t).unwrap();
        assert_eq!((at(6.0).at_risk, at(6.0).deaths, at(6.0).censored), (21, 3, 1));
        assert_eq!(at(7.0).at_risk, 17);
        assert_eq!(at(10.0).at_risk, 15);
        assert_eq!(at(13.0).at_risk, 12);
        assert_eq!(at(22.0).at_risk, 7);
        assert_eq!(at(23.0).at_risk, 6);
        // Greenwood standard errors: 0.857 · √(3 / (21 · 18)) = 0.0764 at t = 6
        // and 0.807 · √(3 / 378 + 1 / (17 · 16)) = 0.0869 at t = 7.
        assert!((at(6.0).std_error.unwrap() - 0.0764).abs() < 5e-5);
        assert!((at(7.0).std_error.unwrap() - 0.0869).abs() < 5e-5);
        assert_eq!(km.median(), Some(23.0));
        assert_eq!(km.survival_at(5.9), 1.0);
        assert_eq!(round3(km.survival_at(40.0)), 0.448);
    }

    #[test]
    fn without_censoring_survival_is_the_empirical_fraction() {
        let records: Vec<Lifespan> = [1.0, 2.0, 2.0, 3.0].iter().map(|&t| Lifespan::death(t)).collect();
        let km = SurvivalCurve::kaplan_meier(&records).unwrap();
        assert_eq!(km.survival_at(1.0), 0.75);
        assert_eq!(km.survival_at(2.0), 0.25);
        assert_eq!(km.survival_at(3.0), 0.0);
        assert_eq!(km.steps().last().unwrap().std_error, None);
        assert_eq!(km.median(), Some(2.0));
    }

    #[test]
    fn invalid_records_are_rejected() {
        assert!(SurvivalCurve::kaplan_meier(&[]).is_err());
        assert!(SurvivalCurve::kaplan_meier(&[Lifespan::death(-1.0)]).is_err());
        assert!(SurvivalCurve::kaplan_meier(&[Lifespan::censored(f64::NAN)]).is_err());
        let all_censored = SurvivalCurve::kaplan_meier(&[Lifespan::censored(5.0)]).unwrap();
        assert_eq!(all_censored.median(), None);
        assert_eq!(all_censored.survival_at(10.0), 1.0);
    }
}
