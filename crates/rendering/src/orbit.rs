//! 3D world cameras built on `bevy_panorbit_camera` 0.36.
//!
//! [`OrbitCameraRig`] adds presets on top of `PanOrbitCamera`:
//!
//! | Preset | Projection | Pitch | Yaw | Buttons |
//! |---|---|---|---|---|
//! | Aerial | perspective | locked top-down | free | orbit left, pan right |
//! | Isometric | orthographic | locked at atan(1/√2) ≈ 35.26° | locked, Q/E rotate 90° | pan right |
//! | Free orbit | perspective | free (no flipping) | free | orbit left, pan right |
//! | Strategic | perspective | 34°–72° | free, Q/E rotate | orbit middle, pan right (left stays free for selection) |
//!
//! Every preset pans with WASD / arrow keys relative to the view direction and,
//! when [`OrbitCameraRig::edge_pan`] is set, when the cursor rests near a
//! viewport edge. Input is gated by [`WorldInput`]: `PanOrbitCamera::enabled`
//! follows `WorldInput::pointer` (a drag that started over the world
//! continues), and keys require `WorldInput::keyboard` and are ignored while
//! Ctrl, Cmd or Alt is held ([`crate::shortcut_modifier_held`]).
//!
//! For the orthographic preset `PanOrbitCamera` interprets its radius as the
//! projection scale; the rig uses `ScalingMode::FixedVertical { 1.0 }`, so the
//! scale equals the visible height in world units, and converts distances with
//! [`distance_to_ortho_scale`].

use bevy::camera::ScalingMode;
use bevy::prelude::*;
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraPlugin};
use serde::{Deserialize, Serialize};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use crate::{WorldCamera, WorldInput, shortcut_modifier_held};

/// Pitch used for the top-down aerial preset (just below 90° so yaw stays defined).
pub const AERIAL_PITCH: f32 = FRAC_PI_2 - 1e-3;
/// True isometric elevation: `atan(1/√2)`.
pub const ISOMETRIC_PITCH: f32 = 0.615_479_7;
/// Far plane of the orthographic preset (the camera sits at half this distance).
const ORTHO_FAR: f32 = 4000.0;

/// Registers `PanOrbitCameraPlugin` (if missing) and the rig systems.
pub struct OrbitCameraPlugin;

/// System set of the rig systems (in `Update`, before `PanOrbitCamera` runs in `PostUpdate`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrbitCameraSystems;

impl Plugin for OrbitCameraPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<PanOrbitCameraPlugin>() {
            app.add_plugins(PanOrbitCameraPlugin);
        }
        app.init_resource::<WorldInput>().add_systems(
            Update,
            (apply_orbit_presets, gate_orbit_input, keyboard_and_edge_controls, follow_targets)
                .chain()
                .in_set(OrbitCameraSystems),
        );
    }
}

/// Camera behaviour presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrbitPreset {
    /// Top-down view.
    Aerial,
    /// Orthographic true-isometric view.
    Isometric,
    /// Unrestricted orbit.
    #[default]
    FreeOrbit,
    /// RTS-like tilted view.
    Strategic,
}

/// Static parameters of a preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresetConfig {
    /// Initial pitch (radians, positive = camera above the focus).
    pub pitch: f32,
    /// Pitch limits (lower, upper).
    pub pitch_limits: (Option<f32>, Option<f32>),
    /// Fixed yaw, if the preset locks it.
    pub yaw: Option<f32>,
    /// Whether the preset uses an orthographic projection.
    pub orthographic: bool,
    /// Mouse button that orbits.
    pub orbit_button: MouseButton,
    /// Mouse button that pans.
    pub pan_button: MouseButton,
    /// Q/E rotation step in radians (`None` = continuous).
    pub rotate_step: Option<f32>,
}

