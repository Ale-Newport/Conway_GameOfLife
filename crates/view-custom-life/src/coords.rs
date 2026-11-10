//! Coordinate mapping between the simulation (world units, `x` right, `y`
//! down, one unit = one field cell), Bevy's 2D world (y up, laid out by
//! [`GridSpace`]) and window points, plus entity picking and scale bars.

use bevy::math::{I64Vec2, Rect, Vec2};
use rendering::GridSpace;

/// Bevy world position of a simulation point.
pub fn sim_to_world(space: &GridSpace, p: Vec2) -> Vec2 {
    let origin = space.cell_corner_to_world(I64Vec2::ZERO);
    origin + Vec2::new(p.x, -p.y) * space.cell_size
}

/// Simulation point of a Bevy world position (inverse of [`sim_to_world`]).
pub fn world_to_sim(space: &GridSpace, world: Vec2) -> Vec2 {
    let (x, y) = space.world_to_cell_f64(world);
    Vec2::new(x as f32, y as f32)
}

/// Window position (logical pixels) of a Bevy world point seen by an
/// orthographic camera with `WindowSize` scaling (`scale` = world units per
/// logical pixel) centred on `camera` and drawing into `viewport`.
pub fn world_to_screen(world: Vec2, camera: Vec2, scale: f32, viewport: Rect) -> Vec2 {
    let c = viewport.center();
    let s = if scale > 0.0 && scale.is_finite() { scale } else { 1.0 };
    Vec2::new(c.x + (world.x - camera.x) / s, c.y - (world.y - camera.y) / s)
}

/// Inverse of [`world_to_screen`].
pub fn screen_to_world(screen: Vec2, camera: Vec2, scale: f32, viewport: Rect) -> Vec2 {
    let c = viewport.center();
    let s = if scale > 0.0 && scale.is_finite() { scale } else { 1.0 };
    Vec2::new(camera.x + (screen.x - c.x) * s, camera.y - (screen.y - c.y) * s)
}

/// Whether `p` lies inside the world rectangle `[0, width] × [0, height]`.
pub fn inside_world(p: Vec2, width: u32, height: u32) -> bool {
    p.is_finite() && p.x >= 0.0 && p.y >= 0.0 && p.x <= width as f32 && p.y <= height as f32
}

