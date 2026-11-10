//! Right panel, "Properties" tab: inspector of the selected person or
//! building, society charts, the trade network, groups and what people
//! are doing.

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, vec2};
use civilization::agent::{CARRY_CAPACITY, TICKS_PER_YEAR};
use civilization::buildings::{BuildingKind, CROP_GROWTH_TICKS};
use civilization::economy::mrs;
use civilization::{Activity, Good, Sex, Work};
use egui_plot::{Line, Plot, PlotPoints};
use interface::theme::{mix, with_alpha};
use interface::{MetricsHistory, ModeRegistry, Palette, ThemeSettings};
use organism_engine::AgentId;

use crate::MODE;
use crate::graph::{ForceLayout, circle_layout, edge_width};
use crate::palette::{activity_color32, activity_title, good_color32, group_color32};
use crate::session::{AgentDetail, BuildingDetail, Frame, Selection};
use crate::state::{CurrentFrame, GraphMode, ViewState};
use crate::ui::icons::badge_widget;
use crate::ui::widgets::{card, clock, diverging_bar, duration, fixed_cell, meter, row, section, segmented};

/// Persistent layout of the people trade graph.
#[derive(Resource, Default)]
pub struct GraphLayout(pub ForceLayout<AgentId>);

/// Request to scroll the Properties tab to a section, by the start of its
/// title (`"skills"`, `"relationships"`, `"trade network"`, …); consumed
/// when the tab is drawn. Used by the standalone example's captures.
#[derive(Resource, Default, Clone, Debug)]
pub struct ScrollTo(pub Option<String>);

/// The "Properties" tab.
#[allow(clippy::too_many_arguments)]
pub fn properties_panel(
    InMut(ui): InMut<egui::Ui>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    current: Res<CurrentFrame>,
    history: Res<MetricsHistory>,
    mut state: ResMut<ViewState>,
    mut layout: ResMut<GraphLayout>,
    mut scroll: ResMut<ScrollTo>,
) {
    let palette = Palette::new(theme.kind, registry.accent(MODE));
    if let Some(target) = scroll.0.take() {
        crate::ui::widgets::request_scroll(ui.ctx(), &target);
    }
    let Some(frame) = current.get() else {
        ui.label(RichText::new("Waiting for the first frame…").weak());
        return;
    };
    match state.selection {
        Selection::Agent(id) => match frame.agent.as_ref().filter(|d| d.id == id) {
            Some(detail) => agent_inspector(ui, &palette, frame, detail, &mut state),
            None => missing(ui, &palette, &mut state, &format!("Person #{} is no longer alive.", id.0)),
        },
        Selection::Building(id) => match frame.building.as_ref().filter(|d| d.building.id == id) {
            Some(detail) => building_inspector(ui, &palette, frame, detail, &mut state),
            None => missing(ui, &palette, &mut state, &format!("Building #{} no longer exists.", id.0)),
        },
        Selection::None => {
            card(&palette).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Nothing selected").strong());
                ui.label(
                    RichText::new(
                        "With the Inspect tool (1), click a person to see needs, skills, inventory, preferences, \
                         relationships, goal and recent actions, or a building to see its stores and crops.",
                    )
                    .size(12.5)
                    .color(palette.text_muted),
                );
            });
        }
    }
    society_charts(ui, &palette, &history);
    trade_network(ui, &palette, frame, &mut state, &mut layout.0);
    groups_table(ui, &palette, frame);
    activities(ui, &palette, frame);
}

fn missing(ui: &mut egui::Ui, palette: &Palette, state: &mut ViewState, text: &str) {
    card(palette).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(text).color(palette.text_muted));
        if ui.button("Clear selection").clicked() {
            state.selection = Selection::None;
            state.follow = false;
        }
    });
}

fn group_label(group: Option<u32>) -> String {
    match group {
        Some(g) => format!("Group {g}"),
        None => "No group".into(),
    }
}

fn group_chip(ui: &mut egui::Ui, palette: &Palette, frame: &Frame, group: Option<u32>) {
    let colour = group.and_then(|g| frame.view.groups.iter().find(|x| x.id.0 == g)).map(|g| g.color);
    interface::widgets::badge(ui, &group_label(group), readable(group_color32(colour), palette));
}

/// A group colour darkened (light theme) or lightened (high contrast) until
/// it reads as text on the panels (contrast ≥ 3:1).
pub fn readable(colour: Color32, palette: &Palette) -> Color32 {
    let target = if palette.dark { Color32::WHITE } else { Color32::BLACK };
    let mut c = colour;
    for _ in 0..12 {
        if interface::theme::contrast_ratio(c, palette.panel) >= 3.0 {
            break;
        }
        c = mix(c, target, 0.12);
    }
    c
}

