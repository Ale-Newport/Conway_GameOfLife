//! Coordinates, axes, boxes and world kinds.
//!
//! Voxel coordinates are `(x, y, z)` triples of `i32`. Finite worlds of size
//! `W × H × D` occupy `[0, W) × [0, H) × [0, D)`. The unbounded sparse world
//! accepts coordinates in `[-2^30, 2^30)` on every axis (see
//! [`COORD_LIMIT`]).

use std::fmt;

use serde::{Deserialize, Serialize};

/// A voxel position `[x, y, z]`.
pub type Pos = [i32; 3];

/// Largest world side accepted by the size parameters.
pub const MAX_SIDE: u32 = 65_536;

/// Largest number of voxels of a dense world (256³). Dense storage keeps one
/// state byte, one scratch byte, one neighbour-sum byte and a two-byte age
/// per voxel, so this is about 80 MiB.
pub const MAX_DENSE_VOXELS: u64 = 1 << 24;

/// The unbounded sparse world is limited to coordinates in
/// `[-COORD_LIMIT, COORD_LIMIT)`; voxels beyond it are never born. A
/// light-speed pattern needs about 10⁹ generations to reach it.
pub const COORD_LIMIT: i32 = 1 << 30;

/// A coordinate axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// The x axis (columns).
    X,
    /// The y axis (rows).
    Y,
    /// The z axis (layers).
    #[default]
    Z,
}

impl Axis {
    /// All axes in order.
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    /// Index of the axis in a [`Pos`].
    pub fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }

    /// The two in-plane axes `(u, v)` of a slice perpendicular to this axis:
    /// `(y, z)` for x, `(x, z)` for y and `(x, y)` for z.
    pub fn plane_axes(self) -> (Axis, Axis) {
        match self {
            Self::X => (Self::Y, Self::Z),
            Self::Y => (Self::X, Self::Z),
            Self::Z => (Self::X, Self::Y),
        }
    }
}

/// An axis-aligned box of voxels with inclusive corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Bounds {
    /// Smallest coordinate on each axis.
    pub min: Pos,
    /// Largest coordinate on each axis (inclusive).
    pub max: Pos,
}

impl Bounds {
    /// The box `[0, size)` on every axis. Sizes must be at least 1.
    pub fn from_size(size: [u32; 3]) -> Self {
        let last = |s: u32| i32::try_from(s.max(1) - 1).unwrap_or(i32::MAX);
        Self { min: [0; 3], max: [last(size[0]), last(size[1]), last(size[2])] }
    }

    /// The box of the unbounded sparse world.
    pub fn unbounded() -> Self {
        Self { min: [-COORD_LIMIT; 3], max: [COORD_LIMIT - 1; 3] }
    }

    /// The single voxel `pos`.
    pub fn point(pos: Pos) -> Self {
        Self { min: pos, max: pos }
    }

    /// Whether `pos` lies inside the box.
    pub fn contains(&self, pos: Pos) -> bool {
        (0..3).all(|a| pos[a] >= self.min[a] && pos[a] <= self.max[a])
    }

    /// Smallest box containing `self` and `pos`.
    pub fn including(mut self, pos: Pos) -> Self {
        for (a, &c) in pos.iter().enumerate() {
            self.min[a] = self.min[a].min(c);
            self.max[a] = self.max[a].max(c);
        }
        self
    }

    /// Smallest box containing both boxes.
    pub fn union(self, other: Bounds) -> Self {
        self.including(other.min).including(other.max)
    }

    /// Intersection of two boxes, or `None` when they do not overlap.
    pub fn intersection(&self, other: &Bounds) -> Option<Bounds> {
        let mut out = *self;
        for a in 0..3 {
            out.min[a] = self.min[a].max(other.min[a]);
            out.max[a] = self.max[a].min(other.max[a]);
            if out.min[a] > out.max[a] {
                return None;
            }
        }
        Some(out)
    }

    /// Number of voxels along each axis.
    pub fn extent(&self) -> [u64; 3] {
        [0, 1, 2].map(|a| (i64::from(self.max[a]) - i64::from(self.min[a]) + 1) as u64)
    }

    /// Number of voxels in the box (saturating).
    pub fn volume(&self) -> u64 {
        let [x, y, z] = self.extent();
        x.saturating_mul(y).saturating_mul(z)
    }
}

