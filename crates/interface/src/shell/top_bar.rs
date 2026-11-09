//! Top bar: home, mode title, scenario picker, save / load / export,
//! fullscreen, tutorial, help, theme and credits.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align, Layout, PopupCloseBehavior, RichText, Ui};

use super::Frame;
use crate::commands::ShellCommand;
use crate::error::InterfaceError;
use crate::export::{
    FrameRecorder, MAX_SEQUENCE_FRAMES, MetricsFormat, ScreenshotTracker, export_base_name, export_metrics,
};
use crate::format::UtcDateTime;
use crate::icons::{Icon, icon_button, paint_logo};
use crate::jobs::{BackgroundJobs, JobKind};
use crate::mode::{AppMode, ModeRegistry, ModeScenarios, ScenarioSelected};
use crate::saves::{DataLocation, LoadDialog, PendingSave, SaveDialog, save_in_progress};
use crate::session::MetricsHistory;
use crate::state::ShellState;
use crate::theme::{ThemeKind, ThemeSettings, mix};
use crate::toasts::Toasts;
use crate::tutorial::TutorialRegistry;

pub(super) fn show(root: &mut Ui, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    let p = frame.palette;
    egui::Panel::top("shell_top_bar")
        .exact_size(48.0)
        .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(12, 0)))
        .show(root, |ui| {
            ui.horizontal_centered(|ui| {
                brand(ui, world, frame);
                if frame.mode.is_simulation() {
                    mode_title(ui, world, frame);
                    scenario_picker(ui, world, frame);
                    if !shell.panels_visible && ui.small_button("Panels hidden — show (Tab)").clicked() {
                        shell.panels_visible = true;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    right_side(ui, world, shell, frame);
                });
            });
        });
}

fn brand(ui: &mut Ui, world: &mut World, frame: &Frame) {
    let p = &frame.palette;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(176.0, 34.0), egui::Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, 6.0, p.surface_hi);
    }
    let logo = egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 7.0), egui::Vec2::splat(20.0));
    let brand_accent = AppMode::Dashboard.default_accent();
    paint_logo(ui.painter(), logo, brand_accent, mix(p.surface_hi, p.text_muted, 0.2));
    ui.painter().text(
        egui::pos2(logo.max.x + 9.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Emergence Lab",
        egui::FontId::proportional(16.5),
        p.text,
    );
    let response = response.on_hover_text("Dashboard (Home)");
    if response.clicked() && frame.mode != AppMode::Dashboard {
        world.write_message(ShellCommand::OpenMode(AppMode::Dashboard));
    }
}

fn mode_title(ui: &mut Ui, world: &mut World, frame: &Frame) {
    let p = &frame.palette;
    ui.label(RichText::new("/").size(18.0).color(p.text_muted));
    let registry = world.resource::<ModeRegistry>();
    let (glyph, title) = registry.card(frame.mode).map_or_else(
        || (frame.mode.default_glyph().to_string(), registry.title(frame.mode)),
        |c| (c.glyph.clone(), c.title.clone()),
    );
    ui.label(RichText::new(glyph).size(16.0).color(p.accent));
    ui.label(RichText::new(title).size(16.0).strong().color(p.text));
    if let Some(descriptor) = frame.session.as_ref().map(|s| s.session().descriptor()) {
        ui.label(RichText::new(&descriptor.name).size(12.5).color(p.text_muted)).on_hover_text(&descriptor.summary);
    }
}

fn scenario_picker(ui: &mut Ui, world: &mut World, frame: &Frame) {
    let Some(list) = world.resource::<ModeScenarios>().get(frame.mode).cloned() else { return };
    if list.entries.is_empty() {
        return;
    }
    ui.add_space(12.0);
    ui.label(RichText::new("Scenario").small().color(frame.palette.text_muted));
    let selected = list
        .selected
        .as_ref()
        .and_then(|id| list.entries.iter().find(|e| &e.id == id))
        .map_or("Choose…", |e| e.title.as_str());
    let mut chosen = None;
    egui::ComboBox::from_id_salt("shell_scenario").selected_text(selected).width(220.0).show_ui(ui, |ui| {
        for entry in &list.entries {
            let on = list.selected.as_deref() == Some(entry.id.as_str());
            let response = ui.selectable_label(on, &entry.title);
            let response =
                if entry.description.is_empty() { response } else { response.on_hover_text(&entry.description) };
            if response.clicked() {
                chosen = Some(entry.id.clone());
            }
        }
    });
    if let Some(id) = chosen {
        world.resource_mut::<ModeScenarios>().mark_selected(frame.mode, &id);
        world.write_message(ScenarioSelected { mode: frame.mode, id });
    }
}

