//! The world viewport: camera, view requests for the visible region, the
//! field surface (grid renderer with a continuous colormap), the RGB
//! composite of multi-channel worlds, and the gizmo overlays (world border,
//! centroid trail, brush and creature previews).

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::math::{DVec2, I64Vec2, Rot2};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use continuous::{ContinuousView, ContinuousViewRequest, Rule, ViewLayer};
use interface::theme::bevy_color;
use interface::{MetricsHistory, ModeRegistry, Palette, ThemeSettings};
use rendering::grid::{FrameGeometry, GridSpace, VisibleCells, frame_world_rect};
use rendering::palette::{Colormap, GridPalette};
use rendering::{GridAppearance, GridCameraController, GridStyle, GridSurface, WorldInput, shortcut_modifier_held};
use simulation_core::Control;

use crate::MODE;
use crate::field::{
    cell_point_to_world, composite_rgba, display_range, inside_world, plane_frame, region_request, request_margin,
    stamp_center, stamp_rotation, trail_segments, world_rect,
};
use crate::session::{Catalog, ContinuousRunner, LoadedWorld};
use crate::tools::{Hover, StampImages, Tool, ToolSettings};

/// Which plane of the requested layer is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DisplayPlane {
    /// Additive RGB composite of every channel (state layer of
    /// multi-channel worlds).
    Composite,
    /// One plane of the view (a channel, a growth, `m` or `n`).
    #[default]
    First,
    /// Plane by index.
    Index(usize),
}

impl DisplayPlane {
    /// Index of the plane to colour, or `None` for the composite.
    pub fn index(self) -> Option<usize> {
        match self {
            DisplayPlane::Composite => None,
            DisplayPlane::First => Some(0),
            DisplayPlane::Index(i) => Some(i),
        }
    }

    /// The selection that is valid for a view of `planes` planes on `layer`:
    /// the composite only exists for the state of multi-channel worlds and
    /// out-of-range indices fall back to the first plane.
    pub fn resolve(self, layer: ViewLayer, planes: usize) -> DisplayPlane {
        match self {
            DisplayPlane::Composite if layer == ViewLayer::State && planes > 1 => DisplayPlane::Composite,
            DisplayPlane::Index(i) if i < planes => DisplayPlane::Index(i),
            _ => DisplayPlane::First,
        }
    }
}

/// How the field is displayed (kept across visits to the mode).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ViewSettings {
    /// Layer requested from the model.
    pub layer: ViewLayer,
    /// Plane of the layer.
    pub plane: DisplayPlane,
    /// Colormap of single planes.
    pub colormap: Colormap,
    /// Colour the shown values from their minimum to maximum instead of the
    /// model's natural range.
    pub auto_contrast: bool,
    /// Bilinear interpolation between cells.
    pub smooth: bool,
    /// Cell grid lines when zoomed in.
    pub grid_lines: bool,
    /// Outline of the torus.
    pub show_border: bool,
    /// Centroid trajectory.
    pub show_trail: bool,
    /// Length of the trajectory in ticks.
    pub trail_ticks: u32,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            layer: ViewLayer::State,
            plane: DisplayPlane::First,
            colormap: Colormap::LeniaBlue,
            auto_contrast: false,
            smooth: true,
            grid_lines: false,
            show_border: true,
            show_trail: false,
            trail_ticks: 2_000,
        }
    }
}

/// Every layer, in display order.
pub const LAYERS: [ViewLayer; 3] = [ViewLayer::State, ViewLayer::Potential, ViewLayer::Growth];

/// Display name of a layer for a model (`potential` is the filling pair in
/// SmoothLife and the affinity is Flow-Lenia's growth).
pub fn layer_label(layer: ViewLayer, rule: &Rule) -> &'static str {
    match (layer, rule) {
        (ViewLayer::State, Rule::FlowLenia(_)) => "Mass",
        (ViewLayer::State, _) => "State",
        (ViewLayer::Potential, Rule::SmoothLife(_)) => "Fillings",
        (ViewLayer::Potential, _) => "Potential",
        (ViewLayer::Growth, Rule::FlowLenia(_)) => "Affinity",
        (ViewLayer::Growth, Rule::SmoothLife(_)) => "Transition",
        (ViewLayer::Growth, _) => "Growth",
    }
}

