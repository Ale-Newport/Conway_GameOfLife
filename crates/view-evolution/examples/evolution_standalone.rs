//! The Evolution Laboratory on its own: the application shell with only
//! this mode registered, opened directly.
//!
//! ```text
//! cargo run -p view-evolution --example evolution_standalone
//! ```
//!
//! Smoke capture (all optional except the first):
//!
//! | Variable | Effect |
//! |---|---|
//! | `VIEW_SMOKE_SCREENSHOT=<png>` | capture the window and exit |
//! | `VIEW_SMOKE_FRAMES=<n>` | frames to render after setup before capturing (default 90) |
//! | `VIEW_SMOKE_SCENARIO=<id>` | load a built-in scenario first |
//! | `VIEW_SMOKE_PLAY=1` or `=<ticks>` | run 600 (or `<ticks>`) ticks before capturing |
//! | `VIEW_SMOKE_TAB=<name>` | `inspector`, `genes`, `genealogy`, `regions`, `parameters`, `rules`, `metrics`, `tutorial`, `help` |
//! | `VIEW_SMOKE_SELECT=1` | select a prey (`predator` for a predator) |
//! | `VIEW_SMOKE_FOLLOW=<distance>` | follow the selection with the camera at that distance |
//! | `VIEW_SMOKE_OVERLAY=<name>` | ground tint: `temperature`, `food`, `regions`, `plain` |
//! | `VIEW_SMOKE_GENE=<name>` | gene shown in the charts (e.g. `marker_a`) |
//! | `VIEW_SMOKE_TOOL=<name>` | `food`, `cull`, `spawn`, `predators` |
//! | `VIEW_SMOKE_THEME=<name>` | `light` or `contrast` |
//! | `VIEW_SMOKE_SPEED=<ticks/s>` | play at this speed while capturing (shows motion) |
//! | `VIEW_SMOKE_REENTER=1` | go to the dashboard and back before capturing (lifecycle check) |
//! | `VIEW_SMOKE_COMPACT=1` | fold the all-genes table and the legend (shows the lower charts) |
//! | `VIEW_SMOKE_CLICK=1` | left-click at the pretended cursor (exercises picking and the brush tools) |
//! | `VIEW_SMOKE_ACTION=<id>` | send a tutorial action before playing (e.g. `control:no_predators`, `overlay:food`) |
//! | `VIEW_SMOKE_CURSOR=<x>,<y>` | pretend the pointer rests there (window logical px): hover card, brush; `selected` = on the selected organism |

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use bevy_panorbit_camera::PanOrbitCamera;
use interface::{
    ActiveSession, AppMode, InterfacePlugin, RightPanelTab, ScreenshotFinished, ScreenshotTracker, SessionStatus,
    ShellCommand, ShellWindow, ThemeKind, ThemeSettings, TutorialAction,
};
use rendering::WorldInput;
use rendering::WorldInputPlugin;
use simulation_core::Speed;
use view_evolution::organisms::OrganismIndex;
use view_evolution::scene::EvoCamera;
use view_evolution::ui_state::GroundOverlay;
use view_evolution::{EvoData, EvoUi, EvolutionViewPlugin};

fn asset_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("EMERGENCE_LAB_ASSETS").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets");
    std::fs::canonicalize(&workspace).unwrap_or(workspace)
}

