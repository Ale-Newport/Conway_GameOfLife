//! UI state of the mode: the runner handle, the latest published frame,
//! tools, layers, selection, camera choice and keyboard shortcuts.

use std::sync::Arc;

use bevy::prelude::*;
use civilization::Resource as Stock;
use rendering::OrbitPreset;
use simulation_core::Runner;

use crate::session::{CivSession, Frame, FrameRequest, Selection};

/// Typed handle of the running session.
#[derive(Resource, Clone)]
pub struct CivRunner(pub Arc<Runner<CivSession>>);

/// Latest frame published by the session.
#[derive(Resource, Default, Clone)]
pub struct CurrentFrame {
    /// Publication counter of `frame`.
    pub seq: u64,
    /// The frame.
    pub frame: Option<Arc<Frame>>,
}

impl CurrentFrame {
    /// The frame, if one arrived.
    pub fn get(&self) -> Option<&Frame> {
        self.frame.as_deref()
    }
}

/// What a click in the world does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Select people and buildings.
    #[default]
    Inspect,
    /// Add people.
    Spawn,
    /// Turn a disc into a resource terrain with full stocks.
    AddResource,
    /// Empty the resource stocks of a disc.
    RemoveResource,
    /// Destroy the buildings of a disc and leave danger behind.
    Disaster,
}

impl Tool {
    /// Every tool in display order.
    pub const ALL: [Tool; 5] = [Tool::Inspect, Tool::Spawn, Tool::AddResource, Tool::RemoveResource, Tool::Disaster];

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Inspect => "Inspect",
            Tool::Spawn => "Spawn people",
            Tool::AddResource => "Add resources",
            Tool::RemoveResource => "Remove resources",
            Tool::Disaster => "Disaster",
        }
    }

    /// One-line explanation shown under the tool buttons and in the overlay.
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Inspect => "Click a person or building to inspect it; click empty ground to deselect.",
            Tool::Spawn => "Click the map: adults of the chosen population arrive within 3 tiles.",
            Tool::AddResource => "Click the map: tiles without buildings become the chosen resource, fully stocked.",
            Tool::RemoveResource => "Click the map: the chosen stocks in the disc drop to zero and regrow normally.",
            Tool::Disaster => "Click the map: every building in the disc is destroyed and the area becomes dangerous.",
        }
    }

    /// Whether the tool acts on a disc whose radius is set in the panel.
    pub fn has_radius(self) -> bool {
        matches!(self, Tool::AddResource | Tool::RemoveResource | Tool::Disaster)
    }
}

/// Layout of the trade-network graph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphMode {
    /// One node per group (and one for people without a group).
    #[default]
    Groups,
    /// One node per trader.
    People,
}

/// Switchable layers of the world view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layers {
    /// Territory overlay with group colours.
    pub territory: bool,
    /// Trails and roads.
    pub roads: bool,
    /// Activity badges above people.
    pub icons: bool,
    /// Trees, berry bushes and rock outcrops.
    pub vegetation: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Self { territory: true, roads: true, icons: true, vegetation: true }
    }
}

/// Everything the panels and world systems share.
#[derive(Resource, Clone, Debug)]
pub struct ViewState {
    /// Active tool.
    pub tool: Tool,
    /// Visible layers.
    pub layers: Layers,
    /// Selected person or building.
    pub selection: Selection,
    /// Camera follows the selected person.
    pub follow: bool,
    /// Camera preset.
    pub camera: OrbitPreset,
    /// Request to frame the whole map (consumed by the camera system).
    pub frame_world: bool,
    /// People added per click.
    pub spawn_count: u32,
    /// Population (index into the scenario's populations) of new people.
    pub spawn_population: u8,
    /// Resource placed by the add tool.
    pub add_resource: Stock,
    /// Resource removed by the remove tool (`None` = all).
    pub remove_resource: Option<Stock>,
    /// Radius of the area tools (tiles).
    pub radius: f32,
    /// Trade-network window (ticks).
    pub trade_window: u64,
    /// Trade-network layout.
    pub graph: GraphMode,
    /// Request last sent to the session.
    pub sent: Option<FrameRequest>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            tool: Tool::Inspect,
            layers: Layers::default(),
            selection: Selection::None,
            follow: false,
            camera: OrbitPreset::Strategic,
            frame_world: true,
            spawn_count: 10,
            spawn_population: 0,
            add_resource: Stock::Berries,
            remove_resource: None,
            radius: 4.0,
            trade_window: 240,
            graph: GraphMode::Groups,
            sent: None,
        }
    }
}

impl ViewState {
    /// The request the session should publish for.
    pub fn request(&self) -> FrameRequest {
        FrameRequest { selection: self.selection, trade_window: self.trade_window }
    }
}

/// Camera presets in cycling order.
pub const CAMERA_PRESETS: [OrbitPreset; 4] =
    [OrbitPreset::Strategic, OrbitPreset::Isometric, OrbitPreset::Aerial, OrbitPreset::FreeOrbit];

/// The next preset after `preset` (C key).
pub fn next_preset(preset: OrbitPreset) -> OrbitPreset {
    let i = CAMERA_PRESETS.iter().position(|p| *p == preset).unwrap_or(0);
    CAMERA_PRESETS[(i + 1) % CAMERA_PRESETS.len()]
}

