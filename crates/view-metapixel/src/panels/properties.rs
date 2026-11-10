//! The Properties tab: the tile inspector (with a mini map of the macro
//! grid), the validation of the exact readouts against the accelerated
//! prediction and the golden table, and the construction (tiles,
//! programming, build, Hashlife, credits).

use bevy::ecs::system::SystemParam;
use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, RichText, Sense, Stroke, Ui, vec2};
use egui_plot::{Line, LineStyle, Plot, PlotPoints, Points};
use interface::format::{format_bytes, format_count};
use interface::theme::with_alpha;
use interface::widgets::badge;
use interface::{ModeRegistry, Palette, ThemeSettings};
use metapixel::spec::{
    Direction, PERIOD, POPULATION_OFF_UNPROGRAMMED, POPULATION_ON_UNPROGRAMMED, READOUT_ON_COUNT, RuleTable,
    rule_bit_cell,
};
use metapixel::{AnnotationKind, LiveReading, MacroGrid, MacroView, Programming, TileReading};
use rendering::grid::{GridSpace, VisibleCells};

use super::{card, kv_grid, notice, phase_bar, rgb32, section, segmented};
use crate::geometry::GridShape;
use crate::golden::{Check, GoldenTable, compare, compare_cores};
use crate::macro_layer::{EditOverlay, MacroSource, colors, macro_texture};
use crate::rule_editor::{heard_count, programming_cells, removed_fuses};
use crate::session::{SimData, SimInfo, Tiles};
use crate::timing::stage_label;
use crate::tools::{Tool, ToolState};
use crate::world::{CameraFit, MpCamera, ViewSettings, current_shape, mode_palette};

/// Credits shown with the construction.
pub const CREDITS: &str = "The OTCA metapixel (Outer Totalistic Cellular Automata Meta-Pixel) was designed and \
built by Brice Due between autumn 2005 and spring 2006. Its tiles are third-party pattern data, redistributed \
under GPL-2.0-or-later (the same cells ship in Golly's GPL metapixel demos) and loaded from the asset folder at \
run time; they are not part of the application's code or binary. Coordinates, golden values and the timing come \
from runs of Golly's bgolly (see PROVENANCE.md next to the tiles and the About window).";

/// Section of the Properties tab.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PropertiesTab {
    /// Tile inspector and mini map.
    #[default]
    Inspector,
    /// Readouts against prediction and golden values.
    Validation,
    /// Tiles, programming, build statistics and credits.
    Construction,
}

impl PropertiesTab {
    /// Every tab.
    pub const ALL: [PropertiesTab; 3] =
        [PropertiesTab::Inspector, PropertiesTab::Validation, PropertiesTab::Construction];

    /// Label.
    pub fn label(self) -> &'static str {
        match self {
            PropertiesTab::Inspector => "Inspector",
            PropertiesTab::Validation => "Validation",
            PropertiesTab::Construction => "Construction",
        }
    }
}

/// Resources of the Properties tab.
#[derive(SystemParam)]
pub(crate) struct PropsCtx<'w, 's> {
    theme: Res<'w, ThemeSettings>,
    registry: Res<'w, ModeRegistry>,
    data: Res<'w, SimData>,
    tools: ResMut<'w, ToolState>,
    settings: Res<'w, ViewSettings>,
    tab: ResMut<'w, PropertiesTab>,
    tiles: Res<'w, Tiles>,
    visible: Res<'w, VisibleCells>,
    space: Res<'w, GridSpace>,
    fit: ResMut<'w, CameraFit>,
    camera: Query<'w, 's, &'static mut Transform, With<MpCamera>>,
}

/// Texture of the mini map.
#[derive(Default)]
pub(crate) struct MiniMap {
    texture: Option<egui::TextureHandle>,
    key: Option<(u64, u64, MacroSource, bool, u64)>,
}

/// Right panel, Properties tab.
pub(crate) fn properties_panel(InMut(ui): InMut<egui::Ui>, mut ctx: PropsCtx, mut minimap: Local<MiniMap>) {
    let palette = mode_palette(&ctx.theme, &ctx.registry);
    let items: Vec<(&str, &str, bool)> = PropertiesTab::ALL
        .iter()
        .map(|t| {
            let tip = match t {
                PropertiesTab::Inspector => "The tile under the pointer or the selected tile, and a mini map",
                PropertiesTab::Validation => "Exact readouts against the accelerated prediction and the golden table",
                PropertiesTab::Construction => "Tiles, programming, build statistics, Hashlife memory and credits",
            };
            (t.label(), tip, *ctx.tab == *t)
        })
        .collect();
    if let Some(i) = segmented(ui, &items, 3) {
        *ctx.tab = PropertiesTab::ALL[i];
    }
    if ctx.data.view.is_none() {
        ui.add_space(8.0);
        if ctx.tiles.error().is_some() {
            notice(
                ui,
                &palette,
                palette.error,
                "No construction: the OTCA tiles could not be loaded (see the Tools panel).",
            );
        } else {
            ui.label(RichText::new("Waiting for the construction…").weak());
        }
        return;
    }
    match *ctx.tab {
        PropertiesTab::Inspector => inspector(ui, &palette, &mut ctx, &mut minimap),
        PropertiesTab::Validation => validation(ui, &palette, &ctx),
        PropertiesTab::Construction => construction(ui, &palette, &ctx),
    }
}

