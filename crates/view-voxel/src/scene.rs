//! The 3D scene: orbit camera, lights, floor, the voxel root entity and the
//! gizmo overlays (world box, cut planes, region, inspected layer, hover).
//!
//! Depth cues: a shadow-casting key light from the upper left, a cool fill
//! light without shadows from the opposite side, per-camera ambient light,
//! a floor that receives shadows, depth-shaded face colours (see
//! [`crate::colors`]) and optional distance fog towards the background.

use std::f32::consts::FRAC_PI_2;

use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, NotShadowCaster};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::view::{Msaa, Tonemapping};
use bevy_egui::egui::Color32;
use bevy_panorbit_camera::PanOrbitCamera;
use interface::theme::bevy_color;
use interface::{ModeRegistry, Palette, ThemeKind, ThemeSettings};
use rendering::orbit::AERIAL_PITCH;
use rendering::{OrbitCameraRig, OrbitPreset, WorldInput};
use voxel_life::Bounds;

use crate::MODE;
use crate::build::MeshState;
use crate::coords::{box_corners, box_edges, framing_distance, plane_rect};
use crate::interact::PointerState;
use crate::requests::{ProbeState, SliceState, WorldState};
use crate::settings::ViewSettings;

/// Parent of the chunk meshes; its transform maps model to world space.
#[derive(Component, Debug, Default)]
pub(crate) struct VoxelRoot;

/// The mode's world camera.
#[derive(Component, Debug, Default)]
pub(crate) struct VoxelCamera;

/// The shadow-casting light.
#[derive(Component, Debug, Default)]
pub(crate) struct KeyLight;

/// The floor plane.
#[derive(Component, Debug, Default)]
pub(crate) struct Floor;

/// Materials of the scene.
#[derive(Resource, Debug, Clone)]
pub(crate) struct SceneAssets {
    /// Opaque voxel faces (vertex colours).
    pub opaque: Handle<StandardMaterial>,
    /// Translucent voxel faces.
    pub translucent: Handle<StandardMaterial>,
    /// The floor.
    pub floor: Handle<StandardMaterial>,
}

/// Camera requests from buttons, shortcuts and tutorial steps.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub(crate) enum CameraAction {
    /// Fit the whole world (or the initialisation box and population of an
    /// unbounded world).
    FrameWorld,
    /// Fit the non-empty voxels.
    FramePopulation,
    /// Look along +y (model), from the front.
    Front,
    /// Look along −x (model), from the right side.
    Side,
    /// Look straight down.
    Top,
    /// Three-quarter view.
    Corner,
    /// Switch the rig preset.
    Preset(OrbitPreset),
}

/// Depth-tested lines (world box, floor grid, axes).
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct FrameGizmos;

/// Lines drawn on top of the voxels (cut planes, region, hover).
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct OverlayGizmos;

/// Gizmo configurations of the mode's groups.
pub(crate) fn gizmo_configs() -> (GizmoConfig, GizmoConfig) {
    let frame = GizmoConfig { line: GizmoLineConfig { width: 1.5, ..default() }, ..default() };
    let overlay = GizmoConfig { line: GizmoLineConfig { width: 2.5, ..default() }, depth_bias: -1.0, ..default() };
    (frame, overlay)
}

/// Initial three-quarter view.
const CORNER_YAW: f32 = 0.72;
const CORNER_PITCH: f32 = 0.5;