/// What a layer shows, for tooltips.
pub fn layer_help(layer: ViewLayer, rule: &Rule) -> &'static str {
    match (layer, rule) {
        (ViewLayer::State, Rule::FlowLenia(_)) => "Matter per cell (unbounded, conserved in total).",
        (ViewLayer::State, _) => "Cell states A between 0 and 1, one plane per channel.",
        (ViewLayer::Potential, Rule::SmoothLife(_)) => "Inner filling m (disc) and outer filling n (annulus).",
        (ViewLayer::Potential, _) => "Potential U = K * A of kernel 1: the kernel-weighted neighbourhood average.",
        (ViewLayer::Growth, Rule::FlowLenia(_)) => "Affinity U_j = Σ h·G(K * A); matter flows up its gradient.",
        (ViewLayer::Growth, Rule::SmoothLife(_)) => "2·s(n, m) − 1 of the last step (+1 = becomes alive).",
        (ViewLayer::Growth, _) => "Growth D of the last tick (added times dt): positive grows, negative decays.",
    }
}

/// Default colormap of a model.
pub fn default_colormap(rule: &Rule) -> Colormap {
    match rule {
        Rule::Lenia(_) => Colormap::LeniaBlue,
        Rule::SmoothLife(_) => Colormap::Inferno,
        Rule::FlowLenia(_) => Colormap::Viridis,
    }
}

impl ViewSettings {
    /// Adapts the settings to a newly loaded rule: the composite for
    /// multi-channel worlds, and the model's default colormap when the
    /// model changed.
    pub fn adapt_to(&mut self, rule: &Rule, model_changed: bool) {
        if model_changed {
            self.colormap = default_colormap(rule);
        }
        self.plane = if rule.channels() > 1 && self.layer == ViewLayer::State {
            DisplayPlane::Composite
        } else {
            DisplayPlane::First
        };
    }

    /// Next layer: state, potential, growth (cycling).
    pub fn cycle_layer(&mut self, channels: usize) {
        let i = LAYERS.iter().position(|l| *l == self.layer).unwrap_or(0);
        self.set_layer(LAYERS[(i + 1) % LAYERS.len()], channels);
    }

    /// Switches the layer, choosing the default plane for it.
    pub fn set_layer(&mut self, layer: ViewLayer, channels: usize) {
        self.layer = layer;
        self.plane =
            if layer == ViewLayer::State && channels > 1 { DisplayPlane::Composite } else { DisplayPlane::First };
    }

    /// Next (or previous) colormap.
    pub fn cycle_colormap(&mut self, forward: bool) {
        let all = Colormap::ALL;
        let i = all.iter().position(|c| *c == self.colormap).unwrap_or(0);
        let n = all.len();
        self.colormap = all[if forward { (i + 1) % n } else { (i + n - 1) % n }];
    }
}

/// The most recent view received from the simulation and how it was drawn.
#[derive(Resource, Default)]
pub struct LatestView {
    /// Publication counter of the view.
    pub seq: u64,
    /// The view.
    pub view: Option<Arc<ContinuousView>>,
    /// Plane drawn (after resolving the selection against the view).
    pub plane: DisplayPlane,
    /// Value range used for colouring.
    pub range: (f32, f32),
}

/// The view request last sent to the runner.
#[derive(Resource, Default)]
pub struct ViewRequestState {
    /// Last request, `None` after a new session.
    pub last: Option<ContinuousViewRequest>,
}

