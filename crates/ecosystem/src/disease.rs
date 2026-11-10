//! Infectious disease with SIR dynamics emerging from contacts between
//! individual animals.
//!
//! A scenario declares [`Pathogen`]s (hosts, transmission per contact,
//! infectious period, mortality, immunity, virulence); outbreaks are
//! started by scheduled events or the `Outbreak` command. Per tick
//! (`Δt` in hours or days):
//!
//! ```text
//! transmission  a susceptible host with k infectious hosts of the same pathogen within the contact
//!               distance is infected with P = 1 − (1 − β)^{k Δt_h}            (β per contact-hour)
//! recovery      P = 1 − e^{−Δt_d / infectious_days}  → recovered (immune for `immunity_days`, 0 = for life)
//! death         P = 1 − e^{−μ Δt_d}                    (DeathCause::Disease)
//! sickness      speed × (1 − ½ virulence), metabolism × (1 + ½ virulence) while infected
//! waning        recovered hosts become susceptible again when their immunity ends
//! ```
//!
//! Draws come from one stream per `(tick, animal)`; the counting of
//! infectious neighbours is a pure parallel phase over a spatial grid, the
//! state changes are applied serially in id order. An animal carries at
//! most one infection record: infection by another pathogen replaces an
//! old immunity (a documented simplification).

use organism_engine::AgentId;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simulation_core::SpatialGrid;

use crate::animal::Animal;
use crate::error::EcosystemError;
use crate::species::SpeciesCatalog;
use crate::units::TimeScale;

/// Infection stage.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfectionStage {
    /// Infectious since a tick.
    Infected {
        /// Tick of infection.
        since: u64,
    },
    /// Recovered and immune until a tick (`u64::MAX` = for life).
    Recovered {
        /// Tick at which immunity ends.
        until: u64,
    },
}

/// The infection record of an animal.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Infection {
    /// Index of the pathogen in the configuration.
    pub pathogen: u16,
    /// Stage.
    pub stage: InfectionStage,
}

impl Infection {
    /// Whether the animal is infectious.
    pub fn is_infected(&self) -> bool {
        matches!(self.stage, InfectionStage::Infected { .. })
    }
}

/// A pathogen of a scenario.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pathogen {
    /// Identifier (lowercase; used in metric keys).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Host species (individual species ids).
    pub hosts: Vec<String>,
    /// Probability per hour that one infectious host within the contact
    /// distance infects a susceptible host.
    pub transmission_per_hour: f32,
    /// Contact distance (m).
    pub contact_m: f32,
    /// Mean infectious period (simulated days).
    pub infectious_days: f32,
    /// Death rate of infected hosts (per simulated day).
    pub mortality_per_day: f32,
    /// Immunity after recovery (simulated days; 0 = for life).
    #[serde(default)]
    pub immunity_days: f32,
    /// Sickness in `[0, 1]`: speed × (1 − ½ v), metabolism × (1 + ½ v).
    #[serde(default)]
    pub virulence: f32,
}

impl Pathogen {
    /// Validates the record against a catalogue.
    pub fn validate(&self, catalog: &SpeciesCatalog) -> Result<(), EcosystemError> {
        let field = |name: &str| format!("pathogens.{}.{name}", self.id);
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 64
            && self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if !id_ok {
            return Err(EcosystemError::config(
                "pathogens.id",
                format!("`{}` must use lowercase letters, digits, `_` or `-`", self.id),
            ));
        }
        if self.hosts.is_empty() {
            return Err(EcosystemError::config(&field("hosts"), "must list at least one host species"));
        }
        for host in &self.hosts {
            let index = catalog.animal_index(host)?;
            if !catalog.is_individual(index) {
                return Err(EcosystemError::config(
                    &field("hosts"),
                    format!(
                        "`{host}` is not simulated as individuals (density fields and colonies carry no infections)"
                    ),
                ));
            }
        }
        for (name, value, lo, hi) in [
            ("transmission_per_hour", self.transmission_per_hour, 0.0, 1.0),
            ("contact_m", self.contact_m, 0.0, 10_000.0),
            ("infectious_days", self.infectious_days, 0.01, 10_000.0),
            ("mortality_per_day", self.mortality_per_day, 0.0, 100.0),
            ("immunity_days", self.immunity_days, 0.0, 1e6),
            ("virulence", self.virulence, 0.0, 1.0),
        ] {
            if !(value.is_finite() && (lo..=hi).contains(&value)) {
                return Err(EcosystemError::config(&field(name), format!("{value} is outside [{lo}, {hi}]")));
            }
        }
        Ok(())
    }
}

