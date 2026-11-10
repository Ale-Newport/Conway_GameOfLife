//! Colours of the mode: terrain, goods, activities, groups, roads.
//!
//! World colours are sRGB triples chosen for a calm, slightly desaturated
//! "map" look under the scene lighting; UI colours (legends, inspector,
//! charts) use the same values so the legend matches the world.

use bevy::color::{Color, ColorToPacked, LinearRgba};
use bevy_egui::egui::Color32;
use civilization::{Activity, Good, Terrain};

/// sRGB bytes of a terrain class.
pub fn terrain_srgb(terrain: Terrain) -> [u8; 3] {
    match terrain {
        Terrain::Grassland => [118, 150, 74],
        Terrain::Forest => [66, 96, 54],
        Terrain::Hills => [136, 122, 100],
        Terrain::Water => [58, 112, 138],
        Terrain::Riverbank => [92, 146, 70],
        Terrain::Berries => [104, 138, 66],
        Terrain::Mountain => [112, 108, 106],
    }
}

/// Snow on high mountain centres.
pub const SNOW: [u8; 3] = [236, 239, 243];
/// Sand on shallow water beds.
pub const SAND: [u8; 3] = [196, 180, 132];

/// egui colour of a terrain class (legend).
pub fn terrain_color32(terrain: Terrain) -> Color32 {
    let [r, g, b] = terrain_srgb(terrain);
    Color32::from_rgb(r, g, b)
}

/// Linear RGBA of sRGB bytes scaled in brightness by `shade`.
pub fn linear(srgb: [u8; 3], shade: f32) -> [f32; 4] {
    let c = Color::srgb_u8(srgb[0], srgb[1], srgb[2]).to_linear();
    [c.red * shade, c.green * shade, c.blue * shade, 1.0]
}

/// Linear interpolation of two sRGB byte colours.
pub fn mix_srgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    [l(a[0], b[0]), l(a[1], b[1]), l(a[2], b[2])]
}

/// Colour of a good (inventory bars, carried bundles, legend).
pub fn good_srgb(good: Good) -> [u8; 3] {
    match good {
        Good::Food => [214, 92, 74],
        Good::Wood => [160, 110, 62],
        Good::Stone => [150, 156, 166],
        Good::Tools => [96, 150, 214],
    }
}

/// egui colour of a good.
pub fn good_color32(good: Good) -> Color32 {
    let [r, g, b] = good_srgb(good);
    Color32::from_rgb(r, g, b)
}

/// Pictogram drawn for an activity (see `ui::icons`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pictogram {
    /// Three dots.
    Idle,
    /// Apple.
    Eat,
    /// Water drop.
    Drink,
    /// Crescent moon.
    Rest,
    /// Berries.
    Forage,
    /// Tree.
    Log,
    /// Rock with a crack.
    Quarry,
    /// Wheat ear.
    Farm,
    /// Hammer.
    Craft,
    /// House.
    Build,
    /// Two opposite arrows.
    Trade,
    /// Speech bubble.
    Talk,
    /// Gift box.
    Share,
    /// Crate.
    Store,
    /// Exclamation triangle.
    Steal,
    /// Heart.
    Court,
    /// Compass.
    Explore,
}

/// Display data of an activity: pictogram and badge colour (sRGB).
pub fn activity_style(activity: Activity) -> (Pictogram, [u8; 3]) {
    match activity {
        Activity::Idle => (Pictogram::Idle, [120, 128, 140]),
        Activity::Eat => (Pictogram::Eat, [222, 98, 78]),
        Activity::Drink => (Pictogram::Drink, [64, 150, 230]),
        Activity::Rest => (Pictogram::Rest, [128, 112, 200]),
        Activity::GatherFood => (Pictogram::Forage, [206, 72, 112]),
        Activity::GatherWood => (Pictogram::Log, [134, 98, 52]),
        Activity::GatherStone => (Pictogram::Quarry, [128, 136, 150]),
        Activity::Farm => (Pictogram::Farm, [204, 168, 48]),
        Activity::Craft => (Pictogram::Craft, [84, 140, 204]),
        Activity::Build => (Pictogram::Build, [214, 134, 60]),
        Activity::Trade => (Pictogram::Trade, [229, 192, 123]),
        Activity::Socialize => (Pictogram::Talk, [80, 182, 160]),
        Activity::Share => (Pictogram::Share, [112, 196, 98]),
        Activity::Store => (Pictogram::Store, [168, 128, 84]),
        Activity::Steal => (Pictogram::Steal, [232, 64, 64]),
        Activity::Mate => (Pictogram::Court, [236, 112, 168]),
        Activity::Explore => (Pictogram::Explore, [96, 176, 214]),
    }
}

/// egui colour of an activity badge.
pub fn activity_color32(activity: Activity) -> Color32 {
    let [r, g, b] = activity_style(activity).1;
    Color32::from_rgb(r, g, b)
}

/// Readable title-case label of an activity.
pub fn activity_title(activity: Activity) -> String {
    let label = activity.label();
    let mut chars = label.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().collect::<String>() + chars.as_str())
}

/// Group colour (model RGBA, sRGB in `[0, 1]`) as egui colour; grey for no group.
pub fn group_color32(color: Option<[f32; 4]>) -> Color32 {
    match color {
        Some(c) => interface::theme::color32_from_rgba([c[0], c[1], c[2], 1.0]),
        None => Color32::from_rgb(150, 156, 168),
    }
}

