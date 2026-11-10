//! [`LabSim`]: the five models of the laboratory behind one
//! [`Simulation`], so a single runner (and a single `ActiveSession`) serves
//! every family and switching scenarios is a `Control::Replace`.
//!
//! * Commands, parameters, metrics and the rule text are the model's own;
//!   the descriptor keeps the model's name, rules, parameters and metrics
//!   but carries the stable id [`LAB_MODEL_ID`], so saves made with any
//!   family load in the laboratory.
//! * Snapshots wrap the model's own snapshot ([`LabSaved`]); restoring one
//!   of another family switches the model (undo across a loaded save, and
//!   loading a save of another family, both work).
//! * The view adds to the model's grid view what the user interface shows
//!   about the running model ([`LabInfo`]), read on the simulation thread
//!   when the view is published.

use cellular_automata::ant::{Ant, AntCommand, AntConfig, AntView, LangtonsAntSim, TurmiteRule};
use cellular_automata::scenario::{BuiltSimulation, Scenario};
use cellular_automata::{
    CellRect, EngineKind, GenerationsConfig, GenerationsSim, GridView, GridViewRequest, LifeCommand, LifeConfig,
    LifeMode, LifeSim, PaintCommand, PatternLibrary, RulePreset, Symmetry, WireworldConfig, WireworldSim,
    WolframCommand, WolframConfig, WolframSim, World,
};
use patterns::{GenerationsRule, LifeRule};
use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, Metrics, ModelDescriptor, ParamSet, ParamValue, SimError, Simulation, Snapshot, Topology,
};

use crate::family::Family;

/// Model id of the laboratory's snapshots and served descriptors.
pub const LAB_MODEL_ID: &str = "ca.lab";
const LAB_SCHEMA: u32 = 1;

/// The running model.
pub enum LabModel {
    /// Life-like rules.
    Life(LifeSim),
    /// Generations rules.
    Generations(GenerationsSim),
    /// Wireworld.
    Wireworld(WireworldSim),
    /// Langton's ant and turmites.
    Ant(LangtonsAntSim),
    /// Elementary cellular automata.
    Wolfram(WolframSim),
}

impl LabModel {
    /// Family of the model.
    pub fn family(&self) -> Family {
        match self {
            LabModel::Life(_) => Family::LifeLike,
            LabModel::Generations(_) => Family::Generations,
            LabModel::Wireworld(_) => Family::Wireworld,
            LabModel::Ant(_) => Family::Ant,
            LabModel::Wolfram(_) => Family::Wolfram,
        }
    }
}

/// Edits of the running model (the variant must match the family).
#[derive(Clone, Debug, PartialEq)]
pub enum LabCommand {
    /// Life-like models.
    Life(LifeCommand),
    /// Generations and Wireworld.
    Paint(PaintCommand),
    /// Ants.
    Ant(AntCommand),
    /// Elementary automata (edits of the current row).
    Wolfram(WolframCommand),
}

/// Settings of the shell's Randomize button (set from the tools panel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FillSettings {
    /// Probability of the scattered state.
    pub density: f64,
    /// State (colour) scattered by Generations, Wireworld and ant fills.
    pub state: u8,
}

impl Default for FillSettings {
    fn default() -> Self {
        Self { density: 0.3, state: 1 }
    }
}

/// What the interface shows about the running model, per family.
#[derive(Clone, Debug, PartialEq)]
pub enum FamilyInfo {
    /// Life-like.
    Life {
        /// Rule being run.
        rule: LifeRule,
        /// Its preset (or Custom).
        preset: RulePreset,
        /// Running world.
        world: World,
        /// Running engine.
        engine: EngineKind,
        /// Generations per tick (pending value included).
        generations_per_tick: u64,
    },
    /// Generations.
    Generations {
        /// Rule being run.
        rule: GenerationsRule,
        /// Running world.
        world: World,
    },
    /// Wireworld.
    Wireworld {
        /// Running world.
        world: World,
    },
    /// Ants.
    Ant {
        /// Rule being run.
        rule: TurmiteRule,
        /// Running world.
        world: World,
        /// Ants left.
        ants: usize,
        /// The first ant (followed by the camera).
        first: Option<Ant>,
    },
    /// Elementary automata.
    Wolfram {
        /// Rule being run.
        rule: u8,
        /// Cells per row.
        width: u32,
        /// Oldest row still kept for the diagram.
        first_row: u64,
        /// Newest row.
        generation: u64,
    },
}

