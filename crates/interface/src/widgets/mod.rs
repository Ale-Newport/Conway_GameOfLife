//! Reusable widgets for mode panels: parameter editor, metric plots,
//! histogram, legends and swatches, Markdown rules.

pub mod histogram;
pub mod legend;
pub mod markdown;
pub mod metrics;
pub mod params;
pub mod rules;

pub use histogram::{histogram_bars, histogram_plot};
pub use legend::{badge, color_ramp, gradient_bar, legend_chip, legend_row, swatch};
pub use markdown::markdown;
pub use metrics::{MetricsPanelState, PlotWindow, metrics_panel};
pub use params::param_editor;
pub use rules::rules_panel;
