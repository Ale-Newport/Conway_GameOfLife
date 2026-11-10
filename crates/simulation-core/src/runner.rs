//! Worker-thread tick loop that decouples simulation from rendering.
//!
//! A [`Runner`] owns a [`Simulation`] on a dedicated thread. The user
//! interface sends [`Control`] messages (play, pause, step, edits...) and reads
//! the most recently published view, status, descriptor and metrics without
//! ever blocking on a long simulation step. When a model cannot keep up with
//! the requested speed the runner simply runs slower and reports the measured
//! rate; it never skips or alters ticks, so results do not depend on
//! rendering performance.
//!
//! After the controls that can change the state other than by ticking
//! (everything but play, pause, step, speed, queries, view and history
//! settings) the worker records a metric sample of the current tick and
//! publishes the descriptor again if it changed, so edits made while paused
//! and long closures show up at once. It rebuilds the descriptor once more
//! after the next tick, which is when a parameter with
//! [`ApplyPolicy::NextTick`] takes effect.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::SimError;
use crate::history::{HistoryConfig, SnapshotHistory, UndoStack};
use crate::metrics::Metrics;
use crate::model::ModelDescriptor;
use crate::params::{ApplyPolicy, ParamSet, ParamValue};
use crate::simulation::Simulation;
use crate::snapshot::Snapshot;

/// Target simulation speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    /// Run at most this many ticks per second.
    TicksPerSecond(f64),
    /// Run as fast as possible while staying responsive to controls.
    Unlimited,
}

impl Default for Speed {
    fn default() -> Self {
        Self::TicksPerSecond(10.0)
    }
}

/// Runner tuning.
#[derive(Clone, Debug)]
pub struct RunnerConfig {
    /// Maximum number of views published per second while playing.
    pub publish_hz: f64,
    /// Maximum time spent stepping before controls are processed again.
    pub max_batch: Duration,
    /// Record metrics every `metrics_every` ticks, and after every control
    /// that changes the state (0 disables recording).
    pub metrics_every: u64,
    /// Maximum number of buffered, not yet drained metric samples.
    pub metrics_capacity: usize,
    /// Periodic snapshot recording.
    pub history: HistoryConfig,
    /// Memory budget for undo/redo snapshots.
    pub undo_bytes: usize,
    /// Initial speed.
    pub speed: Speed,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            publish_hz: 60.0,
            max_batch: Duration::from_millis(12),
            metrics_every: 1,
            metrics_capacity: 100_000,
            history: HistoryConfig::default(),
            undo_bytes: 128 * 1024 * 1024,
            speed: Speed::default(),
        }
    }
}

/// Closure executed on the simulation thread.
pub type SimFn<S> = Box<dyn FnOnce(&mut S) + Send>;

/// Closure reading the simulation on its thread (see [`Control::Query`]).
pub type QueryFn<S> = Box<dyn FnOnce(&S) + Send>;

/// Reusable mode-specific action (e.g. "clear the world").
pub type SimHook<S> = Arc<dyn Fn(&mut S) + Send + Sync>;

/// Reusable mode-specific action taking a seed (e.g. "randomize").
pub type SeededHook<S> = Arc<dyn Fn(&mut S, u64) + Send + Sync>;

/// Builds the descriptor a runner publishes (see [`Runner::spawn_with_descriptor`]).
type DescribeFn<S> = Box<dyn Fn(&S) -> ModelDescriptor + Send>;

/// Messages accepted by a [`Runner`].
pub enum Control<S: Simulation> {
    /// Start running continuously.
    Play,
    /// Stop running.
    Pause,
    /// Toggle between playing and paused.
    TogglePlay,
    /// Run exactly this many ticks now (also while paused).
    Step(u64),
    /// Change the target speed.
    SetSpeed(Speed),
    /// Reset the model to its initial state (undoable).
    Reset,
    /// Apply a command that is not recorded for undo.
    Command(S::Command),
    /// Apply a command and record the previous state for undo.
    Edit(S::Command),
    /// Change one parameter.
    SetParam(String, ParamValue),
    /// Run an arbitrary closure on the simulation thread.
    Exec(SimFn<S>),
    /// Run a closure that only reads the simulation; unlike `Exec` it is
    /// not a state change (nothing is rebuilt, recorded or published).
    Query(QueryFn<S>),
    /// Run a closure and record the previous state for undo.
    ExecEdit(SimFn<S>),
    /// Undo the last edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
    /// Restore the n-th entry (0 = oldest) of the periodic history and pause.
    JumpToHistory(usize),
    /// Restore an external snapshot and pause.
    Restore(Snapshot),
    /// Change what the published view should contain (visible region...).
    SetViewRequest(S::ViewRequest),
    /// Change the periodic history configuration.
    SetHistory(HistoryConfig),
    /// Replace the whole simulation (e.g. a newly configured scenario).
    Replace(Box<S>),
    /// Publish a fresh view as soon as possible.
    RequestView,
    /// Stop the worker thread.
    Shutdown,
}

