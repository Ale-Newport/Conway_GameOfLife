//! The 3D scene: camera rig, sun and sky from the model's calendar and
//! weather, season and wind for the plants, and the request that tells
//! the session what to publish.
//!
//! Submodules build the landscape ([`terrain`]), the plants ([`flora`]),
//! the animals and small life ([`fauna`]) and the weather and fire
//! effects ([`effects`]).

pub mod effects;
pub mod fauna;
pub mod flora;
pub mod terrain;

use std::f32::consts::FRAC_PI_4;

use bevy::camera::Hdr;
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCamera;
use ecosystem::species::PlantLayer;
use ecosystem::view::EcosystemViewRequest;
use organism_render::{PlantSeason, Wind};
use physics_world::{WorldLayer, WorldViewRequest};
use rendering::{OrbitCameraRig, OrbitPreset, ParticleWind, WorldInput};

use crate::MODE;
use crate::coords::block_for;
use crate::palette::{Overlay, sky_state};
use crate::session::{FrameRequest, MarkerRequest};
use crate::state::{CameraMode, EcoData, EcoRunner, EcoUi};

/// Largest terrain grid side the view asks for (larger worlds are block means).
pub const MAX_GRID_SIDE: usize = 256;
/// Radius (cells) around the camera focus whose density-field markers are
/// drawn: the close-up neighbourhood. The model fills the marker budget
/// field by field, so a larger region would leave the last fields (often
/// the flying insects) without markers.
pub const MARKER_RADIUS_CELLS: usize = 4;
/// Largest number of density-field markers per frame.
pub const MARKER_LIMIT: usize = 700;
/// Half-life (s) of the follow camera's catch-up. Short: an animal moves
/// `speed × motion_seconds` per tick, i.e. hundreds of metres per real
/// second at 10 ticks/s, and its drawn motion is already smoothed between
/// frames; a long half-life would leave it far ahead of the view.
pub const FOLLOW_HALF_LIFE: f32 = 0.03;
/// Camera distance (m) beyond which small life is not requested.
pub const SMALL_LIFE_MAX_DISTANCE: f32 = 320.0;

/// The mode's camera.
#[derive(Component, Debug, Clone, Copy)]
pub struct EcoCamera;

/// The sun (and moon) light.
#[derive(Component, Debug, Clone, Copy)]
pub struct EcoSun;

/// Scene-wide state derived from frames.
#[derive(Resource, Debug, Default)]
pub struct SceneState {
    /// Last request sent to the session.
    pub sent: Option<FrameRequest>,
    /// Frames waited for a stable viewport while framing.
    frame_wait: u32,
    frame_stable: u32,
    framed_viewport: Option<UVec2>,
    /// Cinematic orbit angle accumulated (rad).
    cinematic_yaw: f32,
    /// Camera mode applied last frame.
    applied_mode: Option<CameraMode>,
    /// Pitch asked for before the terrain clearance lift.
    pitch_wanted: f32,
    /// Pitch the clearance left the camera at (to detect other changes).
    pitch_set: Option<f32>,
    /// Current sky colour (sRGB) for fog and water.
    pub sky: [f32; 3],
    /// Current daylight factor.
    pub daylight: f32,
    /// Unit vector towards the sun (the moon at night).
    pub light_direction: Vec3,
}

