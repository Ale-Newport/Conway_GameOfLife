//! World interaction: the inspect, brush, eraser and creature-stamp tools
//! (gated by [`WorldInput`]) and the mode's keyboard shortcuts.

use std::collections::HashMap;

use bevy::math::DVec2;
use bevy::prelude::*;
use continuous::{Blend, Brush, BrushMode, ContinuousCommand, CreatureCells, Placement};
use rendering::grid::GridSpace;
use rendering::palette::Colormap;
use rendering::{WorldInput, cursor_to_cell, shortcut_modifier_held};
use simulation_core::Control;

use crate::field::{creature_rgba, dab_spacing, inside_world, stroke_dabs, wrap_degrees};
use crate::session::{Catalog, ContinuousRunner, LoadedWorld};
use crate::world::{CameraFit, FieldCamera, ViewSettings};

/// Active world tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Hover to read values; nothing is edited.
    #[default]
    Inspect,
    /// Soft brush that blends cells towards a value.
    Brush,
    /// Soft brush that blends cells towards zero.
    Erase,
    /// Places a creature from the library.
    Stamp,
}

impl Tool {
    /// Every tool in toolbar order.
    pub const ALL: [Tool; 4] = [Tool::Inspect, Tool::Brush, Tool::Erase, Tool::Stamp];

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Inspect => "Inspect",
            Tool::Brush => "Brush",
            Tool::Erase => "Erase",
            Tool::Stamp => "Stamp",
        }
    }

    /// Tooltip.
    pub fn help(self) -> &'static str {
        match self {
            Tool::Inspect => "Hover the world to read cell values (key 1).",
            Tool::Brush => "Drag to paint matter with a soft round brush; one stroke is one undo step (key 2).",
            Tool::Erase => "Drag to remove matter with a soft round brush (key 3).",
            Tool::Stamp => "Click to place the selected creature; Q / E rotate, [ / ] scale (key 4).",
        }
    }
}

/// Tool parameters (kept across visits to the mode).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ToolSettings {
    /// Active tool.
    pub tool: Tool,
    /// Value the brush paints towards.
    pub brush_value: f32,
    /// Brush radius in cells.
    pub brush_radius: f32,
    /// Fraction of the radius over which the brush fades out.
    pub brush_softness: f32,
    /// Channel edited by the brush (`None` = every channel).
    pub brush_channel: Option<usize>,
    /// Index into [`Catalog::creatures`].
    pub creature: usize,
    /// Clockwise rotation of placed creatures, degrees.
    pub rotation_deg: f32,
    /// Scale of placed creatures.
    pub scale: f32,
    /// Replace the covered cells or add to them.
    pub blend: Blend,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            tool: Tool::Inspect,
            brush_value: 0.6,
            brush_radius: 8.0,
            brush_softness: 0.5,
            brush_channel: None,
            creature: 0,
            rotation_deg: 0.0,
            scale: 1.0,
            blend: Blend::Replace,
        }
    }
}

/// Brush radius range offered by the panel and the shortcuts (cells).
pub const BRUSH_RADIUS: (f32, f32) = (1.0, 64.0);
/// Creature scale range offered by the panel and the shortcuts.
pub const STAMP_SCALE: (f32, f32) = (0.25, 4.0);
/// Rotation step of Q / E, degrees.
pub const ROTATION_STEP: f32 = 15.0;

impl ToolSettings {
    /// Rotates the stamp by `delta` degrees (wrapped to (−180°, 180°]).
    pub fn rotate(&mut self, delta: f32) {
        self.rotation_deg = wrap_degrees(self.rotation_deg + delta);
    }

    /// Grows (or shrinks) the brush or the stamp, depending on the tool.
    pub fn resize(&mut self, grow: bool) {
        if self.tool == Tool::Stamp {
            let f = if grow { 1.25 } else { 0.8 };
            self.scale = round_to(self.scale * f, 0.05).clamp(STAMP_SCALE.0, STAMP_SCALE.1);
        } else {
            let f = if grow { 1.25 } else { 0.8 };
            self.brush_radius = round_to(self.brush_radius * f, 0.5).clamp(BRUSH_RADIUS.0, BRUSH_RADIUS.1);
        }
    }

