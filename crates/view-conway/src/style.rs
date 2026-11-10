//! Render styles of the world (the grid renderer's styles that make sense
//! for two-state Life), cell colour schemes, and the legend each style
//! needs. Styles never touch the simulation; the age, trail and heat styles
//! read channels that the model fills only while it tracks ages or activity.

use bevy::prelude::*;
use bevy_egui::egui::Color32;
use rendering::grid::age_ramp_position;
use rendering::palette::{Colormap, GridPalette};
use rendering::{GridAppearance, GridStyle};
use serde::{Deserialize, Serialize};

/// How live cells are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RenderStyle {
    /// Crisp squares with a subtle bevel (grid renderer "pixel art").
    #[default]
    Classic,
    /// Flat squares in one colour (grid renderer "monochrome").
    Flat,
    /// Anti-aliased discs.
    Circles,
    /// Glowing cells with a short afterglow where cells just died.
    Neon,
    /// Live cells coloured by age (generations alive).
    Age,
    /// Live cells over fading trails of recent births and deaths.
    Trails,
    /// Recent change (activity) of every cell as a heat map.
    Heat,
    /// Fraction of live cells per texel when zoomed out.
    Density,
}

/// Which model channel a style needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tracking {
    /// `track_ages` (view channel G).
    Ages,
    /// `track_activity` (view channel B).
    Activity,
}

impl Tracking {
    /// Parameter key of the model.
    pub fn param(self) -> &'static str {
        match self {
            Tracking::Ages => "track_ages",
            Tracking::Activity => "track_activity",
        }
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Tracking::Ages => "age tracking",
            Tracking::Activity => "activity tracking",
        }
    }
}

impl RenderStyle {
    /// Every style in menu order.
    pub const ALL: [RenderStyle; 8] =
        [Self::Classic, Self::Flat, Self::Circles, Self::Neon, Self::Age, Self::Trails, Self::Heat, Self::Density];

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Flat => "Flat",
            Self::Circles => "Circles",
            Self::Neon => "Neon",
            Self::Age => "Age",
            Self::Trails => "Activity trails",
            Self::Heat => "Heat",
            Self::Density => "Density",
        }
    }

    /// Tooltip.
    pub fn help(self) -> &'static str {
        match self {
            Self::Classic => "Square cells with a subtle bevel; the cell grid fades in when zoomed in.",
            Self::Flat => "Flat squares in a single colour, without shading.",
            Self::Circles => "Each live cell is a disc (one disc per texel when zoomed out).",
            Self::Neon => "Glowing cells; cells that changed recently leave a faint afterglow (activity tracking).",
            Self::Age => "Live cells coloured by how many generations they have been alive (age tracking, 1 to 255+).",
            Self::Trails => {
                "Live cells over trails of recent births and deaths: 255 when a cell changed this tick, 16 less per \
                 tick (activity tracking)."
            }
            Self::Heat => "Every cell coloured by its activity: bright where it changed this tick (activity tracking).",
            Self::Density => "Fraction of live cells in every texel; most useful zoomed out on large patterns.",
        }
    }

    /// Grid renderer style.
    pub fn grid_style(self) -> GridStyle {
        match self {
            Self::Classic => GridStyle::PixelArt,
            Self::Flat => GridStyle::Monochrome,
            Self::Circles => GridStyle::Circles,
            Self::Neon => GridStyle::Neon,
            Self::Age => GridStyle::Age,
            Self::Trails => GridStyle::History,
            Self::Heat => GridStyle::Heatmap,
            Self::Density => GridStyle::Density,
        }
    }

    /// Model channel the style reads.
    pub fn tracking(self) -> Option<Tracking> {
        match self {
            Self::Age => Some(Tracking::Ages),
            Self::Trails | Self::Heat | Self::Neon => Some(Tracking::Activity),
            _ => None,
        }
    }

    /// Ramp colormap of the scalar styles.
    pub fn ramp(self) -> Colormap {
        match self {
            Self::Age => Colormap::Viridis,
            Self::Trails => Colormap::Inferno,
            Self::Heat => Colormap::Inferno,
            // Plasma starts at a saturated blue, so sparse texels stay visible
            // on the dark background.
            Self::Density => Colormap::Plasma,
            _ => Colormap::Viridis,
        }
    }

    /// Next (or previous) style.
    pub fn cycle(self, forward: bool) -> Self {
        let n = Self::ALL.len();
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[if forward { (i + 1) % n } else { (i + n - 1) % n }]
    }
}

/// Cell colour scheme of the two-state styles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellColors {
    /// The mode's blue on near-black.
    #[default]
    Lab,
    /// Warm white on near-black (the renderer's Life palette).
    Ivory,
    /// Phosphor green on black.
    Phosphor,
    /// Amber on black.
    Amber,
    /// Dark ink on light paper.
    Paper,
}

