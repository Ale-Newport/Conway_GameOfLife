//! Contested interactions after movement: attacks, meals and matings are
//! collected as `organism-engine` [`Claim`]s and resolved in a total order
//! that does not depend on iteration order (`ARCHITECTURE.md` §3.2):
//!
//! * **attacks** — [`resolve_exclusive`]: at most one attack per prey per
//!   tick, the best claim (attack strength × hunger, then distance, then id)
//!   wins; outcomes are simultaneous;
//! * **carcasses** — [`resolve_shared`] with the *ordered* rule: the killer
//!   eats first, then the others by dominance ([`carcass_rank`]: predators
//!   before pure scavengers, then aggression × relative body size);
//! * **plants, litter, nectar, mushrooms, browse, density-field prey** —
//!   [`resolve_shared`] with the *proportional* rule per (cell, food); a
//!   forager eats in the cell it heads for once that cell's centre is within
//!   [`CONTACT_CELLS`], otherwise where it stands;
//! * **mating** — [`resolve_matching`]: one male per receptive female;
//! * **territorial fights** — [`resolve_exclusive`]: one owner per intruder
//!   (see `social`).
//!
//! Ground predators cannot attack prey flying more than 1 m above the
//! ground; pack leaders eat before the rest of their pack.
//!
//! The probability that an attack kills is
//!
//! ```text
//! logit p = 2.5 (v_p − v_q)/max(v_p, v_q) + 1.2 ln(A/D) − d/strike − 1.5 cover + (1 − health_q) + ln(efficiency)
//! ```
//!
//! with sprint speeds `v` reduced by stamina and health (an unaware prey
//! reacts late: ×0.6, ×0.45 against stalkers, ×0.3 against ambushers),
//! attack `A = attack · M_p^{1/3} · (1 + 0.3 pack mates)` and defence
//! `D = defence · M_q^{1/3} · (1 + 0.1 herd mates)`.

use organism_engine::intent::{Allocation, Claim, ShareRule, resolve_exclusive, resolve_matching, resolve_shared};
use std::collections::BTreeMap;

use organism_engine::AgentId;
use simulation_core::SimRng;
use simulation_core::glam::Vec2;

use super::genetics::Phenotype;
use super::perceive::{Ctx, available_entry, available_food, compatible_pair, food_value, reachable};
use super::social::{Fight, leader_of, resolve_fight};
use super::{Action, Animal, Target};
use crate::environment::drinkable;
use crate::species::{Food, HuntingStyle, PlantLayer, SocialStructure};
use crate::units::{DETRITUS_KJ_PER_KG, FUNGI_KJ_PER_KG, PLANT_KJ_PER_KG, TISSUE_KJ_PER_KG};

/// Distance (cells) within which an animal can eat a carcass or court a mate.
pub const CONTACT_CELLS: f32 = 1.0;

/// Where a meal comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FoodSource {
    /// A vegetation layer of a cell.
    Plant { cell: usize, layer: PlantLayer },
    /// Tree foliage within reach in a cell.
    Browse { cell: usize },
    /// Flower nectar of a cell.
    Nectar { cell: usize },
    /// Litter of a cell.
    Detritus { cell: usize },
    /// Mushrooms of a cell.
    Mushrooms { cell: usize },
    /// Density-field organisms (insects, worms) of a cell.
    Field { cell: usize, slot: usize },
    /// The eggs of a clutch (nest predation).
    Clutch { id: u64 },
    /// A carcass.
    Carcass { id: u64 },
}

/// A granted meal (amounts are removed from the source when applied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Meal {
    pub eater: AgentId,
    pub source: FoodSource,
    /// Mass granted (kg).
    pub kg: f32,
    /// Assimilation efficiency.
    pub assimilation: f32,
}

/// Outcome of one attack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Attack {
    pub predator: AgentId,
    pub prey: AgentId,
    pub probability: f32,
    pub success: bool,
    /// Health lost by the prey when it escapes.
    pub prey_injury: f32,
    /// Health lost by the predator.
    pub predator_injury: f32,
}

/// Everything resolved this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Interactions {
    pub attacks: Vec<Attack>,
    pub meals: Vec<Meal>,
    pub matings: Vec<(AgentId, AgentId)>,
    pub drinkers: Vec<AgentId>,
    pub fights: Vec<Fight>,
}

