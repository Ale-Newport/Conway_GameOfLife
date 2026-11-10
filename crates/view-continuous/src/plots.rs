//! Curves of the current rule — kernel radial profiles and growth (or
//! transition) functions — and their `egui_plot` charts.
//!
//! The curves are evaluated with the model crate's own functions
//! (`lenia_shell`, `GrowthFunction::eval`, `antialiased_disc`,
//! `gaussian_rings`, `SmoothLifeRule::transition`), so the plots always show
//! what the simulation executes.

use bevy_egui::egui::{self, Color32, RichText, Ui};
use continuous::kernel::{antialiased_disc, gaussian_rings, lenia_shell};
use continuous::{GrowthFunction, Rule};
use egui_plot::{AxisHints, HLine, Line, LineStyle, Plot, PlotPoints, Span, VLine};
use interface::Palette;
use interface::theme::with_alpha;
use interface::widgets::legend_row;

/// A named polyline.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    /// Legend name.
    pub name: String,
    /// Points `[x, y]`.
    pub points: Vec<[f64; 2]>,
}

/// The growth (or transition) chart of one kernel.
#[derive(Clone, Debug, PartialEq)]
pub struct GrowthChart {
    /// Curves to draw.
    pub curves: Vec<Curve>,
    /// Growth centre μ (vertical marker), if the model has one.
    pub mu: Option<f64>,
    /// Interval of potentials with positive growth (shaded).
    pub positive: Option<(f64, f64)>,
    /// Largest x shown.
    pub x_max: f64,
    /// x-axis label.
    pub x_label: &'static str,
    /// y-axis label.
    pub y_label: &'static str,
}

/// Number of kernels of a rule (SmoothLife has its disc/annulus pair).
pub fn kernel_count(rule: &Rule) -> usize {
    match rule {
        Rule::Lenia(r) => r.kernels.len(),
        Rule::SmoothLife(_) => 1,
        Rule::FlowLenia(r) => r.kernels.len(),
    }
}

fn sample(n: usize, x_max: f64, f: impl Fn(f64) -> f64) -> Vec<[f64; 2]> {
    let n = n.max(2);
    (0..n)
        .map(|i| {
            let x = x_max * i as f64 / (n - 1) as f64;
            [x, f(x)]
        })
        .collect()
}

/// Radial profile of kernel `k` against the distance in cells. Lenia and
/// Flow-Lenia: the shell / ring sum before normalisation (the simulation
/// divides by `Σ K` so the discrete kernel sums to 1); SmoothLife: the disc
/// (`m`) and annulus (`n`) weights.
pub fn kernel_curves(rule: &Rule, k: usize, samples: usize) -> Vec<Curve> {
    match rule {
        Rule::Lenia(r) => {
            let Some(kernel) = r.kernels.get(k) else { return Vec::new() };
            let radius = r.radius * kernel.relative_radius;
            let points = sample(samples, radius * 1.05, |d| lenia_shell(d / radius, &kernel.beta, kernel.core));
            vec![Curve { name: format!("K{}(d)", k + 1), points }]
        }
        Rule::SmoothLife(r) => {
            let (ra, ri, b) = (r.outer_radius, r.inner_radius(), r.rim);
            let x_max = ra + b / 2.0 + 1.0;
            vec![
                Curve { name: "disc (m)".into(), points: sample(samples, x_max, |d| antialiased_disc(d, ri, b)) },
                Curve {
                    name: "annulus (n)".into(),
                    points: sample(samples, x_max, |d| {
                        (antialiased_disc(d, ra, b) - antialiased_disc(d, ri, b)).max(0.0)
                    }),
                },
            ]
        }
        Rule::FlowLenia(r) => {
            let Some(kernel) = r.kernels.get(k) else { return Vec::new() };
            let radius = r.radius * kernel.relative_radius;
            let points = sample(samples, radius * 1.05, |d| gaussian_rings(d / radius, &kernel.rings));
            vec![Curve { name: format!("K{}(d)", k + 1), points }]
        }
    }
}

