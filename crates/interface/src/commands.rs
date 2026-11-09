//! Shell commands: one message type for everything the top bar, bottom bar
//! and keyboard shortcuts can ask for, executed in one place.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use simulation_core::{RunnerStatus, SessionControl, Speed};

use crate::mode::{ActiveSession, AppMode, ModeRegistry};
use crate::saves::{DataLocation, LoadDialog, SaveDialog};
use crate::session::SessionStatus;
use crate::state::ShellState;
use crate::toasts::Toasts;
use crate::tutorial::{TutorialRegistry, TutorialView};

/// A request from the user interface. Written by buttons and shortcuts,
/// executed by the shell in `Update`.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ShellCommand {
    /// Switch to a mode (or the dashboard).
    OpenMode(AppMode),
    /// Toggle play / pause.
    TogglePlay,
    /// Run this many ticks now.
    Step(u64),
    /// Reset to the scenario's initial state.
    Reset,
    /// Clear the world (models that support it).
    Clear,
    /// Randomise the world with a seed (models that support it).
    Randomize(u64),
    /// Undo the last edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
    /// Set the target speed.
    SetSpeed(Speed),
    /// Next faster speed on the ladder.
    Faster,
    /// Next slower speed on the ladder.
    Slower,
    /// Restore the n-th stored history snapshot.
    JumpToHistory(usize),
    /// Show / hide the side panels and the bottom bar.
    TogglePanels,
    /// Toggle borderless fullscreen.
    ToggleFullscreen,
    /// Save a PNG screenshot of the window.
    Screenshot,
    /// Show the right panel with this tab selected.
    ShowRightTab(RightPanelTab),
    /// Open one of the shell's windows.
    OpenWindow(ShellWindow),
}

/// Tabs of the right panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RightPanelTab {
    /// The mode's own panel ([`crate::mode::PanelSlot::Right`]); falls back
    /// to Parameters when the mode has none.
    Properties,
    /// Parameter editor generated from the model's specs.
    Parameters,
    /// Formal rules of the model.
    Rules,
    /// Live metric plots.
    Metrics,
}

impl RightPanelTab {
    /// Tab label.
    pub fn label(self) -> &'static str {
        match self {
            RightPanelTab::Properties => "Properties",
            RightPanelTab::Parameters => "Parameters",
            RightPanelTab::Rules => "Rules",
            RightPanelTab::Metrics => "Metrics",
        }
    }
}

/// Windows of the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShellWindow {
    /// Shortcuts and controls.
    Help,
    /// Version, licences and credits.
    About,
    /// Tutorial of the current mode.
    Tutorial,
    /// Save dialog (needs a running session).
    Save,
    /// Load dialog (needs a running session).
    Load,
}

/// Speeds visited by the faster / slower shortcuts (ticks per second);
/// beyond the last one comes [`Speed::Unlimited`].
pub const SPEED_LADDER: [f64; 13] =
    [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0];

/// Slowest selectable speed (ticks per second).
pub const MIN_TICKS_PER_SECOND: f64 = SPEED_LADDER[0];
/// Fastest finite selectable speed (ticks per second).
pub const MAX_TICKS_PER_SECOND: f64 = SPEED_LADDER[SPEED_LADDER.len() - 1];

/// The next faster speed on [`SPEED_LADDER`] (then unlimited).
pub fn faster(speed: Speed) -> Speed {
    match speed {
        Speed::Unlimited => Speed::Unlimited,
        Speed::TicksPerSecond(rate) => SPEED_LADDER
            .iter()
            .copied()
            .find(|&v| v > rate * (1.0 + 1e-9))
            .map_or(Speed::Unlimited, Speed::TicksPerSecond),
    }
}

