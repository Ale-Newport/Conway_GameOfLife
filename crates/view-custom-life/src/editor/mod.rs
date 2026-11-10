//! The system editor: a draft [`LifeSystem`] edited in a window with tabs
//! (entity types, rules, relationships, world, scripts, files), validated
//! live, and applied to the running simulation with the model's own edit
//! classification (next tick or next reset).

pub mod entities;
pub mod files_tab;
pub mod relations;
pub mod rules;
pub mod scripts;
pub mod widgets;
pub mod world_tab;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use interface::{DataLocation, MetricsHistory, ModeRegistry, ModeScenarios, Palette, ThemeSettings, Toasts};
use rule_engine::compile::ir::RuleOrigin;
use rule_engine::def::LifeSystem;
use rule_engine::dsl::RuleAst;
use rule_engine::sim::params::{apply_param, current_params};
use rule_engine::{Diagnostic, EditReport, ItemRef, RelationshipGraph, RuleEngineError, classify_edit, compile};
use simulation_core::{ApplyPolicy, Control, ParamValue};

use crate::MODE;
use crate::draft::{SyncStatus, sync_status};
use crate::graph::{ForceLayout, graph_of};
use crate::session::{ClData, ClRunner, ClScenarios, SimInfo};
use crate::state::ClUi;
use widgets::counts;

/// Tabs of the editor window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EditorTab {
    /// Entity types.
    #[default]
    Entities,
    /// DSL rules (visual builder and text).
    Rules,
    /// Relationships and their graph.
    Relationships,
    /// World, fields, events, end conditions.
    World,
    /// Rhai scripts and sandbox limits.
    Scripts,
    /// Presets, library, export and import.
    Files,
}

impl EditorTab {
    /// Every tab in order.
    pub const ALL: [EditorTab; 6] = [
        EditorTab::Entities,
        EditorTab::Rules,
        EditorTab::Relationships,
        EditorTab::World,
        EditorTab::Scripts,
        EditorTab::Files,
    ];

    /// Label.
    pub fn label(self) -> &'static str {
        match self {
            EditorTab::Entities => "Entities",
            EditorTab::Rules => "Rules",
            EditorTab::Relationships => "Relationships",
            EditorTab::World => "World",
            EditorTab::Scripts => "Scripts",
            EditorTab::Files => "Files",
        }
    }

    /// Tab named `name` (case-insensitive label).
    pub fn from_name(name: &str) -> Option<EditorTab> {
        EditorTab::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(name.trim()))
    }

    /// Which tab shows a diagnostic's item.
    pub fn of_item(item: Option<ItemRef>) -> EditorTab {
        match item {
            Some(ItemRef::EntityType(_)) => EditorTab::Entities,
            Some(ItemRef::Rule(_)) => EditorTab::Rules,
            Some(ItemRef::Relationship(_)) => EditorTab::Relationships,
            Some(ItemRef::Script(_) | ItemRef::ScriptLimits) => EditorTab::Scripts,
            Some(
                ItemRef::World
                | ItemRef::Resource(_)
                | ItemRef::Environment(_)
                | ItemRef::RandomEvent(_)
                | ItemRef::EndCondition(_)
                | ItemRef::System,
            )
            | None => EditorTab::World,
        }
    }
}

/// A rule of the compiled draft (manual or generated), for listings.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleRow {
    /// Entity type.
    pub entity: String,
    /// Rule name.
    pub name: String,
    /// Canonical text.
    pub text: String,
    /// Where it comes from.
    pub origin: RuleOrigin,
    /// Its AST.
    pub ast: RuleAst,
}

