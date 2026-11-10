//! Rule editor: the list of hand-written and generated rules, a visual
//! builder (event, condition rows in AND groups joined by OR, actions with
//! parameter slots, modifiers) that generates DSL text, and a DSL text
//! editor with syntax highlighting and live `line:col` diagnostics from the
//! rule-engine parser and validator. Both edit the same rule; whichever is
//! used, the draft holds the parsed rule.

use bevy_egui::egui::{self, RichText, Ui};
use interface::Palette;
use interface::theme::color32_from_rgba;
use rule_engine::compile::ir::RuleOrigin;
use rule_engine::compile::vocab::{self, FunctionKind, ParamSig, ValueKind};
use rule_engine::dsl::{RuleAst, print_rule};
use rule_engine::{Diagnostic, ItemRef, check_rule, check_rule_text};

use super::widgets::{
    Lang, code_editor, code_view, confirm_delete, diagnostics_list, error_ranges, heading, hint, name_combo, split,
};
use super::{ClEditor, EditorParams};
use crate::blocks::{
    ActionBlock, ArgValue, BlockError, Cmp, CondGroup, Condition, EntitySel, EventBlock, Operand, RuleBlocks, Who,
};
use crate::draft::{detach_rule, unique_name};

/// How the selected rule is edited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RuleMode {
    /// Visual builder.
    #[default]
    Visual,
    /// DSL text.
    Text,
}

/// Buffers of the selected rule.
#[derive(Clone, Debug, Default)]
pub struct RuleEditor {
    /// Builder or text.
    pub mode: RuleMode,
    /// Manual rule held by the buffers.
    pub index: Option<usize>,
    synced: Option<RuleAst>,
    /// DSL text of the rule (as typed).
    pub text: String,
    /// Parser and validator diagnostics of `text` (positions inside `text`).
    pub text_diags: Vec<Diagnostic>,
    /// Builder blocks.
    pub blocks: Option<RuleBlocks>,
    /// Why the blocks do not form a rule, if they do not.
    pub block_error: Option<BlockError>,
}

impl RuleEditor {
    /// Reloads the buffers when another rule is selected or the draft's
    /// rule changed elsewhere (renames, undo of the draft...).
    pub fn sync(&mut self, editor_rules: &[RuleAst], selected: Option<usize>, system: &rule_engine::def::LifeSystem) {
        let current = selected.and_then(|i| editor_rules.get(i));
        if self.index == selected && self.synced.as_ref() == current {
            return;
        }
        self.index = selected;
        self.synced = current.cloned();
        self.block_error = None;
        match current {
            Some(rule) => {
                self.text = print_rule(rule);
                self.blocks = Some(RuleBlocks::from_ast(rule));
                self.text_diags = check_rule(system, rule);
            }
            None => {
                self.text.clear();
                self.blocks = None;
                self.text_diags.clear();
            }
        }
    }

    /// Takes edited text: returns the parsed rule when the text is
    /// syntactically valid (semantic errors are kept in `text_diags`).
    pub fn take_text(&mut self, system: &rule_engine::def::LifeSystem) -> Option<RuleAst> {
        let (rule, diags) = check_rule_text(system, &self.text);
        self.text_diags = diags;
        let rule = rule?;
        self.blocks = Some(RuleBlocks::from_ast(&rule));
        self.block_error = None;
        self.synced = Some(rule.clone());
        Some(rule)
    }

    /// Takes edited blocks: returns the rule they form and regenerates the text.
    pub fn take_blocks(&mut self, system: &rule_engine::def::LifeSystem) -> Option<RuleAst> {
        let blocks = self.blocks.as_ref()?;
        match blocks.to_ast() {
            Ok(rule) => {
                self.block_error = None;
                self.text = print_rule(&rule);
                self.text_diags = check_rule(system, &rule);
                self.synced = Some(rule.clone());
                Some(rule)
            }
            Err(e) => {
                self.block_error = Some(e);
                None
            }
        }
    }
}

/// Names the builder offers.
struct Ctx {
    types: Vec<String>,
    kinds: Vec<String>,
    states: Vec<String>,
    props: Vec<String>,
    writable: Vec<String>,
    other_props: Vec<String>,
    resources: Vec<String>,
    fields: Vec<String>,
    products: Vec<String>,
}

impl Ctx {
    fn new(sys: &rule_engine::def::LifeSystem, entity: &str, collision_with: Option<&str>) -> Self {
        let types: Vec<String> = sys.entity_types.iter().map(|t| t.name.clone()).collect();
        let ty = sys.entity_types.iter().find(|t| t.name == entity);
        let builtin = || vocab::BUILTIN_PROPERTIES.iter().map(|p| p.name.to_string());
        let user = |t: Option<&rule_engine::def::EntityType>| {
            t.map(|t| t.properties.iter().map(|p| p.name.clone()).collect::<Vec<_>>()).unwrap_or_default()
        };
        let carried = |t: Option<&rule_engine::def::EntityType>| {
            t.map(|t| t.carries.iter().map(|c| c.resource.clone()).collect::<Vec<_>>()).unwrap_or_default()
        };
        let mut props: Vec<String> = builtin().collect();
        props.extend(user(ty));
        props.extend(carried(ty));
        props.extend(["state".to_string(), "type".to_string()]);
        let mut writable: Vec<String> =
            vocab::BUILTIN_PROPERTIES.iter().filter(|p| p.writable).map(|p| p.name.to_string()).collect();
        writable.extend(user(ty));
        let other_ty = collision_with.and_then(|o| sys.entity_types.iter().find(|t| t.name == o));
        let mut other_props: Vec<String> = builtin().collect();
        other_props.extend(user(other_ty));
        other_props.extend(carried(other_ty));
        other_props.extend(["state".to_string(), "type".to_string()]);
        let states = ty.map(|t| t.state_names()).unwrap_or_default();
        let resources: Vec<String> = sys.world.resources.iter().map(|r| r.name.clone()).collect();
        let fields: Vec<String> =
            resources.iter().cloned().chain(sys.world.environment.iter().map(|e| e.name.clone())).collect();
        let mut kinds = types.clone();
        kinds.extend(["same_species".to_string(), "any".to_string()]);
        let mut products = states.clone();
        products.extend(types.iter().cloned());
        products.extend(resources.iter().cloned());
        Self { types, kinds, states, props, writable, other_props, resources, fields, products }
    }
}

