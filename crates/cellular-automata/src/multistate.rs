//! Byte-per-cell multi-state worlds shared by [`crate::GenerationsSim`] and
//! [`crate::WireworldSim`]: a finite grid with a ghost ring, a transition
//! table counting state-1 neighbours, per-cell activity trails, painting
//! commands and the grid view.

use patterns::Pattern;
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use crate::engine::{Lut, Padded, StepCounts};
use crate::error::CaError;
use crate::geometry::{
    CellRect, MAX_FILL_CELLS, Rotation, Symmetry, World, check_fill, circle_outline, disc, line, random_fill_each,
    rectangle, stroke,
};
use crate::life::{PatternLibrary, PatternSource, place_pattern};
use crate::view::{GridView, RowAccumulator, TexelGrid};

/// Largest multi-state world (cells).
pub const MAX_MULTISTATE_CELLS: u64 = 1 << 28;
/// Activity lost per generation by the trail of a multi-state world.
pub const STATE_TRAIL_DECAY: u8 = 16;

/// Painting commands shared by the multi-state models. `state` values must
/// be valid for the model's rule; painting state 0 erases. Single cells map
/// through the topology; rectangles are clipped to the world.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintCommand {
    /// Sets one cell.
    SetCell {
        /// Column.
        x: i64,
        /// Row.
        y: i64,
        /// New state.
        state: u8,
    },
    /// Brush stroke of discs of `radius` along consecutive points.
    Stroke {
        /// Stroke points.
        points: Vec<(i64, i64)>,
        /// Brush radius.
        radius: u32,
        /// State to paint.
        state: u8,
    },
    /// One-cell-wide line.
    Line {
        /// Start cell.
        from: (i64, i64),
        /// End cell (inclusive).
        to: (i64, i64),
        /// State to paint.
        state: u8,
    },
    /// Rectangle, filled or outline.
    Rectangle {
        /// The rectangle.
        rect: CellRect,
        /// Filled or outline.
        filled: bool,
        /// State to paint.
        state: u8,
    },
    /// Circle, filled or outline.
    Circle {
        /// Centre cell.
        centre: (i64, i64),
        /// Radius.
        radius: u32,
        /// Filled or outline.
        filled: bool,
        /// State to paint.
        state: u8,
    },
    /// Pastes a pattern keeping its states (non-zero cells overwrite).
    Paste {
        /// The pattern.
        pattern: Pattern,
        /// Horizontal offset.
        x: i64,
        /// Vertical offset.
        y: i64,
        /// Clockwise rotation.
        rotation: Rotation,
        /// Left-right mirror (before the rotation).
        mirror: bool,
    },
    /// Replaces everything by a pattern; generation 0, new reset point.
    LoadPattern {
        /// The pattern.
        pattern: Pattern,
        /// Horizontal offset.
        x: i64,
        /// Vertical offset.
        y: i64,
    },
    /// Parses pattern text (RLE with state letters, plaintext...) and loads
    /// (`replace`) or pastes it.
    ImportText {
        /// File content.
        text: String,
        /// File name for format detection.
        file_name: Option<String>,
        /// Horizontal offset.
        x: i64,
        /// Vertical offset.
        y: i64,
        /// Load (replace) or paste.
        replace: bool,
    },
    /// Sets every cell to 0.
    Clear,
    /// Sets the cells of a rectangle to 0.
    ClearRegion {
        /// The rectangle.
        rect: CellRect,
    },
    /// Fills a region with `state` at random (other cells of the region become 0).
    Randomize {
        /// Probability of `state`.
        density: f64,
        /// State to scatter.
        state: u8,
        /// Dedicated seed, or `None` for the model's generator.
        seed: Option<u64>,
        /// Region (default: the whole world).
        region: Option<CellRect>,
        /// Symmetry.
        symmetry: Symmetry,
    },
}

/// One step of building a multi-state initial state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StateInit {
    /// Places a pattern keeping its states (see [`crate::life::InitItem::Pattern`]).
    Pattern {
        /// The pattern.
        source: PatternSource,
        /// Horizontal offset.
        #[serde(default)]
        x: i64,
        /// Vertical offset.
        #[serde(default)]
        y: i64,
        /// Clockwise rotation.
        #[serde(default)]
        rotation: Rotation,
        /// Left-right mirror.
        #[serde(default)]
        mirror: bool,
    },
    /// Random cells of one state.
    Random {
        /// Region (default: the whole world).
        #[serde(default)]
        region: Option<CellRect>,
        /// Probability.
        density: f64,
        /// State scattered.
        #[serde(default = "one")]
        state: u8,
        /// Symmetry.
        #[serde(default)]
        symmetry: Symmetry,
    },
}

fn one() -> u8 {
    1
}

