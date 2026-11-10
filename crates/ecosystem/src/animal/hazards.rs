//! Hazards and water: fire (with escape to burned-out or unburnable
//! ground), water deeper than an animal can wade, and the search for water
//! by thirsty animals that know none. Pure functions of read-only state,
//! called from [`super::perceive`].

use physics_world::FireState;
use simulation_core::glam::Vec2;

use super::Animal;
use super::perceive::{Ctx, Senses, Threat, ThreatSource};
use crate::environment::{in_hostile_water, passable};
use crate::species::AnimalTraits;

/// Distance (cells) of a fire within which an animal sprints to safe
/// ground, possibly through the fire front; farther fires are walked away
/// from at cruising speed (saving stamina for the dash).
pub(crate) const FIRE_ESCAPE_CELLS: f32 = 3.0;
/// Largest radius (cells) scanned for safe ground (a wind-driven front is
/// several cells deep).
const FIRE_REFUGE_SCAN_CELLS: f32 = 12.0;

/// Where a thirsty animal that knows no water looks for it: the lowest
/// passable point among 8 directions at `distance` cells if it lies below
/// the animal (water collects in valleys), otherwise straight ahead (a
/// straight search covers ground faster than a random walk).
pub(crate) fn water_search(ctx: &Ctx<'_>, me: &Animal, traits: &AnimalTraits, distance: f32) -> Option<Vec2> {
    let pos = me.kin.position;
    let height = |p: Vec2| ctx.world.elevation().as_slice()[ctx.cell_of(p)];
    let here = height(pos);
    let mut lowest: Option<(Vec2, f32)> = None;
    for k in 0..8 {
        let q = pos + Vec2::from_angle(k as f32 * std::f32::consts::TAU / 8.0) * distance;
        if ctx.world.cell_at(q).is_none() || !passable(ctx.world, traits, q) {
            continue;
        }
        let h = height(q);
        if h < here - 0.05 && lowest.is_none_or(|(_, best)| h < best) {
            lowest = Some((q, h));
        }
    }
    let ahead = pos + me.kin.direction() * distance;
    lowest
        .map(|(q, _)| q)
        .or_else(|| (ctx.world.cell_at(ahead).is_some() && passable(ctx.world, traits, ahead)).then_some(ahead))
}

/// Fire and deep water.
pub(crate) fn perceive_hazards(ctx: &Ctx<'_>, me: &Animal, traits: &AnimalTraits, vision: f32, senses: &mut Senses) {
    let pos = me.kin.position;
    if let Some((away, distance)) = ctx.env.fire_near(pos, vision) {
        let level = if distance < 2.0 { 1.0 } else { (1.0 - distance / vision).clamp(0.3, 1.0) };
        if senses.threat.is_none_or(|t| level >= t.level) {
            senses.threat = Some(Threat {
                source: ThreatSource::Fire,
                position: pos - away.normalize_or_zero() * distance.max(0.5),
                velocity: Vec2::ZERO,
                distance,
                level,
            });
        }
        if distance < FIRE_ESCAPE_CELLS {
            let radius = vision.clamp(2.0, FIRE_REFUGE_SCAN_CELLS);
            senses.fire_refuge = fire_refuge(ctx, me, traits, radius);
        }
    }
    if in_hostile_water(ctx.world, traits, pos) {
        // Flee from the deepest direction: look for the nearest passable
        // cell among 16 directions at 1–3 cells.
        let mut best: Option<Vec2> = None;
        'search: for r in 1..=3 {
            for k in 0..16 {
                let dir = Vec2::from_angle(k as f32 * std::f32::consts::TAU / 16.0);
                let q = pos + dir * r as f32;
                if passable(ctx.world, traits, q) && !in_hostile_water(ctx.world, traits, q) {
                    best = Some(q);
                    break 'search;
                }
            }
        }
        let away_from = best.map_or(pos, |q| pos - (q - pos));
        senses.threat = Some(Threat {
            source: ThreatSource::Water,
            position: away_from,
            velocity: Vec2::ZERO,
            distance: 0.0,
            level: 1.0,
        });
    }
}

/// Nearest safe ground from a fire within `radius` cells: a passable cell
/// that is not burning and cannot burn (burned out, bare, wet or water the
/// animal can stand in). Real animals escape to burned ground behind the
/// front or to water rather than run ahead of the fire.
fn fire_refuge(ctx: &Ctx<'_>, me: &Animal, traits: &AnimalTraits, radius: f32) -> Option<(Vec2, f32)> {
    let pos = me.kin.position;
    let fire = ctx.world.fire().as_slice();
    let n = radius.ceil() as i32;
    let mut best: Option<(Vec2, f32)> = None;
    for j in -n..=n {
        for i in -n..=n {
            let p = pos + Vec2::new(i as f32, j as f32);
            if ctx.world.cell_at(p).is_none() || p.distance(pos) > radius {
                continue;
            }
            let cell = ctx.cell_of(p);
            let safe = matches!(fire[cell], FireState::Unburnable | FireState::Burned { .. });
            if !safe || !passable(ctx.world, traits, p) || in_hostile_water(ctx.world, traits, p) {
                continue;
            }
            let centre = ctx.centre(cell);
            let d = centre.distance(pos);
            if best.is_none_or(|(_, b)| d < b) {
                best = Some((centre, d));
            }
        }
    }
    best
}