/// Summary of the running model, published with every view.
#[derive(Clone, Debug, PartialEq)]
pub struct LabInfo {
    /// Scenario the model was built from (empty for loaded states of unknown origin).
    pub scenario: String,
    /// Model tick (generations, ant steps or rows).
    pub tick: u64,
    /// Family details.
    pub detail: FamilyInfo,
    /// Why the model stopped, if it did.
    pub halted: Option<String>,
    /// The family's headline metric ([`Family::headline_metric`]) measured
    /// when the view was built, so the overlay agrees with the cells shown
    /// (also after edits made while paused, which record no sample).
    pub headline: Option<f64>,
}

impl LabInfo {
    /// Family of the model.
    pub fn family(&self) -> Family {
        match self.detail {
            FamilyInfo::Life { .. } => Family::LifeLike,
            FamilyInfo::Generations { .. } => Family::Generations,
            FamilyInfo::Wireworld { .. } => Family::Wireworld,
            FamilyInfo::Ant { .. } => Family::Ant,
            FamilyInfo::Wolfram { .. } => Family::Wolfram,
        }
    }

    /// The finite world, if any (Life-like, Generations, Wireworld, ants).
    pub fn world(&self) -> Option<World> {
        match &self.detail {
            FamilyInfo::Life { world, .. }
            | FamilyInfo::Generations { world, .. }
            | FamilyInfo::Wireworld { world }
            | FamilyInfo::Ant { world, .. } => Some(*world),
            FamilyInfo::Wolfram { .. } => None,
        }
    }

    /// Cells that exist: the finite world, the kept rows of the diagram, or
    /// `None` on the infinite plane.
    pub fn bounds(&self) -> Option<CellRect> {
        match &self.detail {
            FamilyInfo::Wolfram { width, first_row, generation, .. } => {
                Some(CellRect::new(0, *first_row as i64, i64::from(*width) - 1, *generation as i64))
            }
            _ => self.world().and_then(|w| w.bounds()),
        }
    }

    /// Number of states (colours) of the running rule.
    pub fn states(&self) -> u16 {
        match &self.detail {
            FamilyInfo::Life { .. } | FamilyInfo::Wolfram { .. } => 2,
            FamilyInfo::Generations { rule, .. } => u16::from(rule.states),
            FamilyInfo::Wireworld { .. } => 4,
            FamilyInfo::Ant { rule, .. } => u16::from(rule.colors()),
        }
    }

    /// The rule in its canonical notation.
    pub fn rule_text(&self) -> String {
        match &self.detail {
            FamilyInfo::Life { rule, .. } => rule.to_string(),
            FamilyInfo::Generations { rule, .. } => rule.to_bsc_string(),
            FamilyInfo::Wireworld { .. } => "Wireworld".to_string(),
            FamilyInfo::Ant { rule, .. } => rule.to_string(),
            FamilyInfo::Wolfram { rule, .. } => format!("Rule {rule}"),
        }
    }
}

/// Render data: the grid view, the ants in the requested rectangle and the
/// model summary.
#[derive(Clone, Debug)]
pub struct LabView {
    /// The model's grid view.
    pub grid: GridView,
    /// Ants inside the requested rectangle (ant models only).
    pub ants: Vec<Ant>,
    /// Model summary.
    pub info: LabInfo,
}

/// Serialised form of a [`LabSim`] (the payload of a `ca.lab` snapshot).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LabSaved {
    /// Scenario id.
    pub scenario: String,
    /// The model's own snapshot.
    pub inner: Snapshot,
    /// Running world of an ant model (the model has no accessor for it).
    pub ant_world: Option<World>,
}

/// The laboratory's simulation: one of five models.
pub struct LabSim {
    model: LabModel,
    scenario: String,
    ant_world: Option<World>,
    fill: FillSettings,
}

fn ca(e: cellular_automata::CaError) -> SimError {
    SimError::from(e)
}