/// Draws the tab.
pub fn show(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    let fired: Vec<(String, String, u64, bool)> = p
        .data
        .view
        .as_ref()
        .map(|v| {
            v.rules.iter().map(|r| (r.entity.clone(), r.name.clone(), r.fired_total, r.origin.is_generated())).collect()
        })
        .unwrap_or_default();
    split(
        ui,
        "cl_rules",
        240.0,
        &mut *p.editor,
        |ui, editor| list(ui, editor, &fired, palette),
        |ui, editor| detail(ui, editor, &fired, palette),
    );
}

fn fired_of(fired: &[(String, String, u64, bool)], entity: &str, name: &str) -> Option<u64> {
    fired.iter().find(|(e, n, _, _)| e == entity && n == name).map(|(_, _, f, _)| *f)
}

fn list(ui: &mut Ui, editor: &mut ClEditor, fired: &[(String, String, u64, bool)], palette: &Palette) {
    heading(ui, "Hand-written rules", palette);
    let mut select = None;
    for (i, r) in editor.draft.rules.iter().enumerate() {
        let color = editor
            .draft
            .entity_types
            .iter()
            .find(|t| t.name == r.entity.text)
            .map_or(palette.text_muted, |t| color32_from_rgba(t.color));
        let errors = editor.diagnostics.iter().any(|d| d.is_error() && d.item == Some(ItemRef::Rule(i as u16)));
        let fired = fired_of(fired, &r.entity.text, &r.name).map(|f| format!("{f}×")).unwrap_or_default();
        let label = format!("{}{} · {}", if errors { "⚠ " } else { "" }, r.name, r.entity.text);
        let on = editor.sel_rule == Some(i) && editor.sel_generated.is_none();
        let response = ui
            .horizontal(|ui| {
                interface::widgets::swatch(ui, color, 10.0);
                let r = ui.selectable_label(on, label);
                if !fired.is_empty() {
                    ui.label(RichText::new(fired).small().color(palette.text_muted));
                }
                r
            })
            .inner;
        if response.clicked() {
            select = Some(i);
        }
    }
    if editor.draft.rules.is_empty() {
        hint(ui, "No hand-written rules yet.", palette);
    }
    if let Some(i) = select {
        editor.sel_rule = Some(i);
        editor.sel_generated = None;
    }
    ui.add_space(6.0);
    let types = editor.type_names();
    if !types.contains(&editor.new_rule_entity) {
        editor.new_rule_entity = types.first().cloned().unwrap_or_default();
    }
    ui.horizontal(|ui| {
        let mut entity = editor.new_rule_entity.clone();
        name_combo(ui, "cl_new_rule_entity", &mut entity, &types, 100.0);
        editor.new_rule_entity = entity;
        let add = ui.add_enabled(!types.is_empty(), egui::Button::new("+ Rule"));
        if add.clicked() {
            let entity = editor.new_rule_entity.clone();
            let taken: Vec<String> =
                editor.draft.rules.iter().filter(|r| r.entity.text == entity).map(|r| r.name.clone()).collect();
            let name = unique_name("new_rule", taken.iter().map(String::as_str));
            let mobile =
                editor.draft.entity_types.iter().find(|t| t.name == entity).is_some_and(|t| t.movement.is_mobile());
            let action = if mobile { "wander" } else { "set_state" };
            let mut blocks = RuleBlocks::new(&name, &entity, action);
            if !mobile {
                let state = editor
                    .draft
                    .entity_types
                    .iter()
                    .find(|t| t.name == entity)
                    .and_then(|t| t.state_names().first().cloned())
                    .unwrap_or_default();
                blocks.actions =
                    vec![ActionBlock::Call { name: "set_state".into(), args: vec![Some(ArgValue::Name(state))] }];
            }
            if let Ok(rule) = blocks.to_ast() {
                editor.draft.rules.push(rule);
                editor.sel_rule = Some(editor.draft.rules.len() - 1);
                editor.sel_generated = None;
                editor.touch();
            }
        }
    });
    if let Some(i) = editor.sel_rule.filter(|_| editor.sel_generated.is_none()) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(i > 0, egui::Button::new("Up").small())
                .on_hover_text("Earlier (ties keep declaration order)")
                .clicked()
            {
                editor.draft.rules.swap(i, i - 1);
                editor.sel_rule = Some(i - 1);
                editor.touch();
            }
            if ui.add_enabled(i + 1 < editor.draft.rules.len(), egui::Button::new("Down").small()).clicked() {
                editor.draft.rules.swap(i, i + 1);
                editor.sel_rule = Some(i + 1);
                editor.touch();
            }
            if ui.small_button("Duplicate").clicked()
                && let Some(rule) = editor.draft.rules.get(i).cloned()
            {
                let taken: Vec<String> = editor
                    .draft
                    .rules
                    .iter()
                    .filter(|r| r.entity.text == rule.entity.text)
                    .map(|r| r.name.clone())
                    .collect();
                let mut copy = rule;
                copy.name = unique_name(&format!("{}_copy", copy.name), taken.iter().map(String::as_str));
                editor.draft.rules.insert(i + 1, copy);
                editor.sel_rule = Some(i + 1);
                editor.touch();
            }
        });
    }

    ui.add_space(10.0);
    heading(ui, "Generated rules", palette);
    hint(ui, "Compiled from relationships and reproduction rates; read-only (detach to edit).", palette);
    if editor.compiled_rules.is_empty() && editor.has_errors() {
        hint(ui, "Shown once the draft compiles.", palette);
    }
    let mut select_generated = None;
    for (k, row) in editor.compiled_rules.iter().enumerate() {
        if !row.origin.is_generated() {
            continue;
        }
        let fired = fired_of(fired, &row.entity, &row.name).map(|f| format!(" {f}×")).unwrap_or_default();
        let on = editor.sel_generated == Some(k);
        if ui.selectable_label(on, RichText::new(format!("{} · {}{fired}", row.name, row.entity)).small()).clicked() {
            select_generated = Some(k);
        }
    }
    if let Some(k) = select_generated {
        editor.sel_generated = Some(k);
    }
}

