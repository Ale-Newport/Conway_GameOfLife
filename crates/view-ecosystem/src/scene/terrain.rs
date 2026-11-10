//! The landscape: terrain mesh from the model's elevation, coloured by
//! biome, vegetation cover, snow, ash and fire; water bodies with the
//! rendering crate's water material; the data overlay draped over both;
//! an earth skirt around the map.
//!
//! The terrain geometry is rebuilt only when the static terrain changes
//! (new epoch or terrain version); colours, water and the overlay are
//! refreshed at most a few times per second and only when a new frame
//! arrived.

use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ecosystem::species::PlantLayer;
use interface::SessionStatus;
use organism_render::PlantSeason;
use physics_world::fire::{MIN_FUEL_KG_M2, SpreadInputs, fuel_moisture, spread_probability};
use physics_world::{Biome, FireState, SurfaceKind, WorldLayer};
use rendering::WaterMaterial;
use rendering::terrain::{TerrainSettings, water_mesh};
use rendering::water::WaterSettings;
use simulation_core::Grid2;

use crate::MODE;
use crate::coords::MapGeometry;
use crate::palette::{GroundSample, Overlay, ground_color, linear, overlay_scale};
use crate::session::Frame;
use crate::state::{EcoData, EcoUi};

/// Seconds between two refreshes of colours, water and overlay.
pub const REFRESH_SECONDS: f64 = 0.3;
/// Water shallower than this (m) is not drawn.
pub const MIN_WATER_DEPTH: f32 = 0.02;
/// Height of the overlay above the ground or water (m).
pub const OVERLAY_LIFT: f32 = 0.6;
/// Opacity of the overlay.
pub const OVERLAY_ALPHA: f32 = 0.74;
/// Metres covered by one repeat of the ground detail texture.
pub const DETAIL_TILE_M: f32 = 17.0;
/// Mean brightness of the detail texture (vertex colours are divided by it).
pub const DETAIL_MEAN: f32 = 0.86;
/// Side of the detail texture (texels).
const DETAIL_SIZE: u32 = 256;

/// Terrain surface entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct TerrainSurface;

/// Water surface entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct WaterSurface;

/// Overlay surface entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct OverlaySurface;

/// Skirt around the map.
#[derive(Component, Debug, Clone, Copy)]
pub struct Skirt;

/// What the landscape was built from.
#[derive(Resource, Default)]
pub struct MapState {
    /// Epoch of the built terrain.
    pub epoch: Option<u64>,
    /// Terrain version of the built terrain.
    pub terrain_version: Option<u64>,
    /// Scenario the terrain belongs to.
    pub scenario: Option<String>,
    /// Geometry and water surface.
    pub geometry: Option<MapGeometry>,
    /// Biome per sample.
    pub biome: Vec<Biome>,
    /// Surface kind per sample.
    pub surface: Vec<SurfaceKind>,
    /// Terrain mesh.
    terrain_mesh: Option<Handle<Mesh>>,
    /// Overlay mesh.
    overlay_mesh: Option<Handle<Mesh>>,
    /// Water material.
    water_material: Option<Handle<WaterMaterial>>,
    /// Water depth the water mesh was built from.
    water_depth: Vec<f32>,
    /// Real time of the last refresh.
    last_refresh: f64,
    /// Frame sequence of the last refresh.
    refreshed_seq: u64,
    /// Overlay drawn last.
    overlay: Option<Overlay>,
    /// Smallest and largest value of the current overlay (legend).
    pub overlay_range: (f32, f32),
    /// Overlay values per sample (for hover read-outs).
    pub overlay_values: Vec<Option<f32>>,
    /// Sky, light and wind the water was shaded for.
    water_look: Option<WaterLook>,
    /// The overlay surface must be re-shaped (the water surface moved).
    overlay_shape_stale: bool,
}

impl MapState {
    /// Overlay value at a scene x/z, if any.
    pub fn overlay_at(&self, x: f32, z: f32) -> Option<f32> {
        let g = self.geometry.as_ref()?;
        let i = g.sample_of_cell(g.plane_to_cell(x, z))?;
        self.overlay_values.get(i).copied().flatten()
    }
}

/// Vertex coordinates along one axis of a grid of `n` samples spaced `s`:
/// the map edge, every sample centre, the far edge.
pub fn axis(n: usize, spacing: f32) -> Vec<f32> {
    let mut v = Vec::with_capacity(n + 2);
    v.push(0.0);
    v.extend((0..n).map(|i| (i as f32 + 0.5) * spacing));
    v.push(n as f32 * spacing);
    v
}

/// Sample index of vertex `i` of an [`axis`] of `n` samples.
pub fn sample_index(i: usize, n: usize) -> usize {
    i.saturating_sub(1).min(n.saturating_sub(1))
}

