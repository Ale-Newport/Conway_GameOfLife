//! Terrain, water and particles demo.
//!
//! A procedural island (fractal value noise with a radial falloff — example
//! code, not the `physics-world` generator) meshed with biome vertex colours,
//! an animated water surface over everything below sea level, and every
//! particle preset: a campfire (fire, smoke, embers), snow over the summit,
//! a rain shower over the sea, splashes at the shore and dust on the beach.
//!
//! Controls (strategic camera): pan with right drag or WASD/arrows, orbit with
//! middle drag or Q/E, zoom with the wheel; 1–4 switch camera preset; L cycles
//! the terrain level of detail (1, 2, 4); F focuses the campfire.
//! `EMERGENCE_SMOKE_SCREENSHOT=out.png` captures a screenshot and exits.

use bevy::light::CascadeShadowConfigBuilder;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCamera;
use rendering::terrain::{TerrainSettings, terrain_mesh, water_mesh};
use rendering::water::WaterSettings;
use rendering::{
    OrbitCameraPlugin, OrbitCameraRig, OrbitPreset, ParticleEmitter, ParticlePlugin, ParticlePreset, ParticleWind,
    SmokeScreenshotPlugin, StandaloneWorldInputPlugin, WaterMaterial, WaterPlugin, WorldInput,
};

const SIZE: usize = 192;
const HEIGHT_SCALE: f32 = 42.0;
const SEA_LEVEL: f32 = 0.30;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Emergence Lab - terrain".into(),
                resolution: (1400, 900).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            WaterPlugin,
            ParticlePlugin,
            OrbitCameraPlugin,
            StandaloneWorldInputPlugin,
            SmokeScreenshotPlugin { frames: 90 },
        ))
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.85, 0.9, 1.0), brightness: 350.0, ..default() })
        .insert_resource(ParticleWind(Vec3::new(2.0, 0.0, 0.8)))
        .add_systems(Startup, setup)
        .add_systems(Update, controls)
        .run();
}

#[derive(Resource)]
struct Island {
    heights: Vec<f32>,
    colors: Vec<Color>,
    lod_step: usize,
    terrain: Entity,
    campfire: Vec3,
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h =
        (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ seed.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0x00FF_FFFF) as f32 / 16_777_215.0
}

fn value_noise(p: Vec2, seed: u32) -> f32 {
    let i = p.floor();
    let f = p - i;
    let s = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let (x, y) = (i.x as i32, i.y as i32);
    let a = hash(x, y, seed);
    let b = hash(x + 1, y, seed);
    let c = hash(x, y + 1, seed);
    let d = hash(x + 1, y + 1, seed);
    a + (b - a) * s.x + (c - a) * s.y + (a - b - c + d) * s.x * s.y
}

fn island_heights() -> Vec<f32> {
    let mut heights = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = Vec2::new(x as f32, y as f32) / SIZE as f32;
            let mut amp = 0.5;
            let mut freq = 4.0;
            let mut n = 0.0;
            for octave in 0..6 {
                n += amp * value_noise(p * freq, 17 + octave);
                amp *= 0.5;
                freq *= 2.0;
            }
            let r = (p - Vec2::splat(0.5)).length() * 2.0;
            let falloff = (1.0 - r.powf(2.2)).clamp(0.0, 1.0);
            // A peak north-west of the centre for the snow cap.
            let peak = 0.55 * (-((p - Vec2::new(0.42, 0.40)).length_squared()) / 0.012).exp();
            heights.push((n * 0.95 * falloff + peak * falloff).clamp(0.0, 1.2));
        }
    }
    heights
}

