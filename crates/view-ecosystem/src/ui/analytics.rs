//! Analytics tabs: populations and trophic groups, energy and nutrient
//! budgets (with the model's balance errors), climate, deaths and disease.
//! Time series come from the shell's metric history (the model's metrics);
//! totals from the session's frame (the model's counters and ledgers).

use bevy_egui::egui;
use ecosystem::species::Representation;
use ecosystem::view::DEATH_CAUSES;
use ecosystem::{Flux, FluxKind};
use interface::{MetricsHistory, Palette};

use super::{Series, TimeAxis, color32, energy, grid, heading, mass, meter, moving_mean, note, num, time_plot};
use crate::palette::category_color;
use crate::session::Frame;
use crate::session::{LedgerView, cause_label};
use crate::state::EcoUi;

/// The time axis of a frame's run.
pub fn axis(frame: &Frame) -> TimeAxis {
    TimeAxis::new(frame.info.ticks_per_day, frame.info.netlogo.is_some())
}

fn series(history: &MetricsHistory, frame: &Frame, key: &str) -> Option<Vec<[f64; 2]>> {
    let axis = axis(frame);
    let points = history.store().display(key)?;
    (!points.is_empty()).then(|| points.iter().map(|p| [axis.at(p.tick), p.value]).collect())
}

fn latest(history: &MetricsHistory, key: &str) -> Option<f64> {
    history.latest(key)
}

/// Populations, small life, trophic biomass and diversity.
pub fn populations_tab(ui: &mut egui::Ui, frame: &Frame, history: &MetricsHistory, eco: &mut EcoUi, palette: &Palette) {
    let species = frame.info.catalog.animals();
    heading(ui, "Individual animals");
    ui.horizontal(|ui| {
        ui.checkbox(&mut eco.all_species, "all species").on_hover_text("Otherwise the ten most numerous.");
        ui.checkbox(&mut eco.log_populations, "log scale");
    });
    let mut individuals: Vec<(usize, f64)> = species
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_individual())
        .filter_map(|(i, s)| latest(history, &format!("population.{}", s.id)).map(|v| (i, v)))
        .filter(|(i, v)| *v > 0.0 || frame.counters.total_deaths.get(*i).is_some_and(|d| *d > 0))
        .collect();
    individuals.sort_by(|a, b| b.1.total_cmp(&a.1));
    if !eco.all_species {
        individuals.truncate(10);
    }
    let lines: Vec<Series> = individuals
        .iter()
        .filter_map(|&(i, _)| {
            let s = &species[i];
            series(history, frame, &format!("population.{}", s.id))
                .map(|p| (s.name.clone(), p, color32(category_color(i))))
        })
        .collect();
    if lines.is_empty() {
        note(ui, "The plots fill as the simulation runs (press Space).");
    } else {
        time_plot(ui, axis(frame), "eco-pop-individuals", lines, 190.0, "animals", eco.log_populations);
    }
    let small: Vec<Series> = species
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.is_individual())
        .filter_map(|(i, s)| {
            let p = series(history, frame, &format!("population.{}", s.id))?;
            p.iter().any(|v| v[1] > 0.0).then(|| (s.name.clone(), p, color32(category_color(i))))
        })
        .collect();
    if !small.is_empty() {
        heading(ui, "Small life (density fields and colony workers)");
        note(ui, "Field species count individuals (density × area); colony species count workers. Log scale.");
        time_plot(ui, axis(frame), "eco-pop-small", small, 180.0, "individuals", true);
    }
    if frame.info.netlogo.is_some() {
        if let Some(p) = series(history, frame, "green_patches") {
            heading(ui, "Grass");
            time_plot(
                ui,
                axis(frame),
                "eco-green",
                vec![("green patches".into(), p, palette.success)],
                120.0,
                "patches",
                false,
            );
        }
    } else {
        heading(ui, "Biomass by trophic level");
        let levels = [
            ("biomass_producers", "producers (plants, trees)", egui::Color32::from_rgb(90, 170, 80)),
            ("biomass_primary_consumers", "primary consumers", egui::Color32::from_rgb(220, 190, 70)),
            ("biomass_secondary_consumers", "secondary consumers", egui::Color32::from_rgb(230, 120, 60)),
            ("biomass_apex", "apex predators", egui::Color32::from_rgb(200, 60, 70)),
            ("biomass_decomposers", "fungi", egui::Color32::from_rgb(150, 110, 200)),
        ];
        let lines: Vec<Series> =
            levels.iter().filter_map(|(k, l, c)| series(history, frame, k).map(|p| (l.to_string(), p, *c))).collect();
        if !lines.is_empty() {
            time_plot(ui, axis(frame), "eco-trophic", lines, 170.0, "kg (log)", true);
            let rows: Vec<(&str, String)> =
                levels.iter().filter_map(|(k, l, _)| latest(history, k).map(|v| (*l, mass(v)))).collect();
            grid(ui, "eco-trophic-now", &rows);
            note(
                ui,
                "The biomass pyramid: each level holds what it gains from the one below minus what it \
                      respires, egests and loses to its consumers.",
            );
        }
    }
    heading(ui, "Diversity");
    grid(
        ui,
        "eco-diversity",
        &[
            ("Species present", latest(history, "richness").map_or("—".into(), |v| format!("{v:.0}"))),
            ("Shannon", latest(history, "shannon").map_or("—".into(), |v| format!("{v:.3} nats"))),
            ("Gini–Simpson", latest(history, "simpson").map_or("—".into(), |v| format!("{v:.3}"))),
            ("Mean age", latest(history, "mean_age_days").map_or("—".into(), |v| format!("{v:.1} days"))),
        ],
    );
    note(ui, "Shannon and Simpson count individual animals only (field counts would swamp them).");
}