fn origin_text(origin: &RuleOrigin, sys: &rule_engine::def::LifeSystem) -> String {
    match origin {
        RuleOrigin::Manual(i) => format!("hand-written rule #{}", i + 1),
        RuleOrigin::Relationship { index, relation } => {
            let rel = sys.relationships.get(*index);
            rel.map_or_else(
                || format!("`{relation}` relationship #{}", index + 1),
                |r| format!("relationship #{}: {} {} {}", index + 1, r.subject, relation, r.object),
            )
        }
        RuleOrigin::EntityType { entity } => format!("reproduction rate of `{entity}`"),
    }
}

fn detail(ui: &mut Ui, editor: &mut ClEditor, fired: &[(String, String, u64, bool)], palette: &Palette) {
    if let Some(k) = editor.sel_generated {
        generated_detail(ui, editor, k, palette);
        return;
    }
    let sel = editor.sel_rule.filter(|&i| i < editor.draft.rules.len());
    editor.rules.sync(&editor.draft.rules, sel, &editor.draft);
    let Some(i) = sel else {
        hint(ui, "Select a rule on the left or create one with “+ Rule”.", palette);
        vocabulary(ui, palette);
        return;
    };
    let rule = editor.draft.rules[i].clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("rule {} for {}", rule.name, rule.entity.text)).strong());
        if let Some(f) = fired_of(fired, &rule.entity.text, &rule.name) {
            ui.label(RichText::new(format!("fired {f} times since the last reset")).small().color(palette.text_muted));
        }
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut editor.rules.mode, RuleMode::Visual, "Visual builder");
        ui.selectable_value(&mut editor.rules.mode, RuleMode::Text, "DSL text");
    });
    ui.separator();
    let mut new_rule = None;
    match editor.rules.mode {
        RuleMode::Text => {
            let names = |w: &str| editor.is_name(w);
            let errors = error_ranges(&editor.rules.text, &editor.rules.text_diags);
            let mut text = editor.rules.text.clone();
            let response = code_editor(ui, ("cl_rule_text", i), &mut text, Lang::Dsl, &names, &errors, palette, 8);
            if response.changed() {
                editor.rules.text = text;
                new_rule = editor.rules.take_text(&editor.draft);
            }
            ui.horizontal(|ui| {
                if ui.button("Format").on_hover_text("Rewrite the text in canonical form").clicked() {
                    editor.rules.text = print_rule(&rule);
                    editor.rules.text_diags = check_rule(&editor.draft, &rule);
                }
                hint(ui, "Every edit is parsed and checked; positions are line:column in this text.", palette);
            });
            heading(ui, "Diagnostics", palette);
            let diags = editor.rules.text_diags.clone();
            diagnostics_list(ui, &diags, palette, "The rule is valid.");
            if editor.rules.text_diags.iter().any(|d| d.code == "E015") {
                hint(ui, "While the text has syntax errors the draft keeps the last version that parsed.", palette);
            }
        }
        RuleMode::Visual => {
            let mut blocks = editor.rules.blocks.clone().unwrap_or_else(|| RuleBlocks::from_ast(&rule));
            let collision_with = match &blocks.event {
                EventBlock::Collision(with) => with.clone(),
                _ => None,
            };
            let ctx = Ctx::new(&editor.draft, &blocks.entity, collision_with.as_deref());
            let changed = builder(ui, &mut blocks, &ctx, i, palette);
            editor.rules.blocks = Some(blocks);
            if changed {
                new_rule = editor.rules.take_blocks(&editor.draft);
            }
            if let Some(e) = &editor.rules.block_error {
                ui.label(RichText::new(format!("⚠ {e}")).color(palette.error));
                hint(ui, "The draft keeps the last complete version of the rule.", palette);
            }
            heading(ui, "Generated DSL", palette);
            let names = |w: &str| editor.is_name(w);
            code_view(ui, &editor.rules.text, Lang::Dsl, &names, palette);
            let diags = editor.rules.text_diags.clone();
            diagnostics_list(ui, &diags, palette, "The rule is valid.");
        }
    }
    if let Some(rule) = new_rule
        && editor.draft.rules.get(i) != Some(&rule)
    {
        editor.draft.rules[i] = rule;
        editor.touch();
    }
    let item_diags: Vec<Diagnostic> =
        editor.diagnostics_of(ItemRef::Rule(i as u16)).into_iter().filter(|d| d.code == "E016").collect();
    if !item_diags.is_empty() {
        diagnostics_list(ui, &item_diags, palette, "");
    }
    ui.add_space(8.0);
    if confirm_delete(ui, egui::Id::new(("cl_delete_rule", i)), &format!("rule `{}`", rule.name), palette) {
        editor.draft.rules.remove(i);
        editor.sel_rule = if editor.draft.rules.is_empty() { None } else { Some(i.saturating_sub(1)) };
        editor.touch();
    }
    ui.add_space(8.0);
    vocabulary(ui, palette);
}

