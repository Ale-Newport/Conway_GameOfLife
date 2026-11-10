//! Right "Properties" tab: scenario, rule in effect, population, selected
//! voxel, region counts, 2D slice inspector, plots and render statistics.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Stroke};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use interface::widgets::{badge, legend_chip};
use interface::{MetricsHistory, ModeRegistry, Palette, SessionStatus, ThemeSettings};
use voxel_life::{Axis, Evidence, Rule3D, SliceView};

use super::{UiState, card, count, count_set, fitted, hint, mono, section, segmented, stacked_bar, state_name};
use crate::MODE;
use crate::build::MeshState;
use crate::colors::{dying_stops, state_color32};
use crate::requests::{ProbeResult, ProbeState, RegionState, SliceState, WorldState, current_counts};
use crate::settings::{ViewSettings, axis_label, axis_range};

#[cfg(test)]
/// Non-ASCII characters used in this panel (checked against egui's fonts).
pub(crate) const GLYPHS: &str = "–·×…³";

/// Generations shown by the Properties plots.
const PLOT_SPAN: u64 = 500;

/// Resources the Properties tab reads and writes.
#[derive(SystemParam)]
pub(crate) struct PropertiesParams<'w> {
    status: Res<'w, SessionStatus>,
    world: Res<'w, WorldState>,
    mesh: Res<'w, MeshState>,
    settings: ResMut<'w, ViewSettings>,
    ui_state: ResMut<'w, UiState>,
    slice: ResMut<'w, SliceState>,
    region: Res<'w, RegionState>,
    probe: ResMut<'w, ProbeState>,
    history: Res<'w, MetricsHistory>,
    theme: Res<'w, ThemeSettings>,
    registry: Res<'w, ModeRegistry>,
}

/// The Properties tab.
pub(crate) fn properties_panel(InMut(ui): InMut<egui::Ui>, mut p: PropertiesParams) {
    let palette = Palette::new(p.theme.kind, p.registry.accent(MODE));
    let Some(info) = p.world.info.clone() else {
        hint(ui, &palette, "Reading the world…");
        return;
    };
    scenario_card(ui, &palette, &info);
    ui.add_space(6.0);
    rule_card(ui, &palette, &info.rule);
    population_section(ui, &palette, &p, &info);
    probe_section(ui, &palette, &mut p, &info.rule);
    slice_section(ui, &palette, &mut p, info.rule.states());
    region_section(ui, &palette, &p, info.rule.states());
    plots_section(ui, &palette, &p);
    render_section(ui, &palette, &p);
}

fn evidence_badge(evidence: Evidence, palette: &Palette) -> (&'static str, Color32) {
    match evidence {
        Evidence::Tested => ("tested", palette.success),
        Evidence::Documented => ("documented", palette.accent),
        Evidence::Observed => ("observed", palette.warning),
        Evidence::Unverified => ("unverified", palette.text_muted),
    }
}

fn scenario_card(ui: &mut egui::Ui, palette: &Palette, info: &crate::requests::WorldInfo) {
    card(ui, palette, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&info.scenario.name).strong().size(15.0).color(palette.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, color) = evidence_badge(info.scenario.evidence, palette);
                badge(ui, text, color).on_hover_text(info.scenario.evidence.label());
            });
        });
        ui.label(RichText::new(&info.scenario.description).size(12.0).color(palette.text_muted));
        if !info.scenario.reference.is_empty() {
            ui.add_space(2.0);
            ui.label(RichText::new(&info.scenario.reference).size(11.0).italics().color(palette.text_muted));
        }
        ui.add_space(4.0);
        let [w, h, d] = info.config.size;
        let size =
            if info.is_finite() { format!("{w} × {h} × {d}") } else { format!("initial box {w} × {h} × {d}") };
        ui.label(RichText::new(format!("{} · {size}", info.config.kind.label())).size(11.5).color(palette.text));
        if info.pending != info.config {
            ui.add_space(4.0);
            ui.label(
                RichText::new("World or initial-state settings were changed: Reset (R) rebuilds the world with them.")
                    .size(11.5)
                    .color(palette.warning),
            );
        }
    });
}

