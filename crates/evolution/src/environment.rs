//! The self-contained 2D environment: terrain with switchable barriers,
//! regions (islands or sectors), food that regrows logistically and an
//! analytic temperature field.
//!
//! * **Terrain.** The world starts as land; [`Barrier`]s are painted in list
//!   order (water and mountains are impassable; a `land` barrier opens a
//!   corridor through an earlier one). A cell belongs to a shape when its
//!   centre does.
//! * **Food.** Each land cell holds food `F ≤ K` (capacity: background or the
//!   richest patch covering it, times `capacity_scale`). Per tick, with
//!   `F₀ = max(F, s·K)` (seed bank `s`), the exact solution of the logistic
//!   equation over one tick is applied:
//!   `F₁ = K F₀ / (F₀ + (K − F₀) e^{−r})`, `F ← F + (F₁ − F₀)`.
//!   The closed form is unconditionally stable, also above `K` (food added
//!   by a command decays back towards `K`).
//! * **Temperature.** `T(x, y, t) = T₀ + G_x (x/W − ½) + G_y (y/H − ½) + Δ(t) + A sin(2π t / P)`,
//!   where the offset `Δ` accumulates `shift_rate / 1000` per tick (and
//!   command-driven jumps), so a rate change never makes the field jump.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simulation_core::glam::{DVec2, Vec2};
use simulation_core::{Grid2, SimRng};

use crate::config::{Barrier, ClimateConfig, EvolutionConfig, FoodConfig, RegionMode, Shape, Terrain};
use crate::error::EvolutionError;

/// Region label of impassable cells.
pub const NO_REGION: u16 = u16::MAX;

/// Serialisable part of the environment (everything else is derived from
/// the configuration).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentState {
    /// Food per cell.
    pub food: Grid2<f32>,
    /// Whether each configured barrier is currently active.
    pub barrier_active: Vec<bool>,
    /// Accumulated temperature offset Δ (°C).
    pub temperature_offset: f64,
}

/// The environment of an evolution experiment.
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    terrain: Grid2<Terrain>,
    capacity: Grid2<f32>,
    food: Grid2<f32>,
    regions: Grid2<u16>,
    region_count: usize,
    barrier_active: Vec<bool>,
    temperature_offset: f64,
    has_obstacles: bool,
    /// Mean centre of the passable cells (`None` without land).
    land_centroid: Option<Vec2>,
}

impl Environment {
    /// Builds the initial environment of `config` (food at
    /// `initial_fraction × capacity`).
    pub fn new(config: &EvolutionConfig) -> Self {
        let (w, h) = (config.world.width as usize, config.world.height as usize);
        let mut env = Self {
            terrain: Grid2::new(w, h, Terrain::Land),
            capacity: Grid2::new(w, h, 0.0),
            food: Grid2::new(w, h, 0.0),
            regions: Grid2::new(w, h, NO_REGION),
            region_count: 0,
            barrier_active: config.barriers.iter().map(|b| b.active).collect(),
            temperature_offset: 0.0,
            has_obstacles: false,
            land_centroid: None,
        };
        env.repaint(config);
        let fraction = config.food.initial_fraction;
        for (food, k) in env.food.as_mut_slice().iter_mut().zip(env.capacity.as_slice()) {
            *food = fraction * k;
        }
        env
    }

    /// Rebuilds an environment from saved state, checking it against `config`.
    pub fn from_state(config: &EvolutionConfig, state: EnvironmentState) -> Result<Self, EvolutionError> {
        let (w, h) = (config.world.width as usize, config.world.height as usize);
        if state.food.width() != w || state.food.height() != h {
            return Err(EvolutionError::config(
                "snapshot",
                format!("food grid is {}×{} but the world is {w}×{h}", state.food.width(), state.food.height()),
            ));
        }
        if state.barrier_active.len() != config.barriers.len() {
            return Err(EvolutionError::config(
                "snapshot",
                format!("{} barrier states for {} barriers", state.barrier_active.len(), config.barriers.len()),
            ));
        }
        if state.food.as_slice().iter().any(|f| !f.is_finite() || *f < 0.0) || !state.temperature_offset.is_finite() {
            return Err(EvolutionError::config("snapshot", "food and temperature values must be finite"));
        }
        let mut env = Self::new(config);
        env.barrier_active = state.barrier_active;
        env.temperature_offset = state.temperature_offset;
        env.repaint(config);
        env.food = state.food;
        Ok(env)
    }

