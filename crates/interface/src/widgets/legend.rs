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
    gradient_bar_with_mid(ui, stops, width, low, None, high);
}

/// [`gradient_bar`] with an optional `mid` label centred under the bar, on
/// the row of the end labels (the value in the middle of a non-linear scale).
pub fn gradient_bar_with_mid(ui: &mut Ui, stops: &[Color32], width: f32, low: &str, mid: Option<&str>, high: &str) {
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
        let row = ui
            .horizontal(|ui| {
                ui.set_width(width);
                ui.label(RichText::new(low).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(high).small());
                });
            })
            .response
            .rect;
        if let Some(mid) = mid {
            let color = ui.visuals().text_color();
            let font = egui::TextStyle::Small.resolve(ui.style());
            let galley = ui.painter().layout_no_wrap(mid.to_owned(), font, color);
            let centre = egui::pos2(rect.center().x, row.center().y);
            ui.painter().galley(centre - galley.size() / 2.0, galley, color);
        }
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

    /// Texts painted by one frame, with their rectangles.
    fn painted_labels(draw: impl FnMut(&mut Ui)) -> Vec<(String, egui::Rect)> {
        let output = crate::run_headless_ui(&egui::Context::default(), draw);
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                // `pos` is the anchor of the galley's alignment (right-aligned labels end there).
                egui::Shape::Text(text) => {
                    Some((text.galley.text().to_owned(), text.galley.rect.translate(text.pos.to_vec2())))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn gradient_bar_centres_the_mid_label_on_the_row_of_the_end_labels() {
        let stops = [Color32::BLACK, Color32::WHITE];
        let labels = painted_labels(|ui| gradient_bar_with_mid(ui, &stops, 200.0, "1", Some("16"), "255"));
        let [(low, low_rect), (high, high_rect), (mid, mid_rect)] = &labels[..] else { panic!("{labels:?}") };
        assert_eq!([low, high, mid], ["1", "255", "16"]);
        // Within half a pixel: widgets are placed on whole pixels.
        assert!((high_rect.right() - low_rect.left() - 200.0).abs() < 0.5, "the end labels span the bar");
        let bar_centre = (low_rect.left() + high_rect.right()) / 2.0;
        assert!((mid_rect.center().x - bar_centre).abs() < 0.5, "{mid_rect:?} not centred under the bar");
        assert!((mid_rect.center().y - low_rect.center().y).abs() < 0.5, "{mid_rect:?} not on the row of {low_rect:?}");
        // Without a middle label it is the plain gradient bar.
        let plain = painted_labels(|ui| gradient_bar(ui, &stops, 200.0, "1", "255"));
        assert_eq!(plain, labels[..2]);
    }
}
