//! `LifeSim`: Conway's Game of Life and every two-state Life-like rule.
//!
//! The same model serves the **Classic Conway** mode (model id
//! [`CLASSIC_MODEL_ID`], default preset "Conway (B3/S23, unaltered)") and the
//! Life-like part of the **CA Laboratory** ([`LAB_MODEL_ID`], presets
//! HighLife, Seeds, Day & Night...). It runs on any [`crate::engine`]
//! engine, switches engines at runtime without losing a cell, and offers the
//! editing tools of the desktop application as [`LifeCommand`]s.

mod commands;
mod config;
mod params;
mod snapshot;
mod tracking;

pub use commands::{LifeCommand, MirrorAxis, PasteMode};
pub use config::{InitItem, LifeConfig, PatternLibrary, PatternSource, RulePreset, place_pattern};
pub(crate) use params::{parse_topology, topology_name};
pub use tracking::TRAIL_DECAY;

use patterns::{LifeRule, Pattern};
use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamValue, SimError, SimRng, Simulation,
    Snapshot, Topology,
};

use crate::engine::{
    AUTO_HASHLIFE_MIN_POPULATION, AUTO_REGULARITY_MAX_POPULATION, DenseEngine, EngineChoice, EngineKind, GenStats,
    LifeEngine, MAX_SWITCH_CELLS, SelectionInput, create_engine, select_engine, tile_regularity, transfer,
};
use crate::error::CaError;
use crate::geometry::{CellRect, MAX_EDIT_CELLS, MAX_FILL_CELLS, World, check_fill};
use crate::view::{GridView, GridViewRequest, TexelGrid};
use config::ResolvedInit;
use snapshot::ResetPoint;
use tracking::Tracker;

/// Model id of the Classic Conway mode.
pub const CLASSIC_MODEL_ID: &str = "life.classic";
/// Model id of the Life-like rules of the CA Laboratory.
pub const LAB_MODEL_ID: &str = "ca.life_like";
/// Snapshot payload version written by this build.
pub const SCHEMA_VERSION: u32 = 1;
/// Largest number of generations per tick.
pub const MAX_GENERATIONS_PER_TICK: u64 = 1 << 30;

/// Which application mode a [`LifeSim`] serves (only the model id, name and
/// defaults differ).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifeMode {
    /// Classic Conway mode (`life.classic`).
    #[default]
    Classic,
    /// CA Laboratory, Life-like rules (`ca.life_like`).
    Lab,
}

impl LifeMode {
    /// The model id.
    pub fn model_id(self) -> &'static str {
        match self {
            Self::Classic => CLASSIC_MODEL_ID,
            Self::Lab => LAB_MODEL_ID,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Classic => "Classic Conway",
            Self::Lab => "Life-like rules",
        }
    }
}

/// Conway's Life and the Life-like rules as a [`Simulation`].
///
/// One tick advances `generations_per_tick` generations and
/// [`Simulation::tick`] reports the generation count. Parameter changes
/// follow their [`ApplyPolicy`]: the rule and the generations per tick apply
/// at the start of the next tick, the world (topology, size) on reset, the
/// engine and tracking switches immediately (state is transferred exactly).
///
/// [`Simulation::reset`] returns to the state at generation 0, as Golly
/// does: the scenario's initial content plus any edit made at generation 0.
/// After a world change (topology or size) reset rebuilds the initial content
/// from the scenario and the seed.
pub struct LifeSim {
    mode: LifeMode,
    seed: u64,
    /// World currently running.
    world: World,
    /// World used by the next reset (differs after a RequiresReset change).
    next_world: World,
    init: Vec<ResolvedInit>,
    engine: Box<dyn LifeEngine>,
    engine_choice: EngineChoice,
    rule: LifeRule,
    preset: RulePreset,
    pending_rule: Option<(LifeRule, RulePreset)>,
    generations_per_tick: u64,
    pending_generations: Option<u64>,
    generation: u64,
    rng: SimRng,
    tracker: Tracker,
    last_stats: Option<GenStats>,
    reset_point: Option<ResetPoint>,
    halted: Option<String>,
}