/// A resolved initial-state step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum ResolvedState {
    Cells(Vec<(i64, i64, u8)>),
    Random { region: Option<CellRect>, density: f64, state: u8, symmetry: Symmetry },
}

/// Resolves initial-state steps, checking states against `states`.
pub(crate) fn resolve_init(
    init: &[StateInit],
    library: &PatternLibrary,
    states: u16,
) -> Result<Vec<ResolvedState>, CaError> {
    init.iter()
        .map(|item| match item {
            StateInit::Pattern { source, x, y, rotation, mirror } => {
                let pattern = library.load_source(source)?;
                let cells = place_pattern(&pattern, *x, *y, *rotation, *mirror)?;
                check_states(cells.iter().map(|c| c.2), states)?;
                Ok(ResolvedState::Cells(cells))
            }
            StateInit::Random { region, density, state, symmetry } => {
                check_states([*state], states)?;
                Ok(ResolvedState::Random { region: *region, density: *density, state: *state, symmetry: *symmetry })
            }
        })
        .collect()
}

/// Rejects states at or above `states`.
pub(crate) fn check_states(iter: impl IntoIterator<Item = u8>, states: u16) -> Result<(), CaError> {
    match iter.into_iter().find(|&s| u16::from(s) >= states) {
        Some(s) => {
            Err(CaError::invalid(format!("state {s} does not exist in this model (states are 0 to {})", states - 1)))
        }
        None => Ok(()),
    }
}

/// A finite multi-state grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StateGrid {
    world: World,
    grid: Padded,
    next: Padded,
    activity: Option<Vec<u8>>,
}

impl StateGrid {
    /// Checks that `world` can hold a multi-state grid (finite, at most
    /// [`MAX_MULTISTATE_CELLS`] cells) without allocating it.
    pub(crate) fn check_world(world: &World) -> Result<(), CaError> {
        if !world.is_finite() {
            return Err(CaError::UnsupportedTopology {
                engine: "multi-state",
                topology: world.topology.label(),
                hint: "multi-state models run on finite worlds (bounded, torus, Klein bottle, cross-surface)",
            });
        }
        if world.cell_count() > MAX_MULTISTATE_CELLS {
            return Err(CaError::InvalidSize {
                width: u64::from(world.width),
                height: u64::from(world.height),
                reason: format!("multi-state worlds hold at most {MAX_MULTISTATE_CELLS} cells"),
            });
        }
        Ok(())
    }

    /// Empty grid on a finite world.
    pub(crate) fn new(world: World) -> Result<Self, CaError> {
        Self::check_world(&world)?;
        let (w, h) = (world.width as usize, world.height as usize);
        Ok(Self { world, grid: Padded::new(w, h), next: Padded::new(w, h), activity: None })
    }

    pub(crate) fn world(&self) -> World {
        self.world
    }

    /// State of a cell (mapped through the topology; 0 outside a bounded plane).
    pub(crate) fn get(&self, x: i64, y: i64) -> u8 {
        self.world.map(x, y).map_or(0, |(mx, my)| self.grid.get(mx as usize, my as usize))
    }

    /// Sets a cell; returns whether it lies in the world.
    pub(crate) fn set(&mut self, x: i64, y: i64, state: u8) -> bool {
        let Some((mx, my)) = self.world.map(x, y) else {
            return false;
        };
        let old = self.grid.set(mx as usize, my as usize, state);
        if old != state {
            self.touch(mx as usize, my as usize);
        }
        true
    }

    fn touch(&mut self, x: usize, y: usize) {
        if let Some(a) = &mut self.activity {
            a[y * self.world.width as usize + x] = 255;
        }
    }

    /// Enables or disables activity trails.
    pub(crate) fn set_tracking(&mut self, on: bool) {
        match (on, self.activity.is_some()) {
            (true, false) => self.activity = Some(vec![0; self.world.cell_count() as usize]),
            (false, true) => self.activity = None,
            _ => {}
        }
    }

    pub(crate) fn tracking(&self) -> bool {
        self.activity.is_some()
    }

    /// Activity of a cell (0 when not tracked).
    pub(crate) fn activity(&self, x: i64, y: i64) -> u8 {
        match (&self.activity, self.world.map(x, y)) {
            (Some(a), Some((mx, my))) => a[my as usize * self.world.width as usize + mx as usize],
            _ => 0,
        }
    }

    /// One generation through `lut`.
    pub(crate) fn step(&mut self, lut: &Lut) -> StepCounts {
        self.grid.fill_ghosts(self.world.topology);
        let counts = self.grid.step_into(lut, &mut self.next);
        if let Some(activity) = &mut self.activity {
            let w = self.world.width as usize;
            for y in 0..self.world.height as usize {
                let (old, new) = (self.grid.row(y), self.next.row(y));
                for (x, a) in activity[y * w..(y + 1) * w].iter_mut().enumerate() {
                    *a = if old[x] != new[x] { 255 } else { a.saturating_sub(STATE_TRAIL_DECAY) };
                }
            }
        }
        std::mem::swap(&mut self.grid, &mut self.next);
        counts
    }

