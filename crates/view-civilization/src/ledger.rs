//! Observations the model does not publish directly, recorded by the view's
//! session wrapper between ticks: **who traded with whom** (for the trade
//! network) and **recent activity changes** of every person (for the
//! inspector).
//!
//! # Trade detection
//!
//! The model reports how many barter meetings succeeded each tick
//! (`TickStats::trades`) but not between whom. The pairs are recovered from
//! state the model exposes, using facts of its barter step
//! (`civilization::social::trade`):
//!
//! 1. the *initiator* of a meeting had the goal `Trade → agent` and was
//!    within reach of its partner; after the meeting its goal is cleared,
//!    while its `doing` (the activity performed during the tick) stays
//!    `Trade`;
//! 2. the learned selling stock (`Agent::sales`) of the seller of every good
//!    that changed hands rises by the amount sold, and it rises nowhere
//!    else; stocks fade by 3 % at the start of each day (reproduced here) and
//!    are capped at 12, so a sale by a person already at the cap is hidden;
//! 3. partners are matched one-to-one, so a person takes part in at most one
//!    meeting per tick, and in one meeting a good flows one way only;
//! 4. what one partner sold, the other received (its inventory grows by
//!    about that amount; eating, spoilage and work blur this slightly).
//!
//! Every person whose selling stock rose therefore traded this tick. The
//! initiator's partner is its goal target before the tick unless it chose a
//! new goal during the tick (its `next_decision` changed). The remaining
//! sellers are paired greedily, at most as many meetings as the model
//! reported, with people within reach such that one of the two initiated,
//! both show a sale (or sit at the cap), they did not sell the same good,
//! and the goods flows match best. In a crowded market place two meetings
//! held at the same spot in the same hour can be cross-paired; a meeting in
//! which both partners sold only goods they already held at the cap is
//! missed. The view shows the detected count next to the model's own count,
//! and a test checks that both agree.

use std::collections::{BTreeMap, HashMap, VecDeque};

use civilization::Activity;
use civilization::groups::GroupId;
use organism_engine::AgentId;
use simulation_core::glam::Vec2;

/// Daily retention of learned selling stocks in the model (`life::SALES_RETENTION`).
pub const SALES_RETENTION: f32 = 0.97;
/// Cap of a learned selling stock in the model (`social::MAX_SALES_STOCK`).
pub const SALES_CAP: f32 = 12.0;
/// Reach within which two agents can barter (`navigate::AGENT_REACH`), plus a
/// margin for the movement that follows in the same tick.
pub const TRADE_REACH: f32 = 2.6;
/// Longest window of trades kept, in ticks (100 simulated days).
pub const MAX_WINDOW: u64 = 2400;
/// Activity changes remembered per person.
pub const ACTIVITY_HISTORY: usize = 10;

/// State of one person before a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreObs {
    /// Person.
    pub id: AgentId,
    /// Learned selling stock per good.
    pub sales: [f32; 4],
    /// Carried goods.
    pub inventory: [f32; 4],
    /// Partner of a `Trade → agent` goal.
    pub trade_target: Option<AgentId>,
    /// Tick of the next scheduled decision.
    pub next_decision: u64,
}

/// State of one person after a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostObs {
    /// Person.
    pub id: AgentId,
    /// Learned selling stock per good.
    pub sales: [f32; 4],
    /// Carried goods.
    pub inventory: [f32; 4],
    /// Activity performed during the tick.
    pub doing: Activity,
    /// Whether the goal is empty (cleared) after the tick.
    pub goal_cleared: bool,
    /// Tick of the next scheduled decision.
    pub next_decision: u64,
    /// Position after the tick (tiles).
    pub position: Vec2,
    /// Group after the tick.
    pub group: Option<GroupId>,
}

/// Rise of each learned selling stock during the tick (`daily`: the tick
/// started a new day, so stocks first faded by [`SALES_RETENTION`]); 0
/// where it did not rise.
pub fn sales_rise(pre: &[f32; 4], post: &[f32; 4], daily: bool) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (slot, (&before, &after)) in out.iter_mut().zip(pre.iter().zip(post)) {
        let expected = if daily { before * SALES_RETENTION } else { before };
        if after > expected + 1e-5 {
            *slot = after - expected;
        }
    }
    out
}

/// Whether a person's selling stock rose during the tick.
pub fn sold(pre: &[f32; 4], post: &[f32; 4], daily: bool) -> bool {
    sales_rise(pre, post, daily).iter().any(|&r| r > 0.0)
}

