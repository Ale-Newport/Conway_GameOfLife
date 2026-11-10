//! Panel contents: the Tools panel (model, tool, view), the Properties tab
//! (inspector, kernel and growth plots, live parameters, dynamics) and the
//! world overlay (legend, scale, time and regime).

use std::collections::HashMap;

use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, Ui, vec2};
use continuous::{Blend, ContinuousCommand, ContinuousView, Placement, Rule, ViewLayer};
use interface::theme::with_alpha;
use interface::widgets::{badge, gradient_bar, param_editor};
use interface::{ActiveSession, MetricsHistory, ModeRegistry, Palette, SessionStatus, ThemeSettings};
use rendering::grid::VisibleCells;
use rendering::palette::Colormap;
use rendering::{WorldInput, palette::MAX_PALETTE_COLORS};
use simulation_core::{Control, ModelDescriptor};

use crate::field::{
    channel_color, colormap_bytes, creature_rgba, format_cell_value, format_value, inside_world, plane_display_label,
    sample_plane, scale_bar,
};
use crate::plots::{growth_plot, kernel_count, kernel_label, kernel_plot, kernel_summary, series_plot};
use crate::regime::{self, Regime};
use crate::session::{Catalog, ContinuousRunner, Family, LoadRequests, LoadedWorld};
use crate::tools::{BRUSH_RADIUS, Hover, MODE_SHORTCUTS, STAMP_SCALE, Tool, ToolSettings, brush_value_max};
use crate::world::{CameraFit, DisplayPlane, LAYERS, LatestView, ViewSettings, layer_help, layer_label, mode_palette};

fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(palette.text_muted));
    ui.add_space(2.0);
}

fn card<R>(ui: &mut Ui, palette: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(palette.stroke_width, palette.stroke))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Equal-width toggle buttons in rows of `columns`; returns the index of
/// the clicked item.
fn segmented(ui: &mut Ui, items: &[(&str, &str, bool)], columns: usize) -> Option<usize> {
    let gap = 4.0;
    let columns = columns.clamp(1, items.len().max(1));
    let width = ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).floor().max(40.0);
    let mut clicked = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        for (row, chunk) in items.chunks(columns).enumerate() {
            ui.horizontal(|ui| {
                for (j, (label, tip, on)) in chunk.iter().enumerate() {
                    let button =
                        egui::Button::new(RichText::new(*label).size(13.0)).selected(*on).min_size(vec2(width, 28.0));
                    if ui.add(button).on_hover_text(*tip).clicked() {
                        clicked = Some(row * columns + j);
                    }
                }
            });
        }
    });
    clicked
}

fn colormap_stops(map: Colormap) -> Vec<Color32> {
    (0..16)
        .map(|i| {
            let [r, g, b] = colormap_bytes(map, i as f32 / 15.0);
            Color32::from_rgb(r, g, b)
        })
        .collect()
}

/// Small horizontal swatch of a colormap.
fn colormap_swatch(ui: &mut Ui, map: Colormap, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let steps = 24;
    let w = rect.width() / steps as f32;
    for i in 0..steps {
        let [r, g, b] = colormap_bytes(map, (i as f32 + 0.5) / steps as f32);
        let cell = egui::Rect::from_min_size(rect.min + vec2(i as f32 * w, 0.0), vec2(w + 0.5, rect.height()));
        ui.painter().rect_filled(cell, 0.0, Color32::from_rgb(r, g, b));
    }
    ui.painter().rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );
}

/// Label of a plane selection for the current view.
fn plane_label(plane: DisplayPlane, view: Option<&ContinuousView>) -> String {
    match plane {
        DisplayPlane::Composite => "RGB composite".to_string(),
        other => {
            let i = other.index().unwrap_or(0);
            view.and_then(|v| v.labels.get(i)).map_or_else(|| format!("plane {i}"), |l| plane_display_label(l))
        }
    }
}

/// egui textures of creature thumbnails.
#[derive(Default)]
pub(crate) struct Thumbnails {
    textures: HashMap<(usize, Colormap), egui::TextureHandle>,
}

/// Left panel: model, tool and view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tools_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    loaded: Option<Res<LoadedWorld>>,
    catalog: Res<Catalog>,
    latest: Res<LatestView>,
    runner: Option<Res<ContinuousRunner>>,
    mut tools: ResMut<ToolSettings>,
    mut settings: ResMut<ViewSettings>,
    mut requests: ResMut<LoadRequests>,
    mut fit: ResMut<CameraFit>,
    mut thumbs: Local<Thumbnails>,
) {
    let palette = mode_palette(&theme, &registry);
    let Some(world) = loaded else {
        ui.label(RichText::new("Starting the simulation…").weak());
        return;
    };
    // Edit copies so change detection only fires on real changes.
    let mut t = tools.clone();
    let mut s = settings.clone();
    t.fit_world(world.channels(), brush_value_max(world.edit_limit()));
    // Label column + value field take about 150 px: keeping the rows narrower
    // than the panel avoids the panel widening itself frame after frame.
    ui.spacing_mut().slider_width = (ui.available_width() - 165.0).clamp(60.0, 220.0);

    model_section(ui, &palette, &world, &mut requests);
    tool_section(ui, &palette, &world, &catalog, &mut t, &mut thumbs, s.colormap, runner.as_deref());
    view_section(ui, &palette, &world, latest.view.as_deref(), &mut s, &mut fit);
    shortcuts_section(ui, &palette);

    tools.set_if_neq(t);
    settings.set_if_neq(s);
}

