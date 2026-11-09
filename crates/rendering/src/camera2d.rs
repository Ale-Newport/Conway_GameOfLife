//! 2D world camera for grids: drag / key panning, smooth exponential zoom
//! towards the cursor, zoom limits, fit-to-rectangle, floating-origin
//! re-anchoring and the per-frame [`VisibleCells`] resource.
//!
//! Input is consumed only when [`WorldInput`] allows it: pointer actions need
//! `WorldInput::pointer`, key actions need `WorldInput::keyboard`. A drag that
//! starts over the world keeps going if the pointer later crosses a panel.
//!
//! The controller requires `ScalingMode::WindowSize` (Bevy's 2D default), so
//! the orthographic `scale` is world units per logical pixel.

use bevy::camera::ScalingMode;
use bevy::input::gestures::PinchGesture;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::math::I64Vec2;
use bevy::prelude::*;

use crate::WorldInput;
use crate::grid::{GridSpace, VisibleCells};

/// Pixel-unit scroll deltas (trackpads) per equivalent wheel line.
const PIXELS_PER_LINE: f32 = 40.0;
/// Re-anchor the floating origin when the camera is this many cells away from it.
const REANCHOR_CELLS: f64 = 4096.0;

/// Registers the 2D camera controller systems and [`VisibleCells`].
pub struct GridCameraPlugin;

/// Ordering of the controller systems in `Update`. Mode systems that read
/// [`VisibleCells`] should run `.after(GridCameraSystems::Visible)`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GridCameraSystems {
    /// Applies input, fit requests and smoothing to the camera transform.
    Control,
    /// Recomputes [`VisibleCells`].
    Visible,
}

impl Plugin for GridCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldInput>()
            .init_resource::<GridSpace>()
            .init_resource::<VisibleCells>()
            .configure_sets(Update, (GridCameraSystems::Control, GridCameraSystems::Visible).chain())
            .add_systems(
                Update,
                (
                    (control_grid_camera, reanchor_grid_space).chain().in_set(GridCameraSystems::Control),
                    update_visible_cells.in_set(GridCameraSystems::Visible),
                ),
            );
    }
}

/// Request to frame a cell rectangle (see [`GridCameraController::fit_to_rect`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct FitRequest {
    min: I64Vec2,
    max: I64Vec2,
    margin: f32,
}

/// Pan/zoom controller for a 2D world camera. Spawning it adds `Camera2d`
/// and [`crate::WorldCamera`].
#[derive(Component, Debug, Clone)]
#[require(Camera2d, crate::WorldCamera)]
pub struct GridCameraController {
    /// Most zoomed-in level, in cells per logical pixel (e.g. 1/128).
    pub min_cells_per_pixel: f32,
    /// Most zoomed-out level, in cells per logical pixel.
    pub max_cells_per_pixel: f32,
    /// Zoom factor per wheel notch (> 1).
    pub zoom_step: f32,
    /// Time constant of the exponential zoom smoothing, in seconds (0 = instant).
    pub zoom_time_constant: f32,
    /// Keyboard pan speed in logical pixels per second.
    pub key_pan_speed: f32,
    /// Keep world coordinates small by moving [`GridSpace::anchor`] with the camera.
    pub auto_reanchor: bool,
    target_scale: Option<f32>,
    zoom_anchor: Option<Vec2>,
    dragging: bool,
    last_cursor: Option<Vec2>,
    pending_fit: Option<FitRequest>,
}

impl Default for GridCameraController {
    fn default() -> Self {
        Self {
            min_cells_per_pixel: 1.0 / 128.0,
            max_cells_per_pixel: 1_048_576.0,
            zoom_step: 1.2,
            zoom_time_constant: 0.07,
            key_pan_speed: 900.0,
            auto_reanchor: true,
            target_scale: None,
            zoom_anchor: None,
            dragging: false,
            last_cursor: None,
            pending_fit: None,
        }
    }
}

