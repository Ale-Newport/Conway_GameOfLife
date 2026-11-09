//! Lightweight CPU particles rendered as camera-facing billboards.
//!
//! Each [`ParticleEmitter`] simulates its own particles in world space (see
//! [`sim`] for the integration scheme). Every frame all particles that share a
//! [`ParticleBlend`] mode are written into **one** dynamic mesh (two draw calls
//! in total): additive particles in any order, alpha-blended particles sorted
//! back to front. Quads face the active 3D [`crate::WorldCamera`]; with
//! `stretch > 0` they elongate along the velocity projected on the view plane
//! (rain streaks, sparks). A procedurally generated soft disc texture gives
//! round particles.
//!
//! Batching per blend mode (rather than one mesh per preset) keeps the draw
//! count at two and sorts all alpha particles correctly against each other.
//! Bevy sorts each batch against other transparent objects (water,
//! translucent voxels) by the centre of the batch's bounding box, so a
//! particle and a transparent surface can composite in the wrong order where
//! they overlap (documented limitation).

pub mod presets;
pub mod sim;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

pub use presets::ParticlePreset;
pub use sim::{
    EmitterShape, Particle, ParticleBlend, ParticleConfig, ParticleRng, ParticleState, sample_color, sample_scalar,
};

use crate::WorldCamera;

/// Simulates emitters and rebuilds the batched billboard meshes in
/// `PostUpdate`, after transform propagation (emitter and camera positions
/// are current).
pub struct ParticlePlugin;

impl Plugin for ParticlePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ParticleWind>().add_systems(Startup, create_particle_batches).add_systems(
            PostUpdate,
            (simulate_emitters, build_particle_meshes).chain().after(TransformSystems::Propagate),
        );
    }
}

/// Air velocity (m/s, world space) applied to particles in proportion to
/// their `wind_influence`; set it from the weather model.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct ParticleWind(pub Vec3);

/// A particle emitter at its entity's `GlobalTransform`.
#[derive(Component, Debug, Clone)]
#[require(Transform, Visibility)]
pub struct ParticleEmitter {
    /// Behaviour.
    pub config: ParticleConfig,
    /// Whether continuous emission is on (bursts work either way).
    pub emitting: bool,
    state: ParticleState,
    pending_burst: usize,
}

impl ParticleEmitter {
    /// An emitter with a custom configuration; `seed` drives its randomness.
    pub fn new(config: ParticleConfig, seed: u64) -> Self {
        Self { config, emitting: true, state: ParticleState::new(seed), pending_burst: 0 }
    }

    /// An emitter using a preset.
    pub fn preset(preset: ParticlePreset, seed: u64) -> Self {
        Self::new(preset.config(), seed)
    }

    /// Spawns `count` particles at the next update (e.g. a splash on impact).
    pub fn burst(&mut self, count: usize) {
        self.pending_burst = self.pending_burst.saturating_add(count);
    }

    /// Live particle count.
    pub fn particle_count(&self) -> usize {
        self.state.particles().len()
    }

    /// Live particles.
    pub fn particles(&self) -> &[Particle] {
        self.state.particles()
    }
}

/// Batch mesh entity for one blend mode.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParticleBatch(pub ParticleBlend);

#[derive(Resource, Debug, Clone)]
struct ParticleBatches {
    additive: (Entity, Handle<Mesh>),
    alpha: (Entity, Handle<Mesh>),
}

/// Radial soft-disc sprite (white, alpha falling smoothly to 0 at the rim).
fn soft_disc_image(size: u32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) * 0.5;
    for y in 0..size {
        for x in 0..size {
            let d = Vec2::new(x as f32 - c, y as f32 - c).length() / (c + 0.5);
            let a = (1.0 - d * d).clamp(0.0, 1.0).powi(2);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0).round() as u8]);
        }
    }
    Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn empty_mesh() -> Mesh {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD)
}

/// A single invisible (zero-size, transparent) quad: batch meshes start with
/// it because the renderer rejects meshes without vertices.
fn placeholder_buffers() -> BillboardBuffers {
    let mut buffers = BillboardBuffers::default();
    let hidden =
        Billboard { position: Vec3::ZERO, velocity: Vec3::ZERO, size: 0.0, color: LinearRgba::NONE, stretch: 0.0 };
    buffers.push(&hidden, Vec3::Z, Vec3::X, Vec3::Y);
    buffers
}

