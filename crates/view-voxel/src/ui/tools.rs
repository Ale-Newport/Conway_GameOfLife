//! Left "Tools" panel: tool, rule editor, seeding, cut, region, display,
//! camera and shortcuts.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui::{self, RichText};
use interface::icons::{Icon, icon_button};
use interface::{ModeRegistry, Palette, SessionStatus, ThemeSettings};
use rendering::{OrbitCameraRig, OrbitPreset};
use simulation_core::{ParamKind, ParamValue, SessionControl};
use voxel_life::{Axis, Bounds, Mirror, VoxelCommand};

use super::{SeedDraft, SeedValues, UiState, collapsible, fitted, hint, mono, notice, section, segmented};
use crate::MODE;
use crate::requests::{ProbeState, WorldState};
use crate::scene::{CameraAction, VoxelCamera};
use crate::session::VoxelRunner;
use crate::settings::{AgeScale, ColorMode, RENDER_CAPS, Tool, Transparency, ViewSettings, axis_range};
use crate::shortcuts::SHORTCUTS;

#[cfg(test)]
/// Non-ASCII characters used in this panel (checked against egui's fonts).
pub(crate) const GLYPHS: &str = "–·×…³";

/// Named rule presets of the rule editor: (name, rule, note).
pub(crate) const RULE_PRESETS: [(&str, &str, &str); 13] = [
    ("Bays 4555", "4555", "Bays' 3D Life candidate (1987): survive 4–5, born 5. Has a period-4 glider."),
    ("Bays 5766", "5766", "Bays' 3D Life candidate (1987): survive 5–7, born 6. Has a period-4 glider."),
    ("Bays 5655", "5655", "Survive 5–6, born 5 (Bays 1994)."),
    ("Bays 6855", "6855", "Survive 6–8, born 5 (Bays 1994)."),
    ("Rokicki ship", "3D4,7/5,8", "Golly 3D rule with a period-10 ship moving 10 voxels per period."),
    ("Softology 445", "4/4/5/M", "Survive 4, born 4, 5 states."),
    ("Clouds 1", "13-26/13-14,17-19/2/M", "Smoothing rule: needs a dense fill of the volume."),
    ("Amoeba", "9-26/5-7,12-13,15/5/M", "Grows rounded blobs, 5 states."),
    ("Builder 1", "2,6,9/4,6,8-9/10/M", "Grows, then partially decays; 10 states."),
    ("Pyroclastic", "4-7/6-8/10/M", "Expanding with long decay trails; 10 states."),
    ("Crystal growth 1", "0-6/1,3/2/N", "Von Neumann crystal from a single voxel."),
    ("Single-point replication", "/1/2/M", "Born with exactly 1 alive neighbour, nothing survives."),
    ("B3/S23 in 3D", "B3/S23", "Conway's numbers on 26 neighbours: a different automaton from 2D Life."),
];

/// Resources the Tools panel reads and writes.
#[derive(SystemParam)]
pub(crate) struct ToolsParams<'w, 's> {
    runner: Option<Res<'w, VoxelRunner>>,
    status: Res<'w, SessionStatus>,
    world: Res<'w, WorldState>,
    settings: ResMut<'w, ViewSettings>,
    ui_state: ResMut<'w, UiState>,
    probe: Res<'w, ProbeState>,
    theme: Res<'w, ThemeSettings>,
    registry: Res<'w, ModeRegistry>,
    camera: MessageWriter<'w, CameraAction>,
    rigs: Query<'w, 's, &'static OrbitCameraRig, With<VoxelCamera>>,
}

