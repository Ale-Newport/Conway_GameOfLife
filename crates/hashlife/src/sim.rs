//! [`HashlifeSim`]: the Hashlife engine behind the shared
//! [`simulation_core::Simulation`] contract (model id `life.hashlife`).

use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamSpec, ParamValue, SimError, SimRng,
    Simulation, Snapshot,
};

use crate::macrocell;
use crate::rule::Rule;
use crate::universe::{DEFAULT_NODE_LIMIT, MAX_STEP_EXPONENT, MIN_NODE_LIMIT, Universe};

/// Model identifier used in descriptors and snapshots.
pub const MODEL_ID: &str = "life.hashlife";
/// Snapshot payload version written by this build.
pub const SCHEMA_VERSION: u32 = 1;
/// Largest view side, in pixels; larger requests are clamped.
pub const MAX_VIEW_SIDE: u32 = 4096;
/// Largest area of a [`HashlifeCommand::RandomSoup`].
pub const MAX_SOUP_CELLS: u64 = 1 << 24;
/// Largest node limit accepted by the `node_limit` parameter.
const MAX_NODE_LIMIT: i64 = 2_000_000_000;

/// Mutations accepted by [`HashlifeSim::apply`](Simulation::apply).
#[derive(Clone, Debug, PartialEq)]
pub enum HashlifeCommand {
    /// Sets or clears one cell.
    SetCell {
        /// Column.
        x: i64,
        /// Row (downward).
        y: i64,
        /// New state.
        alive: bool,
    },
    /// Sets the listed cells alive, optionally clearing everything first.
    LoadCells {
        /// Cells to set alive.
        cells: Vec<(i64, i64)>,
        /// Clear the universe before loading.
        replace: bool,
    },
    /// Replaces pattern, rule and generation with a Golly macrocell file.
    LoadMacrocell(String),
    /// Fills the rectangle `[x, x + width) × [y, y + height)` with a random
    /// soup: each cell is alive with probability `density`, drawn from the
    /// model's seeded generator in row-major order. Existing cells in the
    /// rectangle are replaced.
    RandomSoup {
        /// Left column.
        x: i64,
        /// Top row.
        y: i64,
        /// Width in cells (1 or more).
        width: u32,
        /// Height in cells (1 or more).
        height: u32,
        /// Probability of a live cell, in `[0, 1]`.
        density: f64,
    },
    /// Kills every cell in the inclusive rectangle `[x0, x1] × [y0, y1]`.
    ClearRect {
        /// Left column.
        x0: i64,
        /// Top row.
        y0: i64,
        /// Right column (inclusive).
        x1: i64,
        /// Bottom row (inclusive).
        y1: i64,
    },
    /// Kills every cell (rule and generation are kept).
    Clear,
}

/// Region and level of detail a renderer wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashlifeViewRequest {
    /// Column of the left edge of pixel column 0.
    pub x0: i64,
    /// Row of the top edge of pixel row 0.
    pub y0: i64,
    /// Width in pixels (clamped to `1..=MAX_VIEW_SIDE`).
    pub width_px: u32,
    /// Height in pixels (clamped to `1..=MAX_VIEW_SIDE`).
    pub height_px: u32,
    /// Each pixel covers `2^log2_scale × 2^log2_scale` cells (clamped to 64).
    pub log2_scale: u32,
}

impl Default for HashlifeViewRequest {
    fn default() -> Self {
        Self { x0: -64, y0: -64, width_px: 128, height_px: 128, log2_scale: 0 }
    }
}

/// Render data: populations of `2^log2_scale`-sided blocks (see
/// [`Universe::density_grid`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HashlifeView {
    /// Column of the left edge of pixel column 0.
    pub x0: i64,
    /// Row of the top edge of pixel row 0.
    pub y0: i64,
    /// Width in pixels (after clamping).
    pub width_px: u32,
    /// Height in pixels (after clamping).
    pub height_px: u32,
    /// Cells per pixel side, as a power of two (after clamping).
    pub log2_scale: u32,
    /// Live cells per pixel, row-major, saturating at `u32::MAX`.
    pub populations: Vec<u32>,
    /// Generation the view shows.
    pub generation: u64,
    /// Total population (saturating).
    pub population: u64,
}

