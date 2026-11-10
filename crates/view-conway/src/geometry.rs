//! Pure geometry of the mode: visible-region view requests with level of
//! detail, framing, selection and stamp placement, pattern transforms,
//! ghost and thumbnail images, scale bars and Hashlife jump plans.

use bevy::math::{I64Vec2, Vec2};
use cellular_automata::life::MAX_GENERATIONS_PER_TICK;
use cellular_automata::view::{MAX_LOG2_CELLS_PER_TEXEL, MAX_VIEW_SIDE};
use cellular_automata::{CellRect, GridViewRequest, Rotation, World};
use patterns::Pattern;
use rendering::grid::VisibleCells;

/// Largest ghost (stamp preview) image side in texels.
pub const MAX_GHOST_SIDE: u64 = 2048;
/// Largest Hashlife jump offered, as a power of two.
pub const MAX_JUMP_LOG2: u32 = 40;

/// A view request and what it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestPlan {
    /// Request sent to the runner.
    pub request: GridViewRequest,
    /// Level of detail the request was budgeted for (`log2(cells_per_texel)`).
    pub lod_log2: u32,
    /// Requested cell rectangle (inclusive).
    pub covered: CellRect,
}

/// The visible cells as a rectangle.
pub fn visible_rect(visible: &VisibleCells) -> CellRect {
    CellRect::new(visible.min_x, visible.min_y, visible.max_x, visible.max_y)
}

/// Number of `2^log2`-aligned blocks touched by `[a, b]` (as the model counts them).
pub fn blocks(a: i64, b: i64, log2: u32) -> i128 {
    (i128::from(b) >> log2) - (i128::from(a) >> log2) + 1
}

fn grow(rect: CellRect, margin: i64) -> CellRect {
    CellRect {
        x0: rect.x0.saturating_sub(margin),
        y0: rect.y0.saturating_sub(margin),
        x1: rect.x1.saturating_add(margin),
        y1: rect.y1.saturating_add(margin),
    }
}

fn contains_rect(outer: &CellRect, inner: &CellRect) -> bool {
    outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
}

/// Plans the view request for the visible region: the visible cells grown
/// by a margin (an eighth of the larger side, at least two texels), clipped
/// to a finite world, with a texel budget that gives one texel per
/// `2^lod` cells, where `2^lod` is the smallest power of two with at least
/// one logical pixel per texel. Returns `None` when `last` still covers the
/// visible cells at the same level of detail (no new request is needed) or
/// when nothing of a finite world is visible.
pub fn plan_view_request(
    visible: &VisibleCells,
    bounds: Option<CellRect>,
    last: Option<&RequestPlan>,
) -> Option<RequestPlan> {
    let mut seen = visible_rect(visible);
    if let Some(b) = bounds {
        seen = seen.intersect(&b)?;
    }
    let lod_log2 = visible.lod_log2().min(MAX_LOG2_CELLS_PER_TEXEL);
    if let Some(last) = last
        && last.lod_log2 == lod_log2
        && contains_rect(&last.covered, &seen)
    {
        return None;
    }
    let extent = visible.width().max(visible.height()).min(1 << 50) as i64;
    let texel = 1i64 << lod_log2;
    let margin = (extent / 8).max(2 * texel);
    let mut covered = grow(visible_rect(visible), margin);
    if let Some(b) = bounds {
        covered = covered.intersect(&b)?;
    }
    let budget = |a: i64, b: i64| blocks(a, b, lod_log2).clamp(1, i128::from(MAX_VIEW_SIDE)) as u32;
    let request = GridViewRequest {
        x0: covered.x0,
        y0: covered.y0,
        x1: covered.x1,
        y1: covered.y1,
        max_width: budget(covered.x0, covered.x1),
        max_height: budget(covered.y0, covered.y1),
    };
    Some(RequestPlan { request, lod_log2, covered })
}

/// The rectangle the camera frames with "Fit": a finite world entirely; on
/// the infinite plane the live cells (at least 64 × 64 cells around them,
/// so a lone glider is not magnified absurdly), or the home region when the
/// plane is empty.
pub fn framing_rect(world: World, bbox: Option<CellRect>) -> CellRect {
    if world.is_finite() {
        return world.home();
    }
    let Some(b) = bbox else { return world.home() };
    const MIN_SIDE: u64 = 64;
    let pad = |side: u64| -> i64 { (MIN_SIDE.saturating_sub(side) / 2) as i64 };
    let (px, py) = (pad(b.width()), pad(b.height()));
    CellRect {
        x0: b.x0.saturating_sub(px),
        y0: b.y0.saturating_sub(py),
        x1: b.x1.saturating_add(px),
        y1: b.y1.saturating_add(py),
    }
}

