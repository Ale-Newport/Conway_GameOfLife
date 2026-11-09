//! Voxel volumes: face-culled meshes with per-vertex state colours, an
//! optional translucent material, chunked incremental rebuilds and slicing.
//!
//! ```no_run
//! use bevy::prelude::*;
//! use rendering::voxel::{SliceAxis, SliceKeep, Voxel, VoxelPalette, VoxelSlice, VoxelVolume};
//!
//! fn spawn_volume(mut commands: Commands) -> Result {
//!     let palette = VoxelPalette::new(vec![Color::NONE, Color::WHITE])?;
//!     let mut volume = VoxelVolume::new(16, 1.0, palette)?;
//!     volume.set_voxels(vec![Voxel::new(0, 0, 0, 1), Voxel::new(1, 0, 0, 1)])?; // e.g. a voxel-life view
//!     volume.set_slice(Some(VoxelSlice { axis: SliceAxis::Y, position: 10, keep: SliceKeep::Below }));
//!     commands.spawn(volume);
//!     Ok(())
//! }
//! ```

pub mod chunks;
pub mod mesher;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use std::collections::BTreeMap;

pub use chunks::{ChunkDiff, ChunkUpdate, ChunkedVoxelMesher};
pub use mesher::{
    MAX_VOXEL_COORD, SliceAxis, SliceKeep, Voxel, VoxelError, VoxelMeshData, VoxelMeshes, VoxelPalette, VoxelSlice,
    check_voxels, face_visible, mesh_voxels,
};

/// Creates the shared voxel materials and rebuilds dirty volumes in `PostUpdate`.
pub struct VoxelPlugin;

impl Plugin for VoxelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, create_voxel_materials)
            .add_systems(PostUpdate, rebuild_voxel_volumes.before(TransformSystems::Propagate));
    }
}

/// Materials shared by all voxel chunks. Both multiply the per-vertex colour;
/// edit the assets to change roughness or emissive.
#[derive(Resource, Debug, Clone)]
pub struct VoxelMaterials {
    /// Opaque faces.
    pub opaque: Handle<StandardMaterial>,
    /// Translucent faces (alpha blended, no shadows).
    pub translucent: Handle<StandardMaterial>,
}

fn create_voxel_materials(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let opaque = materials.add(StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.85, ..default() });
    let translucent = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.2,
        reflectance: 0.6,
        ..default()
    });
    commands.insert_resource(VoxelMaterials { opaque, translucent });
}

#[derive(Debug, Default, Clone)]
struct ChunkParts {
    opaque: Option<(Entity, Handle<Mesh>)>,
    translucent: Option<(Entity, Handle<Mesh>)>,
    faces: usize,
}

/// A voxel volume rendered as chunk child entities. Positions are in voxels
/// relative to the volume entity's transform.
#[derive(Component, Debug)]
#[require(Transform, Visibility)]
pub struct VoxelVolume {
    palette: VoxelPalette,
    voxel_size: f32,
    slice: Option<VoxelSlice>,
    voxels: Vec<Voxel>,
    mesher: ChunkedVoxelMesher,
    dirty: bool,
    parts: BTreeMap<chunks::ChunkKey, ChunkParts>,
}

impl VoxelVolume {
    /// A volume with cubic chunks of `chunk_size` voxels. Non-positive or
    /// non-finite voxel sizes fall back to 1.0.
    pub fn new(chunk_size: u32, voxel_size: f32, palette: VoxelPalette) -> Result<Self, VoxelError> {
        Ok(Self {
            palette,
            voxel_size: if voxel_size.is_finite() && voxel_size > 0.0 { voxel_size } else { 1.0 },
            slice: None,
            voxels: Vec::new(),
            mesher: ChunkedVoxelMesher::new(chunk_size)?,
            dirty: true,
            parts: BTreeMap::new(),
        })
    }

    /// Replaces the voxel set; only affected chunks are re-meshed. Rejects
    /// (keeping the previous set) voxels outside `±`[`MAX_VOXEL_COORD`].
    pub fn set_voxels(&mut self, voxels: Vec<Voxel>) -> Result<(), VoxelError> {
        check_voxels(&voxels)?;
        self.voxels = voxels;
        self.dirty = true;
        Ok(())
    }

    /// Current voxel set (before slicing).
    pub fn voxels(&self) -> &[Voxel] {
        &self.voxels
    }

    /// Sets or clears the slice plane.
    pub fn set_slice(&mut self, slice: Option<VoxelSlice>) {
        if self.slice != slice {
            self.slice = slice;
            self.dirty = true;
        }
    }

    /// Current slice plane.
    pub fn slice(&self) -> Option<VoxelSlice> {
        self.slice
    }

    /// Replaces the palette (re-meshes everything).
    pub fn set_palette(&mut self, palette: VoxelPalette) {
        if self.palette != palette {
            self.palette = palette;
            self.dirty = true;
        }
    }

    /// The palette.
    pub fn palette(&self) -> &VoxelPalette {
        &self.palette
    }

    /// Edge length of one voxel in world units.
    pub fn voxel_size(&self) -> f32 {
        self.voxel_size
    }

    /// Number of non-empty chunks after the last rebuild.
    pub fn chunk_count(&self) -> usize {
        self.mesher.chunk_count()
    }

    /// Faces drawn after the last rebuild.
    pub fn face_count(&self) -> usize {
        self.parts.values().map(|p| p.faces).sum()
    }
}

