//! Resource fields: initialisation, per-tick dynamics and painting.
//!
//! Per tick and cell, in this order: decay `R ← R·(1 − decay)`; regrowth
//! `R ← min(max, R + regen)` for cells below `max` (cells above `max`, e.g.
//! after emission, are left unchanged); explicit diffusion (forward-time,
//! centred-space, 5-point stencil) `R ← R + D·Σₙ(Rₙ − R)` with
//! topology-aware neighbours and zero flux across bounded edges. Diffusion
//! conserves the total exactly in exact arithmetic (every exchange is
//! antisymmetric) and is stable for `D ≤ 0.25`, which validation enforces.

use rayon::prelude::*;
use simulation_core::{Grid2, SimRng, Topology};

use crate::def::{Axis, FieldInit, ResourceField};

/// Rows below which diffusion runs sequentially.
const PARALLEL_ROWS: usize = 64;

/// Initial content of a field.
pub fn init_field(def: &ResourceField, width: usize, height: usize, rng: &mut SimRng) -> Grid2<f32> {
    match def.initial {
        FieldInit::Uniform(v) => Grid2::new(width, height, v),
        FieldInit::Random { min, max } => Grid2::from_fn(width, height, |_, _| rng.range_f32(min, max.max(min))),
        FieldInit::Gradient { from, to, axis } => Grid2::from_fn(width, height, |x, y| {
            let t = match axis {
                Axis::X => (x as f32 + 0.5) / width as f32,
                Axis::Y => (y as f32 + 0.5) / height as f32,
            };
            from + (to - from) * t
        }),
        FieldInit::Patches { count, radius, amount } => {
            let mut g = Grid2::new(width, height, 0.0);
            for _ in 0..count {
                let cx = rng.range_f32(0.0, width as f32);
                let cy = rng.range_f32(0.0, height as f32);
                disc_cells(cx, cy, radius, width, height, Topology::Torus, |i| g.as_mut_slice()[i] = amount);
            }
            g
        }
    }
}

/// Calls `f(cell index)` once for every cell whose centre lies within
/// `radius` of `(cx, cy)`, following the topology's edge identification.
///
/// Cost is bounded whatever the radius: on a bounded world the scan is
/// clipped to the grid; on wrapping worlds a disc at least as wide as the
/// world's diagonal covers every cell, so larger radii are capped there.
/// Cells are visited in scan order (deterministic). A cell reachable
/// through several images is visited once: duplicates can only arise when
/// the scanned box is larger than the world, or near the corners of a
/// cross-surface (where `(-1, 0)` and `(0, -1)` are the same cell); only
/// then is a bitmap (large boxes) or a hash set (small ones) kept.
pub fn disc_cells(
    cx: f32,
    cy: f32,
    radius: f32,
    width: usize,
    height: usize,
    topology: Topology,
    mut f: impl FnMut(usize),
) {
    if !(cx.is_finite() && cy.is_finite()) || width == 0 || height == 0 {
        return;
    }
    let (w, h) = (width as i64, height as i64);
    let diagonal = (width as f32).hypot(height as f32);
    let r = radius.max(0.0).min(diagonal + 1.0);
    if topology != Topology::Bounded && r >= diagonal {
        (0..width * height).for_each(f);
        return;
    }
    let mut x0 = (cx - r - 0.5).floor() as i64;
    let mut x1 = (cx + r - 0.5).ceil() as i64;
    let mut y0 = (cy - r - 0.5).floor() as i64;
    let mut y1 = (cy + r - 0.5).ceil() as i64;
    if topology == Topology::Bounded {
        (x0, x1, y0, y1) = (x0.max(0), x1.min(w - 1), y0.max(0), y1.min(h - 1));
    }
    let (box_w, box_h) = ((x1 - x0 + 1).max(0), (y1 - y0 + 1).max(0));
    let dedupe = topology == Topology::CrossSurface || box_w > w || box_h > h;
    let large = (box_w * box_h) as usize >= width * height / 4;
    let mut bitmap = (dedupe && large).then(|| vec![false; width * height]);
    let mut set = (dedupe && !large).then(std::collections::HashSet::<usize>::new);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            if dx * dx + dy * dy > r * r {
                continue;
            }
            if let Some((wx, wy)) = topology.wrap(x, y, w, h) {
                let i = wy as usize * width + wx as usize;
                let first = match (bitmap.as_mut(), set.as_mut()) {
                    (Some(b), _) => !std::mem::replace(&mut b[i], true),
                    (None, Some(s)) => s.insert(i),
                    (None, None) => true,
                };
                if first {
                    f(i);
                }
            }
        }
    }
}

