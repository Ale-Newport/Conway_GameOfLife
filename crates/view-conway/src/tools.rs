//! World interaction: the editing tools (all gated by [`WorldInput`] and all
//! sent to the model as `LifeCommand`s), the selection, the internal
//! clipboard, the stamp and the mode's keyboard shortcuts.
//!
//! One gesture is one undo step: the first command of a drag stroke is sent
//! as `Control::Edit` (the runner records the state before it) and the rest
//! of the stroke as plain commands; lines, rectangles, fills, moves and
//! stamps are a single `Control::Edit` each.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use bevy::math::I64Vec2;
use bevy::prelude::*;
use cellular_automata::view::TexelGrid;
use cellular_automata::{CaError, CellRect, GridViewRequest, LifeCommand, PasteMode, Rotation, Symmetry};
use patterns::Pattern;
use rendering::grid::GridSpace;
use rendering::{WorldInput, cursor_to_cell};

use crate::commands::{ModeCommand, ModeCommands, SelectionOp};
use crate::geometry::{clip_to_world, rotated_size, stamp_origin, turn};
use crate::session::{ConwayRunner, LiveWorld, lock};
use crate::style::ViewSettings;
use crate::window::{PatternsTab, PatternsWindow};
use crate::world::{CameraFit, LifeCamera};

/// Active world tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Paint live cells along the pointer.
    #[default]
    Draw,
    /// Kill cells along the pointer.
    Erase,
    /// Straight line from press to release.
    Line,
    /// Rectangle (outline or filled) from press to release.
    Rectangle,
    /// Random soup in the dragged rectangle.
    Fill,
    /// Rectangular selection; drag inside it to move its cells.
    Select,
    /// Paste the stamp pattern centred on the pointer.
    Stamp,
}

impl Tool {
    /// Every tool in toolbar order.
    pub const ALL: [Tool; 7] =
        [Tool::Draw, Tool::Erase, Tool::Line, Tool::Rectangle, Tool::Fill, Tool::Select, Tool::Stamp];

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Draw => "Draw",
            Tool::Erase => "Erase",
            Tool::Line => "Line",
            Tool::Rectangle => "Rect",
            Tool::Fill => "Fill",
            Tool::Select => "Select",
            Tool::Stamp => "Stamp",
        }
    }

    /// Tooltip.
    pub fn help(self) -> &'static str {
        match self {
            Tool::Draw => "Drag to paint live cells; one stroke is one undo step (key 1).",
            Tool::Erase => "Drag to kill cells; one stroke is one undo step (key 2).",
            Tool::Line => "Drag a straight line of cells (Bresenham, 8-connected) (key 3).",
            Tool::Rectangle => "Drag a rectangle, outline or filled (key 4).",
            Tool::Fill => "Drag a region and fill it with a random soup of the chosen density (key 5).",
            Tool::Select => "Drag to select; drag inside the selection to move its cells (key 6).",
            Tool::Stamp => "Click to paste the stamp pattern centred on the pointer; Q / E rotate, M mirrors (key 7).",
        }
    }
}

/// Tool parameters (kept across visits to the mode).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ToolSettings {
    /// Active tool.
    pub tool: Tool,
    /// Brush radius of Draw and Erase (0 = single cells; discs `dx² + dy² ≤ r(r+1)`).
    pub brush_radius: u32,
    /// Line and rectangle kill cells instead of painting them.
    pub shape_erase: bool,
    /// Rectangles are filled.
    pub rect_filled: bool,
    /// Probability of a live cell in random fills (also the shell's Randomize).
    pub fill_density: f64,
    /// Symmetry of random fills.
    pub fill_symmetry: Symmetry,
    /// Seed of the next random fill (advanced after each fill).
    pub fill_seed: u64,
    /// Clockwise rotation of the stamp.
    pub rotation: Rotation,
    /// Left-right mirror of the stamp (applied before the rotation).
    pub mirror: bool,
    /// How stamped cells combine with existing cells.
    pub paste_mode: PasteMode,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            tool: Tool::Draw,
            brush_radius: 0,
            shape_erase: false,
            rect_filled: false,
            fill_density: 0.35,
            fill_symmetry: Symmetry::None,
            fill_seed: 1,
            rotation: Rotation::None,
            mirror: false,
            paste_mode: PasteMode::Or,
        }
    }
}

