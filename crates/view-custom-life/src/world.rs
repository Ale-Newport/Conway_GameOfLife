//! World rendering and interaction.
//!
//! * Floor and field overlays: one [`GridSurface`] (rendering crate) under
//!   the entities. Without an overlay it is a flat floor with cell lines;
//!   with one it shows a resource or environment field through a colormap
//!   (`Heatmap` style, value in the B channel).
//! * Entities: one `Mesh2d` each (shared meshes, one `ColorMaterial` per
//!   type and colour variant), coloured by type, energy or state, sized by
//!   their diameter, with a dark rim and — for mobile types — a heading
//!   notch. Glyphs are painted by the overlay (`overlay.rs`).
//! * Gizmos: world border, selection (with its perception radius), hover,
//!   brush preview.
//! * Input (only when [`WorldInput`] allows it): wheel zoom about the
//!   cursor, right/middle drag (or left drag on empty space) to pan, arrow
//!   keys, and the tools (select, spawn, remove, paint).

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::math::I64Vec2;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use interface::{ModeRegistry, Palette, ThemeSettings, Toasts};
use rendering::grid::encode_unit;
use rendering::{
    Colormap, GridAppearance, GridFrame, GridPalette, GridSpace, GridStyle, GridSurface, WorldCamera, WorldInput,
};
use rule_engine::sim::view::FieldView;
use rule_engine::{CustomLifeCommand, CustomLifeView};
use simulation_core::Control;

use crate::MODE;
use crate::colors::{ColorBy, bevy_color, entity_rgba, field_range};
use crate::coords::{inside_world, pick, scale_bar, scatter, screen_to_world, world_to_screen};
use crate::editor::{ClEditor, EditorTab};
use crate::session::{ClData, ClRunner};
use crate::state::{ClUi, Tool};

/// Marker of the mode's world camera.
#[derive(Component, Debug, Default)]
pub struct ClCamera;

/// Marker of the floor / field surface.
#[derive(Component, Debug, Default)]
pub struct ClFieldSurface;

/// A drawn entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct ClEntityVisual {
    /// Simulation id.
    pub id: u64,
}

/// Camera controller state.
#[derive(Component, Debug, Default, Clone)]
pub struct ClCameraController {
    /// Scale the zoom is easing towards.
    pub target_scale: Option<f32>,
    /// World point kept under the cursor while zooming.
    pub zoom_anchor: Option<Vec2>,
    /// Cursor of the previous frame of a pan drag.
    pub pan_from: Option<Vec2>,
    /// Where the left button went down (window px) and whether that drag pans.
    pub left_press: Option<(Vec2, bool)>,
    /// Last painted position (simulation units).
    pub last_dab: Option<Vec2>,
    /// Frames during which a requested fit is re-applied (the viewport may
    /// still be settling while panels lay out).
    pub fit_frames: u8,
}

/// Meshes, materials and the id → entity map of the drawn entities.
#[derive(Resource, Default)]
pub struct ClVisuals {
    disc: Option<Handle<Mesh>>,
    pointer: Option<Handle<Mesh>>,
    materials: HashMap<(u16, u8), (Handle<ColorMaterial>, [f32; 4])>,
    map: HashMap<u64, Entity>,
    drawn: Option<DrawnKey>,
    field_key: Option<FieldKey>,
}

/// What the entity meshes were last built from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DrawnKey {
    seq: u64,
    color_by: ColorBy,
    headings: bool,
    info_rev: u64,
}

/// What the field surface was last built from (`seq` is `None` for the
/// static floor, which does not change with the view).
#[derive(Clone, Debug, PartialEq)]
struct FieldKey {
    seq: Option<u64>,
    size: (u32, u32),
    overlay: Option<String>,
    colormap: Colormap,
    intensity: u32,
    grid: bool,
    floor: [u8; 4],
}

