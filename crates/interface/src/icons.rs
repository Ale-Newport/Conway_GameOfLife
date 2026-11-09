//! Vector icons painted with egui shapes (crisp at any scale, no icon font
//! needed) and icon buttons.

use std::f32::consts::PI;

use bevy_egui::egui::{self, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use crate::theme::{Palette, mix, with_alpha};

/// Icons used by the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// ▶
    Play,
    /// ❚❚
    Pause,
    /// ▶|
    Step,
    /// ▶▶|
    StepMany,
    /// Circular arrow.
    Reset,
    /// Curved arrow to the left.
    Undo,
    /// Curved arrow to the right.
    Redo,
    /// Eraser-like crossed square.
    Clear,
    /// Die with five pips.
    Dice,
    /// House.
    Home,
    /// Floppy disk.
    Save,
    /// Folder.
    Open,
    /// Tray with an outgoing arrow.
    Export,
    /// Camera.
    Camera,
    /// Four corners.
    Fullscreen,
    /// Open book.
    Book,
    /// Circle with a question mark.
    Help,
    /// Circle with an "i".
    Info,
    /// Half-filled circle.
    Theme,
    /// Panel with a highlighted left strip.
    PanelLeft,
    /// Panel with a highlighted right strip.
    PanelRight,
    /// Trash can.
    Trash,
    /// Chevron pointing right.
    ChevronRight,
}

fn stroke(color: Color32, width: f32) -> Stroke {
    Stroke::new(width, color)
}