/// Builds a surface over the sample grid extended to the map edges, with
/// heights `height(sample index)` and smooth normals.
pub fn grid_surface(geometry: &MapGeometry, height: impl Fn(usize) -> f32) -> Mesh {
    let (w, h) = (geometry.width, geometry.height);
    let xs = axis(w, geometry.spacing);
    let zs = axis(h, geometry.spacing);
    let (nx, nz) = (xs.len(), zs.len());
    let y = |i: usize, j: usize| height(sample_index(j, h) * w + sample_index(i, w));
    let mut positions = Vec::with_capacity(nx * nz);
    let mut normals = Vec::with_capacity(nx * nz);
    let mut uvs = Vec::with_capacity(nx * nz);
    for j in 0..nz {
        for i in 0..nx {
            positions.push([xs[i], y(i, j), zs[j]]);
            let (i0, i1) = (i.saturating_sub(1), (i + 1).min(nx - 1));
            let (j0, j1) = (j.saturating_sub(1), (j + 1).min(nz - 1));
            let dx = (xs[i1] - xs[i0]).max(1e-3);
            let dz = (zs[j1] - zs[j0]).max(1e-3);
            let sx = (y(i1, j) - y(i0, j)) / dx;
            let sz = (y(i, j1) - y(i, j0)) / dz;
            normals.push(Vec3::new(-sx, 1.0, -sz).normalize().to_array());
            uvs.push([xs[i] / DETAIL_TILE_M, zs[j] / DETAIL_TILE_M]);
        }
    }
    let mut indices = Vec::with_capacity((nx - 1) * (nz - 1) * 6);
    for j in 0..nz - 1 {
        for i in 0..nx - 1 {
            let a = (j * nx + i) as u32;
            let (b, c) = (a + 1, a + nx as u32);
            indices.extend_from_slice(&[a, c, b, b, c, c + 1]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; nx * nz])
        .with_inserted_indices(Indices::U32(indices))
}

/// Tileable value noise in `[0, 1]` with `period` lattice cells per tile.
fn tile_noise(x: f32, y: f32, period: u32, seed: u32) -> f32 {
    let hash = |i: u32, j: u32| {
        let mut h = (i % period).wrapping_mul(0x8da6_b343) ^ (j % period).wrapping_mul(0xd816_3841) ^ seed;
        h ^= h >> 13;
        h = h.wrapping_mul(0x5bd1_e995);
        h ^= h >> 15;
        (h & 0xffff) as f32 / 65535.0
    };
    let (fx, fy) = (x * period as f32, y * period as f32);
    let (i, j) = (fx.floor() as u32, fy.floor() as u32);
    let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let a = hash(i, j) + (hash(i + 1, j) - hash(i, j)) * sx;
    let b = hash(i, j + 1) + (hash(i + 1, j + 1) - hash(i, j + 1)) * sx;
    a + (b - a) * sy
}

/// Brightness of the ground detail texture at `(u, v)` in `[0, 1)`:
/// three octaves of tileable value noise mapped to `[0.70, 1.0]` (mean
/// about [`DETAIL_MEAN`]), a mottled sward or soil seen up close.
pub fn detail_value(u: f32, v: f32) -> f32 {
    let n = 0.45 * tile_noise(u, v, 12, 11) + 0.35 * tile_noise(u, v, 36, 23) + 0.2 * tile_noise(u, v, 96, 37);
    0.70 + 0.30 * n
}

/// The ground detail texture with its full mip chain (box filtered), so it
/// does not shimmer in the distance.
pub fn ground_detail_image() -> Image {
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let n = DETAIL_SIZE as usize;
    let mut level: Vec<f32> =
        (0..n * n).map(|i| detail_value((i % n) as f32 / n as f32, (i / n) as f32 / n as f32)).collect();
    let mut data = Vec::new();
    let mut side = n;
    let mut levels = 0;
    loop {
        for v in &level {
            let b = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend_from_slice(&[b, b, b, 255]);
        }
        levels += 1;
        if side == 1 {
            break;
        }
        let half = side / 2;
        level = (0..half * half)
            .map(|k| {
                let (x, y) = (2 * (k % half), 2 * (k / half));
                (level[y * side + x]
                    + level[y * side + x + 1]
                    + level[(y + 1) * side + x]
                    + level[(y + 1) * side + x + 1])
                    / 4.0
            })
            .collect();
        side = half;
    }
    let mut image = Image::new_uninit(
        Extent3d { width: DETAIL_SIZE, height: DETAIL_SIZE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

/// The ground detail texture.
#[derive(Resource)]
pub struct GroundDetail(pub Handle<Image>);

impl FromWorld for GroundDetail {
    fn from_world(world: &mut World) -> Self {
        Self(world.resource_mut::<Assets<Image>>().add(ground_detail_image()))
    }
}

/// Per-vertex colours of a [`grid_surface`] from per-sample colours.
pub fn vertex_colors(geometry: &MapGeometry, samples: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let (w, h) = (geometry.width, geometry.height);
    let mut out = Vec::with_capacity((w + 2) * (h + 2));
    for j in 0..h + 2 {
        for i in 0..w + 2 {
            out.push(samples[sample_index(j, h) * w + sample_index(i, w)]);
        }
    }
    out
}

/// Depth (m) below the lowest water surface down to which the ground is
/// drawn: deeper ground lies under opaque water (see [`DEEP_WATER_M`]).
pub const SHOWN_DEPTH_M: f32 = 40.0;

/// Drawn ground height per sample: the model's elevation, cut off
/// [`SHOWN_DEPTH_M`] below the lowest water surface. An oceanic island
/// rises from an abyssal floor 1.5 km down; drawn in full, the map would be
/// a 1.5 km pillar of earth under a sea that hides it anyway.
pub fn shown_elevation(elevation: &[f32], depth: Option<&[f32]>) -> Vec<f32> {
    let Some(depth) = depth.filter(|d| d.len() == elevation.len()) else { return elevation.to_vec() };
    let lowest_surface = elevation.iter().zip(depth).map(|(e, d)| e + d.max(0.0)).fold(f32::INFINITY, f32::min);
    let floor = lowest_surface - SHOWN_DEPTH_M;
    elevation.iter().map(|e| e.max(floor)).collect()
}

/// Vertical earth walls around the map from the top of each edge sample
/// (ground, or water surface where the sea reaches the edge) down to below
/// the lowest drawn ground (`shown`, from [`shown_elevation`]).
pub fn skirt_mesh(geometry: &MapGeometry, shown: &[f32], top: &[f32]) -> Mesh {
    let (w, h) = (geometry.width, geometry.height);
    let xs = axis(w, geometry.spacing);
    let zs = axis(h, geometry.spacing);
    let base = shown.iter().copied().fold(f32::INFINITY, f32::min) - 30.0;
    let ground = |i: usize, j: usize| top[sample_index(j, h) * w + sample_index(i, w)];
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut wall = |points: Vec<(f32, f32, f32)>, normal: Vec3| {
        for pair in points.windows(2) {
            let start = positions.len() as u32;
            let (a, b) = (pair[0], pair[1]);
            positions.extend([[a.0, a.2, a.1], [b.0, b.2, b.1], [a.0, base, a.1], [b.0, base, b.1]]);
            normals.extend([normal.to_array(); 4]);
            // Wind so the face points along `normal`.
            let tri = [start, start + 2, start + 1, start + 1, start + 2, start + 3];
            let p = |k: u32| Vec3::from_array(positions[k as usize]);
            let n = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            if n.dot(normal) >= 0.0 {
                indices.extend_from_slice(&tri);
            } else {
                indices.extend_from_slice(&[tri[0], tri[2], tri[1], tri[3], tri[5], tri[4]]);
            }
        }
    };
    let (nx, nz) = (xs.len(), zs.len());
    wall((0..nx).map(|i| (xs[i], zs[0], ground(i, 0))).collect(), Vec3::NEG_Z);
    wall((0..nx).map(|i| (xs[i], zs[nz - 1], ground(i, nz - 1))).collect(), Vec3::Z);
    wall((0..nz).map(|j| (xs[0], zs[j], ground(0, j))).collect(), Vec3::NEG_X);
    wall((0..nz).map(|j| (xs[nx - 1], zs[j], ground(nx - 1, j))).collect(), Vec3::X);
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_indices(Indices::U32(indices))
}

type BuiltSurface = Or<(With<TerrainSurface>, With<OverlaySurface>, With<Skirt>, With<WaterSurface>)>;

/// Rebuilds the terrain when the frame carries a new static terrain.
#[allow(clippy::too_many_arguments)]
pub fn rebuild_terrain(
    mut commands: Commands,
    data: Res<EcoData>,
    mut map: ResMut<MapState>,
    mut ui: ResMut<EcoUi>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    detail: Res<GroundDetail>,
    built: Query<Entity, BuiltSurface>,
) {
    let Some(frame) = data.get() else { return };
    let world = &frame.view.world;
    let Some(terrain) = &world.terrain else { return };
    if map.epoch == Some(frame.epoch) && map.terrain_version == Some(terrain.version) {
        return;
    }
    let (w, h) = (terrain.elevation.width(), terrain.elevation.height());
    if w < 2 || h < 2 {
        return;
    }
    for e in &built {
        commands.entity(e).despawn();
    }
    let geometry = MapGeometry::new(w, h, world.block, frame.view.cell_size_m, terrain.elevation.as_slice().to_vec());
    let scenario = frame.info.scenario.id.clone();
    let resized = map.geometry.as_ref().is_none_or(|g| g.width != w || g.height != h || g.spacing != geometry.spacing);
    if resized || map.scenario.as_deref() != Some(scenario.as_str()) {
        ui.frame_map = true;
    }
    let depth = layer(frame, WorldLayer::WaterDepth);
    let shown = shown_elevation(&geometry.elevation, depth);
    let top: Vec<f32> = match depth.filter(|d| d.len() == shown.len()) {
        Some(d) => geometry.elevation.iter().zip(d).zip(&shown).map(|((e, d), s)| (e + d.max(0.0)).max(*s)).collect(),
        None => shown.clone(),
    };
    let terrain_mesh = meshes.add(grid_surface(&geometry, |i| shown[i]));
    commands.spawn((
        Name::new("Terrain"),
        TerrainSurface,
        Mesh3d(terrain_mesh.clone()),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(detail.0.clone()),
            perceptual_roughness: 0.94,
            reflectance: 0.18,
            ..default()
        })),
        Transform::IDENTITY,
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Terrain skirt"),
        Skirt,
        Mesh3d(meshes.add(skirt_mesh(&geometry, &shown, &top))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.24, 0.19, 0.14),
            perceptual_roughness: 1.0,
            ..default()
        })),
        NotShadowCaster,
        DespawnOnExit(MODE),
    ));
    let overlay_mesh = meshes.add(grid_surface(&geometry, |i| shown[i] + OVERLAY_LIFT));
    commands.spawn((
        Name::new("Data overlay"),
        OverlaySurface,
        Mesh3d(overlay_mesh.clone()),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            depth_bias: 50.0,
            ..default()
        })),
        Visibility::Hidden,
        NotShadowCaster,
        NotShadowReceiver,
        DespawnOnExit(MODE),
    ));
    let water_material = water_materials.add(WaterMaterial { settings: WaterSettings::default() });
    map.water_material = Some(water_material);
    map.water_depth.clear();
    map.biome = terrain.biome.as_slice().to_vec();
    map.surface = terrain.surface.as_slice().to_vec();
    map.terrain_mesh = Some(terrain_mesh);
    map.overlay_mesh = Some(overlay_mesh);
    map.geometry = Some(geometry);
    map.epoch = Some(frame.epoch);
    map.terrain_version = Some(terrain.version);
    map.scenario = Some(scenario);
    map.refreshed_seq = 0;
    map.overlay = None;
    map.water_look = None;
    map.overlay_shape_stale = true;
}