/// Spawns camera, lights, floor and the voxel root (all despawned on exit).
pub(crate) fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    theme: Res<ThemeSettings>,
) {
    let assets = SceneAssets {
        opaque: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.72,
            reflectance: 0.35,
            ..default()
        }),
        translucent: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.35,
            reflectance: 0.5,
            double_sided: true,
            cull_mode: None,
            ..default()
        }),
        floor: materials.add(StandardMaterial {
            base_color: floor_color(theme.kind),
            perceptual_roughness: 0.95,
            reflectance: 0.15,
            ..default()
        }),
    };
    commands.spawn((
        Name::new("3D Life camera"),
        VoxelCamera,
        OrbitCameraRig::new(OrbitPreset::FreeOrbit).with_distance_limits(2.0, 6000.0),
        PanOrbitCamera {
            focus: Vec3::ZERO,
            yaw: Some(CORNER_YAW),
            pitch: Some(CORNER_PITCH),
            radius: Some(140.0),
            ..default()
        },
        Transform::from_xyz(90.0, 70.0, 80.0).looking_at(Vec3::ZERO, Vec3::Y),
        Msaa::Sample4,
        // An identity tone curve: voxel colours stay faithful to the legend
        // (a filmic curve desaturates them), and fogged surfaces converge
        // to exactly the background (the clear colour is not tone mapped).
        // The lights are dimmed so lit faces do not clip.
        Tonemapping::Linear,
        AmbientLight { color: Color::srgb(0.85, 0.9, 1.0), brightness: 250.0, ..default() },
        DistanceFog {
            color: Color::NONE,
            directional_light_color: Color::NONE,
            directional_light_exponent: 8.0,
            falloff: FogFalloff::Linear { start: 1.0e6, end: 1.0e6 + 1.0 },
        },
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("3D Life key light"),
        KeyLight,
        DirectionalLight {
            illuminance: 5_400.0,
            color: Color::srgb(1.0, 0.97, 0.92),
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(-0.55, 1.0, 0.42).looking_at(Vec3::ZERO, Vec3::Y),
        cascades(150.0, 64.0),
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("3D Life fill light"),
        DirectionalLight { illuminance: 1_300.0, color: Color::srgb(0.78, 0.85, 1.0), ..default() },
        Transform::from_xyz(0.7, 0.35, -0.8).looking_at(Vec3::ZERO, Vec3::Y),
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("3D Life floor"),
        Floor,
        Mesh3d(meshes.add(Plane3d::default().mesh().size(1.0, 1.0))),
        MeshMaterial3d(assets.floor.clone()),
        Transform::from_xyz(0.0, -0.04, 0.0).with_scale(Vec3::new(200.0, 1.0, 200.0)),
        NotShadowCaster,
        Visibility::Hidden,
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("3D Life voxels"),
        VoxelRoot,
        Transform::default(),
        Visibility::default(),
        DespawnOnExit(MODE),
    ));
    commands.insert_resource(assets);
}

fn cascades(camera_distance: f32, diagonal: f32) -> CascadeShadowConfig {
    let maximum = (camera_distance + diagonal * 1.5).max(20.0);
    CascadeShadowConfigBuilder {
        num_cascades: 3,
        minimum_distance: 0.1,
        first_cascade_far_bound: (camera_distance * 0.6).clamp(5.0, maximum * 0.5),
        maximum_distance: maximum,
        ..default()
    }
    .build()
}

fn floor_color(kind: ThemeKind) -> Color {
    match kind {
        ThemeKind::Dark => Color::srgb(0.15, 0.16, 0.19),
        ThemeKind::Light => Color::srgb(0.80, 0.82, 0.86),
        ThemeKind::HighContrast => Color::srgb(0.04, 0.04, 0.04),
    }
}

/// The box the camera frames for "frame world".
pub(crate) fn world_frame_box(world: &WorldState) -> Option<Bounds> {
    let info = world.info.as_ref()?;
    if !info.is_finite() {
        return Some(world.layers());
    }
    let bounds = info.bounds;
    // Very large sparse worlds: frame the population instead of a box the
    // voxels would be invisible in.
    let huge = bounds.extent().iter().any(|&e| e > 512);
    Some(if huge { world.population.unwrap_or_else(|| info.nominal()) } else { bounds })
}

/// Whether `action` must wait for the rig to apply a preset change first.
///
/// `OrbitCameraRig` applies a new preset on its next run and then resets
/// the pitch (and the distance units of the projection), so an aim or a
/// framing requested in the same frame as a preset change would be
/// overwritten. Such actions are deferred by one frame.
pub(crate) fn needs_preset(action: CameraAction, current: OrbitPreset) -> Option<OrbitPreset> {
    let wanted = match action {
        CameraAction::Front | CameraAction::Side | CameraAction::Top | CameraAction::Corner => OrbitPreset::FreeOrbit,
        CameraAction::Preset(preset) => preset,
        CameraAction::FrameWorld | CameraAction::FramePopulation => return None,
    };
    (wanted != current).then_some(wanted)
}

