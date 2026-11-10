# Emergence Lab — application shell (`interface`) and desktop app

This document is the contract between the application shell (crate
`interface`, binary `apps/desktop` / package `emergence-lab`) and the
**view crates** that implement laboratories ("modes"), followed by a
reference of every control. Read `docs/ARCHITECTURE.md` first; Bevy API facts
come from `docs/research/bevy-0.20-api.md`.

## 1. What the shell does

| Area | Responsibility |
|---|---|
| States | `AppMode` (Bevy `States`): `Dashboard` (default) and nine simulation modes |
| Dashboard | Hero header and one card per mode; registered modes open, others are labelled *in development* and disabled |
| Top bar | Home, mode title, scenario picker, Save, Load, Export menu, Fullscreen, Tutorial, Help, Theme, About/Credits, panel toggles |
| Side panels | Left "Tools" (mode content), right tabs *Properties* (mode content), *Parameters*, *Rules*, *Metrics*; collapsible and resizable |
| Bottom bar | Time controls driven by `ActiveSession`, history timeline, live readouts |
| World viewport | Sets the `Viewport` of every `rendering::WorldCamera` to the area left by the panels; writes `rendering::WorldInput` |
| Services | Metric time series, toasts, save/load, exports, screenshots, tutorials, credits, themes, shortcuts |

Frame flow:

```text
PreUpdate   update_world_input        WorldInput from last frame's layout + egui input state
            reserve_tab_for_shell     Tab is kept from egui while no widget has focus (see 3.3)
Update      ShellSet::Poll            SessionStatus, MetricsHistory (drain_metrics), errors -> toasts
            ShellSet::Commands        keyboard shortcuts -> ShellCommand -> execution
            after Poll                pending save -> background writer; finished background jobs -> toasts
PostUpdate  EguiPrimaryContextPass    shell_ui (exclusive): top bar, panels (+ mode one-shot systems), windows, toasts
            after EguiPostUpdateSet::EndPass, before CameraUpdateSystems
                                      WorldCamera viewports = central rectangle
Last        screenshots / PNG sequences requested and tracked
```

## 2. Mode integration contract (for view-crate authors)

A view crate is a Bevy `Plugin` added in `apps/desktop/src/main.rs` inside
`register_modes` (between the `BEGIN/END MODE REGISTRATION` markers):

```rust
fn register_modes(app: &mut App) {
    app.add_plugins(view_conway::ConwayViewPlugin);
}
```

A complete, runnable reference mode (real B3/S23 Life on a torus with a
`Runner`, a `WorldCamera` texture, cell editing through `WorldInput`, left /
right / overlay panels, scenarios and a tutorial) is
`crates/interface/examples/shell_demo.rs`:

```text
cargo run -p interface --example shell_demo
SHELL_DEMO_VIEW=metrics SHELL_DEMO_SCREENSHOT=out.png cargo run -p interface --example shell_demo
```

### 2.1 Register the card

```rust
use interface::{AppMode, ModeAppExt, ModeCard, ModeStatus};

app.register_mode(ModeCard {
    status: ModeStatus::Preview,            // Ready | Preview (badge on the card)
    ..ModeCard::defaults(AppMode::ClassicConway) // title, tagline, description, accent, glyph
});
```

Only registered modes can be opened (dashboard card, `--mode`, `OpenMode`
command). Registering the dashboard or the same mode twice is logged and
ignored. The card's `accent` colours the whole shell while the mode is open
(selection, sliders, plots, tabs).

### 2.2 Provide the session: `ActiveSession`

On `OnEnter(mode)` insert `ActiveSession(Arc<dyn SessionControl>)`; on
`OnExit(mode)` remove it. Keep your own typed handle for views and commands:

```rust
#[derive(Resource, Clone)]
struct ConwayRunner(Arc<Runner<LifeModel>>);

fn enter(mut commands: Commands) {
    let runner = Arc::new(
        Runner::spawn(LifeModel::new(/* scenario */), RunnerConfig::default())
            .with_clear(|sim| sim.clear())                    // enables the Clear button
            .with_randomize(|sim, seed| sim.randomize(seed)), // enables Randomize
    );
    commands.insert_resource(ActiveSession::new(runner.clone()));
    commands.insert_resource(ConwayRunner(runner));
}
fn exit(mut commands: Commands) {
    commands.remove_resource::<ActiveSession>();
    commands.remove_resource::<ConwayRunner>(); // dropping the last Arc stops the runner's thread
}
app.add_systems(OnEnter(AppMode::ClassicConway), enter)
   .add_systems(OnExit(AppMode::ClassicConway), exit);
```

