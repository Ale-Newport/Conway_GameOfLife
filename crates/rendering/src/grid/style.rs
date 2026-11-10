//! Visual styles for 2D grids and the GPU uniforms derived from them.

use bevy::math::I64Vec2;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use serde::{Deserialize, Serialize};

use crate::palette::{Colormap, GridPalette, MAX_PALETTE_COLORS, gpu_stops};

/// How cells are drawn. Styles only change the shader; they never touch the
/// simulation data. For level-of-detail frames (`cells_per_texel > 1`) the
/// shapes of the Circles, Minimal, Neon, Gradient and PixelArt styles are
/// drawn once per texel, because a texel aggregates cells whose individual
/// states are unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridStyle {
    /// Live cells in one foreground colour on a flat background.
    Monochrome,
    /// Palette colours, crisp square cells with a subtle bevel and grid.
    #[default]
    PixelArt,
    /// Emissive palette colours (values above 1.0 bloom on an HDR camera).
    Neon,
    /// Inset rounded squares on a quiet background, no bevel.
    Minimal,
    /// Cells drawn as anti-aliased circles.
    Circles,
    /// Palette colours with radial shading inside each cell.
    Gradient,
    /// Activity channel (B) through the ramp colormap.
    Heatmap,
    /// Age channel (G) of live cells through the ramp colormap, on the
    /// logarithmic scale of [`age_ramp_position`].
    Age,
    /// Density channel (A, fraction of live cells per texel) through the ramp
    /// colormap; intended for zoomed-out level-of-detail frames.
    Density,
    /// Live cells in palette colours over fading trails from the B channel.
    History,
}

impl GridStyle {
    /// Every style, in display order.
    pub const ALL: [GridStyle; 10] = [
        Self::Monochrome,
        Self::PixelArt,
        Self::Neon,
        Self::Minimal,
        Self::Circles,
        Self::Gradient,
        Self::Heatmap,
        Self::Age,
        Self::Density,
        Self::History,
    ];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Monochrome => "Monochrome",
            Self::PixelArt => "Pixel art",
            Self::Neon => "Neon",
            Self::Minimal => "Minimal",
            Self::Circles => "Circles",
            Self::Gradient => "Gradient",
            Self::Heatmap => "Heatmap",
            Self::Age => "Age",
            Self::Density => "Density",
            Self::History => "History",
        }
    }

    /// Which frame channel the style reads besides the state.
    pub fn channel_hint(self) -> &'static str {
        match self {
            Self::Heatmap => "activity (B)",
            Self::Age => "age (G)",
            Self::Density => "density (A)",
            Self::History => "trail (B)",
            _ => "state (R)",
        }
    }

    /// Identifier used by `grid.wesl` (must match the shader constants).
    pub(crate) fn shader_id(self) -> u32 {
        match self {
            Self::Monochrome => 0,
            Self::PixelArt => 1,
            Self::Neon => 2,
            Self::Minimal => 3,
            Self::Circles => 4,
            Self::Gradient => 5,
            Self::Heatmap => 6,
            Self::Age => 7,
            Self::Density => 8,
            Self::History => 9,
        }
    }
}

/// Lowest ramp position of the [`GridStyle::Age`] style: newborn cells start
/// here so they stay visible on dark backgrounds (mirrored in `grid.wesl`).
pub const AGE_RAMP_MIN: f32 = 0.25;

/// Ramp position at which [`GridStyle::Age`] draws a live cell that is `age`
/// generations old (G channel; 0 = not tracked, drawn as newborn). The scale
/// is logarithmic from [`AGE_RAMP_MIN`] at 1 generation to 1.0 at 255, so the
/// young cells that dominate active regions spread over the ramp (16
/// generations sit halfway) instead of sharing its darkest end. The shader
/// (`age_ramp_position` in `grid.wesl`) uses the same mapping; Age legends
/// should sample the ramp through it.
pub fn age_ramp_position(age: u8) -> f32 {
    let generations = f32::from(age.max(1));
    AGE_RAMP_MIN + (1.0 - AGE_RAMP_MIN) * generations.log2() / 255f32.log2()
}

