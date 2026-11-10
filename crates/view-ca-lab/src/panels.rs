//! Panel contents: the Tools panel (model family, rule editor, tools,
//! view), the Properties tab (rule comparison, family details, dynamics,
//! cells in view, inspector) and the world overlay (legend, scale, time).

use std::collections::HashMap;

use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy_egui::egui::{self, RichText, Sense, Stroke, Ui, vec2};
use cellular_automata::ant::{Heading, Turn};
use cellular_automata::{RulePreset, Symmetry, WolframCommand};
use egui_plot::{AxisHints, GridMark, Line, Plot, PlotPoints};
use interface::format::{format_compact, format_count};
use interface::theme::with_alpha;
use interface::widgets::legend::gradient_bar_with_mid;
use interface::widgets::{badge, legend_row, swatch};
use interface::{ActiveSession, MetricsHistory, ModeRegistry, Palette, SessionStatus, ThemeSettings};
use patterns::{GenerationsRule, LifeRule};
use rendering::WorldInput;
use rendering::grid::VisibleCells;
use rendering::grid::style::{AGE_LEGEND_AGES, age_ramp_position};
use simulation_core::{Control, ParamSet, ParamValue, Topology};

use crate::family::{self, Family, Rgb, color32};
use crate::rules::{self, MeasuredClass, RuleCheck};
use crate::session::{Catalog, LabRunner, LoadRequests, LoadedScenario};
use crate::sim::{FamilyInfo, FillSettings, LabCommand, LabInfo, LabView};
use crate::tools::{Circuits, Hover, MAX_RADIUS, MODE_SHORTCUTS, Tool, ToolSettings};
use crate::world::{CameraFit, Colouring, LatestView, OverlayInset, ViewSettings, mode_palette, state_colors};

/// Key (publication counter, address of the view) and counts of a census.
type CachedCensus = ((u64, usize), Option<Vec<u64>>);

/// UI state of the panels.
#[derive(Resource, Default)]
pub struct PanelState {
    /// Rule typed in the rule editor.
    pub rule_draft: String,
    /// Rule the draft was last synchronised with (family and text).
    draft_source: Option<(Family, String)>,
    /// Measured classes of elementary rules (computed once per rule).
    classes: HashMap<u8, MeasuredClass>,
    /// Census of the last view (publication counter and address of the
    /// view, then the counts): counted once per published view, not on
    /// every frame.
    census: Option<CachedCensus>,
    /// Overlay folded to its header.
    pub overlay_folded: bool,
}

impl PanelState {
    /// Keeps the draft equal to the effective rule unless the user is editing it.
    fn sync_draft(&mut self, family: Family, effective: &str, editing: bool) {
        let source = Some((family, effective.to_string()));
        if self.draft_source != source && !editing {
            self.rule_draft = effective.to_string();
            self.draft_source = source;
        }
    }

    /// Measured class of an elementary rule (cached).
    pub fn class_of(&mut self, rule: u8) -> MeasuredClass {
        *self.classes.entry(rule).or_insert_with(|| rules::measure_class(rule))
    }

    /// [`census`] of the view published as number `seq` (cached).
    pub fn census_of(&mut self, seq: u64, view: &std::sync::Arc<LabView>) -> Option<Vec<u64>> {
        let key = (seq, std::sync::Arc::as_ptr(view) as usize);
        match &self.census {
            Some((cached, counts)) if *cached == key => counts.clone(),
            _ => {
                let counts = census(view);
                self.census = Some((key, counts.clone()));
                counts
            }
        }
    }
}

/// Keeps the tool settings valid for the running model and passes the
/// random-fill settings to the model (they drive the shell's Randomize).
pub(crate) fn fit_tool_settings(
    latest: Res<LatestView>,
    runner: Option<Res<LabRunner>>,
    mut tools: ResMut<ToolSettings>,
) {
    // While the view still shows the model being replaced, fitting would
    // undo a tool chosen for the new one (a tutorial's "load, then pick a tool").
    let Some(info) = latest.info().filter(|_| !latest.is_stale()) else { return };
    let mut t = tools.clone();
    t.fit(info.family(), info.states());
    tools.set_if_neq(t);
    if tools.is_changed()
        && let Some(runner) = runner
    {
        let fill = FillSettings { density: tools.density, state: tools.state };
        runner.0.exec(move |sim| sim.set_fill(fill));
    }
}

// ---------------------------------------------------------------- helpers

fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(palette.text_muted));
    ui.add_space(2.0);
}

fn card<R>(ui: &mut Ui, palette: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
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

fn muted(ui: &mut Ui, palette: &Palette, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(12.0).color(palette.text_muted));
}

/// Equal-width toggle buttons in rows of `columns`; returns the clicked index.
fn segmented(ui: &mut Ui, items: &[(&str, &str, bool)], columns: usize) -> Option<usize> {
    let gap = 4.0;
    let columns = columns.clamp(1, items.len().max(1));
    let width = ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).floor().max(36.0);
    let mut clicked = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        for (row, chunk) in items.chunks(columns).enumerate() {
            ui.horizontal(|ui| {
                for (j, (label, tip, on)) in chunk.iter().enumerate() {
                    let button =
                        egui::Button::new(RichText::new(*label).size(12.5)).selected(*on).min_size(vec2(width, 26.0));
                    if ui.add(button).on_hover_text(*tip).clicked() {
                        clicked = Some(row * columns + j);
                    }
                }
            });
        }
    });
    clicked
}

fn text_param(params: Option<&ParamSet>, key: &str) -> Option<String> {
    params.and_then(|p| p.get(key)).and_then(ParamValue::as_str).map(str::to_string)
}

fn bool_param(params: Option<&ParamSet>, key: &str) -> Option<bool> {
    params.and_then(|p| p.get(key)).and_then(ParamValue::as_bool)
}

fn int_param(params: Option<&ParamSet>, key: &str) -> Option<i64> {
    params.and_then(|p| p.get(key)).and_then(ParamValue::as_i64)
}

fn topology_text(topology: Topology) -> &'static str {
    match topology {
        Topology::Bounded => "bounded plane",
        Topology::Torus => "torus",
        Topology::KleinBottle => "Klein bottle",
        Topology::CrossSurface => "cross-surface",
        Topology::Infinite => "infinite plane",
    }
}

/// World facts line of the model card.
pub fn world_line(info: &LabInfo) -> String {
    match &info.detail {
        FamilyInfo::Wolfram { width, first_row, generation, .. } => {
            let kept = generation - first_row + 1;
            let rows = if kept == 1 { "row" } else { "rows" };
            format!("{} cells per row · {} {rows} kept", format_count(u64::from(*width)), format_count(kept))
        }
        _ => match info.world() {
            Some(w) if w.topology == Topology::Infinite => {
                if info.family() == Family::Ant {
                    "infinite plane".to_string()
                } else {
                    format!("infinite plane (home region {} × {})", w.width, w.height)
                }
            }
            Some(w) => format!("{} × {} {}", w.width, w.height, topology_text(w.topology)),
            None => String::new(),
        },
    }
}

/// Names of the states (colours) of the running model.
pub fn state_names(info: &LabInfo) -> Vec<String> {
    match &info.detail {
        FamilyInfo::Life { .. } => vec!["dead".into(), "alive".into()],
        FamilyInfo::Wolfram { .. } => vec!["0".into(), "1".into()],
        FamilyInfo::Generations { rule, .. } => {
            let bb = rule.name() == Some("Brian's Brain");
            (0..rule.states).map(|s| family::generations_state_name(s, rule.states, bb)).collect()
        }
        FamilyInfo::Wireworld { .. } => (0..4).map(|s| family::wireworld_state_name(s).to_string()).collect(),
        FamilyInfo::Ant { rule, .. } => (0..rule.colors())
            .map(|c| {
                let turn = match rule.turn(c) {
                    Turn::Left => "turn left",
                    Turn::Right => "turn right",
                    Turn::None => "no turn",
                    Turn::UTurn => "U-turn",
                };
                format!("colour {c} · {turn}")
            })
            .collect(),
    }
}

/// Number of exact cells per non-zero state in a view, or `None` for a
/// level-of-detail view (texels aggregate several cells).
pub fn census(view: &LabView) -> Option<Vec<u64>> {
    if view.grid.cells_per_texel != 1 {
        return None;
    }
    let mut counts = vec![0u64; usize::from(view.info.states()).max(2)];
    for texel in view.grid.rgba.as_chunks::<4>().0 {
        let s = usize::from(texel[0]);
        if s > 0 {
            if s >= counts.len() {
                counts.resize(s + 1, 0);
            }
            counts[s] += 1;
        }
    }
    Some(counts)
}

fn heading_text(h: Heading) -> &'static str {
    match h {
        Heading::North => "north",
        Heading::East => "east",
        Heading::South => "south",
        Heading::West => "west",
    }
}

// ------------------------------------------------------------- tools panel