impl ClVisuals {
    /// Forgets every drawn entity (they are despawned with the mode).
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Smallest and largest zoom (world units per logical pixel) for a world.
pub fn zoom_limits(width: u32, height: u32) -> (f32, f32) {
    (1.0 / 200.0, (width.max(height) as f32 / 60.0).max(0.5))
}

/// Camera centre and scale framing the whole world in a viewport (5 %
/// margin per side).
pub fn fit_world(width: u32, height: u32, viewport: Vec2) -> (Vec2, f32) {
    let usable = (viewport * 0.9).max(Vec2::ONE);
    let scale = (width as f32 / usable.x).max(height as f32 / usable.y);
    (Vec2::new(width as f32 / 2.0, -(height as f32) / 2.0), scale)
}

/// Like [`fit_world`] for a `viewport` (window logical px) whose top-left
/// corner is covered by the overlay card `avoid`. When the region right of
/// the card frames the world at nearly the same size (at most 12 %
/// smaller), the world is centred in that region; otherwise it keeps the
/// plain size and moves right by as much as the horizontal slack allows
/// (8 px from the edge), so the card hides as little of it as possible.
/// Returns the camera centre and scale.
pub fn fit_world_in(width: u32, height: u32, viewport: Rect, avoid: Option<Rect>) -> (Vec2, f32) {
    let (centre, scale) = fit_world(width, height, viewport.size());
    let Some(card) = avoid.filter(|c| c.max.x > viewport.min.x && c.min.x < viewport.max.x) else {
        return (centre, scale);
    };
    let inset = (card.max.x - viewport.min.x + 8.0).clamp(0.0, viewport.width());
    let free = Vec2::new(viewport.width() - inset, viewport.height());
    if free.x >= viewport.width() * 0.5 {
        let (_, free_scale) = fit_world(width, height, free);
        if free_scale <= scale * 1.12 {
            // The world centre goes to the middle of the free region,
            // `inset / 2` px right of the viewport centre.
            return (Vec2::new(centre.x - inset / 2.0 * free_scale, centre.y), free_scale);
        }
    }
    // Same size, shifted right: the world's left edge moves towards the
    // card's right edge without leaving the viewport.
    let world_px = width as f32 / scale;
    let left = viewport.center().x - world_px / 2.0;
    let wanted = card.max.x + 8.0 - left;
    let room = (viewport.max.x - 8.0 - (left + world_px)).max(0.0);
    let shift = wanted.clamp(0.0, room);
    (Vec2::new(centre.x - shift * scale, centre.y), scale)
}

/// Mesh of an entity: a unit-diameter disc whose outer ring is darker
/// (vertex colours multiply the material colour); `notch` adds a dark
/// wedge pointing along +x (the heading).
pub fn entity_mesh(notch: bool) -> Mesh {
    const SEGMENTS: usize = 40;
    let (inner, outer) = (0.40f32, 0.5f32);
    let mut positions: Vec<[f32; 3]> = vec![[0.0, 0.0, 0.0]];
    let mut colors: Vec<[f32; 4]> = vec![[1.15, 1.15, 1.15, 1.0]];
    let mut indices: Vec<u32> = Vec::new();
    let body = [1.0, 1.0, 1.0, 1.0];
    let rim = [0.38, 0.38, 0.38, 1.0];
    for i in 0..SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        positions.push([c * inner, s * inner, 0.0]);
        colors.push(body);
        positions.push([c * inner, s * inner, 0.0]);
        colors.push(rim);
        positions.push([c * outer, s * outer, 0.0]);
        colors.push(rim);
    }
    for i in 0..SEGMENTS {
        let j = (i + 1) % SEGMENTS;
        let (bi, bj) = (1 + 3 * i as u32, 1 + 3 * j as u32);
        indices.extend([0, bi, bj]);
        let (ri, rj, oi, oj) = (bi + 1, bj + 1, bi + 2, bj + 2);
        indices.extend([ri, oi, oj, ri, oj, rj]);
    }
    if notch {
        let base = positions.len() as u32;
        let dark = [0.2, 0.2, 0.2, 1.0];
        positions.extend([[0.47, 0.0, 0.0], [0.16, 0.13, 0.0], [0.16, -0.13, 0.0]]);
        colors.extend([dark, dark, dark]);
        indices.extend([base, base + 1, base + 2]);
    }
    let n = positions.len();
    let uvs: Vec<[f32; 2]> = positions.iter().map(|p| [p[0] + 0.5, 0.5 - p[1]]).collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; n])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(indices))
}

/// The field surface frame: one texel per `k × k` cells (`k` from the
/// field's block size), value in B through `[lo, hi]` scaled by
/// `intensity` (the share of the colormap used).
pub fn field_frame(
    field: &FieldView,
    world_w: u32,
    world_h: u32,
    range: (f32, f32),
    intensity: f32,
) -> Option<GridFrame> {
    if field.width == 0 || field.height == 0 || field.values.len() != (field.width * field.height) as usize {
        return None;
    }
    let k = field.cell_width.max(field.cell_height).round().max(1.0) as u32;
    let (tw, th) = (world_w.div_ceil(k).max(1), world_h.div_ceil(k).max(1));
    let mut frame = GridFrame::new(I64Vec2::ZERO, tw, th, k).ok()?;
    let (lo, hi) = range;
    let span = (hi - lo).max(1e-9);
    let intensity = intensity.clamp(0.05, 1.0);
    for ty in 0..th {
        let y = (ty as f32 + 0.5) * k as f32;
        let fy = ((y / field.cell_height).floor() as u32).min(field.height - 1);
        for tx in 0..tw {
            let x = (tx as f32 + 0.5) * k as f32;
            let fx = ((x / field.cell_width).floor() as u32).min(field.width - 1);
            let v = field.values[(fy * field.width + fx) as usize];
            let t = ((v - lo) / span).clamp(0.0, 1.0) * intensity;
            let i = ((ty * tw + tx) * 4) as usize;
            frame.data[i + 2] = encode_unit(t);
            frame.data[i + 3] = 255;
        }
    }
    Some(frame)
}

