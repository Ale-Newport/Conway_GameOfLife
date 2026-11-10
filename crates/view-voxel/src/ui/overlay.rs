//! Overlay over the world: rule, "not Conway" reminder, generation and
//! population, phase, colour legend, cut, region, pointer and scale.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui::{self, CornerRadius, Margin, RichText, Stroke};
use interface::theme::with_alpha;
use interface::widgets::{gradient_bar, legend_chip};
use interface::{MetricsHistory, ModeRegistry, Palette, SessionStatus, ThemeSettings};

use super::{UiState, count, mono};
use crate::MODE;
use crate::build::MeshState;
use crate::colors::{dying_stops, ramp_stops, state_color32};
use crate::interact::PointerState;
use crate::phase::PhaseInfo;
use crate::requests::{RegionState, SliceState, WorldState, current_counts};
use crate::scene::grid_step;
use crate::settings::{AgeScale, ColorMode, Transparency, ViewSettings, axis_label, axis_range};

#[cfg(test)]
/// Non-ASCII characters used in the overlay (checked against egui's fonts).
pub(crate) const GLYPHS: &str = "⬣–·×…≥";

/// Resources the overlay reads.
#[derive(SystemParam)]
pub(crate) struct OverlayParams<'w> {
    status: Res<'w, SessionStatus>,
    world: Res<'w, WorldState>,
    settings: Res<'w, ViewSettings>,
    phase: Res<'w, PhaseInfo>,
    region: Res<'w, RegionState>,
    slice: Res<'w, SliceState>,
    mesh: Res<'w, MeshState>,
    pointer: Res<'w, PointerState>,
    ui_state: ResMut<'w, UiState>,
    history: Res<'w, MetricsHistory>,
    theme: Res<'w, ThemeSettings>,
    registry: Res<'w, ModeRegistry>,
}

/// The overlay.
pub(crate) fn overlay_panel(InMut(ui): InMut<egui::Ui>, mut p: OverlayParams) {
    let palette = Palette::new(p.theme.kind, p.registry.accent(MODE));
    let Some(info) = p.world.info.clone() else { return };
    let glyph = p.registry.card(MODE).map_or("⬣".to_string(), |c| c.glyph.clone());
    egui::Frame::new()
        .fill(with_alpha(palette.panel, 0.9))
        .stroke(Stroke::new(palette.stroke_width, with_alpha(palette.stroke, 0.9)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_max_width(310.0);
            ui.spacing_mut().item_spacing.y = 3.0;
            let mut collapsed = p.ui_state.overlay_collapsed;
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).size(15.0).color(palette.accent));
                ui.label(RichText::new("3D LIFE").size(11.0).strong().color(palette.text_muted));
                ui.label(mono(info.rule.to_softology(), 13.5).strong().color(palette.accent));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if collapsed { "+" } else { "–" };
                    if ui.small_button(label).on_hover_text(if collapsed { "Expand" } else { "Collapse" }).clicked() {
                        collapsed = !collapsed;
                    }
                });
            });
            if collapsed != p.ui_state.overlay_collapsed {
                p.ui_state.overlay_collapsed = collapsed;
            }
            if collapsed {
                return;
            }
            let n = info.rule.neighbourhood();
            ui.label(
                RichText::new(format!("3D rule on {} neighbours, not Conway's 2D Life", n.size()))
                    .size(11.5)
                    .italics()
                    .color(palette.text_muted),
            );
            ui.add_space(2.0);
            if let Some(status) = p.status.get() {
                let counts = current_counts(&p.history, Some(&info), status.tick);
                let pop_text = counts.map_or("—".to_string(), |(population, _)| count(population));
                ui.label(
                    RichText::new(format!("Generation {} · population {pop_text}", count(status.tick)))
                        .size(13.0)
                        .color(palette.text),
                );
            }
            ui.label(RichText::new(p.phase.phase.label()).size(12.0).color(palette.text));
            if let Some(halt) = &info.halt {
                ui.label(RichText::new(format!("Halted: {halt}")).size(11.5).color(palette.error));
            }
            ui.add_space(4.0);
            legend(ui, &palette, &p, info.rule.states());
            ui.add_space(2.0);
            let layers = p.world.layers();
            if p.settings.cut.enabled {
                let cut = p.settings.cut.range();
                let (lo, hi) = axis_range(layers, cut.axis);
                ui.label(
                    RichText::new(format!("Cut: {} {}–{} of {lo}–{hi}", axis_label(cut.axis), cut.min, cut.max))
                        .size(11.5)
                        .color(palette.accent),
                );
            }
            let follows_cut = p.settings.inspector.follow_cut && p.settings.cut.enabled;
            if p.slice.shown_recently() && !follows_cut {
                let (axis, layer) = p.settings.inspector_layer();
                ui.label(
                    RichText::new(format!("Teal frame: slice inspector layer {} = {layer}", axis_label(axis)))
                        .size(11.5)
                        .color(crate::scene::inspector_color(&palette)),
                );
            }
            if p.settings.region.enabled
                && let Some(counts) = &p.region.counts
            {
                let share =
                    if counts.volume > 0 { counts.population() as f64 / counts.volume as f64 * 100.0 } else { 0.0 };
                ui.label(
                    RichText::new(format!("Region: {} voxels ({share:.1}% of the box)", count(counts.population())))
                        .size(11.5)
                        .color(palette.accent),
                );
            }
            if let Some(voxel) = p.pointer.hover.and_then(|h| h.voxel.or(h.place)) {
                ui.label(
                    RichText::new(format!("Pointer: ({}, {}, {})", voxel.x, voxel.y, voxel.z))
                        .size(11.5)
                        .color(palette.text_muted),
                );
            }
            let [w, h, d] = info.config.size;
            let world_text = if info.is_finite() {
                format!("World {w} × {h} × {d} voxels")
            } else {
                format!("Unbounded · start box {w} × {h} × {d}")
            };
            let span = ((layers.max[0] - layers.min[0]).max(layers.max[1] - layers.min[1]) + 1) as f32;
            let grid = grid_step(span, 32.0) as u32;
            let grid_text = if p.settings.show_floor {
                format!(" · grid {grid} voxel{}", if grid == 1 { "" } else { "s" })
            } else {
                String::new()
            };
            ui.label(RichText::new(format!("{world_text}{grid_text}")).size(11.5).color(palette.text_muted));
            if p.settings.show_axes {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (name, color) in [
                        ("x", egui::Color32::from_rgb(237, 84, 79)),
                        ("y", egui::Color32::from_rgb(102, 204, 92)),
                        ("z up", egui::Color32::from_rgb(89, 140, 250)),
                    ] {
                        ui.label(RichText::new(name).size(11.5).strong().color(color));
                    }
                    ui.label(RichText::new("axes from the origin corner").size(11.0).color(palette.text_muted));
                });
            }
        });
}

