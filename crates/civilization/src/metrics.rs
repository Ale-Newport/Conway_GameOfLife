//! Metrics published every tick.
//!
//! * **Gini of wealth** over all agents (wealth = carried food + wood +
//!   stone + 2 × tools, plus the food stored in the huts the agent owns),
//!   computed with `analytics::gini`.
//! * **Specialisation index**: for every adult that has produced something,
//!   the mean of `1 − H(p)/ln K` over two distributions — its production
//!   shares (moving average over production ticks of foraging, logging,
//!   quarrying, farming, crafting and building, `K = 6`) and its normalised
//!   skill levels (gathering, farming, crafting, building, trading, `K = 5`)
//!   — where `H` is the Shannon entropy. 0 means a perfect generalist, 1 a
//!   pure specialist; the population value is the mean over those adults.
//! * **Road length**: tiles whose trail intensity reaches the road threshold.

use simulation_core::{MetricSpec, Metrics};

use crate::agent::Agent;
use crate::buildings::BuildingKind;
use crate::economy::Good;
use crate::state::CivState;

/// Metric declarations in publication order.
pub fn metric_specs() -> Vec<MetricSpec> {
    vec![
        MetricSpec::gauge("population", "Population", "agents", "Living agents."),
        MetricSpec::gauge("groups", "Groups", "groups", "Groups formed from mutual affinity."),
        MetricSpec::ratio("grouped", "Grouped share", "Share of agents that belong to a group."),
        MetricSpec::ratio(
            "gini_wealth",
            "Wealth Gini",
            "Gini coefficient of wealth (carried food + wood + stone + 2 × tools, plus food in owned huts).",
        ),
        MetricSpec::counter("trade_volume", "Trade volume", "Units of goods exchanged in barter this tick."),
        MetricSpec::counter("trades", "Trades", "Barter meetings with at least one exchange this tick."),
        MetricSpec::ratio(
            "specialization",
            "Specialisation index",
            "Mean 1 − normalised entropy of production shares and skills (adults).",
        ),
        MetricSpec::gauge(
            "territory_per_group",
            "Territory per group",
            "tiles",
            "Mean number of tiles claimed per group.",
        ),
        MetricSpec::gauge(
            "contested_tiles",
            "Contested tiles",
            "tiles",
            "Claimed tiles where another group's influence nearly matches.",
        ),
        MetricSpec::gauge(
            "road_length",
            "Road length",
            "tiles",
            "Tiles whose trail intensity reaches the road threshold.",
        ),
        MetricSpec::gauge(
            "food_stock",
            "Food stock",
            "units",
            "Food carried by agents plus food in group stores and household (hut) stores.",
        ),
        MetricSpec::gauge("wild_food", "Wild food", "units", "Berries and fish standing on the map."),
        MetricSpec::counter("conflicts", "Conflicts", "Fights this tick (resisted thefts and territorial defence)."),
        MetricSpec::counter("thefts", "Thefts", "Successful thefts this tick."),
        MetricSpec::counter(
            "shared_food",
            "Shared food",
            "Food given to others or deposited in group stores this tick.",
        ),
        MetricSpec::counter("births", "Births", "Children born this tick."),
        MetricSpec::counter("deaths", "Deaths", "Deaths this tick (all causes)."),
        MetricSpec::counter("deaths_starvation", "Deaths: starvation", "Deaths from starvation this tick."),
        MetricSpec::counter("deaths_thirst", "Deaths: thirst", "Deaths from thirst this tick."),
        MetricSpec::counter("deaths_conflict", "Deaths: conflict", "Deaths from fight injuries this tick."),
        MetricSpec::counter("deaths_age", "Deaths: old age", "Deaths from old age this tick."),
        MetricSpec::gauge("buildings", "Buildings", "buildings", "Finished buildings."),
        MetricSpec::gauge("farms", "Farm plots", "plots", "Finished farm plots."),
        MetricSpec::ratio("mean_energy", "Mean energy", "Average metabolic reserve."),
        MetricSpec::ratio("mean_skill", "Mean skill", "Average skill level over all skills and agents."),
    ]
}

/// `1 − H(p)/ln n` of a non-negative vector (`None` if it sums to zero).
pub fn concentration(values: &[f32]) -> Option<f64> {
    let total: f64 = values.iter().map(|&v| f64::from(v.max(0.0))).sum();
    if total <= 0.0 || values.len() < 2 {
        return None;
    }
    let entropy: f64 =
        values.iter().map(|&v| f64::from(v.max(0.0)) / total).filter(|&p| p > 0.0).map(|p| -p * p.ln()).sum();
    Some(1.0 - entropy / (values.len() as f64).ln())
}

