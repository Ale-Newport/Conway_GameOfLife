//! Saving and loading simulation states.
//!
//! States are stored as standard persistence snapshot files (`.elsnap`,
//! envelope kind `snapshot`) under `<data>/saves/<mode-slug>/<name>.elsnap`,
//! so the command-line tool can read them too. Loading restores the state
//! through [`SessionControl::restore`] (which pauses the run and is
//! undoable).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use persistence::binary::{MAGIC, PREFIX_LEN};
use persistence::{Codec, UserDataDirs};
use serde::Deserialize;
use simulation_core::{SessionControl, SimError, Snapshot};

use crate::error::InterfaceError;
use crate::format::{UtcDateTime, format_bytes, format_count, sanitize_file_stem};
use crate::jobs::{BackgroundJobs, JobKind};
use crate::mode::{ActiveSession, AppMode};
use crate::session::MetricsHistory;
use crate::toasts::Toasts;

/// File extension of saved states.
pub const SAVE_EXTENSION: &str = "elsnap";

/// The per-user data folder, resolved once at startup.
#[derive(Resource, Clone, Debug)]
pub struct DataLocation(Result<UserDataDirs, String>);

impl DataLocation {
    /// Resolves the folder from the environment and creates it.
    pub fn resolve() -> Self {
        let dirs = UserDataDirs::resolve().and_then(|dirs| dirs.ensure().map(|()| dirs));
        Self(dirs.map_err(|e| e.to_string()))
    }

    /// Uses an explicit folder (tests, portable installs).
    pub fn at(dirs: UserDataDirs) -> Self {
        Self(Ok(dirs))
    }

    /// The folders, or why they are unavailable.
    pub fn dirs(&self) -> Result<&UserDataDirs, InterfaceError> {
        self.0.as_ref().map_err(|e| InterfaceError::DataDir(e.clone()))
    }
}

/// Folder holding the saves of `mode`.
pub fn mode_save_dir(dirs: &UserDataDirs, mode: AppMode) -> PathBuf {
    dirs.saves().join(mode.slug())
}

/// Path for a save called `name` (sanitised), creating the mode folder.
/// Fails when the file exists and `overwrite` is false.
pub fn save_path(dirs: &UserDataDirs, mode: AppMode, name: &str, overwrite: bool) -> Result<PathBuf, InterfaceError> {
    let stem = sanitize_file_stem(name)?;
    let dir = mode_save_dir(dirs, mode);
    std::fs::create_dir_all(&dir).map_err(|e| InterfaceError::io("create the folder", &dir, e))?;
    let path = dir.join(format!("{stem}.{SAVE_EXTENSION}"));
    if path.exists() && !overwrite {
        return Err(InterfaceError::AlreadyExists(path));
    }
    Ok(path)
}

/// One saved state in a mode folder.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveEntry {
    /// File path.
    pub path: PathBuf,
    /// Name (file stem).
    pub name: String,
    /// Tick stored in the snapshot, when readable.
    pub tick: Option<u64>,
    /// Creation time from the envelope (ISO 8601).
    pub created: String,
    /// Model that produced the state.
    pub model_id: Option<String>,
    /// File size.
    pub size_bytes: u64,
    /// Why the file cannot be loaded, if its header is unreadable.
    pub problem: Option<String>,
}

/// Leading fields of a postcard-encoded [`Snapshot`] (same order as the
/// struct), enough to read the tick without decoding the state.
#[derive(Deserialize)]
struct SnapshotHead {
    #[serde(rename = "model_id")]
    _model_id: String,
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    tick: u64,
}