/// The Tools panel.
pub(crate) fn tools_panel(InMut(ui): InMut<egui::Ui>, mut p: ToolsParams) {
    let palette = Palette::new(p.theme.kind, p.registry.accent(MODE));
    notice(ui, &palette, palette.accent, |ui| {
        ui.label(RichText::new("3D rules, not Conway's Life").strong().color(palette.text));
        hint(
            ui,
            &palette,
            "Every voxel counts 26 neighbours (Moore) or 6 (von Neumann). Conway's B3/S23 typed here is a \
             different, three-dimensional automaton; Bays' 4555 and 5766 are the classic 3D Life rules.",
        );
    });
    tool_section(ui, &palette, &mut p);
    let Some(runner) = p.runner.as_deref().cloned() else {
        hint(ui, &palette, "Starting the simulation…");
        return;
    };
    collapsible(ui, "Rule", true, |ui| rule_section(ui, &palette, &mut p, &runner));
    collapsible(ui, "Cut", true, |ui| cut_section(ui, &palette, &mut p));
    collapsible(ui, "Seed", true, |ui| seed_section(ui, &palette, &mut p, &runner));
    collapsible(ui, "Region", false, |ui| region_section(ui, &palette, &mut p));
    collapsible(ui, "Display", true, |ui| display_section(ui, &palette, &mut p));
    collapsible(ui, "Camera", false, |ui| camera_section(ui, &palette, &mut p));
    collapsible(ui, "Shortcuts", false, |ui| shortcuts_section(ui, &palette));
}

fn tool_section(ui: &mut egui::Ui, palette: &Palette, p: &mut ToolsParams) {
    section(ui, palette, "Tool");
    let mut tool = p.settings.tool;
    if segmented(ui, palette, &mut tool, &[(Tool::Inspect, "Inspect"), (Tool::Place, "Place"), (Tool::Erase, "Erase")])
    {
        p.settings.tool = tool;
    }
    let text = match tool {
        Tool::Inspect => "Click a voxel to read its state, age, alive neighbours and next state (Properties tab).",
        Tool::Place => {
            "Click a face to add an alive voxel next to it, or the floor of an empty area. Shift+click erases."
        }
        Tool::Erase => "Click a voxel to empty it.",
    };
    hint(ui, palette, text);
    if tool != Tool::Inspect {
        hint(ui, palette, "Edits are undoable (Ctrl/Cmd+Z). Drag to orbit as usual.");
    }
    ui.add_space(4.0);
}

fn rule_section(ui: &mut egui::Ui, palette: &Palette, p: &mut ToolsParams, runner: &VoxelRunner) {
    let current_text = p.status.get().and_then(|s| s.params.get("rule")).and_then(|v| v.as_str()).map(str::to_string);
    let current_rule = p.world.info.as_ref().map(|i| i.rule);
    let editor = &mut p.ui_state.rule_editor;
    if let Some(current) = &current_text {
        editor.sync(current);
        ui.horizontal(|ui| {
            ui.label(RichText::new("In effect").color(palette.text_muted));
            ui.label(mono(current.as_str(), 13.0).color(palette.text));
        });
    }
    let response = ui.add(
        egui::TextEdit::singleline(&mut editor.text)
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .hint_text("5766, 4/4/5/M, B6/S567, 3D5..7/6"),
    );
    if response.changed() {
        editor.edited = true;
    }
    let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    match editor.parsed() {
        Ok(rule) => {
            let n = rule.neighbourhood();
            ui.label(
                RichText::new(format!("Valid · {} ({} neighbours) · {} states", n.label(), n.size(), rule.states()))
                    .size(11.5)
                    .color(palette.success),
            );
            egui::Grid::new("view_voxel_rule_forms").num_columns(2).spacing([8.0, 1.0]).show(ui, |ui| {
                let mut row = |name: &str, value: Option<String>| {
                    if let Some(value) = value {
                        ui.label(RichText::new(name).size(11.0).color(palette.text_muted));
                        ui.label(mono(value, 11.5));
                        ui.end_row();
                    }
                };
                row("Softology", Some(rule.to_softology()));
                row("Letters", Some(rule.to_letters()));
                row("Bays", rule.to_bays());
                row("Golly", rule.to_golly());
            });
        }
        Err(e) => {
            ui.label(RichText::new(e).size(11.5).color(palette.error));
        }
    }
    let can_apply = editor.can_apply(current_rule.as_ref());
    let mut apply = submitted && can_apply;
    ui.horizontal(|ui| {
        let button = egui::Button::new(RichText::new("Apply rule").strong().color(if can_apply {
            palette.on_accent
        } else {
            palette.text_muted
        }))
        .fill(if can_apply { palette.accent } else { palette.surface_hi });
        apply |= ui.add_enabled(can_apply, button).clicked();
        if ui.add_enabled(editor.edited, egui::Button::new("Revert")).clicked() {
            editor.revert();
        }
    });
    if apply && let Ok(rule) = editor.parsed() {
        runner.0.set_param("rule", ParamValue::Text(rule.to_string()));
        editor.applied();
    }
    hint(ui, palette, "Applies from the next generation; voxels in states the new rule lacks are emptied.");
    ui.add_space(4.0);
    egui::CollapsingHeader::new(RichText::new("Presets").size(12.5)).id_salt("view_voxel_rule_presets").show(
        ui,
        |ui| {
            egui::Grid::new("view_voxel_presets").num_columns(2).spacing([8.0, 2.0]).striped(true).show(ui, |ui| {
                for (name, rule, note) in RULE_PRESETS {
                    if ui.selectable_label(false, RichText::new(name).size(12.5)).on_hover_text(note).clicked() {
                        editor.load(rule);
                    }
                    ui.label(mono(rule, 11.5).color(palette.text_muted));
                    ui.end_row();
                }
            });
            hint(ui, palette, "A preset fills the editor; press Apply rule to run it.");
        },
    );
}

