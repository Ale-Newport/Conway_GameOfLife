//! Small widgets shared by the editors: code editor with highlighting,
//! committed name fields, diagnostics lists, combos and numeric fields.

use std::ops::Range;
use std::sync::Arc;

use bevy_egui::egui::{self, Color32, FontId, RichText, Ui};
use interface::Palette;
use interface::widgets::badge;
use rule_engine::{Diagnostic, Severity};

use crate::highlight::{CodeColors, dsl_tokens, layout_job, rhai_tokens, span_range};

/// Language of a code editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    /// The rule DSL.
    Dsl,
    /// Rhai.
    Rhai,
}

/// Code colours of the current theme.
pub fn code_colors(palette: &Palette) -> CodeColors {
    CodeColors::new(palette.dark, palette.text, palette.text_muted, palette.accent, palette.error)
}

/// Byte ranges of the diagnostics' `line:col` spans inside `text`.
pub fn error_ranges(text: &str, diags: &[Diagnostic]) -> Vec<Range<usize>> {
    diags
        .iter()
        .filter(|d| d.is_error())
        .filter_map(|d| d.span)
        .filter_map(|s| span_range(text, s.line, s.col, s.len))
        .collect()
}

/// Multi-line code editor with syntax highlighting; `errors` are byte
/// ranges underlined in the error colour. Returns the response.
#[allow(clippy::too_many_arguments)]
pub fn code_editor(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    text: &mut String,
    lang: Lang,
    is_name: &dyn Fn(&str) -> bool,
    errors: &[Range<usize>],
    palette: &Palette,
    rows: usize,
) -> egui::Response {
    let colors = code_colors(palette);
    let font = FontId::monospace(13.0);
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| -> Arc<egui::Galley> {
        let s = buf.as_str();
        let tokens = match lang {
            Lang::Dsl => dsl_tokens(s, is_name),
            Lang::Rhai => rhai_tokens(s),
        };
        let mut job = layout_job(s, &tokens, &colors, font.clone(), errors);
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    egui::Frame::new()
        .fill(interface::theme::mix(palette.panel, palette.background, 0.6))
        .stroke(egui::Stroke::new(1.0, palette.stroke))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .id_salt(id)
                    .code_editor()
                    .lock_focus(true)
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE)
                    .layouter(&mut layouter),
            )
        })
        .inner
}

/// Read-only highlighted code.
pub fn code_view(ui: &mut Ui, text: &str, lang: Lang, is_name: &dyn Fn(&str) -> bool, palette: &Palette) {
    let colors = code_colors(palette);
    let tokens = match lang {
        Lang::Dsl => dsl_tokens(text, is_name),
        Lang::Rhai => rhai_tokens(text),
    };
    let mut job = layout_job(text, &tokens, &colors, FontId::monospace(12.5), &[]);
    job.wrap.max_width = ui.available_width() - 12.0;
    egui::Frame::new()
        .fill(interface::theme::mix(palette.panel, palette.background, 0.6))
        .stroke(egui::Stroke::new(1.0, palette.stroke))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(job);
        });
}

/// One diagnostic line: severity badge, code, `line:col`, message, help.
pub fn diagnostic_row(ui: &mut Ui, d: &Diagnostic, palette: &Palette) {
    let color = match d.severity {
        Severity::Error => palette.error,
        Severity::Warning => palette.warning,
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        badge(ui, &d.code, color);
        if let Some(span) = d.span {
            ui.label(RichText::new(format!("{}:{}", span.line, span.col)).monospace().color(palette.text_muted));
        }
        ui.label(&d.message);
        if let Some(context) = &d.context {
            ui.label(RichText::new(format!("— {context}")).small().color(palette.text_muted));
        }
    });
    if let Some(help) = &d.help {
        ui.label(RichText::new(format!("    help: {help}")).small().color(palette.text_muted));
    }
}

/// A list of diagnostics, or a quiet "no problems" line.
pub fn diagnostics_list(ui: &mut Ui, diags: &[Diagnostic], palette: &Palette, empty: &str) {
    if diags.is_empty() {
        ui.label(RichText::new(format!("✔ {empty}")).color(palette.success));
        return;
    }
    for d in diags {
        diagnostic_row(ui, d, palette);
    }
}

/// Errors and warnings in `diags`.
pub fn counts(diags: &[Diagnostic]) -> (usize, usize) {
    let errors = diags.iter().filter(|d| d.is_error()).count();
    (errors, diags.len() - errors)
}

