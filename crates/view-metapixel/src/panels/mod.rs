//! Panel contents: the Tools panel (left), the Properties tab (inspector,
//! validation, construction) and the overlay over the world, plus the small
//! widgets they share.

pub(crate) mod overlay;
pub(crate) mod properties;
pub(crate) mod tools_panel;

use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, Ui, vec2};
use interface::Palette;
use interface::theme::with_alpha;
use metapixel::spec::PERIOD;

use crate::macro_layer::colors;
use crate::timing::{cycle_fraction, transition_window_fractions};

/// Which sections of the Tools panel are open (kept across visits).
#[derive(bevy::prelude::Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelSections {
    /// Hashlife time control.
    pub time: bool,
    /// Macro rule programming.
    pub rule: bool,
    /// Grid size, seed and soup.
    pub grid: bool,
    /// View options.
    pub view: bool,
    /// Shortcut list.
    pub shortcuts: bool,
}

impl Default for PanelSections {
    fn default() -> Self {
        Self { time: true, rule: true, grid: true, view: true, shortcuts: false }
    }
}

/// A section whose header opens and closes it.
pub(crate) fn collapsible(ui: &mut Ui, palette: &Palette, title: &str, open: &mut bool, add: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    let response =
        egui::CollapsingHeader::new(RichText::new(title.to_uppercase()).size(11.0).strong().color(palette.text_muted))
            .id_salt(("mp_section", title))
            .open(Some(*open))
            .show(ui, |ui| add(ui));
    if response.header_response.clicked() {
        *open = !*open;
    }
}

pub(crate) fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(palette.text_muted));
    ui.add_space(2.0);
}

pub(crate) fn card<R>(ui: &mut Ui, palette: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
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

/// A card with a coloured left edge for notices (warnings, errors, notes).
pub(crate) fn notice(ui: &mut Ui, palette: &Palette, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(with_alpha(color, 0.10))
        .stroke(Stroke::new(1.0, with_alpha(color, 0.45)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(12.0).color(palette.text));
        });
}

/// Equal-width toggle buttons in rows of `columns`; returns the clicked index.
pub(crate) fn segmented(ui: &mut Ui, items: &[(&str, &str, bool)], columns: usize) -> Option<usize> {
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
                        egui::Button::new(RichText::new(*label).size(13.0)).selected(*on).min_size(vec2(width, 26.0));
                    if ui.add(button).on_hover_text(*tip).clicked() {
                        clicked = Some(row * columns + j);
                    }
                }
            });
        }
    });
    clicked
}

/// Two-column key / value grid rows; values wrap so the panel never widens.
pub(crate) fn kv_grid(ui: &mut Ui, id: &str, palette: &Palette, rows: &[(&str, String)]) {
    let key_width = 96.0;
    let value_width = (ui.available_width() - key_width - 10.0).max(80.0);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(vec2(10.0, 3.0))
        .min_col_width(key_width)
        .max_col_width(value_width)
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.add(egui::Label::new(RichText::new(*k).size(12.0).color(palette.text_muted)).wrap());
                ui.add(egui::Label::new(RichText::new(v).size(12.0).color(palette.text)).wrap());
                ui.end_row();
            }
        });
}

pub(crate) fn rgb32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The cycle as a bar: progress `t mod P`, the transition window in which
/// displays may switch, and the readout at the end.
pub(crate) fn phase_bar(ui: &mut Ui, palette: &Palette, phase: u64, first_cycle: bool, width: f32, height: f32) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, palette.surface_hi);
    let (a, b) = transition_window_fractions();
    if !first_cycle {
        let window = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + rect.width() * a, rect.min.y),
            egui::pos2(rect.min.x + rect.width() * b, rect.max.y),
        );
        painter.rect_filled(window, 0.0, with_alpha(palette.warning, 0.45));
    }
    let f = cycle_fraction(phase);
    let done = egui::Rect::from_min_size(rect.min, vec2(rect.width() * f, rect.height()));
    painter.rect_filled(done, 3.0, with_alpha(palette.accent, 0.85));
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
    response.on_hover_text(format!(
        "Position in the {} -generation cycle. Shaded: transition window (t mod P in [4,320, 7,407], from the second \
         cycle on), where displays may switch. The macro states are read at the end of the cycle.",
        interface::format::format_count(PERIOD)
    ));
}

