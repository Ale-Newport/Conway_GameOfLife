//! Aggregate representation of tiny organisms (insects, spiders, worms,
//! small fish): one density field per species (individuals per m² of every
//! cell) instead of individual agents, so that billions of organisms cost
//! `O(cells)` per update.
//!
//! Terrestrial fields live on dry land; aquatic fields (`habitat.aquatic`,
//! e.g. sticklebacks) live in water of their kind (fresh or salt) at least
//! their minimum depth, feel the water temperature (the air temperature,
//! at least 4 °C) and do not burn.
//!
//! Hourly, for every cell with organisms, in catalogue order of the field
//! species (`n` individuals per m² of mass `m`, temperature `T`):
//!
//! ```text
//! active     T ≥ T_pref,low − 5 °C and T ≤ T_tol,high (otherwise dormant: no feeding, basal metabolism)
//! B(T)       = 0.78 · 0.16 · m^{3/4} · e^{−E/k (1/T − 1/293 K)}           W per individual (MTE, ectotherm)
//! R          = n · B(T) · (field multiplier if active) · Δt                  respiration
//! F          = Σ_k pref_k F_k                                                food (kg/m², above grazing residuals)
//! I          = B(T) · field multiplier · 24 · intake_per_hour · F/(F + K_h)  gross intake per individual per day
//! eaten_k    = min(n I Δt · pref_k F_k / F / ε_k, ½ F_k)                     (ε_k energy density)
//! A          = Σ a_k eaten_k ε_k                                            assimilated; (1 − a_k) egested to litter
//! A ≥ R      births = min(max_birth · n Δt, (A − R)/(e (1 + overhead)), nutrients / (30 g/kg · m))
//!            the rest of A − R is respired, surplus nutrients excreted to the soil
//! A < R      tissue of (R − A)/e individuals is burned (starvation), its nutrients excreted
//! deaths     n · μ · (1 + n/K) · Δt into the litter (½ μ while dormant)      (K = capacity of the cell)
//! dispersal  a fraction d Δt of every cell moves equally to its 8 neighbours (if habitat), conservative
//! ```
//!
//! `e = m · 7 kJ/g` is the tissue energy of one individual. Every transfer
//! is booked in the ledgers. Pollinators record flower visits. Fields are
//! food for insectivores (individual animals forage them like plants) and
//! for other field species; aquatic fields only where the water is shallow
//! enough for non-swimmers (see `animal::perceive`). The renderer receives
//! a few deterministic *representative markers* per cell; they are not
//! simulated individually.

use physics_world::{Biome, SurfaceKind, World};

use crate::species::WaterKind;
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

use crate::food::{CellFood, Pantry};
use crate::ledger::{Flux, Ledgers};
use crate::species::{FieldParams, Species, SpeciesCatalog};
use crate::units::{ANIMAL_NUTRIENT_G_PER_KG, SYNTHESIS_OVERHEAD, TISSUE_KJ_PER_KG, basal_metabolic_w};
use crate::vegetation::{FLOOD_DEPTH_M, NEIGHBOURS};

/// Density allowed outside the preferred biomes, as a fraction of the maximum.
const OTHER_BIOME_CAPACITY: f32 = 0.1;
/// Largest share of a food item eaten from a cell in one update.
const MAX_SHARE_PER_UPDATE: f32 = 0.5;
/// Most markers drawn per cell and species.
const MAX_MARKERS_PER_CELL: u32 = 3;

/// One density field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Field {
    /// Species index in the catalogue.
    species: u16,
    /// Body mass of one individual (kg), copied from the species.
    mass_kg: f32,
    /// Individuals per m² of every cell.
    density: Vec<f32>,
    /// Individuals born during the last update.
    births: f64,
    /// Individuals that died during the last update.
    deaths: f64,
}

/// The density fields of every field species of the catalogue.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Fields {
    width: usize,
    height: usize,
    fields: Vec<Field>,
}

/// Inputs of one field update.
pub(crate) struct FieldStep<'a> {
    pub catalog: &'a SpeciesCatalog,
    /// Time step (days).
    pub dt_days: f32,
    /// Area of one cell (m²).
    pub cell_area: f64,
}

