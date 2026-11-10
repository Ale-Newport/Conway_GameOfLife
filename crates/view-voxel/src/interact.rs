//! World interaction: voxel picking under the cursor, the Inspect / Place /
//! Erase tools, keyboard shortcuts and named actions (tutorial buttons).
//!
//! Picking walks the voxels pierced by the cursor ray (Amanatides–Woo) and
//! stops at the first voxel of the displayed surface view, so it agrees
//! with what is drawn, including cuts. A click is a left press and release
//! within 5 px and 0.6 s; longer drags orbit the camera.

use bevy::prelude::*;
use interface::{ModeScenarios, RightPanelTab, ShellCommand, Toasts};
use rendering::{OrbitPreset, WorldInput, shortcut_modifier_held};
use simulation_core::Control;
use voxel_life::{Axis, VoxelCommand};

use crate::MODE;
use crate::build::MeshState;
use crate::coords::{box_contains, ray_layer_voxel, raycast_voxels};
use crate::requests::{ProbeState, WorldState};
use crate::scene::{CameraAction, VoxelCamera};
use crate::session::VoxelRunner;
use crate::settings::{ColorMode, Tool, Transparency, ViewSettings, next_axis};
use crate::shortcuts::{ShortcutAction, match_shortcut};

/// What the cursor points at.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Hover {
    /// Visible voxel under the cursor.
    pub voxel: Option<IVec3>,
    /// Where the Place tool would add a voxel.
    pub place: Option<IVec3>,
}

/// Pointer state of the world view.
#[derive(Resource, Default)]
pub(crate) struct PointerState {
    /// Current hover (none outside the world viewport).
    pub hover: Option<Hover>,
    press: Option<(Vec2, f32)>,
}

/// Largest cursor travel of a click, in logical pixels.
const CLICK_SLOP: f32 = 5.0;
/// Longest press of a click, in seconds.
const CLICK_TIME: f32 = 0.6;

/// Hover picking and tool clicks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn world_pointer(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    settings: Res<ViewSettings>,
    mesh: Res<MeshState>,
    world: Res<WorldState>,
    camera: Option<Single<(&Camera, &GlobalTransform), With<VoxelCamera>>>,
    runner: Res<VoxelRunner>,
    mut probe: ResMut<ProbeState>,
    mut pointer: ResMut<PointerState>,
    mut toasts: ResMut<Toasts>,
) {
    pointer.hover = None;
    if buttons.just_pressed(MouseButton::Left) {
        pointer.press = input.pointer.then(|| input.cursor.map(|c| (c, time.elapsed_secs()))).flatten();
    }
    let (Some(camera), Some(cursor), Some(info)) = (camera, input.cursor, world.info.as_ref()) else { return };
    if !input.pointer {
        return;
    }
    let (camera, camera_transform) = *camera;
    let Ok(ray) = camera.viewport_to_world(camera_transform, cursor) else { return };
    let frame = world.frame();
    let origin = frame.to_model(ray.origin);
    let dir = frame.dir_to_model(*ray.direction);
    let hit = mesh
        .occupancy
        .as_ref()
        .and_then(|occ| raycast_voxels(origin, dir, occ.min(), occ.max(), 8192, |v| occ.contains(v)));
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let tool = match settings.tool {
        Tool::Place if shift => Tool::Erase,
        tool => tool,
    };
    let inside = |v: IVec3| if info.is_finite() { box_contains(info.bounds, v) } else { true };
    let place = (tool == Tool::Place)
        .then(|| match hit {
            Some(h) if h.normal != IVec3::ZERO => Some(h.voxel + h.normal),
            Some(_) => None,
            None => {
                let floor = if settings.cut.enabled && settings.cut.axis == Axis::Z {
                    settings.cut.min.min(settings.cut.max)
                } else {
                    world.layers().min[2]
                };
                ray_layer_voxel(origin, dir, Axis::Z, floor)
            }
        })
        .flatten()
        .filter(|v| inside(*v));
    pointer.hover = Some(Hover { voxel: if tool == Tool::Place { None } else { hit.map(|h| h.voxel) }, place });

    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let Some((pressed_at, t0)) = pointer.press.take() else { return };
    if (cursor - pressed_at).length() > CLICK_SLOP || time.elapsed_secs() - t0 > CLICK_TIME {
        return;
    }
    match tool {
        Tool::Inspect => probe.pinned = hit.map(|h| h.voxel.to_array()),
        Tool::Place => match place {
            Some(target) => runner.0.edit(VoxelCommand::SetVoxel { pos: target.to_array(), state: 1 }),
            None => toasts.info("Point at a voxel face or at the floor of the world to place a voxel."),
        },
        Tool::Erase => {
            if let Some(h) = hit {
                runner.0.edit(VoxelCommand::ClearVoxel { pos: h.voxel.to_array() });
            }
        }
    }
}

