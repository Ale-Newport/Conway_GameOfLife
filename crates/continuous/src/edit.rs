//! World editing primitives shared by commands and scenario initialisation:
//! creature cell data and its placement with rotation and scale, the soft
//! brush, and random soups.
//!
//! Coordinates are in cells on a torus: `x` grows to the right, `y` downward,
//! cell `(i, j)` covers `[i, i+1) × [j, j+1)` and its centre is `(i + ½, j + ½)`.

use serde::{Deserialize, Serialize};
use simulation_core::{Grid2, SimRng};

use crate::error::ContinuousError;
use crate::rle;
use crate::stats::wrapped_delta;

/// Largest side of creature cell data.
pub const MAX_CREATURE_SIDE: usize = 1024;

/// Largest number of discs [`random_discs`] draws in one call.
pub const MAX_DISCS: u32 = 100_000;

/// Cell data of a creature as written in scenario files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CellData {
    /// One Chan-format RLE string per channel (8-bit states).
    Rle(Vec<String>),
    /// Explicit values per channel: `channels[c][row][column]`.
    Rows(Vec<Vec<Vec<f32>>>),
}

impl CellData {
    /// Decodes and validates the cell data.
    pub fn decode(&self) -> Result<CreatureCells, ContinuousError> {
        let planes = match self {
            Self::Rle(channels) => channels.iter().map(|text| rle::decode(text)).collect::<Result<Vec<_>, _>>()?,
            Self::Rows(channels) => channels
                .iter()
                .enumerate()
                .map(|(c, rows)| {
                    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
                    if width == 0 || rows.len() > MAX_CREATURE_SIDE || width > MAX_CREATURE_SIDE {
                        return Err(ContinuousError::InvalidCells(format!(
                            "channel {c} must have between 1 and {MAX_CREATURE_SIDE} rows and columns"
                        )));
                    }
                    Ok(Grid2::from_fn(width, rows.len(), |x, y| rows[y].get(x).copied().unwrap_or(0.0)))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        CreatureCells::from_planes(planes)
    }
}

/// Decoded, validated creature cells: one plane per channel, all the same
/// size, values finite and non-negative.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreatureCells {
    width: usize,
    height: usize,
    planes: Vec<Grid2<f32>>,
}

impl CreatureCells {
    /// Builds creature cells from planes, padding narrower planes with zeros.
    pub fn from_planes(planes: Vec<Grid2<f32>>) -> Result<Self, ContinuousError> {
        if planes.is_empty() {
            return Err(ContinuousError::InvalidCells("at least one channel is required".into()));
        }
        let width = planes.iter().map(Grid2::width).max().unwrap_or(0);
        let height = planes.iter().map(Grid2::height).max().unwrap_or(0);
        if width == 0 || height == 0 || width > MAX_CREATURE_SIDE || height > MAX_CREATURE_SIDE {
            return Err(ContinuousError::InvalidCells(format!(
                "creature size {width}×{height} is outside 1…{MAX_CREATURE_SIDE}"
            )));
        }
        if let Some(v) = planes.iter().flat_map(|p| p.as_slice()).find(|v| !v.is_finite() || **v < 0.0) {
            return Err(ContinuousError::InvalidCells(format!("cell value {v} is not a finite non-negative number")));
        }
        let planes = planes
            .into_iter()
            .map(|p| {
                if p.width() == width && p.height() == height {
                    p
                } else {
                    Grid2::from_fn(width, height, |x, y| p.try_get(x as i64, y as i64).copied().unwrap_or(0.0))
                }
            })
            .collect();
        Ok(Self { width, height, planes })
    }

    /// Width in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in cells.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Number of channels.
    pub fn channels(&self) -> usize {
        self.planes.len()
    }

    /// One channel's values.
    pub fn plane(&self, channel: usize) -> Option<&Grid2<f32>> {
        self.planes.get(channel)
    }

    /// Total of all values.
    pub fn mass(&self) -> f64 {
        self.planes.iter().map(Grid2::sum).sum()
    }
}

/// How placed values combine with the existing world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    /// Overwrite the cells covered by the creature's (rotated, scaled) box.
    #[default]
    Replace,
    /// Add to the existing values.
    Add,
}

/// Position, orientation and size of a placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Centre column (cells, may be fractional; wraps).
    pub x: f32,
    /// Centre row (cells, may be fractional; wraps).
    pub y: f32,
    /// Rotation in degrees, clockwise on screen (y grows downward).
    pub rotation_deg: f32,
    /// Scale factor (> 0); values are resampled bilinearly.
    pub scale: f32,
    /// Combination with existing values.
    pub blend: Blend,
}