fn neighbour(x: usize, y: usize, dx: i64, dy: i64, w: usize, h: usize, topology: Topology) -> usize {
    match topology.wrap(x as i64 + dx, y as i64 + dy, w as i64, h as i64) {
        Some((nx, ny)) => ny as usize * w + nx as usize,
        // Outside a bounded world: zero flux (the cell exchanges with itself).
        None => y * w + x,
    }
}

/// One diffusion step with coefficient `d` into `out`.
pub fn diffuse(src: &Grid2<f32>, out: &mut Grid2<f32>, d: f32, topology: Topology) {
    let (w, h) = (src.width(), src.height());
    let data = src.as_slice();
    let row = |y: usize, dst: &mut [f32]| {
        for (x, cell) in dst.iter_mut().enumerate() {
            let c = data[y * w + x];
            let sum: f32 = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .map(|&(dx, dy)| data[neighbour(x, y, dx, dy, w, h, topology)] - c)
                .sum();
            *cell = c + d * sum;
        }
    };
    if h < PARALLEL_ROWS {
        for (y, dst) in out.as_mut_slice().chunks_mut(w).enumerate() {
            row(y, dst);
        }
    } else {
        out.as_mut_slice().par_chunks_mut(w).enumerate().for_each(|(y, dst)| row(y, dst));
    }
}

