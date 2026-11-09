//! CPU-side grid frames: the RGBA8 data texture a mode uploads for the visible
//! cell region.
//!
//! Channel layout (one texel = `cells_per_texel × cells_per_texel` cells):
//!
//! | Channel | Meaning |
//! |---|---|
//! | R | cell state 0..=255 (0 = dead / empty), or an encoded field value for continuous palettes |
//! | G | age (generations alive, saturating at 255, or a scaled age) |
//! | B | activity / trail intensity (255 = changed this tick, decaying afterwards) |
//! | A | density or auxiliary value (fraction of live cells × 255 for LOD texels) |
//!
//! Row 0 of [`GridFrame::data`] is the top row (smallest `y`), matching the
//! workspace convention that `y` grows downward.

use bevy::math::I64Vec2;
use std::fmt;

/// Largest frame dimension in texels accepted for upload (wgpu's default
/// `max_texture_dimension_2d`).
pub const MAX_FRAME_DIMENSION: u32 = 8192;

/// One texel of a [`GridFrame`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Texel {
    /// Cell state or encoded field value (R).
    pub state: u8,
    /// Age (G).
    pub age: u8,
    /// Activity / trail (B).
    pub activity: u8,
    /// Density / auxiliary (A).
    pub aux: u8,
}

impl Texel {
    /// A texel with only the state set.
    pub const fn state(state: u8) -> Self {
        Self { state, age: 0, activity: 0, aux: 0 }
    }

    /// Packs the texel as RGBA bytes.
    pub const fn to_rgba(self) -> [u8; 4] {
        [self.state, self.age, self.activity, self.aux]
    }

    /// Unpacks RGBA bytes.
    pub const fn from_rgba(rgba: [u8; 4]) -> Self {
        Self { state: rgba[0], age: rgba[1], activity: rgba[2], aux: rgba[3] }
    }
}

/// Encodes a value in `[0, 1]` as a byte (`round(v × 255)`, clamped; NaN → 0).
pub fn encode_unit(value: f32) -> u8 {
    if value.is_nan() {
        return 0;
    }
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Encodes `value` in `[min, max]` as a byte (linear, clamped). A degenerate
/// range encodes everything as 0.
pub fn encode_range(value: f32, min: f32, max: f32) -> u8 {
    let span = max - min;
    if !(span.is_finite() && span > 0.0) {
        return 0;
    }
    encode_unit((value - min) / span)
}

/// Errors detected when validating a [`GridFrame`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridFrameError {
    /// Width or height is zero.
    Empty,
    /// Width or height exceeds [`MAX_FRAME_DIMENSION`].
    TooLarge {
        /// Frame width in texels.
        width: u32,
        /// Frame height in texels.
        height: u32,
    },
    /// `cells_per_texel` is zero.
    ZeroCellsPerTexel,
    /// `data.len()` differs from `width × height × 4`.
    DataLength {
        /// Expected byte count.
        expected: usize,
        /// Actual byte count.
        actual: usize,
    },
    /// A texel coordinate is outside the frame.
    OutOfBounds {
        /// Requested column.
        x: u32,
        /// Requested row.
        y: u32,
    },
}

impl fmt::Display for GridFrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "grid frame has zero width or height"),
            Self::TooLarge { width, height } => write!(
                f,
                "grid frame {width}×{height} exceeds the maximum texture size of {MAX_FRAME_DIMENSION}×{MAX_FRAME_DIMENSION}; use a coarser level of detail"
            ),
            Self::ZeroCellsPerTexel => write!(f, "cells_per_texel must be at least 1"),
            Self::DataLength { expected, actual } => {
                write!(f, "grid frame data has {actual} bytes, expected {expected} (width × height × 4)")
            }
            Self::OutOfBounds { x, y } => write!(f, "texel ({x}, {y}) is outside the grid frame"),
        }
    }
}

impl std::error::Error for GridFrameError {}

/// Rectangular cell region encoded as an RGBA8 texture for the grid renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridFrame {
    /// Cell coordinate of the top-left corner of texel (0, 0).
    pub origin: I64Vec2,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Cells covered by one texel along each axis (1 = exact cells, > 1 = LOD).
    pub cells_per_texel: u32,
    /// RGBA bytes, row-major, row 0 at the top; `width × height × 4` bytes.
    pub data: Vec<u8>,
}

