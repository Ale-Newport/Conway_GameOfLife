//! Viewport overlay: phase card (run state, tick, pending reset, draft
//! status), legend (types in the current colour mode, field colormap with
//! its value range), scale bar, entity glyphs when zoomed in and the hover
//! card. Glyphs, the scale bar and the hover card are painted on a
//! background layer clipped to the world viewport, so they never take
//! pointer input from the world.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, RichText, Stroke};
use interface::theme::{color32_from_rgba, on_color, with_alpha};
use interface::widgets::{badge, gradient_bar};
use interface::{ModeRegistry, Palette, SessionStatus, ThemeSettings};
use rendering::WorldInput;
use rule_engine::CustomLifeView;

use crate::MODE;
use crate::colors::{ColorBy, ENERGY_LEVELS, energy_shade, field_range, state_shade};
use crate::editor::ClEditor;
use crate::glyphs::shown_icon;
use crate::session::ClData;
use crate::state::ClUi;
use crate::world::{ClCamera, camera_view, current_scale_bar, floor_color, sim_to_screen};

/// Glyphs are drawn when entities are at least this many pixels wide.
pub const GLYPH_MIN_PX: f32 = 13.0;
/// …and at most this many entities are visible.
pub const GLYPH_MAX_COUNT: usize = 3000;

/// Run phase shown in the overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Ticking.
    Running,
    /// Paused.
    Paused,
    /// An end condition finished the run.
    Finished(String),
}

impl Phase {
    /// Phase from the runner status and the view.
    pub fn of(playing: bool, finished: Option<&str>) -> Self {
        match (finished, playing) {
            (Some(reason), false) => Phase::Finished(reason.to_string()),
            (_, true) => Phase::Running,
            (None, false) => Phase::Paused,
        }
    }

    /// Label.
    pub fn label(&self) -> String {
        match self {
            Phase::Running => "RUNNING".into(),
            Phase::Paused => "PAUSED".into(),
            Phase::Finished(_) => "FINISHED".into(),
        }
    }
}

/// Colour stops of the field legend: the colormap over the share of it in use.
pub fn legend_stops(map: rendering::Colormap, intensity: f32) -> Vec<Color32> {
    (0..12)
        .map(|i| {
            let [r, g, b] = map.sample_srgb(i as f32 / 11.0 * intensity.clamp(0.05, 1.0));
            Color32::from_rgb((r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
        })
        .collect()
}

/// Compact number for legends.
pub fn short_number(v: f32) -> String {
    let a = v.abs();
    if a == 0.0 {
        "0".into()
    } else if !(0.01..10_000.0).contains(&a) {
        format!("{v:.2e}")
    } else if a < 1.0 {
        format!("{v:.3}")
    } else if a < 100.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.0}")
    }
}

/// Label of a scale bar of `units` cells: `1 cell`, `5 cells`, `200 cells`,
/// `0.5 cells` (bar lengths are 1, 2 or 5 × 10ⁿ, so whole numbers print
/// without decimals).
pub fn scale_label(units: f32) -> String {
    if units == 1.0 {
        "1 cell".to_string()
    } else if units >= 1.0 && units.fract() == 0.0 && units < 1.0e7 {
        format!("{} cells", units as u64)
    } else {
        format!("{} cells", short_number(units).trim_end_matches('0').trim_end_matches('.'))
    }
}

/// The overlay panel system.
#[allow(clippy::too_many_arguments)]
pub fn overlay_panel(
    InMut(ui): InMut<egui::Ui>,
    data: Res<ClData>,
    mut state: ResMut<ClUi>,
    editor: Res<ClEditor>,
    input: Res<WorldInput>,
    status: Res<SessionStatus>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    camera: Query<(&Transform, &Projection), With<ClCamera>>,
) {
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    let Some(view) = data.view.as_ref() else { return };
    let camera = camera.iter().next().and_then(|(t, p)| camera_view(&input, t, p));
    if let Some(camera) = camera {
        paint_world_layer(ui.ctx(), view, &state, &input, camera, &palette);
    }
    let card = egui::Frame::popup(ui.style()).fill(palette.panel).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_max_width(250.0);
        phase_card(ui, view, &data, &editor, &status, &palette);
        ui.separator();
        let header = egui::CollapsingHeader::new(RichText::new("Legend").small().strong())
            .id_salt("cl_overlay_legend")
            .open(Some(state.legend_open))
            .show(ui, |ui| legend(ui, view, &state, &data, &palette));
        if header.header_response.clicked() {
            state.legend_open = !state.legend_open;
        }
    });
    let r = card.response.rect;
    let rect = Rect::new(r.min.x, r.min.y, r.max.x, r.max.y);
    if state.overlay_card != Some(rect) {
        state.overlay_card = Some(rect);
    }
}

