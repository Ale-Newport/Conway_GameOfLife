//! `WolframSim`: elementary (one-dimensional, two-state, radius-1)
//! cellular automata with Wolfram's rule numbering, shown as a space-time
//! diagram.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamSpec, ParamValue, SimError, SimRng,
    Simulation, Snapshot,
};

use crate::engine::swar::{count_word, last_word_mask};
use crate::error::CaError;
use crate::geometry::CellRect;
use crate::view::{GridView, GridViewRequest, TexelGrid};

/// Model id.
pub const WOLFRAM_MODEL_ID: &str = "ca.wolfram";
const SCHEMA_VERSION: u32 = 1;
/// Largest row width.
pub const MAX_WIDTH: u32 = 1 << 20;
/// Largest number of stored rows × width (cells kept for the diagram).
pub const MAX_HISTORY_CELLS: u64 = 1 << 30;

/// What lies beyond the two ends of the row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Boundary {
    /// Cells outside are always 0.
    #[default]
    Fixed0,
    /// Cells outside are always 1.
    Fixed1,
    /// The row is a ring.
    Periodic,
}

impl Boundary {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fixed0 => "fixed0",
            Self::Fixed1 => "fixed1",
            Self::Periodic => "periodic",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Fixed0, Self::Fixed1, Self::Periodic].into_iter().find(|b| b.as_str() == s)
    }
}

/// How the first row is filled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EcaStart {
    /// A single 1 at `width / 2`.
    #[default]
    SingleCell,
    /// Each cell is 1 with probability `density` (seeded).
    Random,
}

/// Configuration of a [`WolframSim`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WolframConfig {
    /// Rule number 0..=255.
    pub rule: u8,
    /// Cells per row.
    #[serde(default = "default_width")]
    pub width: u32,
    /// Boundary condition.
    #[serde(default)]
    pub boundary: Boundary,
    /// First row.
    #[serde(default)]
    pub start: EcaStart,
    /// Density of a random first row.
    #[serde(default = "half")]
    pub density: f64,
    /// Rows kept for the diagram (older rows scroll away).
    #[serde(default = "default_history")]
    pub history: u32,
}

fn default_width() -> u32 {
    1025
}

fn half() -> f64 {
    0.5
}

fn default_history() -> u32 {
    4096
}

impl Default for WolframConfig {
    fn default() -> Self {
        Self {
            rule: 30,
            width: 1025,
            boundary: Boundary::Fixed0,
            start: EcaStart::SingleCell,
            density: 0.5,
            history: 4096,
        }
    }
}

impl WolframConfig {
    fn validate(&self) -> Result<(), CaError> {
        if !(3..=MAX_WIDTH).contains(&self.width) {
            return Err(CaError::invalid(format!("the row width must be between 3 and {MAX_WIDTH}")));
        }
        if self.history == 0 || u64::from(self.history) * u64::from(self.width) > MAX_HISTORY_CELLS {
            return Err(CaError::LimitExceeded(format!(
                "{} rows of {} cells exceed the diagram limit of {MAX_HISTORY_CELLS} cells",
                self.history, self.width
            )));
        }
        if !(0.0..=1.0).contains(&self.density) {
            return Err(CaError::invalid("the density must be between 0 and 1"));
        }
        Ok(())
    }
}

/// Edits of the current (newest) row.
#[derive(Clone, Debug, PartialEq)]
pub enum WolframCommand {
    /// Sets one cell of the current row.
    SetCell {
        /// Cell index `0..width`.
        x: i64,
        /// New value.
        alive: bool,
    },
    /// Replaces the current row by random cells.
    Randomize {
        /// Probability of a 1.
        density: f64,
        /// Dedicated seed, or the model's generator.
        seed: Option<u64>,
    },
    /// Replaces the current row by a single 1 at `width / 2`.
    SingleCell,
    /// Sets the current row to 0.
    Clear,
}