/// The draft and the editor's state.
#[derive(Resource)]
pub struct ClEditor {
    /// The system being edited.
    pub draft: LifeSystem,
    /// Seed the draft is applied with.
    pub seed: u64,
    /// Where the draft came from (preset, file...).
    pub source: String,
    /// Revision, incremented by every edit.
    pub rev: u64,
    /// Whether the window is open.
    pub open: bool,
    /// Selected tab.
    pub tab: EditorTab,
    /// Every diagnostic of the draft (errors and warnings).
    pub diagnostics: Vec<Diagnostic>,
    /// Rules of the compiled draft (empty while it has errors).
    pub compiled_rules: Vec<RuleRow>,
    /// Relationship graph of the draft.
    pub graph: RelationshipGraph,
    /// Force layout of the graph.
    pub layout: ForceLayout,
    /// Compact layout used by the properties panel.
    pub small_layout: ForceLayout,
    checked_rev: Option<u64>,
    /// Selected entity type.
    pub sel_type: usize,
    /// Selected manual rule.
    pub sel_rule: Option<usize>,
    /// Selected generated rule (index into `compiled_rules`).
    pub sel_generated: Option<usize>,
    /// Selected relationship.
    pub sel_rel: Option<usize>,
    /// Selected script.
    pub sel_script: Option<usize>,
    /// Rule editor state.
    pub rules: rules::RuleEditor,
    /// Files tab state.
    pub files: files_tab::FilesState,
    /// Entity type a new rule is created for.
    pub new_rule_entity: String,
    /// Apply state for (draft revision, definitions revision).
    apply_cache: Option<(u64, u64, ApplyState)>,
}

impl Default for ClEditor {
    fn default() -> Self {
        Self {
            draft: crate::draft::blank_system("New system"),
            seed: 1,
            source: "New system".into(),
            rev: 0,
            open: false,
            tab: EditorTab::Entities,
            diagnostics: Vec::new(),
            compiled_rules: Vec::new(),
            graph: RelationshipGraph::default(),
            layout: ForceLayout::default(),
            small_layout: ForceLayout::default(),
            checked_rev: None,
            sel_type: 0,
            sel_rule: None,
            sel_generated: None,
            sel_rel: None,
            sel_script: None,
            rules: rules::RuleEditor::default(),
            files: files_tab::FilesState::default(),
            new_rule_entity: String::new(),
            apply_cache: None,
        }
    }
}

impl ClEditor {
    /// Replaces the draft (preset, file, tutorial).
    pub fn load(&mut self, system: LifeSystem, seed: u64, source: impl Into<String>) {
        self.draft = system;
        self.seed = seed;
        self.source = source.into();
        self.sel_type = 0;
        self.sel_rule = if self.draft.rules.is_empty() { None } else { Some(0) };
        self.sel_generated = None;
        self.sel_rel = if self.draft.relationships.is_empty() { None } else { Some(0) };
        self.sel_script = if self.draft.scripts.is_empty() { None } else { Some(0) };
        self.rules = rules::RuleEditor::default();
        self.touch();
        self.refresh();
    }

    /// Keeps the draft in step when the system of the next reset changed
    /// outside the editor (Parameters tab, undo / redo, a loaded save):
    /// a draft that matched the old system follows it entirely; a draft
    /// with its own edits takes every changed parameter whose draft value
    /// still equals the old one (a three-way merge per parameter key), so
    /// the next Apply does not silently revert a parameter change.
    pub fn follow_target(&mut self, old: &LifeSystem, old_seed: u64, new: &LifeSystem, new_seed: u64) {
        if self.draft == *old && self.seed == old_seed {
            if self.draft != *new || self.seed != new_seed {
                self.draft = new.clone();
                self.seed = new_seed;
                self.touch();
            }
            return;
        }
        if self.seed == old_seed && new_seed != old_seed {
            self.seed = new_seed;
            self.touch();
        }
        let before = current_params(old, old_seed, true);
        let after = current_params(new, new_seed, true);
        let mine = current_params(&self.draft, self.seed, true);
        let mut changed = false;
        for (key, value) in after.iter() {
            if matches!(key, "seed" | "pause_on_end") || before.get(key) == Some(value) {
                continue;
            }
            if mine.get(key).is_some() && mine.get(key) == before.get(key) {
                changed |= apply_param(&mut self.draft, key, value).is_ok();
            }
        }
        if changed {
            self.touch();
        }
    }

