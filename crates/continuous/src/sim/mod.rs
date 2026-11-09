//! [`ContinuousSim`]: Lenia, SmoothLife and Flow-Lenia behind the shared
//! [`simulation_core::Simulation`] contract (model ids `continuous.lenia`,
//! `continuous.smoothlife`, `continuous.flow_lenia`).

mod describe;
pub mod params;
pub mod view;

use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, Grid2, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamValue, SimError, SimRng,
    Simulation, Snapshot,
};

use crate::convolve::{ConvolutionMethod, ConvolutionPlan, ResolvedMethod};
use crate::edit::{self, Brush, CreatureCells, Placement, Region};
use crate::error::ContinuousError;
use crate::flow::FlowEngine;
use crate::lenia::{GrowthNormalization, LeniaEngine};
use crate::rule::{ModelKind, Rule};
use crate::scenario::{Scenario, validate_size};
use crate::smoothlife::SmoothLifeEngine;
use crate::stats::{ACTIVE_THRESHOLD, CentroidTracker, field_sum, torus_centroid};

use params::Change;
use view::{ContinuousView, ContinuousViewRequest, ViewLayer};

/// A labelled plane borrowed from the world or the engine.
type Plane<'a> = (String, &'a [f32]);

/// Snapshot payload version written by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// User interventions accepted by [`ContinuousSim::apply`](Simulation::apply).
#[derive(Clone, Debug, PartialEq)]
pub enum ContinuousCommand {
    /// A soft brush dab (paint or erase).
    Brush(Brush),
    /// Places creature cells at a position with rotation and scale.
    PlaceCreature {
        /// Cells to place (e.g. from [`crate::scenario::creature_library`]).
        cells: CreatureCells,
        /// Where and how.
        placement: Placement,
    },
    /// Replaces a region with random matter (see [`edit::randomize`]).
    Randomize {
        /// Region in cells.
        region: Region,
        /// Probability that a cell receives matter, in `[0, 1]`.
        density: f32,
        /// Values are uniform in `[0, amplitude)` (clamped to 1 for Lenia
        /// and SmoothLife).
        amplitude: f32,
    },
    /// Sets every cell of every channel to zero.
    Clear,
}

/// Derived per-model buffers (kernels, spectra, scratch); rebuilt from the
/// rule, never serialised.
#[derive(Debug)]
enum Engine {
    Lenia(LeniaEngine),
    SmoothLife(SmoothLifeEngine),
    Flow(FlowEngine),
}

impl Engine {
    fn build(rule: &Rule, width: usize, height: usize, method: ConvolutionMethod) -> Result<Self, ContinuousError> {
        Ok(match rule {
            Rule::Lenia(r) => Self::Lenia(LeniaEngine::new(r, width, height, method)?),
            Rule::SmoothLife(r) => Self::SmoothLife(SmoothLifeEngine::new(r, width, height, method)?),
            Rule::FlowLenia(r) => Self::Flow(FlowEngine::new(r, width, height, method)?),
        })
    }

    fn plan(&self) -> &ConvolutionPlan {
        match self {
            Self::Lenia(e) => e.plan(),
            Self::SmoothLife(e) => e.plan(),
            Self::Flow(e) => e.plan(),
        }
    }
}

/// Serialised state (schema version 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct State {
    scenario: Scenario,
    rule: Rule,
    width: usize,
    height: usize,
    next_size: (usize, usize),
    seed: u64,
    method: ConvolutionMethod,
    rng: SimRng,
    tick: u64,
    /// Simulated time: the sum of `dt` over every executed tick (the time
    /// step can change between ticks).
    sim_time: f64,
    fields: Vec<Grid2<f32>>,
    tracker: CentroidTracker,
    mean_growth: f64,
    reference_mass: f64,
}

/// A continuous world: one of the three models, its fields, random
/// generator and the scenario it was built from.
#[derive(Debug)]
pub struct ContinuousSim {
    state: State,
    engine: Engine,
}

impl ContinuousSim {
    /// Builds the world described by `scenario` with the scenario's seed.
    pub fn from_scenario(scenario: Scenario) -> Result<Self, ContinuousError> {
        let seed = scenario.header.seed;
        Self::with_seed(scenario, seed)
    }