/// Elementary cellular automaton as a [`Simulation`]; one tick computes one
/// new row, and [`Simulation::tick`] is the row (time) index.
///
/// Rule numbering (Wolfram 1983): the neighbourhood `(l, c, r)` is the
/// number `i = 4l + 2c + r`, and the new centre cell is bit `i` of the rule
/// number. The view is the space-time diagram: cell `(x, t)` is cell `x` of
/// row `t` (rows = time, row 0 at the top).
pub struct WolframSim {
    config: WolframConfig,
    rule: u8,
    pending_rule: Option<u8>,
    boundary: Boundary,
    pending_boundary: Option<Boundary>,
    rows: VecDeque<Vec<u64>>,
    generation: u64,
    rng: SimRng,
    seed: u64,
    reset_point: Option<(Vec<u64>, SimRng)>,
    /// Configuration used by the next reset.
    next_config: WolframConfig,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    config: WolframConfig,
    rule: u8,
    pending_rule: Option<u8>,
    boundary: Boundary,
    pending_boundary: Option<Boundary>,
    rows: Vec<Vec<u64>>,
    generation: u64,
    rng: SimRng,
    seed: u64,
    reset_point: Option<(Vec<u64>, SimRng)>,
    next_config: WolframConfig,
}

/// Next row of `row` (`width` cells) under `rule` with `boundary`.
pub fn eca_step(rule: u8, row: &[u64], width: usize, boundary: Boundary) -> Vec<u64> {
    let n = row.len();
    let bit = |x: usize| row[x / 64] >> (x % 64) & 1 == 1;
    let (left, right) = match boundary {
        Boundary::Fixed0 => (false, false),
        Boundary::Fixed1 => (true, true),
        Boundary::Periodic => (bit(width - 1), bit(0)),
    };
    let mut l = vec![0u64; n];
    let mut r = vec![0u64; n];
    crate::engine::swar::shift_west(row, left, &mut l);
    crate::engine::swar::shift_east(row, right, width, &mut r);
    let mut out = vec![0u64; n];
    for k in 0..n {
        let (a, b, c) = (l[k], row[k], r[k]);
        let mut word = 0;
        for i in 0..8 {
            if rule >> i & 1 == 1 {
                let pick = |v: u64, on: bool| if on { v } else { !v };
                word |= pick(a, i & 4 != 0) & pick(b, i & 2 != 0) & pick(c, i & 1 != 0);
            }
        }
        out[k] = word;
    }
    out[n - 1] &= last_word_mask(width);
    out
}

impl WolframSim {
    /// Builds the model.
    ///
    /// # Errors
    /// Width, history or density out of range.
    pub fn new(config: &WolframConfig, seed: u64) -> Result<Self, CaError> {
        config.validate()?;
        let mut sim = Self {
            config: config.clone(),
            rule: config.rule,
            pending_rule: None,
            boundary: config.boundary,
            pending_boundary: None,
            rows: VecDeque::new(),
            generation: 0,
            rng: SimRng::seed_from_u64(seed),
            seed,
            reset_point: None,
            next_config: config.clone(),
        };
        sim.rebuild();
        Ok(sim)
    }

    /// Cells per row of the running diagram (the `width` parameter reports
    /// the width of the next reset).
    pub fn width(&self) -> usize {
        self.config.width as usize
    }

    /// The boundary condition of the running rows (the `boundary` parameter
    /// reports a change that applies from the next row).
    pub fn boundary(&self) -> Boundary {
        self.boundary
    }

    /// The oldest row still kept for the diagram: rows
    /// `first_row()..=tick()` exist.
    pub fn first_row(&self) -> u64 {
        self.generation + 1 - self.rows.len() as u64
    }

    fn rebuild(&mut self) {
        self.config = self.next_config.clone();
        self.rng = SimRng::seed_from_u64(self.seed);
        let mut row = vec![0u64; self.width().div_ceil(64)];
        match self.config.start {
            EcaStart::SingleCell => {
                let c = self.width() / 2;
                row[c / 64] |= 1 << (c % 64);
            }
            EcaStart::Random => {
                for x in 0..self.width() {
                    if self.rng.f64() < self.config.density {
                        row[x / 64] |= 1 << (x % 64);
                    }
                }
            }
        }
        self.rows = VecDeque::from([row]);
        self.generation = 0;
        self.reset_point = None;
    }

    /// The current (newest) row as booleans.
    pub fn current_row(&self) -> Vec<bool> {
        self.row_at(self.generation).unwrap_or_default()
    }

    /// Row `t` as booleans, if it is still kept.
    pub fn row_at(&self, t: u64) -> Option<Vec<bool>> {
        let first = self.first_row();
        if t < first || t > self.generation {
            return None;
        }
        let row = &self.rows[(t - first) as usize];
        Some((0..self.width()).map(|x| row[x / 64] >> (x % 64) & 1 == 1).collect())
    }

