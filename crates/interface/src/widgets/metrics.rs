//! Live metric plots fed by [`crate::session::MetricsHistory`].

use analytics::{AnalyticsError, BinRule, DownsampleMethod, Histogram, RunningStats, Series, TimeSeriesStore};
use bevy_egui::egui::{self, Color32, RichText, Ui};
use egui_plot::{Corner, Legend, Line, Plot, PlotPoints};
use simulation_core::MetricSpec;

use crate::format::format_compact;
use crate::theme::Palette;
use crate::widgets::histogram::histogram_plot;
use crate::widgets::legend::swatch;

/// Time range shown by the plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlotWindow {
    /// The whole recorded run.
    #[default]
    All,
    /// The most recent `n` ticks.
    Last(u64),
}

impl PlotWindow {
    const CHOICES: [PlotWindow; 4] =
        [PlotWindow::All, PlotWindow::Last(100), PlotWindow::Last(1_000), PlotWindow::Last(10_000)];

    fn label(self) -> String {
        match self {
            PlotWindow::All => "All".to_string(),
            PlotWindow::Last(n) => format!("Last {}", format_compact(n as f64)),
        }
    }

    /// Inclusive tick range for a store whose newest sample is `last_tick`.
    pub fn range(self, last_tick: u64) -> (u64, u64) {
        match self {
            PlotWindow::All => (0, last_tick),
            PlotWindow::Last(n) => (last_tick.saturating_sub(n.saturating_sub(1)), last_tick),
        }
    }
}

/// Selection state of [`metrics_panel`]. Keep one per panel; reset it when
/// the model changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricsPanelState {
    /// Keys of the plotted metrics. `None` until the default is chosen.
    pub selected: Option<Vec<String>>,
    /// Time range.
    pub window: PlotWindow,
    /// Show a histogram of the first selected metric.
    pub distribution: bool,
    histogram: Option<HistogramCache>,
}

/// Seconds between histogram updates while samples arrive. Binning with
/// Freedman–Diaconis sorts the shown samples (up to 100 000 per metric), too
/// much work to repeat every frame.
pub const HISTOGRAM_REFRESH_S: f64 = 0.25;

/// The last computed histogram and the data it describes.
#[derive(Clone, Debug, PartialEq)]
struct HistogramCache {
    key: String,
    window: PlotWindow,
    last_tick: u64,
    samples: usize,
    computed_at: f64,
    histogram: Result<Histogram, AnalyticsError>,
}

/// Whether the histogram of `key` over `window` must be recomputed at time
/// `now` (seconds): always for another metric or range, never for unchanged
/// data, and at most every [`HISTOGRAM_REFRESH_S`] while data arrives.
fn histogram_stale(
    cache: Option<&HistogramCache>,
    key: &str,
    window: PlotWindow,
    data: (u64, usize),
    now: f64,
) -> bool {
    match cache {
        None => true,
        Some(c) if c.key != key || c.window != window => true,
        Some(c) if (c.last_tick, c.samples) == data => false,
        Some(c) => now - c.computed_at >= HISTOGRAM_REFRESH_S,
    }
}

/// Keys to offer: declared metrics first (in declaration order), then any
/// recorded key that was not declared.
pub fn metric_keys(specs: &[MetricSpec], store: &TimeSeriesStore) -> Vec<String> {
    let mut keys: Vec<String> = specs.iter().map(|s| s.key.clone()).collect();
    for key in store.keys() {
        if !keys.iter().any(|k| k == key) {
            keys.push(key.to_string());
        }
    }
    keys
}

/// Default selection: `population` when available, else the first metric.
pub fn default_selection(keys: &[String]) -> Vec<String> {
    keys.iter().find(|k| *k == "population").or_else(|| keys.first()).cloned().into_iter().collect()
}

/// Plot points of `series` in `window` (decimated to at most `max_points`
/// with M4 so extremes stay visible).
pub fn plot_points(series: &Series, window: PlotWindow, max_points: usize) -> Vec<[f64; 2]> {
    let Some(last) = series.last() else { return Vec::new() };
    let points = match window {
        PlotWindow::All => series.display(),
        PlotWindow::Last(_) => {
            let (start, end) = window.range(last.tick);
            series.downsample_window(start, end, max_points, DownsampleMethod::M4)
        }
    };
    points.into_iter().map(|p| [p.tick as f64, p.value]).collect()
}