/// Mode keyboard shortcuts (see [`crate::shortcuts::SHORTCUTS`]).
pub(crate) fn keyboard_shortcuts(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    world: Res<WorldState>,
    mut settings: ResMut<ViewSettings>,
    mut camera: MessageWriter<CameraAction>,
) {
    if !input.keyboard || shortcut_modifier_held(&keys) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let actions: Vec<ShortcutAction> = keys.get_just_pressed().filter_map(|k| match_shortcut(*k, shift)).collect();
    for action in actions {
        apply_shortcut(action, &mut settings, &world, &mut camera);
    }
}

/// Executes a shortcut action.
pub(crate) fn apply_shortcut(
    action: ShortcutAction,
    settings: &mut ViewSettings,
    world: &WorldState,
    camera: &mut MessageWriter<CameraAction>,
) {
    let layers = world.layers();
    let (lo, hi) = crate::settings::axis_range(layers, settings.cut.axis);
    match action {
        ShortcutAction::PresetAerial => {
            camera.write(CameraAction::Preset(OrbitPreset::Aerial));
        }
        ShortcutAction::PresetIsometric => {
            camera.write(CameraAction::Preset(OrbitPreset::Isometric));
        }
        ShortcutAction::PresetFreeOrbit => {
            camera.write(CameraAction::Preset(OrbitPreset::FreeOrbit));
        }
        ShortcutAction::PresetStrategic => {
            camera.write(CameraAction::Preset(OrbitPreset::Strategic));
        }
        ShortcutAction::FrameWorld => {
            camera.write(CameraAction::FrameWorld);
        }
        ShortcutAction::FramePopulation => {
            camera.write(CameraAction::FramePopulation);
        }
        ShortcutAction::CycleColor => settings.color_mode = settings.color_mode.next(),
        ShortcutAction::CycleTransparency => settings.transparency = settings.transparency.next(),
        ShortcutAction::ToggleCut => {
            let enabled = !settings.cut.enabled;
            settings.set_cut_enabled(enabled, layers);
        }
        ShortcutAction::CycleCutAxis => {
            let axis = next_axis(settings.cut.axis);
            settings.set_cut_axis(axis, layers);
            settings.set_cut_enabled(true, layers);
        }
        ShortcutAction::CutMaxDown | ShortcutAction::CutMaxUp => {
            settings.set_cut_enabled(true, layers);
            let delta = if action == ShortcutAction::CutMaxUp { 1 } else { -1 };
            settings.cut.nudge_max(delta, lo, hi);
        }
        ShortcutAction::CutMinDown | ShortcutAction::CutMinUp => {
            settings.set_cut_enabled(true, layers);
            let delta = if action == ShortcutAction::CutMinUp { 1 } else { -1 };
            settings.cut.nudge_min(delta, lo, hi);
        }
        ShortcutAction::ToggleBounds => settings.show_bounds = !settings.show_bounds,
        ShortcutAction::ToggleTurntable => settings.turntable = !settings.turntable,
        ShortcutAction::ToggleRegion => settings.region.enabled = !settings.region.enabled,
        ShortcutAction::ToolInspect => settings.tool = Tool::Inspect,
        ShortcutAction::ToolPlace => settings.tool = Tool::Place,
        ShortcutAction::ToolErase => settings.tool = Tool::Erase,
    }
}

/// A named action (tutorial buttons, smoke tests): `load:<scenario>`,
/// `probe:<x>/<y>/<z>` or a view action (see [`apply_view_action`]).
pub(crate) enum NamedAction {
    /// Load a scenario.
    Load(String),
    /// Select a voxel for the probe.
    Probe([i32; 3]),
    /// Anything that changes the view.
    View(String),
}

/// Splits tutorial / smoke-test action ids.
pub(crate) fn parse_action(id: &str) -> NamedAction {
    if let Some(scenario) = id.strip_prefix("load:") {
        return NamedAction::Load(scenario.to_string());
    }
    if let Some(coords) = id.strip_prefix("probe:") {
        let parts: Vec<i32> = coords.split('/').filter_map(|c| c.trim().parse().ok()).collect();
        if let [x, y, z] = parts[..] {
            return NamedAction::Probe([x, y, z]);
        }
    }
    NamedAction::View(id.to_string())
}

