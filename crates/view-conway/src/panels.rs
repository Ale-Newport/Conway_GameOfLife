//! Panel contents: the Tools panel (tools and their options, selection,
//! time controls, patterns, view), the Properties tab (world, engine,
//! population and dynamics, period analysis, cell inspector) and the world
//! overlay (generation, population, legend, zoom and scale).

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, Ui, vec2};
use cellular_automata::{EngineChoice, EngineKind, PasteMode, Symmetry};
use egui_plot::{AxisHints, Line, Plot, PlotPoints};
use interface::theme::with_alpha;
use interface::widgets::legend::gradient_bar_with_mid;
use interface::widgets::metrics::plot_points;
use interface::widgets::{PlotWindow, badge, legend_chip, legend_row};
use interface::{MetricsHistory, ModeRegistry, Palette, SessionStatus, ThemeSettings};
use rendering::WorldInput;
use rendering::grid::VisibleCells;
use simulation_core::ParamValue;

use crate::analysis::Analysis;
use crate::commands::{JumpState, ModeCommand, ModeCommands, SelectionOp, TimeSettings, jump_unavailable};
use crate::geometry::{MAX_JUMP_LOG2, jump_plan, rotation_degrees, scale_bar};
use crate::labels::{
    auto_reason, choice_name, choice_unavailable, engine_blurb, engine_name, percent, power_of_two_hint, thousands,
    topology_blurb, world_summary,
};
use crate::session::{Counts, LiveInfo, LiveWorld};
use crate::style::{CellColors, Legend, RenderStyle, ViewSettings, color32, legend};
use crate::tools::{
    Clipboard, DragKind, DragState, Hover, MAX_BRUSH_RADIUS, MODE_SHORTCUTS, Selection, Stamp, StampSource, Tool,
    ToolSettings,
};
use crate::window::{PatternsTab, PatternsWindow};
use crate::world::{CameraFit, LatestView, OverlayFootprint, mode_palette};

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

fn muted(text: impl Into<String>, palette: &Palette) -> RichText {
    RichText::new(text).size(12.0).color(palette.text_muted)
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
                        egui::Button::new(RichText::new(*label).size(13.0)).selected(*on).min_size(vec2(width, 28.0));
                    if ui.add(button).on_hover_text(*tip).clicked() {
                        clicked = Some(row * columns + j);
                    }
                }
            });
        }
    });
    clicked
}

fn grid_row(ui: &mut Ui, palette: &Palette, label: &str, value: impl Into<String>) {
    ui.label(muted(label, palette));
    ui.label(RichText::new(value.into()).size(12.5).monospace());
    ui.end_row();
}

fn param_bool(status: &SessionStatus, key: &str) -> bool {
    status.get().and_then(|s| s.params.get(key)).and_then(ParamValue::as_bool).unwrap_or(false)
}

fn param_text(status: &SessionStatus, key: &str) -> Option<String> {
    status.get().and_then(|s| s.params.get(key)).and_then(|v| v.as_str().map(String::from))
}

/// Left panel: tools, selection, time, patterns and view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tools_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    live: Res<LiveWorld>,
    status: Res<SessionStatus>,
    stamp: Res<Stamp>,
    selection: Res<Selection>,
    clipboard: Res<Clipboard>,
    jump: Res<JumpState>,
    mut tools: ResMut<ToolSettings>,
    mut settings: ResMut<ViewSettings>,
    mut time: ResMut<TimeSettings>,
    mut window: ResMut<PatternsWindow>,
    mut fit: ResMut<CameraFit>,
    mut queue: ResMut<ModeCommands>,
) {
    let palette = mode_palette(&theme, &registry);
    let mut t = tools.clone();
    let mut s = settings.clone();

    section(ui, &palette, "Tool");
    let items: Vec<(&str, &str, bool)> = Tool::ALL.iter().map(|x| (x.label(), x.help(), *x == t.tool)).collect();
    if let Some(i) = segmented(ui, &items, 4) {
        t.tool = Tool::ALL[i];
    }
    ui.add_space(4.0);
    tool_options(ui, &palette, &mut t, &stamp, &selection, &clipboard, &mut window, &mut fit, &mut queue);

    section(ui, &palette, "Time");
    time_section(ui, &palette, &live, &status, &jump, &mut time, &mut queue);

    section(ui, &palette, "Patterns");
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Library…  P")
            .on_hover_text("Browse the pattern catalog by category, with search and metadata.")
            .clicked()
        {
            window.toggle(PatternsTab::Library);
        }
        if ui.button("Import…  I").on_hover_text("Paste RLE / plaintext / Life 1.0x text or read a file.").clicked() {
            window.toggle(PatternsTab::Import);
        }
        if ui.button("Export RLE…").on_hover_text("The selection or the whole world as RLE.").clicked() {
            window.toggle(PatternsTab::Export);
        }
    });

    section(ui, &palette, "View");
    view_section(ui, &palette, &mut s, &status, &mut fit, &mut queue);
    shortcuts_section(ui, &palette);

    tools.set_if_neq(t);
    settings.set_if_neq(s);
}

