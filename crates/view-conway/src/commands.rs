//! Mode commands: actions requested by panels, windows, shortcuts and the
//! tutorial that talk to the simulation (selection operations, paste,
//! Hashlife jumps, engine and tracking changes...). They are queued in
//! [`ModeCommands`] and executed in one place, [`run_mode_commands`].

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::EguiClipboard;
use cellular_automata::{CellRect, EngineChoice, LifeCommand, LifeSim, MirrorAxis, Rotation};
use interface::{SessionStatus, Toasts};
use patterns::Pattern;
use rendering::grid::VisibleCells;
use simulation_core::{Control, ParamValue, Simulation};

use crate::analysis::{Analysis, MAX_ANALYSIS_POPULATION};
use crate::geometry::{jump_plan, moved_rect, rotated_selection};
use crate::labels::{MAX_TRANSFER_CELLS, thousands};
use crate::library::normalized_keeping_metadata;
use crate::session::{ConwayRunner, LiveInfo, LiveWorld, RandomizeDensity, lock};
use crate::style::{RenderStyle, Tracking, ViewSettings};
use crate::tools::{Clipboard, Selection, Stamp, StampPattern, StampSource, Tool, ToolSettings};
use crate::window::{MAX_EXPORT_CELLS, PatternsTab, PatternsWindow};
use crate::world::{CameraFit, view_centre};

/// An operation on the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionOp {
    /// Copy into the clipboard (and the system clipboard as RLE).
    Copy,
    /// Copy, then kill the cells.
    Cut,
    /// Kill the cells.
    Clear,
    /// Rotate a quarter turn in place (top-left corner fixed).
    Rotate {
        /// Clockwise.
        cw: bool,
    },
    /// Mirror left-right.
    FlipHorizontal,
    /// Mirror top-bottom.
    FlipVertical,
    /// Replace by a random soup.
    Fill,
    /// Move the cells by `(dx, dy)`.
    Move(i64, i64),
}

/// A queued mode command.
#[derive(Clone, Debug)]
pub enum ModeCommand {
    /// Operate on the selection.
    Selection(SelectionOp),
    /// Random soup in a rectangle.
    FillRegion(CellRect),
    /// Stamp the system clipboard's pattern (RLE etc.) or the internal clipboard.
    Paste,
    /// Select the bounding box of the live cells.
    SelectPattern,
    /// Change the render style (asks the model for the channel it needs).
    SetStyle(RenderStyle),
    /// Turn a tracking channel on.
    EnableTracking(Tracking),
    /// Generations advanced per tick.
    SetGenerationsPerTick(u64),
    /// Engine choice.
    SetEngine(EngineChoice),
    /// Jump by `2^k` generations with the configured `k`.
    JumpRequested,
    /// Jump by `2^k` generations.
    Jump(u32),
    /// Make a pattern the stamp and select the stamp tool.
    UseStamp(StampPattern),
    /// Replace every cell by a pattern (generation 0, undoable).
    ReplaceWorld(Arc<Pattern>),
    /// Paste a pattern at the centre of the view.
    PlaceAtCentre(Arc<Pattern>),
    /// Set the rule back to Conway's B3/S23.
    RestoreConway,
    /// Open the patterns window on a tab.
    OpenPatterns(PatternsTab),
    /// Start the period analysis of the world (or the selection).
    Analyse,
}

/// Commands waiting for execution.
#[derive(Resource, Default, Debug)]
pub struct ModeCommands(pub Vec<ModeCommand>);

/// Time-control settings.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct TimeSettings {
    /// `k` of the next jump of 2^k generations.
    pub jump_log2: u32,
}

impl Default for TimeSettings {
    fn default() -> Self {
        Self { jump_log2: 10 }
    }
}

/// A jump being prepared: the preparation's outcome, `k`, and the
/// generations per tick to restore afterwards.
type PendingJump = (Receiver<Result<(), String>>, u32, u64);

/// A Hashlife jump being prepared on the simulation thread.
#[derive(Resource, Default)]
pub struct JumpState {
    pending: Mutex<Option<PendingJump>>,
}