fn model_section(ui: &mut Ui, palette: &Palette, world: &LoadedWorld, requests: &mut LoadRequests) {
    section(ui, palette, "Model");
    let current = world.family();
    let items: Vec<(&str, &str, bool)> = Family::ALL.iter().map(|f| (f.label(), f.blurb(), *f == current)).collect();
    if let Some(i) = segmented(ui, &items, 2)
        && Family::ALL[i] != current
    {
        requests.0.push(Family::ALL[i].default_scenario().to_string());
    }
    ui.add_space(4.0);
    card(ui, palette, |ui| {
        ui.label(RichText::new(&world.header.name).size(14.5).strong());
        let channels = world.channels();
        let kernels = kernel_count(&world.rule);
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        let world_line = format!("{} × {} torus · {}", world.width, world.height, plural(channels, "channel"));
        let rule_line = match &world.rule {
            Rule::Lenia(r) => format!("{} · R = {} · T = {}", plural(kernels, "kernel"), r.radius, r.time_resolution),
            Rule::SmoothLife(r) => format!("outer radius {} · inner {:.2}", r.outer_radius, r.inner_radius()),
            Rule::FlowLenia(r) => format!("{} · R = {} · dt = {}", plural(kernels, "kernel"), r.radius, r.dt),
        };
        let facts = format!("{world_line}\n{rule_line}");
        ui.label(RichText::new(facts).size(12.0).color(palette.text_muted));
        ui.add_space(2.0);
        egui::CollapsingHeader::new(RichText::new("About this scenario").size(12.5))
            .id_salt("cl_about_scenario")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(RichText::new(&world.header.description).size(12.5));
            });
    });
}

#[allow(clippy::too_many_arguments)]
fn tool_section(
    ui: &mut Ui,
    palette: &Palette,
    world: &LoadedWorld,
    catalog: &Catalog,
    t: &mut ToolSettings,
    thumbs: &mut Thumbnails,
    colormap: Colormap,
    runner: Option<&ContinuousRunner>,
) {
    section(ui, palette, "Tool");
    let items: Vec<(&str, &str, bool)> = Tool::ALL.iter().map(|x| (x.label(), x.help(), *x == t.tool)).collect();
    if let Some(i) = segmented(ui, &items, 4) {
        t.tool = Tool::ALL[i];
    }
    ui.add_space(4.0);
    match t.tool {
        Tool::Inspect => {
            ui.label(
                RichText::new(
                    "Hover the world: the cell under the pointer is outlined and its values appear next to it \
                     and in Properties › Inspector.",
                )
                .size(12.5)
                .color(palette.text_muted),
            );
        }
        Tool::Brush | Tool::Erase => brush_settings(ui, palette, world, t),
        Tool::Stamp => stamp_settings(ui, palette, world, catalog, t, thumbs, colormap, runner),
    }
}

fn brush_settings(ui: &mut Ui, palette: &Palette, world: &LoadedWorld, t: &mut ToolSettings) {
    egui::Grid::new("cl_brush_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        if t.tool == Tool::Brush {
            ui.label("Value").on_hover_text("Value the brush blends the cells towards.");
            let max = brush_value_max(world.edit_limit());
            ui.add(egui::Slider::new(&mut t.brush_value, 0.0..=max).max_decimals(2));
            ui.end_row();
        }
        ui.label("Radius");
        ui.add(egui::Slider::new(&mut t.brush_radius, BRUSH_RADIUS.0..=BRUSH_RADIUS.1).step_by(0.5).suffix(" cells"));
        ui.end_row();
        ui.label("Softness").on_hover_text("Fraction of the radius over which the brush fades out.");
        ui.add(egui::Slider::new(&mut t.brush_softness, 0.0..=1.0).max_decimals(2));
        ui.end_row();
        if world.channels() > 1 {
            ui.label("Channel");
            let selected = t.brush_channel.map_or_else(|| "All channels".to_string(), |c| format!("Channel {c}"));
            egui::ComboBox::from_id_salt("cl_brush_channel").selected_text(selected).show_ui(ui, |ui| {
                ui.selectable_value(&mut t.brush_channel, None, "All channels");
                for c in 0..world.channels() {
                    ui.selectable_value(&mut t.brush_channel, Some(c), format!("Channel {c}"));
                }
            });
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    brush_profile(ui, palette, t);
    ui.label(
        RichText::new("Drag in the world · [ ] radius · one stroke = one undo step")
            .size(12.0)
            .color(palette.text_muted),
    );
}

/// Falloff of the brush across its diameter.
fn brush_profile(ui: &mut Ui, palette: &Palette, t: &ToolSettings) {
    let brush = t.brush_at(bevy::math::DVec2::ZERO);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, palette.surface);
    let color = if t.tool == Tool::Erase { palette.error } else { palette.accent };
    let n = 64;
    let points: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let f = i as f32 / n as f32;
            let d = (f * 2.0 - 1.0).abs() * brush.radius;
            let w = brush.weight(d);
            egui::pos2(rect.left() + f * rect.width(), rect.bottom() - 3.0 - w * (rect.height() - 8.0))
        })
        .collect();
    let mut fill = points.clone();
    fill.push(rect.right_bottom());
    fill.push(rect.left_bottom());
    painter.add(egui::Shape::convex_polygon(fill, with_alpha(color, 0.18), Stroke::NONE));
    painter.add(egui::Shape::line(points, Stroke::new(1.5, color)));
}