fn generated_detail(ui: &mut Ui, editor: &mut ClEditor, k: usize, palette: &Palette) {
    let Some(row) = editor.compiled_rules.get(k).cloned() else { return };
    ui.label(RichText::new(format!("rule {} for {}", row.name, row.entity)).strong());
    hint(ui, &format!("Generated from the {}.", origin_text(&row.origin, &editor.draft)), palette);
    let names = |w: &str| editor.is_name(w);
    code_view(ui, &row.text, Lang::Dsl, &names, palette);
    ui.add_space(6.0);
    hint(
        ui,
        "Detach copies every rule generated from this source into the hand-written rules and removes the source \
         (the relationship, or the reproduction rate), so they can be edited freely.",
        palette,
    );
    if ui.button("Detach rules").clicked() {
        let siblings: Vec<RuleAst> =
            editor.compiled_rules.iter().filter(|r| r.origin == row.origin).map(|r| r.ast.clone()).collect();
        let mut first = None;
        for ast in &siblings {
            let i = detach_rule(&mut editor.draft, ast);
            first.get_or_insert(i);
        }
        match &row.origin {
            RuleOrigin::Relationship { index, .. } if *index < editor.draft.relationships.len() => {
                editor.draft.relationships.remove(*index);
            }
            RuleOrigin::EntityType { entity } => {
                if let Some(t) = editor.draft.entity_types.iter_mut().find(|t| &t.name == entity) {
                    t.reproduction.rate = 0.0;
                }
            }
            _ => {}
        }
        editor.sel_generated = None;
        editor.sel_rule = first;
        editor.touch();
    }
}

// ---- builder ------------------------------------------------------------

fn builder(ui: &mut Ui, b: &mut RuleBlocks, ctx: &Ctx, rule: usize, palette: &Palette) -> bool {
    let mut changed = false;
    egui::Grid::new(("cl_rule_head", rule)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        changed |= ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(200.0)).changed();
        ui.end_row();
        ui.label("For");
        changed |= name_combo(ui, ("cl_rule_entity", rule), &mut b.entity, &ctx.types, 160.0);
        ui.end_row();
        ui.label("Event");
        changed |= event_ui(ui, &mut b.event, ctx, rule);
        ui.end_row();
    });

    heading(ui, "When", palette);
    if b.groups.is_empty() {
        hint(ui, "No condition: the rule is considered every time its event happens.", palette);
    }
    let mut remove_group = None;
    let groups = b.groups.len();
    for (g, group) in b.groups.iter_mut().enumerate() {
        if g > 0 {
            ui.label(RichText::new("OR").strong().color(palette.accent));
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if group.rows.len() > 1 { "all of:" } else { "" }).small().color(palette.text_muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("× group").clicked() {
                        remove_group = Some(g);
                    }
                });
            });
            changed |= group_ui(ui, group, ctx, (rule, g));
        });
    }
    if let Some(g) = remove_group {
        b.groups.remove(g);
        changed = true;
    }
    ui.horizontal(|ui| {
        let label = if groups == 0 { "+ Condition" } else { "+ OR group" };
        if ui.button(label).clicked() {
            b.groups.push(CondGroup { rows: vec![default_condition()] });
            changed = true;
        }
    });

    heading(ui, "Then", palette);
    if matches!(b.event, EventBlock::Death) {
        hint(ui, "on_death rules may only use emit and spawn.", palette);
    }
    let mut remove_action = None;
    let mut move_action = None;
    let n_actions = b.actions.len();
    for (k, action) in b.actions.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                changed |= action_ui(ui, action, ctx, (rule, k));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.small_button("×").clicked() {
                        remove_action = Some(k);
                    }
                    if k + 1 < n_actions && ui.small_button("Down").clicked() {
                        move_action = Some((k, k + 1));
                    }
                    if k > 0 && ui.small_button("Up").clicked() {
                        move_action = Some((k, k - 1));
                    }
                });
            });
        });
    }
    if let Some(k) = remove_action {
        b.actions.remove(k);
        changed = true;
    }
    if let Some((a, c)) = move_action {
        b.actions.swap(a, c);
        changed = true;
    }
    ui.menu_button("+ Action", |ui| {
        for f in vocab::FUNCTIONS.iter().filter(|f| f.kind == FunctionKind::Action) {
            if ui.button(f.name).on_hover_text(f.help).clicked() {
                b.actions.push(ActionBlock::new_call(f.name));
                changed = true;
                ui.close();
            }
        }
        ui.separator();
        if ui.button("set property").clicked() {
            b.actions.push(ActionBlock::Set { path: "energy".into(), value: ArgValue::Number(0.0) });
            changed = true;
            ui.close();
        }
        if ui.button("add to property").clicked() {
            b.actions.push(ActionBlock::Add { path: "energy".into(), value: ArgValue::Number(1.0), subtract: false });
            changed = true;
            ui.close();
        }
        if ui.button("custom (DSL)").clicked() {
            b.actions.push(ActionBlock::Raw("stay()".into()));
            changed = true;
            ui.close();
        }
    });

    heading(ui, "Modifiers", palette);
    changed |= modifiers_ui(ui, b, ctx, rule);
    changed
}

