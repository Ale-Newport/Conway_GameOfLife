//! The per-tick pipeline.
//!
//! Ecological rules, in this order:
//!
//! 1. queued (`NextTick`) parameter changes are applied;
//! 2. the canopy is recomputed if trees changed; fuel and canopy are written
//!    into the world, which steps (weather, hydrology, fire...);
//! 3. fire and wind effects are applied to plants, trees and the soil; the
//!    world's leaching and volcanic deposition are booked;
//! 4. hourly: water map, cover and daily-mean temperature caches, vegetation
//!    growth, decomposition and fungi, carcass decay, deposition, density
//!    fields of tiny organisms;
//! 5. daily: seed banks, fungal spread, trees (growth, mortality, seeding);
//! 6. colonies: workers, brood, swarming, forager squads, pheromone;
//! 7. animals: index → pack leaders → perceive + decide (parallel, pure) →
//!    move with altitude (parallel, independent) → re-index → interact
//!    (claims, deterministic resolution) → attacks, fights, meals, matings →
//!    disease draws (parallel, pure) → physiology → disease outcomes →
//!    parental care → births, clutches → deaths into carcasses → commit.
//!
//! Scheduled events of the scenario run right after the queued parameter
//! changes.
//!
//! The NetLogo variant steps the world and then runs [`crate::netlogo`].

use std::collections::BTreeSet;

use organism_engine::perception;
use organism_engine::{AgentId, Bounds, MotionLimits};
use physics_world::{SurfaceKind, World};
use rayon::prelude::*;
use simulation_core::{EventSeverity, ParamValue, SimEvent};

use crate::animal::decide::{self, Decision};
use crate::animal::genetics::Phenotype;
use crate::animal::interact::interact;
use crate::animal::perceive::Ctx;
use crate::animal::physiology::{Deaths, Motion};
use crate::animal::{Action, Animal, OrganismRef};
use crate::colony::ColonyStep;
use crate::config::Rules;
use crate::environment::{Disturbance, apply_disturbances, in_hostile_water, passable, write_fuel_and_canopy};
use crate::error::EcosystemError;
use crate::fields::FieldStep;
use crate::food::Pantry;
use crate::fungi::DecayStep;
use crate::ledger::Flux;
use crate::species::AnimalTraits;
use crate::state::{EcoState, Runtime, TRAIL_LENGTH};
use crate::trees::{Canopy, TreeStep, daily_update};
use crate::vegetation::GrowthStep;

/// Largest displacement (cells) of one movement sub-step.
const SUBSTEP_CELLS: f32 = 0.5;

impl EcoState {
    /// Read-only context for perception and interactions.
    pub(crate) fn ctx<'a>(&'a self, rt: &'a Runtime) -> Ctx<'a> {
        self.ctx_with(rt, &rt.grid, &rt.carcass_grid, &rt.clutch_index)
    }

