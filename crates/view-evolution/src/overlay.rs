//! Viewport overlay: experiment status, phase information, colour legend,
//! scale bar and the hover card of the organism under the cursor.

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Stroke, pos2, vec2};
use evolution::genes::{GENES, PERCEPTION, SIZE, SPEED, TEMP_OPTIMUM};
use evolution::{SpeciationStatus, Species};
use interface::theme::with_alpha;
use interface::{ModeRegistry, Palette, SessionStatus, ThemeSettings};
use rendering::WorldInput;

use crate::morph::{thermal_max, thermal_min, thermal_rgb, thermal_t};
use crate::scene::{EvoCamera, region_rgb};
use crate::session::{EvoData, express};
use crate::tools_panel::palette;
use crate::ui_state::{EvoUi, GroundOverlay};
use crate::widgets::{color32, thermal_color32};

/// Screen size of one world cell near the camera focus.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ViewScale {
    /// Logical pixels per world unit at the focus (`None` if unknown).
    pub px_per_cell: Option<f32>,
}

/// A length of 1, 2 or 5 × 10^n cells that is about `target_px` long.
pub fn scale_bar(px_per_cell: f32, target_px: f32) -> Option<(f32, f32)> {
    if !(px_per_cell.is_finite() && px_per_cell > 0.0) {
        return None;
    }
    let raw = target_px / px_per_cell;
    let mag = 10f32.powf(raw.log10().floor());
    let cells = [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * mag).rev().find(|c| *c <= raw).unwrap_or(mag);
    Some((cells, cells * px_per_cell))
}

/// Measures the screen scale at the camera focus.
pub fn measure_scale(
    cameras: Query<(&Camera, &GlobalTransform, &bevy_panorbit_camera::PanOrbitCamera), With<EvoCamera>>,
    mut scale: ResMut<ViewScale>,
) {
    let measured = cameras.iter().next().and_then(|(camera, transform, pan)| {
        let a = camera.world_to_viewport(transform, pan.focus).ok()?;
        let b = camera.world_to_viewport(transform, pan.focus + Vec3::X).ok()?;
        let c = camera.world_to_viewport(transform, pan.focus + Vec3::Z).ok()?;
        Some(a.distance(b).max(a.distance(c)))
    });
    scale.px_per_cell = measured;
}

/// Label of a scale bar length.
pub fn scale_label(cells: f32) -> String {
    let value = if cells >= 1.0 { format!("{cells:.0}") } else { format!("{cells:.1}") };
    let unit = if (cells - 1.0).abs() < 1e-3 { "cell" } else { "cells" };
    format!("{value} {unit} at focus")
}

fn speciation_short(status: &SpeciationStatus) -> (String, bool) {
    match status {
        SpeciationStatus::NotApplicable(_) => ("speciation: n/a".into(), false),
        SpeciationStatus::Monitoring => ("speciation: monitoring".into(), false),
        SpeciationStatus::Separated { generations, .. } => {
            (format!("clusters separated for {generations:.1} gen"), true)
        }
        SpeciationStatus::Speciated(e) => (format!("speciation declared at tick {}", e.tick), true),
    }
}

