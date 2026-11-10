//! # view-continuous — the Continuous Artificial Life mode
//!
//! The laboratory for continuous cellular automata (`AppMode::ContinuousLife`):
//! Lenia, Expanded Lenia, SmoothLife and Flow-Lenia from the `continuous`
//! crate, integrated into the application shell following
//! `docs/UI_SHELL.md`.
//!
//! ```no_run
//! # use bevy::prelude::*;
//! # let mut app = App::new();
//! app.add_plugins(view_continuous::ContinuousViewPlugin);
//! ```
//!
//! What the mode provides:
//!
//! * **Session** — a [`simulation_core::Runner`] behind
//!   [`interface::ActiveSession`] with Clear and Randomize; the built-in
//!   scenarios in the top-bar picker (grouped by model) and the model buttons
//!   of the Tools panel, loaded into the same runner (`Control::Replace`).
//!   The runner publishes a display-ready descriptor of the running model
//!   ([`display_descriptor`]: glyphs egui can draw and "time unit" as the
//!   tick unit).
//! * **World** — a grid camera (pan, zoom towards the pointer, framing) that
//!   requests only the visible region from the model, at a level of detail
//!   that keeps one sample per pixel; single planes are drawn by the shared
//!   grid renderer with a continuous colormap (Lenia blue, viridis, inferno,
//!   magma, plasma, turbo-like, grayscale; optional bilinear smoothing), the
//!   state of multi-channel worlds as an additive RGB composite. Layers:
//!   state, potential (SmoothLife: fillings) and growth (Flow-Lenia:
//!   affinity), per channel or plane.
//! * **Tools** — inspect, a soft brush and eraser (value, radius, softness,
//!   channel; one undo step per stroke) and a creature stamp with rotation,
//!   scale and blend previews. All world input is gated by
//!   [`rendering::WorldInput`].
//! * **Panels** — Tools (model, tool, view), Properties (inspector, kernel
//!   profile and growth function plots that follow the live rule, live
//!   parameters of the rule and the selected kernel, mass and speed series,
//!   channel statistics) and an overlay (legend, time step, regime, scale
//!   bar, hover readout). Gizmos draw the world border, the centroid trail
//!   and the tool outlines.
//! * **Tutorial** — twelve steps (eleven with an action button) and a
//!   concepts page.
//!
//! Rendering note: the RGB composite uses a new image whenever its size or
//! sampler changes, because sprite materials are cached per image.
//!
//! Keyboard shortcuts (in addition to the shell's): 1–4 tools, Q / E rotate
//! the stamp, `[` / `]` resize the brush or stamp, F frames the world, C
//! cycles colormaps, L layers, T toggles the trail, G the cell grid. The
//! shell's `=`/`-` keep changing the speed: they are withheld from the grid
//! camera, which would otherwise also zoom with them.

mod field;
mod panels;
mod plots;
mod regime;
mod session;
mod tools;
mod tutorial;
mod world;

use bevy::prelude::*;
use interface::{AppMode, ModeAppExt, ModeCard, ModeStatus, PanelSlot, ShellSet, TutorialAppExt};
use rendering::{GridCameraPlugin, GridCameraSystems, GridRenderPlugin};

pub use field::{
    CHANNEL_COLORS, cell_point_to_world, composite_rgba, display_range, plane_frame, region_request, scale_bar,
    stroke_dabs, trail_segments,
};
pub use panels::live_parameter_descriptor;
pub use plots::{GrowthChart, growth_chart, kernel_curves, positive_growth_band};
pub use regime::{Regime, classify as classify_regime};
pub use session::{
    Catalog, ContinuousRunner, DEFAULT_SCENARIO, Family, LoadRequests, LoadedWorld, display_descriptor, displayable,
};
pub use tools::{Hover, MODE_SHORTCUTS, ModeShortcut, Tool, ToolSettings};
pub use tutorial::{TutorialCommand, tutorial};
pub use world::{CameraFit, DisplayPlane, LatestView, ViewSettings};

/// The mode implemented by this crate.
pub const MODE: AppMode = AppMode::ContinuousLife;

/// Ordering of the mode's `Update` systems (all run only in [`MODE`], after
/// the shell polled the session and after the camera updated
/// [`rendering::VisibleCells`]).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContinuousViewSystems {
    /// Scenario loading, tutorial actions, live rule synchronisation.
    Session,
    /// Pointer tools and keyboard shortcuts.
    Input,
    /// View requests and drawing of the latest view.
    View,
    /// Gizmos and previews.
    Draw,
}

/// The Continuous Artificial Life mode: register it with
/// `app.add_plugins(view_continuous::ContinuousViewPlugin)` after
/// `InterfacePlugin`.
pub struct ContinuousViewPlugin;

impl Plugin for ContinuousViewPlugin {
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
            description: "Continuous states, smooth kernels and growth functions: gliding Lenia creatures, \
                          multi-channel Expanded Lenia, SmoothLife gliders and mass-conserving Flow-Lenia."
                .to_string(),
            ..ModeCard::defaults(MODE)
        };
        app.register_mode(card)
            .register_tutorial(MODE, tutorial::tutorial())
            .insert_resource(session::Catalog::builtin())
            .init_resource::<session::LoadRequests>()
            .init_resource::<session::LiveSync>()
            .init_resource::<world::ViewSettings>()
            .init_resource::<world::LatestView>()
            .init_resource::<world::ViewRequestState>()
            .init_resource::<world::CameraFit>()
            .init_resource::<tools::ToolSettings>()
            .init_resource::<tools::Hover>()
            .init_resource::<tools::StrokeState>()
            .init_resource::<tools::StampImages>()
            .add_mode_panel(MODE, PanelSlot::Left, panels::tools_panel)
            .add_mode_panel(MODE, PanelSlot::Right, panels::properties_panel)
            .add_mode_panel(MODE, PanelSlot::Overlay, panels::overlay_panel)
            .add_systems(Startup, session::fill_scenarios)
            .add_systems(OnEnter(MODE), (world::spawn_world, session::enter_session).chain())
            .add_systems(OnExit(MODE), session::exit_session)
            .configure_sets(
                Update,
                (
                    ContinuousViewSystems::Session,
                    ContinuousViewSystems::Input,
                    ContinuousViewSystems::View,
                    ContinuousViewSystems::Draw,
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
                        session::collect_scenario_picks,
                        tutorial::tutorial_actions,
                        session::load_requested_scenarios,
                        session::sync_live_world,
                    )
                        .chain()
                        .in_set(ContinuousViewSystems::Session),
                    (tools::pointer_tools, tools::mode_shortcuts).in_set(ContinuousViewSystems::Input),
                    (world::request_visible_region, world::sync_appearance, world::draw_latest_view)
                        .chain()
                        .in_set(ContinuousViewSystems::View),
                    (world::draw_gizmos, world::update_stamp_preview).in_set(ContinuousViewSystems::Draw),
                ),
            )
            .add_systems(PostUpdate, world::place_composite.before(TransformSystems::Propagate).run_if(in_state(MODE)));
    }
}