/// Reads the tick of the snapshot stored in an `.elsnap` file by decoding
/// only the beginning of its payload (container layout documented in
/// `persistence::binary`). The full file is validated when it is loaded.
pub fn peek_snapshot_tick(path: &Path) -> Result<u64, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut prefix = [0u8; PREFIX_LEN];
    file.read_exact(&mut prefix).map_err(|_| "the file is too short".to_string())?;
    if prefix[..8] != MAGIC {
        return Err("not an .elsnap file".to_string());
    }
    let header_len = u32::from_le_bytes([prefix[12], prefix[13], prefix[14], prefix[15]]);
    let payload_len = u64::from_le_bytes(prefix[16..24].try_into().map_err(|_| "corrupt prefix".to_string())?);
    file.seek(SeekFrom::Start(PREFIX_LEN as u64 + u64::from(header_len))).map_err(|e| e.to_string())?;
    let mut head = Vec::new();
    file.take(payload_len.min(64 * 1024)).read_to_end(&mut head).map_err(|e| e.to_string())?;
    postcard::take_from_bytes::<SnapshotHead>(&head)
        .map(|(h, _)| h.tick)
        .map_err(|e| format!("unreadable snapshot header ({e})"))
}

/// Lists the saves in `dir` (newest first). A missing folder is empty.
pub fn list_saves(dir: &Path) -> Result<Vec<SaveEntry>, InterfaceError> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(InterfaceError::io("list", dir, e)),
    };
    let codec = Codec::new();
    let mut entries = Vec::new();
    for item in read {
        let item = item.map_err(|e| InterfaceError::io("list", dir, e))?;
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some(SAVE_EXTENSION) {
            continue;
        }
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let size_bytes = item.metadata().map(|m| m.len()).unwrap_or(0);
        let entry = match codec.read_header(&path) {
            Ok(header) => SaveEntry {
                tick: peek_snapshot_tick(&path).ok(),
                created: header.created,
                model_id: header.model_id,
                problem: None,
                path,
                name,
                size_bytes,
            },
            Err(e) => SaveEntry {
                path,
                name,
                tick: None,
                created: String::new(),
                model_id: None,
                size_bytes,
                problem: Some(e.to_string()),
            },
        };
        entries.push(entry);
    }
    entries.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

/// Loads the snapshot at `path`, checking it belongs to `model_id`.
pub fn load_snapshot(path: &Path, model_id: &str) -> Result<Snapshot, InterfaceError> {
    Ok(Codec::new().load_snapshot(path, Some(model_id))?.into_payload())
}

/// A save waiting for the simulation thread to produce the snapshot.
pub(crate) struct SaveJob {
    // Behind a mutex only so the resource is `Sync`; one system polls it.
    receiver: Mutex<Receiver<Result<Snapshot, SimError>>>,
    path: PathBuf,
    name: String,
    started: Instant,
}

/// The save waiting for its snapshot, if any. Once the snapshot arrives the
/// file is written by a [`JobKind::Save`] background job.
#[derive(Resource, Default)]
pub(crate) struct PendingSave(Option<SaveJob>);

impl PendingSave {
    pub(crate) fn is_busy(&self) -> bool {
        self.0.is_some()
    }
}

/// Whether a save is in progress (waiting for the snapshot or writing).
pub(crate) fn save_in_progress(pending: &PendingSave, jobs: &BackgroundJobs) -> bool {
    pending.is_busy() || jobs.is_running(JobKind::Save)
}

/// How long to wait for a snapshot before reporting a failure.
const SAVE_TIMEOUT: Duration = Duration::from_secs(60);

/// Starts saving the active session as `name`. Refused while another save
/// is in progress, so the "already exists" check stays meaningful.
pub(crate) fn start_save(
    session: &dyn SessionControl,
    location: &DataLocation,
    mode: AppMode,
    name: &str,
    overwrite: bool,
    pending: &mut PendingSave,
    jobs: &BackgroundJobs,
) -> Result<(), InterfaceError> {
    if save_in_progress(pending, jobs) {
        return Err(InterfaceError::Busy("the previous save is still being written"));
    }
    let path = save_path(location.dirs()?, mode, name, overwrite)?;
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    pending.0 = Some(SaveJob { receiver: Mutex::new(session.request_snapshot()), path, name, started: Instant::now() });
    Ok(())
}

