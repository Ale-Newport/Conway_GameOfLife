//! The Custom Life Creator on its own: the application shell with only this
//! mode registered, opened directly.
//!
//! ```text
//! cargo run -p view-custom-life --example custom_life_standalone
//! ```
//!
//! Smoke capture (all optional except the first):
//!
//! | Variable | Effect |
//! |---|---|
//! | `VIEW_SMOKE_SCREENSHOT=<png>` | capture the window and exit |
//! | `VIEW_SMOKE_FRAMES=<n>` | frames to render after setup before capturing (default 60) |
//! | `VIEW_SMOKE_SCENARIO=<id>` | open a preset first (e.g. `bacterial-growth`) |
//! | `VIEW_SMOKE_PLAY=1` or `=<ticks>` | run 400 (or `<ticks>`) ticks before capturing |
//! | `VIEW_SMOKE_ACTION=<id>[,<id>…]` | tutorial actions to send in order (e.g. `tutorial:start,tutorial:plants`) |
//! | `VIEW_SMOKE_TAB=<name>` | `parameters`, `rules`, `metrics`, `tutorial`, `help`, or an editor tab: `entities`, `rules-editor`, `relationships`, `world`, `scripts`, `files` |
//! | `VIEW_SMOKE_RULE_MODE=text` | show the selected rule as DSL text instead of the visual builder |
//! | `VIEW_SMOKE_SELECT=1` or `=<type>` | select an entity (of that type) |
//! | `VIEW_SMOKE_OVERLAY=<field>` | show a field overlay |
//! | `VIEW_SMOKE_COLOR=<type\|energy\|state>` | entity colour mode |
//! | `VIEW_SMOKE_COLORMAP=<name>` | field colormap (`viridis`, `inferno`, `magma`, `plasma`, `turbo-like`, `grayscale`) |
//! | `VIEW_SMOKE_ZOOM=<factor>` | zoom in by this factor (on the selection if any) |
//! | `VIEW_SMOKE_CURSOR=<x>,<y>` | pretend the pointer rests there (window logical px) |
//! | `VIEW_SMOKE_TOOL=<select\|spawn\|remove\|paint>` | active world tool |
//! | `VIEW_SMOKE_THEME=<light\|contrast>` | theme |
//! | `VIEW_SMOKE_REENTER=1` | go to the dashboard and back before capturing (lifecycle check: the world resumes) |
//! | `VIEW_SMOKE_SET=<key>=<value>[;…]` | set parameters as the Parameters tab would (e.g. `entity.plant.metabolism=0.05`) |
//! | `VIEW_SMOKE_GLYPHS=1` | show a sheet of the glyph quick picks and symbols (font coverage check) |

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution, WindowTheme};
use interface::{
    ActiveSession, AppMode, InterfacePlugin, RightPanelTab, ScreenshotFinished, ScreenshotTracker, SessionStatus,
    ShellCommand, ShellWindow, ThemeKind, ThemeSettings, TutorialAction,
};
use rendering::{WorldInput, WorldInputPlugin};
use simulation_core::Speed;
use view_custom_life::editor::rules::RuleMode;
use view_custom_life::world::ClCamera;
use view_custom_life::{ClData, ClEditor, ClUi, CustomLifeSystems, CustomLifeViewPlugin, EditorTab};

fn asset_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("EMERGENCE_LAB_ASSETS").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("assets");
    std::fs::canonicalize(&workspace).unwrap_or(workspace)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Load,
    Run,
    Play,
    Wait,
    Away,
    Back,
    Arrange,
    Settle,
    Capture,
    Done,
}

#[derive(Resource, Debug)]
struct Smoke {
    path: PathBuf,
    frames: u32,
    stage: Stage,
    frame: u32,
    settle: u32,
    target_tick: u64,
    play: Option<u64>,
    cursor: Option<Vec2>,
}