fn default_condition() -> Condition {
    Condition::Compare {
        left: Operand::Property { who: Who::Implicit, name: "energy".into() },
        cmp: Cmp::Lt,
        right: Operand::Number(10.0),
    }
}

fn event_ui(ui: &mut Ui, event: &mut EventBlock, ctx: &Ctx, rule: usize) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(("cl_event", rule)).selected_text(event.label()).width(130.0).show_ui(ui, |ui| {
            let options = [
                EventBlock::Tick,
                EventBlock::Birth,
                EventBlock::Death,
                EventBlock::Collision(None),
                EventBlock::StateEnter(ctx.states.first().cloned().unwrap_or_default()),
            ];
            for option in options {
                let same = std::mem::discriminant(event) == std::mem::discriminant(&option);
                if ui.selectable_label(same, option.label()).clicked() && !same {
                    *event = option;
                    changed = true;
                }
            }
        });
        match event {
            EventBlock::Collision(with) => {
                ui.label("with");
                changed |= super::widgets::optional_name_combo(
                    ui,
                    ("cl_event_with", rule),
                    with,
                    "any type",
                    &ctx.types,
                    110.0,
                );
            }
            EventBlock::StateEnter(state) => {
                ui.label("state");
                changed |= name_combo(ui, ("cl_event_state", rule), state, &ctx.states, 110.0);
            }
            _ => {}
        }
    });
    changed
}

fn group_ui(ui: &mut Ui, group: &mut CondGroup, ctx: &Ctx, id: (usize, usize)) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (r, row) in group.rows.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            if r > 0 {
                ui.label(RichText::new("and").small());
            }
            changed |= condition_ui(ui, row, ctx, (id.0, id.1, r));
            if ui.small_button("×").clicked() {
                remove = Some(r);
            }
        });
    }
    if let Some(r) = remove {
        group.rows.remove(r);
        changed = true;
    }
    if ui.small_button("+ and").clicked() {
        group.rows.push(default_condition());
        changed = true;
    }
    changed
}

fn condition_ui(ui: &mut Ui, cond: &mut Condition, ctx: &Ctx, id: (usize, usize, usize)) -> bool {
    let mut changed = false;
    let kind = match cond {
        Condition::Compare { .. } => "compare",
        Condition::Chance(_) => "chance",
        Condition::Exists(_) => "exists",
        Condition::Expr(_) => "expression",
    };
    egui::ComboBox::from_id_salt(("cl_cond_kind", id)).selected_text(kind).width(84.0).show_ui(ui, |ui| {
        for (label, make) in [
            ("compare", default_condition as fn() -> Condition),
            ("chance", || Condition::Chance(0.5)),
            ("exists", || Condition::Exists(EntitySel::Nearest { kind: "any".into(), radius: None })),
            ("expression", || Condition::Expr("energy > 0".into())),
        ] {
            if ui.selectable_label(label == kind, label).clicked() && label != kind {
                *cond = make();
                changed = true;
            }
        }
    });
    match cond {
        Condition::Compare { left, cmp, right } => {
            changed |= operand_ui(ui, left, ctx, (id, 0));
            egui::ComboBox::from_id_salt(("cl_cmp", id)).selected_text(cmp.symbol()).width(44.0).show_ui(ui, |ui| {
                for c in Cmp::ALL {
                    changed |= ui.selectable_value(cmp, c, c.symbol()).changed();
                }
            });
            // `state ==` and `type ==` compare with names.
            let names_expected = matches!(left, Operand::Property { name, .. } if name == "state" || name == "type");
            if names_expected && !matches!(right, Operand::Name(_)) {
                *right = Operand::Name(String::new());
                changed = true;
            }
            if names_expected {
                let is_type = matches!(left, Operand::Property { name, .. } if name == "type");
                if let Operand::Name(n) = right {
                    let options = if is_type { &ctx.types } else { &ctx.states };
                    changed |= name_combo(ui, ("cl_cmp_name", id), n, options, 110.0);
                }
            } else {
                if matches!(right, Operand::Name(_)) {
                    *right = Operand::Number(0.0);
                    changed = true;
                }
                changed |= operand_ui(ui, right, ctx, (id, 1));
            }
        }
        Condition::Chance(p) => {
            changed |= ui.add(crate::editor::widgets::dv(p).range(0.0..=1.0).speed(0.005).max_decimals(4)).changed();
        }
        Condition::Exists(sel) => {
            changed |= entity_sel_ui(ui, sel, ctx, (id, 9));
        }
        Condition::Expr(text) => {
            changed |= ui.add(egui::TextEdit::singleline(text).code_editor().desired_width(220.0)).changed();
        }
    }
    changed
}

type OperandId = ((usize, usize, usize), u8);

