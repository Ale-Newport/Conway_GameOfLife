//! World interaction: inspecting tiles, drawing macro cells, stamping
//! library patterns as macro patterns, and the mode's keyboard shortcuts.
//! Pointer actions need `WorldInput::pointer`, keys `WorldInput::keyboard`.
//!
//! Every macro edit is a model command that rebuilds the construction at
//! generation 0 (the model's definition: tiles cannot be inserted into a
//! running construction). A drag paints several tiles and is sent as one
//! undoable edit.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Mutex, PoisonError};

use bevy::math::I64Vec2;
use bevy::prelude::*;
use interface::{SessionStatus, Toasts};
use metapixel::spec::CellRect;
use metapixel::{MetapixelCommand, MetapixelSim, StepSize};
use rendering::grid::GridSpace;
use rendering::{WorldInput, cursor_to_cell};
use simulation_core::{Control, ParamValue};

use crate::geometry::GridShape;
use crate::session::{MpSession, SimData, jump_to_next_readout};
use crate::timing::neighbour_step;
use crate::world::{CameraFit, MpCamera, ViewSettings, current_shape};

/// Pointer tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Hover and click tiles to inspect them.
    #[default]
    Inspect,
    /// Click or drag to switch macro cells of the generation-0 pattern.
    Draw,
    /// Place a library pattern as macro cells.
    Stamp,
}

impl Tool {
    /// Every tool, in display order.
    pub const ALL: [Tool; 3] = [Tool::Inspect, Tool::Draw, Tool::Stamp];

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Inspect => "Inspect",
            Tool::Draw => "Draw",
            Tool::Stamp => "Stamp",
        }
    }

    /// Tooltip.
    pub fn help(self) -> &'static str {
        match self {
            Tool::Inspect => "Hover a tile to read it, click to select it (key 1)",
            Tool::Draw => {
                "Click a tile to switch it ON or OFF; drag to paint several. Rebuilds the construction at generation 0 \
                 (key 2)"
            }
            Tool::Stamp => "Click a tile to turn ON the cells of a library pattern there, as macro cells (key 3)",
        }
    }
}

/// Tiles painted by a drag in progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stroke {
    /// State the drag paints (the opposite of the first tile's).
    pub target: bool,
    /// Tiles crossed so far, in order.
    pub tiles: Vec<(u32, u32)>,
}

impl Stroke {
    /// Adds a tile once.
    pub fn add(&mut self, tile: (u32, u32)) {
        if !self.tiles.contains(&tile) {
            self.tiles.push(tile);
        }
    }
}

/// Pointer state and selection.
#[derive(Resource, Debug, Default)]
pub struct ToolState {
    /// Active tool.
    pub tool: Tool,
    /// Library pattern of the stamp tool (index into [`stamp_patterns`]).
    pub stamp: usize,
    /// Cell under the pointer.
    pub hover_cell: Option<I64Vec2>,
    /// Tile under the pointer.
    pub hover_tile: Option<(u32, u32)>,
    /// Selected tile (its inspection is requested from the model).
    pub selected: Option<(u32, u32)>,
    /// Drag in progress.
    pub stroke: Option<Stroke>,
    errors: Mutex<Option<Receiver<String>>>,
}

/// Library patterns usable as macro stamps (the pattern catalogue's
/// built-in Life patterns).
pub fn stamp_patterns() -> &'static [patterns::BuiltinPattern] {
    patterns::builtin_patterns()
}

/// Live cells of stamp `index`, normalised to its bounding box.
pub fn stamp_cells(index: usize) -> Option<Vec<(i64, i64)>> {
    let pattern = stamp_patterns().get(index)?.pattern().ok()?;
    Some(pattern.normalized().live_cells().collect())
}

/// Size (columns, rows) of stamp `index`.
pub fn stamp_size(index: usize) -> Option<(u64, u64)> {
    let pattern = stamp_patterns().get(index)?.pattern().ok()?;
    Some((pattern.width(), pattern.height()))
}