fn rule_card(ui: &mut egui::Ui, palette: &Palette, rule: &Rule3D) {
    section(ui, palette, "Rule in effect");
    card(ui, palette, |ui| {
        ui.label(mono(rule.to_softology(), 18.0).color(palette.accent).strong());
        let mut forms = vec![rule.to_letters()];
        forms.extend(rule.to_bays().map(|b| format!("Bays {b}")));
        forms.extend(rule.to_golly().map(|g| format!("Golly {g}")));
        ui.label(mono(forms.join(" · "), 11.5).color(palette.text_muted));
        ui.add_space(4.0);
        let n = rule.neighbourhood();
        let states = rule.states();
        egui::Grid::new("view_voxel_rule_card").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
            ui.label(RichText::new("Neighbourhood").color(palette.text_muted));
            ui.label(format!("{} ({} voxels)", n.label(), n.size()));
            ui.end_row();
            ui.label(RichText::new("Born with").color(palette.text_muted));
            ui.label(format!("{} alive neighbours", count_set(rule.birth())));
            ui.end_row();
            ui.label(RichText::new("Survives with").color(palette.text_muted));
            ui.label(format!("{} alive neighbours", count_set(rule.survive())));
            ui.end_row();
            ui.label(RichText::new("States").color(palette.text_muted));
            ui.label(if states > 2 {
                format!("{states}: empty, alive, dying 2–{}", states - 1)
            } else {
                "2: empty, alive".to_string()
            });
            ui.end_row();
        });
        ui.add_space(4.0);
        hint(
            ui,
            palette,
            "Neighbours are counted in three dimensions, so this is not Conway's 2D B3/S23. Only alive \
             (state 1) neighbours count; dying voxels fade one state per generation.",
        );
    });
}

fn latest(history: &MetricsHistory, key: &str, tick: u64) -> Option<f64> {
    history.store().last(key).filter(|p| p.tick == tick).map(|p| p.value)
}

fn population_section(ui: &mut egui::Ui, palette: &Palette, p: &PropertiesParams, info: &crate::requests::WorldInfo) {
    let Some(status) = p.status.get() else { return };
    section(ui, palette, &format!("Population · generation {}", count(status.tick)));
    card(ui, palette, |ui| {
        let store_tick = p.history.store().last("population").map(|pt| pt.tick).filter(|t| *t == status.tick);
        let births = store_tick.and_then(|t| latest(&p.history, "births", t));
        let deaths = store_tick.and_then(|t| latest(&p.history, "deaths", t));
        match current_counts(&p.history, Some(info), status.tick) {
            None => {
                ui.label(RichText::new("Reading…").color(palette.text_muted));
            }
            Some((population, alive)) => {
                let (population, alive) = (population as f64, alive as f64);
                let dying = (population - alive).max(0.0);
                ui.label(RichText::new(count(population as u64)).size(22.0).strong().color(palette.text));
                stacked_bar(
                    ui,
                    palette,
                    &[(alive, state_color32(1, info.rule.states())), (dying, dying_stops()[1])],
                    8.0,
                );
                ui.add_space(4.0);
                egui::Grid::new("view_voxel_population").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
                    ui.label(RichText::new("Alive (state 1)").color(palette.text_muted));
                    ui.label(count(alive as u64));
                    ui.end_row();
                    if info.rule.states() > 2 {
                        ui.label(RichText::new("Dying").color(palette.text_muted));
                        ui.label(count(dying as u64));
                        ui.end_row();
                    }
                    if let (Some(b), Some(d)) = (births, deaths) {
                        ui.label(RichText::new("Births / deaths").color(palette.text_muted));
                        ui.label(format!("{} / {}", count(b as u64), count(d as u64)));
                        ui.end_row();
                    }
                    if let Some(bounds) = p.world.population {
                        let [x, y, z] = bounds.extent();
                        ui.label(RichText::new("Extent").color(palette.text_muted));
                        ui.label(format!("{x} × {y} × {z}"));
                        ui.end_row();
                    }
                    let centroid = ["centroid_x", "centroid_y", "centroid_z"]
                        .map(|k| store_tick.and_then(|t| latest(&p.history, k, t)));
                    if let [Some(cx), Some(cy), Some(cz)] = centroid {
                        ui.label(RichText::new("Centroid").color(palette.text_muted));
                        ui.label(format!("({cx:.1}, {cy:.1}, {cz:.1})"));
                        ui.end_row();
                    }
                });
            }
        }
        if let Some(halt) = &info.halt {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Halted: {halt}")).color(palette.error).size(12.0));
        }
    });
}

