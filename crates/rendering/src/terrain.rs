//! Heightmap terrain and water-surface meshes.
//!
//! Grid convention (workspace §3.3): cell `(x, y)` of a `width × depth` grid
//! maps to Bevy `(x, height, y)`; one vertex sits at each cell centre
//! `((x + 0.5) s, h(x, y) k, (y + 0.5) s)` with cell size `s` and height scale
//! `k`. Vertex normals come from central differences of the sampled heights
//! (one-sided at the borders): the gradient is the average of the two
//! adjacent edge slopes along each axis, which is exact for planar slopes and
//! a smooth-shading approximation elsewhere (the mesh itself is two triangles
//! per quad). Level of detail decimates the grid by taking every `step`-th
//! sample (the last row/column is always kept so the extent never shrinks);
//! heights, colours and water are sampled at the kept vertices only, so
//! features narrower than the step can disappear at coarse levels.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::fmt;

/// Errors from terrain inputs.
#[derive(Debug, Clone, PartialEq)]
pub enum TerrainError {
    /// The grid needs at least 2 × 2 cells.
    TooSmall {
        /// Requested width.
        width: usize,
        /// Requested depth.
        depth: usize,
    },
    /// A per-cell buffer has the wrong length.
    Length {
        /// Buffer name.
        what: &'static str,
        /// Expected length (`width × depth`).
        expected: usize,
        /// Actual length.
        actual: usize,
    },
    /// A per-cell value is NaN or infinite.
    NonFinite {
        /// Buffer name.
        what: &'static str,
        /// Cell index of the offending value.
        index: usize,
    },
    /// The LOD step must be at least 1.
    ZeroStep,
    /// `cell_size` must be finite and positive, `height_scale` finite.
    InvalidSettings {
        /// Requested cell size.
        cell_size: f32,
        /// Requested height scale.
        height_scale: f32,
    },
    /// The mesh would have more vertices than 32-bit indices can address.
    TooManyVertices {
        /// Vertices the mesh would need.
        vertices: usize,
    },
}

impl fmt::Display for TerrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall { width, depth } => write!(f, "terrain needs at least 2×2 cells, got {width}×{depth}"),
            Self::Length { what, expected, actual } => {
                write!(f, "{what} has {actual} values, expected {expected} (width × depth)")
            }
            Self::NonFinite { what, index } => write!(f, "{what} at cell index {index} is not finite"),
            Self::ZeroStep => write!(f, "terrain LOD step must be at least 1"),
            Self::InvalidSettings { cell_size, height_scale } => write!(
                f,
                "terrain cell size must be finite and positive and the height scale finite \
                 (got cell size {cell_size}, height scale {height_scale})"
            ),
            Self::TooManyVertices { vertices } => {
                write!(
                    f,
                    "terrain mesh needs {vertices} vertices, more than 32-bit indices allow; use a larger LOD step"
                )
            }
        }
    }
}

impl std::error::Error for TerrainError {}

/// Geometry settings shared by terrain and water meshes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainSettings {
    /// World units per cell.
    pub cell_size: f32,
    /// World units per height unit.
    pub height_scale: f32,
    /// Decimation step (1 = every cell).
    pub lod_step: usize,
}

impl Default for TerrainSettings {
    fn default() -> Self {
        Self { cell_size: 1.0, height_scale: 1.0, lod_step: 1 }
    }
}

/// Mesh buffers produced by [`terrain_mesh`] and [`water_mesh`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceMeshData {
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// Linear RGBA colours (terrain) — empty for water.
    pub colors: Vec<[f32; 4]>,
    /// UVs: grid position in `[0, 1]` for terrain; `(water depth, 0)` in world units for water.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle indices (counter-clockwise seen from above).
    pub indices: Vec<u32>,
}

impl SurfaceMeshData {
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Converts to a Bevy mesh.
    pub fn to_mesh(&self) -> Mesh {
        let mut mesh =
            Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions.clone())
                .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals.clone())
                .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs.clone())
                .with_inserted_indices(Indices::U32(self.indices.clone()));
        if !self.colors.is_empty() {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors.clone());
        }
        mesh
    }
}

/// Sample indices `0, step, 2·step, …` plus `n - 1`.
pub fn lod_samples(n: usize, step: usize) -> Vec<usize> {
    let step = step.max(1);
    let mut out: Vec<usize> = (0..n).step_by(step).collect();
    if n > 0 && out.last() != Some(&(n - 1)) {
        out.push(n - 1);
    }
    out
}

