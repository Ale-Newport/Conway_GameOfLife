//! Application modes, the mode registry and the integration contract between
//! the shell and mode (view) crates.
//!
//! A mode crate:
//!
//! 1. calls [`ModeAppExt::register_mode`] with a [`ModeCard`] (this makes its
//!    dashboard card openable);
//! 2. inserts [`ActiveSession`] in `OnEnter(mode)` and removes it in
//!    `OnExit(mode)`;
//! 3. optionally contributes panel contents with
//!    [`ModeAppExt::add_mode_panel`] (one-shot systems taking
//!    `InMut<egui::Ui>`), scenarios through [`ModeScenarios`], and a tutorial
//!    through [`crate::tutorial::TutorialAppExt::register_tutorial`].
//!
//! See `docs/UI_SHELL.md` for the full contract.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use bevy::ecs::system::SystemId;
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32};
use simulation_core::SessionControl;

/// Top-level application state: the dashboard or one simulation mode.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum AppMode {
    /// Start screen with one card per mode.
    #[default]
    Dashboard,
    /// Conway's Game of Life and Golly-style tools.
    ClassicConway,
    /// Plants, herbivores, predators, fungi and nutrient cycles.
    EcosystemSimulator,
    /// User-defined entities and rules (no-code DSL and scripts).
    CustomLifeCreator,
    /// OTCA metapixel: Life simulated by Life.
    ConwayWithinConway,
    /// Life-like, Generations, Wireworld, Langton's ant, elementary CA.
    CellularAutomataLab,
    /// Heritable genomes, selection and speciation.
    EvolutionLab,
    /// Lenia, SmoothLife and Flow-Lenia.
    ContinuousLife,
    /// Three-dimensional cellular automata.
    ThreeDimensionalLife,
    /// Agents with needs, trade, buildings and groups.
    EmergentCivilizations,
}

impl AppMode {
    /// Every state, dashboard first.
    pub const ALL: [AppMode; 10] = [
        AppMode::Dashboard,
        AppMode::ClassicConway,
        AppMode::EcosystemSimulator,
        AppMode::CustomLifeCreator,
        AppMode::ConwayWithinConway,
        AppMode::CellularAutomataLab,
        AppMode::EvolutionLab,
        AppMode::ContinuousLife,
        AppMode::ThreeDimensionalLife,
        AppMode::EmergentCivilizations,
    ];

    /// The nine simulation modes in dashboard order.
    pub const SIMULATIONS: [AppMode; 9] = [
        AppMode::ClassicConway,
        AppMode::EcosystemSimulator,
        AppMode::CustomLifeCreator,
        AppMode::ConwayWithinConway,
        AppMode::CellularAutomataLab,
        AppMode::EvolutionLab,
        AppMode::ContinuousLife,
        AppMode::ThreeDimensionalLife,
        AppMode::EmergentCivilizations,
    ];

    /// Whether this is a simulation mode (not the dashboard).
    pub fn is_simulation(self) -> bool {
        self != AppMode::Dashboard
    }

