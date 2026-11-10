//! Right "Properties" panel: organism inspector, gene charts, genealogy
//! tree and regions (distance matrix, F_ST, speciation criterion).

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText};
use egui_plot::{Bar, BarChart, FilledArea, Legend, Line, LineStyle, Plot, PlotPoints, VLine};
use evolution::genes::{ACCLIMATION, EXPERIENCE, GENES, HERITABLE_COUNT, TEMP_OPTIMUM};
use evolution::{Activity, EvolutionView, SpeciationStatus, Species};
use interface::theme::with_alpha;
use interface::widgets::metrics::{PlotWindow, plot_points};
use interface::widgets::{badge, gradient_bar};
use interface::{MetricsHistory, ModeRegistry, Palette, SessionStatus, ThemeSettings};
use organism_engine::AgentId;
use rendering::Colormap;

use crate::genealogy::{TreeAction, TreeCache, tree_canvas};
use crate::history::{response_in_sd, thin_band};
use crate::morph::thermal_t;
use crate::session::{EvoData, OrganismDetail, express};
use crate::tools_panel::palette;
use crate::ui_state::{EvoUi, MatrixMeasure, PropertiesTab};
use crate::widgets::{
    card, color32, colormap, matrix_heatmap, note, region_matrix, section, segmented, thermal_color32, value_bar,
};

const PLOT_HEIGHT: f32 = 150.0;

/// Right panel system.
#[allow(clippy::too_many_arguments)]
pub fn properties_panel(
    InMut(ui): InMut<egui::Ui>,
    mut state: ResMut<EvoUi>,
    data: Res<EvoData>,
    history: Res<MetricsHistory>,
    status: Res<SessionStatus>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    mut tree_cache: Local<TreeCache>,
) {
    let palette = palette(&theme, &registry);
    state.properties_frame = state.frame;
    ui.add_space(2.0);
    let mut tab = state.tab;
    let options: Vec<(PropertiesTab, &str)> = PropertiesTab::ALL.iter().map(|t| (*t, t.label())).collect();
    segmented(ui, &palette, &mut tab, &options);
    state.tab = tab;
    ui.add_space(4.0);
    let Some(view) = data.view.clone() else {
        note(ui, &palette, "Waiting for the first view of the experiment…");
        return;
    };
    match state.tab {
        PropertiesTab::Inspector => inspector(ui, &palette, &mut state, &data, &view),
        PropertiesTab::Genes => genes(ui, &palette, &mut state, &data, &view, &history),
        PropertiesTab::Genealogy => genealogy(ui, &palette, &mut state, &data, &mut tree_cache),
        PropertiesTab::Regions => regions(ui, &palette, &mut state, &view, &status),
    }
}

fn activity_label(a: Activity) -> &'static str {
    match a {
        Activity::Wander => "wandering",
        Activity::Forage => "foraging",
        Activity::Graze => "grazing",
        Activity::Flee => "fleeing a predator",
        Activity::Hunt => "hunting",
        Activity::Court => "courting",
        Activity::Rest => "resting",
        Activity::Contest => "contesting a rival",
    }
}

fn species_color(palette: &Palette, s: Species) -> Color32 {
    match s {
        Species::Prey => palette.accent,
        Species::Predator => palette.warning,
    }
}

fn fmt_gene(index: usize, v: f32) -> String {
    let g = &GENES[index];
    let digits = if (g.max - g.min) >= 20.0 {
        1
    } else if (g.max - g.min) >= 2.0 {
        2
    } else {
        3
    };
    if g.unit.is_empty() || g.unit == "fraction" { format!("{v:.digits$}") } else { format!("{v:.digits$} {}", g.unit) }
}

// ------------------------------------------------------------------ inspector

