//! Individual trees: growth stages, height and biomass growth limited by
//! light competition (Beer–Lambert through taller crowns), temperature,
//! water and nutrients; deciduous phenology; seeding with dispersal kernels
//! and establishment; Gompertz ageing; death to standing dead wood that
//! later falls into the dead-wood pool; fire and wind damage.
//!
//! Allometry: wood `B(H) = B_max (H/H_max)^{2.5}`, crown radius `r = c·H`,
//! full leaf mass `LAI·πr²/SLA`. Daily height growth
//! `ΔH = g/days_per_year · (1 − H/H_max) · f_light · f_T · min(f_W, f_N)`
//! with `f_light = min(1, I / (0.1 + 0.6 (1 − shade tolerance)))`.

use organism_engine::{AgentId, AgentStore};
use physics_world::{Season, SurfaceKind, World};
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;
use simulation_core::glam::Vec2;

use crate::fungi::Detritus;
use crate::ledger::{Flux, Ledgers};
use crate::species::SpeciesCatalog;
use crate::species::plants::{PlantLayer, TreeSpecies};
use crate::units::{LIGHT_EXTINCTION, PLANT_KJ_PER_KG, TimeScale, beta_temperature, ramp};
use crate::vegetation::{FLOOD_DEPTH_M, Vegetation};

/// Height of a germinated seedling (m).
pub const SEEDLING_HEIGHT_M: f32 = 0.3;
/// Simulated days a dead tree stands before falling.
pub const SNAG_DAYS: f64 = 60.0;
/// Probability that a seed landing in ideal conditions becomes a seedling.
pub const ESTABLISHMENT: f32 = 0.05;
/// Height below which a tree is a seedling (m).
const SEEDLING_STAGE_M: f32 = 1.3;
/// Largest crown footprint half-width (cells).
const MAX_FOOTPRINT: i64 = 3;

/// Growth stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeStage {
    /// Below 1.3 m.
    Seedling,
    /// Young tree, not yet seeding.
    Sapling,
    /// Seeding adult.
    Mature,
    /// Older than 70 % of the lifespan.
    Old,
    /// Standing dead tree (snag).
    Dead,
}

/// Why a tree died.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeDeath {
    /// Gompertz ageing.
    Age,
    /// Health exhausted by shade, drought or cold.
    Stress,
    /// Killed by fire.
    Fire,
    /// Blown down by wind.
    Wind,
    /// Removed by a command.
    Felled,
}

/// One tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    /// Index into [`SpeciesCatalog::trees`].
    pub species: u16,
    /// Position (cells).
    pub position: Vec2,
    /// Tick of germination (or planting); initial trees created with an age
    /// have a negative birth tick.
    pub birth_tick: i64,
    /// Height (m).
    pub height_m: f32,
    /// Woody biomass (kg).
    pub wood_kg: f32,
    /// Leaf biomass (kg).
    pub leaf_kg: f32,
    /// Health in `[0, 1]`.
    pub health: f32,
    /// Relative light at the crown from the last canopy computation.
    pub light: f32,
    /// Has been through a fire (scarred / charred).
    pub burnt: bool,
    /// Tick of death, if dead.
    pub dead_since: Option<u64>,
    /// Cause of death, if dead.
    pub death: Option<TreeDeath>,
    /// Visual variant (index into the stage's model list).
    pub variant: u8,
}

impl Tree {
    /// Growth stage.
    pub fn stage(&self, species: &TreeSpecies, tick: u64, scale: &TimeScale) -> TreeStage {
        if self.dead_since.is_some() {
            return TreeStage::Dead;
        }
        if self.height_m < SEEDLING_STAGE_M {
            return TreeStage::Seedling;
        }
        let age = self.age_years(tick, scale);
        if age >= 0.7 * species.lifespan_years {
            TreeStage::Old
        } else if age >= species.maturity_years && self.height_m >= 0.3 * species.max_height_m {
            TreeStage::Mature
        } else {
            TreeStage::Sapling
        }
    }