/// Left panel: model family, rule, tool and view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tools_panel(
    InMut(ui): InMut<egui::Ui>,
    (theme, registry): (Res<ThemeSettings>, Res<ModeRegistry>),
    latest: Res<LatestView>,
    loaded: Option<Res<LoadedScenario>>,
    catalog: Option<Res<Catalog>>,
    (session, runner, status): (Option<Res<ActiveSession>>, Option<Res<LabRunner>>, Res<SessionStatus>),
    circuits: Res<Circuits>,
    mut tools: ResMut<ToolSettings>,
    mut settings: ResMut<ViewSettings>,
    mut requests: ResMut<LoadRequests>,
    mut fit: ResMut<CameraFit>,
    mut state: ResMut<PanelState>,
) {
    let palette = mode_palette(&theme, &registry);
    let (Some(view), Some(loaded)) = (latest.view.clone(), loaded) else {
        ui.label(RichText::new("Starting the simulation…").weak());
        return;
    };
    let info = &view.info;
    let params = status.get().map(|s| &s.params);
    ui.spacing_mut().slider_width = (ui.available_width() - 120.0).clamp(60.0, 200.0);

    // Model.
    section(ui, &palette, "Model");
    let family = info.family();
    let items: Vec<(&str, &str, bool)> =
        Family::ALL.iter().map(|f| (f.short_label(), f.blurb(), *f == family)).collect();
    if let Some(i) = segmented(ui, &items, 3)
        && Family::ALL[i] != family
    {
        requests.0.push(Family::ALL[i].default_scenario().to_string());
    }
    ui.add_space(4.0);
    card(ui, &palette, |ui| {
        ui.horizontal(|ui| {
            badge(ui, family.label(), palette.accent);
            ui.label(RichText::new(&loaded.name).size(14.0).strong());
        });
        muted(ui, &palette, format!("{} · {}", info.rule_text(), world_line(info)));
        if let Some(catalog) = &catalog {
            let others: Vec<_> = catalog.of_family(family).filter(|s| s.id != loaded.id).collect();
            if !others.is_empty() {
                ui.add_space(2.0);
                egui::ComboBox::from_id_salt("ca_more_scenarios")
                    .selected_text(RichText::new("Other scenarios of this family").size(12.0))
                    .width(ui.available_width().min(260.0))
                    .show_ui(ui, |ui| {
                        for s in others {
                            if ui.selectable_label(false, &s.name).on_hover_text(&s.description).clicked() {
                                requests.0.push(s.id.clone());
                            }
                        }
                    });
            }
        }
        if !loaded.description.is_empty() {
            egui::CollapsingHeader::new(RichText::new("About this scenario").size(12.5))
                .id_salt("ca_about_scenario")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(RichText::new(&loaded.description).size(12.5));
                    if !loaded.reference.is_empty() {
                        ui.add_space(2.0);
                        muted(ui, &palette, format!("Source: {}", loaded.reference));
                    }
                });
        }
        if let Some(reason) = &info.halted {
            ui.label(RichText::new(format!("Stopped: {reason}")).size(12.0).color(palette.warning));
        }
    });

    // Rule.
    section(ui, &palette, "Rule");
    let session = session.as_deref().map(ActiveSession::session);
    let mut t = tools.clone();
    match &info.detail {
        FamilyInfo::Life { world, .. } => {
            life_rule_editor(ui, &palette, params, session, &mut state, world.topology == Topology::Infinite)
        }
        FamilyInfo::Generations { .. } => generations_rule_editor(ui, &palette, params, session, &mut state),
        FamilyInfo::Wireworld { .. } => wireworld_rule(ui, &palette),
        FamilyInfo::Ant { rule, .. } => ant_rule_editor(ui, &palette, &rule.to_string(), params, session, &mut state),
        FamilyInfo::Wolfram { rule, .. } => {
            wolfram_rule_editor(ui, &palette, *rule, params, session, runner.as_deref(), &mut t)
        }
    }

    // Tool.
    section(ui, &palette, "Tool");
    let list = Tool::for_family(family);
    let items: Vec<(&str, &str, bool)> = list.iter().map(|x| (x.label(family), x.help(family), *x == t.tool)).collect();
    if let Some(i) = segmented(ui, &items, 3) {
        t.tool = list[i];
    }
    ui.add_space(4.0);
    tool_settings(ui, &palette, info, &circuits, &mut t);
    tools.set_if_neq(t);

    // View.
    section(ui, &palette, "View");
    let mut s = settings.clone();
    view_section(ui, &palette, info, params, session, &mut s, &mut fit, latest.is_stale());
    settings.set_if_neq(s);

    shortcuts_section(ui, &palette);
}

/// Clickable birth / survival toggles 0..8; returns the toggled (is_birth, n).
fn bs_toggles(ui: &mut Ui, palette: &Palette, birth: u16, survive: u16, b0_allowed: bool) -> Option<(bool, u16)> {
    let mut clicked = None;
    egui::Grid::new(ui.next_auto_id()).num_columns(10).spacing([3.0, 3.0]).show(ui, |ui| {
        for (row, mask, label, tip) in [
            (true, birth, "B", "Born: a dead cell becomes alive with this many live neighbours"),
            (false, survive, "S", "Survive: a live cell stays alive with this many live neighbours"),
        ] {
            ui.label(RichText::new(label).strong().color(palette.text_muted)).on_hover_text(tip);
            for n in 0..=8u16 {
                let on = mask >> n & 1 == 1;
                let enabled = !(row && n == 0 && !b0_allowed);
                let button = egui::Button::new(RichText::new(n.to_string()).monospace().size(12.0))
                    .selected(on)
                    .min_size(vec2(20.0, 20.0));
                let response = ui.add_enabled(enabled, button);
                let response = if enabled {
                    response.on_hover_text(format!(
                        "{} with {n} live neighbour{}",
                        if row { "Birth" } else { "Survival" },
                        if n == 1 { "" } else { "s" }
                    ))
                } else {
                    response.on_disabled_hover_text("Birth with 0 neighbours (B0) needs a finite world.")
                };
                if response.clicked() {
                    clicked = Some((row, n));
                }
            }
            ui.end_row();
        }
    });
    clicked
}

/// Ids of the rule text fields (the draft is kept while one has focus).
const LIFE_RULE_FIELD: &str = "ca_rule_field";
const GENERATIONS_RULE_FIELD: &str = "ca_gen_field";
const ANT_RULE_FIELD: &str = "ca_ant_field";

/// Whether the rule field `name` has keyboard focus.
fn editing(ui: &Ui, name: &str) -> bool {
    ui.memory(|m| m.has_focus(egui::Id::new(name)))
}