#[allow(clippy::too_many_arguments)]
fn stamp_settings(
    ui: &mut Ui,
    palette: &Palette,
    world: &LoadedWorld,
    catalog: &Catalog,
    t: &mut ToolSettings,
    thumbs: &mut Thumbnails,
    colormap: Colormap,
    runner: Option<&ContinuousRunner>,
) {
    if thumbs.textures.keys().any(|(_, m)| *m != colormap) {
        thumbs.textures.clear();
    }
    for (i, creature) in catalog.creatures.iter().enumerate() {
        let fits = creature.cells.channels() <= world.channels();
        let texture = thumbs.textures.entry((i, colormap)).or_insert_with(|| {
            let (w, h, rgba) = creature_rgba(&creature.cells, colormap);
            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            ui.ctx().load_texture(format!("cl_creature_{i}"), image, egui::TextureOptions::LINEAR)
        });
        let native = catalog.name_of(&creature.scenario_id);
        let channels = creature.cells.channels();
        let selected = t.creature == i;
        let (rect, response) = ui
            .allocate_exact_size(vec2(ui.available_width(), 44.0), if fits { Sense::click() } else { Sense::hover() });
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect);
            let fill = if selected {
                with_alpha(palette.accent, 0.22)
            } else if fits && response.hovered() {
                palette.surface_hi
            } else {
                palette.surface
            };
            painter.rect_filled(rect, 6.0, fill);
            if selected {
                painter.rect_stroke(rect, 6.0, Stroke::new(1.0, palette.accent), egui::StrokeKind::Inside);
            }
            let (w, h) = (creature.cells.width() as f32, creature.cells.height() as f32);
            let k = 34.0 / w.max(h);
            let thumb = egui::Rect::from_center_size(rect.left_center() + vec2(24.0, 0.0), vec2(w * k, h * k));
            let tint = if fits { Color32::WHITE } else { with_alpha(Color32::WHITE, 0.35) };
            egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), thumb.size()))
                .tint(tint)
                .paint_at(ui, thumb);
            let text_left = rect.left() + 50.0;
            let max_width = (rect.right() - text_left - 8.0).max(10.0);
            let line = |text: String, size: f32, color: Color32| {
                let mut job = egui::text::LayoutJob::single_section(
                    text,
                    egui::TextFormat { font_id: egui::FontId::proportional(size), color, ..default() },
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let name_color = if fits { palette.text } else { palette.text_muted };
            let detail = if channels > 1 {
                format!("{channels} channels · {}×{} cells", creature.cells.width(), creature.cells.height())
            } else {
                format!("{}×{} cells · {native}", creature.cells.width(), creature.cells.height())
            };
            painter.galley(
                egui::pos2(text_left, rect.top() + 6.0),
                line(creature.name.clone(), 13.0, name_color),
                name_color,
            );
            painter.galley(
                egui::pos2(text_left, rect.top() + 24.0),
                line(detail, 11.0, palette.text_muted),
                palette.text_muted,
            );
        }
        let hover = if fits {
            format!(
                "{}: defined by the scenario “{native}”. Creatures usually only survive under their own rule.",
                creature.name
            )
        } else {
            format!("{} needs {channels} channels; this world has {}.", creature.name, world.channels())
        };
        let response = response.on_hover_text(hover);
        if response.clicked() {
            t.creature = i;
        }
    }
    ui.add_space(4.0);
    egui::Grid::new("cl_stamp_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Rotation").on_hover_text("Clockwise on screen; Q and E turn by 15°.");
        ui.add(egui::Slider::new(&mut t.rotation_deg, -180.0..=180.0).step_by(1.0).suffix("°"));
        ui.end_row();
        ui.label("Scale").on_hover_text("Resampled bilinearly; the mass scales with the square ([ / ]).");
        ui.add(
            egui::Slider::new(&mut t.scale, STAMP_SCALE.0..=STAMP_SCALE.1)
                .logarithmic(true)
                .max_decimals(2)
                .suffix("×"),
        );
        ui.end_row();
        ui.label("Blend");
        ui.horizontal(|ui| {
            ui.radio_value(&mut t.blend, Blend::Replace, "Replace").on_hover_text("Overwrite the covered box.");
            ui.radio_value(&mut t.blend, Blend::Add, "Add").on_hover_text("Add to the existing values.");
        });
        ui.end_row();
    });
    ui.add_space(4.0);
    let can_place = catalog.creatures.get(t.creature).is_some_and(|c| c.cells.channels() <= world.channels());
    ui.horizontal(|ui| {
        let place = ui.add_enabled(can_place && runner.is_some(), egui::Button::new("Place at centre"));
        if place.on_hover_text("Place the creature at the centre of the world (undoable).").clicked()
            && let (Some(runner), Some(creature)) = (runner, catalog.creatures.get(t.creature))
        {
            let placement = Placement {
                x: world.width as f32 / 2.0,
                y: world.height as f32 / 2.0,
                ..t.placement_at(bevy::math::DVec2::ZERO)
            };
            runner.0.send(Control::Edit(ContinuousCommand::PlaceCreature { cells: creature.cells.clone(), placement }));
        }
        if ui.button("Reset").on_hover_text("Rotation 0°, scale 1×.").clicked() {
            t.rotation_deg = 0.0;
            t.scale = 1.0;
        }
    });
    ui.label(
        RichText::new("Click in the world to place · Q / E rotate · [ ] scale").size(12.0).color(palette.text_muted),
    );
}

