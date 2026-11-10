//! World overlay: clock and population, the active tool, what is under the
//! cursor, a collapsible legend and a scale bar; activity badges above the
//! people are painted on a background layer clipped to the world viewport.

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Stroke, vec2};
use civilization::Terrain;
use civilization::buildings::BuildingKind;
use interface::theme::with_alpha;
use interface::widgets::swatch;
use interface::{ModeRegistry, Palette, ShellLayout, ThemeSettings};

use crate::MODE;
use crate::coords::nice_length;
use crate::interact::Hover;
use crate::palette::{activity_title, building_color32, color32, group_color, group_color32, terrain_color32};
use crate::people::{CameraDistance, PeopleIndex, Person};
use crate::scene::CivCamera;
use crate::session::Selection;
use crate::state::{CurrentFrame, Tool, ViewState};
use crate::ui::icons::paint_badge;
use crate::ui::widgets::{chip, clock, count, overlay_frame};

/// Whether the legend is expanded (per viewer, kept while the app runs).
#[derive(Resource, Clone, Copy, Debug)]
pub struct LegendOpen(pub bool);

impl Default for LegendOpen {
    fn default() -> Self {
        Self(true)
    }
}

/// The mode's camera as read by the overlay.
type OverlayCamera<'w, 's> =
    Single<'w, 's, (&'static Camera, &'static Transform, &'static Projection), With<CivCamera>>;

/// Screen rectangle (logical px) of the overlay panel as last drawn: the
/// camera frames the map clear of it and badges are not drawn under it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct OverlayRect(pub Option<Rect>);

/// Badge radius (logical px) for a camera distance, `None` when too far.
pub fn badge_radius(distance: f32) -> Option<f32> {
    if !(distance.is_finite() && distance > 0.0) || distance > 140.0 {
        return None;
    }
    Some((150.0 / distance).clamp(5.0, 12.0))
}

/// The overlay panel.
#[allow(clippy::too_many_arguments)]
pub fn overlay_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    current: Res<CurrentFrame>,
    state: Res<ViewState>,
    hover: Res<Hover>,
    layout: Res<ShellLayout>,
    mut legend: ResMut<LegendOpen>,
    mut drawn: ResMut<OverlayRect>,
    camera_distance: Res<CameraDistance>,
    people: Query<&Person>,
    index: Res<PeopleIndex>,
    camera: Option<OverlayCamera>,
) {
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    let Some(frame) = current.get() else { return };
    let orthographic = camera.as_ref().is_some_and(|c| matches!(c.2, Projection::Orthographic(_)));
    let camera = camera.map(|c| {
        let (camera, transform, _) = *c;
        (camera.clone(), GlobalTransform::from(*transform))
    });
    let under_overlay =
        drawn.0.map(|r| egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y), egui::pos2(r.max.x, r.max.y)));
    // Activity badges above heads (non-interactive, clipped to the world).
    if state.layers.icons
        && let (Some((camera, gt)), Some(central)) = (&camera, layout.central)
    {
        let clip = egui::Rect::from_min_max(
            egui::pos2(central.min.x, central.min.y),
            egui::pos2(central.max.x, central.max.y),
        );
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("civ_activity_badges")))
            .with_clip_rect(clip);
        let eye = gt.translation();
        let mut badges: Vec<(f32, egui::Pos2, f32, civilization::Activity, bool)> = Vec::new();
        for p in &people {
            let top = p.head_top();
            // An orthographic view has the same scale at every depth.
            let distance = if orthographic { camera_distance.0 } else { eye.distance(top) };
            let selected = state.selection == Selection::Agent(p.id);
            let Some(radius) = badge_radius(distance).or(selected.then_some(9.0)) else { continue };
            if let Ok(screen) = camera.world_to_viewport(gt, top + Vec3::Y * 0.05) {
                let pos = egui::pos2(screen.x, screen.y - radius);
                let hidden = under_overlay.is_some_and(|r| r.expand(radius).contains(pos));
                if clip.expand(radius).contains(pos) && !hidden {
                    badges.push((distance, pos, radius, p.activity, selected));
                }
            }
        }
        // Far badges first so near ones are drawn on top.
        badges.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (distance, pos, radius, activity, selected) in badges {
            let alpha = if selected { 1.0 } else { (1.4 - distance / 60.0).clamp(0.35, 1.0) };
            if selected {
                painter.circle_stroke(pos, radius + 2.5, Stroke::new(2.0, palette.accent));
            }
            paint_badge(&painter, pos, radius, activity, alpha);
        }
    }

    let response = overlay_frame(&palette).show(ui, |ui| {
        ui.set_width(236.0);
        let v = &frame.view;
        ui.label(RichText::new(clock(v.tick)).size(16.0).strong().color(palette.text));
        ui.label(
            RichText::new(format!(
                "{} · {} · {}",
                count(v.agents.len(), "person", "people"),
                count(v.groups.len(), "group", "groups"),
                count(v.buildings.iter().filter(|b| b.progress >= 1.0).count(), "building", "buildings")
            ))
            .size(12.0)
            .color(palette.text_muted),
        );
        if state.tool != Tool::Inspect {
            ui.add_space(2.0);
            ui.label(RichText::new(format!("Tool: {}", state.tool.label())).size(12.0).strong().color(palette.accent));
        }
        if let Some(p) = hover.person.and_then(|id| index.0.get(&id)).and_then(|e| people.get(*e).ok()) {
            ui.add_space(2.0);
            let group = (p.group != 0).then_some(p.group);
            ui.horizontal(|ui| {
                crate::ui::icons::badge_widget(ui, p.activity, 14.0);
                ui.label(
                    RichText::new(format!(
                        "#{} · {} · {}",
                        p.id.0,
                        activity_title(p.activity),
                        group.map_or("no group".to_string(), |g| format!("group {g}"))
                    ))
                    .size(12.0),
                );
            });
        } else if let Some(point) = hover.ground
            && let Some((x, y)) = crate::interact::tile_of(point, UVec2::new(v.world_width, v.world_height))
        {
            let i = (y * v.world_width + x) as usize;
            if let Some(t) = v.terrain.get(i) {
                let stock = v.resources.get(i).copied().unwrap_or(0.0);
                let mut text = format!("({x}, {y}) {}", t.label());
                if t.resource().is_some() {
                    text.push_str(&format!(" · stock {:.0} %", 100.0 * stock));
                }
                let road = v.roads.get(i).copied().unwrap_or(0.0);
                if road >= 0.03 {
                    let kind = if road >= v.road_threshold { "road" } else { "trail" };
                    text.push_str(&format!(" · {kind} {road:.2}"));
                }
                ui.label(RichText::new(text).size(11.5).color(palette.text_muted));
            }
        }
        ui.add_space(4.0);
        let header = egui::CollapsingHeader::new(RichText::new("Legend").size(12.0).color(palette.text_muted))
            .id_salt("civ_legend")
            .open(Some(legend.0))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
                    for t in Terrain::ALL {
                        chip(ui, terrain_color32(t), short_terrain(t), palette.text);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
                    chip(ui, Color32::from_rgb(170, 146, 104), "trail", palette.text);
                    chip(ui, Color32::from_rgb(214, 190, 142), "road", palette.text);
                    for k in BuildingKind::ALL {
                        chip(ui, building_color32(k), k.label(), palette.text);
                    }
                });
                if state.layers.territory && !v.groups.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
                        let mut groups = v.groups.clone();
                        groups.sort_by_key(|g| std::cmp::Reverse(g.members));
                        for g in groups.iter().take(8) {
                            chip(
                                ui,
                                group_color32(Some(g.color)),
                                &format!("G{} ({})", g.id.0, g.members),
                                palette.text,
                            );
                        }
                        if groups.len() > 8 {
                            ui.label(
                                RichText::new(format!("+{}", groups.len() - 8)).size(11.5).color(palette.text_muted),
                            );
                        }
                    });
                }
                ui.horizontal(|ui| {
                    swatch(ui, color32(group_color(None)), 12.0);
                    ui.label(
                        RichText::new("tunic: group colour (undyed: no group)").size(11.5).color(palette.text_muted),
                    );
                });
            });
        if header.header_response.clicked() {
            legend.0 = !legend.0;
        }
        if let (Some((camera, gt)), Some(central)) = (&camera, layout.central) {
            scale_bar(ui, &palette, camera, gt, central);
        }
    });
    let r = response.response.rect;
    let rect = Some(Rect::new(r.min.x, r.min.y, r.max.x, r.max.y));
    if drawn.0 != rect {
        drawn.0 = rect;
    }
}