fn layer(frame: &Frame, layer: WorldLayer) -> Option<&[f32]> {
    frame.view.world.layer(layer).map(Grid2::as_slice)
}

fn vegetation(frame: &Frame, layer: PlantLayer) -> Option<&[f32]> {
    frame.view.vegetation.iter().find(|(l, _)| *l == layer).map(|(_, g)| g.as_slice())
}

/// What the water surface is shaded for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterLook {
    /// Sky colour (sRGB).
    pub sky: [f32; 3],
    /// The model's daylight factor.
    pub daylight: f32,
    /// Unit vector towards the sun (the moon at night).
    pub light_direction: Vec3,
    /// Wind speed (m/s).
    pub wind_m_s: f32,
}

impl WaterLook {
    /// Whether the change is worth re-shading the water.
    pub fn differs(&self, other: &WaterLook) -> bool {
        self.sky != other.sky
            || (self.daylight - other.daylight).abs() > 0.03
            || self.light_direction.angle_between(other.light_direction) > 0.02
            || (self.wind_m_s - other.wind_m_s).abs() > 0.5
    }
}

/// Water shading: the sky it reflects, the sun's glint from the light's
/// direction (dimmed at night), and waves whose slope and length grow with
/// the wind (a calm lake is glassy, a gale makes it choppy).
pub fn water_settings(look: &WaterLook) -> WaterSettings {
    let mut settings = WaterSettings {
        sky_color: Color::srgb(look.sky[0], look.sky[1], look.sky[2]).to_linear().to_vec4(),
        sun_direction: look.light_direction.normalize_or(Vec3::Y).extend(0.0),
        ..WaterSettings::default()
    };
    let night = 1.0 - look.daylight.min(0.4) / 0.4;
    settings.sun_color.w = (0.2 + 1.4 * look.daylight) * (1.0 - night * 0.8);
    settings.shallow_color *= 1.0 - 0.6 * night;
    settings.deep_color *= 1.0 - 0.6 * night;
    let wind = look.wind_m_s.clamp(0.0, 30.0);
    settings.waves.x = 0.015 + 0.006 * wind.min(15.0);
    settings.waves.y = 4.0 + 0.6 * wind;
    // Water is fully opaque from `DEEP_WATER_M`: the ground under deep
    // water (an oceanic island's flanks go down to the abyssal floor, 1.5 km)
    // must not shine through the sea.
    settings.waves.w = DEEP_WATER_M;
    settings.shading.y = 1.0;
    settings
}