fn view_section(
    ui: &mut Ui,
    palette: &Palette,
    world: &LoadedWorld,
    view: Option<&ContinuousView>,
    s: &mut ViewSettings,
    fit: &mut CameraFit,
) {
    section(ui, palette, "View");
    let channels = world.channels();
    let items: Vec<(&str, &str, bool)> =
        LAYERS.iter().map(|l| (layer_label(*l, &world.rule), layer_help(*l, &world.rule), *l == s.layer)).collect();
    if let Some(i) = segmented(ui, &items, 3)
        && LAYERS[i] != s.layer
    {
        s.set_layer(LAYERS[i], channels);
    }
    ui.add_space(4.0);
    // Planes of the layer as published (the composite for multi-channel states).
    let view = view.filter(|v| v.layer == s.layer);
    let mut options: Vec<DisplayPlane> = Vec::new();
    if s.layer == ViewLayer::State && channels > 1 {
        options.push(DisplayPlane::Composite);
    }
    let planes = view.map_or(1, |v| v.planes.len());
    options.extend((0..planes).map(|i| if i == 0 { DisplayPlane::First } else { DisplayPlane::Index(i) }));
    egui::Grid::new("cl_view_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        if options.len() > 1 {
            ui.label("Plane");
            egui::ComboBox::from_id_salt("cl_plane").selected_text(plane_label(s.plane, view)).width(170.0).show_ui(
                ui,
                |ui| {
                    for option in &options {
                        ui.selectable_value(&mut s.plane, *option, plane_label(*option, view));
                    }
                },
            );
            ui.end_row();
        }
        ui.label("Colormap");
        let composite = s.plane == DisplayPlane::Composite && s.layer == ViewLayer::State && channels > 1;
        ui.add_enabled_ui(!composite, |ui| {
            egui::ComboBox::from_id_salt("cl_colormap").selected_text(s.colormap.label()).width(170.0).show_ui(
                ui,
                |ui| {
                    for map in Colormap::ALL {
                        ui.horizontal(|ui| {
                            colormap_swatch(ui, map, vec2(44.0, 12.0));
                            ui.selectable_value(&mut s.colormap, map, map.label());
                        });
                    }
                },
            )
        })
        .response
        .on_disabled_hover_text("The RGB composite colours channels 0, 1, 2 red, green and blue.");
        ui.end_row();
    });
    if !(s.plane == DisplayPlane::Composite && channels > 1 && s.layer == ViewLayer::State) {
        gradient_bar(ui, &colormap_stops(s.colormap), ui.available_width().min(260.0), "low", "high");
    }
    ui.add_space(4.0);
    ui.checkbox(&mut s.auto_contrast, "Auto contrast")
        .on_hover_text("Colour from the smallest to the largest shown value instead of the model's natural range.");
    ui.checkbox(&mut s.smooth, "Smooth interpolation").on_hover_text("Bilinear interpolation between cell centres.");
    ui.checkbox(&mut s.grid_lines, "Cell grid (G)")
        .on_hover_text("Cell outlines; they fade in once the cells are large enough on screen (zoom in).");
    ui.checkbox(&mut s.show_border, "World border").on_hover_text("Outline of the torus (it wraps at the edges).");
    ui.horizontal(|ui| {
        ui.checkbox(&mut s.show_trail, "Centroid trail (T)")
            .on_hover_text("Path of the mass-weighted centre over the last ticks (from the metrics).");
        ui.add_enabled(
            s.show_trail,
            egui::DragValue::new(&mut s.trail_ticks).range(50..=20_000).speed(20.0).suffix(" ticks"),
        );
    });
    ui.add_space(4.0);
    if ui.button("Frame the world  F").on_hover_text("Zoom and pan so the whole torus is visible.").clicked() {
        fit.request(true);
    }
}

fn shortcuts_section(ui: &mut Ui, palette: &Palette) {
    ui.add_space(8.0);
    egui::CollapsingHeader::new(RichText::new("Shortcuts and mouse").size(12.5).color(palette.text_muted))
        .id_salt("cl_shortcuts")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("cl_shortcut_grid").num_columns(2).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
                for (_, _, keys, description) in MODE_SHORTCUTS {
                    ui.label(RichText::new(*keys).monospace().size(12.0));
                    ui.label(RichText::new(*description).size(12.0));
                    ui.end_row();
                }
                for (keys, description) in [
                    ("Left drag", "Use the tool"),
                    ("Right / middle drag", "Pan"),
                    ("Wheel, pinch", "Zoom towards the pointer"),
                    ("Arrows, WASD", "Pan"),
                ] {
                    ui.label(RichText::new(keys).monospace().size(12.0));
                    ui.label(RichText::new(description).size(12.0));
                    ui.end_row();
                }
            });
        });
}

/// Kernel picker state of the Properties tab.
#[derive(Default)]
pub(crate) struct KernelPick(usize);