impl CellColors {
    /// Every scheme.
    pub const ALL: [CellColors; 5] = [Self::Lab, Self::Ivory, Self::Phosphor, Self::Amber, Self::Paper];

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Lab => "Lab blue",
            Self::Ivory => "Ivory",
            Self::Phosphor => "Phosphor",
            Self::Amber => "Amber",
            Self::Paper => "Paper",
        }
    }

    /// `(dead, alive)` sRGB colours.
    pub fn colors(self) -> ([u8; 3], [u8; 3]) {
        match self {
            Self::Lab => ([12, 16, 23], [90, 169, 255]),
            Self::Ivory => ([14, 16, 22], [236, 232, 214]),
            Self::Phosphor => ([5, 11, 7], [102, 255, 140]),
            Self::Amber => ([15, 10, 4], [255, 182, 66]),
            Self::Paper => ([243, 241, 235], [33, 40, 54]),
        }
    }

    /// Whether the background is light.
    pub fn light(self) -> bool {
        self == Self::Paper
    }

    /// Opacity (0–255) of the selection's translucent fill. Sprites blend in
    /// linear light, where a few percent already tint a dark background
    /// clearly; a light background needs more.
    pub fn selection_alpha(self) -> u8 {
        if self.light() { 20 } else { 7 }
    }

    /// Colour of selections and tool previews: distinct from both the dead
    /// and the live colour of the scheme.
    pub fn highlight(self) -> [u8; 3] {
        match self {
            Self::Lab | Self::Ivory | Self::Phosphor => [255, 196, 64],
            Self::Amber => [96, 210, 255],
            Self::Paper => [214, 72, 32],
        }
    }
}

/// How the world is displayed (kept across visits to the mode).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ViewSettings {
    /// Render style.
    pub style: RenderStyle,
    /// Cell colours.
    pub colors: CellColors,
    /// Cell grid lines (fade in when cells are large enough on screen).
    pub grid: bool,
    /// Stronger line every 10 cells, as in Golly.
    pub major_grid: bool,
    /// Outline of finite worlds.
    pub border: bool,
    /// Readout of the cell under the pointer.
    pub hover_readout: bool,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            style: RenderStyle::Classic,
            colors: CellColors::Lab,
            grid: true,
            major_grid: true,
            border: true,
            hover_readout: true,
        }
    }
}

fn srgb(c: [u8; 3]) -> Color {
    Color::srgb_u8(c[0], c[1], c[2])
}

/// Egui colour of an sRGB triple.
pub fn color32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The grid appearance for the settings.
pub fn appearance(settings: &ViewSettings) -> GridAppearance {
    let style = settings.style;
    let (dead, alive) = settings.colors.colors();
    let light = settings.colors.light();
    let grid_color = if light { Color::srgba(0.0, 0.0, 0.0, 0.16) } else { Color::srgba(1.0, 1.0, 1.0, 0.13) };
    let base = GridAppearance::styled(style.grid_style());
    GridAppearance {
        style: style.grid_style(),
        palette: GridPalette::two_state(srgb(dead), srgb(alive)),
        ramp: style.ramp(),
        background: Some(srgb(dead)),
        foreground: srgb(alive),
        show_grid: settings.grid && style != RenderStyle::Density,
        grid_color,
        grid_min_pixels: 10.0,
        major_grid_every: if settings.major_grid { 10 } else { 0 },
        glow: if style == RenderStyle::Neon { 1.6 } else { base.glow },
        smooth_fields: false,
    }
}

/// Egui colour of a Bevy colour (sRGB, opaque).
fn egui_color(c: Color) -> Color32 {
    let s = c.to_srgba();
    Color32::from_rgb((s.red * 255.0).round() as u8, (s.green * 255.0).round() as u8, (s.blue * 255.0).round() as u8)
}

/// `n` colours of a colormap (for legends).
pub fn ramp_stops(map: Colormap, n: usize) -> Vec<Color32> {
    map.stops(n.max(2)).into_iter().map(egui_color).collect()
}

/// Ages (generations) of the Age legend's stops. Powers of two are evenly
/// spaced on the logarithmic scale of [`age_ramp_position`] (255 is almost
/// 2⁸), so the evenly spaced stops follow the shader and 16 sits in the
/// middle.
const AGE_LEGEND_AGES: [u8; 9] = [1, 2, 4, 8, 16, 32, 64, 128, 255];

/// Legend colours of the Age style: the colours the shader gives cells of
/// the ages in [`AGE_LEGEND_AGES`].
fn age_stops(map: Colormap) -> Vec<Color32> {
    AGE_LEGEND_AGES.iter().map(|&age| egui_color(map.sample(age_ramp_position(age)))).collect()
}

/// What the legend of a style shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Legend {
    /// Two chips: alive and dead.
    States {
        /// Live colour.
        alive: Color32,
        /// Dead colour.
        dead: Color32,
    },
    /// A colour ramp with end labels.
    Ramp {
        /// Ramp colours.
        stops: Vec<Color32>,
        /// Label of the low end.
        low: &'static str,
        /// Label of the high end.
        high: &'static str,
        /// Optional live-cell chip drawn over the ramp (trails).
        alive: Option<Color32>,
    },
}