/// A representative organism for rendering (not simulated individually).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldMarker {
    /// Species index.
    pub species: u16,
    /// Position (cells).
    pub position: Vec2,
    /// Height above the ground (m): fliers hover at their cruising altitude,
    /// aquatic organisms swim below the surface (negative: half the water
    /// depth, at most 20 m down, like individual fish).
    pub altitude_m: f32,
    /// Heading (radians).
    pub heading: f32,
    /// Individuals this marker stands for.
    pub represents: f32,
}

impl Fields {
    /// Empty fields for every field species of `catalog`.
    pub fn new(catalog: &SpeciesCatalog, width: usize, height: usize) -> Self {
        let fields = catalog
            .field_species()
            .into_iter()
            .map(|species| Field {
                species,
                mass_kg: catalog.animals()[usize::from(species)].body.mass_kg,
                density: vec![0.0; width * height],
                births: 0.0,
                deaths: 0.0,
            })
            .collect();
        Self { width, height, fields }
    }

    /// Whether the grids match a map and a catalogue.
    pub(crate) fn is_consistent(&self, catalog: &SpeciesCatalog, width: usize, height: usize) -> bool {
        let n = width * height;
        self.width == width
            && self.height == height
            && self.fields.iter().map(|f| f.species).eq(catalog.field_species())
            && self.fields.iter().all(|f| f.mass_kg == catalog.animals()[usize::from(f.species)].body.mass_kg)
            && self.fields.iter().all(|f| f.density.len() == n && f.density.iter().all(|d| d.is_finite() && *d >= 0.0))
    }

    /// Slot of a species in the fields, if it is a field species.
    pub fn slot(&self, species: u16) -> Option<usize> {
        self.fields.iter().position(|f| f.species == species)
    }

    /// Species index of a slot.
    pub fn species_of(&self, slot: usize) -> u16 {
        self.fields[slot].species
    }

    /// Number of field species.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Whether there are no field species.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Density (individuals per m²) of a slot in a cell.
    pub fn density(&self, slot: usize, cell: usize) -> f32 {
        self.fields[slot].density[cell]
    }

    /// Wet biomass (kg/m²) of a slot in a cell.
    pub(crate) fn biomass_kg_m2(&self, slot: usize, cell: usize) -> f32 {
        self.fields[slot].density[cell] * self.mass_of(slot)
    }

    fn mass_of(&self, slot: usize) -> f32 {
        self.fields[slot].mass_kg
    }

    /// Individuals of a slot over the whole map.
    pub fn population(&self, slot: usize, cell_area: f64) -> f64 {
        self.fields[slot].density.iter().map(|&d| f64::from(d)).sum::<f64>() * cell_area
    }

    /// Individuals born and dead in the last update of a slot.
    pub fn last_births_deaths(&self, slot: usize) -> (f64, f64) {
        (self.fields[slot].births, self.fields[slot].deaths)
    }

    /// Density grid (individuals per m²) of a slot.
    pub fn density_grid(&self, slot: usize) -> simulation_core::Grid2<f32> {
        simulation_core::Grid2::from_vec(self.width, self.height, self.fields[slot].density.clone())
    }

    /// Energy (kJ) and nutrients (g) of all fields.
    pub fn stock(&self, cell_area: f64) -> (f64, f64) {
        let mut kg = 0.0;
        for f in &self.fields {
            kg += f.density.iter().map(|&d| f64::from(d)).sum::<f64>() * f64::from(f.mass_kg) * cell_area;
        }
        (kg * TISSUE_KJ_PER_KG, kg * f64::from(ANIMAL_NUTRIENT_G_PER_KG))
    }

    /// Removes up to `kg_m2` of biomass of a slot from a cell (predation);
    /// returns the biomass removed.
    pub(crate) fn remove_biomass(&mut self, slot: usize, cell: usize, kg_m2: f32) -> f32 {
        let m = self.mass_of(slot).max(1e-12);
        let d = &mut self.fields[slot].density[cell];
        let individuals = (kg_m2.max(0.0) / m).min(*d);
        *d -= individuals;
        individuals * m
    }

    /// Multiplies every density of a slot by `factor` (removal events).
    pub(crate) fn scale(&mut self, slot: usize, factor: f32) {
        for d in &mut self.fields[slot].density {
            *d *= factor.clamp(0.0, 1.0);
        }
    }

