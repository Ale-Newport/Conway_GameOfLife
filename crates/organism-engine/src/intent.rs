//! Deterministic resolution of conflicting intents.
//!
//! Agents decide in parallel from read-only state and express what they want
//! as [`Claim`]s ("eat 3 units from plant 17", "attack prey #42"). Conflicts
//! are resolved afterwards, serially and in a fixed order, so the outcome
//! never depends on thread scheduling or on the order in which claims were
//! collected (`ARCHITECTURE.md` §3.2).
//!
//! **Claim order.** Within a target, claims are ranked by priority
//! (descending), then distance (ascending), then claimant id (ascending), then
//! amount (descending, only to order exact duplicates). NaN priorities rank
//! last, NaN distances count as infinitely far, and NaN or negative amounts
//! count as zero.
//!
//! **Modes** ([`Resolution`]):
//! * [`Resolution::Exclusive`] — each target goes to its best-ranked claim;
//!   a claimant may win several targets ([`resolve_exclusive`]).
//! * [`Resolution::Matching`] — greedy one-to-one assignment over all claims
//!   in global rank order: a claimant wins at most one target and a target
//!   at most one claimant ([`resolve_matching`]). Agents can submit fallback
//!   claims (with lower priority) and still end up with a single target.
//! * [`Resolution::Shared`] — divisible resources split among all claimants
//!   according to a [`ShareRule`] ([`resolve_shared`]).

use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::id::AgentId;

/// A request by an agent for (part of) a target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    /// The requesting agent.
    pub claimant: AgentId,
    /// What is claimed: a resource id, a cell index or another agent's id.
    pub target: u64,
    /// Higher priority wins.
    pub priority: f32,
    /// Distance to the target; closer wins among equal priorities.
    pub distance: f32,
    /// Amount requested (ignored by the winner-only resolvers).
    pub amount: f32,
}

impl Claim {
    /// Claim with priority 0, distance 0 and amount 1.
    pub fn new(claimant: AgentId, target: u64) -> Self {
        Self { claimant, target, priority: 0.0, distance: 0.0, amount: 1.0 }
    }

    /// Sets the priority.
    pub fn priority(mut self, priority: f32) -> Self {
        self.priority = priority;
        self
    }

    /// Sets the distance.
    pub fn distance(mut self, distance: f32) -> Self {
        self.distance = distance;
        self
    }

    /// Sets the requested amount.
    pub fn amount(mut self, amount: f32) -> Self {
        self.amount = amount;
        self
    }

    fn sane_priority(&self) -> f32 {
        // `+ 0.0` turns -0.0 into +0.0 so both rank equally.
        if self.priority.is_nan() { f32::NEG_INFINITY } else { self.priority + 0.0 }
    }

    fn sane_distance(&self) -> f32 {
        if self.distance.is_nan() { f32::INFINITY } else { self.distance + 0.0 }
    }

    fn requested(&self) -> f32 {
        if self.amount.is_nan() || self.amount <= 0.0 { 0.0 } else { self.amount }
    }

    /// Rank order among claims for the same target (smaller = better). The
    /// final comparisons on raw bits make this a total order on distinct
    /// claims, so sorting is independent of the input order even for claims
    /// that only differ in sanitised-away details (NaN vs negative amounts).
    fn rank(&self, other: &Self) -> Ordering {
        self.rank_head(other).then_with(|| self.rank_tail(other))
    }

    /// The meaningful keys: priority desc, distance asc, claimant asc.
    fn rank_head(&self, other: &Self) -> Ordering {
        other
            .sane_priority()
            .total_cmp(&self.sane_priority())
            .then_with(|| self.sane_distance().total_cmp(&other.sane_distance()))
            .then_with(|| self.claimant.cmp(&other.claimant))
    }

    /// Keys that only order otherwise identical claims: amount desc, then raw bits.
    fn rank_tail(&self, other: &Self) -> Ordering {
        other
            .requested()
            .total_cmp(&self.requested())
            .then_with(|| self.priority.to_bits().cmp(&other.priority.to_bits()))
            .then_with(|| self.distance.to_bits().cmp(&other.distance.to_bits()))
            .then_with(|| self.amount.to_bits().cmp(&other.amount.to_bits()))
    }
}