/// Writes `snapshot` to `path` (a persistence snapshot file) and returns the
/// confirmation shown to the user.
fn write_save(path: &Path, name: &str, snapshot: &Snapshot) -> Result<String, InterfaceError> {
    Codec::new().save_snapshot(path, snapshot, &UtcDateTime::now().iso8601())?;
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(format!("Saved “{name}” at tick {} ({}).", format_count(snapshot.tick), format_bytes(size)))
}

/// Hands a pending save to a background writer once the snapshot arrives.
pub(crate) fn poll_pending_save(
    mut pending: ResMut<PendingSave>,
    mut jobs: ResMut<BackgroundJobs>,
    mut toasts: ResMut<Toasts>,
) {
    let Some(job) = &pending.0 else { return };
    let received = job.receiver.lock().map_or(Err(TryRecvError::Disconnected), |rx| rx.try_recv());
    let snapshot = match received {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(e)) => Err(InterfaceError::Simulation(e)),
        Err(TryRecvError::Disconnected) => Err(InterfaceError::SessionGone),
        Err(TryRecvError::Empty) if job.started.elapsed() > SAVE_TIMEOUT => {
            Err(InterfaceError::Timeout(SAVE_TIMEOUT.as_secs()))
        }
        Err(TryRecvError::Empty) => return,
    };
    let Some(SaveJob { path, name, .. }) = pending.0.take() else { return };
    let written = snapshot.and_then(|snapshot| jobs.spawn(JobKind::Save, move || write_save(&path, &name, &snapshot)));
    if let Err(e) = written {
        toasts.error(format!("Save failed: {e}"));
    }
}

/// Restores a saved state into the active session and clears the plots.
pub(crate) fn load_into_session(
    entry: &SaveEntry,
    session: &ActiveSession,
    history: &mut MetricsHistory,
) -> Result<String, InterfaceError> {
    let snapshot = load_snapshot(&entry.path, &session.session().descriptor().id)?;
    let tick = snapshot.tick;
    session.session().restore(snapshot);
    history.expect_restore(tick);
    Ok(format!("Loaded “{}” at tick {} (paused).", entry.name, format_count(tick)))
}

/// Deletes a save file.
pub(crate) fn delete_save(entry: &SaveEntry) -> Result<(), InterfaceError> {
    std::fs::remove_file(&entry.path).map_err(|e| InterfaceError::io("delete", &entry.path, e))
}

/// State of the save dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SaveDialog {
    /// Name typed by the user.
    pub name: String,
    /// Replace an existing save with the same name.
    pub overwrite: bool,
    /// Last validation error.
    pub error: Option<String>,
}

impl SaveDialog {
    /// Dialog pre-filled with a name made of the mode title and the tick.
    pub(crate) fn new(mode_title: &str, tick: u64) -> Self {
        Self { name: format!("{mode_title} tick {tick}"), ..Default::default() }
    }
}

/// State of the load dialog.
#[derive(Debug)]
pub(crate) struct LoadDialog {
    /// Saves of the mode (listed when the dialog opened or refreshed).
    pub entries: Result<Vec<SaveEntry>, String>,
    /// Save awaiting delete confirmation.
    pub confirm_delete: Option<PathBuf>,
}

