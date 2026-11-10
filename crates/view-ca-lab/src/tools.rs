//! World editing: the tools of each family (draw / paint, erase, lines and
//! Wireworld wires, random fills, circuit stamps, ants, editing the current
//! row of a diagram), all gated by [`WorldInput`], and the mode's keyboard
//! shortcuts.
//!
//! Every gesture is one undo step: its first edit is sent as
//! `Control::Edit` (the runner records the state before it), the rest of a
//! drag as `Control::Command`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use bevy::math::I64Vec2;
use bevy::prelude::*;
use cellular_automata::ant::{Ant, AntCommand, Heading};
use cellular_automata::library::EMBEDDED_PATTERNS;
use cellular_automata::{CellRect, LifeCommand, PaintCommand, Rotation, Symmetry, WolframCommand};
use patterns::Pattern;
use rendering::grid::GridSpace;
use rendering::{WorldInput, cursor_to_cell};
use simulation_core::{Control, Simulation};

use crate::family::Family;
use crate::session::LabRunner;
use crate::sim::{FamilyInfo, LabCommand, LabSim};
use crate::world::{CameraFit, LabCamera, LatestView, ViewSettings};

/// Editing tools (each family offers a subset, see [`Tool::for_family`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Hover to read cells.
    #[default]
    Inspect,
    /// Paint the chosen state with a round brush.
    Draw,
    /// Paint state 0 with a round brush.
    Erase,
    /// Straight one-cell line of the chosen state (Wireworld: a wire).
    Line,
    /// Drag a rectangle and fill it at random.
    Fill,
    /// Place a Wireworld circuit.
    Stamp,
    /// Add an ant.
    AddAnt,
    /// Remove the ants around the pointer.
    RemoveAnts,
    /// Toggle cells of the current row of a diagram.
    Toggle,
}

impl Tool {
    /// Tools offered for a family, in button and shortcut (1, 2, ...) order.
    pub fn for_family(family: Family) -> &'static [Tool] {
        match family {
            Family::LifeLike | Family::Generations => &[Tool::Inspect, Tool::Draw, Tool::Erase, Tool::Line, Tool::Fill],
            Family::Wireworld => &[Tool::Inspect, Tool::Draw, Tool::Erase, Tool::Line, Tool::Stamp],
            Family::Ant => &[Tool::Inspect, Tool::Draw, Tool::Erase, Tool::AddAnt, Tool::RemoveAnts, Tool::Fill],
            Family::Wolfram => &[Tool::Inspect, Tool::Toggle],
        }
    }

    /// Button label.
    pub fn label(self, family: Family) -> &'static str {
        match (self, family) {
            (Tool::Inspect, _) => "Inspect",
            (Tool::Draw, Family::LifeLike) => "Draw",
            (Tool::Draw, _) => "Paint",
            (Tool::Erase, _) => "Erase",
            (Tool::Line, Family::Wireworld) => "Wire",
            (Tool::Line, _) => "Line",
            (Tool::Fill, _) => "Random",
            (Tool::Stamp, _) => "Circuit",
            (Tool::AddAnt, _) => "Add ant",
            (Tool::RemoveAnts, _) => "Remove",
            (Tool::Toggle, _) => "Edit row",
        }
    }

    /// Tooltip.
    pub fn help(self, family: Family) -> &'static str {
        match (self, family) {
            (Tool::Inspect, _) => "Hover the world to read the state of a cell.",
            (Tool::Draw, Family::LifeLike) => {
                "Drag to draw live cells with a round brush; one stroke is one undo step."
            }
            (Tool::Draw, Family::Ant) => "Drag to paint the chosen colour.",
            (Tool::Draw, _) => "Drag to paint the chosen state.",
            (Tool::Erase, Family::Ant) => "Drag to paint colour 0.",
            (Tool::Erase, _) => "Drag to set cells to state 0 (dead / empty).",
            (Tool::Line, Family::Wireworld) => {
                "Drag from one end to the other: a straight one-cell wire of the chosen state (conductor by default)."
            }
            (Tool::Line, _) => "Drag from one end to the other: a straight one-cell line of the chosen state.",
            (Tool::Fill, _) => "Drag a rectangle: it is replaced by random cells at the chosen density.",
            (Tool::Stamp, _) => "Click to place the chosen circuit, centred on the pointer (Q / E rotate).",
            (Tool::AddAnt, _) => "Click to add an ant facing the chosen heading (Q / E turn it).",
            (Tool::RemoveAnts, _) => "Click to remove the ants inside the square around the pointer.",
            (Tool::Toggle, _) => {
                "Click cells of the newest row to flip them; drag to set more cells to the same value."
            }
        }
    }
}