fn seed_section(ui: &mut egui::Ui, palette: &Palette, p: &mut ToolsParams, runner: &VoxelRunner) {
    let Some(status) = p.status.get() else { return };
    let Some(current) = SeedValues::from_params(&status.params) else { return };
    let interacting = ui.ctx().dragged_id().is_some();
    p.ui_state.seed.sync(&status.params, interacting);
    let mut draft = p.ui_state.seed.values.clone().unwrap_or_else(|| current.clone());
    let max_side = ["size_x", "size_y", "size_z"]
        .iter()
        .filter_map(|k| status.params.get(k).and_then(ParamValue::as_i64))
        .max()
        .unwrap_or(64)
        .clamp(1, largest_cube_side(voxel_life::MAX_RANDOM_VOXELS));
    fitted(ui, "seed", 175.0, |ui| {
        egui::Grid::new("view_voxel_seed").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Cube side");
            ui.add(egui::Slider::new(&mut draft.cube_side, 1..=max_side).suffix(" voxels"));
            ui.end_row();
            ui.label("Density");
            ui.add(egui::Slider::new(&mut draft.density, 0.0..=1.0).step_by(0.01).fixed_decimals(2));
            ui.end_row();
            ui.label("Symmetry");
            combo(ui, "view_voxel_mirror", mirror_label(&draft.mirror), |ui| {
                for mirror in Mirror::ALL {
                    ui.selectable_value(&mut draft.mirror, mirror.key().to_string(), mirror_label(mirror.key()));
                }
            });
            ui.end_row();
            ui.label("Seed");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut draft.seed).range(0..=i64::MAX).speed(1.0));
                if icon_button(ui, Icon::Dice, "New random seed", true, false, palette).clicked() {
                    draft.seed = fresh_seed(draft.seed);
                }
            });
            ui.end_row();
            ui.label("After reset");
            let options: Vec<(String, String)> = match runner.0.descriptor().param("init").map(|s| &s.kind) {
                Some(ParamKind::Choice { options }) => {
                    options.iter().map(|o| (o.value.clone(), o.label.clone())).collect()
                }
                _ => Vec::new(),
            };
            let selected =
                options.iter().find(|(k, _)| *k == draft.init).map_or(draft.init.clone(), |(_, l)| l.clone());
            combo(ui, "view_voxel_init", selected, |ui| {
                for (key, label) in &options {
                    ui.selectable_value(&mut draft.init, key.clone(), label);
                }
            })
            .on_hover_text("Initial state the Reset button (R) rebuilds");
            ui.end_row();
        });
    });
    let seed_now = ui
        .add(
            egui::Button::new(RichText::new("Seed now").strong().color(palette.on_accent))
                .fill(palette.accent)
                .min_size(egui::vec2(110.0, 26.0)),
        )
        .on_hover_text("Empty the world and draw a centred random cube with these settings (undoable).")
        .clicked();
    if (!interacting || seed_now) && p.ui_state.seed.sent.as_ref() != Some(&draft) {
        for (key, value) in SeedDraft::changes(&draft, &current) {
            runner.0.set_param(key, value);
        }
        p.ui_state.seed.sent = Some(draft.clone());
    }
    if seed_now {
        let seed = u64::try_from(draft.seed).unwrap_or_default();
        runner.0.edit(VoxelCommand::Randomize { seed: Some(seed) });
    }
    p.ui_state.seed.values = Some(draft);
    hint(
        ui,
        palette,
        "Seed now replaces the world with a random cube. Reset (R) rebuilds the initial state chosen above \
         from these settings and the seed.",
    );
}