    /// Builds the world described by `scenario` with an explicit seed.
    pub fn with_seed(scenario: Scenario, seed: u64) -> Result<Self, ContinuousError> {
        scenario.validate()?;
        let (width, height) = (scenario.width, scenario.height);
        let rule = scenario.rule.clone();
        let method = ConvolutionMethod::Auto;
        let engine = Engine::build(&rule, width, height, method)?;
        let mut rng = SimRng::seed_from_u64(seed);
        let fields = scenario.initial_fields(width, height, &mut rng)?;
        let mut sim = Self {
            state: State {
                scenario,
                rule,
                width,
                height,
                next_size: (width, height),
                seed,
                method,
                rng,
                tick: 0,
                sim_time: 0.0,
                fields,
                tracker: CentroidTracker::default(),
                mean_growth: 0.0,
                reference_mass: 0.0,
            },
            engine,
        };
        sim.after_edit();
        Ok(sim)
    }

    /// Model family of the current rule.
    pub fn kind(&self) -> ModelKind {
        self.state.rule.kind()
    }

    /// Current rule.
    pub fn rule(&self) -> &Rule {
        &self.state.rule
    }

    /// Scenario the world was built from (used by [`Simulation::reset`]).
    pub fn scenario(&self) -> &Scenario {
        &self.state.scenario
    }

    /// World width in cells.
    pub fn width(&self) -> usize {
        self.state.width
    }

    /// World height in cells.
    pub fn height(&self) -> usize {
        self.state.height
    }

    /// Cell values of every channel.
    pub fn fields(&self) -> &[Grid2<f32>] {
        &self.state.fields
    }

    /// Convolution method the engine resolved to.
    pub fn resolved_method(&self) -> ResolvedMethod {
        self.engine.plan().method()
    }

    /// Total mass `Σ A` over all channels.
    pub fn mass(&self) -> f64 {
        self.state.fields.iter().map(|g| field_sum(g.as_slice(), self.state.width)).sum()
    }

    /// Torus centroid of the summed channels (world coordinates), if defined.
    pub fn centroid(&self) -> Option<(f64, f64)> {
        self.state.tracker.centroid()
    }

    /// Unwrapped centroid trajectory position (see [`CentroidTracker`]).
    pub fn centroid_track(&self) -> (f64, f64) {
        self.state.tracker.unwrapped()
    }

    /// Replaces the rule with one of the same model and channel count,
    /// validating that every kernel fits the current and pending world size.
    pub fn set_rule(&mut self, rule: Rule) -> Result<(), ContinuousError> {
        if rule.kind() != self.kind() || rule.channels() != self.state.rule.channels() {
            return Err(ContinuousError::rule(
                "a rule can only be replaced by one of the same model and channel count",
            ));
        }
        let engine = Engine::build(&rule, self.state.width, self.state.height, self.state.method)?;
        let (nw, nh) = self.state.next_size;
        if (nw, nh) != (self.state.width, self.state.height) {
            check_fits(&rule, nw, nh, self.state.method)?;
        }
        self.state.rule = rule;
        self.engine = engine;
        Ok(())
    }

    /// Clears every channel (the generic "Clear" control).
    pub fn clear(&mut self) {
        for g in &mut self.state.fields {
            g.fill(0.0);
        }
        self.after_edit();
    }

