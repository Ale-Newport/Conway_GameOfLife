//! Perception: what one animal knows about its surroundings this tick.
//!
//! Pure function of read-only state, so it runs in parallel. Other animals
//! are seen within the vision radius and field of view (reduced by the
//! shelter cover of the target's cell, and while foraging by the species'
//! vigilance), heard within the hearing radius when they move, or smelled
//! within the smell radius (chemoreception, moving or not). Burrowed
//! animals are invisible. Fire is perceived from the burning-block cache,
//! water from the nearest-water map (remembered within the home range
//! `AnimalTraits::water_memory_cells`;
//! beyond it thirsty animals search downhill), food by sampling cells
//! around the animal.

use organism_engine::perception::{self, Percept, Query};
use organism_engine::{AgentId, AgentStore, Bounds, GenomeSchema, Kinematics};
use physics_world::World;
use simulation_core::SpatialGrid;
use simulation_core::glam::Vec2;

use super::genetics::Phenotype;
use super::social::PackRank;
use super::{Action, Animal, ReproState, Sex, Target};
use crate::carcass::Remains;
use crate::disease::Epidemiology;
use crate::environment::{EnvCache, passable};
use crate::fields::Fields;
use crate::fungi::{Detritus, Fungi};
use crate::species::{AnimalTraits, Food, Refuge, SpeciesCatalog};
use crate::trees::Canopy;
use crate::units::TimeScale;
use crate::vegetation::Vegetation;

/// Biomass (kg/m²) above the grazing residual at which food is "plentiful"
/// (terrestrial forage, browse, litter); sparse aquatic layers use half of
/// their largest capacity instead (see [`reference_kg_m2`]).
const FOOD_REFERENCE_KG_M2: f32 = 0.15;
/// Biomass (kg/m², wet) of density-field prey (insects, worms, small fish)
/// at which it is "plentiful" for a predator gleaning it (a gram per m²:
/// tens of beetles).
const FIELD_PREY_REFERENCE_KG_M2: f32 = 0.001;
/// Height (m) above which a flier is out of reach of ground predators.
pub const AIRBORNE_M: f32 = 1.0;
/// Largest radius (cells) scanned for food.
const FOOD_SCAN_CELLS: f32 = 6.0;
/// Cover above which a cell counts as dense vegetation.
const DENSE_COVER: f32 = 0.6;
/// Thirst above which an animal that knows no water starts searching for it.
const WATER_SEARCH_THIRST: f32 = 0.3;

/// Read-only world state shared by perception and decisions.
pub(crate) struct Ctx<'a> {
    pub catalog: &'a SpeciesCatalog,
    pub traits: &'a [AnimalTraits],
    pub schema: &'a GenomeSchema,
    pub scale: TimeScale,
    pub world: &'a World,
    pub env: &'a EnvCache,
    pub vegetation: &'a Vegetation,
    pub fungi: &'a Fungi,
    pub detritus: &'a Detritus,
    pub canopy: &'a Canopy,
    pub fields: &'a Fields,
    pub epi: &'a Epidemiology,
    /// Rankings of pack groups, sorted by group id.
    pub leaders: &'a [PackRank],
    pub animals: &'a AgentStore<Animal>,
    pub grid: &'a SpatialGrid,
    pub remains: &'a Remains,
    pub carcass_grid: &'a SpatialGrid,
    /// Clutches by position and mother (slot = index in `remains.clutches`).
    pub clutch_index: &'a crate::carcass::ClutchIndex,
    pub tick: u64,
    pub seed: u64,
    pub hour: f32,
    pub daylight: f32,
    pub bounds: Bounds,
}

impl Ctx<'_> {
    /// Cell index of a position (clamped into the map).
    pub fn cell_of(&self, p: Vec2) -> usize {
        let w = self.env.width;
        let x = (p.x.max(0.0) as usize).min(w - 1);
        let y = (p.y.max(0.0) as usize).min(self.env.height - 1);
        y * w + x
    }

    /// Centre of a cell.
    pub fn centre(&self, cell: usize) -> Vec2 {
        Vec2::new((cell % self.env.width) as f32 + 0.5, (cell / self.env.width) as f32 + 0.5)
    }
}

/// Source of a threat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ThreatSource {
    /// A predator.
    Predator(AgentId),
    /// Fire nearby (or in the own cell).
    Fire,
    /// Water deeper than the animal can wade.
    Water,
    /// An alarm call of a conspecific (position = the caller's threat).
    Alarm,
    /// A territory owner chasing this animal.
    Rival(AgentId),
}

