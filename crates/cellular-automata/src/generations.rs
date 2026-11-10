//! `GenerationsSim`: multi-state "Generations" rules (Brian's Brain, Star
//! Wars and the rest of Golly's Generations family).

use patterns::{GenerationsRule, Rule};
use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamSpec, ParamValue, SimError,
    Simulation, Snapshot, Topology,
};

use crate::engine::Lut;
use crate::error::CaError;
use crate::geometry::World;
use crate::life::PatternLibrary;
use crate::multistate::{PaintCommand, StateInit};
use crate::statemodel::{SavedCore, StateCore};
use crate::view::{GridView, GridViewRequest};

/// Model id.
pub const GENERATIONS_MODEL_ID: &str = "ca.generations";
const SCHEMA_VERSION: u32 = 1;

/// Machine name of a named Generations rule ("Brian's Brain" → `brians_brain`).
fn slug(name: &str) -> String {
    name.to_lowercase().replace('\'', "").replace([' ', '-', '&'], "_")
}

/// Configuration of a [`GenerationsSim`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationsConfig {
    /// Rule: a name (`Brian's Brain`, `Star Wars`...) or any notation
    /// `GenerationsRule::parse` accepts (`/2/3`, `B2/S/C3`, `345/2/4`, `g3b2s`).
    pub rule: String,
    /// Finite topology.
    #[serde(default = "torus")]
    pub topology: Topology,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Activity trails.
    #[serde(default)]
    pub track_activity: bool,
    /// Initial content.
    #[serde(default)]
    pub init: Vec<StateInit>,
}

fn torus() -> Topology {
    Topology::Torus
}

impl Default for GenerationsConfig {
    fn default() -> Self {
        Self {
            rule: "Brian's Brain".into(),
            topology: Topology::Torus,
            width: 256,
            height: 256,
            track_activity: false,
            init: vec![StateInit::Random { region: None, density: 0.2, state: 1, symmetry: crate::Symmetry::None }],
        }
    }
}

/// Parses a Generations rule given by name or notation (two-state rules are
/// accepted as Generations rules with C = 2).
pub fn parse_generations_rule(text: &str) -> Result<GenerationsRule, CaError> {
    if let Some(rule) = GenerationsRule::from_name(text) {
        return Ok(rule);
    }
    match Rule::parse(text)? {
        Rule::Generations(rule) => Ok(rule),
        Rule::Life(life) => Ok(GenerationsRule::new(life.birth, life.survive, 2)?),
    }
}

/// Golly's Generations semantics as a [`Simulation`].
///
/// States: 0 dead, 1 alive ("firing"), 2..C−1 dying. Only state-1 cells count
/// as neighbours (Moore neighbourhood). With *n* live neighbours a dead cell
/// becomes 1 iff *n* ∈ B; a live cell stays 1 iff *n* ∈ S, otherwise starts
/// dying (state 2, or 0 when C = 2); a dying cell in state *k* moves to
/// *k* + 1 and wraps to 0 at C. Dying cells can neither be re-born nor
/// re-stimulated. A rule change applies at the next tick; cells in states
/// that no longer exist decay to 0 in one generation.
pub struct GenerationsSim {
    core: StateCore,
    rule: GenerationsRule,
    pending_rule: Option<GenerationsRule>,
    lut: Lut,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    core: SavedCore,
    rule: (u16, u16, u8),
    pending_rule: Option<(u16, u16, u8)>,
}

impl GenerationsSim {
    /// Builds the model.
    ///
    /// # Errors
    /// Invalid rule, world, patterns or states.
    pub fn new(config: &GenerationsConfig, seed: u64, library: &PatternLibrary) -> Result<Self, CaError> {
        let rule = parse_generations_rule(&config.rule)?;
        let world = World::new(config.topology, config.width, config.height)?;
        let core = StateCore::new(world, &config.init, library, seed, u16::from(rule.states), config.track_activity)?;
        Ok(Self { core, rule, pending_rule: None, lut: Self::lut(rule) })
    }

    /// Brian's Brain on a 256×256 torus with a 20 % soup of firing cells.
    pub fn brians_brain(seed: u64) -> Self {
        match Self::new(&GenerationsConfig::default(), seed, &PatternLibrary::builtin()) {
            Ok(sim) => sim,
            Err(e) => unreachable!("the default Generations configuration is valid: {e}"),
        }
    }

