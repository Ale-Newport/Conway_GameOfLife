//! Small widgets shared by the panels: meters, diverging bars, section
//! headers, key/value grids and simulated-time formatting.

use bevy_egui::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, Ui, vec2};
use interface::Palette;
use interface::theme::{mix, with_alpha};

/// "Day 3 · 14:00" for a tick (one tick is one hour, a day 24 ticks; days
/// count from 1).
pub fn clock(tick: u64) -> String {
    let day = tick / 24 + 1;
    let hour = tick % 24;
    format!("Day {day} · {hour:02}:00")
}

/// "1 group", "3 groups".
pub fn count(n: usize, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}

/// Compact duration of `ticks` hours ("5 h", "3 d", "2 d 4 h").
pub fn duration(ticks: u64) -> String {
    let (d, h) = (ticks / 24, ticks % 24);
    match (d, h) {
        (0, h) => format!("{h} h"),
        (d, 0) => format!("{d} d"),
        (d, h) => format!("{d} d {h} h"),
    }
}

/// egui memory key of a pending [`request_scroll`].
const SCROLL_TARGET: &str = "civ_scroll_target";

/// Asks the enclosing scroll area to bring the next [`section`] whose title
/// starts with `title` (case-insensitive) to the top, once it is drawn.
pub fn request_scroll(ctx: &egui::Context, title: &str) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(SCROLL_TARGET), title.to_lowercase()));
}

/// Whether a section titled `title` answers the scroll request `target`.
pub fn scroll_matches(title: &str, target: &str) -> bool {
    !target.is_empty() && title.to_lowercase().starts_with(target)
}

/// Upper-case section title with a hairline.
pub fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(6.0);
    let top = ui.cursor().min;
    ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(palette.text_muted));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, palette.stroke);
    ui.add_space(4.0);
    let id = egui::Id::new(SCROLL_TARGET);
    let target: Option<String> = ui.ctx().data(|d| d.get_temp(id));
    if target.is_some_and(|t| scroll_matches(title, &t)) {
        ui.scroll_to_rect(egui::Rect::from_min_max(top, rect.max), Some(egui::Align::TOP));
        ui.ctx().data_mut(|d| d.remove::<String>(id));
    }
}

/// Widget placed left-aligned in a cell of fixed `width` (a column of
/// labels that line up whatever their length).
pub fn fixed_cell(ui: &mut Ui, width: f32, align: egui::Align, add: impl FnOnce(&mut Ui)) {
    let layout = match align {
        egui::Align::Max => egui::Layout::right_to_left(egui::Align::Center),
        _ => egui::Layout::left_to_right(egui::Align::Center),
    };
    ui.allocate_ui_with_layout(vec2(width, 16.0), layout, |ui| {
        ui.set_width(width);
        add(ui);
    });
}

/// Labelled horizontal meter for a value in `[0, 1]` with a value text.
pub fn meter(ui: &mut Ui, palette: &Palette, label: &str, value: f32, color: Color32, text: &str) -> egui::Response {
    let value = if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        fixed_cell(ui, 104.0, egui::Align::Min, |ui| {
            ui.add(egui::Label::new(RichText::new(label).size(12.5).color(palette.text)).truncate());
        });
        let width = (ui.available_width() - 52.0).max(40.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
        if ui.is_rect_visible(rect) {
            ui.painter().rect_filled(rect, CornerRadius::same(4), palette.surface_hi);
            let mut fill = rect;
            fill.set_width(rect.width() * value);
            if value > 0.0 {
                ui.painter().rect_filled(fill, CornerRadius::same(4), color);
            }
        }
        fixed_cell(ui, 44.0, egui::Align::Max, |ui| {
            ui.label(RichText::new(text).size(12.0).monospace().color(palette.text_muted));
        });
        response
    })
    .inner
}

/// Bar centred at zero for a value in `[-1, 1]` (affinities).
pub fn diverging_bar(ui: &mut Ui, palette: &Palette, value: f32, width: f32) {
    let value = if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(width, 8.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), palette.surface_hi);
    let mid = rect.center().x;
    let end = mid + value * rect.width() * 0.5;
    let colour = if value >= 0.0 { palette.success } else { palette.error };
    let bar = egui::Rect::from_min_max(egui::pos2(mid.min(end), rect.min.y), egui::pos2(mid.max(end), rect.max.y));
    painter.rect_filled(bar, CornerRadius::same(3), colour);
    painter.line_segment(
        [egui::pos2(mid, rect.min.y - 1.0), egui::pos2(mid, rect.max.y + 1.0)],
        Stroke::new(1.0, palette.text_muted),
    );
}

/// Key/value row inside a `Grid`.
pub fn row(ui: &mut Ui, palette: &Palette, key: &str, value: impl Into<RichText>) {
    ui.label(RichText::new(key).size(12.5).color(palette.text_muted));
    ui.label(value.into().size(12.5));
    ui.end_row();
}

/// Card frame used for grouped content.
pub fn card(palette: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(palette.stroke_width, palette.stroke))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
}

/// Translucent frame for the world overlay.
pub fn overlay_frame(palette: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(with_alpha(palette.panel, 0.95))
        .stroke(Stroke::new(palette.stroke_width, with_alpha(palette.stroke, 0.9)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(10, 8))
}

/// Legend chip (swatch and label) allocated as one widget of its exact
/// size, so it wraps correctly inside `horizontal_wrapped` (a nested
/// `horizontal`, as in `interface::widgets::legend_chip`, requests the whole
/// remaining row and never wraps).
pub fn chip(ui: &mut Ui, color: Color32, label: &str, text_color: Color32) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text_color);
    let size = vec2(16.0 + galley.size().x, galley.size().y.max(14.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let swatch = egui::Rect::from_center_size(egui::pos2(rect.min.x + 5.5, rect.center().y), vec2(11.0, 11.0));
        ui.painter().rect_filled(swatch, CornerRadius::same(3), color);
        ui.painter().rect_stroke(
            swatch,
            CornerRadius::same(3),
            Stroke::new(1.0, Color32::from_black_alpha(90)),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(egui::pos2(rect.min.x + 16.0, rect.center().y - galley.size().y / 2.0), galley, text_color);
    }
    response
}

/// Segmented choice: a row of selectable buttons; returns true when changed.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, palette: &Palette, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
        for (option, label) in options {
            let on = *value == *option;
            let text = RichText::new(*label).size(12.5).color(if on { palette.on_accent } else { palette.text });
            let button = egui::Button::new(text)
                .fill(if on { palette.accent } else { mix(palette.surface, palette.surface_hi, 0.5) })
                .corner_radius(CornerRadius::same(5));
            if ui.add(button).clicked() && !on {
                *value = *option;
                changed = true;
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_time_formats() {
        assert_eq!(clock(0), "Day 1 · 00:00");
        assert_eq!(clock(24 * 3 + 14), "Day 4 · 14:00");
        assert_eq!(duration(5), "5 h");
        assert_eq!(duration(48), "2 d");
        assert_eq!(duration(52), "2 d 4 h");
        assert_eq!(count(1, "group", "groups"), "1 group");
        assert_eq!(count(0, "person", "people"), "0 people");
    }

    #[test]
    fn scroll_requests_match_section_prefixes() {
        assert!(scroll_matches("Relationships (12)", "relationships"));
        assert!(scroll_matches("Exchange rates (MRS)", "exchange"));
        assert!(!scroll_matches("Skills", "needs"));
        assert!(!scroll_matches("Skills", ""), "an empty request matches nothing");
    }
}