impl OrbitPreset {
    /// Every preset, in display order.
    pub const ALL: [OrbitPreset; 4] = [Self::Aerial, Self::Isometric, Self::FreeOrbit, Self::Strategic];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Aerial => "Aerial",
            Self::Isometric => "Isometric",
            Self::FreeOrbit => "Free orbit",
            Self::Strategic => "Strategic",
        }
    }

    /// The preset's parameters.
    pub fn config(self) -> PresetConfig {
        match self {
            Self::Aerial => PresetConfig {
                pitch: AERIAL_PITCH,
                pitch_limits: (Some(AERIAL_PITCH), Some(AERIAL_PITCH)),
                yaw: None,
                orthographic: false,
                orbit_button: MouseButton::Left,
                pan_button: MouseButton::Right,
                rotate_step: None,
            },
            Self::Isometric => PresetConfig {
                pitch: ISOMETRIC_PITCH,
                pitch_limits: (Some(ISOMETRIC_PITCH), Some(ISOMETRIC_PITCH)),
                yaw: Some(FRAC_PI_4),
                orthographic: true,
                orbit_button: MouseButton::Left,
                pan_button: MouseButton::Right,
                rotate_step: Some(FRAC_PI_2),
            },
            Self::FreeOrbit => PresetConfig {
                pitch: 0.5,
                pitch_limits: (None, None),
                yaw: None,
                orthographic: false,
                orbit_button: MouseButton::Left,
                pan_button: MouseButton::Right,
                rotate_step: None,
            },
            Self::Strategic => PresetConfig {
                pitch: 0.96,
                pitch_limits: (Some(0.6), Some(1.25)),
                yaw: None,
                orthographic: false,
                orbit_button: MouseButton::Middle,
                pan_button: MouseButton::Right,
                rotate_step: None,
            },
        }
    }
}

/// A preset-driven world camera. Spawning it adds `PanOrbitCamera` (and so
/// `Camera3d`) and [`WorldCamera`]. Change `preset` at any time; the camera
/// eases to the new pose.
#[derive(Component, Debug, Clone)]
#[require(PanOrbitCamera, WorldCamera)]
pub struct OrbitCameraRig {
    /// Active preset.
    pub preset: OrbitPreset,
    /// Closest allowed distance to the focus (world units).
    pub min_distance: f32,
    /// Farthest allowed distance to the focus (world units).
    pub max_distance: f32,
    /// Keyboard / edge pan speed as a fraction of the distance per second.
    pub pan_speed: f32,
    /// Q/E continuous rotation speed in radians per second.
    pub rotate_speed: f32,
    /// Pan when the cursor rests within [`Self::edge_margin_px`] of the viewport edge.
    pub edge_pan: bool,
    /// Edge-pan band width in logical pixels.
    pub edge_margin_px: f32,
    /// Vertical field of view used for perspective presets (radians).
    pub fov: f32,
    applied: Option<OrbitPreset>,
    dragging: bool,
}

impl Default for OrbitCameraRig {
    fn default() -> Self {
        Self::new(OrbitPreset::FreeOrbit)
    }
}

impl OrbitCameraRig {
    /// A rig with the given preset and default limits.
    pub fn new(preset: OrbitPreset) -> Self {
        Self {
            preset,
            min_distance: 2.0,
            max_distance: 2000.0,
            pan_speed: 0.8,
            rotate_speed: 1.5,
            edge_pan: false,
            edge_margin_px: 14.0,
            fov: FRAC_PI_4,
            applied: None,
            dragging: false,
        }
    }

    /// Sets the allowed distance range (world units).
    pub fn with_distance_limits(mut self, min_distance: f32, max_distance: f32) -> Self {
        self.min_distance = min_distance;
        self.max_distance = max_distance;
        self
    }

    /// Enables or disables edge panning.
    pub fn with_edge_pan(mut self, enabled: bool) -> Self {
        self.edge_pan = enabled;
        self
    }

    fn fov(&self) -> f32 {
        if self.fov.is_finite() { self.fov.clamp(0.05, 3.0) } else { FRAC_PI_4 }
    }