/// Automatic framing of the world: on entering, after loading a scenario
/// or resizing the world, on request (F), and while the user has not moved
/// the camera, whenever the viewport changes size.
#[derive(Resource, Default)]
pub struct CameraFit {
    /// Keep the world framed when the viewport changes.
    pub auto: bool,
    /// Frame the world on the next update.
    pub pending: bool,
    last_viewport: Option<Vec2>,
}

impl CameraFit {
    /// Asks for the world to be framed; `keep` re-frames on viewport
    /// changes until the user pans or zooms.
    pub fn request(&mut self, keep: bool) {
        self.pending = true;
        self.auto = keep;
    }
}

/// Marker of the mode's camera.
#[derive(Component)]
pub struct FieldCamera;

/// Marker of the grid surface showing single planes.
#[derive(Component)]
pub struct FieldSurface;

/// The sprite showing the RGB composite and where its frame lies.
#[derive(Component, Default)]
pub struct CompositeSprite {
    geometry: Option<FrameGeometry>,
    /// Size and sampler of the current image.
    image: Option<(u32, u32, bool)>,
}

/// The creature preview of the stamp tool.
#[derive(Component)]
pub struct StampPreview;

fn appearance(settings: &ViewSettings) -> GridAppearance {
    // The History style colours live cells (value > 0) through the palette
    // and dead cells with the background mixed with the trail channel B,
    // which is always 0 here: the result is a plain colormap of R, with
    // bilinear smoothing when enabled. The background equals the colormap
    // at 0, so the two cases join seamlessly.
    GridAppearance {
        style: GridStyle::History,
        palette: GridPalette::Continuous(settings.colormap),
        ramp: settings.colormap,
        background: None,
        show_grid: settings.grid_lines,
        grid_color: Color::srgba(1.0, 1.0, 1.0, 0.16),
        grid_min_pixels: 10.0,
        major_grid_every: 0,
        smooth_fields: settings.smooth,
        ..GridAppearance::styled(GridStyle::History)
    }
}

fn rgba_image(width: u32, height: u32, data: Vec<u8>, smooth: bool) -> Image {
    let mut image = Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = if smooth { ImageSampler::linear() } else { ImageSampler::nearest() };
    image
}

