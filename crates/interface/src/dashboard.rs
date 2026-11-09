//! The dashboard: hero header and one card per laboratory.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};

use crate::commands::ShellCommand;
use crate::icons::paint_logo;
use crate::layout::card_grid;
use crate::mode::{AppMode, ModeCard, ModeRegistry, ModeStatus};
use crate::previews::{paint_preview, soft_glow};
use crate::shell::Frame;
use crate::theme::{Palette, mix, with_alpha};

const CARD_HEIGHT: f32 = 318.0;
const PREVIEW_HEIGHT: f32 = 132.0;
const GAP: f32 = 18.0;
const MAX_CONTENT_WIDTH: f32 = 1240.0;

pub(crate) fn show(root: &mut Ui, world: &mut World, frame: &Frame) {
    let p = frame.palette;
    let registry = world.resource::<ModeRegistry>().clone();
    let mut open = None;
    egui::CentralPanel::default().frame(egui::Frame::new().fill(p.background)).show(root, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let full = ui.available_width();
            let width = full.min(MAX_CONTENT_WIDTH) - 48.0;
            let side = ((full - width) / 2.0).max(0.0);
            ui.add_space(28.0);
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    hero(ui, &registry, &p, width);
                    ui.add_space(26.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Laboratories").size(18.0).strong().color(p.text));
                        ui.label(RichText::new("Choose a laboratory to open it.").size(13.0).color(p.text_muted));
                    });
                    ui.add_space(10.0);
                    let time = ui.input(|i| i.time);
                    let (cols, card_w) = card_grid(width, GAP, 330.0, 3);
                    for row in AppMode::SIMULATIONS.chunks(cols) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = GAP;
                            for mode in row {
                                if card(ui, *mode, registry.card(*mode), &p, card_w, time) {
                                    open = Some(*mode);
                                }
                            }
                        });
                        ui.add_space(GAP);
                    }
                    footer(ui, &p);
                    ui.add_space(24.0);
                });
            });
        });
    });
    root.ctx().request_repaint();
    if let Some(mode) = open {
        world.write_message(ShellCommand::OpenMode(mode));
    }
}

fn hero(ui: &mut Ui, registry: &ModeRegistry, p: &Palette, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 168.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let brand = AppMode::Dashboard.default_accent();
    painter.rect_filled(rect, CornerRadius::same(16), p.surface);
    // Soft accent glows in the corners.
    soft_glow(&painter, rect.right_top() + vec2(-120.0, 50.0), 260.0, brand, 0.16);
    soft_glow(&painter, rect.right_bottom() + vec2(-330.0, 10.0), 180.0, AppMode::ClassicConway.default_accent(), 0.10);
    painter.rect_stroke(rect, CornerRadius::same(16), Stroke::new(p.stroke_width, p.stroke), egui::StrokeKind::Inside);
    let logo = egui::Rect::from_min_size(rect.min + vec2(30.0, 34.0), egui::Vec2::splat(58.0));
    paint_logo(&painter, logo, brand, mix(p.surface_hi, p.text_muted, 0.15));
    let text_x = logo.max.x + 24.0;
    painter.text(
        egui::pos2(text_x, rect.min.y + 30.0),
        Align2::LEFT_TOP,
        "Emergence Lab",
        FontId::proportional(34.0),
        p.text,
    );
    painter.text(
        egui::pos2(text_x, rect.min.y + 76.0),
        Align2::LEFT_TOP,
        "A laboratory for artificial life — from Conway's Game of Life to evolving ecosystems and emergent societies.",
        FontId::proportional(15.0),
        p.text_muted,
    );
    let available = registry.cards().len();
    let chips = [
        format!("{available} of {} laboratories available", AppMode::SIMULATIONS.len()),
        "Seeded, reproducible runs".to_string(),
        "Save & replay any state".to_string(),
        "CSV / JSON / PNG exports".to_string(),
    ];
    let mut x = text_x;
    for (i, chip) in chips.iter().enumerate() {
        let color = if i == 0 { brand } else { p.text_muted };
        let galley = painter.layout_no_wrap(chip.clone(), FontId::proportional(12.5), color);
        let r = egui::Rect::from_min_size(egui::pos2(x, rect.min.y + 116.0), galley.size() + vec2(20.0, 10.0));
        if r.max.x > rect.max.x - 20.0 {
            break;
        }
        painter.rect_filled(r, CornerRadius::same(255), with_alpha(color, 0.10));
        painter.rect_stroke(
            r,
            CornerRadius::same(255),
            Stroke::new(1.0, with_alpha(color, 0.35)),
            egui::StrokeKind::Inside,
        );
        painter.galley(r.min + vec2(10.0, 5.0), galley, color);
        x = r.max.x + 8.0;
    }
}