#[allow(clippy::too_many_arguments)]
fn tool_options(
    ui: &mut Ui,
    palette: &Palette,
    t: &mut ToolSettings,
    stamp: &Stamp,
    selection: &Selection,
    clipboard: &Clipboard,
    window: &mut PatternsWindow,
    fit: &mut CameraFit,
    queue: &mut ModeCommands,
) {
    match t.tool {
        Tool::Draw | Tool::Erase => {
            // Rows must stay narrower than the panel, or the panel widens itself.
            ui.spacing_mut().slider_width = (ui.available_width() - 200.0).clamp(50.0, 180.0);
            egui::Grid::new("conway_brush").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Brush radius").on_hover_text("0 = single cells; r draws discs dx² + dy² ≤ r(r + 1).  [ / ]");
                ui.add(egui::Slider::new(&mut t.brush_radius, 0..=MAX_BRUSH_RADIUS).suffix(" cells"));
                ui.end_row();
            });
            ui.label(muted("Drag in the world · one stroke = one undo step (Ctrl/Cmd+Z).", palette));
        }
        Tool::Line | Tool::Rectangle => {
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut t.shape_erase, false, "Paint");
                ui.radio_value(&mut t.shape_erase, true, "Erase");
                if t.tool == Tool::Rectangle {
                    ui.separator();
                    ui.radio_value(&mut t.rect_filled, false, "Outline");
                    ui.radio_value(&mut t.rect_filled, true, "Filled");
                }
            });
            ui.label(muted("Press, drag and release in the world; the preview follows the pointer.", palette));
        }
        Tool::Fill => {
            fill_options(ui, palette, t);
            ui.label(muted("Drag a region; it is replaced by a random soup (undoable).", palette));
        }
        Tool::Select => selection_options(ui, palette, selection, clipboard, window, fit, queue),
        Tool::Stamp => stamp_options(ui, palette, t, stamp, window),
    }
}

fn fill_options(ui: &mut Ui, palette: &Palette, t: &mut ToolSettings) {
    ui.spacing_mut().slider_width = (ui.available_width() - 190.0).clamp(50.0, 180.0);
    egui::Grid::new("conway_fill").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Density").on_hover_text("Probability that a cell is alive. The bottom bar's Randomize uses it too.");
        let mut percent = t.fill_density * 100.0;
        if ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).step_by(1.0).suffix(" %")).changed() {
            t.fill_density = (percent / 100.0).clamp(0.0, 1.0);
        }
        ui.end_row();
        ui.label("Symmetry").on_hover_text("Rotational symmetries need a square region.");
        let label = |s: Symmetry| match s {
            Symmetry::None => "None",
            Symmetry::Horizontal => "Left-right mirror",
            Symmetry::Vertical => "Top-bottom mirror",
            Symmetry::Both => "Both mirrors",
            Symmetry::Rotate4 => "4-fold rotation",
            Symmetry::Rotate8 => "All 8 (dihedral)",
        };
        egui::ComboBox::from_id_salt("conway_symmetry").selected_text(label(t.fill_symmetry)).show_ui(ui, |ui| {
            for sym in [
                Symmetry::None,
                Symmetry::Horizontal,
                Symmetry::Vertical,
                Symmetry::Both,
                Symmetry::Rotate4,
                Symmetry::Rotate8,
            ] {
                ui.selectable_value(&mut t.fill_symmetry, sym, label(sym));
            }
        });
        ui.end_row();
        ui.label("Next seed").on_hover_text("Fills are reproducible: each fill uses this seed, then it advances.");
        ui.add(egui::DragValue::new(&mut t.fill_seed).speed(1.0));
        ui.end_row();
    });
    let _ = palette;
}

fn selection_options(
    ui: &mut Ui,
    palette: &Palette,
    selection: &Selection,
    clipboard: &Clipboard,
    window: &mut PatternsWindow,
    fit: &mut CameraFit,
    queue: &mut ModeCommands,
) {
    let Some(rect) = selection.rect else {
        ui.label(muted("Drag in the world to select a rectangle; drag inside it to move its cells.", palette));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Select pattern").on_hover_text("Select the bounding box of every live cell.").clicked() {
                queue.0.push(ModeCommand::SelectPattern);
            }
            if clipboard.pattern.is_some() && ui.button("Paste  Ctrl/Cmd+V").clicked() {
                queue.0.push(ModeCommand::Paste);
            }
        });
        return;
    };
    card(ui, palette, |ui| {
        ui.label(
            RichText::new(format!(
                "{} × {} cells",
                thousands(u128::from(rect.width())),
                thousands(u128::from(rect.height()))
            ))
            .strong(),
        );
        ui.label(muted(format!("from ({}, {}) to ({}, {})", rect.x0, rect.y0, rect.x1, rect.y1), palette));
        match selection.current_info() {
            Some(info) => {
                let area = rect.area() as f64;
                let density = if area > 0.0 { info.population as f64 / area } else { 0.0 };
                ui.label(muted(
                    format!("{} live cells · {} dense", thousands(u128::from(info.population)), percent(density)),
                    palette,
                ));
            }
            None => {
                ui.label(muted("Counting live cells…", palette));
            }
        }
    });
    ui.add_space(4.0);
    let mut op = |ui: &mut Ui, label: &str, tip: &str, command: ModeCommand| {
        if ui.button(label).on_hover_text(tip).clicked() {
            queue.0.push(command);
        }
    };
    ui.horizontal_wrapped(|ui| {
        op(
            ui,
            "Copy",
            "Ctrl/Cmd+C: into the clipboard and the system clipboard as RLE.",
            ModeCommand::Selection(SelectionOp::Copy),
        );
        op(ui, "Cut", "Ctrl/Cmd+X: copy, then kill the cells (undoable).", ModeCommand::Selection(SelectionOp::Cut));
        op(ui, "Clear", "Delete: kill the selected cells (undoable).", ModeCommand::Selection(SelectionOp::Clear));
        op(ui, "Paste", "Ctrl/Cmd+V: the clipboard pattern as the stamp.", ModeCommand::Paste);
    });
    ui.horizontal_wrapped(|ui| {
        op(
            ui,
            "Rotate ⟲",
            "Q: a quarter turn counter-clockwise, top-left corner fixed.",
            ModeCommand::Selection(SelectionOp::Rotate { cw: false }),
        );
        op(
            ui,
            "Rotate ⟳",
            "E: a quarter turn clockwise, top-left corner fixed.",
            ModeCommand::Selection(SelectionOp::Rotate { cw: true }),
        );
        op(ui, "Flip horizontal", "M: mirror left-right.", ModeCommand::Selection(SelectionOp::FlipHorizontal));
        op(ui, "Flip vertical", "Mirror top-bottom.", ModeCommand::Selection(SelectionOp::FlipVertical));
    });
    ui.horizontal_wrapped(|ui| {
        op(
            ui,
            "Random fill",
            "Replace by a soup with the Fill tool's density and symmetry.",
            ModeCommand::Selection(SelectionOp::Fill),
        );
        op(
            ui,
            "Analyse period",
            "Run the selection alone on an empty plane and detect its period.",
            ModeCommand::Analyse,
        );
        if ui.button("Frame").on_hover_text("Zoom to the selection.").clicked() {
            fit.request_rect(rect);
        }
        if ui.button("Export RLE…").clicked() {
            window.export_scope = crate::window::ExportScope::Selection;
            window.open(PatternsTab::Export);
        }
    });
    ui.label(muted("Drag inside the selection to move its cells. Esc deselects.", palette));
}

