//! Error type of the application shell. Messages are shown to users in
//! toasts and dialogs.

use std::path::PathBuf;

use analytics::AnalyticsError;
use persistence::PersistenceError;
use simulation_core::SimError;
use thiserror::Error;

/// Errors raised by shell operations (saving, loading, exporting).
#[derive(Debug, Error)]
pub enum InterfaceError {
    /// A user-chosen name cannot be used as a file name.
    #[error("invalid name: {0}")]
    InvalidName(String),
    /// The per-user data folder could not be resolved or created.
    #[error("the data folder is unavailable: {0}")]
    DataDir(String),
    /// No simulation is running.
    #[error("no simulation is running in this mode")]
    NoSession,
    /// A save file exists and overwriting was not confirmed.
    #[error("`{}` already exists; tick \"Overwrite\" to replace it", .0.display())]
    AlreadyExists(PathBuf),
    /// The simulation thread stopped before answering.
    #[error("the simulation stopped before it could produce a snapshot")]
    SessionGone,
    /// The simulation could not produce or restore a snapshot.
    #[error("simulation error: {0}")]
    Simulation(#[from] SimError),
    /// Reading or writing a save file failed.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Exporting metrics failed.
    #[error("export failed: {0}")]
    Analytics(#[from] AnalyticsError),
    /// A file-system operation failed.
    #[error("cannot {action} `{}`: {source}", path.display())]
    Io {
        /// What was attempted.
        action: &'static str,
        /// File or folder involved.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// There is nothing to export yet.
    #[error("nothing to export: {0}")]
    Empty(String),
    /// The same kind of operation is already running.
    #[error("busy: {0}")]
    Busy(&'static str),
    /// The simulation did not answer in time.
    #[error("the simulation did not answer within {0} seconds")]
    Timeout(u64),
    /// A background thread could not be started.
    #[error("cannot start a background thread: {0}")]
    Thread(std::io::Error),
    /// A background thread ended without reporting (it panicked).
    #[error("the background writer stopped unexpectedly (see the log)")]
    WorkerStopped,
}

impl InterfaceError {
    /// Convenience constructor for [`InterfaceError::Io`].
    pub fn io(action: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { action, path: path.into(), source }
    }
}