/// `OnEnter`: camera and sun.
pub fn spawn_scene(mut commands: Commands, mut state: ResMut<SceneState>) {
    *state = SceneState { sky: [0.5, 0.68, 0.9], daylight: 1.0, light_direction: Vec3::Y, ..SceneState::default() };
    commands.spawn((
        Name::new("Ecosystem camera"),
        EcoCamera,
        OrbitCameraRig::new(OrbitPreset::FreeOrbit).with_distance_limits(2.0, 9_000.0),
        PanOrbitCamera {
            focus: Vec3::new(1000.0, 0.0, 1000.0),
            radius: Some(2500.0),
            yaw: Some(0.0),
            pitch: Some(0.75),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection { fov: FRAC_PI_4, near: 0.1, far: 60_000.0, ..default() }),
        Transform::from_xyz(1000.0, 1800.0, 2800.0).looking_at(Vec3::new(1000.0, 0.0, 1000.0), Vec3::Y),
        Hdr,
        Bloom { intensity: 0.08, ..Bloom::NATURAL },
        AmbientLight { color: Color::srgb(0.8, 0.86, 1.0), brightness: 400.0, ..default() },
        DistanceFog {
            color: Color::srgb(0.5, 0.68, 0.9),
            directional_light_color: Color::srgba(1.0, 0.92, 0.8, 0.3),
            directional_light_exponent: 24.0,
            falloff: FogFalloff::Linear { start: 4_000.0, end: 12_000.0 },
        },
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Ecosystem sun"),
        EcoSun,
        DirectionalLight { illuminance: 10_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(0.0, 1.0, 0.0).looking_to(Vec3::new(-0.3, -1.0, -0.4), Vec3::Y),
        cascades(300.0),
        DespawnOnExit(MODE),
    ));
}

fn cascades(camera_distance: f32) -> CascadeShadowConfig {
    let maximum = (camera_distance * 2.5).clamp(80.0, 4_000.0);
    CascadeShadowConfigBuilder {
        num_cascades: 3,
        minimum_distance: 0.1,
        first_cascade_far_bound: (camera_distance * 0.5).clamp(10.0, maximum * 0.4),
        maximum_distance: maximum,
        ..default()
    }
    .build()
}

/// Pulls the newest published frame.
pub fn poll_frame(runner: Option<Res<EcoRunner>>, time: Res<Time<Real>>, mut data: ResMut<EcoData>) {
    data.fresh = false;
    let Some(runner) = runner else { return };
    let Some(published) = runner.0.latest_view() else { return };
    if published.seq == data.seq && data.frame.is_some() {
        return;
    }
    let now = time.elapsed_secs_f64();
    if data.frame.is_some() {
        let dt = (now - data.arrived) as f32;
        data.interval = if data.interval <= 0.0 { dt } else { data.interval * 0.8 + dt * 0.2 };
    }
    data.seq = published.seq;
    data.arrived = now;
    if published.view.trees_included {
        data.trees_frame = Some(published.view.clone());
        data.trees_seq = published.seq;
    }
    data.frame = Some(published.view);
    data.fresh = true;
}

/// Grid layers the view needs for the landscape and the current overlay.
pub fn world_layers(overlay: Overlay) -> Vec<WorldLayer> {
    let mut layers = vec![WorldLayer::WaterDepth, WorldLayer::Snow, WorldLayer::SoilMoisture, WorldLayer::Temperature];
    match overlay {
        Overlay::Nutrients => layers.push(WorldLayer::Nutrients),
        Overlay::FireRisk => layers.extend([WorldLayer::Fuel, WorldLayer::Humidity]),
        _ => {}
    }
    layers
}

/// Vegetation layers the view needs: those that colour the ground and grow
/// the ground cover, and every layer for the plant biomass overlay.
pub fn vegetation_layers(overlay: Overlay) -> Vec<PlantLayer> {
    if overlay == Overlay::PlantBiomass {
        PlantLayer::ALL.to_vec()
    } else {
        vec![PlantLayer::Grass, PlantLayer::Shrubs, PlantLayer::Flowers, PlantLayer::Moss, PlantLayer::Desert]
    }
}

/// The request for the current state of the view.
pub fn build_request(
    ui: &EcoUi,
    map: &terrain::MapState,
    world_size: Option<(usize, usize)>,
    focus: Option<Vec3>,
    camera_distance: f32,
) -> FrameRequest {
    let block = world_size.map_or(1, |(w, h)| block_for(w, h, MAX_GRID_SIDE));
    let markers =
        match (ui.layers.small_life && camera_distance < SMALL_LIFE_MAX_DISTANCE, focus, map.geometry.as_ref()) {
            (true, Some(f), Some(g)) => {
                let cell = g.plane_to_cell(f.x, f.z);
                // Quantised to 4 cells so a moving camera does not resend every frame.
                let (cx, cy) = (((cell.x.max(0.0) as usize) / 4) * 4, ((cell.y.max(0.0) as usize) / 4) * 4);
                let r = MARKER_RADIUS_CELLS;
                let (w, h) = world_size.unwrap_or((usize::MAX, usize::MAX));
                Some(MarkerRequest {
                    region: (cx.saturating_sub(r), cy.saturating_sub(r), (cx + r + 4).min(w), (cy + r + 4).min(h)),
                    limit: MARKER_LIMIT,
                })
            }
            _ => None,
        };
    FrameRequest {
        view: EcosystemViewRequest {
            world: WorldViewRequest {
                layers: world_layers(ui.overlay),
                fire: true,
                block,
                known_terrain_version: map.terrain_version,
            },
            region: None,
            animals: true,
            trees: true,
            remains: true,
            vegetation: vegetation_layers(ui.overlay),
            mushrooms: true,
            litter: false,
            block,
            field_markers: false,
            max_markers: 0,
            colonies: true,
            pheromone: ui.overlay == Overlay::Pheromone,
            pollination: ui.overlay == Overlay::Pollination,
        },
        known_epoch: map.epoch,
        territories: ui.overlay == Overlay::Territories,
        markers,
    }
}

/// Sends the request when it changed.
pub fn send_request(
    runner: Option<Res<EcoRunner>>,
    ui: Res<EcoUi>,
    data: Res<EcoData>,
    map: Res<terrain::MapState>,
    mut state: ResMut<SceneState>,
    camera: Option<Single<&PanOrbitCamera, With<EcoCamera>>>,
) {
    let Some(runner) = runner else { return };
    let size = data.get().map(|f| f.info.size);
    let (focus, distance) = camera.map_or((None, f32::INFINITY), |c| (Some(c.target_focus), c.target_radius));
    let request = build_request(&ui, &map, size, focus, distance);
    if state.sent.as_ref() != Some(&request) {
        runner.0.send(simulation_core::Control::SetViewRequest(request.clone()));
        state.sent = Some(request);
    }
}

/// Sun, moon, sky, fog, plant season and wind from the latest frame.
#[allow(clippy::type_complexity)]
pub fn update_sky(
    data: Res<EcoData>,
    mut state: ResMut<SceneState>,
    mut season: ResMut<PlantSeason>,
    mut wind: ResMut<Wind>,
    mut particle_wind: ResMut<ParticleWind>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform, &mut CascadeShadowConfig), With<EcoSun>>,
    mut camera: Query<(&mut Camera, &mut AmbientLight, &mut DistanceFog, &PanOrbitCamera), With<EcoCamera>>,
) {
    let Some(frame) = data.get() else { return };
    let view = &frame.view;
    let weather = &view.world.weather;
    let sky = sky_state(view.world.daylight, view.hour, weather.cloud_cover);
    state.sky = sky.sky;
    state.daylight = view.world.daylight;
    state.light_direction = sky.light_direction;
    let distance = camera.iter().next().map_or(300.0, |c| c.3.radius.unwrap_or(300.0));
    for (mut light, mut transform, mut shadow) in &mut sun {
        let rotation = Transform::IDENTITY.looking_to(-sky.light_direction, Vec3::Y).rotation;
        if transform.rotation.angle_between(rotation) > 1e-3 {
            transform.rotation = rotation;
        }
        let color = Color::srgb(sky.light_color[0], sky.light_color[1], sky.light_color[2]);
        if (light.illuminance - sky.illuminance).abs() > 1.0 || light.color != color {
            light.illuminance = sky.illuminance;
            light.color = color;
        }
        let wanted = cascades(distance);
        if (shadow.bounds.last().copied().unwrap_or(0.0) - wanted.bounds.last().copied().unwrap_or(0.0)).abs()
            > 0.15 * wanted.bounds.last().copied().unwrap_or(1.0)
        {
            *shadow = wanted;
        }
    }
    let sky_color = Color::srgb(sky.sky[0], sky.sky[1], sky.sky[2]);
    for (mut cam, mut ambient, mut fog, pan) in &mut camera {
        if !matches!(cam.clear_color, ClearColorConfig::Custom(c) if c == sky_color) {
            cam.clear_color = ClearColorConfig::Custom(sky_color);
        }
        let amb = Color::srgb(sky.ambient_color[0], sky.ambient_color[1], sky.ambient_color[2]);
        if (ambient.brightness - sky.ambient).abs() > 0.5 || ambient.color != amb {
            ambient.brightness = sky.ambient;
            ambient.color = amb;
        }
        if fog.color != sky_color {
            fog.color = sky_color;
        }
        // Haze grows with the distance to the focus, so the map edge fades
        // into the sky from far away while close-ups stay clear.
        let d = pan.radius.unwrap_or(300.0);
        let falloff = FogFalloff::Linear { start: d * 1.6 + 600.0, end: d * 4.5 + 2_500.0 };
        if let (FogFalloff::Linear { start, end }, FogFalloff::Linear { start: s, end: e }) = (&fog.falloff, &falloff) {
            if (start - s).abs() > 10.0 || (end - e).abs() > 10.0 {
                fog.falloff = falloff;
            }
        } else {
            fog.falloff = falloff;
        }
    }
    // Plant season, quantised to half a day of a 120-day year.
    let fraction = (view.time.year_fraction as f32 * 240.0).round() / 240.0;
    if (season.year_fraction - fraction.rem_euclid(1.0)).abs() > 1e-4 {
        *season = PlantSeason::new(fraction);
    }
    let w = weather.wind;
    let speed = w.length();
    let new_wind = Wind { direction: if speed > 1e-3 { w / speed } else { Vec2::X }, speed };
    if *wind != new_wind {
        *wind = new_wind;
    }
    let pw = ParticleWind(Vec3::new(w.x, 0.0, w.y));
    if *particle_wind != pw {
        *particle_wind = pw;
    }
}

