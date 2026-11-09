//! Field statistics: ordered sums, per-channel summaries and the periodic
//! (torus) centroid with displacement tracking.
//!
//! Every reduction accumulates per-row partial sums in `f64` and adds them
//! in row order, so results do not depend on the number of threads.

use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

use crate::parallel::sum_rows;

/// Cells whose (channel-summed) value exceeds this threshold count as
/// "active" in [`ChannelStats::active_fraction`] and the `active_fraction`
/// metric.
pub const ACTIVE_THRESHOLD: f32 = 0.01;

/// Sum of a row-major field (rows of `width` values) in `f64`.
pub fn field_sum(values: &[f32], width: usize) -> f64 {
    if width == 0 {
        return 0.0;
    }
    let rows = values.len() / width;
    sum_rows::<1, _>(rows, values.len(), |y| [values[y * width..(y + 1) * width].iter().map(|&v| f64::from(v)).sum()])
        [0]
}

/// Summary of one channel or field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelStats {
    /// Sum of all values ("mass" in cell units).
    pub mass: f64,
    /// Mean value.
    pub mean: f64,
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
    /// Fraction of cells above [`ACTIVE_THRESHOLD`].
    pub active_fraction: f64,
}

/// Computes [`ChannelStats`] of a row-major field.
pub fn channel_stats(values: &[f32], width: usize) -> ChannelStats {
    if values.is_empty() || width == 0 {
        return ChannelStats::default();
    }
    let rows = values.len() / width;
    let [mass, active, min_neg, max] = sum_rows_minmax(values, width, rows);
    let n = values.len() as f64;
    ChannelStats { mass, mean: mass / n, min: (-min_neg) as f32, max: max as f32, active_fraction: active / n }
}