fn agent_inspector(ui: &mut egui::Ui, palette: &Palette, frame: &Frame, d: &AgentDetail, state: &mut ViewState) {
    let a = &d.agent;
    let tick = frame.view.tick;
    ui.horizontal(|ui| {
        badge_widget(ui, a.doing, 30.0).on_hover_text(activity_title(a.doing));
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Person #{}", d.id.0)).strong().size(16.0));
                group_chip(ui, palette, frame, a.group.map(|g| g.0));
            });
            let population = frame.info.populations.get(usize::from(a.population)).map_or("?", |s| s.as_str());
            ui.label(
                RichText::new(format!(
                    "{} · {:.0} years (lifespan {:.0}) · {} · {}",
                    if a.sex == Sex::Female { "Female" } else { "Male" },
                    a.age as f32 / TICKS_PER_YEAR as f32,
                    a.max_age as f32 / TICKS_PER_YEAR as f32,
                    if a.generation == 0 { "founder".to_string() } else { format!("generation {}", a.generation) },
                    population,
                ))
                .size(12.0)
                .color(palette.text_muted),
            );
        });
    });
    ui.horizontal(|ui| {
        let mut follow = state.follow;
        if ui.toggle_value(&mut follow, "Follow").clicked() {
            state.follow = follow;
        }
        if ui.button("Deselect").clicked() {
            state.selection = Selection::None;
            state.follow = false;
        }
    });

    section(ui, palette, "Needs");
    let pct = |v: f32| format!("{:.0} %", 100.0 * v.clamp(0.0, 1.0));
    let n = &a.needs;
    meter(ui, palette, "Hunger", a.hunger(), Color32::from_rgb(232, 128, 72), &pct(a.hunger()))
        .on_hover_text("1 − energy. Food eaten restores 0.25 energy per unit.");
    meter(ui, palette, "Thirst", n.water.value, Color32::from_rgb(70, 150, 232), &pct(n.water.value));
    meter(ui, palette, "Fatigue", n.rest.value, Color32::from_rgb(150, 120, 214), &pct(n.rest.value));
    meter(ui, palette, "Loneliness", n.social.value, Color32::from_rgb(80, 190, 168), &pct(n.social.value));
    meter(ui, palette, "Danger felt", n.safety, Color32::from_rgb(232, 84, 84), &pct(n.safety));
    meter(ui, palette, "Health", a.health, palette.success, &pct(a.health));

    section(ui, palette, "Goal");
    ui.horizontal(|ui| {
        badge_widget(ui, a.goal.activity, 18.0);
        ui.label(RichText::new(activity_title(a.goal.activity)).strong());
        if a.goal.activity != Activity::Idle {
            ui.label(RichText::new(format!("· {}", d.target)).color(palette.text_muted));
        }
    });
    egui::Grid::new("civ_goal").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        if let Some(plan) = a.goal.plan.or_else(|| a.building_plan()) {
            row(ui, palette, "Building plan", plan.label());
        }
        if !a.path.is_empty() {
            row(ui, palette, "Route", format!("{} waypoints left", a.path.len()));
        }
        let next = a.next_decision.saturating_sub(tick);
        row(ui, palette, "Next decision", format!("{} (in {})", clock(a.next_decision), duration(next)));
    });
    ui.label(
        RichText::new(
            "Chosen by the utility AI: every feasible action is scored from the needs, holdings, skills and \
             distances; the best one (±5 % noise, +15 % for the current goal) wins.",
        )
        .size(11.5)
        .color(palette.text_muted),
    );

    section(ui, palette, "Inventory");
    for good in Good::ALL {
        let amount = a.inventory.get(good);
        let max = if good == Good::Tools { 3.0 } else { CARRY_CAPACITY };
        meter(
            ui,
            palette,
            &interface::format::title_case(good.label()),
            amount / max,
            good_color32(good),
            &format!("{amount:.1}"),
        );
    }
    egui::Grid::new("civ_inventory").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        // Gathering stops at the carry limit; barter and harvests can exceed it.
        row(ui, palette, "Load", format!("{:.1} units (gathering limit {CARRY_CAPACITY:.0})", a.load()));
        if a.home.is_some() {
            row(ui, palette, "Hut store", format!("{:.1} food", d.household_food));
        }
        row(ui, palette, "Food reserve kept", format!("{:.1}", a.prefs.food_reserve()));
        let sale = a.sale_targets();
        let kept: Vec<String> = Good::ALL
            .iter()
            .filter(|g| sale.get(**g) >= 0.05)
            .map(|g| format!("{:.1} {}", sale.get(*g), g.label()))
            .collect();
        row(ui, palette, "Kept for sale", if kept.is_empty() { "nothing yet".to_string() } else { kept.join(", ") });
    });

    section(ui, palette, "Exchange rates (MRS)");
    let weights = a.demand_weights();
    egui::Grid::new("civ_mrs").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        for other in [Good::Wood, Good::Stone, Good::Tools] {
            let text = match mrs(&a.inventory, &weights, Good::Food, other) {
                Some(rate) => format!("gives up to {rate:.2} {} for 1 food", other.label()),
                None => "no value".into(),
            };
            row(ui, palette, &format!("Food for {}", other.label()), text);
        }
    });
    ui.label(
        RichText::new(
            "MRS = (m_food/(h_food+1)) / (m_good/(h_good+1)) from this person's Cobb–Douglas demand weights m and \
             holdings h. Two people barter when their rates differ, at the geometric mean of the two.",
        )
        .size(11.5)
        .color(palette.text_muted),
    );

    section(ui, palette, "Skills");
    for (skill, value) in civilization::Skill::ALL.iter().map(|s| (*s, a.skills.get(*s))) {
        meter(
            ui,
            palette,
            &interface::format::title_case(skill.label()),
            value,
            palette.accent,
            &format!("{value:.2}"),
        );
    }
    let spec = d.specialization.map_or("—".to_string(), |s| format!("{s:.2}"));
    ui.label(
        RichText::new(format!("Specialisation index {spec}  (0 generalist … 1 specialist)"))
            .size(12.0)
            .color(palette.text_muted),
    );
    production_mix(ui, palette, &a.practice);

    section(ui, palette, "Preferences");
    let p = &a.prefs;
    let pref_colour = mix(palette.accent, palette.text_muted, 0.35);
    meter(ui, palette, "Risk tolerance", p.risk, pref_colour, &format!("{:.2}", p.risk));
    meter(ui, palette, "Cooperation", p.cooperation, pref_colour, &format!("{:.2}", p.cooperation));
    meter(ui, palette, "Trade propensity", p.trade, pref_colour, &format!("{:.2}", p.trade));
    meter(ui, palette, "Impatience", p.time, pref_colour, &format!("{:.2}", p.time)).on_hover_text(
        "Time preference: impatient people discount farming, tools and buildings and keep smaller reserves.",
    );

    section(ui, palette, &format!("Relationships ({})", d.relations.len()));
    if d.relations.is_empty() {
        ui.label(RichText::new("Knows nobody yet.").weak());
    }
    for r in d.relations.iter().take(10) {
        ui.horizontal(|ui| {
            let c = group_color32(r.group.and_then(|g| frame.view.groups.iter().find(|x| x.id == g)).map(|g| g.color));
            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, if r.alive { c } else { with_alpha(c, 0.3) });
            let label = format!("#{}{}", r.other.0, if r.alive { "" } else { " (dead)" });
            fixed_cell(ui, 82.0, egui::Align::Min, |ui| {
                let link = ui.add_enabled(r.alive, egui::Link::new(RichText::new(label).size(12.5)));
                if link.clicked() {
                    state.selection = Selection::Agent(r.other);
                    state.follow = false;
                }
            });
            diverging_bar(ui, palette, r.affinity, 90.0);
            ui.label(RichText::new(format!("{:+.2}", r.affinity)).monospace().size(11.5).color(palette.text_muted));
        });
    }

    section(ui, palette, "Recent actions");
    if d.recent.is_empty() {
        ui.label(RichText::new("No activity recorded since the view started observing.").weak());
    }
    for (t, activity) in d.recent.iter().take(8) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(clock(*t)).monospace().size(11.5).color(palette.text_muted));
            badge_widget(ui, *activity, 14.0);
            ui.label(RichText::new(activity_title(*activity)).size(12.5));
        });
    }
    let known: Vec<&str> = [
        (a.memory.water, "water"),
        (a.memory.food, "food"),
        (a.memory.wood, "wood"),
        (a.memory.stone, "stone"),
        (a.memory.market, "a market place"),
    ]
    .iter()
    .filter(|(t, _)| t.is_some())
    .map(|(_, l)| *l)
    .collect();
    ui.label(
        RichText::new(if known.is_empty() {
            "Remembers no places yet.".to_string()
        } else {
            format!("Remembers where to find {}.", known.join(", "))
        })
        .size(12.0)
        .color(palette.text_muted),
    );
}