impl GridFrame {
    /// A zero-filled frame. Fails if the geometry is invalid.
    pub fn new(origin: I64Vec2, width: u32, height: u32, cells_per_texel: u32) -> Result<Self, GridFrameError> {
        check_geometry(width, height, cells_per_texel)?;
        let len = width as usize * height as usize * 4;
        Ok(Self { origin, width, height, cells_per_texel, data: vec![0; len] })
    }

    /// A frame from one state byte per texel (G, B, A set to 0).
    pub fn from_states(origin: I64Vec2, width: u32, height: u32, states: &[u8]) -> Result<Self, GridFrameError> {
        check_geometry(width, height, 1)?;
        let expected = width as usize * height as usize;
        if states.len() != expected {
            return Err(GridFrameError::DataLength { expected: expected * 4, actual: states.len() * 4 });
        }
        let data = states.iter().flat_map(|&s| [s, 0, 0, 0]).collect();
        Ok(Self { origin, width, height, cells_per_texel: 1, data })
    }

    /// Checks geometry and data length.
    pub fn validate(&self) -> Result<(), GridFrameError> {
        check_geometry(self.width, self.height, self.cells_per_texel)?;
        let expected = self.width as usize * self.height as usize * 4;
        if self.data.len() != expected {
            return Err(GridFrameError::DataLength { expected, actual: self.data.len() });
        }
        Ok(())
    }

    fn index(&self, x: u32, y: u32) -> Result<usize, GridFrameError> {
        if x >= self.width || y >= self.height {
            return Err(GridFrameError::OutOfBounds { x, y });
        }
        Ok((y as usize * self.width as usize + x as usize) * 4)
    }

    /// Reads a texel.
    pub fn get(&self, x: u32, y: u32) -> Result<Texel, GridFrameError> {
        let i = self.index(x, y)?;
        let px = self.data.get(i..i + 4).ok_or(GridFrameError::OutOfBounds { x, y })?;
        Ok(Texel::from_rgba([px[0], px[1], px[2], px[3]]))
    }

    /// Writes a texel.
    pub fn set(&mut self, x: u32, y: u32, texel: Texel) -> Result<(), GridFrameError> {
        let i = self.index(x, y)?;
        let px = self.data.get_mut(i..i + 4).ok_or(GridFrameError::OutOfBounds { x, y })?;
        px.copy_from_slice(&texel.to_rgba());
        Ok(())
    }

    /// Writes the texel that contains cell `cell`, if it is inside the frame.
    /// Returns whether the cell was inside.
    pub fn set_cell(&mut self, cell: I64Vec2, texel: Texel) -> bool {
        match self.texel_of_cell(cell) {
            Some((x, y)) => self.set(x, y, texel).is_ok(),
            None => false,
        }
    }

    /// Texel coordinates containing a cell, if inside the frame.
    pub fn texel_of_cell(&self, cell: I64Vec2) -> Option<(u32, u32)> {
        let k = i64::from(self.cells_per_texel.max(1));
        let dx = cell.x.checked_sub(self.origin.x)?.div_euclid(k);
        let dy = cell.y.checked_sub(self.origin.y)?.div_euclid(k);
        let x = u32::try_from(dx).ok().filter(|&x| x < self.width)?;
        let y = u32::try_from(dy).ok().filter(|&y| y < self.height)?;
        Some((x, y))
    }

    /// Size of the covered region in cells (saturating).
    pub fn size_in_cells(&self) -> I64Vec2 {
        let k = i64::from(self.cells_per_texel);
        I64Vec2::new(i64::from(self.width).saturating_mul(k), i64::from(self.height).saturating_mul(k))
    }

    /// Geometry without the pixel data.
    pub fn geometry(&self) -> FrameGeometry {
        FrameGeometry {
            origin: self.origin,
            width: self.width,
            height: self.height,
            cells_per_texel: self.cells_per_texel,
        }
    }
}