/// Applies camera requests (after the rig systems, see [`needs_preset`]).
#[allow(clippy::type_complexity)]
pub(crate) fn camera_actions(
    mut actions: MessageReader<CameraAction>,
    mut deferred: Local<Vec<CameraAction>>,
    world: Res<WorldState>,
    mesh: Res<MeshState>,
    camera: Option<Single<(&mut OrbitCameraRig, &mut PanOrbitCamera, &Projection, &Camera), With<VoxelCamera>>>,
) {
    let Some(camera) = camera else {
        actions.clear();
        deferred.clear();
        return;
    };
    let (mut rig, mut cam, projection, camera) = camera.into_inner();
    let frame = world.frame();
    let pending: Vec<CameraAction> = deferred.drain(..).chain(actions.read().copied()).collect();
    for (index, action) in pending.iter().enumerate() {
        if let Some(preset) = needs_preset(*action, rig.preset) {
            rig.preset = preset;
            // A pure preset switch is done; anything else is re-run once the
            // rig has applied the preset.
            let from = if matches!(action, CameraAction::Preset(_)) { index + 1 } else { index };
            deferred.extend_from_slice(&pending[from..]);
            break;
        }
        let fit = |bounds: Bounds, rig: &OrbitCameraRig, cam: &mut PanOrbitCamera| {
            let (min, max) = box_corners(bounds);
            let centre = frame.to_world((min + max) * 0.5);
            // At least 12 voxels across, so a tiny pattern keeps its context.
            let size = (max - min).max(Vec3::splat(12.0));
            let aspect = camera.logical_viewport_size().map_or(1.5, |s| s.x / s.y.max(1.0));
            let distance = framing_distance(Vec3::new(size.x, size.z, size.y), rig.fov, aspect);
            rig.focus_on(cam, projection, centre, distance);
        };
        match *action {
            CameraAction::FrameWorld => {
                if let Some(bounds) = world_frame_box(&world) {
                    fit(bounds, &rig, &mut cam);
                }
            }
            CameraAction::FramePopulation => {
                let bounds = world
                    .population
                    .or_else(|| mesh.summary.bbox.map(|(lo, hi)| Bounds { min: lo.to_array(), max: hi.to_array() }));
                if let Some(bounds) = bounds {
                    fit(bounds, &rig, &mut cam);
                }
            }
            CameraAction::Front | CameraAction::Side | CameraAction::Top | CameraAction::Corner => {
                rig.preset = OrbitPreset::FreeOrbit;
                let (yaw, pitch) = match *action {
                    CameraAction::Front => (0.0, 0.0),
                    CameraAction::Side => (FRAC_PI_2, 0.0),
                    CameraAction::Top => (0.0, AERIAL_PITCH),
                    _ => (CORNER_YAW, CORNER_PITCH),
                };
                // Shortest way round to the requested yaw.
                let turns = ((cam.target_yaw - yaw) / std::f32::consts::TAU).round();
                cam.target_yaw = yaw + turns * std::f32::consts::TAU;
                cam.target_pitch = pitch;
            }
            CameraAction::Preset(preset) => rig.preset = preset,
        }
    }
}

