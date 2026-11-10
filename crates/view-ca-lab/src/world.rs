//! The world viewport: camera, view requests for the visible region, the
//! grid surface coloured per family, framing and follow modes, and the
//! gizmo overlays (world border, the newest row of a space-time diagram,
//! ant markers and tool previews).

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::math::I64Vec2;
use bevy::prelude::*;
use cellular_automata::ant::Heading;
use cellular_automata::{CellRect, GridView, GridViewRequest};
use interface::theme::bevy_color;
use interface::{ModeRegistry, Palette, ThemeSettings};
use rendering::grid::{GridFrame, GridSpace, VisibleCells};
use rendering::palette::Colormap;
use rendering::{GridAppearance, GridCameraController, GridStyle, GridSurface, WorldInput};
use simulation_core::Control;

use crate::MODE;
use crate::family::{self, Family, Rgb};
use crate::session::{LabRunner, initial_fit};
use crate::sim::{FamilyInfo, LabInfo, LabView};
use crate::tools::{Circuits, Hover, Tool, ToolSettings, ToolState, stamp_footprint};

/// How cells are coloured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Colouring {
    /// State colours.
    #[default]
    States,
    /// State colours over fading trails of recently changed cells (view channel B).
    Trail,
    /// Age of live cells (Life-like rules with age tracking, view channel G).
    Age,
    /// Heat map of recent changes for every cell (view channel B).
    Activity,
}

impl Colouring {
    /// Every colouring, in menu order.
    pub const ALL: [Colouring; 4] = [Colouring::States, Colouring::Trail, Colouring::Age, Colouring::Activity];

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            Colouring::States => "States",
            Colouring::Trail => "States + trail",
            Colouring::Age => "Age",
            Colouring::Activity => "Activity heat map",
        }
    }

    /// Tooltip.
    pub fn help(self) -> &'static str {
        match self {
            Colouring::States => "One colour per state.",
            Colouring::Trail => "State colours; empty cells that changed recently glow and fade.",
            Colouring::Age => "Live cells coloured by how many generations they have been alive (up to 255).",
            Colouring::Activity => "Every cell coloured by how recently it changed: bright = this tick.",
        }
    }

    /// Whether the colouring exists for a family.
    pub fn available(self, family: Family) -> bool {
        match self {
            Colouring::Age => family == Family::LifeLike,
            Colouring::Trail | Colouring::Activity => family != Family::Wireworld,
            Colouring::States => true,
        }
    }

    /// Model parameter that must be on for the colouring to show anything
    /// (`None`: always available).
    pub fn tracking_param(self, family: Family) -> Option<&'static str> {
        match (self, family) {
            (Colouring::States, _) | (_, Family::Wolfram) => None,
            (Colouring::Age, _) => Some("track_ages"),
            _ => Some("track_activity"),
        }
    }

    /// Next colouring available for `family`.
    pub fn next(self, family: Family) -> Colouring {
        let i = Colouring::ALL.iter().position(|c| *c == self).unwrap_or(0);
        (1..=Colouring::ALL.len())
            .map(|k| Colouring::ALL[(i + k) % Colouring::ALL.len()])
            .find(|c| c.available(family))
            .unwrap_or(Colouring::States)
    }
}

/// How the world is displayed (kept across visits to the mode).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ViewSettings {
    /// Cell colouring.
    pub colouring: Colouring,
    /// Cell grid lines when zoomed in.
    pub grid_lines: bool,
    /// Outline of finite worlds.
    pub show_border: bool,
    /// Keep the first ant / the newest row of a diagram in view.
    pub follow: bool,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self { colouring: Colouring::States, grid_lines: true, show_border: true, follow: true }
    }
}

/// The most recent view received from the simulation.
#[derive(Resource, Default)]
pub struct LatestView {
    /// Publication counter.
    pub seq: u64,
    /// The view.
    pub view: Option<Arc<LabView>>,
    /// After a scenario load: its id. Views of the previous model that the
    /// runner published before it processed the replacement are ignored.
    pub awaiting: Option<String>,
    /// Views ignored while awaiting.
    pub(crate) rejected: u32,
}

/// Views of another scenario ignored after a load before they are accepted
/// anyway: the runner publishes at most one or two views of the replaced
/// model, so later ones show the running model (whose scenario may differ,
/// e.g. a save restored right after the load).
pub const MAX_AWAIT_REJECTIONS: u32 = 4;

impl LatestView {
    /// Fresh state that waits for a view of `scenario`.
    pub fn awaiting(scenario: &str) -> Self {
        Self { awaiting: Some(scenario.to_string()), ..Self::default() }
    }

    /// Waits for a view of `scenario` while the previous view stays on
    /// screen (the panels keep their content instead of flashing empty for
    /// the frame or two the new model needs to publish).
    pub fn await_scenario(&mut self, scenario: &str) {
        self.awaiting = Some(scenario.to_string());
        self.rejected = 0;
    }

    /// Whether the view shown still belongs to the model being replaced
    /// (edits made on it would reach the wrong model).
    pub fn is_stale(&self) -> bool {
        self.awaiting.is_some()
    }

    /// Whether a published view may be shown (and clears the wait once it may).
    pub fn accept(&mut self, view: &LabView) -> bool {
        match &self.awaiting {
            Some(id) if *id != view.info.scenario && self.rejected < MAX_AWAIT_REJECTIONS => {
                self.rejected += 1;
                false
            }
            _ => {
                self.awaiting = None;
                self.rejected = 0;
                true
            }
        }
    }
}

impl LatestView {
    /// Summary of the running model, once a view of it arrived.
    pub fn info(&self) -> Option<&LabInfo> {
        self.view.as_deref().map(|v| &v.info)
    }

    /// Family of the running model, once a view arrived.
    pub fn family(&self) -> Option<Family> {
        self.info().map(LabInfo::family)
    }
}

/// The view request last sent to the runner.
#[derive(Resource, Default)]
pub struct ViewRequestState {
    /// Last request (`None` after a new session).
    pub last: Option<GridViewRequest>,
    /// Level of detail (log2 of the cells per texel) the last request asked for.
    pub log2_k: u32,
}

