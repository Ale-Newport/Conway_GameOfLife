//! Keyboard shortcuts and the world-input gate.
//!
//! [`SHORTCUTS`] is the single source of truth: [`map_key`] uses it to turn
//! key presses into [`ShortcutAction`]s and the Help window lists it.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContext, EguiInput, PrimaryEguiContext, egui};
use rendering::WorldInput;

use crate::commands::ShellCommand;
use crate::layout::ShellLayout;
use crate::mode::AppMode;
use crate::state::ShellState;

/// Modifier keys relevant to shortcuts. `command` is Ctrl on Windows/Linux
/// and either Ctrl or Cmd on macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Ctrl or Cmd is held.
    pub command: bool,
}

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    /// Play / pause.
    TogglePlay,
    /// Advance one tick.
    StepOne,
    /// Advance the configured number of ticks.
    StepMany,
    /// Reset to the scenario's initial state.
    Reset,
    /// Undo the last edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
    /// Back to the dashboard.
    Home,
    /// Save a screenshot.
    Screenshot,
    /// Faster.
    SpeedUp,
    /// Slower.
    SpeedDown,
    /// Show / hide panels.
    TogglePanels,
    /// Toggle fullscreen.
    ToggleFullscreen,
    /// Open the help window.
    Help,
}

/// One documented shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// Key.
    pub key: KeyCode,
    /// Required modifiers (must match exactly).
    pub modifiers: Modifiers,
    /// Action.
    pub action: ShortcutAction,
    /// Key label for the help window.
    pub label: &'static str,
    /// Description for the help window.
    pub description: &'static str,
}

const NONE: Modifiers = Modifiers { shift: false, command: false };
const SHIFT: Modifiers = Modifiers { shift: true, command: false };
const CMD: Modifiers = Modifiers { shift: false, command: true };
const CMD_SHIFT: Modifiers = Modifiers { shift: true, command: true };

/// Every keyboard shortcut of the shell.
pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        key: KeyCode::Space,
        modifiers: NONE,
        action: ShortcutAction::TogglePlay,
        label: "Space",
        description: "Play / pause",
    },
    Shortcut {
        key: KeyCode::KeyN,
        modifiers: NONE,
        action: ShortcutAction::StepOne,
        label: "N",
        description: "Step one tick",
    },
    Shortcut {
        key: KeyCode::KeyN,
        modifiers: SHIFT,
        action: ShortcutAction::StepMany,
        label: "Shift+N",
        description: "Step the configured number of ticks",
    },
    Shortcut {
        key: KeyCode::KeyR,
        modifiers: NONE,
        action: ShortcutAction::Reset,
        label: "R",
        description: "Reset to the scenario's initial state (undoable)",
    },
    Shortcut {
        key: KeyCode::KeyZ,
        modifiers: CMD,
        action: ShortcutAction::Undo,
        label: "Ctrl/Cmd+Z",
        description: "Undo the last edit",
    },
    Shortcut {
        key: KeyCode::KeyZ,
        modifiers: CMD_SHIFT,
        action: ShortcutAction::Redo,
        label: "Shift+Ctrl/Cmd+Z",
        description: "Redo",
    },
    Shortcut {
        key: KeyCode::KeyY,
        modifiers: CMD,
        action: ShortcutAction::Redo,
        label: "Ctrl/Cmd+Y",
        description: "Redo (alternative)",
    },
    Shortcut {
        key: KeyCode::Equal,
        modifiers: NONE,
        action: ShortcutAction::SpeedUp,
        label: "=",
        description: "Faster (next speed step)",
    },
    Shortcut {
        key: KeyCode::Equal,
        modifiers: SHIFT,
        action: ShortcutAction::SpeedUp,
        label: "+",
        description: "Faster (Shift+= on US layouts)",
    },
    Shortcut {
        key: KeyCode::NumpadAdd,
        modifiers: NONE,
        action: ShortcutAction::SpeedUp,
        label: "Numpad +",
        description: "Faster",
    },
    Shortcut {
        key: KeyCode::Minus,
        modifiers: NONE,
        action: ShortcutAction::SpeedDown,
        label: "-",
        description: "Slower (previous speed step)",
    },
    Shortcut {
        key: KeyCode::NumpadSubtract,
        modifiers: NONE,
        action: ShortcutAction::SpeedDown,
        label: "Numpad -",
        description: "Slower",
    },
    Shortcut {
        key: KeyCode::Home,
        modifiers: NONE,
        action: ShortcutAction::Home,
        label: "Home",
        description: "Back to the dashboard",
    },
    Shortcut {
        key: KeyCode::Tab,
        modifiers: NONE,
        action: ShortcutAction::TogglePanels,
        label: "Tab",
        description: "Hide / show the side panels and the bottom bar",
    },
    Shortcut {
        key: KeyCode::F11,
        modifiers: NONE,
        action: ShortcutAction::ToggleFullscreen,
        label: "F11",
        description: "Toggle fullscreen",
    },
    Shortcut {
        key: KeyCode::F12,
        modifiers: NONE,
        action: ShortcutAction::Screenshot,
        label: "F12",
        description: "Save a PNG screenshot of the window",
    },
    Shortcut {
        key: KeyCode::F1,
        modifiers: NONE,
        action: ShortcutAction::Help,
        label: "F1",
        description: "Open this help",
    },
];