/// Pathogens resolved for a run: host masks per species.
#[derive(Clone, Debug, Default)]
pub(crate) struct Epidemiology {
    pub pathogens: Vec<Pathogen>,
    /// `hosts[p][species]`.
    pub hosts: Vec<Vec<bool>>,
}

impl Epidemiology {
    /// Resolves the configured pathogens (validated with the configuration).
    pub fn new(pathogens: &[Pathogen], catalog: &SpeciesCatalog) -> Self {
        let n = catalog.animals().len();
        let hosts = pathogens
            .iter()
            .map(|p| {
                let mut mask = vec![false; n];
                for h in &p.hosts {
                    if let Ok(i) = catalog.animal_index(h) {
                        mask[usize::from(i)] = true;
                    }
                }
                mask
            })
            .collect();
        Self { pathogens: pathogens.to_vec(), hosts }
    }

    /// Whether `species` hosts pathogen `p`.
    pub fn hosts(&self, p: usize, species: u16) -> bool {
        self.hosts.get(p).and_then(|m| m.get(usize::from(species))).copied().unwrap_or(false)
    }

    /// Sickness (virulence) of an animal's current infection.
    pub fn virulence(&self, a: &Animal) -> f32 {
        match a.infection {
            Some(i) if i.is_infected() => self.pathogens.get(usize::from(i.pathogen)).map_or(0.0, |p| p.virulence),
            _ => 0.0,
        }
    }
}

/// Per-pathogen counters.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiseaseCounters {
    /// New infections during the last tick.
    pub new_infections: Vec<u32>,
    /// Infections since the start.
    pub total_infections: Vec<u64>,
    /// Recoveries since the start.
    pub recoveries: Vec<u64>,
    /// Deaths since the start.
    pub deaths: Vec<u64>,
}

impl DiseaseCounters {
    /// Counters for `n` pathogens.
    pub fn new(n: usize) -> Self {
        Self { new_infections: vec![0; n], total_infections: vec![0; n], recoveries: vec![0; n], deaths: vec![0; n] }
    }

    /// Whether the counters match `n` pathogens.
    pub(crate) fn is_consistent(&self, n: usize) -> bool {
        [self.new_infections.len(), self.total_infections.len(), self.recoveries.len(), self.deaths.len()]
            .iter()
            .all(|&l| l == n)
    }
}

/// What happens to each animal this tick (store-index order).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DiseaseEvent {
    /// Becomes infected with a pathogen.
    Infected(u16),
    /// Recovers.
    Recovered,
    /// Dies of its infection.
    Died,
    /// Its immunity ends.
    Susceptible,
}

/// Pure phase: decides every animal's disease event from the current state
/// (`grid` slots are store indices).
pub(crate) fn transmission(
    epi: &Epidemiology,
    animals: &[Animal],
    ids: &[AgentId],
    grid: &SpatialGrid,
    scale: &TimeScale,
    seed: u64,
    tick: u64,
) -> Vec<Option<DiseaseEvent>> {
    if epi.pathogens.is_empty() || !animals.iter().any(|a| a.infection.is_some_and(|i| i.is_infected())) {
        return vec![None; animals.len()];
    }
    let dt_h = f64::from(scale.dt_hours());
    let dt_d = f64::from(scale.dt_days());
    (0..animals.len())
        .into_par_iter()
        .map(|i| {
            let a = &animals[i];
            let mut rng = crate::rng::stream(seed, crate::rng::DISEASE, tick, ids[i].get());
            match a.infection {
                Some(Infection { pathogen, stage: InfectionStage::Infected { .. } }) => {
                    let p = &epi.pathogens[usize::from(pathogen)];
                    if rng.chance(1.0 - (-f64::from(p.mortality_per_day) * dt_d).exp()) {
                        Some(DiseaseEvent::Died)
                    } else if rng.chance(1.0 - (-dt_d / f64::from(p.infectious_days)).exp()) {
                        Some(DiseaseEvent::Recovered)
                    } else {
                        None
                    }
                }
                Some(Infection { stage: InfectionStage::Recovered { until }, pathogen }) if tick < until => {
                    // Immune to `pathogen`; still exposed to the others.
                    exposure(epi, animals, grid, scale, i, Some(pathogen), dt_h, &mut rng)
                }
                Some(Infection { stage: InfectionStage::Recovered { .. }, .. }) => Some(DiseaseEvent::Susceptible),
                None => exposure(epi, animals, grid, scale, i, None, dt_h, &mut rng),
            }
        })
        .collect()
}