fn biome_color(h: f32, slope: f32) -> Color {
    let lerp = |a: Color, b: Color, t: f32| a.mix(&b, t.clamp(0.0, 1.0));
    let sand = Color::srgb(0.80, 0.74, 0.52);
    let grass = Color::srgb(0.30, 0.55, 0.22);
    let forest = Color::srgb(0.13, 0.36, 0.16);
    let rock = Color::srgb(0.45, 0.42, 0.40);
    let snow = Color::srgb(0.95, 0.96, 0.98);
    let seabed = Color::srgb(0.30, 0.32, 0.25);
    let base = if h < SEA_LEVEL - 0.02 {
        seabed
    } else if h < SEA_LEVEL + 0.03 {
        sand
    } else if h < 0.45 {
        lerp(grass, forest, (h - (SEA_LEVEL + 0.03)) / 0.12)
    } else if h < 0.65 {
        lerp(forest, rock, (h - 0.45) / 0.2)
    } else {
        lerp(rock, snow, (h - 0.68) / 0.08)
    };
    if slope > 1.1 && h >= SEA_LEVEL { lerp(base, rock, (slope - 1.1) * 1.5) } else { base }
}

fn island_colors(heights: &[f32]) -> Vec<Color> {
    let at = |x: usize, y: usize| heights[y.min(SIZE - 1) * SIZE + x.min(SIZE - 1)];
    (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let dx = at(x + 1, y) - at(x.saturating_sub(1), y);
            let dy = at(x, y + 1) - at(x, y.saturating_sub(1));
            let slope = (dx * dx + dy * dy).sqrt() * HEIGHT_SCALE * 0.5;
            biome_color(heights[i], slope)
        })
        .collect()
}

fn world_at(heights: &[f32], x: usize, y: usize) -> Vec3 {
    Vec3::new(x as f32 + 0.5, heights[y * SIZE + x] * HEIGHT_SCALE, y as f32 + 0.5)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
) {
    let heights = island_heights();
    let colors = island_colors(&heights);
    let settings = TerrainSettings { cell_size: 1.0, height_scale: HEIGHT_SCALE, lod_step: 1 };
    let terrain_material =
        materials.add(StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.92, ..default() });
    let terrain = match terrain_mesh(&heights, SIZE, SIZE, Some(&colors), &settings) {
        Ok(data) => commands.spawn((Mesh3d(meshes.add(data.to_mesh())), MeshMaterial3d(terrain_material))).id(),
        Err(err) => {
            error!("terrain: {err}");
            return;
        }
    };

    let sun = Transform::from_xyz(-60.0, 90.0, 40.0).looking_at(Vec3::ZERO, Vec3::Y);
    let depth: Vec<f32> = heights.iter().map(|h| (SEA_LEVEL - h).max(0.0)).collect();
    match water_mesh(&heights, &depth, SIZE, SIZE, 0.002, &settings) {
        Ok(data) => {
            let material =
                water_materials.add(WaterMaterial { settings: WaterSettings::default().with_sun_from(&sun) });
            commands.spawn((Mesh3d(meshes.add(data.to_mesh())), MeshMaterial3d(material)));
        }
        Err(err) => error!("water: {err}"),
    }

    // Particle emitters on interesting spots (searched deterministically).
    let find = |pred: &dyn Fn(f32) -> bool, from: (usize, usize)| -> (usize, usize) {
        let mut best = from;
        let mut best_d = usize::MAX;
        for y in 8..SIZE - 8 {
            for x in 8..SIZE - 8 {
                let d = x.abs_diff(from.0).pow(2) + y.abs_diff(from.1).pow(2);
                if pred(heights[y * SIZE + x]) && d < best_d {
                    best = (x, y);
                    best_d = d;
                }
            }
        }
        best
    };
    let summit = (0..SIZE * SIZE).max_by(|&a, &b| heights[a].total_cmp(&heights[b])).unwrap_or(0);
    let summit = world_at(&heights, summit % SIZE, summit / SIZE);
    let (fx, fy) = find(&|h| (0.36..0.42).contains(&h), (120, 110));
    let campfire = world_at(&heights, fx, fy);
    // Beach, shoreline and open sea closest to the campfire, so one view shows them all.
    let (bx, by) = find(&|h| (SEA_LEVEL + 0.005..SEA_LEVEL + 0.03).contains(&h), (fx, fy));
    let (sx, sy) = find(&|h| (SEA_LEVEL - 0.01..SEA_LEVEL).contains(&h), (fx, fy));
    let (rx, ry) = find(&|h| h < SEA_LEVEL - 0.08, (fx, fy));
    // Presets are in metres; the effects are enlarged (geometrically similar
    // scaling) so they read at this camera distance.
    let emitters = [
        (ParticlePreset::Fire, campfire, 4.0, 1),
        (ParticlePreset::Smoke, campfire + Vec3::Y * 3.0, 4.0, 2),
        (ParticlePreset::Embers, campfire + Vec3::Y * 1.0, 4.0, 3),
        (ParticlePreset::Snow, summit + Vec3::Y * 20.0, 3.0, 4),
        (ParticlePreset::Rain, world_at(&heights, rx, ry).with_y(SEA_LEVEL * HEIGHT_SCALE + 30.0), 2.0, 5),
        (ParticlePreset::Splash, world_at(&heights, sx, sy) + Vec3::Y * 0.1, 3.0, 6),
        (ParticlePreset::Dust, world_at(&heights, bx, by) + Vec3::Y * 0.5, 3.0, 7),
    ];
    for (preset, position, scale, seed) in emitters {
        let config = preset.config().scaled(scale);
        commands.spawn((ParticleEmitter::new(config, seed), Transform::from_translation(position)));
    }
    // A log pile under the fire.
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(0.12, 1.2))),
        MeshMaterial3d(materials.add(Color::srgb(0.25, 0.15, 0.08))),
        Transform::from_translation(campfire + Vec3::Y * 0.3)
            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(4.0)),
    ));

    commands.spawn((
        DirectionalLight { illuminance: 11_000.0, shadow_maps_enabled: true, ..default() },
        sun,
        CascadeShadowConfigBuilder { maximum_distance: 400.0, first_cascade_far_bound: 80.0, ..default() }.build(),
    ));
    commands.spawn((
        OrbitCameraRig::new(OrbitPreset::Strategic).with_distance_limits(4.0, 600.0),
        PanOrbitCamera { focus: campfire.lerp(summit, 0.3), ..default() },
        Transform::from_translation(campfire.lerp(summit, 0.3) + Vec3::new(30.0, 52.0, 58.0)),
        DistanceFog {
            color: Color::srgba(0.62, 0.74, 0.88, 1.0),
            falloff: FogFalloff::Linear { start: 220.0, end: 520.0 },
            ..default()
        },
    ));
    commands.insert_resource(Island { heights, colors, lod_step: 1, terrain, campfire });
}

