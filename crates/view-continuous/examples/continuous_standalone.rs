//! The Continuous Artificial Life mode on its own: the application shell with
//! only this mode registered, opened directly.
//!
//! ```text
//! cargo run -p view-continuous --example continuous_standalone
//! ```
//!
//! Smoke capture (renders, saves the window as a PNG and exits):
//!
//! | Variable | Meaning |
//! |---|---|
//! | `VIEW_SMOKE_SCREENSHOT=<png>` | enables the capture |
//! | `VIEW_SMOKE_FRAMES=<n>` | frame of the capture (default 90) |
//! | `VIEW_SMOKE_SCENARIO=<id>` | scenario to open (e.g. `lenia.tessellatium_gyrans`) |
//! | `VIEW_SMOKE_STEPS=<n>` | run `n` ticks at once after startup |
//! | `VIEW_SMOKE_PLAY=1` | play at 120 ticks/s from frame 5 |
//! | `VIEW_SMOKE_TAB=<properties\|parameters\|rules\|metrics>` | right-panel tab |
//! | `VIEW_SMOKE_LAYER=<state\|potential\|growth>` | displayed layer |
//! | `VIEW_SMOKE_TOOL=<inspect\|brush\|erase\|stamp>` | active tool |
//! | `VIEW_SMOKE_CURSOR=<fx>,<fy>` | pretend the pointer is at this fraction of the world viewport |
//! | `VIEW_SMOKE_COLORMAP=<name>` | colormap (`viridis`, `inferno`, `magma`, `plasma`, `turbo`, `lenia`, `gray`) |
//! | `VIEW_SMOKE_TRAIL=1` | show the centroid trail |
//! | `VIEW_SMOKE_GRID=1` | show the cell grid lines |
//! | `VIEW_SMOKE_FIT=<x0>,<y0>,<x1>,<y1>` | frame this cell rectangle (zoom in) at frame 10 |
//! | `VIEW_SMOKE_SWITCH=<id>` | load another scenario at frame 30 (runtime switch) |
//! | `VIEW_SMOKE_ACTION=<action>` | run a tutorial action at frame 40 (e.g. `run-trail`, `layer:growth`) |
//! | `VIEW_SMOKE_SMOOTH=0` | nearest-neighbour display instead of bilinear smoothing |
//! | `VIEW_SMOKE_WINDOW=<tutorial\|help>` | open a shell window |
//! | `VIEW_SMOKE_THEME=<light\|contrast>` | theme |
//! | `VIEW_SMOKE_ROTATION=<deg>`, `VIEW_SMOKE_SCALE=<x>` | stamp rotation and scale |
//! | `VIEW_SMOKE_DRAG=<fx0>,<fy0>,<fx1>,<fy1>` | drag with the left button from frame 20 to 50 (brush strokes, stamping) |
//! | `VIEW_SMOKE_PARAM=<key>=<value>[;...]` | set float parameters at frame 12 (e.g. `k0.mu=0.2;k0.sigma=0.03`) |
//! | `VIEW_SMOKE_REENTER=1` | go to the dashboard at frames 15 and 45 and back at 25 and 55 |

use std::path::{Path, PathBuf};

use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use continuous::ViewLayer;
use interface::{
    AppMode, InterfacePlugin, ModeScenarios, RightPanelTab, ScreenshotFinished, ScreenshotTracker, ShellCommand,
    ShellWindow, ThemeKind, ThemeSettings, TutorialAction,
};
use rendering::palette::Colormap;
use rendering::{GridCameraController, WorldInput, WorldInputPlugin};
use simulation_core::Speed;
use view_continuous::{
    ContinuousViewPlugin, ContinuousViewSystems, LoadedWorld, MODE, Tool, ToolSettings, ViewSettings,
};

/// Frames to wait for the screenshot before giving up.
const TIMEOUT_FRAMES: u32 = 600;

/// Asset folder like the desktop app: `$EMERGENCE_LAB_ASSETS`, else the
/// workspace `assets/` (made absolute for `AssetPlugin::file_path`).
fn asset_root() -> PathBuf {
    let path = std::env::var_os("EMERGENCE_LAB_ASSETS")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets"));
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn main() -> AppExit {
    let assets = asset_root();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Emergence Lab — Continuous Artificial Life".into(),
                    resolution: WindowResolution::new(1600, 1000),
                    present_mode: PresentMode::AutoVsync,
                    window_theme: Some(WindowTheme::Dark),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin { file_path: assets.to_string_lossy().into_owned(), ..default() }),
    )
    .add_plugins((WorldInputPlugin, InterfacePlugin { asset_root: assets, initial_mode: Some(MODE) }))
    .add_plugins(ContinuousViewPlugin);
    if let Some(path) = env("VIEW_SMOKE_SCREENSHOT") {
        let path = std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(&path));
        let frames = env("VIEW_SMOKE_FRAMES").and_then(|v| v.parse().ok()).unwrap_or(90);
        app.insert_resource(Smoke { path, frames, frame: 0, requested: false })
            .add_systems(PostStartup, choose_scenario)
            .add_systems(Update, (smoke_setup, smoke_zoom, smoke_runtime, smoke_capture).chain())
            .add_systems(Update, fake_cursor.before(ContinuousViewSystems::Input));
    }
    app.run()
}