/// Largest brush radius.
pub const MAX_RADIUS: u32 = 16;

/// Tool settings (kept across visits to the mode).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ToolSettings {
    /// Active tool.
    pub tool: Tool,
    /// Brush radius in cells (0 = single cells; discs `dx² + dy² ≤ r(r + 1)`).
    pub radius: u32,
    /// State (colour) painted by Draw and Line and scattered by Random.
    pub state: u8,
    /// Density of random fills.
    pub density: f64,
    /// Symmetry of Life-like random fills.
    pub symmetry: Symmetry,
    /// Selected circuit (index into the embedded Wireworld patterns).
    pub circuit: usize,
    /// Clockwise rotation of the circuit stamp.
    pub rotation: Rotation,
    /// Left-right mirror of the circuit stamp (before the rotation).
    pub mirror: bool,
    /// Heading of new ants.
    pub heading: Heading,
    /// Family the settings were last fitted to.
    pub family: Option<Family>,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            tool: Tool::Inspect,
            radius: 1,
            state: 1,
            density: 0.3,
            symmetry: Symmetry::None,
            circuit: 0,
            rotation: Rotation::None,
            mirror: false,
            heading: Heading::North,
            family: None,
        }
    }
}

/// Default paint state of a family (Wireworld: conductor).
pub fn default_state(family: Family) -> u8 {
    if family == Family::Wireworld { 3 } else { 1 }
}

impl ToolSettings {
    /// Keeps the settings valid for `family` with `states` states: a tool
    /// the family offers, a paint state in `1..states`; a new family gets
    /// its default paint state.
    pub fn fit(&mut self, family: Family, states: u16) {
        if self.family != Some(family) {
            self.family = Some(family);
            self.state = default_state(family);
        }
        if !Tool::for_family(family).contains(&self.tool) {
            self.tool = Tool::Inspect;
        }
        let max = states.saturating_sub(1).clamp(1, 255) as u8;
        self.state = self.state.clamp(1, max);
        self.radius = self.radius.min(MAX_RADIUS);
        self.density = self.density.clamp(0.0, 1.0);
    }

    /// Next / previous paint state within `1..states`.
    pub fn cycle_state(&mut self, states: u16, forward: bool) {
        let n = states.saturating_sub(1).max(1);
        let i = u16::from(self.state.max(1)) - 1;
        let next = if forward { (i + 1) % n } else { (i + n - 1) % n };
        self.state = (next + 1) as u8;
    }

    /// Turns the stamp (or the heading of new ants) a quarter clockwise or counter-clockwise.
    pub fn turn(&mut self, clockwise: bool) {
        if self.tool == Tool::AddAnt {
            const ORDER: [Heading; 4] = [Heading::North, Heading::East, Heading::South, Heading::West];
            let i = ORDER.iter().position(|h| *h == self.heading).unwrap_or(0);
            self.heading = ORDER[if clockwise { (i + 1) % 4 } else { (i + 3) % 4 }];
        } else {
            const ORDER: [Rotation; 4] = [Rotation::None, Rotation::Cw90, Rotation::Half, Rotation::Ccw90];
            let i = ORDER.iter().position(|r| *r == self.rotation).unwrap_or(0);
            self.rotation = ORDER[if clockwise { (i + 1) % 4 } else { (i + 3) % 4 }];
        }
    }
}