/// Right panel "Properties": inspector, kernel & growth, live parameters,
/// dynamics.
#[allow(clippy::too_many_arguments)]
pub(crate) fn properties_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    loaded: Option<Res<LoadedWorld>>,
    latest: Res<LatestView>,
    hover: Res<Hover>,
    status: Res<SessionStatus>,
    session: Option<Res<ActiveSession>>,
    history: Res<MetricsHistory>,
    mut pick: Local<KernelPick>,
) {
    let palette = mode_palette(&theme, &registry);
    let Some(world) = loaded else {
        ui.label(RichText::new("Starting the simulation…").weak());
        return;
    };
    inspector(ui, &palette, &world, latest.view.as_deref(), &hover);

    section(ui, &palette, "Kernel and growth");
    let count = kernel_count(&world.rule);
    if pick.0 >= count {
        pick.0 = 0;
    }
    if count > 1 {
        egui::ComboBox::from_id_salt("cl_kernel_pick")
            .selected_text(kernel_label(&world.rule, pick.0))
            .width(ui.available_width().min(280.0))
            .show_ui(ui, |ui| {
                for k in 0..count {
                    ui.selectable_value(&mut pick.0, k, kernel_label(&world.rule, k));
                }
            });
    }
    ui.label(RichText::new(kernel_summary(&world.rule, pick.0)).size(12.0).color(palette.text_muted));
    ui.add_space(2.0);
    kernel_plot(ui, &world.rule, pick.0, &palette);
    growth_plot(ui, &world.rule, pick.0, &palette);
    ui.label(
        RichText::new(match &world.rule {
            Rule::SmoothLife(_) => "Disc and annulus weights against the distance; transition for dead and live cells.",
            _ => "Kernel weight against the distance (normalised to Σ K = 1 when applied); growth against the potential, positive in the shaded band.",
        })
        .size(11.5)
        .color(palette.text_muted),
    );

    section(ui, &palette, "Live parameters");
    match (&session, status.get()) {
        (Some(session), Some(current)) => {
            let descriptor = live_parameter_descriptor(session.session().descriptor(), pick.0);
            param_editor(ui, &descriptor, &current.params, session.session());
        }
        _ => {
            ui.label(RichText::new("No simulation is running.").weak());
        }
    }

    section(ui, &palette, "Dynamics");
    dynamics(ui, &palette, &history, latest.view.as_deref());
}

/// The parameters edited next to the plots: the rule-wide ones (`R`, `T`,
/// SmoothLife's and Flow-Lenia's globals) and those of kernel `k`.
pub fn live_parameter_descriptor(descriptor: &ModelDescriptor, k: usize) -> ModelDescriptor {
    let prefix = format!("k{k}.");
    let mut d = descriptor.clone();
    d.params.retain(|p| p.group == "Rule" || p.key.starts_with(&prefix));
    d
}

fn inspector(ui: &mut Ui, palette: &Palette, world: &LoadedWorld, view: Option<&ContinuousView>, hover: &Hover) {
    section(ui, palette, "Inspector");
    let cell = hover
        .position
        .filter(|p| inside_world(*p, world.width, world.height))
        .map(|p| I64Vec2::new(p.x.floor() as i64, p.y.floor() as i64));
    card(ui, palette, |ui| {
        let Some(cell) = cell else {
            ui.label(
                RichText::new("Hover the world to read the values of a cell.").size(12.5).color(palette.text_muted),
            );
            return;
        };
        let Some(view) = view else {
            ui.label(RichText::new(format!("Cell ({}, {})", cell.x, cell.y)).strong());
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Cell ({}, {})", cell.x, cell.y)).strong());
            ui.label(RichText::new(layer_label(view.layer, &world.rule)).size(12.5).color(palette.text_muted));
        });
        egui::Grid::new("cl_inspector_grid").num_columns(2).spacing([16.0, 2.0]).show(ui, |ui| {
            for (i, label) in view.labels.iter().enumerate() {
                ui.label(RichText::new(plane_display_label(label)).size(12.5).color(palette.text_muted));
                let value = sample_plane(view, i, cell).map_or_else(|| "n/a".to_string(), format_cell_value);
                ui.label(RichText::new(value).monospace().size(12.5));
                ui.end_row();
            }
        });
        if view.cells_per_sample > 1 {
            ui.label(
                RichText::new(format!("Zoomed out: averages of {0}×{0} cells.", view.cells_per_sample))
                    .size(11.5)
                    .color(palette.text_muted),
            );
        }
    });
}

fn dynamics(ui: &mut Ui, palette: &Palette, history: &MetricsHistory, view: Option<&ContinuousView>) {
    let store = history.store();
    let points = |key: &str| -> Vec<[f64; 2]> {
        store.get(key).map(|s| s.display().into_iter().map(|p| [p.tick as f64, p.value]).collect()).unwrap_or_default()
    };
    let latest = |key: &str| history.latest(key).map_or_else(|| "—".to_string(), format_value);
    let mass = format!("Total mass  {}", latest("mass"));
    let speed = format!("Centroid speed  {} cells/tick", latest("speed"));
    series_plot(ui, "cl_mass_plot", "Total mass", &mass, points("mass"), palette.accent, palette);
    ui.add_space(4.0);
    series_plot(ui, "cl_speed_plot", "Centroid speed", &speed, points("speed"), palette.series_color(2), palette);
    let Some(view) = view else { return };
    if view.channel_stats.is_empty() {
        return;
    }
    ui.add_space(4.0);
    egui::Grid::new("cl_channel_stats").num_columns(5).spacing([10.0, 2.0]).striped(true).show(ui, |ui| {
        for head in ["Channel", "Mass", "Mean", "Max", "Active"] {
            ui.label(RichText::new(head).size(11.5).strong().color(palette.text_muted));
        }
        ui.end_row();
        for (c, stats) in view.channel_stats.iter().enumerate() {
            ui.horizontal(|ui| {
                if view.channel_stats.len() > 1 {
                    let [r, g, b] = channel_color(c).map(|v| (v * 255.0) as u8);
                    interface::widgets::swatch(ui, Color32::from_rgb(r, g, b), 10.0);
                }
                ui.label(RichText::new(c.to_string()).size(12.0));
            });
            ui.label(RichText::new(format_value(stats.mass)).monospace().size(12.0));
            ui.label(RichText::new(format_value(stats.mean)).monospace().size(12.0));
            ui.label(RichText::new(format_value(f64::from(stats.max))).monospace().size(12.0));
            ui.label(RichText::new(format!("{:.1} %", stats.active_fraction * 100.0)).monospace().size(12.0));
            ui.end_row();
        }
    });
}