    /// Marks the draft as edited.
    pub fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    /// Re-validates the draft if it changed: diagnostics, compiled rules, graph.
    pub fn refresh(&mut self) {
        if self.checked_rev == Some(self.rev) {
            return;
        }
        self.checked_rev = Some(self.rev);
        match compile(&self.draft) {
            Ok(compiled) => {
                self.diagnostics = compiled.warnings().to_vec();
                self.compiled_rules = compiled
                    .rules()
                    .iter()
                    .map(|r| RuleRow {
                        entity: compiled.types()[usize::from(r.entity)].def.name.clone(),
                        name: r.name.clone(),
                        text: r.text.clone(),
                        origin: r.origin.clone(),
                        ast: r.ast.clone(),
                    })
                    .collect();
                self.graph = compiled.graph().clone();
            }
            Err(RuleEngineError::Invalid(diags)) => {
                self.diagnostics = diags;
                self.compiled_rules.clear();
                self.graph = graph_of(&self.draft);
            }
            Err(other) => {
                self.diagnostics = vec![Diagnostic::error("E020", other.to_string())];
                self.compiled_rules.clear();
                self.graph = graph_of(&self.draft);
            }
        }
        if self.sel_type >= self.draft.entity_types.len() {
            self.sel_type = self.draft.entity_types.len().saturating_sub(1);
        }
        let clamp = |sel: &mut Option<usize>, len: usize| {
            if sel.is_some_and(|i| i >= len) {
                *sel = len.checked_sub(1);
            }
        };
        clamp(&mut self.sel_rule, self.draft.rules.len());
        clamp(&mut self.sel_rel, self.draft.relationships.len());
        clamp(&mut self.sel_script, self.draft.scripts.len());
        clamp(&mut self.sel_generated, self.compiled_rules.len());
    }

    /// What the apply bar shows, recomputed only when the draft or the
    /// definitions read from the simulation (`info_rev`) changed: the
    /// comparison and classification walk both systems, which is wasted
    /// work in every frame of a panel.
    pub fn apply_state(&mut self, info: &SimInfo, info_rev: u64) -> ApplyState {
        self.refresh();
        match &self.apply_cache {
            Some((rev, irev, state)) if *rev == self.rev && *irev == info_rev => state.clone(),
            _ => {
                let state = ApplyState::new(self, info);
                self.apply_cache = Some((self.rev, info_rev, state.clone()));
                state
            }
        }
    }

    /// Whether the draft has errors.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// Diagnostics belonging to one item.
    pub fn diagnostics_of(&self, item: ItemRef) -> Vec<Diagnostic> {
        self.diagnostics.iter().filter(|d| d.item == Some(item)).cloned().collect()
    }

    /// Whether the draft differs from what the next reset would start.
    pub fn is_edited(&self, info: Option<&SimInfo>) -> bool {
        info.is_some_and(|i| self.draft != i.target || self.seed != i.seed)
    }

    /// Opens the window at `tab`.
    pub fn open_at(&mut self, tab: EditorTab) {
        self.open = true;
        self.tab = tab;
    }

    /// Names in the shared namespace (types, resources, environment fields).
    pub fn namespace(&self) -> Vec<String> {
        crate::draft::namespace(&self.draft).into_iter().map(str::to_string).collect()
    }

    /// Entity type names.
    pub fn type_names(&self) -> Vec<String> {
        self.draft.entity_types.iter().map(|t| t.name.clone()).collect()
    }

    /// Resource names.
    pub fn resource_names(&self) -> Vec<String> {
        self.draft.world.resources.iter().map(|r| r.name.clone()).collect()
    }