impl GridCameraController {
    /// Frames the inclusive cell rectangle `min..=max` (with `margin` as a
    /// fraction of the viewport on each side, clamped to `0..0.45`) on the
    /// next update. The zoom is clamped to the controller's limits.
    pub fn fit_to_rect(&mut self, min: I64Vec2, max: I64Vec2, margin: f32) {
        let margin = if margin.is_finite() { margin.clamp(0.0, 0.45) } else { 0.0 };
        self.pending_fit = Some(FitRequest { min: min.min(max), max: min.max(max), margin });
    }

    /// Whether a right/middle-button drag is in progress.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    fn scale_limits(&self, cell_size: f32) -> (f32, f32) {
        let cell_size = if cell_size.is_finite() && cell_size > 0.0 { cell_size } else { 1.0 };
        let lo = self.min_cells_per_pixel.max(f32::MIN_POSITIVE) * cell_size;
        let hi = self.max_cells_per_pixel.max(self.min_cells_per_pixel) * cell_size;
        (lo, hi.max(lo))
    }
}

/// World position under a point of the viewport for an orthographic 2D
/// camera (`WindowSize` scaling, centred viewport origin).
pub fn viewport_to_world(translation: Vec2, scale: f32, viewport_size: Vec2, point_in_viewport: Vec2) -> Vec2 {
    translation
        + Vec2::new(point_in_viewport.x - viewport_size.x * 0.5, viewport_size.y * 0.5 - point_in_viewport.y) * scale
}

/// Camera translation after changing the scale from `scale` to `new_scale`
/// while keeping the world point `anchor` at the same screen position.
pub fn zoom_about(translation: Vec2, scale: f32, new_scale: f32, anchor: Vec2) -> Vec2 {
    if scale <= 0.0 || !scale.is_finite() || !new_scale.is_finite() {
        return translation;
    }
    anchor + (translation - anchor) * (new_scale / scale)
}

/// Fraction of the remaining distance covered in `dt` seconds by an
/// exponential approach with time constant `tau` (`1 - e^(-dt/tau)`).
pub fn smoothing_factor(dt: f32, tau: f32) -> f32 {
    if tau <= 0.0 || !tau.is_finite() {
        return 1.0;
    }
    1.0 - (-dt.max(0.0) / tau).exp()
}

/// Camera centre and orthographic scale that frame the inclusive cell
/// rectangle `min..=max` in a viewport, leaving `margin` (fraction of the
/// viewport) on each side.
pub fn fit_rect(space: &GridSpace, min: I64Vec2, max: I64Vec2, viewport_size: Vec2, margin: f32) -> (Vec2, f32) {
    let top_left = space.cell_corner_to_world(min);
    let bottom_right = space.cell_corner_to_world(max.saturating_add(I64Vec2::ONE));
    let center = (top_left + bottom_right) * 0.5;
    let size = (bottom_right - top_left).abs();
    let usable = (viewport_size * (1.0 - 2.0 * margin.clamp(0.0, 0.45))).max(Vec2::ONE);
    let scale = (size.x / usable.x).max(size.y / usable.y);
    (center, scale)
}

/// Cell under the cursor for a 2D world camera, or `None` when the pointer is
/// not over the world viewport (per [`WorldInput`]). Uses the camera's
/// `GlobalTransform` and projection; when called in `Update` it reflects the
/// previous frame's pan and zoom. Re-anchoring shifts the `GlobalTransform`
/// together with [`GridSpace::anchor`], so the result stays consistent on the
/// frame the floating origin moves.
pub fn cursor_to_cell(
    input: &WorldInput,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    space: &GridSpace,
) -> Option<I64Vec2> {
    if !input.pointer {
        return None;
    }
    input.cursor_in_viewport()?;
    let cursor = input.cursor?;
    let world = camera.viewport_to_world_2d(camera_transform, cursor).ok()?;
    Some(space.world_to_cell(world))
}

const PAN_KEYS: [(KeyCode, Vec2); 8] = [
    (KeyCode::ArrowLeft, Vec2::NEG_X),
    (KeyCode::KeyA, Vec2::NEG_X),
    (KeyCode::ArrowRight, Vec2::X),
    (KeyCode::KeyD, Vec2::X),
    (KeyCode::ArrowUp, Vec2::Y),
    (KeyCode::KeyW, Vec2::Y),
    (KeyCode::ArrowDown, Vec2::NEG_Y),
    (KeyCode::KeyS, Vec2::NEG_Y),
];

