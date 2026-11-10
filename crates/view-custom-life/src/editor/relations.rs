//! Relationship editor: `subject <kind> object` with the kind's parameters,
//! the rules each relationship generates, and the force-directed
//! relationship graph (typed, coloured edges with a legend).

use bevy_egui::egui::{self, RichText, Ui};
use interface::Palette;
use rule_engine::compile::ir::RuleOrigin;
use rule_engine::def::{RelationKind, Relationship, TransformTrigger};
use rule_engine::dsl::parse_expr;
use rule_engine::{Diagnostic, ItemRef};

use super::widgets::{Lang, code_view, confirm_delete, diagnostics_list, heading, hint, name_combo};
use super::{ClEditor, EditorParams, EditorTab};
use crate::colors::{RELATION_LABELS, relation_color};
use crate::graph::{draw_graph, graph_legend};

/// A relationship kind with default parameters.
pub fn default_kind(label: &str, resources: &[String]) -> RelationKind {
    match label {
        "protects" => RelationKind::Protects { radius: 5.0, damage: 5.0 },
        "chases" => RelationKind::Chases { radius: 5.0, speed_factor: 1.0 },
        "avoids" => RelationKind::Avoids { radius: 5.0, speed_factor: 1.0 },
        "transforms" => RelationKind::Transforms { trigger: TransformTrigger::Always, probability: 1.0 },
        "produces" => RelationKind::Produces { amount: 1.0, probability: 1.0, cost: 0.0 },
        "reproduces_with" => RelationKind::ReproducesWith { radius: 5.0, probability: 1.0, min_energy: 0.0 },
        "competes_with" => RelationKind::CompetesWith { aggression: 1.0, damage: 5.0 },
        "trades_with" => RelationKind::TradesWith {
            give: resources.first().cloned().unwrap_or_default(),
            give_amount: 1.0,
            receive: resources.get(1).or(resources.first()).cloned().unwrap_or_default(),
            receive_amount: 1.0,
            radius: 5.0,
        },
        _ => RelationKind::Consumes { efficiency: 1.0, hunger: None, radius: 5.0, bite: 1.0, handling: 0 },
    }
}

/// Whether the object of a kind may be a resource field.
pub fn object_may_be_resource(kind: &RelationKind) -> bool {
    matches!(kind, RelationKind::Consumes { .. } | RelationKind::Produces { .. })
}

/// Draws the tab.
pub fn show(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    let editor = &mut *p.editor;
    egui::ScrollArea::vertical().id_salt("cl_relations_scroll").auto_shrink([false, false]).show(ui, |ui| {
        heading(ui, "Relationship graph", palette);
        hint(
            ui,
            "Entity types (circles, higher trophic levels drawn higher) and resources (squares, bottom). \
             Click an edge to edit it, a node to select the type; drag nodes to pin them, double-click to release.",
            palette,
        );
        let graph = editor.graph.clone();
        let selected_node = (!editor.draft.entity_types.is_empty()).then_some(editor.sel_type);
        // Tall enough that edges between adjacent trophic levels stay
        // readable; the tab scrolls.
        let height = 340.0;
        let action = draw_graph(ui, &graph, &mut editor.layout, palette, height, editor.sel_rel, selected_node);
        if let Some(r) = action.relationship {
            editor.sel_rel = Some(r);
        }
        if let Some(n) = action.node
            && n < editor.draft.entity_types.len()
        {
            editor.sel_type = n;
        }
        graph_legend(ui, &graph);
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(250.0, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(250.0);
                list(ui, editor, palette);
            });
            ui.separator();
            ui.vertical(|ui| detail(ui, editor, palette));
        });
    });
}