fn ratio(numerator: f64, denominator: f64) -> String {
    if denominator <= 0.0 {
        return "—".into();
    }
    let r = 100.0 * numerator / denominator;
    if r >= 1.0 || r == 0.0 { format!("{r:.1} %") } else { format!("{r:.3} %") }
}

fn ledger_table(ui: &mut egui::Ui, id: &str, ledger: &LedgerView, unit: fn(f64) -> String) {
    super::table(ui, id, 4).show(ui, |ui| {
        ui.label(egui::RichText::new("flux").weak());
        ui.label(egui::RichText::new("role").weak());
        ui.label(egui::RichText::new("since the start").weak());
        ui.label(egui::RichText::new("last tick").weak());
        ui.end_row();
        for row in ledger.rows.iter().filter(|r| r.total > 0.0) {
            ui.label(row.flux.key().replace('_', " "));
            ui.label(match row.kind {
                FluxKind::Input => "in",
                FluxKind::Output => "out",
                FluxKind::Internal => "internal",
                FluxKind::Unused => "",
            });
            ui.label(unit(row.total));
            ui.label(unit(row.last_tick));
            ui.end_row();
        }
    });
}

fn balance(ui: &mut egui::Ui, id: &str, ledger: &LedgerView, unit: fn(f64) -> String, palette: &Palette) {
    let books = ledger.initial + ledger.inputs - ledger.outputs;
    grid(
        ui,
        id,
        &[
            ("Stock at the start", unit(ledger.initial)),
            ("+ inputs", unit(ledger.inputs)),
            ("− outputs", unit(ledger.outputs)),
            ("= books", unit(books)),
            ("Stock now", unit(ledger.stock)),
        ],
    );
    let color = if ledger.relative_error < 1e-4 { palette.success } else { palette.warning };
    ui.colored_label(color, format!("Balance error |stock − books| / stock = {:.2e}", ledger.relative_error));
}

fn grams(g: f64) -> String {
    mass(g / 1000.0)
}