/// Largest brush radius.
pub const MAX_BRUSH_RADIUS: u32 = 16;

impl ToolSettings {
    /// Changes the brush radius by `delta` within `0..=MAX_BRUSH_RADIUS`.
    pub fn resize_brush(&mut self, delta: i32) {
        self.brush_radius = self.brush_radius.saturating_add_signed(delta).min(MAX_BRUSH_RADIUS);
    }

    /// Rotates the stamp a quarter turn.
    pub fn rotate_stamp(&mut self, cw: bool) {
        self.rotation = turn(self.rotation, cw);
    }

    /// The paste command for `stamp` centred on `cell`.
    pub fn paste_at(&self, stamp: &Pattern, cell: I64Vec2) -> LifeCommand {
        let size = rotated_size(stamp.width(), stamp.height(), self.rotation);
        let origin = stamp_origin(cell, size);
        LifeCommand::Paste {
            pattern: stamp.clone(),
            x: origin.x,
            y: origin.y,
            rotation: self.rotation,
            mirror: self.mirror,
            mode: self.paste_mode,
        }
    }
}

/// Where the stamp pattern came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StampSource {
    /// A library pattern (id).
    Library(String),
    /// Copied or cut from the world.
    Clipboard,
    /// Imported from RLE or another text format.
    Imported,
}

/// The pattern the stamp tool pastes (normalised: bounding box at the origin).
#[derive(Clone, Debug)]
pub struct StampPattern {
    /// Display name.
    pub name: String,
    /// Origin.
    pub source: StampSource,
    /// The cells.
    pub pattern: Arc<Pattern>,
}

/// The current stamp.
#[derive(Resource, Default, Debug)]
pub struct Stamp(pub Option<StampPattern>);

/// The cell under the pointer.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Hover {
    /// Cell under the pointer while it is over the world.
    pub cell: Option<I64Vec2>,
}

/// What a drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragKind {
    /// Draw or erase stroke.
    Stroke {
        /// Paint (`true`) or erase.
        alive: bool,
    },
    /// Line preview.
    Line,
    /// Rectangle preview.
    Rectangle,
    /// Random-fill region.
    Fill,
    /// New selection.
    Select,
    /// Moving the selection's cells.
    Move {
        /// The selection being moved.
        rect: CellRect,
    },
}

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    /// What it does.
    pub kind: DragKind,
    /// Cell under the press.
    pub start: I64Vec2,
    /// Cell under the pointer now.
    pub current: I64Vec2,
}

impl Drag {
    /// The dragged rectangle.
    pub fn rect(&self) -> CellRect {
        CellRect::new(self.start.x, self.start.y, self.current.x, self.current.y)
    }

    /// Offset of a move.
    pub fn offset(&self) -> I64Vec2 {
        self.current.saturating_sub(self.start)
    }
}

/// The drag in progress, if any.
#[derive(Resource, Default, Debug)]
pub struct DragState {
    /// The drag.
    pub drag: Option<Drag>,
}

/// What is known about the selection's content.
#[derive(Clone, Debug)]
pub struct SelectionInfo {
    /// Rectangle the information describes.
    pub rect: CellRect,
    /// Live cells inside it (inside the world).
    pub population: u64,
    /// Its cells relative to the top-left corner (when small enough).
    pub content: Option<Arc<Pattern>>,
}

