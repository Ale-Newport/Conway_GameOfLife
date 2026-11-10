//! Rhai script editor: script rules (entity, event, priority, enabled),
//! source with highlighting and compile errors at `line:col`, runtime
//! errors reported by the running simulation (each disables its script),
//! and the sandbox limits.

use bevy_egui::egui::{self, RichText, Ui};
use interface::Palette;
use rule_engine::def::{ScriptLimits, ScriptRule};
use rule_engine::dsl::{Event, Name};
use rule_engine::{CustomLifeCommand, ItemRef};
use simulation_core::Control;

use super::widgets::{
    Lang, code_editor, confirm_delete, diagnostics_list, error_ranges, heading, hint, list_row, name_combo,
    optional_name_combo, split,
};
use super::{ClEditor, EditorParams};
use crate::draft::unique_name;
use crate::highlight::{RHAI_ACTIONS, RHAI_QUERIES};

/// Source of a new script.
pub const TEMPLATE: &str = "// `me` is the acting entity. Actions only record intents;\n\
// the engine resolves them like DSL actions, after the type's rules.\n\
fn run(me) {\n    if me.energy < 5.0 {\n        stay();\n    } else {\n        wander();\n    }\n}\n";

/// Draws the tab.
pub fn show(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    let runtime: Vec<rule_engine::ScriptError> =
        p.data.view.as_ref().map(|v| v.script_errors.clone()).unwrap_or_default();
    let runner = p.runner.as_deref().cloned();
    split(
        ui,
        "cl_scripts",
        210.0,
        &mut *p.editor,
        |ui, editor| list(ui, editor, palette),
        |ui, editor| {
            detail(ui, editor, &runtime, palette);
            ui.add_space(8.0);
            if !runtime.is_empty() {
                heading(ui, "Runtime errors (running system)", palette);
                for e in &runtime {
                    ui.label(RichText::new(e.to_string()).small().color(palette.error));
                }
                if let Some(runner) = &runner
                    && ui
                        .button("Re-enable scripts")
                        .on_hover_text("Clear the error list and run disabled scripts again (apply a fix first).")
                        .clicked()
                {
                    runner.send(Control::Command(CustomLifeCommand::ReenableScripts));
                }
            }
            limits(ui, editor, palette);
            api_reference(ui, palette);
        },
    );
}

fn list(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) {
    heading(ui, "Script rules", palette);
    let mut select = None;
    for (i, s) in editor.draft.scripts.iter().enumerate() {
        let errors = editor.diagnostics.iter().any(|d| d.is_error() && d.item == Some(ItemRef::Script(i as u16)));
        let label = format!("{}{}", if errors { "⚠ " } else { "" }, s.name);
        let sub = if s.enabled { s.entity.clone() } else { format!("{} (off)", s.entity) };
        if list_row(ui, editor.sel_script == Some(i), None, &label, &sub) {
            select = Some(i);
        }
    }
    if editor.draft.scripts.is_empty() {
        hint(ui, "No scripts. Prefer DSL rules for large populations: a script call costs about 1 µs.", palette);
    }
    if let Some(i) = select {
        editor.sel_script = Some(i);
    }
    ui.add_space(6.0);
    let types = editor.type_names();
    if ui.add_enabled(!types.is_empty(), egui::Button::new("+ Script")).clicked() {
        let entity = types.get(editor.sel_type).or(types.first()).cloned().unwrap_or_default();
        let taken: Vec<&str> = editor.draft.scripts.iter().map(|s| s.name.as_str()).collect();
        let name = unique_name("script", taken);
        editor.draft.scripts.push(ScriptRule {
            name,
            entity,
            event: Event::Tick,
            priority: 0,
            source: TEMPLATE.to_string(),
            enabled: true,
        });
        editor.sel_script = Some(editor.draft.scripts.len() - 1);
        editor.touch();
    }
}

fn event_label(e: &Event) -> &'static str {
    match e {
        Event::Tick => "every tick",
        Event::Birth => "on birth",
        Event::Death => "on death",
        Event::Collision { .. } => "on collision",
        Event::StateEnter { .. } => "on state enter",
    }
}

