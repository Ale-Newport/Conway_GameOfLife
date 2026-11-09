//! A minimal but complete mode for the shell, used to check the mode layout
//! and as a reference for view-crate authors (see `docs/UI_SHELL.md`).
//!
//! It runs Conway's Game of Life (B3/S23) on a 96 × 64 torus inside a
//! `Runner`, renders it with a `WorldCamera` into a nearest-sampled texture,
//! toggles cells with the left mouse button (undoable) and contributes left,
//! right and overlay panels.
//!
//! ```text
//! cargo run -p interface --example shell_demo
//! SHELL_DEMO_SCREENSHOT=out.png cargo run -p interface --example shell_demo   # capture and exit
//! SHELL_DEMO_VIEW=metrics ...   # parameters | rules | metrics | help | about | tutorial | save | load | light | contrast
//! ```

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_egui::egui;
use interface::widgets::{gradient_bar, legend_row};
use interface::{
    ActiveSession, AppMode, InterfacePlugin, ModeAppExt, ModeCard, ModeScenarios, ModeStatus, PanelSlot, RightPanelTab,
    ScenarioEntry, ScenarioList, ScenarioSelected, ScreenshotFinished, ScreenshotTracker, SessionStatus, ShellCommand,
    ShellWindow, ThemeKind, ThemeSettings, Tutorial, TutorialAction, TutorialAppExt, TutorialPage,
};
use rendering::{WorldCamera, WorldInput, WorldInputPlugin};
use serde::{Deserialize, Serialize};
use simulation_core::{
    ApplyPolicy, Control, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamSpec, ParamValue, Runner,
    RunnerConfig, SimError, SimRng, Simulation, Snapshot,
};

const W: usize = 96;
const H: usize = 64;
const MODEL_ID: &str = "demo.life";

/// The demo model: B3/S23 on a torus.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct DemoLife {
    cells: Vec<u8>,
    initial: Vec<u8>,
    tick: u64,
    seed: u64,
    density: f64,
    births: u64,
    deaths: u64,
    rng: SimRng,
}

enum DemoCommand {
    Toggle(usize, usize),
}

impl DemoLife {
    fn random(seed: u64, density: f64) -> Self {
        let mut rng = SimRng::seed_from_u64(seed);
        let cells: Vec<u8> = (0..W * H).map(|_| u8::from(rng.chance(density))).collect();
        Self { initial: cells.clone(), cells, tick: 0, seed, density, births: 0, deaths: 0, rng }
    }

    fn gosper_gun() -> Self {
        let mut sim = Self::random(0, 0.0);
        let rows = [
            "........................O...........",
            "......................O.O...........",
            "............OO......OO............OO",
            "...........O...O....OO............OO",
            "OO........O.....O...OO..............",
            "OO........O...O.OO....O.O...........",
            "..........O.....O.......O...........",
            "...........O...O....................",
            "............OO......................",
        ];
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                sim.cells[(y + 4) * W + x + 4] = u8::from(c == 'O');
            }
        }
        sim.initial = sim.cells.clone();
        sim
    }
}

