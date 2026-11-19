//! `--smoke-screenshot`: render a few frames, save a screenshot and exit.
//! Used for automated visual checks of the user interface.
//!
//! With `--smoke-sequence <N>` it saves `N` numbered screenshots instead
//! (`<stem>-0001.png`, `<stem>-0002.png`, ... next to the given path), one
//! every `--smoke-every` frames, which is how the README animation is
//! recorded (`scripts/record_demo.sh`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use interface::{ScreenshotFinished, ScreenshotTracker};

use crate::cli::{Sequence, SmokeCapture};

/// Frames to wait for outstanding screenshots before giving up.
const TIMEOUT_FRAMES: u32 = 600;

/// Captures the window after `frames` frames (once, or as a numbered
/// sequence), then exits with success if every file was written.
pub struct SmokeScreenshotPlugin(pub SmokeCapture);

#[derive(Resource)]
struct SmokeState {
    path: PathBuf,
    frames: u32,
    sequence: Sequence,
    elapsed: u32,
    requested: u32,
    pending: HashSet<PathBuf>,
    failed: bool,
}

impl Plugin for SmokeScreenshotPlugin {
    fn build(&self, app: &mut App) {
        let path = std::path::absolute(&self.0.path).unwrap_or_else(|_| self.0.path.clone());
        app.insert_resource(SmokeState {
            path,
            frames: self.0.frames,
            sequence: self.0.sequence.unwrap_or(Sequence { count: 1, every: 1 }),
            elapsed: 0,
            requested: 0,
            pending: HashSet::new(),
            failed: false,
        })
        .add_systems(Update, smoke_capture);
    }
}

/// File of the `index`-th screenshot (0-based): the path itself for a single
/// screenshot, `<stem>-NNNN.png` for a sequence.
fn frame_path(path: &Path, sequence: Option<Sequence>, index: u32) -> PathBuf {
    if sequence.is_none() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("png");
    path.with_file_name(format!("{stem}-{:04}.{ext}", index + 1))
}

fn smoke_capture(
    mut state: ResMut<SmokeState>,
    mut commands: Commands,
    mut tracker: ResMut<ScreenshotTracker>,
    mut finished: MessageReader<ScreenshotFinished>,
    mut exit: MessageWriter<AppExit>,
) {
    state.elapsed += 1;
    let is_sequence = state.sequence.count > 1 || state.sequence.every > 1;
    let sequence = is_sequence.then_some(state.sequence);
    let due = state.frames + state.requested * state.sequence.every;
    if state.requested < state.sequence.count && state.elapsed >= due {
        if state.requested == 0
            && let Some(parent) = state.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            error!("smoke screenshot: cannot create `{}`: {e}", parent.display());
            exit.write(AppExit::error());
            return;
        }
        let path = frame_path(&state.path, sequence, state.requested);
        tracker.capture(&mut commands, path.clone());
        state.pending.insert(path.clone());
        state.requested += 1;
        if sequence.is_none() {
            info!("smoke screenshot requested after {} frames: {}", state.elapsed, path.display());
        }
    }
    for message in finished.read() {
        if state.pending.remove(&message.path) {
            if message.saved {
                if sequence.is_none() {
                    info!("smoke screenshot saved: {}", message.path.display());
                }
            } else {
                error!("smoke screenshot was not written: {}", message.path.display());
                state.failed = true;
            }
        }
    }
    if state.requested == state.sequence.count && state.pending.is_empty() {
        if state.failed {
            exit.write(AppExit::error());
        } else {
            if sequence.is_some() {
                info!("smoke sequence saved: {} screenshots", state.requested);
            }
            exit.write(AppExit::Success);
        }
        return;
    }
    let last_due = state.frames + state.sequence.count.saturating_sub(1) * state.sequence.every;
    if state.elapsed > last_due + TIMEOUT_FRAMES {
        error!("smoke screenshot timed out after {} frames", state.elapsed);
        exit.write(AppExit::error());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_files_are_numbered_next_to_the_path() {
        let path = Path::new("rec/conway.png");
        assert_eq!(frame_path(path, None, 0), PathBuf::from("rec/conway.png"));
        let seq = Some(Sequence { count: 3, every: 2 });
        assert_eq!(frame_path(path, seq, 0), PathBuf::from("rec/conway-0001.png"));
        assert_eq!(frame_path(path, seq, 41), PathBuf::from("rec/conway-0042.png"));
    }
}
