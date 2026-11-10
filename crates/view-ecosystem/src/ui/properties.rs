//! Right panel: tab row, scenario card, organism inspector and species
//! sheet; the analytics tabs live in [`super::analytics`] and the food web
//! in [`super::web`].

use bevy::prelude::*;
use bevy_egui::egui;
use ecosystem::animal::Target;
use ecosystem::species::{Development, Food, Representation};
use ecosystem::view::{AnimalInspection, TreeInspection};
use ecosystem::{EcosystemCommand, InfectionStage, OrganismInspection, OrganismRef, ReproState, Sex};
use interface::{MetricsHistory, ModeRegistry, Palette, ThemeSettings};
use simulation_core::Control;

use super::{UiCache, bar, color32, energy, grid, heading, mass, note, num};
use crate::MODE;
use crate::interact::select;
use crate::mapping::{action_label, tree_stage_label};
use crate::palette::category_color;
use crate::scenarios::card;
use crate::scene::terrain::MapState;
use crate::session::{FeedingMode, Frame};
use crate::state::{CameraMode, EcoData, EcoRunner, EcoUi, PropertiesTab};

/// The Properties panel.
#[allow(clippy::too_many_arguments)]
pub fn properties_panel(
    InMut(ui): InMut<egui::Ui>,
    mut eco: ResMut<EcoUi>,
    data: Res<EcoData>,
    runner: Option<Res<EcoRunner>>,
    history: Res<MetricsHistory>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    time: Res<Time<Real>>,
    map: Res<MapState>,
    mut cache: ResMut<UiCache>,
) {
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    let Some(frame) = data.get() else {
        ui.label("Waiting for the simulation…");
        return;
    };
    ui.horizontal_wrapped(|ui| {
        for tab in PropertiesTab::ALL {
            if ui.add(egui::Button::selectable(eco.tab == tab, tab.label())).clicked() {
                eco.tab = tab;
            }
        }
    });
    ui.separator();
    match eco.tab {
        PropertiesTab::Scenario => scenario_tab(ui, frame, &mut eco),
        PropertiesTab::Inspector => {
            if let Some(runner) = runner {
                inspector_tab(ui, frame, &mut eco, &runner, &palette, &map, &mut cache);
            }
        }
        PropertiesTab::Species => species_tab(ui, frame, &mut eco),
        PropertiesTab::Populations => super::analytics::populations_tab(ui, frame, &history, &mut eco, &palette),
        PropertiesTab::Energy => super::analytics::energy_tab(ui, frame, &history, &palette),
        PropertiesTab::Climate => super::analytics::climate_tab(ui, frame, &history, &palette),
        PropertiesTab::Deaths => super::analytics::deaths_tab(ui, frame, &history, &palette),
        PropertiesTab::FoodWeb => {
            super::web::food_web_tab(ui, frame, data.seq, time.elapsed_secs_f64(), &mut cache, &mut eco, &palette)
        }
    }
}