/// What the rule does with a probed voxel next generation.
pub(crate) fn fate_text(probe: &ProbeResult) -> String {
    let n = probe.alive_neighbours;
    let birth = count_set(probe.rule.birth());
    let survive = count_set(probe.rule.survive());
    let states = probe.rule.states();
    if !probe.inside {
        return "Outside the world: always empty.".to_string();
    }
    match probe.state {
        0 if probe.next_state == 1 => format!("Born: N = {n} is in the birth set {birth}."),
        0 => format!("Stays empty: N = {n} is not in the birth set {birth}."),
        1 if probe.next_state == 1 => format!("Survives: N = {n} is in the survival set {survive}."),
        1 if states > 2 => format!("Starts dying (state 2): N = {n} is not in the survival set {survive}."),
        1 => format!("Dies: N = {n} is not in the survival set {survive}."),
        k if probe.next_state == 0 => format!("Becomes empty: {k} is the last dying state."),
        k => format!("Fades to state {}: dying voxels ignore their neighbours.", k + 1),
    }
}

fn probe_section(ui: &mut egui::Ui, palette: &Palette, p: &mut PropertiesParams, rule: &Rule3D) {
    let Some(pos) = p.probe.pinned else {
        if p.settings.tool == crate::settings::Tool::Inspect {
            section(ui, palette, "Selected voxel");
            hint(ui, palette, "Click a voxel in the world (Inspect tool) to read its state and fate.");
        }
        return;
    };
    section(ui, palette, "Selected voxel");
    let mut clear = false;
    card(ui, palette, |ui| {
        ui.horizontal(|ui| {
            ui.label(mono(format!("({}, {}, {})", pos[0], pos[1], pos[2]), 14.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clear = ui.small_button("Clear").clicked();
            });
        });
        match &p.probe.result {
            Some(result) if result.pos == pos => {
                let color =
                    if result.state == 0 { palette.text_muted } else { state_color32(result.state, rule.states()) };
                egui::Grid::new("view_voxel_probe").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
                    ui.label(RichText::new("State").color(palette.text_muted));
                    ui.horizontal(|ui| {
                        super::swatch_dot(ui, color);
                        ui.label(format!("{} ({})", state_name(result.state), result.state));
                    });
                    ui.end_row();
                    if result.state != 0 {
                        ui.label(RichText::new("Age").color(palette.text_muted));
                        ui.label(format!("{} generations", result.age));
                        ui.end_row();
                    }
                    ui.label(RichText::new("Alive neighbours").color(palette.text_muted));
                    ui.label(format!(
                        "N = {} of {} ({})",
                        result.alive_neighbours,
                        result.neighbourhood.size(),
                        result.neighbourhood.label()
                    ));
                    ui.end_row();
                    ui.label(RichText::new("Next generation").color(palette.text_muted));
                    ui.label(state_name(result.next_state));
                    ui.end_row();
                });
                ui.label(RichText::new(fate_text(result)).size(12.0).color(palette.text));
                ui.label(
                    RichText::new(format!("Read at generation {}", count(result.tick)))
                        .size(11.0)
                        .color(palette.text_muted),
                );
            }
            _ => hint(ui, palette, "Reading…"),
        }
    });
    if clear {
        p.probe.pinned = None;
    }
}

