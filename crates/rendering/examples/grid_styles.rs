//! All grid styles side by side on live toy automata.
//!
//! Twelve tiles: the ten `GridStyle`s on Conway's Life (B3/S23 on a torus),
//! Brian's Brain (Gradient tile), a Wireworld loop with the Golly palette and
//! a continuous test field (sum of moving Gaussian rings, not a model) with the
//! Lenia-blue colormap and smooth interpolation. The Density tile shows a
//! level-of-detail frame (2 × 2 cells per texel, A = live fraction).
//!
//! Controls: right/middle drag or WASD/arrows to pan, wheel/trackpad/pinch or
//! +/- to zoom (towards the cursor), Space to pause, G to toggle grid lines.
//! `EMERGENCE_SMOKE_SCREENSHOT=out.png` captures a screenshot and exits.
//!
//! The automata here are minimal example code; the real engines live in the
//! `cellular-automata` and `continuous` crates.

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::math::I64Vec2;
use bevy::post_process::bloom::{Bloom, BloomPrefilter};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rendering::grid::{GridFrame, GridSpace, Texel, encode_unit};
use rendering::palette::{Colormap, GridPalette};
use rendering::{
    GridAppearance, GridCameraController, GridCameraPlugin, GridCameraSystems, GridRenderPlugin, GridStyle,
    GridSurface, SmokeScreenshotPlugin, StandaloneWorldInputPlugin, VisibleCells, WorldInput, cursor_to_cell,
};

const W: usize = 48;
const H: usize = 32;
const GAP_X: i64 = 8;
const GAP_Y: i64 = 12;
const COLS: i64 = 4;
const CELL: f32 = 8.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Emergence Lab - grid styles".into(),
                resolution: (1600, 1000).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((GridRenderPlugin, GridCameraPlugin, StandaloneWorldInputPlugin, SmokeScreenshotPlugin::default()))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.065, 0.08)))
        .insert_resource(GridSpace::new(CELL))
        .add_systems(Startup, setup)
        .add_systems(Update, (controls, step_worlds, upload_frames).chain())
        .add_systems(Update, show_view_in_title.after(GridCameraSystems::Visible))
        .run();
}

/// Which toy world a tile shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Life,
    LifeDensity,
    Brain,
    Wire,
    Field,
}

#[derive(Component)]
struct Tile {
    source: Source,
    origin: I64Vec2,
}

#[derive(Resource)]
struct Worlds {
    life: Toy,
    brain: Toy,
    wire: Toy,
    time: f32,
    timer: Timer,
    paused: bool,
}

/// Tiny torus automaton with per-cell age and decaying trail (example only).
struct Toy {
    cells: Vec<u8>,
    age: Vec<u8>,
    trail: Vec<u8>,
}

impl Toy {
    fn new(cells: Vec<u8>) -> Self {
        let age = cells.iter().map(|&c| u8::from(c != 0)).collect();
        Self { cells, age, trail: vec![0; W * H] }
    }

    fn neighbours(&self, x: usize, y: usize, state: u8) -> u8 {
        let mut n = 0;
        for dy in [H - 1, 0, 1] {
            for dx in [W - 1, 0, 1] {
                if (dx, dy) != (0, 0) && self.cells[((y + dy) % H) * W + (x + dx) % W] == state {
                    n += 1;
                }
            }
        }
        n
    }

    fn step(&mut self, rule: impl Fn(u8, &Self, usize, usize) -> u8) {
        let mut next = vec![0; W * H];
        for y in 0..H {
            for x in 0..W {
                next[y * W + x] = rule(self.cells[y * W + x], self, x, y);
            }
        }
        for (i, &state) in next.iter().enumerate() {
            let changed = state != self.cells[i];
            self.trail[i] = if changed { 255 } else { (self.trail[i] as f32 * 0.82) as u8 };
            self.age[i] = if state == 0 { 0 } else { self.age[i].saturating_add(1) };
        }
        self.cells = next;
    }
}

fn life_rule(state: u8, toy: &Toy, x: usize, y: usize) -> u8 {
    let n = toy.neighbours(x, y, 1);
    u8::from(n == 3 || (state == 1 && n == 2))
}

fn brain_rule(state: u8, toy: &Toy, x: usize, y: usize) -> u8 {
    match state {
        0 => u8::from(toy.neighbours(x, y, 1) == 2),
        1 => 2,
        _ => 0,
    }
}

