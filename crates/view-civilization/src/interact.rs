//! World interaction: hovering and selecting people and buildings, the
//! intervention tools, keyboard shortcuts and world-space highlights
//! (selection ring, route, relationships, tool radius preview).

use bevy::prelude::*;
use civilization::agent::Target;
use civilization::{CivCommand, Resource as Stock};
use rendering::{WorldInput, shortcut_modifier_held};
use simulation_core::Control;

use crate::coords::{HeightField, WATER_LEVEL, ray_point_distance, to_world};
use crate::people::{PeopleIndex, Person};
use crate::scene::{BuildingIndex, BuildingVisual, CivCamera, MapCache};
use crate::session::Selection;
use crate::state::{CivRunner, CurrentFrame, Tool, ViewState, apply_shortcut, shortcut_for};

/// What is under the cursor.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct Hover {
    /// Person under the cursor.
    pub person: Option<organism_engine::AgentId>,
    /// Ground point under the cursor.
    pub ground: Option<Vec3>,
}

/// Mouse press tracking (a click is a press and release without dragging).
#[derive(Resource, Default)]
pub struct ClickTracker {
    pressed_at: Option<Vec2>,
}

/// Largest cursor travel (logical px) between press and release for a click.
pub const CLICK_SLOP: f32 = 5.0;

/// Whether a press/release pair is a click rather than a camera drag.
pub fn is_click(pressed: Vec2, released: Vec2) -> bool {
    pressed.distance(released) <= CLICK_SLOP
}

/// Person hit by a ray: the closest to the camera among people whose body
/// axis passes within `radius` of the ray.
pub fn pick_person(
    origin: Vec3,
    dir: Vec3,
    people: impl Iterator<Item = (organism_engine::AgentId, Vec3, f32)>,
) -> Option<organism_engine::AgentId> {
    let mut best: Option<(f32, organism_engine::AgentId)> = None;
    for (id, chest, scale) in people {
        let (d, t) = ray_point_distance(origin, dir, chest);
        if t > 0.0 && d < 0.16 * scale + 0.05 && best.is_none_or(|(bt, _)| t < bt) {
            best = Some((t, id));
        }
    }
    best.map(|(_, id)| id)
}

/// Tile under a ground point (inside the map).
pub fn tile_of(point: Vec3, size: UVec2) -> Option<(u32, u32)> {
    let (x, y) = (point.x.floor(), point.z.floor());
    (x >= 0.0 && y >= 0.0 && x < size.x as f32 && y < size.y as f32).then_some((x as u32, y as u32))
}

/// The command a tool sends for a click at `point`.
pub fn tool_command(state: &ViewState, point: Vec3, size: UVec2) -> Option<CivCommand> {
    let (x, y) = tile_of(point, size)?;
    let radius = state.radius.clamp(0.5, civilization::command::MAX_RADIUS);
    match state.tool {
        Tool::Inspect => None,
        Tool::Spawn => Some(CivCommand::SpawnAgents {
            x: point.x,
            y: point.z,
            count: state.spawn_count.clamp(1, civilization::command::MAX_SPAWN),
            population: state.spawn_population,
        }),
        Tool::AddResource => Some(CivCommand::AddResources { x, y, radius, resource: state.add_resource }),
        Tool::RemoveResource => Some(CivCommand::RemoveResources { x, y, radius, resource: state.remove_resource }),
        Tool::Disaster => Some(CivCommand::Disaster { x, y, radius }),
    }
}

/// Keyboard shortcuts of the mode (no modifiers, keyboard not used by egui).
pub fn mode_shortcuts(input: Res<WorldInput>, keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<ViewState>) {
    if !input.keyboard || keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) || shortcut_modifier_held(&keys) {
        return;
    }
    for key in keys.get_just_pressed() {
        if let Some(action) = shortcut_for(*key) {
            apply_shortcut(&mut state, action);
        }
    }
}