fn region_section(ui: &mut egui::Ui, palette: &Palette, p: &PropertiesParams, states: u16) {
    if !p.settings.region.enabled {
        return;
    }
    let bounds = p.settings.region.bounds();
    section(ui, palette, "Region");
    card(ui, palette, |ui| {
        ui.label(
            mono(
                format!(
                    "x {}..{} · y {}..{} · z {}..{}",
                    bounds.min[0], bounds.max[0], bounds.min[1], bounds.max[1], bounds.min[2], bounds.max[2]
                ),
                12.0,
            )
            .color(palette.text),
        );
        let Some(counts) = &p.region.counts else {
            hint(ui, palette, "Counting…");
            return;
        };
        if counts.bounds.is_none() {
            hint(ui, palette, "The box lies outside the world.");
            return;
        }
        let population = counts.population();
        let alive = counts.counts.get(1).copied().unwrap_or(0);
        let share = if counts.volume > 0 { population as f64 / counts.volume as f64 * 100.0 } else { 0.0 };
        ui.label(
            RichText::new(format!("{} non-empty of {} voxels ({share:.1}%)", count(population), count(counts.volume)))
                .size(12.5),
        );
        stacked_bar(
            ui,
            palette,
            &[(alive as f64, state_color32(1, states)), ((population - alive) as f64, dying_stops()[1])],
            8.0,
        );
        egui::Grid::new("view_voxel_region_counts").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
            let shown: Vec<usize> =
                if counts.counts.len() <= 12 { (1..counts.counts.len()).collect() } else { vec![1] };
            for s in shown {
                ui.horizontal(|ui| {
                    super::swatch_dot(ui, state_color32(s as u8, states));
                    ui.label(RichText::new(state_name(s as u8)).color(palette.text_muted));
                });
                ui.label(count(counts.counts[s]));
                ui.end_row();
            }
            if counts.counts.len() > 12 {
                ui.label(RichText::new("dying (all)").color(palette.text_muted));
                ui.label(count(population - alive));
                ui.end_row();
            }
        });
    });
}