fn short_terrain(t: Terrain) -> &'static str {
    match t {
        Terrain::Grassland => "grass",
        Terrain::Forest => "forest",
        Terrain::Hills => "hills",
        Terrain::Water => "water",
        Terrain::Riverbank => "riverbank",
        Terrain::Berries => "berries",
        Terrain::Mountain => "mountain",
    }
}

/// Scale bar measured at the centre of the world viewport (perspective:
/// valid at that depth).
fn scale_bar(ui: &mut egui::Ui, palette: &Palette, camera: &Camera, gt: &GlobalTransform, central: Rect) {
    let centre = central.center();
    let Ok(ray) = camera.viewport_to_world(gt, centre) else { return };
    let Some(d) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) else { return };
    let ground = ray.get_point(d);
    let (Ok(a), Ok(b)) = (camera.world_to_viewport(gt, ground), camera.world_to_viewport(gt, ground + Vec3::X)) else {
        return;
    };
    let px_per_tile = a.distance(b);
    if !(px_per_tile.is_finite() && px_per_tile > 0.1) {
        return;
    }
    let tiles = nice_length(140.0 / px_per_tile);
    let width = tiles * px_per_tile;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(width.min(200.0), 10.0), egui::Sense::hover());
        let p = ui.painter();
        let stroke = Stroke::new(1.5, palette.text);
        p.line_segment([rect.left_bottom(), rect.right_bottom()], stroke);
        p.line_segment([rect.left_bottom(), rect.left_top()], stroke);
        p.line_segment([rect.right_bottom(), rect.right_top()], stroke);
        p.line_segment(
            [rect.center_bottom(), rect.center_bottom() - vec2(0.0, 4.0)],
            Stroke::new(1.0, with_alpha(palette.text, 0.6)),
        );
        let label = if tiles == 1.0 { "1 tile".to_string() } else { format!("{tiles} tiles") };
        ui.label(RichText::new(label).size(11.5).color(palette.text_muted))
            .on_hover_text("Measured at the centre of the view.");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_shrink_with_distance_and_vanish_far_away() {
        let near = badge_radius(8.0).unwrap();
        let mid = badge_radius(25.0).unwrap();
        assert!(near >= mid);
        assert!(badge_radius(200.0).is_none());
        assert!(badge_radius(f32::NAN).is_none());
        assert!((5.0..=12.0).contains(&badge_radius(1.0).unwrap()));
    }
}