    /// Kills a fraction of the organisms of every field in a cell (fire);
    /// returns the energy (kJ/m²) and nutrients (g/m²) of the dead.
    pub(crate) fn kill_in_cell(&mut self, cell: usize, fraction: impl Fn(u16) -> f32) -> (f64, f64) {
        let (mut kj, mut g) = (0.0, 0.0);
        for f in &mut self.fields {
            let dead = f.density[cell] * fraction(f.species).clamp(0.0, 1.0);
            f.density[cell] -= dead;
            let kg = f64::from(dead) * f64::from(f.mass_kg);
            kj += kg * TISSUE_KJ_PER_KG;
            g += kg * f64::from(ANIMAL_NUTRIENT_G_PER_KG);
        }
        (kj, g)
    }

    /// Adds `individuals` of a slot to a cell (setup and spawn commands).
    pub(crate) fn add(&mut self, slot: usize, cell: usize, individuals_m2: f32) {
        self.fields[slot].density[cell] += individuals_m2.max(0.0);
    }

    /// Carrying capacity (individuals per m²) of a cell for a species: dry
    /// land for terrestrial species, water of the right kind and depth for
    /// aquatic ones; the full density in a preferred biome, a tenth elsewhere.
    pub(crate) fn capacity(species: &Species, params: &FieldParams, world: &World, cell: usize) -> f32 {
        if !habitable(species, world, cell) {
            return 0.0;
        }
        let biome: Biome = world.biome().as_slice()[cell];
        let factor = if species.habitat.biomes.contains(&biome) { 1.0 } else { OTHER_BIOME_CAPACITY };
        params.max_density_m2 * factor
    }

    /// One update of every field (see the module documentation).
    pub(crate) fn update(
        &mut self,
        step: &FieldStep<'_>,
        world: &mut World,
        pantry: &mut Pantry<'_>,
        ledgers: &mut Ledgers,
    ) {
        for slot in 0..self.fields.len() {
            self.update_slot(slot, step, world, pantry, ledgers);
        }
    }