    /// Replaces the world with a model-specific random soup drawn from
    /// `seed` (the generic "Randomize" control): Lenia and Flow-Lenia get a
    /// centred disc of random matter with radius a quarter of the smaller
    /// side and density ½; SmoothLife gets `W·H/(2rₐ²)` filled discs with
    /// radii in `[rᵢ, rₐ]` (Rafler's glider-forming soup). Soups are chaotic:
    /// with the paper's glider rule about half of all seeds condense into
    /// gliders and the others die out within a few hundred ticks.
    pub fn randomize(&mut self, seed: u64) {
        self.state.rng = SimRng::seed_from_u64(seed);
        let (w, h) = (self.state.width, self.state.height);
        for g in &mut self.state.fields {
            g.fill(0.0);
        }
        let result = match &self.state.rule {
            Rule::SmoothLife(r) => {
                let count = ((w * h) as f64 / (2.0 * r.outer_radius * r.outer_radius)).round().max(1.0) as u32;
                edit::random_discs(
                    &mut self.state.fields,
                    count.min(edit::MAX_DISCS),
                    r.inner_radius() as f32,
                    r.outer_radius as f32,
                    1.0,
                    &mut self.state.rng,
                )
            }
            _ => {
                let region = Region::Disc { x: w as f32 / 2.0, y: h as f32 / 2.0, radius: w.min(h) as f32 / 4.0 };
                edit::randomize(&mut self.state.fields, &region, 0.5, 1.0, &mut self.state.rng)
            }
        };
        // The arguments above are always valid; an error would leave the world cleared.
        debug_assert!(result.is_ok(), "{result:?}");
        self.after_edit();
    }

    /// Checks that a world of the given size can be built at the next reset:
    /// every kernel fits, the convolution stays within the resource limits
    /// and the scenario's initial elements fit the world.
    fn check_pending_size(&self, width: usize, height: usize) -> Result<(), ContinuousError> {
        check_fits(&self.state.rule, width, height, self.state.method)?;
        let mut rng = SimRng::seed_from_u64(self.state.seed);
        self.state.scenario.initial_fields(width, height, &mut rng).map(drop)
    }

    /// Re-anchors centroid tracking and the Flow-Lenia mass reference after
    /// an edit that changes the field discontinuously.
    fn after_edit(&mut self) {
        let c = self.compute_centroid();
        self.state.tracker.restart(c);
        self.state.reference_mass = self.mass();
    }

    fn compute_centroid(&self) -> Option<(f64, f64)> {
        let slices: Vec<&[f32]> = self.state.fields.iter().map(Grid2::as_slice).collect();
        torus_centroid(&slices, self.state.width, self.state.height)
    }

    fn metric_specs(&self) -> Vec<MetricSpec> {
        let mut specs = vec![
            MetricSpec::gauge("mass", "Total mass", "cells", "Sum of all cell values over all channels."),
            MetricSpec::ratio(
                "active_fraction",
                "Active fraction",
                "Fraction of cells whose channel sum exceeds 0.01.",
            ),
            MetricSpec::gauge(
                "centroid_x",
                "Centroid x",
                "cells",
                "Mass-weighted circular mean column on the torus (0 when undefined: empty or uniform world).",
            ),
            MetricSpec::gauge(
                "centroid_y",
                "Centroid y",
                "cells",
                "Mass-weighted circular mean row on the torus (0 when undefined: empty or uniform world).",
            ),
            MetricSpec::gauge(
                "speed",
                "Centroid speed",
                "cells/tick",
                "Length of the shortest wrapped centroid displacement during the last tick.",
            ),
            MetricSpec::gauge(
                "mean_growth",
                "Mean growth",
                "",
                match self.kind() {
                    ModelKind::Lenia => {
                        "Mean over cells and channels of the growth D added (times dt) in the last tick."
                    }
                    ModelKind::SmoothLife => "Mean of 2·s(n, m) − 1 in the last tick.",
                    ModelKind::FlowLenia => "Mean affinity U over cells and channels in the last tick.",
                },
            ),
        ];
        if self.kind() == ModelKind::FlowLenia {
            specs.push(MetricSpec::gauge(
                "mass_drift",
                "Relative mass drift",
                "",
                "(mass − reference) / reference, where the reference is the mass after the last reset or edit; \
                 Flow-Lenia conserves mass, so this measures floating-point error.",
            ));
        }
        specs
    }