/// The cell rectangle to hand to `GridCameraController::fit_to_rect` (with
/// margin 0) so that `content` is framed, with `margin` (a fraction of the
/// region on each side), in the part of a `viewport`-sized view (logical
/// pixels) that an overlay of size `obstacle` in its top-left corner does
/// not cover: the strip to its right or the strip below it, whichever shows
/// the content larger. Without an obstacle (or one too large to avoid) the
/// whole viewport is used.
pub fn fit_avoiding(content: CellRect, viewport: Vec2, obstacle: Option<Vec2>, margin: f32) -> CellRect {
    let v = viewport.max(Vec2::ONE).as_dvec2();
    let margin = f64::from(margin.clamp(0.0, 0.45));
    let (cw, ch) = (content.width() as f64, content.height() as f64);
    // Regions as (left, top, width, height) in pixels.
    let mut regions = vec![(0.0, 0.0, v.x, v.y)];
    if let Some(o) = obstacle.map(|o| o.as_dvec2()) {
        let right = (o.x, 0.0, v.x - o.x, v.y);
        let below = (0.0, o.y, v.x, v.y - o.y);
        let usable: Vec<_> = [right, below].into_iter().filter(|r| r.2 >= v.x * 0.4 && r.3 >= v.y * 0.4).collect();
        if !usable.is_empty() {
            regions = usable;
        }
    }
    // Cells per pixel needed to fit the content into a region.
    let scale = |r: &(f64, f64, f64, f64)| {
        (cw / (r.2 * (1.0 - 2.0 * margin)).max(1.0)).max(ch / (r.3 * (1.0 - 2.0 * margin)).max(1.0))
    };
    let best = regions.into_iter().min_by(|a, b| scale(a).total_cmp(&scale(b))).unwrap_or((0.0, 0.0, v.x, v.y));
    let s = scale(&best);
    // Centre of the view (in cells) that puts the content's centre at the
    // region's centre; cell y and screen y both grow downwards.
    let cx = content.x0 as f64 + cw / 2.0 - (best.0 + best.2 / 2.0 - v.x / 2.0) * s;
    let cy = content.y0 as f64 + ch / 2.0 - (best.1 + best.3 / 2.0 - v.y / 2.0) * s;
    let to_cell = |x: f64| x.clamp(i64::MIN as f64, i64::MAX as f64) as i64;
    let (hx, hy) = (v.x / 2.0 * s, v.y / 2.0 * s);
    let x0 = to_cell((cx - hx).round());
    let y0 = to_cell((cy - hy).round());
    let x1 = to_cell((cx + hx).round()).saturating_sub(1).max(x0);
    let y1 = to_cell((cy + hy).round()).saturating_sub(1).max(y0);
    CellRect { x0, y0, x1, y1 }
}

/// Size of a `width × height` box after a rotation (odd quarter turns swap).
pub fn rotated_size(width: u64, height: u64, rotation: Rotation) -> (u64, u64) {
    if rotation.quarter_turns() % 2 == 1 { (height, width) } else { (width, height) }
}

/// Top-left cell that centres a `size` stamp on `cursor`.
pub fn stamp_origin(cursor: I64Vec2, size: (u64, u64)) -> I64Vec2 {
    let half = |s: u64| (s / 2).min(i64::MAX as u64) as i64;
    I64Vec2::new(cursor.x.saturating_sub(half(size.0)), cursor.y.saturating_sub(half(size.1)))
}

/// Next clockwise (`cw`) or counter-clockwise quarter turn.
pub fn turn(rotation: Rotation, cw: bool) -> Rotation {
    const ORDER: [Rotation; 4] = [Rotation::None, Rotation::Cw90, Rotation::Half, Rotation::Ccw90];
    let i = rotation.quarter_turns() as usize;
    ORDER[if cw { (i + 1) % 4 } else { (i + 3) % 4 }]
}