/// What [`CameraFit`] frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitTarget {
    /// A cell rectangle.
    Rect(CellRect),
    /// The top of a space-time diagram `width` cells wide, from row `top`
    /// (the oldest kept row) down, as many rows as fit at that width.
    DiagramTop {
        /// Cells per row.
        width: u32,
        /// Oldest kept row.
        top: i64,
    },
}

/// Framing of the world: after loading a scenario, on request (F), and
/// while the user has not moved the camera, whenever the viewport changes
/// or the overlay grows over the framed area.
#[derive(Resource, Default)]
pub struct CameraFit {
    /// Framed rectangle.
    pub target: Option<FitTarget>,
    /// Frame on the next update.
    pub pending: bool,
    /// Re-frame on viewport changes until the user pans or zooms.
    pub auto: bool,
    /// Ask the simulation for the current extent, then frame it.
    pub refresh: bool,
    last_viewport: Option<Vec2>,
    /// Overlay height the current framing kept clear.
    last_inset: f32,
}

impl CameraFit {
    /// Frames `target` now and on viewport changes.
    pub fn request(&mut self, target: FitTarget) {
        self.target = Some(target);
        self.pending = true;
        self.auto = true;
        self.refresh = false;
    }

    /// Frames the world as it is now (F, the panel button): the live cells
    /// of an infinite plane, the ants, the kept rows of a diagram, or the
    /// finite world, measured on the simulation thread.
    pub fn refit(&mut self) {
        self.refresh = true;
        self.auto = true;
    }

    /// Whether an overlay `inset` pixels tall now covers part of the framed
    /// area: it grew past the height the framing kept clear (the check only
    /// reacts to growth, so a framing that changes the overlay cannot
    /// oscillate).
    pub fn overlay_grew(&self, inset: f32) -> bool {
        self.auto && self.target.is_some() && inset > self.last_inset + 2.0
    }
}

/// The simulation thread's answer to a [`CameraFit::refit`].
#[derive(Resource, Default)]
pub struct FitQuery(Mutex<Option<Receiver<FitTarget>>>);

impl FitQuery {
    /// Forgets a pending answer.
    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Height (logical pixels) of the overlay in the top-left corner of the
/// world viewport, measured by the overlay panel; framing keeps the world
/// below it.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct OverlayInset {
    /// Height including the shell's 12-pixel offset.
    pub height: f32,
}

/// Fraction of the viewport left free on each side when framing.
pub const FIT_MARGIN: f32 = 0.03;

/// Rectangle to frame so that `rect` appears below a band of `top_px`
/// logical pixels at the top of a `viewport` (margin [`FIT_MARGIN`]): the
/// rectangle extended upwards by the cells that band will cover.
pub fn below_band(rect: CellRect, viewport: Vec2, top_px: f32) -> CellRect {
    let usable = viewport * (1.0 - 2.0 * FIT_MARGIN);
    let top = top_px.clamp(0.0, usable.y * 0.5);
    if usable.x <= 0.0 || usable.y - top <= 0.0 {
        return rect;
    }
    let (w, h) = (rect.width() as f32, rect.height() as f32);
    // Cells per pixel after framing.
    let scale = (w / usable.x).max(h / (usable.y - top));
    let extra = (top * scale).ceil() as i64;
    CellRect::new(rect.x0, rect.y0.saturating_sub(extra), rect.x1, rect.y1)
}

/// Cell rectangle framed for `target` in a viewport of `viewport` logical
/// pixels, keeping the target below a band of `top_px` pixels (the overlay).
/// A diagram fills the width; its first row sits just below the band.
pub fn fit_rect(target: FitTarget, viewport: Vec2, top_px: f32) -> CellRect {
    match target {
        FitTarget::Rect(r) => below_band(r, viewport, top_px),
        FitTarget::DiagramTop { width, top: first } => {
            let w = i64::from(width.max(1));
            let usable = viewport * (1.0 - 2.0 * FIT_MARGIN);
            if usable.x <= 0.0 || usable.y <= 0.0 {
                return CellRect::new(0, first, w - 1, first);
            }
            let scale = w as f32 / usable.x;
            let top = top_px.clamp(0.0, usable.y * 0.5);
            let extra = (top * scale).ceil() as i64;
            let rows = ((usable.y * scale).floor() as i64).max(extra + 1);
            CellRect::new(0, first.saturating_sub(extra), w - 1, first.saturating_add(rows - extra - 1))
        }
    }
}

/// Marker of the mode's camera.
#[derive(Component)]
pub struct LabCamera;

/// Marker of the grid surface.
#[derive(Component)]
pub struct LabSurface;

/// State colours of the running model.
pub fn state_colors(info: &LabInfo) -> Vec<Rgb> {
    match &info.detail {
        FamilyInfo::Life { .. } | FamilyInfo::Wolfram { .. } => vec![family::DEAD, family::ALIVE],
        FamilyInfo::Generations { rule, .. } => family::generations_colors(*rule),
        FamilyInfo::Wireworld { .. } => family::WIREWORLD.to_vec(),
        FamilyInfo::Ant { rule, .. } => family::ant_colors(usize::from(rule.colors())),
    }
}

/// Grid appearance for the running model and the view settings.
/// Whether a zoomed-out frame (`cells_per_texel > 1`) of this model is
/// shown as the fraction of live cells per texel (two-state models in the
/// States colouring): "alive if any cell is alive" would turn every busy
/// region into a solid block.
pub fn shows_density(info: &LabInfo, settings: &ViewSettings, cells_per_texel: u32) -> bool {
    cells_per_texel > 1
        && matches!(info.family(), Family::LifeLike | Family::Wolfram)
        && (settings.colouring == Colouring::States || !settings.colouring.available(info.family()))
}