impl LoadDialog {
    /// Lists the saves of `mode`.
    pub(crate) fn open(location: &DataLocation, mode: AppMode) -> Self {
        let entries =
            location.dirs().and_then(|dirs| list_saves(&mode_save_dir(dirs, mode))).map_err(|e| e.to_string());
        Self { entries, confirm_delete: None }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::commands::tests::FakeSession;

    fn temp_dirs(tag: &str) -> (PathBuf, UserDataDirs) {
        let root = std::env::temp_dir().join(format!("emergence-interface-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = UserDataDirs::at(&root);
        dirs.ensure().unwrap();
        (root, dirs)
    }

    #[test]
    fn save_list_peek_load_and_delete() {
        let (root, dirs) = temp_dirs("saves");
        let location = DataLocation::at(dirs.clone());
        let fake = Arc::new(FakeSession::new());
        fake.status.lock().unwrap().tick = 1234;
        let mut app = App::new();
        app.init_resource::<PendingSave>()
            .init_resource::<BackgroundJobs>()
            .init_resource::<Toasts>()
            .add_systems(Update, (poll_pending_save, crate::jobs::poll_background_jobs).chain());
        let start = |app: &mut App, name: &str| {
            app.world_mut().resource_scope(|world, mut pending: Mut<PendingSave>| {
                let jobs = world.resource::<BackgroundJobs>();
                start_save(fake.as_ref(), &location, AppMode::ClassicConway, name, false, &mut pending, jobs)
            })
        };
        start(&mut app, " My/run ").unwrap();
        // A second save while the first is in progress is refused, not dropped.
        assert!(matches!(start(&mut app, "other"), Err(InterfaceError::Busy(_))));
        // The snapshot arrives, the file is written on a background thread
        // and the outcome comes back as a toast.
        let begun = Instant::now();
        while app.world().resource::<Toasts>().items().is_empty() {
            assert!(begun.elapsed() < Duration::from_secs(10), "the save did not complete");
            app.update();
            std::thread::sleep(Duration::from_millis(2));
        }
        let message = app.world().resource::<Toasts>().items()[0].message.clone();
        assert!(message.contains("My-run") && message.contains("1,234"), "{message}");
        assert!(!save_in_progress(app.world().resource::<PendingSave>(), app.world().resource::<BackgroundJobs>()));

        // A second save with the same name needs confirmation.
        assert!(matches!(start(&mut app, "My-run"), Err(InterfaceError::AlreadyExists(_))));

        let entries = list_saves(&mode_save_dir(&dirs, AppMode::ClassicConway)).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.name, "My-run");
        assert_eq!(entry.tick, Some(1234));
        assert_eq!(entry.model_id.as_deref(), Some("test.fake"));
        assert!(entry.problem.is_none() && entry.created.ends_with('Z'));
        assert_eq!(peek_snapshot_tick(&entry.path), Ok(1234));

        let session = ActiveSession::new(fake.clone());
        let mut history = MetricsHistory::default();
        let loaded = load_into_session(entry, &session, &mut history).unwrap();
        assert!(loaded.contains("1,234"));
        assert_eq!(fake.calls(), vec!["restore 1234"]);
        assert!(history.awaiting_restore(), "plots wait for the restore before recording again");

        // Other modes have their own folder.
        assert!(list_saves(&mode_save_dir(&dirs, AppMode::EvolutionLab)).unwrap().is_empty());

        delete_save(entry).unwrap();
        assert!(list_saves(&mode_save_dir(&dirs, AppMode::ClassicConway)).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn broken_files_are_listed_with_a_problem() {
        let (root, dirs) = temp_dirs("broken");
        let dir = mode_save_dir(&dirs, AppMode::EvolutionLab);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("junk.elsnap"), b"not a snapshot").unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let entries = list_saves(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].problem.is_some());
        assert!(peek_snapshot_tick(&entries[0].path).is_err());
        let fake = FakeSession::new();
        let session = ActiveSession::new(Arc::new(fake));
        assert!(load_into_session(&entries[0], &session, &mut MetricsHistory::default()).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unavailable_data_folder_is_reported() {
        let location = DataLocation(Err("HOME is not set".to_string()));
        let err = location.dirs().unwrap_err().to_string();
        assert!(err.contains("HOME is not set"));
        let dialog = LoadDialog::open(&location, AppMode::ClassicConway);
        assert!(dialog.entries.is_err());
    }
}