/// Validates the grid, settings and one per-cell buffer.
fn check_inputs(
    what: &'static str,
    width: usize,
    depth: usize,
    values: &[f32],
    settings: &TerrainSettings,
) -> Result<(), TerrainError> {
    if width < 2 || depth < 2 {
        return Err(TerrainError::TooSmall { width, depth });
    }
    if settings.lod_step == 0 {
        return Err(TerrainError::ZeroStep);
    }
    let TerrainSettings { cell_size, height_scale, .. } = *settings;
    if !(cell_size.is_finite() && cell_size > 0.0 && height_scale.is_finite()) {
        return Err(TerrainError::InvalidSettings { cell_size, height_scale });
    }
    check_values(what, width, depth, values)?;
    let vertices = lod_samples(width, settings.lod_step).len() * lod_samples(depth, settings.lod_step).len();
    if vertices > u32::MAX as usize {
        return Err(TerrainError::TooManyVertices { vertices });
    }
    Ok(())
}

/// Checks the length and finiteness of a per-cell buffer.
fn check_values(what: &'static str, width: usize, depth: usize, values: &[f32]) -> Result<(), TerrainError> {
    let expected = width * depth;
    if values.len() != expected {
        return Err(TerrainError::Length { what, expected, actual: values.len() });
    }
    if let Some(index) = values.iter().position(|h| !h.is_finite()) {
        return Err(TerrainError::NonFinite { what, index });
    }
    Ok(())
}

/// Normal of the height field at sample `(i, j)` of the sampled index lists.
fn sampled_normal(
    heights: &[f32],
    width: usize,
    xs: &[usize],
    zs: &[usize],
    i: usize,
    j: usize,
    settings: &TerrainSettings,
) -> Vec3 {
    let h = |x: usize, z: usize| heights[z * width + x] * settings.height_scale;
    let (i0, i1) = (i.saturating_sub(1), (i + 1).min(xs.len() - 1));
    let (j0, j1) = (j.saturating_sub(1), (j + 1).min(zs.len() - 1));
    let z = zs[j];
    let x = xs[i];
    let dx = (xs[i1] - xs[i0]) as f32 * settings.cell_size;
    let dz = (zs[j1] - zs[j0]) as f32 * settings.cell_size;
    let slope_x = if dx > 0.0 { (h(xs[i1], z) - h(xs[i0], z)) / dx } else { 0.0 };
    let slope_z = if dz > 0.0 { (h(x, zs[j1]) - h(x, zs[j0])) / dz } else { 0.0 };
    Vec3::new(-slope_x, 1.0, -slope_z).normalize()
}

