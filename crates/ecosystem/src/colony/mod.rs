//! Eusocial insect colonies (ants, honey bees) as super-agents.
//!
//! A [`Colony`] sits at its nest with a worker count, cohorts of developing
//! brood and a food store (kJ and g). A few [`Forager`] squads, each
//! standing for `workers_per_forager` workers of the colony, leave the nest
//! while it is warm enough (and light, for diurnal species), find food,
//! carry a load home and unload it into the store. Per tick (`Δt`):
//!
//! ```text
//! T_nest      regulated (hives) or the air temperature clamped to 4–30 °C (soil-buffered nests)
//! respiration workers · B(m, T_nest) · (field multiplier while foraging) · Δt   paid from the store;
//!             a deficit is paid with worker tissue (starvation)
//! mortality   workers · Δt / worker lifespan                                   into the litter at the nest
//! laying      min(max_eggs · Δt, affordable) while foraging and below max_workers; each egg costs
//!             m · 7 kJ/g (+ 50 % synthesis overhead) and m · 30 g/kg from the store
//! emergence   brood cohorts become workers after the development time
//! swarming    in the breeding season a colony above `swarm_workers` with a full store splits:
//!             half of the workers and store found a new colony nearby
//! foraging    squads = min(max_foragers, workers · forager fraction / workers per squad);
//!             a flying squad leaving the nest heads for the patch reported by returning foragers
//!             (recruitment, probability 0.7) or scouts a random point of the foraging range;
//!             a searching squad harvests the food of its cell (the first diet item with at
//!             least half a load; carrion within one cell first) or moves: towards higher trail
//!             pheromone (ants), towards the best food it senses, or by a correlated random
//!             walk; beyond the foraging range it turns home
//!             a loaded squad walks straight home, ants marking the trail with pheromone;
//!             at the nest the load is assimilated into the store, the rest egested
//! pheromone   p ← p (1 − ln 2 Δt / half-life), then a fraction D exchanged with the 4 neighbours
//! ```
//!
//! Harvested nectar pollinates (`flower_visits_per_trip` visits per worker).
//! Every transfer is booked in the ledgers; the colony's pools (workers,
//! brood, store, loads) are part of the stocks.

use std::collections::VecDeque;

use physics_world::{SurfaceKind, World};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

use crate::carcass::Remains;
use crate::fields::Fields;
use crate::food::Pantry;
use crate::ledger::{Flux, Ledgers};
use crate::species::{ActivityPattern, ColonyParams, Locomotion, Species, SpeciesCatalog};
use crate::units::{ANIMAL_NUTRIENT_G_PER_KG, SYNTHESIS_OVERHEAD, TISSUE_KJ_PER_KG, TimeScale, basal_metabolic_w};
use crate::vegetation::FLOOD_DEPTH_M;

mod forage;

use forage::departure;

/// Largest number of colonies of one species (swarming stops above it).
pub const MAX_COLONIES_PER_SPECIES: usize = 500;
/// Largest displacement of a squad per tick (cells).
const MAX_SQUAD_STEP_CELLS: f32 = 8.0;
/// Days of colony metabolism kept in the store before the queen lays.
const RESERVE_DAYS: f64 = 2.0;

/// What a squad is doing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForagerState {
    /// Looking for food.
    Searching,
    /// Flying to a patch reported at the nest.
    Recruited(Vec2),
    /// Carrying food home.
    Returning,
}

/// A forager squad (`workers_per_forager` workers moving together).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Forager {
    /// Stable id (random streams).
    pub id: u64,
    /// Colony id.
    pub colony: u64,
    /// Position (cells).
    pub position: Vec2,
    /// Heading (radians).
    pub heading: f32,
    /// What it is doing.
    pub state: ForagerState,
    /// Gross energy carried (kJ).
    pub load_kj: f64,
    /// Nutrients carried (g).
    pub load_g: f64,
    /// Assimilation efficiency of the carried food.
    pub load_assimilation: f32,
    /// Where the load came from and how good the patch was.
    pub patch: Option<(Vec2, f32)>,
}

/// One colony.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Colony {
    /// Stable id.
    pub id: u64,
    /// Species index.
    pub species: u16,
    /// Nest position (cells).
    pub nest: Vec2,
    /// Adult workers (including the foraging squads).
    pub workers: f64,
    /// Brood cohorts `(tick of emergence, individuals)`, oldest first.
    pub brood: VecDeque<(u64, f64)>,
    /// Food store (kJ).
    pub store_kj: f64,
    /// Nutrients of the store (g).
    pub store_g: f64,
    /// Best patch reported by returning foragers and its quality.
    pub best_patch: Option<(Vec2, f32)>,
    /// Tick of foundation.
    pub founded: u64,
}