    /// Age in (simulated) years.
    pub fn age_years(&self, tick: u64, scale: &TimeScale) -> f32 {
        let ticks_per_year = f64::from(scale.ticks_per_day) * f64::from(scale.days_per_year);
        (self.age_ticks(tick) as f64 / ticks_per_year.max(1.0)) as f32
    }

    /// Age in ticks.
    pub fn age_ticks(&self, tick: u64) -> u64 {
        (tick as i64).saturating_sub(self.birth_tick).max(0) as u64
    }

    /// Whether the tree is alive.
    pub fn is_alive(&self) -> bool {
        self.dead_since.is_none()
    }

    /// Cell index of the trunk.
    pub fn cell(&self, width: usize, height: usize) -> usize {
        let x = (self.position.x.max(0.0) as usize).min(width.saturating_sub(1));
        let y = (self.position.y.max(0.0) as usize).min(height.saturating_sub(1));
        y * width + x
    }

    /// Energy (kJ) and nutrients (g).
    pub fn stock(&self, species: &TreeSpecies) -> (f64, f64) {
        let kj = f64::from(self.wood_kg + self.leaf_kg) * PLANT_KJ_PER_KG;
        let g =
            f64::from(self.wood_kg * species.wood_nutrient_g_per_kg + self.leaf_kg * species.leaf_nutrient_g_per_kg);
        (kj, g)
    }
}

/// Height (m) up to which browsers reach tree foliage.
pub const BROWSE_HEIGHT_M: f32 = 2.5;

/// Fraction of a tree's foliage within reach of browsers.
pub fn browse_fraction(height_m: f32) -> f32 {
    (BROWSE_HEIGHT_M / height_m.max(1e-3)).min(1.0)
}

/// Leaf factor of the season for deciduous trees (evergreens: 1).
pub fn phenology(deciduous: bool, season: Season, season_fraction: f32) -> f32 {
    if !deciduous {
        return 1.0;
    }
    match season {
        Season::Spring => season_fraction.clamp(0.0, 1.0),
        Season::Summer => 1.0,
        Season::Autumn => (1.0 - season_fraction).clamp(0.0, 1.0),
        Season::Winter => 0.0,
    }
}

/// Per-cell canopy derived from the trees (recomputed when trees change).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Canopy {
    /// Tree leaf area index per cell.
    pub lai: Vec<f32>,
    /// Tree foliage (kg/m²) per cell (mycorrhizal host).
    pub leaf_kg_m2: Vec<f32>,
    /// Flammable foliage (kg/m²) of the trees standing in each cell.
    pub crown_fuel: Vec<f32>,
    /// Foliage within reach of browsers (kg/m²) of the trees standing in each cell.
    pub browse_kg_m2: Vec<f32>,
    /// Living trees whose trunk stands in each cell.
    pub count: Vec<u16>,
    /// Store indices of the trees standing in each cell (CSR layout).
    pub starts: Vec<u32>,
    /// See `starts`.
    pub items: Vec<u32>,
    /// Relative light at each tree's crown (store index order).
    pub tree_light: Vec<f32>,
}

impl Canopy {
    /// Fraction of light intercepted by tree crowns in a cell.
    pub fn cover(&self, cell: usize) -> f32 {
        1.0 - (-LIGHT_EXTINCTION * self.lai[cell]).exp()
    }

    /// Trees standing in a cell (store indices).
    pub fn trees_in(&self, cell: usize) -> &[u32] {
        &self.items[self.starts[cell] as usize..self.starts[cell + 1] as usize]
    }