fn main() -> AppExit {
    let root = asset_root();
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
                    title: "Emergence Lab — Custom Life Creator".into(),
                    resolution: WindowResolution::new(1600, 1000),
                    present_mode: PresentMode::AutoVsync,
                    window_theme: Some(WindowTheme::Dark),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin { file_path: root.to_string_lossy().into_owned(), ..default() }),
    )
    .add_plugins((
        WorldInputPlugin,
        InterfacePlugin { asset_root: root, initial_mode: Some(AppMode::CustomLifeCreator) },
    ))
    .add_plugins(CustomLifeViewPlugin)
    .insert_resource(ThemeSettings { kind: theme });
    if let Some(path) = env("VIEW_SMOKE_SCREENSHOT") {
        let smoke = Smoke {
            path: std::path::absolute(path).unwrap_or_default(),
            frames: env("VIEW_SMOKE_FRAMES").and_then(|f| f.parse().ok()).unwrap_or(60),
            stage: Stage::Load,
            frame: 0,
            settle: 0,
            target_tick: 0,
            play: env("VIEW_SMOKE_PLAY").map(|p| match p.parse::<u64>() {
                Ok(1) | Err(_) => 400,
                Ok(n) => n,
            }),
            cursor: env("VIEW_SMOKE_CURSOR").and_then(|c| {
                let (x, y) = c.split_once(',')?;
                Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
            }),
        };
        app.insert_resource(smoke)
            .add_systems(Update, drive_smoke.after(interface::ShellSet::Poll).before(CustomLifeSystems::Data))
            .add_systems(Update, pretend_cursor.after(CustomLifeSystems::Data).before(CustomLifeSystems::Input));
        if env("VIEW_SMOKE_GLYPHS").is_some() {
            app.add_systems(bevy_egui::EguiPrimaryContextPass, glyph_sheet);
        }
    }
    app.run()
}

/// Font coverage sheet: every quick-pick glyph and the symbols the mode uses.
fn glyph_sheet(mut contexts: bevy_egui::EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    bevy_egui::egui::Window::new("Glyph sheet").default_pos([700.0, 300.0]).show(ctx, |ui| {
        for (i, g) in view_custom_life::colors::GLYPHS.iter().enumerate() {
            ui.label(bevy_egui::egui::RichText::new(format!("{i:2}: {g}")).size(18.0));
        }
        ui.label("symbols: · — × … ≤ ≥ − Σ “ ” • ⚠ ✔ ⚙ ° ± µ π ƒ ≈ ★ ☘ ✿ ⬢ ⬟ ⬣ ⊕ ◈ ✦ ✖ ✚");
        ui.label("emoji: 🦠 🟠 🟣 🔵 🔴 ✨ 🎐 🦂 🤖 🧫 🌕 🌾 🌳 🐄 🐁 🐙 💧 🔥 ⭐ 🍀 🌵 💎 🧪 👾 🚀");
    });
}

/// Pretends the pointer rests at `VIEW_SMOKE_CURSOR` once the scene is arranged.
fn pretend_cursor(smoke: Res<Smoke>, mut input: ResMut<WorldInput>) {
    let arranged = matches!(smoke.stage, Stage::Settle | Stage::Capture | Stage::Done);
    match smoke.cursor.filter(|_| arranged) {
        Some(cursor) => {
            input.cursor = Some(cursor);
            input.pointer = input.viewport.is_some_and(|v| v.contains(cursor));
        }
        None => input.pointer = false,
    }
}

