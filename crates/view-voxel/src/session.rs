//! The session shared with the shell: a `Runner<VoxelLife>` behind
//! `SessionControl`, with a descriptor that follows the running model.
//!
//! `Runner::descriptor` is captured when the runner is spawned, but the
//! voxel-life descriptor depends on the running rule and world (the rules
//! text, the per-state metrics `state_1 … state_{C−1}`, and the parameter
//! defaults of the loaded scenario). After `Control::Replace` (a new
//! scenario) or a rule change the runner's copy is stale, and the shell's
//! Parameters, Rules and Metrics tabs would describe the previous model.
//! [`VoxelSession`] therefore answers `descriptor()` from a log of
//! descriptors fetched from the simulation thread; the mode refreshes it
//! whenever the rule, parameters or world may have changed.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Sessions whose simulation thread may still be running.
static LIVE_SESSIONS: AtomicUsize = AtomicUsize::new(0);

/// Number of [`VoxelSession`]s alive in this process (diagnostics).
pub(crate) fn live_sessions() -> usize {
    LIVE_SESSIONS.load(Ordering::SeqCst)
}

/// Counts a session; dropped after the runner (fields drop in declaration
/// order), so the count falls only once the simulation thread has joined.
struct LiveGuard;

impl LiveGuard {
    fn new() -> Self {
        LIVE_SESSIONS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        LIVE_SESSIONS.fetch_sub(1, Ordering::SeqCst);
    }
}
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use bevy::log::warn;
use bevy::prelude::Resource;
use simulation_core::{
    HistoryConfig, Metrics, ModelDescriptor, ParamValue, Runner, RunnerConfig, RunnerStatus, SessionControl, SimError,
    Simulation, Snapshot, Speed,
};
use voxel_life::{VoxelCommand, VoxelLife};

/// Characters of the model's texts that egui's built-in fonts cannot draw,
/// with readable replacements (the shell renders the descriptor verbatim).
const REPLACEMENTS: [(char, &str); 3] = [('∈', "in"), ('∉', "not in"), ('∅', "{ }")];

fn replace_glyphs(text: &mut String) {
    if text.chars().any(|c| REPLACEMENTS.iter().any(|(r, _)| *r == c)) {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match REPLACEMENTS.iter().find(|(r, _)| *r == c) {
                Some((_, with)) => out.push_str(with),
                None => out.push(c),
            }
        }
        *text = out;
    }
}

/// The descriptor with every text drawable by egui's built-in fonts.
pub(crate) fn displayable(mut descriptor: ModelDescriptor) -> ModelDescriptor {
    replace_glyphs(&mut descriptor.summary);
    replace_glyphs(&mut descriptor.rules);
    for spec in &mut descriptor.params {
        replace_glyphs(&mut spec.label);
        replace_glyphs(&mut spec.description);
    }
    for spec in &mut descriptor.metrics {
        replace_glyphs(&mut spec.label);
        replace_glyphs(&mut spec.description);
    }
    descriptor
}

/// One logged descriptor.
struct DescriptorNode {
    descriptor: ModelDescriptor,
    next: OnceLock<Box<DescriptorNode>>,
}

impl Drop for DescriptorNode {
    fn drop(&mut self) {
        // Unlink iteratively so a long log cannot overflow the stack.
        let mut next = self.next.take();
        while let Some(mut node) = next {
            next = node.next.take();
        }
    }
}

/// Append-only log of distinct descriptors with a "current" index.
///
/// `SessionControl::descriptor` returns a reference tied to the session, so
/// replaced descriptors cannot be freed while it lives; they are kept in an
/// append-only list (deduplicated, so its length is the number of distinct
/// rule/world combinations used in this session) and freed with the session.
pub(crate) struct DescriptorLog {
    head: DescriptorNode,
    current: AtomicUsize,
    append: Mutex<()>,
}

impl DescriptorLog {
    /// A log holding `descriptor`.
    pub(crate) fn new(descriptor: ModelDescriptor) -> Self {
        Self {
            head: DescriptorNode { descriptor: displayable(descriptor), next: OnceLock::new() },
            current: AtomicUsize::new(0),
            append: Mutex::new(()),
        }
    }

