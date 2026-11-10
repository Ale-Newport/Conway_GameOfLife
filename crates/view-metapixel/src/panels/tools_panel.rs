//! The Tools panel: scenario summary, pointer tool, layers, Hashlife time
//! control, macro rule programming, grid and soup, view options and the
//! mode's shortcuts.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui::{self, RichText, Ui, vec2};
use interface::format::{format_bytes, format_compact, format_count, format_ms};
use interface::{ModeRegistry, ModeScenarios, Palette, SessionStatus, ThemeSettings};
use metapixel::spec::{Direction, PERIOD};
use metapixel::{MAX_GRID_SIDE, MetapixelCommand};
use simulation_core::{Control, ParamValue};

use super::{PanelSections, card, collapsible, kv_grid, notice, phase_bar, section, segmented};
use crate::MODE;
use crate::geometry::GridShape;
use crate::macro_layer::{LayerMode, MacroSource, live_available};
use crate::rule_editor::{RuleDraft, neighbourhood_presets, rule_presets};
use crate::session::{Catalog, Lifecycle, LoadRequests, MpSession, SimData, Tiles, TilesState, jump_to_next_readout};
use crate::timing::{
    generations_per_second, generations_to_next_readout, jumps_text, metagen_fraction_text, stage_label, step_label,
    step_options, ticks_to_next_readout,
};
use crate::tools::{MODE_SHORTCUTS, Tool, ToolState, current_step, rule_table_rect, stamp_patterns, stamp_size};
use crate::world::{COLORMAPS, CameraFit, LayerState, ViewSettings, current_shape, mode_palette};

/// Resources of the Tools panel.
#[derive(SystemParam)]
pub(crate) struct ToolsCtx<'w> {
    theme: Res<'w, ThemeSettings>,
    registry: Res<'w, ModeRegistry>,
    status: Res<'w, SessionStatus>,
    data: ResMut<'w, SimData>,
    session: Option<Res<'w, MpSession>>,
    tools: ResMut<'w, ToolState>,
    settings: ResMut<'w, ViewSettings>,
    fit: ResMut<'w, CameraFit>,
    lifecycle: ResMut<'w, Lifecycle>,
    catalog: Res<'w, Catalog>,
    tiles: Res<'w, Tiles>,
    layer: Res<'w, LayerState>,
    scenarios: Res<'w, ModeScenarios>,
    requests: ResMut<'w, LoadRequests>,
    sections: ResMut<'w, PanelSections>,
}

/// Rule and grid drafts, re-derived when the model's pending values change.
#[derive(Default)]
pub(crate) struct Drafts {
    rule: RuleDraft,
    rule_source: Option<(String, String)>,
    size: (u32, u32, u64),
    size_source: Option<(i64, i64, i64)>,
}

/// Left panel.
pub(crate) fn tools_panel(InMut(ui): InMut<egui::Ui>, mut ctx: ToolsCtx, mut drafts: Local<Drafts>) {
    let palette = mode_palette(&ctx.theme, &ctx.registry);
    ui.spacing_mut().slider_width = (ui.available_width() - 150.0).clamp(60.0, 200.0);
    scenario_section(ui, &palette, &mut ctx);
    if ctx.session.is_none() {
        return;
    }
    tool_section(ui, &palette, &mut ctx);
    layer_section(ui, &palette, &mut ctx);
    let mut sections = *ctx.sections;
    collapsible(ui, &palette, "Time (Hashlife)", &mut sections.time, |ui| time_section(ui, &palette, &mut ctx));
    collapsible(ui, &palette, "Macro rule", &mut sections.rule, |ui| rule_section(ui, &palette, &mut ctx, &mut drafts));
    collapsible(ui, &palette, "Grid and soup", &mut sections.grid, |ui| {
        grid_section(ui, &palette, &mut ctx, &mut drafts)
    });
    collapsible(ui, &palette, "View", &mut sections.view, |ui| view_section(ui, &palette, &mut ctx));
    collapsible(ui, &palette, "Shortcuts", &mut sections.shortcuts, |ui| shortcuts_section(ui, &palette));
    ctx.sections.set_if_neq(sections);
}