/// Mode shortcuts (no modifiers). None of these keys is used by the shell
/// (`interface::input::SHORTCUTS`) or by the camera rig (WASD, arrows, Q/E).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeShortcut {
    /// Select a tool.
    Tool(Tool),
    /// Toggle the territory overlay.
    Territory,
    /// Toggle trails and roads.
    Roads,
    /// Toggle activity badges.
    Icons,
    /// Toggle vegetation.
    Vegetation,
    /// Follow the selected person.
    Follow,
    /// Next camera preset.
    Camera,
    /// Frame the whole map.
    FrameWorld,
    /// Clear the selection and return to the inspect tool.
    Cancel,
}

/// Key table of the mode: (key, label, action, description).
pub const MODE_SHORTCUTS: [(KeyCode, &str, ModeShortcut, &str); 13] = [
    (KeyCode::Digit1, "1", ModeShortcut::Tool(Tool::Inspect), "Inspect tool"),
    (KeyCode::Digit2, "2", ModeShortcut::Tool(Tool::Spawn), "Spawn people"),
    (KeyCode::Digit3, "3", ModeShortcut::Tool(Tool::AddResource), "Add resources"),
    (KeyCode::Digit4, "4", ModeShortcut::Tool(Tool::RemoveResource), "Remove resources"),
    (KeyCode::Digit5, "5", ModeShortcut::Tool(Tool::Disaster), "Disaster"),
    (KeyCode::KeyT, "T", ModeShortcut::Territory, "Territory overlay"),
    (KeyCode::KeyP, "P", ModeShortcut::Roads, "Trails and roads"),
    (KeyCode::KeyI, "I", ModeShortcut::Icons, "Activity badges"),
    (KeyCode::KeyV, "V", ModeShortcut::Vegetation, "Vegetation"),
    (KeyCode::KeyF, "F", ModeShortcut::Follow, "Follow the selected person"),
    (KeyCode::KeyC, "C", ModeShortcut::Camera, "Next camera preset"),
    (KeyCode::KeyG, "G", ModeShortcut::FrameWorld, "Frame the whole map"),
    (KeyCode::Escape, "Esc", ModeShortcut::Cancel, "Deselect / inspect tool"),
];

/// Action of a key pressed without modifiers.
pub fn shortcut_for(key: KeyCode) -> Option<ModeShortcut> {
    MODE_SHORTCUTS.iter().find(|(k, ..)| *k == key).map(|(_, _, action, _)| *action)
}

/// Applies a shortcut to the view state.
pub fn apply_shortcut(state: &mut ViewState, action: ModeShortcut) {
    match action {
        ModeShortcut::Tool(tool) => state.tool = tool,
        ModeShortcut::Territory => state.layers.territory = !state.layers.territory,
        ModeShortcut::Roads => state.layers.roads = !state.layers.roads,
        ModeShortcut::Icons => state.layers.icons = !state.layers.icons,
        ModeShortcut::Vegetation => state.layers.vegetation = !state.layers.vegetation,
        ModeShortcut::Follow => {
            state.follow = !state.follow && matches!(state.selection, Selection::Agent(_));
        }
        ModeShortcut::Camera => state.camera = next_preset(state.camera),
        ModeShortcut::FrameWorld => {
            state.follow = false;
            state.frame_world = true;
        }
        ModeShortcut::Cancel => {
            state.selection = Selection::None;
            state.follow = false;
            state.tool = Tool::Inspect;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface::input::{Modifiers, map_key};
    use organism_engine::AgentId;

    #[test]
    fn mode_keys_do_not_clash_with_the_shell_or_the_camera() {
        let none = Modifiers { shift: false, command: false };
        let camera = [
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
        for (i, (key, label, action, _)) in MODE_SHORTCUTS.iter().enumerate() {
            assert_eq!(map_key(*key, none), None, "{label} is a shell shortcut");
            assert!(!camera.contains(key), "{label} moves the camera");
            assert_eq!(shortcut_for(*key), Some(*action));
            assert!(MODE_SHORTCUTS[i + 1..].iter().all(|(k, ..)| k != key), "{label} is bound twice");
        }
        assert_eq!(shortcut_for(KeyCode::KeyZ), None);
    }

    #[test]
    fn shortcuts_change_the_state() {
        let mut s = ViewState::default();
        apply_shortcut(&mut s, ModeShortcut::Tool(Tool::Disaster));
        assert_eq!(s.tool, Tool::Disaster);
        let territory = s.layers.territory;
        apply_shortcut(&mut s, ModeShortcut::Territory);
        assert_ne!(s.layers.territory, territory);
        apply_shortcut(&mut s, ModeShortcut::Follow);
        assert!(!s.follow, "nothing to follow without a selected person");
        s.selection = Selection::Agent(AgentId(3));
        apply_shortcut(&mut s, ModeShortcut::Follow);
        assert!(s.follow);
        apply_shortcut(&mut s, ModeShortcut::Cancel);
        assert_eq!((s.selection, s.follow, s.tool), (Selection::None, false, Tool::Inspect));
        let start = s.camera;
        for _ in 0..CAMERA_PRESETS.len() {
            apply_shortcut(&mut s, ModeShortcut::Camera);
        }
        assert_eq!(s.camera, start, "the preset cycle wraps");
        s.frame_world = false;
        apply_shortcut(&mut s, ModeShortcut::FrameWorld);
        assert!(s.frame_world);
        assert_eq!(s.request().trade_window, 240);
    }

    #[test]
    fn every_tool_has_text() {
        for tool in Tool::ALL {
            assert!(!tool.label().is_empty() && tool.hint().ends_with('.'));
        }
        assert!(Tool::Disaster.has_radius() && !Tool::Spawn.has_radius());
    }
}