/// Depth (m) from which water shows its deep colour and hides the ground.
pub const DEEP_WATER_M: f32 = 12.0;

/// Per-sample ground colours (linear RGBA) of a frame.
pub fn ground_colors(frame: &Frame, map: &MapState, grass: Color) -> Vec<[f32; 4]> {
    let n = map.biome.len();
    let at = |s: Option<&[f32]>, i: usize| s.and_then(|v| v.get(i)).copied().unwrap_or(0.0);
    let (grass_l, flowers, moss, shrubs, desert) = (
        vegetation(frame, PlantLayer::Grass),
        vegetation(frame, PlantLayer::Flowers),
        vegetation(frame, PlantLayer::Moss),
        vegetation(frame, PlantLayer::Shrubs),
        vegetation(frame, PlantLayer::Desert),
    );
    let (snow, moisture) = (layer(frame, WorldLayer::Snow), layer(frame, WorldLayer::SoilMoisture));
    let fire = frame.view.world.fire.as_ref().map(Grid2::as_slice);
    let grass = grass.to_linear();
    let tick = frame.view.tick;
    let tpd = frame.info.ticks_per_day;
    (0..n)
        .map(|i| {
            let sample = GroundSample {
                biome: map.biome[i],
                herbs: at(grass_l, i) + at(flowers, i),
                moss: at(moss, i),
                flowers: at(flowers, i),
                shrubs: at(shrubs, i),
                desert: at(desert, i),
                snow_m: at(snow, i),
                moisture: at(moisture, i),
                fire: fire.and_then(|f| f.get(i)).copied().unwrap_or(FireState::Fuel),
            };
            // Patchiness between cells (presentation): ±7 % brightness, and the
            // detail texture's mean brightness compensated.
            let h = (i as u32).wrapping_mul(2_654_435_761) >> 16;
            let patch = 0.93 + 0.14 * (h & 0xff) as f32 / 255.0;
            let c = ground_color(&sample, grass, tick, tpd) * (patch / DETAIL_MEAN);
            [c.red, c.green, c.blue, 1.0]
        })
        .collect()
}

