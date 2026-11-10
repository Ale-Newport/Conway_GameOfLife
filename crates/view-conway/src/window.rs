//! The "Patterns" window: the pattern library browser (categories, search,
//! thumbnails, metadata and provenance), RLE import (pasted text or a file,
//! with the reader's line / column errors) and RLE export (the selection or
//! the whole world, to the clipboard or the user's exports folder).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, Ui, vec2};
use bevy_egui::{EguiClipboard, EguiContexts};
use cellular_automata::{CellRect, LifeSim};
use interface::theme::with_alpha;
use interface::widgets::badge;
use interface::{DataLocation, ModeRegistry, Palette, ThemeSettings, Toasts};
use patterns::{LifeRule, Pattern};
use simulation_core::{Control, Simulation};

use crate::commands::{ModeCommand, ModeCommands};
use crate::geometry::thumbnail_rgba;
use crate::labels::thousands;
use crate::library::{
    ConwayLibrary, LibrarySource, category_label, kind_label, measured_facts, normalized_keeping_metadata, period_text,
    speed_text,
};
use crate::session::{ConwayRunner, LiveWorld, LoadRequest, LoadRequests, lock};
use crate::style::ViewSettings;
use crate::tools::{Selection, StampPattern, StampSource, read_selection};
use crate::world::mode_palette;

/// Tabs of the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PatternsTab {
    /// Catalog browser.
    #[default]
    Library,
    /// Text import.
    Import,
    /// RLE export.
    Export,
}

/// What is exported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ExportScope {
    /// Every live cell (absolute coordinates).
    #[default]
    World,
    /// The selection (coordinates relative to its top-left corner).
    Selection,
}

/// Largest number of live cells exported as RLE.
pub const MAX_EXPORT_CELLS: u64 = 1 << 22;
/// Imported text longer than this is not shown in the editor.
pub const MAX_EDITABLE_TEXT: usize = 200_000;

/// A parsed import.
#[derive(Clone, Debug)]
pub struct ImportPreview {
    /// The pattern, normalised.
    pub pattern: Arc<Pattern>,
    /// Detected format.
    pub format: &'static str,
    /// Things the user should know (rule, states).
    pub warnings: Vec<String>,
}

/// Parses pattern text in any format of the `patterns` crate and checks it
/// against Classic Conway (two states, rule B3/S23).
pub fn preview_import(text: &str, file_name: Option<&str>) -> Result<ImportPreview, String> {
    let format = patterns::detect_format(text, file_name).map_err(|e| e.to_string())?;
    let pattern = format.parse(text).map_err(|e| e.to_string())?;
    if pattern.is_empty() {
        return Err("the pattern has no live cell".into());
    }
    let mut warnings = Vec::new();
    if let Some(rule) = pattern.rule.as_deref() {
        match LifeRule::parse_with_grid(rule) {
            Ok((r, _)) if r == LifeRule::conway() => {}
            Ok((r, _)) => warnings.push(format!(
                "The file declares rule {r}. Classic Conway runs B3/S23, so these cells will evolve differently \
                 here; the Cellular Automata Lab runs other Life-like rules."
            )),
            Err(_) => warnings.push(format!(
                "The file declares rule `{rule}`, which is not a two-state Life-like rule; only the cell layout is used."
            )),
        }
    }
    if pattern.max_state() > 1 {
        warnings.push(format!(
            "The pattern has {} states; every non-zero state becomes a live cell.",
            u16::from(pattern.max_state()) + 1
        ));
    }
    Ok(ImportPreview { pattern: Arc::new(normalized_keeping_metadata(&pattern)), format: format.label(), warnings })
}

/// An RLE export.
#[derive(Clone, Debug)]
pub struct ExportText {
    /// The RLE document.
    pub text: String,
    /// Live cells.
    pub cells: u64,
    /// Bounding box.
    pub size: (u64, u64),
    /// What was exported.
    pub scope: ExportScope,
    /// Generation of the export.
    pub generation: u64,
}

/// State of the window (kept across visits to the mode).
#[derive(Resource, Default)]
pub struct PatternsWindow {
    /// Whether the window is shown.
    pub open: bool,
    /// Active tab.
    pub tab: PatternsTab,
    /// Category filter (`None` = all).
    pub category: Option<String>,
    /// Search text.
    pub query: String,
    /// Selected library entry.
    pub selected: Option<String>,
    /// Import text.
    pub import_text: String,
    /// Import file path.
    pub import_path: String,
    /// File name used for format detection (from the last file read).
    pub import_file_name: Option<String>,
    /// Parse result of the import text.
    pub import: Option<Result<ImportPreview, String>>,
    /// Export scope.
    pub export_scope: ExportScope,
    /// Last export.
    pub export: Option<Result<ExportText, String>>,
    /// Where the last export was saved.
    pub saved: Option<PathBuf>,
    /// Generate an export on the next frame (as if "Generate RLE" was pressed).
    pub export_request: bool,
    export_pending: Mutex<Option<Receiver<Result<ExportText, String>>>>,
}