fn scenario_section(ui: &mut Ui, palette: &Palette, ctx: &mut ToolsCtx) {
    section(ui, palette, "Construction");
    match &ctx.tiles.state {
        TilesState::Idle | TilesState::Loading(_) => {
            card(ui, palette, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Reading Brice Due's OTCA tiles…").size(12.5));
                });
                ui.label(RichText::new(ctx.tiles.dir.display().to_string()).size(11.0).color(palette.text_muted));
            });
            return;
        }
        TilesState::Failed(e) => {
            notice(
                ui,
                palette,
                palette.error,
                "The OTCA tiles could not be loaded, so no construction can be built. Brice Due's \
                 OTCAmetapixel-OFF.rle and OTCAmetapixel-ON.rle must be in this folder:",
            );
            ui.add(egui::Label::new(RichText::new(ctx.tiles.dir.display().to_string()).size(11.0).monospace()).wrap())
                .on_hover_text(e.as_str());
            return;
        }
        TilesState::Ready(_) => {}
    }
    if let Some(error) = &ctx.lifecycle.error {
        notice(ui, palette, palette.error, error);
    }
    let selected = ctx.scenarios.get(MODE).and_then(|l| l.selected.clone());
    let scenario = selected.as_deref().and_then(|id| ctx.catalog.get(id));
    card(ui, palette, |ui| {
        if let Some(building) = &ctx.lifecycle.building {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(format!("Building {}…", ctx.catalog.name_of(building))).size(12.5));
            });
        }
        if let Some(s) = scenario {
            let edited = match (&ctx.lifecycle.baseline, &ctx.data.info) {
                (Some((pattern, config)), Some(info)) => info.initial != *pattern || info.active != *config,
                _ => false,
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(&s.name).size(14.5).strong());
                if edited {
                    interface::widgets::badge(ui, "EDITED", palette.warning).on_hover_text(
                        "The generation-0 pattern, rule, neighbourhood or size differ from the scenario; pick the \
                         scenario again to reload it",
                    );
                }
            });
        }
        if let Some(info) = &ctx.data.info {
            let shape = GridShape::new(info.active.width, info.active.height);
            let b = shape.bounds();
            ui.label(
                RichText::new(format!(
                    "{} × {} metacells · macro rule {} · {}\n{} live B3/S23 cells in a {} × {} cell plane",
                    info.active.width,
                    info.active.height,
                    info.active.programming.rule,
                    info.active.programming.neighbourhood,
                    format_count(info.population),
                    format_count(b.width()),
                    format_count(b.height())
                ))
                .size(12.0)
                .color(palette.text_muted),
            );
        }
        if let Some(s) = scenario {
            egui::CollapsingHeader::new(RichText::new("About this scenario").size(12.5))
                .id_salt("mp_about_scenario")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(RichText::new(crate::session::displayable(&s.description)).size(12.0));
                    for note in &s.notes {
                        ui.label(RichText::new(crate::session::displayable(note)).size(11.5).color(palette.text_muted));
                    }
                });
        }
        ui.horizontal_wrapped(|ui| {
            for s in &ctx.catalog.scenarios {
                let on = selected.as_deref() == Some(s.id.as_str());
                let short = short_name(&s.id);
                if ui
                    .add(egui::Button::new(RichText::new(short).size(11.5)).selected(on))
                    .on_hover_text(&s.description)
                    .clicked()
                {
                    ctx.requests.0.push(s.id.clone());
                }
            }
        });
    });
    if let Some(reason) = ctx.data.info.as_ref().and_then(|i| i.halt.clone()) {
        notice(ui, palette, palette.error, &format!("The simulation halted: {reason}"));
    }
}

/// Short button label of a scenario id.
fn short_name(id: &str) -> String {
    let base = id.trim_start_matches("metapixel.");
    match base {
        "parity-von-neumann" => "parity".to_string(),
        "b0-single-tile" => "B0 tile".to_string(),
        other => other.to_string(),
    }
}

