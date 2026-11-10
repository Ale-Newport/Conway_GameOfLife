//! The 3D environment: a diorama slab of the model's grid.
//!
//! * **Coordinates.** World position `(x, y)` (cells, `y` down) maps to
//!   Bevy `(x, height, y)`: one cell is one unit, `+y` of the model is `+Z`.
//! * **Terrain.** Land is nearly flat; water cells are sunk below a water
//!   surface (the animated `rendering` water material) and mountains are
//!   raised, both deeper/higher away from their shores. The heightfield is
//!   padded by one cell on every side so the slab covers `[0, W] × [0, H]`;
//!   a skirt closes its sides.
//! * **Ground tint.** By default every land cell is tinted by its
//!   temperature on the same thermal scale as the prey coats (muted), so a
//!   creature wearing the colour of the ground beneath it sits at its
//!   thermal optimum. Alternative overlays: food, regions, plain.
//! * **Vegetation.** One `organism-render` grass tuft per land cell (fewer
//!   on huge grids) whose size follows the cell's food; rich patches carry
//!   flowers.

use std::f32::consts::FRAC_PI_2;

use bevy::light::CascadeShadowConfigBuilder;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCamera;
use evolution::config::Terrain;
use interface::{Palette, ThemeSettings};
use organism_render::plants::ground::FlowerColor;
use organism_render::plants::{GroundCover, GroundCoverPlant};
use rendering::terrain::{SurfaceMeshData, TerrainSettings, terrain_mesh, water_mesh};
use rendering::water::WaterSettings;
use rendering::{OrbitCameraRig, OrbitPreset, WaterMaterial};

use crate::MODE;
use crate::morph::{ground_rgb, thermal_t};
use crate::session::{EnvSnapshot, EvoData};
use crate::ui_state::{EvoUi, GroundOverlay};

/// Height of the water surface.
pub const WATER_LEVEL: f32 = -0.22;
/// Bottom of the diorama slab.
pub const BASE_Y: f32 = -2.4;
/// Cells added on each side of the grid so the slab covers the whole world.
const PAD: usize = 1;

/// World position → scene position at height `y`.
pub fn world_to_scene(p: Vec2, y: f32) -> Vec3 {
    Vec3::new(p.x, y, p.y)
}