/// World of an ant configuration.
fn ant_config_world(config: &AntConfig) -> Option<World> {
    World::new(config.topology, config.width, config.height).ok()
}

/// World described by an ant model's parameters (the world of its next reset).
fn ant_params_world(params: &ParamSet) -> Option<World> {
    let topology = params.get("topology").and_then(ParamValue::as_str).and_then(|t| match t {
        "infinite" => Some(Topology::Infinite),
        "torus" => Some(Topology::Torus),
        "bounded" => Some(Topology::Bounded),
        _ => None,
    })?;
    let side = |k: &str| params.get(k).and_then(ParamValue::as_i64).and_then(|v| u32::try_from(v).ok());
    World::new(topology, side("width")?, side("height")?).ok()
}

/// Oldest row an elementary automaton still keeps (rows `first..=generation` exist).
fn wolfram_first_row(sim: &WolframSim) -> u64 {
    let generation = sim.tick();
    if sim.row_at(0).is_some() {
        return 0;
    }
    let (mut lo, mut hi) = (0u64, generation);
    // Invariant: row `lo` is gone, row `hi` is kept.
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if sim.row_at(mid).is_some() { hi = mid } else { lo = mid }
    }
    hi
}

impl LabSim {
    /// Wraps a built scenario.
    pub fn from_built(built: BuiltSimulation, scenario: &Scenario) -> Result<Self, SimError> {
        let model = match built {
            BuiltSimulation::Life(s) => LabModel::Life(s),
            BuiltSimulation::Generations(s) => LabModel::Generations(s),
            BuiltSimulation::Wireworld(s) => LabModel::Wireworld(s),
            BuiltSimulation::Ant(s) => LabModel::Ant(s),
            BuiltSimulation::Wolfram(s) => LabModel::Wolfram(s),
        };
        let ant_world = match &scenario.model {
            cellular_automata::ScenarioModel::Ant(config) => ant_config_world(config),
            _ => None,
        };
        if matches!(model, LabModel::Life(ref s) if s.mode() != LifeMode::Lab) {
            return Err(SimError::InvalidConfig(format!(
                "scenario `{}` belongs to the Classic Conway mode",
                scenario.id
            )));
        }
        Ok(Self { model, scenario: scenario.id.clone(), ant_world, fill: FillSettings::default() })
    }

    /// Builds a scenario.
    ///
    /// # Errors
    /// Invalid scenarios and Classic Conway scenarios.
    pub fn from_scenario(scenario: &Scenario, library: &PatternLibrary) -> Result<Self, SimError> {
        let built = scenario.build(library).map_err(ca)?;
        Self::from_built(built, scenario)
    }

    /// The running model.
    pub fn model(&self) -> &LabModel {
        &self.model
    }

    /// Family of the running model.
    pub fn family(&self) -> Family {
        self.model.family()
    }

    /// Scenario id.
    pub fn scenario(&self) -> &str {
        &self.scenario
    }

    /// Exact value of cell `x` of the current row of an elementary
    /// automaton (`None` for other models or outside the row).
    pub fn wolfram_cell(&self, x: i64) -> Option<bool> {
        match &self.model {
            LabModel::Wolfram(s) => usize::try_from(x).ok().and_then(|x| s.current_row().get(x).copied()),
            _ => None,
        }
    }

    /// Settings of the Randomize button.
    pub fn fill(&self) -> FillSettings {
        self.fill
    }

    /// Changes the settings of the Randomize button.
    pub fn set_fill(&mut self, fill: FillSettings) {
        self.fill = FillSettings { density: fill.density.clamp(0.0, 1.0), state: fill.state };
    }

    /// Kills every cell (Life-like, Generations, Wireworld), sets every
    /// colour to 0 (ants keep their places) or clears the current row.
    pub fn clear(&mut self) -> Result<(), SimError> {
        let command = match self.model {
            LabModel::Life(_) => LabCommand::Life(LifeCommand::Clear),
            LabModel::Generations(_) | LabModel::Wireworld(_) => LabCommand::Paint(PaintCommand::Clear),
            LabModel::Ant(_) => LabCommand::Ant(AntCommand::ClearColors),
            LabModel::Wolfram(_) => LabCommand::Wolfram(WolframCommand::Clear),
        };
        self.apply(command)
    }

