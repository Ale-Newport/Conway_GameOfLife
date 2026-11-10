//! The Ecosystem Simulator mode on its own, configured like the desktop
//! application (window, asset folder, shell, world input).
//!
//! ```text
//! cargo run -p view-ecosystem --example ecosystem_standalone
//! VIEW_SMOKE_SCREENSHOT=out.png VIEW_SMOKE_FRAMES=90 cargo run -p view-ecosystem --example ecosystem_standalone
//! ```
//!
//! Capture options (environment):
//!
//! | Variable | Effect |
//! |---|---|
//! | `VIEW_SMOKE_SCREENSHOT=<png>` | save the window after the frames below and exit |
//! | `VIEW_SMOKE_FRAMES=<n>` | frames to render once the state is reached (default 90) |
//! | `VIEW_SMOKE_SCENARIO=<id>` | built-in scenario to open |
//! | `VIEW_SMOKE_PLAY=1` | run the simulation first (`VIEW_SMOKE_TICKS`, default 240 = one day) |
//! | `VIEW_SMOKE_HOUR=<h>` | then run on to that hour of the day (night, dawn…) |
//! | `VIEW_SMOKE_TAB=<name>` | `scenario`, `inspector`, `species`, `populations`, `foodweb`, `energy`, `climate`, `deaths`, `parameters`, `rules`, `metrics`, `tutorial`, `help` |
//! | `VIEW_SMOKE_CAMERA=<mode>` | `orbit`, `topdown`, `ground`, `cinematic`, `follow` |
//! | `VIEW_SMOKE_SELECT=<species or 1>` | select an animal (of that species) and frame it |
//! | `VIEW_SMOKE_TREE=1` | select the tallest tree and frame it |
//! | `VIEW_SMOKE_DISTANCE=<m>` | camera distance to the focus after framing |
//! | `VIEW_SMOKE_PITCH=<rad>` / `VIEW_SMOKE_YAW=<rad>` | camera angles after framing |
//! | `VIEW_SMOKE_FOCUS=<x,y>` | focus on a model cell |
//! | `VIEW_SMOKE_OVERLAY=<name>` | `temperature`, `moisture`, `water`, `nutrients`, `biomass`, `fire`, `territories`, `disease`, `pheromone`, `pollination` |
//! | `VIEW_SMOKE_TOOL=<tool>` | `spawn`, `cull`, `plant`, `ignite`, `flood`, `earthquake`, `landslide`, `volcano` |
//! | `VIEW_SMOKE_EDIT=1` | apply the tool once at the focus (as a click would), then run `VIEW_SMOKE_AFTER` ticks (default 30) |
//! | `VIEW_SMOKE_SPAWN=<species>` | species of the spawn tool |
//! | `VIEW_SMOKE_COUNT=<n>` / `VIEW_SMOKE_RADIUS=<cells>` | count and brush radius of the tools |
//! | `VIEW_SMOKE_HIDE=<layers>` | hide layers: comma-separated `animals`, `trees`, `cover`, `small`, `weather`, `highlights` |
//! | `VIEW_SMOKE_THEME=<theme>` | `light` or `contrast` |
//! | `VIEW_SMOKE_HIDE_PANELS=1` | hide the shell panels (documentary shots) |
//! | `VIEW_SMOKE_REENTER=1` | go to the dashboard and back first (exit / enter lifecycle) |
//! | `VIEW_SMOKE_SWITCH=<id>` | after entering, pick another scenario as the top-bar picker does |
//! | `VIEW_SMOKE_SPEED=<ticks/s or max>` | keep the simulation running while the frames render and print frame-time statistics |

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use bevy_panorbit_camera::PanOrbitCamera;
use ecosystem::OrganismRef;
use interface::{
    AppMode, InterfacePlugin, ModeScenarios, RightPanelTab, ScenarioSelected, ScreenshotFinished, ScreenshotTracker,
    SessionStatus, ShellCommand, ShellWindow, ThemeKind, ThemeSettings,
};
use rendering::WorldInputPlugin;
use simulation_core::{Control, Speed};
use view_ecosystem::palette::Overlay;
use view_ecosystem::scene::EcoCamera;
use view_ecosystem::scene::terrain::MapState;
use view_ecosystem::state::{CameraMode, EcoData, EcoRunner, EcoUi, PropertiesTab, Tool};