fn stamp_options(ui: &mut Ui, palette: &Palette, t: &mut ToolSettings, stamp: &Stamp, window: &mut PatternsWindow) {
    match &stamp.0 {
        Some(s) => {
            card(ui, palette, |ui| {
                ui.label(RichText::new(&s.name).strong());
                let origin = match &s.source {
                    StampSource::Library(id) => format!("library · {id}"),
                    StampSource::Clipboard => "clipboard".to_string(),
                    StampSource::Imported => "imported text".to_string(),
                };
                ui.label(muted(
                    format!(
                        "{} × {} · {} cells · {origin}",
                        s.pattern.width(),
                        s.pattern.height(),
                        thousands(s.pattern.population() as u128)
                    ),
                    palette,
                ));
            });
        }
        None => {
            ui.label(muted("No stamp yet: pick a pattern in the library, import one, or copy a selection.", palette));
        }
    }
    ui.add_space(4.0);
    egui::Grid::new("conway_stamp").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Rotation");
        ui.horizontal(|ui| {
            if ui.button("⟲").on_hover_text("Q").clicked() {
                t.rotate_stamp(false);
            }
            ui.label(RichText::new(format!("{}°", rotation_degrees(t.rotation))).monospace());
            if ui.button("⟳").on_hover_text("E").clicked() {
                t.rotate_stamp(true);
            }
        });
        ui.end_row();
        ui.label("Mirror");
        ui.checkbox(&mut t.mirror, "left-right (M)").on_hover_text("Applied before the rotation.");
        ui.end_row();
        ui.label("Combine").on_hover_text("How stamped cells combine with the cells already there.");
        let label = |m: PasteMode| match m {
            PasteMode::Or => "Or (add cells)",
            PasteMode::Copy => "Copy (clear the box first)",
            PasteMode::Xor => "Xor (toggle cells)",
        };
        egui::ComboBox::from_id_salt("conway_paste_mode").selected_text(label(t.paste_mode)).show_ui(ui, |ui| {
            for m in [PasteMode::Or, PasteMode::Copy, PasteMode::Xor] {
                ui.selectable_value(&mut t.paste_mode, m, label(m));
            }
        });
        ui.end_row();
    });
    if ui.button("Library…").clicked() {
        window.open(PatternsTab::Library);
    }
    ui.label(muted("Click in the world to place the stamp (undoable).", palette));
}

fn time_section(
    ui: &mut Ui,
    palette: &Palette,
    live: &LiveWorld,
    status: &SessionStatus,
    jump: &JumpState,
    time: &mut TimeSettings,
    queue: &mut ModeCommands,
) {
    let info = live.shown();
    let generation = status.get().map_or(0, |s| s.tick);
    card(ui, palette, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(muted("Generation", palette));
            ui.label(RichText::new(thousands(u128::from(generation))).monospace().strong());
            if let Some(hint) = power_of_two_hint(u128::from(generation)) {
                ui.label(muted(hint, palette));
            }
        });
    });
    ui.add_space(4.0);
    let gpt = info.map_or(1, |i| i.generations_per_tick);
    let hashlife_fixed = info.is_some_and(|i| i.choice == EngineChoice::Hashlife);
    ui.label(muted("Generations per tick (Step, Play)", palette));
    let choices: [u64; 8] = [1, 2, 4, 8, 16, 64, 256, 1024];
    let labels: Vec<String> = choices.iter().map(|n| thousands(u128::from(*n))).collect();
    let items: Vec<(&str, &str, bool)> =
        labels.iter().zip(choices).map(|(l, n)| (l.as_str(), "Generations advanced by one tick", n == gpt)).collect();
    if let Some(i) = segmented(ui, &items, 4) {
        queue.0.push(ModeCommand::SetGenerationsPerTick(choices[i]));
    }
    ui.horizontal(|ui| {
        let mut custom = gpt;
        let response = ui.add(
            egui::DragValue::new(&mut custom)
                .range(1..=cellular_automata::life::MAX_GENERATIONS_PER_TICK)
                .speed(1.0)
                .prefix("custom "),
        );
        if response.changed() && custom != gpt {
            if hashlife_fixed && !custom.is_power_of_two() {
                custom = custom.next_power_of_two().min(cellular_automata::life::MAX_GENERATIONS_PER_TICK);
            }
            queue.0.push(ModeCommand::SetGenerationsPerTick(custom));
        }
        if hashlife_fixed {
            ui.label(muted("powers of two (Hashlife)", palette));
        }
    });
    ui.add_space(6.0);
    ui.label(muted("Hashlife jump", palette));
    let unavailable = jump_unavailable(info, status);
    ui.add_enabled_ui(unavailable.is_none(), |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 170.0).clamp(50.0, 200.0);
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut time.jump_log2, 1..=MAX_JUMP_LOG2).prefix("2^"));
            let busy = jump.busy();
            let label = if busy { "Preparing…" } else { "Jump  J" };
            if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                queue.0.push(ModeCommand::JumpRequested);
            }
        });
        let (per_tick, ticks) = jump_plan(time.jump_log2);
        let plan = if ticks == 1 {
            "one memoised step".to_string()
        } else {
            format!("{ticks} ticks of 2^{} generations", per_tick.trailing_zeros())
        };
        ui.label(muted(format!("= {} generations ({plan})", thousands(1u128 << time.jump_log2)), palette));
    })
    .response
    .on_disabled_hover_text(unavailable.clone().unwrap_or_default());
    match unavailable {
        Some(reason) => {
            ui.label(muted(reason, palette));
        }
        None => {
            ui.label(muted(
                "Pauses the run, fixes the engine to Hashlife and turns tracking off; undo returns to the start.",
                palette,
            ));
        }
    }
}