fn wire_rule(state: u8, toy: &Toy, x: usize, y: usize) -> u8 {
    match state {
        1 => 2,
        2 => 3,
        3 => {
            let heads = toy.neighbours(x, y, 1);
            if heads == 1 || heads == 2 { 1 } else { 3 }
        }
        _ => 0,
    }
}

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn random_cells(seed: u64, percent: u64, state: u8) -> Vec<u8> {
    let mut s = seed;
    (0..W * H).map(|_| if xorshift(&mut s) % 100 < percent { state } else { 0 }).collect()
}

fn wire_loop() -> Vec<u8> {
    let mut cells = vec![0; W * H];
    let (x0, y0, x1, y1) = (4, 4, W - 5, H - 5);
    for x in x0..=x1 {
        cells[y0 * W + x] = 3;
        cells[y1 * W + x] = 3;
    }
    for y in y0..=y1 {
        cells[y * W + x0] = 3;
        cells[y * W + x1] = 3;
    }
    // A cross-bar wire feeding the loop both ways.
    for x in x0..=x1 {
        cells[(H / 2) * W + x] = 3;
    }
    // Electrons (head, tail) travelling clockwise.
    for &(hx, tx) in &[(10usize, 9usize), (22, 21), (34, 33)] {
        cells[y0 * W + hx] = 1;
        cells[y0 * W + tx] = 2;
    }
    cells
}

fn tile_origin(index: i64) -> I64Vec2 {
    I64Vec2::new((index % COLS) * (W as i64 + GAP_X), (index / COLS) * (H as i64 + GAP_Y))
}

fn setup(mut commands: Commands, space: Res<GridSpace>) {
    let tiles: [(&str, Source, GridAppearance); 12] = [
        ("Monochrome", Source::Life, GridAppearance::styled(GridStyle::Monochrome)),
        ("Pixel art", Source::Life, GridAppearance::styled(GridStyle::PixelArt)),
        ("Neon (HDR bloom)", Source::Life, GridAppearance::styled(GridStyle::Neon)),
        ("Minimal", Source::Life, GridAppearance::styled(GridStyle::Minimal)),
        ("Circles", Source::Life, GridAppearance::styled(GridStyle::Circles)),
        ("Gradient - Brian's Brain", Source::Brain, GridAppearance::styled(GridStyle::Gradient)),
        ("Heatmap (activity)", Source::Life, GridAppearance::styled(GridStyle::Heatmap)),
        ("Age", Source::Life, GridAppearance::styled(GridStyle::Age)),
        ("Density (LOD 2x2)", Source::LifeDensity, GridAppearance::styled(GridStyle::Density)),
        ("History (trails)", Source::Life, GridAppearance::styled(GridStyle::History)),
        (
            "Wireworld palette",
            Source::Wire,
            GridAppearance { palette: GridPalette::wireworld(), ..GridAppearance::styled(GridStyle::PixelArt) },
        ),
        (
            "Field - Lenia blue, smooth",
            Source::Field,
            GridAppearance {
                palette: GridPalette::Continuous(Colormap::LeniaBlue),
                smooth_fields: true,
                show_grid: false,
                ..GridAppearance::styled(GridStyle::PixelArt)
            },
        ),
    ];
    for (i, (label, source, appearance)) in tiles.into_iter().enumerate() {
        let origin = tile_origin(i as i64);
        commands.spawn((GridSurface::default(), appearance, Tile { source, origin }));
        let top_centre = space.cell_corner_to_world(origin + I64Vec2::new(W as i64 / 2, 0));
        commands.spawn((
            Text2d::new(label),
            TextFont { font_size: FontSize::Px(22.0), ..default() },
            TextColor(Color::srgb(0.85, 0.87, 0.9)),
            Transform::from_translation((top_centre + Vec2::new(0.0, 0.7 * CELL)).extend(5.0)),
            bevy::sprite::Anchor::BOTTOM_CENTER,
        ));
    }

    let mut life = Toy::new(random_cells(0x9E37_79B9_7F4A_7C15, 33, 1));
    let mut brain = Toy::new(random_cells(0xD1B5_4A32_D192_ED03, 20, 1));
    let mut wire = Toy::new(wire_loop());
    for _ in 0..40 {
        life.step(life_rule);
        brain.step(brain_rule);
        wire.step(wire_rule);
    }
    commands.insert_resource(Worlds {
        life,
        brain,
        wire,
        time: 0.0,
        timer: Timer::from_seconds(0.12, TimerMode::Repeating),
        paused: false,
    });

    let mut controller = GridCameraController::default();
    // No application shell here, so `=` / `-` are free for zooming.
    controller.key_zoom = true;
    let last = tile_origin(11) + I64Vec2::new(W as i64 - 1, H as i64 - 1);
    controller.fit_to_rect(I64Vec2::new(0, -3), last, 0.02);
    commands.spawn((
        controller,
        Tonemapping::Linear,
        Bloom { prefilter: BloomPrefilter { threshold: 1.0, threshold_softness: 0.4 }, ..Bloom::NATURAL },
        DebandDither::Enabled,
    ));
}

