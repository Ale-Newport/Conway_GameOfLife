//! Goods, welfare and Sugarscape-style bilateral barter.
//!
//! Agents hold four goods ([`Good`]). Each agent values its holdings with a
//! Cobb–Douglas welfare function (Epstein & Axtell 1996, generalised from
//! sugar and spice to `n` goods, with a small subsistence shift `ε` so that
//! empty holdings stay finite):
//!
//! ```text
//! W(h) = Π_g (h_g + ε)^{m_g / m_T},   m_T = Σ_g m_g
//! ```
//!
//! where `m_g ≥ 0` is the agent's current demand weight for good `g`
//! (metabolic need for food, building and tool plans for materials). The
//! marginal rate of substitution of good `a` for good `b` is
//!
//! ```text
//! MRS_ab = (∂W/∂h_a) / (∂W/∂h_b) = (m_a / (h_a + ε)) / (m_b / (h_b + ε)),
//! ```
//!
//! i.e. how many units of `b` the agent would give for one unit of `a`.
//! Trade (Sugarscape rule T): when two agents' MRSs differ, good `a` flows to
//! the agent with the higher MRS and good `b` the other way at the price
//! `p = √(MRS_A · MRS_B)` (units of `b` per unit of `a`). A step exchanges one
//! quantum of `a` for `p` quanta of `b` if `p ≥ 1`, otherwise `1/p` quanta of
//! `a` for one quantum of `b`, and is executed only if **both welfares rise and
//! the MRSs do not cross**; steps repeat until that fails.

use serde::{Deserialize, Serialize};

/// Subsistence shift `ε` of the welfare function (units of each good).
pub const SUBSISTENCE: f64 = 1.0;
/// Size of one exchange step (units of the cheaper side).
pub const TRADE_QUANTUM: f32 = 0.5;
/// Smallest exchange worth executing.
const MIN_EXCHANGE: f32 = 0.05;

/// A tradeable good.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Good {
    /// Edible food (berries, fish, crops).
    Food,
    /// Timber.
    Wood,
    /// Stone.
    Stone,
    /// Tools, measured in fresh-tool units (wear removes fractions).
    Tools,
}

impl Good {
    /// All goods in index order.
    pub const ALL: [Good; 4] = [Self::Food, Self::Wood, Self::Stone, Self::Tools];

    /// Index into a [`Bundle`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Food => "food",
            Self::Wood => "wood",
            Self::Stone => "stone",
            Self::Tools => "tools",
        }
    }
}

/// Amounts of every good.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bundle(pub [f32; 4]);

impl Bundle {
    /// Empty bundle.
    pub const ZERO: Bundle = Bundle([0.0; 4]);

    /// Bundle with the given amounts.
    pub fn new(food: f32, wood: f32, stone: f32, tools: f32) -> Self {
        Self([food, wood, stone, tools])
    }

    /// Amount of `good`.
    pub fn get(&self, good: Good) -> f32 {
        self.0[good.index()]
    }

    /// Sets the amount of `good`.
    pub fn set(&mut self, good: Good, amount: f32) {
        self.0[good.index()] = amount;
    }

    /// Adds `amount` (may be negative; the result is clamped at 0).
    pub fn add(&mut self, good: Good, amount: f32) {
        let slot = &mut self.0[good.index()];
        *slot = (*slot + amount).max(0.0);
    }

    /// Sum of all amounts.
    pub fn total(&self) -> f32 {
        self.0.iter().sum()
    }

    /// Whether every amount of `other` is available here.
    pub fn covers(&self, other: &Bundle) -> bool {
        self.0.iter().zip(other.0.iter()).all(|(have, need)| have + 1e-6 >= *need)
    }

    /// Subtracts `other` (clamped at 0).
    pub fn subtract(&mut self, other: &Bundle) {
        for (have, take) in self.0.iter_mut().zip(other.0.iter()) {
            *have = (*have - take).max(0.0);
        }
    }
}

/// Cobb–Douglas welfare `Π (h_g + ε)^{m_g/m_T}` (1 when all weights are 0).
pub fn welfare(holdings: &Bundle, weights: &Bundle) -> f64 {
    let total: f64 = weights.0.iter().map(|&m| f64::from(m.max(0.0))).sum();
    if total <= 0.0 {
        return 1.0;
    }
    weights
        .0
        .iter()
        .zip(holdings.0.iter())
        .map(|(&m, &h)| (f64::from(h.max(0.0)) + SUBSISTENCE).powf(f64::from(m.max(0.0)) / total))
        .product()
}