/// Ages (generations) of the evenly spaced stops of an Age legend, from the
/// newborn to the oldest tracked age. Powers of two are evenly spaced on the
/// logarithmic scale of [`age_ramp_position`] (255 is almost 2⁸), so a bar
/// with the ramp colours of these ages follows the shader, and 16, the
/// middle entry, sits in its middle.
pub const AGE_LEGEND_AGES: [u8; 9] = [1, 2, 4, 8, 16, 32, 64, 128, 255];

/// Appearance of one grid surface. Changing it updates the material uniforms
/// only; the uploaded frame stays untouched.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct GridAppearance {
    /// Drawing style.
    pub style: GridStyle,
    /// State colours or field colormap (R channel).
    pub palette: GridPalette,
    /// Colormap for the Heatmap, Age, Density and History channels.
    pub ramp: Colormap,
    /// Background for dead cells; `None` uses the palette's state-0 colour.
    pub background: Option<Color>,
    /// Live-cell colour for the Monochrome style.
    pub foreground: Color,
    /// Whether grid lines are drawn (only above [`Self::grid_min_pixels`]).
    pub show_grid: bool,
    /// Grid line colour; alpha is the line strength.
    pub grid_color: Color,
    /// Minimum on-screen cell size, in framebuffer (physical) pixels, before
    /// grid lines fade in. On a 2× (Retina) display the default of 6 is 3
    /// logical pixels; 0 draws lines at every zoom level. Lines are about one
    /// physical pixel wide.
    pub grid_min_pixels: f32,
    /// Every n-th grid line is drawn stronger (Golly-style); 0 disables.
    pub major_grid_every: u32,
    /// Emissive multiplier for the Neon style (> 1 produces HDR values).
    pub glow: f32,
    /// Bilinear interpolation of continuous fields (continuous palettes only).
    pub smooth_fields: bool,
}

impl Default for GridAppearance {
    fn default() -> Self {
        Self::styled(GridStyle::PixelArt)
    }
}

impl GridAppearance {
    /// Recommended settings for a style (palette, ramp, grid).
    pub fn styled(style: GridStyle) -> Self {
        let base = Self {
            style,
            palette: GridPalette::life(),
            ramp: Colormap::Inferno,
            background: None,
            foreground: Color::srgb_u8(236, 232, 214),
            show_grid: true,
            grid_color: Color::srgba(1.0, 1.0, 1.0, 0.14),
            grid_min_pixels: 6.0,
            major_grid_every: 10,
            glow: 1.0,
            smooth_fields: false,
        };
        match style {
            GridStyle::Monochrome => Self { foreground: Color::WHITE, background: Some(Color::BLACK), ..base },
            GridStyle::PixelArt => base,
            GridStyle::Neon => {
                Self { palette: GridPalette::neon(), glow: 4.0, grid_color: Color::srgba(0.3, 0.6, 1.0, 0.10), ..base }
            }
            GridStyle::Minimal => Self {
                palette: GridPalette::minimal(),
                show_grid: false,
                grid_color: Color::srgba(0.0, 0.0, 0.0, 0.08),
                ..base
            },
            GridStyle::Circles => Self { show_grid: false, ..base },
            GridStyle::Gradient => Self { palette: GridPalette::brians_brain(), ..base },
            GridStyle::Heatmap => Self { ramp: Colormap::Inferno, ..base },
            GridStyle::Age => Self { ramp: Colormap::Viridis, ..base },
            GridStyle::Density => Self { ramp: Colormap::Magma, show_grid: false, ..base },
            GridStyle::History => Self { ramp: Colormap::Plasma, ..base },
        }
    }

    /// The resolved background colour.
    pub fn resolved_background(&self) -> Color {
        self.background.unwrap_or_else(|| self.palette.background())
    }
}

