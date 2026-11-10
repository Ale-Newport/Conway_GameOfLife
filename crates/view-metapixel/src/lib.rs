//! # view-metapixel — Conway within Conway
//!
//! The laboratory for Brice Due's **OTCA metapixel** (`AppMode::ConwayWithinConway`):
//! the real construction, every tile a 2,048 × 2,048 region of one Conway's
//! Life pattern, run cell by cell under B3/S23 with Hashlife by the
//! `metapixel` crate (model `life.metapixel`), integrated into the shell
//! following `docs/UI_SHELL.md`.
//!
//! ```no_run
//! # use bevy::prelude::*;
//! # let mut app = App::new();
//! app.add_plugins(view_metapixel::Plugin);
//! ```
//!
//! What the mode provides:
//!
//! * **Session** — a [`simulation_core::Runner`] behind
//!   [`interface::ActiveSession`] (with Clear and Randomize) whose
//!   descriptor follows the running macro rule in the shell's Rules tab;
//!   the seven built-in scenarios in the top-bar picker, loaded with
//!   `Control::Replace`. The OTCA tiles are read from
//!   `<assets>/patterns/otca/` on a background thread (GPL-2.0-or-later
//!   data, never embedded).
//! * **World** — two linked views of the one simulation: the **physical
//!   cells** (the model's Hashlife grid view of the visible region at a
//!   power-of-two level of detail) and the **macro layer** (one texel per
//!   tile: the exact readout, the live core counts, or the clearly labelled
//!   accelerated prediction). Zooming is continuous; in Auto the macro layer
//!   fades in between 128 and 32 pixels per tile and an overlay badge always
//!   names the layer on screen.
//! * **Tools** — inspect tiles, draw macro cells, stamp library patterns as
//!   macro patterns (every edit rebuilds at generation 0, one undo step per
//!   drag); program the macro rule (B/S digits, presets) and neighbourhood
//!   (compass); grid size, seed and random soups; Hashlife step size,
//!   jumps to the next readout, cycle progress and the cost of each tick.
//! * **Properties** — tile inspector (states, core count, rule bits,
//!   neighbour channels, parts) with a mini map; validation of every exact
//!   readout against the prediction and of populations and core counts
//!   against Golly's golden table (`otca-spec.json`); construction and
//!   Hashlife statistics and credits.
//! * **Tutorial** — twelve steps with actions and a concepts page.
//!
//! Keyboard shortcuts (besides the shell's): 1–3 tools, J next readout,
//! `[` / `]` step size, L layer, M macro source, F frame grid, T frame tile,
//! C rule table, G tile grid, O tile parts, Esc clear selection. The
//! shell's `=`/`-` keep changing the speed (they are withheld from the
//! camera).

mod geometry;
mod golden;
mod macro_layer;
mod panels;
mod rule_editor;
mod session;
mod timing;
mod tools;
mod tutorial;
mod world;

use bevy::prelude::*;
use interface::{AppMode, ModeAppExt, ModeCard, ModeStatus, PanelSlot, ShellSet, TutorialAppExt};
use rendering::grid::GridRenderSystems;
use rendering::{GridCameraPlugin, GridCameraSystems, GridRenderPlugin};

pub use geometry::{
    GridShape, MACRO_FULL_PX, MICRO_FULL_PX, ScaleBar, ScreenRect, auto_macro_opacity, display_density, fit_clear_of,
    micro_frame, micro_request, pixels_per_tile, scale_bar,
};
pub use golden::{Check, GoldenCase, GoldenRow, GoldenTable};
pub use macro_layer::{LayerMode, MacroSource, TileShade, macro_texture, tile_shade};
pub use panels::PanelSections;
pub use panels::properties::PropertiesTab;
pub use rule_editor::RuleDraft;
pub use session::{
    Catalog, DEFAULT_SCENARIO, Lifecycle, LoadRequests, MpSession, SimData, SimInfo, Tiles, TilesState,
    jump_to_next_readout,
};
pub use tools::{MODE_SHORTCUTS, ModeShortcut, Tool, ToolState};
pub use tutorial::tutorial;
pub use world::{CameraFit, LayerState, MpCamera, OverlayFootprint, ViewSettings};