fn inspector(ui: &mut egui::Ui, palette: &Palette, state: &mut EvoUi, data: &EvoData, view: &EvolutionView) {
    let Some(selected) = state.selected else {
        card(palette).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("No organism selected").strong());
            note(
                ui,
                palette,
                "With the Select tool (key 1), click a creature in the world. Its body shows its genes: size sets the scale, \
                 speed the leg length and posture, thermal optimum the coat colour, perception the ear and eye size.",
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Pick a random prey").clicked() {
                    state.selected = random_organism(view, Species::Prey, data.view_seq);
                }
                if view.stats.predators.count > 0 && ui.button("Pick a predator").clicked() {
                    state.selected = random_organism(view, Species::Predator, data.view_seq);
                }
            });
        });
        return;
    };
    let Some(detail) = data.detail.as_ref().filter(|d| d.id == selected) else {
        let alive = view.organisms.binary_search_by_key(&selected, |o| o.id).is_ok();
        if alive {
            note(ui, palette, "Reading the organism…");
        } else {
            card(palette).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("{selected} is no longer alive")).strong());
                note(ui, palette, "It died or was removed. Its lineage may live on: see the Genealogy tab.");
                if ui.button("Clear selection").clicked() {
                    state.selected = None;
                }
            });
        }
        return;
    };
    let mean = |g: usize| -> Option<f32> {
        let stats = match detail.species {
            Species::Prey => &view.stats.prey,
            Species::Predator => &view.stats.predators,
        };
        stats.genes.get(g).map(|s| s.mean as f32)
    };

    // Header.
    ui.horizontal(|ui| {
        badge(ui, &detail.species.label().to_uppercase(), species_color(palette, detail.species));
        ui.label(RichText::new(format!("{}", detail.id)).strong().size(15.0));
        ui.label(RichText::new(format!("generation {}", detail.lineage.generation)).color(palette.text_muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("×").on_hover_text("Clear selection (Esc)").clicked() {
                state.selected = None;
                state.follow = false;
            }
            ui.toggle_value(&mut state.follow, "Follow").on_hover_text("Camera follows this organism (F)");
        });
    });
    let stage = if detail.age < detail.maturity { "juvenile" } else { "adult" };
    note(
        ui,
        palette,
        &format!(
            "{} · {stage} · {}",
            activity_label(detail.activity),
            if detail.ready { "ready to breed" } else { "not ready to breed" }
        ),
    );

    // Heritable genome.
    section(ui, palette, "Heritable genome — passed to offspring");
    note(
        ui,
        palette,
        "Copied (and recombined) at birth, then each gene may mutate. Each bar spans the gene's range (hover it \
         for the limits); the tick marks the population mean.",
    );
    egui::Grid::new("evo_genome_grid").num_columns(3).spacing([8.0, 3.0]).show(ui, |ui| {
        for (g, info) in GENES.iter().enumerate().take(HERITABLE_COUNT) {
            ui.label(RichText::new(info.label).size(12.0)).on_hover_text(info.description);
            let t = detail.normalised[g];
            let reference = mean(g).map(|m| (m - info.min) / (info.max - info.min));
            let color =
                if g == TEMP_OPTIMUM { thermal_color32(thermal_t(detail.expressed[g])) } else { palette.accent };
            value_bar(ui, palette, t, reference, color, 92.0).on_hover_text(format!(
                "range {} … {}",
                fmt_gene(g, info.min),
                fmt_gene(g, info.max)
            ));
            ui.label(RichText::new(fmt_gene(g, detail.expressed[g])).size(12.0).monospace());
            ui.end_row();
        }
    });
    if ui
        .button("Copy genome to the Spawn tool")
        .on_hover_text("Set the custom genome to this organism's genes and pick the Spawn tool (key 4)")
        .clicked()
    {
        state.genome.values = std::array::from_fn(|g| detail.expressed[g]);
        state.genome.species = detail.species;
        state.tool = crate::ui_state::Tool::Spawn;
    }

    // Acquired traits.
    section(ui, palette, "Acquired during life — not inherited");
    note(
        ui,
        palette,
        "Stored in the genome object but declared non-heritable: offspring start at 0 and they never mutate.",
    );
    egui::Grid::new("evo_acquired_grid").num_columns(3).spacing([8.0, 3.0]).show(ui, |ui| {
        for g in [ACCLIMATION, EXPERIENCE] {
            let info = &GENES[g];
            ui.label(RichText::new(info.label).size(12.0)).on_hover_text(info.description);
            value_bar(ui, palette, detail.normalised[g], None, palette.success, 92.0);
            ui.label(RichText::new(fmt_gene(g, detail.expressed[g])).size(12.0).monospace());
            ui.end_row();
        }
    });

    // Individual state.
    section(ui, palette, "Individual state — not inherited");
    egui::Grid::new("evo_state_grid").num_columns(2).spacing([10.0, 3.0]).striped(true).show(ui, |ui| {
        ui.label("Energy");
        ui.horizontal(|ui| {
            let t = detail.energy / detail.storage.max(1e-3);
            value_bar(ui, palette, t, None, palette.warning, 80.0);
            ui.label(RichText::new(format!("{:.1} / {:.1}", detail.energy, detail.storage)).monospace().size(12.0));
        });
        ui.end_row();
        ui.label("Health");
        ui.horizontal(|ui| {
            value_bar(ui, palette, detail.health, None, palette.success, 80.0);
            ui.label(RichText::new(format!("{:.2}", detail.health)).monospace().size(12.0));
        });
        ui.end_row();
        ui.label("Age");
        ui.label(format!("{} ticks (matures at {})", detail.age, detail.maturity));
        ui.end_row();
        ui.label("Offspring");
        ui.label(format!("{}", detail.offspring));
        ui.end_row();
        ui.label("Refractory");
        ui.label(format!("{} ticks", detail.refractory));
        ui.end_row();
        if detail.species == Species::Predator {
            ui.label("Handling");
            ui.label(format!("{} ticks", detail.handling));
            ui.end_row();
        }
        ui.label("Local temperature");
        ui.label(format!("{:.1} °C (mismatch {:+.2} σ)", detail.temperature, detail.mismatch))
            .on_hover_text("(T − optimum − acclimation) / tolerance; |m| > 1 damages health");
        ui.end_row();
        ui.label("Speed now");
        ui.label(format!("{:.2} units/tick", detail.speed));
        ui.end_row();
        ui.label("Senescence");
        ui.label(format!("{:.3} % per tick", detail.senescence * 100.0)).on_hover_text("Gompertz hazard at this age");
        ui.end_row();
        ui.label("Position");
        ui.label(format!("({:.1}, {:.1})", detail.position.x, detail.position.y));
        ui.end_row();
    });

    section(ui, palette, "Energy budget per tick");
    budget(ui, palette, detail);

    section(ui, palette, "Lineage");
    lineage(ui, palette, state, detail, view);
}