impl Placement {
    fn validate(&self) -> Result<(), ContinuousError> {
        if !(self.x.is_finite() && self.y.is_finite() && self.rotation_deg.is_finite()) {
            return Err(ContinuousError::command("placement position and rotation must be finite"));
        }
        if !(self.scale.is_finite() && (0.05..=32.0).contains(&self.scale)) {
            return Err(ContinuousError::command(format!("scale {} is outside [0.05, 32]", self.scale)));
        }
        Ok(())
    }
}

/// Writes `cells` into `world` (one grid per channel; creature channel `c`
/// goes to world channel `c`). Values are clamped to `max_value` when given.
///
/// With rotation 0 and scale 1 the cells are copied exactly, with the
/// top-left corner snapped to the nearest cell. Otherwise each covered world
/// cell samples the creature bilinearly at the inverse-transformed position
/// of its centre (values outside the creature are 0), so the total mass
/// scales with `scale²`.
///
/// The placed creature must fit the torus (its rotated, scaled bounding box
/// must be smaller than the world); otherwise it would overlap itself and
/// [`Blend::Add`] would count cells twice, so the placement is refused.
pub fn place(
    world: &mut [Grid2<f32>],
    cells: &CreatureCells,
    at: &Placement,
    max_value: Option<f32>,
) -> Result<(), ContinuousError> {
    at.validate()?;
    if cells.channels() > world.len() {
        return Err(ContinuousError::command(format!(
            "the creature has {} channels but the world has {}",
            cells.channels(),
            world.len()
        )));
    }
    let (ww, wh) = (world[0].width() as i64, world[0].height() as i64);
    let (cw, ch) = (cells.width as f32, cells.height as f32);
    let exact = at.rotation_deg == 0.0 && at.scale == 1.0;
    let (sin, cos) = at.rotation_deg.to_radians().sin_cos();
    let (extent_x, extent_y) = if exact {
        (cw, ch)
    } else {
        ((cos.abs() * cw + sin.abs() * ch) * at.scale, (sin.abs() * cw + cos.abs() * ch) * at.scale)
    };
    // Exact copies may fill the world; resampled ones must stay strictly
    // smaller so that no world cell is covered by two images of the creature.
    let fits = if exact {
        extent_x <= ww as f32 && extent_y <= wh as f32
    } else {
        extent_x < ww as f32 && extent_y < wh as f32
    };
    if !fits {
        return Err(ContinuousError::command(format!(
            "the placed creature ({extent_x:.0}×{extent_y:.0} cells after rotation and scaling) does not fit \
             the {ww}×{wh} world"
        )));
    }
    // Wrap the centre into the world first so that any finite position works
    // (and integer cell indices cannot overflow).
    let (px, py) = (at.x.rem_euclid(ww as f32), at.y.rem_euclid(wh as f32));
    let write = |grid: &mut Grid2<f32>, x: i64, y: i64, v: f32, blend: Blend| {
        let (x, y) = (x.rem_euclid(ww) as usize, y.rem_euclid(wh) as usize);
        let old = *grid.get(x, y);
        let new = match blend {
            Blend::Replace => v,
            Blend::Add => old + v,
        };
        grid.set(x, y, max_value.map_or(new, |m| new.min(m)));
    };
    if exact {
        let x0 = (px - cw / 2.0).round() as i64;
        let y0 = (py - ch / 2.0).round() as i64;
        for (plane, grid) in cells.planes.iter().zip(world.iter_mut()) {
            for (cx, cy, &v) in plane.iter() {
                write(grid, x0 + cx as i64, y0 + cy as i64, v, at.blend);
            }
        }
        return Ok(());
    }
    let half_diag = 0.5 * (cw * cw + ch * ch).sqrt() * at.scale;
    let (xmin, xmax) = ((px - half_diag).floor() as i64, (px + half_diag).ceil() as i64);
    let (ymin, ymax) = ((py - half_diag).floor() as i64, (py + half_diag).ceil() as i64);
    for ty in ymin..=ymax {
        for tx in xmin..=xmax {
            // World offset of the cell centre from the placement centre,
            // rotated back (screen-clockwise rotation with y downward) and unscaled.
            let ox = tx as f32 + 0.5 - px;
            let oy = ty as f32 + 0.5 - py;
            let u = (cos * ox + sin * oy) / at.scale + cw / 2.0;
            let v = (-sin * ox + cos * oy) / at.scale + ch / 2.0;
            if !(0.0..=cw).contains(&u) || !(0.0..=ch).contains(&v) {
                continue;
            }
            for (plane, grid) in cells.planes.iter().zip(world.iter_mut()) {
                write(grid, tx, ty, sample_zero_outside(plane, u, v), at.blend);
            }
        }
    }
    Ok(())
}