fn reading_text(reading: Option<TileReading>) -> (String, Option<bool>) {
    match reading {
        Some(TileReading::On) => (format!("ON (core {} cells)", format_count(READOUT_ON_COUNT)), Some(true)),
        Some(TileReading::Off) => ("OFF (core empty)".to_string(), Some(false)),
        Some(TileReading::Ambiguous(c)) => (format!("AMBIGUOUS (core {} cells)", format_count(c)), None),
        None => ("not read yet".to_string(), None),
    }
}

fn live_text(reading: LiveReading, count: Option<u64>) -> String {
    let count = count.map(|c| format!(" · core {} cells", format_count(c))).unwrap_or_default();
    match reading {
        LiveReading::On => format!("ON{count}"),
        LiveReading::Off => format!("OFF{count}"),
        LiveReading::Transitioning(c) => {
            format!("switching · core {} of {} cells", format_count(c), format_count(READOUT_ON_COUNT))
        }
    }
}

/// The macro states read at the latest readout (the as-built pattern before
/// the first one), as a grid.
pub fn readout_grid(view: &MacroView) -> Option<MacroGrid> {
    let mut grid = MacroGrid::new(view.columns, view.rows).ok()?;
    for (i, t) in view.tiles.iter().enumerate() {
        let on = match t.exact {
            Some(TileReading::On) => true,
            Some(_) => false,
            None => t.accelerated,
        };
        grid.set(i as u32 % view.columns, i as u32 / view.columns, on);
    }
    Some(grid)
}

/// The next macro state the rule gives a tile with `count` heard ON
/// neighbours, and the table digit that decides it.
pub fn next_state(programming: Programming, on: bool, count: u8) -> (bool, String) {
    if on {
        let survives = programming.rule.survives(count);
        (
            survives,
            format!("S{count} {}", if survives { "is in the rule: survives" } else { "is not in the rule: dies" }),
        )
    } else {
        let born = programming.rule.births(count);
        (born, format!("B{count} {}", if born { "is in the rule: born" } else { "is not in the rule: stays OFF" }))
    }
}