/// Scalar uniforms of the grid shader (layout mirrors `GridParams` in `grid.wesl`).
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct GridParams {
    /// Dead-cell colour (linear RGBA).
    pub background: Vec4,
    /// Monochrome live colour (linear RGBA).
    pub foreground: Vec4,
    /// Grid line colour (linear RGB, alpha = strength).
    pub grid_color: Vec4,
    /// Style identifier.
    pub style: u32,
    /// 0 = discrete state colours, 1 = continuous colormap.
    pub palette_mode: u32,
    /// Number of used palette entries.
    pub palette_len: u32,
    /// Number of used ramp entries.
    pub ramp_len: u32,
    /// Cells per texel of the current frame.
    pub cells_per_texel: u32,
    /// Bit 0: grid lines; bit 1: smooth fields.
    pub flags: u32,
    /// Major grid spacing in cells (0 = none).
    pub major_every: u32,
    /// Frame origin x modulo `major_every`.
    pub origin_mod_x: u32,
    /// Frame origin y modulo `major_every`.
    pub origin_mod_y: u32,
    /// Minimum cell size in pixels for grid lines.
    pub grid_min_px: f32,
    /// Neon emissive multiplier.
    pub glow: f32,
}

/// A table of up to 32 linear RGBA colours.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct ColorTable {
    /// Colours (unused entries are zero).
    pub colors: [Vec4; MAX_PALETTE_COLORS],
}

pub(crate) const FLAG_GRID: u32 = 1;
pub(crate) const FLAG_SMOOTH: u32 = 2;

/// All uniforms of a grid material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GridUniforms {
    pub params: GridParams,
    pub palette: ColorTable,
    pub ramp: ColorTable,
}

