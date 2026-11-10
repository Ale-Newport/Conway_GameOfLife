//! The world viewport: the grid camera, the physical-cell surface (the
//! model's Hashlife grid view at the level of detail of the zoom), the
//! macro layer (one texel per tile) blended over it according to the zoom,
//! view requests for the visible region, and gizmos (tile grid, selection,
//! mismatches, tile anatomy, edit previews).

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use interface::{ModeRegistry, Palette, ThemeSettings};
use metapixel::spec::{CellRect, TILE_PITCH};
use metapixel::{AnnotationKind, MetapixelViewRequest, Programming, tile_annotations};
use rendering::grid::{FrameGeometry, GridSpace, VisibleCells, frame_world_rect};
use rendering::palette::{Colormap, GridPalette};
use rendering::{GridAppearance, GridCameraController, GridStyle, GridSurface, WorldInput, shortcut_modifier_held};
use simulation_core::Control;

use crate::MODE;
use crate::geometry::{
    GridShape, REQUEST_MARGIN, ScreenRect, auto_macro_opacity, fit_clear_of, micro_frame, micro_request,
    pixels_per_tile, request_still_serves,
};
use crate::macro_layer::{EditOverlay, LayerMode, MacroSource, colors, live_available, macro_texture};
use crate::session::{MpSession, SimData};
use crate::tools::{Tool, ToolState, stamp_cells};

/// Largest micro texture side requested (texels).
pub const MAX_MICRO_SIDE: u32 = 4096;

/// How the world is displayed (kept across visits to the mode).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ViewSettings {
    /// Which layer is shown.
    pub layer: LayerMode,
    /// What the macro layer shows.
    pub source: MacroSource,
    /// Colormap of the physical cells (density of live cells per texel).
    pub colormap: Colormap,
    /// Lines between tiles.
    pub tile_grid: bool,
    /// Cell grid lines when zoomed in far enough.
    pub cell_grid: bool,
    /// Outline the parts of the inspected tile (rule table, fuses, core).
    pub anatomy: bool,
    /// Outline tiles whose readout differs from the prediction.
    pub mismatches: bool,
    /// Show the overlay's details (legend, scale, readout status).
    pub overlay_details: bool,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            layer: LayerMode::Auto,
            source: MacroSource::Readout,
            colormap: Colormap::Inferno,
            tile_grid: true,
            cell_grid: true,
            anatomy: true,
            mismatches: true,
            overlay_details: true,
        }
    }
}

/// Colormaps offered for the physical cells.
pub const COLORMAPS: [Colormap; 5] =
    [Colormap::Inferno, Colormap::Magma, Colormap::Viridis, Colormap::Plasma, Colormap::Grayscale];

/// What the current zoom shows (recomputed every frame).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct LayerState {
    /// Cells per logical pixel.
    pub cells_per_pixel: f32,
    /// Logical pixels per tile.
    pub pixels_per_tile: f32,
    /// Opacity of the macro layer (0 = physical cells only).
    pub macro_opacity: f32,
}

impl Default for LayerState {
    fn default() -> Self {
        Self { cells_per_pixel: 1.0, pixels_per_tile: TILE_PITCH as f32, macro_opacity: 0.0 }
    }
}

impl LayerState {
    /// Short name of what is on screen.
    pub fn label(&self) -> String {
        match self.macro_opacity {
            o if o <= 0.0 => "Physical cells".to_string(),
            o if o >= 1.0 => "Macro layer".to_string(),
            o => format!("Blend: {:.0}% macro layer over the cells", o * 100.0),
        }
    }
}

/// The view request last sent to the runner.
#[derive(Resource, Default)]
pub struct ViewRequestState {
    /// Last request, `None` after a new session.
    pub last: Option<MetapixelViewRequest>,
}

/// What the camera should frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FitTarget {
    /// The whole grid.
    Grid,
    /// A cell rectangle (inclusive) with a margin.
    Rect(CellRect, f32),
}