/// The most pressing threat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Threat {
    pub source: ThreatSource,
    /// Point to flee from.
    pub position: Vec2,
    /// Its velocity (predators).
    pub velocity: Vec2,
    pub distance: f32,
    /// Threat level in `[0, 1]`.
    pub level: f32,
}

/// A potential prey.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PreyPercept {
    pub id: AgentId,
    pub position: Vec2,
    pub velocity: Vec2,
    pub distance: f32,
    /// Attractiveness in `[0, 1]`.
    pub value: f32,
}

/// Best food cell.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FoodPercept {
    pub cell: u32,
    pub position: Vec2,
    pub distance: f32,
    /// Availability in `[0, 1]`.
    pub value: f32,
    /// Diet entry of the main food of the cell.
    pub entry: usize,
}

/// Conspecifics nearby.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Herd {
    pub centroid: Vec2,
    pub velocity: Vec2,
    pub count: u32,
    /// Direction in which alarmed (fleeing) conspecifics run.
    pub alarm: Vec2,
}

/// Everything perceived by one animal.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Senses {
    pub threat: Option<Threat>,
    pub prey: Option<PreyPercept>,
    pub carcass: Option<(u64, Vec2, f32)>,
    pub mate: Option<(AgentId, Vec2, f32)>,
    pub herd: Option<Herd>,
    pub food: Option<FoodPercept>,
    pub water: Option<(Vec2, f32)>,
    /// Where a thirsty animal that knows no water searches for it.
    pub water_search: Option<Vec2>,
    pub refuge: Option<(Vec2, f32)>,
    pub mother: Option<(Vec2, f32)>,
    pub clutch: Option<(u64, Vec2, f32)>,
    /// Nearest clutch of another species it can raid (nest predators).
    pub eggs: Option<(u64, Vec2, f32)>,
    /// Neighbours within the separation radius (positions).
    pub crowd: Vec<Vec2>,
    /// Nearest conspecifics (boids neighbours).
    pub flock: Vec<Kinematics>,
    /// Position of the pack leader and its distance (pack members only).
    pub leader: Option<(Vec2, f32)>,
    /// The prey is the pack leader's target.
    pub pack_hunt: bool,
    /// Nearest intruder in the territory.
    pub intruder: Option<(AgentId, Vec2, f32)>,
    /// Migration destination and current discomfort (°C).
    pub migration: Option<(Vec2, f32)>,
    /// Conditions call for (or keep) hibernation.
    pub dormancy: bool,
    /// Nearest cell safe from a nearby fire (burned out or unburnable) and
    /// its distance.
    pub fire_refuge: Option<(Vec2, f32)>,
}

/// Whether a predator with `traits` can reach `prey`: ground hunters cannot
/// catch fliers in the air, and only swimmers reach fish deeper than their
/// wading depth (at least 0.3 m).
pub(crate) fn reachable(traits: &AnimalTraits, prey: &Animal) -> bool {
    let airborne = prey.altitude_m > AIRBORNE_M && !traits.flies;
    let deep = prey.altitude_m < -traits.wade_depth_m.max(0.3) && !traits.swims;
    !airborne && !deep
}

/// Whether `a` is ready to mate.
pub(crate) fn ready_to_mate(id: AgentId, a: &Animal, ctx: &Ctx<'_>) -> bool {
    let species = &ctx.catalog.animals()[usize::from(a.species)];
    let traits = &ctx.traits[usize::from(a.species)];
    let season = ctx.world.time().season;
    matches!(a.repro, ReproState::Ready)
        && (species.reproduction.breeding_seasons.is_empty() || species.reproduction.breeding_seasons.contains(&season))
        && a.energy_kj >= species.reproduction.min_energy_fraction * a.reserve_max_kj(traits)
        && a.health > 0.5
        && super::social::may_breed(ctx, id, a)
}