/// Draws one card; returns true when it was clicked and can be opened.
fn card(ui: &mut Ui, mode: AppMode, registered: Option<&ModeCard>, p: &Palette, width: f32, time: f64) -> bool {
    let defaults = ModeCard::defaults(mode);
    let info = registered.unwrap_or(&defaults);
    let openable = registered.is_some();
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, CARD_HEIGHT), if openable { Sense::click() } else { Sense::hover() });
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let hovered = openable && response.hovered();
    let painter = ui.painter_at(rect.expand(2.0));
    let fill = if hovered { mix(p.surface, info.accent, 0.05) } else { p.surface };
    painter.rect_filled(rect, CornerRadius::same(14), fill);
    let border = if hovered { info.accent } else { p.stroke };
    painter.rect_stroke(
        rect,
        CornerRadius::same(14),
        Stroke::new(p.stroke_width.max(if hovered { 1.5 } else { 1.0 }), border),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(14.0);
    let preview = egui::Rect::from_min_size(inner.min, vec2(inner.width(), PREVIEW_HEIGHT));
    let preview_time = if openable { time } else { time * 0.4 };
    paint_preview(&painter, preview, mode, preview_time, info.accent, p, openable);

    // Status badge in the preview's top-right corner.
    let (label, color) = match (openable, info.status) {
        (false, _) => ("In development", p.text_muted),
        (true, ModeStatus::Preview) => (ModeStatus::Preview.label(), p.warning),
        (true, ModeStatus::Ready) => (ModeStatus::Ready.label(), p.success),
    };
    let galley = painter.layout_no_wrap(label.to_string(), FontId::proportional(11.0), color);
    let pill = egui::Rect::from_min_size(
        egui::pos2(preview.max.x - galley.size().x - 20.0, preview.min.y + 8.0),
        galley.size() + vec2(12.0, 4.0),
    );
    painter.rect_filled(pill, CornerRadius::same(255), mix(p.surface, color, 0.18));
    painter.rect_stroke(
        pill,
        CornerRadius::same(255),
        Stroke::new(1.0, with_alpha(color, 0.5)),
        egui::StrokeKind::Inside,
    );
    painter.galley(pill.min + vec2(6.0, 2.0), galley, color);

    let footer = egui::Rect::from_min_max(egui::pos2(inner.min.x, inner.max.y - 30.0), inner.max);
    let text_area =
        egui::Rect::from_min_max(egui::pos2(inner.min.x, preview.max.y + 12.0), egui::pos2(inner.max.x, footer.min.y));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(text_area).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(text_area);
    child.horizontal(|ui| {
        ui.label(RichText::new(&info.glyph).size(17.0).color(info.accent));
        ui.add(egui::Label::new(RichText::new(&info.title).size(17.0).strong().color(p.text)).truncate());
    });
    child.label(RichText::new(&info.tagline).size(13.0).color(if openable { info.accent } else { p.text_muted }));
    child.add_space(2.0);
    child.add(egui::Label::new(RichText::new(&info.description).size(13.0).color(p.text_muted)).wrap());

    if openable {
        let text = "Open laboratory  ›";
        let color = if hovered { info.accent } else { mix(p.text, info.accent, 0.5) };
        painter.text(footer.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(14.0), color);
    } else {
        painter.text(
            footer.left_center(),
            Align2::LEFT_CENTER,
            "Not available in this build yet",
            FontId::proportional(13.0),
            with_alpha(p.text_muted, 0.8),
        );
    }
    let response = if openable {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response.on_hover_text("This laboratory is still in development and cannot be opened yet.")
    };
    openable && response.clicked()
}

fn footer(ui: &mut Ui, p: &Palette) {
    ui.horizontal_wrapped(|ui| {
        let muted = |t: &str| RichText::new(t).size(12.0).color(p.text_muted);
        ui.label(muted(&format!("Emergence Lab {}", env!("CARGO_PKG_VERSION"))));
        ui.label(muted("·"));
        ui.label(muted("F1 help · F11 fullscreen · F12 screenshot"));
        ui.label(muted("·"));
        ui.label(muted("Open source under MIT OR Apache-2.0; asset licences in About."));
    });
}
