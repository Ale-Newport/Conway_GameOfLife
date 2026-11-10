//! Files tab: the seven presets (open and edit), a new empty system, the
//! user library (save / load / delete RON files in the data folder),
//! export (RON or JSON) and import from a path.

use std::path::PathBuf;

use bevy_egui::egui::{self, RichText, Ui};
use interface::Palette;
use interface::format::{UtcDateTime, format_bytes};

use super::widgets::{heading, hint};
use super::{EditorParams, EditorTab};
use crate::files::{self, Format, LibraryEntry};
use crate::session::{open_preset, start_system};

/// State of the Files tab.
#[derive(Clone, Debug)]
pub struct FilesState {
    /// Name to save the draft under.
    pub save_name: String,
    /// Replace an existing file.
    pub overwrite: bool,
    /// Path typed for import.
    pub import_path: String,
    /// Export encoding.
    pub export_format: Format,
    /// Cached library listing.
    pub library: Option<Result<Vec<LibraryEntry>, String>>,
    /// File armed for deletion.
    pub confirm_delete: Option<PathBuf>,
    /// Cached listing of exported systems.
    pub exports: Option<Result<Vec<LibraryEntry>, String>>,
}

impl Default for FilesState {
    fn default() -> Self {
        Self {
            save_name: String::new(),
            overwrite: false,
            import_path: String::new(),
            export_format: Format::Ron,
            library: None,
            confirm_delete: None,
            exports: None,
        }
    }
}

/// Draws the tab.
pub fn show(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    egui::ScrollArea::vertical().id_salt("cl_files_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width() - 8.0);
        presets(ui, p, palette);
        ui.separator();
        library(ui, p, palette);
        ui.separator();
        export_import(ui, p, palette);
    });
}

fn presets(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    heading(ui, "Presets", palette);
    hint(ui, "“Open” restarts the world from the preset; “Edit” only loads it into the draft.", palette);
    let presets = p.presets.presets.clone();
    for s in &presets {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&s.name).strong());
                ui.label(RichText::new(s.tags.join(" · ")).small().color(palette.text_muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("Edit")
                        .on_hover_text("Load into the draft without touching the running world")
                        .clicked()
                    {
                        p.editor.load(s.system.clone(), s.seed, format!("Preset: {}", s.name));
                        p.editor.tab = EditorTab::Entities;
                    }
                    if ui.button("Open").on_hover_text("Restart the world from this preset").clicked()
                        && let Some(runner) = p.runner.as_deref().cloned()
                    {
                        let result = open_preset(
                            &s.id,
                            &p.presets,
                            &runner,
                            &mut p.editor,
                            &mut p.data,
                            &mut p.ui,
                            &mut p.history,
                            &mut p.list,
                        );
                        match result {
                            Ok(()) => p.toasts.success(format!("Opened the preset “{}”.", s.name)),
                            Err(e) => p.toasts.error(e),
                        }
                    }
                });
            });
            ui.label(RichText::new(&s.expected).small().color(palette.text_muted));
        });
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("New empty system").on_hover_text("Start a draft from scratch (48 × 48 torus)").clicked() {
            p.editor.load(crate::draft::blank_system("My life system"), 1, "New system");
            p.editor.tab = EditorTab::World;
        }
        let can_run = !p.editor.has_errors() && p.runner.is_some();
        if ui
            .add_enabled(can_run, egui::Button::new("Run draft from scratch"))
            .on_hover_text("Restart the world from the draft (also discards undo history and plots)")
            .clicked()
            && let Some(runner) = p.runner.as_deref().cloned()
        {
            let (draft, seed) = (p.editor.draft.clone(), p.editor.seed);
            match start_system(&runner, draft, seed, &mut p.data, &mut p.ui, &mut p.history, &mut p.list) {
                Ok(()) => p.toasts.success("The world restarted from the draft."),
                Err(e) => p.toasts.error(e),
            }
        }
    });
}