/// Scene position → world position.
pub fn scene_to_world(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// Yaw (rotation about +Y) that turns an organism facing +Z towards the
/// model heading `heading` (radians, measured from +x towards +y).
pub fn heading_to_yaw(heading: f32) -> f32 {
    FRAC_PI_2 - heading
}

/// Deterministic hash of a lattice point to `[0, 1)`.
pub fn hash01(x: i32, y: i32, seed: u32) -> f32 {
    let mut h =
        (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Smooth value noise in `[0, 1)` with lattice spacing `scale` cells.
pub fn value_noise(x: f32, y: f32, scale: f32, seed: u32) -> f32 {
    let (fx, fy) = (x / scale, y / scale);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let a = hash01(x0, y0, seed);
    let b = hash01(x0 + 1, y0, seed);
    let c = hash01(x0, y0 + 1, seed);
    let d = hash01(x0 + 1, y0 + 1, seed);
    let top = a + (b - a) * s(tx);
    let bottom = c + (d - c) * s(tx);
    top + (bottom - top) * s(ty)
}

/// 4-neighbour distance (in cells) from every cell inside the set to the
/// nearest cell outside it (0 for cells outside). Cells beyond the grid
/// edge count as inside, so a set reaching the edge keeps growing there.
pub fn distance_field(inside: &[bool], w: usize, h: usize) -> Vec<u16> {
    let mut dist = vec![u16::MAX; w * h];
    let mut queue = std::collections::VecDeque::new();
    for (i, &is_in) in inside.iter().enumerate() {
        if !is_in {
            dist[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % w, i / w);
        let next = dist[i].saturating_add(1);
        let mut visit = |j: usize| {
            if dist[j] > next {
                dist[j] = next;
                queue.push_back(j);
            }
        };
        if x > 0 {
            visit(i - 1);
        }
        if x + 1 < w {
            visit(i + 1);
        }
        if y > 0 {
            visit(i - w);
        }
        if y + 1 < h {
            visit(i + w);
        }
    }
    dist
}

/// Heights of the padded grid (`(w + 2) × (h + 2)`; the outer ring repeats
/// the edge cells).
pub fn padded_heights(terrain: &[Terrain], w: usize, h: usize) -> Vec<f32> {
    let water: Vec<bool> = terrain.iter().map(|t| *t == Terrain::Water).collect();
    let mountain: Vec<bool> = terrain.iter().map(|t| *t == Terrain::Mountain).collect();
    let dw = distance_field(&water, w, h);
    let dm = distance_field(&mountain, w, h);
    let (pw, ph) = (w + 2 * PAD, h + 2 * PAD);
    let mut out = Vec::with_capacity(pw * ph);
    for py in 0..ph {
        for px in 0..pw {
            let x = px.saturating_sub(PAD).min(w - 1);
            let y = py.saturating_sub(PAD).min(h - 1);
            let i = y * w + x;
            let (fx, fy) = (px as f32, py as f32);
            let n = value_noise(fx, fy, 5.0, 11) - 0.5;
            let height = match terrain.get(i).copied().unwrap_or_default() {
                Terrain::Land => 0.05 * n,
                Terrain::Water => -0.42 - 0.2 * f32::from(dw[i].min(4)) + 0.06 * n,
                Terrain::Mountain => {
                    0.55 + 0.55 * f32::from(dm[i].min(5)) + 0.9 * (value_noise(fx, fy, 2.5, 23) - 0.3).max(0.0)
                }
            };
            out.push(height);
        }
    }
    out
}

/// Ground height of the padded grid sampled at world positions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeightField {
    /// Padded width.
    pub width: usize,
    /// Padded depth.
    pub depth: usize,
    /// Heights at padded cell centres.
    pub heights: Vec<f32>,
}

impl HeightField {
    /// Height field of a terrain grid.
    pub fn new(terrain: &[Terrain], w: usize, h: usize) -> Self {
        Self { width: w + 2 * PAD, depth: h + 2 * PAD, heights: padded_heights(terrain, w, h) }
    }

    /// Bilinear height at world position `p` (clamped to the slab).
    pub fn sample(&self, p: Vec2) -> f32 {
        if self.heights.is_empty() {
            return 0.0;
        }
        let gx = (p.x + PAD as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let gy = (p.y + PAD as f32 - 0.5).clamp(0.0, (self.depth - 1) as f32);
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.depth - 1));
        let (tx, ty) = (gx - x0 as f32, gy - y0 as f32);
        let h = |x: usize, y: usize| self.heights[y * self.width + x];
        let top = h(x0, y0) + (h(x1, y0) - h(x0, y0)) * tx;
        let bottom = h(x0, y1) + (h(x1, y1) - h(x0, y1)) * tx;
        top + (bottom - top) * ty
    }
}

/// Categorical colour of region `r` (sRGB 0..1).
pub fn region_rgb(r: u16) -> [f32; 3] {
    const COLORS: [[f32; 3]; 8] = [
        [0.36, 0.58, 0.80],
        [0.85, 0.55, 0.30],
        [0.45, 0.70, 0.40],
        [0.75, 0.45, 0.62],
        [0.82, 0.74, 0.36],
        [0.40, 0.68, 0.70],
        [0.62, 0.52, 0.80],
        [0.70, 0.62, 0.50],
    ];
    COLORS[usize::from(r) % COLORS.len()]
}

/// Inputs of [`ground_colors`].
pub struct GroundInputs<'a> {
    /// Overlay.
    pub overlay: GroundOverlay,
    /// Terrain (unpadded).
    pub terrain: &'a [Terrain],
    /// Grid width.
    pub width: usize,
    /// Grid height.
    pub height: usize,
    /// Padded heights.
    pub heights: &'a [f32],
    /// Temperature per cell.
    pub temperature: Option<&'a [f32]>,
    /// Food per cell.
    pub food: Option<&'a [f32]>,
    /// Food that counts as "full" (largest capacity).
    pub food_reference: f32,
    /// Region per cell.
    pub regions: Option<&'a [u16]>,
}

/// Brightness factor of the temperature tint for a cell holding `food`
/// relative to `reference` (the largest capacity): `0.58` on bare ground,
/// `1` at full capacity, growing with the square root of the fraction.
pub fn food_brightness(food: f32, reference: f32) -> f32 {
    let f = if food.is_finite() && reference > 0.0 { (food / reference).clamp(0.0, 1.0) } else { 1.0 };
    0.58 + 0.42 * f.sqrt()
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// Vertex colours (sRGB) of the padded terrain for an overlay.
pub fn ground_colors(input: &GroundInputs) -> Vec<Color> {
    let (w, h) = (input.width, input.height);
    let (pw, ph) = (w + 2 * PAD, h + 2 * PAD);
    let mut out = Vec::with_capacity(pw * ph);
    for py in 0..ph {
        for px in 0..pw {
            let x = px.saturating_sub(PAD).min(w - 1);
            let y = py.saturating_sub(PAD).min(h - 1);
            let i = y * w + x;
            let height = input.heights.get(py * pw + px).copied().unwrap_or(0.0);
            let jitter = 0.94 + 0.12 * hash01(px as i32, py as i32, 7);
            let temperature = input.temperature.and_then(|t| t.get(i)).copied();
            let rgb = match input.terrain.get(i).copied().unwrap_or_default() {
                Terrain::Water => mix3([0.30, 0.29, 0.22], [0.12, 0.16, 0.20], (-height - 0.4) / 0.9),
                Terrain::Mountain => {
                    let rock = mix3([0.40, 0.37, 0.34], [0.58, 0.56, 0.54], (height - 0.6) / 2.5);
                    let cold = temperature.is_some_and(|t| t < 2.0) && height > 1.4;
                    if cold { mix3(rock, [0.93, 0.95, 0.98], 0.8) } else { rock }
                }
                Terrain::Land => match input.overlay {
                    GroundOverlay::Temperature => {
                        let tint = match temperature {
                            Some(t) => ground_rgb(thermal_t(t)),
                            None => [0.40, 0.42, 0.34],
                        };
                        // Hue = temperature, brightness = food: grazed or
                        // poor ground is darker, so food patches stand out
                        // without changing the thermal hue.
                        let light = input
                            .food
                            .and_then(|f| f.get(i))
                            .map_or(1.0, |f| food_brightness(*f, input.food_reference));
                        tint.map(|c| c * light)
                    }
                    GroundOverlay::Food => {
                        let f = input.food.and_then(|f| f.get(i)).copied().unwrap_or(0.0);
                        let t = (f / input.food_reference.max(1e-3)).clamp(0.0, 1.0).sqrt();
                        mix3([0.33, 0.27, 0.20], [0.30, 0.62, 0.22], t)
                    }
                    GroundOverlay::Regions => match input.regions.and_then(|r| r.get(i)).copied() {
                        Some(r) if r != u16::MAX => mix3(region_rgb(r), [0.25, 0.25, 0.25], 0.25),
                        _ => [0.30, 0.30, 0.30],
                    },
                    GroundOverlay::Plain => [0.43, 0.45, 0.38],
                },
            };
            let [r, g, b] = rgb.map(|c| (c * jitter).clamp(0.0, 1.0));
            out.push(Color::srgb(r, g, b));
        }
    }
    out
}

/// Vertical walls closing the sides of the padded terrain slab, down to
/// [`BASE_Y`], in the coordinates of [`terrain_mesh`] (vertex at padded
/// cell centres).
pub fn skirt_mesh(heights: &[f32], pw: usize, ph: usize) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let top_color = Color::srgb(0.24, 0.19, 0.15).to_linear().to_f32_array();
    let bottom_color = Color::srgb(0.10, 0.085, 0.075).to_linear().to_f32_array();
    let at = |x: usize, z: usize| -> [f32; 3] { [x as f32 + 0.5, heights[z * pw + x], z as f32 + 0.5] };
    // Each edge as a list of (x, z) cells walked so the outward normal is
    // on the right-hand side; plus that normal.
    type Edge = (Vec<(usize, usize)>, [f32; 3]);
    let edges: [Edge; 4] = [
        ((0..pw).map(|x| (x, 0)).collect(), [0.0, 0.0, -1.0]),
        ((0..pw).rev().map(|x| (x, ph - 1)).collect(), [0.0, 0.0, 1.0]),
        ((0..ph).rev().map(|z| (0, z)).collect(), [-1.0, 0.0, 0.0]),
        ((0..ph).map(|z| (pw - 1, z)).collect(), [1.0, 0.0, 0.0]),
    ];
    for (cells, normal) in edges {
        for pair in cells.windows(2) {
            let (a, b) = (at(pair[0].0, pair[0].1), at(pair[1].0, pair[1].1));
            let base = positions.len() as u32;
            positions.extend([a, b, [b[0], BASE_Y, b[2]], [a[0], BASE_Y, a[2]]]);
            normals.extend([normal; 4]);
            colors.extend([top_color, top_color, bottom_color, bottom_color]);
            // Wind both ways is not needed: pick the winding facing outwards.
            let outward = Vec3::from(normal);
            let e1 = Vec3::from(b) - Vec3::from(a);
            let e2 = Vec3::new(a[0], BASE_Y, a[2]) - Vec3::from(a);
            if e1.cross(e2).dot(outward) > 0.0 {
                indices.extend([base, base + 1, base + 3, base + 1, base + 2, base + 3]);
            } else {
                indices.extend([base, base + 3, base + 1, base + 1, base + 3, base + 2]);
            }
        }
    }
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

/// Number of vegetation size levels.
pub const TUFT_LEVELS: u8 = 6;

/// Food below this fraction of the largest capacity shows no vegetation
/// (bare, grazed or poor ground).
pub const BARE_FRACTION: f32 = 0.08;

/// Vegetation level (0 = bare) of a cell holding `food`, relative to the
/// largest capacity.
pub fn tuft_level(food: f32, reference: f32) -> u8 {
    if !(food.is_finite() && reference > 0.0) || food < BARE_FRACTION * reference {
        return 0;
    }
    let t = (food / reference).clamp(0.0, 1.0).sqrt();
    ((t * f32::from(TUFT_LEVELS)).ceil() as u8).clamp(1, TUFT_LEVELS)
}

/// Scale of a tuft at `level`.
pub fn tuft_scale(level: u8) -> f32 {
    if level == 0 { 0.0 } else { 0.55 + 0.2 * f32::from(level.min(TUFT_LEVELS)) }
}

/// Stride between vegetated cells (keeps at most ~8000 tufts).
pub fn tuft_stride(w: usize, h: usize) -> usize {
    let cells = (w * h) as f32;
    (cells / 8_000.0).sqrt().ceil().max(1.0) as usize
}

/// Camera distance at which a perspective camera with vertical field of
/// view `fov` and viewport aspect `aspect` (width / height), looking at the
/// world centre from `pitch` above the horizon, sees the whole `w × h`
/// world (one cell of margin).
///
/// The near edge lies `a = h/2 + 1` closer along the ground: in camera space
/// it sits at depth `d − a cos p` and `a sin p` below the axis, and it is the
/// widest part of the slab on screen. So the distance must satisfy
/// `d ≥ a cos p + (w + 2) / (2 tan(fov_h / 2))` (width of the near edge) and
/// `d ≥ a cos p + a sin p / tan(fov_v / 2)` (near edge inside the bottom).
pub fn framing_distance(w: usize, h: usize, fov: f32, aspect: f32, pitch: f32) -> f32 {
    let half_v = (fov * 0.5).tan().max(1e-3);
    let half_h = half_v * aspect.clamp(0.2, 5.0);
    let a = h as f32 * 0.5 + 1.0;
    let near = a * pitch.cos();
    let by_width = near + (w as f32 + 2.0) / (2.0 * half_h);
    let by_depth = near + a * pitch.sin() / half_v;
    by_width.max(by_depth) * 1.06
}

/// Camera pitch used when framing the world (radians above the horizon).
pub const FRAMING_PITCH: f32 = 0.8;

// ---------------------------------------------------------------- systems

/// Marks the mode's world camera.
#[derive(Component, Debug)]
pub struct EvoCamera;

/// Marks the sun.
#[derive(Component, Debug)]
pub struct EvoSun;

/// The terrain surface.
#[derive(Component, Debug)]
pub struct TerrainSurface;

/// The water surface.
#[derive(Component, Debug)]
pub struct WaterSurface;

/// The slab's side walls.
#[derive(Component, Debug)]
pub struct Skirt;

/// A vegetation tuft on cell `cell`.
#[derive(Component, Debug, Clone, Copy)]
pub struct Tuft {
    /// Cell index.
    pub cell: usize,
    /// Level shown.
    pub level: u8,
}

/// Parent of every tuft.
#[derive(Component, Debug)]
pub struct VegetationRoot;

/// Built environment state.
#[derive(Resource, Default)]
pub struct SceneState {
    /// Heights for placing organisms.
    pub field: HeightField,
    /// World size of the built scene.
    pub size: Option<(usize, usize)>,
    terrain: Vec<Terrain>,
    terrain_mesh: Option<Handle<Mesh>>,
    colors_key: Option<(GroundOverlay, u64, u64)>,
    food_colored_at: f64,
    vegetation_at: f64,
    vegetation_env_seq: u64,
    /// Ask the camera to frame the world.
    pub frame_request: bool,
    frame_wait: u32,
    frame_stable: u32,
    framed_viewport: Option<UVec2>,
}

/// `OnEnter`: camera, sun and the empty scene.
pub fn spawn_scene(mut commands: Commands, theme: Res<ThemeSettings>, mut state: ResMut<SceneState>) {
    *state = SceneState { frame_request: true, ..SceneState::default() };
    let palette = Palette::new(theme.kind, MODE.default_accent());
    let background = interface::theme::bevy_color(palette.background);
    commands.spawn((
        Name::new("Evolution camera"),
        EvoCamera,
        OrbitCameraRig::new(OrbitPreset::Strategic).with_distance_limits(3.0, 400.0),
        PanOrbitCamera {
            focus: Vec3::new(32.0, 0.0, 24.0),
            radius: Some(80.0),
            yaw: Some(0.0),
            pitch: Some(0.95),
            ..default()
        },
        Transform::from_xyz(32.0, 60.0, 80.0).looking_at(Vec3::new(32.0, 0.0, 24.0), Vec3::Y),
        AmbientLight { color: Color::srgb(0.82, 0.88, 1.0), brightness: 420.0, ..default() },
        DistanceFog {
            color: background,
            directional_light_color: Color::NONE,
            directional_light_exponent: 30.0,
            falloff: FogFalloff::Linear { start: 140.0, end: 320.0 },
        },
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Evolution sun"),
        EvoSun,
        DirectionalLight {
            illuminance: 9_500.0,
            shadow_maps_enabled: true,
            color: Color::srgb(1.0, 0.97, 0.92),
            ..default()
        },
        Transform::from_xyz(-40.0, 70.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder { maximum_distance: 260.0, first_cascade_far_bound: 70.0, ..default() }.build(),
        DespawnOnExit(MODE),
    ));
}

fn surface_settings() -> TerrainSettings {
    TerrainSettings { cell_size: 1.0, height_scale: 1.0, lod_step: 1 }
}

/// Filter of the entities rebuilt with the terrain.
type BuiltSurface = Or<(With<TerrainSurface>, With<WaterSurface>, With<Skirt>, With<VegetationRoot>)>;

/// Rebuilds terrain, water, skirt and vegetation when the terrain changes.
#[allow(clippy::too_many_arguments)]
pub fn rebuild_environment(
    mut commands: Commands,
    data: Res<EvoData>,
    mut state: ResMut<SceneState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    old: Query<Entity, BuiltSurface>,
    sun: Query<&Transform, With<EvoSun>>,
) {
    let Some(view) = data.view.as_ref() else { return };
    let (w, h) = (view.width, view.height);
    if w < 2 || h < 2 || view.terrain.len() != w * h {
        return;
    }
    if state.size == Some((w, h)) && state.terrain == view.terrain {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    if state.size != Some((w, h)) {
        state.frame_request = true;
    }
    let field = HeightField::new(&view.terrain, w, h);
    let (pw, ph) = (field.width, field.depth);
    let offset = Transform::from_xyz(-(PAD as f32), 0.0, -(PAD as f32));
    let settings = surface_settings();
    let placeholder = vec![Color::srgb(0.4, 0.42, 0.35); pw * ph];
    match terrain_mesh(&field.heights, pw, ph, Some(&placeholder), &settings) {
        Ok(mesh) => {
            let handle = meshes.add(mesh.to_mesh());
            let material = materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.95,
                reflectance: 0.2,
                ..default()
            });
            commands.spawn((
                Name::new("Terrain"),
                TerrainSurface,
                Mesh3d(handle.clone()),
                MeshMaterial3d(material),
                offset,
                DespawnOnExit(MODE),
            ));
            state.terrain_mesh = Some(handle);
        }
        Err(e) => {
            error!("evolution terrain: {e}");
            return;
        }
    }
    let skirt_material =
        materials.add(StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 1.0, ..default() });
    commands.spawn((
        Name::new("Terrain skirt"),
        Skirt,
        Mesh3d(meshes.add(skirt_mesh(&field.heights, pw, ph))),
        MeshMaterial3d(skirt_material),
        offset,
        DespawnOnExit(MODE),
    ));
    let depth: Vec<f32> = padded_water(&view.terrain, w, h, &field.heights);
    if depth.iter().any(|d| *d > 0.0) {
        match water_mesh(&field.heights, &depth, pw, ph, 0.01, &settings) {
            Ok(mesh) => {
                let sun = sun.iter().next().copied().unwrap_or_default();
                let water = WaterMaterial {
                    settings: WaterSettings {
                        waves: Vec4::new(0.06, 3.0, 0.8, 1.2),
                        ..WaterSettings::default().with_sun_from(&sun)
                    },
                };
                commands.spawn((
                    Name::new("Water"),
                    WaterSurface,
                    Mesh3d(meshes.add(mesh_with_colors(mesh))),
                    MeshMaterial3d(water_materials.add(water)),
                    offset,
                    DespawnOnExit(MODE),
                ));
            }
            Err(e) => warn!("evolution water: {e}"),
        }
    }
    // Vegetation.
    let root = commands
        .spawn((
            Name::new("Vegetation"),
            VegetationRoot,
            Transform::default(),
            Visibility::default(),
            DespawnOnExit(MODE),
        ))
        .id();
    let stride = tuft_stride(w, h);
    for y in (0..h).step_by(stride) {
        for x in (0..w).step_by(stride) {
            let cell = y * w + x;
            if view.terrain[cell] != Terrain::Land {
                continue;
            }
            let jx = hash01(x as i32, y as i32, 3) - 0.5;
            let jy = hash01(x as i32, y as i32, 5) - 0.5;
            let p = Vec2::new(x as f32 + 0.5 + 0.6 * jx, y as f32 + 0.5 + 0.6 * jy);
            let flower = hash01(x as i32, y as i32, 9);
            let kind = if flower > 0.93 {
                GroundCover::Flower(
                    [FlowerColor::Yellow, FlowerColor::White, FlowerColor::Blue][(flower * 997.0) as usize % 3],
                )
            } else {
                GroundCover::Grass
            };
            commands.spawn((
                GroundCoverPlant { kind, variant: (hash01(x as i32, y as i32, 13) * 4.0) as u8, burnt: false },
                Tuft { cell, level: 0 },
                Transform::from_translation(world_to_scene(p, field.sample(p)))
                    .with_rotation(Quat::from_rotation_y(hash01(x as i32, y as i32, 17) * std::f32::consts::TAU))
                    .with_scale(Vec3::ZERO),
                Visibility::Hidden,
                ChildOf(root),
            ));
        }
    }
    state.field = field;
    state.size = Some((w, h));
    state.terrain = view.terrain.clone();
    state.colors_key = None;
    state.vegetation_at = 0.0;
    state.vegetation_env_seq = u64::MAX;
}

fn mesh_with_colors(data: SurfaceMeshData) -> Mesh {
    data.to_mesh()
}

/// Water depth on the padded grid (water cells only).
pub fn padded_water(terrain: &[Terrain], w: usize, h: usize, heights: &[f32]) -> Vec<f32> {
    let (pw, ph) = (w + 2 * PAD, h + 2 * PAD);
    let mut out = vec![0.0; pw * ph];
    for py in 0..ph {
        for px in 0..pw {
            let x = px.saturating_sub(PAD).min(w - 1);
            let y = py.saturating_sub(PAD).min(h - 1);
            if terrain.get(y * w + x) == Some(&Terrain::Water) {
                out[py * pw + px] = (WATER_LEVEL - heights[py * pw + px]).max(0.0);
            }
        }
    }
    out
}

/// Whether the ground tint of `overlay` depends on the food field.
pub fn overlay_shows_food(overlay: GroundOverlay) -> bool {
    matches!(overlay, GroundOverlay::Temperature | GroundOverlay::Food)
}

/// Minimum time between two food-driven recolourings of a `cells`-cell grid (s).
pub fn recolor_interval(cells: usize) -> f64 {
    if cells <= 20_000 { 0.25 } else { 1.0 }
}

/// Recolours the ground when the overlay or its data change.
pub fn recolor_ground(
    data: Res<EvoData>,
    ui: Res<EvoUi>,
    time: Res<Time<Real>>,
    mut state: ResMut<SceneState>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (Some(view), Some(handle)) = (data.view.as_ref(), state.terrain_mesh.clone()) else { return };
    let Some((w, h)) = state.size else { return };
    let now = time.elapsed_secs_f64();
    let food_key = if overlay_shows_food(ui.overlay) { data.view_seq } else { 0 };
    let key = (ui.overlay, data.env_seq, food_key);
    if state.colors_key == Some(key) {
        return;
    }
    // Food changes every tick: limit recolouring (a vertex-colour upload of
    // the whole terrain) to a few times per second, less on huge grids.
    if state.colors_key.is_some_and(|k| k.0 == ui.overlay && k.1 == data.env_seq)
        && now - state.food_colored_at < recolor_interval(w * h)
    {
        return;
    }
    let env: Option<&EnvSnapshot> = data.env.as_deref().filter(|e| e.width == w && e.height == h);
    let food = view.field.as_ref().map(|f| f.values.as_slice()).filter(|f| f.len() == w * h);
    let colors = ground_colors(&GroundInputs {
        overlay: ui.overlay,
        terrain: &view.terrain,
        width: w,
        height: h,
        heights: &state.field.heights,
        temperature: env.map(|e| e.temperature.as_slice()),
        food,
        food_reference: env.map_or(10.0, EnvSnapshot::max_capacity),
        regions: env.map(|e| e.regions.as_slice()),
    });
    if let Some(mut mesh) = meshes.get_mut(&handle) {
        let linear: Vec<[f32; 4]> = colors.iter().map(|c| c.to_linear().to_f32_array()).collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, linear);
    }
    state.colors_key = Some(key);
    state.food_colored_at = now;
}

