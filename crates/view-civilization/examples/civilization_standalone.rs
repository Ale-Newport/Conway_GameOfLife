//! The Emergent Civilizations mode on its own, configured like the desktop
//! application (window, asset folder, shell, world input).
//!
//! ```text
//! cargo run -p view-civilization --example civilization_standalone
//! VIEW_SMOKE_SCREENSHOT=out.png VIEW_SMOKE_FRAMES=90 cargo run -p view-civilization --example civilization_standalone
//! ```
//!
//! Capture options (environment):
//!
//! | Variable | Effect |
//! |---|---|
//! | `VIEW_SMOKE_SCREENSHOT=<png>` | save the window after the frames below and exit |
//! | `VIEW_SMOKE_FRAMES=<n>` | frames to render before capturing (default 90) |
//! | `VIEW_SMOKE_PLAY=1` | run the simulation first (`VIEW_SMOKE_TICKS`, default 480 ticks) |
//! | `VIEW_SMOKE_SCENARIO=<id>` | built-in scenario to open |
//! | `VIEW_SMOKE_TAB=<name>` | `properties`, `parameters`, `rules`, `metrics`, `tutorial`, `concepts`, `help` |
//! | `VIEW_SMOKE_SELECT=1` | select a well-connected person (inspector, route, relationships); `building` selects a store, `hut` a hut, `farm` the most grown crop |
//! | `VIEW_SMOKE_FOLLOW=1` | follow the selected person |
//! | `VIEW_SMOKE_CAMERA=<preset>` | `strategic`, `isometric`, `aerial`, `free` |
//! | `VIEW_SMOKE_DISTANCE=<units>` | camera distance after framing |
//! | `VIEW_SMOKE_TOOL=<tool>` | `spawn`, `add`, `remove`, `disaster` |
//! | `VIEW_SMOKE_GRAPH=people` | trade network by person |
//! | `VIEW_SMOKE_EDIT=1` | apply the chosen tool (`VIEW_SMOKE_TOOL`, radius 6) once at the map centre, as a click would |
//! | `VIEW_SMOKE_THEME=<theme>` | `light` or `contrast` |
//! | `VIEW_SMOKE_LAYERS=<list>` | comma list of layers to hide: `territory,roads,icons,vegetation` |
//! | `VIEW_SMOKE_REENTER=1` | go to the dashboard and back first (exit / enter lifecycle) |
//! | `VIEW_SMOKE_SCROLL=<section>` | scroll the Properties tab to a section (`skills`, `relationships`, `trade network`, `groups`, …) |
//! | `VIEW_SMOKE_SWITCH=<id>` | after entering, pick another scenario as the top-bar picker does (`ScenarioSelected`, replaces the running simulation) |
//! | `VIEW_SMOKE_SPEED=<ticks/s or max>` | keep the simulation running at this speed while the frames render and print frame-time statistics |

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use bevy_panorbit_camera::PanOrbitCamera;
use interface::{
    AppMode, InterfacePlugin, ModeScenarios, RightPanelTab, ScenarioSelected, ScreenshotFinished, ScreenshotTracker,
    SessionStatus, ShellCommand, ShellWindow, ThemeKind, ThemeSettings,
};
use rendering::{OrbitPreset, WorldInputPlugin};
use simulation_core::Speed;
use view_civilization::scene::{CivCamera, MapCache};
use view_civilization::session::Selection;
use view_civilization::state::{CivRunner, CurrentFrame, GraphMode, Tool, ViewState};
use view_civilization::ui::properties::ScrollTo;

/// Asset folder like the desktop app: `$EMERGENCE_LAB_ASSETS`, then
/// `<executable dir>/assets`, then the workspace `assets/`.
fn asset_root() -> PathBuf {
    if let Some(path) = std::env::var_os("EMERGENCE_LAB_ASSETS").map(PathBuf::from).filter(|p| p.is_dir()) {
        return path;
    }
    let exe_assets = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("assets")));
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets");
    let path = exe_assets.filter(|p| p.is_dir()).unwrap_or(workspace);
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
                    title: "Emergence Lab — Emergent Civilizations".into(),
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
        InterfacePlugin { asset_root: assets, initial_mode: Some(AppMode::EmergentCivilizations) },
    ))
    .add_plugins(view_civilization::Plugin);
    if let Some(id) = env("VIEW_SMOKE_SCENARIO") {
        app.add_systems(PostStartup, move |mut scenarios: ResMut<ModeScenarios>| {
            if !scenarios.mark_selected(AppMode::EmergentCivilizations, &id) {
                warn!("unknown scenario `{id}`");
            }
        });
    }
    if let Some(path) = env("VIEW_SMOKE_SCREENSHOT") {
        let frames = env("VIEW_SMOKE_FRAMES").and_then(|v| v.parse().ok()).unwrap_or(90);
        let ticks = if env("VIEW_SMOKE_PLAY").is_some() {
            env("VIEW_SMOKE_TICKS").and_then(|v| v.parse().ok()).unwrap_or(480)
        } else {
            0
        };
        app.insert_resource(Smoke {
            path: std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(&path)),
            frames,
            ticks,
            frame: 0,
            ready_at: None,
            requested: false,
            configured: false,
            focus: None,
            frame_times: Vec::new(),
            stepped: false,
        })
        .add_systems(Update, smoke.after(interface::ShellSet::Poll));
    }
    app.run()
}