    /// The serialisable state.
    pub fn state(&self) -> EnvironmentState {
        EnvironmentState {
            food: self.food.clone(),
            barrier_active: self.barrier_active.clone(),
            temperature_offset: self.temperature_offset,
        }
    }

    /// Repaints terrain, capacity and regions from the configuration and the
    /// current barrier states; food on cells that became impassable is lost.
    fn repaint(&mut self, config: &EvolutionConfig) {
        let (w, h) = (self.terrain.width(), self.terrain.height());
        self.terrain.fill(Terrain::Land);
        for (barrier, _) in config.barriers.iter().zip(&self.barrier_active).filter(|(_, active)| **active) {
            paint_barrier(&mut self.terrain, barrier);
        }
        self.has_obstacles = self.terrain.as_slice().iter().any(|t| !t.is_passable());
        let (sum, n) = self
            .terrain
            .as_slice()
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_passable())
            .fold((DVec2::ZERO, 0usize), |(s, n), (i, _)| (s + DVec2::new((i % w) as f64, (i / w) as f64), n + 1));
        self.land_centroid = (n > 0).then(|| (sum / n as f64 + DVec2::splat(0.5)).as_vec2());
        self.refresh_capacity(&config.food);
        let (regions, count) = match config.regions {
            RegionMode::Islands => label_islands(&self.terrain),
            RegionMode::Sectors { cols, rows } => label_sectors(&self.terrain, cols as usize, rows as usize),
        };
        self.regions = regions;
        self.region_count = count;
        for (food, t) in self.food.as_mut_slice().iter_mut().zip(self.terrain.as_slice()) {
            if !t.is_passable() {
                *food = 0.0;
            }
        }
        debug_assert_eq!((self.food.width(), self.food.height()), (w, h));
    }

    /// Recomputes carrying capacities (after a capacity-scale change).
    pub fn refresh_capacity(&mut self, food: &FoodConfig) {
        let w = self.terrain.width();
        for (i, (k, t)) in self.capacity.as_mut_slice().iter_mut().zip(self.terrain.as_slice()).enumerate() {
            *k = if t.is_passable() {
                let (cx, cy) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                let patch = food.patches.iter().filter(|p| p.shape.contains(cx, cy)).map(|p| p.capacity);
                patch.fold(food.capacity, f32::max) * food.capacity_scale
            } else {
                0.0
            };
        }
    }

    /// Width in cells.
    pub fn width(&self) -> usize {
        self.terrain.width()
    }

    /// Height in cells.
    pub fn height(&self) -> usize {
        self.terrain.height()
    }

    /// Terrain grid.
    pub fn terrain(&self) -> &Grid2<Terrain> {
        &self.terrain
    }

    /// Food grid.
    pub fn food(&self) -> &Grid2<f32> {
        &self.food
    }

    /// Carrying-capacity grid.
    pub fn capacity(&self) -> &Grid2<f32> {
        &self.capacity
    }

    /// Region label per cell ([`NO_REGION`] on impassable cells).
    pub fn regions(&self) -> &Grid2<u16> {
        &self.regions
    }

    /// Number of regions.
    pub fn region_count(&self) -> usize {
        self.region_count
    }

    /// Whether any cell is impassable (enables obstacle avoidance).
    pub fn has_obstacles(&self) -> bool {
        self.has_obstacles
    }

    /// Current barrier states, in configuration order.
    pub fn barrier_active(&self) -> &[bool] {
        &self.barrier_active
    }

    /// Accumulated temperature offset Δ (°C).
    pub fn temperature_offset(&self) -> f64 {
        self.temperature_offset
    }

    /// Linear cell index of a position, if inside the world.
    pub fn cell_of(&self, p: Vec2) -> Option<usize> {
        let (x, y) = (p.x.floor(), p.y.floor());
        self.food.contains(x as i64, y as i64).then(|| self.food.index(x as usize, y as usize))
    }

    /// Whether an organism may stand at `p`.
    pub fn is_passable(&self, p: Vec2) -> bool {
        p.is_finite() && self.cell_of(p).is_some_and(|i| self.terrain.as_slice()[i].is_passable())
    }

    /// Food at `p` (0 outside the world).
    pub fn food_at(&self, p: Vec2) -> f32 {
        self.cell_of(p).map_or(0.0, |i| self.food.as_slice()[i])
    }

    /// Region of the cell under `p`, if passable.
    pub fn region_at(&self, p: Vec2) -> Option<u16> {
        self.cell_of(p).map(|i| self.regions.as_slice()[i]).filter(|r| *r != NO_REGION)
    }

    /// Removes up to `amount` food from cell `index` and returns what was taken.
    pub fn take_food(&mut self, index: usize, amount: f32) -> f32 {
        let Some(food) = self.food.as_mut_slice().get_mut(index) else {
            return 0.0;
        };
        let taken = amount.clamp(0.0, *food);
        *food -= taken;
        taken
    }

    /// Adds `amount` food to every land cell inside `shape`; returns the total added.
    pub fn add_food(&mut self, shape: &Shape, amount: f32) -> f64 {
        let w = self.width();
        let mut total = 0.0;
        for (i, (food, t)) in self.food.as_mut_slice().iter_mut().zip(self.terrain.as_slice()).enumerate() {
            if t.is_passable() && shape.contains((i % w) as f32 + 0.5, (i / w) as f32 + 0.5) {
                *food += amount;
                total += f64::from(amount);
            }
        }
        total
    }

    /// Temperature at `p` at `tick` (see the module documentation).
    pub fn temperature_at(&self, climate: &ClimateConfig, p: Vec2, tick: u64) -> f32 {
        let (w, h) = (self.width() as f32, self.height() as f32);
        let season = if climate.seasonal_amplitude != 0.0 {
            let period = climate.seasonal_period.max(1);
            let phase = (tick % u64::from(period)) as f64 / f64::from(period);
            climate.seasonal_amplitude * (std::f64::consts::TAU * phase).sin() as f32
        } else {
            0.0
        };
        climate.base_temperature
            + climate.gradient_x * (p.x / w - 0.5)
            + climate.gradient_y * (p.y / h - 0.5)
            + self.temperature_offset as f32
            + season
    }

    /// Mean temperature over the centres of passable cells at `tick` (0
    /// without land). The field is affine in position, so the mean equals the
    /// temperature at the mean cell centre, which is cached when the terrain
    /// is painted: `O(1)` instead of one evaluation per cell.
    pub fn mean_land_temperature(&self, climate: &ClimateConfig, tick: u64) -> f64 {
        self.land_centroid.map_or(0.0, |c| f64::from(self.temperature_at(climate, c, tick)))
    }

    /// Adds `delta` °C to the temperature offset (a sudden climate change).
    pub fn shift_temperature(&mut self, delta: f32) {
        self.temperature_offset += f64::from(delta);
    }

    /// One tick: advances the climate offset and regrows food.
    pub fn advance(&mut self, config: &EvolutionConfig) {
        self.temperature_offset += f64::from(config.climate.shift_rate) / 1000.0;
        let r = config.food.regrowth_rate;
        if r <= 0.0 {
            return;
        }
        let decay = (-r).exp();
        let seed_bank = config.food.seed_bank;
        // Pure per-cell update: parallel and deterministic.
        self.food.as_mut_slice().par_iter_mut().zip(self.capacity.as_slice().par_iter()).for_each(|(food, &k)| {
            if k <= 0.0 {
                *food *= decay;
                return;
            }
            let f0 = food.max(seed_bank * k);
            if f0 <= 0.0 {
                return;
            }
            let f1 = k * f0 / (f0 + (k - f0) * decay);
            *food = (*food + (f1 - f0)).max(0.0);
        });
    }

    /// Switches barrier `index` on or off and repaints the terrain.
    pub fn set_barrier(&mut self, config: &EvolutionConfig, index: usize, active: bool) -> Result<(), EvolutionError> {
        let count = self.barrier_active.len();
        let slot = self.barrier_active.get_mut(index).ok_or_else(|| {
            EvolutionError::InvalidCommand(format!("barrier {index} does not exist (the scenario has {count})"))
        })?;
        *slot = active;
        self.repaint(config);
        Ok(())
    }

    /// Indices of passable cells whose centre lies in `area` (all if `None`).
    pub fn passable_cells(&self, area: Option<&Shape>) -> Vec<usize> {
        let w = self.width();
        self.terrain
            .as_slice()
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                t.is_passable() && area.is_none_or(|a| a.contains((i % w) as f32 + 0.5, (i / w) as f32 + 0.5))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Uniform random point inside cell `index` (two draws).
    pub fn random_point_in_cell(&self, index: usize, rng: &mut SimRng) -> Vec2 {
        let w = self.width();
        let max = Vec2::new(self.width() as f32, self.height() as f32) - Vec2::splat(1e-3);
        (Vec2::new((index % w) as f32, (index / w) as f32) + Vec2::new(rng.f32(), rng.f32())).min(max)
    }

    /// Total food in the world.
    pub fn total_food(&self) -> f64 {
        self.food.sum()
    }
}

