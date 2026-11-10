//! Opens the Three-Dimensional Life mode directly inside the full shell,
//! configured like the desktop application (window, asset root, shell,
//! world input).
//!
//! ```text
//! cargo run -p view-voxel --example voxel_standalone
//! VIEW_SMOKE_SCREENSHOT=out.png VIEW_SMOKE_FRAMES=150 cargo run -p view-voxel --example voxel_standalone
//! ```
//!
//! Smoke-test variables (all optional except the screenshot path):
//!
//! | Variable | Effect |
//! |---|---|
//! | `VIEW_SMOKE_SCREENSHOT=<png>` | capture the window after `VIEW_SMOKE_FRAMES` frames and exit |
//! | `VIEW_SMOKE_FRAMES=<n>` | frames before the capture (default 120) |
//! | `VIEW_SMOKE_SCENARIO=<id>` | load a built-in scenario (as if picked in the top bar) |
//! | `VIEW_SMOKE_STEPS=<n>` | step `n` generations at frame 30 |
//! | `VIEW_SMOKE_PLAY=1` | press play at frame 30 |
//! | `VIEW_SMOKE_ACTIONS=a,b` | tutorial action ids at frame 25 (`cut:middle`, `color:age`, `view:ghost`, `camera:top`, `region:population`, `probe:x/y/z`, …) |
//! | `VIEW_SMOKE_LATE_ACTIONS=a,b` | the same, 40 frames before the capture (e.g. `camera:population` after stepping) |
//! | `VIEW_SMOKE_TAB=<name>` | `properties`, `parameters`, `rules`, `metrics`, `tutorial`, `help` |
//! | `VIEW_SMOKE_THEME=<name>` | `light` or `contrast` |
//! | `VIEW_SMOKE_HIDE_PANELS=1` | hide the side panels (Tab) |
//! | `VIEW_SMOKE_ZOOM=<f>` | egui zoom factor (e.g. `0.7` to see whole panels) |
//! | `VIEW_SMOKE_REENTER=<n>` | leave to the dashboard and re-enter the mode `n` times (1–3) before frame 25 |
//!
//! Before capturing, the smoke run checks that exactly one 3D Life session
//! (simulation thread) is alive, so a session leaked by leaving and
//! re-entering the mode fails the run.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use bevy_egui::EguiContexts;
use interface::{
    AppMode, InterfacePlugin, RightPanelTab, ScenarioSelected, ScreenshotFinished, ScreenshotTracker, ShellCommand,
    ShellWindow, ThemeKind, ThemeSettings, TutorialAction,
};
use rendering::WorldInputPlugin;

/// Asset folder: `$EMERGENCE_LAB_ASSETS`, else `<exe dir>/assets`, else the
/// workspace `assets/` (the order of `apps/desktop`).
fn asset_root() -> PathBuf {
    let candidates = std::env::var_os("EMERGENCE_LAB_ASSETS")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .chain(std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("assets"))))
        .chain([Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets")]);
    let found = candidates.into_iter().find(|p| p.is_dir()).unwrap_or_else(|| PathBuf::from("assets"));
    std::fs::canonicalize(&found).unwrap_or(found)
}

fn main() {
    let assets = asset_root();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Emergence Lab — Three-Dimensional Life".into(),
                    resolution: WindowResolution::new(1600, 1000),
                    present_mode: PresentMode::AutoVsync,
                    window_theme: Some(WindowTheme::Dark),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin { file_path: assets.to_string_lossy().into_owned(), ..default() }),
    )
    .add_plugins((
        WorldInputPlugin,
        InterfacePlugin { asset_root: assets, initial_mode: Some(AppMode::ThreeDimensionalLife) },
        view_voxel::VoxelViewPlugin,
    ));
    if let Some(path) = std::env::var_os("VIEW_SMOKE_SCREENSHOT").filter(|p| !p.is_empty()) {
        let frames = std::env::var("VIEW_SMOKE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
        app.insert_resource(Smoke { path: std::path::absolute(path).unwrap_or_default(), frames, frame: 0 })
            .add_systems(Update, (smoke, zoom));
    }
    app.run();
}

#[derive(Resource)]
struct Smoke {
    path: PathBuf,
    frames: u32,
    frame: u32,
}