#[allow(clippy::too_many_arguments)]
fn control_grid_camera(
    input: Res<WorldInput>,
    scroll: Res<AccumulatedMouseScroll>,
    mut pinch: MessageReader<PinchGesture>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    space: Res<GridSpace>,
    mut cameras: Query<(&mut GridCameraController, &mut Transform, &mut Projection, &Camera)>,
) {
    let pinch_total: f32 = pinch.read().map(|p| p.0).sum();
    let dt = time.delta_secs();
    for (mut ctl, mut transform, mut projection, camera) in &mut cameras {
        let Projection::Orthographic(ortho) = &mut *projection else { continue };
        if !matches!(ortho.scaling_mode, ScalingMode::WindowSize) {
            ortho.scaling_mode = ScalingMode::WindowSize;
        }
        let Some(viewport) = camera.logical_viewport_size().filter(|v| v.x > 0.0 && v.y > 0.0) else { continue };
        let (min_scale, max_scale) = ctl.scale_limits(space.cell_size);
        let mut translation = transform.translation.truncate();
        let mut scale = ortho.scale.clamp(min_scale, max_scale);
        let cursor = input.cursor_in_viewport();

        if let Some(fit) = ctl.pending_fit.take() {
            let (center, fit_scale) = fit_rect(&space, fit.min, fit.max, viewport, fit.margin);
            translation = center;
            scale = fit_scale.clamp(min_scale, max_scale);
            ctl.target_scale = None;
            ctl.zoom_anchor = None;
        }

        // Zoom requests: wheel / trackpad scroll and pinch over the world, +/- keys.
        let mut zoom_lines = 0.0;
        let mut pointer_zoom = false;
        if input.pointer {
            zoom_lines += match scroll.unit {
                MouseScrollUnit::Line => scroll.delta.y,
                MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_LINE,
            };
            pointer_zoom = zoom_lines != 0.0 || pinch_total != 0.0;
        }
        if input.keyboard {
            if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
                zoom_lines += 1.0;
            }
            if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
                zoom_lines -= 1.0;
            }
        }
        let pinch_factor = if input.pointer { (1.0 + pinch_total).max(0.1) } else { 1.0 };
        if zoom_lines != 0.0 || pinch_factor != 1.0 {
            let step = ctl.zoom_step.max(1.001);
            let target = ctl.target_scale.unwrap_or(scale) * step.powf(-zoom_lines) / pinch_factor;
            ctl.target_scale = Some(target.clamp(min_scale, max_scale));
            ctl.zoom_anchor = match (pointer_zoom, cursor) {
                (true, Some(c)) => Some(viewport_to_world(translation, scale, viewport, c)),
                _ => Some(translation),
            };
        }
        if let Some(target) = ctl.target_scale {
            let f = smoothing_factor(dt, ctl.zoom_time_constant);
            let mut next = (scale.ln() + (target.ln() - scale.ln()) * f).exp();
            if (next / target - 1.0).abs() < 1e-4 {
                next = target;
                ctl.target_scale = None;
            }
            let anchor = ctl.zoom_anchor.unwrap_or(translation);
            translation = zoom_about(translation, scale, next, anchor);
            scale = next;
            if ctl.target_scale.is_none() {
                ctl.zoom_anchor = None;
            }
        }

        // Grab-style drag panning with the right or middle button.
        let drag_buttons = [MouseButton::Right, MouseButton::Middle];
        if buttons.any_just_pressed(drag_buttons) && input.pointer && cursor.is_some() {
            ctl.dragging = true;
            ctl.last_cursor = input.cursor;
        }
        if !buttons.any_pressed(drag_buttons) {
            ctl.dragging = false;
            ctl.last_cursor = None;
        }
        if ctl.dragging
            && let Some(now) = input.cursor
        {
            if let Some(prev) = ctl.last_cursor {
                let d = now - prev;
                translation += Vec2::new(-d.x, d.y) * scale;
                if let Some(anchor) = ctl.zoom_anchor.as_mut() {
                    *anchor += Vec2::new(-d.x, d.y) * scale;
                }
            }
            ctl.last_cursor = Some(now);
        }

        // Keyboard panning.
        if input.keyboard {
            let dir: Vec2 = PAN_KEYS.iter().filter(|(k, _)| keys.pressed(*k)).map(|(_, d)| *d).sum();
            if dir != Vec2::ZERO {
                let delta = dir.normalize() * ctl.key_pan_speed * scale * dt;
                translation += delta;
                if let Some(anchor) = ctl.zoom_anchor.as_mut() {
                    *anchor += delta;
                }
            }
        }

        if ortho.scale != scale {
            ortho.scale = scale;
        }
        let new_translation = translation.extend(transform.translation.z);
        if transform.translation != new_translation {
            transform.translation = new_translation;
        }
    }
}