fn operand_ui(ui: &mut Ui, op: &mut Operand, ctx: &Ctx, id: OperandId) -> bool {
    let mut changed = false;
    let kind = op.kind_label();
    egui::ComboBox::from_id_salt(("cl_op_kind", id)).selected_text(kind).width(84.0).show_ui(ui, |ui| {
        let options: [(&str, Operand); 8] = [
            ("property", Operand::Property { who: Who::Implicit, name: "energy".into() }),
            ("nearby", Operand::Nearby { kind: ctx.kinds.first().cloned().unwrap_or_default(), radius: None }),
            ("distance", Operand::Distance(EntitySel::Nearest { kind: "any".into(), radius: None })),
            ("field", Operand::Field(ctx.fields.first().cloned().unwrap_or_default())),
            ("population", Operand::Population(ctx.kinds.first().cloned().unwrap_or_default())),
            ("random", Operand::Random),
            ("number", Operand::Number(0.0)),
            ("expression", Operand::Expr("energy * 2".into())),
        ];
        for (label, value) in options {
            if ui.selectable_label(label == kind, label).clicked() && label != kind {
                *op = value;
                changed = true;
            }
        }
    });
    match op {
        Operand::Property { who, name } => {
            egui::ComboBox::from_id_salt(("cl_op_who", id))
                .selected_text(match who {
                    Who::Implicit => "own",
                    Who::Me => "self.",
                    Who::Other => "other.",
                })
                .width(54.0)
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(who, Who::Implicit, "own")
                        .on_hover_text("the acting entity (bare name)")
                        .changed();
                    changed |= ui.selectable_value(who, Who::Me, "self.").changed();
                    changed |= ui.selectable_value(who, Who::Other, "other.").changed();
                });
            let options = if *who == Who::Other { &ctx.other_props } else { &ctx.props };
            changed |= name_combo(ui, ("cl_op_prop", id), name, options, 100.0);
        }
        Operand::Nearby { kind, radius } => {
            changed |= name_combo(ui, ("cl_op_nearby", id), kind, &ctx.kinds, 100.0);
            changed |= radius_ui(ui, radius, id);
        }
        Operand::Distance(sel) => {
            ui.label("to");
            changed |= entity_sel_ui(ui, sel, ctx, (id.0, id.1 + 10));
        }
        Operand::Field(name) => {
            changed |= name_combo(ui, ("cl_op_field", id), name, &ctx.fields, 100.0);
        }
        Operand::Population(kind) => {
            changed |= name_combo(ui, ("cl_op_pop", id), kind, &ctx.kinds, 100.0);
        }
        Operand::Random => {
            ui.label(RichText::new("U[0, 1)").small());
        }
        Operand::Number(n) => {
            changed |= ui.add(crate::editor::widgets::dv(n).speed(0.1).max_decimals(4)).changed();
        }
        Operand::Name(n) => {
            changed |= ui.add(egui::TextEdit::singleline(n).desired_width(90.0)).changed();
        }
        Operand::Expr(text) => {
            changed |= ui.add(egui::TextEdit::singleline(text).code_editor().desired_width(160.0)).changed();
        }
    }
    changed
}

fn radius_ui(ui: &mut Ui, radius: &mut Option<f64>, id: impl std::hash::Hash + std::fmt::Debug) -> bool {
    let mut changed = false;
    let mut on = radius.is_some();
    if ui.checkbox(&mut on, "radius").on_hover_text("Default: the type's perception radius").changed() {
        *radius = on.then_some(5.0);
        changed = true;
    }
    if let Some(r) = radius {
        changed |= ui.add(crate::editor::widgets::dv(r).range(0.1..=32.0).speed(0.05).max_decimals(2)).changed();
    }
    let _ = egui::Id::new(id);
    changed
}

fn entity_sel_ui(
    ui: &mut Ui,
    sel: &mut EntitySel,
    ctx: &Ctx,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
) -> bool {
    let mut changed = false;
    let label = sel.label();
    egui::ComboBox::from_id_salt(("cl_sel", id)).selected_text(&label).width(80.0).show_ui(ui, |ui| {
        let options = [
            ("self", EntitySel::Me),
            ("other", EntitySel::Other),
            ("nearest", EntitySel::Nearest { kind: ctx.kinds.first().cloned().unwrap_or_default(), radius: None }),
            ("expression", EntitySel::Expr("nearest(any)".into())),
        ];
        for (l, value) in options {
            if ui.selectable_label(l == label, l).clicked() && l != label {
                *sel = value;
                changed = true;
            }
        }
    });
    match sel {
        EntitySel::Nearest { kind, radius } => {
            changed |= name_combo(ui, ("cl_sel_kind", id), kind, &ctx.kinds, 100.0);
            changed |= radius_ui(ui, radius, ("cl_sel_r", id));
        }
        EntitySel::Expr(text) => {
            changed |= ui.add(egui::TextEdit::singleline(text).code_editor().desired_width(150.0)).changed();
        }
        EntitySel::Me | EntitySel::Other => {}
    }
    changed
}

fn action_ui(ui: &mut Ui, action: &mut ActionBlock, ctx: &Ctx, id: (usize, usize)) -> bool {
    let mut changed = false;
    let label = action.label();
    egui::ComboBox::from_id_salt(("cl_action", id)).selected_text(&label).width(110.0).show_ui(ui, |ui| {
        for f in vocab::FUNCTIONS.iter().filter(|f| f.kind == FunctionKind::Action) {
            if ui.selectable_label(label == f.name, f.name).on_hover_text(f.help).clicked() && label != f.name {
                *action = ActionBlock::new_call(f.name);
                changed = true;
            }
        }
        ui.separator();
        for (l, value) in [
            ("set", ActionBlock::Set { path: "energy".into(), value: ArgValue::Number(0.0) }),
            ("add", ActionBlock::Add { path: "energy".into(), value: ArgValue::Number(1.0), subtract: false }),
            ("subtract", ActionBlock::Add { path: "energy".into(), value: ArgValue::Number(1.0), subtract: true }),
            ("custom", ActionBlock::Raw("stay()".into())),
        ] {
            if ui.selectable_label(l == label, l).clicked() && l != label {
                *action = value;
                changed = true;
            }
        }
    });
    ui.vertical(|ui| match action {
        ActionBlock::Call { name, args } => {
            let Some(f) = vocab::function(name) else { return };
            if !f.help.is_empty() {
                ui.label(RichText::new(f.help).small().weak());
            }
            if args.len() < f.params.len() {
                args.resize(f.params.len(), None);
            }
            for (k, (slot, sig)) in args.iter_mut().zip(f.params).enumerate() {
                ui.horizontal_wrapped(|ui| {
                    changed |= arg_ui(ui, slot, sig, ctx, (id.0, id.1, k));
                });
            }
        }
        ActionBlock::Set { path, value } | ActionBlock::Add { path, value, .. } => {
            ui.horizontal(|ui| {
                changed |= name_combo(ui, ("cl_act_path", id), path, &ctx.writable, 110.0);
                ui.label(if label == "set" {
                    "="
                } else if label == "add" {
                    "+="
                } else {
                    "-="
                });
                changed |= value_ui(ui, value, ValueKind::Number, ctx, (id.0, id.1, 99));
            });
        }
        ActionBlock::Raw(text) => {
            changed |= ui.add(egui::TextEdit::singleline(text).code_editor().desired_width(220.0)).changed();
        }
    });
    changed
}

