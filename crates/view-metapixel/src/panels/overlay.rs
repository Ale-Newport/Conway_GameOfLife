//! The overlay over the world: which layer is on screen and what it is
//! (exact cells, readout, live cores or prediction), the physical
//! generation, the metagen and the cycle, the latest readout, the scale and
//! a legend; plus labels of the tile's parts when zoomed in.

use bevy::ecs::system::SystemParam;
use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Stroke, Ui, vec2};
use interface::format::format_count;
use interface::theme::with_alpha;
use interface::{ModeRegistry, Palette, ThemeSettings};
use metapixel::spec::{PERIOD, TILE_PITCH, displayed_metagen};
use metapixel::{AnnotationKind, tile_annotations};
use rendering::grid::GridSpace;

use super::{macro_legend, phase_bar};
use crate::geometry::{GridShape, ScreenRect, scale_bar};
use crate::macro_layer::{
    MacroSource, colors, effective_source, exact_detail, exact_headline, macro_headline, macro_label,
};
use crate::session::{MpSession, SimData};
use crate::timing::stage_label;
use crate::tools::{Tool, ToolState};
use crate::world::{LayerState, MpCamera, OverlayFootprint, ViewSettings, anatomy_tile, current_shape, mode_palette};

/// Resources of the overlay.
#[derive(SystemParam)]
pub(crate) struct OverlayCtx<'w, 's> {
    theme: Res<'w, ThemeSettings>,
    registry: Res<'w, ModeRegistry>,
    data: Res<'w, SimData>,
    tools: Res<'w, ToolState>,
    settings: ResMut<'w, ViewSettings>,
    layer: Res<'w, LayerState>,
    space: Res<'w, GridSpace>,
    camera: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MpCamera>>,
    session: Option<Res<'w, MpSession>>,
    footprint: ResMut<'w, OverlayFootprint>,
}

/// Width of the overlay card's content (logical pixels).
const CARD_WIDTH: f32 = 372.0;