/// Size `(width, height)` of a pattern after `rotation`.
pub fn rotated_size(size: (u64, u64), rotation: Rotation) -> (u64, u64) {
    if rotation.quarter_turns() % 2 == 1 { (size.1, size.0) } else { size }
}

/// Top-left cell of a stamp of `size` (before rotation) centred on `cell`.
pub fn stamp_origin(size: (u64, u64), rotation: Rotation, cell: I64Vec2) -> I64Vec2 {
    let (w, h) = rotated_size(size, rotation);
    I64Vec2::new(cell.x - (w / 2) as i64, cell.y - (h / 2) as i64)
}

/// Cells covered by a stamp of `size` centred on `cell`.
pub fn stamp_rect(size: (u64, u64), rotation: Rotation, cell: I64Vec2) -> CellRect {
    let (w, h) = rotated_size(size, rotation);
    let o = stamp_origin(size, rotation, cell);
    CellRect::new(o.x, o.y, o.x + w.max(1) as i64 - 1, o.y + h.max(1) as i64 - 1)
}

/// The bundled Wireworld circuits.
#[derive(Resource, Clone, Debug)]
pub struct Circuits(pub Vec<Circuit>);

/// One bundled circuit.
#[derive(Clone, Debug)]
pub struct Circuit {
    /// Pattern id (`wireworld/diode`...).
    pub id: &'static str,
    /// The pattern (Golly states: 1 head, 2 tail, 3 conductor).
    pub pattern: Pattern,
    /// Licence line of the file.
    pub licence: String,
}

/// Circuits read from the asset folder at run time (not compiled in: the
/// McIntosh AND gate is GPL-2.0-or-later data, see
/// `assets/patterns/wireworld/PROVENANCE.md`).
pub const ASSET_CIRCUITS: [&str; 1] = ["wireworld/and-gate-mcintosh"];

fn circuit(id: &'static str, text: &str) -> Option<Circuit> {
    // Normalised: the bounding box starts at (0, 0), so stamps centre on the pointer.
    let pattern = patterns::rle::parse(text).ok()?.normalized();
    let licence = text
        .lines()
        .find_map(|l| l.strip_prefix("#C License: "))
        .map(|l| l.split(';').next().unwrap_or(l).trim_end_matches('.').to_string())
        .unwrap_or_default();
    Some(Circuit { id, pattern, licence })
}

impl Circuit {
    /// Display name ("diode", "clock, period 8"...).
    pub fn name(&self) -> String {
        let name = self.pattern.name.trim();
        let name = name.strip_prefix("Wireworld ").unwrap_or(name);
        if name.is_empty() { self.id.trim_start_matches("wireworld/").to_string() } else { name.to_string() }
    }
}

impl Circuits {
    /// The circuits embedded in the `cellular-automata` crate, plus those
    /// read from `asset_root/patterns/` when it is given and readable.
    pub fn load(asset_root: Option<&std::path::Path>) -> Self {
        let mut list: Vec<Circuit> = EMBEDDED_PATTERNS.iter().filter_map(|&(id, text)| circuit(id, text)).collect();
        if let Some(root) = asset_root {
            for id in ASSET_CIRCUITS {
                let path = root.join("patterns").join(format!("{id}.rle"));
                if let Some(c) = std::fs::read_to_string(&path).ok().and_then(|text| circuit(id, &text)) {
                    list.push(c);
                }
            }
        }
        Self(list)
    }

    /// Size of circuit `i`.
    pub fn size(&self, i: usize) -> Option<(u64, u64)> {
        self.0.get(i).map(|c| (c.pattern.width(), c.pattern.height()))
    }
}

/// Cells covered by the circuit stamp centred on `cell`.
pub fn stamp_footprint(tools: &ToolSettings, circuits: &Circuits, cell: I64Vec2) -> Option<CellRect> {
    circuits.size(tools.circuit).map(|size| stamp_rect(size, tools.rotation, cell))
}

/// The cell under the pointer (when it is over the world).
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Hover {
    /// Cell coordinates.
    pub cell: Option<I64Vec2>,
}