/// Overlay panel system.
#[allow(clippy::too_many_arguments)]
pub fn overlay_panel(
    InMut(ui): InMut<egui::Ui>,
    mut state: ResMut<EvoUi>,
    data: Res<EvoData>,
    status: Res<SessionStatus>,
    scale: Res<ViewScale>,
    input: Res<WorldInput>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
) {
    let palette = palette(&theme, &registry);
    let Some(view) = data.view.as_ref() else { return };
    let frame = egui::Frame::new()
        .fill(with_alpha(palette.panel, 0.9))
        .stroke(Stroke::new(palette.stroke_width, palette.stroke))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8));
    frame.show(ui, |ui| {
        ui.set_max_width(250.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&data.scenario.name).strong().size(13.0));
            let state_text = match status.get() {
                Some(s) if s.finished => ("FINISHED", palette.warning),
                Some(s) if s.playing => ("RUNNING", palette.success),
                _ => ("PAUSED", palette.text_muted),
            };
            ui.label(RichText::new(state_text.0).size(10.0).color(state_text.1));
        });
        let prey = &view.stats.prey;
        let mut stats = vec![
            (prey.count.to_string(), "prey"),
            (view.stats.predators.count.to_string(), "predators"),
            (format!("{:.1}", prey.mean_generation), "mean gen."),
        ];
        if let Some(env) = &data.env {
            stats.push((format!("{:.1}", env.mean_temperature), "land °C"));
        }
        egui::Grid::new("evo_overlay_stats").num_columns(stats.len()).spacing([14.0, 0.0]).show(ui, |ui| {
            for (value, _) in &stats {
                ui.label(RichText::new(value).strong().size(14.0).color(palette.text));
            }
            ui.end_row();
            for (_, label) in &stats {
                ui.label(RichText::new(*label).size(9.5).color(palette.text_muted));
            }
            ui.end_row();
        });
        let mut line = |text: String, color: Color32| {
            ui.label(RichText::new(text).size(11.0).color(color));
        };
        if let Some(env) = &data.env
            && env.shift_rate.abs() > 1e-3
        {
            line(format!("climate trend {:+.1} °C per 1000 ticks", env.shift_rate), palette.text_muted);
        }
        let (spec, highlight) = speciation_short(&view.speciation);
        let regions = view.stats.regions.iter().filter(|r| r.prey > 0).count();
        line(
            format!("{spec} · F_ST {:.2} · {regions} region{}", view.stats.fst, if regions == 1 { "" } else { "s" }),
            if highlight { palette.warning } else { palette.text_muted },
        );
        if state.tool != crate::ui_state::Tool::Select {
            line(format!("tool: {} · radius {:.0}", state.tool.label(), state.brush_radius), palette.accent);
        }
        if let Some(e) = view.recent_events.first()
            && view.tick.saturating_sub(e.tick) < 400
        {
            ui.label(
                RichText::new(format!("tick {}: {}", e.tick, e.message)).size(10.5).italics().color(palette.text_muted),
            );
        }

        ui.add_space(2.0);
        let toggle = if state.legend_open { "⏷ legend" } else { "⏵ legend" };
        if ui
            .add(egui::Label::new(RichText::new(toggle).size(10.5).color(palette.accent)).sense(egui::Sense::click()))
            .clicked()
        {
            state.legend_open = !state.legend_open;
        }
        if state.legend_open {
            legend(ui, &palette, &state, &data);
        }
        if let Some((cells, px)) = scale.px_per_cell.and_then(|p| scale_bar(p, 90.0)) {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(px.min(200.0), 8.0), egui::Sense::hover());
                let p = ui.painter();
                let stroke = Stroke::new(1.5, palette.text);
                p.line_segment([pos2(rect.left(), rect.center().y), pos2(rect.right(), rect.center().y)], stroke);
                p.line_segment([rect.left_top(), rect.left_bottom()], stroke);
                p.line_segment([rect.right_top(), rect.right_bottom()], stroke);
                ui.label(RichText::new(scale_label(cells)).size(10.5).color(palette.text_muted));
            });
        }
    });
    hover_card(ui.ctx(), &palette, &state, &data, &input);
}

fn legend(ui: &mut egui::Ui, palette: &Palette, state: &EvoUi, data: &EvoData) {
    // Thermal scale shared by prey coats and the ground tint.
    let thermal_ground = state.overlay == GroundOverlay::Temperature;
    let title = if thermal_ground {
        "Prey coat = thermal optimum · ground = temperature"
    } else {
        "Prey coat = thermal optimum"
    };
    ui.label(RichText::new(title).size(11.0));
    let (rect, _) = ui.allocate_exact_size(vec2(230.0, 24.0), egui::Sense::hover());
    let bar = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 9.0));
    let steps = 46;
    let painter = ui.painter();
    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let r = egui::Rect::from_min_max(
            pos2(bar.left() + bar.width() * t0, bar.top()),
            pos2(bar.left() + bar.width() * (i + 1) as f32 / steps as f32 + 0.5, bar.bottom()),
        );
        painter.rect_filled(r, 0.0, thermal_color32(t0 + 0.5 / steps as f32));
    }
    let span = thermal_max() - thermal_min();
    for k in 0..=3 {
        let t = k as f32 / 3.0;
        let x = bar.left() + bar.width() * t;
        let align = match k {
            0 => egui::Align2::LEFT_TOP,
            3 => egui::Align2::RIGHT_TOP,
            _ => egui::Align2::CENTER_TOP,
        };
        painter
            .line_segment([pos2(x, bar.bottom()), pos2(x, bar.bottom() + 3.0)], Stroke::new(1.0, palette.text_muted));
        painter.text(
            pos2(x, bar.bottom() + 3.0),
            align,
            format!("{:.0} °C", thermal_min() + span * t),
            egui::FontId::proportional(9.5),
            palette.text_muted,
        );
    }
    // Where the ground is now, on the same scale.
    if thermal_ground && let Some((lo, hi)) = data.env.as_ref().and_then(|e| e.land_temperature_range()) {
        let (a, b) = (bar.left() + bar.width() * thermal_t(lo), bar.left() + bar.width() * thermal_t(hi));
        painter.rect_stroke(
            egui::Rect::from_min_max(pos2(a - 1.5, bar.top() - 1.5), pos2(b.max(a + 1.0) + 1.5, bar.bottom() + 1.5)),
            2.0,
            Stroke::new(1.5, palette.text),
            egui::StrokeKind::Middle,
        );
    }
    let ground = match state.overlay {
        GroundOverlay::Temperature => {
            "White frame: temperatures present on land. A prey wearing its ground's hue lives at its optimum. \
             Darker ground holds less food."
        }
        GroundOverlay::Food => "Ground = food per cell, from bare soil to green.",
        GroundOverlay::Regions => "Ground = regions (islands or sectors).",
        GroundOverlay::Plain => "Ground = neutral.",
    };
    ui.label(RichText::new(ground).size(10.5).color(palette.text_muted));
    if state.overlay == GroundOverlay::Regions
        && let Some(env) = &data.env
    {
        ui.horizontal_wrapped(|ui| {
            for r in 0..env.region_count.min(8) as u16 {
                ui.colored_label(color32(region_rgb(r)), RichText::new(format!("■ R{r}")).size(10.5));
            }
        });
    }
    ui.label(
        RichText::new("Body: size = scale · speed = legs & posture · perception = ears & eyes")
            .size(10.5)
            .color(palette.text_muted),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.colored_label(Color32::from_rgb(88, 94, 106), RichText::new("■").size(11.0));
        ui.label(RichText::new("predators: dark canids · grass height = food").size(10.5).color(palette.text_muted));
    });
}

