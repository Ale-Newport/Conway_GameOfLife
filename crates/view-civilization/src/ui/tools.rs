//! Left panel ("Tools"): scenario card, intervention tools, regeneration
//! sliders, layers, camera and the mode's keyboard shortcuts.

use bevy::prelude::*;
use bevy_egui::egui::{self, RichText};
use civilization::{CivCommand, Resource as Stock};
use interface::{ModeRegistry, Palette, ThemeSettings};
use simulation_core::Control;

use crate::MODE;
use crate::interact::ADDABLE;
use crate::session::Selection;
use crate::state::{CAMERA_PRESETS, CivRunner, CurrentFrame, MODE_SHORTCUTS, ModeShortcut, Tool, ViewState};
use crate::ui::widgets::{card, count, section, segmented};

fn palette(theme: &ThemeSettings, registry: &ModeRegistry) -> Palette {
    Palette::new(theme.kind, registry.accent(MODE))
}

fn key_of(action: ModeShortcut) -> &'static str {
    MODE_SHORTCUTS.iter().find(|(_, _, a, _)| *a == action).map_or("", |(_, label, ..)| *label)
}

/// Draft values of the regeneration sliders while they are being dragged
/// (the published value catches up a few frames later).
#[derive(Default)]
pub struct RegenDraft([Option<f32>; 5]);

/// The left panel.
#[allow(clippy::too_many_arguments)]
pub fn tools_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    current: Res<CurrentFrame>,
    runner: Option<Res<CivRunner>>,
    mut state: ResMut<ViewState>,
    mut draft: Local<RegenDraft>,
) {
    let palette = palette(&theme, &registry);
    let Some(frame) = current.get() else {
        ui.label(RichText::new("Building the world…").weak());
        return;
    };
    card(&palette).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(&frame.info.name).strong().size(14.5));
        let v = &frame.view;
        ui.label(
            RichText::new(format!(
                "{} × {} tiles · {} · {}",
                v.world_width,
                v.world_height,
                count(v.agents.len(), "person", "people"),
                count(v.buildings.iter().filter(|b| b.progress >= 1.0).count(), "building", "buildings")
            ))
            .size(12.0)
            .color(palette.text_muted),
        );
    });

    section(ui, &palette, "Interventions");
    let mut tool = state.tool;
    let labels: Vec<(Tool, String)> =
        Tool::ALL.iter().map(|t| (*t, format!("{}  {}", t.label(), key_of(ModeShortcut::Tool(*t))))).collect();
    let options: Vec<(Tool, &str)> = labels.iter().map(|(t, l)| (*t, l.as_str())).collect();
    if segmented(ui, &palette, &mut tool, &options) {
        state.tool = tool;
    }
    ui.add_space(2.0);
    ui.label(RichText::new(state.tool.hint()).size(12.0).color(palette.text_muted));
    ui.add_space(4.0);
    match state.tool {
        Tool::Inspect => {}
        Tool::Spawn => {
            let populations = &frame.info.populations;
            if populations.is_empty() {
                ui.label(RichText::new("This scenario has no population to draw newcomers from.").weak());
            } else {
                let mut index = usize::from(state.spawn_population).min(populations.len() - 1);
                egui::ComboBox::from_label("Population")
                    .selected_text(populations[index].clone())
                    .show_ui(ui, |ui| {
                        for (i, label) in populations.iter().enumerate() {
                            ui.selectable_value(&mut index, i, label);
                        }
                    })
                    .response
                    .on_hover_text("Newcomers get the preference distributions of this founding population.");
                state.spawn_population = u8::try_from(index).unwrap_or(0);
                let mut count = state.spawn_count;
                ui.add(
                    egui::Slider::new(&mut count, 1..=civilization::command::MAX_SPAWN)
                        .logarithmic(true)
                        .text("per click"),
                );
                state.spawn_count = count;
            }
        }
        Tool::AddResource => {
            let mut resource = state.add_resource;
            let options: Vec<(Stock, &str)> = ADDABLE.iter().map(|r| (*r, r.label())).collect();
            segmented(ui, &palette, &mut resource, &options);
            state.add_resource = resource;
            radius_slider(ui, &mut state);
        }
        Tool::RemoveResource => {
            let mut resource = state.remove_resource;
            let mut options: Vec<(Option<Stock>, &str)> = vec![(None, "All")];
            options.extend(ADDABLE.iter().map(|r| (Some(*r), r.label())));
            segmented(ui, &palette, &mut resource, &options);
            state.remove_resource = resource;
            radius_slider(ui, &mut state);
        }
        Tool::Disaster => {
            radius_slider(ui, &mut state);
            ui.label(
                RichText::new("Buildings, their stores and crops are lost; territory is recomputed at once.")
                    .size(12.0)
                    .color(palette.warning),
            );
        }
    }
    if state.tool != Tool::Inspect {
        ui.label(RichText::new("Interventions are undoable (Ctrl/Cmd+Z).").size(11.5).color(palette.text_muted));
    }

    section(ui, &palette, "Regeneration");
    ui.label(
        RichText::new("Multiplier of each resource's growback per tick (crops: farm growth).")
            .size(12.0)
            .color(palette.text_muted),
    );
    for resource in Stock::ALL {
        let i = resource.index();
        let mut value = draft.0[i].unwrap_or(frame.regen[i]);
        let response = ui.add(egui::Slider::new(&mut value, 0.0..=5.0).step_by(0.05).text(resource.label()));
        // egui snaps a value that is not on the 0.05 grid (e.g. a scenario's
        // 0.6 stored as f32) and reports that as a change on every frame:
        // only real edits are sent.
        if regen_edited(response.changed(), value, draft.0[i].unwrap_or(frame.regen[i])) {
            draft.0[i] = Some(value);
            if let Some(runner) = &runner {
                runner.0.send(Control::Command(CivCommand::SetRegeneration { resource, multiplier: value }));
            }
        }
        if !response.dragged() && !response.has_focus() && draft.0[i].is_some_and(|d| (d - frame.regen[i]).abs() < 1e-4)
        {
            draft.0[i] = None;
        }
    }

    section(ui, &palette, "Layers");
    let mut layers = state.layers;
    ui.checkbox(&mut layers.territory, format!("Territory  ({})", key_of(ModeShortcut::Territory))).on_hover_text(
        "Group colours on claimed tiles; brighter where another group's influence is close (contested).",
    );
    ui.checkbox(&mut layers.roads, format!("Trails and roads  ({})", key_of(ModeShortcut::Roads)))
        .on_hover_text("Faint trails grow from footfall; tiles above the road threshold are drawn as roads.");
    ui.checkbox(&mut layers.icons, format!("Activity badges  ({})", key_of(ModeShortcut::Icons)));
    ui.checkbox(&mut layers.vegetation, format!("Trees, bushes and rocks  ({})", key_of(ModeShortcut::Vegetation)));
    if layers != state.layers {
        state.layers = layers;
    }

    section(ui, &palette, "Camera");
    let mut preset = state.camera;
    let options: Vec<(rendering::OrbitPreset, &str)> = CAMERA_PRESETS.iter().map(|p| (*p, p.label())).collect();
    if segmented(ui, &palette, &mut preset, &options) {
        state.camera = preset;
    }
    ui.horizontal(|ui| {
        if ui.button(format!("Frame map  {}", key_of(ModeShortcut::FrameWorld))).clicked() {
            state.frame_world = true;
        }
        let person = matches!(state.selection, Selection::Agent(_));
        let mut follow = state.follow;
        ui.add_enabled_ui(person, |ui| {
            ui.toggle_value(&mut follow, format!("Follow  {}", key_of(ModeShortcut::Follow)))
                .on_disabled_hover_text("Select a person first.");
        });
        if follow != state.follow {
            state.follow = follow && person;
        }
    });
    ui.label(
        RichText::new(match state.camera {
            rendering::OrbitPreset::Strategic => {
                "Middle-drag orbits, right-drag pans, wheel zooms; WASD/arrows pan, Q/E rotate."
            }
            rendering::OrbitPreset::Isometric => "Right-drag pans, wheel zooms; Q/E rotate by 90°.",
            _ => "Left-drag orbits, right-drag pans, wheel zooms; WASD/arrows pan.",
        })
        .size(11.5)
        .color(palette.text_muted),
    );

    section(ui, &palette, "Keys");
    egui::Grid::new("civ_keys").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
        for (_, label, _, description) in MODE_SHORTCUTS {
            ui.label(RichText::new(label).monospace().size(11.5).color(palette.accent));
            ui.label(RichText::new(description).size(11.5).color(palette.text_muted));
            ui.end_row();
        }
    });
}

