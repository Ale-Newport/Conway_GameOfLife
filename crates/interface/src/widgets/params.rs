//! Parameter editor generated from a model's [`ParamSpec`]s.
//!
//! Every control sends [`SessionControl::set_param`]; the model validates
//! the value and reports when it takes effect ([`ApplyPolicy`], shown as a
//! badge next to each parameter and as a notice for the latest change).
//! Rejected values come back as error toasts from the session.

use bevy_egui::egui::{self, Color32, RichText, Ui};
use simulation_core::params::ParamKind;
use simulation_core::{ApplyPolicy, ModelDescriptor, ParamSet, ParamSpec, ParamValue, SessionControl};

use crate::theme::{rgba_from_srgba_bytes, srgba_bytes};
use crate::widgets::legend::badge;

/// Seconds an edited value is shown while waiting for the simulation to
/// confirm it (afterwards the model's value wins, e.g. after a rejection).
pub const PENDING_TIMEOUT_S: f64 = 1.0;

/// The value to display: a recent local edit until the model confirms it,
/// otherwise the model's value. Returns the value and whether the pending
/// edit must be kept.
pub fn displayed_value(current: &ParamValue, pending: Option<&(ParamValue, f64)>, now: f64) -> (ParamValue, bool) {
    match pending {
        Some((value, _)) if value == current => (current.clone(), false),
        Some((value, at)) if now - at < PENDING_TIMEOUT_S => (value.clone(), true),
        _ => (current.clone(), false),
    }
}

/// Badge text and colour of an apply policy (darker shades on light
/// themes so the text stays readable).
pub fn policy_badge(policy: ApplyPolicy, dark_mode: bool) -> (&'static str, Color32) {
    let (text, color) = match policy {
        ApplyPolicy::Immediate => ("live", Color32::from_rgb(92, 201, 128)),
        ApplyPolicy::NextTick => ("next tick", Color32::from_rgb(110, 160, 240)),
        ApplyPolicy::RequiresReset => ("on reset", Color32::from_rgb(240, 170, 60)),
    };
    (text, if dark_mode { color } else { crate::theme::mix(color, Color32::BLACK, 0.45) })
}

/// Groups in order of first appearance.
pub fn groups(specs: &[ParamSpec]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for spec in specs {
        if !out.contains(&spec.group.as_str()) {
            out.push(&spec.group);
        }
    }
    out
}

/// Draws editors for every parameter of `descriptor` with the values in
/// `params`, sending changes to `session`, followed by a notice about the
/// latest change reported by the session.
pub fn param_editor(ui: &mut Ui, descriptor: &ModelDescriptor, params: &ParamSet, session: &dyn SessionControl) {
    if descriptor.params.is_empty() {
        ui.label(RichText::new("This model has no adjustable parameters.").weak());
        return;
    }
    if let Some((key, policy)) = session.status().last_param_change {
        change_notice(ui, descriptor, &key, policy, session);
    }
    // Sliders use the panel width (minus room for the value field).
    ui.spacing_mut().slider_width = (ui.available_width() - 90.0).clamp(120.0, 420.0);
    for group in groups(&descriptor.params) {
        egui::CollapsingHeader::new(RichText::new(group).strong())
            .id_salt(("param_group", &descriptor.id, group))
            .default_open(true)
            .show(ui, |ui| {
                for spec in descriptor.params.iter().filter(|s| s.group == group) {
                    param_row(ui, spec, params, session);
                    ui.add_space(4.0);
                }
            });
    }
}

fn change_notice(
    ui: &mut Ui,
    descriptor: &ModelDescriptor,
    key: &str,
    policy: ApplyPolicy,
    session: &dyn SessionControl,
) {
    let label = descriptor.param(key).map_or(key, |s| s.label.as_str());
    let (_, color) = policy_badge(policy, ui.visuals().dark_mode);
    egui::Frame::new()
        .fill(crate::theme::with_alpha(color, 0.10))
        .stroke(egui::Stroke::new(1.0, crate::theme::with_alpha(color, 0.45)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Last change:").small().weak());
                ui.label(RichText::new(label).small().strong());
                ui.label(RichText::new(format!("— {}", policy.label())).small().color(color));
                if policy == ApplyPolicy::RequiresReset
                    && ui
                        .small_button("Reset now")
                        .on_hover_text("Restart the scenario so the change takes effect (undoable)")
                        .clicked()
                {
                    session.reset();
                }
            });
        });
    ui.add_space(4.0);
}

