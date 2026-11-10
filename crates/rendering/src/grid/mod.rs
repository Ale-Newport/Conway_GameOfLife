//! Styled 2D grid renderer (Life, CA lab, metapixel, continuous fields).
//!
//! A mode spawns a [`GridSurface`] (plus an optional [`GridAppearance`]) and
//! submits a [`GridFrame`] whenever its view changes. The frame is an RGBA8
//! data texture covering the visible region (see [`frame`] for the channel
//! layout); [`GridMaterial`] colours it on the GPU according to the
//! appearance. Re-styling never touches the frame, and uploading a frame of
//! the same size reuses the existing `Image` and GPU texture.
//!
//! ```no_run
//! use bevy::math::I64Vec2;
//! use bevy::prelude::*;
//! use rendering::{GridAppearance, GridFrame, GridStyle, GridSurface};
//!
//! fn spawn_surface(mut commands: Commands) {
//!     commands.spawn((GridSurface::default(), GridAppearance::styled(GridStyle::Neon)));
//! }
//!
//! // In a mode system, after the simulation published a new view:
//! fn upload(mut surfaces: Query<&mut GridSurface>) {
//!     for mut surface in &mut surfaces {
//!         let Ok(frame) = GridFrame::from_states(I64Vec2::ZERO, 2, 2, &[0, 1, 1, 0]) else { continue };
//!         if let Err(err) = surface.submit(frame) {
//!             error!("grid frame rejected: {err}");
//!         }
//!     }
//! }
//! ```

pub mod frame;
pub mod space;
pub mod style;

use bevy::asset::{RenderAssetUsages, io::embedded::EmbeddedAssetRegistry};
use bevy::image::ImageSampler;
use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{Material2d, Material2dPlugin};
use std::path::{Path, PathBuf};

pub use frame::{FrameGeometry, GridFrame, GridFrameError, MAX_FRAME_DIMENSION, Texel, encode_range, encode_unit};
pub use space::{GridSpace, VisibleCells};
pub use style::{AGE_RAMP_MIN, ColorTable, GridAppearance, GridParams, GridStyle, age_ramp_position};

use style::GridUniforms;

/// Asset path of the embedded grid shader (source: `assets/shaders/grid.wesl`).
pub const GRID_SHADER_PATH: &str = "embedded://rendering/shaders/grid.wesl";

/// Format of grid data textures. Unorm (not sRGB): the bytes are data, not colours.
const DATA_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// Registers the grid material, the [`GridSpace`] resource and the systems
/// that upload frames and place surfaces.
pub struct GridRenderPlugin;

/// System sets of the grid renderer (all in `PostUpdate`, before transform
/// propagation). Submit frames in `Update` so they are uploaded the same frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GridRenderSystems {
    /// Uploads pending frames and updates material uniforms.
    Upload,
    /// Positions surfaces from their frame geometry and the [`GridSpace`].
    Place,
}

impl Plugin for GridRenderPlugin {
    fn build(&self, app: &mut App) {
        if let Some(registry) = app.world().get_resource::<EmbeddedAssetRegistry>() {
            registry.insert_asset(
                PathBuf::new(),
                Path::new("rendering/shaders/grid.wesl"),
                include_bytes!("../../../../assets/shaders/grid.wesl").as_slice(),
            );
        } else {
            error!(
                "GridRenderPlugin needs AssetPlugin (DefaultPlugins) to be added first; the grid shader is unavailable"
            );
        }
        app.add_plugins(Material2dPlugin::<GridMaterial>::default())
            .init_resource::<GridSpace>()
            .add_observer(init_grid_surface)
            .configure_sets(
                PostUpdate,
                (GridRenderSystems::Upload, GridRenderSystems::Place).chain().before(TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                (
                    (upload_grid_frames, sync_grid_appearance).chain().in_set(GridRenderSystems::Upload),
                    place_grid_surfaces.in_set(GridRenderSystems::Place),
                ),
            );
    }
}

/// Material colouring a grid data texture (see `assets/shaders/grid.wesl`).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct GridMaterial {
    /// Scalar parameters (style, flags, colours).
    #[uniform(0)]
    pub params: GridParams,
    /// State colours or colormap stops.
    #[uniform(1)]
    pub palette: ColorTable,
    /// Ramp colormap stops for scalar channels.
    #[uniform(2)]
    pub ramp: ColorTable,
    /// RGBA8 data texture (R state, G age, B activity, A density).
    #[texture(3)]
    pub cells: Handle<Image>,
}

