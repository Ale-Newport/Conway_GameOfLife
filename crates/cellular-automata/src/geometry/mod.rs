//! Cell rectangles, world descriptions and the drawing primitives shared by
//! every model's editing commands (brush strokes, lines, rectangles, circles,
//! pattern transforms) and symmetric random fills ([`random_fill_each`]).

mod fill;

pub use fill::{Symmetry, check_fill, random_fill, random_fill_each};

use serde::{Deserialize, Serialize};
use simulation_core::Topology;

use crate::error::CaError;

/// Largest number of cells a single drawing command (line, shape, stroke)
/// or a [`random_fill`] returning a list may touch.
pub const MAX_EDIT_CELLS: u64 = 1 << 26;
/// Largest region a streaming random fill ([`random_fill_each`]) may cover:
/// the largest finite world (and home region), 65,536 × 65,536 cells.
pub const MAX_FILL_CELLS: u64 = 1 << 32;
/// Largest side of a finite world, in cells.
pub const MAX_WORLD_SIDE: u32 = 65_536;

/// An inclusive, axis-aligned rectangle of cells `[x0, x1] × [y0, y1]`.
///
/// [`CellRect::new`] orders the corners, so `x0 <= x1` and `y0 <= y1` always
/// hold for rectangles built through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellRect {
    /// Left column (inclusive).
    pub x0: i64,
    /// Top row (inclusive).
    pub y0: i64,
    /// Right column (inclusive).
    pub x1: i64,
    /// Bottom row (inclusive).
    pub y1: i64,
}

impl CellRect {
    /// Rectangle spanning two corners given in any order.
    pub fn new(xa: i64, ya: i64, xb: i64, yb: i64) -> Self {
        Self { x0: xa.min(xb), y0: ya.min(yb), x1: xa.max(xb), y1: ya.max(yb) }
    }

    /// The single cell `(x, y)`.
    pub fn point(x: i64, y: i64) -> Self {
        Self { x0: x, y0: y, x1: x, y1: y }
    }

    /// Rectangle of `width × height` cells with top-left corner `(x, y)`.
    /// Returns `None` for an empty size or when it would leave the `i64` plane.
    pub fn from_origin_size(x: i64, y: i64, width: u64, height: u64) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let x1 = i64::try_from(i128::from(x) + i128::from(width) - 1).ok()?;
        let y1 = i64::try_from(i128::from(y) + i128::from(height) - 1).ok()?;
        Some(Self { x0: x, y0: y, x1, y1 })
    }

    /// Smallest rectangle containing every listed cell, or `None` for none.
    pub fn bounding(cells: impl IntoIterator<Item = (i64, i64)>) -> Option<Self> {
        let mut iter = cells.into_iter();
        let (x, y) = iter.next()?;
        Some(iter.fold(Self::point(x, y), |r, (x, y)| r.including(x, y)))
    }

    /// The rectangle grown to include `(x, y)`.
    pub fn including(self, x: i64, y: i64) -> Self {
        Self { x0: self.x0.min(x), y0: self.y0.min(y), x1: self.x1.max(x), y1: self.y1.max(y) }
    }

    /// Smallest rectangle containing both rectangles.
    pub fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    /// Width in cells (saturating at `u64::MAX`).
    pub fn width(&self) -> u64 {
        span(self.x0, self.x1)
    }

    /// Height in cells (saturating at `u64::MAX`).
    pub fn height(&self) -> u64 {
        span(self.y0, self.y1)
    }

    /// Number of cells (saturating at `u128::MAX`, reached only by a
    /// rectangle spanning the whole `i64` plane).
    pub fn area(&self) -> u128 {
        let w = (i128::from(self.x1) - i128::from(self.x0) + 1) as u128;
        let h = (i128::from(self.y1) - i128::from(self.y0) + 1) as u128;
        w.saturating_mul(h)
    }

    /// Whether `(x, y)` lies inside.
    pub fn contains(&self, x: i64, y: i64) -> bool {
        (self.x0..=self.x1).contains(&x) && (self.y0..=self.y1).contains(&y)
    }

    /// Overlap of two rectangles.
    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let r = Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };
        (r.x0 <= r.x1 && r.y0 <= r.y1).then_some(r)
    }
}

fn span(a: i64, b: i64) -> u64 {
    u64::try_from(i128::from(b) - i128::from(a) + 1).unwrap_or(u64::MAX)
}

