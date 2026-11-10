//! Writing to standard output and writing result files.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, bail};

/// Set once the reader of standard output has gone away.
static STDOUT_CLOSED: AtomicBool = AtomicBool::new(false);
/// The first standard-output error other than a closed pipe.
static STDOUT_ERROR: OnceLock<String> = OnceLock::new();

/// Writes to standard output (used through [`out!`] and [`outln!`]).
///
/// When the reader has gone away (`emergence-cli list scenarios | head`),
/// the rest of the output is dropped and the command carries on, so that
/// result files are still written and the exit status reflects the work.
/// Any other write error is kept for [`stdout_error`] and also stops the
/// output.
pub fn write_stdout(args: fmt::Arguments<'_>) {
    if STDOUT_CLOSED.load(Ordering::Relaxed) {
        return;
    }
    if let Err(error) = io::stdout().lock().write_fmt(args) {
        STDOUT_CLOSED.store(true, Ordering::Relaxed);
        if error.kind() != io::ErrorKind::BrokenPipe {
            let _ = STDOUT_ERROR.set(error.to_string());
        }
    }
}

/// The standard-output error that stopped the output, if any (a closed
/// pipe is not an error).
pub fn stdout_error() -> Option<&'static str> {
    STDOUT_ERROR.get().map(String::as_str)
}

/// `print!` through [`write_stdout`].
macro_rules! out {
    ($($arg:tt)*) => {
        $crate::output::write_stdout(format_args!($($arg)*))
    };
}

/// `println!` through [`write_stdout`].
macro_rules! outln {
    () => {
        $crate::output::write_stdout(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::output::write_stdout(format_args!("{}\n", format_args!($($arg)*)))
    };
}

pub(crate) use {out, outln};

/// Format of an export file, chosen by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// Comma-separated values.
    Csv,
    /// JSON.
    Json,
}

impl ExportFormat {
    /// The format of `path`.
    ///
    /// # Errors
    /// Extensions other than `.csv` and `.json`.
    pub fn of(path: &Path) -> anyhow::Result<Self> {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("csv") => Ok(Self::Csv),
            Some("json") => Ok(Self::Json),
            _ => bail!("{}: the file name must end in .csv or .json", path.display()),
        }
    }
}

/// An error whose message already includes its causes (the persistence
/// crate's errors name the file and quote the operating-system error), as a
/// single-message error so that `{:#}` does not repeat the causes.
pub fn flat(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}

/// Writes `text` to `path` atomically, creating missing parent folders.
///
/// # Errors
/// File-system errors, naming the file.
pub fn write_file(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty() && !p.exists()) {
        std::fs::create_dir_all(parent).with_context(|| format!("cannot create the folder {}", parent.display()))?;
    }
    persistence::atomic_write(path, text.as_bytes()).map_err(flat)
}