/// Evidence about one person in one tick.
#[derive(Clone, Copy, Debug)]
struct Evidence {
    /// Units sold per good: the selling-stock rise, or for a good held at
    /// the cap (rise hidden) the inventory loss.
    out: [f32; 4],
    /// Inventory gain per good.
    gain: [f32; 4],
    /// Goods with a visible selling-stock rise.
    rose: [bool; 4],
    seller: bool,
    capped: bool,
    initiator: bool,
}

/// Mismatch between what `x` sold and `y` received and vice versa.
fn flow_mismatch(x: &Evidence, y: &Evidence) -> f32 {
    (0..4).map(|g| (x.out[g] - y.gain[g]).abs() + (y.out[g] - x.gain[g]).abs()).sum()
}

/// Barter meetings `(initiator, partner)` of one tick, from observations
/// sorted by id and the number of meetings the model `reported` (see the
/// module documentation).
pub fn detect_trades(pre: &[PreObs], post: &[PostObs], daily: bool, reported: u32) -> Vec<(AgentId, AgentId)> {
    let pre_of = |id: AgentId| pre.binary_search_by_key(&id, |p| p.id).ok().map(|i| &pre[i]);
    let evidence: Vec<Evidence> = post
        .iter()
        .map(|p| {
            let (rise, gain, loss) = match pre_of(p.id) {
                Some(before) => {
                    let (mut gain, mut loss) = ([0.0; 4], [0.0; 4]);
                    for g in 0..4 {
                        let delta = p.inventory[g] - before.inventory[g];
                        gain[g] = delta.max(0.0);
                        loss[g] = (-delta).max(0.0);
                    }
                    (sales_rise(&before.sales, &p.sales, daily), gain, loss)
                }
                None => ([0.0; 4], [0.0; 4], [0.0; 4]),
            };
            let mut out = rise;
            for g in 0..4 {
                if rise[g] == 0.0 && p.sales[g] >= SALES_CAP - 1e-3 {
                    out[g] = loss[g];
                }
            }
            Evidence {
                out,
                gain,
                rose: rise.map(|r| r > 0.0),
                seller: rise.iter().any(|&r| r > 0.0),
                capped: p.sales.iter().any(|&v| v >= SALES_CAP - 1e-3),
                initiator: p.doing == Activity::Trade && p.goal_cleared && pre_of(p.id).is_some(),
            }
        })
        .collect();
    // Partners never both sold the same good in one meeting, and each of
    // them sold something (visibly, or hidden at the cap).
    let compatible = |i: usize, j: usize| {
        let (x, y) = (&evidence[i], &evidence[j]);
        let disjoint = (0..4).all(|g| !(x.rose[g] && y.rose[g]));
        disjoint && (x.seller || x.capped) && (y.seller || y.capped)
    };
    let known = |i: usize, j: usize| {
        let target_of = |k: usize| {
            pre_of(post[k].id).filter(|b| b.next_decision == post[k].next_decision).and_then(|b| b.trade_target)
        };
        target_of(i) == Some(post[j].id) || target_of(j) == Some(post[i].id)
    };
    // Candidate meetings: someone who sold (visibly or at the cap) and a
    // person within reach, one of them an initiator, ranked by how well the
    // goods flows match. Meetings without any visible sale need an almost
    // exact flow match.
    let sold_something = |k: usize| evidence[k].out.iter().sum::<f32>() > 0.04;
    let mut edges: Vec<(f32, usize, usize)> = Vec::new();
    for i in (0..post.len()).filter(|&i| sold_something(i)) {
        for j in 0..post.len() {
            if j == i || (sold_something(j) && j < i) || !(evidence[i].initiator || evidence[j].initiator) {
                continue;
            }
            let distance = post[i].position.distance(post[j].position);
            if distance > TRADE_REACH || !compatible(i, j) {
                continue;
            }
            let mismatch = flow_mismatch(&evidence[i], &evidence[j]);
            let visible = u8::from(evidence[i].seller) + u8::from(evidence[j].seller);
            if visible == 0 && (mismatch > 0.2 || !sold_something(j)) {
                continue;
            }
            let hidden = match visible {
                2 => 0.0,
                1 => 0.3,
                _ => 0.6,
            };
            let cost = mismatch + 0.05 * distance + hidden - if known(i, j) { 0.2 } else { 0.0 };
            edges.push((cost, i.min(j), i.max(j)));
        }
    }
    edges.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let mut used = vec![false; post.len()];
    let mut pairs = Vec::new();
    for (_, i, j) in edges {
        if pairs.len() >= reported as usize {
            break;
        }
        if used[i] || used[j] {
            continue;
        }
        used[i] = true;
        used[j] = true;
        let (a, b) = if evidence[i].initiator { (i, j) } else { (j, i) };
        pairs.push((post[a].id, post[b].id));
    }
    pairs
}