/// The part of `layers` between the planes of a cut.
fn cut_slab(layers: Bounds, range: voxel_life::SliceRange) -> Bounds {
    let mut slab = layers;
    let a = range.axis.index();
    slab.min[a] = range.min.max(layers.min[a]);
    slab.max[a] = range.max.min(layers.max[a]);
    slab
}

/// Largest cube side whose volume is at most `max_voxels` (the model
/// refuses random cubes of more than `MAX_RANDOM_VOXELS` voxels).
fn largest_cube_side(max_voxels: u64) -> i64 {
    let mut side = (max_voxels as f64).cbrt().round() as u64;
    while side > 1 && side.saturating_pow(3) > max_voxels {
        side -= 1;
    }
    while (side + 1).saturating_pow(3) <= max_voxels {
        side += 1;
    }
    side as i64
}

/// A combo box as wide as the room left in the row (at most 220 points),
/// truncating a long selection instead of widening the panel.
fn combo(ui: &mut egui::Ui, id: &str, selected: String, add: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let width = ui.available_width().clamp(90.0, 220.0);
    egui::ComboBox::from_id_salt(id).selected_text(selected).width(width).truncate().show_ui(ui, add).response
}

fn mirror_label(key: &str) -> String {
    match key {
        "none" => "None".to_string(),
        other => format!("Mirror {}", other.to_uppercase()),
    }
}

/// A new seed derived from the clock (splitmix64 finaliser).
fn fresh_seed(previous: i64) -> i64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut z = nanos ^ (previous as u64).rotate_left(17) ^ 0x9E37_79B9_7F4A_7C15;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 1) as i64 % 1_000_000
}

