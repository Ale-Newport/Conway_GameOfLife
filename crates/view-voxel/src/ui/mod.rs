//! Panels of the mode: Tools (left), Properties (first right tab) and the
//! overlay over the world, plus shared widgets and editor state.

pub(crate) mod overlay;
pub(crate) mod properties;
pub(crate) mod tools;

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Ui};
use interface::Palette;
use interface::theme::with_alpha;
use simulation_core::{ParamSet, ParamValue};
use voxel_life::Rule3D;

/// Panel state kept between frames.
#[derive(Resource, Default)]
pub(crate) struct UiState {
    /// Rule editor of the Tools panel.
    pub rule_editor: RuleEditor,
    /// Seeding controls of the Tools panel.
    pub seed: SeedDraft,
    /// Texture of the slice inspector and the slice version it shows.
    pub slice_texture: Option<(egui::TextureHandle, u64)>,
    /// Whether the overlay shows only its title line.
    pub overlay_collapsed: bool,
}

/// Text field state of the rule editor.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct RuleEditor {
    /// Text being edited.
    pub text: String,
    /// The rule in effect when the text was last synchronised.
    base: Option<String>,
    /// Whether the user changed the text since.
    pub edited: bool,
}

impl RuleEditor {
    /// Follows the rule in effect while the user is not editing.
    pub(crate) fn sync(&mut self, current: &str) {
        if self.base.as_deref() != Some(current) {
            self.base = Some(current.to_string());
            if !self.edited {
                self.text = current.to_string();
            }
        }
    }

    /// The parsed text.
    pub(crate) fn parsed(&self) -> Result<Rule3D, String> {
        Rule3D::parse(&self.text).map_err(|e| e.to_string())
    }

    /// Whether the text is a valid rule different from `current`.
    pub(crate) fn can_apply(&self, current: Option<&Rule3D>) -> bool {
        self.parsed().is_ok_and(|rule| current != Some(&rule))
    }

    /// Puts `text` into the editor (a preset).
    pub(crate) fn load(&mut self, text: &str) {
        self.text = text.to_string();
        self.edited = true;
    }

    /// Discards the edit.
    pub(crate) fn revert(&mut self) {
        if let Some(base) = &self.base {
            self.text = base.clone();
        }
        self.edited = false;
    }

    /// Marks the edit as sent; the text follows the model again.
    pub(crate) fn applied(&mut self) {
        self.edited = false;
    }
}

/// Values of the seeding parameters (pending until the next reset or
/// randomisation).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SeedValues {
    /// Random cube side.
    pub cube_side: i64,
    /// Alive probability.
    pub density: f64,
    /// Mirror symmetry key.
    pub mirror: String,
    /// Seed.
    pub seed: i64,
    /// Initial-state kind key.
    pub init: String,
}

impl SeedValues {
    /// Reads the values from the session parameters.
    pub(crate) fn from_params(params: &ParamSet) -> Option<Self> {
        Some(Self {
            cube_side: params.get("cube_side")?.as_i64()?,
            density: params.get("density")?.as_f64()?,
            mirror: params.get("mirror")?.as_str()?.to_string(),
            seed: params.get("seed")?.as_i64()?,
            init: params.get("init")?.as_str()?.to_string(),
        })
    }
}

/// Local copy of the seeding parameters for the sliders.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct SeedDraft {
    /// Values shown by the widgets.
    pub values: Option<SeedValues>,
    seen: Option<SeedValues>,
    /// Draft last sent to the session (sent once, not every frame until the
    /// status catches up).
    pub sent: Option<SeedValues>,
}

impl SeedDraft {
    /// Adopts parameter values that changed in the session (another panel,
    /// a scenario load) unless a widget is being dragged.
    pub(crate) fn sync(&mut self, params: &ParamSet, interacting: bool) {
        let Some(current) = SeedValues::from_params(params) else { return };
        if self.seen.as_ref() != Some(&current) && !interacting {
            self.values = Some(current.clone());
            self.seen = Some(current);
        }
    }

