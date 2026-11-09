# Bevy 0.20.1 / bevy_egui 0.43.1 / egui 0.36.2 / egui_plot 0.37.0 / bevy_panorbit_camera 0.36.0 — verified API crib sheet

Status: research reference for UI / rendering / desktop-app implementers. **Do not invent APIs: if it is not
in this file, check the registry sources before writing it.**

## 0. How this was verified (read this first)

| Tag | Meaning |
|-----|---------|
| **[C]** | Compile-verified: the snippet (or the file it is extracted from) passed `cargo check` in the scratch project, with **exactly the workspace feature set** `bevy = { version = "0.20.1", default-features = false, features = ["2d","3d","ui","mesh_picking"] }`, `bevy_egui = "0.43.1"` (default features), `bevy_panorbit_camera = { version = "0.36.0", features = ["bevy_egui"] }`, `egui_plot = "=0.37.0"`, Rust 1.98.1, edition 2024. |
| **[R]** | Runtime-verified: actually executed on this Mac (M2 Max, Metal). Windowed smoke tests rendered, took a screenshot, and exited cleanly. |
| **[S]** | Source-verified: read in `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/<crate>-<ver>/` (bevy examples or crate sources). |
| **[M]** | From Bevy's own 0.19→0.20 migration-guide drafts (`bevy-0.20.1/_release-content/migration-guides/*.md`), cross-checked against source. |
| **UNVERIFIED** | Not checked by compiler/runtime/source. Treat as a hint only. |

Scratch project used: `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/depcheck`
(examples `s01_app_ecs.rs` … `s10_error_handler.rs`, `egui_hover_test.rs`, shaders in `assets/shaders/`). Snippets
below are extracts of those files; the scratch dir is temporary, so this document is the durable copy.

Runtime results obtained:
- `s07_smoke` [R]: 3D scene, custom WESL `Material` sampling a CPU-updated `Image`, `Fox.glb` loaded via
  `WorldAssetRoot` + animation started from a `WorldInstanceReady` observer, HDR + Bloom + fog, gizmo grid, egui
  left/bottom `Panel`s + `egui::Window` + `egui_plot` + a Bevy `Image` shown inside egui, `PanOrbitCamera` whose
  viewport excludes the egui panels, `Screenshot` saved, `AppExit`. ~57 FPS in a debug build. Only warning:
  `bevy_egui::render: Bindless textures are not yet supported on metal. Disabling bindless mode.` (harmless).
- `s08_smoke_2d` [R]: `Camera2d` with `Projection::Orthographic` scale, `Sprite` with nearest-sampled CPU texture,
  `Material2d` with a WESL shader, second camera with `Viewport` + `RenderLayers`, 2D gizmos, screenshot.
- `s06_runtime_ecs_test` [R]: hierarchy despawn, required components, observers, messages, state transitions.
- `s10_error_handler` [R]: what returning `Err` from a system does.
- `egui_hover_test` [R]: how egui 0.36 hover detection behaves under bevy_egui 0.43 (important, see §21.5).
- Asset-root resolution (`AssetPlugin::file_path`, `BEVY_ASSET_ROOT`, exe dir) [R], see §17.

---

## 1. Cargo / features

Workspace (`/Users/alejandro/Projects/Conway_GameOfLife/Cargo.toml`) already pins:
```toml
bevy = { version = "0.20.1", default-features = false, features = ["2d", "3d", "ui", "mesh_picking"] }
bevy_egui = "0.43.1"
egui_plot = "0.37.0"
bevy_panorbit_camera = "0.36.0"
```
Facts [S] (from `bevy-0.20.1/Cargo.toml` `[features]`):
- `2d`/`3d`/`ui` pull `default_app` (asset, log, **state**, reflect_auto_register), `default_platform` (winit, gilrs,
  clipboard, `default_font`, multi_threaded, x11/wayland, `custom_cursor`, `sysinfo_plugin`), `scene`
  (`bevy_scene` = BSN + `bevy_world_serialization` = old DynamicScene/glTF scenes) and `picking`
  (`bevy_picking`, `mesh_picking`, `sprite_picking`, `ui_picking`).
- `3d` adds `bevy_pbr`, `bevy_gltf`, `gltf_animation`, `bevy_post_process` (bloom etc.), `bevy_anti_alias`,
  `tonemapping_luts`, `smaa_luts`, `ktx2`, `zstd_rust`. `common_api` adds `png` and `hdr` image formats.