fn colors_swatch(ui: &mut Ui, colors: CellColors) {
    let (dead, alive) = colors.colors();
    let (rect, _) = ui.allocate_exact_size(vec2(26.0, 14.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, color32(dead));
    let cell = egui::Rect::from_center_size(rect.center(), vec2(8.0, 8.0));
    painter.rect_filled(cell, 1.5, color32(alive));
}

fn view_section(
    ui: &mut Ui,
    palette: &Palette,
    s: &mut ViewSettings,
    status: &SessionStatus,
    fit: &mut CameraFit,
    queue: &mut ModeCommands,
) {
    egui::Grid::new("conway_view").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Style").on_hover_text("V / Shift+V cycle the styles.");
        egui::ComboBox::from_id_salt("conway_style").selected_text(s.style.label()).width(170.0).show_ui(ui, |ui| {
            for style in RenderStyle::ALL {
                if ui.selectable_label(s.style == style, style.label()).on_hover_text(style.help()).clicked()
                    && style != s.style
                {
                    queue.0.push(ModeCommand::SetStyle(style));
                }
            }
        });
        ui.end_row();
        ui.label("Colours");
        egui::ComboBox::from_id_salt("conway_colors").selected_text(s.colors.label()).width(170.0).show_ui(ui, |ui| {
            for colors in CellColors::ALL {
                ui.horizontal(|ui| {
                    colors_swatch(ui, colors);
                    ui.selectable_value(&mut s.colors, colors, colors.label());
                });
            }
        });
        ui.end_row();
    });
    if let Some(tracking) = s.style.tracking()
        && !param_bool(status, tracking.param())
    {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                palette.warning,
                RichText::new(format!("{} is off: this style shows no history.", tracking.label())).size(12.0),
            );
            if ui.small_button("Turn on").clicked() {
                queue.0.push(ModeCommand::EnableTracking(tracking));
            }
        });
    }
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut s.grid, "Cell grid (G)")
            .on_hover_text("Fades in once cells are at least 10 physical pixels wide.");
        ui.add_enabled(s.grid, egui::Checkbox::new(&mut s.major_grid, "Every 10th line"));
    });
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut s.border, "World border");
        ui.checkbox(&mut s.hover_readout, "Cell readout");
    });
    ui.add_space(4.0);
    if ui
        .button("Frame world  F")
        .on_hover_text("Fit a finite world, or the live cells of the infinite plane.")
        .clicked()
    {
        fit.request_world();
    }
}

fn shortcuts_section(ui: &mut Ui, palette: &Palette) {
    ui.add_space(8.0);
    egui::CollapsingHeader::new(RichText::new("Shortcuts and mouse").size(12.5).color(palette.text_muted))
        .id_salt("conway_shortcuts")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("conway_shortcut_grid").num_columns(2).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
                for s in MODE_SHORTCUTS {
                    ui.label(RichText::new(s.label).monospace().size(12.0));
                    ui.label(RichText::new(s.description).size(12.0));
                    ui.end_row();
                }
                for (keys, description) in [
                    ("Left drag", "Use the tool"),
                    ("Right / middle drag", "Pan"),
                    ("Wheel, pinch", "Zoom towards the pointer"),
                    ("Arrows, WASD", "Pan"),
                    ("Space, N, R, Ctrl/Cmd+Z", "Play, step, reset, undo (shell)"),
                ] {
                    ui.label(RichText::new(keys).monospace().size(12.0));
                    ui.label(RichText::new(description).size(12.0));
                    ui.end_row();
                }
            });
        });
}

/// Right panel "Properties": world, engine, population, period, inspector.
#[allow(clippy::too_many_arguments)]
pub(crate) fn properties_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    live: Res<LiveWorld>,
    status: Res<SessionStatus>,
    history: Res<MetricsHistory>,
    latest: Res<LatestView>,
    hover: Res<Hover>,
    selection: Res<Selection>,
    analysis: Res<Analysis>,
    mut queue: ResMut<ModeCommands>,
) {
    let palette = mode_palette(&theme, &registry);
    let Some(info) = live.shown() else {
        ui.label(RichText::new("Starting the simulation…").weak());
        return;
    };
    let counts = live.counts(&history, &status).unwrap_or(Counts {
        generation: info.generation,
        population: info.population,
        births_deaths: info.last_stats.map(|s| (s.births, s.deaths)),
    });
    world_card(ui, &palette, &live, info, &status, &mut queue);
    engine_card(ui, &palette, info, &mut queue);
    population_section(ui, &palette, info, counts, &status);
    period_section(ui, &palette, &analysis, &selection, &mut queue);
    inspector(ui, &palette, &latest, &hover);
    plots_section(ui, &palette, &history);
}

