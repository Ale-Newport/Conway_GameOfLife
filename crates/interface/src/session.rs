//! Per-frame polling of the active session: status, metric samples, error
//! messages and process telemetry.

use std::time::Duration;

use analytics::{StoreConfig, TimeSeriesStore};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use simulation_core::{Metrics, RunnerStatus};

use crate::mode::ActiveSession;
use crate::toasts::Toasts;

/// Status of the active session, read once per frame so every widget shows
/// the same values.
#[derive(Resource, Clone, Debug, Default)]
pub struct SessionStatus(Option<RunnerStatus>);

impl SessionStatus {
    /// Status of this frame (`None` without an active session).
    pub fn get(&self) -> Option<&RunnerStatus> {
        self.0.as_ref()
    }
}

/// Samples kept per metric before the store decimates (M4) older data.
pub const MAX_SAMPLES_PER_METRIC: usize = 100_000;

/// Time series of every metric of the active session, fed from
/// `drain_metrics` each frame. Cleared when the session changes or a saved
/// state is loaded; truncated when the run goes back in time (reset, undo,
/// history jump) so plots never mix two timelines.
#[derive(Resource, Debug)]
pub struct MetricsHistory {
    store: TimeSeriesStore,
    last_status_tick: Option<u64>,
    pending_restore: Option<PendingRestore>,
}

/// A restore sent to the session that the status has not confirmed yet.
#[derive(Clone, Copy, Debug)]
struct PendingRestore {
    tick: u64,
    polls: u32,
}

/// Polls (frames) to wait for a requested restore to show up in the status
/// before recording resumes anyway (the restore failed; the session reports
/// why as an error).
pub const RESTORE_WAIT_POLLS: u32 = 600;

impl Default for MetricsHistory {
    fn default() -> Self {
        Self { store: new_store(), last_status_tick: None, pending_restore: None }
    }
}

fn new_store() -> TimeSeriesStore {
    let config = StoreConfig { display_points: 1_500, max_raw_points: Some(MAX_SAMPLES_PER_METRIC) };
    TimeSeriesStore::with_config(config).unwrap_or_default()
}

impl MetricsHistory {
    /// The recorded series.
    pub fn store(&self) -> &TimeSeriesStore {
        &self.store
    }

    /// Latest value of `key`.
    pub fn latest(&self, key: &str) -> Option<f64> {
        self.store.last(key).map(|p| p.value)
    }

    /// Removes every sample.
    pub fn clear(&mut self) {
        self.store.clear();
        self.last_status_tick = None;
        self.pending_restore = None;
    }

    /// Clears the plots for a state restored at `tick` (a loaded save). The
    /// session applies the restore asynchronously: until a status shows the
    /// run paused at `tick`, drained samples still belong to the old run and
    /// are discarded, so the two runs never mix.
    pub fn expect_restore(&mut self, tick: u64) {
        self.clear();
        self.pending_restore = Some(PendingRestore { tick, polls: 0 });
    }

    /// Whether a restore requested with [`MetricsHistory::expect_restore`]
    /// is still waiting for confirmation.
    pub fn awaiting_restore(&self) -> bool {
        self.pending_restore.is_some()
    }

    /// Feeds one poll of the session: its status (`tick`, `playing`) read
    /// before draining `samples`.
    pub fn ingest(&mut self, tick: u64, playing: bool, samples: &[Metrics]) {
        if let Some(pending) = &mut self.pending_restore {
            // `restore` pauses the run, so "paused at the restored tick" marks
            // the switch; samples drained with that status predate it.
            let confirmed = tick == pending.tick && !playing;
            pending.polls += 1;
            if confirmed || pending.polls >= RESTORE_WAIT_POLLS {
                self.clear();
                self.last_status_tick = Some(tick);
            }
            return;
        }
        self.observe_status_tick(tick);
        self.record(samples);
    }

    /// Notes the tick reported by the runner status. A smaller tick than the
    /// previous status means the simulation went back in time: samples after
    /// it belong to an abandoned future and are removed.
    pub fn observe_status_tick(&mut self, tick: u64) {
        if self.last_status_tick.is_some_and(|last| tick < last) {
            self.store.truncate_after(tick);
        }
        self.last_status_tick = Some(tick);
    }

    /// Records samples in order. A sample older than the newest stored one
    /// (the run was rewound between two polls) truncates the store first.
    pub fn record(&mut self, samples: &[Metrics]) {
        for sample in samples {
            if self.store.last_tick().is_some_and(|last| sample.tick < last) {
                self.store.truncate_after(sample.tick);
            }
            if let Err(e) = self.store.record(sample) {
                warn!("metric sample at tick {} not recorded: {e}", sample.tick);
            }
        }
    }
}

/// Frame rate and process memory, refreshed twice per second.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct Telemetry {
    /// Smoothed frames per second.
    pub fps: Option<f64>,
    /// Resident set size of the process in bytes.
    pub process_memory: Option<u64>,
}