    /// Computes the canopy of `trees` on a `width × height` map.
    pub fn compute(
        trees: &AgentStore<Tree>,
        catalog: &SpeciesCatalog,
        width: usize,
        height: usize,
        scale: &TimeScale,
    ) -> Self {
        let n = width * height;
        let area = scale.cell_area_m2();
        let mut canopy = Canopy {
            lai: vec![0.0; n],
            leaf_kg_m2: vec![0.0; n],
            crown_fuel: vec![0.0; n],
            browse_kg_m2: vec![0.0; n],
            count: vec![0; n],
            starts: vec![0; n + 1],
            items: Vec::new(),
            tree_light: vec![1.0; trees.len()],
        };
        // (cell, height, store index, leaf area contribution in m²)
        let mut contributions: Vec<(u32, f32, u32, f32)> = Vec::new();
        let mut trunk_cells: Vec<u32> = Vec::with_capacity(trees.len());
        for (index, tree) in trees.as_slice().iter().enumerate() {
            let cell = tree.cell(width, height);
            trunk_cells.push(cell as u32);
            if !tree.is_alive() {
                continue;
            }
            let species = &catalog.trees()[usize::from(tree.species)];
            canopy.count[cell] = canopy.count[cell].saturating_add(1);
            canopy.crown_fuel[cell] += tree.leaf_kg * species.flammability / area;
            canopy.browse_kg_m2[cell] += tree.leaf_kg * browse_fraction(tree.height_m) / area;
            let leaf_area = tree.leaf_kg * species.leaf_area_per_kg;
            let cells = crown_footprint(tree.position, scale.cells(species.crown_radius(tree.height_m)), width, height);
            for (c, share) in cells {
                contributions.push((c as u32, tree.height_m, index as u32, leaf_area * share));
                canopy.lai[c] += leaf_area * share / area;
                canopy.leaf_kg_m2[c] += tree.leaf_kg * share / area;
            }
        }
        // Light at each crown: Beer–Lambert through the strictly taller
        // crowns sharing each footprint cell (ties: lower index first).
        contributions.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)).then(a.2.cmp(&b.2)));
        let mut light_sum = vec![0.0f32; trees.len()];
        let mut light_count = vec![0u32; trees.len()];
        let mut i = 0;
        while i < contributions.len() {
            let cell = contributions[i].0;
            let mut above = 0.0f32;
            while i < contributions.len() && contributions[i].0 == cell {
                let (_, _, index, leaf_area) = contributions[i];
                light_sum[index as usize] += (-LIGHT_EXTINCTION * above / area).exp();
                light_count[index as usize] += 1;
                above += leaf_area;
                i += 1;
            }
        }
        for (index, light) in canopy.tree_light.iter_mut().enumerate() {
            if light_count[index] > 0 {
                *light = light_sum[index] / light_count[index] as f32;
            }
        }
        // CSR list of trunks per cell.
        for &cell in &trunk_cells {
            canopy.starts[cell as usize + 1] += 1;
        }
        for c in 0..n {
            canopy.starts[c + 1] += canopy.starts[c];
        }
        let mut cursor = canopy.starts.clone();
        canopy.items = vec![0; trunk_cells.len()];
        for (index, &cell) in trunk_cells.iter().enumerate() {
            let slot = &mut cursor[cell as usize];
            canopy.items[*slot as usize] = index as u32;
            *slot += 1;
        }
        canopy
    }
}

/// Cells covered by a crown disk of `radius` cells centred on `centre`, with
/// the fraction of the crown area in each (sampled on a 7 × 7 lattice;
/// crowns smaller than a cell fall into the trunk's cell). Ordered by cell.
fn crown_footprint(centre: Vec2, radius: f32, width: usize, height: usize) -> Vec<(usize, f32)> {
    let cell_of = |p: Vec2| -> Option<usize> {
        (p.x >= 0.0 && p.y >= 0.0 && (p.x as usize) < width && (p.y as usize) < height)
            .then(|| p.y as usize * width + p.x as usize)
    };
    let radius = radius.clamp(0.0, MAX_FOOTPRINT as f32);
    let mut cells: Vec<(usize, f32)> = Vec::new();
    if radius > 0.25 {
        const N: i32 = 7;
        let mut total = 0.0;
        for j in 0..N {
            for i in 0..N {
                let offset = Vec2::new(i as f32 + 0.5, j as f32 + 0.5) / N as f32 * 2.0 - Vec2::ONE;
                if offset.length_squared() > 1.0 {
                    continue;
                }
                total += 1.0;
                if let Some(c) = cell_of(centre + offset * radius) {
                    match cells.iter_mut().find(|(k, _)| *k == c) {
                        Some(slot) => slot.1 += 1.0,
                        None => cells.push((c, 1.0)),
                    }
                }
            }
        }
        cells.iter_mut().for_each(|(_, w)| *w /= total);
    }
    if cells.is_empty() {
        let x = (centre.x.max(0.0) as usize).min(width.saturating_sub(1));
        let y = (centre.y.max(0.0) as usize).min(height.saturating_sub(1));
        cells.push((y * width + x, 1.0));
    }
    cells.sort_by_key(|(c, _)| *c);
    cells
}