fn world_card(
    ui: &mut Ui,
    palette: &Palette,
    live: &LiveWorld,
    info: &LiveInfo,
    status: &SessionStatus,
    queue: &mut ModeCommands,
) {
    section(ui, palette, "World");
    card(ui, palette, |ui| {
        ui.horizontal_wrapped(|ui| {
            if info.is_conway() {
                badge(ui, "B3/S23", palette.accent).on_hover_text("Conway's Game of Life, unaltered.");
            } else {
                badge(ui, &info.rule.to_string(), palette.warning);
            }
            ui.label(RichText::new(world_summary(info.world)).strong());
        });
        ui.label(muted(topology_blurb(info.world.topology), palette));
        if !info.is_conway() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    palette.warning,
                    RichText::new(format!("This state runs {} (loaded from a save).", info.rule)).size(12.0),
                );
                if ui.small_button("Restore B3/S23").clicked() {
                    queue.0.push(ModeCommand::RestoreConway);
                }
            });
        }
        let next_topology = param_text(status, "topology");
        let next_size =
            status.get().and_then(|s| Some((s.params.get("width")?.as_i64()?, s.params.get("height")?.as_i64()?)));
        let current_topology = next_topology_name(info.world.topology);
        let pending_topology = next_topology.as_deref().is_some_and(|t| t != current_topology);
        let pending_size =
            next_size.is_some_and(|(w, h)| (w, h) != (i64::from(info.world.width), i64::from(info.world.height)));
        if pending_topology || pending_size {
            let (w, h) = next_size.unwrap_or((i64::from(info.world.width), i64::from(info.world.height)));
            let topology = next_topology.unwrap_or_default().replace('_', " ");
            ui.colored_label(
                palette.warning,
                RichText::new(format!("Next reset (R): {w} × {h} {topology}.")).size(12.0),
            );
        }
        if let Some(halt) = &info.halt {
            ui.colored_label(palette.error, RichText::new(format!("Stopped: {halt}")).size(12.0));
        }
        if let Some(origin) = &live.origin {
            ui.add_space(2.0);
            egui::CollapsingHeader::new(RichText::new(&origin.name).size(12.5))
                .id_salt("conway_about_scenario")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(RichText::new(&origin.description).size(12.5));
                });
        }
    });
}

fn next_topology_name(t: simulation_core::Topology) -> &'static str {
    match t {
        simulation_core::Topology::Bounded => "bounded",
        simulation_core::Topology::Torus => "torus",
        simulation_core::Topology::KleinBottle => "klein_bottle",
        simulation_core::Topology::CrossSurface => "cross_surface",
        simulation_core::Topology::Infinite => "infinite",
    }
}

fn engine_card(ui: &mut Ui, palette: &Palette, info: &LiveInfo, queue: &mut ModeCommands) {
    section(ui, palette, "Engine");
    card(ui, palette, |ui| {
        ui.horizontal(|ui| {
            badge(ui, engine_name(info.engine), palette.accent);
            ui.label(muted("running", palette));
        });
        let bbox_area = info.bbox.map(|b| b.area());
        let choices =
            [EngineChoice::Auto, EngineChoice::Dense, EngineChoice::Bit, EngineChoice::Sparse, EngineChoice::Hashlife];
        ui.horizontal(|ui| {
            ui.label("Choice");
            egui::ComboBox::from_id_salt("conway_engine").selected_text(choice_name(info.choice)).width(150.0).show_ui(
                ui,
                |ui| {
                    for choice in choices {
                        let blocked = choice_unavailable(
                            choice,
                            info.world,
                            info.generations_per_tick,
                            info.engine,
                            info.population,
                            bbox_area,
                        );
                        let response = ui.add_enabled(
                            blocked.is_none(),
                            egui::Button::selectable(info.choice == choice, choice_name(choice)),
                        );
                        let response = match &blocked {
                            Some(reason) => response.on_disabled_hover_text(reason),
                            None => response.on_hover_text(match choice.fixed() {
                                Some(kind) => engine_blurb(kind),
                                None => "Chosen from the world, the generations per tick and the pattern.",
                            }),
                        };
                        if response.clicked() && choice != info.choice {
                            queue.0.push(ModeCommand::SetEngine(choice));
                        }
                    }
                },
            );
        });
        let text = if info.choice == EngineChoice::Auto {
            auto_reason(info.world, info.engine, info.generations_per_tick)
        } else {
            format!("Fixed choice. {}", engine_blurb(info.engine))
        };
        ui.label(muted(text, palette));
        if let Some(h) = info.hashlife {
            ui.add_space(4.0);
            egui::Grid::new("conway_hashlife").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                grid_row(ui, palette, "Distinct nodes", thousands(h.nodes as u128));
                grid_row(ui, palette, "Root square", format!("2^{0} × 2^{0} cells", h.root_level));
                grid_row(ui, palette, "Memo memory", interface::format::format_bytes(h.memory_bytes as u64));
                grid_row(ui, palette, "Collections", thousands(u128::from(h.collections)));
            });
            if info.population > h.nodes as u128 {
                ui.label(muted(
                    format!(
                        "{} live cells are stored in {} distinct squares: repeated structure is shared.",
                        thousands(info.population),
                        thousands(h.nodes as u128)
                    ),
                    palette,
                ));
            }
        }
    });
}

fn series(history: &MetricsHistory, key: &str) -> Vec<[f64; 2]> {
    history.store().get(key).map_or_else(Vec::new, |s| plot_points(s, PlotWindow::Last(u64::MAX), 600))
}

fn small_plot(ui: &mut Ui, id: &str, palette: &Palette, lines: Vec<(&str, Vec<[f64; 2]>, Color32)>) {
    if lines.iter().all(|(_, p, _)| p.is_empty()) {
        ui.label(muted("Run the simulation to record it.", palette));
        return;
    }
    Plot::new(id.to_string())
        .height(96.0)
        .allow_scroll(false)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_background(false)
        .include_y(0.0)
        .grid_color(with_alpha(palette.text_muted, 0.3))
        .custom_x_axes(vec![
            AxisHints::new_x()
                .label("generation")
                .label_spacing(26.0..=40.0)
                .formatter(|mark, _| interface::format::format_compact(mark.value)),
        ])
        .y_axis_formatter(|mark, _| interface::format::format_compact(mark.value))
        .link_axis(egui::Id::new("conway_dynamics"), [true, false])
        .show(ui, |plot| {
            for (name, points, color) in lines {
                plot.line(Line::new(name.to_string(), PlotPoints::from(points)).color(color).width(1.6));
            }
        });
}