/// Turntable rotation and the cut sweep.
pub(crate) fn animate(
    time: Res<Time<Real>>,
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    world: Res<WorldState>,
    mut settings: ResMut<ViewSettings>,
    camera: Option<Single<(&OrbitCameraRig, &mut PanOrbitCamera), With<VoxelCamera>>>,
    mut sweep_progress: Local<f32>,
) {
    let dt = time.delta_secs().min(0.1);
    if settings.turntable
        && let Some(camera) = camera
    {
        let (rig, mut cam) = camera.into_inner();
        let dragging = input.pointer && buttons.any_pressed([MouseButton::Left, MouseButton::Right]);
        if rig.preset.config().yaw.is_none() && !dragging {
            cam.target_yaw += settings.turntable_speed * dt;
        }
    }
    if settings.sweep && settings.cut.enabled {
        *sweep_progress += dt * settings.sweep_rate.max(0.1);
        let (_, hi) = crate::settings::axis_range(world.layers(), settings.cut.axis);
        while *sweep_progress >= 1.0 {
            *sweep_progress -= 1.0;
            settings.cut.sweep_step(hi);
        }
    } else {
        *sweep_progress = 0.0;
    }
}

/// Keeps the root transform, floor, fog, shadows and floor colour in step
/// with the world, the camera and the theme.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn update_scene(
    world: Res<WorldState>,
    settings: Res<ViewSettings>,
    clear: Res<ClearColor>,
    theme: Res<ThemeSettings>,
    assets: Option<Res<SceneAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut root: Query<&mut Transform, With<VoxelRoot>>,
    mut floor: Query<(&mut Transform, &mut Visibility), (With<Floor>, Without<VoxelRoot>)>,
    mut camera: Query<
        (&Transform, &PanOrbitCamera, &Projection, &mut DistanceFog),
        (With<VoxelCamera>, Without<VoxelRoot>, Without<Floor>),
    >,
    mut lights: Query<&mut CascadeShadowConfig, With<KeyLight>>,
    mut shadow_distance: Local<f32>,
) {
    let frame = world.frame();
    let target = frame.transform();
    for mut transform in &mut root {
        if *transform != target {
            *transform = target;
        }
    }
    let layers = world.layers();
    let (min, max) = box_corners(layers);
    let diagonal = (max - min).length().max(4.0);
    if let Ok((mut transform, mut visibility)) = floor.single_mut() {
        let show = settings.show_floor && world.info.is_some();
        visibility.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
        let centre = frame.to_world(Vec3::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, min.z));
        // Large enough for its edge to fade into the fog.
        let footprint = (max.x - min.x).max(max.y - min.y).max(24.0) * 16.0;
        let wanted =
            Transform::from_translation(centre - Vec3::Y * 0.04).with_scale(Vec3::new(footprint, 1.0, footprint));
        if *transform != wanted {
            *transform = wanted;
        }
    }
    if let Ok((transform, cam, projection, mut fog)) = camera.single_mut() {
        let distance = (transform.translation - cam.focus).length();
        let orthographic = matches!(projection, Projection::Orthographic(_));
        let falloff = if settings.fog && !orthographic {
            // From just behind the focus, so the near half of the world keeps
            // full contrast and the floor's edge dissolves into the background.
            FogFalloff::Linear { start: distance + diagonal * 0.1, end: distance + diagonal * 2.5 }
        } else {
            FogFalloff::Linear { start: 1.0e6, end: 1.0e6 + 1.0 }
        };
        let color = clear.0;
        if fog.color != color {
            fog.color = color;
        }
        let changed = match (&fog.falloff, &falloff) {
            (FogFalloff::Linear { start: a, end: b }, FogFalloff::Linear { start: c, end: d }) => {
                (a - c).abs() > 0.05 || (b - d).abs() > 0.05
            }
            _ => true,
        };
        if changed {
            fog.falloff = falloff;
        }
        let shadow = if orthographic { diagonal * 2.0 } else { distance };
        if (*shadow_distance - shadow).abs() > shadow.max(1.0) * 0.15 {
            *shadow_distance = shadow;
            for mut config in &mut lights {
                *config = cascades(shadow, diagonal);
            }
        }
    }
    if theme.is_changed()
        && let Some(assets) = assets
        && let Some(mut material) = materials.get_mut(&assets.floor)
    {
        material.base_color = floor_color(theme.kind);
    }
}

/// A "nice" grid step (1, 2, 4, 8, …) giving at most `lines` lines over `span`.
pub(crate) fn grid_step(span: f32, lines: f32) -> f32 {
    let mut step = 1.0;
    while span / step > lines {
        step *= 2.0;
    }
    step
}

