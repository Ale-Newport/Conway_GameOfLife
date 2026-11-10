//! External mutations of an ecosystem (`Simulation::apply`).

use organism_engine::AgentId;
use physics_world::{Disaster, SurfaceKind};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent};

use crate::animal::physiology::Deaths;
use crate::animal::{DeathCause, OrganismRef};
use crate::config::Rules;
use crate::error::EcosystemError;
use crate::ledger::Flux;
use crate::state::{EcoState, Runtime};
use crate::trees::{SEEDLING_HEIGHT_M, Tree, phenology, topple};
use crate::units::PLANT_KJ_PER_KG;

/// Largest number of animals spawned by one command.
pub const MAX_SPAWN: u32 = 1_000;

/// A command of the ecosystem model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EcosystemCommand {
    /// Adds adult or juvenile animals of a species within `radius` cells of a point.
    SpawnAnimals {
        /// Species id.
        species: String,
        /// Centre (cells).
        x: f32,
        /// Centre (cells).
        y: f32,
        /// Radius (cells).
        radius: f32,
        /// Number of animals (1–1000).
        count: u32,
    },
    /// Kills an animal (it becomes a carcass).
    KillAnimal {
        /// Animal id.
        id: u64,
    },
    /// Removes an animal from the world (its matter leaves the ecosystem).
    RemoveAnimal {
        /// Animal id.
        id: u64,
    },
    /// Plants a seedling (its nutrients come from the soil of the cell).
    PlantTree {
        /// Tree species id.
        species: String,
        /// Position (cells).
        x: f32,
        /// Position (cells).
        y: f32,
    },
    /// Fells a tree (its wood falls into the dead-wood pool).
    FellTree {
        /// Tree id.
        id: u64,
    },
    /// Sets a cell on fire (it needs dry fuel).
    Ignite {
        /// Column.
        x: usize,
        /// Row.
        y: usize,
    },
    /// Any intervention of the physics world (rain, drought, heatwave, storm,
    /// flood, earthquake, tornado, landslide, volcano...).
    Disaster(Disaster),
    /// Changes the season length; the world calendar is fixed during a run,
    /// so it takes effect at the next reset (same as the `days_per_season`
    /// parameter).
    SetSeasonLength {
        /// Days per season (1–3650).
        days_per_season: u32,
    },
    /// Selects an organism for inspection (`None` clears the selection).
    Select(Option<OrganismRef>),
    /// Follows an organism with the camera and records its decisions.
    Follow(Option<OrganismRef>),
    /// Infects random hosts of a species with one of the scenario's pathogens.
    Outbreak {
        /// Pathogen id.
        pathogen: String,
        /// Host species id.
        species: String,
        /// Number of animals to infect.
        count: u32,
    },
}