const WORK_COLOURS: [Color32; Work::COUNT] = [
    Color32::from_rgb(206, 72, 112),
    Color32::from_rgb(160, 110, 62),
    Color32::from_rgb(150, 156, 166),
    Color32::from_rgb(204, 168, 48),
    Color32::from_rgb(84, 140, 204),
    Color32::from_rgb(214, 134, 60),
];

fn production_mix(ui: &mut egui::Ui, palette: &Palette, practice: &[f32; Work::COUNT]) {
    let total: f32 = practice.iter().map(|v| v.max(0.0)).sum();
    if total <= 0.0 {
        return;
    }
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    let mut x = rect.min.x;
    let mut tip = Vec::new();
    for (k, work) in Work::ALL.iter().enumerate() {
        let share = practice[k].max(0.0) / total;
        let w = rect.width() * share;
        if w > 0.0 {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(egui::pos2(x, rect.min.y), vec2(w, rect.height())),
                0.0,
                WORK_COLOURS[k],
            );
            tip.push(format!("{} {:.0} %", work.label(), share * 100.0));
        }
        x += w;
    }
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
    response.on_hover_text(format!("Production mix (moving average): {}", tip.join(", ")));
}

fn building_inspector(ui: &mut egui::Ui, palette: &Palette, frame: &Frame, d: &BuildingDetail, state: &mut ViewState) {
    let b = &d.building;
    ui.horizontal(|ui| {
        badge_widget(ui, if b.kind == BuildingKind::FarmPlot { Activity::Farm } else { Activity::Build }, 30.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} #{}", interface::format::title_case(b.kind.label()), b.id.0))
                        .strong()
                        .size(16.0),
                );
                group_chip(ui, palette, frame, b.group.map(|g| g.0));
            });
            ui.label(
                RichText::new(match b.kind {
                    BuildingKind::Hut => "Home: faster rest, a feeling of safety and a household food store.",
                    BuildingKind::Storage => "Group food store: members deposit surplus and eat from it.",
                    BuildingKind::FarmPlot => "Farm plot: plant, wait for the crop to grow, harvest.",
                })
                .size(12.0)
                .color(palette.text_muted),
            );
        });
    });
    if ui.button("Deselect").clicked() {
        state.selection = Selection::None;
    }
    section(ui, palette, "State");
    egui::Grid::new("civ_building").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        row(ui, palette, "Tile", format!("({}, {})", d.tile.0, d.tile.1));
        match b.owner {
            Some(owner) => {
                ui.label(RichText::new("Owner").size(12.5).color(palette.text_muted));
                if d.owner_alive {
                    if ui.link(format!("Person #{}", owner.0)).clicked() {
                        state.selection = Selection::Agent(owner);
                    }
                } else {
                    ui.label(format!("#{} (dead)", owner.0));
                }
                ui.end_row();
            }
            None => row(ui, palette, "Owner", "none (decaying when without group)"),
        }
        let progress =
            if b.progress >= 1.0 { "complete".to_string() } else { format!("{:.0} % built", 100.0 * b.progress) };
        row(ui, palette, "Construction", progress);
        row(ui, palette, "Condition", format!("{:.0} %", 100.0 * b.condition));
        if b.kind != BuildingKind::FarmPlot {
            let stored: Vec<String> = Good::ALL
                .iter()
                .filter(|g| b.store.get(**g) >= 0.05)
                .map(|g| format!("{:.1} {}", b.store.get(*g), g.label()))
                .collect();
            row(ui, palette, "Stored", if stored.is_empty() { "empty".to_string() } else { stored.join(", ") });
        }
        if b.kind == BuildingKind::FarmPlot {
            match b.crop {
                Some(crop) => {
                    row(ui, palette, "Crop growth", format!("{:.0} %", 100.0 * crop.growth.min(1.0)));
                    if crop.is_ripe() {
                        row(ui, palette, "Harvest left", format!("{:.1} food", crop.yield_left));
                        row(ui, palette, "Ripe for", duration(u64::from(crop.ripe_ticks)));
                    } else {
                        row(ui, palette, "Quality", format!("{:.2} (0.5 + planter's farming skill)", crop.quality));
                    }
                }
                None => row(ui, palette, "Crop", "not planted"),
            }
        }
    });
    if b.kind == BuildingKind::FarmPlot {
        ui.label(
            RichText::new(format!(
                "Crops grow fertility × crop multiplier / {CROP_GROWTH_TICKS:.0} per tick and rot 120 ticks after ripening."
            ))
            .size(11.5)
            .color(palette.text_muted),
        );
    }
}