/// Share of the view (in normalised device coordinates) the framed map
/// may fill.
pub const FRAMING_MARGIN: f32 = 0.94;

/// Normalised device coordinates of the scene points `(x, z)` (at the
/// focus height) for a camera looking at `(0, 0, dz)` from the south (yaw 0)
/// at `pitch` and `distance`; `None` for a point behind the camera.
fn project(points: &[(f32, f32)], half_v: f32, aspect: f32, pitch: f32, dz: f32, distance: f32) -> Vec<Option<Vec2>> {
    let back = Vec3::new(0.0, pitch.sin(), pitch.cos());
    let forward = -back;
    let up = Vec3::X.cross(forward);
    let eye = Vec3::new(0.0, 0.0, dz) + back * distance;
    points
        .iter()
        .map(|&(x, z)| {
            let rel = Vec3::new(x, 0.0, z) - eye;
            let depth = rel.dot(forward);
            (depth > 0.0).then(|| Vec2::new(rel.x / (depth * half_v * aspect), rel.dot(up) / (depth * half_v)))
        })
        .collect()
}

/// Whether every corner of a `width × depth` map (centred on the origin)
/// lies within [`FRAMING_MARGIN`] of the view.
fn map_fits(extent: Vec2, half_v: f32, aspect: f32, pitch: f32, dz: f32, distance: f32) -> bool {
    let (w, d) = (0.5 * extent.x, 0.5 * extent.y);
    project(&[(-w, -d), (w, -d), (-w, d), (w, d)], half_v, aspect, pitch, dz, distance)
        .into_iter()
        .all(|p| p.is_some_and(|p| p.x.abs() <= FRAMING_MARGIN && p.y.abs() <= FRAMING_MARGIN))
}