fn tool_section(ui: &mut Ui, palette: &Palette, ctx: &mut ToolsCtx) {
    section(ui, palette, "Tool");
    let items: Vec<(&str, &str, bool)> =
        Tool::ALL.iter().map(|t| (t.label(), t.help(), ctx.tools.tool == *t)).collect();
    if let Some(i) = segmented(ui, &items, 3) {
        ctx.tools.tool = Tool::ALL[i];
    }
    let t = ctx.data.tick();
    match ctx.tools.tool {
        Tool::Inspect => {
            ui.label(
                RichText::new("Hover a tile to read it; click to pin it in the inspector (Properties tab).")
                    .size(12.0)
                    .color(palette.text_muted),
            );
        }
        Tool::Draw | Tool::Stamp => {
            ui.label(
                RichText::new(
                    "Edits change the generation-0 macro pattern (shown on the macro layer while this tool is \
                     active) and rebuild the construction at t = 0. Each click or drag is one undo step.",
                )
                .size(12.0)
                .color(palette.text_muted),
            );
            let latest = ctx.data.view.as_ref().and_then(|v| v.macro_grid.as_ref()).and_then(|m| m.exact_metagen);
            if t > 0
                && let Some(n) = latest
            {
                let ambiguous =
                    ctx.data.view.as_ref().and_then(|v| v.macro_grid.as_ref()).map_or(0, |m| m.ambiguous_count);
                let button =
                    egui::Button::new(RichText::new(format!("Continue from the metagen {n} readout")).size(12.5));
                let response = ui.add_enabled(ambiguous == 0, button).on_hover_text(
                    "Makes the latest exact readout the new generation-0 pattern and rebuilds, so edits apply to what \
                     the macro layer showed",
                );
                if response.clicked()
                    && let Some(session) = &ctx.session
                {
                    session.edit(MetapixelCommand::RestartFromLatestReadout);
                }
            }
            if ctx.tools.tool == Tool::Stamp {
                let patterns = stamp_patterns();
                let current = patterns.get(ctx.tools.stamp).map_or("—", |p| p.name);
                egui::ComboBox::from_id_salt("mp_stamp_pattern")
                    .width(ui.available_width() - 8.0)
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (i, p) in patterns.iter().enumerate() {
                            let size = stamp_size(i).map_or(String::new(), |(w, h)| format!(" ({w}×{h})"));
                            ui.selectable_value(&mut ctx.tools.stamp, i, format!("{}{size}", p.name))
                                .on_hover_text(p.description);
                        }
                    });
                if let (Some((w, h)), Some(shape)) = (stamp_size(ctx.tools.stamp), current_shape(&ctx.data)) {
                    let fits = w <= u64::from(shape.columns) && h <= u64::from(shape.rows);
                    let text = format!("Needs {w} × {h} metacells; the grid is {} × {}.", shape.columns, shape.rows);
                    ui.label(RichText::new(text).size(11.5).color(if fits {
                        palette.text_muted
                    } else {
                        palette.warning
                    }));
                }
            }
        }
    }
}

fn layer_section(ui: &mut Ui, palette: &Palette, ctx: &mut ToolsCtx) {
    section(ui, palette, "Layer");
    let mut settings = ctx.settings.clone();
    let items: Vec<(&str, &str, bool)> =
        LayerMode::ALL.iter().map(|m| (m.label(), m.help(), settings.layer == *m)).collect();
    if let Some(i) = segmented(ui, &items, 3) {
        settings.layer = LayerMode::ALL[i];
    }
    ui.label(RichText::new("Macro layer shows").size(11.5).color(palette.text_muted));
    let items: Vec<(&str, &str, bool)> =
        MacroSource::ALL.iter().map(|m| (m.label(), m.help(), settings.source == *m)).collect();
    if let Some(i) = segmented(ui, &items, 3) {
        settings.source = MacroSource::ALL[i];
    }
    let shape = current_shape(&ctx.data);
    if settings.source == MacroSource::Live && shape.is_some_and(|s| !live_available(s.tile_count())) {
        notice(
            ui,
            palette,
            palette.warning,
            "Live core counts are computed for grids of up to 1,024 tiles; this grid shows the latest readout instead.",
        );
    }
    if settings.source == MacroSource::Prediction {
        notice(ui, palette, palette.warning, metapixel::ACCELERATED_LABEL);
    }
    ui.add_space(2.0);
    let on_screen = ctx.layer.label();
    ui.label(
        RichText::new(format!(
            "On screen: {on_screen} · one tile = {} px",
            if ctx.layer.pixels_per_tile.is_finite() {
                format!("{:.0}", ctx.layer.pixels_per_tile)
            } else {
                "—".into()
            }
        ))
        .size(11.5)
        .color(palette.text_muted),
    );
    ctx.settings.set_if_neq(settings);
}

