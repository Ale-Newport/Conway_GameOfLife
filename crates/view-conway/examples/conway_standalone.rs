//! The Classic Conway mode on its own: the application shell (configured
//! like `apps/desktop`) with only this mode registered, opened directly.
//!
//! ```text
//! cargo run -p view-conway --example conway_standalone
//! ```
//!
//! Smoke capture (renders, saves the window as a PNG and exits):
//!
//! | Variable | Meaning |
//! |---|---|
//! | `VIEW_SMOKE_SCREENSHOT=<png>` | enables the capture |
//! | `VIEW_SMOKE_FRAMES=<n>` | frame of the capture (default 90) |
//! | `VIEW_SMOKE_SCENARIO=<id>` | scenario to open (e.g. `random-soup-torus`) |
//! | `VIEW_SMOKE_PATTERN=<id>` | open a library pattern alone on an empty plane (frame 3) |
//! | `VIEW_SMOKE_STEPS=<n>` | run `n` ticks at once (frame 6) |
//! | `VIEW_SMOKE_PLAY=1` | play from frame 5 (`VIEW_SMOKE_SPEED=<ticks/s>`, default 60) |
//! | `VIEW_SMOKE_GPT=<n>` | generations per tick (frame 4) |
//! | `VIEW_SMOKE_ENGINE=<auto\|dense\|bit\|sparse\|hashlife>` | engine choice (frame 4) |
//! | `VIEW_SMOKE_TAB=<properties\|parameters\|rules\|metrics>` | right-panel tab |
//! | `VIEW_SMOKE_WINDOW=<library\|import\|export\|tutorial\|help>` | open a window |
//! | `VIEW_SMOKE_THEME=<light\|contrast>` | theme |
//! | `VIEW_SMOKE_STYLE=<classic\|flat\|circles\|neon\|age\|trails\|heat\|density>` | render style |
//! | `VIEW_SMOKE_COLORS=<lab\|ivory\|phosphor\|amber\|paper>` | cell colours |
//! | `VIEW_SMOKE_TOOL=<draw\|erase\|line\|rect\|fill\|select\|stamp>` | active tool |
//! | `VIEW_SMOKE_STAMP=<id>` | library pattern as the stamp (selects the stamp tool) |
//! | `VIEW_SMOKE_ROTATE=<quarter turns>`, `VIEW_SMOKE_MIRROR=1` | stamp orientation |
//! | `VIEW_SMOKE_FILLED=1`, `VIEW_SMOKE_SHAPE_ERASE=1` | filled rectangles, shapes that erase |
//! | `VIEW_SMOKE_CURSOR=<fx>,<fy>` | pretend the pointer is at this fraction of the world viewport |
//! | `VIEW_SMOKE_DRAG=<fx0>,<fy0>,<fx1>,<fy1>` | drag with the left button from frame 20 to 50 |
//! | `VIEW_SMOKE_HOLD=1` | keep the button held at the end of the drag (shows the drag preview) |
//! | `VIEW_SMOKE_FIT=<x0>,<y0>,<x1>,<y1>` | frame this cell rectangle at frame 12 |
//! | `VIEW_SMOKE_SELECT=<x0>,<y0>,<x1>,<y1>` | select this rectangle at frame 8 |
//! | `VIEW_SMOKE_JUMP=<k>` | Hashlife jump of 2^k generations at frame 30 |
//! | `VIEW_SMOKE_ACTION=<action>` | tutorial action at frame 40 (e.g. `pattern:glider`, `jump:20`) |
//! | `VIEW_SMOKE_ANALYSE=1` | period analysis at frame 30 |
//! | `VIEW_SMOKE_EXPORT=1` | generate the RLE export at frame 30 |
//! | `VIEW_SMOKE_IMPORT=<text>` | put this text (`\n` for newlines) into the import tab |
//! | `VIEW_SMOKE_SEARCH=<text>`, `VIEW_SMOKE_SELECTED=<id>` | library search and selected entry |
//! | `VIEW_SMOKE_REENTER=1` | go to the dashboard at frames 15 and 45 and back at 25 and 55 |

use std::path::{Path, PathBuf};

use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use cellular_automata::{CellRect, Rotation};
use interface::{
    AppMode, InterfacePlugin, ModeScenarios, RightPanelTab, ScreenshotFinished, ScreenshotTracker, ShellCommand,
    ShellWindow, ThemeKind, ThemeSettings, TutorialAction,
};
use rendering::{GridCameraController, WorldInput, WorldInputPlugin};
use simulation_core::Speed;
use view_conway::commands::{ModeCommand, ModeCommands, TimeSettings};
use view_conway::library::ConwayLibrary;
use view_conway::session::{LoadRequest, LoadRequests};
use view_conway::style::{CellColors, RenderStyle, ViewSettings};
use view_conway::tools::{Selection, StampPattern, StampSource, Tool, ToolSettings};
use view_conway::window::{PatternsTab, PatternsWindow};
use view_conway::{ConwayViewPlugin, ConwayViewSystems, MODE};