/// Values of a grid overlay per sample (`None`: no data there).
pub fn overlay_values(frame: &Frame, map: &MapState, overlay: Overlay, fire_spread: f32) -> Vec<Option<f32>> {
    let n = map.biome.len();
    let from = |s: Option<&[f32]>| -> Vec<Option<f32>> {
        match s {
            Some(v) if v.len() == n => v.iter().map(|x| Some(*x)).collect(),
            _ => vec![None; n],
        }
    };
    match overlay {
        Overlay::None | Overlay::Territories => vec![None; n],
        Overlay::Temperature => from(layer(frame, WorldLayer::Temperature)),
        Overlay::Moisture => {
            let mut v = from(layer(frame, WorldLayer::SoilMoisture));
            for (value, s) in v.iter_mut().zip(&map.surface) {
                if *s != SurfaceKind::Land {
                    *value = None;
                }
            }
            v
        }
        Overlay::SurfaceWater => {
            from(layer(frame, WorldLayer::WaterDepth)).into_iter().map(|v| v.filter(|d| *d > MIN_WATER_DEPTH)).collect()
        }
        Overlay::Nutrients => from(layer(frame, WorldLayer::Nutrients)),
        Overlay::PlantBiomass => {
            let mut total = vec![0.0f32; n];
            for (_, grid) in &frame.view.vegetation {
                if grid.as_slice().len() == n {
                    for (t, v) in total.iter_mut().zip(grid.as_slice()) {
                        *t += v;
                    }
                }
            }
            total.into_iter().map(Some).collect()
        }
        Overlay::FireRisk => fire_risk(frame, map, fire_spread),
        Overlay::Disease => disease_prevalence(frame, map),
        Overlay::Pheromone => from(frame.view.pheromone.as_ref().map(Grid2::as_slice)),
        Overlay::Pollination => from(frame.view.pollination.as_ref().map(Grid2::as_slice)),
    }
}