fn inspector(ui: &mut Ui, palette: &Palette, ctx: &mut PropsCtx, minimap: &mut MiniMap) {
    let Some(view) = ctx.data.view.clone() else { return };
    let Some(m) = view.macro_grid.as_ref() else { return };
    let shape = GridShape::new(m.columns, m.rows);
    section(ui, palette, "Macro grid");
    mini_map(ui, palette, ctx, m, shape, minimap);
    let pinned = ctx.tools.selected.filter(|&(c, r)| shape.contains(c, r));
    let focus = pinned.or(ctx.tools.hover_tile.filter(|&(c, r)| shape.contains(c, r)));
    section(ui, palette, "Tile");
    let Some((col, row)) = focus else {
        card(ui, palette, |ui| {
            ui.label(
                RichText::new("Hover a tile in the world or on the map to read it; click to pin it here.")
                    .size(12.5)
                    .color(palette.text_muted),
            );
        });
        return;
    };
    let Some(tile) = m.tiles.get(row as usize * m.columns as usize + col as usize) else { return };
    let programming = ctx.data.info.as_ref().map_or(Programming::conway(), |i| i.active.programming);
    let inspection = view.inspection.as_ref().filter(|i| (i.col, i.row) == (col, row));
    card(ui, palette, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Tile ({col}, {row})")).size(14.5).strong());
            if pinned.is_some() {
                badge(ui, "PINNED", palette.accent);
                if ui.small_button("Unpin").clicked() {
                    ctx.tools.selected = None;
                }
            } else {
                badge(ui, "HOVER", palette.text_muted);
            }
        });
        let rect = shape.tile_rect(col, row);
        ui.label(
            RichText::new(format!(
                "Cells x {}..{}, y {}..{} (2,048 × 2,048)",
                format_count(rect.x0 as u64),
                format_count(rect.x1 as u64),
                format_count(rect.y0 as u64),
                format_count(rect.y1 as u64)
            ))
            .size(11.5)
            .color(palette.text_muted),
        );
        ui.add_space(4.0);
        let mut rows: Vec<(&str, String)> = Vec::new();
        let read_label = match m.exact_metagen {
            Some(n) => format!("metagen {}", format_count(n)),
            None => "as built".to_string(),
        };
        let (text, _) = reading_text(tile.exact);
        let readout = if tile.exact.is_some() {
            format!("{text} · {read_label}")
        } else {
            format!("{} (metagen 0, no readout yet)", if tile.accelerated { "ON" } else { "OFF" })
        };
        rows.push(("Readout", readout));
        if tile.exact.is_some() {
            rows.push((
                "Prediction",
                format!(
                    "{} (accelerated, metagen {})",
                    if tile.accelerated { "ON" } else { "OFF" },
                    m.accelerated_metagen
                ),
            ));
            rows.push(("Agreement", if tile.mismatch { "MISMATCH".to_string() } else { "agrees".to_string() }));
        }
        match (inspection, tile.live) {
            (Some(i), _) => rows.push(("Core now", live_text(i.live, Some(i.core_count)))),
            (None, Some(live)) => rows.push(("Core now", live_text(live, None))),
            (None, None) => rows.push(("Core now", "pin the tile to count its core".to_string())),
        }
        rows.push(("Live cells now", format_count(tile.population)));
        rows.push((
            "Phase",
            format!(
                "t mod P = {} of {} · {}",
                format_count(view.phase.phase),
                format_count(PERIOD),
                stage_label(view.phase.stage)
            ),
        ));
        kv_grid(ui, "mp_tile_rows", palette, &rows);
        let width = ui.available_width();
        phase_bar(ui, palette, view.phase.phase, view.phase.generation < PERIOD, width, 8.0);
        if tile.mismatch {
            notice(
                ui,
                palette,
                palette.error,
                "The exact readout of this tile differs from the accelerated prediction. The readout is what the \
                 physical cells did; the prediction is only a check.",
            );
        }
        // Next state from the readout and the rule.
        if let Some(grid) = readout_grid(m) {
            let count = heard_count(&grid, programming, col, row);
            let on = grid.get(col, row);
            let (next, why) = next_state(programming, on, count);
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Hears {count} ON neighbour{} in the {} pattern; {why}. Expected next metagen: {}.",
                    if count == 1 { "" } else { "s" },
                    if m.exact_metagen.is_some() { "latest readout" } else { "as-built" },
                    if next { "ON" } else { "OFF" }
                ))
                .size(12.0),
            );
            rule_bits(ui, palette, programming, on, count);
            neighbour_compass(ui, palette, programming, &grid, col, row);
        }
    });
    if let Some(i) = inspection {
        egui::CollapsingHeader::new(RichText::new("Parts of this tile").size(12.5))
            .id_salt("mp_parts")
            .default_open(false)
            .show(ui, |ui| {
                for a in &i.annotations {
                    let color = if a.verified { palette.text } else { palette.text_muted };
                    let kind = match a.kind {
                        AnnotationKind::RuleBit { programmed: true, .. } => "rule bit, programmed",
                        AnnotationKind::RuleBit { .. } => "rule bit, not programmed",
                        AnnotationKind::Fuse { present: true, .. } => "fuse, present",
                        AnnotationKind::Fuse { .. } => "fuse, removed",
                        AnnotationKind::Readout => "readout core",
                        AnnotationKind::Display => "display",
                        AnnotationKind::Mechanism => "mechanism (UNVERIFIED role)",
                    };
                    ui.label(
                        RichText::new(format!(
                            "{} · {kind} · ({}, {})..({}, {})",
                            a.label, a.rect.x0, a.rect.y0, a.rect.x1, a.rect.y1
                        ))
                        .size(11.5)
                        .color(color),
                    )
                    .on_hover_text(&a.note);
                }
                ui.label(
                    RichText::new("Coordinates are tile-nominal (0..2047). Verified parts were checked by simulation; the roles of the mechanism anchors are interpretations of Brice Due's slide captions.")
                        .size(11.0)
                        .color(palette.text_muted),
                );
            });
    }
}

/// The B and S rows of the rule table, with the deciding digit marked.
fn rule_bits(ui: &mut Ui, palette: &Palette, programming: Programming, on: bool, count: u8) {
    ui.add_space(4.0);
    for (table, letter) in [(RuleTable::Birth, 'B'), (RuleTable::Survival, 'S')] {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.label(RichText::new(letter.to_string()).monospace().size(12.0).color(palette.text_muted));
            for d in 0..=8u8 {
                let set = match table {
                    RuleTable::Birth => programming.rule.births(d),
                    RuleTable::Survival => programming.rule.survives(d),
                };
                let deciding = d == count && on == (table == RuleTable::Survival);
                let (rect, response) = ui.allocate_exact_size(vec2(22.0, 20.0), Sense::hover());
                let fill = if set { with_alpha(palette.success, 0.75) } else { palette.surface_hi };
                ui.painter().rect_filled(rect, 3.0, fill);
                if deciding {
                    ui.painter().rect_stroke(rect, 3.0, Stroke::new(2.0, palette.accent), egui::StrokeKind::Inside);
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    d.to_string(),
                    egui::FontId::monospace(12.0),
                    if set { Color32::BLACK } else { palette.text_muted },
                );
                let (x, y) = rule_bit_cell(table, d);
                response.on_hover_text(format!(
                    "{letter}{d}: eater {} — completion cell ({x}, {y}) in every tile{}",
                    if set { "complete (in the rule)" } else { "broken (not in the rule)" },
                    if deciding { ". This digit decides the tile's next state." } else { "" }
                ));
            }
        });
    }
}