/// Selections larger than this many cells are counted but not copied for
/// the move preview.
pub const MAX_PREVIEW_AREA: u128 = 1 << 22;
/// Selections with more live cells than this are not copied for the move
/// preview (the copy runs on the simulation thread twice a second).
pub const MAX_PREVIEW_CELLS: u64 = 1 << 16;

/// The selection rectangle and its content.
#[derive(Resource, Default)]
pub struct Selection {
    /// Selected rectangle.
    pub rect: Option<CellRect>,
    /// Latest information about the content.
    pub info: Option<SelectionInfo>,
    pending: Mutex<Option<Receiver<SelectionInfo>>>,
    since: f32,
}

impl Selection {
    /// Removes the selection.
    pub fn clear(&mut self) {
        self.rect = None;
        self.info = None;
        *lock(&self.pending) = None;
    }

    /// Selects `rect` (asks for its content on the next update).
    pub fn set(&mut self, rect: CellRect) {
        self.rect = Some(rect);
        self.since = f32::INFINITY;
    }

    /// Information about the current rectangle, if up to date.
    pub fn current_info(&self) -> Option<&SelectionInfo> {
        self.info.as_ref().filter(|i| Some(i.rect) == self.rect)
    }
}

/// Counts the live cells of `rect` with the engine's level-of-detail counter
/// (exact; Hashlife uses its cached quadtree populations) and copies the
/// cells when the rectangle is small.
pub fn read_selection(sim: &cellular_automata::LifeSim, rect: CellRect) -> SelectionInfo {
    let request = GridViewRequest { x0: rect.x0, y0: rect.y0, x1: rect.x1, y1: rect.y1, max_width: 64, max_height: 64 };
    let grid = TexelGrid::plan(&request, sim.world().bounds());
    let mut counts = vec![0u32; grid.len()];
    if grid.clip.is_some() {
        sim.engine().count_live(&grid, &mut counts);
    }
    let population = counts.iter().map(|&c| u64::from(c)).sum();
    let content = (rect.area() <= MAX_PREVIEW_AREA && population <= MAX_PREVIEW_CELLS)
        .then(|| sim.copy_region(rect).ok().map(Arc::new))
        .flatten();
    SelectionInfo { rect, population, content }
}

/// Seconds between two reads of the selection's content.
pub const SELECTION_PERIOD: f32 = 0.5;

/// Keeps the selection's population and content up to date.
pub(crate) fn poll_selection(time: Res<Time>, runner: Option<Res<ConwayRunner>>, mut selection: ResMut<Selection>) {
    let Some(runner) = runner else { return };
    selection.since += time.delta_secs();
    let received = lock(&selection.pending).as_ref().map(Receiver::try_recv);
    match received {
        Some(Ok(info)) => {
            *lock(&selection.pending) = None;
            selection.info = Some(info);
        }
        Some(Err(TryRecvError::Disconnected)) => *lock(&selection.pending) = None,
        Some(Err(TryRecvError::Empty)) => {}
        None => {
            let Some(rect) = selection.rect else { return };
            if selection.since >= SELECTION_PERIOD {
                selection.since = 0.0;
                *lock(&selection.pending) = Some(runner.request(move |sim| read_selection(sim, rect)));
            }
        }
    }
}

/// The internal clipboard (patterns copied or cut from the world).
#[derive(Resource, Default)]
pub struct Clipboard {
    /// Last copied pattern (normalised).
    pub pattern: Option<Arc<Pattern>>,
    pub(crate) pending: Mutex<Option<Receiver<Result<Pattern, CaError>>>>,
}

