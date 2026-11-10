//! Applying the outcome of a tick to the animals: attacks, meals,
//! metabolism, drives, growth, stress, ageing, the energy investment of
//! pregnancy and deaths — with every energy and nutrient transfer booked in
//! the ledgers (births, clutches and parental care are in
//! [`super::reproduction`]).
//!
//! Per tick and animal (`dt` = one tick):
//!
//! ```text
//! B        = 0.78 c_th M^{3/4} e^{−E/kT} (body temperature; for ectotherms the air, the water
//!            (≥ 4 °C) when swimming, or in shelter — burrow, den, dormancy, torpor — the daily mean ≥ 0 °C)
//! cost     = B · activity (sleep 0.85, rest 1.0, torpor 0.3, hibernation: the species' dormant
//!            fraction, awake: field multiplier) · (1 + ½ virulence of an infection)
//!          + B · 0.05 · (T_low − T)⁺ + B · 0.03 (T − T_high)⁺     (endotherm thermoregulation)
//!          + COT · distance · speed_gene²                          (locomotion; COT of running, × 0.35
//!            airborne, × 0.12 swimming)
//! energy  −= min(cost, energy)             (unpaid cost → starvation damage)
//! meal     gross E, N → assimilated a·E to reserves, a·N to the store (surplus excreted);
//!          (1 − a) egested as dung (litter); water: food share × days of food × food_water
//! growth   dm = a m^{3/4} (1 − (m/M)^{1/4}) dt (West et al. 2001), paid with
//!          (1 + overhead) · dm · E_tissue and dm · n_tissue
//! ageing   death with the Gompertz hazard of the species
//! ```

use std::collections::{BTreeMap, BTreeSet};

use organism_engine::AgentId;
use simulation_core::glam::Vec2;
use simulation_core::{EventSeverity, SimEvent};

use super::genetics::Phenotype;
use super::interact::{Attack, FoodSource, Meal};
use super::reproduction::offspring_cost;
use super::{Action, Animal, DeathCause, ReproState, Sex};
use crate::carcass::Carcass;
use crate::environment::in_hostile_water;
use crate::error::EcosystemError;
use crate::ledger::Flux;
use crate::species::{AnimalTraits, PlantLayer, ReproductionMode};
use crate::state::{EcoState, Runtime};
use crate::trees::browse_fraction;
use crate::units::{
    ANIMAL_NUTRIENT_G_PER_KG, DETRITUS_KJ_PER_KG, FUNGI_KJ_PER_KG, PLANT_KJ_PER_KG, SYNTHESIS_OVERHEAD,
    TISSUE_KJ_PER_KG,
};

/// Days of unpaid metabolism (empty reserves) that kill.
const STARVATION_DAYS: f32 = 2.0;
/// Days at zero hydration that kill.
const DEHYDRATION_DAYS: f32 = 1.0;
/// Health lost per tick in water deeper than the animal can wade.
const DROWNING_PER_TICK: f32 = 0.25;
/// Health lost per tick in a burning cell.
const BURN_PER_TICK: f32 = 0.5;
/// Hydration regained per tick while drinking.
const DRINK_PER_TICK: f32 = 0.25;
/// Injury healed per day by a fed and watered animal (twice at rest).
pub const INJURY_HEALING_PER_DAY: f32 = 0.2;

/// What happened to the animals during the tick, by store-index order.
pub(crate) struct Motion {
    /// Distance moved this tick (cells), per store index.
    pub distance: Vec<f32>,
    /// Whether the animal sprinted, per store index.
    pub sprint: Vec<bool>,
}

/// Deaths decided during the tick: cause and killer.
pub(crate) type Deaths = BTreeMap<AgentId, (DeathCause, Option<AgentId>)>;

fn reserve_at(traits: &AnimalTraits, mass: f32) -> f32 {
    traits.reserve_max_kj * (mass / traits.adult_mass_kg).max(0.01).powf(0.75)
}

impl EcoState {
    pub(crate) fn cell_of(&self, p: Vec2) -> usize {
        let w = self.world.width();
        let x = (p.x.max(0.0) as usize).min(w - 1);
        let y = (p.y.max(0.0) as usize).min(self.world.height() - 1);
        y * w + x
    }

