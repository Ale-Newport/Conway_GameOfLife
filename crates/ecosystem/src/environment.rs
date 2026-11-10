//! Coupling with the abiotic world (`physics-world`).
//!
//! * Before each world step the ecosystem writes **fuel** (flammable
//!   vegetation, litter, fine dead wood and tree foliage, kg/m²) and
//!   **canopy** cover (`1 − e^{−k·LAI}` of the tree crowns) into the world.
//! * After the step it reads back the **fuel consumed** per cell and removes
//!   the same fraction of every fuel component; the burned matter's energy
//!   is booked as combustion, its nutrients split into volatilisation (lost)
//!   and ash (returned to the soil). Trees in burning cells die with a
//!   probability that falls with bark resistance and height.
//! * **Wind damage** (tornadoes) flattens vegetation into litter and throws
//!   trees into the dead-wood pool.
//! * Caches used by the animals: nearest drinkable water (multi-source BFS),
//!   burning-cell blocks (fire perception), shelter cover per cell, daily
//!   mean temperature.

use std::collections::VecDeque;

use organism_engine::AgentStore;
use physics_world::{EnvironmentReport, SurfaceKind, World};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

use crate::fungi::Detritus;
use crate::ledger::{Flux, Ledgers};
use crate::species::{AnimalTraits, PlantLayer, SpeciesCatalog, WaterKind};
use crate::trees::{Canopy, Tree, TreeDeath, kill, topple};
use crate::units::{DETRITUS_KJ_PER_KG, PLANT_KJ_PER_KG};
use crate::vegetation::Vegetation;

/// Fraction of dead wood counted as fine fuel.
pub const FINE_WOOD_FRACTION: f32 = 0.3;
/// Water depth (m) that makes a land cell drinkable (puddles, floods).
pub const DRINKABLE_DEPTH_M: f32 = 0.02;
/// Farthest water remembered by animals (cells, BFS limit).
pub const MAX_WATER_DISTANCE: u16 = 256;
/// Block size (cells) of the fire-perception grid.
pub const FIRE_BLOCK: usize = 4;
/// "No water within reach" marker.
pub const NO_WATER: u32 = u32::MAX;

/// Per-cell caches derived from the world and the vegetation. They are part
/// of the snapshot because they are refreshed on a schedule (hourly water
/// map and cover, running daily temperature), so a restored run must see
/// the same values as the original one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct EnvCache {
    pub width: usize,
    pub height: usize,
    /// Nearest drinkable cell (index) of every cell, or [`NO_WATER`].
    pub water_nearest: Vec<u32>,
    /// Chebyshev distance to it (cells).
    pub water_distance: Vec<u16>,
    /// Burning cells per `FIRE_BLOCK²` block.
    pub fire_blocks: Vec<u16>,
    /// Shelter cover per cell in `[0, 1]` (shrubs + tree canopy).
    pub cover: Vec<f32>,
    /// Running daily mean air temperature (°C).
    pub day_temperature: Vec<f32>,
    /// Fuel written to the world this tick (kg/m²).
    pub fuel_written: Vec<f32>,
}

impl EnvCache {
    /// Empty cache for a map.
    pub fn new(world: &World) -> Self {
        let (w, h) = (world.width(), world.height());
        let n = w * h;
        let mut cache = Self {
            width: w,
            height: h,
            water_nearest: vec![NO_WATER; n],
            water_distance: vec![u16::MAX; n],
            fire_blocks: vec![0; w.div_ceil(FIRE_BLOCK) * h.div_ceil(FIRE_BLOCK)],
            cover: vec![0.0; n],
            day_temperature: world.temperature().as_slice().to_vec(),
            fuel_written: vec![0.0; n],
        };
        cache.update_water(world);
        cache
    }