fn paint_barrier(terrain: &mut Grid2<Terrain>, barrier: &Barrier) {
    let w = terrain.width();
    for (i, cell) in terrain.as_mut_slice().iter_mut().enumerate() {
        let (cx, cy) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
        let inside = barrier.shapes.iter().any(|s| s.contains(cx, cy));
        if inside != barrier.outside {
            *cell = barrier.terrain;
        }
    }
}

/// Connected components of passable cells (4-neighbourhood), numbered in
/// row-major order of their first cell. Components beyond `u16::MAX − 1`
/// stay unlabelled.
fn label_islands(terrain: &Grid2<Terrain>) -> (Grid2<u16>, usize) {
    let (w, h) = (terrain.width(), terrain.height());
    let mut labels = Grid2::new(w, h, NO_REGION);
    let mut count = 0usize;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !terrain.as_slice()[start].is_passable() || labels.as_slice()[start] != NO_REGION {
            continue;
        }
        if count >= usize::from(NO_REGION) {
            break;
        }
        let label = count as u16;
        count += 1;
        labels.as_mut_slice()[start] = label;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let neighbours = [
                (x > 0).then(|| i - 1),
                (x + 1 < w).then_some(i + 1),
                (y > 0).then(|| i - w),
                (y + 1 < h).then_some(i + w),
            ];
            for j in neighbours.into_iter().flatten() {
                if terrain.as_slice()[j].is_passable() && labels.as_slice()[j] == NO_REGION {
                    labels.as_mut_slice()[j] = label;
                    stack.push(j);
                }
            }
        }
    }
    (labels, count)
}