/// Potentials `u` with `G(u) > 0` for a growth function centred at `mu`
/// with width `sigma`: Gaussian `|u − μ| < σ·√(2 ln 2)`, polynomial
/// `|u − μ| < 3σ·√(1 − 2^(−1/4))`, rectangular `|u − μ| ≤ σ`.
pub fn positive_growth_band(growth: GrowthFunction, mu: f64, sigma: f64) -> (f64, f64) {
    let half = match growth {
        GrowthFunction::Gaussian => sigma * (2.0 * std::f64::consts::LN_2).sqrt(),
        GrowthFunction::Polynomial => 3.0 * sigma * (1.0 - 2f64.powf(-0.25)).sqrt(),
        GrowthFunction::Rectangular => sigma,
    };
    (mu - half, mu + half)
}

fn growth_x_max(mu: f64, sigma: f64) -> f64 {
    (mu + 6.0 * sigma).max(2.0 * mu).clamp(0.05, 1.0)
}

/// Growth chart of kernel `k`: Lenia `G_k(u)`, Flow-Lenia's Gaussian
/// `G_k(u)`, SmoothLife's transition `s(n, m)` for a dead (`m = 0`) and a
/// live (`m = 1`) cell.
pub fn growth_chart(rule: &Rule, k: usize, samples: usize) -> Option<GrowthChart> {
    match rule {
        Rule::Lenia(r) => {
            let kernel = r.kernels.get(k)?;
            let (mu, sigma, g) = (kernel.mu, kernel.sigma, kernel.growth);
            let x_max = growth_x_max(mu, sigma);
            Some(GrowthChart {
                curves: vec![Curve {
                    name: format!("G{}(u)", k + 1),
                    points: sample(samples, x_max, |u| g.eval(u, mu, sigma)),
                }],
                mu: Some(mu),
                positive: Some(positive_growth_band(g, mu, sigma)),
                x_max,
                x_label: "potential u = K * A",
                y_label: "growth",
            })
        }
        Rule::FlowLenia(r) => {
            let kernel = r.kernels.get(k)?;
            let (mu, sigma) = (kernel.mu, kernel.sigma);
            let g = GrowthFunction::Gaussian;
            let x_max = growth_x_max(mu, sigma);
            Some(GrowthChart {
                curves: vec![Curve {
                    name: format!("G{}(u)", k + 1),
                    points: sample(samples, x_max, |u| g.eval(u, mu, sigma)),
                }],
                mu: Some(mu),
                positive: Some(positive_growth_band(g, mu, sigma)),
                x_max,
                x_label: "potential u = K * A",
                y_label: "affinity term",
            })
        }
        Rule::SmoothLife(r) => Some(GrowthChart {
            curves: vec![
                Curve { name: "dead cell (m = 0)".into(), points: sample(samples, 1.0, |n| r.transition(n, 0.0)) },
                Curve { name: "live cell (m = 1)".into(), points: sample(samples, 1.0, |n| r.transition(n, 1.0)) },
            ],
            mu: None,
            positive: None,
            x_max: 1.0,
            x_label: "outer filling n",
            y_label: "s(n, m)",
        }),
    }
}