fn slice_section(ui: &mut egui::Ui, palette: &Palette, p: &mut PropertiesParams, states: u16) {
    egui::CollapsingHeader::new(RichText::new("2D slice inspector").strong().size(13.5))
        .id_salt("view_voxel_slice_section")
        .default_open(true)
        .show(ui, |ui| {
            p.slice.wanted = true;
            let layers = p.world.layers();
            let mut inspector = p.settings.inspector;
            let following = inspector.follow_cut && p.settings.cut.enabled;
            // The layer actually shown (the upper cut plane while following).
            let (axis, layer) = p.settings.inspector_layer();
            ui.horizontal(|ui| {
                let mut shown_axis = axis;
                ui.add_enabled_ui(!following, |ui| {
                    if segmented(ui, palette, &mut shown_axis, &[(Axis::X, "x"), (Axis::Y, "y"), (Axis::Z, "z")]) {
                        let (lo, hi) = axis_range(layers, shown_axis);
                        inspector.axis = shown_axis;
                        inspector.layer = lo + (hi - lo) / 2;
                    }
                });
                ui.checkbox(&mut inspector.follow_cut, "Follow cut")
                    .on_hover_text("While the cut is on, show the layer of its upper plane");
            });
            fitted(ui, "slice_layer", 125.0, |ui| {
                if following {
                    let (lo, hi) = axis_range(layers, axis);
                    let mut shown = layer;
                    ui.add_enabled(
                        false,
                        egui::Slider::new(&mut shown, lo.min(layer)..=hi.max(layer))
                            .text(format!("{} (cut)", axis_label(axis))),
                    );
                } else {
                    let (lo, hi) = axis_range(layers, inspector.axis);
                    inspector.layer = inspector.layer.clamp(lo, hi);
                    ui.add(
                        egui::Slider::new(&mut inspector.layer, lo..=hi)
                            .text(format!("{} layer", axis_label(inspector.axis))),
                    );
                }
            });
            if inspector != p.settings.inspector {
                p.settings.inspector = inspector;
            }
            let Some(view) = p.slice.view.as_ref() else {
                hint(ui, palette, "Reading the layer…");
                return;
            };
            // The texture is uploaded only when a new slice arrived.
            if p.slice.version != p.ui_state.slice_texture.as_ref().map_or(0, |(_, v)| *v) {
                let image = slice_image(view, states, palette.background);
                match p.ui_state.slice_texture.as_mut() {
                    Some((handle, version)) => {
                        handle.set(image, egui::TextureOptions::NEAREST);
                        *version = p.slice.version;
                    }
                    None => {
                        let handle = ui.ctx().load_texture("view_voxel_slice", image, egui::TextureOptions::NEAREST);
                        p.ui_state.slice_texture = Some((handle, p.slice.version));
                    }
                }
            }
            let Some((handle, _)) = &p.ui_state.slice_texture else { return };
            let (w, h) = (view.window.width.max(1) as f32, view.window.height.max(1) as f32);
            let width = ui.available_width().min(420.0);
            let scale = (width / w).min(360.0 / h);
            let size = egui::vec2(w * scale, h * scale);
            let response =
                ui.add(egui::Image::new(egui::load::SizedTexture::new(handle.id(), size)).sense(egui::Sense::hover()));
            let rect = response.rect;
            let painter = ui.painter_at(rect);
            if scale >= 6.0 {
                let stroke = Stroke::new(1.0, interface::theme::with_alpha(palette.stroke, 0.6));
                for i in 1..view.window.width {
                    let x = rect.min.x + i as f32 * scale;
                    painter.line_segment([egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)], stroke);
                }
                for j in 1..view.window.height {
                    let y = rect.min.y + j as f32 * scale;
                    painter.line_segment([egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)], stroke);
                }
            }
            painter.rect_stroke(rect, 2.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Outside);
            let (u_axis, v_axis) = view.axis.plane_axes();
            let readout = response.hover_pos().and_then(|pos| {
                let i = ((pos.x - rect.min.x) / scale).floor() as i64;
                let j_top = ((pos.y - rect.min.y) / scale).floor() as i64;
                let (wi, hi_) = (i64::from(view.window.width), i64::from(view.window.height));
                if !(0..wi).contains(&i) || !(0..hi_).contains(&j_top) {
                    return None;
                }
                let j = hi_ - 1 - j_top;
                let cell = egui::Rect::from_min_size(
                    egui::pos2(rect.min.x + i as f32 * scale, rect.min.y + j_top as f32 * scale),
                    egui::vec2(scale, scale),
                );
                painter.rect_stroke(cell, 0.0, Stroke::new(1.5, palette.accent), egui::StrokeKind::Inside);
                let mut pos3 = [view.layer; 3];
                pos3[u_axis.index()] = view.window.u0 + i as i32;
                pos3[v_axis.index()] = view.window.v0 + j as i32;
                let state = *view.cells.get(i as usize, j as usize);
                Some((pos3, state))
            });
            ui.label(
                RichText::new(match readout {
                    Some((pos, state)) => format!("({}, {}, {}) · {}", pos[0], pos[1], pos[2], state_name(state)),
                    None => format!(
                        "{} = {} · {} right, {} up · hover to read voxels",
                        axis_label(view.axis),
                        view.layer,
                        axis_label(u_axis),
                        axis_label(v_axis)
                    ),
                })
                .size(11.5)
                .color(palette.text_muted),
            );
            let (alive, dying) = p.slice.counts;
            ui.horizontal_wrapped(|ui| {
                legend_chip(ui, state_color32(1, states), &format!("alive {}", count(alive)));
                if states > 2 {
                    legend_chip(ui, dying_stops()[1], &format!("dying {}", count(dying)));
                }
                legend_chip(ui, palette.background, "empty");
            });
            if !following {
                ui.horizontal(|ui| {
                    super::swatch_dot(ui, crate::scene::inspector_color(palette));
                    ui.add(
                        egui::Label::new(
                            RichText::new("The teal frame in the world marks this layer.")
                                .size(11.5)
                                .color(palette.text_muted),
                        )
                        .wrap(),
                    );
                });
            }
            if view.truncated {
                hint(ui, palette, "The layer is larger than 4096 voxels per side; only part of it is shown.");
            }
        });
}