/// Frames to wait for the screenshot before giving up.
const TIMEOUT_FRAMES: u32 = 600;
/// Frames over which `VIEW_SMOKE_DRAG` drags the pointer.
const DRAG_FRAMES: (u32, u32) = (20, 50);

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

fn numbers<T: std::str::FromStr>(name: &str) -> Option<Vec<T>> {
    env(name).map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect())
}

fn main() -> AppExit {
    let assets = asset_root();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Emergence Lab — Classic Conway".into(),
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
    .add_plugins(ConwayViewPlugin);
    if let Some(path) = env("VIEW_SMOKE_SCREENSHOT") {
        let path = std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(&path));
        let frames = env("VIEW_SMOKE_FRAMES").and_then(|v| v.parse().ok()).unwrap_or(90);
        app.insert_resource(Smoke { path, frames, frame: 0, requested: false })
            .add_systems(PostStartup, choose_scenario)
            .add_systems(Update, (smoke_setup, smoke_runtime, smoke_capture).chain())
            .add_systems(Update, fake_cursor.before(ConwayViewSystems::Input));
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

#[allow(clippy::too_many_arguments)]
fn smoke_setup(
    smoke: Res<Smoke>,
    mut shell: MessageWriter<ShellCommand>,
    mut theme: ResMut<ThemeSettings>,
    mut settings: ResMut<ViewSettings>,
    mut tools: ResMut<ToolSettings>,
    mut window: ResMut<PatternsWindow>,
    mut queue: ResMut<ModeCommands>,
    mut loads: ResMut<LoadRequests>,
    mut library: ResMut<ConwayLibrary>,
    mut selection: ResMut<Selection>,
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
                Some("library") => window.open(PatternsTab::Library),
                Some("import") => window.open(PatternsTab::Import),
                Some("export") => window.open(PatternsTab::Export),
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
            if let Some(colors) = env("VIEW_SMOKE_COLORS") {
                settings.colors = match colors.as_str() {
                    "ivory" => CellColors::Ivory,
                    "phosphor" => CellColors::Phosphor,
                    "amber" => CellColors::Amber,
                    "paper" => CellColors::Paper,
                    _ => CellColors::Lab,
                };
            }
            if let Some(tool) = env("VIEW_SMOKE_TOOL") {
                tools.tool = match tool.as_str() {
                    "erase" => Tool::Erase,
                    "line" => Tool::Line,
                    "rect" => Tool::Rectangle,
                    "fill" => Tool::Fill,
                    "select" => Tool::Select,
                    "stamp" => Tool::Stamp,
                    _ => Tool::Draw,
                };
            }
            if let Some(turns) = env("VIEW_SMOKE_ROTATE").and_then(|v| v.parse::<u8>().ok()) {
                tools.rotation =
                    [Rotation::None, Rotation::Cw90, Rotation::Half, Rotation::Ccw90][usize::from(turns % 4)];
            }
            if env("VIEW_SMOKE_MIRROR").is_some() {
                tools.mirror = true;
            }
            tools.rect_filled |= env("VIEW_SMOKE_FILLED").is_some();
            tools.shape_erase |= env("VIEW_SMOKE_SHAPE_ERASE").is_some();
            if let Some(text) = env("VIEW_SMOKE_IMPORT") {
                window.import_text = text.replace("\\n", "\n");
                window.reparse();
            }
            if let Some(query) = env("VIEW_SMOKE_SEARCH") {
                window.query = query;
            }
            if let Some(id) = env("VIEW_SMOKE_SELECTED") {
                window.selected = Some(id);
            }
        }
        3 => {
            if let Some(id) = env("VIEW_SMOKE_PATTERN") {
                loads.0.push(LoadRequest::Pattern(id));
            }
            if let Some(id) = env("VIEW_SMOKE_STAMP") {
                match library.pattern(&id) {
                    Ok(pattern) => {
                        let name = library.entry(&id).map_or_else(|| id.clone(), |e| e.name.clone());
                        queue.0.push(ModeCommand::UseStamp(StampPattern {
                            name,
                            source: StampSource::Library(id),
                            pattern,
                        }));
                    }
                    Err(e) => error!("stamp: {e}"),
                }
            }
        }
        4 => {
            if let Some(n) = env("VIEW_SMOKE_GPT").and_then(|v| v.parse::<u64>().ok()) {
                queue.0.push(ModeCommand::SetGenerationsPerTick(n));
            }
            if let Some(engine) = env("VIEW_SMOKE_ENGINE").and_then(|e| cellular_automata::EngineChoice::parse(&e)) {
                queue.0.push(ModeCommand::SetEngine(engine));
            }
            if let Some(style) = env("VIEW_SMOKE_STYLE") {
                let style = match style.as_str() {
                    "flat" => RenderStyle::Flat,
                    "circles" => RenderStyle::Circles,
                    "neon" => RenderStyle::Neon,
                    "age" => RenderStyle::Age,
                    "trails" => RenderStyle::Trails,
                    "heat" => RenderStyle::Heat,
                    "density" => RenderStyle::Density,
                    _ => RenderStyle::Classic,
                };
                queue.0.push(ModeCommand::SetStyle(style));
            }
        }
        5 if env("VIEW_SMOKE_PLAY").is_some() => {
            let speed = env("VIEW_SMOKE_SPEED").and_then(|v| v.parse::<f64>().ok()).unwrap_or(60.0);
            shell.write(ShellCommand::SetSpeed(Speed::TicksPerSecond(speed)));
            shell.write(ShellCommand::TogglePlay);
        }
        6 => {
            if let Some(n) = env("VIEW_SMOKE_STEPS").and_then(|v| v.parse::<u64>().ok()) {
                shell.write(ShellCommand::Step(n));
            }
        }
        8 => {
            if let Some(v) = numbers::<i64>("VIEW_SMOKE_SELECT")
                && let [x0, y0, x1, y1] = v[..]
            {
                selection.set(CellRect::new(x0, y0, x1, y1));
            }
        }
        _ => {}
    }
}

