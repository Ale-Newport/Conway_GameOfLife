//! Left panel: camera, overlays, layers and interventions. Every
//! intervention is a model command (undoable with Ctrl/Cmd+Z) or a model
//! parameter; each is explained where it is offered.

use bevy::prelude::*;
use bevy_egui::egui;
use ecosystem::species::Representation;
use ecosystem::{EcosystemCommand, OrganismRef};
use interface::{ModeRegistry, Palette, SessionStatus, ThemeSettings, Toasts};
use physics_world::Disaster;
use simulation_core::{Control, ParamValue};

use super::{heading, note, slider};
use crate::MODE;
use crate::palette::Overlay;
use crate::session::Frame;
use crate::state::{CameraMode, EcoData, EcoRunner, EcoUi, Tool};

/// The Tools panel.
#[allow(clippy::too_many_arguments)]
pub fn tools_panel(
    InMut(ui): InMut<egui::Ui>,
    mut eco: ResMut<EcoUi>,
    data: Res<EcoData>,
    runner: Option<Res<EcoRunner>>,
    status: Res<SessionStatus>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    mut toasts: ResMut<Toasts>,
) {
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    let Some(frame) = data.get() else {
        ui.label("Waiting for the simulation…");
        return;
    };
    camera_section(ui, &mut eco);
    ui.separator();
    overlay_section(ui, &mut eco, frame);
    ui.separator();
    layers_section(ui, &mut eco);
    ui.separator();
    if let Some(runner) = runner {
        interventions(ui, &mut eco, frame, &runner, &status, &palette, &mut toasts);
    }
}

fn camera_section(ui: &mut egui::Ui, eco: &mut EcoUi) {
    heading(ui, "Camera");
    ui.horizontal_wrapped(|ui| {
        for mode in CameraMode::ALL {
            let enabled = mode != CameraMode::Follow || matches!(eco.selection, Some(OrganismRef::Animal(_)));
            let response = ui
                .add_enabled(enabled, egui::Button::selectable(eco.camera == mode, mode.label()))
                .on_hover_text(mode.hint())
                .on_disabled_hover_text("Select an animal first.");
            if response.clicked() {
                eco.camera = mode;
            }
        }
    });
    note(ui, eco.camera.hint());
    ui.horizontal(|ui| {
        if ui.button("Frame the whole map").on_hover_text("Shortcut: M").clicked() {
            eco.frame_map = true;
        }
    });
    note(ui, "Shortcuts: O orbit, T top-down, G ground, C cinematic, F follow, Esc deselect.");
}

fn overlay_section(ui: &mut egui::Ui, eco: &mut EcoUi, frame: &Frame) {
    heading(ui, "Overlay");
    let colonies = frame.view.colonies.iter().any(|c| {
        frame
            .info
            .catalog
            .animals()
            .get(usize::from(c.species))
            .is_some_and(|s| !s.can(ecosystem::species::Locomotion::Fly))
    });
    egui::Grid::new("eco-overlays").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
        for (i, overlay) in Overlay::ALL.into_iter().enumerate() {
            let available = match overlay {
                Overlay::Pheromone => colonies,
                // The NetLogo rules keep no nutrients, flowers or diseases, and
                // fire does not reach their animals or grass.
                Overlay::Pollination | Overlay::Disease | Overlay::Nutrients | Overlay::FireRisk => {
                    frame.info.netlogo.is_none()
                }
                _ => true,
            };
            let response = ui
                .add_enabled(available, egui::RadioButton::new(eco.overlay == overlay, overlay.label()))
                .on_hover_text(overlay.explanation())
                .on_disabled_hover_text("This scenario has no data for it.");
            if response.clicked() {
                eco.overlay = overlay;
            } else if !available && eco.overlay == overlay {
                // A scenario without the data was loaded under this overlay.
                eco.overlay = Overlay::None;
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    note(ui, eco.overlay.explanation());
}

fn layers_section(ui: &mut egui::Ui, eco: &mut EcoUi) {
    heading(ui, "Layers");
    let mut layers = eco.layers;
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut layers.animals, "Animals").on_hover_text("Individual animals, carcasses and nests.");
        ui.checkbox(&mut layers.trees, "Trees");
        ui.checkbox(&mut layers.ground_cover, "Grass & flowers")
            .on_hover_text("Plants near the camera, from the vegetation biomass of each cell.");
        ui.checkbox(&mut layers.small_life, "Small life").on_hover_text(
            "Insects, spiders, worms and forage fish near the camera (the model's representative markers of its \
             density fields, each standing for many individuals, drawn 4× life size), ant nests, bee hives and \
             forager squads.",
        );
        ui.checkbox(&mut layers.weather, "Weather & fire");
        ui.checkbox(&mut layers.highlights, "Highlights").on_hover_text("Selection ring and followed trail.");
    });
    if layers != eco.layers {
        eco.layers = layers;
    }
}

