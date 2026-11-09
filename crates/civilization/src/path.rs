//! A* route planning on the tile grid with road-discounted costs.
//!
//! Moves go to the 8 neighbours; a diagonal move is allowed only when both
//! orthogonal tiles it cuts past are passable. Moving from tile `a` to `b`
//! costs `len · (c_a + c_b)/2` with `len ∈ {1, √2}` and `c` the
//! road-discounted movement cost ([`World::move_cost`]). The heuristic is the
//! octile distance times the cheapest possible tile cost `1 − k` (a saturated
//! road on open land), which never overestimates, so returned routes are
//! optimal. Heap ties are broken by tile index, so the result is
//! deterministic. A search that expands more than `max_expansions` tiles
//! gives up.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::config::Rules;
use crate::world::World;

#[derive(Clone, Copy, Debug)]
struct Node {
    f: f32,
    tile: u32,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Node {}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Node {
    /// Max-heap order: the smallest `f`, then the smallest tile, is greatest.
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f).then_with(|| other.tile.cmp(&self.tile))
    }
}

/// Reusable A* scratch buffers (generation-stamped, so no clearing between
/// searches).
#[derive(Clone, Debug, Default)]
pub struct PathFinder {
    g: Vec<f32>,
    parent: Vec<u32>,
    stamp: Vec<u32>,
    closed: Vec<u32>,
    generation: u32,
    heap: BinaryHeap<Node>,
}

impl PathFinder {
    /// Empty scratch; buffers grow on first use.
    pub fn new() -> Self {
        Self::default()
    }

    fn prepare(&mut self, n: usize) {
        if self.g.len() != n {
            self.g = vec![0.0; n];
            self.parent = vec![0; n];
            self.stamp = vec![0; n];
            self.closed = vec![0; n];
            self.generation = 0;
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamp.fill(0);
            self.closed.fill(0);
            self.generation = 1;
        }
        self.heap.clear();
    }

    /// Cheapest route from `start` to `goal`. Returns the tiles to visit
    /// **in reverse order** (goal first, next waypoint last, `start`
    /// excluded) so callers can `pop()` waypoints, or `None` if the goal is
    /// impassable, unreachable or too far for `max_expansions`.
    pub fn find(
        &mut self,
        world: &World,
        rules: &Rules,
        start: u32,
        goal: u32,
        max_expansions: usize,
    ) -> Option<Vec<u32>> {
        let n = world.len();
        if start as usize >= n || goal as usize >= n || !world.passable(goal) {
            return None;
        }
        if start == goal {
            return Some(Vec::new());
        }
        self.prepare(n);
        let gen_ = self.generation;
        let min_cost = (1.0 - rules.road_discount).max(0.05);
        let (gx, gy) = world.coords(goal);
        let heuristic = |t: u32| {
            let (x, y) = world.coords(t);
            let (dx, dy) = (x.abs_diff(gx) as f32, y.abs_diff(gy) as f32);
            (dx.max(dy) + (std::f32::consts::SQRT_2 - 1.0) * dx.min(dy)) * min_cost
        };
        self.g[start as usize] = 0.0;
        self.stamp[start as usize] = gen_;
        self.heap.push(Node { f: heuristic(start), tile: start });
        let mut expansions = 0;
        while let Some(Node { tile, .. }) = self.heap.pop() {
            if self.closed[tile as usize] == gen_ {
                continue;
            }
            self.closed[tile as usize] = gen_;
            if tile == goal {
                return Some(self.reconstruct(start, goal));
            }
            expansions += 1;
            if expansions > max_expansions {
                return None;
            }
            let (x, y) = world.coords(tile);
            let cost_here = world.move_cost(tile, rules);
            let g_here = self.g[tile as usize];
            for next in world.neighbors(tile) {
                if !world.passable(next) || self.closed[next as usize] == gen_ {
                    continue;
                }
                let (nx, ny) = world.coords(next);
                let diagonal = nx != x && ny != y;
                if diagonal && !(world.passable(world.index(nx, y)) && world.passable(world.index(x, ny))) {
                    continue;
                }
                let len = if diagonal { std::f32::consts::SQRT_2 } else { 1.0 };
                let g = g_here + len * 0.5 * (cost_here + world.move_cost(next, rules));
                let slot = next as usize;
                if self.stamp[slot] != gen_ || g < self.g[slot] {
                    self.stamp[slot] = gen_;
                    self.g[slot] = g;
                    self.parent[slot] = tile;
                    self.heap.push(Node { f: g + heuristic(next), tile: next });
                }
            }
        }
        None
    }

