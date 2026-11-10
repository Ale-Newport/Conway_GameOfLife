//! Right "Properties" tab: apply status of the draft, selection inspector,
//! live populations, the relationship graph, problems and recent events.

use bevy::prelude::*;
use bevy_egui::egui::{self, RichText};
use interface::Palette;
use interface::theme::{color32_from_rgba, mix};
use interface::widgets::badge;
use rule_engine::CustomLifeView;
use rule_engine::def::EntityType;
use simulation_core::EventSeverity;

use crate::editor::widgets::diagnostic_row;
use crate::editor::{EditorParams, EditorTab, apply_bar, focus_item};
use crate::glyphs::shown_icon;
use crate::graph::{draw_graph, graph_legend};
use crate::session::EntityDetail;

fn section(ui: &mut egui::Ui, title: &str, palette: &Palette) {
    ui.add_space(10.0);
    ui.label(RichText::new(title.to_uppercase()).size(11.5).strong().color(palette.text_muted));
    ui.add_space(3.0);
}

/// The right panel system.
pub fn properties_panel(InMut(ui): InMut<egui::Ui>, mut p: EditorParams) {
    let palette = p.palette();
    section(ui, "Draft and running system", &palette);
    apply_bar(ui, &mut p, &palette, true);
    let Some(view) = p.data.view.clone() else {
        ui.label(RichText::new("Waiting for the first view…").color(palette.text_muted));
        return;
    };
    if let Some(id) = p.ui.selected {
        // Only the selected entity's type is copied (not the whole system).
        let ty = view
            .entities
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| p.data.info.as_ref()?.running.entity_types.get(usize::from(e.type_index)).cloned());
        section(ui, "Selection", &palette);
        inspector(ui, &mut p, &view, ty.as_ref(), &palette);
    }
    section(ui, "Populations", &palette);
    populations(ui, &mut p, &view, &palette);
    section(ui, "Relationship graph", &palette);
    {
        let editor = &mut *p.editor;
        let graph = editor.graph.clone();
        let action = draw_graph(ui, &graph, &mut editor.small_layout, &palette, 240.0, editor.sel_rel, None);
        if let Some(r) = action.relationship {
            editor.sel_rel = Some(r);
            editor.open_at(EditorTab::Relationships);
        }
        if let Some(n) = action.node
            && n < editor.draft.entity_types.len()
        {
            editor.sel_type = n;
            editor.open_at(EditorTab::Entities);
        }
        graph_legend(ui, &graph);
        ui.label(
            RichText::new("Graph of the draft; click an edge or node to edit it.").small().color(palette.text_muted),
        );
    }
    section(ui, "Problems", &palette);
    problems(ui, &mut p, &view, &palette);
    section(ui, "Recent events", &palette);
    if view.recent_events.is_empty() {
        ui.label(RichText::new("No notable events yet.").small().color(palette.text_muted));
    }
    for ev in view.recent_events.iter().take(12) {
        let color = match ev.severity {
            EventSeverity::Major => palette.warning,
            _ => palette.text,
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{:>6}", ev.tick)).monospace().small().color(palette.text_muted));
            ui.label(RichText::new(&ev.message).small().color(color));
        });
    }
}

