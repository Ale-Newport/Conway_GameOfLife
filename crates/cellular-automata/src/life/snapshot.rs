//! Snapshots and reset points of [`LifeSim`].
//!
//! Cells are stored compactly: finite worlds as packed bit rows, Hashlife as
//! a macrocell file (which shares repeated structure), other engines as a
//! sorted cell list. The payload (schema version 1) holds the complete
//! state, including the random generator, pending parameter changes, the
//! tracker and the reset point, so a restored run replays identically.

use patterns::LifeRule;
use serde::{Deserialize, Serialize};
use simulation_core::{SimError, SimRng, Snapshot};

use super::config::ResolvedInit;
use super::tracking::Tracker;
use super::{LifeMode, LifeSim, RulePreset, SCHEMA_VERSION};
use crate::engine::{EngineChoice, EngineKind, GenStats, HashlifeEngine, LifeEngine, create_engine};
use crate::error::CaError;
use crate::geometry::{CellRect, World};

/// Live cells of an engine in a compact form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CellData {
    /// Finite world: `ceil(width / 64)` words per row, row-major.
    Packed { width: u32, height: u32, words: Vec<u64> },
    /// Sorted live cells.
    List(Vec<(i64, i64)>),
    /// Golly macrocell text (Hashlife).
    Macrocell(String),
}

impl CellData {
    pub(crate) fn capture(engine: &dyn LifeEngine) -> Self {
        if let Some(h) = engine.as_any().downcast_ref::<HashlifeEngine>() {
            return Self::Macrocell(hashlife::macrocell::write(h.universe()));
        }
        let world = engine.world();
        if world.is_finite() {
            let n = (world.width as usize).div_ceil(64);
            let mut words = vec![0u64; n * world.height as usize];
            for (x, y) in engine.export_cells() {
                words[y as usize * n + x as usize / 64] |= 1 << (x % 64);
            }
            Self::Packed { width: world.width, height: world.height, words }
        } else {
            Self::List(engine.export_cells())
        }
    }

    /// Checks that the data fits `world` (packed rows of exactly its size;
    /// lists and macrocells only on the infinite plane).
    pub(crate) fn check(&self, world: &World) -> Result<(), CaError> {
        let fits = match self {
            Self::Packed { width, height, words } => {
                (world.width, world.height) == (*width, *height)
                    && world.is_finite()
                    && words.len() as u64 == u64::from(width.div_ceil(64)) * u64::from(*height)
            }
            Self::List(_) | Self::Macrocell(_) => !world.is_finite(),
        };
        if fits { Ok(()) } else { Err(CaError::invalid("snapshot cell data does not match the world")) }
    }

    /// Loads the cells into `engine` (an empty engine of the saved world).
    ///
    /// # Errors
    /// Size mismatches, unreadable macrocell text, or an engine that cannot
    /// hold the cells (the dense engine's window on the infinite plane).
    pub(crate) fn apply(&self, engine: &mut dyn LifeEngine) -> Result<(), CaError> {
        match self {
            Self::Packed { width, height, words } => {
                let world = engine.world();
                let n = (*width as usize).div_ceil(64);
                if (world.width, world.height) != (*width, *height) || words.len() != n * *height as usize {
                    return Err(CaError::invalid("snapshot cell data does not match the world size"));
                }
                let mut cells = Vec::new();
                for (i, &w) in words.iter().enumerate() {
                    let (y, k) = ((i / n) as i64, (i % n) as i64);
                    let mut bits = w;
                    while bits != 0 {
                        cells.push((64 * k + i64::from(bits.trailing_zeros()), y));
                        bits &= bits - 1;
                    }
                }
                engine.load_cells(&cells);
            }
            Self::List(cells) => {
                if let Some(bbox) = CellRect::bounding(cells.iter().copied()) {
                    engine.reserve(bbox)?;
                }
                engine.load_cells(cells);
            }
            Self::Macrocell(text) => {
                let universe = hashlife::macrocell::read(text)?;
                match engine.as_any_mut().downcast_mut::<HashlifeEngine>() {
                    Some(h) => {
                        // The saved rule wins over the rule written in the macrocell text.
                        let rule = h.rule();
                        h.set_universe(universe)?;
                        h.set_rule(rule)?;
                    }
                    None => {
                        let cells: Vec<(i64, i64)> = universe.cells().collect();
                        if let Some(bbox) = CellRect::bounding(cells.iter().copied()) {
                            engine.reserve(bbox)?;
                        }
                        engine.load_cells(&cells);
                    }
                }
            }
        }
        Ok(())
    }
}