    /// Resource and environment field names.
    pub fn field_names(&self) -> Vec<String> {
        self.draft
            .world
            .resources
            .iter()
            .map(|r| r.name.clone())
            .chain(self.draft.world.environment.iter().map(|e| e.name.clone()))
            .collect()
    }

    /// Whether `word` is a name defined by the draft (for highlighting).
    pub fn is_name(&self, word: &str) -> bool {
        self.draft.entity_types.iter().any(|t| t.name == word || t.states.iter().any(|s| s == word))
            || self.draft.world.resources.iter().any(|r| r.name == word)
            || self.draft.world.environment.iter().any(|e| e.name == word)
            || self.draft.entity_types.iter().any(|t| t.properties.iter().any(|p| p.name == word))
    }
}

/// What the apply bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyState {
    /// Draft vs running vs reset target.
    pub sync: SyncStatus,
    /// What applying the draft would do (relative to the running system).
    pub report: EditReport,
    /// Whether the seed of the next reset would change.
    pub seed_changed: bool,
    /// Errors and warnings of the draft.
    pub errors: usize,
    /// Warnings of the draft.
    pub warnings: usize,
}

impl ApplyState {
    /// Computes the state from the definitions read from the simulation.
    pub fn new(editor: &ClEditor, info: &SimInfo) -> Self {
        let (errors, warnings) = counts(&editor.diagnostics);
        Self {
            sync: sync_status(&info.running, &info.target, &editor.draft),
            report: classify_edit(&info.running, &editor.draft),
            seed_changed: editor.seed != info.seed,
            errors,
            warnings,
        }
    }

    /// Whether Apply does something.
    pub fn can_apply(&self) -> bool {
        self.errors == 0 && (self.sync == SyncStatus::Edited || self.seed_changed)
    }

    /// Whether "Apply & reset" is possible.
    pub fn can_apply_and_reset(&self) -> bool {
        self.errors == 0
    }

    /// Short status text.
    pub fn status_text(&self) -> &'static str {
        match (self.sync, self.seed_changed) {
            (SyncStatus::Edited, _) | (_, true) => "Draft has unapplied edits",
            (SyncStatus::PendingReset, false) => "Applied — waits for the next reset",
            (SyncStatus::InSync, false) => "Draft matches the running system",
        }
    }

    /// Text of the policy of applying now.
    pub fn policy_text(&self) -> &'static str {
        if self.seed_changed && self.report.policy != ApplyPolicy::RequiresReset && self.sync != SyncStatus::Edited {
            return "A new seed takes effect at the next reset.";
        }
        crate::draft::policy_text(self.report.policy)
    }
}

/// Sends the draft to the simulation (undoable), optionally followed by a reset.
pub fn apply_draft(runner: &ClRunner, editor: &ClEditor, info: &SimInfo, reset: bool, data: &mut ClData) {
    if editor.draft != info.target {
        runner.edit(rule_engine::CustomLifeCommand::ReplaceSystem(Box::new(editor.draft.clone())));
    }
    if editor.seed != info.seed {
        runner.send(Control::SetParam("seed".into(), ParamValue::Int((editor.seed & i64::MAX as u64) as i64)));
    }
    if reset {
        runner.send(Control::Reset);
    }
    data.refresh_info();
}

/// Everything the editor window needs.
#[derive(SystemParam)]
pub struct EditorParams<'w> {
    /// The draft.
    pub editor: ResMut<'w, ClEditor>,
    /// Simulation data.
    pub data: ResMut<'w, ClData>,
    /// The session.
    pub runner: Option<Res<'w, ClRunner>>,
    /// Display state.
    pub ui: ResMut<'w, ClUi>,
    /// Presets.
    pub presets: Res<'w, ClScenarios>,
    /// User data folders.
    pub location: Res<'w, DataLocation>,
    /// Messages.
    pub toasts: ResMut<'w, Toasts>,
    /// Metric plots (cleared when a new system starts).
    pub history: ResMut<'w, MetricsHistory>,
    /// Scenario picker.
    pub list: ResMut<'w, ModeScenarios>,
    /// Theme.
    pub theme: Res<'w, ThemeSettings>,
    /// Mode accents.
    pub registry: Res<'w, ModeRegistry>,
}