/// Overlay panel.
pub(crate) fn overlay_panel(InMut(ui): InMut<egui::Ui>, mut ctx: OverlayCtx) {
    if ctx.session.is_none() {
        return;
    }
    let palette = mode_palette(&ctx.theme, &ctx.registry);
    let Some(view) = ctx.data.view.as_ref() else { return };
    let Some(shape) = current_shape(&ctx.data) else { return };
    let opacity = ctx.layer.macro_opacity;
    let editing = matches!(ctx.tools.tool, Tool::Draw | Tool::Stamp);
    let source = effective_source(ctx.settings.source, shape.tile_count());
    let mut details = ctx.settings.overlay_details;
    // Every row is a single line whatever the state, so the card keeps its
    // size (the camera frames the world clear of it).
    let line = |ui: &mut Ui, text: RichText| ui.add(egui::Label::new(text).truncate());
    let card = egui::Frame::new()
        .fill(with_alpha(palette.panel, 0.92))
        .stroke(Stroke::new(palette.stroke_width, palette.stroke))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            ui.spacing_mut().item_spacing.y = 3.0;
            // What is on screen.
            ui.horizontal(|ui| {
                let (text, color) = match opacity {
                    o if o <= 0.0 => ("PHYSICAL CELLS".to_string(), palette.accent),
                    o if o >= 1.0 => (layer_name(source, editing), macro_color(source, &palette)),
                    o => (format!("BLEND · {:.0}% MACRO", o * 100.0), palette.warning),
                };
                interface::widgets::badge(ui, &text, color).on_hover_text(
                    "Which layer is on screen: the physical B3/S23 cells, the macro layer, or a blend of both \
                     (Layer in the Tools panel, key L)",
                );
                if ctx.layer.pixels_per_tile.is_finite() {
                    ui.label(
                        RichText::new(format!("1 tile = {:.0} px", ctx.layer.pixels_per_tile))
                            .size(11.5)
                            .color(palette.text_muted),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let toggle = if details { "Less" } else { "More" };
                    if ui
                        .small_button(toggle)
                        .on_hover_text("Show or hide the legend and the readout details")
                        .clicked()
                    {
                        details = !details;
                    }
                    if ctx.data.jump.is_some() {
                        ui.spinner().on_hover_text("Hashlife is advancing to the next readout");
                    } else if let Some(waited) = ctx.data.busy_for(0.6) {
                        ui.spinner().on_hover_text(format!("Hashlife jump in progress ({waited:.1} s)"));
                    }
                });
            });
            // Both representations are always named; the one not on screen is dimmed.
            let population = ctx.data.info.as_ref().map_or(0, |i| i.population);
            let b = shape.bounds();
            let cells_color = if opacity < 1.0 { palette.text } else { palette.text_muted };
            line(ui, RichText::new(exact_headline(population)).size(11.5).color(cells_color))
                .on_hover_text(exact_detail((b.width(), b.height())));
            let macro_text_color = if opacity > 0.0 { macro_color(source, &palette) } else { palette.text_muted };
            let response = line(ui, RichText::new(macro_label(source, editing)).size(11.5).color(macro_text_color));
            if let Some(m) = view.macro_grid.as_ref() {
                response.on_hover_text(format!("{}\n\n{}", macro_headline(source, m, editing), source.help()));
            }
            // Time.
            let p = view.phase;
            line(
                ui,
                RichText::new(format!(
                    "t = {} · metagen m(t) = {} · P = {}",
                    format_count(p.generation),
                    format_count(p.displayed_metagen),
                    format_count(PERIOD)
                ))
                .size(12.5)
                .strong(),
            )
            .on_hover_text(format!(
                "t is the physical B3/S23 generation. m(t) = max(0, floor((t - 7,408) / {})) is the metagen the \
                 displays show (outside the switching window); here max(0, floor(({} - 7,408) / {})) = {}.",
                format_count(PERIOD),
                format_count(p.generation),
                format_count(PERIOD),
                format_count(displayed_metagen(p.generation))
            ));
            ui.horizontal(|ui| {
                phase_bar(ui, &palette, p.phase, p.generation < PERIOD, 110.0, 8.0);
                let at_readout = p.phase == 0 && p.generation > 0;
                line(
                    ui,
                    RichText::new(format!(
                        "t mod P = {} · {}",
                        format_count(p.phase),
                        if at_readout { "readout time" } else { stage_label(p.stage) }
                    ))
                    .size(11.5)
                    .color(palette.text_muted),
                );
            });
            if !details {
                return;
            }
            let readout = match view.macro_grid.as_ref().map(|m| (m, m.exact_metagen, m.exact_generation)) {
                Some((m, Some(n), Some(t))) => {
                    let clean = m.mismatch_count == 0 && m.ambiguous_count == 0;
                    let text = format!(
                        "Latest readout: metagen {} at t = {} · {}",
                        format_count(n),
                        format_count(t),
                        if clean {
                            "matches the prediction".to_string()
                        } else {
                            format!("{} mismatch, {} ambiguous", m.mismatch_count, m.ambiguous_count)
                        }
                    );
                    RichText::new(text).color(if clean { palette.success } else { palette.error })
                }
                _ => RichText::new(format!("First readout (metagen 0) at t = {}", format_count(PERIOD)))
                    .color(palette.text_muted),
            };
            line(ui, readout.size(11.5));
            ui.separator();
            // Scale and legends.
            ui.horizontal(|ui| {
                if let Some(bar) = scale_bar(ctx.layer.cells_per_pixel, 110.0) {
                    let (rect, _) = ui.allocate_exact_size(vec2(bar.pixels.max(4.0), 10.0), egui::Sense::hover());
                    let y = rect.center().y;
                    let stroke = Stroke::new(1.5, palette.text);
                    ui.painter().line_segment([egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)], stroke);
                    for x in [rect.min.x, rect.max.x] {
                        ui.painter().line_segment([egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)], stroke);
                    }
                    ui.label(RichText::new(bar.label).size(11.5));
                }
                if let Some(scale) = view.scale.as_ref().filter(|_| opacity < 1.0) {
                    let text = if scale.cells_per_texel == 1 {
                        "exact cells".to_string()
                    } else {
                        format!("1 texel = {0} × {0} cells", scale.cells_per_texel)
                    };
                    line(ui, RichText::new(format!("· {text}")).size(11.5).color(palette.text_muted));
                }
            });
            ui.horizontal(|ui| {
                let stops: Vec<Color32> = (0..10)
                    .map(|i| {
                        let [r, g, b] =
                            ctx.settings.colormap.sample_srgb(crate::geometry::display_density(i as f32 / 9.0));
                        Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
                    })
                    .collect();
                let (rect, _) = ui.allocate_exact_size(vec2(80.0, 9.0), egui::Sense::hover());
                for (i, c) in stops.iter().enumerate() {
                    let w = rect.width() / stops.len() as f32;
                    let r = egui::Rect::from_min_size(rect.min + vec2(i as f32 * w, 0.0), vec2(w + 0.5, rect.height()));
                    ui.painter().rect_filled(r, 0.0, *c);
                }
                line(
                    ui,
                    RichText::new("cells: live fraction per texel, few to all").size(11.0).color(palette.text_muted),
                );
            });
            macro_legend(ui, source == MacroSource::Prediction && !editing, source == MacroSource::Live && !editing);
            // Pointer.
            let pointer = match (ctx.tools.hover_cell, ctx.tools.hover_tile) {
                (Some(_), Some((c, r))) if ctx.tools.tool == Tool::Draw => {
                    let on = ctx.data.info.as_ref().is_some_and(|i| i.initial.get(c, r));
                    RichText::new(format!(
                        "Click: switch tile ({c}, {r}) {} · rebuilds at t = 0",
                        if on { "OFF" } else { "ON" }
                    ))
                    .color(palette.text)
                }
                (Some(_), Some((c, r))) if ctx.tools.tool == Tool::Stamp => {
                    let name = crate::tools::stamp_patterns().get(ctx.tools.stamp).map_or("the pattern", |p| p.name);
                    RichText::new(format!("Click: stamp {name} at tile ({c}, {r}) · rebuilds at t = 0"))
                        .color(palette.text)
                }
                (Some(cell), tile) => {
                    let place = match tile {
                        Some((c, r)) => format!(
                            "tile ({c}, {r}) · in-tile ({}, {})",
                            cell.x.rem_euclid(TILE_PITCH),
                            cell.y.rem_euclid(TILE_PITCH)
                        ),
                        None => "outside the tiles".to_string(),
                    };
                    RichText::new(format!("Pointer: cell ({}, {}) · {place}", cell.x, cell.y)).color(palette.text_muted)
                }
                (None, _) => RichText::new("Pointer: not over the world").color(palette.text_muted),
            };
            line(ui, pointer.size(11.0));
        });
    part_labels(ui, &ctx, shape, card.response.rect);
    if details != ctx.settings.overlay_details {
        ctx.settings.overlay_details = details;
    }
    // Where the card is, for framing the world clear of it.
    let origin = ui.clip_rect().min;
    let r = card.response.rect;
    let footprint = OverlayFootprint(Some(ScreenRect {
        min: Vec2::new(r.min.x - origin.x, r.min.y - origin.y),
        max: Vec2::new(r.max.x - origin.x, r.max.y - origin.y),
    }));
    ctx.footprint.set_if_neq(footprint);
}

