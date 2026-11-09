//! Dense 2D buffers and world topologies.

use serde::{Deserialize, Serialize};

/// How the edges of a finite world connect. Orientation conventions follow
/// Golly's bounded grids: the Klein bottle joins the left/right edges directly
/// and the top/bottom edges with a horizontal flip; the cross-surface (real
/// projective plane) twists both pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Topology {
    /// Finite plane; cells outside the world are permanently dead/empty.
    #[default]
    Bounded,
    /// Opposite edges joined (doughnut).
    Torus,
    /// Left/right joined; top/bottom joined with a horizontal flip.
    KleinBottle,
    /// Both pairs of edges joined with a flip.
    CrossSurface,
    /// Logically unbounded plane (sparse storage); no wrapping.
    Infinite,
}

impl Topology {
    /// All variants, in display order.
    pub const ALL: [Topology; 5] = [Self::Bounded, Self::Torus, Self::KleinBottle, Self::CrossSurface, Self::Infinite];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bounded => "Bounded (dead outside)",
            Self::Torus => "Torus (wrap-around)",
            Self::KleinBottle => "Klein bottle",
            Self::CrossSurface => "Cross-surface (projective plane)",
            Self::Infinite => "Infinite (sparse)",
        }
    }

    /// Whether the world has finite dimensions.
    pub fn is_finite(self) -> bool {
        !matches!(self, Self::Infinite)
    }

    /// Maps a possibly out-of-range coordinate onto the world. Returns `None`
    /// for coordinates outside a bounded world. For [`Topology::Infinite`] the
    /// coordinate is returned unchanged.
    pub fn wrap(self, x: i64, y: i64, width: i64, height: i64) -> Option<(i64, i64)> {
        match self {
            Self::Infinite => Some((x, y)),
            Self::Bounded => (x >= 0 && y >= 0 && x < width && y < height).then_some((x, y)),
            Self::Torus => Some((x.rem_euclid(width), y.rem_euclid(height))),
            Self::KleinBottle => {
                let flip_x = y.div_euclid(height).rem_euclid(2) == 1;
                let wx = x.rem_euclid(width);
                let wx = if flip_x { width - 1 - wx } else { wx };
                Some((wx, y.rem_euclid(height)))
            }
            Self::CrossSurface => {
                let flip_x = y.div_euclid(height).rem_euclid(2) == 1;
                let flip_y = x.div_euclid(width).rem_euclid(2) == 1;
                let mut wx = x.rem_euclid(width);
                let mut wy = y.rem_euclid(height);
                if flip_x {
                    wx = width - 1 - wx;
                }
                if flip_y {
                    wy = height - 1 - wy;
                }
                Some((wx, wy))
            }
        }
    }
}

/// Dense row-major 2D buffer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grid2<T> {
    width: usize,
    height: usize,
    data: Vec<T>,
}

impl<T: Clone> Grid2<T> {
    /// Grid filled with `value`.
    pub fn new(width: usize, height: usize, value: T) -> Self {
        Self { width, height, data: vec![value; width * height] }
    }

    /// Sets every cell to `value`.
    pub fn fill(&mut self, value: T) {
        self.data.fill(value);
    }
}

impl<T> Grid2<T> {
    /// Wraps an existing buffer.
    ///
    /// # Panics
    /// Panics when `data.len() != width * height`.
    pub fn from_vec(width: usize, height: usize, data: Vec<T>) -> Self {
        assert_eq!(data.len(), width * height, "buffer size does not match dimensions");
        Self { width, height, data }
    }