fn budget(ui: &mut egui::Ui, palette: &Palette, d: &OrganismDetail) {
    let b = &d.budget;
    let total = b.basal + b.movement + b.upkeep + b.perception;
    let parts = [
        ("basal", b.basal, palette.series_color(1)),
        ("movement", b.movement, palette.series_color(2)),
        ("speed upkeep", b.upkeep, palette.series_color(3)),
        ("perception", b.perception, palette.series_color(4)),
    ];
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(320.0), 12.0), egui::Sense::hover());
    let mut x = rect.left();
    for (_, v, c) in parts {
        let w = rect.width() * (v / total.max(1e-6));
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(w, rect.height())),
            0.0,
            c,
        );
        x += w;
    }
    ui.horizontal_wrapped(|ui| {
        for (label, v, c) in parts {
            ui.colored_label(c, "■");
            ui.label(RichText::new(format!("{label} {v:.3}")).size(11.0));
        }
    });
    note(
        ui,
        palette,
        &format!(
            "Spends {total:.3} per tick; can take in at most {:.2} per tick when food is available.",
            b.intake_capacity
        ),
    );
}

fn lineage(ui: &mut egui::Ui, palette: &Palette, state: &mut EvoUi, d: &OrganismDetail, view: &EvolutionView) {
    let alive = |id: AgentId| view.organisms.binary_search_by_key(&id, |o| o.id).is_ok();
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Founder {}", d.founder));
        note(ui, palette, &format!("· {} living in this founder's line", d.lineage_living));
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Parents:");
        let parents: Vec<AgentId> = d.lineage.parent_ids().collect();
        if parents.is_empty() {
            note(ui, palette, "none (a founder or a spawned organism)");
        }
        for p in parents {
            if alive(p) {
                if ui.link(format!("{p}")).on_hover_text("alive — click to inspect").clicked() {
                    state.selected = Some(p);
                }
            } else {
                ui.label(RichText::new(format!("{p} †")).color(palette.text_muted));
            }
        }
    });
    if !d.ancestors.is_empty() {
        ui.label(RichText::new("Line of first parents (newest first):").size(11.5));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for (i, a) in d.ancestors.iter().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("‹").color(palette.text_muted));
                }
                let text = format!("{} g{}", a.id, a.generation);
                let hover = match a.death_tick {
                    Some(t) => format!("born {} · died {}", a.birth_tick, t),
                    None => format!("born {} · alive", a.birth_tick),
                };
                if a.death_tick.is_none() {
                    if ui.link(text).on_hover_text(hover).clicked() {
                        state.selected = Some(a.id);
                    }
                } else {
                    ui.label(RichText::new(text).size(11.0).color(palette.text_muted)).on_hover_text(hover);
                }
            }
        });
        note(ui, palette, "Older ancestors without living descendants elsewhere are pruned by the model.");
    }
    if ui.button("Show in genealogy (G)").clicked() {
        state.tab = PropertiesTab::Genealogy;
    }
}

