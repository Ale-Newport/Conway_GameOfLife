//! The complete, serialisable state of an ecosystem run and its
//! initialisation from a configuration.

use std::collections::VecDeque;

use organism_engine::{AgentId, AgentStore, Bounds, Genealogy, GenomeSchema, Kinematics, Lineage};
use physics_world::{Biome, SurfaceKind, World};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;
use simulation_core::{EventLog, SimRng, SpatialGrid};

use crate::animal::genetics::{self, Phenotype};
use crate::animal::{Animal, DeathCause, OrganismRef, ReproState, Sex};
use crate::carcass::Remains;
use crate::colony::Colonies;
use crate::config::{EcosystemConfig, Rules};
use crate::disease::{DiseaseCounters, Epidemiology};
use crate::environment::{EnvCache, passable};
use crate::error::EcosystemError;
use crate::fields::Fields;
use crate::fungi::{Detritus, Fungi};
use crate::ledger::{Ledgers, Stocks};
use crate::netlogo::NetLogoState;
use crate::params::RuntimeParams;
use crate::species::{AnimalTraits, PlantLayer, SocialStructure, SpeciesCatalog};
use crate::trees::{Canopy, Tree, phenology};
use crate::units::TimeScale;
use crate::vegetation::{FLOOD_DEPTH_M, Vegetation};

/// Largest genealogy kept (oldest dead ancestors are truncated beyond it).
pub const GENEALOGY_CAP: usize = 50_000;
/// Capacity of the global event log.
pub const EVENT_LOG_CAPACITY: usize = 20_000;
/// Positions kept in the trail of a followed organism.
pub const TRAIL_LENGTH: usize = 240;
/// Litter nutrient content at initialisation (g/kg).
const INITIAL_LITTER_G_PER_KG: f32 = 15.0;
/// Dead-wood nutrient content at initialisation (g/kg).
const INITIAL_WOOD_G_PER_KG: f32 = 1.5;

/// Per-species and global counters.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Counters {
    /// Births per species during the last tick.
    pub births: Vec<u32>,
    /// Deaths per species during the last tick.
    pub deaths: Vec<u32>,
    /// Births per species since the start.
    pub total_births: Vec<u64>,
    /// Deaths per species since the start.
    pub total_deaths: Vec<u64>,
    /// Deaths by cause since the start (index = [`DeathCause`] order).
    pub deaths_by_cause: [u64; 10],
    /// Attacks during the last tick.
    pub attacks: u32,
    /// Kills during the last tick.
    pub kills: u32,
    /// Territorial fights during the last tick.
    pub fights: u32,
    /// Animals calling an alarm during the last tick.
    pub alarms: u32,
    /// Offspring not born because of a population cap.
    pub births_suppressed: u64,
    /// Trees germinated since the start.
    pub trees_germinated: u64,
    /// Trees that died since the start.
    pub trees_died: u64,
    /// Biomass burned during the last tick (kg).
    pub burned_kg: f64,
    /// Rain during the last tick (mm over land).
    pub rain_mm: f64,
}

impl Counters {
    fn new(species: usize) -> Self {
        Self {
            births: vec![0; species],
            deaths: vec![0; species],
            total_births: vec![0; species],
            total_deaths: vec![0; species],
            ..Self::default()
        }
    }

    /// Clears the per-tick counters.
    pub(crate) fn start_tick(&mut self) {
        self.births.iter_mut().for_each(|v| *v = 0);
        self.deaths.iter_mut().for_each(|v| *v = 0);
        self.attacks = 0;
        self.kills = 0;
        self.fights = 0;
        self.alarms = 0;
        self.burned_kg = 0.0;
        self.rain_mm = 0.0;
    }

    /// Records a death.
    pub(crate) fn death(&mut self, species: u16, cause: DeathCause) {
        let s = usize::from(species);
        if s < self.deaths.len() {
            self.deaths[s] += 1;
            self.total_deaths[s] += 1;
        }
        self.deaths_by_cause[cause as usize] += 1;
    }

    /// Records a birth.
    pub(crate) fn birth(&mut self, species: u16) {
        let s = usize::from(species);
        if s < self.births.len() {
            self.births[s] += 1;
            self.total_births[s] += 1;
        }
    }
}

