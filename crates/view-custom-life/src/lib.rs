//! # view-custom-life — the Custom Life Creator mode
//!
//! Design a life system without code — entity types, relationships, rules
//! (visual builder or DSL text), Rhai scripts, the world and its fields —
//! and watch the population it produces. Integrate it with one line (after
//! `InterfacePlugin`):
//!
//! ```ignore
//! app.add_plugins(view_custom_life::CustomLifeViewPlugin);
//! ```
//!
//! | Part | Module |
//! |---|---|
//! | Runner with a live descriptor, presets, data polling | [`session`] |
//! | Draft editing helpers (renames, sync status, detach) | [`draft`] |
//! | System editor window and its tabs, apply semantics | [`editor`] |
//! | Visual rule builder model ↔ DSL AST | [`blocks`] |
//! | DSL / Rhai highlighting and `line:col` mapping | [`highlight`] |
//! | Relationship graph (force layout, painter) | [`graph`] |
//! | World rendering, camera, tools, shortcuts | [`world`] |
//! | Viewport overlay: phase, legend, scale bar, glyphs, hover | [`overlay`] |
//! | Left tools panel / right properties panel | [`tools_panel`] / [`properties`] |
//! | Save, load, export, import | [`files`] |
//! | Tutorial pages and step actions | [`tutorial`] |
//!
//! The mode follows `docs/UI_SHELL.md`: it registers its card, inserts
//! `ActiveSession` on enter, fills the scenario picker with the seven
//! presets, contributes left / right / overlay panels, renders through a
//! `WorldCamera` and gates world input on `WorldInput`.
//!
//! Behaviour worth knowing:
//!
//! * Leaving the mode keeps a snapshot of the world and the editor's draft;
//!   entering again continues from them (a design is never discarded by a
//!   trip to the dashboard). The runner thread itself stops on exit.
//! * Changes made outside the editor (Parameters tab, undo / redo, loading
//!   a save) flow into the draft, so Apply never reverts them silently.
//! * The served descriptor is named after the running system
//!   ([`session::served_descriptor`]).

pub mod blocks;
pub mod colors;
pub mod coords;
pub mod draft;
pub mod editor;
pub mod files;
pub mod glyphs;
pub mod graph;
pub mod highlight;
pub mod overlay;
pub mod properties;
pub mod session;
pub mod state;
pub mod tools_panel;
pub mod tutorial;
pub mod world;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use interface::{AppMode, ModeAppExt, ModeCard, ModeStatus, PanelSlot, ShellSet, TutorialAppExt};
use rendering::GridRenderPlugin;

pub use editor::{ClEditor, EditorTab};
pub use session::{ClData, ClRunner, ClScenarios};
pub use state::{ClUi, Tool};

/// The application state of this mode.
pub const MODE: AppMode = AppMode::CustomLifeCreator;

/// The Custom Life Creator mode.
#[derive(Clone, Copy, Debug, Default)]
pub struct CustomLifeViewPlugin;

/// Short alias: `app.add_plugins(view_custom_life::Plugin)`.
pub type Plugin = CustomLifeViewPlugin;

/// System sets of the mode in `Update` (all after `ShellSet::Poll`, only
/// while the mode is active).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum CustomLifeSystems {
    /// Scenario changes, tutorial actions, data polling, draft validation.
    Data,
    /// Camera, tools and shortcuts.
    Input,
    /// Field surface, entity meshes, gizmos.
    World,
}

impl bevy::app::Plugin for CustomLifeViewPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<GridRenderPlugin>() {
            app.add_plugins(GridRenderPlugin);
        }
        app.register_mode(ModeCard { status: ModeStatus::Ready, ..ModeCard::defaults(MODE) })
            .register_tutorial(MODE, tutorial::tutorial())
            .insert_resource(session::ClScenarios::load())
            .init_resource::<session::ClData>()
            .init_resource::<session::ClResume>()
            .init_resource::<state::ClUi>()
            .init_resource::<editor::ClEditor>()
            .init_resource::<world::ClVisuals>()
            .add_mode_panel(MODE, PanelSlot::Left, tools_panel::tools_panel)
            .add_mode_panel(MODE, PanelSlot::Right, properties::properties_panel)
            .add_mode_panel(MODE, PanelSlot::Overlay, overlay::overlay_panel)
            .add_systems(Startup, session::fill_scenarios)
            .add_systems(OnEnter(MODE), (session::enter, world::spawn_world))
            .add_systems(OnExit(MODE), (session::exit, world::clear_world))
            .configure_sets(
                Update,
                (CustomLifeSystems::Data, CustomLifeSystems::Input, CustomLifeSystems::World)
                    .chain()
                    .after(ShellSet::Poll)
                    .run_if(in_state(MODE)),
            )
            .add_systems(
                Update,
                (
                    session::handle_scenario_selection,
                    tutorial::handle_actions,
                    session::poll_data,
                    editor::refresh_draft,
                )
                    .chain()
                    .in_set(CustomLifeSystems::Data),
            )
            .add_systems(Update, world::world_input.in_set(CustomLifeSystems::Input))
            .add_systems(
                Update,
                (world::update_field, world::sync_entities, world::draw_gizmos)
                    .chain()
                    .in_set(CustomLifeSystems::World),
            )
            .add_systems(EguiPrimaryContextPass, editor::editor_window.run_if(in_state(MODE)));
    }
}