    /// Moves the rig's camera focus to `point` at `distance` world units,
    /// converting the distance for orthographic presets with the rig's own
    /// field of view (so the framing matches the perspective presets). The
    /// camera eases there.
    pub fn focus_on(&self, camera: &mut PanOrbitCamera, projection: &Projection, point: Vec3, distance: f32) {
        focus_with_fov(camera, projection, point, distance, self.fov());
    }
}

/// Orthographic scale (visible height with `FixedVertical { 1.0 }`) matching a
/// perspective view at `distance` with vertical field of view `fov`.
pub fn distance_to_ortho_scale(distance: f32, fov: f32) -> f32 {
    2.0 * distance * (fov * 0.5).tan()
}

/// Inverse of [`distance_to_ortho_scale`].
pub fn ortho_scale_to_distance(scale: f32, fov: f32) -> f32 {
    scale / (2.0 * (fov * 0.5).tan())
}

/// Moves the camera's focus to `point` at `distance` world units. For
/// orthographic projections the distance is converted to a scale with a 45°
/// reference field of view (the rig default); use
/// [`OrbitCameraRig::focus_on`] to honour a rig's own field of view. The
/// camera eases there.
pub fn focus_on(camera: &mut PanOrbitCamera, projection: &Projection, point: Vec3, distance: f32) {
    focus_with_fov(camera, projection, point, distance, FRAC_PI_4);
}

fn focus_with_fov(camera: &mut PanOrbitCamera, projection: &Projection, point: Vec3, distance: f32, fov: f32) {
    camera.target_focus = point;
    if !distance.is_finite() {
        // Keep the current zoom; only the focus moves.
        return;
    }
    let distance = distance.max(0.0);
    camera.target_radius = match projection {
        Projection::Orthographic(_) => distance_to_ortho_scale(distance, fov),
        _ => distance,
    }
    .max(camera.zoom_lower_limit);
}

/// Unit vectors on the ground plane for a camera yaw: (forward, right).
pub fn ground_axes(yaw: f32) -> (Vec3, Vec3) {
    let forward = -Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    (forward, right)
}

fn apply_orbit_presets(mut rigs: Query<(&mut OrbitCameraRig, &mut PanOrbitCamera, &mut Projection, &Transform)>) {
    for (mut rig, mut cam, mut projection, transform) in &mut rigs {
        let fov = rig.fov();
        let cfg = rig.preset.config();
        let was_ortho = matches!(*projection, Projection::Orthographic(_));
        let to_units = |d: f32| if cfg.orthographic { distance_to_ortho_scale(d, fov) } else { d };
        let (lo, hi) = (rig.min_distance.max(0.01), rig.max_distance.max(rig.min_distance.max(0.01)));
        let (zoom_lo, zoom_hi) = (to_units(lo), Some(to_units(hi)));
        // Keep the perspective field of view in sync with the rig (checked
        // without a mutable borrow so the projection is not marked changed).
        let stale_fov = !cfg.orthographic && matches!(&*projection, Projection::Perspective(p) if p.fov != fov);
        if stale_fov && let Projection::Perspective(p) = &mut *projection {
            p.fov = fov;
        }
        if rig.applied == Some(rig.preset) {
            if cam.zoom_lower_limit != zoom_lo || cam.zoom_upper_limit != zoom_hi {
                cam.zoom_lower_limit = zoom_lo;
                cam.zoom_upper_limit = zoom_hi;
            }
            continue;
        }
        // Current distance in world units, whatever the projection was.
        let current = if cam.initialized {
            if was_ortho { ortho_scale_to_distance(cam.target_radius, fov) } else { cam.target_radius }
        } else {
            (transform.translation - cam.focus).length()
        };
        let distance = current.clamp(lo, hi);
        if cfg.orthographic != was_ortho {
            *projection = if cfg.orthographic {
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::FixedVertical { viewport_height: 1.0 },
                    far: ORTHO_FAR,
                    ..OrthographicProjection::default_3d()
                })
            } else {
                Projection::Perspective(PerspectiveProjection { fov, ..default() })
            };
        }
        cam.zoom_lower_limit = zoom_lo;
        cam.zoom_upper_limit = zoom_hi;
        cam.pitch_lower_limit = cfg.pitch_limits.0;
        cam.pitch_upper_limit = cfg.pitch_limits.1;
        cam.yaw_lower_limit = cfg.yaw;
        cam.yaw_upper_limit = cfg.yaw;
        cam.button_orbit = cfg.orbit_button;
        cam.button_pan = cfg.pan_button;
        let units = to_units(distance);
        if cam.initialized {
            cam.target_pitch = cfg.pitch;
            if let Some(yaw) = cfg.yaw {
                cam.target_yaw = yaw;
            }
            cam.target_radius = units;
            if cfg.orthographic != was_ortho {
                // Units changed: jump the current radius instead of easing across units.
                cam.radius = Some(units);
            }
            cam.force_update = true;
        } else {
            cam.pitch = Some(cfg.pitch);
            if let Some(yaw) = cfg.yaw {
                cam.yaw = Some(yaw);
            }
            cam.radius = Some(units);
        }
        rig.applied = Some(rig.preset);
    }
}

