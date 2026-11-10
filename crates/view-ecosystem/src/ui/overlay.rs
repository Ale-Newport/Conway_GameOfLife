//! Viewport overlay: calendar, weather and census card, the legend of the
//! active overlay (with the value under the pointer) and a hover read-out
//! next to the pointer.

use bevy::prelude::*;
use bevy_egui::egui;
use ecosystem::OrganismRef;
use interface::widgets::gradient_bar;
use interface::{MetricsHistory, ModeRegistry, Palette, ThemeSettings};

use super::{color32, num};
use crate::MODE;
use crate::interact::Hover;
use crate::mapping::{action_label, tree_stage_label};
use crate::palette::{Overlay, category_color, overlay_scale};
use crate::scene::terrain::MapState;
use crate::state::{CameraMode, EcoData, EcoUi, Tool};

/// A unit-less index with three significant digits (`0`, `7.79`, `0.0123`,
/// `3.2e-5`).
pub fn index_text(v: f32) -> String {
    let a = v.abs();
    if a == 0.0 {
        "0".into()
    } else if a >= 1000.0 {
        num(f64::from(v))
    } else if a >= 0.01 {
        let decimals = (2 - a.log10().floor() as i32).clamp(0, 4) as usize;
        format!("{v:.decimals$}")
    } else {
        format!("{v:.1e}")
    }
}

fn value_text(overlay: Overlay, v: f32, unit: &str) -> String {
    match overlay {
        Overlay::Moisture | Overlay::FireRisk | Overlay::Disease => format!("{:.0} %", v * 100.0),
        Overlay::Pheromone | Overlay::Pollination => index_text(v),
        _ => format!("{v:.2} {unit}"),
    }
}