    /// Context with explicit spatial indices (slot = store / carcass /
    /// clutch index).
    pub(crate) fn ctx_with<'a>(
        &'a self,
        rt: &'a Runtime,
        grid: &'a simulation_core::SpatialGrid,
        carcass_grid: &'a simulation_core::SpatialGrid,
        clutch_index: &'a crate::carcass::ClutchIndex,
    ) -> Ctx<'a> {
        Ctx {
            catalog: &self.catalog,
            traits: &rt.traits,
            schema: &rt.schema,
            scale: rt.scale,
            world: &self.world,
            env: &self.env,
            vegetation: &self.vegetation,
            fungi: &self.fungi,
            detritus: &self.detritus,
            canopy: &self.canopy,
            fields: &self.fields,
            epi: &rt.epi,
            leaders: &rt.leaders,
            animals: &self.animals,
            grid,
            remains: &self.remains,
            carcass_grid,
            clutch_index,
            tick: self.tick,
            seed: self.seed,
            hour: self.world.calendar().hour_of_day(self.tick) as f32,
            daylight: self.world.daylight(),
            bounds: self.bounds(),
        }
    }

    /// Advances one tick.
    pub(crate) fn step(&mut self, rt: &mut Runtime) -> Result<(), EcosystemError> {
        self.apply_pending(rt)?;
        self.ledgers.start_tick();
        self.counters.start_tick();
        self.run_schedule(rt);
        match self.config.rules {
            Rules::NetLogo(_) => {
                let report = self.world.step();
                self.counters.rain_mm = report.rain_total_mm;
                for event in report.events {
                    self.events.push(event);
                }
                self.netlogo_tick(rt)?;
            }
            Rules::Ecological => self.ecological_tick(rt)?,
        }
        self.update_follow();
        self.tick += 1;
        Ok(())
    }

    /// Applies queued parameter changes; rebuilds derived traits when needed.
    fn apply_pending(&mut self, rt: &mut Runtime) -> Result<(), EcosystemError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        let mut rebuild = false;
        for (key, value) in pending {
            let value = ParamValue::from(&value);
            crate::params::write(&key, &value, &mut self.config, &mut self.params)
                .map_err(|e| EcosystemError::config(&key, e.to_string()))?;
            rebuild |= key == "motion_seconds";
        }
        if rebuild {
            *rt = Runtime::new(self);
        }
        Ok(())
    }

    /// Queues a `NextTick` parameter change.
    pub(crate) fn queue_param(&mut self, key: &str, value: ParamValue) {
        self.pending.push((key.to_string(), crate::params::StoredValue::from(&value)));
    }

    fn ecological_tick(&mut self, rt: &mut Runtime) -> Result<(), EcosystemError> {
        let tick = self.tick;
        let hours = self.world.calendar().hours_started(tick);
        let new_day = tick.is_multiple_of(u64::from(self.world.calendar().ticks_per_day()));
        let (w, h) = (self.world.width(), self.world.height());
        if self.canopy_dirty {
            self.canopy = Canopy::compute(&self.trees, &self.catalog, w, h, &rt.scale);
            self.canopy_dirty = false;
        }
        write_fuel_and_canopy(
            &mut self.world,
            &self.catalog,
            &self.vegetation,
            &self.detritus,
            &self.canopy,
            &mut self.env,
        );
        // With a closed budget, remember the soil nutrients of every cell so
        // that leaching can be returned where it happened.
        let soil_before = self.params.leaching_retained().then(|| soil_nutrients(&self.world));
        let report = self.world.step();
        self.book_world_budget(soil_before.as_deref());
        self.counters.rain_mm = report.rain_total_mm;
        for event in &report.events {
            self.events.push(event.clone());
        }
        let disturbance = Disturbance {
            catalog: &self.catalog,
            tick,
            seed: self.seed,
            cell_area: self.world.config().cell_area_m2(),
            volatilisation: self.params.volatilisation(),
            canopy: &self.canopy,
        };
        let out = apply_disturbances(
            &report,
            &disturbance,
            &mut self.world,
            &self.env,
            &mut self.vegetation,
            &mut self.detritus,
            &mut self.trees,
            &mut self.ledgers,
        );
        self.counters.burned_kg = out.burned_kg;
        self.burn_fields();
        if !report.fuel_consumed.is_empty() || !report.wind_damage.is_empty() {
            self.canopy_dirty = true;
        }
        if out.trees_burned + out.trees_thrown > 0 {
            self.counters.trees_died += u64::from(out.trees_burned + out.trees_thrown);
            let message = format!("{} trees killed by fire, {} thrown by wind", out.trees_burned, out.trees_thrown);
            self.log(SimEvent::new(tick, "tree_damage", message).severity(EventSeverity::Info));
        }
        self.env.update_fire(&self.world);
        if report.terrain_changed {
            self.env.update_water(&self.world);
        }
        if hours > 0 {
            self.hourly(hours as f32);
        }
        if new_day {
            self.daily(rt);
        }
        self.colonies_tick(rt);
        self.animals_tick(rt)
    }

    /// Insects and worms in burning cells die (fliers escape more often;
    /// aquatic organisms are not touched); their bodies go to the litter.
    fn burn_fields(&mut self) {
        if self.fields.is_empty() || self.world.burning_cells().is_empty() {
            return;
        }
        let area = self.cell_area();
        let catalog = &self.catalog;
        let burning: Vec<u32> = self.world.burning_cells().to_vec();
        for cell in burning {
            let cell = cell as usize;
            let (kj, g) = self.fields.kill_in_cell(cell, |s| {
                let species = &catalog.animals()[usize::from(s)];
                if species.habitat.aquatic {
                    0.0
                } else if species.can(crate::species::Locomotion::Fly) {
                    0.3
                } else {
                    0.8
                }
            });
            if kj > 0.0 || g > 0.0 {
                self.detritus.add_litter_energy(cell, kj, g as f32);
                self.ledgers.book(Flux::AnimalMortality, kj * area, g * area);
            }
        }
    }

    /// One tick of the colonies and their squads.
    fn colonies_tick(&mut self, rt: &Runtime) {
        if self.colonies.colonies.is_empty() && self.colonies.foragers.is_empty() {
            return;
        }
        let step = ColonyStep {
            catalog: &self.catalog,
            scale: rt.scale,
            world: &self.world,
            tick: self.tick,
            seed: self.seed,
            pheromone_half_life_h: self.params.pheromone_half_life_h,
            pheromone_diffusion: self.params.pheromone_diffusion,
        };
        let mut pantry = Pantry {
            catalog: &self.catalog,
            vegetation: &mut self.vegetation,
            detritus: &mut self.detritus,
            fungi: &mut self.fungi,
        };
        self.colonies.step(&step, &mut pantry, &mut self.fields, &mut self.remains, &mut self.ledgers);
    }

    /// Books the world's own nutrient fluxes since the last call: volcanic
    /// ash (input) and leaching. Leached nutrients are an output, unless the
    /// budget retains them (`soil_before` is then the mineral + organic
    /// nutrients of every cell before the world step): they are returned to
    /// the cells in proportion to each cell's decrease during the step.
    fn book_world_budget(&mut self, soil_before: Option<&[f64]>) {
        let budget = self.world.nutrient_budget();
        let (seen_leached, seen_deposited) = self.world_budget_seen;
        let (leached, deposited) = (budget.leached_g - seen_leached, budget.deposited_g - seen_deposited);
        self.world_budget_seen = (budget.leached_g, budget.deposited_g);
        self.ledgers.book(Flux::VolcanicDeposition, 0.0, deposited);
        let lost = match soil_before {
            Some(before) if leached > 0.0 => self.return_leached(before, leached),
            _ => leached,
        };
        self.ledgers.book(Flux::LeachingRetained, 0.0, leached - lost);
        self.ledgers.book(Flux::Leaching, 0.0, lost);
    }

    /// Returns `leached_g` grams to the soil, weighted by each cell's loss of
    /// nutrients during the world step (uniformly over non-ocean cells if
    /// no cell lost any). Returns the grams the soil refused (never for real
    /// amounts: a single addition is capped at 10⁶ g/m²), which stay lost.
    fn return_leached(&mut self, before: &[f64], leached_g: f64) -> f64 {
        let after = soil_nutrients(&self.world);
        let mut weights: Vec<f64> = before.iter().zip(&after).map(|(b, a)| (b - a).max(0.0)).collect();
        if weights.iter().sum::<f64>() <= 0.0 {
            let surface = self.world.surface().as_slice();
            weights = surface.iter().map(|s| if *s == SurfaceKind::Ocean { 0.0 } else { 1.0 }).collect();
        }
        let total: f64 = weights.iter().sum();
        if total <= 0.0 {
            return leached_g;
        }
        let area = self.cell_area();
        let w = self.world.width();
        let mut refused = 0.0;
        for (cell, weight) in weights.iter().enumerate() {
            if *weight > 0.0 {
                let g_m2 = (leached_g * weight / total / area) as f32;
                let added = self.world.add_nutrients(cell % w, cell / w, g_m2).unwrap_or(0.0);
                refused += f64::from(g_m2 - added) * area;
            }
        }
        refused
    }

    fn hourly(&mut self, hours: f32) {
        let dt_days = hours / 24.0;
        let area = self.world.config().cell_area_m2();
        self.env.update_water(&self.world);
        self.env.update_day_temperature(&self.world, hours);
        self.env.update_cover(&self.vegetation, &self.canopy);
        let n = self.world.width() * self.world.height();
        let boost: Vec<f32> =
            (0..n).map(|c| self.fungi.uptake_boost(self.catalog.fungi(), c, self.params.mycorrhizal_boost)).collect();
        let growth = GrowthStep {
            catalog: &self.catalog,
            myco_boost: &boost,
            dt_days,
            growth_multiplier: self.params.plant_growth,
            cell_area: area,
        };
        self.vegetation.grow(&growth, &mut self.world, &mut self.detritus, &mut self.ledgers);
        let host: Vec<f32> = (0..n)
            .map(|c| self.vegetation.biomass(c, crate::species::PlantLayer::Shrubs) + self.canopy.leaf_kg_m2[c])
            .collect();
        let decay = DecayStep {
            params: self.catalog.fungi(),
            dt_days,
            multiplier: self.params.decomposition,
            cell_area: area,
            host_kg_m2: &host,
        };
        self.fungi.decompose(&decay, &mut self.world, &mut self.detritus, &mut self.ledgers);
        if !self.fields.is_empty() {
            let step = FieldStep { catalog: &self.catalog, dt_days, cell_area: area };
            let mut pantry = Pantry {
                catalog: &self.catalog,
                vegetation: &mut self.vegetation,
                detritus: &mut self.detritus,
                fungi: &mut self.fungi,
            };
            self.fields.update(&step, &mut self.world, &mut pantry, &mut self.ledgers);
        }
        let width = self.world.width();
        let height = self.world.height();
        let temperature = self.world.temperature().as_slice().to_vec();
        self.remains.decay(
            dt_days,
            area,
            |p| {
                let x = (p.x.max(0.0) as usize).min(width - 1);
                let y = (p.y.max(0.0) as usize).min(height - 1);
                y * width + x
            },
            |c| temperature[c],
            &mut self.detritus,
            &mut self.ledgers,
        );
        let deposition = self.params.deposition();
        if deposition > 0.0 {
            let per_cell = deposition / (self.world.calendar().days_per_year() as f32 * 24.0) * hours;
            let mut added = 0.0f64;
            for c in 0..n {
                if self.world.surface().as_slice()[c] == physics_world::SurfaceKind::Land {
                    added += f64::from(self.world.add_nutrients(c % width, c / width, per_cell).unwrap_or(0.0));
                }
            }
            self.ledgers.book(Flux::Deposition, 0.0, added * area);
        }
    }

    fn daily(&mut self, rt: &Runtime) {
        let (w, h) = (self.world.width(), self.world.height());
        self.vegetation.update_seed_banks(&self.catalog, &self.world, 1.0, self.cell_area());
        self.fungi.spread(self.catalog.fungi(), &self.world, 1.0);
        if self.canopy_dirty {
            self.canopy = Canopy::compute(&self.trees, &self.catalog, w, h, &rt.scale);
        }
        let time = self.world.time();
        let days_per_season = self.world.calendar().days_per_season().max(1);
        let n = w * h;
        let boost: Vec<f32> =
            (0..n).map(|c| self.fungi.uptake_boost(self.catalog.fungi(), c, self.params.mycorrhizal_boost)).collect();
        let step = TreeStep {
            catalog: &self.catalog,
            scale: rt.scale,
            tick: self.tick,
            seed: self.seed,
            growth_multiplier: self.params.tree_growth,
            season: time.season,
            season_fraction: (time.day_of_season as f32 + 0.5) / days_per_season as f32,
            myco_boost: &boost,
            day_temperature: &self.env.day_temperature,
            max_trees_per_cell: self.params.max_trees_per_cell,
            seeding: (self.trees.len() as u32) < self.params.max_trees,
        };
        let report = daily_update(
            &mut self.trees,
            &self.canopy,
            &step,
            &mut self.world,
            &self.vegetation,
            &mut self.detritus,
            &mut self.ledgers,
        );
        self.counters.trees_germinated += u64::from(report.germinated);
        self.counters.trees_died += u64::from(report.died);
        self.canopy = Canopy::compute(&self.trees, &self.catalog, w, h, &rt.scale);
        self.canopy_dirty = false;
    }

    fn index(&self, rt: &mut Runtime) -> Result<(), EcosystemError> {
        perception::index_agents(&mut rt.grid, &self.animals, |a| a.kin.position)?;
        rt.carcass_grid.rebuild(self.remains.carcasses.iter().enumerate().map(|(i, c)| (i as u32, c.position)));
        rt.clutch_index.rebuild(&self.remains.clutches);
        Ok(())
    }

    fn animals_tick(&mut self, rt: &mut Runtime) -> Result<(), EcosystemError> {
        self.index(rt)?;
        rt.leaders = crate::animal::social::pack_leaders(&self.animals, &self.catalog);
        let decisions: Vec<Decision> = {
            let ctx = self.ctx(rt);
            (0..self.animals.len()).into_par_iter().map(|i| decide::decide(&ctx, i)).collect()
        };
        self.record_decisions(&decisions);
        let motion = {
            let world = &self.world;
            let traits = &rt.traits;
            let schema = &rt.schema;
            let bounds = self.bounds();
            let moved: Vec<(f32, bool)> = self
                .animals
                .as_mut_slice()
                .par_iter_mut()
                .zip(decisions.par_iter())
                .map(|(a, d)| {
                    move_animal(a, d, world, &traits[usize::from(a.species)], &Phenotype::of(&a.genome, schema), bounds)
                })
                .collect();
            Motion { distance: moved.iter().map(|m| m.0).collect(), sprint: moved.iter().map(|m| m.1).collect() }
        };
        for (a, d) in self.animals.as_mut_slice().iter_mut().zip(&motion.distance) {
            a.stats.distance_cells += d;
        }
        self.index(rt)?;
        let interactions = {
            let ctx = self.ctx(rt);
            interact(&ctx, self.params.predation_efficiency)
        };
        let mut deaths = Deaths::new();
        self.apply_attacks(&interactions.attacks, &mut deaths);
        self.apply_fights(&interactions.fights, &mut deaths);
        self.apply_meals(rt, &interactions.meals);
        self.apply_matings(rt, &interactions.matings);
        let drinkers: BTreeSet<AgentId> = interactions.drinkers.iter().copied().collect();
        let disease = crate::disease::transmission(
            &rt.epi,
            self.animals.as_slice(),
            self.animals.ids(),
            &rt.grid,
            &rt.scale,
            self.seed,
            self.tick,
        );
        let due = self.physiology(rt, &motion, &drinkers, &mut deaths);
        let ticks_per_day = f64::from(rt.scale.ticks_per_day);
        self.apply_disease(&rt.epi, &disease, &mut deaths, ticks_per_day);
        self.parental_care(rt);
        self.deliver(rt, &due)?;
        self.clutches(rt)?;
        self.bury(&deaths)?;
        self.animals.commit();
        Ok(())
    }

    /// Stores decisions in the animals and traces the followed animal's choices.
    fn record_decisions(&mut self, decisions: &[Decision]) {
        let tick = self.tick;
        let followed = match self.follow.followed {
            Some(OrganismRef::Animal(id)) => Some(AgentId(id)),
            _ => None,
        };
        let mut trace = None;
        let mut alarms = 0u32;
        for ((id, a), d) in self.animals.iter_mut().zip(decisions) {
            if a.action != d.action {
                if Some(id) == followed {
                    trace = Some(format!("{} → {}", a.action.key(), d.action.key()));
                }
                a.action = d.action;
                a.action_since = tick;
            }
            a.target = d.target;
            a.wander = d.wander;
            a.hidden = d.hidden;
            a.last_scores = d.scores;
            a.alarm = d.alarm;
            alarms += u32::from(d.alarm.is_some());
        }
        self.counters.alarms = alarms;
        if let (Some(message), Some(id)) = (trace, followed) {
            self.follow
                .log
                .push(SimEvent::new(tick, "decision", message).subject(id.get()).severity(EventSeverity::Trace));
        }
    }

    /// Extends the trail of the followed organism.
    fn update_follow(&mut self) {
        let position = match self.follow.followed {
            Some(OrganismRef::Animal(id)) => self.animals.get(AgentId(id)).map(|a| a.kin.position),
            Some(OrganismRef::Tree(id)) => self.trees.get(AgentId(id)).map(|t| t.position),
            None => None,
        };
        if let Some(p) = position {
            self.follow.trail.push_back(p);
            while self.follow.trail.len() > TRAIL_LENGTH {
                self.follow.trail.pop_front();
            }
        }
    }
}