impl Material2d for GridMaterial {
    fn fragment_shader() -> ShaderRef {
        GRID_SHADER_PATH.into()
    }
}

/// A rectangle of cells rendered from uploaded [`GridFrame`]s. The renderer
/// owns the entity's `Transform` translation x/y and scale (derived from the
/// frame origin, size and [`GridSpace`]); `translation.z` is left to the user
/// for layering.
#[derive(Component, Debug, Default)]
#[require(GridAppearance, Transform, Visibility)]
pub struct GridSurface {
    pending: Option<GridFrame>,
    geometry: Option<FrameGeometry>,
}

impl GridSurface {
    /// Queues a frame for upload (validated now, uploaded in `PostUpdate`).
    /// A frame submitted before the previous one was uploaded replaces it.
    pub fn submit(&mut self, frame: GridFrame) -> Result<(), GridFrameError> {
        frame.validate()?;
        self.pending = Some(frame);
        Ok(())
    }

    /// Geometry of the last uploaded frame.
    pub fn geometry(&self) -> Option<FrameGeometry> {
        self.geometry
    }

    /// Whether a submitted frame is waiting for upload.
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
}

/// GPU handles of a surface, created when the surface is added.
#[derive(Component, Debug, Clone)]
pub struct GridSurfaceGpu {
    /// The data texture.
    pub image: Handle<Image>,
    /// The surface's material.
    pub material: Handle<GridMaterial>,
}

/// Shared unit quad for all grid surfaces.
#[derive(Resource, Clone)]
struct GridQuad(Handle<Mesh>);