    /// Non-zero cells `(x, y, state)`, row-major.
    pub(crate) fn cells(&self) -> Vec<(i64, i64, u8)> {
        let mut out = Vec::new();
        for y in 0..self.world.height as usize {
            for (x, &s) in self.grid.row(y).iter().enumerate() {
                if s != 0 {
                    out.push((x as i64, y as i64, s));
                }
            }
        }
        out
    }

    /// Number of cells per state (index = state).
    pub(crate) fn histogram(&self) -> [u64; 256] {
        let mut h = [0u64; 256];
        for y in 0..self.world.height as usize {
            for &s in self.grid.row(y) {
                h[usize::from(s)] += 1;
            }
        }
        h
    }

    /// Raw interior rows (for snapshots).
    pub(crate) fn raw(&self) -> Vec<u8> {
        (0..self.world.height as usize).flat_map(|y| self.grid.row(y).iter().copied()).collect()
    }

    /// Restores raw rows produced by [`StateGrid::raw`].
    pub(crate) fn load_raw(&mut self, raw: &[u8], states: u16) -> Result<(), CaError> {
        if raw.len() as u64 != self.world.cell_count() {
            return Err(CaError::invalid("snapshot cell data does not match the world size"));
        }
        check_states(raw.iter().copied(), states)?;
        let w = self.world.width as usize;
        for (i, &s) in raw.iter().enumerate() {
            self.grid.set(i % w, i / w, s);
        }
        Ok(())
    }

    pub(crate) fn activity_raw(&self) -> Option<Vec<u8>> {
        self.activity.clone()
    }

    pub(crate) fn set_activity_raw(&mut self, activity: Option<Vec<u8>>) -> Result<(), CaError> {
        if activity.as_ref().is_some_and(|a| a.len() as u64 != self.world.cell_count()) {
            return Err(CaError::invalid("snapshot activity data does not match the world size"));
        }
        self.activity = activity;
        Ok(())
    }

    pub(crate) fn clear(&mut self) {
        for (x, y, _) in self.cells() {
            self.set(x, y, 0);
        }
    }

    pub(crate) fn clear_rect(&mut self, rect: CellRect) {
        if let Some(clip) = self.world.clip(&rect) {
            for y in clip.y0..=clip.y1 {
                for x in clip.x0..=clip.x1 {
                    self.set(x, y, 0);
                }
            }
        }
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        self.grid.memory_bytes() + self.next.memory_bytes() + self.activity.as_ref().map_or(0, Vec::capacity)
    }

    /// Applies a random fill step.
    pub(crate) fn random(
        &mut self,
        rng: &mut SimRng,
        region: Option<CellRect>,
        density: f64,
        state: u8,
        symmetry: Symmetry,
    ) -> Result<Option<CellRect>, CaError> {
        let region = match region {
            Some(r) => self.world.clip(&r).ok_or_else(|| CaError::invalid("the region lies outside the world"))?,
            None => self.world.home(),
        };
        // Validated before anything changes; then streamed (no cell list).
        check_fill(region, density, symmetry, MAX_FILL_CELLS)?;
        self.clear_rect(region);
        random_fill_each(rng, region, density, symmetry, |x, y| {
            self.set(x, y, state);
        })?;
        Ok(Some(region))
    }