fn species_label(frame: &Frame, index: usize) -> String {
    let s = &frame.info.catalog.animals()[index];
    let kind = match s.representation {
        Representation::Individual => "",
        Representation::Field(_) => " (field)",
        Representation::Colony(_) => " (colonies)",
    };
    format!("{}{kind}", s.name)
}

#[allow(clippy::too_many_arguments)]
fn interventions(
    ui: &mut egui::Ui,
    eco: &mut EcoUi,
    frame: &Frame,
    runner: &EcoRunner,
    status: &SessionStatus,
    palette: &Palette,
    toasts: &mut Toasts,
) {
    heading(ui, "Interventions");
    note(ui, "Click tools act where you click on the map. Every intervention can be undone (Ctrl/Cmd+Z).");
    let netlogo = frame.info.netlogo.is_some();
    // Under the NetLogo rules the physical world does not touch the animals
    // or the grass patches: fire, water and terrain tools would change nothing.
    let tools: &[Tool] = if netlogo {
        &[Tool::Inspect, Tool::Spawn, Tool::Cull]
    } else {
        &[
            Tool::Inspect,
            Tool::Spawn,
            Tool::Cull,
            Tool::PlantTree,
            Tool::Ignite,
            Tool::Flood,
            Tool::Earthquake,
            Tool::Landslide,
            Tool::Volcano,
            Tool::Tornado,
        ]
    };
    ui.horizontal_wrapped(|ui| {
        for &tool in tools {
            if ui.add(egui::Button::selectable(eco.tool == tool, tool.label())).clicked() {
                eco.tool = tool;
                eco.settings.tornado_start = None;
            }
        }
    });
    if !tools.contains(&eco.tool) {
        eco.tool = Tool::Inspect;
    }
    let cells = frame.view.cell_size_m;
    let s = &mut eco.settings;
    let radius = |ui: &mut egui::Ui, r: &mut f32| {
        slider(
            ui,
            &format!("Brush radius: cells ({:.0} m)", *r * cells),
            egui::Slider::new(r, 0.5..=20.0).step_by(0.5),
        );
    };
    egui::Frame::group(ui.style()).fill(palette.surface).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match eco.tool {
            Tool::Inspect => {
                note(
                    ui,
                    "Click an animal or a tree to inspect it; click empty ground to select the nearest animal \
                          within 1.5 cells.",
                );
            }
            Tool::Spawn => {
                let animals = frame.info.catalog.animals();
                let current = animals.iter().position(|a| a.id == s.spawn_species);
                egui::ComboBox::from_id_salt("eco-spawn-species")
                    .selected_text(current.map_or(s.spawn_species.clone(), |i| species_label(frame, i)))
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        let present = |i: usize| frame.view.species.get(i).is_some_and(|p| p.population > 0);
                        ui.label(egui::RichText::new("In this ecosystem").weak());
                        for i in (0..animals.len()).filter(|&i| present(i)) {
                            ui.selectable_value(&mut s.spawn_species, animals[i].id.clone(), species_label(frame, i));
                        }
                        ui.separator();
                        ui.label(egui::RichText::new("New to it (introduce an invader)").weak());
                        for i in (0..animals.len()).filter(|&i| !present(i)) {
                            ui.selectable_value(&mut s.spawn_species, animals[i].id.clone(), species_label(frame, i));
                        }
                    });
                slider(
                    ui,
                    "Count",
                    egui::Slider::new(&mut s.spawn_count, 1..=ecosystem::command::MAX_SPAWN).logarithmic(true),
                );
                radius(ui, &mut s.radius);
                note(
                    ui,
                    "Adds animals with random ages and condition around the click (individuals; for a density \
                     field `count` individuals; for ants and bees `count` colonies). Their tissue is booked as an \
                     intervention input. A species new to the ecosystem is an introduction: watch its prey and \
                     competitors in Populations and the Food web.",
                );
            }
            Tool::Cull => {
                let animals = frame.info.catalog.animals();
                let label =
                    s.cull_species.map_or("Every species".to_string(), |i| species_label(frame, usize::from(i)));
                egui::ComboBox::from_id_salt("eco-cull-species").selected_text(label).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.cull_species, None, "Every species");
                    for (i, sp) in animals.iter().enumerate() {
                        let alive = frame.view.species.get(i).is_some_and(|p| p.population > 0);
                        if sp.is_individual() && alive {
                            ui.selectable_value(&mut s.cull_species, Some(i as u16), sp.name.clone());
                        }
                    }
                });
                radius(ui, &mut s.radius);
                note(
                    ui,
                    if netlogo {
                        "Removes the animals within the brush (one model command each, undone together); \
                         deaths count as `killed`."
                    } else {
                        "Kills the individual animals within the brush (one model command each, undone \
                         together). They become carcasses that scavengers and decomposers use; deaths count as \
                         `killed`."
                    },
                );
            }
            Tool::PlantTree => {
                let trees = frame.info.catalog.trees();
                let current =
                    trees.iter().find(|t| t.id == s.tree_species).map_or(s.tree_species.clone(), |t| t.name.clone());
                egui::ComboBox::from_id_salt("eco-tree-species").selected_text(current).show_ui(ui, |ui| {
                    for t in trees {
                        ui.selectable_value(&mut s.tree_species, t.id.clone(), t.name.clone());
                    }
                });
                note(
                    ui,
                    "Plants a seedling on dry land; its nutrients come from the soil of the cell (refused \
                          when the soil holds too few).",
                );
            }
            Tool::Ignite => {
                note(
                    ui,
                    "Sets the clicked cell on fire if it holds enough dry fuel. The fire then spreads by the \
                          physics world's rule: with the wind, uphill and through dry fuel (Fire risk overlay).",
                );
            }
            Tool::Flood => {
                slider(ui, "Depth at the centre (m)", egui::Slider::new(&mut s.flood_depth_m, 0.2..=10.0));
                radius(ui, &mut s.radius);
                note(
                    ui,
                    "Adds a dome of water around the click; it flows downhill and soaks in. Non-swimmers \
                          flee deep water and drown if caught; land plants suffer flood dieback.",
                );
            }
            Tool::Earthquake => {
                slider(ui, "Magnitude", egui::Slider::new(&mut s.magnitude, 3.0..=9.5));
                radius(ui, &mut s.radius);
                note(
                    ui,
                    "Fault displacement, rockfall and slope failures around the epicentre change the \
                          terrain (the landscape is rebuilt).",
                );
            }
            Tool::Landslide => {
                radius(ui, &mut s.radius);
                note(ui, "Forces slope failures on steep cells around the click.");
            }
            Tool::Volcano => {
                slider(
                    ui,
                    "Cone height (m)",
                    egui::Slider::new(&mut s.volcano_height_m, 10.0..=1000.0).logarithmic(true),
                );
                radius(ui, &mut s.radius);
                note(ui, "Raises a cone, pours lava (it ignites what it touches) and deposits ash nutrients.");
            }
            Tool::Tornado => {
                slider(ui, "Swath width (cells)", egui::Slider::new(&mut s.tornado_width, 0.5..=8.0));
                note(
                    ui,
                    if s.tornado_start.is_some() {
                        "Click where the path ends."
                    } else {
                        "Click where the tornado starts, then where it ends. It flattens vegetation into litter \
                     and throws trees (taller ones more likely)."
                    },
                );
            }
        }
    });
    if netlogo {
        note(
            ui,
            "Under the NetLogo rules the physical world (weather, fire, water, terrain) does not affect the \
             animals or the grass, and there are no trees or diseases.",
        );
        return;
    }
    weather_actions(ui, eco, frame, runner, status, toasts);
    disease_actions(ui, eco, frame, runner, toasts);
}