/// The legend of the current settings.
pub fn legend(settings: &ViewSettings) -> Legend {
    let (dead, alive) = settings.colors.colors();
    let map = settings.style.ramp();
    let stops = if settings.style == RenderStyle::Age { age_stops(map) } else { ramp_stops(map, 12) };
    match settings.style {
        RenderStyle::Age => Legend::Ramp { stops, low: "1 generation", high: "255+ (log scale)", alive: None },
        RenderStyle::Trails => {
            Legend::Ramp { stops, low: "16 ticks ago", high: "changed this tick", alive: Some(color32(alive)) }
        }
        RenderStyle::Heat => Legend::Ramp { stops, low: "no change", high: "changed this tick", alive: None },
        RenderStyle::Density => Legend::Ramp { stops, low: "0 % alive", high: "100 % alive", alive: None },
        _ => Legend::States { alive: color32(alive), dead: color32(dead) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_map_to_renderer_styles_and_channels() {
        assert_eq!(RenderStyle::Classic.grid_style(), GridStyle::PixelArt);
        assert_eq!(RenderStyle::Trails.grid_style(), GridStyle::History);
        assert_eq!(RenderStyle::Age.tracking(), Some(Tracking::Ages));
        assert_eq!(RenderStyle::Heat.tracking(), Some(Tracking::Activity));
        assert_eq!(RenderStyle::Flat.tracking(), None);
        assert_eq!(Tracking::Ages.param(), "track_ages");
        assert_eq!(RenderStyle::Density.cycle(true), RenderStyle::Classic);
        assert_eq!(RenderStyle::Classic.cycle(false), RenderStyle::Density);
        let mut seen: Vec<GridStyle> = RenderStyle::ALL.iter().map(|s| s.grid_style()).collect();
        seen.dedup();
        assert_eq!(seen.len(), RenderStyle::ALL.len(), "every style is distinct");
    }

    #[test]
    fn appearance_follows_the_settings() {
        let s = ViewSettings { colors: CellColors::Amber, grid: true, major_grid: false, ..default() };
        let a = appearance(&s);
        assert_eq!(a.palette, GridPalette::two_state(srgb([15, 10, 4]), srgb([255, 182, 66])));
        assert_eq!(a.resolved_background(), srgb([15, 10, 4]));
        assert!(a.show_grid);
        assert_eq!(a.major_grid_every, 0);
        let density = appearance(&ViewSettings { style: RenderStyle::Density, ..default() });
        assert!(!density.show_grid, "no grid over density texels");
        assert_eq!(density.ramp, Colormap::Plasma);
        let paper = appearance(&ViewSettings { colors: CellColors::Paper, ..default() });
        assert!(paper.grid_color.to_srgba().red < 0.5, "dark grid lines on paper");
    }

    #[test]
    fn highlights_contrast_with_both_cell_colours() {
        let distance =
            |a: [u8; 3], b: [u8; 3]| -> i32 { (0..3).map(|i| (i32::from(a[i]) - i32::from(b[i])).abs()).sum() };
        for scheme in CellColors::ALL {
            let (dead, alive) = scheme.colors();
            let h = scheme.highlight();
            assert!(distance(h, dead) > 200 && distance(h, alive) > 150, "{scheme:?}");
        }
    }

    #[test]
    fn legends() {
        let s = ViewSettings::default();
        assert_eq!(legend(&s), Legend::States { alive: color32([90, 169, 255]), dead: color32([12, 16, 23]) });
        match legend(&ViewSettings { style: RenderStyle::Trails, ..default() }) {
            Legend::Ramp { stops, alive, .. } => {
                assert_eq!(stops.len(), 12);
                assert!(alive.is_some());
            }
            other => panic!("unexpected legend {other:?}"),
        }
    }

    /// The Age legend is drawn on the shader's logarithmic scale: where the
    /// bar places an age, it shows the colour the shader gives a live cell
    /// of that age (not the ramp from 0, whose dark start no cell has).
    #[test]
    fn age_legend_matches_the_shader_scale() {
        let settings = ViewSettings { style: RenderStyle::Age, ..default() };
        let Legend::Ramp { stops, .. } = legend(&settings) else { panic!("the Age style has a ramp legend") };
        let shader = |age: u8| egui_color(appearance(&settings).ramp.sample(age_ramp_position(age)));
        let ends = [stops[0], stops[stops.len() / 2], stops[stops.len() - 1]];
        assert_eq!(ends, [shader(1), shader(16), shader(255)], "1, 16 and 255 generations: start, middle, end");
        // Between stops the bar interpolates linearly, which bends a little
        // less than the colormap.
        for age in [2, 3, 7, 40, 100, 200] {
            let bar = interface::widgets::color_ramp(f32::from(age).ln() / 255f32.ln(), &stops);
            let close = bar.to_array().iter().zip(shader(age).to_array()).all(|(a, b)| a.abs_diff(b) <= 8);
            assert!(close, "age {age}: legend {bar:?}, shader {:?}", shader(age));
        }
    }
}