/// Index of the entity under `p`: among the discs (centre, diameter) that
/// contain `p` (radius at least `min_radius`, so tiny entities stay
/// clickable), the one whose centre is closest. Ties keep the first.
pub fn pick<I>(discs: I, p: Vec2, min_radius: f32) -> Option<usize>
where
    I: IntoIterator<Item = (Vec2, f32)>,
{
    let mut best: Option<(usize, f32)> = None;
    for (i, (centre, diameter)) in discs.into_iter().enumerate() {
        let r = (diameter * 0.5).max(min_radius);
        let d = centre.distance(p);
        if d <= r && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// A "nice" scale-bar length (1, 2 or 5 × 10ⁿ world units) close to
/// `target_px` pixels at `units_per_px`; returns the length in world units
/// and in pixels.
pub fn scale_bar(units_per_px: f32, target_px: f32) -> (f32, f32) {
    if !(units_per_px.is_finite() && units_per_px > 0.0 && target_px > 0.0) {
        return (0.0, 0.0);
    }
    let raw = units_per_px * target_px;
    let magnitude = 10f32.powf(raw.log10().floor());
    let normalised = raw / magnitude;
    let nice = if normalised < 1.5 {
        1.0
    } else if normalised < 3.5 {
        2.0
    } else if normalised < 7.5 {
        5.0
    } else {
        10.0
    };
    let units = nice * magnitude;
    (units, units / units_per_px)
}

/// Points spread on a sunflower (Vogel) spiral inside a disc of `radius`
/// around `centre`, clamped into the world: placement of several entities
/// spawned with one click.
pub fn scatter(centre: Vec2, radius: f32, count: usize, width: u32, height: u32) -> Vec<Vec2> {
    const GOLDEN_ANGLE: f32 = 2.399_963;
    let n = count.max(1) as f32;
    (0..count)
        .map(|i| {
            let r = if count == 1 { 0.0 } else { radius * ((i as f32 + 0.5) / n).sqrt() };
            let a = i as f32 * GOLDEN_ANGLE;
            let p = centre + Vec2::new(a.cos(), a.sin()) * r;
            Vec2::new(p.x.clamp(0.0, width as f32), p.y.clamp(0.0, height as f32))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_and_world_round_trip_with_y_flipped() {
        let space = GridSpace::new(2.0);
        let p = Vec2::new(3.5, 7.25);
        let w = sim_to_world(&space, p);
        assert_eq!(w, Vec2::new(7.0, -14.5));
        assert!(world_to_sim(&space, w).distance(p) < 1e-5);
        let anchored = GridSpace { cell_size: 1.0, anchor: I64Vec2::new(10, 4) };
        let w = sim_to_world(&anchored, Vec2::new(10.0, 4.0));
        assert_eq!(w, Vec2::ZERO);
        assert!(world_to_sim(&anchored, w).distance(Vec2::new(10.0, 4.0)) < 1e-5);
    }

    #[test]
    fn screen_mapping_is_centred_and_invertible() {
        let viewport = Rect::new(100.0, 50.0, 500.0, 350.0);
        let camera = Vec2::new(24.0, -24.0);
        assert_eq!(world_to_screen(camera, camera, 0.1, viewport), viewport.center());
        let s = world_to_screen(Vec2::new(25.0, -23.0), camera, 0.1, viewport);
        assert_eq!(s, viewport.center() + Vec2::new(10.0, -10.0));
        let back = screen_to_world(s, camera, 0.1, viewport);
        assert!(back.distance(Vec2::new(25.0, -23.0)) < 1e-4);
    }

    #[test]
    fn picking_prefers_the_closest_containing_disc() {
        let discs = [(Vec2::new(1.0, 1.0), 1.0), (Vec2::new(1.3, 1.0), 2.0), (Vec2::new(5.0, 5.0), 1.0)];
        assert_eq!(pick(discs, Vec2::new(1.2, 1.0), 0.0), Some(1));
        assert_eq!(pick(discs, Vec2::new(0.7, 1.0), 0.0), Some(0));
        assert_eq!(pick(discs, Vec2::new(3.0, 3.0), 0.0), None);
        // A tiny entity is still clickable with a minimum radius.
        assert_eq!(pick([(Vec2::ZERO, 0.05)], Vec2::new(0.3, 0.0), 0.4), Some(0));
    }

    #[test]
    fn scale_bars_use_round_lengths() {
        let (units, px) = scale_bar(0.1, 100.0);
        assert_eq!(units, 10.0);
        assert!((px - 100.0).abs() < 1e-3);
        let (units, px) = scale_bar(0.37, 100.0);
        assert_eq!(units, 50.0);
        assert!((px - 135.135).abs() < 0.01);
        assert_eq!(scale_bar(0.0, 100.0), (0.0, 0.0));
    }

    #[test]
    fn scatter_stays_inside_the_world_and_spreads() {
        let pts = scatter(Vec2::new(1.0, 1.0), 3.0, 12, 48, 48);
        assert_eq!(pts.len(), 12);
        assert!(pts.iter().all(|p| inside_world(*p, 48, 48)));
        assert_eq!(scatter(Vec2::new(5.0, 6.0), 3.0, 1, 48, 48), vec![Vec2::new(5.0, 6.0)]);
        let distinct = pts.windows(2).all(|w| w[0].distance(w[1]) > 1e-3);
        assert!(distinct);
    }

    #[test]
    fn world_bounds() {
        assert!(inside_world(Vec2::new(0.0, 48.0), 48, 48));
        assert!(!inside_world(Vec2::new(-0.1, 2.0), 48, 48));
        assert!(!inside_world(Vec2::new(f32::NAN, 2.0), 48, 48));
    }
}
