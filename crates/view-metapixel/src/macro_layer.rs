//! The macro layer: which representation of the metacells is drawn (the
//! exact readout, the live core counts or the accelerated prediction), how
//! each tile is shaded, the colours, and the labels that keep the exact
//! physical simulation apart from anything derived from it.

use metapixel::spec::{PERIOD, READOUT_ON_COUNT, READOUT_ON_STEADY_MIN};
use metapixel::{LiveReading, MAX_LIVE_READOUT_TILES, MacroGrid, MacroTile, MacroView, TileReading};

/// Which layer the world shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayerMode {
    /// Physical cells when zoomed in, the macro layer when zoomed out, a
    /// blend in between.
    #[default]
    Auto,
    /// Physical B3/S23 cells only.
    Cells,
    /// Macro layer only.
    Macro,
}

impl LayerMode {
    /// Every mode, in display order.
    pub const ALL: [LayerMode; 3] = [LayerMode::Auto, LayerMode::Cells, LayerMode::Macro];

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            LayerMode::Auto => "Auto",
            LayerMode::Cells => "Cells",
            LayerMode::Macro => "Macro",
        }
    }

    /// Tooltip.
    pub fn help(self) -> &'static str {
        match self {
            LayerMode::Auto => {
                "Physical cells when a tile is large on screen, the macro layer when it is small, blended in between"
            }
            LayerMode::Cells => "Only the physical B3/S23 cells of the construction (exact)",
            LayerMode::Macro => "Only the macro layer: one big cell per tile",
        }
    }

    /// The next mode (keyboard cycling).
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// Opacity of the macro layer for this mode given the automatic one.
    pub fn macro_opacity(self, auto: f32) -> f32 {
        match self {
            LayerMode::Auto => auto.clamp(0.0, 1.0),
            LayerMode::Cells => 0.0,
            LayerMode::Macro => 1.0,
        }
    }
}

/// What the macro layer shows for each tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MacroSource {
    /// The latest exact readout (core counted at `t = (n + 1)·P`), or the
    /// as-built pattern before the first readout.
    #[default]
    Readout,
    /// The core population now, classified by thresholds.
    Live,
    /// The accelerated macro prediction (not a physical simulation).
    Prediction,
}

impl MacroSource {
    /// Every source, in display order.
    pub const ALL: [MacroSource; 3] = [MacroSource::Readout, MacroSource::Live, MacroSource::Prediction];

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            MacroSource::Readout => "Readout",
            MacroSource::Live => "Live cores",
            MacroSource::Prediction => "Prediction",
        }
    }

    /// Tooltip.
    pub fn help(self) -> &'static str {
        match self {
            MacroSource::Readout => {
                "Macro states read from the physical cells: live cells counted in each tile's core [250, 1750]^2 at \
                 t = (n + 1)·35,328 (0 = OFF, 29,795 = ON, anything else ambiguous)"
            }
            MacroSource::Live => {
                "Core populations now, between readouts: 0 = OFF, at least 21,677 = ON, in between the display is \
                 switching (grids up to 1,024 tiles)"
            }
            MacroSource::Prediction => {
                "Accelerated macro representation, not a physical simulation: the macro rule applied directly to \
                 the macro grid, used only to check the readouts"
            }
        }
    }

    /// The next source (keyboard cycling).
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// How one tile is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TileShade {
    /// OFF (read, live or as built).
    Off,
    /// ON (read, live or as built).
    On,
    /// A readout core that was neither 0 nor 29,795.
    Ambiguous,
    /// A core between the thresholds now: its fraction of 29,795.
    Switching(f32),
    /// Predicted OFF by the accelerated model.
    PredictedOff,
    /// Predicted ON by the accelerated model.
    PredictedOn,
}

/// sRGB colours of the macro layer.
pub mod colors {
    /// OFF metacell.
    pub const OFF: [u8; 3] = [21, 26, 34];
    /// ON metacell (the mode accent).
    pub const ON: [u8; 3] = [242, 184, 75];
    /// Ambiguous readout.
    pub const AMBIGUOUS: [u8; 3] = [236, 72, 153];
    /// Predicted OFF (accelerated model).
    pub const PREDICTED_OFF: [u8; 3] = [18, 27, 40];
    /// Predicted ON (accelerated model).
    pub const PREDICTED_ON: [u8; 3] = [96, 165, 250];
    /// Mismatch outline.
    pub const MISMATCH: [u8; 3] = [248, 92, 92];
    /// Background of the physical layer.
    pub const CELL_BACKGROUND: [u8; 3] = [10, 12, 17];
    /// Markers drawn over the world (always on the dark cell background,
    /// whatever the interface theme): highlight.
    pub const WORLD_ACCENT: [u8; 3] = ON;
    /// Markers over the world: programmed / present.
    pub const WORLD_GOOD: [u8; 3] = [74, 222, 128];
    /// Markers over the world: removed / does not fit.
    pub const WORLD_BAD: [u8; 3] = MISMATCH;
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8)
}