/// Moves the floating origin to the cell under the camera when it drifted
/// more than [`REANCHOR_CELLS`] away. The controlled camera must be a root
/// entity (its translation is in world space).
fn reanchor_grid_space(
    mut space: ResMut<GridSpace>,
    mut cameras: Query<(&mut GridCameraController, &mut Transform, Option<&mut GlobalTransform>)>,
) {
    let Some((mut ctl, mut transform, global)) = cameras.iter_mut().find(|(c, _, _)| c.auto_reanchor) else {
        return;
    };
    let cell = space.cell_size as f64;
    let offset = Vec2::new(transform.translation.x, transform.translation.y);
    let (dx, dy) = ((offset.x as f64 / cell).round(), (-(offset.y as f64) / cell).round());
    if dx.abs() < REANCHOR_CELLS && dy.abs() < REANCHOR_CELLS {
        return;
    }
    let shift = I64Vec2::new(dx as i64, dy as i64);
    let world_shift = Vec2::new((dx * cell) as f32, (-dy * cell) as f32);
    space.anchor = space.anchor.saturating_add(shift);
    transform.translation -= world_shift.extend(0.0);
    if let Some(anchor) = ctl.zoom_anchor.as_mut() {
        *anchor -= world_shift;
    }
    // Keep this frame's GlobalTransform consistent with the new anchor so
    // `cursor_to_cell` and other world <-> cell conversions made later in
    // `Update` do not jump by the shift for one frame.
    if let Some(mut global) = global {
        let mut affine = global.affine();
        affine.translation -= bevy::math::Vec3A::from(world_shift.extend(0.0));
        *global = GlobalTransform::from(affine);
    }
}