/// State restored by [`Simulation::reset`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ResetPoint {
    macrocell: String,
    rng: SimRng,
}

/// Snapshot payload (schema version 1).
#[derive(Serialize, Deserialize)]
struct SavedState {
    macrocell: String,
    step_exponent: u32,
    node_limit: u64,
    seed: u64,
    rng: SimRng,
    reset_point: ResetPoint,
    edited: bool,
    halted: Option<String>,
}

/// Hashlife as a [`Simulation`]: one tick advances `2^step_exponent`
/// generations and [`Simulation::tick`] reports the generation.
///
/// The unit of [`Simulation::tick`] (and of `ModelDescriptor::dt`) is the
/// generation, so [`Simulation::advance`]`(n)` advances exactly `n`
/// generations (by binary decomposition, whatever the step exponent) and
/// returns how many were advanced, while [`Simulation::step`] advances one
/// configured step of `2^step_exponent` generations.
///
/// The reset point is the pattern as it was when the run last started after
/// an edit (any command), so [`Simulation::reset`] behaves like Golly's
/// "reset". If a step fails (generation counter or coordinate overflow) the
/// model halts: [`Simulation::is_finished`] becomes true and
/// [`HashlifeSim::halt_reason`] explains why; any command, reset or restore
/// clears the halt.
pub struct HashlifeSim {
    universe: Universe,
    step_exponent: u32,
    seed: u64,
    rng: SimRng,
    reset_point: ResetPoint,
    edited: bool,
    halted: Option<String>,
}

impl HashlifeSim {
    /// An empty Conway's Life universe; `seed` drives random soups.
    pub fn new(seed: u64) -> Self {
        Self::with_universe(Universe::conway(), seed)
    }

    /// Wraps an existing universe (its pattern becomes the reset point).
    pub fn with_universe(universe: Universe, seed: u64) -> Self {
        let rng = SimRng::seed_from_u64(seed);
        let reset_point = ResetPoint { macrocell: macrocell::write(&universe), rng: rng.clone() };
        Self { universe, step_exponent: 0, seed, rng, reset_point, edited: false, halted: None }
    }

    /// Read-only access to the engine (queries, macrocell export...).
    pub fn universe(&self) -> &Universe {
        &self.universe
    }

    /// Generations per tick, as a power of two.
    pub fn step_exponent(&self) -> u32 {
        self.step_exponent
    }

    /// Why the model halted, if it did.
    pub fn halt_reason(&self) -> Option<&str> {
        self.halted.as_deref()
    }

    fn capture_reset_point(&mut self) {
        if self.edited {
            self.reset_point = ResetPoint { macrocell: macrocell::write(&self.universe), rng: self.rng.clone() };
            self.edited = false;
        }
    }

    fn random_soup(&mut self, x: i64, y: i64, width: u32, height: u32, density: f64) -> Result<(), SimError> {
        let invalid = |reason: String| SimError::InvalidCommand(format!("random soup: {reason}"));
        if !(0.0..=1.0).contains(&density) {
            return Err(invalid(format!("density must be in [0, 1], got {density}")));
        }
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_SOUP_CELLS {
            return Err(invalid(format!("{width}×{height} must be non-empty and at most {MAX_SOUP_CELLS} cells")));
        }
        let x1 = x.checked_add(i64::from(width) - 1);
        let y1 = y.checked_add(i64::from(height) - 1);
        let (Some(x1), Some(y1)) = (x1, y1) else {
            return Err(invalid("the rectangle leaves the 64-bit coordinate plane".into()));
        };
        let mut cells = Vec::new();
        for row in y..=y1 {
            for col in x..=x1 {
                if self.rng.chance(density) {
                    cells.push((col, row));
                }
            }
        }
        self.universe.clear_rect(x, y, x1, y1);
        self.universe.insert_cells(cells);
        Ok(())
    }
}