fn recent(history: &MetricsHistory, key: &str) -> Vec<f64> {
    history.store().get(key).map_or_else(Vec::new, |s| {
        let p = s.points();
        p[p.len().saturating_sub(regime::WINDOW)..].iter().map(|p| p.value).collect()
    })
}

/// Overlay over the world: model, layer legend, time, regime and scale.
#[allow(clippy::too_many_arguments)]
pub(crate) fn overlay_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    loaded: Option<Res<LoadedWorld>>,
    latest: Res<LatestView>,
    settings: Res<ViewSettings>,
    status: Res<SessionStatus>,
    history: Res<MetricsHistory>,
    visible: Res<VisibleCells>,
    hover: Res<Hover>,
    input: Res<WorldInput>,
    mut collapsed: Local<bool>,
) {
    let palette = mode_palette(&theme, &registry);
    let Some(world) = loaded else { return };
    egui::Frame::new()
        .fill(with_alpha(palette.panel, 0.9))
        .stroke(Stroke::new(palette.stroke_width, with_alpha(palette.stroke, 0.9)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_max_width(250.0);
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.horizontal(|ui| {
                // The overlay covers a corner of the world: it can be folded
                // down to this header line.
                let (_, toggle) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
                egui::collapsing_header::paint_default_icon(ui, if *collapsed { 0.0 } else { 1.0 }, &toggle);
                let tip = if *collapsed { "Show the legend, time, regime and scale" } else { "Fold the overlay" };
                if toggle.on_hover_text(tip).clicked() {
                    *collapsed = !*collapsed;
                }
                badge(ui, world.family().label(), palette.accent);
                ui.label(RichText::new(&world.header.name).size(13.0).strong());
            });
            let view = latest.view.as_deref();
            let layer = view.map_or(settings.layer, |v| v.layer);
            let plane = plane_label(latest.plane, view);
            ui.label(
                RichText::new(format!("{} · {plane}", layer_label(layer, &world.rule))).size(12.0).color(palette.text),
            );
            if *collapsed {
                return;
            }
            legend(ui, &palette, &settings, &latest, world.channels());
            if let Some(current) = status.get() {
                let dt = world.rule.dt();
                ui.label(
                    RichText::new(format!("tick {} · dt = {}", current.tick, format_value(dt)))
                        .size(11.5)
                        .color(palette.text_muted),
                );
            }
            let active = history.latest("active_fraction").unwrap_or(0.0);
            let regime = regime::classify(&recent(&history, "mass"), &recent(&history, "speed"), active);
            let color = match regime {
                Regime::Empty => palette.error,
                Regime::Travelling | Regime::Stationary => palette.success,
                Regime::Unknown => palette.text_muted,
                _ => palette.warning,
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new("Regime").size(11.5).color(palette.text_muted));
                badge(ui, regime.label(), color).on_hover_text(regime::CRITERIA);
            });
            if let Some((cells, px)) = scale_bar(visible.cells_per_pixel, 90.0) {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(px.min(200.0), 10.0), Sense::hover());
                    let y = rect.center().y;
                    let stroke = Stroke::new(1.5, palette.text);
                    ui.painter().line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], stroke);
                    for x in [rect.left(), rect.right()] {
                        ui.painter().line_segment([egui::pos2(x, y - 4.0), egui::pos2(x, y + 4.0)], stroke);
                    }
                    let unit = if cells == 1.0 { "cell" } else { "cells" };
                    ui.label(RichText::new(format!("{cells} {unit}")).size(11.5).color(palette.text_muted));
                });
            }
        });
    hover_readout(ui, &palette, &world, latest.view.as_deref(), &hover, &input);
}

fn legend(ui: &mut Ui, palette: &Palette, settings: &ViewSettings, latest: &LatestView, channels: usize) {
    let (lo, hi) = latest.range;
    if latest.plane == DisplayPlane::Composite {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for c in 0..channels {
                let [r, g, b] = channel_color(c).map(|v| (v * 255.0) as u8);
                interface::widgets::legend_chip(ui, Color32::from_rgb(r, g, b), &format!("ch {c}"));
            }
        });
        ui.label(
            RichText::new(format!("each channel {} to {}", format_value(f64::from(lo)), format_value(f64::from(hi))))
                .size(11.5)
                .color(palette.text_muted),
        );
    } else {
        let stops: Vec<Color32> = colormap_stops(settings.colormap);
        debug_assert!(stops.len() <= MAX_PALETTE_COLORS);
        gradient_bar(ui, &stops, 220.0, &format_value(f64::from(lo)), &format_value(f64::from(hi)));
        if let Some(note) = zero_note(lo, hi) {
            ui.label(RichText::new(note).size(11.5).color(palette.text_muted));
        }
    }
}

