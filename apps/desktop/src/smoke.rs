//! `--smoke-screenshot`: render a few frames, save a screenshot and exit.
//! Used for automated visual checks of the user interface.

use std::path::PathBuf;

use bevy::prelude::*;
use interface::{ScreenshotFinished, ScreenshotTracker};

use crate::cli::SmokeCapture;

/// Frames to wait for the screenshot before giving up.
const TIMEOUT_FRAMES: u32 = 600;

/// Captures the window once after `frames` frames, then exits with success
/// if the file was written.
pub struct SmokeScreenshotPlugin(pub SmokeCapture);

#[derive(Resource)]
struct SmokeState {
    path: PathBuf,
    frames: u32,
    elapsed: u32,
    requested: bool,
}

impl Plugin for SmokeScreenshotPlugin {
    fn build(&self, app: &mut App) {
        let path = std::path::absolute(&self.0.path).unwrap_or_else(|_| self.0.path.clone());
        app.insert_resource(SmokeState { path, frames: self.0.frames, elapsed: 0, requested: false })
            .add_systems(Update, smoke_capture);
    }
}

fn smoke_capture(
    mut state: ResMut<SmokeState>,
    mut commands: Commands,
    mut tracker: ResMut<ScreenshotTracker>,
    mut finished: MessageReader<ScreenshotFinished>,
    mut exit: MessageWriter<AppExit>,
) {
    state.elapsed += 1;
    if !state.requested && state.elapsed >= state.frames {
        if let Some(parent) = state.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            error!("smoke screenshot: cannot create `{}`: {e}", parent.display());
            exit.write(AppExit::error());
            return;
        }
        tracker.capture(&mut commands, state.path.clone());
        state.requested = true;
        info!("smoke screenshot requested after {} frames: {}", state.elapsed, state.path.display());
    }
    for message in finished.read() {
        if message.path == state.path {
            if message.saved {
                info!("smoke screenshot saved: {}", message.path.display());
                exit.write(AppExit::Success);
            } else {
                error!("smoke screenshot was not written: {}", message.path.display());
                exit.write(AppExit::error());
            }
        }
    }
    if state.elapsed > state.frames + TIMEOUT_FRAMES {
        error!("smoke screenshot timed out after {} frames", state.elapsed);
        exit.write(AppExit::error());
    }
}