fn gate_orbit_input(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut rigs: Query<(&mut OrbitCameraRig, &mut PanOrbitCamera)>,
) {
    for (mut rig, mut cam) in &mut rigs {
        let watched = [cam.button_orbit, cam.button_pan];
        if buttons.any_just_pressed(watched) {
            rig.dragging = input.pointer;
        }
        if !buttons.any_pressed(watched) {
            rig.dragging = false;
        }
        let allow = input.pointer || rig.dragging;
        if cam.enabled != allow {
            cam.enabled = allow;
        }
    }
}

const PAN_KEYS: [(KeyCode, Vec2); 8] = [
    (KeyCode::KeyW, Vec2::Y),
    (KeyCode::ArrowUp, Vec2::Y),
    (KeyCode::KeyS, Vec2::NEG_Y),
    (KeyCode::ArrowDown, Vec2::NEG_Y),
    (KeyCode::KeyD, Vec2::X),
    (KeyCode::ArrowRight, Vec2::X),
    (KeyCode::KeyA, Vec2::NEG_X),
    (KeyCode::ArrowLeft, Vec2::NEG_X),
];

/// Screen-space pan direction (x right, y up) for a cursor near the viewport
/// edges, or zero.
pub fn edge_pan_direction(cursor_in_viewport: Vec2, viewport_size: Vec2, margin: f32) -> Vec2 {
    let mut dir = Vec2::ZERO;
    if margin <= 0.0 {
        return dir;
    }
    if cursor_in_viewport.x < margin {
        dir.x -= 1.0;
    } else if cursor_in_viewport.x > viewport_size.x - margin {
        dir.x += 1.0;
    }
    if cursor_in_viewport.y < margin {
        dir.y += 1.0;
    } else if cursor_in_viewport.y > viewport_size.y - margin {
        dir.y -= 1.0;
    }
    dir
}

fn keyboard_and_edge_controls(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut rigs: Query<(&OrbitCameraRig, &mut PanOrbitCamera)>,
) {
    let dt = time.delta_secs();
    for (rig, mut cam) in &mut rigs {
        let mut screen_dir = Vec2::ZERO;
        let mut rotate = 0.0;
        if input.keyboard && !shortcut_modifier_held(&keys) {
            screen_dir += PAN_KEYS.iter().filter(|(k, _)| keys.pressed(*k)).map(|(_, d)| *d).sum::<Vec2>();
            let step = rig.preset.config().rotate_step;
            match step {
                Some(step) => {
                    if keys.just_pressed(KeyCode::KeyQ) {
                        rotate -= step;
                    }
                    if keys.just_pressed(KeyCode::KeyE) {
                        rotate += step;
                    }
                }
                None => {
                    if keys.pressed(KeyCode::KeyQ) {
                        rotate -= rig.rotate_speed * dt;
                    }
                    if keys.pressed(KeyCode::KeyE) {
                        rotate += rig.rotate_speed * dt;
                    }
                }
            }
        }
        if rig.edge_pan
            && input.pointer
            && let (Some(cursor), Some(viewport)) = (input.cursor_in_viewport(), input.viewport)
        {
            screen_dir += edge_pan_direction(cursor, viewport.size(), rig.edge_margin_px);
        }
        if rotate != 0.0 {
            cam.target_yaw += rotate;
            if rig.preset.config().yaw.is_some() {
                // Locked-yaw presets rotate by moving the lock.
                cam.yaw_lower_limit = Some(cam.target_yaw);
                cam.yaw_upper_limit = Some(cam.target_yaw);
            }
        }
        if screen_dir != Vec2::ZERO {
            let (forward, right) = ground_axes(cam.target_yaw);
            let dir = (forward * screen_dir.y + right * screen_dir.x).normalize_or_zero();
            let distance = cam.target_radius.max(0.01);
            cam.target_focus += dir * rig.pan_speed * distance * dt;
        }
    }
}