impl PatternsWindow {
    /// Opens the window on `tab`.
    pub fn open(&mut self, tab: PatternsTab) {
        self.open = true;
        self.tab = tab;
    }

    /// Opens on `tab`, or closes when already showing it.
    pub fn toggle(&mut self, tab: PatternsTab) {
        if self.open && self.tab == tab {
            self.open = false;
        } else {
            self.open(tab);
        }
    }

    /// Re-parses the import text.
    pub fn reparse(&mut self) {
        self.import = if self.import_text.trim().is_empty() {
            None
        } else {
            Some(preview_import(&self.import_text, self.import_file_name.as_deref()))
        };
    }

    fn exporting(&self) -> bool {
        lock(&self.export_pending).is_some()
    }
}

/// Builds an export on the simulation thread (cells) and a writer thread (RLE).
fn request_export(
    runner: &ConwayRunner,
    scope: ExportScope,
    selection: Option<CellRect>,
) -> Receiver<Result<ExportText, String>> {
    let (tx, rx) = mpsc::channel();
    runner.send(Control::Query(Box::new(move |sim: &LifeSim| {
        let generation = sim.tick();
        let rect = match (scope, selection) {
            (ExportScope::Selection, Some(rect)) => Some(rect),
            (ExportScope::Selection, None) => {
                let _ = tx.send(Err("there is no selection".into()));
                return;
            }
            (ExportScope::World, _) => None,
        };
        let population = match rect {
            Some(r) => read_selection(sim, r).population,
            None => sim.population(),
        };
        if population > MAX_EXPORT_CELLS {
            let _ = tx.send(Err(format!(
                "{} live cells are more than the {} an RLE export holds; save the state instead (top bar, Save).",
                thousands(u128::from(population)),
                thousands(u128::from(MAX_EXPORT_CELLS))
            )));
            return;
        }
        let rule = sim.rule().to_string();
        let mut pattern = match rect.map(|r| sim.copy_region(r)) {
            Some(Ok(copy)) => copy,
            Some(Err(e)) => {
                let _ = tx.send(Err(e.to_string()));
                return;
            }
            None => Pattern::from_live_cells(sim.live_cells()).with_rule(rule.clone()),
        };
        pattern.name = match rect {
            Some(_) => "Classic Conway selection".into(),
            None => "Classic Conway world".into(),
        };
        pattern.comments.push(format!("Exported by Emergence Lab at generation {generation}."));
        let spawned = std::thread::Builder::new().name("conway-rle-export".into()).spawn(move || {
            let text = patterns::rle::write(&pattern);
            let _ = tx.send(Ok(ExportText {
                text,
                cells: pattern.population() as u64,
                size: (pattern.width(), pattern.height()),
                scope,
                generation,
            }));
        });
        if let Err(e) = spawned {
            warn!("could not start the RLE writer: {e}");
        }
    })));
    rx
}