/// Rule text field `name`; returns `true` when Enter was pressed in it.
fn rule_field(ui: &mut Ui, name: &str, draft: &mut String, hint: &str) -> bool {
    let response = ui.add(
        egui::TextEdit::singleline(draft)
            .id(egui::Id::new(name))
            .hint_text(hint)
            .desired_width(ui.available_width() - 70.0),
    );
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

fn validation_line(ui: &mut Ui, palette: &Palette, result: Result<String, String>) {
    match result {
        Ok(text) => ui.label(RichText::new(text).size(12.0).color(palette.success)),
        Err(text) => ui.label(RichText::new(text).size(12.0).color(palette.error)),
    };
}

fn life_rule_editor(
    ui: &mut Ui,
    palette: &Palette,
    params: Option<&ParamSet>,
    session: Option<&dyn simulation_core::SessionControl>,
    state: &mut PanelState,
    infinite: bool,
) {
    let effective = text_param(params, "rule").and_then(|t| LifeRule::parse(&t).ok()).unwrap_or(LifeRule::conway());
    let preset = text_param(params, "preset").and_then(|p| RulePreset::parse(&p)).unwrap_or(RulePreset::Custom);
    egui::ComboBox::from_id_salt("ca_life_preset").selected_text(preset.label()).width(ui.available_width()).show_ui(
        ui,
        |ui| {
            for p in RulePreset::ALL.into_iter().filter(|p| *p != RulePreset::Custom) {
                if ui.selectable_label(p == preset, p.label()).clicked()
                    && let Some(session) = session
                {
                    session.set_param("preset", ParamValue::Text(p.as_str().to_string()));
                }
            }
        },
    );
    ui.add_space(4.0);
    if let Some((birth, n)) = bs_toggles(ui, palette, effective.birth, effective.survive, !infinite) {
        let (b, s) = if birth {
            (effective.birth ^ (1 << n), effective.survive)
        } else {
            (effective.birth, effective.survive ^ (1 << n))
        };
        if let (Ok(rule), Some(session)) = (LifeRule::new(b, s), session) {
            session.set_param("rule", ParamValue::Text(rule.to_string()));
        }
    }
    ui.add_space(4.0);
    state.sync_draft(Family::LifeLike, &effective.to_string(), editing(ui, LIFE_RULE_FIELD));
    let check = rules::check_life_rule(&state.rule_draft, infinite);
    let mut apply = false;
    ui.horizontal(|ui| {
        apply |= rule_field(ui, LIFE_RULE_FIELD, &mut state.rule_draft, "e.g. B36/S23 or 23/3");
        let valid = matches!(check, RuleCheck::Valid(r) if r != effective);
        apply |= ui.add_enabled(valid, egui::Button::new("Apply")).clicked();
    });
    match &check {
        RuleCheck::Valid(rule) => {
            let name = rule.name().map(|n| format!(" ({n})")).unwrap_or_default();
            validation_line(ui, palette, Ok(format!("Valid: {rule}{name}")));
        }
        RuleCheck::Invalid(e) => validation_line(ui, palette, Err(e.clone())),
        RuleCheck::Empty => {}
    }
    if apply && let (RuleCheck::Valid(rule), Some(session)) = (&check, session) {
        session.set_param("rule", ParamValue::Text(rule.to_string()));
        state.draft_source = None;
    }
    muted(ui, palette, "Rule changes apply from the next generation; presets are never modified.");
}

fn generations_rule_editor(
    ui: &mut Ui,
    palette: &Palette,
    params: Option<&ParamSet>,
    session: Option<&dyn simulation_core::SessionControl>,
    state: &mut PanelState,
) {
    let effective = text_param(params, "rule")
        .and_then(|t| rules::check_generations_rule(&t).ok())
        .unwrap_or(GenerationsRule::brians_brain());
    let set = |rule: GenerationsRule| {
        if let Some(session) = session {
            session.set_param("rule", ParamValue::Text(rule.to_string()));
        }
    };
    let current_name = effective.name().unwrap_or("Custom rule");
    egui::ComboBox::from_id_salt("ca_gen_preset").selected_text(current_name).width(ui.available_width()).show_ui(
        ui,
        |ui| {
            for (name, rule) in GenerationsRule::known_rules() {
                let label = format!("{name} ({})", rule.to_bsc_string());
                if ui.selectable_label(rule == effective, label).clicked() {
                    set(rule);
                }
            }
        },
    );
    ui.add_space(4.0);
    if let Some((birth, n)) = bs_toggles(ui, palette, effective.birth, effective.survive, false)
        && let Ok(rule) = if birth {
            GenerationsRule::new(effective.birth ^ (1 << n), effective.survive, effective.states)
        } else {
            GenerationsRule::new(effective.birth, effective.survive ^ (1 << n), effective.states)
        }
    {
        set(rule);
    }
    ui.horizontal(|ui| {
        ui.label("States (C)").on_hover_text("Dead, alive and C − 2 dying states.");
        let mut c = effective.states;
        if ui.add(egui::DragValue::new(&mut c).range(2..=255)).changed()
            && let Ok(rule) = GenerationsRule::new(effective.birth, effective.survive, c)
        {
            set(rule);
        }
    });
    ui.add_space(4.0);
    state.sync_draft(Family::Generations, &effective.to_bsc_string(), editing(ui, GENERATIONS_RULE_FIELD));
    let check = rules::check_generations_rule(&state.rule_draft);
    let mut apply = false;
    ui.horizontal(|ui| {
        apply |= rule_field(ui, GENERATIONS_RULE_FIELD, &mut state.rule_draft, "e.g. B2/S/C3, /2/3 or Star Wars");
        let valid = matches!(check, Ok(r) if r != effective);
        apply |= ui.add_enabled(valid, egui::Button::new("Apply")).clicked();
    });
    match &check {
        Ok(rule) => validation_line(
            ui,
            palette,
            Ok(format!(
                "Valid: {} (Golly {rule}){}",
                rule.to_bsc_string(),
                rule.name().map(|n| format!(", {n}")).unwrap_or_default()
            )),
        ),
        Err(e) if !state.rule_draft.trim().is_empty() => validation_line(ui, palette, Err(e.clone())),
        Err(_) => {}
    }
    if apply && let Ok(rule) = check {
        set(rule);
        state.draft_source = None;
    }
    muted(ui, palette, "Applies from the next generation; cells in states the new rule lacks decay to 0.");
}

fn wireworld_rule(ui: &mut Ui, palette: &Palette) {
    card(ui, palette, |ui| {
        for (state, text) in [
            (1usize, "electron head becomes electron tail"),
            (2, "electron tail becomes conductor"),
            (3, "conductor becomes head with 1 or 2 head neighbours"),
            (0, "empty stays empty"),
        ] {
            ui.horizontal(|ui| {
                swatch(ui, color32(family::WIREWORLD[state]), 12.0);
                ui.label(RichText::new(text).size(12.5));
            });
        }
    });
    muted(ui, palette, "Wireworld has one fixed rule: circuits are built from the cells, not from the rule.");
}

fn ant_rule_editor(
    ui: &mut Ui,
    palette: &Palette,
    running: &str,
    params: Option<&ParamSet>,
    session: Option<&dyn simulation_core::SessionControl>,
    state: &mut PanelState,
) {
    let next = text_param(params, "rule").unwrap_or_else(|| running.to_string());
    egui::ComboBox::from_id_salt("ca_ant_preset")
        .selected_text(format!("Rule presets · {next}"))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (rule, what) in rules::TURMITE_PRESETS {
                if ui.selectable_label(rule == next, format!("{rule}  ·  {what}")).clicked() {
                    state.rule_draft = rule.to_string();
                    state.draft_source = Some((Family::Ant, next.clone()));
                }
            }
        })
        .response
        .on_hover_text("Puts the rule in the field below; Apply sets it and resets to step 0.");
    ui.add_space(4.0);
    state.sync_draft(Family::Ant, &next, editing(ui, ANT_RULE_FIELD));
    let check = rules::check_turmite_rule(&state.rule_draft);
    let mut apply = false;
    ui.horizontal(|ui| {
        apply |= rule_field(ui, ANT_RULE_FIELD, &mut state.rule_draft, "letters L R N U, e.g. LLRR");
        let valid = matches!(&check, Ok(r) if r.to_string() != running);
        apply |= ui
            .add_enabled(valid, egui::Button::new("Apply"))
            .on_hover_text("Sets the rule and resets to step 0 (the reset is undoable).")
            .clicked();
    });
    match &check {
        Ok(rule) => validation_line(ui, palette, Ok(format!("Valid: {rule}, {} colours", rule.colors()))),
        Err(e) if !state.rule_draft.trim().is_empty() => validation_line(ui, palette, Err(e.clone())),
        Err(_) => {}
    }
    if apply && let (Ok(rule), Some(session)) = (&check, session) {
        session.set_param("rule", ParamValue::Text(rule.to_string()));
        session.reset();
        state.draft_source = None;
    }
    if next != running {
        ui.label(
            RichText::new(format!("Running {running}; the next reset runs {next}.")).size(12.0).color(palette.warning),
        );
    }
    muted(ui, palette, "One letter per colour: L left, R right, N no turn, U U-turn. RL is Langton's ant.");
}

/// The 8-case table of an elementary rule (111 ... 000, Wolfram's order),
/// each case clickable; returns the clicked neighbourhood index.
fn rule_icons(ui: &mut Ui, palette: &Palette, rule: u8, enabled: bool) -> Option<u8> {
    let cell = 11.0;
    let gap = 1.0;
    let case_w = 3.0 * cell + 2.0 * gap;
    let spacing = ((ui.available_width() - 4.0 * case_w) / 4.0).clamp(6.0, 18.0);
    let mut clicked = None;
    let on = color32(family::ALIVE);
    let off = color32(family::DEAD);
    for row in 0..2u8 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            for col in 0..4u8 {
                let i = 7 - (row * 4 + col);
                let size = vec2(case_w, 2.0 * cell + 6.0 + 12.0);
                let sense = if enabled { Sense::click() } else { Sense::hover() };
                let (rect, response) = ui.allocate_exact_size(size, sense);
                let painter = ui.painter_at(rect.expand(2.0));
                if response.hovered() && enabled {
                    painter.rect_filled(rect.expand(2.0), 3.0, palette.surface_hi);
                }
                let [l, c, r] = rules::neighbourhood(i);
                for (k, v) in [l, c, r].into_iter().enumerate() {
                    let min = rect.min + vec2(k as f32 * (cell + gap), 0.0);
                    let sq = egui::Rect::from_min_size(min, vec2(cell, cell));
                    painter.rect_filled(sq, 1.0, if v { on } else { off });
                    painter.rect_stroke(sq, 1.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
                }
                let out = egui::Rect::from_min_size(rect.min + vec2(cell + gap, cell + 3.0), vec2(cell, cell));
                let value = rules::output(rule, i);
                painter.rect_filled(out, 1.0, if value { on } else { off });
                painter.rect_stroke(out, 1.0, Stroke::new(1.2, palette.accent), egui::StrokeKind::Inside);
                painter.text(
                    egui::pos2(rect.center().x, rect.max.y - 5.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{i:03b}"),
                    egui::FontId::monospace(9.5),
                    palette.text_muted,
                );
                let response = response.on_hover_text(format!(
                    "Left {}, centre {}, right {}: new cell {} (bit {i} of the rule){}",
                    u8::from(l),
                    u8::from(c),
                    u8::from(r),
                    u8::from(value),
                    if enabled { ". Click to flip it." } else { "" }
                ));
                if response.clicked() {
                    clicked = Some(i);
                }
            }
        });
    }
    clicked
}

fn wolfram_rule_editor(
    ui: &mut Ui,
    palette: &Palette,
    running: u8,
    params: Option<&ParamSet>,
    session: Option<&dyn simulation_core::SessionControl>,
    runner: Option<&LabRunner>,
    tools: &mut ToolSettings,
) {
    let next = int_param(params, "rule").and_then(|r| u8::try_from(r).ok()).unwrap_or(running);
    let set = |rule: u8| {
        if let Some(session) = session {
            session.set_param("rule", ParamValue::Int(i64::from(rule)));
        }
    };
    ui.horizontal(|ui| {
        ui.label("Rule number");
        let mut r = next;
        if ui.add(egui::DragValue::new(&mut r).range(0..=255)).on_hover_text("0 to 255; drag or type").changed() {
            set(r);
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Examples").size(12.0).color(palette.text_muted));
        for famous in [30u8, 90, 110, 184] {
            if ui.add(egui::Button::new(famous.to_string()).selected(next == famous)).clicked() {
                set(famous);
            }
        }
    });
    ui.add_space(4.0);
    if let Some(i) = rule_icons(ui, palette, next, session.is_some()) {
        set(rules::toggle_output(next, i));
    }
    let bits: String = (0..8).rev().map(|i| if rules::output(next, i) { '1' } else { '0' }).collect();
    muted(ui, palette, format!("{next} = {bits} in binary: the outputs for 111 down to 000."));
    if next != running {
        ui.label(
            RichText::new(format!("Rule {next} applies from the next row (running {running})."))
                .size(12.0)
                .color(palette.warning),
        );
    }
    ui.add_space(4.0);
    let boundary = text_param(params, "boundary").unwrap_or_else(|| "fixed0".to_string());
    ui.horizontal(|ui| {
        ui.label("Edges");
        for (key, label, tip) in [
            ("fixed0", "0", "Cells beyond both ends are always 0."),
            ("fixed1", "1", "Cells beyond both ends are always 1."),
            ("periodic", "Ring", "The row is a ring: the left end touches the right end."),
        ] {
            if ui.add(egui::Button::new(label).selected(boundary == key)).on_hover_text(tip).clicked()
                && let Some(session) = session
            {
                session.set_param("boundary", ParamValue::Text(key.to_string()));
            }
        }
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Current row").size(12.5).strong());
    ui.horizontal_wrapped(|ui| {
        let edit = |command: WolframCommand| {
            if let Some(runner) = runner {
                runner.0.send(Control::Edit(LabCommand::Wolfram(command)));
            }
        };
        if ui
            .button("Single cell")
            .on_hover_text("Replace the current row by one 1 in the middle (undoable).")
            .clicked()
        {
            edit(WolframCommand::SingleCell);
        }
        if ui
            .button(format!("Random ({:.0} %)", tools.density * 100.0))
            .on_hover_text("Replace the current row by random cells at the Random tool's density (undoable).")
            .clicked()
        {
            edit(WolframCommand::Randomize { density: tools.density, seed: None });
        }
        if ui.button("Clear").on_hover_text("Set the current row to 0 (undoable).").clicked() {
            edit(WolframCommand::Clear);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Density of 1s");
        // Leave room for the value box: the slider must not widen the panel.
        ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(60.0);
        ui.add(egui::Slider::new(&mut tools.density, 0.0..=1.0).max_decimals(2))
            .on_hover_text("Fraction of 1s in a random row: the Random button above and Randomize in the bottom bar.");
    });
}

fn state_picker(ui: &mut Ui, palette: &Palette, info: &LabInfo, t: &mut ToolSettings) {
    let colors = state_colors(info);
    let names = state_names(info);
    let states = info.states();
    if states <= 2 {
        return;
    }
    ui.label(RichText::new(if info.family() == Family::Ant { "Colour" } else { "State" }).size(12.5));
    if states <= 16 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for s in 1..states.min(256) as u8 {
                let color = colors.get(usize::from(s)).or(colors.last()).copied().unwrap_or(family::ALIVE);
                let selected = t.state == s;
                let (rect, response) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect.shrink(3.0), 3.0, color32(color));
                let stroke = if selected { Stroke::new(2.0, palette.accent) } else { Stroke::new(1.0, palette.stroke) };
                painter.rect_stroke(rect.shrink(1.0), 4.0, stroke, egui::StrokeKind::Inside);
                let name = names.get(usize::from(s)).cloned().unwrap_or_default();
                if response.on_hover_text(format!("{s}: {name}")).clicked() {
                    t.state = s;
                }
            }
        });
        if let Some(name) = names.get(usize::from(t.state)) {
            muted(ui, palette, format!("{}: {name}", t.state));
        }
    } else {
        ui.horizontal(|ui| {
            let mut s = t.state;
            ui.add(egui::DragValue::new(&mut s).range(1..=states.saturating_sub(1).min(255) as u8));
            t.state = s;
            let color = colors.get(usize::from(s)).or(colors.last()).copied().unwrap_or(family::ALIVE);
            swatch(ui, color32(color), 14.0);
            if let Some(name) = names.get(usize::from(s)) {
                muted(ui, palette, name.clone());
            }
        });
    }
}

