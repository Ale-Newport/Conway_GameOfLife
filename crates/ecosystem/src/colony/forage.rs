//! Forager squads: departure, search, harvest, return and unloading, and
//! the ant trail pheromone they lay (see the parent module for the rules).

use physics_world::World;
use simulation_core::glam::Vec2;

use super::{Colonies, Colony, ColonyStep, Forager, ForagerState, cell_of, passable, squad_speed};
use crate::carcass::Remains;
use crate::fields::Fields;
use crate::food::{CellFood, Pantry};
use crate::ledger::{Flux, Ledgers};
use crate::species::{ColonyParams, Food, Locomotion, Species};
use crate::units::TimeScale;

/// Pheromone deposited per tick by a loaded ant squad (units per cell).
const PHEROMONE_DEPOSIT: f32 = 1.0;
/// Pheromone below which a cell is treated as unmarked.
const PHEROMONE_THRESHOLD: f32 = 0.01;
/// Fraction of a full load below which a squad keeps searching.
const MIN_LOAD_FRACTION: f32 = 0.25;
/// Probability that a new squad of a recruiting species flies to the reported patch.
const RECRUITMENT_PROBABILITY: f64 = 0.7;
/// Net displacement of a searching squad as a fraction of its travel.
const SEARCH_DISPLACEMENT: f32 = 0.25;

impl Colonies {
    /// Moves every squad one tick: home, to a recruited patch, or searching.
    pub(super) fn move_squads(
        &mut self,
        step: &ColonyStep<'_>,
        pantry: &mut Pantry<'_>,
        fields: &mut Fields,
        remains: &mut Remains,
        ledgers: &mut Ledgers,
        area: f64,
    ) {
        let colonies = &mut self.colonies;
        let pheromone = &mut self.pheromone;
        let (w, h) = (self.width, self.height);
        for f in &mut self.foragers {
            let Some(c) = colonies.iter_mut().find(|c| c.id == f.colony) else { continue };
            let species = &step.catalog.animals()[usize::from(c.species)];
            let Some(p) = species.representation.colony() else { continue };
            let speed = squad_speed(species, p, &step.scale);
            let range = step.scale.cells(p.foraging_range_m).max(1.0);
            let mut rng = crate::rng::stream(step.seed, crate::rng::COLONY, step.tick, f.id);
            match f.state {
                ForagerState::Returning => {
                    let to_nest = c.nest - f.position;
                    let d = to_nest.length();
                    if p.pheromone && f.load_kj > 0.0 {
                        let end = if d <= speed.max(0.5) { c.nest } else { f.position + to_nest / d * speed };
                        mark_segment(pheromone, w, h, f.position, end);
                    }
                    if d <= speed.max(0.5) {
                        f.position = c.nest;
                        unload(f, c, pantry, step.world, ledgers, area);
                        f.state = departure(species, p, c, range, step.world, &mut rng);
                    } else {
                        f.heading = to_nest.y.atan2(to_nest.x);
                        f.position += to_nest / d * speed;
                    }
                }
                ForagerState::Recruited(patch) => {
                    let to_patch = patch - f.position;
                    let d = to_patch.length();
                    if d <= speed.max(0.5) {
                        f.position = patch;
                        f.state = ForagerState::Searching;
                    } else {
                        f.heading = to_patch.y.atan2(to_patch.x);
                        f.position += to_patch / d * speed;
                    }
                }
                ForagerState::Searching => {
                    let cell = cell_of(step.world, f.position);
                    if harvest(f, species, p, cell, step, pantry, fields, remains, ledgers, area) {
                        f.state = ForagerState::Returning;
                        continue;
                    }
                    let away = f.position - c.nest;
                    let sense = sense_cells(species, &step.scale);
                    let towards = |v: Vec2| v.y.atan2(v.x);
                    let trail = p.pheromone.then(|| trail_direction(pheromone, w, h, f.position, c.nest)).flatten();
                    let (desired, pace) = if away.length() > range {
                        // Too far: turn home and search on the way.
                        (towards(-away), SEARCH_DISPLACEMENT)
                    } else if let Some(carcass) = carrion_nearby(species, remains, f.position, sense) {
                        (towards(carcass - f.position), SEARCH_DISPLACEMENT)
                    } else if let Some(dir) = trail {
                        // Following a trail is direct.
                        (towards(dir), 1.0)
                    } else if let Some(target) = best_food_nearby(species, step, pantry, fields, f.position) {
                        (towards(target - f.position), SEARCH_DISPLACEMENT)
                    } else {
                        (f.heading, SEARCH_DISPLACEMENT)
                    };
                    // Correlated random walk around the desired heading; a
                    // searching squad meanders, so its net displacement is a
                    // quarter of its travel.
                    let turn = (rng.f32() - 0.5) * if pace < 1.0 { 1.2 } else { 0.3 };
                    f.heading = desired + turn;
                    let next = f.position + Vec2::from_angle(f.heading) * pace * speed;
                    if passable(species, step.world, next) {
                        f.position = next;
                    } else {
                        f.heading += std::f32::consts::PI * (0.5 + rng.f32());
                    }
                }
            }
        }
    }
}