impl Colony {
    /// Developing brood.
    pub fn brood_count(&self) -> f64 {
        self.brood.iter().map(|(_, n)| n).sum()
    }
}

/// Colonies, their squads and the trail pheromone of the map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Colonies {
    /// Colonies in id order.
    pub colonies: Vec<Colony>,
    /// Squads in creation order.
    pub foragers: Vec<Forager>,
    /// Trail pheromone per cell.
    pub pheromone: Vec<f32>,
    next_id: u64,
    width: usize,
    height: usize,
}

/// Inputs of one colony step.
pub(crate) struct ColonyStep<'a> {
    pub catalog: &'a SpeciesCatalog,
    pub scale: TimeScale,
    pub world: &'a World,
    pub tick: u64,
    pub seed: u64,
    /// Pheromone half-life (hours).
    pub pheromone_half_life_h: f32,
    /// Fraction of pheromone exchanged with the 4 neighbours per tick.
    pub pheromone_diffusion: f32,
}

/// Tissue energy (kJ) and nutrients (g) of one worker.
fn worker_tissue(species: &Species) -> (f64, f64) {
    let m = f64::from(species.body.mass_kg);
    (m * TISSUE_KJ_PER_KG, m * f64::from(ANIMAL_NUTRIENT_G_PER_KG))
}

/// Whether workers of `species` can walk on (or fly over) a cell.
fn passable(species: &Species, world: &World, p: Vec2) -> bool {
    let Some((x, y)) = world.cell_at(p) else { return false };
    if species.can(Locomotion::Fly) {
        return true;
    }
    let i = y * world.width() + x;
    world.surface().as_slice()[i] == SurfaceKind::Land && world.water_depth().as_slice()[i] <= FLOOD_DEPTH_M
}

impl Colonies {
    /// No colonies on a `width × height` map.
    pub fn new(width: usize, height: usize) -> Self {
        Self { pheromone: vec![0.0; width * height], width, height, ..Self::default() }
    }

    /// Whether the data match a map and a catalogue.
    pub(crate) fn is_consistent(&self, catalog: &SpeciesCatalog, width: usize, height: usize) -> bool {
        let ok_colony = |c: &Colony| {
            catalog.animals().get(usize::from(c.species)).is_some_and(|s| s.representation.colony().is_some())
                && c.workers.is_finite()
                && c.workers >= 0.0
                && c.store_kj.is_finite()
                && c.store_g.is_finite()
                && c.id < self.next_id
        };
        self.width == width
            && self.height == height
            && self.pheromone.len() == width * height
            && self.pheromone.iter().all(|p| p.is_finite() && *p >= 0.0)
            && self.colonies.iter().all(ok_colony)
            && self.colonies.windows(2).all(|w| w[0].id < w[1].id)
            && self.foragers.iter().all(|f| self.colonies.iter().any(|c| c.id == f.colony))
    }

    /// Founds a colony of `species` at `nest` with the species' founding
    /// workers and a store of two days of their metabolism at 20 °C.
    /// Returns its id and its energy and nutrients (for the ledgers).
    pub(crate) fn found(&mut self, species_index: u16, species: &Species, nest: Vec2, tick: u64) -> (u64, f64, f64) {
        let Some(p) = species.representation.colony() else { return (0, 0.0, 0.0) };
        let (e, n) = worker_tissue(species);
        let workers = f64::from(p.founding_workers);
        let day_kj = basal_metabolic_w(f64::from(species.body.mass_kg), false, 20.0) * 86.4;
        let store_kj = workers * day_kj * f64::from(species.metabolism.field_multiplier) * 2.0;
        let store_g = workers * n * 0.1;
        let id = self.next_id;
        self.next_id += 1;
        self.colonies.push(Colony {
            id,
            species: species_index,
            nest,
            workers,
            brood: VecDeque::new(),
            store_kj,
            store_g,
            best_patch: None,
            founded: tick,
        });
        (id, workers * e + store_kj, workers * n + store_g)
    }