/// Bilinear sample at continuous position `(u, v)` (cell centres at
/// integer + ½), treating cells outside the grid as zero.
fn sample_zero_outside(grid: &Grid2<f32>, u: f32, v: f32) -> f32 {
    let fx = u - 0.5;
    let fy = v - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let at = |x: f32, y: f32| grid.try_get(x as i64, y as i64).copied().unwrap_or(0.0);
    let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1.0, y0) * tx;
    let bottom = at(x0, y0 + 1.0) * (1.0 - tx) + at(x0 + 1.0, y0 + 1.0) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// What a brush stroke does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushMode {
    /// Blend the cells towards the brush value.
    #[default]
    Paint,
    /// Blend the cells towards zero.
    Erase,
}

/// A soft circular brush dab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brush {
    /// Centre column (cells).
    pub x: f32,
    /// Centre row (cells).
    pub y: f32,
    /// Radius in cells (> 0).
    pub radius: f32,
    /// Target value when painting.
    pub value: f32,
    /// Fraction of the radius over which the effect fades out (0 = hard edge,
    /// 1 = fades from the centre).
    pub softness: f32,
    /// Paint or erase.
    pub mode: BrushMode,
    /// Channel to edit, or every channel when `None`.
    pub channel: Option<usize>,
}

impl Brush {
    /// Brush weight at distance `d`: 1 inside `(1 − softness)·radius`, then
    /// a smoothstep down to 0 at `radius`.
    pub fn weight(&self, d: f32) -> f32 {
        let inner = (1.0 - self.softness) * self.radius;
        if d <= inner {
            1.0
        } else if d >= self.radius {
            0.0
        } else {
            let t = (self.radius - d) / (self.radius - inner);
            t * t * (3.0 - 2.0 * t)
        }
    }
}

/// Applies a brush dab: `A ← A + w·(value − A)` (paint) or `A ← A·(1 − w)`
/// (erase), with `w` from [`Brush::weight`] at the shortest distance on the
/// torus between the brush centre and the cell centre. Every world cell is
/// edited at most once, even when the brush is larger than the world.
/// `max_value` bounds the value.
pub fn apply_brush(world: &mut [Grid2<f32>], brush: &Brush, max_value: f32) -> Result<(), ContinuousError> {
    if !(brush.x.is_finite() && brush.y.is_finite()) {
        return Err(ContinuousError::command("brush position must be finite"));
    }
    if !(brush.radius.is_finite() && brush.radius > 0.0 && brush.radius <= 2048.0) {
        return Err(ContinuousError::command(format!("brush radius {} is outside (0, 2048]", brush.radius)));
    }
    if !(brush.softness.is_finite() && (0.0..=1.0).contains(&brush.softness)) {
        return Err(ContinuousError::command(format!("brush softness {} is outside [0, 1]", brush.softness)));
    }
    if !(brush.value.is_finite() && (0.0..=max_value).contains(&brush.value)) {
        return Err(ContinuousError::command(format!("brush value {} is outside [0, {max_value}]", brush.value)));
    }
    let channels = channel_range(brush.channel, world.len())?;
    let columns = axis_cells(brush.x, brush.radius, world[0].width());
    let rows = axis_cells(brush.y, brush.radius, world[0].height());
    for &(y, oy) in &rows {
        for &(x, ox) in &columns {
            let w = brush.weight(ox.hypot(oy));
            if w <= 0.0 {
                continue;
            }
            for c in channels.clone() {
                let a = world[c].get_mut(x, y);
                *a = match brush.mode {
                    BrushMode::Paint => *a + w * (brush.value - *a),
                    BrushMode::Erase => *a * (1.0 - w),
                };
            }
        }
    }
    Ok(())
}