    /// Largest `|D_j|` (Lenia) or `|U_j|` (Flow-Lenia) the growth layer can
    /// reach: the largest per-channel weight sum `Σ h_k` for summed growths,
    /// 1 for weighted means (1 when no weight is positive).
    fn growth_bound(&self) -> f32 {
        let sum_bound = |channels: usize, weights: &mut dyn Iterator<Item = (usize, f64)>| {
            let mut sums = vec![0.0f64; channels];
            for (dst, h) in weights {
                if let Some(s) = sums.get_mut(dst) {
                    *s += h;
                }
            }
            let max = sums.into_iter().fold(0.0, f64::max);
            if max > 0.0 { max as f32 } else { 1.0 }
        };
        match &self.state.rule {
            Rule::Lenia(r) => match r.normalization {
                GrowthNormalization::Sum => {
                    sum_bound(r.channels, &mut r.kernels.iter().map(|k| (k.destination, k.weight)))
                }
                GrowthNormalization::PerDestination => 1.0,
            },
            Rule::SmoothLife(_) => 1.0,
            Rule::FlowLenia(r) => sum_bound(r.channels, &mut r.kernels.iter().map(|k| (k.destination, k.weight))),
        }
    }

    fn view_planes(&self, layer: ViewLayer) -> (Vec<Plane<'_>>, (f32, f32)) {
        let state_planes = || {
            self.state
                .fields
                .iter()
                .enumerate()
                .map(|(i, g)| (format!("channel {i}"), g.as_slice()))
                .collect::<Vec<_>>()
        };
        let state_range = match self.state.rule.max_value() {
            Some(m) => (0.0, m),
            None => {
                let max = self.state.fields.iter().flat_map(|g| g.as_slice().iter().copied()).fold(0.0f32, f32::max);
                (0.0, max.max(1.0))
            }
        };
        let g = self.growth_bound();
        match (layer, &self.engine) {
            (ViewLayer::State, _) => (state_planes(), state_range),
            (ViewLayer::Growth, Engine::Lenia(e)) => {
                (e.growth().iter().enumerate().map(|(i, g)| (format!("growth {i}"), g.as_slice())).collect(), (-g, g))
            }
            (ViewLayer::Growth, Engine::SmoothLife(e)) => (vec![("2s − 1".to_string(), e.growth())], (-1.0, 1.0)),
            (ViewLayer::Growth, Engine::Flow(e)) => (
                e.affinity().iter().enumerate().map(|(i, g)| (format!("affinity {i}"), g.as_slice())).collect(),
                (-g, g),
            ),
            (ViewLayer::Potential, Engine::Lenia(e)) => {
                (vec![("potential (kernel 0)".to_string(), e.potential())], (0.0, 1.0))
            }
            (ViewLayer::Potential, Engine::SmoothLife(e)) => (
                vec![
                    ("inner filling m".to_string(), e.inner_filling()),
                    ("outer filling n".to_string(), e.outer_filling()),
                ],
                (0.0, 1.0),
            ),
            (ViewLayer::Potential, Engine::Flow(e)) => {
                (vec![("potential (kernel 0)".to_string(), e.potential())], (0.0, state_range.1))
            }
        }
    }
}

/// Checks that every kernel of `rule` fits a `width × height` torus and
/// that its convolution plan stays within the resource limits.
fn check_fits(rule: &Rule, width: usize, height: usize, method: ConvolutionMethod) -> Result<(), ContinuousError> {
    validate_size(width, height)?;
    let kernels = match rule {
        Rule::Lenia(r) => r.kernel_images(width, height)?,
        Rule::SmoothLife(r) => r.kernel_images(width, height)?,
        Rule::FlowLenia(r) => r.kernel_images(width, height)?,
    };
    ConvolutionPlan::check(width, height, rule.channels(), &kernels, method).map(drop)
}

impl Simulation for ContinuousSim {
    type Command = ContinuousCommand;
    type ViewRequest = ContinuousViewRequest;
    type View = ContinuousView;

    fn descriptor(&self) -> ModelDescriptor {
        let kind = self.kind();
        let (summary, unit) = match kind {
            ModelKind::Lenia => (
                "Continuous cellular automaton with smooth kernels and growth functions (Chan 2019), including \
                 multi-channel, multi-kernel Expanded Lenia (Chan 2020).",
                "time unit (T ticks)",
            ),
            ModelKind::SmoothLife => {
                ("Rafler's continuous generalisation of Conway's Game of Life with disc and annulus fillings.", "step")
            }
            ModelKind::FlowLenia => (
                "Mass-conservative Lenia: matter flows along affinity gradients and is redistributed by \
                 reintegration tracking (Plantec et al. 2023).",
                "time unit",
            ),
        };
        ModelDescriptor::new(kind.model_id(), kind.name(), ModelFamily::Continuous)
            .summary(summary)
            .rules(&describe::rules_markdown(&self.state.rule))
            .params(params::specs_with_defaults(&self.state.rule, &self.state.scenario))
            .metrics(self.metric_specs())
            .time(unit, self.state.rule.dt())
    }