/// The slice as an image: `u` to the right, `v` upwards.
pub(crate) fn slice_image(view: &SliceView, states: u16, empty: Color32) -> egui::ColorImage {
    let (w, h) = (view.cells.width(), view.cells.height());
    let mut pixels = Vec::with_capacity(w * h);
    for row in 0..h {
        let j = h - 1 - row;
        for i in 0..w {
            let state = *view.cells.get(i, j);
            pixels.push(if state == 0 { empty } else { state_color32(state, states) });
        }
    }
    egui::ColorImage::new([w, h], pixels)
}

fn series(history: &MetricsHistory, key: &str, from: u64) -> Vec<[f64; 2]> {
    history
        .store()
        .get(key)
        .map(|s| s.window(from, u64::MAX).iter().map(|p| [p.tick as f64, p.value]).collect())
        .unwrap_or_default()
}

fn plots_section(ui: &mut egui::Ui, palette: &Palette, p: &PropertiesParams) {
    egui::CollapsingHeader::new(RichText::new("Plots").strong().size(13.5))
        .id_salt("view_voxel_plots")
        .default_open(true)
        .show(ui, |ui| {
            let Some(last) = p.history.store().last_tick() else {
                hint(ui, palette, "Run or step the simulation to record metrics.");
                return;
            };
            let from = last.saturating_sub(PLOT_SPAN);
            let population = series(&p.history, "population", from);
            let store = p.history.store();
            let alive: Vec<[f64; 2]> = population
                .iter()
                .map(|&[t, _]| [t, store.get("state_1").and_then(|s| s.value_at(t as u64)).unwrap_or(0.0)])
                .collect();
            let dying: Vec<[f64; 2]> =
                population.iter().zip(&alive).map(|(&[t, pop], &[_, a])| [t, (pop - a).max(0.0)]).collect();
            let states = p.world.info.as_ref().map_or(2, |i| i.rule.states());
            hint(ui, palette, &format!("Last {} generations; the Metrics tab has the full history.", PLOT_SPAN));
            let plot = |id: &str| {
                Plot::new(id.to_string())
                    .height(130.0)
                    .allow_drag(false)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .allow_boxed_zoom(false)
                    .allow_double_click_reset(false)
                    .include_y(0.0)
                    .x_axis_label("generation")
                    .legend(Legend::default().position(egui_plot::Corner::LeftTop))
            };
            plot("view_voxel_plot_population").show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("population", PlotPoints::from(population.clone())).color(palette.text_muted).width(1.5),
                );
                plot_ui.line(Line::new("alive", PlotPoints::from(alive)).color(state_color32(1, states)).width(1.8));
                if states > 2 {
                    plot_ui.line(Line::new("dying", PlotPoints::from(dying)).color(dying_stops()[1]).width(1.5));
                }
            });
            ui.add_space(4.0);
            plot("view_voxel_plot_births").show(ui, |plot_ui| {
                plot_ui.line(
                    Line::new("births", PlotPoints::from(series(&p.history, "births", from)))
                        .color(palette.success)
                        .width(1.5),
                );
                plot_ui.line(
                    Line::new("deaths", PlotPoints::from(series(&p.history, "deaths", from)))
                        .color(palette.error)
                        .width(1.5),
                );
            });
        });
}

