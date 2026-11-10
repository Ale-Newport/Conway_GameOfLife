//! Metrics published every tick: populations, births and deaths per
//! species, biomass per trophic level, energy flows, nutrient pools and the
//! balance of both ledgers, mean age, diversity (Shannon and Gini–Simpson via
//! `analytics`), social and seasonal activity, disease, pollination and
//! environmental state.
//!
//! Populations of density-field species are numbers of individuals (density
//! × area); of colony species, numbers of workers (`colonies.<id>` counts
//! the colonies). Shannon and Simpson indices are computed over individual
//! animals only (field counts of billions of insects would swamp them);
//! richness counts every species present.

use analytics::Diversity;
use physics_world::{FireState, SurfaceKind};
use simulation_core::{MetricSpec, Metrics};

use crate::animal::Action;
use crate::config::{EcosystemConfig, Rules};
use crate::ledger::Flux;
use crate::species::{Representation, Role, Species, SpeciesCatalog};
use crate::state::EcoState;

/// Trophic level used for biomass metrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrophicLevel {
    /// Herbivores, nectarivores and detritivores.
    Primary,
    /// Omnivores, carnivores and scavengers that are not apex predators.
    Secondary,
    /// Apex predators.
    Apex,
}

/// Trophic level of a species from its roles.
pub fn trophic_level(species: &Species) -> TrophicLevel {
    if species.has_role(Role::Apex) {
        TrophicLevel::Apex
    } else if species.has_role(Role::Carnivore)
        || species.has_role(Role::Omnivore)
        || species.has_role(Role::Predator)
        || species.has_role(Role::Scavenger)
    {
        TrophicLevel::Secondary
    } else {
        TrophicLevel::Primary
    }
}