/// Why a new name cannot be used, if it cannot.
pub fn name_problem(new: &str, current: &str, taken: &[&str], identifier: bool) -> Option<String> {
    let new = new.trim();
    if new.is_empty() {
        return Some("names must not be empty".into());
    }
    if identifier && !crate::blocks::is_bare(new) {
        return Some("use letters, digits and `_` (not starting with a digit, not a keyword)".into());
    }
    if new != current && taken.contains(&new) {
        return Some(format!("`{new}` is already used"));
    }
    None
}

/// A name field that commits on Enter or when it loses focus. Returns the
/// new name when it was changed and is valid; an invalid name is shown in
/// the error colour with the reason and not committed.
pub fn name_field(ui: &mut Ui, id: egui::Id, current: &str, taken: &[&str], identifier: bool) -> Option<String> {
    let mut buffer: String = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.to_string());
    let problem = name_problem(&buffer, current, taken, identifier);
    let mut edit = egui::TextEdit::singleline(&mut buffer).id(id).desired_width(180.0);
    if problem.is_some() {
        edit = edit.text_color(ui.visuals().error_fg_color);
    }
    let response = ui.add(edit);
    let response = match &problem {
        Some(p) => response.on_hover_text(p),
        None => response.on_hover_text("Press Enter to rename; every reference follows."),
    };
    let commit = response.lost_focus();
    if response.has_focus() || response.changed() {
        ui.data_mut(|d| d.insert_temp(id, buffer.clone()));
    }
    if commit {
        ui.data_mut(|d| d.remove::<String>(id));
        let new = buffer.trim().to_string();
        if problem.is_none() && new != current {
            return Some(new);
        }
    } else if !response.has_focus() {
        // Keep showing the stored name when the field is idle.
        ui.data_mut(|d| d.remove::<String>(id));
    }
    None
}

/// Combo box choosing one of `options` (the current value is kept even
/// when it is not among them, so invalid references stay visible).
pub fn name_combo(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    options: &[String],
    width: f32,
) -> bool {
    let mut changed = false;
    let shown = if value.is_empty() { "—".to_string() } else { value.clone() };
    egui::ComboBox::from_id_salt(id).width(width).selected_text(shown).show_ui(ui, |ui| {
        for option in options {
            if ui.selectable_label(value == option, option).clicked() && value != option {
                *value = option.clone();
                changed = true;
            }
        }
    });
    changed
}

/// Optional name: "none" or one of `options`.
pub fn optional_name_combo(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut Option<String>,
    none_label: &str,
    options: &[String],
    width: f32,
) -> bool {
    let mut changed = false;
    let shown = value.clone().unwrap_or_else(|| none_label.to_string());
    egui::ComboBox::from_id_salt(id).width(width).selected_text(shown).show_ui(ui, |ui| {
        if ui.selectable_label(value.is_none(), none_label).clicked() && value.is_some() {
            *value = None;
            changed = true;
        }
        for option in options {
            if ui.selectable_label(value.as_deref() == Some(option), option).clicked()
                && value.as_deref() != Some(option)
            {
                *value = Some(option.clone());
                changed = true;
            }
        }
    });
    changed
}

/// A drag value that clamps what the user enters but never rewrites a
/// stored value just because it is displayed (egui clamps existing values
/// by default, which would silently edit the draft).
pub fn dv<Num: egui::emath::Numeric>(value: &mut Num) -> egui::DragValue<'_> {
    egui::DragValue::new(value).clamp_existing_to_range(false)
}

/// Colour button for definition colours (unmultiplied **sRGB** RGBA in
/// `[0, 1]`, the convention of the renderer and the legends). egui's
/// `color_edit_button_rgba_*` editors work in *linear* space, which showed
/// every definition colour washed out and stored edits in the wrong space,
/// so the 8-bit sRGB editor is used. The value is only written when the
/// user changes it (the round trip through 8 bits is not exact).
pub fn color_edit(ui: &mut Ui, rgba: &mut [f32; 4]) -> bool {
    let mut bytes = interface::theme::srgba_bytes(*rgba);
    let changed = ui.color_edit_button_srgba_unmultiplied(&mut bytes).changed();
    if changed {
        *rgba = interface::theme::rgba_from_srgba_bytes(bytes);
    }
    changed
}

/// A labelled `f64` drag value clamped to `range`.
pub fn num_f64(ui: &mut Ui, label: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>, speed: f64) -> bool {
    ui.label(label);
    let r = ui.add(dv(value).range(range).speed(speed).max_decimals(4));
    ui.end_row();
    r.changed()
}