/// Where zero lies on a signed colour scale (growth, affinity), or `None`
/// when the range does not straddle zero.
fn zero_note(lo: f32, hi: f32) -> Option<String> {
    if !(lo < 0.0 && hi > 0.0) {
        return None;
    }
    let t = -lo / (hi - lo);
    Some(if (t - 0.5).abs() < 0.01 {
        "zero is the middle colour".to_string()
    } else {
        format!("zero is at {:.0} % of the scale", t * 100.0)
    })
}

/// Top-left corner of the hover readout (text of `size`): below and right
/// of the pointer, flipped to the other side of the pointer where it would
/// leave the world viewport (and cover a side panel).
fn readout_position(cursor: egui::Pos2, size: egui::Vec2, viewport: Option<egui::Rect>) -> egui::Pos2 {
    const OFFSET: f32 = 16.0;
    // The painted box extends 7 × 5 points around the text.
    let margin = vec2(7.0, 5.0);
    let mut pos = cursor + vec2(OFFSET, OFFSET);
    if let Some(v) = viewport {
        if pos.x + size.x + margin.x > v.max.x {
            pos.x = cursor.x - OFFSET - size.x;
        }
        if pos.y + size.y + margin.y > v.max.y {
            pos.y = cursor.y - OFFSET - size.y;
        }
    }
    pos
}

/// Values of the cell under the pointer, painted next to it (a paint-only
/// layer, so it never takes the pointer from the world).
fn hover_readout(
    ui: &Ui,
    palette: &Palette,
    world: &LoadedWorld,
    view: Option<&ContinuousView>,
    hover: &Hover,
    input: &WorldInput,
) {
    let (Some(p), Some(cursor), Some(view)) = (hover.position, input.cursor, view) else { return };
    if !inside_world(p, world.width, world.height) {
        return;
    }
    let cell = I64Vec2::new(p.x.floor() as i64, p.y.floor() as i64);
    let values: Vec<String> = view
        .labels
        .iter()
        .enumerate()
        .take(4)
        .filter_map(|(i, label)| {
            sample_plane(view, i, cell).map(|v| format!("{}  {}", plane_display_label(label), format_cell_value(v)))
        })
        .collect();
    let mut text = format!("({}, {})  {}", cell.x, cell.y, layer_label(view.layer, &world.rule));
    for line in values {
        text.push('\n');
        text.push_str(&line);
    }
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("cl_hover_readout")));
    let galley = painter.layout_no_wrap(text, egui::FontId::monospace(11.5), palette.text);
    let viewport =
        input.viewport.map(|v| egui::Rect::from_min_max(egui::pos2(v.min.x, v.min.y), egui::pos2(v.max.x, v.max.y)));
    let pos = readout_position(egui::pos2(cursor.x, cursor.y), galley.size(), viewport);
    let rect = egui::Rect::from_min_size(pos, galley.size()).expand2(vec2(7.0, 5.0));
    painter.rect(
        rect,
        5.0,
        with_alpha(palette.panel, 0.92),
        Stroke::new(1.0, with_alpha(palette.stroke, 0.9)),
        egui::StrokeKind::Inside,
    );
    painter.galley(pos, galley, palette.text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Catalog;
    use continuous::ContinuousSim;
    use simulation_core::Simulation;

    #[test]
    fn live_parameters_are_the_rule_and_one_kernel() {
        let catalog = Catalog::builtin();
        let sim = ContinuousSim::from_scenario(catalog.get("lenia.tessellatium_gyrans").unwrap().clone()).unwrap();
        let d = live_parameter_descriptor(&sim.descriptor(), 3);
        let keys: Vec<&str> = d.params.iter().map(|p| p.key.as_str()).collect();
        assert!(keys.contains(&"radius") && keys.contains(&"time_resolution"));
        assert!(keys.contains(&"k3.mu") && keys.contains(&"k3.sigma") && keys.contains(&"k3.beta"));
        assert!(!keys.iter().any(|k| k.starts_with("k1.") || k.starts_with("k13.")), "{keys:?}");
        assert!(!keys.contains(&"width") && !keys.contains(&"seed") && !keys.contains(&"convolution"));
        let sl = ContinuousSim::from_scenario(catalog.get("smoothlife.glider").unwrap().clone()).unwrap();
        let d = live_parameter_descriptor(&sl.descriptor(), 0);
        assert!(d.params.iter().any(|p| p.key == "b1") && d.params.iter().all(|p| p.group == "Rule"));
    }

    #[test]
    fn hover_readouts_stay_inside_the_viewport() {
        let viewport = Some(egui::Rect::from_min_max(egui::pos2(300.0, 40.0), egui::pos2(1200.0, 900.0)));
        let size = vec2(120.0, 40.0);
        assert_eq!(readout_position(egui::pos2(500.0, 300.0), size, viewport), egui::pos2(516.0, 316.0));
        // Near the right edge (next to the Properties panel): to the left of the pointer.
        assert_eq!(readout_position(egui::pos2(1150.0, 300.0), size, viewport), egui::pos2(1014.0, 316.0));
        // Near the bottom (above the time bar): above the pointer.
        assert_eq!(readout_position(egui::pos2(500.0, 880.0), size, viewport), egui::pos2(516.0, 824.0));
        assert_eq!(readout_position(egui::pos2(1150.0, 880.0), size, None), egui::pos2(1166.0, 896.0));
    }

    #[test]
    fn signed_scales_mark_zero() {
        assert_eq!(zero_note(-2.0, 2.0).as_deref(), Some("zero is the middle colour"));
        assert_eq!(zero_note(-1.0, 3.0).as_deref(), Some("zero is at 25 % of the scale"));
        assert_eq!(zero_note(0.0, 1.0), None);
        assert_eq!(zero_note(0.2, 0.6), None);
    }

    #[test]
    fn plane_labels() {
        assert_eq!(plane_label(DisplayPlane::Composite, None), "RGB composite");
        assert_eq!(plane_label(DisplayPlane::Index(2), None), "plane 2");
    }

    /// The panels draw without a running simulation and with one.
    #[test]
    fn panels_render_headless() {
        use crate::tools::{Hover, StrokeState};
        let mut app = App::new();
        app.init_resource::<ThemeSettings>()
            .init_resource::<ModeRegistry>()
            .init_resource::<ToolSettings>()
            .init_resource::<ViewSettings>()
            .init_resource::<LoadRequests>()
            .init_resource::<CameraFit>()
            .init_resource::<LatestView>()
            .init_resource::<Hover>()
            .init_resource::<StrokeState>()
            .init_resource::<SessionStatus>()
            .init_resource::<MetricsHistory>()
            .init_resource::<VisibleCells>()
            .init_resource::<WorldInput>()
            .insert_resource(Catalog::builtin());
        let tools = app.world_mut().register_system(tools_panel);
        let props = app.world_mut().register_system(properties_panel);
        let overlay = app.world_mut().register_system(overlay_panel);
        let ctx = egui::Context::default();
        let run = |app: &mut App, ctx: &egui::Context| {
            let world = app.world_mut();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                for id in [tools, props, overlay] {
                    world.run_system_with(id, ui).expect("panel system runs");
                }
            });
            output.textures_delta.clear();
        };
        run(&mut app, &ctx);
        let scenario = Catalog::builtin().get("lenia.tessellatium_gyrans").unwrap().clone();
        app.insert_resource(LoadedWorld::from_scenario(&scenario));
        app.world_mut().resource_mut::<ToolSettings>().tool = Tool::Stamp;
        run(&mut app, &ctx);
        app.world_mut().resource_mut::<ToolSettings>().tool = Tool::Brush;
        run(&mut app, &ctx);
        assert!(app.world().resource::<LoadRequests>().0.is_empty(), "drawing alone loads nothing");
    }
}