/// The state [`simulation_core::Simulation::reset`] returns to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ResetPoint {
    cells: CellData,
    rng: SimRng,
}

impl ResetPoint {
    pub(crate) fn capture(sim: &LifeSim) -> Self {
        Self { cells: CellData::capture(sim.engine.as_ref()), rng: sim.rng.clone() }
    }

    /// Returns `sim` to this point (into a fresh engine of the running kind,
    /// so nothing changes on error).
    pub(crate) fn restore(&self, sim: &mut LifeSim) -> Result<(), CaError> {
        self.cells.check(&sim.world)?;
        let mut engine = create_engine(sim.engine.kind(), sim.world, sim.rule)?;
        self.cells.apply(engine.as_mut())?;
        sim.engine = engine;
        sim.rng = self.rng.clone();
        sim.generation = 0;
        sim.last_stats = None;
        sim.halted = None;
        sim.restart_tracking();
        Ok(())
    }
}

/// Snapshot payload, schema version 1.
#[derive(Serialize, Deserialize)]
struct SavedState {
    mode: LifeMode,
    seed: u64,
    world: World,
    next_world: World,
    init: Vec<ResolvedInit>,
    engine_kind: EngineKind,
    engine_choice: EngineChoice,
    rule: (u16, u16),
    preset: RulePreset,
    pending_rule: Option<((u16, u16), RulePreset)>,
    generations_per_tick: u64,
    pending_generations: Option<u64>,
    generation: u64,
    rng: SimRng,
    tracker: Tracker,
    last_stats: Option<GenStats>,
    reset_point: Option<ResetPoint>,
    halted: Option<String>,
    cells: CellData,
}

pub(super) fn save(sim: &LifeSim) -> Result<Snapshot, SimError> {
    let state = SavedState {
        mode: sim.mode,
        seed: sim.seed,
        world: sim.world,
        next_world: sim.next_world,
        init: sim.init.clone(),
        engine_kind: sim.engine.kind(),
        engine_choice: sim.engine_choice,
        rule: (sim.rule.birth, sim.rule.survive),
        preset: sim.preset,
        pending_rule: sim.pending_rule.map(|(r, p)| ((r.birth, r.survive), p)),
        generations_per_tick: sim.generations_per_tick,
        pending_generations: sim.pending_generations,
        generation: sim.generation,
        rng: sim.rng.clone(),
        tracker: sim.tracker.clone(),
        last_stats: sim.last_stats,
        reset_point: sim.reset_point.clone(),
        halted: sim.halted.clone(),
        cells: CellData::capture(sim.engine.as_ref()),
    };
    Snapshot::encode(sim.mode.model_id(), SCHEMA_VERSION, sim.generation, &state)
}

pub(super) fn load(sim: &mut LifeSim, snapshot: &Snapshot) -> Result<(), SimError> {
    let state: SavedState = snapshot.decode(sim.mode.model_id(), SCHEMA_VERSION)?;
    let rule = |(b, s): (u16, u16)| LifeRule::new(b, s).map_err(|e| SimError::Serialization(e.to_string()));
    let world = World::new(state.world.topology, state.world.width, state.world.height)?;
    let next_world = World::new(state.next_world.topology, state.next_world.width, state.next_world.height)?;
    let current = rule(state.rule)?;
    let pending = match state.pending_rule {
        Some((masks, preset)) => Some((rule(masks)?, preset)),
        None => None,
    };
    state.tracker.validate()?;
    // Sizes are checked before the engine is allocated.
    state.cells.check(&world)?;
    let mut engine = create_engine(state.engine_kind, world, current)?;
    state.cells.apply(engine.as_mut())?;
    // Everything validated: commit.
    sim.mode = state.mode;
    sim.seed = state.seed;
    sim.world = world;
    sim.next_world = next_world;
    sim.init = state.init;
    sim.engine = engine;
    sim.engine_choice = state.engine_choice;
    sim.rule = current;
    sim.preset = state.preset;
    sim.pending_rule = pending;
    sim.generations_per_tick = state.generations_per_tick;
    sim.pending_generations = state.pending_generations;
    sim.generation = state.generation;
    sim.rng = state.rng;
    sim.tracker = state.tracker;
    sim.tracking_off = None;
    sim.last_stats = state.last_stats;
    sim.reset_point = state.reset_point;
    sim.halted = state.halted;
    Ok(())
}