impl Simulation for DemoLife {
    type Command = DemoCommand;
    type ViewRequest = ();
    type View = Vec<u8>;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(MODEL_ID, "Life B3/S23 (demo)", ModelFamily::Cellular)
            .summary("Conway's Game of Life on a 96 x 64 torus.")
            .rules(
                "## Rule B3/S23\n\
                 Every cell has 8 neighbours (Moore neighbourhood), wrapping around the edges (torus).\n\n\
                 - A **dead** cell with exactly `3` live neighbours becomes alive (birth).\n\
                 - A **live** cell with `2` or `3` live neighbours stays alive (survival).\n\
                 - Every other cell is dead in the next generation.\n\n\
                 All cells update synchronously.",
            )
            .params(vec![
                ParamSpec::float("density", "Randomize density", 0.0, 1.0, 0.3)
                    .describe(
                        "Probability that a cell is alive after Randomize. Reset returns to the \
                         scenario's own initial cells, which do not depend on it.",
                    )
                    .group("World")
                    .apply(ApplyPolicy::Immediate),
            ])
            .metrics(vec![
                MetricSpec::gauge("population", "Population", "cells", "Live cells after the generation."),
                MetricSpec::counter("births", "Births", "Cells that became alive in the last generation."),
                MetricSpec::counter("deaths", "Deaths", "Cells that died in the last generation."),
            ])
    }
    fn tick(&self) -> u64 {
        self.tick
    }
    fn step(&mut self) {
        let mut next = vec![0u8; W * H];
        let (mut births, mut deaths) = (0, 0);
        for y in 0..H {
            for x in 0..W {
                let mut n = 0;
                for dy in [H - 1, 0, 1] {
                    for dx in [W - 1, 0, 1] {
                        if (dx, dy) != (0, 0) {
                            n += self.cells[((y + dy) % H) * W + (x + dx) % W];
                        }
                    }
                }
                let alive = self.cells[y * W + x] == 1;
                let on = n == 3 || (alive && n == 2);
                births += u64::from(on && !alive);
                deaths += u64::from(alive && !on);
                next[y * W + x] = u8::from(on);
            }
        }
        self.cells = next;
        self.births = births;
        self.deaths = deaths;
        self.tick += 1;
    }
    fn reset(&mut self) {
        self.cells = self.initial.clone();
        self.tick = 0;
        self.births = 0;
        self.deaths = 0;
        self.rng = SimRng::seed_from_u64(self.seed);
    }
    fn apply(&mut self, command: DemoCommand) -> Result<(), SimError> {
        match command {
            DemoCommand::Toggle(x, y) if x < W && y < H => {
                self.cells[y * W + x] ^= 1;
                Ok(())
            }
            DemoCommand::Toggle(x, y) => Err(SimError::InvalidCommand(format!("cell ({x}, {y}) is outside the world"))),
        }
    }
    fn params(&self) -> ParamSet {
        ParamSet::new().with("density", ParamValue::Float(self.density))
    }
    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let spec = self.descriptor().param(key).cloned().ok_or_else(|| SimError::UnknownParameter(key.into()))?;
        self.density = spec.validate(&value)?.as_f64().unwrap_or(self.density);
        Ok(spec.apply)
    }
    fn metrics(&self) -> Metrics {
        let population = self.cells.iter().map(|&c| u64::from(c)).sum::<u64>();
        Metrics::new(self.tick, self.tick as f64)
            .with("population", population as f64)
            .with("births", self.births as f64)
            .with("deaths", self.deaths as f64)
    }
    fn view(&self, _request: &()) -> Vec<u8> {
        self.cells.clone()
    }
    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(MODEL_ID, 1, self.tick, self)
    }
    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        *self = snapshot.decode(MODEL_ID, 1)?;
        Ok(())
    }
    fn memory_bytes(&self) -> usize {
        self.cells.len() * 2
    }
}

#[derive(Resource, Clone)]
struct DemoRunner(Arc<Runner<DemoLife>>);

#[derive(Resource)]
struct DemoTexture {
    image: Handle<Image>,
    seq: u64,
}

const MODE: AppMode = AppMode::ClassicConway;

fn main() {
    let mut app = App::new();
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window { title: "Emergence Lab — shell demo".into(), ..default() }),
        ..default()
    }))
    .add_plugins((WorldInputPlugin, InterfacePlugin { asset_root: assets, initial_mode: Some(MODE) }))
    .register_mode(ModeCard { status: ModeStatus::Preview, ..ModeCard::defaults(MODE) })
    .register_tutorial(
        MODE,
        Tutorial {
            pages: vec![
                TutorialPage::new("Draw", "Click cells in the world to toggle them. Each click can be undone."),
                TutorialPage::new("Guns", "The Gosper gun emits a glider every **30** generations.")
                    .with_action("gosper", "Load the gun"),
            ],
            concepts: "## Gliders\nA glider moves one cell diagonally every 4 generations.".into(),
        },
    )
    .add_mode_panel(MODE, PanelSlot::Left, tools_panel)
    .add_mode_panel(MODE, PanelSlot::Right, properties_panel)
    .add_mode_panel(MODE, PanelSlot::Overlay, overlay_panel)
    .add_systems(Startup, fill_scenarios)
    .add_systems(OnEnter(MODE), enter)
    .add_systems(OnExit(MODE), exit)
    .add_systems(
        Update,
        (upload_view, click_cells, load_scenarios).run_if(in_state(MODE)).after(interface::ShellSet::Poll),
    );
    if let Some(path) = std::env::var_os("SHELL_DEMO_SCREENSHOT") {
        app.insert_resource(Capture { path: std::path::absolute(path).unwrap_or_default(), frame: 0 })
            .add_systems(Update, (capture_and_exit, apply_view).chain());
    }
    app.run();
}