/// Degrees of a rotation (clockwise).
pub fn rotation_degrees(rotation: Rotation) -> u32 {
    u32::from(rotation.quarter_turns()) * 90
}

/// The pattern as `LifeCommand::Paste` places it: mirrored left-right first
/// (when `mirror`), then rotated clockwise, the top-left corner of the
/// bounding box fixed (exactly `cellular_automata::place_pattern`).
pub fn transform_pattern(pattern: &Pattern, rotation: Rotation, mirror: bool) -> Pattern {
    let mut p = if mirror { pattern.flip_horizontal() } else { pattern.clone() };
    for _ in 0..rotation.quarter_turns() {
        p = p.rotate_cw();
    }
    p
}

/// The rectangle a selection occupies after `RotateRegion` (top-left fixed,
/// sides swapped for odd quarter turns); `None` if it would leave the plane.
pub fn rotated_selection(rect: CellRect, rotation: Rotation) -> Option<CellRect> {
    let (w, h) = rotated_size(rect.width(), rect.height(), rotation);
    CellRect::from_origin_size(rect.x0, rect.y0, w, h)
}

/// A rectangle moved by `(dx, dy)`, or `None` if it would leave the plane.
pub fn moved_rect(rect: CellRect, dx: i64, dy: i64) -> Option<CellRect> {
    Some(CellRect {
        x0: rect.x0.checked_add(dx)?,
        y0: rect.y0.checked_add(dy)?,
        x1: rect.x1.checked_add(dx)?,
        y1: rect.y1.checked_add(dy)?,
    })
}

/// Selection rectangle clipped to a finite world (unchanged on the plane).
pub fn clip_to_world(rect: CellRect, world: World) -> Option<CellRect> {
    world.clip(&rect)
}

/// RGBA image (row 0 at the top) of a pattern's bounding box: live cells in
/// `color`, dead cells transparent. `None` for an empty pattern or one wider
/// or taller than [`MAX_GHOST_SIDE`].
pub fn ghost_rgba(pattern: &Pattern, color: [u8; 4]) -> Option<(u32, u32, Vec<u8>)> {
    let (x0, y0, _, _) = pattern.bounding_box()?;
    let (w, h) = (pattern.width(), pattern.height());
    if w > MAX_GHOST_SIDE || h > MAX_GHOST_SIDE {
        return None;
    }
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (x, y) in pattern.live_cells() {
        let i = (((y - y0) as u64 * w + (x - x0) as u64) * 4) as usize;
        rgba[i..i + 4].copy_from_slice(&color);
    }
    Some((w as u32, h as u32, rgba))
}

/// Thumbnail of a pattern (unmultiplied sRGBA, row 0 at the top) at most
/// `max_side` pixels per side. Small patterns are magnified to whole-pixel
/// cells (with a one-pixel gap between cells of four pixels or more, so
/// single cells stay distinguishable); large ones are reduced so that each
/// pixel covers `f × f` cells and shows `fg` with an opacity that grows
/// with the fraction of live cells (a single live cell still shows at 35 %,
/// so sparse patterns stay visible). Empty patterns give a single `bg` pixel.
pub fn thumbnail_rgba(pattern: &Pattern, max_side: u64, fg: [u8; 3], bg: [u8; 3]) -> (usize, usize, Vec<u8>) {
    let max_side = max_side.max(1);
    let Some((x0, y0, _, _)) = pattern.bounding_box() else {
        return (1, 1, vec![bg[0], bg[1], bg[2], 255]);
    };
    let (w, h) = (pattern.width(), pattern.height());
    let side = w.max(h);
    if side <= max_side {
        let s = (max_side / side).clamp(1, 8) as usize;
        let gap = usize::from(s >= 4);
        let (tw, th) = (w as usize * s, h as usize * s);
        let mut rgba: Vec<u8> = std::iter::repeat_n([bg[0], bg[1], bg[2], 255], tw * th).flatten().collect();
        for (x, y) in pattern.live_cells() {
            let (cx, cy) = ((x - x0) as usize * s, (y - y0) as usize * s);
            for py in cy..cy + s - gap {
                for px in cx..cx + s - gap {
                    let i = (py * tw + px) * 4;
                    rgba[i..i + 3].copy_from_slice(&fg);
                }
            }
        }
        return (tw, th, rgba);
    }
    let f = side.div_ceil(max_side);
    let (tw, th) = (w.div_ceil(f) as usize, h.div_ceil(f) as usize);
    let mut counts = vec![0u32; tw * th];
    for (x, y) in pattern.live_cells() {
        let tx = ((x - x0) as u64 / f) as usize;
        let ty = ((y - y0) as u64 / f) as usize;
        counts[ty * tw + tx] += 1;
    }
    let area = (f * f) as f32;
    let mut rgba = Vec::with_capacity(tw * th * 4);
    for &c in &counts {
        let t = if c == 0 { 0.0 } else { 0.35 + 0.65 * (c as f32 / area).sqrt() };
        let mix = |a: u8, b: u8| (f32::from(a) * (1.0 - t) + f32::from(b) * t).round() as u8;
        rgba.extend_from_slice(&[mix(bg[0], fg[0]), mix(bg[1], fg[1]), mix(bg[2], fg[2]), 255]);
    }
    (tw, th, rgba)
}