/// The overlay panel.
#[allow(clippy::too_many_arguments)]
pub fn overlay_panel(
    InMut(ui): InMut<egui::Ui>,
    data: Res<EcoData>,
    eco: Res<EcoUi>,
    map: Res<MapState>,
    hover: Res<Hover>,
    history: Res<MetricsHistory>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
) {
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    let Some(frame) = data.get() else { return };
    let v = &frame.view;
    egui::Frame::popup(ui.style()).show(ui, |ui| {
        ui.set_max_width(300.0);
        ui.label(egui::RichText::new(&frame.info.scenario.name).strong());
        ui.label(format!(
            "Year {} · {} day {} · {:02}:{:02}",
            v.time.year + 1,
            v.season.label(),
            v.time.day_of_season + 1,
            v.time.hour,
            v.time.minute
        ));
        let temp = history.latest("mean_temperature").map_or(String::new(), |t| format!(" · {t:.1} °C"));
        let wind = v.world.weather.wind.length();
        ui.label(egui::RichText::new(format!("{}{temp} · wind {wind:.0} m/s", v.world.weather.kind.label())).weak());
        let species = v.species.iter().filter(|s| s.population > 0).count();
        ui.label(format!(
            "{} animals · {species} species · {} trees",
            num(v.animals.len() as f64),
            num(history.latest("trees").unwrap_or(0.0))
        ));
        if let Some(sel) = eco.selection {
            let text = match sel {
                OrganismRef::Animal(id) => v
                    .animals
                    .iter()
                    .find(|a| a.id == id)
                    .and_then(|a| v.species.get(usize::from(a.species)))
                    .map_or(format!("Animal #{id}"), |s| format!("{} #{id}", s.name)),
                OrganismRef::Tree(id) => format!("Tree #{id}"),
            };
            let verb = if eco.camera == CameraMode::Follow { "Following" } else { "Selected" };
            ui.colored_label(palette.accent, format!("{verb}: {text}"));
        }
        if eco.tool != Tool::Inspect {
            ui.colored_label(palette.warning, format!("Tool: {} — click the map (Esc cancels)", eco.tool.label()));
        }
        if eco.overlay.is_grid() {
            ui.separator();
            let scale = overlay_scale(eco.overlay, map.overlay_range);
            ui.label(egui::RichText::new(eco.overlay.label()).strong());
            let stops: Vec<egui::Color32> = scale.ramp.stops(9).into_iter().map(color32).collect();
            let low = value_text(eco.overlay, scale.min, scale.unit);
            let high = value_text(eco.overlay, scale.max, scale.unit);
            gradient_bar(ui, &stops, 220.0, &low, &high);
            if scale.fade {
                ui.label(egui::RichText::new("Clear where zero, fainter where low.").weak().small());
            }
            if let Some(g) = hover.ground {
                match map.overlay_at(g.x, g.z) {
                    Some(value) => ui.label(format!("here: {}", value_text(eco.overlay, value, scale.unit))),
                    None => ui.label(egui::RichText::new("here: no data").weak()),
                };
            }
        } else if eco.overlay == Overlay::Territories {
            ui.separator();
            ui.label(egui::RichText::new("Territories").strong());
            let mut shown: Vec<u16> = frame.territories.iter().map(|t| t.species).collect();
            shown.sort_unstable();
            shown.dedup();
            if shown.is_empty() {
                ui.label(egui::RichText::new("No territorial adult holds a territory.").weak());
            }
            let half_diagonal = map.geometry.as_ref().map_or(f32::INFINITY, |g| g.extent().length() * 0.5);
            for s in shown {
                let name = v.species.get(usize::from(s)).map_or("?", |i| i.name.as_str());
                let mine: Vec<_> = frame.territories.iter().filter(|t| t.species == s).collect();
                let mut homes: Vec<(i64, i64)> = mine
                    .iter()
                    .map(|t| ((t.centre.x * 4.0).round() as i64, (t.centre.y * 4.0).round() as i64))
                    .collect();
                homes.sort_unstable();
                homes.dedup();
                let radius_m = mine.first().map_or(0.0, |t| t.radius * frame.info.cell_size_m);
                let beyond = if radius_m > half_diagonal { " · reaches beyond the map" } else { "" };
                ui.colored_label(
                    color32(category_color(usize::from(s))),
                    format!(
                        "{name}: {} holders, {} home{} · radius {}{beyond}",
                        mine.len(),
                        homes.len(),
                        if homes.len() == 1 { "" } else { "s" },
                        if radius_m >= 1000.0 {
                            format!("{:.1} km", radius_m / 1000.0)
                        } else {
                            format!("{radius_m:.0} m")
                        }
                    ),
                );
            }
            ui.label(egui::RichText::new("Poles mark the homes; circles the boundaries.").weak().small());
        }
    });
    // Read-out next to the pointer.
    let (Some(organism), Some(cursor)) = (hover.organism, hover.cursor) else { return };
    let text = match organism {
        OrganismRef::Animal(id) => v.animals.iter().find(|a| a.id == id).map(|a| {
            let name = v.species.get(usize::from(a.species)).map_or("Animal", |s| s.name.as_str());
            let age = if a.juvenile { "young " } else { "" };
            format!(
                "{age}{name} · {} · energy {:.0} % · health {:.0} %{}",
                action_label(a.action).to_lowercase(),
                a.energy_fraction * 100.0,
                a.health * 100.0,
                if a.infected { " · infectious" } else { "" }
            )
        }),
        OrganismRef::Tree(id) => data.trees_frame.as_ref().and_then(|f| {
            f.view.trees.iter().find(|t| t.id == id).map(|t| {
                let name = f.info.catalog.trees().get(usize::from(t.species)).map_or("Tree", |s| s.name.as_str());
                format!(
                    "{name} · {} · {:.1} m{}",
                    tree_stage_label(t.stage),
                    t.height_m,
                    if t.burnt { " · fire-scarred" } else { "" }
                )
            })
        }),
    };
    let Some(text) = text else { return };
    egui::Area::new(egui::Id::new("eco-hover-readout"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(egui::pos2(cursor.x + 16.0, cursor.y + 14.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(text);
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_keep_three_significant_digits() {
        assert_eq!(index_text(0.0), "0");
        assert_eq!(index_text(7.786), "7.79");
        assert_eq!(index_text(0.01234), "0.0123");
        assert_eq!(index_text(123.4), "123");
        assert_eq!(index_text(3.2e-5), "3.2e-5");
        assert_eq!(value_text(Overlay::FireRisk, 0.25, ""), "25 %");
    }
}