    /// The current descriptor.
    pub(crate) fn current(&self) -> &ModelDescriptor {
        self.nth(self.current.load(Ordering::Acquire))
    }

    fn nth(&self, index: usize) -> &ModelDescriptor {
        let mut node = &self.head;
        for _ in 0..index {
            match node.next.get() {
                Some(next) => node = next,
                None => break,
            }
        }
        &node.descriptor
    }

    /// Makes `descriptor` current. Returns whether the current descriptor
    /// changed.
    pub(crate) fn update(&self, descriptor: ModelDescriptor) -> bool {
        let descriptor = displayable(descriptor);
        let _guard = self.append.lock().unwrap_or_else(PoisonError::into_inner);
        let current = self.current.load(Ordering::Acquire);
        let mut node = &self.head;
        let mut index = 0;
        loop {
            if node.descriptor == descriptor {
                self.current.store(index, Ordering::Release);
                return index != current;
            }
            match node.next.get() {
                Some(next) => {
                    node = next;
                    index += 1;
                }
                None => break,
            }
        }
        let _ = node.next.set(Box::new(DescriptorNode { descriptor, next: OnceLock::new() }));
        self.current.store(index + 1, Ordering::Release);
        true
    }

    /// Number of distinct descriptors logged.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        let mut node = &self.head;
        let mut len = 1;
        while let Some(next) = node.next.get() {
            node = next;
            len += 1;
        }
        len
    }
}

/// Typed handle of the running session (also inserted as the shell's
/// `ActiveSession`).
#[derive(Resource, Clone)]
pub(crate) struct VoxelRunner(pub Arc<VoxelSession>);

/// The running Three-Dimensional Life session.
pub(crate) struct VoxelSession {
    runner: Runner<VoxelLife>,
    descriptors: DescriptorLog,
    // Must stay the last field (see `LiveGuard`).
    _live: LiveGuard,
}

impl VoxelSession {
    /// Moves `life` to a simulation thread. Clear and Randomize are enabled
    /// (Randomize draws the pending random cube with the given seed).
    pub(crate) fn spawn(life: VoxelLife) -> Self {
        let config = RunnerConfig { publish_hz: 30.0, ..RunnerConfig::default() };
        let runner = Runner::spawn(life, config)
            .with_clear(|sim: &mut VoxelLife| {
                // Clearing cannot fail.
                let _ = sim.apply(VoxelCommand::Clear);
            })
            .with_randomize(|sim: &mut VoxelLife, seed| {
                if let Err(e) = sim.apply(VoxelCommand::Randomize { seed: Some(seed) }) {
                    warn!("3D Life: randomize refused: {e}");
                }
            });
        let descriptors = DescriptorLog::new(runner.descriptor().clone());
        Self { runner, descriptors, _live: LiveGuard::new() }
    }

    /// The typed runner.
    pub(crate) fn runner(&self) -> &Runner<VoxelLife> {
        &self.runner
    }

    /// Replaces the descriptor reported to the shell.
    pub(crate) fn set_descriptor(&self, descriptor: ModelDescriptor) -> bool {
        self.descriptors.update(descriptor)
    }
}