/// Grant produced by [`resolve_shared`] or [`resolve_claims`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Allocation {
    /// The requesting agent.
    pub claimant: AgentId,
    /// The claimed target.
    pub target: u64,
    /// Sanitised amount requested.
    pub requested: f32,
    /// Amount granted, `0 ≤ granted ≤ requested`.
    pub granted: f32,
}

/// How a divisible target is split by [`resolve_shared`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareRule {
    /// If the requests exceed the available amount, every claimant gets the
    /// same fraction `available / Σ requested` of its request (fair sharing;
    /// priority and distance only decide who absorbs rounding error).
    #[default]
    Proportional,
    /// Claimants are served in rank order, each receiving
    /// `min(requested, remaining)` (first come, first served, where "first"
    /// means highest priority, then closest, then lowest id).
    Ordered,
}

/// Conflict-resolution mode for [`resolve_claims`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// One winner per target ([`resolve_exclusive`]).
    Exclusive,
    /// One target per claimant and one claimant per target ([`resolve_matching`]).
    Matching,
    /// Divisible targets ([`resolve_shared`]).
    Shared(ShareRule),
}

fn sorted_by_target(claims: &[Claim]) -> Vec<Claim> {
    let mut sorted = claims.to_vec();
    sorted.sort_by(|a, b| a.target.cmp(&b.target).then_with(|| a.rank(b)));
    sorted
}

/// Winning claim for each target (the best-ranked one), sorted by target.
/// A claimant may win several targets; use [`resolve_matching`] if it must
/// not.
pub fn resolve_exclusive(claims: &[Claim]) -> Vec<Claim> {
    let mut winners = sorted_by_target(claims);
    winners.dedup_by_key(|c| c.target);
    winners
}

/// Greedy one-to-one assignment: all claims are visited in global rank order
/// (priority desc, distance asc, claimant asc, then target asc; the amount
/// only orders exact duplicates) and accepted when neither their claimant nor
/// their target is taken yet. Result sorted by target. This yields a maximal
/// (not necessarily maximum-cardinality) matching that always respects
/// priorities first.
pub fn resolve_matching(claims: &[Claim]) -> Vec<Claim> {
    let mut sorted = claims.to_vec();
    sorted.sort_by(|a, b| a.rank_head(b).then_with(|| a.target.cmp(&b.target)).then_with(|| a.rank_tail(b)));
    let mut taken_claimants = BTreeSet::new();
    let mut taken_targets = BTreeSet::new();
    let mut winners: Vec<Claim> = sorted
        .into_iter()
        .filter(|c| {
            if taken_claimants.contains(&c.claimant) || taken_targets.contains(&c.target) {
                return false;
            }
            taken_claimants.insert(c.claimant);
            taken_targets.insert(c.target);
            true
        })
        .collect();
    winners.sort_by_key(|c| c.target);
    winners
}

/// Splits divisible targets among their claimants. `available(target)` is
/// called once per distinct target, in increasing target order; negative or
/// NaN availability counts as zero, `f32::INFINITY` as unlimited. An
/// infinite request takes everything that remains when it is served.
///
/// Returns one [`Allocation`] per claim, sorted by target and then rank.
/// Grants are produced by sequential subtraction from the available amount
/// in that order, so subtracting them from a resource in the returned order
/// never drives it below zero (exact conservation for nutrient accounting).
/// The proportional fraction is computed in `f64`, so requests whose `f32`
/// sum would overflow are still shared correctly.
pub fn resolve_shared(claims: &[Claim], mut available: impl FnMut(u64) -> f32, rule: ShareRule) -> Vec<Allocation> {
    let sorted = sorted_by_target(claims);
    let mut out = Vec::with_capacity(sorted.len());
    for group in sorted.chunk_by(|a, b| a.target == b.target) {
        let target = group[0].target;
        let supply = available(target);
        let mut remaining = if supply.is_nan() || supply < 0.0 { 0.0 } else { supply };
        let total: f64 = group.iter().map(|c| f64::from(c.requested())).sum();
        let fraction = match rule {
            ShareRule::Proportional if total > f64::from(remaining) => f64::from(remaining) / total,
            _ => 1.0,
        };
        for claim in group {
            let requested = claim.requested();
            let share = if requested.is_infinite() { remaining } else { (f64::from(requested) * fraction) as f32 };
            let granted = share.min(requested).min(remaining).max(0.0);
            // `max` only matters for ∞ − ∞ (an infinite request on an infinite supply).
            remaining = (remaining - granted).max(0.0);
            out.push(Allocation { claimant: claim.claimant, target, requested, granted });
        }
    }
    out
}