/// Neighbour channels: heard or deaf, and the neighbour's state.
fn neighbour_compass(ui: &mut Ui, palette: &Palette, programming: Programming, grid: &MacroGrid, col: u32, row: u32) {
    ui.add_space(4.0);
    let layout = [
        [Some(Direction::NW), Some(Direction::N), Some(Direction::NE)],
        [Some(Direction::W), None, Some(Direction::E)],
        [Some(Direction::SW), Some(Direction::S), Some(Direction::SE)],
    ];
    ui.vertical(|ui| {
        egui::Grid::new("mp_neighbours").striped(false).spacing(vec2(2.0, 2.0)).show(ui, |ui| {
            for line in layout {
                for cell in line {
                    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                    let painter = ui.painter();
                    match cell {
                        None => {
                            let on = grid.get(col, row);
                            painter.rect_filled(rect, 3.0, rgb32(if on { colors::ON } else { colors::OFF }));
                            painter.rect_stroke(rect, 3.0, Stroke::new(2.0, palette.accent), egui::StrokeKind::Inside);
                        }
                        Some(d) => {
                            let (dx, dy) = d.offset();
                            let (c, r) = (i64::from(col) + dx, i64::from(row) + dy);
                            let exists = c >= 0 && r >= 0 && (c as u32) < grid.width() && (r as u32) < grid.height();
                            let on = exists && grid.get(c as u32, r as u32);
                            let heard = programming.neighbourhood.listens(d);
                            let fill = if !exists {
                                palette.surface
                            } else {
                                rgb32(if on { colors::ON } else { colors::OFF })
                            };
                            painter.rect_filled(rect, 3.0, fill);
                            let stroke = if heard {
                                Stroke::new(1.0, palette.stroke)
                            } else {
                                Stroke::new(1.5, palette.error)
                            };
                            painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);
                            if !heard {
                                painter.line_segment([rect.left_top() + vec2(5.0, 5.0), rect.right_bottom() - vec2(5.0, 5.0)], Stroke::new(1.5, palette.error));
                            }
                            response.on_hover_text(format!(
                                "{}: {} · {}",
                                d.name(),
                                if !exists { "outside the grid (permanently OFF)" } else if on { "ON" } else { "OFF" },
                                if heard { "heard" } else { "not heard (fuse removed)" }
                            ));
                        }
                    }
                }
                ui.end_row();
            }
        });
        ui.label(
            RichText::new("Neighbours as the tile hears them. Crossed: a removed fuse makes every tile deaf to that direction. Outside the grid tiles do not exist and count as OFF.")
                .size(11.0)
                .color(palette.text_muted),
        );
    });
}