fn cut_section(ui: &mut egui::Ui, palette: &Palette, p: &mut ToolsParams) {
    let layers = p.world.layers();
    let mut settings = p.settings.clone();
    let mut enabled = settings.cut.enabled;
    if ui.checkbox(&mut enabled, "Cut the volume").changed() {
        settings.set_cut_enabled(enabled, layers);
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Axis").color(palette.text_muted));
        let mut axis = settings.cut.axis;
        if segmented(ui, palette, &mut axis, &[(Axis::X, "x"), (Axis::Y, "y"), (Axis::Z, "z (up)")]) {
            settings.set_cut_axis(axis, layers);
            settings.set_cut_enabled(true, layers);
        }
    });
    let (lo, hi) = axis_range(layers, settings.cut.axis);
    let mut cut = settings.cut;
    fitted(ui, "cut", 130.0, |ui| {
        egui::Grid::new("view_voxel_cut").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("From layer");
            let from = ui.add(egui::Slider::new(&mut cut.min, lo..=hi));
            ui.end_row();
            ui.label("To layer");
            let to = ui.add(egui::Slider::new(&mut cut.max, lo..=hi));
            ui.end_row();
            if from.changed() {
                cut.max = cut.max.max(cut.min);
                cut.enabled = true;
            }
            if to.changed() {
                cut.min = cut.min.min(cut.max);
                cut.enabled = true;
            }
        });
    });
    settings.cut = cut;
    ui.horizontal_wrapped(|ui| {
        if ui.button("Lower half").clicked() {
            settings.cut = crate::settings::CutSettings { enabled: true, min: lo, max: lo + (hi - lo) / 2, ..cut };
        }
        if ui.button("Single layer").on_hover_text("Keep only the upper plane's layer").clicked() {
            settings.cut = crate::settings::CutSettings { enabled: true, min: cut.max, max: cut.max, ..cut };
        }
        if ui.button("All layers").clicked() {
            settings.cut = crate::settings::CutSettings { enabled: true, min: lo, max: hi, ..cut };
        }
    });
    fitted(ui, "sweep", 175.0, |ui| {
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut settings.sweep, "Sweep")
                .on_hover_text("Advance the upper plane automatically, wrapping to the lower plane")
                .changed()
                && settings.sweep
            {
                settings.set_cut_enabled(true, layers);
            }
            ui.add_enabled(
                settings.sweep,
                egui::Slider::new(&mut settings.sweep_rate, 1.0..=30.0).suffix(" layers/s").fixed_decimals(0),
            );
        });
    });
    hint(
        ui,
        palette,
        "Only layers between the planes are drawn; voxels cut open show their inside faces. [ and ] move the \
         upper plane, Shift+[ and Shift+] the lower one.",
    );
    if settings != *p.settings {
        *p.settings = settings;
    }
}