/// `cols × rows` sectors (row-major numbering), passable cells only.
fn label_sectors(terrain: &Grid2<Terrain>, cols: usize, rows: usize) -> (Grid2<u16>, usize) {
    let (w, h) = (terrain.width(), terrain.height());
    let (cols, rows) = (cols.clamp(1, w), rows.clamp(1, h));
    let labels = Grid2::from_fn(w, h, |x, y| {
        if terrain.get(x, y).is_passable() { ((y * rows / h) * cols + x * cols / w) as u16 } else { NO_REGION }
    });
    (labels, cols * rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FoodPatch;

    fn config_with(barriers: Vec<Barrier>) -> EvolutionConfig {
        let mut c = EvolutionConfig::default();
        c.world.width = 20;
        c.world.height = 10;
        c.barriers = barriers;
        c
    }

    fn river(active: bool) -> Barrier {
        Barrier {
            name: "river".into(),
            terrain: Terrain::Water,
            shapes: vec![Shape::Rect { x: 9.0, y: 0.0, w: 2.0, h: 10.0 }],
            outside: false,
            active,
        }
    }

    #[test]
    fn barriers_split_the_world_into_islands_and_can_be_toggled() {
        let c = config_with(vec![river(true)]);
        let mut env = Environment::new(&c);
        assert_eq!(env.region_count(), 2);
        assert!(!env.is_passable(Vec2::new(9.5, 3.0)) && env.is_passable(Vec2::new(8.5, 3.0)));
        assert_eq!(env.region_at(Vec2::new(1.0, 1.0)), Some(0));
        assert_eq!(env.region_at(Vec2::new(15.0, 1.0)), Some(1));
        assert_eq!(env.food_at(Vec2::new(9.5, 3.0)), 0.0, "no food in water");
        env.set_barrier(&c, 0, false).unwrap();
        assert_eq!(env.region_count(), 1);
        assert!(env.is_passable(Vec2::new(9.5, 3.0)));
        assert!(env.set_barrier(&c, 3, true).is_err());
        assert!(!env.is_passable(Vec2::new(-0.5, 3.0)) && !env.is_passable(Vec2::new(20.0, 3.0)));
    }

    #[test]
    fn outside_barriers_make_islands_and_land_barriers_bridge_them() {
        let ocean = Barrier {
            name: "ocean".into(),
            terrain: Terrain::Water,
            shapes: vec![Shape::Disc { x: 4.0, y: 5.0, r: 3.0 }, Shape::Disc { x: 16.0, y: 5.0, r: 3.0 }],
            outside: true,
            active: true,
        };
        let bridge = Barrier {
            name: "bridge".into(),
            terrain: Terrain::Land,
            shapes: vec![Shape::Rect { x: 4.0, y: 4.0, w: 12.0, h: 2.0 }],
            outside: false,
            active: false,
        };
        let c = config_with(vec![ocean, bridge]);
        let mut env = Environment::new(&c);
        assert_eq!(env.region_count(), 2);
        env.set_barrier(&c, 1, true).unwrap();
        assert_eq!(env.region_count(), 1);
    }

    #[test]
    fn sectors_follow_the_grid() {
        let mut c = config_with(vec![]);
        c.regions = RegionMode::Sectors { cols: 4, rows: 1 };
        let env = Environment::new(&c);
        assert_eq!(env.region_count(), 4);
        assert_eq!(env.region_at(Vec2::new(0.5, 9.5)), Some(0));
        assert_eq!(env.region_at(Vec2::new(19.5, 0.5)), Some(3));
    }

    #[test]
    fn food_regrows_logistically_and_patches_raise_capacity() {
        let mut c = config_with(vec![]);
        c.food.capacity = 10.0;
        c.food.patches.push(FoodPatch { shape: Shape::Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, capacity: 30.0 });
        c.food.initial_fraction = 0.0;
        c.food.seed_bank = 0.0;
        c.food.regrowth_rate = 0.02;
        let mut env = Environment::new(&c);
        assert_eq!(env.capacity().get(0, 0), &30.0);
        assert_eq!(env.capacity().get(5, 5), &10.0);
        assert_eq!(env.total_food(), 0.0);
        env.advance(&c);
        assert_eq!(env.total_food(), 0.0, "without a seed bank an empty cell stays empty");
        // Logistic solution from F0 = 1: F(t) = K / (1 + (K − 1) e^{−rt}).
        let i = env.food.index(5, 5);
        env.food.as_mut_slice()[i] = 1.0;
        for _ in 0..50 {
            env.advance(&c);
        }
        let expected = 10.0 / (1.0 + 9.0 * (-0.02f32 * 50.0).exp());
        assert!((env.food.as_slice()[i] - expected).abs() < 1e-3, "{} vs {expected}", env.food.as_slice()[i]);
        // Above capacity the food decays back towards K and never goes negative.
        env.food.as_mut_slice()[i] = 1000.0;
        c.food.regrowth_rate = 0.5;
        env.advance(&c);
        let after = env.food.as_slice()[i];
        assert!(after > 10.0 && after < 1000.0, "{after}");
        // The seed bank lets grazed-out cells regrow.
        c.food.seed_bank = 0.05;
        env.food.as_mut_slice()[i] = 0.0;
        env.advance(&c);
        assert!(env.food.as_slice()[i] > 0.0);
    }

    #[test]
    fn temperature_field_has_gradient_shift_and_season() {
        let mut c = config_with(vec![]);
        c.climate.gradient_x = 10.0;
        c.climate.shift_rate = 2.0;
        let mut env = Environment::new(&c);
        let centre = Vec2::new(10.0, 5.0);
        assert_eq!(env.temperature_at(&c.climate, centre, 0), 20.0);
        assert_eq!(env.temperature_at(&c.climate, Vec2::new(20.0, 5.0), 0), 25.0);
        for _ in 0..500 {
            env.advance(&c);
        }
        assert!((env.temperature_at(&c.climate, centre, 500) - 21.0).abs() < 1e-4);
        c.climate.seasonal_amplitude = 4.0;
        c.climate.seasonal_period = 100;
        let quarter = env.temperature_at(&c.climate, centre, 25);
        assert!((quarter - 25.0).abs() < 1e-3, "{quarter}");
        assert!((env.mean_land_temperature(&c.climate, 0) - 21.0).abs() < 1e-3);
    }

    #[test]
    fn mean_land_temperature_matches_the_cell_average() {
        let ocean = Barrier {
            name: "ocean".into(),
            terrain: Terrain::Water,
            shapes: vec![Shape::Disc { x: 4.0, y: 3.0, r: 3.0 }, Shape::Rect { x: 13.0, y: 2.0, w: 6.0, h: 7.0 }],
            outside: true,
            active: true,
        };
        let mut c = config_with(vec![ocean]);
        c.climate.gradient_x = 17.0;
        c.climate.gradient_y = -9.0;
        c.climate.seasonal_amplitude = 3.0;
        c.climate.seasonal_period = 50;
        let mut env = Environment::new(&c);
        env.shift_temperature(-2.5);
        let w = env.width();
        for tick in [0, 7, 31] {
            let cells: Vec<f64> = env
                .passable_cells(None)
                .into_iter()
                .map(|i| {
                    let centre = Vec2::new((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                    f64::from(env.temperature_at(&c.climate, centre, tick))
                })
                .collect();
            let brute = cells.iter().sum::<f64>() / cells.len() as f64;
            let fast = env.mean_land_temperature(&c.climate, tick);
            assert!((fast - brute).abs() < 1e-4, "tick {tick}: {fast} vs {brute}");
        }
        env.set_barrier(&c, 0, false).unwrap();
        assert!((env.mean_land_temperature(&c.climate, 0) - 17.5).abs() < 1e-5, "whole world: centre value");
    }

    #[test]
    fn state_round_trip_rebuilds_derived_grids() {
        let c = config_with(vec![river(false)]);
        let mut env = Environment::new(&c);
        env.set_barrier(&c, 0, true).unwrap();
        env.shift_temperature(1.5);
        let back = Environment::from_state(&c, env.state()).unwrap();
        assert_eq!(back, env);
        let mut bad = env.state();
        bad.barrier_active.push(true);
        assert!(Environment::from_state(&c, bad).is_err());
    }
}