impl LifeSim {
    /// Builds the model from a configuration.
    ///
    /// # Errors
    /// Invalid rule, world, engine combination, unknown pattern ids or
    /// unreadable pattern text, invalid random fills.
    pub fn new(config: &LifeConfig, seed: u64, library: &PatternLibrary, mode: LifeMode) -> Result<Self, CaError> {
        let (rule, preset) = config.resolve_rule()?;
        let world = World::new(config.topology, config.width, config.height)?;
        if config.generations_per_tick == 0 || config.generations_per_tick > MAX_GENERATIONS_PER_TICK {
            return Err(CaError::invalid(format!(
                "generations per tick must be between 1 and {MAX_GENERATIONS_PER_TICK}"
            )));
        }
        let init = config
            .init
            .iter()
            .map(|item| match item {
                InitItem::Pattern { source, x, y, rotation, mirror } => {
                    let pattern = library.load_source(source)?;
                    let cells = place_pattern(&pattern, *x, *y, *rotation, *mirror)?;
                    Ok(ResolvedInit::Cells(cells.into_iter().map(|(x, y, _)| (x, y)).collect()))
                }
                InitItem::Random { region, density, symmetry } => {
                    Ok(ResolvedInit::Random { region: *region, density: *density, symmetry: *symmetry })
                }
            })
            .collect::<Result<Vec<_>, CaError>>()?;
        let kind = Self::choose(config.engine, world, rule, 0, config.generations_per_tick, None)?;
        let mut sim = Self {
            mode,
            seed,
            world,
            next_world: world,
            init,
            engine: create_engine(kind, world, rule)?,
            engine_choice: config.engine,
            rule,
            preset,
            pending_rule: None,
            generations_per_tick: config.generations_per_tick,
            pending_generations: None,
            generation: 0,
            rng: SimRng::seed_from_u64(seed),
            tracker: Tracker::default(),
            last_stats: None,
            reset_point: None,
            halted: None,
        };
        sim.rebuild()?;
        sim.tracker.configure(config.track_ages, config.track_activity, || sim.engine.export_cells());
        Ok(sim)
    }

    /// Classic Conway on a 256×256 torus filled with a 30 % random soup.
    pub fn conway(seed: u64) -> Self {
        let config = LifeConfig::default();
        let library = PatternLibrary::builtin();
        // The default configuration is valid by construction (checked by tests).
        match Self::new(&config, seed, &library, LifeMode::Classic) {
            Ok(sim) => sim,
            Err(e) => unreachable!("the default Life configuration is valid: {e}"),
        }
    }

    /// The mode (model id) of this instance.
    pub fn mode(&self) -> LifeMode {
        self.mode
    }

    /// The rule currently executed.
    pub fn rule(&self) -> LifeRule {
        self.rule
    }

    /// The preset of the current rule.
    pub fn preset(&self) -> RulePreset {
        self.preset
    }

    /// The world currently running.
    pub fn world(&self) -> World {
        self.world
    }

    /// The engine currently running.
    pub fn engine_kind(&self) -> EngineKind {
        self.engine.kind()
    }

    /// The engine choice (automatic or fixed).
    pub fn engine_choice(&self) -> EngineChoice {
        self.engine_choice
    }

    /// Read-only access to the engine.
    pub fn engine(&self) -> &dyn LifeEngine {
        self.engine.as_ref()
    }

    /// Generations advanced per tick (after pending changes).
    pub fn generations_per_tick(&self) -> u64 {
        self.pending_generations.unwrap_or(self.generations_per_tick)
    }

    /// Number of live cells.
    pub fn population(&self) -> u64 {
        self.engine.population()
    }

    /// Whether a cell is alive.
    pub fn cell(&self, x: i64, y: i64) -> bool {
        self.engine.get_cell(x, y)
    }

    /// Every live cell, sorted row-major.
    pub fn live_cells(&self) -> Vec<(i64, i64)> {
        self.engine.export_cells()
    }

    /// Age of a live cell in generations (0 when dead or not tracked).
    pub fn age(&self, x: i64, y: i64) -> u8 {
        self.tracker.age(x, y)
    }

    /// Activity of a cell (0..=255; 0 when not tracked).
    pub fn activity(&self, x: i64, y: i64) -> u8 {
        self.tracker.activity(x, y)
    }

    /// Births and deaths of the last tick (`None` before the first tick, or
    /// when Hashlife jumped without tracking).
    pub fn last_stats(&self) -> Option<GenStats> {
        self.last_stats
    }

    /// Why the model stopped, if it did (e.g. Hashlife reaching the edge of
    /// the 64-bit plane).
    pub fn halt_reason(&self) -> Option<&str> {
        self.halted.as_deref()
    }