/// One-line description of kernel `k` of the live rule.
pub fn kernel_summary(rule: &Rule, k: usize) -> String {
    match rule {
        Rule::Lenia(r) => r.kernels.get(k).map_or_else(String::new, |kernel| {
            let beta = continuous::sim::params::format_list(&kernel.beta);
            let route = if r.channels > 1 {
                format!("channel {} to {} · ", kernel.source, kernel.destination)
            } else {
                String::new()
            };
            format!(
                "{route}radius {:.1} cells · β = [{beta}] · {} core · {} growth μ = {}, σ = {}, h = {}",
                r.radius * kernel.relative_radius,
                kernel.core.label(),
                kernel.growth.label(),
                kernel.mu,
                kernel.sigma,
                kernel.weight
            )
        }),
        Rule::SmoothLife(r) => format!(
            "outer radius {} cells, inner {:.2} cells, rim {} · birth [{}, {}] · survival [{}, {}] · α_n = {}, α_m = {}",
            r.outer_radius,
            r.inner_radius(),
            r.rim,
            r.birth[0],
            r.birth[1],
            r.survival[0],
            r.survival[1],
            r.alpha_n,
            r.alpha_m
        ),
        Rule::FlowLenia(r) => r.kernels.get(k).map_or_else(String::new, |kernel| {
            let route = if r.channels > 1 {
                format!("channel {} to {} · ", kernel.source, kernel.destination)
            } else {
                String::new()
            };
            format!(
                "{route}radius {:.1} cells · {} rings · Gaussian growth μ = {}, σ = {}, h = {}",
                r.radius * kernel.relative_radius,
                kernel.rings.len(),
                kernel.mu,
                kernel.sigma,
                kernel.weight
            )
        }),
    }
}

/// Label of kernel `k` in pickers.
pub fn kernel_label(rule: &Rule, k: usize) -> String {
    match rule {
        Rule::Lenia(r) if r.channels > 1 => r.kernels.get(k).map_or_else(
            || format!("Kernel {}", k + 1),
            |kernel| format!("Kernel {} (channel {} to {})", k + 1, kernel.source, kernel.destination),
        ),
        Rule::FlowLenia(r) if r.channels > 1 => r.kernels.get(k).map_or_else(
            || format!("Kernel {}", k + 1),
            |kernel| format!("Kernel {} (channel {} to {})", k + 1, kernel.source, kernel.destination),
        ),
        Rule::SmoothLife(_) => "Disc and annulus".to_string(),
        _ => format!("Kernel {}", k + 1),
    }
}

/// Colours of the curves of a chart: the mode accent, then a contrasting
/// series colour (series 1 is a blue close to this mode's accent).
fn curve_colors(palette: &Palette) -> [Color32; 2] {
    [palette.accent, palette.series_color(2)]
}

/// A compact, non-interactive chart. The legend is drawn above the chart
/// (see [`legend_row`]) so it never hides the curves, and x tick labels are
/// allowed closer together than egui_plot's default (60 px), which would
/// leave a 300 px wide chart with a single label.
fn base_plot(id: &str, height: f32, palette: &Palette, x_label: &str) -> Plot<'static> {
    Plot::new(id.to_string())
        .height(height)
        .allow_scroll(false)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .show_background(false)
        .grid_color(with_alpha(palette.text_muted, 0.35))
        .custom_x_axes(vec![AxisHints::new_x().label(x_label.to_string()).label_spacing(26.0..=40.0)])
}

/// Kernel profile chart.
pub fn kernel_plot(ui: &mut Ui, rule: &Rule, k: usize, palette: &Palette) {
    let curves = kernel_curves(rule, k, 240);
    if curves.is_empty() {
        ui.label(RichText::new("No kernel to show.").weak());
        return;
    }
    let colors = curve_colors(palette);
    let entries: Vec<(Color32, &str)> =
        curves.iter().enumerate().map(|(i, c)| (colors[i % colors.len()], c.name.as_str())).collect();
    legend_row(ui, &entries);
    base_plot("cl_kernel_plot", 120.0, palette, "distance (cells)")
        .y_axis_label("weight")
        .include_y(0.0)
        .include_y(1.05)
        .show(ui, |plot| {
            for (i, curve) in curves.iter().enumerate() {
                let color = colors[i % colors.len()];
                plot.line(
                    Line::new(curve.name.clone(), PlotPoints::from(curve.points.clone()))
                        .color(color)
                        .width(1.8)
                        .fill(0.0)
                        .fill_alpha(0.12),
                );
            }
        });
}

