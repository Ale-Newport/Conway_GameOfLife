//! `LangtonsAntSim`: Langton's ant and multi-colour turmites.
//!
//! Ants are agents walking on a grid of colours, not a CA encoding (the
//! research note recommends this over Golly's state encoding). A rule string
//! such as `RL` (Langton's ant), `LLRR` or `RLR` gives one turn per colour:
//! `L` left, `R` right, `N` no turn, `U` U-turn.

mod colors;
mod rule;

pub use colors::ColorGrid;
pub use rule::{Heading, MAX_COLORS, TurmiteRule, Turn};

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamSpec, ParamValue, SimError, SimRng,
    Simulation, Snapshot, Topology,
};

use crate::error::CaError;
use crate::geometry::{
    CellRect, MAX_EDIT_CELLS, MAX_FILL_CELLS, MAX_WORLD_SIDE, Symmetry, World, check_fill, random_fill_each, stroke,
};
use crate::view::{GridView, GridViewRequest};

/// Model id.
pub const ANT_MODEL_ID: &str = "ca.langtons_ant";
const SCHEMA_VERSION: u32 = 1;
/// Largest number of ants.
pub const MAX_ANTS: usize = 4096;
/// Ticks a trail entry stays visible.
const TRAIL_TICKS: u64 = 16;
/// Activity lost per tick by a trail entry (255 in the tick of the flip).
pub const ANT_TRAIL_DECAY: u8 = 16;
/// Largest number of trail entries kept (the most recent flips); only
/// reached with very many steps per tick.
pub const MAX_TRAIL: usize = 1 << 20;

/// One ant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ant {
    /// Column.
    pub x: i64,
    /// Row.
    pub y: i64,
    /// Facing direction.
    pub heading: Heading,
}

/// Configuration of a [`LangtonsAntSim`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AntConfig {
    /// Turmite rule (`RL`, `LLRR`...).
    pub rule: String,
    /// `Infinite`, `Torus` or `Bounded` (ants leaving a bounded world are removed).
    #[serde(default = "infinite")]
    pub topology: Topology,
    /// Width (home region on the infinite plane).
    #[serde(default = "side")]
    pub width: u32,
    /// Height.
    #[serde(default = "side")]
    pub height: u32,
    /// Ant steps per tick.
    #[serde(default = "one")]
    pub steps_per_tick: u64,
    /// Activity trails.
    #[serde(default)]
    pub track_activity: bool,
    /// Initial ants, moved in this order every step.
    pub ants: Vec<Ant>,
}

fn infinite() -> Topology {
    Topology::Infinite
}

fn side() -> u32 {
    256
}

fn one() -> u64 {
    1
}

impl Default for AntConfig {
    fn default() -> Self {
        Self {
            rule: "RL".into(),
            topology: Topology::Infinite,
            width: 256,
            height: 256,
            steps_per_tick: 1,
            track_activity: true,
            ants: vec![Ant { x: 0, y: 0, heading: Heading::North }],
        }
    }
}

/// Mutations of a [`LangtonsAntSim`].
#[derive(Clone, Debug, PartialEq)]
pub enum AntCommand {
    /// Paints one cell.
    SetCell {
        /// Column.
        x: i64,
        /// Row.
        y: i64,
        /// Colour (below the rule's colour count).
        color: u8,
    },
    /// Paints a brush stroke.
    Stroke {
        /// Points.
        points: Vec<(i64, i64)>,
        /// Radius.
        radius: u32,
        /// Colour.
        color: u8,
    },
    /// Scatters `color` at random over a region (other cells become colour 0).
    Randomize {
        /// Probability.
        density: f64,
        /// Colour.
        color: u8,
        /// Dedicated seed, or the model's generator.
        seed: Option<u64>,
        /// Region (default: the world, or the home region of the infinite plane).
        region: Option<CellRect>,
    },
    /// Sets every cell to colour 0 (ants stay).
    ClearColors,
    /// Adds an ant (moved after the existing ones).
    AddAnt(Ant),
    /// Removes the ants inside a rectangle.
    RemoveAnts {
        /// The rectangle.
        rect: CellRect,
    },
}

/// Render data: the colour grid plus ant positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AntView {
    /// Colours (R = colour; B = trail of recently flipped cells; A = non-zero).
    pub grid: GridView,
    /// Ants inside the requested rectangle.
    pub ants: Vec<Ant>,
}