    /// Multi-source breadth-first search (8-neighbourhood, index order)
    /// from every drinkable cell: fresh-water lakes and rivers, and land
    /// cells with standing water. The ocean is salt and not drinkable.
    pub fn update_water(&mut self, world: &World) {
        let (w, h) = (self.width, self.height);
        self.water_nearest.fill(NO_WATER);
        self.water_distance.fill(u16::MAX);
        let mut queue = VecDeque::new();
        for i in 0..w * h {
            if drinkable(world, i) {
                self.water_nearest[i] = i as u32;
                self.water_distance[i] = 0;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            let d = self.water_distance[i];
            if d >= MAX_WATER_DISTANCE {
                continue;
            }
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            for (dx, dy) in crate::vegetation::NEIGHBOURS {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if self.water_distance[j] == u16::MAX {
                    self.water_distance[j] = d + 1;
                    self.water_nearest[j] = self.water_nearest[i];
                    queue.push_back(j);
                }
            }
        }
    }

    /// Counts burning cells per block.
    pub fn update_fire(&mut self, world: &World) {
        self.fire_blocks.fill(0);
        let bw = self.width.div_ceil(FIRE_BLOCK);
        for &i in world.burning_cells() {
            let i = i as usize;
            let (x, y) = (i % self.width, i / self.width);
            let b = (y / FIRE_BLOCK) * bw + x / FIRE_BLOCK;
            self.fire_blocks[b] = self.fire_blocks[b].saturating_add(1);
        }
    }

    /// Shelter cover: `1 − (1 − shrubs/0.6)(1 − canopy)`, both clamped.
    pub fn update_cover(&mut self, vegetation: &Vegetation, canopy: &Canopy) {
        for (i, c) in self.cover.iter_mut().enumerate() {
            let shrubs = (vegetation.biomass(i, PlantLayer::Shrubs) / 0.6).min(1.0);
            let trees = canopy.cover(i);
            *c = 1.0 - (1.0 - shrubs) * (1.0 - trees);
        }
    }

    /// Hourly update of the running daily mean temperature (24 h EMA).
    pub fn update_day_temperature(&mut self, world: &World, hours: f32) {
        let alpha = (hours / 24.0).clamp(0.0, 1.0);
        for (m, t) in self.day_temperature.iter_mut().zip(world.temperature().as_slice()) {
            *m += (t - *m) * alpha;
        }
    }

    /// Centre of the nearest drinkable cell and its distance (cells).
    pub fn nearest_water(&self, cell: usize) -> Option<(Vec2, u16)> {
        let target = *self.water_nearest.get(cell)?;
        (target != NO_WATER).then(|| {
            let t = target as usize;
            (Vec2::new((t % self.width) as f32 + 0.5, (t / self.width) as f32 + 0.5), self.water_distance[cell])
        })
    }

    /// Burning blocks within `radius` of `p`: returns the vector pointing
    /// away from the fire (weighted by count / distance²) and the distance
    /// to the nearest burning block.
    pub fn fire_near(&self, p: Vec2, radius: f32) -> Option<(Vec2, f32)> {
        let bw = self.width.div_ceil(FIRE_BLOCK);
        let bh = self.height.div_ceil(FIRE_BLOCK);
        let b = FIRE_BLOCK as f32;
        let r_blocks = (radius / b).ceil() as i64 + 1;
        let (bx, by) = ((p.x / b).floor() as i64, (p.y / b).floor() as i64);
        let mut away = Vec2::ZERO;
        let mut nearest = f32::INFINITY;
        for y in (by - r_blocks).max(0)..=(by + r_blocks).min(bh as i64 - 1) {
            for x in (bx - r_blocks).max(0)..=(bx + r_blocks).min(bw as i64 - 1) {
                let count = self.fire_blocks[y as usize * bw + x as usize];
                if count == 0 {
                    continue;
                }
                let centre = Vec2::new((x as f32 + 0.5) * b, (y as f32 + 0.5) * b);
                let d = centre.distance(p);
                if d > radius + b {
                    continue;
                }
                nearest = nearest.min((d - 0.7 * b).max(0.0));
                away += (p - centre).normalize_or_zero() * f32::from(count) / (d * d).max(1.0);
            }
        }
        nearest.is_finite().then_some((away, nearest))
    }
}

/// Whether a cell holds drinkable (fresh) water.
pub fn drinkable(world: &World, cell: usize) -> bool {
    match world.surface().as_slice()[cell] {
        SurfaceKind::Lake | SurfaceKind::River => true,
        SurfaceKind::Ocean => false,
        SurfaceKind::Land => world.water_depth().as_slice()[cell] >= DRINKABLE_DEPTH_M,
    }
}

/// Whether water of `surface` and `depth` suits an aquatic species (kind
/// of water and minimum depth).
fn suits_aquatic(traits: &AnimalTraits, surface: SurfaceKind, depth: f32) -> bool {
    let kind = match traits.water {
        WaterKind::Any => true,
        WaterKind::Fresh => surface != SurfaceKind::Ocean,
        WaterKind::Salt => surface == SurfaceKind::Ocean,
    };
    kind && depth >= traits.min_depth_m
}

/// Whether an animal with `traits` can stand at `p`: fliers anywhere;
/// aquatic species in water of their kind (fresh or salt) at least their
/// minimum depth; swimmers on land and in water; walkers on land with a
/// gentle enough slope and in water no deeper than they can wade.
pub fn passable(world: &World, traits: &AnimalTraits, p: Vec2) -> bool {
    let Some((x, y)) = world.cell_at(p) else { return false };
    if traits.flies {
        return true;
    }
    let i = y * world.width() + x;
    let surface = world.surface().as_slice()[i];
    let depth = world.water_depth().as_slice()[i];
    let water = surface != SurfaceKind::Land || depth > 0.3;
    if traits.aquatic {
        return water && suits_aquatic(traits, surface, depth);
    }
    if surface != SurfaceKind::Land || depth > traits.wade_depth_m {
        return traits.swims;
    }
    world.slope().as_slice()[i] <= traits.max_slope
}

/// Whether a non-flying, non-swimming animal at `p` is in water deeper than
/// it can wade (drowning risk), or an aquatic animal is stranded on land.
pub fn in_hostile_water(world: &World, traits: &AnimalTraits, p: Vec2) -> bool {
    let Some((x, y)) = world.cell_at(p) else { return false };
    if traits.flies {
        return false;
    }
    let i = y * world.width() + x;
    let surface = world.surface().as_slice()[i];
    let depth = world.water_depth().as_slice()[i];
    if traits.aquatic {
        // Stranded on land, or in water of the wrong kind (a freshwater fish at sea).
        let wrong_kind = match traits.water {
            WaterKind::Any => false,
            WaterKind::Fresh => surface == SurfaceKind::Ocean,
            WaterKind::Salt => surface != SurfaceKind::Ocean,
        };
        return (surface == SurfaceKind::Land && depth < 0.05) || wrong_kind;
    }
    !traits.swims && (surface != SurfaceKind::Land || depth > traits.wade_depth_m)
}

/// Writes fuel and canopy for the next world step and remembers the fuel.
pub(crate) fn write_fuel_and_canopy(
    world: &mut World,
    catalog: &SpeciesCatalog,
    vegetation: &Vegetation,
    detritus: &Detritus,
    canopy: &Canopy,
    cache: &mut EnvCache,
) {
    let n = cache.width * cache.height;
    for i in 0..n {
        let mut fuel = detritus.litter(i) + FINE_WOOD_FRACTION * detritus.wood(i) + canopy.crown_fuel[i];
        for layer in PlantLayer::ALL {
            fuel += catalog.layer(layer).flammability * vegetation.biomass(i, layer);
        }
        cache.fuel_written[i] = fuel;
    }
    world.fuel_mut().as_mut_slice().copy_from_slice(&cache.fuel_written);
    for (slot, i) in world.canopy_mut().as_mut_slice().iter_mut().zip(0..n) {
        *slot = canopy.cover(i);
    }
}

/// Fire and wind effects of one world step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DisturbanceReport {
    /// Biomass burned (kg).
    pub burned_kg: f64,
    /// Trees killed by fire.
    pub trees_burned: u32,
    /// Trees thrown by wind.
    pub trees_thrown: u32,
}