/// A view published by the worker thread.
pub struct Published<V> {
    /// Monotonic publication counter (changes whenever a new view is ready).
    pub seq: u64,
    /// Tick the view was built at.
    pub tick: u64,
    /// The view itself.
    pub view: Arc<V>,
}

impl<V> Clone for Published<V> {
    fn clone(&self) -> Self {
        Self { seq: self.seq, tick: self.tick, view: Arc::clone(&self.view) }
    }
}

/// Snapshot of the runner's state for display.
#[derive(Clone, Debug, Default)]
pub struct RunnerStatus {
    /// Whether the simulation is running continuously.
    pub playing: bool,
    /// Current tick (generation).
    pub tick: u64,
    /// Target speed.
    pub speed: Speed,
    /// Measured ticks per second.
    pub measured_tps: f64,
    /// Measured generations per second (differs from ticks for engines that
    /// advance several generations per tick, such as Hashlife).
    pub measured_gps: f64,
    /// Duration of the most recent tick in milliseconds.
    pub last_step_ms: f64,
    /// Exponential moving average of tick duration in milliseconds.
    pub avg_step_ms: f64,
    /// Ticks of the stored periodic snapshots (for timeline widgets).
    pub history_ticks: Vec<u64>,
    /// Memory used by stored snapshots.
    pub history_bytes: usize,
    /// Whether undo is available.
    pub can_undo: bool,
    /// Whether redo is available.
    pub can_redo: bool,
    /// Approximate model memory use.
    pub memory_bytes: usize,
    /// Whether the scenario's end condition was reached.
    pub finished: bool,
    /// Current parameter values.
    pub params: ParamSet,
    /// Apply policy reported for the most recent parameter change.
    pub last_param_change: Option<(String, ApplyPolicy)>,
    /// Number of times the published descriptor changed since spawning
    /// (compare it to notice a new [`Runner::descriptor`], which is always
    /// published before the status counting it).
    pub descriptor_seq: u64,
}

struct Shared<S: Simulation> {
    view: Mutex<Option<Published<S::View>>>,
    status: Mutex<RunnerStatus>,
    descriptor: Mutex<Arc<ModelDescriptor>>,
    metrics: Mutex<VecDeque<Metrics>>,
    errors: Mutex<Vec<String>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic on another thread must not take the user interface down with it.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Worker threads that have not exited yet.
static LIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

/// Number of runner worker threads alive in this process, including those
/// of dropped runners still handling the controls sent before the drop.
pub fn live_workers() -> usize {
    LIVE_WORKERS.load(Ordering::SeqCst)
}

/// Counts a worker thread for as long as it is owned by that thread.
struct LiveWorker;

impl LiveWorker {
    fn new() -> Self {
        LIVE_WORKERS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for LiveWorker {
    fn drop(&mut self) {
        LIVE_WORKERS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Owns a simulation on a worker thread.
pub struct Runner<S: Simulation> {
    tx: Sender<Control<S>>,
    shared: Arc<Shared<S>>,
    handle: Option<JoinHandle<()>>,
    clear_hook: Option<SimHook<S>>,
    randomize_hook: Option<SeededHook<S>>,
}

impl<S: Simulation> Runner<S> {
    /// Moves `sim` to a new worker thread.
    pub fn spawn(sim: S, config: RunnerConfig) -> Self {
        Self::spawn_with_descriptor(sim, config, S::descriptor)
    }

    /// Like [`Runner::spawn`], but the published descriptor is
    /// `describe(sim)` instead of [`Simulation::descriptor`], e.g. with texts
    /// adapted to the user interface. `describe` runs here once, then on the
    /// simulation thread after every control that may change the state and
    /// after the tick that follows it.
    pub fn spawn_with_descriptor(
        sim: S,
        config: RunnerConfig,
        describe: impl Fn(&S) -> ModelDescriptor + Send + 'static,
    ) -> Self {
        let descriptor = Arc::new(describe(&sim));
        let shared = Arc::new(Shared {
            view: Mutex::new(None),
            status: Mutex::new(RunnerStatus {
                speed: config.speed,
                params: sim.params(),
                tick: sim.tick(),
                ..Default::default()
            }),
            descriptor: Mutex::new(Arc::clone(&descriptor)),
            metrics: Mutex::new(VecDeque::new()),
            errors: Mutex::new(Vec::new()),
        });
        let (tx, rx) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let name = format!("sim-{}", descriptor.id);
        let live = LiveWorker::new();
        let handle = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                let _live = live;
                Worker::new(sim, rx, worker_shared, config, Box::new(describe), descriptor).run();
            })
            .expect("failed to spawn simulation thread");
        Self { tx, shared, handle: Some(handle), clear_hook: None, randomize_hook: None }
    }

    /// Declares how this model is cleared, enabling the generic "Clear" control.
    pub fn with_clear(mut self, hook: impl Fn(&mut S) + Send + Sync + 'static) -> Self {
        self.clear_hook = Some(Arc::new(hook));
        self
    }

    /// Declares how this model is randomised from a seed, enabling the generic
    /// "Randomize" control.
    pub fn with_randomize(mut self, hook: impl Fn(&mut S, u64) + Send + Sync + 'static) -> Self {
        self.randomize_hook = Some(Arc::new(hook));
        self
    }

    /// Current descriptor of the model, published again by the worker when
    /// a control, or the tick applying a [`ApplyPolicy::NextTick`] change,
    /// changes it ([`RunnerStatus::descriptor_seq`] counts the changes).
    pub fn descriptor(&self) -> Arc<ModelDescriptor> {
        Arc::clone(&lock(&self.shared.descriptor))
    }

    /// Sends a control message. Messages to a stopped worker are dropped.
    pub fn send(&self, control: Control<S>) {
        let _ = self.tx.send(control);
    }

    /// Starts continuous running.
    pub fn play(&self) {
        self.send(Control::Play);
    }

    /// Pauses.
    pub fn pause(&self) {
        self.send(Control::Pause);
    }

    /// Runs `n` ticks.
    pub fn step(&self, n: u64) {
        self.send(Control::Step(n));
    }

    /// Applies a non-undoable command.
    pub fn command(&self, command: S::Command) {
        self.send(Control::Command(command));
    }

    /// Applies an undoable command.
    pub fn edit(&self, command: S::Command) {
        self.send(Control::Edit(command));
    }

    /// Runs a closure on the simulation thread.
    pub fn exec(&self, f: impl FnOnce(&mut S) + Send + 'static) {
        self.send(Control::Exec(Box::new(f)));
    }

    /// Runs a closure that may change the simulation on its thread and
    /// returns a receiver for its result. Poll it with `try_recv` to avoid
    /// blocking a frame. Like [`Runner::exec`] it counts as a state change;
    /// use [`Runner::query`] to read.
    pub fn request<R: Send + 'static>(&self, f: impl FnOnce(&mut S) -> R + Send + 'static) -> Receiver<R> {
        let (tx, rx) = mpsc::channel();
        self.exec(move |sim| {
            let _ = tx.send(f(sim));
        });
        rx
    }

    /// Reads the simulation on its thread, after the controls sent before,
    /// and returns a receiver for the result. Poll it with `try_recv` to
    /// avoid blocking a frame. Nothing is rebuilt, recorded or published.
    pub fn query<R: Send + 'static>(&self, f: impl FnOnce(&S) -> R + Send + 'static) -> Receiver<R> {
        let (tx, rx) = mpsc::channel();
        self.send(Control::Query(Box::new(move |sim| {
            let _ = tx.send(f(sim));
        })));
        rx
    }

