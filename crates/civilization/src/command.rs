//! User interventions.

use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent, SimRng};

use crate::agent::{Agent, Needs, SOCIAL_RATE, Sex, Skills, TICKS_PER_YEAR};
use crate::config::{PopulationSpec, Rules};
use crate::economy::Good;
use crate::error::CivError;
use crate::state::CivState;
use crate::terrain::Resource;
use crate::territory;

/// Largest radius accepted by area commands, in tiles.
pub const MAX_RADIUS: f32 = 64.0;
/// Largest number of agents spawned by one command.
pub const MAX_SPAWN: u32 = 200;
/// Hard limit on the number of living agents.
pub const MAX_AGENTS: usize = 5000;

/// A command applied between ticks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CivCommand {
    /// Adds `count` adults drawn from scenario population `population` on
    /// passable tiles within 3 tiles of `(x, y)`.
    SpawnAgents {
        /// Centre x in tile units.
        x: f32,
        /// Centre y in tile units.
        y: f32,
        /// Number of agents (1–200).
        count: u32,
        /// Index of the scenario population whose preferences they get.
        population: u8,
    },
    /// Turns the tiles of a disc (without buildings) into the terrain that
    /// carries `resource`, with a full stock. Water (fish) is not placed under
    /// agents.
    AddResources {
        /// Centre column.
        x: u32,
        /// Centre row.
        y: u32,
        /// Radius in tiles.
        radius: f32,
        /// Berries, fish, wood or stone.
        resource: Resource,
    },
    /// Empties the stock of `resource` (all resources if `None`) in a disc;
    /// it grows back normally.
    RemoveResources {
        /// Centre column.
        x: u32,
        /// Centre row.
        y: u32,
        /// Radius in tiles.
        radius: f32,
        /// Resource to remove.
        resource: Option<Resource>,
    },
    /// Destroys every building (with stores and crops) in a disc, leaves
    /// danger behind and recomputes territory immediately.
    Disaster {
        /// Centre column.
        x: u32,
        /// Centre row.
        y: u32,
        /// Radius in tiles.
        radius: f32,
    },
    /// Sets the growback multiplier of a resource (`0..=5`).
    SetRegeneration {
        /// Resource (crops scale farm growth).
        resource: Resource,
        /// Multiplier.
        multiplier: f32,
    },
}

fn check_radius(radius: f32) -> Result<(), CivError> {
    if radius.is_finite() && radius > 0.0 && radius <= MAX_RADIUS {
        Ok(())
    } else {
        Err(CivError::InvalidCommand(format!("radius {radius} must be in (0, {MAX_RADIUS}]")))
    }
}

fn check_tile(state: &CivState, x: u32, y: u32) -> Result<(), CivError> {
    if x < state.world.width() && y < state.world.height() {
        Ok(())
    } else {
        Err(CivError::InvalidCommand(format!(
            "tile ({x}, {y}) is outside the {}×{} world",
            state.world.width(),
            state.world.height()
        )))
    }
}

/// Tiles whose centres lie within `radius` of the centre of `(x, y)`.
fn disc(state: &CivState, x: u32, y: u32, radius: f32) -> Vec<u32> {
    let world = &state.world;
    let centre = world.center(world.index(x, y));
    let r = radius.ceil() as i64;
    let mut out = Vec::new();
    for ty in (i64::from(y) - r).max(0)..=(i64::from(y) + r).min(i64::from(world.height()) - 1) {
        for tx in (i64::from(x) - r).max(0)..=(i64::from(x) + r).min(i64::from(world.width()) - 1) {
            let t = world.index(tx as u32, ty as u32);
            if world.center(t).distance(centre) <= radius {
                out.push(t);
            }
        }
    }
    out
}

/// Builds a founder-like adult of `population` at `position`: age 16–35
/// years, lifespan 55–75 years, preferences drawn from the population,
/// small random initial skills (0–0.15) and partially satisfied needs.
pub fn make_adult(position: Vec2, population: &PopulationSpec, index: u8, rules: &Rules, rng: &mut SimRng) -> Agent {
    let year = TICKS_PER_YEAR as i64;
    let sex = if rng.chance(0.5) { Sex::Female } else { Sex::Male };
    let age = rng.range_i64(16 * year, 35 * year + 1) as u64;
    let max_age = rng.range_i64(55 * year, 75 * year + 1) as u64;
    let prefs = population.preferences.sample(rng);
    let mut agent =
        Agent::new(position, sex, age, max_age, prefs, Needs::new(rules.thirst_rate, rules.fatigue_rate, SOCIAL_RATE));
    let mut skills = Skills::default();
    for s in &mut skills.0 {
        *s = rng.range_f32(0.0, 0.15);
    }
    agent.skills = skills;
    agent.energy = rng.range_f32(0.7, 0.9);
    agent.needs.water.value = rng.range_f32(0.0, 0.3);
    agent.needs.rest.value = rng.range_f32(0.0, 0.3);
    agent.inventory.set(Good::Food, population.starting_food);
    agent.population = index;
    agent
}