    /// Records an event in the global log and, for the followed animal, its trace log.
    pub(crate) fn log(&mut self, event: SimEvent) {
        let followed = match self.follow.followed {
            Some(crate::animal::OrganismRef::Animal(id)) => Some(id),
            _ => None,
        };
        if followed.is_some() && (event.subject == followed || event.object == followed) {
            self.follow.log.push(event.clone());
        }
        if event.severity >= EventSeverity::Info {
            self.events.push(event);
        }
    }

    /// Applies attack outcomes (kills are recorded in `deaths`).
    pub(crate) fn apply_attacks(&mut self, attacks: &[Attack], deaths: &mut Deaths) {
        for a in attacks {
            self.counters.attacks += 1;
            if let Some(p) = self.animals.get_mut(a.predator) {
                p.stamina = (p.stamina - 0.3).max(0.0);
                p.health = (p.health - a.predator_injury).max(0.0);
                if a.success {
                    p.stats.kills += 1;
                }
            }
            if let Some(p) = self.animals.get_mut(a.predator) {
                p.injury = (p.injury + a.predator_injury).min(1.0);
            }
            if a.success {
                deaths.entry(a.prey).or_insert((DeathCause::Predation, Some(a.predator)));
                self.counters.kills += 1;
            } else if let Some(q) = self.animals.get_mut(a.prey) {
                q.health = (q.health - a.prey_injury).max(0.0);
                q.injury = (q.injury + a.prey_injury).min(1.0);
                q.stats.escapes += 1;
            }
            let message = if a.success {
                format!("#{} killed #{} (p = {:.2})", a.predator.get(), a.prey.get(), a.probability)
            } else {
                format!("#{} failed to catch #{} (p = {:.2})", a.predator.get(), a.prey.get(), a.probability)
            };
            let severity = if a.success { EventSeverity::Info } else { EventSeverity::Trace };
            self.log(
                SimEvent::new(self.tick, "attack", message)
                    .subject(a.predator.get())
                    .object(a.prey.get())
                    .severity(severity),
            );
        }
    }

    /// Applies granted meals: removes food from its source and books
    /// assimilation, egestion and excretion.
    pub(crate) fn apply_meals(&mut self, rt: &Runtime, meals: &[Meal]) {
        let area = self.cell_area();
        for meal in meals {
            let Some(eater) = self.animals.get(meal.eater) else { continue };
            let (eater_species, position) = (eater.species, eater.kin.position);
            let (gross_kj, gross_g, flux) = self.take_food(meal, eater_species, area);
            if gross_kj <= 0.0 && gross_g <= 0.0 {
                continue;
            }
            self.ledgers.book(flux, gross_kj, gross_g);
            let cell = self.cell_of(position);
            let a = f64::from(meal.assimilation.clamp(0.0, 1.0));
            let egested_kj = gross_kj * (1.0 - a);
            let egested_g = gross_g * (1.0 - a);
            self.detritus.add_litter_energy(cell, egested_kj / area, (egested_g / area) as f32);
            self.ledgers.book(Flux::Egestion, egested_kj, egested_g);
            let mut excreted = 0.0f64;
            let mut respired = 0.0f64;
            let moisture = self.world.soil_moisture().as_slice()[cell];
            let water = preformed_water(meal.source, moisture);
            if let Some(eater) = self.animals.get_mut(meal.eater) {
                let traits = &rt.traits[usize::from(eater.species)];
                let economy = self.catalog.animals()[usize::from(eater.species)].metabolism.food_water;
                // Water contained in the food: `water` of the daily need per
                // day's worth of the eater's own field metabolism eaten,
                // times the species' water economy.
                let days_of_food = (gross_kj / f64::from(eater.field_kj_per_day(traits).max(1e-6))) as f32;
                let per_day = traits.thirst_per_tick * traits.ticks_per_day;
                eater.hydration = (eater.hydration + economy * water * days_of_food * per_day).min(1.0);
                eater.stats.eaten_kj += gross_kj;
                eater.energy_kj += (gross_kj * a) as f32;
                eater.nutrient_store_g += (gross_g * a) as f32;
                // Assimilated energy above the reserve capacity is respired
                // (heat increment of feeding).
                let reserve = eater.reserve_max_kj(traits);
                if eater.energy_kj > reserve {
                    respired = f64::from(eater.energy_kj - reserve);
                    eater.energy_kj = reserve;
                }
                let cap = eater.store_capacity_g();
                if eater.nutrient_store_g > cap {
                    excreted = f64::from(eater.nutrient_store_g - cap);
                    eater.nutrient_store_g = cap;
                }
            }
            if excreted > 0.0 {
                let (x, y) = (cell % self.world.width(), cell / self.world.width());
                let added = self.world.add_nutrients(x, y, (excreted / area) as f32).unwrap_or(0.0);
                self.ledgers.book(Flux::Excretion, 0.0, f64::from(added) * area);
            }
            self.ledgers.book(Flux::AnimalRespiration, respired, 0.0);
        }
    }

