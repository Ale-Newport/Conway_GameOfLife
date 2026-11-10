//! World interaction: hover and click picking, area tools, keyboard
//! shortcuts and camera follow. Every world input is gated by
//! `rendering::WorldInput`.
//!
//! Mode shortcuts (only while the keyboard belongs to the world; none clash
//! with the shell's Space, N, R, Z, Y, =, −, Home, Tab, F1, F11, F12 or the
//! camera's W A S D, arrows, Q, E):
//!
//! | Key | Action |
//! |---|---|
//! | 1–5 | Select, Add food, Cull, Spawn genome, Predators tool |
//! | Esc | Back to Select, clear the selection |
//! | F | Follow the selected organism (toggle) |
//! | V | Next ground tint (temperature, food, regions, plain) |
//! | C | Next camera preset |
//! | [ / ] | Smaller / larger brush |
//! | G | Show the genealogy of the selection |

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use evolution::{EvolutionCommand, Shape, Species};
use interface::{RightPanelTab, ShellCommand};
use organism_engine::AgentId;
use rendering::{FollowTarget, OrbitPreset, WorldInput, shortcut_modifier_held};

use crate::organisms::{EvoOrganism, pick_along_ray};
use crate::scene::{EvoCamera, SceneState, scene_to_world};
use crate::session::{EvoData, EvoRunner};
use crate::ui_state::{EvoUi, PropertiesTab, Tool};

/// Mode shortcuts, for the help text in the tools panel (the mouse line
/// depends on the camera preset, see [`mouse_help`]).
pub const SHORTCUTS: [(&str, &str); 8] = [
    ("1 – 5", "Select · Food · Cull · Spawn · Predators tool"),
    ("Esc", "Select tool, clear selection"),
    ("F", "Follow the selected organism"),
    ("G", "Genealogy of the selection"),
    ("V", "Next ground tint"),
    ("C", "Next camera preset"),
    ("[  ]", "Brush smaller / larger"),
    ("WASD QE", "Move / turn the camera"),
];

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
        _ => "other",
    }
}

/// Mouse help for a camera preset: a click (press and release within a few
/// pixels) always applies the tool; drags belong to the camera.
pub fn mouse_help(preset: OrbitPreset) -> String {
    let cfg = preset.config();
    let locked = cfg.yaw.is_some() && cfg.pitch_limits.0.is_some() && cfg.pitch_limits.0 == cfg.pitch_limits.1;
    let orbit = if locked { String::new() } else { format!(" · {} drag: orbit", button_name(cfg.orbit_button)) };
    format!("Left click: tool{orbit} · {} drag: pan · wheel: zoom", button_name(cfg.pan_button))
}

/// Disc of radius `r` around `p`, clamped to stay finite and positive.
pub fn brush(p: Vec2, r: f32) -> Shape {
    Shape::Disc { x: p.x, y: p.y, r: r.max(0.5) }
}

/// The command a brush tool sends for a click at world position `p`
/// (`None` for the Select tool).
pub fn tool_command(ui: &EvoUi, p: Vec2) -> Option<EvolutionCommand> {
    let area = brush(p, ui.brush_radius);
    match ui.tool {
        Tool::Select => None,
        Tool::Food => Some(EvolutionCommand::AddFood { area, amount: ui.food_amount }),
        Tool::Cull => Some(EvolutionCommand::Cull {
            fraction: ui.cull_fraction,
            species: ui.cull_target.species(),
            area: Some(area),
        }),
        Tool::Spawn => Some(EvolutionCommand::Spawn {
            species: ui.genome.species,
            count: ui.genome.count,
            area: Some(area),
            traits: ui.genome_trait_values(),
            spread: ui.genome.spread,
        }),
        Tool::Predators => Some(EvolutionCommand::IntroducePredators {
            count: ui.predator_count,
            area: Some(area),
            traits: ui.predator_trait_values(),
        }),
    }
}

/// Where the cursor ray meets the ground (`y ≈ 0`), in world units.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct CursorGround {
    /// World position under the cursor, if over the world.
    pub world: Option<Vec2>,
    press: Option<Vec2>,
}

