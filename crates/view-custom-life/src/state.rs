//! UI state of the mode (world tools, display options, selection) and its
//! pure transitions.

use bevy::prelude::*;
use rendering::Colormap;
use rule_engine::PaintMode;

use crate::colors::ColorBy;

/// World tool of the left mouse button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Click an entity to inspect it; drag empty space to pan.
    #[default]
    Select,
    /// Click to create entities of the chosen type.
    Spawn,
    /// Click an entity to remove it (its carried resources drop to the field).
    Remove,
    /// Drag to paint a resource field with a disc brush.
    Paint,
}

impl Tool {
    /// Every tool in toolbar order.
    pub const ALL: [Tool; 4] = [Tool::Select, Tool::Spawn, Tool::Remove, Tool::Paint];

    /// Label.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Spawn => "Spawn",
            Tool::Remove => "Remove",
            Tool::Paint => "Paint",
        }
    }

    /// Keyboard shortcut (digit key).
    pub fn key_label(self) -> &'static str {
        match self {
            Tool::Select => "1",
            Tool::Spawn => "2",
            Tool::Remove => "3",
            Tool::Paint => "4",
        }
    }

    /// What the tool does.
    pub fn help(self) -> &'static str {
        match self {
            Tool::Select => "Click an entity to inspect it. Drag empty space to pan.",
            Tool::Spawn => "Click to create entities of the chosen type (undoable).",
            Tool::Remove => "Click an entity to remove it; its carried resources drop into the field (undoable).",
            Tool::Paint => "Drag to add to or set a resource field with a disc brush (one undo step per stroke).",
        }
    }

    /// The tool of a digit key (1–4).
    pub fn from_digit(digit: u8) -> Option<Tool> {
        Tool::ALL.get(usize::from(digit).checked_sub(1)?).copied()
    }
}

/// Colormaps offered for field overlays.
pub const FIELD_COLORMAPS: [Colormap; 6] =
    [Colormap::Viridis, Colormap::Inferno, Colormap::Magma, Colormap::Plasma, Colormap::TurboLike, Colormap::Grayscale];

/// Display and tool state of the mode (kept across scenario changes).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct ClUi {
    /// Active world tool.
    pub tool: Tool,
    /// What entity colours encode.
    pub color_by: ColorBy,
    /// Field shown under the entities (`None` = plain floor).
    pub overlay: Option<String>,
    /// Colormap of the field overlay.
    pub colormap: Colormap,
    /// Fraction of the colormap used for the field's range (dims bright ends).
    pub intensity: f32,
    /// Draw each type's glyph on its entities when zoomed in.
    pub glyphs: bool,
    /// Draw heading notches on mobile entities.
    pub headings: bool,
    /// Draw cell grid lines when zoomed in.
    pub grid: bool,
    /// Selected entity id.
    pub selected: Option<u64>,
    /// Entity under the cursor.
    pub hovered: Option<u64>,
    /// Keep the camera on the selected entity.
    pub follow: bool,
    /// Entity type created by the spawn tool.
    pub spawn_type: String,
    /// Entities created per click.
    pub spawn_count: u32,
    /// Scatter radius of a multi-entity spawn.
    pub spawn_radius: f32,
    /// Resource painted by the paint tool.
    pub paint_resource: String,
    /// Brush radius in cells.
    pub paint_radius: f32,
    /// Amount per cell (negative removes when adding).
    pub paint_amount: f32,
    /// Add to or set the field.
    pub paint_mode: PaintMode,
    /// Frame the whole world on the next update.
    pub fit_requested: bool,
    /// Whether the overlay legend is expanded.
    pub legend_open: bool,
    /// Where the overlay card was drawn last frame (window logical px);
    /// framing the world keeps clear of it when there is room.
    pub overlay_card: Option<Rect>,
}