fn inspector(
    ui: &mut egui::Ui,
    p: &mut EditorParams,
    view: &CustomLifeView,
    ty: Option<&EntityType>,
    palette: &Palette,
) {
    let Some(id) = p.ui.selected else { return };
    let Some(e) = view.entities.iter().find(|e| e.id == id) else {
        ui.label(RichText::new(format!("Entity #{id} no longer exists.")).color(palette.text_muted));
        if ui.small_button("Clear selection").clicked() {
            p.ui.clear_selection();
        }
        return;
    };
    let entry = view.legend.get(usize::from(e.type_index));
    egui::Frame::group(ui.style()).fill(mix(palette.panel, palette.surface, 0.6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            interface::widgets::swatch(ui, color32_from_rgba(e.color), 16.0);
            let name =
                entry.map(|l| format!("{} {}", shown_icon(ui.ctx(), &l.icon, &l.name), l.name)).unwrap_or_default();
            ui.label(RichText::new(name).strong().size(15.0));
            ui.label(RichText::new(format!("#{id}")).monospace().color(palette.text_muted));
        });
        if let Some(t) = ty
            && !t.description.is_empty()
        {
            ui.label(RichText::new(&t.description).small().color(palette.text_muted));
        }
        let detail: Option<&EntityDetail> = p.data.detail.as_ref().filter(|d| d.id == id);
        let entity = detail.and_then(|d| d.entity.as_ref());
        egui::Grid::new("cl_inspector").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            let state_name = entry.and_then(|l| l.states.get(usize::from(e.state))).cloned().unwrap_or_default();
            ui.label("State");
            ui.label(state_name);
            ui.end_row();
            ui.label("Energy");
            let max = ty.map_or(0.0, |t| t.energy.max);
            ui.add(egui::ProgressBar::new(e.energy).desired_width(150.0).text(match entity {
                Some(x) => format!("{:.2} / {}", x.energy, max),
                None => format!("{:.0} %", e.energy * 100.0),
            }));
            ui.end_row();
            ui.label("Position");
            ui.label(format!("({:.2}, {:.2})", e.position.x, e.position.y));
            ui.end_row();
            ui.label("Heading");
            ui.label(format!("{:.0}°", e.heading.to_degrees()));
            ui.end_row();
            ui.label("Size");
            ui.label(format!("{:.2}", e.size));
            ui.end_row();
            if let Some(x) = entity {
                let tick = detail.map_or(0, |d| d.tick);
                ui.label("Age");
                ui.label(format!("{} ticks (in state {})", x.age, tick.saturating_sub(x.state_since)));
                ui.end_row();
                ui.label("Speed");
                ui.label(format!("{:.2} (velocity {:.2})", x.speed, x.kin.velocity.length()));
                ui.end_row();
                ui.label("Generation");
                ui.label(match x.parent {
                    Some(parent) => format!("{} (parent #{})", x.generation, parent.get()),
                    None => x.generation.to_string(),
                });
                ui.end_row();
                if tick < x.next_reproduction {
                    ui.label("Can reproduce");
                    ui.label(format!("in {} ticks", x.next_reproduction - tick));
                    ui.end_row();
                }
                if let Some(t) = ty {
                    for (def, v) in t.properties.iter().zip(&x.props) {
                        ui.label(&def.name);
                        ui.label(format!("{v:.3}  [{} … {}]", def.min, def.max));
                        ui.end_row();
                    }
                    for (c, v) in t.carries.iter().zip(&x.carried) {
                        ui.label(format!("carries {}", c.resource));
                        ui.label(format!("{v:.3} / {}", c.capacity));
                        ui.end_row();
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.toggle_value(&mut p.ui.follow, "Follow (L)");
            if let Some(runner) = p.runner.as_deref()
                && ui.button("Remove").on_hover_text("Remove this entity (undoable)").clicked()
            {
                runner.edit(rule_engine::CustomLifeCommand::Remove { id });
                p.ui.clear_selection();
            }
            if ui.button("Edit type").clicked() {
                p.editor.sel_type = usize::from(e.type_index);
                p.editor.open_at(EditorTab::Entities);
            }
        });
    });
}

fn populations(ui: &mut egui::Ui, p: &mut EditorParams, view: &CustomLifeView, palette: &Palette) {
    let total: u32 = view.legend.iter().map(|l| l.count).sum::<u32>().max(1);
    for (i, l) in view.legend.iter().enumerate() {
        let response = ui
            .horizontal(|ui| {
                interface::widgets::swatch(ui, color32_from_rgba(l.color), 12.0);
                let label = format!("{} {}", shown_icon(ui.ctx(), &l.icon, &l.name), l.name);
                let r = ui.add(egui::Label::new(RichText::new(label)).sense(egui::Sense::click()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(l.count.to_string()).monospace());
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(70.0, 6.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 3.0, palette.surface_hi);
                    let mut fill = rect;
                    fill.set_width(rect.width() * l.count as f32 / total as f32);
                    ui.painter().rect_filled(fill, 3.0, color32_from_rgba(l.color));
                });
                r
            })
            .inner;
        if response.on_hover_text("Edit this type").clicked() {
            p.editor.sel_type = i;
            p.editor.open_at(EditorTab::Entities);
        }
    }
    if view.legend.is_empty() {
        ui.label(RichText::new("The running system has no entity types.").small().color(palette.text_muted));
    }
}

fn problems(ui: &mut egui::Ui, p: &mut EditorParams, view: &CustomLifeView, palette: &Palette) {
    let draft_errors: Vec<_> = p.editor.diagnostics.iter().filter(|d| d.is_error()).cloned().collect();
    if draft_errors.is_empty() && view.diagnostics.is_empty() && view.script_errors.is_empty() {
        ui.label(RichText::new("✔ No problems.").color(palette.success));
        return;
    }
    if !draft_errors.is_empty() {
        ui.horizontal(|ui| {
            badge(ui, &format!("draft: {} error(s)", draft_errors.len()), palette.error);
        });
        for d in draft_errors.iter().take(4) {
            ui.horizontal(|ui| {
                if ui.small_button("Show").clicked() {
                    focus_item(&mut p.editor, d.item);
                }
                ui.vertical(|ui| diagnostic_row(ui, d, palette));
            });
        }
    }
    if !view.diagnostics.is_empty() {
        ui.label(RichText::new("Warnings of the running system").small().strong());
        for d in view.diagnostics.iter().take(4) {
            diagnostic_row(ui, d, palette);
        }
    }
    for e in view.script_errors.iter().take(4) {
        ui.label(RichText::new(e.to_string()).small().color(palette.error));
    }
    if !view.script_errors.is_empty() && ui.small_button("Open scripts").clicked() {
        p.editor.open_at(EditorTab::Scripts);
    }
}