impl EditorParams<'_> {
    /// Palette of the mode.
    pub fn palette(&self) -> Palette {
        Palette::new(self.theme.kind, self.registry.accent(MODE))
    }
}

/// Validates the draft once per change (also while the window is closed,
/// so the panels show current problems).
pub fn refresh_draft(mut editor: ResMut<ClEditor>) {
    editor.refresh();
}

/// Draws the editor window.
pub fn editor_window(mut contexts: EguiContexts, mut p: EditorParams) {
    if !p.editor.open {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let ctx = ctx.clone();
    let palette = p.palette();
    let mut open = true;
    let screen = ctx.content_rect();
    let default_size =
        egui::vec2((screen.width() * 0.62).clamp(640.0, 1080.0), (screen.height() * 0.76).clamp(460.0, 860.0));
    egui::Window::new(egui::RichText::new(format!("⚙  System editor — {}", p.editor.draft.name)).strong())
        .id(egui::Id::new("custom_life_editor"))
        .open(&mut open)
        .default_size(default_size)
        .default_pos(screen.center() - default_size / 2.0)
        .min_size(egui::vec2(560.0, 380.0))
        .resizable(true)
        .collapsible(true)
        .show(&ctx, |ui| {
            egui::Panel::bottom("custom_life_editor_apply")
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 6)))
                .show(ui, |ui| {
                    ui.separator();
                    apply_bar(ui, &mut p, &palette, false);
                });
            tab_bar(ui, &mut p.editor, &palette);
            ui.separator();
            match p.editor.tab {
                EditorTab::Entities => entities::show(ui, &mut p, &palette),
                EditorTab::Rules => rules::show(ui, &mut p, &palette),
                EditorTab::Relationships => relations::show(ui, &mut p, &palette),
                EditorTab::World => world_tab::show(ui, &mut p, &palette),
                EditorTab::Scripts => scripts::show(ui, &mut p, &palette),
                EditorTab::Files => files_tab::show(ui, &mut p, &palette),
            }
            // Ctrl/Cmd+Enter applies from anywhere in the window.
            if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))
                && let (Some(runner), Some(info)) = (p.runner.as_deref(), p.data.info.clone())
            {
                p.editor.refresh();
                let state = ApplyState::new(&p.editor, &info);
                if state.can_apply() {
                    apply_draft(runner, &p.editor, &info, false, &mut p.data);
                    p.toasts.success(format!("Draft applied. {}", state.policy_text()));
                }
            }
        });
    if !open {
        p.editor.open = false;
    }
}

fn tab_bar(ui: &mut egui::Ui, editor: &mut ClEditor, palette: &Palette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for tab in EditorTab::ALL {
            let on = editor.tab == tab;
            let problems =
                editor.diagnostics.iter().filter(|d| d.is_error() && EditorTab::of_item(d.item) == tab).count();
            let mut text = egui::RichText::new(tab.label()).size(13.5);
            text = if on { text.color(palette.text) } else { text.color(palette.text_muted) };
            let response = ui.selectable_label(on, text);
            if problems > 0 {
                let c = response.rect.right_top() + egui::vec2(-4.0, 5.0);
                ui.painter().circle_filled(c, 3.5, palette.error);
            }
            if on {
                let r = response.rect;
                ui.painter().line_segment(
                    [egui::pos2(r.min.x + 6.0, r.max.y + 2.0), egui::pos2(r.max.x - 6.0, r.max.y + 2.0)],
                    egui::Stroke::new(2.0, palette.accent),
                );
            }
            if response.clicked() {
                editor.tab = tab;
            }
        }
    });
}