/// World indices along one axis of `n` cells that cover every cell centre
/// within `radius` of `centre` (each index at most once; a few cells just
/// outside the radius are included and receive weight 0), with the shortest
/// signed offset of the cell centre from `centre` on the circle.
fn axis_cells(centre: f32, radius: f32, n: usize) -> Vec<(usize, f32)> {
    let c = f64::from(centre).rem_euclid(n as f64);
    let offset = |i: usize| wrapped_delta(c, i as f64 + 0.5, n as f64) as f32;
    let first = (c - f64::from(radius) - 0.5).floor() as i64;
    let last = (c + f64::from(radius) - 0.5).ceil() as i64;
    if last - first + 1 >= n as i64 {
        return (0..n).map(|i| (i, offset(i))).collect();
    }
    (first..=last)
        .map(|t| {
            let i = t.rem_euclid(n as i64) as usize;
            (i, offset(i))
        })
        .collect()
}

fn channel_range(channel: Option<usize>, channels: usize) -> Result<std::ops::Range<usize>, ContinuousError> {
    match channel {
        None => Ok(0..channels),
        Some(c) if c < channels => Ok(c..c + 1),
        Some(c) => Err(ContinuousError::command(format!("channel {c} does not exist (the world has {channels})"))),
    }
}

/// Region of the world, in cells.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Region {
    /// Axis-aligned rectangle with top-left corner `(x, y)`.
    Rect {
        /// Left column.
        x: f32,
        /// Top row.
        y: f32,
        /// Width in cells.
        width: f32,
        /// Height in cells.
        height: f32,
    },
    /// Disc of the given radius around `(x, y)`.
    Disc {
        /// Centre column.
        x: f32,
        /// Centre row.
        y: f32,
        /// Radius in cells.
        radius: f32,
    },
}

impl Region {
    /// Integer bounding box `(x0, y0, x1, y1)` (exclusive end) of the cells
    /// whose centres lie inside the region, and the membership test.
    fn cells(&self) -> Result<(i64, i64, i64, i64), ContinuousError> {
        let (x0, y0, x1, y1) = match *self {
            Region::Rect { x, y, width, height } => {
                if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
                    return Err(ContinuousError::command("region width and height must be positive"));
                }
                (x, y, x + width, y + height)
            }
            Region::Disc { x, y, radius } => {
                if !(radius.is_finite() && radius > 0.0) {
                    return Err(ContinuousError::command("region radius must be positive"));
                }
                (x - radius, y - radius, x + radius, y + radius)
            }
        };
        if ![x0, y0, x1, y1].iter().all(|v| v.is_finite() && v.abs() < 1.0e7) {
            return Err(ContinuousError::command("region coordinates must be finite and within ±10⁷"));
        }
        Ok((x0.floor() as i64, y0.floor() as i64, x1.ceil() as i64, y1.ceil() as i64))
    }

    fn contains(&self, cx: f32, cy: f32) -> bool {
        match *self {
            Region::Rect { x, y, width, height } => cx >= x && cx < x + width && cy >= y && cy < y + height,
            Region::Disc { x, y, radius } => (cx - x).powi(2) + (cy - y).powi(2) <= radius * radius,
        }
    }
}