/// The pattern `grid` becomes when `stroke` is applied.
pub fn apply_stroke(grid: &metapixel::MacroGrid, stroke: &Stroke) -> metapixel::MacroGrid {
    let mut next = grid.clone();
    for &(c, r) in &stroke.tiles {
        if c < next.width() && r < next.height() {
            next.set(c, r, stroke.target);
        }
    }
    next
}

/// Sends a finished stroke as one undoable edit: the generation-0 pattern
/// is read on the simulation thread (never stale), the stroke applied and
/// the construction rebuilt once.
fn commit_stroke(session: &MpSession, stroke: Stroke, errors: &Mutex<Option<Receiver<String>>>) {
    let (tx, rx) = std::sync::mpsc::channel();
    session.send(Control::ExecEdit(Box::new(move |sim: &mut MetapixelSim| {
        let next = apply_stroke(sim.initial_pattern(), &stroke);
        if let Err(e) = simulation_core::Simulation::apply(sim, MetapixelCommand::SetPattern(next)) {
            let _ = tx.send(e.to_string());
        }
    })));
    *errors.lock().unwrap_or_else(PoisonError::into_inner) = Some(rx);
}

/// Hover, selection, drawing and stamping.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pointer_tools(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    space: Res<GridSpace>,
    data: Res<SimData>,
    session: Option<Res<MpSession>>,
    mut tools: ResMut<ToolState>,
    mut toasts: ResMut<Toasts>,
    camera: Single<(&Camera, &GlobalTransform), With<MpCamera>>,
) {
    let (camera, transform) = *camera;
    let shape = current_shape(&data);
    let cell = cursor_to_cell(&input, camera, transform, &space);
    let tile = cell.zip(shape).and_then(|(c, s)| s.tile_at(c));
    if tools.hover_cell != cell {
        tools.hover_cell = cell;
    }
    if tools.hover_tile != tile {
        tools.hover_tile = tile;
    }
    // A selection outside a new, smaller grid is dropped.
    if let (Some((c, r)), Some(shape)) = (tools.selected, shape)
        && !shape.contains(c, r)
    {
        tools.selected = None;
    }
    // Errors of committed strokes.
    let error = {
        let mut guard = tools.errors.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref().map(Receiver::try_recv) {
            Some(Ok(e)) => {
                *guard = None;
                Some(e)
            }
            Some(Err(TryRecvError::Disconnected)) => {
                *guard = None;
                None
            }
            _ => None,
        }
    };
    if let Some(e) = error {
        toasts.error(format!("Edit refused: {e}"));
    }
    let Some(session) = session else { return };
    let pressed = buttons.just_pressed(MouseButton::Left) && input.pointer;
    match tools.tool {
        Tool::Inspect => {
            if pressed {
                tools.selected = tile;
            }
        }
        Tool::Draw => {
            if pressed && let (Some(tile), Some(info)) = (tile, data.info.as_ref()) {
                let current = info.initial.get(tile.0, tile.1);
                tools.stroke = Some(Stroke { target: !current, tiles: vec![tile] });
            }
            if buttons.pressed(MouseButton::Left)
                && let (Some(tile), Some(stroke)) = (tile, tools.stroke.as_mut())
            {
                stroke.add(tile);
            }
            if !buttons.pressed(MouseButton::Left)
                && let Some(stroke) = tools.stroke.take()
            {
                commit_stroke(&session, stroke, &tools.errors);
            }
        }
        Tool::Stamp => {
            if pressed
                && let Some((col, row)) = tile
                && let Some(entry) = stamp_patterns().get(tools.stamp)
            {
                session.edit(MetapixelCommand::PlaceRle {
                    rle: entry.rle.to_string(),
                    col: i64::from(col),
                    row: i64::from(row),
                });
            }
        }
    }
    if tools.tool != Tool::Draw && tools.stroke.is_some() {
        tools.stroke = None;
    }
}

