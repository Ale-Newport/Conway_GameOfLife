//! Reproduction: conception (sexual matings and asexual females), the
//! energy and nutrient investment of pregnancy, births and clutches of eggs,
//! hatching, parental care (lactation) and population caps.
//!
//! An offspring costs `birth tissue + ½ reserve at birth mass` (kJ) and
//! `birth mass × 30 g/kg` (g); the mother accrues `litter ×` these over the
//! gestation (paying a synthesis overhead on the tissue part). At term the
//! number of viable young is limited by what was accrued and by the
//! population caps; the rest returns to the mother.

use organism_engine::{AgentId, Kinematics, Lineage};
use simulation_core::SimEvent;
use simulation_core::glam::Vec2;

use super::genetics::{self, Phenotype};
use super::{Animal, Pregnancy, ReproState, Sex};
use crate::carcass::Clutch;
use crate::environment::{in_hostile_water, passable};
use crate::error::EcosystemError;
use crate::ledger::Flux;
use crate::species::{AnimalTraits, Development, TaxonClass};
use crate::state::{EcoState, Runtime};
use crate::units::{ANIMAL_NUTRIENT_G_PER_KG, TISSUE_KJ_PER_KG};

/// Nutrients transferred with milk (g per kJ).
const MILK_G_PER_KJ: f32 = 0.002;
/// Reserve fraction a mother keeps for herself.
const MATERNAL_FLOOR: f32 = 0.3;

fn reserve_at(traits: &AnimalTraits, mass: f32) -> f32 {
    traits.reserve_max_kj * (mass / traits.adult_mass_kg).max(0.01).powf(0.75)
}

impl EcoState {
    /// Conception of the females of granted matings.
    pub(crate) fn apply_matings(&mut self, rt: &Runtime, matings: &[(AgentId, AgentId)]) {
        for &(male, female) in matings {
            let Some(father) = self.animals.get(male) else { continue };
            let (father_lineage, father_genome) = (father.lineage, father.genome.clone());
            self.conceive(rt, female, Some((male, father_lineage, father_genome)));
            let message = format!("#{} mated with #{}", male.get(), female.get());
            self.log(SimEvent::new(self.tick, "mating", message).subject(female.get()).object(male.get()));
        }
    }

    /// Starts a pregnancy (litter size drawn from the species range × fecundity).
    pub(crate) fn conceive(
        &mut self,
        rt: &Runtime,
        mother: AgentId,
        father: Option<(AgentId, Lineage, organism_engine::Genome)>,
    ) {
        let tick = self.tick;
        let seed = self.seed;
        let Some(m) = self.animals.get_mut(mother) else { return };
        let s = usize::from(m.species);
        let species = &self.catalog.animals()[s];
        let traits = &rt.traits[s];
        let pheno = Phenotype::of(&m.genome, &rt.schema);
        let mut rng = crate::rng::stream(seed, crate::rng::LIFE, tick, mother.get() ^ 0xC0_17CE);
        let (lo, hi) = species.reproduction.litter;
        let base = rng.range_i64(i64::from(lo), i64::from(hi) + 1) as f32;
        let litter = (base * pheno.fecundity).round().clamp(1.0, f32::from(u16::MAX)) as u16;
        let (father_id, father_lineage, father_genome) = match father {
            Some((id, lineage, genome)) => (Some(id), Some(lineage), Some(genome)),
            None => (None, None, None),
        };
        m.repro = ReproState::Gestating(Box::new(Pregnancy {
            conceived: tick,
            due: tick + traits.gestation_ticks.ceil() as u64,
            litter,
            father: father_id,
            father_lineage,
            father_genome,
            embryo_kj: 0.0,
            embryo_g: 0.0,
        }));
    }