fn region_section(ui: &mut egui::Ui, palette: &Palette, p: &mut ToolsParams) {
    let layers = p.world.layers();
    let mut region = p.settings.region;
    ui.checkbox(&mut region.enabled, "Count voxels in a box");
    egui::Grid::new("view_voxel_region").num_columns(4).spacing([6.0, 4.0]).show(ui, |ui| {
        for (a, name) in ["x", "y", "z"].iter().enumerate() {
            ui.label(*name);
            ui.add(egui::DragValue::new(&mut region.min[a]).range(layers.min[a]..=layers.max[a]));
            ui.label(RichText::new("to").color(palette.text_muted));
            ui.add(egui::DragValue::new(&mut region.max[a]).range(layers.min[a]..=layers.max[a]));
            ui.end_row();
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(p.world.population.is_some(), egui::Button::new("Fit population")).clicked()
            && let Some(population) = p.world.population
        {
            region.set(population);
            region.enabled = true;
        }
        if ui.button("Whole world").clicked() {
            region.set(layers);
            region.enabled = true;
        }
        if ui
            .add_enabled(p.probe.pinned.is_some(), egui::Button::new("Around probe"))
            .on_hover_text("9³ box centred on the inspected voxel")
            .clicked()
            && let Some(pos) = p.probe.pinned
        {
            region.set(Bounds { min: pos.map(|c| c - 4), max: pos.map(|c| c + 4) });
            region.clamp_to(layers);
            region.enabled = true;
        }
        let cut = p.settings.cut;
        if ui
            .add_enabled(cut.enabled, egui::Button::new("Cut slab"))
            .on_hover_text("The layers between the cut planes, across the whole world")
            .clicked()
        {
            region.set(cut_slab(layers, cut.range()));
            region.enabled = true;
        }
    });
    hint(ui, palette, "Counts per state appear in the Properties tab and the overlay (key M toggles the box).");
    if region != p.settings.region {
        p.settings.region = region;
    }
}

fn display_section(ui: &mut egui::Ui, palette: &Palette, p: &mut ToolsParams) {
    let mut settings = p.settings.clone();
    ui.label(RichText::new("Colour by").color(palette.text_muted));
    segmented(
        ui,
        palette,
        &mut settings.color_mode,
        &[(ColorMode::State, "State"), (ColorMode::Age, "Age"), (ColorMode::Height, "Height")],
    );
    if settings.color_mode == ColorMode::Age {
        let mut auto = settings.age_scale == AgeScale::Auto;
        let mut top = match settings.age_scale {
            AgeScale::Fixed(top) => top,
            AgeScale::Auto => 100,
        };
        fitted(ui, "age", 210.0, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut auto, "Auto").on_hover_text("Top of the ramp = oldest voxel shown");
                ui.add_enabled(!auto, egui::Slider::new(&mut top, 2..=5000).logarithmic(true).text("generations"));
            });
        });
        settings.age_scale = if auto { AgeScale::Auto } else { AgeScale::Fixed(top) };
    }
    ui.add_space(4.0);
    ui.label(RichText::new("Transparency").color(palette.text_muted));
    segmented(
        ui,
        palette,
        &mut settings.transparency,
        &[(Transparency::Opaque, "Opaque"), (Transparency::GhostDying, "Ghost dying"), (Transparency::XRay, "X-ray")],
    );
    if settings.transparency != Transparency::Opaque {
        fitted(ui, "opacity", 125.0, |ui| {
            ui.add(egui::Slider::new(&mut settings.opacity, 0.05..=0.9).text("opacity").fixed_decimals(2));
        });
    }
    if settings.transparency == Transparency::XRay {
        hint(
            ui,
            palette,
            "The model lists only voxels with an exposed face, so X-ray shows the outer layers of solid \
             regions; cut the volume to look deeper.",
        );
    }
    ui.add_space(4.0);
    ui.checkbox(&mut settings.depth_shading, "Depth shading")
        .on_hover_text("Darken voxels with many visible neighbours (crevices); display only");
    ui.checkbox(&mut settings.checker, "Cell checker")
        .on_hover_text("Odd voxels slightly darker (3D checkerboard), so single voxels stay distinct; display only");
    ui.checkbox(&mut settings.fog, "Distance fog").on_hover_text("Fade far voxels towards the background");
    ui.checkbox(&mut settings.show_bounds, "World box");
    ui.checkbox(&mut settings.show_floor, "Floor and grid");
    ui.checkbox(&mut settings.show_axes, "Axes (x red, y green, z blue)");
    ui.horizontal(|ui| {
        ui.label("Draw at most");
        let selected = format!("{} voxels", super::count(settings.render_cap as u64));
        combo(ui, "view_voxel_cap", selected, |ui| {
            for cap in RENDER_CAPS {
                ui.selectable_value(&mut settings.render_cap, cap, format!("{} voxels", super::count(cap as u64)));
            }
        })
        .on_hover_text("Most surface voxels requested from the model per view");
    });
    if settings != *p.settings {
        *p.settings = settings;
    }
}