fn create_particle_batches(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let texture = images.add(soft_disc_image(64));
    let mut spawn = |blend: ParticleBlend| {
        let mesh = meshes.add(placeholder_buffers().into_mesh(empty_mesh()));
        let material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(texture.clone()),
            unlit: true,
            cull_mode: None,
            double_sided: true,
            alpha_mode: match blend {
                ParticleBlend::Additive => AlphaMode::Add,
                ParticleBlend::Alpha => AlphaMode::Blend,
            },
            ..default()
        });
        let entity = commands
            .spawn((
                ParticleBatch(blend),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                Visibility::Hidden,
                NotShadowCaster,
                NotShadowReceiver,
                // The mesh is rebuilt after transform propagation every frame;
                // an AABB computed from last frame's particles would cull the
                // new ones. A world-spanning batch gains nothing from culling.
                NoFrustumCulling,
            ))
            .id();
        (entity, mesh)
    };
    let additive = spawn(ParticleBlend::Additive);
    let alpha = spawn(ParticleBlend::Alpha);
    commands.insert_resource(ParticleBatches { additive, alpha });
}

fn simulate_emitters(
    time: Res<Time>,
    wind: Res<ParticleWind>,
    mut emitters: Query<(&mut ParticleEmitter, &GlobalTransform)>,
) {
    let dt = time.delta_secs();
    for (mut emitter, transform) in &mut emitters {
        let emitter = &mut *emitter;
        if emitter.pending_burst > 0 {
            emitter.state.burst(&emitter.config, transform, emitter.pending_burst);
            emitter.pending_burst = 0;
        }
        emitter.state.step(&emitter.config, transform, emitter.emitting, wind.0, dt);
    }
}

/// One billboard to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Billboard {
    /// Centre in world space.
    pub position: Vec3,
    /// Velocity (for stretching).
    pub velocity: Vec3,
    /// Edge length in world units.
    pub size: f32,
    /// Linear colour with alpha.
    pub color: LinearRgba,
    /// Stretch factor (seconds; 0 = square).
    pub stretch: f32,
}

/// Vertex buffers of a batch of billboards.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BillboardBuffers {
    /// Positions.
    pub positions: Vec<[f32; 3]>,
    /// Normals (towards the camera).
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Linear RGBA colours.
    pub colors: Vec<[f32; 4]>,
    /// Indices (two triangles per quad).
    pub indices: Vec<u32>,
}

impl BillboardBuffers {
    /// Clears all buffers, keeping their allocations.
    pub fn clear(&mut self) {
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        self.colors.clear();
        self.indices.clear();
    }

