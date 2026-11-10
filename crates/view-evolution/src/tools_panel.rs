//! Left "Tools" panel: experiment and replicates, world tools,
//! interventions (climate, barriers, population), custom genome, display.

use bevy::prelude::*;
use bevy_egui::egui::{self, RichText};
use evolution::genes::{GENES, PERCEPTION, SIZE, SPEED};
use evolution::{EvolutionCommand, Species};
use interface::{ModeRegistry, ModeScenarios, Palette, SessionStatus, ThemeSettings, Toasts};
use rendering::OrbitPreset;
use simulation_core::SessionControl;

use crate::MODE;
use crate::interaction::{SHORTCUTS, mouse_help};
use crate::scene::SceneState;
use crate::session::{EvoData, EvoRunner, EvoScenarios, load_scenario};
use crate::ui_state::{CullTarget, EvoUi, GroundOverlay, Tool};
use crate::widgets::{card, note, section, segmented};

/// The mode's palette.
pub fn palette(theme: &ThemeSettings, registry: &ModeRegistry) -> Palette {
    Palette::new(theme.kind, registry.accent(MODE))
}

/// Whole-world rectangle for global interventions.
fn whole_world(data: &EvoData) -> Option<evolution::Shape> {
    data.view.as_ref().map(|v| evolution::Shape::Rect { x: 0.0, y: 0.0, w: v.width as f32, h: v.height as f32 })
}