#[derive(Resource)]
struct Smoke {
    path: PathBuf,
    frames: u32,
    frame: u32,
    requested: bool,
}

/// Selects the scenario before the mode is entered (the mode opens the
/// selected scenario).
fn choose_scenario(mut scenarios: ResMut<ModeScenarios>) {
    if let Some(id) = env("VIEW_SMOKE_SCENARIO")
        && !scenarios.mark_selected(MODE, &id)
    {
        error!("unknown scenario `{id}`");
    }
}

fn smoke_setup(
    smoke: Res<Smoke>,
    mut shell: MessageWriter<ShellCommand>,
    mut theme: ResMut<ThemeSettings>,
    mut settings: ResMut<ViewSettings>,
    mut tools: ResMut<ToolSettings>,
    loaded: Option<Res<LoadedWorld>>,
) {
    match smoke.frame {
        2 => {
            match env("VIEW_SMOKE_TAB").as_deref() {
                Some("parameters") => {
                    shell.write(ShellCommand::ShowRightTab(RightPanelTab::Parameters));
                }
                Some("rules") => {
                    shell.write(ShellCommand::ShowRightTab(RightPanelTab::Rules));
                }
                Some("metrics") => {
                    shell.write(ShellCommand::ShowRightTab(RightPanelTab::Metrics));
                }
                _ => {}
            }
            match env("VIEW_SMOKE_WINDOW").as_deref() {
                Some("tutorial") => {
                    shell.write(ShellCommand::OpenWindow(ShellWindow::Tutorial));
                }
                Some("help") => {
                    shell.write(ShellCommand::OpenWindow(ShellWindow::Help));
                }
                _ => {}
            }
            match env("VIEW_SMOKE_THEME").as_deref() {
                Some("light") => theme.kind = ThemeKind::Light,
                Some("contrast") => theme.kind = ThemeKind::HighContrast,
                _ => {}
            }
            if let Some(map) = env("VIEW_SMOKE_COLORMAP") {
                settings.colormap = match map.as_str() {
                    "viridis" => Colormap::Viridis,
                    "inferno" => Colormap::Inferno,
                    "magma" => Colormap::Magma,
                    "plasma" => Colormap::Plasma,
                    "turbo" => Colormap::TurboLike,
                    "gray" => Colormap::Grayscale,
                    _ => Colormap::LeniaBlue,
                };
            }
            if env("VIEW_SMOKE_SMOOTH").as_deref() == Some("0") {
                settings.smooth = false;
            }
            if env("VIEW_SMOKE_TRAIL").is_some() {
                settings.show_trail = true;
            }
            if env("VIEW_SMOKE_GRID").is_some() {
                settings.grid_lines = true;
            }
            if let Some(tool) = env("VIEW_SMOKE_TOOL") {
                tools.tool = match tool.as_str() {
                    "brush" => Tool::Brush,
                    "erase" => Tool::Erase,
                    "stamp" => Tool::Stamp,
                    _ => Tool::Inspect,
                };
            }
            if let Some(r) = env("VIEW_SMOKE_ROTATION").and_then(|v| v.parse().ok()) {
                tools.rotation_deg = r;
            }
            if let Some(s) = env("VIEW_SMOKE_SCALE").and_then(|v| v.parse().ok()) {
                tools.scale = s;
            }
        }
        3 => {
            if let Some(n) = env("VIEW_SMOKE_STEPS").and_then(|v| v.parse::<u64>().ok()) {
                shell.write(ShellCommand::Step(n));
            }
            if let Some(layer) = env("VIEW_SMOKE_LAYER") {
                let layer = match layer.as_str() {
                    "potential" => ViewLayer::Potential,
                    "growth" => ViewLayer::Growth,
                    _ => ViewLayer::State,
                };
                settings.set_layer(layer, loaded.map_or(1, |l| l.channels()));
            }
        }
        5 if env("VIEW_SMOKE_PLAY").is_some() => {
            shell.write(ShellCommand::SetSpeed(Speed::TicksPerSecond(120.0)));
            shell.write(ShellCommand::TogglePlay);
        }
        _ => {}
    }
}

