//! Reynolds-style steering behaviours.
//!
//! Every behaviour returns a *desired acceleration* (steering force per unit
//! mass) whose length never exceeds `limits.max_accel`; feed the (combined)
//! result to [`crate::kinematics::Kinematics::integrate`]. Most behaviours use
//! Reynolds' formulation `steering = desired_velocity − velocity` (Reynolds,
//! "Steering Behaviors For Autonomous Characters", GDC 1999).
//!
//! Positions are taken literally. In a toroidal world pass the nearest image
//! of a target, `agent.position + bounds.delta(agent.position, target)`;
//! wrap-aware perception ([`crate::perception::Query::in_world`]) already
//! reports neighbours at that image ([`crate::perception::Percept::position`]),
//! so neighbour [`Kinematics`] for the boids rules can be built from it.
//! Behaviours are pure functions except [`wander`], which keeps its state in a
//! [`WanderState`] and draws from a caller-provided [`SimRng`].
//!
//! Desired velocities use [`MotionLimits::desired_speed`], so an agent with
//! unlimited (infinite) `max_speed` or `max_accel` steers at full strength
//! instead of producing a NaN that [`truncate`] would discard.

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;
use simulation_core::glam::Vec2;

use crate::kinematics::{Kinematics, MotionLimits, truncate};

/// Steers at full speed towards `target`.
pub fn seek(agent: &Kinematics, target: Vec2, limits: MotionLimits) -> Vec2 {
    let desired = (target - agent.position).normalize_or_zero() * limits.desired_speed();
    truncate(desired - agent.velocity, limits.max_accel)
}

/// Steers at full speed directly away from `threat`. An agent standing exactly
/// on the threat flees along its current direction.
pub fn flee(agent: &Kinematics, threat: Vec2, limits: MotionLimits) -> Vec2 {
    let away = (agent.position - threat).try_normalize().unwrap_or_else(|| agent.direction());
    truncate(away * limits.desired_speed() - agent.velocity, limits.max_accel)
}

/// Seeks `target` but slows down inside `slowing_radius`: the desired speed is
/// `max_speed · min(1, distance / slowing_radius)`, reaching zero at the target.
pub fn arrive(agent: &Kinematics, target: Vec2, slowing_radius: f32, limits: MotionLimits) -> Vec2 {
    let offset = target - agent.position;
    let distance = offset.length();
    let speed = if slowing_radius > 0.0 && distance < slowing_radius {
        // Ratio first: `max_speed · distance` could overflow for unlimited agents.
        limits.desired_speed() * (distance / slowing_radius)
    } else {
        limits.desired_speed()
    };
    truncate(offset.normalize_or_zero() * speed - agent.velocity, limits.max_accel)
}

/// Prediction horizon used by [`pursue`] and [`evade`] (Millington & Funge,
/// *Artificial Intelligence for Games*, §3.3.6): `distance / current speed`,
/// capped at `max_prediction`.
fn prediction_time(agent: &Kinematics, other: &Kinematics, max_prediction: f32) -> f32 {
    let max_prediction = if max_prediction.is_finite() { max_prediction.max(0.0) } else { 0.0 };
    let distance = agent.position.distance(other.position);
    let speed = agent.speed();
    if speed <= 0.0 || distance >= speed * max_prediction { max_prediction } else { distance / speed }
}

/// Seeks the position `quarry` will reach if it keeps its velocity for the
/// prediction horizon `T = min(distance / current speed, max_prediction)`
/// (`T = max_prediction` while the agent stands still).
pub fn pursue(agent: &Kinematics, quarry: &Kinematics, max_prediction: f32, limits: MotionLimits) -> Vec2 {
    let t = prediction_time(agent, quarry, max_prediction);
    seek(agent, quarry.position + quarry.velocity * t, limits)
}

/// Flees from the predicted position of `threat` (mirror of [`pursue`]).
pub fn evade(agent: &Kinematics, threat: &Kinematics, max_prediction: f32, limits: MotionLimits) -> Vec2 {
    let t = prediction_time(agent, threat, max_prediction);
    flee(agent, threat.position + threat.velocity * t, limits)
}

/// Geometry of the [`wander`] behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WanderParams {
    /// Distance of the wander circle's centre ahead of the agent.
    pub distance: f32,
    /// Radius of the wander circle.
    pub radius: f32,
    /// Maximum change of the target angle per call, in radians.
    pub jitter: f32,
}