Rules:

* **The shell is the only consumer of `drain_metrics()` and `take_errors()`.**
  Read metrics from `Res<MetricsHistory>` (`store()`, `latest(key)`) and push
  your own messages to `ResMut<Toasts>`. Systems that read `SessionStatus` or
  `MetricsHistory` should run `.after(interface::ShellSet::Poll)`.
* `SessionStatus` holds the `RunnerStatus` read once this frame; use it instead
  of calling `status()` again.
* Publish a metric with key **`population`** if the model has a natural
  population: the bottom bar shows its latest value.
* The model's `ModelDescriptor` drives the Parameters, Rules and Metrics tabs:
  keep `rules` (Markdown) in sync with the code, give every `ParamSpec` a
  `description`, `group` and correct `ApplyPolicy`, and describe every metric.
  The descriptor may depend on the running system: after every control that
  can change the state (parameters, commands, edits, closures, reset,
  undo/redo, history jumps, restores, `Control::Replace`; not after plain
  ticks or queries) the runner rebuilds it on the simulation thread and
  publishes it if it changed, and it rebuilds it once more after the next
  tick: a parameter with `ApplyPolicy::NextTick` (set with `SetParam` or by
  a closure, or carried by a restored snapshot) only takes effect inside
  that tick, so the rules text names a new rule from the first tick that
  runs it. `SessionControl::descriptor()` returns the current
  `Arc<ModelDescriptor>` (the shell reads it once per frame) and
  `RunnerStatus::descriptor_seq` counts the changes, so a mode can rebuild
  what it derives from the descriptor when the number moves. To show the
  model with adapted texts (glyphs egui's fonts cannot draw, a shorter
  summary, hidden parameters), spawn with
  `Runner::spawn_with_descriptor(sim, config, |sim| adapt(sim.descriptor()))`
  instead of wrapping the runner in another `SessionControl`.
* After those controls the runner also records a metric sample of the
  current tick (replacing a sample of the same tick), so the bottom-bar
  population follows edits made while paused, loaded saves and closures that
  advance the model by themselves.
* Read the model with `Runner::query(|sim: &S| …)` (`Control::Query`): the
  closure runs on the simulation thread after the controls sent before and
  its result arrives on a `Receiver` (poll it with `try_recv`, never block
  a frame). A query changes nothing: no descriptor rebuild, metric sample or
  view publication. `Runner::request`, `Runner::exec` and `Control::Exec`
  take `&mut S` and count as state changes; use them only for closures that
  change the model. `SessionControl::request_snapshot()` (Save) is a query.
* Dropping the last `Runner` handle never waits: the worker is told to stop
  and exits once it has handled the controls sent before (a long Hashlife
  jump keeps running on its thread, not on the UI thread). Tests that must
  know the thread has ended call `Runner::shutdown_and_wait()`;
  `simulation_core::live_workers()` counts the worker threads still alive in
  the process (diagnostics).
* Save/Load uses `snapshot()` / `restore()`; the descriptor `id` must be stable
  (it is checked when loading) and snapshots carry your `schema_version`.
* Replacing the whole simulation (new scenario) through
  `Control::Replace` keeps the same `ActiveSession`; re-inserting a different
  `ActiveSession` clears the metric plots.
* While the mode has no `ActiveSession` the world area shows *Starting the
  simulation…*. A mode that loads asynchronously or fails fatally (missing
  assets, for example) replaces that text through `ResMut<ModePlaceholders>`:
  `set(mode, ToastLevel::Error, "…")` (`Warning` and `Error` are drawn in
  their theme colours, `Info` and `Success` muted; long texts wrap) and
  `clear(mode)` to go back to the default. The entry belongs to the mode:
  set it when entering or when the failure happens, clear it when it no
  longer applies.

### 2.3 World cameras and input

* Spawn cameras with `rendering::WorldCamera` (and `DespawnOnExit(mode)`).
  Do **not** set `Camera::viewport`: the shell overwrites it every frame with
  the central rectangle (physical pixels, edges rounded to the nearest pixel).
  Keep `order` below `interface::UI_CAMERA_ORDER` (1000; the shell's egui
  camera). The default clear colour (`ClearColor`) follows the theme.