    /// The rule number.
    pub fn rule(&self) -> u8 {
        self.rule
    }

    fn current_mut(&mut self) -> &mut Vec<u64> {
        let last = self.rows.len() - 1;
        &mut self.rows[last]
    }

    fn rules_text(&self) -> String {
        let table: Vec<String> = (0..8).rev().map(|i| format!("{:03b}->{}", i, self.rule >> i & 1)).collect();
        let boundary = match self.boundary {
            Boundary::Fixed0 => "cells beyond both ends are always 0",
            Boundary::Fixed1 => "cells beyond both ends are always 1",
            Boundary::Periodic => "the row is a ring (cell −1 is cell W−1, cell W is cell 0)",
        };
        format!(
            "## Elementary rule {rule}\n\n\
             A row of W cells, each 0 or 1, updated simultaneously. The new value of a cell depends on its left \
             neighbour *l*, itself *c* and its right neighbour *r*: it is bit 4*l* + 2*c* + *r* of the rule number \
             (Wolfram's numbering): {table}. Boundary: {boundary}. The diagram shows time downward: row *t* is the \
             row after *t* steps.",
            rule = self.rule,
            table = table.join(", ")
        )
    }

    fn specs() -> Vec<ParamSpec> {
        vec![
            ParamSpec::int("rule", "Rule number", 0, 255, 30).group("Rules").apply(ApplyPolicy::NextTick),
            ParamSpec::choice(
                "boundary",
                "Boundary",
                &[("fixed0", "Fixed 0"), ("fixed1", "Fixed 1"), ("periodic", "Periodic (ring)")],
                "fixed0",
            )
            .group("Rules")
            .apply(ApplyPolicy::NextTick),
            ParamSpec::int("width", "Width", 3, i64::from(MAX_WIDTH), 1025)
                .group("World")
                .apply(ApplyPolicy::RequiresReset),
            ParamSpec::choice(
                "start",
                "First row",
                &[("single_cell", "Single cell"), ("random", "Random")],
                "single_cell",
            )
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
            ParamSpec::float("density", "Random density", 0.0, 1.0, 0.5)
                .group("World")
                .apply(ApplyPolicy::RequiresReset),
            ParamSpec::int("history", "Rows kept", 1, 1 << 20, 4096)
                .group("Display")
                .apply(ApplyPolicy::RequiresReset)
                .describe("Rows kept for the space-time diagram; rows × width is limited to 2^30 cells."),
        ]
    }
}