/// Declared metrics of a configuration (its catalogue, rule set and pathogens).
pub fn metric_specs(catalog: &SpeciesCatalog, config: &EcosystemConfig) -> Vec<MetricSpec> {
    let rules = &config.rules;
    let mut specs = vec![
        MetricSpec::gauge("animals", "Animals", "", "Living individual animals of every species."),
        MetricSpec::gauge("trees", "Trees", "", "Living trees."),
    ];
    for s in catalog.animals() {
        match s.representation {
            Representation::Individual => {
                specs.push(MetricSpec::gauge(
                    &format!("population.{}", s.id),
                    &s.name,
                    "",
                    &format!("Living {}.", s.name),
                ));
                specs.push(MetricSpec::counter(
                    &format!("births.{}", s.id),
                    &format!("{} births", s.name),
                    "Births this tick.",
                ));
                specs.push(MetricSpec::counter(
                    &format!("deaths.{}", s.id),
                    &format!("{} deaths", s.name),
                    "Deaths this tick.",
                ));
            }
            Representation::Field(_) => specs.push(MetricSpec::gauge(
                &format!("population.{}", s.id),
                &s.name,
                "",
                &format!("{} individuals (density field × area).", s.name),
            )),
            Representation::Colony(_) => {
                specs.push(MetricSpec::gauge(
                    &format!("population.{}", s.id),
                    &s.name,
                    "workers",
                    &format!("Workers of all {} colonies.", s.name),
                ));
                specs.push(MetricSpec::gauge(
                    &format!("colonies.{}", s.id),
                    &format!("{} colonies", s.name),
                    "",
                    "Living colonies.",
                ));
            }
        }
    }
    if !matches!(rules, Rules::NetLogo(_)) {
        specs.extend([
            MetricSpec::counter(
                "fights",
                "Territorial fights",
                "Fights between territory owners and intruders this tick.",
            ),
            MetricSpec::gauge("alarms", "Alarm calls", "", "Animals calling an alarm."),
            MetricSpec::gauge("hibernating", "Hibernating", "", "Animals dormant in their dens."),
            MetricSpec::gauge("migrating", "Migrating", "", "Animals travelling towards their preferred climate."),
            MetricSpec::gauge("airborne", "Airborne", "", "Fliers more than 1 m above the ground."),
            MetricSpec::gauge("injured", "Injured", "", "Animals with an injury above 0.1."),
            MetricSpec::ratio(
                "flowers.pollination",
                "Pollination",
                "Mean pollination index of the flowers (biomass weighted).",
            ),
            MetricSpec::gauge(
                "flowers.seed_production",
                "Flower seed production",
                "seeds per day",
                "Seeds set by the flower layer on the last day (rises with pollination).",
            ),
            MetricSpec::gauge("pheromone", "Trail pheromone", "", "Total ant trail pheromone on the map."),
        ]);
        for p in &config.pathogens {
            specs.push(MetricSpec::gauge(
                &format!("infected.{}", p.id),
                &format!("Infected ({})", p.name),
                "",
                "Infectious animals.",
            ));
            specs.push(MetricSpec::gauge(
                &format!("recovered.{}", p.id),
                &format!("Recovered ({})", p.name),
                "",
                "Recovered animals still immune.",
            ));
            specs.push(MetricSpec::counter(
                &format!("new_infections.{}", p.id),
                &format!("New infections ({})", p.name),
                "Infections this tick.",
            ));
            specs.push(MetricSpec::gauge(
                &format!("disease_deaths.{}", p.id),
                &format!("Disease deaths ({})", p.name),
                "",
                "Deaths from the infection since the start.",
            ));
        }
    }
    specs.extend([
        MetricSpec::counter("attacks", "Attacks", "Attacks resolved this tick."),
        MetricSpec::counter("kills", "Kills", "Successful attacks this tick."),
        MetricSpec::gauge("mean_age_days", "Mean age", "days", "Mean age of living animals."),
        MetricSpec::gauge("richness", "Species richness", "", "Animal species with at least one individual."),
        MetricSpec::gauge("shannon", "Shannon diversity", "nats", "Shannon index of animal abundances."),
        MetricSpec::ratio("simpson", "Gini–Simpson diversity", "1 − Σ p² of animal abundances."),
    ]);
    if let Rules::NetLogo(_) = rules {
        specs.push(MetricSpec::gauge("green_patches", "Green patches", "", "Patches with grass (NetLogo variant)."));
    } else {
        specs.extend([
            MetricSpec::gauge(
                "biomass_producers",
                "Producer biomass",
                "kg",
                "Vegetation layers and trees (dry matter).",
            ),
            MetricSpec::gauge("biomass_primary_consumers", "Herbivore biomass", "kg", "Animals of trophic level 2."),
            MetricSpec::gauge(
                "biomass_secondary_consumers",
                "Omnivore and carnivore biomass",
                "kg",
                "Non-apex consumers of animals.",
            ),
            MetricSpec::gauge("biomass_apex", "Apex predator biomass", "kg", "Apex predators."),
            MetricSpec::gauge("biomass_decomposers", "Fungal biomass", "kg", "Mycelium, mycorrhiza and mushrooms."),
            MetricSpec::gauge("energy.primary_production", "Primary production", "kJ per tick", "Net plant growth."),
            MetricSpec::gauge("energy.herbivory", "Herbivory", "kJ per tick", "Plant energy eaten."),
            MetricSpec::gauge("energy.predation", "Predation", "kJ per tick", "Energy of fresh kills eaten."),
            MetricSpec::gauge("energy.scavenging", "Scavenging", "kJ per tick", "Carrion eaten."),
            MetricSpec::gauge(
                "energy.animal_respiration",
                "Animal respiration",
                "kJ per tick",
                "Metabolism of animals.",
            ),
            MetricSpec::gauge(
                "energy.decomposer_respiration",
                "Decomposer respiration",
                "kJ per tick",
                "Respiration of decomposition.",
            ),
            MetricSpec::gauge("energy.combustion", "Combustion", "kJ per tick", "Biomass energy burned."),
            MetricSpec::gauge("nutrients.soil_mineral", "Soil mineral nutrients", "kg", "Mineral nutrient pool."),
            MetricSpec::gauge("nutrients.soil_organic", "Soil organic nutrients", "kg", "Soil organic matter."),
            MetricSpec::gauge("nutrients.plants", "Plant nutrients", "kg", "Vegetation and trees."),
            MetricSpec::gauge("nutrients.animals", "Animal nutrients", "kg", "Bodies, stores and embryos."),
            MetricSpec::gauge("nutrients.carcasses", "Carcass nutrients", "kg", "Carcasses and eggs."),
            MetricSpec::gauge("nutrients.detritus", "Detritus nutrients", "kg", "Litter, dung and dead wood."),
            MetricSpec::gauge("nutrients.fungi", "Fungal nutrients", "kg", "Fungi."),
            MetricSpec::ratio("nutrient_balance_error", "Nutrient balance error", "|stock − books| / stock."),
            MetricSpec::ratio("energy_balance_error", "Energy balance error", "|stock − books| / stock."),
            MetricSpec::gauge("burned_biomass", "Burned biomass", "kg per tick", "Biomass burned this tick."),
        ]);
    }
    specs.extend([
        MetricSpec::ratio("burned_area", "Burned area", "Fraction of non-ocean cells in the burned state."),
        MetricSpec::gauge("burning_cells", "Burning cells", "", "Cells on fire."),
        MetricSpec::gauge("rain", "Rain", "mm per tick", "Mean precipitation over land."),
        MetricSpec::gauge("water_volume", "Surface water", "m³", "Water on land, in lakes and rivers."),
        MetricSpec::ratio("soil_moisture", "Soil moisture", "Mean relative saturation of land."),
        MetricSpec::gauge("mean_temperature", "Temperature", "°C", "Mean air temperature over land."),
        MetricSpec::ratio("daylight", "Daylight", "Sun-elevation proxy."),
        MetricSpec::ratio("drought_index", "Drought index", "Precipitation and soil-moisture deficit."),
    ]);
    specs
}

