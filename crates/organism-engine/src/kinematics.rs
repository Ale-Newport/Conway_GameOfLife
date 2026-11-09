//! Point-mass kinematics for agents in continuous 2D space.
//!
//! Positions are `glam::Vec2` in world units (one unit = one terrain cell, `y`
//! growing downward, see `ARCHITECTURE.md` §3.3). Headings are angles in
//! radians measured from `+x` towards `+y` (`atan2(v.y, v.x)`), which appears
//! clockwise on screen because `y` points down.
//!
//! [`Kinematics::integrate`] uses semi-implicit (symplectic) Euler:
//!
//! ```text
//! a  = truncate(a_desired, max_accel)
//! v' = truncate(v + a·dt, max_speed)
//! p' = bounds(p + v'·dt)
//! ```
//!
//! with an optional passability test that lets agents slide along walls.

use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

/// Speeds at or below this value are treated as standing still when deriving
/// a heading from the velocity.
pub const MIN_HEADING_SPEED: f32 = 1e-6;

/// Unit vector pointing along `heading` (radians). Non-finite headings map to `+x`.
pub fn heading_vector(heading: f32) -> Vec2 {
    if heading.is_finite() { Vec2::new(heading.cos(), heading.sin()) } else { Vec2::X }
}

/// Limits `v` to length `max`. Returns zero when `v` is not finite or `max` is
/// not a positive number, so a degenerate steering result can never inject
/// NaN into an agent's state. Vectors whose squared length overflows `f32`
/// keep their direction (they are rescaled before normalising).
pub fn truncate(v: Vec2, max: f32) -> Vec2 {
    if !v.is_finite() || max.is_nan() || max <= 0.0 {
        return Vec2::ZERO;
    }
    // `max * max` is infinite for huge `max`, which correctly accepts every finite `v`.
    if v.length_squared() <= max * max {
        return v;
    }
    // Divide by the largest component first so the length cannot overflow.
    let scaled = v / v.abs().max_element();
    scaled.normalize() * max
}

/// Speed and acceleration limits of an agent. Negative or NaN limits act as
/// zero (the agent cannot move or steer); infinite limits mean "unlimited".
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionLimits {
    /// Maximum speed in world units per time unit.
    pub max_speed: f32,
    /// Maximum acceleration (steering force per unit mass).
    pub max_accel: f32,
}

impl MotionLimits {
    /// Limits with negative or NaN values replaced by zero (infinity is allowed).
    pub fn new(max_speed: f32, max_accel: f32) -> Self {
        let clean = |x: f32| if x.is_nan() || x < 0.0 { 0.0 } else { x };
        Self { max_speed: clean(max_speed), max_accel: clean(max_accel) }
    }

    /// Speed used to build desired velocities in steering: `max_speed` with
    /// NaN/negative mapped to 0 and infinity to `f32::MAX`, so that
    /// `direction · speed` stays finite (`0 · ∞` would be NaN) and an
    /// unlimited agent steers at full acceleration instead of not at all.
    pub fn desired_speed(&self) -> f32 {
        if self.max_speed.is_nan() { 0.0 } else { self.max_speed.clamp(0.0, f32::MAX) }
    }
}

/// How the world edges constrain movement. Rectangles are given by their
/// corners; an axis whose bounds are not finite or not ordered (`max < min`,
/// or `max <= min` for wrapping) is left unconstrained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bounds {
    /// No edges.
    #[default]
    Unbounded,
    /// Positions are clamped into `[min, max]` (inclusive) and the velocity
    /// component pointing out of the rectangle is removed.
    Clamp {
        /// Lower corner.
        min: Vec2,
        /// Upper corner.
        max: Vec2,
    },
    /// Positions are mirrored back at the edges and the outward velocity
    /// component is reversed (elastic bounce).
    Reflect {
        /// Lower corner.
        min: Vec2,
        /// Upper corner.
        max: Vec2,
    },
    /// Toroidal world: positions wrap into `[min, max)`.
    Wrap {
        /// Lower corner.
        min: Vec2,
        /// Upper corner (exclusive).
        max: Vec2,
    },
}

