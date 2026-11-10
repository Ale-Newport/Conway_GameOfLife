//! Visual themes: colour tokens, egui style and colour helpers.
//!
//! Every colour the shell paints comes from a [`Palette`], so the three
//! themes (dark lab, light, high contrast) stay consistent. The accent is
//! per mode ([`crate::mode::ModeCard::accent`]); the high-contrast theme
//! brightens it to keep at least 4.5:1 contrast against the panels.

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, CornerRadius, FontId, Margin, Shadow, Stroke, TextStyle};

/// User-selectable theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ThemeKind {
    /// Dark laboratory theme (default).
    #[default]
    Dark,
    /// Light theme for bright rooms and printing.
    Light,
    /// Maximum contrast: black surfaces, white text, thick strokes.
    HighContrast,
}

impl ThemeKind {
    /// Every theme.
    pub const ALL: [ThemeKind; 3] = [ThemeKind::Dark, ThemeKind::Light, ThemeKind::HighContrast];

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Dark => "Dark lab",
            ThemeKind::Light => "Light",
            ThemeKind::HighContrast => "High contrast",
        }
    }
}

/// The selected theme. Change it to switch themes; the shell re-applies the
/// egui style when it (or the active mode's accent,
/// [`crate::mode::ModeRegistry::accent`]) changes.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ThemeSettings {
    /// Selected theme.
    pub kind: ThemeKind,
}

/// Colour tokens of a theme with an accent applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Whether this is a dark theme.
    pub dark: bool,
    /// Window background (also the world clear colour around viewports).
    pub background: Color32,
    /// Panel fill.
    pub panel: Color32,
    /// Raised surface (cards, groups).
    pub surface: Color32,
    /// Hovered / more raised surface.
    pub surface_hi: Color32,
    /// Hairline separators and outlines.
    pub stroke: Color32,
    /// Primary text.
    pub text: Color32,
    /// Secondary text.
    pub text_muted: Color32,
    /// Mode accent.
    pub accent: Color32,
    /// Text drawn on top of the accent.
    pub on_accent: Color32,
    /// Positive state.
    pub success: Color32,
    /// Warning state.
    pub warning: Color32,
    /// Error state.
    pub error: Color32,
    /// Stroke width of outlines.
    pub stroke_width: f32,
}

impl Palette {
    /// Palette of `kind` with `accent`.
    pub fn new(kind: ThemeKind, accent: Color32) -> Self {
        match kind {
            ThemeKind::Dark => Self {
                dark: true,
                background: Color32::from_rgb(11, 14, 19),
                panel: Color32::from_rgb(17, 21, 28),
                surface: Color32::from_rgb(24, 29, 38),
                surface_hi: Color32::from_rgb(32, 39, 51),
                stroke: Color32::from_rgb(43, 51, 65),
                text: Color32::from_rgb(225, 230, 239),
                text_muted: Color32::from_rgb(140, 151, 170),
                accent,
                on_accent: on_color(accent),
                success: Color32::from_rgb(92, 201, 128),
                warning: Color32::from_rgb(240, 180, 70),
                error: Color32::from_rgb(240, 98, 98),
                stroke_width: 1.0,
            },
            ThemeKind::Light => {
                let accent = darken_for_light(accent);
                Self {
                    dark: false,
                    background: Color32::from_rgb(236, 239, 244),
                    panel: Color32::from_rgb(250, 251, 253),
                    surface: Color32::from_rgb(242, 244, 248),
                    surface_hi: Color32::from_rgb(229, 233, 240),
                    stroke: Color32::from_rgb(208, 214, 224),
                    text: Color32::from_rgb(25, 30, 38),
                    text_muted: Color32::from_rgb(88, 98, 114),
                    accent,
                    on_accent: on_color(accent),
                    success: Color32::from_rgb(30, 135, 70),
                    warning: Color32::from_rgb(170, 105, 0),
                    error: Color32::from_rgb(196, 40, 40),
                    stroke_width: 1.0,
                }
            }
            ThemeKind::HighContrast => {
                let accent = brighten_for_contrast(accent);
                Self {
                    dark: true,
                    background: Color32::BLACK,
                    panel: Color32::BLACK,
                    surface: Color32::from_rgb(14, 14, 14),
                    surface_hi: Color32::from_rgb(38, 38, 38),
                    stroke: Color32::from_rgb(235, 235, 235),
                    text: Color32::WHITE,
                    text_muted: Color32::from_rgb(220, 220, 220),
                    accent,
                    on_accent: Color32::BLACK,
                    success: Color32::from_rgb(110, 255, 140),
                    warning: Color32::from_rgb(255, 220, 0),
                    error: Color32::from_rgb(255, 110, 110),
                    stroke_width: 1.5,
                }
            }
        }
    }