- **Not included** with the workspace set: `audio`, `jpeg` (add `"jpeg"` if any texture is .jpg), `file_watcher`
  (hot reload), `dynamic_linking`, `serialize`, `bevy_dev_tools` (FPS overlay, state-transition logging),
  `pan_orbit_camera` (Bevy's *own* new camera controller — see §23 name clash).
- `bevy_egui` default features: `manage_clipboard`, `open_url`, `default_fonts`, `render`, `bevy_ui`, `picking`.
- **`bevy_panorbit_camera` only integrates with egui with its `bevy_egui` feature** (`features = ["bevy_egui"]`);
  the workspace entry currently does not enable it. [S]
- **egui version pinning**: `egui_plot 0.37.0` depends on `egui 0.36`, matching `bevy_egui 0.43.1` (`egui 0.36`).
  `egui_plot 0.36.x` depends on `egui 0.35` → two egui versions in the graph and type mismatches. Keep 0.37. [S]
- Use egui through the re-export: `use bevy_egui::egui;` (no direct `egui` dependency needed). [C]
- Edition 2024: `gen` is a reserved keyword (`let gen = …`, `struct S { gen: u64 }` fail to parse). [C]

Toolchain note: the workspace already sets `[profile.dev.package."*"] opt-level = 3` (Bevy in debug is otherwise slow).

---

## 2. Import-path cheat table (all [C] unless noted)

| Item | Path |
|------|------|
| Most things (App, Commands, Query, Res, Single, Transform, Camera2d/3d, Projection, Mesh3d, MeshMaterial3d, StandardMaterial, Sprite, Image, Assets, AssetServer, Gizmos, Time, Timer, KeyCode, MouseButton, ButtonInput, States, NextState, OnEnter, DespawnOnExit, AnimationPlayer, AnimationGraph, AnimationTransitions, GltfAssetLabel, WorldAssetRoot, MeshPickingPlugin, PointerClick, Pickable, MeshRayCast, MeshRayCastSettings, GlobalAmbientLight, DirectionalLight, PointLight, Message, MessageReader/Writer, Event, EntityEvent, On, Add, Remove, ChildOf, Children, children!, Isometry2d/3d, Dir3, Ray3d, `Result`) | `bevy::prelude::*` |
| `PrimaryWindow`, `WindowMode`, `PresentMode`, `WindowResolution`, `CursorOptions`, `WindowResized` | `bevy::window::*` (`MonitorSelection` is also in prelude) |
| `Viewport`, `RenderLayers`, `Hdr`, `ScalingMode`, `CameraOutputMode` | `bevy::camera::{Viewport, Hdr, ScalingMode, CameraOutputMode}`, `bevy::camera::visibility::RenderLayers` |
| `Msaa`, `Screenshot`, `save_to_disk` | `bevy::render::view::Msaa`, `bevy::render::view::screenshot::{Screenshot, save_to_disk}` |
| `Tonemapping` | `bevy::core_pipeline::tonemapping::Tonemapping` (re-export; real home `bevy::render::view::Tonemapping`) |
| `Bloom` | `bevy::post_process::bloom::Bloom` |
| `DistanceFog`, `FogFalloff` | `bevy::pbr::{DistanceFog, FogFalloff}` |
| `CascadeShadowConfigBuilder`, `DirectionalLightShadowMap`, `NotShadowCaster`, `NotShadowReceiver` | `bevy::light::*` |
| `Indices`, `PrimitiveTopology`, `MeshTag`, `VertexAttributeValues` | `bevy::mesh::{Indices, PrimitiveTopology, MeshTag, VertexAttributeValues}` |
| `Extent3d`, `TextureDimension`, `TextureFormat`, `AsBindGroup`, `BlendState` | `bevy::render::render_resource::*` |
| `RenderAssetUsages` | `bevy::asset::RenderAssetUsages` |
| `ImageSampler` | `bevy::image::ImageSampler` |
| `ShaderRef` | `bevy::shader::ShaderRef` |
| `Material2d`, `Material2dPlugin`, `ColorMaterial`, `MeshMaterial2d` | `bevy::sprite_render::{Material2d, Material2dPlugin}` (ColorMaterial/MeshMaterial2d/Mesh2d in prelude) |
| `WorldInstanceReady` | `bevy::world_serialization::WorldInstanceReady` |
| `AccumulatedMouseScroll`, `AccumulatedMouseMotion`, `MouseWheel`, `MouseScrollUnit` | `bevy::input::mouse::*` |
| `input_just_pressed` (run condition) | `bevy::input::common_conditions::input_just_pressed` |
| `on_timer` | `bevy::time::common_conditions::on_timer` |
| `DiagnosticsStore`, `FrameTimeDiagnosticsPlugin`, `FrameCount` | `bevy::diagnostic::*` |
| `LogPlugin`, `Level` | `bevy::log::{LogPlugin, Level}` |
| `StatesPlugin` (needed with MinimalPlugins) | `bevy::state::app::StatesPlugin` |
| Error handlers / severity | `bevy::ecs::error::{warn, error, ignore, ResultSeverityExt, Severity}` |
| `IsResource` | `bevy::ecs::resource::IsResource` |
| `TransformSystems::Propagate` (system set) | prelude |

---

## 3. App, DefaultPlugins, WindowPlugin, plugin structure [C][R]

```rust
use bevy::{prelude::*, window::{PresentMode, WindowResolution}};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Emergence Lab".into(),
                resolution: WindowResolution::new(1600, 900), // PHYSICAL px, u32 (also (1600, 900).into())
                present_mode: PresentMode::AutoVsync,          // AutoNoVsync / Immediate / Mailbox / Fifo
                resizable: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.08)))
        .add_plugins(SimPlugin)
        .run();
}

pub struct SimPlugin;
impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SimSettings>()
            .add_message::<GenerationAdvanced>()
            .add_systems(Startup, setup)
            .add_systems(Update, toggle_fullscreen);
    }
}
```
- Multiple plugin configs: `DefaultPlugins.set(WindowPlugin{..}).set(ImagePlugin::default_nearest()).set(AssetPlugin{..})`.
- Other `Window` fields [S]: `mode`, `position`, `resize_constraints`, `decorations`, `transparent`, `focused`,
  `window_level`, `visible`, `window_theme`, `enabled_buttons`, `composite_alpha_mode`, `desired_maximum_frame_latency`.
  Readers: `width()`, `height()`, `size()` (logical), `physical_width()`, `physical_size()`, `scale_factor()`,
  `cursor_position()` (logical, origin **top-left**, `None` outside window), `physical_cursor_position()`.
- Cursor visibility/grab is a **separate component** `CursorOptions { visible, grab_mode, hit_test }` on the
  window entity (`Single<&mut CursorOptions>`). [S]
- macOS: app activation follows `Window::focused` (default `true`). [M]
- `LogPlugin { filter: "warn,emergence_lab=debug,wgpu=error".into(), level: Level::INFO, ..default() }`. [C]

### 3.1 Fullscreen toggle at runtime [C]
```rust
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};

fn toggle_fullscreen(keys: Res<ButtonInput<KeyCode>>, mut window: Single<&mut Window, With<PrimaryWindow>>) {
    let cmd = keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight]);
    if keys.just_pressed(KeyCode::F11) || (cmd && keys.just_pressed(KeyCode::KeyF)) {
        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
}
```
Exclusive fullscreen: `WindowMode::Fullscreen(MonitorSelection::Current, VideoModeSelection::Current)` (falls back to
borderless if unsupported) [S]. Maximize/minimize: `window.set_maximized(true)`, `set_minimized(true)` [S].

---

## 4. Schedules, time, fixed timestep [C][R]

- Schedules: `PreStartup`, `Startup`, `PostStartup`, `First`, `PreUpdate`, `StateTransition`, `RunFixedMainLoop`
  (→ `FixedFirst`, `FixedPreUpdate`, `FixedUpdate`, `FixedPostUpdate`, `FixedLast`), `Update`, `PostUpdate`, `Last`.
- `Time<Fixed>` default timestep = 15 625 µs = **64 Hz** [S]. Set it:
```rust
app.insert_resource(Time::<Fixed>::from_hz(60.0));          // or from_seconds(0.5) / from_duration(..)
fn change_rate(mut fixed: ResMut<Time<Fixed>>) { fixed.set_timestep_hz(240.0); } // also set_timestep_seconds / set_timestep(Duration)
```
- Inside `FixedUpdate`, `Res<Time>` is the fixed clock (`delta_secs()` = timestep). `Res<Time<Fixed>>::overstep_fraction()`
  for render interpolation.
- Pause/speed of *all* game time: `ResMut<Time<Virtual>>` → `pause()`, `unpause()`, `is_paused()`,
  `set_relative_speed(2.0)`, `set_max_delta(Duration)`. Pausing virtual time also stops `FixedUpdate`.
- `time.delta_secs()`, `time.elapsed_secs()`, `time.delta()` (Duration). (`delta_seconds()` no longer exists.)
- Timer [C]: `Timer::from_seconds(0.5, TimerMode::Repeating)`; `timer.tick(time.delta()).just_finished()`.
- Run conditions [C]: `.run_if(in_state(S::A))`, `.run_if(on_timer(Duration::from_secs(1)))`,
  `.run_if(input_just_pressed(KeyCode::KeyR))`, `.run_if(resource_changed::<Speed>)`, `.run_if(resource_exists::<T>)`,
  `.run_if(not(cond))`, `.run_if(run_once)`, `any_with_component::<T>`, `on_message::<M>`.
- System sets [C]:
```rust
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
enum SimSet { Input, Step }
app.configure_sets(FixedUpdate, (SimSet::Input, SimSet::Step).chain())
   .add_systems(FixedUpdate, step_simulation.in_set(SimSet::Step).run_if(in_state(SimState::Playing)));
```
- Note [M]: several *built-in* schedules now use weak ordering (`chain_weak`); only matters if you relied on
  untracked side-channel ordering inside Bevy's render/UI sets.
- `bevy_egui`'s `EguiPrimaryContextPass` schedule runs **inside `PostUpdate`** (see §21.2).

---

## 5. States, sub-states, computed states [C][R]

```rust
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum AppState { #[default] Menu, Running }

#[derive(SubStates, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[source(AppState = AppState::Running)]          // exists only while AppState::Running
enum SimState { #[default] Playing, Paused }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct InSimulation;
impl ComputedStates for InSimulation {
    type SourceStates = AppState;                 // tuple / Option<..> allowed, e.g. (A, Option<B>)
    fn compute(sources: AppState) -> Option<Self> {
        match sources { AppState::Running => Some(InSimulation), _ => None }
    }
}

app.init_state::<AppState>()                      // or .insert_state(AppState::Running)
   .add_sub_state::<SimState>()
   .add_computed_state::<InSimulation>()
   .add_systems(OnEnter(AppState::Running), on_enter_running)
   .add_systems(OnExit(AppState::Running), on_exit_running)
   .add_systems(Update, toggle_pause.run_if(in_state(AppState::Running)));

fn toggle_pause(keys: Res<ButtonInput<KeyCode>>, state: Res<State<SimState>>, mut next: ResMut<NextState<SimState>>) {
    if keys.just_pressed(KeyCode::Space) {
        next.set(match state.get() { SimState::Playing => SimState::Paused, SimState::Paused => SimState::Playing });
    }
}
// State-scoped entities (always enabled, no opt-in needed in 0.20):
commands.spawn((Cell, DespawnOnExit(AppState::Running)));   // also DespawnOnEnter(state)
```
Runtime facts [R]: `DespawnOnExit(S::A)` entity is gone after A→B. `next.set(same_state)` **re-runs**
`OnExit`/`OnEnter` (same-state transition); `next.set_if_different(same_state)` does not.
`NextState::set_if_neq` was renamed `set_if_different` (old name deprecated) [M].
Observers can have run conditions: `app.add_observer(on_click.run_if(in_state(AppState::Menu)))` [S].
`DefaultPlugins` includes `StatesPlugin`; with `MinimalPlugins` add `bevy::state::app::StatesPlugin` [R].

---

## 6. ECS: resources, queries, params, errors [C][R]

```rust
#[derive(Resource, Default)] struct SimSettings { ticks_per_second: f64, generation: u64 }
#[derive(Component)] struct Cell;
#[derive(Component, Default)] struct Velocity(Vec3);

fn param_set_demo(mut set: ParamSet<(Query<&mut Transform, With<Agent>>, Query<&mut Transform, With<Cell>>)>) {
    for mut t in set.p0().iter_mut() { t.translation.x += 0.0; }
    for mut t in set.p1().iter_mut() { t.translation.y += 0.0; }
}

fn single_demo(
    window: Single<&Window, With<PrimaryWindow>>,           // skipped (not panicking) unless exactly 1 match
    camera: Single<(&Camera, &GlobalTransform)>,
    mut counter: Local<u32>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    agents: Query<(Entity, &Velocity), Added<Agent>>,
) {
    *counter += 1;
    let (_cam, _gt) = *camera;                               // mutable Single: camera.into_inner()
    for (e, _v) in &agents {
        let _n = children.get(e).map(|c| c.len()).unwrap_or(0);
        let _p = parents.get(e).map(|c| c.parent()).ok();
        for _d in children.iter_descendants(e) {}
    }
}
```
- `Single<D, F>`: system is **skipped** if 0 or >1 matches; use `Option<Single<D, F>>` for 0-or-1. [S]
- `Query::single()` / `single_mut()` return `Result` (`get_single` is gone). `get(e)`, `get_mut(e)`, `contains(e)`,
  `iter_many(..)` (items are now `Result`, use `.matched()` for the old behavior [M]), `par_iter_mut()`.
- Change detection [C]: `Changed<T>`, `Added<T>`, `Ref<T>` (`.is_added()`, `.is_changed()`),
  `RemovedComponents<T>` (`.read()`), `Res::is_changed()`, `Mut::set_if_neq(v)`.
- **Systems may return `Result`** (`bevy::prelude::Result` = `Result<(), BevyError>`, `?` works on most errors).
  **[R] By default an unannotated `Err` PANICS the app** (errors default to `Severity::Panic`). Options:
  ```rust
  App::new().set_error_handler(bevy::ecs::error::warn)  // call once, before add_plugins
  let _m = q.single().with_severity(Severity::Warning)?; // per-error (ResultSeverityExt)
  ```
  This also applies to `EguiContexts::ctx_mut()?` failing.
- **[R] Resources are entities in 0.20**: an unfiltered `Query<Entity>` (or `Query<EntityRef>`) iterates every
  resource entity too (test: 21 entities in a fresh MinimalPlugins world, 19 of them `IsResource`). Always filter
  (`With<MyComponent>`) or add `Without<IsResource>` (`bevy::ecs::resource::IsResource`).
- `&mut World` is now an ordinary system param and may appear anywhere in the param list (exclusive systems) [M].

---

## 7. Commands, bundles, required components, hierarchy, despawn [C][R]

```rust
#[derive(Component)]
#[require(Transform, Visibility, Velocity)]   // inserted with Default (or `Velocity = expr`) if absent
struct Agent;

commands.spawn((Agent, Name::new("agent-0")));             // tuple = bundle; no *Bundle structs
let parent = commands.spawn((
    Transform::default(), Visibility::default(),
    children![(Cell, Transform::from_xyz(1.0, 0.0, 0.0)), (Cell, Transform::from_xyz(-1.0, 0.0, 0.0))],
)).id();
commands.entity(parent).with_children(|p| { p.spawn((Cell, Transform::default())); });
commands.entity(parent).add_child(orphan);                  // also add_children(&[..]), with_child(bundle)
commands.spawn((Cell, ChildOf(parent)));                    // relationship component (was `Parent`)
commands.entity(parent).despawn();                          // RECURSIVE (despawns Children); no despawn_recursive()
commands.entity(parent).despawn_children();                 // children only
commands.entity(parent).detach_children(&[child]);          // un-parent without despawn
commands.entity(e).insert(X).remove::<Y>().try_insert(Z);   // insert_if_new, insert_if, retain::<B>()
```
Runtime facts [R]: despawning the root of a 4-entity tree removes all 4. Required components are transitive
(`#[require(Transform)]` also yields `GlobalTransform`). Also available [S]: `Children::spawn((Spawn(b), SpawnWith(|s| ..)))`,
`SpawnIter(iter)`, `related!(Children[..])`. BSN (`bsn!{}` + `commands.spawn_scene(..)`) exists in 0.20 but is
optional; plain tuples are fine.

---

## 8. Messages vs observer Events [C][R]

0.17+ split: **`Message`** = buffered, pulled by systems (old `Event`+`EventReader/EventWriter`); **`Event`** = pushed
to observers immediately (old `Trigger`).
```rust
#[derive(Message, Debug, Clone)] struct GenerationAdvanced(u64);
app.add_message::<GenerationAdvanced>();                      // required registration
fn writer(mut w: MessageWriter<GenerationAdvanced>) { w.write(GenerationAdvanced(1)); } // write_default()
fn reader(mut r: MessageReader<GenerationAdvanced>) { for m in r.read() { let _ = m.0; } }

#[derive(Event, Debug)] struct ResetWorld;                    // global event
#[derive(EntityEvent, Debug)] struct Selected { entity: Entity }  // targeted: field named `entity` (or #[event_target])

app.add_observer(|_ev: On<ResetWorld>, mut commands: Commands, cells: Query<Entity, With<Cell>>| {
    for e in &cells { commands.entity(e).despawn(); }
});
commands.trigger(ResetWorld);                                 // world.trigger(..) in exclusive code
commands.spawn(Cell).observe(|ev: On<Selected>, mut commands: Commands| {
    commands.entity(ev.entity).insert(Name::new("selected"));
});
commands.trigger(Selected { entity: e });                     // runs entity observers + global observers

// Lifecycle observers — 0.20 syntax:
fn on_agent_added(ev: On<Add<Agent>>, mut commands: Commands) { commands.trigger(Selected { entity: ev.entity }); }
// lifecycle event types in bevy_ecs-0.20.1/src/lifecycle.rs [S]: Add<B>, Insert<B>, Discard<B>, Remove<B>, Despawn<B>
// (there is no `Replace` type any more: `Discard` carries #[doc(alias = "Replace", "OnReplace")], i.e. it is the old Replace)
```
- Propagating entity events: `#[derive(EntityEvent)] #[entity_event(propagate, auto_propagate)]`, `ev.propagate(false)` [S].
- Window/input "events" (`MouseWheel`, `KeyboardInput`, `CursorMoved`, `WindowResized`, `AppExit`, `AssetEvent<T>`)
  are **Messages**: read with `MessageReader`, write `AppExit` with `MessageWriter<AppExit>` → `.write(AppExit::Success)` [C][R].

---

## 9. Cameras

### 9.1 Camera2d, orthographic zoom/pan [C][R]
`Camera2d` requires `Camera` + `Projection::Orthographic(OrthographicProjection::default_2d())`. Projection is the
`Projection` **enum component** (`Perspective`, `Orthographic`, `Custom`); there is no `OrthographicProjection` component.
```rust
commands.spawn((Camera2d, WorldCamera));

fn camera_zoom_pan(
    scroll: Res<AccumulatedMouseScroll>, motion: Res<AccumulatedMouseMotion>,
    buttons: Res<ButtonInput<MouseButton>>,
    camera: Single<(&mut Transform, &mut Projection), With<WorldCamera>>,
) {
    let (mut transform, mut projection) = camera.into_inner();
    let Projection::Orthographic(ortho) = &mut *projection else { return };
    let dy = match scroll.unit { MouseScrollUnit::Line => scroll.delta.y, MouseScrollUnit::Pixel => scroll.delta.y / 40.0 };
    if dy != 0.0 { ortho.scale = (ortho.scale * (1.0 - dy * 0.1)).clamp(0.05, 50.0); }
    if buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle) {
        let d = motion.delta * ortho.scale;           // screen y down, world y up
        transform.translation.x -= d.x;
        transform.translation.y += d.y;
    }
}
```
`OrthographicProjection { scaling_mode: ScalingMode::FixedVertical { viewport_height: 5.0 }, ..OrthographicProjection::default_3d() }`
for ortho 3D (`bevy::camera::ScalingMode`). `AccumulatedMouseScroll::to_lines(&MouseScrollPixelsPerLine)` also exists [S].

### 9.2 Cursor → world [C][R]
```rust
let Some(cursor) = window.cursor_position() else { return };            // logical px, top-left origin
let Ok(world2d) = camera.viewport_to_world_2d(cam_gt, cursor) else { return };   // 2D
let Ok(ray)     = camera.viewport_to_world(cam_gt, cursor) else { return };      // 3D: Ray3d
if let Some(d) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) { let p = ray.get_point(d); }
let screen = camera.world_to_viewport(cam_gt, world_pos);               // Result<Vec2, _>
```
Pass the **window** cursor position even when the camera has a `Viewport`: `viewport_to_ndc` subtracts the viewport
origin internally [S]. Run in `PostUpdate.after(TransformSystems::Propagate)` if you need this frame's camera transform.

### 9.3 Camera3d, perspective, HDR, MSAA [C][R]
```rust
commands.spawn((
    Camera3d::default(),                              // requires Camera + Projection (Perspective default)
    Projection::Perspective(PerspectiveProjection { fov: 60f32.to_radians(), near: 0.1, far: 2000.0, ..default() }),
    Transform::from_xyz(30.0, 25.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y),
    Hdr,                                              // marker component (bevy::camera::Hdr); Camera.hdr is gone
    Tonemapping::TonyMcMapface,
    Bloom::NATURAL,                                   // Bloom #[require(Hdr)]
    Msaa::Sample4,                                    // per-camera component
));
```
`Camera` fields [S]: `viewport`, `order`, `is_active`, `output_mode`, `msaa_writeback`, `clear_color`,
`invert_culling`, `sub_camera_view`, `computed`. **The render target is a separate `RenderTarget` component**
(`RenderTarget::Window(WindowRef::Primary)` default, `RenderTarget::Image(..)`).

### 9.4 Multiple cameras / viewports / layers / ClearColor [C][R]
```rust
commands.spawn((
    Camera2d, MinimapCamera,
    Camera {
        order: 1,                                     // higher renders later (on top)
        clear_color: ClearColorConfig::None,          // or ::Custom(color) / ::Default (= ClearColor resource)
        viewport: Some(Viewport { physical_position: UVec2::new(16, 16), physical_size: UVec2::new(256, 256), ..default() }),
        ..default()
    },
    Projection::Orthographic(OrthographicProjection { scale: 4.0, ..OrthographicProjection::default_2d() }),
    RenderLayers::layer(1),                           // sees only layer-1 entities (entities default to layer 0)
));
app.insert_resource(ClearColor(Color::BLACK));
```
`Viewport` is in **physical** pixels; rebuild it on `WindowResized`/scale changes and never exceed the window's
physical size (wgpu validation error). Gizmos default to layer 0 (not drawn by a layer-1-only camera) [R].
`Camera2d` defaults to `Tonemapping::Linear`; `Tonemapping::None` is now a full passthrough [M].

---

## 10. Dynamic textures (`Image`) — the Life grid fast path [C][R]

```rust
use bevy::{asset::RenderAssetUsages, image::ImageSampler,
           render::render_resource::{Extent3d, TextureDimension, TextureFormat}};

let mut image = Image::new_fill(
    Extent3d { width: GRID_W, height: GRID_H, depth_or_array_layers: 1 },
    TextureDimension::D2,
    &[0, 0, 0, 255],                                  // one pixel, repeated
    TextureFormat::Rgba8UnormSrgb,
    RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD, // keep CPU copy so we can mutate it
);
image.sampler = ImageSampler::nearest();               // crisp pixels; or globally ImagePlugin::default_nearest()
let handle = images.add(image);                        // images: ResMut<Assets<Image>>

// every frame (or when the sim changed):
fn upload(tex: Res<LifeTexture>, mut images: ResMut<Assets<Image>>) {
    let Some(mut image) = images.get_mut(&tex.handle) else { return };   // AssetMut guard -> `let mut`
    let Some(data) = image.data.as_mut() else { return };               // data: Option<Vec<u8>>
    for (px, &alive) in data.chunks_exact_mut(4).zip(tex.cells.iter()) {
        let v = if alive != 0 { 255 } else { 0 };
        px.copy_from_slice(&[v, v, v, 255]);
    }
}
```
- `Assets::get_mut` returns `AssetMut<'_, Image>`; dropping it marks the asset modified → the **whole** texture is
  re-uploaded that frame [S][R]. `get_mut_untracked` mutates without triggering an upload.
- Other constructors: `Image::new(size, dim, Vec<u8>, format, usage)`, `Image::new_uninit(..)`, `Image::new_target_texture(..)`
  (render target). Helpers: `pixel_bytes_mut(UVec3)`, `set_color_at(x, y, Color)`, `get_color_at`, `width()`, `size()`.
- Row 0 of `data` is the **top** row of the sprite; UV v=0 is at the top [R].
- Bandwidth tip: a 4096² RGBA8 grid is 64 MiB per upload. Prefer `TextureFormat::R8Unorm` (1 B/cell) + a custom
  material that colorizes (§13), or only upload when the generation changed (guard with `is_changed()`).
  R8Unorm usage itself is standard wgpu but was **not runtime-tested here (UNVERIFIED)**.
- Bevy ships a GPU-compute Game of Life example: `examples/shader/compute_shader_game_of_life.rs` [S].

## 11. Sprite [C][R]
```rust
commands.spawn((
    Sprite { image: handle, custom_size: Some(Vec2::new(1024.0, 1024.0)), ..default() },
    Transform::from_xyz(0.0, 0.0, 0.0),
));
commands.spawn(Sprite::from_image(handle));
commands.spawn(Sprite::from_color(Color::srgb(1.0, 0.0, 0.0), Vec2::splat(8.0)));
```
`Sprite` fields [S]: `image`, `texture_atlas`, `color`, `flip_x`, `flip_y`, `custom_size`, `rect`, `image_mode`,
`alpha_mode: SpriteAlphaMode` (new; default Blend). Sprites now render through Mesh2d/Material2d; always give
overlapping sprites distinct z [M].

---

## 12. Meshes, StandardMaterial, lights, fog, post-processing [C][R]

### 12.1 Building a mesh [C]
```rust
use bevy::{asset::RenderAssetUsages, mesh::{Indices, PrimitiveTopology}};

let mut mesh = Mesh::new(PrimitiveTopology::TriangleList,
                         RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);   // Vec<[f32; 3]>
mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);       // Vec<[f32; 3]>
mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);             // Vec<[f32; 2]>
mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);         // Vec<[f32; 4]> linear RGBA (multiplies base_color)
mesh.insert_indices(Indices::U32(indices));                   // CCW = front face
mesh.compute_smooth_normals();                                // or compute_normals / compute_flat_normals
let handle = meshes.add(mesh);
```
Builder forms: `.with_inserted_attribute(..)`, `.with_inserted_indices(..)`, `.with_computed_normals()`.
`insert_attribute` **panics** if the mesh data was already extracted to the render world (i.e. created with
`RENDER_WORLD` only); use `try_insert_attribute` or keep `MAIN_WORLD` [S]. `Aabb` is recomputed automatically when the
`Mesh` asset changes (`AssetChanged<Mesh3d>`), unless `NoAutoAabb`/`NoFrustumCulling` is present [S].
Primitives: `meshes.add(Cuboid::new(1.,1.,1.))`, `Sphere::new(0.5).mesh().uv(32, 18)`, `Sphere::default().mesh().ico(5)`,
`Plane3d::default().mesh().size(10., 10.).subdivisions(10)`, `Cylinder`, `Capsule3d`, `Torus`, `Cone`; 2D: `Rectangle`, `Circle`, `RegularPolygon` [C][S].

### 12.2 Spawning + StandardMaterial [C][R]
```rust
commands.spawn((
    Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
    MeshMaterial3d(materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.3, 0.2),
        perceptual_roughness: 0.6,
        metallic: 0.0,
        ..default()
    })),
    Transform::from_xyz(0.0, 0.5, 0.0),
));
// emissive (HDR, glows with Bloom):
StandardMaterial { base_color: Color::BLACK, emissive: LinearRgba::rgb(8.0, 4.0, 0.5), ..default() }
// transparent + unlit:
StandardMaterial { base_color: Color::srgba(0.2, 0.4, 1.0, 0.4), alpha_mode: AlphaMode::Blend,
                   unlit: true, double_sided: true, cull_mode: None, ..default() }
materials.add(Color::srgb(0.3, 0.5, 0.3));   // From<Color> shortcut
```
Other fields [S]: `base_color_texture`, `emissive_texture`, `emissive_exposure_weight`, `metallic_roughness_texture`,
`normal_map_texture`, `occlusion_texture`, `reflectance`, `specular_tint`, `specular_transmission`,
`diffuse_transmission`, `thickness`, `ior`, `clearcoat*`, `anisotropy*`, `fog_enabled`, `depth_bias`, `uv_transform`
(`Affine2`), `opaque_render_method`. `AlphaMode::{Opaque, Mask(f32), Blend, Premultiplied, Add, Multiply, AlphaToCoverage}`.
Screen-space transmission is now opt-in (`ScreenSpaceTransmission` on the camera) [M].
2D: `Mesh2d(handle)` + `MeshMaterial2d(color_materials.add(ColorMaterial::from_color(c)))` [S].

### 12.3 Lights, shadows, cascades, ambient [C][R]
```rust
app.insert_resource(DirectionalLightShadowMap { size: 4096 })
   .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 300.0, ..default() }); // RESOURCE

commands.spawn((
    DirectionalLight { illuminance: light_consts::lux::AMBIENT_DAYLIGHT, shadow_maps_enabled: true, ..default() },
    Transform::from_rotation(Quat::from_euler(EulerRot::ZYX, 0.0, 1.0, -0.8)),
    CascadeShadowConfigBuilder { num_cascades: 4, first_cascade_far_bound: 20.0, maximum_distance: 300.0, ..default() }.build(),
));
commands.spawn((PointLight { shadow_maps_enabled: true, intensity: 10_000_000., range: 100.0, ..default() },
                Transform::from_xyz(8.0, 16.0, 8.0)));
```
- **`shadows_enabled` is now `shadow_maps_enabled`** (Directional/Point/Spot); there is also `contact_shadows_enabled`,
  `shadow_depth_bias`, `shadow_normal_bias`, `soft_shadow_size` [S].
- **Ambient**: scene-wide = `GlobalAmbientLight` resource; `AmbientLight` is now a **component** (`#[require(Camera)]`)
  that overrides ambient per camera [S].
- `CascadeShadowConfigBuilder` fields: `num_cascades`, `minimum_distance`, `maximum_distance`,
  `first_cascade_far_bound`, `overlap_proportion` [S]. Exclude: `NotShadowCaster`, `NotShadowReceiver`.

### 12.4 Fog, bloom, tonemapping [C][R]
```rust
DistanceFog {                                   // component on the camera
    color: Color::srgba(0.6, 0.7, 0.8, 1.0),
    directional_light_color: Color::srgba(1.0, 0.95, 0.85, 0.5),
    directional_light_exponent: 30.0,
    falloff: FogFalloff::from_visibility_colors(400.0, Color::srgb(0.35, 0.5, 0.66), Color::srgb(0.8, 0.844, 1.0)),
}
// or falloff: FogFalloff::Linear { start: 20.0, end: 60.0 } / Exponential { density } / ExponentialSquared { density }
```
`Bloom::NATURAL` (also `bloom.composite_mode = BloomCompositeMode::Additive`), `Tonemapping::{TonyMcMapface,
AgX, AcesFitted, BlenderFilmic, KhronosPbrNeutral, Reinhard, ReinhardLuminance, SomewhatBoringDisplayTransform, Linear, None}` [S]
(LUT tonemappers need `tonemapping_luts`, included by `3d`). Also available: `Atmosphere`, `VolumetricFog`, SSAO,
SSR, DoF, motion blur (see `examples/3d/*.rs`; not verified here).

---

## 13. Custom `Material` with a WESL shader [C][R]

**Bevy 0.20 shaders are WESL** (`.wesl`): the naga_oil `#import` / `#ifdef` / `#{DEF}` dialect is gone. Plain WGSL
without preprocessor directives still works [M]. Material bind group index is **3** (`MATERIAL_BIND_GROUP_INDEX = 3`) [S];
always write `@group(constants::MATERIAL_BIND_GROUP)`.

Rust [C][R]:
```rust
use bevy::{prelude::*, reflect::TypePath, render::render_resource::AsBindGroup, shader::ShaderRef};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CellMaterial {
    #[uniform(0)] pub tint: LinearRgba,
    #[texture(1)] #[sampler(2)] pub cells: Handle<Image>,
}
impl Material for CellMaterial {
    fn fragment_shader() -> ShaderRef { "shaders/cell_material.wesl".into() } // path in the asset folder
    // optional: vertex_shader(), alpha_mode(&self), enable_shadows(), enable_prepass(), specialize(..)
}
app.add_plugins(MaterialPlugin::<CellMaterial>::default());
commands.spawn((Mesh3d(mesh), MeshMaterial3d(cell_mats.add(CellMaterial { tint: LinearRgba::rgb(0.3, 1.0, 0.5), cells: image }))));
```
`assets/shaders/cell_material.wesl` [R]:
```wgsl
import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::globals,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> tint: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var cells_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var cells_sampler: sampler;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(cells_texture, cells_sampler, mesh.uv);
    let pulse = 0.75 + 0.25 * sin(globals.time * 3.0);
    return vec4<f32>(c.rgb * tint.rgb * pulse, 1.0);
}
```
2D variant [C][R]: `impl Material2d for Cell2dMaterial` + `Material2dPlugin::<Cell2dMaterial>::default()`
(`bevy::sprite_render::{Material2d, Material2dPlugin}`), shader imports
`import bevy_sprite_render::mesh2d::vertex_output::VertexOutput;`.

WESL rules [M]: imports end with `;` and come first; module paths follow file paths (`bevy_pbr::forward_io` →
`bevy_pbr::render::forward_io`, `bevy_pbr::mesh_view_bindings` → `bevy_pbr::render::mesh_view_bindings`); relative
asset imports `import super::util::hsv_to_rgb;`; conditional compilation `@if(VERTEX_COLORS)`; integer shader defs via
`constants::NAME`. `VertexOutput.uv` only exists if the mesh has `ATTRIBUTE_UV_0` (`@if(VERTEX_UVS_A)`).
Reference shaders (Bevy repo, tag v0.20.1, `assets/shaders/`): `custom_material.wesl`, `custom_material_2d.wesl`,
`animate_shader.wesl`, `instancing.wesl` — https://github.com/bevyengine/bevy/tree/v0.20.1/assets/shaders
Note: the doc comment in `bevy_pbr-0.20.1/src/material.rs` still shows the old `#{MATERIAL_BIND_GROUP}` syntax — it is stale.

Shipping shaders inside a crate [S]: `bevy::asset::embedded_asset!(app, "cell.wesl")` in `crates/foo/src/render/mod.rs`
→ load as `"embedded://foo/render/cell.wesl"` (`src/` is stripped; the macro searches for `<crate_name>/src` in the
path, so keep the directory name equal to the crate name, with `-`→`_` caveat UNVERIFIED). Embedded WESL is
importable as `foo::render::cell` [M]. Runtime-UNVERIFIED here.

Other examples to copy from [S]: `shader/extended_material.rs` (extend `StandardMaterial`), `shader/animate_shader.rs`,
`shader/storage_buffer.rs`, `shader/array_texture.rs`, `shader/shader_material_2d.rs`.

## 14. Instancing guidance [S][C]
- **Automatic**: entities sharing the *same* `Mesh3d` handle and *same* `MeshMaterial3d` handle are batched /
  GPU-instanced automatically (`examples/shader/automatic_instancing.rs`, stress test `examples/stress_tests/many_cubes.rs`).
  Different material handles (e.g. one `StandardMaterial` per color) break batches — reuse a small palette of handles.
- Per-instance data with automatic instancing: `MeshTag::new(u32)` component (0.20: constructor, field `.value`) read in
  a custom material shader via `mesh_functions::get_tag(instance_index)` (see automatic_instancing example).
- Fully custom instance buffers: `examples/shader_advanced/custom_shader_instancing.rs` (low-level render API).
- For 2D grids, one textured quad (`Sprite` or `Mesh2d`) with a CPU/GPU texture beats per-cell entities by orders of magnitude.

---

## 15. Gizmos [C][R]
```rust
fn draw(mut gizmos: Gizmos) {
    gizmos.grid(Isometry3d::from_rotation(Quat::from_rotation_x(FRAC_PI_2)),   // XZ ground grid
                UVec2::splat(40), Vec2::splat(1.0), Color::srgba(1.0, 1.0, 1.0, 0.08));
    gizmos.line(Vec3::ZERO, Vec3::Y * 5.0, Color::srgb(0.0, 1.0, 0.0));
    gizmos.circle(Isometry3d::new(Vec3::new(0.0, 0.01, 0.0), Quat::from_rotation_x(FRAC_PI_2)), 3.0, Color::WHITE);
    gizmos.sphere(Isometry3d::from_translation(p), 0.3, Color::srgb(1.0, 0.0, 0.0));
    gizmos.arrow(a, b, Color::srgb(1.0, 1.0, 0.0));
    gizmos.cube(Transform::from_xyz(0.0, 1.0, 0.0), Color::srgb(0.0, 0.5, 1.0));
    gizmos.axes(Transform::IDENTITY, 2.0);
    // 2D
    gizmos.grid_2d(Isometry2d::IDENTITY, UVec2::new(16, 16), Vec2::splat(64.0), Color::srgba(1.0, 1.0, 1.0, 0.1)).outer_edges();
    gizmos.line_2d(Vec2::ZERO, Vec2::new(100.0, 0.0), Color::srgb(1.0, 0.0, 0.0));
    gizmos.circle_2d(Isometry2d::from_translation(Vec2::new(50.0, 50.0)), 20.0, Color::WHITE);
    gizmos.rect_2d(Isometry2d::IDENTITY, Vec2::new(200.0, 100.0), Color::srgb(0.0, 1.0, 0.0));
}
// config: let (config, _) = config_store.config_mut::<DefaultGizmoConfigGroup>(); config.line.width = 2.0; config.depth_bias = -1.0;
```
Shapes take an `Isometry2d/3d` (position + rotation), not a bare position. Also `grid_3d`, `linestrip`, `linestrip_2d`,
`linestrip_gradient`, `rect`, `cross`, `ray`, `arrow_2d`. Gizmos are immediate-mode (redraw every frame).

---

## 16. glTF scenes and animation [C][R]

```rust
use bevy::world_serialization::WorldInstanceReady;
const FOX: &str = "models/animated/Fox.glb";

fn setup_fox(mut commands: Commands, asset_server: Res<AssetServer>, mut graphs: ResMut<Assets<AnimationGraph>>) {
    let (graph, nodes) = AnimationGraph::from_clips([
        asset_server.load(GltfAssetLabel::Animation(0).from_asset(FOX)),
        asset_server.load(GltfAssetLabel::Animation(1).from_asset(FOX)),
    ]);                                                       // or AnimationGraph::from_clip(h) -> (graph, index)
    commands.insert_resource(FoxAnimations { graph: graphs.add(graph), nodes });
    commands.spawn((
        WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(FOX))),   // was SceneRoot
        Transform::from_xyz(5.0, 0.0, 5.0).with_scale(Vec3::splat(0.02)),
    )).observe(start_fox_animation);
}

fn start_fox_animation(
    ready: On<WorldInstanceReady>,                            // was SceneInstanceReady
    mut commands: Commands, anims: Res<FoxAnimations>,
    children: Query<&Children>, mut players: Query<&mut AnimationPlayer>,
) {
    for e in children.iter_descendants(ready.entity) {        // AnimationPlayer is on a descendant
        if let Ok(mut player) = players.get_mut(e) {
            let mut transitions = AnimationTransitions::new();
            transitions.play(&mut player, anims.nodes[0], Duration::ZERO).repeat();
            commands.entity(e).insert((AnimationGraphHandle(anims.graph.clone()), transitions));
        }
    }
}
// crossfade later:
transitions.play(&mut player, anims.nodes[1], Duration::from_millis(250)).set_speed(1.5).repeat();
```
- Without transitions: `player.play(index).repeat()`; control: `player.animation_mut(i)` → `pause()`, `resume()`,
  `set_speed()`, `seek_to()`, `is_paused()`; `player.playing_animations()`.
- Whole-file access: `let h: Handle<Gltf> = asset_server.load("x.glb")`; when loaded, `Gltf { scenes, named_scenes,
  default_scene, animations, named_animations, meshes, materials, nodes, .. }` (`named_animations["Run"]`) [S].
- Labels [S]: `GltfAssetLabel::{Scene(i), Node(i), Mesh(i), Primitive{mesh, primitive}, Texture(i), Material{index,
  is_scale_inverted}, DefaultMaterial, Animation(i), Skin(i), InverseBindMatrices(i)}`; `.from_asset(path)`.
- `bevy_scene` in 0.20 is the new BSN scene system (`bsn!`, `spawn_scene`, `On<Ready>`); glTF scenes are
  `WorldAsset`s from `bevy_world_serialization`. Don't mix them up.
- [R] `Fox.glb` (Bevy repo asset, CC0 model + CC-BY 4.0 rigging/animation — attribution required if ever shipped)
  loaded, `AnimationPlayer` found and playing.

---

## 17. Assets: paths and the workspace binary [S][R]

Base path resolution (`bevy_asset-0.20.1/src/io/file/mod.rs`): `BEVY_ASSET_ROOT` env var, else `CARGO_MANIFEST_DIR`
(set by `cargo run`), else the executable's directory. The asset root is `base_path.join(AssetPlugin::file_path)`
(`file_path` default `"assets"`). Verified at runtime:
- `cargo run` with `file_path: "../depcheck/assets"` → loads fine (relative climbing works) [R].
- Binary started directly with `BEVY_ASSET_ROOT=<dir>` → loads `<dir>/assets/...` [R].
- Binary started directly with no env → looks in `<exe dir>/assets/...`, logs `ERROR bevy_asset::server: Path not found: …` [R].

For this repo (`apps/desktop` binary, assets at repo root `assets/`): `cargo run -p emergence-lab` sets
`CARGO_MANIFEST_DIR=apps/desktop`, so the default would look in `apps/desktop/assets` (**wrong**). Use:
```rust
DefaultPlugins.set(AssetPlugin { file_path: "../../assets".into(), ..default() })   // dev via cargo run
```
and for distribution set `BEVY_ASSET_ROOT` or ship `assets/` next to the binary (and switch `file_path` back), or
embed (§13). A `.cargo/config.toml` `[env] BEVY_ASSET_ROOT = { value = ".", relative = true }` is a Cargo feature
that should also work (UNVERIFIED here). `AssetPlugin` also has `processed_file_path`, `watch_for_changes_override`,
`mode`, `meta_check`, `unapproved_path_mode` (default `Forbid` for paths escaping the root via `..` in load requests).
`asset_server.load("shaders/x.wesl")` paths are relative to the asset root; labels via `#` / `GltfAssetLabel`.
`asset_server.is_loaded_with_dependencies(&handle)`; `MessageReader<AssetEvent<Image>>` for load/modify events.
`AssetId::invalid()` is deprecated; use `Option<AssetId<_>>` [M].

---

## 18. Picking and ray casting [C][R-render, click not simulated]

```rust
app.add_plugins((DefaultPlugins, MeshPickingPlugin));        // MeshPickingPlugin is NOT in DefaultPlugins
                                                             // (DefaultPickingPlugins + sprite picking are)
commands.spawn((Mesh3d(m), MeshMaterial3d(mat), Transform::default()))
    .observe(|click: On<PointerClick>| {                     // 0.20: flattened events, not On<Pointer<Click>>
        if click.button == PointerButton::Primary { info!("hit at {:?}", click.hit.position); }
    });
app.add_observer(on_any_click);                              // global: every entity's PointerClick
fn on_any_click(click: On<PointerClick>, names: Query<&Name>) {
    let _ = (click.entity, click.button, click.pointer.position, click.hit.normal, click.duration, click.count);
}
commands.spawn((Mesh3d(ground), MeshMaterial3d(mat), Pickable::IGNORE));   // not pickable, doesn't block
```
Event types [S]: `PointerOver`, `PointerOut`, `PointerEnter`, `PointerLeave`, `PointerPress`, `PointerRelease`,
`PointerClick`, `PointerMove` (`delta`), `PointerDragStart`, `PointerDrag` (`delta`, `distance`), `PointerDragEnd`,
`PointerDragEnter/Over/Leave/Drop`, `PointerScroll`, `PointerCancel`. Each has `entity`, `pointer: Pointer { id,
target, position: Vec2 }`, `hit: HitData { camera, depth, position: Option<Vec3>, normal: Option<Vec3> }`.
Generic handlers: `fn h<E: EntityEvent>(ev: On<E>)` or `E: PointerEvent` [M]. `MeshPickingSettings { require_markers,
ray_cast_visibility }` resource; `MeshPickingCamera` marker when `require_markers` [S].

Manual ray cast (no plugin needed for the `MeshRayCast` param) [C]:
```rust
fn ray_cast_from_cursor(
    window: Single<&Window, With<PrimaryWindow>>, camera: Single<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut ray_cast: MeshRayCast, terrain: Query<(), With<Terrain>>, mut gizmos: Gizmos,
) {
    let (camera, cam_gt) = *camera;
    let Some(cursor) = window.cursor_position() else { return };
    let Ok(ray) = camera.viewport_to_world(cam_gt, cursor) else { return };
    let only_terrain = |e: Entity| terrain.contains(e);
    let settings = MeshRayCastSettings::default().with_filter(&only_terrain); // with_visibility, with_early_exit_test, always_early_exit
    if let Some((_entity, hit)) = ray_cast.cast_ray(ray, &settings).first() {
        gizmos.sphere(Isometry3d::from_translation(hit.point), 0.3, Color::srgb(1.0, 0.0, 0.0)); // RayMeshHit { point, normal, distance, uv, triangle, .. }
    }
}
```
Picking respects camera viewports (pointer outside a camera's viewport produces no ray for it) [S].

## 19. Input [C]
```rust
fn kb(keys: Res<ButtonInput<KeyCode>>, mouse: Res<ButtonInput<MouseButton>>) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl  = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let cmd   = keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight]);   // macOS Command
    let alt   = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if (ctrl || cmd) && shift && keys.just_pressed(KeyCode::KeyZ) { /* redo */ }
    let _ = (alt, mouse.just_released(MouseButton::Left), keys.pressed(KeyCode::ArrowUp));
}
fn wheel(mut wheel: MessageReader<MouseWheel>) { for ev in wheel.read() { let _ = (ev.unit, ev.x, ev.y, ev.window); } }
```
Prefer the per-frame resources `AccumulatedMouseScroll { unit, delta }` and `AccumulatedMouseMotion { delta }`.
Key names: `KeyCode::KeyA`, `Digit1`, `Space`, `Enter`, `Escape`, `F11`, `Equal`, `Minus`, `ArrowUp`, `SuperLeft`.
Text/character input: `MessageReader<KeyboardInput>` (`bevy::input::keyboard::KeyboardInput`) [S].
Cursor: `window.cursor_position()` (§9.2); `MessageReader<CursorMoved>`.

## 20. Screenshots, diagnostics, transforms, misc

Screenshot [C][R]:
```rust
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
commands.spawn(Screenshot::primary_window()).observe(save_to_disk(format!("screenshot-{n}.png")));
// in-memory: .observe(|ev: On<ScreenshotCaptured>| { let img: &Image = &ev.image; })   (ScreenshotCaptured: [S])
```
Saved PNG is in physical pixels (2560×1440 for a 1280×720 window on Retina) [R]. Also `Screenshot::window(e)`,
`Screenshot::image(handle)`; entity gets `Capturing` while in flight.

Diagnostics / FPS [C][R]:
```rust
app.add_plugins(FrameTimeDiagnosticsPlugin::default());   // + LogDiagnosticsPlugin::default() to log
fn show_fps(diagnostics: Res<DiagnosticsStore>) {
    let fps = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()); // value(), average()
}
```
`FrameTimeDiagnosticsPlugin::{FPS, FRAME_TIME, FRAME_COUNT}`; `Res<FrameCount>` (`.0`). Custom: `DiagnosticPath::const_new("sim/tps")`
+ `app.register_diagnostic(Diagnostic::new(path))` + `Diagnostics` param `add_measurement` (pattern from
`examples/diagnostics/custom_diagnostic.rs`, UNVERIFIED by compile). `RenderDebugOverlay` (F1/F2) exists but needs
`render_dev_tools` [M].

Transform / GlobalTransform [C]:
```rust
Transform::from_xyz(x, y, z).looking_at(target, Vec3::Y).with_scale(Vec3::splat(2.0))
t.rotate_y(angle); t.rotate_around(Vec3::ZERO, Quat::from_rotation_y(0.01)); t.look_at(target, Vec3::Y);
let fwd: Dir3 = t.forward();       // Dir3, deref to Vec3 with *fwd
let world: Vec3 = global_transform.translation(); let local: Transform = global_transform.compute_transform();
```
`GlobalTransform` is computed in `PostUpdate` (`TransformSystems::Propagate`); in `Update` it is last frame's value.
Visibility: `Visibility::{Inherited, Visible, Hidden}`.

---

## 21. bevy_egui 0.43.1 (egui 0.36.2)

### 21.1 Plugin setup [C][R]
```rust
use bevy_egui::{egui, EguiContexts, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass,
                EguiTextureHandle, PrimaryEguiContext, input::{egui_wants_any_pointer_input, EguiWantsInput}};

app.add_plugins(DefaultPlugins)
   .insert_resource(EguiGlobalSettings { auto_create_primary_context: false, ..default() }) // see 21.4
   .add_plugins(EguiPlugin::default())
   .add_systems(EguiPrimaryContextPass, ui_system);
```
- `EguiPlugin` fields [S]: `ui_render_order: UiRenderOrder` (`EguiAboveBevyUi` default / `BevyUiAboveEgui`) and
  `bindless_mode_array_size: Option<NonZero<u32>>`. The old single-pass mode and `enable_multipass_for_primary_context`
  are **gone** (removed in 0.42) [S: CHANGELOG].
- `EguiGlobalSettings` fields [S]: `auto_create_primary_context` (default true), `enable_focused_non_window_context_updates`,
  `input_system_settings`, `enable_absorb_bevy_input_system` (default false), `enable_cursor_icon_updates`, `enable_ime`.
  `init_resource` won't overwrite a resource inserted before the plugin, so inserting it first works [S][R].
- Per-context `EguiContextSettings` component: `run_manually`, `capture_pointer_input` (default true), `enable_ime`, ... [S].

### 21.2 Where UI systems go [S][R]
UI systems go in the **`EguiPrimaryContextPass`** schedule (not `Update`). It is run by
`run_egui_context_pass_loop_system` in `PostUpdate` (`EguiPostUpdateSet::EndPass`) via
`ctx.run_ui(input, |_| world.run_schedule(EguiPrimaryContextPass))`. Consequences: UI runs after `Update`; values the UI
writes into resources are seen by `Update` systems on the next frame; `EguiWantsInput` is written later in PostUpdate.
Startup-time context access: spawn the camera in `PreStartup` with `.before(EguiStartupSet::InitContexts)` (pattern
from `bevy_egui/examples/ui.rs`) or simply configure once inside the pass (`.run_if(run_once)`) [C].

### 21.3 `EguiContexts` [S][C]
- `ctx_mut(&mut self) -> Result<&mut egui::Context, QuerySingleError>`; `ctx() -> Result<&egui::Context, _>`;
  `ctx_for_entity_mut(entity)`. Use in a system returning `Result`: `let ctx = contexts.ctx_mut()?;`
  (failure panics under the default error handler, §6).
- Bevy image → egui: `let id: egui::TextureId = contexts.add_image(EguiTextureHandle::Strong(handle.clone()));`
  (`EguiTextureHandle::Weak(asset_id)` also); idempotent; `contexts.image_id(&handle)`, `contexts.remove_image(&handle)`.
  Render with `ui.image(egui::load::SizedTexture::new(id, [128.0, 128.0]))` or
  `egui::Image::new(egui::load::SizedTexture::new(id, size))`. Updates to the Bevy `Image` show up live [R].
  Since 0.39.1 user textures are **not** re-premultiplied: non-opaque images must contain premultiplied alpha [S: CHANGELOG].

### 21.4 Panels in egui 0.36 (renamed!) and the world viewport [C][R]
egui 0.36 has **no `SidePanel` / `TopBottomPanel`**. Use `egui::Panel::left/right/top/bottom(id)` and
`egui::CentralPanel`, and **`show` takes a parent `&mut Ui`, not a `&Context`** (`show_inside` is a deprecated alias).
`egui::Window::new(..).show(ctx, ..)` still takes the `Context`. Builder methods: `.resizable(bool)`,
`.default_size(f32)`, `.exact_size(f32)`, `.show_separator_line(bool)`, `.frame(Frame)`, `.show_collapsible(..)`.
`Context::screen_rect()`/`available_rect()` are gone → `ctx.viewport_rect()` / `ctx.content_rect()`.

Recommended layout (what `bevy_egui/examples/side_panel.rs` does; [R] in `s07_smoke`): a **dedicated egui camera**
plus a world camera whose `Viewport` is the area left over by the panels.
```rust
// setup: (auto_create_primary_context = false, see 21.1)
commands.spawn((
    PrimaryEguiContext, Camera2d, RenderLayers::none(),
    Camera {
        order: 10,
        output_mode: CameraOutputMode::Write { blend_state: Some(BlendState::ALPHA_BLENDING), clear_color: ClearColorConfig::None },
        clear_color: ClearColorConfig::Custom(Color::NONE),
        ..default()
    },
));

fn ui_system(
    mut contexts: EguiContexts, mut layout: ResMut<UiLayout>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut world_cam: Single<&mut Camera, With<PanOrbitCamera>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut root = egui::Ui::new(ctx.clone(), "root".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()));

    egui::Panel::top("menu").show(&mut root, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {                    // egui::menu::bar is gone
            ui.menu_button("File", |ui| { if ui.button("Quit").clicked() { std::process::exit(0); } });
        });
    });
    egui::Panel::left("controls").resizable(true).default_size(260.0).show(&mut root, |ui| { /* widgets */ });
    egui::Panel::right("plots").resizable(true).default_size(320.0).show(&mut root, |ui| { /* plots */ });
    // NO CentralPanel: it would cover the world and swallow input.

    let central = root.available_rect_before_wrap();        // egui logical px
    layout.central = Some(Rect::new(central.min.x, central.min.y, central.max.x, central.max.y));
    let sf = window.scale_factor();
    let pos  = UVec2::new((central.min.x * sf) as u32, (central.min.y * sf) as u32);
    let size = UVec2::new((central.width() * sf) as u32, (central.height() * sf) as u32);
    let win = window.physical_size();
    if size.x > 0 && size.y > 0 && pos.x + size.x <= win.x && pos.y + size.y <= win.y {
        world_cam.viewport = Some(Viewport { physical_position: pos, physical_size: size, ..default() });
    }
    egui::Window::new("Inspector").show(ctx, |ui| { ui.label("floating"); });
    Ok(())
}
```
Panel order matters: the first panel shown takes the full edge (left-then-bottom ⇒ bottom panel is only right of the
left panel) [R]. If you instead let the egui context live on the world camera (default auto-create), do **not**
shrink that camera's viewport (egui would be clipped to it).

### 21.5 Keeping egui from eating world input — VERIFIED BEHAVIOR (important) [R][S]
`bevy_egui` exposes `Res<EguiWantsInput>` (`bevy_egui::input::EguiWantsInput`: `is_pointer_over_area()`,
`wants_pointer_input()`, `is_using_pointer()`, `wants_keyboard_input()`, `is_popup_open()`,
`wants_any_pointer_input()`, `wants_any_keyboard_input()`, `wants_any_input()`) and run conditions
`egui_wants_any_pointer_input`, `egui_wants_any_keyboard_input`, `egui_wants_any_input`. Its picking integration
(`capture_pointer_input_system`, feature `picking`) blocks Bevy picking when `ctx.egui_wants_pointer_input()`.
egui 0.36 renamed `ctx.wants_pointer_input()` → `egui_wants_pointer_input()`, `is_pointer_over_area()` →
`is_pointer_over_egui()`, `is_using_pointer()` → `egui_is_using_pointer()`, `wants_keyboard_input()` → `egui_wants_keyboard_input()`.

Runtime test (`egui_hover_test.rs`, egui 0.36.2, reproducing bevy_egui's exact call path `ctx.run_ui(input, |_| user_code)`):

| Pointer location | `is_pointer_over_egui()` | `egui_wants_pointer_input()` |
|---|---|---|
| over a `Panel::left` inside the manual root `Ui` | **false** | **false** |
| over empty world area | false | false |
| over an `egui::Window` | true | true |

**So hovering/clicking a side/top/bottom `Panel` is NOT reported** by `EguiWantsInput`, the run conditions, the
picking blocker, or bevy_panorbit_camera's egui integration (it uses the same egui calls). Only `egui::Window`s,
popups, focused text fields and active drags (`egui_is_using_pointer`, e.g. dragging a slider — source-inferred) are.
(Reason [S]: egui's root `Ui` from `run_ui` is ignored by bevy_egui; panels live on the background layer, and
`is_pointer_over_egui` treats background-layer hits inside the run_ui root's available rect as "not egui".)
Mitigations (use both):
1. Shrink the world camera `Viewport` to the central rect (21.4). Bevy picking and PanOrbitCamera only react
   inside a camera's viewport [S], so panels are protected automatically.
2. For custom world-input systems: gate on `layout.central.contains(cursor)` **and**
   `.run_if(not(egui_wants_any_pointer_input))` (windows/drags/popups) and check `wants_any_keyboard_input()`
   before handling hotkeys [C]:
```rust
app.add_systems(Update, world_click_system.run_if(not(egui_wants_any_pointer_input)));
fn world_click_system(layout: Res<UiLayout>, egui_input: Res<EguiWantsInput>,
                      window: Single<&Window, With<PrimaryWindow>>, buttons: Res<ButtonInput<MouseButton>>) {
    let Some(cursor) = window.cursor_position() else { return };
    let in_world = layout.central.is_some_and(|r| r.contains(cursor));
    if in_world && !egui_input.wants_any_keyboard_input() && buttons.just_pressed(MouseButton::Left) { /* ... */ }
}
```
`EguiGlobalSettings::enable_absorb_bevy_input_system = true` makes egui swallow all Bevy input messages while it wants
input (less safe; documented in `examples/absorb_input.rs`) [S].

### 21.6 Theme, style, fonts [C]
```rust
ctx.set_theme(egui::Theme::Dark);
ctx.set_visuals(egui::Visuals {
    panel_fill: egui::Color32::from_rgb(20, 22, 28),
    window_fill: egui::Color32::from_rgb(28, 30, 38),
    window_corner_radius: 4.into(),                  // CornerRadius (was `window_rounding`)
    override_text_color: Some(egui::Color32::from_gray(220)),
    ..egui::Visuals::dark()
});
ctx.all_styles_mut(|style| { style.spacing.item_spacing = egui::vec2(6.0, 4.0); style.spacing.slider_width = 180.0; });
let mut fonts = egui::FontDefinitions::default();
fonts.font_data.insert("inter".to_owned(),
    std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter.ttf")))); // Arc required
fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "inter".to_owned()); // push = fallback
ctx.set_fonts(fonts);                                 // or ctx.add_font(egui::FontInsert::new(..))
ctx.set_zoom_factor(1.0);                             // UI scale (bevy_egui uses native_pixels_per_point)
```
(The `include_bytes!` path/font file is illustrative; the compiled test used `FontData::from_owned(Vec::new())`.)
Other `Visuals` fields [S]: `dark_mode`, `widgets`, `selection`, `hyperlink_color`, `faint_bg_color`, `extreme_bg_color`,
`window_shadow`, `window_stroke`, `menu_corner_radius`, `popup_shadow`, `striped`, `slider_trailing_fill`.
Set visuals once (guard with a flag or `run_once`), not every frame.

### 21.7 Widgets (egui 0.36) [C]
```rust
egui::ScrollArea::vertical().show(ui, |ui| {          // .horizontal()/.both(), .max_height(h), .auto_shrink([false;2]), .stick_to_bottom(true)
    ui.heading("Simulation");
    ui.add(egui::Slider::new(&mut state.speed, 0.0..=240.0).text("ticks/s").logarithmic(true)); // .step_by, .integer(), .suffix
    ui.add(egui::DragValue::new(&mut state.seed).speed(1.0).range(0..=u64::MAX).prefix("seed "));
    egui::ComboBox::from_label("Rule").selected_text(format!("{:?}", state.rule)).show_ui(ui, |ui| {
        ui.selectable_value(&mut state.rule, Rule::Life, "Life B3/S23");
        ui.selectable_value(&mut state.rule, Rule::HighLife, "HighLife B36/S23");
    });
    ui.color_edit_button_srgba(&mut state.color);     // also color_edit_button_rgb(&mut [f32;3]), _srgb, _hsva, _rgba_unmultiplied
    egui::CollapsingHeader::new("Stats").default_open(true).show(ui, |ui| {
        egui::Grid::new("stats").num_columns(2).striped(true).show(ui, |ui| {
            ui.label("Generation"); ui.label("123"); ui.end_row();
        });
    });
    if ui.add(egui::Button::image(egui::Image::new(egui::load::SizedTexture::new(id, [24.0, 24.0])))).clicked() {}
    ui.toggle_value(&mut state.show_window, "Inspector");
    ui.checkbox(&mut b, "label"); ui.radio_value(&mut v, X, "x"); ui.separator();
});
let mut open = state.show_window;
egui::Window::new("Inspector").open(&mut open).default_pos([900.0, 60.0]).show(ctx, |ui| { /* .. */ });
state.show_window = open;
```
`egui::ImageButton` no longer exists → `egui::Button::image(..)` / `Button::image_and_text(..)` [S].
`ui.collapsing("title", |ui| ..)`, `ui.horizontal(|ui| ..)`, `ui.add_enabled(bool, widget)`, `ui.add_sized(size, widget)` [S].

## 22. egui_plot 0.37 [C][R]
```rust
use egui_plot::{Bar, BarChart, Corner, Legend, Line, Plot, PlotPoints, Points};

Plot::new("population").height(200.0).legend(Legend::default().position(Corner::LeftTop))
    .show(ui, |plot_ui| {
        plot_ui.line(Line::new("alive", PlotPoints::from(history.clone())));          // NAME FIRST, then data
        plot_ui.points(Points::new("births", vec![[0.0, 1.0], [1.0, 2.0]]).radius(3.0));
    });
let bars: Vec<Bar> = (0..8).map(|i| Bar::new(i as f64, (i * i) as f64).width(0.8)).collect();
Plot::new("histogram").height(160.0).show(ui, |plot_ui| plot_ui.bar_chart(BarChart::new("ages", bars)));
```
- `PlotPoints`: enum `Owned(Vec<PlotPoint>)` / `Generator(..)` / `Borrowed(&[PlotPoint])`; `From<Vec<[f64;2]>>`,
  `FromIterator<[f64;2]>`, `PlotPoints::new(vec)`, `from_ys_f32(&[f32])`, `from_ys_f64`, `from_explicit_callback(..)`.
  For long histories avoid cloning every frame: keep a `Vec<PlotPoint>` and pass `PlotPoints::Borrowed(&v)` (UNVERIFIED by compile).
- `Line`: `.color(c)`, `.width(w)`, `.style(LineStyle::..)`, `.fill(y_ref)`, `.fill_alpha(a)`, `.name(..)`.
  `BarChart::new(name, Vec<Bar>)`: `.color`, `.width`, `.vertical()/.horizontal()`, `.stack_on(&[&other])`.
  `Bar::new(argument, height)`: `.fill`, `.stroke`, `.width`, `.name`.
- `Plot`: `.width/.height/.view_aspect/.data_aspect`, `.allow_zoom/.allow_drag/.allow_scroll(bool)`,
  `.include_x/.include_y`, `.default_x_bounds(min,max)`, `.auto_bounds(..)`, `.x_axis_label/.y_axis_label`,
  `.show_axes/.show_grid`, `.link_axis(id, ..)`, `.legend(Legend)`. Also `HLine`, `VLine`, `Polygon`, `Text`,
  `PlotImage`, `BoxPlot`, `Heatmap`, `Arrows`, `FilledArea`, `Span` items. `Corner` lives in `egui_plot::Corner` (root export).

## 23. bevy_panorbit_camera 0.36 [C][R][S]
```rust
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraPlugin};
app.add_plugins(PanOrbitCameraPlugin);
commands.spawn((
    PanOrbitCamera {                                  // #[require(Camera3d)]: no need to add Camera3d
        focus: Vec3::ZERO, radius: Some(30.0), yaw: Some(0.6), pitch: Some(0.5),
        button_orbit: MouseButton::Left, button_pan: MouseButton::Right,
        zoom_lower_limit: 2.0, zoom_upper_limit: Some(500.0),
        ..default()
    },
    Transform::from_xyz(0.0, 15.0, 30.0),
));
fn refocus(mut cam: Single<&mut PanOrbitCamera>) { cam.target_focus = Vec3::new(10.0, 0.0, 10.0); } // smooth move
```
- Fields [S]: `focus`, `radius`, `yaw`, `pitch` (current; Option ones are initial values), **`target_focus`,
  `target_yaw`, `target_pitch`, `target_radius`** (set these to move the camera programmatically), limits
  (`yaw_upper/lower_limit`, `pitch_upper/lower_limit`, `zoom_upper_limit`, `zoom_lower_limit`, `focus_bounds_origin`,
  `focus_bounds_shape`), sensitivities/smoothness (`orbit_*`, `pan_*`, `zoom_*`), `button_orbit`, `button_pan`,
  `button_zoom`, `modifier_orbit`, `modifier_pan`, `touch_enabled`, `trackpad_behavior`, `reversed_zoom`,
  `allow_upside_down`, **`enabled`** (set false to disable input), `force_update`, `axis`, `use_real_time`.
- Runs in `PostUpdate` in `PanOrbitCameraSystemSet`, before `TransformSystems::Propagate` and `CameraUpdateSystems` [S].
- Input only goes to the camera whose `Camera::logical_viewport_rect()` contains the cursor [S] → shrinking the
  viewport (§21.4) protects panels.
- With feature `bevy_egui`: resources `EguiWantsFocus { prev, curr }` and `EguiFocusIncludesHover(bool)`; input is
  ignored while `egui_wants_pointer_input() || egui_wants_keyboard_input()` (and optionally `is_pointer_over_egui()`).
  Per §21.5 these do **not** detect bevy_egui `Panel`s, so the hover option does not help with panels; the viewport does.
- Name clash warning: Bevy 0.20 also has its own `PanOrbitCamera` (crate `bevy_camera_controller`, cargo feature
  `pan_orbit_camera`, plugin `DefaultPanOrbitCameraPlugins`) [S: release notes]. It is not enabled in the workspace;
  keep it off, or import `bevy_panorbit_camera::PanOrbitCamera` explicitly. Whether Bevy's version enters
  `bevy::prelude` is UNVERIFIED.

---

## 24. Common mistakes from older Bevy (0.12–0.17-era LLM knowledge) — use the right column

| Old / wrong | Correct in Bevy 0.20.1 | Tag |
|---|---|---|
| `SpriteBundle`, `PbrBundle`, `MaterialMeshBundle`, `Camera2dBundle`, `Camera3dBundle`, `SceneBundle`, `NodeBundle` | spawn component tuples; required components fill the rest (`Sprite{..}`, `(Mesh3d, MeshMaterial3d, Transform)`, `Camera3d::default()`) | S |
| `Handle<Mesh>` / `Handle<StandardMaterial>` as components | `Mesh3d(h)`, `MeshMaterial3d(h)`; 2D: `Mesh2d(h)`, `MeshMaterial2d(h)` | C |
| `Color::rgb(..)`, `Color::rgba(..)` | `Color::srgb(..)`, `Color::srgba(..)`, `Color::linear_rgb(..)`, `Color::srgb_u8(..)`; emissive uses `LinearRgba` | S/C |
| `time.delta_seconds()`, `elapsed_seconds()` | `delta_secs()`, `elapsed_secs()` | S |
| `query.get_single()`, `single()` panicking | `single()` / `single_mut()` return `Result`; or `Single<..>` param | S |
| `despawn_recursive()` | `despawn()` is recursive; `despawn_children()` | S/R |
| `Parent`, `parent.get()`, `set_parent(p)` | `ChildOf(p)`, `child_of.parent()`, `add_child`/`insert(ChildOf(p))` | S/C |
| `EventReader`, `EventWriter`, `Events<T>`, `add_event`, `.send()` | `MessageReader`, `MessageWriter`, `Messages<T>`, `add_message`, `.write()` | S/C |
| `Trigger<E>`, `trigger.target()` | `On<E>`, `ev.entity` / `ev.event_target()` | S/C |
| `OnAdd`, `OnReplace`, `On<Add, T>` (0.17–0.19) | `On<Add<T>>` (also `Insert<T>`, `Discard<T>`, `Remove<T>`, `Despawn<T>`) | M/S/C/R |
| `On<Pointer<Click>>`, `.pointer_location.position`, deref to `Click` fields | `On<PointerClick>`, `.pointer.position`, fields directly on the event | M/C |
| `Camera { hdr: true, .. }` | `Hdr` component (`bevy::camera::Hdr`) | S/C |
| `Camera { target: .. }` | `RenderTarget` component | S |
| `Msaa` resource | `Msaa` per-camera component | C |
| `AmbientLight` resource | `GlobalAmbientLight` resource; `AmbientLight` = per-camera component | S/C |
| `DirectionalLight { shadows_enabled: true }` | `shadow_maps_enabled: true` (Point/Spot too) | S/C/R |
| `SceneRoot`, `SceneInstanceReady`, `DynamicSceneRoot` | `WorldAssetRoot`, `WorldInstanceReady`, `DynamicWorldRoot` (`bevy::world_serialization`) | S/C/R |
| `StateScoped(S)` / `#[states(scoped_entities)]` | `DespawnOnExit(S)` / `DespawnOnEnter(S)`, always enabled | S/R |
| `NextState::set_if_neq` | `set_if_different` | M/S |
| `.wgsl` with `#import bevy_pbr::forward_io::VertexOutput`, `@group(2)`, `#{MATERIAL_BIND_GROUP}` | `.wesl`, `import bevy_pbr::render::forward_io::VertexOutput;`, `@group(constants::MATERIAL_BIND_GROUP)` (= 3) | M/S/R |
| `let image = images.get_mut(h).unwrap(); image.data[..]` | `let mut image = images.get_mut(h).unwrap();` (AssetMut) and `image.data.as_mut()` (`Option<Vec<u8>>`) | S/C/R |
| `bevy::render::camera::*`, `bevy::render::mesh::*`, `bevy::pbr::{DirectionalLight, CascadeShadowConfigBuilder}`, `bevy::core_pipeline::bloom::Bloom`, `bevy::render::texture::ImageSampler`, `bevy::render::render_asset::RenderAssetUsages`, `bevy::render::view::RenderLayers` | `bevy::camera::*`, `bevy::mesh::*`, `bevy::light::*`, `bevy::post_process::bloom::Bloom`, `bevy::image::ImageSampler`, `bevy::asset::RenderAssetUsages`, `bevy::camera::visibility::RenderLayers` | C |
| `WindowResolution::new(1280.0, 720.0)` | `WindowResolution::new(1280, 720)` (u32, physical) | S/C |
| `window.cursor.visible`, `window.cursor.grab_mode` | `CursorOptions` component on the window entity | S |
| `TransformSystem::TransformPropagate` | `TransformSystems::Propagate` | S |
| `Input<KeyCode>`, `KeyCode::A` | `ButtonInput<KeyCode>`, `KeyCode::KeyA` | C |
| `weak_handle!("..")` | `uuid_handle!("..")` (weak_handle deprecated) | S |
| `MeshTag(x)`, `tag.0` | `MeshTag::new(x)`, `tag.value` | M/C |
| `Tonemapping::None` to disable the curve | `Tonemapping::Linear`; `None` is full passthrough | M |
| `query.iter_many(..)` yields items | yields `Result`s; `.matched()` for old behavior | M |
| Systems returning `Err` "just log" | default handler **panics**; `App::set_error_handler(warn)` or `.with_severity(..)` | R |
| `Query<Entity>` = "all my entities" | also matches resource entities (`IsResource`) | R |
| `MeshPickingPlugin` assumed in DefaultPlugins | add it explicitly | S |
| `Name::from(&string)` | `Name::new(s.to_owned())` (From only for `&'static str`) | M |
| `bevy_math::primitives::*` | `bevy_shape::*` (prelude unchanged) | M |
| bevy_egui: `EguiContextPass`, `ctx_mut()` returning `&mut Context`, `EguiPlugin { enable_multipass_for_primary_context }` | `EguiPrimaryContextPass`, `ctx_mut()?` (Result), `EguiPlugin::default()` | S/C |
| egui: `SidePanel::left(..).show(ctx, ..)`, `TopBottomPanel::top(..)` | `Panel::left(..).show(&mut ui, ..)`, `Panel::top(..)` | S/C/R |
| egui: `ctx.wants_pointer_input()`, `ctx.is_pointer_over_area()`, `ctx.screen_rect()`, `ctx.available_rect()` | `egui_wants_pointer_input()`, `is_pointer_over_egui()`, `viewport_rect()` / `content_rect()` | S/C |
| egui: `egui::menu::bar(ui, ..)`, `ImageButton::new(..)`, `window_rounding` | `egui::MenuBar::new().ui(ui, ..)`, `Button::image(..)`, `window_corner_radius` | S/C |
| egui_plot: `Line::new(points)` / `.name("x")` only | `Line::new("x", points)` (name first; same for `Points`, `BarChart`) | C |
| `egui_plot = "0.36"` with bevy_egui 0.43 | `egui_plot = "0.37"` (egui 0.36) | S |
| `let gen = ..` (edition 2024) | `gen` is reserved; rename | C |

### 24.1 Other 0.19→0.20 changes (from `_release-content/migration-guides`, [M]) likely to matter here
- Shaders: WESL migration (above); GLSL support removed; `shader_format_wesl` feature removed.
- `Sprite` rendered via Mesh2d/Material2d; new `Sprite::alpha_mode`; `SpriteMaterial` → `SpriteMeshMaterial` (internal).
- `ScreenSpaceTransmission` opt-in; `Tonemapping::None` passthrough + `Tonemapping::Linear`; `Camera2d` defaults to Linear.
- `ShaderBuffer` API reworked (`new(Vec<T>)`, `extend`, `cast_slice`); `UnpreparedBindGroup` → `BindGroupBuilder`
  (only relevant for manual `AsBindGroup` impls).
- `NextState::set_if_different`; lifecycle observer syntax `On<Add<T>>`; flat pointer events; `iter_many` results;
  `MeshTag::new`; `MeshAabb::compute_aabb` → `get_aabb`; `bevy_curve` and `bevy_shape` split out of `bevy_math`.
- `DeferredWorld::query` deprecated (`query_state.query_mut(&mut world)`); `FilteredResources` deprecated.
- `TextFont::default()` size is `FontSize::Rem(1.)`; `FontSource::sans_serif()` etc.; `Val::Em/Rem` (Bevy UI only;
  we use egui).
- `CompressedImageSaver` feature split; JPEG added to default processor.
- `RenderDebugOverlay` added (keybinds opt-in, needs `render_dev_tools`).
- Release notes: panics inside systems/commands/observers are now routed to the fallback error handler (default
  re-panics); schedule randomization (`ScheduleBuildSettings::shuffle_seed` with `debug` feature) for order-bug hunting;
  BSN `Ready` event.

## 25. Things NOT verified (treat as UNVERIFIED)
- `TextureFormat::R8Unorm` dynamic textures sampled in a custom material (expected to work; not run).
- `embedded_asset!` path computation for workspace crates whose dir name ≠ crate name.
- `.cargo/config.toml` `[env] BEVY_ASSET_ROOT = { value = ".", relative = true }`.
- Clicking (PointerClick) end-to-end — observers compile and the picking pipeline renders, but no click was simulated.
- egui drag-in-panel blocking via `egui_is_using_pointer()` (source-inferred, not run).
- `PlotPoints::Borrowed` usage; custom diagnostics registration API.
- Whether Bevy's own `PanOrbitCamera` (feature `pan_orbit_camera`) is exported from `bevy::prelude`.
- Performance numbers other than the single debug-build FPS sample.

## 26. Sources
- Local crate sources (authoritative): `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/{bevy-0.20.1 (examples/, _release-content/), bevy_ecs, bevy_app, bevy_state, bevy_time, bevy_window, bevy_input, bevy_camera, bevy_light, bevy_pbr, bevy_mesh, bevy_image, bevy_asset, bevy_sprite, bevy_sprite_render, bevy_gizmos, bevy_picking, bevy_animation, bevy_gltf, bevy_world_serialization, bevy_render, bevy_post_process, bevy_diagnostic, bevy_internal}-0.20.1`, `bevy_egui-0.43.1` (src, examples, CHANGELOG.md), `egui-0.36.2`, `epaint-0.36.2`, `egui_plot-0.37.0`, `bevy_panorbit_camera-0.36.0`.
- Bevy example list/docs: https://github.com/bevyengine/bevy/tree/v0.20.1/examples ; API docs https://docs.rs/bevy/0.20.1
- Reference WESL shaders downloaded (read only, not copied verbatim into the project) from
  https://raw.githubusercontent.com/bevyengine/bevy/v0.20.1/assets/shaders/ (`custom_material.wesl`, `custom_material_2d.wesl`,
  `animate_shader.wesl`, `instancing.wesl`), MIT OR Apache-2.0.
- Test model `Fox.glb`: https://raw.githubusercontent.com/bevyengine/bevy/v0.20.1/assets/models/animated/Fox.glb
  (glTF Sample Assets Fox: model CC0 by PixelMannen, rigging/animation CC-BY 4.0 by @tomkranis; per Bevy CREDITS.md).
  Used only for local testing; not added to the project.
- bevy_egui: https://github.com/vladbat00/bevy_egui ; egui: https://github.com/emilk/egui ; egui_plot:
  https://github.com/emilk/egui_plot ; bevy_panorbit_camera: https://github.com/Plonq/bevy_panorbit_camera