impl SessionControl for VoxelSession {
    fn descriptor(&self) -> &ModelDescriptor {
        self.descriptors.current()
    }
    fn status(&self) -> RunnerStatus {
        self.runner.status()
    }
    fn play(&self) {
        self.runner.play();
    }
    fn pause(&self) {
        self.runner.pause();
    }
    fn toggle_play(&self) {
        SessionControl::toggle_play(&self.runner);
    }
    fn step(&self, n: u64) {
        self.runner.step(n);
    }
    fn set_speed(&self, speed: Speed) {
        SessionControl::set_speed(&self.runner, speed);
    }
    fn reset(&self) {
        SessionControl::reset(&self.runner);
    }
    fn undo(&self) {
        SessionControl::undo(&self.runner);
    }
    fn redo(&self) {
        SessionControl::redo(&self.runner);
    }
    fn jump_to_history(&self, index: usize) {
        SessionControl::jump_to_history(&self.runner, index);
    }
    fn set_history(&self, config: HistoryConfig) {
        SessionControl::set_history(&self.runner, config);
    }
    fn set_param(&self, key: &str, value: ParamValue) {
        SessionControl::set_param(&self.runner, key, value);
    }
    fn drain_metrics(&self) -> Vec<Metrics> {
        self.runner.drain_metrics()
    }
    fn take_errors(&self) -> Vec<String> {
        self.runner.take_errors()
    }
    fn request_snapshot(&self) -> Receiver<Result<Snapshot, SimError>> {
        SessionControl::request_snapshot(&self.runner)
    }
    fn restore(&self, snapshot: Snapshot) {
        SessionControl::restore(&self.runner, snapshot);
    }
    fn can_clear(&self) -> bool {
        SessionControl::can_clear(&self.runner)
    }
    fn clear(&self) {
        SessionControl::clear(&self.runner);
    }
    fn can_randomize(&self) -> bool {
        SessionControl::can_randomize(&self.runner)
    }
    fn randomize(&self, seed: u64) {
        SessionControl::randomize(&self.runner, seed);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use simulation_core::{MetricSpec, ModelFamily};

    fn descriptor(rules: &str) -> ModelDescriptor {
        ModelDescriptor::new("voxel.life", "Three-Dimensional Life", ModelFamily::Voxel).rules(rules)
    }

    #[test]
    fn displayable_replaces_missing_glyphs_everywhere() {
        let descriptor = ModelDescriptor::new("m", "M", ModelFamily::Voxel)
            .summary("x ∈ A")
            .rules("N ∈ ∅, 2 … 9")
            .metrics(vec![MetricSpec::gauge("k", "∅", "u", "n ∉ S")]);
        let shown = displayable(descriptor);
        assert_eq!(shown.summary, "x in A");
        assert_eq!(shown.rules, "N in { }, 2 … 9", "drawable glyphs are kept");
        assert_eq!(shown.metrics[0].label, "{ }");
        assert_eq!(shown.metrics[0].description, "n not in S");
    }

    #[test]
    fn descriptor_log_switches_and_deduplicates() {
        let log = DescriptorLog::new(descriptor("a"));
        assert_eq!(log.current().rules, "a");
        assert!(!log.update(descriptor("a")), "same descriptor: no change");
        assert!(log.update(descriptor("b")));
        assert_eq!(log.current().rules, "b");
        assert!(log.update(descriptor("a")), "back to a known descriptor");
        assert_eq!(log.current().rules, "a");
        assert!(log.update(descriptor("b")));
        assert_eq!(log.len(), 2, "known descriptors are reused, not appended");
        assert!(log.update(descriptor("c")));
        assert_eq!((log.len(), log.current().rules.as_str()), (3, "c"));
    }

    #[test]
    fn session_reports_the_refreshed_descriptor_and_supports_clear() {
        let scenario = voxel_life::scenarios::builtin_by_id("life-5766-glider").expect("builtin");
        let session = VoxelSession::spawn(VoxelLife::new(scenario).expect("valid"));
        assert!(session.descriptor().rules.contains("5-7/6/2/M"), "{}", session.descriptor().rules);
        assert!(session.can_clear() && session.can_randomize());
        // Change the rule on the simulation thread and fetch its descriptor,
        // as the mode does after a rule edit.
        session.set_param("rule", ParamValue::Text("4555".into()));
        let rx = session.runner().request(|sim| sim.descriptor());
        let fresh = rx.recv_timeout(Duration::from_secs(5)).expect("descriptor");
        assert!(fresh.rules.contains("4-5/5/2/M"), "{}", fresh.rules);
        assert!(fresh.rules.contains('∈'), "the model writes set membership with ∈");
        assert!(session.descriptor().rules.contains("5-7/6/2/M"), "the runner's copy is stale");
        assert!(session.set_descriptor(fresh));
        assert!(session.descriptor().rules.contains("4-5/5/2/M"));
        assert!(!session.descriptor().rules.contains('∈'), "replaced for egui's fonts");
        assert!(session.descriptor().rules.contains("N in {5}"), "{}", session.descriptor().rules);
        session.clear();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let population = session.runner().request(|sim| sim.stats().population());
            if population.recv_timeout(Duration::from_secs(5)).expect("population") == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "clear was not applied");
        }
    }
}
