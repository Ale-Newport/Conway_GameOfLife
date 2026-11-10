//! Carcasses (food for predators and scavengers, decaying into litter) and
//! clutches of eggs (incubating until they hatch).

use organism_engine::{AgentId, Genome, Lineage};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

use crate::animal::DeathCause;
use crate::fungi::Detritus;
use crate::ledger::{Flux, Ledgers};
use crate::units::q10_factor;

/// Carcass decay at 20 °C (fraction of the remaining meat per day; insects
/// and microbes), with Q10 = 2.
pub const CARCASS_DECAY_PER_DAY: f32 = 0.15;
/// Remains below this mass (kg) go to the litter at once.
pub const MIN_CARCASS_KG: f32 = 0.05;

/// A dead animal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Carcass {
    /// Identifier (unique within the run).
    pub id: u64,
    /// Species of the dead animal.
    pub species: u16,
    /// Position (cells).
    pub position: Vec2,
    /// Remaining edible mass (kg).
    pub meat_kg: f32,
    /// Initial mass (kg).
    pub initial_kg: f32,
    /// Remaining energy (kJ).
    pub energy_kj: f64,
    /// Remaining nutrients (g).
    pub nutrients_g: f64,
    /// Tick of death.
    pub created_tick: u64,
    /// Cause of death.
    pub cause: DeathCause,
    /// Predator that killed it (feeds first).
    pub killer: Option<AgentId>,
}

impl Carcass {
    /// Removes up to `kg` of meat and returns `(kg, kJ, g)` taken.
    pub fn take(&mut self, kg: f32) -> (f32, f64, f64) {
        let taken = kg.max(0.0).min(self.meat_kg);
        if taken <= 0.0 || self.meat_kg <= 0.0 {
            return (0.0, 0.0, 0.0);
        }
        let share = f64::from(taken / self.meat_kg);
        let (kj, g) = if taken >= self.meat_kg {
            (self.energy_kj, self.nutrients_g)
        } else {
            (self.energy_kj * share, self.nutrients_g * share)
        };
        self.meat_kg -= taken;
        self.energy_kj = (self.energy_kj - kj).max(0.0);
        self.nutrients_g = (self.nutrients_g - g).max(0.0);
        (taken, kj, g)
    }
}

/// A clutch of eggs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clutch {
    /// Identifier (unique within the run).
    pub id: u64,
    /// Species.
    pub species: u16,
    /// Nest position (cells).
    pub position: Vec2,
    /// Number of eggs.
    pub eggs: u16,
    /// Tick of laying.
    pub laid_tick: u64,
    /// Tick of hatching.
    pub hatch_tick: u64,
    /// Mother.
    pub mother: AgentId,
    /// Mother's lineage.
    pub mother_lineage: Lineage,
    /// Mother's genome.
    pub mother_genome: Genome,
    /// Father's lineage, if any.
    pub father_lineage: Option<Lineage>,
    /// Father's genome, if any.
    pub father_genome: Option<Genome>,
    /// Energy in the eggs (kJ).
    pub energy_kj: f64,
    /// Nutrients in the eggs (g).
    pub nutrients_g: f64,
}

impl Clutch {
    /// Egg mass (kg, as wet tissue of the clutch's energy).
    pub fn egg_kg(&self) -> f32 {
        (self.energy_kj / crate::units::TISSUE_KJ_PER_KG) as f32
    }

    /// Removes up to `kg` of eggs (nest predation) and returns the energy
    /// (kJ) and nutrients (g) taken. The egg count shrinks in proportion,
    /// rounded down: a partly eaten egg is lost, so every remaining egg
    /// keeps at least the energy and nutrients it was laid with (rounding
    /// up would let hatchlings carry more than their eggs contained).
    pub fn take(&mut self, kg: f32) -> (f64, f64) {
        let total = self.egg_kg();
        if total <= 0.0 || kg <= 0.0 {
            return (0.0, 0.0);
        }
        let share = f64::from((kg / total).min(1.0));
        let (kj, g) = (self.energy_kj * share, self.nutrients_g * share);
        self.energy_kj -= kj;
        self.nutrients_g -= g;
        self.eggs = (f64::from(self.eggs) * (1.0 - share)).floor() as u16;
        if self.eggs == 0 {
            // The rest is too little for an egg: it is eaten too.
            let (rest_kj, rest_g) = (self.energy_kj, self.nutrients_g);
            self.energy_kj = 0.0;
            self.nutrients_g = 0.0;
            return (kj + rest_kj, g + rest_g);
        }
        (kj, g)
    }
}