fn tool_settings(ui: &mut Ui, palette: &Palette, info: &LabInfo, circuits: &Circuits, t: &mut ToolSettings) {
    let family = info.family();
    match t.tool {
        Tool::Inspect => muted(
            ui,
            palette,
            "Hover the world: the cell is outlined and described next to the pointer and in Properties.",
        ),
        Tool::Draw | Tool::Erase | Tool::RemoveAnts => {
            if t.tool == Tool::Draw {
                state_picker(ui, palette, info, t);
            }
            ui.horizontal(|ui| {
                ui.label(if t.tool == Tool::RemoveAnts { "Reach (cells)" } else { "Radius (cells)" });
                ui.add(egui::Slider::new(&mut t.radius, 0..=MAX_RADIUS));
            });
            muted(ui, palette, "[ and ] change the radius; one stroke is one undo step.");
        }
        Tool::Line => {
            state_picker(ui, palette, info, t);
            muted(
                ui,
                palette,
                if family == Family::Wireworld {
                    "Drag from one end of the wire to the other. Paint a head next to a tail on a wire to send an electron."
                } else {
                    "Drag from one end of the line to the other."
                },
            );
        }
        Tool::Fill => {
            state_picker(ui, palette, info, t);
            ui.horizontal(|ui| {
                ui.label("Density");
                ui.add(egui::Slider::new(&mut t.density, 0.0..=1.0).max_decimals(2));
            });
            if family == Family::LifeLike {
                egui::ComboBox::from_id_salt("ca_symmetry").selected_text(symmetry_label(t.symmetry)).show_ui(
                    ui,
                    |ui| {
                        for s in [
                            Symmetry::None,
                            Symmetry::Horizontal,
                            Symmetry::Vertical,
                            Symmetry::Both,
                            Symmetry::Rotate4,
                            Symmetry::Rotate8,
                        ] {
                            ui.selectable_value(&mut t.symmetry, s, symmetry_label(s));
                        }
                    },
                );
            }
            muted(
                ui,
                palette,
                "Drag a rectangle: its cells are replaced at random. The density also drives Randomize in the bottom bar.",
            );
        }
        Tool::Stamp => {
            if t.circuit >= circuits.0.len() {
                t.circuit = 0;
            }
            for (i, circuit) in circuits.0.iter().enumerate() {
                let name = circuit.name();
                let size = format!("{} × {}", circuit.pattern.width(), circuit.pattern.height());
                let response =
                    ui.selectable_label(t.circuit == i, format!("{name}   ({size})")).on_hover_text(format!(
                        "{}\nLicence: {}",
                        circuit.pattern.description.lines().next().unwrap_or_default(),
                        circuit.licence
                    ));
                if response.clicked() {
                    t.circuit = i;
                }
            }
            ui.horizontal(|ui| {
                ui.label("Rotation");
                if ui.button("⟲").on_hover_text("Counter-clockwise (Q)").clicked() {
                    t.turn(false);
                }
                ui.label(RichText::new(rotation_label(t.rotation)).monospace());
                if ui.button("⟳").on_hover_text("Clockwise (E)").clicked() {
                    t.turn(true);
                }
                ui.checkbox(&mut t.mirror, "Mirror");
            });
            muted(
                ui,
                palette,
                "Click to place the circuit centred on the pointer (one undo step). Circuits come with their input electrons, which start moving when the simulation runs.",
            );
        }
        Tool::AddAnt => {
            ui.horizontal(|ui| {
                ui.label("Heading");
                for h in [Heading::North, Heading::East, Heading::South, Heading::West] {
                    ui.radio_value(&mut t.heading, h, heading_text(h));
                }
            });
            muted(ui, palette, "Click to add an ant; it moves after the existing ants. Q / E turn the heading.");
        }
        Tool::Toggle => muted(
            ui,
            palette,
            "Click cells of the newest row (just above the accent line) to flip them; drag along it to set more cells to the same value. Older rows are history and cannot change.",
        ),
    }
}

fn symmetry_label(s: Symmetry) -> &'static str {
    match s {
        Symmetry::None => "No symmetry",
        Symmetry::Horizontal => "Left-right mirror",
        Symmetry::Vertical => "Top-bottom mirror",
        Symmetry::Both => "Both mirrors",
        Symmetry::Rotate4 => "90° rotations (square)",
        Symmetry::Rotate8 => "All 8 symmetries (square)",
    }
}

fn rotation_label(r: cellular_automata::Rotation) -> &'static str {
    match r.quarter_turns() % 4 {
        0 => "0°",
        1 => "90°",
        2 => "180°",
        _ => "270°",
    }
}

#[allow(clippy::too_many_arguments)]
fn view_section(
    ui: &mut Ui,
    palette: &Palette,
    info: &LabInfo,
    params: Option<&ParamSet>,
    session: Option<&dyn simulation_core::SessionControl>,
    s: &mut ViewSettings,
    fit: &mut CameraFit,
    stale: bool,
) {
    let family = info.family();
    // (Not while the view still belongs to the replaced model: a colouring
    // chosen for the new one would be lost.)
    if !s.colouring.available(family) && !stale {
        s.colouring = Colouring::States;
    }
    ui.horizontal(|ui| {
        ui.label("Colouring");
        egui::ComboBox::from_id_salt("ca_colouring").selected_text(s.colouring.label()).show_ui(ui, |ui| {
            for c in Colouring::ALL.into_iter().filter(|c| c.available(family)) {
                ui.selectable_value(&mut s.colouring, c, c.label()).on_hover_text(c.help());
            }
        });
    });
    if let Some(key) = s.colouring.tracking_param(family)
        && bool_param(params, key) == Some(false)
    {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("The model does not record this yet.").size(12.0).color(palette.warning));
            if ui.small_button("Start recording").clicked()
                && let Some(session) = session
            {
                session.set_param(key, ParamValue::Bool(true));
            }
        });
    }
    ui.checkbox(&mut s.grid_lines, "Cell grid (G)")
        .on_hover_text("Cell outlines; they fade in once cells are large on screen.");
    if info.bounds().is_some() {
        ui.checkbox(&mut s.show_border, "World border");
    }
    match family {
        Family::Ant => {
            ui.checkbox(&mut s.follow, "Follow the first ant (V)")
                .on_hover_text("Recentre the view when the ant nears its edge. Panning turns it off.");
        }
        Family::Wolfram => {
            ui.checkbox(&mut s.follow, "Follow the newest row (V)")
                .on_hover_text("Scroll down as the diagram grows. Panning turns it off.");
        }
        _ => {}
    }
    ui.add_space(4.0);
    if ui.button("Frame the world  F").clicked() {
        fit.refit();
    }
}

fn shortcuts_section(ui: &mut Ui, palette: &Palette) {
    ui.add_space(8.0);
    egui::CollapsingHeader::new(RichText::new("Shortcuts and mouse").size(12.5).color(palette.text_muted))
        .id_salt("ca_shortcuts")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("ca_shortcut_grid").num_columns(2).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
                for (_, _, keys, description) in MODE_SHORTCUTS {
                    ui.label(RichText::new(*keys).monospace().size(12.0));
                    ui.label(RichText::new(*description).size(12.0));
                    ui.end_row();
                }
                for (keys, description) in [
                    ("Left drag", "Use the tool"),
                    ("Right / middle drag", "Pan"),
                    ("Wheel, pinch", "Zoom towards the pointer"),
                    ("Arrows, WASD", "Pan"),
                ] {
                    ui.label(RichText::new(keys).monospace().size(12.0));
                    ui.label(RichText::new(description).size(12.0));
                    ui.end_row();
                }
            });
        });
}

// -------------------------------------------------------- properties panel