    /// The parameters that differ between the draft and `current`.
    pub(crate) fn changes(draft: &SeedValues, current: &SeedValues) -> Vec<(&'static str, ParamValue)> {
        let mut out = Vec::new();
        if draft.cube_side != current.cube_side {
            out.push(("cube_side", ParamValue::Int(draft.cube_side)));
        }
        if (draft.density - current.density).abs() > 1e-9 {
            out.push(("density", ParamValue::Float(draft.density)));
        }
        if draft.mirror != current.mirror {
            out.push(("mirror", ParamValue::Text(draft.mirror.clone())));
        }
        if draft.seed != current.seed {
            out.push(("seed", ParamValue::Int(draft.seed)));
        }
        if draft.init != current.init {
            out.push(("init", ParamValue::Text(draft.init.clone())));
        }
        out
    }
}

/// Uppercase section caption, like the shell's panel titles.
pub(crate) fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title.to_uppercase()).size(11.0).strong().color(palette.text_muted));
    ui.add_space(2.0);
}

/// A raised card.
pub(crate) fn card<R>(ui: &mut Ui, palette: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(palette.stroke_width, palette.stroke))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A card with a coloured left edge for notices.
pub(crate) fn notice(ui: &mut Ui, palette: &Palette, color: Color32, add: impl FnOnce(&mut Ui)) {
    let response = egui::Frame::new()
        .fill(with_alpha(color, if palette.dark { 0.10 } else { 0.12 }))
        .stroke(Stroke::new(1.0, with_alpha(color, 0.45)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin { left: 12, right: 10, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response;
    let r = response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(r.min, egui::pos2(r.min.x + 3.0, r.max.y)),
        CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 },
        color,
    );
}

/// Narrowest slider rail.
const MIN_SLIDER: f32 = 48.0;
/// Widest slider rail.
const MAX_SLIDER: f32 = 260.0;

/// The slider rail width that makes content fill `available` points:
/// `reserve` estimates the labels and value boxes beside the rail and
/// `correction` is the error of that estimate measured on earlier frames.
pub(crate) fn rail_width(available: f32, reserve: f32, correction: f32) -> f32 {
    (available - reserve - correction).clamp(MIN_SLIDER, MAX_SLIDER)
}

/// The corrected estimate after content laid out with rail `width` turned
/// out `overshoot` points wider than the room (negative: narrower). A
/// clamped rail cannot absorb the error, so it does not teach anything.
pub(crate) fn next_correction(correction: f32, width: f32, overshoot: f32) -> f32 {
    let absorbable = if overshoot > 0.0 { width > MIN_SLIDER } else { width < MAX_SLIDER };
    if overshoot.abs() < 0.5 || !absorbable {
        return correction;
    }
    (correction + overshoot).clamp(-200.0, 400.0)
}

/// Extra reserve before the first reliable measurement of a row.
const FIRST_MARGIN: f32 = 40.0;

/// Adds content whose sliders share the room left after `reserve` points of
/// labels and value boxes. egui panels grow to fit their content (and do
/// not shrink back), so a slider sized from the available width with a too
/// small reserve would widen the panel a little every frame. The content is
/// therefore first laid out with a safety margin, and from the second frame
/// on (egui grids measure their columns on their first frame) the measured
/// overshoot corrects the reserve, so the content converges to the panel
/// width instead of pushing it.
pub(crate) fn fitted<R>(ui: &mut Ui, salt: &str, reserve: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let id = ui.id().with(("view_voxel_fit", salt));
    let available = ui.available_width();
    let stored = ui.ctx().data(|d| d.get_temp::<f32>(id));
    let correction = stored.unwrap_or(FIRST_MARGIN);
    let width = rail_width(available, reserve, correction);
    let inner = ui.scope(|ui| {
        ui.spacing_mut().slider_width = width;
        add(ui)
    });
    let reliable = stored.is_some() && !ui.ctx().will_discard();
    let next =
        if reliable { next_correction(correction, width, inner.response.rect.width() - available) } else { correction };
    if stored != Some(next) {
        ui.ctx().data_mut(|d| d.insert_temp(id, next));
    }
    inner.inner
}

/// Small muted explanatory text.
pub(crate) fn hint(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(palette.text_muted));
}