/// Codes per cell of shared food claims: `cell × FOOD_CODES + code`.
const FOOD_CODES: u64 = 2048;
/// First code of density-field prey (`FIELD_CODE + field slot`).
const FIELD_CODE: u64 = 64;

/// Food code of diet entry `k` of species `s` (`None` for foods that are
/// not foraged from cells).
fn code(ctx: &Ctx<'_>, s: usize, k: usize) -> Option<u64> {
    match &ctx.catalog.animals()[s].diet[k].food {
        Food::Plant(layer) => Some(layer.index() as u64),
        Food::Browse => Some(7),
        Food::Nectar => Some(8),
        Food::Detritus => Some(9),
        Food::Fungi => Some(10),
        Food::Prey(_) => {
            ctx.catalog.diet_prey(s, k).and_then(|p| ctx.fields.slot(p)).map(|slot| FIELD_CODE + slot as u64)
        }
        Food::Carrion | Food::Eggs => None,
    }
}

fn source_of(target: u64) -> FoodSource {
    let cell = (target / FOOD_CODES) as usize;
    match target % FOOD_CODES {
        c @ 0..=6 => FoodSource::Plant { cell, layer: PlantLayer::ALL[c as usize] },
        7 => FoodSource::Browse { cell },
        8 => FoodSource::Nectar { cell },
        9 => FoodSource::Detritus { cell },
        10 => FoodSource::Mushrooms { cell },
        c => FoodSource::Field { cell, slot: (c - FIELD_CODE) as usize },
    }
}

/// Edible amount (kg/m²) behind a food-claim target.
fn available_at(ctx: &Ctx<'_>, target: u64) -> f32 {
    match source_of(target) {
        FoodSource::Plant { cell, layer } => available_food(ctx, &Food::Plant(layer), cell),
        FoodSource::Browse { cell } => available_food(ctx, &Food::Browse, cell),
        FoodSource::Nectar { cell } => available_food(ctx, &Food::Nectar, cell),
        FoodSource::Detritus { cell } => available_food(ctx, &Food::Detritus, cell),
        FoodSource::Mushrooms { cell } => available_food(ctx, &Food::Fungi, cell),
        FoodSource::Field { cell, slot } => ctx.fields.biomass_kg_m2(slot, cell),
        FoodSource::Carcass { .. } | FoodSource::Clutch { .. } => 0.0,
    }
}

/// Gross energy density (kJ/kg) of a foraged food.
pub(crate) fn energy_density(food: &Food) -> f64 {
    match food {
        Food::Detritus => DETRITUS_KJ_PER_KG,
        Food::Fungi => FUNGI_KJ_PER_KG,
        Food::Prey(_) | Food::Carrion | Food::Eggs => TISSUE_KJ_PER_KG,
        _ => PLANT_KJ_PER_KG,
    }
}

/// Largest mass (kg) an animal wants to eat this tick of a food with
/// `kj_per_kg` gross energy and assimilation `ae`.
pub(crate) fn wanted_kg(me: &Animal, ctx: &Ctx<'_>, kj_per_kg: f64, ae: f32) -> f32 {
    let traits = &ctx.traits[usize::from(me.species)];
    let room = (me.reserve_max_kj(traits) - me.energy_kj).max(0.0);
    let cap = traits.intake_kj_per_tick * (me.mass_kg / traits.adult_mass_kg).max(0.05).powf(0.75);
    let gross = cap.min(room / ae.max(1e-3));
    (f64::from(gross) / kj_per_kg.max(1.0)) as f32
}