/// The apply bar: sync status, what applying would do (from the model's
/// classification), problem counts and the Apply / Apply & reset / Revert
/// buttons. `compact` is the narrow version for the properties panel.
pub fn apply_bar(ui: &mut egui::Ui, p: &mut EditorParams, palette: &Palette, compact: bool) {
    p.editor.refresh();
    let info_rev = p.data.info_rev;
    let Some(info) = p.data.info.as_ref() else {
        ui.label(egui::RichText::new("Reading the running system…").color(palette.text_muted));
        return;
    };
    let state = p.editor.apply_state(info, info_rev);
    let info_seed = info.seed;
    let (status_color, policy_color) = (
        match (state.sync, state.seed_changed) {
            (SyncStatus::InSync, false) => palette.success,
            (SyncStatus::PendingReset, false) => palette.warning,
            _ => palette.accent,
        },
        if state.report.policy == ApplyPolicy::RequiresReset { palette.warning } else { palette.success },
    );
    let edited = state.sync == SyncStatus::Edited || state.seed_changed;
    ui.horizontal_wrapped(|ui| {
        interface::widgets::badge(ui, state.status_text(), status_color);
        if edited {
            let label = if state.report.policy == ApplyPolicy::RequiresReset
                || (state.seed_changed && state.sync != SyncStatus::Edited)
            {
                "Requires reset"
            } else {
                "Applies next tick"
            };
            interface::widgets::badge(ui, label, policy_color).on_hover_text(state.report.notes.join("\n"));
        }
        if state.errors > 0 {
            interface::widgets::badge(ui, &format!("{} error(s)", state.errors), palette.error);
        } else if state.warnings > 0 {
            interface::widgets::badge(ui, &format!("{} warning(s)", state.warnings), palette.warning);
        }
    });
    if edited {
        let mut notes = state.report.notes.clone();
        if state.seed_changed {
            notes.push(format!("the seed changes from {info_seed} to {} at the next reset", p.editor.seed));
        }
        let shown = if compact { notes.iter().take(2).cloned().collect::<Vec<_>>() } else { notes };
        for note in shown {
            ui.label(egui::RichText::new(format!("• {note}")).small().color(palette.text_muted));
        }
    } else if state.sync == SyncStatus::PendingReset {
        ui.label(
            egui::RichText::new("Structural edits are stored; press Reset (R) to start the world with them.")
                .small()
                .color(palette.text_muted),
        );
    }
    if state.errors > 0 {
        ui.label(egui::RichText::new("Fix the errors to apply the draft.").small().color(palette.error));
    }
    let Some(runner) = p.runner.as_deref().cloned() else { return };
    ui.horizontal_wrapped(|ui| {
        let apply_label = if state.report.policy == ApplyPolicy::RequiresReset { "Apply (at reset)" } else { "Apply" };
        let apply = ui
            .add_enabled(
                state.can_apply(),
                egui::Button::new(egui::RichText::new(apply_label).strong()).fill(if state.can_apply() {
                    interface::theme::with_alpha(palette.accent, 0.35)
                } else {
                    palette.surface
                }),
            )
            .on_hover_text("Send the draft to the simulation (undoable, Ctrl/Cmd+Enter).");
        if apply.clicked()
            && let Some(info) = p.data.info.clone()
        {
            apply_draft(&runner, &p.editor, &info, false, &mut p.data);
            let msg = if state.report.policy == ApplyPolicy::RequiresReset {
                "Draft stored: it becomes the world at the next reset.".to_string()
            } else {
                format!("Draft applied. {}", state.policy_text())
            };
            p.toasts.success(msg);
        }
        let reset = ui
            .add_enabled(state.can_apply_and_reset(), egui::Button::new("Apply & reset"))
            .on_hover_text("Apply the draft and restart the world from its initial populations (undoable).");
        if reset.clicked()
            && let Some(info) = p.data.info.clone()
        {
            apply_draft(&runner, &p.editor, &info, true, &mut p.data);
            p.ui.clear_selection();
            p.toasts.success("Draft applied; the world restarted from it.");
        }
        let revert = ui
            .add_enabled(edited, egui::Button::new("Revert"))
            .on_hover_text("Discard unapplied edits: the draft becomes the system of the next reset.");
        if revert.clicked()
            && let Some(info) = p.data.info.as_ref()
        {
            let source = p.editor.source.clone();
            p.editor.load(info.target.clone(), info.seed, source);
        }
        if !compact && ui.button("Problems…").on_hover_text("Show every diagnostic of the draft.").clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("custom_life_problems_open"), true));
        }
    });
    if !compact {
        problems_window(ui.ctx(), &mut p.editor, palette);
    }
}