    /// Latest published view, if any.
    pub fn latest_view(&self) -> Option<Published<S::View>> {
        lock(&self.shared.view).clone()
    }

    /// Current status.
    pub fn status(&self) -> RunnerStatus {
        lock(&self.shared.status).clone()
    }

    /// Removes and returns buffered metric samples (oldest first).
    pub fn drain_metrics(&self) -> Vec<Metrics> {
        lock(&self.shared.metrics).drain(..).collect()
    }

    /// Removes and returns error messages produced by commands.
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.shared.errors))
    }

    /// Stops the worker and waits until its thread has exited, which happens
    /// once it has handled the controls sent before. Dropping a runner stops
    /// the worker without waiting.
    pub fn shutdown_and_wait(mut self) {
        self.send(Control::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Type-erased control surface of a running simulation. Generic user
/// interface code (time controls, metric plots, history timeline, save/load)
/// works through this trait without knowing the model type.
pub trait SessionControl: Send + Sync {
    /// Current model descriptor (it changes when [`RunnerStatus::descriptor_seq`] does).
    fn descriptor(&self) -> Arc<ModelDescriptor>;
    /// Current runner status.
    fn status(&self) -> RunnerStatus;
    /// Start running.
    fn play(&self);
    /// Pause.
    fn pause(&self);
    /// Toggle play/pause.
    fn toggle_play(&self);
    /// Run `n` ticks now.
    fn step(&self, n: u64);
    /// Change the target speed.
    fn set_speed(&self, speed: Speed);
    /// Reset to the scenario's initial state (undoable).
    fn reset(&self);
    /// Undo the last edit.
    fn undo(&self);
    /// Redo the last undone edit.
    fn redo(&self);
    /// Restore the n-th stored history snapshot.
    fn jump_to_history(&self, index: usize);
    /// Configure periodic history snapshots.
    fn set_history(&self, config: HistoryConfig);
    /// Change a parameter (the resulting apply policy appears in the status).
    fn set_param(&self, key: &str, value: ParamValue);
    /// Drain buffered metric samples.
    fn drain_metrics(&self) -> Vec<Metrics>;
    /// Drain error messages.
    fn take_errors(&self) -> Vec<String>;
    /// Ask for a snapshot of the current state.
    fn request_snapshot(&self) -> Receiver<Result<Snapshot, SimError>>;
    /// Restore a snapshot (pauses the run).
    fn restore(&self, snapshot: Snapshot);
    /// Whether this model supports clearing.
    fn can_clear(&self) -> bool;
    /// Clear the world (undoable). No-op when unsupported.
    fn clear(&self);
    /// Whether this model supports randomisation.
    fn can_randomize(&self) -> bool;
    /// Randomise the world with a seed (undoable). No-op when unsupported.
    fn randomize(&self, seed: u64);
}

impl<S: Simulation> SessionControl for Runner<S> {
    fn descriptor(&self) -> Arc<ModelDescriptor> {
        Runner::descriptor(self)
    }
    fn status(&self) -> RunnerStatus {
        Runner::status(self)
    }
    fn play(&self) {
        self.send(Control::Play);
    }
    fn pause(&self) {
        self.send(Control::Pause);
    }
    fn toggle_play(&self) {
        self.send(Control::TogglePlay);
    }
    fn step(&self, n: u64) {
        self.send(Control::Step(n));
    }
    fn set_speed(&self, speed: Speed) {
        self.send(Control::SetSpeed(speed));
    }
    fn reset(&self) {
        self.send(Control::Reset);
    }
    fn undo(&self) {
        self.send(Control::Undo);
    }
    fn redo(&self) {
        self.send(Control::Redo);
    }
    fn jump_to_history(&self, index: usize) {
        self.send(Control::JumpToHistory(index));
    }
    fn set_history(&self, config: HistoryConfig) {
        self.send(Control::SetHistory(config));
    }
    fn set_param(&self, key: &str, value: ParamValue) {
        self.send(Control::SetParam(key.to_string(), value));
    }
    fn drain_metrics(&self) -> Vec<Metrics> {
        Runner::drain_metrics(self)
    }
    fn take_errors(&self) -> Vec<String> {
        Runner::take_errors(self)
    }
    fn request_snapshot(&self) -> Receiver<Result<Snapshot, SimError>> {
        self.query(|sim| sim.snapshot())
    }
    fn restore(&self, snapshot: Snapshot) {
        self.send(Control::Restore(snapshot));
    }
    fn can_clear(&self) -> bool {
        self.clear_hook.is_some()
    }
    fn clear(&self) {
        if let Some(hook) = &self.clear_hook {
            let hook = Arc::clone(hook);
            self.send(Control::ExecEdit(Box::new(move |sim| hook(sim))));
        }
    }
    fn can_randomize(&self) -> bool {
        self.randomize_hook.is_some()
    }
    fn randomize(&self, seed: u64) {
        if let Some(hook) = &self.randomize_hook {
            let hook = Arc::clone(hook);
            self.send(Control::ExecEdit(Box::new(move |sim| hook(sim, seed))));
        }
    }
}

impl<S: Simulation> Drop for Runner<S> {
    fn drop(&mut self) {
        // Never wait for the worker here: it may be in the middle of a long
        // command (a Hashlife jump). It exits once it has handled the
        // controls sent before this one; dropping the join handle detaches
        // the thread.
        self.send(Control::Shutdown);
    }
}

struct Worker<S: Simulation> {
    sim: S,
    rx: Receiver<Control<S>>,
    shared: Arc<Shared<S>>,
    config: RunnerConfig,
    describe: DescribeFn<S>,
    descriptor: Arc<ModelDescriptor>,
    descriptor_seq: u64,
    /// A control handled since the last publication may have changed the
    /// state (and the descriptor).
    state_changed: bool,
    /// The state changed since the last tick: the next tick may apply a
    /// pending [`ApplyPolicy::NextTick`] change, so the descriptor is
    /// rebuilt after it.
    describe_after_tick: bool,
    playing: bool,
    speed: Speed,
    view_request: S::ViewRequest,
    dirty: bool,
    seq: u64,
    last_publish: Option<Instant>,
    history: SnapshotHistory,
    undo: UndoStack,
    tick_budget: f64,
    last_advance: Instant,
    last_step_ms: f64,
    avg_step_ms: f64,
    rate_window_start: Instant,
    rate_ticks: u64,
    rate_start_gen: u64,
    measured_tps: f64,
    measured_gps: f64,
    last_param_change: Option<(String, ApplyPolicy)>,
}

impl<S: Simulation> Worker<S> {
    fn new(
        sim: S,
        rx: Receiver<Control<S>>,
        shared: Arc<Shared<S>>,
        config: RunnerConfig,
        describe: DescribeFn<S>,
        descriptor: Arc<ModelDescriptor>,
    ) -> Self {
        let now = Instant::now();
        let start_gen = sim.tick();
        Self {
            sim,
            rx,
            shared,
            describe,
            descriptor,
            descriptor_seq: 0,
            state_changed: false,
            describe_after_tick: false,
            playing: false,
            speed: config.speed,
            view_request: S::ViewRequest::default(),
            dirty: true,
            seq: 0,
            last_publish: None,
            history: SnapshotHistory::new(config.history),
            undo: UndoStack::new(config.undo_bytes),
            config,
            tick_budget: 0.0,
            last_advance: now,
            last_step_ms: 0.0,
            avg_step_ms: 0.0,
            rate_window_start: now,
            rate_ticks: 0,
            rate_start_gen: start_gen,
            measured_tps: 0.0,
            measured_gps: 0.0,
            last_param_change: None,
        }
    }

    fn run(mut self) {
        self.capture_history();
        self.publish(true);
        self.update_status();
        loop {
            let wait = self.wait_time();
            let first = if wait.is_zero() {
                match self.rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            } else {
                match self.rx.recv_timeout(wait) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            if let Some(control) = first {
                if !self.handle(control) {
                    return;
                }
                loop {
                    match self.rx.try_recv() {
                        Ok(c) => {
                            if !self.handle(c) {
                                return;
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
            }
            if std::mem::take(&mut self.state_changed) {
                self.refresh_descriptor();
                if self.config.metrics_every > 0 {
                    self.record_metrics();
                }
            }
            if self.playing {
                self.run_due_ticks();
            }
            self.publish(false);
            self.update_rates();
            self.update_status();
        }
    }

    fn wait_time(&self) -> Duration {
        if !self.playing {
            return Duration::from_millis(50);
        }
        match self.speed {
            Speed::Unlimited => Duration::ZERO,
            Speed::TicksPerSecond(rate) => {
                let missing = (1.0 - self.tick_budget).max(0.0);
                let secs = if rate > 0.0 { missing / rate } else { 0.05 };
                Duration::from_secs_f64(secs.min(0.05))
            }
        }
    }

    /// Returns `false` when the worker must stop.
    fn handle(&mut self, control: Control<S>) -> bool {
        match control {
            Control::Play => self.set_playing(true),
            Control::Pause => self.set_playing(false),
            Control::TogglePlay => self.set_playing(!self.playing),
            Control::Step(n) => {
                for _ in 0..n {
                    self.do_tick();
                    if self.sim.is_finished() {
                        break;
                    }
                }
            }
            Control::SetSpeed(speed) => {
                self.speed = speed;
                self.tick_budget = 0.0;
                self.last_advance = Instant::now();
            }
            Control::Reset => {
                self.record_undo();
                self.sim.reset();
                self.history.clear();
                self.capture_history();
                self.changed();
            }
            Control::Command(command) => {
                if let Err(e) = self.sim.apply(command) {
                    self.report(e.to_string());
                }
                self.changed();
            }
            Control::Edit(command) => {
                let before = self.sim.snapshot();
                match self.sim.apply(command) {
                    Ok(()) => {
                        if let Ok(snapshot) = before {
                            self.undo.record(snapshot);
                        }
                    }
                    Err(e) => self.report(e.to_string()),
                }
                self.changed();
            }
            Control::SetParam(key, value) => {
                match self.sim.set_param(&key, value) {
                    Ok(policy) => self.last_param_change = Some((key, policy)),
                    Err(e) => self.report(e.to_string()),
                }
                self.changed();
            }
            Control::Exec(f) => {
                f(&mut self.sim);
                self.changed();
            }
            Control::Query(f) => f(&self.sim),
            Control::ExecEdit(f) => {
                self.record_undo();
                f(&mut self.sim);
                self.changed();
            }
            Control::Undo => self.undo_redo(true),
            Control::Redo => self.undo_redo(false),
            Control::JumpToHistory(index) => {
                if let Some(snapshot) = self.history.get(index).cloned() {
                    self.record_undo();
                    self.restore(&snapshot);
                    // The stored future is abandoned; running again recreates it.
                    self.history.truncate_after(snapshot.tick);
                    self.set_playing(false);
                }
            }
            Control::Restore(snapshot) => {
                self.record_undo();
                self.restore(&snapshot);
                self.history.clear();
                self.capture_history();
                self.set_playing(false);
            }
            Control::SetViewRequest(request) => {
                self.view_request = request;
                self.dirty = true;
                self.publish(true);
            }
            Control::SetHistory(config) => self.history.set_config(config),
            Control::Replace(sim) => {
                self.sim = *sim;
                self.history.clear();
                self.undo.clear();
                self.capture_history();
                self.set_playing(false);
                self.rate_start_gen = self.sim.tick();
                self.changed();
            }
            Control::RequestView => {
                self.dirty = true;
                self.publish(true);
            }
            Control::Shutdown => return false,
        }
        true
    }

    /// Notes that the state may have changed: the view, the descriptor and a
    /// metric sample are published again, and the descriptor once more after
    /// the next tick.
    fn changed(&mut self) {
        self.dirty = true;
        self.state_changed = true;
        self.describe_after_tick = true;
    }

    fn set_playing(&mut self, playing: bool) {
        if playing && !self.playing {
            // Start with one tick available so pressing play reacts at once.
            self.tick_budget = 1.0;
            self.last_advance = Instant::now();
        }
        self.playing = playing && !self.sim.is_finished();
    }

    fn record_undo(&mut self) {
        match self.sim.snapshot() {
            Ok(snapshot) => self.undo.record(snapshot),
            Err(e) => self.report(format!("could not record undo state: {e}")),
        }
    }

    fn undo_redo(&mut self, undo: bool) {
        let current = match self.sim.snapshot() {
            Ok(s) => s,
            Err(e) => {
                self.report(e.to_string());
                return;
            }
        };
        let target = if undo { self.undo.undo(current) } else { self.undo.redo(current) };
        if let Some(snapshot) = target {
            self.restore(&snapshot);
        }
    }

    fn restore(&mut self, snapshot: &Snapshot) {
        if let Err(e) = self.sim.restore(snapshot) {
            self.report(e.to_string());
        }
        self.rate_start_gen = self.sim.tick();
        self.changed();
    }

    fn run_due_ticks(&mut self) {
        let start = Instant::now();
        let deadline = start + self.config.max_batch;
        match self.speed {
            Speed::Unlimited => loop {
                self.do_tick();
                if self.sim.is_finished() {
                    self.playing = false;
                    break;
                }
                if Instant::now() >= deadline {
                    break;
                }
            },
            Speed::TicksPerSecond(rate) => {
                let elapsed = start.duration_since(self.last_advance).as_secs_f64();
                self.last_advance = start;
                // Do not accumulate more than a quarter second of debt: a slow
                // model runs slower instead of bursting to catch up.
                self.tick_budget = (self.tick_budget + elapsed * rate).min((rate * 0.25).max(1.0));
                while self.tick_budget >= 1.0 {
                    self.do_tick();
                    self.tick_budget -= 1.0;
                    if self.sim.is_finished() {
                        self.playing = false;
                        break;
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                }
            }
        }
    }

    fn do_tick(&mut self) {
        let t0 = Instant::now();
        self.sim.step();
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if std::mem::take(&mut self.describe_after_tick) {
            self.refresh_descriptor();
        }
        self.last_step_ms = ms;
        self.avg_step_ms = if self.avg_step_ms == 0.0 { ms } else { self.avg_step_ms * 0.9 + ms * 0.1 };
        self.rate_ticks += 1;
        let tick = self.sim.tick();
        if self.config.metrics_every > 0 && tick.is_multiple_of(self.config.metrics_every) {
            self.record_metrics();
        }
        self.capture_history();
        self.dirty = true;
    }

    /// Buffers a sample of the current tick. A buffered sample of the same
    /// tick (taken before a state change) is replaced: one sample per tick.
    fn record_metrics(&mut self) {
        let sample = self.sim.metrics();
        let mut buffer = lock(&self.shared.metrics);
        if buffer.back().is_some_and(|last| last.tick == sample.tick) {
            buffer.pop_back();
        } else if buffer.len() >= self.config.metrics_capacity {
            buffer.pop_front();
        }
        buffer.push_back(sample);
    }

    /// Publishes the descriptor again if it changed.
    fn refresh_descriptor(&mut self) {
        let descriptor = (self.describe)(&self.sim);
        if descriptor != *self.descriptor {
            self.descriptor = Arc::new(descriptor);
            self.descriptor_seq += 1;
            *lock(&self.shared.descriptor) = Arc::clone(&self.descriptor);
        }
    }

    fn capture_history(&mut self) {
        if self.history.wants(self.sim.tick()) {
            match self.sim.snapshot() {
                Ok(snapshot) => self.history.push(snapshot),
                Err(e) => self.report(format!("history snapshot failed: {e}")),
            }
        }
    }

    fn publish(&mut self, force: bool) {
        if !self.dirty {
            return;
        }
        let due = force
            || !self.playing
            || self.last_publish.is_none_or(|t| t.elapsed().as_secs_f64() >= 1.0 / self.config.publish_hz.max(1.0));
        if !due {
            return;
        }
        let view = self.sim.view(&self.view_request);
        self.seq += 1;
        *lock(&self.shared.view) = Some(Published { seq: self.seq, tick: self.sim.tick(), view: Arc::new(view) });
        self.last_publish = Some(Instant::now());
        self.dirty = false;
    }

    fn update_rates(&mut self) {
        let elapsed = self.rate_window_start.elapsed().as_secs_f64();
        if elapsed >= 0.5 {
            let generations = self.sim.tick().saturating_sub(self.rate_start_gen);
            self.measured_tps = self.rate_ticks as f64 / elapsed;
            self.measured_gps = generations as f64 / elapsed;
            self.rate_window_start = Instant::now();
            self.rate_ticks = 0;
            self.rate_start_gen = self.sim.tick();
        }
    }

    fn update_status(&mut self) {
        let status = RunnerStatus {
            playing: self.playing,
            tick: self.sim.tick(),
            speed: self.speed,
            measured_tps: if self.playing { self.measured_tps } else { 0.0 },
            measured_gps: if self.playing { self.measured_gps } else { 0.0 },
            last_step_ms: self.last_step_ms,
            avg_step_ms: self.avg_step_ms,
            history_ticks: self.history.entries().map(|s| s.tick).collect(),
            history_bytes: self.history.bytes(),
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            memory_bytes: self.sim.memory_bytes(),
            finished: self.sim.is_finished(),
            params: self.sim.params(),
            last_param_change: self.last_param_change.clone(),
            descriptor_seq: self.descriptor_seq,
        };
        *lock(&self.shared.status) = status;
    }

    fn report(&self, message: String) {
        let mut errors = lock(&self.shared.errors);
        if errors.len() < 256 {
            errors.push(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::test_model::{Walker, WalkerCommand};

    fn wait_for(runner: &Runner<Walker>, mut pred: impl FnMut(&RunnerStatus) -> bool) -> RunnerStatus {
        let start = Instant::now();
        loop {
            let status = runner.status();
            if pred(&status) {
                return status;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for runner state");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn step_and_view_publication() {
        let runner = Runner::spawn(Walker::new(3), RunnerConfig::default());
        runner.step(25);
        wait_for(&runner, |s| s.tick == 25);
        let rx = runner.query(|sim| sim.value);
        let value = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        runner.send(Control::RequestView);
        let start = Instant::now();
        loop {
            if let Some(p) = runner.latest_view()
                && p.tick == 25
            {
                assert_eq!(*p.view, value);
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(runner.drain_metrics().len(), 25);
    }

    #[test]
    fn results_do_not_depend_on_speed() {
        let reference = {
            let mut w = Walker::new(8);
            for _ in 0..200 {
                Simulation::step(&mut w);
            }
            w.value
        };
        let runner = Runner::spawn(Walker::new(8), RunnerConfig { speed: Speed::Unlimited, ..Default::default() });
        runner.play();
        wait_for(&runner, |s| s.tick >= 200);
        runner.pause();
        let st = wait_for(&runner, |s| !s.playing);
        // Replay the same number of ticks directly and compare.
        let mut w = Walker::new(8);
        for _ in 0..st.tick {
            Simulation::step(&mut w);
        }
        let value = runner.query(|sim| sim.value).recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(value, w.value);
        assert!(st.tick < 200 || reference > 0);
    }

    #[test]
    fn edits_are_undoable_and_errors_reported() {
        let runner = Runner::spawn(Walker::new(1), RunnerConfig::default());
        runner.edit(WalkerCommand::Add(100));
        let st = wait_for(&runner, |s| s.can_undo);
        assert!(!st.can_redo);
        runner.send(Control::Undo);
        wait_for(&runner, |s| s.can_redo);
        let value = runner.query(|sim| sim.value).recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(value, 0);
        runner.send(Control::Redo);
        wait_for(&runner, |s| !s.can_redo);
        let value = runner.query(|sim| sim.value).recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(value, 100);
        runner.command(WalkerCommand::Fail);
        let start = Instant::now();
        let errors = loop {
            let e = runner.take_errors();
            if !e.is_empty() {
                break e;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(errors[0].contains("requested failure"));
    }

    #[test]
    fn param_changes_report_policy() {
        let runner = Runner::spawn(Walker::new(1), RunnerConfig::default());
        runner.send(Control::SetParam("step_max".into(), ParamValue::Int(10)));
        let st = wait_for(&runner, |s| s.last_param_change.is_some());
        assert_eq!(st.last_param_change, Some(("step_max".to_string(), ApplyPolicy::NextTick)));
        assert_eq!(st.params.i64_or("step_max", 0), 10);
    }

    #[test]
    fn session_control_hooks() {
        let runner = Runner::spawn(Walker::new(2), RunnerConfig::default())
            .with_clear(|sim: &mut Walker| sim.value = 0)
            .with_randomize(|sim: &mut Walker, seed| sim.value = seed);
        let session: &dyn SessionControl = &runner;
        assert!(session.can_clear() && session.can_randomize());
        session.randomize(77);
        wait_for(&runner, |s| s.can_undo);
        let value = runner.query(|sim| sim.value).recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(value, 77);
        session.clear();
        let value = runner.query(|sim| sim.value).recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(value, 0);
        let snapshot = session.request_snapshot().recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(snapshot.model_id, "test.walker");
    }

    #[test]
    fn descriptor_follows_the_model() {
        let runner = Runner::spawn(Walker::new(1), RunnerConfig::default());
        let first = runner.descriptor();
        assert!(first.rules.contains("0..=3"), "{}", first.rules);
        // A NextTick parameter takes effect inside the next tick, which then
        // publishes the descriptor, also when it runs in a later batch.
        runner.send(Control::SetParam("step_max".into(), ParamValue::Int(10)));
        let st = wait_for(&runner, |s| s.params.i64_or("step_max", 0) == 10);
        assert_eq!(st.descriptor_seq, 0, "the new value is not in effect yet");
        runner.step(1);
        let st = wait_for(&runner, |s| s.tick == 1);
        assert_eq!(st.descriptor_seq, 1);
        assert!(runner.descriptor().rules.contains("0..=10"));
        assert!(first.rules.contains("0..=3"), "a published descriptor never changes");
        // Ticks and unchanged descriptors publish nothing new.
        runner.step(5);
        runner.edit(WalkerCommand::Add(1));
        let st = wait_for(&runner, |s| s.tick == 6 && s.can_undo);
        assert_eq!(st.descriptor_seq, 1);
        runner.send(Control::Reset);
        wait_for(&runner, |s| s.descriptor_seq == 2);
        assert_eq!(*runner.descriptor(), *first);
    }

    #[test]
    fn custom_descriptors_are_published_from_the_start() {
        let describe = |sim: &Walker| {
            let mut d = sim.descriptor();
            d.name = format!("Walker up to {}", sim.step_max);
            d
        };
        let runner = Runner::spawn_with_descriptor(Walker::new(1), RunnerConfig::default(), describe);
        assert_eq!(SessionControl::descriptor(&runner).name, "Walker up to 3");
        runner.send(Control::SetParam("step_max".into(), ParamValue::Int(7)));
        runner.step(1);
        wait_for(&runner, |s| s.descriptor_seq == 1);
        assert_eq!(runner.descriptor().name, "Walker up to 7");
    }

    #[test]
    fn queries_change_nothing() {
        let describes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&describes);
        let describe = move |sim: &Walker| {
            counter.fetch_add(1, Ordering::SeqCst);
            sim.descriptor()
        };
        let runner = Runner::spawn_with_descriptor(Walker::new(1), RunnerConfig::default(), describe);
        runner.step(2);
        wait_for(&runner, |s| s.tick == 2);
        runner.drain_metrics();
        let (described, view) = (describes.load(Ordering::SeqCst), runner.latest_view().unwrap());
        let value = runner.query(|sim| sim.value).recv_timeout(Duration::from_secs(5)).unwrap();
        let session: &dyn SessionControl = &runner;
        let snapshot = session.request_snapshot().recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!((value, snapshot.tick), (*view.view, 2));
        // The status shows the new speed once the batch holding the queries is done.
        runner.send(Control::SetSpeed(Speed::Unlimited));
        wait_for(&runner, |s| s.speed == Speed::Unlimited);
        assert_eq!(describes.load(Ordering::SeqCst), described, "a query rebuilt the descriptor");
        assert!(runner.drain_metrics().is_empty(), "a query recorded a metric sample");
        assert_eq!(runner.latest_view().unwrap().seq, view.seq, "a query published a view");
    }

    #[test]
    fn state_changes_publish_fresh_metrics() {
        let runner = Runner::spawn(Walker::new(1), RunnerConfig::default());
        runner.step(3);
        wait_for(&runner, |s| s.tick == 3);
        let value = runner.query(|sim| sim.value).recv_timeout(Duration::from_secs(5)).unwrap();
        // An edit while paused replaces the sample of the current tick.
        runner.edit(WalkerCommand::Add(100));
        wait_for(&runner, |s| s.can_undo);
        let samples = runner.drain_metrics();
        assert_eq!(samples.iter().map(|m| m.tick).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(samples[2].get("value"), Some((value + 100) as f64));
        // So does a closure that advances the model by itself.
        runner.exec(|sim| {
            for _ in 0..10 {
                Simulation::step(sim);
            }
        });
        wait_for(&runner, |s| s.tick == 13);
        let samples = runner.drain_metrics();
        let value = runner.query(|sim| sim.value).recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!((samples[0].tick, samples[0].get("value")), (13, Some(value as f64)));
    }

    #[test]
    fn dropping_does_not_wait_for_a_running_command() {
        // The worker owns `describe`: `exited` disconnects when its thread ends.
        let (exited_tx, exited) = mpsc::channel::<()>();
        let describe = move |sim: &Walker| {
            let _owned = &exited_tx;
            sim.descriptor()
        };
        let runner = Runner::spawn_with_descriptor(Walker::new(1), RunnerConfig::default(), describe);
        let (release_tx, release) = mpsc::channel::<()>();
        runner.exec(move |_| {
            let _ = release.recv();
        });
        let (dropped_tx, dropped) = mpsc::channel();
        std::thread::spawn(move || {
            drop(runner);
            let _ = dropped_tx.send(());
        });
        dropped.recv_timeout(Duration::from_secs(5)).expect("dropping the runner waited for the running command");
        assert_eq!(exited.recv_timeout(Duration::from_millis(50)), Err(RecvTimeoutError::Timeout));
        release_tx.send(()).unwrap();
        assert_eq!(exited.recv_timeout(Duration::from_secs(10)), Err(RecvTimeoutError::Disconnected));
    }

    #[test]
    fn shutdown_and_wait_returns_once_the_worker_exited() {
        let runner = Runner::spawn(Walker::new(1), RunnerConfig::default());
        let (release_tx, release) = mpsc::channel::<()>();
        runner.exec(move |_| {
            let _ = release.recv();
        });
        let (joined_tx, joined) = mpsc::channel();
        std::thread::spawn(move || {
            runner.shutdown_and_wait();
            let _ = joined_tx.send(());
        });
        assert!(joined.recv_timeout(Duration::from_millis(50)).is_err(), "the command is still running");
        release_tx.send(()).unwrap();
        joined.recv_timeout(Duration::from_secs(10)).expect("the worker exited");
    }

    #[test]
    fn history_jump_restores_state() {
        let config =
            RunnerConfig { history: HistoryConfig { interval: 10, ..Default::default() }, ..Default::default() };
        let runner = Runner::spawn(Walker::new(5), config);
        runner.step(35);
        let st = wait_for(&runner, |s| s.tick == 35);
        assert_eq!(st.history_ticks, vec![0, 10, 20, 30]);
        runner.send(Control::JumpToHistory(2));
        wait_for(&runner, |s| s.tick == 20);
    }
}