/// Perceives the surroundings of the animal at store index `index`.
pub(crate) fn perceive(ctx: &Ctx<'_>, index: usize) -> Senses {
    let me = &ctx.animals.as_slice()[index];
    let my_id = ctx.animals.ids()[index];
    let s = usize::from(me.species);
    let species = &ctx.catalog.animals()[s];
    let traits = &ctx.traits[s];
    let pheno = Phenotype::of(&me.genome, ctx.schema);
    let pos = me.kin.position;
    let mut senses = Senses::default();

    let asleep = me.action == Action::Sleep || me.hidden;
    let foraging = matches!(me.action, Action::Graze | Action::Browse | Action::Forage | Action::Feed);
    let vigilance = if foraging { 0.5 + 0.5 * species.behaviour.vigilance } else { 1.0 };
    let vision = if asleep { 0.0 } else { traits.vision_cells * pheno.vision * vigilance };
    let hearing = traits.hearing_cells * if asleep { 0.5 } else { 1.0 };
    let smell = if asleep { 0.0 } else { traits.smell_cells };
    let radius = vision.max(hearing).max(smell).max(1.0);
    let fov_dir = me.kin.direction();
    let in_fov = |offset: Vec2| {
        traits.fov_rad >= std::f32::consts::TAU - 1e-3
            || offset.length_squared() < 1e-9
            || offset.normalize().dot(fov_dir) >= (traits.fov_rad / 2.0).cos()
    };
    let me_ready = ready_to_mate(my_id, me, ctx);
    let mut percepts: Vec<Percept> = Vec::new();
    perception::within(
        ctx.grid,
        &Query::new(pos, radius).in_world(ctx.bounds),
        |slot, _| slot as usize != index,
        &mut percepts,
    );

    let mut herd_sum = Vec2::ZERO;
    let mut herd_vel = Vec2::ZERO;
    let mut herd_n = 0u32;
    let mut alarm = Vec2::ZERO;
    // Personal space: three body lengths.
    let separation = (3.0 * species.body.length_m / ctx.scale.cell_size_m).clamp(0.05, 2.0);
    let my_mass = me.mass_kg;
    let pack_mates = percepts
        .iter()
        .filter(|p| {
            let o = &ctx.animals.as_slice()[p.slot as usize];
            o.species == me.species && o.action == Action::Hunt && p.distance < 4.0
        })
        .count() as f32;
    let mother_id = me.lineage.parents[0];
    let mut flock = Vec::new();
    for p in &percepts {
        let other = &ctx.animals.as_slice()[p.slot as usize];
        let other_id = ctx.animals.ids()[p.slot as usize];
        if other.hidden {
            continue;
        }
        let offset = p.position - pos;
        let cover = ctx.env.cover[ctx.cell_of(p.position)];
        let seen = p.distance <= vision * (1.0 - 0.5 * cover) && in_fov(offset);
        let moving = other.kin.speed() > 0.3 * ctx.traits[usize::from(other.species)].cruise_cells;
        let heard = p.distance <= hearing && moving;
        let smelled = p.distance <= smell;
        if !(seen || heard || smelled) {
            continue;
        }
        if other.species == me.species {
            if p.distance <= separation {
                senses.crowd.push(p.position);
            }
            if p.distance <= 0.5 * radius {
                herd_sum += p.position;
                herd_vel += other.kin.velocity;
                herd_n += 1;
                if other.action == Action::Flee {
                    alarm += other.kin.velocity.normalize_or_zero();
                }
            }
            if Some(other_id) == mother_id && senses.mother.is_none() {
                senses.mother = Some((p.position, p.distance));
            }
            if me_ready && other.sex != me.sex && ready_to_mate(other_id, other, ctx) && senses.mate.is_none() {
                senses.mate = Some((other_id, p.position, p.distance));
            }
            super::social::conspecific(
                ctx,
                me,
                my_id,
                species,
                other,
                other_id,
                p.distance,
                hearing,
                &mut flock,
                &mut senses,
            );
            continue;
        }
        // Threat: an adult (or hunting) animal of a species that eats mine.
        if ctx.catalog.eats(other.species, me.species) > 0.0 && (other.is_adult() || other.action == Action::Hunt) {
            let hunting_me = other.action == Action::Hunt && other.target == Target::Animal(my_id);
            let danger = if hunting_me {
                1.0
            } else if other.action == Action::Hunt {
                0.8
            } else {
                0.5
            };
            let level = ((1.0 - p.distance / radius).max(0.0) * danger).clamp(0.0, 1.0);
            if senses.threat.is_none_or(|t| level > t.level) {
                senses.threat = Some(Threat {
                    source: ThreatSource::Predator(other_id),
                    position: p.position,
                    velocity: other.kin.velocity,
                    distance: p.distance,
                    level,
                });
            }
        }
        // Prey: a species mine eats, within reach (ground hunters cannot
        // catch fliers in the air).
        let preference = ctx.catalog.eats(me.species, other.species);
        let reachable = reachable(traits, other);
        if preference > 0.0
            && reachable
            && species.behaviour.weights.hunt > 0.0
            && passable(ctx.world, traits, p.position)
        {
            let group = 1.0 + 0.5 * pack_mates;
            let size = (2.0 * my_mass * group / other.mass_kg.max(1e-6)).min(1.0);
            let vulnerability = 1.0 - 0.5 * other.health + if other.is_adult() { 0.0 } else { 0.3 };
            let value = (preference * size * vulnerability / (1.0 + p.distance / radius)).clamp(0.0, 1.0);
            if senses.prey.is_none_or(|q| value > q.value) {
                senses.prey = Some(PreyPercept {
                    id: other_id,
                    position: p.position,
                    velocity: other.kin.velocity,
                    distance: p.distance,
                    value,
                });
            }
        }
    }
    if herd_n > 0 {
        senses.herd =
            Some(Herd { centroid: herd_sum / herd_n as f32, velocity: herd_vel / herd_n as f32, count: herd_n, alarm });
    }
    super::social::finish_flock(flock, &mut senses);
    super::social::pack(ctx, me, my_id, species, hearing, &mut senses);
    senses.migration = super::seasonal::migration_target(ctx, me, species, traits, pheno.thermal_shift);

    super::hazards::perceive_hazards(ctx, me, traits, vision.max(2.0), &mut senses);
    perceive_food(ctx, me, traits, species, vision, &mut senses);
    perceive_carcass(ctx, me, traits, my_id, vision.max(hearing).max(smell), &mut senses);
    if species.diet_entry(&Food::Eggs).is_some() {
        // Nearest raidable clutch of another species (ties: laying order);
        // nests in dense cover are harder to find.
        let reach = vision.max(hearing).max(smell);
        let found = perception::nearest(&ctx.clutch_index.grid, &Query::new(pos, reach.max(1.0)), |slot, p| {
            let c = &ctx.remains.clutches[slot as usize];
            c.species != me.species
                && c.eggs > 0
                && p.distance(pos) <= reach * (1.0 - 0.5 * ctx.env.cover[ctx.cell_of(p)])
                && passable(ctx.world, traits, p)
        });
        senses.eggs = found.map(|f| {
            let c = &ctx.remains.clutches[f.slot as usize];
            (c.id, c.position, f.distance)
        });
    }
    if let Some((p, d)) = ctx.env.nearest_water(ctx.cell_of(pos)) {
        let d = f32::from(d);
        if d <= traits.water_memory_cells {
            senses.water = Some((p, d));
        }
    }
    if senses.water.is_none() && me.thirst() > WATER_SEARCH_THIRST {
        senses.water_search = super::hazards::water_search(ctx, me, traits, vision.clamp(3.0, 10.0));
    }
    senses.refuge = find_refuge(ctx, me, traits, species, vision.max(3.0));
    if let Some(slot) = ctx.clutch_index.of_mother(my_id) {
        let clutch = &ctx.remains.clutches[slot as usize];
        senses.clutch = Some((clutch.id, clutch.position, clutch.position.distance(pos)));
    }
    senses.dormancy = super::seasonal::wants_dormancy(ctx, me, species, traits, &senses);
    senses
}

