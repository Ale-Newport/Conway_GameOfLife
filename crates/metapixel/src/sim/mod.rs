//! [`MetapixelSim`]: the "Conway within Conway" mode (model id
//! `life.metapixel`).
//!
//! The model state is a real B3/S23 pattern — Brice Due's OTCA metapixel
//! tiles OR-ed at pitch 2048 — evolved by Hashlife. Macro states are only
//! ever *read* from it (at `t = (n + 1)·P`). A separate, clearly labelled
//! accelerated macro representation predicts each metageneration so that
//! disagreements can be reported.

mod command;
mod history;
mod params;
mod state;
mod view;

use std::sync::Arc;

use hashlife::Universe;
use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamValue, SimError, SimRng, Simulation, Snapshot,
};

pub use command::MetapixelCommand;
pub use history::{AmbiguousTile, HISTORY_CAPACITY, Predictor, ReadoutHistory, ReadoutRecord};
pub use params::{MAX_NODE_LIMIT, MAX_STEP_EXPONENT, StepSize};
pub use view::{
    CycleHalf, CycleStage, GridView, GridViewRequest, MAX_LIVE_READOUT_TILES, MAX_LOD_LOG2, MAX_TEXTURE_SIDE,
    MacroTile, MacroView, MetapixelView, MetapixelViewRequest, NeighbourChannel, PhaseInfo, ScaleInfo, TileInspection,
};

use crate::accelerated::{ACCELERATED_LABEL, EXACT_LABEL, MacroModel};
use crate::annotations::tile_annotations;
use crate::build::{BuildStats, construction_macrocell};
use crate::error::MetapixelError;
use crate::macro_grid::MacroGrid;
use crate::programming::{Neighbourhood, Programming, parse_rule};
use crate::readout::{LiveReading, TileGeometry, TileReading, core_count, core_counts, tile_populations};
use crate::scenario::Scenario;
use crate::spec::{Direction, PERIOD};
use crate::tile::{ProgrammedTiles, TileSet};

/// Model identifier used in descriptors and snapshots.
pub const MODEL_ID: &str = "life.metapixel";
/// Snapshot payload version written by this build.
pub const SCHEMA_VERSION: u32 = 1;
/// [`SimRng::derive`] stream used for random scenario patterns.
pub const SCENARIO_STREAM: u64 = 1;

/// What a construction is built from (rule, neighbourhood, size, seed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldConfig {
    /// Macro rule and neighbourhood programmed into every tile.
    pub programming: Programming,
    /// Grid width in tiles.
    pub width: u32,
    /// Grid height in tiles.
    pub height: u32,
    /// Seed of the random generator (Randomize command).
    pub seed: u64,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self { programming: Programming::conway(), width: 3, height: 3, seed: 1 }
    }
}

/// The OTCA metapixel construction as a [`Simulation`].
///
/// * One tick advances [`StepSize::generations`] physical generations;
///   [`Simulation::tick`] reports the physical generation and
///   [`Simulation::advance`]`(n)` advances exactly `n` generations.
/// * Whatever the step, evolution stops at every multiple of `P = 35 328` to
///   read every tile ([`ReadoutRecord`]) and compare it with the
///   accelerated prediction.
/// * Commands edit the macro pattern and rebuild the construction at
///   generation 0; rule, neighbourhood, size and seed are
///   [`ApplyPolicy::RequiresReset`] parameters.
/// * If Hashlife reports an error the model halts
///   ([`Simulation::is_finished`]); a command, reset or restore clears it.
pub struct MetapixelSim {
    tiles: Arc<TileSet>,
    programmed: ProgrammedTiles,
    model: MacroModel,
    active: WorldConfig,
    pending: WorldConfig,
    initial: MacroGrid,
    universe: Universe,
    geometry: TileGeometry,
    build_stats: BuildStats,
    step: StepSize,
    rng: SimRng,
    history: ReadoutHistory,
    predictor: Predictor,
    halted: Option<String>,
}

