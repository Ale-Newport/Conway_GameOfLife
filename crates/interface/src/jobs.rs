//! File work on background threads.
//!
//! Writing a saved state or a metrics export can take tens of milliseconds
//! even for small files (`persistence::atomic_write` flushes the file and
//! its folder to disk) and much longer for large states, so it never runs in
//! a frame. A job runs on its own thread and its outcome comes back as a
//! toast.

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use bevy::prelude::*;

use crate::error::InterfaceError;
use crate::toasts::Toasts;

/// What a background job does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum JobKind {
    /// Writing a saved state.
    Save,
    /// Writing a metrics export.
    MetricsExport,
}

impl JobKind {
    fn failure_prefix(self) -> &'static str {
        match self {
            JobKind::Save => "Save failed",
            JobKind::MetricsExport => "Export failed",
        }
    }
}

struct Job {
    kind: JobKind,
    // Behind a mutex only so the resource is `Sync`; one system polls it.
    receiver: Mutex<Receiver<Result<String, InterfaceError>>>,
}

/// Jobs in flight.
#[derive(Resource, Default)]
pub(crate) struct BackgroundJobs {
    jobs: Vec<Job>,
}

impl BackgroundJobs {
    /// Runs `work` on a new thread; its `Ok` message becomes a success toast
    /// and its error an error toast.
    pub(crate) fn spawn(
        &mut self,
        kind: JobKind,
        work: impl FnOnce() -> Result<String, InterfaceError> + Send + 'static,
    ) -> Result<(), InterfaceError> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name(format!("{kind:?} writer"))
            .spawn(move || {
                // The receiver is gone only if the app is shutting down.
                let _ = tx.send(work());
            })
            .map_err(InterfaceError::Thread)?;
        self.jobs.push(Job { kind, receiver: Mutex::new(rx) });
        Ok(())
    }

    /// Whether a job of `kind` is still running.
    pub(crate) fn is_running(&self, kind: JobKind) -> bool {
        self.jobs.iter().any(|job| job.kind == kind)
    }

    /// Collects finished jobs, oldest first.
    fn finished(&mut self) -> Vec<(JobKind, Result<String, InterfaceError>)> {
        let mut done = Vec::new();
        self.jobs.retain(|job| {
            let received = job.receiver.lock().map_or(Err(TryRecvError::Disconnected), |rx| rx.try_recv());
            match received {
                Ok(outcome) => {
                    done.push((job.kind, outcome));
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(TryRecvError::Disconnected) => {
                    done.push((job.kind, Err(InterfaceError::WorkerStopped)));
                    false
                }
            }
        });
        done
    }
}

/// Reports finished jobs as toasts.
pub(crate) fn poll_background_jobs(mut jobs: ResMut<BackgroundJobs>, mut toasts: ResMut<Toasts>) {
    for (kind, outcome) in jobs.finished() {
        match outcome {
            Ok(message) => toasts.success(message),
            Err(e) => {
                let message = format!("{}: {e}", kind.failure_prefix());
                error!("{message}");
                toasts.error(message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn wait_for_toasts(app: &mut App, count: usize) {
        let start = Instant::now();
        while app.world().resource::<Toasts>().items().len() < count {
            assert!(start.elapsed() < Duration::from_secs(10), "background jobs did not finish");
            std::thread::sleep(Duration::from_millis(2));
            app.update();
        }
    }

    #[test]
    fn outcomes_become_toasts() {
        let mut app = App::new();
        app.init_resource::<BackgroundJobs>().init_resource::<Toasts>().add_systems(Update, poll_background_jobs);
        let mut jobs = app.world_mut().resource_mut::<BackgroundJobs>();
        jobs.spawn(JobKind::Save, || Ok("written".to_string())).unwrap();
        jobs.spawn(JobKind::MetricsExport, || Err(InterfaceError::Empty("no samples".into()))).unwrap();
        assert!(jobs.is_running(JobKind::Save) && jobs.is_running(JobKind::MetricsExport));
        wait_for_toasts(&mut app, 2);
        let messages: Vec<String> =
            app.world().resource::<Toasts>().items().iter().map(|t| t.message.clone()).collect();
        assert!(messages.contains(&"written".to_string()), "{messages:?}");
        assert!(messages.iter().any(|m| m.starts_with("Export failed: nothing to export")), "{messages:?}");
        assert!(!app.world().resource::<BackgroundJobs>().is_running(JobKind::Save));
    }
}