/// Decay, regrowth and diffusion of every field for one tick.
pub fn update_fields(fields: &mut [Grid2<f32>], defs: &[ResourceField], topology: Topology, scratch: &mut Grid2<f32>) {
    for (g, def) in fields.iter_mut().zip(defs) {
        if def.decay > 0.0 || def.regen > 0.0 {
            let keep = 1.0 - def.decay;
            for v in g.as_mut_slice() {
                *v *= keep;
                if *v < def.max {
                    *v = (*v + def.regen).min(def.max);
                }
            }
        }
        if def.diffusion > 0.0 {
            if scratch.width() != g.width() || scratch.height() != g.height() {
                *scratch = Grid2::new(g.width(), g.height(), 0.0);
            }
            diffuse(g, scratch, def.diffusion, topology);
            std::mem::swap(g, scratch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(g: &Grid2<f32>) -> f64 {
        g.sum()
    }

    #[test]
    fn diffusion_conserves_mass_on_every_topology() {
        for topology in [Topology::Bounded, Topology::Torus, Topology::KleinBottle, Topology::CrossSurface] {
            let mut rng = SimRng::seed_from_u64(3);
            let mut g = Grid2::from_fn(17, 13, |_, _| rng.range_f32(0.0, 4.0));
            let before = total(&g);
            let mut out = Grid2::new(17, 13, 0.0);
            for _ in 0..200 {
                diffuse(&g, &mut out, 0.2, topology);
                std::mem::swap(&mut g, &mut out);
            }
            assert!((total(&g) - before).abs() < 1e-3 * before, "{topology:?}: {} vs {before}", total(&g));
            // Diffusion smooths: the spread shrinks.
            let (lo, hi) = g.as_slice().iter().fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
            assert!(hi - lo < 1.0, "{topology:?} not smoothed: {lo}..{hi}");
        }
    }

    #[test]
    fn diffusion_spreads_symmetrically_from_a_point() {
        let mut g = Grid2::new(9, 9, 0.0);
        g.set(4, 4, 1.0);
        let mut out = Grid2::new(9, 9, 0.0);
        diffuse(&g, &mut out, 0.2, Topology::Torus);
        assert!((out.get(4, 4) - 0.2).abs() < 1e-6);
        for (x, y) in [(3, 4), (5, 4), (4, 3), (4, 5)] {
            assert!((out.get(x, y) - 0.2).abs() < 1e-6);
        }
    }

    #[test]
    fn regrowth_saturates_at_max_and_decay_shrinks() {
        let mut def = ResourceField::new("food");
        def.regen = 0.3;
        def.max = 1.0;
        let mut fields = vec![Grid2::new(4, 4, 0.0)];
        let mut scratch = Grid2::new(1, 1, 0.0);
        for _ in 0..10 {
            update_fields(&mut fields, std::slice::from_ref(&def), Topology::Torus, &mut scratch);
        }
        assert!(fields[0].as_slice().iter().all(|&v| (v - 1.0).abs() < 1e-6));
        def.regen = 0.0;
        def.decay = 0.5;
        update_fields(&mut fields, std::slice::from_ref(&def), Topology::Torus, &mut scratch);
        assert!(fields[0].as_slice().iter().all(|&v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn disc_cells_is_exact_and_bounded_for_any_radius() {
        // Brute force: every cell whose centre has an image within the radius.
        for topology in [Topology::Bounded, Topology::Torus, Topology::KleinBottle, Topology::CrossSurface] {
            for (cx, cy, r) in [(3.2, 4.7, 2.5), (0.3, 9.6, 4.0), (5.0, 5.0, 7.5), (9.9, 0.1, 12.0), (2.0, 3.0, 1e9)] {
                let (w, h) = (11usize, 10usize);
                let mut got = Vec::new();
                disc_cells(cx, cy, r, w, h, topology, |i| got.push(i));
                let mut sorted = got.clone();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(sorted.len(), got.len(), "{topology:?} r={r}: duplicates");
                let mut expected = Vec::new();
                let span = 3 * (w.max(h) as i64);
                for y in -span..span {
                    for x in -span..span {
                        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                        if dx * dx + dy * dy <= r.min(100.0) * r.min(100.0)
                            && let Some((wx, wy)) = topology.wrap(x, y, w as i64, h as i64)
                        {
                            expected.push(wy as usize * w + wx as usize);
                        }
                    }
                }
                expected.sort_unstable();
                expected.dedup();
                assert_eq!(sorted, expected, "{topology:?} at ({cx}, {cy}) r={r}");
            }
        }
        // A huge brush on a big world finishes immediately and covers it once.
        let t = std::time::Instant::now();
        let mut n = 0usize;
        disc_cells(10.0, 10.0, 1e30, 1024, 1024, Topology::Torus, |_| n += 1);
        assert_eq!(n, 1024 * 1024);
        let mut m = 0usize;
        disc_cells(512.0, 512.0, 400.0, 1024, 1024, Topology::KleinBottle, |_| m += 1);
        assert!(m > 400_000);
        assert!(t.elapsed().as_secs_f64() < 2.0, "{:?}", t.elapsed());
    }

    #[test]
    fn disc_cells_wraps_on_a_torus_and_clips_when_bounded() {
        let mut torus = Vec::new();
        disc_cells(0.5, 0.5, 1.0, 8, 8, Topology::Torus, |i| torus.push(i));
        assert_eq!(torus.len(), 5);
        assert!(torus.contains(&(7 * 8)), "wraps to the last row");
        let mut bounded = Vec::new();
        disc_cells(0.5, 0.5, 1.0, 8, 8, Topology::Bounded, |i| bounded.push(i));
        assert_eq!(bounded.len(), 3);
    }
}
