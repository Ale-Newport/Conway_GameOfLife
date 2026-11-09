//! Cell addressing and the hooks through which the ecosystem exchanges fuel,
//! canopy and nutrients with the world. Every nutrient transfer is booked in
//! the [`NutrientBudget`](crate::NutrientBudget), so mass is conserved.

use simulation_core::glam::Vec2;

use crate::error::{WorldError, check_range};
use crate::world::World;

/// Largest nutrient or organic-matter amount (g/m²) accepted by one call of
/// [`World::add_nutrients`] or [`World::add_organic_matter`] (1 t/m², far
/// above any biological flux; it keeps the pools finite).
pub const MAX_NUTRIENT_ADDITION_G_M2: f32 = 1.0e6;

impl World {
    /// Linear index of `(x, y)`, or an error outside the world.
    pub fn cell_index(&self, x: usize, y: usize) -> Result<usize, WorldError> {
        if x < self.config.width && y < self.config.height {
            Ok(y * self.config.width + x)
        } else {
            Err(WorldError::OutOfBounds {
                x: x as i64,
                y: y as i64,
                width: self.config.width,
                height: self.config.height,
            })
        }
    }

    /// Cell containing a continuous position (one world unit per cell), if any.
    pub fn cell_at(&self, position: Vec2) -> Option<(usize, usize)> {
        let (x, y) = (position.x.floor(), position.y.floor());
        (x >= 0.0 && y >= 0.0 && (x as usize) < self.config.width && (y as usize) < self.config.height)
            .then_some((x as usize, y as usize))
    }

    /// Sets the fuel load of one cell (kg/m², finite and ≥ 0).
    pub fn set_fuel(&mut self, x: usize, y: usize, kg_m2: f32) -> Result<(), WorldError> {
        let i = self.cell_index(x, y)?;
        check_range("fuel", kg_m2, 0.0, 1.0e4)?;
        self.fuel.as_mut_slice()[i] = kg_m2;
        Ok(())
    }

    /// Sets the canopy cover of one cell (`[0, 1]`).
    pub fn set_canopy(&mut self, x: usize, y: usize, cover: f32) -> Result<(), WorldError> {
        let i = self.cell_index(x, y)?;
        check_range("canopy", cover, 0.0, 1.0)?;
        self.canopy.as_mut_slice()[i] = cover;
        Ok(())
    }

    /// Adds mineral nutrients (g/m², `0..=`[`MAX_NUTRIENT_ADDITION_G_M2`])
    /// to a cell and returns the amount added.
    pub fn add_nutrients(&mut self, x: usize, y: usize, amount_g_m2: f32) -> Result<f32, WorldError> {
        let i = self.cell_index(x, y)?;
        check_range("amount", amount_g_m2, 0.0, MAX_NUTRIENT_ADDITION_G_M2)?;
        self.nutrients.as_mut_slice()[i] += amount_g_m2;
        self.nutrient_budget.added_g += f64::from(amount_g_m2) * self.config.cell_area_m2();
        Ok(amount_g_m2)
    }

    /// Takes up to `amount_g_m2` mineral nutrients from a cell; returns the
    /// amount actually taken (never more than available, so the pool never
    /// becomes negative and mass is conserved).
    pub fn take_nutrients(&mut self, x: usize, y: usize, amount_g_m2: f32) -> Result<f32, WorldError> {
        let i = self.cell_index(x, y)?;
        check_range("amount", amount_g_m2, 0.0, f32::MAX)?;
        let slot = &mut self.nutrients.as_mut_slice()[i];
        let taken = amount_g_m2.min(*slot);
        *slot -= taken;
        self.nutrient_budget.taken_g += f64::from(taken) * self.config.cell_area_m2();
        Ok(taken)
    }

    /// Adds organic matter (litter, carcasses; g/m², `0..=`
    /// [`MAX_NUTRIENT_ADDITION_G_M2`]) to a cell; it is mineralised into
    /// nutrients over time. Returns the amount added.
    pub fn add_organic_matter(&mut self, x: usize, y: usize, amount_g_m2: f32) -> Result<f32, WorldError> {
        let i = self.cell_index(x, y)?;
        check_range("amount", amount_g_m2, 0.0, MAX_NUTRIENT_ADDITION_G_M2)?;
        self.organic.as_mut_slice()[i] += amount_g_m2;
        self.nutrient_budget.added_g += f64::from(amount_g_m2) * self.config.cell_area_m2();
        Ok(amount_g_m2)
    }

    /// Mineral + organic nutrients stored in the soil (g).
    pub fn nutrient_storage_g(&self) -> f64 {
        let area = self.config.cell_area_m2();
        let sum: f64 = self
            .nutrients
            .as_slice()
            .iter()
            .zip(self.organic.as_slice())
            .map(|(n, o)| f64::from(*n) + f64::from(*o))
            .sum();
        sum * area
    }
}
