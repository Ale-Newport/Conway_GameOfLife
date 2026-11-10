//! The world viewport: the grid camera, the cell surface drawn by the
//! rendering crate's grid renderer from the model's [`GridView`]s,
//! visible-region view requests with level of detail, framing, and the
//! overlays drawn with gizmos (world border, selection, tool previews) and a
//! ghost sprite (the stamp preview).

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use cellular_automata::{CellRect, GridView, Rotation};
use interface::theme::bevy_color;
use interface::{ModeRegistry, Palette, ThemeSettings};
use rendering::grid::{GridSpace, VisibleCells};
use rendering::{GridCameraController, GridFrame, GridSurface};
use simulation_core::Control;

use crate::MODE;
use crate::geometry::{
    RequestPlan, fit_avoiding, framing_rect, ghost_rgba, plan_view_request, rotated_size, stamp_origin,
    transform_pattern,
};
use crate::session::{ConwayRunner, LiveWorld, WorldLoaded, lock};
use crate::style::{ViewSettings, appearance};
use crate::tools::{DragKind, DragState, Hover, Selection, Stamp, Tool, ToolSettings};

/// Marker of the mode's camera.
#[derive(Component)]
pub struct LifeCamera;

/// Marker of the cell surface.
#[derive(Component)]
pub struct LifeSurface;

/// Marker of the translucent selection / drag-rectangle fill.
#[derive(Component)]
pub struct SelectionFill;

/// Marker of the stamp preview sprite.
#[derive(Component, Default)]
pub struct StampGhost {
    key: Option<GhostKey>,
    image: Option<Handle<Image>>,
    size: (u64, u64),
    /// The pattern the image shows, kept alive so that its address (part of
    /// the key) cannot be reused by another pattern.
    keep: Option<Arc<patterns::Pattern>>,
}

/// What the ghost image was built from.
#[derive(Clone, Debug, PartialEq)]
enum GhostKey {
    /// A pattern (by identity) with an orientation.
    Pattern { pattern: usize, rotation: u8, mirror: bool, color: [u8; 4] },
    /// A line or rectangle being dragged.
    Shape { kind: u8, rect: (i64, i64, i64, i64), filled: bool, color: [u8; 4] },
}

/// Largest dragged line or rectangle outline (bounding-box cells) previewed
/// cell by cell; the image is rebuilt whenever the pointer enters another
/// cell, so it stays small (1 MiB). Larger shapes keep the gizmo outline.
const MAX_SHAPE_PREVIEW_AREA: u128 = 1 << 18;

/// The cells of a dragged line or rectangle, as the model will draw them
/// (`geometry::line`, `geometry::rectangle`), relative to the bounding box.
pub fn shape_preview(
    kind: DragKind,
    start: I64Vec2,
    end: I64Vec2,
    filled: bool,
) -> Option<(patterns::Pattern, I64Vec2)> {
    let rect = CellRect::new(start.x, start.y, end.x, end.y);
    if rect.area() > MAX_SHAPE_PREVIEW_AREA {
        return None;
    }
    let cells = match kind {
        DragKind::Line => cellular_automata::geometry::line(start.x, start.y, end.x, end.y).ok()?,
        DragKind::Rectangle => cellular_automata::geometry::rectangle(rect, filled).ok()?,
        _ => return None,
    };
    let origin = I64Vec2::new(rect.x0, rect.y0);
    Some((patterns::Pattern::from_live_cells(cells.into_iter().map(|(x, y)| (x - rect.x0, y - rect.y0))), origin))
}

/// The view request last sent to the runner.
#[derive(Resource, Default, Debug)]
pub struct ViewRequestState {
    /// Last plan, `None` after a new world.
    pub last: Option<RequestPlan>,
}

/// The most recent view received from the simulation.
#[derive(Resource, Default)]
pub struct LatestView {
    /// Publication counter.
    pub seq: u64,
    /// Generation the view shows.
    pub tick: u64,
    /// The view.
    pub view: Option<Arc<GridView>>,
}

impl LatestView {
    /// RGBA texel covering a cell, if the view covers it.
    pub fn texel_at(&self, cell: I64Vec2) -> Option<([u8; 4], u32)> {
        let view = self.view.as_ref()?;
        Some((view.texel_at_cell(cell.x, cell.y)?, view.cells_per_texel))
    }
}

/// The world and the live cells' bounding box read for framing.
type FreshBounds = (cellular_automata::World, Option<CellRect>);