fn camera_section(ui: &mut egui::Ui, palette: &Palette, p: &mut ToolsParams) {
    let current = p.rigs.iter().next().map(|r| r.preset).unwrap_or_default();
    let mut preset = current;
    ui.label(RichText::new("Preset").color(palette.text_muted));
    ui.horizontal_wrapped(|ui| {
        if segmented(
            ui,
            palette,
            &mut preset,
            &[
                (OrbitPreset::FreeOrbit, "Orbit"),
                (OrbitPreset::Isometric, "Isometric"),
                (OrbitPreset::Aerial, "Aerial"),
                (OrbitPreset::Strategic, "Strategic"),
            ],
        ) {
            p.camera.write(CameraAction::Preset(preset));
        }
    });
    ui.label(RichText::new("View").color(palette.text_muted));
    ui.horizontal_wrapped(|ui| {
        for (label, action) in [
            ("Front", CameraAction::Front),
            ("Side", CameraAction::Side),
            ("Top", CameraAction::Top),
            ("3/4", CameraAction::Corner),
        ] {
            if ui.button(label).clicked() {
                p.camera.write(action);
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Frame world").on_hover_text("F").clicked() {
            p.camera.write(CameraAction::FrameWorld);
        }
        if ui.button("Frame population").on_hover_text("G").clicked() {
            p.camera.write(CameraAction::FramePopulation);
        }
    });
    let mut turntable = p.settings.turntable;
    let mut speed = p.settings.turntable_speed;
    fitted(ui, "turntable", 200.0, |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut turntable, "Turntable");
            ui.add_enabled(turntable, egui::Slider::new(&mut speed, 0.05..=1.5).suffix(" rad/s").fixed_decimals(2));
        });
    });
    if (turntable, speed) != (p.settings.turntable, p.settings.turntable_speed) {
        p.settings.turntable = turntable;
        p.settings.turntable_speed = speed;
    }
    hint(
        ui,
        palette,
        "Left drag orbits, right drag pans, the wheel zooms. W A S D or the arrows pan, Q / E rotate. Strategic \
         orbits with the middle button.",
    );
}

fn shortcuts_section(ui: &mut egui::Ui, palette: &Palette) {
    egui::Grid::new("view_voxel_shortcuts").num_columns(2).spacing([10.0, 3.0]).striped(true).show(ui, |ui| {
        for shortcut in SHORTCUTS {
            ui.label(mono(shortcut.label, 12.0).color(palette.text));
            ui.label(RichText::new(shortcut.description).size(12.0));
            ui.end_row();
        }
    });
    hint(ui, palette, "The shell's shortcuts (Space, N, R, Ctrl/Cmd+Z, Tab…) are listed under Help (F1).");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_parse() {
        for (name, rule, note) in RULE_PRESETS {
            assert!(voxel_life::Rule3D::parse(rule).is_ok(), "{name}: {rule}");
            assert!(!note.is_empty());
        }
    }

    #[test]
    fn preset_notes_match_the_rules() {
        let parse = |s: &str| voxel_life::Rule3D::parse(s).expect("valid");
        assert_eq!(parse("5655").survive().iter().collect::<Vec<_>>(), vec![5, 6]);
        assert_eq!(parse("5655").birth().iter().collect::<Vec<_>>(), vec![5]);
        assert_eq!(parse("6855").survive().iter().collect::<Vec<_>>(), vec![6, 7, 8]);
        assert_eq!(parse("4555").survive().iter().collect::<Vec<_>>(), vec![4, 5]);
        assert!(parse("/1/2/M").survive().is_empty());
    }

    #[test]
    fn fresh_seeds_vary_and_stay_in_range() {
        let a = fresh_seed(1);
        let b = fresh_seed(a);
        assert!((0..1_000_000).contains(&a) && (0..1_000_000).contains(&b));
        assert_eq!(mirror_label("xyz"), "Mirror XYZ");
        assert_eq!(mirror_label("none"), "None");
    }

    #[test]
    fn cut_slab_spans_the_other_axes() {
        let layers = Bounds { min: [0, 0, 0], max: [63, 31, 15] };
        let slab = cut_slab(layers, voxel_life::SliceRange::new(Axis::Y, 4, 40));
        assert_eq!(slab, Bounds { min: [0, 4, 0], max: [63, 31, 15] }, "clipped to the world");
    }

    #[test]
    fn random_cube_side_respects_the_model_limit() {
        assert_eq!(largest_cube_side(voxel_life::MAX_RANDOM_VOXELS), 256);
        assert_eq!(largest_cube_side(26), 2);
        assert_eq!(largest_cube_side(27), 3);
        assert_eq!(largest_cube_side(1), 1);
    }
}