/// Builds and resolves the claims of every animal (serial, id order).
pub(crate) fn interact(ctx: &Ctx<'_>, efficiency: f32) -> Interactions {
    let mut out = Interactions::default();
    let mut attack_claims = Vec::new();
    let mut meat_claims = Vec::new();
    let mut food_claims = Vec::new();
    let mut food_assimilation: BTreeMap<AgentId, f32> = BTreeMap::new();
    let mut mate_claims = Vec::new();
    let mut fight_claims = Vec::new();
    let mut egg_claims = Vec::new();
    let area = f64::from(ctx.scale.cell_area_m2());
    let animals = ctx.animals.as_slice();
    for (index, me) in animals.iter().enumerate() {
        let id = ctx.animals.ids()[index];
        let s = usize::from(me.species);
        let species = &ctx.catalog.animals()[s];
        let traits = &ctx.traits[s];
        let pos = me.kin.position;
        let cell = ctx.cell_of(pos);
        match (me.action, me.target) {
            (Action::Hunt, Target::Animal(prey_id)) => {
                if let Some(prey) = ctx.animals.get(prey_id)
                    && !prey.hidden
                    && reachable(traits, prey)
                {
                    let distance = prey.kin.position.distance(pos);
                    if distance <= traits.strike_cells {
                        let hunger = me.hunger(traits);
                        let priority = species.combat.attack * me.mass_kg.cbrt() * (0.5 + hunger);
                        attack_claims.push(Claim::new(id, prey_id.get()).priority(priority).distance(distance));
                    }
                }
            }
            (Action::Feed, Target::Carcass(cid)) => {
                if let Some(c) = ctx.remains.carcass(cid)
                    && c.position.distance(pos) <= CONTACT_CELLS
                    && c.meat_kg > 0.0
                {
                    let ae = species.diet_entry(&Food::Carrion).map_or(0.8, |d| d.assimilation());
                    let kj_per_kg = c.energy_kj / f64::from(c.meat_kg);
                    let kg = wanted_kg(me, ctx, kj_per_kg, ae);
                    let own = c.killer == Some(id);
                    let alpha = leader_of(ctx, me.group) == Some(id);
                    let priority =
                        carcass_rank(own, ctx.catalog.hunts_animals(me.species), species.combat.aggression, me.mass_kg)
                            + if alpha && !own { 0.5 } else { 0.0 };
                    if kg > 0.0 {
                        meat_claims
                            .push(Claim::new(id, cid).priority(priority).distance(c.position.distance(pos)).amount(kg));
                    }
                }
            }
            (Action::Graze | Action::Browse | Action::Forage, target) => {
                // The food cell the animal heads for counts as within reach
                // once its centre is within contact distance (the animal is
                // at its edge); otherwise the animal feeds where it stands.
                let cell = match target {
                    Target::Cell(c)
                        if (c as usize) < ctx.env.width * ctx.env.height
                            && ctx.centre(c as usize).distance(pos) <= CONTACT_CELLS
                            && food_value(ctx, s, c as usize).is_some() =>
                    {
                        c as usize
                    }
                    _ => cell,
                };
                if let Some((_, k)) = food_value(ctx, s, cell)
                    && let Some(code) = code(ctx, s, k)
                {
                    let entry = &species.diet[k];
                    let ae = entry.assimilation();
                    let kg = wanted_kg(me, ctx, energy_density(&entry.food), ae);
                    let available = available_entry(ctx, s, k, cell);
                    if kg > 0.0 && available > 0.0 {
                        let target = cell as u64 * FOOD_CODES + code;
                        food_claims.push(Claim::new(id, target).priority(me.hunger(traits)).amount(kg));
                        food_assimilation.insert(id, ae);
                    }
                }
            }
            (Action::Feed, Target::Clutch(cid)) => {
                if let Some(c) = ctx.remains.clutch(cid)
                    && c.position.distance(pos) <= CONTACT_CELLS
                    && c.eggs > 0
                {
                    let ae = species.diet_entry(&Food::Eggs).map_or(0.8, |d| d.assimilation());
                    let kg = wanted_kg(me, ctx, TISSUE_KJ_PER_KG, ae);
                    if kg > 0.0 {
                        egg_claims.push(
                            Claim::new(id, cid)
                                .priority(me.hunger(traits))
                                .distance(c.position.distance(pos))
                                .amount(kg),
                        );
                    }
                }
            }
            (Action::Defend, Target::Animal(intruder)) => {
                if let Some(other) = ctx.animals.get(intruder) {
                    let distance = other.kin.position.distance(pos);
                    if distance <= CONTACT_CELLS.max(traits.strike_cells) {
                        fight_claims.push(Claim::new(id, intruder.get()).priority(me.mass_kg).distance(distance));
                    }
                }
            }
            (Action::SeekMate, Target::Animal(mate_id)) => {
                if let Some(mate) = ctx.animals.get(mate_id)
                    && mate.kin.position.distance(pos) <= CONTACT_CELLS
                    && compatible_pair((id, me), (mate_id, mate), ctx)
                {
                    mate_claims.push(
                        Claim::new(id, mate_id.get())
                            .priority(me.health * me.mass_kg)
                            .distance(mate.kin.position.distance(pos)),
                    );
                }
            }
            (Action::Drink, _) if near_water(ctx, cell) => out.drinkers.push(id),
            _ => {}
        }
    }
    for claim in resolve_exclusive(&attack_claims) {
        if let Some(attack) = resolve_attack(ctx, claim.claimant, AgentId(claim.target), efficiency) {
            out.attacks.push(attack);
        }
    }
    let carcass_grants =
        resolve_shared(&meat_claims, |cid| ctx.remains.carcass(cid).map_or(0.0, |c| c.meat_kg), ShareRule::Ordered);
    for Allocation { claimant, target, granted, .. } in carcass_grants {
        if granted > 0.0 {
            let ae = ctx
                .animals
                .get(claimant)
                .and_then(|a| ctx.catalog.animals()[usize::from(a.species)].diet_entry(&Food::Carrion))
                .map_or(0.8, |d| d.assimilation());
            out.meals.push(Meal {
                eater: claimant,
                source: FoodSource::Carcass { id: target },
                kg: granted,
                assimilation: ae,
            });
        }
    }
    let food_grants =
        resolve_shared(&food_claims, |target| available_at(ctx, target) * area as f32, ShareRule::Proportional);
    for grant in food_grants {
        if grant.granted > 0.0 {
            let ae = food_assimilation.get(&grant.claimant).copied().unwrap_or(0.5);
            out.meals.push(Meal {
                eater: grant.claimant,
                source: source_of(grant.target),
                kg: grant.granted,
                assimilation: ae,
            });
        }
    }
    for claim in resolve_matching(&mate_claims) {
        out.matings.push((claim.claimant, AgentId(claim.target)));
    }
    let egg_grants =
        resolve_shared(&egg_claims, |cid| ctx.remains.clutch(cid).map_or(0.0, |c| c.egg_kg()), ShareRule::Ordered);
    for Allocation { claimant, target, granted, .. } in egg_grants {
        if granted > 0.0 {
            let ae = ctx
                .animals
                .get(claimant)
                .and_then(|a| ctx.catalog.animals()[usize::from(a.species)].diet_entry(&Food::Eggs))
                .map_or(0.8, |d| d.assimilation());
            out.meals.push(Meal {
                eater: claimant,
                source: FoodSource::Clutch { id: target },
                kg: granted,
                assimilation: ae,
            });
        }
    }
    for claim in resolve_exclusive(&fight_claims) {
        if let Some(fight) = resolve_fight(ctx, claim.claimant, AgentId(claim.target)) {
            out.fights.push(fight);
        }
    }
    out
}