    /// Stable kebab-case identifier used for folders, file names and the
    /// `--mode` command-line flag.
    pub fn slug(self) -> &'static str {
        match self {
            AppMode::Dashboard => "dashboard",
            AppMode::ClassicConway => "classic-conway",
            AppMode::EcosystemSimulator => "ecosystem",
            AppMode::CustomLifeCreator => "custom-life",
            AppMode::ConwayWithinConway => "conway-within-conway",
            AppMode::CellularAutomataLab => "cellular-automata",
            AppMode::EvolutionLab => "evolution",
            AppMode::ContinuousLife => "continuous-life",
            AppMode::ThreeDimensionalLife => "3d-life",
            AppMode::EmergentCivilizations => "civilizations",
        }
    }

    /// Parses a mode name: the slug, the variant name or the default title,
    /// ignoring case, spaces, `-` and `_` (`classic-conway`, `ClassicConway`,
    /// `classic_conway` and `Classic Conway` are equivalent).
    pub fn from_name(name: &str) -> Option<AppMode> {
        let wanted = normalize_name(name);
        if wanted.is_empty() {
            return None;
        }
        AppMode::ALL.into_iter().find(|mode| {
            normalize_name(mode.slug()) == wanted
                || normalize_name(&format!("{mode:?}")) == wanted
                || normalize_name(mode.default_title()) == wanted
        })
    }

    /// Default display title (used until a mode crate registers its card).
    pub fn default_title(self) -> &'static str {
        match self {
            AppMode::Dashboard => "Dashboard",
            AppMode::ClassicConway => "Classic Conway",
            AppMode::EcosystemSimulator => "Ecosystem Simulator",
            AppMode::CustomLifeCreator => "Custom Life Creator",
            AppMode::ConwayWithinConway => "Conway within Conway",
            AppMode::CellularAutomataLab => "Cellular Automata Lab",
            AppMode::EvolutionLab => "Evolution Lab",
            AppMode::ContinuousLife => "Continuous Life",
            AppMode::ThreeDimensionalLife => "3D Life",
            AppMode::EmergentCivilizations => "Emergent Civilizations",
        }
    }

    /// Default one-line tagline.
    pub fn default_tagline(self) -> &'static str {
        match self {
            AppMode::Dashboard => "All laboratories",
            AppMode::ClassicConway => "B3/S23, Hashlife and the pattern library",
            AppMode::EcosystemSimulator => "Food webs, climate, fire and nutrient cycles",
            AppMode::CustomLifeCreator => "Design entities and rules without code",
            AppMode::ConwayWithinConway => "The OTCA metapixel: Life computed by Life",
            AppMode::CellularAutomataLab => "Generations, Wireworld, ants and Wolfram rules",
            AppMode::EvolutionLab => "Heredity, mutation and selection in action",
            AppMode::ContinuousLife => "Lenia, SmoothLife and Flow-Lenia fields",
            AppMode::ThreeDimensionalLife => "Voxel automata in three dimensions",
            AppMode::EmergentCivilizations => "Needs, trade, roads and settlements",
        }
    }

    /// Default card description.
    pub fn default_description(self) -> &'static str {
        match self {
            AppMode::Dashboard => "Choose a laboratory.",
            AppMode::ClassicConway => {
                "Edit and run Conway's Game of Life with dense, sparse and Hashlife engines, \
                 a measured pattern library and Golly file formats."
            }
            AppMode::EcosystemSimulator => {
                "Individual plants and animals on generated terrain with weather, \
                 hydrology, fire, disease and conserved nutrient accounting."
            }
            AppMode::CustomLifeCreator => {
                "Define entity types and condition-to-action rules, then watch the \
                 population they produce. Scripts run in a sandbox."
            }
            AppMode::ConwayWithinConway => {
                "Tile OTCA metapixels so that each 2048 x 2048 block of Life behaves \
                 as one cell of a larger Life universe."
            }
            AppMode::CellularAutomataLab => {
                "Life-like and Generations rules, Brian's Brain, Wireworld, Langton's \
                 ant, turmites and the 256 elementary automata."
            }
            AppMode::EvolutionLab => {
                "Populations with heritable genomes in a patchy environment; selection \
                 emerges from survival and reproduction."
            }
            AppMode::ContinuousLife => {
                "Continuous states, kernels and growth functions: gliding Lenia \
                 creatures and SmoothLife structures."
            }
            AppMode::ThreeDimensionalLife => {
                "Survival/birth rules on Moore-26 and von Neumann-6 neighbourhoods, \
                 with slicing and surface views."
            }
            AppMode::EmergentCivilizations => {
                "Agents gather, produce, barter, build and form groups; territory and \
                 roads emerge from their decisions."
            }
        }
    }

    /// Default accent colour.
    pub fn default_accent(self) -> Color32 {
        match self {
            AppMode::Dashboard => Color32::from_rgb(56, 200, 180),
            AppMode::ClassicConway => Color32::from_rgb(90, 169, 255),
            AppMode::EcosystemSimulator => Color32::from_rgb(108, 203, 95),
            AppMode::CustomLifeCreator => Color32::from_rgb(199, 146, 234),
            AppMode::ConwayWithinConway => Color32::from_rgb(242, 184, 75),
            AppMode::CellularAutomataLab => Color32::from_rgb(79, 209, 197),
            AppMode::EvolutionLab => Color32::from_rgb(255, 122, 144),
            AppMode::ContinuousLife => Color32::from_rgb(125, 211, 252),
            AppMode::ThreeDimensionalLife => Color32::from_rgb(245, 158, 11),
            AppMode::EmergentCivilizations => Color32::from_rgb(229, 192, 123),
        }
    }

    /// Default short glyph shown next to the title (all present in egui's
    /// built-in fonts; checked by a test).
    pub fn default_glyph(self) -> &'static str {
        match self {
            AppMode::Dashboard => "★",
            AppMode::ClassicConway => "⊞",
            AppMode::EcosystemSimulator => "☘",
            AppMode::CustomLifeCreator => "⚙",
            AppMode::ConwayWithinConway => "▣",
            AppMode::CellularAutomataLab => "☰",
            AppMode::EvolutionLab => "⚘",
            AppMode::ContinuousLife => "◎",
            AppMode::ThreeDimensionalLife => "⬣",
            AppMode::EmergentCivilizations => "⌘",
        }
    }
}