fn time_section(ui: &mut Ui, palette: &Palette, ctx: &mut ToolsCtx) {
    let Some(session) = ctx.session.as_deref().cloned() else { return };
    let Some(view) = ctx.data.view.clone() else { return };
    let phase = view.phase;
    let step = current_step(&ctx.status).unwrap_or_default();
    card(ui, palette, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Per tick").size(12.0).color(palette.text_muted));
            egui::ComboBox::from_id_salt("mp_step")
                .width(ui.available_width() - 4.0)
                .selected_text(format!("{} gens", step_label(step)))
                .show_ui(ui, |ui| {
                    for option in step_options() {
                        let text = format!("{} ({})", step_label(option), metagen_fraction_text(option.generations()));
                        if ui.selectable_label(option == step, text).clicked() && option != step {
                            session.send(Control::SetParam("step".into(), ParamValue::Text(option.value())));
                        }
                    }
                });
        });
        let gens = step.generations();
        ui.label(
            RichText::new(format!(
                "{} per tick: {} generations, advanced by Hashlife as {}. A tick that crosses a readout time \
                 t = k·{} pauses there to read every tile at exactly that generation.",
                metagen_fraction_text(gens),
                format_count(gens),
                jumps_text(gens),
                format_count(PERIOD)
            ))
            .size(11.5)
            .color(palette.text_muted),
        );
        ui.add_space(4.0);
        let width = ui.available_width();
        phase_bar(ui, palette, phase.phase, phase.generation < PERIOD, width, 12.0);
        ui.label(
            RichText::new(format!(
                "t mod P = {} / {} ({:.1}%) · {}",
                format_count(phase.phase),
                format_count(PERIOD),
                100.0 * phase.phase as f64 / PERIOD as f64,
                stage_label(phase.stage)
            ))
            .size(12.0),
        )
        .on_hover_text(crate::timing::stage_help(phase.stage));
        let to_next = generations_to_next_readout(phase.generation);
        ui.label(
            RichText::new(format!(
                "Next readout: metagen {} at t = {} ({} generations, {} tick{})",
                format_count(phase.next_readout_generation / PERIOD - 1),
                format_count(phase.next_readout_generation),
                format_count(to_next),
                ticks_to_next_readout(phase.generation, step),
                if ticks_to_next_readout(phase.generation, step) == 1 { "" } else { "s" }
            ))
            .size(12.0)
            .color(palette.text_muted),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let busy = ctx.data.jump.is_some();
            let label = if busy { "Advancing…" } else { "Advance to next readout (J)" };
            let response = ui
                .add_enabled(!busy, egui::Button::new(RichText::new(label).size(12.5)))
                .on_hover_text("Pauses and runs exactly to the next multiple of P on the simulation thread");
            if response.clicked() {
                jump_to_next_readout(&session, &mut ctx.data);
            }
            if busy {
                ui.spinner();
            }
        });
        // Cost of the last ticks.
        if let Some(status) = ctx.status.get() {
            let ms = |v: f64| if v > 0.0 { format_ms(v) } else { "— (no tick yet)".to_string() };
            let mut rows = vec![("Last tick", ms(status.last_step_ms)), ("Average tick", ms(status.avg_step_ms))];
            if let Some(gps) = generations_per_second(gens, status.avg_step_ms) {
                rows.push(("Hashlife speed", format!("{} generations/s", format_compact(gps.round()))));
            }
            if status.playing {
                rows.push(("Measured", format!("{} generations/s", format_compact(status.measured_gps.round()))));
            }
            if let Some(secs) = ctx.data.last_jump_seconds {
                rows.push(("Last jump", format_ms(secs * 1000.0)));
            }
            if let Some(info) = &ctx.data.info {
                rows.push((
                    "Hashlife nodes",
                    format!(
                        "{} of {} ({})",
                        format_count(info.node_count as u64),
                        format_count(info.node_limit as u64),
                        format_bytes(info.memory_bytes as u64)
                    ),
                ));
            }
            kv_grid(ui, "mp_cost", palette, &rows);
            if let Some(waited) = ctx.data.busy_for(0.4) {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(format!("Simulation thread busy with a Hashlife jump ({waited:.1} s)"))
                            .size(11.5)
                            .color(palette.text_muted),
                    );
                });
            }
        }
    });
}

