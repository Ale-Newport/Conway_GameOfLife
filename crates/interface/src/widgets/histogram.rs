//! Histogram plot for [`analytics::Histogram`].

use analytics::Histogram;
use bevy_egui::egui::{self, Color32, RichText, Ui};
use egui_plot::{Bar, BarChart, Plot};

use crate::format::{format_compact, format_count};

/// Bars of a histogram: one per bin, centred on the bin with its full
/// width, height = count.
pub fn histogram_bars(histogram: &Histogram) -> Vec<Bar> {
    let width = histogram.bin_width();
    histogram
        .centers()
        .into_iter()
        .zip(histogram.counts())
        .map(|(center, &count)| Bar::new(center, count as f64).width(width))
        .collect()
}

/// Draws `histogram` as a bar chart `height` points tall. `x_label` names
/// the measured quantity. Values outside the range are reported below.
pub fn histogram_plot(
    ui: &mut Ui,
    id: impl egui::AsId,
    histogram: &Histogram,
    color: Color32,
    height: f32,
    x_label: &str,
) {
    let chart = BarChart::new(x_label.to_string(), histogram_bars(histogram)).color(color);
    Plot::new(id)
        .height(height)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .x_axis_label(x_label.to_string())
        .y_axis_label("count")
        .include_y(0.0)
        .show(ui, |plot_ui| plot_ui.bar_chart(chart));
    let (lo, hi) = histogram.range();
    let mut note = format!(
        "{} values in {} bins over [{}, {}]",
        format_count(histogram.in_range()),
        histogram.bins(),
        format_compact(lo),
        format_compact(hi)
    );
    if histogram.underflow() + histogram.overflow() > 0 {
        note.push_str(&format!(
            "; {} below, {} above",
            format_count(histogram.underflow()),
            format_count(histogram.overflow())
        ));
    }
    ui.label(RichText::new(note).small().weak());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_cover_bins() {
        let h = Histogram::fixed(&[0.0, 1.0, 1.0, 2.0, 4.0], 4).unwrap();
        let bars = histogram_bars(&h);
        assert_eq!(bars.len(), 4);
        let heights: Vec<f64> = bars.iter().map(|b| b.value).collect();
        assert_eq!(heights.iter().sum::<f64>(), 5.0);
        assert!((bars[0].argument - 0.5).abs() < 1e-12);
        assert!((bars[0].bar_width - 1.0).abs() < 1e-12);
    }
}