fn weather_actions(
    ui: &mut egui::Ui,
    eco: &mut EcoUi,
    frame: &Frame,
    runner: &EcoRunner,
    status: &SessionStatus,
    toasts: &mut Toasts,
) {
    egui::CollapsingHeader::new("Weather, water and nutrients").default_open(true).show(ui, |ui| {
        let s = &mut eco.settings;
        let ticks = |days: f32| ((days * frame.info.ticks_per_day as f32).round() as u64).max(1);
        slider(ui, "Spell (days)", egui::Slider::new(&mut s.spell_days, 1.0..=120.0));
        let edit = |d: Disaster| runner.0.send(Control::Edit(EcosystemCommand::Disaster(d)));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Drought").on_hover_text("No rain, +2 °C and ×1.5 evaporation for the spell.").clicked() {
                edit(Disaster::Drought { duration_ticks: ticks(s.spell_days) });
                toasts.info(format!("{:.0}-day drought imposed.", s.spell_days));
            }
            if ui.button("Heatwave").on_hover_text("+4.2 °C to +7 °C and ×1.4 evaporation for the spell.").clicked()
            {
                edit(Disaster::Heatwave { duration_ticks: ticks(s.spell_days) });
                toasts.info(format!("{:.0}-day heatwave imposed.", s.spell_days));
            }
            if ui.button("Storm").on_hover_text("Rain × 3 × intensity, gusts and lightning for the spell.").clicked()
            {
                edit(Disaster::Storm { duration_ticks: ticks(s.spell_days), intensity: s.storm_intensity });
                toasts.info(format!("{:.0}-day storm imposed.", s.spell_days));
            }
        });
        slider(ui, "Storm intensity (× rain)", egui::Slider::new(&mut s.storm_intensity, 0.2..=10.0));
        slider(ui, "Rain to add (mm)", egui::Slider::new(&mut s.rain_mm, 1.0..=500.0).logarithmic(true));
        if ui.button("Add rain").on_hover_text("Rain (snow below 0 °C) on every land cell at once.").clicked() {
            edit(Disaster::AddRain { amount_mm: s.rain_mm });
            toasts.info(format!("{:.0} mm of rain added.", s.rain_mm));
        }
        note(
            ui,
            "Water leaves the land by flowing to rivers, lakes and the sea, by infiltration and \
                  evaporation; a drought is the way to dry the landscape (no command removes water).",
        );
        // Long-term inputs are model parameters.
        let params = status.get().map(|st| &st.params);
        let value = |key: &str| params.and_then(|p| p.get(key)).and_then(ParamValue::as_f64);
        if let Some(mut rain) = value("rainfall_mm") {
            let r = slider(ui, "Annual rainfall (mm)", egui::Slider::new(&mut rain, 0.0..=8000.0).logarithmic(true));
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                runner.0.send(Control::SetParam("rainfall_mm".into(), ParamValue::Float(rain)));
            }
        }
        if let Some(mut dep) = value("deposition_g_m2_year") {
            let r = slider(
                ui,
                "Nutrient deposition (g/m² per year)",
                egui::Slider::new(&mut dep, 0.0..=100.0).logarithmic(true),
            );
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                runner.0.send(Control::SetParam("deposition_g_m2_year".into(), ParamValue::Float(dep)));
            }
        }
        note(
            ui,
            "Rainfall and nutrient deposition are model parameters (applied next tick; also in the \
                  Parameters tab). Deposition adds mineral nutrients to the land each year; the model has no \
                  command that removes nutrients — leaching and fire volatilisation do.",
        );
    });
}