/// Left panel system.
#[allow(clippy::too_many_arguments)]
pub fn tools_panel(
    InMut(ui): InMut<egui::Ui>,
    mut state: ResMut<EvoUi>,
    mut data: ResMut<EvoData>,
    runner: Option<Res<EvoRunner>>,
    scenarios: Res<EvoScenarios>,
    status: Res<SessionStatus>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    mut toasts: ResMut<Toasts>,
    mut scene: ResMut<SceneState>,
    mut picker: ResMut<ModeScenarios>,
) {
    let palette = palette(&theme, &registry);
    let Some(runner) = runner else {
        note(ui, &palette, "The experiment is not running.");
        return;
    };
    let ui_state = &mut *state;
    // Slider + value + label must fit the panel's default width; wider rows
    // would make the panel jump in width when another tool is picked.
    ui.spacing_mut().slider_width = 140.0;

    // ---------------------------------------------------------- experiment
    section(ui, &palette, "Experiment");
    card(&palette).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(&data.scenario.name).strong().size(14.0));
        if !data.scenario.description.is_empty() {
            note(ui, &palette, &data.scenario.description);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Seed");
            ui.add(egui::DragValue::new(&mut ui_state.seed_input).speed(1.0));
            ui.label(RichText::new(format!("(running: {})", data.scenario.seed)).color(palette.text_muted).size(11.0));
        });
        ui.horizontal_wrapped(|ui| {
            let restart = ui.button("Restart with seed").on_hover_text(
                "Rebuild the current experiment (including parameter changes) from tick 0 with the seed above. Undoable.",
            );
            let next = ui
                .button("Next replicate")
                .on_hover_text("Same experiment with seed + 1: an independent replicate run. Undoable.");
            let preset = ui
                .button("Reload preset")
                .on_hover_text("Reload the scenario file as shipped (its own seed and settings, parameter changes undone).");
            let seed = if restart.clicked() {
                Some(ui_state.seed_input)
            } else if next.clicked() {
                Some(data.scenario.seed.wrapping_add(1))
            } else {
                None
            };
            if let Some(seed) = seed {
                // Same ids may now denote other organisms: forget the run.
                runner.0.randomize(seed);
                data.reset_for_new_run();
                data.scenario.seed = seed;
                ui_state.on_new_run(seed);
            }
            if preset.clicked()
                && let Some(base) = scenarios.get(&data.scenario.id).cloned()
                && load_scenario(&runner, &base, None, &mut data, ui_state, &mut toasts, &mut picker)
            {
                scene.frame_request = true;
            }
        });
        note(
            ui,
            &palette,
            "Equal seed + equal interventions = identical run. Compare treatments on the same seeds, then repeat on new seeds.",
        );
    });

    // ---------------------------------------------------------- tools
    section(ui, &palette, "World tool");
    egui::Grid::new("evo_tools").num_columns(2).spacing([4.0, 4.0]).show(ui, |ui| {
        for (i, tool) in Tool::ALL.iter().enumerate() {
            let on = ui_state.tool == *tool;
            let text = RichText::new(format!("{}  {}", i + 1, tool.label())).size(12.0).color(if on {
                palette.on_accent
            } else {
                palette.text
            });
            let button = egui::Button::new(text)
                .fill(if on { palette.accent } else { palette.surface })
                .min_size(egui::vec2(126.0, 24.0));
            if ui.add(button).on_hover_text(tool.hint()).clicked() {
                ui_state.tool = *tool;
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    note(ui, &palette, ui_state.tool.hint());
    if ui_state.tool.uses_brush() {
        ui.add(egui::Slider::new(&mut ui_state.brush_radius, 1.0..=40.0).text("radius (cells)").logarithmic(true));
    }
    match ui_state.tool {
        Tool::Select => {}
        Tool::Food => {
            ui.add(egui::Slider::new(&mut ui_state.food_amount, 0.5..=50.0).text("food per cell").logarithmic(true));
            note(ui, &palette, "Food above a cell's capacity decays back towards it (logistic).");
        }
        Tool::Cull => {
            cull_controls(ui, ui_state, &palette);
            if ui.button("Cull everywhere").clicked() {
                let species = ui_state.cull_target.species();
                runner.0.edit(EvolutionCommand::Cull { fraction: ui_state.cull_fraction, species, area: None });
            }
        }
        Tool::Spawn => {
            spawn_controls(ui, ui_state, &palette);
            if ui.button("Release at random places").clicked() {
                runner.0.edit(EvolutionCommand::Spawn {
                    species: ui_state.genome.species,
                    count: ui_state.genome.count,
                    area: None,
                    traits: ui_state.genome_trait_values(),
                    spread: ui_state.genome.spread,
                });
            }
        }
        Tool::Predators => {
            predator_controls(ui, ui_state);
            if ui.button("Introduce anywhere").clicked() {
                runner.0.edit(EvolutionCommand::IntroducePredators {
                    count: ui_state.predator_count,
                    area: None,
                    traits: ui_state.predator_trait_values(),
                });
            }
        }
    }

    // ---------------------------------------------------------- interventions
    section(ui, &palette, "Climate");
    let env = data.env.clone();
    if let Some(env) = &env {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{:.1} °C", env.mean_temperature)).strong().size(15.0));
            note(ui, &palette, "mean over land");
        });
        if let Some((lo, hi)) = env.land_temperature_range() {
            note(ui, &palette, &format!("range {lo:.1} … {hi:.1} °C · trend {:+.1} °C / 1000 ticks", env.shift_rate));
        }
    }
    if let Some(env) = &env {
        ui_state.sync_trend(env.shift_rate);
    }
    ui.horizontal(|ui| {
        let step = ui_state.temp_step;
        if ui.button(format!("−{step:.1} °C")).on_hover_text("Cool the whole field at once").clicked() {
            runner.0.edit(EvolutionCommand::ChangeTemperature { delta: -step });
        }
        if ui.button(format!("+{step:.1} °C")).on_hover_text("Warm the whole field at once").clicked() {
            runner.0.edit(EvolutionCommand::ChangeTemperature { delta: step });
        }
        note(ui, &palette, "step");
        ui.add(egui::DragValue::new(&mut ui_state.temp_step).range(0.5..=20.0).speed(0.1).suffix(" °C"))
            .on_hover_text("Size of one cooling / warming step");
    });
    ui.add(egui::Slider::new(&mut ui_state.trend, -20.0..=20.0).text("°C / 1000 ticks"))
        .on_hover_text("Climate trend: directional shift of the whole field; applied with the button below");
    let running = env.as_ref().map(|e| e.shift_rate);
    let pending = running.is_none_or(|r| (r - ui_state.trend).abs() > 0.05);
    if pending {
        if ui
            .button("Apply climate trend")
            .on_hover_text("Sets the directional shift of the whole field (undoable)")
            .clicked()
        {
            runner.0.edit(EvolutionCommand::SetTemperatureShift { per_1000_ticks: ui_state.trend });
        }
    } else {
        note(ui, &palette, "This trend is in effect; move the slider to change it.");
    }

    section(ui, &palette, "Barriers");
    match data.view.as_ref().map(|v| v.barriers.clone()) {
        Some(barriers) if !barriers.is_empty() => {
            for (index, barrier) in barriers.iter().enumerate() {
                let mut active = barrier.active;
                let label = format!("{index}: {}", barrier.name);
                if ui.checkbox(&mut active, label).on_hover_text("Raise or remove (regions are recomputed)").changed() {
                    runner.0.edit(EvolutionCommand::SetBarrier { index, active });
                }
            }
        }
        _ => note(ui, &palette, "This scenario has no switchable barriers (try Island evolution)."),
    }

    section(ui, &palette, "Population");
    ui.horizontal_wrapped(|ui| {
        if ui.button("Cull 50 % of prey").clicked() {
            runner.0.edit(EvolutionCommand::Cull { fraction: 0.5, species: Some(Species::Prey), area: None });
        }
        if ui.button("Remove predators").clicked() {
            runner.0.edit(EvolutionCommand::Cull { fraction: 1.0, species: Some(Species::Predator), area: None });
        }
        if ui.button("Feed everywhere").on_hover_text("Adds 4 food to every land cell").clicked()
            && let Some(area) = whole_world(&data)
        {
            runner.0.edit(EvolutionCommand::AddFood { area, amount: 4.0 });
        }
    });
    let (prey, predators) = data.counts();
    note(ui, &palette, &format!("{prey} prey · {predators} predators alive"));

    // ---------------------------------------------------------- display
    section(ui, &palette, "Display");
    ui.horizontal(|ui| {
        ui.label("Ground");
        egui::ComboBox::from_id_salt("evo_overlay").selected_text(ui_state.overlay.label()).show_ui(ui, |ui| {
            for overlay in GroundOverlay::ALL {
                ui.selectable_value(&mut ui_state.overlay, overlay, overlay.label());
            }
        });
        ui.checkbox(&mut ui_state.vegetation, "Vegetation");
    });
    ui.horizontal(|ui| {
        ui.label("Camera");
        egui::ComboBox::from_id_salt("evo_camera").selected_text(ui_state.camera_preset.label()).show_ui(ui, |ui| {
            for preset in OrbitPreset::ALL {
                ui.selectable_value(&mut ui_state.camera_preset, preset, preset.label());
            }
        });
        if ui.button("Frame world").clicked() {
            scene.frame_request = true;
            ui_state.follow = false;
        }
    });
    ui.checkbox(&mut ui_state.legend_open, "Legend in the viewport");

    section(ui, &palette, "Shortcuts");
    egui::Grid::new("evo_shortcuts").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
        for (key, action) in SHORTCUTS {
            ui.label(RichText::new(key).monospace().size(11.0).color(palette.accent));
            ui.label(RichText::new(action).size(11.0));
            ui.end_row();
        }
    });
    // Wrapped below the grid: a long grid cell would widen the whole panel.
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Mouse").monospace().size(11.0).color(palette.accent));
        ui.label(RichText::new(mouse_help(ui_state.camera_preset)).size(11.0));
    });
    if status.get().is_some_and(|s| s.finished) {
        ui.add_space(6.0);
        ui.colored_label(palette.warning, "The run has finished (no organism can change the world any more).");
    }
}