fn render_section(ui: &mut egui::Ui, palette: &Palette, p: &PropertiesParams) {
    egui::CollapsingHeader::new(RichText::new("Rendering").strong().size(13.5))
        .id_salt("view_voxel_render")
        .default_open(false)
        .show(ui, |ui| {
            let summary = &p.mesh.summary;
            if summary.truncated {
                ui.label(
                    RichText::new(format!(
                        "Showing {} of {} visible voxels. Raise the cap (Display) or cut the volume.",
                        count(summary.listed as u64),
                        count(summary.visible)
                    ))
                    .color(palette.warning)
                    .size(12.0),
                );
            }
            egui::Grid::new("view_voxel_render_stats").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(RichText::new(k).color(palette.text_muted));
                    ui.label(v);
                    ui.end_row();
                };
                row("Visible voxels", count(summary.visible));
                row("Drawn voxels", count(summary.drawn as u64));
                row("Chunks (16³)", count(p.mesh.chunk_count() as u64));
                row("Faces", count(p.mesh.faces as u64));
                row("Last mesh build", format!("{:.1} ms", p.mesh.build_ms));
                if summary.out_of_range > 0 {
                    row("Beyond ±2^24 (not drawn)", count(summary.out_of_range as u64));
                }
            });
            if let Some(error) = &p.mesh.error {
                ui.label(RichText::new(error).color(palette.error).size(12.0));
            }
            hint(
                ui,
                palette,
                "Only voxels with an exposed face are listed by the model; meshes are rebuilt in the background \
                 and only for chunks that changed.",
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::Grid2;
    use voxel_life::{Neighbourhood, SliceWindow};

    fn probe(state: u8, n: u32, next: u8, rule: &str) -> ProbeResult {
        ProbeResult {
            pos: [1, 2, 3],
            state,
            age: 0,
            alive_neighbours: n,
            neighbourhood: Neighbourhood::Moore,
            next_state: next,
            rule: Rule3D::parse(rule).expect("valid"),
            tick: 0,
            inside: true,
        }
    }

    #[test]
    fn fates_explain_the_rule() {
        assert_eq!(fate_text(&probe(0, 6, 1, "5766")), "Born: N = 6 is in the birth set {6}.");
        assert!(fate_text(&probe(0, 5, 0, "5766")).starts_with("Stays empty"));
        assert!(fate_text(&probe(1, 6, 1, "5766")).contains("survival set {5–7}"));
        assert!(fate_text(&probe(1, 3, 0, "5766")).starts_with("Dies"));
        assert!(fate_text(&probe(1, 3, 2, "4/4/5/M")).starts_with("Starts dying"));
        assert!(fate_text(&probe(3, 0, 4, "4/4/5/M")).starts_with("Fades to state 4"));
        assert!(fate_text(&probe(4, 0, 0, "4/4/5/M")).starts_with("Becomes empty"));
        let outside = ProbeResult { inside: false, ..probe(0, 0, 0, "5766") };
        assert!(fate_text(&outside).starts_with("Outside"));
    }

    #[test]
    fn slice_images_put_v_up() {
        // 2 x 3 layer: alive at (u=0, v=2) (top-left) and dying at (1, 0).
        let mut cells = Grid2::new(2, 3, 0u8);
        cells.set(0, 2, 1);
        cells.set(1, 0, 2);
        let view = SliceView {
            tick: 0,
            axis: Axis::Z,
            layer: 0,
            window: SliceWindow { u0: 0, v0: 0, width: 2, height: 3 },
            cells,
            truncated: false,
        };
        let empty = Color32::from_rgb(1, 2, 3);
        let image = slice_image(&view, 3, empty);
        assert_eq!(image.size, [2, 3]);
        assert_eq!(image.pixels[0], state_color32(1, 3), "top-left is the highest v");
        assert_eq!(image.pixels[5], state_color32(2, 3), "bottom-right is v = 0, u = 1");
        assert_eq!(image.pixels[1], empty);
    }
}