/// Spatial and mother index of the clutches, rebuilt with the animal index
/// so that perception finds an animal's own clutch and the nearest clutch
/// it can raid without scanning every clutch (slot = index in
/// [`Remains::clutches`], which is in laying order).
#[derive(Clone, Debug)]
pub(crate) struct ClutchIndex {
    /// Positions of the clutches.
    pub grid: simulation_core::SpatialGrid,
    /// `(mother, slot)` sorted by mother then slot.
    pub mothers: Vec<(AgentId, u32)>,
}

impl ClutchIndex {
    /// An empty index for a `width × height` map.
    pub fn new(width: f32, height: f32) -> Self {
        Self { grid: simulation_core::SpatialGrid::new(width, height, 8.0), mothers: Vec::new() }
    }

    /// Rebuilds the index from the clutches.
    pub fn rebuild(&mut self, clutches: &[Clutch]) {
        self.grid.rebuild(clutches.iter().enumerate().map(|(i, c)| (i as u32, c.position)));
        self.mothers.clear();
        self.mothers.extend(clutches.iter().enumerate().map(|(i, c)| (c.mother, i as u32)));
        self.mothers.sort_unstable();
    }

    /// Slot of the first clutch (in laying order) of a mother.
    pub fn of_mother(&self, mother: AgentId) -> Option<u32> {
        let i = self.mothers.partition_point(|(m, _)| *m < mother);
        self.mothers.get(i).filter(|(m, _)| *m == mother).map(|(_, slot)| *slot)
    }
}

/// Carcasses and clutches with their id counters.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Remains {
    /// Carcasses in creation order.
    pub carcasses: Vec<Carcass>,
    /// Clutches in laying order.
    pub clutches: Vec<Clutch>,
    next_id: u64,
}

impl Remains {
    /// Issues a new id.
    pub(crate) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Whether ids are unique, increasing and below the counter (snapshot check).
    pub(crate) fn is_consistent(&self) -> bool {
        let increasing =
            |ids: Vec<u64>| ids.windows(2).all(|w| w[0] < w[1]) && ids.iter().all(|&i| i >= 1 && i <= self.next_id);
        increasing(self.carcasses.iter().map(|c| c.id).collect())
            && increasing(self.clutches.iter().map(|c| c.id).collect())
    }

    /// Carcass by id.
    pub fn carcass(&self, id: u64) -> Option<&Carcass> {
        self.carcasses.binary_search_by_key(&id, |c| c.id).ok().map(|i| &self.carcasses[i])
    }

    /// Clutch by id (clutches are in increasing id order).
    pub fn clutch(&self, id: u64) -> Option<&Clutch> {
        self.clutches.binary_search_by_key(&id, |c| c.id).ok().map(|i| &self.clutches[i])
    }

    /// Mutable clutch by id.
    pub(crate) fn clutch_mut(&mut self, id: u64) -> Option<&mut Clutch> {
        self.clutches.binary_search_by_key(&id, |c| c.id).ok().map(|i| &mut self.clutches[i])
    }

    /// Mutable carcass by id.
    pub(crate) fn carcass_mut(&mut self, id: u64) -> Option<&mut Carcass> {
        self.carcasses.binary_search_by_key(&id, |c| c.id).ok().map(|i| &mut self.carcasses[i])
    }

    /// Energy (kJ) and nutrients (g) of all carcasses.
    pub fn carcass_stock(&self) -> (f64, f64) {
        self.carcasses.iter().fold((0.0, 0.0), |(e, n), c| (e + c.energy_kj, n + c.nutrients_g))
    }

    /// Energy (kJ) and nutrients (g) of all clutches.
    pub fn clutch_stock(&self) -> (f64, f64) {
        self.clutches.iter().fold((0.0, 0.0), |(e, n), c| (e + c.energy_kj, n + c.nutrients_g))
    }