fn grid_indices(nx: usize, nz: usize, keep_quad: impl Fn(usize, usize) -> bool) -> Vec<u32> {
    let mut indices = Vec::with_capacity((nx - 1) * (nz - 1) * 6);
    for j in 0..nz - 1 {
        for i in 0..nx - 1 {
            if !keep_quad(i, j) {
                continue;
            }
            let a = (j * nx + i) as u32;
            let b = a + 1;
            let c = a + nx as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    indices
}

/// Builds a terrain mesh from a `width × depth` heightmap (row-major, row =
/// y / z) with optional per-cell colours (biome colours).
pub fn terrain_mesh(
    heights: &[f32],
    width: usize,
    depth: usize,
    colors: Option<&[Color]>,
    settings: &TerrainSettings,
) -> Result<SurfaceMeshData, TerrainError> {
    check_inputs("heights", width, depth, heights, settings)?;
    if let Some(colors) = colors
        && colors.len() != width * depth
    {
        return Err(TerrainError::Length { what: "colors", expected: width * depth, actual: colors.len() });
    }
    let xs = lod_samples(width, settings.lod_step);
    let zs = lod_samples(depth, settings.lod_step);
    let mut out = SurfaceMeshData::default();
    for (j, &z) in zs.iter().enumerate() {
        for (i, &x) in xs.iter().enumerate() {
            let idx = z * width + x;
            out.positions.push([
                (x as f32 + 0.5) * settings.cell_size,
                heights[idx] * settings.height_scale,
                (z as f32 + 0.5) * settings.cell_size,
            ]);
            out.normals.push(sampled_normal(heights, width, &xs, &zs, i, j, settings).to_array());
            out.uvs.push([x as f32 / (width - 1) as f32, z as f32 / (depth - 1) as f32]);
            if let Some(colors) = colors {
                out.colors.push(colors[idx].to_linear().to_f32_array());
            }
        }
    }
    out.indices = grid_indices(xs.len(), zs.len(), |_, _| true);
    Ok(out)
}

/// Builds a water surface over a terrain from per-cell water depth (same
/// units as the heights). Vertices of wet cells (`depth > min_depth`) sit at
/// `terrain + depth`; dry vertices next to water take the highest neighbouring
/// water level, so the surface runs into the shore and is clipped by the
/// terrain there. UV.x carries `level − terrain` in world units (≤ 0 on dry
/// land), which the water shader uses for depth colour, opacity and the
/// shoreline fade. Only quads touching water are emitted.
pub fn water_mesh(
    terrain: &[f32],
    water_depth: &[f32],
    width: usize,
    depth: usize,
    min_depth: f32,
    settings: &TerrainSettings,
) -> Result<SurfaceMeshData, TerrainError> {
    check_inputs("heights", width, depth, terrain, settings)?;
    check_values("water depth", width, depth, water_depth)?;
    let xs = lod_samples(width, settings.lod_step);
    let zs = lod_samples(depth, settings.lod_step);
    let (nx, nz) = (xs.len(), zs.len());
    let wet = |i: usize, j: usize| water_depth[zs[j] * width + xs[i]] > min_depth;
    let level = |i: usize, j: usize| {
        let idx = zs[j] * width + xs[i];
        terrain[idx] + water_depth[idx]
    };
    let mut out = SurfaceMeshData::default();
    for j in 0..nz {
        for i in 0..nx {
            let ground = terrain[zs[j] * width + xs[i]];
            let surface = if wet(i, j) {
                level(i, j)
            } else {
                let mut best: Option<f32> = None;
                for nj in j.saturating_sub(1)..=(j + 1).min(nz - 1) {
                    for ni in i.saturating_sub(1)..=(i + 1).min(nx - 1) {
                        if wet(ni, nj) {
                            best = Some(best.map_or(level(ni, nj), |b: f32| b.max(level(ni, nj))));
                        }
                    }
                }
                best.unwrap_or(ground)
            };
            out.positions.push([
                (xs[i] as f32 + 0.5) * settings.cell_size,
                surface * settings.height_scale,
                (zs[j] as f32 + 0.5) * settings.cell_size,
            ]);
            out.normals.push([0.0, 1.0, 0.0]);
            out.uvs.push([(surface - ground) * settings.height_scale, 0.0]);
        }
    }
    out.indices = grid_indices(nx, nz, |i, j| wet(i, j) || wet(i + 1, j) || wet(i, j + 1) || wet(i + 1, j + 1));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: usize, depth: usize, h: f32) -> Vec<f32> {
        vec![h; width * depth]
    }

    #[test]
    fn lod_samples_keep_the_edge() {
        assert_eq!(lod_samples(5, 1), vec![0, 1, 2, 3, 4]);
        assert_eq!(lod_samples(5, 2), vec![0, 2, 4]);
        assert_eq!(lod_samples(6, 2), vec![0, 2, 4, 5]);
        assert_eq!(lod_samples(6, 0), vec![0, 1, 2, 3, 4, 5]);
        assert!(lod_samples(0, 3).is_empty());
    }

    #[test]
    fn input_validation() {
        let s = TerrainSettings::default();
        assert_eq!(terrain_mesh(&[0.0], 1, 1, None, &s), Err(TerrainError::TooSmall { width: 1, depth: 1 }));
        assert_eq!(
            terrain_mesh(&[0.0; 5], 2, 3, None, &s),
            Err(TerrainError::Length { what: "heights", expected: 6, actual: 5 })
        );
        let mut h = flat(3, 3, 0.0);
        h[4] = f32::NAN;
        assert_eq!(terrain_mesh(&h, 3, 3, None, &s), Err(TerrainError::NonFinite { what: "heights", index: 4 }));
        let zero = TerrainSettings { lod_step: 0, ..s };
        assert_eq!(terrain_mesh(&flat(3, 3, 0.0), 3, 3, None, &zero), Err(TerrainError::ZeroStep));
        let colors = vec![Color::WHITE; 4];
        assert!(terrain_mesh(&flat(3, 3, 0.0), 3, 3, Some(&colors), &s).is_err());
        for (cell_size, height_scale) in [(0.0, 1.0), (-1.0, 1.0), (f32::NAN, 1.0), (1.0, f32::INFINITY)] {
            let bad = TerrainSettings { cell_size, height_scale, lod_step: 1 };
            assert!(
                matches!(terrain_mesh(&flat(3, 3, 0.0), 3, 3, None, &bad), Err(TerrainError::InvalidSettings { .. })),
                "{bad:?}"
            );
        }
        // Water-depth errors name the water buffer, not the heights.
        let mut water = flat(3, 3, 0.0);
        water[7] = f32::NAN;
        let err = water_mesh(&flat(3, 3, 0.0), &water, 3, 3, 0.0, &s).expect_err("NaN depth");
        assert_eq!(err, TerrainError::NonFinite { what: "water depth", index: 7 });
        assert!(err.to_string().starts_with("water depth"), "{err}");
    }

    #[test]
    fn flat_terrain_counts_and_up_normals() {
        let s = TerrainSettings { cell_size: 2.0, height_scale: 3.0, lod_step: 1 };
        let colors = vec![Color::srgb(0.2, 0.6, 0.1); 4 * 3];
        let m = terrain_mesh(&flat(4, 3, 1.0), 4, 3, Some(&colors), &s).expect("valid");
        assert_eq!(m.positions.len(), 12);
        assert_eq!(m.triangle_count(), 3 * 2 * 2);
        assert!(m.normals.iter().all(|n| *n == [0.0, 1.0, 0.0]));
        assert_eq!(m.positions[0], [1.0, 3.0, 1.0]);
        assert_eq!(m.positions[11], [7.0, 3.0, 5.0]);
        assert_eq!(m.colors.len(), 12);
        // Triangles face up (counter-clockwise seen from +Y).
        for tri in m.indices.as_chunks::<3>().0 {
            let p = |k: u32| Vec3::from_array(m.positions[k as usize]);
            let n = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            assert!(n.y > 0.0);
        }
    }

    #[test]
    fn slope_normals_match_analytic_gradient() {
        // h = 0.5 x (height units), cell size 1, height scale 2 → world slope 1 along x.
        let (w, d) = (5, 4);
        let heights: Vec<f32> = (0..w * d).map(|i| 0.5 * (i % w) as f32).collect();
        let s = TerrainSettings { cell_size: 1.0, height_scale: 2.0, lod_step: 1 };
        let m = terrain_mesh(&heights, w, d, None, &s).expect("valid");
        let expected = Vec3::new(-1.0, 1.0, 0.0).normalize();
        for n in &m.normals {
            assert!((Vec3::from_array(*n) - expected).length() < 1e-5, "{n:?}");
        }
        // Same slope survives decimation.
        let lod = TerrainSettings { lod_step: 2, ..s };
        let m = terrain_mesh(&heights, w, d, None, &lod).expect("valid");
        assert_eq!(m.positions.len(), 3 * 3);
        for n in &m.normals {
            assert!((Vec3::from_array(*n) - expected).length() < 1e-5);
        }
    }

    #[test]
    fn lod_reduces_triangles() {
        let heights = flat(65, 65, 0.0);
        let full = terrain_mesh(&heights, 65, 65, None, &TerrainSettings::default()).expect("valid");
        let lod = terrain_mesh(&heights, 65, 65, None, &TerrainSettings { lod_step: 4, ..default() }).expect("valid");
        assert_eq!(full.triangle_count(), 64 * 64 * 2);
        assert_eq!(lod.triangle_count(), 16 * 16 * 2);
    }

    #[test]
    fn water_surface_only_over_wet_cells() {
        let (w, d) = (5, 5);
        let terrain = flat(w, d, 1.0);
        let mut water = vec![0.0; w * d];
        water[2 * w + 2] = 0.5; // one wet cell in the middle
        let m = water_mesh(&terrain, &water, w, d, 0.01, &TerrainSettings::default()).expect("valid");
        // The four quads around the wet vertex.
        assert_eq!(m.triangle_count(), 8);
        // Wet vertex at the water level; its dry neighbours extend the level.
        assert_eq!(m.positions[2 * w + 2][1], 1.5);
        assert_eq!(m.positions[w + 1][1], 1.5);
        // Far corner stays at ground level with zero depth.
        assert_eq!(m.positions[0][1], 1.0);
        assert_eq!(m.uvs[2 * w + 2][0], 0.5);
        assert_eq!(m.uvs[w + 1][0], 0.5, "dry shore vertex: level − ground");
        assert!(m.colors.is_empty());
        let none = water_mesh(&terrain, &vec![0.0; w * d], w, d, 0.01, &TerrainSettings::default()).expect("valid");
        assert_eq!(none.triangle_count(), 0);
        assert!(water_mesh(&terrain, &[0.0; 3], w, d, 0.0, &TerrainSettings::default()).is_err());
    }
}