/// Row reduction returning `[sum, active count, max(−v), max(v)]`; the
/// extremes are combined with `max`, which is order-independent.
fn sum_rows_minmax(values: &[f32], width: usize, rows: usize) -> [f64; 4] {
    let per_row: Vec<[f64; 4]> = (0..rows)
        .map(|y| {
            let row = &values[y * width..(y + 1) * width];
            let mut acc = [0.0, 0.0, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for &v in row {
                acc[0] += f64::from(v);
                acc[1] += f64::from(u8::from(v > ACTIVE_THRESHOLD));
                acc[2] = acc[2].max(-f64::from(v));
                acc[3] = acc[3].max(f64::from(v));
            }
            acc
        })
        .collect();
    let mut total = [0.0, 0.0, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for r in &per_row {
        total[0] += r[0];
        total[1] += r[1];
        total[2] = total[2].max(r[2]);
        total[3] = total[3].max(r[3]);
    }
    total
}

/// Mass-weighted circular mean of the channel-summed field on a
/// `width × height` torus: per axis, `atan2(Σ m·sin θ, Σ m·cos θ)` with
/// `θ = 2π(x + ½)/W`. The result is in world coordinates (the centre of cell
/// `(i, j)` is `(i + ½, j + ½)`), wrapped into `[0, W) × [0, H)`. Returns
/// `None` when the mass is zero or spread so evenly that the mean direction
/// is undefined.
pub fn torus_centroid(channels: &[&[f32]], width: usize, height: usize) -> Option<(f64, f64)> {
    if width == 0 || height == 0 || channels.is_empty() {
        return None;
    }
    let col_angle: Vec<(f64, f64)> = (0..width).map(|x| (TAU * (x as f64 + 0.5) / width as f64).sin_cos()).collect();
    let [mass, xs, xc, ys, yc] = sum_rows::<5, _>(height, width * height, |y| {
        let (sy, cy) = (TAU * (y as f64 + 0.5) / height as f64).sin_cos();
        let mut row_mass = 0.0;
        let mut s = 0.0;
        let mut c = 0.0;
        for (x, &(sx, cx)) in col_angle.iter().enumerate() {
            let m: f64 = channels.iter().map(|ch| f64::from(ch[y * width + x])).sum();
            row_mass += m;
            s += m * sx;
            c += m * cx;
        }
        [row_mass, s, c, row_mass * sy, row_mass * cy]
    });
    if mass <= 0.0 {
        return None;
    }
    let axis = |s: f64, c: f64, n: usize| {
        if (s * s + c * c).sqrt() <= 1e-9 * mass {
            return None;
        }
        let angle = s.atan2(c).rem_euclid(TAU);
        Some((angle / TAU * n as f64).rem_euclid(n as f64))
    };
    Some((axis(xs, xc, width)?, axis(ys, yc, height)?))
}

/// Shortest signed difference `b − a` on a circle of circumference `n`.
pub fn wrapped_delta(a: f64, b: f64, n: f64) -> f64 {
    let d = (b - a).rem_euclid(n);
    if d >= n / 2.0 { d - n } else { d }
}

/// Follows the torus centroid from tick to tick and accumulates its
/// unwrapped trajectory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CentroidTracker {
    last: Option<(f64, f64)>,
    unwrapped: (f64, f64),
    speed: f64,
}

impl CentroidTracker {
    /// Records the centroid of a new tick on a `width × height` torus. The
    /// speed is the length of the shortest wrapped displacement since the
    /// previous tick (0 when either centroid is undefined).
    pub fn update(&mut self, centroid: Option<(f64, f64)>, width: usize, height: usize) {
        match (self.last, centroid) {
            (Some((x0, y0)), Some((x1, y1))) => {
                let dx = wrapped_delta(x0, x1, width as f64);
                let dy = wrapped_delta(y0, y1, height as f64);
                self.unwrapped.0 += dx;
                self.unwrapped.1 += dy;
                self.speed = dx.hypot(dy);
            }
            (_, Some((x1, y1))) => {
                self.unwrapped = (x1, y1);
                self.speed = 0.0;
            }
            (_, None) => self.speed = 0.0,
        }
        self.last = centroid;
    }

    /// Forgets the previous centroid (after an edit that moves mass
    /// discontinuously) without touching the trajectory origin.
    pub fn restart(&mut self, centroid: Option<(f64, f64)>) {
        self.last = centroid;
        if let Some(c) = centroid {
            self.unwrapped = c;
        }
        self.speed = 0.0;
    }

    /// Current centroid (wrapped into the world), if defined.
    pub fn centroid(&self) -> Option<(f64, f64)> {
        self.last
    }

    /// Unwrapped centroid position: the wrapped start position plus every
    /// per-tick displacement since the last restart.
    pub fn unwrapped(&self) -> (f64, f64) {
        self.unwrapped
    }

    /// Centroid speed during the last tick, in cells per tick.
    pub fn speed(&self) -> f64 {
        self.speed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_of_a_small_field() {
        let v = [0.0, 0.5, 1.0, 0.005, 0.25, 0.25];
        let s = channel_stats(&v, 3);
        assert!((s.mass - 2.005).abs() < 1e-6);
        assert_eq!((s.min, s.max), (0.0, 1.0));
        assert!((s.active_fraction - 4.0 / 6.0).abs() < 1e-12);
        assert!((field_sum(&v, 3) - 2.005).abs() < 1e-6);
    }

    #[test]
    fn centroid_handles_wrapping_blobs() {
        let (w, h) = (20, 10);
        let mut f = vec![0.0f32; w * h];
        // A blob straddling the left/right edge at columns 19 and 0, row 4.
        f[4 * w + 19] = 1.0;
        f[4 * w] = 1.0;
        let (x, y) = torus_centroid(&[&f], w, h).expect("defined");
        assert!(x.abs() < 1e-9 || (x - 20.0).abs() < 1e-9, "x = {x}");
        assert!((y - 4.5).abs() < 1e-9);
        let uniform = vec![1.0f32; w * h];
        assert_eq!(torus_centroid(&[&uniform], w, h), None);
        assert_eq!(torus_centroid(&[&vec![0.0; w * h]], w, h), None);
    }

    #[test]
    fn tracker_unwraps_displacements() {
        let mut t = CentroidTracker::default();
        t.update(Some((98.0, 5.0)), 100, 100);
        t.update(Some((1.0, 5.0)), 100, 100);
        assert!((t.speed() - 3.0).abs() < 1e-12);
        assert!((t.unwrapped().0 - 101.0).abs() < 1e-12);
        t.update(None, 100, 100);
        assert_eq!(t.speed(), 0.0);
        assert_eq!(wrapped_delta(10.0, 60.0, 100.0), -50.0);
        assert_eq!(wrapped_delta(10.0, 59.0, 100.0), 49.0);
    }
}