/// Harvests food in the squad's cell (or a carcass within one cell);
/// returns whether the squad is loaded.
#[allow(clippy::too_many_arguments)]
fn harvest(
    f: &mut Forager,
    species: &Species,
    p: &ColonyParams,
    cell: usize,
    step: &ColonyStep<'_>,
    pantry: &mut Pantry<'_>,
    fields: &mut Fields,
    remains: &mut Remains,
    ledgers: &mut Ledgers,
    area: f64,
) -> bool {
    let capacity_kg = f64::from(p.workers_per_forager * p.load_kg);
    for entry in &species.diet {
        let ae = entry.assimilation();
        if entry.food == Food::Carrion {
            let near = remains
                .carcasses
                .iter_mut()
                .filter(|c| c.meat_kg > 0.0 && c.position.distance(f.position) <= 1.0)
                .min_by(|a, b| a.position.distance(f.position).total_cmp(&b.position.distance(f.position)));
            if let Some(carcass) = near {
                let (_, kj, g) = carcass.take(capacity_kg as f32);
                if kj > 0.0 {
                    ledgers.book(Flux::Scavenging, kj, g);
                    load(f, kj, g, ae, carcass.position, 1.0);
                    return true;
                }
            }
            continue;
        }
        let Some(food) = CellFood::resolve(&entry.food, step.catalog, fields) else { continue };
        let available = f64::from(pantry.available(fields, food, cell)) * area;
        if available < MIN_LOAD_FRACTION as f64 * capacity_kg * 2.0 {
            continue;
        }
        let want = capacity_kg.min(0.5 * available);
        let taken = pantry.take(fields, food, cell, (want / area) as f32);
        if taken.kj <= 0.0 {
            continue;
        }
        ledgers.book(food.flux(), taken.kj * area, taken.g * area);
        if food == CellFood::Nectar && p.flower_visits_per_trip > 0.0 {
            let visits = f64::from(p.workers_per_forager * p.flower_visits_per_trip);
            pantry.vegetation.visit_flowers(cell, visits, area);
        }
        let quality = (available / (capacity_kg * 20.0)).min(1.0) as f32 * entry.preference;
        let centre = Vec2::new((cell % step.world.width()) as f32 + 0.5, (cell / step.world.width()) as f32 + 0.5);
        load(f, taken.kj * area, taken.g * area, ae, centre, quality);
        return true;
    }
    false
}

/// Where a squad goes when it leaves the nest: recruited to the reported
/// patch, a scouting flight to a random point of the foraging range
/// (fliers), or a search from the nest (walkers).
pub(super) fn departure(
    species: &Species,
    p: &ColonyParams,
    c: &Colony,
    range: f32,
    world: &World,
    rng: &mut simulation_core::SimRng,
) -> ForagerState {
    if let Some((patch, _)) = c.best_patch
        && p.recruitment
        && rng.chance(RECRUITMENT_PROBABILITY)
    {
        return ForagerState::Recruited(patch);
    }
    if species.can(Locomotion::Fly) {
        let (w, h) = (world.width() as f32, world.height() as f32);
        let target = (c.nest + rng.in_disc(range)).clamp(Vec2::splat(0.5), Vec2::new(w - 0.5, h - 0.5));
        return ForagerState::Recruited(target);
    }
    ForagerState::Searching
}

fn load(f: &mut Forager, kj: f64, g: f64, ae: f32, patch: Vec2, quality: f32) {
    f.load_kj += kj;
    f.load_g += g;
    f.load_assimilation = ae;
    f.patch = Some((patch, quality));
}

