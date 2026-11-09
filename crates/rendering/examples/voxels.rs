//! Voxel meshing demo: a level-3 Menger sponge (opaque, coloured by height
//! layer), the same sponge cut by a slice plane to reveal its cross-section,
//! and a translucent glass shell around an opaque core.
//!
//! Controls: orbit with left drag, pan with right drag, zoom with the wheel;
//! WASD/arrows pan, Q/E rotate; 1–4 switch camera preset (aerial, isometric,
//! free orbit, strategic); `[` / `]` move the slice plane (only affected
//! chunks are re-meshed). `EMERGENCE_SMOKE_SCREENSHOT=out.png` captures a
//! screenshot and exits.

use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;
use rendering::voxel::{SliceAxis, SliceKeep, VoxelError, VoxelSlice};
use rendering::{
    OrbitCameraPlugin, OrbitCameraRig, OrbitPreset, SmokeScreenshotPlugin, StandaloneWorldInputPlugin, Voxel,
    VoxelPalette, VoxelPlugin, VoxelVolume, WorldInput,
};

const GLASS: u8 = 7;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Emergence Lab - voxels".into(),
                resolution: (1400, 900).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((VoxelPlugin, OrbitCameraPlugin, StandaloneWorldInputPlugin, SmokeScreenshotPlugin::default()))
        .insert_resource(ClearColor(Color::srgb(0.55, 0.62, 0.72)))
        .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 400.0, ..default() })
        .add_systems(Startup, setup)
        .add_systems(Update, controls)
        .run();
}

#[derive(Component)]
struct SlicedVolume;

/// Whether `(x, y, z)` (0..27) belongs to the level-3 Menger sponge: no two
/// base-3 digits at the same position may both be 1.
fn in_sponge(mut x: i32, mut y: i32, mut z: i32) -> bool {
    for _ in 0..3 {
        let ones = [x % 3, y % 3, z % 3].iter().filter(|&&d| d == 1).count();
        if ones >= 2 {
            return false;
        }
        x /= 3;
        y /= 3;
        z /= 3;
    }
    true
}

fn sponge() -> Vec<Voxel> {
    let mut voxels = Vec::new();
    for x in 0..27 {
        for y in 0..27 {
            for z in 0..27 {
                if in_sponge(x, y, z) {
                    voxels.push(Voxel::new(x, y, z, 1 + (y / 5) as u8));
                }
            }
        }
    }
    voxels
}

fn glass_ball() -> Vec<Voxel> {
    let mut voxels = Vec::new();
    for x in -10..=10 {
        for y in -10..=10 {
            for z in -10..=10 {
                let r = ((x * x + y * y + z * z) as f32).sqrt();
                if (8.5..=10.0).contains(&r) {
                    voxels.push(Voxel::new(x, y + 10, z, GLASS));
                } else if r <= 4.5 {
                    voxels.push(Voxel::new(x, y + 10, z, 3));
                }
            }
        }
    }
    voxels
}

fn palette() -> Result<VoxelPalette, VoxelError> {
    VoxelPalette::new(vec![
        Color::NONE,
        Color::srgb(0.16, 0.20, 0.42),
        Color::srgb(0.18, 0.42, 0.62),
        Color::srgb(0.20, 0.66, 0.60),
        Color::srgb(0.58, 0.80, 0.36),
        Color::srgb(0.96, 0.80, 0.30),
        Color::srgb(0.96, 0.52, 0.28),
        Color::srgba(0.75, 0.90, 1.0, 0.28),
    ])
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let palette = match palette() {
        Ok(p) => p,
        Err(err) => {
            error!("invalid voxel palette: {err}");
            return;
        }
    };
    let volumes = [
        (Vec3::new(-44.0, 0.0, -13.5), None),
        (Vec3::new(-13.5, 0.0, -13.5), Some(VoxelSlice { axis: SliceAxis::Z, position: 13, keep: SliceKeep::Below })),
    ];
    let build_volume = |chunk_size: u32, voxel_size: f32, voxels: Vec<Voxel>, slice: Option<VoxelSlice>| {
        let mut volume = VoxelVolume::new(chunk_size, voxel_size, palette.clone())?;
        volume.set_voxels(voxels)?;
        volume.set_slice(slice);
        Ok::<_, VoxelError>(volume)
    };
    for (i, (position, slice)) in volumes.into_iter().enumerate() {
        match build_volume(9, 1.0, sponge(), slice) {
            Ok(volume) => {
                let mut entity = commands.spawn((volume, Transform::from_translation(position)));
                if i == 1 {
                    entity.insert(SlicedVolume);
                }
            }
            Err(err) => error!("voxel volume: {err}"),
        }
    }
    match build_volume(16, 1.2, glass_ball(), None) {
        Ok(ball) => {
            commands.spawn((ball, Transform::from_xyz(32.0, 0.0, 0.0)));
        }
        Err(err) => error!("voxel volume: {err}"),
    }

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(160.0, 120.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.32, 0.34, 0.36),
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.01, 0.0),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 9000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(-40.0, 80.0, 50.0).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder { maximum_distance: 260.0, first_cascade_far_bound: 60.0, ..default() }.build(),
    ));
    commands.spawn((
        OrbitCameraRig::new(OrbitPreset::FreeOrbit).with_distance_limits(2.0, 400.0),
        Transform::from_xyz(10.0, 70.0, 110.0).looking_at(Vec3::new(0.0, 12.0, 0.0), Vec3::Y),
        bevy_panorbit_camera::PanOrbitCamera { focus: Vec3::new(-6.0, 10.0, 0.0), ..default() },
    ));
}

fn controls(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut rigs: Query<&mut OrbitCameraRig>,
    mut sliced: Query<&mut VoxelVolume, With<SlicedVolume>>,
) {
    if !input.keyboard {
        return;
    }
    let presets = [
        (KeyCode::Digit1, OrbitPreset::Aerial),
        (KeyCode::Digit2, OrbitPreset::Isometric),
        (KeyCode::Digit3, OrbitPreset::FreeOrbit),
        (KeyCode::Digit4, OrbitPreset::Strategic),
    ];
    for (key, preset) in presets {
        if keys.just_pressed(key) {
            for mut rig in &mut rigs {
                rig.preset = preset;
            }
        }
    }
    let delta =
        i32::from(keys.just_pressed(KeyCode::BracketRight)) - i32::from(keys.just_pressed(KeyCode::BracketLeft));
    if delta != 0 {
        for mut volume in &mut sliced {
            if let Some(mut slice) = volume.slice() {
                slice.position = (slice.position + delta).clamp(0, 26);
                volume.set_slice(Some(slice));
                info!("slice z <= {}: {} chunks, {} faces", slice.position, volume.chunk_count(), volume.face_count());
            }
        }
    }
}