/// Framing requests.
#[derive(Resource, Default)]
pub struct CameraFit {
    /// Frame the world (finite) or the live cells (infinite plane).
    pub world: bool,
    /// Frame this rectangle.
    pub rect: Option<CellRect>,
    fresh: Mutex<Option<Receiver<FreshBounds>>>,
}

impl CameraFit {
    /// Frames the world or the pattern on the next update.
    pub fn request_world(&mut self) {
        self.world = true;
    }

    /// Frames `rect`.
    pub fn request_rect(&mut self, rect: CellRect) {
        self.rect = Some(rect);
    }

    /// Frames the world as it is after the commands sent so far (the
    /// bounding box is read on the simulation thread after them).
    pub fn request_fresh(&mut self, runner: &ConwayRunner) {
        *lock(&self.fresh) = Some(runner.request(|sim| (sim.world(), sim.engine().bounding_box())));
    }

    /// Drops a pending [`Self::request_fresh`] (its world was replaced).
    pub fn forget_fresh(&mut self) {
        *lock(&self.fresh) = None;
    }
}

/// Theme palette of the mode.
pub fn mode_palette(theme: &ThemeSettings, registry: &ModeRegistry) -> Palette {
    Palette::new(theme.kind, registry.accent(MODE))
}

/// `OnEnter`: camera, cell surface and stamp ghost.
pub(crate) fn spawn_world(mut commands: Commands, settings: Res<ViewSettings>) {
    commands.insert_resource(GridSpace::new(1.0));
    let mut controller = GridCameraController::default();
    controller.min_cells_per_pixel = 1.0 / 64.0;
    controller.max_cells_per_pixel = (1u64 << 40) as f32;
    commands.spawn((Name::new("Classic Conway camera"), LifeCamera, controller, DespawnOnExit(MODE)));
    commands.spawn((
        Name::new("Classic Conway cells"),
        LifeSurface,
        GridSurface::default(),
        appearance(&settings),
        Transform::from_xyz(0.0, 0.0, 0.0),
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Classic Conway selection fill"),
        SelectionFill,
        Sprite::from_color(Color::NONE, Vec2::ONE),
        Transform::from_xyz(0.0, 0.0, 0.5),
        Visibility::Hidden,
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Classic Conway stamp preview"),
        StampGhost::default(),
        Sprite::default(),
        Transform::from_xyz(0.0, 0.0, 1.0),
        Visibility::Hidden,
        DespawnOnExit(MODE),
    ));
}

/// The shell binds `=` / `-` (and the numpad keys) to Faster / Slower; the
/// grid camera would also zoom with them. In this mode they keep their shell
/// meaning: they are cleared after the shell's shortcuts ran and before the
/// camera reads them (zoom with the wheel or a pinch).
pub(crate) fn release_speed_keys_from_camera(mut keys: ResMut<ButtonInput<KeyCode>>) {
    for key in [KeyCode::Equal, KeyCode::Minus, KeyCode::NumpadAdd, KeyCode::NumpadSubtract] {
        keys.clear_just_pressed(key);
    }
}

/// Size of the mode's overlay measured from the top-left corner of the world
/// viewport (logical pixels), written by the overlay panel every frame it is
/// drawn. Framing keeps the framed cells out of this corner.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct OverlayFootprint(pub Option<Vec2>);

/// Space kept between the overlay and the framed cells (logical pixels).
pub const OVERLAY_GAP: f32 = 8.0;
/// Frames a load waits for the overlay's size before framing without it.
const FOOTPRINT_WAIT_FRAMES: u32 = 10;