/// Focus shift (towards the viewer, m) that centres the map vertically in
/// the view: its far and near edges equally far from the top and bottom.
fn centring_shift(extent: Vec2, half_v: f32, pitch: f32, distance: f32) -> f32 {
    let d = 0.5 * extent.y;
    let balance = |dz: f32| match project(&[(0.0, -d), (0.0, d)], half_v, 1.0, pitch, dz, distance)[..] {
        [Some(far), Some(near)] => far.y + near.y,
        // The near edge is behind the eye: shift the map away.
        _ => f32::NEG_INFINITY,
    };
    // Shifting the focus towards the viewer moves the map up the view.
    let (mut lo, mut hi) = (-d, d);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if balance(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// How to frame a `width × depth` map seen from the south at `pitch`: the
/// shift of the focus from the map centre towards the viewer (+z, m) that
/// centres the map vertically (perspective makes the near half look larger
/// than the far half), and the smallest camera distance (m) at which all
/// four corners fit the view.
pub fn map_framing(extent: Vec2, fov: f32, aspect: f32, pitch: f32) -> (f32, f32) {
    let half_v = (fov * 0.5).tan();
    let aspect = aspect.max(0.2);
    let pitch = pitch.clamp(0.05, std::f32::consts::FRAC_PI_2 - 1e-3);
    let fits = |distance: f32| {
        let dz = centring_shift(extent, half_v, pitch, distance);
        map_fits(extent, half_v, aspect, pitch, dz, distance)
    };
    let (mut lo, mut hi) = (1.0f32, 1.0e6f32);
    for _ in 0..48 {
        let mid = (lo * hi).sqrt();
        if fits(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    (centring_shift(extent, half_v, pitch, hi), hi)
}

/// What the camera driver reads and writes.
type CameraRig = (
    Entity,
    &'static mut OrbitCameraRig,
    &'static mut PanOrbitCamera,
    &'static Projection,
    &'static Camera,
    Option<&'static rendering::FollowTarget>,
);

/// Applies the camera mode, framing, focus requests, following and the
/// cinematic orbit.
#[allow(clippy::too_many_arguments)]
pub fn drive_camera(
    mut commands: Commands,
    mut ui: ResMut<EcoUi>,
    mut state: ResMut<SceneState>,
    map: Res<terrain::MapState>,
    time: Res<Time<Real>>,
    input: Res<WorldInput>,
    animals: Res<fauna::AnimalIndex>,
    mut cameras: Query<CameraRig, With<EcoCamera>>,
) {
    let Some(geometry) = map.geometry.as_ref() else { return };
    for (entity, mut rig, mut cam, projection, camera, follow) in &mut cameras {
        // Framing the whole map is an overview: it leaves ground-level and
        // following views for the orbit.
        if ui.frame_map && matches!(ui.camera, CameraMode::Ground | CameraMode::Follow) {
            ui.camera = CameraMode::Orbit;
        }
        let mode = ui.camera;
        let followed = match (mode, ui.selection) {
            (CameraMode::Follow | CameraMode::Cinematic, Some(ecosystem::OrganismRef::Animal(id))) => {
                animals.entity(id)
            }
            _ => None,
        };
        let preset = if mode == CameraMode::TopDown { OrbitPreset::Aerial } else { OrbitPreset::FreeOrbit };
        if rig.preset != preset {
            rig.preset = preset;
        }
        if state.applied_mode != Some(mode) && cam.initialized {
            state.applied_mode = Some(mode);
            match mode {
                CameraMode::Ground => {
                    cam.target_pitch = 0.12;
                    cam.target_radius = cam.target_radius.min(70.0);
                    // The overview's focus floats at the map's mean height;
                    // a ground-level view looks at the ground itself.
                    let f = cam.target_focus;
                    cam.target_focus = ground_focus(geometry, f);
                }
                CameraMode::Orbit => cam.target_pitch = cam.target_pitch.clamp(0.25, 1.2),
                CameraMode::Cinematic => {
                    cam.target_pitch = 0.32;
                    state.cinematic_yaw = 0.0;
                }
                CameraMode::Follow => {
                    cam.target_pitch = 0.38;
                    cam.target_radius = cam.target_radius.min(14.0);
                }
                CameraMode::TopDown => {}
            }
        }
        match (followed, follow) {
            (Some(target), Some(f)) if f.subject == rendering::FollowSubject::Entity(target) => {}
            (Some(target), _) => {
                commands.entity(entity).insert(rendering::FollowTarget {
                    offset: Vec3::Y * 0.6,
                    half_life: FOLLOW_HALF_LIFE,
                    ..rendering::FollowTarget::entity(target)
                });
            }
            (None, Some(_)) => {
                commands.entity(entity).remove::<rendering::FollowTarget>();
            }
            (None, None) => {}
        }
        if mode == CameraMode::Cinematic && cam.initialized {
            let dt = time.delta_secs();
            cam.target_yaw += 0.07 * dt;
            state.cinematic_yaw += 0.07 * dt;
            // A gentle breathing of the height keeps the shot alive.
            cam.target_pitch = 0.3 + 0.08 * (state.cinematic_yaw * 0.7).sin();
        }
        if let Some(point) = ui.focus.take() {
            let distance = cam.target_radius.min(120.0);
            rig.focus_on(&mut cam, projection, point, distance);
        }
        // Framing waits for a stable viewport (the shell settles its panels
        // over the first frames) and for PanOrbitCamera's initialisation.
        if ui.frame_map {
            let size = camera.viewport.as_ref().map(|v| v.physical_size);
            state.frame_wait += 1;
            if size != state.framed_viewport {
                state.framed_viewport = size;
                state.frame_stable = 0;
            } else {
                state.frame_stable += 1;
            }
            if cam.initialized {
                let aspect = camera.logical_viewport_size().map_or(1.5, |s| s.x / s.y.max(1.0));
                let pitch = if mode == CameraMode::TopDown { std::f32::consts::FRAC_PI_2 } else { 0.72 };
                let (dz, distance) = map_framing(geometry.extent(), rig.fov, aspect, pitch);
                let centre = geometry.centre() + Vec3::new(0.0, mean_height(geometry), dz);
                rig.focus_on(&mut cam, projection, centre, distance);
                // The map is framed from the south (north up).
                cam.target_yaw = 0.0;
                if mode != CameraMode::TopDown {
                    cam.target_pitch = pitch;
                }
                if (size.is_some() && state.frame_stable >= 8) || state.frame_wait > 120 {
                    ui.frame_map = false;
                    state.frame_wait = 0;
                }
            }
        }
        // Keep the eye above the ground and the water surface (trees the
        // eye would sit in, or that hide the focus, are hidden instead:
        // `flora::hide_occluding_trees`).
        // The lift sits on top of the pitch the user (or the camera mode)
        // asked for and is undone once the hill is out of the way, so a
        // ground-level view comes back down after passing a ridge.
        if cam.initialized && mode != CameraMode::TopDown {
            if state.pitch_set != Some(cam.target_pitch) {
                state.pitch_wanted = cam.target_pitch;
            }
            let lifted = clear_pitch(geometry, cam.target_focus, cam.target_yaw, state.pitch_wanted, cam.target_radius);
            if (lifted - cam.target_pitch).abs() > 1e-4 {
                cam.target_pitch = lifted;
            }
            state.pitch_set = Some(cam.target_pitch);
        }
        // Keep the focus over the map and on the ground.
        let e = geometry.extent();
        let f = cam.target_focus;
        let clamped = Vec3::new(f.x.clamp(0.0, e.x), f.y, f.z.clamp(0.0, e.y));
        if followed.is_none() && !ui.frame_map && input.pointer && clamped != f {
            cam.target_focus = clamped;
        }
    }
}

/// A focus moved onto the visible surface (ground or water) under it,
/// clamped to the map.
pub fn ground_focus(g: &crate::coords::MapGeometry, focus: Vec3) -> Vec3 {
    let e = g.extent();
    let (x, z) = (focus.x.clamp(0.0, e.x), focus.z.clamp(0.0, e.y));
    Vec3::new(x, g.surface(x, z), z)
}

/// Eye position of an orbit camera (`bevy_panorbit_camera` places it at
/// `R_y(yaw) R_x(−pitch) (0, 0, r)` from the focus).
pub fn orbit_eye(focus: Vec3, yaw: f32, pitch: f32, radius: f32) -> Vec3 {
    focus + Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()) * radius
}

/// Smallest pitch (at least `pitch`) that keeps the eye clear of the
/// terrain and water by `1.5 m + 3 %` of the distance.
pub fn clear_pitch(g: &crate::coords::MapGeometry, focus: Vec3, yaw: f32, pitch: f32, radius: f32) -> f32 {
    let clearance = 1.5 + 0.03 * radius;
    let mut p = pitch;
    while p < 1.45 {
        let eye = orbit_eye(focus, yaw, p, radius);
        if eye.y >= g.surface(eye.x, eye.z) + clearance {
            break;
        }
        p += 0.01;
    }
    p.min(1.45).max(pitch)
}

/// Mean height of the visible surface (ground or water): the sea floor
/// around an oceanic island lies 1.5 km below the waves the camera frames.
pub fn mean_height(g: &crate::coords::MapGeometry) -> f32 {
    let n = g.water_level.len();
    if n == 0 { 0.0 } else { g.water_level.iter().sum::<f32>() / n as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_fits_and_centres_the_whole_map() {
        let extent = Vec2::new(2000.0, 2000.0);
        let half_v = (FRAC_PI_4 * 0.5).tan();
        let (dz, d) = map_framing(extent, FRAC_PI_4, 1.6, 0.72);
        assert!(map_fits(extent, half_v, 1.6, 0.72, dz, d), "all four corners in view");
        assert!(!map_fits(extent, half_v, 1.6, 0.72, dz, d * 0.97), "and no farther than needed");
        // Centred vertically: the far and near edges are equally far from the borders.
        let ys = project(&[(0.0, -1000.0), (0.0, 1000.0)], half_v, 1.6, 0.72, dz, d);
        let (far, near) = (ys[0].unwrap().y, ys[1].unwrap().y);
        assert!((far + near).abs() < 1e-3, "{far} {near}");
        // Perspective: the near half looks larger, so the focus (the view's
        // centre) moves towards the viewer, into the near half.
        assert!(dz > 0.0, "{dz}");
        let (_, narrow) = map_framing(extent, FRAC_PI_4, 0.8, 0.72);
        assert!(narrow > d, "narrow viewports need more distance");
        // From straight above the map is centred.
        let (dz, d) = map_framing(Vec2::new(1000.0, 500.0), FRAC_PI_4, 2.5, std::f32::consts::FRAC_PI_2);
        assert!(dz.abs() < 1.0, "{dz}");
        assert!((d - 500.0 / (2.0 * half_v * FRAMING_MARGIN)).abs() < 2.0, "{d}");
    }

    #[test]
    fn the_eye_stays_above_hills_and_water() {
        // A ridge 40 m high 30 m east of the focus.
        let elevation: Vec<f32> = (0..100).map(|i| if i % 10 >= 7 { 40.0 } else { 0.0 }).collect();
        let g = crate::coords::MapGeometry::new(10, 10, 1, 10.0, elevation);
        let focus = Vec3::new(45.0, 0.0, 45.0);
        let yaw = std::f32::consts::FRAC_PI_2; // looking from +x
        let p = clear_pitch(&g, focus, yaw, 0.1, 40.0);
        let eye = orbit_eye(focus, yaw, p, 40.0);
        assert!(eye.y >= g.surface(eye.x, eye.z) + 1.5, "{eye:?} at pitch {p}");
        assert!(p > 0.1);
        // Over open flat ground nothing changes: a ground-level view stays low.
        assert_eq!(clear_pitch(&g, focus, -yaw, 0.12, 20.0), 0.12);
        // Water counts as surface.
        let mut wet = g.clone();
        wet.set_water(&[3.0; 100]);
        let p = clear_pitch(&wet, focus, -yaw, 0.12, 20.0);
        assert!(orbit_eye(focus, -yaw, p, 20.0).y >= 3.0 + 1.5 + 0.6 - 1e-3, "pitch {p}");
        // Ground level looks at the ground under a floating focus.
        let p = ground_focus(&g, Vec3::new(45.0, 300.0, 45.0));
        assert_eq!(p, Vec3::new(45.0, g.surface(45.0, 45.0), 45.0));
        assert_eq!(ground_focus(&g, Vec3::new(-5.0, 0.0, 500.0)).x, 0.0);
        // Framing centres on the water surface, not on the sea floor.
        let mut sea = crate::coords::MapGeometry::new(2, 1, 1, 10.0, vec![-1500.0, 10.0]);
        assert!((mean_height(&sea) + 745.0).abs() < 1e-3);
        sea.set_water(&[1500.5, 0.0]);
        assert!((mean_height(&sea) - 5.25).abs() < 1e-3);
        // The eye of panorbit's convention: yaw 0, pitch 0 is on +z.
        assert!((orbit_eye(Vec3::ZERO, 0.0, 0.0, 5.0) - Vec3::new(0.0, 0.0, 5.0)).length() < 1e-6);
    }

    #[test]
    fn requests_follow_the_overlay_and_camera() {
        let ui = EcoUi::default();
        let map = terrain::MapState::default();
        let r = build_request(&ui, &map, Some((100, 100)), None, 100.0);
        assert_eq!(r.view.block, 1);
        assert!(r.markers.is_none(), "no focus yet");
        assert!(!r.view.field_markers, "markers come from the session's regional request");
        assert!(!r.territories);
        let fire = EcoUi { overlay: Overlay::FireRisk, ..EcoUi::default() };
        let r = build_request(&fire, &map, Some((1024, 512)), None, 100.0);
        assert_eq!(r.view.block, 4);
        assert!(r.view.world.layers.contains(&WorldLayer::Fuel));
        let territory = EcoUi { overlay: Overlay::Territories, ..EcoUi::default() };
        assert!(build_request(&territory, &map, None, None, 100.0).territories);
        // Water plants are only sent for the biomass overlay.
        assert!(!r.view.vegetation.contains(&PlantLayer::Algae));
        let biomass = EcoUi { overlay: Overlay::PlantBiomass, ..EcoUi::default() };
        assert_eq!(build_request(&biomass, &map, None, None, 100.0).view.vegetation.len(), PlantLayer::ALL.len());
    }
}