fn arc(center: Pos2, radius: f32, from: f32, to: f32, segments: usize) -> Vec<Pos2> {
    (0..=segments)
        .map(|i| {
            let a = from + (to - from) * i as f32 / segments as f32;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect()
}

fn arrow_head(painter: &egui::Painter, tip: Pos2, direction: Vec2, size: f32, color: Color32) {
    let d = direction.normalized();
    let n = vec2(-d.y, d.x);
    let base = tip - d * size;
    painter.add(Shape::convex_polygon(vec![tip, base + n * size * 0.6, base - n * size * 0.6], color, Stroke::NONE));
}

/// Paints `icon` inside `rect` (square works best) with `color`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 16.0; // design grid: 16 x 16
    let p = |x: f32, y: f32| pos2(c.x + (x - 8.0) * u, c.y + (y - 8.0) * u);
    let w = (1.6 * u).max(1.2);
    let line = stroke(color, w);
    match icon {
        Icon::Play => {
            painter.add(Shape::convex_polygon(vec![p(4.5, 3.0), p(13.0, 8.0), p(4.5, 13.0)], color, Stroke::NONE));
        }
        Icon::Pause => {
            painter.rect_filled(Rect::from_min_max(p(4.0, 3.0), p(7.0, 13.0)), 1.0, color);
            painter.rect_filled(Rect::from_min_max(p(9.0, 3.0), p(12.0, 13.0)), 1.0, color);
        }
        Icon::Step => {
            painter.add(Shape::convex_polygon(vec![p(3.5, 3.5), p(10.5, 8.0), p(3.5, 12.5)], color, Stroke::NONE));
            painter.rect_filled(Rect::from_min_max(p(11.0, 3.5), p(13.0, 12.5)), 0.5, color);
        }
        Icon::StepMany => {
            painter.add(Shape::convex_polygon(vec![p(1.5, 4.0), p(6.5, 8.0), p(1.5, 12.0)], color, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![p(6.5, 4.0), p(11.5, 8.0), p(6.5, 12.0)], color, Stroke::NONE));
            painter.rect_filled(Rect::from_min_max(p(12.0, 4.0), p(14.0, 12.0)), 0.5, color);
        }
        Icon::Reset => {
            let points = arc(p(8.0, 8.5), 5.0 * u, -PI * 0.35, PI * 1.45, 24);
            let tip = points[0];
            painter.add(Shape::line(points, line));
            arrow_head(painter, tip + vec2(1.2 * u, -1.6 * u), vec2(1.0, -1.0), 3.2 * u, color);
        }
        Icon::Undo | Icon::Redo => {
            let mirror = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let q = |x: f32, y: f32| p(8.0 + (x - 8.0) * mirror, y);
            // Half circle on the right (left for redo) from the top to the bottom.
            let mut points: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = -PI / 2.0 + PI * i as f32 / 16.0;
                    q(8.5 + 4.5 * a.cos(), 9.0 + 4.5 * a.sin())
                })
                .collect();
            points.push(q(5.0, 13.5));
            painter.add(Shape::line(points, line));
            painter.line_segment([q(8.5, 4.5), q(4.5, 4.5)], line);
            arrow_head(painter, q(2.0, 4.5), vec2(-mirror, 0.0), 3.4 * u, color);
        }
        Icon::Clear => {
            painter.rect_stroke(Rect::from_min_max(p(3.0, 3.0), p(13.0, 13.0)), 2.0, line, egui::StrokeKind::Middle);
            painter.line_segment([p(5.5, 5.5), p(10.5, 10.5)], line);
            painter.line_segment([p(10.5, 5.5), p(5.5, 10.5)], line);
        }
        Icon::Dice => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.5, 2.5), p(13.5, 13.5)),
                2.5 * u,
                line,
                egui::StrokeKind::Middle,
            );
            for (x, y) in [(5.5, 5.5), (10.5, 5.5), (8.0, 8.0), (5.5, 10.5), (10.5, 10.5)] {
                painter.circle_filled(p(x, y), 1.1 * u, color);
            }
        }
        Icon::Home => {
            painter.add(Shape::line(vec![p(2.0, 8.0), p(8.0, 2.5), p(14.0, 8.0)], line));
            painter.add(Shape::line(vec![p(4.0, 6.5), p(4.0, 13.5), p(12.0, 13.5), p(12.0, 6.5)], line));
            painter.rect_filled(Rect::from_min_max(p(6.8, 9.5), p(9.2, 13.5)), 0.5, color);
        }
        Icon::Save => {
            painter.add(Shape::closed_line(
                vec![p(2.5, 2.5), p(11.5, 2.5), p(13.5, 4.5), p(13.5, 13.5), p(2.5, 13.5)],
                line,
            ));
            painter.rect_filled(Rect::from_min_max(p(5.0, 2.5), p(10.0, 6.0)), 0.5, color);
            painter.rect_stroke(Rect::from_min_max(p(5.0, 9.0), p(11.0, 13.5)), 0.5, line, egui::StrokeKind::Middle);
        }
        Icon::Open => {
            painter.add(Shape::closed_line(
                vec![p(1.5, 3.5), p(6.0, 3.5), p(7.5, 5.0), p(14.5, 5.0), p(14.5, 13.0), p(1.5, 13.0)],
                line,
            ));
            painter.line_segment([p(1.5, 7.0), p(14.5, 7.0)], line);
        }
        Icon::Export => {
            painter.add(Shape::line(vec![p(2.5, 9.5), p(2.5, 13.5), p(13.5, 13.5), p(13.5, 9.5)], line));
            painter.line_segment([p(8.0, 10.5), p(8.0, 4.0)], line);
            arrow_head(painter, p(8.0, 1.8), vec2(0.0, -1.0), 3.6 * u, color);
        }
        Icon::Camera => {
            painter.add(Shape::closed_line(
                vec![
                    p(1.5, 5.0),
                    p(5.0, 5.0),
                    p(6.5, 3.0),
                    p(9.5, 3.0),
                    p(11.0, 5.0),
                    p(14.5, 5.0),
                    p(14.5, 13.0),
                    p(1.5, 13.0),
                ],
                line,
            ));
            painter.circle_stroke(p(8.0, 9.0), 2.6 * u, line);
        }
        Icon::Fullscreen => {
            for (corner, dx, dy) in
                [((2.5, 2.5), 1.0, 1.0), ((13.5, 2.5), -1.0, 1.0), ((2.5, 13.5), 1.0, -1.0), ((13.5, 13.5), -1.0, -1.0)]
            {
                let o = p(corner.0, corner.1);
                painter.add(Shape::line(vec![o + vec2(0.0, 4.0 * u * dy), o, o + vec2(4.0 * u * dx, 0.0)], line));
            }
        }
        Icon::Book => {
            painter.add(Shape::closed_line(vec![p(8.0, 4.0), p(2.0, 3.0), p(2.0, 12.5), p(8.0, 13.5)], line));
            painter.add(Shape::closed_line(vec![p(8.0, 4.0), p(14.0, 3.0), p(14.0, 12.5), p(8.0, 13.5)], line));
        }
        Icon::Help | Icon::Info => {
            painter.circle_stroke(c, 6.2 * u, line);
            let text = if icon == Icon::Help { "?" } else { "i" };
            painter.text(
                c + vec2(0.0, 0.3 * u),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(9.5 * u),
                color,
            );
        }
        Icon::Theme => {
            painter.circle_stroke(c, 6.0 * u, line);
            let mut half = arc(c, 6.0 * u, -PI / 2.0, PI / 2.0, 20);
            half.push(c);
            painter.add(Shape::convex_polygon(half, color, Stroke::NONE));
        }
        Icon::PanelLeft | Icon::PanelRight => {
            let frame = Rect::from_min_max(p(1.5, 2.5), p(14.5, 13.5));
            painter.rect_stroke(frame, 2.0, line, egui::StrokeKind::Middle);
            let strip = if icon == Icon::PanelLeft {
                Rect::from_min_max(p(1.5, 2.5), p(6.0, 13.5))
            } else {
                Rect::from_min_max(p(10.0, 2.5), p(14.5, 13.5))
            };
            painter.rect_filled(strip, 2.0, with_alpha(color, 0.7));
        }
        Icon::Trash => {
            painter.line_segment([p(2.5, 4.0), p(13.5, 4.0)], line);
            painter.add(Shape::line(vec![p(6.0, 4.0), p(6.5, 2.0), p(9.5, 2.0), p(10.0, 4.0)], line));
            painter.add(Shape::line(vec![p(4.0, 4.0), p(5.0, 14.0), p(11.0, 14.0), p(12.0, 4.0)], line));
            painter.line_segment([p(8.0, 6.5), p(8.0, 11.5)], line);
        }
        Icon::ChevronRight => {
            painter.add(Shape::line(vec![p(6.0, 3.0), p(11.0, 8.0), p(6.0, 13.0)], line));
        }
    }
}