/// Mineral + organic soil nutrients of every cell (g/m²).
fn soil_nutrients(world: &World) -> Vec<f64> {
    world
        .nutrients()
        .as_slice()
        .iter()
        .zip(world.organic_matter().as_slice())
        .map(|(n, o)| f64::from(*n) + f64::from(*o))
        .collect()
}

/// Integrates one animal's movement in sub-steps of at most half a cell.
/// Returns the distance moved (cells) and whether it sprinted.
fn move_animal(
    a: &mut Animal,
    d: &Decision,
    world: &World,
    traits: &AnimalTraits,
    pheno: &Phenotype,
    bounds: Bounds,
) -> (f32, bool) {
    if a.hidden || matches!(a.action, Action::Torpor | Action::Hibernate) {
        a.kin.velocity = simulation_core::glam::Vec2::ZERO;
        a.altitude_m = if traits.aquatic { a.altitude_m } else { 0.0 };
        return (0.0, false);
    }
    let limits = MotionLimits::new(d.max_speed, traits.accel_cells * pheno.speed);
    let start = a.kin.position;
    // An animal stranded in hostile water may cross water to get out.
    let stranded = in_hostile_water(world, traits, start);
    let steps = ((d.max_speed / SUBSTEP_CELLS).ceil() as u32).clamp(1, 16);
    let dt = 1.0 / steps as f32;
    for _ in 0..steps {
        a.kin.integrate_with(d.accel, dt, limits, bounds, |p| {
            if stranded { world.cell_at(p).is_some() } else { passable(world, traits, p) }
        });
    }
    a.altitude_m = altitude(a, d, world, traits);
    let distance = (a.kin.position - start).length();
    let wanted = d.max_speed > 0.05 && d.accel.length_squared() > 1e-6;
    a.stuck_ticks = if wanted && distance < 0.1 * d.max_speed { a.stuck_ticks.saturating_add(1) } else { 0 };
    (distance, d.sprint && distance > 0.0)
}