* Gate world interaction on `Res<rendering::WorldInput>`:
  `pointer` is true only when the cursor is inside the world viewport and
  egui is not using the pointer (windows, popups, drags, the overlay area);
  `keyboard` is true when no egui widget has keyboard focus.
  `cursor_in_viewport()` gives viewport-relative coordinates. For
  `Camera::viewport_to_world*` pass the **window** cursor position
  (`WorldInput::cursor`); Bevy subtracts the viewport origin itself.
* Background (crib §21.5): egui does not report hovering a side `Panel`
  drawn in the shell's root `Ui`. The shell compensates by (1) shrinking every
  `WorldCamera` viewport to the central area (Bevy picking and
  `bevy_panorbit_camera` only react inside a viewport) and (2) computing
  `WorldInput.pointer` from the central rectangle. Use `WorldInput`, not
  `EguiWantsInput`, in world systems.
* While no `WorldCamera` is active the shell paints the central area with the
  theme background, so a mode without a camera still looks clean.
* Camera key controls (`GridCameraController`, `OrbitCameraRig`) ignore keys
  pressed while Ctrl, Cmd or Alt is held (`rendering::shortcut_modifier_held`):
  such chords are shortcuts (Cmd+S, Ctrl+Z…), and mode key handlers should
  use the same test. `GridCameraController::key_zoom` (zoom with `=` / `-`
  and numpad `+` / `-`) is off by default because those keys are the
  shell's Faster / Slower shortcuts (3.3); enable it only in apps without
  the shell.

### 2.4 Panel contents: one-shot systems with `InMut<egui::Ui>`

Mode panels are ordinary Bevy systems whose first parameter is
`InMut<egui::Ui>`; the shell runs them with `World::run_system_with` from its
exclusive egui system. This was verified on Bevy 0.20.1 / bevy_egui 0.43.1
(test `mode::tests::one_shot_panel_systems_run_with_a_ui`).

```rust
use bevy::prelude::*;
use bevy_egui::egui;
use interface::{AppMode, ModeAppExt, PanelSlot};

fn conway_tools(InMut(ui): InMut<egui::Ui>, runner: Res<ConwayRunner>, mut tool: ResMut<Tool>) {
    ui.heading("Draw");
    ui.radio_value(&mut *tool, Tool::Pencil, "Pencil");
    if ui.button("Glider gun").clicked() { /* runner.0.edit(...) */ }
}

app.add_mode_panel(AppMode::ClassicConway, PanelSlot::Left, conway_tools)
   .add_mode_panel(AppMode::ClassicConway, PanelSlot::Right, conway_properties)
   .add_mode_panel(AppMode::ClassicConway, PanelSlot::Toolbar, conway_toolbar)
   .add_mode_panel(AppMode::ClassicConway, PanelSlot::Overlay, conway_legend);
```

| Slot | Where | Notes |
|---|---|---|
| `Left` | Left panel "Tools", inside a vertical `ScrollArea` | Without a left system the panel is not shown and its toggle is disabled |
| `Right` | First right tab "Properties", inside a `ScrollArea` | Without it the tabs start at *Parameters* |
| `Toolbar` | Strip below the top bar, laid out horizontally | Hidden with the panels (Tab) |
| `Overlay` | `egui::Area` anchored 12 px inside the top-left of the world viewport, clipped to it | Interactive; it blocks world pointer input where it is drawn |

`ModePanels { left, right, toolbar, overlay }` (each `Option<PanelSystem>`,
`PanelSystem = SystemId<InMut<'static, egui::Ui>>`) can also be built
manually with `World::register_system` and installed with
`app.set_mode_panels(mode, panels)`.

Panel systems run inside `PostUpdate` (in the egui pass). They may read and
write any resource (`Toasts`, `MetricsHistory`, your own), use `Commands`
(applied when the system finishes) and write messages. A system that Bevy
*skips* (e.g. a `Single<..>` with no match) simply draws nothing; a system
that *fails* (e.g. `Res<T>` for a resource that does not exist — use
`Option<Res<T>>` for optional ones) is reported as an error toast every
frame it fails (identical toasts merge into one with a repeat counter).

### 2.5 Reusable widgets (`interface::widgets`)