/// Unloads a squad at the nest: the assimilated share goes to the store,
/// the rest to the litter; the patch is reported.
fn unload(f: &mut Forager, c: &mut Colony, pantry: &mut Pantry<'_>, world: &World, ledgers: &mut Ledgers, area: f64) {
    if f.load_kj <= 0.0 && f.load_g <= 0.0 {
        return;
    }
    let a = f64::from(f.load_assimilation.clamp(0.0, 1.0));
    c.store_kj += a * f.load_kj;
    c.store_g += a * f.load_g;
    let (dung_kj, dung_g) = ((1.0 - a) * f.load_kj, (1.0 - a) * f.load_g);
    let cell = cell_of(world, c.nest);
    pantry.detritus.add_litter_energy(cell, dung_kj / area, (dung_g / area) as f32);
    ledgers.book(Flux::Egestion, dung_kj, dung_g);
    if let Some((patch, quality)) = f.patch
        && c.best_patch.is_none_or(|(_, q)| quality >= q * 0.9)
    {
        c.best_patch = Some((patch, quality));
    }
    f.load_kj = 0.0;
    f.load_g = 0.0;
}

/// Deposits [`PHEROMONE_DEPOSIT`] once on every cell crossed by the segment
/// `from → to` (sampled every half cell), so fast squads leave continuous
/// trails.
fn mark_segment(pheromone: &mut [f32], w: usize, h: usize, from: Vec2, to: Vec2) {
    let samples = ((to - from).length() / 0.5).ceil().max(1.0) as usize;
    let mut last = usize::MAX;
    for k in 0..=samples {
        let p = from.lerp(to, k as f32 / samples as f32);
        let (x, y) = ((p.x.max(0.0) as usize).min(w - 1), (p.y.max(0.0) as usize).min(h - 1));
        let cell = y * w + x;
        if cell != last {
            pheromone[cell] += PHEROMONE_DEPOSIT;
            last = cell;
        }
    }
}

/// Direction of the trail leading away from the nest: the most strongly
/// marked of the 8 neighbouring cells that lie farther from the nest than
/// the squad (outbound ants follow the trail outwards, combining the
/// pheromone with their home vector).
fn trail_direction(pheromone: &[f32], w: usize, h: usize, p: Vec2, nest: Vec2) -> Option<Vec2> {
    let (x, y) = ((p.x.max(0.0) as usize).min(w - 1), (p.y.max(0.0) as usize).min(h - 1));
    let here = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(nest);
    let mut best: Option<(Vec2, f32)> = None;
    for (dx, dy) in crate::vegetation::NEIGHBOURS {
        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
        if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
            continue;
        }
        let outward = Vec2::new(nx as f32 + 0.5, ny as f32 + 0.5).distance(nest) > here;
        let v = pheromone[ny as usize * w + nx as usize];
        if outward && v > PHEROMONE_THRESHOLD && best.is_none_or(|(_, b)| v > b) {
            best = Some((Vec2::new(dx as f32, dy as f32), v));
        }
    }
    best.map(|(d, _)| d.normalize())
}

/// Sensing range of a squad (cells): its vision, at least 1.5 cells.
fn sense_cells(species: &Species, scale: &TimeScale) -> f32 {
    scale.cells(species.perception.vision_m).clamp(1.5, 6.0)
}

/// Nearest carcass with meat within `radius` (species that eat carrion).
fn carrion_nearby(species: &Species, remains: &Remains, p: Vec2, radius: f32) -> Option<Vec2> {
    species.diet_entry(&Food::Carrion)?;
    remains
        .carcasses
        .iter()
        .filter(|c| c.meat_kg > 0.0 && c.position.distance(p) <= radius)
        .min_by(|a, b| a.position.distance(p).total_cmp(&b.position.distance(p)).then(a.id.cmp(&b.id)))
        .map(|c| c.position)
}

/// Best food cell within the species' sensing range (vision, at least 1.5 cells).
fn best_food_nearby(
    species: &Species,
    step: &ColonyStep<'_>,
    pantry: &Pantry<'_>,
    fields: &Fields,
    p: Vec2,
) -> Option<Vec2> {
    let r = sense_cells(species, &step.scale);
    let n = r.ceil() as i32;
    let mut best: Option<(Vec2, f32)> = None;
    for j in -n..=n {
        for i in -n..=n {
            let q = p + Vec2::new(i as f32, j as f32);
            if q.distance(p) > r || step.world.cell_at(q).is_none() || !passable(species, step.world, q) {
                continue;
            }
            let cell = cell_of(step.world, q);
            let value: f32 = species
                .diet
                .iter()
                .filter_map(|d| {
                    CellFood::resolve(&d.food, step.catalog, fields)
                        .map(|f| d.preference * pantry.available(fields, f, cell))
                })
                .sum();
            if value > 0.0 && best.is_none_or(|(_, b)| value > b) {
                best = Some((q, value));
            }
        }
    }
    best.map(|(q, _)| q)
}