/// A labelled `f32` drag value clamped to `range`.
pub fn num_f32(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, speed: f64) -> bool {
    ui.label(label);
    let r = ui.add(dv(value).range(range).speed(speed).max_decimals(4));
    ui.end_row();
    r.changed()
}

/// A labelled integer drag value clamped to `range`.
pub fn num_u64(ui: &mut Ui, label: &str, value: &mut u64, range: std::ops::RangeInclusive<u64>) -> bool {
    ui.label(label);
    let r = ui.add(dv(value).range(range).speed(0.2));
    ui.end_row();
    r.changed()
}

/// A labelled `u32` drag value clamped to `range`.
pub fn num_u32(ui: &mut Ui, label: &str, value: &mut u32, range: std::ops::RangeInclusive<u32>) -> bool {
    ui.label(label);
    let r = ui.add(dv(value).range(range).speed(0.2));
    ui.end_row();
    r.changed()
}

/// Section heading inside an editor.
pub fn heading(ui: &mut Ui, text: &str, palette: &Palette) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).strong().size(13.5).color(palette.text));
    ui.add_space(2.0);
}

/// Muted explanatory text.
pub fn hint(ui: &mut Ui, text: &str, palette: &Palette) {
    ui.label(RichText::new(text).small().color(palette.text_muted));
}

/// A selectable row of a list with a colour swatch; returns whether it was clicked.
pub fn list_row(ui: &mut Ui, selected: bool, swatch: Option<Color32>, text: &str, sub: &str) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(c) = swatch {
            interface::widgets::swatch(ui, c, 12.0);
        }
        let label = if sub.is_empty() { text.to_string() } else { format!("{text}  · {sub}") };
        ui.selectable_label(selected, label)
    })
    .inner
    .clicked()
}

/// A fixed-width list column and a detail column, each scrolling on its own.
pub fn split<T>(
    ui: &mut Ui,
    id: &str,
    list_width: f32,
    state: &mut T,
    list: impl FnOnce(&mut Ui, &mut T),
    detail: impl FnOnce(&mut Ui, &mut T),
) {
    let height = ui.available_height().max(160.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(list_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_size(egui::vec2(list_width, height));
            egui::ScrollArea::vertical()
                .id_salt((id, "list"))
                .auto_shrink([false, false])
                .max_height(height)
                .show(ui, |ui| list(ui, state));
        });
        ui.separator();
        let width = ui.available_width();
        ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_size(egui::vec2(width, height));
            egui::ScrollArea::vertical().id_salt((id, "detail")).auto_shrink([false, false]).max_height(height).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width() - 8.0);
                    detail(ui, state);
                },
            );
        });
    });
}

/// A two-step delete button: the first click arms it, the second deletes.
pub fn confirm_delete(ui: &mut Ui, id: egui::Id, what: &str, palette: &Palette) -> bool {
    let armed: bool = ui.data(|d| d.get_temp(id)).unwrap_or(false);
    if armed {
        let mut confirmed = false;
        ui.horizontal(|ui| {
            if ui.button(RichText::new(format!("Delete {what}")).color(palette.error)).clicked() {
                confirmed = true;
                ui.data_mut(|d| d.remove::<bool>(id));
            }
            if ui.button("Cancel").clicked() {
                ui.data_mut(|d| d.remove::<bool>(id));
            }
        });
        confirmed
    } else {
        if ui.button("Delete…").on_hover_text(format!("Delete {what}")).clicked() {
            ui.data_mut(|d| d.insert_temp(id, true));
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::Span;

    #[test]
    fn name_rules() {
        assert!(name_problem("", "a", &[], true).is_some());
        assert!(name_problem("2x", "a", &[], true).is_some());
        assert!(name_problem("when", "a", &[], true).is_some(), "keywords are not plain names");
        assert!(name_problem("lumen moss", "a", &[], false).is_none(), "free names may contain spaces");
        assert!(name_problem("plant", "a", &["plant"], true).is_some());
        assert!(name_problem("plant", "plant", &["plant"], true).is_none(), "keeping the own name is fine");
        assert!(name_problem(" tree ", "a", &["plant"], true).is_none());
    }

    #[test]
    fn diagnostics_map_to_text_ranges() {
        let text = "rule a for x:\n  when energy > then die()";
        let d = Diagnostic::error("E015", "expected an expression").at(Span::new(2, 17, 4));
        let w = Diagnostic::warning("W102", "duplicate").at(Span::new(1, 1, 4));
        assert_eq!(error_ranges(text, &[d.clone(), w.clone()]), vec![30..34]);
        assert_eq!(counts(&[d, w]), (1, 1));
    }
}