/// Selection and camera following.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FollowState {
    /// Selected organism (its inspection is included in views).
    pub selected: Option<OrganismRef>,
    /// Followed organism (the camera tracks it; its trail and decisions are recorded).
    pub followed: Option<OrganismRef>,
    /// Recent positions of the followed organism.
    pub trail: VecDeque<Vec2>,
    /// Detailed (trace-level) events of the followed organism.
    pub log: EventLog,
}

/// The serialisable model state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct EcoState {
    pub config: EcosystemConfig,
    pub catalog: SpeciesCatalog,
    pub params: RuntimeParams,
    /// Parameter changes queued for the start of the next tick.
    pub pending: Vec<(String, crate::params::StoredValue)>,
    /// Seed of the running world and of every random stream (a changed
    /// `seed` parameter waits in `config.world.seed` until the next reset).
    pub seed: u64,
    pub tick: u64,
    pub rng: SimRng,
    pub world: World,
    pub animals: AgentStore<Animal>,
    pub trees: AgentStore<Tree>,
    pub remains: Remains,
    pub fields: Fields,
    pub colonies: Colonies,
    pub vegetation: Vegetation,
    pub detritus: Detritus,
    pub fungi: Fungi,
    pub canopy: Canopy,
    pub canopy_dirty: bool,
    pub env: EnvCache,
    pub ledgers: Ledgers,
    pub genealogy: Genealogy,
    pub events: EventLog,
    pub follow: FollowState,
    pub counters: Counters,
    pub netlogo: Option<NetLogoState>,
    /// World nutrient budget already booked: (leached, deposited) grams.
    pub world_budget_seen: (f64, f64),
    /// Last social-group id handed out.
    pub next_group: u64,
    /// Scheduled events already run (they run in order).
    pub schedule_done: usize,
    /// Infection, recovery and death counts per pathogen.
    pub disease: DiseaseCounters,
    /// Configuration and parameters `reset()` rebuilds from: the scenario
    /// plus the user's parameter changes, without the changes made by
    /// scheduled events.
    pub reset_point: Box<(EcosystemConfig, RuntimeParams)>,
}

/// Derived, non-serialised helpers rebuilt from the state.
pub(crate) struct Runtime {
    pub traits: Vec<AnimalTraits>,
    pub schema: GenomeSchema,
    pub grid: SpatialGrid,
    pub carcass_grid: SpatialGrid,
    /// Clutches by position and by mother.
    pub clutch_index: crate::carcass::ClutchIndex,
    pub scale: TimeScale,
    /// Pathogens resolved against the catalogue.
    pub epi: Epidemiology,
    /// Rankings (leader, breeding pair) of pack groups this tick, sorted by group id.
    pub leaders: Vec<crate::animal::social::PackRank>,
}

impl Runtime {
    /// Rebuilds the helpers for a state.
    pub fn new(state: &EcoState) -> Self {
        let scale = state.time_scale();
        let traits = state.catalog.animals().iter().map(|s| AnimalTraits::derive(s, &scale)).collect();
        let (w, h) = (state.world.width() as f32, state.world.height() as f32);
        Self {
            traits,
            schema: genetics::schema(),
            grid: SpatialGrid::new(w, h, 8.0),
            carcass_grid: SpatialGrid::new(w, h, 8.0),
            clutch_index: crate::carcass::ClutchIndex::new(w, h),
            scale,
            epi: Epidemiology::new(&state.config.pathogens, &state.catalog),
            leaders: Vec::new(),
        }
    }
}

impl EcoState {
    /// Time and space scales of the run.
    pub fn time_scale(&self) -> TimeScale {
        TimeScale {
            ticks_per_day: self.world.calendar().ticks_per_day(),
            days_per_year: self.world.calendar().days_per_year(),
            cell_size_m: self.world.config().cell_size_m,
            motion_seconds: self.params.motion_seconds,
        }
    }

    /// Area of one cell (m²).
    pub fn cell_area(&self) -> f64 {
        self.world.config().cell_area_m2()
    }

    /// Movement bounds of animals (the NetLogo variant wraps like a torus).
    pub fn bounds(&self) -> Bounds {
        let max = Vec2::new(self.world.width() as f32, self.world.height() as f32);
        match self.config.rules {
            Rules::NetLogo(_) => Bounds::Wrap { min: Vec2::ZERO, max },
            Rules::Ecological => Bounds::Clamp { min: Vec2::ZERO, max: max - Vec2::splat(1e-3) },
        }
    }