/// Mini map: the macro layer at one texel per tile, the camera's view and
/// the selection; a click selects the tile and centres the camera on it.
fn mini_map(
    ui: &mut Ui,
    palette: &Palette,
    ctx: &mut PropsCtx,
    m: &MacroView,
    shape: GridShape,
    minimap: &mut MiniMap,
) {
    let editing = matches!(ctx.tools.tool, Tool::Draw | Tool::Stamp);
    let key = (ctx.data.epoch, ctx.data.view_seq, ctx.settings.source, editing, ctx.data.info_rev);
    if minimap.key != Some(key) || minimap.texture.is_none() {
        let pattern = ctx.data.info.as_ref().map(|i| &i.initial).filter(|_| editing);
        let edit = pattern.map(|p| EditOverlay { pattern: p, stroke: &[], target: false });
        let bytes = macro_texture(m, ctx.settings.source, edit);
        let image = egui::ColorImage::from_rgba_unmultiplied([m.columns as usize, m.rows as usize], &bytes);
        match &mut minimap.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
            None => {
                minimap.texture = Some(ui.ctx().load_texture("mp_minimap", image, egui::TextureOptions::NEAREST));
            }
        }
        minimap.key = Some(key);
    }
    let Some(texture) = &minimap.texture else { return };
    let aspect = m.rows as f32 / m.columns.max(1) as f32;
    let width = ui.available_width().min(260.0 / aspect.max(0.2)).max(80.0);
    let height = (width * aspect).min(260.0);
    let width = height / aspect;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let painter = ui.painter_at(rect.expand(2.0));
    painter.image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Outside);
    let to_screen = |x: f64, y: f64| {
        let (w, h) = shape.size_cells();
        egui::pos2(
            rect.min.x + (x / w as f64) as f32 * rect.width(),
            rect.min.y + (y / h as f64) as f32 * rect.height(),
        )
    };
    // Mismatches.
    if m.mismatch_count > 0 {
        for (i, _) in m.tiles.iter().enumerate().filter(|(_, t)| t.mismatch) {
            let (c, r) = (i as u32 % m.columns, i as u32 / m.columns);
            let a = to_screen(f64::from(c) * 2048.0, f64::from(r) * 2048.0);
            let b = to_screen(f64::from(c + 1) * 2048.0, f64::from(r + 1) * 2048.0);
            painter.rect_stroke(
                egui::Rect::from_min_max(a, b),
                0.0,
                Stroke::new(1.5, rgb32(colors::MISMATCH)),
                egui::StrokeKind::Inside,
            );
        }
    }
    // The camera's view.
    let v = &ctx.visible;
    let a = to_screen(v.min_x as f64, v.min_y as f64);
    let b = to_screen(v.max_x as f64 + 1.0, v.max_y as f64 + 1.0);
    let view_rect = egui::Rect::from_min_max(a, b).intersect(rect);
    if view_rect.is_positive() {
        painter.rect_stroke(view_rect, 0.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Inside);
    }
    if let Some((c, r)) = ctx.tools.selected.filter(|&(c, r)| shape.contains(c, r)) {
        let a = to_screen(f64::from(c) * 2048.0, f64::from(r) * 2048.0);
        let b = to_screen(f64::from(c + 1) * 2048.0, f64::from(r + 1) * 2048.0);
        painter.rect_stroke(
            egui::Rect::from_min_max(a, b).expand(1.0),
            0.0,
            Stroke::new(2.0, palette.accent),
            egui::StrokeKind::Outside,
        );
    }
    let hovered_tile = response.hover_pos().map(|p| {
        let fx = ((p.x - rect.min.x) / rect.width()).clamp(0.0, 0.9999);
        let fy = ((p.y - rect.min.y) / rect.height()).clamp(0.0, 0.9999);
        ((fx * m.columns as f32) as u32, (fy * m.rows as f32) as u32)
    });
    if let Some((c, r)) = hovered_tile {
        ctx.tools.hover_tile = Some((c, r));
        let a = to_screen(f64::from(c) * 2048.0, f64::from(r) * 2048.0);
        let b = to_screen(f64::from(c + 1) * 2048.0, f64::from(r + 1) * 2048.0);
        painter.rect_stroke(
            egui::Rect::from_min_max(a, b),
            0.0,
            Stroke::new(1.0, with_alpha(palette.accent, 0.8)),
            egui::StrokeKind::Inside,
        );
    }
    if response.clicked()
        && let Some((c, r)) = hovered_tile
    {
        ctx.tools.selected = Some((c, r));
        let centre = shape.tile_rect(c, r);
        let cell = I64Vec2::new(centre.x0 + 1024, centre.y0 + 1024);
        let world = ctx.space.cell_corner_to_world(cell);
        for mut transform in &mut ctx.camera {
            transform.translation.x = world.x;
            transform.translation.y = world.y;
        }
        ctx.fit.auto = false;
    }
    let (on, off, other) = crate::macro_layer::shade_counts(m, ctx.settings.source);
    let other = if other > 0 { format!(" · {other} ambiguous or switching") } else { String::new() };
    ui.label(
        RichText::new(format!(
            "{} × {} tiles · {} ON · {} OFF{other} · white frame: your view · click to pin and centre",
            m.columns,
            m.rows,
            format_count(on as u64),
            format_count(off as u64)
        ))
        .size(11.0)
        .color(palette.text_muted),
    );
}

fn check_color(check: Check, palette: &Palette) -> Color32 {
    match check {
        Check::Pass => palette.success,
        Check::Fail => palette.error,
        Check::Pending | Check::NotSampled => palette.text_muted,
    }
}