fn right_side(ui: &mut Ui, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    let p = frame.palette;
    // Laid out right to left.
    if icon_button(ui, Icon::Info, "About and credits", true, shell.show_about, &p).clicked() {
        shell.show_about = !shell.show_about;
    }
    theme_menu(ui, world, frame);
    if icon_button(ui, Icon::Help, "Help and keyboard shortcuts (F1)", true, shell.show_help, &p).clicked() {
        shell.show_help = !shell.show_help;
    }
    let has_tutorial =
        frame.mode == AppMode::Dashboard || world.resource::<TutorialRegistry>().get(frame.mode).is_some();
    let tip = if has_tutorial { "Tutorial and concepts" } else { "No tutorial is registered for this mode yet" };
    if icon_button(ui, Icon::Book, tip, has_tutorial, shell.tutorial.open, &p).clicked() {
        shell.tutorial.open = !shell.tutorial.open;
        shell.tutorial.page = 0;
    }
    if icon_button(ui, Icon::Fullscreen, "Fullscreen (F11)", true, false, &p).clicked() {
        world.write_message(ShellCommand::ToggleFullscreen);
    }
    ui.separator();
    export_menu(ui, world, shell, frame);
    let in_mode = frame.mode.is_simulation();
    let has_session = frame.session.is_some();
    let busy = save_in_progress(world.resource::<PendingSave>(), world.resource::<BackgroundJobs>());
    let load_tip = if in_mode { "Load a saved state" } else { "Open a laboratory to load its saved states" };
    if icon_button(ui, Icon::Open, load_tip, in_mode && has_session, shell.load_dialog.is_some(), &p).clicked() {
        shell.load_dialog = match shell.load_dialog {
            Some(_) => None,
            None => Some(LoadDialog::open(world.resource::<DataLocation>(), frame.mode)),
        };
    }
    let save_tip = if busy {
        "Saving…"
    } else if has_session {
        "Save the current state"
    } else {
        "Nothing to save"
    };
    if icon_button(ui, Icon::Save, save_tip, has_session && !busy, shell.save_dialog.is_some(), &p).clicked() {
        shell.save_dialog = match shell.save_dialog {
            Some(_) => None,
            None => {
                let title = world.resource::<ModeRegistry>().title(frame.mode);
                Some(SaveDialog::new(&title, frame.status.as_ref().map_or(0, |s| s.tick)))
            }
        };
    }
    if in_mode {
        ui.separator();
        let has_left = frame.panels.left.is_some();
        if icon_button(
            ui,
            Icon::PanelRight,
            "Show / hide the right panel",
            true,
            shell.right_open && shell.panels_visible,
            &p,
        )
        .clicked()
        {
            shell.right_open = !shell.right_open;
            shell.panels_visible = true;
        }
        let tip = if has_left { "Show / hide the tools panel" } else { "This mode has no tools panel" };
        if icon_button(ui, Icon::PanelLeft, tip, has_left, has_left && shell.left_open && shell.panels_visible, &p)
            .clicked()
        {
            shell.left_open = !shell.left_open;
            shell.panels_visible = true;
        }
    }
}

fn theme_menu(ui: &mut Ui, world: &mut World, frame: &Frame) {
    let response = icon_button(ui, Icon::Theme, "Theme", true, false, &frame.palette);
    egui::Popup::menu(&response).show(|ui| {
        ui.label(RichText::new("Theme").small().weak());
        let mut kind = world.resource::<ThemeSettings>().kind;
        for choice in ThemeKind::ALL {
            ui.radio_value(&mut kind, choice, choice.label());
        }
        if kind != world.resource::<ThemeSettings>().kind {
            world.resource_mut::<ThemeSettings>().kind = kind;
        }
    });
}

