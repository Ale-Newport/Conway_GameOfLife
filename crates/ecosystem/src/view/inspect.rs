//! The organism inspector: the full state, perception, utility scores and
//! history of one animal or tree.

use organism_engine::AgentId;
use serde::{Deserialize, Serialize};
use simulation_core::SimEvent;
use simulation_core::glam::Vec2;

use crate::animal::decide::{ACTIONS, scores};
use crate::animal::genetics::Phenotype;
use crate::animal::perceive::perceive;
use crate::animal::{Action, LifeStats, OrganismRef, ReproState, Sex, Target};
use crate::disease::Infection;
use crate::error::EcosystemError;
use crate::species::{AnimalTraits, TreeStyle};
use crate::state::{EcoState, Runtime};
use crate::trees::{TreeDeath, TreeStage};

/// Full state of an animal for the inspector.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimalInspection {
    /// Id.
    pub id: u64,
    /// Species id.
    pub species: String,
    /// Species name.
    pub name: String,
    /// Sex.
    pub sex: Sex,
    /// Age (simulated days).
    pub age_days: f32,
    /// Age as a fraction of the lifespan.
    pub age_fraction: f32,
    /// Mass (kg) and adult mass (kg).
    pub mass_kg: (f32, f32),
    /// Energy reserve and capacity (kJ).
    pub energy_kj: (f32, f32),
    /// Nutrient store (g).
    pub nutrient_store_g: f32,
    /// Hunger, thirst, fatigue in `[0, 1]`.
    pub drives: (f32, f32, f32),
    /// Stamina, health, thermal comfort in `[0, 1]`.
    pub condition: (f32, f32, f32),
    /// Air temperature at its position (°C).
    pub temperature_c: f32,
    /// Position, velocity (cells, cells per tick), heading.
    pub kinematics: (Vec2, Vec2, f32),
    /// Current action, its target and ticks since it started.
    pub action: (Action, Target, u64),
    /// Hidden in its burrow.
    pub hidden: bool,
    /// Burrow / den.
    pub home: Option<Vec2>,
    /// Reproductive state.
    pub reproduction: ReproState,
    /// Expressed genes `(name, value)`.
    pub genes: Vec<(String, f32)>,
    /// Generation and parents.
    pub lineage: (u32, [Option<u64>; 2]),
    /// Recorded children in the genealogy.
    pub children: Vec<u64>,
    /// Utility scores of the current situation `(action, score)`.
    pub utility: Vec<(String, f32)>,
    /// Perception summary: threat, prey, mate, food, water (distances in cells).
    pub senses: Vec<(String, f32)>,
    /// Life counters.
    pub stats: LifeStats,
    /// Species-derived numbers (metabolism, speeds, durations).
    pub traits: AnimalTraits,
    /// Events about it, oldest first (global log and detailed follow log).
    pub history: Vec<SimEvent>,
    /// Height above the ground (m; negative below the water surface).
    pub altitude_m: f32,
    /// Injury in `[0, 1]`.
    pub injury: f32,
    /// Infection record.
    pub infection: Option<Infection>,
    /// Social group and its leader (packs).
    pub group: Option<(u64, Option<u64>)>,
    /// Alarm call: threat position and strength.
    pub alarm: Option<(Vec2, f32)>,
}

/// Full state of a tree for the inspector.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreeInspection {
    /// Id.
    pub id: u64,
    /// Species id and name.
    pub species: (String, String),
    /// Style.
    pub style: TreeStyle,
    /// Stage.
    pub stage: TreeStage,
    /// Age (simulated years).
    pub age_years: f32,
    /// Height and crown radius (m).
    pub size_m: (f32, f32),
    /// Wood and leaf mass (kg).
    pub biomass_kg: (f32, f32),
    /// Health and relative light at the crown.
    pub condition: (f32, f32),
    /// Fire scar.
    pub burnt: bool,
    /// Death tick and cause.
    pub death: Option<(u64, Option<TreeDeath>)>,
    /// Position (cells).
    pub position: Vec2,
}

/// Inspection of a selected organism.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrganismInspection {
    /// An animal.
    Animal(Box<AnimalInspection>),
    /// A tree.
    Tree(TreeInspection),
}

