//! Guided tutorials and concept pages, one per mode.
//!
//! A mode registers a [`Tutorial`] with
//! [`TutorialAppExt::register_tutorial`]. Pages are shown in a window with
//! previous / next navigation; a page may carry an action button that
//! writes [`TutorialAction`] (for example `"load:glider-gun"`) for the mode
//! to react to. The "Concepts" tab renders the tutorial's Markdown concepts
//! page.

use std::collections::BTreeMap;

use bevy::prelude::*;

use crate::mode::AppMode;

/// Button on a tutorial page that asks the mode to do something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TutorialStepAction {
    /// Identifier sent in [`TutorialAction`].
    pub id: String,
    /// Button text.
    pub label: String,
}

/// One tutorial page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TutorialPage {
    /// Page title.
    pub title: String,
    /// Body (Markdown).
    pub body: String,
    /// Optional action button.
    pub action: Option<TutorialStepAction>,
}

impl TutorialPage {
    /// Page without an action.
    pub fn new(title: &str, body: &str) -> Self {
        Self { title: title.to_string(), body: body.to_string(), action: None }
    }

    /// Adds an action button.
    pub fn with_action(mut self, id: &str, label: &str) -> Self {
        self.action = Some(TutorialStepAction { id: id.to_string(), label: label.to_string() });
        self
    }
}

/// Tutorial pages and the concepts page of one mode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tutorial {
    /// Step-by-step pages.
    pub pages: Vec<TutorialPage>,
    /// Background concepts (Markdown).
    pub concepts: String,
}

/// Written when the user presses a tutorial page's action button.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct TutorialAction(pub String);

/// Tutorials keyed by mode.
#[derive(Resource, Clone, Debug, Default)]
pub struct TutorialRegistry {
    tutorials: BTreeMap<AppMode, Tutorial>,
}

impl TutorialRegistry {
    /// Registers (or replaces) the tutorial of `mode`.
    pub fn insert(&mut self, mode: AppMode, tutorial: Tutorial) {
        self.tutorials.insert(mode, tutorial);
    }

    /// Tutorial of `mode`.
    pub fn get(&self, mode: AppMode) -> Option<&Tutorial> {
        self.tutorials.get(&mode)
    }
}

/// Registration helper.
pub trait TutorialAppExt {
    /// Registers the tutorial of `mode`.
    fn register_tutorial(&mut self, mode: AppMode, tutorial: Tutorial) -> &mut Self;
}

impl TutorialAppExt for App {
    fn register_tutorial(&mut self, mode: AppMode, tutorial: Tutorial) -> &mut Self {
        self.world_mut().get_resource_or_init::<TutorialRegistry>().insert(mode, tutorial);
        self
    }
}

/// Tab of the tutorial window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TutorialTab {
    /// Step-by-step pages.
    #[default]
    Steps,
    /// Concepts page.
    Concepts,
}

/// State of the tutorial window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TutorialView {
    /// Window open.
    pub open: bool,
    /// Current page index.
    pub page: usize,
    /// Selected tab.
    pub tab: TutorialTab,
}

impl TutorialView {
    /// Moves `delta` pages, staying within `0..page_count`.
    pub fn turn(&mut self, delta: isize, page_count: usize) {
        let last = page_count.saturating_sub(1) as isize;
        self.page = (self.page as isize + delta).clamp(0, last.max(0)) as usize;
    }
}

/// The dashboard's own tutorial: how to use the application.
pub fn dashboard_tutorial() -> Tutorial {
    Tutorial {
        pages: vec![
            TutorialPage::new(
                "Welcome to Emergence Lab",
                "Emergence Lab is a set of laboratories for **artificial life**: simple local rules \
                 that produce complex global behaviour.\n\nEach card on the dashboard opens one \
                 laboratory. Cards marked *in development* are not available in this build yet.",
            ),
            TutorialPage::new(
                "Running a simulation",
                "Inside a laboratory the **bottom bar** controls time:\n\n\
                 - **Play / pause** (Space) runs ticks on a background thread.\n\
                 - **Step** (N) advances one tick; **Step N** (Shift+N) advances the chosen count.\n\
                 - **Speed** sets ticks per second on a logarithmic scale; **Max** runs as fast as \
                 the model allows. The measured rate is shown when the model is slower than requested.\n\
                 - **Reset** (R) returns to the scenario's initial state. It is undoable (Ctrl/Cmd+Z).",
            ),
            TutorialPage::new(
                "Panels",
                "The **left panel** holds the laboratory's tools. The **right panel** has tabs for \
                 the laboratory's properties, the model's **parameters** (each marked with when a \
                 change takes effect: *live*, *next tick* or *on reset*), the formal **rules** that \
                 are executed, and live **metric** plots.\n\nPress **Tab** to hide the panels and \
                 **F11** for fullscreen.",
            ),
            TutorialPage::new(
                "History, saving and exporting",
                "The **timeline** in the bottom bar marks stored snapshots; click one to go back to \
                 that tick and continue from there.\n\n**Save** stores the complete state, including \
                 the random generator, so a loaded run continues exactly as the original would. \
                 **Export** writes the recorded metrics as CSV or JSON, or captures screenshots and \
                 PNG frame sequences.",
            ),
        ],
        concepts: "## Emergence\n\
            Emergent behaviour is structure that no single rule mentions: gliders in Life, \
            predator–prey cycles, trade routes. The laboratories let you change rules and \
            watch which structures survive.\n\n\
            ## Ticks, not frames\n\
            Simulations advance in **ticks** of fixed simulated duration. The speed control \
            only decides how many ticks run per second; the rendering frame rate never changes \
            results.\n\n\
            ## Determinism and seeds\n\
            All randomness comes from a seeded generator stored in the simulation state. The \
            same seed, scenario and interventions give the same run, bit for bit, on the same \
            build and platform; saved states include the generator, so a restored run replays \
            identically.\n\n\
            ## Measurements\n\
            Every model publishes formally defined **metrics** (see the Rules tab's metrics \
            guide). Plots show the recorded samples; very long runs keep the first, minimum, \
            maximum and last sample of older time buckets so extremes are never lost."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_turning_is_clamped() {
        let mut view = TutorialView::default();
        view.turn(-1, 4);
        assert_eq!(view.page, 0);
        view.turn(1, 4);
        view.turn(5, 4);
        assert_eq!(view.page, 3);
        view.turn(-2, 4);
        assert_eq!(view.page, 1);
        view.turn(1, 0);
        assert_eq!(view.page, 0, "no pages");
    }

    #[test]
    fn registry_and_builtin_tutorial() {
        let mut app = App::new();
        let tutorial = Tutorial {
            pages: vec![TutorialPage::new("Gun", "Load it").with_action("load:gun", "Load the gun")],
            concepts: "# Guns".into(),
        };
        app.register_tutorial(AppMode::ClassicConway, tutorial.clone());
        let registry = app.world().resource::<TutorialRegistry>();
        assert_eq!(registry.get(AppMode::ClassicConway), Some(&tutorial));
        assert!(registry.get(AppMode::EvolutionLab).is_none());
        let builtin = dashboard_tutorial();
        assert!(builtin.pages.len() >= 3 && !builtin.concepts.is_empty());
    }
}
