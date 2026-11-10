//! State shared by the multi-state models ([`crate::GenerationsSim`],
//! [`crate::WireworldSim`]): world, initial content, generation counter,
//! random generator, reset point, painting and snapshots.

use serde::{Deserialize, Serialize};
use simulation_core::{ApplyPolicy, ParamSpec, ParamValue, SimError, SimRng, Topology};

use crate::engine::Lut;
use crate::error::CaError;
use crate::geometry::{MAX_FILL_CELLS, MAX_WORLD_SIDE, World, check_fill};
use crate::life::{PatternLibrary, place_pattern};
use crate::multistate::{PaintCommand, ResolvedState, StateGrid, StateInit, check_states, resolve_init};

/// Topology choices of the multi-state models (finite worlds only).
pub(crate) const FINITE_TOPOLOGIES: [(&str, &str); 4] = [
    ("bounded", "Bounded plane (dead outside)"),
    ("torus", "Torus (wrap-around)"),
    ("klein_bottle", "Klein bottle"),
    ("cross_surface", "Cross-surface (projective plane)"),
];

/// Counts of the last generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LastCounts {
    pub entered_one: u64,
    pub left_one: u64,
}

/// The common state of a multi-state model.
#[derive(Clone, Debug)]
pub(crate) struct StateCore {
    pub seed: u64,
    pub next_world: World,
    pub init: Vec<ResolvedState>,
    pub grid: StateGrid,
    pub generation: u64,
    pub rng: SimRng,
    pub reset_point: Option<(Vec<u8>, SimRng)>,
    pub last: Option<LastCounts>,
    /// Why the model stopped (a reset that could not be carried out); the
    /// state is unchanged then, and the next successful reset or edit
    /// clears it.
    pub halted: Option<String>,
}

/// States accepted when a reset point is reloaded: a reset point may hold
/// states of a rule replaced since (they decay to 0 in one generation, as
/// after any rule change), so only the size is checked.
const ANY_STATE: u16 = 256;

/// Serialisable form of [`StateCore`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SavedCore {
    seed: u64,
    world: World,
    next_world: World,
    init: Vec<ResolvedState>,
    cells: Vec<u8>,
    activity: Option<Vec<u8>>,
    generation: u64,
    rng: SimRng,
    reset_point: Option<(Vec<u8>, SimRng)>,
    last: Option<LastCounts>,
    halted: Option<String>,
}

impl StateCore {
    /// Builds the core and its initial content.
    pub(crate) fn new(
        world: World,
        init: &[StateInit],
        library: &PatternLibrary,
        seed: u64,
        states: u16,
        track_activity: bool,
    ) -> Result<Self, CaError> {
        let init = resolve_init(init, library, states)?;
        let mut grid = StateGrid::new(world)?;
        grid.set_tracking(track_activity);
        let mut core = Self {
            seed,
            next_world: world,
            init,
            grid,
            generation: 0,
            rng: SimRng::seed_from_u64(seed),
            reset_point: None,
            last: None,
            halted: None,
        };
        core.rebuild()?;
        Ok(core)
    }

