//! Perception queries on top of [`simulation_core::SpatialGrid`].
//!
//! The grid stores `u32` *slots*. The convention used throughout Emergence Lab
//! is that a slot is the agent's index in its [`AgentStore`] (see
//! [`index_agents`]); because the store is sorted by [`crate::id::AgentId`],
//! ordering by slot is the same as ordering by id, so "ties broken by the
//! lower slot" means "ties broken by the older agent".
//!
//! A [`Query`] combines a radius with an optional [`FieldOfView`] cone.
//!
//! **Toroidal worlds.** [`SpatialGrid`] itself does not wrap. A query placed
//! in a world with [`Bounds::Wrap`] (see [`Query::in_world`]) sees every item
//! at its *nearest periodic image*: the distance is the shortest wrapped
//! distance and [`Percept::position`] is that image in the observer's frame
//! (`origin + bounds.delta(origin, p)`), so it can be fed directly to the
//! steering behaviours. Each item is reported at most once. Indexed positions
//! must lie inside the wrapped rectangle (as produced by
//! [`Bounds::apply`]) and the rectangle should match the grid's world.

use std::cmp::Ordering;

use simulation_core::SpatialGrid;
use simulation_core::glam::Vec2;

use crate::error::OrganismError;
use crate::kinematics::{Bounds, wrap_axis_is_valid};
use crate::store::AgentStore;

/// Upper bound on the number of samples [`line_of_sight`] takes.
pub const MAX_LINE_OF_SIGHT_SAMPLES: u32 = 1024;

/// Vision cone: points whose direction from the observer lies within half the
/// aperture of `direction` are visible.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldOfView {
    direction: Vec2,
    cos_half_angle: f32,
}

impl FieldOfView {
    /// Cone around `direction` with full aperture `angle` radians (e.g. `π`
    /// for a half-plane, `1.5π` for typical boids). An aperture of `2π` or
    /// more, a NaN aperture or a zero direction sees everything.
    pub fn new(direction: Vec2, angle: f32) -> Self {
        match direction.try_normalize() {
            Some(direction) if angle < std::f32::consts::TAU => {
                Self { direction, cos_half_angle: (angle.max(0.0) * 0.5).cos() }
            }
            // `dot` of unit vectors is >= -1, so -2 accepts every direction.
            _ => Self { direction: Vec2::X, cos_half_angle: -2.0 },
        }
    }

    /// Whether a point at `offset` from the observer lies inside the cone.
    /// The observer's own position (`offset == 0`) is always inside.
    pub fn contains(&self, offset: Vec2) -> bool {
        match offset.try_normalize() {
            Some(dir) => dir.dot(self.direction) >= self.cos_half_angle,
            None => true,
        }
    }
}

/// A perception query: everything within `radius` of `origin`, optionally
/// restricted to a field of view, in a world whose edges may wrap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Query {
    /// Observer position.
    pub origin: Vec2,
    /// Perception radius (inclusive). Negative or NaN radii see nothing.
    pub radius: f32,
    /// Optional vision cone.
    pub fov: Option<FieldOfView>,
    /// World edges. Only [`Bounds::Wrap`] changes the query (periodic
    /// images, see the module documentation); other variants have no images.
    pub world: Bounds,
}

impl Query {
    /// Omnidirectional query in an unbounded (non-wrapping) world.
    pub fn new(origin: Vec2, radius: f32) -> Self {
        Self { origin, radius, fov: None, world: Bounds::Unbounded }
    }

    /// Restricts the query to a cone; see [`FieldOfView::new`].
    pub fn with_fov(mut self, direction: Vec2, angle: f32) -> Self {
        self.fov = Some(FieldOfView::new(direction, angle));
        self
    }

    /// Places the query in a world with the given edges; with
    /// [`Bounds::Wrap`] items are seen across the edges.
    pub fn in_world(mut self, world: Bounds) -> Self {
        self.world = world;
        self
    }

    /// Whether `point` satisfies the query (radius and cone, measured to its
    /// nearest periodic image in a wrapped world). Like the grid queries, a
    /// negative or NaN radius accepts nothing.
    pub fn accepts(&self, point: Vec2) -> bool {
        self.radius >= 0.0 && self.accepts_offset(self.world.delta(self.origin, point))
    }

    fn accepts_offset(&self, offset: Vec2) -> bool {
        offset.length_squared() <= self.radius * self.radius && self.fov.is_none_or(|f| f.contains(offset))
    }