fn library(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    heading(ui, "My systems", palette);
    let dirs = match p.location.dirs() {
        Ok(d) => d.clone(),
        Err(e) => {
            ui.label(RichText::new(format!("The data folder is unavailable: {e}")).color(palette.error));
            return;
        }
    };
    let dir = files::library_dir(&dirs);
    hint(ui, &format!("Saved as RON in {}", dir.display()), palette);
    if p.editor.files.save_name.is_empty() {
        p.editor.files.save_name = p.editor.draft.name.clone();
    }
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut p.editor.files.save_name).desired_width(200.0).hint_text("file name"));
        ui.checkbox(&mut p.editor.files.overwrite, "Overwrite");
        if ui.button("Save draft").clicked() {
            let name = p.editor.files.save_name.clone();
            match files::save_to_library(&dir, &name, &p.editor.draft, p.editor.files.overwrite) {
                Ok(path) => {
                    p.toasts.success(format!("Saved {}", path.display()));
                    p.editor.files.library = None;
                    p.editor.source = format!("File: {}", files::sanitize_name(&name));
                }
                Err(e) => p.toasts.error(e),
            }
        }
        if ui.button("Refresh").clicked() {
            p.editor.files.library = None;
            p.editor.files.exports = None;
        }
    });
    if p.editor.has_errors() {
        hint(ui, "The draft has errors; it can still be saved and fixed later.", palette);
    }
    if p.editor.files.library.is_none() {
        p.editor.files.library = Some(files::list_library(&dir));
    }
    let listing = p.editor.files.library.clone().unwrap_or_else(|| Ok(Vec::new()));
    match listing {
        Err(e) => {
            ui.label(RichText::new(e).color(palette.error));
        }
        Ok(entries) if entries.is_empty() => hint(ui, "No saved systems yet.", palette),
        Ok(entries) => {
            for entry in entries {
                let when = entry.modified.map(|t| UtcDateTime::from_system_time(t).iso8601()).unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&entry.name).strong());
                        ui.label(RichText::new(format!("{when} · {}", format_bytes(entry.bytes))).small().weak());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let armed = p.editor.files.confirm_delete.as_ref() == Some(&entry.path);
                        if armed {
                            if ui.button("Cancel").clicked() {
                                p.editor.files.confirm_delete = None;
                            }
                            if ui.button(RichText::new("Confirm delete").color(palette.error)).clicked() {
                                match std::fs::remove_file(&entry.path) {
                                    Ok(()) => p.toasts.info(format!("Deleted {}", entry.name)),
                                    Err(e) => p.toasts.error(format!("Could not delete {}: {e}", entry.path.display())),
                                }
                                p.editor.files.confirm_delete = None;
                                p.editor.files.library = None;
                            }
                        } else if ui.button("Delete…").clicked() {
                            p.editor.files.confirm_delete = Some(entry.path.clone());
                        }
                        if ui.button("Load").on_hover_text("Load into the draft").clicked() {
                            match files::read_file(&entry.path) {
                                Ok(loaded) => {
                                    let seed = loaded.seed.unwrap_or(p.editor.seed);
                                    p.editor.load(loaded.system, seed, format!("File: {}", entry.name));
                                    p.editor.files.save_name = entry.name.clone();
                                    p.toasts.info(format!("Loaded {} into the draft; apply it to run it.", entry.name));
                                }
                                Err(e) => p.toasts.error(format!("{}: {e}", entry.path.display())),
                            }
                        }
                    });
                });
                ui.separator();
            }
        }
    }
}

fn export_import(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    heading(ui, "Export", palette);
    let dirs = p.location.dirs().ok().cloned();
    ui.horizontal(|ui| {
        ui.radio_value(&mut p.editor.files.export_format, Format::Ron, "RON");
        ui.radio_value(&mut p.editor.files.export_format, Format::Json, "JSON");
        let enabled = dirs.is_some();
        if ui.add_enabled(enabled, egui::Button::new("Export draft")).clicked()
            && let Some(dirs) = &dirs
        {
            let stamp = UtcDateTime::now().file_stamp();
            match files::export(&dirs.exports(), &p.editor.draft, p.editor.files.export_format, &stamp) {
                Ok(path) => {
                    p.toasts.success(format!("Exported {}", path.display()));
                    p.editor.files.exports = None;
                }
                Err(e) => p.toasts.error(e),
            }
        }
    });
    if let Some(dirs) = &dirs {
        hint(ui, &format!("Written to {}", dirs.exports().display()), palette);
    }
    heading(ui, "Import", palette);
    hint(ui, "A LifeSystem document (RON or JSON) or a preset file (RON with a header).", palette);
    if let Some(dirs) = &dirs {
        if p.editor.files.exports.is_none() {
            p.editor.files.exports = Some(files::list_exports(&dirs.exports()));
        }
        if let Some(Ok(entries)) = p.editor.files.exports.clone()
            && !entries.is_empty()
        {
            egui::CollapsingHeader::new(RichText::new(format!("Exported systems ({})", entries.len())).small())
                .id_salt("cl_exports_list")
                .show(ui, |ui| {
                    for entry in entries.iter().take(20) {
                        ui.horizontal(|ui| {
                            let file = entry.path.file_name().map(|n| n.to_string_lossy().into_owned());
                            ui.label(RichText::new(file.unwrap_or_default()).small());
                            if ui.small_button("Import").clicked() {
                                import(p, &entry.path);
                            }
                        });
                    }
                });
        }
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut p.editor.files.import_path)
                .desired_width(320.0)
                .hint_text("/path/to/system.ron"),
        );
        let path = PathBuf::from(p.editor.files.import_path.trim());
        if ui.add_enabled(!p.editor.files.import_path.trim().is_empty(), egui::Button::new("Import")).clicked() {
            import(p, &path);
        }
    });
}

/// Reads a document into the draft (not applied).
fn import(p: &mut EditorParams, path: &std::path::Path) {
    match files::read_file(path) {
        Ok(loaded) => {
            let seed = loaded.seed.unwrap_or(p.editor.seed);
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            p.editor.load(loaded.system, seed, format!("Imported: {name}"));
            p.toasts.success(format!("Imported {} into the draft; apply it to run it.", path.display()));
        }
        Err(e) => p.toasts.error(format!("{}: {e}", path.display())),
    }
}