impl EcoState {
    /// Full state and history of an organism.
    pub(crate) fn inspect(&self, rt: &Runtime, organism: OrganismRef) -> Result<OrganismInspection, EcosystemError> {
        match organism {
            OrganismRef::Animal(raw) => {
                let id = AgentId(raw);
                let index = self.animals.index_of(id).ok_or(EcosystemError::UnknownOrganism(raw))?;
                let a = &self.animals.as_slice()[index];
                let s = usize::from(a.species);
                let species = &self.catalog.animals()[s];
                let traits = &rt.traits[s];
                let pheno = Phenotype::of(&a.genome, &rt.schema);
                let genes = rt
                    .schema
                    .genes()
                    .iter()
                    .enumerate()
                    .map(|(i, g)| (g.name.clone(), a.genome.express(&rt.schema, i)))
                    .collect();
                // Re-run perception and scoring on the current state (pure)
                // with a spatial index of the current positions.
                let (utility, senses) = if self.netlogo.is_none() {
                    let (w, h) = (self.world.width() as f32, self.world.height() as f32);
                    let mut grid = simulation_core::SpatialGrid::new(w, h, 8.0);
                    grid.rebuild(self.animals.as_slice().iter().enumerate().map(|(i, a)| (i as u32, a.kin.position)));
                    let mut carcass_grid = simulation_core::SpatialGrid::new(w, h, 8.0);
                    carcass_grid
                        .rebuild(self.remains.carcasses.iter().enumerate().map(|(i, c)| (i as u32, c.position)));
                    let mut clutch_index = crate::carcass::ClutchIndex::new(w, h);
                    clutch_index.rebuild(&self.remains.clutches);
                    let ctx = self.ctx_with(rt, &grid, &carcass_grid, &clutch_index);
                    let senses = perceive(&ctx, index);
                    let utility = ACTIONS
                        .iter()
                        .zip(scores(&ctx, index, &senses))
                        .map(|(a, v)| (a.key().to_string(), v))
                        .collect();
                    let mut summary = Vec::new();
                    if let Some(t) = senses.threat {
                        summary.push(("threat".to_string(), t.distance));
                    }
                    if let Some(p) = senses.prey {
                        summary.push(("prey".to_string(), p.distance));
                    }
                    if let Some((_, _, d)) = senses.mate {
                        summary.push(("mate".to_string(), d));
                    }
                    if let Some(f) = senses.food {
                        summary.push(("food".to_string(), f.distance));
                    }
                    if let Some((_, d)) = senses.water {
                        summary.push(("water".to_string(), d));
                    }
                    if let Some(p) = senses.water_search {
                        summary.push(("water_search".to_string(), p.distance(a.kin.position)));
                    }
                    (utility, summary)
                } else {
                    (ACTIONS.iter().zip(a.last_scores).map(|(a, v)| (a.key().to_string(), v)).collect(), Vec::new())
                };
                let mut history: Vec<SimEvent> = self.events.about(raw).cloned().collect();
                history.extend(
                    self.follow.log.about(raw).filter(|e| e.severity < simulation_core::EventSeverity::Info).cloned(),
                );
                history.sort_by_key(|e| e.tick);
                let children =
                    self.genealogy.get(id).map(|n| n.children().iter().map(|c| c.get()).collect()).unwrap_or_default();
                let cell = {
                    let w = self.world.width();
                    let x = (a.kin.position.x.max(0.0) as usize).min(w - 1);
                    let y = (a.kin.position.y.max(0.0) as usize).min(self.world.height() - 1);
                    y * w + x
                };
                Ok(OrganismInspection::Animal(Box::new(AnimalInspection {
                    id: raw,
                    species: species.id.clone(),
                    name: species.name.clone(),
                    sex: a.sex,
                    age_days: (a.age_ticks(self.tick) as f64 * f64::from(rt.scale.dt_days())) as f32,
                    age_fraction: (a.age_ticks(self.tick) as f64 / traits.lifespan_ticks) as f32,
                    mass_kg: (a.mass_kg, traits.adult_mass_kg * pheno.size),
                    energy_kj: (a.energy_kj, a.reserve_max_kj(traits)),
                    nutrient_store_g: a.nutrient_store_g,
                    drives: (a.hunger(traits), a.thirst(), a.fatigue),
                    condition: (a.stamina, a.health, a.comfort),
                    temperature_c: self.world.temperature().as_slice()[cell],
                    kinematics: (a.kin.position, a.kin.velocity, a.kin.heading),
                    action: (a.action, a.target, self.tick.saturating_sub(a.action_since)),
                    hidden: a.hidden,
                    home: a.home,
                    reproduction: a.repro.clone(),
                    genes,
                    lineage: (a.lineage.generation, a.lineage.parents.map(|p| p.map(AgentId::get))),
                    children,
                    utility,
                    senses,
                    stats: a.stats,
                    traits: traits.clone(),
                    history,
                    altitude_m: a.altitude_m,
                    injury: a.injury,
                    infection: a.infection,
                    group: a
                        .group
                        .map(|g| (g, crate::animal::social::leader_in(&rt.leaders, Some(g)).map(AgentId::get))),
                    alarm: a.alarm,
                })))
            }
            OrganismRef::Tree(raw) => {
                let t = self.trees.get(AgentId(raw)).ok_or(EcosystemError::UnknownOrganism(raw))?;
                let sp = &self.catalog.trees()[usize::from(t.species)];
                Ok(OrganismInspection::Tree(TreeInspection {
                    id: raw,
                    species: (sp.id.clone(), sp.name.clone()),
                    style: sp.style,
                    stage: t.stage(sp, self.tick, &rt.scale),
                    age_years: t.age_years(self.tick, &rt.scale),
                    size_m: (t.height_m, sp.crown_radius(t.height_m)),
                    biomass_kg: (t.wood_kg, t.leaf_kg),
                    condition: (t.health, t.light),
                    burnt: t.burnt,
                    death: t.dead_since.map(|d| (d, t.death)),
                    position: t.position,
                }))
            }
        }
    }
}