fn population_section(ui: &mut Ui, palette: &Palette, info: &LiveInfo, counts: Counts, status: &SessionStatus) {
    section(ui, palette, "Population and dynamics");
    egui::Grid::new("conway_population").num_columns(2).spacing([14.0, 3.0]).striped(true).show(ui, |ui| {
        grid_row(ui, palette, "Generation", thousands(u128::from(counts.generation)));
        grid_row(ui, palette, "Population", thousands(counts.population));
        match counts.births_deaths {
            Some((births, deaths)) => {
                grid_row(
                    ui,
                    palette,
                    "Births / deaths (last tick)",
                    format!("{} / {}", thousands(u128::from(births)), thousands(u128::from(deaths))),
                );
            }
            None => {
                let why =
                    if info.engine == EngineKind::Hashlife { "not counted in jumps" } else { "after the first tick" };
                grid_row(ui, palette, "Births / deaths", why);
            }
        }
        match info.bbox {
            Some(b) => {
                grid_row(
                    ui,
                    palette,
                    "Bounding box",
                    format!("{} × {}", thousands(u128::from(b.width())), thousands(u128::from(b.height()))),
                );
                grid_row(ui, palette, "Box top-left", format!("({}, {})", b.x0, b.y0));
                let area = if info.world.is_finite() { u128::from(info.world.cell_count()) } else { b.area() };
                let what = if info.world.is_finite() { "Density (world)" } else { "Density (box)" };
                // The world's area is fixed; the box was read with `info`.
                let population = if info.world.is_finite() { counts.population } else { info.population };
                grid_row(ui, palette, what, percent(population as f64 / area.max(1) as f64));
            }
            None => grid_row(ui, palette, "Bounding box", "empty"),
        }
        if let Some(s) = status.get() {
            let rate = if s.playing {
                format!("{} gen/s", interface::format::format_compact(s.measured_gps))
            } else {
                "paused".into()
            };
            grid_row(ui, palette, "Generation rate", rate);
        }
        grid_row(ui, palette, "Model memory", interface::format::format_bytes(info.memory_bytes as u64));
    });
}

fn plots_section(ui: &mut Ui, palette: &Palette, history: &MetricsHistory) {
    section(ui, palette, "Plots");
    legend_row(ui, &[(palette.accent, "population")]);
    small_plot(
        ui,
        "conway_population_plot",
        palette,
        vec![("population", series(history, "population"), palette.accent)],
    );
    legend_row(ui, &[(palette.series_color(2), "births"), (palette.series_color(1), "deaths")]);
    small_plot(
        ui,
        "conway_births_plot",
        palette,
        vec![
            ("births", series(history, "births"), palette.series_color(2)),
            ("deaths", series(history, "deaths"), palette.series_color(1)),
        ],
    );
}

fn period_section(
    ui: &mut Ui,
    palette: &Palette,
    analysis: &Analysis,
    selection: &Selection,
    queue: &mut ModeCommands,
) {
    section(ui, palette, "Period");
    let scope = if selection.rect.is_some() { "the selection (alone on an empty plane)" } else { "the whole world" };
    ui.horizontal_wrapped(|ui| {
        let busy = analysis.running();
        if ui
            .add_enabled(!busy, egui::Button::new("Analyse"))
            .on_hover_text(
                "Runs a copy of the state up to 4,096 generations (4 s at most) and reports when it returns to its \
             starting state, with its displacement. The live run is not touched.",
            )
            .clicked()
        {
            queue.0.push(ModeCommand::Analyse);
        }
        if busy {
            ui.spinner();
        }
        ui.label(muted(format!("examines {scope}"), palette));
    });
    if let Some(report) = &analysis.report {
        card(ui, palette, |ui| {
            let what = match report.selection {
                Some(r) => format!(
                    "Selection {} × {} at generation {}",
                    r.width(),
                    r.height(),
                    thousands(u128::from(report.generation))
                ),
                None => format!("World at generation {}", thousands(u128::from(report.generation))),
            };
            ui.label(muted(what, palette));
            ui.label(RichText::new(report.result.describe(report.finite)).size(13.0));
        });
    }
}

fn inspector(ui: &mut Ui, palette: &Palette, latest: &LatestView, hover: &Hover) {
    section(ui, palette, "Cell under the pointer");
    let Some(cell) = hover.cell else {
        ui.label(muted("Hover the world to inspect a cell.", palette));
        return;
    };
    egui::Grid::new("conway_inspector").num_columns(2).spacing([14.0, 2.0]).show(ui, |ui| {
        grid_row(ui, palette, "Cell", format!("({}, {})", cell.x, cell.y));
        match latest.texel_at(cell) {
            Some((texel, 1)) => {
                grid_row(ui, palette, "State", if texel[0] > 0 { "alive" } else { "dead" });
                if texel[1] > 0 {
                    let age = if texel[1] == 255 { "255+".to_string() } else { texel[1].to_string() };
                    grid_row(ui, palette, "Age", format!("{age} generations"));
                }
                if texel[2] > 0 {
                    grid_row(ui, palette, "Activity", format!("{} / 255", texel[2]));
                }
            }
            Some((texel, k)) => {
                grid_row(ui, palette, "Texel", format!("{k} × {k} cells"));
                grid_row(ui, palette, "Live fraction", format!("{:.1} %", f32::from(texel[3]) / 2.55));
            }
            None => grid_row(ui, palette, "State", "outside the shown region"),
        }
    });
}