fn detail(ui: &mut Ui, editor: &mut ClEditor, runtime: &[rule_engine::ScriptError], palette: &Palette) {
    let Some(i) = editor.sel_script.filter(|&i| i < editor.draft.scripts.len()) else {
        hint(ui, "Select a script or add one.", palette);
        return;
    };
    let types = editor.type_names();
    let diags = editor.diagnostics_of(ItemRef::Script(i as u16));
    let states: Vec<String> = editor
        .draft
        .entity_types
        .iter()
        .find(|t| t.name == editor.draft.scripts[i].entity)
        .map(|t| t.state_names())
        .unwrap_or_default();
    let mut changed = false;
    let s = &mut editor.draft.scripts[i];
    egui::Grid::new(("cl_script_head", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        changed |= ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(180.0)).changed();
        ui.end_row();
        ui.label("For");
        changed |= name_combo(ui, ("cl_script_entity", i), &mut s.entity, &types, 140.0);
        ui.end_row();
        ui.label("Event");
        ui.horizontal(|ui| {
            let label = event_label(&s.event);
            egui::ComboBox::from_id_salt(("cl_script_event", i)).selected_text(label).show_ui(ui, |ui| {
                let options = [
                    Event::Tick,
                    Event::Birth,
                    Event::Death,
                    Event::Collision { with: None },
                    Event::StateEnter { state: Name::new(states.first().cloned().unwrap_or_default()) },
                ];
                for value in options {
                    let l = event_label(&value);
                    if ui.selectable_label(l == label, l).clicked() && l != label {
                        s.event = value;
                        changed = true;
                    }
                }
            });
            match &mut s.event {
                Event::Collision { with } => {
                    let mut name = with.as_ref().map(|n| n.text.clone());
                    if optional_name_combo(ui, ("cl_script_with", i), &mut name, "any type", &types, 110.0) {
                        *with = name.map(Name::new);
                        changed = true;
                    }
                }
                Event::StateEnter { state } => {
                    let mut name = state.text.clone();
                    if name_combo(ui, ("cl_script_state", i), &mut name, &states, 110.0) {
                        *state = Name::new(name);
                        changed = true;
                    }
                }
                _ => {}
            }
        });
        ui.end_row();
        ui.label("Priority");
        changed |= ui
            .add(crate::editor::widgets::dv(&mut s.priority).range(-1_000_000..=1_000_000))
            .on_hover_text("Among this type's scripts (higher first); scripts always run after the DSL rules.")
            .changed();
        ui.end_row();
        ui.label("Enabled");
        changed |= ui.checkbox(&mut s.enabled, "run this script").changed();
        ui.end_row();
    });
    heading(ui, "Source (Rhai)", palette);
    let mut source = s.source.clone();
    let errors = error_ranges(&source, &diags);
    let response = code_editor(ui, ("cl_script_source", i), &mut source, Lang::Rhai, &|_| false, &errors, palette, 12);
    if response.changed() {
        editor.draft.scripts[i].source = source;
        changed = true;
    }
    hint(ui, "The script must define `fn run(me)`; top-level statements never run.", palette);
    let ops = editor.draft.script_limits.max_operations;
    hint(
        ui,
        &format!(
            "Sandboxed: no files, network, clock or modules; at most {ops} operations per call \
             (limits under “Sandbox and limits” below)."
        ),
        palette,
    );
    heading(ui, "Compile errors", palette);
    diagnostics_list(ui, &diags, palette, "The script compiles.");
    let script = &editor.draft.scripts[i];
    let mine: Vec<&rule_engine::ScriptError> =
        runtime.iter().filter(|e| e.script == script.name && e.entity == script.entity).collect();
    if !mine.is_empty() {
        heading(ui, "This script failed while running", palette);
        for e in mine {
            let at = match (e.line, e.column) {
                (Some(l), Some(c)) => format!("{l}:{c} "),
                _ => String::new(),
            };
            ui.label(RichText::new(format!("tick {}: {at}{:?} — {}", e.tick, e.kind, e.message)).color(palette.error));
        }
    }
    ui.add_space(6.0);
    if confirm_delete(ui, egui::Id::new(("cl_delete_script", i)), "this script", palette) {
        editor.draft.scripts.remove(i);
        editor.sel_script = if editor.draft.scripts.is_empty() { None } else { Some(i.saturating_sub(1)) };
        changed = true;
    }
    if changed {
        editor.touch();
    }
}