fn phase_card(
    ui: &mut egui::Ui,
    view: &CustomLifeView,
    data: &ClData,
    editor: &ClEditor,
    status: &SessionStatus,
    palette: &Palette,
) {
    let playing = status.get().is_some_and(|s| s.playing);
    let phase = Phase::of(playing, view.finished.as_deref());
    let color = match phase {
        Phase::Running => palette.success,
        Phase::Paused => palette.text_muted,
        Phase::Finished(_) => palette.warning,
    };
    ui.horizontal(|ui| {
        badge(ui, &phase.label(), color);
        ui.label(RichText::new(format!("tick {}", view.tick)).monospace().color(palette.text));
    });
    ui.label(RichText::new(&editor.draft.name).strong());
    if let Phase::Finished(reason) = &phase {
        ui.label(RichText::new(reason).small().color(palette.warning));
    }
    ui.horizontal_wrapped(|ui| {
        if view.reset_pending {
            badge(ui, "reset pending", palette.warning)
                .on_hover_text("Structural edits wait for the next reset (R) to take effect.");
        }
        if editor.is_edited(data.info.as_ref()) {
            badge(ui, "draft edited", palette.accent).on_hover_text("The editor holds edits that were not applied.");
        }
        if !view.script_errors.is_empty() {
            badge(ui, &format!("{} script error(s)", view.script_errors.len()), palette.error);
        }
        if editor.has_errors() {
            badge(ui, "draft has errors", palette.error);
        }
    });
}

fn legend(ui: &mut egui::Ui, view: &CustomLifeView, state: &ClUi, data: &ClData, palette: &Palette) {
    let total: u32 = view.legend.iter().map(|l| l.count).sum();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Entities").small().strong());
        ui.label(
            RichText::new(format!("{total} alive · colour = {}", state.color_by.label().to_lowercase()))
                .small()
                .color(palette.text_muted),
        );
    });
    for entry in &view.legend {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            match state.color_by {
                ColorBy::Type => {
                    interface::widgets::swatch(ui, color32_from_rgba(entry.color), 12.0);
                }
                ColorBy::Energy => {
                    for level in 0..ENERGY_LEVELS {
                        let c = color32_from_rgba(energy_shade(entry.color, level));
                        let (r, _) = ui.allocate_exact_size(egui::vec2(6.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 1.0, c);
                    }
                }
                ColorBy::State => {
                    for s in 0..entry.states.len().min(6) {
                        let c = color32_from_rgba(state_shade(entry.color, s as u16, entry.states.len()));
                        let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 1.0, c);
                    }
                }
            }
            let name = format!("{} {}", shown_icon(ui.ctx(), &entry.icon, &entry.name), entry.name);
            ui.label(RichText::new(name).size(12.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(entry.count.to_string()).monospace().size(12.0));
            });
        })
        .response
        .on_hover_text(if state.color_by == ColorBy::State {
            format!("{}\nstates: {}", entry.description, entry.states.join(", "))
        } else {
            entry.description.clone()
        });
    }
    if state.color_by == ColorBy::Energy {
        ui.label(
            RichText::new("darker = less energy (5 levels of the type's maximum)").small().color(palette.text_muted),
        );
    }
    if let Some(name) = &state.overlay
        && let Some(f) = view.fields.iter().find(|f| &f.name == name)
    {
        ui.separator();
        let kind = if f.resource { "resource" } else { "environment" };
        ui.label(RichText::new(format!("Field: {name} ({kind})")).small().strong());
        let resource_max = if f.resource {
            data.info.as_ref().and_then(|i| i.running.world.resources.iter().find(|r| &r.name == name)).map(|r| r.max)
        } else {
            None
        };
        let (lo, hi) = field_range(f.min, f.max, resource_max);
        gradient_bar(ui, &legend_stops(state.colormap, state.intensity), 200.0, &short_number(lo), &short_number(hi));
        ui.label(
            RichText::new(format!("now {} – {} per cell", short_number(f.min), short_number(f.max)))
                .small()
                .color(palette.text_muted),
        );
    }
}