/// Colour of the frame marking the slice inspector's layer in the world
/// (and of its legend): teal, darker on light surfaces for contrast.
pub(crate) fn inspector_color(palette: &Palette) -> Color32 {
    if palette.dark { Color32::from_rgb(79, 209, 197) } else { Color32::from_rgb(14, 128, 118) }
}

/// Gizmo colours of the current theme.
struct GizmoColors {
    frame: Color,
    grid: Color,
    accent: Color,
    accent_soft: Color,
    inspector: Color,
    hover: Color,
}

fn gizmo_colors(palette: &Palette) -> GizmoColors {
    let c = |color: Color32, alpha: f32| bevy_color(color).with_alpha(alpha);
    GizmoColors {
        frame: c(palette.text_muted, 0.75),
        grid: c(palette.text_muted, if palette.dark { 0.10 } else { 0.22 }),
        accent: c(palette.accent, 1.0),
        accent_soft: c(palette.accent, 0.55),
        inspector: c(inspector_color(palette), 0.7),
        hover: c(palette.text, 0.95),
    }
}

/// Draws the world box, floor grid, axes, cut planes, inspected layer,
/// region box, hovered and pinned voxels.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_gizmos(
    mut frame_gizmos: Gizmos<FrameGizmos>,
    mut overlay: Gizmos<OverlayGizmos>,
    world: Res<WorldState>,
    settings: Res<ViewSettings>,
    pointer: Res<PointerState>,
    probe: Res<ProbeState>,
    slice: Res<SliceState>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
) {
    let Some(info) = &world.info else { return };
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    let colors = gizmo_colors(&palette);
    let frame = world.frame();
    let line = |g: &mut Gizmos<FrameGizmos>, a: Vec3, b: Vec3, color: Color| {
        g.line(frame.to_world(a), frame.to_world(b), color)
    };
    let layers = world.layers();
    let (min, max) = box_corners(layers);
    if settings.show_bounds {
        let (bmin, bmax) = box_corners(if info.is_finite() { info.bounds } else { info.nominal() });
        for (a, b) in box_edges(bmin, bmax) {
            line(&mut frame_gizmos, a, b, colors.frame);
        }
        if !info.is_finite()
            && let Some(population) = world.population
        {
            let (pmin, pmax) = box_corners(population);
            for (a, b) in box_edges(pmin, pmax) {
                line(&mut frame_gizmos, a, b, colors.accent_soft);
            }
        }
    }
    if settings.show_floor {
        let step = grid_step((max.x - min.x).max(max.y - min.y), 32.0);
        let z = min.z;
        let mut x = min.x;
        while x <= max.x + 1e-3 {
            line(&mut frame_gizmos, Vec3::new(x, min.y, z), Vec3::new(x, max.y, z), colors.grid);
            x += step;
        }
        let mut y = min.y;
        while y <= max.y + 1e-3 {
            line(&mut frame_gizmos, Vec3::new(min.x, y, z), Vec3::new(max.x, y, z), colors.grid);
            y += step;
        }
    }
    if settings.show_axes {
        let length = ((max - min).max_element() * 0.18).clamp(3.0, 40.0);
        let o = min;
        line(&mut frame_gizmos, o, o + Vec3::X * length, Color::srgb(0.93, 0.33, 0.31));
        line(&mut frame_gizmos, o, o + Vec3::Y * length, Color::srgb(0.40, 0.80, 0.36));
        line(&mut frame_gizmos, o, o + Vec3::Z * length, Color::srgb(0.35, 0.55, 0.98));
    }
    let rect = |g: &mut Gizmos<OverlayGizmos>, corners: [Vec3; 4], color: Color| {
        let points: Vec<Vec3> = corners.iter().chain(corners.first()).map(|p| frame.to_world(*p)).collect();
        g.linestrip(points, color);
    };
    if settings.cut.enabled {
        let cut = settings.cut.range();
        rect(&mut overlay, plane_rect(min, max, cut.axis, cut.min as f32), colors.accent_soft);
        rect(&mut overlay, plane_rect(min, max, cut.axis, cut.max as f32 + 1.0), colors.accent);
    }
    let follows_cut = settings.inspector.follow_cut && settings.cut.enabled;
    if slice.shown_recently() && !follows_cut {
        let (axis, layer) = settings.inspector_layer();
        rect(&mut overlay, plane_rect(min, max, axis, layer as f32 + 0.5), colors.inspector);
    }
    if settings.region.enabled {
        let (rmin, rmax) = box_corners(settings.region.bounds());
        for (a, b) in box_edges(rmin, rmax) {
            overlay.line(frame.to_world(a), frame.to_world(b), colors.accent);
        }
    }
    let voxel_box = |g: &mut Gizmos<OverlayGizmos>, v: IVec3, color: Color, inset: f32| {
        let lo = v.as_vec3() - Vec3::splat(inset);
        let hi = v.as_vec3() + Vec3::splat(1.0 + inset);
        for (a, b) in box_edges(lo, hi) {
            g.line(frame.to_world(a), frame.to_world(b), color);
        }
    };
    if let Some(hover) = pointer.hover {
        if let Some(voxel) = hover.voxel {
            voxel_box(&mut overlay, voxel, colors.hover, 0.03);
        }
        if let Some(target) = hover.place {
            voxel_box(&mut overlay, target, colors.accent, 0.0);
        }
    }
    if let Some(pos) = probe.pinned {
        voxel_box(&mut overlay, IVec3::from_array(pos), colors.accent, 0.08);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_steps_are_powers_of_two_within_the_line_budget() {
        assert_eq!(grid_step(24.0, 32.0), 1.0);
        assert_eq!(grid_step(64.0, 32.0), 2.0);
        assert_eq!(grid_step(65_536.0, 32.0), 2048.0);
        for span in [1.0f32, 33.0, 100.0, 1000.0] {
            let step = grid_step(span, 32.0);
            assert!(span / step <= 32.0);
            assert!(step == 1.0 || span / (step / 2.0) > 32.0, "the smallest such step");
        }
    }

    #[test]
    fn views_wait_for_a_preset_change() {
        use OrbitPreset::*;
        assert_eq!(needs_preset(CameraAction::Front, Isometric), Some(FreeOrbit));
        assert_eq!(needs_preset(CameraAction::Top, FreeOrbit), None);
        assert_eq!(needs_preset(CameraAction::FrameWorld, Isometric), None, "framing works in every preset");
        assert_eq!(needs_preset(CameraAction::Preset(Aerial), FreeOrbit), Some(Aerial));
        assert_eq!(needs_preset(CameraAction::Preset(Aerial), Aerial), None);
    }

    #[test]
    fn huge_sparse_worlds_frame_the_population() {
        use crate::requests::{ScenarioMeta, WorldInfo};
        use voxel_life::{Evidence, InitSpec, Rule3D, WorldConfig, WorldKind};
        let config =
            WorldConfig { kind: WorldKind::SparseBounded, size: [4096, 4096, 64], init: InitSpec::default(), seed: 0 };
        let info = WorldInfo {
            descriptor: simulation_core::ModelDescriptor::new("voxel.life", "3D", simulation_core::ModelFamily::Voxel),
            scenario: ScenarioMeta {
                id: "x".into(),
                name: "x".into(),
                description: String::new(),
                evidence: Evidence::Unverified,
                reference: String::new(),
            },
            rule: Rule3D::bays_5766(),
            pending: config.clone(),
            bounds: Bounds::from_size(config.size),
            config,
            halt: None,
            tick: 0,
            counts: vec![0, 0],
            population_bounds: None,
        };
        let mut world = WorldState::default();
        world.info = Some(std::sync::Arc::new(info));
        let population = Bounds { min: [10, 10, 10], max: [20, 20, 20] };
        world.population = Some(population);
        assert_eq!(world_frame_box(&world), Some(population));
    }
}