/// Progress of the current gesture.
#[derive(Resource, Default, Debug)]
pub struct ToolState {
    /// Start cell of a line or rectangle drag.
    pub drag_start: Option<I64Vec2>,
    /// Last cell of a stroke.
    last: Option<I64Vec2>,
    /// Value written by a row-editing drag, chosen on the simulation
    /// thread by its first click ([`ROW_VALUE_UNSET`] until then).
    toggle_value: Option<Arc<AtomicU8>>,
    /// A gesture started over the world.
    active: bool,
}

/// The edit for a brush stroke through `points` with the settings.
pub fn stroke_command(family: Family, tools: &ToolSettings, points: Vec<(i64, i64)>) -> Option<LabCommand> {
    let erase = tools.tool == Tool::Erase;
    let state = if erase { 0 } else { tools.state };
    Some(match family {
        Family::LifeLike => LabCommand::Life(LifeCommand::Stroke { points, radius: tools.radius, alive: !erase }),
        Family::Generations | Family::Wireworld => {
            LabCommand::Paint(PaintCommand::Stroke { points, radius: tools.radius, state })
        }
        Family::Ant => LabCommand::Ant(AntCommand::Stroke { points, radius: tools.radius, color: state }),
        Family::Wolfram => return None,
    })
}

/// The edit for a line from `a` to `b`.
pub fn line_command(family: Family, tools: &ToolSettings, a: I64Vec2, b: I64Vec2) -> Option<LabCommand> {
    let (from, to) = ((a.x, a.y), (b.x, b.y));
    Some(match family {
        Family::LifeLike => LabCommand::Life(LifeCommand::Line { from, to, alive: true }),
        Family::Generations | Family::Wireworld => {
            LabCommand::Paint(PaintCommand::Line { from, to, state: tools.state })
        }
        _ => return None,
    })
}

/// The edit for a random fill of the rectangle spanned by `a` and `b`.
pub fn fill_command(family: Family, tools: &ToolSettings, a: I64Vec2, b: I64Vec2) -> Option<LabCommand> {
    let region = Some(CellRect::new(a.x, a.y, b.x, b.y));
    let density = tools.density;
    Some(match family {
        Family::LifeLike => {
            LabCommand::Life(LifeCommand::Randomize { density, seed: None, region, symmetry: tools.symmetry })
        }
        Family::Generations | Family::Wireworld => LabCommand::Paint(PaintCommand::Randomize {
            density,
            state: tools.state,
            seed: None,
            region,
            symmetry: Symmetry::None,
        }),
        Family::Ant => LabCommand::Ant(AntCommand::Randomize { density, color: tools.state, seed: None, region }),
        Family::Wolfram => return None,
    })
}

/// [`ToolState`]'s row value before the first click of a drag ran.
pub const ROW_VALUE_UNSET: u8 = u8::MAX;

/// Flips cell `x` of the current row of an elementary automaton and stores
/// the new value in `chosen`. The value is read from the model itself: a
/// zoomed-out view aggregates several cells per texel, so it cannot tell
/// the value of one cell.
pub fn flip_row_cell(sim: &mut LabSim, x: i64, chosen: &AtomicU8) {
    let Some(current) = sim.wolfram_cell(x) else { return };
    chosen.store(u8::from(!current), Ordering::Release);
    if let Err(e) = sim.apply(LabCommand::Wolfram(WolframCommand::SetCell { x, alive: !current })) {
        warn!("row edit failed: {e}");
    }
}

/// Sets cell `x` of the current row to the value the drag's first click chose.
pub fn copy_row_value(sim: &mut LabSim, x: i64, chosen: &AtomicU8) {
    let value = chosen.load(Ordering::Acquire);
    if value <= 1
        && let Err(e) = sim.apply(LabCommand::Wolfram(WolframCommand::SetCell { x, alive: value == 1 }))
    {
        warn!("row edit failed: {e}");
    }
}