fn list(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) {
    heading(ui, "Relationships", palette);
    let mut select = None;
    for (i, r) in editor.draft.relationships.iter().enumerate() {
        let label = r.kind.label();
        let errors = editor.diagnostics.iter().any(|d| d.is_error() && d.item == Some(ItemRef::Relationship(i as u16)));
        let text = format!("{}{} {} {}", if errors { "⚠ " } else { "" }, r.subject, label.replace('_', " "), r.object);
        let response = ui
            .horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 10.0), egui::Sense::hover());
                ui.painter().line_segment(
                    [rect.left_center(), rect.right_center()],
                    egui::Stroke::new(2.5, relation_color(label)),
                );
                ui.selectable_label(editor.sel_rel == Some(i), text)
            })
            .inner;
        if response.clicked() {
            select = Some(i);
        }
    }
    if editor.draft.relationships.is_empty() {
        hint(ui, "No relationships yet: they are the quickest way to make types interact.", palette);
    }
    if let Some(i) = select {
        editor.sel_rel = Some(i);
    }
    ui.add_space(6.0);
    let types = editor.type_names();
    let can_add = !types.is_empty();
    ui.menu_button("+ Relationship", |ui| {
        for label in RELATION_LABELS {
            let text = RichText::new(label.replace('_', " ")).color(relation_color(label));
            if ui.add_enabled(can_add, egui::Button::new(text)).clicked() {
                let subject = types.get(editor.sel_type).or(types.first()).cloned().unwrap_or_default();
                let kind = default_kind(label, &editor.resource_names());
                let object = if object_may_be_resource(&kind) && label == "consumes" {
                    types.iter().find(|t| **t != subject).cloned().or_else(|| editor.resource_names().first().cloned())
                } else {
                    types.iter().find(|t| **t != subject).cloned()
                }
                .unwrap_or_else(|| subject.clone());
                editor.draft.relationships.push(Relationship { subject, kind, object, priority_shift: 0 });
                editor.sel_rel = Some(editor.draft.relationships.len() - 1);
                editor.touch();
                ui.close();
            }
        }
    });
    if let Some(i) = editor.sel_rel
        && i < editor.draft.relationships.len()
        && ui.button("Duplicate").clicked()
    {
        let copy = editor.draft.relationships[i].clone();
        editor.draft.relationships.insert(i + 1, copy);
        editor.sel_rel = Some(i + 1);
        editor.touch();
    }
}

fn detail(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) {
    let Some(i) = editor.sel_rel.filter(|&i| i < editor.draft.relationships.len()) else {
        hint(ui, "Select a relationship (in the list or the graph) or add one.", palette);
        return;
    };
    let types = editor.type_names();
    let resources = editor.resource_names();
    let mut objects = types.clone();
    let diags: Vec<Diagnostic> = editor.diagnostics_of(ItemRef::Relationship(i as u16));
    let generated: Vec<(String, String)> = editor
        .compiled_rules
        .iter()
        .filter(|r| matches!(&r.origin, RuleOrigin::Relationship { index, .. } if *index == i))
        .map(|r| (r.name.clone(), r.text.clone()))
        .collect();
    let mut changed = false;
    let rel = &mut editor.draft.relationships[i];
    if object_may_be_resource(&rel.kind) {
        objects.extend(resources.iter().cloned());
    }
    heading(ui, "Edge", palette);
    ui.horizontal_wrapped(|ui| {
        changed |= name_combo(ui, ("cl_rel_subject", i), &mut rel.subject, &types, 110.0);
        let label = rel.kind.label();
        egui::ComboBox::from_id_salt(("cl_rel_kind", i))
            .selected_text(RichText::new(label.replace('_', " ")).color(relation_color(label)))
            .width(130.0)
            .show_ui(ui, |ui| {
                for l in RELATION_LABELS {
                    if ui
                        .selectable_label(l == label, RichText::new(l.replace('_', " ")).color(relation_color(l)))
                        .clicked()
                        && l != label
                    {
                        rel.kind = default_kind(l, &resources);
                        changed = true;
                    }
                }
            });
        changed |= name_combo(ui, ("cl_rel_object", i), &mut rel.object, &objects, 110.0);
    });
    heading(ui, "Parameters", palette);
    egui::Grid::new(("cl_rel_params", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        changed |= kind_params(ui, &mut rel.kind, &resources, &types, i);
        ui.label("Priority shift");
        changed |= ui
            .add(crate::editor::widgets::dv(&mut rel.priority_shift).range(-99..=99))
            .on_hover_text(format!("Moves the generated rules within their band (base {}).", rel.kind.priority_band()))
            .changed();
        ui.end_row();
    });
    ui.label(RichText::new(rel.kind.summary()).small().color(palette.text_muted));
    heading(ui, "Generated rules", palette);
    if generated.is_empty() {
        hint(ui, "Shown once the draft compiles (some kinds generate nothing for some objects).", palette);
    }
    let names = |w: &str| editor.is_name(w);
    for (_, text) in &generated {
        code_view(ui, text, Lang::Dsl, &names, palette);
    }
    if !generated.is_empty() && ui.small_button("Open in the rule editor").clicked() {
        let first = editor
            .compiled_rules
            .iter()
            .position(|r| matches!(&r.origin, RuleOrigin::Relationship { index, .. } if *index == i));
        editor.sel_generated = first;
        editor.tab = EditorTab::Rules;
    }
    heading(ui, "Problems", palette);
    diagnostics_list(ui, &diags, palette, "No problems for this relationship.");
    ui.add_space(6.0);
    if confirm_delete(ui, egui::Id::new(("cl_delete_rel", i)), "this relationship and its rules", palette) {
        editor.draft.relationships.remove(i);
        editor.sel_rel = if editor.draft.relationships.is_empty() { None } else { Some(i.saturating_sub(1)) };
        changed = true;
    }
    if changed {
        editor.touch();
    }
}

