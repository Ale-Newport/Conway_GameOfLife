//! # view-conway — the Classic Conway mode
//!
//! Conway's Game of Life, exactly B3/S23, as a professional tool
//! (`AppMode::ClassicConway`), built on `cellular_automata::LifeSim` (model
//! id `life.classic`) and the shared grid renderer. Register it after
//! `InterfacePlugin`:
//!
//! ```ignore
//! app.add_plugins(view_conway::Plugin);
//! ```
//!
//! | Part | Module |
//! |---|---|
//! | Runner, live descriptor, scenarios, live world reads | [`session`] |
//! | Camera, cell surface, view requests, gizmos, stamp ghost | [`world`] |
//! | Tools, selection, clipboard, shortcuts | [`tools`] |
//! | Queued actions (selection ops, paste, Hashlife jumps, engines) | [`commands`] |
//! | Pattern library | [`library`] |
//! | Patterns window: library browser, RLE import and export | [`window`] |
//! | Tools / Properties panels and the overlay | [`panels`] |
//! | Render styles and legends | [`style`] |
//! | Period analysis on a copy of the world | [`analysis`] |
//! | Tutorial and concepts | [`tutorial`] |
//! | Pure geometry and text helpers | [`geometry`], [`labels`] |
//!
//! What the mode provides:
//!
//! * **Worlds** — every topology of the model (bounded, torus, Klein bottle,
//!   cross-surface, infinite plane) and size, the nine Classic Conway
//!   scenarios in the top-bar picker, and the engine in use (dense,
//!   bit-packed, sparse, Hashlife or automatic, with the reason for the
//!   automatic choice) switchable from the Engine card.
//! * **Rendering** — the grid renderer with visible-region view requests:
//!   one texel per `2^k × 2^k` cells when zoomed out (from a whole Hashlife
//!   pattern down to single cells with a cell grid), eight styles (classic,
//!   flat, circles, neon, age, activity trails, heat, density), five colour
//!   schemes and the world border of finite worlds.
//! * **Editing** — draw / erase strokes, lines, rectangles, random fills,
//!   selection with copy / cut / paste / clear / move / rotate / flip, and a
//!   pattern stamp with rotation, mirror and a ghost preview; all through
//!   `LifeCommand`s, one undo step per gesture (undo / redo via the shell).
//! * **Patterns** — the catalog browser (categories, search, thumbnails,
//!   metadata, licence and provenance), RLE / plaintext / Life 1.0x import
//!   with line / column errors and RLE export to the clipboard or the
//!   user's exports folder.
//! * **Time** — generations per tick and Hashlife jumps of 2^k generations
//!   (k ≤ 40); generation numbers are shown exactly.
//! * **Metrics** — population, births / deaths, bounding box, density and
//!   generation rate with plots, Hashlife memo statistics and a period
//!   analysis (still life, oscillator, spaceship with displacement).
//! * **Tutorial** — seventeen steps with actions placing catalog patterns,
//!   and a concepts page.
//!
//! Keyboard shortcuts (in addition to the shell's): 1–7 tools, Q / E
//! rotate, M mirror, `[` / `]` brush, F frame, G grid, V / Shift+V style,
//! P library, I import, J jump, Ctrl/Cmd+C / X / V, Delete, Esc. The
//! shell's `=` / `-` keep changing the speed (they are withheld from the
//! camera).

pub mod analysis;
pub mod commands;
pub mod geometry;
pub mod labels;
pub mod library;
pub mod panels;
pub mod session;
pub mod style;
pub mod tools;
pub mod tutorial;
pub mod window;
pub mod world;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use interface::{AppMode, ModeAppExt, ModeCard, ModeStatus, PanelSlot, ShellSet, TutorialAppExt};
use rendering::{GridCameraPlugin, GridCameraSystems, GridRenderPlugin};

pub use session::{ConwayRunner, ConwayScenarios, DEFAULT_SCENARIO, LiveWorld, LoadRequest, LoadRequests};
pub use style::{CellColors, RenderStyle, ViewSettings};
pub use tools::{Selection, Tool, ToolSettings};

/// The mode implemented by this crate.
pub const MODE: AppMode = AppMode::ClassicConway;