/// A world: its topology and size.
///
/// Finite worlds hold the cells `0 <= x < width`, `0 <= y < height`;
/// coordinates outside are mapped with [`Topology::wrap`] (on the bounded
/// plane they do not exist). On the [`Topology::Infinite`] plane `width` and
/// `height` only describe the *home region* `[0, width) × [0, height)` used
/// for default random fills and the initial view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct World {
    /// Edge behaviour.
    pub topology: Topology,
    /// Width in cells (≥ 1).
    pub width: u32,
    /// Height in cells (≥ 1).
    pub height: u32,
}

impl World {
    /// A world after validating its size (`1..=65536` per side).
    pub fn new(topology: Topology, width: u32, height: u32) -> Result<Self, CaError> {
        if width == 0 || height == 0 || width > MAX_WORLD_SIDE || height > MAX_WORLD_SIDE {
            return Err(CaError::InvalidSize {
                width: u64::from(width),
                height: u64::from(height),
                reason: format!("each side must be between 1 and {MAX_WORLD_SIDE} cells"),
            });
        }
        Ok(Self { topology, width, height })
    }

    /// Whether the world has finite extent.
    pub fn is_finite(&self) -> bool {
        self.topology.is_finite()
    }

    /// Cells of a finite world (`width × height`); the home region otherwise.
    pub fn cell_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// The home region `[0, width) × [0, height)` (all cells of a finite world).
    pub fn home(&self) -> CellRect {
        CellRect { x0: 0, y0: 0, x1: i64::from(self.width) - 1, y1: i64::from(self.height) - 1 }
    }

    /// Bounds of a finite world, `None` for the infinite plane.
    pub fn bounds(&self) -> Option<CellRect> {
        self.is_finite().then(|| self.home())
    }

    /// Maps a coordinate onto the world through its topology (identity on
    /// the infinite plane, `None` outside a bounded plane).
    pub fn map(&self, x: i64, y: i64) -> Option<(i64, i64)> {
        self.topology.wrap(x, y, i64::from(self.width), i64::from(self.height))
    }

    /// Clips a rectangle to the world (unchanged on the infinite plane).
    pub fn clip(&self, rect: &CellRect) -> Option<CellRect> {
        match self.bounds() {
            Some(bounds) => rect.intersect(&bounds),
            None => Some(*rect),
        }
    }

    /// The cells of `rect` as the topology sees them: the rectangles of world
    /// cells that the cells of `rect` map to through [`World::map`]. The
    /// rectangle itself on the infinite plane, its clip on the bounded plane,
    /// and up to nine rectangles (one per pair of crossed periods, mirrored
    /// where an edge is twisted) on tori, Klein bottles and cross-surfaces.
    /// Editing tools clear these rectangles, so that clearing a destination
    /// and then writing cells that wrap around an edge touch the same cells.
    pub fn images(&self, rect: &CellRect) -> Vec<CellRect> {
        match self.topology {
            Topology::Infinite => vec![*rect],
            Topology::Bounded => self.clip(rect).into_iter().collect(),
            Topology::Torus | Topology::KleinBottle | Topology::CrossSurface => {
                let xs = period_pieces(rect.x0, rect.x1, i64::from(self.width));
                let ys = period_pieces(rect.y0, rect.y1, i64::from(self.height));
                let mut out = Vec::with_capacity(xs.len() * ys.len());
                for &(xa, xb) in &xs {
                    for &(ya, yb) in &ys {
                        // Inside one pair of periods the mapping is a translation with
                        // optional mirrors, so the image of the piece is the rectangle
                        // spanned by the images of two opposite corners.
                        if let (Some((ax, ay)), Some((bx, by))) = (self.map(xa, ya), self.map(xb, yb)) {
                            out.push(CellRect::new(ax, ay, bx, by));
                        }
                    }
                }
                out
            }
        }
    }
}

/// Splits `[lo, hi]` into pieces that each lie inside one period
/// `[k·n, (k + 1)·n − 1]`. A range of at least two periods is replaced by
/// two whole periods of opposite parity, which have the same images under
/// every topology (twists depend only on the parity of the period).
fn period_pieces(lo: i64, hi: i64, n: i64) -> Vec<(i64, i64)> {
    let (lo, hi, n) = (i128::from(lo), i128::from(hi), i128::from(n));
    if hi - lo + 1 >= 2 * n {
        // n ≤ 65,536, so these fit in i64.
        return vec![(0, (n - 1) as i64), (n as i64, (2 * n - 1) as i64)];
    }
    let mut pieces = Vec::with_capacity(3);
    let mut start = lo;
    while start <= hi {
        let end = ((start.div_euclid(n) + 1) * n - 1).min(hi);
        // Both ends lie between lo and hi, which are i64 values.
        pieces.push((start as i64, end as i64));
        start = end + 1;
    }
    pieces
}