fn data_image(width: u32, height: u32, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        DATA_FORMAT,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

fn init_grid_surface(
    added: On<Add<GridSurface>>,
    mut commands: Commands,
    appearances: Query<&GridAppearance>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<GridMaterial>>,
) {
    let appearance = appearances.get(added.entity).cloned().unwrap_or_default();
    let image = images.add(data_image(1, 1, vec![0; 4]));
    let uniforms = GridUniforms::new(&appearance, 1, I64Vec2::ZERO);
    let material = materials.add(GridMaterial {
        params: uniforms.params,
        palette: uniforms.palette,
        ramp: uniforms.ramp,
        cells: image.clone(),
    });
    commands
        .entity(added.entity)
        .insert((GridSurfaceGpu { image, material: material.clone() }, MeshMaterial2d(material)));
}

fn upload_grid_frames(
    mut commands: Commands,
    mut surfaces: Query<(Entity, &mut GridSurface, &GridSurfaceGpu, &GridAppearance)>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<GridMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut quad: Local<Option<GridQuad>>,
) {
    for (entity, mut surface, gpu, appearance) in &mut surfaces {
        let Some(frame) = surface.pending.take() else { continue };
        let geometry = frame.geometry();
        let Some(mut image) = images.get_mut(&gpu.image) else { continue };
        let reallocated = if image.width() == frame.width
            && image.height() == frame.height
            && image.texture_descriptor.format == DATA_FORMAT
        {
            // Same size: move the bytes in; the GPU texture is reused.
            image.data = Some(frame.data);
            false
        } else {
            *image = data_image(frame.width, frame.height, frame.data);
            true
        };
        let first_upload = surface.geometry.is_none();
        surface.geometry = Some(geometry);
        let uniforms = GridUniforms::new(appearance, geometry.cells_per_texel, geometry.origin);
        // A reallocated texture is a new GPU resource: the material's bind
        // group still points at the old one until the material is marked
        // modified, so it is touched even when its uniforms are unchanged.
        if (reallocated || materials.get(&gpu.material).is_some_and(|m| m.params != uniforms.params))
            && let Some(mut material) = materials.get_mut(&gpu.material)
        {
            material.params = uniforms.params;
        }
        if first_upload {
            let quad = quad.get_or_insert_with(|| GridQuad(meshes.add(Rectangle::new(1.0, 1.0)))).0.clone();
            commands.entity(entity).insert(Mesh2d(quad));
        }
    }
}

fn sync_grid_appearance(
    surfaces: Query<(&GridSurface, &GridSurfaceGpu, &GridAppearance), Changed<GridAppearance>>,
    mut materials: ResMut<Assets<GridMaterial>>,
) {
    for (surface, gpu, appearance) in &surfaces {
        let (cpt, origin) = surface.geometry.map_or((1, I64Vec2::ZERO), |g| (g.cells_per_texel, g.origin));
        let uniforms = GridUniforms::new(appearance, cpt, origin);
        let Some(current) = materials.get(&gpu.material) else { continue };
        if current.params == uniforms.params && current.palette == uniforms.palette && current.ramp == uniforms.ramp {
            continue;
        }
        if let Some(mut material) = materials.get_mut(&gpu.material) {
            material.params = uniforms.params;
            material.palette = uniforms.palette;
            material.ramp = uniforms.ramp;
        }
    }
}

/// Transform (centre and size in world units) of a frame in `space`.
pub fn frame_world_rect(space: &GridSpace, geometry: &FrameGeometry) -> (Vec2, Vec2) {
    let corner = space.cell_corner_to_world(geometry.origin);
    let k = geometry.cells_per_texel as f32 * space.cell_size;
    let size = Vec2::new(geometry.width as f32 * k, geometry.height as f32 * k);
    (Vec2::new(corner.x + size.x * 0.5, corner.y - size.y * 0.5), size)
}

fn place_grid_surfaces(space: Res<GridSpace>, mut surfaces: Query<(&GridSurface, &mut Transform)>) {
    for (surface, mut transform) in &mut surfaces {
        let Some(geometry) = surface.geometry else { continue };
        let (center, size) = frame_world_rect(&space, &geometry);
        let target = Transform {
            translation: center.extend(transform.translation.z),
            rotation: Quat::IDENTITY,
            scale: size.extend(1.0),
        };
        transform.set_if_neq(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rect_matches_cell_mapping() {
        let space = GridSpace { cell_size: 2.0, anchor: I64Vec2::new(10, 10) };
        let geometry = FrameGeometry { origin: I64Vec2::new(10, 10), width: 4, height: 2, cells_per_texel: 1 };
        let (center, size) = frame_world_rect(&space, &geometry);
        assert_eq!(size, Vec2::new(8.0, 4.0));
        assert_eq!(center, Vec2::new(4.0, -2.0));
        // Top-left corner of the quad is the top-left corner of the origin cell.
        assert_eq!(center + Vec2::new(-size.x, size.y) * 0.5, space.cell_corner_to_world(geometry.origin));
        let lod = FrameGeometry { cells_per_texel: 8, ..geometry };
        assert_eq!(frame_world_rect(&space, &lod).1, Vec2::new(64.0, 32.0));
    }

    /// Headless app with the grid systems (no renderer).
    fn grid_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<Mesh>()
            .init_asset::<GridMaterial>()
            .insert_resource(GridSpace::new(2.0))
            .add_observer(init_grid_surface)
            .add_systems(Update, (upload_grid_frames, sync_grid_appearance, place_grid_surfaces).chain());
        app
    }

    #[test]
    fn upload_reuses_image_and_places_surface() {
        let mut app = grid_app();
        let entity = app
            .world_mut()
            .spawn((
                GridSurface::default(),
                GridAppearance::styled(GridStyle::Neon),
                Transform::from_xyz(0.0, 0.0, 3.0),
            ))
            .id();
        app.update();
        let gpu = app.world().get::<GridSurfaceGpu>(entity).expect("gpu handles created on add").clone();
        assert!(app.world().get::<Mesh2d>(entity).is_none(), "no quad before the first frame");

        let frame = GridFrame::from_states(I64Vec2::new(10, -4), 3, 2, &[1, 0, 2, 0, 3, 0]).expect("valid");
        app.world_mut().get_mut::<GridSurface>(entity).expect("surface").submit(frame).expect("valid frame");
        app.update();
        {
            let images = app.world().resource::<Assets<Image>>();
            let image = images.get(&gpu.image).expect("image");
            assert_eq!(image.size(), UVec2::new(3, 2));
            assert_eq!(image.texture_descriptor.format, DATA_FORMAT);
            assert_eq!(image.data.as_deref().map(|d| &d[8..12]), Some(&[2, 0, 0, 0][..]));
        }
        assert!(app.world().get::<Mesh2d>(entity).is_some());
        let transform = *app.world().get::<Transform>(entity).expect("transform");
        // Cells x 10..13, y -4..-2 with cell size 2: world x 20..26, y 8..4.
        assert_eq!(transform.translation, Vec3::new(23.0, 6.0, 3.0));
        assert_eq!(transform.scale, Vec3::new(6.0, 4.0, 1.0));
        let material = app.world().resource::<Assets<GridMaterial>>().get(&gpu.material).expect("material").clone();
        assert_eq!(material.params.style, GridStyle::Neon.shader_id());
        assert_eq!(material.cells, gpu.image);

        // Same size: same image asset, new bytes.
        let frame = GridFrame::from_states(I64Vec2::new(10, -4), 3, 2, &[9; 6]).expect("valid");
        app.world_mut().get_mut::<GridSurface>(entity).expect("surface").submit(frame).expect("valid frame");
        app.update();
        let images = app.world().resource::<Assets<Image>>();
        assert_eq!(images.len(), 1, "no new image allocated");
        assert_eq!(images.get(&gpu.image).and_then(|i| i.data.as_deref()).map(|d| d[0]), Some(9));

        // New size and LOD: image resized in place, uniforms follow the geometry.
        let frame = GridFrame::new(I64Vec2::new(-8, 0), 5, 4, 4).expect("valid");
        app.world_mut().get_mut::<GridSurface>(entity).expect("surface").submit(frame).expect("valid frame");
        app.update();
        let images = app.world().resource::<Assets<Image>>();
        assert_eq!(images.get(&gpu.image).map(Image::size), Some(UVec2::new(5, 4)));
        let material = app.world().resource::<Assets<GridMaterial>>().get(&gpu.material).expect("material").clone();
        assert_eq!(material.params.cells_per_texel, 4);
        let transform = *app.world().get::<Transform>(entity).expect("transform");
        assert_eq!(transform.scale, Vec3::new(40.0, 32.0, 1.0));

        // Restyling only touches uniforms.
        app.world_mut().get_mut::<GridAppearance>(entity).expect("appearance").style = GridStyle::Circles;
        app.update();
        let material = app.world().resource::<Assets<GridMaterial>>().get(&gpu.material).expect("material").clone();
        assert_eq!(material.params.style, GridStyle::Circles.shader_id());
        assert_eq!(material.params.cells_per_texel, 4);
    }

    #[test]
    fn surfaces_follow_reanchoring() {
        let mut app = grid_app();
        let entity = app.world_mut().spawn(GridSurface::default()).id();
        let frame = GridFrame::new(I64Vec2::new(5000, 0), 2, 2, 1).expect("valid");
        app.world_mut().get_mut::<GridSurface>(entity).expect("surface").submit(frame).expect("valid frame");
        app.update();
        let before = app.world().get::<Transform>(entity).expect("transform").translation;
        app.world_mut().resource_mut::<GridSpace>().anchor = I64Vec2::new(5000, 0);
        app.update();
        let after = app.world().get::<Transform>(entity).expect("transform").translation;
        assert_eq!(before.x - after.x, 10_000.0);
        assert_eq!(after.truncate(), Vec2::new(2.0, -2.0));
    }

    #[test]
    fn surface_rejects_invalid_frames() {
        let mut surface = GridSurface::default();
        let mut frame = GridFrame::new(I64Vec2::ZERO, 2, 2, 1).expect("valid");
        frame.data.truncate(3);
        assert!(surface.submit(frame).is_err());
        assert!(!surface.has_pending());
        let frame = GridFrame::new(I64Vec2::ZERO, 2, 2, 1).expect("valid");
        assert!(surface.submit(frame).is_ok());
        assert!(surface.has_pending());
    }
}