/// Hover picking and ground position.
pub fn hover(
    input: Res<WorldInput>,
    camera: Query<(&Camera, &GlobalTransform), With<EvoCamera>>,
    organisms: Query<(&EvoOrganism, &GlobalTransform, &Visibility)>,
    mut ground: ResMut<CursorGround>,
    mut ui: ResMut<EvoUi>,
) {
    let ray = input
        .pointer
        .then_some(input.cursor)
        .flatten()
        .and_then(|cursor| camera.iter().next().and_then(|(c, t)| c.viewport_to_world(t, cursor).ok()));
    let Some(ray) = ray else {
        ground.world = None;
        if ui.hovered.is_some() {
            ui.hovered = None;
        }
        return;
    };
    ground.world =
        ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)).map(|d| scene_to_world(ray.get_point(d)));
    let mut ids: Vec<AgentId> = Vec::new();
    let mut candidates: Vec<(Vec3, f32)> = Vec::new();
    for (evo, transform, visibility) in &organisms {
        if *visibility == Visibility::Hidden {
            continue;
        }
        let size = if evo.species == Species::Prey { 0.32 } else { 0.55 } * evo.scale;
        ids.push(evo.id);
        candidates.push((transform.translation() + Vec3::Y * size * 0.8, size.max(0.35)));
    }
    let hit = pick_along_ray(ray.origin, *ray.direction, &candidates).map(|i| ids[i]);
    if ui.hovered != hit {
        ui.hovered = hit;
    }
}

/// Left click: select (Select tool) or apply the brush tool.
pub fn click(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    runner: Option<Res<EvoRunner>>,
    mut ground: ResMut<CursorGround>,
    mut ui: ResMut<EvoUi>,
    mut shell: MessageWriter<ShellCommand>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        ground.press = if input.pointer { input.cursor } else { None };
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let Some(press) = ground.press.take() else { return };
    // A drag is a camera gesture, not a click.
    if input.cursor.is_none_or(|c| c.distance(press) > 5.0) || !input.pointer {
        return;
    }
    match ui.tool {
        Tool::Select => {
            ui.selected = ui.hovered;
            if ui.selected.is_some() {
                ui.tab = PropertiesTab::Inspector;
                shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
            } else {
                ui.follow = false;
            }
        }
        _ => {
            if let (Some(p), Some(runner)) = (ground.world, runner)
                && let Some(command) = tool_command(&ui, p)
            {
                runner.0.edit(command);
            }
        }
    }
}

/// Mode keyboard shortcuts.
pub fn shortcuts(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut ui: ResMut<EvoUi>,
    mut scene: ResMut<SceneState>,
    mut shell: MessageWriter<ShellCommand>,
) {
    if !input.keyboard || shortcut_modifier_held(&keys) {
        return;
    }
    const DIGITS: [KeyCode; 5] = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5];
    for (key, tool) in DIGITS.iter().zip(Tool::ALL) {
        if keys.just_pressed(*key) {
            ui.tool = tool;
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        ui.tool = Tool::Select;
        ui.selected = None;
        ui.follow = false;
    }
    if keys.just_pressed(KeyCode::KeyF) && ui.selected.is_some() {
        ui.follow = !ui.follow;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        ui.tab = PropertiesTab::Genealogy;
        shell.write(ShellCommand::ShowRightTab(RightPanelTab::Properties));
    }
    if keys.just_pressed(KeyCode::KeyV) {
        ui.overlay = ui.overlay.next();
    }
    if keys.just_pressed(KeyCode::KeyC) {
        ui.cycle_camera();
        scene.frame_request = false;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        ui.brush_radius = (ui.brush_radius / 1.25).max(1.0);
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        ui.brush_radius = (ui.brush_radius * 1.25).min(40.0);
    }
}

/// Camera follow of the selected organism.
pub fn follow(
    mut commands: Commands,
    mut ui: ResMut<EvoUi>,
    index: Res<crate::organisms::OrganismIndex>,
    cameras: Query<(Entity, Option<&FollowTarget>), With<EvoCamera>>,
) {
    let target = if ui.follow { ui.selected.and_then(|id| index.map.get(&id).copied()) } else { None };
    if ui.follow && target.is_none() && ui.selected.is_some_and(|id| !index.map.contains_key(&id)) {
        // The organism died: stop following.
        ui.follow = false;
    }
    for (camera, current) in &cameras {
        match (target, current) {
            (Some(e), Some(f)) if f.subject == rendering::FollowSubject::Entity(e) => {}
            (Some(e), _) => {
                commands.entity(camera).insert(FollowTarget { half_life: 0.3, ..FollowTarget::entity(e) });
            }
            (None, Some(_)) => {
                commands.entity(camera).remove::<FollowTarget>();
            }
            (None, None) => {}
        }
    }
}