impl Default for WanderParams {
    fn default() -> Self {
        Self { distance: 2.0, radius: 1.0, jitter: 0.5 }
    }
}

/// Per-agent state of the [`wander`] behaviour: the angle of the target point
/// on the wander circle, relative to the agent's direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WanderState {
    /// Target angle on the wander circle in radians, kept in `(-π, π]`.
    pub angle: f32,
}

/// Reynolds' wander: a target point moves randomly on a circle placed
/// `params.distance` ahead of the agent; the agent seeks it. Each call draws
/// exactly one number from `rng` (`angle += U(-jitter, jitter)`), producing a
/// smooth, deterministic random walk of the heading.
pub fn wander(
    agent: &Kinematics,
    state: &mut WanderState,
    params: &WanderParams,
    rng: &mut SimRng,
    limits: MotionLimits,
) -> Vec2 {
    let jitter = if params.jitter.is_finite() { params.jitter.abs() } else { 0.0 };
    let step = rng.range_f32(-jitter, jitter);
    state.angle = wrap_angle(state.angle + step);
    let forward = agent.direction();
    let centre = agent.position + forward * params.distance;
    let target = centre + Vec2::from_angle(state.angle).rotate(forward) * params.radius;
    seek(agent, target, limits)
}

/// Wraps an angle into `(-π, π]`; non-finite angles become `0`.
fn wrap_angle(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    if !angle.is_finite() {
        return 0.0;
    }
    let a = (angle + PI).rem_euclid(TAU) - PI;
    if a <= -PI { a + TAU } else { a }
}

/// Neighbours within `radius` of `position` as `(neighbour, position −
/// neighbour position, distance²)`, excluding co-located ones (they carry no
/// direction, and include the agent itself if it is part of the slice). A
/// negative or NaN radius selects nobody.
fn in_radius<'a>(
    position: Vec2,
    neighbours: impl IntoIterator<Item = &'a Kinematics>,
    radius: f32,
) -> impl Iterator<Item = (&'a Kinematics, Vec2, f32)> {
    // NaN compares false, so it also yields -1 and selects nobody.
    let r2 = if radius >= 0.0 { radius * radius } else { -1.0 };
    neighbours.into_iter().filter_map(move |n| {
        let offset = position - n.position;
        let d2 = offset.length_squared();
        (d2 > 0.0 && d2 <= r2).then_some((n, offset, d2))
    })
}

/// Boids separation: steer away from neighbours closer than `radius`, each
/// contributing `offset / distance²` (unit direction weighted by inverse
/// distance). Neighbours at exactly the agent's position are ignored, so the
/// slice may include the agent itself.
pub fn separation<'a>(
    agent: &Kinematics,
    neighbours: impl IntoIterator<Item = &'a Kinematics>,
    radius: f32,
    limits: MotionLimits,
) -> Vec2 {
    let push: Vec2 = in_radius(agent.position, neighbours, radius).map(|(_, offset, d2)| offset / d2).sum();
    match push.try_normalize() {
        Some(dir) => truncate(dir * limits.desired_speed() - agent.velocity, limits.max_accel),
        None => Vec2::ZERO,
    }
}

/// Boids alignment: match the mean velocity direction of neighbours within
/// `radius` (at full speed). Returns zero when there are none or their mean
/// velocity is zero.
pub fn alignment<'a>(
    agent: &Kinematics,
    neighbours: impl IntoIterator<Item = &'a Kinematics>,
    radius: f32,
    limits: MotionLimits,
) -> Vec2 {
    let heading: Vec2 = in_radius(agent.position, neighbours, radius).map(|(n, _, _)| n.velocity).sum();
    match heading.try_normalize() {
        Some(dir) => truncate(dir * limits.desired_speed() - agent.velocity, limits.max_accel),
        None => Vec2::ZERO,
    }
}

/// Boids cohesion: seek the centroid of neighbours within `radius`.
pub fn cohesion<'a>(
    agent: &Kinematics,
    neighbours: impl IntoIterator<Item = &'a Kinematics>,
    radius: f32,
    limits: MotionLimits,
) -> Vec2 {
    let (sum, count) = in_radius(agent.position, neighbours, radius)
        .fold((Vec2::ZERO, 0u32), |(s, c), (n, _, _)| (s + n.position, c + 1));
    if count == 0 { Vec2::ZERO } else { seek(agent, sum / count as f32, limits) }
}