/// Inputs of the daily tree update.
pub(crate) struct TreeStep<'a> {
    pub catalog: &'a SpeciesCatalog,
    pub scale: TimeScale,
    pub tick: u64,
    pub seed: u64,
    pub growth_multiplier: f32,
    pub season: Season,
    pub season_fraction: f32,
    /// Nutrient-uptake multiplier from mycorrhiza per cell.
    pub myco_boost: &'a [f32],
    /// Daily mean air temperature per cell (°C).
    pub day_temperature: &'a [f32],
    /// Largest number of living trunks per cell.
    pub max_trees_per_cell: u16,
    /// Seeding allowed (false when the tree population cap is reached).
    pub seeding: bool,
}

/// Counts of the daily update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TreeDayReport {
    pub germinated: u32,
    pub died: u32,
    pub fell: u32,
}

/// Random stream of one tree for one tick.
pub(crate) fn tree_rng(seed: u64, tick: u64, id: AgentId) -> SimRng {
    crate::rng::stream(seed, crate::rng::TREES, tick, id.get())
}

/// Daily growth, phenology, mortality, seeding and snag fall.
pub(crate) fn daily_update(
    trees: &mut AgentStore<Tree>,
    canopy: &Canopy,
    step: &TreeStep<'_>,
    world: &mut World,
    vegetation: &Vegetation,
    detritus: &mut Detritus,
    ledgers: &mut Ledgers,
) -> TreeDayReport {
    let (w, h) = (world.width(), world.height());
    let area = f64::from(step.scale.cell_area_m2());
    let days_per_year = step.scale.days_per_year.max(1) as f32;
    let mut report = TreeDayReport::default();
    let mut new_counts = canopy.count.clone();
    let ticks_per_day = f64::from(step.scale.ticks_per_day);
    let (mut agents, pending) = trees.split_mut();
    let ids: Vec<AgentId> = agents.ids().to_vec();
    for (index, id) in ids.into_iter().enumerate() {
        let Some(tree) = agents.get_mut(id) else { continue };
        let species = &step.catalog.trees()[usize::from(tree.species)];
        let cell = tree.cell(w, h);
        let (x, y) = (cell % w, cell / w);
        if let Some(since) = tree.dead_since {
            if (step.tick.saturating_sub(since)) as f64 >= SNAG_DAYS * ticks_per_day {
                topple(tree, species, cell, area, detritus, ledgers);
                pending.despawn(id);
                report.fell += 1;
            }
            continue;
        }
        let mut rng = tree_rng(step.seed, step.tick, id);
        let light = canopy.tree_light.get(index).copied().unwrap_or(1.0);
        tree.light = light;
        let moisture = world.soil_moisture().as_slice()[cell];
        let t = step.day_temperature[cell];
        let f_t = beta_temperature(t, species.temperature_c.0, species.temperature_c.1, species.temperature_c.2);
        let wilt = 0.25 * (1.0 - species.drought_tolerance);
        let f_w = ramp(moisture, wilt, wilt + 0.25);
        let requirement = 0.1 + 0.6 * (1.0 - species.shade_tolerance);
        let f_light = (light / requirement).min(1.0);
        let soil = world.nutrients().as_slice()[cell];
        let f_n = (soil / (2.0 + soil) * step.myco_boost[cell]).min(1.0);
        // Growth of height, wood and leaves.
        let dh = species.height_growth_m_per_year / days_per_year
            * (1.0 - tree.height_m / species.max_height_m).max(0.0)
            * f_light
            * f_t
            * f_w.min(f_n)
            * step.growth_multiplier;
        let new_height = (tree.height_m + dh).min(species.max_height_m);
        let d_wood = (species.wood_at(new_height) - species.wood_at(tree.height_m)).max(0.0);
        let target_leaf =
            species.full_leaf_mass(new_height) * phenology(species.deciduous, step.season, step.season_fraction);
        let d_leaf = (target_leaf - tree.leaf_kg).max(0.0);
        let leaf_drop = (tree.leaf_kg - target_leaf).max(0.0);
        if leaf_drop > 0.0 {
            let g = leaf_drop * species.leaf_nutrient_g_per_kg;
            tree.leaf_kg -= leaf_drop;
            detritus.add_litter(cell, leaf_drop / area as f32, g / area as f32);
            ledgers.book(Flux::Litterfall, f64::from(leaf_drop) * PLANT_KJ_PER_KG, f64::from(g));
        }
        let demand = d_wood * species.wood_nutrient_g_per_kg + d_leaf * species.leaf_nutrient_g_per_kg;
        if demand > 0.0 {
            let taken = world.take_nutrients(x, y, demand / area as f32).unwrap_or(0.0) * area as f32;
            let scale = (taken / demand).min(1.0);
            let (gw, gl) = (d_wood * scale, d_leaf * scale);
            tree.wood_kg += gw;
            tree.leaf_kg += gl;
            tree.height_m = if scale >= 1.0 { new_height } else { tree.height_m + dh * scale };
            ledgers.book(Flux::PrimaryProduction, f64::from(gw + gl) * PLANT_KJ_PER_KG, f64::from(taken));
        }
        // Health: deep shade, drought and frost stress.
        let growing = step.season != Season::Winter || !species.deciduous;
        let mut stress = 0.0;
        if growing && light < 0.3 * requirement {
            stress += 1.0;
        }
        if moisture < wilt {
            stress += 1.0;
        }
        if t < species.temperature_c.0 - 10.0 || t > species.temperature_c.2 {
            stress += 1.0;
        }
        tree.health = if stress > 0.0 { tree.health - 0.02 * stress } else { (tree.health + 0.01).min(1.0) };
        // Mortality: ageing (Gompertz) and exhausted health.
        let age_days = tree.age_ticks(step.tick) as f64 / ticks_per_day;
        let lifespan_days = f64::from(species.lifespan_years) * f64::from(days_per_year);
        let gamma = std::f64::consts::LN_2 / (lifespan_days / 6.0);
        let h0 = gamma * 100f64.ln() / ((gamma * lifespan_days).exp() - 1.0);
        let hazard = 1.0 - (-(h0 * (gamma * age_days).exp())).exp();
        let death = if tree.health <= 0.0 {
            Some(TreeDeath::Stress)
        } else if rng.chance(hazard) {
            Some(TreeDeath::Age)
        } else {
            None
        };
        if let Some(cause) = death {
            kill(tree, species, cause, step.tick, cell, area, detritus, ledgers);
            report.died += 1;
            continue;
        }
        // Seeding.
        let age_years = tree.age_years(step.tick, &step.scale);
        let in_season = species.seeding_seasons.is_empty() || species.seeding_seasons.contains(&step.season);
        if step.seeding && in_season && age_years >= species.maturity_years && tree.health > 0.5 {
            let season_days = if species.seeding_seasons.is_empty() {
                days_per_year
            } else {
                days_per_year / 4.0 * species.seeding_seasons.len() as f32
            };
            let expected = species.seeds_per_year / season_days.max(1.0) * tree.health;
            let seeds = poisson(&mut rng, f64::from(expected)).min(50);
            let origin = tree.position;
            let sp = tree.species;
            for _ in 0..seeds {
                let distance = step.scale.cells(species.dispersal.sample_distance(&mut rng));
                let angle = rng.f32() * std::f32::consts::TAU;
                let target = origin + Vec2::from_angle(angle) * distance;
                if let Some(seedling) =
                    establish(sp, species, target, step, world, vegetation, canopy, &new_counts, &mut rng)
                {
                    let tc = seedling.cell(w, h);
                    let (wood, leaf) = (seedling.wood_kg, seedling.leaf_kg);
                    let need = wood * species.wood_nutrient_g_per_kg + leaf * species.leaf_nutrient_g_per_kg;
                    let (tx, ty) = (tc % w, tc / w);
                    let available = world.nutrients().as_slice()[tc] * area as f32;
                    if available >= need {
                        let taken = world.take_nutrients(tx, ty, need / area as f32).unwrap_or(0.0) * area as f32;
                        ledgers.book(
                            Flux::PrimaryProduction,
                            f64::from(wood + leaf) * PLANT_KJ_PER_KG,
                            f64::from(taken),
                        );
                        new_counts[tc] = new_counts[tc].saturating_add(1);
                        pending.spawn(seedling);
                        report.germinated += 1;
                    }
                }
            }
        }
    }
    trees.commit();
    report
}