/// `OnEnter`: camera, field surface, composite sprite and stamp preview.
pub(crate) fn spawn_world(mut commands: Commands, mut images: ResMut<Assets<Image>>, settings: Res<ViewSettings>) {
    commands.insert_resource(GridSpace::new(1.0));
    let mut controller = GridCameraController::default();
    controller.min_cells_per_pixel = 1.0 / 64.0;
    controller.max_cells_per_pixel = 16.0;
    commands.spawn((Name::new("Continuous life camera"), FieldCamera, controller, DespawnOnExit(MODE)));
    commands.spawn((
        Name::new("Continuous field"),
        FieldSurface,
        GridSurface::default(),
        appearance(&settings),
        Transform::from_xyz(0.0, 0.0, 0.0),
        DespawnOnExit(MODE),
    ));
    let image = images.add(rgba_image(1, 1, vec![0, 0, 0, 255], settings.smooth));
    commands.spawn((
        Name::new("Continuous RGB composite"),
        CompositeSprite::default(),
        Sprite { image, custom_size: Some(Vec2::ONE), ..default() },
        Transform::from_xyz(0.0, 0.0, 0.1),
        Visibility::Hidden,
        DespawnOnExit(MODE),
    ));
    commands.spawn((
        Name::new("Creature preview"),
        StampPreview,
        Sprite::default(),
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

/// Frames the world when requested; stops following viewport changes once
/// the user pans or zooms.
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame_world(
    input: Res<WorldInput>,
    scroll: Res<AccumulatedMouseScroll>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    loaded: Option<Res<LoadedWorld>>,
    mut fit: ResMut<CameraFit>,
    camera: Single<(&Camera, &mut GridCameraController), With<FieldCamera>>,
) {
    let (camera, mut controller) = camera.into_inner();
    let user_moved = (input.pointer
        && (scroll.delta != Vec2::ZERO || buttons.any_just_pressed([MouseButton::Right, MouseButton::Middle])))
        || (input.keyboard && keys.any_pressed(CAMERA_KEYS) && !shortcut_modifier_held(&keys));
    if user_moved {
        fit.auto = false;
    }
    let viewport = camera.logical_viewport_size();
    if fit.auto && viewport.is_some() && viewport != fit.last_viewport {
        fit.pending = true;
    }
    fit.last_viewport = viewport;
    let Some(world) = loaded else { return };
    if fit.pending && viewport.is_some() {
        fit.pending = false;
        let max = I64Vec2::new(world.width as i64 - 1, world.height as i64 - 1);
        controller.fit_to_rect(I64Vec2::ZERO, max, 0.04);
    }
}

/// Sends the visible region (and layer) to the runner when it changed.
pub(crate) fn request_visible_region(
    runner: Option<Res<ContinuousRunner>>,
    loaded: Option<Res<LoadedWorld>>,
    visible: Res<VisibleCells>,
    settings: Res<ViewSettings>,
    mut state: ResMut<ViewRequestState>,
) {
    let (Some(runner), Some(loaded)) = (runner, loaded) else { return };
    let margin = request_margin(&visible);
    let Some(request) = region_request(&visible, (loaded.width, loaded.height), settings.layer, margin) else { return };
    if state.last != Some(request) {
        state.last = Some(request);
        runner.0.send(Control::SetViewRequest(request));
    }
}

/// Keeps the surface's appearance in step with the settings.
pub(crate) fn sync_appearance(
    settings: Res<ViewSettings>,
    mut surfaces: Query<&mut GridAppearance, With<FieldSurface>>,
) {
    if !settings.is_changed() {
        return;
    }
    let wanted = appearance(&settings);
    for mut current in &mut surfaces {
        current.set_if_neq(wanted.clone());
    }
}

/// The grid surface and its visibility.
type SurfaceQuery<'w, 's> =
    Query<'w, 's, (&'static mut GridSurface, &'static mut Visibility), (With<FieldSurface>, Without<CompositeSprite>)>;

/// Draws the latest published view: one plane through the grid renderer,
/// or the RGB composite on its sprite.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_latest_view(
    runner: Option<Res<ContinuousRunner>>,
    settings: Res<ViewSettings>,
    mut latest: ResMut<LatestView>,
    mut surfaces: SurfaceQuery,
    mut composites: Query<(&mut CompositeSprite, &mut Sprite, &mut Visibility), Without<FieldSurface>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(runner) = runner else { return };
    let fresh = runner.0.latest_view().filter(|p| p.seq != latest.seq);
    if let Some(published) = &fresh {
        latest.seq = published.seq;
        latest.view = Some(published.view.clone());
    } else if !settings.is_changed() {
        return;
    }
    let Some(view) = latest.view.clone() else { return };
    let plane = settings.plane.resolve(view.layer, view.planes.len());
    let shown: Vec<usize> = match plane.index() {
        Some(i) => vec![i],
        None => (0..view.planes.len()).collect(),
    };
    let range = display_range(&view, &shown, settings.auto_contrast);
    latest.plane = plane;
    latest.range = range;

    let composite = plane == DisplayPlane::Composite;
    if let Ok((mut surface, mut visibility)) = surfaces.single_mut() {
        visibility.set_if_neq(if composite { Visibility::Hidden } else { Visibility::Inherited });
        if !composite
            && let Some(frame) = plane.index().and_then(|i| plane_frame(&view, i, range))
            && let Err(e) = surface.submit(frame)
        {
            warn!("continuous field frame rejected: {e}");
        }
    }
    if let Ok((mut sprite_state, mut sprite, mut visibility)) = composites.single_mut() {
        visibility.set_if_neq(if composite { Visibility::Inherited } else { Visibility::Hidden });
        if composite {
            let (w, h) = (view.cols as u32, view.rows as u32);
            let data = composite_rgba(&view.planes, view.cols * view.rows, range);
            let wanted = Some((w, h, settings.smooth));
            // The sprite's cached material keeps the texture and sampler it was
            // built with, so a new size or sampler needs a new image (and with
            // it a new material); same-size updates are written in place.
            let reuse = sprite_state.image == wanted && images.contains(&sprite.image);
            if reuse {
                if let Some(mut image) = images.get_mut(&sprite.image) {
                    image.data = Some(data);
                }
            } else {
                sprite.image = images.add(rgba_image(w, h, data, settings.smooth));
                sprite_state.image = wanted;
            }
            sprite_state.geometry = Some(FrameGeometry {
                origin: I64Vec2::new(view.x0 as i64, view.y0 as i64),
                width: w,
                height: h,
                cells_per_texel: view.cells_per_sample as u32,
            });
        }
    }
}

/// Places the composite sprite over its frame (follows re-anchoring).
pub(crate) fn place_composite(
    space: Res<GridSpace>,
    mut composites: Query<(&CompositeSprite, &mut Sprite, &mut Transform)>,
) {
    for (state, mut sprite, mut transform) in &mut composites {
        let Some(geometry) = state.geometry else { continue };
        let (centre, size) = frame_world_rect(&space, &geometry);
        if sprite.custom_size != Some(size) {
            sprite.custom_size = Some(size);
        }
        let translation = centre.extend(transform.translation.z);
        if transform.translation != translation {
            transform.translation = translation;
        }
    }
}

/// Theme palette of the mode.
pub fn mode_palette(theme: &ThemeSettings, registry: &ModeRegistry) -> Palette {
    Palette::new(theme.kind, registry.accent(MODE))
}

/// World border, centroid trail and the brush / stamp outlines.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_gizmos(
    mut gizmos: Gizmos,
    space: Res<GridSpace>,
    loaded: Option<Res<LoadedWorld>>,
    settings: Res<ViewSettings>,
    tools: Res<ToolSettings>,
    hover: Res<Hover>,
    history: Res<MetricsHistory>,
    catalog: Res<Catalog>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
) {
    let Some(world) = loaded else { return };
    let palette = mode_palette(&theme, &registry);
    let accent = bevy_color(palette.accent);
    let (w, h) = (world.width, world.height);
    if settings.show_border {
        let (centre, size) = world_rect(&space, w, h);
        gizmos.rect_2d(Isometry2d::from_translation(centre), size, bevy_color(palette.stroke).with_alpha(0.9));
    }
    if settings.show_trail {
        draw_trail(&mut gizmos, &space, &history, (w, h), settings.trail_ticks, accent);
    }
    let Some(cursor) = hover.position.filter(|p| inside_world(*p, w, h)) else { return };
    let s = space.cell_size;
    match tools.tool {
        Tool::Brush | Tool::Erase => {
            let centre = cell_point_to_world(&space, cursor);
            let color = if tools.tool == Tool::Erase { bevy_color(palette.error) } else { accent };
            let radius = tools.brush_radius * s;
            gizmos.circle_2d(Isometry2d::from_translation(centre), radius, color).resolution(64);
            let inner = (1.0 - tools.brush_softness) * radius;
            if tools.brush_softness > 0.05 && inner > 0.5 * s {
                gizmos.circle_2d(Isometry2d::from_translation(centre), inner, color.with_alpha(0.45)).resolution(48);
            }
        }
        Tool::Stamp => {
            if let Some(creature) = catalog.creatures.get(tools.creature) {
                let size = (creature.cells.width(), creature.cells.height());
                let centre = stamp_center(cursor, size, tools.rotation_deg, tools.scale);
                let world_centre = cell_point_to_world(&space, centre);
                let extent = Vec2::new(size.0 as f32, size.1 as f32) * tools.scale * s;
                let rotation = Rot2::radians(-tools.rotation_deg.to_radians());
                gizmos.rect_2d(Isometry2d::new(world_centre, rotation), extent, accent);
                // Heading marker: the creature's +x axis.
                let tip = world_centre + rotation * Vec2::new(extent.x * 0.5 + 2.0 * s, 0.0);
                gizmos.line_2d(world_centre + rotation * Vec2::new(extent.x * 0.5, 0.0), tip, accent.with_alpha(0.6));
            }
        }
        Tool::Inspect => {
            let cell = DVec2::new(cursor.x.floor(), cursor.y.floor());
            let centre = cell_point_to_world(&space, cell + DVec2::splat(0.5));
            gizmos.rect_2d(Isometry2d::from_translation(centre), Vec2::splat(s), accent);
        }
    }
}

fn draw_trail(
    gizmos: &mut Gizmos,
    space: &GridSpace,
    history: &MetricsHistory,
    world: (usize, usize),
    ticks: u32,
    color: Color,
) {
    let store = history.store();
    let (Some(xs), Some(ys)) = (store.get("centroid_x"), store.get("centroid_y")) else { return };
    let Some(last) = xs.last() else { return };
    let start = last.tick.saturating_sub(u64::from(ticks));
    let points: Vec<DVec2> = xs
        .window(start, last.tick)
        .iter()
        .filter_map(|p| ys.value_at(p.tick).map(|y| DVec2::new(p.value, y)))
        // The centroid is reported as (0, 0) while undefined (empty world).
        .filter(|p| *p != DVec2::ZERO)
        .collect();
    let total = points.len().max(1) as f32;
    let mut index = 0usize;
    for segment in trail_segments(&points, world) {
        let colored: Vec<(Vec2, Color)> = segment
            .iter()
            .map(|p| {
                index += 1;
                let age = index as f32 / total;
                (cell_point_to_world(space, *p), color.with_alpha(0.15 + 0.85 * age))
            })
            .collect();
        gizmos.linestrip_gradient_2d(colored);
    }
    if let Some(head) = points.last() {
        gizmos.circle_2d(Isometry2d::from_translation(cell_point_to_world(space, *head)), 1.5 * space.cell_size, color);
    }
}

/// Positions and shows the creature preview under the cursor.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_stamp_preview(
    space: Res<GridSpace>,
    loaded: Option<Res<LoadedWorld>>,
    tools: Res<ToolSettings>,
    hover: Res<Hover>,
    settings: Res<ViewSettings>,
    catalog: Res<Catalog>,
    mut stamp_images: ResMut<StampImages>,
    mut images: ResMut<Assets<Image>>,
    preview: Single<(&mut Sprite, &mut Transform, &mut Visibility), With<StampPreview>>,
) {
    let (mut sprite, mut transform, mut visibility) = preview.into_inner();
    let cursor = match (&loaded, hover.position) {
        (Some(world), Some(p)) if tools.tool == Tool::Stamp && inside_world(p, world.width, world.height) => p,
        _ => {
            visibility.set_if_neq(Visibility::Hidden);
            return;
        }
    };
    let Some(creature) = catalog.creatures.get(tools.creature) else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    let handle = stamp_images.image(tools.creature, &creature.cells, settings.colormap, &mut images);
    if sprite.image != handle {
        sprite.image = handle;
    }
    let size = (creature.cells.width(), creature.cells.height());
    let centre = stamp_center(cursor, size, tools.rotation_deg, tools.scale);
    let extent = Vec2::new(size.0 as f32, size.1 as f32) * tools.scale * space.cell_size;
    sprite.custom_size = Some(extent);
    transform.translation = cell_point_to_world(&space, centre).extend(1.0);
    transform.rotation = stamp_rotation(tools.rotation_deg);
    visibility.set_if_neq(Visibility::Inherited);
}