/// The physics world's spread probability into each sample from a burning
/// upwind neighbour of intensity 1 (see [`Overlay::FireRisk`]).
pub fn fire_risk(frame: &Frame, map: &MapState, fire_spread: f32) -> Vec<Option<f32>> {
    let n = map.biome.len();
    let (Some(fuel), Some(humidity), Some(moisture), Some(water), Some(geometry)) = (
        layer(frame, WorldLayer::Fuel),
        layer(frame, WorldLayer::Humidity),
        layer(frame, WorldLayer::SoilMoisture),
        layer(frame, WorldLayer::WaterDepth),
        map.geometry.as_ref(),
    ) else {
        return vec![None; n];
    };
    let wind = frame.view.world.weather.wind;
    let direction = if wind.length() > 1e-3 { wind.normalize() } else { simulation_core::glam::Vec2::X };
    let fire = frame.view.world.fire.as_ref().map(Grid2::as_slice);
    (0..n)
        .map(|i| {
            let state = fire.and_then(|f| f.get(i)).copied().unwrap_or(FireState::Fuel);
            if state.is_active() {
                return Some(1.0);
            }
            if matches!(state, FireState::Unburnable | FireState::Burned { .. }) || fuel[i] < MIN_FUEL_KG_M2 {
                return Some(0.0);
            }
            Some(spread_probability(&SpreadInputs {
                wind,
                direction,
                rise_m: 0.0,
                distance_m: geometry.spacing,
                target_fuel: fuel[i],
                target_biome: map.biome[i],
                target_moisture: fuel_moisture(moisture[i], humidity[i], water[i]),
                source_intensity: 1.0,
                spread_factor: fire_spread,
            }))
        })
        .collect()
}

/// Side (m) of the blocks the disease prevalence is counted in: a cell
/// holds too few hosts for a share to mean anything.
pub const DISEASE_BLOCK_M: f32 = 100.0;

/// Share of infectious animals among the hosts of the run's pathogens,
/// per block of about [`DISEASE_BLOCK_M`] (`None` where a block has no
/// host).
pub fn disease_prevalence(frame: &Frame, map: &MapState) -> Vec<Option<f32>> {
    let n = map.biome.len();
    let Some(geometry) = map.geometry.as_ref() else { return vec![None; n] };
    let catalog = &frame.info.catalog;
    let hosts: Vec<u16> =
        frame.info.pathogens.iter().flat_map(|p| p.hosts.iter()).filter_map(|h| catalog.animal_index(h).ok()).collect();
    let positions = frame.view.animals.iter().filter(|a| hosts.contains(&a.species)).map(|a| (a.position, a.infected));
    block_shares(geometry, positions, DISEASE_BLOCK_M)
}

/// Share of `true` among the flagged positions (cells) per block of about
/// `block_m` metres, spread over the samples of each block.
pub fn block_shares(
    geometry: &MapGeometry,
    positions: impl Iterator<Item = (Vec2, bool)>,
    block_m: f32,
) -> Vec<Option<f32>> {
    let (w, h) = (geometry.width, geometry.height);
    let k = ((block_m / geometry.spacing).round() as usize).max(1);
    let (bw, bh) = (w.div_ceil(k), h.div_ceil(k));
    let mut counts = vec![(0u32, 0u32); bw * bh];
    for (p, flag) in positions {
        if let Some(i) = geometry.sample_of_cell(p) {
            let b = (i / w / k) * bw + (i % w) / k;
            counts[b].0 += 1;
            counts[b].1 += u32::from(flag);
        }
    }
    (0..w * h)
        .map(|i| {
            let (total, flagged) = counts[(i / w / k) * bw + (i % w) / k];
            (total > 0).then(|| flagged as f32 / total as f32)
        })
        .collect()
}

