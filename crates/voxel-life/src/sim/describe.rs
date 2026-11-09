//! Descriptor texts and metric declarations.

use simulation_core::MetricSpec;

use crate::geometry::WorldKind;
use crate::rule::{CountSet, Neighbourhood, Rule3D};

/// One-paragraph summary of the model.
pub(super) const SUMMARY: &str = "Three-dimensional outer-totalistic cellular automata on the cubic lattice \
     (Bays' 3D Life, Softology-style multi-state rules with Generations decay) with Moore-26 or von Neumann-6 \
     neighbourhoods, bounded, toroidal or unbounded sparse worlds, and sliceable surface views.";

fn list(set: CountSet) -> String {
    if set.is_empty() {
        "∅".to_string()
    } else {
        format!("{{{}}}", set.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", "))
    }
}

/// Formal statement of the executed rule (kept in sync with the engines).
pub(super) fn rules_text(rule: &Rule3D, kind: WorldKind, size: [u32; 3]) -> String {
    let states = rule.states();
    let last = states - 1;
    let mut forms = vec![format!("`{}`", rule.to_letters())];
    if let Some(bays) = rule.to_bays() {
        forms.push(format!("Bays `{bays}`"));
    }
    if let Some(golly) = rule.to_golly() {
        forms.push(format!("Golly `{golly}`"));
    }
    let neighbourhood = match rule.neighbourhood() {
        Neighbourhood::Moore => "Moore, the 26 voxels with max(|dx|, |dy|, |dz|) = 1",
        Neighbourhood::VonNeumann => "von Neumann, the 6 face-adjacent voxels (|dx| + |dy| + |dz| = 1)",
    };
    let dying = if states > 2 { format!(", 2 … {last} = dying (refractory)") } else { String::new() };
    let failure = if states > 2 { "enters dying state 2" } else { "becomes empty (state 0)" };
    let decay = if states > 2 {
        format!(
            "* **Decay:** a voxel in state k ∈ {{2, …, {last}}} becomes k + 1, and state {last} becomes empty, \
             whatever its neighbours. A dying voxel cannot be reborn before it is empty.\n"
        )
    } else {
        String::new()
    };
    let [w, h, d] = size;
    let edges = match kind {
        WorldKind::DenseBounded | WorldKind::SparseBounded => {
            format!("the world is the box [0, {w}) × [0, {h}) × [0, {d}); voxels outside it are permanently empty")
        }
        WorldKind::DenseTorus => format!(
            "the world is the 3-torus Z{w} × Z{h} × Z{d}: neighbour offsets are added modulo the size of each \
             axis (along an axis shorter than 3 the same voxel is reached through several offsets and is counted \
             once per offset)"
        ),
        WorldKind::SparseUnbounded => format!(
            "unbounded; coordinates range over [−2^30, 2^30) on every axis and voxels beyond are never born. The \
             {w} × {h} × {d} box only positions the initial state"
        ),
    };
    format!(
        "**Rule `{rule}`** (also {forms}): synchronous {states}-state outer-totalistic cellular automaton on the \
         cubic lattice.\n\n\
         * **States:** 0 = empty, 1 = alive{dying}.\n\
         * **Neighbourhood:** {neighbourhood}. N is the number of neighbours in state 1; dying voxels are not \
         counted.\n\
         * **Birth:** an empty voxel becomes alive iff N ∈ {birth}.\n\
         * **Survival:** an alive voxel stays alive iff N ∈ {survive}; otherwise it {failure}.\n\
         {decay}\
         * **Edges:** {edges}.\n\
         * All voxels update simultaneously from the previous generation. Birth on 0 neighbours is not allowed, \
         so empty space stays empty.\n\
         * **Age** (display only): generations since the voxel was born or placed; it does not affect the \
         dynamics.",
        forms = forms.join(", "),
        birth = list(rule.birth()),
        survive = list(rule.survive()),
    )
}

/// Metric declarations for a rule with `states` states.
pub(super) fn metric_specs(states: u16) -> Vec<MetricSpec> {
    let mut specs =
        vec![MetricSpec::gauge("population", "Population", "voxels", "Non-empty voxels (every state except 0).")];
    for state in 1..states {
        let (label, description) = if state == 1 {
            ("Alive (state 1)".to_string(), "Voxels in state 1, the only state counted as a neighbour.".to_string())
        } else {
            (format!("Dying state {state}"), format!("Voxels in the refractory state {state}."))
        };
        specs.push(MetricSpec::gauge(&format!("state_{state}"), &label, "voxels", &description));
    }
    specs.push(MetricSpec::counter("births", "Births", "Empty voxels that became alive in the last generation."));
    specs.push(MetricSpec::counter(
        "deaths",
        "Deaths",
        "Alive voxels that did not survive the last generation (became empty or started dying).",
    ));
    for axis in ["x", "y", "z"] {
        specs.push(MetricSpec::gauge(
            &format!("bbox_min_{axis}"),
            &format!("Bounding box min {axis}"),
            "voxels",
            &format!("Smallest {axis} of a non-empty voxel (absent when the world is empty)."),
        ));
        specs.push(MetricSpec::gauge(
            &format!("bbox_max_{axis}"),
            &format!("Bounding box max {axis}"),
            "voxels",
            &format!("Largest {axis} of a non-empty voxel (absent when the world is empty)."),
        ));
    }
    for axis in ["x", "y", "z"] {
        specs.push(MetricSpec::gauge(
            &format!("centroid_{axis}"),
            &format!("Centroid {axis}"),
            "voxels",
            &format!("Mean {axis} of the non-empty voxels (arithmetic mean, also on a torus; absent when empty)."),
        ));
    }
    specs
}