/// Grid appearance for the running model, the view settings and the level
/// of detail of the frame.
pub fn appearance(info: &LabInfo, settings: &ViewSettings, cells_per_texel: u32) -> GridAppearance {
    let colouring = if settings.colouring.available(info.family()) { settings.colouring } else { Colouring::States };
    let (style, ramp) = match colouring {
        _ if shows_density(info, settings, cells_per_texel) => (GridStyle::Density, Colormap::Grayscale),
        Colouring::States => (GridStyle::PixelArt, Colormap::Inferno),
        Colouring::Trail => (GridStyle::History, Colormap::Inferno),
        Colouring::Age => (GridStyle::Age, Colormap::Viridis),
        Colouring::Activity => (GridStyle::Heatmap, Colormap::Inferno),
    };
    let rows = info.family() == Family::Wolfram;
    GridAppearance {
        style,
        palette: family::grid_palette(&state_colors(info)),
        ramp,
        background: None,
        show_grid: settings.grid_lines,
        grid_color: Color::srgba(1.0, 1.0, 1.0, 0.07),
        grid_min_pixels: 12.0,
        major_grid_every: if rows { 0 } else { 10 },
        ..GridAppearance::styled(style)
    }
}

/// Margin (cells) requested around the visible region: a quarter of the
/// visible extent, at least two cells.
pub fn request_margin(visible: &VisibleCells) -> i64 {
    let extent = visible.width().max(visible.height()).min(1 << 40) as i64;
    (extent / 4).max(2)
}

/// Most texels one view may hold (12 MiB of RGBA): beyond it the level of
/// detail is coarsened.
pub const MAX_REQUEST_TEXELS: u64 = 3 << 20;

/// A view request and the cells it must cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionRequest {
    /// The request (visible region plus margin, clipped).
    pub request: GridViewRequest,
    /// Level of detail asked for: log2 of the cells per texel.
    pub log2_k: u32,
    /// The visible cells that exist (visible region, clipped, no margin).
    pub needed: CellRect,
}

/// Texels of a `width` x `height` cell region at `2^log2_k` cells per
/// texel, with one extra texel per side for alignment.
fn texels(width: u64, height: u64, log2_k: u32) -> u64 {
    let k = 1u64 << log2_k.min(62);
    width.div_ceil(k).saturating_add(1).saturating_mul(height.div_ceil(k).saturating_add(1))
}

/// The view request for the visible region (plus a margin): clipped to the
/// world's bounds where the model has them, and to the columns and the
/// kept rows of a space-time diagram, with rows still to come below the
/// newest one so the diagram can grow into them before the next request.
///
/// The texel budget gives about one texel per **physical** pixel
/// (`pixels_per_point` physical pixels per logical pixel), so cells stay
/// sharp on high-density displays down to one cell per physical pixel;
/// views larger than [`MAX_REQUEST_TEXELS`] use a coarser level of detail.
pub fn region_request(visible: &VisibleCells, info: Option<&LabInfo>, pixels_per_point: f32) -> Option<RegionRequest> {
    let cpp = visible.cells_per_pixel;
    if !(cpp.is_finite() && cpp > 0.0) {
        return None;
    }
    let ppp = if pixels_per_point.is_finite() && pixels_per_point >= 1.0 { pixels_per_point } else { 1.0 };
    let margin = request_margin(visible);
    let mut needed = CellRect::new(visible.min_x, visible.min_y, visible.max_x, visible.max_y);
    let mut rect = CellRect::new(
        visible.min_x.saturating_sub(margin),
        visible.min_y.saturating_sub(margin),
        visible.max_x.saturating_add(margin),
        visible.max_y.saturating_add(margin),
    );
    if let Some(info) = info {
        let clip = match &info.detail {
            FamilyInfo::Wolfram { width, first_row, .. } => {
                Some(CellRect::new(0, *first_row as i64, i64::from(*width) - 1, i64::MAX))
            }
            _ => info.bounds(),
        };
        if let Some(clip) = clip {
            rect = rect.intersect(&clip)?;
            needed = needed.intersect(&clip).unwrap_or(rect);
        }
    }
    let cells_per_texel = f64::from(cpp) / f64::from(ppp);
    let mut log2_k = if cells_per_texel <= 1.0 { 0 } else { (cells_per_texel.log2().ceil() as u32).min(31) };
    while log2_k < 31 && texels(rect.width(), rect.height(), log2_k) > MAX_REQUEST_TEXELS {
        log2_k += 1;
    }
    let k = 1u64 << log2_k;
    let side = |cells: u64| (cells.div_ceil(k).saturating_add(1)).clamp(1, 8192) as u32;
    let request = GridViewRequest {
        x0: rect.x0,
        y0: rect.y0,
        x1: rect.x1,
        y1: rect.y1,
        max_width: side(rect.width()),
        max_height: side(rect.height()),
    };
    Some(RegionRequest { request, log2_k, needed })
}

/// Whether the request `last` (level of detail `last_log2_k`) still serves
/// `next`: same level of detail, every needed cell inside it, and not more
/// than four times the area of the new request (after zooming in a lot the
/// old request would make the model build far more texels than shown).
/// Small pans and the growth of a diagram then do not rebuild the view.
pub fn still_serves(last: &GridViewRequest, last_log2_k: u32, next: &RegionRequest) -> bool {
    let rect = last.rect();
    let n = next.needed;
    last_log2_k == next.log2_k
        && rect.contains(n.x0, n.y0)
        && rect.contains(n.x1, n.y1)
        && rect.area() <= next.request.rect().area().saturating_mul(4)
}