fn problems_window(ctx: &egui::Context, editor: &mut ClEditor, palette: &Palette) {
    let id = egui::Id::new("custom_life_problems_open");
    let mut open: bool = ctx.data(|d| d.get_temp(id)).unwrap_or(false);
    if !open {
        return;
    }
    let diags = editor.diagnostics.clone();
    egui::Window::new("Problems of the draft")
        .id(egui::Id::new("custom_life_problems"))
        .open(&mut open)
        .default_size(egui::vec2(560.0, 300.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if diags.is_empty() {
                    widgets::diagnostics_list(ui, &diags, palette, "No problems: the draft compiles.");
                }
                for d in &diags {
                    ui.horizontal(|ui| {
                        if ui.small_button("Show").on_hover_text("Open the item in the editor").clicked() {
                            focus_item(editor, d.item);
                        }
                        ui.vertical(|ui| widgets::diagnostic_row(ui, d, palette));
                    });
                    ui.separator();
                }
            });
        });
    ctx.data_mut(|d| d.insert_temp(id, open));
}

/// Opens the editor on the item a diagnostic belongs to.
pub fn focus_item(editor: &mut ClEditor, item: Option<ItemRef>) {
    editor.open_at(EditorTab::of_item(item));
    match item {
        Some(ItemRef::EntityType(i)) => editor.sel_type = usize::from(i),
        Some(ItemRef::Rule(i)) => {
            editor.sel_rule = Some(usize::from(i));
            editor.sel_generated = None;
        }
        Some(ItemRef::Relationship(i)) => editor.sel_rel = Some(usize::from(i)),
        Some(ItemRef::Script(i)) => editor.sel_script = Some(usize::from(i)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::scenarios;

    fn info(sys: &LifeSystem, seed: u64) -> SimInfo {
        SimInfo { running: sys.clone(), target: sys.clone(), seed }
    }

    #[test]
    fn apply_state_follows_the_model_classification() {
        let preset = scenarios::builtin_by_id("producers-consumers").unwrap();
        let mut editor = ClEditor::default();
        editor.load(preset.system.clone(), preset.seed, "test");
        let info = info(&preset.system, preset.seed);
        let state = ApplyState::new(&editor, &info);
        assert_eq!(state.sync, SyncStatus::InSync);
        assert!(!state.can_apply());
        assert!(state.can_apply_and_reset());

        editor.draft.entity_types[1].energy.metabolism = 0.07;
        editor.touch();
        editor.refresh();
        let state = ApplyState::new(&editor, &info);
        assert_eq!(state.sync, SyncStatus::Edited);
        assert_eq!(state.report.policy, ApplyPolicy::NextTick);
        assert!(state.can_apply());

        editor.draft.world.width = 64;
        editor.touch();
        editor.refresh();
        assert_eq!(ApplyState::new(&editor, &info).report.policy, ApplyPolicy::RequiresReset);

        // A seed change alone is an edit that waits for the reset.
        let mut editor = ClEditor::default();
        editor.load(preset.system.clone(), preset.seed + 1, "test");
        let state = ApplyState::new(&editor, &info);
        assert!(state.seed_changed && state.can_apply());
        assert!(state.policy_text().contains("next reset"));
    }

    #[test]
    fn errors_block_apply_and_point_to_their_tab() {
        let preset = scenarios::builtin_by_id("bacterial-growth").unwrap();
        let mut editor = ClEditor::default();
        editor.load(preset.system.clone(), preset.seed, "test");
        editor
            .draft
            .rules
            .push(rule_engine::dsl::parse_rule("rule bad for bacterium: when enrgy > 1 then die()").unwrap());
        editor.touch();
        editor.refresh();
        assert!(editor.has_errors());
        let state = ApplyState::new(&editor, &info(&preset.system, preset.seed));
        assert!(!state.can_apply() && !state.can_apply_and_reset());
        let item = editor.diagnostics.iter().find(|d| d.is_error()).unwrap().item;
        assert_eq!(EditorTab::of_item(item), EditorTab::Rules);
        assert_eq!(editor.diagnostics_of(ItemRef::Rule(2)).len(), 1);
        focus_item(&mut editor, item);
        assert_eq!((editor.tab, editor.sel_rule), (EditorTab::Rules, Some(2)));
        assert!(editor.compiled_rules.is_empty(), "no compiled rules while invalid");
        assert_eq!(editor.graph.nodes.len(), 2, "the graph is still built from the definition");
    }

    #[test]
    fn the_draft_follows_changes_made_outside_the_editor() {
        let preset = scenarios::builtin_by_id("producers-consumers").unwrap();
        let (sys, seed) = (preset.system.clone(), preset.seed);
        let mut editor = ClEditor::default();
        editor.load(sys.clone(), seed, "test");
        // In sync: the draft mirrors the new system (Parameters tab, undo).
        let mut changed = sys.clone();
        apply_param(&mut changed, "entity.plant.metabolism", &ParamValue::Float(0.05)).unwrap();
        editor.follow_target(&sys, seed, &changed, seed);
        assert_eq!(editor.draft, changed);
        // With edits of its own: changed parameters merge in, edits stay.
        editor.draft.entity_types[1].description = "my edit".into();
        let mut regrown = changed.clone();
        apply_param(&mut regrown, "resource.sunlight.regen", &ParamValue::Float(0.08)).unwrap();
        editor.follow_target(&changed, seed, &regrown, seed);
        assert_eq!(editor.draft.world.resources[0].regen, 0.08);
        assert_eq!(editor.draft.entity_types[1].description, "my edit");
        // A parameter the draft changed itself wins over the outside change.
        editor.draft.entity_types[0].energy.metabolism = 0.09;
        let mut other = regrown.clone();
        apply_param(&mut other, "entity.plant.metabolism", &ParamValue::Float(0.02)).unwrap();
        editor.follow_target(&regrown, seed, &other, seed + 1);
        assert_eq!(editor.draft.entity_types[0].energy.metabolism, 0.09);
        assert_eq!(editor.seed, seed + 1, "an unedited seed follows");
    }

    #[test]
    fn apply_state_is_cached_per_revision() {
        let preset = scenarios::builtin_by_id("bacterial-growth").unwrap();
        let mut editor = ClEditor::default();
        editor.load(preset.system.clone(), preset.seed, "test");
        let info = info(&preset.system, preset.seed);
        assert_eq!(editor.apply_state(&info, 1).sync, SyncStatus::InSync);
        editor.draft.entity_types[0].energy.metabolism = 0.5;
        editor.touch();
        assert_eq!(editor.apply_state(&info, 1).sync, SyncStatus::Edited, "a new draft revision recomputes");
        let mut applied = info.clone();
        applied.target = editor.draft.clone();
        applied.running = editor.draft.clone();
        assert_eq!(editor.apply_state(&applied, 2).sync, SyncStatus::InSync, "new definitions recompute");
    }

    #[test]
    fn tabs_have_names() {
        for t in EditorTab::ALL {
            assert_eq!(EditorTab::from_name(&t.label().to_lowercase()), Some(t));
        }
        assert_eq!(EditorTab::from_name("nope"), None);
    }
}