fn random_organism(view: &EvolutionView, species: Species, salt: u64) -> Option<AgentId> {
    let candidates: Vec<AgentId> = view.organisms.iter().filter(|o| o.species == species).map(|o| o.id).collect();
    if candidates.is_empty() {
        return None;
    }
    let i = (salt.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 33) as usize % candidates.len();
    Some(candidates[i])
}

// ------------------------------------------------------------------ genes

fn genes(
    ui: &mut egui::Ui,
    palette: &Palette,
    state: &mut EvoUi,
    data: &EvoData,
    view: &EvolutionView,
    metrics: &MetricsHistory,
) {
    ui.horizontal(|ui| {
        segmented(ui, palette, &mut state.chart_species, &[(Species::Prey, "Prey"), (Species::Predator, "Predators")]);
        egui::ComboBox::from_id_salt("evo_chart_gene")
            .selected_text(GENES[state.chart_gene].label)
            .width(150.0)
            .show_ui(ui, |ui| {
                for (g, info) in GENES.iter().enumerate().take(HERITABLE_COUNT) {
                    ui.selectable_value(&mut state.chart_gene, g, info.label);
                }
            });
    });
    let species = state.chart_species;
    let gene = state.chart_gene.min(HERITABLE_COUNT - 1);
    let info = &GENES[gene];
    let stats = match species {
        Species::Prey => &view.stats.prey,
        Species::Predator => &view.stats.predators,
    };
    if stats.count == 0 {
        note(ui, palette, &format!("No living {}.", if species == Species::Prey { "prey" } else { "predators" }));
        return;
    }
    note(ui, palette, info.description);

    // Overview table: response since the founders.
    section(ui, palette, "All genes — change since the founders");
    let founders = data.history.first_of(species).map(|s| s.of(species).clone());
    let toggle = if state.genes_table_open { "⏷ hide table" } else { "⏵ show table (click a gene to chart it)" };
    if ui
        .add(egui::Label::new(RichText::new(toggle).size(11.0).color(palette.accent)).sense(egui::Sense::click()))
        .clicked()
    {
        state.genes_table_open = !state.genes_table_open;
    }
    if state.genes_table_open {
        egui::Grid::new("evo_gene_table").num_columns(4).spacing([8.0, 2.0]).striped(true).show(ui, |ui| {
            ui.label(RichText::new("gene").size(11.0).color(palette.text_muted));
            ui.label(RichText::new("mean ± sd").size(11.0).color(palette.text_muted));
            ui.label(RichText::new("Δ (founder sd)").size(11.0).color(palette.text_muted));
            ui.label("");
            ui.end_row();
            for (g, info) in GENES.iter().enumerate().take(HERITABLE_COUNT) {
                let Some(s) = stats.genes.get(g) else { continue };
                let selected = g == gene;
                let label = RichText::new(info.label).size(11.5);
                if ui.selectable_label(selected, label).clicked() {
                    state.chart_gene = g;
                }
                ui.label(RichText::new(format!("{:.3} ± {:.3}", s.mean, s.sd)).monospace().size(11.0));
                let response = founders.as_ref().and_then(|f| response_in_sd(f.mean[g], f.sd[g], s.mean as f32));
                match response {
                    Some(r) => {
                        let color = if r.abs() < 0.5 { palette.text_muted } else { palette.accent };
                        let arrow = if r > 0.05 {
                            "⏶"
                        } else if r < -0.05 {
                            "⏷"
                        } else {
                            "·"
                        };
                        ui.label(RichText::new(format!("{arrow} {r:+.2}")).size(11.0).color(color));
                    }
                    None => {
                        ui.label(RichText::new("—").color(palette.text_muted));
                    }
                }
                sparkline(ui, palette, &s.histogram, selected);
                ui.end_row();
            }
        });
    }

    // Mean ± SD over time.
    section(ui, palette, &format!("{} — mean ± sd over time", info.label));
    // Prey: the shell records the gene means and SDs every tick; predators
    // only have the samples of the published views.
    let band = match species {
        Species::Prey => {
            let store = metrics.store();
            match (store.get(&format!("mean_{}", info.name)), store.get(&format!("sd_{}", info.name))) {
                (Some(mean), Some(sd)) => {
                    let points = mean.points();
                    thin_band(points.len(), |i| (points[i].tick, points[i].value), |t| sd.value_at(t), 700)
                }
                _ => Vec::new(),
            }
        }
        Species::Predator => data.history.band(species, gene),
    };
    let xs: Vec<f64> = band.iter().map(|b| b.tick).collect();
    let lows: Vec<f64> = band.iter().map(|b| b.low).collect();
    let highs: Vec<f64> = band.iter().map(|b| b.high).collect();
    let means: Vec<[f64; 2]> = band.iter().map(|b| [b.tick, b.mean]).collect();
    Plot::new("evo_band")
        .height(PLOT_HEIGHT)
        .allow_scroll(false)
        .allow_zoom(false)
        .allow_drag(false)
        .x_axis_label("tick")
        .y_axis_label(info.unit)
        .show(ui, |plot| {
            if xs.len() >= 2 {
                plot.add(FilledArea::new("± sd", &xs, &lows, &highs).fill_color(with_alpha(palette.accent, 0.22)));
            }
            plot.line(Line::new("mean", PlotPoints::from(means)).color(palette.accent).width(2.0));
            if let Some(f) = &founders {
                plot.add(
                    egui_plot::HLine::new("founders", f64::from(f.mean[gene]))
                        .color(palette.text_muted)
                        .style(LineStyle::dashed_dense()),
                );
            }
        });
    note(
        ui,
        palette,
        "Dashed: founders' mean. Selection and drift move the mean and narrow the band; mutation restores the spread.",
    );

    // Distribution now.
    section(ui, palette, "Distribution now");
    let bins = stats.genes.get(gene).map(|s| s.histogram.clone()).unwrap_or_default();
    let width = f64::from(info.max - info.min) / bins.len().max(1) as f64;
    let bars: Vec<Bar> = bins
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let centre = f64::from(info.min) + (i as f64 + 0.5) * width;
            let fill = if gene == TEMP_OPTIMUM {
                thermal_color32(thermal_t(centre as f32))
            } else {
                with_alpha(palette.accent, 0.8)
            };
            Bar::new(centre, f64::from(c)).width(width * 0.92).fill(fill)
        })
        .collect();
    let current_mean = stats.genes.get(gene).map_or(0.0, |s| s.mean);
    Plot::new("evo_hist")
        .height(PLOT_HEIGHT * 0.85)
        .allow_scroll(false)
        .allow_zoom(false)
        .allow_drag(false)
        .x_axis_label(info.unit)
        .y_axis_label("organisms")
        .include_x(f64::from(info.min))
        .include_x(f64::from(info.max))
        .include_y(0.0)
        .show(ui, |plot| {
            plot.bar_chart(BarChart::new("count", bars));
            plot.vline(VLine::new("mean", current_mean).color(palette.text).width(1.5));
            if let Some(f) = &founders {
                plot.vline(
                    VLine::new("founders", f64::from(f.mean[gene]))
                        .color(palette.text_muted)
                        .style(LineStyle::dashed_dense()),
                );
            }
        });

    // Allele-class frequencies.
    let (k, ticks, cumulative) = data.history.stacked_classes(species, gene);
    section(ui, palette, &format!("Allele-class frequencies ({k} classes)"));
    note(
        ui,
        palette,
        "The gene's range is cut into equal classes, from low to high values. A class that sweeps to 1 has become fixed; \
         a neutral marker drifts, a selected gene moves consistently.",
    );
    Plot::new("evo_classes")
        .height(PLOT_HEIGHT)
        .allow_scroll(false)
        .allow_zoom(false)
        .allow_drag(false)
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label("tick")
        .y_axis_label("frequency")
        .show(ui, |plot| {
            if ticks.len() >= 2 {
                for c in 0..k {
                    let color = colormap(Colormap::Viridis, (c as f32 + 0.5) / k as f32);
                    let lo = info.min + (info.max - info.min) * c as f32 / k as f32;
                    let hi = info.min + (info.max - info.min) * (c + 1) as f32 / k as f32;
                    plot.add(
                        FilledArea::new(format!("{lo:.2}–{hi:.2}"), &ticks, &cumulative[c], &cumulative[c + 1])
                            .fill_color(with_alpha(color, 0.85)),
                    );
                }
            }
        });
    if k > 0 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for c in 0..k {
                let color = colormap(Colormap::Viridis, (c as f32 + 0.5) / k as f32);
                let lo = info.min + (info.max - info.min) * c as f32 / k as f32;
                let hi = info.min + (info.max - info.min) * (c + 1) as f32 / k as f32;
                ui.label(RichText::new(format!("■ {lo:.2}–{hi:.2}")).size(10.5).color(color));
            }
        });
    }
    if let Some(s) = stats.genes.get(gene) {
        note(ui, palette, &format!("Expected heterozygosity of the classes now: H = {:.3}", s.heterozygosity));
    }

    // Diversity.
    section(ui, palette, "Genetic diversity (prey)");
    let series = [
        ("genetic_diversity", "mean pairwise distance π", palette.accent),
        ("heterozygosity", "heterozygosity H", palette.series_color(3)),
        ("fst", "F_ST", palette.series_color(5)),
    ];
    Plot::new("evo_diversity")
        .height(PLOT_HEIGHT)
        .allow_scroll(false)
        .allow_zoom(false)
        .allow_drag(false)
        .include_y(0.0)
        .x_axis_label("tick")
        .legend(Legend::default().position(egui_plot::Corner::LeftTop))
        .show(ui, |plot| {
            for (key, label, color) in series {
                if let Some(s) = metrics.store().get(key) {
                    plot.line(
                        Line::new(label, PlotPoints::from(plot_points(s, PlotWindow::All, 600)))
                            .color(color)
                            .width(1.5),
                    );
                }
            }
        });
    note(
        ui,
        palette,
        "π: mean RMS distance between two prey genomes (0–1). H: chance that two random prey fall in different classes. \
         F_ST: share of diversity lying between regions.",
    );
}