impl MetapixelSim {
    /// Builds the construction of `pattern` (whose size must match
    /// `config`).
    ///
    /// # Errors
    /// [`MetapixelError::InvalidPattern`] for a size mismatch and the build
    /// errors of [`crate::build_universe`].
    pub fn new(tiles: Arc<TileSet>, config: WorldConfig, pattern: MacroGrid) -> Result<Self, MetapixelError> {
        if (pattern.width(), pattern.height()) != (config.width, config.height) {
            return Err(MetapixelError::InvalidPattern(format!(
                "the pattern is {}×{} but the configuration asks for {}×{}",
                pattern.width(),
                pattern.height(),
                config.width,
                config.height
            )));
        }
        let programmed = tiles.program(config.programming);
        let mut sim = Self {
            model: MacroModel::new(config.programming),
            programmed,
            tiles,
            active: config,
            pending: config,
            initial: pattern.clone(),
            universe: Universe::conway(),
            geometry: TileGeometry::at_origin(config.width, config.height),
            build_stats: BuildStats { distinct_squares: 0, document_nodes: 0, document_bytes: 0 },
            step: StepSize::default(),
            rng: SimRng::seed_from_u64(config.seed),
            history: ReadoutHistory::default(),
            predictor: Predictor::new(pattern.clone()),
            halted: None,
        };
        sim.rebuild(None, pattern)?;
        Ok(sim)
    }

    /// Builds a scenario (see [`crate::scenario`]). Random initial patterns
    /// are drawn from the independent stream [`SCENARIO_STREAM`] of the seed,
    /// so the model's own generator starts (and restarts on
    /// [`Simulation::reset`]) from the plain seed.
    ///
    /// # Errors
    /// Scenario validation errors and the errors of [`MetapixelSim::new`].
    pub fn from_scenario(tiles: Arc<TileSet>, scenario: &Scenario) -> Result<Self, MetapixelError> {
        let config = scenario.config()?;
        let step = scenario.step_size()?;
        let pattern = scenario.initial_pattern(&mut SimRng::derive(config.seed, SCENARIO_STREAM))?;
        let mut sim = Self::new(tiles, config, pattern)?;
        sim.step = step;
        Ok(sim)
    }

    /// The physical pattern (read-only).
    pub fn universe(&self) -> &Universe {
        &self.universe
    }

    /// Placement of the tiles (tile `(0, 0)` at the origin).
    pub fn geometry(&self) -> TileGeometry {
        self.geometry
    }

    /// The macro pattern the construction was built from (metagen 0).
    pub fn initial_pattern(&self) -> &MacroGrid {
        &self.initial
    }

    /// The configuration of the running construction.
    pub fn active_config(&self) -> WorldConfig {
        self.active
    }

    /// The configuration the next reset will use.
    pub fn pending_config(&self) -> WorldConfig {
        self.pending
    }

    /// Generations per tick.
    pub fn step_size(&self) -> StepSize {
        self.step
    }

    /// Statistics of the last build.
    pub fn build_stats(&self) -> BuildStats {
        self.build_stats
    }

    /// The programmed tiles of the running construction.
    pub fn programmed_tiles(&self) -> &ProgrammedTiles {
        &self.programmed
    }

    /// Exact readouts so far.
    pub fn history(&self) -> &ReadoutHistory {
        &self.history
    }

    /// Why the model halted, if it did.
    pub fn halt_reason(&self) -> Option<&str> {
        self.halted.as_deref()
    }

    /// Accelerated prediction (**not** a physical result) for `metagen`, if
    /// it is not older than the predictor's current state.
    pub fn accelerated_prediction(&self, metagen: u64) -> Option<MacroGrid> {
        self.predictor.peek(&self.model, metagen)
    }

    /// Threshold readings of every tile's core at the current generation,
    /// row-major.
    ///
    /// # Errors
    /// The errors of [`crate::readout::core_counts`] (not expected for the
    /// grids this model builds).
    pub fn live_readings(&self) -> Result<Vec<LiveReading>, MetapixelError> {
        Ok(core_counts(&self.universe, &self.geometry)?.into_iter().map(LiveReading::classify).collect())
    }

    /// Rebuilds the construction of `pattern` at generation 0, with newly
    /// programmed tiles when `programmed` is given. Nodes are interned into
    /// the existing Hashlife store so memoised futures of unchanged squares
    /// are reused. On error nothing changes.
    fn rebuild(&mut self, programmed: Option<ProgrammedTiles>, pattern: MacroGrid) -> Result<(), MetapixelError> {
        let tiles = programmed.as_ref().unwrap_or(&self.programmed);
        let (text, stats) = construction_macrocell(tiles, &pattern);
        hashlife::macrocell::read_into(&mut self.universe, &text)?;
        if let Some(programmed) = programmed {
            self.model = MacroModel::new(programmed.programming());
            self.programmed = programmed;
        }
        self.geometry = TileGeometry::at_origin(pattern.width(), pattern.height());
        self.build_stats = stats;
        self.predictor = Predictor::new(pattern.clone());
        self.initial = pattern;
        self.history = ReadoutHistory::default();
        self.halted = None;
        Ok(())
    }