/// Square of side `2r + 1` around `cell`.
pub fn square(cell: I64Vec2, r: u32) -> CellRect {
    let r = i64::from(r);
    CellRect::new(cell.x - r, cell.y - r, cell.x + r, cell.y + r)
}

/// Applies the active tool to the pointer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pointer_tools(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    camera: Single<(&Camera, &GlobalTransform), With<LabCamera>>,
    space: Res<GridSpace>,
    latest: Res<LatestView>,
    runner: Option<Res<LabRunner>>,
    circuits: Res<Circuits>,
    tools: Res<ToolSettings>,
    mut state: ResMut<ToolState>,
    mut hover: ResMut<Hover>,
) {
    let (camera, transform) = *camera;
    let cell = cursor_to_cell(&input, camera, transform, &space);
    hover.cell = cell;
    let (Some(runner), Some(view)) = (runner, latest.view.as_deref()) else { return };
    if latest.is_stale() {
        // The shown view belongs to the model being replaced: no edits.
        *state = ToolState::default();
        return;
    }
    let family = view.info.family();
    let send = |edit: bool, command: Option<LabCommand>| {
        if let Some(command) = command {
            runner.0.send(if edit { Control::Edit(command) } else { Control::Command(command) });
        }
    };
    let pressed = buttons.just_pressed(MouseButton::Left);
    if pressed && cell.is_some() {
        state.active = true;
    }
    if state.active && buttons.just_released(MouseButton::Left) {
        // End of a line or rectangle drag (at the last cell hovered over the world).
        let end = cell.or(state.last);
        if let (Some(start), Some(end)) = (state.drag_start, end) {
            match tools.tool {
                Tool::Line => send(true, line_command(family, &tools, start, end)),
                Tool::Fill => send(true, fill_command(family, &tools, start, end)),
                _ => {}
            }
        }
    }
    if !buttons.pressed(MouseButton::Left) {
        *state = ToolState::default();
        return;
    }
    if !state.active {
        return;
    }
    let Some(c) = cell else { return };
    match tools.tool {
        Tool::Inspect => {}
        Tool::Draw | Tool::Erase => {
            if pressed {
                send(true, stroke_command(family, &tools, vec![(c.x, c.y)]));
            } else if let Some(last) = state.last
                && last != c
            {
                send(false, stroke_command(family, &tools, vec![(last.x, last.y), (c.x, c.y)]));
            }
        }
        Tool::Line | Tool::Fill => {
            if pressed {
                state.drag_start = Some(c);
            }
        }
        Tool::Stamp => {
            if pressed
                && family == Family::Wireworld
                && let Some(circuit) = circuits.0.get(tools.circuit)
            {
                let size = (circuit.pattern.width(), circuit.pattern.height());
                let o = stamp_origin(size, tools.rotation, c);
                send(
                    true,
                    Some(LabCommand::Paint(PaintCommand::Paste {
                        pattern: circuit.pattern.clone(),
                        x: o.x,
                        y: o.y,
                        rotation: tools.rotation,
                        mirror: tools.mirror,
                    })),
                );
            }
        }
        Tool::AddAnt => {
            if pressed && family == Family::Ant {
                send(true, Some(LabCommand::Ant(AntCommand::AddAnt(Ant { x: c.x, y: c.y, heading: tools.heading }))));
            }
        }
        Tool::RemoveAnts => {
            if pressed && family == Family::Ant {
                let rect = square(c, tools.radius.max(1));
                send(true, Some(LabCommand::Ant(AntCommand::RemoveAnts { rect })));
            }
        }
        Tool::Toggle => {
            let FamilyInfo::Wolfram { generation, width, .. } = &view.info.detail else { return };
            let on_row = c.y == *generation as i64 && (0..i64::from(*width)).contains(&c.x);
            let x = c.x;
            if pressed && on_row {
                // One undo step; the cell's exact value is read on the simulation thread.
                let chosen = Arc::new(AtomicU8::new(ROW_VALUE_UNSET));
                state.toggle_value = Some(chosen.clone());
                runner.0.send(Control::ExecEdit(Box::new(move |sim: &mut LabSim| flip_row_cell(sim, x, &chosen))));
            } else if let (Some(chosen), Some(last)) = (state.toggle_value.clone(), state.last)
                && last != c
                && on_row
            {
                runner.0.send(Control::Exec(Box::new(move |sim: &mut LabSim| copy_row_value(sim, x, &chosen))));
            }
        }
    }
    state.last = Some(c);
}