    /// Brush dab at `p` (cell coordinates).
    pub fn brush_at(&self, p: DVec2) -> Brush {
        Brush {
            x: p.x as f32,
            y: p.y as f32,
            radius: self.brush_radius,
            value: self.brush_value,
            softness: self.brush_softness,
            mode: if self.tool == Tool::Erase { BrushMode::Erase } else { BrushMode::Paint },
            channel: self.brush_channel,
        }
    }

    /// Placement of the stamp centred at `p` (cell coordinates).
    pub fn placement_at(&self, p: DVec2) -> Placement {
        Placement {
            x: p.x as f32,
            y: p.y as f32,
            rotation_deg: self.rotation_deg,
            scale: self.scale,
            blend: self.blend,
        }
    }

    /// Clamps settings that depend on the world (channel index, brush value).
    pub fn fit_world(&mut self, channels: usize, value_limit: f32) {
        if self.brush_channel.is_some_and(|c| c >= channels) || channels <= 1 {
            self.brush_channel = None;
        }
        self.brush_value = self.brush_value.clamp(0.0, value_limit);
    }
}

fn round_to(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}

/// Largest brush value offered for a world whose edits are limited to
/// `edit_limit` (Flow-Lenia's mass is unbounded; values above a few units
/// are rarely useful).
pub fn brush_value_max(edit_limit: f32) -> f32 {
    edit_limit.min(5.0)
}

/// Cursor position over the world (cell coordinates), if any.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Hover {
    /// Fractional cell position under the pointer.
    pub position: Option<DVec2>,
}

/// An in-progress brush stroke.
#[derive(Resource, Default, Debug)]
pub struct StrokeState {
    last: Option<DVec2>,
}

/// World-space images of creatures for the stamp preview.
#[derive(Resource, Default)]
pub struct StampImages {
    images: HashMap<(usize, Colormap), Handle<Image>>,
}

impl StampImages {
    /// Image of creature `index` in `map`, created on first use.
    pub fn image(
        &mut self,
        index: usize,
        cells: &CreatureCells,
        map: Colormap,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        self.images
            .entry((index, map))
            .or_insert_with(|| {
                let (w, h, data) = creature_rgba(cells, map);
                let mut image = Image::new(
                    bevy::render::render_resource::Extent3d {
                        width: w as u32,
                        height: h as u32,
                        depth_or_array_layers: 1,
                    },
                    bevy::render::render_resource::TextureDimension::D2,
                    data,
                    bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                    bevy::asset::RenderAssetUsages::RENDER_WORLD,
                );
                image.sampler = bevy::image::ImageSampler::linear();
                images.add(image)
            })
            .clone()
    }
}

/// Hover tracking and the pointer tools. A brush stroke is one undoable
/// edit: its first dab is sent as `Control::Edit`, the rest as commands.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pointer_tools(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    camera: Single<(&Camera, &GlobalTransform), With<FieldCamera>>,
    space: Res<GridSpace>,
    loaded: Option<Res<LoadedWorld>>,
    runner: Option<Res<ContinuousRunner>>,
    catalog: Res<Catalog>,
    tools: Res<ToolSettings>,
    mut stroke: ResMut<StrokeState>,
    mut hover: ResMut<Hover>,
) {
    let (camera, camera_transform) = *camera;
    let position = cursor_to_cell(&input, camera, camera_transform, &space).and_then(|_| {
        let world = camera.viewport_to_world_2d(camera_transform, input.cursor?).ok()?;
        let (x, y) = space.world_to_cell_f64(world);
        Some(DVec2::new(x, y))
    });
    hover.position = position;
    let (Some(loaded), Some(runner)) = (loaded, runner) else { return };
    let (w, h) = (loaded.width, loaded.height);

    if !buttons.pressed(MouseButton::Left) {
        stroke.last = None;
    }
    match tools.tool {
        Tool::Inspect => {}
        Tool::Brush | Tool::Erase => {
            let Some(p) = position else { return };
            if buttons.just_pressed(MouseButton::Left) && inside_world(p, w, h) {
                runner.0.send(Control::Edit(ContinuousCommand::Brush(tools.brush_at(p))));
                stroke.last = Some(p);
            } else if let Some(last) = stroke.last
                && buttons.pressed(MouseButton::Left)
            {
                let (dabs, next) = stroke_dabs(last, p, dab_spacing(tools.brush_radius));
                for dab in dabs.into_iter().filter(|d| inside_world(*d, w, h)) {
                    runner.0.send(Control::Command(ContinuousCommand::Brush(tools.brush_at(dab))));
                }
                stroke.last = Some(next);
            }
        }
        Tool::Stamp => {
            let Some(p) = position.filter(|p| inside_world(*p, w, h)) else { return };
            if buttons.just_pressed(MouseButton::Left)
                && let Some(creature) = catalog.creatures.get(tools.creature)
            {
                runner.0.send(Control::Edit(ContinuousCommand::PlaceCreature {
                    cells: creature.cells.clone(),
                    placement: tools.placement_at(p),
                }));
            }
        }
    }
}

