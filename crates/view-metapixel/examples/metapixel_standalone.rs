//! The Conway-within-Conway mode on its own: the application shell with
//! only this mode registered, opened directly (configured like
//! `apps/desktop`).
//!
//! ```text
//! cargo run -p view-metapixel --example metapixel_standalone
//! ```
//!
//! Smoke capture (renders, saves the window as a PNG and exits). Frames
//! counted from the moment the first construction is shown ("ready"):
//!
//! | Variable | Meaning |
//! |---|---|
//! | `VIEW_SMOKE_SCREENSHOT=<png>` | enables the capture |
//! | `VIEW_SMOKE_FRAMES=<n>` | frames after ready before the capture (default 60) |
//! | `VIEW_SMOKE_SCENARIO=<id>` | scenario to open (`metapixel.galaxy` or just `galaxy`) |
//! | `VIEW_SMOKE_PLAY=1` | play from ready + 3 |
//! | `VIEW_SMOKE_SPEED=<ticks/s>` | target speed when playing |
//! | `VIEW_SMOKE_STEPS=<n>` | run `n` ticks at ready + 3 |
//! | `VIEW_SMOKE_JUMPS=<n>` | advance to the next readout `n` times (one after the other) |
//! | `VIEW_SMOKE_WAIT_TICK=<t>` | before capturing, wait until the generation is at least `t` |
//! | `VIEW_SMOKE_STEP_SIZE=<gens>` | generations per tick (`1` … `32768`, `35328`) |
//! | `VIEW_SMOKE_PARAM=<k>=<v>[;...]` | set model parameters at ready + 1 (`rule=B36/S23;width=5`) |
//! | `VIEW_SMOKE_RESET=1` | Reset (rebuild) at ready + 2, after the parameters |
//! | `VIEW_SMOKE_SOUP=<density>` | random macro soup (model generator) at ready + 2, after the reset |
//! | `VIEW_SMOKE_TAB=<properties\|parameters\|rules\|metrics\|inspector\|validation\|construction>` | right panel |
//! | `VIEW_SMOKE_WINDOW=<tutorial\|help\|about>` | open a shell window |
//! | `VIEW_SMOKE_THEME=<light\|contrast>` | theme |
//! | `VIEW_SMOKE_LAYER=<auto\|cells\|macro>` | layer |
//! | `VIEW_SMOKE_SOURCE=<readout\|live\|prediction>` | macro layer source |
//! | `VIEW_SMOKE_TOOL=<inspect\|draw\|stamp>` | tool |
//! | `VIEW_SMOKE_STAMP=<pattern id>` | stamp pattern (`glider`, `r-pentomino`, ...) |
//! | `VIEW_SMOKE_SELECT=<col>,<row>` | pin a tile |
//! | `VIEW_SMOKE_ZOOM=<grid\|tile\|rules>` | frame the grid, the pinned tile or its rule table (ready + 6) |
//! | `VIEW_SMOKE_FIT=<x0>,<y0>,<x1>,<y1>` | frame a cell rectangle (ready + 6) |
//! | `VIEW_SMOKE_CURSOR=<fx>,<fy>` | pretend the pointer is at this fraction of the world viewport |
//! | `VIEW_SMOKE_CLICK=<fx>,<fy>` | left click there at ready + 8 |
//! | `VIEW_SMOKE_DRAG=<fx0>,<fy0>,<fx1>,<fy1>` | left drag from ready + 8 to ready + 20 |
//! | `VIEW_SMOKE_ACTION=<id>` | run a tutorial action at ready + 4 (`zoom:rules`, `watch:transition`...) |
//! | `VIEW_SMOKE_REENTER=1` | dashboard and back twice (ready + 10 … + 40) while running |
//! | `VIEW_SMOKE_PANELS=0` | hide the side panels (Tab) |
//! | `VIEW_SMOKE_OVERLAY=compact` | overlay without the details |
//! | `VIEW_SMOKE_SECTIONS=<time,rule,grid,view,shortcuts>` | open Tools sections (the others are closed) |

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use interface::{
    AppMode, InterfacePlugin, ModeScenarios, RightPanelTab, ScreenshotFinished, ScreenshotTracker, ShellCommand,
    ShellWindow, ThemeKind, ThemeSettings, TutorialAction,
};
use metapixel::spec::CellRect;
use rendering::{WorldInput, WorldInputPlugin};
use simulation_core::{Control, ParamValue, Speed};
use view_metapixel::{
    CameraFit, LayerMode, MODE, MacroSource, MetapixelViewSystems, MpSession, PanelSections, PropertiesTab, SimData,
    Tiles, Tool, ToolState, ViewSettings, jump_to_next_readout,
};

