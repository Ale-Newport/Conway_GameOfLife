//! World editor: name and seed, size, topology and entity cap, resource
//! fields, environment fields, random events and end conditions.

use bevy_egui::egui::{self, RichText, Ui};
use interface::Palette;
use rule_engine::ItemRef;
use rule_engine::compile::{MAX_ENTITIES_CEILING, MAX_WORLD_SIDE, MIN_WORLD_SIDE};
use rule_engine::def::{
    Axis, EndCondition, EnvPattern, EnvironmentField, EventEffect, FieldInit, RandomEvent, ResourceField, Seasonality,
};
use simulation_core::Topology;

use super::widgets::{diagnostics_list, heading, hint, name_combo, name_field, num_f32, num_f64, optional_name_combo};
use super::{ClEditor, EditorParams};
use crate::colors::new_color;
use crate::draft::{rename_environment, rename_resource, unique_name};

/// Topologies a custom-life world supports (fields need a finite size).
pub const TOPOLOGIES: [(Topology, &str, &str); 4] = [
    (Topology::Torus, "Torus", "Edges wrap around (no borders)."),
    (Topology::Bounded, "Bounded", "Reflecting walls."),
    (Topology::KleinBottle, "Klein bottle", "Wraps; crossing the top/bottom edge mirrors x."),
    (Topology::CrossSurface, "Cross-surface", "Wraps with both crossings mirrored (projective plane)."),
];

/// Label of a topology.
pub fn topology_label(t: Topology) -> &'static str {
    TOPOLOGIES.iter().find(|(x, _, _)| *x == t).map_or("Infinite (unsupported)", |(_, l, _)| l)
}

/// Draws the tab.
pub fn show(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    let editor = &mut *p.editor;
    egui::ScrollArea::vertical().id_salt("cl_world_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width() - 8.0);
        let mut changed = false;
        changed |= system_section(ui, editor, palette);
        changed |= resources_section(ui, editor, palette);
        changed |= environment_section(ui, editor, palette);
        changed |= events_section(ui, editor, palette);
        changed |= end_section(ui, editor, palette);
        let diags: Vec<_> = editor
            .diagnostics
            .iter()
            .filter(|d| {
                matches!(
                    d.item,
                    Some(
                        ItemRef::System
                            | ItemRef::World
                            | ItemRef::Resource(_)
                            | ItemRef::Environment(_)
                            | ItemRef::RandomEvent(_)
                            | ItemRef::EndCondition(_)
                    ) | None
                )
            })
            .cloned()
            .collect();
        heading(ui, "Problems", palette);
        diagnostics_list(ui, &diags, palette, "No problems in the world definition.");
        if changed {
            editor.touch();
        }
    });
}

fn system_section(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) -> bool {
    let mut changed = false;
    heading(ui, "System", palette);
    egui::Grid::new("cl_world_system").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        changed |= ui.add(egui::TextEdit::singleline(&mut editor.draft.name).desired_width(260.0)).changed();
        ui.end_row();
        ui.label("Description");
        changed |= ui
            .add(egui::TextEdit::multiline(&mut editor.draft.description).desired_rows(2).desired_width(360.0))
            .changed();
        ui.end_row();
        ui.label("Seed");
        changed |= ui
            .add(crate::editor::widgets::dv(&mut editor.seed).range(0..=i64::MAX as u64).speed(1.0))
            .on_hover_text("Placement, initial fields, decisions and events follow the seed (next reset).")
            .changed();
        ui.end_row();
    });
    heading(ui, "World", palette);
    let w = &mut editor.draft.world;
    egui::Grid::new("cl_world_size").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Size (cells)");
        ui.horizontal(|ui| {
            changed |= ui
                .add(crate::editor::widgets::dv(&mut w.width).range(MIN_WORLD_SIDE..=MAX_WORLD_SIDE).speed(1.0))
                .changed();
            ui.label("×");
            changed |= ui
                .add(crate::editor::widgets::dv(&mut w.height).range(MIN_WORLD_SIDE..=MAX_WORLD_SIDE).speed(1.0))
                .changed();
        });
        ui.end_row();
        ui.label("Topology");
        egui::ComboBox::from_id_salt("cl_topology").selected_text(topology_label(w.topology)).show_ui(ui, |ui| {
            for (t, label, help) in TOPOLOGIES {
                changed |= ui.selectable_value(&mut w.topology, t, label).on_hover_text(help).changed();
            }
        });
        ui.end_row();
        ui.label("Entity cap");
        changed |= ui
            .add(crate::editor::widgets::dv(&mut w.max_entities).range(1..=MAX_ENTITIES_CEILING).speed(10.0))
            .on_hover_text("Births and spawns beyond this many live entities are refused (and counted).")
            .changed();
        ui.end_row();
    });
    hint(ui, "One cell is one world unit. Size and topology changes take effect at the next reset.", palette);
    changed
}