/// Right panel "Properties": rule comparison, family details, dynamics,
/// cells in view, inspector.
#[allow(clippy::too_many_arguments)]
pub(crate) fn properties_panel(
    InMut(ui): InMut<egui::Ui>,
    (theme, registry): (Res<ThemeSettings>, Res<ModeRegistry>),
    latest: Res<LatestView>,
    status: Res<SessionStatus>,
    session: Option<Res<ActiveSession>>,
    history: Res<MetricsHistory>,
    hover: Res<Hover>,
    mut state: ResMut<PanelState>,
) {
    let palette = mode_palette(&theme, &registry);
    let Some(view) = latest.view.clone() else {
        ui.label(RichText::new("Starting the simulation…").weak());
        return;
    };
    let info = &view.info;
    let params = status.get().map(|s| &s.params);

    section(ui, &palette, "Rule");
    rule_comparison(ui, &palette, info);

    match &info.detail {
        FamilyInfo::Wolfram { rule, .. } => {
            section(ui, &palette, "Behaviour class");
            let session = session.as_deref().map(ActiveSession::session);
            wolfram_class(ui, &palette, *rule, &mut state, session);
        }
        FamilyInfo::Ant { rule, ants, first, .. } => {
            section(ui, &palette, "Ants");
            ant_details(ui, &palette, &rule.to_string(), *ants, *first, info, params);
        }
        _ => {}
    }

    section(ui, &palette, "Dynamics");
    dynamics(ui, &palette, info, &history);

    section(ui, &palette, "Cells in view");
    let counts = state.census_of(latest.seq, &view);
    cells_in_view(ui, &palette, &view, counts);

    section(ui, &palette, "Inspector");
    card(ui, &palette, |ui| match hover.cell {
        Some(cell) => {
            for line in inspect(&view, cell) {
                ui.label(RichText::new(line).size(12.5));
            }
        }
        None => muted(ui, &palette, "Hover the world to read a cell."),
    });
}

/// The rule comparison card: born / survive sets, states, neighbourhood,
/// and the differences from Conway's B3/S23.
pub fn rule_facts(info: &LabInfo) -> (Vec<(&'static str, String)>, Vec<String>) {
    match &info.detail {
        FamilyInfo::Life { rule, .. } => {
            let mut notes = rules::conway_differences(rule.birth, rule.survive);
            if notes.is_empty() {
                notes.push("This is Conway's Game of Life.".to_string());
            }
            if rules::self_complementary(*rule) {
                notes.push("Swapping live and dead cells gives the same rule: dead islands in a live sea behave like live islands in a dead sea.".to_string());
            }
            if rule.has_b0() {
                notes.push("B0: empty cells with no live neighbour are born (finite worlds only).".to_string());
            }
            (
                vec![
                    ("Born with", rules::counts(rule.birth)),
                    ("Survive with", rules::counts(rule.survive)),
                    ("States", "2 (dead, alive)".to_string()),
                    ("Neighbourhood", "Moore: the 8 surrounding cells".to_string()),
                ],
                notes,
            )
        }
        FamilyInfo::Generations { rule, .. } => {
            let mut notes = rules::conway_differences(rule.birth, rule.survive);
            let c = rule.states;
            if c > 2 {
                let n = c - 2;
                let states = if n == 1 { "state 2".to_string() } else { format!("states 2 to {}", c - 1) };
                notes.push(format!(
                    "A live cell that does not survive spends {n} generation{} dying ({states}) before it is dead; Conway's cells die at once.",
                    if n == 1 { "" } else { "s" },
                ));
                notes.push("Dying cells cannot be born again or counted as neighbours: only state-1 cells count.".to_string());
            } else {
                notes.push("With 2 states this is the Life-like rule with the same B/S sets.".to_string());
            }
            (
                vec![
                    ("Born with", rules::counts(rule.birth)),
                    ("Survive with", rules::counts(rule.survive)),
                    ("States", format!("{c} (dead, alive, {} dying)", c.saturating_sub(2))),
                    ("Neighbourhood", "Moore (8 cells), counting alive (state 1) cells".to_string()),
                ],
                notes,
            )
        }
        FamilyInfo::Wireworld { .. } => (
            vec![
                ("Born with", "not a birth rule: empty cells never change".to_string()),
                ("Conductor", "becomes a head with 1 or 2 head neighbours".to_string()),
                ("States", "4 (empty, head, tail, conductor)".to_string()),
                ("Neighbourhood", "Moore (8 cells), counting electron heads".to_string()),
            ],
            vec![
                "Unlike Conway's rule, only conductors can start an electron; heads always become tails and tails conductors, so a pulse moves one cell per generation along a wire and never reverses.".to_string(),
            ],
        ),
        FamilyInfo::Ant { rule, .. } => {
            let turns: Vec<String> = (0..rule.colors())
                .map(|c| format!("{c}: {}", rule.to_string().chars().nth(usize::from(c)).unwrap_or('?')))
                .collect();
            (
                vec![
                    ("Colours", rule.colors().to_string()),
                    ("Turn per colour", turns.join(", ")),
                    ("Neighbourhood", "the single cell under each ant".to_string()),
                ],
                vec![
                    "Not a rule applied to every cell at once: each step, each ant turns by the colour under it, advances that cell to the next colour and moves one cell forward. Conway updates every cell simultaneously from its 8 neighbours.".to_string(),
                ],
            )
        }
        FamilyInfo::Wolfram { rule, .. } => (
            vec![
                ("Rule", format!("{rule} = {:08b}", rule)),
                ("States", "2 (0, 1)".to_string()),
                ("Neighbourhood", "left, centre, right: 3 cells in one row".to_string()),
            ],
            vec![
                "One-dimensional: the whole world is one row, and time runs down the diagram. Conway's rule only counts live neighbours (outer totalistic); here every arrangement of the 3 cells has its own output, 8 cases in all.".to_string(),
            ],
        ),
    }
}

fn rule_comparison(ui: &mut Ui, palette: &Palette, info: &LabInfo) {
    let (facts, notes) = rule_facts(info);
    card(ui, palette, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(info.rule_text()).size(15.0).strong().monospace());
            if let FamilyInfo::Life { rule, .. } = &info.detail
                && let Some(name) = rule.name()
            {
                badge(ui, name, palette.accent);
            }
            if let FamilyInfo::Generations { rule, .. } = &info.detail
                && let Some(name) = rule.name()
            {
                badge(ui, name, palette.accent);
            }
        });
        egui::Grid::new("ca_rule_facts").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            for (k, v) in &facts {
                ui.label(RichText::new(*k).size(12.0).color(palette.text_muted));
                ui.label(RichText::new(v).size(12.5));
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Compared with Conway's B3/S23").size(12.0).strong());
        for note in &notes {
            // A hanging indent: wrapped lines continue under the text, not under the bullet.
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("•").color(palette.accent));
                ui.add(egui::Label::new(RichText::new(note).size(12.5)).wrap());
            });
        }
    });
}

fn wolfram_class(
    ui: &mut Ui,
    palette: &Palette,
    rule: u8,
    state: &mut PanelState,
    session: Option<&dyn simulation_core::SessionControl>,
) {
    let class = state.class_of(rule);
    card(ui, palette, |ui| {
        ui.horizontal(|ui| {
            let color = match class {
                MeasuredClass::Uniform => palette.text_muted,
                MeasuredClass::Periodic { .. } => palette.success,
                MeasuredClass::Aperiodic => palette.warning,
            };
            badge(ui, class.label(), color);
            ui.label(RichText::new("measured").size(11.5).color(palette.text_muted));
        });
        ui.label(RichText::new(format!("Rule {rule}: {}.", class.detail())).size(12.5));
        muted(
            ui,
            palette,
            format!(
                "{} random rows (half 1s) on a ring of {} cells; rows equal up to a shift count as a repeat. The test cannot tell class III from IV.",
                rules::CLASS_TRIALS,
                rules::CLASS_WIDTH
            ),
        );
        if let Some(note) = rules::literature_note(rule) {
            ui.label(RichText::new(note).size(12.5));
        }
        if let Some(form) = rules::closed_form(rule) {
            ui.label(RichText::new(format!("Closed form: {form}.")).size(12.5));
        }
        ui.add_space(4.0);
        let others = rules::distinct_equivalents(rule);
        if others.is_empty() {
            ui.label(
                RichText::new(format!("Rule {rule} is its own mirror image and its own 0/1 complement.")).size(12.0),
            );
        } else {
            ui.label(RichText::new("Equivalent rules (same behaviour, mirrored or with 0 and 1 swapped)").size(12.0));
            ui.horizontal_wrapped(|ui| {
                for (r, label) in others {
                    let button = egui::Button::new(RichText::new(format!("{r} ({label})")).size(12.0));
                    if ui
                        .add_enabled(session.is_some(), button)
                        .on_hover_text(format!("Run rule {r} from the next row"))
                        .clicked()
                        && let Some(session) = session
                    {
                        session.set_param("rule", ParamValue::Int(i64::from(r)));
                    }
                }
            });
        }
    });
}

fn ant_details(
    ui: &mut Ui,
    palette: &Palette,
    rule: &str,
    ants: usize,
    first: Option<cellular_automata::ant::Ant>,
    info: &LabInfo,
    params: Option<&ParamSet>,
) {
    let steps = info.tick;
    card(ui, palette, |ui| {
        egui::Grid::new("ca_ant_facts").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            ui.label(RichText::new("Steps").size(12.0).color(palette.text_muted));
            ui.label(RichText::new(format_count(steps)).monospace());
            ui.end_row();
            ui.label(RichText::new("Ants").size(12.0).color(palette.text_muted));
            ui.label(RichText::new(ants.to_string()).monospace());
            ui.end_row();
            if let Some(a) = first {
                ui.label(RichText::new("First ant").size(12.0).color(palette.text_muted));
                ui.label(RichText::new(format!("({}, {}) heading {}", a.x, a.y, heading_text(a.heading))).monospace());
                ui.end_row();
            }
            if let Some(n) = int_param(params, "steps_per_tick") {
                ui.label(RichText::new("Steps per tick").size(12.0).color(palette.text_muted));
                ui.label(RichText::new(n.to_string()).monospace());
                ui.end_row();
            }
        });
        if rule == "RL" && ants == 1 {
            ui.add_space(4.0);
            ui.label(RichText::new("Langton's highway").size(12.5).strong());
            ui.label(RichText::new(
                "A single RL ant started on an empty grid walks chaotically for about 10,000 steps; from step 9977 the colours it reads repeat every 104 steps and it builds a diagonal highway, moving 2 cells diagonally and adding 12 coloured cells per period.",
            ).size(12.5));
            // The countdown only holds for the bundled start (one ant on an empty plane).
            if info.scenario == "langtons-ant-highway" {
                let phase = if steps < 9977 {
                    format!("{} steps until step 9977 (if the start was not edited).", format_count(9977 - steps))
                } else {
                    "Past step 9977: the highway phase, unless the start was edited.".to_string()
                };
                muted(ui, palette, phase);
            }
        }
    });
}