/// Brush preview on the ground.
pub fn draw_brush(ui: Res<EvoUi>, ground: Res<CursorGround>, scene: Res<SceneState>, mut gizmos: Gizmos) {
    if !ui.tool.uses_brush() {
        return;
    }
    let Some(p) = ground.world else { return };
    let color = match ui.tool {
        Tool::Food => Color::srgb(0.45, 0.85, 0.35),
        Tool::Cull => Color::srgb(0.95, 0.35, 0.35),
        Tool::Spawn => Color::srgb(0.45, 0.75, 1.0),
        Tool::Predators => Color::srgb(0.95, 0.65, 0.25),
        Tool::Select => Color::WHITE,
    };
    let y = scene.field.sample(p).max(0.0) + 0.08;
    let iso = Isometry3d::new(Vec3::new(p.x, y, p.y), Quat::from_rotation_x(FRAC_PI_2));
    gizmos.circle(iso, ui.brush_radius, color);
    gizmos.circle(iso, ui.brush_radius * 0.98, color.with_alpha(0.4));
}

/// Clears a selection whose organism no longer exists.
pub fn prune_selection(data: Res<EvoData>, mut ui: ResMut<EvoUi>) {
    let Some(view) = data.view.as_ref() else { return };
    if !data.view_changed {
        return;
    }
    if let Some(h) = ui.hovered
        && view.organisms.binary_search_by_key(&h, |o| o.id).is_err()
    {
        ui.hovered = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_build_the_matching_commands() {
        let mut ui = EvoUi::default();
        assert!(tool_command(&ui, Vec2::ZERO).is_none());
        ui.tool = Tool::Food;
        ui.brush_radius = 3.0;
        match tool_command(&ui, Vec2::new(4.0, 5.0)) {
            Some(EvolutionCommand::AddFood { area: Shape::Disc { x, y, r }, amount }) => {
                assert_eq!((x, y, r, amount), (4.0, 5.0, 3.0, ui.food_amount));
            }
            other => panic!("{other:?}"),
        }
        ui.tool = Tool::Cull;
        assert!(matches!(
            tool_command(&ui, Vec2::ZERO),
            Some(EvolutionCommand::Cull { species: Some(Species::Prey), area: Some(_), .. })
        ));
        ui.tool = Tool::Spawn;
        match tool_command(&ui, Vec2::ZERO) {
            Some(EvolutionCommand::Spawn { traits, count, .. }) => {
                assert_eq!(traits.len(), evolution::genes::HERITABLE_COUNT);
                assert_eq!(count, ui.genome.count);
            }
            other => panic!("{other:?}"),
        }
        ui.tool = Tool::Predators;
        assert!(matches!(tool_command(&ui, Vec2::ZERO), Some(EvolutionCommand::IntroducePredators { .. })));
        assert!(matches!(brush(Vec2::ZERO, -3.0), Shape::Disc { r, .. } if r > 0.0));
    }

    #[test]
    fn mouse_help_follows_the_camera_preset() {
        assert!(mouse_help(OrbitPreset::Strategic).contains("middle drag: orbit"));
        assert!(mouse_help(OrbitPreset::FreeOrbit).contains("left drag: orbit"));
        assert!(!mouse_help(OrbitPreset::Isometric).contains("orbit"), "isometric angles are locked");
        for preset in OrbitPreset::ALL {
            assert!(mouse_help(preset).starts_with("Left click: tool"));
        }
    }

    #[test]
    fn commands_from_tools_are_accepted_by_the_model() {
        let mut scenario = evolution::scenarios::find_builtin("speed_selection").unwrap();
        scenario.config.world.width = 30;
        scenario.config.world.height = 20;
        scenario.config.prey.initial = 30;
        let mut sim = evolution::EvolutionSim::from_scenario(&scenario).unwrap();
        let mut ui = EvoUi::default();
        for tool in [Tool::Food, Tool::Cull, Tool::Spawn, Tool::Predators] {
            ui.tool = tool;
            let command = tool_command(&ui, Vec2::new(15.0, 10.0)).unwrap();
            simulation_core::Simulation::apply(&mut sim, command).unwrap_or_else(|e| panic!("{tool:?}: {e}"));
        }
    }
}