impl Simulation for WolframSim {
    type Command = WolframCommand;
    type ViewRequest = GridViewRequest;
    type View = GridView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(WOLFRAM_MODEL_ID, "Elementary cellular automata", ModelFamily::Cellular)
            .summary("Wolfram's 256 one-dimensional rules drawn as space-time diagrams (rule 30, 90, 110, 184...).")
            .rules(&self.rules_text())
            .params(Self::specs())
            .metrics(vec![
                MetricSpec::gauge("ones", "Ones", "cells", "Cells equal to 1 in the current row."),
                MetricSpec::ratio("density", "Density", "Ones divided by the row width."),
                MetricSpec::gauge("generation", "Row", "steps", "Index of the current row."),
            ])
            .time("step", 1.0)
    }

    fn tick(&self) -> u64 {
        self.generation
    }

    fn step(&mut self) {
        if let Some(rule) = self.pending_rule.take() {
            self.rule = rule;
        }
        if let Some(b) = self.pending_boundary.take() {
            self.boundary = b;
        }
        if self.generation == 0 && self.reset_point.is_none() {
            self.reset_point = Some((self.rows[0].clone(), self.rng.clone()));
        }
        let next = eca_step(self.rule, &self.rows[self.rows.len() - 1], self.width(), self.boundary);
        self.rows.push_back(next);
        while self.rows.len() > self.config.history as usize {
            self.rows.pop_front();
        }
        self.generation += 1;
    }

    fn reset(&mut self) {
        if let Some(rule) = self.pending_rule.take() {
            self.rule = rule;
        }
        if let Some(b) = self.pending_boundary.take() {
            self.boundary = b;
        }
        match self.reset_point.clone() {
            Some((row, rng)) if self.next_config == self.config => {
                self.rows = VecDeque::from([row]);
                self.rng = rng;
                self.generation = 0;
            }
            _ => self.rebuild(),
        }
    }

    fn apply(&mut self, command: WolframCommand) -> Result<(), SimError> {
        let width = self.width();
        match command {
            WolframCommand::SetCell { x, alive } => {
                let x = usize::try_from(x)
                    .ok()
                    .filter(|&x| x < width)
                    .ok_or_else(|| CaError::invalid(format!("cell {x} is outside the row (0 to {})", width - 1)))?;
                let row = self.current_mut();
                if alive {
                    row[x / 64] |= 1 << (x % 64);
                } else {
                    row[x / 64] &= !(1 << (x % 64));
                }
            }
            WolframCommand::Randomize { density, seed } => {
                if !(0.0..=1.0).contains(&density) {
                    return Err(CaError::invalid("the density must be between 0 and 1").into());
                }
                let mut own = seed.map(SimRng::seed_from_u64);
                let mut row = vec![0u64; width.div_ceil(64)];
                for x in 0..width {
                    let draw = match own.as_mut() {
                        Some(rng) => rng.f64(),
                        None => self.rng.f64(),
                    };
                    if draw < density {
                        row[x / 64] |= 1 << (x % 64);
                    }
                }
                *self.current_mut() = row;
            }
            WolframCommand::SingleCell => {
                let mut row = vec![0u64; width.div_ceil(64)];
                row[(width / 2) / 64] |= 1 << ((width / 2) % 64);
                *self.current_mut() = row;
            }
            WolframCommand::Clear => self.current_mut().fill(0),
        }
        if self.generation == 0 {
            self.reset_point = None;
        }
        Ok(())
    }

    fn params(&self) -> ParamSet {
        let c = &self.next_config;
        ParamSet::new()
            .with("rule", ParamValue::Int(i64::from(self.pending_rule.unwrap_or(self.rule))))
            .with("boundary", ParamValue::Text(self.pending_boundary.unwrap_or(self.boundary).as_str().into()))
            .with("width", ParamValue::Int(i64::from(c.width)))
            .with(
                "start",
                ParamValue::Text(match c.start {
                    EcaStart::SingleCell => "single_cell".into(),
                    EcaStart::Random => "random".into(),
                }),
            )
            .with("density", ParamValue::Float(c.density))
            .with("history", ParamValue::Int(i64::from(c.history)))
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let spec =
            Self::specs().into_iter().find(|s| s.key == key).ok_or_else(|| SimError::UnknownParameter(key.into()))?;
        let value = spec.validate(&value)?;
        let mut next = self.next_config.clone();
        match key {
            "rule" => {
                self.pending_rule = Some(value.as_i64().unwrap_or(30) as u8);
                return Ok(spec.apply);
            }
            "boundary" => {
                self.pending_boundary = value.as_str().and_then(Boundary::parse);
                return Ok(spec.apply);
            }
            "width" => next.width = value.as_i64().unwrap_or(3) as u32,
            "start" => {
                next.start = if value.as_str() == Some("random") { EcaStart::Random } else { EcaStart::SingleCell }
            }
            "density" => next.density = value.as_f64().unwrap_or(0.5),
            "history" => next.history = value.as_i64().unwrap_or(1) as u32,
            _ => return Err(SimError::UnknownParameter(key.into())),
        }
        next.validate().map_err(|e| SimError::invalid_param(key, e.to_string()))?;
        self.next_config = next;
        Ok(spec.apply)
    }

    fn metrics(&self) -> Metrics {
        let ones: u64 = self.rows[self.rows.len() - 1].iter().map(|w| u64::from(w.count_ones())).sum();
        Metrics::new(self.generation, self.generation as f64)
            .with("ones", ones as f64)
            .with("density", ones as f64 / self.width() as f64)
            .with("generation", self.generation as f64)
    }

    /// The space-time diagram: R = cell value, B = 255 where the cell differs
    /// from the cell above it (it changed in that step), A = fraction of ones.
    fn view(&self, request: &GridViewRequest) -> GridView {
        let first = self.first_row();
        let bounds = CellRect { x0: 0, y0: first as i64, x1: self.width() as i64 - 1, y1: self.generation as i64 };
        let grid = TexelGrid::plan(request, Some(bounds));
        let len = grid.len();
        let (mut ones, mut changed) = (vec![0u32; len], vec![0u32; len]);
        if let Some(clip) = grid.clip {
            let width = grid.width as usize;
            let (k0, k1) = ((clip.x0 / 64) as usize, (clip.x1 / 64) as usize);
            for t in clip.y0..=clip.y1 {
                let i = (t as u64 - first) as usize;
                let row = &self.rows[i];
                let above = if i > 0 { Some(&self.rows[i - 1]) } else { None };
                let base = grid.row(t) * width;
                for k in k0..=k1 {
                    let lo = if k == k0 { (clip.x0 % 64) as u32 } else { 0 };
                    let hi = if k == k1 { (clip.x1 % 64) as u32 } else { 63 };
                    let mask = crate::engine::swar::bit_range(lo, hi);
                    count_word(row[k] & mask, 64 * k as i64, &grid, &mut ones[base..base + width]);
                    if let Some(above) = above {
                        count_word((row[k] ^ above[k]) & mask, 64 * k as i64, &grid, &mut changed[base..base + width]);
                    }
                }
            }
        }
        let state: Vec<u8> = ones.iter().map(|&c| u8::from(c > 0)).collect();
        let activity: Vec<u8> = changed.iter().map(|&c| if c > 0 { 255 } else { 0 }).collect();
        grid.assemble(&ones, &state, &vec![0u8; len], &activity)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        let saved = Saved {
            config: self.config.clone(),
            rule: self.rule,
            pending_rule: self.pending_rule,
            boundary: self.boundary,
            pending_boundary: self.pending_boundary,
            rows: self.rows.iter().cloned().collect(),
            generation: self.generation,
            rng: self.rng.clone(),
            seed: self.seed,
            reset_point: self.reset_point.clone(),
            next_config: self.next_config.clone(),
        };
        Snapshot::encode(WOLFRAM_MODEL_ID, SCHEMA_VERSION, self.generation, &saved)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let saved: Saved = snapshot.decode(WOLFRAM_MODEL_ID, SCHEMA_VERSION)?;
        saved.config.validate()?;
        saved.next_config.validate()?;
        let words = (saved.config.width as usize).div_ceil(64);
        let rows_ok = !saved.rows.is_empty()
            && saved.rows.len() <= saved.config.history as usize
            && saved.rows.len() as u64 <= saved.generation + 1
            && saved.rows.iter().all(|r| r.len() == words)
            && saved.reset_point.as_ref().is_none_or(|(row, _)| row.len() == words);
        if !rows_ok {
            return Err(SimError::Serialization("snapshot rows do not match the configuration".into()));
        }
        self.config = saved.config;
        self.rule = saved.rule;
        self.pending_rule = saved.pending_rule;
        self.boundary = saved.boundary;
        self.pending_boundary = saved.pending_boundary;
        self.rows = saved.rows.into_iter().collect();
        self.generation = saved.generation;
        self.rng = saved.rng;
        self.seed = saved.seed;
        self.reset_point = saved.reset_point;
        self.next_config = saved.next_config;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.rows.len() * self.width().div_ceil(64) * 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(cells: &[u8]) -> Vec<u64> {
        let mut row = vec![0u64; cells.len().div_ceil(64)];
        for (x, &c) in cells.iter().enumerate() {
            if c == 1 {
                row[x / 64] |= 1 << (x % 64);
            }
        }
        row
    }

    #[test]
    fn rule_table_lookup() {
        // Rule 30 on 0100 with fixed-0 ends: neighbourhoods 001, 010, 100, 000 -> 1, 1, 1, 0.
        assert_eq!(eca_step(30, &bits(&[0, 1, 0, 0]), 4, Boundary::Fixed0), bits(&[1, 1, 1, 0]));
        // Fixed-1 ends: both end cells see a 1 beyond the row (100 -> 1 and 001 -> 1).
        assert_eq!(eca_step(30, &bits(&[0, 0, 0]), 3, Boundary::Fixed1), bits(&[1, 0, 1]));
        // Periodic rule 90 (l xor r) on a ring of 3.
        assert_eq!(eca_step(90, &bits(&[1, 0, 0]), 3, Boundary::Periodic), bits(&[0, 1, 1]));
    }
}