/// The next slower speed on [`SPEED_LADDER`] (never below the first rung).
pub fn slower(speed: Speed) -> Speed {
    match speed {
        Speed::Unlimited => Speed::TicksPerSecond(MAX_TICKS_PER_SECOND),
        Speed::TicksPerSecond(rate) => Speed::TicksPerSecond(
            SPEED_LADDER.iter().rev().copied().find(|&v| v < rate * (1.0 - 1e-9)).unwrap_or(MIN_TICKS_PER_SECOND),
        ),
    }
}

/// Outcome of a session command that the user should hear about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandNotice {
    /// The command was not available; the text says why.
    Unavailable(String),
}

/// Sends a session-related command to `session`, checking availability
/// against `status` (undo/redo stacks, clear/randomize support). Returns a
/// notice when the command could not be executed, or `None` for commands
/// that are not session commands or were executed.
pub fn apply_session_command(
    command: &ShellCommand,
    session: &dyn SessionControl,
    status: &RunnerStatus,
) -> Option<CommandNotice> {
    let unavailable = |why: &str| Some(CommandNotice::Unavailable(why.to_string()));
    match command {
        ShellCommand::TogglePlay => {
            if status.finished && !status.playing {
                return unavailable("the scenario has finished; reset to run it again");
            }
            session.toggle_play();
        }
        ShellCommand::Step(n) => {
            if status.finished {
                return unavailable("the scenario has finished; reset to run it again");
            }
            session.step((*n).max(1));
        }
        ShellCommand::Reset => session.reset(),
        ShellCommand::Clear => {
            if !session.can_clear() {
                return unavailable("this model cannot be cleared");
            }
            session.clear();
        }
        ShellCommand::Randomize(seed) => {
            if !session.can_randomize() {
                return unavailable("this model cannot be randomised");
            }
            session.randomize(*seed);
        }
        ShellCommand::Undo => {
            if !status.can_undo {
                return unavailable("nothing to undo");
            }
            session.undo();
        }
        ShellCommand::Redo => {
            if !status.can_redo {
                return unavailable("nothing to redo");
            }
            session.redo();
        }
        ShellCommand::SetSpeed(speed) => session.set_speed(sanitize_speed(*speed)),
        ShellCommand::Faster => session.set_speed(faster(status.speed)),
        ShellCommand::Slower => session.set_speed(slower(status.speed)),
        ShellCommand::JumpToHistory(index) => {
            if *index >= status.history_ticks.len() {
                return unavailable("that history snapshot is no longer stored");
            }
            session.jump_to_history(*index);
        }
        ShellCommand::OpenMode(_)
        | ShellCommand::TogglePanels
        | ShellCommand::ToggleFullscreen
        | ShellCommand::Screenshot
        | ShellCommand::ShowRightTab(_)
        | ShellCommand::OpenWindow(_) => {}
    }
    None
}

/// Clamps a finite speed to the selectable range.
pub fn sanitize_speed(speed: Speed) -> Speed {
    match speed {
        Speed::Unlimited => Speed::Unlimited,
        Speed::TicksPerSecond(rate) if rate.is_finite() => {
            Speed::TicksPerSecond(rate.clamp(MIN_TICKS_PER_SECOND, MAX_TICKS_PER_SECOND))
        }
        Speed::TicksPerSecond(_) => Speed::TicksPerSecond(MIN_TICKS_PER_SECOND),
    }
}