/// Logarithm of the welfare, `ln W = Σ (m_g/m_T)·ln(h_g + ε)` (0 when all
/// weights are 0). Comparing `ln W` is equivalent to comparing `W` and is
/// cheaper and better conditioned.
pub fn ln_welfare(holdings: &Bundle, weights: &Bundle) -> f64 {
    let total: f64 = weights.0.iter().map(|&m| f64::from(m.max(0.0))).sum();
    if total <= 0.0 {
        return 0.0;
    }
    weights
        .0
        .iter()
        .zip(holdings.0.iter())
        .map(|(&m, &h)| f64::from(m.max(0.0)) / total * (f64::from(h.max(0.0)) + SUBSISTENCE).ln())
        .sum()
}

/// Change of `ln W` when the holding of `good` changes from `h` by `delta`
/// (the result is clamped at 0 like [`Bundle::add`]); exact, using `ln_1p`.
fn ln_welfare_change(holdings: &Bundle, weights: &Bundle, total: f64, good: Good, delta: f32) -> f64 {
    let m = f64::from(weights.get(good).max(0.0));
    if m <= 0.0 || total <= 0.0 {
        return 0.0;
    }
    let base = f64::from(holdings.get(good).max(0.0)) + SUBSISTENCE;
    let after = (f64::from(holdings.get(good).max(0.0)) + f64::from(delta)).max(0.0) + SUBSISTENCE;
    m / total * ((after - base) / base).ln_1p()
}

/// Absolute need for one more unit of each good, in `[0, 1]`:
///
/// ```text
/// need_g = (m_g / (h_g + ε)) / max_k m_k
/// ```
///
/// This is the marginal welfare `∂ln W/∂h_g = (m_g/m_T)/(h_g + ε)` divided by
/// its largest possible value (the most-wanted good held at zero), so it
/// falls as holdings grow (diminishing marginal utility) and an agent that
/// holds plenty of everything has no strong reason to gather more. Unlike
/// shares relative to the currently most-wanted good, it saturates, which
/// bounds hoarding. All zeros when every weight is 0.
pub fn marginal_need(holdings: &Bundle, weights: &Bundle) -> Bundle {
    let top = weights.0.iter().fold(0.0f64, |acc, &m| acc.max(f64::from(m)));
    let mut out = Bundle::ZERO;
    if top > 0.0 {
        for ((slot, &m), &h) in out.0.iter_mut().zip(weights.0.iter()).zip(holdings.0.iter()) {
            let mu = f64::from(m.max(0.0)) / (f64::from(h.max(0.0)) + SUBSISTENCE);
            *slot = (mu / top) as f32;
        }
    }
    out
}

/// `MRS_ab`: units of `b` the holder would give for one unit of `a`.
/// Returns `None` when either weight is zero (the good has no value).
pub fn mrs(holdings: &Bundle, weights: &Bundle, a: Good, b: Good) -> Option<f64> {
    let (ma, mb) = (f64::from(weights.get(a)), f64::from(weights.get(b)));
    if ma <= 0.0 || mb <= 0.0 {
        return None;
    }
    let ha = f64::from(holdings.get(a).max(0.0)) + SUBSISTENCE;
    let hb = f64::from(holdings.get(b).max(0.0)) + SUBSISTENCE;
    Some((ma / ha) / (mb / hb))
}

/// One side of a trade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trader<'a> {
    /// Holdings.
    pub holdings: &'a Bundle,
    /// Demand weights.
    pub weights: &'a Bundle,
}

/// Result of [`barter`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TradeOutcome {
    /// Executed exchange steps.
    pub steps: u32,
    /// Net change of the first trader's holdings (the second gets the negation).
    pub delta_a: Bundle,
    /// Total units that changed hands (both directions).
    pub volume: f32,
}

/// Every unordered pair of distinct goods, in a fixed order.
pub const GOOD_PAIRS: [(Good, Good); 6] = [
    (Good::Food, Good::Wood),
    (Good::Food, Good::Stone),
    (Good::Food, Good::Tools),
    (Good::Wood, Good::Stone),
    (Good::Wood, Good::Tools),
    (Good::Stone, Good::Tools),
];