/// The mode implemented by this crate.
pub const MODE: AppMode = AppMode::ConwayWithinConway;

/// Ordering of the mode's `Update` systems (all run only in [`MODE`],
/// after the shell polled the session and the camera updated
/// [`rendering::VisibleCells`]).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum MetapixelViewSystems {
    /// Scenario loading, tutorial actions, model data.
    Session,
    /// Pointer tools and keyboard shortcuts.
    Input,
    /// View requests and drawing of the latest view.
    View,
    /// Gizmos.
    Draw,
}

/// The Conway-within-Conway mode. Register it with
/// `app.add_plugins(view_metapixel::Plugin)` after `InterfacePlugin`.
pub struct MetapixelViewPlugin;

/// Short name of [`MetapixelViewPlugin`].
pub use self::MetapixelViewPlugin as Plugin;

impl bevy::app::Plugin for MetapixelViewPlugin {
    fn build(&self, app: &mut App) {
        // Shared rendering plugins; another mode may have added them already.
        if !app.is_plugin_added::<GridRenderPlugin>() {
            app.add_plugins(GridRenderPlugin);
        }
        if !app.is_plugin_added::<GridCameraPlugin>() {
            app.add_plugins(GridCameraPlugin);
        }
        let card = ModeCard {
            status: ModeStatus::Ready,
            description: "Brice Due's OTCA metapixel run exactly: every tile is 2,048 × 2,048 cells of one \
                          B3/S23 pattern advanced by Hashlife, and every 35,328 generations the tiles show the \
                          next state of a programmable Life-like rule."
                .to_string(),
            ..ModeCard::defaults(MODE)
        };
        app.register_mode(card)
            .register_tutorial(MODE, tutorial::tutorial())
            .insert_resource(session::Catalog::builtin())
            .init_resource::<session::Tiles>()
            .init_resource::<session::SimData>()
            .init_resource::<session::Lifecycle>()
            .init_resource::<session::LoadRequests>()
            .init_resource::<world::ViewSettings>()
            .init_resource::<world::LayerState>()
            .init_resource::<world::ViewRequestState>()
            .init_resource::<world::CameraFit>()
            .init_resource::<world::OverlayFootprint>()
            .init_resource::<tools::ToolState>()
            .init_resource::<panels::properties::PropertiesTab>()
            .init_resource::<panels::PanelSections>()
            .add_mode_panel(MODE, PanelSlot::Left, panels::tools_panel::tools_panel)
            .add_mode_panel(MODE, PanelSlot::Right, panels::properties::properties_panel)
            .add_mode_panel(MODE, PanelSlot::Overlay, panels::overlay::overlay_panel)
            .add_systems(Startup, session::startup)
            .add_systems(OnEnter(MODE), (world::spawn_world, session::enter_session).chain())
            .add_systems(OnExit(MODE), session::exit_session)
            .configure_sets(
                Update,
                (
                    MetapixelViewSystems::Session,
                    MetapixelViewSystems::Input,
                    MetapixelViewSystems::View,
                    MetapixelViewSystems::Draw,
                )
                    .chain()
                    .after(ShellSet::Poll)
                    .after(GridCameraSystems::Visible)
                    .run_if(in_state(MODE)),
            )
            .add_systems(
                Update,
                (
                    world::frame_world.before(GridCameraSystems::Control).run_if(in_state(MODE)),
                    (
                        tutorial::tutorial_actions,
                        session::collect_scenario_requests,
                        session::drive_session,
                        session::update_placeholder,
                        session::poll_data,
                    )
                        .chain()
                        .in_set(MetapixelViewSystems::Session),
                    (tools::pointer_tools, tools::mode_shortcuts).chain().in_set(MetapixelViewSystems::Input),
                    (
                        world::update_layer_state,
                        world::request_view,
                        world::sync_appearance,
                        world::draw_micro,
                        world::draw_macro,
                    )
                        .chain()
                        .in_set(MetapixelViewSystems::View),
                    world::draw_gizmos.in_set(MetapixelViewSystems::Draw),
                ),
            )
            .add_systems(
                PostUpdate,
                world::place_layers
                    .after(GridRenderSystems::Place)
                    .before(TransformSystems::Propagate)
                    .run_if(in_state(MODE)),
            );
    }
}