/// Applies framing requests: after a new world was loaded (once its first
/// live information arrived) or its topology or size changed, on request
/// (F), after edits that replaced the world, and for rectangles. Framed
/// cells avoid the overlay in the viewport's top-left corner.
pub(crate) fn frame_camera(
    live: Res<LiveWorld>,
    footprint: Res<OverlayFootprint>,
    mut loaded: ResMut<WorldLoaded>,
    mut fit: ResMut<CameraFit>,
    mut waited: Local<u32>,
    camera: Single<(&Camera, &mut GridCameraController), With<LifeCamera>>,
) {
    let (camera, mut controller) = camera.into_inner();
    let Some(viewport) = camera.logical_viewport_size().filter(|v| v.x >= 2.0 && v.y >= 2.0) else { return };
    let obstacle = footprint.0.map(|o| o + Vec2::splat(OVERLAY_GAP));
    let mut frame = |rect: CellRect, margin: f32| {
        let e = fit_avoiding(rect, viewport, obstacle, margin);
        controller.fit_to_rect(I64Vec2::new(e.x0, e.y0), I64Vec2::new(e.x1, e.y1), 0.0);
    };
    let margin_for = |world: cellular_automata::World| if world.is_finite() { 0.03 } else { 0.08 };
    let fresh = lock(&fit.fresh).as_ref().map(Receiver::try_recv);
    match fresh {
        Some(Ok((world, bbox))) => {
            *lock(&fit.fresh) = None;
            frame(framing_rect(world, bbox), margin_for(world));
        }
        Some(Err(TryRecvError::Disconnected)) => *lock(&fit.fresh) = None,
        Some(Err(TryRecvError::Empty)) | None => {}
    }
    if let Some(info) = live.current()
        && (fit.world || loaded.frame_epoch == Some(live.epoch))
    {
        // The overlay is drawn from the first live information on: wait a
        // few frames for its size so the first framing already avoids it.
        if footprint.0.is_none() && *waited < FOOTPRINT_WAIT_FRAMES {
            *waited += 1;
        } else {
            frame(framing_rect(info.world, info.bbox), margin_for(info.world));
            fit.world = false;
            loaded.frame_epoch = None;
            *waited = 0;
        }
    }
    if let Some(rect) = fit.rect.take() {
        frame(rect, 0.08);
    }
}

/// Sends the visible region (with a margin and a level of detail of about
/// one texel per pixel) to the runner when the camera left the last one.
pub(crate) fn request_visible_region(
    runner: Option<Res<ConwayRunner>>,
    live: Res<LiveWorld>,
    visible: Res<VisibleCells>,
    mut state: ResMut<ViewRequestState>,
) {
    let (Some(runner), Some(info)) = (runner, live.current()) else { return };
    if visible.cells_per_pixel <= 0.0 {
        return;
    }
    if let Some(plan) = plan_view_request(&visible, info.world.bounds(), state.last.as_ref()) {
        runner.send(Control::SetViewRequest(plan.request));
        state.last = Some(plan);
    }
}

/// Keeps the surface's appearance in step with the view settings.
pub(crate) fn sync_appearance(
    settings: Res<ViewSettings>,
    mut surfaces: Query<&mut rendering::GridAppearance, With<LifeSurface>>,
) {
    if !settings.is_changed() {
        return;
    }
    let wanted = appearance(&settings);
    for mut current in &mut surfaces {
        current.set_if_neq(wanted.clone());
    }
}

/// The grid frame of a model view.
pub fn frame_of(view: &GridView) -> GridFrame {
    GridFrame {
        origin: I64Vec2::new(view.origin_x, view.origin_y),
        width: view.width,
        height: view.height,
        cells_per_texel: view.cells_per_texel,
        data: view.rgba.clone(),
    }
}

/// Uploads the latest published view.
pub(crate) fn draw_latest_view(
    runner: Option<Res<ConwayRunner>>,
    mut latest: ResMut<LatestView>,
    mut surfaces: Query<&mut GridSurface, With<LifeSurface>>,
) {
    let Some(runner) = runner else { return };
    let Some(published) = runner.0.runner().latest_view().filter(|p| p.seq != latest.seq) else { return };
    latest.seq = published.seq;
    latest.tick = published.tick;
    for mut surface in &mut surfaces {
        if let Err(e) = surface.submit(frame_of(&published.view)) {
            warn!("Life frame rejected: {e}");
        }
    }
    latest.view = Some(published.view);
}

/// World rectangle (centre, size) of an inclusive cell rectangle.
pub fn cell_rect_world(space: &GridSpace, rect: CellRect) -> (Vec2, Vec2) {
    let a = space.cell_corner_to_world(I64Vec2::new(rect.x0, rect.y0));
    let b = space.cell_corner_to_world(I64Vec2::new(rect.x1, rect.y1).saturating_add(I64Vec2::ONE));
    ((a + b) * 0.5, (b - a).abs())
}

fn outline(gizmos: &mut Gizmos, space: &GridSpace, rect: CellRect, color: Color) {
    let (centre, size) = cell_rect_world(space, rect);
    gizmos.rect_2d(Isometry2d::from_translation(centre), size, color);
}