/// Grid marks of a plot axis: one "nice" step (1, 2 or 5 times a power of
/// ten) that splits `bounds` into about `target` intervals, so every
/// labelled line has the same weight (egui_plot's default powers of ten
/// leave a 130-pixel plot of 0..800 without a single label).
pub fn nice_marks(bounds: (f64, f64), target: f64) -> Vec<GridMark> {
    let (lo, hi) = bounds;
    let span = hi - lo;
    if !(lo.is_finite() && hi.is_finite() && span > 0.0) {
        return Vec::new();
    }
    let raw = span / target.max(1.0);
    let base = 10f64.powf(raw.log10().floor());
    let m = raw / base;
    let step = base
        * if m <= 1.0 {
            1.0
        } else if m <= 2.0 {
            2.0
        } else if m <= 5.0 {
            5.0
        } else {
            10.0
        };
    let (first, last) = ((lo / step).ceil(), (hi / step).floor());
    if !(first.is_finite() && last.is_finite()) || last - first > 64.0 {
        return Vec::new();
    }
    let first = first as i64;
    (0..=(last as i64 - first)).map(|i| GridMark { value: (first + i) as f64 * step, step_size: step }).collect()
}

/// Axis label of a grid mark: counts in full (or with k / M suffixes),
/// fractions with as many decimals as the step needs.
pub fn axis_label(value: f64, step: f64) -> String {
    if step >= 1.0 || value == 0.0 {
        format_compact(value.round())
    } else {
        let decimals = (-step.log10().floor()).clamp(1.0, 6.0) as usize;
        format!("{value:.decimals$}")
    }
}

/// Metric series plotted per family: (key, label).
pub fn dynamics_series(family: Family) -> &'static [(&'static str, &'static str)] {
    match family {
        Family::LifeLike => &[("population", "Population"), ("births", "Births"), ("deaths", "Deaths")],
        Family::Generations => &[("alive", "Alive"), ("dying", "Dying"), ("births", "Births")],
        Family::Wireworld => &[("heads", "Electron heads"), ("tails", "Electron tails")],
        Family::Ant => &[("colored", "Coloured cells")],
        Family::Wolfram => &[("density", "Density of each row")],
    }
}

fn dynamics(ui: &mut Ui, palette: &Palette, info: &LabInfo, history: &MetricsHistory) {
    let store = history.store();
    let family = info.family();
    let series = dynamics_series(family);
    let lines: Vec<(usize, &str, Vec<[f64; 2]>)> = series
        .iter()
        .enumerate()
        .filter_map(|(i, (key, label))| {
            let pts: Vec<[f64; 2]> = store.get(key)?.display().into_iter().map(|p| [p.tick as f64, p.value]).collect();
            (!pts.is_empty()).then_some((i, *label, pts))
        })
        .collect();
    if lines.is_empty() {
        muted(ui, palette, "Run the simulation (Space) to record its metrics.");
        return;
    }
    let mut latest_line = Vec::new();
    for (key, label) in series {
        if let Some(v) = history.latest(key) {
            let value = if *key == "density" { format!("{:.3}", v) } else { format_count(v.max(0.0) as u64) };
            latest_line.push(format!("{label} {value}"));
        }
    }
    muted(ui, palette, latest_line.join("  ·  "));
    let entries: Vec<(egui::Color32, &str)> =
        lines.iter().map(|(i, label, _)| (palette.series_color(*i), *label)).collect();
    legend_row(ui, &entries);
    let x_label = match family {
        Family::Ant => "step",
        Family::Wolfram => "row",
        _ => "generation",
    };
    Plot::new("ca_dynamics")
        .height(130.0)
        .allow_scroll(false)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_background(false)
        .grid_color(with_alpha(palette.text_muted, 0.3))
        .custom_x_axes(vec![AxisHints::new_x().label(x_label).label_spacing(26.0..=40.0)])
        .y_grid_spacer(|input| nice_marks(input.bounds, 5.0))
        .custom_y_axes(vec![
            AxisHints::new_y()
                .formatter(|mark, _| axis_label(mark.value, mark.step_size))
                .min_thickness(38.0)
                .label_spacing(16.0..=24.0),
        ])
        .show(ui, |plot| {
            for (i, label, pts) in lines {
                plot.line(
                    Line::new(label.to_string(), PlotPoints::from(pts)).color(palette.series_color(i)).width(1.6),
                );
            }
        });
    if family == Family::LifeLike {
        muted(ui, palette, "Births and deaths are summed over the generations of each tick.");
    }
    if family == Family::Wolfram {
        muted(ui, palette, "One point per row: the fraction of 1s in that row (metric \u{201c}density\u{201d}).");
    }
}

fn cells_in_view(ui: &mut Ui, palette: &Palette, view: &LabView, counts: Option<Vec<u64>>) {
    let colors = state_colors(&view.info);
    let names = state_names(&view.info);
    match counts {
        Some(counts) => {
            let total: u64 = counts.iter().sum();
            card(ui, palette, |ui| {
                if total > 0 {
                    egui::Grid::new("ca_census").num_columns(3).spacing([8.0, 2.0]).show(ui, |ui| {
                        for (s, n) in counts.iter().enumerate().skip(1).filter(|(_, n)| **n > 0).take(24) {
                            let color = colors.get(s).or(colors.last()).copied().unwrap_or(family::ALIVE);
                            swatch(ui, color32(color), 10.0);
                            ui.label(
                                RichText::new(names.get(s).cloned().unwrap_or_else(|| format!("state {s}"))).size(12.5),
                            );
                            ui.label(RichText::new(format_count(*n)).monospace().size(12.5));
                            ui.end_row();
                        }
                    });
                }
                if total == 0 {
                    muted(ui, palette, "No occupied cell in view.");
                }
                muted(ui, palette, "Counted in the displayed region (with a margin around the screen).");
            });
        }
        None => muted(
            ui,
            palette,
            format!(
                "Zoomed out: each texel covers {0} × {0} cells. Zoom in to count cells.",
                view.grid.cells_per_texel
            ),
        ),
    }
}

/// Lines describing a cell of the view.
pub fn inspect(view: &LabView, cell: I64Vec2) -> Vec<String> {
    let info = &view.info;
    let mut lines = vec![format!("Cell ({}, {})", cell.x, cell.y)];
    if let Some(b) = info.bounds()
        && !b.contains(cell.x, cell.y)
    {
        lines.push(match info.family() {
            Family::Wolfram => "Not part of the diagram (no such row or column).".to_string(),
            _ => "Outside the world.".to_string(),
        });
        return lines;
    }
    let Some([state, age, activity, density]) = view.grid.texel_at_cell(cell.x, cell.y) else {
        lines.push("Not in the displayed region yet.".to_string());
        return lines;
    };
    if view.grid.cells_per_texel > 1 {
        lines.push(format!(
            "Zoomed out ({0} × {0} cells): {1:.0} % occupied",
            view.grid.cells_per_texel,
            f64::from(density) / 2.55
        ));
        return lines;
    }
    let names = state_names(info);
    lines.push(format!("State {state}: {}", names.get(usize::from(state)).cloned().unwrap_or_default()));
    if info.family() == Family::Wolfram {
        if let FamilyInfo::Wolfram { generation, .. } = &info.detail {
            lines.push(format!("Row {} of {}", cell.y, generation));
        }
        if activity > 0 {
            lines.push("Differs from the cell above it.".to_string());
        }
        return lines;
    }
    if age > 0 {
        lines.push(format!("Alive for {age} generation{}", if age == 1 { "" } else { "s" }));
    }
    if activity > 0 {
        let ago = (255 - u32::from(activity)) / 16;
        lines.push(if ago == 0 {
            "Changed during the last tick".to_string()
        } else {
            format!("Changed {} ticks ago", ago + 1)
        });
    }
    lines
}

// ---------------------------------------------------------------- overlay

/// Scale bar length: a round number of cells close to `target_px` pixels.
pub fn scale_bar(cells_per_px: f32, target_px: f32) -> Option<(f32, f32)> {
    if !(cells_per_px.is_finite() && cells_per_px > 0.0 && target_px > 0.0) {
        return None;
    }
    let raw = target_px * cells_per_px;
    let base = 10f32.powf(raw.log10().floor());
    let m = raw / base;
    let nice = if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    };
    let cells = (nice * base).max(1.0).round();
    Some((cells, cells / cells_per_px))
}

fn legend_entries(info: &LabInfo) -> Vec<(Rgb, String)> {
    let colors = state_colors(info);
    let names = state_names(info);
    let n = names.len();
    let pick = |s: usize| (colors.get(s).or(colors.last()).copied().unwrap_or(family::ALIVE), names[s].clone());
    if n <= 8 {
        (0..n).map(pick).collect()
    } else {
        // Many dying states: dead, alive, the first and the last dying state.
        vec![pick(0), pick(1), pick(2), pick(n - 1)]
    }
}