impl TileShade {
    /// sRGB colour.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            TileShade::Off => colors::OFF,
            TileShade::On => colors::ON,
            TileShade::Ambiguous => colors::AMBIGUOUS,
            TileShade::Switching(f) => mix(colors::OFF, colors::ON, 0.15 + 0.7 * f),
            TileShade::PredictedOff => colors::PREDICTED_OFF,
            TileShade::PredictedOn => colors::PREDICTED_ON,
        }
    }

    /// Whether the tile is shown as ON (or predicted ON).
    pub fn is_on(self) -> bool {
        matches!(self, TileShade::On | TileShade::PredictedOn)
    }
}

/// Shade of a tile for `source`. `edit` (the generation-0 pattern being
/// edited) overrides the source while an editing tool is active.
pub fn tile_shade(tile: &MacroTile, source: MacroSource, edit: Option<bool>) -> TileShade {
    if let Some(on) = edit {
        return if on { TileShade::On } else { TileShade::Off };
    }
    let readout = || match tile.exact {
        Some(TileReading::On) => TileShade::On,
        Some(TileReading::Off) => TileShade::Off,
        Some(TileReading::Ambiguous(_)) => TileShade::Ambiguous,
        // Before the first readout the macro view carries the as-built
        // (metagen 0) pattern in `accelerated`.
        None if tile.accelerated => TileShade::On,
        None => TileShade::Off,
    };
    match source {
        MacroSource::Readout => readout(),
        MacroSource::Live => match tile.live {
            Some(LiveReading::On) => TileShade::On,
            Some(LiveReading::Off) => TileShade::Off,
            Some(LiveReading::Transitioning(count)) => {
                TileShade::Switching((count as f32 / READOUT_ON_COUNT as f32).clamp(0.0, 1.0))
            }
            None => readout(),
        },
        MacroSource::Prediction => {
            if tile.accelerated {
                TileShade::PredictedOn
            } else {
                TileShade::PredictedOff
            }
        }
    }
}

/// Edits waiting to be sent (a drag in progress) on top of the generation-0
/// pattern.
#[derive(Clone, Copy, Debug)]
pub struct EditOverlay<'a> {
    /// The generation-0 pattern of the construction.
    pub pattern: &'a MacroGrid,
    /// Tiles being painted.
    pub stroke: &'a [(u32, u32)],
    /// State the stroke paints.
    pub target: bool,
}

impl EditOverlay<'_> {
    /// State shown for tile `(col, row)`.
    pub fn state(&self, col: u32, row: u32) -> bool {
        if self.stroke.contains(&(col, row)) { self.target } else { self.pattern.get(col, row) }
    }
}

/// RGBA8 (sRGB) texture of the macro layer, one texel per tile, row 0 at
/// the top. An edit overlay of a different size than the view is ignored.
pub fn macro_texture(view: &MacroView, source: MacroSource, edit: Option<EditOverlay<'_>>) -> Vec<u8> {
    let edit = edit.filter(|e| (e.pattern.width(), e.pattern.height()) == (view.columns, view.rows));
    let mut out = Vec::with_capacity(view.tiles.len() * 4);
    for (i, tile) in view.tiles.iter().enumerate() {
        let (col, row) = (i as u32 % view.columns.max(1), i as u32 / view.columns.max(1));
        let shade = tile_shade(tile, source, edit.map(|e| e.state(col, row)));
        let [r, g, b] = shade.rgb();
        out.extend_from_slice(&[r, g, b, 255]);
    }
    out
}

/// Whether live core counts can be requested for a grid of `tiles` tiles.
pub fn live_available(tiles: usize) -> bool {
    tiles <= MAX_LIVE_READOUT_TILES
}

/// The source actually drawn: live core counts are only computed for grids
/// of up to [`MAX_LIVE_READOUT_TILES`] tiles, larger grids show the readout.
pub fn effective_source(source: MacroSource, tiles: usize) -> MacroSource {
    if source == MacroSource::Live && !live_available(tiles) { MacroSource::Readout } else { source }
}

