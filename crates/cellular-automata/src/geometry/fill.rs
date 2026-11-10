//! Random fills with optional symmetry, shared by every model's
//! "randomize" command and by random initial content.

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use super::{CellRect, MAX_EDIT_CELLS, MAX_FILL_CELLS};
use crate::error::CaError;

/// Symmetry imposed by a random fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Symmetry {
    /// Every cell drawn independently.
    #[default]
    None,
    /// Left-right mirror image: `x ↦ x0 + x1 − x`.
    Horizontal,
    /// Top-bottom mirror image: `y ↦ y0 + y1 − y`.
    Vertical,
    /// Both mirrors (four copies).
    Both,
    /// Invariant under 90° rotation about the centre (square regions only).
    Rotate4,
    /// All eight symmetries of the square (square regions only).
    Rotate8,
}

impl Symmetry {
    /// Stable machine name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
            Self::Both => "both",
            Self::Rotate4 => "rotate4",
            Self::Rotate8 => "rotate8",
        }
    }

    /// Images of the region-relative cell `(u, v)` in an `w × h` region.
    fn orbit(self, u: u64, v: u64, w: u64, h: u64, out: &mut Vec<(u64, u64)>) {
        out.clear();
        out.push((u, v));
        let mirror_x = |(a, b): (u64, u64)| (w - 1 - a, b);
        let mirror_y = |(a, b): (u64, u64)| (a, h - 1 - b);
        // Rotation by 90° in a square of side w: (a, b) -> (w-1-b, a).
        let rot = |(a, b): (u64, u64)| (w - 1 - b, a);
        match self {
            Self::None => {}
            Self::Horizontal => out.push(mirror_x((u, v))),
            Self::Vertical => out.push(mirror_y((u, v))),
            Self::Both => {
                out.push(mirror_x((u, v)));
                out.push(mirror_y((u, v)));
                out.push(mirror_y(mirror_x((u, v))));
            }
            Self::Rotate4 | Self::Rotate8 => {
                let mut p = (u, v);
                for _ in 0..3 {
                    p = rot(p);
                    out.push(p);
                }
                if self == Self::Rotate8 {
                    let mut q = mirror_x((u, v));
                    out.push(q);
                    for _ in 0..3 {
                        q = rot(q);
                        out.push(q);
                    }
                }
            }
        }
        out.sort_unstable_by_key(|&(a, b)| (b, a));
        out.dedup();
    }
}

/// Checks the arguments of a random fill: density in `[0, 1]`, a square
/// region for [`Symmetry::Rotate4`] / [`Symmetry::Rotate8`], and at most
/// `limit` cells.
///
/// # Errors
/// [`CaError::InvalidArgument`] or [`CaError::LimitExceeded`] describing the problem.
pub fn check_fill(region: CellRect, density: f64, symmetry: Symmetry, limit: u64) -> Result<(), CaError> {
    if !(0.0..=1.0).contains(&density) {
        return Err(CaError::invalid(format!("density {density} must be between 0 and 1")));
    }
    if region.area() > u128::from(limit) {
        return Err(CaError::LimitExceeded(format!(
            "a random fill of {}×{} cells exceeds the limit of {limit} cells",
            region.width(),
            region.height()
        )));
    }
    let (w, h) = (region.width(), region.height());
    if matches!(symmetry, Symmetry::Rotate4 | Symmetry::Rotate8) && w != h {
        return Err(CaError::invalid(format!("{} symmetry needs a square region, got {w}×{h}", symmetry.as_str())));
    }
    Ok(())
}

/// Draws a random fill of `region` with the requested symmetry and passes
/// every live cell to `emit`, in row-major order, without building a list
/// (regions up to [`MAX_FILL_CELLS`] cells, i.e. the largest world).
///
/// Cells are visited in row-major order; for every symmetry orbit, its
/// row-major-first cell consumes exactly one random number (`rng.f64()`), and
/// the whole orbit becomes alive with probability `density`. Symmetric fills
/// remember the decided orbits in a bitset of one bit per cell of the region.
///
/// # Errors
/// See [`check_fill`] (with the limit [`MAX_FILL_CELLS`]); nothing is
/// emitted and no random number is drawn on error.
pub fn random_fill_each(
    rng: &mut SimRng,
    region: CellRect,
    density: f64,
    symmetry: Symmetry,
    mut emit: impl FnMut(i64, i64),
) -> Result<(), CaError> {
    check_fill(region, density, symmetry, MAX_FILL_CELLS)?;
    let (w, h) = (region.width(), region.height());
    if symmetry == Symmetry::None {
        for v in 0..h {
            for u in 0..w {
                if rng.f64() < density {
                    emit(region.x0 + u as i64, region.y0 + v as i64);
                }
            }
        }
        return Ok(());
    }
    // One bit per cell: set when the cell's orbit was drawn alive.
    let mut alive = vec![0u64; (w * h).div_ceil(64) as usize];
    let mut orbit = Vec::with_capacity(8);
    for v in 0..h {
        for u in 0..w {
            symmetry.orbit(u, v, w, h, &mut orbit);
            // The orbit's first cell draws for the whole orbit; later cells
            // were decided when their first cell was visited.
            if orbit[0] == (u, v) && rng.f64() < density {
                for &(a, b) in &orbit {
                    let i = b * w + a;
                    alive[(i / 64) as usize] |= 1 << (i % 64);
                }
            }
            let i = v * w + u;
            if alive[(i / 64) as usize] >> (i % 64) & 1 == 1 {
                emit(region.x0 + u as i64, region.y0 + v as i64);
            }
        }
    }
    Ok(())
}