fn init_ui(ui: &mut Ui, init: &mut FieldInit, id: usize) -> bool {
    let mut changed = false;
    let label = match init {
        FieldInit::Uniform(_) => "Uniform",
        FieldInit::Random { .. } => "Random",
        FieldInit::Patches { .. } => "Patches",
        FieldInit::Gradient { .. } => "Gradient",
    };
    ui.label("Initial content");
    egui::ComboBox::from_id_salt(("cl_field_init", id)).selected_text(label).show_ui(ui, |ui| {
        let options = [
            ("Uniform", FieldInit::Uniform(1.0)),
            ("Random", FieldInit::Random { min: 0.0, max: 1.0 }),
            ("Patches", FieldInit::Patches { count: 6, radius: 4.0, amount: 1.0 }),
            ("Gradient", FieldInit::Gradient { from: 0.0, to: 1.0, axis: Axis::X }),
        ];
        for (l, value) in options {
            if ui.selectable_label(l == label, l).clicked() && l != label {
                *init = value;
                changed = true;
            }
        }
    });
    ui.end_row();
    match init {
        FieldInit::Uniform(v) => changed |= num_f32(ui, "Amount per cell", v, 0.0..=1.0e6, 0.01),
        FieldInit::Random { min, max } => {
            changed |= num_f32(ui, "Minimum", min, 0.0..=1.0e6, 0.01);
            changed |= num_f32(ui, "Maximum", max, 0.0..=1.0e6, 0.01);
        }
        FieldInit::Patches { count, radius, amount } => {
            ui.label("Patches");
            changed |= ui.add(crate::editor::widgets::dv(count).range(0..=10_000)).changed();
            ui.end_row();
            changed |= num_f32(ui, "Patch radius", radius, 0.0..=4096.0, 0.05);
            changed |= num_f32(ui, "Amount per cell", amount, 0.0..=1.0e6, 0.01);
        }
        FieldInit::Gradient { from, to, axis } => {
            changed |= num_f32(ui, "From", from, 0.0..=1.0e6, 0.01);
            changed |= num_f32(ui, "To", to, 0.0..=1.0e6, 0.01);
            changed |= axis_ui(ui, axis);
        }
    }
    changed
}

fn axis_ui(ui: &mut Ui, axis: &mut Axis) -> bool {
    let mut changed = false;
    ui.label("Axis");
    ui.horizontal(|ui| {
        changed |= ui.radio_value(axis, Axis::X, "x (left to right)").changed();
        changed |= ui.radio_value(axis, Axis::Y, "y (top to bottom)").changed();
    });
    ui.end_row();
    changed
}

fn resources_section(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) -> bool {
    let mut changed = false;
    heading(ui, "Resource fields", palette);
    hint(
        ui,
        "Per tick and cell: R := R·(1 − decay); R := min(max, R + regen) below max; then explicit diffusion \
         R := R + D·Σ(R_n − R) over 4 neighbours (stable for D ≤ 0.25).",
        palette,
    );
    let namespace = editor.namespace();
    let mut rename = None;
    let mut remove = None;
    let n = editor.draft.world.resources.len();
    for k in 0..n {
        let name = editor.draft.world.resources[k].name.clone();
        let errors = editor.diagnostics.iter().any(|d| d.is_error() && d.item == Some(ItemRef::Resource(k as u16)));
        let title = format!("{}{name}", if errors { "⚠ " } else { "" });
        egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(("cl_res", k)).default_open(k == 0).show(
            ui,
            |ui| {
                egui::Grid::new(("cl_res_grid", k)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    let taken: Vec<&str> = namespace.iter().map(String::as_str).collect();
                    if let Some(new) = name_field(ui, egui::Id::new(("cl_res_name", k)), &name, &taken, true) {
                        rename = Some((name.clone(), new));
                    }
                    ui.end_row();
                    let r = &mut editor.draft.world.resources[k];
                    ui.label("Colour");
                    changed |= crate::editor::widgets::color_edit(ui, &mut r.color);
                    ui.end_row();
                    changed |= init_ui(ui, &mut r.initial, k);
                    changed |= num_f32(ui, "Regrowth per tick", &mut r.regen, 0.0..=100.0, 0.001);
                    changed |= num_f32(ui, "Maximum per cell", &mut r.max, 0.0..=1.0e6, 0.01);
                    changed |= num_f32(ui, "Diffusion", &mut r.diffusion, 0.0..=0.25, 0.001);
                    changed |= num_f32(ui, "Decay per tick", &mut r.decay, 0.0..=1.0, 0.001);
                    changed |= num_f64(ui, "Energy value", &mut r.energy_value, 0.0..=1.0e6, 0.01);
                });
                if ui.small_button("Remove field").clicked() {
                    remove = Some(k);
                }
            },
        );
    }
    if ui.button("+ Resource field").clicked() {
        let name = unique_name("resource", namespace.iter().map(String::as_str));
        let mut r = ResourceField::new(&name);
        r.color = new_color(editor.draft.world.resources.len() + 3);
        r.initial = FieldInit::Uniform(1.0);
        r.regen = 0.02;
        editor.draft.world.resources.push(r);
        changed = true;
    }
    if let Some((old, new)) = rename {
        rename_resource(&mut editor.draft, &old, &new);
        changed = true;
    }
    if let Some(k) = remove {
        editor.draft.world.resources.remove(k);
        changed = true;
    }
    changed
}

