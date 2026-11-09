//! The three model families behind one rule type.

use serde::{Deserialize, Serialize};

use crate::error::ContinuousError;
use crate::flow::FlowLeniaRule;
use crate::lenia::LeniaRule;
use crate::smoothlife::SmoothLifeRule;

/// Model identifier of Lenia worlds.
pub const LENIA_ID: &str = "continuous.lenia";
/// Model identifier of SmoothLife worlds.
pub const SMOOTHLIFE_ID: &str = "continuous.smoothlife";
/// Model identifier of Flow-Lenia worlds.
pub const FLOW_LENIA_ID: &str = "continuous.flow_lenia";

/// Largest cell value an edit may write into a Flow-Lenia world (100 times
/// the papers' critical mass θ_A = 1; the dynamics themselves conserve mass
/// and never exceed the total).
pub const FLOW_EDIT_LIMIT: f32 = 100.0;

/// Which continuous model a world runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// Lenia / Expanded Lenia.
    Lenia,
    /// SmoothLife.
    SmoothLife,
    /// Flow-Lenia.
    FlowLenia,
}

impl ModelKind {
    /// Every kind.
    pub const ALL: [ModelKind; 3] = [Self::Lenia, Self::SmoothLife, Self::FlowLenia];

    /// Stable model identifier (`continuous.*`).
    pub fn model_id(self) -> &'static str {
        match self {
            Self::Lenia => LENIA_ID,
            Self::SmoothLife => SMOOTHLIFE_ID,
            Self::FlowLenia => FLOW_LENIA_ID,
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Lenia => "Lenia",
            Self::SmoothLife => "SmoothLife",
            Self::FlowLenia => "Flow-Lenia",
        }
    }
}

/// A rule of any of the three models.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Rule {
    /// Lenia / Expanded Lenia.
    Lenia(LeniaRule),
    /// SmoothLife.
    SmoothLife(SmoothLifeRule),
    /// Flow-Lenia.
    FlowLenia(FlowLeniaRule),
}

impl Rule {
    /// Model family.
    pub fn kind(&self) -> ModelKind {
        match self {
            Self::Lenia(_) => ModelKind::Lenia,
            Self::SmoothLife(_) => ModelKind::SmoothLife,
            Self::FlowLenia(_) => ModelKind::FlowLenia,
        }
    }

    /// Number of channels.
    pub fn channels(&self) -> usize {
        match self {
            Self::Lenia(r) => r.channels,
            Self::SmoothLife(_) => 1,
            Self::FlowLenia(r) => r.channels,
        }
    }

    /// Simulated time per tick.
    pub fn dt(&self) -> f64 {
        match self {
            Self::Lenia(r) => r.dt(),
            Self::SmoothLife(r) => match r.stepping {
                crate::smoothlife::SmoothLifeStepping::Discrete => 1.0,
                _ => r.dt,
            },
            Self::FlowLenia(r) => r.dt(),
        }
    }

    /// Upper bound of cell values (`None` for Flow-Lenia's unbounded mass).
    pub fn max_value(&self) -> Option<f32> {
        match self {
            Self::Lenia(_) | Self::SmoothLife(_) => Some(1.0),
            Self::FlowLenia(_) => None,
        }
    }

    /// Largest value that edits (brush, placement, random matter, scenario
    /// initialisation) may write into a cell: the state bound for Lenia and
    /// SmoothLife, [`FLOW_EDIT_LIMIT`] for Flow-Lenia, whose dynamics leave
    /// mass unbounded but whose sums must stay finite in `f32`.
    pub fn edit_limit(&self) -> f32 {
        self.max_value().unwrap_or(FLOW_EDIT_LIMIT)
    }

    /// Validates the rule.
    pub fn validate(&self) -> Result<(), ContinuousError> {
        match self {
            Self::Lenia(r) => r.validate(),
            Self::SmoothLife(r) => r.validate(),
            Self::FlowLenia(r) => r.validate(),
        }
    }
}
