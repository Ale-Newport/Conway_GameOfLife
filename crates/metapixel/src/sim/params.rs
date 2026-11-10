//! Parameter and metric declarations of [`super::MetapixelSim`], the step
//! size, and the formal rule text shown in the rule panel.

use serde::{Deserialize, Serialize};
use simulation_core::{ApplyPolicy, MetricSpec, ParamSpec, ParamValue};

use crate::macro_grid::MAX_GRID_SIDE;
use crate::programming::Programming;
use crate::spec::PERIOD;

/// Largest power-of-two step exponent offered (2^15 = 32 768 < P).
pub const MAX_STEP_EXPONENT: u32 = 15;
/// Largest Hashlife node limit accepted by the `node_limit` parameter.
pub const MAX_NODE_LIMIT: i64 = 2_000_000_000;

/// Generations advanced by one tick (default: one metageneration).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StepSize {
    /// `2^k` generations, `k` in `0..=15`.
    Pow2(u32),
    /// One metageneration: 35 328 generations.
    #[default]
    Metagen,
}

impl StepSize {
    /// Generations per tick.
    pub fn generations(self) -> u64 {
        match self {
            Self::Pow2(k) => 1 << k.min(MAX_STEP_EXPONENT),
            Self::Metagen => PERIOD,
        }
    }

    /// Parameter value (`"1"`, `"2"`, ..., `"32768"`, `"35328"`).
    pub fn value(self) -> String {
        self.generations().to_string()
    }

    /// Parses a parameter value: a power of two up to 32 768, or 35 328
    /// (also `"metagen"`).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("metagen") {
            return Some(Self::Metagen);
        }
        let n: u64 = text.replace(['_', ' '], "").parse().ok()?;
        if n == PERIOD {
            Some(Self::Metagen)
        } else if n.is_power_of_two() && n.trailing_zeros() <= MAX_STEP_EXPONENT {
            Some(Self::Pow2(n.trailing_zeros()))
        } else {
            None
        }
    }
}

/// The `step` parameter is a choice whose options are numbers written as
/// text, but command-line overrides (`ParamValue::parse_loose`) and scenario
/// files naturally produce integers (`step=1024`) or the word `metagen`.
/// Any spelling [`StepSize::parse`] accepts is rewritten to the option text;
/// every other key and value passes through unchanged, so validation still
/// rejects anything that is not an option.
pub fn normalise_step_value(key: &str, value: ParamValue) -> ParamValue {
    if key != "step" {
        return value;
    }
    let parsed = match &value {
        ParamValue::Text(text) => StepSize::parse(text),
        other => other.as_i64().and_then(|n| StepSize::parse(&n.to_string())),
    };
    parsed.map_or(value, |step| ParamValue::Text(step.value()))
}

/// Parameter declarations.
pub fn param_specs() -> Vec<ParamSpec> {
    let mut steps: Vec<(String, String)> = (0..=MAX_STEP_EXPONENT)
        .map(|k| ((1u64 << k).to_string(), format!("2^{k} = {} generations", 1u64 << k)))
        .collect();
    steps.push((PERIOD.to_string(), "one metageneration (35 328 generations)".into()));
    let options: Vec<(&str, &str)> = steps.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect();
    let side = i64::from(MAX_GRID_SIDE);
    vec![
        ParamSpec::text("rule", "Macro rule", 32, "B3/S23")
            .describe(
                "Outer-totalistic rule computed by the metacells (B3/S23, B36/S23, B0/S...). It is programmed by \
                 completing rule-table eaters inside every tile; the physical cells always run B3/S23. B0 is \
                 allowed: tiles inside the grid can be born from nothing, tiles outside never exist.",
            )
            .group("Rules")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::text("neighbourhood", "Macro neighbourhood", 40, "moore")
            .describe(
                "Directions every metacell listens to: moore, von_neumann, none, or a list such as N,E,S,W,NE. \
                 Implemented by removing fuse objects (edge blocks or opposite corner LWSSs) from every tile.",
            )
            .group("Rules")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("width", "Width (tiles)", 1, side, 3)
            .describe("Macro grid width in metacells (2048 cells each). Outside the grid is permanently OFF.")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("height", "Height (tiles)", 1, side, 3)
            .describe("Macro grid height in metacells (2048 cells each).")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("seed", "Seed", 0, i64::MAX, 1)
            .describe("Seed of the random generator used by the Randomize command.")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::choice("step", "Generations per tick", &options, &PERIOD.to_string())
            .describe(
                "Physical B3/S23 generations advanced per tick by Hashlife. Ticks stop exactly at every multiple of \
                 35 328 to read the macro states, whatever the step.",
            )
            .group("Speed")
            .apply(ApplyPolicy::Immediate),
        ParamSpec::int(
            "node_limit",
            "Hashlife node limit",
            hashlife::MIN_NODE_LIMIT as i64,
            MAX_NODE_LIMIT,
            hashlife::DEFAULT_NODE_LIMIT as i64,
        )
        .describe(
            "Live quadtree nodes (about 48 bytes each) at which Hashlife collects garbage. Soft limit: larger grids \
             run faster with more memory.",
        )
        .group("Memory")
        .apply(ApplyPolicy::Immediate),
    ]
}