/// A keyboard shortcut of this mode (none clashes with the shell's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeShortcut {
    /// Key.
    pub key: KeyCode,
    /// Label in the shortcut list.
    pub label: &'static str,
    /// What it does.
    pub description: &'static str,
}

/// The mode's shortcuts (also listed in the Tools panel).
pub const MODE_SHORTCUTS: &[ModeShortcut] = &[
    ModeShortcut { key: KeyCode::Digit1, label: "1", description: "Inspect tool" },
    ModeShortcut { key: KeyCode::Digit2, label: "2", description: "Draw macro cells" },
    ModeShortcut { key: KeyCode::Digit3, label: "3", description: "Stamp a pattern" },
    ModeShortcut { key: KeyCode::KeyJ, label: "J", description: "Advance to the next readout" },
    ModeShortcut { key: KeyCode::BracketLeft, label: "[", description: "Smaller Hashlife step" },
    ModeShortcut { key: KeyCode::BracketRight, label: "]", description: "Larger Hashlife step" },
    ModeShortcut { key: KeyCode::KeyL, label: "L", description: "Layer: auto, cells, macro" },
    ModeShortcut { key: KeyCode::KeyM, label: "M", description: "Macro layer: readout, live, prediction" },
    ModeShortcut { key: KeyCode::KeyF, label: "F", description: "Frame the whole grid" },
    ModeShortcut { key: KeyCode::KeyT, label: "T", description: "Frame the selected tile" },
    ModeShortcut { key: KeyCode::KeyC, label: "C", description: "Zoom to the rule table of the selected tile" },
    ModeShortcut { key: KeyCode::KeyG, label: "G", description: "Tile grid lines" },
    ModeShortcut { key: KeyCode::KeyO, label: "O", description: "Outline the parts of the tile" },
    ModeShortcut { key: KeyCode::Escape, label: "Esc", description: "Clear the selection" },
];

/// Tile-nominal rectangle around the rule table (the 18 eaters).
pub const RULE_TABLE_RECT: CellRect = CellRect { x0: 100, y0: 1380, x1: 210, y1: 1540 };

/// Cell rectangle of the rule table of tile `(col, row)`.
pub fn rule_table_rect(shape: GridShape, col: u32, row: u32) -> CellRect {
    let t = shape.tile_rect(col, row);
    RULE_TABLE_RECT.translated(t.x0, t.y0)
}

/// Current step size from the session status.
pub fn current_step(status: &SessionStatus) -> Option<StepSize> {
    status.get().and_then(|s| s.params.get("step")).and_then(|v| v.as_str()).and_then(StepSize::parse)
}