/// World border, selection, drag previews, brush footprint and stamp outline.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_gizmos(
    mut gizmos: Gizmos,
    space: Res<GridSpace>,
    live: Res<LiveWorld>,
    settings: Res<ViewSettings>,
    tools: Res<ToolSettings>,
    stamp: Res<Stamp>,
    hover: Res<Hover>,
    drag: Res<DragState>,
    selection: Res<Selection>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    visible: Res<VisibleCells>,
) {
    let palette = mode_palette(&theme, &registry);
    let [r, g, b] = settings.colors.highlight();
    let accent = Color::srgb_u8(r, g, b);
    let erase = bevy_color(palette.error);
    if settings.border
        && let Some(bounds) = live.current().and_then(|i| i.world.bounds())
    {
        outline(&mut gizmos, &space, bounds, bevy_color(palette.text_muted).with_alpha(0.85));
    }
    if let Some(rect) = selection.rect {
        let moving = matches!(drag.drag.map(|d| d.kind), Some(DragKind::Move { .. }));
        let color = if moving { accent.with_alpha(0.35) } else { accent };
        outline(&mut gizmos, &space, rect, color);
        // Corner ticks make small selections visible when zoomed out.
        let (centre, size) = cell_rect_world(&space, rect);
        let tick = (visible.cells_per_pixel * 6.0).max(0.3) * space.cell_size;
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let corner = centre + Vec2::new(sx * size.x, sy * size.y) * 0.5;
            gizmos.line_2d(corner, corner - Vec2::new(sx * tick, 0.0), color);
            gizmos.line_2d(corner, corner - Vec2::new(0.0, sy * tick), color);
        }
    }
    if let Some(d) = drag.drag {
        let paint = if tools.shape_erase { erase } else { accent };
        match d.kind {
            DragKind::Line => {
                let a = space.cell_center_to_world(d.start);
                let b = space.cell_center_to_world(d.current);
                gizmos.line_2d(a, b, paint);
                for p in [d.start, d.current] {
                    outline(&mut gizmos, &space, CellRect::point(p.x, p.y), paint);
                }
            }
            DragKind::Rectangle => outline(&mut gizmos, &space, d.rect(), paint),
            DragKind::Fill | DragKind::Select => outline(&mut gizmos, &space, d.rect(), accent),
            DragKind::Move { rect } => {
                let o = d.offset();
                if let Some(dest) = crate::geometry::moved_rect(rect, o.x, o.y) {
                    outline(&mut gizmos, &space, dest, accent);
                }
            }
            DragKind::Stroke { .. } => {}
        }
    }
    let Some(cell) = hover.cell else { return };
    match tools.tool {
        Tool::Draw | Tool::Erase => {
            let color = if tools.tool == Tool::Erase { erase } else { accent };
            if tools.brush_radius == 0 {
                outline(&mut gizmos, &space, CellRect::point(cell.x, cell.y), color);
            } else {
                // Discs are dx² + dy² ≤ r(r + 1): radius √(r(r+1)) + ½ cell.
                let r = tools.brush_radius as f32;
                let radius = ((r * (r + 1.0)).sqrt() + 0.5) * space.cell_size;
                gizmos
                    .circle_2d(Isometry2d::from_translation(space.cell_center_to_world(cell)), radius, color)
                    .resolution(48);
            }
        }
        Tool::Line | Tool::Rectangle | Tool::Fill | Tool::Select if drag.drag.is_none() => {
            outline(&mut gizmos, &space, CellRect::point(cell.x, cell.y), accent.with_alpha(0.7));
        }
        Tool::Stamp => {
            if let Some(s) = &stamp.0 {
                let size = rotated_size(s.pattern.width(), s.pattern.height(), tools.rotation);
                let origin = stamp_origin(cell, size);
                if let Some(rect) = CellRect::from_origin_size(origin.x, origin.y, size.0, size.1) {
                    outline(&mut gizmos, &space, rect, accent.with_alpha(0.8));
                }
            }
        }
        _ => {}
    }
}