#[cfg(test)]
mod glyph_tests {
    use bevy_egui::egui;

    /// Non-ASCII characters of the string literals in this crate's sources
    /// (comment lines are skipped).
    fn literal_chars() -> Vec<(char, &'static str)> {
        let sources: [(&str, &str); 8] = [
            ("field.rs", include_str!("field.rs")),
            ("panels.rs", include_str!("panels.rs")),
            ("plots.rs", include_str!("plots.rs")),
            ("regime.rs", include_str!("regime.rs")),
            ("session.rs", include_str!("session.rs")),
            ("tools.rs", include_str!("tools.rs")),
            ("tutorial.rs", include_str!("tutorial.rs")),
            ("world.rs", include_str!("world.rs")),
        ];
        let mut out = Vec::new();
        for (file, text) in sources {
            // Unit tests and the glyph fallback table are never displayed.
            let text = text.split("\n#[cfg(test)]").next().unwrap_or(text);
            let text = match (text.find("const GLYPH_FALLBACKS"), text.find("pub fn display_descriptor")) {
                (Some(a), Some(b)) if a < b => format!("{}{}", &text[..a], &text[b..]),
                _ => text.to_string(),
            };
            for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
                for c in line.chars().filter(|c| !c.is_ascii()) {
                    if !out.iter().any(|(x, _)| *x == c) {
                        out.push((c, file));
                    }
                }
            }
        }
        out
    }

    /// Every character the mode displays exists in egui's built-in fonts
    /// (otherwise it renders as a box). Checked against the proportional
    /// family; the monospace family (tutorial `code` spans, value readouts)
    /// falls back to the same fonts after Hack, so it covers at least as much.
    /// (egui's `has_glyphs` reports false negatives for the monospace family
    /// itself.)
    #[test]
    fn every_displayed_character_has_a_glyph() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.label("x");
        });
        output.textures_delta.clear();
        let has = |c: char| ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), &c.to_string()));
        let mut missing: Vec<String> = literal_chars()
            .into_iter()
            .filter(|(c, _)| !has(*c))
            .map(|(c, file)| format!("{c} (U+{:04X}, {file})", c as u32))
            .collect();
        let tutorial = crate::tutorial::tutorial();
        let texts = tutorial.pages.iter().map(|p| p.body.as_str()).chain([tutorial.concepts.as_str()]);
        for text in texts {
            for c in text.chars().filter(|c| !c.is_ascii() && !has(*c)) {
                missing.push(format!("{c} (U+{:04X}, tutorial)", c as u32));
            }
        }
        assert!(missing.is_empty(), "characters without a glyph: {missing:?}");
    }
}
