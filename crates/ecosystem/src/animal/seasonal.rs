//! Seasonal behaviour: migration towards the preferred climate and
//! hibernation in the cold season.
//!
//! * **Migration** (`seasonal.migrates`): when the running daily mean
//!   temperature where a migrant stands is more than 1 °C outside its
//!   preferred range (shifted by its thermal gene), it samples the daily
//!   mean at 8 passable points `migration_range` away and heads for the one
//!   closest to its range if it is at least 0.5 °C better. The `migrate`
//!   action scores the species weight times the compensated product of
//!   `discomfort/10` and `1 − hunger` (half weight).
//! * **Hibernation** (`seasonal.hibernation`): outside summer (in autumn,
//!   winter, or a spring that is still cold), an animal whose daily mean
//!   temperature is below `below_c` and whose reserves exceed
//!   `min_reserve_fraction` enters dormancy (score 2.5);
//!   a dormant animal stays (score 2.5) until the daily mean exceeds
//!   `wake_above_c`, a fire or flood threatens it, or its hydration falls
//!   below 0.25 (periodic arousal to drink; it needs 0.5 to go back to
//!   sleep). A torpid ectotherm meeting these conditions goes dormant
//!   instead of staying torpid. Dormant animals hide
//!   in their den (invisible), do not move and run at
//!   `metabolic_fraction` of their basal rate with 5 % of the usual water
//!   loss (see `physiology`).

use physics_world::Season;
use simulation_core::glam::Vec2;

use super::perceive::{Ctx, Senses, ThreatSource};
use super::{Action, Animal};
use crate::environment::passable;
use crate::species::{AnimalTraits, Species};

/// Score of entering or staying in dormancy.
pub const HIBERNATION_SCORE: f32 = 2.5;
/// Discomfort (°C outside the preferred range) below which migrants stay.
const MIGRATION_TRIGGER_C: f32 = 1.0;
/// Least improvement (°C) that makes a direction worth travelling.
const MIGRATION_GAIN_C: f32 = 0.5;
/// Hydration below which a dormant animal rouses to drink.
pub const AROUSAL_HYDRATION: f32 = 0.25;
/// Hydration an animal needs to enter dormancy (it drinks first).
pub const ENTRY_HYDRATION: f32 = 0.5;

/// Degrees outside the preferred range `(lo, hi)`.
pub fn discomfort(t: f32, lo: f32, hi: f32) -> f32 {
    (lo - t).max(0.0) + (t - hi).max(0.0)
}

/// Where a migrant heads, and how uncomfortable it is now (°C).
pub(crate) fn migration_target(
    ctx: &Ctx<'_>,
    me: &Animal,
    species: &Species,
    traits: &AnimalTraits,
    thermal_shift: f32,
) -> Option<(Vec2, f32)> {
    if !species.seasonal.migrates {
        return None;
    }
    let (lo, hi) = (species.climate.preferred_c.0 + thermal_shift, species.climate.preferred_c.1 + thermal_shift);
    let pos = me.kin.position;
    let here = discomfort(ctx.env.day_temperature[ctx.cell_of(pos)], lo, hi);
    if here <= MIGRATION_TRIGGER_C {
        return None;
    }
    let reach = traits.migration_cells.max(2.0);
    let (w, h) = (ctx.env.width as f32, ctx.env.height as f32);
    let mut best: Option<(Vec2, f32)> = None;
    for k in 0..8 {
        let q = pos + Vec2::from_angle(k as f32 * std::f32::consts::TAU / 8.0) * reach;
        let q = q.clamp(Vec2::splat(0.5), Vec2::new(w - 0.5, h - 0.5));
        if q.distance(pos) < 1.0 || !passable(ctx.world, traits, q) {
            continue;
        }
        let d = discomfort(ctx.env.day_temperature[ctx.cell_of(q)], lo, hi);
        if d < here - MIGRATION_GAIN_C && best.is_none_or(|(_, b)| d < b) {
            best = Some((q, d));
        }
    }
    best.map(|(q, _)| (q, here))
}

/// Whether an animal with a hibernation strategy should be (or stay)
/// dormant now.
pub(crate) fn wants_dormancy(
    ctx: &Ctx<'_>,
    me: &Animal,
    species: &Species,
    traits: &AnimalTraits,
    senses: &Senses,
) -> bool {
    let Some(h) = &species.seasonal.hibernation else { return false };
    if senses.threat.is_some_and(|t| matches!(t.source, ThreatSource::Fire | ThreatSource::Water)) {
        return false;
    }
    let t = ctx.env.day_temperature[ctx.cell_of(me.kin.position)];
    if me.action == Action::Hibernate {
        // Periodic arousal: a dormant animal whose water runs low wakes up
        // to drink (hibernating bats rouse every week or two to drink).
        return t <= h.wake_above_c && me.hydration > AROUSAL_HYDRATION;
    }
    if me.hydration < ENTRY_HYDRATION {
        return false;
    }
    // Dormancy starts in the cold part of the year: autumn and winter, and
    // a spring that has not warmed up yet (hibernators stay in or return
    // to their dens through late cold spells instead of dying of exposure).
    let cold_season = ctx.world.time().season != Season::Summer;
    cold_season && t < h.below_c && me.energy_kj >= h.min_reserve_fraction * me.reserve_max_kj(traits)
}
