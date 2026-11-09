//! Community diversity indices and inequality measures.
//!
//! All diversity functions take *abundances*: one non-negative number per
//! category (species, genotype, strategy...). Zero abundances are allowed and
//! do not count towards richness. With `pᵢ = nᵢ / N`:
//!
//! * richness `S` = number of categories with `nᵢ > 0`;
//! * Shannon `H' = −Σ pᵢ ln pᵢ` (natural logarithm, unit "nats");
//! * Gini–Simpson `1 − D` with `D = Σ pᵢ²` (probability that two individuals
//!   drawn with replacement belong to different categories);
//! * inverse Simpson `1 / D` (effective number of equally common categories);
//! * Pielou evenness `J = H' / ln S` (undefined for `S ≤ 1`).
//!
//! The Gini coefficient measures inequality of a distribution of non-negative
//! amounts (wealth, energy, offspring counts).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{AnalyticsError, check_finite};

/// Diversity indices of one community.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diversity {
    /// Number of categories with positive abundance.
    pub richness: usize,
    /// Total abundance `N`.
    pub total: f64,
    /// Shannon entropy `H'` in nats (0 for an empty or single-category community).
    pub shannon: f64,
    /// Gini–Simpson index `1 − Σ pᵢ²` (0 for an empty community).
    pub simpson: f64,
    /// Inverse Simpson index `1 / Σ pᵢ²` (`None` for an empty community).
    pub inverse_simpson: Option<f64>,
    /// Pielou evenness `H' / ln S` (`None` when `S ≤ 1`).
    pub pielou: Option<f64>,
}

fn check_abundances(abundances: &[f64]) -> Result<(), AnalyticsError> {
    check_finite("diversity", abundances)?;
    match abundances.iter().position(|&a| a < 0.0) {
        Some(i) => Err(AnalyticsError::invalid(
            "diversity",
            format!("abundance {} at position {i} is negative", abundances[i]),
        )),
        None => Ok(()),
    }
}

impl Diversity {
    /// Indices of a community given per-category abundances (non-negative,
    /// finite). An all-zero (extinct) community has richness 0 and zero
    /// Shannon and Simpson indices.
    pub fn from_abundances(abundances: &[f64]) -> Result<Self, AnalyticsError> {
        check_abundances(abundances)?;
        Ok(Self::compute(abundances.iter().copied()))
    }

    /// Indices from integer counts.
    pub fn from_counts(counts: &[u64]) -> Self {
        Self::compute(counts.iter().map(|&c| c as f64))
    }

    /// Indices of validated (finite, non-negative) abundances.
    fn compute(abundances: impl Iterator<Item = f64> + Clone) -> Self {
        let total: f64 = abundances.clone().sum();
        let richness = abundances.clone().filter(|&a| a > 0.0).count();
        if total <= 0.0 {
            return Self { richness: 0, total: 0.0, shannon: 0.0, simpson: 0.0, inverse_simpson: None, pielou: None };
        }
        let mut shannon = 0.0;
        let mut dominance = 0.0;
        for a in abundances.filter(|&a| a > 0.0) {
            let p = a / total;
            shannon -= p * p.ln();
            dominance += p * p;
        }
        // Rounding can leave −0.0 or a tiny negative for a single category.
        let shannon = shannon.max(0.0);
        let pielou = (richness > 1).then(|| (shannon / (richness as f64).ln()).clamp(0.0, 1.0));
        Self {
            richness,
            total,
            shannon,
            simpson: (1.0 - dominance).max(0.0),
            inverse_simpson: Some(1.0 / dominance),
            pielou,
        }
    }

    /// Indices from one label per individual (e.g. a species id per
    /// organism). Labels are counted in sorted order, so the result does not
    /// depend on iteration order.
    pub fn from_labels<K: Ord, I: IntoIterator<Item = K>>(labels: I) -> Self {
        let mut counts: BTreeMap<K, u64> = BTreeMap::new();
        for label in labels {
            *counts.entry(label).or_insert(0) += 1;
        }
        let counts: Vec<u64> = counts.into_values().collect();
        Self::from_counts(&counts)
    }
}