/// Langton's ant and turmites as a [`Simulation`]; one tick = `steps_per_tick`
/// steps, and [`Simulation::tick`] counts steps.
///
/// One step moves every ant once, in order: the ant turns by the rule's turn
/// for the colour it stands on, advances that cell's colour by one (modulo
/// the number of colours), then moves one cell forward. Ants never collide;
/// two ants on one cell act one after the other. On a torus ants wrap; on a
/// bounded plane an ant stepping outside is removed (the model finishes when
/// no ant is left).
pub struct LangtonsAntSim {
    config: AntConfig,
    rule: TurmiteRule,
    world: World,
    colors: ColorGrid,
    ants: Vec<Ant>,
    steps: u64,
    steps_per_tick: u64,
    pending_steps: Option<u64>,
    rng: SimRng,
    seed: u64,
    /// Ticks (calls of `step`) since the last reset; trail entries record
    /// the tick of their flip.
    ticks: u64,
    trail: VecDeque<(i64, i64, u64)>,
    track_activity: bool,
    reset_point: Option<(ColorGrid, Vec<Ant>, SimRng)>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    config: AntConfig,
    /// The running rule (`config.rule` is the rule of the next reset).
    rule: String,
    ticks: u64,
    colors: ColorGrid,
    ants: Vec<Ant>,
    steps: u64,
    steps_per_tick: u64,
    pending_steps: Option<u64>,
    rng: SimRng,
    seed: u64,
    trail: Vec<(i64, i64, u64)>,
    track_activity: bool,
    reset_point: Option<(ColorGrid, Vec<Ant>, SimRng)>,
}

