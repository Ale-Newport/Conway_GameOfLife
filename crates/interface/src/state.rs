//! Private user-interface state of the shell (open windows, panel toggles,
//! dialog contents).

use bevy::prelude::*;
use bevy_egui::egui::Color32;

use crate::commands::RightPanelTab;
use crate::mode::{AppMode, ModePanelRegistry};
use crate::saves::{LoadDialog, SaveDialog};
use crate::theme::ThemeKind;
use crate::tutorial::TutorialView;
use crate::widgets::metrics::MetricsPanelState;

/// Shell UI state.
#[derive(Resource)]
pub(crate) struct ShellState {
    /// Side panels and bottom bar visible (Tab toggles).
    pub panels_visible: bool,
    /// Left panel expanded.
    pub left_open: bool,
    /// Right panel expanded.
    pub right_open: bool,
    /// Selected right-panel tab.
    pub right_tab: RightPanelTab,
    /// Tick count of the "Step N" control and Shift+N.
    pub step_many: u64,
    /// Seed used by the Randomize control.
    pub randomize_seed: u64,
    /// Help window open.
    pub show_help: bool,
    /// About / credits window open.
    pub show_about: bool,
    /// Tutorial window state.
    pub tutorial: TutorialView,
    /// Open save dialog.
    pub save_dialog: Option<SaveDialog>,
    /// Open load dialog.
    pub load_dialog: Option<LoadDialog>,
    /// Frames to capture in a PNG sequence export.
    pub sequence_frames: u32,
    /// Metric plot selection.
    pub metrics: MetricsPanelState,
    /// Theme and accent the egui style was last built for.
    pub applied_style: Option<(ThemeKind, Color32)>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            panels_visible: true,
            left_open: true,
            right_open: true,
            right_tab: RightPanelTab::Parameters,
            step_many: 10,
            randomize_seed: 1,
            show_help: false,
            show_about: false,
            tutorial: TutorialView::default(),
            save_dialog: None,
            load_dialog: None,
            sequence_frames: 120,
            metrics: MetricsPanelState::default(),
            applied_style: None,
        }
    }
}

/// Resets per-mode UI state when the mode changes.
pub(crate) fn reset_mode_ui(state: Res<State<AppMode>>, panels: Res<ModePanelRegistry>, mut shell: ResMut<ShellState>) {
    let has_properties = panels.get(*state.get()).right.is_some();
    shell.right_tab = if has_properties { RightPanelTab::Properties } else { RightPanelTab::Parameters };
    shell.metrics = MetricsPanelState::default();
    shell.save_dialog = None;
    shell.load_dialog = None;
    shell.tutorial = TutorialView::default();
}