/// A mode shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeShortcut {
    /// Select a tool.
    Tool(Tool),
    /// Rotate the stamp counter-clockwise.
    RotateLeft,
    /// Rotate the stamp clockwise.
    RotateRight,
    /// Smaller brush / stamp.
    Smaller,
    /// Larger brush / stamp.
    Larger,
    /// Frame the world.
    Fit,
    /// Next colormap (previous with Shift).
    Colormap,
    /// Next displayed layer.
    Layer,
    /// Toggle the centroid trail.
    Trail,
    /// Toggle grid lines.
    Grid,
}

/// The mode's keys, for the shortcut list in the tools panel. None of them
/// is used by the shell (`interface::input::SHORTCUTS`) or by the grid
/// camera (arrows, WASD); checked by a test.
pub const MODE_SHORTCUTS: &[(KeyCode, ModeShortcut, &str, &str)] = &[
    (KeyCode::Digit1, ModeShortcut::Tool(Tool::Inspect), "1", "Inspect tool"),
    (KeyCode::Digit2, ModeShortcut::Tool(Tool::Brush), "2", "Brush tool"),
    (KeyCode::Digit3, ModeShortcut::Tool(Tool::Erase), "3", "Eraser tool"),
    (KeyCode::Digit4, ModeShortcut::Tool(Tool::Stamp), "4", "Creature stamp tool"),
    (KeyCode::KeyQ, ModeShortcut::RotateLeft, "Q", "Rotate the stamp 15° counter-clockwise"),
    (KeyCode::KeyE, ModeShortcut::RotateRight, "E", "Rotate the stamp 15° clockwise"),
    (KeyCode::BracketLeft, ModeShortcut::Smaller, "[", "Smaller brush or stamp"),
    (KeyCode::BracketRight, ModeShortcut::Larger, "]", "Larger brush or stamp"),
    (KeyCode::KeyF, ModeShortcut::Fit, "F", "Frame the whole world"),
    (KeyCode::KeyC, ModeShortcut::Colormap, "C / Shift+C", "Next / previous colormap"),
    (KeyCode::KeyL, ModeShortcut::Layer, "L", "Next layer (state, potential, growth)"),
    (KeyCode::KeyT, ModeShortcut::Trail, "T", "Show / hide the centroid trail"),
    (KeyCode::KeyG, ModeShortcut::Grid, "G", "Show / hide cell grid lines"),
];

/// The shortcut bound to `key`, if any.
pub fn mode_shortcut(key: KeyCode) -> Option<ModeShortcut> {
    MODE_SHORTCUTS.iter().find(|(k, ..)| *k == key).map(|(_, s, ..)| *s)
}