| Widget | Purpose |
|---|---|
| `param_editor(ui, &ModelDescriptor, &ParamSet, &dyn SessionControl)` | Controls generated from `ParamSpec` (bool, int/float sliders incl. logarithmic, text applied on Enter, choice, colour), grouped by `group`, with *live / next tick / on reset* badges, reset-to-default buttons and a notice for the session's last change (with "Reset now" for `RequiresReset`). Linear float sliders snap edits to the spec's `step` (counted from `min`; `SliderClamping::Edits`, so a value off that grid is shown as it is and never sent back merely by drawing it); logarithmic sliders are not stepped (a fixed step would make the low end unreachable); values are clamped to `[min, max]` before sending because the model rejects out-of-range values. Colours are edited as unmultiplied **sRGB** (the convention of `ParamValue::parse_loose("#rrggbb")`) |
| `metrics_panel(ui, &TimeSeriesStore, &[MetricSpec], x_label, &Palette, &mut MetricsPanelState)` | Metric picker, `egui_plot` time series over ticks (M4 decimation keeps extremes), window (all / last N ticks), latest/min/max/mean table, optional histogram (Freedman–Diaconis bins, recomputed at most every 0.25 s while data arrives). Pass `widgets::rules::tick_label(&descriptor)` as `x_label`: the tick unit only when one tick is one unit |
| `histogram_plot(ui, id, &analytics::Histogram, colour, height, x_label)` | Bar chart of a histogram with under/overflow note |
| `rules_panel(ui, &ModelDescriptor, &Palette)` / `markdown(ui, text)` | Model identity, time step, determinism, rules in Markdown (headings, lists, bold, italic, code, fenced code, quotes, rules, pipe tables), metrics guide |
| `legend_chip`, `legend_row`, `swatch`, `badge`, `gradient_bar`, `color_ramp` | Legends and colour helpers; `theme::color32_from_rgba` / `theme::srgba_bytes` convert `ParamValue::Color` |
| `rules::time_step_text`, `rules::tick_label`, `rules::simulated_time_text`, `rules::counter_rate_label` | Honest labels for ticks and simulated time (`1 tick = 6 minutes`; 1,000 ticks are `6,000 minutes`, never "minute 1,000") |
| `icons::{Icon, icon_button, paint_icon, primary_button}` | The shell's vector icon set and buttons, for mode toolbars |

The current `Palette` is `Palette::new(Res<ThemeSettings>.kind, registry.accent(mode))`.

### 2.6 Scenarios

Fill the top-bar picker through `ResMut<ModeScenarios>`:

```rust
scenarios.set(AppMode::ClassicConway, ScenarioList {
    entries: vec![ScenarioEntry { id: "gosper".into(), title: "Gosper glider gun".into(), description: "...".into() }],
    selected: Some("gosper".into()),
});
```

When the user picks one the shell marks it selected and writes
`ScenarioSelected { mode, id }`; the mode loads it (for example with
`Control::Replace`).

### 2.7 Tutorials and concepts

```rust
app.register_tutorial(AppMode::ClassicConway, Tutorial {
    pages: vec![
        TutorialPage::new("A glider", "Gliders move one cell diagonally every **4** generations.")
            .with_action("load:glider", "Load a glider"),
    ],
    concepts: "## Still lifes\n...".into(),
});
```

The window shows *Steps* (previous / next) and *Concepts* (Markdown). A
page's action button writes `TutorialAction(String)`; read it with
`MessageReader<TutorialAction>`. The dashboard has a built-in tutorial.

### 2.8 Other hooks

* `ShellCommand` messages (`OpenMode`, `TogglePlay`, `Step(n)`, `Reset`,
  `Clear`, `Randomize(seed)`, `Undo`, `Redo`, `SetSpeed`, `Faster`, `Slower`,
  `JumpToHistory(i)`, `TogglePanels`, `ToggleFullscreen`, `Screenshot`,
  `ShowRightTab(RightPanelTab)`, `OpenWindow(ShellWindow)`) can be written by
  modes too, e.g. a tutorial step that opens the Rules tab.
* `ScreenshotTracker::capture(&mut commands, path)` + `ScreenshotFinished`.
* `DataLocation::dirs()` gives the per-user folders (`saves/`, `scenarios/`,
  `exports/`, `screenshots/`).

## 3. Controls reference

### 3.1 Top bar