fn spec_for<'a>(specs: &'a [MetricSpec], key: &str) -> Option<&'a MetricSpec> {
    specs.iter().find(|s| s.key == key)
}

fn display_name(specs: &[MetricSpec], key: &str) -> String {
    spec_for(specs, key).map_or_else(|| key.to_string(), |s| s.label.clone())
}

/// Metric picker, time-series plot, summary table and optional histogram.
/// The x axis shows ticks and is labelled `x_label` (use
/// [`crate::widgets::rules::tick_label`]: the tick unit only when one tick
/// is one unit).
pub fn metrics_panel(
    ui: &mut Ui,
    store: &TimeSeriesStore,
    specs: &[MetricSpec],
    x_label: &str,
    palette: &Palette,
    state: &mut MetricsPanelState,
) {
    let keys = metric_keys(specs, store);
    if keys.is_empty() {
        ui.label(RichText::new("This model publishes no metrics.").weak());
        return;
    }
    let mut selected = state.selected.take().unwrap_or_else(|| default_selection(&keys));
    let color_of = |key: &str| palette.series_color(keys.iter().position(|k| k == key).unwrap_or(0));

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for key in &keys {
            let on = selected.iter().any(|k| k == key);
            let label = display_name(specs, key);
            let text = RichText::new(label).size(12.5).color(if on { color_of(key) } else { palette.text_muted });
            let mut response = ui.selectable_label(on, text);
            if let Some(spec) = spec_for(specs, key) {
                let unit = if spec.unit.is_empty() { String::new() } else { format!(" [{}]", spec.unit) };
                response = response.on_hover_text(format!("{}{unit}\n{}", spec.label, spec.description));
            }
            if response.clicked() {
                if on {
                    selected.retain(|k| k != key);
                } else {
                    selected.push(key.clone());
                }
            }
        }
    });
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("metrics_window").selected_text(state.window.label()).width(110.0).show_ui(
            ui,
            |ui| {
                for choice in PlotWindow::CHOICES {
                    ui.selectable_value(&mut state.window, choice, choice.label());
                }
            },
        );
        ui.checkbox(&mut state.distribution, "Distribution")
            .on_hover_text("Histogram of the first selected metric over the shown range (Freedman–Diaconis bins).");
    });

    state.selected = Some(selected.clone());

    let lines: Vec<(String, Color32, Vec<[f64; 2]>)> = selected
        .iter()
        .filter_map(|key| store.get(key).map(|s| (key, s)))
        .map(|(key, series)| (display_name(specs, key), color_of(key), plot_points(series, state.window, 1_500)))
        .collect();
    Plot::new("shell_metrics_plot")
        .height(210.0)
        .legend(Legend::default().position(Corner::LeftTop))
        .x_axis_label(x_label.to_string())
        .allow_scroll(false)
        .show(ui, |plot_ui| {
            for (name, color, points) in lines {
                plot_ui.line(Line::new(name, PlotPoints::from(points)).color(color).width(1.6));
            }
        });

    let mut compacted = false;
    egui::Grid::new("metrics_summary").num_columns(5).striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
        for heading in ["Metric", "Latest", "Min", "Max", "Mean"] {
            ui.label(RichText::new(heading).small().weak());
        }
        ui.end_row();
        for key in selected.iter() {
            let Some(series) = store.get(key) else { continue };
            compacted |= series.is_compacted();
            let window = match series.last() {
                Some(last) => {
                    let (start, end) = state.window.range(last.tick);
                    series.window(start, end)
                }
                None => &[],
            };
            let stats = RunningStats::from_slice(&window.iter().map(|p| p.value).collect::<Vec<_>>()).ok();
            ui.horizontal(|ui| {
                swatch(ui, color_of(key), 10.0);
                ui.label(display_name(specs, key));
            });
            let cell = |v: Option<f64>| v.map_or_else(|| "—".to_string(), format_compact);
            ui.label(cell(series.last().map(|p| p.value)));
            ui.label(cell(stats.and_then(|s| s.min())));
            ui.label(cell(stats.and_then(|s| s.max())));
            ui.label(cell(stats.and_then(|s| s.mean())));
            ui.end_row();
        }
    });
    if compacted {
        ui.label(
            RichText::new(format!(
                "Long run: older samples are summarised (first/min/max/last per bucket) beyond {} per metric.",
                format_compact(crate::session::MAX_SAMPLES_PER_METRIC as f64)
            ))
            .small()
            .weak(),
        );
    }

    if state.distribution
        && let Some(key) = selected.first()
        && let Some(series) = store.get(key)
        && let Some(last) = series.last()
    {
        let now = ui.input(|i| i.time);
        let data = (last.tick, series.len());
        if histogram_stale(state.histogram.as_ref(), key, state.window, data, now) {
            let (start, end) = state.window.range(last.tick);
            let values: Vec<f64> = series.window(start, end).iter().map(|p| p.value).collect();
            state.histogram = Some(HistogramCache {
                key: key.clone(),
                window: state.window,
                last_tick: data.0,
                samples: data.1,
                computed_at: now,
                histogram: Histogram::auto(&values, BinRule::FreedmanDiaconis),
            });
        }
        ui.add_space(6.0);
        match state.histogram.as_ref().map(|c| &c.histogram) {
            Some(Ok(histogram)) => {
                histogram_plot(ui, "shell_metrics_hist", histogram, color_of(key), 140.0, &display_name(specs, key))
            }
            Some(Err(e)) => {
                ui.label(RichText::new(format!("No distribution: {e}")).small().weak());
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use simulation_core::Metrics;

    use super::*;

    fn store() -> TimeSeriesStore {
        let mut store = TimeSeriesStore::new();
        for t in 1..=50u64 {
            store.record(&Metrics::new(t, t as f64).with("births", (t % 3) as f64).with("extra", 1.0)).unwrap();
        }
        store
    }

    #[test]
    fn keys_and_default_selection() {
        let specs = vec![
            MetricSpec::counter("births", "Births", "cells born"),
            MetricSpec::gauge("population", "Population", "cells", "live cells"),
        ];
        let keys = metric_keys(&specs, &store());
        assert_eq!(keys, vec!["births", "population", "extra"]);
        assert_eq!(default_selection(&keys), vec!["population"]);
        assert_eq!(default_selection(&keys[..1]), vec!["births"]);
        assert!(default_selection(&[]).is_empty());
    }

    #[test]
    fn histogram_is_recomputed_only_when_needed() {
        let cache = HistogramCache {
            key: "population".into(),
            window: PlotWindow::All,
            last_tick: 100,
            samples: 100,
            computed_at: 10.0,
            histogram: Histogram::fixed(&[1.0, 2.0], 2),
        };
        let c = Some(&cache);
        assert!(histogram_stale(None, "population", PlotWindow::All, (100, 100), 10.0));
        assert!(!histogram_stale(c, "population", PlotWindow::All, (100, 100), 99.0), "unchanged data");
        assert!(histogram_stale(c, "births", PlotWindow::All, (100, 100), 10.0), "another metric");
        assert!(histogram_stale(c, "population", PlotWindow::Last(100), (100, 100), 10.0), "another range");
        assert!(!histogram_stale(c, "population", PlotWindow::All, (101, 101), 10.1), "throttled");
        assert!(histogram_stale(c, "population", PlotWindow::All, (101, 101), 10.0 + HISTOGRAM_REFRESH_S));
    }

    #[test]
    fn windows_select_recent_ticks() {
        assert_eq!(PlotWindow::Last(10).range(50), (41, 50));
        assert_eq!(PlotWindow::Last(100).range(50), (0, 50));
        assert_eq!(PlotWindow::All.range(7), (0, 7));
        let store = store();
        let series = store.get("births").unwrap();
        let points = plot_points(series, PlotWindow::Last(10), 1_000);
        assert_eq!(points.len(), 10);
        assert_eq!(points[0], [41.0, (41 % 3) as f64]);
        assert_eq!(plot_points(series, PlotWindow::All, 1_000).len(), 50);
    }
}
