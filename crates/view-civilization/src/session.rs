//! The simulation as run by this view: [`CivilizationSim`] wrapped in
//! [`CivSession`], which records observations the model does not publish
//! (see [`crate::ledger`]) and adds the selected person or building to the
//! published view.
//!
//! Everything else is delegated unchanged: descriptor (same model id, so
//! saves are ordinary `civilization` snapshots), parameters, commands,
//! metrics, snapshots. The ledger is display data only; it is cleared
//! whenever the run jumps in time (reset, restore, undo, history).

use civilization::agent::Target;
use civilization::buildings::{Building, BuildingId};
use civilization::view::CivView;
use civilization::{Activity, Agent, CivCommand, CivError, CivViewRequest, CivilizationSim, Scenario, TickStats};
use organism_engine::AgentId;
use simulation_core::{ApplyPolicy, Metrics, ModelDescriptor, ParamSet, ParamValue, SimError, Simulation, Snapshot};

use crate::ledger::{Ledger, PostObs, PreObs, TradeSummary, detect_trades};

/// What the inspector shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    /// Nothing selected.
    #[default]
    None,
    /// A person.
    Agent(AgentId),
    /// A building or construction site.
    Building(BuildingId),
}

/// What the view asks the session to publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRequest {
    /// Selected person or building (its details are included).
    pub selection: Selection,
    /// Window of the trade network in ticks.
    pub trade_window: u64,
}

impl Default for FrameRequest {
    fn default() -> Self {
        Self { selection: Selection::None, trade_window: 240 }
    }
}

/// One relationship as shown by the inspector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelationView {
    /// The other person.
    pub other: AgentId,
    /// Affinity in `[-1, 1]`.
    pub affinity: f32,
    /// Whether the other person is alive.
    pub alive: bool,
    /// The other person's group.
    pub group: Option<civilization::GroupId>,
}

/// Everything the inspector shows about one person.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentDetail {
    /// Identifier.
    pub id: AgentId,
    /// Full state of the person.
    pub agent: Agent,
    /// Relationships, strongest first.
    pub relations: Vec<RelationView>,
    /// Readable description of the goal's target.
    pub target: String,
    /// Specialisation index of the person (`None` before producing anything).
    pub specialization: Option<f64>,
    /// Activity changes, most recent first `(tick, activity)`.
    pub recent: Vec<(u64, Activity)>,
    /// Food in the household store of the person's hut.
    pub household_food: f32,
    /// Tile of the person's hut.
    pub home_tile: Option<(u32, u32)>,
    /// Tile of the person's farm plot.
    pub farm_tile: Option<(u32, u32)>,
}

/// Everything the inspector shows about one building.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingDetail {
    /// The building.
    pub building: Building,
    /// Tile coordinates.
    pub tile: (u32, u32),
    /// Whether the owner is alive.
    pub owner_alive: bool,
}

/// Scenario facts the tools need.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScenarioInfo {
    /// Scenario id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Founding population labels (spawn tool).
    pub populations: Vec<String>,
}

/// A published frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// The model's render data (whole world).
    pub view: CivView,
    /// Selected person.
    pub agent: Option<AgentDetail>,
    /// Selected building.
    pub building: Option<BuildingDetail>,
    /// Trade network over the requested window.
    pub trade: TradeSummary,
    /// Scenario facts.
    pub info: ScenarioInfo,
    /// Regeneration multipliers (berries, fish, wood, stone, crops).
    pub regen: [f32; 5],
    /// Event counts of the last tick.
    pub stats: TickStats,
}

/// [`CivilizationSim`] plus the view's ledger.
#[derive(Debug)]
pub struct CivSession {
    sim: CivilizationSim,
    ledger: Ledger,
}

impl CivSession {
    /// Builds the scenario.
    pub fn new(scenario: Scenario) -> Result<Self, CivError> {
        Ok(Self { sim: CivilizationSim::new(scenario)?, ledger: Ledger::default() })
    }

    /// The wrapped model.
    pub fn sim(&self) -> &CivilizationSim {
        &self.sim
    }