fn scenario_tab(ui: &mut egui::Ui, frame: &Frame, eco: &mut EcoUi) {
    let meta = &frame.info.scenario;
    ui.label(egui::RichText::new(&meta.name).strong().size(16.0));
    let (w, h) = frame.info.size;
    let cell = frame.info.cell_size_m;
    note(
        ui,
        &format!(
            "{w} × {h} cells of {cell:.0} m ({:.1} km²) · seed {} · {} rules · 1 tick = {:.0} simulated minutes",
            (w * h) as f32 * cell * cell / 1e6,
            frame.info.seed,
            if frame.info.netlogo.is_some() { "NetLogo" } else { "ecological" },
            frame.info.minutes_per_tick
        ),
    );
    ui.add_space(4.0);
    ui.label(&meta.description);
    if let Some(c) = card(&meta.id) {
        heading(ui, "What to watch");
        ui.label(c.watch);
        heading(
            ui,
            if c.measured { "What the model's documentation measured" } else { "What the model's rules produce here" },
        );
        ui.label(c.expected);
        ui.add_space(4.0);
        if ui.button(format!("Open {}", c.focus.label())).clicked() {
            eco.tab = c.focus;
        }
    }
    if !meta.events.is_empty() {
        heading(ui, "Scheduled interventions");
        note(
            ui,
            "Only the interventions are scripted; their consequences emerge. Days count elapsed time from 0 \
             at the start, so day 22 begins with the run's 23rd day on the clock.",
        );
        for (day, text) in &meta.events {
            let done = f64::from(*day) * f64::from(frame.info.ticks_per_day) <= frame.view.tick as f64;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("day {day:.0}")).monospace());
                ui.label(if done { egui::RichText::new(text).weak() } else { egui::RichText::new(text) });
            });
        }
    }
    heading(ui, "Species in this run");
    super::table(ui, "eco-scenario-species", 3).show(ui, |ui| {
        for (i, info) in frame.view.species.iter().enumerate() {
            let ever = frame
                .counters
                .total_births
                .get(i)
                .zip(frame.counters.total_deaths.get(i))
                .is_some_and(|(b, d)| b + d > 0);
            if info.population == 0 && !ever {
                continue;
            }
            let c = color32(category_color(i));
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.5, c);
                if ui.link(&info.name).clicked() {
                    eco.species = Some(i as u16);
                    eco.tab = PropertiesTab::Species;
                }
            });
            ui.label(num(f64::from(info.population)));
            ui.label(egui::RichText::new(&info.representation).weak());
            ui.end_row();
        }
    });
}

fn day(frame: &Frame, tick: u64) -> f64 {
    tick as f64 / f64::from(frame.info.ticks_per_day.max(1))
}

fn target_text(frame: &Frame, target: &Target) -> String {
    match target {
        Target::None => "—".into(),
        Target::Animal(id) => {
            let name = frame
                .view
                .animals
                .iter()
                .find(|a| a.id == id.get())
                .and_then(|a| frame.info.catalog.animals().get(usize::from(a.species)))
                .map_or("animal".to_string(), |s| s.name.clone());
            format!("{name} #{}", id.get())
        }
        Target::Carcass(id) => format!("carcass #{id}"),
        Target::Cell(c) => {
            let w = frame.info.size.0.max(1) as u32;
            format!("cell ({}, {})", c % w, c / w)
        }
        Target::Point(p) => format!("point ({:.1}, {:.1})", p.x, p.y),
        Target::Clutch(id) => format!("clutch #{id}"),
    }
}

