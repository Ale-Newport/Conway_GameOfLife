//! Bottom bar: time controls driven by the active session and live
//! readouts.

use bevy::prelude::*;
use bevy_egui::egui::{self, RichText, Ui};
use simulation_core::{ModelDescriptor, RunnerStatus, SessionControl, Speed};

use super::Frame;
use super::timeline::timeline;
use crate::commands::{MAX_TICKS_PER_SECOND, MIN_TICKS_PER_SECOND, ShellCommand};
use crate::format::{format_bytes, format_compact, format_count, format_ms, format_rate, title_case};
use crate::icons::{Icon, icon_button};
use crate::session::{MetricsHistory, Telemetry};
use crate::state::ShellState;
use crate::theme::Palette;
use crate::widgets::rules::{counter_rate_label, simulated_time_text, tick_label, time_step_text};

pub(super) fn show(ui: &mut Ui, world: &mut World, shell: &mut ShellState, frame: &Frame) {
    let (Some(session), Some(descriptor), Some(status)) =
        (frame.session.as_ref(), frame.descriptor.as_deref(), frame.status.as_ref())
    else {
        ui.label(RichText::new("Time controls appear once the simulation is running.").weak());
        return;
    };
    let session = session.session();
    let mut commands: Vec<ShellCommand> = Vec::new();
    ui.horizontal(|ui| {
        transport(ui, session, status, shell, &frame.palette, &mut commands);
        ui.separator();
        speed(ui, status, &mut commands);
        ui.separator();
        let width = ui.available_width() - 8.0;
        if let Some(command) = timeline(ui, status, &frame.palette, width) {
            commands.push(command);
        }
    });
    ui.add_space(2.0);
    readouts(ui, world, descriptor, status, &frame.palette);
    for command in commands {
        world.write_message(command);
    }
}

fn transport(
    ui: &mut Ui,
    session: &dyn SessionControl,
    status: &RunnerStatus,
    shell: &mut ShellState,
    p: &Palette,
    out: &mut Vec<ShellCommand>,
) {
    let can_run = !status.finished;
    let (icon, tip) = if status.playing { (Icon::Pause, "Pause (Space)") } else { (Icon::Play, "Play (Space)") };
    let play_tip = if can_run || status.playing { tip } else { "The scenario has finished — reset to run it again" };
    if icon_button(ui, icon, play_tip, can_run || status.playing, status.playing, p).clicked() {
        out.push(ShellCommand::TogglePlay);
    }
    if icon_button(ui, Icon::Step, "Step one tick (N)", can_run, false, p).clicked() {
        out.push(ShellCommand::Step(1));
    }
    let tip = format!("Step {} ticks (Shift+N)", format_count(shell.step_many));
    if icon_button(ui, Icon::StepMany, &tip, can_run, false, p).clicked() {
        out.push(ShellCommand::Step(shell.step_many));
    }
    ui.add(egui::DragValue::new(&mut shell.step_many).range(1..=1_000_000).speed(1.0).prefix("×"))
        .on_hover_text("Ticks per multi-step");
    ui.separator();
    if icon_button(ui, Icon::Reset, "Reset to the scenario's initial state (R, undoable)", true, false, p).clicked() {
        out.push(ShellCommand::Reset);
    }
    if session.can_clear() && icon_button(ui, Icon::Clear, "Clear the world (undoable)", true, false, p).clicked() {
        out.push(ShellCommand::Clear);
    }
    if session.can_randomize() {
        let tip = format!("Randomise the world with seed {} (undoable; the seed then advances)", shell.randomize_seed);
        if icon_button(ui, Icon::Dice, &tip, true, false, p).clicked() {
            out.push(ShellCommand::Randomize(shell.randomize_seed));
            shell.randomize_seed = shell.randomize_seed.wrapping_add(1);
        }
        ui.add(egui::DragValue::new(&mut shell.randomize_seed).prefix("seed "))
            .on_hover_text("Seed of the next randomisation");
    }
    ui.separator();
    if icon_button(ui, Icon::Undo, "Undo (Ctrl/Cmd+Z)", status.can_undo, false, p).clicked() {
        out.push(ShellCommand::Undo);
    }
    if icon_button(ui, Icon::Redo, "Redo (Shift+Ctrl/Cmd+Z)", status.can_redo, false, p).clicked() {
        out.push(ShellCommand::Redo);
    }
}