/// egui colour of a Bevy colour (sRGB, opaque).
pub fn color32(color: Color) -> Color32 {
    let [r, g, b] = color.to_srgba().to_u8_array_no_alpha();
    Color32::from_rgb(r, g, b)
}

/// Legend colour of a building: the dominant colour of its mesh (thatch,
/// timber roof, ripening crop).
pub fn building_color32(kind: civilization::BuildingKind) -> Color32 {
    match kind {
        civilization::BuildingKind::Hut => Color32::from_rgb(206, 170, 92),
        civilization::BuildingKind::Storage => Color32::from_rgb(132, 92, 60),
        civilization::BuildingKind::FarmPlot => Color32::from_rgb(168, 172, 76),
    }
}

/// Group colour as a Bevy colour; neutral cloth for people without a group.
pub fn group_color(color: Option<[f32; 4]>) -> Color {
    match color {
        Some(c) => Color::srgb(c[0], c[1], c[2]),
        None => Color::srgb_u8(176, 168, 150),
    }
}

/// Road surface colour: faint trail below the road threshold, packed earth
/// above it. Returns linear RGBA with the alpha used for blending.
pub fn road_rgba(intensity: f32, threshold: f32) -> [f32; 4] {
    let threshold = threshold.max(1e-3);
    let road = intensity >= threshold;
    let t = (intensity / threshold).clamp(0.0, 1.0);
    let srgb = if road {
        mix_srgb([196, 170, 124], [222, 200, 152], ((intensity - threshold) * 3.0).clamp(0.0, 1.0))
    } else {
        [170, 146, 104]
    };
    let c = linear(srgb, 1.0);
    // Trails must read from the whole-map view (emerging routes), yet stay
    // clearly fainter than roads.
    let alpha = if road { 0.92 } else { 0.1 + 0.45 * t };
    [c[0], c[1], c[2], alpha]
}

/// Half width (world units) of a road ribbon.
pub fn road_half_width(intensity: f32, threshold: f32) -> f32 {
    let t = (intensity / threshold.max(1e-3)).clamp(0.0, 3.0);
    0.04 + 0.06 * t.min(1.0) + 0.05 * (t - 1.0).max(0.0)
}

/// Linear colour of a group colour with an alpha.
pub fn linear_with_alpha(color: Color, alpha: f32) -> [f32; 4] {
    let LinearRgba { red, green, blue, .. } = color.to_linear();
    [red, green, blue, alpha]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_activity_has_a_distinct_style() {
        let mut seen = Vec::new();
        for a in Activity::ALL {
            let (pictogram, colour) = activity_style(a);
            assert!(!seen.contains(&pictogram), "{a:?} shares its pictogram");
            seen.push(pictogram);
            assert_ne!(colour, [0, 0, 0]);
            assert!(!activity_title(a).is_empty());
        }
        assert_eq!(activity_title(Activity::GatherFood), "Foraging");
    }

    #[test]
    fn terrain_colours_are_distinct_and_water_is_blue() {
        for (i, a) in Terrain::ALL.iter().enumerate() {
            for b in &Terrain::ALL[i + 1..] {
                assert_ne!(terrain_srgb(*a), terrain_srgb(*b), "{a:?} vs {b:?}");
            }
        }
        let [r, g, b] = terrain_srgb(Terrain::Water);
        assert!(b > r && b > g);
    }

    #[test]
    fn roads_are_more_opaque_and_wider_than_trails() {
        let trail = road_rgba(0.05, 0.2);
        let road = road_rgba(0.3, 0.2);
        assert!(road[3] > trail[3]);
        assert!(road_rgba(0.15, 0.2)[3] > trail[3], "busier trails are more visible");
        assert!(road_rgba(0.199, 0.2)[3] < road[3] - 0.25, "a trail stays fainter than a road");
        assert!(road_half_width(0.3, 0.2) > road_half_width(0.05, 0.2));
        assert!(road_half_width(0.9, 0.2) > road_half_width(0.3, 0.2), "busier roads are wider");
        assert!(road_half_width(5.0, 0.2) < 0.5, "a ribbon stays within its tile");
        let [r, g, b] = mix_srgb([0, 0, 0], [200, 100, 50], 0.5);
        assert_eq!([r, g, b], [100, 50, 25]);
    }

    #[test]
    fn group_colours_convert() {
        assert_eq!(group_color32(Some([1.0, 0.0, 0.0, 1.0])), Color32::from_rgb(255, 0, 0));
        assert_eq!(group_color32(None), Color32::from_rgb(150, 156, 168));
        assert_eq!(color32(group_color(None)), Color32::from_rgb(176, 168, 150), "legend swatch = undyed tunic");
        let kinds = civilization::BuildingKind::ALL.map(building_color32);
        assert!(kinds[0] != kinds[1] && kinds[1] != kinds[2] && kinds[0] != kinds[2]);
        let l = linear_with_alpha(Color::srgb(1.0, 1.0, 1.0), 0.3);
        assert!((l[0] - 1.0).abs() < 1e-5 && (l[3] - 0.3).abs() < 1e-6);
    }
}