/// Height of an animal after its move (m). Fliers climb towards their
/// cruising altitude while travelling (wandering, migrating, fleeing,
/// defending, searching for prey or water) and descend to land for every
/// other action; a hunter stoops onto its prey when it sprints. Aquatic
/// animals report a negative altitude: half the water depth, at most 20 m
/// below the surface.
fn altitude(a: &Animal, d: &Decision, world: &World, traits: &AnimalTraits) -> f32 {
    let cell = world.cell_at(a.kin.position).map(|(x, y)| y * world.width() + x);
    let depth = cell.map_or(0.0, |i| {
        if world.surface().as_slice()[i] == SurfaceKind::Land && world.water_depth().as_slice()[i] < 0.3 {
            0.0
        } else {
            world.water_depth().as_slice()[i]
        }
    });
    if !traits.flies {
        return if traits.aquatic && depth > 0.0 { -(0.5 * depth).min(20.0) } else { 0.0 };
    }
    let cruise = traits.flight_altitude_m;
    let target = match d.action {
        Action::Wander | Action::Migrate | Action::Flee | Action::Defend => cruise,
        Action::Hunt | Action::Drink if !d.sprint => cruise,
        Action::SeekMate if d.max_speed > 0.0 && a.kin.speed() > 0.5 * d.max_speed => cruise,
        _ => 0.0,
    };
    // Fliers that cannot swim stay just above open water.
    let floor = if depth > 0.0 && !traits.swims { 1.0 } else { 0.0 };
    let target = target.max(floor);
    if d.action == Action::Hunt && d.sprint {
        return floor; // stoop
    }
    let rate = (0.5 * cruise).max(5.0);
    let h = a.altitude_m.max(0.0);
    if target > h { (h + rate).min(target) } else { (h - rate).max(target) }
}