/// Frames to wait for the screenshot (and for the conditions) before giving up.
const TIMEOUT_FRAMES: u32 = 3_000;

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

fn floats(spec: &str) -> Vec<f32> {
    spec.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

fn main() -> AppExit {
    let assets = asset_root();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Emergence Lab — Conway within Conway".into(),
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
    .add_plugins(view_metapixel::Plugin);
    if let Some(path) = env("VIEW_SMOKE_SCREENSHOT") {
        let path = std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(&path));
        let frames = env("VIEW_SMOKE_FRAMES").and_then(|v| v.parse().ok()).unwrap_or(60);
        let jumps = env("VIEW_SMOKE_JUMPS").and_then(|v| v.parse().ok()).unwrap_or(0);
        app.insert_resource(Smoke { path, frames, frame: 0, ready: None, requested: false, jumps_left: jumps })
            .add_systems(PostStartup, choose_scenario)
            .add_systems(Update, (smoke_script, smoke_capture).chain().after(MetapixelViewSystems::Session))
            .add_systems(Update, fake_pointer.before(MetapixelViewSystems::Input));
    }
    app.run()
}

#[derive(Resource)]
struct Smoke {
    path: PathBuf,
    frames: u32,
    frame: u32,
    ready: Option<u32>,
    requested: bool,
    jumps_left: u32,
}

impl Smoke {
    /// Frames since the first construction was shown.
    fn since_ready(&self) -> Option<u32> {
        self.ready.map(|r| self.frame - r)
    }
}

/// Selects the scenario before the mode is entered.
fn choose_scenario(mut scenarios: ResMut<ModeScenarios>) {
    if let Some(id) = env("VIEW_SMOKE_SCENARIO") {
        let id = if id.starts_with("metapixel.") { id } else { format!("metapixel.{id}") };
        if !scenarios.mark_selected(MODE, &id) {
            error!("unknown scenario `{id}`");
        }
    }
}