/// Energy flows, transfer ratios, ledgers and nutrient pools.
pub fn energy_tab(ui: &mut egui::Ui, frame: &Frame, history: &MetricsHistory, palette: &Palette) {
    if frame.info.netlogo.is_some() {
        note(
            ui,
            "The NetLogo rule set counts energy in NetLogo units and keeps no energy or nutrient ledger; \
                  load an ecological scenario for the budgets.",
        );
        return;
    }
    heading(ui, "Energy flows per tick (hourly means)");
    let flows = [
        ("energy.primary_production", "primary production", egui::Color32::from_rgb(90, 170, 80)),
        ("energy.herbivory", "herbivory", egui::Color32::from_rgb(220, 190, 70)),
        ("energy.predation", "predation", egui::Color32::from_rgb(220, 80, 70)),
        ("energy.scavenging", "scavenging", egui::Color32::from_rgb(150, 110, 90)),
        ("energy.animal_respiration", "animal respiration", egui::Color32::from_rgb(90, 150, 230)),
        ("energy.decomposer_respiration", "decomposer respiration", egui::Color32::from_rgb(150, 110, 200)),
        ("energy.combustion", "combustion", egui::Color32::from_rgb(255, 140, 40)),
    ];
    // Vegetation grows hourly: an hour's mean shows the flows without the bursts.
    let hour = (frame.info.ticks_per_day as usize / 24).max(1);
    let lines: Vec<Series> = flows
        .iter()
        .filter_map(|(k, l, c)| series(history, frame, k).map(|p| (l.to_string(), moving_mean(&p, hour), *c)))
        .collect();
    if !lines.is_empty() {
        time_plot(ui, axis(frame), "eco-energy-flows", lines, 230.0, "kJ per tick (log)", true);
    }
    heading(ui, "Transfer between trophic levels");
    let e = &frame.energy;
    // Food of the first consumer level, by kind of food (the ledger books
    // flows by food, not by who ate it).
    let eaten_by_primary =
        e.total(Flux::Herbivory) + e.total(Flux::Nectarivory) + e.total(Flux::Detritivory) + e.total(Flux::Fungivory);
    let ingestion = eaten_by_primary + e.total(Flux::Predation) + e.total(Flux::Scavenging);
    let plants_eaten = e.total(Flux::Herbivory) + e.total(Flux::Nectarivory);
    grid(
        ui,
        "eco-transfer",
        &[
            ("Primary production", energy(e.total(Flux::PrimaryProduction))),
            ("Eaten from plants", energy(plants_eaten)),
            ("Litter and mushrooms eaten", energy(e.total(Flux::Detritivory) + e.total(Flux::Fungivory))),
            ("Prey eaten (fresh kills, gleaned insects)", energy(e.total(Flux::Predation))),
            ("Plants eaten / primary production", ratio(plants_eaten, e.total(Flux::PrimaryProduction))),
            ("Prey eaten / plants eaten", ratio(e.total(Flux::Predation), plants_eaten)),
            ("Prey eaten / plant, litter and mushroom food eaten", ratio(e.total(Flux::Predation), eaten_by_primary)),
            ("Egested as dung", ratio(e.total(Flux::Egestion), ingestion)),
            ("Respired by animals", ratio(e.total(Flux::AnimalRespiration), ingestion)),
        ],
    );
    note(
        ui,
        "Ratios of the cumulative flows since the start. The model has no fixed 10 % rule: each step's \
         efficiency emerges from assimilation (the rest is dung), respiration and what consumers leave \
         uneaten. Grazers eat a small share of what plants produce; most production goes to litter and \
         decomposers.",
    );
    egui::CollapsingHeader::new("Energy ledger (kJ)").default_open(true).show(ui, |ui| {
        balance(ui, "eco-energy-balance", e, energy, palette);
        ledger_table(ui, "eco-energy-ledger", e, energy);
    });
    egui::CollapsingHeader::new("Pools: energy and nutrients held").default_open(true).show(ui, |ui| {
        let s = &frame.stocks;
        let rows = [
            ("vegetation", s.vegetation),
            ("trees", s.trees),
            ("animals", s.animals),
            ("insects, worms, forage fish", s.fields),
            ("colonies", s.colonies),
            ("eggs", s.clutches),
            ("carcasses", s.carcasses),
            ("litter and dung", s.litter),
            ("dead wood", s.deadwood),
            ("fungi", s.fungi),
        ];
        super::table(ui, "eco-stocks", 3).show(ui, |ui| {
            for h in ["pool", "energy", "nutrients"] {
                ui.label(egui::RichText::new(h).weak());
            }
            ui.end_row();
            for (name, (kj, g)) in rows {
                ui.label(name);
                ui.label(energy(kj));
                ui.label(grams(g));
                ui.end_row();
            }
            ui.label("soil (mineral + organic)");
            ui.label("—");
            ui.label(grams(s.soil_mineral_g + s.soil_organic_g));
            ui.end_row();
        });
        note(
            ui,
            "Biomass pools as the model's ledgers count them (tissue 7 kJ/g, plant and litter dry matter \
                  18 kJ/g).",
        );
    });
    heading(ui, "Nutrient pools");
    let pools = [
        ("nutrients.soil_mineral", "soil mineral", egui::Color32::from_rgb(170, 120, 70)),
        ("nutrients.soil_organic", "soil organic", egui::Color32::from_rgb(110, 80, 50)),
        ("nutrients.plants", "plants", egui::Color32::from_rgb(90, 170, 80)),
        ("nutrients.animals", "animals", egui::Color32::from_rgb(220, 120, 70)),
        ("nutrients.carcasses", "carcasses and eggs", egui::Color32::from_rgb(200, 70, 80)),
        ("nutrients.detritus", "litter, dung, dead wood", egui::Color32::from_rgb(160, 140, 90)),
        ("nutrients.fungi", "fungi", egui::Color32::from_rgb(150, 110, 200)),
    ];
    let lines: Vec<Series> =
        pools.iter().filter_map(|(k, l, c)| series(history, frame, k).map(|p| (l.to_string(), p, *c))).collect();
    if !lines.is_empty() {
        time_plot(ui, axis(frame), "eco-nutrient-pools", lines, 170.0, "kg (log)", true);
    }
    egui::CollapsingHeader::new("Nutrient ledger (g)").default_open(true).show(ui, |ui| {
        balance(ui, "eco-nutrient-balance", &frame.nutrients, grams, palette);
        ledger_table(ui, "eco-nutrient-ledger", &frame.nutrients, grams);
    });
    let errors: Vec<Series> =
        [("energy_balance_error", "energy", palette.accent), ("nutrient_balance_error", "nutrients", palette.warning)]
            .iter()
            .filter_map(|(k, l, c)| series(history, frame, k).map(|p| (l.to_string(), p, *c)))
            .collect();
    if !errors.is_empty() {
        heading(ui, "Balance errors over time");
        time_plot(ui, axis(frame), "eco-errors", errors, 120.0, "relative error", false);
        note(
            ui,
            "Reported by the model every tick. The documented drift of the nutrient error comes from the \
                  physics world's single-precision soil grids (docs/ECOSYSTEM.md §14).",
        );
    }
}