/// Overlay over the world: generation, population, world, zoom and legend.
#[allow(clippy::too_many_arguments)]
pub(crate) fn overlay_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    live: Res<LiveWorld>,
    settings: Res<ViewSettings>,
    latest: Res<LatestView>,
    visible: Res<VisibleCells>,
    hover: Res<Hover>,
    drag: Res<DragState>,
    input: Res<WorldInput>,
    history: Res<MetricsHistory>,
    status: Res<SessionStatus>,
    mut footprint: ResMut<OverlayFootprint>,
    mut collapsed: Local<bool>,
) {
    let palette = mode_palette(&theme, &registry);
    let (Some(info), Some(counts)) = (live.shown(), live.counts(&history, &status)) else { return };
    // The shell places the overlay 12 px inside the viewport's top-left corner.
    let viewport_origin = ui.max_rect().min - vec2(12.0, 12.0);
    let shown = egui::Frame::new()
        .fill(with_alpha(palette.panel, 0.95))
        .stroke(Stroke::new(palette.stroke_width, with_alpha(palette.stroke, 0.9)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_max_width(260.0);
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.horizontal(|ui| {
                let (_, toggle) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
                egui::collapsing_header::paint_default_icon(ui, if *collapsed { 0.0 } else { 1.0 }, &toggle);
                let tip = if *collapsed { "Show the legend, world and zoom" } else { "Fold the overlay" };
                if toggle.on_hover_text(tip).clicked() {
                    *collapsed = !*collapsed;
                }
                let rule = if info.is_conway() { "B3/S23".to_string() } else { info.rule.to_string() };
                badge(ui, &rule, if info.is_conway() { palette.accent } else { palette.warning });
                let name = live.origin.as_ref().map_or("Classic Conway", |o| o.name.as_str());
                ui.label(RichText::new(name).size(13.0).strong());
            });
            // Generation and population of the same state.
            let generation = counts.generation;
            ui.horizontal(|ui| {
                ui.label(muted("Gen", &palette));
                ui.label(RichText::new(thousands(u128::from(generation))).monospace().size(13.0).strong());
                if let Some(hint) = power_of_two_hint(u128::from(generation)) {
                    ui.label(muted(hint, &palette));
                }
            });
            ui.horizontal(|ui| {
                ui.label(muted("Pop", &palette));
                ui.label(RichText::new(thousands(counts.population)).monospace().size(13.0));
            });
            if *collapsed {
                return;
            }
            ui.label(muted(format!("{} · {}", world_summary(info.world), engine_name(info.engine)), &palette));
            overlay_legend(ui, &settings);
            let cpp = visible.cells_per_pixel;
            if cpp > 0.0 {
                let zoom = if cpp <= 1.0 {
                    format!("1 cell = {:.1} px", 1.0 / cpp)
                } else {
                    format!("1 px = {} cells", interface::format::format_compact(f64::from(cpp)))
                };
                let lod = latest.view.as_ref().map_or(1, |v| v.cells_per_texel);
                let lod_text = if lod > 1 { format!(" · texel = {lod} × {lod} cells") } else { String::new() };
                ui.label(muted(format!("{zoom}{lod_text}"), &palette));
            }
            if let Some((cells, px)) = scale_bar(cpp, 90.0) {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(px.min(200.0), 10.0), Sense::hover());
                    let y = rect.center().y;
                    let stroke = Stroke::new(1.5, palette.text);
                    ui.painter().line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], stroke);
                    for x in [rect.left(), rect.right()] {
                        ui.painter().line_segment([egui::pos2(x, y - 4.0), egui::pos2(x, y + 4.0)], stroke);
                    }
                    let unit = if cells == 1.0 { "cell" } else { "cells" };
                    ui.label(muted(format!("{} {unit}", thousands(cells as u128)), &palette));
                });
            }
        });
    let corner = shown.response.rect.max - viewport_origin;
    footprint.0 = Some(Vec2::new(corner.x, corner.y));
    if settings.hover_readout {
        hover_readout(ui, &palette, &latest, &hover, &drag, &input);
    }
}

fn overlay_legend(ui: &mut Ui, settings: &ViewSettings) {
    match legend(settings) {
        Legend::States { alive, dead } => {
            ui.horizontal(|ui| {
                legend_chip(ui, alive, "alive");
                legend_chip(ui, dead, "dead");
            });
        }
        Legend::Ramp { stops, low, mid, high, alive } => {
            if let Some(alive) = alive {
                legend_chip(ui, alive, "alive");
            }
            gradient_bar_with_mid(ui, &stops, 230.0, low, mid, high);
        }
    }
}

/// Top-left corner of a readout of `size` next to the pointer, flipped to
/// the other side where it would leave the world viewport.
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

/// Text of the readout: the cell and its state, or the size of a drag.
pub fn readout_text(
    cell: bevy::math::I64Vec2,
    texel: Option<([u8; 4], u32)>,
    drag: Option<&crate::tools::Drag>,
) -> String {
    if let Some(d) = drag {
        let r = d.rect();
        return match d.kind {
            DragKind::Line => {
                let n = r.width().max(r.height());
                format!("line {n} cells")
            }
            DragKind::Move { .. } => {
                let o = d.offset();
                format!("move ({:+}, {:+})", o.x, o.y)
            }
            DragKind::Stroke { .. } => format!("({}, {})", cell.x, cell.y),
            _ => format!("{} × {}", r.width(), r.height()),
        };
    }
    let mut text = format!("({}, {})", cell.x, cell.y);
    match texel {
        Some((t, 1)) => {
            text.push_str(if t[0] > 0 { "  alive" } else { "  dead" });
            if t[1] > 0 {
                text.push_str(&format!("  age {}", if t[1] == 255 { "255+".into() } else { t[1].to_string() }));
            }
        }
        Some((t, _)) => text.push_str(&format!("  {:.0} % alive", f32::from(t[3]) / 2.55)),
        None => {}
    }
    text
}