fn update_visible_cells(
    space: Res<GridSpace>,
    cameras: Query<(&Transform, &Projection, &Camera), With<GridCameraController>>,
    mut visible: ResMut<VisibleCells>,
) {
    let Some((transform, projection, camera)) = cameras.iter().find(|(_, _, c)| c.is_active) else { return };
    let Projection::Orthographic(ortho) = projection else { return };
    let Some(viewport) = camera.logical_viewport_size() else { return };
    let next = space.visible_cells(transform.translation.truncate(), ortho.scale, viewport);
    visible.set_if_neq(next);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_mapping_centre_and_corners() {
        let t = Vec2::new(10.0, -4.0);
        let vp = Vec2::new(200.0, 100.0);
        assert_eq!(viewport_to_world(t, 0.5, vp, Vec2::new(100.0, 50.0)), t);
        assert_eq!(viewport_to_world(t, 0.5, vp, Vec2::ZERO), t + Vec2::new(-50.0, 25.0));
        assert_eq!(viewport_to_world(t, 0.5, vp, vp), t + Vec2::new(50.0, -25.0));
    }

    #[test]
    fn zoom_keeps_anchor_under_cursor() {
        let vp = Vec2::new(800.0, 600.0);
        let cursor = Vec2::new(620.0, 140.0);
        let t = Vec2::new(3.0, 7.0);
        let anchor = viewport_to_world(t, 2.0, vp, cursor);
        let t2 = zoom_about(t, 2.0, 0.5, anchor);
        let after = viewport_to_world(t2, 0.5, vp, cursor);
        assert!((after - anchor).length() < 1e-4, "{after:?} vs {anchor:?}");
        // Degenerate scales leave the camera alone.
        assert_eq!(zoom_about(t, 0.0, 1.0, anchor), t);
    }

    #[test]
    fn smoothing_factor_properties() {
        assert_eq!(smoothing_factor(0.1, 0.0), 1.0);
        assert_eq!(smoothing_factor(0.0, 0.1), 0.0);
        let one = smoothing_factor(0.1, 0.1);
        assert!((one - (1.0 - (-1.0f32).exp())).abs() < 1e-6);
        // Two half steps equal one full step (frame-rate independence).
        let half = smoothing_factor(0.05, 0.1);
        assert!(((1.0 - half) * (1.0 - half) - (1.0 - one)).abs() < 1e-6);
    }

    #[test]
    fn fit_rect_frames_cells() {
        let space = GridSpace::new(2.0);
        let (center, scale) = fit_rect(&space, I64Vec2::new(0, 0), I64Vec2::new(99, 49), Vec2::new(400.0, 400.0), 0.0);
        // 100x50 cells of size 2 = 200x100 world units in a 400 px square.
        assert_eq!(center, Vec2::new(100.0, -50.0));
        assert_eq!(scale, 0.5);
        let (_, with_margin) =
            fit_rect(&space, I64Vec2::new(0, 0), I64Vec2::new(99, 49), Vec2::new(400.0, 400.0), 0.25);
        assert_eq!(with_margin, 1.0);
        // Tall rectangle is limited by height.
        let (_, tall) = fit_rect(&space, I64Vec2::new(0, 0), I64Vec2::new(9, 199), Vec2::new(400.0, 400.0), 0.0);
        assert_eq!(tall, 1.0);
    }

    #[test]
    fn fit_request_normalises_rectangle() {
        let mut ctl = GridCameraController::default();
        ctl.fit_to_rect(I64Vec2::new(5, -2), I64Vec2::new(-5, 2), f32::NAN);
        assert_eq!(
            ctl.pending_fit,
            Some(FitRequest { min: I64Vec2::new(-5, -2), max: I64Vec2::new(5, 2), margin: 0.0 })
        );
    }

    #[test]
    fn scale_limits_follow_cell_size() {
        let ctl = GridCameraController { min_cells_per_pixel: 0.25, max_cells_per_pixel: 4.0, ..default() };
        assert_eq!(ctl.scale_limits(2.0), (0.5, 8.0));
        let inverted = GridCameraController { min_cells_per_pixel: 4.0, max_cells_per_pixel: 1.0, ..default() };
        let (lo, hi) = inverted.scale_limits(1.0);
        assert!(lo <= hi);
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;

    #[test]
    fn reanchoring_moves_origin_with_camera() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).insert_resource(GridSpace::new(2.0)).add_systems(Update, reanchor_grid_space);
        let near =
            app.world_mut().spawn((GridCameraController::default(), Transform::from_xyz(100.0, -50.0, 0.0))).id();
        app.update();
        assert_eq!(app.world().resource::<GridSpace>().anchor, I64Vec2::ZERO, "close to the anchor: unchanged");
        // 10 001 cells right and 5 000 cells down (world y negative).
        let far = Vec3::new(20_002.5, -10_000.0, 7.0);
        app.world_mut().get_mut::<Transform>(near).expect("camera").translation = far;
        // Last frame's propagated transform, as `Update` systems would see it.
        *app.world_mut().get_mut::<GlobalTransform>(near).expect("camera") = GlobalTransform::from_translation(far);
        let cell_before = app.world().resource::<GridSpace>().world_to_cell(far.truncate());
        app.update();
        let space = *app.world().resource::<GridSpace>();
        assert_eq!(space.anchor, I64Vec2::new(10_001, 5_000));
        let t = app.world().get::<Transform>(near).expect("camera").translation;
        assert_eq!(t, Vec3::new(0.5, 0.0, 7.0));
        // The cell under the camera centre is unchanged by re-anchoring.
        assert_eq!(space.world_to_cell(t.truncate()), cell_before);
        // The GlobalTransform moved with the anchor, so conversions made later
        // in the same frame (cursor_to_cell) still hit the same cell.
        let global = app.world().get::<GlobalTransform>(near).expect("camera").translation();
        assert_eq!(global, t);
        assert_eq!(space.world_to_cell(global.truncate()), cell_before);
    }
}