/// Applies a view action: `tab:<rules|metrics|parameters|properties>`,
/// `cut:<middle|off|sweep>`, `color:<state|age|height>`,
/// `view:<opaque|ghost|xray>`,
/// `camera:<front|side|top|corner|world|population|aerial|isometric|orbit|strategic>`,
/// `region:population`, `tool:<inspect|place|erase>`. Returns whether it was
/// recognised.
pub(crate) fn apply_view_action(
    id: &str,
    settings: &mut ViewSettings,
    world: &WorldState,
    camera: &mut MessageWriter<CameraAction>,
    shell: &mut MessageWriter<ShellCommand>,
) -> bool {
    let layers = world.layers();
    match id {
        "tab:rules" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Rules));
        }
        "tab:metrics" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Metrics));
        }
        "tab:parameters" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Parameters));
        }
        "tab:properties" => {
            shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
        }
        "cut:middle" => {
            settings.set_cut_axis(Axis::Z, layers);
            settings.set_cut_enabled(true, layers);
        }
        "cut:off" => settings.set_cut_enabled(false, layers),
        "cut:sweep" => {
            settings.set_cut_enabled(true, layers);
            settings.sweep = true;
        }
        "color:state" => settings.color_mode = ColorMode::State,
        "color:age" => settings.color_mode = ColorMode::Age,
        "color:height" => settings.color_mode = ColorMode::Height,
        "view:opaque" => settings.transparency = Transparency::Opaque,
        "view:ghost" => settings.transparency = Transparency::GhostDying,
        "view:xray" => settings.transparency = Transparency::XRay,
        "camera:front" => {
            camera.write(CameraAction::Front);
        }
        "camera:side" => {
            camera.write(CameraAction::Side);
        }
        "camera:top" => {
            camera.write(CameraAction::Top);
        }
        "camera:corner" => {
            camera.write(CameraAction::Corner);
        }
        "camera:world" => {
            camera.write(CameraAction::FrameWorld);
        }
        "camera:population" => {
            camera.write(CameraAction::FramePopulation);
        }
        "camera:aerial" => {
            camera.write(CameraAction::Preset(OrbitPreset::Aerial));
        }
        "camera:isometric" => {
            camera.write(CameraAction::Preset(OrbitPreset::Isometric));
        }
        "camera:orbit" => {
            camera.write(CameraAction::Preset(OrbitPreset::FreeOrbit));
        }
        "camera:strategic" => {
            camera.write(CameraAction::Preset(OrbitPreset::Strategic));
        }
        "region:population" => {
            if let Some(population) = world.population {
                settings.region.set(population);
            }
            settings.region.enabled = true;
        }
        "tool:inspect" => settings.tool = Tool::Inspect,
        "tool:place" => settings.tool = Tool::Place,
        "tool:erase" => settings.tool = Tool::Erase,
        _ => return false,
    }
    true
}

/// Loads a built-in scenario into the running session (`Control::Replace`).
pub(crate) fn load_scenario(
    id: &str,
    runner: &VoxelRunner,
    scenarios: &mut ModeScenarios,
    world: &mut WorldState,
    probe: &mut ProbeState,
) -> Result<(), String> {
    let life = crate::scenario_life(id)?;
    runner.0.send(Control::Replace(Box::new(life)));
    scenarios.mark_selected(MODE, id);
    world.invalidate();
    probe.pinned = None;
    Ok(())
}

/// Reacts to the top-bar scenario picker and to tutorial buttons.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_actions(
    mut picked: MessageReader<interface::ScenarioSelected>,
    mut tutorial: MessageReader<interface::TutorialAction>,
    runner: Res<VoxelRunner>,
    mut scenarios: ResMut<ModeScenarios>,
    mut world: ResMut<WorldState>,
    mut probe: ResMut<ProbeState>,
    mut settings: ResMut<ViewSettings>,
    mut toasts: ResMut<Toasts>,
    mut camera: MessageWriter<CameraAction>,
    mut shell: MessageWriter<ShellCommand>,
) {
    let ids: Vec<String> = picked
        .read()
        .filter(|s| s.mode == MODE)
        .map(|s| format!("load:{}", s.id))
        .chain(tutorial.read().map(|a| a.0.clone()))
        .collect();
    for id in ids {
        match parse_action(&id) {
            NamedAction::Load(scenario) => {
                if let Err(e) = load_scenario(&scenario, &runner, &mut scenarios, &mut world, &mut probe) {
                    toasts.error(e);
                }
            }
            NamedAction::Probe(pos) => probe.pinned = Some(pos),
            NamedAction::View(action) => {
                if !apply_view_action(&action, &mut settings, &world, &mut camera, &mut shell) {
                    warn!("3D Life: unknown action `{action}`");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_split_loads_from_view_changes() {
        assert!(matches!(parse_action("load:builder-1"), NamedAction::Load(id) if id == "builder-1"));
        assert!(matches!(parse_action("color:age"), NamedAction::View(id) if id == "color:age"));
        assert!(matches!(parse_action("probe:3/-4/5"), NamedAction::Probe([3, -4, 5])));
        assert!(matches!(parse_action("probe:3/4"), NamedAction::View(_)), "incomplete coordinates");
    }
}