/// The part of `grid` covering `bounds` (whole texels), as a frame for the
/// grid renderer; `None` when they do not overlap.
pub fn crop_frame(grid: &GridView, bounds: Option<CellRect>) -> Option<GridFrame> {
    let k = i128::from(grid.cells_per_texel.max(1));
    let (w, h) = (i128::from(grid.width), i128::from(grid.height));
    if w == 0 || h == 0 || grid.rgba.len() as i128 != w * h * 4 {
        return None;
    }
    let (mut tx0, mut ty0, mut tx1, mut ty1) = (0i128, 0i128, w - 1, h - 1);
    if let Some(b) = bounds {
        let (ox, oy) = (i128::from(grid.origin_x), i128::from(grid.origin_y));
        tx0 = tx0.max((i128::from(b.x0) - ox).div_euclid(k));
        tx1 = tx1.min((i128::from(b.x1) - ox).div_euclid(k));
        ty0 = ty0.max((i128::from(b.y0) - oy).div_euclid(k));
        ty1 = ty1.min((i128::from(b.y1) - oy).div_euclid(k));
    }
    if tx0 > tx1 || ty0 > ty1 {
        return None;
    }
    let (cw, ch) = ((tx1 - tx0 + 1) as usize, (ty1 - ty0 + 1) as usize);
    let mut data = Vec::with_capacity(cw * ch * 4);
    for ty in ty0..=ty1 {
        let start = ((ty * w + tx0) * 4) as usize;
        data.extend_from_slice(&grid.rgba[start..start + cw * 4]);
    }
    let origin = I64Vec2::new(
        (i128::from(grid.origin_x) + tx0 * k).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        (i128::from(grid.origin_y) + ty0 * k).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
    );
    Some(GridFrame { origin, width: cw as u32, height: ch as u32, cells_per_texel: k as u32, data })
}

/// Bounds a frame is cropped to: the kept rows up to the newest one of a
/// diagram, the world of finite models.
fn crop_bounds(info: &LabInfo) -> Option<CellRect> {
    info.bounds()
}

/// Corners of an ant marker (a triangle pointing along `heading`) centred
/// on `centre`, `size` world units across. World y points up, cell y down.
pub fn ant_triangle(centre: Vec2, heading: Heading, size: f32) -> [Vec2; 3] {
    let dir = match heading {
        Heading::North => Vec2::Y,
        Heading::East => Vec2::X,
        Heading::South => Vec2::NEG_Y,
        Heading::West => Vec2::NEG_X,
    };
    let perp = Vec2::new(-dir.y, dir.x);
    [
        centre + dir * 0.48 * size,
        centre - dir * 0.36 * size + perp * 0.36 * size,
        centre - dir * 0.36 * size - perp * 0.36 * size,
    ]
}

/// `OnEnter`: camera and grid surface.
pub(crate) fn spawn_world(mut commands: Commands) {
    commands.insert_resource(GridSpace::new(1.0));
    let mut controller = GridCameraController::default();
    controller.min_cells_per_pixel = 1.0 / 64.0;
    commands.spawn((Name::new("CA lab camera"), LabCamera, controller, DespawnOnExit(MODE)));
    commands.spawn((
        Name::new("CA lab cells"),
        LabSurface,
        GridSurface::default(),
        GridAppearance::styled(GridStyle::PixelArt),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Visibility::Hidden,
        DespawnOnExit(MODE),
    ));
}

/// The shell binds `=` / `-` (and the numpad keys) to Faster / Slower; the
/// grid camera also zooms with them. In this mode the shell keeps them.
pub(crate) fn release_speed_keys_from_camera(mut keys: ResMut<ButtonInput<KeyCode>>) {
    for key in [KeyCode::Equal, KeyCode::Minus, KeyCode::NumpadAdd, KeyCode::NumpadSubtract] {
        keys.clear_just_pressed(key);
    }
}