    /// Builds the initial state of a configuration.
    pub fn build(
        config: EcosystemConfig,
        seed: u64,
        catalog: SpeciesCatalog,
        params: RuntimeParams,
    ) -> Result<(Self, Runtime), EcosystemError> {
        config.validate(&catalog)?;
        // The configuration records the seed of the run (reported by
        // `params()` and reused by `reset()`).
        let mut config = config;
        config.world.seed = seed;
        let world_cfg = config.world.clone();
        let effective = world_cfg.effective();
        let world = match config.terrain.heightmap(effective.width, effective.height) {
            None => World::try_generate(&world_cfg)?,
            Some(heightmap) => World::from_heightmap(&world_cfg, heightmap)?,
        };
        let (w, h) = (world.width(), world.height());
        let species = catalog.animals().len();
        let mut state = EcoState {
            canopy: Canopy::default(),
            canopy_dirty: true,
            env: EnvCache::new(&world),
            vegetation: Vegetation::new(w, h),
            detritus: Detritus::new(w, h),
            fungi: Fungi::new(w, h),
            animals: AgentStore::new(),
            trees: AgentStore::new(),
            remains: Remains::default(),
            fields: Fields::new(&catalog, w, h),
            colonies: Colonies::new(w, h),
            ledgers: Ledgers::default(),
            genealogy: Genealogy::with_max_nodes(GENEALOGY_CAP),
            events: EventLog::new(EVENT_LOG_CAPACITY),
            follow: FollowState { log: EventLog::new(2_000), ..FollowState::default() },
            counters: Counters::new(species),
            netlogo: None,
            world_budget_seen: (0.0, 0.0),
            next_group: 0,
            schedule_done: 0,
            disease: DiseaseCounters::new(config.pathogens.len()),
            reset_point: Box::new((config.clone(), params.clone())),
            pending: Vec::new(),
            rng: SimRng::derive(seed, crate::rng::SETUP),
            tick: 0,
            seed,
            params,
            catalog,
            config,
            world,
        };
        let mut rt = Runtime::new(&state);
        match state.config.rules.clone() {
            Rules::Ecological => {
                state.populate_vegetation();
                state.populate_soil()?;
                state.populate_trees();
                state.canopy = Canopy::compute(&state.trees, &state.catalog, w, h, &rt.scale);
                state.canopy_dirty = false;
                state.env.update_cover(&state.vegetation, &state.canopy);
                let setups = state.config.animals.clone();
                for setup in &setups {
                    let index = state.catalog.animal_index(&setup.species)?;
                    state.introduce(&rt, index, setup.count, setup.region, setup.groups)?;
                }
                state.animals.commit();
            }
            Rules::NetLogo(n) => {
                state.netlogo = Some(NetLogoState::init(&mut state, &rt, &n)?);
            }
        }
        let budget = state.world.nutrient_budget();
        state.world_budget_seen = (budget.leached_g, budget.deposited_g);
        let stocks = state.stocks();
        rt.leaders = crate::animal::social::pack_leaders(&state.animals, &state.catalog);
        state.ledgers.energy.initial_stock = stocks.energy();
        state.ledgers.nutrients.initial_stock = stocks.nutrients();
        rt.grid.rebuild(state.animals.as_slice().iter().enumerate().map(|(i, a)| (i as u32, a.kin.position)));
        Ok((state, rt))
    }

    /// Whether `cell` is land not deeply flooded.
    fn dry_land(&self, cell: usize) -> bool {
        self.world.surface().as_slice()[cell] == SurfaceKind::Land
            && self.world.water_depth().as_slice()[cell] <= FLOOD_DEPTH_M
    }

    fn populate_vegetation(&mut self) {
        let setup = self.config.vegetation.clone();
        for cell in 0..self.world.width() * self.world.height() {
            let surface = self.world.surface().as_slice()[cell];
            let biome = self.world.biome().as_slice()[cell];
            let land = surface == SurfaceKind::Land;
            for layer in PlantLayer::ALL {
                let p = self.catalog.layer(layer);
                let habitat = if p.aquatic { !land || biome == Biome::Wetland } else { land };
                let k = if habitat { p.capacity(biome) } else { 0.0 };
                if k <= 0.0 {
                    continue;
                }
                let noise = 0.7 + 0.6 * self.rng.f32();
                self.vegetation.set(cell, layer, k * setup.initial_fraction * noise);
                self.vegetation.set_seed_bank(cell, layer, setup.seed_bank);
            }
        }
    }