impl JumpState {
    /// Whether a jump is being prepared.
    pub fn busy(&self) -> bool {
        lock(&self.pending).is_some()
    }

    /// Drops a jump being prepared (its world was replaced).
    pub fn forget(&mut self) {
        *lock(&self.pending) = None;
    }
}

/// Why a jump cannot run now, or `None`.
pub fn jump_unavailable(info: Option<&LiveInfo>, status: &SessionStatus) -> Option<String> {
    let Some(info) = info else { return Some("Waiting for the simulation.".into()) };
    if info.world.is_finite() {
        return Some(
            "Hashlife runs only on the infinite plane; set Topology to Infinite plane (Parameters tab, applies on \
             reset)."
                .into(),
        );
    }
    let next_topology = status.get().and_then(|s| s.params.get("topology")).and_then(|v| v.as_str().map(String::from));
    if next_topology.as_deref().is_some_and(|t| t != "infinite") {
        return Some("The next reset changes the topology to a finite world; reset first or undo that change.".into());
    }
    if !info.generations_per_tick.is_power_of_two() {
        return Some(format!(
            "Hashlife needs a power-of-two number of generations per tick (now {}).",
            info.generations_per_tick
        ));
    }
    if !info.is_conway() {
        return Some("Restore the rule B3/S23 first.".into());
    }
    if info.halt.is_some() {
        return Some("The simulation has stopped; reset it first.".into());
    }
    None
}

/// Why an operation that lists the selected cells one by one is refused for
/// `rect` holding `known` live cells (`None` = not counted yet), or `None`.
/// A selection whose area is within `limit` is always accepted.
pub fn too_many_selected(rect: CellRect, known: Option<u64>, limit: u64) -> Option<String> {
    if rect.area() <= u128::from(limit) {
        return None;
    }
    match known {
        Some(n) if n <= limit => None,
        Some(n) => Some(format!(
            "The selection holds {} live cells, more than the {} this operation copies one by one.",
            thousands(u128::from(n)),
            thousands(u128::from(limit))
        )),
        None => Some("Still counting the selection's live cells; try again in a moment.".into()),
    }
}

/// Largest random fill on the infinite plane (the model's limit for edits,
/// [`cellular_automata::geometry::MAX_EDIT_CELLS`]); finite worlds are
/// clipped to their area, which the model fills whole.
pub const MAX_PLANE_FILL_CELLS: u64 = cellular_automata::geometry::MAX_EDIT_CELLS;

/// Why a random fill of `rect` is refused, or `None`.
pub fn fill_refused(rect: CellRect, world: cellular_automata::World) -> Option<String> {
    if world.is_finite() || rect.area() <= u128::from(MAX_PLANE_FILL_CELLS) {
        return None;
    }
    Some(format!(
        "Random fills on the infinite plane are limited to {} cells (8,192 × 8,192); this region has {}. Zoom in or \
         drag a smaller region.",
        thousands(u128::from(MAX_PLANE_FILL_CELLS)),
        thousands(rect.area())
    ))
}