    fn update_slot(
        &mut self,
        slot: usize,
        step: &FieldStep<'_>,
        world: &mut World,
        pantry: &mut Pantry<'_>,
        ledgers: &mut Ledgers,
    ) {
        let species = &step.catalog.animals()[usize::from(self.fields[slot].species)];
        let Some(params) = species.representation.field() else { return };
        let area = step.cell_area;
        let dt = f64::from(step.dt_days);
        let m = f64::from(species.body.mass_kg);
        let e_ind = m * TISSUE_KJ_PER_KG;
        let n_ind = m * f64::from(ANIMAL_NUTRIENT_G_PER_KG);
        let fm = f64::from(species.metabolism.field_multiplier);
        let intake_share = f64::from(species.metabolism.intake_per_hour) * 24.0;
        let half = f64::from(params.half_saturation_kg_m2);
        let diet: Vec<(CellFood, f64, f64)> = species
            .diet
            .iter()
            .filter_map(|d| {
                CellFood::resolve(&d.food, step.catalog, self)
                    .filter(|f| *f != CellFood::Field(slot))
                    .map(|f| (f, f64::from(d.preference), f64::from(d.assimilation())))
            })
            .collect();
        let (active_low, active_high) = (species.climate.preferred_c.0 - 5.0, species.climate.tolerance_c.1);
        let w = self.width;
        let mut births_total = 0.0;
        let mut deaths_total = 0.0;
        for cell in 0..w * self.height {
            let n = f64::from(self.fields[slot].density[cell]);
            if n <= 0.0 {
                continue;
            }
            let t = temperature(species, world, cell);
            let active = t >= active_low && t <= active_high;
            let day_kj = basal_metabolic_w(m, false, f64::from(t)) * 86.4;
            let individuals = n * area;
            let respiration = individuals * day_kj * if active { fm } else { 1.0 } * dt;
            let (mut assimilated, mut assimilated_g) = (0.0, 0.0);
            if active && !diet.is_empty() {
                let amounts: Vec<f64> =
                    diet.iter().map(|(f, p, _)| p * f64::from(pantry.available(self, *f, cell))).collect();
                let food: f64 = amounts.iter().sum();
                if food > 0.0 {
                    let gross = individuals * day_kj * fm * intake_share * food / (food + half) * dt;
                    for ((kind, pref, ae), weighted) in diet.iter().zip(&amounts) {
                        if *weighted <= 0.0 {
                            continue;
                        }
                        let available = weighted / pref;
                        let want = (gross * weighted / food / kind.kj_per_kg() / area)
                            .min(available * f64::from(MAX_SHARE_PER_UPDATE));
                        let taken = pantry.take(self, *kind, cell, want as f32);
                        if taken.kj <= 0.0 && taken.g <= 0.0 {
                            continue;
                        }
                        ledgers.book(kind.flux(), taken.kj * area, taken.g * area);
                        assimilated += ae * taken.kj * area;
                        assimilated_g += ae * taken.g * area;
                        let (dung_kj, dung_g) = ((1.0 - ae) * taken.kj, (1.0 - ae) * taken.g);
                        pantry.detritus.add_litter_energy(cell, dung_kj, dung_g as f32);
                        ledgers.book(Flux::Egestion, dung_kj * area, dung_g * area);
                        if *kind == CellFood::Nectar && params.flower_visits_per_hour > 0.0 {
                            let visits = individuals * f64::from(params.flower_visits_per_hour) * dt * 24.0;
                            pantry.vegetation.visit_flowers(cell, visits, area);
                        }
                    }
                }
            }
            let mut count = individuals;
            let net = assimilated - respiration;
            let mut excreted_g = 0.0;
            if net >= 0.0 {
                let births = (f64::from(params.max_birth_per_day) * individuals * dt)
                    .min(net / (e_ind * (1.0 + f64::from(SYNTHESIS_OVERHEAD))))
                    .min(assimilated_g / n_ind)
                    .max(0.0);
                count += births;
                births_total += births;
                // Maintenance, synthesis overhead and surplus are respired.
                ledgers.book(Flux::AnimalRespiration, assimilated - births * e_ind, 0.0);
                excreted_g += assimilated_g - births * n_ind;
            } else {
                // Starvation: tissue pays the deficit.
                let burned = (-net / e_ind).min(count);
                count -= burned;
                ledgers.book(Flux::AnimalRespiration, assimilated + burned * e_ind, 0.0);
                excreted_g += assimilated_g + burned * n_ind;
            }
            let capacity = f64::from(Self::capacity(species, params, world, cell));
            // Dormant stages (eggs, pupae, diapause) die at half the rate.
            let dormancy = if active { 1.0 } else { 0.5 };
            let mortality = if capacity > 0.0 {
                f64::from(params.mortality_per_day) * dormancy * (1.0 + count / area / capacity) * dt
            } else {
                dt
            };
            let deaths = (count * mortality.min(1.0)).min(count);
            count -= deaths;
            deaths_total += deaths;
            if deaths > 0.0 {
                pantry.detritus.add_litter_energy(cell, deaths * e_ind / area, (deaths * n_ind / area) as f32);
                ledgers.book(Flux::AnimalMortality, deaths * e_ind, deaths * n_ind);
            }
            if excreted_g > 0.0 {
                let added = world.add_nutrients(cell % w, cell / w, (excreted_g / area) as f32).unwrap_or(0.0);
                ledgers.book(Flux::Excretion, 0.0, f64::from(added) * area);
                // Grams the soil could not take (never for real amounts) stay
                // in the organisms' cell as litter.
                let refused = excreted_g - f64::from(added) * area;
                if refused > 0.0 {
                    pantry.detritus.add_litter(cell, 0.0, (refused / area) as f32);
                }
            }
            self.fields[slot].density[cell] = (count / area).max(0.0) as f32;
        }
        self.fields[slot].births = births_total;
        self.fields[slot].deaths = deaths_total;
        if params.dispersal_per_day > 0.0 {
            self.disperse(slot, species, params, world, step.dt_days);
        }
    }

    /// Conservative exchange with the 8 neighbours that are habitat.
    fn disperse(&mut self, slot: usize, species: &Species, params: &FieldParams, world: &World, dt_days: f32) {
        let (w, h) = (self.width, self.height);
        let share = (params.dispersal_per_day * dt_days).min(0.5);
        let density = &self.fields[slot].density;
        let habitat: Vec<bool> = (0..w * h).map(|c| Self::capacity(species, params, world, c) > 0.0).collect();
        let mut next = density.clone();
        for cell in 0..w * h {
            let d = density[cell];
            if d <= 0.0 {
                continue;
            }
            let (x, y) = ((cell % w) as i64, (cell / w) as i64);
            let portion = d * share / 8.0;
            for (dx, dy) in NEIGHBOURS {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if habitat[j] {
                    next[cell] -= portion;
                    next[j] += portion;
                }
            }
        }
        for v in &mut next {
            *v = v.max(0.0);
        }
        self.fields[slot].density = next;
    }