impl LangtonsAntSim {
    /// Builds the model.
    ///
    /// # Errors
    /// Invalid rule, world (Klein bottles and cross-surfaces are not offered),
    /// too many ants, or ants outside a finite world.
    pub fn new(config: &AntConfig, seed: u64) -> Result<Self, CaError> {
        let rule = TurmiteRule::parse(&config.rule)?;
        let world = World::new(config.topology, config.width, config.height)?;
        if matches!(world.topology, Topology::KleinBottle | Topology::CrossSurface) {
            return Err(CaError::invalid("ants run on the infinite plane, a torus or a bounded plane"));
        }
        if config.ants.len() > MAX_ANTS {
            return Err(CaError::LimitExceeded(format!("at most {MAX_ANTS} ants")));
        }
        if !(1..=1_000_000).contains(&config.steps_per_tick) {
            return Err(CaError::invalid("steps per tick must be between 1 and 1000000"));
        }
        let ants = config
            .ants
            .iter()
            .map(|a| {
                world.map(a.x, a.y).map(|(x, y)| Ant { x, y, heading: a.heading }).ok_or_else(|| {
                    CaError::invalid(format!("the ant at ({}, {}) lies outside the bounded world", a.x, a.y))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut stored = config.clone();
        stored.rule = rule.to_string();
        Ok(Self {
            config: stored,
            rule,
            world,
            colors: ColorGrid::new(world)?,
            ants,
            steps: 0,
            steps_per_tick: config.steps_per_tick,
            pending_steps: None,
            rng: SimRng::seed_from_u64(seed),
            seed,
            ticks: 0,
            trail: VecDeque::new(),
            track_activity: config.track_activity,
            reset_point: None,
        })
    }

    /// Langton's ant (`RL`) alone on the infinite plane, heading north at (0, 0).
    pub fn langton(seed: u64) -> Self {
        match Self::new(&AntConfig::default(), seed) {
            Ok(sim) => sim,
            Err(e) => unreachable!("the default ant configuration is valid: {e}"),
        }
    }

    /// The rule.
    pub fn rule(&self) -> &TurmiteRule {
        &self.rule
    }

    /// The ants, in move order.
    pub fn ants(&self) -> &[Ant] {
        &self.ants
    }

    /// Colour of a cell.
    pub fn color(&self, x: i64, y: i64) -> u8 {
        self.world.map(x, y).map_or(0, |(x, y)| self.colors.get(x, y))
    }

    /// Number of cells with a non-zero colour.
    pub fn colored(&self) -> u64 {
        self.colors.nonzero()
    }

    /// Steps taken since the last reset.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Moves every ant once.
    fn step_once(&mut self) {
        let n = self.rule.colors();
        let mut kept = Vec::with_capacity(self.ants.len());
        for i in 0..self.ants.len() {
            let mut ant = self.ants[i];
            let color = self.colors.get(ant.x, ant.y);
            ant.heading = ant.heading.turned(self.rule.turn(color));
            self.colors.set(ant.x, ant.y, (color + 1) % n);
            if self.track_activity {
                if self.trail.len() == MAX_TRAIL {
                    self.trail.pop_front();
                }
                self.trail.push_back((ant.x, ant.y, self.ticks));
            }
            let (dx, dy) = ant.heading.delta();
            let moved = ant.x.checked_add(dx).zip(ant.y.checked_add(dy)).and_then(|(x, y)| self.world.map(x, y));
            if let Some((x, y)) = moved {
                ant.x = x;
                ant.y = y;
                kept.push(ant);
            }
        }
        self.ants = kept;
        self.steps += 1;
    }

    fn after_edit(&mut self) {
        if self.steps == 0 {
            self.reset_point = None;
        }
    }

    fn rules_text(&self) -> String {
        format!(
            "## Turmite {rule} ({n} colours)\n\n\
             Every step moves each ant once, in a fixed order. An ant standing on a cell of colour *c*:\n\n\
             1. turns by letter *c* of the rule (`L` 90° left, `R` 90° right, `N` none, `U` 180°);\n\
             2. changes the cell to colour (*c* + 1) mod {n};\n\
             3. moves one cell forward.\n\n\
             `RL` is Langton's ant (white → turn right, black → turn left). Ants do not interact except through \
             the colours; on a bounded plane an ant leaving the world is removed, on a torus it wraps.",
            rule = self.rule,
            n = self.rule.colors()
        )
    }

    fn params_specs() -> Vec<ParamSpec> {
        vec![
            ParamSpec::text("rule", "Turmite rule", MAX_COLORS, "RL")
                .group("Rules")
                .apply(ApplyPolicy::RequiresReset)
                .describe("One letter per colour: L, R, N (no turn), U (U-turn). RL is Langton's ant."),
            ParamSpec::choice(
                "topology",
                "Topology",
                &[("infinite", "Infinite plane"), ("torus", "Torus"), ("bounded", "Bounded plane")],
                "infinite",
            )
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
            ParamSpec::int("width", "Width", 1, i64::from(MAX_WORLD_SIDE), 256)
                .group("World")
                .apply(ApplyPolicy::RequiresReset),
            ParamSpec::int("height", "Height", 1, i64::from(MAX_WORLD_SIDE), 256)
                .group("World")
                .apply(ApplyPolicy::RequiresReset),
            ParamSpec::int("steps_per_tick", "Steps per tick", 1, 1_000_000, 1)
                .group("Speed")
                .apply(ApplyPolicy::NextTick)
                .describe("Ant steps advanced by one tick."),
            ParamSpec::boolean("track_activity", "Show trail", true)
                .group("Display")
                .apply(ApplyPolicy::Immediate)
                .describe(
                    "Highlight flipped cells (view channel B): 255 for the cells flipped during the last tick, \
                     16 less per tick afterwards.",
                ),
        ]
    }
}

impl Simulation for LangtonsAntSim {
    type Command = AntCommand;
    type ViewRequest = GridViewRequest;
    type View = AntView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(ANT_MODEL_ID, "Langton's ant & turmites", ModelFamily::AgentBased)
            .summary("Ants that turn by the colour under them and recolour it: Langton's highway, cardioids, chaos.")
            .rules(&self.rules_text())
            .params(Self::params_specs())
            .metrics(vec![
                MetricSpec::gauge(
                    "colored",
                    "Coloured cells",
                    "cells",
                    "Cells whose colour is not 0 (black cells for RL).",
                ),
                MetricSpec::gauge("ants", "Ants", "", "Ants still in the world."),
                MetricSpec::gauge("steps", "Steps", "steps", "Steps since the last reset."),
            ])
            .time("step", 1.0)
    }

    fn tick(&self) -> u64 {
        self.steps
    }

    fn step(&mut self) {
        if let Some(n) = self.pending_steps.take() {
            self.steps_per_tick = n;
        }
        if self.steps == 0 && self.reset_point.is_none() {
            self.reset_point = Some((self.colors.clone(), self.ants.clone(), self.rng.clone()));
        }
        for _ in 0..self.steps_per_tick {
            if self.ants.is_empty() {
                break;
            }
            self.step_once();
        }
        self.ticks += 1;
        while self.trail.front().is_some_and(|&(_, _, t)| self.ticks - t > TRAIL_TICKS) {
            self.trail.pop_front();
        }
    }

    fn reset(&mut self) {
        if let Some(n) = self.pending_steps.take() {
            self.steps_per_tick = n;
        }
        let same_world = self.config.topology == self.world.topology
            && (self.config.width, self.config.height) == (self.world.width, self.world.height)
            && self.config.rule == self.rule.to_string();
        match self.reset_point.clone() {
            Some((colors, ants, rng)) if same_world => {
                self.colors = colors;
                self.ants = ants;
                self.rng = rng;
                self.steps = 0;
                self.ticks = 0;
                self.trail.clear();
            }
            _ => {
                let mut config = self.config.clone();
                config.steps_per_tick = self.steps_per_tick;
                config.track_activity = self.track_activity;
                // The configuration was validated by set_param; keep the state otherwise.
                if let Ok(fresh) = Self::new(&config, self.seed) {
                    *self = fresh;
                }
            }
        }
    }

    fn apply(&mut self, command: AntCommand) -> Result<(), SimError> {
        let n = self.rule.colors();
        let check = |c: u8| {
            if c < n {
                Ok(())
            } else {
                Err(CaError::invalid(format!("colour {c} does not exist (the rule has {n} colours)")))
            }
        };
        match command {
            AntCommand::SetCell { x, y, color } => {
                check(color)?;
                if let Some((x, y)) = self.world.map(x, y) {
                    self.colors.set(x, y, color);
                }
            }
            AntCommand::Stroke { points, radius, color } => {
                check(color)?;
                for (x, y) in stroke(&points, radius)? {
                    if let Some((x, y)) = self.world.map(x, y) {
                        self.colors.set(x, y, color);
                    }
                }
            }
            AntCommand::Randomize { density, color, seed, region } => {
                check(color)?;
                let region = match region {
                    Some(r) => {
                        self.world.clip(&r).ok_or_else(|| CaError::invalid("the region lies outside the world"))?
                    }
                    None => self.world.home(),
                };
                // Finite worlds are allocated already; on the infinite plane every
                // touched 64×64 tile is allocated, so the region is limited.
                let limit = if self.world.is_finite() { MAX_FILL_CELLS } else { MAX_EDIT_CELLS };
                check_fill(region, density, Symmetry::None, limit)?;
                let mut own = seed.map(SimRng::seed_from_u64);
                let rng = match own.as_mut() {
                    Some(rng) => rng,
                    None => &mut self.rng,
                };
                let colors = &mut self.colors;
                colors.clear_rect(region);
                random_fill_each(rng, region, density, Symmetry::None, |x, y| colors.set(x, y, color))?;
            }
            AntCommand::ClearColors => self.colors.clear(),
            AntCommand::AddAnt(ant) => {
                if self.ants.len() >= MAX_ANTS {
                    return Err(CaError::LimitExceeded(format!("at most {MAX_ANTS} ants")).into());
                }
                let (x, y) = self
                    .world
                    .map(ant.x, ant.y)
                    .ok_or_else(|| CaError::invalid("the ant lies outside the bounded world"))?;
                self.ants.push(Ant { x, y, heading: ant.heading });
            }
            AntCommand::RemoveAnts { rect } => self.ants.retain(|a| !rect.contains(a.x, a.y)),
        }
        self.after_edit();
        Ok(())
    }

    fn params(&self) -> ParamSet {
        ParamSet::new()
            .with("rule", ParamValue::Text(self.config.rule.clone()))
            .with("topology", ParamValue::Text(crate::life::topology_name(self.config.topology).into()))
            .with("width", ParamValue::Int(i64::from(self.config.width)))
            .with("height", ParamValue::Int(i64::from(self.config.height)))
            .with("steps_per_tick", ParamValue::Int(self.pending_steps.unwrap_or(self.steps_per_tick) as i64))
            .with("track_activity", ParamValue::Bool(self.track_activity))
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let spec = Self::params_specs()
            .into_iter()
            .find(|s| s.key == key)
            .ok_or_else(|| SimError::UnknownParameter(key.into()))?;
        let value = spec.validate(&value)?;
        let invalid = |e: CaError| SimError::invalid_param(key, e.to_string());
        let mut next = self.config.clone();
        match key {
            "rule" => {
                let rule = TurmiteRule::parse(value.as_str().unwrap_or_default()).map_err(invalid)?;
                next.rule = rule.to_string();
            }
            "topology" => {
                next.topology = value
                    .as_str()
                    .and_then(crate::life::parse_topology)
                    .ok_or_else(|| SimError::invalid_param(key, "unknown topology"))?;
            }
            "width" => next.width = value.as_i64().unwrap_or(1) as u32,
            "height" => next.height = value.as_i64().unwrap_or(1) as u32,
            "steps_per_tick" => {
                self.pending_steps = Some(value.as_i64().unwrap_or(1) as u64);
                return Ok(spec.apply);
            }
            "track_activity" => {
                self.track_activity = value.as_bool().unwrap_or(true);
                if !self.track_activity {
                    self.trail.clear();
                }
                return Ok(spec.apply);
            }
            _ => return Err(SimError::UnknownParameter(key.into())),
        }
        // Validate the whole configuration the next reset will build.
        Self::new(&next, self.seed).map_err(invalid)?;
        self.config = next;
        Ok(spec.apply)
    }

    fn metrics(&self) -> Metrics {
        Metrics::new(self.steps, self.steps as f64)
            .with("colored", self.colors.nonzero() as f64)
            .with("ants", self.ants.len() as f64)
            .with("steps", self.steps as f64)
    }

    fn view(&self, request: &GridViewRequest) -> AntView {
        // Entries are at most TRAIL_TICKS - 1 ticks old once a tick has
        // ended (older ones are dropped), so the value stays positive.
        let trail: Vec<(i64, i64, u8)> = if self.track_activity {
            self.trail
                .iter()
                .filter(|&&(_, _, t)| t < self.ticks)
                .map(|&(x, y, t)| {
                    let age = (self.ticks - 1 - t).min(TRAIL_TICKS - 1) as u8;
                    (x, y, 255 - ANT_TRAIL_DECAY * age)
                })
                .collect()
        } else {
            Vec::new()
        };
        let grid = self.colors.view(request, &trail);
        let rect = request.rect();
        let ants = self.ants.iter().copied().filter(|a| rect.contains(a.x, a.y)).collect();
        AntView { grid, ants }
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        let saved = Saved {
            config: self.config.clone(),
            rule: self.rule.to_string(),
            ticks: self.ticks,
            colors: self.colors.clone(),
            ants: self.ants.clone(),
            steps: self.steps,
            steps_per_tick: self.steps_per_tick,
            pending_steps: self.pending_steps,
            rng: self.rng.clone(),
            seed: self.seed,
            trail: self.trail.iter().copied().collect(),
            track_activity: self.track_activity,
            reset_point: self.reset_point.clone(),
        };
        Snapshot::encode(ANT_MODEL_ID, SCHEMA_VERSION, self.steps, &saved)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let saved: Saved = snapshot.decode(ANT_MODEL_ID, SCHEMA_VERSION)?;
        // The running rule is saved separately (the configuration holds the
        // rule of the next reset); the running world is the colour grid's.
        let rule = TurmiteRule::parse(&saved.rule)?;
        TurmiteRule::parse(&saved.config.rule)?;
        let world = saved.colors.world();
        let checked = World::new(world.topology, world.width, world.height)?;
        if checked != world || matches!(world.topology, Topology::KleinBottle | Topology::CrossSurface) {
            return Err(SimError::Serialization("snapshot world is not an ant world".into()));
        }
        saved.colors.validate(rule.colors())?;
        let inside = |ants: &[Ant]| ants.iter().all(|a| world.map(a.x, a.y) == Some((a.x, a.y)));
        if let Some((colors, ants, _)) = &saved.reset_point {
            colors.validate(rule.colors())?;
            if colors.world() != world || !inside(ants) {
                return Err(SimError::Serialization("snapshot reset point does not match the world".into()));
            }
        }
        if !inside(&saved.ants) || saved.ants.len() > MAX_ANTS {
            return Err(SimError::Serialization("snapshot ants lie outside the world".into()));
        }
        self.config = saved.config;
        self.rule = rule;
        self.world = world;
        self.colors = saved.colors;
        self.ants = saved.ants;
        self.steps = saved.steps;
        self.steps_per_tick = saved.steps_per_tick;
        self.pending_steps = saved.pending_steps;
        self.rng = saved.rng;
        self.seed = saved.seed;
        self.ticks = saved.ticks;
        self.trail = saved.trail.into_iter().collect();
        self.track_activity = saved.track_activity;
        self.reset_point = saved.reset_point;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.colors.memory_bytes() + self.ants.capacity() * std::mem::size_of::<Ant>() + self.trail.len() * 24
    }

    fn is_finished(&self) -> bool {
        self.ants.is_empty()
    }
}