/// Prepares a jump on the simulation thread: tracking off (it would force
/// generation-by-generation stepping) and the engine fixed to Hashlife (the
/// jump is a Hashlife jump whatever its size).
pub(crate) fn prepare_jump(sim: &mut LifeSim) -> Result<(), String> {
    for key in ["track_ages", "track_activity"] {
        if sim.params().get(key).and_then(ParamValue::as_bool) == Some(true) {
            sim.set_param(key, ParamValue::Bool(false)).map_err(|e| e.to_string())?;
        }
    }
    if sim.engine_choice() != EngineChoice::Hashlife {
        sim.set_param("engine", ParamValue::Text("hashlife".into())).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Everything the executor needs.
#[derive(SystemParam)]
pub struct CommandContext<'w> {
    runner: Option<Res<'w, ConwayRunner>>,
    live: Res<'w, LiveWorld>,
    status: Res<'w, SessionStatus>,
    tools: ResMut<'w, ToolSettings>,
    stamp: ResMut<'w, Stamp>,
    selection: ResMut<'w, Selection>,
    clipboard: ResMut<'w, Clipboard>,
    settings: ResMut<'w, ViewSettings>,
    time: Res<'w, TimeSettings>,
    jump: ResMut<'w, JumpState>,
    window: ResMut<'w, PatternsWindow>,
    toasts: ResMut<'w, Toasts>,
    visible: Res<'w, VisibleCells>,
    density: Res<'w, RandomizeDensity>,
    fit: ResMut<'w, CameraFit>,
    analysis: ResMut<'w, Analysis>,
    egui_clipboard: Option<ResMut<'w, EguiClipboard>>,
}

impl CommandContext<'_> {
    fn param_bool(&self, key: &str) -> bool {
        self.status.get().and_then(|s| s.params.get(key)).and_then(ParamValue::as_bool).unwrap_or(false)
    }

    fn run(&mut self, command: ModeCommand) {
        match command {
            ModeCommand::OpenPatterns(tab) => self.window.open(tab),
            ModeCommand::UseStamp(stamp) => {
                self.stamp.0 = Some(stamp);
                self.tools.tool = Tool::Stamp;
            }
            ModeCommand::SetStyle(style) => self.set_style(style),
            command => {
                let Some(runner) = self.runner.as_deref().cloned() else { return };
                self.run_with(&runner, command);
            }
        }
    }

    fn set_style(&mut self, style: RenderStyle) {
        self.settings.style = style;
        if let Some(tracking) = style.tracking()
            && !self.param_bool(tracking.param())
        {
            self.enable_tracking(tracking);
        }
    }

    fn enable_tracking(&mut self, tracking: Tracking) {
        let Some(runner) = self.runner.as_deref() else { return };
        let info = self.live.current();
        let population = info.map_or(0, |i| i.population);
        if population > MAX_TRANSFER_CELLS {
            self.toasts.warning(format!(
                "Not enabling {}: it records every one of the {} live cells each generation.",
                tracking.label(),
                thousands(population)
            ));
            return;
        }
        runner.set_param(tracking.param(), ParamValue::Bool(true));
        let hashlife = info.is_some_and(|i| i.engine == cellular_automata::EngineKind::Hashlife);
        let note = if hashlife { " Hashlife now steps one generation at a time." } else { "" };
        self.toasts.info(format!("Turned on {} for this style.{note}", tracking.label()));
    }

    fn run_with(&mut self, runner: &ConwayRunner, command: ModeCommand) {
        match command {
            ModeCommand::Selection(op) => self.selection_op(runner, op),
            ModeCommand::FillRegion(rect) => self.fill(runner, rect),
            ModeCommand::Paste => self.paste(),
            ModeCommand::SelectPattern => match self.live.current().and_then(|i| i.bbox) {
                Some(bbox) => {
                    self.selection.set(bbox);
                    self.tools.tool = Tool::Select;
                }
                None => self.toasts.info("The world is empty: nothing to select."),
            },
            ModeCommand::EnableTracking(t) => self.enable_tracking(t),
            ModeCommand::SetGenerationsPerTick(n) => {
                runner.set_param("generations_per_tick", ParamValue::Int(n.min(i64::MAX as u64) as i64))
            }
            ModeCommand::SetEngine(choice) => runner.set_param("engine", ParamValue::Text(choice.as_str().into())),
            ModeCommand::JumpRequested => {
                let k = self.time.jump_log2;
                self.run_with(runner, ModeCommand::Jump(k));
            }
            ModeCommand::Jump(k) => self.jump(runner, k),
            ModeCommand::ReplaceWorld(pattern) => {
                let world = self.live.current().map(|i| i.world);
                let (x, y) = match world {
                    Some(w) if w.is_finite() => (
                        (i64::from(w.width) - pattern.width() as i64) / 2,
                        (i64::from(w.height) - pattern.height() as i64) / 2,
                    ),
                    _ => (0, 0),
                };
                runner.edit(LifeCommand::LoadPattern { pattern: (*pattern).clone(), x, y, use_pattern_rule: false });
                self.selection.clear();
                self.fit.request_fresh(runner);
            }
            ModeCommand::PlaceAtCentre(pattern) => {
                runner.edit(self.tools.paste_at(&pattern, view_centre(&self.visible)));
            }
            ModeCommand::RestoreConway => runner.set_param("preset", ParamValue::Text("conway".into())),
            ModeCommand::Analyse => {
                let rect = self.selection.rect;
                let Some(info) = self.live.current() else { return };
                let infinite = !info.world.is_finite();
                // Known too large: report it without copying the cells.
                let population = match rect {
                    Some(_) => self.selection.current_info().map(|i| u128::from(i.population)),
                    None => Some(info.population),
                };
                let limit = u128::from(MAX_ANALYSIS_POPULATION);
                match population {
                    Some(p) if p > limit => {
                        let exact = rect.is_none() && !infinite;
                        self.analysis.report_too_large(info.generation, rect, exact, p);
                    }
                    // Not counted yet and possibly huge.
                    None if rect.is_some_and(|r| r.area() > limit) => {
                        self.toasts.warning("Still counting the selection's live cells; try again in a moment.")
                    }
                    _ => self.analysis.start(runner, rect, infinite),
                }
            }
            ModeCommand::OpenPatterns(_) | ModeCommand::UseStamp(_) | ModeCommand::SetStyle(_) => {}
        }
    }

    fn fill(&mut self, runner: &ConwayRunner, rect: CellRect) {
        let world = self.live.current().map(|i| i.world);
        if let Some(reason) = world.and_then(|w| fill_refused(rect, w)) {
            self.toasts.warning(reason);
            return;
        }
        let seed = self.tools.fill_seed;
        self.tools.fill_seed = self.tools.fill_seed.wrapping_add(1);
        self.density.set(self.tools.fill_density);
        runner.edit(LifeCommand::Randomize {
            density: self.tools.fill_density,
            seed: Some(seed),
            region: Some(rect),
            symmetry: self.tools.fill_symmetry,
        });
    }

    fn selection_op(&mut self, runner: &ConwayRunner, op: SelectionOp) {
        let Some(rect) = self.selection.rect else {
            if matches!(op, SelectionOp::Copy | SelectionOp::Cut | SelectionOp::Clear) {
                self.toasts.info("Select a region first (Select tool, key 6).");
            }
            return;
        };
        // Copies become RLE text for the system clipboard (at most
        // MAX_EXPORT_CELLS cells); the model refuses moves, rotations and
        // mirrors of more than its edit limit. Refusing here, before sending,
        // keeps the selection (moved or rotated right after sending) on its
        // cells and a cut from clearing cells it could not copy.
        let limit = match op {
            SelectionOp::Copy | SelectionOp::Cut => Some(MAX_EXPORT_CELLS),
            SelectionOp::Rotate { .. }
            | SelectionOp::FlipHorizontal
            | SelectionOp::FlipVertical
            | SelectionOp::Move(..) => Some(cellular_automata::geometry::MAX_EDIT_CELLS),
            SelectionOp::Clear | SelectionOp::Fill => None,
        };
        let known = self.selection.current_info().map(|i| i.population);
        if let Some(reason) = limit.and_then(|limit| too_many_selected(rect, known, limit)) {
            self.toasts.warning(reason);
            return;
        }
        match op {
            SelectionOp::Copy | SelectionOp::Cut => {
                *lock(&self.clipboard.pending) = Some(runner.request(move |sim| sim.copy_region(rect)));
                if op == SelectionOp::Cut {
                    runner.edit(LifeCommand::ClearRegion { rect });
                }
            }
            SelectionOp::Clear => runner.edit(LifeCommand::ClearRegion { rect }),
            SelectionOp::Rotate { cw } => {
                let rotation = if cw { Rotation::Cw90 } else { Rotation::Ccw90 };
                runner.edit(LifeCommand::RotateRegion { rect, rotation });
                if let Some(r) = rotated_selection(rect, rotation) {
                    self.selection.set(r);
                }
            }
            SelectionOp::FlipHorizontal => {
                runner.edit(LifeCommand::MirrorRegion { rect, axis: MirrorAxis::Horizontal })
            }
            SelectionOp::FlipVertical => runner.edit(LifeCommand::MirrorRegion { rect, axis: MirrorAxis::Vertical }),
            SelectionOp::Fill => self.fill(runner, rect),
            SelectionOp::Move(dx, dy) => match moved_rect(rect, dx, dy) {
                Some(dest) => {
                    runner.edit(LifeCommand::MoveRegion { rect, dx, dy });
                    self.selection.set(dest);
                }
                None => self.toasts.error("The move would leave the 64-bit coordinate plane."),
            },
        }
    }

    fn paste(&mut self) {
        let text = self.egui_clipboard.as_mut().and_then(|c| c.get_text()).filter(|t| !t.trim().is_empty());
        if let Some(text) = text
            && let Ok(pattern) = patterns::parse_any(&text, None)
            && !pattern.is_empty()
        {
            let name = if pattern.name.is_empty() { "Pasted pattern".to_string() } else { pattern.name.clone() };
            self.stamp.0 = Some(StampPattern {
                name,
                source: StampSource::Imported,
                pattern: Arc::new(normalized_keeping_metadata(&pattern)),
            });
            self.tools.tool = Tool::Stamp;
            return;
        }
        match self.clipboard.pattern.clone() {
            Some(pattern) => {
                self.stamp.0 = Some(StampPattern { name: "Clipboard".into(), source: StampSource::Clipboard, pattern });
                self.tools.tool = Tool::Stamp;
            }
            None => self.toasts.info("The clipboard holds no pattern. Copy a selection or RLE text first."),
        }
    }

    fn jump(&mut self, runner: &ConwayRunner, k: u32) {
        if let Some(reason) = jump_unavailable(self.live.current(), &self.status) {
            self.toasts.warning(format!("Cannot jump: {reason}"));
            return;
        }
        if self.jump.busy() {
            return;
        }
        let previous = self.live.current().map_or(1, |i| i.generations_per_tick);
        // Paused, so no ordinary tick runs between the steps below; the
        // empty undoable edit records the state before the jump.
        runner.send(Control::Pause);
        runner.send(Control::ExecEdit(Box::new(|_| {})));
        *lock(&self.jump.pending) = Some((runner.request(prepare_jump), k, previous));
    }
}

