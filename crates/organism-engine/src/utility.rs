//! Utility-based action selection (Infinite Axis Utility System style).
//!
//! Each candidate action is scored from a set of [`Consideration`]s. A
//! consideration maps a normalised input (`0..=1`: hunger, distance to food,
//! danger...) through a response [`Curve`] and an importance weight; the
//! action's [`ActionScore`] is the product of its consideration scores with
//! Dave Mark's *compensation factor* (GDC 2015, "Building a Better Centaur:
//! AI at Massive Scale"), multiplied by the action's own weight. The agent
//! then executes the action chosen by [`choose_best`].
//!
//! All functions clamp inputs and outputs to `[0, 1]` and map NaN to `0`, so a
//! bad sensor value can veto an action but never poison the decision.

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::error::OrganismError;

/// Clamps to `[0, 1]`, mapping NaN to `0`.
fn unit(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// Response curve mapping an input in `[0, 1]` to a score in `[0, 1]`.
/// Inputs are clamped to `[0, 1]` before evaluation and results are clamped
/// to `[0, 1]` afterwards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// `y = x`.
    Linear,
    /// `y = 1 − x`.
    InverseLinear,
    /// `y = x²`.
    Quadratic,
    /// Normalised exponential `y = (e^{k·x} − 1) / (e^{k} − 1)`: convex for
    /// `k > 0` (stays low, then rises sharply), concave for `k < 0`, linear
    /// for `k = 0`. Evaluated in a numerically stable form for any `k`.
    Exponential {
        /// Steepness.
        k: f32,
    },
    /// Logistic `y = 1 / (1 + e^{−k·(x − x0)})`.
    Logistic {
        /// Steepness (negative values give a falling sigmoid).
        k: f32,
        /// Midpoint.
        x0: f32,
    },
    /// `y = 1` if `x ≥ threshold`, else `0`.
    Step {
        /// Switching input.
        threshold: f32,
    },
    /// Piecewise-linear interpolation between validated control points.
    Custom(PiecewiseLinear),
}

impl Curve {
    /// Builds a [`Curve::Custom`]; see [`PiecewiseLinear::new`].
    pub fn custom(points: Vec<(f32, f32)>) -> Result<Self, OrganismError> {
        PiecewiseLinear::new(points).map(Self::Custom)
    }