/// [`random_fill_each`] collected into a list of live cells (row-major),
/// for regions of at most [`MAX_EDIT_CELLS`] cells.
///
/// # Errors
/// See [`check_fill`] (with the limit [`MAX_EDIT_CELLS`]).
pub fn random_fill(
    rng: &mut SimRng,
    region: CellRect,
    density: f64,
    symmetry: Symmetry,
) -> Result<Vec<(i64, i64)>, CaError> {
    check_fill(region, density, symmetry, MAX_EDIT_CELLS)?;
    let mut cells = Vec::new();
    random_fill_each(rng, region, density, symmetry, |x, y| cells.push((x, y)))?;
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symmetric_fills_are_symmetric() {
        let region = CellRect::new(0, 0, 15, 15);
        for symmetry in [Symmetry::Horizontal, Symmetry::Vertical, Symmetry::Both, Symmetry::Rotate4, Symmetry::Rotate8]
        {
            let mut rng = SimRng::seed_from_u64(9);
            let cells = random_fill(&mut rng, region, 0.4, symmetry).unwrap();
            let set: std::collections::BTreeSet<_> = cells.iter().copied().collect();
            for &(x, y) in &cells {
                let images: Vec<(i64, i64)> = match symmetry {
                    Symmetry::Horizontal => vec![(15 - x, y)],
                    Symmetry::Vertical => vec![(x, 15 - y)],
                    Symmetry::Both => vec![(15 - x, y), (x, 15 - y)],
                    Symmetry::Rotate4 => vec![(15 - y, x)],
                    _ => vec![(15 - y, x), (15 - x, y)],
                };
                for image in images {
                    assert!(set.contains(&image), "{symmetry:?}: {:?} missing", image);
                }
            }
        }
        let mut rng = SimRng::seed_from_u64(1);
        assert!(random_fill(&mut rng, CellRect::new(0, 0, 3, 5), 0.5, Symmetry::Rotate4).is_err());
        assert!(random_fill(&mut rng, region, 1.5, Symmetry::None).is_err());
    }

    #[test]
    fn streaming_fill_equals_the_list_and_checks_first() {
        let region = CellRect::new(-3, 5, 12, 20);
        for symmetry in [Symmetry::None, Symmetry::Vertical, Symmetry::Both, Symmetry::Rotate8] {
            let mut a = SimRng::seed_from_u64(4);
            let mut b = SimRng::seed_from_u64(4);
            let list = random_fill(&mut a, region, 0.45, symmetry).unwrap();
            let mut streamed = Vec::new();
            random_fill_each(&mut b, region, 0.45, symmetry, |x, y| streamed.push((x, y))).unwrap();
            assert_eq!(streamed, list, "{symmetry:?}");
            assert_eq!(a, b, "the same random numbers are drawn");
        }
        // Larger than the list limit, still fine when streamed; invalid input draws nothing.
        let big = CellRect::new(0, 0, (1 << 13) - 1, (1 << 13) + 7);
        let mut rng = SimRng::seed_from_u64(1);
        assert!(random_fill(&mut rng, big, 0.0, Symmetry::None).is_err());
        let before = rng.clone();
        assert!(random_fill_each(&mut rng, big, 1.5, Symmetry::None, |_, _| {}).is_err());
        assert!(random_fill_each(&mut rng, big, 0.5, Symmetry::Rotate4, |_, _| {}).is_err());
        assert_eq!(rng, before);
        let mut count = 0u64;
        random_fill_each(&mut rng, big, 0.001, Symmetry::None, |_, _| count += 1).unwrap();
        assert!(count > 0 && count < 1 << 17);
    }
}