/// One Sugarscape exchange step on the pair `(x, y)`. Returns the amounts
/// `(qx, qy)` the *buyer of x* receives and pays, or `None` if no mutually
/// beneficial, non-crossing step exists.
fn exchange_step(
    buyer: (&Bundle, &Bundle),
    seller: (&Bundle, &Bundle),
    x: Good,
    y: Good,
    quantum: f32,
) -> Option<(f32, f32)> {
    let mrs_buyer = mrs(buyer.0, buyer.1, x, y)?;
    let mrs_seller = mrs(seller.0, seller.1, x, y)?;
    if mrs_buyer <= mrs_seller * (1.0 + 1e-9) {
        return None;
    }
    let price = (mrs_buyer * mrs_seller).sqrt();
    let (mut qx, mut qy) = if price >= 1.0 {
        (quantum, (f64::from(quantum) * price) as f32)
    } else {
        ((f64::from(quantum) / price) as f32, quantum)
    };
    // Scale down to what the parties actually hold.
    let scale = 1.0f32.min(seller.0.get(x) / qx).min(buyer.0.get(y) / qy);
    qx *= scale;
    qy *= scale;
    if qx < MIN_EXCHANGE || qy < MIN_EXCHANGE {
        return None;
    }
    // Both welfares must rise: Δln W > 0 (only goods x and y change).
    let weight_sum = |w: &Bundle| w.0.iter().map(|&m| f64::from(m.max(0.0))).sum::<f64>();
    let (tb, ts) = (weight_sum(buyer.1), weight_sum(seller.1));
    let buyer_gain = ln_welfare_change(buyer.0, buyer.1, tb, x, qx) + ln_welfare_change(buyer.0, buyer.1, tb, y, -qy);
    let seller_gain =
        ln_welfare_change(seller.0, seller.1, ts, x, -qx) + ln_welfare_change(seller.0, seller.1, ts, y, qy);
    let mut buyer_after = *buyer.0;
    buyer_after.add(x, qx);
    buyer_after.add(y, -qy);
    let mut seller_after = *seller.0;
    seller_after.add(x, -qx);
    seller_after.add(y, qy);
    let gains = buyer_gain > 0.0 && seller_gain > 0.0;
    let crossed = mrs(&buyer_after, buyer.1, x, y)? < mrs(&seller_after, seller.1, x, y)?;
    (gains && !crossed).then_some((qx, qy))
}

/// Bilateral barter between `a` and `b` over every pair of goods, at most
/// `max_steps` exchange steps in total. Holdings are updated in place; the
/// outcome reports the net transfer and volume. Each executed step strictly
/// increases both welfares (see the module documentation).
pub fn barter(a: &mut Bundle, a_weights: &Bundle, b: &mut Bundle, b_weights: &Bundle, max_steps: u32) -> TradeOutcome {
    let start = *a;
    let mut outcome = TradeOutcome::default();
    'outer: while outcome.steps < max_steps {
        let mut progressed = false;
        for (x, y) in GOOD_PAIRS {
            if outcome.steps >= max_steps {
                break 'outer;
            }
            let (Some(mrs_a), Some(mrs_b)) = (mrs(a, a_weights, x, y), mrs(b, b_weights, x, y)) else {
                continue;
            };
            let a_buys = mrs_a > mrs_b;
            let step = if a_buys {
                exchange_step((&*a, a_weights), (&*b, b_weights), x, y, TRADE_QUANTUM)
            } else {
                exchange_step((&*b, b_weights), (&*a, a_weights), x, y, TRADE_QUANTUM)
            };
            let Some((qx, qy)) = step else { continue };
            let (buyer, seller) = if a_buys { (&mut *a, &mut *b) } else { (&mut *b, &mut *a) };
            buyer.add(x, qx);
            buyer.add(y, -qy);
            seller.add(x, -qx);
            seller.add(y, qy);
            outcome.steps += 1;
            outcome.volume += qx + qy;
            progressed = true;
        }
        if !progressed {
            break;
        }
    }
    for g in Good::ALL {
        outcome.delta_a.0[g.index()] = a.get(g) - start.get(g);
    }
    outcome
}

/// MRS gap in `[0, 1)`: `1 − exp(−max |ln MRS_A − ln MRS_B|)` over the pairs
/// of goods for which at least one mutually beneficial, non-crossing exchange
/// step exists. Cheap; agents use it to preselect partners (whose meetings
/// are then valued with [`expected_gain`]) and to rank meetings for matching.
/// It does not measure how much a meeting is worth.
pub fn trade_potential(a: Trader<'_>, b: Trader<'_>) -> f32 {
    let mut best = 0.0f64;
    for (x, y) in GOOD_PAIRS {
        let (Some(ra), Some(rb)) = (mrs(a.holdings, a.weights, x, y), mrs(b.holdings, b.weights, x, y)) else {
            continue;
        };
        let (buyer, seller) = if ra > rb { (a, b) } else { (b, a) };
        let feasible =
            exchange_step((buyer.holdings, buyer.weights), (seller.holdings, seller.weights), x, y, TRADE_QUANTUM)
                .is_some();
        if feasible {
            best = best.max((ra.ln() - rb.ln()).abs());
        }
    }
    (1.0 - (-best).exp()) as f32
}