/// Ordering of the mode's `Update` systems (all run only in [`MODE`], after
/// the shell polled the session and the camera updated
/// [`rendering::VisibleCells`]).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConwayViewSystems {
    /// Scenario loading, tutorial actions, live-world reads.
    Session,
    /// Pointer tools and shortcuts.
    Input,
    /// Queued commands and asynchronous results.
    Commands,
    /// View requests and drawing of the latest view.
    View,
    /// Gizmos and the stamp ghost.
    Draw,
}

/// The Classic Conway mode.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConwayViewPlugin;

/// Short alias: `app.add_plugins(view_conway::Plugin)`.
pub type Plugin = ConwayViewPlugin;

impl bevy::app::Plugin for ConwayViewPlugin {
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
            description: "Conway's Game of Life, exactly B3/S23, on bounded, toroidal, Klein-bottle, cross-surface \
                          or infinite worlds, with dense, bit-packed, sparse and Hashlife engines, editing tools and \
                          an 80-pattern library."
                .to_string(),
            ..ModeCard::defaults(MODE)
        };
        app.register_mode(card)
            .register_tutorial(MODE, tutorial::tutorial())
            .insert_resource(session::ConwayScenarios::builtin())
            .init_resource::<library::ConwayLibrary>()
            .init_resource::<session::RandomizeDensity>()
            .init_resource::<session::LiveWorld>()
            .init_resource::<session::LiveSync>()
            .init_resource::<session::LoadRequests>()
            .init_resource::<session::WorldLoaded>()
            .init_resource::<world::ViewRequestState>()
            .init_resource::<world::LatestView>()
            .init_resource::<world::CameraFit>()
            .init_resource::<world::OverlayFootprint>()
            .init_resource::<style::ViewSettings>()
            .init_resource::<tools::ToolSettings>()
            .init_resource::<tools::Stamp>()
            .init_resource::<tools::Hover>()
            .init_resource::<tools::DragState>()
            .init_resource::<tools::Selection>()
            .init_resource::<tools::Clipboard>()
            .init_resource::<commands::ModeCommands>()
            .init_resource::<commands::TimeSettings>()
            .init_resource::<commands::JumpState>()
            .init_resource::<window::PatternsWindow>()
            .init_resource::<analysis::Analysis>()
            .add_mode_panel(MODE, PanelSlot::Left, panels::tools_panel)
            .add_mode_panel(MODE, PanelSlot::Right, panels::properties_panel)
            .add_mode_panel(MODE, PanelSlot::Overlay, panels::overlay_panel)
            .add_systems(Startup, (library::load_library, session::fill_scenarios))
            .add_systems(OnEnter(MODE), (world::spawn_world, session::enter_session).chain())
            .add_systems(OnExit(MODE), session::exit_session)
            .configure_sets(
                Update,
                (
                    ConwayViewSystems::Session,
                    ConwayViewSystems::Input,
                    ConwayViewSystems::Commands,
                    ConwayViewSystems::View,
                    ConwayViewSystems::Draw,
                )
                    .chain()
                    .after(ShellSet::Poll)
                    .after(GridCameraSystems::Visible)
                    .run_if(in_state(MODE)),
            )
            .add_systems(
                Update,
                (
                    world::frame_camera.before(GridCameraSystems::Control).run_if(in_state(MODE)),
                    (
                        session::collect_scenario_picks,
                        tutorial::tutorial_actions,
                        session::load_requests,
                        session::poll_live_world,
                        tools::poll_selection,
                        session::sync_randomize_density,
                    )
                        .chain()
                        .in_set(ConwayViewSystems::Session),
                    (tools::pointer_tools, tools::mode_shortcuts).chain().in_set(ConwayViewSystems::Input),
                    (commands::run_mode_commands, commands::poll_async_commands, analysis::poll_analysis)
                        .chain()
                        .in_set(ConwayViewSystems::Commands),
                    (world::request_visible_region, world::sync_appearance, world::draw_latest_view)
                        .chain()
                        .in_set(ConwayViewSystems::View),
                    (world::draw_gizmos, world::update_ghost, world::update_selection_fill)
                        .in_set(ConwayViewSystems::Draw),
                ),
            )
            .add_systems(EguiPrimaryContextPass, window::patterns_window.run_if(in_state(MODE)));
    }
}
