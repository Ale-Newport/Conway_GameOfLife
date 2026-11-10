//! Explicit energy and nutrient accounting.
//!
//! Every transfer between pools is booked as a [`Flux`] in two ledgers, one
//! for energy (kJ) and one for nutrients (g). A flux is either an *input*
//! (enters the ecosystem: primary production, deposition, spawned animals),
//! an *output* (leaves it: respiration, combustion, volatilisation,
//! leaching, removed animals) or *internal* (moves between pools). The
//! balance check
//!
//! ```text
//! stock(now) = stock(start) + Σ inputs − Σ outputs
//! ```
//!
//! must hold to rounding error for both ledgers; with external nutrient
//! fluxes disabled the nutrient stock is constant.

use serde::{Deserialize, Serialize};

/// Direction of a flux relative to the ecosystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FluxKind {
    /// Enters the ecosystem.
    Input,
    /// Leaves the ecosystem.
    Output,
    /// Moves between pools.
    Internal,
    /// Not used by this ledger.
    Unused,
}

macro_rules! fluxes {
    ($( $(#[$doc:meta])* $name:ident => $key:literal, energy: $e:ident, nutrients: $n:ident; )*) => {
        /// A booked transfer.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum Flux {
            $( $(#[$doc])* $name, )*
        }

        impl Flux {
            /// Every flux in declaration order.
            pub const ALL: &'static [Flux] = &[$(Flux::$name),*];

            /// Stable machine key (metric suffix).
            pub fn key(self) -> &'static str {
                match self { $(Flux::$name => $key,)* }
            }

            /// Role of the flux in the energy ledger.
            pub fn energy_kind(self) -> FluxKind {
                match self { $(Flux::$name => FluxKind::$e,)* }
            }

            /// Role of the flux in the nutrient ledger.
            pub fn nutrient_kind(self) -> FluxKind {
                match self { $(Flux::$name => FluxKind::$n,)* }
            }
        }
    };
}

fluxes! {
    /// Net plant growth and germination (energy fixed from light; nutrients taken from the soil).
    PrimaryProduction => "primary_production", energy: Input, nutrients: Internal;
    /// Plant carbon allocated to mycorrhizal fungi.
    MycorrhizalCarbon => "mycorrhizal_carbon", energy: Input, nutrients: Internal;
    /// Soil nutrients immobilised by decomposer fungi.
    Immobilisation => "immobilisation", energy: Unused, nutrients: Internal;
    /// Atmospheric deposition onto land.
    Deposition => "deposition", energy: Unused, nutrients: Input;
    /// Volcanic ash deposited by the physics world.
    VolcanicDeposition => "volcanic_deposition", energy: Unused, nutrients: Input;
    /// Organisms added by commands.
    InterventionIn => "intervention_in", energy: Input, nutrients: Input;
    /// Organisms removed by commands.
    InterventionOut => "intervention_out", energy: Output, nutrients: Output;
    /// Grazing and browsing.
    Herbivory => "herbivory", energy: Internal, nutrients: Internal;
    /// Meat eaten from fresh kills.
    Predation => "predation", energy: Internal, nutrients: Internal;
    /// Carrion eaten by other animals.
    Scavenging => "scavenging", energy: Internal, nutrients: Internal;
    /// Nectar eaten (flower biomass).
    Nectarivory => "nectarivory", energy: Internal, nutrients: Internal;
    /// Litter eaten by detritivores.
    Detritivory => "detritivory", energy: Internal, nutrients: Internal;
    /// Mushrooms eaten.
    Fungivory => "fungivory", energy: Internal, nutrients: Internal;
    /// Unassimilated food returned as dung.
    Egestion => "egestion", energy: Internal, nutrients: Internal;
    /// Surplus nutrients returned to the soil (urine).
    Excretion => "excretion", energy: Unused, nutrients: Internal;
    /// Maintenance, activity, locomotion, thermoregulation and synthesis costs.
    AnimalRespiration => "animal_respiration", energy: Output, nutrients: Unused;
    /// Mother → embryos, clutches and suckling young.
    ReproductiveInvestment => "reproduction", energy: Internal, nutrients: Internal;
    /// Animals dying (bodies become carcasses).
    AnimalMortality => "animal_mortality", energy: Internal, nutrients: Internal;
    /// Carcass remains decaying into litter.
    CarcassDecay => "carcass_decay", energy: Internal, nutrients: Internal;
    /// Plant turnover, leaf fall and dieback into litter.
    Litterfall => "litterfall", energy: Internal, nutrients: Internal;
    /// Trees falling into the dead-wood pool.
    TreeMortality => "tree_mortality", energy: Internal, nutrients: Internal;
    /// Mineral nutrients released by decomposition.
    Mineralisation => "mineralisation", energy: Unused, nutrients: Internal;
    /// Decomposed matter assimilated by saprotrophic fungi.
    FungalGrowth => "fungal_growth", energy: Internal, nutrients: Internal;
    /// Dead fungi and rotting mushrooms into litter.
    FungalMortality => "fungal_mortality", energy: Internal, nutrients: Internal;
    /// Mycelium converted into mushrooms.
    Fruiting => "fruiting", energy: Internal, nutrients: Internal;
    /// Decomposer respiration (CO₂).
    DecomposerRespiration => "decomposer_respiration", energy: Output, nutrients: Unused;
    /// Decomposed matter humified into soil organic matter.
    Humification => "humification", energy: Output, nutrients: Internal;
    /// Biomass burned by fire.
    Combustion => "combustion", energy: Output, nutrients: Unused;
    /// Nutrients of burned biomass lost to the atmosphere.
    Volatilisation => "volatilisation", energy: Unused, nutrients: Output;
    /// Nutrients of burned biomass returned as ash.
    Ash => "ash", energy: Unused, nutrients: Internal;
    /// Nutrients leached to groundwater (physics world).
    Leaching => "leaching", energy: Unused, nutrients: Output;
    /// Leached nutrients retained in the root zone (closed budget).
    LeachingRetained => "leaching_retained", energy: Unused, nutrients: Internal;
    /// Reproductive energy that could not be used (respired).
    ReproductiveLoss => "reproductive_loss", energy: Output, nutrients: Unused;
}

/// Cumulative and last-tick totals of every flux of one ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    /// Stock when accounting started.
    pub initial_stock: f64,
    cumulative: Vec<f64>,
    tick: Vec<f64>,
    energy: bool,
}

impl Ledger {
    fn new(energy: bool) -> Self {
        Self { initial_stock: 0.0, cumulative: vec![0.0; Flux::ALL.len()], tick: vec![0.0; Flux::ALL.len()], energy }
    }

    /// Energy ledger (kJ).
    pub fn energy() -> Self {
        Self::new(true)
    }

    /// Nutrient ledger (g).
    pub fn nutrients() -> Self {
        Self::new(false)
    }

    fn kind(&self, flux: Flux) -> FluxKind {
        if self.energy { flux.energy_kind() } else { flux.nutrient_kind() }
    }

    /// Books an amount (negative or non-finite amounts are ignored: every
    /// call site books a non-negative transfer).
    pub fn book(&mut self, flux: Flux, amount: f64) {
        if amount.is_finite() && amount > 0.0 && self.kind(flux) != FluxKind::Unused {
            let i = flux as usize;
            self.cumulative[i] += amount;
            self.tick[i] += amount;
        }
    }

    /// Clears the per-tick totals (called at the start of every tick).
    pub fn start_tick(&mut self) {
        self.tick.iter_mut().for_each(|v| *v = 0.0);
    }

    /// Cumulative total of a flux.
    pub fn total(&self, flux: Flux) -> f64 {
        self.cumulative[flux as usize]
    }

    /// Total of a flux during the last tick.
    pub fn last_tick(&self, flux: Flux) -> f64 {
        self.tick[flux as usize]
    }

    /// Sum of all inputs so far.
    pub fn inputs(&self) -> f64 {
        self.sum(FluxKind::Input)
    }

    /// Sum of all outputs so far.
    pub fn outputs(&self) -> f64 {
        self.sum(FluxKind::Output)
    }

    fn sum(&self, kind: FluxKind) -> f64 {
        Flux::ALL.iter().filter(|f| self.kind(**f) == kind).map(|f| self.cumulative[*f as usize]).sum()
    }

    /// Stock implied by the books.
    pub fn expected_stock(&self) -> f64 {
        self.initial_stock + self.inputs() - self.outputs()
    }

    /// Relative balance error `|stock − expected| / max(stock, expected, 1)`.
    pub fn relative_error(&self, stock: f64) -> f64 {
        let expected = self.expected_stock();
        (stock - expected).abs() / stock.abs().max(expected.abs()).max(1.0)
    }

    /// Whether the ledger has the expected number of entries (snapshot check).
    pub(crate) fn is_consistent(&self) -> bool {
        self.cumulative.len() == Flux::ALL.len() && self.tick.len() == Flux::ALL.len()
    }
}

/// Both ledgers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ledgers {
    /// Energy (kJ).
    pub energy: Ledger,
    /// Nutrients (g).
    pub nutrients: Ledger,
}