/// Small time-series card of one metric.
fn spark(
    ui: &mut egui::Ui,
    palette: &Palette,
    history: &MetricsHistory,
    key: &str,
    title: &str,
    colour: Color32,
    digits: usize,
) {
    let latest = history.latest(key);
    card(palette).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(11.5).color(palette.text_muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = latest.map_or("—".to_string(), |v| format!("{v:.digits$}"));
                ui.label(RichText::new(text).size(13.0).strong().color(colour));
            });
        });
        let points: Vec<[f64; 2]> = history
            .store()
            .display(key)
            .map(|pts| pts.iter().map(|p| [p.tick as f64, p.value]).collect())
            .unwrap_or_default();
        Plot::new(("civ_spark", key))
            .height(52.0)
            .show_axes([false, false])
            .show_grid([false, false])
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .show_x(false)
            .show_y(false)
            .include_y(0.0)
            .set_margin_fraction(vec2(0.0, 0.08))
            .show_background(false)
            .show(ui, |plot| {
                if points.len() > 1 {
                    plot.line(
                        Line::new(title, PlotPoints::from(points)).color(colour).width(1.6).fill(0.0).fill_alpha(0.12),
                    );
                }
            });
    });
}

fn society_charts(ui: &mut egui::Ui, palette: &Palette, history: &MetricsHistory) {
    section(ui, palette, "Society");
    let charts = [
        ("population", "Population", 0),
        ("groups", "Groups", 0),
        ("gini_wealth", "Wealth Gini", 3),
        ("trade_volume", "Trade volume / tick", 1),
        ("specialization", "Specialisation", 3),
        ("territory_per_group", "Territory per group", 0),
    ];
    let width = ui.available_width();
    let columns = if width > 330.0 { 2 } else { 1 };
    egui::Grid::new("civ_sparks").num_columns(columns).spacing([6.0, 6.0]).show(ui, |ui| {
        for (i, (key, title, digits)) in charts.iter().enumerate() {
            ui.vertical(|ui| {
                ui.set_width((width - 6.0 * (columns as f32 - 1.0)) / columns as f32 - 2.0);
                spark(ui, palette, history, key, title, palette.series_color(i), *digits);
            });
            if (i + 1) % columns == 0 {
                ui.end_row();
            }
        }
    });
    ui.label(
        RichText::new("Full plots, histograms and every other metric are in the Metrics tab.")
            .size(11.5)
            .color(palette.text_muted),
    );
}