/// Rank of a claim on a carcass (higher eats first): the killer, then by
/// dominance `class + aggression · m^{1/3} / (1 + m^{1/3})` with class 1 for
/// predators and 0 for pure scavengers, so large aggressive species displace
/// small timid ones at a shared carcass.
pub fn carcass_rank(own_kill: bool, predator: bool, aggression: f32, mass_kg: f32) -> f32 {
    if own_kill {
        return 3.0;
    }
    let size = mass_kg.max(0.0).cbrt();
    f32::from(u8::from(predator)) + aggression.clamp(0.0, 1.0) * size / (1.0 + size)
}

/// Whether drinkable water is in the cell or one of its neighbours.
fn near_water(ctx: &Ctx<'_>, cell: usize) -> bool {
    let (w, h) = (ctx.env.width as i64, ctx.env.height as i64);
    let (x, y) = ((cell as i64) % w, (cell as i64) / w);
    (-1..=1).any(|dy| {
        (-1..=1).any(|dx| {
            let (nx, ny) = (x + dx, y + dy);
            nx >= 0 && ny >= 0 && nx < w && ny < h && drinkable(ctx.world, (ny * w + nx) as usize)
        })
    })
}

/// Effective sprint speed (cells per tick) of an animal.
fn sprint(ctx: &Ctx<'_>, a: &Animal) -> f32 {
    let traits = &ctx.traits[usize::from(a.species)];
    let pheno = Phenotype::of(&a.genome, ctx.schema);
    let exhausted = if a.stamina > 0.15 { 1.0 } else { traits.cruise_cells / traits.sprint_cells.max(1e-6) };
    let sick = 1.0 - 0.5 * ctx.epi.virulence(a);
    traits.sprint_cells * pheno.speed * (0.5 + 0.5 * a.health) * exhausted * a.impairment() * sick
}