impl EcoState {
    /// Applies a command.
    pub(crate) fn apply_command(&mut self, rt: &mut Runtime, command: EcosystemCommand) -> Result<(), EcosystemError> {
        match command {
            EcosystemCommand::SpawnAnimals { species, x, y, radius, count } => {
                self.spawn_command(rt, &species, x, y, radius, count)
            }
            EcosystemCommand::KillAnimal { id } => {
                if self.animals.get(AgentId(id)).is_none() {
                    return Err(EcosystemError::UnknownOrganism(id));
                }
                if self.netlogo.is_some() {
                    let species = self.animals.get(AgentId(id)).map_or(0, |a| a.species);
                    self.animals.queue_despawn(AgentId(id));
                    self.counters.death(species, DeathCause::Killed);
                    self.genealogy.record_death(AgentId(id), self.tick)?;
                } else {
                    let mut deaths = Deaths::new();
                    deaths.insert(AgentId(id), (DeathCause::Killed, None));
                    self.bury(&deaths)?;
                }
                self.animals.commit();
                Ok(())
            }
            EcosystemCommand::RemoveAnimal { id } => {
                let a = self.animals.get(AgentId(id)).ok_or(EcosystemError::UnknownOrganism(id))?;
                self.ledgers.book(Flux::InterventionOut, a.energy_stock_kj(), a.nutrient_stock_g());
                self.animals.queue_despawn(AgentId(id));
                self.genealogy.record_death(AgentId(id), self.tick)?;
                self.animals.commit();
                self.log(SimEvent::new(self.tick, "removed", format!("#{id} removed")).subject(id));
                Ok(())
            }
            EcosystemCommand::PlantTree { species, x, y } => self.plant_tree(&species, x, y),
            EcosystemCommand::FellTree { id } => {
                let tree = self.trees.get_mut(AgentId(id)).ok_or(EcosystemError::UnknownOrganism(id))?;
                let sp = self.catalog.trees()[usize::from(tree.species)].clone();
                let w = self.world.width();
                let cell = tree.cell(w, self.world.height());
                let area = self.world.config().cell_area_m2();
                topple(tree, &sp, cell, area, &mut self.detritus, &mut self.ledgers);
                self.trees.queue_despawn(AgentId(id));
                self.trees.commit();
                self.canopy_dirty = true;
                Ok(())
            }
            EcosystemCommand::Ignite { x, y } => {
                if self.world.ignite(x, y)? {
                    Ok(())
                } else {
                    Err(EcosystemError::Command(format!("cell ({x}, {y}) has no dry fuel to burn")))
                }
            }
            EcosystemCommand::Disaster(disaster) => {
                let outcome = self.world.trigger(disaster)?;
                if outcome.terrain_changed || outcome.water_added_m3 > 0.0 {
                    self.env.update_water(&self.world);
                }
                Ok(())
            }
            EcosystemCommand::SetSeasonLength { days_per_season } => {
                if !(1..=3650).contains(&days_per_season) {
                    return Err(EcosystemError::Command(format!(
                        "{days_per_season} days per season is outside [1, 3650]"
                    )));
                }
                self.config.world.days_per_season = days_per_season;
                self.reset_point.0.world.days_per_season = days_per_season;
                let message = format!("season length set to {days_per_season} days (applies at reset)");
                self.log(SimEvent::new(self.tick, "settings", message).severity(EventSeverity::Info));
                Ok(())
            }
            EcosystemCommand::Select(target) => {
                if let Some(t) = target {
                    self.check_exists(t)?;
                }
                self.follow.selected = target;
                Ok(())
            }
            EcosystemCommand::Outbreak { pathogen, species, count } => {
                if self.netlogo.is_some() {
                    return Err(EcosystemError::Command("diseases are not part of the NetLogo rule set".into()));
                }
                let n = self.outbreak(&pathogen, &species, count)?;
                let message = format!("{n} {species} infected with {pathogen}");
                self.log(SimEvent::new(self.tick, "outbreak", message).severity(EventSeverity::Major));
                Ok(())
            }
            EcosystemCommand::Follow(target) => {
                if let Some(t) = target {
                    self.check_exists(t)?;
                }
                if self.follow.followed != target {
                    self.follow.trail.clear();
                    self.follow.log.clear();
                }
                self.follow.followed = target;
                Ok(())
            }
        }
    }

    /// Introduces organisms (see [`EcoState::introduce`]), commits them and
    /// books them as an intervention input.
    pub(crate) fn introduce_booked(
        &mut self,
        rt: &Runtime,
        species: u16,
        count: u32,
        region: Option<(f32, f32, f32, f32)>,
    ) -> Result<(), EcosystemError> {
        let introduced = self.introduce(rt, species, count, region, None)?;
        self.animals.commit();
        let (mut kj, mut g) = (introduced.kj, introduced.g);
        for id in &introduced.animals {
            if let Some(a) = self.animals.get(*id) {
                kj += a.energy_stock_kj();
                g += a.nutrient_stock_g();
            }
        }
        self.ledgers.book(Flux::InterventionIn, kj, g);
        Ok(())
    }