    fn tick(&self) -> u64 {
        self.state.tick
    }

    fn step(&mut self) {
        let fields = &mut self.state.fields;
        self.state.mean_growth = match (&mut self.engine, &self.state.rule) {
            (Engine::Lenia(e), Rule::Lenia(r)) => e.step(r, fields),
            (Engine::SmoothLife(e), Rule::SmoothLife(r)) => match fields.first_mut() {
                Some(f) => e.step(r, f),
                None => 0.0,
            },
            (Engine::Flow(e), Rule::FlowLenia(r)) => e.step(r, fields),
            // The engine is always rebuilt together with the rule.
            _ => self.state.mean_growth,
        };
        self.state.tick += 1;
        self.state.sim_time += self.state.rule.dt();
        let c = self.compute_centroid();
        self.state.tracker.update(c, self.state.width, self.state.height);
    }

    fn reset(&mut self) {
        let (w, h) = self.state.next_size;
        let mut rng = SimRng::seed_from_u64(self.state.seed);
        let built = Engine::build(&self.state.rule, w, h, self.state.method);
        let fields = self.state.scenario.initial_fields(w, h, &mut rng);
        let (Ok(engine), Ok(fields)) = (built, fields) else {
            // Not reachable through the public API: the pending size is
            // checked against the rule whenever either changes. Keep the
            // current world rather than leaving it half-reset.
            self.state.next_size = (self.state.width, self.state.height);
            return;
        };
        self.engine = engine;
        self.state.fields = fields;
        self.state.width = w;
        self.state.height = h;
        self.state.rng = rng;
        self.state.tick = 0;
        self.state.sim_time = 0.0;
        self.state.mean_growth = 0.0;
        self.state.tracker = CentroidTracker::default();
        self.after_edit();
    }

    fn apply(&mut self, command: ContinuousCommand) -> Result<(), SimError> {
        let limit = self.state.rule.edit_limit();
        match command {
            ContinuousCommand::Brush(brush) => {
                edit::apply_brush(&mut self.state.fields, &brush, limit)?;
            }
            ContinuousCommand::PlaceCreature { cells, placement } => {
                edit::place(&mut self.state.fields, &cells, &placement, Some(limit))?;
            }
            ContinuousCommand::Randomize { region, density, amplitude } => {
                // NaN is rejected by `randomize`; larger amplitudes are capped.
                let amplitude = if amplitude.is_nan() { amplitude } else { amplitude.min(limit) };
                edit::randomize(&mut self.state.fields, &region, density, amplitude, &mut self.state.rng)?;
            }
            ContinuousCommand::Clear => {
                for g in &mut self.state.fields {
                    g.fill(0.0);
                }
            }
        }
        self.after_edit();
        Ok(())
    }

    fn params(&self) -> ParamSet {
        let s = &self.state;
        let mut set = params::values(&s.rule, s.width, s.height, s.seed, s.method);
        // Pending (reset-time) values are what the user last chose.
        set.set("width", ParamValue::Int(s.next_size.0 as i64));
        set.set("height", ParamValue::Int(s.next_size.1 as i64));
        set
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let invalid = |e: ContinuousError| SimError::invalid_param(key, e.to_string());
        match params::change(&self.state.rule, key, &value)? {
            Change::Width(w) => {
                let size = (w, self.state.next_size.1);
                self.check_pending_size(size.0, size.1).map_err(invalid)?;
                self.state.next_size = size;
                Ok(ApplyPolicy::RequiresReset)
            }
            Change::Height(h) => {
                let size = (self.state.next_size.0, h);
                self.check_pending_size(size.0, size.1).map_err(invalid)?;
                self.state.next_size = size;
                Ok(ApplyPolicy::RequiresReset)
            }
            Change::Seed(seed) => {
                self.state.seed = seed;
                Ok(ApplyPolicy::RequiresReset)
            }
            Change::Method(method) => {
                let (nw, nh) = self.state.next_size;
                if (nw, nh) != (self.state.width, self.state.height) {
                    check_fits(&self.state.rule, nw, nh, method).map_err(invalid)?;
                }
                self.engine =
                    Engine::build(&self.state.rule, self.state.width, self.state.height, method).map_err(invalid)?;
                self.state.method = method;
                Ok(ApplyPolicy::NextTick)
            }
            Change::Rule(rule) => {
                self.set_rule(rule).map_err(invalid)?;
                Ok(ApplyPolicy::NextTick)
            }
        }
    }

