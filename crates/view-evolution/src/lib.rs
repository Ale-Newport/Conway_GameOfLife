//! # view-evolution — the Evolution Laboratory mode
//!
//! A 3D laboratory for the `evolution` model: prey and predators with
//! heritable genomes live on a diorama of the model's grid. Integrate it
//! with one line (after `InterfacePlugin`):
//!
//! ```ignore
//! app.add_plugins(view_evolution::EvolutionViewPlugin);
//! ```
//!
//! What the mode shows and how (see `docs/UI_SHELL.md` for the contract):
//!
//! | Part | Module |
//! |---|---|
//! | Runner lifecycle, scenarios, data read from the simulation thread | [`session`] |
//! | Gene → body mapping (size, speed, thermal optimum, perception) | [`morph`] |
//! | Terrain slab tinted by temperature, water, mountains, food vegetation, camera | [`scene`] |
//! | Animated procedural creatures, motion smoothing, selection rings | [`organisms`] |
//! | Picking, world tools, shortcuts, camera follow | [`interaction`] |
//! | Tools panel: replicates, tools, climate, barriers, custom genome | [`tools_panel`] |
//! | Properties: inspector, gene charts, genealogy, regions | [`properties`] |
//! | Viewport overlay: status, legend, scale bar, hover card | [`overlay`] |
//! | Gene statistics over time | [`history`] |
//! | Genealogy layout and tree canvas | [`genealogy`] |
//! | Tutorial pages and their actions | [`tutorial`] |
//!
//! The plugin adds `organism_render::OrganismRenderPlugin`,
//! `rendering::OrbitCameraPlugin` and `rendering::WaterPlugin` when the app
//! does not have them yet.

pub mod genealogy;
pub mod history;
pub mod interaction;
pub mod morph;
pub mod organisms;
pub mod overlay;
pub mod properties;
pub mod scene;
pub mod session;
pub mod tools_panel;
pub mod tutorial;
pub mod ui_state;
pub mod widgets;

use std::path::PathBuf;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use interface::{AppMode, ModeAppExt, ModeCard, ModeStatus, PanelSlot, ShellSet, TutorialAppExt};
use organism_render::{OrganismRenderPlugin, OrganismSystems};
use rendering::{OrbitCameraPlugin, WaterPlugin};

pub use session::{EvoData, EvoRunner, EvoScenarios};
pub use ui_state::{EvoUi, PropertiesTab, Tool};

/// The application state of this mode.
pub const MODE: AppMode = AppMode::EvolutionLab;

/// The Evolution Laboratory mode.
#[derive(Clone, Copy, Debug, Default)]
pub struct EvolutionViewPlugin;

/// Short alias: `app.add_plugins(view_evolution::Plugin)`.
pub type Plugin = EvolutionViewPlugin;

/// System sets of the mode in `Update` (all after `ShellSet::Poll`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum EvolutionViewSystems {
    /// Scenario changes, tutorial actions, data polling.
    Data,
    /// Environment and organism entities.
    World,
    /// Picking, tools, shortcuts.
    Input,
}

fn default_assets() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets")
}

fn count_frames(frame: Res<FrameCount>, mut ui: ResMut<EvoUi>) {
    ui.frame = frame.0;
}

impl bevy::app::Plugin for EvolutionViewPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<OrganismRenderPlugin>() {
            let root = app.world().get_resource::<interface::AssetRoot>().map_or_else(default_assets, |r| r.0.clone());
            app.add_plugins(OrganismRenderPlugin { assets_root: root });
        }
        if !app.is_plugin_added::<OrbitCameraPlugin>() {
            app.add_plugins(OrbitCameraPlugin);
        }
        if !app.is_plugin_added::<WaterPlugin>() {
            app.add_plugins(WaterPlugin);
        }
        app.register_mode(ModeCard { status: ModeStatus::Ready, ..ModeCard::defaults(MODE) })
            .register_tutorial(MODE, tutorial::tutorial())
            .insert_resource(EvoScenarios::load())
            .init_resource::<EvoData>()
            .init_resource::<EvoUi>()
            .init_resource::<scene::SceneState>()
            .init_resource::<organisms::OrganismIndex>()
            .init_resource::<organisms::MorphRegistry>()
            .init_resource::<interaction::CursorGround>()
            .init_resource::<overlay::ViewScale>()
            .add_mode_panel(MODE, PanelSlot::Left, tools_panel::tools_panel)
            .add_mode_panel(MODE, PanelSlot::Right, properties::properties_panel)
            .add_mode_panel(MODE, PanelSlot::Overlay, overlay::overlay_panel)
            .add_systems(Startup, session::fill_scenarios)
            .add_systems(OnEnter(MODE), (session::enter, scene::spawn_scene, organisms::spawn_markers))
            .add_systems(OnExit(MODE), (session::exit, organisms::clear_index))
            .configure_sets(
                Update,
                (EvolutionViewSystems::Data, EvolutionViewSystems::World, EvolutionViewSystems::Input)
                    .chain()
                    .after(ShellSet::Poll)
                    .before(OrganismSystems::Lod)
                    .run_if(in_state(MODE)),
            )
            .add_systems(
                Update,
                (
                    count_frames,
                    session::handle_scenario_selection,
                    tutorial::handle_tutorial_actions,
                    session::poll_data,
                    interaction::prune_selection,
                )
                    .chain()
                    .in_set(EvolutionViewSystems::Data),
            )
            .add_systems(
                Update,
                (
                    scene::rebuild_environment,
                    scene::recolor_ground,
                    scene::update_vegetation,
                    organisms::sync_organisms,
                    organisms::update_actions,
                    organisms::animate_motion,
                    organisms::update_markers,
                )
                    .chain()
                    .in_set(EvolutionViewSystems::World),
            )
            .add_systems(
                Update,
                (
                    interaction::hover,
                    interaction::click,
                    interaction::shortcuts,
                    interaction::follow,
                    interaction::draw_brush,
                    scene::update_camera,
                    overlay::measure_scale,
                )
                    .chain()
                    .in_set(EvolutionViewSystems::Input),
            );
    }
}