fn legend(ui: &mut egui::Ui, palette: &Palette, p: &OverlayParams, states: u16) {
    match p.settings.color_mode {
        ColorMode::State => {
            ui.horizontal_wrapped(|ui| {
                legend_chip(ui, state_color32(1, states), "alive (state 1)");
                if states > 2 {
                    let ghost = if p.settings.transparency == Transparency::GhostDying { ", translucent" } else { "" };
                    legend_chip(ui, dying_stops()[1], &format!("dying 2–{}{ghost}", states - 1));
                }
            });
            if states > 3 {
                gradient_bar(ui, &dying_stops(), 200.0, "freshly dying", "about to vanish");
            }
        }
        ColorMode::Age => {
            let top = match p.settings.age_scale {
                AgeScale::Auto => p.mesh.summary.age_top,
                AgeScale::Fixed(top) => top,
            };
            ui.label(RichText::new("Age (generations, logarithmic)").size(11.5).color(palette.text_muted));
            gradient_bar(ui, &ramp_stops(ColorMode::Age), 200.0, "0", &format!("≥ {top}"));
        }
        ColorMode::Height => {
            let (lo, hi) = p.world.height_range();
            ui.label(RichText::new("Height (z layer)").size(11.5).color(palette.text_muted));
            gradient_bar(ui, &ramp_stops(ColorMode::Height), 200.0, &format!("z = {lo}"), &format!("z = {hi}"));
        }
    }
    match p.settings.transparency {
        Transparency::Opaque => {}
        Transparency::GhostDying => {
            ui.label(RichText::new("Dying voxels translucent (ghost)").size(11.0).color(palette.text_muted));
        }
        Transparency::XRay => {
            ui.label(
                RichText::new("Drawn voxels translucent (X-ray, surface layers only)")
                    .size(11.0)
                    .color(palette.text_muted),
            );
        }
    }
    if p.settings.depth_shading && p.settings.transparency != Transparency::XRay {
        ui.label(RichText::new("Darker faces: crowded voxels (depth shading)").size(11.0).color(palette.text_muted));
    }
}