#[derive(Resource, Debug)]
struct Smoke {
    path: Option<PathBuf>,
    frames: u32,
    scenario: Option<String>,
    play: Option<u64>,
    tab: Option<String>,
    select: Option<String>,
    follow: Option<f32>,
    overlay: Option<String>,
    gene: Option<String>,
    tool: Option<String>,
    speed: Option<f64>,
    cursor: Option<Vec2>,
    cursor_on_selection: bool,
    frame: u32,
    stage: Stage,
    target_tick: u64,
    settle: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Load,
    Run,
    Wait,
    Arrange,
    Away,
    Back,
    Settle,
    Capture,
    Done,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn main() -> AppExit {
    let root = asset_root();
    let smoke = Smoke {
        path: env("VIEW_SMOKE_SCREENSHOT").map(|p| std::path::absolute(p).unwrap_or_default()),
        frames: env("VIEW_SMOKE_FRAMES").and_then(|f| f.parse().ok()).unwrap_or(90),
        scenario: env("VIEW_SMOKE_SCENARIO"),
        play: env("VIEW_SMOKE_PLAY").map(|p| match p.parse::<u64>() {
            Ok(1) | Err(_) => 600,
            Ok(n) => n,
        }),
        tab: env("VIEW_SMOKE_TAB"),
        select: env("VIEW_SMOKE_SELECT"),
        follow: env("VIEW_SMOKE_FOLLOW").and_then(|d| d.parse().ok()),
        overlay: env("VIEW_SMOKE_OVERLAY"),
        gene: env("VIEW_SMOKE_GENE"),
        tool: env("VIEW_SMOKE_TOOL"),
        speed: env("VIEW_SMOKE_SPEED").and_then(|s| s.parse().ok()),
        cursor: env("VIEW_SMOKE_CURSOR").and_then(|c| {
            let (x, y) = c.split_once(',')?;
            Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
        }),
        cursor_on_selection: env("VIEW_SMOKE_CURSOR").as_deref() == Some("selected"),
        frame: 0,
        stage: Stage::Load,
        target_tick: 0,
        settle: 0,
    };
    let theme = match env("VIEW_SMOKE_THEME").as_deref() {
        Some("light") => ThemeKind::Light,
        Some("contrast") => ThemeKind::HighContrast,
        _ => ThemeKind::Dark,
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Emergence Lab — Evolution Laboratory".into(),
                    resolution: WindowResolution::new(1600, 1000),
                    present_mode: PresentMode::AutoVsync,
                    window_theme: Some(WindowTheme::Dark),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin { file_path: root.to_string_lossy().into_owned(), ..default() }),
    )
    .add_plugins((WorldInputPlugin, InterfacePlugin { asset_root: root, initial_mode: Some(AppMode::EvolutionLab) }))
    .add_plugins(EvolutionViewPlugin)
    .insert_resource(ThemeSettings { kind: theme });
    if smoke.path.is_some() {
        app.insert_resource(smoke).add_systems(
            Update,
            drive_smoke.after(interface::ShellSet::Poll).before(view_evolution::EvolutionViewSystems::Data),
        );
    }
    app.run()
}