/// Writes an export to `<data>/exports/`.
fn save_export(data: &DataLocation, export: &ExportText) -> Result<PathBuf, String> {
    let dirs = data.dirs().map_err(|e| e.to_string())?;
    let folder = dirs.exports();
    std::fs::create_dir_all(&folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    let what = match export.scope {
        ExportScope::World => "world",
        ExportScope::Selection => "selection",
    };
    let stamp = interface::format::UtcDateTime::now().file_stamp();
    let path = folder.join(format!("classic-conway-{stamp}-{what}-gen{}.rle", export.generation));
    persistence::atomic_write(&path, export.text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// egui textures of pattern thumbnails.
#[derive(Default)]
pub(crate) struct Thumbnails {
    textures: HashMap<(String, [u8; 3]), egui::TextureHandle>,
    /// The import preview's texture and the pattern (identity, colour) it shows.
    import: Option<(usize, [u8; 3], egui::TextureHandle)>,
}

impl Thumbnails {
    /// Texture of the current import preview (one texture, replaced when the
    /// parsed pattern changes).
    fn import(
        &mut self,
        ctx: &egui::Context,
        pattern: &Arc<Pattern>,
        colors: ([u8; 3], [u8; 3]),
    ) -> egui::TextureHandle {
        let id = Arc::as_ptr(pattern) as usize;
        match &self.import {
            Some((known, alive, texture)) if *known == id && *alive == colors.1 => texture.clone(),
            _ => {
                let (w, h, rgba) = thumbnail_rgba(pattern, 192, colors.1, colors.0);
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                let texture = ctx.load_texture("conway_thumb_import", image, egui::TextureOptions::NEAREST);
                self.import = Some((id, colors.1, texture.clone()));
                texture
            }
        }
    }

    fn get(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        pattern: &Pattern,
        colors: ([u8; 3], [u8; 3]),
    ) -> egui::TextureHandle {
        self.textures
            .entry((key.to_string(), colors.1))
            .or_insert_with(|| {
                let (w, h, rgba) = thumbnail_rgba(pattern, 192, colors.1, colors.0);
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                // Crisp when magnified, averaged when reduced.
                let options =
                    egui::TextureOptions { minification: egui::TextureFilter::Linear, ..egui::TextureOptions::NEAREST };
                ctx.load_texture(format!("conway_thumb_{key}"), image, options)
            })
            .clone()
    }
}

/// Whether a kind (`still_life`) only repeats its category (`still-lifes`).
pub fn kind_repeats_category(category: Option<&str>, kind: &str) -> bool {
    let Some(category) = category else { return false };
    let (c, k) = (category_label(category).to_lowercase(), kind_label(kind).to_lowercase());
    c == k || c == format!("{k}s")
}

/// Draws a thumbnail fitted into `max` (aspect kept, centred).
fn paint_thumbnail(ui: &mut Ui, texture: &egui::TextureHandle, rect: egui::Rect, palette: &Palette, bg: Color32) {
    ui.painter().rect_filled(rect, 4.0, bg);
    let [w, h] = texture.size();
    let k = ((rect.width() - 6.0) / w as f32).min((rect.height() - 6.0) / h as f32);
    let size = vec2(w as f32 * k, h as f32 * k);
    let r = egui::Rect::from_center_size(rect.center(), size);
    egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), size)).paint_at(ui, r);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
}

/// `EguiPrimaryContextPass`: the Patterns window.
#[allow(clippy::too_many_arguments)]
pub(crate) fn patterns_window(
    mut contexts: EguiContexts,
    mut window: ResMut<PatternsWindow>,
    mut library: ResMut<ConwayLibrary>,
    theme: Res<ThemeSettings>,
    registry: Res<ModeRegistry>,
    runner: Option<Res<ConwayRunner>>,
    selection: Res<Selection>,
    live: Res<LiveWorld>,
    settings: Res<ViewSettings>,
    data: Option<Res<DataLocation>>,
    mut queue: ResMut<ModeCommands>,
    mut loads: ResMut<LoadRequests>,
    mut toasts: ResMut<Toasts>,
    mut clipboard: Option<ResMut<EguiClipboard>>,
    mut thumbs: Local<Thumbnails>,
) {
    // Collect a finished export even while the window is closed.
    let finished = lock(&window.export_pending).as_ref().map(Receiver::try_recv);
    match finished {
        Some(Ok(result)) => {
            *lock(&window.export_pending) = None;
            window.export = Some(result);
        }
        Some(Err(TryRecvError::Disconnected)) => *lock(&window.export_pending) = None,
        Some(Err(TryRecvError::Empty)) | None => {}
    }
    if std::mem::take(&mut window.export_request)
        && let Some(runner) = runner.as_deref()
        && !window.exporting()
    {
        let rx = request_export(runner, window.export_scope, selection.rect);
        *lock(&window.export_pending) = Some(rx);
        window.saved = None;
    }
    if !window.open {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let ctx = ctx.clone();
    let palette = mode_palette(&theme, &registry);
    let colors = settings.colors.colors();
    let mut open = window.open;
    egui::Window::new(RichText::new("Patterns").strong())
        .id(egui::Id::new("conway_patterns_window"))
        .open(&mut open)
        .default_size([880.0, 560.0])
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(egui::pos2(ctx.content_rect().center().x, 90.0))
        .min_width(640.0)
        .min_height(380.0)
        .resizable(true)
        .collapsible(false)
        .show(&ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (PatternsTab::Library, "Library"),
                    (PatternsTab::Import, "Import"),
                    (PatternsTab::Export, "Export RLE"),
                ] {
                    if ui.selectable_label(window.tab == tab, RichText::new(label).size(14.0)).clicked() {
                        window.tab = tab;
                    }
                }
            });
            ui.separator();
            match window.tab {
                PatternsTab::Library => {
                    library_tab(ui, &palette, colors, &mut window, &mut library, &mut thumbs, &mut queue, &mut loads)
                }
                PatternsTab::Import => {
                    import_tab(ui, &palette, colors, &mut window, &mut thumbs, &mut queue, clipboard.as_deref_mut())
                }
                PatternsTab::Export => export_tab(
                    ui,
                    &palette,
                    &mut window,
                    runner.as_deref(),
                    &selection,
                    &live,
                    data.as_deref(),
                    &mut toasts,
                    clipboard.as_deref_mut(),
                ),
            }
        });
    window.open = open;
}