fn validation(ui: &mut Ui, palette: &Palette, ctx: &PropsCtx) {
    let Some(info) = ctx.data.info.as_ref() else {
        ui.label(RichText::new("Reading the model…").weak());
        return;
    };
    let accent = palette.accent;
    let predicted = rgb32(colors::PREDICTED_ON);
    section(ui, palette, "Exact readouts vs prediction");
    card(ui, palette, |ui| {
        let clean = info.total_mismatches == 0 && info.total_ambiguous == 0;
        let (text, color) = if info.total_readouts == 0 {
            (format!("No readout yet: metagen 0 is read at t = {}.", format_count(PERIOD)), palette.text_muted)
        } else if clean {
            (
                format!(
                    "{} readout{}: every tile matched the prediction, no ambiguous core.",
                    format_count(info.total_readouts),
                    if info.total_readouts == 1 { "" } else { "s" }
                ),
                palette.success,
            )
        } else {
            (
                format!(
                    "{} readouts: {} mismatching and {} ambiguous tile readings in total.",
                    format_count(info.total_readouts),
                    format_count(info.total_mismatches),
                    format_count(info.total_ambiguous)
                ),
                palette.error,
            )
        };
        ui.label(RichText::new(text).size(13.0).strong().color(color));
        ui.label(RichText::new(metapixel::EXACT_LABEL).size(11.5).color(accent));
        ui.label(RichText::new(metapixel::ACCELERATED_LABEL).size(11.5).color(predicted));
        ui.label(
            RichText::new(format!(
                "Each readout counts the live cells of every tile's core at t = (n + 1)·{}: exactly 0 is OFF, exactly \
                 {} is ON, anything else is ambiguous and reported, never rounded.",
                format_count(PERIOD),
                format_count(READOUT_ON_COUNT)
            ))
            .size(11.5)
            .color(palette.text_muted),
        );
        if let Some((n, grid)) = &info.next_prediction {
            ui.label(
                RichText::new(format!(
                    "Prediction for metagen {}: {} ON tile{} (accelerated, not physical).",
                    format_count(*n),
                    format_count(grid.population()),
                    if grid.population() == 1 { "" } else { "s" }
                ))
                .size(11.5)
                .color(predicted),
            );
        }
    });
    if let (Some(first), Some(last)) = (info.records.first(), info.records.last()) {
        let marks: PlotPoints = info.records.iter().map(|r| [r.metagen as f64, r.exact_on as f64]).collect();
        let exact: PlotPoints = info.records.iter().map(|r| [r.metagen as f64, r.exact_on as f64]).collect();
        let pred: PlotPoints = info.records.iter().map(|r| [r.metagen as f64, r.predicted_on as f64]).collect();
        let top = info.records.iter().map(|r| r.exact_on.max(r.predicted_on)).max().unwrap_or(0).max(1) as f64;
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("ON tiles:").size(12.0).color(palette.text_muted));
            interface::widgets::legend_chip(ui, accent, "exact readout");
            interface::widgets::legend_chip(ui, predicted, "prediction (dashed)");
        });
        // Bounds from the data: one metagen of slack on each side, so a
        // single readout does not leave the axis to egui's defaults.
        Plot::new("mp_on_tiles")
            .height(130.0)
            .width(ui.available_width())
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_double_click_reset(false)
            .include_x(first.metagen as f64 - 0.5)
            .include_x(last.metagen as f64 + 0.5)
            .include_y(0.0)
            .include_y(top * 1.1)
            .x_axis_label("metagen")
            .show(ui, |plot| {
                plot.line(Line::new("ON tiles, exact readout", exact).color(accent).width(2.0));
                plot.points(Points::new("ON tiles, exact readout", marks).color(accent).radius(2.5));
                plot.line(Line::new("ON tiles, prediction", pred).color(predicted).style(LineStyle::dashed_dense()));
            });
        section(ui, palette, "Per metagen (newest first)");
        egui::ScrollArea::vertical().id_salt("mp_records").max_height(220.0).show(ui, |ui| {
            egui::Grid::new("mp_record_grid").num_columns(5).striped(true).spacing(vec2(10.0, 2.0)).show(ui, |ui| {
                for h in ["metagen", "read at t", "ON exact", "ON pred.", "result"] {
                    ui.label(RichText::new(h).size(11.0).strong().color(palette.text_muted));
                }
                ui.end_row();
                for r in info.records.iter().rev() {
                    ui.label(RichText::new(format_count(r.metagen)).size(12.0).monospace());
                    ui.label(RichText::new(format_count(r.generation)).size(12.0).monospace());
                    ui.label(RichText::new(format_count(r.exact_on)).size(12.0).monospace());
                    ui.label(RichText::new(format_count(r.predicted_on)).size(12.0).monospace());
                    if r.mismatches == 0 {
                        ui.label(RichText::new("agree").size(12.0).color(palette.success));
                    } else {
                        let tiles: Vec<String> =
                            r.mismatch_tiles.iter().map(|(c, rw)| format!("({c}, {rw})")).collect();
                        ui.label(
                            RichText::new(format!(
                                "{} mismatch{}",
                                r.mismatches,
                                if r.mismatches == 1 { "" } else { "es" }
                            ))
                            .size(12.0)
                            .color(palette.error),
                        )
                        .on_hover_text(format!(
                            "Tiles: {}{}{}",
                            tiles.join(", "),
                            if r.mismatches > tiles.len() { ", ..." } else { "" },
                            if r.ambiguous_count > 0 {
                                format!(
                                    "\nAmbiguous cores: {}",
                                    r.ambiguous
                                        .iter()
                                        .map(|a| format!("({}, {}) {} cells", a.col, a.row, a.count))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            } else {
                                String::new()
                            }
                        ));
                    }
                    ui.end_row();
                }
            });
        });
    }
    golden(ui, palette, ctx, info);
}