/// Runtime scenario switch, tutorial action and mode re-entry.
fn smoke_runtime(
    smoke: Res<Smoke>,
    mut requests: ResMut<view_continuous::LoadRequests>,
    mut actions: MessageWriter<TutorialAction>,
    mut shell: MessageWriter<ShellCommand>,
    session: Option<Res<interface::ActiveSession>>,
) {
    // Live parameter edits through the session, like the parameter editor.
    if smoke.frame == 12
        && let (Some(spec), Some(session)) = (env("VIEW_SMOKE_PARAM"), session)
    {
        for assignment in spec.split(';') {
            match assignment.split_once('=').and_then(|(k, v)| Some((k.trim(), v.trim().parse::<f64>().ok()?))) {
                Some((key, value)) => session.session().set_param(key, simulation_core::ParamValue::Float(value)),
                None => error!("bad VIEW_SMOKE_PARAM entry `{assignment}`"),
            }
        }
    }
    // Leave to the dashboard and come back (twice) to exercise the
    // OnExit / OnEnter lifecycle with a running simulation.
    if env("VIEW_SMOKE_REENTER").is_some() {
        match smoke.frame {
            15 | 45 => {
                shell.write(ShellCommand::OpenMode(AppMode::Dashboard));
            }
            25 | 55 => {
                shell.write(ShellCommand::OpenMode(MODE));
            }
            _ => {}
        }
    }
    if smoke.frame == 30
        && let Some(id) = env("VIEW_SMOKE_SWITCH")
    {
        requests.0.push(id);
    }
    if smoke.frame == 40
        && let Some(action) = env("VIEW_SMOKE_ACTION")
    {
        actions.write(TutorialAction(action));
    }
}

/// Frames the cell rectangle of `VIEW_SMOKE_FIT` (after the mode's own
/// framing of the world).
fn smoke_zoom(
    smoke: Res<Smoke>,
    mut cameras: Query<&mut GridCameraController>,
    mut fit: Option<ResMut<view_continuous::CameraFit>>,
) {
    if smoke.frame != 10 {
        return;
    }
    let Some(spec) = env("VIEW_SMOKE_FIT") else { return };
    let v: Vec<i64> = spec.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    let [x0, y0, x1, y1] = v[..] else { return };
    if let Some(fit) = fit.as_mut() {
        fit.auto = false;
    }
    for mut camera in &mut cameras {
        camera.fit_to_rect(I64Vec2::new(x0, y0), I64Vec2::new(x1, y1), 0.0);
    }
}

/// Moves the (virtual) pointer over the world for previews of the tools
/// and, with `VIEW_SMOKE_DRAG`, drags it with the left button held.
fn fake_cursor(smoke: Res<Smoke>, mut input: ResMut<WorldInput>, mut buttons: ResMut<ButtonInput<MouseButton>>) {
    let Some(viewport) = input.viewport else { return };
    let at = |fx: f32, fy: f32| viewport.min + viewport.size() * Vec2::new(fx, fy);
    if let Some(spec) = env("VIEW_SMOKE_DRAG") {
        let v: Vec<f32> = spec.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let [x0, y0, x1, y1] = v[..] else { return };
        let (start, end) = (DRAG_FRAMES.0, DRAG_FRAMES.1);
        if (start..=end).contains(&smoke.frame) {
            let t = (smoke.frame - start) as f32 / (end - start) as f32;
            input.cursor = Some(at(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t));
            input.pointer = true;
            if smoke.frame == start {
                buttons.press(MouseButton::Left);
            }
        } else if smoke.frame == end + 1 {
            buttons.release(MouseButton::Left);
        }
        return;
    }
    let Some(spec) = env("VIEW_SMOKE_CURSOR") else { return };
    let Some((fx, fy)) = spec.split_once(',').and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)))
    else {
        return;
    };
    input.cursor = Some(at(fx, fy));
    input.pointer = true;
}

/// Frames over which `VIEW_SMOKE_DRAG` drags the pointer.
const DRAG_FRAMES: (u32, u32) = (20, 50);

fn smoke_capture(
    mut smoke: ResMut<Smoke>,
    mut commands: Commands,
    mut tracker: ResMut<ScreenshotTracker>,
    mut finished: MessageReader<ScreenshotFinished>,
    mut exit: MessageWriter<AppExit>,
) {
    smoke.frame += 1;
    if !smoke.requested && smoke.frame >= smoke.frames {
        if let Some(parent) = smoke.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            error!("cannot create `{}`: {e}", parent.display());
            exit.write(AppExit::error());
            return;
        }
        tracker.capture(&mut commands, smoke.path.clone());
        smoke.requested = true;
    }
    for message in finished.read() {
        if message.path == smoke.path {
            exit.write(if message.saved { AppExit::Success } else { AppExit::error() });
        }
    }
    if smoke.frame > smoke.frames + TIMEOUT_FRAMES {
        error!("screenshot timed out");
        exit.write(AppExit::error());
    }
}