    fn lut(rule: GenerationsRule) -> Lut {
        Lut::from_fn(u16::from(rule.states), |s, n| rule.next_state(s, n))
    }

    /// The rule being run.
    pub fn rule(&self) -> GenerationsRule {
        self.rule
    }

    /// State of a cell.
    pub fn cell(&self, x: i64, y: i64) -> u8 {
        self.core.grid.get(x, y)
    }

    /// Non-zero cells `(x, y, state)`, row-major.
    pub fn cells(&self) -> Vec<(i64, i64, u8)> {
        self.core.grid.cells()
    }

    /// Activity of a cell: 255 when it changed in the last generation, then
    /// 16 less per generation (0 when tracking is off).
    pub fn activity(&self, x: i64, y: i64) -> u8 {
        self.core.grid.activity(x, y)
    }

    /// The world.
    pub fn world(&self) -> World {
        self.core.world()
    }

    /// Why the model stopped, if it did (a reset that could not be carried
    /// out; the state is then unchanged).
    pub fn halt_reason(&self) -> Option<&str> {
        self.core.halted.as_deref()
    }

    fn states(&self) -> u16 {
        // Painting must respect the larger of the running and pending rules.
        u16::from(self.pending_rule.map_or(self.rule.states, |p| p.states.max(self.rule.states)))
    }

    fn presets() -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = GenerationsRule::known_rules()
            .map(|(name, rule)| (slug(name), format!("{name} ({})", rule.to_bsc_string())))
            .collect();
        v.push(("custom".into(), "Custom rule".into()));
        v
    }
}

impl Simulation for GenerationsSim {
    type Command = PaintCommand;
    type ViewRequest = GridViewRequest;
    type View = GridView;

    fn descriptor(&self) -> ModelDescriptor {
        let presets = Self::presets();
        let options: Vec<(&str, &str)> = presets.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let mut params = vec![
            ParamSpec::choice("preset", "Rule preset", &options, "brians_brain")
                .group("Rules")
                .apply(ApplyPolicy::NextTick)
                .describe("Named Generations rules from Golly's collection."),
            ParamSpec::text("rule", "Rule", 40, "/2/3").group("Rules").apply(ApplyPolicy::NextTick).describe(
                "Survival/birth/states in Golly's S/B/C form (/2/3), the LifeWiki form B2/S/C3, or a rule name.",
            ),
        ];
        params.extend(StateCore::world_specs("torus", 256));
        let r = self.rule;
        let digits = |m: u16| (0..=8).filter(|n| m >> n & 1 == 1).map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
        let rules = format!(
            "## Generations rule {canon} (B{b}/S{s}/C{c})\n\n\
             States: 0 = dead, 1 = alive (firing), 2..{last} = dying. Only cells in state 1 count as neighbours \
             (Moore neighbourhood, 8 cells; positions outside a finite world map through its topology). With *n* \
             live neighbours:\n\n\
             * a dead cell becomes alive iff *n* is in B = {{{bl}}};\n\
             * a live cell stays alive iff *n* is in S = {{{sl}}}, otherwise it starts dying (state 2, or dead when C = 2);\n\
             * a dying cell in state *k* moves to state *k* + 1, and to 0 after state C − 1 = {last}.\n\n\
             Dying cells can be neither re-born nor re-stimulated. This is Golly's `generationsalgo`.",
            canon = r,
            b = digits(r.birth).replace(", ", ""),
            s = digits(r.survive).replace(", ", ""),
            c = r.states,
            last = r.states.saturating_sub(1),
            bl = digits(r.birth),
            sl = digits(r.survive),
        );
        ModelDescriptor::new(GENERATIONS_MODEL_ID, "Generations rules", ModelFamily::Cellular)
            .summary("Multi-state Life-like rules with decaying 'dying' states: Brian's Brain, Star Wars and more.")
            .rules(&rules)
            .params(params)
            .metrics(vec![
                MetricSpec::gauge("alive", "Alive", "cells", "Cells in state 1."),
                MetricSpec::gauge("dying", "Dying", "cells", "Cells in states 2 and above."),
                MetricSpec::counter("births", "Births", "Cells that became alive in the last generation."),
                MetricSpec::counter(
                    "deaths",
                    "Deaths",
                    "Cells that left the alive state (started dying) in the last generation.",
                ),
                MetricSpec::ratio("density", "Occupied fraction", "Non-zero cells divided by the world area."),
                MetricSpec::gauge("generation", "Generation", "generations", "Generations since the last reset."),
            ])
    }