fn speed(ui: &mut Ui, status: &RunnerStatus, out: &mut Vec<ShellCommand>) {
    let unlimited = status.speed == Speed::Unlimited;
    let mut rate = match status.speed {
        Speed::TicksPerSecond(r) => r.clamp(MIN_TICKS_PER_SECOND, MAX_TICKS_PER_SECOND),
        Speed::Unlimited => MAX_TICKS_PER_SECOND,
    };
    ui.label(RichText::new("Speed").small().weak());
    let slider = egui::Slider::new(&mut rate, MIN_TICKS_PER_SECOND..=MAX_TICKS_PER_SECOND)
        .logarithmic(true)
        .clamping(egui::SliderClamping::Always)
        .custom_formatter(|v, _| format_compact(v.round()))
        .custom_parser(|text| text.replace(',', "").trim().parse().ok())
        .suffix(" ticks/s");
    let response = ui.add_enabled(!unlimited, slider).on_hover_text("Target ticks per second (logarithmic; +/- keys)");
    if response.changed() {
        out.push(ShellCommand::SetSpeed(Speed::TicksPerSecond(rate.round().max(MIN_TICKS_PER_SECOND))));
    }
    if ui
        .selectable_label(unlimited, "Max")
        .on_hover_text("Run as fast as the model allows while staying responsive")
        .clicked()
    {
        out.push(ShellCommand::SetSpeed(if unlimited {
            Speed::TicksPerSecond(MAX_TICKS_PER_SECOND)
        } else {
            Speed::Unlimited
        }));
    }
}

fn chip(ui: &mut Ui, p: &Palette, label: &str, value: String, tooltip: &str) {
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(label).size(11.5).color(p.text_muted));
            ui.label(RichText::new(value).size(13.0).monospace().color(p.text));
        })
        .response;
    response.on_hover_text(tooltip);
    ui.add_space(10.0);
}

fn readouts(ui: &mut Ui, world: &World, descriptor: &ModelDescriptor, status: &RunnerStatus, p: &Palette) {
    let telemetry = *world.resource::<Telemetry>();
    let population = world.resource::<MetricsHistory>().latest("population");
    ui.horizontal_wrapped(|ui| {
        let (state, color) = if status.finished {
            ("FINISHED", p.warning)
        } else if status.playing {
            ("RUNNING", p.success)
        } else {
            ("PAUSED", p.text_muted)
        };
        ui.label(RichText::new(state).size(11.0).strong().color(color));
        ui.add_space(8.0);
        chip(
            ui,
            p,
            &title_case(&tick_label(descriptor)),
            format_count(status.tick),
            "Ticks executed since the last reset",
        );
        if let Some(time) = simulated_time_text(descriptor, status.tick) {
            chip(ui, p, "Time", time, &format!("Simulated time ({})", time_step_text(descriptor)));
        }
        if let Some(population) = population {
            chip(ui, p, "Population", format_compact(population), "Latest value of the model's `population` metric");
        }
        let target = match status.speed {
            Speed::TicksPerSecond(r) => format_rate(r),
            Speed::Unlimited => "max".to_string(),
        };
        let measured = if status.playing { format_rate(status.measured_tps) } else { "—".to_string() };
        chip(
            ui,
            p,
            "Ticks",
            format!("{measured} (target {target})"),
            "Ticks per second measured over the last half second",
        );
        if let Some(label) = counter_rate_label(descriptor) {
            let rate = if status.playing { format_rate(status.measured_gps) } else { "—".to_string() };
            chip(
                ui,
                p,
                &title_case(&label),
                rate.trim_end_matches("/s").to_string(),
                "Measured from the tick counter; differs from ticks per second for engines that advance \
                 several generations per tick (Hashlife)",
            );
        }
        chip(ui, p, "Tick", format_ms(status.avg_step_ms), "Average duration of one tick (exponential moving average)");
        chip(
            ui,
            p,
            "FPS",
            telemetry.fps.map_or_else(|| "—".to_string(), |f| format!("{f:.0}")),
            "Rendered frames per second (independent of the simulation speed)",
        );
        let app = telemetry.process_memory.map_or_else(|| "—".to_string(), format_bytes);
        chip(
            ui,
            p,
            "Memory",
            format!(
                "{app} · model {} · history {}",
                format_bytes(status.memory_bytes as u64),
                format_bytes(status.history_bytes as u64)
            ),
            "Process resident memory · model estimate · stored history snapshots",
        );
    });
}