fn sparkline(ui: &mut egui::Ui, palette: &Palette, histogram: &[u32], highlight: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(54.0, 14.0), egui::Sense::hover());
    let max = histogram.iter().copied().max().unwrap_or(0).max(1) as f32;
    let n = histogram.len().max(1) as f32;
    let color = if highlight { palette.accent } else { with_alpha(palette.text_muted, 0.8) };
    for (i, &c) in histogram.iter().enumerate() {
        let h = rect.height() * c as f32 / max;
        let x0 = rect.left() + rect.width() * i as f32 / n;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x0, rect.bottom() - h),
            egui::pos2(x0 + rect.width() / n - 0.5, rect.bottom()),
        );
        ui.painter().rect_filled(bar, 0.0, color);
    }
}

// ------------------------------------------------------------------ genealogy

fn genealogy(ui: &mut egui::Ui, palette: &Palette, state: &mut EvoUi, data: &EvoData, cache: &mut TreeCache) {
    let Some(g) = data.genealogy.as_ref() else {
        note(ui, palette, "Reading the genealogy…");
        return;
    };
    note(
        ui,
        palette,
        "Living ancestry, traced through first parents. Lines run from birth to death (or now); \
         offspring branch off at birth. Colours mark founder lines. Drag to pan, Ctrl/⌘ + scroll to zoom.",
    );
    ui.horizontal(|ui| {
        let mut filter = state.tree.filter;
        segmented(
            ui,
            palette,
            &mut filter,
            &[(Some(Species::Prey), "Prey"), (Some(Species::Predator), "Predators"), (None, "Both")],
        );
        state.tree.filter = filter;
    });
    let tree = cache.layout(g, state.tree.filter, &state.tree.collapsed);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Fit").clicked() {
            state.tree.fit();
        }
        if ui.button("Fold founders").on_hover_text("Collapse every founder line into one wedge").clicked() {
            state.tree.collapsed =
                tree.nodes.iter().filter(|n| n.parent.is_none() && n.internal).map(|n| n.id).collect();
        }
        if ui.button("Unfold all").clicked() {
            state.tree.collapsed.clear();
        }
        if ui.button("−").clicked() {
            state.tree.zoom_y = (state.tree.zoom_y / 1.5).max(0.5);
        }
        if ui.button("+").clicked() {
            state.tree.zoom_y = (state.tree.zoom_y * 1.5).min(200.0);
        }
    });
    let roots = tree.nodes.iter().filter(|n| n.parent.is_none()).count();
    if tree.nodes.iter().all(|n| n.parent.is_none() && !n.internal) {
        note(ui, palette, "No births yet: every organism is a founder. Run the experiment to grow the tree.");
    }
    let action = tree_canvas(ui, tree, &mut state.tree, state.selected, palette, 380.0);
    if let Some(TreeAction::Select(id)) = action {
        state.selected = Some(id);
    }
    egui::Grid::new("evo_genealogy_stats").num_columns(2).spacing([10.0, 2.0]).show(ui, |ui| {
        ui.label("Read at tick");
        ui.label(format!("{}", g.tick)).on_hover_text("The tree is re-read about twice per second while it is shown");
        ui.end_row();
        ui.label("Founder lines with descendants");
        ui.label(format!("{roots}"));
        ui.end_row();
        ui.label("Living organisms shown");
        ui.label(format!("{}", tree.living));
        ui.end_row();
        ui.label("Recorded nodes");
        ui.label(format!("{}", g.nodes.len()));
        ui.end_row();
        ui.label("Births since start");
        ui.label(format!("{}", g.births));
        ui.end_row();
        ui.label("Pruned (no living descendants)");
        ui.label(format!("{}", g.pruned));
        ui.end_row();
        if g.truncated > 0 {
            ui.label("Truncated by the node cap");
            ui.label(format!("{}", g.truncated));
            ui.end_row();
        }
    });
    if let Some(sel) = state.selected {
        note(ui, palette, &format!("Highlighted: the line of first parents of {sel}."));
    }
}