| Control | Function |
|---|---|
| Logo / "Emergence Lab" | Back to the dashboard (Home) |
| Mode title, model name | Hover the model name for its summary |
| Scenario | Scenarios of the mode (`ModeScenarios`) |
| Panel toggles | Show/hide the tools panel (disabled without a left system) and the right panel |
| Save | Name the state; saved as `<data>/saves/<mode>/<name>.elsnap` (complete state including the RNG). "Overwrite" must be ticked to replace a save. The snapshot is taken on the simulation thread and the file is written on a background thread (a toast reports the result); a second save is refused until the first is written |
| Load | Saves of this mode with tick, date and size; *Load* restores (pauses, undoable, clears the plots); saves from another model or unreadable files are listed but disabled with the reason; *Delete…* asks for confirmation. The plots stay empty until the status shows the restored state, so samples of the previous run never mix with the loaded one |
| Export | Metrics as CSV or JSON (`<data>/exports/<mode>-<stamp>-metrics.csv/json`, written on a background thread from a copy taken at the click), screenshot PNG, PNG sequence of N rendered frames (`<data>/screenshots/<mode>-<stamp>-frames/frame-00001.png`), stop recording |
| Fullscreen | Borderless fullscreen toggle (F11) |
| Tutorial | Tutorial / concepts of the mode (disabled if none is registered) |
| Help | Shortcuts and mouse notes (F1) |
| Theme | Dark lab (default), Light, High contrast |
| About | Version, data folder, libraries and licences, asset attributions read from `assets/models/*/ATTRIBUTION.txt`, pattern-library licence counts from `assets/patterns/catalog.toml`, OTCA provenance (`assets/patterns/otca/PROVENANCE.md`) |

### 3.2 Bottom bar

| Control / readout | Function |
|---|---|
| Play / Pause | Toggle running (disabled when the scenario has finished) |
| Step, Step × N | Run 1 or N ticks now (N editable) |
| Reset | Scenario's initial state (undoable) |
| Clear, Randomize + seed | Only shown when the runner declares `with_clear` / `with_randomize`; the seed advances after each randomisation |
| Undo / Redo | Enabled from `RunnerStatus::can_undo/can_redo` |
| Speed | Logarithmic slider 1–10 000 ticks/s; **Max** = `Speed::Unlimited` |
| Timeline | Markers at stored history snapshots (`history_ticks`), cursor at the current tick; click a marker to restore it (`jump_to_history`, pauses, undoable) |
| State | RUNNING / PAUSED / FINISHED |
| Generation / Tick | `RunnerStatus::tick`, labelled with the tick unit when one tick is one unit (`Generation`), else `Tick` |
| Time | Simulated time `tick × dt` in the model's unit; shown only when `dt ≠ 1` (e.g. `6,000 minutes`) |
| Population | Latest `population` metric, if published |
| Ticks | Measured ticks/s and the target |
| Generations/s (unit/s) | Rate of the tick counter (`measured_gps`), e.g. `Generations/s`; differs from ticks/s for engines that jump several generations per tick (Hashlife). Hidden when it would repeat ticks/s (unit `tick`, or `dt ≠ 1`) |
| Tick | Average tick duration (EMA from the runner) |
| FPS | `FrameTimeDiagnosticsPlugin` smoothed frame rate |
| Memory | Process resident memory (`memory-stats`) · model estimate (`memory_bytes`) · stored history snapshots |

### 3.3 Keyboard shortcuts

Ignored while an egui widget has keyboard focus (a text field, a number
being typed); press Escape or click the world to give the keyboard back.
Simulation shortcuts only act inside a mode. Inside a mode, Tab is not passed
to egui while no widget has focus: egui would otherwise move keyboard focus
to the first widget, which then swallows every later shortcut.

| Keys | Action |
|---|---|
| Space | Play / pause |
| N | Step one tick |
| Shift+N | Step the configured number of ticks |
| R | Reset (undoable) |
| Ctrl/Cmd+Z | Undo |
| Shift+Ctrl/Cmd+Z, Ctrl/Cmd+Y | Redo |
| = or + (Shift+=), Numpad + | Faster (1, 2, 5, 10 … 10 000 ticks/s, then Max) |
| -, Numpad - | Slower |
| Home | Dashboard |
| Tab | Hide / show side panels, toolbar and bottom bar |
| F11 | Fullscreen |
| F12 | Screenshot |
| F1 | Help |