const WINDOWS: [(u64, &str); 4] = [(24, "1 day"), (240, "10 days"), (720, "30 days"), (2400, "100 days")];

fn trade_network(
    ui: &mut egui::Ui,
    palette: &Palette,
    frame: &Frame,
    state: &mut ViewState,
    layout: &mut ForceLayout<AgentId>,
) {
    section(ui, palette, "Trade network");
    ui.horizontal(|ui| {
        let mut mode = state.graph;
        segmented(ui, palette, &mut mode, &[(GraphMode::Groups, "Groups"), (GraphMode::People, "People")]);
        state.graph = mode;
        let label = WINDOWS.iter().find(|(w, _)| *w == state.trade_window).map_or("custom", |(_, l)| *l);
        egui::ComboBox::from_id_salt("civ_trade_window").selected_text(format!("last {label}")).width(110.0).show_ui(
            ui,
            |ui| {
                for (w, l) in WINDOWS {
                    ui.selectable_value(&mut state.trade_window, w, format!("last {l}"));
                }
            },
        );
    });
    let t = &frame.trade;
    let size = vec2(ui.available_width(), 230.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, palette.surface);
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
    let hover = response.hover_pos();
    let mut tooltip: Option<String> = None;
    match state.graph {
        GraphMode::Groups => {
            let mut ids: Vec<u32> = frame.view.groups.iter().map(|g| g.id.0).collect();
            for (a, b, _) in &t.group_edges {
                for g in [*a, *b] {
                    if !ids.contains(&g) {
                        ids.push(g);
                    }
                }
            }
            ids.sort_unstable();
            if ids.is_empty() {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No trade yet",
                    egui::FontId::proportional(13.0),
                    palette.text_muted,
                );
            } else {
                let colour = |g: u32| group_color32(frame.view.groups.iter().find(|x| x.id.0 == g).map(|x| x.color));
                let members = |g: u32| -> u32 {
                    if g == 0 {
                        frame.view.agents.iter().filter(|a| a.group.is_none()).count() as u32
                    } else {
                        frame.view.groups.iter().find(|x| x.id.0 == g).map_or(0, |x| x.members)
                    }
                };
                let max = t.group_edges.iter().map(|e| e.2).max().unwrap_or(0);
                let node_radius = |g: u32| (6.0 + 2.2 * (members(g) as f32).sqrt()).min(22.0);
                // Trade inside a group: a ring around its node.
                let ring = |g: u32| {
                    t.group_edges.iter().find(|e| e.0 == g && e.1 == g).map(|e| {
                        (node_radius(g) + 3.0 + 0.5 * edge_width(e.2, max, 1.0, 9.0), edge_width(e.2, max, 1.0, 9.0))
                    })
                };
                let outer = |g: u32| ring(g).map_or(node_radius(g), |(radius, w)| radius + 0.5 * w);
                // Room for the largest node, its ring and a label outside it.
                let reach = ids.iter().map(|g| outer(*g)).fold(0.0f32, f32::max) + 20.0;
                let centre = rect.center();
                let radius = (rect.height().min(rect.width()) * 0.5 - reach).max(24.0);
                let pts = circle_layout(ids.len(), bevy::math::Vec2::new(centre.x, centre.y), radius);
                let pos = |g: u32| ids.iter().position(|x| *x == g).map(|i| egui::pos2(pts[i].x, pts[i].y));
                for (a, b, n) in &t.group_edges {
                    let (Some(pa), Some(pb)) = (pos(*a), pos(*b)) else { continue };
                    if a == b {
                        if let Some((radius, w)) = ring(*a) {
                            painter.circle_stroke(pa, radius, Stroke::new(w, with_alpha(colour(*a), 0.45)));
                        }
                    } else {
                        let c = with_alpha(mix(colour(*a), colour(*b), 0.5), 0.75);
                        painter.line_segment([pa, pb], Stroke::new(edge_width(*n, max, 1.0, 9.0), c));
                    }
                }
                for g in &ids {
                    let Some(p) = pos(*g) else { continue };
                    let r = node_radius(*g);
                    painter.circle(p, r, colour(*g), Stroke::new(1.5, palette.panel));
                    let label = if *g == 0 { "no group".to_string() } else { format!("G{g}") };
                    // Labels sit outside the circle, away from the links.
                    let galley = painter.layout_no_wrap(label, egui::FontId::proportional(11.0), palette.text);
                    let at = label_position(p, centre, outer(*g), galley.size());
                    painter.galley(at - galley.size() * 0.5, galley, palette.text);
                    if hover.is_some_and(|h| h.distance(p) <= r + 2.0) {
                        let inside: u32 = t.group_edges.iter().filter(|e| e.0 == *g && e.1 == *g).map(|e| e.2).sum();
                        let outside: u32 =
                            t.group_edges.iter().filter(|e| (e.0 == *g) != (e.1 == *g)).map(|e| e.2).sum();
                        tooltip = Some(format!(
                            "{}: {}\n{inside} meetings inside, {outside} with others",
                            group_label((*g != 0).then_some(*g)),
                            crate::ui::widgets::count(members(*g) as usize, "person", "people")
                        ));
                    }
                }
            }
        }
        GraphMode::People => {
            let mut edges: Vec<(AgentId, AgentId, u32)> = t.agent_edges.clone();
            edges.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
            edges.truncate(160);
            let mut nodes: Vec<AgentId> = edges.iter().flat_map(|e| [e.0, e.1]).collect();
            nodes.sort_unstable();
            nodes.dedup();
            if nodes.is_empty() {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No trade yet",
                    egui::FontId::proportional(13.0),
                    palette.text_muted,
                );
                layout.step(&[], &[], 0, |_| bevy::math::Vec2::ZERO);
            } else {
                let max = edges.iter().map(|e| e.2).max().unwrap_or(1);
                let weighted: Vec<(AgentId, AgentId, f32)> =
                    edges.iter().map(|e| (e.0, e.1, 0.5 + e.2 as f32 / max as f32)).collect();
                layout.step(&nodes, &weighted, 3, |id| {
                    let h = crate::coords::tile_hash(id.0 as u32, (id.0 >> 32) as u32, 5);
                    let k = crate::coords::tile_hash(id.0 as u32, 3, 6);
                    bevy::math::Vec2::new(h, k)
                });
                let inner = rect.shrink(14.0);
                // The force layout keeps a compact graph near the centre of
                // its unit square: zoom it to use the canvas.
                let fit = fit_unit_layout(nodes.iter().filter_map(|id| layout.position(id)));
                let at = |id: AgentId| {
                    layout.position(&id).map(|p| {
                        let q = fit(p);
                        egui::pos2(inner.min.x + q.x * inner.width(), inner.min.y + q.y * inner.height())
                    })
                };
                let group_of = |id: AgentId| frame.view.agents.iter().find(|a| a.id == id).and_then(|a| a.group);
                let colour = |id: AgentId| {
                    group_color32(
                        group_of(id).and_then(|g| frame.view.groups.iter().find(|x| x.id == g)).map(|g| g.color),
                    )
                };
                for (a, b, n) in &edges {
                    if let (Some(pa), Some(pb)) = (at(*a), at(*b)) {
                        painter.line_segment(
                            [pa, pb],
                            Stroke::new(edge_width(*n, max, 0.8, 4.0), with_alpha(palette.text_muted, 0.45)),
                        );
                    }
                }
                let mut clicked = None;
                for id in &nodes {
                    let Some(p) = at(*id) else { continue };
                    let degree = edges.iter().filter(|e| e.0 == *id || e.1 == *id).count();
                    let r = 3.0 + 1.3 * (degree as f32).sqrt();
                    let selected = state.selection == Selection::Agent(*id);
                    painter.circle(
                        p,
                        r,
                        colour(*id),
                        Stroke::new(
                            if selected { 2.5 } else { 1.0 },
                            if selected { palette.accent } else { palette.panel },
                        ),
                    );
                    if hover.is_some_and(|h| h.distance(p) <= r + 2.0) {
                        let meetings: u32 = edges.iter().filter(|e| e.0 == *id || e.1 == *id).map(|e| e.2).sum();
                        tooltip = Some(format!(
                            "Person #{} ({})\n{degree} partners, {meetings} meetings\nClick to inspect",
                            id.0,
                            group_label(group_of(*id).map(|g| g.0))
                        ));
                        if response.clicked() {
                            clicked = Some(*id);
                        }
                    }
                }
                if let Some(id) = clicked {
                    state.selection = Selection::Agent(id);
                    state.follow = false;
                }
            }
        }
    }
    if let Some(text) = tooltip {
        response.on_hover_text_at_pointer(text);
    }
    let coverage = if t.reported > 0 {
        format!(" ({:.0} %)", 100.0 * t.detected as f64 / t.reported as f64)
    } else {
        String::new()
    };
    ui.label(
        RichText::new(format!(
            "{} of {} barter meetings reported by the model{coverage} attributed to pairs in this window.",
            t.detected, t.reported
        ))
        .size(11.5)
        .color(palette.text_muted),
    )
    .on_hover_text(
        "The model counts successful meetings but does not publish who met whom. The view pairs people from the \
         rise of their learned selling stocks, who initiated a meeting and how the goods flows match; meetings \
         where both partners' sales are hidden at the stock cap can be missed, and two meetings at the same \
         spot in the same hour can be cross-paired.",
    );
}