#[cfg(test)]
mod tests {
    use super::*;
    use continuous::scenario::builtin_by_id;

    #[test]
    fn plane_selection_resolves_against_the_view() {
        assert_eq!(DisplayPlane::Composite.resolve(ViewLayer::State, 3), DisplayPlane::Composite);
        assert_eq!(DisplayPlane::Composite.resolve(ViewLayer::State, 1), DisplayPlane::First);
        assert_eq!(DisplayPlane::Composite.resolve(ViewLayer::Growth, 3), DisplayPlane::First);
        assert_eq!(DisplayPlane::Index(2).resolve(ViewLayer::Growth, 3), DisplayPlane::Index(2));
        assert_eq!(DisplayPlane::Index(2).resolve(ViewLayer::Potential, 1), DisplayPlane::First);
        assert_eq!(DisplayPlane::Composite.index(), None);
        assert_eq!(DisplayPlane::First.index(), Some(0));
    }

    #[test]
    fn settings_follow_the_loaded_model() {
        let mut s = ViewSettings { colormap: Colormap::Magma, ..default() };
        let tess = builtin_by_id("lenia.tessellatium_gyrans").unwrap().rule;
        s.adapt_to(&tess, false);
        assert_eq!((s.plane, s.colormap), (DisplayPlane::Composite, Colormap::Magma), "same model keeps the colormap");
        let sl = builtin_by_id("smoothlife.glider").unwrap().rule;
        s.adapt_to(&sl, true);
        assert_eq!((s.plane, s.colormap), (DisplayPlane::First, Colormap::Inferno));
        s.cycle_layer(1);
        assert_eq!(s.layer, ViewLayer::Potential);
        s.cycle_layer(3);
        assert_eq!(s.layer, ViewLayer::Growth);
        s.cycle_layer(3);
        assert_eq!((s.layer, s.plane), (ViewLayer::State, DisplayPlane::Composite));
        s.colormap = Colormap::Viridis;
        s.cycle_colormap(false);
        assert_eq!(s.colormap, Colormap::Grayscale, "wraps backwards");
        s.cycle_colormap(true);
        assert_eq!(s.colormap, Colormap::Viridis);
    }