/// Executes the queued commands.
pub(crate) fn run_mode_commands(mut queue: ResMut<ModeCommands>, mut ctx: CommandContext) {
    let commands = std::mem::take(&mut queue.0);
    for command in commands {
        ctx.run(command);
    }
}

/// Finishes jumps whose preparation succeeded, and copies.
pub(crate) fn poll_async_commands(
    runner: Option<Res<ConwayRunner>>,
    jump: Res<JumpState>,
    mut fit: ResMut<CameraFit>,
    mut clipboard: ResMut<Clipboard>,
    mut toasts: ResMut<Toasts>,
    mut egui_clipboard: Option<ResMut<EguiClipboard>>,
) {
    let prepared = lock(&jump.pending).as_ref().map(|(rx, k, previous)| (rx.try_recv(), *k, *previous));
    match prepared {
        Some((Ok(Ok(())), k, previous)) => {
            *lock(&jump.pending) = None;
            if let Some(runner) = &runner {
                let (per_tick, ticks) = jump_plan(k);
                runner.send(Control::Exec(Box::new(move |sim: &mut LifeSim| {
                    if let Err(e) = sim.set_param("generations_per_tick", ParamValue::Int(per_tick as i64)) {
                        warn!("jump: {e}");
                    }
                })));
                runner.send(Control::Step(ticks));
                runner.send(Control::Exec(Box::new(move |sim: &mut LifeSim| {
                    if let Err(e) = sim.set_param("generations_per_tick", ParamValue::Int(previous as i64)) {
                        warn!("jump: could not restore the generations per tick: {e}");
                    }
                })));
                fit.request_fresh(runner);
                toasts.info(format!(
                    "Jumping 2^{k} = {} generations with Hashlife (paused; undo returns to the start).",
                    thousands(1u128 << k)
                ));
            }
        }
        Some((Ok(Err(e)), ..)) => {
            *lock(&jump.pending) = None;
            toasts.error(format!("Cannot jump: {e}"));
        }
        Some((Err(TryRecvError::Disconnected), ..)) => *lock(&jump.pending) = None,
        Some((Err(TryRecvError::Empty), ..)) | None => {}
    }

    let copied = lock(&clipboard.pending).as_ref().map(Receiver::try_recv);
    match copied {
        Some(Ok(Ok(pattern))) => {
            *lock(&clipboard.pending) = None;
            let population = pattern.population();
            let normalized = normalized_keeping_metadata(&pattern);
            if let Some(system) = egui_clipboard.as_mut() {
                system.set_text(&patterns::rle::write(&normalized));
            }
            let size = format!("{} × {}", normalized.width(), normalized.height());
            clipboard.pattern = Some(Arc::new(normalized));
            toasts.success(format!(
                "Copied {} live cells ({size}) as RLE. Ctrl/Cmd+V pastes them with the stamp tool.",
                thousands(population as u128)
            ));
        }
        Some(Ok(Err(e))) => {
            *lock(&clipboard.pending) = None;
            toasts.error(format!("Cannot copy: {e}"));
        }
        Some(Err(TryRecvError::Disconnected)) => *lock(&clipboard.pending) = None,
        Some(Err(TryRecvError::Empty)) | None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellular_automata::{LifeConfig, LifeMode, PatternLibrary};
    use simulation_core::Topology;

    fn plane() -> LifeSim {
        let config = LifeConfig { topology: Topology::Infinite, init: vec![], ..LifeConfig::default() };
        LifeSim::new(&config, 1, &PatternLibrary::builtin(), LifeMode::Classic).unwrap()
    }

    #[test]
    fn jumps_fix_hashlife_and_stop_tracking() {
        let mut sim = plane();
        sim.set_param("track_ages", ParamValue::Bool(true)).unwrap();
        prepare_jump(&mut sim).unwrap();
        assert_eq!(sim.engine_choice(), EngineChoice::Hashlife);
        assert_eq!(sim.params().get("track_ages").and_then(ParamValue::as_bool), Some(false));
        // The jump itself: one tick of 2^p generations, `ticks` times.
        let glider = Pattern::from_ascii(".O.\n..O\nOOO").unwrap();
        sim.apply(LifeCommand::Paste {
            pattern: glider,
            x: 0,
            y: 0,
            rotation: Rotation::None,
            mirror: false,
            mode: cellular_automata::PasteMode::Or,
        })
        .unwrap();
        let (per_tick, ticks) = jump_plan(32);
        sim.set_param("generations_per_tick", ParamValue::Int(per_tick as i64)).unwrap();
        for _ in 0..ticks {
            sim.step();
        }
        assert_eq!(sim.tick(), 1 << 32);
        assert_eq!(sim.population(), 5, "the glider is intact after 2^32 generations");
        let bbox = sim.engine().bounding_box().unwrap();
        assert_eq!((bbox.x0, bbox.y0), (1 << 30, 1 << 30), "it moved (1, 1) every 4 generations");
    }

    #[test]
    fn huge_selections_are_not_listed() {
        let small = CellRect::new(0, 0, 99, 99);
        assert!(too_many_selected(small, None, 10_000).is_none(), "the area bounds the population");
        let large = CellRect::new(0, 0, 1 << 20, 1 << 20);
        assert!(too_many_selected(large, Some(5), 10_000).is_none());
        assert!(too_many_selected(large, Some(20_000), 10_000).unwrap().contains("20,000 live cells"));
        assert!(too_many_selected(large, None, 10_000).unwrap().contains("counting"));
    }

    #[test]
    fn huge_fills_of_the_plane_are_refused() {
        let plane = cellular_automata::World::new(Topology::Infinite, 256, 256).unwrap();
        let torus = cellular_automata::World::new(Topology::Torus, 4096, 4096).unwrap();
        assert!(fill_refused(CellRect::new(0, 0, 8191, 8191), plane).is_none());
        let huge = CellRect::new(0, 0, 8192, 8191);
        assert!(fill_refused(huge, plane).unwrap().contains("67,117,056"));
        assert!(fill_refused(CellRect::new(-100_000, -100_000, 100_000, 100_000), torus).is_none(), "clipped");
    }

    #[test]
    fn finite_worlds_refuse_hashlife() {
        let mut sim = LifeSim::conway(1);
        assert!(prepare_jump(&mut sim).is_err());
    }
}