fn arg_ui(ui: &mut Ui, slot: &mut Option<ArgValue>, sig: &ParamSig, ctx: &Ctx, id: (usize, usize, usize)) -> bool {
    let mut changed = false;
    if sig.required {
        ui.label(RichText::new(sig.name).small().strong());
    } else {
        let mut on = slot.is_some();
        if ui.checkbox(&mut on, RichText::new(sig.name).small()).on_hover_text(sig.help).changed() {
            *slot = on.then(|| ArgValue::default_for(sig.kind));
            changed = true;
        }
    }
    match slot {
        Some(value) => changed |= value_ui(ui, value, sig.kind, ctx, id),
        None if sig.required => {
            *slot = Some(ArgValue::default_for(sig.kind));
            changed = true;
        }
        None => {
            ui.label(RichText::new(sig.help).small().weak());
        }
    }
    changed
}

fn value_ui(ui: &mut Ui, value: &mut ArgValue, kind: ValueKind, ctx: &Ctx, id: (usize, usize, usize)) -> bool {
    let mut changed = false;
    match value {
        ArgValue::Number(n) => {
            let range = match kind {
                ValueKind::Radius => 0.1..=32.0,
                ValueKind::Probability => 0.0..=1.0,
                _ => -1.0e9..=1.0e9,
            };
            changed |= ui.add(crate::editor::widgets::dv(n).range(range).speed(0.05).max_decimals(4)).changed();
            if ui.small_button("ƒx").on_hover_text("Use an expression").clicked() {
                *value = ArgValue::Expr(rule_engine::dsl::printer::print_number(*n));
                changed = true;
            }
        }
        ArgValue::Name(n) => {
            let options = match kind {
                ValueKind::Kind => &ctx.kinds,
                ValueKind::EntityType => &ctx.types,
                ValueKind::Resource => &ctx.resources,
                ValueKind::Field => &ctx.fields,
                ValueKind::State => &ctx.states,
                _ => &ctx.types,
            };
            changed |= name_combo(ui, ("cl_arg_name", id), n, options, 110.0);
        }
        ArgValue::Entity(sel) => {
            changed |= entity_sel_ui(ui, sel, ctx, ("cl_arg_ent", id));
        }
        ArgValue::Expr(text) => {
            changed |= ui.add(egui::TextEdit::singleline(text).code_editor().desired_width(150.0)).changed();
            let numeric = matches!(kind, ValueKind::Number | ValueKind::Radius | ValueKind::Probability);
            if numeric
                && let Ok(n) = text.trim().parse::<f64>()
                && ui.small_button("123").on_hover_text("Use a number").clicked()
            {
                *value = ArgValue::Number(n);
                changed = true;
            }
        }
    }
    changed
}

fn modifiers_ui(ui: &mut Ui, b: &mut RuleBlocks, ctx: &Ctx, rule: usize) -> bool {
    let mut changed = false;
    egui::Grid::new(("cl_mods", rule)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Priority");
        changed |= ui
            .add(crate::editor::widgets::dv(&mut b.priority).range(-1_000_000..=1_000_000).speed(1.0))
            .on_hover_text("Higher first; ties keep declaration order. Relationship rules use 100–910.")
            .changed();
        ui.end_row();
        ui.label("Probability");
        ui.horizontal(|ui| {
            let mut on = b.probability.is_some();
            if ui.checkbox(&mut on, "").changed() {
                b.probability = on.then_some(0.5);
                changed = true;
            }
            if let Some(p) = &mut b.probability {
                changed |= ui
                    .add(egui::Slider::new(p, 0.0..=1.0).max_decimals(3).clamping(egui::SliderClamping::Edits))
                    .changed();
            } else {
                ui.label(RichText::new("always fires when eligible").small().weak());
            }
        });
        ui.end_row();
        ui.label("Cooldown");
        ui.horizontal(|ui| {
            let mut on = b.cooldown.is_some();
            if ui.checkbox(&mut on, "").changed() {
                b.cooldown = on.then_some(10);
                changed = true;
            }
            if let Some(c) = &mut b.cooldown {
                changed |= ui.add(crate::editor::widgets::dv(c).range(0..=10_000_000).suffix(" ticks")).changed();
            }
        });
        ui.end_row();
        ui.label("Once");
        changed |= ui.checkbox(&mut b.once, "at most once per lifetime").changed();
        ui.end_row();
    });
    ui.label(RichText::new("Costs (charged on firing; the rule needs them to be affordable)").small());
    let mut remove = None;
    for (k, (path, amount)) in b.costs.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label("cost");
            changed |= name_combo(ui, ("cl_cost", rule, k), path, &ctx.writable, 100.0);
            changed |= ui.add(crate::editor::widgets::dv(amount).range(0.0..=1.0e6).speed(0.05)).changed();
            if ui.small_button("×").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        b.costs.remove(k);
        changed = true;
    }
    if ui.small_button("+ cost").clicked() {
        b.costs.push(("energy".into(), 1.0));
        changed = true;
    }
    ui.label(RichText::new("Products (a state entered, entities spawned or a resource released)").small());
    let mut remove = None;
    for (k, (name, amount)) in b.produces.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label("produces");
            changed |= name_combo(ui, ("cl_prod", rule, k), name, &ctx.products, 110.0);
            changed |= ui.add(crate::editor::widgets::dv(amount).range(0.0..=100.0).speed(0.05)).changed();
            if ui.small_button("×").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        b.produces.remove(k);
        changed = true;
    }
    if ui.small_button("+ product").clicked() {
        b.produces.push((ctx.products.first().cloned().unwrap_or_default(), 1.0));
        changed = true;
    }
    changed
}

