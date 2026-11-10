//! Entity-type editor: every field of [`EntityType`] — identity and
//! appearance, properties with ranges, states, carried resources, movement,
//! energy, reproduction, lifecycle and the initial population.

use bevy_egui::egui::{self, RichText, Ui};
use interface::Palette;
use interface::theme::color32_from_rgba;
use rule_engine::ItemRef;
use rule_engine::def::{CarriedResource, Distribution, EntityType, MovementDef, PropertyDef, ReproductionMode};

use super::widgets::{
    confirm_delete, diagnostics_list, heading, hint, list_row, name_combo, name_field, num_f32, num_f64, num_u32,
    num_u64, split,
};
use super::{ClEditor, EditorParams, EditorTab};
use crate::colors::{GLYPHS, new_color};
use crate::draft::{rename_entity_type, rename_property, rename_state, unique_name};

/// Draws the tab.
pub fn show(ui: &mut Ui, p: &mut EditorParams, palette: &Palette) {
    let counts: Vec<(String, u32)> =
        p.data.view.as_ref().map(|v| v.legend.iter().map(|l| (l.name.clone(), l.count)).collect()).unwrap_or_default();
    split(
        ui,
        "cl_entities",
        210.0,
        &mut *p.editor,
        |ui, editor| list(ui, editor, &counts, palette),
        |ui, editor| {
            if editor.draft.entity_types.is_empty() {
                hint(ui, "No entity types yet. Add one on the left, then give it a way to gain energy.", palette);
                return;
            }
            detail(ui, editor, palette);
        },
    );
}

fn list(ui: &mut Ui, editor: &mut ClEditor, counts: &[(String, u32)], palette: &Palette) {
    heading(ui, "Entity types", palette);
    let mut clicked = None;
    for (i, t) in editor.draft.entity_types.iter().enumerate() {
        let live = counts.iter().find(|(n, _)| *n == t.name).map(|(_, c)| format!("{c} alive")).unwrap_or_default();
        let label = format!("{} {}", crate::glyphs::shown_icon(ui.ctx(), &t.icon, &t.name), t.name);
        let errors = editor.diagnostics.iter().any(|d| d.is_error() && d.item == Some(ItemRef::EntityType(i as u16)));
        let label = if errors { format!("⚠ {label}") } else { label };
        if list_row(ui, editor.sel_type == i, Some(color32_from_rgba(t.color)), &label, &live) {
            clicked = Some(i);
        }
    }
    if let Some(i) = clicked {
        editor.sel_type = i;
    }
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("+ Add type").clicked() {
            let taken = editor.namespace();
            let name = unique_name("organism", taken.iter().map(String::as_str));
            let mut t = EntityType::new(&name);
            t.color = new_color(editor.draft.entity_types.len());
            t.icon = "⬣".into();
            t.movement = MovementDef::RandomWalk { speed: 0.5 };
            t.initial.count = 20;
            t.energy.metabolism = 0.05;
            editor.draft.entity_types.push(t);
            editor.sel_type = editor.draft.entity_types.len() - 1;
            editor.touch();
        }
        let has = editor.sel_type < editor.draft.entity_types.len();
        if ui.add_enabled(has, egui::Button::new("Duplicate")).clicked() {
            let taken = editor.namespace();
            let mut t = editor.draft.entity_types[editor.sel_type].clone();
            t.name = unique_name(&format!("{}_copy", t.name), taken.iter().map(String::as_str));
            editor.draft.entity_types.insert(editor.sel_type + 1, t);
            editor.sel_type += 1;
            editor.touch();
        }
    });
    ui.horizontal(|ui| {
        let i = editor.sel_type;
        let n = editor.draft.entity_types.len();
        if ui.add_enabled(i > 0 && i < n, egui::Button::new("Up")).on_hover_text("Move up (requires a reset)").clicked()
        {
            editor.draft.entity_types.swap(i, i - 1);
            editor.sel_type -= 1;
            editor.touch();
        }
        if ui.add_enabled(i + 1 < n, egui::Button::new("Down")).on_hover_text("Move down (requires a reset)").clicked()
        {
            editor.draft.entity_types.swap(i, i + 1);
            editor.sel_type += 1;
            editor.touch();
        }
    });
    ui.add_space(8.0);
    hint(
        ui,
        "Adding, removing, renaming or reordering types (and their states, properties or carried resources) \
         requires a reset; everything else applies from the next tick.",
        palette,
    );
}