    /// Observations recorded so far.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Sets the seed and rebuilds (the shell's Randomize control).
    pub fn reseed(&mut self, seed: u64) -> Result<(), CivError> {
        self.ledger.clear();
        self.sim.reseed(seed)
    }

    fn observe_pre(&self) -> Vec<PreObs> {
        let mut out: Vec<PreObs> = self
            .sim
            .agents()
            .iter()
            .map(|(id, a)| PreObs {
                id,
                sales: a.sales.0,
                inventory: a.inventory.0,
                trade_target: match (a.goal.activity, a.goal.target) {
                    (Activity::Trade, Target::Agent(other)) => Some(other),
                    _ => None,
                },
                next_decision: a.next_decision,
            })
            .collect();
        out.sort_by_key(|p| p.id);
        out
    }

    fn observe_post(&self) -> Vec<PostObs> {
        let mut out: Vec<PostObs> = self
            .sim
            .agents()
            .iter()
            .map(|(id, a)| PostObs {
                id,
                sales: a.sales.0,
                inventory: a.inventory.0,
                doing: a.doing,
                goal_cleared: a.goal.activity == Activity::Idle,
                next_decision: a.next_decision,
                position: a.kin.position,
                group: a.group,
            })
            .collect();
        out.sort_by_key(|p| p.id);
        out
    }

    fn describe_target(&self, target: Target) -> String {
        let world = self.sim.world();
        match target {
            Target::None => "where it stands".into(),
            Target::Tile(t) if (t as usize) < world.len() => {
                let (x, y) = world.coords(t);
                format!("{} at ({x}, {y})", world.terrain(t).label().to_lowercase())
            }
            Target::Tile(t) => format!("tile {t}"),
            Target::Agent(id) => format!("person #{}", id.0),
            Target::Building(b) => match self.sim.buildings().get(b) {
                Some(building) => {
                    let (x, y) = world.coords(building.tile);
                    format!("{} #{} at ({x}, {y})", building.kind.label(), b.0)
                }
                None => format!("building #{} (gone)", b.0),
            },
        }
    }

    fn agent_detail(&self, id: AgentId) -> Option<AgentDetail> {
        let agent = self.sim.agents().get(id)?.clone();
        let world = self.sim.world();
        let buildings = self.sim.buildings();
        let mut relations: Vec<RelationView> = agent
            .relations
            .iter()
            .map(|r| {
                let other = self.sim.agents().get(r.other);
                RelationView {
                    other: r.other,
                    affinity: r.affinity,
                    alive: other.is_some(),
                    group: other.and_then(|o| o.group),
                }
            })
            .collect();
        relations.sort_by(|a, b| b.affinity.abs().total_cmp(&a.affinity.abs()).then(a.other.cmp(&b.other)));
        let tile_of = |b: Option<BuildingId>| b.and_then(|b| buildings.get(b)).map(|b| world.coords(b.tile));
        let household_food =
            agent.home.and_then(|h| buildings.get(h)).map_or(0.0, |b| b.store.get(civilization::Good::Food));
        Some(AgentDetail {
            id,
            target: self.describe_target(agent.goal.target),
            specialization: civilization::metrics::specialization(&agent),
            recent: self.ledger.activity_of(id),
            household_food,
            home_tile: tile_of(agent.home),
            farm_tile: tile_of(agent.farm),
            relations,
            agent,
        })
    }

    fn building_detail(&self, id: BuildingId) -> Option<BuildingDetail> {
        let building = self.sim.buildings().get(id)?.clone();
        let tile = self.sim.world().coords(building.tile);
        let owner_alive = building.owner.is_some_and(|o| self.sim.agents().contains(o));
        Some(BuildingDetail { building, tile, owner_alive })
    }
}

impl Simulation for CivSession {
    type Command = CivCommand;
    type ViewRequest = FrameRequest;
    type View = Frame;

    fn descriptor(&self) -> ModelDescriptor {
        self.sim.descriptor()
    }

    fn tick(&self) -> u64 {
        self.sim.tick()
    }