/// The scripted session.
#[allow(clippy::too_many_arguments)]
fn smoke_script(
    mut smoke: ResMut<Smoke>,
    session: Option<Res<MpSession>>,
    mut data: ResMut<SimData>,
    mut shell: MessageWriter<ShellCommand>,
    mut actions: MessageWriter<TutorialAction>,
    mut theme: ResMut<ThemeSettings>,
    mut settings: ResMut<ViewSettings>,
    mut tools: ResMut<ToolState>,
    mut tab: ResMut<PropertiesTab>,
    mut fit: ResMut<CameraFit>,
    mut sections: ResMut<PanelSections>,
    state: Res<State<AppMode>>,
    tiles: Res<Tiles>,
) {
    smoke.frame += 1;
    if smoke.ready.is_none() {
        let failed = tiles.error().is_some();
        if (session.is_some() && data.view.is_some() || failed) && *state.get() == MODE {
            smoke.ready = Some(smoke.frame);
            info!("smoke: ready at frame {}", smoke.frame);
        } else {
            return;
        }
    }
    let Some(f) = smoke.since_ready() else { return };
    match f {
        0 => {
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
                Some(t @ ("properties" | "inspector" | "validation" | "construction")) => {
                    shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
                    *tab = match t {
                        "validation" => PropertiesTab::Validation,
                        "construction" => PropertiesTab::Construction,
                        _ => PropertiesTab::Inspector,
                    };
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
                Some("about") => {
                    shell.write(ShellCommand::OpenWindow(ShellWindow::About));
                }
                _ => {}
            }
            match env("VIEW_SMOKE_THEME").as_deref() {
                Some("light") => theme.kind = ThemeKind::Light,
                Some("contrast") => theme.kind = ThemeKind::HighContrast,
                _ => {}
            }
            match env("VIEW_SMOKE_LAYER").as_deref() {
                Some("cells") => settings.layer = LayerMode::Cells,
                Some("macro") => settings.layer = LayerMode::Macro,
                Some("auto") => settings.layer = LayerMode::Auto,
                _ => {}
            }
            match env("VIEW_SMOKE_SOURCE").as_deref() {
                Some("live") => settings.source = MacroSource::Live,
                Some("prediction") => settings.source = MacroSource::Prediction,
                Some("readout") => settings.source = MacroSource::Readout,
                _ => {}
            }
            match env("VIEW_SMOKE_TOOL").as_deref() {
                Some("draw") => tools.tool = Tool::Draw,
                Some("stamp") => tools.tool = Tool::Stamp,
                Some("inspect") => tools.tool = Tool::Inspect,
                _ => {}
            }
            if let Some(id) = env("VIEW_SMOKE_STAMP") {
                match view_metapixel_stamp(&id) {
                    Some(i) => tools.stamp = i,
                    None => error!("unknown stamp `{id}`"),
                }
            }
            if let Some(spec) = env("VIEW_SMOKE_SELECT") {
                let v: Vec<u32> = spec.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let [c, r] = v[..] {
                    tools.selected = Some((c, r));
                }
            }
            if let Some(list) = env("VIEW_SMOKE_SECTIONS") {
                let open = |name: &str| list.split(',').any(|s| s.trim() == name);
                *sections = PanelSections {
                    time: open("time"),
                    rule: open("rule"),
                    grid: open("grid"),
                    view: open("view"),
                    shortcuts: open("shortcuts"),
                };
            }
            if env("VIEW_SMOKE_OVERLAY").as_deref() == Some("compact") {
                settings.overlay_details = false;
            }
            if env("VIEW_SMOKE_PANELS").as_deref() == Some("0") {
                shell.write(ShellCommand::TogglePanels);
            }
        }
        1 => {
            let Some(session) = &session else { return };
            if let Some(step) = env("VIEW_SMOKE_STEP_SIZE") {
                session.send(Control::SetParam("step".into(), ParamValue::Text(step)));
            }
            if let Some(spec) = env("VIEW_SMOKE_PARAM") {
                for assignment in spec.split(';') {
                    match assignment.split_once('=') {
                        Some((k, v)) => {
                            session.send(Control::SetParam(k.trim().into(), ParamValue::parse_loose(v.trim())))
                        }
                        None => error!("bad VIEW_SMOKE_PARAM entry `{assignment}`"),
                    }
                }
            }
        }
        2 => {
            if let Some(session) = &session {
                if env("VIEW_SMOKE_RESET").is_some() {
                    session.send(Control::Reset);
                }
                if let Some(density) = env("VIEW_SMOKE_SOUP").and_then(|v| v.parse::<f64>().ok()) {
                    session.edit(metapixel::MetapixelCommand::Randomize { density });
                }
            }
        }
        3 => {
            let Some(session) = &session else { return };
            if let Some(n) = env("VIEW_SMOKE_STEPS").and_then(|v| v.parse::<u64>().ok()) {
                session.send(Control::Step(n));
            }
            if env("VIEW_SMOKE_PLAY").is_some() {
                let speed = env("VIEW_SMOKE_SPEED").and_then(|v| v.parse::<f64>().ok()).unwrap_or(10.0);
                session.send(Control::SetSpeed(Speed::TicksPerSecond(speed)));
                session.send(Control::Play);
            }
        }
        4 => {
            if let Some(action) = env("VIEW_SMOKE_ACTION") {
                actions.write(TutorialAction(action));
            }
        }
        6 => {
            if let Some(spec) = env("VIEW_SMOKE_FIT") {
                let v: Vec<i64> = spec.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if let [x0, y0, x1, y1] = v[..] {
                    fit.frame(CellRect { x0, y0, x1, y1 }, 0.0);
                }
            }
            if let Some(zoom) = env("VIEW_SMOKE_ZOOM") {
                let shape = data.view.as_ref().and_then(|v| v.macro_grid.as_ref()).map(|m| (m.columns, m.rows));
                let focus = tools.selected.or(shape.map(|(c, r)| (c / 2, r / 2)));
                match (zoom.as_str(), focus) {
                    ("tile", Some((c, r))) => {
                        let x = 2048 * i64::from(c);
                        let y = 2048 * i64::from(r);
                        fit.frame(CellRect { x0: x, y0: y, x1: x + 2047, y1: y + 2047 }, 0.06);
                    }
                    ("rules", Some((c, r))) => {
                        let x = 2048 * i64::from(c);
                        let y = 2048 * i64::from(r);
                        fit.frame(CellRect { x0: x + 100, y0: y + 1380, x1: x + 210, y1: y + 1540 }, 0.05);
                    }
                    _ => fit.request(false),
                }
            }
        }
        _ => {}
    }
    // Leave to the dashboard and come back (twice) with a running simulation.
    if env("VIEW_SMOKE_REENTER").is_some() {
        match f {
            10 | 30 => {
                shell.write(ShellCommand::OpenMode(AppMode::Dashboard));
            }
            20 | 40 => {
                shell.write(ShellCommand::OpenMode(MODE));
            }
            _ => {}
        }
    }
    // Jumps to the next readout, one after the other.
    if f >= 3
        && smoke.jumps_left > 0
        && data.jump.is_none()
        && let Some(session) = &session
    {
        jump_to_next_readout(session, &mut data);
        smoke.jumps_left -= 1;
    }
}