const MODE: AppMode = AppMode::EcosystemSimulator;

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
                    title: "Emergence Lab — Ecosystem Simulator".into(),
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
    .add_plugins(view_ecosystem::EcosystemViewPlugin);
    if let Some(id) = env("VIEW_SMOKE_SCENARIO") {
        app.add_systems(PostStartup, move |mut scenarios: ResMut<ModeScenarios>| {
            if !scenarios.mark_selected(MODE, &id) {
                warn!("unknown scenario `{id}`");
            }
        });
    }
    if let Some(path) = env("VIEW_SMOKE_SCREENSHOT") {
        let ticks = if env("VIEW_SMOKE_PLAY").is_some() {
            env("VIEW_SMOKE_TICKS").and_then(|v| v.parse().ok()).unwrap_or(240)
        } else {
            0
        };
        app.insert_resource(Smoke {
            path: std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(&path)),
            frames: env("VIEW_SMOKE_FRAMES").and_then(|v| v.parse().ok()).unwrap_or(90),
            ticks,
            hour: env("VIEW_SMOKE_HOUR").and_then(|v| v.parse().ok()),
            ..Smoke::default()
        })
        .add_systems(Update, smoke.after(interface::ShellSet::Poll));
    }
    app.run()
}

#[derive(Resource, Default)]
struct Smoke {
    path: PathBuf,
    frames: u32,
    ticks: u64,
    hour: Option<u32>,
    frame: u32,
    stage: Stage,
    stage_frame: u32,
    target_tick: u64,
    focus: Option<Vec3>,
    frame_times: Vec<f32>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    #[default]
    Start,
    Running,
    Configure,
    Edited,
    Settle,
    Capture,
    Done,
}

fn parse_overlay(name: &str) -> Overlay {
    match name {
        "temperature" => Overlay::Temperature,
        "moisture" => Overlay::Moisture,
        "water" => Overlay::SurfaceWater,
        "nutrients" => Overlay::Nutrients,
        "biomass" => Overlay::PlantBiomass,
        "fire" => Overlay::FireRisk,
        "territories" => Overlay::Territories,
        "disease" => Overlay::Disease,
        "pheromone" => Overlay::Pheromone,
        "pollination" => Overlay::Pollination,
        _ => Overlay::None,
    }
}