impl EcoState {
    /// Measurements of the current tick.
    pub(crate) fn measure(&self) -> Metrics {
        let minutes = self.world.calendar().minutes_per_tick();
        let mut m = Metrics::new(self.tick, self.tick as f64 * minutes);
        let n = self.catalog.animals().len();
        let mut counts = vec![0u64; n];
        let mut age_sum = 0.0f64;
        let tpd = f64::from(self.world.calendar().ticks_per_day());
        let mut consumer_mass = [0.0f64; 3];
        let (mut hibernating, mut migrating, mut airborne, mut injured) = (0u32, 0u32, 0u32, 0u32);
        for a in self.animals.as_slice() {
            let s = usize::from(a.species);
            if s < n {
                counts[s] += 1;
                let level = trophic_level(&self.catalog.animals()[s]) as usize;
                consumer_mass[level] += f64::from(a.mass_kg);
            }
            age_sum += a.age_ticks(self.tick) as f64 / tpd;
            hibernating += u32::from(a.action == Action::Hibernate);
            migrating += u32::from(a.action == Action::Migrate);
            airborne += u32::from(a.altitude_m > crate::animal::perceive::AIRBORNE_M);
            injured += u32::from(a.injury > 0.1);
        }
        let area = self.cell_area();
        let mut aggregate_present = 0usize;
        let mut aggregate = vec![None; n];
        for slot in 0..self.fields.len() {
            let s = usize::from(self.fields.species_of(slot));
            let individuals = self.fields.population(slot, area);
            let mass = f64::from(self.catalog.animals()[s].body.mass_kg);
            consumer_mass[trophic_level(&self.catalog.animals()[s]) as usize] += individuals * mass;
            aggregate_present += usize::from(individuals >= 1.0);
            aggregate[s] = Some(individuals);
        }
        for (s, species) in self.catalog.animals().iter().enumerate() {
            if species.representation.colony().is_some() {
                let workers = self.colonies.workers(s as u16);
                consumer_mass[trophic_level(species) as usize] += workers * f64::from(species.body.mass_kg);
                aggregate_present += usize::from(self.colonies.count(s as u16) > 0);
                aggregate[s] = Some(workers);
            }
        }
        m.push("animals", self.animals.len() as f64);
        m.push("trees", self.trees.as_slice().iter().filter(|t| t.is_alive()).count() as f64);
        for (i, s) in self.catalog.animals().iter().enumerate() {
            match aggregate[i] {
                Some(population) => {
                    m.push(&format!("population.{}", s.id), population);
                    if s.representation.colony().is_some() {
                        m.push(&format!("colonies.{}", s.id), self.colonies.count(i as u16) as f64);
                    }
                }
                None => {
                    m.push(&format!("population.{}", s.id), counts[i] as f64);
                    m.push(&format!("births.{}", s.id), f64::from(self.counters.births.get(i).copied().unwrap_or(0)));
                    m.push(&format!("deaths.{}", s.id), f64::from(self.counters.deaths.get(i).copied().unwrap_or(0)));
                }
            }
        }
        m.push("attacks", f64::from(self.counters.attacks));
        m.push("kills", f64::from(self.counters.kills));
        m.push("mean_age_days", if self.animals.is_empty() { 0.0 } else { age_sum / self.animals.len() as f64 });
        let diversity = Diversity::from_counts(&counts);
        m.push("richness", (diversity.richness + aggregate_present) as f64);
        m.push("shannon", diversity.shannon);
        m.push("simpson", diversity.simpson);
        match &self.netlogo {
            Some(nl) => {
                m.push("green_patches", nl.green_patches() as f64);
            }
            None => {
                m.push("fights", f64::from(self.counters.fights));
                m.push("alarms", f64::from(self.counters.alarms));
                m.push("hibernating", f64::from(hibernating));
                m.push("migrating", f64::from(migrating));
                m.push("airborne", f64::from(airborne));
                m.push("injured", f64::from(injured));
                m.push("flowers.pollination", self.vegetation.flower_pollination());
                m.push("flowers.seed_production", self.vegetation.flower_seed_production());
                m.push("pheromone", self.colonies.pheromone.iter().map(|p| f64::from(*p)).sum());
                self.measure_disease(&mut m);
                self.measure_ecology(&mut m, consumer_mass);
            }
        }
        let world = &self.world;
        let non_ocean = world.surface().as_slice().iter().filter(|s| **s != SurfaceKind::Ocean).count().max(1);
        let burned = world.fire().as_slice().iter().filter(|s| matches!(s, FireState::Burned { .. })).count();
        m.push("burned_area", burned as f64 / non_ocean as f64);
        m.push("burning_cells", world.burning_cells().len() as f64);
        m.push("rain", self.counters.rain_mm);
        m.push("water_volume", world.water_storage().surface_m3);
        m.push("soil_moisture", f64::from(world.mean_land_moisture()));
        m.push("mean_temperature", f64::from(world.mean_land_temperature()));
        m.push("daylight", f64::from(world.daylight()));
        m.push("drought_index", f64::from(world.drought_index()));
        m
    }