/// Inputs of [`apply_disturbances`].
pub(crate) struct Disturbance<'a> {
    pub catalog: &'a SpeciesCatalog,
    pub tick: u64,
    pub seed: u64,
    pub cell_area: f64,
    /// Fraction of burned nutrients lost to the atmosphere.
    pub volatilisation: f32,
    pub canopy: &'a Canopy,
}

/// Applies fuel consumption and wind damage reported by the world.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_disturbances(
    report: &EnvironmentReport,
    d: &Disturbance<'_>,
    world: &mut World,
    cache: &EnvCache,
    vegetation: &mut Vegetation,
    detritus: &mut Detritus,
    trees: &mut AgentStore<Tree>,
    ledgers: &mut Ledgers,
) -> DisturbanceReport {
    let mut out = DisturbanceReport::default();
    let w = cache.width;
    let area = d.cell_area;
    for burn in &report.fuel_consumed {
        let cell = burn.y as usize * w + burn.x as usize;
        let written = cache.fuel_written.get(cell).copied().unwrap_or(0.0);
        if written <= 0.0 || burn.amount <= 0.0 {
            continue;
        }
        let fraction = (burn.amount / written).clamp(0.0, 1.0);
        // Vegetation layers burn in proportion to their flammable share.
        let (veg_kg, veg_g) =
            vegetation.remove_fraction(d.catalog, cell, fraction, |layer| d.catalog.layer(layer).flammability);
        let ((litter_kg, litter_g), (wood_kg, wood_g)) = detritus.burn(cell, fraction, fraction * FINE_WOOD_FRACTION);
        let mut energy = f64::from(veg_kg) * PLANT_KJ_PER_KG + f64::from(litter_kg + wood_kg) * DETRITUS_KJ_PER_KG;
        let mut nutrients = f64::from(veg_g + litter_g + wood_g);
        energy *= area;
        nutrients *= area;
        // Tree foliage and fire mortality.
        let mut rng = crate::rng::stream(d.seed, crate::rng::DISTURBANCE, d.tick, cell as u64);
        for &index in d.canopy.trees_in(cell) {
            let Some(tree) = trees.as_mut_slice().get_mut(index as usize) else { continue };
            if !tree.is_alive() {
                continue;
            }
            let species = &d.catalog.trees()[usize::from(tree.species)];
            let leaves = tree.leaf_kg * (fraction * species.flammability).clamp(0.0, 1.0);
            tree.leaf_kg -= leaves;
            energy += f64::from(leaves) * PLANT_KJ_PER_KG;
            nutrients += f64::from(leaves * species.leaf_nutrient_g_per_kg);
            tree.burnt = true;
            let intensity = burn.amount;
            let p_kill =
                1.0 - (-2.0 * intensity * (1.0 - species.bark_resistance) / (1.0 + tree.height_m / 10.0)).exp();
            if rng.chance(f64::from(p_kill)) {
                out.trees_burned += 1;
                if species.resprouts {
                    // Top-kill: the stem falls into the dead-wood pool and
                    // the tree resprouts from the root collar.
                    let stem = tree.wood_kg - species.wood_at(0.5);
                    if stem > 0.0 {
                        let g = stem * species.wood_nutrient_g_per_kg;
                        detritus.add_wood(cell, stem / area as f32, g / area as f32);
                        ledgers.book(Flux::TreeMortality, f64::from(stem) * PLANT_KJ_PER_KG, f64::from(g));
                        tree.wood_kg -= stem;
                    }
                    tree.height_m = tree.height_m.min(0.5);
                    tree.health = 0.5;
                } else {
                    kill(tree, species, TreeDeath::Fire, d.tick, cell, area, detritus, ledgers);
                }
            } else {
                tree.health = (tree.health - 0.3 * intensity).max(0.05);
            }
        }
        out.burned_kg += energy / PLANT_KJ_PER_KG;
        let lost = nutrients * f64::from(d.volatilisation.clamp(0.0, 1.0));
        let ash = nutrients - lost;
        ledgers.book(Flux::Combustion, energy, 0.0);
        ledgers.book(Flux::Volatilisation, 0.0, lost);
        if ash > 0.0 {
            let (x, y) = (cell % w, cell / w);
            let added = world.add_nutrients(x, y, (ash / area) as f32).unwrap_or(0.0);
            // Whatever the world could not accept (never for real amounts)
            // is booked as volatilised so the books stay closed.
            let returned = f64::from(added) * area;
            ledgers.book(Flux::Ash, 0.0, returned);
            ledgers.book(Flux::Volatilisation, 0.0, (ash - returned).max(0.0));
        }
    }
    for damage in &report.wind_damage {
        let cell = damage.y as usize * w + damage.x as usize;
        let f = damage.damage.clamp(0.0, 1.0);
        let (kg, g) = vegetation.remove_fraction(d.catalog, cell, f, |layer| d.catalog.layer(layer).height_m.min(1.0));
        detritus.add_litter(cell, kg, g);
        ledgers.book(Flux::Litterfall, f64::from(kg) * area * PLANT_KJ_PER_KG, f64::from(g) * area);
        let mut rng = crate::rng::stream(d.seed, crate::rng::DISTURBANCE, d.tick ^ 0x5749_4E44, cell as u64);
        for &index in d.canopy.trees_in(cell) {
            let Some(tree) = trees.as_mut_slice().get_mut(index as usize) else { continue };
            let species = &d.catalog.trees()[usize::from(tree.species)];
            let p = f * (0.3 + 0.7 * tree.height_m / species.max_height_m);
            if tree.wood_kg > 0.0 && rng.chance(f64::from(p)) {
                if tree.is_alive() {
                    kill(tree, species, TreeDeath::Wind, d.tick, cell, area, detritus, ledgers);
                }
                topple(tree, species, cell, area, detritus, ledgers);
                out.trees_thrown += 1;
            }
        }
    }
    out
}