impl fmt::Display for AppMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.default_title())
    }
}

fn normalize_name(name: &str) -> String {
    name.chars().filter(|c| !matches!(c, '-' | '_' | ' ')).flat_map(char::to_lowercase).collect()
}

/// Maturity of a registered mode, shown on its card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ModeStatus {
    /// Complete and supported.
    #[default]
    Ready,
    /// Usable but incomplete; the card carries a "Preview" badge.
    Preview,
}

impl ModeStatus {
    /// Badge text.
    pub fn label(self) -> &'static str {
        match self {
            ModeStatus::Ready => "Ready",
            ModeStatus::Preview => "Preview",
        }
    }
}

/// Dashboard card and title-bar metadata of a registered mode.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeCard {
    /// The state this card opens.
    pub mode: AppMode,
    /// Display title.
    pub title: String,
    /// One-line tagline.
    pub tagline: String,
    /// Two or three sentences for the dashboard card.
    pub description: String,
    /// Accent colour used for the card, selection and plots in this mode.
    pub accent: Color32,
    /// Short glyph (one or two characters) shown next to the title.
    pub glyph: String,
    /// Maturity badge.
    pub status: ModeStatus,
}

impl ModeCard {
    /// Card filled with the defaults of `mode` (status [`ModeStatus::Ready`]).
    pub fn defaults(mode: AppMode) -> Self {
        Self {
            mode,
            title: mode.default_title().to_string(),
            tagline: mode.default_tagline().to_string(),
            description: mode.default_description().to_string(),
            accent: mode.default_accent(),
            glyph: mode.default_glyph().to_string(),
            status: ModeStatus::Ready,
        }
    }
}

/// Errors raised while registering modes.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModeRegistryError {
    /// The dashboard is part of the shell and cannot be registered.
    #[error("the dashboard is built into the shell and cannot be registered as a mode")]
    Dashboard,
    /// A card for this mode exists already.
    #[error("mode `{0}` is already registered")]
    Duplicate(AppMode),
}

/// Registered modes. Cards of unregistered modes are shown on the dashboard
/// as "in development" and cannot be opened.
#[derive(Resource, Clone, Debug, Default)]
pub struct ModeRegistry {
    cards: Vec<ModeCard>,
}

impl ModeRegistry {
    /// Adds a card (kept in [`AppMode`] order).
    pub fn register(&mut self, card: ModeCard) -> Result<(), ModeRegistryError> {
        if card.mode == AppMode::Dashboard {
            return Err(ModeRegistryError::Dashboard);
        }
        if self.is_registered(card.mode) {
            return Err(ModeRegistryError::Duplicate(card.mode));
        }
        self.cards.push(card);
        self.cards.sort_by_key(|c| c.mode);
        Ok(())
    }