/// Square icon button. `active` draws it as toggled on.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str, enabled: bool, active: bool, palette: &Palette) -> Response {
    let size = Vec2::splat(30.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let hovered = enabled && response.hovered();
        let fill = if active {
            mix(palette.surface_hi, palette.accent, 0.35)
        } else if hovered {
            palette.surface_hi
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 6.0, fill);
        if active {
            ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, palette.accent), egui::StrokeKind::Inside);
        }
        let color = if !enabled {
            with_alpha(palette.text_muted, 0.45)
        } else if active || hovered {
            palette.text
        } else {
            mix(palette.text, palette.text_muted, 0.35)
        };
        paint_icon(ui.painter(), rect.shrink(7.0), icon, color);
    }
    let response = response.on_hover_text(tooltip);
    if enabled { response } else { response.on_disabled_hover_text(tooltip) }
}

/// Accent-filled button for the primary action of a dialog.
pub fn primary_button(ui: &mut Ui, text: &str, palette: &Palette) -> Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).strong().color(palette.on_accent))
            .fill(palette.accent)
            .min_size(vec2(96.0, 30.0)),
    )
}

/// The application mark: a 3 x 3 cell motif (a glider) in the accent colour.
pub fn paint_logo(painter: &egui::Painter, rect: Rect, accent: Color32, dim: Color32) {
    let cell = rect.width().min(rect.height()) / 3.0;
    let glider = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
    for y in 0..3 {
        for x in 0..3 {
            let min = rect.min + vec2(x as f32 * cell, y as f32 * cell);
            let r = Rect::from_min_size(min, Vec2::splat(cell)).shrink(cell * 0.12);
            let on = glider.contains(&(x, y));
            painter.rect_filled(r, cell * 0.22, if on { accent } else { dim });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_paints_inside_its_rect() {
        let ctx = egui::Context::default();
        let icons = [
            Icon::Play,
            Icon::Pause,
            Icon::Step,
            Icon::StepMany,
            Icon::Reset,
            Icon::Undo,
            Icon::Redo,
            Icon::Clear,
            Icon::Dice,
            Icon::Home,
            Icon::Save,
            Icon::Open,
            Icon::Export,
            Icon::Camera,
            Icon::Fullscreen,
            Icon::Book,
            Icon::Help,
            Icon::Info,
            Icon::Theme,
            Icon::PanelLeft,
            Icon::PanelRight,
            Icon::Trash,
            Icon::ChevronRight,
        ];
        let output = crate::run_headless_ui(&ctx, |ui| {
            for (i, icon) in icons.iter().enumerate() {
                let rect = Rect::from_min_size(pos2(i as f32 * 40.0, 0.0), Vec2::splat(32.0));
                paint_icon(ui.painter(), rect, *icon, Color32::WHITE);
            }
        });
        let shapes = output.shapes;
        assert!(shapes.len() >= icons.len());
        for clipped in shapes {
            let bounds = clipped.shape.visual_bounding_rect();
            if bounds.is_positive() {
                let slot = (bounds.center().x / 40.0).floor();
                let rect = Rect::from_min_size(pos2(slot * 40.0, 0.0), Vec2::splat(32.0)).expand(2.0);
                assert!(rect.contains_rect(bounds), "shape {bounds:?} escapes slot {slot}");
            }
        }
    }
}