/// Makes a camera smoothly track an entity or a fixed world position.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct FollowTarget {
    /// What to follow.
    pub subject: FollowSubject,
    /// Offset added to the subject's position.
    pub offset: Vec3,
    /// Time for the focus to cover half the remaining distance (seconds).
    pub half_life: f32,
}

impl FollowTarget {
    /// Follow an entity's global position.
    pub fn entity(entity: Entity) -> Self {
        Self { subject: FollowSubject::Entity(entity), offset: Vec3::ZERO, half_life: 0.25 }
    }

    /// Follow (glide to) a world position.
    pub fn point(point: Vec3) -> Self {
        Self { subject: FollowSubject::Point(point), offset: Vec3::ZERO, half_life: 0.25 }
    }
}

/// Subject of a [`FollowTarget`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FollowSubject {
    /// An entity with a `GlobalTransform`; following stops if it despawns.
    Entity(Entity),
    /// A fixed world position.
    Point(Vec3),
}

/// Fraction of the remaining distance covered in `dt` for a given half-life.
pub fn half_life_factor(dt: f32, half_life: f32) -> f32 {
    if half_life <= 0.0 || !half_life.is_finite() {
        return 1.0;
    }
    1.0 - 0.5f32.powf(dt.max(0.0) / half_life)
}