/// Growth (transition) chart with the μ marker and the band of positive growth.
pub fn growth_plot(ui: &mut Ui, rule: &Rule, k: usize, palette: &Palette) {
    let Some(chart) = growth_chart(rule, k, 300) else {
        ui.label(RichText::new("No growth function to show.").weak());
        return;
    };
    let colors = curve_colors(palette);
    let zero = with_alpha(palette.text_muted, 0.7);
    let band_fill: Color32 = with_alpha(palette.success, 0.14);
    let mut entries: Vec<(Color32, &str)> =
        chart.curves.iter().enumerate().map(|(i, c)| (colors[i % colors.len()], c.name.as_str())).collect();
    if chart.mu.is_some() {
        entries.push((palette.warning, "μ"));
    }
    if chart.positive.is_some() {
        entries.push((with_alpha(palette.success, 0.5), "G > 0"));
    }
    legend_row(ui, &entries);
    let y_min = if chart.mu.is_some() { -1.05 } else { -0.05 };
    base_plot("cl_growth_plot", 120.0, palette, chart.x_label)
        .y_axis_label(chart.y_label)
        .include_x(0.0)
        .include_x(chart.x_max)
        .include_y(y_min)
        .include_y(1.05)
        .show(ui, |plot| {
            if let Some((lo, hi)) = chart.positive {
                // Unnamed: egui_plot writes a span's name inside the chart.
                plot.add(Span::new("", lo.max(0.0)..=hi).fill(band_fill).border_width(0.0));
            }
            if chart.mu.is_some() {
                plot.add(HLine::new("", 0.0).color(zero).width(1.0).style(LineStyle::dashed_dense()));
            }
            if let Some(mu) = chart.mu {
                plot.add(VLine::new("μ", mu).color(palette.warning).width(1.2));
            }
            for (i, curve) in chart.curves.iter().enumerate() {
                plot.line(
                    Line::new(curve.name.clone(), PlotPoints::from(curve.points.clone()))
                        .color(colors[i % colors.len()])
                        .width(1.8),
                );
            }
        });
}

