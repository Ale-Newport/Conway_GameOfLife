//! Exports: metric tables (CSV / JSON through `analytics`), PNG screenshots
//! and PNG frame sequences (Bevy's screenshot pipeline).
//!
//! Screenshots capture the whole window as rendered, user interface
//! included; hide the panels with Tab first for a clean view of the world.
//! Completion is detected when Bevy despawns the screenshot entity (after its
//! `save_to_disk` observer ran) and confirmed by checking the file exists.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use analytics::{TimeSeriesStore, store_to_json, write_store_csv};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::commands::ShellCommand;
use crate::error::InterfaceError;
use crate::format::{UtcDateTime, format_count};
use crate::mode::AppMode;
use crate::saves::DataLocation;
use crate::toasts::Toasts;

/// File format of a metrics export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetricsFormat {
    /// One `tick` column plus one column per metric (RFC 4180).
    Csv,
    /// Versioned columnar JSON document (`analytics::store_to_json`).
    Json,
}

impl MetricsFormat {
    /// File extension.
    pub fn extension(self) -> &'static str {
        match self {
            MetricsFormat::Csv => "csv",
            MetricsFormat::Json => "json",
        }
    }
}

/// Base file name for exports of `mode` at `time`: `classic-conway-20261109-140312`.
pub fn export_base_name(mode: AppMode, time: UtcDateTime) -> String {
    format!("{}-{}", mode.slug(), time.file_stamp())
}

/// `dir/<base><suffix>.<ext>`, adding `-2`, `-3`... if the file exists.
pub fn unique_path(dir: &Path, base: &str, suffix: &str, extension: &str) -> PathBuf {
    let mut path = dir.join(format!("{base}{suffix}.{extension}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}{suffix}-{n}.{extension}"));
        n += 1;
    }
    path
}

/// Writes every recorded metric of `store` to `dir` and returns the path.
pub fn export_metrics(
    store: &TimeSeriesStore,
    dir: &Path,
    base: &str,
    format: MetricsFormat,
) -> Result<PathBuf, InterfaceError> {
    if store.total_samples() == 0 {
        return Err(InterfaceError::Empty("no metric samples have been recorded yet; run the simulation first".into()));
    }
    std::fs::create_dir_all(dir).map_err(|e| InterfaceError::io("create the folder", dir, e))?;
    let path = unique_path(dir, base, "-metrics", format.extension());
    match format {
        MetricsFormat::Csv => {
            let file = std::fs::File::create(&path).map_err(|e| InterfaceError::io("create", &path, e))?;
            write_store_csv(store, file)?;
        }
        MetricsFormat::Json => {
            let text = store_to_json(store, true)?;
            persistence::atomic_write(&path, text.as_bytes())?;
        }
    }
    Ok(path)
}

/// Why a screenshot was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapturePurpose {
    /// A single screenshot requested by the user.
    Single,
    /// One frame of a sequence.
    Frame,
    /// Requested through [`ScreenshotTracker::capture`] by other code.
    External,
}

#[derive(Debug)]
struct PendingCapture {
    entity: Entity,
    path: PathBuf,
    purpose: CapturePurpose,
    frames_waited: u32,
    /// Modification time of a file already at `path` when the capture was
    /// requested, so a stale file is not mistaken for the new screenshot.
    previous: Option<SystemTime>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Whether a file was (re)written at `path` since `previous` was observed.
fn written_since(path: &Path, previous: Option<SystemTime>) -> bool {
    path.is_file() && modified(path).is_some_and(|now| previous.is_none_or(|before| now != before))
}

/// Screenshots in flight.
#[derive(Resource, Debug, Default)]
pub struct ScreenshotTracker {
    pending: Vec<PendingCapture>,
}

impl ScreenshotTracker {
    /// Captures the primary window to `path` (PNG). A
    /// [`ScreenshotFinished`] message reports the outcome.
    pub fn capture(&mut self, commands: &mut Commands, path: PathBuf) {
        self.spawn(commands, path, CapturePurpose::External);
    }

    fn spawn(&mut self, commands: &mut Commands, path: PathBuf, purpose: CapturePurpose) {
        let previous = modified(&path);
        let entity = commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone())).id();
        self.pending.push(PendingCapture { entity, path, purpose, frames_waited: 0, previous });
    }

    /// Number of screenshots in flight.
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }
}

/// Outcome of a screenshot.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct ScreenshotFinished {
    /// Target file.
    pub path: PathBuf,
    /// Whether the file was written.
    pub saved: bool,
}