/// Infection draw of a susceptible animal (pathogens in index order).
#[allow(clippy::too_many_arguments)]
fn exposure(
    epi: &Epidemiology,
    animals: &[Animal],
    grid: &SpatialGrid,
    scale: &TimeScale,
    index: usize,
    immune_to: Option<u16>,
    dt_h: f64,
    rng: &mut simulation_core::SimRng,
) -> Option<DiseaseEvent> {
    let me = &animals[index];
    for (p, pathogen) in epi.pathogens.iter().enumerate() {
        if immune_to == Some(p as u16) || !epi.hosts(p, me.species) || pathogen.transmission_per_hour <= 0.0 {
            continue;
        }
        let radius = scale.cells(pathogen.contact_m).max(0.05);
        let k = grid.count_within(me.kin.position, radius, |slot| {
            let o = &animals[slot as usize];
            slot as usize != index && o.infection.is_some_and(|i| i.is_infected() && usize::from(i.pathogen) == p)
        });
        if k == 0 {
            continue;
        }
        let escape = (1.0 - f64::from(pathogen.transmission_per_hour)).powf(k as f64 * dt_h);
        if rng.chance(1.0 - escape) {
            return Some(DiseaseEvent::Infected(p as u16));
        }
    }
    None
}

impl crate::state::EcoState {
    /// Applies the disease events decided by [`transmission`] (store-index
    /// order, skipping animals that already died this tick).
    pub(crate) fn apply_disease(
        &mut self,
        epi: &Epidemiology,
        events: &[Option<DiseaseEvent>],
        deaths: &mut crate::animal::physiology::Deaths,
        ticks_per_day: f64,
    ) {
        self.disease.new_infections.iter_mut().for_each(|n| *n = 0);
        let tick = self.tick;
        let ids: Vec<AgentId> = self.animals.ids().to_vec();
        for (index, event) in events.iter().enumerate() {
            let Some(event) = event else { continue };
            let Some(&id) = ids.get(index) else { continue };
            if deaths.contains_key(&id) {
                continue;
            }
            let Some(a) = self.animals.get_mut(id) else { continue };
            match *event {
                DiseaseEvent::Infected(p) => {
                    a.infection = Some(Infection { pathogen: p, stage: InfectionStage::Infected { since: tick } });
                    self.disease.new_infections[usize::from(p)] += 1;
                    self.disease.total_infections[usize::from(p)] += 1;
                }
                DiseaseEvent::Recovered => {
                    if let Some(i) = a.infection {
                        let days = epi.pathogens[usize::from(i.pathogen)].immunity_days;
                        let until =
                            if days <= 0.0 { u64::MAX } else { tick + (f64::from(days) * ticks_per_day).ceil() as u64 };
                        a.infection =
                            Some(Infection { pathogen: i.pathogen, stage: InfectionStage::Recovered { until } });
                        self.disease.recoveries[usize::from(i.pathogen)] += 1;
                    }
                }
                DiseaseEvent::Died => {
                    if let Some(i) = a.infection {
                        self.disease.deaths[usize::from(i.pathogen)] += 1;
                    }
                    deaths.insert(id, (crate::animal::DeathCause::Disease, None));
                }
                DiseaseEvent::Susceptible => a.infection = None,
            }
        }
    }
}