    /// Distinct series colours for plots, starting with the accent.
    pub fn series_color(&self, index: usize) -> Color32 {
        const SERIES: [Color32; 8] = [
            Color32::from_rgb(90, 169, 255),
            Color32::from_rgb(255, 122, 144),
            Color32::from_rgb(108, 203, 95),
            Color32::from_rgb(242, 184, 75),
            Color32::from_rgb(199, 146, 234),
            Color32::from_rgb(79, 209, 197),
            Color32::from_rgb(245, 120, 60),
            Color32::from_rgb(170, 180, 200),
        ];
        if index == 0 {
            return self.accent;
        }
        let c = SERIES[(index - 1) % SERIES.len()];
        if self.dark { c } else { darken_for_light(c) }
    }
}

/// Relative luminance of an sRGB colour (WCAG 2.x definition).
pub fn relative_luminance(c: Color32) -> f32 {
    let channel = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// WCAG contrast ratio between two colours (1 to 21).
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Black or white, whichever reads better on `background`.
pub fn on_color(background: Color32) -> Color32 {
    let dark = Color32::from_rgb(12, 14, 18);
    if contrast_ratio(background, dark) >= contrast_ratio(background, Color32::WHITE) { dark } else { Color32::WHITE }
}

/// Linear interpolation between two colours in sRGB space (`t` in `[0, 1]`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()), lerp(a.a(), b.a()))
}

/// `color` with its alpha multiplied by `alpha` (`[0, 1]`).
pub fn with_alpha(color: Color32, alpha: f32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, (f32::from(a) * alpha.clamp(0.0, 1.0)).round() as u8)
}

fn darken_for_light(c: Color32) -> Color32 {
    let white = Color32::from_rgb(250, 251, 253);
    let mut out = c;
    let mut step = 0;
    while contrast_ratio(out, white) < 3.0 && step < 20 {
        out = mix(out, Color32::BLACK, 0.08);
        step += 1;
    }
    out
}

fn brighten_for_contrast(c: Color32) -> Color32 {
    let mut out = c;
    let mut step = 0;
    while contrast_ratio(out, Color32::BLACK) < 7.0 && step < 20 {
        out = mix(out, Color32::WHITE, 0.1);
        step += 1;
    }
    out
}

/// Quantises a simulation colour (`ParamValue::Color`: unmultiplied sRGB
/// RGBA in `[0, 1]`, the convention of `ParamValue::parse_loose("#rrggbb")`)
/// to 8-bit sRGB channels. Out-of-range and NaN channels are clamped.
pub fn srgba_bytes(rgba: [f32; 4]) -> [u8; 4] {
    let c = |v: f32| if v.is_nan() { 0 } else { (v.clamp(0.0, 1.0) * 255.0).round() as u8 };
    [c(rgba[0]), c(rgba[1]), c(rgba[2]), c(rgba[3])]
}

/// Inverse of [`srgba_bytes`]: 8-bit sRGB channels to `[0, 1]`.
pub fn rgba_from_srgba_bytes(bytes: [u8; 4]) -> [f32; 4] {
    bytes.map(|v| f32::from(v) / 255.0)
}