fn rule_section(ui: &mut Ui, palette: &Palette, ctx: &mut ToolsCtx, drafts: &mut Drafts) {
    let Some(status) = ctx.status.get() else { return };
    let rule_text = status.params.str_or("rule", "B3/S23").to_string();
    let nbhd_text = status.params.str_or("neighbourhood", "moore").to_string();
    let source = (rule_text.clone(), nbhd_text.clone());
    if drafts.rule_source.as_ref() != Some(&source) {
        if let Some(d) = RuleDraft::from_texts(&rule_text, &nbhd_text) {
            drafts.rule = d;
        }
        drafts.rule_source = Some(source);
    }
    let active = ctx.data.info.as_ref().map(|i| i.active.programming);
    card(ui, palette, |ui| {
        let draft = &mut drafts.rule;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("mp_rule_preset")
                .width(ui.available_width() - 4.0)
                .selected_text(match draft.programming().rule.name() {
                    Some(name) => format!("{} · {name}", draft.rule_text()),
                    None => draft.rule_text(),
                })
                .show_ui(ui, |ui| {
                    for (name, rule) in rule_presets() {
                        if ui.selectable_label(draft.programming().rule == rule, format!("{rule} · {name}")).clicked()
                        {
                            draft.set_rule(rule);
                        }
                    }
                });
        });
        ui.add_space(2.0);
        for (letter, label, digits) in
            [('B', "Born with", &mut draft.birth), ('S', "Survives with", &mut draft.survive)]
        {
            ui.label(RichText::new(label).size(11.5).color(palette.text_muted));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let w = ((ui.available_width() - 16.0) / 9.0).clamp(18.0, 30.0);
                for (d, on) in digits.iter_mut().enumerate() {
                    let button = egui::Button::new(RichText::new(d.to_string()).size(12.5).monospace())
                        .selected(*on)
                        .min_size(vec2(w, 22.0));
                    if ui
                        .add(button)
                        .on_hover_text(format!(
                            "{letter}{d}: {} this neighbour count",
                            if *on { "remove" } else { "add" }
                        ))
                        .clicked()
                    {
                        *on = !*on;
                    }
                }
            });
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Neighbourhood (directions every tile hears)").size(11.5).color(palette.text_muted));
        ui.horizontal(|ui| {
            compass(ui, palette, &mut draft.listens);
            ui.vertical(|ui| {
                for (name, tip, n) in neighbourhood_presets() {
                    let on = draft.neighbourhood() == n;
                    if ui
                        .add(egui::Button::new(RichText::new(name).size(12.0)).selected(on))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        draft.listens = Direction::ALL.map(|d| n.listens(d));
                    }
                }
                ui.label(RichText::new(format!("{} heard", draft.max_count())).size(11.5).color(palette.text_muted));
            });
        });
        let unreachable = draft.unreachable_digits();
        if !unreachable.is_empty() {
            notice(
                ui,
                palette,
                palette.warning,
                &format!(
                    "{} can never apply: a tile hears at most {} neighbours.",
                    unreachable.join(", "),
                    draft.max_count()
                ),
            );
        }
        if draft.birth[0] {
            ui.label(
                RichText::new("B0: OFF tiles inside the grid are born from nothing; tiles outside never exist.")
                    .size(11.5)
                    .color(palette.text_muted),
            );
        }
        let draft_rule = draft.rule_text();
        let draft_nbhd = draft.neighbourhood_text();
        let changed = draft_rule != rule_text || draft_nbhd != nbhd_text;
        let pending =
            active.is_some_and(|a| a.rule.to_string() != rule_text || a.neighbourhood.to_string() != nbhd_text);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let apply = ui
                .add_enabled(changed || pending, egui::Button::new(RichText::new("Program and rebuild").size(12.5)))
                .on_hover_text("Programs every tile with this rule and neighbourhood and rebuilds the construction at t = 0 (undoable)");
            if apply.clicked()
                && let Some(session) = &ctx.session
            {
                session.send(Control::SetParam("rule".into(), ParamValue::Text(draft_rule.clone())));
                session.send(Control::SetParam("neighbourhood".into(), ParamValue::Text(draft_nbhd.clone())));
                session.send(Control::Reset);
            }
            if changed && ui.button(RichText::new("Revert").size(12.5)).clicked() {
                drafts_revert(draft, &rule_text, &nbhd_text);
            }
        });
        if let Some(a) = active {
            ui.label(
                RichText::new(format!("Running: {} · {}", a.rule, a.neighbourhood))
                    .size(11.5)
                    .color(palette.text_muted),
            );
        }
        if pending && !changed {
            ui.label(
                RichText::new(format!("Pending: {rule_text} · {nbhd_text} (applies at the next rebuild or Reset)"))
                    .size(11.5)
                    .color(palette.warning),
            );
        }
    });
}