impl Default for ClUi {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            color_by: ColorBy::Type,
            overlay: None,
            colormap: Colormap::Viridis,
            intensity: 0.85,
            glyphs: true,
            headings: true,
            grid: true,
            selected: None,
            hovered: None,
            follow: false,
            spawn_type: String::new(),
            spawn_count: 1,
            spawn_radius: 3.0,
            paint_resource: String::new(),
            paint_radius: 3.0,
            paint_amount: 0.5,
            paint_mode: PaintMode::Add,
            fit_requested: true,
            legend_open: true,
            overlay_card: None,
        }
    }
}

impl ClUi {
    /// Keeps names valid after the running system changed: tool targets
    /// fall back to the first available type / resource, an overlay naming
    /// a field that no longer exists is switched off.
    pub fn sync_names(&mut self, types: &[String], resources: &[String], fields: &[String]) {
        if !types.contains(&self.spawn_type) {
            self.spawn_type = types.first().cloned().unwrap_or_default();
        }
        if !resources.contains(&self.paint_resource) {
            self.paint_resource = resources.first().cloned().unwrap_or_default();
        }
        if self.overlay.as_ref().is_some_and(|o| !fields.contains(o)) {
            self.overlay = None;
        }
    }

    /// Forgets the selection and hover (new scenario, entity removed).
    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.hovered = None;
        self.follow = false;
    }

    /// The next overlay in the cycle `None → field 1 → … → None`.
    pub fn next_overlay(&self, fields: &[String]) -> Option<String> {
        match &self.overlay {
            None => fields.first().cloned(),
            Some(current) => {
                let i = fields.iter().position(|f| f == current)?;
                fields.get(i + 1).cloned()
            }
        }
    }

    /// The next colour mode (`Type → Energy → State → Type`).
    pub fn next_color_by(&self) -> ColorBy {
        let all = ColorBy::ALL;
        let i = all.iter().position(|c| *c == self.color_by).unwrap_or(0);
        all[(i + 1) % all.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn digits_map_to_tools() {
        assert_eq!(Tool::from_digit(1), Some(Tool::Select));
        assert_eq!(Tool::from_digit(4), Some(Tool::Paint));
        assert_eq!(Tool::from_digit(0), None);
        assert_eq!(Tool::from_digit(5), None);
        for t in Tool::ALL {
            assert_eq!(Tool::from_digit(t.key_label().parse().unwrap()), Some(t));
        }
    }

    #[test]
    fn names_follow_the_running_system() {
        let mut ui = ClUi { spawn_type: "fox".into(), overlay: Some("water".into()), ..ClUi::default() };
        ui.sync_names(&names(&["plant", "grazer"]), &names(&["sunlight"]), &names(&["sunlight", "daylight"]));
        assert_eq!(ui.spawn_type, "plant");
        assert_eq!(ui.paint_resource, "sunlight");
        assert_eq!(ui.overlay, None);
        ui.spawn_type = "grazer".into();
        ui.overlay = Some("daylight".into());
        ui.sync_names(&names(&["plant", "grazer"]), &names(&["sunlight"]), &names(&["sunlight", "daylight"]));
        assert_eq!(ui.spawn_type, "grazer", "valid choices are kept");
        assert_eq!(ui.overlay.as_deref(), Some("daylight"));
        ui.sync_names(&[], &[], &[]);
        assert_eq!((ui.spawn_type.as_str(), ui.paint_resource.as_str()), ("", ""));
    }

    #[test]
    fn overlay_and_colour_cycles() {
        let fields = names(&["a", "b"]);
        let mut ui = ClUi::default();
        assert_eq!(ui.next_overlay(&fields).as_deref(), Some("a"));
        ui.overlay = Some("a".into());
        assert_eq!(ui.next_overlay(&fields).as_deref(), Some("b"));
        ui.overlay = Some("b".into());
        assert_eq!(ui.next_overlay(&fields), None);
        assert_eq!(ui.next_overlay(&[]), None);
        assert_eq!(ui.next_color_by(), ColorBy::Energy);
        ui.color_by = ColorBy::State;
        assert_eq!(ui.next_color_by(), ColorBy::Type);
    }
}