/// Best food cell around the animal.
fn perceive_food(
    ctx: &Ctx<'_>,
    me: &Animal,
    traits: &AnimalTraits,
    species: &crate::species::Species,
    vision: f32,
    senses: &mut Senses,
) {
    let s = usize::from(me.species);
    let forage_diet = species.diet.iter().enumerate().any(|(k, d)| {
        matches!(d.food, Food::Plant(_) | Food::Browse | Food::Nectar | Food::Detritus | Food::Fungi)
            || ctx.catalog.diet_prey(s, k).is_some_and(|p| ctx.fields.slot(p).is_some())
    });
    if !forage_diet || species.behaviour.weights.forage <= 0.0 {
        return;
    }
    let pos = me.kin.position;
    let radius = vision.clamp(1.0, FOOD_SCAN_CELLS);
    let step = (radius / 3.0).max(1.0);
    let n = (radius / step).round() as i32;
    let mut best: Option<FoodPercept> = None;
    for j in -n..=n {
        for i in -n..=n {
            let p = pos + Vec2::new(i as f32, j as f32) * step;
            if ctx.world.cell_at(p).is_none() || !passable(ctx.world, traits, p) {
                continue;
            }
            let cell = ctx.cell_of(p);
            let Some((value, entry)) = food_value(ctx, s, cell) else { continue };
            let distance = p.distance(pos);
            let score = value / (1.0 + 0.3 * distance);
            if best.as_ref().is_none_or(|b| score > b.value / (1.0 + 0.3 * b.distance)) {
                best = Some(FoodPercept { cell: cell as u32, position: ctx.centre(cell), distance, value, entry });
            }
        }
    }
    senses.food = best;
}