impl Bounds {
    /// Constrains a proposed `(position, velocity)` pair.
    pub fn apply(self, position: Vec2, velocity: Vec2) -> (Vec2, Vec2) {
        let per_axis = |min: Vec2, max: Vec2, f: fn(f32, f32, f32, f32) -> (f32, f32)| {
            let (px, vx) = f(position.x, velocity.x, min.x, max.x);
            let (py, vy) = f(position.y, velocity.y, min.y, max.y);
            (Vec2::new(px, py), Vec2::new(vx, vy))
        };
        match self {
            Self::Unbounded => (position, velocity),
            Self::Clamp { min, max } => per_axis(min, max, clamp_axis),
            Self::Reflect { min, max } => per_axis(min, max, reflect_axis),
            Self::Wrap { min, max } => per_axis(min, max, |p, v, lo, hi| (wrap_axis(p, lo, hi), v)),
        }
    }

    /// Shortest displacement from `from` to `to`, taking wrapping into account.
    /// Steering towards a target in a toroidal world should use
    /// `from + bounds.delta(from, to)` as the target position.
    pub fn delta(self, from: Vec2, to: Vec2) -> Vec2 {
        let d = to - from;
        match self {
            Self::Wrap { min, max } => {
                let axis = |d: f32, lo: f32, hi: f32| {
                    let w = hi - lo;
                    if wrap_axis_is_valid(lo, hi) { d - w * (d / w).round() } else { d }
                };
                Vec2::new(axis(d.x, min.x, max.x), axis(d.y, min.y, max.y))
            }
            _ => d,
        }
    }

    /// Whether `p` lies inside the bounds (always true when unbounded). An
    /// axis that [`Bounds::apply`] leaves unconstrained accepts every value,
    /// so `contains(apply(p, v).0)` holds for every finite `p`.
    pub fn contains(self, p: Vec2) -> bool {
        match self {
            Self::Unbounded => true,
            Self::Clamp { min, max } | Self::Reflect { min, max } => {
                let inside = |p: f32, lo: f32, hi: f32| !axis_is_valid(lo, hi) || (lo..=hi).contains(&p);
                inside(p.x, min.x, max.x) && inside(p.y, min.y, max.y)
            }
            Self::Wrap { min, max } => {
                let inside = |p: f32, lo: f32, hi: f32| !wrap_axis_is_valid(lo, hi) || (lo..hi).contains(&p);
                inside(p.x, min.x, max.x) && inside(p.y, min.y, max.y)
            }
        }
    }
}

/// Clamp/reflect axes need finite, ordered bounds (`hi == lo` pins the axis).
fn axis_is_valid(lo: f32, hi: f32) -> bool {
    lo.is_finite() && hi.is_finite() && hi >= lo
}

/// Wrapping axes need a finite, positive period.
pub(crate) fn wrap_axis_is_valid(lo: f32, hi: f32) -> bool {
    lo.is_finite() && hi.is_finite() && hi > lo
}

fn clamp_axis(p: f32, v: f32, lo: f32, hi: f32) -> (f32, f32) {
    if !axis_is_valid(lo, hi) {
        (p, v)
    } else if p < lo {
        (lo, v.max(0.0))
    } else if p > hi {
        (hi, v.min(0.0))
    } else {
        (p, v)
    }
}

fn reflect_axis(p: f32, v: f32, lo: f32, hi: f32) -> (f32, f32) {
    if !axis_is_valid(lo, hi) {
        (p, v)
    } else if p < lo {
        ((2.0 * lo - p).min(hi), v.abs())
    } else if p > hi {
        ((2.0 * hi - p).max(lo), -v.abs())
    } else {
        (p, v)
    }
}

fn wrap_axis(p: f32, lo: f32, hi: f32) -> f32 {
    if !wrap_axis_is_valid(lo, hi) {
        return p;
    }
    let w = hi - lo;
    let r = (p - lo).rem_euclid(w);
    // `rem_euclid` can round up to exactly `w` for tiny negative inputs, and
    // `lo + r` can round up to `hi`. Both mean "just below `hi`", which is the
    // same point of the torus as `lo`.
    let q = lo + r;
    if r >= w || q >= hi { lo } else { q }
}