impl fmt::Display for Bounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}..{}] × [{}..{}] × [{}..{}]",
            self.min[0], self.max[0], self.min[1], self.max[1], self.min[2], self.max[2]
        )
    }
}

/// Storage and edge behaviour of a world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorldKind {
    /// Dense `u8` grid; voxels outside the box are permanently empty.
    #[default]
    DenseBounded,
    /// Dense `u8` grid whose opposite faces are joined (3-torus).
    DenseTorus,
    /// Hash map of 16³ chunks clipped to the world box (large, mostly empty
    /// bounded worlds).
    SparseBounded,
    /// Hash map of 16³ chunks without edges (expanding patterns); the world
    /// size only sets the box used for initialisation.
    SparseUnbounded,
}

impl WorldKind {
    /// All kinds in display order.
    pub const ALL: [WorldKind; 4] = [Self::DenseBounded, Self::DenseTorus, Self::SparseBounded, Self::SparseUnbounded];

    /// Stable key used in parameters and scenario files.
    pub fn key(self) -> &'static str {
        match self {
            Self::DenseBounded => "dense_bounded",
            Self::DenseTorus => "dense_torus",
            Self::SparseBounded => "sparse_bounded",
            Self::SparseUnbounded => "sparse_unbounded",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::DenseBounded => "Dense grid, bounded (empty outside)",
            Self::DenseTorus => "Dense grid, toroidal (wrap-around)",
            Self::SparseBounded => "Sparse chunks, bounded",
            Self::SparseUnbounded => "Sparse chunks, unbounded",
        }
    }

    /// Parses a [`WorldKind::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    /// Whether the kind uses chunked sparse storage.
    pub fn is_sparse(self) -> bool {
        matches!(self, Self::SparseBounded | Self::SparseUnbounded)
    }

    /// Whether the world has a finite box of voxels.
    pub fn is_finite(self) -> bool {
        !matches!(self, Self::SparseUnbounded)
    }

    /// The box of coordinates that can hold voxels.
    pub fn bounds(self, size: [u32; 3]) -> Bounds {
        if self.is_finite() { Bounds::from_size(size) } else { Bounds::unbounded() }
    }
}

/// Validates a world size for a world kind.
pub(crate) fn validate_size(kind: WorldKind, size: [u32; 3]) -> Result<(), String> {
    if let Some(axis) = size.iter().position(|&s| s == 0 || s > MAX_SIDE) {
        return Err(format!(
            "size along {} is {}; it must be between 1 and {MAX_SIDE}",
            ["x", "y", "z"][axis],
            size[axis]
        ));
    }
    let volume = size.iter().map(|&s| u64::from(s)).product::<u64>();
    if !kind.is_sparse() && volume > MAX_DENSE_VOXELS {
        return Err(format!(
            "a dense world of {}×{}×{} has {volume} voxels, more than the dense limit of {MAX_DENSE_VOXELS} \
             (256³); choose sparse storage or a smaller size",
            size[0], size[1], size[2]
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_operations() {
        let b = Bounds::from_size([4, 5, 6]);
        assert_eq!(b.max, [3, 4, 5]);
        assert!(b.contains([0, 0, 0]) && b.contains([3, 4, 5]) && !b.contains([4, 0, 0]) && !b.contains([0, -1, 0]));
        assert_eq!(b.volume(), 120);
        let other = Bounds { min: [2, 2, 2], max: [9, 9, 9] };
        assert_eq!(b.intersection(&other), Some(Bounds { min: [2, 2, 2], max: [3, 4, 5] }));
        assert_eq!(b.intersection(&Bounds::point([10, 0, 0])), None);
        assert_eq!(Bounds::point([1, 1, 1]).including([-1, 3, 0]), Bounds { min: [-1, 1, 0], max: [1, 3, 1] });
    }

    #[test]
    fn size_validation() {
        assert!(validate_size(WorldKind::DenseBounded, [256, 256, 256]).is_ok());
        assert!(validate_size(WorldKind::DenseTorus, [256, 256, 257]).is_err());
        assert!(validate_size(WorldKind::SparseBounded, [4096, 4096, 4096]).is_ok());
        assert!(validate_size(WorldKind::SparseUnbounded, [0, 1, 1]).is_err());
        assert!(validate_size(WorldKind::SparseUnbounded, [MAX_SIDE + 1, 1, 1]).is_err());
        for kind in WorldKind::ALL {
            assert_eq!(WorldKind::from_key(kind.key()), Some(kind));
        }
    }
}