/// Availability (in `[0, 1]`) and the diet entry of the main foraged food
/// of a cell (plants, browse, nectar, litter, mushrooms and density-field
/// prey) for species index `s`.
pub(crate) fn food_value(ctx: &Ctx<'_>, s: usize, cell: usize) -> Option<(f32, usize)> {
    let species = &ctx.catalog.animals()[s];
    let mut total = 0.0;
    let mut best: Option<(f32, usize)> = None;
    for (k, entry) in species.diet.iter().enumerate() {
        let available = available_entry(ctx, s, k, cell);
        if available <= 0.0 {
            continue;
        }
        let v = entry.preference * (available / reference_kg_m2(ctx.catalog, &entry.food)).min(1.0);
        total += v;
        if best.is_none_or(|(b, _)| v > b) {
            best = Some((v, k));
        }
    }
    best.map(|(_, k)| (total.min(1.0), k))
}

/// Amount (kg/m²) of a food at which a cell counts as plentiful: 0.15 kg/m²
/// for terrestrial forage, browse, nectar-bearing flowers, litter and
/// mushrooms; aquatic layers (algae, water plants) use `min(0.15, ½ K_max)`
/// so that algae, whose capacity is a few dozen g/m², are not mistaken for
/// scarce food; density-field prey 1 g/m².
pub(crate) fn reference_kg_m2(catalog: &SpeciesCatalog, food: &Food) -> f32 {
    match food {
        Food::Plant(layer) => {
            let params = catalog.layer(*layer);
            if params.aquatic {
                let k_max = params.capacity_kg_m2.iter().fold(0.0f32, |m, (_, k)| m.max(*k));
                FOOD_REFERENCE_KG_M2.min(0.5 * k_max).max(1e-4)
            } else {
                FOOD_REFERENCE_KG_M2
            }
        }
        Food::Prey(_) => FIELD_PREY_REFERENCE_KG_M2,
        Food::Browse | Food::Nectar | Food::Detritus | Food::Fungi | Food::Carrion | Food::Eggs => FOOD_REFERENCE_KG_M2,
    }
}

/// Edible amount (kg/m²) of diet entry `k` of species `s` in a cell; live
/// prey counts only when it is a density field, and an aquatic field (small
/// fish) only where the eater can reach it: swimmers everywhere, others
/// where the water is at most twice their wading depth (at least 0.3 m),
/// the depth at which fish swim within their reach (§5).
pub(crate) fn available_entry(ctx: &Ctx<'_>, s: usize, k: usize, cell: usize) -> f32 {
    match &ctx.catalog.animals()[s].diet[k].food {
        Food::Prey(_) => {
            let Some(prey) = ctx.catalog.diet_prey(s, k) else { return 0.0 };
            let Some(slot) = ctx.fields.slot(prey) else { return 0.0 };
            if ctx.catalog.animals()[usize::from(prey)].habitat.aquatic
                && !aquatic_prey_within_reach(ctx.world, &ctx.traits[s], cell)
            {
                return 0.0;
            }
            ctx.fields.biomass_kg_m2(slot, cell)
        }
        food => available_food(ctx, food, cell),
    }
}

/// Whether an animal with `traits` reaches aquatic organisms of a cell:
/// swimmers and aquatic species always, others (wading birds, bears) where
/// the water is no deeper than twice their wading depth (at least 0.3 m),
/// as fish swim at half the water depth.
pub(crate) fn aquatic_prey_within_reach(world: &World, traits: &AnimalTraits, cell: usize) -> bool {
    traits.swims || traits.aquatic || world.water_depth().as_slice()[cell] <= 2.0 * traits.wade_depth_m.max(0.3)
}