/// Tries to establish a seedling at `target`.
#[allow(clippy::too_many_arguments)]
fn establish(
    species_index: u16,
    species: &TreeSpecies,
    target: Vec2,
    step: &TreeStep<'_>,
    world: &World,
    vegetation: &Vegetation,
    canopy: &Canopy,
    counts: &[u16],
    rng: &mut SimRng,
) -> Option<Tree> {
    let cell_xy = world.cell_at(target)?;
    let cell = world.cell_index(cell_xy.0, cell_xy.1).ok()?;
    if world.surface().as_slice()[cell] != SurfaceKind::Land
        || world.water_depth().as_slice()[cell] > FLOOD_DEPTH_M
        || !species.biomes.contains(&world.biome().as_slice()[cell])
        || counts[cell] >= step.max_trees_per_cell
    {
        return None;
    }
    let requirement = 0.1 + 0.6 * (1.0 - species.shade_tolerance);
    let ground_light = 1.0 - canopy.cover(cell);
    let f_light = (ground_light / requirement).min(1.0);
    let wilt = 0.25 * (1.0 - species.drought_tolerance);
    let f_w = ramp(world.soil_moisture().as_slice()[cell], wilt, wilt + 0.25);
    let t = step.day_temperature[cell];
    let f_t = beta_temperature(t, species.temperature_c.0, species.temperature_c.1, species.temperature_c.2);
    let grass = vegetation.biomass(cell, PlantLayer::Grass);
    let competition = 1.0 - 0.5 * (grass / 0.5).min(1.0);
    let p = ESTABLISHMENT * f_light * f_w * competition * f_t.max(0.2);
    if !rng.chance(f64::from(p)) {
        return None;
    }
    let variant = rng.below(256) as u8;
    let phen = phenology(species.deciduous, step.season, step.season_fraction);
    Some(Tree {
        species: species_index,
        position: target,
        birth_tick: step.tick as i64,
        height_m: SEEDLING_HEIGHT_M,
        wood_kg: species.wood_at(SEEDLING_HEIGHT_M),
        leaf_kg: species.full_leaf_mass(SEEDLING_HEIGHT_M) * phen,
        health: 1.0,
        light: ground_light,
        burnt: false,
        dead_since: None,
        death: None,
        variant,
    })
}