/// Outcome of one integration step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Movement {
    /// The full step was accepted.
    Moved,
    /// The full step was impassable; the agent moved along one axis only and
    /// the velocity component of the blocked axis was removed.
    Slid,
    /// No passable move existed; position unchanged, velocity set to zero.
    Blocked,
    /// `dt` was not a positive finite number; nothing changed.
    Idle,
}

/// Position, velocity and facing of a point-mass agent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Kinematics {
    /// Position in world units.
    pub position: Vec2,
    /// Velocity in world units per time unit.
    pub velocity: Vec2,
    /// Facing angle in radians (`atan2(y, x)` convention). Updated from the
    /// velocity whenever the agent moves faster than [`MIN_HEADING_SPEED`].
    pub heading: f32,
}

impl Kinematics {
    /// Agent at rest at `position`, facing `heading`.
    pub fn new(position: Vec2, heading: f32) -> Self {
        Self { position, velocity: Vec2::ZERO, heading }
    }

    /// Current speed.
    pub fn speed(&self) -> f32 {
        self.velocity.length()
    }

    /// Unit direction of travel: the velocity direction while moving,
    /// otherwise the heading.
    pub fn direction(&self) -> Vec2 {
        let speed = self.velocity.length();
        if speed.is_finite() && speed > MIN_HEADING_SPEED {
            self.velocity / speed
        } else {
            heading_vector(self.heading)
        }
    }

    /// Advances one step of length `dt` under `acceleration` (see the module
    /// documentation for the scheme) without obstacles.
    pub fn integrate(&mut self, acceleration: Vec2, dt: f32, limits: MotionLimits, bounds: Bounds) -> Movement {
        self.integrate_with(acceleration, dt, limits, bounds, |_| true)
    }

