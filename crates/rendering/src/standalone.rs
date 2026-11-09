//! Helpers for running rendering scenes without the application shell:
//! a [`WorldInput`] writer for windows without UI panels, and the
//! `EMERGENCE_SMOKE_SCREENSHOT` capture-and-exit plugin used by examples and
//! smoke tests.

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

use crate::{WorldCamera, WorldInput};

/// Writes [`WorldInput`] for apps without the egui shell: the whole primary
/// window (or the first world camera's viewport) belongs to the world. The
/// application shell writes `WorldInput` itself and must not add this plugin.
pub struct StandaloneWorldInputPlugin;

impl Plugin for StandaloneWorldInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldInput>().add_systems(PreUpdate, write_standalone_world_input);
    }
}

fn write_standalone_world_input(
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    cameras: Query<&Camera, With<WorldCamera>>,
    mut input: ResMut<WorldInput>,
) {
    let Some(window) = window else {
        *input = WorldInput::default();
        return;
    };
    let viewport = cameras
        .iter()
        .find(|c| c.is_active)
        .and_then(Camera::logical_viewport_rect)
        .unwrap_or_else(|| Rect::new(0.0, 0.0, window.width(), window.height()));
    let cursor = window.cursor_position();
    *input = WorldInput {
        pointer: cursor.is_some_and(|c| viewport.contains(c)),
        keyboard: window.focused,
        cursor,
        viewport: Some(viewport),
    };
}

/// Environment variable naming the PNG to capture before exiting.
pub const SMOKE_SCREENSHOT_ENV: &str = "EMERGENCE_SMOKE_SCREENSHOT";

/// When [`SMOKE_SCREENSHOT_ENV`] is set, captures the primary window after
/// `frames` frames, waits for the PNG to be written and exits. Does nothing
/// when the variable is unset.
pub struct SmokeScreenshotPlugin {
    /// Frames to render before capturing (lets assets and smoothing settle).
    pub frames: u32,
}

impl Default for SmokeScreenshotPlugin {
    fn default() -> Self {
        Self { frames: 60 }
    }
}

#[derive(Resource, Debug)]
struct SmokeCapture {
    path: PathBuf,
    capture_at: u32,
    requested_at: Option<u32>,
    seen_at: Option<u32>,
    frame: u32,
}

/// Frames to wait after the file appears so the encoder can finish writing.
const SETTLE_FRAMES: u32 = 5;
/// Give up if the screenshot has not appeared after this many frames.
const TIMEOUT_FRAMES: u32 = 600;

impl Plugin for SmokeScreenshotPlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = std::env::var_os(SMOKE_SCREENSHOT_ENV).filter(|p| !p.is_empty()) else { return };
        let path = PathBuf::from(path);
        if path.exists()
            && let Err(err) = std::fs::remove_file(&path)
        {
            warn!("could not remove stale screenshot {}: {err}", path.display());
        }
        app.insert_resource(SmokeCapture {
            path,
            capture_at: self.frames.max(1),
            requested_at: None,
            seen_at: None,
            frame: 0,
        })
        .add_systems(Last, drive_smoke_capture);
    }
}

fn drive_smoke_capture(mut commands: Commands, mut capture: ResMut<SmokeCapture>, mut exit: MessageWriter<AppExit>) {
    capture.frame += 1;
    let frame = capture.frame;
    match capture.requested_at {
        None if frame >= capture.capture_at => {
            info!("smoke test: capturing {}", capture.path.display());
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(capture.path.clone()));
            capture.requested_at = Some(frame);
        }
        Some(requested) => {
            let written = std::fs::metadata(&capture.path).is_ok_and(|m| m.len() > 0);
            if written && capture.seen_at.is_none() {
                capture.seen_at = Some(frame);
            }
            if capture.seen_at.is_some_and(|seen| frame >= seen + SETTLE_FRAMES) {
                info!("smoke test: screenshot written, exiting");
                exit.write(AppExit::Success);
            } else if frame >= requested + TIMEOUT_FRAMES {
                error!("smoke test: screenshot {} was not written", capture.path.display());
                exit.write(AppExit::error());
            }
        }
        None => {}
    }
}
