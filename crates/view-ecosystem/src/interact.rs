//! World interaction: picking animals and trees (screen-space nearest
//! within their drawn size, then the nearest animal within a radius of the
//! clicked ground), hover read-outs, the intervention tools (every one a
//! model command), keyboard shortcuts, and the gizmos for the selection,
//! the followed animal's trail, territories and the tool brush.

use std::sync::Mutex;
use std::sync::mpsc;

use bevy::prelude::*;
use ecosystem::{EcosystemCommand, OrganismRef};
use interface::Toasts;
use organism_render::{AnimatedOrganism, OrganismCatalog};
use physics_world::Disaster;
use rendering::WorldInput;
use simulation_core::Control;
use simulation_core::glam::Vec2 as SimVec2;

use crate::palette::{Overlay, category_color};
use crate::scene::EcoCamera;
use crate::scene::fauna::EcoAnimal;
use crate::scene::flora::EcoTree;
use crate::scene::terrain::MapState;
use crate::session::EcoSession;
use crate::state::{CameraMode, EcoData, EcoRunner, EcoUi, PendingJobs, PropertiesTab, Tool};

/// Click tolerance (px): a press and release closer than this is a click.
pub const CLICK_SLOP_PX: f32 = 5.0;
/// Smallest pick radius on screen (px).
pub const MIN_PICK_PX: f32 = 9.0;
/// Trees farther than this (m) from the camera are not picked.
pub const TREE_PICK_DISTANCE: f32 = 900.0;
/// Radius (cells) of the nearest-animal fallback around a clicked point.
pub const NEAREST_RADIUS_CELLS: f32 = 1.5;

/// What picking reads from the camera.
type CameraView = (&'static Camera, &'static GlobalTransform, &'static Projection);

/// Gizmos of this mode (own depth bias and line width).
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct EcoGizmos;

/// What the pointer is over.
#[derive(Resource, Debug, Default, Clone)]
pub struct Hover {
    /// Organism under the pointer.
    pub organism: Option<OrganismRef>,
    /// Ground point under the pointer (scene).
    pub ground: Option<Vec3>,
    /// Window position of the pointer.
    pub cursor: Option<Vec2>,
}

/// Press position of the left button (click detection).
#[derive(Resource, Debug, Default)]
pub struct ClickTracker {
    press: Option<Vec2>,
}

/// A pickable thing on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// What it is.
    pub organism: OrganismRef,
    /// Window position (logical px).
    pub screen: Vec2,
    /// Distance from the camera (m).
    pub depth: f32,
    /// Drawn radius on screen (px).
    pub radius_px: f32,
}

/// The candidate under `cursor`: within its drawn radius (at least
/// [`MIN_PICK_PX`]), closest relative to that radius; animals win over trees
/// at equal score.
pub fn pick(cursor: Vec2, candidates: &[Candidate]) -> Option<OrganismRef> {
    candidates
        .iter()
        .filter_map(|c| {
            let r = c.radius_px.max(MIN_PICK_PX);
            let d = c.screen.distance(cursor);
            (d <= r).then(|| {
                let tree_penalty = if matches!(c.organism, OrganismRef::Tree(_)) { 0.75 } else { 0.0 };
                (d / r + tree_penalty + c.depth * 1e-6, c.organism)
            })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, o)| o)
}