/// Headline of the physical layer.
pub fn exact_headline(population: u64) -> String {
    format!("Exact: B3/S23 on {} live cells (Hashlife)", interface::format::format_count(population))
}

/// What the physical layer is, in full (tooltip of the headline).
pub fn exact_detail(size: (u64, u64)) -> String {
    format!(
        "The physical simulation: every cell of the {} × {}-cell plane follows Conway's rule B3/S23, generation \
         by generation. Hashlife computes it exactly (a memoised quadtree); nothing is approximated. Zoomed out, \
         each texel summarises a power-of-two block of these cells.",
        interface::format::format_count(size.0),
        interface::format::format_count(size.1)
    )
}

/// One-line name of what the macro layer shows.
pub fn macro_label(source: MacroSource, editing: bool) -> &'static str {
    if editing {
        return "Editing the generation-0 pattern (edits rebuild at t = 0)";
    }
    match source {
        MacroSource::Readout => "Readout: macro cells derived from tile populations",
        MacroSource::Live => "Live readout: tile core populations now, thresholded",
        MacroSource::Prediction => metapixel::ACCELERATED_LABEL,
    }
}

/// Headline of the macro layer for `source`.
pub fn macro_headline(source: MacroSource, view: &MacroView, editing: bool) -> String {
    if editing {
        return "Editing the generation-0 macro pattern (each edit rebuilds the construction at t = 0)".to_string();
    }
    match source {
        MacroSource::Readout => match (view.exact_metagen, view.exact_generation) {
            (Some(n), Some(t)) => format!(
                "Readout: macro cells derived from tile populations, metagen {} read at t = {}",
                interface::format::format_count(n),
                interface::format::format_count(t)
            ),
            _ => format!(
                "As built (metagen 0): no readout yet, the first one is at t = {}",
                interface::format::format_count(PERIOD)
            ),
        },
        MacroSource::Live => format!(
            "Readout (live): core populations now, 0 = OFF, at least {} = ON",
            interface::format::format_count(READOUT_ON_STEADY_MIN)
        ),
        MacroSource::Prediction => format!(
            "{} (metagen {})",
            metapixel::ACCELERATED_LABEL,
            interface::format::format_count(view.accelerated_metagen)
        ),
    }
}