    fn tick(&self) -> u64 {
        self.core.generation
    }

    fn step(&mut self) {
        if let Some(rule) = self.pending_rule.take() {
            self.rule = rule;
            self.lut = Self::lut(rule);
        }
        self.core.step(&self.lut);
    }

    fn reset(&mut self) {
        if let Some(rule) = self.pending_rule.take() {
            self.rule = rule;
            self.lut = Self::lut(rule);
        }
        self.core.reset();
    }

    fn apply(&mut self, command: PaintCommand) -> Result<(), SimError> {
        let states = self.states();
        Ok(self.core.apply(command, states)?)
    }

    fn params(&self) -> ParamSet {
        let rule = self.pending_rule.unwrap_or(self.rule);
        let preset = rule.name().map_or_else(|| "custom".to_string(), slug);
        let mut set =
            ParamSet::new().with("preset", ParamValue::Text(preset)).with("rule", ParamValue::Text(rule.to_string()));
        for (k, v) in self.core.world_values() {
            set.set(k, v);
        }
        set
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let spec = self.descriptor().param(key).cloned().ok_or_else(|| SimError::UnknownParameter(key.into()))?;
        let value = spec.validate(&value)?;
        match key {
            "preset" => {
                let name = value.as_str().unwrap_or_default();
                if name != "custom" {
                    let rule = GenerationsRule::known_rules()
                        .find(|(n, _)| slug(n) == name)
                        .map(|(_, r)| r)
                        .ok_or_else(|| SimError::invalid_param(key, "unknown preset"))?;
                    self.pending_rule = Some(rule);
                }
            }
            "rule" => {
                let rule = parse_generations_rule(value.as_str().unwrap_or_default())
                    .map_err(|e| SimError::invalid_param(key, e.to_string()))?;
                self.pending_rule = Some(rule);
            }
            other => match self.core.set_world_param(other, &value) {
                Some(result) => result?,
                None => return Err(SimError::UnknownParameter(key.into())),
            },
        }
        Ok(spec.apply)
    }

    fn metrics(&self) -> Metrics {
        let h = self.core.grid.histogram();
        let total = self.core.world().cell_count() as f64;
        let alive = h[1];
        let nonzero: u64 = h[1..].iter().sum();
        let mut m = Metrics::new(self.core.generation, self.core.generation as f64)
            .with("alive", alive as f64)
            .with("dying", (nonzero - alive) as f64)
            .with("density", nonzero as f64 / total)
            .with("generation", self.core.generation as f64);
        if let Some(last) = self.core.last {
            m.push("births", last.entered_one as f64).push("deaths", last.left_one as f64);
        }
        m
    }

    fn view(&self, request: &GridViewRequest) -> GridView {
        self.core.grid.view(request)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        let saved = Saved {
            core: self.core.save(),
            rule: (self.rule.birth, self.rule.survive, self.rule.states),
            pending_rule: self.pending_rule.map(|r| (r.birth, r.survive, r.states)),
        };
        Snapshot::encode(GENERATIONS_MODEL_ID, SCHEMA_VERSION, self.core.generation, &saved)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let saved: Saved = snapshot.decode(GENERATIONS_MODEL_ID, SCHEMA_VERSION)?;
        let to_rule = |(b, s, c): (u16, u16, u8)| GenerationsRule::new(b, s, c).map_err(CaError::from);
        let rule = to_rule(saved.rule)?;
        let pending = saved.pending_rule.map(to_rule).transpose()?;
        let states = u16::from(pending.map_or(rule.states, |p| p.states.max(rule.states)));
        self.core = StateCore::load_saved(saved.core, states)?;
        self.rule = rule;
        self.pending_rule = pending;
        self.lut = Self::lut(rule);
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.core.grid.memory_bytes()
    }

    fn is_finished(&self) -> bool {
        self.core.halted.is_some()
    }
}