/// Quarter-turn rotation applied when pasting a pattern.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    /// No rotation.
    #[default]
    None,
    /// 90° clockwise.
    Cw90,
    /// 180°.
    Half,
    /// 90° counter-clockwise.
    Ccw90,
}

impl Rotation {
    /// Number of clockwise quarter turns.
    pub fn quarter_turns(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Cw90 => 1,
            Self::Half => 2,
            Self::Ccw90 => 3,
        }
    }
}

fn check_count(count: u128, what: &str) -> Result<(), CaError> {
    if count > u128::from(MAX_EDIT_CELLS) {
        return Err(CaError::LimitExceeded(format!(
            "the {what} would touch {count} cells, more than the limit of {MAX_EDIT_CELLS}"
        )));
    }
    Ok(())
}

/// Cells of a straight line from `(x0, y0)` to `(x1, y1)` inclusive
/// (Bresenham's algorithm; 8-connected, one cell per step of the major axis).
pub fn line(x0: i64, y0: i64, x1: i64, y1: i64) -> Result<Vec<(i64, i64)>, CaError> {
    let dx = (i128::from(x1) - i128::from(x0)).abs();
    let dy = -(i128::from(y1) - i128::from(y0)).abs();
    check_count(dx.max(-dy) as u128 + 1, "line")?;
    let sx: i128 = if x0 < x1 { 1 } else { -1 };
    let sy: i128 = if y0 < y1 { 1 } else { -1 };
    let (mut x, mut y) = (i128::from(x0), i128::from(y0));
    let mut err = dx + dy;
    let mut out = Vec::with_capacity((dx.max(-dy) + 1) as usize);
    loop {
        // In range: x and y stay between the two i64 endpoints.
        out.push((x as i64, y as i64));
        if x == i128::from(x1) && y == i128::from(y1) {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    Ok(out)
}

/// Whether `(dx, dy)` lies in the disc of radius `r`: `dx² + dy² ≤ r(r + 1)`.
/// The `r(r + 1)` bound (instead of `r²`) gives rounder small discs; radius 0
/// is the single centre cell.
fn in_disc(dx: i64, dy: i64, r: i64) -> bool {
    i128::from(dx) * i128::from(dx) + i128::from(dy) * i128::from(dy) <= i128::from(r) * i128::from(r + 1)
}

/// Checks that the square `[cx − r, cx + r] × [cy − r, cy + r]` lies on the
/// 64-bit plane, so that shapes inside it never wrap or saturate.
fn check_square(cx: i64, cy: i64, r: i64, what: &str) -> Result<(), CaError> {
    let inside = |c: i64| c.checked_sub(r).is_some() && c.checked_add(r).is_some();
    if inside(cx) && inside(cy) {
        Ok(())
    } else {
        Err(CaError::invalid(format!("the {what} would leave the 64-bit coordinate plane")))
    }
}

/// Cells of a filled disc of radius `radius` centred on `(cx, cy)`.
///
/// # Errors
/// More than [`MAX_EDIT_CELLS`] cells, or a disc leaving the 64-bit plane.
pub fn disc(cx: i64, cy: i64, radius: u32) -> Result<Vec<(i64, i64)>, CaError> {
    let r = i64::from(radius);
    check_count((2 * u128::from(radius) + 1).pow(2), "disc")?;
    check_square(cx, cy, r, "disc")?;
    let mut out = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            if in_disc(dx, dy, r) {
                out.push((cx + dx, cy + dy));
            }
        }
    }
    Ok(out)
}

/// Cells of a circle outline: the cells of [`disc`] that have at least one
/// orthogonal neighbour outside the disc (a closed, 4-connected ring).
///
/// # Errors
/// As [`disc`].
pub fn circle_outline(cx: i64, cy: i64, radius: u32) -> Result<Vec<(i64, i64)>, CaError> {
    let r = i64::from(radius);
    check_count((2 * u128::from(radius) + 1).pow(2), "circle")?;
    check_square(cx, cy, r, "circle")?;
    let mut out = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(ex, ey)| !in_disc(dx + ex, dy + ey, r));
            if in_disc(dx, dy, r) && edge {
                out.push((cx + dx, cy + dy));
            }
        }
    }
    Ok(out)
}

/// Cells of a rectangle, filled or as a one-cell-wide outline.
pub fn rectangle(rect: CellRect, filled: bool) -> Result<Vec<(i64, i64)>, CaError> {
    let count = if filled { rect.area() } else { 2 * u128::from(rect.width()) + 2 * u128::from(rect.height()) };
    check_count(count, "rectangle")?;
    let mut out = Vec::new();
    for y in rect.y0..=rect.y1 {
        if filled || y == rect.y0 || y == rect.y1 {
            out.extend((rect.x0..=rect.x1).map(|x| (x, y)));
        } else {
            out.push((rect.x0, y));
            if rect.x1 != rect.x0 {
                out.push((rect.x1, y));
            }
        }
    }
    Ok(out)
}

