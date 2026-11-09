//! Per-tick measurements published by simulations.

use serde::{Deserialize, Serialize};

/// How a metric should be interpreted and aggregated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    /// Instantaneous value (population, biomass, temperature...).
    #[default]
    Gauge,
    /// Number of events during the last tick (births, deaths...).
    Counter,
    /// Dimensionless ratio in `[0, 1]` (density, diversity index...).
    Ratio,
}

/// Declaration of a metric a model reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSpec {
    /// Stable key used in exports (CSV column name).
    pub key: String,
    /// Label for user interfaces.
    pub label: String,
    /// Unit (may be empty).
    pub unit: String,
    /// Explanation shown in tooltips and the metrics guide.
    pub description: String,
    /// Interpretation.
    pub kind: MetricKind,
}

impl MetricSpec {
    /// Gauge metric.
    pub fn gauge(key: &str, label: &str, unit: &str, description: &str) -> Self {
        Self::new(key, label, unit, description, MetricKind::Gauge)
    }

    /// Counter metric.
    pub fn counter(key: &str, label: &str, description: &str) -> Self {
        Self::new(key, label, "per tick", description, MetricKind::Counter)
    }

    /// Ratio metric.
    pub fn ratio(key: &str, label: &str, description: &str) -> Self {
        Self::new(key, label, "", description, MetricKind::Ratio)
    }

    fn new(key: &str, label: &str, unit: &str, description: &str, kind: MetricKind) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            unit: unit.to_string(),
            description: description.to_string(),
            kind,
        }
    }
}

/// Values measured at one tick, in a stable order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Tick (generation) the values belong to.
    pub tick: u64,
    /// Simulated time (tick × dt) in model units.
    pub sim_time: f64,
    /// Ordered `(key, value)` pairs.
    pub values: Vec<(String, f64)>,
}

impl Metrics {
    /// Empty measurement for a tick.
    pub fn new(tick: u64, sim_time: f64) -> Self {
        Self { tick, sim_time, values: Vec::new() }
    }

    /// Appends (or replaces) a value.
    pub fn push(&mut self, key: &str, value: f64) -> &mut Self {
        if let Some(slot) = self.values.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        } else {
            self.values.push((key.to_string(), value));
        }
        self
    }

    /// Builder-style [`Metrics::push`].
    pub fn with(mut self, key: &str, value: f64) -> Self {
        self.push(key, value);
        self
    }

    /// Looks up a value.
    pub fn get(&self, key: &str) -> Option<f64> {
        self.values.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
    }

    /// Iterates over keys in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.values.iter().map(|(k, _)| k.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_replaces_and_preserves_order() {
        let mut m = Metrics::new(3, 3.0);
        m.push("population", 10.0).push("births", 2.0).push("population", 12.0);
        assert_eq!(m.get("population"), Some(12.0));
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["population", "births"]);
    }
}