/// Sizes the grass tufts after the food on their cells.
pub fn update_vegetation(
    data: Res<EvoData>,
    ui: Res<EvoUi>,
    time: Res<Time<Real>>,
    mut state: ResMut<SceneState>,
    mut tufts: Query<(&mut Tuft, &mut Transform, &mut Visibility)>,
) {
    let Some(view) = data.view.as_ref() else { return };
    let now = time.elapsed_secs_f64();
    if now - state.vegetation_at < 0.2 && state.vegetation_env_seq == data.env_seq {
        return;
    }
    let Some(food) = view.field.as_ref().map(|f| &f.values) else { return };
    let reference = data.env.as_ref().map_or(10.0, |e| e.max_capacity());
    state.vegetation_at = now;
    state.vegetation_env_seq = data.env_seq;
    for (mut tuft, mut transform, mut visibility) in &mut tufts {
        let level = if ui.vegetation { tuft_level(food.get(tuft.cell).copied().unwrap_or(0.0), reference) } else { 0 };
        if level == tuft.level {
            continue;
        }
        tuft.level = level;
        transform.scale = Vec3::splat(tuft_scale(level));
        let wanted = if level == 0 { Visibility::Hidden } else { Visibility::Inherited };
        visibility.set_if_neq(wanted);
    }
}