The table mirrors `interface::input::SHORTCUTS`, the single source of the
key mapping and of the Help window's list (a unit test checks that every
shortcut is documented here).

### 3.4 Themes

All colours come from `theme::Palette` tokens. The light theme darkens the
mode accent until it has at least 3:1 contrast on panels; the high-contrast
theme uses black surfaces, white text, 1.5 px outlines and brightens the
accent to at least 7:1 (checked by unit tests for every mode accent).
Selections (selected buttons and labels, selected text, the filled part of
sliders) are painted with an opaque mix of the panel and the accent (35 %)
in every theme: bevy_egui blends in linear light, where a translucent accent
over the light panel would wash out to grey (a unit test checks the colour
as composited on screen).
Text in the accent colour is the accent shaded (darkened in the light
theme, lightened in the dark ones) until it reads as text, at least 4.5:1
(7:1 in high contrast): selected text and icons (selected labels and
buttons, text selections, the focused text-edit outline) on the selection
fill, and links on the panel and on the stripes of striped grids. Links are
egui's link widgets (`ui.link`, `ui.hyperlink_to`, as in the About window),
which paint in `Visuals::hyperlink_color`, and Markdown emphasis is the text
colour tinted toward it; draw clickable text with them rather than with
`Palette::accent`. Unit tests check every theme and mode accent.

## 4. Desktop application

```text
cargo run -p emergence-lab                                  # dashboard
cargo run -p emergence-lab -- --mode classic-conway          # open a mode directly
cargo run -p emergence-lab -- --smoke-screenshot out.png --smoke-frames 60
```

* Window "Emergence Lab", 1600 × 1000 logical pixels, dark window theme.
* `--mode <name>` accepts slugs (`classic-conway`, `ecosystem`, `custom-life`,
  `conway-within-conway`, `cellular-automata`, `evolution`, `continuous-life`,
  `3d-life`, `civilizations`, `dashboard`), variant names or titles. An
  unregistered mode shows the dashboard and a warning.
* `--smoke-screenshot <png> [--smoke-frames N]` renders N frames (default 30),
  saves the window to the PNG and exits with status 0 (non-zero if the file
  could not be written or after 600 extra frames).
* Asset folder: `$EMERGENCE_LAB_ASSETS`, else `<executable dir>/assets`, else
  the workspace `assets/` (`CARGO_MANIFEST_DIR/../../assets`, compile time).
  The absolute path is passed to `AssetPlugin::file_path`, so the same binary
  works from `cargo run`, from `target/` and when packaged.
* Data folder: `persistence::UserDataDirs` (`$EMERGENCE_LAB_DATA` or the
  platform location, e.g. `~/Library/Application Support/EmergenceLab`).

## 5. Approximations and limitations

* Screenshots and PNG sequences capture the whole window as rendered, user
  interface included (hide panels with Tab first). Sequences take one PNG per
  **rendered frame**, not per simulation tick. Bevy's `save_to_disk` encodes
  each PNG on the main thread, so the frame rate drops while a sequence is
  recorded (no frame is skipped; the simulation runs on its own thread).
* Metric plots keep up to 100 000 samples per metric; beyond that the store
  keeps an M4 summary (first, minimum, maximum, last per bucket) of older
  data, and exports contain that summary (the Metrics tab says so).
* The shell polls the session once per frame, so readouts lag the simulation
  thread by at most one frame. `WorldInput` uses the previous frame's layout
  and egui state (egui runs late in `PostUpdate`).
* The Load dialog reads each save's tick from the first bytes of its payload
  (layout of `persistence::binary` + `Snapshot` field order); the whole file
  is validated (checksum, model id) only when it is loaded. Loading reads the
  file on the UI thread (a large save pauses the window briefly); saving and
  exporting write on background threads.
* The timeline addresses stored snapshots by index (`jump_to_history`, the
  core API). If the runner evicts its oldest snapshot (history budget full)
  between the frame that drew the marker and the click being processed, the
  jump lands on the neighbouring snapshot; the timeline then shows where the
  run actually is.
* *Step N* runs the N ticks as one runner command; the runner handles other
  controls (including Pause) only after they are done, so very large N on a
  slow model keeps it busy for that long.
* Only modes whose view crates are registered can be opened; all others are
  shown honestly as "in development".