    /// Copies the live cells of `rect` into a pattern whose coordinates are
    /// relative to the rectangle's top-left corner (so pasting it at
    /// `(rect.x0, rect.y0)` restores it). The pattern carries the current rule.
    ///
    /// # Errors
    /// [`CaError::LimitExceeded`] when `rect` holds more than
    /// [`MAX_EDIT_CELLS`] live cells.
    pub fn copy_region(&self, rect: CellRect) -> Result<Pattern, CaError> {
        let cells = self.region_cells(rect)?.into_iter().map(|(x, y)| (x - rect.x0, y - rect.y0));
        Ok(Pattern::from_live_cells(cells).with_rule(self.rule.to_string()))
    }

    /// Cut: [`LifeSim::copy_region`] followed by
    /// [`LifeCommand::ClearRegion`] of the same rectangle (an edit, like any
    /// command). Paste the returned pattern with [`LifeCommand::Paste`].
    ///
    /// # Errors
    /// Errors of [`LifeSim::copy_region`] (nothing is cleared then) and of
    /// [`Simulation::apply`].
    pub fn cut_region(&mut self, rect: CellRect) -> Result<Pattern, SimError> {
        let pattern = self.copy_region(rect)?;
        Simulation::apply(self, LifeCommand::ClearRegion { rect })?;
        Ok(pattern)
    }

    /// The live cells of `rect`, sorted row-major, for the commands and
    /// queries that list a region's cells one by one.
    ///
    /// # Errors
    /// [`CaError::LimitExceeded`] when `rect` holds more than
    /// [`MAX_EDIT_CELLS`] live cells (counted before anything is listed).
    fn region_cells(&self, rect: CellRect) -> Result<Vec<(i64, i64)>, CaError> {
        if self.engine.count_in_rect(rect, MAX_EDIT_CELLS + 1) > MAX_EDIT_CELLS {
            return Err(CaError::LimitExceeded(format!(
                "the region holds more than {MAX_EDIT_CELLS} live cells, the most a copy, move, rotation or mirror \
                 handles"
            )));
        }
        Ok(self.engine.cells_in_rect(rect))
    }

    /// Picks the engine for a choice.
    fn choose(
        choice: EngineChoice,
        world: World,
        rule: LifeRule,
        population: u64,
        generations_per_tick: u64,
        regularity: Option<f64>,
    ) -> Result<EngineKind, CaError> {
        match choice.fixed() {
            Some(kind) => {
                kind.check(&world, rule)?;
                if kind == EngineKind::Hashlife && !generations_per_tick.is_power_of_two() {
                    return Err(CaError::invalid(format!(
                        "Hashlife advances in powers of two; {generations_per_tick} generations per tick is not one \
                         (use 1, 2, 4, 8, ...)"
                    )));
                }
                Ok(kind)
            }
            None => select_engine(&SelectionInput { world, rule, population, generations_per_tick, regularity }),
        }
    }

    /// Re-runs the automatic engine choice (no-op for a fixed choice). The
    /// tile regularity is measured only up to
    /// [`AUTO_REGULARITY_MAX_POPULATION`] live cells: a larger pattern keeps
    /// Hashlife if it runs, and is treated as irregular otherwise. A pattern
    /// of more than [`MAX_SWITCH_CELLS`] live cells keeps the running engine
    /// (a switch would copy every cell), which can run any tick.
    fn reselect(&mut self) -> Result<(), CaError> {
        if self.engine_choice != EngineChoice::Auto {
            return Ok(());
        }
        let population = self.engine.population();
        if population > MAX_SWITCH_CELLS
            || (population > AUTO_REGULARITY_MAX_POPULATION && self.engine.kind() == EngineKind::Hashlife)
        {
            return Ok(());
        }
        let measured = AUTO_HASHLIFE_MIN_POPULATION..=AUTO_REGULARITY_MAX_POPULATION;
        let regularity = (!self.world.is_finite() && measured.contains(&population))
            .then(|| tile_regularity(&self.engine.export_cells()));
        let kind =
            Self::choose(EngineChoice::Auto, self.world, self.rule, population, self.generations_per_tick, regularity)?;
        self.set_engine_kind(kind)
    }

    /// Switches to `kind`, transferring the exact state.
    fn set_engine_kind(&mut self, kind: EngineKind) -> Result<(), CaError> {
        if self.engine.kind() == kind {
            return Ok(());
        }
        self.engine = transfer(self.engine.as_ref(), kind)?;
        Ok(())
    }