/// Polls the active session: status, metrics, errors.
pub(crate) fn poll_session(
    session: Option<Res<ActiveSession>>,
    mut status: ResMut<SessionStatus>,
    mut history: ResMut<MetricsHistory>,
    mut toasts: ResMut<Toasts>,
) {
    let Some(session) = session else {
        if status.0.is_some() {
            status.0 = None;
            history.clear();
        }
        return;
    };
    if session.is_changed() {
        history.clear();
    }
    let session = session.session();
    // Status first, then samples: every drained sample is at most as old as
    // a later status, so a rewind seen in the status never drops new data.
    let current = session.status();
    let samples = session.drain_metrics();
    history.ingest(current.tick, current.playing, &samples);
    for message in session.take_errors() {
        toasts.error(message);
    }
    status.0 = Some(current);
}

/// Refreshes [`Telemetry`].
pub(crate) fn update_telemetry(
    time: Res<Time<Real>>,
    diagnostics: Option<Res<DiagnosticsStore>>,
    mut telemetry: ResMut<Telemetry>,
    mut since: Local<Duration>,
) {
    *since += time.delta();
    if *since < Duration::from_millis(500) && telemetry.fps.is_some() {
        return;
    }
    *since = Duration::ZERO;
    telemetry.fps = diagnostics.and_then(|d| d.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()));
    telemetry.process_memory = memory_stats::memory_stats().map(|m| m.physical_mem as u64);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::commands::tests::FakeSession;

    fn sample(tick: u64, population: f64) -> Metrics {
        Metrics::new(tick, tick as f64).with("population", population)
    }

    #[test]
    fn rewinds_truncate_the_abandoned_future() {
        let mut h = MetricsHistory::default();
        h.observe_status_tick(0);
        h.record(&(1..=10).map(|t| sample(t, t as f64)).collect::<Vec<_>>());
        h.observe_status_tick(10);
        // Jump back to tick 4 and run on: new samples replace the old future.
        h.observe_status_tick(4);
        assert_eq!(h.store().last_tick(), Some(4));
        h.record(&[sample(5, 50.0), sample(6, 60.0)]);
        let series = h.store().get("population").unwrap();
        let ticks: Vec<u64> = series.ticks().collect();
        assert_eq!(ticks, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(h.latest("population"), Some(60.0));
    }

    #[test]
    fn samples_from_a_rewind_between_polls_are_kept() {
        let mut h = MetricsHistory::default();
        // Old future 1..=5, then (same drain) a reset and new ticks 1..=2.
        let drained: Vec<Metrics> = (1..=5).map(|t| sample(t, 1.0)).chain((1..=2).map(|t| sample(t, 2.0))).collect();
        h.record(&drained);
        let series = h.store().get("population").unwrap();
        assert_eq!(series.ticks().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(series.values().collect::<Vec<_>>(), vec![2.0, 2.0]);
    }

    #[test]
    fn loading_a_save_while_running_never_mixes_runs() {
        let mut h = MetricsHistory::default();
        h.ingest(400, true, &(391..=400).map(|t| sample(t, 1.0)).collect::<Vec<_>>());
        // Load a save made at tick 900 while the old run keeps playing.
        h.expect_restore(900);
        h.ingest(450, true, &(401..=450).map(|t| sample(t, 1.0)).collect::<Vec<_>>());
        assert!(h.store().is_empty() && h.awaiting_restore(), "old-run samples are discarded");
        // The status now shows the restored state (paused at 900); samples
        // drained together with it were produced before the restore.
        h.ingest(900, false, &[sample(451, 1.0), sample(452, 1.0)]);
        assert!(h.store().is_empty() && !h.awaiting_restore());
        h.ingest(902, true, &[sample(901, 2.0), sample(902, 2.0)]);
        let ticks: Vec<u64> = h.store().get("population").unwrap().ticks().collect();
        assert_eq!(ticks, vec![901, 902]);
    }

    #[test]
    fn a_failed_restore_stops_waiting() {
        let mut h = MetricsHistory::default();
        h.expect_restore(900);
        for _ in 0..RESTORE_WAIT_POLLS {
            h.ingest(10, false, &[]);
        }
        assert!(!h.awaiting_restore());
        h.ingest(11, true, &[sample(11, 3.0)]);
        assert_eq!(h.latest("population"), Some(3.0));
    }

    #[test]
    fn polling_feeds_store_status_and_toasts() {
        let fake = Arc::new(FakeSession::new());
        fake.status.lock().unwrap().tick = 3;
        *fake.metrics.lock().unwrap() = vec![sample(1, 4.0), sample(2, 5.0), sample(3, 6.0)];
        fake.errors.lock().unwrap().push("rule string `B9` is invalid".into());
        let mut app = App::new();
        app.init_resource::<SessionStatus>()
            .init_resource::<MetricsHistory>()
            .init_resource::<Toasts>()
            .insert_resource(ActiveSession::new(fake.clone()))
            .add_systems(Update, poll_session);
        app.update();
        assert_eq!(app.world().resource::<SessionStatus>().get().map(|s| s.tick), Some(3));
        assert_eq!(app.world().resource::<MetricsHistory>().latest("population"), Some(6.0));
        assert_eq!(app.world().resource::<Toasts>().items().len(), 1);
        // Removing the session clears everything.
        app.world_mut().remove_resource::<ActiveSession>();
        app.update();
        assert!(app.world().resource::<SessionStatus>().get().is_none());
        assert!(app.world().resource::<MetricsHistory>().store().is_empty());
    }
}
