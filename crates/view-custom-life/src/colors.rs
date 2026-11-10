//! Colour rules of the mode: entity shading by type, energy or state,
//! relationship-kind colours and display ranges of field overlays.

use bevy::prelude::Color;
use bevy_egui::egui::Color32;

/// What the colour of an entity disc encodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorBy {
    /// The type's colour.
    #[default]
    Type,
    /// The type's colour, darker at low energy (5 levels).
    Energy,
    /// The type's colour, shaded per state.
    State,
}

impl ColorBy {
    /// Every option, in menu order.
    pub const ALL: [ColorBy; 3] = [ColorBy::Type, ColorBy::Energy, ColorBy::State];

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            ColorBy::Type => "Type",
            ColorBy::Energy => "Energy",
            ColorBy::State => "State",
        }
    }
}

/// Number of energy shades.
pub const ENERGY_LEVELS: u8 = 5;

/// Energy shade (0 = starving … 4 = full) of a normalised energy.
pub fn energy_level(energy: f32) -> u8 {
    if !energy.is_finite() {
        return 0;
    }
    let e = energy.clamp(0.0, 1.0);
    ((e * f32::from(ENERGY_LEVELS)).floor() as u8).min(ENERGY_LEVELS - 1)
}

fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

const DARK: [f32; 4] = [0.08, 0.09, 0.11, 1.0];
const LIGHT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Colour of an energy level: the type colour mixed towards dark grey as
/// energy falls (level 4 is the type colour itself).
pub fn energy_shade(base: [f32; 4], level: u8) -> [f32; 4] {
    let t = f32::from(level.min(ENERGY_LEVELS - 1)) / f32::from(ENERGY_LEVELS - 1);
    let mut c = mix(DARK, base, 0.25 + 0.75 * t);
    c[3] = base[3];
    c
}

/// Colour of state `state` of a type with `states` states: the first state
/// is the type colour; later states alternate lighter and darker shades.
pub fn state_shade(base: [f32; 4], state: u16, states: usize) -> [f32; 4] {
    if state == 0 || states <= 1 {
        return base;
    }
    let step = 0.55 * f32::from(state.div_ceil(2)) / (states as f32 / 2.0).ceil().max(1.0);
    let mut c = if state % 2 == 1 { mix(base, LIGHT, step) } else { mix(base, DARK, step) };
    c[3] = base[3];
    c
}

/// Colour of an entity and the material variant key that identifies it
/// (entities with the same type and key share one material).
pub fn entity_rgba(base: [f32; 4], by: ColorBy, energy: f32, state: u16, states: usize) -> ([f32; 4], u8) {
    match by {
        ColorBy::Type => (base, 0),
        ColorBy::Energy => {
            let level = energy_level(energy);
            (energy_shade(base, level), level)
        }
        ColorBy::State => {
            let s = state.min(250);
            (state_shade(base, s, states), s as u8)
        }
    }
}

/// Bevy colour of a definition colour (unmultiplied sRGB in `[0, 1]`).
pub fn bevy_color(rgba: [f32; 4]) -> Color {
    let c = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    Color::srgba(c(rgba[0]), c(rgba[1]), c(rgba[2]), c(rgba[3]))
}

/// Colour of a relationship kind (by its label), shared by the graph, its
/// legend and the relationship list.
pub fn relation_color(label: &str) -> Color32 {
    match label {
        "consumes" => Color32::from_rgb(240, 120, 90),
        "protects" => Color32::from_rgb(90, 200, 140),
        "chases" => Color32::from_rgb(245, 170, 60),
        "avoids" => Color32::from_rgb(110, 170, 250),
        "transforms" => Color32::from_rgb(200, 140, 240),
        "produces" => Color32::from_rgb(230, 210, 90),
        "reproduces_with" => Color32::from_rgb(250, 130, 190),
        "competes_with" => Color32::from_rgb(235, 85, 85),
        "trades_with" => Color32::from_rgb(80, 210, 210),
        _ => Color32::from_rgb(160, 168, 182),
    }
}

/// Every relationship label in legend order.
pub const RELATION_LABELS: [&str; 9] = [
    "consumes",
    "avoids",
    "chases",
    "protects",
    "competes_with",
    "transforms",
    "produces",
    "reproduces_with",
    "trades_with",
];