/// Row of mutually exclusive buttons; returns whether the value changed.
pub(crate) fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    palette: &Palette,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (option, label) in options {
            let selected = *value == *option;
            let text = RichText::new(*label).size(12.5).color(if selected { palette.on_accent } else { palette.text });
            let button = egui::Button::new(text)
                .fill(if selected { palette.accent } else { palette.surface_hi })
                .stroke(Stroke::new(1.0, if selected { palette.accent } else { palette.stroke }))
                .corner_radius(CornerRadius::same(4))
                .min_size(egui::vec2(0.0, 24.0));
            if ui.add(button).clicked() && !selected {
                *value = *option;
                changed = true;
            }
        }
    });
    changed
}

/// A collapsible section of the Tools panel.
pub(crate) fn collapsible(ui: &mut Ui, title: &str, open: bool, add: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong().size(13.5))
        .id_salt(("view_voxel_section", title))
        .default_open(open)
        .show(ui, |ui| {
            ui.add_space(2.0);
            add(ui);
            ui.add_space(4.0);
        });
}

/// Monospace text.
pub(crate) fn mono(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).monospace().size(size)
}

/// Thousands-separated count.
pub(crate) fn count(value: u64) -> String {
    interface::format::format_count(value)
}

/// A horizontal bar split into coloured parts proportional to `parts`.
pub(crate) fn stacked_bar(ui: &mut Ui, palette: &Palette, parts: &[(f64, Color32)], height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().rect_filled(rect, CornerRadius::same(3), palette.surface_hi);
    let total: f64 = parts.iter().map(|(v, _)| v.max(0.0)).sum();
    if total <= 0.0 {
        return;
    }
    let mut x = rect.min.x;
    for (value, color) in parts {
        let w = (value.max(0.0) / total) as f32 * rect.width();
        if w > 0.0 {
            let part =
                egui::Rect::from_min_max(egui::pos2(x, rect.min.y), egui::pos2((x + w).min(rect.max.x), rect.max.y));
            ui.painter().rect_filled(part, CornerRadius::ZERO, *color);
            x += w;
        }
    }
    ui.painter().rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
}

/// A small round colour dot (legend for list rows).
pub(crate) fn swatch_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.5, color);
    ui.painter().circle_stroke(rect.center(), 4.5, Stroke::new(1.0, with_alpha(Color32::BLACK, 0.25)));
}

/// Name of a state.
pub(crate) fn state_name(state: u8) -> String {
    match state {
        0 => "empty".to_string(),
        1 => "alive".to_string(),
        k => format!("dying {k}"),
    }
}