    fn step(&mut self) {
        let tick = self.sim.tick();
        let daily = tick.is_multiple_of(civilization::agent::TICKS_PER_DAY);
        let pre = self.observe_pre();
        self.sim.step();
        let post = self.observe_post();
        let reported = self.sim.last_tick_stats().trades;
        let pairs = detect_trades(&pre, &post, daily, reported);
        self.ledger.record_trades(tick, &pairs, &post, reported);
        self.ledger.record_activity(tick, &post);
    }

    fn reset(&mut self) {
        self.ledger.clear();
        self.sim.reset();
    }

    fn apply(&mut self, command: CivCommand) -> Result<(), SimError> {
        self.sim.apply(command)
    }

    fn params(&self) -> ParamSet {
        self.sim.params()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        self.sim.set_param(key, value)
    }

    fn metrics(&self) -> Metrics {
        self.sim.metrics()
    }

    fn view(&self, request: &FrameRequest) -> Frame {
        let view = self.sim.view(&CivViewRequest::default());
        let (agent, building) = match request.selection {
            Selection::None => (None, None),
            Selection::Agent(id) => (self.agent_detail(id), None),
            Selection::Building(id) => (None, self.building_detail(id)),
        };
        let scenario = self.sim.scenario();
        Frame {
            trade: self.ledger.summary(self.sim.tick(), request.trade_window),
            info: ScenarioInfo {
                id: scenario.id.clone(),
                name: scenario.name.clone(),
                populations: scenario.populations.iter().map(|p| p.label.clone()).collect(),
            },
            regen: self.sim.rules().regen,
            stats: *self.sim.last_tick_stats(),
            view,
            agent,
            building,
        }
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        self.sim.snapshot()
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        self.sim.restore(snapshot)?;
        self.ledger.clear();
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        self.sim.memory_bytes() + self.ledger.memory_bytes()
    }

    fn is_finished(&self) -> bool {
        self.sim.is_finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> CivSession {
        CivSession::new(civilization::scenarios::builtin_by_id(id).expect("builtin")).expect("valid")
    }

    /// Trade detection must agree with the model's own count of successful
    /// meetings: per tick the detected meetings may never exceed the
    /// reported ones, and over a long run they must cover almost all.
    #[test]
    fn detected_trades_agree_with_the_model() {
        for id in ["trade_routes", "two_strategies"] {
            let mut s = session(id);
            let (mut detected, mut reported, mut over) = (0u64, 0u64, 0u64);
            for _ in 0..1500 {
                let tick = s.tick();
                s.step();
                let found = s.ledger().trades_since(tick, 0).filter(|t| t.tick == tick).count() as u64;
                let model = u64::from(s.sim().last_tick_stats().trades);
                detected += found.min(model);
                over += found.saturating_sub(model);
                reported += model;
            }
            assert!(reported > 500, "{id}: the scenario trades ({reported})");
            let recall = detected as f64 / reported as f64;
            eprintln!("{id}: detected {detected} of {reported} meetings ({recall:.3}), {over} spurious");
            assert!(recall > 0.95, "{id}: recall {recall:.3}");
            assert_eq!(over, 0, "{id}: never more meetings than the model reported");
        }
    }

    #[test]
    fn wrapper_delegates_and_clears_the_ledger_on_time_jumps() {
        let mut s = session("abundant");
        assert_eq!(s.descriptor().id, civilization::MODEL_ID);
        let snap = s.snapshot().unwrap();
        for _ in 0..30 {
            s.step();
        }
        let id = s.sim().agents().ids()[0];
        assert!(!s.ledger().activity_of(id).is_empty());
        s.restore(&snap).unwrap();
        assert_eq!(s.tick(), 0);
        assert!(s.ledger().activity_of(id).is_empty());
        let frame = s.view(&FrameRequest { selection: Selection::Agent(id), trade_window: 24 });
        let detail = frame.agent.expect("selected person");
        assert_eq!(detail.id, id);
        assert_eq!(frame.info.id, "abundant");
        assert!(!frame.view.terrain.is_empty());
        let missing = s.view(&FrameRequest { selection: Selection::Agent(AgentId(u64::MAX)), trade_window: 24 });
        assert!(missing.agent.is_none());
    }
}