/// A PNG sequence being recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceProgress {
    /// Output folder.
    pub dir: PathBuf,
    /// Frames to capture.
    pub total: u32,
    /// Frames requested so far.
    pub requested: u32,
    /// Frames written.
    pub saved: u32,
    /// Frames that failed.
    pub failed: u32,
    /// The user stopped the recording early.
    pub cancelled: bool,
}

/// Records one screenshot per rendered frame into a folder.
#[derive(Resource, Debug, Default)]
pub struct FrameRecorder {
    job: Option<SequenceProgress>,
}

/// Largest accepted frame count for one sequence.
pub const MAX_SEQUENCE_FRAMES: u32 = 10_000;

impl FrameRecorder {
    /// Starts recording `frames` frames into a new folder `dir`.
    pub fn start(&mut self, dir: PathBuf, frames: u32) -> Result<(), InterfaceError> {
        if self.job.is_some() {
            return Err(InterfaceError::Busy("a frame sequence is already being recorded"));
        }
        let total = frames.clamp(1, MAX_SEQUENCE_FRAMES);
        std::fs::create_dir_all(&dir).map_err(|e| InterfaceError::io("create the folder", &dir, e))?;
        self.job = Some(SequenceProgress { dir, total, requested: 0, saved: 0, failed: 0, cancelled: false });
        Ok(())
    }

    /// Stops requesting frames; frames in flight still complete.
    pub fn cancel(&mut self) {
        if let Some(job) = &mut self.job {
            job.cancelled = true;
        }
    }

    /// Current recording, if any.
    pub fn progress(&self) -> Option<&SequenceProgress> {
        self.job.as_ref()
    }

    /// Path of frame `index` (1-based) in `dir`.
    pub fn frame_path(dir: &Path, index: u32) -> PathBuf {
        dir.join(format!("frame-{index:05}.png"))
    }
}

/// Handles [`ShellCommand::Screenshot`].
pub(crate) fn screenshot_command(
    mut reader: MessageReader<ShellCommand>,
    mut commands: Commands,
    mut tracker: ResMut<ScreenshotTracker>,
    location: Res<DataLocation>,
    state: Res<State<AppMode>>,
    mut toasts: ResMut<Toasts>,
) {
    for _ in reader.read().filter(|c| **c == ShellCommand::Screenshot) {
        match location.dirs() {
            Ok(dirs) => {
                let dir = dirs.screenshots();
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    toasts.error(format!("Screenshot failed: cannot create `{}`: {e}", dir.display()));
                    continue;
                }
                let base = export_base_name(*state.get(), UtcDateTime::now());
                let path = unique_path(&dir, &base, "", "png");
                tracker.spawn(&mut commands, path, CapturePurpose::Single);
            }
            Err(e) => toasts.error(format!("Screenshot failed: {e}")),
        }
    }
}

/// Requests the next frame of a recording.
pub(crate) fn record_frames(
    mut commands: Commands,
    mut recorder: ResMut<FrameRecorder>,
    mut tracker: ResMut<ScreenshotTracker>,
) {
    let Some(job) = &mut recorder.job else { return };
    if !job.cancelled && job.requested < job.total {
        job.requested += 1;
        let path = FrameRecorder::frame_path(&job.dir, job.requested);
        tracker.spawn(&mut commands, path, CapturePurpose::Frame);
    }
}

/// Detects finished screenshots and reports them.
pub(crate) fn track_screenshots(
    in_flight: Query<(), With<Screenshot>>,
    mut tracker: ResMut<ScreenshotTracker>,
    mut recorder: ResMut<FrameRecorder>,
    mut toasts: ResMut<Toasts>,
    mut finished: MessageWriter<ScreenshotFinished>,
) {
    let mut done = Vec::new();
    tracker.pending.retain_mut(|capture| {
        capture.frames_waited += 1;
        // Commands spawning the entity apply after the requesting system;
        // only judge from the next frame on.
        if capture.frames_waited < 2 || in_flight.contains(capture.entity) {
            return true;
        }
        done.push((capture.path.clone(), capture.purpose, written_since(&capture.path, capture.previous)));
        false
    });
    for (path, purpose, saved) in done {
        match purpose {
            CapturePurpose::Single if saved => toasts.success(format!("Screenshot saved to {}", path.display())),
            CapturePurpose::Single => {
                toasts.error(format!("The screenshot could not be written to {} (see the log).", path.display()))
            }
            CapturePurpose::Frame => {
                if let Some(job) = &mut recorder.job {
                    if saved { job.saved += 1 } else { job.failed += 1 }
                }
            }
            CapturePurpose::External => {}
        }
        finished.write(ScreenshotFinished { path, saved });
    }
    let complete = recorder
        .job
        .as_ref()
        .is_some_and(|job| job.saved + job.failed >= job.requested && (job.cancelled || job.requested >= job.total));
    if complete && let Some(job) = recorder.job.take() {
        let summary = format!("{} frames saved to {}", format_count(job.saved.into()), job.dir.display());
        if job.failed > 0 {
            toasts.warning(format!("{summary}; {} frames failed (see the log).", job.failed));
        } else if job.cancelled {
            toasts.info(format!("Recording stopped: {summary}."));
        } else {
            toasts.success(format!("Sequence complete: {summary}."));
        }
    }
}