fn ghost_image(width: u32, height: u32, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// The translucent fill of the selection (or of the rectangle being dragged).
pub(crate) fn update_selection_fill(
    space: Res<GridSpace>,
    settings: Res<ViewSettings>,
    selection: Res<Selection>,
    drag: Res<DragState>,
    fill: Single<(&mut Sprite, &mut Transform, &mut Visibility), With<SelectionFill>>,
) {
    let (mut sprite, mut transform, mut visibility) = fill.into_inner();
    let dragged = drag.drag.and_then(|d| match d.kind {
        DragKind::Select | DragKind::Fill => Some(d.rect()),
        DragKind::Move { rect } => crate::geometry::moved_rect(rect, d.offset().x, d.offset().y),
        _ => None,
    });
    let Some(rect) = dragged.or(selection.rect) else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    let [r, g, b] = settings.colors.highlight();
    // Live cells under the fill must stay readable (see `selection_alpha`).
    sprite.color = Color::srgba_u8(r, g, b, settings.colors.selection_alpha());
    let (centre, size) = cell_rect_world(&space, rect);
    sprite.custom_size = Some(size);
    transform.translation = centre.extend(0.5);
    visibility.set_if_neq(Visibility::Inherited);
}

/// Positions and shows the ghost: the stamp under the pointer, the cells
/// of a line or rectangle being dragged, or the selection's cells at the
/// destination of a move.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_ghost(
    space: Res<GridSpace>,
    tools: Res<ToolSettings>,
    stamp: Res<Stamp>,
    hover: Res<Hover>,
    drag: Res<DragState>,
    selection: Res<Selection>,
    settings: Res<ViewSettings>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    mut images: ResMut<Assets<Image>>,
    ghost: Single<(&mut StampGhost, &mut Sprite, &mut Transform, &mut Visibility)>,
) {
    let (mut state, mut sprite, mut transform, mut visibility) = ghost.into_inner();
    let [r, g, b] = settings.colors.highlight();
    let paint = [r, g, b, 170];
    let erase = {
        let e = mode_palette(&theme, &registry).error;
        [e.r(), e.g(), e.b(), 170]
    };
    // What to show: a key, a way to build the pattern, and the top-left cell
    // of the shown (shaped) pattern's bounding box.
    enum Source {
        Pattern(Arc<patterns::Pattern>, Rotation, bool, I64Vec2),
        Shape(DragKind, I64Vec2, I64Vec2),
    }
    let source = match drag.drag {
        Some(d) => match d.kind {
            DragKind::Move { rect } => {
                selection.current_info().filter(|i| i.rect == rect).and_then(|i| i.content.clone()).and_then(
                    |content| {
                        let (bx, by, _, _) = content.bounding_box()?;
                        let o = d.offset();
                        let x = rect.x0.checked_add(o.x)?.checked_add(bx)?;
                        let y = rect.y0.checked_add(o.y)?.checked_add(by)?;
                        Some(Source::Pattern(content, Rotation::None, false, I64Vec2::new(x, y)))
                    },
                )
            }
            DragKind::Line | DragKind::Rectangle => Some(Source::Shape(d.kind, d.start, d.current)),
            _ => None,
        },
        None => match (&stamp.0, hover.cell, tools.tool) {
            (Some(s), Some(cell), Tool::Stamp) => {
                let size = rotated_size(s.pattern.width(), s.pattern.height(), tools.rotation);
                Some(Source::Pattern(s.pattern.clone(), tools.rotation, tools.mirror, stamp_origin(cell, size)))
            }
            _ => None,
        },
    };
    let Some(source) = source else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    // A filled rectangle is every cell of its box: a plain coloured sprite,
    // whatever its size.
    if let Source::Shape(DragKind::Rectangle, a, b) = &source
        && tools.rect_filled
    {
        let [r, g, b2, a2] = if tools.shape_erase { erase } else { paint };
        if let Some(old) = state.image.take() {
            images.remove(&old);
        }
        state.key = None;
        state.keep = None;
        sprite.image = Handle::default();
        sprite.color = Color::srgba_u8(r, g, b2, a2);
        let (centre, extent) = cell_rect_world(&space, CellRect::new(a.x, a.y, b.x, b.y));
        sprite.custom_size = Some(extent);
        transform.translation = centre.extend(1.0);
        visibility.set_if_neq(Visibility::Inherited);
        return;
    }
    let (key, origin) = match &source {
        Source::Pattern(p, rotation, mirror, origin) => (
            GhostKey::Pattern {
                pattern: Arc::as_ptr(p) as usize,
                rotation: rotation.quarter_turns(),
                mirror: *mirror,
                color: paint,
            },
            *origin,
        ),
        Source::Shape(kind, a, b) => {
            let color = if tools.shape_erase { erase } else { paint };
            let filled = *kind == DragKind::Rectangle && tools.rect_filled;
            let code = if *kind == DragKind::Line { 0 } else { 1 };
            (GhostKey::Shape { kind: code, rect: (a.x, a.y, b.x, b.y), filled, color }, a.min(*b))
        }
    };
    if state.key.as_ref() != Some(&key) {
        let built = match &source {
            Source::Pattern(p, rotation, mirror, _) => ghost_rgba(&transform_pattern(p, *rotation, *mirror), paint),
            Source::Shape(kind, a, b) => {
                let filled = *kind == DragKind::Rectangle && tools.rect_filled;
                let color = if tools.shape_erase { erase } else { paint };
                shape_preview(*kind, *a, *b, filled).and_then(|(p, _)| ghost_rgba(&p, color))
            }
        };
        let Some((w, h, data)) = built else {
            // Empty or too large for a preview image: the gizmo outline remains.
            visibility.set_if_neq(Visibility::Hidden);
            return;
        };
        if let Some(old) = state.image.take() {
            images.remove(&old);
        }
        let handle = images.add(ghost_image(w, h, data));
        sprite.image = handle.clone();
        sprite.color = Color::WHITE;
        state.image = Some(handle);
        state.keep = match &source {
            Source::Pattern(p, ..) => Some(p.clone()),
            Source::Shape(..) => None,
        };
        state.key = Some(key);
        state.size = (u64::from(w), u64::from(h));
    }
    let Some(rect) = CellRect::from_origin_size(origin.x, origin.y, state.size.0, state.size.1) else { return };
    let (centre, extent) = cell_rect_world(&space, rect);
    sprite.custom_size = Some(extent);
    transform.translation = centre.extend(1.0);
    visibility.set_if_neq(Visibility::Inherited);
}