fn cursor_ray(input: &WorldInput, camera: &Camera, transform: &GlobalTransform) -> Option<Ray3d> {
    let cursor = input.cursor?;
    camera.viewport_to_world(transform, cursor).ok()
}

/// Updates [`Hover`] and handles clicks in the world.
#[allow(clippy::too_many_arguments)]
pub fn world_pointer(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    camera: Option<Single<(&Camera, &GlobalTransform), With<CivCamera>>>,
    cache: Res<MapCache>,
    current: Res<CurrentFrame>,
    runner: Option<Res<CivRunner>>,
    people: Query<&Person>,
    mut hover: ResMut<Hover>,
    mut clicks: ResMut<ClickTracker>,
    mut state: ResMut<ViewState>,
) {
    let (Some(camera), Some(field)) = (camera, cache.field.as_ref()) else { return };
    let (camera, transform) = *camera;
    let ray = if input.pointer { cursor_ray(&input, camera, transform) } else { None };
    let next = match ray {
        Some(ray) => Hover {
            person: pick_person(ray.origin, *ray.direction, people.iter().map(|p| (p.id, p.chest(), p.scale()))),
            ground: field.ray_hit(ray.origin, *ray.direction),
        },
        None => Hover::default(),
    };
    if *hover != next {
        *hover = next;
    }
    if buttons.just_pressed(MouseButton::Left) {
        clicks.pressed_at = if input.pointer { input.cursor } else { None };
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let (Some(pressed), Some(released)) = (clicks.pressed_at.take(), input.cursor) else { return };
    if !input.pointer || !is_click(pressed, released) {
        return;
    }
    match state.tool {
        Tool::Inspect => {
            let selection = if let Some(id) = hover.person {
                Selection::Agent(id)
            } else if let (Some(point), Some(frame)) = (hover.ground, current.get()) {
                tile_of(point, cache.size)
                    .and_then(|tile| frame.view.buildings.iter().find(|b| b.tile == tile))
                    .map_or(Selection::None, |b| Selection::Building(b.id))
            } else {
                Selection::None
            };
            if state.selection != selection {
                state.selection = selection;
                state.follow = false;
            }
        }
        _ => {
            if let (Some(point), Some(runner)) = (hover.ground, runner)
                && let Some(command) = tool_command(&state, point, cache.size)
            {
                // Interventions are undoable edits.
                runner.0.send(Control::Edit(command));
            }
        }
    }
}

/// Polyline of a circle of `radius` around `centre` following the terrain.
pub fn ground_circle(field: &HeightField, centre: Vec2, radius: f32, segments: usize, lift: f32) -> Vec<Vec3> {
    (0..=segments)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / segments as f32;
            let p = centre + Vec2::new(a.cos(), a.sin()) * radius;
            to_world(p, field.height_at(p).max(WATER_LEVEL) + lift)
        })
        .collect()
}

/// Colour of a tool's radius preview.
pub fn tool_color(tool: Tool, accent: Color) -> Color {
    match tool {
        Tool::Inspect | Tool::Spawn => accent,
        Tool::AddResource => Color::srgb(0.45, 0.85, 0.45),
        Tool::RemoveResource => Color::srgb(0.95, 0.72, 0.3),
        Tool::Disaster => Color::srgb(0.95, 0.3, 0.28),
    }
}