/// Nearest of `points` (cells) to `target` within `radius` cells.
pub fn nearest_within(target: SimVec2, points: impl Iterator<Item = (u64, SimVec2)>, radius: f32) -> Option<u64> {
    points
        .map(|(id, p)| (p.distance(target), id))
        .filter(|(d, _)| *d <= radius)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// Whether a sphere of `radius_m` (at least the minimum pick size on
/// screen) around `centre` comes near the ray from `origin` along the unit
/// `direction`, in front of it (a generous pre-filter of [`pick`]).
pub fn near_ray(origin: Vec3, direction: Vec3, centre: Vec3, radius_m: f32, fov: f32, viewport_height: f32) -> bool {
    let rel = centre - origin;
    let along = rel.dot(direction);
    if along <= 0.0 {
        return false;
    }
    let off = (rel - direction * along).length();
    off <= 1.5 * radius_m.max(MIN_PICK_PX / px_per_metre(along, fov, viewport_height))
}

/// Pixels per metre at `depth` for a perspective camera.
fn px_per_metre(depth: f32, fov: f32, viewport_height: f32) -> f32 {
    viewport_height * 0.5 / (depth.max(0.1) * (fov * 0.5).tan())
}

/// Hover detection and clicks (selection or the active tool).
#[allow(clippy::too_many_arguments)]
pub fn world_pointer(
    input: Res<WorldInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    data: Res<EcoData>,
    map: Res<MapState>,
    catalog: Res<OrganismCatalog>,
    runner: Option<Res<EcoRunner>>,
    mut ui: ResMut<EcoUi>,
    mut hover: ResMut<Hover>,
    mut click: ResMut<ClickTracker>,
    mut toasts: ResMut<Toasts>,
    mut jobs: ResMut<PendingJobs>,
    camera: Option<Single<CameraView, With<EcoCamera>>>,
    animals: Query<(&EcoAnimal, &GlobalTransform, &AnimatedOrganism, &InheritedVisibility)>,
    trees: Query<(&EcoTree, &GlobalTransform, &InheritedVisibility)>,
) {
    let (Some(camera), Some(geometry)) = (camera, map.geometry.as_ref()) else { return };
    let (cam, cam_gt, projection) = *camera;
    hover.cursor = input.cursor;
    if !input.pointer {
        hover.organism = None;
        hover.ground = None;
        if buttons.just_released(MouseButton::Left) {
            click.press = None;
        }
        return;
    }
    let Some(cursor) = input.cursor else { return };
    let Ok(ray) = cam.viewport_to_world(cam_gt, cursor) else { return };
    hover.ground = geometry.raycast(ray.origin, *ray.direction);
    // Screen-space candidates.
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => std::f32::consts::FRAC_PI_4,
    };
    let vh = cam.logical_viewport_size().map_or(800.0, |s| s.y);
    let eye = cam_gt.translation();
    // Only things whose drawn size (or the minimum pick size) reaches the
    // pointer ray are projected: thousands of trees and animals per frame
    // otherwise.
    let near_ray = |centre: Vec3, radius_m: f32| near_ray(ray.origin, *ray.direction, centre, radius_m, fov, vh);
    let mut candidates = Vec::new();
    if ui.layers.animals {
        for (a, gt, org, vis) in &animals {
            if !vis.get() {
                continue;
            }
            let extent = catalog.extent(&org.species) * org.effective_age_scale();
            let centre = gt.translation() + Vec3::Y * extent * 0.3;
            if !near_ray(centre, 0.6 * extent) {
                continue;
            }
            if let Ok(screen) = cam.world_to_viewport(cam_gt, centre) {
                let depth = eye.distance(centre);
                let radius_px = 0.6 * extent * px_per_metre(depth, fov, vh);
                candidates.push(Candidate { organism: OrganismRef::Animal(a.id), screen, depth, radius_px });
            }
        }
    }
    if ui.layers.trees {
        for (t, gt, vis) in &trees {
            // Distant crowns are a few pixels: not worth projecting.
            if !vis.get() || gt.translation().distance(eye) > TREE_PICK_DISTANCE {
                continue;
            }
            let centre = gt.translation() + Vec3::Y * t.height * 0.6;
            if !near_ray(centre, t.crown.max(t.height * 0.2)) {
                continue;
            }
            if let Ok(screen) = cam.world_to_viewport(cam_gt, centre) {
                let depth = eye.distance(centre);
                let radius_px = t.crown.max(t.height * 0.2) * px_per_metre(depth, fov, vh);
                candidates.push(Candidate { organism: OrganismRef::Tree(t.id), screen, depth, radius_px });
            }
        }
    }
    hover.organism = pick(cursor, &candidates);
    if hover.organism.is_none()
        && let (Some(ground), Some(frame)) = (hover.ground, data.get())
    {
        let target = geometry.plane_to_cell(ground.x, ground.z);
        hover.organism = nearest_within(
            SimVec2::new(target.x, target.y),
            frame.view.animals.iter().filter(|a| !a.hidden).map(|a| (a.id, a.position)),
            NEAREST_RADIUS_CELLS,
        )
        .map(OrganismRef::Animal);
    }
    // Clicks: press and release without dragging (a drag orbits the camera).
    if buttons.just_pressed(MouseButton::Left) {
        click.press = Some(cursor);
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let Some(press) = click.press.take() else { return };
    if press.distance(cursor) > CLICK_SLOP_PX {
        return;
    }
    let Some(runner) = runner else { return };
    if ui.tool == Tool::Inspect {
        select(&runner, &mut ui, hover.organism);
        return;
    }
    let (Some(ground), Some(frame)) = (hover.ground, data.get()) else {
        toasts.warning("Click on the map to apply the tool.");
        return;
    };
    let cell = geometry.plane_to_cell(ground.x, ground.z);
    let cell = SimVec2::new(cell.x, cell.y);
    let (w, h) = frame.info.size;
    let (cx, cy) = ((cell.x.max(0.0) as usize).min(w - 1), (cell.y.max(0.0) as usize).min(h - 1));
    let s = ui.settings.clone();
    match ui.tool {
        Tool::Inspect => {}
        Tool::Cull => {
            let ids: Vec<u64> = frame
                .view
                .animals
                .iter()
                .filter(|a| s.cull_species.is_none_or(|sp| sp == a.species))
                .filter(|a| a.position.distance(SimVec2::new(cell.x, cell.y)) <= s.radius.max(0.5))
                .map(|a| a.id)
                .collect();
            if ids.is_empty() {
                toasts.warning("No animal of that kind within the brush.");
            } else {
                let label = s
                    .cull_species
                    .and_then(|sp| frame.info.catalog.animals().get(usize::from(sp)))
                    .map_or("animals".to_string(), |sp| sp.name.clone());
                cull(&runner, &mut jobs, ids, label, frame.info.netlogo.is_none());
            }
        }
        Tool::Tornado if s.tornado_start.is_none() => {
            ui.settings.tornado_start = Some(cell);
            toasts.info("Tornado: click where the path ends.");
        }
        tool => {
            if tool == Tool::Tornado {
                ui.settings.tornado_start = None;
            }
            if let Some(command) = tool_command(tool, &s, cell, (w, h)) {
                runner.0.send(Control::Edit(command));
                match tool {
                    Tool::Spawn => {
                        let name = frame
                            .info
                            .catalog
                            .animal(&s.spawn_species)
                            .map_or(s.spawn_species.clone(), |sp| sp.name.clone());
                        toasts.info(format!("Adding {} × {name} at cell ({cx}, {cy}).", s.spawn_count));
                    }
                    Tool::Ignite => {
                        toasts.info(format!("Ignition at cell ({cx}, {cy}); it spreads only through dry fuel."))
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The model command a click tool sends at a model position (cells) of a
/// `width × height` world; `None` for tools that are not a single command
/// (inspect, cull, the first click of a tornado).
pub fn tool_command(
    tool: Tool,
    s: &crate::state::ToolSettings,
    cell: SimVec2,
    (w, h): (usize, usize),
) -> Option<EcosystemCommand> {
    let (cx, cy) =
        ((cell.x.max(0.0) as usize).min(w.saturating_sub(1)), (cell.y.max(0.0) as usize).min(h.saturating_sub(1)));
    Some(match tool {
        Tool::Inspect | Tool::Cull => return None,
        Tool::Spawn => EcosystemCommand::SpawnAnimals {
            species: s.spawn_species.clone(),
            x: cell.x,
            y: cell.y,
            radius: s.radius,
            count: s.spawn_count,
        },
        Tool::PlantTree => EcosystemCommand::PlantTree { species: s.tree_species.clone(), x: cell.x, y: cell.y },
        Tool::Ignite => EcosystemCommand::Ignite { x: cx, y: cy },
        Tool::Flood => EcosystemCommand::Disaster(Disaster::Flood {
            x: cx,
            y: cy,
            radius: s.radius.max(1.0),
            depth_m: s.flood_depth_m,
        }),
        Tool::Earthquake => EcosystemCommand::Disaster(Disaster::Earthquake {
            x: cx,
            y: cy,
            radius: s.radius.max(1.0),
            magnitude: s.magnitude,
        }),
        Tool::Landslide => EcosystemCommand::Disaster(Disaster::Landslide { x: cx, y: cy, radius: s.radius.max(1.0) }),
        Tool::Volcano => EcosystemCommand::Disaster(Disaster::Volcano {
            x: cx,
            y: cy,
            radius: s.radius.max(2.0),
            height_m: s.volcano_height_m,
        }),
        Tool::Tornado => {
            let start = s.tornado_start?;
            EcosystemCommand::Disaster(Disaster::Tornado {
                path: vec![[start.x, start.y], [cell.x, cell.y]],
                width: s.tornado_width,
            })
        }
    })
}

/// Kills the given animals in one undoable edit and reports the result.
pub fn cull(runner: &EcoRunner, jobs: &mut PendingJobs, ids: Vec<u64>, label: String, carcasses: bool) {
    let (tx, rx) = mpsc::channel();
    let n = ids.len();
    runner.0.send(Control::ExecEdit(Box::new(move |session: &mut EcoSession| {
        let killed = ids.into_iter().filter(|&id| session.command(EcosystemCommand::KillAnimal { id }).is_ok()).count();
        let _ = tx.send(killed);
    })));
    let fate = if carcasses { " (they become carcasses)" } else { "" };
    jobs.culls.push((format!("{label}{fate}"), n, Mutex::new(rx)));
}

/// Selects an organism (or clears the selection) in the model and the view.
pub fn select(runner: &EcoRunner, ui: &mut EcoUi, organism: Option<OrganismRef>) {
    runner.0.send(Control::Command(EcosystemCommand::Select(organism)));
    ui.selection = organism;
    // The Species tab shows the newly selected animal's species.
    if matches!(organism, Some(OrganismRef::Animal(_))) {
        ui.species = None;
    }
    if organism.is_some() && ui.tab != PropertiesTab::Species {
        ui.tab = PropertiesTab::Inspector;
    }
    if organism.is_none() && ui.camera == CameraMode::Follow {
        ui.camera = CameraMode::Orbit;
    }
}

/// Keeps the model's follow state (trail and decision log) in step with the
/// camera mode and the selection; the camera stops following a dead animal.
pub fn sync_follow(
    runner: Option<Res<EcoRunner>>,
    data: Res<EcoData>,
    mut ui: ResMut<EcoUi>,
    mut last: Local<Option<(CameraMode, Option<OrganismRef>)>>,
) {
    let Some(runner) = runner else { return };
    let state = (ui.camera, ui.selection);
    if *last != Some(state) {
        *last = Some(state);
        // The camera tracks the selected animal in Follow and Cinematic mode;
        // the model then keeps its trail and logs its decisions.
        let follow = match ui.selection {
            Some(OrganismRef::Animal(_)) if matches!(ui.camera, CameraMode::Follow | CameraMode::Cinematic) => {
                ui.selection
            }
            _ => None,
        };
        runner.0.send(Control::Command(EcosystemCommand::Follow(follow)));
    }
    // A dead animal's camera stops following it.
    if ui.camera == CameraMode::Follow
        && data.fresh
        && let (Some(OrganismRef::Animal(id)), Some(frame)) = (ui.selection, data.get())
        && !frame.view.animals.iter().any(|a| a.id == id)
    {
        ui.camera = CameraMode::Orbit;
    }
}

/// Reports finished culls.
pub fn poll_jobs(mut jobs: ResMut<PendingJobs>, mut toasts: ResMut<Toasts>) {
    jobs.culls.retain(|(label, requested, rx)| {
        let result = rx.lock().ok().and_then(|r| r.try_recv().ok());
        match result {
            Some(killed) => {
                toasts.info(format!("Culled {killed} of {requested} {label}."));
                false
            }
            None => true,
        }
    });
}

/// Mode shortcuts: Escape clears the selection and the tool, F follows,
/// C cinematic, G ground level, T top-down, O orbit, M frames the map.
pub fn mode_shortcuts(
    input: Res<WorldInput>,
    keys: Res<ButtonInput<KeyCode>>,
    runner: Option<Res<EcoRunner>>,
    mut ui: ResMut<EcoUi>,
) {
    if !input.keyboard || rendering::shortcut_modifier_held(&keys) {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        if ui.tool != Tool::Inspect {
            ui.tool = Tool::Inspect;
            ui.settings.tornado_start = None;
        } else if let Some(runner) = &runner {
            select(runner, &mut ui, None);
        }
    }
    let mode = [
        (KeyCode::KeyF, CameraMode::Follow),
        (KeyCode::KeyC, CameraMode::Cinematic),
        (KeyCode::KeyG, CameraMode::Ground),
        (KeyCode::KeyT, CameraMode::TopDown),
        (KeyCode::KeyO, CameraMode::Orbit),
    ]
    .into_iter()
    .find(|(k, _)| keys.just_pressed(*k))
    .map(|(_, m)| m);
    if let Some(mode) = mode {
        ui.camera =
            if mode == CameraMode::Follow && ui.camera == CameraMode::Follow { CameraMode::Orbit } else { mode };
    }
    if keys.just_pressed(KeyCode::KeyM) {
        ui.frame_map = true;
    }
}

/// Points on a ground circle (for gizmos), following the terrain.
fn ground_circle(map: &MapState, centre: Vec3, radius: f32, segments: usize) -> Vec<Vec3> {
    let Some(g) = map.geometry.as_ref() else { return Vec::new() };
    (0..=segments)
        .map(|k| {
            let a = k as f32 / segments as f32 * std::f32::consts::TAU;
            let (x, z) = (centre.x + radius * a.cos(), centre.z + radius * a.sin());
            Vec3::new(x, g.surface(x, z) + 0.3, z)
        })
        .collect()
}

/// Selection and hover rings, the followed animal's trail, territories and
/// the tool brush.
#[allow(clippy::too_many_arguments)]
pub fn draw_gizmos(
    mut gizmos: Gizmos<EcoGizmos>,
    ui: Res<EcoUi>,
    hover: Res<Hover>,
    data: Res<EcoData>,
    map: Res<MapState>,
    time: Res<Time<Real>>,
    animals: Query<(&EcoAnimal, &GlobalTransform, &AnimatedOrganism)>,
    trees: Query<(&EcoTree, &GlobalTransform)>,
    catalog: Res<OrganismCatalog>,
    theme: Res<interface::ThemeSettings>,
) {
    let (Some(frame), Some(geometry)) = (data.get(), map.geometry.as_ref()) else { return };
    let accent = interface::theme::bevy_color(interface::Palette::new(theme.kind, crate::MODE.default_accent()).accent);
    let ring = |org: OrganismRef| -> Option<(Vec3, f32)> {
        match org {
            OrganismRef::Animal(id) => animals.iter().find(|(a, _, _)| a.id == id).map(|(_, gt, o)| {
                (gt.translation(), (catalog.extent(&o.species) * o.effective_age_scale() * 0.7).max(0.6))
            }),
            OrganismRef::Tree(id) => {
                trees.iter().find(|(t, _)| t.id == id).map(|(t, gt)| (gt.translation(), t.crown.max(1.0)))
            }
        }
    };
    if ui.layers.highlights {
        let pulse = 1.0 + 0.08 * (time.elapsed_secs() * 4.0).sin();
        if let Some((p, r)) = ui.selection.and_then(ring) {
            let pts = ground_circle(&map, Vec3::new(p.x, 0.0, p.z), r * pulse, 40);
            gizmos.linestrip(pts, accent);
            // A plumb line to the surface for fliers and divers.
            let surface = geometry.surface(p.x, p.z);
            if p.y > surface + 2.0 || p.y < surface - 1.0 {
                gizmos.line(p, Vec3::new(p.x, surface, p.z), accent.with_alpha(0.6));
            }
        }
        if hover.organism != ui.selection
            && let Some((p, r)) = hover.organism.and_then(ring)
        {
            gizmos.linestrip(ground_circle(&map, Vec3::new(p.x, 0.0, p.z), r, 32), Color::srgba(1.0, 1.0, 1.0, 0.6));
        }
        if frame.view.trail.len() > 1 {
            let pts: Vec<Vec3> = frame
                .view
                .trail
                .iter()
                .map(|c| {
                    let p = geometry.cell_to_plane(*c);
                    Vec3::new(p.x, geometry.surface(p.x, p.y) + 0.4, p.y)
                })
                .collect();
            gizmos.linestrip(pts, accent.with_alpha(0.8));
        }
    }
    if ui.overlay == Overlay::Territories {
        // Pack and pair members share a home: each territory is drawn once.
        let mut drawn = std::collections::BTreeSet::new();
        let pole = (geometry.extent().max_element() * 0.02).clamp(6.0, 60.0);
        for t in &frame.territories {
            let key = (t.species, (t.centre.x * 4.0).round() as i64, (t.centre.y * 4.0).round() as i64);
            if !drawn.insert(key) {
                continue;
            }
            let c = category_color(usize::from(t.species));
            let color = Color::srgba(c[0], c[1], c[2], 0.9);
            let p = geometry.cell_to_plane(t.centre);
            let centre = Vec3::new(p.x, geometry.surface(p.x, p.y), p.y);
            // The home: a pole and a ring, visible even when the boundary
            // lies beyond the map.
            gizmos.line(centre, centre + Vec3::Y * pole, color);
            gizmos.linestrip(ground_circle(&map, centre, geometry.cell_size, 24), color);
            let pts = ground_circle(&map, centre, t.radius * geometry.cell_size, 96);
            // Only the arcs over the map (large territories reach past its edges).
            for run in pts.split(|q| !geometry.contains(q.x, q.z)).filter(|r| r.len() > 1) {
                gizmos.linestrip(run.iter().copied(), color);
            }
        }
    }
    if ui.tool != crate::state::Tool::Inspect
        && let Some(g) = hover.ground
    {
        let r = if ui.tool.has_radius() {
            ui.settings.radius.max(0.5) * geometry.cell_size
        } else {
            geometry.cell_size * 0.5
        };
        gizmos.linestrip(ground_circle(&map, g, r, 48), Color::srgb(1.0, 0.85, 0.3));
        if ui.tool == Tool::Tornado
            && let Some(start) = ui.settings.tornado_start
        {
            let p = geometry.cell_to_plane(start);
            let a = Vec3::new(p.x, geometry.surface(p.x, p.y) + 1.0, p.y);
            gizmos.line(a, g + Vec3::Y, Color::srgb(1.0, 0.5, 0.3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(organism: OrganismRef, x: f32, depth: f32, radius_px: f32) -> Candidate {
        Candidate { organism, screen: Vec2::new(x, 0.0), depth, radius_px }
    }

    #[test]
    fn picking_prefers_the_closest_within_its_size() {
        let a = OrganismRef::Animal(1);
        let b = OrganismRef::Animal(2);
        let t = OrganismRef::Tree(3);
        // Cursor at x = 10: animal b (centre 12, radius 20) beats a (centre 0, radius 12).
        assert_eq!(pick(Vec2::new(10.0, 0.0), &[c(a, 0.0, 10.0, 12.0), c(b, 12.0, 20.0, 20.0)]), Some(b));
        // Nothing within reach.
        assert_eq!(pick(Vec2::new(100.0, 0.0), &[c(a, 0.0, 10.0, 12.0)]), None);
        // Tiny animals still get the minimum radius.
        assert_eq!(pick(Vec2::new(7.0, 0.0), &[c(a, 0.0, 10.0, 0.5)]), Some(a));
        // An animal in front of a tree crown wins.
        assert_eq!(pick(Vec2::new(5.0, 0.0), &[c(t, 0.0, 50.0, 80.0), c(a, 9.0, 60.0, 10.0)]), Some(a));
        assert_eq!(pick(Vec2::new(5.0, 0.0), &[c(t, 0.0, 50.0, 80.0)]), Some(t));
    }

    #[test]
    fn nearest_animal_within_radius() {
        let pts = vec![(1u64, SimVec2::new(0.0, 0.0)), (2, SimVec2::new(1.0, 1.0)), (3, SimVec2::new(5.0, 5.0))];
        assert_eq!(nearest_within(SimVec2::new(0.9, 0.8), pts.clone().into_iter(), 1.5), Some(2));
        assert_eq!(nearest_within(SimVec2::new(3.0, 3.0), pts.into_iter(), 1.5), None);
        assert!((px_per_metre(10.0, std::f32::consts::FRAC_PI_2, 200.0) - 10.0).abs() < 1e-4);
        // The ray pre-filter keeps what the pointer is on, drops what is far off or behind.
        let fov = std::f32::consts::FRAC_PI_4;
        assert!(near_ray(Vec3::ZERO, Vec3::Z, Vec3::new(0.5, 0.0, 30.0), 1.0, fov, 800.0));
        assert!(!near_ray(Vec3::ZERO, Vec3::Z, Vec3::new(20.0, 0.0, 30.0), 1.0, fov, 800.0));
        assert!(!near_ray(Vec3::ZERO, Vec3::Z, Vec3::new(0.0, 0.0, -30.0), 1.0, fov, 800.0));
        // A tiny animal far away still gets the minimum pick size.
        assert!(near_ray(Vec3::ZERO, Vec3::Z, Vec3::new(4.0, 0.0, 400.0), 0.05, fov, 800.0));
    }
}