    /// Whether `mode` can be opened (the dashboard always can).
    pub fn is_registered(&self, mode: AppMode) -> bool {
        mode == AppMode::Dashboard || self.cards.iter().any(|c| c.mode == mode)
    }

    /// Card of a registered mode.
    pub fn card(&self, mode: AppMode) -> Option<&ModeCard> {
        self.cards.iter().find(|c| c.mode == mode)
    }

    /// Registered cards in mode order.
    pub fn cards(&self) -> &[ModeCard] {
        &self.cards
    }

    /// Accent of `mode`: the registered card's, else the default.
    pub fn accent(&self, mode: AppMode) -> Color32 {
        self.card(mode).map_or_else(|| mode.default_accent(), |c| c.accent)
    }

    /// Title of `mode`: the registered card's, else the default.
    pub fn title(&self, mode: AppMode) -> String {
        self.card(mode).map_or_else(|| mode.default_title().to_string(), |c| c.title.clone())
    }
}

/// The running simulation of the active mode, inserted by the mode on
/// `OnEnter` and removed on `OnExit`. The shell drives time controls, the
/// metrics store, save/load and parameter panels through it.
///
/// The shell is the **only** consumer of [`SessionControl::drain_metrics`]
/// and [`SessionControl::take_errors`]; modes read metrics from
/// [`crate::session::MetricsHistory`] and push messages to
/// [`crate::toasts::Toasts`].
#[derive(Resource, Clone)]
pub struct ActiveSession(pub Arc<dyn SessionControl>);

impl ActiveSession {
    /// Wraps a session.
    pub fn new(session: Arc<dyn SessionControl>) -> Self {
        Self(session)
    }

    /// The session.
    pub fn session(&self) -> &dyn SessionControl {
        self.0.as_ref()
    }
}

/// A one-shot system that draws mode content into a shell-provided `Ui`.
pub type PanelSystem = SystemId<InMut<'static, egui::Ui>>;

/// Where a mode panel system is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelSlot {
    /// Left side panel ("Tools").
    Left,
    /// First tab ("Properties") of the right side panel.
    Right,
    /// Thin strip below the top bar.
    Toolbar,
    /// Area over the top-left corner of the world viewport.
    Overlay,
}

/// Panel contents contributed by one mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModePanels {
    /// Left "Tools" panel.
    pub left: Option<PanelSystem>,
    /// "Properties" tab of the right panel.
    pub right: Option<PanelSystem>,
    /// Toolbar strip below the top bar.
    pub toolbar: Option<PanelSystem>,
    /// Overlay over the world viewport.
    pub overlay: Option<PanelSystem>,
}

impl ModePanels {
    /// Sets the system of one slot, returning the previous one.
    pub fn set(&mut self, slot: PanelSlot, system: PanelSystem) -> Option<PanelSystem> {
        let target = match slot {
            PanelSlot::Left => &mut self.left,
            PanelSlot::Right => &mut self.right,
            PanelSlot::Toolbar => &mut self.toolbar,
            PanelSlot::Overlay => &mut self.overlay,
        };
        target.replace(system)
    }
}

/// Panel systems of every mode.
#[derive(Resource, Clone, Debug, Default)]
pub struct ModePanelRegistry {
    panels: BTreeMap<AppMode, ModePanels>,
}

impl ModePanelRegistry {
    /// Panels of `mode` (empty when none were registered).
    pub fn get(&self, mode: AppMode) -> ModePanels {
        self.panels.get(&mode).copied().unwrap_or_default()
    }

    /// Replaces every panel of `mode`.
    pub fn set(&mut self, mode: AppMode, panels: ModePanels) {
        self.panels.insert(mode, panels);
    }

    /// Sets one slot of `mode`, returning the replaced system.
    pub fn set_slot(&mut self, mode: AppMode, slot: PanelSlot, system: PanelSystem) -> Option<PanelSystem> {
        self.panels.entry(mode).or_default().set(slot, system)
    }
}

/// One selectable scenario of a mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioEntry {
    /// Stable identifier sent back in [`ScenarioSelected`].
    pub id: String,
    /// Display name.
    pub title: String,
    /// Tooltip text.
    pub description: String,
}