/// Placement of an uploaded frame (everything but the data).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameGeometry {
    /// Cell coordinate of texel (0, 0)'s top-left corner.
    pub origin: I64Vec2,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Cells per texel along each axis.
    pub cells_per_texel: u32,
}

fn check_geometry(width: u32, height: u32, cells_per_texel: u32) -> Result<(), GridFrameError> {
    if width == 0 || height == 0 {
        return Err(GridFrameError::Empty);
    }
    if width > MAX_FRAME_DIMENSION || height > MAX_FRAME_DIMENSION {
        return Err(GridFrameError::TooLarge { width, height });
    }
    if cells_per_texel == 0 {
        return Err(GridFrameError::ZeroCellsPerTexel);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_unit_rounds_and_clamps() {
        assert_eq!(encode_unit(0.0), 0);
        assert_eq!(encode_unit(1.0), 255);
        assert_eq!(encode_unit(0.5), 128);
        assert_eq!(encode_unit(-1.0), 0);
        assert_eq!(encode_unit(3.0), 255);
        assert_eq!(encode_unit(f32::NAN), 0);
        assert_eq!(encode_range(5.0, 0.0, 10.0), 128);
        assert_eq!(encode_range(5.0, 3.0, 3.0), 0);
    }

    #[test]
    fn frame_validation() {
        assert_eq!(GridFrame::new(I64Vec2::ZERO, 0, 4, 1), Err(GridFrameError::Empty));
        assert_eq!(GridFrame::new(I64Vec2::ZERO, 9000, 4, 1), Err(GridFrameError::TooLarge { width: 9000, height: 4 }));
        assert_eq!(GridFrame::new(I64Vec2::ZERO, 2, 2, 0), Err(GridFrameError::ZeroCellsPerTexel));
        let mut f = GridFrame::new(I64Vec2::ZERO, 3, 2, 1).expect("valid");
        assert_eq!(f.data.len(), 24);
        assert!(f.validate().is_ok());
        f.data.pop();
        assert_eq!(f.validate(), Err(GridFrameError::DataLength { expected: 24, actual: 23 }));
        assert!(GridFrame::from_states(I64Vec2::ZERO, 2, 2, &[1, 2, 3]).is_err());
    }

    #[test]
    fn texel_layout_is_row_major_top_down() {
        let f = GridFrame::from_states(I64Vec2::new(10, 20), 3, 2, &[1, 2, 3, 4, 5, 6]).expect("valid");
        assert_eq!(f.get(0, 0).map(|t| t.state), Ok(1));
        assert_eq!(f.get(2, 0).map(|t| t.state), Ok(3));
        assert_eq!(f.get(0, 1).map(|t| t.state), Ok(4));
        assert_eq!(f.get(3, 0), Err(GridFrameError::OutOfBounds { x: 3, y: 0 }));
        assert_eq!(&f.data[12..16], &[4, 0, 0, 0]);
    }

    #[test]
    fn cell_to_texel_with_lod() {
        let mut f = GridFrame::new(I64Vec2::new(-8, 4), 4, 4, 4).expect("valid");
        assert_eq!(f.texel_of_cell(I64Vec2::new(-8, 4)), Some((0, 0)));
        assert_eq!(f.texel_of_cell(I64Vec2::new(-5, 7)), Some((0, 0)));
        assert_eq!(f.texel_of_cell(I64Vec2::new(-4, 8)), Some((1, 1)));
        assert_eq!(f.texel_of_cell(I64Vec2::new(7, 19)), Some((3, 3)));
        assert_eq!(f.texel_of_cell(I64Vec2::new(8, 4)), None);
        assert_eq!(f.texel_of_cell(I64Vec2::new(-9, 4)), None);
        assert_eq!(f.texel_of_cell(I64Vec2::new(i64::MIN, 4)), None);
        assert!(f.set_cell(I64Vec2::new(0, 10), Texel { state: 1, age: 2, activity: 3, aux: 4 }));
        assert_eq!(f.get(2, 1), Ok(Texel { state: 1, age: 2, activity: 3, aux: 4 }));
        assert!(!f.set_cell(I64Vec2::new(100, 100), Texel::state(1)));
        assert_eq!(f.size_in_cells(), I64Vec2::new(16, 16));
    }
}