/// Keeps fog in step with the theme and applies camera presets / framing.
pub fn update_camera(
    theme: Res<ThemeSettings>,
    ui: Res<EvoUi>,
    mut state: ResMut<SceneState>,
    mut cameras: Query<
        (&mut OrbitCameraRig, &mut PanOrbitCamera, &Projection, &Camera, &mut DistanceFog),
        With<EvoCamera>,
    >,
) {
    let palette = Palette::new(theme.kind, MODE.default_accent());
    let background = interface::theme::bevy_color(palette.background);
    for (mut rig, mut cam, projection, camera, mut fog) in &mut cameras {
        if fog.color != background {
            fog.color = background;
        }
        if rig.preset != ui.camera_preset {
            rig.preset = ui.camera_preset;
        }
        // PanOrbitCamera overwrites its targets when it initialises, and the
        // window and shell panels settle over the first frames (the viewport
        // aspect decides the distance): keep framing until the viewport size
        // has been stable for a few frames (or a timeout).
        let size = camera.viewport.as_ref().map(|v| v.physical_size);
        if state.frame_request {
            state.frame_wait += 1;
            if size != state.framed_viewport {
                state.framed_viewport = size;
                state.frame_stable = 0;
            } else {
                state.frame_stable += 1;
            }
        }
        if state.frame_request
            && cam.initialized
            && let Some((w, h)) = state.size
        {
            let centre = Vec3::new(w as f32 * 0.5, 0.0, h as f32 * 0.5);
            let aspect = camera.logical_viewport_size().map_or(1.4, |s| s.x / s.y.max(1.0));
            let distance = framing_distance(w, h, rig.fov, aspect, FRAMING_PITCH);
            rig.focus_on(&mut cam, projection, centre, distance);
            if rig.preset == OrbitPreset::Strategic || rig.preset == OrbitPreset::FreeOrbit {
                cam.target_pitch = FRAMING_PITCH;
                cam.target_yaw = 0.0;
            }
            let diagonal = ((w * w + h * h) as f32).sqrt();
            fog.falloff =
                FogFalloff::Linear { start: distance + diagonal * 0.8, end: distance + diagonal * 2.6 + 60.0 };
            if (size.is_some() && state.frame_stable >= 8) || state.frame_wait > 120 {
                state.frame_request = false;
                state.frame_wait = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_round_trip_and_headings_face_forward() {
        let p = Vec2::new(12.5, 7.25);
        assert_eq!(scene_to_world(world_to_scene(p, 3.0)), p);
        for heading in [0.0f32, 0.7, FRAC_PI_2, 3.0, -1.2] {
            let facing = Quat::from_rotation_y(heading_to_yaw(heading)) * Vec3::Z;
            let expected = world_to_scene(Vec2::new(heading.cos(), heading.sin()), 0.0);
            assert!((facing - expected).length() < 1e-5, "{heading}: {facing:?} vs {expected:?}");
        }
    }

    #[test]
    fn noise_is_deterministic_and_bounded() {
        assert_eq!(hash01(3, 4, 1), hash01(3, 4, 1));
        assert_ne!(hash01(3, 4, 1), hash01(4, 3, 1));
        for i in 0..200 {
            let v = value_noise(i as f32 * 0.37, i as f32 * 0.11, 4.0, 2);
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn distance_field_measures_steps_to_the_outside() {
        // 5 × 1 strip: inside cells 1..=3.
        let inside = [false, true, true, true, false];
        assert_eq!(distance_field(&inside, 5, 1), vec![0, 1, 2, 1, 0]);
        let all = [true; 4];
        assert!(distance_field(&all, 2, 2).iter().all(|d| *d == u16::MAX));
    }

    #[test]
    fn heights_sink_water_and_raise_mountains() {
        let (w, h) = (6, 4);
        let mut terrain = vec![Terrain::Land; w * h];
        terrain[0] = Terrain::Water;
        terrain[w * h - 1] = Terrain::Mountain;
        let field = HeightField::new(&terrain, w, h);
        assert_eq!((field.width, field.depth), (w + 2, h + 2));
        let land = field.sample(Vec2::new(3.5, 2.5));
        assert!(land.abs() < 0.06);
        assert!(field.sample(Vec2::new(0.5, 0.5)) < WATER_LEVEL - 0.1, "water bed below the surface");
        assert!(field.sample(Vec2::new(5.5, 3.5)) > 0.5, "mountain above land");
        // Sampling outside the grid is clamped, never panics.
        assert!(field.sample(Vec2::new(-50.0, 99.0)).is_finite());
        let water = padded_water(&terrain, w, h, &field.heights);
        assert!(water[field.width + 1] > 0.0 && water[field.width * 2 + 3] == 0.0);
    }

    #[test]
    fn ground_colours_follow_the_overlay() {
        let (w, h) = (3, 1);
        let terrain = vec![Terrain::Land; 3];
        let heights = padded_heights(&terrain, w, h);
        let temps = [5.0, 17.5, 40.0];
        let colors = |overlay| {
            ground_colors(&GroundInputs {
                overlay,
                terrain: &terrain,
                width: w,
                height: h,
                heights: &heights,
                temperature: Some(&temps),
                food: Some(&[0.0, 5.0, 10.0]),
                food_reference: 10.0,
                regions: Some(&[0, 1, u16::MAX]),
            })
        };
        let t = colors(GroundOverlay::Temperature);
        assert_eq!(t.len(), (w + 2) * (h + 2));
        let srgb = |c: Color| c.to_srgba();
        let (cold, hot) = (srgb(t[(w + 2) + 1]), srgb(t[(w + 2) + 3]));
        assert!(cold.blue > cold.red && hot.red > hot.blue, "cold ground is blue, hot ground red");
        let f = colors(GroundOverlay::Food);
        assert!(srgb(f[(w + 2) + 3]).green > srgb(f[(w + 2) + 1]).green, "more food is greener");
        let r = colors(GroundOverlay::Regions);
        assert_ne!(r[(w + 2) + 1], r[(w + 2) + 2]);
    }

    #[test]
    fn temperature_tint_is_darkened_by_missing_food_without_changing_hue() {
        let (w, h) = (2, 1);
        let terrain = vec![Terrain::Land; 2];
        // Flat heights so the per-vertex jitter is the only other difference.
        let heights = vec![0.0; (w + 2) * (h + 2)];
        let t = ground_colors(&GroundInputs {
            overlay: GroundOverlay::Temperature,
            terrain: &terrain,
            width: w,
            height: h,
            heights: &heights,
            temperature: Some(&[20.0, 20.0]),
            food: Some(&[0.0, 10.0]),
            food_reference: 10.0,
            regions: None,
        });
        let jitter = |px: i32, py: i32| 0.94 + 0.12 * hash01(px, py, 7);
        let (bare, full) = (t[(w + 2) + 1].to_srgba(), t[(w + 2) + 2].to_srgba());
        let (jb, jf) = (jitter(1, 1), jitter(2, 1));
        let ratio = (bare.green / jb) / (full.green / jf);
        assert!((ratio - food_brightness(0.0, 10.0)).abs() < 1e-3, "{ratio}");
        assert!(((bare.red / bare.green) - (full.red / full.green)).abs() < 1e-3, "same hue");
        assert_eq!(food_brightness(10.0, 10.0), 1.0);
        assert_eq!(food_brightness(f32::NAN, 10.0), 1.0, "unknown food leaves the tint alone");
        assert!(overlay_shows_food(GroundOverlay::Temperature) && !overlay_shows_food(GroundOverlay::Regions));
        assert!(recolor_interval(512 * 512) > recolor_interval(64 * 48));
    }

    #[test]
    fn vegetation_levels() {
        assert_eq!(tuft_level(0.0, 10.0), 0);
        assert_eq!(tuft_level(0.7, 10.0), 0, "below 8 % of capacity the ground is bare");
        assert!(tuft_level(0.9, 10.0) >= 1, "just above the threshold grows grass");
        assert_eq!(tuft_level(10.0, 10.0), TUFT_LEVELS);
        assert_eq!(tuft_level(50.0, 10.0), TUFT_LEVELS, "food above capacity is clamped");
        assert!(tuft_level(2.5, 10.0) < tuft_level(9.0, 10.0));
        assert_eq!(tuft_level(f32::NAN, 10.0), 0);
        assert_eq!(tuft_scale(0), 0.0);
        assert!(tuft_scale(2) > tuft_scale(1));
        assert_eq!(tuft_stride(96, 64), 1);
        assert!(tuft_stride(512, 512) >= 5);
    }

    #[test]
    fn skirt_has_four_walls() {
        let heights = vec![0.0; 4 * 3];
        let mesh = skirt_mesh(&heights, 4, 3);
        // (pw − 1) quads on two walls and (ph − 1) on the other two.
        let quads = 2 * (4 - 1) + 2 * (3 - 1);
        assert_eq!(mesh.count_vertices(), quads * 4);
        let fov = std::f32::consts::FRAC_PI_4;
        let wide = framing_distance(64, 48, fov, 2.0, FRAMING_PITCH);
        let narrow = framing_distance(64, 48, fov, 0.8, FRAMING_PITCH);
        assert!(narrow > wide, "a narrow viewport needs a farther camera");
        // The near edge fits: at its depth the visible width covers the world.
        let a = 48.0 * 0.5 + 1.0;
        let near_depth = wide - a * FRAMING_PITCH.cos();
        assert!(2.0 * near_depth * (fov * 0.5).tan() * 2.0 >= 66.0);
        // And it stays above the bottom of the view.
        assert!(a * FRAMING_PITCH.sin() / near_depth <= (fov * 0.5).tan() + 1e-4);
    }
}