fn drafts_revert(draft: &mut RuleDraft, rule: &str, neighbourhood: &str) {
    if let Some(d) = RuleDraft::from_texts(rule, neighbourhood) {
        *draft = d;
    }
}

/// 3×3 compass of the eight directions around the tile.
fn compass(ui: &mut Ui, palette: &Palette, listens: &mut [bool; 8]) {
    let layout = [
        [Some(Direction::NW), Some(Direction::N), Some(Direction::NE)],
        [Some(Direction::W), None, Some(Direction::E)],
        [Some(Direction::SW), Some(Direction::S), Some(Direction::SE)],
    ];
    egui::Grid::new("mp_compass").striped(false).spacing(vec2(2.0, 2.0)).show(ui, |ui| {
        for row in layout {
            for cell in row {
                match cell {
                    Some(d) => {
                        let on = listens[d.index()];
                        let button = egui::Button::new(RichText::new(d.name()).size(11.0))
                            .selected(on)
                            .min_size(vec2(34.0, 22.0));
                        let tip = if d.is_corner() {
                            format!(
                                "{}: {} (removing the {} corner LWSS fuse silences it)",
                                d.name(),
                                if on { "heard" } else { "deaf" },
                                d.fuse_silencing().name()
                            )
                        } else {
                            format!(
                                "{}: {} (removing the {} edge block silences it)",
                                d.name(),
                                if on { "heard" } else { "deaf" },
                                d.name()
                            )
                        };
                        if ui.add(button).on_hover_text(tip).clicked() {
                            listens[d.index()] = !on;
                        }
                    }
                    None => {
                        let (rect, _) = ui.allocate_exact_size(vec2(34.0, 22.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect.shrink(3.0), 3.0, palette.accent);
                    }
                }
            }
            ui.end_row();
        }
    });
}

fn grid_section(ui: &mut Ui, palette: &Palette, ctx: &mut ToolsCtx, drafts: &mut Drafts) {
    let Some(status) = ctx.status.get() else { return };
    let source = (status.params.i64_or("width", 3), status.params.i64_or("height", 3), status.params.i64_or("seed", 1));
    if drafts.size_source != Some(source) {
        drafts.size = (source.0.max(1) as u32, source.1.max(1) as u32, source.2.max(0) as u64);
        drafts.size_source = Some(source);
    }
    let session = ctx.session.as_deref().cloned();
    card(ui, palette, |ui| {
        let (w, h, seed) = &mut drafts.size;
        egui::Grid::new("mp_grid_size").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
            ui.label(RichText::new("Width").size(12.0));
            ui.add(egui::DragValue::new(w).range(1..=MAX_GRID_SIDE).suffix(" tiles"));
            ui.end_row();
            ui.label(RichText::new("Height").size(12.0));
            ui.add(egui::DragValue::new(h).range(1..=MAX_GRID_SIDE).suffix(" tiles"));
            ui.end_row();
            ui.label(RichText::new("Seed").size(12.0));
            ui.add(egui::DragValue::new(seed).range(0..=i64::MAX as u64));
            ui.end_row();
        });
        let cells = u64::from(*w) * 2048;
        ui.label(
            RichText::new(format!(
                "{} tiles = {} × {} physical cells. The pattern is kept at the top-left.",
                format_count(u64::from(*w) * u64::from(*h)),
                format_count(cells),
                format_count(u64::from(*h) * 2048)
            ))
            .size(11.5)
            .color(palette.text_muted),
        );
        let changed = (i64::from(*w), i64::from(*h), *seed as i64) != source;
        if ui
            .add_enabled(changed, egui::Button::new(RichText::new("Apply size and seed").size(12.5)))
            .on_hover_text("Rebuilds the construction at t = 0 with this size and seed (undoable)")
            .clicked()
            && let Some(session) = &session
        {
            session.send(Control::SetParam("width".into(), ParamValue::Int(i64::from(*w))));
            session.send(Control::SetParam("height".into(), ParamValue::Int(i64::from(*h))));
            session.send(Control::SetParam("seed".into(), ParamValue::Int(*seed as i64)));
            session.send(Control::Reset);
        }
        ui.separator();
        let mut density = ctx.lifecycle.density_value();
        if ui.add(egui::Slider::new(&mut density, 0.0..=1.0).text("ON density").fixed_decimals(2)).changed() {
            ctx.lifecycle.set_density(density);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(RichText::new("Random soup").size(12.5))
                .on_hover_text(
                    "Each tile ON with this probability, drawn from the model's seeded generator; rebuilds at t = 0",
                )
                .clicked()
                && let Some(session) = &session
            {
                session.edit(MetapixelCommand::Randomize { density: f64::from(density) });
            }
            if ui
                .button(RichText::new("All OFF").size(12.5))
                .on_hover_text("Every tile OFF; rebuilds at t = 0")
                .clicked()
                && let Some(session) = &session
            {
                session.edit(MetapixelCommand::Clear);
            }
        });
    });
}

