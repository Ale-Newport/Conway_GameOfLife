//! Adding organisms to a run (initial populations, spawn commands and
//! scheduled introductions) for every representation: individual founders
//! (in social groups), density-field individuals spread over the habitat,
//! and colonies founded at suitable nests.

use organism_engine::AgentId;
use physics_world::SurfaceKind;
use simulation_core::glam::Vec2;

use crate::error::EcosystemError;
use crate::fields::Fields;
use crate::species::Representation;
use crate::state::{EcoState, Runtime};
use crate::units::{ANIMAL_NUTRIENT_G_PER_KG, TISSUE_KJ_PER_KG};

/// What an introduction added.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Introduced {
    /// Queued individual animals (committed by the caller).
    pub animals: Vec<AgentId>,
    /// Energy (kJ) of the field individuals and colonies added.
    pub kj: f64,
    /// Nutrients (g) of the field individuals and colonies added.
    pub g: f64,
}

impl EcoState {
    /// Adds `count` organisms of `species` in `region` (cells; `None` = the
    /// map): individual founders in groups, `count` field individuals
    /// spread over the habitat in proportion to its capacity, or `count`
    /// colonies.
    pub(crate) fn introduce(
        &mut self,
        rt: &Runtime,
        species: u16,
        count: u32,
        region: Option<(f32, f32, f32, f32)>,
        groups: Option<u32>,
    ) -> Result<Introduced, EcosystemError> {
        let representation = self.catalog.animals()[usize::from(species)].representation.clone();
        match representation {
            Representation::Individual => {
                let animals = self.spawn_founder_groups(rt, species, count, region, groups)?;
                Ok(Introduced { animals, ..Introduced::default() })
            }
            Representation::Field(params) => {
                let slot = self.fields.slot(species).ok_or_else(|| {
                    EcosystemError::Command(format!("species {species} has no density field in this run"))
                })?;
                let (w, h) = (self.world.width(), self.world.height());
                let (x0, y0, x1, y1) = region.unwrap_or((0.0, 0.0, w as f32, h as f32));
                let cells: Vec<usize> = (0..w * h)
                    .filter(|&c| {
                        let (x, y) = ((c % w) as f32 + 0.5, (c / w) as f32 + 0.5);
                        x >= x0 && x <= x1 && y >= y0 && y <= y1
                    })
                    .collect();
                let sp = &self.catalog.animals()[usize::from(species)];
                let weights: Vec<f32> = cells.iter().map(|&c| Fields::capacity(sp, &params, &self.world, c)).collect();
                let total: f64 = weights.iter().map(|&k| f64::from(k)).sum();
                if total <= 0.0 {
                    return Err(EcosystemError::Command(format!(
                        "no habitat for {} in the requested area",
                        self.catalog.animals()[usize::from(species)].name
                    )));
                }
                let area = self.cell_area();
                let mass = f64::from(sp.body.mass_kg);
                let mut added = 0.0f64;
                for (&c, &k) in cells.iter().zip(&weights) {
                    if k > 0.0 {
                        let density = (f64::from(count) * f64::from(k) / total / area) as f32;
                        self.fields.add(slot, c, density);
                        added += f64::from(density) * area;
                    }
                }
                Ok(Introduced {
                    animals: Vec::new(),
                    kj: added * mass * TISSUE_KJ_PER_KG,
                    g: added * mass * f64::from(ANIMAL_NUTRIENT_G_PER_KG),
                })
            }
            Representation::Colony(_) => {
                let mut introduced = Introduced::default();
                for _ in 0..count {
                    let nest = self.nest_site(rt, species, region)?;
                    let sp = self.catalog.animals()[usize::from(species)].clone();
                    let (_, kj, g) = self.colonies.found(species, &sp, nest, self.tick);
                    introduced.kj += kj;
                    introduced.g += g;
                }
                Ok(introduced)
            }
        }
    }

    /// A nest site on dry land in `region`.
    fn nest_site(
        &mut self,
        rt: &Runtime,
        species: u16,
        region: Option<(f32, f32, f32, f32)>,
    ) -> Result<Vec2, EcosystemError> {
        for _ in 0..50 {
            let Some(p) = self.place(rt, species, region) else { break };
            if let Some((x, y)) = self.world.cell_at(p)
                && self.world.surface().as_slice()[y * self.world.width() + x] == SurfaceKind::Land
            {
                return Ok(p);
            }
        }
        Err(EcosystemError::Command(format!(
            "no dry land for a {} nest in the requested area",
            self.catalog.animals()[usize::from(species)].name
        )))
    }
}