/// A set of neighbour counts as `{1, 3, 5–7}`.
pub(crate) fn count_set(set: voxel_life::CountSet) -> String {
    if set.is_empty() {
        return "none".to_string();
    }
    let parts: Vec<String> = set
        .runs()
        .iter()
        .map(|&(a, b)| {
            if a == b {
                a.to_string()
            } else if b == a + 1 {
                format!("{a}, {b}")
            } else {
                format!("{a}–{b}")
            }
        })
        .collect();
    format!("{{{}}}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_editor_follows_the_model_until_edited() {
        let mut editor = RuleEditor::default();
        editor.sync("5-7/6/2/M");
        assert_eq!(editor.text, "5-7/6/2/M");
        assert!(!editor.can_apply(Some(&Rule3D::bays_5766())), "same rule");
        editor.load("4555");
        assert!(editor.can_apply(Some(&Rule3D::bays_5766())));
        // A model change while editing keeps the user's text.
        editor.sync("2,6,9/4,6,8-9/10/M");
        assert_eq!(editor.text, "4555");
        editor.revert();
        assert_eq!(editor.text, "2,6,9/4,6,8-9/10/M");
        editor.load("B0/S1");
        assert!(editor.parsed().is_err());
        assert!(!editor.can_apply(None));
        editor.load("4555");
        editor.applied();
        editor.sync("4-5/5/2/M");
        assert_eq!(editor.text, "4-5/5/2/M", "after applying, the canonical form appears");
    }

    #[test]
    fn seed_draft_syncs_and_reports_changes() {
        let params = |side: i64, mirror: &str| {
            ParamSet::new()
                .with("cube_side", ParamValue::Int(side))
                .with("density", ParamValue::Float(0.5))
                .with("mirror", ParamValue::Text(mirror.into()))
                .with("seed", ParamValue::Int(3))
                .with("init", ParamValue::Text("random_cube".into()))
        };
        let mut draft = SeedDraft::default();
        draft.sync(&params(12, "none"), false);
        let mut values = draft.values.clone().expect("synced");
        assert_eq!(values.cube_side, 12);
        values.cube_side = 20;
        values.mirror = "xyz".into();
        let current = SeedValues::from_params(&params(12, "none")).expect("complete");
        let changes = SeedDraft::changes(&values, &current);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0], ("cube_side", ParamValue::Int(20)));
        // A change from elsewhere is adopted, but not while dragging.
        draft.values = Some(values);
        draft.sync(&params(16, "none"), true);
        assert_eq!(draft.values.as_ref().map(|v| v.cube_side), Some(20));
        draft.sync(&params(16, "none"), false);
        assert_eq!(draft.values.as_ref().map(|v| v.cube_side), Some(16));
        assert!(SeedValues::from_params(&ParamSet::new()).is_none());
    }

    #[test]
    fn slider_rails_converge_to_the_room_without_widening() {
        // Content = rail + 150 points of labels; the estimate says 120.
        let (available, reserve, overhead) = (265.0, 120.0, 150.0);
        let mut correction = FIRST_MARGIN;
        let mut widest: f32 = 0.0;
        for frame in 0..4 {
            let rail = rail_width(available, reserve, correction);
            let content = rail + overhead;
            widest = widest.max(content);
            // The first frame only lays out (egui grids measure then).
            if frame > 0 {
                correction = next_correction(correction, rail, content - available);
            }
        }
        assert!(widest <= available, "never wider than the panel: {widest}");
        assert!((rail_width(available, reserve, correction) + overhead - available).abs() < 0.5, "fills it");
        // A clamped rail cannot absorb an error, so nothing is learnt.
        assert_eq!(next_correction(5.0, MIN_SLIDER, 30.0), 5.0);
        assert_eq!(next_correction(5.0, MAX_SLIDER, -30.0), 5.0);
        assert_eq!(next_correction(5.0, 100.0, 0.2), 5.0, "sub-pixel noise is ignored");
        assert_eq!(rail_width(100.0, 400.0, 0.0), MIN_SLIDER);
        assert_eq!(rail_width(2000.0, 10.0, 0.0), MAX_SLIDER);
    }

    #[test]
    fn count_sets_read_naturally() {
        let rule = Rule3D::parse("2,6,9/4,6,8-9/10/M").expect("valid");
        assert_eq!(count_set(rule.survive()), "{2, 6, 9}");
        assert_eq!(count_set(rule.birth()), "{4, 6, 8, 9}");
        assert_eq!(count_set(Rule3D::parse("13-26/13-14,17-19/2/M").expect("valid").birth()), "{13, 14, 17–19}");
        assert_eq!(count_set(Rule3D::parse("/1/2/M").expect("valid").survive()), "none");
        assert_eq!(state_name(0), "empty");
        assert_eq!(state_name(4), "dying 4");
    }

    #[test]
    fn glyphs_used_by_the_mode_exist_in_the_default_fonts() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        let mut text = String::new();
        text.push_str(crate::tutorial::CONCEPTS);
        for page in crate::tutorial::tutorial().pages {
            text.push_str(&page.title);
            text.push_str(&page.body);
        }
        text.push_str(overlay::GLYPHS);
        text.push_str(properties::GLYPHS);
        text.push_str(tools::GLYPHS);
        for scenario in crate::builtin_scenarios() {
            let life = voxel_life::VoxelLife::new(scenario.clone()).expect("valid scenario");
            let d = crate::session::displayable(simulation_core::Simulation::descriptor(&life));
            text.push_str(&d.summary);
            text.push_str(&d.rules);
            for p in &d.params {
                text.push_str(&p.label);
                text.push_str(&p.description);
            }
            for m in &d.metrics {
                text.push_str(&m.label);
                text.push_str(&m.description);
            }
        }
        let font = egui::FontId::proportional(14.0);
        let missing: String =
            ctx.fonts_mut(|f| text.chars().filter(|c| !c.is_ascii() && !f.has_glyphs(&font, &c.to_string())).collect());
        assert!(missing.is_empty(), "glyphs missing from egui's fonts: {missing}");
    }
}