/// Counts of the shades in a macro view (ON, OFF, ambiguous or switching).
pub fn shade_counts(view: &MacroView, source: MacroSource) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    for tile in &view.tiles {
        match tile_shade(tile, source, None) {
            TileShade::On | TileShade::PredictedOn => counts.0 += 1,
            TileShade::Off | TileShade::PredictedOff => counts.1 += 1,
            TileShade::Ambiguous | TileShade::Switching(_) => counts.2 += 1,
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(exact: Option<TileReading>, accelerated: bool, live: Option<LiveReading>) -> MacroTile {
        MacroTile { exact, accelerated, mismatch: false, population: 0, live }
    }

    fn view(tiles: Vec<MacroTile>, columns: u32) -> MacroView {
        let rows = tiles.len() as u32 / columns;
        MacroView {
            columns,
            rows,
            exact_metagen: Some(2),
            exact_generation: Some(3 * PERIOD),
            accelerated_metagen: 2,
            tiles,
            mismatch_count: 0,
            ambiguous_count: 0,
            exact_label: metapixel::EXACT_LABEL.into(),
            accelerated_label: metapixel::ACCELERATED_LABEL.into(),
        }
    }

    #[test]
    fn readout_shading_follows_the_exact_reading() {
        let on = tile(Some(TileReading::On), false, None);
        let off = tile(Some(TileReading::Off), true, None);
        let odd = tile(Some(TileReading::Ambiguous(17)), true, None);
        assert_eq!(tile_shade(&on, MacroSource::Readout, None), TileShade::On);
        assert_eq!(tile_shade(&off, MacroSource::Readout, None), TileShade::Off, "the prediction never leaks in");
        assert_eq!(tile_shade(&odd, MacroSource::Readout, None), TileShade::Ambiguous);
        // Before the first readout: the as-built pattern.
        assert_eq!(tile_shade(&tile(None, true, None), MacroSource::Readout, None), TileShade::On);
        assert_eq!(tile_shade(&tile(None, false, None), MacroSource::Readout, None), TileShade::Off);
    }

    #[test]
    fn live_and_prediction_shading() {
        let switching = tile(Some(TileReading::Off), false, Some(LiveReading::Transitioning(READOUT_ON_COUNT / 2)));
        match tile_shade(&switching, MacroSource::Live, None) {
            TileShade::Switching(f) => assert!((f - 0.5).abs() < 1e-3),
            other => panic!("{other:?}"),
        }
        let live_on = tile(Some(TileReading::Off), false, Some(LiveReading::On));
        assert_eq!(tile_shade(&live_on, MacroSource::Live, None), TileShade::On);
        let no_live = tile(Some(TileReading::On), false, None);
        assert_eq!(tile_shade(&no_live, MacroSource::Live, None), TileShade::On, "falls back to the readout");
        let predicted = tile(Some(TileReading::Off), true, None);
        assert_eq!(tile_shade(&predicted, MacroSource::Prediction, None), TileShade::PredictedOn);
        assert_ne!(TileShade::PredictedOn.rgb(), TileShade::On.rgb(), "predictions never look like readouts");
        assert_eq!(tile_shade(&predicted, MacroSource::Prediction, Some(false)), TileShade::Off, "edits win");
    }

    #[test]
    fn textures_have_one_texel_per_tile_row_major() {
        let v = view(
            vec![
                tile(Some(TileReading::On), false, None),
                tile(Some(TileReading::Off), false, None),
                tile(Some(TileReading::Off), false, None),
                tile(Some(TileReading::Ambiguous(5)), false, None),
            ],
            2,
        );
        let bytes = macro_texture(&v, MacroSource::Readout, None);
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[0..3], &colors::ON);
        assert_eq!(&bytes[4..7], &colors::OFF);
        assert_eq!(&bytes[12..15], &colors::AMBIGUOUS);
        assert!(bytes.chunks(4).all(|t| t[3] == 255));
        let pattern = MacroGrid::from_rows(&[".O", ".."]).unwrap();
        let edit = EditOverlay { pattern: &pattern, stroke: &[(0, 1)], target: true };
        let bytes = macro_texture(&v, MacroSource::Readout, Some(edit));
        assert_eq!(&bytes[0..3], &colors::OFF, "the edit pattern replaces the readout");
        assert_eq!(&bytes[4..7], &colors::ON);
        assert_eq!(&bytes[8..11], &colors::ON, "stroke tiles are previewed");
        let wrong_size = MacroGrid::new(3, 3).unwrap();
        let edit = EditOverlay { pattern: &wrong_size, stroke: &[], target: true };
        assert_eq!(&macro_texture(&v, MacroSource::Readout, Some(edit))[0..3], &colors::ON);
        assert_eq!(shade_counts(&v, MacroSource::Readout), (1, 2, 1));
    }

    #[test]
    fn modes_cycle_and_set_opacity() {
        assert_eq!(LayerMode::Auto.next(), LayerMode::Cells);
        assert_eq!(LayerMode::Macro.next(), LayerMode::Auto);
        assert_eq!(MacroSource::Prediction.next(), MacroSource::Readout);
        assert_eq!(LayerMode::Auto.macro_opacity(0.3), 0.3);
        assert_eq!(LayerMode::Cells.macro_opacity(0.3), 0.0);
        assert_eq!(LayerMode::Macro.macro_opacity(0.3), 1.0);
        assert!(live_available(1024) && !live_available(1025));
    }

    #[test]
    fn headlines_name_the_layer() {
        let v = view(vec![tile(Some(TileReading::On), false, None)], 1);
        assert!(macro_headline(MacroSource::Readout, &v, false).starts_with("Readout: macro cells derived"));
        assert!(macro_headline(MacroSource::Prediction, &v, false).contains("not a physical simulation"));
        assert!(macro_headline(MacroSource::Readout, &v, true).contains("generation-0"));
        assert!(macro_label(MacroSource::Readout, false).starts_with("Readout: macro cells derived from tile"));
        assert_eq!(macro_label(MacroSource::Prediction, false), metapixel::ACCELERATED_LABEL);
        assert!(macro_label(MacroSource::Live, true).contains("generation-0"));
        let unread = MacroView { exact_metagen: None, exact_generation: None, ..v };
        assert!(macro_headline(MacroSource::Readout, &unread, false).contains("35,328"));
        assert_eq!(exact_headline(342_076), "Exact: B3/S23 on 342,076 live cells (Hashlife)");
        assert!(exact_detail((6154, 6154)).contains("6,154 × 6,154-cell plane"));
        assert_eq!(effective_source(MacroSource::Live, 1024), MacroSource::Live);
        assert_eq!(effective_source(MacroSource::Live, 1025), MacroSource::Readout, "no live counts that large");
        assert_eq!(effective_source(MacroSource::Prediction, 65_536), MacroSource::Prediction);
    }
}