    /// Removes a fraction of a species (individuals in id order of a
    /// seeded shuffle, field densities proportionally, whole colonies);
    /// returns how many were removed. The matter leaves the ecosystem.
    pub(crate) fn remove_fraction(&mut self, species: &str, fraction: f32) -> Result<f64, EcosystemError> {
        let index = self.catalog.animal_index(species)?;
        let fraction = fraction.clamp(0.0, 1.0);
        let area = self.cell_area();
        if let Some(slot) = self.fields.slot(index) {
            let before = self.fields.stock(area);
            let population = self.fields.population(slot, area);
            self.fields.scale(slot, 1.0 - fraction);
            let after = self.fields.stock(area);
            self.ledgers.book(Flux::InterventionOut, before.0 - after.0, before.1 - after.1);
            return Ok(population * f64::from(fraction));
        }
        if self.catalog.animals()[usize::from(index)].representation.colony().is_some() {
            let mut ids: Vec<u64> =
                self.colonies.colonies.iter().filter(|c| c.species == index).map(|c| c.id).collect();
            self.rng.shuffle(&mut ids);
            let n = (ids.len() as f32 * fraction).round() as usize;
            let before = self.colonies.stock(&self.catalog);
            self.colonies.remove(&ids[..n]);
            let after = self.colonies.stock(&self.catalog);
            self.ledgers.book(Flux::InterventionOut, before.0 - after.0, before.1 - after.1);
            return Ok(n as f64);
        }
        let mut ids: Vec<AgentId> = self.animals.iter().filter(|(_, a)| a.species == index).map(|(id, _)| id).collect();
        self.rng.shuffle(&mut ids);
        let n = (ids.len() as f32 * fraction).round() as usize;
        for &id in &ids[..n] {
            if let Some(a) = self.animals.get(id) {
                self.ledgers.book(Flux::InterventionOut, a.energy_stock_kj(), a.nutrient_stock_g());
            }
            self.animals.queue_despawn(id);
            self.genealogy.record_death(id, self.tick)?;
        }
        self.animals.commit();
        Ok(n as f64)
    }

    /// Infects up to `count` random susceptible hosts of `species` with a
    /// pathogen; returns how many were infected.
    pub(crate) fn outbreak(&mut self, pathogen: &str, species: &str, count: u32) -> Result<u32, EcosystemError> {
        let p = self
            .config
            .pathogens
            .iter()
            .position(|p| p.id == pathogen)
            .ok_or_else(|| EcosystemError::Command(format!("unknown pathogen `{pathogen}`")))?;
        let index = self.catalog.animal_index(species)?;
        if !self.config.pathogens[p].hosts.iter().any(|h| h == species) {
            return Err(EcosystemError::Command(format!("`{species}` is not a host of `{pathogen}`")));
        }
        let mut candidates: Vec<AgentId> = self
            .animals
            .iter()
            .filter(|(_, a)| a.species == index && a.infection.is_none_or(|i| !i.is_infected()))
            .map(|(id, _)| id)
            .collect();
        self.rng.shuffle(&mut candidates);
        let n = candidates.len().min(count as usize);
        let tick = self.tick;
        for &id in &candidates[..n] {
            if let Some(a) = self.animals.get_mut(id) {
                a.infection = Some(crate::disease::Infection {
                    pathogen: p as u16,
                    stage: crate::disease::InfectionStage::Infected { since: tick },
                });
            }
        }
        self.disease.total_infections[p] += n as u64;
        Ok(n as u32)
    }

    fn check_exists(&self, organism: OrganismRef) -> Result<(), EcosystemError> {
        let ok = match organism {
            OrganismRef::Animal(id) => self.animals.contains(AgentId(id)),
            OrganismRef::Tree(id) => self.trees.contains(AgentId(id)),
        };
        if ok { Ok(()) } else { Err(EcosystemError::UnknownOrganism(organism_id(organism))) }
    }