impl GridUniforms {
    /// Packs an appearance plus frame geometry into GPU uniforms.
    pub fn new(appearance: &GridAppearance, cells_per_texel: u32, origin: I64Vec2) -> Self {
        let (palette_mode, palette_colors) = match &appearance.palette {
            GridPalette::States(colors) => (0, colors.clone()),
            GridPalette::Continuous(map) => (1, map.stops(MAX_PALETTE_COLORS)),
        };
        let palette_len = palette_colors.len().clamp(1, MAX_PALETTE_COLORS) as u32;
        let ramp_colors = appearance.ramp.stops(MAX_PALETTE_COLORS);
        let mut flags = 0;
        if appearance.show_grid {
            flags |= FLAG_GRID;
        }
        if appearance.smooth_fields {
            flags |= FLAG_SMOOTH;
        }
        let major = appearance.major_grid_every;
        let (origin_mod_x, origin_mod_y) = if major > 0 {
            let m = i64::from(major);
            (origin.x.rem_euclid(m) as u32, origin.y.rem_euclid(m) as u32)
        } else {
            (0, 0)
        };
        let finite_or = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        Self {
            params: GridParams {
                background: appearance.resolved_background().to_linear().to_vec4(),
                foreground: appearance.foreground.to_linear().to_vec4(),
                grid_color: appearance.grid_color.to_linear().to_vec4(),
                style: appearance.style.shader_id(),
                palette_mode,
                palette_len,
                ramp_len: ramp_colors.len() as u32,
                cells_per_texel: cells_per_texel.max(1),
                flags,
                major_every: major,
                origin_mod_x,
                origin_mod_y,
                grid_min_px: finite_or(appearance.grid_min_pixels, 6.0).max(0.0),
                glow: finite_or(appearance.glow, 1.0).max(0.0),
            },
            palette: ColorTable { colors: gpu_stops(&palette_colors) },
            ramp: ColorTable { colors: gpu_stops(&ramp_colors) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_ids_are_unique_and_dense() {
        let mut ids: Vec<u32> = GridStyle::ALL.iter().map(|s| s.shader_id()).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn uniforms_pack_discrete_palette() {
        let appearance = GridAppearance {
            palette: GridPalette::wireworld(),
            major_grid_every: 10,
            ..GridAppearance::styled(GridStyle::PixelArt)
        };
        let u = GridUniforms::new(&appearance, 1, I64Vec2::new(-3, 27));
        assert_eq!(u.params.palette_mode, 0);
        assert_eq!(u.params.palette_len, 4);
        assert_eq!(u.params.style, 1);
        assert_eq!((u.params.origin_mod_x, u.params.origin_mod_y), (7, 7));
        assert_eq!(u.params.flags & FLAG_GRID, FLAG_GRID);
        assert_eq!(u.palette.colors[1], Color::srgb_u8(0, 128, 255).to_linear().to_vec4());
        assert_eq!(u.palette.colors[4], Vec4::ZERO);
        // Background defaults to the palette's state 0.
        assert_eq!(u.params.background, Color::srgb_u8(48, 48, 48).to_linear().to_vec4());
    }

    #[test]
    fn uniforms_pack_continuous_palette_and_flags() {
        let appearance = GridAppearance {
            palette: GridPalette::Continuous(Colormap::Grayscale),
            smooth_fields: true,
            show_grid: false,
            major_grid_every: 0,
            glow: f32::NAN,
            ..GridAppearance::default()
        };
        let u = GridUniforms::new(&appearance, 0, I64Vec2::new(5, 5));
        assert_eq!(u.params.palette_mode, 1);
        assert_eq!(u.params.palette_len, MAX_PALETTE_COLORS as u32);
        assert_eq!(u.params.flags, FLAG_SMOOTH);
        assert_eq!(u.params.cells_per_texel, 1, "zero is clamped to one");
        assert_eq!((u.params.origin_mod_x, u.params.origin_mod_y), (0, 0));
        assert_eq!(u.params.glow, 1.0, "non-finite glow falls back");
        assert_eq!(u.palette.colors[0], Vec4::new(0.0, 0.0, 0.0, 1.0));
        assert_eq!(u.palette.colors[31], Vec4::ONE);
    }

    #[test]
    fn age_scale_spreads_young_cells() {
        assert_eq!(age_ramp_position(0), AGE_RAMP_MIN, "untracked ages are drawn as newborn");
        assert_eq!(age_ramp_position(1), AGE_RAMP_MIN);
        assert!((age_ramp_position(16) - (1.0 + AGE_RAMP_MIN) / 2.0).abs() < 1e-3, "16 generations sit halfway");
        assert!((age_ramp_position(255) - 1.0).abs() < 1e-6);
        // Ages 1 to 20 cover more than half of the ramp from its minimum.
        assert!(age_ramp_position(20) - AGE_RAMP_MIN > 0.5 * (1.0 - AGE_RAMP_MIN));
        assert!((1..=255).map(age_ramp_position).collect::<Vec<_>>().windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn age_legend_stops_are_evenly_spaced_on_the_age_scale() {
        let positions = AGE_LEGEND_AGES.map(age_ramp_position);
        let n = positions.len() - 1;
        for (i, position) in positions.into_iter().enumerate() {
            let even = AGE_RAMP_MIN + (1.0 - AGE_RAMP_MIN) * i as f32 / n as f32;
            assert!((position - even).abs() < 1e-3, "age {}: {position} instead of {even}", AGE_LEGEND_AGES[i]);
        }
        assert_eq!((AGE_LEGEND_AGES[0], AGE_LEGEND_AGES[n / 2], AGE_LEGEND_AGES[n]), (1, 16, 255));
    }

    #[test]
    fn shader_uses_the_age_scale() {
        let shader = include_str!("../../../../assets/shaders/grid.wesl");
        let min = shader.lines().find_map(|l| l.strip_prefix("const AGE_RAMP_MIN: f32 = "));
        assert_eq!(min.and_then(|v| v.trim_end_matches(';').parse().ok()), Some(AGE_RAMP_MIN));
        assert!(shader.contains("ramp_lerp(age_ramp_position(raw.g))"), "the Age style reads the scale");
    }

    #[test]
    fn styled_presets_are_distinct() {
        for (i, a) in GridStyle::ALL.iter().enumerate() {
            assert_eq!(GridAppearance::styled(*a).style, *a);
            for b in &GridStyle::ALL[i + 1..] {
                assert_ne!(GridAppearance::styled(*a), GridAppearance::styled(*b));
            }
        }
    }
}