/// Runtime actions: framing, jumps, analyses, exports, tutorial actions and
/// re-entering the mode.
#[allow(clippy::too_many_arguments)]
fn smoke_runtime(
    smoke: Res<Smoke>,
    mut queue: ResMut<ModeCommands>,
    mut time: ResMut<TimeSettings>,
    mut actions: MessageWriter<TutorialAction>,
    mut shell: MessageWriter<ShellCommand>,
    mut cameras: Query<&mut GridCameraController>,
    mut window: ResMut<PatternsWindow>,
) {
    if smoke.frame == 12
        && let Some(v) = numbers::<i64>("VIEW_SMOKE_FIT")
        && let [x0, y0, x1, y1] = v[..]
    {
        for mut camera in &mut cameras {
            camera.fit_to_rect(I64Vec2::new(x0, y0), I64Vec2::new(x1, y1), 0.0);
        }
    }
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
    if smoke.frame == 30 {
        if let Some(k) = env("VIEW_SMOKE_JUMP").and_then(|v| v.parse::<u32>().ok()) {
            time.jump_log2 = k;
            queue.0.push(ModeCommand::Jump(k));
        }
        if env("VIEW_SMOKE_ANALYSE").is_some() {
            queue.0.push(ModeCommand::Analyse);
        }
        if env("VIEW_SMOKE_EXPORT").is_some() {
            window.open(PatternsTab::Export);
            window.export_request = true;
        }
    }
    if smoke.frame == 40
        && let Some(action) = env("VIEW_SMOKE_ACTION")
    {
        actions.write(TutorialAction(action));
    }
}

/// Moves the (virtual) pointer over the world for previews of the tools
/// and, with `VIEW_SMOKE_DRAG`, drags it with the left button held.
fn fake_cursor(smoke: Res<Smoke>, mut input: ResMut<WorldInput>, mut buttons: ResMut<ButtonInput<MouseButton>>) {
    let Some(viewport) = input.viewport else { return };
    let at = |fx: f32, fy: f32| viewport.min + viewport.size() * Vec2::new(fx, fy);
    if let Some(v) = numbers::<f32>("VIEW_SMOKE_DRAG")
        && let [x0, y0, x1, y1] = v[..]
    {
        let (start, end) = DRAG_FRAMES;
        let hold = env("VIEW_SMOKE_HOLD").is_some();
        if (start..=end).contains(&smoke.frame) || (hold && smoke.frame > end) {
            let t = ((smoke.frame.min(end) - start) as f32 / (end - start) as f32).min(1.0);
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
    let Some(v) = numbers::<f32>("VIEW_SMOKE_CURSOR") else { return };
    let [fx, fy] = v[..] else { return };
    input.cursor = Some(at(fx, fy));
    input.pointer = true;
}

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
            exit_watchdog(if message.saved { 0 } else { 1 });
        }
    }
    if smoke.frame > smoke.frames + TIMEOUT_FRAMES {
        error!("screenshot timed out");
        exit.write(AppExit::error());
        exit_watchdog(1);
    }
}

/// Bevy 0.20 on macOS can deadlock while tearing the window down (the render
/// thread drops the winit window, which must happen on the main thread, while
/// the main thread waits for the render thread). The PNG is written by then,
/// so a stuck exit must not hang scripted captures: exit after 10 s anyway.
fn exit_watchdog(code: i32) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(10));
        std::process::exit(code);
    });
}