/// Index of a stamp pattern by library id.
fn view_metapixel_stamp(id: &str) -> Option<usize> {
    patterns::builtin_patterns().iter().position(|p| p.id == id)
}

/// Moves the (virtual) pointer over the world and clicks or drags.
fn fake_pointer(smoke: Res<Smoke>, mut input: ResMut<WorldInput>, mut buttons: ResMut<ButtonInput<MouseButton>>) {
    let Some(f) = smoke.since_ready() else { return };
    let Some(viewport) = input.viewport else { return };
    let at = |fx: f32, fy: f32| viewport.min + viewport.size() * Vec2::new(fx, fy);
    if let Some(spec) = env("VIEW_SMOKE_DRAG")
        && let [x0, y0, x1, y1] = floats(&spec)[..]
    {
        let (start, end) = (8u32, 20u32);
        if (start..=end).contains(&f) {
            let t = (f - start) as f32 / (end - start) as f32;
            input.cursor = Some(at(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t));
            input.pointer = true;
            if f == start {
                buttons.press(MouseButton::Left);
            }
        } else if f == end + 1 {
            buttons.release(MouseButton::Left);
        }
        return;
    }
    if let Some(spec) = env("VIEW_SMOKE_CLICK")
        && let [x, y] = floats(&spec)[..]
    {
        if f == 8 {
            input.cursor = Some(at(x, y));
            input.pointer = true;
            buttons.press(MouseButton::Left);
        } else if f == 9 {
            input.cursor = Some(at(x, y));
            input.pointer = true;
            buttons.release(MouseButton::Left);
        }
        if f <= 9 {
            return;
        }
    }
    if let Some(spec) = env("VIEW_SMOKE_CURSOR")
        && let [x, y] = floats(&spec)[..]
    {
        input.cursor = Some(at(x, y));
        input.pointer = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn smoke_capture(
    mut smoke: ResMut<Smoke>,
    data: Res<SimData>,
    session: Option<Res<MpSession>>,
    tiles: Res<Tiles>,
    mut commands: Commands,
    mut tracker: ResMut<ScreenshotTracker>,
    mut finished: MessageReader<ScreenshotFinished>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(f) = smoke.since_ready() else {
        if smoke.frame > TIMEOUT_FRAMES {
            error!("the construction never appeared");
            exit.write(AppExit::error());
        }
        return;
    };
    let wait_tick = env("VIEW_SMOKE_WAIT_TICK").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let conditions = tiles.error().is_some()
        || session.is_some()
            && data.view.is_some()
            && data.jump.is_none()
            && smoke.jumps_left == 0
            && data.tick() >= wait_tick;
    if !smoke.requested && f >= smoke.frames && (conditions || f > smoke.frames + TIMEOUT_FRAMES) {
        if !conditions {
            warn!("capturing although the scripted conditions were not reached");
        }
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
    if f > smoke.frames + 2 * TIMEOUT_FRAMES {
        error!("screenshot timed out");
        exit.write(AppExit::error());
    }
}