    /// Advances `generations` physical generations, stopping at every
    /// multiple of P to read the tiles. Returns the generations advanced.
    fn run(&mut self, generations: u64) -> u64 {
        if self.halted.is_some() {
            return 0;
        }
        let start = self.universe.generation();
        let Some(target) = start.checked_add(generations) else {
            self.halted = Some(format!("cannot advance {generations} generations from {start}: counter overflow"));
            return 0;
        };
        while self.universe.generation() < target {
            let now = self.universe.generation();
            let boundary = (now / PERIOD + 1).saturating_mul(PERIOD);
            let stop = boundary.min(target);
            if let Err(error) = self.universe.advance(stop - now) {
                self.halted = Some(error.to_string());
                break;
            }
            let now = self.universe.generation();
            if now.is_multiple_of(PERIOD) && now > 0 {
                self.read_metagen(now / PERIOD - 1);
            }
        }
        self.universe.generation() - start
    }

    /// Reads every tile at `t = (metagen + 1)·P` and records the comparison
    /// with the accelerated prediction. A failed readout halts the model
    /// (it is never replaced by a guess).
    fn read_metagen(&mut self, metagen: u64) {
        let readings: Vec<TileReading> = match core_counts(&self.universe, &self.geometry) {
            Ok(counts) => counts.into_iter().map(TileReading::classify).collect(),
            Err(error) => {
                self.halted = Some(format!("readout of metagen {metagen}: {error}"));
                return;
            }
        };
        let predicted = self.predictor.advance_to(&self.model, metagen).clone();
        match ReadoutRecord::new(metagen, self.universe.generation(), &readings, predicted) {
            Ok(record) => self.history.push(record),
            Err(error) => self.halted = Some(format!("readout of metagen {metagen}: {error}")),
        }
    }

    /// Applies the pending configuration and rebuilds (atomically: on error
    /// the running construction is kept).
    fn apply_pending(&mut self) -> Result<(), MetapixelError> {
        let pending = self.pending;
        let pattern = self.initial.resized(pending.width, pending.height)?;
        let programmed =
            (pending.programming != self.active.programming).then(|| self.tiles.program(pending.programming));
        self.rebuild(programmed, pattern)?;
        self.active = pending;
        self.rng = SimRng::seed_from_u64(pending.seed);
        Ok(())
    }

    /// The pattern a command produces (validated; nothing changes yet).
    fn edited_pattern(&mut self, command: MetapixelCommand) -> Result<MacroGrid, MetapixelError> {
        let (columns, rows) = (self.initial.width(), self.initial.height());
        let check = |col: i64, row: i64| {
            if (0..i64::from(columns)).contains(&col) && (0..i64::from(rows)).contains(&row) {
                Ok(())
            } else {
                Err(MetapixelError::TileOutOfRange { col, row, columns, rows })
            }
        };
        let mut pattern = self.initial.clone();
        match command {
            MetapixelCommand::SetTile { col, row, on } => {
                check(col.into(), row.into())?;
                pattern.set(col, row, on);
            }
            MetapixelCommand::ToggleTile { col, row } => {
                check(col.into(), row.into())?;
                pattern.set(col, row, !pattern.get(col, row));
            }
            MetapixelCommand::SetPattern(grid) => pattern = grid,
            MetapixelCommand::PlaceRle { rle, col, row } => {
                let parsed = patterns::rle::parse(&rle).map_err(|e| MetapixelError::InvalidPattern(e.to_string()))?;
                for (x, y) in parsed.normalized().live_cells() {
                    let (c, r) = (col.saturating_add(x), row.saturating_add(y));
                    check(c, r)?;
                    pattern.set(c as u32, r as u32, true);
                }
            }
            MetapixelCommand::Randomize { density } => {
                pattern = MacroGrid::random(columns, rows, density, &mut self.rng)?;
            }
            MetapixelCommand::Clear => pattern = MacroGrid::new(columns, rows)?,
            MetapixelCommand::RestartFromLatestReadout => {
                let record = self
                    .history
                    .latest()
                    .ok_or_else(|| MetapixelError::InvalidPattern("no metageneration has been read yet".into()))?;
                if let Some(a) = record.ambiguous.first() {
                    return Err(MetapixelError::InvalidPattern(format!(
                        "tile ({}, {}) was ambiguous in metagen {} (core count {})",
                        a.col, a.row, record.metagen, a.count
                    )));
                }
                pattern = record.exact.clone();
            }
        }
        Ok(pattern)
    }