    fn metrics(&self) -> Metrics {
        let s = &self.state;
        let n = (s.width * s.height) as f64;
        let mut active = 0usize;
        for i in 0..s.width * s.height {
            let v: f32 = s.fields.iter().map(|g| g.as_slice()[i]).sum();
            active += usize::from(v > ACTIVE_THRESHOLD);
        }
        let mass = self.mass();
        let (cx, cy) = s.tracker.centroid().unwrap_or((0.0, 0.0));
        let mut m = Metrics::new(s.tick, s.sim_time)
            .with("mass", mass)
            .with("active_fraction", active as f64 / n)
            .with("centroid_x", cx)
            .with("centroid_y", cy)
            .with("speed", s.tracker.speed())
            .with("mean_growth", s.mean_growth);
        if self.kind() == ModelKind::FlowLenia {
            let drift = if s.reference_mass > 0.0 { (mass - s.reference_mass) / s.reference_mass } else { 0.0 };
            m.push("mass_drift", drift);
        }
        m
    }

    fn view(&self, request: &ContinuousViewRequest) -> ContinuousView {
        let (planes, range) = self.view_planes(request.layer);
        view::build(request, self.state.tick, &self.state.fields, planes, range)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(self.kind().model_id(), SCHEMA_VERSION, self.state.tick, &self.state)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let state: State = snapshot.decode(self.kind().model_id(), SCHEMA_VERSION)?;
        let corrupt = |m: &str| SimError::Serialization(format!("snapshot is inconsistent: {m}"));
        validate_size(state.width, state.height).map_err(SimError::from)?;
        if state.fields.len() != state.rule.channels()
            || state.fields.iter().any(|g| g.width() != state.width || g.height() != state.height)
        {
            return Err(corrupt("field sizes do not match the world"));
        }
        if state.rule.kind() != self.kind() {
            return Err(corrupt("rule model differs from the snapshot model"));
        }
        state.scenario.validate().map_err(SimError::from)?;
        // `reset` rebuilds the fields from the scenario and the engine from
        // the rule: both must describe the same model and channels, and the
        // scenario must fit the pending size.
        if state.scenario.rule.kind() != state.rule.kind() || state.scenario.rule.channels() != state.rule.channels() {
            return Err(corrupt("the scenario's rule and the current rule have different models or channels"));
        }
        check_fits(&state.rule, state.next_size.0, state.next_size.1, state.method).map_err(SimError::from)?;
        let mut rng = SimRng::seed_from_u64(state.seed);
        state.scenario.initial_fields(state.next_size.0, state.next_size.1, &mut rng).map_err(SimError::from)?;
        let engine = Engine::build(&state.rule, state.width, state.height, state.method).map_err(SimError::from)?;
        self.state = state;
        self.engine = engine;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        let cells = self.state.width * self.state.height;
        let channels = self.state.fields.len();
        // Per-cell buffers in 4-byte units: Lenia fields, growths and the
        // potential; SmoothLife field, fillings and transition; Flow-Lenia
        // fields, affinities, potential, total mass, three 2-D vectors and
        // the f64 accumulator. Plus the convolution plan.
        let per_cell = match &self.engine {
            Engine::Lenia(_) => 2 * channels + 1,
            Engine::SmoothLife(_) => 4,
            Engine::Flow(_) => 2 * channels + 10,
        };
        per_cell * cells * 4 + self.engine.plan().memory_bytes()
    }
}