    /// Checks that [`LifeSim::rebuild`] can build the scenario's initial
    /// content on `world` with `choice`, `rule` and `generations` per tick:
    /// the engine accepts the world and rule, random fills are valid, and
    /// the dense engine's window on the infinite plane can hold everything.
    fn check_rebuild(
        &self,
        world: World,
        choice: EngineChoice,
        rule: LifeRule,
        generations: u64,
    ) -> Result<(), CaError> {
        let kind = Self::choose(choice, world, rule, 0, generations, None)?;
        let mut bounds: Option<CellRect> = None;
        for item in &self.init {
            let rect = match item {
                ResolvedInit::Cells(cells) => CellRect::bounding(cells.iter().filter_map(|&(x, y)| world.map(x, y))),
                ResolvedInit::Random { region, density, symmetry } => {
                    let region = region.map_or(Some(world.home()), |r| world.clip(&r));
                    if let Some(r) = region {
                        check_fill(r, *density, *symmetry, MAX_FILL_CELLS)?;
                    }
                    region
                }
            };
            bounds = match (bounds, rect) {
                (Some(a), Some(b)) => Some(a.union(b)),
                (a, b) => a.or(b),
            };
        }
        if kind == EngineKind::Dense
            && !world.is_finite()
            && let Some(rect) = bounds
        {
            DenseEngine::window_fits(rect)?;
        }
        Ok(())
    }

    /// Builds the initial state of the scenario on `next_world` from the
    /// seed. Nothing changes on error.
    fn rebuild(&mut self) -> Result<(), CaError> {
        let world = self.next_world;
        let kind = Self::choose(self.engine_choice, world, self.rule, 0, self.generations_per_tick, None)?;
        let mut engine = create_engine(kind, world, self.rule)?;
        let mut rng = SimRng::seed_from_u64(self.seed);
        for item in &self.init {
            match item {
                ResolvedInit::Cells(cells) => {
                    if let Some(bbox) = CellRect::bounding(cells.iter().filter_map(|&(x, y)| world.map(x, y))) {
                        engine.reserve(bbox)?;
                    }
                    engine.set_cells(cells, true);
                }
                ResolvedInit::Random { region, density, symmetry } => {
                    if let Some(region) = region.map_or(Some(world.home()), |r| world.clip(&r)) {
                        commands::fill_random(engine.as_mut(), &mut rng, region, *density, *symmetry)?;
                    }
                }
            }
        }
        self.world = world;
        self.engine = engine;
        self.rng = rng;
        self.generation = 0;
        self.last_stats = None;
        self.halted = None;
        self.reset_point = None;
        self.reselect()?;
        let live = if self.tracker.enabled() { self.engine.export_cells() } else { Vec::new() };
        self.tracker.restart(live);
        Ok(())
    }

    /// Applies NextTick parameter changes.
    fn apply_pending(&mut self) -> Result<(), CaError> {
        if let Some((rule, preset)) = self.pending_rule.take() {
            self.engine.set_rule(rule)?;
            self.rule = rule;
            self.preset = preset;
        }
        if let Some(n) = self.pending_generations.take() {
            self.generations_per_tick = n;
            self.reselect()?;
        }
        Ok(())
    }

    /// Runs `ticks` ticks; returns the generations advanced.
    fn run(&mut self, ticks: u64) -> u64 {
        if self.halted.is_some() || ticks == 0 {
            return 0;
        }
        if let Err(e) = self.apply_pending() {
            self.halted = Some(e.to_string());
            return 0;
        }
        if self.generation == 0 && self.reset_point.is_none() {
            self.reset_point = Some(ResetPoint::capture(self));
        }
        let Some(total) = ticks.checked_mul(self.generations_per_tick) else {
            self.halted = Some("the requested number of generations overflows a 64-bit counter".into());
            return 0;
        };
        if self.generation.checked_add(total).is_none() {
            self.halted = Some("the generation counter would exceed 2^64 - 1".into());
            return 0;
        }
        let start = self.generation;
        let result = if !self.tracker.enabled() && self.engine.kind() == EngineKind::Hashlife {
            self.jump(total).map(|()| None)
        } else {
            let mut last = Ok(None);
            for _ in 0..ticks {
                last = self.step_tick(self.generations_per_tick);
                if last.is_err() {
                    break;
                }
            }
            last
        };
        match result {
            Ok(stats) => self.last_stats = stats,
            Err(e) => {
                self.last_stats = None;
                self.halted = Some(e.to_string());
            }
        }
        self.generation - start
    }