fn golden(ui: &mut Ui, palette: &Palette, ctx: &PropsCtx, info: &SimInfo) {
    section(ui, palette, "Golden values (bgolly)");
    let Some(table) = &ctx.data.golden else { return };
    let rows = info.initial.to_rows();
    let conway = info.active.programming == Programming::conway();
    match table {
        GoldenTable::Unavailable(e) => {
            notice(ui, palette, palette.warning, &format!("The golden table could not be read: {e}"));
        }
        GoldenTable::Loaded(cases) => match table.case_for(&rows).filter(|_| conway) {
            None => {
                let names: Vec<String> = cases
                    .iter()
                    .map(|c| {
                        let on: usize = c.model.iter().map(|r| r.chars().filter(|&ch| ch == 'O').count()).sum();
                        let w = c.model.first().map_or(0, |r| r.chars().count());
                        format!("{} ({}×{}, {} ON)", c.name, w, c.model.len(), on)
                    })
                    .collect();
                card(ui, palette, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Golly's bgolly golden table covers these B3/S23 Moore tilings: {}. This construction is \
                             not one of them; load the blinker or block scenario, or a 1×1 grid with one ON tile, to \
                             compare.",
                            names.join(", ")
                        ))
                        .size(12.0)
                        .color(palette.text_muted),
                    );
                });
            }
            Some(case) => {
                let now = info.tick;
                card(ui, palette, |ui| {
                    ui.label(
                        RichText::new(format!("Case `{}` from otca-spec.json ({} rows)", case.name, case.rows.len()))
                            .size(12.0)
                            .color(palette.text_muted),
                    );
                    egui::Grid::new("mp_golden").num_columns(5).striped(true).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
                        for h in ["t", "bgolly pop.", "this run", "pop. check", "cores check"] {
                            ui.label(RichText::new(h).size(11.0).strong().color(palette.text_muted));
                        }
                        ui.end_row();
                        for row in &case.rows {
                            // Only populations logged for this construction (never
                            // metric samples that may belong to a newer one).
                            let measured = ctx.data.populations.by_generation.get(&row.t).copied();
                            let pop_check = compare(row.population, measured, row.t, now);
                            let core_check = row.core_counts.as_ref().map(|expected| {
                                let read = (row.t / PERIOD)
                                    .checked_sub(1)
                                    .and_then(|n| info.records.iter().find(|r| r.metagen == n))
                                    .and_then(|r| r.core_counts.as_deref());
                                compare_cores(expected, read, row.t, now)
                            });
                            ui.label(RichText::new(format_count(row.t)).size(12.0).monospace());
                            ui.label(RichText::new(format_count(row.population)).size(12.0).monospace());
                            ui.label(
                                RichText::new(measured.map_or("—".to_string(), format_count)).size(12.0).monospace(),
                            );
                            ui.label(
                                RichText::new(pop_check.label()).size(12.0).color(check_color(pop_check, palette)),
                            );
                            match core_check {
                                Some(c) => ui.label(RichText::new(c.label()).size(12.0).color(check_color(c, palette))),
                                None => ui.label(RichText::new("—").size(12.0).color(palette.text_muted)),
                            };
                            ui.end_row();
                        }
                    });
                    ui.label(
                        RichText::new(
                            "Populations are the whole construction's live cells at t = k·35,328; cores are every \
                             tile's readout count. 'not sampled': the run passed that generation without stopping \
                             on it (use a step that divides 35,328, such as one metagen, or Advance to next readout).",
                        )
                        .size(11.0)
                        .color(palette.text_muted),
                    );
                });
            }
        },
    }
}