/// Applies `VIEW_SMOKE_ZOOM` once the egui context exists.
fn zoom(mut contexts: EguiContexts, smoke: Res<Smoke>) {
    let Some(factor) = env("VIEW_SMOKE_ZOOM").and_then(|v| v.parse::<f32>().ok()) else { return };
    if smoke.frame == 2
        && let Ok(ctx) = contexts.ctx_mut()
    {
        ctx.set_zoom_factor(factor.clamp(0.4, 2.0));
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[allow(clippy::too_many_arguments)]
fn smoke(
    mut smoke: ResMut<Smoke>,
    mut commands: Commands,
    mut tracker: ResMut<ScreenshotTracker>,
    mut finished: MessageReader<ScreenshotFinished>,
    mut scenario: MessageWriter<ScenarioSelected>,
    mut actions: MessageWriter<TutorialAction>,
    mut shell: MessageWriter<ShellCommand>,
    mut theme: ResMut<ThemeSettings>,
    mut exit: MessageWriter<AppExit>,
) {
    smoke.frame += 1;
    let reenter = env("VIEW_SMOKE_REENTER").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0).min(3);
    if (5..5 + 6 * reenter).contains(&smoke.frame) {
        match (smoke.frame - 5) % 6 {
            0 => {
                shell.write(ShellCommand::OpenMode(AppMode::Dashboard));
            }
            3 => {
                shell.write(ShellCommand::OpenMode(AppMode::ThreeDimensionalLife));
            }
            _ => {}
        }
    }
    match smoke.frame {
        3 => {
            match env("VIEW_SMOKE_THEME").as_deref() {
                Some("light") => theme.kind = ThemeKind::Light,
                Some("contrast") => theme.kind = ThemeKind::HighContrast,
                _ => {}
            }
            if let Some(id) = env("VIEW_SMOKE_SCENARIO") {
                scenario.write(ScenarioSelected { mode: AppMode::ThreeDimensionalLife, id });
            }
        }
        25 => {
            for action in env("VIEW_SMOKE_ACTIONS").unwrap_or_default().split(',').filter(|a| !a.is_empty()) {
                actions.write(TutorialAction(action.trim().to_string()));
            }
        }
        30 => {
            if let Some(steps) = env("VIEW_SMOKE_STEPS").and_then(|v| v.parse().ok()) {
                shell.write(ShellCommand::Step(steps));
            }
            if env("VIEW_SMOKE_PLAY").is_some() {
                shell.write(ShellCommand::TogglePlay);
            }
        }
        35 => {
            let command = match env("VIEW_SMOKE_TAB").as_deref() {
                Some("parameters") => Some(ShellCommand::ShowRightTab(RightPanelTab::Parameters)),
                Some("rules") => Some(ShellCommand::ShowRightTab(RightPanelTab::Rules)),
                Some("metrics") => Some(ShellCommand::ShowRightTab(RightPanelTab::Metrics)),
                Some("properties") => Some(ShellCommand::ShowRightTab(RightPanelTab::Properties)),
                Some("tutorial") => Some(ShellCommand::OpenWindow(ShellWindow::Tutorial)),
                Some("help") => Some(ShellCommand::OpenWindow(ShellWindow::Help)),
                _ => None,
            };
            if let Some(command) = command {
                shell.write(command);
            }
            if env("VIEW_SMOKE_HIDE_PANELS").is_some() {
                shell.write(ShellCommand::TogglePanels);
            }
        }
        _ => {}
    }
    if smoke.frame + 40 == smoke.frames {
        for action in env("VIEW_SMOKE_LATE_ACTIONS").unwrap_or_default().split(',').filter(|a| !a.is_empty()) {
            actions.write(TutorialAction(action.trim().to_string()));
        }
    }
    if smoke.frame == smoke.frames {
        let live = view_voxel::live_sessions();
        if live != 1 {
            error!("expected one 3D Life session, found {live}");
            exit.write(AppExit::error());
            return;
        }
        info!("3D Life sessions alive: {live}");
        if let Some(parent) = smoke.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        tracker.capture(&mut commands, smoke.path.clone());
    }
    for done in finished.read() {
        if done.path == smoke.path {
            exit.write(if done.saved { AppExit::Success } else { AppExit::error() });
        }
    }
    if smoke.frame > smoke.frames + 600 {
        error!("smoke screenshot timed out");
        exit.write(AppExit::error());
    }
}