/// A mode shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeShortcut {
    /// Tool by position in the family's list.
    Tool(usize),
    /// Smaller brush.
    Smaller,
    /// Larger brush.
    Larger,
    /// Previous paint state.
    PreviousState,
    /// Next paint state.
    NextState,
    /// Turn the stamp / new ant counter-clockwise.
    TurnLeft,
    /// Turn the stamp / new ant clockwise.
    TurnRight,
    /// Frame the world.
    Fit,
    /// Cell grid lines.
    Grid,
    /// Next colouring.
    Colouring,
    /// Follow the ant / newest row.
    Follow,
}

/// The mode's shortcuts: key, action, key label, description.
pub const MODE_SHORTCUTS: &[(KeyCode, ModeShortcut, &str, &str)] = &[
    (KeyCode::Digit1, ModeShortcut::Tool(0), "1", "Inspect"),
    (KeyCode::Digit2, ModeShortcut::Tool(1), "2", "Second tool (draw, paint or edit row)"),
    (KeyCode::Digit3, ModeShortcut::Tool(2), "3", "Third tool (erase)"),
    (KeyCode::Digit4, ModeShortcut::Tool(3), "4", "Fourth tool (line, wire or add ant)"),
    (KeyCode::Digit5, ModeShortcut::Tool(4), "5", "Fifth tool (random fill, circuit or remove ants)"),
    (KeyCode::Digit6, ModeShortcut::Tool(5), "6", "Sixth tool (ants: random colours)"),
    (KeyCode::BracketLeft, ModeShortcut::Smaller, "[", "Smaller brush"),
    (KeyCode::BracketRight, ModeShortcut::Larger, "]", "Larger brush"),
    (KeyCode::Comma, ModeShortcut::PreviousState, ",", "Previous paint state / colour"),
    (KeyCode::Period, ModeShortcut::NextState, ".", "Next paint state / colour"),
    (KeyCode::KeyQ, ModeShortcut::TurnLeft, "Q", "Turn the circuit or the new ant counter-clockwise"),
    (KeyCode::KeyE, ModeShortcut::TurnRight, "E", "Turn the circuit or the new ant clockwise"),
    (KeyCode::KeyF, ModeShortcut::Fit, "F", "Frame the world"),
    (KeyCode::KeyG, ModeShortcut::Grid, "G", "Show / hide cell grid lines"),
    (KeyCode::KeyT, ModeShortcut::Colouring, "T", "Next colouring (states, trail, age, heat map)"),
    (KeyCode::KeyV, ModeShortcut::Follow, "V", "Follow the ant / the newest row"),
];

/// Action bound to a key.
pub fn mode_shortcut(key: KeyCode) -> Option<ModeShortcut> {
    MODE_SHORTCUTS.iter().find(|(k, ..)| *k == key).map(|(_, s, ..)| *s)
}