/// Expected gain of a barter meeting for `a`: `ln W_a(after) − ln W_a(before)`
/// when [`barter`] runs between copies of both holdings with `max_steps`
/// steps (`b` gains too, by construction). This measures how much a meeting
/// is worth — unlike [`trade_potential`], it is small when the holdings that
/// could change hands are small.
pub fn expected_gain(a: Trader<'_>, b: Trader<'_>, max_steps: u32) -> f64 {
    let (mut ha, mut hb) = (*a.holdings, *b.holdings);
    let outcome = barter(&mut ha, a.weights, &mut hb, b.weights, max_steps);
    if outcome.steps == 0 {
        return 0.0;
    }
    (ln_welfare(&ha, a.weights) - ln_welfare(a.holdings, a.weights)).max(0.0)
}

/// Exchange steps allowed in one meeting: `4 + 8·mean trading skill`.
pub fn meeting_steps(skill_a: f32, skill_b: f32) -> u32 {
    4 + (8.0 * 0.5 * (skill_a + skill_b)).clamp(0.0, 8.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights() -> Bundle {
        Bundle::new(1.0, 0.6, 0.4, 0.2)
    }

    #[test]
    fn mrs_matches_sugarscape_definition() {
        let h = Bundle::new(3.0, 1.0, 0.0, 0.0);
        let m = Bundle::new(1.0, 0.5, 0.5, 0.5);
        // (m_food/(3+1)) / (m_wood/(1+1)) = 0.25 / 0.25 = 1
        assert!((mrs(&h, &m, Good::Food, Good::Wood).unwrap() - 1.0).abs() < 1e-12);
        assert!((mrs(&h, &m, Good::Wood, Good::Food).unwrap() - 1.0).abs() < 1e-12);
        let h = Bundle::new(1.0, 7.0, 0.0, 0.0);
        // food is scarce: (1/2)/(0.5/8) = 8 units of wood per unit of food
        assert!((mrs(&h, &m, Good::Food, Good::Wood).unwrap() - 8.0).abs() < 1e-12);
        assert_eq!(mrs(&h, &Bundle::new(1.0, 0.0, 0.0, 0.0), Good::Food, Good::Wood), None);
    }

    #[test]
    fn mrs_trade_increases_both_utilities() {
        // A is rich in food and poor in wood, B the opposite.
        let mut a = Bundle::new(12.0, 0.5, 2.0, 0.0);
        let mut b = Bundle::new(1.0, 10.0, 2.0, 0.0);
        let (wa, wb) = (weights(), Bundle::new(1.0, 0.5, 0.5, 0.2));
        let before = (welfare(&a, &wa), welfare(&b, &wb));
        let total_before: Vec<f32> = Good::ALL.iter().map(|&g| a.get(g) + b.get(g)).collect();
        let outcome = barter(&mut a, &wa, &mut b, &wb, 100);
        assert!(outcome.steps > 0, "complementary holdings must trade");
        assert!(welfare(&a, &wa) > before.0, "A gains");
        assert!(welfare(&b, &wb) > before.1, "B gains");
        assert!(outcome.delta_a.get(Good::Food) < 0.0 && outcome.delta_a.get(Good::Wood) > 0.0);
        // Goods are conserved.
        for (i, g) in Good::ALL.iter().enumerate() {
            assert!((a.get(*g) + b.get(*g) - total_before[i]).abs() < 1e-4);
        }
        // After trading the MRSs are close: no further mutually beneficial step.
        let again = barter(&mut a, &wa, &mut b, &wb, 100);
        assert_eq!(again.steps, 0);
        let (ra, rb) = (mrs(&a, &wa, Good::Food, Good::Wood).unwrap(), mrs(&b, &wb, Good::Food, Good::Wood).unwrap());
        assert!((ra.ln() - rb.ln()).abs() < 0.6, "MRSs converged: {ra} vs {rb}");
    }

    #[test]
    fn every_step_raises_both_welfares_and_never_crosses() {
        let mut a = Bundle::new(9.0, 0.0, 6.0, 1.0);
        let mut b = Bundle::new(0.5, 8.0, 0.0, 0.0);
        let (wa, wb) = (Bundle::new(1.0, 0.8, 0.3, 0.3), Bundle::new(1.2, 0.2, 0.7, 0.5));
        let mut last = (welfare(&a, &wa), welfare(&b, &wb));
        for _ in 0..50 {
            let o = barter(&mut a, &wa, &mut b, &wb, 1);
            if o.steps == 0 {
                break;
            }
            let now = (welfare(&a, &wa), welfare(&b, &wb));
            assert!(now.0 > last.0 && now.1 > last.1);
            last = now;
        }
        assert!(a.0.iter().chain(b.0.iter()).all(|&x| x >= 0.0));
    }

    #[test]
    fn identical_agents_do_not_trade_and_potential_reflects_gaps() {
        let mut a = Bundle::new(5.0, 5.0, 5.0, 1.0);
        let mut b = a;
        let w = weights();
        assert_eq!(barter(&mut a, &w, &mut b, &w, 10).steps, 0);
        assert_eq!(trade_potential(Trader { holdings: &a, weights: &w }, Trader { holdings: &b, weights: &w }), 0.0);
        let rich_wood = Bundle::new(0.0, 12.0, 0.0, 0.0);
        let rich_food = Bundle::new(12.0, 0.0, 0.0, 0.0);
        let p =
            trade_potential(Trader { holdings: &rich_wood, weights: &w }, Trader { holdings: &rich_food, weights: &w });
        assert!(p > 0.5, "{p}");
    }

    #[test]
    fn ln_welfare_matches_welfare_and_expected_gain_scales_with_holdings() {
        let (h, w) = (Bundle::new(3.0, 0.5, 7.0, 1.0), weights());
        assert!((ln_welfare(&h, &w) - welfare(&h, &w).ln()).abs() < 1e-12);
        let mut after = h;
        after.add(Good::Wood, 2.0);
        let total: f64 = w.0.iter().map(|&m| f64::from(m)).sum();
        let delta = ln_welfare_change(&h, &w, total, Good::Wood, 2.0);
        assert!((delta - (ln_welfare(&after, &w) - ln_welfare(&h, &w))).abs() < 1e-12);
        // Same MRS gap, very different stakes: crumbs are worth little.
        let wb = Bundle::new(1.0, 0.5, 0.5, 0.2);
        let rich = (Bundle::new(12.0, 0.0, 2.0, 0.0), Bundle::new(0.0, 10.0, 2.0, 0.0));
        let poor = (Bundle::new(0.6, 0.0, 0.1, 0.0), Bundle::new(0.0, 0.5, 0.1, 0.0));
        let gain = |pair: &(Bundle, Bundle)| {
            expected_gain(Trader { holdings: &pair.0, weights: &w }, Trader { holdings: &pair.1, weights: &wb }, 12)
        };
        assert!(gain(&rich) > 0.1, "{}", gain(&rich));
        assert!(gain(&poor) < 0.25 * gain(&rich), "{} vs {}", gain(&poor), gain(&rich));
        let same = Bundle::new(5.0, 5.0, 5.0, 1.0);
        let t = Trader { holdings: &same, weights: &w };
        assert_eq!(expected_gain(t, t, 12), 0.0);
        assert_eq!(meeting_steps(0.0, 0.0), 4);
        assert_eq!(meeting_steps(1.0, 1.0), 12);
    }

    #[test]
    fn marginal_need_is_absolute_and_diminishing() {
        let w = Bundle::new(1.0, 0.5, 0.25, 0.0);
        let empty = marginal_need(&Bundle::ZERO, &w);
        // Destitute: the most-wanted good scores 1, others their weight ratio.
        assert_eq!(empty, Bundle::new(1.0, 0.5, 0.25, 0.0));
        // (m/(h + 1))/m_max: holding 3 units of food quarters its need.
        let some = marginal_need(&Bundle::new(3.0, 1.0, 0.0, 9.0), &w);
        assert!((some.get(Good::Food) - 0.25).abs() < 1e-6);
        assert!((some.get(Good::Wood) - 0.25).abs() < 1e-6);
        assert!((some.get(Good::Stone) - 0.25).abs() < 1e-6);
        // Plenty of everything: every need is small (it saturates).
        let rich = marginal_need(&Bundle::new(99.0, 99.0, 99.0, 99.0), &w);
        assert!(rich.0.iter().all(|&n| n <= 0.01), "{rich:?}");
        assert_eq!(marginal_need(&Bundle::ZERO, &Bundle::ZERO), Bundle::ZERO);
    }
}