    /// Advances `total` generations with Hashlife jumps: one memoised
    /// power-of-two step per set bit of `total` (each at most 2^61, the
    /// largest single step). The generation counter follows every completed
    /// jump, so it stays exact when a later jump fails.
    fn jump(&mut self, total: u64) -> Result<(), CaError> {
        let mut remaining = total;
        while remaining != 0 {
            let chunk = 1u64 << remaining.trailing_zeros().min(hashlife::MAX_STEP_EXPONENT);
            self.engine.advance(chunk)?;
            self.generation += chunk;
            remaining -= chunk;
        }
        Ok(())
    }

    /// One tick of `n` generations stepped one by one (exact births, deaths
    /// and tracking). Returns the births and deaths summed over the tick
    /// (`None` when the engine does not count them and tracking is off).
    fn step_tick(&mut self, n: u64) -> Result<Option<GenStats>, CaError> {
        let mut total = Some(GenStats::default());
        for i in 0..n {
            let stats = self.engine.step()?;
            self.generation += 1;
            let stats = if self.tracker.enabled() {
                Some(self.tracker.generation(self.engine.export_cells(), i == 0))
            } else {
                stats
            };
            total = total.zip(stats).map(|(a, b)| a + b);
        }
        Ok(total)
    }

    /// Bookkeeping after an edit of `rect` (`None` = anywhere). The tracker
    /// merges the live cells of the edited region, listed only when it holds
    /// at most [`MAX_EDIT_CELLS`] of them (counted first): above that limit
    /// tracking stops (the `track_*` parameters turn off) instead of listing
    /// them.
    fn after_edit(&mut self, rect: Option<CellRect>) {
        if self.generation == 0 {
            self.reset_point = None;
        }
        if self.tracker.enabled() {
            let region = match rect {
                Some(r) => self.world.clip(&r),
                None => Some(CellRect::new(i64::MIN, i64::MIN, i64::MAX, i64::MAX)),
            };
            if let Some(r) = region {
                if self.engine.count_in_rect(r, MAX_EDIT_CELLS + 1) > MAX_EDIT_CELLS {
                    self.tracker.configure(false, false, Vec::new);
                } else {
                    self.tracker.edit(r, self.engine.cells_in_rect(r));
                }
            }
        }
        self.halted = None;
    }

    fn rules_text(&self) -> String {
        let digits =
            |mask: u16| (0..=8).filter(|n| mask >> n & 1 == 1).map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
        let topology = match self.world.topology {
            Topology::Bounded => "a bounded plane: cells outside the W×H rectangle are permanently dead",
            Topology::Torus => "a torus: opposite edges are joined",
            Topology::KleinBottle => {
                "a Klein bottle: left and right edges are joined; crossing the top or bottom edge mirrors x (x becomes \
                 W−1−x)"
            }
            Topology::CrossSurface => {
                "a cross-surface (projective plane): crossing the top or bottom edge mirrors x, crossing the left or \
                 right edge mirrors y"
            }
            Topology::Infinite => "the infinite plane with 64-bit coordinates (unbounded in every direction)",
        };
        format!(
            "## {rule} — {label}\n\n\
             Cells are dead (0) or alive (1) on a square grid. Every cell has the 8 cells of its Moore \
             neighbourhood as neighbours, and all cells update simultaneously once per generation. With *n* live \
             neighbours:\n\n\
             * a dead cell becomes alive iff *n* is in B = {{{b}}};\n\
             * a live cell stays alive iff *n* is in S = {{{s}}};\n\
             * every other cell is dead in the next generation.\n\n\
             **World.** The world is {topology}. On finite worlds a neighbour position outside the world is the \
             cell the topology maps it to (Golly's ghost-cell emulation; on worlds narrower than 3 cells the same \
             cell can be counted twice).\n\n\
             **B0 rules** (birth with no live neighbour) are run literally on finite worlds: every cell of the world \
             follows the rule, and on the bounded plane the outside stays dead. They are rejected on the infinite \
             plane, where they would fill all of space.\n\n\
             **Ticks.** One tick advances {g} generation(s). Births and deaths are counted per generation and summed \
             over the tick (not available when Hashlife jumps without tracking).\n\n\
             **Engines.** Dense (byte per cell), Bit (64 cells per word), Sparse (64×64 tiles) and Hashlife compute \
             identical generations; switching engines keeps every cell.\n\n\
             **Tracking.** Age = generations a cell has been alive (its birth generation counts as 1, saturating at \
             255). Activity = 255 when a cell was born or died during the last tick (in any of its generations), \
             minus {decay} per tick afterwards. Both are updated every generation, never per frame.",
            rule = self.rule,
            label = self.preset.label(),
            b = digits(self.rule.birth),
            s = digits(self.rule.survive),
            g = self.generations_per_tick,
            decay = TRAIL_DECAY,
        )
    }
}