// ------------------------------------------------------------------ regions

fn regions(ui: &mut egui::Ui, palette: &Palette, state: &mut EvoUi, view: &EvolutionView, status: &SessionStatus) {
    let stats = &view.stats;
    section(ui, palette, "Speciation criterion");
    card(palette).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let theta = status.get().map_or(1.0, |s| s.params.f64_or("mate_distance_limit", 1.0));
        let generations = status.get().map_or(5, |s| s.params.i64_or("speciation_generations", 5));
        match &view.speciation {
            SpeciationStatus::NotApplicable(reason) => {
                ui.label(RichText::new("Not applicable").strong().color(palette.text_muted));
                note(ui, palette, reason);
            }
            SpeciationStatus::Monitoring => {
                ui.label(RichText::new("Monitoring").strong());
                note(ui, palette, "No persistent split into two mutually incompatible clusters.");
            }
            SpeciationStatus::Separated { since_tick, generations: g, .. } => {
                ui.label(RichText::new("Clusters separated").strong().color(palette.warning));
                note(ui, palette, &format!("since tick {since_tick}: {g:.1} of {generations} generations required"));
            }
            SpeciationStatus::Speciated(event) => {
                ui.label(RichText::new("Speciation declared").strong().color(palette.success));
                note(
                    ui,
                    palette,
                    &format!("at tick {} (mean generation {:.1})", event.tick, event.generation),
                );
            }
        }
        if let Some(split) = &view.split {
            note(
                ui,
                palette,
                &format!(
                    "Last 2-means split: {} + {} prey; mean cross-cluster distance {:.3} vs mating limit θ = {theta:.2}",
                    split.sizes[0], split.sizes[1], split.between
                ),
            );
        }
    });

    section(ui, palette, "Differentiation");
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("F_ST = {:.3}", stats.fst)).strong().size(14.0));
        note(ui, palette, &format!("· max net divergence {:.3}", stats.max_net_divergence()));
    });
    note(ui, palette, "F_ST = (H_T − H_S) / H_T over allele classes: 0 = regions alike, 1 = fixed differences.");

    section(ui, palette, "Genetic distance between regions");
    segmented(
        ui,
        palette,
        &mut state.matrix,
        &[(MatrixMeasure::Net, "Net divergence"), (MatrixMeasure::Between, "Mean distance")],
    );
    let matrix = region_matrix(stats, state.matrix);
    if matrix.regions.is_empty() {
        note(ui, palette, "Needs at least two regions with prey (islands or sectors; see the Regions ground tint).");
    } else {
        if let Some((i, j)) = matrix_heatmap(ui, palette, &matrix) {
            let n = matrix.regions.len();
            let text = match matrix.values[i * n + j] {
                Some(v) => format!("R{} – R{}: {v:.4}", matrix.regions[i], matrix.regions[j]),
                None => format!("R{} (diagonal)", matrix.regions[i]),
            };
            ui.label(RichText::new(text).monospace().size(11.5));
        }
        // Colour scale of the cells (same mapping as `matrix_heatmap`).
        let stops: Vec<Color32> = (0..=8).map(|k| colormap(Colormap::Magma, 0.15 + 0.85 * k as f32 / 8.0)).collect();
        let max = if matrix.max > 0.0 { matrix.max } else { 1.0 };
        gradient_bar(ui, &stops, ui.available_width().min(240.0), "0", &format!("{max:.3}"));
        note(
            ui,
            palette,
            match state.matrix {
                MatrixMeasure::Net => {
                    "Net divergence d_AB − (d_A + d_B)/2 (Nei's D_A analogue): distance beyond the variation inside each region."
                }
                MatrixMeasure::Between => "Mean genetic distance between a prey of one region and a prey of the other.",
            },
        );
    }

    section(ui, palette, "Regions");
    egui::Grid::new("evo_regions").num_columns(4).spacing([10.0, 2.0]).striped(true).show(ui, |ui| {
        for h in ["region", "prey", "predators", "mean T_opt"] {
            ui.label(RichText::new(h).size(11.0).color(palette.text_muted));
        }
        ui.end_row();
        for r in &stats.regions {
            ui.label(format!("R{}", r.region));
            ui.label(format!("{}", r.prey));
            ui.label(format!("{}", r.predators));
            match r.mean_genome.get(TEMP_OPTIMUM) {
                Some(t) if r.prey > 0 => {
                    let v = express(TEMP_OPTIMUM, *t as f32);
                    ui.colored_label(color32(crate::morph::thermal_rgb(thermal_t(v))), format!("{v:.1} °C"));
                }
                _ => {
                    ui.label("—");
                }
            }
            ui.end_row();
        }
    });
    if stats.unassigned > 0 {
        note(ui, palette, &format!("{} organisms stand on cells outside every region.", stats.unassigned));
    }
}
