//! Render data for user interfaces.

use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{SimEvent, Topology};

use crate::compile::RelationshipGraph;
use crate::compile::ir::RuleOrigin;
use crate::error::Diagnostic;
use crate::script::ScriptError;

/// What a renderer asks for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomLifeViewRequest {
    /// Include resource and environment field grids.
    pub include_fields: bool,
    /// Maximum samples per axis of each field grid (fields are
    /// downsampled by block averaging to at most this resolution).
    pub field_resolution: u32,
    /// Include the rule listing (text of every rule, generated ones too).
    pub include_rules: bool,
    /// Number of recent log events to include.
    pub recent_events: usize,
}

impl Default for CustomLifeViewRequest {
    fn default() -> Self {
        Self { include_fields: true, field_resolution: 128, include_rules: false, recent_events: 20 }
    }
}

/// One entity to draw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityView {
    /// Stable id.
    pub id: u64,
    /// Index into [`CustomLifeView::legend`].
    pub type_index: u16,
    /// Position in world units.
    pub position: Vec2,
    /// Heading in radians.
    pub heading: f32,
    /// Type-local state index (name in the legend entry's `states`).
    pub state: u16,
    /// Diameter.
    pub size: f32,
    /// Colour (the type's colour).
    pub color: [f32; 4],
    /// Energy relative to the type's maximum, in `[0, 1]`.
    pub energy: f32,
}

/// A downsampled field grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldView {
    /// Field name.
    pub name: String,
    /// Whether it is a consumable resource (otherwise an environment field).
    pub resource: bool,
    /// Samples per row.
    pub width: u32,
    /// Rows.
    pub height: u32,
    /// World units covered by one sample along x.
    pub cell_width: f32,
    /// World units covered by one sample along y.
    pub cell_height: f32,
    /// Row-major samples (block means).
    pub values: Vec<f32>,
    /// Smallest sample.
    pub min: f32,
    /// Largest sample.
    pub max: f32,
    /// Display colour (resources).
    pub color: [f32; 4],
}

/// Legend entry of an entity type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegendEntry {
    /// Type index.
    pub index: u16,
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Colour.
    pub color: [f32; 4],
    /// Glyph / emoji.
    pub icon: String,
    /// Sprite or model id.
    pub sprite: String,
    /// State names.
    pub states: Vec<String>,
    /// Live population.
    pub count: u32,
}

/// One rule in the rule list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleListing {
    /// Entity type name.
    pub entity: String,
    /// Rule name.
    pub name: String,
    /// Where it comes from.
    pub origin: RuleOrigin,
    /// Canonical DSL text.
    pub text: String,
    /// Firings since the last reset.
    pub fired_total: u64,
}

/// Owned render data of a custom life world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomLifeView {
    /// Tick.
    pub tick: u64,
    /// World width.
    pub width: u32,
    /// World height.
    pub height: u32,
    /// Topology.
    pub topology: Topology,
    /// Entities in id order.
    pub entities: Vec<EntityView>,
    /// Field grids (resources, then environment fields).
    pub fields: Vec<FieldView>,
    /// One entry per entity type.
    pub legend: Vec<LegendEntry>,
    /// Relationship graph (nodes and typed edges).
    pub graph: RelationshipGraph,
    /// Rule listing (when requested).
    pub rules: Vec<RuleListing>,
    /// Script failures (each disabled its script).
    pub script_errors: Vec<ScriptError>,
    /// Compilation warnings, rendered.
    pub warnings: Vec<String>,
    /// The same warnings as structured diagnostics (code, item, span, help).
    pub diagnostics: Vec<Diagnostic>,
    /// Whether an edit (definition, size, topology, seed...) is waiting for
    /// the next reset to take effect.
    pub reset_pending: bool,
    /// Why the run ended, if an end condition was met.
    pub finished: Option<String>,
    /// Most recent log events, newest first.
    pub recent_events: Vec<SimEvent>,
}

/// Block-mean downsampling of a `width × height` buffer to at most
/// `max_dim` samples per axis.
pub fn downsample(values: &[f32], width: usize, height: usize, max_dim: usize) -> (Vec<f32>, usize, usize) {
    let max_dim = max_dim.max(1);
    let fx = width.div_ceil(max_dim).max(1);
    let fy = height.div_ceil(max_dim).max(1);
    let (w, h) = (width.div_ceil(fx), height.div_ceil(fy));
    let mut out = vec![0.0f32; w * h];
    let mut counts = vec![0u32; w * h];
    for y in 0..height {
        for x in 0..width {
            let i = (y / fy) * w + x / fx;
            out[i] += values[y * width + x];
            counts[i] += 1;
        }
    }
    for (v, c) in out.iter_mut().zip(&counts) {
        if *c > 0 {
            *v /= *c as f32;
        }
    }
    (out, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsampling_preserves_means() {
        let values: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let (out, w, h) = downsample(&values, 4, 4, 2);
        assert_eq!((w, h), (2, 2));
        assert_eq!(out, vec![2.5, 4.5, 10.5, 12.5]);
        let (same, w, h) = downsample(&values, 4, 4, 8);
        assert_eq!((w, h), (4, 4));
        assert_eq!(same, values);
    }
}