fn param_row(ui: &mut Ui, spec: &ParamSpec, params: &ParamSet, session: &dyn SessionControl) {
    let current = params.get(&spec.key).cloned().unwrap_or_else(|| spec.default.clone());
    let pending_id = egui::Id::new(("param_pending", &spec.key));
    let now = ui.input(|i| i.time);
    let pending: Option<(ParamValue, f64)> = ui.data(|d| d.get_temp(pending_id));
    let (shown, keep) = displayed_value(&current, pending.as_ref(), now);
    if !keep && pending.is_some() {
        ui.data_mut(|d| d.remove::<(ParamValue, f64)>(pending_id));
    }

    ui.horizontal(|ui| {
        let label = ui.label(RichText::new(&spec.label).size(13.5));
        if !spec.description.is_empty() {
            label.on_hover_text(&spec.description);
        }
        let (text, color) = policy_badge(spec.apply, ui.visuals().dark_mode);
        badge(ui, text, color).on_hover_text(spec.apply.label());
        if shown != spec.default {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("↺").on_hover_text(format!("Back to the default ({})", spec.default)).clicked() {
                    send(ui, session, spec, spec.default.clone(), pending_id, now);
                }
            });
        }
    });

    if let Some(value) = edit_value(ui, spec, &shown) {
        send(ui, session, spec, value, pending_id, now);
    }
}

fn send(ui: &mut Ui, session: &dyn SessionControl, spec: &ParamSpec, value: ParamValue, id: egui::Id, now: f64) {
    session.set_param(&spec.key, value.clone());
    ui.data_mut(|d| d.insert_temp(id, (value, now)));
}

/// Draws the control for `spec` showing `value`; returns a new value when
/// the user changed it.
fn edit_value(ui: &mut Ui, spec: &ParamSpec, value: &ParamValue) -> Option<ParamValue> {
    let hover = spec.description.as_str();
    match &spec.kind {
        ParamKind::Bool => {
            let mut v = value.as_bool().unwrap_or(false);
            let text = if v { "Enabled" } else { "Disabled" };
            ui.checkbox(&mut v, text).on_hover_text(hover).changed().then_some(ParamValue::Bool(v))
        }
        ParamKind::Int { min, max } => {
            let mut v = value.as_i64().unwrap_or(*min).clamp(*min, *max);
            let span = max.saturating_sub(*min);
            let response = if span <= 100_000 {
                ui.add(egui::Slider::new(&mut v, *min..=*max).clamping(egui::SliderClamping::Always))
            } else {
                ui.add(egui::DragValue::new(&mut v).range(*min..=*max).speed((span as f64 / 2_000.0).max(1.0)))
            };
            response.on_hover_text(hover).changed().then_some(ParamValue::Int(v))
        }
        ParamKind::Float { min, max, step, logarithmic } => {
            let mut v = value.as_f64().unwrap_or(*min);
            let response = if min.is_finite() && max.is_finite() && min < max {
                v = v.clamp(*min, *max);
                let mut slider = egui::Slider::new(&mut v, *min..=*max)
                    .logarithmic(*logarithmic)
                    .clamping(egui::SliderClamping::Always);
                if let Some(step) = slider_step(*step, *logarithmic) {
                    slider = slider.step_by(step);
                }
                ui.add(slider)
            } else {
                let mut drag = egui::DragValue::new(&mut v).speed(step.abs().max(1e-3));
                if min <= max {
                    drag = drag.range(*min..=*max);
                }
                ui.add(drag)
            };
            response.on_hover_text(hover).changed().then(|| ParamValue::Float(within(v, *min, *max)))
        }
        ParamKind::Text { max_len } => text_field(ui, spec, value.as_str().unwrap_or(""), *max_len),
        ParamKind::Choice { options } => {
            let current = value.as_str().unwrap_or("");
            let selected = options.iter().find(|o| o.value == current).map_or(current, |o| o.label.as_str());
            let mut chosen = None;
            egui::ComboBox::from_id_salt(("param_choice", &spec.key)).selected_text(selected).show_ui(ui, |ui| {
                for option in options {
                    if ui.selectable_label(option.value == current, &option.label).clicked() && option.value != current
                    {
                        chosen = Some(ParamValue::Text(option.value.clone()));
                    }
                }
            });
            chosen
        }
        ParamKind::Color => {
            // `ParamValue::Color` is unmultiplied sRGB, so the sRGB editor is
            // used (the `rgba` editors of egui work in linear space).
            let mut srgba = srgba_bytes(value.as_color().unwrap_or([1.0; 4]));
            ui.color_edit_button_srgba_unmultiplied(&mut srgba)
                .on_hover_text(hover)
                .changed()
                .then(|| ParamValue::Color(rgba_from_srgba_bytes(srgba)))
        }
    }
}

/// Slider step of a float parameter. egui rounds slider values to
/// `min + k·step`; on a logarithmic scale a fixed step would make the low
/// end of the range unreachable (e.g. nothing between 0.001 and 0.011 for a
/// 0.001–10 range with the default step), so logarithmic sliders are not
/// stepped. Non-positive or non-finite steps are ignored.
pub fn slider_step(step: f64, logarithmic: bool) -> Option<f64> {
    (!logarithmic && step.is_finite() && step > 0.0).then_some(step)
}

/// Keeps an edited value inside `[min, max]`. Step rounding (`min + k·step`
/// with an inexact `step`) can overshoot `max` by a rounding error, and the
/// model rejects out-of-range values instead of clamping them.
pub fn within(value: f64, min: f64, max: f64) -> f64 {
    if min <= max { value.clamp(min, max) } else { value }
}