    fn spawn_command(
        &mut self,
        rt: &Runtime,
        species: &str,
        x: f32,
        y: f32,
        radius: f32,
        count: u32,
    ) -> Result<(), EcosystemError> {
        let index = self.catalog.animal_index(species)?;
        if !(1..=MAX_SPAWN).contains(&count) {
            return Err(EcosystemError::Command(format!("count {count} is outside [1, {MAX_SPAWN}]")));
        }
        let (w, h) = (self.world.width() as f32, self.world.height() as f32);
        if !(x.is_finite() && y.is_finite() && (0.0..w).contains(&x) && (0.0..h).contains(&y)) {
            return Err(EcosystemError::Command(format!("({x}, {y}) is outside the {w}×{h} world")));
        }
        if !(radius.is_finite() && (0.0..=w.max(h)).contains(&radius)) {
            return Err(EcosystemError::Command(format!("radius {radius} must be in [0, {}]", w.max(h))));
        }
        let r = radius.max(0.5);
        let region = Some((x - r, y - r, x + r, y + r));
        match &self.config.rules {
            Rules::NetLogo(n) => {
                let (sheep_gain, wolf_gain) = (n.sheep_gain_from_food, n.wolf_gain_from_food);
                for _ in 0..count {
                    let gain =
                        if index == self.netlogo.as_ref().map_or(0, |nl| nl.sheep) { sheep_gain } else { wolf_gain };
                    let p = self.place(rt, index, region).unwrap_or(Vec2::new(x, y));
                    crate::netlogo::spawn_with_energy(self, rt, index, p, gain)?;
                }
                self.animals.commit();
            }
            Rules::Ecological => {
                self.introduce_booked(rt, index, count, region)?;
            }
        }
        let name = self.catalog.animals()[usize::from(index)].name.clone();
        let message = format!("{count} {name} added at ({x:.1}, {y:.1})");
        self.log(SimEvent::new(self.tick, "spawn", message).severity(EventSeverity::Info));
        Ok(())
    }

    fn plant_tree(&mut self, species: &str, x: f32, y: f32) -> Result<(), EcosystemError> {
        if self.netlogo.is_some() {
            return Err(EcosystemError::Command("trees are not part of the NetLogo rule set".into()));
        }
        let index = self.catalog.tree_index(species)?;
        let p = Vec2::new(x, y);
        let (cx, cy) =
            self.world.cell_at(p).ok_or_else(|| EcosystemError::Command(format!("({x}, {y}) is outside the world")))?;
        let cell = cy * self.world.width() + cx;
        if self.world.surface().as_slice()[cell] != SurfaceKind::Land {
            return Err(EcosystemError::Command(format!("cell ({cx}, {cy}) is water: trees need land")));
        }
        let sp = &self.catalog.trees()[usize::from(index)];
        let season = self.world.time().season;
        let wood = sp.wood_at(SEEDLING_HEIGHT_M);
        let leaf = sp.full_leaf_mass(SEEDLING_HEIGHT_M) * phenology(sp.deciduous, season, 0.5);
        let need = wood * sp.wood_nutrient_g_per_kg + leaf * sp.leaf_nutrient_g_per_kg;
        let area = self.world.config().cell_area_m2() as f32;
        if self.world.nutrients().as_slice()[cell] * area < need {
            return Err(EcosystemError::Command(format!(
                "the soil of cell ({cx}, {cy}) has too few nutrients for a seedling"
            )));
        }
        let taken = self.world.take_nutrients(cx, cy, need / area)? * area;
        // The seedling's tissue is supplied by the planter (energy input);
        // its nutrients come from the soil (internal transfer).
        self.ledgers.book(Flux::InterventionIn, f64::from(wood + leaf) * PLANT_KJ_PER_KG, 0.0);
        self.ledgers.book(Flux::PrimaryProduction, 0.0, f64::from(taken));
        let variant = self.rng.below(256) as u8;
        self.trees.queue_spawn(Tree {
            species: index,
            position: p,
            birth_tick: self.tick as i64,
            height_m: SEEDLING_HEIGHT_M,
            wood_kg: wood,
            leaf_kg: leaf,
            health: 1.0,
            light: 1.0,
            burnt: false,
            dead_since: None,
            death: None,
            variant,
        });
        self.trees.commit();
        self.canopy_dirty = true;
        Ok(())
    }
}

fn organism_id(organism: OrganismRef) -> u64 {
    match organism {
        OrganismRef::Animal(id) | OrganismRef::Tree(id) => id,
    }
}