    /// Random content with the fill settings and `seed`: a soup over the
    /// world (or the home region of the infinite plane), random colours for
    /// ants, a random current row for elementary automata.
    pub fn randomize(&mut self, seed: u64) -> Result<(), SimError> {
        let FillSettings { density, state } = self.fill;
        let max_state = self.info().states().saturating_sub(1) as u8;
        let state = state.clamp(1, max_state.max(1));
        let command = match self.model {
            LabModel::Life(_) => LabCommand::Life(LifeCommand::Randomize {
                density,
                seed: Some(seed),
                region: None,
                symmetry: Symmetry::None,
            }),
            LabModel::Generations(_) | LabModel::Wireworld(_) => LabCommand::Paint(PaintCommand::Randomize {
                density,
                state,
                seed: Some(seed),
                region: None,
                symmetry: Symmetry::None,
            }),
            LabModel::Ant(_) => {
                LabCommand::Ant(AntCommand::Randomize { density, color: state, seed: Some(seed), region: None })
            }
            LabModel::Wolfram(_) => LabCommand::Wolfram(WolframCommand::Randomize { density, seed: Some(seed) }),
        };
        self.apply(command)
    }

    /// Summary of the running model.
    pub fn info(&self) -> LabInfo {
        let (detail, halted) = match &self.model {
            LabModel::Life(s) => (
                FamilyInfo::Life {
                    rule: s.rule(),
                    preset: s.preset(),
                    world: s.world(),
                    engine: s.engine_kind(),
                    generations_per_tick: s.generations_per_tick(),
                },
                s.halt_reason().map(str::to_string),
            ),
            LabModel::Generations(s) => {
                (FamilyInfo::Generations { rule: s.rule(), world: s.world() }, s.halt_reason().map(str::to_string))
            }
            LabModel::Wireworld(s) => (FamilyInfo::Wireworld { world: s.world() }, s.halt_reason().map(str::to_string)),
            LabModel::Ant(s) => {
                let world = self.ant_world.or_else(|| ant_params_world(&s.params())).unwrap_or(World {
                    topology: Topology::Infinite,
                    width: 256,
                    height: 256,
                });
                let halted = (s.ants().is_empty()).then(|| "every ant has left the bounded world".to_string());
                (
                    FamilyInfo::Ant {
                        rule: s.rule().clone(),
                        world,
                        ants: s.ants().len(),
                        first: s.ants().first().copied(),
                    },
                    halted,
                )
            }
            LabModel::Wolfram(s) => (
                FamilyInfo::Wolfram {
                    rule: s.rule(),
                    width: s.current_row().len() as u32,
                    first_row: wolfram_first_row(s),
                    generation: s.tick(),
                },
                None,
            ),
        };
        LabInfo { scenario: self.scenario.clone(), tick: self.tick(), detail, halted, headline: None }
    }

    fn blank(family: Family) -> Result<LabModel, SimError> {
        let library = PatternLibrary::builtin();
        Ok(match family {
            Family::LifeLike => LabModel::Life(
                LifeSim::new(&LifeConfig { init: Vec::new(), ..LifeConfig::default() }, 0, &library, LifeMode::Lab)
                    .map_err(ca)?,
            ),
            Family::Generations => LabModel::Generations(
                GenerationsSim::new(
                    &GenerationsConfig { init: Vec::new(), ..GenerationsConfig::default() },
                    0,
                    &library,
                )
                .map_err(ca)?,
            ),
            Family::Wireworld => {
                LabModel::Wireworld(WireworldSim::new(&WireworldConfig::default(), 0, &library).map_err(ca)?)
            }
            Family::Ant => LabModel::Ant(LangtonsAntSim::langton(0)),
            Family::Wolfram => LabModel::Wolfram(WolframSim::new(&WolframConfig::default(), 0).map_err(ca)?),
        })
    }