impl Default for Ledgers {
    fn default() -> Self {
        Self { energy: Ledger::energy(), nutrients: Ledger::nutrients() }
    }
}

impl Ledgers {
    /// Books one transfer in both ledgers.
    pub fn book(&mut self, flux: Flux, energy_kj: f64, nutrients_g: f64) {
        self.energy.book(flux, energy_kj);
        self.nutrients.book(flux, nutrients_g);
    }

    /// Clears per-tick totals.
    pub fn start_tick(&mut self) {
        self.energy.start_tick();
        self.nutrients.start_tick();
    }
}

/// Energy and nutrient stocks of every pool at one instant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stocks {
    /// Vegetation layers (kJ, g).
    pub vegetation: (f64, f64),
    /// Living and standing dead trees.
    pub trees: (f64, f64),
    /// Animals (tissue, reserves, embryos).
    pub animals: (f64, f64),
    /// Egg clutches.
    pub clutches: (f64, f64),
    /// Carcasses.
    pub carcasses: (f64, f64),
    /// Litter and dung.
    pub litter: (f64, f64),
    /// Dead wood.
    pub deadwood: (f64, f64),
    /// Fungi (mycelium, mycorrhiza, mushrooms).
    pub fungi: (f64, f64),
    /// Density-field organisms (insects, spiders, worms).
    pub fields: (f64, f64),
    /// Colonies (workers, brood, food stores, forager loads).
    pub colonies: (f64, f64),
    /// Soil mineral nutrients (g).
    pub soil_mineral_g: f64,
    /// Soil organic nutrients (g).
    pub soil_organic_g: f64,
}