/// Scenarios offered by one mode and the one currently loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenarioList {
    /// Entries in display order.
    pub entries: Vec<ScenarioEntry>,
    /// Identifier of the scenario currently loaded.
    pub selected: Option<String>,
}

/// Scenario lists that feed the top-bar picker. Modes fill their list (for
/// example on startup or `OnEnter`) and react to [`ScenarioSelected`].
#[derive(Resource, Clone, Debug, Default)]
pub struct ModeScenarios {
    lists: BTreeMap<AppMode, ScenarioList>,
}

impl ModeScenarios {
    /// Replaces the list of `mode`.
    pub fn set(&mut self, mode: AppMode, list: ScenarioList) {
        self.lists.insert(mode, list);
    }

    /// List of `mode`.
    pub fn get(&self, mode: AppMode) -> Option<&ScenarioList> {
        self.lists.get(&mode)
    }

    /// Marks `id` as the loaded scenario of `mode`. Returns `false` (and
    /// changes nothing) when `mode` has no such scenario.
    pub fn mark_selected(&mut self, mode: AppMode, id: &str) -> bool {
        match self.lists.get_mut(&mode) {
            Some(list) if list.entries.iter().any(|e| e.id == id) => {
                list.selected = Some(id.to_string());
                true
            }
            _ => false,
        }
    }
}

/// Written by the shell when the user picks a scenario in the top bar. The
/// mode loads it (typically by replacing its runner's simulation).
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct ScenarioSelected {
    /// Mode the scenario belongs to.
    pub mode: AppMode,
    /// [`ScenarioEntry::id`] of the chosen scenario.
    pub id: String,
}

/// Registration helpers for mode crates.
pub trait ModeAppExt {
    /// Makes `card.mode` openable from the dashboard. Registering the
    /// dashboard or the same mode twice is logged as an error and ignored.
    fn register_mode(&mut self, card: ModeCard) -> &mut Self;

    /// Registers a one-shot system drawing into `slot` while `mode` is
    /// active. The system's first parameter must be `InMut<egui::Ui>`.
    fn add_mode_panel<M>(
        &mut self,
        mode: AppMode,
        slot: PanelSlot,
        system: impl IntoSystem<InMut<'static, egui::Ui>, (), M> + 'static,
    ) -> &mut Self;

    /// Replaces every panel of `mode` with systems registered elsewhere
    /// (for example with `World::register_system`).
    fn set_mode_panels(&mut self, mode: AppMode, panels: ModePanels) -> &mut Self;
}

impl ModeAppExt for App {
    fn register_mode(&mut self, card: ModeCard) -> &mut Self {
        let mode = card.mode;
        let result = self.world_mut().get_resource_or_init::<ModeRegistry>().register(card);
        if let Err(e) = result {
            error!("cannot register mode {mode:?}: {e}");
        }
        self
    }

    fn add_mode_panel<M>(
        &mut self,
        mode: AppMode,
        slot: PanelSlot,
        system: impl IntoSystem<InMut<'static, egui::Ui>, (), M> + 'static,
    ) -> &mut Self {
        let id = self.world_mut().register_system(system);
        let replaced = self.world_mut().get_resource_or_init::<ModePanelRegistry>().set_slot(mode, slot, id);
        if let Some(old) = replaced
            && let Err(e) = self.world_mut().unregister_system(old)
        {
            warn!("could not unregister the replaced {slot:?} panel of {mode:?}: {e}");
        }
        self
    }