/// Single entry point for data-driven models: resolves `claims` with the
/// given mode and reports grants as [`Allocation`]s. For the winner-only
/// modes the winner is granted `min(requested, available(target))`; pass
/// `|_| f32::INFINITY` when amounts are irrelevant (e.g. attacks).
pub fn resolve_claims(
    claims: &[Claim],
    resolution: Resolution,
    mut available: impl FnMut(u64) -> f32,
) -> Vec<Allocation> {
    let winners = match resolution {
        Resolution::Shared(rule) => return resolve_shared(claims, available, rule),
        Resolution::Exclusive => resolve_exclusive(claims),
        Resolution::Matching => resolve_matching(claims),
    };
    winners
        .into_iter()
        .map(|c| {
            let supply = available(c.target);
            let supply = if supply.is_nan() || supply < 0.0 { 0.0 } else { supply };
            let requested = c.requested();
            Allocation { claimant: c.claimant, target: c.target, requested, granted: requested.min(supply) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::SimRng;

    fn claim(claimant: u64, target: u64, priority: f32, distance: f32) -> Claim {
        Claim::new(AgentId(claimant), target).priority(priority).distance(distance)
    }

    #[test]
    fn exclusive_orders_by_priority_distance_then_id() {
        let claims = [
            claim(1, 10, 1.0, 5.0),
            claim(2, 10, 2.0, 9.0), // highest priority wins target 10
            claim(3, 20, 1.0, 3.0),
            claim(4, 20, 1.0, 2.0), // closer wins target 20
            claim(6, 30, 1.0, 1.0),
            claim(5, 30, 1.0, 1.0), // equal: lower id wins target 30
        ];
        let winners: Vec<(u64, u64)> = resolve_exclusive(&claims).iter().map(|c| (c.target, c.claimant.0)).collect();
        assert_eq!(winners, vec![(10, 2), (20, 4), (30, 5)]);
    }

    #[test]
    fn resolution_is_independent_of_input_order() {
        let mut rng = SimRng::seed_from_u64(77);
        let mut claims: Vec<Claim> = (0..200)
            .map(|i| {
                let priority = if i % 37 == 0 { f32::NAN } else { rng.below(3) as f32 };
                let amount = if i % 41 == 0 { f32::NAN } else { rng.range_f32(-1.0, 3.0) };
                claim(rng.below(40), rng.below(15), priority, rng.below(4) as f32).amount(amount)
            })
            .collect();
        // NaN fields make `PartialEq` useless, so compare the debug renderings.
        let show = |v: &dyn std::fmt::Debug| format!("{v:?}");
        let exclusive = resolve_exclusive(&claims);
        let matching = resolve_matching(&claims);
        let shared = resolve_shared(&claims, |t| t as f32, ShareRule::Proportional);
        let ordered = resolve_shared(&claims, |t| t as f32, ShareRule::Ordered);
        for _ in 0..5 {
            rng.shuffle(&mut claims);
            assert_eq!(show(&resolve_exclusive(&claims)), show(&exclusive));
            assert_eq!(show(&resolve_matching(&claims)), show(&matching));
            assert_eq!(resolve_shared(&claims, |t| t as f32, ShareRule::Proportional), shared);
            assert_eq!(resolve_shared(&claims, |t| t as f32, ShareRule::Ordered), ordered);
        }
        // Every target with claims has exactly one exclusive winner.
        let targets: BTreeSet<u64> = claims.iter().map(|c| c.target).collect();
        assert_eq!(exclusive.len(), targets.len());
    }

    #[test]
    fn nan_priority_ranks_last_and_negative_zero_equals_zero() {
        let w = resolve_exclusive(&[claim(1, 0, f32::NAN, 0.0), claim(2, 0, -100.0, 0.0)]);
        assert_eq!(w[0].claimant, AgentId(2));
        let w = resolve_exclusive(&[claim(2, 0, 0.0, 0.0), claim(1, 0, -0.0, 0.0)]);
        assert_eq!(w[0].claimant, AgentId(1), "-0.0 ties with 0.0, so the lower id wins");
    }

    #[test]
    fn matching_gives_each_claimant_at_most_one_target() {
        // Wolf 1 prefers deer 10, falls back to deer 11; wolf 2 only wants deer 10
        // but is farther. Exclusive would give both deer to wolf 1.
        let claims = [claim(1, 10, 1.0, 1.0), claim(1, 11, 0.5, 1.0), claim(2, 10, 1.0, 3.0), claim(2, 11, 0.5, 9.0)];
        let exclusive: Vec<_> = resolve_exclusive(&claims).iter().map(|c| (c.target, c.claimant.0)).collect();
        assert_eq!(exclusive, vec![(10, 1), (11, 1)]);
        let matching: Vec<_> = resolve_matching(&claims).iter().map(|c| (c.target, c.claimant.0)).collect();
        assert_eq!(matching, vec![(10, 1), (11, 2)]);
    }

    #[test]
    fn proportional_sharing_is_fair_and_conserves_the_resource() {
        let claims = [claim(1, 7, 0.0, 0.0).amount(2.0), claim(2, 7, 5.0, 0.0).amount(6.0)];
        let a = resolve_shared(&claims, |_| 4.0, ShareRule::Proportional);
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].claimant, a[0].granted), (AgentId(2), 3.0));
        assert_eq!((a[1].claimant, a[1].granted), (AgentId(1), 1.0));
        // Enough for everybody: requests are granted in full.
        let plenty = resolve_shared(&claims, |_| 100.0, ShareRule::Proportional);
        assert_eq!(plenty.iter().map(|a| a.granted).sum::<f32>(), 8.0);
        // Conservation with awkward numbers: sequential subtraction never goes negative.
        let many: Vec<Claim> = (0..7).map(|i| claim(i, 1, 0.0, 0.0).amount(0.1 + i as f32 * 0.37)).collect();
        let grants = resolve_shared(&many, |_| 1.3, ShareRule::Proportional);
        let mut resource = 1.3f32;
        for g in &grants {
            assert!(g.granted <= g.requested);
            resource -= g.granted;
            assert!(resource >= 0.0);
        }
        assert!(resource < 1e-5);
    }

    #[test]
    fn ordered_sharing_serves_by_rank() {
        let claims = [
            claim(1, 7, 0.0, 1.0).amount(2.0),
            claim(2, 7, 1.0, 9.0).amount(3.0),
            claim(3, 7, 0.0, 0.5).amount(2.0),
            claim(4, 8, 0.0, 0.0).amount(f32::NAN),
        ];
        let a = resolve_shared(&claims, |t| if t == 7 { 4.0 } else { f32::NAN }, ShareRule::Ordered);
        let got: Vec<(u64, f32)> = a.iter().map(|a| (a.claimant.0, a.granted)).collect();
        assert_eq!(got, vec![(2, 3.0), (3, 1.0), (1, 0.0), (4, 0.0)]);
        assert_eq!(a[3].requested, 0.0, "NaN amounts count as zero");
        let greedy = [claim(1, 0, 1.0, 0.0).amount(f32::INFINITY), claim(2, 0, 0.0, 0.0).amount(1.0)];
        for rule in [ShareRule::Ordered, ShareRule::Proportional] {
            let a = resolve_shared(&greedy, |_| 5.0, rule);
            assert_eq!((a[0].granted, a[1].granted), (5.0, 0.0), "{rule:?}");
            let a = resolve_shared(&greedy, |_| f32::INFINITY, rule);
            assert!(a.iter().all(|x| !x.granted.is_nan()), "{rule:?}: {a:?}");
        }
    }

    #[test]
    fn proportional_sharing_survives_f32_overflow_of_the_total() {
        // 3e38 + 3e38 overflows f32; each claimant must still get half.
        let claims = [claim(1, 0, 0.0, 0.0).amount(3.0e38), claim(2, 0, 0.0, 0.0).amount(3.0e38)];
        let a = resolve_shared(&claims, |_| 1.0e38, ShareRule::Proportional);
        for g in &a {
            assert!((g.granted / 5.0e37 - 1.0).abs() < 1e-6, "{a:?}");
        }
        assert!(a.iter().map(|g| f64::from(g.granted)).sum::<f64>() <= 1.0e38);
    }

    #[test]
    fn matching_breaks_claimant_ties_by_target_before_amount() {
        // Same claimant, priority and distance: the lower target wins even
        // though the other claim asks for more.
        let claims = [claim(1, 2, 0.0, 0.0).amount(5.0), claim(1, 1, 0.0, 0.0).amount(1.0)];
        assert_eq!(resolve_matching(&claims)[0].target, 1);
    }

    /// Explicit, independent statement of the documented ranking (for
    /// non-NaN inputs): larger is better.
    fn key(c: &Claim) -> (f32, f32, std::cmp::Reverse<AgentId>) {
        (c.priority, -c.distance, std::cmp::Reverse(c.claimant))
    }

    fn better(a: &Claim, b: &Claim) -> bool {
        key(a).partial_cmp(&key(b)) == Some(Ordering::Greater)
    }

    #[test]
    fn exclusive_and_matching_agree_with_brute_force() {
        let mut rng = SimRng::seed_from_u64(31);
        for _ in 0..50 {
            // Distinct (claimant, target) pairs so the brute-force key is strict.
            let mut pairs: Vec<(u64, u64)> = (0..12).flat_map(|c| (0..8).map(move |t| (c, t))).collect();
            rng.shuffle(&mut pairs);
            let claims: Vec<Claim> =
                pairs[..40].iter().map(|&(c, t)| claim(c, t, rng.below(3) as f32, rng.below(3) as f32)).collect();

            // Exclusive: the winner of each target beats every other claim on it.
            let winners = resolve_exclusive(&claims);
            for w in &winners {
                assert!(claims.iter().filter(|c| c.target == w.target && *c != w).all(|c| better(w, c)));
            }

            // Matching: one-to-one, and maximal in the greedy sense: a claim is
            // rejected only if a better-ranked accepted claim took its claimant
            // or its target.
            let matched = resolve_matching(&claims);
            let claimants: BTreeSet<AgentId> = matched.iter().map(|c| c.claimant).collect();
            let targets: BTreeSet<u64> = matched.iter().map(|c| c.target).collect();
            assert_eq!((claimants.len(), targets.len()), (matched.len(), matched.len()));
            for c in claims.iter().filter(|c| !matched.contains(c)) {
                let blocked =
                    matched.iter().any(|m| (m.claimant == c.claimant || m.target == c.target) && !better(c, m));
                assert!(blocked, "{c:?} was rejected without a better conflicting winner");
            }
        }
    }

    #[test]
    fn resolve_claims_dispatches_and_caps_by_availability() {
        let claims = [claim(1, 5, 1.0, 0.0).amount(3.0), claim(2, 5, 0.0, 0.0).amount(3.0)];
        let ex = resolve_claims(&claims, Resolution::Exclusive, |_| 2.0);
        assert_eq!(ex, vec![Allocation { claimant: AgentId(1), target: 5, requested: 3.0, granted: 2.0 }]);
        let unlimited = resolve_claims(&claims, Resolution::Matching, |_| f32::INFINITY);
        assert_eq!(unlimited[0].granted, 3.0);
        let shared = resolve_claims(&claims, Resolution::Shared(ShareRule::Proportional), |_| 3.0);
        assert_eq!(shared.iter().map(|a| a.granted).collect::<Vec<_>>(), vec![1.5, 1.5]);
    }

    #[test]
    fn serde_round_trip() {
        let c = claim(3, 9, 0.5, 2.0).amount(1.25);
        assert_eq!(serde_json::from_str::<Claim>(&serde_json::to_string(&c).unwrap()).unwrap(), c);
        let r = Resolution::Shared(ShareRule::Ordered);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"shared":"ordered"}"#);
        assert_eq!(serde_json::from_str::<Resolution>(&json).unwrap(), r);
    }
}