/// Colour ramp of a colouring that reads the age or activity channel.
#[derive(Clone, Debug, PartialEq)]
pub struct RampLegend {
    /// Evenly spaced colours of the bar.
    pub stops: Vec<egui::Color32>,
    /// Label of the low end.
    pub low: &'static str,
    /// Label under the middle of the bar.
    pub mid: Option<&'static str>,
    /// Label of the high end.
    pub high: &'static str,
}

/// The colour ramp of the colourings that read the age or activity channel.
pub fn ramp_legend(info: &LabInfo, settings: &ViewSettings) -> Option<RampLegend> {
    let colouring = if settings.colouring.available(info.family()) { settings.colouring } else { Colouring::States };
    let (map, low, mid, high) = match colouring {
        Colouring::States => return None,
        Colouring::Trail | Colouring::Activity if info.family() == Family::Wolfram => {
            (rendering::palette::Colormap::Inferno, "same as above", None, "differs from the row above")
        }
        Colouring::Trail | Colouring::Activity => {
            (rendering::palette::Colormap::Inferno, "16+ ticks ago", None, "this tick")
        }
        Colouring::Age => (rendering::palette::Colormap::Viridis, "1 generation", Some("16"), "255+ (log scale)"),
    };
    let positions: Vec<f32> = if colouring == Colouring::Age {
        AGE_LEGEND_AGES.map(age_ramp_position).to_vec()
    } else {
        (0..16).map(|i| i as f32 / 15.0).collect()
    };
    let stops = positions
        .into_iter()
        .map(|t| {
            let [r, g, b] = map.sample_srgb(t).map(|v| (v * 255.0).round() as u8);
            egui::Color32::from_rgb(r, g, b)
        })
        .collect();
    Some(RampLegend { stops, low, mid, high })
}

/// What a zoomed-out texel (`k` × `k` cells) shows, or `None` at one texel per cell.
pub fn zoom_note(info: &LabInfo, settings: &ViewSettings, k: u32) -> Option<String> {
    if k <= 1 {
        return None;
    }
    let colouring = if settings.colouring.available(info.family()) { settings.colouring } else { Colouring::States };
    Some(if crate::world::shows_density(info, settings, k) {
        let ones = if info.family() == Family::Wolfram { "1s" } else { "live cells" };
        format!("Zoomed out: brightness is the fraction of {ones} in each {k} × {k} block.")
    } else {
        match colouring {
            Colouring::States => {
                format!("Zoomed out: each texel shows the most frequent non-zero state of its {k} × {k} block.")
            }
            Colouring::Age => {
                format!("Zoomed out: each texel shows the mean age of the live cells of its {k} × {k} block.")
            }
            Colouring::Activity => {
                format!("Zoomed out: each texel shows the most recent change in its {k} × {k} block.")
            }
            Colouring::Trail => format!(
                "Zoomed out: each texel shows the most frequent state and the most recent change of its {k} × {k} block."
            ),
        }
    })
}

/// A one-line legend entry: an outlined swatch and a label.
fn compact_chip(ui: &mut Ui, palette: &Palette, color: egui::Color32, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().interact_size.y = 15.0;
        let (rect, _) = ui.allocate_exact_size(vec2(11.0, 11.0), Sense::hover());
        ui.painter().rect_filled(rect, 2.0, color);
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Outside);
        ui.label(RichText::new(label).size(12.0));
    });
}

/// Overlay over the world: family, scenario, rule, time, legend and scale.
#[allow(clippy::too_many_arguments)]
pub(crate) fn overlay_panel(
    InMut(ui): InMut<egui::Ui>,
    (theme, registry): (Res<ThemeSettings>, Res<ModeRegistry>),
    latest: Res<LatestView>,
    loaded: Option<Res<LoadedScenario>>,
    status: Res<SessionStatus>,
    history: Res<MetricsHistory>,
    visible: Res<VisibleCells>,
    (hover, input, settings): (Res<Hover>, Res<WorldInput>, Res<ViewSettings>),
    mut state: ResMut<PanelState>,
    mut inset: ResMut<OverlayInset>,
) {
    let palette = mode_palette(&theme, &registry);
    let (Some(view), Some(loaded)) = (latest.view.clone(), loaded) else { return };
    let info = &view.info;
    let family = info.family();
    let frame = egui::Frame::new()
        .fill(with_alpha(palette.panel, 0.96))
        .stroke(Stroke::new(palette.stroke_width, with_alpha(palette.stroke, 0.9)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_max_width(260.0);
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.horizontal(|ui| {
                let (_, toggle) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
                egui::collapsing_header::paint_default_icon(ui, if state.overlay_folded { 0.0 } else { 1.0 }, &toggle);
                let tip = if state.overlay_folded { "Show the legend, time and scale" } else { "Fold the overlay" };
                if toggle.on_hover_text(tip).clicked() {
                    state.overlay_folded = !state.overlay_folded;
                }
                badge(ui, family.short_label(), palette.accent);
                ui.label(RichText::new(&loaded.name).size(13.0).strong());
            });
            let tick = status.get().map_or(info.tick, |s| s.tick);
            let mut line = format!("{}  ·  {} {}", info.rule_text(), family.tick_name(), format_count(tick));
            if let Some(n) = match &info.detail {
                FamilyInfo::Life { generations_per_tick, .. } if *generations_per_tick > 1 => {
                    Some(*generations_per_tick)
                }
                _ => None,
            } {
                line.push_str(&format!("  ({n} per tick)"));
            }
            ui.label(RichText::new(line).size(12.0).monospace());
            if state.overlay_folded {
                return;
            }
            // Always one line, so the overlay keeps its height when the first sample arrives.
            let (key, label) = family.headline_metric();
            let value = info
                .headline
                .or_else(|| history.latest(key))
                .map_or_else(|| "\u{2014}".to_string(), |v| format_count(v.max(0.0) as u64));
            ui.label(RichText::new(format!("{label}: {value}")).size(12.0).color(palette.text_muted));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 3.0);
                for (rgb, name) in legend_entries(info) {
                    compact_chip(ui, &palette, color32(rgb), &name);
                }
                if family == Family::Ant {
                    compact_chip(ui, &palette, palette.accent, "ant, pointing along its heading");
                }
                if family == Family::Wolfram {
                    compact_chip(ui, &palette, palette.accent, "line under the newest row (time runs down)");
                }
            });
            if let Some(ramp) = ramp_legend(info, &settings) {
                gradient_bar_with_mid(ui, &ramp.stops, 200.0, ramp.low, ramp.mid, ramp.high);
            }
            let k = view.grid.cells_per_texel;
            if let Some(note) = zoom_note(info, &settings, k) {
                muted(ui, &palette, note);
            }
            if let FamilyInfo::Generations { rule, .. } = &info.detail
                && rule.states > 8
            {
                muted(
                    ui,
                    &palette,
                    format!("{} dying states fade from the first to the last colour.", rule.states - 2),
                );
            }
            if let Some(reason) = &info.halted {
                ui.label(RichText::new(format!("Stopped: {reason}")).size(11.5).color(palette.warning));
            }
            if let Some((cells, px)) = scale_bar(visible.cells_per_pixel, 90.0) {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(px.min(200.0), 10.0), Sense::hover());
                    let y = rect.center().y;
                    let stroke = Stroke::new(1.5, palette.text);
                    ui.painter().line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], stroke);
                    for x in [rect.left(), rect.right()] {
                        ui.painter().line_segment([egui::pos2(x, y - 4.0), egui::pos2(x, y + 4.0)], stroke);
                    }
                    let unit = if cells == 1.0 { "cell" } else { "cells" };
                    ui.label(
                        RichText::new(format!("{} {unit}", format_count(cells as u64)))
                            .size(11.5)
                            .color(palette.text_muted),
                    );
                });
            }
        });
    inset.height = frame.response.rect.height() + 12.0;
    hover_readout(ui, &palette, &view, &hover, &input);
}

/// Top-left corner of the hover readout: below-right of the pointer,
/// flipped to the other side where it would leave the world viewport.
pub fn readout_position(cursor: egui::Pos2, size: egui::Vec2, viewport: Option<egui::Rect>) -> egui::Pos2 {
    const OFFSET: f32 = 16.0;
    let margin = vec2(7.0, 5.0);
    let mut pos = cursor + vec2(OFFSET, OFFSET);
    if let Some(v) = viewport {
        if pos.x + size.x + margin.x > v.max.x {
            pos.x = cursor.x - OFFSET - size.x;
        }
        if pos.y + size.y + margin.y > v.max.y {
            pos.y = cursor.y - OFFSET - size.y;
        }
    }
    pos
}