    /// Grid built by evaluating `f(x, y)` for every cell.
    pub fn from_fn(width: usize, height: usize, mut f: impl FnMut(usize, usize) -> T) -> Self {
        let mut data = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                data.push(f(x, y));
            }
        }
        Self { width, height, data }
    }

    /// Width in cells.
    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in cells.
    #[inline]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Linear index of `(x, y)`.
    #[inline]
    pub fn index(&self, x: usize, y: usize) -> usize {
        debug_assert!(x < self.width && y < self.height);
        y * self.width + x
    }

    /// Whether `(x, y)` lies inside the grid.
    #[inline]
    pub fn contains(&self, x: i64, y: i64) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height
    }

    /// Cell reference.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> &T {
        &self.data[self.index(x, y)]
    }

    /// Mutable cell reference.
    #[inline]
    pub fn get_mut(&mut self, x: usize, y: usize) -> &mut T {
        let i = self.index(x, y);
        &mut self.data[i]
    }

    /// Checked lookup with signed coordinates.
    #[inline]
    pub fn try_get(&self, x: i64, y: i64) -> Option<&T> {
        self.contains(x, y).then(|| self.get(x as usize, y as usize))
    }

    /// Writes a cell.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, value: T) {
        let i = self.index(x, y);
        self.data[i] = value;
    }

    /// Row-major cell slice.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// Mutable row-major cell slice.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// One row.
    #[inline]
    pub fn row(&self, y: usize) -> &[T] {
        &self.data[y * self.width..(y + 1) * self.width]
    }

    /// Iterates over `(x, y, &value)`.
    pub fn iter(&self) -> impl Iterator<Item = (usize, usize, &T)> {
        let w = self.width.max(1);
        self.data.iter().enumerate().map(move |(i, v)| (i % w, i / w, v))
    }

    /// Consumes the grid and returns its buffer.
    pub fn into_vec(self) -> Vec<T> {
        self.data
    }
}

impl Grid2<f32> {
    /// Bilinear sample at continuous coordinates (cell centres at integer +
    /// 0.5), clamped to the grid.
    pub fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let fx = (x - 0.5).clamp(0.0, (self.width - 1) as f32);
        let fy = (y - 0.5).clamp(0.0, (self.height - 1) as f32);
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let top = self.get(x0, y0) * (1.0 - tx) + self.get(x1, y0) * tx;
        let bottom = self.get(x0, y1) * (1.0 - tx) + self.get(x1, y1) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    /// Sum of all cells (f64 accumulation).
    pub fn sum(&self) -> f64 {
        self.data.iter().map(|&v| f64::from(v)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_and_torus() {
        assert_eq!(Topology::Bounded.wrap(-1, 0, 10, 10), None);
        assert_eq!(Topology::Bounded.wrap(3, 4, 10, 10), Some((3, 4)));
        assert_eq!(Topology::Torus.wrap(-1, 10, 10, 10), Some((9, 0)));
        assert_eq!(Topology::Torus.wrap(25, -13, 10, 10), Some((5, 7)));
        assert_eq!(Topology::Infinite.wrap(-100, 7, 1, 1), Some((-100, 7)));
    }

    #[test]
    fn klein_bottle_flips_on_vertical_crossing() {
        // Crossing the top edge mirrors x.
        assert_eq!(Topology::KleinBottle.wrap(2, -1, 10, 8), Some((7, 7)));
        // Crossing left/right does not.
        assert_eq!(Topology::KleinBottle.wrap(-1, 3, 10, 8), Some((9, 3)));
        // Crossing twice restores orientation.
        assert_eq!(Topology::KleinBottle.wrap(2, 16, 10, 8), Some((2, 0)));
    }

    #[test]
    fn cross_surface_flips_both() {
        assert_eq!(Topology::CrossSurface.wrap(-1, 2, 10, 8), Some((9, 5)));
        assert_eq!(Topology::CrossSurface.wrap(3, 8, 10, 8), Some((6, 0)));
    }

    #[test]
    fn grid_access_and_sampling() {
        let mut g = Grid2::new(4, 3, 0.0f32);
        g.set(1, 1, 1.0);
        assert_eq!(*g.get(1, 1), 1.0);
        assert_eq!(g.try_get(-1, 0), None);
        assert!((g.sample_bilinear(1.5, 1.5) - 1.0).abs() < 1e-6);
        assert!((g.sample_bilinear(2.0, 1.5) - 0.5).abs() < 1e-6);
        assert_eq!(g.iter().filter(|(_, _, v)| **v > 0.0).count(), 1);
    }
}