fn environment_section(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) -> bool {
    let mut changed = false;
    heading(ui, "Environment fields", palette);
    hint(ui, "Read-only quantities computed from position and time; rules read them with field(\"name\").", palette);
    let namespace = editor.namespace();
    let (w, h) = (f64::from(editor.draft.world.width), f64::from(editor.draft.world.height));
    let mut rename = None;
    let mut remove = None;
    for k in 0..editor.draft.world.environment.len() {
        let name = editor.draft.world.environment[k].name.clone();
        egui::CollapsingHeader::new(RichText::new(&name).strong()).id_salt(("cl_env", k)).show(ui, |ui| {
            egui::Grid::new(("cl_env_grid", k)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Name");
                let taken: Vec<&str> = namespace.iter().map(String::as_str).collect();
                if let Some(new) = name_field(ui, egui::Id::new(("cl_env_name", k)), &name, &taken, true) {
                    rename = Some((name.clone(), new));
                }
                ui.end_row();
                let e = &mut editor.draft.world.environment[k];
                let label = match e.pattern {
                    EnvPattern::Constant(_) => "Constant",
                    EnvPattern::Gradient { .. } => "Gradient",
                    EnvPattern::Radial { .. } => "Radial",
                };
                ui.label("Pattern");
                egui::ComboBox::from_id_salt(("cl_env_pattern", k)).selected_text(label).show_ui(ui, |ui| {
                    let options = [
                        ("Constant", EnvPattern::Constant(1.0)),
                        ("Gradient", EnvPattern::Gradient { from: 0.0, to: 1.0, axis: Axis::X }),
                        (
                            "Radial",
                            EnvPattern::Radial { x: w / 2.0, y: h / 2.0, radius: w.min(h) / 2.0, peak: 1.0, edge: 0.0 },
                        ),
                    ];
                    for (l, value) in options {
                        if ui.selectable_label(l == label, l).clicked() && l != label {
                            e.pattern = value;
                            changed = true;
                        }
                    }
                });
                ui.end_row();
                match &mut e.pattern {
                    EnvPattern::Constant(v) => changed |= num_f64(ui, "Value", v, -1.0e6..=1.0e6, 0.01),
                    EnvPattern::Gradient { from, to, axis } => {
                        changed |= num_f64(ui, "From", from, -1.0e6..=1.0e6, 0.01);
                        changed |= num_f64(ui, "To", to, -1.0e6..=1.0e6, 0.01);
                        changed |= axis_ui(ui, axis);
                    }
                    EnvPattern::Radial { x, y, radius, peak, edge } => {
                        changed |= num_f64(ui, "Centre x", x, 0.0..=w, 0.1);
                        changed |= num_f64(ui, "Centre y", y, 0.0..=h, 0.1);
                        changed |= num_f64(ui, "Radius", radius, 0.0..=w.max(h), 0.1);
                        changed |= num_f64(ui, "Value at centre", peak, -1.0e6..=1.0e6, 0.01);
                        changed |= num_f64(ui, "Value at edge", edge, -1.0e6..=1.0e6, 0.01);
                    }
                }
                ui.label("Seasonal");
                let mut on = e.seasonal.is_some();
                if ui.checkbox(&mut on, "× (1 + a·sin(2π·tick / period))").changed() {
                    e.seasonal = on.then_some(Seasonality { amplitude: 0.2, period: 500.0 });
                    changed = true;
                }
                ui.end_row();
                if let Some(s) = &mut e.seasonal {
                    changed |= num_f64(ui, "Amplitude", &mut s.amplitude, 0.0..=10.0, 0.005);
                    changed |= num_f64(ui, "Period (ticks)", &mut s.period, 1.0..=1.0e7, 1.0);
                }
            });
            if ui.small_button("Remove field").clicked() {
                remove = Some(k);
            }
        });
    }
    if ui.button("+ Environment field").clicked() {
        let name = unique_name("light", namespace.iter().map(String::as_str));
        editor.draft.world.environment.push(EnvironmentField {
            name,
            pattern: EnvPattern::Gradient { from: 0.2, to: 1.0, axis: Axis::X },
            seasonal: None,
        });
        changed = true;
    }
    if let Some((old, new)) = rename {
        rename_environment(&mut editor.draft, &old, &new);
        changed = true;
    }
    if let Some(k) = remove {
        editor.draft.world.environment.remove(k);
        changed = true;
    }
    changed
}