/// Keyboard shortcuts of the mode.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mode_shortcuts(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    status: Res<SessionStatus>,
    session: Option<Res<MpSession>>,
    mut data: ResMut<SimData>,
    mut tools: ResMut<ToolState>,
    mut settings: ResMut<ViewSettings>,
    mut fit: ResMut<CameraFit>,
) {
    if !input.keyboard {
        return;
    }
    let modifiers = [KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight];
    if keys.any_pressed(modifiers) {
        return;
    }
    let shape = current_shape(&data);
    let focus = tools.selected.or(tools.hover_tile);
    for shortcut in MODE_SHORTCUTS.iter().filter(|s| keys.just_pressed(s.key)) {
        match shortcut.key {
            KeyCode::Digit1 => tools.tool = Tool::Inspect,
            KeyCode::Digit2 => tools.tool = Tool::Draw,
            KeyCode::Digit3 => tools.tool = Tool::Stamp,
            KeyCode::KeyJ => {
                if let Some(session) = &session {
                    jump_to_next_readout(session, &mut data);
                }
            }
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                if let (Some(session), Some(step)) = (&session, current_step(&status)) {
                    let next = neighbour_step(step, shortcut.key == KeyCode::BracketRight);
                    session.send(Control::SetParam("step".into(), ParamValue::Text(next.value())));
                }
            }
            KeyCode::KeyL => settings.layer = settings.layer.next(),
            KeyCode::KeyM => settings.source = settings.source.next(),
            KeyCode::KeyF => fit.request(true),
            KeyCode::KeyT => {
                if let (Some(shape), Some((c, r))) = (shape, focus) {
                    fit.frame(shape.tile_rect(c, r), 0.06);
                }
            }
            KeyCode::KeyC => {
                if let (Some(shape), Some((c, r))) = (shape, focus) {
                    fit.frame(rule_table_rect(shape, c, r), 0.05);
                }
            }
            KeyCode::KeyG => settings.tile_grid = !settings.tile_grid,
            KeyCode::KeyO => settings.anatomy = !settings.anatomy,
            KeyCode::Escape => {
                tools.selected = None;
                tools.stroke = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macro_layer::{LayerMode, MacroSource};

    /// Every layer mode and source is reachable from the keyboard.
    fn cycles_cover_everything() -> bool {
        let mut layer = LayerMode::Auto;
        let mut layers = vec![layer];
        let mut source = MacroSource::Readout;
        let mut sources = vec![source];
        for _ in 0..3 {
            layer = layer.next();
            source = source.next();
            layers.push(layer);
            sources.push(source);
        }
        LayerMode::ALL.iter().all(|l| layers.contains(l)) && MacroSource::ALL.iter().all(|s| sources.contains(s))
    }

    #[test]
    fn shortcuts_do_not_clash_with_the_shell_or_the_camera() {
        let camera = [
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
        ];
        for s in MODE_SHORTCUTS {
            assert!(!interface::input::SHORTCUTS.iter().any(|shell| shell.key == s.key), "{} is a shell key", s.label);
            assert!(!camera.contains(&s.key), "{} pans the camera", s.label);
        }
        let mut keys: Vec<KeyCode> = MODE_SHORTCUTS.iter().map(|s| s.key).collect();
        keys.sort_by_key(|k| format!("{k:?}"));
        keys.dedup();
        assert_eq!(keys.len(), MODE_SHORTCUTS.len(), "each key does one thing");
        assert!(cycles_cover_everything());
    }

    #[test]
    fn strokes_paint_one_state() {
        let grid = metapixel::MacroGrid::from_rows(&["O..", "...", "..O"]).unwrap();
        let mut stroke = Stroke { target: true, tiles: vec![(1, 1)] };
        stroke.add((2, 1));
        stroke.add((1, 1));
        assert_eq!(stroke.tiles.len(), 2, "tiles are added once");
        stroke.add((7, 7));
        let next = apply_stroke(&grid, &stroke);
        assert_eq!(next.to_rows(), ["O..", ".OO", "..O"], "tiles outside the grid are ignored");
        let erase = Stroke { target: false, tiles: vec![(0, 0), (2, 2)] };
        assert_eq!(apply_stroke(&grid, &erase).population(), 0);
    }

    #[test]
    fn stamps_come_from_the_pattern_library() {
        let glider = stamp_patterns().iter().position(|p| p.id == "glider").expect("glider in the library");
        let cells = stamp_cells(glider).unwrap();
        assert_eq!(cells.len(), 5);
        assert!(cells.iter().all(|&(x, y)| (0..3).contains(&x) && (0..3).contains(&y)));
        assert_eq!(stamp_size(glider), Some((3, 3)));
        assert!(stamp_cells(usize::MAX).is_none());
    }

    #[test]
    fn rule_table_rect_contains_every_eater() {
        let programming = metapixel::Programming::conway();
        for a in metapixel::tile_annotations(programming) {
            if let metapixel::AnnotationKind::RuleBit { .. } = a.kind {
                assert!(
                    RULE_TABLE_RECT.contains(a.rect.x0, a.rect.y0) && RULE_TABLE_RECT.contains(a.rect.x1, a.rect.y1)
                );
            }
        }
        let shape = GridShape::new(2, 2);
        assert_eq!(rule_table_rect(shape, 1, 1).x0, 2048 + 100);
    }
}