fn small(text: impl Into<String>, palette: &Palette) -> RichText {
    RichText::new(text).size(12.0).color(palette.text_muted)
}

#[allow(clippy::too_many_arguments)]
fn library_tab(
    ui: &mut Ui,
    palette: &Palette,
    colors: ([u8; 3], [u8; 3]),
    window: &mut PatternsWindow,
    library: &mut ConwayLibrary,
    thumbs: &mut Thumbnails,
    queue: &mut ModeCommands,
    loads: &mut LoadRequests,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut window.query)
                .hint_text("Search name, kind, discoverer, year…")
                .desired_width(280.0),
        );
        if !window.query.is_empty() && ui.small_button("Clear").clicked() {
            window.query.clear();
        }
        let shown = library.filtered(window.category.as_deref(), &window.query).len();
        let total = library.catalog.len();
        let count = if shown == total { format!("{total} patterns") } else { format!("{shown} of {total} patterns") };
        let note = match &library.source {
            LibrarySource::Assets(_) => format!("{count} · assets/patterns/catalog.toml"),
            LibrarySource::Builtin { .. } => format!("{count} · built-in classics (asset catalog unavailable)"),
        };
        let response = ui.label(small(note, palette));
        if let LibrarySource::Builtin { reason } = &library.source {
            response.on_hover_text(reason);
        }
    });
    ui.add_space(4.0);
    let ids = library.filtered(window.category.as_deref(), &window.query);
    if window.selected.as_ref().is_none_or(|s| library.entry(s).is_none()) {
        window.selected = ids.first().cloned();
    }
    let height = ui.available_height().max(200.0);
    let categories = library.categories();
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(vec2(160.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_width(160.0);
            egui::ScrollArea::vertical().id_salt("conway_lib_categories").show(ui, |ui| {
                let total = library.catalog.len();
                if ui.selectable_label(window.category.is_none(), format!("All  ({total})")).clicked() {
                    window.category = None;
                }
                for (category, count) in &categories {
                    let on = window.category.as_deref() == Some(category.as_str());
                    if ui.selectable_label(on, format!("{}  ({count})", category_label(category))).clicked() {
                        window.category = Some(category.clone());
                    }
                }
            });
        });
        ui.separator();
        ui.allocate_ui_with_layout(vec2(290.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_width(290.0);
            if ids.is_empty() {
                ui.label(small("No pattern matches.", palette));
            }
            let row_height = 48.0;
            egui::ScrollArea::vertical().id_salt("conway_lib_list").auto_shrink([false, false]).show_rows(
                ui,
                row_height,
                ids.len(),
                |ui, range| {
                    for id in &ids[range] {
                        let clicked = entry_row(
                            ui,
                            palette,
                            colors,
                            library,
                            thumbs,
                            id,
                            window.selected.as_deref() == Some(id.as_str()),
                            row_height,
                        );
                        if clicked {
                            window.selected = Some(id.clone());
                        }
                    }
                },
            );
        });
        ui.separator();
        ui.vertical(|ui| {
            egui::ScrollArea::vertical().id_salt("conway_lib_details").auto_shrink([false, false]).show(ui, |ui| {
                match window.selected.clone() {
                    Some(id) => details(ui, palette, colors, library, thumbs, &id, queue, loads),
                    None => {
                        ui.label(small("Select a pattern.", palette));
                    }
                }
            });
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn entry_row(
    ui: &mut Ui,
    palette: &Palette,
    colors: ([u8; 3], [u8; 3]),
    library: &mut ConwayLibrary,
    thumbs: &mut Thumbnails,
    id: &str,
    selected: bool,
    height: f32,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height - 2.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response.clicked();
    }
    let fill = if selected {
        with_alpha(palette.accent, 0.20)
    } else if response.hovered() {
        palette.surface_hi
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    if selected {
        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, palette.accent), egui::StrokeKind::Inside);
    }
    let thumb_rect = egui::Rect::from_min_size(rect.min + vec2(4.0, 3.0), vec2(height - 8.0, height - 8.0));
    let bg = Color32::from_rgb(colors.0[0], colors.0[1], colors.0[2]);
    if let Ok(pattern) = library.pattern(id) {
        let texture = thumbs.get(ui.ctx(), id, &pattern, colors);
        paint_thumbnail(ui, &texture, thumb_rect, palette, bg);
    } else {
        ui.painter().rect_filled(thumb_rect, 4.0, bg);
    }
    let Some(entry) = library.entry(id) else { return response.clicked() };
    let left = thumb_rect.right() + 10.0;
    let max_width = (rect.right() - left - 6.0).max(10.0);
    let line = |text: String, size: f32, color: Color32| {
        let mut job = egui::text::LayoutJob::single_section(
            text,
            egui::TextFormat { font_id: egui::FontId::proportional(size), color, ..default() },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let kind = entry.kind.as_deref().map(kind_label).unwrap_or_default();
    let mut detail = kind;
    if let Some(p) = entry.period.filter(|p| *p > 0) {
        detail.push_str(&format!(" · p{p}"));
    }
    if let Some(speed) = speed_text(entry) {
        detail.push_str(&format!(" · {speed}"));
    }
    ui.painter().galley(egui::pos2(left, rect.top() + 6.0), line(entry.name.clone(), 13.5, palette.text), palette.text);
    ui.painter().galley(
        egui::pos2(left, rect.top() + 25.0),
        line(detail, 11.5, palette.text_muted),
        palette.text_muted,
    );
    response.clicked()
}

#[allow(clippy::too_many_arguments)]
fn details(
    ui: &mut Ui,
    palette: &Palette,
    colors: ([u8; 3], [u8; 3]),
    library: &mut ConwayLibrary,
    thumbs: &mut Thumbnails,
    id: &str,
    queue: &mut ModeCommands,
    loads: &mut LoadRequests,
) {
    let pattern = library.pattern(id);
    let Some(entry) = library.entry(id).cloned() else { return };
    ui.label(RichText::new(&entry.name).size(18.0).strong());
    ui.horizontal_wrapped(|ui| {
        if let Some(c) = &entry.category {
            badge(ui, &category_label(c), palette.accent);
        }
        // The kind adds nothing when it is the category's singular.
        if let Some(k) = entry.kind.as_deref().filter(|k| !kind_repeats_category(entry.category.as_deref(), k)) {
            badge(ui, &kind_label(k), palette.text_muted);
        }
        if entry.extra.get("verified").map(String::as_str) == Some("true") {
            badge(ui, "verified", palette.success)
                .on_hover_text(entry.extra.get("verification").cloned().unwrap_or_default());
        }
    });
    ui.add_space(6.0);
    let bg = Color32::from_rgb(colors.0[0], colors.0[1], colors.0[2]);
    match &pattern {
        Ok(p) => {
            let width = ui.available_width().min(420.0);
            let (rect, _) = ui.allocate_exact_size(vec2(width, 180.0), Sense::hover());
            let texture = thumbs.get(ui.ctx(), id, p, colors);
            paint_thumbnail(ui, &texture, rect, palette, bg);
        }
        Err(e) => {
            ui.colored_label(palette.error, format!("Could not read the pattern file: {e}"));
        }
    }
    ui.add_space(6.0);
    if let Ok(p) = &pattern {
        ui.horizontal_wrapped(|ui| {
            let stamp = StampPattern { name: entry.name.clone(), source: StampSource::Library(id.to_string()), pattern: p.clone() };
            if interface::icons::primary_button(ui, "Use as stamp", palette)
                .on_hover_text("Select the stamp tool with this pattern: click in the world to place it (Q / E rotate, M mirrors).")
                .clicked()
            {
                queue.0.push(ModeCommand::UseStamp(stamp));
            }
            if ui.button("Place at view centre").on_hover_text("Paste it at the centre of the view (undoable).").clicked() {
                queue.0.push(ModeCommand::PlaceAtCentre(p.clone()));
            }
            if ui
                .button("Open on an empty plane")
                .on_hover_text("Replace the simulation by this pattern alone on an empty infinite plane.")
                .clicked()
            {
                loads.0.push(LoadRequest::Pattern(id.to_string()));
            }
            if ui
                .button("Replace world")
                .on_hover_text("Keep the world (topology, size) but replace every cell by this pattern, at generation 0 (undoable).")
                .clicked()
            {
                queue.0.push(ModeCommand::ReplaceWorld(p.clone()));
            }
        });
        ui.add_space(6.0);
    }
    if let Some(d) = &entry.description {
        ui.label(RichText::new(d).size(13.0));
        ui.add_space(4.0);
    }
    egui::Grid::new("conway_lib_facts").num_columns(2).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
        let mut row = |label: &str, value: String| {
            ui.label(small(label, palette));
            ui.label(RichText::new(value).size(12.5));
            ui.end_row();
        };
        if let Some(d) = &entry.discoverer {
            row("Discoverer", d.clone());
        }
        if let Some(y) = entry.year {
            row("Year", y.to_string());
        }
        row("Period", period_text(&entry));
        if let Some(s) = speed_text(&entry) {
            row("Speed", s);
        }
        if let Some((dx, dy)) = entry.displacement.filter(|d| *d != (0, 0)) {
            row("Displacement per period", format!("({dx}, {dy})"));
        }
        if let Some(n) = entry.population {
            row("Population (initial)", thousands(u128::from(n)));
        }
        if let (Some(w), Some(h)) = (entry.width, entry.height) {
            row("Bounding box", format!("{w} × {h}"));
        }
        for (label, value) in measured_facts(&entry) {
            row(label, value);
        }
        if let Some(b) = &entry.expected_behavior {
            row("Expected behaviour", b.clone());
        }
        if let Some(r) = entry.rule.as_ref().or(pattern.as_ref().ok().and_then(|p| p.rule.as_ref())) {
            row("Rule", r.clone());
        }
    });
    ui.add_space(8.0);
    egui::CollapsingHeader::new(RichText::new("Licence and provenance").size(13.0))
        .id_salt("conway_lib_licence")
        .default_open(true)
        .show(ui, |ui| {
            if let Some(l) = &entry.license {
                ui.label(RichText::new(l).size(12.5).strong());
            }
            if let Some(n) = entry.extra.get("license_note") {
                ui.label(small(n.clone(), palette));
            }
            if let Some(s) = &entry.source {
                ui.label(small(format!("Source: {s}"), palette));
            }
            if let Some(f) = entry.extra.get("facts_source") {
                ui.label(small(format!("Facts: {f}"), palette));
            }
            if let Some(v) = entry.extra.get("verification") {
                ui.label(small(format!("Verified: {v}"), palette));
            }
            ui.label(small(format!("File: {}", entry.file), palette));
        });
}