fn events_section(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) -> bool {
    let mut changed = false;
    heading(ui, "Random events", palette);
    hint(ui, "Each tick every event happens with its probability, at a uniformly random place.", palette);
    let types = editor.type_names();
    let resources = editor.resource_names();
    let mut remove = None;
    for (k, ev) in editor.draft.world.random_events.iter_mut().enumerate() {
        egui::CollapsingHeader::new(RichText::new(&ev.name).strong()).id_salt(("cl_event", k)).show(ui, |ui| {
            egui::Grid::new(("cl_event_grid", k)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Name");
                changed |= ui.add(egui::TextEdit::singleline(&mut ev.name).desired_width(200.0)).changed();
                ui.end_row();
                ui.label("Probability per tick");
                changed |= ui
                    .add(crate::editor::widgets::dv(&mut ev.probability).range(0.0..=1.0).speed(0.0005).max_decimals(4))
                    .changed();
                ui.end_row();
                let label = match ev.effect {
                    EventEffect::ResourcePulse { .. } => "Resource pulse",
                    EventEffect::ScaleResource { .. } => "Scale resource",
                    EventEffect::Catastrophe { .. } => "Catastrophe",
                    EventEffect::Immigration { .. } => "Immigration",
                };
                ui.label("Effect");
                egui::ComboBox::from_id_salt(("cl_event_effect", k)).selected_text(label).show_ui(ui, |ui| {
                    let first_res = resources.first().cloned().unwrap_or_default();
                    let options = [
                        (
                            "Resource pulse",
                            EventEffect::ResourcePulse { resource: first_res.clone(), amount: 1.0, radius: 4.0 },
                        ),
                        ("Scale resource", EventEffect::ScaleResource { resource: first_res, factor: 0.5 }),
                        ("Catastrophe", EventEffect::Catastrophe { radius: 5.0, kill_probability: 0.5, entity: None }),
                        (
                            "Immigration",
                            EventEffect::Immigration { entity: types.first().cloned().unwrap_or_default(), count: 3 },
                        ),
                    ];
                    for (l, value) in options {
                        if ui.selectable_label(l == label, l).clicked() && l != label {
                            ev.effect = value;
                            changed = true;
                        }
                    }
                });
                ui.end_row();
                match &mut ev.effect {
                    EventEffect::ResourcePulse { resource, amount, radius } => {
                        ui.label("Resource");
                        changed |= name_combo(ui, ("cl_ev_res", k), resource, &resources, 120.0);
                        ui.end_row();
                        changed |= num_f32(ui, "Amount per cell", amount, -1.0e6..=1.0e6, 0.01);
                        changed |= num_f32(ui, "Radius", radius, 0.0..=4096.0, 0.05);
                    }
                    EventEffect::ScaleResource { resource, factor } => {
                        ui.label("Resource");
                        changed |= name_combo(ui, ("cl_ev_res", k), resource, &resources, 120.0);
                        ui.end_row();
                        changed |= num_f32(ui, "Factor", factor, 0.0..=100.0, 0.01);
                    }
                    EventEffect::Catastrophe { radius, kill_probability, entity } => {
                        changed |= num_f32(ui, "Radius", radius, 0.0..=4096.0, 0.05);
                        changed |= num_f64(ui, "Kill probability", kill_probability, 0.0..=1.0, 0.005);
                        ui.label("Victims");
                        changed |= optional_name_combo(ui, ("cl_ev_victims", k), entity, "every type", &types, 120.0);
                        ui.end_row();
                    }
                    EventEffect::Immigration { entity, count } => {
                        ui.label("Type");
                        changed |= name_combo(ui, ("cl_ev_type", k), entity, &types, 120.0);
                        ui.end_row();
                        ui.label("Count");
                        changed |= ui.add(crate::editor::widgets::dv(count).range(0..=10_000)).changed();
                        ui.end_row();
                    }
                }
            });
            if ui.small_button("Remove event").clicked() {
                remove = Some(k);
            }
        });
    }
    if ui.button("+ Random event").clicked() {
        let taken: Vec<&str> = editor.draft.world.random_events.iter().map(|e| e.name.as_str()).collect();
        let name = unique_name("event", taken);
        let effect = match (types.first(), resources.first()) {
            (Some(t), _) => EventEffect::Immigration { entity: t.clone(), count: 3 },
            (None, Some(r)) => EventEffect::ResourcePulse { resource: r.clone(), amount: 1.0, radius: 4.0 },
            (None, None) => EventEffect::Catastrophe { radius: 5.0, kill_probability: 0.5, entity: None },
        };
        editor.draft.world.random_events.push(RandomEvent { name, probability: 0.01, effect });
        changed = true;
    }
    if let Some(k) = remove {
        editor.draft.world.random_events.remove(k);
        changed = true;
    }
    changed
}

