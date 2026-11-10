//! Floating windows: help, about / credits, tutorial, save and load.

use bevy::prelude::*;
use bevy_egui::egui::{self, RichText, Ui};

use super::Frame;
use crate::credits::{Credits, LIBRARIES};
use crate::format::{format_bytes, format_count, pretty_timestamp};
use crate::icons::primary_button;
use crate::input::SHORTCUTS;
use crate::jobs::BackgroundJobs;
use crate::mode::{ActiveSession, AppMode};
use crate::saves::{DataLocation, LoadDialog, PendingSave, SaveEntry, delete_save, load_into_session, start_save};
use crate::session::MetricsHistory;
use crate::state::ShellState;
use crate::toasts::Toasts;
use crate::tutorial::{TutorialAction, TutorialRegistry, TutorialTab, dashboard_tutorial};
use crate::widgets::markdown;

pub(super) fn show(ctx: &egui::Context, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    help_window(ctx, shell);
    about_window(ctx, world, shell);
    tutorial_window(ctx, world, shell, frame);
    save_window(ctx, world, shell, frame);
    load_window(ctx, world, shell, frame);
}

/// A centred, non-collapsible window.
fn window<'a>(ctx: &egui::Context, title: &'a str, width: f32) -> egui::Window<'a> {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(true)
        .default_width(width)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .max_height((ctx.content_rect().height() - 80.0).max(200.0))
        .constrain_to(ctx.content_rect())
}

fn help_window(ctx: &egui::Context, shell: &mut ShellState) {
    let mut open = shell.show_help;
    window(ctx, "Help", 460.0).open(&mut open).vscroll(true).show(ctx, |ui| {
        ui.label(RichText::new("Keyboard shortcuts").strong());
        ui.label(
            RichText::new("Shortcuts are ignored while a text field has keyboard focus (Escape releases it).")
                .small()
                .weak(),
        );
        ui.add_space(4.0);
        egui::Grid::new("help_shortcuts").striped(true).num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
            for shortcut in SHORTCUTS {
                ui.label(RichText::new(shortcut.label).monospace());
                ui.label(shortcut.description);
                ui.end_row();
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Mouse").strong());
        ui.label(
            "Inside the world view each laboratory defines its own tools (see its tutorial). Panels \
             can be resized by dragging their inner edge and collapsed with the buttons in the top bar.",
        );
    });
    shell.show_help = open;
}

fn about_window(ctx: &egui::Context, world: &World, shell: &mut ShellState) {
    let mut open = shell.show_about;
    let credits = world.get_resource::<Credits>();
    window(ctx, "About Emergence Lab", 560.0).open(&mut open).default_height(560.0).show(ctx, |ui| {
        ui.label(RichText::new(format!("Emergence Lab {}", env!("CARGO_PKG_VERSION"))).size(18.0).strong());
        ui.label("A laboratory for artificial life. Code licensed MIT OR Apache-2.0.");
        if let Ok(dirs) = world.resource::<DataLocation>().dirs() {
            ui.label(RichText::new(format!("Data folder: {}", dirs.root().display())).small().weak());
        }
        ui.separator();
        egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
            ui.label(RichText::new("Software").strong());
            egui::Grid::new("about_libraries").striped(true).num_columns(3).show(ui, |ui| {
                for lib in LIBRARIES {
                    ui.label(lib.name);
                    ui.label(RichText::new(lib.license).monospace());
                    ui.hyperlink_to("website", lib.url);
                    ui.end_row();
                }
            });
            let Some(credits) = credits else { return };
            ui.add_space(8.0);
            ui.label(RichText::new("3D models").strong());
            if credits.attributions.is_empty() {
                ui.label(RichText::new("No attribution files were found in the asset folder.").weak());
            }
            for file in &credits.attributions {
                egui::CollapsingHeader::new(&file.title).id_salt(("credits", &file.source)).show(ui, |ui| {
                    ui.label(RichText::new(&file.source).small().weak());
                    ui.label(RichText::new(&file.text).monospace().size(11.5));
                });
            }
            if !credits.pattern_licenses.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Pattern library").strong());
                ui.label(RichText::new("Licence of each catalogue entry (details in each RLE file header):").small());
                for (license, count) in &credits.pattern_licenses {
                    ui.label(format!("• {count} × {license}"));
                }
            }
            if let Some(provenance) = &credits.otca_provenance {
                ui.add_space(8.0);
                egui::CollapsingHeader::new(RichText::new("OTCA metapixel data — provenance and licence").strong())
                    .id_salt("credits_otca")
                    .show(ui, |ui| markdown(ui, provenance));
            }
            for problem in &credits.problems {
                ui.colored_label(ui.visuals().warn_fg_color, problem);
            }
        });
    });
    shell.show_about = open;
}