/// Transform of unit-square layout positions that zooms their bounding box
/// (each axis at most 2.5×) to fill the unit square, centred.
pub fn fit_unit_layout(
    points: impl Iterator<Item = bevy::math::Vec2>,
) -> impl Fn(bevy::math::Vec2) -> bevy::math::Vec2 {
    use bevy::math::Vec2;
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in points {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let (centre, scale) = if lo.x <= hi.x {
        let span = (hi - lo).max(Vec2::splat(1e-3));
        (0.5 * (lo + hi), (Vec2::ONE / span).min(Vec2::splat(2.5)))
    } else {
        (Vec2::splat(0.5), Vec2::ONE)
    };
    move |p: Vec2| (Vec2::splat(0.5) + (p - centre) * scale).clamp(Vec2::ZERO, Vec2::ONE)
}

/// Centre of a node label of `size` placed radially outside a node at
/// `node` (outer radius `outer`) of a circle around `centre`; below the node
/// when it is the centre itself.
pub fn label_position(node: egui::Pos2, centre: egui::Pos2, outer: f32, size: egui::Vec2) -> egui::Pos2 {
    let d = node - centre;
    let dir = if d.length() < 1.0 { vec2(0.0, 1.0) } else { d / d.length() };
    node + dir * (outer + 4.0 + 0.5 * (dir.x.abs() * size.x + dir.y.abs() * size.y))
}

fn groups_table(ui: &mut egui::Ui, palette: &Palette, frame: &Frame) {
    section(ui, palette, &format!("Groups ({})", frame.view.groups.len()));
    if frame.view.groups.is_empty() {
        ui.label(
            RichText::new(
                "No groups yet: a group forms when at least three people are linked by mutual affinity at or above \
                 the bond threshold.",
            )
            .size(12.0)
            .color(palette.text_muted),
        );
        return;
    }
    let mut groups = frame.view.groups.clone();
    groups.sort_by(|a, b| b.members.cmp(&a.members).then(a.id.cmp(&b.id)));
    egui::Grid::new("civ_groups").num_columns(4).striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
        for h in ["Group", "Members", "Territory", "Contested"] {
            ui.label(RichText::new(h).size(11.5).color(palette.text_muted));
        }
        ui.end_row();
        for g in groups.iter().take(12) {
            ui.horizontal(|ui| {
                interface::widgets::swatch(ui, group_color32(Some(g.color)), 11.0);
                ui.label(format!("G{}", g.id.0));
            });
            ui.label(g.members.to_string());
            ui.label(format!("{} tiles", g.territory));
            ui.label(g.contested.to_string());
            ui.end_row();
        }
    });
}

