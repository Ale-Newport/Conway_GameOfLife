//! The "Rules" panel: the model's identity, time step and the formal rule
//! statement ([`ModelDescriptor::rules`]) rendered as Markdown, plus a
//! glossary of its metrics.

use bevy_egui::egui::{self, RichText, Ui};
use simulation_core::ModelDescriptor;

use crate::format::{format_compact, title_case};
use crate::theme::Palette;
use crate::widgets::legend::badge;
use crate::widgets::markdown::markdown;

/// Whether one tick is exactly one `tick_unit` (cellular automata:
/// one tick = one generation).
fn unit_ticks(descriptor: &ModelDescriptor) -> bool {
    (descriptor.dt - 1.0).abs() < f64::EPSILON
}

fn plural(unit: &str) -> String {
    if unit.ends_with('s') || unit.is_empty() { unit.to_string() } else { format!("{unit}s") }
}

/// Describes how much simulated time one tick represents.
pub fn time_step_text(descriptor: &ModelDescriptor) -> String {
    let unit = &descriptor.tick_unit;
    if unit_ticks(descriptor) {
        format!("1 tick = 1 {unit}")
    } else {
        format!("1 tick = {} {}", format_compact(descriptor.dt), plural(unit))
    }
}

/// Name of the tick counter: the tick unit when one tick is one unit
/// (`generation`), otherwise `tick` — a count of ticks is not a count of
/// `tick_unit`s when `dt != 1` (e.g. 1 tick = 6 minutes).
pub fn tick_label(descriptor: &ModelDescriptor) -> String {
    if unit_ticks(descriptor) { descriptor.tick_unit.clone() } else { "tick".to_string() }
}

/// Label of the rate of the tick counter (`RunnerStatus::measured_gps`),
/// e.g. `generations/s`, or `None` when it would only repeat ticks/s (the
/// unit is `tick`, or one tick is not one unit).
pub fn counter_rate_label(descriptor: &ModelDescriptor) -> Option<String> {
    let unit = &descriptor.tick_unit;
    (unit_ticks(descriptor) && !unit.is_empty() && unit != "tick").then(|| format!("{}/s", plural(unit)))
}

/// Simulated time reached after `tick` ticks (`tick × dt` in the model's
/// unit, e.g. `6,000 minutes`), or `None` when it equals the tick count.
pub fn simulated_time_text(descriptor: &ModelDescriptor, tick: u64) -> Option<String> {
    (!unit_ticks(descriptor))
        .then(|| format!("{} {}", format_compact(tick as f64 * descriptor.dt), plural(&descriptor.tick_unit)))
}

/// Draws the rules panel for `descriptor`.
pub fn rules_panel(ui: &mut Ui, descriptor: &ModelDescriptor, palette: &Palette) {
    ui.label(RichText::new(&descriptor.name).size(17.0).strong());
    ui.horizontal_wrapped(|ui| {
        badge(ui, descriptor.family.label(), palette.accent);
        if descriptor.deterministic {
            badge(ui, "deterministic", palette.success)
                .on_hover_text("Runs are bit-for-bit reproducible for a fixed seed and inputs.");
        } else {
            badge(ui, "not deterministic", palette.warning);
        }
        badge(ui, &time_step_text(descriptor), palette.text_muted);
    });
    ui.label(RichText::new(format!("Model id: {}", descriptor.id)).small().weak());
    if !descriptor.summary.is_empty() {
        ui.add_space(4.0);
        ui.label(&descriptor.summary);
    }
    ui.separator();
    if descriptor.rules.trim().is_empty() {
        ui.label(RichText::new("This model does not publish a formal rule statement.").weak());
    } else {
        markdown(ui, &descriptor.rules);
    }
    if !descriptor.metrics.is_empty() {
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new("Metrics guide").strong())
            .id_salt(("metrics_guide", &descriptor.id))
            .default_open(false)
            .show(ui, |ui| {
                for metric in &descriptor.metrics {
                    let unit = if metric.unit.is_empty() { String::new() } else { format!(" ({})", metric.unit) };
                    ui.label(RichText::new(format!("{}{unit}", title_case(&metric.label))).strong());
                    ui.label(RichText::new(&metric.description).small());
                    ui.add_space(3.0);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use simulation_core::ModelFamily;

    use super::*;

    #[test]
    fn time_step_wording() {
        let d = ModelDescriptor::new("m", "M", ModelFamily::Cellular);
        assert_eq!(time_step_text(&d), "1 tick = 1 generation");
        let d = d.time("minute", 6.0);
        assert_eq!(time_step_text(&d), "1 tick = 6 minutes");
        let d = ModelDescriptor::new("m", "M", ModelFamily::Continuous).time("time unit", 0.1);
        assert_eq!(time_step_text(&d), "1 tick = 0.100 time units");
    }

    #[test]
    fn tick_counts_are_not_mislabelled_as_time() {
        let life = ModelDescriptor::new("m", "M", ModelFamily::Cellular);
        assert_eq!(tick_label(&life), "generation");
        assert_eq!(simulated_time_text(&life, 1_000), None);
        assert_eq!(counter_rate_label(&life).as_deref(), Some("generations/s"));
        let agents = ModelDescriptor::new("e", "E", ModelFamily::AgentBased).time("tick", 1.0);
        assert_eq!(counter_rate_label(&agents), None, "would repeat ticks/s");
        // The physics world runs 6 simulated minutes per tick: 1,000 ticks
        // are 6,000 minutes, not "minute 1,000".
        let world = ModelDescriptor::new("w", "W", ModelFamily::Hybrid).time("minute", 6.0);
        assert_eq!(tick_label(&world), "tick");
        assert_eq!(simulated_time_text(&world, 1_000).as_deref(), Some("6,000 minutes"));
        assert_eq!(counter_rate_label(&world), None);
        let lenia = ModelDescriptor::new("l", "L", ModelFamily::Continuous).time("time unit", 0.1);
        assert_eq!(simulated_time_text(&lenia, 25).as_deref(), Some("2.50 time units"));
    }
}