    fn family_of_model_id(id: &str) -> Option<Family> {
        match id {
            cellular_automata::life::LAB_MODEL_ID => Some(Family::LifeLike),
            cellular_automata::generations::GENERATIONS_MODEL_ID => Some(Family::Generations),
            cellular_automata::wireworld::WIREWORLD_MODEL_ID => Some(Family::Wireworld),
            cellular_automata::ant::ANT_MODEL_ID => Some(Family::Ant),
            cellular_automata::wolfram::WOLFRAM_MODEL_ID => Some(Family::Wolfram),
            _ => None,
        }
    }

    fn inner_descriptor(&self) -> ModelDescriptor {
        match &self.model {
            LabModel::Life(s) => s.descriptor(),
            LabModel::Generations(s) => s.descriptor(),
            LabModel::Wireworld(s) => s.descriptor(),
            LabModel::Ant(s) => s.descriptor(),
            LabModel::Wolfram(s) => s.descriptor(),
        }
    }
}

macro_rules! delegate {
    ($self:expr, $s:ident => $body:expr) => {
        match $self {
            LabModel::Life($s) => $body,
            LabModel::Generations($s) => $body,
            LabModel::Wireworld($s) => $body,
            LabModel::Ant($s) => $body,
            LabModel::Wolfram($s) => $body,
        }
    };
}

impl Simulation for LabSim {
    type Command = LabCommand;
    type ViewRequest = GridViewRequest;
    type View = LabView;

    fn descriptor(&self) -> ModelDescriptor {
        let mut d = self.inner_descriptor();
        d.summary = format!("{} Model {} of the cellular-automata crate.", d.summary, d.id);
        d.id = LAB_MODEL_ID.to_string();
        d
    }

    fn tick(&self) -> u64 {
        delegate!(&self.model, s => s.tick())
    }

    fn step(&mut self) {
        delegate!(&mut self.model, s => s.step());
    }

    fn advance(&mut self, max_ticks: u64) -> u64 {
        delegate!(&mut self.model, s => s.advance(max_ticks))
    }

    fn reset(&mut self) {
        delegate!(&mut self.model, s => s.reset());
        if let LabModel::Ant(s) = &self.model {
            // A reset rebuilds the configured world (or returns to the reset
            // point, which lies in that same world).
            if let Some(world) = ant_params_world(&s.params()) {
                self.ant_world = Some(world);
            }
        }
    }

    fn apply(&mut self, command: LabCommand) -> Result<(), SimError> {
        match (&mut self.model, command) {
            (LabModel::Life(s), LabCommand::Life(c)) => s.apply(c),
            (LabModel::Generations(s), LabCommand::Paint(c)) => s.apply(c),
            (LabModel::Wireworld(s), LabCommand::Paint(c)) => s.apply(c),
            (LabModel::Ant(s), LabCommand::Ant(c)) => s.apply(c),
            (LabModel::Wolfram(s), LabCommand::Wolfram(c)) => s.apply(c),
            (model, _) => Err(SimError::InvalidCommand(format!(
                "this edit does not apply to the running {} model",
                model.family().label()
            ))),
        }
    }

    fn params(&self) -> ParamSet {
        delegate!(&self.model, s => s.params())
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        delegate!(&mut self.model, s => s.set_param(key, value))
    }

    fn metrics(&self) -> Metrics {
        delegate!(&self.model, s => s.metrics())
    }

    fn view(&self, request: &GridViewRequest) -> LabView {
        let mut info = self.info();
        info.headline = self.metrics().get(self.family().headline_metric().0);
        match &self.model {
            LabModel::Ant(s) => {
                let AntView { grid, ants } = s.view(request);
                LabView { grid, ants, info }
            }
            LabModel::Life(s) => LabView { grid: s.view(request), ants: Vec::new(), info },
            LabModel::Generations(s) => LabView { grid: s.view(request), ants: Vec::new(), info },
            LabModel::Wireworld(s) => LabView { grid: s.view(request), ants: Vec::new(), info },
            LabModel::Wolfram(s) => LabView { grid: s.view(request), ants: Vec::new(), info },
        }
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        let inner = delegate!(&self.model, s => s.snapshot())?;
        let saved = LabSaved { scenario: self.scenario.clone(), inner, ant_world: self.ant_world };
        Snapshot::encode(LAB_MODEL_ID, LAB_SCHEMA, self.tick(), &saved)
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let saved: LabSaved = snapshot.decode(LAB_MODEL_ID, LAB_SCHEMA)?;
        let family = Self::family_of_model_id(&saved.inner.model_id).ok_or_else(|| SimError::WrongModel {
            expected: "a model of the cellular automata laboratory".to_string(),
            found: saved.inner.model_id.clone(),
        })?;
        if family == self.family() {
            delegate!(&mut self.model, s => s.restore(&saved.inner))?;
        } else {
            let mut model = Self::blank(family)?;
            delegate!(&mut model, s => s.restore(&saved.inner))?;
            self.model = model;
        }
        self.scenario = saved.scenario;
        self.ant_world = saved.ant_world;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        delegate!(&self.model, s => s.memory_bytes())
    }