    /// Appends a quad facing a camera at `camera_position` with the given
    /// camera right/up axes.
    pub fn push(&mut self, b: &Billboard, camera_position: Vec3, camera_right: Vec3, camera_up: Vec3) {
        let to_camera = (camera_position - b.position).normalize_or(Vec3::Z);
        let half = b.size * 0.5;
        let (axis_u, axis_v) = if b.stretch > 0.0 {
            // Velocity projected on the view plane gives the streak direction.
            let planar = b.velocity - to_camera * b.velocity.dot(to_camera);
            let speed = planar.length();
            if speed > 1e-4 {
                let along = planar / speed;
                let across = along.cross(to_camera).normalize_or(camera_right);
                (across * half, along * (half + 0.5 * speed * b.stretch))
            } else {
                (camera_right * half, camera_up * half)
            }
        } else {
            (camera_right * half, camera_up * half)
        };
        let base = self.positions.len() as u32;
        let corners =
            [(-1.0, -1.0, [0.0, 1.0]), (1.0, -1.0, [1.0, 1.0]), (1.0, 1.0, [1.0, 0.0]), (-1.0, 1.0, [0.0, 0.0])];
        for (su, sv, uv) in corners {
            self.positions.push((b.position + axis_u * su + axis_v * sv).to_array());
            self.normals.push(to_camera.to_array());
            self.uvs.push(uv);
            self.colors.push(b.color.to_f32_array());
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Writes the buffers into `mesh` (replacing its attributes).
    pub fn into_mesh(self, mut mesh: Mesh) -> Mesh {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

/// Billboards for all live particles of an emitter.
pub fn emitter_billboards(config: &ParticleConfig, particles: &[Particle], out: &mut Vec<Billboard>) {
    for p in particles {
        let t = p.life_fraction();
        let color = sample_color(&config.color_over_life, t);
        let size = sample_scalar(&config.size_over_life, t) * p.size_scale;
        if color.alpha <= 0.0 || size <= 0.0 {
            continue;
        }
        out.push(Billboard { position: p.position, velocity: p.velocity, size, color, stretch: config.stretch });
    }
}

/// Sorts billboards back to front (farthest from the camera first).
pub fn sort_back_to_front(billboards: &mut [Billboard], camera_position: Vec3) {
    billboards.sort_by(|a, b| {
        let da = a.position.distance_squared(camera_position);
        let db = b.position.distance_squared(camera_position);
        db.total_cmp(&da)
    });
}

#[allow(clippy::type_complexity)]
fn build_particle_meshes(
    batches: Option<Res<ParticleBatches>>,
    emitters: Query<(&ParticleEmitter, &InheritedVisibility)>,
    cameras: Query<(&Camera, &GlobalTransform), (With<WorldCamera>, With<Camera3d>)>,
    mut visibility: Query<&mut Visibility, With<ParticleBatch>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut scratch: Local<(Vec<Billboard>, Vec<Billboard>)>,
) {
    let Some(batches) = batches else { return };
    let Some((_, camera)) = cameras.iter().find(|(c, _)| c.is_active) else { return };
    let (additive, alpha) = &mut *scratch;
    additive.clear();
    alpha.clear();
    for (emitter, inherited) in &emitters {
        if !inherited.get() {
            continue;
        }
        let target = match emitter.config.blend {
            ParticleBlend::Additive => &mut *additive,
            ParticleBlend::Alpha => &mut *alpha,
        };
        emitter_billboards(&emitter.config, emitter.particles(), target);
    }
    let cam_pos = camera.translation();
    let (right, up) = (camera.right().as_vec3(), camera.up().as_vec3());
    sort_back_to_front(alpha, cam_pos);
    for ((entity, handle), list) in [(&batches.additive, &*additive), (&batches.alpha, &*alpha)] {
        let target_visibility = if list.is_empty() { Visibility::Hidden } else { Visibility::Inherited };
        if let Ok(mut vis) = visibility.get_mut(*entity) {
            vis.set_if_neq(target_visibility);
        }
        // An empty batch is hidden; its last mesh is kept (empty meshes are invalid).
        if list.is_empty() {
            continue;
        }
        let Some(mut mesh) = meshes.get_mut(handle) else { continue };
        let mut buffers = BillboardBuffers::default();
        for b in list {
            buffers.push(b, cam_pos, right, up);
        }
        *mesh = buffers.into_mesh(empty_mesh());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn billboard(position: Vec3, velocity: Vec3, stretch: f32) -> Billboard {
        Billboard { position, velocity, size: 2.0, color: LinearRgba::WHITE, stretch }
    }

    #[test]
    fn quads_face_the_camera() {
        let mut buf = BillboardBuffers::default();
        let cam = Vec3::new(0.0, 0.0, 10.0);
        buf.push(&billboard(Vec3::ZERO, Vec3::ZERO, 0.0), cam, Vec3::X, Vec3::Y);
        assert_eq!(buf.positions.len(), 4);
        assert_eq!(buf.indices.len(), 6);
        let p = |i: u32| Vec3::from_array(buf.positions[i as usize]);
        for tri in buf.indices.as_chunks::<3>().0 {
            let n = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            assert!(n.dot(cam) > 0.0, "front face towards the camera");
        }
        assert_eq!(p(0), Vec3::new(-1.0, -1.0, 0.0));
        assert_eq!(p(2), Vec3::new(1.0, 1.0, 0.0));
    }

    #[test]
    fn stretched_quads_follow_velocity() {
        let mut buf = BillboardBuffers::default();
        let cam = Vec3::new(0.0, 0.0, 10.0);
        // Falling at 10 m/s with stretch 0.1 → half-length 1 + 0.5 = 1.5 along -Y... (axis sign irrelevant).
        buf.push(&billboard(Vec3::ZERO, Vec3::new(0.0, -10.0, 0.0), 0.1), cam, Vec3::X, Vec3::Y);
        let ys: Vec<f32> = buf.positions.iter().map(|p| p[1]).collect();
        let xs: Vec<f32> = buf.positions.iter().map(|p| p[0]).collect();
        let span_y = ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min);
        let span_x = xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min);
        assert!((span_y - 3.0).abs() < 1e-5, "{span_y}");
        assert!((span_x - 2.0).abs() < 1e-5, "{span_x}");
        // Velocity along the view direction cannot stretch: falls back to a square.
        let mut buf = BillboardBuffers::default();
        buf.push(&billboard(Vec3::ZERO, Vec3::new(0.0, 0.0, -5.0), 0.1), cam, Vec3::X, Vec3::Y);
        assert_eq!(buf.positions[2], [1.0, 1.0, 0.0]);
    }

    #[test]
    fn alpha_sorting_is_back_to_front() {
        let cam = Vec3::ZERO;
        let mut list = vec![
            billboard(Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO, 0.0),
            billboard(Vec3::new(0.0, 0.0, -9.0), Vec3::ZERO, 0.0),
            billboard(Vec3::new(0.0, 0.0, -4.0), Vec3::ZERO, 0.0),
        ];
        sort_back_to_front(&mut list, cam);
        let z: Vec<f32> = list.iter().map(|b| b.position.z).collect();
        assert_eq!(z, vec![-9.0, -4.0, -1.0]);
    }

    #[test]
    fn billboards_use_life_curves_and_skip_invisible() {
        let config = ParticleConfig {
            color_over_life: vec![
                (0.0, LinearRgba::new(1.0, 0.0, 0.0, 1.0)),
                (1.0, LinearRgba::new(0.0, 0.0, 1.0, 0.0)),
            ],
            size_over_life: vec![(0.0, 1.0), (1.0, 3.0)],
            ..ParticlePreset::Smoke.config()
        };
        let particle =
            |age: f32| Particle { position: Vec3::ZERO, velocity: Vec3::ZERO, age, lifetime: 2.0, size_scale: 1.0 };
        let mut out = Vec::new();
        emitter_billboards(&config, &[particle(1.0), particle(2.0)], &mut out);
        assert_eq!(out.len(), 1, "fully transparent particle skipped");
        assert_eq!(out[0].size, 2.0);
        assert_eq!(out[0].color, LinearRgba::new(0.5, 0.0, 0.5, 0.5));
    }

    #[test]
    fn soft_disc_is_opaque_in_the_centre_and_clear_at_the_rim() {
        let img = soft_disc_image(16);
        let data = img.data.as_ref().expect("cpu data");
        let alpha = |x: usize, y: usize| data[(y * 16 + x) * 4 + 3];
        assert!(alpha(8, 8) > 230);
        assert_eq!(alpha(0, 0), 0);
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[test]
    fn emitters_feed_one_batched_mesh_per_blend_mode() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(50)))
            .init_resource::<ParticleWind>()
            .add_systems(Startup, create_particle_batches)
            .add_systems(Update, (simulate_emitters, build_particle_meshes).chain());
        app.world_mut().spawn((Camera3d::default(), WorldCamera, GlobalTransform::from_xyz(0.0, 2.0, 20.0)));
        let quiet = |preset: ParticlePreset| ParticleConfig { rate: 20.0, turbulence: 0.0, ..preset.config() };
        for (i, preset) in [ParticlePreset::Fire, ParticlePreset::Smoke, ParticlePreset::Dust].into_iter().enumerate() {
            app.world_mut().spawn((
                ParticleEmitter::new(quiet(preset), i as u64),
                GlobalTransform::from_xyz(i as f32 * 3.0, 0.0, 0.0),
                InheritedVisibility::VISIBLE,
            ));
        }
        for _ in 0..11 {
            app.update();
        }
        let counts: Vec<usize> = {
            let mut q = app.world_mut().query::<&ParticleEmitter>();
            q.iter(app.world()).map(ParticleEmitter::particle_count).collect()
        };
        // 0.5 s of simulated time at 20 particles/s (the first update has dt = 0).
        assert!(counts.iter().all(|&c| (9..=11).contains(&c)), "{counts:?}");
        let batches = app.world().resource::<ParticleBatches>().clone();
        let meshes = app.world().resource::<Assets<Mesh>>();
        let quads = |handle: &Handle<Mesh>| meshes.get(handle).and_then(|m| m.indices()).map_or(0, |i| i.len() / 6);
        assert_eq!(quads(&batches.additive.1), counts[0], "fire is additive");
        assert_eq!(quads(&batches.alpha.1), counts[1] + counts[2], "smoke and dust share the alpha batch");
        let vis = app.world().get::<Visibility>(batches.alpha.0).copied();
        assert_eq!(vis, Some(Visibility::Inherited));
        assert!(app.world().get::<NoFrustumCulling>(batches.additive.0).is_some(), "batches are never culled");
    }
}