/// Hover tracking and the pointer tools.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pointer_tools(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    camera: Single<(&Camera, &GlobalTransform), With<LifeCamera>>,
    space: Res<GridSpace>,
    runner: Option<Res<ConwayRunner>>,
    live: Res<LiveWorld>,
    tools: Res<ToolSettings>,
    stamp: Res<Stamp>,
    mut selection: ResMut<Selection>,
    mut drag: ResMut<DragState>,
    mut hover: ResMut<Hover>,
    mut queue: ResMut<ModeCommands>,
) {
    let (camera, transform) = *camera;
    hover.cell = cursor_to_cell(&input, camera, transform, &space);
    let Some(runner) = runner else {
        drag.drag = None;
        return;
    };
    // While dragging, follow the pointer even across panels.
    let raw_cell =
        input.cursor.and_then(|c| camera.viewport_to_world_2d(transform, c).ok()).map(|w| space.world_to_cell(w));
    let world = live.current().map(|i| i.world);

    if buttons.just_pressed(MouseButton::Left)
        && drag.drag.is_none()
        && let Some(cell) = hover.cell
    {
        let start = |kind| Some(Drag { kind, start: cell, current: cell });
        drag.drag = match tools.tool {
            Tool::Draw | Tool::Erase => {
                let alive = tools.tool == Tool::Draw;
                runner.edit(LifeCommand::Stroke { points: vec![(cell.x, cell.y)], radius: tools.brush_radius, alive });
                start(DragKind::Stroke { alive })
            }
            Tool::Line => start(DragKind::Line),
            Tool::Rectangle => start(DragKind::Rectangle),
            Tool::Fill => start(DragKind::Fill),
            Tool::Select => match selection.rect {
                Some(rect) if rect.contains(cell.x, cell.y) => start(DragKind::Move { rect }),
                _ => start(DragKind::Select),
            },
            Tool::Stamp => {
                match &stamp.0 {
                    Some(s) => runner.edit(tools.paste_at(&s.pattern, cell)),
                    None => queue.0.push(ModeCommand::OpenPatterns(PatternsTab::Library)),
                }
                None
            }
        };
    }

    let Some(mut current) = drag.drag else { return };
    if let Some(cell) = raw_cell {
        if let DragKind::Stroke { alive } = current.kind
            && cell != current.current
        {
            runner.command(LifeCommand::Stroke {
                points: vec![(current.current.x, current.current.y), (cell.x, cell.y)],
                radius: tools.brush_radius,
                alive,
            });
        }
        current.current = cell;
    }
    if buttons.pressed(MouseButton::Left) {
        drag.drag = Some(current);
        return;
    }
    // Released: finish the gesture.
    drag.drag = None;
    let rect = current.rect();
    match current.kind {
        DragKind::Stroke { .. } => {}
        DragKind::Line => runner.edit(LifeCommand::Line {
            from: (current.start.x, current.start.y),
            to: (current.current.x, current.current.y),
            alive: !tools.shape_erase,
        }),
        DragKind::Rectangle => {
            runner.edit(LifeCommand::Rectangle { rect, filled: tools.rect_filled, alive: !tools.shape_erase })
        }
        DragKind::Fill => queue.0.push(ModeCommand::FillRegion(rect)),
        DragKind::Select => {
            let clipped = world.map_or(Some(rect), |w| clip_to_world(rect, w));
            match clipped {
                Some(r) if current.start != current.current => selection.set(r),
                _ => selection.clear(),
            }
        }
        DragKind::Move { rect } => {
            let d = current.offset();
            if d != I64Vec2::ZERO && selection.rect == Some(rect) {
                queue.0.push(ModeCommand::Selection(SelectionOp::Move(d.x, d.y)));
            }
        }
    }
}

/// Modifier needed by a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyModifier {
    /// No modifier (Shift allowed only where stated).
    None,
    /// Shift.
    Shift,
    /// Ctrl or Cmd.
    Command,
}