/// Plain floor frame (no overlay): exact cells up to 512 per side.
pub fn floor_frame(world_w: u32, world_h: u32) -> Option<GridFrame> {
    let k = world_w.max(world_h).div_ceil(512).max(1);
    GridFrame::new(I64Vec2::ZERO, world_w.div_ceil(k).max(1), world_h.div_ceil(k).max(1), k).ok()
}

/// Floor colour of a theme.
pub fn floor_color(palette: &Palette) -> bevy_egui::egui::Color32 {
    if palette.dark { interface::theme::mix(palette.background, palette.surface, 0.55) } else { palette.panel }
}

/// Spawns the camera and the floor surface.
pub fn spawn_world(mut commands: Commands, mut space: ResMut<GridSpace>, mut visuals: ResMut<ClVisuals>) {
    *space = GridSpace::default();
    visuals.clear();
    commands.spawn((
        Name::new("Custom life camera"),
        Camera2d,
        WorldCamera,
        ClCamera,
        ClCameraController::default(),
        Transform::from_xyz(24.0, -24.0, 100.0),
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Custom life field"),
        GridSurface::default(),
        GridAppearance::styled(GridStyle::Monochrome),
        ClFieldSurface,
        Transform::from_xyz(0.0, 0.0, 0.0),
        DespawnOnExit(MODE),
    ));
}

/// Clears the visual bookkeeping on exit (the entities despawn with the mode).
pub fn clear_world(mut visuals: ResMut<ClVisuals>) {
    visuals.clear();
}

/// Keeps the floor / field surface in sync with the view and display options.
pub fn update_field(
    data: Res<ClData>,
    ui: Res<ClUi>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    mut visuals: ResMut<ClVisuals>,
    mut surfaces: Query<(&mut GridSurface, &mut GridAppearance), With<ClFieldSurface>>,
    mut toasts: ResMut<Toasts>,
) {
    let Some(view) = data.view.as_ref() else { return };
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    let floor32 = floor_color(&palette);
    let floor = interface::theme::bevy_color(floor32);
    let field = ui.overlay.as_ref().and_then(|name| view.fields.iter().find(|f| &f.name == name));
    let key = FieldKey {
        seq: field.map(|_| data.view_seq),
        size: (view.width, view.height),
        overlay: ui.overlay.clone(),
        colormap: ui.colormap,
        intensity: (ui.intensity * 1000.0) as u32,
        grid: ui.grid,
        floor: floor32.to_array(),
    };
    if visuals.field_key.as_ref() == Some(&key) {
        return;
    }
    visuals.field_key = Some(key);
    let grid_color = if palette.dark { Color::srgba(1.0, 1.0, 1.0, 0.07) } else { Color::srgba(0.0, 0.0, 0.0, 0.07) };
    let (frame, appearance) = match field {
        Some(f) => {
            let resource_max = if f.resource {
                data.info
                    .as_ref()
                    .and_then(|i| i.running.world.resources.iter().find(|r| r.name == f.name))
                    .map(|r| r.max)
            } else {
                None
            };
            let range = field_range(f.min, f.max, resource_max);
            let appearance = GridAppearance {
                ramp: ui.colormap,
                show_grid: ui.grid,
                grid_color,
                grid_min_pixels: 10.0,
                major_grid_every: 0,
                ..GridAppearance::styled(GridStyle::Heatmap)
            };
            (field_frame(f, view.width, view.height, range, ui.intensity), appearance)
        }
        None => {
            let appearance = GridAppearance {
                palette: GridPalette::two_state(floor, floor),
                background: Some(floor),
                foreground: floor,
                show_grid: ui.grid,
                grid_color,
                grid_min_pixels: 10.0,
                major_grid_every: 10,
                ..GridAppearance::styled(GridStyle::Monochrome)
            };
            (floor_frame(view.width, view.height), appearance)
        }
    };
    for (mut surface, mut current) in &mut surfaces {
        if *current != appearance {
            *current = appearance.clone();
        }
        if let Some(frame) = frame.clone()
            && let Err(e) = surface.submit(frame)
        {
            toasts.error(format!("The field overlay could not be drawn: {e}"));
        }
    }
}