/// Automatic framing: on entering and after loading a scenario (and, while
/// the user has not moved the camera, whenever the viewport, the grid or
/// the overlay's size changes), or on request. Framed rectangles are kept
/// clear of the overlay card when possible ([`fit_clear_of`]).
#[derive(Resource, Default)]
pub struct CameraFit {
    /// Keep the grid framed when the viewport changes.
    pub auto: bool,
    pending: Option<FitTarget>,
    last_viewport: Option<Vec2>,
    last_shape: Option<GridShape>,
    last_details: Option<bool>,
    fitted_clear_of: Option<ScreenRect>,
}

/// Whether the overlay card now reaches further right or down than the
/// card a framing kept clear of (it only grows when its content does).
fn card_grew(fitted: Option<ScreenRect>, now: Option<ScreenRect>) -> bool {
    match (fitted, now) {
        (Some(a), Some(b)) => b.max.x > a.max.x + 4.0 || b.max.y > a.max.y + 4.0,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// Where the overlay card was drawn last frame, relative to the world
/// viewport (written by the overlay panel, read when framing).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct OverlayFootprint(pub Option<ScreenRect>);

impl CameraFit {
    /// Frames the grid; `keep` re-frames it on viewport changes until the
    /// user pans or zooms.
    pub fn request(&mut self, keep: bool) {
        self.pending = Some(FitTarget::Grid);
        self.auto = keep;
    }

    /// Frames a cell rectangle once.
    pub fn frame(&mut self, rect: CellRect, margin: f32) {
        self.pending = Some(FitTarget::Rect(rect, margin));
        self.auto = false;
    }
}

/// Marker of the mode's camera.
#[derive(Component)]
pub struct MpCamera;

/// Marker of the physical-cell surface.
#[derive(Component)]
pub struct MicroSurface;

/// The macro layer sprite.
#[derive(Component, Default)]
pub struct MacroSprite {
    shape: Option<GridShape>,
    image_size: Option<(u32, u32)>,
    key: Option<u64>,
}

/// Dark backdrop under the construction.
#[derive(Component)]
pub struct Backdrop;

/// Theme palette of the mode.
pub fn mode_palette(theme: &ThemeSettings, registry: &ModeRegistry) -> Palette {
    Palette::new(theme.kind, registry.accent(MODE))
}

/// Size of the running grid (from the latest view, else the model data).
pub fn current_shape(data: &SimData) -> Option<GridShape> {
    if let Some(m) = data.view.as_ref().and_then(|v| v.macro_grid.as_ref()) {
        return Some(GridShape::new(m.columns, m.rows));
    }
    data.info.as_ref().map(|i| GridShape::new(i.active.width, i.active.height))
}

fn rgb(c: [u8; 3]) -> Color {
    Color::srgb_u8(c[0], c[1], c[2])
}

/// Appearance of the physical-cell surface: the Density style colours each
/// texel by its displayed density (A channel), exact cells included.
pub fn micro_appearance(settings: &ViewSettings) -> GridAppearance {
    GridAppearance {
        style: GridStyle::Density,
        palette: GridPalette::life(),
        ramp: settings.colormap,
        background: Some(rgb(colors::CELL_BACKGROUND)),
        show_grid: settings.cell_grid,
        grid_color: Color::srgba(1.0, 1.0, 1.0, 0.12),
        grid_min_pixels: 8.0,
        major_grid_every: 0,
        ..GridAppearance::styled(GridStyle::Density)
    }
}

fn macro_image(width: u32, height: u32, data: Vec<u8>) -> Image {
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

/// `OnEnter`: camera, cell surface, macro sprite and backdrop.
pub(crate) fn spawn_world(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    settings: Res<ViewSettings>,
    mut requests: ResMut<ViewRequestState>,
    mut fit: ResMut<CameraFit>,
) {
    commands.insert_resource(GridSpace::new(1.0));
    requests.last = None;
    fit.request(true);
    let mut controller = GridCameraController::default();
    controller.min_cells_per_pixel = 1.0 / 48.0;
    controller.max_cells_per_pixel = 4096.0;
    commands.spawn((Name::new("Metapixel camera"), MpCamera, controller, DespawnOnExit(MODE)));
    commands.spawn((
        Name::new("Metapixel backdrop"),
        Backdrop,
        Sprite { color: rgb(colors::CELL_BACKGROUND), custom_size: Some(Vec2::ONE), ..default() },
        Transform::from_xyz(0.0, 0.0, -1.0),
        Visibility::Hidden,
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Physical B3/S23 cells"),
        MicroSurface,
        GridSurface::default(),
        micro_appearance(&settings),
        Transform::from_xyz(0.0, 0.0, 0.0),
        DespawnOnExit(MODE),
    ));
    let image = images.add(macro_image(1, 1, vec![0, 0, 0, 255]));
    commands.spawn((
        Name::new("Macro layer"),
        MacroSprite::default(),
        Sprite { image, custom_size: Some(Vec2::ONE), color: Color::srgba(1.0, 1.0, 1.0, 0.0), ..default() },
        Transform::from_xyz(0.0, 0.0, 1.0),
        Visibility::Hidden,
        DespawnOnExit(MODE),
    ));
}

const CAMERA_KEYS: [KeyCode; 8] = [
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::KeyW,
    KeyCode::KeyA,
    KeyCode::KeyS,
    KeyCode::KeyD,
];

/// Frames the grid or a requested rectangle (clear of the overlay card),
/// follows viewport, grid and overlay size changes while the user has not
/// moved the camera, and keeps the zoom-out limit proportional to the grid.
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame_world(
    input: Res<WorldInput>,
    scroll: Res<AccumulatedMouseScroll>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    data: Res<SimData>,
    settings: Res<ViewSettings>,
    footprint: Res<OverlayFootprint>,
    mut fit: ResMut<CameraFit>,
    camera: Single<(&Camera, &mut GridCameraController), With<MpCamera>>,
) {
    let (camera, mut controller) = camera.into_inner();
    let user_moved = (input.pointer
        && (scroll.delta != Vec2::ZERO || buttons.any_just_pressed([MouseButton::Right, MouseButton::Middle])))
        || (input.keyboard && keys.any_pressed(CAMERA_KEYS) && !shortcut_modifier_held(&keys));
    if user_moved {
        fit.auto = false;
    }
    let Some(shape) = current_shape(&data) else { return };
    let viewport = camera.logical_viewport_size();
    let details = Some(settings.overlay_details);
    if fit.auto
        && viewport.is_some()
        && (viewport != fit.last_viewport
            || Some(shape) != fit.last_shape
            || details != fit.last_details
            || card_grew(fit.fitted_clear_of, footprint.0))
    {
        fit.pending = Some(FitTarget::Grid);
    }
    fit.last_viewport = viewport;
    fit.last_shape = Some(shape);
    fit.last_details = details;
    // Zooming out is limited to the whole grid at about 160 pixels.
    let extent = shape.size_cells().0.max(shape.size_cells().1) as f32;
    let max = (extent / 160.0).max(8.0);
    if controller.max_cells_per_pixel != max {
        controller.max_cells_per_pixel = max;
    }
    let Some(viewport) = viewport.filter(|v| v.x >= 1.0 && v.y >= 1.0) else { return };
    let (target, margin) = match fit.pending.take() {
        Some(FitTarget::Grid) => (shape.bounds(), 0.04),
        Some(FitTarget::Rect(r, margin)) => (r, margin),
        None => return,
    };
    let framed = fit_clear_of(target, viewport, margin * viewport.min_element(), footprint.0);
    fit.fitted_clear_of = footprint.0;
    controller.fit_to_rect(I64Vec2::new(framed.x0, framed.y0), I64Vec2::new(framed.x1, framed.y1), 0.0);
}

/// Zoom-dependent opacity of the macro layer.
pub(crate) fn update_layer_state(
    visible: Res<VisibleCells>,
    settings: Res<ViewSettings>,
    tools: Res<ToolState>,
    mut layer: ResMut<LayerState>,
) {
    let px = pixels_per_tile(visible.cells_per_pixel);
    let mut opacity = settings.layer.macro_opacity(auto_macro_opacity(px));
    // Editing tools show the edited pattern at least faintly over the cells.
    if matches!(tools.tool, Tool::Draw | Tool::Stamp) && settings.layer != LayerMode::Cells {
        opacity = opacity.max(0.45);
    }
    let next = LayerState { cells_per_pixel: visible.cells_per_pixel, pixels_per_tile: px, macro_opacity: opacity };
    layer.set_if_neq(next);
}

/// Sends the view request (visible micro region at the right level of
/// detail, macro grid, live readings, inspected tile) when it changed.
pub(crate) fn request_view(
    session: Option<Res<MpSession>>,
    data: Res<SimData>,
    visible: Res<VisibleCells>,
    layer: Res<LayerState>,
    settings: Res<ViewSettings>,
    tools: Res<ToolState>,
    mut state: ResMut<ViewRequestState>,
) {
    let Some(session) = session else { return };
    let Some(shape) = current_shape(&data) else { return };
    if visible.cells_per_pixel.is_nan() || visible.cells_per_pixel <= 0.0 {
        return;
    }
    let wanted =
        (layer.macro_opacity < 1.0).then(|| micro_request(&visible, shape, REQUEST_MARGIN, MAX_MICRO_SIDE)).flatten();
    let last_micro = state.last.as_ref().and_then(|r| r.micro);
    let micro = match (last_micro, wanted) {
        (Some(last), Some(w)) if request_still_serves(&last, &w, &visible, shape) => Some(last),
        (_, w) => w,
    };
    let request = MetapixelViewRequest {
        micro,
        macro_grid: true,
        live_readings: settings.source == MacroSource::Live && live_available(shape.tile_count()),
        inspect: tools.selected.filter(|&(c, r)| shape.contains(c, r)),
    };
    if state.last.as_ref() != Some(&request) {
        state.last = Some(request.clone());
        session.send(Control::SetViewRequest(request));
    }
}

/// Keeps the cell surface's appearance in step with the settings.
pub(crate) fn sync_appearance(
    settings: Res<ViewSettings>,
    mut surfaces: Query<&mut GridAppearance, With<MicroSurface>>,
) {
    if !settings.is_changed() {
        return;
    }
    let wanted = micro_appearance(&settings);
    for mut current in &mut surfaces {
        current.set_if_neq(wanted.clone());
    }
}

/// Uploads new cell frames.
pub(crate) fn draw_micro(
    data: Res<SimData>,
    layer: Res<LayerState>,
    mut drawn: Local<(u64, u64)>,
    surface: Single<(&mut GridSurface, &mut Visibility), With<MicroSurface>>,
) {
    let (mut surface, mut visibility) = surface.into_inner();
    visibility.set_if_neq(if layer.macro_opacity >= 1.0 { Visibility::Hidden } else { Visibility::Inherited });
    if (data.epoch, data.view_seq) == *drawn {
        return;
    }
    *drawn = (data.epoch, data.view_seq);
    let Some(grid) = data.view.as_ref().and_then(|v| v.micro.as_ref()) else { return };
    match micro_frame(grid) {
        Ok(frame) => {
            if let Err(e) = surface.submit(frame) {
                warn!("metapixel cell frame rejected: {e}");
            }
        }
        Err(e) => warn!("metapixel cell frame rejected: {e}"),
    }
}

/// Rebuilds the macro texture when the view, the source or an edit
/// preview changed, and sets the layer's opacity.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_macro(
    data: Res<SimData>,
    settings: Res<ViewSettings>,
    tools: Res<ToolState>,
    layer: Res<LayerState>,
    mut images: ResMut<Assets<Image>>,
    sprite: Single<(&mut MacroSprite, &mut Sprite, &mut Visibility)>,
    backdrop: Single<&mut Visibility, (With<Backdrop>, Without<MacroSprite>)>,
) {
    let (mut state, mut sprite, mut visibility) = sprite.into_inner();
    let Some(view) = data.view.as_ref().and_then(|v| v.macro_grid.as_ref()) else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    backdrop.into_inner().set_if_neq(Visibility::Inherited);
    let editing = matches!(tools.tool, Tool::Draw | Tool::Stamp);
    let pattern = data.info.as_ref().map(|i| &i.initial).filter(|_| editing);
    let (stroke, target) = tools.stroke.as_ref().map_or((&[][..], false), |s| (&s.tiles[..], s.target));
    let key = {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        (data.epoch, data.view_seq, settings.source, editing, data.info_rev, stroke, target).hash(&mut h);
        h.finish()
    };
    if state.key != Some(key) {
        state.key = Some(key);
        let edit = pattern.map(|p| EditOverlay { pattern: p, stroke, target });
        let bytes = macro_texture(view, settings.source, edit);
        let size = (view.columns, view.rows);
        // A new size needs a new image (sprite materials are cached per image).
        if state.image_size == Some(size) && images.contains(&sprite.image) {
            if let Some(mut image) = images.get_mut(&sprite.image) {
                image.data = Some(bytes);
            }
        } else {
            sprite.image = images.add(macro_image(size.0, size.1, bytes));
            state.image_size = Some(size);
        }
        state.shape = Some(GridShape::new(view.columns, view.rows));
    }
    let alpha = layer.macro_opacity;
    let color = Color::srgba(1.0, 1.0, 1.0, alpha);
    if sprite.color != color {
        sprite.color = color;
    }
    visibility.set_if_neq(if alpha > 0.0 { Visibility::Inherited } else { Visibility::Hidden });
}

/// Places the macro sprite and the backdrop over the grid (follows the
/// floating origin).
pub(crate) fn place_layers(
    space: Res<GridSpace>,
    mut sprites: Query<(&MacroSprite, &mut Sprite, &mut Transform), Without<Backdrop>>,
    mut backdrops: Query<(&mut Sprite, &mut Transform), With<Backdrop>>,
) {
    for (state, mut sprite, mut transform) in &mut sprites {
        let Some(shape) = state.shape else { continue };
        let geometry = FrameGeometry {
            origin: I64Vec2::ZERO,
            width: shape.columns,
            height: shape.rows,
            cells_per_texel: TILE_PITCH as u32,
        };
        let (centre, size) = frame_world_rect(&space, &geometry);
        if sprite.custom_size != Some(size) {
            sprite.custom_size = Some(size);
        }
        let translation = centre.extend(transform.translation.z);
        if transform.translation != translation {
            transform.translation = translation;
        }
        for (mut backdrop, mut t) in &mut backdrops {
            let b = shape.bounds();
            let corner = space.cell_corner_to_world(I64Vec2::new(b.x0, b.y0));
            let far = space.cell_corner_to_world(I64Vec2::new(b.x1 + 1, b.y1 + 1));
            let size = (far - corner).abs();
            if backdrop.custom_size != Some(size) {
                backdrop.custom_size = Some(size);
            }
            let translation = ((corner + far) * 0.5).extend(t.translation.z);
            if t.translation != translation {
                t.translation = translation;
            }
        }
    }
}

/// World centre and size of a cell rectangle.
fn rect_world(space: &GridSpace, r: CellRect) -> (Vec2, Vec2) {
    let a = space.cell_corner_to_world(I64Vec2::new(r.x0, r.y0));
    let b = space.cell_corner_to_world(I64Vec2::new(r.x1.saturating_add(1), r.y1.saturating_add(1)));
    ((a + b) * 0.5, (b - a).abs())
}

fn draw_rect(gizmos: &mut Gizmos, space: &GridSpace, r: CellRect, color: Color) {
    let (centre, size) = rect_world(space, r);
    gizmos.rect_2d(Isometry2d::from_translation(centre), size, color);
}

/// Tile rectangle shrunk by `inset` cells.
fn inset(r: CellRect, inset: i64) -> CellRect {
    CellRect { x0: r.x0 + inset, y0: r.y0 + inset, x1: r.x1 - inset, y1: r.y1 - inset }
}

/// The tile whose parts are outlined: the selected one, else the hovered one.
pub fn anatomy_tile(tools: &ToolState) -> Option<(u32, u32)> {
    tools.selected.or(tools.hover_tile)
}

/// Grid outline, tile lines, hover and selection, mismatches, edit and
/// stamp previews, and the parts of the inspected tile.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_gizmos(
    mut gizmos: Gizmos,
    space: Res<GridSpace>,
    data: Res<SimData>,
    settings: Res<ViewSettings>,
    tools: Res<ToolState>,
    layer: Res<LayerState>,
    visible: Res<VisibleCells>,
) {
    let Some(shape) = current_shape(&data) else { return };
    // The world is always dark, so its markers use fixed colours (the
    // theme's accent is darkened for light panels).
    let accent = rgb(colors::WORLD_ACCENT);
    let px_tile = layer.pixels_per_tile;
    // Construction outline.
    draw_rect(&mut gizmos, &space, shape.nominal(), Color::srgba(0.55, 0.62, 0.75, 0.55));
    // Lines between tiles: faint over the cells, dark gaps over the macro layer.
    if settings.tile_grid
        && px_tile >= 10.0
        && let Some(((c0, c1), (r0, r1))) = shape.visible_tiles(&visible)
    {
        let o = layer.macro_opacity;
        let light = Vec4::new(0.62, 0.70, 0.85, 0.22);
        let dark = Vec4::new(0.04, 0.05, 0.07, 0.9);
        let mixed = light.lerp(dark, o);
        let color = Color::srgba(mixed.x, mixed.y, mixed.z, mixed.w * ((px_tile - 10.0) / 14.0).clamp(0.0, 1.0));
        let n = shape.nominal();
        for c in (c0 + 1)..=c1 {
            let x = TILE_PITCH * i64::from(c);
            let a = space.cell_corner_to_world(I64Vec2::new(x, n.y0));
            let b = space.cell_corner_to_world(I64Vec2::new(x, n.y1 + 1));
            gizmos.line_2d(a, b, color);
        }
        for r in (r0 + 1)..=r1 {
            let y = TILE_PITCH * i64::from(r);
            let a = space.cell_corner_to_world(I64Vec2::new(n.x0, y));
            let b = space.cell_corner_to_world(I64Vec2::new(n.x1 + 1, y));
            gizmos.line_2d(a, b, color);
        }
    }
    let Some(view) = data.view.as_ref() else { return };
    // Readouts that differ from the prediction (or are ambiguous).
    if settings.mismatches
        && let Some(m) = &view.macro_grid
        && m.mismatch_count > 0
    {
        let red = rgb(colors::MISMATCH);
        let pad = (2.0 * layer.cells_per_pixel).ceil() as i64;
        for (i, tile) in m.tiles.iter().enumerate() {
            if tile.mismatch || matches!(tile.exact, Some(metapixel::TileReading::Ambiguous(_))) {
                let (c, r) = (i as u32 % m.columns, i as u32 / m.columns);
                let rect = shape.tile_rect(c, r);
                draw_rect(&mut gizmos, &space, inset(rect, pad.min(200)), red);
                draw_rect(&mut gizmos, &space, inset(rect, (2 * pad).min(400)), red.with_alpha(0.6));
            }
        }
    }
    // Hover and selection. ON tiles are drawn in the accent colour, so
    // every marker pairs a light line with a dark or accent one and stays
    // visible over ON and OFF tiles alike.
    let pad = (1.5 * layer.cells_per_pixel).ceil() as i64;
    let light = Color::srgba(1.0, 1.0, 1.0, 0.85);
    let dark = Color::srgba(0.0, 0.0, 0.0, 0.55);
    if let Some((c, r)) = tools.hover_tile.filter(|&(c, r)| shape.contains(c, r)) {
        let rect = shape.tile_rect(c, r);
        draw_rect(&mut gizmos, &space, inset(rect, pad.min(300)), light.with_alpha(0.7));
        draw_rect(&mut gizmos, &space, inset(rect, (2 * pad).min(400)), dark);
    }
    if let Some((c, r)) = tools.selected.filter(|&(c, r)| shape.contains(c, r)) {
        let rect = shape.tile_rect(c, r);
        draw_rect(&mut gizmos, &space, rect, accent);
        draw_rect(&mut gizmos, &space, inset(rect, (2 * pad).min(400)), light);
        draw_rect(&mut gizmos, &space, inset(rect, (3 * pad).min(500)), dark);
    }
    // Stroke tiles while the macro layer is faint.
    if let Some(stroke) = &tools.stroke {
        for &(c, r) in &stroke.tiles {
            draw_rect(&mut gizmos, &space, inset(shape.tile_rect(c, r), (3 * pad).min(500)), light);
        }
    }
    // Footprint of the stamp at the hovered tile.
    if tools.tool == Tool::Stamp
        && let Some((col, row)) = tools.hover_tile
        && let Some(cells) = stamp_cells(tools.stamp)
    {
        let fits = cells.iter().all(|&(x, y)| shape.contains(col + x as u32, row + y as u32));
        let color = if fits { light } else { rgb(colors::WORLD_BAD) };
        for (x, y) in cells {
            let (c, r) = (col.saturating_add(x as u32), row.saturating_add(y as u32));
            draw_rect(&mut gizmos, &space, inset(shape.tile_rect(c, r), (3 * pad).min(500)), color);
        }
    }
    // Parts of the inspected tile, once the tile is large on screen.
    if settings.anatomy
        && px_tile >= 160.0
        && layer.macro_opacity < 0.95
        && let Some((col, row)) = anatomy_tile(&tools).filter(|&(c, r)| shape.contains(c, r))
        && let Some(info) = &data.info
    {
        draw_anatomy(&mut gizmos, &space, info.active.programming, shape.tile_rect(col, row), px_tile);
    }
}

/// Outlines the verified parts of a tile: the readout core, the display,
/// the 18 rule-table eaters and the 8 fuses (unverified mechanism anchors
/// are listed in the inspector only).
fn draw_anatomy(gizmos: &mut Gizmos, space: &GridSpace, programming: Programming, tile: CellRect, px_tile: f32) {
    let accent = rgb(colors::WORLD_ACCENT);
    let on = rgb(colors::WORLD_GOOD);
    let off = Color::srgba(0.6, 0.65, 0.75, 0.55);
    let removed = rgb(colors::WORLD_BAD);
    let small_visible = px_tile >= 900.0;
    for a in tile_annotations(programming) {
        let rect = a.rect.translated(tile.x0, tile.y0);
        let color = match a.kind {
            AnnotationKind::Readout => accent.with_alpha(0.85),
            // Faint once the small parts show: its left edge runs past the rule table.
            AnnotationKind::Display => Color::srgba(0.75, 0.8, 0.9, if small_visible { 0.22 } else { 0.45 }),
            AnnotationKind::RuleBit { programmed, .. } if small_visible => {
                if programmed {
                    on
                } else {
                    off
                }
            }
            AnnotationKind::Fuse { present, .. } if small_visible => {
                if present {
                    accent
                } else {
                    removed
                }
            }
            _ => continue,
        };
        // Tiny parts get a 2-cell halo so they stay visible.
        let grow = if rect.width() < 16 { 2 } else { 0 };
        let rect = CellRect { x0: rect.x0 - grow, y0: rect.y0 - grow, x1: rect.x1 + grow, y1: rect.y1 + grow };
        draw_rect(gizmos, space, rect, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_labels_say_what_is_on_screen() {
        let mut s = LayerState::default();
        assert_eq!(s.label(), "Physical cells");
        s.macro_opacity = 1.0;
        assert_eq!(s.label(), "Macro layer");
        s.macro_opacity = 0.4;
        assert_eq!(s.label(), "Blend: 40% macro layer over the cells");
    }

    #[test]
    fn camera_fit_requests() {
        let mut fit = CameraFit::default();
        fit.request(true);
        assert!(fit.auto && fit.pending == Some(FitTarget::Grid));
        let r = CellRect { x0: 0, y0: 0, x1: 9, y1: 9 };
        fit.frame(r, 0.1);
        assert!(!fit.auto && fit.pending == Some(FitTarget::Rect(r, 0.1)));
    }

    #[test]
    fn micro_appearance_follows_settings() {
        let settings = ViewSettings { colormap: Colormap::Viridis, cell_grid: false, ..ViewSettings::default() };
        let a = micro_appearance(&settings);
        assert_eq!(a.style, GridStyle::Density);
        assert_eq!(a.ramp, Colormap::Viridis);
        assert!(!a.show_grid);
        assert_eq!(a.resolved_background(), rgb(colors::CELL_BACKGROUND));
    }

    #[test]
    fn framing_follows_a_growing_overlay_card_only() {
        let card = |w: f32, h: f32| Some(ScreenRect { min: Vec2::splat(12.0), max: Vec2::new(12.0 + w, 12.0 + h) });
        assert!(card_grew(None, card(300.0, 100.0)), "first framing with a card");
        assert!(!card_grew(card(300.0, 100.0), card(300.0, 102.0)), "a pixel of jitter is ignored");
        assert!(card_grew(card(300.0, 100.0), card(300.0, 140.0)));
        assert!(!card_grew(card(300.0, 140.0), card(300.0, 100.0)), "a smaller card needs no new framing");
        assert!(!card_grew(card(300.0, 100.0), None));
    }

    #[test]
    fn anatomy_prefers_the_selection() {
        let mut tools = ToolState::default();
        tools.hover_tile = Some((1, 1));
        assert_eq!(anatomy_tile(&tools), Some((1, 1)));
        tools.selected = Some((0, 2));
        assert_eq!(anatomy_tile(&tools), Some((0, 2)));
    }

    /// Pan keys pressed with Ctrl, Cmd or Alt are shortcuts that leave the
    /// camera where it is, so the framing keeps following the grid.
    #[test]
    fn shortcut_chords_keep_the_framing() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(WorldInput { keyboard: true, ..default() });
        world.init_resource::<AccumulatedMouseScroll>();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.init_resource::<SimData>();
        world.init_resource::<ViewSettings>();
        world.init_resource::<OverlayFootprint>();
        world.insert_resource(CameraFit { auto: true, ..default() });
        world.spawn((MpCamera, GridCameraController::default()));
        let mut keys = world.resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::ControlLeft);
        keys.press(KeyCode::KeyS);
        world.run_system_once(frame_world).expect("frame_world runs");
        assert!(world.resource::<CameraFit>().auto, "Ctrl+S");
        world.resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::ControlLeft);
        world.run_system_once(frame_world).expect("frame_world runs");
        assert!(!world.resource::<CameraFit>().auto, "S pans");
    }
}