    /// Calls `f(slot, position, distance²)` once per accepted item, where
    /// `position` is the (nearest image of the) item.
    fn for_each(&self, grid: &SpatialGrid, mut f: impl FnMut(u32, Vec2, f32)) {
        let Bounds::Wrap { min, max } = self.world else {
            grid.for_each_within(self.origin, self.radius, |slot, p, d2| {
                if self.fov.is_none_or(|fov| fov.contains(p - self.origin)) {
                    f(slot, p, d2);
                }
            });
            return;
        };
        if self.radius.is_nan() || self.radius < 0.0 {
            return;
        }
        let wraps = (wrap_axis_is_valid(min.x, max.x), wrap_axis_is_valid(min.y, max.y));
        let period = max - min;
        let mut emit = |slot: u32, offset: Vec2| {
            if self.accepts_offset(offset) {
                f(slot, self.origin + offset, offset.length_squared());
            }
        };
        // With a diameter clearly below the period, the query circles around
        // the periodic images of the origin are disjoint, so every item is
        // found from at most one image (the 1 % margin absorbs rounding).
        let small = |wraps: bool, period: f32| !wraps || 2.0 * self.radius < 0.99 * period;
        if !(small(wraps.0, period.x) && small(wraps.1, period.y)) {
            // Large radius: visit every item once at its nearest image.
            grid.for_each_within(self.origin, f32::INFINITY, |slot, p, _| emit(slot, self.world.delta(self.origin, p)));
            return;
        }
        let base = self.world.apply(self.origin, Vec2::ZERO).0;
        let shifts = |wraps: bool, o: f32, lo: f32, hi: f32| {
            // +period finds images of items near `hi` (seen below `lo`), and
            // −period images of items near `lo` (seen above `hi`).
            let below = wraps && o - self.radius < lo;
            let above = wraps && o + self.radius >= hi;
            [Some(0.0), below.then_some(hi - lo), above.then_some(lo - hi)].into_iter().flatten()
        };
        for sy in shifts(wraps.1, base.y, min.y, max.y) {
            for sx in shifts(wraps.0, base.x, min.x, max.x) {
                let shift = Vec2::new(sx, sy);
                let centre = base + shift;
                grid.for_each_within(centre, self.radius, |slot, p, _| emit(slot, p - centre));
            }
        }
    }
}

/// Something perceived by a query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Percept {
    /// Slot of the item in the spatial grid (store index by convention).
    pub slot: u32,
    /// Its position; in a wrapped world, its image nearest to the observer.
    pub position: Vec2,
    /// Euclidean distance from the query origin.
    pub distance: f32,
}

/// Orders percepts by distance, then slot: the single ranking used by
/// [`nearest`] and [`within`], so the nearest item is always the first one
/// listed.
fn closer(a: &Percept, b: &Percept) -> Ordering {
    a.distance.total_cmp(&b.distance).then(a.slot.cmp(&b.slot))
}

/// Nearest item accepted by the query and by `filter(slot, position)`; ties at
/// equal distance go to the lower slot. `filter` is only called for
/// candidates that would improve the current best.
pub fn nearest(grid: &SpatialGrid, query: &Query, mut filter: impl FnMut(u32, Vec2) -> bool) -> Option<Percept> {
    let mut best: Option<Percept> = None;
    query.for_each(grid, |slot, position, d2| {
        let candidate = Percept { slot, position, distance: d2.sqrt() };
        let improves = best.is_none_or(|b| closer(&candidate, &b) == Ordering::Less);
        if improves && filter(slot, position) {
            best = Some(candidate);
        }
    });
    best
}

/// Number of items accepted by the query and by `filter`.
pub fn count(grid: &SpatialGrid, query: &Query, mut filter: impl FnMut(u32, Vec2) -> bool) -> usize {
    let mut n = 0;
    query.for_each(grid, |slot, p, _| {
        if filter(slot, p) {
            n += 1;
        }
    });
    n
}

/// Collects every item accepted by the query and by `filter` into `out`
/// (cleared first), sorted by distance and then by slot.
pub fn within(grid: &SpatialGrid, query: &Query, mut filter: impl FnMut(u32, Vec2) -> bool, out: &mut Vec<Percept>) {
    out.clear();
    query.for_each(grid, |slot, position, d2| {
        if filter(slot, position) {
            out.push(Percept { slot, position, distance: d2.sqrt() });
        }
    });
    // Slots are unique, so the unstable sort is fully determined.
    out.sort_unstable_by(closer);
}