const PAN_KEYS: [KeyCode; 8] = [
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
/// the user zooms or pans. Panning also stops the follow mode.
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame_world(
    input: Res<WorldInput>,
    scroll: Res<AccumulatedMouseScroll>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut fit: ResMut<CameraFit>,
    query: Res<FitQuery>,
    runner: Option<Res<LabRunner>>,
    mut settings: ResMut<ViewSettings>,
    inset: Res<OverlayInset>,
    camera: Single<(&Camera, &mut GridCameraController), With<LabCamera>>,
) {
    let (camera, mut controller) = camera.into_inner();
    // F / "Frame the world": measure the current extent on the simulation thread.
    let mut pending_query = query.0.lock().unwrap_or_else(PoisonError::into_inner);
    if fit.refresh {
        fit.refresh = false;
        match &runner {
            Some(runner) => *pending_query = Some(runner.0.request(|sim| initial_fit(sim))),
            None => fit.pending = fit.target.is_some(),
        }
    }
    match pending_query.as_ref().map(Receiver::try_recv) {
        Some(Ok(target)) => {
            *pending_query = None;
            fit.request(target);
        }
        Some(Err(TryRecvError::Disconnected)) => *pending_query = None,
        Some(Err(TryRecvError::Empty)) | None => {}
    }
    drop(pending_query);
    if fit.overlay_grew(inset.height) {
        fit.pending = true;
    }
    let panned = (input.pointer && buttons.any_just_pressed([MouseButton::Right, MouseButton::Middle]))
        || (input.keyboard && keys.any_pressed(PAN_KEYS));
    if panned || (input.pointer && scroll.delta != Vec2::ZERO) {
        fit.auto = false;
    }
    if panned && settings.follow {
        settings.follow = false;
    }
    let viewport = camera.logical_viewport_size();
    if fit.auto && viewport.is_some() && viewport != fit.last_viewport {
        fit.pending = true;
    }
    fit.last_viewport = viewport;
    if let (true, Some(viewport), Some(target)) = (fit.pending, viewport, fit.target) {
        fit.pending = false;
        fit.last_inset = inset.height;
        let r = fit_rect(target, viewport, inset.height);
        controller.fit_to_rect(I64Vec2::new(r.x0, r.y0), I64Vec2::new(r.x1, r.y1), FIT_MARGIN);
    }
}

/// Follow mode: keeps the first ant inside the middle of the view, or the
/// newest row of a space-time diagram near the bottom of the view.
pub(crate) fn follow_subject(
    settings: Res<ViewSettings>,
    latest: Res<LatestView>,
    mut fit: ResMut<CameraFit>,
    space: Res<GridSpace>,
    visible: Res<VisibleCells>,
    camera: Single<(&Camera, &mut Transform, &GridCameraController), With<LabCamera>>,
) {
    if !settings.follow || fit.pending {
        return;
    }
    let Some(info) = latest.info() else { return };
    let (camera, mut transform, controller) = camera.into_inner();
    if controller.is_dragging() {
        return;
    }
    let Some(viewport) = camera.logical_viewport_size() else { return };
    match &info.detail {
        FamilyInfo::Ant { first, .. } => {
            let Some(ant) = first else { return };
            let (w, h) = (visible.width() as i64, visible.height() as i64);
            let inner = CellRect::new(
                visible.min_x + w / 5,
                visible.min_y + h / 5,
                visible.max_x - w / 5,
                visible.max_y - h / 5,
            );
            if !inner.contains(ant.x, ant.y) {
                let target = space.cell_center_to_world(I64Vec2::new(ant.x, ant.y));
                transform.translation.x = target.x;
                transform.translation.y = target.y;
            }
        }
        FamilyInfo::Wolfram { generation, .. } => {
            // After a reset (or undo) the newest row lies above the view: back to the top.
            if (*generation as i64) < visible.min_y && fit.target.is_some() && !fit.refresh {
                fit.refit();
                return;
            }
            let rows = f64::from(viewport.y) * f64::from(visible.cells_per_pixel);
            let newest = *generation as f64 + 1.0;
            let bottom = visible.max_y as f64 + 1.0;
            // Only scroll once the diagram is taller than 85 % of the view.
            if newest > bottom - 0.1 * rows && newest > 0.85 * rows {
                let top = newest - 0.85 * rows;
                let centre_row = top + rows / 2.0;
                let world_y = space.cell_corner_to_world(I64Vec2::new(0, centre_row.floor() as i64)).y
                    - (centre_row.fract() as f32) * space.cell_size;
                transform.translation.y = world_y;
            }
        }
        _ => {}
    }
}

/// Sends the visible region to the runner when the last request no longer
/// serves it (see [`still_serves`]).
pub(crate) fn request_visible_region(
    runner: Option<Res<LabRunner>>,
    latest: Res<LatestView>,
    visible: Res<VisibleCells>,
    camera: Single<&Camera, With<LabCamera>>,
    mut state: ResMut<ViewRequestState>,
) {
    let Some(runner) = runner else { return };
    let ppp = camera.target_scaling_factor().unwrap_or(1.0);
    let Some(next) = region_request(&visible, latest.info(), ppp) else { return };
    if state.last.is_some_and(|last| still_serves(&last, state.log2_k, &next)) {
        return;
    }
    state.last = Some(next.request);
    state.log2_k = next.log2_k;
    runner.0.send(Control::SetViewRequest(next.request));
}

/// Draws the latest published view on the grid surface.
pub(crate) fn draw_latest_view(
    runner: Option<Res<LabRunner>>,
    settings: Res<ViewSettings>,
    mut latest: ResMut<LatestView>,
    surface: Single<(&mut GridSurface, &mut GridAppearance, &mut Visibility), With<LabSurface>>,
) {
    let (mut surface, mut appearance_now, mut visibility) = surface.into_inner();
    let Some(runner) = runner else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    let fresh = runner.0.latest_view().filter(|p| p.seq != latest.seq);
    // Every publication is judged once (a rejected one is not looked at again).
    let fresh = fresh.filter(|p| {
        latest.seq = p.seq;
        latest.accept(&p.view)
    });
    if let Some(published) = &fresh {
        latest.view = Some(published.view.clone());
    } else if !settings.is_changed() {
        return;
    }
    let Some(view) = latest.view.clone() else { return };
    appearance_now.set_if_neq(appearance(&view.info, &settings, view.grid.cells_per_texel));
    if fresh.is_none() {
        return;
    }
    match crop_frame(&view.grid, crop_bounds(&view.info)) {
        Some(frame) => {
            if let Err(e) = surface.submit(frame) {
                warn!("cellular automata frame rejected: {e}");
            }
            visibility.set_if_neq(Visibility::Inherited);
        }
        None => {
            visibility.set_if_neq(Visibility::Hidden);
        }
    }
}

/// Theme palette of the mode.
pub fn mode_palette(theme: &ThemeSettings, registry: &ModeRegistry) -> Palette {
    Palette::new(theme.kind, registry.accent(MODE))
}

/// World border, the newest diagram row, ant markers and tool previews.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_gizmos(
    mut gizmos: Gizmos,
    space: Res<GridSpace>,
    latest: Res<LatestView>,
    settings: Res<ViewSettings>,
    visible: Res<VisibleCells>,
    tools: Res<ToolSettings>,
    circuits: Res<Circuits>,
    state: Res<ToolState>,
    hover: Res<Hover>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
) {
    let Some(view) = latest.view.as_deref() else { return };
    let info = &view.info;
    let palette = mode_palette(&theme, &registry);
    let accent = bevy_color(palette.accent);
    let s = space.cell_size;
    // World units per logical pixel.
    let px = visible.cells_per_pixel.max(1e-6) * s;
    let rect_of = |r: CellRect| {
        let a = space.cell_corner_to_world(I64Vec2::new(r.x0, r.y0));
        let b = space.cell_corner_to_world(I64Vec2::new(r.x1.saturating_add(1), r.y1.saturating_add(1)));
        ((a + b) * 0.5, (b - a).abs())
    };
    let border = bevy_color(palette.stroke).with_alpha(0.95);
    if let FamilyInfo::Wolfram { width, generation, first_row, .. } = &info.detail {
        // Left, top and right edges of the kept rows, then the newest row in
        // the accent colour: the diagram grows below that line.
        let corner = |x: i64, y: i64| space.cell_corner_to_world(I64Vec2::new(x, y));
        let (x0, x1) = (0, i64::from(*width));
        let (y0, y1) = (*first_row as i64, *generation as i64 + 1);
        if settings.show_border {
            gizmos.linestrip_2d([corner(x0, y1), corner(x0, y0), corner(x1, y0), corner(x1, y1)], border);
        }
        gizmos.line_2d(corner(x0, y1), corner(x1, y1), accent);
        // Thicker on screen: a second line one pixel below.
        gizmos.line_2d(corner(x0, y1) - Vec2::Y * px, corner(x1, y1) - Vec2::Y * px, accent);
    } else if settings.show_border
        && let Some(bounds) = info.bounds()
    {
        let (centre, size) = rect_of(bounds);
        gizmos.rect_2d(Isometry2d::from_translation(centre), size, border);
    }
    // Ants: a ring and a nested triangle along their heading, at least 22 px across.
    let size = (1.2 * s).max(22.0 * px);
    let shadow = bevy_color(palette.background);
    for ant in &view.ants {
        let centre = space.cell_center_to_world(I64Vec2::new(ant.x, ant.y));
        gizmos.circle_2d(Isometry2d::from_translation(centre), size * 0.75, accent.with_alpha(0.55)).resolution(32);
        // A dark outline keeps the marker visible on light cells.
        let [a, b, c] = ant_triangle(centre, ant.heading, size * 1.15);
        gizmos.linestrip_2d([a, b, c, a], shadow);
        for k in [1.0, 0.75, 0.5, 0.25] {
            let [a, b, c] = ant_triangle(centre, ant.heading, size * k);
            gizmos.linestrip_2d([a, b, c, a], accent);
        }
    }
    // Tool previews.
    let Some(cell) = hover.cell else { return };
    let family = info.family();
    let outline = |gizmos: &mut Gizmos, r: CellRect, color: Color| {
        let (centre, size) = rect_of(r);
        gizmos.rect_2d(Isometry2d::from_translation(centre), size, color);
    };
    let erase = bevy_color(palette.error);
    match tools.tool {
        Tool::Draw | Tool::Erase => {
            let color = if tools.tool == Tool::Erase { erase } else { accent };
            let r = i64::from(tools.radius);
            if r == 0 {
                outline(&mut gizmos, CellRect::point(cell.x, cell.y), color);
            } else {
                let centre = space.cell_center_to_world(cell);
                // Discs are dx² + dy² ≤ r(r + 1): radius √(r(r + 1)) + ½ in cell units.
                let radius = (((r * (r + 1)) as f32).sqrt() + 0.5) * s;
                gizmos.circle_2d(Isometry2d::from_translation(centre), radius, color).resolution(48);
            }
        }
        Tool::Line | Tool::Fill => {
            if let Some(start) = state.drag_start {
                if tools.tool == Tool::Line {
                    let a = space.cell_center_to_world(start);
                    let b = space.cell_center_to_world(cell);
                    gizmos.line_2d(a, b, accent);
                    outline(&mut gizmos, CellRect::point(start.x, start.y), accent.with_alpha(0.7));
                }
                outline(&mut gizmos, CellRect::new(start.x, start.y, cell.x, cell.y), accent.with_alpha(0.8));
            } else {
                outline(&mut gizmos, CellRect::point(cell.x, cell.y), accent);
            }
        }
        Tool::Stamp => {
            if let Some(r) = stamp_footprint(&tools, &circuits, cell) {
                outline(&mut gizmos, r, accent);
            }
        }
        Tool::AddAnt => {
            let centre = space.cell_center_to_world(cell);
            let [a, b, c] = ant_triangle(centre, tools.heading, size);
            gizmos.linestrip_2d([a, b, c, a], accent.with_alpha(0.6));
        }
        Tool::RemoveAnts => {
            let r = i64::from(tools.radius.max(1));
            outline(&mut gizmos, CellRect::new(cell.x - r, cell.y - r, cell.x + r, cell.y + r), erase);
        }
        Tool::Toggle => {
            if let FamilyInfo::Wolfram { generation, .. } = &info.detail {
                let color = if cell.y == *generation as i64 { accent } else { bevy_color(palette.text_muted) };
                outline(&mut gizmos, CellRect::point(cell.x, *generation as i64), color);
            }
        }
        Tool::Inspect => {
            if family != Family::Wolfram || info.bounds().is_some_and(|b| b.contains(cell.x, cell.y)) {
                outline(&mut gizmos, CellRect::point(cell.x, cell.y), accent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::FamilyInfo;
    use cellular_automata::World;
    use patterns::LifeRule;
    use simulation_core::Topology;

    fn info(detail: FamilyInfo) -> LabInfo {
        LabInfo { scenario: String::new(), tick: 0, detail, halted: None, headline: None }
    }

    fn life(topology: Topology) -> LabInfo {
        info(FamilyInfo::Life {
            rule: LifeRule::conway(),
            preset: cellular_automata::RulePreset::Conway,
            world: World { topology, width: 100, height: 50 },
            engine: cellular_automata::EngineKind::Dense,
            generations_per_tick: 1,
        })
    }

    #[test]
    fn requests_cover_the_visible_region_and_respect_bounds() {
        let visible = VisibleCells { min_x: -20, min_y: -10, max_x: 79, max_y: 39, cells_per_pixel: 0.5 };
        // Margin: a quarter of 100 = 25 cells.
        let r = region_request(&visible, None, 1.0).expect("request");
        assert_eq!((r.request.x0, r.request.y0, r.request.x1, r.request.y1), (-45, -35, 104, 64));
        assert_eq!((r.request.max_width, r.request.max_height), (151, 101), "one texel per cell, plus one");
        assert_eq!((r.log2_k, r.needed), (0, CellRect::new(-20, -10, 79, 39)));
        let r = region_request(&visible, Some(&life(Topology::Torus)), 1.0).expect("request");
        let q = r.request;
        assert_eq!((q.x0, q.y0, q.x1, q.y1), (0, 0, 99, 49), "clipped to the finite world");
        assert_eq!(r.needed, CellRect::new(0, 0, 79, 39));
        let r = region_request(&visible, Some(&life(Topology::Infinite)), 1.0).expect("request");
        assert_eq!(r.request.x0, -45, "the infinite plane is not clipped");
        // Zoomed out 4 cells per logical pixel: a quarter of the cells per side...
        let far = VisibleCells { cells_per_pixel: 4.0, ..visible };
        let r = region_request(&far, None, 1.0).expect("request");
        assert_eq!((r.log2_k, r.request.max_width), (2, 150u32.div_ceil(4) + 1), "150 cells wide");
        // ... but half on a display with 2 physical pixels per logical pixel.
        let r = region_request(&far, None, 2.0).expect("request");
        assert_eq!((r.log2_k, r.request.max_width), (1, 150u32.div_ceil(2) + 1));
        // Outside a finite world: nothing to request.
        let away = VisibleCells { min_x: 500, max_x: 600, ..visible };
        assert!(region_request(&away, Some(&life(Topology::Bounded)), 1.0).is_none());
        assert!(region_request(&VisibleCells { cells_per_pixel: f32::NAN, ..visible }, None, 1.0).is_none());
    }

    #[test]
    fn requests_stay_within_the_texel_cap() {
        // 4000 x 3000 visible cells at 2 cells per logical pixel on a 2x display:
        // one texel per cell would be 6000 x 4500 texels with the margin.
        let visible = VisibleCells { min_x: 0, min_y: 0, max_x: 3999, max_y: 2999, cells_per_pixel: 2.0 };
        let r = region_request(&visible, None, 2.0).expect("request");
        let q = r.request;
        assert!(r.log2_k >= 1, "coarsened");
        assert!(u64::from(q.max_width) * u64::from(q.max_height) <= MAX_REQUEST_TEXELS);
        // The model picks the smallest power of two that fits the budget: the planned one.
        let plan = cellular_automata::view::TexelGrid::plan(&q, None);
        assert_eq!(plan.cells_per_texel(), 1 << r.log2_k);
    }

    #[test]
    fn small_moves_keep_the_last_request() {
        let visible = VisibleCells { min_x: 0, min_y: 0, max_x: 99, max_y: 49, cells_per_pixel: 0.5 };
        let first = region_request(&visible, None, 1.0).expect("request");
        assert!(still_serves(&first.request, first.log2_k, &first));
        // A pan of 10 cells stays inside the 25-cell margin.
        let panned = VisibleCells { min_x: 10, max_x: 109, ..visible };
        assert!(still_serves(&first.request, first.log2_k, &region_request(&panned, None, 1.0).unwrap()));
        // A pan of 40 cells leaves it.
        let far = VisibleCells { min_x: 40, max_x: 139, ..visible };
        assert!(!still_serves(&first.request, first.log2_k, &region_request(&far, None, 1.0).unwrap()));
        // Another level of detail.
        let zoomed_out = VisibleCells { cells_per_pixel: 3.0, ..visible };
        assert!(!still_serves(&first.request, first.log2_k, &region_request(&zoomed_out, None, 1.0).unwrap()));
        // Zoomed far in: the old request would be much larger than needed.
        let zoomed_in = VisibleCells { min_x: 40, min_y: 20, max_x: 49, max_y: 24, cells_per_pixel: 0.05 };
        assert!(!still_serves(&first.request, first.log2_k, &region_request(&zoomed_in, None, 1.0).unwrap()));
    }

    #[test]
    fn overlay_growth_reframes_once() {
        let mut fit = CameraFit::default();
        assert!(!fit.overlay_grew(300.0), "nothing framed yet");
        fit.request(FitTarget::Rect(CellRect::new(0, 0, 9, 9)));
        fit.last_inset = 200.0;
        assert!(!fit.overlay_grew(201.0) && !fit.overlay_grew(150.0), "small changes and shrinking are ignored");
        assert!(fit.overlay_grew(240.0));
        fit.auto = false;
        assert!(!fit.overlay_grew(240.0), "not after the user moved the camera");
        fit.pending = false;
        fit.refit();
        assert!(fit.refresh && fit.auto && !fit.pending, "F asks the simulation for the current extent first");
    }

    #[test]
    fn views_of_the_replaced_model_are_ignored() {
        let mut stale = LatestView {
            view: Some(Arc::new(LabView {
                grid: GridView { origin_x: 0, origin_y: 0, width: 1, height: 1, cells_per_texel: 1, rgba: vec![0; 4] },
                ants: Vec::new(),
                info: LabInfo { scenario: "maze".to_string(), ..life(Topology::Torus) },
            })),
            ..default()
        };
        stale.await_scenario("seeds");
        assert!(stale.is_stale() && stale.view.is_some(), "the old view stays shown meanwhile");
        let mut latest = LatestView::awaiting("seeds");
        let view = |id: &str| LabView {
            grid: GridView { origin_x: 0, origin_y: 0, width: 1, height: 1, cells_per_texel: 1, rgba: vec![0; 4] },
            ants: Vec::new(),
            info: LabInfo { scenario: id.to_string(), ..life(Topology::Torus) },
        };
        assert!(!latest.accept(&view("maze")), "published before the replacement");
        assert!(latest.accept(&view("seeds")));
        assert!(latest.awaiting.is_none());
        assert!(latest.accept(&view("maze")), "later scenario changes (loaded saves) are shown");
        // A load whose own view was never seen (another state replaced it at
        // once) does not freeze the display: after a few views it gives up.
        latest.await_scenario("seeds");
        let accepted: Vec<bool> = (0..=MAX_AWAIT_REJECTIONS).map(|_| latest.accept(&view("maze"))).collect();
        assert_eq!(accepted.iter().filter(|a| **a).count(), 1);
        assert_eq!(accepted.last(), Some(&true));
        assert!(!latest.is_stale());
    }

    #[test]
    fn diagram_requests_reach_below_the_newest_row() {
        let wolfram = info(FamilyInfo::Wolfram { rule: 30, width: 101, first_row: 40, generation: 90 });
        let visible = VisibleCells { min_x: -10, min_y: 0, max_x: 110, max_y: 99, cells_per_pixel: 1.0 };
        let r = region_request(&visible, Some(&wolfram), 1.0).expect("request").request;
        assert_eq!((r.x0, r.x1), (0, 100));
        assert_eq!(r.y0, 40, "rows that scrolled away are not requested");
        assert!(r.y1 > 90, "rows still to come are requested: {}", r.y1);
        assert_eq!(wolfram.bounds(), Some(CellRect::new(0, 40, 100, 90)));
    }

    #[test]
    fn frames_are_cropped_to_whole_texels() {
        // 4 × 3 texels of 2 cells from (-2, 10): cells x -2..5, y 10..15.
        let rgba: Vec<u8> = (0..12u8).flat_map(|i| [i, 0, 0, 255]).collect();
        let grid = GridView { origin_x: -2, origin_y: 10, width: 4, height: 3, cells_per_texel: 2, rgba };
        let whole = crop_frame(&grid, None).expect("frame");
        assert_eq!((whole.origin, whole.width, whole.height, whole.cells_per_texel), (I64Vec2::new(-2, 10), 4, 3, 2));
        // Keep cells x 0..3 and y 10..12: texel columns 1..=2, rows 0..=1.
        let part = crop_frame(&grid, Some(CellRect::new(0, 10, 3, 12))).expect("frame");
        assert_eq!((part.origin, part.width, part.height), (I64Vec2::new(0, 10), 2, 2));
        assert_eq!(part.data.chunks(4).map(|t| t[0]).collect::<Vec<_>>(), vec![1, 2, 5, 6]);
        assert!(crop_frame(&grid, Some(CellRect::new(100, 0, 120, 5))).is_none());
        let broken = GridView { rgba: vec![0; 3], ..grid };
        assert!(crop_frame(&broken, None).is_none());
    }

    #[test]
    fn framing_leaves_room_for_the_overlay() {
        // A 100 × 50 world in a 1000 × 600 viewport under a 200-pixel band:
        // 50 rows must fit in 564 − 200 pixels (0.137 cells per pixel), so
        // the band covers 28 more rows.
        let r = below_band(CellRect::new(0, 0, 99, 49), Vec2::new(1000.0, 600.0), 200.0);
        assert_eq!((r.x0, r.x1, r.y1), (0, 99, 49));
        assert_eq!(r.y0, -28);
        // Without a band nothing changes.
        assert_eq!(below_band(CellRect::new(0, 0, 99, 49), Vec2::new(1000.0, 600.0), 0.0), CellRect::new(0, 0, 99, 49));
        // A tall world is height-limited by the space under the band.
        let r = below_band(CellRect::new(0, 0, 9, 99), Vec2::new(1000.0, 600.0), 100.0);
        let scale = 100.0 / (600.0 * 0.94 - 100.0);
        assert_eq!(r.y0, -((100.0 * scale) as f32).ceil() as i64);
        assert_eq!(below_band(CellRect::new(0, 0, 9, 9), Vec2::ZERO, 50.0), CellRect::new(0, 0, 9, 9));
    }

    #[test]
    fn diagram_fits_by_width() {
        // 1025 cells across 940 usable pixels: 1.09 cells per pixel.
        let r = fit_rect(FitTarget::DiagramTop { width: 1025, top: 0 }, Vec2::new(1000.0, 500.0), 0.0);
        assert_eq!(r, CellRect::new(0, 0, 1024, 511));
        let r = fit_rect(FitTarget::DiagramTop { width: 1025, top: 0 }, Vec2::new(1000.0, 500.0), 100.0);
        assert_eq!((r.x0, r.x1), (0, 1024), "the diagram still fills the width");
        assert_eq!((r.y0, r.y1), (-110, 401), "row 0 sits below the band");
        // Older rows scrolled out of the history: the oldest kept row sits there.
        let r = fit_rect(FitTarget::DiagramTop { width: 1025, top: 3000 }, Vec2::new(1000.0, 500.0), 100.0);
        assert_eq!((r.y0, r.y1), (2890, 3401));
        let r = fit_rect(FitTarget::Rect(CellRect::new(-5, -5, 5, 5)), Vec2::new(100.0, 100.0), 0.0);
        assert_eq!(r, CellRect::new(-5, -5, 5, 5));
    }

    #[test]
    fn ant_markers_point_along_the_heading() {
        let [tip, ..] = ant_triangle(Vec2::ZERO, Heading::North, 10.0);
        assert!((tip - Vec2::new(0.0, 4.8)).length() < 1e-5, "north is up on screen: {tip}");
        let [tip, b, c] = ant_triangle(Vec2::new(1.0, 1.0), Heading::West, 10.0);
        assert!((tip - Vec2::new(-3.8, 1.0)).length() < 1e-5, "{tip}");
        assert_eq!(b.x, c.x, "the base is perpendicular to the heading");
    }

    #[test]
    fn colourings_per_family() {
        assert!(!Colouring::Age.available(Family::Ant));
        assert!(!Colouring::Trail.available(Family::Wireworld));
        assert_eq!(Colouring::States.next(Family::Wireworld), Colouring::States);
        assert_eq!(Colouring::States.next(Family::LifeLike), Colouring::Trail);
        assert_eq!(Colouring::Trail.next(Family::Ant), Colouring::Activity, "age is skipped");
        assert_eq!(Colouring::Age.tracking_param(Family::LifeLike), Some("track_ages"));
        assert_eq!(Colouring::Trail.tracking_param(Family::Wolfram), None, "diagrams always mark changes");
        let a = appearance(&life(Topology::Torus), &ViewSettings { colouring: Colouring::Age, ..default() }, 1);
        assert_eq!(a.style, GridStyle::Age);
        let ww = info(FamilyInfo::Wireworld { world: World { topology: Topology::Bounded, width: 4, height: 4 } });
        let a = appearance(&ww, &ViewSettings { colouring: Colouring::Trail, ..default() }, 1);
        assert_eq!(a.style, GridStyle::PixelArt, "unavailable colourings fall back to states");
        // Zoomed out: two-state models show the live fraction, multi-state models their states.
        let a = appearance(&life(Topology::Torus), &ViewSettings::default(), 4);
        assert_eq!((a.style, a.ramp), (GridStyle::Density, Colormap::Grayscale));
        assert_eq!(appearance(&ww, &ViewSettings::default(), 4).style, GridStyle::PixelArt);
        let age = ViewSettings { colouring: Colouring::Age, ..default() };
        assert_eq!(appearance(&life(Topology::Torus), &age, 4).style, GridStyle::Age);
        assert_eq!(state_colors(&ww), family::WIREWORLD.to_vec());
    }
}