    /// Removes the food of a meal from its source; returns gross energy (kJ),
    /// nutrients (g) and the flux to book.
    fn take_food(&mut self, meal: &Meal, eater_species: u16, area: f64) -> (f64, f64, Flux) {
        let kg_m2 = meal.kg / area as f32;
        match meal.source {
            FoodSource::Plant { cell, layer } => {
                let residual = self.catalog.layer(layer).residual_kg_m2;
                let edible = (self.vegetation.biomass(cell, layer) - residual).max(0.0);
                let taken = self.vegetation.remove(cell, layer, kg_m2.min(edible));
                let n = self.catalog.layer(layer).nutrient_g_per_kg;
                (f64::from(taken) * area * PLANT_KJ_PER_KG, f64::from(taken * n) * area, Flux::Herbivory)
            }
            FoodSource::Nectar { cell } => {
                let taken = self.vegetation.remove(cell, PlantLayer::Flowers, kg_m2);
                let visits = f64::from(taken) * area / crate::vegetation::NECTAR_PER_VISIT_KG;
                self.vegetation.visit_flowers(cell, visits, area);
                let n = self.catalog.layer(PlantLayer::Flowers).nutrient_g_per_kg;
                (f64::from(taken) * area * PLANT_KJ_PER_KG, f64::from(taken * n) * area, Flux::Nectarivory)
            }
            FoodSource::Detritus { cell } => {
                let (kg, g) = self.detritus.take_litter(cell, kg_m2);
                (f64::from(kg) * area * DETRITUS_KJ_PER_KG, f64::from(g) * area, Flux::Detritivory)
            }
            FoodSource::Mushrooms { cell } => {
                let taken = self.fungi.take_mushrooms(cell, kg_m2);
                let n = self.catalog.fungi().saprotroph.nutrient_g_per_kg;
                (f64::from(taken) * area * FUNGI_KJ_PER_KG, f64::from(taken * n) * area, Flux::Fungivory)
            }
            FoodSource::Field { cell, slot } => {
                let taken = self.fields.remove_biomass(slot, cell, kg_m2);
                (
                    f64::from(taken) * area * TISSUE_KJ_PER_KG,
                    f64::from(taken * ANIMAL_NUTRIENT_G_PER_KG) * area,
                    Flux::Predation,
                )
            }
            FoodSource::Browse { cell } => {
                // Spread the bite over the reachable foliage of the cell's trees.
                let trees: Vec<u32> = self.canopy.trees_in(cell).to_vec();
                let reach: f32 = trees
                    .iter()
                    .filter_map(|&i| self.trees.as_slice().get(i as usize))
                    .filter(|t| t.is_alive())
                    .map(|t| t.leaf_kg * browse_fraction(t.height_m))
                    .sum();
                let want = meal.kg.min(reach);
                if reach <= 0.0 || want <= 0.0 {
                    return (0.0, 0.0, Flux::Herbivory);
                }
                let mut kj = 0.0;
                let mut g = 0.0;
                for i in trees {
                    let Some(tree) = self.trees.as_mut_slice().get_mut(i as usize) else { continue };
                    if !tree.is_alive() {
                        continue;
                    }
                    let n = self.catalog.trees()[usize::from(tree.species)].leaf_nutrient_g_per_kg;
                    let bite = (tree.leaf_kg * browse_fraction(tree.height_m) * want / reach).min(tree.leaf_kg);
                    tree.leaf_kg -= bite;
                    kj += f64::from(bite) * PLANT_KJ_PER_KG;
                    g += f64::from(bite * n);
                }
                let removed = (kj / PLANT_KJ_PER_KG) as f32;
                self.canopy.browse_kg_m2[cell] = (self.canopy.browse_kg_m2[cell] - removed / area as f32).max(0.0);
                (kj, g, Flux::Herbivory)
            }
            FoodSource::Clutch { id } => {
                let Some(clutch) = self.remains.clutch_mut(id) else { return (0.0, 0.0, Flux::Predation) };
                let (kj, g) = clutch.take(meal.kg);
                (kj, g, Flux::Predation)
            }
            FoodSource::Carcass { id } => {
                let Some(carcass) = self.remains.carcass_mut(id) else { return (0.0, 0.0, Flux::Scavenging) };
                let killer = carcass.killer;
                let (_, kj, g) = carcass.take(meal.kg);
                let fresh_kill = killer == Some(meal.eater)
                    || killer.and_then(|k| self.animals.get(k)).is_some_and(|k| k.species == eater_species);
                (kj, g, if fresh_kill { Flux::Predation } else { Flux::Scavenging })
            }
        }
    }