/// Metric declarations.
pub fn metric_specs() -> Vec<MetricSpec> {
    vec![
        MetricSpec::gauge("population", "Physical population", "cells", "Live B3/S23 cells of the construction."),
        MetricSpec::gauge("generation", "Generation", "generations", "Physical generations since the build."),
        MetricSpec::gauge(
            "metagen",
            "Displayed metageneration",
            "metagens",
            "m(t) = max(0, ⌊(t − 7408) / 35328⌋): the metageneration the displays show (valid outside transitions).",
        ),
        MetricSpec::gauge("phase", "Phase", "generations", "t mod 35328, the position inside the metacell cycle."),
        MetricSpec::gauge(
            "readouts",
            "Exact readouts",
            "metagens",
            "Metagenerations read from the physical construction so far (metagen n is read at t = (n + 1)·35328).",
        ),
        MetricSpec::gauge(
            "exact_on_tiles",
            "ON metacells (exact)",
            "tiles",
            "ON tiles in the latest exact readout (0 before the first readout).",
        ),
        MetricSpec::gauge(
            "accelerated_on_tiles",
            "ON metacells (accelerated)",
            "tiles",
            "ON tiles predicted by the accelerated macro representation for the same metageneration — not a \
             physical simulation.",
        ),
        MetricSpec::gauge(
            "mismatches",
            "Mismatches",
            "tiles",
            "Tiles of the latest readout whose exact state differs from the accelerated prediction (ambiguous tiles \
             count as mismatches).",
        ),
        MetricSpec::gauge("ambiguous_tiles", "Ambiguous tiles", "tiles", "Latest readout cores neither 0 nor 29 795."),
        MetricSpec::gauge("total_mismatches", "Total mismatches", "tiles", "Mismatches summed over every readout."),
        MetricSpec::gauge("node_count", "Hashlife nodes", "nodes", "Live quadtree nodes, including memoised results."),
        MetricSpec::gauge("memory_bytes", "Hashlife memory", "bytes", "Bytes allocated by the node arena and table."),
    ]
}