/// Whether a regeneration slider was really edited (`shown`: the draft
/// being dragged, else the published value). egui also reports `changed`
/// when it snaps a value that is not on the slider's 0.05 grid in `f64`
/// (any f32 such as 0.6), which leaves the f32 value unchanged.
pub fn regen_edited(changed: bool, value: f32, shown: f32) -> bool {
    changed && (value - shown).abs() > 1e-4
}

fn radius_slider(ui: &mut egui::Ui, state: &mut ViewState) {
    let mut radius = state.radius;
    ui.scope(|ui| {
        // Fit the row (value field with a unit and a label) in the panel,
        // which would otherwise grow while an area tool is selected.
        ui.spacing_mut().slider_width = (ui.available_width() - 150.0).clamp(80.0, 200.0);
        ui.add(egui::Slider::new(&mut radius, 1.0..=16.0).step_by(0.5).text("radius").suffix(" tiles"));
    });
    state.radius = radius;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regeneration_sliders_ignore_step_snapping() {
        // A scenario value of 0.6 (f32) is snapped to the 0.05 grid by egui.
        let published = 0.6f32;
        let snapped = (f64::from(published) / 0.05).round() * 0.05;
        assert_ne!(snapped, f64::from(published), "egui sees a change in f64");
        assert!(!regen_edited(true, snapped as f32, published), "a snap is not an edit");
        assert!(regen_edited(true, 0.65, published), "a real step is");
        assert!(!regen_edited(false, 0.65, published));
        // Dragging back to the published value from an unsent draft is sent.
        assert!(regen_edited(true, 0.6, 1.5));
    }
}
