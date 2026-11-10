//! Small egui helpers shared by the panels, and the region distance matrix.

use bevy_egui::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, Ui, pos2, vec2};
use evolution::PopulationStats;
use interface::Palette;
use interface::theme::{mix, with_alpha};
use rendering::Colormap;

use crate::ui_state::MatrixMeasure;

/// `[r, g, b]` (0..1 sRGB) → egui colour.
pub fn color32(rgb: [f32; 3]) -> Color32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(b(rgb[0]), b(rgb[1]), b(rgb[2]))
}

/// Colormap sample as an egui colour.
pub fn colormap(map: Colormap, t: f32) -> Color32 {
    color32(map.sample_srgb(t))
}

/// Upper-case section caption with a hairline.
pub fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title.to_uppercase()).size(10.5).strong().color(palette.text_muted));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, palette.stroke);
    ui.add_space(4.0);
}

/// Muted explanatory text.
pub fn note(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(palette.text_muted));
}

/// Card frame on the surface colour.
pub fn card(palette: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(palette.stroke_width, palette.stroke))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
}

/// Segmented control over `options`; returns true if the value changed.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, palette: &Palette, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (option, label) in options {
            let on = *value == *option;
            let text = RichText::new(*label).size(12.0).color(if on { palette.on_accent } else { palette.text });
            let button = egui::Button::new(text)
                .fill(if on { palette.accent } else { palette.surface })
                .stroke(Stroke::new(1.0, if on { palette.accent } else { palette.stroke }))
                .corner_radius(CornerRadius::same(4));
            if ui.add(button).clicked() && !on {
                *value = *option;
                changed = true;
            }
        }
    });
    changed
}

/// A horizontal bar showing `t ∈ [0, 1]` within a range, with an optional
/// reference tick (e.g. the population mean).
pub fn value_bar(
    ui: &mut Ui,
    palette: &Palette,
    t: f32,
    reference: Option<f32>,
    color: Color32,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let track = rect.shrink2(vec2(0.0, 2.0));
        painter.rect_filled(track, CornerRadius::same(3), palette.surface_hi);
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let fill = egui::Rect::from_min_max(track.min, pos2(track.left() + track.width() * t, track.bottom()));
        painter.rect_filled(fill, CornerRadius::same(3), with_alpha(color, 0.85));
        if let Some(r) = reference.filter(|r| r.is_finite()) {
            let x = track.left() + track.width() * r.clamp(0.0, 1.0);
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.5, palette.text));
        }
    }
    response
}

/// Distance matrix between prey regions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionMatrix {
    /// Regions in display order (ids).
    pub regions: Vec<u16>,
    /// Prey per region.
    pub prey: Vec<u32>,
    /// `n × n` values (row-major); `None` on the diagonal and for pairs
    /// the model did not compare.
    pub values: Vec<Option<f64>>,
    /// Largest value (for the colour scale).
    pub max: f64,
}

/// Builds the matrix from the model's pairwise comparisons (the six most
/// populous prey regions).
pub fn region_matrix(stats: &PopulationStats, measure: MatrixMeasure) -> RegionMatrix {
    let mut regions: Vec<u16> = stats.divergence.iter().flat_map(|d| [d.a, d.b]).collect();
    regions.sort_unstable();
    regions.dedup();
    let n = regions.len();
    let mut values = vec![None; n * n];
    let mut max: f64 = 0.0;
    for d in &stats.divergence {
        let (Ok(i), Ok(j)) = (regions.binary_search(&d.a), regions.binary_search(&d.b)) else { continue };
        let v = match measure {
            MatrixMeasure::Net => d.net,
            MatrixMeasure::Between => d.between,
        };
        if v.is_finite() {
            values[i * n + j] = Some(v);
            values[j * n + i] = Some(v);
            max = max.max(v);
        }
    }
    let prey = regions.iter().map(|r| stats.regions.iter().find(|s| s.region == *r).map_or(0, |s| s.prey)).collect();
    RegionMatrix { regions, prey, values, max }
}