/// Spawns, moves, recolours and despawns entity meshes to match the view.
#[allow(clippy::too_many_arguments)]
pub fn sync_entities(
    mut commands: Commands,
    data: Res<ClData>,
    ui: Res<ClUi>,
    mut visuals: ResMut<ClVisuals>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut drawn: Query<(&mut Transform, &mut MeshMaterial2d<ColorMaterial>, &mut Mesh2d), With<ClEntityVisual>>,
) {
    let Some(view) = data.view.as_ref() else { return };
    let key = DrawnKey { seq: data.view_seq, color_by: ui.color_by, headings: ui.headings, info_rev: data.info_rev };
    if visuals.drawn == Some(key) {
        return;
    }
    visuals.drawn = Some(key);
    let disc = visuals.disc.get_or_insert_with(|| meshes.add(entity_mesh(false))).clone();
    let pointer = visuals.pointer.get_or_insert_with(|| meshes.add(entity_mesh(true))).clone();
    let mobile: Vec<bool> = data
        .info
        .as_ref()
        .map(|i| i.running.entity_types.iter().map(|t| t.movement.is_mobile()).collect())
        .unwrap_or_default();
    let states: Vec<usize> = view.legend.iter().map(|l| l.states.len()).collect();
    let mut alive: HashMap<u64, Entity> = HashMap::with_capacity(view.entities.len());
    for e in &view.entities {
        let t = usize::from(e.type_index);
        let (rgba, variant) = entity_rgba(e.color, ui.color_by, e.energy, e.state, states.get(t).copied().unwrap_or(1));
        let material = match visuals.materials.get(&(e.type_index, variant)) {
            Some((handle, stored)) if *stored == rgba => handle.clone(),
            Some((handle, _)) => {
                let handle = handle.clone();
                if let Some(mut m) = materials.get_mut(&handle) {
                    m.color = bevy_color(rgba);
                }
                visuals.materials.insert((e.type_index, variant), (handle.clone(), rgba));
                handle
            }
            None => {
                let handle = materials.add(ColorMaterial::from_color(bevy_color(rgba)));
                visuals.materials.insert((e.type_index, variant), (handle.clone(), rgba));
                handle
            }
        };
        let notch = ui.headings && mobile.get(t).copied().unwrap_or(false);
        let mesh = if notch { pointer.clone() } else { disc.clone() };
        let size = e.size.max(0.05);
        let transform = Transform {
            translation: Vec3::new(e.position.x, -e.position.y, 1.0 + f32::from(e.type_index) * 0.01),
            rotation: Quat::from_rotation_z(-e.heading),
            scale: Vec3::new(size, size, 1.0),
        };
        match visuals.map.get(&e.id).copied().filter(|entity| drawn.contains(*entity)) {
            Some(entity) => {
                if let Ok((mut tf, mut mat, mut m)) = drawn.get_mut(entity) {
                    tf.set_if_neq(transform);
                    if mat.0 != material {
                        mat.0 = material;
                    }
                    if m.0 != mesh {
                        m.0 = mesh;
                    }
                }
                alive.insert(e.id, entity);
            }
            None => {
                let entity = commands
                    .spawn((
                        ClEntityVisual { id: e.id },
                        Mesh2d(mesh),
                        MeshMaterial2d(material),
                        transform,
                        DespawnOnExit(MODE),
                    ))
                    .id();
                alive.insert(e.id, entity);
            }
        }
    }
    for (id, entity) in visuals.map.drain() {
        if !alive.contains_key(&id) {
            commands.entity(entity).try_despawn();
        }
    }
    visuals.map = alive;
}

/// Camera translation (world), scale (world units per logical px) and the
/// viewport rectangle (logical px), when all are known.
pub fn camera_view(input: &WorldInput, transform: &Transform, projection: &Projection) -> Option<(Vec2, f32, Rect)> {
    let Projection::Orthographic(ortho) = projection else { return None };
    Some((transform.translation.truncate(), ortho.scale, input.viewport?))
}

/// Pointer position in simulation units, when over the world viewport.
pub fn cursor_sim(input: &WorldInput, camera: (Vec2, f32, Rect)) -> Option<Vec2> {
    let cursor = input.cursor?;
    let (pos, scale, viewport) = camera;
    if !viewport.contains(cursor) {
        return None;
    }
    let w = screen_to_world(cursor, pos, scale, viewport);
    Some(Vec2::new(w.x, -w.y))
}

/// Entity of the view under a simulation point (min. 6 px pick radius).
pub fn entity_at(view: &CustomLifeView, p: Vec2, units_per_px: f32) -> Option<u64> {
    let discs = view.entities.iter().map(|e| (e.position, e.size));
    pick(discs, p, 6.0 * units_per_px).map(|i| view.entities[i].id)
}