fn follow_targets(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut cams: Query<(Entity, &FollowTarget, &mut PanOrbitCamera)>,
    transforms: Query<&GlobalTransform>,
) {
    let dt = time.delta_secs();
    for (entity, follow, mut cam) in &mut cams {
        let subject = match follow.subject {
            FollowSubject::Point(p) => p,
            FollowSubject::Entity(target) => match transforms.get(target) {
                Ok(gt) => gt.translation(),
                Err(_) => {
                    commands.entity(entity).remove::<FollowTarget>();
                    continue;
                }
            },
        };
        let goal = subject + follow.offset;
        let next = cam.target_focus.lerp(goal, half_life_factor(dt, follow.half_life));
        if next != cam.target_focus {
            cam.target_focus = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[test]
    fn ortho_conversion_round_trips() {
        let fov = FRAC_PI_4;
        let s = distance_to_ortho_scale(50.0, fov);
        assert!((s - 2.0 * 50.0 * (fov / 2.0).tan()).abs() < 1e-4);
        assert!((ortho_scale_to_distance(s, fov) - 50.0).abs() < 1e-3);
    }

    #[test]
    fn isometric_pitch_is_true_isometric() {
        // The view direction of a true isometric camera is along (1, 1, 1) up to sign.
        let expected = (1.0f32 / 2.0f32.sqrt()).atan();
        assert!((ISOMETRIC_PITCH - expected).abs() < 1e-6);
        let cfg = OrbitPreset::Isometric.config();
        assert!(cfg.orthographic);
        assert_eq!(cfg.yaw, Some(FRAC_PI_4));
        // With panorbit's convention the camera sits at R_y(yaw) R_x(-pitch) (0, 0, r).
        let rot = Quat::from_axis_angle(Vec3::Y, FRAC_PI_4) * Quat::from_axis_angle(Vec3::X, -ISOMETRIC_PITCH);
        let offset = (rot * Vec3::Z).normalize();
        let iso = Vec3::ONE.normalize();
        assert!((offset - iso).length() < 1e-5, "{offset:?}");
    }

    #[test]
    fn aerial_looks_straight_down() {
        let rot = Quat::from_axis_angle(Vec3::X, -AERIAL_PITCH);
        let offset = rot * Vec3::Z;
        assert!(offset.y > 0.999_999);
    }

    #[test]
    fn ground_axes_match_camera_orientation() {
        // yaw 0: camera on +Z looking toward -Z.
        let (forward, right) = ground_axes(0.0);
        assert!((forward - Vec3::NEG_Z).length() < 1e-6);
        assert!((right - Vec3::X).length() < 1e-6);
        // For any yaw, forward points from the camera position toward the focus.
        for yaw in [0.3f32, 1.7, -2.4] {
            let cam_offset = Quat::from_axis_angle(Vec3::Y, yaw) * Vec3::Z;
            let (forward, right) = ground_axes(yaw);
            assert!((forward + cam_offset).length() < 1e-5);
            assert!(forward.dot(right).abs() < 1e-6);
            // Right-handed frame: up × right = forward.
            assert!((Vec3::Y.cross(right) - forward).length() < 1e-5);
        }
    }

    #[test]
    fn edge_pan_detects_edges() {
        let vp = Vec2::new(800.0, 600.0);
        assert_eq!(edge_pan_direction(Vec2::new(400.0, 300.0), vp, 10.0), Vec2::ZERO);
        assert_eq!(edge_pan_direction(Vec2::new(2.0, 300.0), vp, 10.0), Vec2::NEG_X);
        assert_eq!(edge_pan_direction(Vec2::new(795.0, 2.0), vp, 10.0), Vec2::new(1.0, 1.0));
        assert_eq!(edge_pan_direction(Vec2::new(400.0, 599.0), vp, 10.0), Vec2::NEG_Y);
        assert_eq!(edge_pan_direction(Vec2::new(2.0, 2.0), vp, 0.0), Vec2::ZERO);
    }

    #[test]
    fn half_life_halves() {
        assert!((half_life_factor(0.25, 0.25) - 0.5).abs() < 1e-6);
        assert!((half_life_factor(0.5, 0.25) - 0.75).abs() < 1e-6);
        assert_eq!(half_life_factor(0.1, 0.0), 1.0);
    }

    #[test]
    fn focus_on_converts_for_ortho() {
        let mut cam = PanOrbitCamera::default();
        let ortho = Projection::Orthographic(OrthographicProjection::default_3d());
        focus_on(&mut cam, &ortho, Vec3::new(1.0, 2.0, 3.0), 10.0);
        assert_eq!(cam.target_focus, Vec3::new(1.0, 2.0, 3.0));
        assert!((cam.target_radius - distance_to_ortho_scale(10.0, FRAC_PI_4)).abs() < 1e-5);
        let persp = Projection::Perspective(PerspectiveProjection::default());
        focus_on(&mut cam, &persp, Vec3::ZERO, 10.0);
        assert_eq!(cam.target_radius, 10.0);
        // A rig with a narrow field of view frames the same distance tighter.
        let rig = OrbitCameraRig { fov: 0.5, ..OrbitCameraRig::new(OrbitPreset::Isometric) };
        rig.focus_on(&mut cam, &ortho, Vec3::ONE, 10.0);
        assert!((cam.target_radius - distance_to_ortho_scale(10.0, 0.5)).abs() < 1e-5);
        assert_eq!(cam.target_focus, Vec3::ONE);
        // Non-finite distances keep the current zoom.
        let before = cam.target_radius;
        rig.focus_on(&mut cam, &persp, Vec3::ZERO, f32::NAN);
        assert_eq!(cam.target_radius, before);
    }

    #[test]
    fn preset_switches_convert_projection_and_distance() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_systems(Update, apply_orbit_presets);
        let rig = OrbitCameraRig { fov: 1.0, ..OrbitCameraRig::new(OrbitPreset::FreeOrbit) };
        let cam = app.world_mut().spawn((rig, Transform::from_xyz(0.0, 0.0, 20.0))).id();
        app.update();
        let fov_of = |app: &App| match app.world().get::<Projection>(cam) {
            Some(Projection::Perspective(p)) => Some(p.fov),
            _ => None,
        };
        assert_eq!(fov_of(&app), Some(1.0), "the rig's field of view is applied");
        assert_eq!(app.world().get::<PanOrbitCamera>(cam).and_then(|c| c.radius), Some(20.0));

        // As PanOrbitCamera would after its first update.
        {
            let mut pan = app.world_mut().get_mut::<PanOrbitCamera>(cam).expect("camera");
            pan.initialized = true;
            pan.target_radius = 20.0;
        }
        app.world_mut().get_mut::<OrbitCameraRig>(cam).expect("rig").preset = OrbitPreset::Isometric;
        app.update();
        let pan = *app.world().get::<PanOrbitCamera>(cam).expect("camera");
        let scale = distance_to_ortho_scale(20.0, 1.0);
        assert!(matches!(app.world().get::<Projection>(cam), Some(Projection::Orthographic(_))));
        assert!((pan.target_radius - scale).abs() < 1e-4, "{}", pan.target_radius);
        assert_eq!(pan.radius, Some(pan.target_radius), "units changed: radius jumps");
        assert_eq!((pan.yaw_lower_limit, pan.yaw_upper_limit), (Some(FRAC_PI_4), Some(FRAC_PI_4)));
        assert_eq!(pan.target_pitch, ISOMETRIC_PITCH);

        // Back to perspective: same framing distance, rig field of view.
        app.world_mut().get_mut::<OrbitCameraRig>(cam).expect("rig").preset = OrbitPreset::Strategic;
        app.update();
        let pan = *app.world().get::<PanOrbitCamera>(cam).expect("camera");
        assert_eq!(fov_of(&app), Some(1.0));
        assert!((pan.target_radius - 20.0).abs() < 1e-3, "{}", pan.target_radius);
        assert_eq!(pan.button_orbit, MouseButton::Middle);
        assert_eq!(pan.yaw_lower_limit, None);

        // Changing the field of view later takes effect too.
        app.world_mut().get_mut::<OrbitCameraRig>(cam).expect("rig").fov = 0.7;
        app.update();
        assert_eq!(fov_of(&app), Some(0.7));
    }

    #[test]
    fn shortcut_chords_never_move_the_rig() {
        fn press(app: &mut App, keys: &[KeyCode]) {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.reset_all();
            for key in keys {
                input.press(*key);
            }
            app.update();
        }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(100)))
            .insert_resource(WorldInput { keyboard: true, ..default() })
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, keyboard_and_edge_controls);
        let cam = app.world_mut().spawn(OrbitCameraRig::new(OrbitPreset::Strategic)).id();
        // The first frame has no time delta.
        app.update();
        let pose = |app: &App| {
            let pan = app.world().get::<PanOrbitCamera>(cam).expect("camera");
            (pan.target_focus, pan.target_yaw)
        };
        let start = pose(&app);
        for modifier in [KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::AltLeft] {
            press(&mut app, &[modifier, KeyCode::KeyW, KeyCode::KeyE]);
            assert_eq!(pose(&app), start, "{modifier:?}+W / {modifier:?}+E");
        }
        press(&mut app, &[KeyCode::KeyW, KeyCode::KeyE]);
        let (focus, yaw) = pose(&app);
        assert!(focus != start.0 && yaw > start.1, "W pans and E rotates without modifiers");
    }

    #[test]
    fn presets_have_consistent_limits() {
        for preset in OrbitPreset::ALL {
            let cfg = preset.config();
            if let (Some(lo), Some(hi)) = cfg.pitch_limits {
                assert!(lo <= cfg.pitch && cfg.pitch <= hi, "{preset:?}");
            }
            assert!(!preset.label().is_empty());
        }
    }
}