    /// Representative markers of the fields inside `region` (cells): up to
    /// three per cell and species, one per `individuals_per_marker`
    /// organisms, at positions derived from the cell, the marker and the tick
    /// (deterministic; they drift slowly so a renderer can animate them).
    pub fn markers(
        &self,
        catalog: &SpeciesCatalog,
        world: &World,
        region: (usize, usize, usize, usize),
        tick: u64,
        limit: usize,
    ) -> Vec<FieldMarker> {
        let cell_area = world.config().cell_area_m2();
        let mut out = Vec::new();
        let (x0, y0, x1, y1) = region;
        for f in &self.fields {
            let species = &catalog.animals()[usize::from(f.species)];
            let Some(params) = species.representation.field() else { continue };
            let flight = if species.locomotion.contains(&crate::species::Locomotion::Fly) {
                species.movement.flight_altitude_m
            } else {
                0.0
            };
            for y in y0..y1.min(self.height) {
                for x in x0..x1.min(self.width) {
                    let cell = y * self.width + x;
                    let altitude = if species.habitat.aquatic {
                        -(0.5 * world.water_depth().as_slice()[cell]).min(20.0)
                    } else {
                        flight
                    };
                    let individuals = f64::from(f.density[cell]) * cell_area;
                    let markers = ((individuals / f64::from(params.individuals_per_marker)).ceil() as u32)
                        .min(MAX_MARKERS_PER_CELL);
                    for k in 0..markers {
                        if out.len() >= limit {
                            return out;
                        }
                        let h = hash(cell as u64, u64::from(f.species), u64::from(k));
                        let phase = (h >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU;
                        let base = Vec2::new((h & 0xFFFF) as f32 / 65536.0, ((h >> 16) & 0xFFFF) as f32 / 65536.0);
                        let angle = phase + tick as f32 * 0.05;
                        let wobble = Vec2::from_angle(angle) * 0.15;
                        let position = Vec2::new(x as f32, y as f32)
                            + (base * 0.7 + Vec2::splat(0.15) + wobble).clamp(Vec2::splat(0.01), Vec2::splat(0.99));
                        out.push(FieldMarker {
                            species: f.species,
                            position,
                            altitude_m: altitude,
                            heading: angle + std::f32::consts::FRAC_PI_2,
                            represents: (individuals / f64::from(markers.max(1))) as f32,
                        });
                    }
                }
            }
        }
        out
    }
}

/// Whether a cell is habitat of a field species: dry land (not flooded) for
/// terrestrial species; for aquatic species water (lake, river, ocean, or
/// land flooded deeper than 0.3 m) of their kind and at least their
/// minimum depth.
fn habitable(species: &Species, world: &World, cell: usize) -> bool {
    let surface = world.surface().as_slice()[cell];
    let depth = world.water_depth().as_slice()[cell];
    if !species.habitat.aquatic {
        return surface == SurfaceKind::Land && depth <= FLOOD_DEPTH_M;
    }
    let water = surface != SurfaceKind::Land || depth > 0.3;
    let kind = match species.habitat.water {
        WaterKind::Any => true,
        WaterKind::Fresh => surface != SurfaceKind::Ocean,
        WaterKind::Salt => surface == SurfaceKind::Ocean,
    };
    water && kind && depth >= species.habitat.min_depth_m
}

/// Temperature (°C) the organisms of a cell experience: the air, and for
/// aquatic species in water at least 4 °C (water under ice stays liquid).
fn temperature(species: &Species, world: &World, cell: usize) -> f32 {
    let air = world.temperature().as_slice()[cell];
    if species.habitat.aquatic && habitable(species, world, cell) { air.max(4.0) } else { air }
}

/// SplitMix-style hash of three integers (display jitter only).
fn hash(a: u64, b: u64, c: u64) -> u64 {
    let mut z = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.rotate_left(21) ^ c.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
