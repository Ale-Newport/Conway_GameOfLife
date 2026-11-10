//! Layout helpers of the trade-network graph (pure, unit tested): circular
//! layout for groups, an incremental Fruchterman–Reingold force layout for
//! people, and edge widths.

use std::collections::HashMap;
use std::hash::Hash;

use bevy::math::Vec2;

/// Positions of `n` nodes evenly spaced on a circle (first node at the top,
/// clockwise in screen coordinates with y down).
pub fn circle_layout(n: usize, centre: Vec2, radius: f32) -> Vec<Vec2> {
    if n == 1 {
        return vec![centre];
    }
    (0..n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / n as f32;
            centre + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect()
}

/// Stroke width of an edge carrying `count` meetings when the busiest edge
/// carries `max` (logarithmic, between `min_width` and `max_width`).
pub fn edge_width(count: u32, max: u32, min_width: f32, max_width: f32) -> f32 {
    if max == 0 || count == 0 {
        return min_width;
    }
    let t = ((1.0 + count as f32).ln() / (1.0 + max as f32).ln()).clamp(0.0, 1.0);
    min_width + (max_width - min_width) * t
}

/// Force-directed layout in the unit square `[0, 1]²` that persists between
/// frames: nodes keep their positions while the set of nodes changes, new
/// nodes start at a deterministic position, and every call runs a few
/// Fruchterman–Reingold iterations with a cooling step size.
#[derive(Clone, Debug)]
pub struct ForceLayout<K: Eq + Hash + Copy + Ord> {
    positions: HashMap<K, Vec2>,
    temperature: f32,
}

impl<K: Eq + Hash + Copy + Ord> Default for ForceLayout<K> {
    fn default() -> Self {
        Self { positions: HashMap::new(), temperature: 0.1 }
    }
}

impl<K: Eq + Hash + Copy + Ord> ForceLayout<K> {
    /// Position of a node (unit square), if laid out.
    pub fn position(&self, key: &K) -> Option<Vec2> {
        self.positions.get(key).copied()
    }

    /// Runs `iterations` steps for `nodes` (sorted, unique) and weighted
    /// undirected `edges`; `seed_of(key)` gives a deterministic start in
    /// `[0, 1)²` for new nodes. Nodes not listed are forgotten.
    pub fn step(&mut self, nodes: &[K], edges: &[(K, K, f32)], iterations: usize, seed_of: impl Fn(K) -> Vec2) {
        let before = self.positions.len();
        self.positions.retain(|k, _| nodes.binary_search(k).is_ok());
        let mut added = before != self.positions.len();
        for &k in nodes {
            self.positions.entry(k).or_insert_with(|| {
                added = true;
                seed_of(k).clamp(Vec2::splat(0.05), Vec2::splat(0.95))
            });
        }
        if added {
            // Re-heat so new nodes find their place.
            self.temperature = self.temperature.max(0.06);
        }
        let n = nodes.len();
        if n < 2 {
            if let Some(&k) = nodes.first() {
                self.positions.insert(k, Vec2::splat(0.5));
            }
            return;
        }
        let k_ideal = (1.0 / n as f32).sqrt() * 0.5;
        let index: HashMap<K, usize> = nodes.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        let mut pos: Vec<Vec2> = nodes.iter().map(|k| self.positions[k]).collect();
        for _ in 0..iterations {
            let mut disp = vec![Vec2::ZERO; n];
            for i in 0..n {
                for j in i + 1..n {
                    let mut d = pos[i] - pos[j];
                    if d.length_squared() < 1e-8 {
                        d = Vec2::new(1e-3 * (i as f32 - j as f32), 1e-3);
                    }
                    let dist = d.length();
                    let force = k_ideal * k_ideal / dist;
                    let push = d / dist * force;
                    disp[i] += push;
                    disp[j] -= push;
                }
            }
            for &(a, b, w) in edges {
                let (Some(&i), Some(&j)) = (index.get(&a), index.get(&b)) else { continue };
                if i == j {
                    continue;
                }
                let d = pos[i] - pos[j];
                let dist = d.length().max(1e-4);
                let pull = d / dist * (dist * dist / k_ideal) * w.clamp(0.2, 3.0);
                disp[i] -= pull;
                disp[j] += pull;
            }
            // Gravity towards the centre keeps disconnected parts on the
            // canvas instead of against its borders.
            for (p, d) in pos.iter().zip(disp.iter_mut()) {
                *d += (Vec2::splat(0.5) - *p) * 1.5;
            }
            for (p, d) in pos.iter_mut().zip(&disp) {
                let len = d.length();
                if len > 0.0 {
                    *p += *d / len * len.min(self.temperature);
                }
                *p = p.clamp(Vec2::splat(0.04), Vec2::splat(0.96));
            }
            self.temperature = (self.temperature * 0.97).max(0.004);
        }
        for (k, p) in nodes.iter().zip(pos) {
            self.positions.insert(*k, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_layout_is_evenly_spaced() {
        let pts = circle_layout(4, Vec2::new(10.0, 10.0), 5.0);
        assert!((pts[0] - Vec2::new(10.0, 5.0)).length() < 1e-4, "first node at the top");
        for p in &pts {
            assert!((p.distance(Vec2::new(10.0, 10.0)) - 5.0).abs() < 1e-4);
        }
        let d01 = pts[0].distance(pts[1]);
        let d12 = pts[1].distance(pts[2]);
        assert!((d01 - d12).abs() < 1e-4);
        assert_eq!(circle_layout(1, Vec2::ONE, 3.0), vec![Vec2::ONE]);
        assert!(circle_layout(0, Vec2::ONE, 3.0).is_empty());
    }

    #[test]
    fn edge_widths_grow_logarithmically() {
        assert_eq!(edge_width(0, 10, 1.0, 5.0), 1.0);
        assert_eq!(edge_width(10, 10, 1.0, 5.0), 5.0);
        let mid = edge_width(3, 10, 1.0, 5.0);
        assert!(mid > 1.0 && mid < 5.0);
        assert!(edge_width(3, 100, 1.0, 5.0) < mid, "relative to the busiest edge");
        assert_eq!(edge_width(3, 0, 1.0, 5.0), 1.0);
    }

    #[test]
    fn force_layout_pulls_partners_together_and_stays_in_bounds() {
        let mut layout: ForceLayout<u32> = ForceLayout::default();
        let nodes = [1u32, 2, 3, 4, 5, 6];
        // Two triangles of traders joined by a weak bridge.
        let edges = [(1, 2, 2.0), (2, 3, 2.0), (1, 3, 2.0), (4, 5, 2.0), (5, 6, 2.0), (4, 6, 2.0), (3, 4, 0.2)];
        let seed = |k: u32| Vec2::new((k as f32 * 0.37).fract(), (k as f32 * 0.61).fract());
        for _ in 0..40 {
            layout.step(&nodes, &edges, 5, seed);
        }
        let p = |k: u32| layout.position(&k).expect("laid out");
        for k in nodes {
            let q = p(k);
            assert!((0.0..=1.0).contains(&q.x) && (0.0..=1.0).contains(&q.y));
        }
        let within = p(1).distance(p(2)) + p(4).distance(p(5));
        let across = p(1).distance(p(5)) + p(2).distance(p(6));
        assert!(within < across, "clusters: {within} vs {across}");
        // A larger graph of several trading clusters stays off the borders.
        let mut big: ForceLayout<u32> = ForceLayout::default();
        let nodes: Vec<u32> = (0..48).collect();
        let mut edges = Vec::new();
        for c in 0..4u32 {
            for i in 0..12u32 {
                for j in i + 1..12 {
                    if (i * 7 + j * 3) % 4 == 0 {
                        edges.push((c * 12 + i, c * 12 + j, 1.0));
                    }
                }
            }
        }
        edges.push((0, 12, 0.3));
        edges.push((24, 36, 0.3));
        for _ in 0..150 {
            big.step(&nodes, &edges, 3, seed);
        }
        let on_border = nodes
            .iter()
            .filter(|k| {
                let q = big.position(k).unwrap();
                q.x < 0.06 || q.y < 0.06 || q.x > 0.94 || q.y > 0.94
            })
            .count();
        assert!(on_border <= 2, "{on_border} nodes pressed against the border");
        let xs: Vec<f32> = nodes.iter().map(|k| big.position(k).unwrap().x).collect();
        let ys: Vec<f32> = nodes.iter().map(|k| big.position(k).unwrap().y).collect();
        let span = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min);
        assert!(span(&xs) > 0.45 && span(&ys) > 0.45, "the graph uses the canvas: {} × {}", span(&xs), span(&ys));
        // Removing nodes forgets them; adding keeps the others in place.
        let kept = p(1);
        layout.step(&[1, 2, 3, 9], &[(1, 2, 1.0)], 0, seed);
        assert!(layout.position(&5).is_none());
        assert_eq!(layout.position(&1), Some(kept), "zero iterations do not move nodes");
        assert!(layout.position(&9).is_some());
    }
}