/// Mode keyboard shortcuts (none clash with the shell's, see UI_SHELL.md §3.3).
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("1 / 2 / 3 / 4", "Select / Spawn / Remove / Paint tool"),
    ("F", "Fit the world in the view"),
    ("Arrow keys", "Pan"),
    ("Wheel", "Zoom about the cursor"),
    ("Right / middle drag", "Pan (left drag on empty space with Select)"),
    ("Delete / Backspace", "Remove the selected entity"),
    ("Esc", "Clear the selection"),
    ("C", "Cycle entity colours: type / energy / state"),
    ("V", "Cycle the field overlay"),
    ("G", "Glyphs on / off"),
    ("L", "Follow the selection on / off"),
    ("E", "Open / close the system editor"),
    ("Ctrl/Cmd+Enter", "Apply the draft (in the editor)"),
];

/// Camera, hover, tools and shortcuts.
#[allow(clippy::too_many_arguments)]
pub fn world_input(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time<Real>>,
    data: Res<ClData>,
    runner: Option<Res<ClRunner>>,
    mut ui: ResMut<ClUi>,
    mut editor: ResMut<ClEditor>,
    camera: Single<(&mut Transform, &mut Projection, &mut ClCameraController), With<ClCamera>>,
) {
    let (mut transform, mut projection, mut ctl) = camera.into_inner();
    let Some(view) = data.view.clone() else { return };
    let Projection::Orthographic(ortho) = &mut *projection else { return };
    let (min_scale, max_scale) = zoom_limits(view.width, view.height);
    let Some(viewport) = input.viewport.filter(|v| v.width() > 1.0 && v.height() > 1.0) else { return };

    if ui.fit_requested {
        ctl.fit_frames = 10;
        ui.fit_requested = false;
    }
    if ctl.fit_frames > 0 {
        ctl.fit_frames -= 1;
        let (centre, scale) = fit_world_in(view.width, view.height, viewport, ui.overlay_card);
        transform.translation.x = centre.x;
        transform.translation.y = centre.y;
        ortho.scale = scale.clamp(min_scale, max_scale.max(scale));
        ctl.target_scale = None;
        ctl.zoom_anchor = None;
    }
    let max_scale = max_scale.max(fit_world(view.width, view.height, viewport.size()).1 * 2.0);
    let dt = time.delta_secs();
    let pos = transform.translation.truncate();
    let scale = ortho.scale;
    let cursor = input.cursor;
    let cursor_world = cursor.map(|c| screen_to_world(c, pos, scale, viewport));

    // ---- zoom -----------------------------------------------------------
    if input.pointer {
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        if lines != 0.0 {
            ctl.fit_frames = 0;
            let target = (ctl.target_scale.unwrap_or(scale) * 1.2f32.powf(-lines)).clamp(min_scale, max_scale);
            ctl.target_scale = Some(target);
            ctl.zoom_anchor = cursor_world;
        }
    }
    if let Some(target) = ctl.target_scale {
        let f = 1.0 - (-dt / 0.07).exp();
        let new_scale = if (target - scale).abs() < target * 0.002 { target } else { scale + (target - scale) * f };
        if let Some(anchor) = ctl.zoom_anchor {
            let t = rendering::camera2d::zoom_about(pos, scale, new_scale, anchor);
            transform.translation.x = t.x;
            transform.translation.y = t.y;
        }
        ortho.scale = new_scale;
        if new_scale == target {
            ctl.target_scale = None;
            ctl.zoom_anchor = None;
        }
    }
    let scale = ortho.scale;
    let pos = transform.translation.truncate();
    let camera = (pos, scale, viewport);

    // ---- pan --------------------------------------------------------------
    let pan_button = buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle);
    let pan_start =
        input.pointer && (buttons.just_pressed(MouseButton::Right) || buttons.just_pressed(MouseButton::Middle));
    if pan_start {
        ctl.pan_from = cursor;
    }
    let left_pan = ctl.left_press.is_some_and(|(_, pans)| pans) && buttons.pressed(MouseButton::Left);
    if (pan_button && ctl.pan_from.is_some()) || left_pan {
        if let (Some(from), Some(now)) = (ctl.pan_from, cursor) {
            let d = (now - from) * scale;
            transform.translation.x -= d.x;
            transform.translation.y += d.y;
            ui.follow = false;
        }
        ctl.pan_from = cursor;
    } else {
        ctl.pan_from = None;
    }
    if input.keyboard {
        let mut dir = Vec2::ZERO;
        for (key, d) in [
            (KeyCode::ArrowLeft, Vec2::NEG_X),
            (KeyCode::ArrowRight, Vec2::X),
            (KeyCode::ArrowUp, Vec2::Y),
            (KeyCode::ArrowDown, Vec2::NEG_Y),
        ] {
            if keys.pressed(key) {
                dir += d;
            }
        }
        if dir != Vec2::ZERO {
            let step = dir.normalize() * 700.0 * scale * dt;
            transform.translation.x += step.x;
            transform.translation.y += step.y;
            ui.follow = false;
        }
    }

    // ---- hover ------------------------------------------------------------
    let pointer_sim = if input.pointer { cursor_sim(&input, camera) } else { None };
    ui.hovered = pointer_sim.and_then(|p| entity_at(&view, p, scale));

    // ---- follow -------------------------------------------------------------
    if ui.follow
        && let Some(e) = ui.selected.and_then(|id| view.entities.iter().find(|e| e.id == id))
    {
        let target = Vec2::new(e.position.x, -e.position.y);
        let f = 1.0 - (-dt / 0.15).exp();
        let p = transform.translation.truncate();
        let n = p + (target - p) * f;
        transform.translation.x = n.x;
        transform.translation.y = n.y;
    }

    // ---- tools ------------------------------------------------------------
    let Some(runner) = runner else { return };
    if input.pointer
        && buttons.just_pressed(MouseButton::Left)
        && let Some(c) = cursor
    {
        let on_entity = ui.hovered.is_some();
        let pans = ui.tool == Tool::Select && !on_entity;
        ctl.left_press = Some((c, pans));
        if pans {
            ctl.pan_from = Some(c);
        }
        if ui.tool == Tool::Paint
            && let Some(p) = pointer_sim
        {
            // One undo step per stroke, then plain commands while dragging.
            runner.send(Control::ExecEdit(Box::new(|_| {})));
            paint(&runner, &ui, p);
            ctl.last_dab = Some(p);
        }
    }
    if ui.tool == Tool::Paint
        && buttons.pressed(MouseButton::Left)
        && ctl.last_dab.is_some()
        && let Some(p) = pointer_sim
        && ctl.last_dab.is_some_and(|last| last.distance(p) >= (ui.paint_radius * 0.5).max(0.5))
    {
        paint(&runner, &ui, p);
        ctl.last_dab = Some(p);
    }
    if buttons.just_released(MouseButton::Left) {
        let press = ctl.left_press.take();
        ctl.last_dab = None;
        let click = press.zip(cursor).is_some_and(|((start, _), now)| start.distance(now) < 4.0);
        if click
            && input.pointer
            && let Some(p) = pointer_sim
        {
            match ui.tool {
                Tool::Select => {
                    ui.selected = ui.hovered;
                    if ui.selected.is_none() {
                        ui.follow = false;
                    }
                }
                Tool::Remove => {
                    if let Some(id) = ui.hovered {
                        runner.edit(CustomLifeCommand::Remove { id });
                        if ui.selected == Some(id) {
                            ui.clear_selection();
                        }
                    }
                }
                Tool::Spawn => spawn(&runner, &ui, p, view.width, view.height),
                Tool::Paint => {}
            }
        }
    }

    // ---- shortcuts ----------------------------------------------------------
    if !input.keyboard {
        return;
    }
    let modifier = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::AltLeft,
        KeyCode::AltRight,
    ]);
    if modifier {
        return;
    }
    for (key, digit) in [(KeyCode::Digit1, 1), (KeyCode::Digit2, 2), (KeyCode::Digit3, 3), (KeyCode::Digit4, 4)] {
        if keys.just_pressed(key)
            && let Some(tool) = Tool::from_digit(digit)
        {
            ui.tool = tool;
        }
    }
    if keys.just_pressed(KeyCode::KeyF) {
        ui.fit_requested = true;
    }
    if keys.just_pressed(KeyCode::Escape) {
        ui.clear_selection();
    }
    if (keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace))
        && let Some(id) = ui.selected
    {
        runner.edit(CustomLifeCommand::Remove { id });
        ui.clear_selection();
    }
    if keys.just_pressed(KeyCode::KeyC) {
        ui.color_by = ui.next_color_by();
    }
    if keys.just_pressed(KeyCode::KeyV) {
        let fields: Vec<String> = view.fields.iter().map(|f| f.name.clone()).collect();
        ui.overlay = ui.next_overlay(&fields);
    }
    if keys.just_pressed(KeyCode::KeyG) {
        ui.glyphs = !ui.glyphs;
    }
    if keys.just_pressed(KeyCode::KeyL) && ui.selected.is_some() {
        ui.follow = !ui.follow;
    }
    if keys.just_pressed(KeyCode::KeyE) {
        if editor.open {
            editor.open = false;
        } else {
            let tab = editor.tab;
            editor.open_at(if tab == EditorTab::Files { EditorTab::Entities } else { tab });
        }
    }
}