/// Specialisation of one agent, if it has produced anything.
pub fn specialization(agent: &Agent) -> Option<f64> {
    let activity = concentration(&agent.practice)?;
    let skills = concentration(&agent.skills.0).unwrap_or(activity);
    Some(0.5 * (activity + skills))
}

/// Measures the current state.
pub(crate) fn measure(state: &CivState, sim_time: f64) -> Metrics {
    let agents = state.agents.as_slice();
    let n = agents.len();
    let mut m = Metrics::new(state.tick, sim_time);
    m.push("population", n as f64);
    m.push("groups", state.groups.len() as f64);
    let grouped = agents.iter().filter(|a| a.group.is_some()).count();
    m.push("grouped", if n > 0 { grouped as f64 / n as f64 } else { 0.0 });
    let mut wealth: Vec<f64> = agents.iter().map(|a| f64::from(a.wealth())).collect();
    for b in state.buildings.iter().filter(|b| b.kind == BuildingKind::Hut) {
        if let Some(slot) = b.owner.and_then(|o| state.agents.index_of(o)) {
            wealth[slot] += f64::from(b.store.get(Good::Food));
        }
    }
    m.push("gini_wealth", analytics::gini(&wealth).unwrap_or(0.0));
    m.push("trade_volume", f64::from(state.stats.trade_volume));
    m.push("trades", f64::from(state.stats.trades));
    let specs: Vec<f64> = agents.iter().filter(|a| a.is_adult()).filter_map(specialization).collect();
    m.push("specialization", if specs.is_empty() { 0.0 } else { specs.iter().sum::<f64>() / specs.len() as f64 });
    let groups = state.groups.len();
    let claimed: u64 = state.groups.iter().map(|g| u64::from(g.territory)).sum();
    m.push("territory_per_group", if groups > 0 { claimed as f64 / groups as f64 } else { 0.0 });
    m.push("contested_tiles", state.groups.iter().map(|g| f64::from(g.contested)).sum());
    let threshold = state.rules.road_threshold;
    let roads = (0..state.world.len() as u32).filter(|&t| state.world.road(t) >= threshold).count();
    m.push("road_length", roads as f64);
    let carried: f64 = agents.iter().map(|a| f64::from(a.inventory.get(Good::Food))).sum();
    let stored: f64 = state.buildings.iter().map(|b| f64::from(b.store.get(Good::Food))).sum();
    m.push("food_stock", carried + stored);
    m.push("wild_food", state.world.wild_food());
    let s = &state.stats;
    m.push("conflicts", f64::from(s.conflicts));
    m.push("thefts", f64::from(s.thefts));
    m.push("shared_food", f64::from(s.shared_food));
    m.push("births", f64::from(s.births));
    m.push("deaths", f64::from(s.deaths.iter().sum::<u32>()));
    m.push("deaths_starvation", f64::from(s.deaths[0]));
    m.push("deaths_thirst", f64::from(s.deaths[1]));
    m.push("deaths_conflict", f64::from(s.deaths[2]));
    m.push("deaths_age", f64::from(s.deaths[3]));
    let finished: Vec<_> = state.buildings.iter().filter(|b| b.is_complete()).collect();
    m.push("buildings", finished.len() as f64);
    m.push("farms", finished.iter().filter(|b| b.kind == BuildingKind::FarmPlot).count() as f64);
    let mean = |f: &dyn Fn(&Agent) -> f32| {
        if n == 0 { 0.0 } else { agents.iter().map(|a| f64::from(f(a))).sum::<f64>() / n as f64 }
    };
    m.push("mean_energy", mean(&|a| a.energy));
    m.push("mean_skill", mean(&|a| a.skills.0.iter().sum::<f32>() / 5.0));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concentration_spans_generalist_to_specialist() {
        assert!(concentration(&[1.0, 1.0, 1.0, 1.0, 1.0]).unwrap().abs() < 1e-12);
        assert!((concentration(&[0.0, 3.0, 0.0, 0.0, 0.0]).unwrap() - 1.0).abs() < 1e-12);
        let mid = concentration(&[1.0, 1.0, 0.0, 0.0, 0.0]).unwrap();
        assert!((mid - (1.0 - 2f64.ln() / 5f64.ln())).abs() < 1e-12);
        assert_eq!(concentration(&[0.0; 5]), None);
    }

    #[test]
    fn specs_match_measured_keys() {
        let keys: Vec<String> = metric_specs().into_iter().map(|s| s.key).collect();
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), keys.len());
    }
}