/// One detected barter meeting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TradeRecord {
    /// Tick during which it happened.
    pub tick: u64,
    /// Initiator.
    pub a: AgentId,
    /// Partner.
    pub b: AgentId,
    /// Group of the initiator at the time.
    pub group_a: Option<GroupId>,
    /// Group of the partner at the time.
    pub group_b: Option<GroupId>,
    /// Place (initiator's position, tiles).
    pub at: Vec2,
}

/// Recent observations kept by the session.
#[derive(Clone, Debug, Default)]
pub struct Ledger {
    trades: VecDeque<TradeRecord>,
    /// `(tick, meetings reported by the model)` for ticks with meetings.
    model_trades: VecDeque<(u64, u32)>,
    activity: HashMap<AgentId, VecDeque<(u64, Activity)>>,
}

impl Ledger {
    /// Forgets everything (the run jumped in time).
    pub fn clear(&mut self) {
        self.trades.clear();
        self.model_trades.clear();
        self.activity.clear();
    }

    /// Records the meetings of tick `tick` and the model's own count.
    pub fn record_trades(&mut self, tick: u64, pairs: &[(AgentId, AgentId)], post: &[PostObs], model_count: u32) {
        let post_of = |id: AgentId| post.binary_search_by_key(&id, |p| p.id).ok().map(|i| &post[i]);
        for &(a, b) in pairs {
            let (pa, pb) = (post_of(a), post_of(b));
            self.trades.push_back(TradeRecord {
                tick,
                a,
                b,
                group_a: pa.and_then(|p| p.group),
                group_b: pb.and_then(|p| p.group),
                at: pa.map_or(Vec2::ZERO, |p| p.position),
            });
        }
        if model_count > 0 {
            self.model_trades.push_back((tick, model_count));
        }
        let oldest = tick.saturating_sub(MAX_WINDOW);
        while self.trades.front().is_some_and(|t| t.tick < oldest) {
            self.trades.pop_front();
        }
        while self.model_trades.front().is_some_and(|t| t.0 < oldest) {
            self.model_trades.pop_front();
        }
    }

    /// Notes the activity of every person after tick `tick` (only changes
    /// are stored) and forgets people that no longer exist.
    pub fn record_activity(&mut self, tick: u64, post: &[PostObs]) {
        for p in post {
            let log = self.activity.entry(p.id).or_default();
            if log.back().is_none_or(|(_, last)| *last != p.doing) {
                log.push_back((tick, p.doing));
                while log.len() > ACTIVITY_HISTORY {
                    log.pop_front();
                }
            }
        }
        if self.activity.len() > post.len() {
            self.activity.retain(|id, _| post.binary_search_by_key(id, |p| p.id).is_ok());
        }
    }

    /// Activity changes of `id`, most recent first.
    pub fn activity_of(&self, id: AgentId) -> Vec<(u64, Activity)> {
        self.activity.get(&id).map(|log| log.iter().rev().copied().collect()).unwrap_or_default()
    }

    /// Meetings in the last `window` ticks before `now` (exclusive of older).
    pub fn trades_since(&self, now: u64, window: u64) -> impl Iterator<Item = &TradeRecord> {
        let start = now.saturating_sub(window.min(MAX_WINDOW));
        self.trades.iter().filter(move |t| t.tick >= start)
    }

    /// Meetings reported by the model in the last `window` ticks.
    pub fn model_trades_since(&self, now: u64, window: u64) -> u64 {
        let start = now.saturating_sub(window.min(MAX_WINDOW));
        self.model_trades.iter().filter(|t| t.0 >= start).map(|t| u64::from(t.1)).sum()
    }

    /// Approximate heap use.
    pub fn memory_bytes(&self) -> usize {
        self.trades.len() * std::mem::size_of::<TradeRecord>()
            + self.model_trades.len() * 16
            + self.activity.len() * (ACTIVITY_HISTORY * 16 + 48)
    }

    /// Summary of the meetings in a window.
    pub fn summary(&self, now: u64, window: u64) -> TradeSummary {
        let mut agent_edges: BTreeMap<(AgentId, AgentId), u32> = BTreeMap::new();
        let mut group_edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        let mut detected = 0u64;
        for t in self.trades_since(now, window) {
            detected += 1;
            let key = if t.a <= t.b { (t.a, t.b) } else { (t.b, t.a) };
            *agent_edges.entry(key).or_default() += 1;
            let (ga, gb) = (t.group_a.map_or(0, |g| g.0), t.group_b.map_or(0, |g| g.0));
            let gkey = if ga <= gb { (ga, gb) } else { (gb, ga) };
            *group_edges.entry(gkey).or_default() += 1;
        }
        TradeSummary {
            window: window.min(MAX_WINDOW),
            detected,
            reported: self.model_trades_since(now, window),
            agent_edges: agent_edges.into_iter().map(|((a, b), n)| (a, b, n)).collect(),
            group_edges: group_edges.into_iter().map(|((a, b), n)| (a, b, n)).collect(),
        }
    }
}