fn inspector_tab(
    ui: &mut egui::Ui,
    frame: &Frame,
    eco: &mut EcoUi,
    runner: &EcoRunner,
    palette: &Palette,
    map: &MapState,
    cache: &mut UiCache,
) {
    let Some(selection) = eco.selection else {
        heading(ui, "Nothing selected");
        note(
            ui,
            "Click an animal or a tree in the world (Inspect tool). The inspector shows the model's full \
                  state of the organism: physiology, decisions, family, genes and history.",
        );
        return;
    };
    let matches = |s: &OrganismInspection| match (s, selection) {
        (OrganismInspection::Animal(a), OrganismRef::Animal(id)) => a.id == id,
        (OrganismInspection::Tree(t), OrganismRef::Tree(id)) => t.id == id,
        _ => false,
    };
    // The model includes the inspection exactly while the organism exists.
    let current = frame.view.selected.clone().filter(matches);
    let alive = current.is_some();
    if alive {
        cache.last_inspection = current.clone();
    }
    let shown = current.or_else(|| cache.last_inspection.clone().filter(matches));
    ui.horizontal(|ui| {
        if matches!(selection, OrganismRef::Animal(_)) {
            let following = eco.camera == CameraMode::Follow;
            if ui.add(egui::Button::selectable(following, "Follow")).on_hover_text("Shortcut: F").clicked() {
                eco.camera = if following { CameraMode::Orbit } else { CameraMode::Follow };
            }
        }
        let position = match &shown {
            Some(OrganismInspection::Animal(a)) => Some((a.kinematics.0, a.altitude_m)),
            Some(OrganismInspection::Tree(t)) => Some((t.position, 0.0)),
            None => None,
        };
        if let (Some((cell, altitude)), Some(g)) = (position, map.geometry.as_ref())
            && alive
            && ui.button("Look at it").on_hover_text("Moves the camera focus to it.").clicked()
        {
            let p = g.ground_point(cell);
            eco.focus = Some(Vec3::new(p.x, (g.surface(p.x, p.z) + altitude).max(p.y), p.z));
        }
        if ui.button("Deselect").clicked() {
            select(runner, eco, None);
        }
        if let Some(OrganismInspection::Animal(a)) = &shown
            && alive
            && ui.button("Kill").on_hover_text("The animal dies and becomes a carcass (undoable).").clicked()
        {
            runner.0.send(Control::Edit(EcosystemCommand::KillAnimal { id: a.id }));
        }
        if let Some(OrganismInspection::Tree(t)) = &shown
            && alive
            && ui.button("Fell").on_hover_text("The tree falls into the dead-wood pool (undoable).").clicked()
        {
            runner.0.send(Control::Edit(EcosystemCommand::FellTree { id: t.id }));
        }
    });
    match shown {
        None => {
            note(ui, "Waiting for the model's inspection…");
        }
        Some(OrganismInspection::Animal(a)) => {
            if !alive {
                ui.colored_label(palette.warning, "This animal is gone; its last recorded state is shown.");
            }
            animal_inspector(ui, frame, eco, &a, palette);
        }
        Some(OrganismInspection::Tree(t)) => {
            if !alive {
                ui.colored_label(palette.warning, "This tree is gone; its last recorded state is shown.");
            }
            tree_inspector(ui, frame, &t);
        }
    }
}

/// Life stage of an animal; `eggs` tells whether its species lays eggs
/// (a gestating female is then forming a clutch, not pregnant).
fn life_stage(a: &AnimalInspection, eggs: bool) -> &'static str {
    match a.reproduction {
        ReproState::Juvenile => "juvenile",
        ReproState::Ready => "adult, ready to breed",
        ReproState::Gestating(_) if eggs => "adult, forming eggs",
        ReproState::Gestating(_) => "adult, pregnant",
        ReproState::Recovering { .. } => "adult, between breeding attempts",
    }
}

/// Whether a species lays eggs.
fn lays_eggs(frame: &Frame, species: &str) -> bool {
    frame.info.catalog.animal(species).is_some_and(|s| matches!(s.reproduction.development, Development::Eggs { .. }))
}