/// Highlights drawn with gizmos: selection, hover, route, relationships and
/// the tool radius.
#[allow(clippy::too_many_arguments)]
pub fn draw_highlights(
    mut gizmos: Gizmos,
    state: Res<ViewState>,
    hover: Res<Hover>,
    cache: Res<MapCache>,
    current: Res<CurrentFrame>,
    people_index: Res<PeopleIndex>,
    buildings: Res<BuildingIndex>,
    people: Query<&Person>,
    visuals: Query<&BuildingVisual>,
    theme: Option<Res<interface::ThemeSettings>>,
    registry: Option<Res<interface::ModeRegistry>>,
) {
    let Some(field) = cache.field.as_ref() else { return };
    let accent = match (&theme, &registry) {
        (Some(theme), Some(registry)) => {
            interface::theme::bevy_color(interface::Palette::new(theme.kind, registry.accent(crate::MODE)).accent)
        }
        _ => Color::srgb(0.9, 0.75, 0.48),
    };
    let person = |id| people_index.0.get(&id).and_then(|e| people.get(*e).ok());
    if let Some(p) = hover.person.and_then(person)
        && state.tool == Tool::Inspect
        && state.selection != Selection::Agent(p.id)
    {
        gizmos.linestrip(ground_circle(field, p.drawn(), 0.2 * p.scale(), 20, 0.03), accent.with_alpha(0.55));
    }
    match state.selection {
        Selection::Agent(id) => {
            if let Some(p) = person(id) {
                let ring = ground_circle(field, p.drawn(), 0.24 * p.scale(), 28, 0.03);
                gizmos.linestrip(ring, accent);
                gizmos.linestrip(ground_circle(field, p.drawn(), 0.3 * p.scale(), 28, 0.03), accent.with_alpha(0.4));
                if let Some(detail) = current.get().and_then(|f| f.agent.as_ref()).filter(|d| d.id == id) {
                    // Planned route: waypoints are stored next-one-last.
                    let w = field.width();
                    let mut route = vec![to_world(p.drawn(), p.ground + 0.04)];
                    for &t in detail.agent.path.iter().rev() {
                        let c = Vec2::new((t % w) as f32 + 0.5, (t / w) as f32 + 0.5);
                        route.push(to_world(c, field.height_at(c).max(WATER_LEVEL) + 0.04));
                    }
                    if route.len() > 1 {
                        gizmos.linestrip(route, Color::srgba(1.0, 1.0, 1.0, 0.7));
                    }
                    let goal = match detail.agent.goal.target {
                        Target::Tile(t) => Some(Vec2::new((t % w) as f32 + 0.5, (t / w) as f32 + 0.5)),
                        Target::Agent(other) => person(other).map(|o| o.drawn()),
                        Target::Building(b) => buildings
                            .0
                            .get(&b)
                            .and_then(|e| visuals.get(*e).ok())
                            .map(|v| Vec2::new(v.tile.0 as f32 + 0.5, v.tile.1 as f32 + 0.5)),
                        Target::None => None,
                    };
                    if let Some(goal) = goal {
                        let colour = crate::palette::activity_style(detail.agent.goal.activity).1;
                        let c = Color::srgb_u8(colour[0], colour[1], colour[2]);
                        gizmos.linestrip(ground_circle(field, goal, 0.35, 24, 0.05), c);
                        gizmos.linestrip(ground_circle(field, goal, 0.22, 24, 0.05), c.with_alpha(0.6));
                    }
                    // Strongest relationships with people on the map.
                    for r in detail.relations.iter().take(8) {
                        if let Some(o) = person(r.other) {
                            let colour = if r.affinity >= 0.0 {
                                Color::srgba(0.45, 0.9, 0.55, 0.25 + 0.6 * r.affinity.abs())
                            } else {
                                Color::srgba(0.95, 0.35, 0.35, 0.25 + 0.6 * r.affinity.abs())
                            };
                            gizmos.line(p.chest(), o.chest(), colour);
                        }
                    }
                }
            }
        }
        Selection::Building(id) => {
            if let Some(v) = buildings.0.get(&id).and_then(|e| visuals.get(*e).ok()) {
                let c = Vec2::new(v.tile.0 as f32 + 0.5, v.tile.1 as f32 + 0.5);
                gizmos.linestrip(ground_circle(field, c, 0.55, 32, 0.04), accent);
            }
        }
        Selection::None => {}
    }
    if state.tool != Tool::Inspect
        && let Some(point) = hover.ground
    {
        let centre = Vec2::new(point.x, point.z);
        let radius = if state.tool == Tool::Spawn { 3.0 } else { state.radius };
        let colour = tool_color(state.tool, accent);
        gizmos.linestrip(ground_circle(field, centre, radius, 64, 0.06), colour);
        gizmos.linestrip(ground_circle(field, centre, 0.12, 12, 0.06), colour);
    }
}