    fn measure_disease(&self, m: &mut Metrics) {
        for (p, pathogen) in self.config.pathogens.iter().enumerate() {
            let (mut infected, mut recovered) = (0u32, 0u32);
            for a in self.animals.as_slice() {
                match a.infection {
                    Some(i) if usize::from(i.pathogen) == p => {
                        if i.is_infected() {
                            infected += 1;
                        } else {
                            recovered += 1;
                        }
                    }
                    _ => {}
                }
            }
            m.push(&format!("infected.{}", pathogen.id), f64::from(infected));
            m.push(&format!("recovered.{}", pathogen.id), f64::from(recovered));
            m.push(&format!("new_infections.{}", pathogen.id), f64::from(self.disease.new_infections[p]));
            m.push(&format!("disease_deaths.{}", pathogen.id), self.disease.deaths[p] as f64);
        }
    }

    fn measure_ecology(&self, m: &mut Metrics, consumer_mass: [f64; 3]) {
        let stocks = self.stocks();
        let area = self.cell_area();
        let tree_kg: f64 = self.trees.as_slice().iter().map(|t| f64::from(t.wood_kg + t.leaf_kg)).sum();
        m.push("biomass_producers", self.vegetation.all_total_kg(area) + tree_kg);
        m.push("biomass_primary_consumers", consumer_mass[0]);
        m.push("biomass_secondary_consumers", consumer_mass[1]);
        m.push("biomass_apex", consumer_mass[2]);
        m.push("biomass_decomposers", stocks.fungi.0 / crate::units::FUNGI_KJ_PER_KG);
        let e = &self.ledgers.energy;
        for flux in [
            Flux::PrimaryProduction,
            Flux::Herbivory,
            Flux::Predation,
            Flux::Scavenging,
            Flux::AnimalRespiration,
            Flux::DecomposerRespiration,
            Flux::Combustion,
        ] {
            m.push(&format!("energy.{}", flux.key()), e.last_tick(flux));
        }
        m.push("nutrients.soil_mineral", stocks.soil_mineral_g / 1000.0);
        m.push("nutrients.soil_organic", stocks.soil_organic_g / 1000.0);
        m.push("nutrients.plants", (stocks.vegetation.1 + stocks.trees.1) / 1000.0);
        m.push("nutrients.animals", stocks.animals.1 / 1000.0);
        m.push("nutrients.carcasses", (stocks.carcasses.1 + stocks.clutches.1) / 1000.0);
        m.push("nutrients.detritus", (stocks.litter.1 + stocks.deadwood.1) / 1000.0);
        m.push("nutrients.fungi", stocks.fungi.1 / 1000.0);
        m.push("nutrient_balance_error", self.ledgers.nutrients.relative_error(stocks.nutrients()));
        m.push("energy_balance_error", self.ledgers.energy.relative_error(stocks.energy()));
        m.push("burned_biomass", self.counters.burned_kg);
    }
}