fn hover_card(ctx: &egui::Context, palette: &Palette, state: &EvoUi, data: &EvoData, input: &WorldInput) {
    let (Some(id), Some(cursor), Some(view)) = (state.hovered, input.cursor, data.view.as_ref()) else { return };
    if !input.pointer {
        return;
    }
    let Ok(i) = view.organisms.binary_search_by_key(&id, |o| o.id) else { return };
    let o = &view.organisms[i];
    let genome = data.traits.get(&id).map(|(_, g)| g);
    egui::Area::new(egui::Id::new("evo_hover_card"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos2(cursor.x + 16.0, cursor.y + 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(220.0);
                let species = match o.species {
                    Species::Prey => "Prey",
                    Species::Predator => "Predator",
                };
                ui.label(RichText::new(format!("{species} {id} · gen {}", o.generation)).strong().size(12.0));
                ui.label(
                    RichText::new(format!("energy {:.0} % · health {:.2}", o.condition * 100.0, o.health))
                        .size(11.0)
                        .color(palette.text_muted),
                );
                if let Some(g) = genome {
                    for gene in [SIZE, SPEED, PERCEPTION, TEMP_OPTIMUM] {
                        let v = express(gene, g[gene]);
                        let text = format!("{} {v:.2} {}", GENES[gene].label, GENES[gene].unit);
                        if gene == TEMP_OPTIMUM {
                            ui.colored_label(color32(thermal_rgb(thermal_t(v))), RichText::new(text).size(11.0));
                        } else {
                            ui.label(RichText::new(text).size(11.0));
                        }
                    }
                }
                ui.label(RichText::new("click to inspect").size(10.0).italics().color(palette.text_muted));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_bar_picks_round_lengths() {
        let (cells, px) = scale_bar(9.0, 90.0).unwrap();
        assert_eq!(cells, 10.0);
        assert!((px - 90.0).abs() < 1e-4);
        let (cells, _) = scale_bar(3.0, 90.0).unwrap();
        assert_eq!(cells, 20.0);
        let (cells, px) = scale_bar(100.0, 90.0).unwrap();
        assert!(cells < 1.0 && px <= 90.0);
        assert!(scale_bar(0.0, 90.0).is_none());
        assert!(scale_bar(f32::NAN, 90.0).is_none());
        assert_eq!(scale_label(1.0), "1 cell at focus");
        assert_eq!(scale_label(0.5), "0.5 cells at focus");
        assert_eq!(scale_label(20.0), "20 cells at focus");
    }

    #[test]
    fn speciation_summary_flags_progress() {
        assert!(!speciation_short(&SpeciationStatus::Monitoring).1);
        let (text, flag) =
            speciation_short(&SpeciationStatus::Separated { since_tick: 1, since_generation: 2.0, generations: 3.0 });
        assert!(flag && text.contains("3.0"));
    }
}