fn fill_scenarios(mut scenarios: ResMut<ModeScenarios>) {
    let entry = |id: &str, title: &str, description: &str| ScenarioEntry {
        id: id.into(),
        title: title.into(),
        description: description.into(),
    };
    scenarios.set(
        MODE,
        ScenarioList {
            entries: vec![
                entry("random", "Random soup", "30% of cells alive, seed 7"),
                entry("gosper", "Gosper glider gun", "Period-30 gun on an empty torus"),
            ],
            selected: Some("random".into()),
        },
    );
}

fn spawn_runner(sim: DemoLife) -> Arc<Runner<DemoLife>> {
    Arc::new(
        Runner::spawn(sim, RunnerConfig::default()).with_clear(|sim: &mut DemoLife| sim.cells.fill(0)).with_randomize(
            |sim: &mut DemoLife, seed| {
                let mut rng = SimRng::seed_from_u64(seed);
                for c in &mut sim.cells {
                    *c = u8::from(rng.chance(sim.density));
                }
            },
        ),
    )
}

fn enter(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let runner = spawn_runner(DemoLife::random(7, 0.3));
    commands.insert_resource(ActiveSession::new(runner.clone()));
    commands.insert_resource(DemoRunner(runner));
    let mut image = Image::new_fill(
        Extent3d { width: W as u32, height: H as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    let handle = images.add(image);
    commands.insert_resource(DemoTexture { image: handle.clone(), seq: 0 });
    commands.spawn((Camera2d, WorldCamera, DespawnOnExit(MODE)));
    commands.spawn((
        Sprite { image: handle, custom_size: Some(Vec2::new(W as f32 * 10.0, H as f32 * 10.0)), ..default() },
        DespawnOnExit(MODE),
    ));
}

fn exit(mut commands: Commands) {
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<DemoRunner>();
    commands.remove_resource::<DemoTexture>();
}

fn upload_view(runner: Res<DemoRunner>, mut texture: ResMut<DemoTexture>, mut images: ResMut<Assets<Image>>) {
    let Some(published) = runner.0.latest_view() else { return };
    if published.seq == texture.seq {
        return;
    }
    texture.seq = published.seq;
    let Some(mut image) = images.get_mut(&texture.image) else { return };
    let Some(data) = image.data.as_mut() else { return };
    let (pixels, _) = data.as_chunks_mut::<4>();
    for (px, &alive) in pixels.iter_mut().zip(published.view.iter()) {
        *px = if alive == 1 { [90, 169, 255, 255] } else { [14, 18, 25, 255] };
    }
}

fn click_cells(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    runner: Res<DemoRunner>,
    camera: Single<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !(input.pointer && buttons.just_pressed(MouseButton::Left)) {
        return;
    }
    let (camera, transform) = *camera;
    let Some(cursor) = input.cursor else { return };
    let Ok(world) = camera.viewport_to_world_2d(transform, cursor) else { return };
    let x = ((world.x + W as f32 * 5.0) / 10.0).floor();
    let y = ((H as f32 * 5.0 - world.y) / 10.0).floor();
    // Clicks beside the grid (the viewport is larger than the world) do nothing.
    if (0.0..W as f32).contains(&x) && (0.0..H as f32).contains(&y) {
        runner.0.send(Control::Edit(DemoCommand::Toggle(x as usize, y as usize)));
    }
}

fn load_scenarios(
    mut picked: MessageReader<ScenarioSelected>,
    mut tutorial: MessageReader<TutorialAction>,
    mut scenarios: ResMut<ModeScenarios>,
    runner: Res<DemoRunner>,
) {
    let ids = picked.read().map(|s| s.id.clone()).chain(tutorial.read().map(|a| a.0.clone()));
    for id in ids.collect::<Vec<_>>() {
        let sim = if id == "gosper" { DemoLife::gosper_gun() } else { DemoLife::random(7, 0.3) };
        runner.0.send(Control::Replace(Box::new(sim)));
        scenarios.mark_selected(MODE, &id);
    }
}

fn tools_panel(InMut(ui): InMut<egui::Ui>, status: Res<SessionStatus>) {
    ui.label(egui::RichText::new("Pencil").strong());
    ui.label("Left-click a cell in the world to toggle it (undoable).");
    ui.add_space(8.0);
    if let Some(status) = status.get() {
        ui.label(format!("History snapshots: {}", status.history_ticks.len()));
    }
}

fn properties_panel(InMut(ui): InMut<egui::Ui>) {
    ui.label(egui::RichText::new("Colours").strong());
    legend_row(ui, &[(egui::Color32::from_rgb(90, 169, 255), "alive"), (egui::Color32::from_rgb(14, 18, 25), "dead")]);
    ui.add_space(6.0);
    gradient_bar(
        ui,
        &[egui::Color32::from_rgb(14, 18, 25), egui::Color32::from_rgb(90, 169, 255)],
        200.0,
        "dead",
        "alive",
    );
}

fn overlay_panel(InMut(ui): InMut<egui::Ui>, history: Res<interface::MetricsHistory>) {
    egui::Frame::popup(ui.style()).show(ui, |ui| {
        let population = history.latest("population").unwrap_or(0.0);
        ui.label(format!("Population {population:.0}"));
    });
}

#[derive(Resource)]
struct Capture {
    path: std::path::PathBuf,
    frame: u32,
}

/// Opens the view named in `SHELL_DEMO_VIEW` (tab, window or theme).
fn apply_view(capture: Res<Capture>, mut shell: MessageWriter<ShellCommand>, mut theme: ResMut<ThemeSettings>) {
    if capture.frame != 30 {
        return;
    }
    match std::env::var("SHELL_DEMO_VIEW").unwrap_or_default().as_str() {
        "parameters" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Parameters));
        }
        "rules" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Rules));
        }
        "metrics" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Metrics));
        }
        "help" => {
            shell.write(ShellCommand::OpenWindow(ShellWindow::Help));
        }
        "about" => {
            shell.write(ShellCommand::OpenWindow(ShellWindow::About));
        }
        "tutorial" => {
            shell.write(ShellCommand::OpenWindow(ShellWindow::Tutorial));
        }
        "save" => {
            shell.write(ShellCommand::OpenWindow(ShellWindow::Save));
        }
        "load" => {
            shell.write(ShellCommand::OpenWindow(ShellWindow::Load));
        }
        "light" => theme.kind = ThemeKind::Light,
        "contrast" => theme.kind = ThemeKind::HighContrast,
        _ => {}
    }
}

fn capture_and_exit(
    mut capture: ResMut<Capture>,
    mut tracker: ResMut<ScreenshotTracker>,
    mut commands: Commands,
    mut finished: MessageReader<ScreenshotFinished>,
    mut exit: MessageWriter<AppExit>,
    runner: Option<Res<DemoRunner>>,
) {
    capture.frame += 1;
    if capture.frame == 20
        && let Some(runner) = runner
    {
        runner.0.step(120);
    }
    if capture.frame == 60 {
        tracker.capture(&mut commands, capture.path.clone());
    }
    if finished.read().any(|f| f.path == capture.path) {
        exit.write(AppExit::Success);
    }
}