fn construction(ui: &mut Ui, palette: &Palette, ctx: &PropsCtx) {
    let Some(info) = ctx.data.info.as_ref() else { return };
    section(ui, palette, "Tiles");
    card(ui, palette, |ui| {
        let load = ctx.tiles.load_seconds.map(|s| format!("{:.0} ms", s * 1000.0)).unwrap_or_else(|| "—".into());
        kv_grid(
            ui,
            "mp_tiles",
            palette,
            &[
                ("Folder", ctx.tiles.dir.display().to_string()),
                ("Files", "OTCAmetapixel-OFF.rle, OTCAmetapixel-ON.rle".to_string()),
                (
                    "Unprogrammed",
                    format!(
                        "OFF {} cells, ON {} cells (validated)",
                        format_count(POPULATION_OFF_UNPROGRAMMED as u64),
                        format_count(POPULATION_ON_UNPROGRAMMED as u64)
                    ),
                ),
                (
                    "Programmed",
                    format!(
                        "OFF {} cells, ON {} cells",
                        format_count(info.programmed_populations.0 as u64),
                        format_count(info.programmed_populations.1 as u64)
                    ),
                ),
                ("Loaded in", load),
            ],
        );
    });
    section(ui, palette, "Programming");
    card(ui, palette, |ui| {
        let p = info.active.programming;
        ui.label(
            RichText::new(format!("Macro rule {} · neighbourhood {}", p.rule, p.neighbourhood)).size(12.5).strong(),
        );
        let added = programming_cells(p);
        ui.label(
            RichText::new(if added.is_empty() {
                "No rule-table cell added (the rule has no digit).".to_string()
            } else {
                format!(
                    "Added to every tile (completing the eater of each digit): {}",
                    added.iter().map(|(l, (x, y))| format!("{l} at ({x}, {y})")).collect::<Vec<_>>().join(", ")
                )
            })
            .size(12.0),
        );
        let removed = removed_fuses(p);
        ui.label(
            RichText::new(if removed.is_empty() {
                "All eight fuses present: every tile hears all eight neighbours.".to_string()
            } else {
                format!(
                    "Removed from every tile: {}",
                    removed
                        .iter()
                        .map(|(object, deaf, cells)| format!(
                            "{} {} ({cells} cells, deaf to {})",
                            object.name(),
                            if object.is_corner() { "corner LWSS" } else { "edge block" },
                            deaf.name()
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .size(12.0),
        );
        ui.label(
            RichText::new("Whatever is programmed, every physical cell follows B3/S23.")
                .size(11.5)
                .color(palette.text_muted),
        );
    });
    section(ui, palette, "Build and Hashlife");
    card(ui, palette, |ui| {
        let shape = GridShape::new(info.active.width, info.active.height);
        let (w, h) = shape.size_cells();
        kv_grid(
            ui,
            "mp_build",
            palette,
            &[
                (
                    "Grid",
                    format!(
                        "{} × {} tiles = {} × {} cells",
                        info.active.width,
                        info.active.height,
                        format_count(w),
                        format_count(h)
                    ),
                ),
                ("Live cells", format_count(info.population)),
                ("Distinct squares", format_count(info.build.distinct_squares as u64)),
                (
                    "Macrocell document",
                    format!(
                        "{} nodes, {}",
                        format_count(info.build.document_nodes as u64),
                        format_bytes(info.build.document_bytes as u64)
                    ),
                ),
                (
                    "Hashlife nodes",
                    format!(
                        "{} (limit {})",
                        format_count(info.node_count as u64),
                        format_count(info.node_limit as u64)
                    ),
                ),
                ("Hashlife memory", format_bytes(info.memory_bytes as u64)),
                ("Generation", format_count(info.tick)),
            ],
        );
        ui.label(
            RichText::new(
                "Tiles are OR-ed at pitch 2,048 (neighbours overlap by 5 cells). Every distinct 2,048 × 2,048 square \
                 is built once and shared, so the quadtree is a small DAG and Hashlife memoises each square's future \
                 once for all its copies.",
            )
            .size(11.5)
            .color(palette.text_muted),
        );
    });
    if let Some(shape) = current_shape(&ctx.data)
        && shape.tile_count() > 4096
    {
        notice(
            ui,
            palette,
            palette.warning,
            "Large grids: a metagen can take a second or more; the window stays responsive.",
        );
    }
    section(ui, palette, "Credits");
    card(ui, palette, |ui| {
        ui.label(RichText::new(CREDITS).size(12.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use metapixel::MacroTile;

    #[test]
    fn next_state_names_the_deciding_digit() {
        let life = Programming::conway();
        assert_eq!(next_state(life, false, 3), (true, "B3 is in the rule: born".to_string()));
        assert!(!next_state(life, false, 2).0);
        assert_eq!(next_state(life, true, 2), (true, "S2 is in the rule: survives".to_string()));
        assert!(!next_state(life, true, 4).0);
    }

    #[test]
    fn readout_grid_uses_exact_states_and_as_built_before() {
        let tile = |exact, accelerated| MacroTile { exact, accelerated, mismatch: false, population: 0, live: None };
        let mut view = MacroView {
            columns: 2,
            rows: 1,
            exact_metagen: Some(0),
            exact_generation: Some(PERIOD),
            accelerated_metagen: 0,
            tiles: vec![tile(Some(TileReading::On), false), tile(Some(TileReading::Ambiguous(4)), true)],
            mismatch_count: 1,
            ambiguous_count: 1,
            exact_label: String::new(),
            accelerated_label: String::new(),
        };
        assert_eq!(readout_grid(&view).unwrap().to_rows(), ["O."], "ambiguous tiles are not ON");
        view.tiles = vec![tile(None, false), tile(None, true)];
        assert_eq!(readout_grid(&view).unwrap().to_rows(), [".O"], "before the first readout: as built");
    }

    #[test]
    fn texts() {
        assert_eq!(reading_text(Some(TileReading::On)).0, "ON (core 29,795 cells)");
        assert!(reading_text(Some(TileReading::Ambiguous(3))).0.starts_with("AMBIGUOUS"));
        assert_eq!(live_text(LiveReading::Off, Some(0)), "OFF · core 0 cells");
        assert!(live_text(LiveReading::Transitioning(100), None).contains("switching"));
        assert_eq!(PropertiesTab::ALL.len(), 3);
    }
}