fn import_tab(
    ui: &mut Ui,
    palette: &Palette,
    colors: ([u8; 3], [u8; 3]),
    window: &mut PatternsWindow,
    thumbs: &mut Thumbnails,
    queue: &mut ModeCommands,
    clipboard: Option<&mut EguiClipboard>,
) {
    ui.label(small(
        "Paste pattern text (RLE, plaintext .cells, Life 1.05 or Life 1.06) or read a file. Errors give the line and column.",
        palette,
    ));
    ui.add_space(4.0);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut window.import_path).hint_text("/path/to/pattern.rle").desired_width(360.0),
        );
        if ui.button("Read file").clicked() {
            let path = PathBuf::from(window.import_path.trim());
            match persistence::read_limited(&path, 16 << 20) {
                Ok(bytes) => {
                    window.import_text = String::from_utf8_lossy(&bytes).into_owned();
                    window.import_file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                    changed = true;
                }
                Err(e) => window.import = Some(Err(format!("cannot read {}: {e}", path.display()))),
            }
        }
        if let Some(clipboard) = clipboard
            && ui.button("Paste from clipboard").clicked()
        {
            match clipboard.get_text() {
                Some(text) => {
                    window.import_text = text;
                    window.import_file_name = None;
                    changed = true;
                }
                None => window.import = Some(Err("the clipboard holds no text".into())),
            }
        }
        if ui.button("Clear").clicked() {
            window.import_text.clear();
            window.import_file_name = None;
            changed = true;
        }
    });
    ui.add_space(4.0);
    if window.import_text.len() <= MAX_EDITABLE_TEXT {
        let editor = egui::TextEdit::multiline(&mut window.import_text)
            .code_editor()
            .desired_rows(10)
            .desired_width(f32::INFINITY)
            .hint_text("x = 3, y = 3, rule = B3/S23\nbo$2bo$3o!");
        let response = egui::ScrollArea::vertical()
            .id_salt("conway_import_text")
            .max_height(200.0)
            .show(ui, |ui| ui.add(editor))
            .inner;
        if response.changed() {
            window.import_file_name = None;
            changed = true;
        }
    } else {
        ui.label(small(
            format!("{} bytes of text (too long to edit here).", thousands(window.import_text.len() as u128)),
            palette,
        ));
    }
    if changed {
        window.reparse();
    }
    ui.add_space(6.0);
    match &window.import {
        None => {
            ui.label(small("Nothing to import yet.", palette));
        }
        Some(Err(e)) => {
            ui.colored_label(palette.error, RichText::new(e).size(13.0));
        }
        Some(Ok(preview)) => {
            let preview = preview.clone();
            ui.horizontal_top(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(140.0, 110.0), Sense::hover());
                let texture = thumbs.import(ui.ctx(), &preview.pattern, colors);
                paint_thumbnail(ui, &texture, rect, palette, Color32::from_rgb(colors.0[0], colors.0[1], colors.0[2]));
                ui.vertical(|ui| {
                    let p = &preview.pattern;
                    let name = if p.name.is_empty() { "Unnamed pattern" } else { p.name.as_str() };
                    ui.label(RichText::new(name).size(15.0).strong());
                    let rule = p.rule.as_deref().unwrap_or("none stated");
                    ui.label(small(
                        format!(
                            "{} · {} live cells · {} × {} · rule {rule}",
                            preview.format,
                            thousands(p.population() as u128),
                            p.width(),
                            p.height()
                        ),
                        palette,
                    ));
                    if !p.author.is_empty() {
                        ui.label(small(format!("Author: {}", p.author), palette));
                    }
                    for w in &preview.warnings {
                        ui.colored_label(palette.warning, RichText::new(w).size(12.5));
                    }
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if interface::icons::primary_button(ui, "Use as stamp", palette).clicked() {
                            let name = if p.name.is_empty() { "Imported pattern".to_string() } else { p.name.clone() };
                            queue.0.push(ModeCommand::UseStamp(StampPattern {
                                name,
                                source: StampSource::Imported,
                                pattern: p.clone(),
                            }));
                        }
                        if ui.button("Place at view centre").clicked() {
                            queue.0.push(ModeCommand::PlaceAtCentre(p.clone()));
                        }
                        if ui
                            .button("Replace world")
                            .on_hover_text("Every cell replaced, generation 0 (undoable).")
                            .clicked()
                        {
                            queue.0.push(ModeCommand::ReplaceWorld(p.clone()));
                        }
                    });
                });
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn export_tab(
    ui: &mut Ui,
    palette: &Palette,
    window: &mut PatternsWindow,
    runner: Option<&ConwayRunner>,
    selection: &Selection,
    live: &LiveWorld,
    data: Option<&DataLocation>,
    toasts: &mut Toasts,
    clipboard: Option<&mut EguiClipboard>,
) {
    ui.label(small(
        "Golly-compatible extended RLE. The whole world keeps its absolute position (#CXRLE Pos); a selection is \
         written relative to its top-left corner.",
        palette,
    ));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.radio_value(&mut window.export_scope, ExportScope::World, "Whole world");
        ui.add_enabled_ui(selection.rect.is_some(), |ui| {
            ui.radio_value(&mut window.export_scope, ExportScope::Selection, "Selection");
        })
        .response
        .on_disabled_hover_text("Select a region with the Select tool (key 6) first.");
        if selection.rect.is_none() && window.export_scope == ExportScope::Selection {
            window.export_scope = ExportScope::World;
        }
        let population = live.current().map(|i| i.population);
        if let Some(p) = population {
            ui.label(small(format!("world: {} live cells", thousands(p)), palette));
        }
    });
    ui.add_space(4.0);
    let busy = window.exporting();
    ui.horizontal(|ui| {
        let generate = ui.add_enabled(runner.is_some() && !busy, egui::Button::new("Generate RLE"));
        if generate.clicked()
            && let Some(runner) = runner
        {
            let rx = request_export(runner, window.export_scope, selection.rect);
            *lock(&window.export_pending) = Some(rx);
            window.saved = None;
        }
        if busy {
            ui.spinner();
        }
    });
    ui.add_space(6.0);
    let export = match &window.export {
        None => return,
        Some(Err(e)) => {
            ui.colored_label(palette.error, e);
            return;
        }
        Some(Ok(export)) => export.clone(),
    };
    let what = match export.scope {
        ExportScope::World => "World",
        ExportScope::Selection => "Selection",
    };
    ui.label(
        RichText::new(format!(
            "{what} at generation {} · {} live cells · {} × {} · {} bytes",
            thousands(u128::from(export.generation)),
            thousands(u128::from(export.cells)),
            export.size.0,
            export.size.1,
            thousands(export.text.len() as u128)
        ))
        .size(13.0),
    );
    ui.horizontal(|ui| {
        if let Some(clipboard) = clipboard
            && ui.button("Copy to clipboard").clicked()
        {
            clipboard.set_text(&export.text);
            toasts.success("RLE copied to the clipboard.");
        }
        let can_save = data.is_some();
        if ui.add_enabled(can_save, egui::Button::new("Save to exports folder")).clicked()
            && let Some(data) = data
        {
            match save_export(data, &export) {
                Ok(path) => {
                    toasts.success(format!("Saved {}", path.display()));
                    window.saved = Some(path);
                }
                Err(e) => toasts.error(format!("Could not save the RLE: {e}")),
            }
        }
    });
    if let Some(path) = &window.saved {
        ui.label(small(format!("Saved as {}", path.display()), palette));
    }
    ui.add_space(4.0);
    const PREVIEW: usize = 6_000;
    let mut shown = if export.text.len() > PREVIEW {
        let cut = export.text.char_indices().take_while(|(i, _)| *i < PREVIEW).last().map_or(0, |(i, _)| i);
        format!("{}\n… ({} more bytes)", &export.text[..cut], thousands((export.text.len() - cut) as u128))
    } else {
        export.text.clone()
    };
    egui::ScrollArea::vertical().id_salt("conway_export_text").max_height(ui.available_height()).show(ui, |ui| {
        ui.add(egui::TextEdit::multiline(&mut shown).code_editor().desired_width(f32::INFINITY).interactive(false));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exports read the model without changing it.
    #[test]
    fn exports_are_reads() {
        let runner = ConwayRunner(Arc::new(simulation_core::Runner::spawn(
            LifeSim::conway(4),
            simulation_core::RunnerConfig::default(),
        )));
        let mut rx = None;
        crate::commands::assert_reads_only(&runner, || rx = Some(request_export(&runner, ExportScope::World, None)));
        let export = rx.unwrap().recv_timeout(std::time::Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!(export.generation, 0);
        assert!(export.cells > 0);
    }

    #[test]
    fn imports_report_formats_errors_and_rules() {
        let ok = preview_import("x = 3, y = 3, rule = B3/S23\nbo$2bo$3o!", None).unwrap();
        assert_eq!(ok.pattern.population(), 5);
        assert_eq!(ok.format, "RLE");
        assert!(ok.warnings.is_empty());
        let plain = preview_import("!Name: Blinker\nOOO\n", None).unwrap();
        assert_eq!(plain.pattern.name, "Blinker");
        let highlife = preview_import("x = 3, y = 1, rule = B36/S23\n3o!", None).unwrap();
        assert!(highlife.warnings[0].contains("B36/S23"), "{:?}", highlife.warnings);
        let generations = preview_import("x = 2, y = 1, rule = /2/3\nAB!", None).unwrap();
        assert!(generations.warnings.iter().any(|w| w.contains("states")), "{:?}", generations.warnings);
        let err = preview_import("x = 1, y = 1\n2yX!", None).err().unwrap();
        assert!(err.contains("line 2, column 2") && err.contains("out of range"), "{err}");
        assert!(preview_import("x = 0, y = 0\n!", None).is_err(), "empty patterns are refused");
    }

    #[test]
    fn kind_badges_skip_repeated_categories() {
        assert!(kind_repeats_category(Some("still-lifes"), "still_life"));
        assert!(kind_repeats_category(Some("guns"), "gun"));
        assert!(!kind_repeats_category(Some("guns"), "oscillator"));
        assert!(!kind_repeats_category(None, "gun"));
    }

    #[test]
    fn window_toggles_tabs() {
        let mut w = PatternsWindow::default();
        w.toggle(PatternsTab::Import);
        assert!(w.open && w.tab == PatternsTab::Import);
        w.toggle(PatternsTab::Library);
        assert!(w.open && w.tab == PatternsTab::Library);
        w.toggle(PatternsTab::Library);
        assert!(!w.open);
        w.import_text = "OOO".into();
        w.reparse();
        assert!(matches!(w.import, Some(Ok(_))));
        w.import_text = "  ".into();
        w.reparse();
        assert!(w.import.is_none());
    }

    #[test]
    fn exports_are_valid_rle_and_saved_atomically() {
        let mut sim = cellular_automata::LifeSim::conway(4);
        use simulation_core::Simulation;
        sim.step();
        let pattern = Pattern::from_live_cells(sim.live_cells()).with_rule(sim.rule().to_string());
        let text = patterns::rle::write(&pattern);
        let back = patterns::parse_any(&text, Some("x.rle")).unwrap();
        assert_eq!(back.population(), pattern.population());
        let dir = std::env::temp_dir().join(format!("conway-export-test-{}", std::process::id()));
        let data = DataLocation::at(persistence::UserDataDirs::at(&dir));
        let export = ExportText {
            text,
            cells: pattern.population() as u64,
            size: (1, 1),
            scope: ExportScope::World,
            generation: 1,
        };
        let path = save_export(&data, &export).unwrap();
        assert!(path.starts_with(dir.join("exports")));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), export.text);
        let _ = std::fs::remove_dir_all(dir);
    }
}
