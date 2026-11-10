//! Vector pictograms of the activities (egui shapes on a 16 × 16 grid, crisp
//! at any size; egui's built-in fonts lack most emoji) and activity badges.

use bevy_egui::egui::{self, Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};
use civilization::Activity;

use crate::palette::{Pictogram, activity_style};

/// Paints a pictogram inside `rect` with `color`.
pub fn paint_pictogram(painter: &egui::Painter, rect: Rect, pictogram: Pictogram, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 16.0;
    let p = |x: f32, y: f32| pos2(c.x + (x - 8.0) * u, c.y + (y - 8.0) * u);
    let line = Stroke::new((1.6 * u).max(1.0), color);
    match pictogram {
        Pictogram::Idle => {
            for x in [4.0, 8.0, 12.0] {
                painter.circle_filled(p(x, 8.5), 1.3 * u, color);
            }
        }
        Pictogram::Eat => {
            painter.circle_filled(p(6.3, 9.5), 3.6 * u, color);
            painter.circle_filled(p(9.7, 9.5), 3.6 * u, color);
            painter.line_segment([p(8.0, 6.0), p(8.6, 2.8)], line);
            painter.add(Shape::convex_polygon(vec![p(8.8, 4.0), p(12.0, 2.4), p(11.0, 4.8)], color, Stroke::NONE));
        }
        Pictogram::Drink => {
            painter.circle_filled(p(8.0, 10.0), 4.0 * u, color);
            painter.add(Shape::convex_polygon(vec![p(8.0, 1.8), p(11.6, 8.4), p(4.4, 8.4)], color, Stroke::NONE));
        }
        Pictogram::Rest => {
            let font = |size: f32| egui::FontId::proportional(size * u);
            painter.text(p(6.0, 9.5), egui::Align2::CENTER_CENTER, "Z", font(10.0), color);
            painter.text(p(12.0, 4.5), egui::Align2::CENTER_CENTER, "z", font(7.0), color);
        }
        Pictogram::Forage => {
            for (x, y) in [(5.5, 10.0), (10.5, 10.0), (8.0, 6.0)] {
                painter.circle_filled(p(x, y), 2.6 * u, color);
            }
            painter.line_segment([p(8.0, 3.4), p(8.0, 1.5)], line);
        }
        Pictogram::Log => {
            painter.add(Shape::convex_polygon(vec![p(8.0, 1.5), p(13.0, 10.0), p(3.0, 10.0)], color, Stroke::NONE));
            painter.rect_filled(Rect::from_min_max(p(7.0, 10.0), p(9.0, 14.5)), 0.0, color);
        }
        Pictogram::Quarry => {
            painter.add(Shape::convex_polygon(
                vec![p(2.0, 13.5), p(4.0, 7.0), p(8.0, 4.0), p(12.5, 6.0), p(14.0, 13.5)],
                color,
                Stroke::NONE,
            ));
            let dark = Stroke::new((1.4 * u).max(1.0), Color32::from_black_alpha(140));
            painter.add(Shape::line(vec![p(8.0, 5.0), p(7.0, 8.5), p(9.0, 10.5), p(8.0, 13.5)], dark));
        }
        Pictogram::Farm => {
            painter.line_segment([p(8.0, 15.0), p(8.0, 3.0)], line);
            for k in 0..4 {
                let y = 4.0 + 2.4 * k as f32;
                for side in [-1.0f32, 1.0] {
                    let centre = p(8.0 + side * 2.0, y + 1.0);
                    painter.add(Shape::ellipse_filled(centre, vec2(1.6 * u, 1.0 * u), color));
                }
            }
        }
        Pictogram::Craft => {
            painter.line_segment([p(4.0, 14.0), p(10.0, 6.0)], Stroke::new(2.0 * u, color));
            painter.add(Shape::convex_polygon(
                vec![p(7.0, 2.5), p(14.0, 7.5), p(12.4, 9.8), p(5.4, 4.8)],
                color,
                Stroke::NONE,
            ));
        }
        Pictogram::Build => {
            painter.add(Shape::convex_polygon(vec![p(8.0, 2.0), p(14.5, 8.0), p(1.5, 8.0)], color, Stroke::NONE));
            painter.rect_filled(Rect::from_min_max(p(3.5, 8.0), p(12.5, 14.0)), 0.0, color);
            painter.rect_filled(Rect::from_min_max(p(7.0, 10.0), p(9.0, 14.0)), 0.0, Color32::from_black_alpha(150));
        }
        Pictogram::Trade => {
            painter.line_segment([p(2.5, 5.5), p(12.5, 5.5)], line);
            painter.add(Shape::convex_polygon(vec![p(14.5, 5.5), p(11.0, 2.5), p(11.0, 8.5)], color, Stroke::NONE));
            painter.line_segment([p(13.5, 11.0), p(3.5, 11.0)], line);
            painter.add(Shape::convex_polygon(vec![p(1.5, 11.0), p(5.0, 8.0), p(5.0, 14.0)], color, Stroke::NONE));
        }
        Pictogram::Talk => {
            painter.rect_filled(Rect::from_min_max(p(1.5, 2.5), p(14.5, 11.0)), 2.5 * u, color);
            painter.add(Shape::convex_polygon(vec![p(4.0, 10.0), p(8.0, 10.0), p(3.5, 14.5)], color, Stroke::NONE));
        }
        Pictogram::Share => {
            painter.rect_filled(Rect::from_min_max(p(2.5, 6.0), p(13.5, 14.0)), 1.0, color);
            let dark = Color32::from_black_alpha(150);
            painter.rect_filled(Rect::from_min_max(p(7.0, 6.0), p(9.0, 14.0)), 0.0, dark);
            painter.rect_filled(Rect::from_min_max(p(2.5, 8.0), p(13.5, 9.5)), 0.0, dark);
            painter.circle_stroke(p(6.0, 4.0), 1.8 * u, line);
            painter.circle_stroke(p(10.0, 4.0), 1.8 * u, line);
        }
        Pictogram::Store => {
            painter.rect_filled(Rect::from_min_max(p(2.0, 3.0), p(14.0, 14.0)), 1.0, color);
            let dark = Stroke::new((1.3 * u).max(1.0), Color32::from_black_alpha(150));
            painter.line_segment([p(2.0, 3.0), p(14.0, 14.0)], dark);
            painter.line_segment([p(14.0, 3.0), p(2.0, 14.0)], dark);
        }
        Pictogram::Steal => {
            painter.add(Shape::convex_polygon(vec![p(8.0, 1.5), p(15.0, 14.0), p(1.0, 14.0)], color, Stroke::NONE));
            let dark = Color32::from_black_alpha(170);
            painter.rect_filled(Rect::from_min_max(p(7.1, 5.5), p(8.9, 10.5)), 0.5, dark);
            painter.circle_filled(p(8.0, 12.2), 1.0 * u, dark);
        }
        Pictogram::Court => {
            painter.circle_filled(p(5.5, 6.0), 3.3 * u, color);
            painter.circle_filled(p(10.5, 6.0), 3.3 * u, color);
            painter.add(Shape::convex_polygon(vec![p(2.3, 7.2), p(13.7, 7.2), p(8.0, 14.0)], color, Stroke::NONE));
        }
        Pictogram::Explore => {
            painter.circle_stroke(p(8.0, 8.0), 6.0 * u, line);
            painter.add(Shape::convex_polygon(vec![p(8.0, 2.8), p(9.6, 8.0), p(6.4, 8.0)], color, Stroke::NONE));
            painter.add(Shape::convex_polygon(
                vec![p(8.0, 13.2), p(6.4, 8.0), p(9.6, 8.0)],
                Color32::from_white_alpha(90).gamma_multiply(0.8),
                Stroke::NONE,
            ));
        }
    }
}

/// Round activity badge (coloured disc with a white pictogram).
pub fn paint_badge(painter: &egui::Painter, centre: Pos2, radius: f32, activity: Activity, alpha: f32) {
    let (pictogram, [r, g, b]) = activity_style(activity);
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    painter.circle(
        centre,
        radius,
        Color32::from_rgba_unmultiplied(r, g, b, a),
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, (f32::from(a) * 0.85) as u8)),
    );
    let inner = Rect::from_center_size(centre, vec2(radius * 1.25, radius * 1.25));
    paint_pictogram(painter, inner, pictogram, Color32::from_rgba_unmultiplied(255, 255, 255, a));
}

/// Allocates a small badge widget inline (legends, inspector).
pub fn badge_widget(ui: &mut egui::Ui, activity: Activity, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_badge(ui.painter(), rect.center(), size * 0.5, activity, 1.0);
    }
    response
}