fn paint(runner: &ClRunner, ui: &ClUi, p: Vec2) {
    if ui.paint_resource.is_empty() {
        return;
    }
    runner.send(Control::Command(CustomLifeCommand::PaintResource {
        resource: ui.paint_resource.clone(),
        x: p.x,
        y: p.y,
        radius: ui.paint_radius,
        amount: ui.paint_amount,
        mode: ui.paint_mode,
    }));
}

fn spawn(runner: &ClRunner, ui: &ClUi, p: Vec2, width: u32, height: u32) {
    if ui.spawn_type.is_empty() || !inside_world(p, width, height) {
        return;
    }
    let count = ui.spawn_count.clamp(1, 200) as usize;
    let positions = scatter(p, if count == 1 { 0.0 } else { ui.spawn_radius }, count, width, height);
    // One undo step for the whole click.
    runner.send(Control::ExecEdit(Box::new(|_| {})));
    for q in positions {
        runner.send(Control::Command(CustomLifeCommand::Spawn { entity: ui.spawn_type.clone(), x: q.x, y: q.y }));
    }
}

/// World border, selection, hover and brush gizmos.
pub fn draw_gizmos(
    mut gizmos: Gizmos,
    data: Res<ClData>,
    ui: Res<ClUi>,
    input: Res<WorldInput>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    camera: Single<(&Transform, &Projection), With<ClCamera>>,
) {
    let Some(view) = data.view.as_ref() else { return };
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    let accent = interface::theme::bevy_color(palette.accent);
    let (w, h) = (view.width as f32, view.height as f32);
    let border = if palette.dark { Color::srgba(1.0, 1.0, 1.0, 0.35) } else { Color::srgba(0.0, 0.0, 0.0, 0.4) };
    gizmos.rect_2d(Isometry2d::from_translation(Vec2::new(w / 2.0, -h / 2.0)), Vec2::new(w, h), border);
    let at = |p: Vec2| Isometry2d::from_translation(Vec2::new(p.x, -p.y));
    if let Some(e) =
        ui.hovered.filter(|id| Some(*id) != ui.selected).and_then(|id| view.entities.iter().find(|e| e.id == id))
    {
        gizmos.circle_2d(at(e.position), e.size * 0.5 + 0.12, accent.with_alpha(0.55));
    }
    if let Some(e) = ui.selected.and_then(|id| view.entities.iter().find(|e| e.id == id)) {
        gizmos.circle_2d(at(e.position), e.size * 0.5 + 0.18, accent);
        gizmos.circle_2d(at(e.position), e.size * 0.5 + 0.30, accent.with_alpha(0.35));
        let perception = data
            .info
            .as_ref()
            .and_then(|i| i.running.entity_types.get(usize::from(e.type_index)))
            .map(|t| t.perception);
        if let Some(r) = perception {
            gizmos.circle_2d(at(e.position), r, accent.with_alpha(0.22)).resolution(64);
        }
    }
    let (transform, projection) = *camera;
    let brush = match ui.tool {
        Tool::Paint => !ui.paint_resource.is_empty(),
        Tool::Spawn => !ui.spawn_type.is_empty(),
        Tool::Select | Tool::Remove => false,
    };
    if input.pointer
        && brush
        && let Some(camera) = camera_view(&input, transform, projection)
        && let Some(p) = cursor_sim(&input, camera)
    {
        let r = if ui.tool == Tool::Paint {
            ui.paint_radius
        } else if ui.spawn_count > 1 {
            ui.spawn_radius
        } else {
            0.5
        };
        gizmos.circle_2d(at(p), r.max(0.3), accent.with_alpha(0.8)).resolution(48);
    }
}