fn hover_readout(ui: &Ui, palette: &Palette, latest: &LatestView, hover: &Hover, drag: &DragState, input: &WorldInput) {
    let (Some(cell), Some(cursor)) = (hover.cell, input.cursor) else { return };
    let text = readout_text(cell, latest.texel_at(cell), drag.drag.as_ref());
    let painter =
        ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("conway_hover_readout")));
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
    use crate::tools::{Drag, DragKind};
    use bevy::math::I64Vec2;

    #[test]
    fn readouts_describe_cells_and_drags() {
        assert_eq!(readout_text(I64Vec2::new(3, -4), Some(([1, 12, 0, 255], 1)), None), "(3, -4)  alive  age 12");
        assert_eq!(readout_text(I64Vec2::new(0, 0), Some(([0, 0, 0, 0], 1)), None), "(0, 0)  dead");
        assert_eq!(readout_text(I64Vec2::new(0, 0), Some(([1, 0, 0, 128], 8)), None), "(0, 0)  50 % alive");
        let d = Drag { kind: DragKind::Rectangle, start: I64Vec2::new(0, 0), current: I64Vec2::new(9, 4) };
        assert_eq!(readout_text(I64Vec2::ZERO, None, Some(&d)), "10 × 5");
        let m = Drag {
            kind: DragKind::Move { rect: cellular_automata::CellRect::point(0, 0) },
            start: I64Vec2::new(5, 5),
            current: I64Vec2::new(2, 7),
        };
        assert_eq!(readout_text(I64Vec2::ZERO, None, Some(&m)), "move (-3, +2)");
        let l = Drag { kind: DragKind::Line, start: I64Vec2::new(0, 0), current: I64Vec2::new(-7, 3) };
        assert_eq!(readout_text(I64Vec2::ZERO, None, Some(&l)), "line 8 cells");
    }

    #[test]
    fn readouts_stay_inside_the_viewport() {
        let viewport = Some(egui::Rect::from_min_max(egui::pos2(300.0, 40.0), egui::pos2(1200.0, 900.0)));
        let size = vec2(120.0, 40.0);
        assert_eq!(readout_position(egui::pos2(500.0, 300.0), size, viewport), egui::pos2(516.0, 316.0));
        assert_eq!(readout_position(egui::pos2(1150.0, 300.0), size, viewport), egui::pos2(1014.0, 316.0));
        assert_eq!(readout_position(egui::pos2(500.0, 880.0), size, viewport), egui::pos2(516.0, 824.0));
    }
}

#[cfg(test)]
mod glyph_tests {
    use bevy_egui::egui;

    /// Non-ASCII characters of the string literals in this crate's sources
    /// (comment lines and unit tests skipped).
    fn literal_chars() -> Vec<(char, &'static str)> {
        let sources: [(&str, &str); 12] = [
            ("analysis.rs", include_str!("analysis.rs")),
            ("commands.rs", include_str!("commands.rs")),
            ("geometry.rs", include_str!("geometry.rs")),
            ("labels.rs", include_str!("labels.rs")),
            ("library.rs", include_str!("library.rs")),
            ("panels.rs", include_str!("panels.rs")),
            ("session.rs", include_str!("session.rs")),
            ("style.rs", include_str!("style.rs")),
            ("tools.rs", include_str!("tools.rs")),
            ("tutorial.rs", include_str!("tutorial.rs")),
            ("window.rs", include_str!("window.rs")),
            ("world.rs", include_str!("world.rs")),
        ];
        let mut out = Vec::new();
        for (file, text) in sources {
            let text = text.split("\n#[cfg(test)]").next().unwrap_or(text);
            for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
                for c in line.chars().filter(|c| !c.is_ascii()) {
                    if !out.iter().any(|(x, _)| *x == c) {
                        out.push((c, file));
                    }
                }
            }
        }
        out
    }

    /// Every character the mode displays (its own texts, the served model
    /// descriptor, the tutorial, the scenarios and the pattern catalog)
    /// exists in egui's built-in fonts; otherwise it renders as a box.
    #[test]
    fn every_displayed_character_has_a_glyph() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.label("x");
        });
        output.textures_delta.clear();
        let has = |c: char| ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), &c.to_string()));
        let mut missing: Vec<String> = literal_chars()
            .into_iter()
            .filter(|(c, _)| !has(*c))
            .map(|(c, file)| format!("{c} (U+{:04X}, {file})", c as u32))
            .collect();
        let mut texts: Vec<(String, &str)> = Vec::new();
        let tutorial = crate::tutorial::tutorial();
        texts.extend(tutorial.pages.iter().map(|p| (format!("{} {}", p.title, p.body), "tutorial")));
        texts.push((tutorial.concepts.clone(), "concepts"));
        for topology in simulation_core::Topology::ALL {
            let config = cellular_automata::LifeConfig { topology, init: vec![], ..Default::default() };
            let sim = cellular_automata::LifeSim::new(
                &config,
                1,
                &cellular_automata::PatternLibrary::builtin(),
                cellular_automata::LifeMode::Classic,
            )
            .unwrap();
            let d = crate::session::served_descriptor(&simulation_core::Simulation::descriptor(&sim));
            texts.push((format!("{} {} {}", d.name, d.summary, d.rules), "descriptor"));
            for p in &d.params {
                texts.push((format!("{} {} {}", p.label, p.description, p.group), "parameter"));
                if let simulation_core::ParamKind::Choice { options } = &p.kind {
                    texts.extend(options.iter().map(|o| (o.label.clone(), "choice")));
                }
            }
            texts.extend(d.metrics.iter().map(|m| (format!("{} {} {}", m.label, m.description, m.unit), "metric")));
        }
        for s in &crate::session::ConwayScenarios::builtin().0 {
            texts.push((format!("{} {}", s.name, s.description), "scenario"));
        }
        let library =
            crate::library::ConwayLibrary::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"));
        for e in library.catalog.entries() {
            let mut t = format!("{} {}", e.name, e.description.clone().unwrap_or_default());
            for v in [&e.discoverer, &e.expected_behavior, &e.source, &e.license] {
                t.push_str(v.as_deref().unwrap_or_default());
            }
            t.extend(e.extra.values().map(String::as_str));
            texts.push((t, "catalog"));
        }
        for (text, origin) in texts {
            for c in text.chars().filter(|c| !c.is_ascii() && !has(*c)) {
                let entry = format!("{c} (U+{:04X}, {origin})", c as u32);
                if !missing.contains(&entry) {
                    missing.push(entry);
                }
            }
        }
        assert!(missing.is_empty(), "characters without a glyph: {missing:?}");
    }
}