    /// The macro view, or `None` if the tiles cannot be counted (not
    /// expected for the grids this model builds).
    fn macro_view(&self, live: bool) -> Option<MacroView> {
        let latest = self.history.latest();
        let accelerated_metagen = latest.map_or(0, |r| r.metagen);
        let accelerated = match latest {
            Some(record) => record.accelerated.clone(),
            None => self.initial.clone(),
        };
        let populations = tile_populations(&self.universe, &self.geometry).ok()?;
        let live: Option<Vec<LiveReading>> = if live && self.geometry.tile_count() <= MAX_LIVE_READOUT_TILES {
            Some(self.live_readings().ok()?)
        } else {
            None
        };
        let columns = self.geometry.columns;
        let tiles = populations
            .into_iter()
            .enumerate()
            .map(|(i, population)| {
                let (col, row) = (i as u32 % columns, i as u32 / columns);
                MacroTile {
                    exact: latest.map(|r| r.reading(col, row)),
                    accelerated: accelerated.get(col, row),
                    mismatch: latest
                        .is_some_and(|r| r.mismatches.binary_search_by_key(&(row, col), |&(c, r)| (r, c)).is_ok()),
                    population,
                    live: live.as_ref().and_then(|l| l.get(i).copied()),
                }
            })
            .collect();
        Some(MacroView {
            columns,
            rows: self.geometry.rows,
            exact_metagen: latest.map(|r| r.metagen),
            exact_generation: latest.map(|r| r.generation),
            accelerated_metagen,
            tiles,
            mismatch_count: latest.map_or(0, |r| r.mismatches.len()),
            ambiguous_count: latest.map_or(0, |r| r.ambiguous.len()),
            exact_label: EXACT_LABEL.to_string(),
            accelerated_label: ACCELERATED_LABEL.to_string(),
        })
    }

    /// Inspection data of tile `(col, row)`, or `None` outside the grid.
    fn inspection(&self, col: u32, row: u32) -> Option<TileInspection> {
        let count = core_count(&self.universe, &self.geometry, col, row).ok()?;
        let (origin_x, origin_y) = self.geometry.tile_origin(col, row);
        let rect = self.geometry.tile_rect(col, row);
        let population = crate::readout::count_live_in_rect(&self.universe, rect).ok()?;
        let latest = self.history.latest();
        let accelerated = latest.map_or_else(|| self.initial.get(col, row), |r| r.accelerated.get(col, row));
        let neighbours = Direction::ALL
            .into_iter()
            .map(|direction| {
                let (dx, dy) = direction.offset();
                let (c, r) = (i64::from(col) + dx, i64::from(row) + dy);
                let inside = (0..i64::from(self.geometry.columns)).contains(&c)
                    && (0..i64::from(self.geometry.rows)).contains(&r);
                NeighbourChannel {
                    direction,
                    tile: inside.then_some((c as u32, r as u32)),
                    heard: self.active.programming.neighbourhood.listens(direction),
                }
            })
            .collect();
        Some(TileInspection {
            col,
            row,
            origin_x,
            origin_y,
            annotations: tile_annotations(self.active.programming),
            core_count: count,
            live: LiveReading::classify(count),
            population,
            exact: latest.map(|r| r.reading(col, row)),
            accelerated,
            neighbours,
        })
    }
}

