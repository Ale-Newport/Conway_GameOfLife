//! Per-cell food shared by density-field organisms and colony foragers:
//! vegetation layers, nectar, litter, mushrooms and the density of other
//! field species. Each kind has an energy density and a nutrient content,
//! and every removal reports the flux to book.

use crate::fields::Fields;
use crate::fungi::{Detritus, Fungi};
use crate::ledger::Flux;
use crate::species::{Food, PlantLayer, SpeciesCatalog};
use crate::units::{ANIMAL_NUTRIENT_G_PER_KG, DETRITUS_KJ_PER_KG, FUNGI_KJ_PER_KG, PLANT_KJ_PER_KG, TISSUE_KJ_PER_KG};
use crate::vegetation::Vegetation;

/// Fraction of flower biomass that is standing nectar and pollen available
/// to nectar feeders (order of magnitude: about a gram of sugar per m² of a
/// flowering meadow).
pub const NECTAR_FRACTION: f32 = 0.01;

/// A food found in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellFood {
    /// Biomass of a vegetation layer above its grazing residual.
    Plant(PlantLayer),
    /// Nectar of the flower layer.
    Nectar,
    /// Litter and dung.
    Detritus,
    /// Mushrooms.
    Fungi,
    /// Organisms of a density-field species (slot in [`Fields`]).
    Field(usize),
}

impl CellFood {
    /// Resolves a diet entry (`None` for foods that are not cell foods:
    /// browse, carrion and individual prey).
    pub fn resolve(food: &Food, catalog: &SpeciesCatalog, fields: &Fields) -> Option<Self> {
        match food {
            Food::Plant(layer) => Some(Self::Plant(*layer)),
            Food::Nectar => Some(Self::Nectar),
            Food::Detritus => Some(Self::Detritus),
            Food::Fungi => Some(Self::Fungi),
            Food::Prey(id) => catalog.animal_index(id).ok().and_then(|s| fields.slot(s)).map(Self::Field),
            Food::Browse | Food::Carrion | Food::Eggs => None,
        }
    }

    /// Gross energy density (kJ per kg of the food as stored).
    pub fn kj_per_kg(self) -> f64 {
        match self {
            Self::Plant(_) | Self::Nectar => PLANT_KJ_PER_KG,
            Self::Detritus => DETRITUS_KJ_PER_KG,
            Self::Fungi => FUNGI_KJ_PER_KG,
            Self::Field(_) => TISSUE_KJ_PER_KG,
        }
    }

    /// Flux booked when the food is eaten.
    pub fn flux(self) -> Flux {
        match self {
            Self::Plant(_) => Flux::Herbivory,
            Self::Nectar => Flux::Nectarivory,
            Self::Detritus => Flux::Detritivory,
            Self::Fungi => Flux::Fungivory,
            Self::Field(_) => Flux::Predation,
        }
    }
}

/// Mutable access to the pools that hold cell food.
pub(crate) struct Pantry<'a> {
    pub catalog: &'a SpeciesCatalog,
    pub vegetation: &'a mut Vegetation,
    pub detritus: &'a mut Detritus,
    pub fungi: &'a mut Fungi,
}

/// What was removed from a cell (per m²).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Taken {
    /// Gross energy (kJ/m²).
    pub kj: f64,
    /// Nutrients (g/m²).
    pub g: f64,
}

impl Pantry<'_> {
    /// Edible amount (kg/m²) of a food in a cell.
    pub fn available(&self, fields: &Fields, food: CellFood, cell: usize) -> f32 {
        available(self.catalog, self.vegetation, self.detritus, self.fungi, fields, food, cell)
    }

    /// Removes up to `kg_m2` of a food from a cell.
    pub fn take(&mut self, fields: &mut Fields, food: CellFood, cell: usize, kg_m2: f32) -> Taken {
        let kg_m2 = kg_m2.max(0.0);
        match food {
            CellFood::Plant(layer) => {
                let p = self.catalog.layer(layer);
                let edible = (self.vegetation.biomass(cell, layer) - p.residual_kg_m2).max(0.0);
                let taken = self.vegetation.remove(cell, layer, kg_m2.min(edible));
                Taken { kj: f64::from(taken) * PLANT_KJ_PER_KG, g: f64::from(taken * p.nutrient_g_per_kg) }
            }
            CellFood::Nectar => {
                let edible = NECTAR_FRACTION * self.vegetation.biomass(cell, PlantLayer::Flowers);
                let taken = self.vegetation.remove(cell, PlantLayer::Flowers, kg_m2.min(edible));
                let n = self.catalog.layer(PlantLayer::Flowers).nutrient_g_per_kg;
                Taken { kj: f64::from(taken) * PLANT_KJ_PER_KG, g: f64::from(taken * n) }
            }
            CellFood::Detritus => {
                let (kg, g) = self.detritus.take_litter(cell, kg_m2);
                Taken { kj: f64::from(kg) * DETRITUS_KJ_PER_KG, g: f64::from(g) }
            }
            CellFood::Fungi => {
                let taken = self.fungi.take_mushrooms(cell, kg_m2);
                let n = self.catalog.fungi().saprotroph.nutrient_g_per_kg;
                Taken { kj: f64::from(taken) * FUNGI_KJ_PER_KG, g: f64::from(taken * n) }
            }
            CellFood::Field(slot) => {
                let kg = fields.remove_biomass(slot, cell, kg_m2);
                Taken { kj: f64::from(kg) * TISSUE_KJ_PER_KG, g: f64::from(kg * ANIMAL_NUTRIENT_G_PER_KG) }
            }
        }
    }
}

/// Edible amount (kg/m²) of a cell food (shared by readers that only hold
/// shared references).
pub(crate) fn available(
    catalog: &SpeciesCatalog,
    vegetation: &Vegetation,
    detritus: &Detritus,
    fungi: &Fungi,
    fields: &Fields,
    food: CellFood,
    cell: usize,
) -> f32 {
    match food {
        CellFood::Plant(layer) => (vegetation.biomass(cell, layer) - catalog.layer(layer).residual_kg_m2).max(0.0),
        CellFood::Nectar => NECTAR_FRACTION * vegetation.biomass(cell, PlantLayer::Flowers),
        CellFood::Detritus => detritus.litter(cell),
        CellFood::Fungi => fungi.mushrooms(cell),
        CellFood::Field(slot) => fields.biomass_kg_m2(slot, cell),
    }
}