/// Formal statement of what is executed (shown in the rule panel).
pub fn rules_text(programming: Programming, width: u32, height: u32, step: StepSize) -> String {
    let set = |counts: Vec<u8>| {
        if counts.is_empty() {
            "∅".to_string()
        } else {
            format!("{{{}}}", counts.iter().map(u8::to_string).collect::<Vec<_>>().join(", "))
        }
    };
    let directions: Vec<&str> = programming.neighbourhood.directions().map(|d| d.name()).collect();
    format!(
        "**Macro rule `{rule}` on a {width}×{height} grid of OTCA metapixels**, neighbourhood {nbhd} \
         (listened directions: {dirs}).\n\n\
         * N(c, n) = number of ON metacells among c's listened neighbours at metageneration n; metacells outside \
         the grid do not exist and count as permanently OFF (they are never born).\n\
         * An OFF metacell turns ON iff N ∈ {birth}; an ON metacell stays ON iff N ∈ {survive}. All metacells \
         update synchronously.\n\n\
         **Physical level (exact).** Each metacell is Brice Due's 2048×2048 OTCA metapixel tile, programmed by \
         completing the rule-table eaters of the digits above and removing the fuse objects of unheard directions; \
         ON or OFF tiles are OR-ed at pitch 2048 at generation 0. The whole plane then runs Conway's Life \
         **B3/S23** under Hashlife — nothing else is simulated.\n\
         * One metageneration = P = 35 328 generations. Metagen n is read at t = (n + 1)·P by counting live cells \
         in each tile's core [250, 1750]²: 0 = OFF, 29 795 = ON, anything else = ambiguous (reported, never \
         rounded).\n\
         * The display shows m(t) = max(0, ⌊(t − 7408)/P⌋); cores may change while t mod P ∈ [4320, 7407] (t ≥ P).\n\
         * One tick advances {gens} generations, stopping at every multiple of P to read the tiles.\n\n\
         **Accelerated macro representation — not a physical simulation.** The same rule is also applied directly \
         to the macro grid (same neighbourhood, outside OFF) to predict each metageneration; every exact readout \
         is compared with it and disagreements are reported as mismatches.",
        rule = programming.rule,
        nbhd = programming.neighbourhood,
        dirs = if directions.is_empty() { "none".to_string() } else { directions.join(", ") },
        birth = set(programming.rule.birth_counts()),
        survive = set(programming.rule.survival_counts()),
        gens = step.generations(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_sizes_parse_and_round_trip() {
        assert_eq!(StepSize::parse("35328"), Some(StepSize::Metagen));
        assert_eq!(StepSize::parse("metagen"), Some(StepSize::Metagen));
        assert_eq!(StepSize::parse("1024"), Some(StepSize::Pow2(10)));
        assert_eq!(StepSize::parse("1"), Some(StepSize::Pow2(0)));
        assert_eq!(StepSize::parse("1000"), None);
        assert_eq!(StepSize::parse("65536"), None);
        for k in 0..=MAX_STEP_EXPONENT {
            assert_eq!(StepSize::parse(&StepSize::Pow2(k).value()), Some(StepSize::Pow2(k)));
        }
        let spec = param_specs().into_iter().find(|s| s.key == "step").unwrap();
        for k in 0..=MAX_STEP_EXPONENT {
            assert!(spec.validate(&ParamValue::Text(StepSize::Pow2(k).value())).is_ok());
        }
    }

    #[test]
    fn step_values_from_command_lines_are_normalised() {
        let spec = param_specs().into_iter().find(|s| s.key == "step").unwrap();
        let accept = |value: ParamValue| spec.validate(&normalise_step_value("step", value));
        // `--param step=1024` arrives as an integer from `ParamValue::parse_loose`.
        assert_eq!(accept(ParamValue::parse_loose("1024")), Ok(ParamValue::Text("1024".into())));
        assert_eq!(accept(ParamValue::Int(35_328)), Ok(ParamValue::Text("35328".into())));
        assert_eq!(accept(ParamValue::Float(4.0)), Ok(ParamValue::Text("4".into())));
        assert_eq!(accept(ParamValue::Text("metagen".into())), Ok(ParamValue::Text("35328".into())));
        // Values that are not step sizes still fail validation.
        assert!(accept(ParamValue::Int(1000)).is_err());
        assert!(accept(ParamValue::Bool(true)).is_err());
        // Other keys are untouched.
        assert_eq!(normalise_step_value("width", ParamValue::Int(4)), ParamValue::Int(4));
    }

    #[test]
    fn rules_text_states_the_executed_rule() {
        let text = rules_text(Programming::parse("B36/S23", "von_neumann").unwrap(), 4, 5, StepSize::Pow2(10));
        assert!(text.contains("B36/S23") && text.contains("4×5") && text.contains("{3, 6}"));
        assert!(text.contains("N, E, S, W") && text.contains("1024 generations"));
        assert!(text.contains("not a physical simulation"));
    }
}