/// A mode shortcut action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Select a tool.
    Tool(Tool),
    /// Rotate the stamp or the selection counter-clockwise.
    RotateCcw,
    /// Rotate the stamp or the selection clockwise.
    RotateCw,
    /// Mirror the stamp or the selection left-right.
    Mirror,
    /// Smaller brush.
    BrushSmaller,
    /// Larger brush.
    BrushLarger,
    /// Frame the world or the pattern.
    Fit,
    /// Toggle the cell grid.
    Grid,
    /// Next render style.
    NextStyle,
    /// Previous render style.
    PreviousStyle,
    /// Pattern library window.
    Library,
    /// Import / export window.
    Import,
    /// Hashlife jump.
    Jump,
    /// Copy the selection.
    Copy,
    /// Cut the selection.
    Cut,
    /// Paste (system clipboard pattern or internal clipboard) as the stamp.
    Paste,
    /// Kill the selection's cells.
    Clear,
    /// Cancel the drag, then remove the selection.
    Cancel,
}

/// A keyboard shortcut of the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeShortcut {
    /// Key.
    pub key: KeyCode,
    /// Required modifier.
    pub modifier: KeyModifier,
    /// Action.
    pub action: ShortcutAction,
    /// Key label.
    pub label: &'static str,
    /// What it does.
    pub description: &'static str,
}

const fn sc(
    key: KeyCode,
    modifier: KeyModifier,
    action: ShortcutAction,
    label: &'static str,
    description: &'static str,
) -> ModeShortcut {
    ModeShortcut { key, modifier, action, label, description }
}

/// The mode's shortcuts. None uses a key of the shell (`interface::input::SHORTCUTS`)
/// or of the grid camera (arrows, WASD); checked by a test.
pub const MODE_SHORTCUTS: &[ModeShortcut] = &[
    sc(KeyCode::Digit1, KeyModifier::None, ShortcutAction::Tool(Tool::Draw), "1", "Draw tool"),
    sc(KeyCode::Digit2, KeyModifier::None, ShortcutAction::Tool(Tool::Erase), "2", "Erase tool"),
    sc(KeyCode::Digit3, KeyModifier::None, ShortcutAction::Tool(Tool::Line), "3", "Line tool"),
    sc(KeyCode::Digit4, KeyModifier::None, ShortcutAction::Tool(Tool::Rectangle), "4", "Rectangle tool"),
    sc(KeyCode::Digit5, KeyModifier::None, ShortcutAction::Tool(Tool::Fill), "5", "Random fill tool"),
    sc(KeyCode::Digit6, KeyModifier::None, ShortcutAction::Tool(Tool::Select), "6", "Select tool"),
    sc(KeyCode::Digit7, KeyModifier::None, ShortcutAction::Tool(Tool::Stamp), "7", "Stamp tool"),
    sc(KeyCode::KeyQ, KeyModifier::None, ShortcutAction::RotateCcw, "Q", "Rotate stamp / selection 90° left"),
    sc(KeyCode::KeyE, KeyModifier::None, ShortcutAction::RotateCw, "E", "Rotate stamp / selection 90° right"),
    sc(KeyCode::KeyM, KeyModifier::None, ShortcutAction::Mirror, "M", "Mirror stamp / selection left-right"),
    sc(KeyCode::BracketLeft, KeyModifier::None, ShortcutAction::BrushSmaller, "[", "Smaller brush"),
    sc(KeyCode::BracketRight, KeyModifier::None, ShortcutAction::BrushLarger, "]", "Larger brush"),
    sc(KeyCode::KeyF, KeyModifier::None, ShortcutAction::Fit, "F", "Frame the world / pattern"),
    sc(KeyCode::KeyG, KeyModifier::None, ShortcutAction::Grid, "G", "Cell grid on / off"),
    sc(KeyCode::KeyV, KeyModifier::None, ShortcutAction::NextStyle, "V", "Next render style"),
    sc(KeyCode::KeyV, KeyModifier::Shift, ShortcutAction::PreviousStyle, "Shift+V", "Previous render style"),
    sc(KeyCode::KeyP, KeyModifier::None, ShortcutAction::Library, "P", "Pattern library"),
    sc(KeyCode::KeyI, KeyModifier::None, ShortcutAction::Import, "I", "Import / export RLE"),
    sc(KeyCode::KeyJ, KeyModifier::None, ShortcutAction::Jump, "J", "Hashlife jump of 2^k generations"),
    sc(KeyCode::KeyC, KeyModifier::Command, ShortcutAction::Copy, "Ctrl/Cmd+C", "Copy the selection"),
    sc(KeyCode::KeyX, KeyModifier::Command, ShortcutAction::Cut, "Ctrl/Cmd+X", "Cut the selection"),
    sc(KeyCode::KeyV, KeyModifier::Command, ShortcutAction::Paste, "Ctrl/Cmd+V", "Paste as stamp"),
    sc(KeyCode::Delete, KeyModifier::None, ShortcutAction::Clear, "Delete", "Kill the selected cells"),
    sc(KeyCode::Backspace, KeyModifier::None, ShortcutAction::Clear, "Backspace", "Kill the selected cells"),
    sc(KeyCode::Escape, KeyModifier::None, ShortcutAction::Cancel, "Esc", "Cancel the drag / deselect"),
];

