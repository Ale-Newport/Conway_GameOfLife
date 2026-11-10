//! Bevy rendering building blocks for Emergence Lab.
//!
//! Shared world-input contract (stable; other crates depend on it):
//! * [`WorldCamera`] marks cameras that render the simulated world. The
//!   application shell resizes their viewport to the area left free by panels.
//! * [`WorldInput`] tells world-interaction systems (camera controllers,
//!   editing tools, picking) whether the pointer and keyboard currently belong
//!   to the world rather than to the user interface.
//! * [`shortcut_modifier_held`]: camera key controls ignore keys pressed with
//!   Ctrl, Cmd (Super) or Alt, which form shortcuts (Cmd+S, Ctrl+Z...).
//!
//! Building blocks (each a plugin plus plain-data helpers that are unit tested):
//! * [`grid`] — styled 2D grid renderer driven by RGBA8 data frames
//!   ([`GridRenderPlugin`], [`GridSurface`], [`GridFrame`], [`GridStyle`]).
//! * [`palette`] — state palettes (Life, Generations, Wireworld, Brian's Brain)
//!   and continuous colormaps (viridis, inferno, magma, plasma, turbo-like,
//!   Lenia blue).
//! * [`camera2d`] — pan/zoom controller for grids and [`VisibleCells`].
//! * [`orbit`] — 3D camera rigs on top of `bevy_panorbit_camera` (aerial,
//!   isometric, free orbit, strategic, follow target).
//! * [`voxel`] — face-culled voxel meshing with chunked rebuilds and slicing.
//! * [`terrain`] and [`water`] — heightmap meshes with biome colours and LOD,
//!   water surface meshes and an animated WESL water material.
//! * [`particles`] — CPU particle emitters rendered as batched billboards.
//! * [`standalone`] — `WorldInput` for apps without the shell and the
//!   `EMERGENCE_SMOKE_SCREENSHOT` capture plugin.
//!
//! Shaders live in `assets/shaders/` and are embedded into the binary at
//! compile time, so they load regardless of the application's asset root.

pub mod camera2d;
pub mod grid;
pub mod orbit;
pub mod palette;
pub mod particles;
pub mod standalone;
pub mod terrain;
pub mod voxel;
pub mod water;

pub use camera2d::{GridCameraController, GridCameraPlugin, GridCameraSystems, cursor_to_cell};
pub use grid::{
    GridAppearance, GridFrame, GridFrameError, GridMaterial, GridRenderPlugin, GridSpace, GridStyle, GridSurface,
    Texel, VisibleCells,
};
pub use orbit::{FollowSubject, FollowTarget, OrbitCameraPlugin, OrbitCameraRig, OrbitPreset, focus_on};
pub use palette::{Colormap, GridPalette, PaletteError};
pub use particles::{ParticleEmitter, ParticlePlugin, ParticlePreset, ParticleWind};
pub use standalone::{SmokeScreenshotPlugin, StandaloneWorldInputPlugin};
pub use voxel::{Voxel, VoxelPalette, VoxelPlugin, VoxelVolume};
pub use water::{WaterMaterial, WaterPlugin};

use bevy::prelude::*;

/// Marker for cameras that render the simulated world (not the UI).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct WorldCamera;

/// Whether world-interaction systems may consume input this frame. Written by
/// the application shell every frame before `Update` systems read it.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct WorldInput {
    /// The pointer is over the world viewport and not over any UI element.
    pub pointer: bool,
    /// No text field or widget has keyboard focus.
    pub keyboard: bool,
    /// Cursor position in window logical pixels, if inside the window.
    pub cursor: Option<Vec2>,
    /// World viewport rectangle in window logical pixels.
    pub viewport: Option<Rect>,
}

impl WorldInput {
    /// Cursor position relative to the world viewport's top-left corner.
    pub fn cursor_in_viewport(&self) -> Option<Vec2> {
        let cursor = self.cursor?;
        let viewport = self.viewport?;
        viewport.contains(cursor).then(|| cursor - viewport.min)
    }
}

/// Whether Ctrl, Cmd (Super) or Alt is held. A key pressed with one of them
/// is part of a shortcut (Cmd+S, Cmd+A, Ctrl+Z...), so camera key controls
/// ignore it.
pub fn shortcut_modifier_held(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::AltLeft,
        KeyCode::AltRight,
    ])
}

/// Registers the shared world-input resource.
pub struct WorldInputPlugin;

impl Plugin for WorldInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldInput>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::ParticleBlend;
    use crate::voxel::{SliceAxis, SliceKeep, VoxelSlice};

    fn round_trip<T>(value: T, json: &str)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let text = serde_json::to_string(&value).expect("serializable");
        assert_eq!(text, json);
        assert_eq!(serde_json::from_str::<T>(&text).expect("deserializable"), value);
    }

    #[test]
    fn viewer_settings_serialize_in_snake_case() {
        round_trip(GridStyle::PixelArt, "\"pixel_art\"");
        round_trip(Colormap::TurboLike, "\"turbo_like\"");
        round_trip(OrbitPreset::FreeOrbit, "\"free_orbit\"");
        round_trip(ParticlePreset::Embers, "\"embers\"");
        round_trip(ParticleBlend::Additive, "\"additive\"");
        round_trip(
            VoxelSlice { axis: SliceAxis::Y, position: -3, keep: SliceKeep::Below },
            r#"{"axis":"y","position":-3,"keep":"below"}"#,
        );
        assert!(serde_json::from_str::<GridStyle>("\"sparkles\"").is_err(), "unknown styles are rejected");
    }

    #[test]
    fn cursor_in_viewport_is_relative() {
        let input = WorldInput {
            pointer: true,
            keyboard: true,
            cursor: Some(Vec2::new(150.0, 80.0)),
            viewport: Some(Rect::new(100.0, 50.0, 400.0, 300.0)),
        };
        assert_eq!(input.cursor_in_viewport(), Some(Vec2::new(50.0, 30.0)));
        let outside = WorldInput { cursor: Some(Vec2::new(10.0, 10.0)), ..input };
        assert_eq!(outside.cursor_in_viewport(), None);
    }
}
