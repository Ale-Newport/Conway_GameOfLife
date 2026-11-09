//! History timeline: stored snapshot ticks as markers, the current tick as
//! a cursor; clicking a marker restores that snapshot.

use bevy_egui::egui::{Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use simulation_core::RunnerStatus;

use crate::commands::ShellCommand;
use crate::format::{format_bytes, format_count};
use crate::theme::{Palette, with_alpha};

/// Tick range shown by the timeline: from the oldest stored snapshot (or
/// the current tick) to the newest of the current tick and the snapshots.
/// The range is never empty.
pub(crate) fn timeline_domain(history: &[u64], current: u64) -> (u64, u64) {
    let lo = history.iter().copied().min().unwrap_or(current).min(current);
    let hi = history.iter().copied().max().unwrap_or(current).max(current);
    (lo, hi.max(lo + 1))
}

/// Horizontal position of `tick` in `[x0, x1]`.
pub(crate) fn timeline_x(tick: u64, domain: (u64, u64), x0: f32, x1: f32) -> f32 {
    let span = (domain.1 - domain.0) as f64;
    let f = (tick.saturating_sub(domain.0) as f64 / span).clamp(0.0, 1.0);
    x0 + (x1 - x0) * f as f32
}

/// Index of the marker closest to `x`, if within `max_distance` pixels.
pub(crate) fn nearest_marker(xs: &[f32], x: f32, max_distance: f32) -> Option<usize> {
    xs.iter()
        .enumerate()
        .map(|(i, &mx)| (i, (mx - x).abs()))
        .filter(|&(_, d)| d <= max_distance)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Draws the timeline; returns a command when a marker was clicked.
pub(crate) fn timeline(ui: &mut Ui, status: &RunnerStatus, palette: &Palette, width: f32) -> Option<ShellCommand> {
    let (rect, response) = ui.allocate_exact_size(vec2(width.max(60.0), 28.0), Sense::click());
    let domain = timeline_domain(&status.history_ticks, status.tick);
    let track = Rect::from_min_max(
        pos2(rect.min.x + 6.0, rect.center().y - 2.0),
        pos2(rect.max.x - 6.0, rect.center().y + 2.0),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(track, 2.0, palette.surface_hi);
    let xs: Vec<f32> = status.history_ticks.iter().map(|&t| timeline_x(t, domain, track.min.x, track.max.x)).collect();
    let now_x = timeline_x(status.tick, domain, track.min.x, track.max.x);
    painter.rect_filled(Rect::from_min_max(track.min, pos2(now_x, track.max.y)), 2.0, with_alpha(palette.accent, 0.35));
    let hover = response.hover_pos().and_then(|p| nearest_marker(&xs, p.x, 8.0));
    for (i, &x) in xs.iter().enumerate() {
        let hot = hover == Some(i);
        let h = if hot { 9.0 } else { 6.0 };
        let color = if hot { palette.accent } else { palette.text_muted };
        painter.line_segment(
            [pos2(x, rect.center().y - h), pos2(x, rect.center().y + h)],
            Stroke::new(if hot { 2.5 } else { 1.5 }, color),
        );
    }
    painter.circle_filled(pos2(now_x, rect.center().y), 5.5, palette.accent);
    painter.circle_stroke(pos2(now_x, rect.center().y), 5.5, Stroke::new(1.5, palette.panel));

    let response = match hover {
        Some(i) => response.on_hover_text(format!(
            "Snapshot at tick {} — click to go back to it (pauses; undoable)",
            format_count(status.history_ticks[i])
        )),
        None => response.on_hover_ui(|ui| {
            ui.label(RichText::new("History").strong());
            ui.label(format!(
                "{} stored snapshots ({}), ticks {}–{}.",
                status.history_ticks.len(),
                format_bytes(status.history_bytes as u64),
                format_count(domain.0),
                format_count(status.history_ticks.last().copied().unwrap_or(domain.0)),
            ));
            ui.label("Click a marker to restore that state and continue from there.");
        }),
    };
    if response.clicked() { hover.map(ShellCommand::JumpToHistory) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_covers_history_and_current_tick() {
        assert_eq!(timeline_domain(&[], 0), (0, 1));
        assert_eq!(timeline_domain(&[0, 50, 100], 120), (0, 120));
        assert_eq!(timeline_domain(&[200, 250], 220), (200, 250));
        assert_eq!(timeline_domain(&[200], 150), (150, 200));
    }

    #[test]
    fn positions_and_hit_testing() {
        let d = (0, 100);
        assert_eq!(timeline_x(0, d, 10.0, 110.0), 10.0);
        assert_eq!(timeline_x(50, d, 10.0, 110.0), 60.0);
        assert_eq!(timeline_x(500, d, 10.0, 110.0), 110.0);
        let xs = [10.0, 60.0, 64.0, 110.0];
        assert_eq!(nearest_marker(&xs, 63.0, 8.0), Some(2));
        assert_eq!(nearest_marker(&xs, 35.0, 8.0), None);
        assert_eq!(nearest_marker(&[], 1.0, 8.0), None);
    }
}
