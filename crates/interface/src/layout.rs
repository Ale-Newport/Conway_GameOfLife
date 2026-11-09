//! Layout math: the world viewport left free by the panels and the
//! dashboard card grid.

use bevy::camera::Viewport;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rendering::WorldCamera;

/// Geometry computed by the shell each frame.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct ShellLayout {
    /// Rectangle left free for the world by the panels, in window logical
    /// pixels (`None` on the dashboard).
    pub central: Option<Rect>,
}

/// Converts a logical rectangle to a physical camera viewport that lies
/// inside a window of `window_physical` pixels. Edges are rounded to the
/// nearest physical pixel so adjacent panels and the viewport do not leave a
/// gap. Returns `None` for an empty or degenerate result.
pub fn physical_viewport(logical: Rect, scale_factor: f32, window_physical: UVec2) -> Option<(UVec2, UVec2)> {
    if !(scale_factor.is_finite() && scale_factor > 0.0) || window_physical.x == 0 || window_physical.y == 0 {
        return None;
    }
    let to_px = |v: f32, max: u32| ((v * scale_factor).round().max(0.0) as u32).min(max);
    let x0 = to_px(logical.min.x, window_physical.x);
    let y0 = to_px(logical.min.y, window_physical.y);
    let x1 = to_px(logical.max.x, window_physical.x);
    let y1 = to_px(logical.max.y, window_physical.y);
    (x1 > x0 && y1 > y0).then(|| (UVec2::new(x0, y0), UVec2::new(x1 - x0, y1 - y0)))
}

/// Number of columns and card width for a grid of cards: as many columns as
/// fit at `min_card` width (at most `max_cols`, at least one), sharing the
/// width equally with `gap` between cards.
pub fn card_grid(available: f32, gap: f32, min_card: f32, max_cols: usize) -> (usize, f32) {
    let available = available.max(0.0);
    let max_cols = max_cols.max(1);
    let fit = ((available + gap) / (min_card + gap)).floor().max(1.0) as usize;
    let cols = fit.min(max_cols);
    let width = ((available - gap * (cols as f32 - 1.0)) / cols as f32).max(0.0);
    (cols, width)
}

/// Sets every [`WorldCamera`] viewport to the central rectangle. Runs after
/// the shell UI pass.
pub(crate) fn apply_world_viewports(
    layout: Res<ShellLayout>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut cameras: Query<&mut Camera, With<WorldCamera>>,
) {
    let Some(window) = window else { return };
    let viewport = layout
        .central
        .and_then(|rect| physical_viewport(rect, window.scale_factor(), window.physical_size()))
        .map(|(physical_position, physical_size)| Viewport { physical_position, physical_size, ..default() });
    let key = |v: &Option<Viewport>| v.as_ref().map(|v| (v.physical_position, v.physical_size));
    for mut camera in &mut cameras {
        // Only write on change so camera change detection stays meaningful.
        if key(&camera.viewport) != key(&viewport) {
            camera.viewport = viewport.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_scales_rounds_and_clamps() {
        let rect = Rect::new(240.0, 44.5, 1300.0, 900.0);
        let (pos, size) = physical_viewport(rect, 2.0, UVec2::new(3200, 2000)).unwrap();
        assert_eq!(pos, UVec2::new(480, 89));
        assert_eq!(size, UVec2::new(2120, 1711));
        // A rectangle reaching past the window is clamped to it.
        let (pos, size) = physical_viewport(Rect::new(-10.0, 0.0, 2000.0, 600.0), 1.0, UVec2::new(1600, 1000)).unwrap();
        assert_eq!((pos, size), (UVec2::ZERO, UVec2::new(1600, 600)));
        // Degenerate inputs.
        assert!(physical_viewport(Rect::new(10.0, 10.0, 10.2, 50.0), 1.0, UVec2::new(100, 100)).is_none());
        assert!(physical_viewport(rect, 0.0, UVec2::new(100, 100)).is_none());
        assert!(physical_viewport(rect, 1.0, UVec2::ZERO).is_none());
    }

    #[test]
    fn adjacent_rectangles_share_pixel_edges() {
        let sf = 1.5;
        let window = UVec2::new(2400, 1500);
        let (_, left_size) = physical_viewport(Rect::new(0.0, 0.0, 263.3, 100.0), sf, window).unwrap();
        let (right_pos, _) = physical_viewport(Rect::new(263.3, 0.0, 900.0, 100.0), sf, window).unwrap();
        assert_eq!(left_size.x, right_pos.x, "no gap or overlap between neighbours");
    }

    #[test]
    fn card_grid_columns() {
        assert_eq!(card_grid(1200.0, 20.0, 320.0, 3), (3, (1200.0 - 40.0) / 3.0));
        assert_eq!(card_grid(700.0, 20.0, 320.0, 3).0, 2);
        assert_eq!(card_grid(200.0, 20.0, 320.0, 3), (1, 200.0));
        assert_eq!(card_grid(5000.0, 20.0, 320.0, 3).0, 3);
        assert_eq!(card_grid(-5.0, 20.0, 320.0, 0), (1, 0.0));
    }
}