/// Replaces the cells of `region` (in every channel) with random matter:
/// each cell and channel independently becomes `amplitude · U(0, 1)` with
/// probability `density`, and 0 otherwise. Cells are visited in row-major
/// order of the region and channels in order, so the result is a pure
/// function of the generator state. A region larger than the world wraps
/// and later cells overwrite earlier ones.
pub fn randomize(
    world: &mut [Grid2<f32>],
    region: &Region,
    density: f32,
    amplitude: f32,
    rng: &mut SimRng,
) -> Result<(), ContinuousError> {
    if !(density.is_finite() && (0.0..=1.0).contains(&density)) {
        return Err(ContinuousError::command(format!("density {density} is outside [0, 1]")));
    }
    if !(amplitude.is_finite() && amplitude >= 0.0) {
        return Err(ContinuousError::command(format!("amplitude {amplitude} must be a non-negative number")));
    }
    let (x0, y0, x1, y1) = region.cells()?;
    let (ww, wh) = (world[0].width() as i64, world[0].height() as i64);
    if (x1 - x0) > 4 * ww || (y1 - y0) > 4 * wh {
        return Err(ContinuousError::command("region is more than four times larger than the world"));
    }
    for ty in y0..y1 {
        for tx in x0..x1 {
            if !region.contains(tx as f32 + 0.5, ty as f32 + 0.5) {
                continue;
            }
            let (x, y) = (tx.rem_euclid(ww) as usize, ty.rem_euclid(wh) as usize);
            for grid in world.iter_mut() {
                let v = if rng.chance(f64::from(density)) { amplitude * rng.f32() } else { 0.0 };
                grid.set(x, y, v);
            }
        }
    }
    Ok(())
}