/// Applies `command` to `state`. `populations` are the scenario's
/// populations (for spawning) and `rules` the configured rules that survive
/// resets (updated by [`CivCommand::SetRegeneration`]).
pub(crate) fn apply(
    state: &mut CivState,
    populations: &[PopulationSpec],
    rules: &mut Rules,
    command: CivCommand,
) -> Result<(), CivError> {
    match command {
        CivCommand::SpawnAgents { x, y, count, population } => {
            if !(1..=MAX_SPAWN).contains(&count) {
                return Err(CivError::InvalidCommand(format!("count {count} must be in [1, {MAX_SPAWN}]")));
            }
            let spec = populations.get(usize::from(population)).ok_or_else(|| {
                CivError::InvalidCommand(format!(
                    "population {population} does not exist (the scenario has {})",
                    populations.len()
                ))
            })?;
            if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
                return Err(CivError::InvalidCommand(format!("position ({x}, {y}) is not inside the world")));
            }
            check_tile(state, x as u32, y as u32)?;
            if state.agents.len() + count as usize > MAX_AGENTS {
                return Err(CivError::InvalidCommand(format!(
                    "spawning {count} agents would exceed the limit of {MAX_AGENTS}"
                )));
            }
            let spots: Vec<u32> =
                disc(state, x as u32, y as u32, 3.0).into_iter().filter(|&t| state.world.passable(t)).collect();
            if spots.is_empty() {
                return Err(CivError::InvalidCommand(format!("no passable land within 3 tiles of ({x}, {y})")));
            }
            let shores = state.world.shore_tiles();
            for _ in 0..count {
                let tile = spots[state.rng.index(spots.len())];
                let position = state.world.center(tile)
                    + Vec2::new(state.rng.range_f32(-0.3, 0.3), state.rng.range_f32(-0.3, 0.3));
                let mut agent = make_adult(position, spec, population, &state.rules, &mut state.rng);
                agent.memory.water = state.world.nearest_of(&shores, position);
                state.agents.queue_spawn(agent);
            }
            state.agents.commit();
            state.events.push(
                SimEvent::new(state.tick, "spawn", format!("{count} agents of `{}` arrived", spec.label))
                    .severity(EventSeverity::Major),
            );
            Ok(())
        }
        CivCommand::AddResources { x, y, radius, resource } => {
            check_tile(state, x, y)?;
            check_radius(radius)?;
            let terrain = resource.terrain().ok_or_else(|| {
                CivError::InvalidCommand("crops cannot be placed; build a farm plot and plant them".into())
            })?;
            let mut occupied: Vec<u32> =
                state.agents.as_slice().iter().filter_map(|a| state.world.tile_at(a.kin.position)).collect();
            occupied.sort_unstable();
            let tiles: Vec<u32> = disc(state, x, y, radius)
                .into_iter()
                .filter(|&t| {
                    state.world.building_at(t) == 0 && (terrain.passable() || occupied.binary_search(&t).is_err())
                })
                .collect();
            state.world.set_terrains(tiles, terrain);
            Ok(())
        }
        CivCommand::RemoveResources { x, y, radius, resource } => {
            check_tile(state, x, y)?;
            check_radius(radius)?;
            for t in disc(state, x, y, radius) {
                let here = state.world.terrain(t).resource();
                if here.is_some() && (resource.is_none() || resource == here) {
                    state.world.set_stock(t, 0.0);
                }
            }
            Ok(())
        }
        CivCommand::Disaster { x, y, radius } => {
            check_tile(state, x, y)?;
            check_radius(radius)?;
            let area = disc(state, x, y, radius);
            let centre = state.world.center(state.world.index(x, y));
            let world = &state.world;
            let destroyed = state.buildings.remove_where(|b| world.center(b.tile).distance(centre) > radius);
            for b in &destroyed {
                state.world.set_building(b.tile, 0);
                for a in state.agents.as_mut_slice() {
                    if a.home == Some(b.id) {
                        a.home = None;
                    }
                    if a.farm == Some(b.id) {
                        a.farm = None;
                    }
                }
            }
            for &t in &area {
                state.world.add_danger(t, 0.5);
            }
            // Territory follows the remaining buildings at once rather than at
            // the next daily update.
            if !destroyed.is_empty() {
                territory::update(&mut state.world, &state.buildings, &mut state.groups, &state.rules);
            }
            state.events.push(
                SimEvent::new(
                    state.tick,
                    "disaster",
                    format!("disaster at ({x}, {y}) destroyed {} buildings", destroyed.len()),
                )
                .severity(EventSeverity::Major),
            );
            Ok(())
        }
        CivCommand::SetRegeneration { resource, multiplier } => {
            if !(multiplier.is_finite() && (0.0..=5.0).contains(&multiplier)) {
                return Err(CivError::InvalidCommand(format!("multiplier {multiplier} must be in [0, 5]")));
            }
            state.rules.regen[resource.index()] = multiplier;
            rules.regen[resource.index()] = multiplier;
            Ok(())
        }
    }
}