fn drag(ui: &mut Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, speed: f64, help: &str) -> bool {
    ui.label(label).on_hover_text(help);
    let r = ui.add(crate::editor::widgets::dv(v).range(range).speed(speed).max_decimals(3)).on_hover_text(help);
    ui.end_row();
    r.changed()
}

fn kind_params(ui: &mut Ui, kind: &mut RelationKind, resources: &[String], types: &[String], i: usize) -> bool {
    let mut changed = false;
    match kind {
        RelationKind::Consumes { efficiency, hunger, radius, bite, handling } => {
            changed |= drag(ui, "Efficiency", efficiency, 0.0..=1.0, 0.005, "Fraction of the prey's energy gained.");
            ui.label("Hunger threshold").on_hover_text("Only forage/eat while energy is below this.");
            ui.horizontal(|ui| {
                let mut on = hunger.is_some();
                if ui.checkbox(&mut on, "").changed() {
                    *hunger = on.then_some(20.0);
                    changed = true;
                }
                match hunger {
                    Some(h) => changed |= ui.add(crate::editor::widgets::dv(h).range(0.0..=1.0e6).speed(0.1)).changed(),
                    None => {
                        ui.label(RichText::new("always").weak());
                    }
                }
            });
            ui.end_row();
            changed |= drag(ui, "Search radius", radius, 0.1..=32.0, 0.05, "Sensing radius used to find food.");
            changed |=
                drag(ui, "Bite (resources)", bite, 0.0..=1.0e6, 0.01, "Amount taken per tick from a resource field.");
            ui.label("Handling time").on_hover_text("Minimum ticks between two meals (Holling type II).");
            changed |= ui.add(crate::editor::widgets::dv(handling).range(0..=1_000_000).suffix(" ticks")).changed();
            ui.end_row();
        }
        RelationKind::Protects { radius, damage } => {
            changed |= drag(ui, "Guard radius", radius, 0.1..=32.0, 0.05, "Radius in which enemies are engaged.");
            changed |= drag(ui, "Damage", damage, 0.0..=1.0e6, 0.1, "Energy removed from an attacked enemy.");
        }
        RelationKind::Chases { radius, speed_factor } | RelationKind::Avoids { radius, speed_factor } => {
            changed |= drag(ui, "Sensing radius", radius, 0.1..=32.0, 0.05, "Radius in which the object is noticed.");
            changed |= drag(ui, "Speed factor", speed_factor, 0.0..=10.0, 0.01, "Speed multiplier while moving.");
        }
        RelationKind::Transforms { trigger, probability } => {
            ui.label("Trigger");
            let label = match trigger {
                TransformTrigger::Always => "always",
                TransformTrigger::Condition(_) => "condition",
                TransformTrigger::Contact(_) => "contact",
                TransformTrigger::AfterTicks(_) => "after ticks",
            };
            egui::ComboBox::from_id_salt(("cl_trigger", i)).selected_text(label).show_ui(ui, |ui| {
                let options = [
                    ("always", TransformTrigger::Always),
                    ("condition", TransformTrigger::Condition("energy > 50".into())),
                    ("contact", TransformTrigger::Contact(types.first().cloned().unwrap_or_default())),
                    ("after ticks", TransformTrigger::AfterTicks(20)),
                ];
                for (l, value) in options {
                    if ui.selectable_label(l == label, l).clicked() && l != label {
                        *trigger = value;
                        changed = true;
                    }
                }
            });
            ui.end_row();
            match trigger {
                TransformTrigger::Condition(text) => {
                    ui.label("When");
                    ui.vertical(|ui| {
                        changed |=
                            ui.add(egui::TextEdit::singleline(text).code_editor().desired_width(220.0)).changed();
                        if let Err(d) = parse_expr(text) {
                            let msg = d.first().map_or_else(
                                || "syntax error".to_string(),
                                |d| d.span.map_or(d.message.clone(), |s| format!("{}:{} {}", s.line, s.col, d.message)),
                            );
                            ui.label(RichText::new(msg).small().color(ui.visuals().error_fg_color));
                        }
                    });
                    ui.end_row();
                }
                TransformTrigger::Contact(t) => {
                    ui.label("Touching");
                    changed |= name_combo(ui, ("cl_trigger_contact", i), t, types, 120.0);
                    ui.end_row();
                }
                TransformTrigger::AfterTicks(n) => {
                    ui.label("Ticks in state");
                    changed |= ui.add(crate::editor::widgets::dv(n).range(0..=10_000_000)).changed();
                    ui.end_row();
                }
                TransformTrigger::Always => {}
            }
            changed |= drag(ui, "Probability", probability, 0.0..=1.0, 0.005, "Chance per tick once triggered.");
        }
        RelationKind::Produces { amount, probability, cost } => {
            changed |=
                drag(ui, "Amount", amount, 0.0..=100.0, 0.01, "Resource amount or number of entities per firing.");
            changed |= drag(ui, "Probability", probability, 0.0..=1.0, 0.005, "Chance per tick.");
            changed |= drag(ui, "Energy cost", cost, 0.0..=1.0e6, 0.05, "Energy the subject pays per firing.");
        }
        RelationKind::ReproducesWith { radius, probability, min_energy } => {
            changed |= drag(ui, "Partner radius", radius, 0.1..=32.0, 0.05, "Radius in which a partner is searched.");
            changed |= drag(ui, "Probability", probability, 0.0..=1.0, 0.005, "Chance per tick with a partner.");
            changed |=
                drag(ui, "Minimum energy", min_energy, 0.0..=1.0e6, 0.1, "In addition to the reproduction cost.");
        }
        RelationKind::CompetesWith { aggression, damage } => {
            changed |= drag(ui, "Aggression", aggression, 0.0..=1.0, 0.005, "Probability of attacking on contact.");
            changed |= drag(ui, "Damage", damage, 0.0..=1.0e6, 0.1, "Energy removed per attack.");
        }
        RelationKind::TradesWith { give, give_amount, receive, receive_amount, radius } => {
            ui.label("Gives");
            ui.horizontal(|ui| {
                changed |= ui.add(crate::editor::widgets::dv(give_amount).range(0.0..=1.0e6).speed(0.05)).changed();
                changed |= name_combo(ui, ("cl_trade_give", i), give, resources, 100.0);
            });
            ui.end_row();
            ui.label("Receives");
            ui.horizontal(|ui| {
                changed |= ui.add(crate::editor::widgets::dv(receive_amount).range(0.0..=1.0e6).speed(0.05)).changed();
                changed |= name_combo(ui, ("cl_trade_receive", i), receive, resources, 100.0);
            });
            ui.end_row();
            changed |= drag(ui, "Partner radius", radius, 0.1..=32.0, 0.05, "Radius in which a partner is searched.");
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::compile;
    use rule_engine::def::LifeSystem;
    use rule_engine::def::{EntityType, ResourceField};

    #[test]
    fn every_kind_has_valid_defaults() {
        let mut sys = LifeSystem::new("t", 32, 32);
        let mut a = EntityType::new("a");
        a.movement = rule_engine::def::MovementDef::RandomWalk { speed: 0.5 };
        a.reproduction.mode = rule_engine::def::ReproductionMode::Sexual;
        a.carries = vec![
            rule_engine::def::CarriedResource { resource: "gold".into(), initial: 1.0, capacity: 5.0 },
            rule_engine::def::CarriedResource { resource: "food".into(), initial: 1.0, capacity: 5.0 },
        ];
        let mut b = a.clone();
        b.name = "b".into();
        sys.entity_types = vec![a, b];
        sys.world.resources = vec![ResourceField::new("gold"), ResourceField::new("food")];
        let resources: Vec<String> = vec!["gold".into(), "food".into()];
        for label in RELATION_LABELS {
            let kind = default_kind(label, &resources);
            assert_eq!(kind.label(), label);
            let mut s = sys.clone();
            s.relationships.push(Relationship { subject: "a".into(), kind, object: "b".into(), priority_shift: 0 });
            compile(&s).unwrap_or_else(|e| panic!("{label}: {e}"));
        }
        assert!(object_may_be_resource(&default_kind("consumes", &resources)));
        assert!(!object_may_be_resource(&default_kind("avoids", &resources)));
    }
}