    fn populate_soil(&mut self) -> Result<(), EcosystemError> {
        let s = self.config.soil.clone();
        let fungi = self.catalog.fungi().clone();
        let w = self.world.width();
        for cell in 0..w * self.world.height() {
            if self.world.surface().as_slice()[cell] != SurfaceKind::Land {
                if s.water_nutrients_g_m2 > 0.0 {
                    self.world.add_nutrients(cell % w, cell / w, s.water_nutrients_g_m2)?;
                }
                continue;
            }
            self.detritus.add_litter(cell, s.litter_kg_m2, s.litter_kg_m2 * INITIAL_LITTER_G_PER_KG);
            self.detritus.add_wood(cell, s.deadwood_kg_m2, s.deadwood_kg_m2 * INITIAL_WOOD_G_PER_KG);
            let host = self.vegetation.biomass(cell, PlantLayer::Shrubs) > 0.0;
            let myco = if host { s.mycorrhiza_kg_m2.min(fungi.mycorrhiza.capacity_per_host) } else { 0.0 };
            self.fungi.seed(cell, s.mycelium_kg_m2, myco);
        }
        Ok(())
    }

    fn populate_trees(&mut self) {
        let setup = self.config.trees.clone();
        let scale = self.time_scale();
        let area_ha = f64::from(scale.cell_area_m2()) / 10_000.0;
        let max_per_cell = self.params.max_trees_per_cell;
        let season = self.world.time().season;
        let candidates: Vec<u16> = if setup.species.is_empty() {
            (0..self.catalog.trees().len() as u16).collect()
        } else {
            setup.species.iter().filter_map(|id| self.catalog.tree_index(id).ok()).collect()
        };
        let (w, h) = (self.world.width(), self.world.height());
        for cell in 0..w * h {
            if !self.dry_land(cell) {
                continue;
            }
            let biome = self.world.biome().as_slice()[cell];
            let factor = match biome {
                Biome::TemperateForest | Biome::TropicalRainforest => 1.0,
                Biome::Wetland => 0.3,
                Biome::Grassland | Biome::Savanna => 0.2,
                Biome::Tundra | Biome::Mountain => 0.1,
                _ => 0.05,
            };
            let suited: Vec<u16> = candidates
                .iter()
                .copied()
                .filter(|&i| self.catalog.trees()[usize::from(i)].biomes.contains(&biome))
                .collect();
            if suited.is_empty() {
                continue;
            }
            let expected = f64::from(setup.density_per_ha) * area_ha * factor;
            let count = crate::trees::poisson(&mut self.rng, expected).min(u32::from(max_per_cell));
            for _ in 0..count {
                let species_index = suited[self.rng.index(suited.len())];
                let species = &self.catalog.trees()[usize::from(species_index)];
                let age_years = self.rng.f32() * 0.6 * species.lifespan_years;
                let g = species.height_growth_m_per_year;
                let height = species.max_height_m * (1.0 - (-g * age_years / species.max_height_m).exp());
                let height = height.max(crate::trees::SEEDLING_HEIGHT_M);
                let ticks_per_year = f64::from(scale.ticks_per_day) * f64::from(scale.days_per_year);
                let position = Vec2::new((cell % w) as f32 + self.rng.f32(), (cell / w) as f32 + self.rng.f32());
                let phen = phenology(species.deciduous, season, 0.5);
                let tree = Tree {
                    species: species_index,
                    position,
                    birth_tick: -((f64::from(age_years) * ticks_per_year) as i64),
                    height_m: height,
                    wood_kg: species.wood_at(height),
                    leaf_kg: species.full_leaf_mass(height) * phen,
                    health: 1.0,
                    light: 1.0,
                    burnt: false,
                    dead_since: None,
                    death: None,
                    variant: self.rng.below(256) as u8,
                };
                self.trees.queue_spawn(tree);
            }
        }
        self.trees.commit();
    }