fn drop_leaves(
    tree: &mut Tree,
    species: &TreeSpecies,
    cell: usize,
    area: f64,
    detritus: &mut Detritus,
    ledgers: &mut Ledgers,
) {
    if tree.leaf_kg > 0.0 {
        let g = tree.leaf_kg * species.leaf_nutrient_g_per_kg;
        detritus.add_litter(cell, tree.leaf_kg / area as f32, g / area as f32);
        ledgers.book(Flux::Litterfall, f64::from(tree.leaf_kg) * PLANT_KJ_PER_KG, f64::from(g));
        tree.leaf_kg = 0.0;
    }
}

/// Kills a tree: it becomes a standing snag and drops its leaves.
#[allow(clippy::too_many_arguments)]
pub(crate) fn kill(
    tree: &mut Tree,
    species: &TreeSpecies,
    cause: TreeDeath,
    tick: u64,
    cell: usize,
    area: f64,
    detritus: &mut Detritus,
    ledgers: &mut Ledgers,
) {
    tree.dead_since = Some(tick);
    tree.death = Some(cause);
    tree.health = 0.0;
    drop_leaves(tree, species, cell, area, detritus, ledgers);
}

/// Moves a whole tree (wood and leaves) into the dead-wood and litter pools
/// of its cell (windthrow, felling, snag fall).
pub(crate) fn topple(
    tree: &mut Tree,
    species: &TreeSpecies,
    cell: usize,
    area: f64,
    detritus: &mut Detritus,
    ledgers: &mut Ledgers,
) {
    let g = tree.wood_kg * species.wood_nutrient_g_per_kg;
    detritus.add_wood(cell, tree.wood_kg / area as f32, g / area as f32);
    ledgers.book(Flux::TreeMortality, f64::from(tree.wood_kg) * PLANT_KJ_PER_KG, f64::from(g));
    tree.wood_kg = 0.0;
    drop_leaves(tree, species, cell, area, detritus, ledgers);
}