fn hover_readout(ui: &Ui, palette: &Palette, view: &LabView, hover: &Hover, input: &WorldInput) {
    let (Some(cell), Some(cursor)) = (hover.cell, input.cursor) else { return };
    let text = inspect(view, cell).join("\n");
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("ca_hover_readout")));
    let galley = painter.layout_no_wrap(text, egui::FontId::monospace(11.5), palette.text);
    let viewport =
        input.viewport.map(|v| egui::Rect::from_min_max(egui::pos2(v.min.x, v.min.y), egui::pos2(v.max.x, v.max.y)));
    let pos = readout_position(egui::pos2(cursor.x, cursor.y), galley.size(), viewport);
    let rect = egui::Rect::from_min_size(pos, galley.size()).expand2(vec2(7.0, 5.0));
    painter.rect(
        rect,
        5.0,
        with_alpha(palette.panel, 0.92),
        Stroke::new(1.0, with_alpha(palette.stroke, 0.9)),
        egui::StrokeKind::Inside,
    );
    painter.galley(pos, galley, palette.text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::LabSim;
    use simulation_core::Simulation;

    fn sim(id: &str) -> LabSim {
        crate::sim::tests::lab(id)
    }

    #[test]
    fn state_names_follow_the_model() {
        assert_eq!(
            state_names(&sim("brians-brain").info()),
            vec!["ready (dead)", "firing (alive)", "refractory (dying)"]
        );
        let ant = state_names(&sim("turmite-llrr").info());
        assert_eq!(
            ant,
            vec!["colour 0 · turn left", "colour 1 · turn left", "colour 2 · turn right", "colour 3 · turn right"]
        );
        assert_eq!(state_names(&sim("wireworld-circuits").info())[3], "conductor");
        assert_eq!(legend_entries(&sim("star-wars").info()).len(), 4);
    }

    #[test]
    fn census_counts_exact_cells_only() {
        let s = sim("wireworld-circuits");
        let view =
            s.view(&cellular_automata::GridViewRequest { x0: 0, y0: 0, x1: 63, y1: 71, max_width: 64, max_height: 72 });
        let counts = census(&view).expect("exact view");
        let m = s.metrics();
        assert_eq!(counts[1] as f64, m.get("heads").unwrap());
        assert_eq!(counts[2] as f64, m.get("tails").unwrap());
        assert_eq!(counts[3] as f64, m.get("conductors").unwrap());
        let far =
            s.view(&cellular_automata::GridViewRequest { x0: 0, y0: 0, x1: 63, y1: 71, max_width: 8, max_height: 8 });
        assert!(census(&far).is_none());
    }

    #[test]
    fn rule_facts_compare_with_conway() {
        let (facts, notes) = rule_facts(&sim("brians-brain").info());
        assert_eq!(facts[0].1, "2");
        assert_eq!(facts[1].1, "none");
        assert!(notes.iter().any(|n| n.contains("spends 1 generation dying")), "{notes:?}");
        let (_, notes) = rule_facts(&sim("day-and-night-rocket").info());
        assert!(notes.iter().any(|n| n.contains("Swapping live and dead")));
        let (facts, _) = rule_facts(&sim("turmite-llrr").info());
        assert_eq!(facts[1].1, "0: L, 1: L, 2: R, 3: R");
        let (facts, _) = rule_facts(&sim("wolfram-rule-110").info());
        assert_eq!(facts[0].1, "110 = 01101110");
    }

    #[test]
    fn inspector_describes_cells() {
        let s = sim("wireworld-circuits");
        let view =
            s.view(&cellular_automata::GridViewRequest { x0: 0, y0: 0, x1: 63, y1: 71, max_width: 64, max_height: 72 });
        // The diode's electron: tail at (2, 3), head at (3, 3) (pattern at (2, 2), row 1 = "BA2C.6C").
        assert_eq!(inspect(&view, I64Vec2::new(3, 3))[1], "State 1: electron head");
        assert_eq!(inspect(&view, I64Vec2::new(2, 3))[1], "State 2: electron tail");
        assert_eq!(inspect(&view, I64Vec2::new(100, 3))[1], "Outside the world.");
        assert_eq!(world_line(&view.info), "64 × 72 bounded plane");
    }

    #[test]
    fn zoom_notes_describe_the_aggregation() {
        let ww = sim("wireworld-circuits").info();
        let life = sim("seeds").info();
        let eca = sim("wolfram-rule-30").info();
        let states = ViewSettings::default();
        assert_eq!(zoom_note(&life, &states, 1), None);
        assert!(zoom_note(&life, &states, 4).is_some_and(|n| n.contains("fraction of live cells in each 4 × 4")));
        assert!(zoom_note(&eca, &states, 2).is_some_and(|n| n.contains("fraction of 1s")));
        assert!(zoom_note(&ww, &states, 2).is_some_and(|n| n.contains("most frequent non-zero state")));
        let age = ViewSettings { colouring: Colouring::Age, ..default() };
        assert!(zoom_note(&life, &age, 2).is_some_and(|n| n.contains("mean age")));
        let heat = ViewSettings { colouring: Colouring::Activity, ..default() };
        assert!(zoom_note(&life, &heat, 2).is_some_and(|n| n.contains("most recent change") && !n.contains("age")));
    }

    /// The Age legend is drawn on the shader's logarithmic scale: where the
    /// bar places an age, it shows the colour the shader gives a live cell
    /// of that age (not the ramp from 0, whose dark start no cell has).
    #[test]
    fn age_legend_matches_the_shader_scale() {
        let life = sim("seeds").info();
        let settings = ViewSettings { colouring: Colouring::Age, ..default() };
        let ramp = ramp_legend(&life, &settings).expect("the Age colouring has a ramp");
        let map = crate::world::appearance(&life, &settings, 1).ramp;
        let shader = |age: u8| {
            let [r, g, b] = map.sample_srgb(age_ramp_position(age)).map(|v| (v * 255.0).round() as u8);
            egui::Color32::from_rgb(r, g, b)
        };
        let stops = &ramp.stops;
        let ends = [stops[0], stops[stops.len() / 2], stops[stops.len() - 1]];
        assert_eq!(ends, [shader(1), shader(16), shader(255)], "1, 16 and 255 generations: start, middle, end");
        assert_eq!((ramp.low, ramp.mid, ramp.high), ("1 generation", Some("16"), "255+ (log scale)"));
        // Between stops the bar interpolates linearly, which bends a little
        // less than the colormap.
        for age in [2, 3, 7, 40, 100, 200] {
            let bar = interface::widgets::color_ramp(f32::from(age).ln() / 255f32.ln(), stops);
            let close = bar.to_array().iter().zip(shader(age).to_array()).all(|(a, b)| a.abs_diff(b) <= 8);
            assert!(close, "age {age}: legend {bar:?}, shader {:?}", shader(age));
        }
        let trail = ViewSettings { colouring: Colouring::Trail, ..default() };
        assert!(ramp_legend(&life, &trail).is_some_and(|r| r.mid.is_none() && r.stops.len() == 16));
    }

    #[test]
    fn plot_axes_get_nice_labels() {
        let marks = nice_marks((0.0, 834.0), 5.0);
        assert_eq!(marks.iter().map(|m| m.value).collect::<Vec<_>>(), vec![0.0, 200.0, 400.0, 600.0, 800.0]);
        assert!(marks.iter().all(|m| m.step_size == 200.0));
        let marks = nice_marks((-0.01, 0.37), 5.0);
        assert_eq!(
            marks.iter().map(|m| axis_label(m.value, m.step_size)).collect::<Vec<_>>(),
            ["0", "0.1", "0.2", "0.3"]
        );
        assert_eq!(axis_label(20_000.0, 10_000.0), "20,000");
        assert_eq!(axis_label(250_000.0, 50_000.0), "250k");
        assert!(nice_marks((1.0, 1.0), 4.0).is_empty() && nice_marks((0.0, f64::NAN), 4.0).is_empty());
    }

    #[test]
    fn scale_bar_rounds() {
        assert_eq!(scale_bar(1.0, 90.0), Some((100.0, 100.0)));
        assert_eq!(scale_bar(0.1, 90.0), Some((10.0, 100.0)));
        assert_eq!(scale_bar(0.02, 90.0), Some((2.0, 100.0)));
        assert!(scale_bar(0.0, 90.0).is_none());
    }

    /// Every panel draws for every family and every tool, without a
    /// session and with one (headless egui).
    #[test]
    fn panels_render_headless_for_every_family_and_tool() {
        use crate::session::{Catalog, LoadedScenario};
        use crate::world::{CameraFit, OverlayInset};
        use std::sync::Arc;
        let mut app = App::new();
        app.init_resource::<ThemeSettings>()
            .init_resource::<ModeRegistry>()
            .init_resource::<ToolSettings>()
            .init_resource::<ViewSettings>()
            .init_resource::<LoadRequests>()
            .init_resource::<CameraFit>()
            .init_resource::<LatestView>()
            .init_resource::<Hover>()
            .init_resource::<PanelState>()
            .init_resource::<OverlayInset>()
            .init_resource::<SessionStatus>()
            .init_resource::<MetricsHistory>()
            .init_resource::<VisibleCells>()
            .init_resource::<WorldInput>()
            .insert_resource(Circuits::load(None));
        let systems = [
            app.world_mut().register_system(tools_panel),
            app.world_mut().register_system(properties_panel),
            app.world_mut().register_system(overlay_panel),
        ];
        let ctx = egui::Context::default();
        let run = |app: &mut App| {
            let world = app.world_mut();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                for id in systems {
                    world.run_system_with(id, ui).expect("panel system runs");
                }
            });
            output.textures_delta.clear();
        };
        run(&mut app);
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let catalog = Catalog::load(Some(&assets));
        app.insert_resource(catalog.clone());
        for id in ["seeds", "star-wars", "wireworld-circuits", "turmite-four-ants", "wolfram-rule-184"] {
            let sim = catalog.build(id).unwrap();
            let request =
                cellular_automata::GridViewRequest { x0: 0, y0: 0, x1: 99, y1: 99, max_width: 100, max_height: 100 };
            let view = sim.view(&request);
            let family = view.info.family();
            app.insert_resource(LatestView { seq: 1, view: Some(Arc::new(view)), ..default() });
            app.insert_resource(LoadedScenario {
                id: id.to_string(),
                name: id.to_string(),
                description: "d".into(),
                reference: "r".into(),
                family,
            });
            app.world_mut().resource_mut::<Hover>().cell = Some(I64Vec2::new(5, 5));
            for colouring in Colouring::ALL {
                app.world_mut().resource_mut::<ViewSettings>().colouring = colouring;
                run(&mut app);
            }
            for tool in Tool::for_family(family) {
                app.world_mut().resource_mut::<ToolSettings>().tool = *tool;
                run(&mut app);
            }
        }
        assert!(app.world().resource::<LoadRequests>().0.is_empty(), "drawing alone loads nothing");
        assert!(app.world().resource::<OverlayInset>().height > 12.0, "the overlay measured itself");
    }

    #[test]
    fn readouts_stay_in_the_viewport() {
        let viewport = Some(egui::Rect::from_min_max(egui::pos2(300.0, 40.0), egui::pos2(1200.0, 900.0)));
        let size = vec2(120.0, 40.0);
        assert_eq!(readout_position(egui::pos2(500.0, 300.0), size, viewport), egui::pos2(516.0, 316.0));
        assert_eq!(readout_position(egui::pos2(1150.0, 880.0), size, viewport), egui::pos2(1014.0, 824.0));
    }
}