/// A "nice" scale-bar length (1, 2 or 5 × 10^n cells, at least one cell)
/// close to `target_px` logical pixels, and its length in pixels.
pub fn scale_bar(cells_per_px: f32, target_px: f32) -> Option<(f64, f32)> {
    if !(cells_per_px.is_finite() && cells_per_px > 0.0 && target_px > 0.0) {
        return None;
    }
    let raw = f64::from(target_px) * f64::from(cells_per_px);
    let base = 10f64.powf(raw.log10().floor());
    let m = raw / base;
    let nice = if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    };
    let cells = (nice * base).max(1.0).round();
    Some((cells, (cells / f64::from(cells_per_px)) as f32))
}

/// How a jump of `2^k` generations is run: generations per tick (at most
/// the model's 2^30) and the number of ticks.
pub fn jump_plan(k: u32) -> (u64, u64) {
    let k = k.min(63);
    let per_tick = k.min(MAX_GENERATIONS_PER_TICK.trailing_zeros());
    (1u64 << per_tick, 1u64 << (k - per_tick))
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::Topology;

    fn visible(min_x: i64, min_y: i64, max_x: i64, max_y: i64, cpp: f32) -> VisibleCells {
        VisibleCells { min_x, min_y, max_x, max_y, cells_per_pixel: cpp }
    }

    #[test]
    fn zoomed_in_requests_exact_cells_with_a_margin() {
        let v = visible(0, 0, 99, 49, 0.25);
        let plan = plan_view_request(&v, None, None).expect("first request");
        assert_eq!(plan.lod_log2, 0);
        // Margin = max(100 / 8, 2) = 12 cells on every side.
        assert_eq!(plan.covered, CellRect::new(-12, -12, 111, 61));
        assert_eq!((plan.request.max_width, plan.request.max_height), (124, 74));
        // Panning inside the margin needs no new request.
        assert!(plan_view_request(&visible(5, 3, 104, 52, 0.25), None, Some(&plan)).is_none());
        // Leaving it does.
        assert!(plan_view_request(&visible(20, 0, 119, 49, 0.25), None, Some(&plan)).is_some());
        // So does a change of the level of detail.
        assert!(plan_view_request(&visible(0, 0, 99, 49, 3.0), None, Some(&plan)).is_some());
    }

    #[test]
    fn zoomed_out_requests_budget_one_texel_per_pixel() {
        // 4096 x 2048 cells on 1024 x 512 px: 4 cells per pixel -> k = 4.
        let v = visible(-2048, -1024, 2047, 1023, 4.0);
        let plan = plan_view_request(&v, None, None).expect("request");
        assert_eq!(plan.lod_log2, 2);
        let r = plan.request;
        assert_eq!(i128::from(r.max_width), blocks(r.x0, r.x1, 2));
        assert!(r.max_width <= 1024 + 1024 / 4 + 2, "about the viewport plus margins: {}", r.max_width);
        // The model's planner picks exactly this level of detail.
        let grid = cellular_automata::view::TexelGrid::plan(&r, None);
        assert_eq!(grid.log2_k, 2);
    }

    #[test]
    fn finite_worlds_are_clipped() {
        let bounds = Some(CellRect::new(0, 0, 63, 63));
        let plan = plan_view_request(&visible(-100, -100, 100, 100, 1.0), bounds, None).expect("visible");
        assert_eq!(plan.covered, CellRect::new(0, 0, 63, 63));
        assert!(plan_view_request(&visible(200, 200, 300, 300, 1.0), bounds, None).is_none(), "world not visible");
        // Huge coordinates saturate instead of overflowing.
        let far = visible(i64::MAX - 10, 0, i64::MAX, 10, 0.5);
        assert!(plan_view_request(&far, None, None).is_some());
    }

    #[test]
    fn framing() {
        let torus = World::new(Topology::Torus, 300, 200).unwrap();
        assert_eq!(framing_rect(torus, Some(CellRect::new(5, 5, 6, 6))), CellRect::new(0, 0, 299, 199));
        let plane = World::new(Topology::Infinite, 256, 192).unwrap();
        assert_eq!(framing_rect(plane, None), CellRect::new(0, 0, 255, 191));
        let glider = framing_rect(plane, Some(CellRect::new(10, 10, 12, 12)));
        assert_eq!((glider.width(), glider.height()), (63, 63));
        assert!(glider.contains(10, 10) && glider.contains(12, 12));
        let big = CellRect::new(-500, 0, 500, 99);
        assert_eq!(framing_rect(plane, Some(big)), big, "large patterns are framed as they are");
    }

    /// Where `fit_rect` (centre of the rectangle, scale fitting it in the
    /// viewport) puts the corners of `content`, in viewport pixels.
    fn placed(e: CellRect, viewport: Vec2, content: CellRect) -> (Vec2, Vec2) {
        let (w, h) = ((e.x1 + 1 - e.x0) as f32, (e.y1 + 1 - e.y0) as f32);
        let s = (w / viewport.x).max(h / viewport.y);
        let centre = Vec2::new(e.x0 as f32 + w / 2.0, e.y0 as f32 + h / 2.0);
        let px = |x: i64, y: i64| (Vec2::new(x as f32, y as f32) - centre) / s + viewport / 2.0;
        (px(content.x0, content.y0), px(content.x1 + 1, content.y1 + 1))
    }

    #[test]
    fn framing_avoids_the_overlay() {
        let v = Vec2::new(1200.0, 1000.0);
        let square = CellRect::new(0, 0, 99, 99);
        assert_eq!(fit_avoiding(square, Vec2::splat(1000.0), None, 0.0), square, "plain fit");
        // A square goes to the right of a 300 x 250 overlay (larger than below it).
        let obstacle = Some(Vec2::new(300.0, 250.0));
        let (a, b) = placed(fit_avoiding(square, v, obstacle, 0.0), v, square);
        assert!(a.x >= 298.0 && b.x <= 1201.0 && a.y >= -1.0 && b.y <= 1001.0, "{a} {b}");
        assert!((b.x - a.x) > 880.0, "fills the strip: {a} {b}");
        // A wide pattern goes below it.
        let wide = CellRect::new(-86, 10, 85, 73);
        let (a, b) = placed(fit_avoiding(wide, v, obstacle, 0.08), v, wide);
        assert!(a.y >= 249.0 && b.y <= 1001.0 && a.x >= 0.0 && b.x <= 1200.0, "{a} {b}");
        // An overlay too large to avoid is ignored.
        let (a, b) = placed(fit_avoiding(square, v, Some(Vec2::new(1000.0, 900.0)), 0.0), v, square);
        assert!(a.y.abs() < 2.0 && (b.y - 1000.0).abs() < 2.0, "{a} {b}");
        // Huge coordinates stay finite.
        let far = CellRect::new(1 << 40, 1 << 40, (1 << 40) + 1_000_000, (1 << 40) + 1_000);
        let e = fit_avoiding(far, v, obstacle, 0.08);
        assert!(e.x0 < far.x0 && e.x1 > far.x1);
    }

    #[test]
    fn stamps_and_selections() {
        assert_eq!(rotated_size(3, 5, Rotation::Cw90), (5, 3));
        assert_eq!(rotated_size(3, 5, Rotation::Half), (3, 5));
        assert_eq!(stamp_origin(I64Vec2::new(10, 10), (3, 4)), I64Vec2::new(9, 8));
        assert_eq!(turn(Rotation::None, true), Rotation::Cw90);
        assert_eq!(turn(Rotation::None, false), Rotation::Ccw90);
        assert_eq!(turn(Rotation::Ccw90, true), Rotation::None);
        assert_eq!(rotation_degrees(Rotation::Half), 180);
        let r = CellRect::new(2, 3, 11, 5);
        assert_eq!(rotated_selection(r, Rotation::Ccw90), Some(CellRect::new(2, 3, 4, 12)));
        assert_eq!(moved_rect(r, -2, 1), Some(CellRect::new(0, 4, 9, 6)));
        assert_eq!(moved_rect(CellRect::new(i64::MAX - 1, 0, i64::MAX, 0), 5, 0), None);
        let torus = World::new(Topology::Torus, 8, 8).unwrap();
        assert_eq!(clip_to_world(CellRect::new(-3, 2, 4, 20), torus), Some(CellRect::new(0, 2, 4, 7)));
    }

    #[test]
    fn transforms_match_the_model_placement() {
        // An L tromino: rotate, mirror and compare with place_pattern.
        let p = Pattern::from_ascii("O.\nO.\nOO").unwrap();
        for rotation in [Rotation::None, Rotation::Cw90, Rotation::Half, Rotation::Ccw90] {
            for mirror in [false, true] {
                let mine = transform_pattern(&p, rotation, mirror);
                let placed = cellular_automata::life::place_pattern(&p, 0, 0, rotation, mirror).unwrap();
                let mut a: Vec<(i64, i64)> = mine.live_cells().collect();
                a.sort_by_key(|&(x, y)| (y, x));
                let b: Vec<(i64, i64)> = placed.iter().map(|&(x, y, _)| (x, y)).collect();
                assert_eq!(a, b, "{rotation:?} mirror {mirror}");
            }
        }
    }

    #[test]
    fn ghost_and_thumbnail_images() {
        let glider = Pattern::from_ascii(".O.\n..O\nOOO").unwrap();
        let (w, h, rgba) = ghost_rgba(&glider, [1, 2, 3, 4]).unwrap();
        assert_eq!((w, h), (3, 3));
        assert_eq!(&rgba[4..8], &[1, 2, 3, 4], "(1, 0) is live");
        assert_eq!(&rgba[0..4], &[0, 0, 0, 0], "(0, 0) is transparent");
        assert!(ghost_rgba(&Pattern::default(), [255; 4]).is_none());
        // 3 x 3 cells in 12 px: 4 px per cell, 3 px lit and a 1 px gap.
        let (tw, th, img) = thumbnail_rgba(&glider, 12, [255, 255, 255], [0, 0, 0]);
        assert_eq!((tw, th, img.len()), (12, 12, 12 * 12 * 4));
        let px = |x: usize, y: usize| &img[(y * 12 + x) * 4..(y * 12 + x) * 4 + 4];
        assert_eq!(px(4, 0), &[255, 255, 255, 255], "cell (1, 0) is live");
        assert_eq!(px(7, 0), &[0, 0, 0, 255], "gap after it");
        assert_eq!(px(0, 0), &[0, 0, 0, 255], "cell (0, 0) is dead");
        let (tw, _, _) = thumbnail_rgba(&glider, 3, [255, 255, 255], [0, 0, 0]);
        assert_eq!(tw, 3, "one pixel per cell without gaps when there is no room");
        // A 100-cell-wide line in at most 10 pixels: 10 cells per pixel.
        let line = Pattern::from_live_cells((0..100).map(|x| (x, 0)));
        let (tw, th, img) = thumbnail_rgba(&line, 10, [200, 0, 0], [0, 0, 0]);
        assert_eq!((tw, th), (10, 1));
        assert!(img[0] > 70 && img[0] < 200, "partly covered pixel: {}", img[0]);
    }

    #[test]
    fn scale_bars_and_jumps() {
        assert_eq!(scale_bar(1.0, 90.0), Some((100.0, 100.0)));
        assert_eq!(scale_bar(0.1, 90.0), Some((10.0, 100.0)));
        let (cells, px) = scale_bar(0.001, 90.0).unwrap();
        assert_eq!(cells, 1.0, "never less than one cell");
        assert!((px - 1000.0).abs() < 0.01);
        assert!(scale_bar(f32::NAN, 90.0).is_none());
        assert_eq!(jump_plan(10), (1024, 1));
        assert_eq!(jump_plan(30), (1 << 30, 1));
        assert_eq!(jump_plan(40), (1 << 30, 1024));
    }
}