    /// A random position inside `region` (or the map) where the species can
    /// stand, preferring its habitat biomes within reach of drinkable water.
    pub(crate) fn place(&mut self, rt: &Runtime, species: u16, region: Option<(f32, f32, f32, f32)>) -> Option<Vec2> {
        let (w, h) = (self.world.width() as f32, self.world.height() as f32);
        let (x0, y0, x1, y1) = region.unwrap_or((0.0, 0.0, w, h));
        let (x0, y0, x1, y1) = (x0.clamp(0.0, w), y0.clamp(0.0, h), x1.clamp(0.0, w), y1.clamp(0.0, h));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let traits = &rt.traits[usize::from(species)];
        let sp = &self.catalog.animals()[usize::from(species)];
        let biomes = &sp.habitat.biomes;
        // Animals that drink start within the range at which they remember
        // water, like real home ranges around water.
        let drinks = !traits.aquatic && sp.metabolism.dehydration_days < 100.0;
        let water_range = traits.water_memory_cells;
        let mut fallback = None;
        for attempt in 0..200 {
            let p = Vec2::new(self.rng.range_f32(x0, x1), self.rng.range_f32(y0, y1)).min(Vec2::new(w, h) - 1e-3);
            if !passable(&self.world, traits, p) {
                continue;
            }
            let Some((cx, cy)) = self.world.cell_at(p) else { continue };
            let cell = cy * self.world.width() + cx;
            let biome = self.world.biome().as_slice()[cell];
            let watered = !drinks || self.env.nearest_water(cell).is_some_and(|(_, d)| f32::from(d) <= water_range);
            if (biomes.contains(&biome) && (watered || attempt >= 100)) || attempt >= 150 {
                return Some(p);
            }
            fallback.get_or_insert(p);
        }
        fallback
    }

    /// Places `count` founders of a species. Social species (any structure
    /// but solitary) start in groups — `groups` groups of near-equal size,
    /// or sizes drawn uniformly from the species' `group_size` range: each
    /// group gets a centre in `region` and its members stand within
    /// `1 + ½√size` cells of it (falling back to the whole region when the
    /// centre's surroundings are impassable).
    pub(crate) fn spawn_founder_groups(
        &mut self,
        rt: &Runtime,
        species: u16,
        count: u32,
        region: Option<(f32, f32, f32, f32)>,
        groups: Option<u32>,
    ) -> Result<Vec<AgentId>, EcosystemError> {
        let social = &self.catalog.animals()[usize::from(species)].social;
        let (lo, hi) = match social.structure {
            SocialStructure::Solitary => (1, 1),
            _ => social.group_size,
        };
        // Packs and pairs are founded by a breeding pair (an adult female
        // and an adult male); the other founders are random.
        let paired = matches!(social.structure, SocialStructure::Pack | SocialStructure::Pair);
        let social = social.structure != SocialStructure::Solitary;
        let mut ids = Vec::with_capacity(count as usize);
        let mut left = count;
        let mut made = 0u32;
        while left > 0 {
            let size = match groups {
                // Near-equal shares of what is left.
                Some(g) => left.div_ceil(g.saturating_sub(made).max(1)),
                None => (self.rng.range_i64(i64::from(lo), i64::from(hi) + 1) as u32).clamp(1, left),
            };
            made += 1;
            left -= size;
            let group = social.then(|| self.new_group());
            let centre = if size > 1 { self.place(rt, species, region) } else { None };
            for member in 0..size {
                let breeder = match member {
                    0 if paired && size > 1 => Some(Sex::Female),
                    1 if paired => Some(Sex::Male),
                    _ => None,
                };
                let near = centre.and_then(|c| {
                    let r = 1.0 + 0.5 * (size as f32).sqrt();
                    let (x0, y0, x1, y1) = region.unwrap_or((0.0, 0.0, f32::INFINITY, f32::INFINITY));
                    let group = (c.x - r).max(x0)..(c.x + r).min(x1);
                    let rows = (c.y - r).max(y0)..(c.y + r).min(y1);
                    self.place(rt, species, Some((group.start, rows.start, group.end, rows.end)))
                });
                let position = match near {
                    Some(p) => p,
                    None => self.position_in(rt, species, region)?,
                };
                ids.push(self.spawn_founder_at(rt, species, position, group, breeder)?);
            }
        }
        Ok(ids)
    }

    /// A fresh social-group id.
    pub(crate) fn new_group(&mut self) -> u64 {
        self.next_group += 1;
        self.next_group
    }