#[cfg(test)]
mod tests {
    use simulation_core::Metrics;

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("emergence-export-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn metrics_export_round_trips() {
        let dir = temp_dir("metrics");
        let mut store = TimeSeriesStore::new();
        assert!(matches!(export_metrics(&store, &dir, "x", MetricsFormat::Csv), Err(InterfaceError::Empty(_))));
        for t in 1..=3u64 {
            store.record(&Metrics::new(t, t as f64).with("population", 10.0 * t as f64)).unwrap();
        }
        let csv = export_metrics(&store, &dir, "run", MetricsFormat::Csv).unwrap();
        assert_eq!(csv.file_name().unwrap(), "run-metrics.csv");
        let text = std::fs::read_to_string(&csv).unwrap();
        assert_eq!(text, "tick,population\n1,10\n2,20\n3,30\n");
        let json = export_metrics(&store, &dir, "run", MetricsFormat::Json).unwrap();
        let back = analytics::store_from_json(&std::fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(back.get("population").unwrap().values().collect::<Vec<_>>(), vec![10.0, 20.0, 30.0]);
        // Same name again gets a numeric suffix instead of overwriting.
        let csv2 = export_metrics(&store, &dir, "run", MetricsFormat::Csv).unwrap();
        assert_eq!(csv2.file_name().unwrap(), "run-metrics-2.csv");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_files_are_not_reported_as_new_screenshots() {
        let dir = temp_dir("stale");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        assert!(!written_since(&path, None), "nothing written yet");
        std::fs::write(&path, b"old").unwrap();
        let before = modified(&path);
        assert!(!written_since(&path, before), "the old file is not the new capture");
        let later = before.unwrap() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        assert!(written_since(&path, before), "a rewritten file counts");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_and_frame_paths() {
        let t = UtcDateTime::from_unix_seconds(0);
        assert_eq!(export_base_name(AppMode::EvolutionLab, t), "evolution-19700101-000000");
        assert_eq!(FrameRecorder::frame_path(Path::new("/x"), 7), PathBuf::from("/x/frame-00007.png"));
    }

    #[test]
    fn recorder_requests_each_frame_and_reports_completion() {
        let dir = temp_dir("frames");
        let mut app = App::new();
        app.init_resource::<FrameRecorder>()
            .init_resource::<ScreenshotTracker>()
            .init_resource::<Toasts>()
            .add_message::<ScreenshotFinished>()
            .add_systems(Update, (record_frames, track_screenshots).chain());
        app.world_mut().resource_mut::<FrameRecorder>().start(dir.clone(), 3).unwrap();
        assert!(app.world_mut().resource_mut::<FrameRecorder>().start(dir.clone(), 3).is_err());
        // Without a renderer nothing captures the screenshots: every entity
        // keeps its `Screenshot` component, so nothing completes.
        for _ in 0..5 {
            app.update();
        }
        let progress = app.world().resource::<FrameRecorder>().progress().cloned().unwrap();
        assert_eq!((progress.requested, progress.saved, progress.failed), (3, 0, 0));
        assert_eq!(app.world().resource::<ScreenshotTracker>().in_flight(), 3);
        // Simulate Bevy finishing the captures: two files written, one not.
        std::fs::write(FrameRecorder::frame_path(&dir, 1), b"png").unwrap();
        std::fs::write(FrameRecorder::frame_path(&dir, 2), b"png").unwrap();
        let entities: Vec<Entity> =
            app.world_mut().query_filtered::<Entity, With<Screenshot>>().iter(app.world()).collect();
        for e in entities {
            app.world_mut().despawn(e);
        }
        app.update();
        assert!(app.world().resource::<FrameRecorder>().progress().is_none());
        let toasts = app.world().resource::<Toasts>();
        assert!(toasts.items()[0].message.contains("2 frames saved"), "{:?}", toasts.items());
        let _ = std::fs::remove_dir_all(dir);
    }
}