/// Small time-series chart of one metric (`points` are `[tick, value]`);
/// `caption` is the legend above it (name and latest value).
pub fn series_plot(
    ui: &mut Ui,
    id: &str,
    name: &str,
    caption: &str,
    points: Vec<[f64; 2]>,
    color: Color32,
    palette: &Palette,
) {
    if points.is_empty() {
        ui.label(RichText::new(format!("{name}: run the simulation to record it.")).size(12.0).weak());
        return;
    }
    legend_row(ui, &[(color, caption)]);
    base_plot(id, 90.0, palette, "tick").link_axis(egui::Id::new("cl_dynamics"), [true, false]).show(ui, |plot| {
        plot.line(Line::new(name.to_string(), PlotPoints::from(points)).color(color).width(1.6));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use continuous::scenario::builtin_by_id;

    fn rule(id: &str) -> Rule {
        builtin_by_id(id).expect("builtin scenario").rule
    }

    #[test]
    fn lenia_kernel_peaks_at_the_ring_and_vanishes_outside() {
        let r = rule("lenia.orbium");
        let curves = kernel_curves(&r, 0, 261);
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0].name, "K1(d)", "kernels are numbered from 1, like the parameter groups");
        let pts = &curves[0].points;
        // Gaussian-bump core with one ring: maximum at half the radius (6.5 cells).
        let peak = pts.iter().copied().fold([0.0, f64::MIN], |a, p| if p[1] > a[1] { p } else { a });
        assert!((peak[0] - 6.5).abs() < 0.2, "peak at {peak:?}");
        assert!((peak[1] - 1.0).abs() < 1e-3);
        assert!(pts.iter().filter(|p| p[0] >= 13.0).all(|p| p[1] == 0.0), "zero beyond R");
        assert!(kernel_curves(&r, 3, 10).is_empty());
    }

    #[test]
    fn growth_is_one_at_mu_and_changes_sign_at_the_band_edges() {
        for g in GrowthFunction::ALL {
            let (mu, sigma) = (0.15, 0.015);
            assert!((g.eval(mu, mu, sigma) - 1.0).abs() < 1e-12, "{g:?}");
            let (lo, hi) = positive_growth_band(g, mu, sigma);
            assert!((hi - mu - (mu - lo)).abs() < 1e-12, "symmetric band");
            if g != GrowthFunction::Rectangular {
                assert!(g.eval(hi, mu, sigma).abs() < 1e-9, "{g:?}: G = 0 at the band edge");
                assert!(g.eval(lo, mu, sigma).abs() < 1e-9);
            }
            assert!(g.eval(hi + 1e-3, mu, sigma) < 0.0 && g.eval(hi - 1e-3, mu, sigma) > 0.0, "{g:?}");
        }
    }

    #[test]
    fn growth_charts_follow_the_model() {
        let chart = growth_chart(&rule("lenia.orbium"), 0, 101).unwrap();
        assert_eq!(chart.mu, Some(0.15));
        assert_eq!(chart.curves[0].name, "G1(u)");
        assert!((chart.x_max - 0.3).abs() < 1e-12, "x range shows the whole bump");
        let y: Vec<f64> = chart.curves[0].points.iter().map(|p| p[1]).collect();
        assert!(y.iter().all(|v| (-1.0..=1.0).contains(v)));
        assert!((y[0] + 1.0).abs() < 1e-9, "no growth in empty space");
        // SmoothLife: transition curves for dead and live cells, in [0, 1].
        let sl = growth_chart(&rule("smoothlife.glider"), 0, 201).unwrap();
        assert_eq!(sl.curves.len(), 2);
        assert!(sl.mu.is_none());
        let birth_peak =
            sl.curves[0].points.iter().copied().fold([0.0, f64::MIN], |a, p| if p[1] > a[1] { p } else { a });
        assert!((0.278..=0.365).contains(&birth_peak[0]), "births happen inside [b1, b2]: {birth_peak:?}");
        // Flow-Lenia: one Gaussian growth per kernel.
        let flow = rule("flow_lenia.demo");
        assert_eq!(kernel_count(&flow), 7);
        assert!(growth_chart(&flow, 6, 50).is_some());
        assert!(growth_chart(&flow, 7, 50).is_none());
    }

    #[test]
    fn smoothlife_kernels_split_disc_and_annulus() {
        let curves = kernel_curves(&rule("smoothlife.glider"), 0, 512);
        assert_eq!(curves.len(), 2);
        let at = |c: &Curve, d: f64| {
            c.points.iter().min_by(|a, b| (a[0] - d).abs().total_cmp(&(b[0] - d).abs())).unwrap()[1]
        };
        assert_eq!(at(&curves[0], 3.0), 1.0, "inside the inner disc (r_i = 7)");
        assert_eq!(at(&curves[1], 3.0), 0.0);
        assert_eq!(at(&curves[0], 14.0), 0.0);
        assert_eq!(at(&curves[1], 14.0), 1.0, "inside the annulus");
    }

    #[test]
    fn summaries_and_labels() {
        let t = rule("lenia.tessellatium_gyrans");
        assert_eq!(kernel_count(&t), 15);
        assert!(kernel_label(&t, 0).contains("channel 0 to 0"));
        assert!(kernel_summary(&t, 0).starts_with("channel 0 to 0"));
        let o = rule("lenia.orbium");
        assert_eq!(kernel_label(&o, 0), "Kernel 1");
        let s = kernel_summary(&o, 0);
        assert!(s.contains("μ = 0.15") && s.contains("σ = 0.015") && s.contains("13.0 cells"), "{s}");
        assert_eq!(kernel_label(&rule("smoothlife.glider"), 0), "Disc and annulus");
    }
}