/// The action bound to `key` with exactly `modifiers`, if any.
pub fn map_key(key: KeyCode, modifiers: Modifiers) -> Option<ShortcutAction> {
    SHORTCUTS.iter().find(|s| s.key == key && s.modifiers == modifiers).map(|s| s.action)
}

/// Turns a shortcut into a shell command (`None` for the help window, which
/// is local UI state).
pub fn action_command(action: ShortcutAction, step_many: u64) -> Option<ShellCommand> {
    Some(match action {
        ShortcutAction::TogglePlay => ShellCommand::TogglePlay,
        ShortcutAction::StepOne => ShellCommand::Step(1),
        ShortcutAction::StepMany => ShellCommand::Step(step_many.max(1)),
        ShortcutAction::Reset => ShellCommand::Reset,
        ShortcutAction::Undo => ShellCommand::Undo,
        ShortcutAction::Redo => ShellCommand::Redo,
        ShortcutAction::Home => ShellCommand::OpenMode(AppMode::Dashboard),
        ShortcutAction::Screenshot => ShellCommand::Screenshot,
        ShortcutAction::SpeedUp => ShellCommand::Faster,
        ShortcutAction::SpeedDown => ShellCommand::Slower,
        ShortcutAction::TogglePanels => ShellCommand::TogglePanels,
        ShortcutAction::ToggleFullscreen => ShellCommand::ToggleFullscreen,
        ShortcutAction::Help => return None,
    })
}

/// Whether a shortcut applies only inside a simulation mode.
pub fn needs_simulation(action: ShortcutAction) -> bool {
    !matches!(
        action,
        ShortcutAction::Home | ShortcutAction::Screenshot | ShortcutAction::ToggleFullscreen | ShortcutAction::Help
    )
}

/// Reads key presses and writes shell commands. Ignored while an egui
/// widget has keyboard focus (typing a name must not pause the run).
pub(crate) fn keyboard_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    egui_input: Option<Res<EguiWantsInput>>,
    state: Res<State<AppMode>>,
    mut shell: ResMut<ShellState>,
    mut out: MessageWriter<ShellCommand>,
) {
    if egui_input.is_some_and(|e| e.wants_any_keyboard_input()) {
        return;
    }
    let modifiers = Modifiers {
        shift: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        command: keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]),
    };
    let in_simulation = state.get().is_simulation();
    for key in keys.get_just_pressed() {
        let Some(action) = map_key(*key, modifiers) else { continue };
        if needs_simulation(action) && !in_simulation {
            continue;
        }
        match action_command(action, shell.step_many) {
            Some(command) => {
                out.write(command);
            }
            None => shell.show_help = true,
        }
    }
}

/// Whether `event` is a press or release of Tab (with any modifiers).
pub fn is_tab_key(event: &egui::Event) -> bool {
    matches!(event, egui::Event::Key { key: egui::Key::Tab, .. })
}

/// Reserves Tab for the panels shortcut inside a mode: while no egui widget
/// has keyboard focus, Tab events are removed from egui's input. Otherwise
/// egui would also move keyboard focus to the first widget, after which it
/// wants the keyboard and every later shortcut (and world key) is ignored
/// until the user clicks elsewhere. Tab still moves between widgets once one
/// has focus (e.g. from a text field). Runs between bevy_egui's input
/// processing and the start of its pass.
pub(crate) fn reserve_tab_for_shell(
    state: Res<State<AppMode>>,
    mut contexts: Query<(&mut EguiContext, &mut EguiInput), With<PrimaryEguiContext>>,
) {
    if !state.get().is_simulation() {
        return;
    }
    for (mut context, mut input) in &mut contexts {
        if context.get_mut().memory(|m| m.focused().is_none()) {
            input.events.retain(|event| !is_tab_key(event));
        }
    }
}

/// Pure decision behind [`WorldInput`]: pointer input belongs to the world
/// when a mode is active, the cursor is inside the world viewport and egui
/// is not using the pointer (windows, popups, drags); keyboard input when
/// egui has no keyboard focus.
pub fn compute_world_input(
    in_simulation: bool,
    cursor: Option<Vec2>,
    central: Option<Rect>,
    egui_pointer_busy: bool,
    egui_keyboard_busy: bool,
) -> WorldInput {
    let viewport = if in_simulation { central } else { None };
    let inside = matches!((cursor, viewport), (Some(c), Some(v)) if v.contains(c));
    WorldInput {
        pointer: inside && !egui_pointer_busy,
        keyboard: in_simulation && !egui_keyboard_busy,
        cursor,
        viewport,
    }
}