    /// Hourly decay of carcasses into the litter of their cell; exhausted
    /// carcasses are removed. `cell_of` maps a position to a cell index and
    /// `temperature` gives the air temperature of a cell.
    pub(crate) fn decay(
        &mut self,
        dt_days: f32,
        cell_area: f64,
        cell_of: impl Fn(Vec2) -> usize,
        temperature: impl Fn(usize) -> f32,
        detritus: &mut Detritus,
        ledgers: &mut Ledgers,
    ) {
        for carcass in &mut self.carcasses {
            let cell = cell_of(carcass.position);
            let rate = CARCASS_DECAY_PER_DAY * q10_factor(temperature(cell), 2.0, 20.0).max(0.05);
            let amount = if carcass.meat_kg - carcass.meat_kg * (rate * dt_days).min(1.0) < MIN_CARCASS_KG {
                carcass.meat_kg
            } else {
                carcass.meat_kg * (rate * dt_days).min(1.0)
            };
            let (_, kj, g) = carcass.take(amount);
            if kj > 0.0 || g > 0.0 {
                detritus.add_litter_energy(cell, kj / cell_area, (g / cell_area) as f32);
                ledgers.book(Flux::CarcassDecay, kj, g);
            }
        }
        self.carcasses.retain(|c| c.meat_kg > 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::TISSUE_KJ_PER_KG;

    fn carcass(kg: f32) -> Carcass {
        Carcass {
            id: 1,
            species: 0,
            position: Vec2::new(0.5, 0.5),
            meat_kg: kg,
            initial_kg: kg,
            energy_kj: f64::from(kg) * TISSUE_KJ_PER_KG,
            nutrients_g: f64::from(kg) * 30.0,
            created_tick: 0,
            cause: DeathCause::Predation,
            killer: None,
        }
    }

    #[test]
    fn take_is_proportional_and_exhaustive() {
        let mut c = carcass(10.0);
        let (kg, kj, g) = c.take(2.5);
        assert_eq!(kg, 2.5);
        assert!((kj - 2.5 * TISSUE_KJ_PER_KG).abs() < 1e-6 && (g - 75.0).abs() < 1e-9);
        let (kg, kj, _) = c.take(100.0);
        assert_eq!(kg, 7.5);
        assert!((kj - 7.5 * TISSUE_KJ_PER_KG).abs() < 1e-3);
        assert_eq!((c.meat_kg, c.energy_kj, c.nutrients_g), (0.0, 0.0, 0.0));
    }

    #[test]
    fn partly_eaten_clutches_never_enrich_the_remaining_eggs_below_their_cost() {
        // Every remaining egg keeps at least the energy and nutrients it was
        // laid with, whatever bite a nest predator takes.
        for bite in [0.001f32, 0.1, 0.25, 0.4, 0.5, 0.66, 0.9, 0.999] {
            for eggs in 1u16..=12 {
                let per_kj = 40.0;
                let per_g = 0.2;
                let mut clutch = Clutch {
                    id: 1,
                    species: 0,
                    position: Vec2::ZERO,
                    eggs,
                    laid_tick: 0,
                    hatch_tick: 10,
                    mother: AgentId(1),
                    mother_lineage: Lineage::founder(AgentId(1), 0),
                    mother_genome: Genome::defaults(&crate::animal::genetics::schema()),
                    father_lineage: None,
                    father_genome: None,
                    energy_kj: per_kj * f64::from(eggs),
                    nutrients_g: per_g * f64::from(eggs),
                };
                let before = (clutch.energy_kj, clutch.nutrients_g);
                let (kj, g) = clutch.take(bite * clutch.egg_kg());
                assert!((kj + clutch.energy_kj - before.0).abs() < 1e-9);
                assert!((g + clutch.nutrients_g - before.1).abs() < 1e-12);
                if clutch.eggs > 0 {
                    let n = f64::from(clutch.eggs);
                    assert!(
                        clutch.energy_kj / n >= per_kj - 1e-9,
                        "{eggs} eggs, bite {bite}: {}",
                        clutch.energy_kj / n
                    );
                    assert!(clutch.nutrients_g / n >= per_g - 1e-12, "{eggs} eggs, bite {bite}");
                } else {
                    assert_eq!((clutch.energy_kj, clutch.nutrients_g), (0.0, 0.0));
                }
            }
        }
    }

    #[test]
    fn decay_moves_everything_to_litter() {
        let mut remains = Remains::default();
        let mut c = carcass(5.0);
        c.id = remains.next_id();
        remains.carcasses.push(c);
        let mut detritus = Detritus::new(1, 1);
        let mut ledgers = Ledgers::default();
        let before = remains.carcass_stock();
        for _ in 0..2000 {
            remains.decay(1.0 / 24.0, 1.0, |_| 0, |_| 20.0, &mut detritus, &mut ledgers);
        }
        assert!(remains.carcasses.is_empty());
        let (kj, g) = detritus.litter_stock(1.0);
        assert!((kj - before.0).abs() / before.0 < 1e-6 && (g - before.1).abs() / before.1 < 1e-5);
    }
}