/// Line-of-sight test by sampling: checks `is_clear` at evenly spaced points
/// strictly between `from` and `to` (the endpoints themselves are not tested,
/// so an observer standing in a bush can still see a target behind a tree
/// trunk it is touching). The number of samples is `ceil(distance / step)`
/// minus one, capped by [`MAX_LINE_OF_SIGHT_SAMPLES`]; a non-positive or NaN
/// `step` uses the cap. Obstacles thinner than `step` can be missed.
pub fn line_of_sight(from: Vec2, to: Vec2, step: f32, mut is_clear: impl FnMut(Vec2) -> bool) -> bool {
    let distance = from.distance(to);
    if !distance.is_finite() || distance == 0.0 {
        return distance == 0.0;
    }
    let segments = if step > 0.0 {
        // Saturating float-to-int conversion: huge ratios hit the cap.
        ((distance / step).ceil() as u32).clamp(1, MAX_LINE_OF_SIGHT_SAMPLES + 1)
    } else {
        MAX_LINE_OF_SIGHT_SAMPLES + 1
    };
    (1..segments).all(|k| is_clear(from.lerp(to, k as f32 / segments as f32)))
}

/// Rebuilds `grid` with every live agent of `store`, using the agent's store
/// index as its slot. Returns [`OrganismError::TooManyAgents`] (leaving the grid
/// unchanged) if the store holds more than `u32::MAX` agents.
pub fn index_agents<T>(
    grid: &mut SpatialGrid,
    store: &AgentStore<T>,
    position: impl Fn(&T) -> Vec2,
) -> Result<(), OrganismError> {
    if u32::try_from(store.len()).is_err() {
        return Err(OrganismError::TooManyAgents(store.len()));
    }
    grid.rebuild(store.as_slice().iter().enumerate().map(|(i, agent)| (i as u32, position(agent))));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use simulation_core::SimRng;

    use super::*;

    fn grid_of(points: &[(f32, f32)]) -> SpatialGrid {
        let mut grid = SpatialGrid::new(20.0, 20.0, 3.0);
        grid.rebuild(points.iter().enumerate().map(|(i, &(x, y))| (i as u32, Vec2::new(x, y))));
        grid
    }

    #[test]
    fn nearest_respects_filter_cone_and_ties() {
        let grid = grid_of(&[(12.0, 10.0), (8.0, 10.0), (10.0, 13.0), (10.0, 10.0)]);
        let q = Query::new(Vec2::new(10.0, 10.0), 5.0);
        let not_self = |s: u32, _| s != 3;
        assert_eq!(nearest(&grid, &q, not_self).unwrap().slot, 0, "equal distance: lower slot wins");
        assert_eq!(nearest(&grid, &q, |s, _| s != 3 && s != 0).unwrap().slot, 1);
        // Looking towards -x with a 90° cone only sees slot 1 (and itself).
        let cone = q.with_fov(Vec2::new(-1.0, 0.0), std::f32::consts::FRAC_PI_2);
        let seen = nearest(&grid, &cone, not_self).unwrap();
        assert_eq!((seen.slot, seen.distance), (1, 2.0));
        let up = q.with_fov(Vec2::new(0.0, 1.0), 0.5);
        assert_eq!(nearest(&grid, &up, not_self).unwrap().slot, 2);
        assert!(nearest(&grid, &Query::new(Vec2::ZERO, 1.0), |_, _| true).is_none());
    }

    #[test]
    fn within_is_sorted_by_distance_then_slot_and_count_matches() {
        let mut rng = SimRng::seed_from_u64(4);
        let points: Vec<(f32, f32)> = (0..300).map(|_| (rng.range_f32(0.0, 20.0), rng.range_f32(0.0, 20.0))).collect();
        let mut with_ties = points.clone();
        with_ties.extend([(13.0, 10.0), (7.0, 10.0)]);
        let grid = grid_of(&with_ties);
        let q = Query::new(Vec2::new(10.0, 10.0), 6.0).with_fov(Vec2::new(1.0, 1.0), 4.0);
        let mut out = Vec::new();
        within(&grid, &q, |_, _| true, &mut out);
        assert!(out.windows(2).all(|w| (w[0].distance, w[0].slot) < (w[1].distance, w[1].slot)));
        let brute = with_ties.iter().filter(|&&(x, y)| q.accepts(Vec2::new(x, y))).count();
        assert_eq!(out.len(), brute);
        assert_eq!(count(&grid, &q, |_, _| true), brute);
        assert_eq!(nearest(&grid, &q, |_, _| true), out.first().copied(), "nearest is the first listed");
        assert_eq!(count(&grid, &q, |s, _| s % 2 == 0), out.iter().filter(|p| p.slot % 2 == 0).count());
    }

    #[test]
    fn wrapped_queries_match_brute_force_nearest_images() {
        let (w, h) = (40.0, 24.0);
        let world = Bounds::Wrap { min: Vec2::ZERO, max: Vec2::new(w, h) };
        let mut rng = SimRng::seed_from_u64(17);
        let points: Vec<Vec2> = (0..400).map(|_| Vec2::new(rng.range_f32(0.0, w), rng.range_f32(0.0, h))).collect();
        let mut grid = SpatialGrid::new(w, h, 4.0);
        grid.rebuild(points.iter().enumerate().map(|(i, &p)| (i as u32, p)));
        let mut out = Vec::new();
        let tol = 1e-3;
        for case in 0..300 {
            // Origins inside and (unwrapped) outside the world; radii below and above half a period.
            let origin = Vec2::new(rng.range_f32(-w, 2.0 * w), rng.range_f32(-h, 2.0 * h));
            let radius = if case % 3 == 0 { rng.range_f32(12.0, 30.0) } else { rng.range_f32(0.0, 11.0) };
            let mut q = Query::new(origin, radius).in_world(world);
            if case % 2 == 0 {
                q = q.with_fov(rng.unit_vec2(), rng.range_f32(0.5, 6.0));
            }
            within(&grid, &q, |_, _| true, &mut out);

            let slots: BTreeSet<u32> = out.iter().map(|p| p.slot).collect();
            assert_eq!(slots.len(), out.len(), "each item is reported once");
            for p in &out {
                let offset = world.delta(origin, points[p.slot as usize]);
                assert!((p.position - (origin + offset)).length() < tol, "nearest image in the observer frame");
                assert!((p.distance - offset.length()).abs() < tol && p.distance <= radius + tol);
            }
            for (i, &point) in points.iter().enumerate() {
                let offset = world.delta(origin, point);
                if offset.length() < radius - tol && q.accepts(point) {
                    assert!(slots.contains(&(i as u32)), "case {case}: item {i} at {point} missed");
                }
            }
            assert_eq!(count(&grid, &q, |_, _| true), out.len());
            assert_eq!(nearest(&grid, &q, |_, _| true), out.first().copied());
        }
        // A neighbour just across the edge is seen at its image next to the observer.
        let mut pair = SpatialGrid::new(w, h, 4.0);
        pair.rebuild([(0, Vec2::new(39.5, 12.0))]);
        let seen = nearest(&pair, &Query::new(Vec2::new(0.5, 12.0), 2.0).in_world(world), |_, _| true).unwrap();
        assert!((seen.position - Vec2::new(-0.5, 12.0)).length() < 1e-5 && (seen.distance - 1.0).abs() < 1e-5);
        assert!(nearest(&pair, &Query::new(Vec2::new(0.5, 12.0), 2.0), |_, _| true).is_none(), "plane: not seen");
    }

    #[test]
    fn field_of_view_edge_cases() {
        let full = FieldOfView::new(Vec2::X, std::f32::consts::TAU);
        assert!(full.contains(Vec2::new(-1.0, 0.0)));
        let blind_direction = FieldOfView::new(Vec2::ZERO, 0.1);
        assert!(blind_direction.contains(Vec2::new(0.0, -3.0)));
        let narrow = FieldOfView::new(Vec2::X, 0.2);
        assert!(narrow.contains(Vec2::new(5.0, 0.4)) && !narrow.contains(Vec2::new(5.0, 1.0)));
        assert!(narrow.contains(Vec2::ZERO));
    }

    #[test]
    fn line_of_sight_samples_between_endpoints() {
        let wall = |p: Vec2| !(4.9..=5.1).contains(&p.x);
        assert!(!line_of_sight(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), 0.1, wall));
        assert!(line_of_sight(Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), 0.1, wall));
        // Endpoints are not tested.
        assert!(line_of_sight(Vec2::new(5.0, 0.0), Vec2::new(5.0, 3.0), 0.5, |p| p.y > 0.1 && p.y < 2.9));
        // Too coarse a step misses a thin wall (documented limitation).
        assert!(line_of_sight(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), 4.0, wall));
        assert!(line_of_sight(Vec2::ONE, Vec2::ONE, 1.0, |_| false));
        let mut calls = 0;
        line_of_sight(Vec2::ZERO, Vec2::new(1.0e9, 0.0), f32::NAN, |_| {
            calls += 1;
            true
        });
        assert_eq!(calls, MAX_LINE_OF_SIGHT_SAMPLES);
    }

    #[test]
    fn index_agents_uses_store_order() {
        let store: AgentStore<Vec2> = [Vec2::new(1.0, 1.0), Vec2::new(5.0, 5.0)].into_iter().collect();
        let mut grid = SpatialGrid::new(10.0, 10.0, 2.0);
        index_agents(&mut grid, &store, |p| *p).unwrap();
        let hit = nearest(&grid, &Query::new(Vec2::new(4.0, 4.0), 3.0), |_, _| true).unwrap();
        assert_eq!(store.ids()[hit.slot as usize], crate::id::AgentId(2));
    }
}