impl Simulation for LifeSim {
    type Command = LifeCommand;
    type ViewRequest = GridViewRequest;
    type View = GridView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(self.mode.model_id(), self.mode.name(), ModelFamily::Cellular)
            .summary(
                "Conway's Game of Life and any Life-like B/S rule on bounded, toroidal, Klein-bottle, \
                 cross-surface or infinite worlds, with dense, bit-packed, sparse and Hashlife engines.",
            )
            .rules(&self.rules_text())
            .params(params::specs())
            .metrics(vec![
                MetricSpec::gauge("population", "Population", "cells", "Number of live cells."),
                MetricSpec::counter("births", "Births", "Cells born during the last tick, summed over its generations."),
                MetricSpec::counter("deaths", "Deaths", "Cells that died during the last tick, summed over its generations."),
                MetricSpec::ratio(
                    "density",
                    "Density",
                    "Population divided by the world area (finite worlds) or by the bounding-box area (infinite plane).",
                ),
                MetricSpec::gauge("bbox_width", "Bounding box width", "cells", "Width of the smallest rectangle holding every live cell."),
                MetricSpec::gauge("bbox_height", "Bounding box height", "cells", "Height of that rectangle."),
                MetricSpec::gauge("generation", "Generation", "generations", "Generations since the last reset."),
                MetricSpec::gauge("engine", "Engine", "", "Running engine: 0 = dense, 1 = bit-packed, 2 = sparse, 3 = Hashlife."),
            ])
            .time("generation", 1.0)
    }

    fn tick(&self) -> u64 {
        self.generation
    }

    fn step(&mut self) {
        self.run(1);
    }

    fn advance(&mut self, max_ticks: u64) -> u64 {
        self.run(max_ticks)
    }

    fn reset(&mut self) {
        let outcome = self.apply_pending().and_then(|()| match self.reset_point.clone() {
            Some(point) if self.next_world == self.world => point.restore(self),
            _ => self.rebuild(),
        });
        if let Err(e) = outcome {
            self.halted = Some(e.to_string());
        }
    }

    fn apply(&mut self, command: LifeCommand) -> Result<(), SimError> {
        let rect = self.execute(command)?;
        self.after_edit(rect);
        Ok(())
    }

    fn params(&self) -> ParamSet {
        params::values(self)
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        params::set(self, key, value)
    }

    fn metrics(&self) -> Metrics {
        let population = self.engine.population();
        let bbox = self.engine.bounding_box();
        let (bw, bh) = bbox.map_or((0, 0), |b| (b.width(), b.height()));
        let area =
            if self.world.is_finite() { self.world.cell_count() as f64 } else { bbox.map_or(0.0, |b| b.area() as f64) };
        let mut m = Metrics::new(self.generation, self.generation as f64)
            .with("population", population as f64)
            .with("density", if area > 0.0 { population as f64 / area } else { 0.0 })
            .with("bbox_width", bw as f64)
            .with("bbox_height", bh as f64)
            .with("generation", self.generation as f64)
            .with("engine", f64::from(self.engine.kind().code()));
        if let Some(stats) = self.last_stats {
            m.push("births", stats.births as f64).push("deaths", stats.deaths as f64);
        }
        m
    }

    fn view(&self, request: &GridViewRequest) -> GridView {
        let grid = TexelGrid::plan(request, self.world.bounds());
        let mut counts = vec![0u32; grid.len()];
        self.engine.count_live(&grid, &mut counts);
        let state: Vec<u8> = counts.iter().map(|&c| u8::from(c > 0)).collect();
        let (age, activity) = self.tracker.texels(&grid);
        grid.assemble(&counts, &state, &age, &activity)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        snapshot::save(self)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        snapshot::load(self, snapshot)
    }

    fn memory_bytes(&self) -> usize {
        self.engine.memory_bytes() + self.tracker.memory_bytes()
    }

    fn is_finished(&self) -> bool {
        self.halted.is_some()
    }
}