/// Probe rays ("feelers") used by [`avoid_obstacles`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feelers {
    /// Length of each feeler in world units.
    pub length: f32,
    /// Angle between the central feeler and each side feeler, in radians.
    pub spread: f32,
    /// Number of evenly spaced samples per feeler (clamped to `1..=64`).
    pub samples: u32,
}

impl Default for Feelers {
    fn default() -> Self {
        Self { length: 3.0, spread: 0.6, samples: 6 }
    }
}

/// Fraction of the feeler length at which the first impassable sample lies.
fn first_hit(origin: Vec2, dir: Vec2, feelers: &Feelers, is_passable: &mut impl FnMut(Vec2) -> bool) -> Option<f32> {
    let samples = feelers.samples.clamp(1, 64);
    (1..=samples).map(|k| k as f32 / samples as f32).find(|&t| !is_passable(origin + dir * (feelers.length * t)))
}

/// Obstacle and edge avoidance by sampling. Three feelers (straight ahead and
/// rotated by `±spread`) are sampled from the agent along its direction of
/// travel; `is_passable` decides what blocks (walls, water, world edges...).
///
/// * No feeler hits: returns zero.
/// * Otherwise, with `urgency = (samples − k + 1) / samples` where `k` is the
///   index (1 = nearest) of the closest blocked sample over all feelers, the
///   agent turns away from the side whose feeler hits closer (towards the
///   positive-angle side when both are equal) with lateral acceleration
///   `max_accel·urgency`, and additionally brakes with `max_accel·urgency`
///   when the central feeler hits. The sum is truncated to `max_accel`.
///
/// This is a sampled approximation: obstacles thinner than the sample spacing
/// (`length / samples`) can be missed.
pub fn avoid_obstacles(
    agent: &Kinematics,
    feelers: &Feelers,
    limits: MotionLimits,
    mut is_passable: impl FnMut(Vec2) -> bool,
) -> Vec2 {
    let forward = agent.direction();
    let positive = Vec2::from_angle(feelers.spread).rotate(forward);
    let negative = Vec2::from_angle(-feelers.spread).rotate(forward);
    let centre_hit = first_hit(agent.position, forward, feelers, &mut is_passable);
    let positive_hit = first_hit(agent.position, positive, feelers, &mut is_passable);
    let negative_hit = first_hit(agent.position, negative, feelers, &mut is_passable);
    let closest = [centre_hit, positive_hit, negative_hit].into_iter().flatten().reduce(f32::min);
    let Some(closest) = closest else {
        return Vec2::ZERO;
    };
    // `closest` is k / samples, so this is (samples − k + 1) / samples.
    let urgency = (1.0 - closest + 1.0 / feelers.samples.clamp(1, 64) as f32).min(1.0);
    // Perpendicular rotated by +90° = the positive-angle side.
    let positive_side = forward.perp();
    let lateral = if positive_hit.unwrap_or(f32::INFINITY) < negative_hit.unwrap_or(f32::INFINITY) {
        -positive_side
    } else {
        positive_side
    };
    let brake = if centre_hit.is_some() { -forward } else { Vec2::ZERO };
    // truncate(v·a, a) = a·truncate(v, 1); this form stays finite for an
    // unlimited (infinite) acceleration, which is capped at `f32::MAX`.
    let accel = if limits.max_accel.is_nan() { 0.0 } else { limits.max_accel.clamp(0.0, f32::MAX) };
    truncate((lateral + brake) * urgency, 1.0) * accel
}

/// Prioritised acceleration allocation (Reynolds 1999): behaviours are visited
/// in order, each scaled by its weight; each one adds as much of itself as the
/// remaining acceleration budget `max_accel − |sum|` allows (truncating the
/// last one), and lower-priority behaviours are ignored once the budget is
/// spent. Put collision avoidance first and wandering last.
pub fn combine_prioritized(behaviours: &[(Vec2, f32)], max_accel: f32) -> Vec2 {
    let mut total = Vec2::ZERO;
    for &(accel, weight) in behaviours {
        let remaining = max_accel - total.length();
        if remaining.is_nan() || remaining <= 0.0 {
            break;
        }
        let weight = if weight.is_finite() { weight } else { 0.0 };
        total += truncate(accel * weight, remaining);
    }
    truncate(total, max_accel)
}