fn layer_name(source: MacroSource, editing: bool) -> String {
    if editing {
        return "MACRO PATTERN (EDITING)".to_string();
    }
    match source {
        MacroSource::Readout => "MACRO READOUT".to_string(),
        MacroSource::Live => "LIVE CORE READOUT".to_string(),
        MacroSource::Prediction => "ACCELERATED PREVIEW".to_string(),
    }
}

fn macro_color(source: MacroSource, palette: &Palette) -> Color32 {
    match source {
        MacroSource::Prediction => super::rgb32(colors::PREDICTED_ON),
        _ => palette.accent,
    }
}

/// Labels of the inspected tile's parts, painted over the world when the
/// tile is large on screen (the outlines are gizmos).
fn part_labels(ui: &Ui, ctx: &OverlayCtx, shape: GridShape, card: egui::Rect) {
    if !ctx.settings.anatomy || ctx.layer.pixels_per_tile < 160.0 || ctx.layer.macro_opacity >= 0.95 {
        return;
    }
    let Some((col, row)) = anatomy_tile(&ctx.tools).filter(|&(c, r)| shape.contains(c, r)) else { return };
    let Some(info) = &ctx.data.info else { return };
    let Ok((camera, transform)) = ctx.camera.single() else { return };
    // The overlay's clip rectangle is this frame's world area.
    let clip = ui.clip_rect();
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("mp_part_labels")))
        .with_clip_rect(clip);
    let tile = shape.tile_rect(col, row);
    let to_screen = |x: i64, y: i64| -> Option<egui::Pos2> {
        let world = ctx.space.cell_corner_to_world(I64Vec2::new(x, y));
        // Window coordinates (Bevy adds the viewport origin).
        let p = camera.world_to_viewport(transform, world.extend(0.0)).ok()?;
        Some(egui::pos2(p.x, p.y))
    };
    let small = ctx.layer.pixels_per_tile >= 900.0;
    let font = egui::FontId::proportional(11.0);
    for a in tile_annotations(info.active.programming) {
        let r = a.rect.translated(tile.x0, tile.y0);
        let (pos, align, text, color) = match a.kind {
            AnnotationKind::Readout => (
                to_screen(r.x0, r.y0),
                egui::Align2::LEFT_TOP,
                "readout core [250, 1750]^2".to_string(),
                super::rgb32(colors::WORLD_ACCENT),
            ),
            AnnotationKind::Display => {
                (to_screen(r.x0, r.y0), egui::Align2::LEFT_BOTTOM, "display area".to_string(), Color32::from_gray(200))
            }
            AnnotationKind::RuleBit { table, programmed, .. } if small => {
                // Just outside the outline (drawn with a 2-cell halo).
                let (x, align) = if table == metapixel::spec::RuleTable::Birth {
                    (r.x0 - 2, egui::Align2::RIGHT_CENTER)
                } else {
                    (r.x1 + 3, egui::Align2::LEFT_CENTER)
                };
                let color = if programmed { super::rgb32(colors::WORLD_GOOD) } else { Color32::from_gray(160) };
                (to_screen(x, (r.y0 + r.y1) / 2), align, a.label.clone(), color)
            }
            AnnotationKind::Fuse { present, hears, .. } if small => (
                to_screen((r.x0 + r.x1) / 2, r.y0 - 3),
                egui::Align2::CENTER_BOTTOM,
                format!(
                    "{} ({})",
                    a.label,
                    if present { format!("hears {}", hears.name()) } else { "removed".to_string() }
                ),
                super::rgb32(if present { colors::WORLD_ACCENT } else { colors::WORLD_BAD }),
            ),
            _ => continue,
        };
        let Some(pos) = pos else { continue };
        let pos = pos
            + match align {
                egui::Align2::LEFT_TOP => vec2(4.0, 3.0),
                egui::Align2::LEFT_BOTTOM => vec2(4.0, -3.0),
                egui::Align2::RIGHT_CENTER => vec2(-4.0, 0.0),
                egui::Align2::LEFT_CENTER => vec2(4.0, 0.0),
                _ => vec2(0.0, 0.0),
            };
        let galley = painter.layout_no_wrap(text, font.clone(), color);
        let rect = align.anchor_size(pos, galley.size());
        // Never under the overlay card (it is translucent).
        if rect.expand(2.0).intersects(card) {
            continue;
        }
        painter.rect_filled(rect.expand(2.0), 3.0, with_alpha(Color32::from_rgb(10, 12, 17), 0.9));
        painter.galley(rect.min, galley, color);
    }
}