    /// Metabolism, drives, growth, stress, ageing and pregnancy of every
    /// animal (id order). Returns the mothers whose young are due.
    pub(crate) fn physiology(
        &mut self,
        rt: &Runtime,
        motion: &Motion,
        drinkers: &BTreeSet<AgentId>,
        deaths: &mut Deaths,
    ) -> Vec<AgentId> {
        let scale = rt.scale;
        let dt_days = scale.dt_days();
        let tick = self.tick;
        let seed = self.seed;
        let season = self.world.time().season;
        let mut due = Vec::new();
        let mut conceive_asexual = Vec::new();
        let ids: Vec<AgentId> = self.animals.ids().to_vec();
        for (index, id) in ids.into_iter().enumerate() {
            if deaths.contains_key(&id) {
                continue;
            }
            let cell = {
                let a = &self.animals.as_slice()[index];
                self.cell_of(a.kin.position)
            };
            let air = {
                let a = &self.animals.as_slice()[index];
                let sheltered = a.hidden || matches!(a.action, Action::Torpor | Action::Hibernate);
                crate::environment::experienced_temperature(
                    &self.world,
                    &self.env,
                    &rt.traits[usize::from(a.species)],
                    sheltered,
                    cell,
                )
            };
            let burning = self.world.is_burning(cell % self.world.width(), cell / self.world.width());
            let in_water = self.world.surface().as_slice()[cell] != physics_world::SurfaceKind::Land
                || self.world.water_depth().as_slice()[cell]
                    > rt.traits[usize::from(self.animals.as_slice()[index].species)].wade_depth_m.max(0.3);
            let a = &mut self.animals.as_mut_slice()[index];
            let s = usize::from(a.species);
            let species = &self.catalog.animals()[s];
            let traits = &rt.traits[s];
            let pheno = Phenotype::of(&a.genome, &rt.schema);
            let mut cause: Option<DeathCause> = None;
            let mut damage = false;
            let dormant = a.action == Action::Hibernate;
            // Metabolism.
            let fever = 1.0 + 0.5 * rt.epi.virulence(a);
            let bmr = traits.bmr_kj_per_tick(a.mass_kg, air, &scale) * (1.0 + 0.1 * (pheno.vision - 1.0)) * fever;
            let activity = match a.action {
                Action::Sleep => 0.85,
                Action::Rest | Action::Incubate => 1.0,
                Action::Torpor => 0.3,
                Action::Hibernate => species.seasonal.hibernation.as_ref().map_or(1.0, |h| h.metabolic_fraction),
                _ => species.metabolism.field_multiplier,
            };
            let (lo, hi) = (
                species.climate.preferred_c.0 + pheno.thermal_shift,
                species.climate.preferred_c.1 + pheno.thermal_shift,
            );
            // Dormant animals sit in an insulated den.
            let thermo = if traits.endotherm && !dormant {
                bmr * (0.05 * (lo - air).max(0.0) + 0.03 * (air - hi).max(0.0))
            } else {
                0.0
            };
            let locomotion = transport_cost_factor(traits, a.altitude_m, in_water)
                * traits.cot_kj_per_cell
                * (a.mass_kg / traits.adult_mass_kg).max(0.01).powf(0.68)
                * motion.distance[index]
                * pheno.speed
                * pheno.speed;
            let cost = bmr * activity + thermo + locomotion;
            let paid = cost.min(a.energy_kj.max(0.0));
            a.energy_kj -= paid;
            self.ledgers.book(Flux::AnimalRespiration, f64::from(paid), 0.0);
            if cost > paid + 1e-6 {
                a.health -= dt_days / STARVATION_DAYS;
                damage = true;
                cause = Some(DeathCause::Starvation);
            }
            // Water.
            let heat = 1.0 + 0.03 * (air - 25.0).max(0.0);
            let water_loss = if dormant { 0.05 } else { 1.0 };
            a.hydration -= traits.thirst_per_tick * heat * water_loss;
            if drinkers.contains(&id) {
                a.hydration += DRINK_PER_TICK;
            }
            a.hydration = a.hydration.clamp(0.0, 1.0);
            if a.hydration <= 0.0 {
                a.health -= dt_days / DEHYDRATION_DAYS;
                damage = true;
                cause = Some(DeathCause::Dehydration);
            }
            // Sleep pressure and stamina.
            a.fatigue = if a.action == Action::Sleep { a.fatigue - 3.0 * dt_days } else { a.fatigue + 1.5 * dt_days }
                .clamp(0.0, 1.0);
            let endurance = traits.endurance_ticks.max(0.5);
            a.stamina = if motion.sprint[index] {
                a.stamina - 1.0 / endurance
            } else {
                let rate = if matches!(a.action, Action::Rest | Action::Sleep) { 2.0 } else { 1.0 };
                a.stamina + rate / (3.0 * endurance)
            }
            .clamp(0.0, 1.0);
            // Temperature.
            let (tl, th) = (
                species.climate.tolerance_c.0 + pheno.thermal_shift,
                species.climate.tolerance_c.1 + pheno.thermal_shift,
            );
            a.comfort = if air < lo {
                ((air - tl) / (lo - tl).max(1e-3)).clamp(0.0, 1.0)
            } else if air > hi {
                ((th - air) / (th - hi).max(1e-3)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let excess = if dormant { 0.0 } else { (tl - air).max(0.0) + (air - th).max(0.0) };
            if excess > 0.0 {
                a.health -= 0.5 * dt_days * (excess / 5.0).max(0.2);
                damage = true;
                cause = Some(DeathCause::Exposure);
            }
            // Hazards.
            let hostile = in_hostile_water(&self.world, traits, a.kin.position);
            if hostile {
                a.drowning_ticks = a.drowning_ticks.saturating_add(1);
                a.health -= DROWNING_PER_TICK;
                damage = true;
                cause = Some(DeathCause::Drowning);
            } else {
                a.drowning_ticks = 0;
            }
            if burning && !a.hidden && !traits.aquatic {
                a.health -= BURN_PER_TICK * if traits.flies { 0.3 } else { 1.0 };
                damage = true;
                cause = Some(DeathCause::Fire);
            }
            if !damage && a.hunger(traits) < 0.5 && a.hydration > 0.5 {
                a.health = (a.health + 0.5 * dt_days).min(1.0);
                // Injuries heal, twice as fast at rest.
                let rest = if matches!(a.action, Action::Rest | Action::Sleep | Action::Hibernate) { 2.0 } else { 1.0 };
                a.injury = (a.injury - INJURY_HEALING_PER_DAY * rest * dt_days).max(0.0);
            }
            if a.health <= 0.0 {
                deaths.insert(id, (cause.unwrap_or(DeathCause::Injury), None));
                continue;
            }
            // Ageing.
            let mut rng = crate::rng::stream(seed, crate::rng::LIFE, tick, id.get());
            if rng.chance(traits.ageing_hazard(a.age_ticks(tick) as f64)) {
                deaths.insert(id, (DeathCause::OldAge, None));
                continue;
            }
            grow(a, traits, &pheno, dt_days, &mut self.ledgers);
            // Reproductive state.
            match &mut a.repro {
                ReproState::Juvenile => {
                    if a.age_ticks(tick) as f64 >= traits.maturity_ticks
                        && a.mass_kg >= 0.7 * traits.adult_mass_kg * pheno.size
                    {
                        a.repro = ReproState::Ready;
                        // A territorial animal settles where it matures
                        // (burrowers keep their burrow as the centre).
                        if traits.territory_cells > 0.0 && a.home.is_none() {
                            a.home = Some(a.kin.position);
                        }
                    }
                }
                ReproState::Recovering { until } => {
                    if tick >= *until {
                        a.repro = ReproState::Ready;
                    }
                }
                ReproState::Ready => {
                    let asexual = species.reproduction.mode == ReproductionMode::Asexual;
                    let in_season = species.reproduction.breeding_seasons.is_empty()
                        || species.reproduction.breeding_seasons.contains(&season);
                    if asexual
                        && a.sex == Sex::Female
                        && in_season
                        && a.energy_kj >= species.reproduction.min_energy_fraction * reserve_at(traits, a.mass_kg)
                    {
                        conceive_asexual.push(id);
                    }
                }
                ReproState::Gestating(p) => {
                    let (e_off, n_off) = offspring_cost(traits);
                    let litter = f64::from(p.litter);
                    let per_tick_kj = litter * e_off / traits.gestation_ticks.max(1.0);
                    let per_tick_g = litter * n_off / traits.gestation_ticks.max(1.0);
                    let tissue_share = f64::from(traits.birth_mass_kg) * TISSUE_KJ_PER_KG / e_off.max(1e-9);
                    let overhead = per_tick_kj * tissue_share * f64::from(SYNTHESIS_OVERHEAD);
                    let available = f64::from((a.energy_kj - 0.2 * reserve_at(traits, a.mass_kg)).max(0.0));
                    let pay = (per_tick_kj + overhead).min(available);
                    let invest = pay * per_tick_kj / (per_tick_kj + overhead).max(1e-12);
                    a.energy_kj -= pay as f32;
                    p.embryo_kj += invest;
                    self.ledgers.book(Flux::AnimalRespiration, pay - invest, 0.0);
                    let g = per_tick_g.min(f64::from(a.nutrient_store_g.max(0.0)));
                    a.nutrient_store_g -= g as f32;
                    p.embryo_g += g;
                    if tick >= p.due {
                        due.push(id);
                    }
                }
            }
        }
        for id in conceive_asexual {
            self.conceive(rt, id, None);
        }
        due
    }

    /// Turns the dead into carcasses, updates counters, genealogy and logs.
    pub(crate) fn bury(&mut self, deaths: &Deaths) -> Result<(), EcosystemError> {
        for (&id, &(cause, killer)) in deaths {
            let Some(a) = self.animals.get(id) else { continue };
            let (species, position, mass) = (a.species, a.kin.position, a.mass_kg);
            let (kj, g) = (a.energy_stock_kj(), a.nutrient_stock_g());
            let carcass_id = self.remains.next_id();
            self.remains.carcasses.push(Carcass {
                id: carcass_id,
                species,
                position,
                meat_kg: mass,
                initial_kg: mass,
                energy_kj: kj,
                nutrients_g: g,
                created_tick: self.tick,
                cause,
                killer,
            });
            self.ledgers.book(Flux::AnimalMortality, kj, g);
            self.animals.queue_despawn(id);
            self.counters.death(species, cause);
            self.genealogy.record_death(id, self.tick)?;
            let name = self.catalog.animals()[usize::from(species)].name.clone();
            let mut event = SimEvent::new(self.tick, "death", format!("{name} #{} died ({})", id.get(), cause.key()))
                .subject(id.get());
            if let Some(k) = killer {
                event = event.object(k.get());
            }
            self.log(event);
        }
        Ok(())
    }
}

/// Cost of transport relative to walking or running, by locomotion mode:
/// flying when airborne (above 1 m), swimming for aquatic animals and for
/// swimmers in water deeper than they wade, otherwise 1.
pub(crate) fn transport_cost_factor(traits: &AnimalTraits, altitude_m: f32, in_water: bool) -> f32 {
    if traits.flies && altitude_m > crate::animal::perceive::AIRBORNE_M {
        crate::units::FLYING_COT_FRACTION as f32
    } else if traits.aquatic || (traits.swims && in_water) {
        crate::units::SWIMMING_COT_FRACTION as f32
    } else {
        1.0
    }
}

/// Fraction of the daily water need covered by one day's worth of a food
/// (preformed water): fresh plants 0.3–0.8 depending on soil moisture
/// (forage dries out in drought), nectar 1, mushrooms 0.8, meat 0.6,
/// browse 0.6, litter 0.1.
fn preformed_water(source: FoodSource, soil_moisture: f32) -> f32 {
    match source {
        FoodSource::Plant { .. } => 0.3 + 0.5 * crate::units::ramp(soil_moisture, 0.1, 0.4),
        FoodSource::Nectar { .. } => 1.0,
        FoodSource::Mushrooms { .. } => 0.8,
        FoodSource::Carcass { .. }
        | FoodSource::Browse { .. }
        | FoodSource::Field { .. }
        | FoodSource::Clutch { .. } => 0.6,
        FoodSource::Detritus { .. } => 0.1,
    }
}

/// Ontogenetic growth (West et al. 2001) towards the adult mass: the rate
/// constant is chosen so that an animal reaches 90 % of its adult mass at
/// maturity; growth is paid from the reserve (with the synthesis overhead
/// respired) and the nutrient store.
fn grow(a: &mut Animal, traits: &AnimalTraits, pheno: &Phenotype, dt_days: f32, ledgers: &mut crate::ledger::Ledgers) {
    let adult = traits.adult_mass_kg * pheno.size;
    if a.mass_kg >= adult {
        return;
    }
    let tau_days = (traits.maturity_ticks * f64::from(dt_days)).max(1.0) as f32;
    let m0 = (traits.birth_mass_kg / adult).clamp(1e-6, 0.99).powf(0.25);
    let ratio = ((1.0 - 0.9f32.powf(0.25)) / (1.0 - m0)).clamp(1e-6, 0.999);
    let a_rate = -4.0 * adult.powf(0.25) * ratio.ln() / tau_days;
    let m = a.mass_kg;
    let mut dm = a_rate * m.powf(0.75) * (1.0 - (m / adult).powf(0.25)) * dt_days;
    dm = dm.clamp(0.0, adult - m);
    let tissue = dm * TISSUE_KJ_PER_KG as f32;
    let cost = tissue * (1.0 + SYNTHESIS_OVERHEAD);
    let need_g = dm * ANIMAL_NUTRIENT_G_PER_KG;
    let affordable =
        (a.energy_kj.max(0.0) / cost.max(1e-9)).min(a.nutrient_store_g.max(0.0) / need_g.max(1e-9)).min(1.0);
    // f32 rounding can make the scaled cost exceed the reserve by an ulp:
    // shrink the step so that reserve and store never go negative.
    let dm = (dm * affordable * (1.0 - 1e-5))
        .min(a.energy_kj.max(0.0) / (TISSUE_KJ_PER_KG as f32 * (1.0 + SYNTHESIS_OVERHEAD)));
    let dm = dm.min(a.nutrient_store_g.max(0.0) / ANIMAL_NUTRIENT_G_PER_KG);
    if dm <= 0.0 {
        return;
    }
    let tissue = dm * TISSUE_KJ_PER_KG as f32;
    a.energy_kj = (a.energy_kj - tissue * (1.0 + SYNTHESIS_OVERHEAD)).max(0.0);
    a.nutrient_store_g = (a.nutrient_store_g - dm * ANIMAL_NUTRIENT_G_PER_KG).max(0.0);
    a.mass_kg += dm;
    ledgers.book(Flux::AnimalRespiration, f64::from(tissue * SYNTHESIS_OVERHEAD), 0.0);
}

#[cfg(test)]
mod tests {
    use super::transport_cost_factor;
    use crate::species::{AnimalTraits, SpeciesCatalog};
    use crate::units::TimeScale;

    fn traits(id: &str) -> AnimalTraits {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let scale = TimeScale { ticks_per_day: 240, days_per_year: 120, cell_size_m: 20.0, motion_seconds: 5.0 };
        AnimalTraits::derive(catalog.animal(id).unwrap(), &scale)
    }

    #[test]
    fn locomotion_costs_depend_on_the_mode() {
        // Per metre, flight costs about a third and swimming about a tenth
        // of running at the same mass (Schmidt-Nielsen 1972; Tucker 1975).
        let (duck, deer, perch, frog) = (traits("duck"), traits("deer"), traits("perch"), traits("frog"));
        assert_eq!(transport_cost_factor(&deer, 0.0, false), 1.0);
        assert_eq!(transport_cost_factor(&duck, 0.0, false), 1.0, "a duck walking on land");
        let flying = transport_cost_factor(&duck, 30.0, false);
        let swimming = transport_cost_factor(&duck, 0.0, true);
        assert!(swimming < flying && flying < 1.0, "{swimming} < {flying} < 1");
        assert_eq!(transport_cost_factor(&perch, -1.0, true), swimming, "fish always swim");
        assert_eq!(transport_cost_factor(&frog, 0.0, true), swimming, "a frog swimming in a pond");
        assert_eq!(transport_cost_factor(&frog, 0.0, false), 1.0, "a frog hopping on land");
    }
}