impl Simulation for MetapixelSim {
    type Command = MetapixelCommand;
    type ViewRequest = MetapixelViewRequest;
    type View = MetapixelView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(MODEL_ID, "Conway within Conway (OTCA metapixel)", ModelFamily::MetaCellular)
            .summary(
                "Brice Due's OTCA metapixel: every macro cell is a 2048×2048 B3/S23 construction that computes a \
                 programmable Life-like rule. The physical pattern is run exactly with Hashlife and the macro \
                 states are read from it every 35 328 generations; a separately labelled accelerated macro model \
                 predicts them for comparison.",
            )
            .rules(&params::rules_text(self.active.programming, self.active.width, self.active.height, self.step))
            .params(params::param_specs())
            .metrics(params::metric_specs())
            .time("generation", 1.0)
    }

    fn tick(&self) -> u64 {
        self.universe.generation()
    }

    fn step(&mut self) {
        self.run(self.step.generations());
    }

    fn advance(&mut self, max_ticks: u64) -> u64 {
        self.run(max_ticks)
    }

    fn reset(&mut self) {
        if let Err(error) = self.apply_pending() {
            self.halted = Some(format!("reset failed: {error}"));
        }
    }

    fn apply(&mut self, command: MetapixelCommand) -> Result<(), SimError> {
        // Randomize draws from the generator: keep its state if anything fails.
        let rng_before = self.rng.clone();
        let rebuilt = self.edited_pattern(command).and_then(|pattern| {
            let size_changed = (pattern.width(), pattern.height()) != (self.initial.width(), self.initial.height());
            self.rebuild(None, pattern).map(|()| size_changed)
        });
        let size_changed = rebuilt.inspect_err(|_| self.rng = rng_before)?;
        if size_changed {
            self.active.width = self.initial.width();
            self.active.height = self.initial.height();
            self.pending.width = self.active.width;
            self.pending.height = self.active.height;
        }
        Ok(())
    }

    fn params(&self) -> ParamSet {
        let p = &self.pending;
        ParamSet::new()
            .with("rule", ParamValue::Text(p.programming.rule.to_string()))
            .with("neighbourhood", ParamValue::Text(p.programming.neighbourhood.to_string()))
            .with("width", ParamValue::Int(i64::from(p.width)))
            .with("height", ParamValue::Int(i64::from(p.height)))
            .with("seed", ParamValue::Int(i64::try_from(p.seed).unwrap_or(i64::MAX)))
            .with("step", ParamValue::Text(self.step.value()))
            .with("node_limit", ParamValue::Int(i64::try_from(self.universe.node_limit()).unwrap_or(i64::MAX)))
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let spec = params::param_specs()
            .into_iter()
            .find(|s| s.key == key)
            .ok_or_else(|| SimError::UnknownParameter(key.to_string()))?;
        let value = spec.validate(&params::normalise_step_value(key, value))?;
        let invalid = |e: MetapixelError| SimError::invalid_param(key, e.to_string());
        let text = || value.as_str().unwrap_or_default().to_string();
        // Integer parameters were range-checked by `validate`.
        let int = || value.as_i64().unwrap_or_default();
        match key {
            "rule" => self.pending.programming.rule = parse_rule(&text()).map_err(invalid)?,
            "neighbourhood" => {
                self.pending.programming.neighbourhood = Neighbourhood::parse(&text()).map_err(invalid)?
            }
            "width" => self.pending.width = int() as u32,
            "height" => self.pending.height = int() as u32,
            "seed" => self.pending.seed = int() as u64,
            "step" => {
                self.step = StepSize::parse(&text())
                    .ok_or_else(|| SimError::invalid_param(key, format!("`{}` is not a step size", text())))?;
            }
            _ => self.universe.set_node_limit(int() as usize),
        }
        Ok(spec.apply)
    }

    fn metrics(&self) -> Metrics {
        let t = self.universe.generation();
        let latest = self.history.latest();
        Metrics::new(t, t as f64)
            .with("population", self.universe.population() as f64)
            .with("generation", t as f64)
            .with("metagen", crate::spec::displayed_metagen(t) as f64)
            .with("phase", (t % PERIOD) as f64)
            .with("readouts", self.history.total_readouts() as f64)
            .with("exact_on_tiles", latest.map_or(0, |r| r.exact.population()) as f64)
            .with(
                "accelerated_on_tiles",
                latest.map_or_else(|| self.initial.population(), |r| r.accelerated.population()) as f64,
            )
            .with("mismatches", latest.map_or(0, |r| r.mismatches.len()) as f64)
            .with("ambiguous_tiles", latest.map_or(0, |r| r.ambiguous.len()) as f64)
            .with("total_mismatches", self.history.total_mismatches() as f64)
            .with("node_count", self.universe.node_count() as f64)
            .with("memory_bytes", self.universe.memory_bytes() as f64)
    }

    fn view(&self, request: &MetapixelViewRequest) -> MetapixelView {
        let micro = request.micro.map(|r| {
            let view = view::grid_view(&self.universe, &r);
            let scale = view::scale_info(&view, &r, &self.geometry);
            (view, scale)
        });
        let (micro, scale) = match micro {
            Some((v, s)) => (Some(v), Some(s)),
            None => (None, None),
        };
        MetapixelView {
            micro,
            scale,
            macro_grid: request.macro_grid.then(|| self.macro_view(request.live_readings)).flatten(),
            inspection: request.inspect.and_then(|(c, r)| self.inspection(c, r)),
            phase: PhaseInfo::at(self.universe.generation(), self.history.latest().map(|r| r.metagen)),
        }
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(MODEL_ID, SCHEMA_VERSION, self.universe.generation(), &state::SavedState::capture(self))
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let saved: state::SavedState = snapshot.decode(MODEL_ID, SCHEMA_VERSION)?;
        saved.apply(self).map_err(|e| SimError::Serialization(e.to_string()))
    }

    fn memory_bytes(&self) -> usize {
        self.universe.memory_bytes() + self.history.memory_bytes()
    }

    fn is_finished(&self) -> bool {
        self.halted.is_some()
    }
}
