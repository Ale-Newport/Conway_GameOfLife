//! Serialisable model state and per-tick statistics.

use organism_engine::AgentStore;
use organism_engine::perception::Percept;
use serde::{Deserialize, Serialize};
use simulation_core::{EventLog, SimRng, SpatialGrid};

use crate::agent::{Agent, DeathCause};
use crate::buildings::Buildings;
use crate::config::Rules;
use crate::groups::Groups;
use crate::path::PathFinder;
use crate::world::World;

/// Counts of events during the last tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TickStats {
    /// Units of goods that changed hands in barter.
    pub trade_volume: f32,
    /// Completed barter meetings.
    pub trades: u32,
    /// Fights (theft resisted or territory defended).
    pub conflicts: u32,
    /// Successful thefts.
    pub thefts: u32,
    /// Food given to others.
    pub shared_food: f32,
    /// Children born.
    pub births: u32,
    /// Deaths by cause, in the order starvation, thirst, conflict, old age.
    pub deaths: [u32; 4],
    /// Buildings completed.
    pub completed: u32,
    /// Units gathered from wild stocks (food, wood, stone) and harvested crops.
    pub gathered: f32,
}

/// Index of a death cause in [`TickStats::deaths`].
pub fn death_index(cause: DeathCause) -> usize {
    match cause {
        DeathCause::Starvation => 0,
        DeathCause::Thirst => 1,
        DeathCause::Conflict => 2,
        DeathCause::Age => 3,
    }
}

/// Everything that evolves during a run (captured by snapshots).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CivState {
    /// Ticks since reset.
    pub tick: u64,
    /// Active rule parameters.
    pub rules: Rules,
    /// Tile world.
    pub world: World,
    /// People.
    pub agents: AgentStore<Agent>,
    /// Buildings and construction sites.
    pub buildings: Buildings,
    /// Groups.
    pub groups: Groups,
    /// Behaviour and social random stream (drawn in id order only).
    pub rng: SimRng,
    /// Events of the last tick.
    pub stats: TickStats,
    /// Notable events (births, deaths, groups, buildings, disasters).
    pub events: EventLog,
}

/// Per-run scratch buffers that are rebuilt from the state and therefore not
/// serialised.
#[derive(Clone, Debug)]
pub struct Scratch {
    /// Agent positions indexed by store slot.
    pub grid: SpatialGrid,
    /// Route planner buffers.
    pub paths: PathFinder,
    /// Perception buffer.
    pub percepts: Vec<Percept>,
}

impl Scratch {
    /// Scratch sized for `world`.
    pub fn new(world: &World) -> Self {
        Self {
            grid: SpatialGrid::new(world.width() as f32, world.height() as f32, 4.0),
            paths: PathFinder::new(),
            percepts: Vec::new(),
        }
    }
}