fn tutorial_window(ctx: &egui::Context, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    if !shell.tutorial.open {
        return;
    }
    let tutorial = if frame.mode == AppMode::Dashboard {
        dashboard_tutorial()
    } else {
        match world.resource::<TutorialRegistry>().get(frame.mode) {
            Some(t) => t.clone(),
            None => {
                shell.tutorial.open = false;
                return;
            }
        }
    };
    let mut open = true;
    let mut action = None;
    let view = &mut shell.tutorial;
    window(ctx, "Tutorial", 520.0).open(&mut open).default_height(420.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut view.tab, TutorialTab::Steps, "Steps");
            let has_concepts = !tutorial.concepts.trim().is_empty();
            ui.add_enabled_ui(has_concepts, |ui| ui.selectable_value(&mut view.tab, TutorialTab::Concepts, "Concepts"));
        });
        ui.separator();
        match view.tab {
            TutorialTab::Steps => {
                let count = tutorial.pages.len();
                if count == 0 {
                    ui.label(RichText::new("This tutorial has no pages.").weak());
                    return;
                }
                view.turn(0, count);
                let page = &tutorial.pages[view.page];
                ui.label(RichText::new(format!("Step {} of {count}", view.page + 1)).small().weak());
                ui.label(RichText::new(&page.title).size(17.0).strong());
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| markdown(ui, &page.body));
                if let Some(step) = &page.action
                    && primary_button(ui, &step.label, &frame.palette).clicked()
                {
                    action = Some(step.id.clone());
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.add_enabled(view.page > 0, egui::Button::new("‹ Previous")).clicked() {
                        view.turn(-1, count);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(view.page + 1 < count, egui::Button::new("Next ›")).clicked() {
                            view.turn(1, count);
                        }
                    });
                });
            }
            TutorialTab::Concepts => {
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| markdown(ui, &tutorial.concepts));
            }
        }
    });
    view.open = open;
    if let Some(id) = action {
        world.write_message(TutorialAction(id));
    }
}

fn save_window(ctx: &egui::Context, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    let Some(dialog) = &mut shell.save_dialog else { return };
    let mut open = true;
    let mut submit = false;
    window(ctx, "Save state", 420.0).open(&mut open).show(ctx, |ui| {
        ui.label("Saves the complete simulation state, including its random generator.");
        ui.add_space(4.0);
        let response =
            ui.add(egui::TextEdit::singleline(&mut dialog.name).hint_text("Name").desired_width(f32::INFINITY));
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
        ui.checkbox(&mut dialog.overwrite, "Overwrite a save with the same name");
        if let Some(error) = &dialog.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Ok(dirs) = world.resource::<DataLocation>().dirs() {
            let folder = crate::saves::mode_save_dir(dirs, frame.mode);
            ui.label(RichText::new(format!("Folder: {}", folder.display())).small().weak());
        }
        ui.horizontal(|ui| {
            if primary_button(ui, "Save", &frame.palette).clicked() {
                submit = true;
            }
        });
    });
    if submit {
        let result = match &frame.session {
            Some(session) => world.resource_scope(|world, mut pending: Mut<PendingSave>| {
                start_save(
                    session.session(),
                    world.resource::<DataLocation>(),
                    frame.mode,
                    &dialog.name,
                    dialog.overwrite,
                    &mut pending,
                    world.resource::<BackgroundJobs>(),
                )
            }),
            None => Err(crate::error::InterfaceError::NoSession),
        };
        match result {
            Ok(()) => open = false,
            Err(e) => dialog.error = Some(e.to_string()),
        }
    }
    if !open {
        shell.save_dialog = None;
    }
}