    fn is_finished(&self) -> bool {
        delegate!(&self.model, s => s.is_finished())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use cellular_automata::scenario::builtin_scenario;

    /// The pattern library with the asset catalog (the AND gate is read from `assets/`).
    pub(crate) fn test_library() -> PatternLibrary {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/patterns");
        PatternLibrary::load(root).expect("asset catalog")
    }

    pub(crate) fn lab(id: &str) -> LabSim {
        let scenario = builtin_scenario(id).expect("bundled scenario");
        LabSim::from_scenario(&scenario, &test_library()).expect("builds")
    }

    #[test]
    fn every_family_builds_and_reports_its_info() {
        let cases = [
            ("highlife-replicator", Family::LifeLike, "B36/S23", 2),
            ("brians-brain", Family::Generations, "B2/S/C3", 3),
            ("wireworld-circuits", Family::Wireworld, "Wireworld", 4),
            ("turmite-llrr", Family::Ant, "LLRR", 4),
            ("wolfram-rule-110", Family::Wolfram, "Rule 110", 2),
        ];
        for (id, family, rule, states) in cases {
            let sim = lab(id);
            assert_eq!(sim.family(), family);
            let info = sim.info();
            assert_eq!(info.family(), family);
            assert_eq!(info.rule_text(), rule, "{id}");
            assert_eq!(info.states(), states, "{id}");
            assert_eq!(info.scenario, id);
            let d = sim.descriptor();
            assert_eq!(d.id, LAB_MODEL_ID);
            assert!(d.summary.contains("cellular-automata crate"), "{}", d.summary);
        }
        let ww = lab("wireworld-circuits").info();
        assert_eq!(ww.bounds(), Some(CellRect::new(0, 0, 63, 71)));
        assert_eq!(lab("langtons-ant-highway").info().bounds(), None, "infinite plane");
        assert_eq!(lab("turmite-four-ants").info().bounds(), Some(CellRect::new(0, 0, 255, 255)));
        assert!(
            LabSim::from_scenario(&builtin_scenario("glider-gun-showcase").unwrap(), &PatternLibrary::builtin())
                .is_err()
        );
    }

    #[test]
    fn views_carry_the_headline_metric() {
        let request = GridViewRequest { x0: 0, y0: 0, x1: 9, y1: 9, max_width: 10, max_height: 10 };
        let mut life = lab("seeds");
        let population = life.metrics().get("population");
        assert!(population.is_some_and(|p| p > 0.0));
        assert_eq!(life.view(&request).info.headline, population);
        // An edit while paused records no metric sample, but the next view shows it.
        life.apply(LabCommand::Life(LifeCommand::Clear)).unwrap();
        assert_eq!(life.view(&request).info.headline, Some(0.0));
        let ww = lab("wireworld-circuits");
        assert_eq!(ww.view(&request).info.headline, ww.metrics().get("heads"));
        assert_eq!(lab("wolfram-rule-30").view(&request).info.headline, Some(1.0), "ones in the current row");
        assert_eq!(lab("brians-brain").info().headline, None, "only views carry it");
    }

    #[test]
    fn commands_must_match_the_family() {
        let mut sim = lab("wolfram-rule-30");
        let err = sim.apply(LabCommand::Paint(PaintCommand::Clear)).unwrap_err().to_string();
        assert!(err.contains("Elementary CA"), "{err}");
        sim.apply(LabCommand::Wolfram(WolframCommand::Clear)).unwrap();
        assert_eq!(sim.metrics().get("ones"), Some(0.0));
        let mut life = lab("seeds");
        life.apply(LabCommand::Life(LifeCommand::SetCell { x: 3, y: 4, alive: true })).unwrap();
        let LabModel::Life(s) = life.model() else { panic!() };
        assert!(s.cell(3, 4));
    }

    #[test]
    fn wolfram_rows_scroll_away() {
        let scenario = Scenario {
            model: cellular_automata::ScenarioModel::Wolfram(WolframConfig {
                history: 8,
                width: 33,
                ..Default::default()
            }),
            ..builtin_scenario("wolfram-rule-90").unwrap()
        };
        let mut sim = LabSim::from_scenario(&scenario, &test_library()).unwrap();
        for _ in 0..20 {
            sim.step();
        }
        let FamilyInfo::Wolfram { first_row, generation, width, .. } = sim.info().detail else { panic!() };
        assert_eq!((first_row, generation, width), (13, 20, 33), "the last 8 rows are kept");
        assert_eq!(sim.info().bounds(), Some(CellRect::new(0, 13, 32, 20)));
    }

    #[test]
    fn snapshots_round_trip_and_switch_families() {
        let mut ant = lab("langtons-ant-highway");
        for _ in 0..30 {
            ant.step();
        }
        let ant_snapshot = ant.snapshot().unwrap();
        assert_eq!(ant_snapshot.model_id, LAB_MODEL_ID);
        assert_eq!(ant_snapshot.tick, 300);
        let mut sim = lab("brians-brain");
        sim.restore(&ant_snapshot).unwrap();
        assert_eq!(sim.family(), Family::Ant);
        assert_eq!(sim.tick(), 300);
        assert_eq!(sim.scenario(), "langtons-ant-highway");
        // The futures agree.
        ant.step();
        sim.step();
        assert_eq!(ant.metrics().get("colored"), sim.metrics().get("colored"));
        // Back to the brain.
        let brain = lab("brians-brain").snapshot().unwrap();
        sim.restore(&brain).unwrap();
        assert_eq!((sim.family(), sim.tick()), (Family::Generations, 0));
        // Foreign snapshots are refused without touching the state.
        let foreign = Snapshot::encode("other", 1, 0, &1u8).unwrap();
        assert!(sim.restore(&foreign).is_err());
        assert_eq!(sim.family(), Family::Generations);
    }

    #[test]
    fn ant_world_follows_resets() {
        let mut sim = lab("turmite-four-ants");
        sim.set_param("width", ParamValue::Int(64)).unwrap();
        assert_eq!(sim.info().world().map(|w| w.width), Some(256), "pending until the reset");
        sim.reset();
        assert_eq!(sim.info().world().map(|w| w.width), Some(64));
        let snapshot = sim.snapshot().unwrap();
        let mut other = lab("langtons-ant-highway");
        other.restore(&snapshot).unwrap();
        assert_eq!(other.info().world().map(|w| (w.topology, w.width)), Some((Topology::Torus, 64)));
    }

    #[test]
    fn clear_and_randomize_hooks() {
        let mut sim = lab("seeds");
        sim.clear().unwrap();
        assert_eq!(sim.metrics().get("population"), Some(0.0));
        sim.set_fill(FillSettings { density: 0.5, state: 1 });
        sim.randomize(7).unwrap();
        let p = sim.metrics().get("population").unwrap();
        assert!((p / 65_536.0 - 0.5).abs() < 0.02, "{p}");
        let mut ww = lab("wireworld-circuits");
        ww.set_fill(FillSettings { density: 0.2, state: 3 });
        ww.randomize(1).unwrap();
        assert!(ww.metrics().get("conductors").unwrap() > 0.0);
        assert_eq!(ww.metrics().get("heads"), Some(0.0));
        let mut eca = lab("wolfram-rule-30");
        eca.set_fill(FillSettings { density: 1.0, state: 1 });
        eca.randomize(3).unwrap();
        assert_eq!(eca.metrics().get("ones"), Some(1025.0));
    }
}