    /// Checks that the initial content can be rebuilt on `world` (random
    /// fills: density, symmetry and region size).
    pub(crate) fn check_init(&self, world: &World) -> Result<(), CaError> {
        for item in &self.init {
            if let ResolvedState::Random { region, density, symmetry, .. } = item {
                let region = region.map_or(Some(world.home()), |r| world.clip(&r));
                if let Some(region) = region {
                    check_fill(region, *density, *symmetry, MAX_FILL_CELLS)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn world(&self) -> World {
        self.grid.world()
    }

    /// Re-creates the initial content on `next_world` from the seed.
    pub(crate) fn rebuild(&mut self) -> Result<(), CaError> {
        let mut grid = StateGrid::new(self.next_world)?;
        grid.set_tracking(self.grid.tracking());
        let mut rng = SimRng::seed_from_u64(self.seed);
        for item in &self.init {
            match item {
                ResolvedState::Cells(cells) => {
                    for &(x, y, s) in cells {
                        grid.set(x, y, s);
                    }
                }
                ResolvedState::Random { region, density, state, symmetry } => {
                    grid.random(&mut rng, *region, *density, *state, *symmetry)?;
                }
            }
        }
        // A fresh start shows no trail.
        if grid.tracking() {
            grid.set_tracking(false);
            grid.set_tracking(true);
        }
        self.grid = grid;
        self.rng = rng;
        self.generation = 0;
        self.reset_point = None;
        self.last = None;
        self.halted = None;
        Ok(())
    }

    /// One generation (nothing while halted).
    pub(crate) fn step(&mut self, lut: &Lut) {
        if self.halted.is_some() {
            return;
        }
        if self.generation == 0 && self.reset_point.is_none() {
            self.reset_point = Some((self.grid.raw(), self.rng.clone()));
        }
        let counts = self.grid.step(lut);
        self.generation += 1;
        self.last = Some(LastCounts { entered_one: counts.entered_one, left_one: counts.left_one });
    }

    /// Returns to generation 0 (see `LifeSim`'s reset semantics): the reset
    /// point captured at the first step, or the scenario rebuilt from the
    /// seed after a world change. A failure (not expected: world changes are
    /// validated by `set_world_param`) leaves the state unchanged and is
    /// reported through [`StateCore::halted`].
    pub(crate) fn reset(&mut self) {
        let outcome = match self.reset_point.clone() {
            Some((raw, rng)) if self.next_world == self.world() => {
                let tracking = self.grid.tracking();
                StateGrid::new(self.world()).and_then(|mut grid| {
                    grid.load_raw(&raw, ANY_STATE)?;
                    grid.set_tracking(tracking);
                    self.grid = grid;
                    self.rng = rng;
                    self.generation = 0;
                    self.last = None;
                    self.halted = None;
                    Ok(())
                })
            }
            _ => self.rebuild(),
        };
        if let Err(e) = outcome {
            self.halted = Some(format!("reset failed: {e}"));
        }
    }

    /// Applies a painting command.
    pub(crate) fn apply(&mut self, command: PaintCommand, states: u16) -> Result<(), CaError> {
        match command {
            PaintCommand::LoadPattern { pattern, x, y } => self.load(&pattern, x, y, states),
            PaintCommand::ImportText { text, file_name, x, y, replace: true } => {
                let pattern = patterns::parse_any(&text, file_name.as_deref())?;
                self.load(&pattern, x, y, states)
            }
            other => {
                self.grid.paint(other, states, &mut self.rng)?;
                if self.generation == 0 {
                    self.reset_point = None;
                }
                self.halted = None;
                Ok(())
            }
        }
    }

    fn load(&mut self, pattern: &patterns::Pattern, x: i64, y: i64, states: u16) -> Result<(), CaError> {
        let cells = place_pattern(pattern, x, y, crate::geometry::Rotation::None, false)?;
        check_states(cells.iter().map(|c| c.2), states)?;
        self.grid.clear();
        for &(cx, cy, s) in &cells {
            self.grid.set(cx, cy, s);
        }
        self.init = vec![ResolvedState::Cells(cells)];
        self.generation = 0;
        self.reset_point = None;
        self.last = None;
        self.halted = None;
        Ok(())
    }

    /// Specs of the world and tracking parameters.
    pub(crate) fn world_specs(default_topology: &str, default_side: i64) -> Vec<ParamSpec> {
        vec![
            ParamSpec::choice("topology", "Topology", &FINITE_TOPOLOGIES, default_topology)
                .group("World")
                .apply(ApplyPolicy::RequiresReset)
                .describe("Edge behaviour of the finite world."),
            ParamSpec::int("width", "Width", 1, i64::from(MAX_WORLD_SIDE), default_side)
                .group("World")
                .apply(ApplyPolicy::RequiresReset)
                .describe("World width in cells."),
            ParamSpec::int("height", "Height", 1, i64::from(MAX_WORLD_SIDE), default_side)
                .group("World")
                .apply(ApplyPolicy::RequiresReset)
                .describe("World height in cells."),
            ParamSpec::boolean("track_activity", "Track activity", false)
                .group("Display")
                .apply(ApplyPolicy::Immediate)
                .describe("Show cells that changed recently (view channel B), fading over 16 generations."),
        ]
    }

    /// Current values of the world and tracking parameters.
    pub(crate) fn world_values(&self) -> [(&'static str, ParamValue); 4] {
        [
            ("topology", ParamValue::Text(crate::life::topology_name(self.next_world.topology).into())),
            ("width", ParamValue::Int(i64::from(self.next_world.width))),
            ("height", ParamValue::Int(i64::from(self.next_world.height))),
            ("track_activity", ParamValue::Bool(self.grid.tracking())),
        ]
    }

    /// Applies a validated world or tracking parameter; `None` if `key` is not one.
    pub(crate) fn set_world_param(&mut self, key: &str, value: &ParamValue) -> Option<Result<(), SimError>> {
        let mut next = self.next_world;
        match key {
            "topology" => {
                let topology = value.as_str().and_then(crate::life::parse_topology);
                match topology {
                    Some(t) if t != Topology::Infinite => next.topology = t,
                    _ => return Some(Err(SimError::invalid_param(key, "choose a finite topology"))),
                }
            }
            "width" => next.width = value.as_i64().unwrap_or(1) as u32,
            "height" => next.height = value.as_i64().unwrap_or(1) as u32,
            "track_activity" => {
                self.grid.set_tracking(value.as_bool().unwrap_or(false));
                return Some(Ok(()));
            }
            _ => return None,
        }
        // The world must be valid and the scenario must be rebuildable on it.
        let checked = World::new(next.topology, next.width, next.height)
            .and_then(|w| StateGrid::check_world(&w).map(|()| w))
            .and_then(|w| self.check_init(&w).map(|()| w))
            .map_err(|e| SimError::invalid_param(key, e.to_string()));
        Some(checked.map(|w| self.next_world = w))
    }

    pub(crate) fn save(&self) -> SavedCore {
        SavedCore {
            seed: self.seed,
            world: self.world(),
            next_world: self.next_world,
            init: self.init.clone(),
            cells: self.grid.raw(),
            activity: self.grid.activity_raw(),
            generation: self.generation,
            rng: self.rng.clone(),
            reset_point: self.reset_point.clone(),
            last: self.last,
            halted: self.halted.clone(),
        }
    }

    pub(crate) fn load_saved(saved: SavedCore, states: u16) -> Result<Self, CaError> {
        let world = World::new(saved.world.topology, saved.world.width, saved.world.height)?;
        let next_world = World::new(saved.next_world.topology, saved.next_world.width, saved.next_world.height)?;
        // Sizes are checked before the grid is allocated.
        StateGrid::check_world(&world)?;
        if saved.cells.len() as u64 != world.cell_count() {
            return Err(CaError::invalid("snapshot cell data does not match the world size"));
        }
        let mut grid = StateGrid::new(world)?;
        grid.load_raw(&saved.cells, states)?;
        grid.set_activity_raw(saved.activity)?;
        Ok(Self {
            seed: saved.seed,
            next_world,
            init: saved.init,
            grid,
            generation: saved.generation,
            rng: saved.rng,
            reset_point: saved.reset_point,
            last: saved.last,
            halted: saved.halted,
        })
    }
}