/// Temperature (°C) an animal feels at a cell: the air temperature, and at
/// least 4 °C (deep water does not freeze) for aquatic animals and for
/// swimming ectotherms (frogs, turtles, snakes) in water, whose bodies take
/// the water temperature.
pub fn felt_temperature(world: &World, traits: &AnimalTraits, cell: usize) -> f32 {
    let air = world.temperature().as_slice()[cell];
    let in_water = world.surface().as_slice()[cell] != SurfaceKind::Land || world.water_depth().as_slice()[cell] > 0.3;
    if in_water && (traits.aquatic || (traits.swims && !traits.endotherm)) { air.max(4.0) } else { air }
}

/// Temperature (°C) an animal experiences for its metabolism and thermal
/// stress: in shelter — hidden in its burrow or den, dormant, or torpid in
/// the litter — soil and cover damp the daily cycle and keep frost out, so
/// it feels the running daily mean, at least 0 °C; otherwise
/// [`felt_temperature`].
pub(crate) fn experienced_temperature(
    world: &World,
    cache: &EnvCache,
    traits: &AnimalTraits,
    sheltered: bool,
    cell: usize,
) -> f32 {
    if sheltered { cache.day_temperature[cell].max(0.0) } else { felt_temperature(world, traits, cell) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use physics_world::WorldConfig;
    use simulation_core::Grid2;

    #[test]
    fn water_search_finds_nearest_drinkable_cell() {
        let cfg = WorldConfig { width: 16, height: 16, river_count: 0, lake_tendency: 0.0, ..WorldConfig::default() };
        let mut world = World::from_heightmap(&cfg, Grid2::new(16, 16, 40.0)).unwrap();
        world.flood(3, 3, 1.0, 0.5).unwrap();
        let cache = EnvCache::new(&world);
        let (p, d) = cache.nearest_water(15 * 16 + 15).expect("water within reach");
        assert!(p.distance(Vec2::new(3.5, 3.5)) < 2.0, "{p}");
        assert!(d >= 10, "{d}");
        assert_eq!(cache.nearest_water(3 * 16 + 3).unwrap().1, 0);
    }
}
