//! Layout of a simulation mode: toolbar, bottom bar, left tools panel,
//! right tabbed panel, overlay; returns the rectangle left for the world.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, RichText, Ui};
use rendering::WorldCamera;

use super::{Frame, bottom_bar, run_panel};
use crate::commands::RightPanelTab;
use crate::session::MetricsHistory;
use crate::state::ShellState;
use crate::theme::with_alpha;
use crate::widgets::rules::tick_label;
use crate::widgets::{metrics_panel, param_editor, rules_panel};

fn panel_frame(frame: &Frame, margin: i8) -> egui::Frame {
    egui::Frame::new().fill(frame.palette.panel).inner_margin(egui::Margin::same(margin))
}

pub(super) fn show_mode_layout(root: &mut Ui, world: &mut World, shell: &mut ShellState, frame: &Frame) -> egui::Rect {
    if shell.panels_visible {
        if let Some(toolbar) = frame.panels.toolbar {
            egui::Panel::top("shell_mode_toolbar")
                .frame(panel_frame(frame, 6).inner_margin(egui::Margin::symmetric(12, 6)))
                .show(root, |ui| {
                    ui.horizontal(|ui| run_panel(world, toolbar, ui));
                });
        }
        egui::Panel::bottom("shell_bottom_bar")
            .frame(panel_frame(frame, 0).inner_margin(egui::Margin::symmetric(12, 6)))
            .show(root, |ui| bottom_bar::show(ui, world, shell, frame));
        if let Some(left) = frame.panels.left {
            egui::Panel::left("shell_left_panel")
                .resizable(true)
                .default_size(290.0)
                .size_range(220.0..=520.0)
                .frame(panel_frame(frame, 12))
                .show_collapsible(root, &mut shell.left_open, |ui| {
                    section_title(ui, frame, "Tools");
                    egui::ScrollArea::vertical().id_salt("shell_left_scroll").auto_shrink([false, false]).show(
                        ui,
                        |ui| {
                            run_panel(world, left, ui);
                        },
                    );
                });
        }
        let mut right_open = shell.right_open;
        egui::Panel::right("shell_right_panel")
            .resizable(true)
            .default_size(360.0)
            .size_range(280.0..=640.0)
            .frame(panel_frame(frame, 12))
            .show_collapsible(root, &mut right_open, |ui| right_panel(ui, world, shell, frame));
        shell.right_open = right_open;
    }
    let central = root.available_rect_before_wrap();
    let world_visible = world.query_filtered::<&Camera, With<WorldCamera>>().iter(world).any(|camera| camera.is_active);
    if !world_visible {
        // Nothing renders there yet: keep the area clean.
        root.painter().rect_filled(central, 0.0, frame.palette.background);
    }
    if let Some(overlay) = frame.panels.overlay {
        egui::Area::new(egui::Id::new("shell_mode_overlay"))
            .order(egui::Order::Middle)
            .fixed_pos(central.min + egui::vec2(12.0, 12.0))
            .constrain_to(central)
            .show(root.ctx(), |ui| {
                ui.set_max_size(central.size() - egui::vec2(24.0, 24.0));
                ui.set_clip_rect(central);
                run_panel(world, overlay, ui);
            });
    }
    if frame.session.is_none() {
        root.painter().text(
            central.center(),
            Align2::CENTER_CENTER,
            "Starting the simulation…",
            egui::FontId::proportional(16.0),
            with_alpha(frame.palette.text_muted, 0.8),
        );
    }
    central
}

fn section_title(ui: &mut Ui, frame: &Frame, title: &str) {
    ui.label(RichText::new(title.to_uppercase()).size(11.5).strong().color(frame.palette.text_muted));
    ui.add_space(4.0);
}

fn right_panel(ui: &mut Ui, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    let mut tabs = Vec::with_capacity(4);
    if frame.panels.right.is_some() {
        tabs.push(RightPanelTab::Properties);
    }
    tabs.extend([RightPanelTab::Parameters, RightPanelTab::Rules, RightPanelTab::Metrics]);
    if !tabs.contains(&shell.right_tab) {
        shell.right_tab = tabs[0];
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for tab in &tabs {
            let on = shell.right_tab == *tab;
            let text = RichText::new(tab.label()).size(13.5).color(if on {
                frame.palette.text
            } else {
                frame.palette.text_muted
            });
            let response = ui.selectable_label(on, text);
            if on {
                let r = response.rect;
                ui.painter().line_segment(
                    [egui::pos2(r.min.x + 6.0, r.max.y + 2.0), egui::pos2(r.max.x - 6.0, r.max.y + 2.0)],
                    egui::Stroke::new(2.0, frame.palette.accent),
                );
            }
            if response.clicked() {
                shell.right_tab = *tab;
            }
        }
    });
    ui.add_space(4.0);
    ui.separator();
    egui::ScrollArea::vertical().id_salt(("shell_right_scroll", shell.right_tab)).auto_shrink([false, false]).show(
        ui,
        |ui| match shell.right_tab {
            RightPanelTab::Properties => {
                if let Some(system) = frame.panels.right {
                    run_panel(world, system, ui);
                }
            }
            RightPanelTab::Parameters => match (&frame.session, &frame.status) {
                (Some(session), Some(status)) => {
                    param_editor(ui, session.session().descriptor(), &status.params, session.session())
                }
                _ => no_session(ui),
            },
            RightPanelTab::Rules => match &frame.session {
                Some(session) => rules_panel(ui, session.session().descriptor(), &frame.palette),
                None => no_session(ui),
            },
            RightPanelTab::Metrics => match &frame.session {
                Some(session) => {
                    let descriptor = session.session().descriptor();
                    let history = world.resource::<MetricsHistory>();
                    if history.awaiting_restore() {
                        ui.label(RichText::new("Waiting for the loaded state; plots restart from it.").small().weak());
                    }
                    metrics_panel(
                        ui,
                        history.store(),
                        &descriptor.metrics,
                        &tick_label(descriptor),
                        &frame.palette,
                        &mut shell.metrics,
                    );
                }
                None => no_session(ui),
            },
        },
    );
}

fn no_session(ui: &mut Ui) {
    ui.label(RichText::new("No simulation is running in this mode.").weak());
}