    /// Executes a painting command (`LoadPattern` and `ImportText` with
    /// `replace` are handled by the models). `states` bounds valid states.
    pub(crate) fn paint(&mut self, command: PaintCommand, states: u16, rng: &mut SimRng) -> Result<(), CaError> {
        let shape = |cells: Vec<(i64, i64)>, state: u8, grid: &mut Self| -> Result<(), CaError> {
            check_states([state], states)?;
            for (x, y) in cells {
                grid.set(x, y, state);
            }
            Ok(())
        };
        match command {
            PaintCommand::SetCell { x, y, state } => shape(vec![(x, y)], state, self),
            PaintCommand::Stroke { points, radius, state } => shape(stroke(&points, radius)?, state, self),
            PaintCommand::Line { from, to, state } => shape(line(from.0, from.1, to.0, to.1)?, state, self),
            PaintCommand::Rectangle { rect, filled, state } => shape(rectangle(rect, filled)?, state, self),
            PaintCommand::Circle { centre, radius, filled, state } => {
                let cells = if filled {
                    disc(centre.0, centre.1, radius)?
                } else {
                    circle_outline(centre.0, centre.1, radius)?
                };
                shape(cells, state, self)
            }
            PaintCommand::Paste { pattern, x, y, rotation, mirror } => {
                let cells = place_pattern(&pattern, x, y, rotation, mirror)?;
                check_states(cells.iter().map(|c| c.2), states)?;
                for (x, y, s) in cells {
                    self.set(x, y, s);
                }
                Ok(())
            }
            PaintCommand::ImportText { text, file_name, x, y, .. } => {
                let pattern = patterns::parse_any(&text, file_name.as_deref())?;
                self.paint(PaintCommand::Paste { pattern, x, y, rotation: Rotation::None, mirror: false }, states, rng)
            }
            PaintCommand::Clear => {
                self.clear();
                Ok(())
            }
            PaintCommand::ClearRegion { rect } => {
                self.clear_rect(rect);
                Ok(())
            }
            PaintCommand::Randomize { density, state, seed, region, symmetry } => {
                check_states([state], states)?;
                match seed {
                    Some(seed) => self.random(&mut SimRng::seed_from_u64(seed), region, density, state, symmetry)?,
                    None => self.random(rng, region, density, state, symmetry)?,
                };
                Ok(())
            }
            PaintCommand::LoadPattern { pattern, x, y } => {
                let cells = place_pattern(&pattern, x, y, Rotation::None, false)?;
                check_states(cells.iter().map(|c| c.2), states)?;
                self.clear();
                for (x, y, s) in cells {
                    self.set(x, y, s);
                }
                Ok(())
            }
        }
    }

    /// Builds the view (see [`crate::view`]): R = state (most frequent
    /// non-zero state for LOD texels), G = 0, B = activity (maximum),
    /// A = non-zero fraction.
    pub(crate) fn view(&self, request: &crate::view::GridViewRequest) -> GridView {
        let grid = TexelGrid::plan(request, self.world.bounds());
        let len = grid.len();
        let (mut occupied, mut state) = (vec![0u32; len], vec![0u8; len]);
        let mut activity = vec![0u8; len];
        if let Some(clip) = grid.clip {
            let width = grid.width as usize;
            let mut acc = RowAccumulator::new(width);
            let k = 1i64 << grid.log2_k;
            let (t0, t1) = (grid.row(clip.y0), grid.row(clip.y1));
            for ty in t0..=t1 {
                let top = grid.origin_y + ty as i64 * k;
                let (y0, y1) = (clip.y0.max(top), clip.y1.min(top + k - 1));
                for y in y0..=y1 {
                    let row = self.grid.row(y as usize);
                    for x in clip.x0..=clip.x1 {
                        let s = row[x as usize];
                        let col = grid.column(x);
                        acc.add(col, s);
                        if let Some(a) = &self.activity {
                            let v = a[y as usize * self.world.width as usize + x as usize];
                            let slot = &mut activity[ty * width + col];
                            *slot = (*slot).max(v);
                        }
                    }
                }
                let base = ty * width;
                acc.flush(&mut occupied[base..base + width], &mut state[base..base + width]);
            }
        }
        grid.assemble(&occupied, &state, &vec![0u8; len], &activity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::GridViewRequest;
    use simulation_core::Topology;

    #[test]
    fn paint_and_view() {
        let world = World::new(Topology::Torus, 8, 4).unwrap();
        let mut g = StateGrid::new(world).unwrap();
        let mut rng = SimRng::seed_from_u64(1);
        g.paint(PaintCommand::Line { from: (0, 1), to: (7, 1), state: 3 }, 4, &mut rng).unwrap();
        g.paint(PaintCommand::SetCell { x: -1, y: 0, state: 1 }, 4, &mut rng).unwrap();
        assert_eq!(g.get(7, 0), 1, "wrapped on the torus");
        assert!(g.paint(PaintCommand::SetCell { x: 0, y: 0, state: 4 }, 4, &mut rng).is_err());
        let view = g.view(&GridViewRequest { x0: 0, y0: 0, x1: 7, y1: 3, max_width: 8, max_height: 8 });
        assert_eq!(view.texel(7, 0), Some([1, 0, 0, 255]));
        assert_eq!(view.texel(0, 1), Some([3, 0, 0, 255]));
        let lod = g.view(&GridViewRequest { x0: 0, y0: 0, x1: 7, y1: 3, max_width: 4, max_height: 2 });
        assert_eq!(lod.cells_per_texel, 2);
        assert_eq!(lod.texel(0, 0), Some([3, 0, 0, 128]), "two wire cells out of four");
        assert_eq!(lod.texel(3, 0), Some([3, 0, 0, 191]), "two wires beat one head");
    }

    #[test]
    fn infinite_worlds_are_rejected() {
        assert!(StateGrid::new(World::new(Topology::Infinite, 8, 8).unwrap()).is_err());
    }
}
