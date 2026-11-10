//! Left "Tools" panel: the system being designed (editor launchers),
//! world tools with their options, display options and the mode's
//! keyboard shortcuts.

use bevy::prelude::*;
use bevy_egui::egui::{self, RichText};
use interface::theme::color32_from_rgba;
use interface::widgets::badge;
use interface::{ModeRegistry, Palette, ThemeSettings};
use rule_engine::PaintMode;

use crate::MODE;
use crate::colors::ColorBy;
use crate::editor::{ClEditor, EditorTab};
use crate::session::ClData;
use crate::state::{ClUi, FIELD_COLORMAPS, Tool};
use crate::world::SHORTCUTS;

fn section(ui: &mut egui::Ui, title: &str, palette: &Palette) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).strong().size(13.0).color(palette.text));
    ui.add_space(2.0);
}

/// The left panel system.
pub fn tools_panel(
    InMut(ui): InMut<egui::Ui>,
    mut state: ResMut<ClUi>,
    mut editor: ResMut<ClEditor>,
    data: Res<ClData>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
) {
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    design_section(ui, &mut editor, &data, &palette);
    ui.separator();
    tools_section(ui, &mut state, &data, &palette);
    ui.separator();
    display_section(ui, &mut state, &data, &palette);
    ui.separator();
    egui::CollapsingHeader::new(RichText::new("Shortcuts").strong()).id_salt("cl_shortcuts").show(ui, |ui| {
        egui::Grid::new("cl_shortcut_grid").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            for (keys, what) in SHORTCUTS {
                ui.label(RichText::new(*keys).monospace().small());
                ui.label(RichText::new(*what).small());
                ui.end_row();
            }
        });
        ui.label(
            RichText::new("Space, N, R, Ctrl/Cmd+Z, = / -, Tab, Home and F-keys belong to the application (Help, F1).")
                .small()
                .color(palette.text_muted),
        );
    });
}

fn design_section(ui: &mut egui::Ui, editor: &mut ClEditor, data: &ClData, palette: &Palette) {
    section(ui, "Design", palette);
    ui.label(RichText::new(&editor.draft.name).size(15.0).strong());
    ui.label(RichText::new(&editor.source).small().color(palette.text_muted));
    ui.horizontal_wrapped(|ui| {
        let (errors, warnings) = crate::editor::widgets::counts(&editor.diagnostics);
        if editor.is_edited(data.info.as_ref()) {
            badge(ui, "unapplied edits", palette.accent);
        } else {
            badge(ui, "draft = running", palette.success);
        }
        if errors > 0 {
            badge(ui, &format!("{errors} error(s)"), palette.error);
        } else if warnings > 0 {
            badge(ui, &format!("{warnings} warning(s)"), palette.warning);
        }
    });
    let counts = (
        editor.draft.entity_types.len(),
        editor.draft.rules.len(),
        editor.draft.relationships.len(),
        editor.draft.world.resources.len() + editor.draft.world.environment.len(),
        editor.draft.scripts.len(),
    );
    ui.add_space(4.0);
    let open = ui
        .add_sized(
            [ui.available_width(), 30.0],
            egui::Button::new(RichText::new("⚙  Open system editor").strong().color(palette.on_accent))
                .fill(palette.accent),
        )
        .on_hover_text("Entity types, rules, relationships, world, scripts and files (E)");
    if open.clicked() {
        editor.open = true;
    }
    ui.add_space(4.0);
    egui::Grid::new("cl_editor_tabs").num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
        let items = [
            (EditorTab::Entities, format!("Entities ({})", counts.0)),
            (EditorTab::Rules, format!("Rules ({})", counts.1)),
            (EditorTab::Relationships, format!("Relations ({})", counts.2)),
            (EditorTab::World, format!("World & fields ({})", counts.3)),
            (EditorTab::Scripts, format!("Scripts ({})", counts.4)),
            (EditorTab::Files, "Files & presets".to_string()),
        ];
        for (k, (tab, label)) in items.into_iter().enumerate() {
            let w = (ui.available_width() - 6.0).max(80.0);
            if ui.add_sized([w.min(124.0), 24.0], egui::Button::new(RichText::new(label).size(12.5))).clicked() {
                editor.open_at(tab);
            }
            if k % 2 == 1 {
                ui.end_row();
            }
        }
    });
}