/// Weighted blending: `truncate(Σ weight·accel, max_accel)`. Simpler than
/// [`combine_prioritized`] but behaviours can cancel each other out.
pub fn combine_weighted(behaviours: &[(Vec2, f32)], max_accel: f32) -> Vec2 {
    let sum: Vec2 =
        behaviours.iter().filter(|(a, w)| a.is_finite() && w.is_finite()).map(|&(accel, weight)| accel * weight).sum();
    truncate(sum, max_accel)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: MotionLimits = MotionLimits { max_speed: 2.0, max_accel: 1.0 };

    fn at(x: f32, y: f32) -> Kinematics {
        Kinematics::new(Vec2::new(x, y), 0.0)
    }

    fn moving(x: f32, y: f32, vx: f32, vy: f32) -> Kinematics {
        Kinematics { position: Vec2::new(x, y), velocity: Vec2::new(vx, vy), heading: vy.atan2(vx) }
    }

    #[test]
    fn seek_points_toward_target_and_flee_away() {
        let a = at(0.0, 0.0);
        let s = seek(&a, Vec2::new(10.0, 0.0), LIMITS);
        assert!(s.x > 0.0 && s.y.abs() < 1e-6);
        assert!((s.length() - 1.0).abs() < 1e-6, "truncated to max_accel");
        let f = flee(&a, Vec2::new(10.0, 0.0), LIMITS);
        assert!(f.x < 0.0);
        // A moving agent seeking a target behind it steers backwards.
        let m = moving(0.0, 0.0, 2.0, 0.0);
        assert!(seek(&m, Vec2::new(-5.0, 0.0), LIMITS).x < 0.0);
    }

    #[test]
    fn arrive_slows_down_inside_the_slowing_radius() {
        // Moving at max speed towards a target 1 unit away with a radius of 4:
        // desired speed is 0.5, so the agent must brake.
        let near = moving(0.0, 0.0, 2.0, 0.0);
        let a = arrive(&near, Vec2::new(1.0, 0.0), 4.0, MotionLimits::new(2.0, 10.0));
        assert!((a - Vec2::new(-1.5, 0.0)).length() < 1e-5, "{a}");
        // Far away it behaves like seek.
        let far = at(0.0, 0.0);
        assert_eq!(arrive(&far, Vec2::new(10.0, 0.0), 4.0, LIMITS), seek(&far, Vec2::new(10.0, 0.0), LIMITS));
        // On the target it cancels the velocity.
        let on = moving(1.0, 1.0, 0.5, 0.0);
        assert_eq!(arrive(&on, Vec2::new(1.0, 1.0), 4.0, LIMITS), Vec2::new(-0.5, 0.0));
    }

    #[test]
    fn arrive_converges_on_the_target() {
        let mut k = at(0.0, 0.0);
        let target = Vec2::new(10.0, 5.0);
        for _ in 0..400 {
            let a = arrive(&k, target, 3.0, LIMITS);
            k.integrate(a, 0.1, LIMITS, crate::kinematics::Bounds::Unbounded);
        }
        assert!(k.position.distance(target) < 0.05, "{}", k.position);
        assert!(k.speed() < 0.05);
    }

    #[test]
    fn pursue_leads_the_target_and_evade_avoids_the_prediction() {
        let hunter = moving(0.0, 0.0, 1.0, 0.0);
        let prey = moving(10.0, 0.0, 0.0, 1.0);
        let p = pursue(&hunter, &prey, 2.0, MotionLimits::new(2.0, 10.0));
        assert!(p.y > 0.0, "aims ahead of the prey's motion: {p}");
        let e = evade(&prey, &hunter, 2.0, MotionLimits::new(2.0, 10.0));
        assert!(e.x > 0.0, "moves away from the hunter: {e}");
        // A stationary quarry is pursued exactly like seek.
        let still = at(5.0, 5.0);
        assert_eq!(pursue(&hunter, &still, 2.0, LIMITS), seek(&hunter, still.position, LIMITS));
    }

    #[test]
    fn pursue_and_evade_use_the_documented_prediction_horizon() {
        let limits = MotionLimits::new(2.0, 10.0);
        let hunter = moving(0.0, 0.0, 1.0, 0.0);
        let prey = moving(10.0, 0.0, 0.0, 1.0);
        // d = 10, speed 1, T_max = 2: d ≥ speed·T_max, so T = 2 and the aim is (10, 2).
        assert_eq!(pursue(&hunter, &prey, 2.0, limits), seek(&hunter, Vec2::new(10.0, 2.0), limits));
        // T_max = 20: T = d / speed = 10, aim (10, 10).
        assert_eq!(pursue(&hunter, &prey, 20.0, limits), seek(&hunter, Vec2::new(10.0, 10.0), limits));
        // A pursuer standing still uses T_max.
        let still = at(0.0, 0.0);
        assert_eq!(pursue(&still, &prey, 3.0, limits), seek(&still, Vec2::new(10.0, 3.0), limits));
        // Evade flees the hunter's predicted position (0, 0) + (1, 0)·2.
        assert_eq!(evade(&prey, &hunter, 2.0, limits), flee(&prey, Vec2::new(2.0, 0.0), limits));
        // Non-finite or negative horizons predict nothing.
        assert_eq!(pursue(&hunter, &prey, f32::NAN, limits), seek(&hunter, prey.position, limits));
        assert_eq!(pursue(&hunter, &prey, -1.0, limits), seek(&hunter, prey.position, limits));
    }

    #[test]
    fn unlimited_agents_steer_at_full_acceleration() {
        let free = MotionLimits::new(f32::INFINITY, 1.0);
        let me = at(0.0, 0.0);
        let close = |a: Vec2, b: Vec2| (a - b).length() < 1e-6;
        assert!(close(seek(&me, Vec2::new(5.0, 0.0), free), Vec2::X));
        assert!(close(flee(&me, Vec2::new(0.0, 5.0), free), Vec2::NEG_Y));
        assert!(close(arrive(&me, Vec2::new(5.0, 0.0), 10.0, free), Vec2::X), "inside the slowing radius");
        assert!(close(arrive(&me, Vec2::new(50.0, 0.0), 10.0, free), Vec2::X));
        assert!(close(separation(&me, &[at(1.0, 0.0)], 3.0, free), Vec2::NEG_X));
        // Unlimited acceleration: obstacle avoidance still produces a finite push.
        let wall = |p: Vec2| p.x < 5.0;
        let a = avoid_obstacles(
            &moving(4.0, 0.0, 1.0, 0.0),
            &Feelers::default(),
            MotionLimits::new(2.0, f32::INFINITY),
            wall,
        );
        assert!(a.is_finite() && a.x < 0.0, "{a}");
        let bounded = avoid_obstacles(&moving(4.0, 0.0, 1.0, 0.0), &Feelers::default(), LIMITS, wall);
        // LIMITS has max_accel 1, so `bounded` is the unit-budget push that `a` scales by f32::MAX.
        assert!((a / f32::MAX - bounded).length() < 1e-6, "same push as with a finite limit: {bounded}");
    }

    #[test]
    fn negative_or_nan_radii_select_no_neighbours() {
        let me = at(0.0, 0.0);
        let flock = [moving(1.0, 0.0, 0.0, 1.0), moving(0.0, 1.0, 0.0, 1.0)];
        for radius in [-3.0, f32::NAN] {
            assert_eq!(separation(&me, &flock, radius, LIMITS), Vec2::ZERO);
            assert_eq!(alignment(&me, &flock, radius, LIMITS), Vec2::ZERO);
            assert_eq!(cohesion(&me, &flock, radius, LIMITS), Vec2::ZERO);
        }
    }

    #[test]
    fn wander_is_deterministic_and_keeps_the_angle_wrapped() {
        let k = moving(0.0, 0.0, 1.0, 0.0);
        let params = WanderParams { distance: 2.0, radius: 1.0, jitter: 3.0 };
        let run = |seed| {
            let mut rng = SimRng::seed_from_u64(seed);
            let mut state = WanderState::default();
            (0..50).map(|_| wander(&k, &mut state, &params, &mut rng, LIMITS)).collect::<Vec<_>>()
        };
        assert_eq!(run(1), run(1));
        assert_ne!(run(1), run(2));
        let mut rng = SimRng::seed_from_u64(3);
        let mut state = WanderState::default();
        for _ in 0..1000 {
            let a = wander(&k, &mut state, &params, &mut rng, LIMITS);
            assert!(a.length() <= 1.0 + 1e-5);
            assert!(state.angle > -std::f32::consts::PI && state.angle <= std::f32::consts::PI);
        }
        assert_eq!(wrap_angle(-std::f32::consts::PI), std::f32::consts::PI);
        assert_eq!(wrap_angle(f32::NAN), 0.0);
    }

    #[test]
    fn boids_separation_pushes_apart() {
        let me = at(0.0, 0.0);
        let others = [at(1.0, 0.0), at(0.0, 0.0), at(50.0, 0.0)];
        let s = separation(&me, &others, 3.0, LIMITS);
        assert!(s.x < 0.0 && s.y.abs() < 1e-6, "{s}");
        // The closer neighbour dominates.
        let pair = [at(1.0, 0.0), at(0.0, -2.5)];
        let s = separation(&me, &pair, 3.0, LIMITS);
        assert!(s.x < 0.0 && s.y > 0.0 && s.x.abs() > s.y.abs(), "{s}");
        assert_eq!(separation(&me, &[at(10.0, 0.0)], 3.0, LIMITS), Vec2::ZERO);
    }

    #[test]
    fn boids_alignment_and_cohesion() {
        let me = moving(0.0, 0.0, 1.0, 0.0);
        let flock = [moving(1.0, 1.0, 0.0, 2.0), moving(-1.0, 1.0, 0.0, 2.0)];
        let a = alignment(&me, &flock, 5.0, LIMITS);
        assert!(a.y > 0.0 && a.x < 0.0, "turns towards +y: {a}");
        let c = cohesion(&me, &flock, 5.0, LIMITS);
        assert!(c.y > 0.0, "towards the centroid (0, 1): {c}");
        assert_eq!(cohesion(&me, &flock, 0.5, LIMITS), Vec2::ZERO);
        assert_eq!(alignment(&me, &[moving(1.0, 0.0, 0.0, 0.0)], 5.0, LIMITS), Vec2::ZERO);
    }

    #[test]
    fn avoid_obstacles_turns_and_brakes_before_walls() {
        let passable = |p: Vec2| p.x < 5.0;
        let heading_to_wall = moving(3.0, 0.0, 1.0, 0.0);
        let a = avoid_obstacles(&heading_to_wall, &Feelers::default(), LIMITS, passable);
        assert!(a.x < 0.0, "brakes: {a}");
        assert!(a.y != 0.0, "turns: {a}");
        assert!(a.length() <= 1.0 + 1e-5);
        // A wall on the positive-angle side (y > 1) makes the agent turn to -y.
        let corridor = |p: Vec2| p.y < 1.0;
        let a = avoid_obstacles(&moving(0.0, 0.0, 1.0, 0.0), &Feelers::default(), LIMITS, corridor);
        assert!(a.y < 0.0, "{a}");
        let open = avoid_obstacles(&heading_to_wall, &Feelers::default(), LIMITS, |_| true);
        assert_eq!(open, Vec2::ZERO);
        // Urgency grows as the wall gets closer.
        let far = avoid_obstacles(&moving(2.2, 0.0, 1.0, 0.0), &Feelers::default(), LIMITS, passable);
        let near = avoid_obstacles(&moving(4.6, 0.0, 1.0, 0.0), &Feelers::default(), LIMITS, passable);
        assert!(near.length() > far.length(), "{near} vs {far}");
    }

    #[test]
    fn prioritized_combination_truncates_by_priority() {
        let first = (Vec2::new(0.0, 1.0), 1.0);
        let second = (Vec2::new(1.0, 0.0), 1.0);
        assert_eq!(combine_prioritized(&[first, second], 1.0), Vec2::new(0.0, 1.0), "budget spent by first");
        let c = combine_prioritized(&[(Vec2::new(0.0, 0.5), 1.0), second], 1.0);
        assert!((c - Vec2::new(0.5, 0.5)).length() < 1e-6, "second fills the remaining budget: {c}");
        let w = combine_weighted(&[(Vec2::new(1.0, 0.0), 2.0), (Vec2::new(-1.0, 0.0), 1.0)], 10.0);
        assert_eq!(w, Vec2::new(1.0, 0.0));
        assert_eq!(combine_weighted(&[(Vec2::new(f32::NAN, 0.0), 1.0)], 1.0), Vec2::ZERO);
    }
}