fn end_label(c: &EndCondition) -> &'static str {
    match c {
        EndCondition::TickLimit(_) => "Tick limit",
        EndCondition::Extinction(_) => "Extinction of",
        EndCondition::AllExtinct => "All extinct",
        EndCondition::PopulationAbove { .. } => "Population above",
        EndCondition::PopulationBelow { .. } => "Population below",
    }
}

fn end_section(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) -> bool {
    let mut changed = false;
    heading(ui, "End conditions", palette);
    hint(ui, "The run finishes (pauses) when one is met, unless “Pause on end condition” is off.", palette);
    let types = editor.type_names();
    let mut remove = None;
    for (k, c) in editor.draft.end_conditions.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            let label = end_label(c);
            egui::ComboBox::from_id_salt(("cl_end_kind", k)).selected_text(label).width(130.0).show_ui(ui, |ui| {
                let first = types.first().cloned().unwrap_or_default();
                let options = [
                    EndCondition::TickLimit(10_000),
                    EndCondition::Extinction(first),
                    EndCondition::AllExtinct,
                    EndCondition::PopulationAbove { entity: None, count: 5_000 },
                    EndCondition::PopulationBelow { entity: None, count: 10 },
                ];
                for value in options {
                    let l = end_label(&value);
                    if ui.selectable_label(l == label, l).clicked() && l != label {
                        *c = value;
                        changed = true;
                    }
                }
            });
            match c {
                EndCondition::TickLimit(t) => {
                    changed |= ui.add(crate::editor::widgets::dv(t).range(1..=u64::MAX / 2).suffix(" ticks")).changed();
                }
                EndCondition::Extinction(t) => {
                    changed |= name_combo(ui, ("cl_end_type", k), t, &types, 110.0);
                }
                EndCondition::AllExtinct => {}
                EndCondition::PopulationAbove { entity, count } | EndCondition::PopulationBelow { entity, count } => {
                    changed |= optional_name_combo(ui, ("cl_end_pop", k), entity, "all types", &types, 110.0);
                    changed |= ui.add(crate::editor::widgets::dv(count).range(0..=MAX_ENTITIES_CEILING)).changed();
                }
            }
            if ui.small_button("×").clicked() {
                remove = Some(k);
            }
        });
    }
    if ui.button("+ End condition").clicked() {
        editor.draft.end_conditions.push(EndCondition::AllExtinct);
        changed = true;
    }
    if let Some(k) = remove {
        editor.draft.end_conditions.remove(k);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topologies_are_labelled_and_finite() {
        assert_eq!(topology_label(Topology::Torus), "Torus");
        assert_eq!(topology_label(Topology::Infinite), "Infinite (unsupported)");
        assert!(TOPOLOGIES.iter().all(|(t, _, _)| *t != Topology::Infinite));
        assert_eq!(end_label(&EndCondition::AllExtinct), "All extinct");
    }
}