#[derive(Resource)]
struct Smoke {
    path: PathBuf,
    frames: u32,
    ticks: u64,
    frame: u32,
    ready_at: Option<u32>,
    requested: bool,
    configured: bool,
    focus: Option<Vec3>,
    /// Frame times (s) recorded while the simulation runs (`VIEW_SMOKE_SPEED`).
    frame_times: Vec<f32>,
    /// The requested ticks were sent.
    stepped: bool,
}

#[allow(clippy::too_many_arguments)]
fn smoke(
    mut smoke: ResMut<Smoke>,
    mut commands: Commands,
    mut tracker: ResMut<ScreenshotTracker>,
    mut finished: MessageReader<ScreenshotFinished>,
    mut exit: MessageWriter<AppExit>,
    mut shell: MessageWriter<ShellCommand>,
    mut picked: MessageWriter<ScenarioSelected>,
    mut theme: ResMut<ThemeSettings>,
    mut state: ResMut<ViewState>,
    status: Res<SessionStatus>,
    current: Res<CurrentFrame>,
    cache: Res<MapCache>,
    runner: Option<Res<CivRunner>>,
    mut scroll: ResMut<ScrollTo>,
    camera: Option<Single<(&mut PanOrbitCamera, &Projection), With<CivCamera>>>,
    time: Res<Time<Real>>,
) {
    smoke.frame += 1;
    let frame = smoke.frame;
    if env("VIEW_SMOKE_REENTER").is_some() {
        if frame == 2 {
            shell.write(ShellCommand::OpenMode(AppMode::Dashboard));
        }
        if frame == 6 {
            shell.write(ShellCommand::OpenMode(AppMode::EmergentCivilizations));
        }
        if frame < 10 {
            return;
        }
    }
    let switch = env("VIEW_SMOKE_SWITCH");
    if frame == 10
        && let Some(id) = &switch
    {
        picked.write(ScenarioSelected { mode: AppMode::EmergentCivilizations, id: id.clone() });
    }
    // The simulation to run is the switched-to scenario, once it is loaded.
    let loaded = switch.as_ref().is_none_or(|id| current.get().is_some_and(|f| &f.info.id == id));
    if frame >= 10 && loaded && !smoke.stepped {
        smoke.stepped = true;
        if smoke.ticks > 0
            && let Some(runner) = &runner
        {
            runner.0.step(smoke.ticks);
        }
    }
    if frame == 10 {
        match env("VIEW_SMOKE_THEME").as_deref() {
            Some("light") => theme.kind = ThemeKind::Light,
            Some("contrast") => theme.kind = ThemeKind::HighContrast,
            _ => {}
        }
        if let Some(preset) = env("VIEW_SMOKE_CAMERA") {
            state.camera = match preset.as_str() {
                "isometric" => OrbitPreset::Isometric,
                "aerial" => OrbitPreset::Aerial,
                "free" => OrbitPreset::FreeOrbit,
                _ => OrbitPreset::Strategic,
            };
        }
        state.tool = match env("VIEW_SMOKE_TOOL").as_deref() {
            Some("spawn") => Tool::Spawn,
            Some("add") => Tool::AddResource,
            Some("remove") => Tool::RemoveResource,
            Some("disaster") => Tool::Disaster,
            _ => Tool::Inspect,
        };
        if env("VIEW_SMOKE_GRAPH").as_deref() == Some("people") {
            state.graph = GraphMode::People;
        }
        if let Some(hidden) = env("VIEW_SMOKE_LAYERS") {
            for layer in hidden.split(',') {
                match layer.trim() {
                    "territory" => state.layers.territory = false,
                    "roads" => state.layers.roads = false,
                    "icons" => state.layers.icons = false,
                    "vegetation" => state.layers.vegetation = false,
                    _ => {}
                }
            }
        }
    }
    // Wait until the requested ticks ran and the map is built.
    let reached = smoke.stepped
        && loaded
        && status.get().is_some_and(|s| s.tick >= smoke.ticks)
        && current.get().is_some()
        && cache.field.is_some();
    if smoke.ready_at.is_none() && reached {
        smoke.ready_at = Some(frame);
    }
    let Some(ready) = smoke.ready_at else {
        if frame > 3000 {
            error!("standalone smoke: the simulation never reached tick {}", smoke.ticks);
            exit.write(AppExit::error());
        }
        return;
    };
    if !smoke.configured && frame >= ready + 2 {
        smoke.configured = true;
        let select = env("VIEW_SMOKE_SELECT");
        if matches!(select.as_deref(), Some("building") | Some("hut") | Some("farm"))
            && let Some(f) = current.get()
        {
            use civilization::BuildingKind;
            // A finished group store, a hut, or the farm plot with the tallest crop.
            let finished = || f.view.buildings.iter().filter(|b| b.progress >= 1.0);
            let pick = match select.as_deref() {
                Some("farm") => finished()
                    .filter(|b| b.crop_growth.is_some())
                    .max_by(|a, b| a.crop_growth.unwrap_or(0.0).total_cmp(&b.crop_growth.unwrap_or(0.0))),
                Some("hut") => finished().find(|b| b.kind == BuildingKind::Hut),
                _ => finished().max_by_key(|b| (b.kind == BuildingKind::Storage, b.kind == BuildingKind::FarmPlot)),
            }
            .map(|b| b.id);
            if let Some(id) = pick {
                state.selection = Selection::Building(id);
                smoke.focus = f
                    .view
                    .buildings
                    .iter()
                    .find(|b| b.id == id)
                    .map(|b| Vec3::new(b.tile.0 as f32 + 0.5, 0.2, b.tile.1 as f32 + 0.5));
            }
        } else if env("VIEW_SMOKE_SELECT").is_some()
            && let Some(f) = current.get()
        {
            // A well-connected adult: in a group if possible, carrying something.
            let pick = f
                .view
                .agents
                .iter()
                .filter(|a| a.adult)
                .max_by(|a, b| {
                    let score = |x: &civilization::view::AgentView| {
                        f32::from(u8::from(x.group.is_some())) * 10.0 + x.carrying.0.iter().sum::<f32>()
                    };
                    score(a).total_cmp(&score(b))
                })
                .map(|a| (a.id, a.position));
            if let Some((id, at)) = pick {
                state.selection = Selection::Agent(id);
                state.follow = env("VIEW_SMOKE_FOLLOW").is_some();
                smoke.focus = Some(Vec3::new(at.x, 0.2, at.y));
            }
        }
        if env("VIEW_SMOKE_EDIT").is_some()
            && let (Some(runner), Some(f)) = (&runner, current.get())
        {
            state.radius = 6.0;
            let size = UVec2::new(f.view.world_width, f.view.world_height);
            let centre = Vec3::new(size.x as f32 * 0.5, 0.0, size.y as f32 * 0.5);
            match view_civilization::interact::tool_command(&state, centre, size) {
                Some(command) => runner.0.send(simulation_core::Control::Edit(command)),
                None => warn!("VIEW_SMOKE_EDIT needs an intervention tool (VIEW_SMOKE_TOOL)"),
            }
        }
        if let Some(section) = env("VIEW_SMOKE_SCROLL") {
            scroll.0 = Some(section);
        }
        if let Some(speed) = env("VIEW_SMOKE_SPEED") {
            let speed = match speed.as_str() {
                "max" => Speed::Unlimited,
                other => Speed::TicksPerSecond(other.parse().unwrap_or(100.0)),
            };
            shell.write(ShellCommand::SetSpeed(speed));
            if let Some(runner) = &runner {
                runner.0.play();
            }
        }
        let tab = env("VIEW_SMOKE_TAB");
        match tab.as_deref() {
            Some("parameters") => {
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Parameters));
            }
            Some("rules") => {
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Rules));
            }
            Some("metrics") => {
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Metrics));
            }
            Some("tutorial") | Some("concepts") => {
                shell.write(ShellCommand::OpenWindow(ShellWindow::Tutorial));
            }
            Some("help") => {
                shell.write(ShellCommand::OpenWindow(ShellWindow::Help));
            }
            _ => {
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
            }
        }
    }
    if smoke.configured
        && frame == ready + 8
        && let Some(distance) = env("VIEW_SMOKE_DISTANCE").and_then(|v| v.parse::<f32>().ok())
        && let Some(camera) = camera
    {
        let (mut pan, projection) = camera.into_inner();
        let focus = smoke.focus.unwrap_or(pan.target_focus);
        rendering::focus_on(&mut pan, projection, focus, distance);
    }
    if smoke.configured && !smoke.requested && env("VIEW_SMOKE_SPEED").is_some() && frame > ready + 2 {
        let dt = time.delta_secs();
        smoke.frame_times.push(dt);
    }
    if !smoke.requested && frame >= ready + smoke.frames {
        if !smoke.frame_times.is_empty() {
            let mut times = smoke.frame_times.clone();
            times.sort_by(f32::total_cmp);
            let mean = times.iter().sum::<f32>() / times.len() as f32;
            let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
            let max = times[times.len() - 1];
            let tick = status.get().map_or(0, |s| s.tick);
            println!(
                "frame times over {} frames: mean {:.1} ms, p95 {:.1} ms, max {:.1} ms (tick {tick})",
                times.len(),
                mean * 1e3,
                p95 * 1e3,
                max * 1e3
            );
        }
        if let Some(parent) = smoke.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        tracker.capture(&mut commands, smoke.path.clone());
        smoke.requested = true;
    }
    for message in finished.read() {
        if message.path == smoke.path {
            exit.write(if message.saved { AppExit::Success } else { AppExit::error() });
        }
    }
    if frame > ready + smoke.frames + 600 {
        error!("standalone smoke: screenshot timed out");
        exit.write(AppExit::error());
    }
}