fn rebuild_voxel_volumes(
    mut commands: Commands,
    materials: Option<Res<VoxelMaterials>>,
    mut volumes: Query<(Entity, &mut VoxelVolume)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(materials) = materials else { return };
    for (volume_entity, mut volume) in &mut volumes {
        if !volume.dirty {
            continue;
        }
        volume.dirty = false;
        let volume = &mut *volume;
        let visible = match volume.slice {
            Some(slice) => slice.apply(&volume.voxels),
            None => volume.voxels.clone(),
        };
        // `set_voxels` validated the coordinates, so this cannot fail; log
        // instead of panicking should that invariant ever break.
        let diff = match volume.mesher.rebuild(&visible, &volume.palette, volume.voxel_size) {
            Ok(diff) => diff,
            Err(err) => {
                error!("voxel volume {volume_entity}: {err}");
                continue;
            }
        };
        for chunk in diff.removed {
            if let Some(parts) = volume.parts.remove(&chunks::chunk_key(chunk)) {
                for (entity, _) in [parts.opaque, parts.translucent].into_iter().flatten() {
                    commands.entity(entity).despawn();
                }
            }
        }
        for update in diff.updated {
            let translation = update.origin.as_vec3() * volume.voxel_size;
            let parts = volume.parts.entry(chunks::chunk_key(update.chunk)).or_default();
            parts.faces = update.meshes.face_count();
            let mut sync = |slot: &mut Option<(Entity, Handle<Mesh>)>, data: &VoxelMeshData, translucent: bool| {
                if data.is_empty() {
                    if let Some((entity, _)) = slot.take() {
                        commands.entity(entity).despawn();
                    }
                    return;
                }
                if let Some((_, handle)) = slot.as_ref()
                    && let Some(mut mesh) = meshes.get_mut(handle)
                {
                    *mesh = data.to_mesh();
                    return;
                }
                let handle = meshes.add(data.to_mesh());
                let material = if translucent { materials.translucent.clone() } else { materials.opaque.clone() };
                let mut entity = commands.spawn((
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(translation),
                    ChildOf(volume_entity),
                ));
                if translucent {
                    entity.insert(NotShadowCaster);
                }
                *slot = Some((entity.id(), handle));
            };
            sync(&mut parts.opaque, &update.meshes.opaque, false);
            sync(&mut parts.translucent, &update.meshes.translucent, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voxel_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .add_systems(Startup, create_voxel_materials)
            .add_systems(Update, rebuild_voxel_volumes);
        app
    }

    fn palette() -> VoxelPalette {
        VoxelPalette::new(vec![Color::NONE, Color::WHITE, Color::srgba(0.5, 0.8, 1.0, 0.3)]).expect("non-empty")
    }

    fn chunk_children(app: &mut App, volume: Entity) -> Vec<(Vec3, usize)> {
        let mut query = app.world_mut().query::<(&ChildOf, &Transform, &Mesh3d)>();
        let world = app.world();
        let meshes = world.resource::<Assets<Mesh>>();
        let mut out: Vec<(Vec3, usize)> = query
            .iter(world)
            .filter(|(parent, _, _)| parent.parent() == volume)
            .map(|(_, t, m)| (t.translation, meshes.get(&m.0).and_then(|m| m.indices()).map_or(0, |i| i.len() / 6)))
            .collect();
        out.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.1.cmp(&b.1)));
        out
    }

    #[test]
    fn volume_spawns_and_updates_chunk_entities() {
        let mut app = voxel_app();
        let mut volume = VoxelVolume::new(4, 0.5, palette()).expect("valid");
        // Two opaque voxels in chunk 0, one glass voxel in chunk 2.
        volume
            .set_voxels(vec![Voxel::new(0, 0, 0, 1), Voxel::new(1, 0, 0, 1), Voxel::new(9, 0, 0, 2)])
            .expect("in range");
        let entity = app.world_mut().spawn(volume).id();
        app.update();
        let children = chunk_children(&mut app, entity);
        assert_eq!(children, vec![(Vec3::ZERO, 10), (Vec3::new(4.0, 0.0, 0.0), 6)]);
        let volume = app.world().get::<VoxelVolume>(entity).expect("volume");
        assert_eq!((volume.chunk_count(), volume.face_count()), (2, 16));

        // Slicing away x >= 1 removes chunk 2 and re-meshes chunk 0.
        app.world_mut().get_mut::<VoxelVolume>(entity).expect("volume").set_slice(Some(VoxelSlice {
            axis: SliceAxis::X,
            position: 0,
            keep: SliceKeep::Below,
        }));
        app.update();
        assert_eq!(chunk_children(&mut app, entity), vec![(Vec3::ZERO, 6)]);
        assert_eq!(app.world().get::<VoxelVolume>(entity).expect("volume").face_count(), 6);
    }

    #[test]
    fn volume_rejects_bad_settings() {
        assert_eq!(VoxelVolume::new(0, 1.0, palette()).err(), Some(VoxelError::ChunkSize(0)));
        let mut volume = VoxelVolume::new(8, f32::NAN, palette()).expect("valid chunk size");
        assert_eq!(volume.voxel_size(), 1.0);
        volume.set_voxels(vec![Voxel::new(1, 2, 3, 1)]).expect("in range");
        let far = Voxel::new(0, -(MAX_VOXEL_COORD + 1), 0, 1);
        assert_eq!(volume.set_voxels(vec![far]), Err(VoxelError::OutOfRange { voxel: far }));
        assert_eq!(volume.voxels(), &[Voxel::new(1, 2, 3, 1)], "previous voxels kept");
    }
}