/// Number of animals of `species` within `radius` of `p` satisfying `filter`.
fn count_near(ctx: &Ctx<'_>, p: Vec2, radius: f32, mut filter: impl FnMut(&Animal) -> bool) -> usize {
    ctx.grid.count_within(p, radius, |slot| filter(&ctx.animals.as_slice()[slot as usize]))
}

/// Resolves one attack (pure apart from the attack's own random stream).
pub(crate) fn resolve_attack(ctx: &Ctx<'_>, predator_id: AgentId, prey_id: AgentId, efficiency: f32) -> Option<Attack> {
    let predator = ctx.animals.get(predator_id)?;
    let prey = ctx.animals.get(prey_id)?;
    let ps = &ctx.catalog.animals()[usize::from(predator.species)];
    let qs = &ctx.catalog.animals()[usize::from(prey.species)];
    let traits = &ctx.traits[usize::from(predator.species)];
    let aware = prey.action == Action::Flee;
    let surprise = if aware {
        1.0
    } else {
        match ps.behaviour.hunting {
            HuntingStyle::Ambush => 0.3,
            HuntingStyle::Stalk => 0.45,
            _ => 0.6,
        }
    };
    let v_p = sprint(ctx, predator);
    let v_q = sprint(ctx, prey) * surprise;
    let speed_term = (v_p - v_q) / v_p.max(v_q).max(1e-6);
    let pack = count_near(ctx, predator.kin.position, 3.0, |a| {
        a.species == predator.species && a.action == Action::Hunt && a.target == Target::Animal(prey_id)
    })
    .saturating_sub(1) as f32;
    let herd =
        if matches!(qs.social.structure, SocialStructure::Herd | SocialStructure::Colony | SocialStructure::Flock) {
            (count_near(ctx, prey.kin.position, 2.0, |a| a.species == prey.species).saturating_sub(1) as f32).min(10.0)
        } else {
            0.0
        };
    let attack = (ps.combat.attack * predator.mass_kg.cbrt() * (1.0 + 0.3 * pack)).max(1e-6);
    let defence = (qs.combat.defence * prey.mass_kg.cbrt() * (1.0 + 0.1 * herd)).max(1e-6);
    let distance = predator.kin.position.distance(prey.kin.position);
    let cover = ctx.env.cover[ctx.cell_of(prey.kin.position)];
    let logit =
        2.5 * speed_term + 1.2 * (attack / defence).ln() - distance / traits.strike_cells.max(1e-6) - 1.5 * cover
            + (1.0 - prey.health)
            + efficiency.max(1e-6).ln();
    let probability = 1.0 / (1.0 + (-logit).exp());
    let mut rng: SimRng = crate::rng::stream(ctx.seed, crate::rng::ATTACK, ctx.tick, predator_id.get());
    let success = rng.chance(f64::from(probability));
    let (prey_injury, predator_injury) = if success {
        (0.0, 0.0)
    } else {
        let prey_hurt = if rng.chance(f64::from(0.3 * probability)) { 0.25 } else { 0.0 };
        let risk = defence / (attack + defence);
        let predator_hurt = if rng.chance(f64::from(0.5 * risk)) { 0.15 * risk } else { 0.0 };
        (prey_hurt, predator_hurt)
    };
    Some(Attack { predator: predator_id, prey: prey_id, probability, success, prey_injury, predator_injury })
}

#[cfg(test)]
mod tests {
    use super::carcass_rank;
    use crate::species::SpeciesCatalog;

    #[test]
    fn carcass_dominance_follows_kill_class_aggression_and_size() {
        let catalog = SpeciesCatalog::builtin().unwrap();
        let rank = |id: &str, own: bool| {
            let i = catalog.animal_index(id).unwrap();
            let s = catalog.animal(id).unwrap();
            carcass_rank(own, catalog.hunts_animals(i), s.combat.aggression, s.body.mass_kg)
        };
        // The killer eats first whoever it is; then wolves displace foxes,
        // and predators displace the pure scavenger.
        assert!(rank("fox", true) > rank("wolf", false));
        assert!(rank("wolf", false) > rank("fox", false));
        assert!(rank("fox", false) > rank("raven", false));
        assert!((0.0..1.0).contains(&rank("raven", false)));
    }
}