    /// A standing place for `species` in `region`, or a user-facing error.
    fn position_in(
        &mut self,
        rt: &Runtime,
        species: u16,
        region: Option<(f32, f32, f32, f32)>,
    ) -> Result<Vec2, EcosystemError> {
        self.place(rt, species, region).ok_or_else(|| {
            EcosystemError::Command(format!(
                "no place where a {} can stand in the requested area",
                self.catalog.animals()[usize::from(species)].name
            ))
        })
    }

    /// Creates an animal of `species` at `position` with a random age and
    /// condition, in social `group` (if any); a `breeder` is an adult of the
    /// given sex, other founders have a random sex and are adults with
    /// probability 0.8. Burrowers dig their burrow there; territorial adults
    /// make it the centre of their territory. Queued until the next commit.
    fn spawn_founder_at(
        &mut self,
        rt: &Runtime,
        species: u16,
        position: Vec2,
        group: Option<u64>,
        breeder: Option<Sex>,
    ) -> Result<AgentId, EcosystemError> {
        let s = usize::from(species);
        let traits = &rt.traits[s];
        let genome = genetics::founder(&rt.schema, &mut self.rng)?;
        let pheno = Phenotype::of(&genome, &rt.schema);
        let adult = breeder.is_some() || self.rng.chance(0.8);
        let age = if adult {
            self.rng.range_f64(traits.maturity_ticks, traits.maturity_ticks.max(0.4 * traits.lifespan_ticks))
        } else {
            self.rng.range_f64(0.0, traits.maturity_ticks.max(1.0))
        };
        let adult_mass = traits.adult_mass_kg * pheno.size;
        let mass = if adult {
            adult_mass
        } else {
            let f = (age / traits.maturity_ticks.max(1.0)) as f32;
            traits.birth_mass_kg + (adult_mass - traits.birth_mass_kg) * f.clamp(0.0, 1.0)
        };
        let sex = match breeder {
            Some(sex) => sex,
            None if self.rng.chance(0.5) => Sex::Female,
            None => Sex::Male,
        };
        let heading = self.rng.f32() * std::f32::consts::TAU;
        let reserve = traits.reserve_max_kj * (mass / traits.adult_mass_kg).powf(0.75);
        let energy = reserve * self.rng.range_f32(0.6, 0.9);
        let id = self.animals.next_id();
        let lineage = Lineage::founder(id, self.tick);
        let birth = self.tick as i64 - age as i64;
        let mut animal = Animal::new(species, sex, Kinematics::new(position, heading), birth, genome, lineage);
        animal.mass_kg = mass;
        animal.energy_kj = energy;
        animal.hydration = self.rng.range_f32(0.8, 1.0);
        animal.fatigue = self.rng.range_f32(0.0, 0.4);
        animal.repro = if adult { ReproState::Ready } else { ReproState::Juvenile };
        animal.action_since = self.tick;
        animal.home = (traits.burrows || (adult && traits.territory_cells > 0.0)).then_some(position);
        animal.group = group;
        let queued = self.animals.queue_spawn(animal);
        debug_assert_eq!(queued, id);
        self.genealogy.record_birth(lineage)?;
        Ok(queued)
    }

    /// Energy and nutrient stocks of every pool.
    pub fn stocks(&self) -> Stocks {
        let area = self.cell_area();
        let mut trees = (0.0, 0.0);
        for tree in self.trees.as_slice() {
            let (e, n) = tree.stock(&self.catalog.trees()[usize::from(tree.species)]);
            trees.0 += e;
            trees.1 += n;
        }
        let mut animals = (0.0, 0.0);
        for a in self.animals.as_slice() {
            animals.0 += a.energy_stock_kj();
            animals.1 += a.nutrient_stock_g();
        }
        let (mut mineral, mut organic) = (0.0, 0.0);
        for (n, o) in self.world.nutrients().as_slice().iter().zip(self.world.organic_matter().as_slice()) {
            mineral += f64::from(*n);
            organic += f64::from(*o);
        }
        Stocks {
            vegetation: self.vegetation.stock(&self.catalog, area),
            trees,
            animals,
            fields: self.fields.stock(area),
            colonies: self.colonies.stock(&self.catalog),
            clutches: self.remains.clutch_stock(),
            carcasses: self.remains.carcass_stock(),
            litter: self.detritus.litter_stock(area),
            deadwood: self.detritus.wood_stock(area),
            fungi: self.fungi.stock(self.catalog.fungi(), area),
            soil_mineral_g: mineral * area,
            soil_organic_g: organic * area,
        }
    }
}