/// Parameter declarations.
fn param_specs() -> Vec<ParamSpec> {
    vec![
        ParamSpec::text("rule", "Rule", 32, "B3/S23")
            .describe(
                "Outer-totalistic Moore rule in B/S notation (B3/S23, B36/S23, 23/3...). B0 rules are rejected. \
                 Changing the rule discards all memoised Hashlife results.",
            )
            .group("Rules")
            .apply(ApplyPolicy::NextTick),
        ParamSpec::int("step_exponent", "Step exponent", 0, i64::from(MAX_STEP_EXPONENT), 0)
            .describe("Each tick advances 2^k generations in one Hashlife step (Golly's step size 2^k).")
            .group("Speed")
            .apply(ApplyPolicy::Immediate),
        ParamSpec::int("node_limit", "Node limit", MIN_NODE_LIMIT as i64, MAX_NODE_LIMIT, DEFAULT_NODE_LIMIT as i64)
            .describe(
                "Live quadtree nodes (about 48 bytes each, hash table included) at which garbage collection \
                 runs. Soft limit: the pattern and the memoised results of its nodes are always kept.",
            )
            .group("Memory")
            .apply(ApplyPolicy::Immediate),
    ]
}

/// Metric declarations.
fn metric_specs() -> Vec<MetricSpec> {
    vec![
        MetricSpec::gauge("population", "Population", "cells", "Number of live cells."),
        MetricSpec::gauge("generation", "Generation", "generations", "Generations run since generation 0."),
        MetricSpec::gauge("node_count", "Nodes", "nodes", "Live quadtree nodes, including memoised results."),
        MetricSpec::gauge("memory_bytes", "Memory", "bytes", "Bytes allocated by the node arena and its hash table."),
        MetricSpec::gauge(
            "root_level",
            "Root level",
            "log2 cells",
            "The quadtree root covers 2^level × 2^level cells.",
        ),
    ]
}

/// Formal rule text for the descriptor (kept in sync with the engine).
fn rules_text(rule: Rule, step_exponent: u32) -> String {
    let set = |mask: u16| {
        let counts: Vec<String> = (0..=8u32).filter(|n| mask >> n & 1 == 1).map(|n| n.to_string()).collect();
        if counts.is_empty() { "∅".to_string() } else { format!("{{{}}}", counts.join(", ")) }
    };
    format!(
        "**Rule `{rule}`**: two-state outer-totalistic cellular automaton on the unbounded square lattice \
         (64-bit coordinates, x to the right, y downward).\n\n\
         * **Neighbourhood:** Moore, the 8 cells at Chebyshev distance 1. N(c, t) is the number of live \
         neighbours of cell c at generation t.\n\
         * **Birth:** a dead cell is alive at t + 1 iff N(c, t) ∈ {birth}.\n\
         * **Survival:** a live cell stays alive at t + 1 iff N(c, t) ∈ {survive}; otherwise it dies.\n\
         * All cells update synchronously. Rules with B0 are rejected, so empty space stays empty.\n\n\
         **Evaluation (exact).** Gosper's Hashlife: the plane is a hash-consed quadtree of 8×8 bit-packed \
         leaves; for a node of side 2^L the centre square of side 2^(L−1) after 2^j generations (j ≤ L − 2) \
         is computed recursively and memoised. One tick advances 2^k generations, k = step exponent = \
         {step_exponent}; the tick counter reports generations.",
        birth = set(rule.birth()),
        survive = set(rule.survive()),
    )
}