fn limits(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) {
    egui::CollapsingHeader::new(RichText::new("Sandbox and limits").strong())
        .id_salt("cl_sandbox")
        .default_open(false)
        .show(ui, |ui| {
            hint(
                ui,
                "Scripts run in a Rhai engine without file, network, process, clock or module access \
                 (eval/import disabled). Only the operation budget decides outcomes, so runs stay deterministic; \
                 any runtime error disables that script and is listed above.",
                palette,
            );
            let mut changed = false;
            let c = ScriptLimits::CEILING;
            let l = &mut editor.draft.script_limits;
            egui::Grid::new("cl_limits").num_columns(3).spacing([10.0, 4.0]).show(ui, |ui| {
                ui.label("Operations per call");
                changed |= ui
                    .add(crate::editor::widgets::dv(&mut l.max_operations).range(1..=c.max_operations).speed(100.0))
                    .changed();
                ui.label(RichText::new(format!("≤ {}", c.max_operations)).small().weak());
                ui.end_row();
                let mut row = |ui: &mut Ui, label: &str, v: &mut usize, max: usize| {
                    ui.label(label);
                    changed |= ui.add(crate::editor::widgets::dv(v).range(1..=max).speed(1.0)).changed();
                    ui.label(RichText::new(format!("≤ {max}")).small().weak());
                    ui.end_row();
                };
                row(ui, "Call depth", &mut l.max_call_levels, c.max_call_levels);
                row(ui, "Expression depth", &mut l.max_expr_depth, c.max_expr_depth);
                row(ui, "String bytes", &mut l.max_string_size, c.max_string_size);
                row(ui, "Array length", &mut l.max_array_size, c.max_array_size);
                row(ui, "Map entries", &mut l.max_map_size, c.max_map_size);
                row(ui, "Variables", &mut l.max_variables, c.max_variables);
                row(ui, "Functions", &mut l.max_functions, c.max_functions);
            });
            if ui.small_button("Restore defaults").clicked() {
                editor.draft.script_limits = ScriptLimits::default();
                changed = true;
            }
            let diags = editor.diagnostics_of(ItemRef::ScriptLimits);
            diagnostics_list(ui, &diags, palette, "Limits are within the ceilings.");
            if changed {
                editor.touch();
            }
        });
}

fn api_reference(ui: &mut Ui, palette: &Palette) {
    egui::CollapsingHeader::new(RichText::new("Script API").strong()).id_salt("cl_rhai_api").show(ui, |ui| {
        hint(
            ui,
            "Entity handles (`me`, results of `nearest`, `other()`) have read-only getters exists, id, kind, state, \
             energy, age, state_age, size, speed, x, y, generation, distance and the method prop(\"name\").",
            palette,
        );
        ui.label(RichText::new("Queries").strong());
        ui.label(RichText::new(RHAI_QUERIES.join(", ")).monospace().small());
        hint(
            ui,
            "nearby(kind[, radius]): int · nearest(kind[, radius]): entity · field(name): float · \
                  population(kind): int · other() · tick() · random()",
            palette,
        );
        ui.label(RichText::new("Actions (record intents)").strong());
        ui.label(RichText::new(RHAI_ACTIONS.join(", ")).monospace().small());
        hint(
            ui,
            "`spawn` is a reserved word in Rhai, hence spawn_entity(type[, n]). on_death scripts may only \
                  emit and spawn_entity.",
            palette,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::{compile, scenarios};

    #[test]
    fn the_template_compiles_for_a_mobile_type() {
        let mut sys = scenarios::builtin_by_id("robot-swarm").unwrap().system;
        sys.scripts.push(ScriptRule {
            name: "template".into(),
            entity: "robot".into(),
            event: Event::Tick,
            priority: 0,
            source: TEMPLATE.into(),
            enabled: true,
        });
        compile(&sys).unwrap();
        // A broken script is reported with its position.
        sys.scripts[1].source = "fn run(me) {\n  let x = ;\n}".into();
        let diags = rule_engine::validate(&sys);
        let e = diags.iter().find(|d| d.code == "E030").expect("compile error");
        assert_eq!(e.item, Some(ItemRef::Script(1)));
        assert_eq!(e.span.map(|s| s.line), Some(2));
    }
}
