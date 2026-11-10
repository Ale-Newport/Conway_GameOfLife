//! # interface
//!
//! The Emergence Lab application shell (Bevy + egui): dashboard, mode
//! registry and states, top bar, collapsible side panels, bottom time
//! controls driven by the active [`simulation_core::SessionControl`],
//! parameter editors generated from [`simulation_core::ParamSpec`]s, live
//! metric plots, save / load, exports, tutorials, credits and themes.
//!
//! Mode (view) crates integrate through [`mode::ModeAppExt`],
//! [`mode::ActiveSession`], [`mode::ModeScenarios`],
//! [`mode::ModePlaceholders`] and [`tutorial::TutorialAppExt`]; the
//! contract is documented in `docs/UI_SHELL.md`.
//!
//! Frame flow:
//!
//! * `PreUpdate` — [`rendering::WorldInput`] is written from the previous
//!   frame's layout and egui input state.
//! * `Update` — the session is polled (status, metrics, errors), keyboard
//!   shortcuts become [`commands::ShellCommand`]s, commands execute.
//! * `EguiPrimaryContextPass` (inside `PostUpdate`) — one exclusive system
//!   draws the shell and runs the mode's panel systems.
//! * `PostUpdate` — world camera viewports follow the central rectangle
//!   (before `CameraUpdateSystems`, so projections use the new size).
//! * `Last` — screenshots and frame sequences are requested and tracked.

pub mod commands;
pub mod credits;
mod dashboard;
pub mod error;
pub mod export;
pub mod format;
pub mod icons;
pub mod input;
mod jobs;
pub mod layout;
pub mod mode;
pub mod previews;
pub mod saves;
pub mod session;
mod shell;
mod state;
pub mod theme;
pub mod toasts;
pub mod tutorial;
pub mod widgets;

use std::path::PathBuf;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, CameraUpdateSystems, ClearColorConfig};
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;
use bevy_egui::{
    EguiGlobalSettings, EguiPlugin, EguiPostUpdateSet, EguiPreUpdateSet, EguiPrimaryContextPass, PrimaryEguiContext,
};
use rendering::WorldInput;

pub use commands::{RightPanelTab, ShellCommand, ShellWindow};
pub use credits::AssetRoot;
pub use error::InterfaceError;
pub use export::{ScreenshotFinished, ScreenshotTracker};
pub use layout::ShellLayout;
pub use mode::{
    ActiveSession, AppMode, ModeAppExt, ModeCard, ModePanels, ModePlaceholders, ModeRegistry, ModeScenarios,
    ModeStatus, PanelSlot, PanelSystem, Placeholder, ScenarioEntry, ScenarioList, ScenarioSelected,
};
pub use saves::DataLocation;
pub use session::{MetricsHistory, SessionStatus, Telemetry};
pub use theme::{Palette, ThemeKind, ThemeSettings};
pub use toasts::{ToastLevel, Toasts};
pub use tutorial::{Tutorial, TutorialAction, TutorialAppExt, TutorialPage, TutorialRegistry};

/// Camera order of the dedicated egui camera (above every world camera).
pub const UI_CAMERA_ORDER: isize = 1_000;

/// System sets of the shell, for ordering mode systems around it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ShellSet {
    /// `Update`: session polling (status, metrics, errors). Mode systems
    /// that read [`SessionStatus`] or [`MetricsHistory`] run after it.
    Poll,
    /// `Update`: shortcuts and command execution.
    Commands,
}

/// Marker of the shell's dedicated egui camera.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct ShellUiCamera;

/// The application shell.
#[derive(Clone, Debug, Default)]
pub struct InterfacePlugin {
    /// Folder holding the application assets (for credits). The binary
    /// resolves it; see `apps/desktop`.
    pub asset_root: PathBuf,
    /// Mode to open after startup (ignored, with a warning, if it is not
    /// registered).
    pub initial_mode: Option<AppMode>,
}