/// Applies the mode's keyboard shortcuts (only while no egui widget has the
/// keyboard and without Ctrl/Cmd/Alt, which belong to the shell and the OS).
#[allow(clippy::too_many_arguments)]
pub(crate) fn mode_shortcuts(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    loaded: Option<Res<LoadedWorld>>,
    mut tools: ResMut<ToolSettings>,
    mut settings: ResMut<ViewSettings>,
    mut fit: ResMut<CameraFit>,
) {
    if !input.keyboard || shortcut_modifier_held(&keys) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let channels = loaded.as_ref().map_or(1, |l| l.channels());
    for key in keys.get_just_pressed() {
        let Some(shortcut) = mode_shortcut(*key) else { continue };
        match shortcut {
            ModeShortcut::Tool(tool) => tools.tool = tool,
            ModeShortcut::RotateLeft => tools.rotate(-ROTATION_STEP),
            ModeShortcut::RotateRight => tools.rotate(ROTATION_STEP),
            ModeShortcut::Smaller => tools.resize(false),
            ModeShortcut::Larger => tools.resize(true),
            ModeShortcut::Fit => fit.request(true),
            ModeShortcut::Colormap => settings.cycle_colormap(!shift),
            ModeShortcut::Layer => settings.cycle_layer(channels),
            ModeShortcut::Trail => settings.show_trail = !settings.show_trail,
            ModeShortcut::Grid => settings.grid_lines = !settings.grid_lines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface::input::SHORTCUTS;

    #[test]
    fn shortcuts_do_not_clash_with_the_shell_or_the_camera() {
        let camera = [
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
        ];
        for (i, (key, _, label, _)) in MODE_SHORTCUTS.iter().enumerate() {
            assert!(SHORTCUTS.iter().all(|s| s.key != *key), "{label} is a shell shortcut");
            assert!(!camera.contains(key), "{label} moves the camera");
            assert!(MODE_SHORTCUTS[i + 1..].iter().all(|(k, ..)| k != key), "{label} is bound twice");
        }
        assert_eq!(mode_shortcut(KeyCode::Digit4), Some(ModeShortcut::Tool(Tool::Stamp)));
        assert_eq!(mode_shortcut(KeyCode::KeyR), None, "R is the shell's reset");
    }

    #[test]
    fn rotation_and_size_steps() {
        let mut t = ToolSettings { rotation_deg: 170.0, ..default() };
        t.rotate(ROTATION_STEP);
        assert_eq!(t.rotation_deg, -175.0);
        t.rotate(-ROTATION_STEP);
        assert_eq!(t.rotation_deg, 170.0);
        t.tool = Tool::Brush;
        t.resize(true);
        assert_eq!(t.brush_radius, 10.0);
        for _ in 0..30 {
            t.resize(true);
        }
        assert_eq!(t.brush_radius, BRUSH_RADIUS.1);
        for _ in 0..30 {
            t.resize(false);
        }
        assert_eq!(t.brush_radius, BRUSH_RADIUS.0);
        t.tool = Tool::Stamp;
        t.resize(true);
        assert_eq!(t.scale, 1.25);
        for _ in 0..20 {
            t.resize(false);
        }
        assert_eq!(t.scale, STAMP_SCALE.0);
        assert_eq!(t.brush_radius, BRUSH_RADIUS.0, "the stamp keys leave the brush alone");
    }

    #[test]
    fn brushes_and_placements_carry_the_settings() {
        let t = ToolSettings { tool: Tool::Erase, brush_channel: Some(2), ..default() };
        let b = t.brush_at(DVec2::new(3.5, 4.25));
        assert_eq!((b.x, b.y, b.mode, b.channel), (3.5, 4.25, BrushMode::Erase, Some(2)));
        assert_eq!(ToolSettings::default().brush_at(DVec2::ZERO).mode, BrushMode::Paint);
        let p = ToolSettings { rotation_deg: 30.0, scale: 2.0, blend: Blend::Add, ..default() }
            .placement_at(DVec2::new(10.0, 20.0));
        assert_eq!((p.x, p.y, p.rotation_deg, p.scale, p.blend), (10.0, 20.0, 30.0, 2.0, Blend::Add));
    }

    #[test]
    fn world_limits_are_applied() {
        let mut t = ToolSettings { brush_channel: Some(2), brush_value: 0.9, ..default() };
        t.fit_world(3, 1.0);
        assert_eq!(t.brush_channel, Some(2));
        t.fit_world(2, 1.0);
        assert_eq!(t.brush_channel, None);
        t.brush_value = 4.0;
        t.fit_world(1, 1.0);
        assert_eq!(t.brush_value, 1.0);
        assert_eq!(brush_value_max(100.0), 5.0);
        assert_eq!(brush_value_max(1.0), 1.0);
    }
}
