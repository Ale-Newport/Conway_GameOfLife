//! The mode's panels: tools (left), properties with the inspector and the
//! analytics (right), and the viewport overlay; shared helpers.

pub mod analytics;
pub mod overlay;
pub mod properties;
pub mod tools;
pub mod web;

use bevy::prelude::Resource;
use bevy_egui::egui;
use egui_plot::{AxisHints, Corner, Legend, Line, Plot, PlotPoints};
use interface::Palette;
use interface::format::format_compact;

use crate::foodweb::FoodWeb;

/// Data the panels keep between frames.
#[derive(Resource, Default)]
pub struct UiCache {
    /// Food web and the frame sequence it was built from.
    pub web: Option<(u64, FoodWeb)>,
    /// Real time (s) the web was built.
    pub web_time: f64,
    /// Inspection kept for an organism that just died.
    pub last_inspection: Option<ecosystem::OrganismInspection>,
}

/// Compact number (`12.3 k`, `4.5 M`).
pub fn num(x: f64) -> String {
    format_compact(x)
}

/// Energy in readable units (kJ, MJ, GJ, TJ).
pub fn energy(kj: f64) -> String {
    let a = kj.abs();
    if a >= 1e9 {
        format!("{:.2} TJ", kj / 1e9)
    } else if a >= 1e6 {
        format!("{:.2} GJ", kj / 1e6)
    } else if a >= 1e3 {
        format!("{:.2} MJ", kj / 1e3)
    } else {
        format!("{kj:.1} kJ")
    }
}

/// Mass from kilograms (g, kg, t).
pub fn mass(kg: f64) -> String {
    let a = kg.abs();
    if a >= 1e3 {
        format!("{} t", num(kg / 1e3))
    } else if a >= 1.0 {
        format!("{kg:.1} kg")
    } else if a >= 1e-3 {
        format!("{:.1} g", kg * 1e3)
    } else {
        format!("{:.2} mg", kg * 1e6)
    }
}

/// Widest column of a key–value grid in a panel `width` wide: the first
/// column takes at most this, the last one the rest, so long texts wrap
/// instead of widening the panel.
pub fn grid_max_column(width: f32) -> f32 {
    ((width - GRID_SPACING) * 0.58).max(60.0)
}

/// Horizontal spacing of the key–value grids.
const GRID_SPACING: f32 = 12.0;

/// A key–value grid whose texts wrap to the panel width.
pub fn grid(ui: &mut egui::Ui, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([GRID_SPACING, 3.0])
        .max_col_width(grid_max_column(ui.available_width()))
        .striped(true)
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(egui::RichText::new(*k).weak());
                ui.label(v);
                ui.end_row();
            }
        });
}

/// A striped table of `columns` columns whose texts wrap so that it never
/// gets wider than the panel (the first column takes at most its share
/// plus a half, the last one the rest).
pub fn table(ui: &egui::Ui, id: &str, columns: usize) -> egui::Grid {
    let n = columns.max(1) as f32;
    let share = (ui.available_width() - GRID_SPACING * (n - 1.0)) / n;
    egui::Grid::new(id)
        .num_columns(columns)
        .spacing([GRID_SPACING, 3.0])
        .max_col_width((share * 1.5).max(40.0))
        .striped(true)
}

/// A labelled bar for a value in `[0, 1]` that always fits the panel:
/// fixed label column, the percentage on the right, the bar in between.
pub fn meter(
    ui: &mut egui::Ui,
    label: &str,
    label_width: f32,
    fraction: f32,
    value: &str,
    color: egui::Color32,
    palette: &Palette,
) {
    ui.horizontal(|ui| {
        ui.add_sized([label_width, 16.0], egui::Label::new(egui::RichText::new(label).weak()).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value);
            let width = ui.available_width().max(0.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 3.0, palette.surface_hi);
            let mut fill = rect;
            fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
            painter.rect_filled(fill, 3.0, color);
        });
    });
}

/// A thin horizontal bar for a fraction in `[0, 1]`.
pub fn bar(ui: &mut egui::Ui, label: &str, fraction: f32, color: egui::Color32, palette: &Palette) {
    let percent = format!("{:.0} %", fraction.clamp(0.0, 1.0) * 100.0);
    meter(ui, label, 96.0, fraction, &percent, color, palette);
}

/// A slider with its label on the line above, as wide as the panel allows
/// (a label beside a wide slider would widen the panel).
pub fn slider<'a>(ui: &mut egui::Ui, label: &str, slider: egui::Slider<'a>) -> egui::Response {
    ui.label(egui::RichText::new(label).weak());
    ui.scope(|ui| {
        // Room for the value box next to the track.
        ui.spacing_mut().slider_width = (ui.available_width() - SLIDER_VALUE_WIDTH).max(60.0);
        ui.add(slider)
    })
    .inner
}

/// Width kept for a slider's value box.
pub const SLIDER_VALUE_WIDTH: f32 = 76.0;

/// One plotted series: label, points `[time, value]` (time in the units of
/// a [`TimeAxis`]), colour.
pub type Series = (String, Vec<[f64; 2]>, egui::Color32);

/// The time axis of the plots: simulated days for the ecological rules,
/// ticks for the NetLogo rules (whose dynamics are documented in ticks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeAxis {
    /// Ticks per plotted unit.
    pub ticks_per_unit: f64,
    /// Axis label.
    pub label: &'static str,
}