impl Simulation for HashlifeSim {
    type Command = HashlifeCommand;
    type ViewRequest = HashlifeViewRequest;
    type View = HashlifeView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(MODEL_ID, "Hashlife", ModelFamily::Cellular)
            .summary(
                "Gosper's Hashlife for any two-state Life-like rule without B0: a hash-consed quadtree with \
                 memoised evolution that advances regular patterns (guns, breeders, metapixels) by up to 2^61 \
                 generations per tick on an unbounded plane.",
            )
            .rules(&rules_text(self.universe.rule(), self.step_exponent))
            .params(param_specs())
            .metrics(metric_specs())
            .time("generation", 1.0)
    }

    fn tick(&self) -> u64 {
        self.universe.generation()
    }

    fn step(&mut self) {
        if self.halted.is_some() {
            return;
        }
        self.capture_reset_point();
        if let Err(error) = self.universe.step_pow2(self.step_exponent) {
            self.halted = Some(error.to_string());
        }
    }

    fn advance(&mut self, max_ticks: u64) -> u64 {
        if self.halted.is_some() {
            return 0;
        }
        self.capture_reset_point();
        let start = self.universe.generation();
        if let Err(error) = self.universe.advance(max_ticks) {
            self.halted = Some(error.to_string());
        }
        self.universe.generation() - start
    }

    fn reset(&mut self) {
        match macrocell::read_into(&mut self.universe, &self.reset_point.macrocell) {
            Ok(()) => {
                self.rng = self.reset_point.rng.clone();
                self.edited = false;
                self.halted = None;
            }
            Err(error) => self.halted = Some(format!("could not restore the reset point: {error}")),
        }
    }

    fn apply(&mut self, command: HashlifeCommand) -> Result<(), SimError> {
        match command {
            HashlifeCommand::SetCell { x, y, alive } => self.universe.set_cell(x, y, alive),
            HashlifeCommand::LoadCells { cells, replace } => {
                if replace {
                    self.universe.clear();
                }
                self.universe.insert_cells(cells);
            }
            HashlifeCommand::LoadMacrocell(text) => {
                macrocell::read_into(&mut self.universe, &text).map_err(|e| SimError::Parse(e.to_string()))?;
            }
            HashlifeCommand::RandomSoup { x, y, width, height, density } => {
                self.random_soup(x, y, width, height, density)?;
            }
            HashlifeCommand::ClearRect { x0, y0, x1, y1 } => {
                if x0 > x1 || y0 > y1 {
                    return Err(SimError::InvalidCommand(format!(
                        "clear rectangle [{x0}, {x1}] × [{y0}, {y1}] is empty (x0 > x1 or y0 > y1)"
                    )));
                }
                self.universe.clear_rect(x0, y0, x1, y1);
            }
            HashlifeCommand::Clear => self.universe.clear(),
        }
        self.edited = true;
        self.halted = None;
        Ok(())
    }

    fn params(&self) -> ParamSet {
        ParamSet::new()
            .with("rule", ParamValue::Text(self.universe.rule().to_string()))
            .with("step_exponent", ParamValue::Int(i64::from(self.step_exponent)))
            .with("node_limit", ParamValue::Int(i64::try_from(self.universe.node_limit()).unwrap_or(i64::MAX)))
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let spec = param_specs()
            .into_iter()
            .find(|s| s.key == key)
            .ok_or_else(|| SimError::UnknownParameter(key.to_string()))?;
        let value = spec.validate(&value)?;
        let integer =
            || value.as_i64().ok_or_else(|| SimError::invalid_param(key, format!("expected an integer, got {value}")));
        match key {
            "rule" => {
                let text = value.as_str().unwrap_or_default();
                let rule = Rule::parse(text).map_err(|e| SimError::invalid_param(key, e.to_string()))?;
                self.universe.set_rule(rule);
            }
            "step_exponent" => {
                // Validated against [0, MAX_STEP_EXPONENT].
                self.step_exponent = integer()? as u32;
            }
            _ => {
                // "node_limit", validated against [MIN_NODE_LIMIT, MAX_NODE_LIMIT].
                self.universe.set_node_limit(integer()? as usize);
            }
        }
        Ok(spec.apply)
    }

    fn metrics(&self) -> Metrics {
        let generation = self.universe.generation();
        Metrics::new(generation, generation as f64)
            .with("population", self.universe.population() as f64)
            .with("generation", generation as f64)
            .with("node_count", self.universe.node_count() as f64)
            .with("memory_bytes", self.universe.memory_bytes() as f64)
            .with("root_level", f64::from(self.universe.root_level()))
    }

    fn view(&self, request: &HashlifeViewRequest) -> HashlifeView {
        let width_px = request.width_px.clamp(1, MAX_VIEW_SIDE);
        let height_px = request.height_px.clamp(1, MAX_VIEW_SIDE);
        let log2_scale = request.log2_scale.min(64);
        let mut populations = vec![0u32; width_px as usize * height_px as usize];
        self.universe.density_into(
            request.x0,
            request.y0,
            width_px as usize,
            height_px as usize,
            log2_scale,
            &mut populations,
        );
        HashlifeView {
            x0: request.x0,
            y0: request.y0,
            width_px,
            height_px,
            log2_scale,
            populations,
            generation: self.universe.generation(),
            population: self.universe.population(),
        }
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        let state = SavedState {
            macrocell: macrocell::write(&self.universe),
            step_exponent: self.step_exponent,
            node_limit: self.universe.node_limit() as u64,
            seed: self.seed,
            rng: self.rng.clone(),
            reset_point: self.reset_point.clone(),
            edited: self.edited,
            halted: self.halted.clone(),
        };
        Snapshot::encode(MODEL_ID, SCHEMA_VERSION, self.universe.generation(), &state)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let state: SavedState = snapshot.decode(MODEL_ID, SCHEMA_VERSION)?;
        if state.step_exponent > MAX_STEP_EXPONENT {
            return Err(SimError::Serialization(format!("step exponent {} is out of range", state.step_exponent)));
        }
        // The same range as the `node_limit` parameter.
        let node_limit = usize::try_from(state.node_limit)
            .ok()
            .filter(|&limit| (MIN_NODE_LIMIT..=MAX_NODE_LIMIT as usize).contains(&limit))
            .ok_or_else(|| {
                SimError::Serialization(format!(
                    "node limit {} is outside [{MIN_NODE_LIMIT}, {MAX_NODE_LIMIT}]",
                    state.node_limit
                ))
            })?;
        macrocell::read_into(&mut self.universe, &state.macrocell)
            .map_err(|e| SimError::Serialization(format!("snapshot pattern: {e}")))?;
        self.universe.set_node_limit(node_limit);
        self.step_exponent = state.step_exponent;
        self.seed = state.seed;
        self.rng = state.rng;
        self.reset_point = state.reset_point;
        self.edited = state.edited;
        self.halted = state.halted;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.universe.memory_bytes() + self.reset_point.macrocell.capacity()
    }

    fn is_finished(&self) -> bool {
        self.halted.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_rejects_out_of_range_settings_and_changes_nothing() {
        let mut source = HashlifeSim::new(5);
        source.apply(HashlifeCommand::LoadCells { cells: vec![(0, 0), (1, 0), (2, 0)], replace: true }).unwrap();
        let encode = |node_limit: u64, step_exponent: u32| {
            let state = SavedState {
                macrocell: macrocell::write(source.universe()),
                step_exponent,
                node_limit,
                seed: 5,
                rng: SimRng::seed_from_u64(5),
                reset_point: source.reset_point.clone(),
                edited: false,
                halted: None,
            };
            Snapshot::encode(MODEL_ID, SCHEMA_VERSION, 0, &state).unwrap()
        };
        let too_big = MAX_NODE_LIMIT as u64 + 1;
        for (node_limit, step_exponent) in [(0, 0), (1023, 0), (too_big, 0), (u64::MAX, 0), (1 << 20, 62)] {
            let mut target = HashlifeSim::new(1);
            let result = target.restore(&encode(node_limit, step_exponent));
            assert!(matches!(result, Err(SimError::Serialization(_))), "{node_limit}, {step_exponent}");
            assert!(target.universe().is_empty(), "a rejected snapshot changes nothing");
            assert_eq!(target.universe().node_limit(), DEFAULT_NODE_LIMIT);
        }
        let mut target = HashlifeSim::new(1);
        target.restore(&encode(1 << 20, 3)).unwrap();
        assert_eq!((target.universe().node_limit(), target.step_exponent()), (1 << 20, 3));
        assert_eq!(target.universe().population(), 3);
    }
}