/// Screen position of a simulation point.
pub fn sim_to_screen(p: Vec2, camera: (Vec2, f32, Rect)) -> Vec2 {
    world_to_screen(Vec2::new(p.x, -p.y), camera.0, camera.1, camera.2)
}

/// A scale bar for the current zoom: `(world units, pixels)`.
pub fn current_scale_bar(scale: f32) -> (f32, f32) {
    scale_bar(scale, 90.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(width: u32, height: u32, cell: f32, values: Vec<f32>) -> FieldView {
        FieldView {
            name: "f".into(),
            resource: true,
            width,
            height,
            cell_width: cell,
            cell_height: cell,
            min: values.iter().copied().fold(f32::INFINITY, f32::min),
            max: values.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            values,
            color: [1.0; 4],
        }
    }

    #[test]
    fn field_frames_encode_the_range_into_the_ramp_share() {
        let f = field(2, 2, 1.0, vec![0.0, 0.5, 1.0, 2.0]);
        let frame = field_frame(&f, 2, 2, (0.0, 1.0), 1.0).unwrap();
        assert_eq!((frame.width, frame.height, frame.cells_per_texel), (2, 2, 1));
        let b: Vec<u8> = frame.data.chunks(4).map(|px| px[2]).collect();
        assert_eq!(b, vec![0, 128, 255, 255], "values above the range saturate");
        let dim = field_frame(&f, 2, 2, (0.0, 1.0), 0.5).unwrap();
        assert_eq!(dim.data[10], 128, "intensity uses only part of the colormap");
        assert!(field_frame(&field(2, 2, 1.0, vec![0.0; 3]), 2, 2, (0.0, 1.0), 1.0).is_none());
    }

    #[test]
    fn downsampled_fields_use_lod_texels() {
        // A 6 × 4 world sampled in 2 × 2 blocks: 3 × 2 samples.
        let f = field(3, 2, 2.0, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        let frame = field_frame(&f, 6, 4, (0.0, 5.0), 1.0).unwrap();
        assert_eq!((frame.width, frame.height, frame.cells_per_texel), (3, 2, 2));
        assert_eq!(frame.data[(3 + 2) * 4 + 2], 255);
        let floor = floor_frame(2048, 100).unwrap();
        assert_eq!((floor.cells_per_texel, floor.width, floor.height), (4, 512, 25));
    }

    #[test]
    fn fit_and_zoom_limits() {
        let (centre, scale) = fit_world(48, 24, Vec2::new(960.0, 540.0));
        assert_eq!(centre, Vec2::new(24.0, -12.0));
        assert!((scale - 48.0 / 864.0).abs() < 1e-6);
        let (lo, hi) = zoom_limits(48, 48);
        assert!(lo < scale && scale < hi);
    }

    #[test]
    fn fitting_keeps_clear_of_the_overlay_card() {
        // A square world in a wide viewport: room on the right of the card.
        let viewport = Rect::new(300.0, 50.0, 1400.0, 850.0);
        let card = Rect::new(312.0, 62.0, 580.0, 300.0);
        let (plain_centre, plain_scale) = fit_world(48, 48, viewport.size());
        let (centre, scale) = fit_world_in(48, 48, viewport, Some(card));
        assert_eq!(scale, plain_scale, "height-limited: same size");
        // The world's left edge is right of the card.
        let left = world_to_screen(Vec2::new(0.0, -24.0), centre, scale, viewport).x;
        assert!(left > card.max.x, "{left} vs {}", card.max.x);
        assert!(centre.x < plain_centre.x);
        assert_eq!(fit_world_in(48, 48, viewport, None), (plain_centre, plain_scale));
        // A wide world would shrink too much: same size, moved right only
        // as far as it stays inside the viewport.
        let (c, s) = fit_world_in(200, 20, viewport, Some(card));
        let (pc, ps) = fit_world(200, 20, viewport.size());
        assert_eq!(s, ps);
        assert!(c.x <= pc.x);
        let right = world_to_screen(Vec2::new(200.0, -10.0), c, s, viewport).x;
        assert!(right <= viewport.max.x - 7.9, "{right}");
        // A narrower viewport: same size, shifted right but not off screen.
        let narrow = Rect::new(300.0, 50.0, 1100.0, 850.0);
        let (c, s) = fit_world_in(48, 48, narrow, Some(card));
        let (pc, ps) = fit_world(48, 48, narrow.size());
        assert_eq!(s, ps);
        assert!(c.x < pc.x);
        let right = world_to_screen(Vec2::new(48.0, -24.0), c, s, narrow).x;
        assert!(right <= narrow.max.x - 7.9, "{right}");
    }

    #[test]
    fn cursor_mapping_round_trips() {
        let viewport = Rect::new(300.0, 50.0, 1300.0, 850.0);
        let camera = (Vec2::new(24.0, -24.0), 0.05, viewport);
        let input =
            WorldInput { pointer: true, keyboard: true, cursor: Some(viewport.center()), viewport: Some(viewport) };
        assert_eq!(cursor_sim(&input, camera), Some(Vec2::new(24.0, 24.0)));
        let p = Vec2::new(10.0, 30.0);
        let s = sim_to_screen(p, camera);
        let back = cursor_sim(&WorldInput { cursor: Some(s), ..input }, camera).unwrap();
        assert!(back.distance(p) < 1e-3);
        let outside = WorldInput { cursor: Some(Vec2::new(10.0, 10.0)), ..input };
        assert_eq!(cursor_sim(&outside, camera), None);
    }

    #[test]
    fn entity_meshes_are_well_formed() {
        for notch in [false, true] {
            let mesh = entity_mesh(notch);
            let n = mesh.count_vertices();
            let Some(Indices::U32(indices)) = mesh.indices() else { panic!("u32 indices") };
            assert_eq!(indices.len() % 3, 0);
            assert!(indices.iter().all(|&i| (i as usize) < n));
        }
    }
}