    /// Energy (kJ) and nutrients (g) of every colony (workers, brood,
    /// store and the loads of their squads).
    pub fn stock(&self, catalog: &SpeciesCatalog) -> (f64, f64) {
        let mut stock = (0.0, 0.0);
        for c in &self.colonies {
            let (e, n) = worker_tissue(&catalog.animals()[usize::from(c.species)]);
            let bodies = c.workers + c.brood_count();
            stock.0 += bodies * e + c.store_kj;
            stock.1 += bodies * n + c.store_g;
        }
        for f in &self.foragers {
            stock.0 += f.load_kj;
            stock.1 += f.load_g;
        }
        stock
    }

    /// Removes colonies (and their squads) by id.
    pub(crate) fn remove(&mut self, ids: &[u64]) {
        self.colonies.retain(|c| !ids.contains(&c.id));
        self.foragers.retain(|f| !ids.contains(&f.colony));
    }

    /// Workers of every colony of a species.
    pub fn workers(&self, species: u16) -> f64 {
        self.colonies.iter().filter(|c| c.species == species).map(|c| c.workers).sum()
    }

    /// Number of colonies of a species.
    pub fn count(&self, species: u16) -> usize {
        self.colonies.iter().filter(|c| c.species == species).count()
    }

    /// One tick of every colony, then of every squad and of the pheromone.
    pub(crate) fn step(
        &mut self,
        step: &ColonyStep<'_>,
        pantry: &mut Pantry<'_>,
        fields: &mut Fields,
        remains: &mut Remains,
        ledgers: &mut Ledgers,
    ) {
        if self.colonies.is_empty() && self.pheromone.iter().all(|p| *p == 0.0) {
            return;
        }
        let area = f64::from(step.scale.cell_area_m2());
        let mut swarms = Vec::new();
        for c in &mut self.colonies {
            let species = &step.catalog.animals()[usize::from(c.species)];
            let Some(p) = species.representation.colony() else { continue };
            if let Some(nest) = colony_tick(c, species, p, step, pantry, ledgers, area) {
                swarms.push((c.id, nest));
            }
        }
        self.split(step, &swarms);
        self.dissolve(step.catalog, step.world, pantry, ledgers, area);
        self.manage_squads(step);
        self.move_squads(step, pantry, fields, remains, ledgers, area);
        if self.colonies.iter().any(|c| {
            step.catalog.animals()[usize::from(c.species)].representation.colony().is_some_and(|p| p.pheromone)
        }) || self.pheromone.iter().any(|p| *p > 0.0)
        {
            self.pheromone_step(step);
        }
    }

    /// Splits swarming colonies: half the workers and store found a colony at `nest`.
    fn split(&mut self, step: &ColonyStep<'_>, swarms: &[(u64, Vec2)]) {
        for &(id, nest) in swarms {
            let Some(index) = self.colonies.iter().position(|c| c.id == id) else { continue };
            let species = self.colonies[index].species;
            if self.count(species) >= MAX_COLONIES_PER_SPECIES {
                continue;
            }
            let parent = &mut self.colonies[index];
            let (workers, store_kj, store_g) = (parent.workers / 2.0, parent.store_kj / 2.0, parent.store_g / 2.0);
            parent.workers -= workers;
            parent.store_kj -= store_kj;
            parent.store_g -= store_g;
            let new_id = self.next_id;
            self.next_id += 1;
            self.colonies.push(Colony {
                id: new_id,
                species,
                nest,
                workers,
                brood: VecDeque::new(),
                store_kj,
                store_g,
                best_patch: None,
                founded: step.tick,
            });
        }
    }

    /// Removes colonies without workers or brood; their store goes to the litter.
    fn dissolve(
        &mut self,
        catalog: &SpeciesCatalog,
        world: &World,
        pantry: &mut Pantry<'_>,
        ledgers: &mut Ledgers,
        area: f64,
    ) {
        let mut dead = Vec::new();
        for c in &self.colonies {
            if c.workers < 1.0 && c.brood.is_empty() {
                dead.push(c.id);
            }
        }
        if dead.is_empty() {
            return;
        }
        for c in self.colonies.iter().filter(|c| dead.contains(&c.id)) {
            let (e, n) = worker_tissue(&catalog.animals()[usize::from(c.species)]);
            let (kj, g) = (c.workers.max(0.0) * e + c.store_kj.max(0.0), c.workers.max(0.0) * n + c.store_g.max(0.0));
            let cell = cell_of(world, c.nest);
            pantry.detritus.add_litter_energy(cell, kj / area, (g / area) as f32);
            ledgers.book(Flux::AnimalMortality, kj, g);
        }
        for f in self.foragers.iter().filter(|f| dead.contains(&f.colony)) {
            let cell = cell_of(world, f.position);
            pantry.detritus.add_litter_energy(cell, f.load_kj / area, (f.load_g / area) as f32);
            ledgers.book(Flux::AnimalMortality, f.load_kj, f.load_g);
        }
        self.colonies.retain(|c| !dead.contains(&c.id));
        self.foragers.retain(|f| !dead.contains(&f.colony));
    }