fn controls(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut island: Option<ResMut<Island>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mesh_handles: Query<&Mesh3d>,
    mut rigs: Query<(&mut OrbitCameraRig, &mut PanOrbitCamera, &Projection)>,
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
            for (mut rig, _, _) in &mut rigs {
                rig.preset = preset;
            }
        }
    }
    let Some(island) = island.as_mut() else { return };
    if keys.just_pressed(KeyCode::KeyF) {
        for (rig, mut cam, projection) in &mut rigs {
            rig.focus_on(&mut cam, projection, island.campfire, 25.0);
        }
    }
    if keys.just_pressed(KeyCode::KeyL) {
        island.lod_step = match island.lod_step {
            1 => 2,
            2 => 4,
            _ => 1,
        };
        let settings = TerrainSettings { cell_size: 1.0, height_scale: HEIGHT_SCALE, lod_step: island.lod_step };
        match terrain_mesh(&island.heights, SIZE, SIZE, Some(&island.colors), &settings) {
            Ok(data) => {
                if let Ok(handle) = mesh_handles.get(island.terrain)
                    && let Some(mut mesh) = meshes.get_mut(&handle.0)
                {
                    *mesh = data.to_mesh();
                    info!("terrain LOD step {}: {} triangles", island.lod_step, data.triangle_count());
                }
            }
            Err(err) => error!("terrain: {err}"),
        }
    }
}