/// Executes [`ShellCommand`]s (screenshots are handled by the export module).
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_shell_commands(
    mut commands: MessageReader<ShellCommand>,
    session: Option<Res<ActiveSession>>,
    status: Res<SessionStatus>,
    registry: Res<ModeRegistry>,
    state: Res<State<AppMode>>,
    mut next_mode: ResMut<NextState<AppMode>>,
    mut shell: ResMut<ShellState>,
    mut toasts: ResMut<Toasts>,
    mut window: Option<Single<&mut Window, With<PrimaryWindow>>>,
    tutorials: Res<TutorialRegistry>,
    location: Res<DataLocation>,
) {
    for command in commands.read() {
        match command {
            ShellCommand::OpenMode(mode) => {
                if !registry.is_registered(*mode) {
                    toasts.warning(format!(
                        "{} is still in development and cannot be opened yet.",
                        registry.title(*mode)
                    ));
                } else if *mode != *state.get() {
                    next_mode.set(*mode);
                }
            }
            ShellCommand::TogglePanels => shell.panels_visible = !shell.panels_visible,
            ShellCommand::ToggleFullscreen => {
                if let Some(window) = window.as_deref_mut() {
                    window.mode = match window.mode {
                        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
                        _ => WindowMode::Windowed,
                    };
                }
            }
            ShellCommand::Screenshot => {}
            ShellCommand::ShowRightTab(tab) => {
                shell.panels_visible = true;
                shell.right_open = true;
                shell.right_tab = *tab;
            }
            ShellCommand::OpenWindow(which) => {
                let mode = *state.get();
                let has_session = session.is_some() && mode.is_simulation();
                match which {
                    ShellWindow::Help => shell.show_help = true,
                    ShellWindow::About => shell.show_about = true,
                    ShellWindow::Tutorial if mode == AppMode::Dashboard || tutorials.get(mode).is_some() => {
                        shell.tutorial = TutorialView { open: true, ..default() };
                    }
                    ShellWindow::Tutorial => toasts.info("No tutorial is registered for this mode yet."),
                    ShellWindow::Save if has_session => {
                        let tick = status.get().map_or(0, |s| s.tick);
                        shell.save_dialog = Some(SaveDialog::new(&registry.title(mode), tick));
                    }
                    ShellWindow::Load if has_session => shell.load_dialog = Some(LoadDialog::open(&location, mode)),
                    ShellWindow::Save | ShellWindow::Load => toasts.info("Open a running laboratory first."),
                }
            }
            session_command => {
                let (Some(session), Some(status)) = (session.as_deref(), status.get()) else {
                    if state.get().is_simulation() {
                        toasts.info("The simulation is not running yet.");
                    }
                    continue;
                };
                if let Some(CommandNotice::Unavailable(why)) =
                    apply_session_command(session_command, session.session(), status)
                {
                    toasts.info(format!("Not available: {why}."));
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Receiver};

    use simulation_core::{HistoryConfig, Metrics, ModelDescriptor, ModelFamily, ParamValue, SimError, Snapshot};

    use super::*;

    /// Session double recording every call.
    pub(crate) struct FakeSession {
        pub descriptor: ModelDescriptor,
        pub calls: Mutex<Vec<String>>,
        pub status: Mutex<RunnerStatus>,
        pub clear: bool,
        pub randomize: bool,
        pub metrics: Mutex<Vec<Metrics>>,
        pub errors: Mutex<Vec<String>>,
    }

    impl FakeSession {
        pub(crate) fn new() -> Self {
            Self {
                descriptor: ModelDescriptor::new("test.fake", "Fake", ModelFamily::Cellular),
                calls: Mutex::new(Vec::new()),
                status: Mutex::new(RunnerStatus::default()),
                clear: false,
                randomize: true,
                metrics: Mutex::new(Vec::new()),
                errors: Mutex::new(Vec::new()),
            }
        }

        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn log(&self, call: impl Into<String>) {
            self.calls.lock().unwrap().push(call.into());
        }
    }

    impl SessionControl for FakeSession {
        fn descriptor(&self) -> &ModelDescriptor {
            &self.descriptor
        }
        fn status(&self) -> RunnerStatus {
            self.status.lock().unwrap().clone()
        }
        fn play(&self) {
            self.log("play");
        }
        fn pause(&self) {
            self.log("pause");
        }
        fn toggle_play(&self) {
            self.log("toggle_play");
        }
        fn step(&self, n: u64) {
            self.log(format!("step {n}"));
        }
        fn set_speed(&self, speed: Speed) {
            self.log(format!("speed {speed:?}"));
        }
        fn reset(&self) {
            self.log("reset");
        }
        fn undo(&self) {
            self.log("undo");
        }
        fn redo(&self) {
            self.log("redo");
        }
        fn jump_to_history(&self, index: usize) {
            self.log(format!("jump {index}"));
        }
        fn set_history(&self, _config: HistoryConfig) {
            self.log("set_history");
        }
        fn set_param(&self, key: &str, value: ParamValue) {
            self.log(format!("param {key}={value}"));
        }
        fn drain_metrics(&self) -> Vec<Metrics> {
            std::mem::take(&mut *self.metrics.lock().unwrap())
        }
        fn take_errors(&self) -> Vec<String> {
            std::mem::take(&mut *self.errors.lock().unwrap())
        }
        fn request_snapshot(&self) -> Receiver<Result<Snapshot, SimError>> {
            let (tx, rx) = mpsc::channel();
            let snapshot = Snapshot::encode("test.fake", 1, self.status().tick, &42u32);
            let _ = tx.send(snapshot);
            rx
        }
        fn restore(&self, snapshot: Snapshot) {
            self.log(format!("restore {}", snapshot.tick));
        }
        fn can_clear(&self) -> bool {
            self.clear
        }
        fn clear(&self) {
            self.log("clear");
        }
        fn can_randomize(&self) -> bool {
            self.randomize
        }
        fn randomize(&self, seed: u64) {
            self.log(format!("randomize {seed}"));
        }
    }

    #[test]
    fn speed_ladder_moves_both_ways() {
        assert_eq!(faster(Speed::TicksPerSecond(10.0)), Speed::TicksPerSecond(20.0));
        assert_eq!(faster(Speed::TicksPerSecond(12.0)), Speed::TicksPerSecond(20.0));
        assert_eq!(faster(Speed::TicksPerSecond(10_000.0)), Speed::Unlimited);
        assert_eq!(faster(Speed::Unlimited), Speed::Unlimited);
        assert_eq!(slower(Speed::Unlimited), Speed::TicksPerSecond(10_000.0));
        assert_eq!(slower(Speed::TicksPerSecond(12.0)), Speed::TicksPerSecond(10.0));
        assert_eq!(slower(Speed::TicksPerSecond(1.0)), Speed::TicksPerSecond(1.0));
        assert_eq!(sanitize_speed(Speed::TicksPerSecond(1e9)), Speed::TicksPerSecond(10_000.0));
        assert_eq!(sanitize_speed(Speed::TicksPerSecond(f64::NAN)), Speed::TicksPerSecond(1.0));
    }

    #[test]
    fn session_commands_respect_availability() {
        let session = FakeSession::new();
        let mut status = RunnerStatus { speed: Speed::TicksPerSecond(10.0), ..Default::default() };
        assert!(apply_session_command(&ShellCommand::Undo, &session, &status).is_some());
        assert!(apply_session_command(&ShellCommand::Clear, &session, &status).is_some());
        assert!(apply_session_command(&ShellCommand::JumpToHistory(0), &session, &status).is_some());
        status.can_undo = true;
        status.history_ticks = vec![0, 50];
        for command in [
            ShellCommand::TogglePlay,
            ShellCommand::Step(0),
            ShellCommand::Undo,
            ShellCommand::Randomize(7),
            ShellCommand::Faster,
            ShellCommand::JumpToHistory(1),
            ShellCommand::Reset,
        ] {
            assert_eq!(apply_session_command(&command, &session, &status), None, "{command:?}");
        }
        assert_eq!(
            session.calls(),
            vec!["toggle_play", "step 1", "undo", "randomize 7", "speed TicksPerSecond(20.0)", "jump 1", "reset"]
        );
        status.finished = true;
        assert!(apply_session_command(&ShellCommand::Step(3), &session, &status).is_some());
    }
}
