//! Formal, machine-readable description of a simulation model.

use serde::{Deserialize, Serialize};

use crate::metrics::MetricSpec;
use crate::params::ParamSpec;

/// Broad classification of a model, used for grouping and for choosing
/// sensible defaults in tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    /// Discrete cells with discrete states updated synchronously.
    Cellular,
    /// A cellular construction interpreted at a higher scale (metacells).
    MetaCellular,
    /// Continuous-state fields (Lenia, SmoothLife).
    Continuous,
    /// Three-dimensional cellular automata.
    Voxel,
    /// Individual agents with their own state and decisions.
    AgentBased,
    /// Agents coupled with cellular automata and continuous fields.
    Hybrid,
}

impl ModelFamily {
    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cellular => "Cellular automaton",
            Self::MetaCellular => "Meta-cellular construction",
            Self::Continuous => "Continuous field",
            Self::Voxel => "3D cellular automaton",
            Self::AgentBased => "Agent-based model",
            Self::Hybrid => "Hybrid agent / field model",
        }
    }
}

/// Self-description published by every model: identity, the formal rules it
/// executes, its parameters and its metrics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Stable identifier, e.g. `"life.conway"` or `"ecosystem"`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Classification.
    pub family: ModelFamily,
    /// One-paragraph summary.
    pub summary: String,
    /// Formal statement of the rules actually executed (Markdown). This text
    /// is shown in the rule panel and must match the implementation.
    pub rules: String,
    /// Declared parameters.
    pub params: Vec<ParamSpec>,
    /// Declared metrics.
    pub metrics: Vec<MetricSpec>,
    /// Unit of one tick ("generation", "step", "hour"...).
    pub tick_unit: String,
    /// Simulated time per tick in `tick_unit`s (1.0 for discrete automata).
    pub dt: f64,
    /// Whether runs are bit-for-bit reproducible for a fixed seed and inputs.
    pub deterministic: bool,
}

impl ModelDescriptor {
    /// Starts a descriptor with the mandatory fields.
    pub fn new(id: &str, name: &str, family: ModelFamily) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            family,
            summary: String::new(),
            rules: String::new(),
            params: Vec::new(),
            metrics: Vec::new(),
            tick_unit: "generation".to_string(),
            dt: 1.0,
            deterministic: true,
        }
    }

    /// Sets the summary.
    pub fn summary(mut self, summary: &str) -> Self {
        self.summary = summary.to_string();
        self
    }

    /// Sets the formal rule text.
    pub fn rules(mut self, rules: &str) -> Self {
        self.rules = rules.to_string();
        self
    }

    /// Sets the parameter list.
    pub fn params(mut self, params: Vec<ParamSpec>) -> Self {
        self.params = params;
        self
    }

    /// Sets the metric list.
    pub fn metrics(mut self, metrics: Vec<MetricSpec>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Sets the tick unit and time step.
    pub fn time(mut self, tick_unit: &str, dt: f64) -> Self {
        self.tick_unit = tick_unit.to_string();
        self.dt = dt;
        self
    }

    /// Looks up a parameter spec.
    pub fn param(&self, key: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.key == key)
    }
}