/// The cell at the centre of the visible region.
pub fn view_centre(visible: &VisibleCells) -> I64Vec2 {
    I64Vec2::new(visible.min_x / 2 + visible.max_x / 2, visible.min_y / 2 + visible.max_y / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_carry_the_view_geometry() {
        let view = GridView { origin_x: -8, origin_y: 4, width: 2, height: 1, cells_per_texel: 4, rgba: vec![1; 8] };
        let frame = frame_of(&view);
        assert_eq!(frame.origin, I64Vec2::new(-8, 4));
        assert_eq!((frame.width, frame.height, frame.cells_per_texel), (2, 1, 4));
        assert!(frame.validate().is_ok());
    }

    #[test]
    fn cell_rectangles_map_to_world_space() {
        let space = GridSpace::new(2.0);
        let (centre, size) = cell_rect_world(&space, CellRect::new(0, 0, 1, 0));
        assert_eq!(size, Vec2::new(4.0, 2.0));
        assert_eq!(centre, Vec2::new(2.0, -1.0), "y grows downward in cells, upward in the world");
        assert_eq!(
            view_centre(&VisibleCells { min_x: -10, min_y: 0, max_x: 10, max_y: 20, cells_per_pixel: 1.0 }),
            I64Vec2::new(0, 10)
        );
    }

    #[test]
    fn shape_previews_match_the_model() {
        let (line, origin) = shape_preview(DragKind::Line, I64Vec2::new(5, 2), I64Vec2::new(0, 0), false).unwrap();
        assert_eq!(origin, I64Vec2::new(0, 0));
        assert_eq!(line.population(), 6, "one cell per step of the major axis");
        let (outline, origin) =
            shape_preview(DragKind::Rectangle, I64Vec2::new(3, 3), I64Vec2::new(6, 5), false).unwrap();
        assert_eq!((origin, outline.population()), (I64Vec2::new(3, 3), 10));
        let (filled, _) = shape_preview(DragKind::Rectangle, I64Vec2::new(3, 3), I64Vec2::new(6, 5), true).unwrap();
        assert_eq!(filled.population(), 12);
        assert!(
            shape_preview(DragKind::Rectangle, I64Vec2::ZERO, I64Vec2::splat(1 << 20), true).is_none(),
            "too large"
        );
        assert!(shape_preview(DragKind::Select, I64Vec2::ZERO, I64Vec2::ONE, false).is_none());
    }

    #[test]
    fn latest_view_reads_texels() {
        let mut latest = LatestView::default();
        assert!(latest.texel_at(I64Vec2::ZERO).is_none());
        latest.view = Some(Arc::new(GridView {
            origin_x: 0,
            origin_y: 0,
            width: 2,
            height: 1,
            cells_per_texel: 1,
            rgba: vec![1, 3, 0, 255, 0, 0, 0, 0],
        }));
        assert_eq!(latest.texel_at(I64Vec2::new(0, 0)), Some(([1, 3, 0, 255], 1)));
        assert_eq!(latest.texel_at(I64Vec2::new(1, 0)), Some(([0, 0, 0, 0], 1)));
        assert!(latest.texel_at(I64Vec2::new(5, 0)).is_none());
    }
}