/// Species richness: number of positive abundances.
pub fn richness(abundances: &[f64]) -> Result<usize, AnalyticsError> {
    Diversity::from_abundances(abundances).map(|d| d.richness)
}

/// Shannon entropy `H' = −Σ pᵢ ln pᵢ` in nats.
pub fn shannon(abundances: &[f64]) -> Result<f64, AnalyticsError> {
    Diversity::from_abundances(abundances).map(|d| d.shannon)
}

/// Gini–Simpson index `1 − Σ pᵢ²`.
pub fn simpson(abundances: &[f64]) -> Result<f64, AnalyticsError> {
    Diversity::from_abundances(abundances).map(|d| d.simpson)
}

/// Pielou evenness `H' / ln S` (`None` when fewer than two categories are present).
pub fn pielou(abundances: &[f64]) -> Result<Option<f64>, AnalyticsError> {
    Diversity::from_abundances(abundances).map(|d| d.pielou)
}

fn sorted_amounts(what: &'static str, values: &[f64]) -> Result<Vec<f64>, AnalyticsError> {
    if values.is_empty() {
        return Err(AnalyticsError::NotEnoughData { what, needed: 1, got: 0 });
    }
    check_finite(what, values)?;
    if let Some(i) = values.iter().position(|&v| v < 0.0) {
        return Err(AnalyticsError::invalid(what, format!("amount {} at position {i} is negative", values[i])));
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Ok(sorted)
}

/// Gini coefficient of non-negative amounts:
/// `G = Σᵢ Σⱼ |xᵢ − xⱼ| / (2 n² x̄)`, computed in O(n log n) from the sorted
/// values as `G = Σᵢ (2i − n − 1)·x₍ᵢ₎ / (n Σ x)` (1-based ranks), which
/// avoids the cancellation of the equivalent `2 Σᵢ i·x₍ᵢ₎ / (n Σ x) − (n + 1) / n`.
/// 0 means perfect equality (returned exactly when all amounts are equal,
/// including all zero), `(n − 1) / n` that one member holds everything.
pub fn gini(values: &[f64]) -> Result<f64, AnalyticsError> {
    let sorted = sorted_amounts("Gini coefficient", values)?;
    let (smallest, largest) = (sorted[0], sorted[sorted.len() - 1]);
    if smallest == largest {
        return Ok(0.0);
    }
    let n = sorted.len();
    let total: f64 = sorted.iter().sum();
    let weighted: f64 = sorted.iter().enumerate().map(|(i, &x)| (2.0 * (i + 1) as f64 - (n + 1) as f64) * x).sum();
    Ok((weighted / (n as f64 * total)).clamp(0.0, 1.0))
}

/// Lorenz curve of non-negative amounts: points `(population share, amount
/// share)` from `(0, 0)` to `(1, 1)`, poorest first. For an all-zero
/// distribution the line of equality is returned.
pub fn lorenz_curve(values: &[f64]) -> Result<Vec<(f64, f64)>, AnalyticsError> {
    let sorted = sorted_amounts("Lorenz curve", values)?;
    let n = sorted.len() as f64;
    let total: f64 = sorted.iter().sum();
    let mut out = Vec::with_capacity(sorted.len() + 1);
    out.push((0.0, 0.0));
    let mut cumulative = 0.0;
    for (i, &x) in sorted.iter().enumerate() {
        cumulative += x;
        let share = if total > 0.0 { cumulative / total } else { (i + 1) as f64 / n };
        out.push(((i + 1) as f64 / n, share));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn even_community_maximises_indices() {
        let d = Diversity::from_abundances(&[5.0, 5.0, 5.0, 5.0]).unwrap();
        assert_eq!(d.richness, 4);
        assert!(close(d.shannon, 4f64.ln()));
        assert!(close(d.simpson, 0.75));
        assert!(close(d.inverse_simpson.unwrap(), 4.0));
        assert!(close(d.pielou.unwrap(), 1.0));
    }

    #[test]
    fn uneven_community_matches_hand_computation() {
        // p = 0.5, 0.25, 0.25.
        let d = Diversity::from_abundances(&[2.0, 1.0, 1.0, 0.0]).unwrap();
        let h = -(0.5 * 0.5f64.ln() + 2.0 * 0.25 * 0.25f64.ln());
        assert_eq!(d.richness, 3);
        assert!(close(d.shannon, h));
        assert!(close(d.simpson, 1.0 - (0.25 + 0.0625 + 0.0625)));
        assert!(close(d.pielou.unwrap(), h / 3f64.ln()));
        assert!(close(shannon(&[2.0, 1.0, 1.0]).unwrap(), h));
        assert_eq!(richness(&[0.0, 3.0]).unwrap(), 1);
    }

    #[test]
    fn degenerate_communities() {
        let single = Diversity::from_abundances(&[7.0]).unwrap();
        assert_eq!((single.richness, single.shannon, single.simpson), (1, 0.0, 0.0));
        assert_eq!(single.pielou, None);
        let extinct = Diversity::from_abundances(&[0.0, 0.0]).unwrap();
        assert_eq!((extinct.richness, extinct.shannon, extinct.inverse_simpson), (0, 0.0, None));
        assert!(Diversity::from_abundances(&[1.0, -1.0]).is_err());
        assert!(pielou(&[f64::NAN]).is_err());
    }

    #[test]
    fn labels_are_counted() {
        let d = Diversity::from_labels(["wolf", "deer", "deer", "deer", "wolf", "hare"]);
        assert_eq!(d.richness, 3);
        assert_eq!(d.total, 6.0);
        let same = Diversity::from_counts(&[2, 3, 1]);
        assert!(close(d.shannon, same.shannon));
        assert!(close(simpson(&[2.0, 3.0, 1.0]).unwrap(), same.simpson));
    }

    #[test]
    fn gini_of_equal_distribution_is_zero() {
        assert_eq!(gini(&[3.0, 3.0, 3.0, 3.0]).unwrap(), 0.0);
        assert_eq!(gini(&[0.0, 0.0]).unwrap(), 0.0);
        assert_eq!(gini(&[42.0]).unwrap(), 0.0);
    }

    #[test]
    fn gini_known_values() {
        // One member holds everything: (n − 1) / n.
        assert!(close(gini(&[0.0, 0.0, 0.0, 10.0]).unwrap(), 0.75));
        // 1, 2, 3, 4: Σ|xi − xj| over ordered pairs = 20, G = 20 / (2·16·2.5) = 0.25.
        assert!(close(gini(&[4.0, 1.0, 3.0, 2.0]).unwrap(), 0.25));
        assert!(gini(&[]).is_err());
        assert!(gini(&[1.0, -2.0]).is_err());
    }

    #[test]
    fn gini_matches_mean_absolute_difference_definition() {
        let mut rng = simulation_core::SimRng::seed_from_u64(3);
        for n in [2usize, 3, 7, 50, 333] {
            let values: Vec<f64> = (0..n).map(|_| rng.range_f64(0.0, 100.0).powi(2)).collect();
            let mean = values.iter().sum::<f64>() / n as f64;
            let pairs: f64 = values.iter().map(|a| values.iter().map(|b| (a - b).abs()).sum::<f64>()).sum();
            let expected = pairs / (2.0 * (n * n) as f64 * mean);
            assert!((gini(&values).unwrap() - expected).abs() < 1e-12, "n = {n}");
        }
        // Equal amounts give exactly 0 whatever their value and count.
        for x in [0.1, 1.0 / 3.0, 7.7, 1e-300] {
            for n in 1..60 {
                assert_eq!(gini(&vec![x; n]).unwrap(), 0.0, "{x} × {n}");
            }
        }
    }

    #[test]
    fn lorenz_curve_endpoints_and_shape() {
        let curve = lorenz_curve(&[1.0, 3.0]).unwrap();
        assert_eq!(curve, vec![(0.0, 0.0), (0.5, 0.25), (1.0, 1.0)]);
        assert_eq!(lorenz_curve(&[0.0, 0.0]).unwrap(), vec![(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)]);
    }
}