fn animal_inspector(ui: &mut egui::Ui, frame: &Frame, eco: &mut EcoUi, a: &AnimalInspection, palette: &Palette) {
    let species_index = frame.info.catalog.animal_index(&a.species).ok();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("{} #{}", a.name, a.id)).strong().size(16.0));
        if let Some(i) = species_index
            && ui.link("species sheet").clicked()
        {
            eco.species = Some(i);
            eco.tab = PropertiesTab::Species;
        }
    });
    let sex = match a.sex {
        Sex::Female => "female",
        Sex::Male => "male",
    };
    note(
        ui,
        &format!(
            "{sex}, {:.1} days old ({:.0} % of the lifespan), {}",
            a.age_days,
            a.age_fraction * 100.0,
            life_stage(a, lays_eggs(frame, &a.species))
        ),
    );
    heading(ui, "Behaviour");
    let (action, target, ticks) = &a.action;
    grid(
        ui,
        "eco-behaviour",
        &[
            ("Doing", action_label(*action).to_string()),
            ("Goal", target_text(frame, target)),
            ("For", format!("{ticks} ticks ({:.1} h)", *ticks as f64 * frame.info.minutes_per_tick / 60.0)),
            (
                "Where",
                format!(
                    "cell ({:.1}, {:.1}){}{}",
                    a.kinematics.0.x,
                    a.kinematics.0.y,
                    if a.altitude_m > 1.0 { format!(", {:.0} m up", a.altitude_m) } else { String::new() },
                    if a.hidden { ", hidden in its burrow" } else { "" }
                ),
            ),
        ],
    );
    if !a.utility.is_empty() {
        egui::CollapsingHeader::new("Utility of each option (what it weighs now)").default_open(true).show(ui, |ui| {
            let mut scores = a.utility.clone();
            scores.sort_by(|x, y| y.1.total_cmp(&x.1));
            let max = scores.first().map_or(1.0, |s| s.1).max(1e-6);
            for (k, v) in scores.iter().take(8) {
                bar(ui, k, v / max, palette.accent, palette);
            }
            note(
                ui,
                "Scores of the utility AI for the current state (relative to the best); the chosen action \
                      keeps a momentum bonus and decisions carry 5 % noise.",
            );
        });
    }
    if !a.senses.is_empty() {
        let rows: Vec<(&str, String)> =
            a.senses.iter().map(|(k, d)| (k.as_str(), format!("{:.0} m", d * frame.info.cell_size_m))).collect();
        egui::CollapsingHeader::new("Perceives").default_open(false).show(ui, |ui| grid(ui, "eco-senses", &rows));
    }
    heading(ui, "Body and condition (individual state)");
    let (energy_now, energy_cap) = a.energy_kj;
    let (hunger, thirst, fatigue) = a.drives;
    let (stamina, health, comfort) = a.condition;
    grid(
        ui,
        "eco-body",
        &[
            ("Mass", format!("{} of {} adult", mass(f64::from(a.mass_kg.0)), mass(f64::from(a.mass_kg.1)))),
            ("Energy reserve", format!("{} of {}", energy(f64::from(energy_now)), energy(f64::from(energy_cap)))),
            ("Nutrient store", format!("{:.1} g", a.nutrient_store_g)),
            ("Air temperature", format!("{:.1} °C", a.temperature_c)),
        ],
    );
    let ok = palette.success;
    bar(ui, "reserves", energy_now / energy_cap.max(1e-6), ok, palette);
    bar(ui, "hydration", 1.0 - thirst, egui::Color32::from_rgb(70, 150, 230), palette);
    bar(ui, "satiety", 1.0 - hunger, ok, palette);
    bar(ui, "rested", 1.0 - fatigue, ok, palette);
    bar(ui, "stamina", stamina, ok, palette);
    bar(ui, "health", health, if health < 0.4 { palette.error } else { ok }, palette);
    bar(ui, "thermal comfort", comfort, if comfort < 0.5 { palette.warning } else { ok }, palette);
    if a.injury > 0.0 {
        bar(ui, "injury", a.injury, palette.error, palette);
    }
    if let Some(infection) = a.infection {
        let name = frame
            .info
            .pathogens
            .get(usize::from(infection.pathogen))
            .map_or("pathogen".to_string(), |p| p.name.clone());
        let text = match infection.stage {
            InfectionStage::Infected { since } => {
                format!("infectious with {name} since day {:.1}", day(frame, since))
            }
            InfectionStage::Recovered { until } if until == u64::MAX => {
                format!("recovered from {name}, immune for life")
            }
            InfectionStage::Recovered { until } => {
                format!("recovered from {name}, immune until day {:.1}", day(frame, until))
            }
        };
        ui.colored_label(palette.warning, text);
    }
    heading(ui, "Society");
    let traits = species_index.and_then(|i| frame.info.traits.get(usize::from(i)));
    let mut rows = vec![(
        "Group",
        match a.group {
            Some((g, Some(leader))) if leader == a.id => format!("#{g} — it leads"),
            Some((g, Some(leader))) => format!("#{g}, led by #{leader}"),
            Some((g, None)) => format!("#{g}"),
            None => "alone".into(),
        },
    )];
    if let (Some(home), Some(t)) = (a.home, traits)
        && t.territory_cells > 0.0
    {
        rows.push((
            "Territory",
            format!(
                "centre ({:.1}, {:.1}), radius {:.0} m",
                home.x,
                home.y,
                t.territory_cells * frame.info.cell_size_m
            ),
        ));
    } else if let Some(home) = a.home {
        rows.push(("Burrow / den", format!("({:.1}, {:.1})", home.x, home.y)));
    }
    if let Some((p, s)) = a.alarm {
        rows.push(("Alarm call", format!("strength {s:.2}, threat at ({:.1}, {:.1})", p.x, p.y)));
    }
    let (generation, parents) = a.lineage;
    rows.push((
        "Lineage",
        format!(
            "generation {generation}; parents {}",
            match parents {
                [Some(m), Some(f)] => format!("#{m} and #{f}"),
                [Some(m), None] | [None, Some(m)] => format!("#{m}"),
                [None, None] => "— (founder)".into(),
            }
        ),
    ));
    rows.push(("Offspring", format!("{} born; {} recorded in the genealogy", a.stats.offspring, a.children.len())));
    if let ReproState::Gestating(p) = &a.reproduction {
        let text = if lays_eggs(frame, &a.species) {
            format!("{} eggs, to be laid on day {:.1}", p.litter, day(frame, p.due))
        } else {
            format!("{} young, due on day {:.1}", p.litter, day(frame, p.due))
        };
        rows.push(("Expecting", text));
    }
    grid(ui, "eco-society", &rows);
    heading(ui, "Life record");
    grid(
        ui,
        "eco-record",
        &[
            ("Kills", a.stats.kills.to_string()),
            ("Escapes", a.stats.escapes.to_string()),
            ("Travelled", format!("{:.1} km", a.stats.distance_cells * frame.info.cell_size_m / 1000.0)),
            ("Eaten", energy(a.stats.eaten_kj)),
        ],
    );
    if !a.genes.is_empty() {
        egui::CollapsingHeader::new("Heritable traits (genome)").default_open(true).show(ui, |ui| {
            note(
                ui,
                "Expressed genes, inherited by crossover of the parents' genomes with mutation \
                      (mutation_rate); everything above is individual state.",
            );
            let rows: Vec<(&str, String)> = a.genes.iter().map(|(k, v)| (k.as_str(), format!("{v:.3}"))).collect();
            grid(ui, "eco-genes", &rows);
        });
    }
    egui::CollapsingHeader::new("Recent events").default_open(true).show(ui, |ui| {
        if a.history.is_empty() {
            note(ui, "No events recorded yet (follow the animal to log its decisions).");
        }
        for e in a.history.iter().rev().take(14) {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(format!("day {:.2}", day(frame, e.tick))).monospace().weak());
                ui.label(&e.message);
            });
        }
    });
}