/// Writes [`WorldInput`] in `PreUpdate` from the layout of the previous
/// frame and egui's input state (also from the previous frame: egui runs
/// late in `PostUpdate`).
pub(crate) fn update_world_input(
    mut world_input: ResMut<WorldInput>,
    layout: Res<ShellLayout>,
    state: Res<State<AppMode>>,
    egui_input: Option<Res<EguiWantsInput>>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
) {
    let cursor = window.and_then(|w| w.cursor_position());
    let (pointer_busy, keyboard_busy) =
        egui_input.map_or((false, false), |e| (e.wants_any_pointer_input(), e.wants_any_keyboard_input()));
    let next = compute_world_input(state.get().is_simulation(), cursor, layout.central, pointer_busy, keyboard_busy);
    let changed = world_input.pointer != next.pointer
        || world_input.keyboard != next.keyboard
        || world_input.cursor != next.cursor
        || world_input.viewport != next.viewport;
    if changed {
        *world_input = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_unambiguous() {
        for (i, a) in SHORTCUTS.iter().enumerate() {
            for b in &SHORTCUTS[i + 1..] {
                assert!(!(a.key == b.key && a.modifiers == b.modifiers), "{} and {} collide", a.label, b.label);
            }
        }
    }

    #[test]
    fn documented_keys_map_to_their_actions() {
        for s in SHORTCUTS {
            assert_eq!(map_key(s.key, s.modifiers), Some(s.action), "{}", s.label);
        }
        assert_eq!(map_key(KeyCode::KeyZ, CMD), Some(ShortcutAction::Undo));
        assert_eq!(map_key(KeyCode::KeyZ, CMD_SHIFT), Some(ShortcutAction::Redo));
        assert_eq!(map_key(KeyCode::KeyZ, NONE), None, "a bare Z does nothing");
        assert_eq!(map_key(KeyCode::Space, CMD), None, "modifiers must match exactly");
    }

    #[test]
    fn every_shortcut_is_documented() {
        let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/UI_SHELL.md")).unwrap();
        let table = doc.split("### 3.3 Keyboard shortcuts").nth(1).unwrap().split("###").next().unwrap();
        for s in SHORTCUTS {
            let row = table
                .lines()
                .find(|line| line.split('|').nth(1).is_some_and(|keys| keys.split(',').any(|k| k.contains(s.label))));
            assert!(row.is_some(), "`{}` ({}) is missing from docs/UI_SHELL.md §3.3", s.label, s.description);
        }
    }

    #[test]
    fn actions_become_commands() {
        assert_eq!(action_command(ShortcutAction::StepMany, 25), Some(ShellCommand::Step(25)));
        assert_eq!(action_command(ShortcutAction::StepMany, 0), Some(ShellCommand::Step(1)));
        assert_eq!(action_command(ShortcutAction::Home, 1), Some(ShellCommand::OpenMode(AppMode::Dashboard)));
        assert_eq!(action_command(ShortcutAction::Help, 1), None);
        assert!(!needs_simulation(ShortcutAction::Screenshot));
        assert!(needs_simulation(ShortcutAction::TogglePlay));
    }

    #[test]
    fn tab_events_are_recognised() {
        let key = |key, pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        let mut events = vec![key(egui::Key::Tab, true), egui::Event::Text("a".into()), key(egui::Key::Space, true)];
        events.push(key(egui::Key::Tab, false));
        events.retain(|e| !is_tab_key(e));
        assert_eq!(events, vec![egui::Event::Text("a".into()), key(egui::Key::Space, true)]);
    }

    /// Without the filter, egui gives keyboard focus to the first widget
    /// when Tab is pressed (and then wants the keyboard); with it, nothing
    /// is focused and the shell's shortcuts keep working.
    #[test]
    fn filtered_tab_leaves_no_widget_focused() {
        let run = |filter: bool| {
            let ctx = egui::Context::default();
            let draw = |ui: &mut egui::Ui| {
                let _ = ui.button("first");
                let _ = ui.button("second");
            };
            let _ = crate::run_headless_ui(&ctx, draw);
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            });
            if filter && ctx.memory(|m| m.focused().is_none()) {
                input.events.retain(|e| !is_tab_key(e));
            }
            let mut output = ctx.run_ui(input, draw);
            output.textures_delta.clear();
            ctx.egui_wants_keyboard_input()
        };
        assert!(run(false), "precondition: egui focuses a widget on Tab");
        assert!(!run(true));
    }

    #[test]
    fn world_input_gate() {
        let central = Some(Rect::new(200.0, 40.0, 1200.0, 900.0));
        let inside = Some(Vec2::new(600.0, 400.0));
        let over_panel = Some(Vec2::new(100.0, 400.0));
        let open = compute_world_input(true, inside, central, false, false);
        assert!(open.pointer && open.keyboard);
        assert_eq!(open.cursor_in_viewport(), Some(Vec2::new(400.0, 360.0)));
        assert!(!compute_world_input(true, over_panel, central, false, false).pointer);
        assert!(!compute_world_input(true, inside, central, true, false).pointer, "egui window or drag");
        assert!(!compute_world_input(true, inside, central, false, true).keyboard, "text field focused");
        let dashboard = compute_world_input(false, inside, central, false, false);
        assert!(!dashboard.pointer && !dashboard.keyboard && dashboard.viewport.is_none());
        assert!(!compute_world_input(true, None, central, false, false).pointer);
    }
}