/// The shortcut for `key` with the given modifiers.
pub fn find_shortcut(key: KeyCode, shift: bool, command: bool) -> Option<ShortcutAction> {
    let wanted = if command {
        KeyModifier::Command
    } else if shift {
        KeyModifier::Shift
    } else {
        KeyModifier::None
    };
    MODE_SHORTCUTS.iter().find(|s| s.key == key && s.modifier == wanted).map(|s| s.action)
}

/// Applies the mode's keyboard shortcuts while no egui widget has the keyboard.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mode_shortcuts(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut tools: ResMut<ToolSettings>,
    mut settings: ResMut<ViewSettings>,
    mut fit: ResMut<CameraFit>,
    mut window: ResMut<PatternsWindow>,
    mut drag: ResMut<DragState>,
    mut selection: ResMut<Selection>,
    mut queue: ResMut<ModeCommands>,
) {
    if !input.keyboard {
        return;
    }
    let command =
        keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if alt {
        return;
    }
    let pressed: Vec<KeyCode> = keys.get_just_pressed().copied().collect();
    for key in pressed {
        let Some(action) = find_shortcut(key, shift, command) else { continue };
        let on_selection = tools.tool == Tool::Select && selection.rect.is_some();
        match action {
            ShortcutAction::Tool(tool) => tools.tool = tool,
            ShortcutAction::RotateCcw | ShortcutAction::RotateCw => {
                let cw = action == ShortcutAction::RotateCw;
                if on_selection {
                    queue.0.push(ModeCommand::Selection(SelectionOp::Rotate { cw }));
                } else {
                    tools.rotate_stamp(cw);
                }
            }
            ShortcutAction::Mirror => {
                if on_selection {
                    queue.0.push(ModeCommand::Selection(SelectionOp::FlipHorizontal));
                } else {
                    tools.mirror = !tools.mirror;
                }
            }
            ShortcutAction::BrushSmaller => tools.resize_brush(-1),
            ShortcutAction::BrushLarger => tools.resize_brush(1),
            ShortcutAction::Fit => fit.request_world(),
            ShortcutAction::Grid => settings.grid = !settings.grid,
            ShortcutAction::NextStyle => queue.0.push(ModeCommand::SetStyle(settings.style.cycle(true))),
            ShortcutAction::PreviousStyle => queue.0.push(ModeCommand::SetStyle(settings.style.cycle(false))),
            ShortcutAction::Library => window.toggle(PatternsTab::Library),
            ShortcutAction::Import => window.toggle(PatternsTab::Import),
            ShortcutAction::Jump => queue.0.push(ModeCommand::JumpRequested),
            ShortcutAction::Copy => queue.0.push(ModeCommand::Selection(SelectionOp::Copy)),
            ShortcutAction::Cut => queue.0.push(ModeCommand::Selection(SelectionOp::Cut)),
            ShortcutAction::Paste => queue.0.push(ModeCommand::Paste),
            ShortcutAction::Clear => queue.0.push(ModeCommand::Selection(SelectionOp::Clear)),
            ShortcutAction::Cancel => {
                if drag.drag.take().is_none() {
                    selection.clear();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface::input::SHORTCUTS;

    #[test]
    fn shortcuts_do_not_clash_with_the_shell_or_the_camera() {
        let camera = [
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
        ];
        for (i, s) in MODE_SHORTCUTS.iter().enumerate() {
            assert!(SHORTCUTS.iter().all(|shell| shell.key != s.key), "{} is a shell key", s.label);
            assert!(!camera.contains(&s.key), "{} moves the camera", s.label);
            let twice = MODE_SHORTCUTS[i + 1..].iter().any(|o| o.key == s.key && o.modifier == s.modifier);
            assert!(!twice, "{} is bound twice", s.label);
        }
        assert_eq!(find_shortcut(KeyCode::KeyV, false, false), Some(ShortcutAction::NextStyle));
        assert_eq!(find_shortcut(KeyCode::KeyV, true, false), Some(ShortcutAction::PreviousStyle));
        assert_eq!(find_shortcut(KeyCode::KeyV, false, true), Some(ShortcutAction::Paste));
        assert_eq!(find_shortcut(KeyCode::KeyR, false, false), None, "R is the shell's reset");
        assert_eq!(find_shortcut(KeyCode::KeyN, false, false), None, "N is the shell's step");
    }

    #[test]
    fn brush_and_stamp_settings() {
        let mut t = ToolSettings::default();
        t.resize_brush(-1);
        assert_eq!(t.brush_radius, 0);
        for _ in 0..40 {
            t.resize_brush(1);
        }
        assert_eq!(t.brush_radius, MAX_BRUSH_RADIUS);
        t.rotate_stamp(true);
        assert_eq!(t.rotation, Rotation::Cw90);
        t.rotate_stamp(false);
        t.rotate_stamp(false);
        assert_eq!(t.rotation, Rotation::Ccw90);
        // A 3 x 1 blinker rotated a quarter turn, centred on (10, 10): top-left (10, 9).
        let blinker = Pattern::from_ascii("OOO").unwrap();
        let t = ToolSettings { rotation: Rotation::Cw90, ..default() };
        match t.paste_at(&blinker, I64Vec2::new(10, 10)) {
            LifeCommand::Paste { x, y, rotation, .. } => assert_eq!((x, y, rotation), (10, 9, Rotation::Cw90)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn drags_and_selections() {
        let d = Drag { kind: DragKind::Select, start: I64Vec2::new(5, 9), current: I64Vec2::new(2, 3) };
        assert_eq!(d.rect(), CellRect::new(2, 3, 5, 9));
        assert_eq!(d.offset(), I64Vec2::new(-3, -6));
        let mut s = Selection::default();
        s.set(CellRect::new(0, 0, 3, 3));
        assert!(s.current_info().is_none());
        s.info = Some(SelectionInfo { rect: CellRect::new(0, 0, 3, 3), population: 4, content: None });
        assert_eq!(s.current_info().map(|i| i.population), Some(4));
        s.set(CellRect::new(0, 0, 4, 4));
        assert!(s.current_info().is_none(), "stale information is not shown");
        s.clear();
        assert!(s.rect.is_none());
    }

    #[test]
    fn selection_reads_count_exactly() {
        let mut sim = cellular_automata::LifeSim::conway(3);
        let rect = CellRect::new(10, 10, 99, 59);
        let info = read_selection(&sim, rect);
        let direct = sim.live_cells().iter().filter(|&&(x, y)| rect.contains(x, y)).count() as u64;
        assert_eq!(info.population, direct);
        assert_eq!(info.content.as_ref().map(|p| p.population() as u64), Some(direct));
        use simulation_core::Simulation;
        sim.apply(LifeCommand::Clear).unwrap();
        assert_eq!(read_selection(&sim, rect).population, 0);
    }
}