/// Refreshes ground colours, water and the overlay from new frames.
#[allow(clippy::too_many_arguments)]
pub fn refresh_landscape(
    mut commands: Commands,
    data: Res<EcoData>,
    ui: Res<EcoUi>,
    status: Res<SessionStatus>,
    time: Res<Time<Real>>,
    season: Res<PlantSeason>,
    scene: Res<super::SceneState>,
    mut map: ResMut<MapState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    water: Query<Entity, With<WaterSurface>>,
    mut overlay_vis: Query<&mut Visibility, With<OverlaySurface>>,
) {
    let Some(frame) = data.get() else { return };
    if map.geometry.is_none() || map.epoch != Some(frame.epoch) {
        return;
    }
    let now = time.elapsed_secs_f64();
    let overlay_changed = map.overlay != Some(ui.overlay);
    let due = data.seq != map.refreshed_seq && now - map.last_refresh >= REFRESH_SECONDS;
    if !(due || overlay_changed) {
        return;
    }
    map.last_refresh = now;
    map.refreshed_seq = data.seq;
    // Ground colours.
    let colors = ground_colors(frame, &map, season.grass());
    if let (Some(handle), Some(g)) = (map.terrain_mesh.clone(), map.geometry.as_ref())
        && let Some(mut mesh) = meshes.get_mut(&handle)
    {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors(g, &colors));
    }
    // Water: rebuilt when the depth changed by more than 2 cm somewhere.
    if let Some(depth) = layer(frame, WorldLayer::WaterDepth).map(<[f32]>::to_vec) {
        let changed = depth.len() != map.water_depth.len()
            || depth.iter().zip(&map.water_depth).any(|(a, b)| (a - b).abs() > 0.02);
        if changed {
            if let Some(g) = map.geometry.as_mut() {
                g.set_water(&depth);
            }
            for e in &water {
                commands.entity(e).despawn();
            }
            if let Some(g) = map.geometry.as_ref() {
                let settings = TerrainSettings { cell_size: g.spacing, height_scale: 1.0, lod_step: 1 };
                if let Ok(data) = water_mesh(&g.elevation, &depth, g.width, g.height, MIN_WATER_DEPTH, &settings)
                    && data.triangle_count() > 0
                    && let Some(material) = map.water_material.clone()
                {
                    commands.spawn((
                        Name::new("Water"),
                        WaterSurface,
                        Mesh3d(meshes.add(data.to_mesh())),
                        MeshMaterial3d(material),
                        NotShadowCaster,
                        DespawnOnExit(MODE),
                    ));
                }
            }
            map.water_depth = depth;
            map.overlay_shape_stale = true;
        }
    }
    // Water shading follows the sky, the sun (or moon) and the wind.
    let look = WaterLook {
        sky: scene.sky,
        daylight: scene.daylight,
        light_direction: scene.light_direction,
        wind_m_s: frame.view.world.weather.wind.length(),
    };
    if map.water_look.is_none_or(|old| old.differs(&look))
        && let Some(handle) = map.water_material.clone()
        && let Some(mut material) = water_materials.get_mut(&handle)
    {
        material.settings = water_settings(&look);
        map.water_look = Some(look);
    }
    // Overlay.
    map.overlay = Some(ui.overlay);
    let visible = ui.overlay.is_grid();
    for mut v in &mut overlay_vis {
        v.set_if_neq(if visible { Visibility::Inherited } else { Visibility::Hidden });
    }
    if !visible {
        map.overlay_values.clear();
        return;
    }
    let fire_spread = status
        .get()
        .and_then(|s| s.params.get("fire_spread"))
        .and_then(simulation_core::ParamValue::as_f64)
        .unwrap_or(1.0) as f32;
    let values = overlay_values(frame, &map, ui.overlay, fire_spread);
    let range =
        values.iter().flatten().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let scale = overlay_scale(ui.overlay, range);
    let samples: Vec<[f32; 4]> = values
        .iter()
        .map(|v| match v {
            Some(v) => linear(scale.color(*v), scale.alpha(*v, OVERLAY_ALPHA)),
            None => [0.0; 4],
        })
        .collect();
    map.overlay_range = range;
    let reshape = std::mem::take(&mut map.overlay_shape_stale);
    if let (Some(handle), Some(g)) = (map.overlay_mesh.clone(), map.geometry.as_ref())
        && let Some(mut m) = meshes.get_mut(&handle)
    {
        let colors = vertex_colors(g, &samples);
        // The surface follows the water: re-shaped only when it moved.
        if reshape {
            *m = grid_surface(g, |i| g.water_level[i].max(g.elevation[i]) + OVERLAY_LIFT)
                .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        } else {
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
    }
    map.overlay_values = values;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> MapGeometry {
        MapGeometry::new(3, 2, 1, 10.0, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0])
    }

    #[test]
    fn axes_reach_the_map_edges() {
        assert_eq!(axis(3, 10.0), vec![0.0, 5.0, 15.0, 25.0, 30.0]);
        assert_eq!((sample_index(0, 3), sample_index(1, 3), sample_index(3, 3), sample_index(4, 3)), (0, 0, 2, 2));
    }

    #[test]
    fn grid_surface_spans_the_map_with_upward_faces() {
        let g = geometry();
        let mesh = grid_surface(&g, |i| g.elevation[i]);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        assert_eq!(positions.len(), 5 * 4);
        assert_eq!(positions[0], [0.0, 0.0, 0.0]);
        assert_eq!(*positions.last().unwrap(), [30.0, 5.0, 20.0]);
        let Some(Indices::U32(indices)) = mesh.indices() else { panic!("indices") };
        for tri in indices.chunks(3) {
            let p = |k: u32| Vec3::from_array(positions[k as usize]);
            let n = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            assert!(n.y > 0.0, "faces point up");
        }
        let colors = vertex_colors(&g, &[[0.1, 0.0, 0.0, 1.0]; 6]);
        assert_eq!(colors.len(), positions.len());
    }

    #[test]
    fn detail_texture_tiles_and_has_mips() {
        // Seamless: the right edge continues the left one.
        for v in [0.1f32, 0.5, 0.9] {
            assert!((detail_value(0.0, v) - detail_value(1.0, v)).abs() < 1e-4);
            assert!((detail_value(v, 0.0) - detail_value(v, 1.0)).abs() < 1e-4);
        }
        let values: Vec<f32> =
            (0..4096).map(|i| detail_value((i % 64) as f32 / 64.0, (i / 64) as f32 / 64.0)).collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - DETAIL_MEAN).abs() < 0.03, "{mean}");
        assert!(values.iter().all(|v| (0.69..=1.0).contains(v)));
        let image = ground_detail_image();
        assert_eq!(image.texture_descriptor.mip_level_count, 9);
        let texels: usize = (0..9).map(|l| (256usize >> l).pow(2)).sum();
        assert_eq!(image.data.as_ref().map(Vec::len), Some(texels * 4));
    }

    #[test]
    fn water_follows_wind_sun_and_night() {
        let calm = WaterLook { sky: [0.5, 0.6, 0.9], daylight: 0.9, light_direction: Vec3::Y, wind_m_s: 0.0 };
        let gale = WaterLook { wind_m_s: 20.0, ..calm };
        let (c, g) = (water_settings(&calm), water_settings(&gale));
        assert!(g.waves.x > 3.0 * c.waves.x && g.waves.y > c.waves.y, "wind raises and lengthens the waves");
        let low_sun = WaterLook { light_direction: Vec3::new(1.0, 0.2, 0.0).normalize(), ..calm };
        assert!((water_settings(&low_sun).sun_direction.truncate() - low_sun.light_direction).length() < 1e-5);
        let night = water_settings(&WaterLook { daylight: 0.0, ..calm });
        assert!(night.sun_color.w < c.sun_color.w && night.deep_color.x < c.deep_color.x);
        assert!(!calm.differs(&WaterLook { wind_m_s: 0.2, ..calm }) && calm.differs(&gale));
    }

    #[test]
    fn shares_are_counted_per_block() {
        // 10 × 10 samples of 20 m: blocks of 100 m are 5 × 5 samples.
        let g = MapGeometry::new(10, 10, 1, 20.0, vec![0.0; 100]);
        let p = Vec2::new;
        let shares =
            block_shares(&g, [(p(0.5, 0.5), true), (p(4.5, 4.5), false), (p(9.5, 0.5), false)].into_iter(), 100.0);
        assert_eq!(shares[0], Some(0.5), "two hosts in the first block, one sick");
        assert_eq!(shares[4 * 10 + 4], Some(0.5), "the whole block carries the share");
        assert_eq!(shares[9], Some(0.0), "healthy hosts: zero, not empty");
        assert_eq!(shares[99], None, "no host");
    }

    #[test]
    fn ground_under_deep_water_is_cut_off() {
        // An oceanic island: abyssal floor, a shore and a hill.
        let elevation = [-1500.0, -2.0, 3.0, 40.0];
        let depth = [1500.5, 2.5, 0.0, 0.0];
        let shown = shown_elevation(&elevation, Some(&depth));
        assert_eq!(shown[0], 0.5 - SHOWN_DEPTH_M, "cut off below the lowest surface");
        assert_eq!(&shown[1..], &elevation[1..], "shallows and land unchanged");
        // Without water (or a mismatched grid) nothing is cut.
        assert_eq!(shown_elevation(&elevation, None), elevation.to_vec());
        assert_eq!(shown_elevation(&elevation, Some(&[1.0])), elevation.to_vec());
    }

    #[test]
    fn skirt_faces_outwards() {
        let g = geometry();
        let mesh = skirt_mesh(&g, &g.elevation, &g.elevation);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        let Some(Indices::U32(indices)) = mesh.indices() else { panic!("indices") };
        let centre = Vec3::new(15.0, 0.0, 10.0);
        for tri in indices.chunks(3) {
            let p = |k: u32| Vec3::from_array(positions[k as usize]);
            let n = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            let mid = (p(tri[0]) + p(tri[1]) + p(tri[2])) / 3.0;
            let out = (mid - centre) * Vec3::new(1.0, 0.0, 1.0);
            assert!(n.dot(out) > 0.0, "{tri:?}");
        }
        assert!(positions.iter().all(|p| p[1] <= 5.0 && p[1] >= -30.0));
    }
}