    /// Evaluates the curve at `x` (see the type documentation for clamping).
    pub fn evaluate(&self, x: f32) -> f32 {
        let x = unit(x);
        let y = match self {
            Self::Linear => x,
            Self::InverseLinear => 1.0 - x,
            Self::Quadratic => x * x,
            Self::Exponential { k } => normalised_exponential(x, *k),
            Self::Logistic { k, x0 } => 1.0 / (1.0 + (-k * (x - x0)).exp()),
            Self::Step { threshold } => {
                if x >= *threshold {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Custom(points) => points.evaluate(x),
        };
        unit(y)
    }

    /// Checks that all parameters are finite (custom curves are validated on
    /// construction). Use after loading curves from data files.
    pub fn validate(&self) -> Result<(), OrganismError> {
        let finite = |name: &str, v: f32| {
            if v.is_finite() {
                Ok(())
            } else {
                Err(OrganismError::InvalidCurve(format!("{name} must be finite, got {v}")))
            }
        };
        match self {
            Self::Exponential { k } => finite("k", *k),
            Self::Logistic { k, x0 } => finite("k", *k).and_then(|()| finite("x0", *x0)),
            Self::Step { threshold } => finite("threshold", *threshold),
            Self::Linear | Self::InverseLinear | Self::Quadratic | Self::Custom(_) => Ok(()),
        }
    }
}

fn normalised_exponential(x: f32, k: f32) -> f32 {
    let (x, k) = (f64::from(x), f64::from(k));
    if k.abs() < 1e-6 {
        return x as f32;
    }
    let y = if k > 0.0 {
        // (e^{kx} − 1)/(e^k − 1) = e^{k(x−1)} · (1 − e^{−kx}) / (1 − e^{−k}); no overflow.
        (k * (x - 1.0)).exp() * -(-k * x).exp_m1() / -(-k).exp_m1()
    } else {
        (k * x).exp_m1() / k.exp_m1()
    };
    y as f32
}

/// Validated control points of a [`Curve::Custom`]: at least one point, all
/// coordinates finite and in `[0, 1]`, `x` strictly increasing. Outside the
/// first/last point the curve is constant. Serialised as a list of `(x, y)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<(f32, f32)>", into = "Vec<(f32, f32)>")]
pub struct PiecewiseLinear {
    points: Vec<(f32, f32)>,
}

impl PiecewiseLinear {
    /// Validates the control points.
    pub fn new(points: Vec<(f32, f32)>) -> Result<Self, OrganismError> {
        let invalid = |m: String| Err(OrganismError::InvalidCurve(m));
        if points.is_empty() {
            return invalid("a custom curve needs at least one point".into());
        }
        if let Some((x, y)) = points.iter().find(|(x, y)| !(0.0..=1.0).contains(x) || !(0.0..=1.0).contains(y)) {
            return invalid(format!("point ({x}, {y}) is outside [0, 1] × [0, 1]"));
        }
        if let Some(w) = points.windows(2).find(|w| w[0].0 >= w[1].0) {
            return invalid(format!("x values must increase strictly ({} is followed by {})", w[0].0, w[1].0));
        }
        Ok(Self { points })
    }

    /// The control points.
    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// Linear interpolation at `x`.
    pub fn evaluate(&self, x: f32) -> f32 {
        let after = self.points.partition_point(|p| p.0 <= x);
        match (after.checked_sub(1).and_then(|i| self.points.get(i)), self.points.get(after)) {
            (Some(&(x0, y0)), Some(&(x1, y1))) => y0 + (y1 - y0) * (x - x0) / (x1 - x0),
            (Some(&(_, y)), None) | (None, Some(&(_, y))) => y,
            (None, None) => 0.0,
        }
    }
}

impl TryFrom<Vec<(f32, f32)>> for PiecewiseLinear {
    type Error = OrganismError;

    fn try_from(points: Vec<(f32, f32)>) -> Result<Self, Self::Error> {
        Self::new(points)
    }
}

impl From<PiecewiseLinear> for Vec<(f32, f32)> {
    fn from(curve: PiecewiseLinear) -> Self {
        curve.points
    }
}

/// One factor of an action's utility.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Consideration<'a> {
    /// Normalised input in `[0, 1]` (clamped; NaN counts as `0`).
    pub input: f32,
    /// Response curve.
    pub curve: &'a Curve,
    /// Importance in `[0, 1]` (clamped): the score is `1 − weight·(1 − curve(input))`,
    /// so weight `0` makes the consideration irrelevant and `1` applies it fully.
    pub weight: f32,
}

impl<'a> Consideration<'a> {
    /// Fully weighted consideration.
    pub fn new(input: f32, curve: &'a Curve) -> Self {
        Self { input, curve, weight: 1.0 }
    }