fn paint_world_layer(
    ctx: &egui::Context,
    view: &CustomLifeView,
    state: &ClUi,
    input: &WorldInput,
    camera: (Vec2, f32, Rect),
    palette: &Palette,
) {
    let (_, scale, viewport) = camera;
    let clip = egui::Rect::from_min_max(
        egui::pos2(viewport.min.x, viewport.min.y),
        egui::pos2(viewport.max.x, viewport.max.y),
    );
    let painter = ctx
        .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("custom_life_world_layer")))
        .with_clip_rect(clip);
    // Glyphs.
    if state.glyphs && scale > 0.0 {
        let visible: Vec<_> = view
            .entities
            .iter()
            .filter_map(|e| {
                let px = e.size / scale;
                let s = sim_to_screen(e.position, camera);
                let r = px * 0.5;
                let inside = s.x + r >= viewport.min.x
                    && s.x - r <= viewport.max.x
                    && s.y + r >= viewport.min.y
                    && s.y - r <= viewport.max.y;
                (inside && px >= GLYPH_MIN_PX).then_some((e, s, px))
            })
            .collect();
        let icons: Vec<String> = view.legend.iter().map(|l| shown_icon(ctx, &l.icon, &l.name)).collect();
        if visible.len() <= GLYPH_MAX_COUNT {
            for (e, s, px) in visible {
                let Some(icon) = icons.get(usize::from(e.type_index)) else { continue };
                if icon.is_empty() {
                    continue;
                }
                let color = with_alpha(on_color(color32_from_rgba(e.color)), 0.9);
                painter.text(
                    egui::pos2(s.x, s.y),
                    Align2::CENTER_CENTER,
                    icon,
                    FontId::proportional((px * 0.55).clamp(8.0, 72.0)),
                    color,
                );
            }
        }
    }
    // Scale bar (bottom-left).
    let (units, px) = current_scale_bar(scale);
    if px > 0.0 {
        let origin = egui::pos2(clip.min.x + 16.0, clip.max.y - 18.0);
        let ink = if palette.dark { Color32::from_gray(230) } else { Color32::from_gray(30) };
        let shadow = with_alpha(floor_color(palette), 0.85);
        let bg = egui::Rect::from_min_max(origin + egui::vec2(-8.0, -22.0), origin + egui::vec2(px + 8.0, 8.0));
        painter.rect_filled(bg, 4.0, shadow);
        painter.line_segment([origin, origin + egui::vec2(px, 0.0)], Stroke::new(2.0, ink));
        for x in [0.0, px] {
            painter.line_segment([origin + egui::vec2(x, -5.0), origin + egui::vec2(x, 0.0)], Stroke::new(2.0, ink));
        }
        let label = scale_label(units);
        painter.text(
            origin + egui::vec2(px / 2.0, -8.0),
            Align2::CENTER_BOTTOM,
            label,
            FontId::proportional(11.5),
            ink,
        );
    }
    // Hover card.
    if let (Some(id), Some(cursor)) = (state.hovered, input.cursor)
        && let Some(e) = view.entities.iter().find(|e| e.id == id)
        && let Some(entry) = view.legend.get(usize::from(e.type_index))
    {
        let state_name = entry.states.get(usize::from(e.state)).cloned().unwrap_or_default();
        let text = format!(
            "{} {} #{}\nenergy {:.0} % · {state_name}\n({:.1}, {:.1})",
            shown_icon(ctx, &entry.icon, &entry.name),
            entry.name,
            e.id,
            e.energy * 100.0,
            e.position.x,
            e.position.y
        );
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), palette.text);
        let pos = egui::pos2(cursor.x + 16.0, cursor.y + 12.0);
        let rect = egui::Rect::from_min_size(pos, galley.size() + egui::vec2(14.0, 10.0));
        painter.rect_filled(rect, 6.0, palette.panel);
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
        painter.galley(pos + egui::vec2(7.0, 5.0), galley, palette.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases() {
        assert_eq!(Phase::of(true, None), Phase::Running);
        assert_eq!(Phase::of(false, None), Phase::Paused);
        assert_eq!(Phase::of(false, Some("all extinct")), Phase::Finished("all extinct".into()));
        assert_eq!(Phase::of(true, Some("x")), Phase::Running, "a resumed run after disabling pause-on-end");
    }

    #[test]
    fn legend_numbers_and_stops() {
        assert_eq!(short_number(0.0), "0");
        assert_eq!(short_number(0.25), "0.250");
        assert_eq!(short_number(12.345), "12.35");
        assert_eq!(short_number(2304.0), "2304");
        assert_eq!(short_number(1.0e6), "1.00e6");
        assert_eq!(scale_label(1.0), "1 cell");
        assert_eq!(scale_label(5.0), "5 cells");
        assert_eq!(scale_label(200.0), "200 cells");
        assert_eq!(scale_label(0.5), "0.5 cells");
        assert_eq!(scale_label(0.02), "0.02 cells");
        let full = legend_stops(rendering::Colormap::Grayscale, 1.0);
        let half = legend_stops(rendering::Colormap::Grayscale, 0.5);
        assert_eq!(full.last(), Some(&Color32::WHITE));
        assert_eq!(half.last(), Some(&Color32::from_gray(128)));
    }
}