/// Weather now and the climate series.
pub fn climate_tab(ui: &mut egui::Ui, frame: &Frame, history: &MetricsHistory, palette: &Palette) {
    let v = &frame.view;
    let w = &v.world.weather;
    heading(ui, "Now");
    let wind = w.wind;
    let bearing = (wind.x.atan2(-wind.y).to_degrees() + 360.0) % 360.0;
    grid(
        ui,
        "eco-weather",
        &[
            (
                "Date",
                format!(
                    "year {}, {} day {}, {:02}:{:02}",
                    v.time.year + 1,
                    v.season.label(),
                    v.time.day_of_season + 1,
                    v.time.hour,
                    v.time.minute
                ),
            ),
            ("Weather", w.kind.label().to_string()),
            ("Precipitation", format!("{:.2} mm/h", w.precipitation_rate_mm_h)),
            ("Cloud cover", format!("{:.0} %", w.cloud_cover * 100.0)),
            ("Wind", format!("{:.1} m/s towards {bearing:.0}°", wind.length())),
            ("Daylight", format!("{:.0} %", v.world.daylight * 100.0)),
            ("Drought index", format!("{:.2}", v.world.drought_index)),
            ("Flooded cells", v.world.flooded_cells.to_string()),
            ("Burning cells", v.world.burning_cells.to_string()),
            ("Imposed", w.forced.map_or("—".into(), |f| imposed_label(&f, frame.view.tick, frame.info.ticks_per_day))),
        ],
    );
    let plot = |ui: &mut egui::Ui, id: &str, title: &str, keys: &[(&str, &str, egui::Color32)], unit: &str| {
        let lines: Vec<Series> =
            keys.iter().filter_map(|(k, l, c)| series(history, frame, k).map(|p| (l.to_string(), p, *c))).collect();
        if !lines.is_empty() {
            heading(ui, title);
            time_plot(ui, axis(frame), id, lines, 130.0, unit, false);
        }
    };
    plot(ui, "eco-temp", "Temperature", &[("mean_temperature", "mean over land", palette.error)], "°C");
    plot(ui, "eco-rain", "Rain", &[("rain", "over land", egui::Color32::from_rgb(70, 140, 230))], "mm per tick");
    plot(
        ui,
        "eco-moisture",
        "Soil moisture and drought",
        &[
            ("soil_moisture", "soil moisture", egui::Color32::from_rgb(70, 160, 120)),
            ("drought_index", "drought index", egui::Color32::from_rgb(210, 150, 60)),
        ],
        "",
    );
    plot(
        ui,
        "eco-water",
        "Surface water",
        &[("water_volume", "rivers, lakes, floods", egui::Color32::from_rgb(60, 120, 220))],
        "m³",
    );
    // A fraction and a cell count do not share an axis.
    plot(
        ui,
        "eco-burning",
        "Fire",
        &[("burning_cells", "burning cells", egui::Color32::from_rgb(255, 120, 40))],
        "cells",
    );
    plot(
        ui,
        "eco-burned",
        "Burned ground",
        &[("burned_area", "burned share of the land", egui::Color32::from_rgb(120, 100, 90))],
        "fraction",
    );
    if frame.info.netlogo.is_none() {
        plot(
            ui,
            "eco-seasonal",
            "Seasonal behaviour",
            &[
                ("hibernating", "hibernating", egui::Color32::from_rgb(120, 150, 220)),
                ("migrating", "migrating", egui::Color32::from_rgb(220, 160, 80)),
                ("airborne", "airborne", egui::Color32::from_rgb(160, 200, 240)),
            ],
            "animals",
        );
    }
}