/// Converts a simulation colour (`ParamValue::Color`, RGBA in `[0, 1]`,
/// unmultiplied sRGB) to an egui colour.
pub fn color32_from_rgba(rgba: [f32; 4]) -> Color32 {
    let [r, g, b, a] = srgba_bytes(rgba);
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// `#rrggbb` (or `#rrggbbaa` when not opaque).
pub fn hex(color: Color32) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// Bevy colour of an egui colour (used for camera clear colours).
pub fn bevy_color(color: Color32) -> Color {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    Color::srgba_u8(r, g, b, a)
}

/// Applies the palette to an egui context: theme preference, visuals,
/// spacing and text sizes.
pub fn apply_palette(ctx: &egui::Context, palette: &Palette) {
    let theme = if palette.dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(theme);
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals(palette);
    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(8.0, 6.0);
    spacing.button_padding = egui::vec2(10.0, 5.0);
    spacing.interact_size = egui::vec2(36.0, 26.0);
    spacing.slider_width = 170.0;
    spacing.combo_width = 160.0;
    spacing.menu_margin = Margin::same(8);
    spacing.window_margin = Margin::same(14);
    spacing.indent = 16.0;
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(19.0)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    ctx.set_style_of(theme, style);
}

fn visuals(p: &Palette) -> egui::Visuals {
    let base = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let radius = CornerRadius::same(6);
    let mut widgets = base.widgets.clone();
    let hc = p.stroke_width > 1.0;
    let border = |c: Color32| Stroke::new(p.stroke_width, c);
    widgets.noninteractive.bg_fill = p.panel;
    widgets.noninteractive.weak_bg_fill = p.panel;
    widgets.noninteractive.bg_stroke = border(if hc { p.stroke } else { mix(p.stroke, p.panel, 0.2) });
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.noninteractive.corner_radius = radius;
    widgets.inactive.bg_fill = p.surface_hi;
    widgets.inactive.weak_bg_fill = p.surface;
    widgets.inactive.bg_stroke = border(if hc { p.stroke } else { Color32::TRANSPARENT });
    widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    widgets.inactive.corner_radius = radius;
    widgets.hovered.bg_fill = mix(p.surface_hi, p.accent, 0.18);
    widgets.hovered.weak_bg_fill = mix(p.surface_hi, p.accent, 0.12);
    widgets.hovered.bg_stroke = border(with_alpha(p.accent, 0.7));
    widgets.hovered.fg_stroke = Stroke::new(1.5, p.text);
    widgets.hovered.corner_radius = radius;
    widgets.active.bg_fill = mix(p.surface_hi, p.accent, 0.35);
    widgets.active.weak_bg_fill = mix(p.surface_hi, p.accent, 0.3);
    widgets.active.bg_stroke = border(p.accent);
    widgets.active.fg_stroke = Stroke::new(2.0, p.text);
    widgets.active.corner_radius = radius;
    widgets.open.bg_fill = p.surface_hi;
    widgets.open.weak_bg_fill = p.surface_hi;
    widgets.open.bg_stroke = border(p.stroke);
    widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    widgets.open.corner_radius = radius;
    let shadow_alpha = if p.dark { 110 } else { 40 };
    egui::Visuals {
        dark_mode: p.dark,
        override_text_color: Some(p.text),
        weak_text_color: Some(p.text_muted),
        widgets,
        selection: egui::style::Selection {
            // Opaque: bevy_egui blends in linear light (sRGB target), where a
            // translucent accent over the light panel washes out to grey.
            bg_fill: mix(p.panel, p.accent, 0.35),
            stroke: Stroke::new(1.0, p.accent),
        },
        hyperlink_color: p.accent,
        faint_bg_color: mix(p.panel, p.surface, 0.5),
        extreme_bg_color: if p.dark { mix(p.background, Color32::BLACK, 0.3) } else { Color32::WHITE },
        code_bg_color: p.surface_hi,
        warn_fg_color: p.warning,
        error_fg_color: p.error,
        window_corner_radius: CornerRadius::same(10),
        window_shadow: Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(shadow_alpha) },
        window_fill: p.panel,
        window_stroke: Stroke::new(p.stroke_width, p.stroke),
        menu_corner_radius: CornerRadius::same(8),
        panel_fill: p.panel,
        popup_shadow: Shadow { offset: [0, 6], blur: 16, spread: 0, color: Color32::from_black_alpha(shadow_alpha) },
        striped: true,
        slider_trailing_fill: true,
        ..base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_math_matches_wcag_reference_values() {
        assert!((contrast_ratio(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 1e-3);
        assert!((contrast_ratio(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-6);
        // #777777 on white is the classic 4.48:1 example.
        assert!((contrast_ratio(Color32::from_gray(0x77), Color32::WHITE) - 4.48).abs() < 0.01);
    }

    #[test]
    fn text_is_readable_in_every_theme_and_accent() {
        for kind in ThemeKind::ALL {
            for mode in crate::mode::AppMode::ALL {
                let p = Palette::new(kind, mode.default_accent());
                assert!(contrast_ratio(p.text, p.panel) >= 7.0, "{kind:?} body text");
                assert!(contrast_ratio(p.text_muted, p.panel) >= 4.5, "{kind:?} muted text");
                assert!(contrast_ratio(p.on_accent, p.accent) >= 4.5, "{kind:?} {mode:?} on-accent text");
                let min_accent = if kind == ThemeKind::HighContrast { 7.0 } else { 3.0 };
                assert!(contrast_ratio(p.accent, p.panel) >= min_accent, "{kind:?} {mode:?} accent");
            }
        }
    }

    #[test]
    fn colour_conversions_round_trip() {
        let c = Color32::from_rgba_unmultiplied(12, 200, 99, 255);
        assert_eq!(color32_from_rgba(rgba_from_srgba_bytes([12, 200, 99, 255])), c);
        for bytes in [[0, 1, 127, 128], [254, 255, 3, 0], [17, 34, 51, 68]] {
            assert_eq!(srgba_bytes(rgba_from_srgba_bytes(bytes)), bytes, "8-bit sRGB round trip");
        }
        // `#ff8800` as parsed by `ParamValue::parse_loose` is sRGB.
        let orange = simulation_core::ParamValue::parse_loose("#ff8800").as_color().unwrap();
        assert_eq!(srgba_bytes(orange), [255, 136, 0, 255]);
        assert_eq!(srgba_bytes([f32::NAN, 2.0, -1.0, 0.5]), [0, 255, 0, 128]);
        assert_eq!(hex(c), "#0cc863");
        assert_eq!(hex(Color32::from_rgba_unmultiplied(255, 255, 255, 128)), "#ffffff80");
        assert_eq!(hex(Color32::TRANSPARENT), "#00000000");
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.5), Color32::from_rgb(128, 128, 128));
        assert_eq!(color32_from_rgba([2.0, -1.0, 0.5, 1.0]), Color32::from_rgb(255, 0, 128));
    }

    /// What the screen shows for the egui colour `color` drawn over
    /// `background`: bevy_egui writes egui's premultiplied sRGB colours to
    /// an sRGB target, which blends in linear light.
    fn composite(background: Color32, color: Color32) -> Color32 {
        use egui::ecolor::{gamma_u8_from_linear_f32, linear_f32_from_gamma_u8};
        let keep = 1.0 - f32::from(color.a()) / 255.0;
        let channel =
            |c: u8, b: u8| gamma_u8_from_linear_f32(linear_f32_from_gamma_u8(c) + keep * linear_f32_from_gamma_u8(b));
        Color32::from_rgb(
            channel(color.r(), background.r()),
            channel(color.g(), background.g()),
            channel(color.b(), background.b()),
        )
    }

    /// Chroma (largest minus smallest channel) and hue in degrees.
    fn chroma_and_hue(c: Color32) -> (f32, f32) {
        let [r, g, b] = [c.r(), c.g(), c.b()].map(f32::from);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let chroma = max - min;
        let hue = if chroma == 0.0 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / chroma).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / chroma + 2.0)
        } else {
            60.0 * ((r - g) / chroma + 4.0)
        };
        (chroma, hue)
    }

    #[test]
    fn selected_widgets_show_the_accent_in_every_theme() {
        use egui::widget_style::{Classes, HasClasses, SELECTED_CLASS, WidgetState};
        let selected = Classes::default().with_class(SELECTED_CLASS);
        let ctx = egui::Context::default();
        for kind in ThemeKind::ALL {
            for mode in crate::mode::AppMode::ALL {
                let p = Palette::new(kind, mode.default_accent());
                apply_palette(&ctx, &p);
                let fill = ctx.global_style().button_style(&selected, WidgetState::Inactive).frame.fill;
                let (chroma, hue) = chroma_and_hue(composite(p.panel, fill));
                let (accent_chroma, accent_hue) = chroma_and_hue(p.accent);
                assert!(chroma >= 0.25 * accent_chroma, "{kind:?} {mode:?}: the selection shows grey");
                let gap = (hue - accent_hue).abs();
                assert!(gap.min(360.0 - gap) <= 15.0, "{kind:?} {mode:?}: the selection is not the accent's hue");
            }
        }
    }

    #[test]
    fn applying_a_palette_sets_the_theme() {
        let ctx = egui::Context::default();
        let p = Palette::new(ThemeKind::Light, Color32::from_rgb(90, 169, 255));
        apply_palette(&ctx, &p);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert_eq!(ctx.style_of(egui::Theme::Light).visuals.panel_fill, p.panel);
    }
}