    /// Sets the importance weight.
    pub fn weighted(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Score in `[0, 1]`.
    pub fn score(&self) -> f32 {
        1.0 - unit(self.weight) * (1.0 - self.curve.evaluate(self.input))
    }
}

/// Combined utility of one candidate action.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionScore {
    /// Final utility: `weight × product` (≥ 0). Feed these to [`choose_best`].
    pub value: f32,
    /// Compensated product of the consideration scores, in `[0, 1]`.
    pub product: f32,
    /// Index of the first consideration that scored zero, which vetoes the
    /// action (then `value == product == 0`). Useful for decision debugging.
    pub vetoed_by: Option<usize>,
}

impl ActionScore {
    /// Scores an action from its considerations. `weight` is the action's
    /// own multiplier (e.g. 1 for idle behaviours, 3 for emergencies); a
    /// negative or non-finite weight counts as `0`.
    ///
    /// With `n` considerations, each score `s` is first compensated as
    /// `s + (1 − s)·(1 − 1/n)·s` and the results are multiplied, which keeps
    /// actions with many considerations from being unfairly penalised by the
    /// product. An action without considerations scores `weight`.
    pub fn evaluate(weight: f32, considerations: &[Consideration<'_>]) -> Self {
        Self::combine(weight, considerations.len(), considerations.iter().map(Consideration::score))
    }

    /// Same as [`ActionScore::evaluate`] for already computed consideration
    /// scores (clamped to `[0, 1]`), e.g. produced by a script.
    pub fn from_scores(weight: f32, scores: &[f32]) -> Self {
        Self::combine(weight, scores.len(), scores.iter().map(|s| unit(*s)))
    }

    fn combine(weight: f32, n: usize, scores: impl Iterator<Item = f32>) -> Self {
        let weight = if weight.is_finite() && weight > 0.0 { weight } else { 0.0 };
        if n == 0 {
            return Self { value: weight, product: 1.0, vetoed_by: None };
        }
        let modification = 1.0 - 1.0 / n as f32;
        let mut product = 1.0;
        for (i, s) in scores.enumerate() {
            if s <= 0.0 {
                return Self { value: 0.0, product: 0.0, vetoed_by: Some(i) };
            }
            let makeup = (1.0 - s) * modification;
            product *= s + makeup * s;
        }
        Self { value: weight * product, product, vetoed_by: None }
    }
}

/// Index of the highest positive, finite score, or `None` if there is none
/// (all actions vetoed: the caller should fall back to a default behaviour).
///
/// When `rng` is given and `noise > 0`, each score is multiplied by
/// `1 + noise·(2u − 1)` with `u ~ U[0, 1)` (`noise` clamped to `[0, 1]`) to
/// break up identical behaviour among similar agents; zero scores stay zero.
/// Whenever an rng is given, exactly one number is drawn per score, so the
/// random stream advances independently of the score values. Ties go to the
/// lowest index.
pub fn choose_best(scores: &[f32], mut rng: Option<&mut SimRng>, noise: f32) -> Option<usize> {
    let noise = unit(noise);
    let mut best: Option<(usize, f32)> = None;
    for (i, &score) in scores.iter().enumerate() {
        let jitter = rng.as_deref_mut().map_or(1.0, |r| 1.0 + noise * (2.0 * r.f32() - 1.0));
        if !(score.is_finite() && score > 0.0) {
            continue;
        }
        let value = score * jitter;
        if best.is_none_or(|(_, b)| value > b) {
            best = Some((i, value));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn curve_shapes() {
        assert!(approx(Curve::Linear.evaluate(0.3), 0.3));
        assert!(approx(Curve::InverseLinear.evaluate(0.3), 0.7));
        assert!(approx(Curve::Quadratic.evaluate(0.5), 0.25));
        let exp = Curve::Exponential { k: 5.0 };
        assert!(approx(exp.evaluate(0.0), 0.0) && approx(exp.evaluate(1.0), 1.0));
        assert!(exp.evaluate(0.5) < 0.5, "convex for k > 0");
        assert!(Curve::Exponential { k: -5.0 }.evaluate(0.5) > 0.5, "concave for k < 0");
        assert!(approx(Curve::Exponential { k: 0.0 }.evaluate(0.4), 0.4));
        let steep = Curve::Exponential { k: 500.0 };
        assert!(approx(steep.evaluate(1.0), 1.0) && steep.evaluate(0.9) < 1e-6, "no overflow for large k");
        let logistic = Curve::Logistic { k: 10.0, x0: 0.5 };
        assert!(approx(logistic.evaluate(0.5), 0.5));
        assert!(logistic.evaluate(0.9) > 0.98 && logistic.evaluate(0.1) < 0.02);
        let step = Curve::Step { threshold: 0.6 };
        assert_eq!((step.evaluate(0.59), step.evaluate(0.6)), (0.0, 1.0));
        // Inputs are clamped and NaN maps to 0.
        assert_eq!(Curve::Linear.evaluate(3.0), 1.0);
        assert_eq!(Curve::Linear.evaluate(f32::NAN), 0.0);
        assert_eq!(Curve::Logistic { k: f32::NAN, x0: 0.5 }.evaluate(0.7), 0.0);
        assert!(Curve::Logistic { k: f32::NAN, x0: 0.5 }.validate().is_err());
        assert!(logistic.validate().is_ok());
    }

    #[test]
    fn custom_curve_interpolates_and_validates() {
        let c = Curve::custom(vec![(0.2, 0.0), (0.5, 1.0), (1.0, 0.5)]).unwrap();
        assert_eq!(c.evaluate(0.0), 0.0);
        assert!(approx(c.evaluate(0.35), 0.5));
        assert!(approx(c.evaluate(0.75), 0.75));
        assert_eq!(c.evaluate(1.0), 0.5);
        assert_eq!(Curve::custom(vec![(0.5, 0.7)]).unwrap().evaluate(0.1), 0.7);
        assert!(Curve::custom(vec![]).is_err());
        assert!(Curve::custom(vec![(0.5, 0.0), (0.5, 1.0)]).is_err());
        assert!(Curve::custom(vec![(0.0, 2.0)]).is_err());
        assert!(Curve::custom(vec![(f32::NAN, 0.0)]).is_err());
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#"{"custom":[[0.2,0.0],[0.5,1.0],[1.0,0.5]]}"#);
        assert_eq!(serde_json::from_str::<Curve>(&json).unwrap(), c);
        let err = serde_json::from_str::<Curve>(r#"{"custom":[[0.6,0.0],[0.5,1.0]]}"#).unwrap_err();
        assert!(err.to_string().contains("increase strictly"), "{err}");
        let tagged = serde_json::to_string(&Curve::Logistic { k: 2.0, x0: 0.25 }).unwrap();
        assert_eq!(tagged, r#"{"logistic":{"k":2.0,"x0":0.25}}"#);
    }

    #[test]
    fn consideration_weight_softens_the_curve() {
        let c = Curve::Linear;
        assert!(approx(Consideration::new(0.2, &c).score(), 0.2));
        assert!(approx(Consideration::new(0.2, &c).weighted(0.5).score(), 0.6));
        assert_eq!(Consideration::new(0.0, &c).weighted(0.0).score(), 1.0, "weight 0 is ignored");
    }

    #[test]
    fn action_score_uses_the_compensation_factor() {
        // Two considerations of 0.5: m = 0.5, each becomes 0.5 + 0.25·0.5 = 0.625.
        let s = ActionScore::from_scores(2.0, &[0.5, 0.5]);
        assert!(approx(s.product, 0.390_625) && approx(s.value, 0.781_25));
        // A single consideration is not modified.
        assert!(approx(ActionScore::from_scores(1.0, &[0.4]).value, 0.4));
        // Compensation: more considerations of the same value lose less than the raw product.
        let many = ActionScore::from_scores(1.0, &[0.8; 6]).value;
        assert!(many > 0.8f32.powi(6) && many < 0.8);
        // Any zero vetoes.
        let vetoed = ActionScore::from_scores(5.0, &[0.9, 0.0, 0.7]);
        assert_eq!((vetoed.value, vetoed.vetoed_by), (0.0, Some(1)));
        assert_eq!(ActionScore::from_scores(1.5, &[]).value, 1.5);
        assert_eq!(ActionScore::from_scores(-1.0, &[1.0]).value, 0.0);
        let hunger = Curve::Quadratic;
        let food_near = Curve::InverseLinear;
        let eat = ActionScore::evaluate(
            1.0,
            &[Consideration::new(0.9, &hunger), Consideration::new(0.1, &food_near).weighted(0.8)],
        );
        let expected = ActionScore::from_scores(1.0, &[0.81, 1.0 - 0.8 * 0.1]);
        assert!(approx(eat.value, expected.value));
    }

    #[test]
    fn choose_best_breaks_ties_by_lowest_index() {
        assert_eq!(choose_best(&[0.2, 0.7, 0.7, 0.1], None, 0.0), Some(1));
        assert_eq!(choose_best(&[0.0, f32::NAN, -1.0], None, 0.0), None);
        assert_eq!(choose_best(&[], None, 0.0), None);
        assert_eq!(choose_best(&[f32::INFINITY, 0.5], None, 0.0), Some(1), "non-finite scores are ignored");
    }

    #[test]
    fn choose_best_noise_is_deterministic_and_bounded() {
        let scores = [0.50, 0.52, 0.0, 0.1];
        let picks = |seed| {
            let mut rng = SimRng::seed_from_u64(seed);
            (0..2000).map(|_| choose_best(&scores, Some(&mut rng), 0.2).unwrap()).collect::<Vec<_>>()
        };
        let a = picks(9);
        assert_eq!(a, picks(9));
        let ones = a.iter().filter(|&&i| i == 1).count();
        assert!(a.contains(&0) && ones > a.len() / 2, "noise lets the close runner-up win sometimes");
        assert!(!a.contains(&2), "zero scores never win");
        assert!(!a.contains(&3), "0.1 can never beat 0.5 with ±20% noise");
        // One draw per score regardless of values.
        let mut r1 = SimRng::seed_from_u64(1);
        let mut r2 = SimRng::seed_from_u64(1);
        choose_best(&[0.0, 0.0, 0.0], Some(&mut r1), 0.5);
        choose_best(&[1.0, 2.0, 3.0], Some(&mut r2), 0.5);
        assert_eq!(r1, r2);
    }
}