    /// Adds or retires squads to match each colony's forager target.
    fn manage_squads(&mut self, step: &ColonyStep<'_>) {
        for c in &self.colonies {
            let species = &step.catalog.animals()[usize::from(c.species)];
            let Some(p) = species.representation.colony() else { continue };
            let active = foraging_weather(species, step, c.nest);
            let target = if active {
                ((c.workers * f64::from(p.forager_fraction) / f64::from(p.workers_per_forager)).floor() as usize)
                    .min(usize::from(p.max_foragers))
            } else {
                0
            };
            let current = self.foragers.iter().filter(|f| f.colony == c.id).count();
            if current < target {
                for _ in current..target {
                    let id = self.next_id;
                    self.next_id += 1;
                    let mut rng = crate::rng::stream(step.seed, crate::rng::COLONY, step.tick, id);
                    let range = step.scale.cells(p.foraging_range_m).max(1.0);
                    let recruited = departure(species, p, c, range, step.world, &mut rng);
                    self.foragers.push(Forager {
                        id,
                        colony: c.id,
                        position: c.nest,
                        heading: rng.f32() * std::f32::consts::TAU,
                        state: recruited,
                        load_kj: 0.0,
                        load_g: 0.0,
                        load_assimilation: 0.0,
                        patch: None,
                    });
                }
            } else if current > target {
                // Unloaded squads at the nest retire first (newest first).
                let mut excess = current - target;
                let nest = c.nest;
                let colony = c.id;
                for f in self.foragers.iter_mut().rev() {
                    if excess == 0 {
                        break;
                    }
                    if f.colony == colony && f.load_kj <= 0.0 && f.position.distance(nest) < 0.5 {
                        f.colony = u64::MAX;
                        excess -= 1;
                    }
                }
                // The others head home.
                for f in self.foragers.iter_mut().filter(|f| f.colony == colony) {
                    if f.state != ForagerState::Returning && excess > 0 {
                        f.state = ForagerState::Returning;
                        excess -= 1;
                    }
                }
            }
        }
        self.foragers.retain(|f| f.colony != u64::MAX);
    }

    /// Evaporation and diffusion of the trail pheromone.
    fn pheromone_step(&mut self, step: &ColonyStep<'_>) {
        let (w, h) = (self.width, self.height);
        let dt_h = step.scale.dt_hours();
        let keep = (1.0 - std::f32::consts::LN_2 * dt_h / step.pheromone_half_life_h.max(1e-3)).clamp(0.0, 1.0);
        let d = step.pheromone_diffusion.clamp(0.0, 0.25);
        let old: Vec<f32> = self.pheromone.iter().map(|p| p * keep).collect();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let mut v = old[i];
                for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                    if nx < w && ny < h {
                        let j = ny * w + nx;
                        v += d * (old[j] - old[i]);
                    }
                }
                self.pheromone[i] = if v < 1e-6 { 0.0 } else { v };
            }
        }
    }
}

/// Cell index of a position (clamped into the map).
fn cell_of(world: &World, p: Vec2) -> usize {
    let w = world.width();
    let x = (p.x.max(0.0) as usize).min(w - 1);
    let y = (p.y.max(0.0) as usize).min(world.height() - 1);
    y * w + x
}

/// Whether the weather at the nest lets the species forage.
fn foraging_weather(species: &Species, step: &ColonyStep<'_>, nest: Vec2) -> bool {
    let air = step.world.temperature().as_slice()[cell_of(step.world, nest)];
    let light = match species.behaviour.activity {
        ActivityPattern::Diurnal => step.world.daylight() > 0.05,
        ActivityPattern::Nocturnal => step.world.daylight() <= 0.05,
        ActivityPattern::Crepuscular | ActivityPattern::Cathemeral => true,
    };
    light && air >= species.climate.preferred_c.0
}

/// Displacement of a squad per tick (cells).
fn squad_speed(species: &Species, p: &ColonyParams, scale: &TimeScale) -> f32 {
    let sprint = species.movement.sprint_speed_m_s.unwrap_or(1.0);
    let cruise = sprint * species.movement.cruise_fraction;
    let seconds = scale.dt_hours() * 3600.0 * p.travel_fraction;
    (cruise * seconds / scale.cell_size_m).min(MAX_SQUAD_STEP_CELLS)
}