    /// Lactation / feeding of dependent young by their mothers.
    pub(crate) fn parental_care(&mut self, rt: &Runtime) {
        let tick = self.tick;
        let mut transfers: Vec<(AgentId, AgentId, f32, f32)> = Vec::new();
        for (id, young) in self.animals.iter() {
            let traits = &rt.traits[usize::from(young.species)];
            if !matches!(young.repro, ReproState::Juvenile) || young.age_ticks(tick) as f64 >= traits.care_ticks {
                continue;
            }
            let Some(mother_id) = young.lineage.parents[0] else { continue };
            let Some(mother) = self.animals.get(mother_id) else { continue };
            if mother.kin.position.distance(young.kin.position) > 2.0 {
                continue;
            }
            let room = (reserve_at(traits, young.mass_kg) - young.energy_kj).max(0.0);
            let cap = traits.intake_kj_per_tick * (young.mass_kg / traits.adult_mass_kg).max(0.01).powf(0.75);
            let surplus = (mother.energy_kj - MATERNAL_FLOOR * reserve_at(traits, mother.mass_kg)).max(0.0);
            let kj = room.min(cap).min(surplus);
            if kj > 0.0 {
                let g = (kj * MILK_G_PER_KJ).min(mother.nutrient_store_g.max(0.0));
                transfers.push((mother_id, id, kj, g));
            }
        }
        for (mother, young, kj, g) in transfers {
            let Some((m, y)) = self.animals.get_pair_mut(mother, young) else { continue };
            let kj = kj.min(m.energy_kj.max(0.0));
            let g = g.min(m.nutrient_store_g.max(0.0));
            m.energy_kj -= kj;
            m.nutrient_store_g -= g;
            y.energy_kj += kj;
            y.nutrient_store_g += g;
            // Milk also covers the water need of the young.
            let traits = &rt.traits[usize::from(y.species)];
            let days_of_food = kj / y.field_kj_per_day(traits).max(1e-6);
            y.hydration = (y.hydration + days_of_food * traits.thirst_per_tick * traits.ticks_per_day).min(1.0);
            self.ledgers.book(Flux::ReproductiveInvestment, f64::from(kj), f64::from(g));
        }
    }

    /// Animals alive or queued for spawning, per species and in total
    /// (one `O(n)` count per phase instead of one per mother).
    fn census(&self) -> Census {
        let mut per_species = vec![0u32; self.catalog.animals().len()];
        let alive = self.animals.as_slice().iter().map(|a| a.species);
        let queued = self.animals.pending().spawns().map(|(_, a)| a.species);
        for s in alive.chain(queued) {
            if let Some(n) = per_species.get_mut(usize::from(s)) {
                *n = n.saturating_add(1);
            }
        }
        let total = per_species.iter().fold(0u32, |t, n| t.saturating_add(*n));
        Census { per_species, total }
    }

    /// Population room of a species under the caps.
    fn population_room(&self, census: &Census, species: u16) -> u32 {
        let count = census.per_species.get(usize::from(species)).copied().unwrap_or(0);
        let per_species = self.params.max_population_per_species.saturating_sub(count);
        let all = self.params.max_population_total.saturating_sub(census.total);
        per_species.min(all)
    }

    /// Births (or laying) of the due mothers.
    pub(crate) fn deliver(&mut self, rt: &Runtime, mothers: &[AgentId]) -> Result<(), EcosystemError> {
        if mothers.is_empty() {
            return Ok(());
        }
        let mut census = self.census();
        for &mother_id in mothers {
            let Some(mother) = self.animals.get(mother_id) else { continue };
            let ReproState::Gestating(p) = mother.repro.clone() else { continue };
            let s = usize::from(mother.species);
            let traits = &rt.traits[s];
            let species = &self.catalog.animals()[s];
            let (e_off, n_off) = offspring_cost(traits);
            let affordable =
                ((p.embryo_kj / e_off + 1e-9).floor()).min((p.embryo_g / n_off.max(1e-12) + 1e-9).floor()) as u32;
            let room = self.population_room(&census, mother.species);
            let viable = u32::from(p.litter).min(affordable).min(room);
            if room < u32::from(p.litter).min(affordable) {
                self.counters.births_suppressed += u64::from(u32::from(p.litter).min(affordable) - room);
            }
            let invested_kj = f64::from(viable) * e_off;
            let invested_g = f64::from(viable) * n_off;
            let leftover_kj = (p.embryo_kj - invested_kj).max(0.0);
            let leftover_g = (p.embryo_g - invested_g).max(0.0);
            let (position, lineage, genome, sex_species, interval) =
                (mother.kin.position, mother.lineage, mother.genome.clone(), mother.species, traits.interval_ticks);
            let development = species.reproduction.development;
            let tick = self.tick;
            if viable > 0 {
                self.ledgers.book(Flux::ReproductiveInvestment, invested_kj, invested_g);
                match development {
                    Development::Live { .. } => {
                        census.add(sex_species, viable);
                        let mut rng = crate::rng::stream(self.seed, crate::rng::LIFE, tick, mother_id.get() ^ 0xB1_27);
                        for _ in 0..viable {
                            let child_genome = genetics::offspring(
                                &rt.schema,
                                &genome,
                                p.father_genome.as_ref(),
                                self.params.mutation_rate,
                                &mut rng,
                            )?;
                            let spot = position + rng.in_disc(0.3);
                            let spot = if passable(&self.world, traits, spot) { spot } else { position };
                            let reserve = (e_off - f64::from(traits.birth_mass_kg) * TISSUE_KJ_PER_KG) as f32;
                            let store =
                                (n_off - f64::from(traits.birth_mass_kg * ANIMAL_NUTRIENT_G_PER_KG)).max(0.0) as f32;
                            self.spawn_young(
                                rt,
                                sex_species,
                                spot,
                                child_genome,
                                &lineage,
                                p.father_lineage.as_ref(),
                                reserve,
                                store,
                                &mut rng,
                            )?;
                        }
                    }
                    Development::Eggs { .. } => {
                        let id = self.remains.next_id();
                        self.remains.clutches.push(Clutch {
                            id,
                            species: sex_species,
                            position,
                            eggs: viable as u16,
                            laid_tick: tick,
                            hatch_tick: tick + traits.incubation_ticks.ceil() as u64,
                            mother: mother_id,
                            mother_lineage: lineage,
                            mother_genome: genome.clone(),
                            father_lineage: p.father_lineage,
                            father_genome: p.father_genome.clone(),
                            energy_kj: invested_kj,
                            nutrients_g: invested_g,
                        });
                        let message = format!("#{} laid {viable} eggs", mother_id.get());
                        self.log(SimEvent::new(tick, "laying", message).subject(mother_id.get()));
                    }
                }
            }
            // Return what was not used to the mother (surplus energy respired,
            // surplus nutrients excreted).
            let cell = self.cell_of(position);
            let area = self.cell_area();
            let mut excrete = 0.0f64;
            if let Some(m) = self.animals.get_mut(mother_id) {
                let cap = reserve_at(traits, m.mass_kg);
                let back = (leftover_kj as f32).min((cap - m.energy_kj).max(0.0));
                m.energy_kj += back;
                self.ledgers.book(Flux::ReproductiveLoss, leftover_kj - f64::from(back), 0.0);
                let room_g = (m.store_capacity_g() - m.nutrient_store_g).max(0.0);
                let back_g = (leftover_g as f32).min(room_g);
                m.nutrient_store_g += back_g;
                excrete = leftover_g - f64::from(back_g);
                m.repro = ReproState::Recovering { until: tick + interval.ceil() as u64 };
                m.stats.offspring += if matches!(development, Development::Live { .. }) { viable } else { 0 };
            }
            if excrete > 0.0 {
                let added = self.world.add_nutrients(
                    cell % self.world.width(),
                    cell / self.world.width(),
                    (excrete / area) as f32,
                );
                self.ledgers.book(Flux::Excretion, 0.0, f64::from(added.unwrap_or(0.0)) * area);
            }
            if viable > 0 && matches!(development, Development::Live { .. }) {
                let message = format!("#{} gave birth to {viable}", mother_id.get());
                self.log(SimEvent::new(tick, "birth", message).subject(mother_id.get()));
            }
        }
        Ok(())
    }