/// Counts of people per current activity, most common first.
pub fn activity_counts(frame: &Frame) -> Vec<(Activity, usize)> {
    let mut counts: Vec<(Activity, usize)> = Activity::ALL
        .iter()
        .map(|a| (*a, frame.view.agents.iter().filter(|x| x.activity == *a).count()))
        .filter(|(_, n)| *n > 0)
        .collect();
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    counts
}

fn activities(ui: &mut egui::Ui, palette: &Palette, frame: &Frame) {
    section(ui, palette, "What people are doing");
    let counts = activity_counts(frame);
    let total = frame.view.agents.len().max(1);
    for (activity, n) in counts {
        ui.horizontal(|ui| {
            badge_widget(ui, activity, 16.0);
            let share = n as f32 / total as f32;
            let label = format!("{} ", activity_title(activity));
            meter(ui, palette, &label, share, activity_color32(activity), &n.to_string());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface::ThemeKind;

    #[test]
    fn compact_graphs_are_zoomed_to_the_canvas() {
        use bevy::math::Vec2;
        let pts = [Vec2::new(0.4, 0.45), Vec2::new(0.6, 0.55), Vec2::new(0.5, 0.5)];
        let fit = fit_unit_layout(pts.into_iter());
        // x span 0.2 -> zoom capped at 2.5 (0.5 of the canvas); y span 0.1 -> 2.5 as well.
        assert!((fit(pts[0]) - Vec2::new(0.25, 0.375)).length() < 1e-5, "{}", fit(pts[0]));
        assert!((fit(pts[2]) - Vec2::splat(0.5)).length() < 1e-5, "centred");
        let wide = [Vec2::new(0.05, 0.3), Vec2::new(0.95, 0.7)];
        let fit = fit_unit_layout(wide.into_iter());
        assert!((fit(wide[0]).x - 0.0).abs() < 1e-5 && (fit(wide[1]).x - 1.0).abs() < 1e-5, "fills the width");
        let single = fit_unit_layout(std::iter::once(Vec2::new(0.3, 0.3)));
        assert!((single(Vec2::new(0.3, 0.3)) - Vec2::splat(0.5)).length() < 1e-5);
        let none = fit_unit_layout(std::iter::empty());
        assert!((none(Vec2::new(0.2, 0.7)) - Vec2::new(0.2, 0.7)).length() < 1e-5, "no nodes: identity");
    }

    #[test]
    fn graph_labels_sit_outside_their_nodes() {
        let centre = egui::pos2(100.0, 100.0);
        let size = vec2(30.0, 12.0);
        // A node at the top of the circle has its label above it, clear of the node.
        let top = label_position(egui::pos2(100.0, 40.0), centre, 10.0, size);
        assert!(top.y + size.y * 0.5 <= 40.0 - 10.0 && (top.x - 100.0).abs() < 1e-4, "{top:?}");
        // On the right, the label is to the right.
        let right = label_position(egui::pos2(160.0, 100.0), centre, 10.0, size);
        assert!(right.x - size.x * 0.5 >= 170.0, "{right:?}");
        // A single node in the centre gets its label below.
        assert!(label_position(centre, centre, 10.0, size).y > 110.0);
    }

    #[test]
    fn group_text_stays_readable_in_every_theme() {
        for kind in ThemeKind::ALL {
            let palette = Palette::new(kind, Color32::from_rgb(229, 192, 123));
            for colour in
                [Color32::from_rgb(200, 235, 80), Color32::from_rgb(40, 40, 160), Color32::from_rgb(150, 156, 168)]
            {
                let c = readable(colour, &palette);
                assert!(interface::theme::contrast_ratio(c, palette.panel) >= 3.0, "{kind:?} {colour:?} -> {c:?}");
            }
        }
    }
}