fn detail(ui: &mut Ui, editor: &mut ClEditor, palette: &Palette) {
    let i = editor.sel_type.min(editor.draft.entity_types.len() - 1);
    let namespace = editor.namespace();
    let resources = editor.resource_names();
    let (world_w, world_h) = (editor.draft.world.width as f32, editor.draft.world.height as f32);
    let max_entities = editor.draft.world.max_entities;
    let diags = editor.diagnostics_of(ItemRef::EntityType(i as u16));
    let mut changed = false;
    let old_name = editor.draft.entity_types[i].name.clone();

    // ---- identity -------------------------------------------------------
    heading(ui, "Identity and appearance", palette);
    let mut rename = None;
    egui::Grid::new(("cl_type_identity", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        let taken: Vec<&str> = namespace.iter().map(String::as_str).collect();
        rename = name_field(ui, egui::Id::new(("cl_type_name", i)), &old_name, &taken, true);
        ui.end_row();
        let t = &mut editor.draft.entity_types[i];
        ui.label("Description");
        changed |= ui.add(egui::TextEdit::multiline(&mut t.description).desired_rows(2).desired_width(320.0)).changed();
        ui.end_row();
        ui.label("Colour");
        changed |= crate::editor::widgets::color_edit(ui, &mut t.color);
        ui.end_row();
        ui.label("Glyph");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::TextEdit::singleline(&mut t.icon).char_limit(4).desired_width(48.0)).changed();
            if !t.icon.trim().is_empty() && !crate::glyphs::renders(ui.ctx(), &t.icon) {
                ui.label(RichText::new("⚠").color(palette.warning)).on_hover_text(format!(
                    "The built-in fonts have no glyph for this; the initial “{}” is drawn instead.",
                    crate::glyphs::initial(&t.name)
                ));
            }
            ui.menu_button("Pick…", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(220.0);
                    for g in GLYPHS {
                        if ui.button(RichText::new(g).size(18.0)).clicked() {
                            t.icon = g.to_string();
                            changed = true;
                            ui.close();
                        }
                    }
                });
            });
        });
        ui.end_row();
        ui.label("Sprite id");
        changed |= ui
            .add(egui::TextEdit::singleline(&mut t.sprite).desired_width(160.0).hint_text("optional"))
            .on_hover_text("Sprite or 3D model id for other renderers; this 2D view draws the colour and glyph.")
            .changed();
        ui.end_row();
        changed |= num_f32(ui, "Size (diameter)", &mut t.size, 0.05..=20.0, 0.01);
        changed |= num_f32(ui, "Perception radius", &mut t.perception, 0.1..=32.0, 0.05);
    });
    hint(ui, "Two entities touch when their centres are closer than the mean of their diameters.", palette);
    if let Some(new) = rename {
        rename_entity_type(&mut editor.draft, &old_name, &new);
        changed = true;
    }
    let name = editor.draft.entity_types[i].name.clone();

    // ---- properties -----------------------------------------------------
    heading(ui, "Properties", palette);
    hint(
        ui,
        "Numbers readable as `self.name` and writable with `set` / `add`; values are clamped to [min, max].",
        palette,
    );
    let mut remove_prop = None;
    let mut rename_prop = None;
    let prop_names: Vec<String> = editor.draft.entity_types[i].properties.iter().map(|p| p.name.clone()).collect();
    if !prop_names.is_empty() {
        egui::Grid::new(("cl_props", i)).num_columns(5).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new("name").small());
            ui.label(RichText::new("default").small());
            ui.label(RichText::new("min").small());
            ui.label(RichText::new("max").small());
            ui.label("");
            ui.end_row();
            for (k, old) in prop_names.iter().enumerate() {
                let taken: Vec<&str> = prop_names.iter().map(String::as_str).collect();
                if let Some(new) = name_field(ui, egui::Id::new(("cl_prop_name", i, k)), old, &taken, true) {
                    rename_prop = Some((old.clone(), new));
                }
                let p = &mut editor.draft.entity_types[i].properties[k];
                changed |= ui.add(crate::editor::widgets::dv(&mut p.default).speed(0.1).max_decimals(4)).changed();
                changed |= ui.add(crate::editor::widgets::dv(&mut p.min).speed(0.1).max_decimals(4)).changed();
                changed |= ui.add(crate::editor::widgets::dv(&mut p.max).speed(0.1).max_decimals(4)).changed();
                if ui.small_button("×").on_hover_text("Remove the property").clicked() {
                    remove_prop = Some(k);
                }
                ui.end_row();
            }
        });
    }
    if ui.button("+ Property").clicked() {
        let taken: Vec<&str> = prop_names.iter().map(String::as_str).collect();
        let pname = unique_name("value", taken);
        editor.draft.entity_types[i].properties.push(PropertyDef { name: pname, default: 0.0, min: 0.0, max: 100.0 });
        changed = true;
    }
    if let Some((old, new)) = rename_prop {
        rename_property(&mut editor.draft, &name, &old, &new);
        changed = true;
    }
    if let Some(k) = remove_prop {
        editor.draft.entity_types[i].properties.remove(k);
        changed = true;
    }

    // ---- states ---------------------------------------------------------
    heading(ui, "States", palette);
    hint(ui, "The first state is the initial one; none means a single implicit state `default`.", palette);
    let states = editor.draft.entity_types[i].states.clone();
    let mut state_action: Option<(usize, i8)> = None;
    let mut rename_st = None;
    for (k, s) in states.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(if k == 0 { "initial" } else { "" }).small().color(palette.text_muted));
            let taken: Vec<&str> = states.iter().map(String::as_str).collect();
            if let Some(new) = name_field(ui, egui::Id::new(("cl_state_name", i, k)), s, &taken, false) {
                rename_st = Some((s.clone(), new));
            }
            if ui.add_enabled(k > 0, egui::Button::new("Up").small()).clicked() {
                state_action = Some((k, -1));
            }
            if ui.add_enabled(k + 1 < states.len(), egui::Button::new("Down").small()).clicked() {
                state_action = Some((k, 1));
            }
            if ui.small_button("×").clicked() {
                state_action = Some((k, 0));
            }
        });
    }
    if ui.button("+ State").clicked() {
        let taken: Vec<&str> = states.iter().map(String::as_str).collect();
        let base = if states.is_empty() { "active" } else { "state" };
        editor.draft.entity_types[i].states.push(unique_name(base, taken));
        changed = true;
    }
    if let Some((old, new)) = rename_st {
        rename_state(&mut editor.draft, &name, &old, &new);
        changed = true;
    }
    if let Some((k, dir)) = state_action {
        let list = &mut editor.draft.entity_types[i].states;
        match dir {
            0 => {
                list.remove(k);
            }
            -1 => list.swap(k, k - 1),
            _ => list.swap(k, k + 1),
        }
        changed = true;
    }

    // ---- carried resources ----------------------------------------------
    heading(ui, "Carried resources", palette);
    hint(
        ui,
        "Stocks of a world resource: `consume` fills them instead of giving energy, `emit` releases from them, \
         `trade` exchanges them, death drops them into the field.",
        palette,
    );
    let mut remove_carry = None;
    {
        let t = &mut editor.draft.entity_types[i];
        if !t.carries.is_empty() {
            egui::Grid::new(("cl_carries", i)).num_columns(4).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label(RichText::new("resource").small());
                ui.label(RichText::new("initial").small());
                ui.label(RichText::new("capacity").small());
                ui.label("");
                ui.end_row();
                for (k, c) in t.carries.iter_mut().enumerate() {
                    changed |= name_combo(ui, ("cl_carry_res", i, k), &mut c.resource, &resources, 120.0);
                    changed |=
                        ui.add(crate::editor::widgets::dv(&mut c.initial).range(0.0..=1.0e6).speed(0.1)).changed();
                    changed |=
                        ui.add(crate::editor::widgets::dv(&mut c.capacity).range(0.0..=1.0e6).speed(0.1)).changed();
                    if ui.small_button("×").clicked() {
                        remove_carry = Some(k);
                    }
                    ui.end_row();
                }
            });
        }
        let free: Vec<&String> = resources.iter().filter(|r| !t.carries.iter().any(|c| &c.resource == *r)).collect();
        let add = ui
            .add_enabled(!free.is_empty(), egui::Button::new("+ Carried resource"))
            .on_disabled_hover_text("Add a resource field in the World tab first.");
        if add.clicked()
            && let Some(r) = free.first()
        {
            t.carries.push(CarriedResource { resource: (*r).clone(), initial: 0.0, capacity: 10.0 });
            changed = true;
        }
    }
    if let Some(k) = remove_carry {
        editor.draft.entity_types[i].carries.remove(k);
        changed = true;
    }

    let t = &mut editor.draft.entity_types[i];
    // ---- movement -------------------------------------------------------
    heading(ui, "Default movement", palette);
    hint(ui, "Used whenever no rule or script moved the entity in a tick.", palette);
    let mut kind = match t.movement {
        MovementDef::None => 0,
        MovementDef::RandomWalk { .. } => 1,
        MovementDef::Directed { .. } => 2,
    };
    let before = kind;
    ui.horizontal(|ui| {
        ui.radio_value(&mut kind, 0, "Sessile");
        ui.radio_value(&mut kind, 1, "Random walk");
        ui.radio_value(&mut kind, 2, "Correlated walk");
    });
    if kind != before {
        let speed = t.movement.speed().max(0.5);
        t.movement = match kind {
            0 => MovementDef::None,
            1 => MovementDef::RandomWalk { speed },
            _ => MovementDef::Directed { speed, turn: 0.6 },
        };
        changed = true;
    }
    egui::Grid::new(("cl_movement", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| match &mut t.movement {
        MovementDef::None => {}
        MovementDef::RandomWalk { speed } => {
            changed |= num_f32(ui, "Speed (units/tick)", speed, 0.0..=50.0, 0.01);
        }
        MovementDef::Directed { speed, turn } => {
            changed |= num_f32(ui, "Speed (units/tick)", speed, 0.0..=50.0, 0.01);
            changed |= num_f32(ui, "Max turn (rad/tick)", turn, 0.0..=std::f32::consts::PI, 0.01);
        }
    });

    // ---- energy ---------------------------------------------------------
    heading(ui, "Energy", palette);
    egui::Grid::new(("cl_energy", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        changed |= num_f64(ui, "Initial", &mut t.energy.initial, 0.0..=1.0e6, 0.1);
        changed |= num_f64(ui, "Maximum", &mut t.energy.max, 0.0..=1.0e6, 0.1);
        changed |= num_f64(ui, "Metabolism (per tick)", &mut t.energy.metabolism, 0.0..=1000.0, 0.005);
        ui.label("Starvation");
        changed |= ui.checkbox(&mut t.energy.starvation, "dies at energy ≤ 0").changed();
        ui.end_row();
    });

    // ---- reproduction ---------------------------------------------------
    heading(ui, "Reproduction", palette);
    hint(ui, "Each parent pays the cost; the litter shares it, so reproduction never creates energy.", palette);
    egui::Grid::new(("cl_repro", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Mode");
        egui::ComboBox::from_id_salt(("cl_repro_mode", i)).selected_text(format!("{:?}", t.reproduction.mode)).show_ui(
            ui,
            |ui| {
                for m in [ReproductionMode::None, ReproductionMode::Asexual, ReproductionMode::Sexual] {
                    changed |= ui.selectable_value(&mut t.reproduction.mode, m, format!("{m:?}")).changed();
                }
            },
        );
        ui.end_row();
        if t.reproduction.mode != ReproductionMode::None {
            changed |= num_f64(ui, "Cost (energy per parent)", &mut t.reproduction.cost, 0.0..=1.0e6, 0.1);
            changed |= num_u64(ui, "Cooldown (ticks)", &mut t.reproduction.cooldown, 0..=1_000_000);
            changed |= num_u32(ui, "Litter", &mut t.reproduction.litter, 1..=100);
            ui.label("Rate (per tick)");
            changed |= ui
                .add(
                    egui::Slider::new(&mut t.reproduction.rate, 0.0..=1.0)
                        .max_decimals(3)
                        .clamping(egui::SliderClamping::Edits),
                )
                .on_hover_text("Chance per tick of a generated reproduce rule; 0 = only rules that say `reproduce`.")
                .changed();
            ui.end_row();
        }
    });

    // ---- lifecycle ------------------------------------------------------
    heading(ui, "Lifecycle", palette);
    egui::Grid::new(("cl_life", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Maximum age");
        ui.horizontal(|ui| {
            let mut limited = t.lifecycle.max_age.is_some();
            if ui.checkbox(&mut limited, "").changed() {
                t.lifecycle.max_age = limited.then_some(500);
                changed = true;
            }
            if let Some(age) = &mut t.lifecycle.max_age {
                changed |= ui.add(crate::editor::widgets::dv(age).range(1..=10_000_000).suffix(" ticks")).changed();
            } else {
                ui.label(RichText::new("immortal").color(palette.text_muted));
            }
        });
        ui.end_row();
        changed |= num_u64(ui, "Maturity (age)", &mut t.lifecycle.maturity, 0..=10_000_000);
    });

    // ---- initial population ---------------------------------------------
    heading(ui, "Initial population", palette);
    egui::Grid::new(("cl_initial", i)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        changed |= num_u32(ui, "Count at reset", &mut t.initial.count, 0..=max_entities);
        ui.label("Distribution");
        let label = match t.initial.distribution {
            Distribution::Uniform => "Uniform",
            Distribution::Cluster { .. } => "Cluster",
            Distribution::Rect { .. } => "Rectangle",
            Distribution::Grid => "Grid",
        };
        egui::ComboBox::from_id_salt(("cl_dist", i)).selected_text(label).show_ui(ui, |ui| {
            let (cx, cy) = (world_w / 2.0, world_h / 2.0);
            let options = [
                ("Uniform", Distribution::Uniform),
                ("Cluster", Distribution::Cluster { x: cx, y: cy, radius: world_w.min(world_h) / 4.0 }),
                ("Rectangle", Distribution::Rect { x: 0.0, y: 0.0, width: world_w / 2.0, height: world_h / 2.0 }),
                ("Grid", Distribution::Grid),
            ];
            for (name, value) in options {
                let same = std::mem::discriminant(&t.initial.distribution) == std::mem::discriminant(&value);
                if ui.selectable_label(same, name).clicked() && !same {
                    t.initial.distribution = value;
                    changed = true;
                }
            }
        });
        ui.end_row();
        match &mut t.initial.distribution {
            Distribution::Cluster { x, y, radius } => {
                changed |= num_f32(ui, "Centre x", x, 0.0..=world_w, 0.1);
                changed |= num_f32(ui, "Centre y", y, 0.0..=world_h, 0.1);
                changed |= num_f32(ui, "Radius", radius, 0.0..=world_w.max(world_h), 0.1);
            }
            Distribution::Rect { x, y, width, height } => {
                changed |= num_f32(ui, "Left", x, 0.0..=world_w, 0.1);
                changed |= num_f32(ui, "Top", y, 0.0..=world_h, 0.1);
                changed |= num_f32(ui, "Width", width, 0.0..=world_w, 0.1);
                changed |= num_f32(ui, "Height", height, 0.0..=world_h, 0.1);
            }
            Distribution::Uniform | Distribution::Grid => {}
        }
    });
    hint(ui, "Initial populations only matter at the next reset.", palette);

    // ---- problems and deletion -------------------------------------------
    heading(ui, "Problems", palette);
    diagnostics_list(ui, &diags, palette, "No problems for this type.");
    ui.add_space(8.0);
    if confirm_delete(
        ui,
        egui::Id::new(("cl_delete_type", i)),
        &format!("`{name}` with its rules and relationships"),
        palette,
    ) {
        crate::draft::remove_entity_type(&mut editor.draft, i);
        changed = true;
    }
    if changed {
        editor.touch();
    }
    let _ = EditorTab::Entities;
}