fn tree_inspector(ui: &mut egui::Ui, frame: &Frame, t: &TreeInspection) {
    ui.label(egui::RichText::new(format!("{} #{}", t.species.1, t.id)).strong().size(16.0));
    note(ui, &format!("{}, {:.1} years old (compressed years of the model)", tree_stage_label(t.stage), t.age_years));
    grid(
        ui,
        "eco-tree",
        &[
            ("Height", format!("{:.1} m", t.size_m.0)),
            ("Crown radius", format!("{:.1} m", t.size_m.1)),
            ("Wood", mass(f64::from(t.biomass_kg.0))),
            ("Leaves", mass(f64::from(t.biomass_kg.1))),
            ("Health", format!("{:.0} %", t.condition.0 * 100.0)),
            ("Light at the crown", format!("{:.0} %", t.condition.1 * 100.0)),
            ("Fire scar", if t.burnt { "yes".into() } else { "no".into() }),
            ("Position", format!("cell ({:.1}, {:.1})", t.position.x, t.position.y)),
        ],
    );
    if let Some((tick, cause)) = t.death {
        note(
            ui,
            &format!(
                "Died on day {:.1} ({}); stands as a snag before it falls.",
                day(frame, tick),
                cause.map_or("unknown cause".into(), |c| format!("{c:?}").to_lowercase())
            ),
        );
    }
    if let Some(sp) = frame.info.catalog.trees().iter().find(|s| s.id == t.species.0) {
        heading(ui, "Species");
        grid(
            ui,
            "eco-tree-species",
            &[
                ("Scientific name", sp.scientific_name.clone()),
                ("Leaves", if sp.deciduous { "deciduous".into() } else { "evergreen".into() }),
                ("Maximum height", format!("{:.0} m", sp.max_height_m)),
                ("Lifespan", format!("{:.0} years", sp.lifespan_years)),
                ("Shade tolerance", format!("{:.2}", sp.shade_tolerance)),
                ("Drought tolerance", format!("{:.2}", sp.drought_tolerance)),
            ],
        );
    }
}