fn export_menu(ui: &mut Ui, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    let recording = world.resource::<FrameRecorder>().progress().cloned();
    let response = icon_button(
        ui,
        Icon::Export,
        "Export metrics, screenshots and frame sequences",
        true,
        recording.is_some(),
        &frame.palette,
    );
    egui::Popup::menu(&response).close_behavior(PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(250.0);
        let has_metrics = world.resource::<MetricsHistory>().store().total_samples() > 0;
        let exporting = world.resource::<BackgroundJobs>().is_running(JobKind::MetricsExport);
        ui.label(RichText::new("Metrics").small().weak());
        for (format, label) in [(MetricsFormat::Csv, "Metrics as CSV…"), (MetricsFormat::Json, "Metrics as JSON…")]
        {
            let button = ui.add_enabled(has_metrics && !exporting, egui::Button::new(label));
            let why = if exporting {
                "The previous export is still being written."
            } else {
                "No metric samples recorded yet: run the simulation first."
            };
            let button = button.on_disabled_hover_text(why);
            if button.clicked() {
                export_metrics_now(world, frame.mode, format);
                ui.close();
            }
        }
        ui.separator();
        ui.label(RichText::new("Images").small().weak());
        if ui.button("Screenshot (PNG)   F12").clicked() {
            world.write_message(ShellCommand::Screenshot);
            ui.close();
        }
        match &recording {
            Some(progress) => {
                ui.label(format!("Recording frame {} of {}…", progress.requested, progress.total));
                if ui.button("Stop recording").clicked() {
                    world.resource_mut::<FrameRecorder>().cancel();
                }
            }
            None => {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut shell.sequence_frames)
                            .range(1..=MAX_SEQUENCE_FRAMES)
                            .suffix(" frames"),
                    );
                    if ui
                        .button("Record PNG sequence")
                        .on_hover_text("One PNG per rendered frame, in a new folder")
                        .clicked()
                    {
                        start_sequence(world, frame.mode, shell.sequence_frames);
                        ui.close();
                    }
                });
            }
        }
        let writing = world.resource::<ScreenshotTracker>().in_flight();
        if writing > 0 {
            ui.label(RichText::new(format!("Writing {writing} image(s)…")).small().weak());
        }
        if let Ok(dirs) = world.resource::<DataLocation>().dirs() {
            ui.separator();
            ui.label(RichText::new(format!("Exports: {}", dirs.exports().display())).small().weak());
            ui.label(RichText::new(format!("Images: {}", dirs.screenshots().display())).small().weak());
        }
    });
}

/// Writes the recorded metrics on a background thread (a copy of the store
/// is taken now, so the export is the state at the moment of the click).
fn export_metrics_now(world: &mut World, mode: AppMode, format: MetricsFormat) {
    let result = world.resource::<DataLocation>().dirs().map(|dirs| dirs.exports()).and_then(|dir| {
        let store = world.resource::<MetricsHistory>().store().clone();
        let base = export_base_name(mode, UtcDateTime::now());
        world.resource_mut::<BackgroundJobs>().spawn(JobKind::MetricsExport, move || {
            export_metrics(&store, &dir, &base, format).map(|path| format!("Metrics exported to {}", path.display()))
        })
    });
    if let Err(e) = result {
        world.resource_mut::<Toasts>().error(format!("Export failed: {e}"));
    }
}

fn start_sequence(world: &mut World, mode: AppMode, frames: u32) {
    let result: Result<(), InterfaceError> = world.resource::<DataLocation>().dirs().cloned().and_then(|dirs| {
        let dir = dirs.screenshots().join(format!("{}-frames", export_base_name(mode, UtcDateTime::now())));
        world.resource_mut::<FrameRecorder>().start(dir, frames)
    });
    if let Err(e) = result {
        world.resource_mut::<Toasts>().error(format!("Cannot record frames: {e}"));
    }
}