/// Draws `count` filled discs of value `value` with radii uniform in
/// `[min_radius, max_radius]` at uniformly random positions (Rafler's
/// SmoothLife soup; the research reproduction found gliders emerge from
/// about `N²/(2r_a²)` discs with radii between `r_i` and `r_a`).
pub fn random_discs(
    world: &mut [Grid2<f32>],
    count: u32,
    min_radius: f32,
    max_radius: f32,
    value: f32,
    rng: &mut SimRng,
) -> Result<(), ContinuousError> {
    if !(min_radius.is_finite() && max_radius.is_finite() && 0.0 < min_radius && min_radius <= max_radius) {
        return Err(ContinuousError::command("disc radii must satisfy 0 < min ≤ max"));
    }
    if count > MAX_DISCS {
        return Err(ContinuousError::command(format!("at most {MAX_DISCS} discs can be drawn")));
    }
    let (w, h) = (world[0].width() as f32, world[0].height() as f32);
    for _ in 0..count {
        let x = rng.range_f32(0.0, w);
        let y = rng.range_f32(0.0, h);
        let radius = rng.range_f32(min_radius, max_radius);
        let region = Region::Disc { x, y, radius };
        let (x0, y0, x1, y1) = region.cells()?;
        let (ww, wh) = (world[0].width() as i64, world[0].height() as i64);
        for ty in y0..y1 {
            for tx in x0..x1 {
                if region.contains(tx as f32 + 0.5, ty as f32 + 0.5) {
                    let (cx, cy) = (tx.rem_euclid(ww) as usize, ty.rem_euclid(wh) as usize);
                    for grid in world.iter_mut() {
                        grid.set(cx, cy, value);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(w: usize, h: usize, channels: usize) -> Vec<Grid2<f32>> {
        vec![Grid2::new(w, h, 0.0f32); channels]
    }

    fn sample_cells() -> CreatureCells {
        CellData::Rows(vec![vec![vec![0.0, 0.5, 1.0], vec![0.25, 0.75, 0.0]]]).decode().expect("valid cells")
    }

    #[test]
    fn exact_placement_copies_and_wraps() {
        let mut w = world(8, 8, 1);
        let cells = sample_cells();
        let at = Placement { x: 0.5, y: 0.0, rotation_deg: 0.0, scale: 1.0, blend: Blend::Replace };
        place(&mut w, &cells, &at, Some(1.0)).expect("placed");
        // top-left = round(0.5 − 1.5, 0 − 1) = (−1, −1) → wraps to (7, 7)
        assert_eq!(*w[0].get(0, 7), 0.5);
        assert_eq!(*w[0].get(1, 7), 1.0);
        assert_eq!(*w[0].get(7, 0), 0.25);
        assert!((w[0].sum() - cells.mass()).abs() < 1e-6);
    }

    #[test]
    fn rotation_by_quarter_turns_permutes_cells() {
        let cells = sample_cells();
        let mut w = world(16, 16, 1);
        // The 3×2 creature becomes 2×3: centre on a column boundary and a row centre.
        let at = Placement { x: 8.0, y: 8.5, rotation_deg: 90.0, scale: 1.0, blend: Blend::Replace };
        place(&mut w, &cells, &at, None).expect("placed");
        assert!((w[0].sum() - cells.mass()).abs() < 1e-4, "mass {}", w[0].sum());
        // A clockwise quarter turn maps the top row onto the right column (top to
        // bottom) and the bottom row onto the left column.
        let col = |x: usize| [*w[0].get(x, 7), *w[0].get(x, 8), *w[0].get(x, 9)];
        for (got, want) in col(8).iter().zip([0.0, 0.5, 1.0]).chain(col(7).iter().zip([0.25, 0.75, 0.0])) {
            assert!((got - want).abs() < 1e-5, "{got} vs {want}");
        }
    }

    #[test]
    fn scaling_resamples_bilinearly() {
        let blob = Grid2::from_fn(16, 16, |x, y| {
            let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
            (-(dx * dx + dy * dy) / 8.0).exp()
        });
        let cells = CreatureCells::from_planes(vec![blob]).expect("valid");
        let mut w = world(64, 64, 1);
        let at = Placement { x: 32.0, y: 32.0, rotation_deg: 30.0, scale: 2.0, blend: Blend::Add };
        place(&mut w, &cells, &at, None).expect("placed");
        let ratio = w[0].sum() / cells.mass();
        assert!((ratio - 4.0).abs() < 0.08, "mass ratio {ratio}");
        // The peak stays at the placement centre.
        let peak = w[0].iter().max_by(|a, b| a.2.total_cmp(b.2)).map(|(x, y, _)| (x, y));
        assert!(matches!(peak, Some((31 | 32, 31 | 32))), "{peak:?}");
    }

    #[test]
    fn brush_paints_and_erases() {
        let mut w = world(32, 32, 2);
        let brush = Brush {
            x: 10.0,
            y: 10.0,
            radius: 4.0,
            value: 0.8,
            softness: 0.5,
            mode: BrushMode::Paint,
            channel: Some(1),
        };
        apply_brush(&mut w, &brush, 1.0).expect("paint");
        assert!((w[1].get(9, 9) - 0.8).abs() < 1e-6);
        assert_eq!(w[0].sum(), 0.0);
        assert!(w[1].get(13, 10) < &0.8 && w[1].get(13, 10) > &0.0);
        apply_brush(&mut w, &Brush { mode: BrushMode::Erase, softness: 0.0, radius: 6.0, ..brush }, 1.0)
            .expect("erase");
        assert_eq!(w[1].sum(), 0.0);
        assert!(apply_brush(&mut w, &Brush { channel: Some(2), ..brush }, 1.0).is_err());
        assert!(apply_brush(&mut w, &Brush { value: 1.5, ..brush }, 1.0).is_err());
        assert!(apply_brush(&mut w, &Brush { radius: f32::NAN, ..brush }, 1.0).is_err());
    }

    #[test]
    fn randomize_is_deterministic_and_bounded() {
        let region = Region::Disc { x: 16.0, y: 16.0, radius: 6.0 };
        let mut a = world(32, 32, 1);
        let mut b = world(32, 32, 1);
        randomize(&mut a, &region, 0.5, 0.9, &mut SimRng::seed_from_u64(3)).expect("ok");
        randomize(&mut b, &region, 0.5, 0.9, &mut SimRng::seed_from_u64(3)).expect("ok");
        assert_eq!(a, b);
        let filled = a[0].as_slice().iter().filter(|&&v| v > 0.0).count();
        assert!(filled > 30 && filled < 85, "filled {filled}");
        assert!(a[0].as_slice().iter().all(|&v| v < 0.9));
        assert_eq!(*a[0].get(0, 0), 0.0);
        assert!(randomize(&mut a, &region, 1.5, 1.0, &mut SimRng::seed_from_u64(1)).is_err());
    }

    #[test]
    fn invalid_cells_are_rejected() {
        assert!(CellData::Rows(vec![]).decode().is_err());
        assert!(CellData::Rows(vec![vec![vec![-1.0]]]).decode().is_err());
        assert!(CellData::Rle(vec!["A$B!".into(), "q".into()]).decode().is_err());
        let two = CellData::Rows(vec![vec![vec![1.0]], vec![vec![0.5, 0.5]]]).decode().expect("padded");
        assert_eq!((two.width(), two.height(), two.channels()), (2, 1, 2));
        let mut w = world(8, 8, 1);
        let at = Placement { x: 4.0, y: 4.0, rotation_deg: 0.0, scale: 1.0, blend: Blend::Add };
        assert!(place(&mut w, &two, &at, None).is_err());
    }

    #[test]
    fn brush_larger_than_the_world_edits_each_cell_once() {
        // A soft brush wider than a 12 × 10 torus: every cell is blended once,
        // with the weight of its shortest distance to the centre.
        let mut w = world(12, 10, 1);
        let brush =
            Brush { x: 3.0, y: 2.0, radius: 9.0, value: 1.0, softness: 1.0, mode: BrushMode::Paint, channel: None };
        apply_brush(&mut w, &brush, 1.0).expect("paint");
        for (x, y, &v) in w[0].iter() {
            let dx = wrapped_delta(3.0, x as f64 + 0.5, 12.0) as f32;
            let dy = wrapped_delta(2.0, y as f64 + 0.5, 10.0) as f32;
            let expected = brush.weight(dx.hypot(dy));
            assert!((v - expected).abs() < 1e-6, "({x}, {y}): {v} vs {expected}");
        }
        // Far-away centres wrap like nearby ones.
        let mut far = world(12, 10, 1);
        apply_brush(&mut far, &Brush { x: 3.0 + 12.0 * 1000.0, y: 2.0 - 10.0 * 7.0, ..brush }, 1.0).expect("paint");
        assert_eq!(far, w);
    }

    #[test]
    fn placements_must_fit_the_world() {
        let blob = CreatureCells::from_planes(vec![Grid2::new(10, 6, 0.5f32)]).expect("valid");
        let mut w = world(16, 12, 1);
        let at = Placement { x: 8.0, y: 6.0, rotation_deg: 0.0, scale: 1.0, blend: Blend::Add };
        place(&mut w, &blob, &at, None).expect("fits");
        // Twice as large: 20 × 12 cells on a 16 × 12 world.
        assert!(place(&mut w, &blob, &Placement { scale: 2.0, ..at }, None).is_err());
        // Rotated by 90°, the 10 × 6 box is 6 × 10: too tall for 16 × 8.
        let mut short = world(16, 8, 1);
        assert!(place(&mut short, &blob, &Placement { rotation_deg: 90.0, ..at }, None).is_err());
        place(&mut short, &blob, &Placement { rotation_deg: 90.0, scale: 0.7, ..at }, None).expect("fits");
        // Huge but finite coordinates wrap instead of overflowing.
        let mut a = world(16, 12, 1);
        let mut b = world(16, 12, 1);
        place(&mut a, &blob, &Placement { x: 8.0 + 16.0 * 4096.0, ..at }, None).expect("wraps");
        place(&mut b, &blob, &at, None).expect("placed");
        assert_eq!(a, b);
        assert!(place(&mut a, &blob, &Placement { x: 1.0e30, ..at }, None).is_ok());
    }
}