fn controls(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut worlds: ResMut<Worlds>,
    mut appearances: Query<&mut GridAppearance>,
) {
    if !input.keyboard {
        return;
    }
    if keys.just_pressed(KeyCode::Space) {
        worlds.paused = !worlds.paused;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        for mut appearance in &mut appearances {
            appearance.show_grid = !appearance.show_grid;
        }
    }
}

fn step_worlds(time: Res<Time>, mut worlds: ResMut<Worlds>) {
    if worlds.paused {
        return;
    }
    worlds.time += time.delta_secs();
    if worlds.timer.tick(time.delta()).just_finished() {
        worlds.life.step(life_rule);
        worlds.brain.step(brain_rule);
        worlds.wire.step(wire_rule);
    }
}

fn toy_frame(origin: I64Vec2, toy: &Toy) -> Option<GridFrame> {
    let mut frame = GridFrame::new(origin, W as u32, H as u32, 1).ok()?;
    for (i, px) in frame.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let alive = toy.cells[i] != 0;
        *px = Texel { state: toy.cells[i], age: toy.age[i], activity: toy.trail[i], aux: if alive { 255 } else { 0 } }
            .to_rgba();
    }
    Some(frame)
}

fn density_frame(origin: I64Vec2, toy: &Toy, k: usize) -> Option<GridFrame> {
    let (tw, th) = (W / k, H / k);
    let mut frame = GridFrame::new(origin, tw as u32, th as u32, k as u32).ok()?;
    for ty in 0..th {
        for tx in 0..tw {
            let live = (0..k * k).filter(|j| toy.cells[(ty * k + j / k) * W + tx * k + j % k] != 0).count();
            let texel =
                Texel { state: u8::from(live > 0), aux: encode_unit(live as f32 / (k * k) as f32), ..default() };
            frame.set(tx as u32, ty as u32, texel).ok()?;
        }
    }
    Some(frame)
}

fn field_frame(origin: I64Vec2, t: f32) -> Option<GridFrame> {
    let mut frame = GridFrame::new(origin, W as u32, H as u32, 1).ok()?;
    let centres = [
        Vec2::new(12.0 + 4.0 * (0.7 * t).sin(), 14.0 + 3.0 * (0.5 * t).cos()),
        Vec2::new(32.0 + 5.0 * (0.4 * t).cos(), 18.0 + 4.0 * (0.6 * t).sin()),
    ];
    for y in 0..H {
        for x in 0..W {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let v: f32 = centres
                .iter()
                .map(|c| {
                    let d = p.distance(*c);
                    (-(d - 6.0).powi(2) / 6.0).exp() + 0.6 * (-(d * d) / 4.0).exp()
                })
                .sum();
            frame.set(x as u32, y as u32, Texel::state(encode_unit(v.min(1.0)))).ok()?;
        }
    }
    Some(frame)
}

fn upload_frames(worlds: Res<Worlds>, mut tiles: Query<(&Tile, &mut GridSurface)>) {
    if !worlds.is_changed() {
        return;
    }
    for (tile, mut surface) in &mut tiles {
        let frame = match tile.source {
            Source::Life => toy_frame(tile.origin, &worlds.life),
            Source::LifeDensity => density_frame(tile.origin, &worlds.life, 2),
            Source::Brain => toy_frame(tile.origin, &worlds.brain),
            Source::Wire => toy_frame(tile.origin, &worlds.wire),
            Source::Field => field_frame(tile.origin, worlds.time),
        };
        if let Some(frame) = frame
            && let Err(err) = surface.submit(frame)
        {
            error!("grid frame rejected: {err}");
        }
    }
}

/// Shows the visible cell range, zoom and hovered cell in the window title.
fn show_view_in_title(
    input: Res<WorldInput>,
    space: Res<GridSpace>,
    visible: Res<VisibleCells>,
    camera: Single<(&Camera, &GlobalTransform), With<GridCameraController>>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
) {
    let (camera, camera_transform) = *camera;
    let hovered = cursor_to_cell(&input, camera, camera_transform, &space)
        .map_or_else(|| "-".to_string(), |c| format!("({}, {})", c.x, c.y));
    let title = format!(
        "Emergence Lab - grid styles | visible x {}..={} y {}..={} | {:.3} cells/px | cell {hovered}",
        visible.min_x, visible.max_x, visible.min_y, visible.max_y, visible.cells_per_pixel
    );
    if visible.is_changed() {
        info!("{title}");
    }
    if window.title != title {
        window.title = title;
    }
}