fn tools_section(ui: &mut egui::Ui, state: &mut ClUi, data: &ClData, palette: &Palette) {
    section(ui, "World tools", palette);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for tool in Tool::ALL {
            let on = state.tool == tool;
            let text = RichText::new(format!("{} {}", tool.key_label(), tool.label())).size(12.5);
            let text = if on { text.color(palette.on_accent) } else { text };
            let button = egui::Button::new(text).fill(if on { palette.accent } else { palette.surface });
            if ui.add(button).on_hover_text(tool.help()).clicked() {
                state.tool = tool;
            }
        }
    });
    ui.label(RichText::new(state.tool.help()).small().color(palette.text_muted));
    let info = data.info.as_ref();
    match state.tool {
        Tool::Select => {
            ui.label(
                RichText::new("Selected entities are shown in Properties; L follows them.")
                    .small()
                    .color(palette.text_muted),
            );
        }
        Tool::Remove => {}
        Tool::Spawn => {
            let types: Vec<(String, [f32; 4], String)> = info
                .map(|i| {
                    i.running
                        .entity_types
                        .iter()
                        .map(|t| (t.name.clone(), t.color, crate::glyphs::shown_icon(ui.ctx(), &t.icon, &t.name)))
                        .collect()
                })
                .unwrap_or_default();
            egui::Grid::new("cl_spawn_opts").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Type");
                egui::ComboBox::from_id_salt("cl_spawn_type").selected_text(&state.spawn_type).show_ui(ui, |ui| {
                    for (name, color, icon) in &types {
                        let text = RichText::new(format!("{icon} {name}")).color(color32_from_rgba(*color));
                        ui.selectable_value(&mut state.spawn_type, name.clone(), text);
                    }
                });
                ui.end_row();
                ui.label("Per click");
                ui.add(crate::editor::widgets::dv(&mut state.spawn_count).range(1..=200));
                ui.end_row();
                if state.spawn_count > 1 {
                    ui.label("Scatter radius");
                    ui.add(crate::editor::widgets::dv(&mut state.spawn_radius).range(0.5..=64.0).speed(0.1));
                    ui.end_row();
                }
            });
            ui.label(
                RichText::new("Types of the running system; new types need Apply & reset first.")
                    .small()
                    .color(palette.text_muted),
            );
        }
        Tool::Paint => {
            let resources: Vec<String> =
                info.map(|i| i.running.world.resources.iter().map(|r| r.name.clone()).collect()).unwrap_or_default();
            if resources.is_empty() {
                ui.label(
                    RichText::new("The running system has no resource fields to paint; add one in the World tab.")
                        .small()
                        .color(palette.warning),
                );
                return;
            }
            let slider_width = ui.spacing().slider_width;
            ui.spacing_mut().slider_width = (ui.available_width() - 190.0).clamp(80.0, 160.0);
            egui::Grid::new("cl_paint_opts").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Resource");
                egui::ComboBox::from_id_salt("cl_paint_res").selected_text(&state.paint_resource).show_ui(ui, |ui| {
                    for r in &resources {
                        ui.selectable_value(&mut state.paint_resource, r.clone(), r);
                    }
                });
                ui.end_row();
                ui.label("Mode");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut state.paint_mode, PaintMode::Add, "Add");
                    ui.radio_value(&mut state.paint_mode, PaintMode::Set, "Set");
                });
                ui.end_row();
                ui.label(if state.paint_mode == PaintMode::Add { "Amount (− erases)" } else { "Value" });
                ui.add(crate::editor::widgets::dv(&mut state.paint_amount).range(-1.0e4..=1.0e4).speed(0.01));
                ui.end_row();
                ui.label("Brush radius");
                ui.add(egui::Slider::new(&mut state.paint_radius, 0.5..=24.0).max_decimals(1));
                ui.end_row();
            });
            ui.spacing_mut().slider_width = slider_width;
            if state.overlay.as_deref() != Some(state.paint_resource.as_str())
                && !state.paint_resource.is_empty()
                && ui.small_button("Show this field").clicked()
            {
                state.overlay = Some(state.paint_resource.clone());
            }
        }
    }
}

fn display_section(ui: &mut egui::Ui, state: &mut ClUi, data: &ClData, palette: &Palette) {
    section(ui, "Display", palette);
    egui::Grid::new("cl_display").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        ui.label("Colour by");
        ui.horizontal(|ui| {
            for c in ColorBy::ALL {
                ui.selectable_value(&mut state.color_by, c, c.label());
            }
        });
        ui.end_row();
        ui.label("Field overlay");
        let fields: Vec<(String, bool)> = data
            .view
            .as_ref()
            .map(|v| v.fields.iter().map(|f| (f.name.clone(), f.resource)).collect())
            .unwrap_or_default();
        egui::ComboBox::from_id_salt("cl_overlay")
            .selected_text(state.overlay.clone().unwrap_or_else(|| "none (floor)".into()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.overlay, None, "none (floor)");
                for (name, resource) in &fields {
                    let label = format!("{name} · {}", if *resource { "resource" } else { "environment" });
                    ui.selectable_value(&mut state.overlay, Some(name.clone()), label);
                }
            });
        ui.end_row();
        ui.label("Colormap");
        egui::ComboBox::from_id_salt("cl_colormap").selected_text(state.colormap.label()).show_ui(ui, |ui| {
            for c in FIELD_COLORMAPS {
                ui.selectable_value(&mut state.colormap, c, c.label());
            }
        });
        ui.end_row();
        ui.label("Intensity");
        ui.add(egui::Slider::new(&mut state.intensity, 0.3..=1.0).max_decimals(2))
            .on_hover_text("Share of the colormap used for the field's range (lower = darker, entities stand out).");
        ui.end_row();
    });
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut state.glyphs, "Glyphs");
        ui.checkbox(&mut state.headings, "Headings");
        ui.checkbox(&mut state.grid, "Cell grid");
    });
    ui.horizontal(|ui| {
        if ui.button("Fit world (F)").clicked() {
            state.fit_requested = true;
        }
        let can_follow = state.selected.is_some();
        ui.add_enabled_ui(can_follow, |ui| {
            ui.toggle_value(&mut state.follow, "Follow (L)").on_disabled_hover_text("Select an entity first");
        });
    });
}