/// Metabolism, mortality, laying, emergence of one colony; returns a nest
/// for a swarm if the colony splits this tick.
fn colony_tick(
    c: &mut Colony,
    species: &Species,
    p: &ColonyParams,
    step: &ColonyStep<'_>,
    pantry: &mut Pantry<'_>,
    ledgers: &mut Ledgers,
    area: f64,
) -> Option<Vec2> {
    let (e, n) = worker_tissue(species);
    let m = f64::from(species.body.mass_kg);
    let dt_days = f64::from(step.scale.dt_days());
    let cell = cell_of(step.world, c.nest);
    let air = step.world.temperature().as_slice()[cell];
    let t_nest = p.nest_temperature_c.unwrap_or_else(|| air.clamp(4.0, 30.0));
    let active = foraging_weather(species, step, c.nest);
    let day_kj = basal_metabolic_w(m, false, f64::from(t_nest)) * 86.4;
    let field = if active { f64::from(species.metabolism.field_multiplier) } else { 1.0 };
    let cost = c.workers * day_kj * field * dt_days;
    let paid = cost.min(c.store_kj.max(0.0));
    c.store_kj -= paid;
    let deficit = cost - paid;
    let burned = (deficit / e).min(c.workers);
    c.workers -= burned;
    // Starving workers are eaten by the colony: their nutrients stay in the store.
    c.store_g += burned * n;
    ledgers.book(Flux::AnimalRespiration, paid + burned * e, 0.0);
    // Background mortality of workers.
    let lifespan_days = (f64::from(p.worker_lifespan_days) * step.scale.year_compression()).max(1e-3);
    let deaths = (c.workers * dt_days / lifespan_days).min(c.workers);
    if deaths > 0.0 {
        c.workers -= deaths;
        pantry.detritus.add_litter_energy(cell, deaths * e / area, (deaths * n / area) as f32);
        ledgers.book(Flux::AnimalMortality, deaths * e, deaths * n);
    }
    // Emergence.
    while c.brood.front().is_some_and(|(due, _)| *due <= step.tick) {
        if let Some((_, count)) = c.brood.pop_front() {
            c.workers += count;
        }
    }
    // Laying (rates per real day, compressed with the calendar like every life history).
    let room = f64::from(p.max_workers) - c.workers - c.brood_count();
    if active && room > 0.0 {
        let reserve = c.workers * day_kj * f64::from(species.metabolism.field_multiplier) * RESERVE_DAYS;
        let egg_kj = e * (1.0 + f64::from(SYNTHESIS_OVERHEAD));
        let rate = f64::from(p.max_eggs_per_day) / step.scale.year_compression() * dt_days;
        let eggs = rate.min(room).min(((c.store_kj - reserve) / egg_kj).max(0.0)).min((c.store_g / n).max(0.0));
        if eggs > 0.0 {
            c.store_kj -= eggs * egg_kj;
            c.store_g -= eggs * n;
            ledgers.book(Flux::ReproductiveInvestment, eggs * e, eggs * n);
            ledgers.book(Flux::AnimalRespiration, eggs * e * f64::from(SYNTHESIS_OVERHEAD), 0.0);
            let development = step.scale.life_ticks_from_days(p.development_days).ceil() as u64;
            let due = step.tick + development.max(1);
            match c.brood.back_mut() {
                Some((d, count)) if *d == due => *count += eggs,
                _ => c.brood.push_back((due, eggs)),
            }
        }
    }
    // Swarming.
    let season = step.world.time().season;
    let breeding =
        species.reproduction.breeding_seasons.is_empty() || species.reproduction.breeding_seasons.contains(&season);
    let full = c.store_kj > c.workers * e;
    if breeding && active && c.workers >= f64::from(p.swarm_workers) && full {
        let mut rng = crate::rng::stream(step.seed, crate::rng::COLONY, step.tick, c.id ^ 0x5A4D);
        let range = step.scale.cells(p.foraging_range_m).max(2.0);
        for _ in 0..16 {
            let spot = c.nest + rng.in_disc(range);
            if passable(species, step.world, spot)
                && step.world.cell_at(spot).is_some_and(|(x, y)| {
                    step.world.surface().as_slice()[y * step.world.width() + x] == SurfaceKind::Land
                })
            {
                return Some(spot);
            }
        }
    }
    None
}