fn load_window(ctx: &egui::Context, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    let Some(dialog) = &mut shell.load_dialog else { return };
    let mut open = true;
    let mut chosen: Option<SaveEntry> = None;
    let mut delete: Option<SaveEntry> = None;
    let mut refresh = false;
    let model_id = frame.descriptor.as_ref().map(|d| d.id.clone());
    window(ctx, "Load state", 620.0).open(&mut open).default_height(380.0).show(ctx, |ui| {
        match &dialog.entries {
            Err(e) => {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            Ok(entries) if entries.is_empty() => {
                ui.label(RichText::new("No saved states for this laboratory yet. Use Save in the top bar.").weak());
            }
            Ok(entries) => {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    egui::Grid::new("load_saves").striped(true).num_columns(5).spacing([12.0, 6.0]).show(ui, |ui| {
                        for heading in ["Name", "Tick", "Saved", "Size", ""] {
                            ui.label(RichText::new(heading).small().weak());
                        }
                        ui.end_row();
                        for entry in entries {
                            entry_row(
                                ui,
                                entry,
                                model_id.as_deref(),
                                &mut dialog.confirm_delete,
                                &mut chosen,
                                &mut delete,
                            );
                            ui.end_row();
                        }
                    });
                });
            }
        }
        ui.separator();
        if ui.button("Refresh").clicked() {
            refresh = true;
        }
    });
    if let Some(entry) = chosen {
        let result = match world.get_resource::<ActiveSession>().cloned() {
            Some(session) => load_into_session(&entry, &session, &mut world.resource_mut::<MetricsHistory>()),
            None => Err(crate::error::InterfaceError::NoSession),
        };
        let mut toasts = world.resource_mut::<Toasts>();
        match result {
            Ok(message) => {
                toasts.success(message);
                open = false;
            }
            Err(e) => toasts.error(format!("Load failed: {e}")),
        }
    }
    if let Some(entry) = delete {
        match delete_save(&entry) {
            Ok(()) => world.resource_mut::<Toasts>().info(format!("Deleted “{}”.", entry.name)),
            Err(e) => world.resource_mut::<Toasts>().error(e.to_string()),
        }
        refresh = true;
    }
    if !open {
        shell.load_dialog = None;
    } else if refresh {
        shell.load_dialog = Some(LoadDialog::open(world.resource::<DataLocation>(), frame.mode));
    }
}

fn entry_row(
    ui: &mut Ui,
    entry: &SaveEntry,
    model_id: Option<&str>,
    confirm: &mut Option<std::path::PathBuf>,
    chosen: &mut Option<SaveEntry>,
    delete: &mut Option<SaveEntry>,
) {
    ui.label(&entry.name).on_hover_text(entry.path.display().to_string());
    ui.label(entry.tick.map_or_else(|| "—".to_string(), format_count));
    ui.label(pretty_timestamp(&entry.created));
    ui.label(format_bytes(entry.size_bytes));
    ui.horizontal(|ui| {
        let compatible = match (&entry.problem, entry.model_id.as_deref(), model_id) {
            (Some(problem), _, _) => Err(problem.clone()),
            (None, Some(found), Some(expected)) if found != expected => {
                Err(format!("saved by model `{found}`; this laboratory runs `{expected}`"))
            }
            _ => Ok(()),
        };
        match compatible {
            Ok(()) => {
                if ui.button("Load").on_hover_text("Restore this state (pauses; undoable)").clicked() {
                    *chosen = Some(entry.clone());
                }
            }
            Err(why) => {
                ui.add_enabled(false, egui::Button::new("Load")).on_disabled_hover_text(why);
            }
        }
        if confirm.as_ref() == Some(&entry.path) {
            if ui.button(RichText::new("Delete permanently").color(ui.visuals().error_fg_color)).clicked() {
                *delete = Some(entry.clone());
                *confirm = None;
            }
            if ui.button("Keep").clicked() {
                *confirm = None;
            }
        } else if ui.small_button("Delete…").clicked() {
            *confirm = Some(entry.path.clone());
        }
    });
}