/// Births, deaths by cause, disease and the event log.
pub fn deaths_tab(ui: &mut egui::Ui, frame: &Frame, history: &MetricsHistory, palette: &Palette) {
    heading(ui, "Deaths by cause (since the start)");
    let c = &frame.counters;
    let most = c.deaths_by_cause.iter().copied().max().unwrap_or(0).max(1) as f32;
    for (i, cause) in DEATH_CAUSES.iter().enumerate() {
        let n = c.deaths_by_cause[i];
        meter(ui, cause_label(*cause), 110.0, n as f32 / most, &num(n as f64), color32(category_color(i + 3)), palette);
    }
    let lines: Vec<Series> = DEATH_CAUSES
        .iter()
        .enumerate()
        .filter(|(i, _)| c.deaths_by_cause[*i] > 0)
        .filter_map(|(i, cause)| {
            series(history, frame, &format!("deaths_by.{}", cause.key()))
                .map(|p| (cause_label(*cause).to_string(), p, color32(category_color(i + 3))))
        })
        .collect();
    if !lines.is_empty() {
        time_plot(ui, axis(frame), "eco-deaths-time", lines, 140.0, "cumulative deaths", false);
    }
    heading(ui, "Births and deaths per species");
    super::table(ui, "eco-births-deaths", 4).show(ui, |ui| {
        for h in ["species", "alive", "births", "deaths"] {
            ui.label(egui::RichText::new(h).weak());
        }
        ui.end_row();
        for (i, s) in frame.info.catalog.animals().iter().enumerate() {
            let (b, d) = (c.total_births.get(i).copied().unwrap_or(0), c.total_deaths.get(i).copied().unwrap_or(0));
            let alive = frame.view.species.get(i).map_or(0, |p| p.population);
            if !matches!(s.representation, Representation::Individual) || (b + d == 0 && alive == 0) {
                continue;
            }
            ui.label(&s.name);
            ui.label(num(f64::from(alive)));
            ui.label(num(b as f64));
            ui.label(num(d as f64));
            ui.end_row();
        }
    });
    grid(
        ui,
        "eco-other-counters",
        &[
            ("Births suppressed by the population cap", num(c.births_suppressed as f64)),
            ("Trees germinated", num(c.trees_germinated as f64)),
            ("Trees died", num(c.trees_died as f64)),
        ],
    );
    if !frame.info.pathogens.is_empty() {
        heading(ui, "Disease");
        for (p, pathogen) in frame.info.pathogens.iter().enumerate() {
            let (infections, recoveries, deaths) = c.disease.get(p).copied().unwrap_or_default();
            ui.label(egui::RichText::new(&pathogen.name).strong());
            note(ui, &format!("hosts: {}", pathogen.hosts.join(", ")));
            grid(
                ui,
                &format!("eco-disease-{p}"),
                &[
                    ("Infections", num(infections as f64)),
                    ("Recoveries", num(recoveries as f64)),
                    ("Deaths", num(deaths as f64)),
                ],
            );
            let lines: Vec<Series> = [
                (format!("infected.{}", pathogen.id), "infectious", palette.error),
                (format!("recovered.{}", pathogen.id), "recovered (immune)", palette.success),
            ]
            .into_iter()
            .filter_map(|(k, l, col)| series(history, frame, &k).map(|s| (l.to_string(), s, col)))
            .collect();
            if !lines.is_empty() {
                time_plot(ui, axis(frame), &format!("eco-sir-{p}"), lines, 130.0, "animals", false);
            }
        }
    }
    heading(ui, "Recent events");
    let tpd = f64::from(frame.info.ticks_per_day.max(1));
    for e in frame.events.iter().take(20) {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("day {:.2}", e.tick as f64 / tpd)).monospace().weak());
            ui.label(&e.message);
        });
    }
}

/// An imposed weather spell in words: its state, the days it still lasts
/// and, for storms, the rain multiplier.
pub fn imposed_label(f: &physics_world::weather::ForcedWeather, tick: u64, ticks_per_day: u32) -> String {
    let days = f.until_tick.saturating_sub(tick) as f64 / f64::from(ticks_per_day.max(1));
    let storm = if f.kind == physics_world::weather::WeatherKind::Storm {
        format!(", rain × {:.1}", 3.0 * f.intensity)
    } else {
        String::new()
    };
    format!("{} for {days:.1} more days{storm}", f.kind.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use physics_world::weather::{ForcedWeather, WeatherKind};

    #[test]
    fn imposed_spells_read_as_words() {
        let drought = ForcedWeather { kind: WeatherKind::Drought, until_tick: 5_280, intensity: 1.0 };
        assert_eq!(imposed_label(&drought, 5_040, 240), "Drought for 1.0 more days");
        let storm = ForcedWeather { kind: WeatherKind::Storm, until_tick: 100, intensity: 2.0 };
        assert_eq!(imposed_label(&storm, 400, 240), "Storm for 0.0 more days, rain × 6.0");
    }
}