fn cull_controls(ui: &mut egui::Ui, state: &mut EvoUi, palette: &Palette) {
    ui.add(egui::Slider::new(&mut state.cull_fraction, 0.0..=1.0).text("fraction removed"));
    segmented(
        ui,
        palette,
        &mut state.cull_target,
        &[(CullTarget::Prey, "Prey"), (CullTarget::Predators, "Predators"), (CullTarget::Both, "Both")],
    );
    note(ui, palette, "Victims are chosen uniformly at random: a cull is not selective (a control for predation).");
}

fn predator_controls(ui: &mut egui::Ui, state: &mut EvoUi) {
    ui.add(egui::Slider::new(&mut state.predator_count, 1..=100).text("predators"));
    gene_sliders(
        ui,
        "evo_predator_traits",
        [SPEED, PERCEPTION, SIZE].into_iter().zip(state.predator_traits.iter_mut()),
    );
}

/// Gene sliders laid out as `label | slider`, so long gene names never widen
/// the panel; the hover text gives the unit and what the gene does.
fn gene_sliders<'a>(ui: &mut egui::Ui, id: &str, genes: impl Iterator<Item = (usize, &'a mut f32)>) {
    egui::Grid::new(id).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.spacing_mut().slider_width = 90.0;
        for (gene, value) in genes {
            let g = &GENES[gene];
            let hover = if g.unit.is_empty() {
                g.description.to_string()
            } else {
                format!("{} ({}) — {}", g.label, g.unit, g.description)
            };
            ui.label(RichText::new(g.label).size(12.0)).on_hover_text(&hover);
            let unit = if g.unit.is_empty() || g.unit == "fraction" { String::new() } else { format!(" {}", g.unit) };
            ui.add(egui::Slider::new(value, g.min..=g.max).suffix(unit)).on_hover_text(hover);
            ui.end_row();
        }
    });
}

fn spawn_controls(ui: &mut egui::Ui, state: &mut EvoUi, palette: &Palette) {
    segmented(ui, palette, &mut state.genome.species, &[(Species::Prey, "Prey"), (Species::Predator, "Predators")]);
    ui.add(egui::Slider::new(&mut state.genome.count, 1..=200).text("organisms"));
    ui.add(egui::Slider::new(&mut state.genome.spread, 0.0..=0.2).text("spread"))
        .on_hover_text("Standard deviation around the genome, in normalised gene units (0 = identical clones)");
    egui::CollapsingHeader::new("Custom genome").id_salt("evo_genome").default_open(true).show(ui, |ui| {
        gene_sliders(ui, "evo_genome_grid", state.genome.values.iter_mut().enumerate());
        if ui.button("Reset to defaults").clicked() {
            state.genome.values = std::array::from_fn(|i| GENES[i].default);
        }
        note(ui, palette, "Spawned organisms start new lineages (founders). Use “Copy genome” in the inspector to clone a selected organism.");
    });
}