/// Trade network over a window of ticks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TradeSummary {
    /// Window length in ticks.
    pub window: u64,
    /// Meetings detected by the view.
    pub detected: u64,
    /// Meetings reported by the model (`trades` metric) in the same window.
    pub reported: u64,
    /// Meetings per pair of people (`a < b`).
    pub agent_edges: Vec<(AgentId, AgentId, u32)>,
    /// Meetings per pair of groups (`0` = no group; `a ≤ b`, `a == b` is trade inside a group).
    pub group_edges: Vec<(u32, u32, u32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(id: u64, sales: [f32; 4], target: Option<u64>, next: u64) -> PreObs {
        PreObs { id: AgentId(id), sales, inventory: [0.0; 4], trade_target: target.map(AgentId), next_decision: next }
    }

    fn post(id: u64, sales: [f32; 4], doing: Activity, cleared: bool, next: u64, x: f32) -> PostObs {
        PostObs {
            id: AgentId(id),
            sales,
            inventory: [0.0; 4],
            doing,
            goal_cleared: cleared,
            next_decision: next,
            position: Vec2::new(x, 0.0),
            group: None,
        }
    }

    const FOOD: [f32; 4] = [0.5, 0.0, 0.0, 0.0];
    const WOOD: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

    #[test]
    fn selling_stock_rises_are_detected_through_daily_fading() {
        assert!(sold(&[1.0, 0.0, 0.0, 0.0], &[1.5, 0.0, 0.0, 0.0], false));
        assert!(!sold(&[1.0, 0.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0], false));
        assert!(!sold(&[1.0, 0.0, 0.0, 0.0], &[0.97, 0.0, 0.0, 0.0], true), "fading alone is no sale");
        assert!(sold(&[1.0, 0.0, 0.0, 0.0], &[1.2, 0.0, 0.0, 0.0], true));
        let rise = sales_rise(&[1.0, 2.0, 0.0, 0.0], &[1.5, 2.0, 0.0, 0.0], false);
        assert!((rise[0] - 0.5).abs() < 1e-6 && rise[1] == 0.0);
    }

    #[test]
    fn known_partner_with_a_sale_is_paired() {
        let s0 = [0.0; 4];
        let pre_obs = [pre(1, s0, Some(2), 10), pre(2, s0, None, 10), pre(3, s0, None, 10)];
        let post_obs = [
            post(1, FOOD, Activity::Trade, true, 10, 0.0),
            post(2, WOOD, Activity::GatherFood, false, 10, 1.0),
            post(3, s0, Activity::Idle, true, 10, 0.5),
        ];
        assert_eq!(detect_trades(&pre_obs, &post_obs, false, 1), vec![(AgentId(1), AgentId(2))]);
        assert!(detect_trades(&pre_obs, &post_obs, false, 0).is_empty(), "never more than the model reported");
    }

    #[test]
    fn failed_meetings_and_far_partners_are_ignored() {
        let s0 = [0.0; 4];
        let pre_obs = [pre(1, s0, Some(2), 10), pre(2, s0, None, 10)];
        // No sale on either side: the meeting found no mutually beneficial exchange.
        let none = [post(1, s0, Activity::Trade, true, 10, 0.0), post(2, s0, Activity::Rest, false, 10, 1.0)];
        assert!(detect_trades(&pre_obs, &none, false, 1).is_empty());
        // A partner out of reach cannot have traded.
        let far = [post(1, FOOD, Activity::Trade, true, 10, 0.0), post(2, WOOD, Activity::Rest, false, 10, 9.0)];
        assert!(detect_trades(&pre_obs, &far, false, 1).is_empty());
        // Both sold the same good: they cannot have been partners.
        let same = [post(1, FOOD, Activity::Trade, true, 10, 0.0), post(2, FOOD, Activity::Rest, false, 10, 1.0)];
        assert!(detect_trades(&pre_obs, &same, false, 1).is_empty());
    }

    #[test]
    fn new_decisions_are_paired_by_evidence_and_people_meet_once() {
        let s0 = [0.0; 4];
        let pre_obs = [pre(1, s0, Some(9), 10), pre(2, s0, None, 10), pre(3, s0, None, 10), pre(4, s0, Some(2), 10)];
        let post_obs = [
            // Decided anew this tick (next_decision changed): target unknown.
            post(1, FOOD, Activity::Trade, true, 30, 0.0),
            post(2, FOOD, Activity::Idle, false, 10, 3.0),
            post(3, WOOD, Activity::Idle, false, 10, 1.0),
            // Agent 4's known partner (2) is free.
            post(4, WOOD, Activity::Trade, true, 10, 3.5),
        ];
        let pairs = detect_trades(&pre_obs, &post_obs, false, 2);
        assert_eq!(pairs, vec![(AgentId(4), AgentId(2)), (AgentId(1), AgentId(3))]);
    }

    #[test]
    fn capped_partners_and_goods_flows() {
        let s0 = [0.0; 4];
        let capped = [SALES_CAP, 0.0, 0.0, 0.0];
        let pre_obs = [
            PreObs {
                id: AgentId(1),
                sales: capped,
                inventory: [6.0, 0.0, 0.0, 0.0],
                trade_target: None,
                next_decision: 1,
            },
            PreObs { id: AgentId(2), sales: s0, inventory: [0.0, 4.0, 0.0, 0.0], trade_target: None, next_decision: 1 },
            PreObs { id: AgentId(3), sales: s0, inventory: [0.0, 4.0, 0.0, 0.0], trade_target: None, next_decision: 1 },
        ];
        let mut a = post(1, capped, Activity::Trade, true, 9, 0.0);
        a.inventory = [4.0, 2.0, 0.0, 0.0];
        // Person 2 sold 2 wood and received 2 food: matches the initiator.
        let mut b = post(2, [0.0, 2.0, 0.0, 0.0], Activity::Rest, false, 1, 1.5);
        b.inventory = [2.0, 2.0, 0.0, 0.0];
        // Person 3 sold 1 wood but received nothing from person 1 (closer, worse flow).
        let mut c = post(3, [0.0, 1.0, 0.0, 0.0], Activity::Rest, false, 1, 0.5);
        c.inventory = [0.0, 3.0, 0.0, 0.0];
        let pairs = detect_trades(&pre_obs, &[a, b, c], false, 1);
        assert_eq!(pairs, vec![(AgentId(1), AgentId(2))], "the capped initiator's hidden sale is matched by flows");
    }

    #[test]
    fn ledger_windows_and_summaries() {
        let mut ledger = Ledger::default();
        let mk = |id: u64, g: Option<u32>| PostObs {
            group: g.map(GroupId),
            ..post(id, [0.0; 4], Activity::Trade, true, 0, 0.0)
        };
        let obs = [mk(1, Some(1)), mk(2, Some(2)), mk(3, None)];
        ledger.record_trades(5, &[(AgentId(1), AgentId(2))], &obs, 1);
        ledger.record_trades(6, &[(AgentId(2), AgentId(1)), (AgentId(3), AgentId(1))], &obs, 3);
        let s = ledger.summary(6, 10);
        assert_eq!((s.detected, s.reported), (3, 4));
        assert_eq!(s.agent_edges, vec![(AgentId(1), AgentId(2), 2), (AgentId(1), AgentId(3), 1)]);
        assert_eq!(s.group_edges, vec![(0, 1, 1), (1, 2, 2)]);
        let recent = ledger.summary(6, 0);
        assert_eq!(recent.detected, 2, "a zero window still covers the current tick");
        ledger.record_trades(6 + MAX_WINDOW + 1, &[], &obs, 0);
        assert_eq!(ledger.summary(6 + MAX_WINDOW + 1, MAX_WINDOW).detected, 0, "old meetings expire");
    }

    #[test]
    fn activity_log_keeps_changes_only() {
        let mut ledger = Ledger::default();
        let mut p = post(1, [0.0; 4], Activity::Rest, false, 0, 0.0);
        ledger.record_activity(1, &[p]);
        ledger.record_activity(2, &[p]);
        p.doing = Activity::Eat;
        ledger.record_activity(3, &[p]);
        assert_eq!(ledger.activity_of(AgentId(1)), vec![(3, Activity::Eat), (1, Activity::Rest)]);
        for t in 0..30u64 {
            p.doing = if t % 2 == 0 { Activity::Trade } else { Activity::Idle };
            ledger.record_activity(10 + t, &[p]);
        }
        assert_eq!(ledger.activity_of(AgentId(1)).len(), ACTIVITY_HISTORY);
        ledger.record_activity(50, &[]);
        assert!(ledger.activity_of(AgentId(1)).is_empty(), "dead people are forgotten");
    }
}