/// Poisson sample (Knuth's method; means above 30 use a rounded normal).
pub(crate) fn poisson(rng: &mut SimRng, mean: f64) -> u32 {
    if mean.is_nan() || mean <= 0.0 {
        return 0;
    }
    if mean > 30.0 {
        return rng.normal_with(mean, mean.sqrt()).round().max(0.0) as u32;
    }
    let limit = (-mean).exp();
    let mut product = rng.f64();
    let mut count = 0;
    while product > limit {
        product *= rng.f64();
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale() -> TimeScale {
        TimeScale { ticks_per_day: 240, days_per_year: 120, cell_size_m: 10.0, motion_seconds: 10.0 }
    }

    fn tree(species: u16, x: f32, y: f32, height: f32, catalog: &SpeciesCatalog) -> Tree {
        let s = &catalog.trees()[usize::from(species)];
        Tree {
            species,
            position: Vec2::new(x, y),
            birth_tick: 0,
            height_m: height,
            wood_kg: s.wood_at(height),
            leaf_kg: s.full_leaf_mass(height),
            health: 1.0,
            light: 1.0,
            burnt: false,
            dead_since: None,
            death: None,
            variant: 0,
        }
    }

    #[test]
    fn taller_crowns_shade_shorter_ones() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let oak = catalog.tree_index("oak").unwrap();
        let mut store: AgentStore<Tree> = AgentStore::new();
        store.queue_spawn(tree(oak, 4.5, 4.5, 25.0, &catalog));
        store.queue_spawn(tree(oak, 4.6, 4.4, 3.0, &catalog));
        store.queue_spawn(tree(oak, 0.5, 0.5, 3.0, &catalog));
        store.commit();
        let canopy = Canopy::compute(&store, &catalog, 10, 10, &scale());
        assert!((canopy.tree_light[0] - 1.0).abs() < 1e-6, "the tallest tree is unshaded");
        assert!(canopy.tree_light[1] < 0.5, "the sapling under it is shaded: {}", canopy.tree_light[1]);
        assert!((canopy.tree_light[2] - 1.0).abs() < 1e-6, "an isolated sapling is unshaded");
        assert_eq!(canopy.trees_in(44), &[0, 1]);
        assert!(canopy.cover(44) > 0.5);
    }

    #[test]
    fn poisson_mean() {
        let mut rng = SimRng::seed_from_u64(9);
        let n = 20_000;
        let mean = (0..n).map(|_| f64::from(poisson(&mut rng, 2.5))).sum::<f64>() / f64::from(n);
        assert!((mean - 2.5).abs() < 0.05, "{mean}");
        assert_eq!(poisson(&mut rng, 0.0), 0);
    }

    #[test]
    fn deciduous_phenology() {
        assert_eq!(phenology(true, Season::Winter, 0.5), 0.0);
        assert_eq!(phenology(true, Season::Summer, 0.5), 1.0);
        assert_eq!(phenology(false, Season::Winter, 0.5), 1.0);
        assert!((phenology(true, Season::Autumn, 0.25) - 0.75).abs() < 1e-6);
    }
}