/// Edible amount (kg/m²) of a plant-like food in a cell.
pub(crate) fn available_food(ctx: &Ctx<'_>, food: &Food, cell: usize) -> f32 {
    match food {
        Food::Plant(layer) => {
            (ctx.vegetation.biomass(cell, *layer) - ctx.catalog.layer(*layer).residual_kg_m2).max(0.0)
        }
        // Browse: tree foliage within reach of the trees standing in the cell.
        Food::Browse => ctx.canopy.browse_kg_m2[cell],
        // Nectar: a small fraction of flower biomass is nectar.
        Food::Nectar => {
            crate::food::NECTAR_FRACTION * ctx.vegetation.biomass(cell, crate::species::PlantLayer::Flowers)
        }
        Food::Detritus => ctx.detritus.litter(cell),
        Food::Fungi => ctx.fungi.mushrooms(cell),
        Food::Prey(_) | Food::Carrion | Food::Eggs => 0.0,
    }
}

/// Nearest carcass the animal would eat and can reach: the straight way to
/// it (sampled every half cell) crosses only ground the species can stand
/// on, up to contact distance of the carcass. A carcass behind a cliff or
/// a deep lake is ignored; otherwise a hungry wolf pack fixated on a dead
/// pack mate across a ridge and starved within reach of live prey.
fn perceive_carcass(
    ctx: &Ctx<'_>,
    me: &Animal,
    traits: &AnimalTraits,
    my_id: AgentId,
    radius: f32,
    senses: &mut Senses,
) {
    let species = &ctx.catalog.animals()[usize::from(me.species)];
    let scavenger = species.diet_entry(&Food::Carrion).is_some();
    let hunter = ctx.catalog.hunts_animals(me.species);
    if !scavenger && !hunter {
        return;
    }
    let pos = me.kin.position;
    let found = perception::nearest(ctx.carcass_grid, &Query::new(pos, radius.max(1.0)), |slot, p| {
        let c = &ctx.remains.carcasses[slot as usize];
        let edible =
            c.meat_kg > 0.0 && (scavenger || c.killer == Some(my_id) || ctx.catalog.eats(me.species, c.species) > 0.0);
        let side = p + (pos - p).normalize_or_zero() * (0.9 * super::interact::CONTACT_CELLS);
        edible && perception::line_of_sight(pos, side, 0.5, |q| passable(ctx.world, traits, q))
    });
    if let Some(p) = found {
        let c = &ctx.remains.carcasses[p.slot as usize];
        senses.carcass = Some((c.id, p.position, p.distance));
    }
}

/// Nearest refuge of the species within `radius`.
fn find_refuge(
    ctx: &Ctx<'_>,
    me: &Animal,
    traits: &AnimalTraits,
    species: &crate::species::Species,
    radius: f32,
) -> Option<(Vec2, f32)> {
    let pos = me.kin.position;
    let mut best: Option<(Vec2, f32)> = None;
    let mut consider = |p: Vec2| {
        let d = p.distance(pos);
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((p, d));
        }
    };
    for refuge in &species.habitat.refuges {
        match refuge {
            Refuge::Burrow => {
                if let Some(home) = me.home
                    && home.distance(pos) <= 4.0 * radius
                {
                    consider(home);
                }
            }
            Refuge::Water => {
                if let Some((p, d)) = ctx.env.nearest_water(ctx.cell_of(pos))
                    && f32::from(d) <= radius
                {
                    consider(p);
                }
            }
            Refuge::DenseVegetation | Refuge::Trees => {
                let r = radius.min(8.0);
                let n = r.ceil() as i32;
                for j in -n..=n {
                    for i in -n..=n {
                        let p = pos + Vec2::new(i as f32, j as f32);
                        if ctx.world.cell_at(p).is_none() || !passable(ctx.world, traits, p) {
                            continue;
                        }
                        let cell = ctx.cell_of(p);
                        let ok = match refuge {
                            Refuge::Trees => ctx.canopy.cover(cell) > 0.5,
                            _ => ctx.env.cover[cell] > DENSE_COVER,
                        };
                        if ok {
                            consider(ctx.centre(cell));
                        }
                    }
                }
            }
        }
    }
    best
}

/// Whether `a` is a male ready to mate and `b` a receptive female of the
/// same species (used when resolving mating claims).
pub(crate) fn compatible_pair(a: (AgentId, &Animal), b: (AgentId, &Animal), ctx: &Ctx<'_>) -> bool {
    a.1.species == b.1.species
        && a.1.sex == Sex::Male
        && b.1.sex == Sex::Female
        && ready_to_mate(a.0, a.1, ctx)
        && ready_to_mate(b.0, b.1, ctx)
}

#[cfg(test)]
#[path = "perceive_tests.rs"]
mod tests;