/// Executes the mode's shortcuts (only while no widget has the keyboard and
/// without Ctrl / Cmd / Alt).
pub(crate) fn mode_shortcuts(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    latest: Res<LatestView>,
    mut tools: ResMut<ToolSettings>,
    mut settings: ResMut<ViewSettings>,
    mut fit: ResMut<CameraFit>,
) {
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
    if modifier || latest.is_stale() {
        return;
    }
    let Some(info) = latest.info() else { return };
    let family = info.family();
    let states = info.states();
    for key in keys.get_just_pressed() {
        let Some(shortcut) = mode_shortcut(*key) else { continue };
        match shortcut {
            ModeShortcut::Tool(i) => {
                if let Some(tool) = Tool::for_family(family).get(i) {
                    tools.tool = *tool;
                }
            }
            ModeShortcut::Smaller => tools.radius = tools.radius.saturating_sub(1),
            ModeShortcut::Larger => tools.radius = (tools.radius + 1).min(MAX_RADIUS),
            ModeShortcut::PreviousState => tools.cycle_state(states, false),
            ModeShortcut::NextState => tools.cycle_state(states, true),
            ModeShortcut::TurnLeft => tools.turn(false),
            ModeShortcut::TurnRight => tools.turn(true),
            ModeShortcut::Fit => fit.refit(),
            ModeShortcut::Grid => settings.grid_lines = !settings.grid_lines,
            ModeShortcut::Colouring => settings.colouring = settings.colouring.next(family),
            ModeShortcut::Follow => settings.follow = !settings.follow,
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
        for (i, (key, _, label, _)) in MODE_SHORTCUTS.iter().enumerate() {
            assert!(SHORTCUTS.iter().all(|s| s.key != *key), "{label} is a shell shortcut");
            assert!(!camera.contains(key), "{label} moves the camera");
            assert!(MODE_SHORTCUTS[i + 1..].iter().all(|(k, ..)| k != key), "{label} is bound twice");
        }
        assert_eq!(mode_shortcut(KeyCode::KeyR), None, "R is the shell's reset");
        assert_eq!(mode_shortcut(KeyCode::KeyN), None, "N is the shell's step");
        // Every tool of every family has a digit.
        for family in Family::ALL {
            assert!(Tool::for_family(family).len() <= 6);
            assert_eq!(Tool::for_family(family)[0], Tool::Inspect);
        }
    }

    #[test]
    fn settings_follow_the_family() {
        let mut t = ToolSettings { tool: Tool::Stamp, state: 9, ..default() };
        t.fit(Family::Wireworld, 4);
        assert_eq!((t.tool, t.state), (Tool::Stamp, 3), "Wireworld paints conductor by default");
        t.fit(Family::Wolfram, 2);
        assert_eq!((t.tool, t.state), (Tool::Inspect, 1), "no stamp for diagrams");
        t.tool = Tool::Draw;
        t.state = 7;
        t.fit(Family::Wolfram, 2);
        assert_eq!(t.tool, Tool::Inspect);
        t.fit(Family::Generations, 4);
        t.state = 3;
        t.cycle_state(4, true);
        assert_eq!(t.state, 1, "wraps within 1..states");
        t.cycle_state(4, false);
        assert_eq!(t.state, 3);
        t.cycle_state(2, true);
        assert_eq!(t.state, 1);
    }

    #[test]
    fn stamps_and_headings_turn() {
        let mut t = ToolSettings { tool: Tool::Stamp, ..default() };
        t.turn(true);
        assert_eq!(t.rotation, Rotation::Cw90);
        t.turn(false);
        t.turn(false);
        assert_eq!(t.rotation, Rotation::Ccw90);
        t.tool = Tool::AddAnt;
        t.turn(false);
        assert_eq!(t.heading, Heading::West);
        assert_eq!(t.rotation, Rotation::Ccw90, "turning an ant leaves the stamp alone");
    }

    #[test]
    fn stamp_geometry() {
        // An 11 × 3 diode centred on (20, 10).
        assert_eq!(stamp_origin((11, 3), Rotation::None, I64Vec2::new(20, 10)), I64Vec2::new(15, 9));
        assert_eq!(stamp_rect((11, 3), Rotation::None, I64Vec2::new(20, 10)), CellRect::new(15, 9, 25, 11));
        assert_eq!(stamp_rect((11, 3), Rotation::Cw90, I64Vec2::new(20, 10)), CellRect::new(19, 5, 21, 15));
        assert_eq!(rotated_size((4, 7), Rotation::Half), (4, 7));
        assert_eq!(square(I64Vec2::new(1, 1), 2), CellRect::new(-1, -1, 3, 3));
    }

    #[test]
    fn bundled_circuits_parse_with_licences() {
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        assert_eq!(Circuits::load(None).0.len(), EMBEDDED_PATTERNS.len());
        let c = Circuits::load(Some(&assets));
        assert_eq!(c.0.len(), EMBEDDED_PATTERNS.len() + ASSET_CIRCUITS.len());
        assert_eq!(c.size(0), Some((11, 3)), "diode");
        assert_eq!(c.0[0].name(), "diode");
        let and = c.0.last().expect("AND gate");
        assert_eq!(and.pattern.bounding_box().map(|b| (b.0, b.1)), Some((0, 0)), "normalised");
        assert_eq!(c.size(c.0.len() - 1), Some((24, 8)));
        assert!(c.0.iter().all(|x| !x.licence.is_empty()), "every file states its licence");
        let and = c.0.iter().find(|x| x.id.contains("and-gate")).expect("AND gate");
        assert!(and.licence.contains("GPL"), "{}", and.licence);
        assert!(c.0.iter().all(|x| x.pattern.cells.iter().all(|cell| (1..=3).contains(&cell.state))));
    }

    /// Row edits read the exact cell from the model, flip it, and a drag
    /// copies the chosen value (also when the view is zoomed out).
    #[test]
    fn row_edits_use_the_exact_cell_value() {
        let mut sim = crate::sim::tests::lab("wolfram-rule-30");
        assert_eq!(sim.wolfram_cell(512), Some(true), "the single seed cell");
        let chosen = AtomicU8::new(ROW_VALUE_UNSET);
        copy_row_value(&mut sim, 10, &chosen);
        assert_eq!(sim.wolfram_cell(10), Some(false), "nothing is copied before the first click ran");
        flip_row_cell(&mut sim, 512, &chosen);
        assert_eq!((sim.wolfram_cell(512), chosen.load(Ordering::Acquire)), (Some(false), 0));
        flip_row_cell(&mut sim, 3, &chosen);
        assert_eq!((sim.wolfram_cell(3), chosen.load(Ordering::Acquire)), (Some(true), 1));
        copy_row_value(&mut sim, 4, &chosen);
        copy_row_value(&mut sim, 4, &chosen);
        assert_eq!(sim.wolfram_cell(4), Some(true), "a drag sets, it does not toggle");
        assert_eq!(sim.wolfram_cell(5000), None, "outside the row");
        flip_row_cell(&mut sim, 5000, &chosen);
        assert_eq!(chosen.load(Ordering::Acquire), 1, "a click outside the row chooses nothing");
        assert_eq!(crate::sim::tests::lab("seeds").wolfram_cell(0), None);
    }

    #[test]
    fn commands_carry_the_settings() {
        let t = ToolSettings { tool: Tool::Erase, radius: 2, state: 3, density: 0.4, ..default() };
        assert_eq!(
            stroke_command(Family::Wireworld, &t, vec![(1, 2)]),
            Some(LabCommand::Paint(PaintCommand::Stroke { points: vec![(1, 2)], radius: 2, state: 0 }))
        );
        assert_eq!(
            stroke_command(Family::LifeLike, &t, vec![(1, 2)]),
            Some(LabCommand::Life(LifeCommand::Stroke { points: vec![(1, 2)], radius: 2, alive: false }))
        );
        assert_eq!(stroke_command(Family::Wolfram, &t, vec![]), None);
        let wire = line_command(Family::Wireworld, &t, I64Vec2::new(0, 0), I64Vec2::new(5, 0));
        assert_eq!(wire, Some(LabCommand::Paint(PaintCommand::Line { from: (0, 0), to: (5, 0), state: 3 })));
        assert_eq!(line_command(Family::Ant, &t, I64Vec2::ZERO, I64Vec2::ONE), None);
        let fill = fill_command(Family::Ant, &t, I64Vec2::new(4, 4), I64Vec2::new(0, 0));
        assert_eq!(
            fill,
            Some(LabCommand::Ant(AntCommand::Randomize {
                density: 0.4,
                color: 3,
                seed: None,
                region: Some(CellRect::new(0, 0, 4, 4))
            }))
        );
    }
}