    fn reconstruct(&self, start: u32, goal: u32) -> Vec<u32> {
        let mut path = Vec::new();
        let mut t = goal;
        while t != start {
            path.push(t);
            t = self.parent[t as usize];
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::parse_map;

    fn map(rows: &[&str]) -> World {
        World::new(parse_map(&rows.iter().map(|r| r.to_string()).collect::<Vec<_>>()).unwrap(), 1.0)
    }

    #[test]
    fn finds_valid_routes_around_obstacles() {
        let w = map(&["........", "..^^^^..", "..^.....", "..^.^^^.", "..^.^...", "........", "........", "........"]);
        let rules = Rules::default();
        let mut pf = PathFinder::new();
        let (start, goal) = (w.index(0, 0), w.index(3, 4));
        let path = pf.find(&w, &rules, start, goal, 10_000).unwrap();
        assert_eq!(path.first(), Some(&goal));
        // Every step is to a passable neighbour, without cutting corners.
        let mut prev = start;
        for &t in path.iter().rev() {
            assert!(w.passable(t));
            assert!(w.neighbors(prev).any(|n| n == t), "{prev} -> {t}");
            prev = t;
        }
        assert_eq!(pf.find(&w, &rules, start, w.index(2, 1), 10_000), None, "impassable goal");
        assert_eq!(pf.find(&w, &rules, start, start, 10), Some(vec![]));
    }

    /// Reference Dijkstra over the same move rules and edge costs.
    fn dijkstra(w: &World, rules: &Rules, start: u32, goal: u32) -> Option<f32> {
        let n = w.len();
        let mut dist = vec![f32::INFINITY; n];
        let mut done = vec![false; n];
        dist[start as usize] = 0.0;
        loop {
            let next =
                (0..n).filter(|&i| !done[i] && dist[i].is_finite()).min_by(|&a, &b| dist[a].total_cmp(&dist[b]))?;
            if next as u32 == goal {
                return Some(dist[next]);
            }
            done[next] = true;
            let t = next as u32;
            let (x, y) = w.coords(t);
            for nb in w.neighbors(t).filter(|&nb| w.passable(nb)) {
                let (nx, ny) = w.coords(nb);
                let diagonal = nx != x && ny != y;
                if diagonal && !(w.passable(w.index(nx, y)) && w.passable(w.index(x, ny))) {
                    continue;
                }
                let len = if diagonal { std::f32::consts::SQRT_2 } else { 1.0 };
                let d = dist[next] + len * 0.5 * (w.move_cost(t, rules) + w.move_cost(nb, rules));
                if d < dist[nb as usize] {
                    dist[nb as usize] = d;
                }
            }
        }
    }

    fn route_cost(w: &World, rules: &Rules, start: u32, path: &[u32]) -> f32 {
        let mut prev = start;
        let mut cost = 0.0;
        for &t in path.iter().rev() {
            let diagonal = w.coords(prev).0 != w.coords(t).0 && w.coords(prev).1 != w.coords(t).1;
            let len = if diagonal { std::f32::consts::SQRT_2 } else { 1.0 };
            cost += len * 0.5 * (w.move_cost(prev, rules) + w.move_cost(t, rules));
            prev = t;
        }
        cost
    }

    #[test]
    fn routes_are_optimal_against_dijkstra() {
        // Mixed terrain costs, obstacles and partly worn roads.
        let mut w = map(&[
            "..TT..hh..",
            ".^^T..hh.b",
            ".^...TT..b",
            "..~~.^^^..",
            "..~~...h..",
            "T....b.h.^",
            "TT.^^b....",
            "..h..b.TT.",
        ]);
        let rules = Rules::default();
        for x in 0..10 {
            for _ in 0..(20 * x) {
                w.add_footfall(w.index(x, 4));
            }
        }
        let mut pf = PathFinder::new();
        let land: Vec<u32> = (0..w.len() as u32).filter(|&t| w.passable(t)).collect();
        let mut checked = 0;
        for (i, &start) in land.iter().enumerate().step_by(5) {
            for &goal in land.iter().skip(i % 7).step_by(9) {
                let expected = dijkstra(&w, &rules, start, goal);
                let found = pf.find(&w, &rules, start, goal, 10_000);
                match (expected, found) {
                    (Some(best), Some(path)) => {
                        let cost = route_cost(&w, &rules, start, &path);
                        assert!((cost - best).abs() < 1e-3, "{start} -> {goal}: A* {cost} vs Dijkstra {best}");
                        checked += 1;
                    }
                    (None, None) => {}
                    (e, f) => panic!("{start} -> {goal}: Dijkstra {e:?} vs A* {f:?}"),
                }
            }
        }
        assert!(checked > 40, "{checked} routes compared");
    }

    #[test]
    fn roads_attract_routes() {
        // Two routes of equal length around a mountain block; a road on the
        // southern one makes the planner take it.
        let mut w =
            map(&["........", "........", "..^^^^..", "..^^^^..", "..^^^^..", "........", "........", "........"]);
        let rules = Rules::default();
        let mut pf = PathFinder::new();
        let (start, goal) = (w.index(0, 3), w.index(7, 3));
        let north = pf.find(&w, &rules, start, goal, 10_000).unwrap();
        let goes_south = |p: &[u32]| p.iter().any(|&t| w.coords(t).1 >= 5);
        assert!(!goes_south(&north), "without roads ties go north (lower indices)");
        let south_row: Vec<u32> = (1..7).map(|x| w.index(x, 5)).collect();
        for &t in &south_row {
            for _ in 0..300 {
                w.add_footfall(t);
            }
        }
        let routed = pf.find(&w, &rules, start, goal, 10_000).unwrap();
        assert!(routed.iter().any(|&t| w.coords(t).1 >= 5), "road attracts the route: {routed:?}");
        assert_eq!(pf.find(&w, &rules, start, goal, 3), None, "expansion budget");
    }
}