/// Legend chip with an sRGB swatch.
pub(crate) fn chip(ui: &mut Ui, color: [u8; 3], label: &str) {
    interface::widgets::legend_chip(ui, rgb32(color), label);
}

/// Legend of the macro layer shades.
pub(crate) fn macro_legend(ui: &mut Ui, prediction: bool, live: bool) {
    ui.horizontal(|ui| {
        if prediction {
            chip(ui, colors::PREDICTED_ON, "predicted ON");
            chip(ui, colors::PREDICTED_OFF, "predicted OFF");
        } else {
            chip(ui, colors::ON, "ON");
            chip(ui, colors::OFF, "OFF");
            if live {
                chip(ui, crate::macro_layer::TileShade::Switching(0.5).rgb(), "switching");
            } else {
                chip(ui, colors::AMBIGUOUS, "ambiguous");
            }
        }
        chip(ui, colors::MISMATCH, "mismatch (outline)");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every string the mode draws with egui's built-in fonts must have
    /// glyphs (missing ones render as boxes).
    #[test]
    fn mode_texts_have_glyphs() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        let font = egui::FontId::proportional(13.0);
        let mut texts: Vec<String> = Vec::new();
        let tutorial = crate::tutorial::tutorial();
        for page in &tutorial.pages {
            texts.push(page.title.clone());
            texts.push(page.body.clone());
            if let Some(a) = &page.action {
                texts.push(a.label.clone());
            }
        }
        texts.push(tutorial.concepts.clone());
        for s in crate::tools::MODE_SHORTCUTS {
            texts.push(s.description.to_string());
            texts.push(s.label.to_string());
        }
        for m in crate::macro_layer::MacroSource::ALL {
            texts.push(m.help().to_string());
            texts.push(crate::macro_layer::macro_label(m, false).to_string());
        }
        texts.push(crate::macro_layer::macro_label(crate::macro_layer::MacroSource::Readout, true).to_string());
        for m in crate::macro_layer::LayerMode::ALL {
            texts.push(m.help().to_string());
        }
        for t in crate::tools::Tool::ALL {
            texts.push(t.help().to_string());
        }
        for stage in [metapixel::CycleStage::FirstCycle, metapixel::CycleStage::Transition] {
            texts.push(crate::timing::stage_help(stage));
        }
        texts.push(crate::macro_layer::exact_headline(1));
        texts.push(crate::macro_layer::exact_detail((2, 3)));
        texts.push(metapixel::ACCELERATED_LABEL.to_string());
        texts.push(metapixel::EXACT_LABEL.to_string());
        texts.push(crate::panels::properties::CREDITS.to_string());
        let catalog = crate::session::Catalog::builtin();
        for e in catalog.entries() {
            texts.push(e.title);
            texts.push(e.description);
        }
        // The model's descriptor as the shell shows it.
        let tiles = std::sync::Arc::new(metapixel::TileSet::from_dir(&metapixel::TileSet::source_tree_dir()).unwrap());
        let sim = metapixel::MetapixelSim::new(
            tiles,
            metapixel::WorldConfig::default(),
            metapixel::MacroGrid::from_rows(&["...", "OOO", "..."]).unwrap(),
        )
        .unwrap();
        let d = crate::session::served_descriptor(simulation_core::Simulation::descriptor(&sim));
        texts.push(d.rules.clone());
        texts.push(d.summary.clone());
        texts.extend(d.params.iter().map(|p| p.description.clone()));
        texts.extend(d.metrics.iter().map(|m| m.description.clone()));
        for text in texts {
            let missing: Vec<char> =
                text.chars().filter(|c| !ctx.fonts_mut(|f| f.has_glyph(&font, *c)) && !c.is_whitespace()).collect();
            assert!(missing.is_empty(), "no glyph for {missing:?} in {text:?}");
        }
    }
}