/// Cells of a brush stroke: discs of `radius` along straight segments joining
/// consecutive points (a single point paints one disc). Sorted row-major,
/// without duplicates.
///
/// # Errors
/// More than [`MAX_EDIT_CELLS`] cells, or a brush leaving the 64-bit plane.
pub fn stroke(points: &[(i64, i64)], radius: u32) -> Result<Vec<(i64, i64)>, CaError> {
    let Some(&first) = points.first() else {
        return Ok(Vec::new());
    };
    let mut centres = vec![first];
    for pair in points.windows(2) {
        let segment = line(pair[0].0, pair[0].1, pair[1].0, pair[1].1)?;
        centres.extend(segment.into_iter().skip(1));
        check_count(centres.len() as u128 * (2 * u128::from(radius) + 1).pow(2), "brush stroke")?;
    }
    for &(x, y) in points {
        check_square(x, y, i64::from(radius), "brush stroke")?;
    }
    let brush = disc(0, 0, radius)?;
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(centres.len() * brush.len());
    for (cx, cy) in centres {
        // Centres lie between stroke points, whose brush squares were checked.
        out.extend(brush.iter().map(|&(dx, dy)| (cx + dx, cy + dy)));
    }
    sort_row_major(&mut out);
    Ok(out)
}

/// Sorts cells by row, then column, and removes duplicates.
pub fn sort_row_major(cells: &mut Vec<(i64, i64)>) {
    cells.sort_unstable_by_key(|&(x, y)| (y, x));
    cells.dedup();
}