    fn set_mode_panels(&mut self, mode: AppMode, panels: ModePanels) -> &mut Self {
        self.world_mut().get_resource_or_init::<ModePanelRegistry>().set(mode, panels);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_aliases() {
        for mode in AppMode::ALL {
            assert_eq!(AppMode::from_name(mode.slug()), Some(mode), "{mode:?}");
            assert_eq!(AppMode::from_name(&format!("{mode:?}")), Some(mode));
            assert_eq!(AppMode::from_name(mode.default_title()), Some(mode));
        }
        assert_eq!(AppMode::from_name("classic_conway"), Some(AppMode::ClassicConway));
        assert_eq!(AppMode::from_name("  "), None);
        assert_eq!(AppMode::from_name("conway2"), None);
    }

    #[test]
    fn default_glyphs_exist_in_the_builtin_fonts() {
        let ctx = egui::Context::default();
        let _ = crate::run_headless_ui(&ctx, |_| {});
        for mode in AppMode::ALL {
            let glyph = mode.default_glyph();
            let present = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(16.0), glyph));
            assert!(present, "{mode:?} glyph {glyph} would render as a box");
        }
        // Typographic symbols used by the shell's labels.
        for symbol in ["•", "‹", "›", "—", "–", "×", "…", "↺", "·", "“", "”", "ø"] {
            let present = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), symbol));
            assert!(present, "symbol {symbol} would render as a box");
        }
    }

    #[test]
    fn slugs_are_unique_and_file_safe() {
        let mut slugs: Vec<_> = AppMode::ALL.iter().map(|m| m.slug()).collect();
        assert!(slugs.iter().all(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')));
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), AppMode::ALL.len());
    }

    #[test]
    fn registry_rules() {
        let mut registry = ModeRegistry::default();
        assert!(registry.is_registered(AppMode::Dashboard));
        assert!(!registry.is_registered(AppMode::EvolutionLab));
        assert_eq!(registry.register(ModeCard::defaults(AppMode::Dashboard)), Err(ModeRegistryError::Dashboard));
        registry.register(ModeCard::defaults(AppMode::EvolutionLab)).unwrap();
        registry.register(ModeCard::defaults(AppMode::ClassicConway)).unwrap();
        assert_eq!(
            registry.register(ModeCard::defaults(AppMode::ClassicConway)),
            Err(ModeRegistryError::Duplicate(AppMode::ClassicConway))
        );
        let order: Vec<_> = registry.cards().iter().map(|c| c.mode).collect();
        assert_eq!(order, vec![AppMode::ClassicConway, AppMode::EvolutionLab]);
        assert!(registry.is_registered(AppMode::EvolutionLab));
        assert_eq!(registry.title(AppMode::ContinuousLife), "Continuous Life");
    }

    #[test]
    fn scenario_selection_requires_known_id() {
        let mut scenarios = ModeScenarios::default();
        let entry = ScenarioEntry { id: "gun".into(), title: "Gun".into(), description: String::new() };
        scenarios.set(AppMode::ClassicConway, ScenarioList { entries: vec![entry], selected: None });
        assert!(!scenarios.mark_selected(AppMode::ClassicConway, "nope"));
        assert!(scenarios.mark_selected(AppMode::ClassicConway, "gun"));
        assert_eq!(scenarios.get(AppMode::ClassicConway).unwrap().selected.as_deref(), Some("gun"));
        assert!(!scenarios.mark_selected(AppMode::EvolutionLab, "gun"));
    }

    #[derive(Resource, Default)]
    struct Drawn(Vec<String>);

    fn tools(InMut(ui): InMut<egui::Ui>, mut drawn: ResMut<Drawn>) {
        ui.label("tools");
        drawn.0.push("tools".into());
    }

    /// Verifies the panel contract on Bevy 0.20: one-shot systems taking
    /// `InMut<egui::Ui>` registered on the App run inside an egui pass from
    /// exclusive world access.
    #[test]
    fn one_shot_panel_systems_run_with_a_ui() {
        let mut app = App::new();
        app.init_resource::<Drawn>();
        app.add_mode_panel(AppMode::ClassicConway, PanelSlot::Left, tools);
        let panels = app.world().resource::<ModePanelRegistry>().get(AppMode::ClassicConway);
        let id = panels.left.unwrap();
        assert!(panels.right.is_none());
        let ctx = egui::Context::default();
        let world = app.world_mut();
        let mut result = None;
        let _ = crate::run_headless_ui(&ctx, |ui| {
            result = Some(world.run_system_with(id, ui));
        });
        assert!(matches!(result, Some(Ok(()))));
        assert_eq!(world.resource::<Drawn>().0, vec!["tools".to_string()]);
    }
}