    /// Like [`Kinematics::integrate`] but only accepts destinations for which
    /// `is_passable` returns `true` (water for land animals, walls, steep
    /// slopes...). When the full move is blocked the agent tries to slide
    /// along the axis with the larger displacement first, then the other axis;
    /// if both are blocked it stops. Only destinations are tested, so an agent
    /// placed inside an obstacle can still walk out of it.
    pub fn integrate_with(
        &mut self,
        acceleration: Vec2,
        dt: f32,
        limits: MotionLimits,
        bounds: Bounds,
        mut is_passable: impl FnMut(Vec2) -> bool,
    ) -> Movement {
        if !(dt.is_finite() && dt > 0.0) {
            return Movement::Idle;
        }
        let accel = truncate(acceleration, limits.max_accel);
        let mut velocity = truncate(self.velocity + accel * dt, limits.max_speed);
        let displacement = velocity * dt;
        let (full_position, full_velocity) = bounds.apply(self.position + displacement, velocity);
        let outcome = if is_passable(full_position) {
            self.position = full_position;
            velocity = full_velocity;
            Movement::Moved
        } else {
            let along_x = (Vec2::new(displacement.x, 0.0), Vec2::new(velocity.x, 0.0));
            let along_y = (Vec2::new(0.0, displacement.y), Vec2::new(0.0, velocity.y));
            let order =
                if displacement.x.abs() >= displacement.y.abs() { [along_x, along_y] } else { [along_y, along_x] };
            let slide = order.into_iter().filter(|(d, _)| *d != Vec2::ZERO).find_map(|(d, v)| {
                let (p, v) = bounds.apply(self.position + d, v);
                is_passable(p).then_some((p, v))
            });
            match slide {
                Some((p, v)) => {
                    self.position = p;
                    velocity = v;
                    Movement::Slid
                }
                None => {
                    velocity = Vec2::ZERO;
                    Movement::Blocked
                }
            }
        };
        self.velocity = velocity;
        if velocity.length_squared() > MIN_HEADING_SPEED * MIN_HEADING_SPEED {
            self.heading = velocity.y.atan2(velocity.x);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: MotionLimits = MotionLimits { max_speed: 2.0, max_accel: 1.0 };

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn semi_implicit_euler_with_limits() {
        let mut k = Kinematics::new(Vec2::ZERO, 0.0);
        // Acceleration is truncated to 1, so after 1 s velocity is (1, 0).
        assert_eq!(k.integrate(Vec2::new(10.0, 0.0), 1.0, LIMITS, Bounds::Unbounded), Movement::Moved);
        assert!(close(k.velocity, Vec2::new(1.0, 0.0)));
        assert!(close(k.position, Vec2::new(1.0, 0.0)), "position uses the updated velocity");
        for _ in 0..10 {
            k.integrate(Vec2::new(10.0, 0.0), 1.0, LIMITS, Bounds::Unbounded);
        }
        assert!((k.speed() - 2.0).abs() < 1e-6, "speed is capped at max_speed");
        k.integrate(Vec2::new(0.0, 1.0), 0.5, LIMITS, Bounds::Unbounded);
        assert!(k.heading > 0.0 && k.heading < std::f32::consts::FRAC_PI_2, "heading follows the velocity");
    }

    #[test]
    fn invalid_inputs_never_produce_nan() {
        let mut k = Kinematics::new(Vec2::new(1.0, 1.0), 0.0);
        assert_eq!(k.integrate(Vec2::ONE, f32::NAN, LIMITS, Bounds::Unbounded), Movement::Idle);
        assert_eq!(k.integrate(Vec2::ONE, -1.0, LIMITS, Bounds::Unbounded), Movement::Idle);
        k.integrate(Vec2::new(f32::NAN, 1.0), 1.0, LIMITS, Bounds::Unbounded);
        assert!(k.position.is_finite() && k.velocity.is_finite());
        assert_eq!(k.position, Vec2::new(1.0, 1.0));
    }

    #[test]
    fn bounds_clamp_reflect_and_wrap() {
        let min = Vec2::ZERO;
        let max = Vec2::new(10.0, 10.0);
        let (p, v) = Bounds::Clamp { min, max }.apply(Vec2::new(11.0, 5.0), Vec2::new(2.0, 1.0));
        assert_eq!((p, v), (Vec2::new(10.0, 5.0), Vec2::new(0.0, 1.0)));
        let (p, v) = Bounds::Reflect { min, max }.apply(Vec2::new(11.0, -1.0), Vec2::new(2.0, -1.0));
        assert_eq!((p, v), (Vec2::new(9.0, 1.0), Vec2::new(-2.0, 1.0)));
        let (p, v) = Bounds::Wrap { min, max }.apply(Vec2::new(11.0, -1.0), Vec2::new(2.0, -1.0));
        assert_eq!((p, v), (Vec2::new(1.0, 9.0), Vec2::new(2.0, -1.0)));
        let wrap = Bounds::Wrap { min, max };
        assert_eq!(wrap.delta(Vec2::new(9.0, 1.0), Vec2::new(1.0, 9.0)), Vec2::new(2.0, -2.0));
        assert!(wrap.contains(Vec2::new(0.0, 9.99)) && !wrap.contains(Vec2::new(10.0, 1.0)));
        assert!(Bounds::Clamp { min, max }.contains(Vec2::new(10.0, 10.0)));
        // Inverted rectangles leave the axis unconstrained instead of panicking.
        let broken = Bounds::Clamp { min: max, max: min };
        assert_eq!(broken.apply(Vec2::new(20.0, -3.0), Vec2::ONE).0, Vec2::new(20.0, -3.0));
    }

    #[test]
    fn contains_accepts_everything_apply_produces() {
        let mut rng = simulation_core::SimRng::seed_from_u64(6);
        let rects = [
            (Vec2::ZERO, Vec2::new(10.0, 4.0)),
            (Vec2::ZERO, Vec2::new(10.0, 0.0)),         // degenerate y axis
            (Vec2::new(5.0, 5.0), Vec2::new(1.0, 9.0)), // inverted x axis
            (Vec2::new(f32::NEG_INFINITY, 0.0), Vec2::new(0.0, 3.0)),
        ];
        for (min, max) in rects {
            for bounds in [Bounds::Clamp { min, max }, Bounds::Reflect { min, max }, Bounds::Wrap { min, max }] {
                let edges = [min.x, min.y, max.x, max.y].map(|e| if e.is_finite() { e } else { 0.0 });
                let near_edges = edges.into_iter().flat_map(|e| [e.next_down(), e, e.next_up()]);
                let samples = (0..200)
                    .map(|_| Vec2::new(rng.range_f32(-30.0, 30.0), rng.range_f32(-30.0, 30.0)))
                    .chain(near_edges.clone().flat_map(|x| near_edges.clone().map(move |y| Vec2::new(x, y))))
                    .collect::<Vec<_>>();
                for p in samples {
                    let (q, _) = bounds.apply(p, Vec2::ONE);
                    assert!(bounds.contains(q), "{bounds:?}: {p} -> {q}");
                }
            }
        }
        // Rounding regression: just below `min` on an axis with `min != 0`.
        let wrap = Bounds::Wrap { min: Vec2::splat(5.0), max: Vec2::splat(9.0) };
        let (q, _) = wrap.apply(Vec2::splat(5.0f32.next_down()), Vec2::ZERO);
        assert!(wrap.contains(q), "{q}");
    }

    #[test]
    fn truncate_keeps_the_direction_of_huge_vectors() {
        let huge = Vec2::new(3.0e38, -3.0e38);
        let t = truncate(huge, 2.0);
        assert!((t - Vec2::new(1.0, -1.0).normalize() * 2.0).length() < 1e-6, "{t}");
        assert_eq!(truncate(Vec2::new(f32::MAX, 0.0), 1.0), Vec2::X);
        assert_eq!(truncate(Vec2::new(3.0, 4.0), f32::INFINITY), Vec2::new(3.0, 4.0));
        assert_eq!(truncate(Vec2::new(3.0, 4.0), 2.5), Vec2::new(1.5, 2.0));
        assert_eq!(MotionLimits::new(f32::INFINITY, 1.0).desired_speed(), f32::MAX);
        assert_eq!(MotionLimits { max_speed: f32::NAN, max_accel: 1.0 }.desired_speed(), 0.0);
    }

    #[test]
    fn passability_hook_slides_along_walls_and_blocks() {
        // Wall: x >= 5 is impassable.
        let passable = |p: Vec2| p.x < 5.0;
        let mut k = Kinematics { position: Vec2::new(4.5, 0.0), velocity: Vec2::new(1.0, 0.5), heading: 0.0 };
        let limits = MotionLimits::new(2.0, 0.0);
        assert_eq!(k.integrate_with(Vec2::ZERO, 1.0, limits, Bounds::Unbounded, passable), Movement::Slid);
        assert_eq!(k.position, Vec2::new(4.5, 0.5), "slides along y");
        assert_eq!(k.velocity, Vec2::new(0.0, 0.5), "blocked axis velocity removed");

        let mut head_on = Kinematics { position: Vec2::new(4.5, 0.0), velocity: Vec2::new(1.0, 0.0), heading: 0.0 };
        assert_eq!(head_on.integrate_with(Vec2::ZERO, 1.0, limits, Bounds::Unbounded, passable), Movement::Blocked);
        assert_eq!(head_on.position, Vec2::new(4.5, 0.0));
        assert_eq!(head_on.velocity, Vec2::ZERO);
        assert_eq!(head_on.heading, 0.0, "heading kept when stopped");
    }

    #[test]
    fn serde_round_trip() {
        let k = Kinematics { position: Vec2::new(1.5, -2.0), velocity: Vec2::new(0.25, 3.0), heading: 1.25 };
        let json = serde_json::to_string(&k).unwrap();
        assert_eq!(serde_json::from_str::<Kinematics>(&json).unwrap(), k);
        let b = Bounds::Wrap { min: Vec2::ZERO, max: Vec2::splat(64.0) };
        assert_eq!(serde_json::from_str::<Bounds>(&serde_json::to_string(&b).unwrap()).unwrap(), b);
    }
}