#[allow(clippy::too_many_arguments)]
fn drive_smoke(
    mut smoke: ResMut<Smoke>,
    mut tutorial: MessageWriter<TutorialAction>,
    mut shell: MessageWriter<ShellCommand>,
    session: Option<Res<ActiveSession>>,
    status: Res<SessionStatus>,
    data: Res<EvoData>,
    mut ui: ResMut<EvoUi>,
    (mut input, mut buttons): (ResMut<WorldInput>, ResMut<ButtonInput<MouseButton>>),
    mut tracker: ResMut<ScreenshotTracker>,
    mut finished: MessageReader<ScreenshotFinished>,
    mut cameras: Query<&mut PanOrbitCamera, With<EvoCamera>>,
    views: Query<(&Camera, &GlobalTransform), With<EvoCamera>>,
    transforms: Query<&GlobalTransform>,
    index: Res<OrganismIndex>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    smoke.frame += 1;
    let Some(path) = smoke.path.clone() else { return };
    // The world reacts only to the pointer we pretend to have.
    if smoke.cursor_on_selection {
        smoke.cursor =
            ui.selected.and_then(|id| index.map.get(&id)).and_then(|e| transforms.get(*e).ok()).and_then(|t| {
                let (camera, view) = views.iter().next()?;
                camera.world_to_viewport(view, t.translation() + Vec3::Y * 0.35).ok()
            });
    }
    match smoke.cursor.filter(|_| matches!(smoke.stage, Stage::Settle | Stage::Capture | Stage::Done)) {
        Some(cursor) => {
            input.cursor = Some(cursor);
            input.pointer = input.viewport.is_some_and(|v| v.contains(cursor));
        }
        None => input.pointer = false,
    }
    match smoke.stage {
        Stage::Load => {
            if smoke.frame == 5 {
                if let Some(id) = &smoke.scenario {
                    tutorial.write(TutorialAction(format!("load:{id}")));
                }
                smoke.stage = Stage::Run;
            }
        }
        Stage::Run if smoke.frame == 15 => {
            if let Some(action) = env("VIEW_SMOKE_ACTION") {
                tutorial.write(TutorialAction(action));
            }
        }
        Stage::Run => {
            if smoke.frame >= 25 && data.view.is_some() {
                if let (Some(ticks), Some(session)) = (smoke.play, &session) {
                    // Play flat out (not one big Step) so views, and the
                    // view-sampled charts, are published along the way.
                    let tick = status.get().map_or(0, |s| s.tick);
                    smoke.target_tick = tick + ticks;
                    session.session().set_speed(Speed::Unlimited);
                    session.session().play();
                }
                smoke.stage = Stage::Wait;
            }
        }
        Stage::Wait => {
            let tick = data.view.as_ref().map_or(0, |v| v.tick);
            if tick >= smoke.target_tick || status.get().is_some_and(|s| s.finished) {
                if let Some(session) = &session
                    && smoke.play.is_some()
                {
                    session.session().pause();
                    session.session().set_speed(Speed::TicksPerSecond(10.0));
                }
                smoke.stage = Stage::Arrange;
            }
        }
        Stage::Arrange if env("VIEW_SMOKE_REENTER").is_some() && smoke.settle == 0 => {
            shell.write(ShellCommand::OpenMode(AppMode::Dashboard));
            smoke.settle = 1;
            smoke.stage = Stage::Away;
        }
        Stage::Away => {
            smoke.settle += 1;
            if smoke.settle == 30 {
                shell.write(ShellCommand::OpenMode(AppMode::EvolutionLab));
                smoke.stage = Stage::Back;
            }
        }
        Stage::Back => {
            smoke.settle += 1;
            if smoke.settle >= 90 && data.view.is_some() {
                smoke.settle = 1000;
                smoke.stage = Stage::Arrange;
            }
        }
        Stage::Arrange => {
            if let Some(o) = &smoke.overlay
                && let Some(overlay) = GroundOverlay::ALL.into_iter().find(|g| g.label().eq_ignore_ascii_case(o))
            {
                ui.overlay = overlay;
            }
            if env("VIEW_SMOKE_COMPACT").is_some() {
                ui.genes_table_open = false;
                ui.legend_open = false;
            }
            if let Some(gene) = &smoke.gene {
                ui.select_gene(gene);
            }
            if let Some(tool) = &smoke.tool {
                tutorial.write(TutorialAction(format!("tool:{tool}")));
            }
            // Selecting opens the Inspector, so with a selection the tab
            // is opened afterwards (Settle stage).
            if smoke.select.is_none()
                && let Some(tab) = smoke.tab.as_deref()
            {
                open_tab(tab, &mut shell, &mut tutorial);
            }
            if let (Some(speed), Some(session)) = (smoke.speed, &session) {
                session.session().set_speed(Speed::TicksPerSecond(speed));
                session.session().play();
            }
            smoke.stage = Stage::Settle;
            smoke.settle = 0;
        }
        Stage::Settle => {
            smoke.settle += 1;
            // Select once the run has really stopped (a pause takes a few ticks to land).
            if smoke.settle == 4
                && let Some(which) = &smoke.select
            {
                let arg = if which == "predator" { "predator" } else { "prey" };
                tutorial.write(TutorialAction(format!("select:{arg}")));
            }
            // A click: press, then release two frames later at the same place.
            if env("VIEW_SMOKE_CLICK").is_some() && smoke.cursor.is_some() {
                if smoke.settle == 20 {
                    buttons.press(MouseButton::Left);
                } else if smoke.settle == 22 {
                    buttons.release(MouseButton::Left);
                }
            }
            if smoke.settle == 6
                && smoke.select.is_some()
                && let Some(tab) = smoke.tab.as_deref()
            {
                open_tab(tab, &mut shell, &mut tutorial);
            }
            if smoke.settle == 10
                && let Some(distance) = smoke.follow
                && ui.selected.is_some()
            {
                ui.follow = true;
                for mut cam in &mut cameras {
                    cam.target_radius = distance;
                    cam.target_pitch = 0.55;
                }
            }
            if smoke.settle >= smoke.frames {
                smoke.stage = Stage::Capture;
            }
        }
        Stage::Capture => {
            tracker.capture(&mut commands, path.clone());
            smoke.stage = Stage::Done;
        }
        Stage::Done => {
            if finished.read().any(|f| f.path == path) {
                exit.write(AppExit::Success);
            }
            if smoke.frame > 20_000 {
                exit.write(AppExit::error());
            }
        }
    }
}

/// Opens a right-panel tab, a Properties sub-tab or a shell window by name.
fn open_tab(tab: &str, shell: &mut MessageWriter<ShellCommand>, tutorial: &mut MessageWriter<TutorialAction>) {
    match tab {
        "parameters" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Parameters));
        }
        "rules" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Rules));
        }
        "metrics" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Metrics));
        }
        "tutorial" => {
            shell.write(ShellCommand::OpenWindow(ShellWindow::Tutorial));
        }
        "help" => {
            shell.write(ShellCommand::OpenWindow(ShellWindow::Help));
        }
        other => {
            tutorial.write(TutorialAction(format!("tab:{other}")));
        }
    }
}