fn disease_actions(ui: &mut egui::Ui, eco: &mut EcoUi, frame: &Frame, runner: &EcoRunner, toasts: &mut Toasts) {
    egui::CollapsingHeader::new("Disease outbreak").default_open(false).show(ui, |ui| {
        let pathogens = &frame.info.pathogens;
        if pathogens.is_empty() {
            note(
                ui,
                "This scenario defines no pathogen. Outbreaks use the scenario's pathogens (with their \
                      transmission, recovery, immunity and mortality); try the temperate forest.",
            );
            return;
        }
        let s = &mut eco.settings;
        if !pathogens.iter().any(|p| p.id == s.pathogen) {
            s.pathogen = pathogens[0].id.clone();
        }
        let Some(p) = pathogens.iter().find(|p| p.id == s.pathogen) else { return };
        egui::ComboBox::from_id_salt("eco-pathogen").selected_text(p.name.clone()).show_ui(ui, |ui| {
            for q in pathogens {
                ui.selectable_value(&mut s.pathogen, q.id.clone(), q.name.clone());
            }
        });
        if !p.hosts.contains(&s.outbreak_species) {
            s.outbreak_species = p.hosts.first().cloned().unwrap_or_default();
        }
        egui::ComboBox::from_id_salt("eco-outbreak-host").selected_text(s.outbreak_species.clone()).show_ui(ui, |ui| {
            for h in &p.hosts {
                let name = frame.info.catalog.animal(h).map_or(h.clone(), |sp| sp.name.clone());
                ui.selectable_value(&mut s.outbreak_species, h.clone(), name);
            }
        });
        slider(ui, "Hosts to infect", egui::Slider::new(&mut s.outbreak_count, 1..=200));
        super::grid(
            ui,
            "eco-pathogen-traits",
            &[
                ("Transmission", format!("{:.3} per contact-hour", p.transmission_per_hour)),
                ("Contact distance", format!("{:.0} m", p.contact_m)),
                ("Infectious for", format!("{:.0} days", p.infectious_days)),
                ("Mortality", format!("{:.3} per day", p.mortality_per_day)),
                (
                    "Immunity",
                    if p.immunity_days > 0.0 { format!("{:.0} days", p.immunity_days) } else { "for life".into() },
                ),
                ("Virulence", format!("{:.2}", p.virulence)),
            ],
        );
        if ui.button("Start the outbreak").clicked() {
            runner.0.send(Control::Edit(EcosystemCommand::Outbreak {
                pathogen: s.pathogen.clone(),
                species: s.outbreak_species.clone(),
                count: s.outbreak_count,
            }));
            toasts.info(format!("Outbreak of {} started in {} hosts.", p.name, s.outbreak_count));
            eco.overlay = Overlay::Disease;
        }
        note(
            ui,
            "Infects random susceptible hosts; the infection then spreads by contact (SIR). Watch the \
                  Disease prevalence overlay and the Deaths & disease tab.",
        );
    });
}