    /// Queues a newborn with its genealogy record.
    #[allow(clippy::too_many_arguments)]
    fn spawn_young(
        &mut self,
        rt: &Runtime,
        species: u16,
        position: Vec2,
        genome: organism_engine::Genome,
        mother: &Lineage,
        father: Option<&Lineage>,
        reserve_kj: f32,
        store_g: f32,
        rng: &mut simulation_core::SimRng,
    ) -> Result<AgentId, EcosystemError> {
        let traits = &rt.traits[usize::from(species)];
        let id = self.animals.next_id();
        let lineage = Lineage::offspring(id, mother, father, self.tick);
        let sex = if rng.chance(0.5) { Sex::Female } else { Sex::Male };
        let heading = rng.f32() * std::f32::consts::TAU;
        let mother_home = self.animals.get(mother.id).and_then(|m| m.home);
        let mother_group = self.animals.get(mother.id).and_then(|m| m.group);
        let mut young =
            Animal::new(species, sex, Kinematics::new(position, heading), self.tick as i64, genome, lineage);
        young.mass_kg = traits.birth_mass_kg;
        young.energy_kj = reserve_kj.max(0.0);
        young.nutrient_store_g = store_g;
        young.home = if traits.burrows { mother_home.or(Some(position)) } else { None };
        young.group = mother_group;
        let queued = self.animals.queue_spawn(young);
        self.genealogy.record_birth(lineage)?;
        self.counters.birth(species);
        Ok(queued)
    }

    /// A bird that lost its clutch may lay a replacement clutch: it is ready
    /// to breed again (in its breeding season, with enough reserve) instead
    /// of waiting for its usual interval.
    fn replacement_clutch(&mut self, mother: AgentId, species: u16) {
        if self.catalog.animals()[usize::from(species)].class != TaxonClass::Bird {
            return;
        }
        if let Some(m) = self.animals.get_mut(mother)
            && matches!(m.repro, ReproState::Recovering { .. })
        {
            m.repro = ReproState::Ready;
        }
    }