fn food_label(food: &Food) -> String {
    match food {
        Food::Plant(layer) => format!("{} (plants)", layer.key()),
        Food::Browse => "tree foliage (browse)".into(),
        Food::Prey(id) => id.clone(),
        Food::Carrion => "carrion".into(),
        Food::Nectar => "nectar".into(),
        Food::Detritus => "detritus".into(),
        Food::Fungi => "mushrooms".into(),
        Food::Eggs => "eggs of other species".into(),
    }
}

fn species_tab(ui: &mut egui::Ui, frame: &Frame, eco: &mut EcoUi) {
    let animals = frame.info.catalog.animals();
    // Without a choice: the selected animal's species, else the most
    // numerous individual species of the run.
    let selected = match eco.selection {
        Some(ecosystem::OrganismRef::Animal(id)) => frame.view.animals.iter().find(|a| a.id == id).map(|a| a.species),
        _ => None,
    };
    let default = || {
        selected.unwrap_or_else(|| {
            frame
                .view
                .species
                .iter()
                .enumerate()
                .filter(|(i, _)| animals.get(*i).is_some_and(|s| s.is_individual()))
                .max_by_key(|(_, p)| p.population)
                .map_or(0, |(i, _)| i as u16)
        })
    };
    let index = usize::from(eco.species.unwrap_or_else(default)).min(animals.len().saturating_sub(1));
    egui::ComboBox::from_id_salt("eco-species-sheet").selected_text(animals[index].name.clone()).show_ui(ui, |ui| {
        for (i, s) in animals.iter().enumerate() {
            let pop = frame.view.species.get(i).map_or(0, |p| p.population);
            let text = if pop > 0 { format!("{} ({})", s.name, num(f64::from(pop))) } else { s.name.clone() };
            if ui.selectable_label(i == index, text).clicked() {
                eco.species = Some(i as u16);
            }
        }
    });
    let s = &animals[index];
    let t = &frame.info.traits[index];
    ui.label(egui::RichText::new(&s.name).strong().size(16.0));
    note(ui, &format!("{} · {:?}", s.scientific_name, s.class));
    let level = match frame.info.trophic[index] {
        ecosystem::metrics::TrophicLevel::Primary => "primary consumer",
        ecosystem::metrics::TrophicLevel::Secondary => "secondary consumer",
        ecosystem::metrics::TrophicLevel::Apex => "apex predator",
    };
    let roles: Vec<String> = s.roles.iter().map(|r| format!("{r:?}").to_lowercase()).collect();
    let representation = match &s.representation {
        Representation::Individual => "individual animals".to_string(),
        Representation::Field(_) => "density field (individuals per m² in every cell)".to_string(),
        Representation::Colony(_) => "colonies (super-agents with workers and forager squads)".to_string(),
    };
    let population = frame.view.species.get(index).map_or(0, |p| p.population);
    grid(
        ui,
        "eco-species-identity",
        &[
            ("Trophic level", level.to_string()),
            ("Roles", roles.join(", ")),
            ("Simulated as", representation),
            ("Population now", num(f64::from(population))),
            (
                "Births / deaths",
                format!(
                    "{} / {} since the start",
                    frame.counters.total_births.get(index).copied().unwrap_or(0),
                    frame.counters.total_deaths.get(index).copied().unwrap_or(0)
                ),
            ),
        ],
    );
    heading(ui, "Diet");
    super::table(ui, "eco-diet", 3).show(ui, |ui| {
        ui.label(egui::RichText::new("food").weak());
        ui.label(egui::RichText::new("preference").weak());
        ui.label(egui::RichText::new("assimilated").weak());
        ui.end_row();
        for d in &s.diet {
            ui.label(food_label(&d.food));
            ui.label(format!("{:.2}", d.preference));
            ui.label(format!("{:.0} %", d.assimilation() * 100.0));
            ui.end_row();
        }
    });
    if s.is_individual() && frame.info.netlogo.is_none() {
        let observed = &frame.observed;
        let total: f64 = FeedingMode::ALL.iter().map(|m| observed.eaten_by(index as u16, *m)).sum();
        if total > 0.0 {
            let days = observed.ticks as f64 / f64::from(frame.info.ticks_per_day.max(1));
            heading(ui, "Meals observed");
            note(
                ui,
                &format!(
                    "Gross energy eaten by all its individuals over the last {days:.1} days, by what they were doing."
                ),
            );
            for mode in FeedingMode::ALL {
                let kj = observed.eaten_by(index as u16, mode);
                if kj > 0.0 {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(mode.label()).weak());
                        ui.label(format!("{} ({:.0} %)", energy(kj), 100.0 * kj / total));
                    });
                }
            }
        }
    }
    heading(ui, "Body and metabolism");
    let days = |ticks: f64| ticks / f64::from(frame.info.ticks_per_day.max(1));
    grid(
        ui,
        "eco-species-body",
        &[
            ("Adult mass", mass(f64::from(s.body.mass_kg))),
            ("Body length", format!("{:.2} m", s.body.length_m)),
            ("Thermoregulation", format!("{:?}", s.body.thermoregulation).to_lowercase()),
            ("Basal metabolism", format!("{}/day", energy(f64::from(t.bmr_kj_per_day)))),
            ("Field metabolism", format!("{}/day", energy(f64::from(t.field_kj_per_day)))),
            ("Energy reserve", energy(f64::from(t.reserve_max_kj))),
            ("Sprint", format!("{:.1} m/s", t.sprint_m_s)),
            ("Vision", format!("{:.0} m", t.vision_cells * frame.info.cell_size_m)),
        ],
    );
    heading(ui, "Life history and reproduction");
    let r = &s.reproduction;
    let development = match r.development {
        Development::Live { .. } => format!("live birth after {:.1} days", days(t.gestation_ticks)),
        Development::Eggs { .. } => format!(
            "eggs formed in {:.1} days, hatching after {:.1} days",
            days(t.gestation_ticks),
            days(t.incubation_ticks)
        ),
    };
    let seasons: Vec<&str> = r.breeding_seasons.iter().map(|s| s.label()).collect();
    grid(
        ui,
        "eco-species-life",
        &[
            ("Lifespan", format!("{:.0} days (1 % reach it)", days(t.lifespan_ticks))),
            ("Maturity", format!("{:.0} days", days(t.maturity_ticks))),
            ("Development", development),
            ("Litter / clutch", format!("{}–{}", r.litter.0, r.litter.1)),
            ("Breeding seasons", if seasons.is_empty() { "any".into() } else { seasons.join(", ") }),
            ("Between births", format!("{:.0} days", days(t.interval_ticks))),
            ("Social structure", format!("{:?}", s.social.structure).to_lowercase()),
            ("Activity", format!("{:?}", s.behaviour.activity).to_lowercase()),
        ],
    );
    note(
        ui,
        "Durations are simulated days of the compressed calendar (life-history times scaled by \
              days per year / 365).",
    );
}