    #[test]
    fn layer_names_depend_on_the_model() {
        let flow = builtin_by_id("flow_lenia.demo").unwrap().rule;
        let sl = builtin_by_id("smoothlife.glider").unwrap().rule;
        let lenia = builtin_by_id("lenia.orbium").unwrap().rule;
        assert_eq!(layer_label(ViewLayer::Growth, &flow), "Affinity");
        assert_eq!(layer_label(ViewLayer::Potential, &sl), "Fillings");
        assert_eq!(layer_label(ViewLayer::Growth, &lenia), "Growth");
        assert_eq!(default_colormap(&flow), Colormap::Viridis);
    }

    #[test]
    fn surface_appearance_is_a_plain_colormap() {
        let settings = ViewSettings { colormap: Colormap::Magma, smooth: false, grid_lines: true, ..default() };
        let a = appearance(&settings);
        assert_eq!(a.style, GridStyle::History);
        assert_eq!(a.palette, GridPalette::Continuous(Colormap::Magma));
        assert!(a.show_grid && !a.smooth_fields);
        assert_eq!(a.resolved_background(), a.palette.color(0), "background joins the colormap at 0");
    }

    /// Pan keys pressed with Ctrl, Cmd or Alt are shortcuts that leave the
    /// camera where it is, so the framing keeps following the viewport.
    #[test]
    fn shortcut_chords_keep_the_framing() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(WorldInput { keyboard: true, ..default() });
        world.init_resource::<AccumulatedMouseScroll>();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.insert_resource(CameraFit { auto: true, ..default() });
        world.spawn((FieldCamera, GridCameraController::default()));
        let mut keys = world.resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::SuperLeft);
        keys.press(KeyCode::KeyA);
        world.run_system_once(frame_world).expect("frame_world runs");
        assert!(world.resource::<CameraFit>().auto, "Cmd+A");
        world.resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::SuperLeft);
        world.run_system_once(frame_world).expect("frame_world runs");
        assert!(!world.resource::<CameraFit>().auto, "A pans");
    }
}