    /// Hatches due clutches and destroys clutches in burning cells or under water.
    pub(crate) fn clutches(&mut self, rt: &Runtime) -> Result<(), EcosystemError> {
        let tick = self.tick;
        let area = self.cell_area();
        let clutches = std::mem::take(&mut self.remains.clutches);
        let mut kept = Vec::with_capacity(clutches.len());
        let mut census = self.census();
        for clutch in clutches {
            let cell = self.cell_of(clutch.position);
            let traits = &rt.traits[usize::from(clutch.species)];
            if clutch.eggs == 0 {
                // Raided: nothing left to incubate.
                let message = format!("clutch of #{} eaten", clutch.mother.get());
                self.log(SimEvent::new(tick, "clutch_lost", message).subject(clutch.mother.get()));
                self.replacement_clutch(clutch.mother, clutch.species);
                continue;
            }
            let (x, y) = (cell % self.world.width(), cell / self.world.width());
            if self.world.is_burning(x, y) || in_hostile_water(&self.world, traits, clutch.position) {
                self.detritus.add_litter_energy(cell, clutch.energy_kj / area, (clutch.nutrients_g / area) as f32);
                self.ledgers.book(Flux::AnimalMortality, clutch.energy_kj, clutch.nutrients_g);
                let message = format!("clutch of #{} destroyed", clutch.mother.get());
                self.log(SimEvent::new(tick, "clutch_lost", message).subject(clutch.mother.get()));
                self.replacement_clutch(clutch.mother, clutch.species);
                continue;
            }
            if tick < clutch.hatch_tick {
                kept.push(clutch);
                continue;
            }
            let room = self.population_room(&census, clutch.species).min(u32::from(clutch.eggs));
            census.add(clutch.species, room);
            if room < u32::from(clutch.eggs) {
                self.counters.births_suppressed += u64::from(u32::from(clutch.eggs) - room);
            }
            let eggs = f64::from(clutch.eggs.max(1));
            let per_kj = clutch.energy_kj / eggs;
            let per_g = clutch.nutrients_g / eggs;
            let tissue_kj = f64::from(traits.birth_mass_kg) * TISSUE_KJ_PER_KG;
            let tissue_g = f64::from(traits.birth_mass_kg * ANIMAL_NUTRIENT_G_PER_KG);
            let mut rng = crate::rng::stream(self.seed, crate::rng::LIFE, tick, clutch.id ^ 0x0E66);
            for _ in 0..room {
                let genome = genetics::offspring(
                    &rt.schema,
                    &clutch.mother_genome,
                    clutch.father_genome.as_ref(),
                    self.params.mutation_rate,
                    &mut rng,
                )?;
                let spot = clutch.position + rng.in_disc(0.3);
                let spot = if passable(&self.world, traits, spot) { spot } else { clutch.position };
                let reserve = (per_kj - tissue_kj).max(0.0) as f32;
                let store = (per_g - tissue_g).max(0.0) as f32;
                self.spawn_young(
                    rt,
                    clutch.species,
                    spot,
                    genome,
                    &clutch.mother_lineage,
                    clutch.father_lineage.as_ref(),
                    reserve,
                    store,
                    &mut rng,
                )?;
            }
            // Unhatched eggs (caps) and rounding remainders go to the litter.
            let used_kj = f64::from(room) * per_kj;
            let used_g = f64::from(room) * per_g;
            let rest_kj = (clutch.energy_kj - used_kj).max(0.0);
            let rest_g = (clutch.nutrients_g - used_g).max(0.0);
            if rest_kj > 0.0 || rest_g > 0.0 {
                self.detritus.add_litter_energy(cell, rest_kj / area, (rest_g / area) as f32);
                self.ledgers.book(Flux::AnimalMortality, rest_kj, rest_g);
            }
            if let Some(m) = self.animals.get_mut(clutch.mother) {
                m.stats.offspring += room;
            }
            let message = format!("{room} eggs of #{} hatched", clutch.mother.get());
            self.log(SimEvent::new(tick, "hatching", message).subject(clutch.mother.get()));
        }
        self.remains.clutches = kept;
        Ok(())
    }
}

/// Head count used for the population caps during one phase.
struct Census {
    /// Animals alive or queued per species index.
    per_species: Vec<u32>,
    /// All animals alive or queued.
    total: u32,
}

impl Census {
    /// Records `n` newborns of a species.
    fn add(&mut self, species: u16, n: u32) {
        if let Some(count) = self.per_species.get_mut(usize::from(species)) {
            *count = count.saturating_add(n);
        }
        self.total = self.total.saturating_add(n);
    }
}

/// Energy (kJ) and nutrients (g) one offspring receives: tissue plus half a
/// reserve at birth mass.
pub(crate) fn offspring_cost(traits: &AnimalTraits) -> (f64, f64) {
    let m = traits.birth_mass_kg;
    let reserve = 0.5 * reserve_at(traits, m);
    (f64::from(m) * TISSUE_KJ_PER_KG + f64::from(reserve), f64::from(m * ANIMAL_NUTRIENT_G_PER_KG))
}