/// Draws a region matrix as a heatmap; returns the hovered cell.
pub fn matrix_heatmap(ui: &mut Ui, palette: &Palette, matrix: &RegionMatrix) -> Option<(usize, usize)> {
    let n = matrix.regions.len();
    if n == 0 {
        return None;
    }
    let label_w = 34.0;
    let width = ui.available_width().min(330.0);
    let cell = ((width - label_w) / n as f32).clamp(18.0, 52.0);
    let size = vec2(label_w + cell * n as f32, label_w * 0.6 + cell * n as f32);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    let origin = rect.min + vec2(label_w, label_w * 0.6);
    let font = egui::FontId::proportional(10.5);
    let scale = if matrix.max > 0.0 { matrix.max } else { 1.0 };
    let mut hovered = None;
    for (i, r) in matrix.regions.iter().enumerate() {
        painter.text(
            pos2(origin.x - 4.0, origin.y + (i as f32 + 0.5) * cell),
            egui::Align2::RIGHT_CENTER,
            format!("R{r}"),
            font.clone(),
            palette.text_muted,
        );
        painter.text(
            pos2(origin.x + (i as f32 + 0.5) * cell, origin.y - 4.0),
            egui::Align2::CENTER_BOTTOM,
            format!("R{r}"),
            font.clone(),
            palette.text_muted,
        );
        for j in 0..n {
            let cell_rect = egui::Rect::from_min_size(
                pos2(origin.x + j as f32 * cell, origin.y + i as f32 * cell),
                vec2(cell, cell),
            )
            .shrink(1.0);
            let value = matrix.values[i * n + j];
            let fill = match value {
                Some(v) => colormap(Colormap::Magma, (0.15 + 0.85 * (v / scale)) as f32),
                None => palette.surface_hi,
            };
            painter.rect_filled(cell_rect, CornerRadius::same(2), fill);
            if let Some(v) = value
                && cell >= 30.0
            {
                let t = (v / scale) as f32;
                let text_color = if t > 0.55 { Color32::BLACK } else { Color32::WHITE };
                painter.text(
                    cell_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{v:.3}"),
                    font.clone(),
                    text_color,
                );
            }
            if response.hover_pos().is_some_and(|p| cell_rect.contains(p)) {
                hovered = Some((i, j));
                painter.rect_stroke(
                    cell_rect,
                    CornerRadius::same(2),
                    Stroke::new(1.5, palette.text),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
    hovered
}

/// Colour of a temperature on the thermal scale for legends.
pub fn thermal_color32(t: f32) -> Color32 {
    color32(crate::morph::thermal_rgb(t))
}

/// Accent tinted towards the surface (for subtle fills).
pub fn soft_accent(palette: &Palette) -> Color32 {
    mix(palette.surface, palette.accent, 0.18)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolution::stats::{RegionDivergence, RegionStatistics};

    #[test]
    fn region_matrix_is_symmetric_with_an_empty_diagonal() {
        let stats = PopulationStats {
            divergence: vec![
                RegionDivergence { a: 0, b: 2, between: 0.2, net: 0.05 },
                RegionDivergence { a: 0, b: 1, between: 0.3, net: 0.1 },
                RegionDivergence { a: 1, b: 2, between: 0.25, net: f64::NAN },
            ],
            regions: vec![
                RegionStatistics { region: 0, prey: 10, predators: 0, mean_genome: vec![] },
                RegionStatistics { region: 2, prey: 7, predators: 1, mean_genome: vec![] },
            ],
            ..PopulationStats::default()
        };
        let m = region_matrix(&stats, MatrixMeasure::Net);
        assert_eq!(m.regions, vec![0, 1, 2]);
        assert_eq!(m.prey, vec![10, 0, 7]);
        assert_eq!(m.values[1], Some(0.1));
        assert_eq!(m.values[3], Some(0.1), "symmetric");
        assert_eq!(m.values[0], None, "diagonal");
        assert_eq!(m.values[5], None, "non-finite values are skipped");
        assert_eq!(m.max, 0.1);
        let b = region_matrix(&stats, MatrixMeasure::Between);
        assert_eq!(b.values[2], Some(0.2));
        assert_eq!(b.max, 0.3);
        assert!(region_matrix(&PopulationStats::default(), MatrixMeasure::Net).regions.is_empty());
    }

    #[test]
    fn colours_convert() {
        assert_eq!(color32([1.0, 0.0, 0.5]), Color32::from_rgb(255, 0, 128));
        assert_ne!(thermal_color32(0.0), thermal_color32(1.0));
    }
}

#[cfg(test)]
mod glyph_tests {
    use bevy_egui::egui;

    /// Text shown in the UI comes from these files; every non-ASCII
    /// character in them must exist in egui's default fonts (otherwise it
    /// is drawn as a box).
    const SOURCES: [&str; 8] = [
        include_str!("tools_panel.rs"),
        include_str!("properties.rs"),
        include_str!("overlay.rs"),
        include_str!("tutorial.rs"),
        include_str!("interaction.rs"),
        include_str!("genealogy.rs"),
        include_str!("ui_state.rs"),
        include_str!("session.rs"),
    ];

    #[test]
    fn ui_text_uses_only_available_glyphs() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        let mut chars: Vec<char> = SOURCES.iter().flat_map(|s| s.chars()).filter(|c| !c.is_ascii()).collect();
        chars.sort_unstable();
        chars.dedup();
        let missing: Vec<char> = ctx
            .fonts_mut(|f| chars.into_iter().filter(|c| !f.has_glyph(&egui::FontId::proportional(12.0), *c)).collect());
        // Layout may have uploaded new glyphs; discard those texture updates too.
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
    }
}