fn parse_tool(name: &str) -> Tool {
    match name {
        "spawn" => Tool::Spawn,
        "cull" => Tool::Cull,
        "plant" => Tool::PlantTree,
        "ignite" => Tool::Ignite,
        "flood" => Tool::Flood,
        "earthquake" => Tool::Earthquake,
        "landslide" => Tool::Landslide,
        "volcano" => Tool::Volcano,
        _ => Tool::Inspect,
    }
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
    mut ui: ResMut<EcoUi>,
    mut jobs: ResMut<view_ecosystem::state::PendingJobs>,
    status: Res<SessionStatus>,
    data: Res<EcoData>,
    map: Res<MapState>,
    runner: Option<Res<EcoRunner>>,
    camera: Option<Single<(&mut PanOrbitCamera, &Projection), With<EcoCamera>>>,
    time: Res<Time<Real>>,
) {
    smoke.frame += 1;
    smoke.stage_frame += 1;
    let frame = smoke.frame;
    if env("VIEW_SMOKE_REENTER").is_some() && frame < 12 {
        if frame == 2 {
            shell.write(ShellCommand::OpenMode(AppMode::Dashboard));
        }
        if frame == 6 {
            shell.write(ShellCommand::OpenMode(MODE));
        }
        // The stages start counting once the mode is open again.
        smoke.stage_frame = 0;
        return;
    }
    if frame > 100_000 {
        error!("standalone smoke: stuck in stage {:?}", smoke.stage);
        exit.write(AppExit::error());
        return;
    }
    let switch = env("VIEW_SMOKE_SWITCH");
    let tick = status.get().map_or(0, |s| s.tick);
    let next = |smoke: &mut Smoke, stage: Stage| {
        smoke.stage = stage;
        smoke.stage_frame = 0;
    };
    match smoke.stage {
        Stage::Start => {
            // Wait for the landscape of the scenario that will be shown.
            let wanted = switch.clone().or_else(|| env("VIEW_SMOKE_SCENARIO"));
            let loaded = data.get().is_some_and(|f| wanted.as_ref().is_none_or(|id| &f.info.scenario.id == id));
            if smoke.stage_frame == 10
                && let Some(id) = &switch
            {
                picked.write(ScenarioSelected { mode: MODE, id: id.clone() });
            }
            if smoke.stage_frame > 12 && loaded && map.geometry.is_some() {
                if let Some(runner) = &runner {
                    let mut ticks = smoke.ticks;
                    if let (Some(hour), Some(f)) = (smoke.hour, data.get()) {
                        let tpd = u64::from(f.info.ticks_per_day);
                        let at = tick + ticks;
                        let target = (at / tpd) * tpd + u64::from(hour) * tpd / 24;
                        ticks = if target >= at { target - tick } else { target + tpd - tick };
                    }
                    if ticks > 0 {
                        runner.0.step(ticks);
                    }
                    smoke.target_tick = tick + ticks;
                }
                next(&mut smoke, Stage::Running);
            }
        }
        Stage::Running => {
            let fresh = data.get().is_some_and(|f| f.view.tick >= smoke.target_tick);
            // The camera is configured once the map has been framed.
            if tick >= smoke.target_tick && fresh && smoke.stage_frame > 3 && !ui.frame_map {
                next(&mut smoke, Stage::Configure);
            }
        }
        Stage::Configure => {
            let Some(f) = data.get() else { return };
            match env("VIEW_SMOKE_THEME").as_deref() {
                Some("light") => theme.kind = ThemeKind::Light,
                Some("contrast") => theme.kind = ThemeKind::HighContrast,
                _ => {}
            }
            if let Some(o) = env("VIEW_SMOKE_OVERLAY") {
                ui.overlay = parse_overlay(&o);
            }
            for layer in env("VIEW_SMOKE_HIDE").unwrap_or_default().split(',') {
                let l = &mut ui.layers;
                match layer.trim() {
                    "animals" => l.animals = false,
                    "trees" => l.trees = false,
                    "cover" => l.ground_cover = false,
                    "small" => l.small_life = false,
                    "weather" => l.weather = false,
                    "highlights" => l.highlights = false,
                    _ => {}
                }
            }
            if let Some(spawn) = env("VIEW_SMOKE_SPAWN") {
                ui.settings.spawn_species = spawn;
            }
            if let Some(count) = env("VIEW_SMOKE_COUNT").and_then(|v| v.parse().ok()) {
                ui.settings.spawn_count = count;
            }
            if let Some(radius) = env("VIEW_SMOKE_RADIUS").and_then(|v| v.parse().ok()) {
                ui.settings.radius = radius;
            }
            if let Some(focus) = env("VIEW_SMOKE_FOCUS") {
                let mut it = focus.split(',').filter_map(|v| v.trim().parse::<f32>().ok());
                if let (Some(x), Some(y)) = (it.next(), it.next()) {
                    smoke.focus = map.geometry.as_ref().map(|g| g.ground_point(Vec2::new(x, y)));
                }
            }
            if let Some(which) = env("VIEW_SMOKE_SELECT")
                && let Some(runner) = &runner
            {
                let species = f.info.catalog.animal_index(&which).ok();
                // A visible adult of the species (or any), near the middle of its kind.
                let pick = f
                    .view
                    .animals
                    .iter()
                    .filter(|a| !a.hidden && !a.juvenile && species.is_none_or(|s| s == a.species))
                    .max_by(|a, b| (a.size_m * a.health).total_cmp(&(b.size_m * b.health)));
                match pick {
                    Some(a) => {
                        view_ecosystem::interact::select(runner, &mut ui, Some(OrganismRef::Animal(a.id)));
                        // Swimmers and fliers: the focus is the animal, not the ground under it.
                        smoke.focus = map.geometry.as_ref().map(|g| {
                            let p = g.ground_point(a.position);
                            let surface = g.surface(p.x, p.z);
                            Vec3::new(p.x, (surface + a.altitude_m).max(p.y), p.z)
                        });
                    }
                    None => warn!("no animal to select"),
                }
            }
            if env("VIEW_SMOKE_TREE").is_some()
                && let (Some(runner), Some(tf)) = (&runner, data.trees_frame.as_deref())
                && let Some(t) = tf.view.trees.iter().max_by(|a, b| a.height_m.total_cmp(&b.height_m))
            {
                view_ecosystem::interact::select(runner, &mut ui, Some(OrganismRef::Tree(t.id)));
                smoke.focus = map.geometry.as_ref().map(|g| g.ground_point(t.position) + Vec3::Y * t.height_m * 0.4);
            }
            if let Some(mode) = env("VIEW_SMOKE_CAMERA") {
                ui.camera = match mode.as_str() {
                    "topdown" => CameraMode::TopDown,
                    "ground" => CameraMode::Ground,
                    "cinematic" => CameraMode::Cinematic,
                    "follow" => CameraMode::Follow,
                    _ => CameraMode::Orbit,
                };
            }
            let tool = env("VIEW_SMOKE_TOOL").map_or(Tool::Inspect, |t| parse_tool(&t));
            ui.tool = tool;
            if env("VIEW_SMOKE_EDIT").is_some()
                && let (Some(runner), Some(g)) = (&runner, map.geometry.as_ref())
            {
                let at = smoke.focus.unwrap_or_else(|| g.centre());
                let c = g.plane_to_cell(at.x, at.z);
                let c = simulation_core::glam::Vec2::new(c.x, c.y);
                if tool == Tool::Cull {
                    let ids: Vec<u64> = f
                        .view
                        .animals
                        .iter()
                        .filter(|a| a.position.distance(c) <= ui.settings.radius)
                        .map(|a| a.id)
                        .collect();
                    view_ecosystem::interact::cull(runner, &mut jobs, ids, "animals".into(), f.info.netlogo.is_none());
                } else if let Some(command) = view_ecosystem::interact::tool_command(tool, &ui.settings, c, f.info.size)
                {
                    runner.0.send(Control::Edit(command));
                }
                let after: u64 = env("VIEW_SMOKE_AFTER").and_then(|v| v.parse().ok()).unwrap_or(30);
                if after > 0 {
                    runner.0.step(after);
                }
                smoke.target_tick = tick + after;
            }
            let tab = env("VIEW_SMOKE_TAB");
            let props = |t: PropertiesTab, ui: &mut EcoUi, shell: &mut MessageWriter<ShellCommand>| {
                ui.tab = t;
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
            };
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
                Some("tutorial") => {
                    shell.write(ShellCommand::OpenWindow(ShellWindow::Tutorial));
                }
                Some("help") => {
                    shell.write(ShellCommand::OpenWindow(ShellWindow::Help));
                }
                Some("inspector") => props(PropertiesTab::Inspector, &mut ui, &mut shell),
                Some("species") => props(PropertiesTab::Species, &mut ui, &mut shell),
                Some("populations") => props(PropertiesTab::Populations, &mut ui, &mut shell),
                Some("foodweb") => props(PropertiesTab::FoodWeb, &mut ui, &mut shell),
                Some("energy") => props(PropertiesTab::Energy, &mut ui, &mut shell),
                Some("climate") => props(PropertiesTab::Climate, &mut ui, &mut shell),
                Some("deaths") => props(PropertiesTab::Deaths, &mut ui, &mut shell),
                Some("scenario") => props(PropertiesTab::Scenario, &mut ui, &mut shell),
                _ => {}
            }
            if env("VIEW_SMOKE_HIDE_PANELS").is_some() {
                shell.write(ShellCommand::TogglePanels);
            }
            if let Some(speed) = env("VIEW_SMOKE_SPEED")
                && let Some(runner) = &runner
            {
                let speed = match speed.as_str() {
                    "max" => Speed::Unlimited,
                    other => Speed::TicksPerSecond(other.parse().unwrap_or(30.0)),
                };
                shell.write(ShellCommand::SetSpeed(speed));
                runner.0.play();
            }
            next(&mut smoke, Stage::Edited);
        }
        Stage::Edited => {
            let ready = env("VIEW_SMOKE_EDIT").is_none()
                || (tick >= smoke.target_tick && data.get().is_some_and(|f| f.view.tick >= smoke.target_tick));
            if ready && !ui.frame_map && smoke.stage_frame > 4 {
                if let Some(camera) = camera {
                    let (mut pan, projection) = camera.into_inner();
                    let focus = smoke.focus.unwrap_or(pan.target_focus);
                    let distance = env("VIEW_SMOKE_DISTANCE")
                        .and_then(|v| v.parse::<f32>().ok())
                        .unwrap_or(if smoke.focus.is_some() { 30.0 } else { pan.target_radius });
                    rendering::focus_on(&mut pan, projection, focus, distance);
                    if let Some(p) = env("VIEW_SMOKE_PITCH").and_then(|v| v.parse::<f32>().ok()) {
                        pan.target_pitch = p;
                    }
                    if let Some(y) = env("VIEW_SMOKE_YAW").and_then(|v| v.parse::<f32>().ok()) {
                        pan.target_yaw = y;
                    }
                }
                next(&mut smoke, Stage::Settle);
            }
        }
        Stage::Settle => {
            if env("VIEW_SMOKE_SPEED").is_some() && smoke.stage_frame > 10 {
                let dt = time.delta_secs();
                smoke.frame_times.push(dt);
            }
            if smoke.stage_frame >= smoke.frames {
                if !smoke.frame_times.is_empty() {
                    let mut times = smoke.frame_times.clone();
                    times.sort_by(f32::total_cmp);
                    let mean = times.iter().sum::<f32>() / times.len() as f32;
                    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
                    let animals = data.get().map_or(0, |f| f.view.animals.len());
                    println!(
                        "frame times over {} frames: mean {:.1} ms, p95 {:.1} ms, max {:.1} ms (tick {tick}, {animals} animals)",
                        times.len(),
                        mean * 1e3,
                        p95 * 1e3,
                        times[times.len() - 1] * 1e3
                    );
                }
                next(&mut smoke, Stage::Capture);
            }
        }
        Stage::Capture => {
            if let Some(parent) = smoke.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            tracker.capture(&mut commands, smoke.path.clone());
            next(&mut smoke, Stage::Done);
        }
        Stage::Done => {
            if smoke.stage_frame > 600 {
                error!("standalone smoke: screenshot timed out");
                exit.write(AppExit::error());
            }
        }
    }
    for message in finished.read() {
        if message.path == smoke.path {
            exit.write(if message.saved { AppExit::Success } else { AppExit::error() });
        }
    }
}