#[allow(clippy::too_many_arguments)]
fn drive_smoke(
    mut smoke: ResMut<Smoke>,
    mut tutorial: MessageWriter<TutorialAction>,
    mut shell: MessageWriter<ShellCommand>,
    session: Option<Res<ActiveSession>>,
    status: Res<SessionStatus>,
    data: Res<ClData>,
    mut ui: ResMut<ClUi>,
    mut editor: ResMut<ClEditor>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<ClCamera>>,
    mut tracker: ResMut<ScreenshotTracker>,
    mut finished: MessageReader<ScreenshotFinished>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    smoke.frame += 1;
    let path = smoke.path.clone();
    match smoke.stage {
        Stage::Load => {
            if smoke.frame == 5 {
                if let Some(id) = env("VIEW_SMOKE_SCENARIO") {
                    tutorial.write(TutorialAction(format!("load:{id}")));
                }
                smoke.stage = Stage::Run;
            }
        }
        Stage::Run => {
            if smoke.frame >= 25 && data.view.is_some() && data.info.is_some() {
                if let Some(actions) = env("VIEW_SMOKE_ACTION") {
                    for a in actions.split(',') {
                        tutorial.write(TutorialAction(a.trim().to_string()));
                    }
                }
                smoke.settle = 0;
                smoke.stage = Stage::Play;
            }
        }
        Stage::Play => {
            // Let actions that replace the simulation land before playing.
            smoke.settle += 1;
            if smoke.settle == 1
                && let (Some(sets), Some(session)) = (env("VIEW_SMOKE_SET"), &session)
            {
                for item in sets.split(';') {
                    if let Some((key, value)) = item.split_once('=') {
                        let value = simulation_core::ParamValue::parse_loose(value.trim());
                        session.session().set_param(key.trim(), value);
                    }
                }
            }
            if smoke.settle >= 20 {
                if let (Some(ticks), Some(session)) = (smoke.play, &session) {
                    let tick = status.get().map_or(0, |s| s.tick);
                    smoke.target_tick = tick + ticks;
                    session.session().set_speed(Speed::Unlimited);
                    session.session().play();
                }
                smoke.settle = 0;
                smoke.stage = Stage::Wait;
            }
        }
        Stage::Wait => {
            smoke.settle += 1;
            let tick = data.tick();
            let done = smoke.play.is_none() || tick >= smoke.target_tick || status.get().is_some_and(|s| s.finished);
            if done && smoke.settle > 5 {
                if let Some(session) = &session
                    && smoke.play.is_some()
                {
                    session.session().pause();
                    session.session().set_speed(Speed::TicksPerSecond(10.0));
                }
                smoke.settle = 0;
                smoke.stage = if env("VIEW_SMOKE_REENTER").is_some() { Stage::Away } else { Stage::Arrange };
            }
            if smoke.settle > 20_000 {
                smoke.stage = Stage::Arrange;
            }
        }
        Stage::Away => {
            if smoke.settle == 0 {
                shell.write(ShellCommand::OpenMode(AppMode::Dashboard));
            }
            smoke.settle += 1;
            if smoke.settle == 30 {
                shell.write(ShellCommand::OpenMode(AppMode::CustomLifeCreator));
                smoke.settle = 0;
                smoke.stage = Stage::Back;
            }
        }
        Stage::Back => {
            smoke.settle += 1;
            if smoke.settle >= 60 && data.view.is_some() {
                smoke.settle = 0;
                smoke.stage = Stage::Arrange;
            }
        }
        Stage::Arrange => {
            if let Some(field) = env("VIEW_SMOKE_OVERLAY") {
                ui.overlay = Some(field);
            }
            if let Some(name) = env("VIEW_SMOKE_COLORMAP")
                && let Some(map) =
                    view_custom_life::state::FIELD_COLORMAPS.into_iter().find(|m| m.label().eq_ignore_ascii_case(&name))
            {
                ui.colormap = map;
            }
            if let Some(mode) = env("VIEW_SMOKE_COLOR") {
                tutorial.write(TutorialAction(format!("color:{mode}")));
            }
            if let Some(tool) = env("VIEW_SMOKE_TOOL") {
                tutorial.write(TutorialAction(format!("tool:{tool}")));
            }
            if let Some(which) = env("VIEW_SMOKE_SELECT") {
                let arg = if which == "1" { String::new() } else { which };
                tutorial.write(TutorialAction(format!("select:{arg}")));
            }
            if env("VIEW_SMOKE_RULE_MODE").as_deref() == Some("text") {
                editor.rules.mode = RuleMode::Text;
            }
            if let Some(tab) = env("VIEW_SMOKE_TAB") {
                match tab.as_str() {
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
                    "rules-editor" => editor.open_at(EditorTab::Rules),
                    other => {
                        if let Some(t) = EditorTab::from_name(other) {
                            editor.open_at(t);
                        }
                    }
                }
            }
            smoke.stage = Stage::Settle;
            smoke.settle = 0;
        }
        Stage::Settle => {
            smoke.settle += 1;
            if smoke.settle == 8
                && let Some(zoom) = env("VIEW_SMOKE_ZOOM").and_then(|z| z.parse::<f32>().ok())
            {
                let focus = ui
                    .selected
                    .and_then(|id| data.view.as_ref()?.entities.iter().find(|e| e.id == id).map(|e| e.position));
                for (mut t, mut p) in &mut cameras {
                    if let Projection::Orthographic(o) = &mut *p {
                        o.scale /= zoom.max(0.01);
                    }
                    if let Some(f) = focus {
                        t.translation.x = f.x;
                        t.translation.y = -f.y;
                    }
                }
                ui.fit_requested = false;
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
                // Bevy 0.20 on macOS can deadlock while tearing the window
                // down (the render thread drops the winit window off the main
                // thread); the PNG is written, so a stuck exit must not hang
                // scripted captures.
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    std::process::exit(0);
                });
            }
            if smoke.frame > 40_000 {
                exit.write(AppExit::error());
            }
        }
    }
}