impl Stocks {
    /// Total energy of living and dead organic pools (kJ).
    pub fn energy(&self) -> f64 {
        self.biotic().iter().map(|p| p.0).sum()
    }

    /// Total nutrients including the soil (g).
    pub fn nutrients(&self) -> f64 {
        self.biotic().iter().map(|p| p.1).sum::<f64>() + self.soil_mineral_g + self.soil_organic_g
    }

    fn biotic(&self) -> [(f64, f64); 10] {
        [
            self.vegetation,
            self.trees,
            self.animals,
            self.clutches,
            self.carcasses,
            self.litter,
            self.deadwood,
            self.fungi,
            self.fields,
            self.colonies,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_closes_with_inputs_and_outputs() {
        let mut ledger = Ledger::energy();
        ledger.initial_stock = 100.0;
        ledger.book(Flux::PrimaryProduction, 30.0);
        ledger.book(Flux::Herbivory, 10.0);
        ledger.book(Flux::AnimalRespiration, 5.0);
        ledger.book(Flux::Excretion, 99.0); // unused in the energy ledger
        assert_eq!(ledger.expected_stock(), 125.0);
        assert!(ledger.relative_error(125.0) < 1e-12);
        assert_eq!(ledger.last_tick(Flux::Herbivory), 10.0);
        ledger.start_tick();
        assert_eq!(ledger.last_tick(Flux::Herbivory), 0.0);
        assert_eq!(ledger.total(Flux::Herbivory), 10.0);
        assert_eq!(ledger.total(Flux::Excretion), 0.0);
    }

    #[test]
    fn nutrient_kinds_differ_from_energy_kinds() {
        assert_eq!(Flux::PrimaryProduction.energy_kind(), FluxKind::Input);
        assert_eq!(Flux::PrimaryProduction.nutrient_kind(), FluxKind::Internal);
        assert_eq!(Flux::Humification.energy_kind(), FluxKind::Output);
        assert_eq!(Flux::Humification.nutrient_kind(), FluxKind::Internal);
        let mut keys: Vec<&str> = Flux::ALL.iter().map(|f| f.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Flux::ALL.len(), "keys are unique");
    }
}