impl Plugin for InterfacePlugin {
    fn build(&self, app: &mut App) {
        // A dedicated egui camera (spawned below) hosts the primary context,
        // so world cameras can have viewports without clipping the UI.
        if !app.world().contains_resource::<EguiGlobalSettings>() {
            app.insert_resource(EguiGlobalSettings { auto_create_primary_context: false, ..default() });
        }
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin::default());
        }
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        if !app.world().contains_resource::<saves::DataLocation>() {
            app.insert_resource(saves::DataLocation::resolve());
        }
        app.init_state::<AppMode>()
            .insert_resource(AssetRoot(self.asset_root.clone()))
            .insert_resource(InitialMode(self.initial_mode))
            .init_resource::<ModeRegistry>()
            .init_resource::<mode::ModePanelRegistry>()
            .init_resource::<ModeScenarios>()
            .init_resource::<ModePlaceholders>()
            .init_resource::<TutorialRegistry>()
            .init_resource::<ThemeSettings>()
            .init_resource::<state::ShellState>()
            .init_resource::<ShellLayout>()
            .init_resource::<SessionStatus>()
            .init_resource::<MetricsHistory>()
            .init_resource::<Telemetry>()
            .init_resource::<Toasts>()
            .init_resource::<saves::PendingSave>()
            .init_resource::<jobs::BackgroundJobs>()
            .init_resource::<ScreenshotTracker>()
            .init_resource::<export::FrameRecorder>()
            .init_resource::<WorldInput>()
            .add_message::<ShellCommand>()
            .add_message::<ScenarioSelected>()
            .add_message::<TutorialAction>()
            .add_message::<ScreenshotFinished>()
            .configure_sets(Update, (ShellSet::Poll, ShellSet::Commands).chain())
            .add_systems(Startup, (spawn_ui_camera, credits::load_credits_system, open_initial_mode))
            .add_systems(
                PreUpdate,
                (
                    input::update_world_input.after(InputSystems),
                    input::reserve_tab_for_shell
                        .after(EguiPreUpdateSet::ProcessInput)
                        .before(EguiPreUpdateSet::BeginPass),
                ),
            )
            .add_systems(
                Update,
                (
                    (session::poll_session, session::update_telemetry, toasts::age_toasts, sync_clear_color)
                        .in_set(ShellSet::Poll),
                    (input::keyboard_shortcuts, commands::execute_shell_commands, export::screenshot_command)
                        .chain()
                        .in_set(ShellSet::Commands),
                    (saves::poll_pending_save, jobs::poll_background_jobs).chain().after(ShellSet::Poll),
                    state::reset_mode_ui.run_if(state_changed::<AppMode>).before(ShellSet::Commands),
                ),
            )
            .add_systems(EguiPrimaryContextPass, shell::shell_ui)
            // After the UI pass computed the central rectangle and before the
            // cameras compute their projections, so a resized panel never
            // leaves a frame with a stretched world.
            .add_systems(
                PostUpdate,
                layout::apply_world_viewports.after(EguiPostUpdateSet::EndPass).before(CameraUpdateSystems),
            )
            .add_systems(Last, (export::record_frames, export::track_screenshots).chain());
    }
}

/// Test helper: runs one headless egui frame and discards its texture
/// uploads (egui asserts that they are handled).
#[cfg(test)]
pub(crate) fn run_headless_ui(
    ctx: &bevy_egui::egui::Context,
    ui: impl FnMut(&mut bevy_egui::egui::Ui),
) -> bevy_egui::egui::FullOutput {
    let mut output = ctx.run_ui(Default::default(), ui);
    output.textures_delta.clear();
    output
}

#[derive(Resource)]
struct InitialMode(Option<AppMode>);

fn spawn_ui_camera(mut commands: Commands) {
    commands.spawn((
        Name::new("Shell UI camera"),
        ShellUiCamera,
        PrimaryEguiContext,
        Camera2d,
        RenderLayers::none(),
        Camera {
            order: UI_CAMERA_ORDER,
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
    ));
}

fn open_initial_mode(
    initial: Res<InitialMode>,
    registry: Res<ModeRegistry>,
    mut next: ResMut<NextState<AppMode>>,
    mut toasts: ResMut<Toasts>,
) {
    let Some(mode) = initial.0 else { return };
    if registry.is_registered(mode) {
        // `--mode dashboard` must not re-run the dashboard's enter/exit.
        next.set_if_different(mode);
    } else {
        let message = format!("{} is still in development; showing the dashboard instead.", registry.title(mode));
        warn!("{message}");
        toasts.warning(message);
    }
}

/// Keeps the default clear colour of world cameras in line with the theme.
fn sync_clear_color(
    theme: Res<ThemeSettings>,
    state: Res<State<AppMode>>,
    registry: Res<ModeRegistry>,
    clear: Option<ResMut<ClearColor>>,
) {
    let Some(mut clear) = clear else { return };
    let background = theme::bevy_color(Palette::new(theme.kind, registry.accent(*state.get())).background);
    if clear.0 != background {
        clear.0 = background;
    }
}