/// Single-line editor applied on Enter or when focus leaves the field.
fn text_field(ui: &mut Ui, spec: &ParamSpec, current: &str, max_len: usize) -> Option<ParamValue> {
    let buffer_id = egui::Id::new(("param_text", &spec.key));
    let mut buffer: String = ui.data(|d| d.get_temp(buffer_id)).unwrap_or_else(|| current.to_string());
    let response = ui.add(
        egui::TextEdit::singleline(&mut buffer).char_limit(max_len).desired_width(f32::INFINITY).hint_text(&spec.label),
    );
    let response = if spec.description.is_empty() { response } else { response.on_hover_text(&spec.description) };
    let committed = response.lost_focus();
    if response.has_focus() || response.changed() {
        ui.data_mut(|d| d.insert_temp(buffer_id, buffer.clone()));
    }
    if committed {
        ui.data_mut(|d| d.remove::<String>(buffer_id));
        if buffer != current {
            return Some(ParamValue::Text(buffer));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::commands::tests::FakeSession;

    #[test]
    fn pending_edits_win_until_confirmed_or_stale() {
        let current = ParamValue::Float(0.3);
        let edit = (ParamValue::Float(0.5), 10.0);
        assert_eq!(displayed_value(&current, Some(&edit), 10.2), (ParamValue::Float(0.5), true));
        assert_eq!(displayed_value(&current, Some(&edit), 11.5), (ParamValue::Float(0.3), false));
        let confirmed = ParamValue::Float(0.5);
        assert_eq!(displayed_value(&confirmed, Some(&edit), 10.2), (ParamValue::Float(0.5), false));
        assert_eq!(displayed_value(&current, None, 0.0), (ParamValue::Float(0.3), false));
    }

    #[test]
    fn stepped_values_stay_valid_for_the_model() {
        // The default step of `ParamSpec::float` is (max - min) / 1000; egui
        // rounds to min + k·step, which can overshoot max in floating point.
        let mut overshoots = 0;
        for (lo, hi) in [(0.1, 0.7), (0.0, 0.3), (0.03, 0.3), (0.03, 0.29), (1.1, 2.3), (0.05, 0.95), (0.01, 0.07)] {
            let spec = ParamSpec::float("p", "P", lo, hi, lo);
            let ParamKind::Float { min, max, step, logarithmic } = spec.kind.clone() else { unreachable!() };
            let step = slider_step(step, logarithmic).unwrap();
            // What egui produces for the right end of the slider.
            let top = min + ((max - min) / step).round() * step;
            overshoots += usize::from(top > max);
            let sent = ParamValue::Float(within(top, min, max));
            assert!(spec.validate(&sent).is_ok(), "[{lo}, {hi}]: {top} must be sent as an accepted value");
        }
        assert!(overshoots > 0, "the rounding overshoot this guards against occurs for some ranges");
        // Logarithmic sliders are not stepped, so small values stay reachable.
        let log = ParamSpec::float("mu", "Mu", 0.001, 10.0, 0.15).logarithmic();
        let ParamKind::Float { step, logarithmic, .. } = log.kind else { unreachable!() };
        assert_eq!(slider_step(step, logarithmic), None);
        assert_eq!(slider_step(f64::NAN, false), None);
        assert_eq!(slider_step(0.0, false), None);
        assert_eq!(within(f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY), f64::INFINITY);
    }

    #[test]
    fn groups_keep_declaration_order() {
        let specs = vec![
            ParamSpec::int("w", "Width", 1, 10, 5).group("World"),
            ParamSpec::text("rule", "Rule", 20, "B3/S23").group("Rules"),
            ParamSpec::int("h", "Height", 1, 10, 5).group("World"),
        ];
        assert_eq!(groups(&specs), vec!["World", "Rules"]);
    }

    #[test]
    fn every_kind_renders_without_sending() {
        let fake = Arc::new(FakeSession::new());
        let mut descriptor = fake.descriptor.clone();
        descriptor.params = vec![
            ParamSpec::boolean("wrap", "Wrap", true),
            ParamSpec::int("w", "Width", 1, 1_000_000, 64).apply(ApplyPolicy::RequiresReset),
            ParamSpec::float("p", "Density", 0.0, 1.0, 0.3).logarithmic(),
            ParamSpec::text("rule", "Rule", 20, "B3/S23"),
            ParamSpec::choice("topo", "Topology", &[("torus", "Torus"), ("plane", "Plane")], "torus"),
            ParamSpec::color("c", "Colour", [1.0, 0.5, 0.0, 1.0]),
        ];
        let params = ParamSet::from_defaults(&descriptor.params);
        fake.status.lock().unwrap().last_param_change = Some(("w".into(), ApplyPolicy::RequiresReset));
        let ctx = egui::Context::default();
        let _ = crate::run_headless_ui(&ctx, |ui| param_editor(ui, &descriptor, &params, fake.as_ref()));
        assert!(fake.calls().is_empty(), "rendering alone must not change parameters");
    }
}