fn view_section(ui: &mut Ui, palette: &Palette, ctx: &mut ToolsCtx) {
    let mut s = ctx.settings.clone();
    card(ui, palette, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Cells").size(12.0));
            egui::ComboBox::from_id_salt("mp_colormap").selected_text(s.colormap.label()).show_ui(ui, |ui| {
                for map in COLORMAPS {
                    ui.selectable_value(&mut s.colormap, map, map.label());
                }
            });
        });
        let stops: Vec<egui::Color32> = (0..12)
            .map(|i| {
                let [r, g, b] = s.colormap.sample_srgb(crate::geometry::display_density(i as f32 / 11.0));
                egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
            })
            .collect();
        interface::widgets::gradient_bar(ui, &stops, ui.available_width().min(220.0), "few live", "all live");
        ui.label(
            RichText::new("Colour = live fraction of the cells in each texel (power scale, exponent 0.4); one live cell is enough to light a texel.")
                .size(11.0)
                .color(palette.text_muted),
        );
        ui.checkbox(&mut s.tile_grid, "Tile grid (G)");
        ui.checkbox(&mut s.cell_grid, "Cell grid when zoomed in");
        ui.checkbox(&mut s.anatomy, "Outline the parts of the tile (O)");
        ui.checkbox(&mut s.mismatches, "Outline readout mismatches");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Frame grid (F)").clicked() {
                ctx.fit.request(true);
            }
            let shape = current_shape(&ctx.data);
            let focus = ctx.tools.selected.or(ctx.tools.hover_tile);
            let enabled = shape.is_some() && focus.is_some();
            if ui
                .add_enabled(enabled, egui::Button::new("Tile (T)"))
                .on_disabled_hover_text("Select a tile first")
                .clicked()
                && let (Some(shape), Some((c, r))) = (shape, focus)
            {
                ctx.fit.frame(shape.tile_rect(c, r), 0.06);
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Rule table (C)"))
                .on_disabled_hover_text("Select a tile first")
                .clicked()
                && let (Some(shape), Some((c, r))) = (shape, focus)
            {
                ctx.fit.frame(rule_table_rect(shape, c, r), 0.05);
            }
        });
    });
    ctx.settings.set_if_neq(s);
}

fn shortcuts_section(ui: &mut Ui, palette: &Palette) {
    egui::Grid::new("mp_shortcut_grid").num_columns(2).spacing(vec2(10.0, 2.0)).show(ui, |ui| {
        for s in MODE_SHORTCUTS {
            ui.label(RichText::new(s.label).monospace().size(12.0));
            ui.label(RichText::new(s.description).size(12.0));
            ui.end_row();
        }
        ui.label(RichText::new("Wheel").monospace().size(12.0));
        ui.label(RichText::new("Zoom towards the pointer").size(12.0));
        ui.end_row();
        ui.label(RichText::new("Right drag").monospace().size(12.0));
        ui.label(RichText::new("Pan (also WASD / arrows)").size(12.0));
        ui.end_row();
    });
    ui.label(
        RichText::new("Shell keys (Space, N, R, =, -, Tab, ...) work as everywhere; see Help (F1).")
            .size(11.5)
            .color(palette.text_muted),
    );
}