fn vocabulary(ui: &mut Ui, palette: &Palette) {
    egui::CollapsingHeader::new("Rule language reference").id_salt("cl_vocab").show(ui, |ui| {
        hint(ui, "rule NAME for TYPE: [event] [when CONDITION] then ACTION, ... [modifiers]", palette);
        ui.label(RichText::new("Sensors").strong());
        for f in vocab::FUNCTIONS.iter().filter(|f| f.kind == FunctionKind::Sensor) {
            signature_row(ui, f, palette);
        }
        ui.label(RichText::new("Actions").strong());
        for f in vocab::FUNCTIONS.iter().filter(|f| f.kind == FunctionKind::Action) {
            signature_row(ui, f, palette);
        }
        ui.label(RichText::new("Events").strong());
        for e in vocab::EVENTS {
            ui.label(RichText::new(format!("{} — {}", e.syntax, e.help)).small());
        }
        ui.label(RichText::new("Modifiers").strong());
        for m in vocab::MODIFIERS {
            ui.label(RichText::new(format!("{} — {}", m.syntax, m.help)).small());
        }
        ui.label(RichText::new("Built-in properties").strong());
        for p in vocab::BUILTIN_PROPERTIES {
            let rw = if p.writable { "read/write" } else { "read-only" };
            ui.label(RichText::new(format!("{} ({rw}) — {}", p.name, p.help)).small());
        }
    });
}

/// `name(a, b=…)` signature text of a function.
pub fn signature(f: &vocab::FunctionSig) -> String {
    let params: Vec<String> =
        f.params.iter().map(|p| if p.required { p.name.to_string() } else { format!("{}=…", p.name) }).collect();
    format!("{}({})", f.name, params.join(", "))
}

fn signature_row(ui: &mut Ui, f: &vocab::FunctionSig, palette: &Palette) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(signature(f)).monospace().color(palette.accent));
        ui.label(RichText::new(f.help).small());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::dsl::parse_rule;
    use rule_engine::scenarios;

    fn system() -> rule_engine::def::LifeSystem {
        scenarios::builtin_by_id("producers-consumers").unwrap().system
    }

    #[test]
    fn text_edits_keep_the_last_parsed_rule_and_report_positions() {
        let sys = system();
        let mut ed = RuleEditor::default();
        ed.sync(&sys.rules, Some(0), &sys);
        assert!(ed.text.starts_with("rule spread for plant:"));
        assert!(ed.blocks.is_some());
        ed.text = "rule spread for plant:\n    when energy > then die()".into();
        assert!(ed.take_text(&sys).is_none(), "syntax error: nothing to commit");
        let e = ed.text_diags.iter().find(|d| d.code == "E015").expect("a syntax error");
        assert_eq!(e.span.map(|s| s.line), Some(2));
        ed.text = "rule spread for plant: when enrgy > 3 then die()".into();
        let parsed = ed.take_text(&sys).expect("parses despite the unknown property");
        assert_eq!(parsed.name, "spread");
        assert!(ed.text_diags.iter().any(|d| d.code == "E001" && d.help.as_deref() == Some("did you mean `energy`?")));
        // The buffers now hold the committed rule: no reload overwrites the text.
        let rules = vec![parsed];
        let typed = ed.text.clone();
        ed.sync(&rules, Some(0), &sys);
        assert_eq!(ed.text, typed);
    }

    #[test]
    fn block_edits_regenerate_canonical_text() {
        let sys = system();
        let mut ed = RuleEditor::default();
        ed.sync(&sys.rules, Some(0), &sys);
        let blocks = ed.blocks.as_mut().unwrap();
        blocks.priority = 310;
        blocks.cooldown = Some(4);
        let rule = ed.take_blocks(&sys).expect("valid blocks");
        assert!(ed.text.contains("priority 310 cooldown 4"), "{}", ed.text);
        assert_eq!(parse_rule(&ed.text).unwrap(), rule);
        assert!(ed.text_diags.is_empty());
        ed.blocks.as_mut().unwrap().actions.clear();
        assert!(ed.take_blocks(&sys).is_none());
        assert!(ed.block_error.is_some());
    }

    #[test]
    fn context_lists_names_of_the_rule_entity() {
        let sys = scenarios::builtin_by_id("robot-swarm").unwrap().system;
        let ctx = Ctx::new(&sys, "robot", Some("robot"));
        assert_eq!(ctx.states, vec!["moving".to_string(), "waiting".to_string()]);
        assert!(ctx.props.contains(&"state_age".to_string()) && ctx.props.contains(&"state".to_string()));
        assert!(!ctx.writable.contains(&"age".to_string()), "age is read-only");
        assert!(ctx.kinds.ends_with(&["same_species".to_string(), "any".to_string()]));
        assert_eq!(ctx.fields, vec!["beacon".to_string()]);
        let eat = vocab::function("eat").unwrap();
        assert_eq!(signature(eat), "eat(target, efficiency=…)");
    }
}
