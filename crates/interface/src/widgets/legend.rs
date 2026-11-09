//! Colour swatches, legend chips, badges and gradient bars.

use bevy_egui::egui::{self, Color32, CornerRadius, Response, RichText, Sense, Stroke, Ui, Vec2, vec2};

use crate::theme::{hex, mix, with_alpha};

/// A rounded colour square with a hairline border; returns its response
/// (hover it for the hex code tooltip the caller may add).
pub fn swatch(ui: &mut Ui, color: Color32, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
        ui.painter().rect_filled(rect, CornerRadius::same(3), color);
        ui.painter().rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, border), egui::StrokeKind::Inside);
    }
    response
}

/// Swatch followed by a label; hovering shows the colour code.
pub fn legend_chip(ui: &mut Ui, color: Color32, label: &str) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        swatch(ui, color, 12.0);
        ui.label(RichText::new(label).size(13.0));
    })
    .response
    .on_hover_text(hex(color))
}

/// A wrapping row of legend chips.
pub fn legend_row(ui: &mut Ui, entries: &[(Color32, &str)]) {
    ui.horizontal_wrapped(|ui| {
        for (color, label) in entries {
            legend_chip(ui, *color, label);
            ui.add_space(6.0);
        }
    });
}

/// Small pill with text tinted by `color` (status badges, policy badges).
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let font = egui::FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = vec2(galley.size().x + 12.0, galley.size().y + 4.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let fill = with_alpha(color, 0.16);
        ui.painter().rect_filled(rect, CornerRadius::same(255), fill);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(255),
            Stroke::new(1.0, with_alpha(color, 0.5)),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    response
}

/// Colour at `t` in `[0, 1]` along evenly spaced `stops` (linear sRGB
/// interpolation between neighbours). Empty stops give transparent.
pub fn color_ramp(t: f32, stops: &[Color32]) -> Color32 {
    match stops {
        [] => Color32::TRANSPARENT,
        [only] => *only,
        _ => {
            let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
            let scaled = t * (stops.len() - 1) as f32;
            let i = (scaled.floor() as usize).min(stops.len() - 2);
            mix(stops[i], stops[i + 1], scaled - i as f32)
        }
    }
}

/// Horizontal gradient bar with labels at both ends (value legends for
/// continuous fields).
pub fn gradient_bar(ui: &mut Ui, stops: &[Color32], width: f32, low: &str, high: &str) {
    ui.vertical(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
        if ui.is_rect_visible(rect) {
            let steps = 48;
            let step_w = rect.width() / steps as f32;
            for i in 0..steps {
                let t = (i as f32 + 0.5) / steps as f32;
                let r = egui::Rect::from_min_size(rect.min + vec2(i as f32 * step_w, 0.0), vec2(step_w + 0.5, 10.0));
                ui.painter().rect_filled(r, 0.0, color_ramp(t, stops));
            }
            let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
        }
        ui.horizontal(|ui| {
            ui.set_width(width);
            ui.label(RichText::new(low).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(high).small());
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_interpolates_between_stops() {
        let stops = [Color32::BLACK, Color32::from_rgb(200, 0, 0), Color32::WHITE];
        assert_eq!(color_ramp(0.0, &stops), Color32::BLACK);
        assert_eq!(color_ramp(0.5, &stops), Color32::from_rgb(200, 0, 0));
        assert_eq!(color_ramp(1.0, &stops), Color32::WHITE);
        assert_eq!(color_ramp(0.25, &stops), Color32::from_rgb(100, 0, 0));
        assert_eq!(color_ramp(f32::NAN, &stops), Color32::BLACK);
        assert_eq!(color_ramp(7.0, &stops[..1]), Color32::BLACK);
        assert_eq!(color_ramp(0.3, &[]), Color32::TRANSPARENT);
    }
}