/// Display range of a field overlay. Resources use `[0, max(def_max,
/// observed max)]` so colours stay comparable over time (a regrowth
/// ceiling of 0, as for pure emitted fields, falls back to the observed
/// maximum); environment fields use their observed `[min, max]`.
pub fn field_range(observed_min: f32, observed_max: f32, resource_max: Option<f32>) -> (f32, f32) {
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let (lo, hi) = match resource_max {
        Some(def_max) => (0.0, finite(def_max).max(finite(observed_max))),
        None => (finite(observed_min), finite(observed_max)),
    };
    if hi - lo > 1e-9 { (lo, hi) } else { (lo, lo + 1.0) }
}

/// Glyphs offered as quick picks for entity types (emoji and symbols that
/// egui's built-in fonts draw; checked by `glyphs::tests`).
pub const GLYPHS: [&str; 24] = [
    "🌿", "🌱", "🍄", "🌸", "🐑", "🐺", "🐇", "🐟", "🐝", "★", "■", "♦", "☀", "❄", "⚡", "☘", "✿", "⬟", "⬣", "✖", "✚",
    "⚙", "🔵", "🔴",
];

/// Colours given to new entity types and resources in turn.
pub const NEW_COLORS: [[f32; 4]; 8] = [
    [0.36, 0.78, 0.42, 1.0],
    [0.92, 0.72, 0.30, 1.0],
    [0.86, 0.34, 0.30, 1.0],
    [0.42, 0.62, 0.95, 1.0],
    [0.78, 0.52, 0.92, 1.0],
    [0.32, 0.82, 0.82, 1.0],
    [0.95, 0.55, 0.72, 1.0],
    [0.72, 0.74, 0.78, 1.0],
];

/// The `n`-th colour of [`NEW_COLORS`].
pub fn new_color(n: usize) -> [f32; 4] {
    NEW_COLORS[n % NEW_COLORS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: [f32; 4] = [0.8, 0.4, 0.2, 1.0];

    #[test]
    fn energy_levels_cover_the_unit_interval() {
        assert_eq!(energy_level(0.0), 0);
        assert_eq!(energy_level(0.39), 1);
        assert_eq!(energy_level(1.0), 4);
        assert_eq!(energy_level(7.0), 4);
        assert_eq!(energy_level(f32::NAN), 0);
        assert_eq!(energy_shade(BASE, 4), BASE);
        let dim = energy_shade(BASE, 0);
        assert!(dim[0] < BASE[0] && dim[3] == 1.0);
    }

    #[test]
    fn states_get_distinct_shades_and_state_zero_keeps_the_type_colour() {
        assert_eq!(state_shade(BASE, 0, 3), BASE);
        let a = state_shade(BASE, 1, 3);
        let b = state_shade(BASE, 2, 3);
        assert_ne!(a, BASE);
        assert_ne!(b, BASE);
        assert_ne!(a, b);
        assert!(a[1] > BASE[1], "odd states are lighter");
        assert!(b[1] < BASE[1], "even states are darker");
        assert_eq!(state_shade(BASE, 3, 1), BASE, "single-state types never shade");
    }

    #[test]
    fn material_keys_separate_variants() {
        assert_eq!(entity_rgba(BASE, ColorBy::Type, 0.1, 2, 3).1, 0);
        assert_eq!(entity_rgba(BASE, ColorBy::Energy, 0.95, 0, 1).1, 4);
        assert_eq!(entity_rgba(BASE, ColorBy::State, 0.5, 2, 3).1, 2);
    }

    #[test]
    fn field_ranges_are_stable_and_never_empty() {
        assert_eq!(field_range(0.1, 0.4, Some(1.0)), (0.0, 1.0));
        assert_eq!(field_range(0.0, 3.0, Some(1.0)), (0.0, 3.0));
        assert_eq!(field_range(0.0, 0.0, Some(0.0)), (0.0, 1.0));
        assert_eq!(field_range(0.3, 0.9, None), (0.3, 0.9));
        assert_eq!(field_range(f32::NAN, f32::INFINITY, None), (0.0, 1.0));
    }

    #[test]
    fn quick_picks_are_short_and_unique() {
        let mut glyphs: Vec<&str> = GLYPHS.to_vec();
        assert!(glyphs.iter().all(|g| !g.is_empty() && g.chars().count() <= 4), "icons are limited to 4 characters");
        glyphs.sort_unstable();
        glyphs.dedup();
        assert_eq!(glyphs.len(), GLYPHS.len());
        assert_ne!(new_color(0), new_color(1));
        assert_eq!(new_color(8), new_color(0));
    }

    #[test]
    fn every_relation_has_its_own_colour() {
        let mut colours: Vec<Color32> = RELATION_LABELS.iter().map(|l| relation_color(l)).collect();
        colours.sort_by_key(|c| c.to_array());
        colours.dedup();
        assert_eq!(colours.len(), RELATION_LABELS.len());
    }
}
