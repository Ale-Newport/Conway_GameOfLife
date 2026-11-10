//! Keyboard shortcuts of the mode. They complement the shell's shortcuts
//! (`interface::input::SHORTCUTS`) and the orbit rig's keys (W A S D and
//! the arrows pan, Q / E rotate) without reusing any of their keys, and are
//! ignored while a text field or widget has keyboard focus.

use bevy::prelude::*;

/// What a mode shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutAction {
    /// Camera preset: aerial.
    PresetAerial,
    /// Camera preset: isometric.
    PresetIsometric,
    /// Camera preset: free orbit.
    PresetFreeOrbit,
    /// Camera preset: strategic.
    PresetStrategic,
    /// Fit the world in view.
    FrameWorld,
    /// Fit the population in view.
    FramePopulation,
    /// Cycle colouring (state, age, height).
    CycleColor,
    /// Cycle transparency (opaque, ghost dying, X-ray).
    CycleTransparency,
    /// Toggle the cut.
    ToggleCut,
    /// Cycle the cut axis.
    CycleCutAxis,
    /// Upper cut plane down one layer.
    CutMaxDown,
    /// Upper cut plane up one layer.
    CutMaxUp,
    /// Lower cut plane down one layer.
    CutMinDown,
    /// Lower cut plane up one layer.
    CutMinUp,
    /// Toggle the world box.
    ToggleBounds,
    /// Toggle the turntable rotation.
    ToggleTurntable,
    /// Toggle the region box.
    ToggleRegion,
    /// Inspect tool.
    ToolInspect,
    /// Place tool.
    ToolPlace,
    /// Erase tool.
    ToolErase,
}

/// One documented mode shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeShortcut {
    /// Key.
    pub key: KeyCode,
    /// Whether Shift must be held.
    pub shift: bool,
    /// Key label.
    pub label: &'static str,
    /// What it does.
    pub description: &'static str,
    /// Action.
    pub action: ShortcutAction,
}

const fn sc(
    key: KeyCode,
    shift: bool,
    label: &'static str,
    description: &'static str,
    action: ShortcutAction,
) -> ModeShortcut {
    ModeShortcut { key, shift, label, description, action }
}

/// Every shortcut of the Three-Dimensional Life mode.
pub const SHORTCUTS: &[ModeShortcut] = &[
    sc(KeyCode::Digit1, false, "1", "Camera: aerial (top-down)", ShortcutAction::PresetAerial),
    sc(KeyCode::Digit2, false, "2", "Camera: isometric (orthographic)", ShortcutAction::PresetIsometric),
    sc(KeyCode::Digit3, false, "3", "Camera: free orbit", ShortcutAction::PresetFreeOrbit),
    sc(KeyCode::Digit4, false, "4", "Camera: strategic", ShortcutAction::PresetStrategic),
    sc(KeyCode::KeyF, false, "F", "Frame the world", ShortcutAction::FrameWorld),
    sc(KeyCode::KeyG, false, "G", "Frame the population", ShortcutAction::FramePopulation),
    sc(KeyCode::KeyC, false, "C", "Colour by state / age / height", ShortcutAction::CycleColor),
    sc(KeyCode::KeyX, false, "X", "Opaque / ghost dying / X-ray", ShortcutAction::CycleTransparency),
    sc(KeyCode::KeyK, false, "K", "Cut on / off", ShortcutAction::ToggleCut),
    sc(KeyCode::KeyL, false, "L", "Cut axis x / y / z", ShortcutAction::CycleCutAxis),
    sc(KeyCode::BracketLeft, false, "[", "Upper cut plane down", ShortcutAction::CutMaxDown),
    sc(KeyCode::BracketRight, false, "]", "Upper cut plane up", ShortcutAction::CutMaxUp),
    sc(KeyCode::BracketLeft, true, "Shift+[", "Lower cut plane down", ShortcutAction::CutMinDown),
    sc(KeyCode::BracketRight, true, "Shift+]", "Lower cut plane up", ShortcutAction::CutMinUp),
    sc(KeyCode::KeyB, false, "B", "World box on / off", ShortcutAction::ToggleBounds),
    sc(KeyCode::KeyT, false, "T", "Turntable rotation", ShortcutAction::ToggleTurntable),
    sc(KeyCode::KeyM, false, "M", "Region box on / off", ShortcutAction::ToggleRegion),
    sc(KeyCode::KeyI, false, "I", "Inspect tool", ShortcutAction::ToolInspect),
    sc(KeyCode::KeyP, false, "P", "Place tool (Shift+click erases)", ShortcutAction::ToolPlace),
    sc(KeyCode::KeyO, false, "O", "Erase tool", ShortcutAction::ToolErase),
];

/// Keys the orbit rig uses (pan and rotate).
#[cfg(test)]
const RIG_KEYS: [KeyCode; 10] = [
    KeyCode::KeyW,
    KeyCode::KeyA,
    KeyCode::KeyS,
    KeyCode::KeyD,
    KeyCode::KeyQ,
    KeyCode::KeyE,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
];

/// The action of a key press (`shift` held), if any.
pub fn match_shortcut(key: KeyCode, shift: bool) -> Option<ShortcutAction> {
    SHORTCUTS.iter().find(|s| s.key == key && s.shift == shift).map(|s| s.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_do_not_clash_with_the_shell_or_the_rig() {
        for shortcut in SHORTCUTS {
            for shell in interface::input::SHORTCUTS {
                assert_ne!(shortcut.key, shell.key, "{} clashes with the shell's {}", shortcut.label, shell.label);
            }
            assert!(!RIG_KEYS.contains(&shortcut.key), "{} clashes with the orbit rig", shortcut.label);
        }
    }

    #[test]
    fn shortcuts_are_unique_and_documented() {
        for (i, a) in SHORTCUTS.iter().enumerate() {
            assert!(!a.label.is_empty() && !a.description.is_empty());
            for b in &SHORTCUTS[i + 1..] {
                assert!(!(a.key == b.key && a.shift == b.shift), "{} is bound twice", a.label);
                assert_ne!(a.action, b.action, "{} and {} do the same", a.label, b.label);
            }
        }
        assert_eq!(match_shortcut(KeyCode::BracketLeft, true), Some(ShortcutAction::CutMinDown));
        assert_eq!(match_shortcut(KeyCode::BracketLeft, false), Some(ShortcutAction::CutMaxDown));
        assert_eq!(match_shortcut(KeyCode::KeyF, true), None);
    }
}