impl TimeAxis {
    /// Days of `ticks_per_day` ticks, or ticks (`netlogo`).
    pub fn new(ticks_per_day: u32, netlogo: bool) -> Self {
        if netlogo {
            Self { ticks_per_unit: 1.0, label: "tick" }
        } else {
            Self { ticks_per_unit: f64::from(ticks_per_day.max(1)), label: "day" }
        }
    }

    /// Plotted time of a tick.
    pub fn at(&self, tick: u64) -> f64 {
        tick as f64 / self.ticks_per_unit
    }
}

/// Minimum spacing (points) of the x-axis tick labels: day and tick numbers
/// are short (egui_plot's default of 60 points hides them on a narrow panel).
const X_LABEL_SPACING: std::ops::RangeInclusive<f32> = 26.0..=44.0;

/// Minimum spacing (points) of the y-axis tick labels of linear plots
/// (the default hides them all on short plots of small values).
const Y_LABEL_SPACING: std::ops::RangeInclusive<f32> = 14.0..=24.0;

/// Minimum spacing (points) of the decade labels of a log axis.
const LOG_LABEL_SPACING: std::ops::RangeInclusive<f32> = 9.0..=12.0;

/// Time-series plot over a [`TimeAxis`]; `log` plots log10(1 + y).
pub fn time_plot(
    ui: &mut egui::Ui,
    axis: TimeAxis,
    id: &str,
    series: Vec<Series>,
    height: f32,
    y_label: &str,
    log: bool,
) {
    let mut plot = Plot::new(id)
        .height(height)
        .legend(
            Legend::default()
                .position(Corner::LeftTop)
                .text_style(egui::TextStyle::Small)
                .background_alpha(0.8)
                .follow_insertion_order(true),
        )
        .custom_x_axes(vec![AxisHints::new_x().label(axis.label).label_spacing(X_LABEL_SPACING)])
        .y_axis_label(y_label.to_string())
        .allow_scroll(false)
        .include_y(0.0);
    if log {
        // Decades only, labelled down to a tighter spacing (a log axis over
        // eight decades in a short plot would otherwise hide them all).
        plot = plot.custom_y_axes(vec![
            AxisHints::new_y()
                .label(y_label.to_string())
                .label_spacing(LOG_LABEL_SPACING)
                .formatter(|mark, _| log_axis_label(mark.value)),
        ]);
    } else {
        plot = plot.custom_y_axes(vec![AxisHints::new_y().label(y_label.to_string()).label_spacing(Y_LABEL_SPACING)]);
    }
    plot.show(ui, |p| {
        for (name, points, color) in series {
            let points: Vec<[f64; 2]> =
                if log { points.into_iter().map(|[x, y]| [x, (1.0 + y.max(0.0)).log10()]).collect() } else { points };
            p.line(Line::new(name, PlotPoints::from(points)).color(color).width(1.6));
        }
    });
}

/// Label of a log-axis mark: values are drawn at `log10(1 + y)`, so the
/// baseline is zero (not 1) and each further decade is labelled with its
/// power of ten; marks between decades stay unlabelled.
pub fn log_axis_label(mark: f64) -> String {
    if (mark - mark.round()).abs() > 1e-6 {
        String::new()
    } else if mark.round() == 0.0 {
        "0".into()
    } else {
        num(10f64.powf(mark.round()))
    }
}

/// Mean over a sliding window of `window` samples (centred), for per-tick
/// flows that the model books in hourly or daily bursts.
pub fn moving_mean(points: &[[f64; 2]], window: usize) -> Vec<[f64; 2]> {
    let w = window.max(1);
    if w == 1 || points.len() < 2 {
        return points.to_vec();
    }
    let mut prefix = Vec::with_capacity(points.len() + 1);
    prefix.push(0.0);
    for p in points {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + p[1]);
    }
    (0..points.len())
        .map(|i| {
            let lo = i.saturating_sub(w / 2);
            let hi = (i + w - w / 2).min(points.len());
            [points[i][0], (prefix[hi] - prefix[lo]) / (hi - lo) as f64]
        })
        .collect()
}

/// Section title.
pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).strong().size(14.0));
}

/// Muted explanatory text.
pub fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak().size(11.5));
}

/// egui colour of an sRGB triple.
pub fn color32(rgb: [f32; 3]) -> egui::Color32 {
    egui::Color32::from_rgb((rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_axis_starts_at_zero() {
        // log10(1 + 0) = 0: a zero flow sits on the baseline, labelled 0.
        assert_eq!(log_axis_label(0.0), "0");
        assert_eq!(log_axis_label(3.0), num(1000.0));
        assert_eq!(log_axis_label(2.5), "");
    }

    #[test]
    fn units_scale() {
        assert_eq!(energy(12.0), "12.0 kJ");
        assert_eq!(energy(2_500.0), "2.50 MJ");
        assert_eq!(energy(3.0e9), "3.00 TJ");
        assert_eq!(mass(0.5), "500.0 g");
        assert_eq!(mass(12.0), "12.0 kg");
        assert!(mass(2_000.0).ends_with(" t"));
        assert_eq!(color32([1.0, 0.0, 0.5]), egui::Color32::from_rgb(255, 0, 127));
        let spikes: Vec<[f64; 2]> = (0..20).map(|i| [i as f64, if i % 10 == 0 { 10.0 } else { 0.0 }]).collect();
        let smooth = moving_mean(&spikes, 10);
        assert_eq!(smooth.len(), 20);
        assert!((smooth[10][1] - 1.0).abs() < 1e-9, "{:?}", smooth[10]);
        assert_eq!(moving_mean(&spikes, 1), spikes);
    }
}