/// Applies a quarter-turn rotation (clockwise) and optional left-right mirror
/// (applied first) to the cells of `rect`, keeping the top-left corner of the
/// rectangle fixed. Returns the transformed cells and the transformed
/// rectangle (its width and height swap for odd quarter turns).
pub fn transform_in_rect(
    cells: &[(i64, i64)],
    rect: CellRect,
    rotation: Rotation,
    mirror: bool,
) -> (Vec<(i64, i64)>, CellRect) {
    let (w, h) = (rect.width() as i128, rect.height() as i128);
    let turns = rotation.quarter_turns();
    let mut out = Vec::with_capacity(cells.len());
    for &(x, y) in cells {
        let mut u = i128::from(x) - i128::from(rect.x0);
        let v = i128::from(y) - i128::from(rect.y0);
        if mirror {
            u = w - 1 - u;
        }
        let (mut a, mut b, mut cw, mut ch) = (u, v, w, h);
        for _ in 0..turns {
            // Clockwise quarter turn inside a cw × ch box: (a, b) -> (ch-1-b, a).
            (a, b) = (ch - 1 - b, a);
            std::mem::swap(&mut cw, &mut ch);
        }
        let sat = |v: i128| v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
        out.push((sat(i128::from(rect.x0) + a), sat(i128::from(rect.y0) + b)));
    }
    let (nw, nh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    let sat = |v: i128| v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
    let new_rect = CellRect {
        x0: rect.x0,
        y0: rect.y0,
        x1: sat(i128::from(rect.x0) + nw - 1),
        y1: sat(i128::from(rect.y0) + nh - 1),
    };
    sort_row_major(&mut out);
    (out, new_rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_basics() {
        let r = CellRect::new(5, 7, -1, 2);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (-1, 2, 5, 7));
        assert_eq!((r.width(), r.height(), r.area()), (7, 6, 42));
        assert!(r.contains(0, 2) && !r.contains(6, 2));
        assert_eq!(r.intersect(&CellRect::new(4, 0, 10, 3)), Some(CellRect::new(4, 2, 5, 3)));
        assert_eq!(r.intersect(&CellRect::new(6, 0, 10, 3)), None);
        let full = CellRect::new(i64::MIN, i64::MIN, i64::MAX, i64::MAX);
        assert_eq!(full.width(), u64::MAX);
        assert_eq!(full.area(), u128::MAX);
    }

    #[test]
    fn world_validation_and_mapping() {
        assert!(World::new(Topology::Torus, 0, 4).is_err());
        assert!(World::new(Topology::Torus, 4, MAX_WORLD_SIDE + 1).is_err());
        let w = World::new(Topology::Torus, 10, 5).unwrap();
        assert_eq!(w.map(-1, 5), Some((9, 0)));
        let b = World::new(Topology::Bounded, 10, 5).unwrap();
        assert_eq!(b.map(-1, 0), None);
        assert_eq!(b.clip(&CellRect::new(-5, -5, 3, 3)), Some(CellRect::new(0, 0, 3, 3)));
    }

    #[test]
    fn line_is_symmetric_and_connected() {
        let l = line(0, 0, 5, 2).unwrap();
        assert_eq!(l.first(), Some(&(0, 0)));
        assert_eq!(l.last(), Some(&(5, 2)));
        assert_eq!(l.len(), 6);
        for pair in l.windows(2) {
            assert!((pair[0].0 - pair[1].0).abs() <= 1 && (pair[0].1 - pair[1].1).abs() <= 1);
        }
        assert_eq!(line(3, 3, 3, 3).unwrap(), vec![(3, 3)]);
        assert!(line(0, 0, 1 << 40, 0).is_err());
    }

    #[test]
    fn discs_and_outlines() {
        assert_eq!(disc(0, 0, 0).unwrap(), vec![(0, 0)]);
        assert_eq!(disc(0, 0, 1).unwrap().len(), 9);
        let d = disc(0, 0, 3).unwrap();
        let ring = circle_outline(0, 0, 3).unwrap();
        assert!(ring.iter().all(|c| d.contains(c)));
        assert!(!ring.contains(&(0, 0)));
        assert!(ring.contains(&(3, 0)) && ring.contains(&(0, -3)));
        let outline = rectangle(CellRect::new(0, 0, 3, 2), false).unwrap();
        assert_eq!(outline.len(), 10);
        assert_eq!(rectangle(CellRect::new(0, 0, 3, 2), true).unwrap().len(), 12);
        // Shapes never saturate at the edge of the 64-bit plane: they are refused.
        assert!(disc(i64::MAX, 0, 1).is_err() && circle_outline(0, i64::MIN, 2).is_err());
        assert!(stroke(&[(0, 0), (i64::MAX - 1, 0)], 3).is_err());
        assert_eq!(disc(i64::MAX - 1, 0, 1).unwrap().len(), 9);
    }

    #[test]
    fn stroke_joins_points() {
        let s = stroke(&[(0, 0), (4, 0)], 0).unwrap();
        assert_eq!(s, vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);
        let thick = stroke(&[(0, 0), (4, 0)], 1).unwrap();
        assert_eq!(thick.len(), 3 * 7);
    }

    #[test]
    fn images_follow_the_topology() {
        let cells = |world: &World, rect: CellRect| -> std::collections::BTreeSet<(i64, i64)> {
            let mut set = std::collections::BTreeSet::new();
            for y in rect.y0..=rect.y1 {
                for x in rect.x0..=rect.x1 {
                    if let Some(c) = world.map(x, y) {
                        set.insert(c);
                    }
                }
            }
            set
        };
        let rects = [
            CellRect::new(-3, -2, 4, 1),
            CellRect::new(8, 3, 13, 9),
            CellRect::new(-12, -1, 30, 0),
            CellRect::new(2, 2, 3, 3),
        ];
        for topology in [Topology::Bounded, Topology::Torus, Topology::KleinBottle, Topology::CrossSurface] {
            let world = World::new(topology, 10, 7).unwrap();
            for rect in rects {
                let mut covered = std::collections::BTreeSet::new();
                for image in world.images(&rect) {
                    assert!(world.clip(&image) == Some(image), "{topology:?}: images lie in the world");
                    covered.extend(cells(&world, image));
                }
                assert_eq!(covered, cells(&world, rect), "{topology:?} {rect:?}");
            }
        }
        let plane = World::new(Topology::Infinite, 4, 4).unwrap();
        assert_eq!(plane.images(&rects[0]), vec![rects[0]]);
    }

    #[test]
    fn transform_keeps_top_left() {
        let rect = CellRect::new(10, 20, 12, 21); // 3 wide, 2 high
        let cells = vec![(10, 20), (12, 21)];
        let (rotated, r) = transform_in_rect(&cells, rect, Rotation::Cw90, false);
        assert_eq!(r, CellRect::new(10, 20, 11, 22));
        assert_eq!(rotated, vec![(11, 20), (10, 22)]);
        let (mirrored, r2) = transform_in_rect(&cells, rect, Rotation::None, true);
        assert_eq!(r2, rect);
        assert_eq!(mirrored, vec![(12, 20), (10, 21)]);
        let (back, _) = transform_in_rect(&rotated, r, Rotation::Ccw90, false);
        assert_eq!(back, vec![(10, 20), (12, 21)]);
    }
}