/// Resources offered by the add tool (crops only grow on farm plots).
pub const ADDABLE: [Stock; 4] = [Stock::Berries, Stock::Fish, Stock::Wood, Stock::Stone];

#[cfg(test)]
mod tests {
    use super::*;
    use organism_engine::AgentId;

    #[test]
    fn clicks_tolerate_small_jitter_only() {
        assert!(is_click(Vec2::new(10.0, 10.0), Vec2::new(13.0, 12.0)));
        assert!(!is_click(Vec2::new(10.0, 10.0), Vec2::new(30.0, 10.0)), "a drag orbits the camera");
    }

    #[test]
    fn picking_prefers_the_nearest_person_on_the_ray() {
        let origin = Vec3::new(0.0, 5.0, 0.0);
        let dir = Vec3::NEG_Y;
        let people = vec![
            (AgentId(1), Vec3::new(0.05, 0.3, 0.0), 1.0),
            (AgentId(2), Vec3::new(0.0, 1.3, 0.0), 1.0),
            (AgentId(3), Vec3::new(2.0, 0.3, 0.0), 1.0),
        ];
        assert_eq!(pick_person(origin, dir, people.clone().into_iter()), Some(AgentId(2)));
        assert_eq!(pick_person(origin, dir, people[2..].iter().copied()), None, "too far from the ray");
        assert_eq!(pick_person(origin, Vec3::Y, people.into_iter()), None, "behind the camera");
    }

    #[test]
    fn tools_build_their_commands() {
        let size = UVec2::new(10, 8);
        let mut s = ViewState { tool: Tool::Disaster, radius: 3.0, ..ViewState::default() };
        assert_eq!(
            tool_command(&s, Vec3::new(4.6, 0.1, 2.2), size),
            Some(CivCommand::Disaster { x: 4, y: 2, radius: 3.0 })
        );
        assert_eq!(tool_command(&s, Vec3::new(14.0, 0.1, 2.2), size), None, "outside the map");
        s.tool = Tool::Spawn;
        s.spawn_count = 500;
        match tool_command(&s, Vec3::new(1.5, 0.0, 1.5), size) {
            Some(CivCommand::SpawnAgents { count, x, y, .. }) => {
                assert_eq!(count, civilization::command::MAX_SPAWN, "clamped to the model's limit");
                assert_eq!((x, y), (1.5, 1.5));
            }
            other => panic!("{other:?}"),
        }
        s.tool = Tool::RemoveResource;
        s.remove_resource = Some(Stock::Wood);
        s.radius = 1000.0;
        assert_eq!(
            tool_command(&s, Vec3::new(0.2, 0.0, 0.2), size),
            Some(CivCommand::RemoveResources {
                x: 0,
                y: 0,
                radius: civilization::command::MAX_RADIUS,
                resource: Some(Stock::Wood)
            })
        );
        s.tool = Tool::Inspect;
        assert_eq!(tool_command(&s, Vec3::new(0.2, 0.0, 0.2), size), None);
        assert_eq!(tile_of(Vec3::new(-0.1, 0.0, 1.0), size), None);
    }

    #[test]
    fn ground_circles_close_and_follow_the_terrain() {
        let terrain = vec![civilization::Terrain::Grassland; 16];
        let field = HeightField::from_terrain(&terrain, 4, 4);
        let ring = ground_circle(&field, Vec2::new(2.0, 2.0), 1.0, 16, 0.05);
        assert_eq!(ring.len(), 17);
        assert!((ring[0] - ring[16]).length() < 1e-4, "closed");
        for p in &ring {
            assert!((p.y - field.height_at(Vec2::new(p.x, p.z)) - 0.05).abs() < 1e-4);
        }
    }
}
