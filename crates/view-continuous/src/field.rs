//! Pure helpers shared by the rendering, tool and panel systems: mapping
//! between cell space and Bevy world space, visible-region view requests,
//! encoding of field planes for the grid renderer, the RGB composite of
//! multi-channel worlds, display ranges, brush strokes, the centroid trail
//! and the scale bar. Everything here is plain data and unit tested.

use bevy::math::{DVec2, I64Vec2};
use bevy::prelude::*;
use continuous::sim::view::MAX_VIEW_SIDE;
use continuous::{ContinuousView, ContinuousViewRequest, CreatureCells, ViewLayer};
use rendering::grid::{GridFrame, GridSpace, VisibleCells, encode_range};
use rendering::palette::Colormap;

/// Colours of the channels in the RGB composite (sRGB, additive). The first
/// three are pure red, green and blue, the convention of Chan's
/// multi-channel Lenia renderings; further channels use secondary hues.
pub const CHANNEL_COLORS: [[f32; 3]; 8] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [1.0, 1.0, 0.0],
    [1.0, 0.0, 1.0],
    [0.0, 1.0, 1.0],
    [1.0, 0.5, 0.0],
    [0.6, 0.6, 0.6],
];

/// Colour of channel `c` in the composite.
pub fn channel_color(c: usize) -> [f32; 3] {
    CHANNEL_COLORS[c % CHANNEL_COLORS.len()]
}

/// World position of a fractional cell-space point (`x` right, `y` down;
/// cell `(i, j)` covers `[i, i + 1) × [j, j + 1)`). Inverse of
/// [`GridSpace::world_to_cell_f64`].
pub fn cell_point_to_world(space: &GridSpace, p: DVec2) -> Vec2 {
    let s = f64::from(space.cell_size);
    Vec2::new(((p.x - space.anchor.x as f64) * s) as f32, (-(p.y - space.anchor.y as f64) * s) as f32)
}

/// Centre and size (world units) of the whole `width × height` world.
pub fn world_rect(space: &GridSpace, width: usize, height: usize) -> (Vec2, Vec2) {
    let centre = cell_point_to_world(space, DVec2::new(width as f64 / 2.0, height as f64 / 2.0));
    (centre, Vec2::new(width as f32, height as f32) * space.cell_size)
}

/// Whether a cell-space point lies inside the `width × height` world.
pub fn inside_world(p: DVec2, width: usize, height: usize) -> bool {
    (0.0..width as f64).contains(&p.x) && (0.0..height as f64).contains(&p.y)
}

/// Request for the part of the world that is visible through the camera,
/// grown by `margin` cells on every side so that small pans do not expose
/// missing edges, clipped to the world and aligned to the level of detail
/// (one output sample per `2^lod` cells, at least one logical pixel each).
/// `None` when the camera does not show the world or the zoom is unknown.
pub fn region_request(
    visible: &VisibleCells,
    world: (usize, usize),
    layer: ViewLayer,
    margin: i64,
) -> Option<ContinuousViewRequest> {
    let cpp = visible.cells_per_pixel;
    if !(cpp.is_finite() && cpp > 0.0) || world.0 == 0 || world.1 == 0 {
        return None;
    }
    let (w, h) = (world.0 as i64, world.1 as i64);
    let margin = margin.max(0);
    let x0 = visible.min_x.saturating_sub(margin).max(0);
    let y0 = visible.min_y.saturating_sub(margin).max(0);
    let x1 = visible.max_x.saturating_add(margin).min(w - 1);
    let y1 = visible.max_y.saturating_add(margin).min(h - 1);
    if x0 > x1 || y0 > y1 {
        return None;
    }
    let k = 1i64 << visible.lod_log2().min(12);
    // Texel boundaries stay fixed in cell space while panning.
    let (ax0, ay0) = (x0.div_euclid(k) * k, y0.div_euclid(k) * k);
    let (width, height) = (x1 - ax0 + 1, y1 - ay0 + 1);
    let side = |cells: i64| ((cells + k - 1) / k).clamp(1, i64::from(MAX_VIEW_SIDE)) as u32;
    Some(ContinuousViewRequest {
        x0: ax0,
        y0: ay0,
        width: width as u32,
        height: height as u32,
        max_cols: side(width),
        max_rows: side(height),
        layer,
    })
}

/// Margin (cells) added around the visible region: a quarter of the visible
/// extent, at least two cells.
pub fn request_margin(visible: &VisibleCells) -> i64 {
    let extent = visible.width().max(visible.height()).min(1 << 40) as i64;
    (extent / 4).max(2)
}

/// Natural display range of a view, made non-degenerate.
pub fn natural_range(view: &ContinuousView) -> (f32, f32) {
    sanitize_range(view.value_range)
}

fn sanitize_range((lo, hi): (f32, f32)) -> (f32, f32) {
    let lo = if lo.is_finite() { lo } else { 0.0 };
    let hi = if hi.is_finite() { hi } else { lo + 1.0 };
    if hi > lo { (lo, hi) } else { (lo, lo + 1.0) }
}

/// Range used to colour `planes` of `view`: the model's natural range, or
/// with `auto` the minimum and maximum of the shown samples (falling back
/// to the natural range for an almost uniform field).
pub fn display_range(view: &ContinuousView, planes: &[usize], auto: bool) -> (f32, f32) {
    let natural = natural_range(view);
    if !auto {
        return natural;
    }
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for v in planes.iter().filter_map(|&p| view.planes.get(p)).flatten().copied().filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if hi - lo > 1e-6 { (lo, hi) } else { natural }
}

/// Grid frame of one plane of `view`, values mapped linearly from `range`
/// to the R byte (G, B and A stay 0). Texel `(0, 0)` is the view's top-left
/// region cell; one texel covers `cells_per_sample²` cells.
pub fn plane_frame(view: &ContinuousView, plane: usize, range: (f32, f32)) -> Option<GridFrame> {
    let data = view.planes.get(plane)?;
    let origin = I64Vec2::new(view.x0 as i64, view.y0 as i64);
    let mut frame = GridFrame::new(origin, view.cols as u32, view.rows as u32, view.cells_per_sample as u32).ok()?;
    if data.len() != view.cols * view.rows {
        return None;
    }
    for (px, &v) in frame.data.as_chunks_mut::<4>().0.iter_mut().zip(data) {
        px[0] = encode_range(v, range.0, range.1);
    }
    Some(frame)
}

/// Additive RGB composite of `planes` (each `len` samples), every channel
/// normalised by `range` and tinted with `channel_color`; opaque sRGB
/// RGBA bytes.
pub fn composite_rgba(planes: &[Vec<f32>], len: usize, range: (f32, f32)) -> Vec<u8> {
    let (lo, hi) = sanitize_range(range);
    let span = hi - lo;
    let mut out = vec![0u8; len * 4];
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let mut rgb = [0.0f32; 3];
        for (c, plane) in planes.iter().enumerate() {
            let v = plane.get(i).copied().unwrap_or(0.0);
            let t = if v.is_finite() { ((v - lo) / span).clamp(0.0, 1.0) } else { 0.0 };
            let color = channel_color(c);
            for k in 0..3 {
                rgb[k] += t * color[k];
            }
        }
        px[0] = to_byte(rgb[0]);
        px[1] = to_byte(rgb[1]);
        px[2] = to_byte(rgb[2]);
        px[3] = 255;
    }
    out
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Value of `plane` of `view` at cell `cell`, if the cell is inside the
/// view's region (the sample covering it when the view is downsampled).
pub fn sample_plane(view: &ContinuousView, plane: usize, cell: I64Vec2) -> Option<f32> {
    let data = view.planes.get(plane)?;
    let (ww, wh) = (view.world_width as i64, view.world_height as i64);
    if ww == 0 || wh == 0 || !(0..ww).contains(&cell.x) || !(0..wh).contains(&cell.y) {
        return None;
    }
    let dx = (cell.x - view.x0 as i64).rem_euclid(ww) as usize;
    let dy = (cell.y - view.y0 as i64).rem_euclid(wh) as usize;
    if dx >= view.width || dy >= view.height {
        return None;
    }
    let k = view.cells_per_sample.max(1);
    data.get((dy / k) * view.cols + dx / k).copied()
}

/// Positions of brush dabs along the segment `from → to`, every `spacing`
/// cells (the first one `spacing` away from `from`), and the position the
/// next segment continues from (the last dab, or `from` when the segment is
/// shorter than the spacing).
pub fn stroke_dabs(from: DVec2, to: DVec2, spacing: f64) -> (Vec<DVec2>, DVec2) {
    let spacing = if spacing.is_finite() && spacing > 1e-3 { spacing } else { 1e-3 };
    let delta = to - from;
    let distance = delta.length();
    if !distance.is_finite() || distance < spacing {
        return (Vec::new(), from);
    }
    let steps = (distance / spacing).floor().min(10_000.0) as usize;
    let dir = delta / distance;
    let dabs: Vec<DVec2> = (1..=steps).map(|i| from + dir * (spacing * i as f64)).collect();
    let last = dabs.last().copied().unwrap_or(from);
    (dabs, last)
}

/// Spacing between brush dabs for a brush of `radius` cells.
pub fn dab_spacing(radius: f32) -> f64 {
    (f64::from(radius) * 0.35).max(0.5)
}

/// Centre of a placed creature as the model computes it: an unrotated,
/// unscaled copy snaps its top-left corner to the nearest cell, any other
/// placement is centred on the cursor.
pub fn stamp_center(cursor: DVec2, size: (usize, usize), rotation_deg: f32, scale: f32) -> DVec2 {
    if rotation_deg == 0.0 && scale == 1.0 {
        let (cw, ch) = (size.0 as f32, size.1 as f32);
        let x0 = (cursor.x as f32 - cw / 2.0).round();
        let y0 = (cursor.y as f32 - ch / 2.0).round();
        DVec2::new(f64::from(x0 + cw / 2.0), f64::from(y0 + ch / 2.0))
    } else {
        cursor
    }
}

/// Rotation of the world-space preview for a placement rotated
/// `rotation_deg` clockwise on screen (cell `y` grows downward, Bevy's `y`
/// upward, so a clockwise screen rotation is a negative angle about `z`).
pub fn stamp_rotation(rotation_deg: f32) -> Quat {
    Quat::from_rotation_z(-rotation_deg.to_radians())
}

/// Wraps an angle into `(-180, 180]` degrees.
pub fn wrap_degrees(deg: f32) -> f32 {
    if !deg.is_finite() {
        return 0.0;
    }
    let r = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if r == -180.0 { 180.0 } else { r }
}

/// Splits a centroid trajectory (cell coordinates on a torus) into pieces
/// that do not jump across the world's seams: a step longer than half the
/// world along either axis starts a new piece.
pub fn trail_segments(points: &[DVec2], world: (usize, usize)) -> Vec<Vec<DVec2>> {
    let (hw, hh) = (world.0 as f64 / 2.0, world.1 as f64 / 2.0);
    let mut out: Vec<Vec<DVec2>> = Vec::new();
    let mut current: Vec<DVec2> = Vec::new();
    for &p in points {
        if let Some(prev) = current.last()
            && ((p.x - prev.x).abs() > hw || (p.y - prev.y).abs() > hh)
        {
            if current.len() > 1 {
                out.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
        current.push(p);
    }
    if current.len() > 1 {
        out.push(current);
    }
    out
}

/// A scale bar for a zoom of `cells_per_px` cells per logical pixel: a
/// round length (1, 2 or 5 × 10ⁿ cells, at least one cell) close to
/// `target_px` pixels, and its length in pixels.
pub fn scale_bar(cells_per_px: f32, target_px: f32) -> Option<(f32, f32)> {
    if !(cells_per_px.is_finite() && cells_per_px > 0.0 && target_px > 0.0) {
        return None;
    }
    let raw = target_px * cells_per_px;
    let base = 10f32.powf(raw.log10().floor());
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
    Some((cells, cells / cells_per_px))
}

/// sRGB bytes of a colormap at `t`.
pub fn colormap_bytes(map: Colormap, t: f32) -> [u8; 3] {
    let [r, g, b] = map.sample_srgb(t);
    [to_byte(r), to_byte(g), to_byte(b)]
}

/// RGBA image (unmultiplied sRGB, row 0 at the top) of creature cells: one
/// channel through `map`, several channels as the RGB composite; alpha
/// follows the local mass so empty cells are transparent.
pub fn creature_rgba(cells: &CreatureCells, map: Colormap) -> (usize, usize, Vec<u8>) {
    let (w, h) = (cells.width(), cells.height());
    let planes: Vec<&[f32]> = (0..cells.channels()).filter_map(|c| cells.plane(c)).map(|p| p.as_slice()).collect();
    let peak = planes.iter().flat_map(|p| p.iter()).copied().fold(0.0f32, f32::max).max(1e-6);
    let mut out = vec![0u8; w * h * 4];
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (rgb, strength) = if planes.len() == 1 {
            let t = (planes[0].get(i).copied().unwrap_or(0.0) / peak).clamp(0.0, 1.0);
            // Start the ramp above its darkest colour so faint cells stay visible.
            (colormap_bytes(map, 0.25 + 0.75 * t), t)
        } else {
            let mut rgb = [0.0f32; 3];
            let mut strength = 0.0f32;
            for (c, plane) in planes.iter().enumerate() {
                let t = (plane.get(i).copied().unwrap_or(0.0) / peak).clamp(0.0, 1.0);
                strength = strength.max(t);
                let color = channel_color(c);
                for k in 0..3 {
                    rgb[k] += t * color[k];
                }
            }
            ([to_byte(rgb[0]), to_byte(rgb[1]), to_byte(rgb[2])], strength)
        };
        px[..3].copy_from_slice(&rgb);
        px[3] = to_byte((strength * 3.0).min(1.0) * 0.92);
    }
    (w, h, out)
}

/// A plane label of the model as the mode displays it next to the layer
/// name ("Growth · channel 0"): the layer word is not repeated (`growth 0`
/// and `affinity 0` are the growth and affinity of channel 0), and kernels
/// are numbered from 1 as everywhere else in the interface (parameter
/// groups "Kernel 1", the kernel picker, `K1(d)`) while the model labels the
/// potential `potential (kernel 0)`. Channels keep the model's numbering
/// from 0.
pub fn plane_display_label(label: &str) -> String {
    for prefix in ["growth ", "affinity "] {
        if let Some(n) = label.strip_prefix(prefix).filter(|n| n.parse::<usize>().is_ok()) {
            return format!("channel {n}");
        }
    }
    if let Some(n) = label.strip_prefix("potential (kernel ").and_then(|r| r.strip_suffix(')'))
        && let Ok(k) = n.parse::<usize>()
    {
        return format!("kernel {}", k + 1);
    }
    label.to_string()
}

/// A cell value for readouts: magnitudes below 10⁻⁶ are rounding noise of
/// the FFT convolutions (e.g. a filling of −8·10⁻⁹) and read as 0.
pub fn format_cell_value(v: f32) -> String {
    if v.abs() < 1e-6 { "0".to_string() } else { format_value(f64::from(v)) }
}

/// Short human-readable number for legends and readouts.
pub fn format_value(v: f64) -> String {
    if !v.is_finite() {
        return "n/a".to_string();
    }
    let a = v.abs();
    if a == 0.0 {
        "0".to_string()
    } else if a >= 1000.0 {
        format!("{v:.0}")
    } else if a >= 10.0 {
        format!("{v:.1}")
    } else if a >= 0.01 {
        format!("{v:.3}")
    } else {
        format!("{v:.2e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use continuous::ChannelStats;

    fn view(cols: usize, rows: usize, k: usize, planes: Vec<Vec<f32>>) -> ContinuousView {
        ContinuousView {
            tick: 0,
            world_width: 64,
            world_height: 32,
            x0: 8,
            y0: 4,
            width: cols * k,
            height: rows * k,
            cells_per_sample: k,
            cols,
            rows,
            layer: ViewLayer::State,
            labels: (0..planes.len()).map(|i| format!("channel {i}")).collect(),
            planes,
            value_range: (0.0, 1.0),
            channel_stats: vec![ChannelStats::default()],
        }
    }

    #[test]
    fn cell_points_round_trip_through_world_space() {
        let space = GridSpace { cell_size: 2.0, anchor: I64Vec2::new(10, -5) };
        for p in [DVec2::new(10.0, -5.0), DVec2::new(12.25, 3.5), DVec2::new(-7.0, 100.75)] {
            let world = cell_point_to_world(&space, p);
            let (x, y) = space.world_to_cell_f64(world);
            assert!((x - p.x).abs() < 1e-5 && (y - p.y).abs() < 1e-5, "{p:?} -> {world:?} -> ({x}, {y})");
        }
        // Integer points are cell corners.
        let corner = cell_point_to_world(&space, DVec2::new(13.0, -2.0));
        assert_eq!(corner, space.cell_corner_to_world(I64Vec2::new(13, -2)));
        let (centre, size) = world_rect(&GridSpace::new(1.0), 128, 64);
        assert_eq!((centre, size), (Vec2::new(64.0, -32.0), Vec2::new(128.0, 64.0)));
        assert!(inside_world(DVec2::new(0.0, 63.9), 128, 64));
        assert!(!inside_world(DVec2::new(128.0, 1.0), 128, 64));
        assert!(!inside_world(DVec2::new(-0.01, 1.0), 128, 64));
    }

    #[test]
    fn region_requests_clip_to_the_world_and_follow_the_zoom() {
        let visible = VisibleCells { min_x: -20, min_y: 10, max_x: 50, max_y: 40, cells_per_pixel: 0.25 };
        let r = region_request(&visible, (128, 32), ViewLayer::Growth, 4).unwrap();
        assert_eq!((r.x0, r.y0), (0, 6));
        assert_eq!((r.width, r.height), (55, 26), "clipped to x 0..=54, y 6..=31");
        assert_eq!((r.max_cols, r.max_rows), (55, 26), "zoomed in: full resolution");
        assert_eq!(r.layer, ViewLayer::Growth);
        // Zoomed out 3 cells per pixel: LOD 4, origin aligned to multiples of 4.
        let far = VisibleCells { min_x: 5, min_y: 0, max_x: 1000, max_y: 900, cells_per_pixel: 3.0 };
        let r = region_request(&far, (512, 512), ViewLayer::State, 0).unwrap();
        assert_eq!((r.x0, r.y0, r.width, r.height), (4, 0, 508, 512));
        assert_eq!((r.max_cols, r.max_rows), (127, 128));
        // Looking away from the world, or before the camera reported a zoom.
        let away = VisibleCells { min_x: 600, min_y: 0, max_x: 700, max_y: 10, cells_per_pixel: 1.0 };
        assert!(region_request(&away, (512, 512), ViewLayer::State, 2).is_none());
        assert!(region_request(&VisibleCells::default(), (512, 512), ViewLayer::State, 2).is_none());
        assert_eq!(request_margin(&visible), 17);
    }

    #[test]
    fn ranges_are_natural_or_automatic() {
        let mut v = view(2, 1, 1, vec![vec![0.2, 0.6]]);
        assert_eq!(display_range(&v, &[0], false), (0.0, 1.0));
        assert_eq!(display_range(&v, &[0], true), (0.2, 0.6));
        v.planes[0] = vec![0.5, 0.5];
        assert_eq!(display_range(&v, &[0], true), (0.0, 1.0), "uniform fields keep the natural range");
        v.value_range = (1.0, 1.0);
        assert_eq!(natural_range(&v), (1.0, 2.0), "degenerate ranges are widened");
        v.value_range = (-2.0, 2.0);
        assert_eq!(display_range(&v, &[7], true), (-2.0, 2.0), "missing planes fall back");
    }

    #[test]
    fn plane_frames_encode_values_and_geometry() {
        let v = view(3, 2, 2, vec![vec![0.0, 0.5, 1.0, -1.0, 2.0, 0.25]]);
        let f = plane_frame(&v, 0, (0.0, 1.0)).unwrap();
        assert_eq!((f.origin, f.width, f.height, f.cells_per_texel), (I64Vec2::new(8, 4), 3, 2, 2));
        let reds: Vec<u8> = f.data.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
        assert_eq!(reds, vec![0, 128, 255, 0, 255, 64]);
        assert!(f.data.as_chunks::<4>().0.iter().all(|p| p[1..] == [0, 0, 0]));
        // Signed growth range: zero growth maps to the middle of the colormap.
        let g = plane_frame(&view(1, 1, 1, vec![vec![0.0]]), 0, (-1.0, 1.0)).unwrap();
        assert_eq!(g.data[0], 128);
        assert!(plane_frame(&v, 3, (0.0, 1.0)).is_none());
    }

    #[test]
    fn composite_adds_channel_colours() {
        let planes = vec![vec![1.0, 0.0, 0.5], vec![0.0, 1.0, 0.5], vec![0.0, 0.0, 0.5]];
        let rgba = composite_rgba(&planes, 3, (0.0, 1.0));
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[0, 255, 0, 255]);
        assert_eq!(&rgba[8..12], &[128, 128, 128, 255]);
        // A fourth channel (yellow) saturates instead of overflowing.
        let four = vec![vec![1.0], vec![1.0], vec![0.0], vec![1.0]];
        assert_eq!(composite_rgba(&four, 1, (0.0, 1.0)), vec![255, 255, 0, 255]);
        assert_eq!(composite_rgba(&[vec![f32::NAN]], 1, (0.0, 1.0)), vec![0, 0, 0, 255]);
    }

    #[test]
    fn sampling_follows_the_region_and_level_of_detail() {
        let v = view(3, 2, 2, vec![vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]]);
        assert_eq!(sample_plane(&v, 0, I64Vec2::new(8, 4)), Some(1.0));
        assert_eq!(sample_plane(&v, 0, I64Vec2::new(11, 5)), Some(2.0));
        assert_eq!(sample_plane(&v, 0, I64Vec2::new(13, 7)), Some(6.0));
        assert_eq!(sample_plane(&v, 0, I64Vec2::new(14, 4)), None, "right of the region");
        assert_eq!(sample_plane(&v, 0, I64Vec2::new(7, 4)), None, "left of the region");
        assert_eq!(sample_plane(&v, 0, I64Vec2::new(-1, 4)), None, "outside the world");
        assert_eq!(sample_plane(&v, 1, I64Vec2::new(8, 4)), None);
    }

    #[test]
    fn strokes_place_evenly_spaced_dabs() {
        let (dabs, last) = stroke_dabs(DVec2::ZERO, DVec2::new(10.0, 0.0), 3.0);
        assert_eq!(dabs, vec![DVec2::new(3.0, 0.0), DVec2::new(6.0, 0.0), DVec2::new(9.0, 0.0)]);
        assert_eq!(last, DVec2::new(9.0, 0.0));
        let (dabs, last) = stroke_dabs(DVec2::ONE, DVec2::new(1.5, 1.0), 3.0);
        assert!(dabs.is_empty());
        assert_eq!(last, DVec2::ONE, "short moves accumulate until a dab is due");
        assert_eq!(dab_spacing(10.0), 3.5);
        assert_eq!(dab_spacing(0.5), 0.5);
    }

    #[test]
    fn stamps_snap_like_the_model() {
        // Exact copies snap the top-left corner: 20×20 cells at 64.3 → corner 54.
        assert_eq!(stamp_center(DVec2::new(64.3, 10.6), (20, 20), 0.0, 1.0), DVec2::new(64.0, 11.0));
        assert_eq!(stamp_center(DVec2::new(64.3, 10.0), (21, 20), 0.0, 1.0), DVec2::new(64.5, 10.0));
        let free = DVec2::new(64.3, 10.6);
        assert_eq!(stamp_center(free, (20, 20), 15.0, 1.0), free);
        assert_eq!(stamp_center(free, (20, 20), 0.0, 2.0), free);
        // 90° clockwise on screen maps the creature's +x (right) to screen down = Bevy −y.
        let x = stamp_rotation(90.0) * Vec3::X;
        assert!((x - Vec3::NEG_Y).length() < 1e-6, "{x:?}");
    }

    #[test]
    fn angles_wrap() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-180.0), 180.0);
        assert_eq!(wrap_degrees(180.0), 180.0);
        assert_eq!(wrap_degrees(-15.0), -15.0);
        assert_eq!(wrap_degrees(720.0 + 30.0), 30.0);
        assert_eq!(wrap_degrees(f32::NAN), 0.0);
    }

    #[test]
    fn trails_break_at_the_torus_seams() {
        let pts: Vec<DVec2> = [(10.0, 10.0), (12.0, 11.0), (14.0, 12.0), (126.0, 13.0), (124.0, 14.0), (2.0, 14.0)]
            .iter()
            .map(|&(x, y)| DVec2::new(x, y))
            .collect();
        let segments = trail_segments(&pts, (128, 128));
        assert_eq!(segments.len(), 2, "the single point after the last wrap is dropped");
        assert_eq!(segments[0].len(), 3);
        assert_eq!(segments[1], vec![DVec2::new(126.0, 13.0), DVec2::new(124.0, 14.0)]);
        assert!(trail_segments(&pts[..1], (128, 128)).is_empty());
    }

    #[test]
    fn scale_bars_are_round_numbers() {
        assert_eq!(scale_bar(0.25, 100.0), Some((20.0, 80.0)));
        assert_eq!(scale_bar(1.0, 100.0), Some((100.0, 100.0)));
        assert_eq!(scale_bar(0.6, 100.0), Some((50.0, 50.0 / 0.6)));
        let (cells, _) = scale_bar(0.001, 100.0).unwrap();
        assert_eq!(cells, 1.0, "never less than one cell");
        assert!(scale_bar(0.0, 100.0).is_none());
        assert!(scale_bar(f32::NAN, 100.0).is_none());
    }

    #[test]
    fn creature_images_are_transparent_where_empty() {
        use continuous::edit::CellData;
        let cells = CellData::Rows(vec![vec![vec![0.0, 0.5], vec![1.0, 0.0]]]).decode().unwrap();
        let (w, h, rgba) = creature_rgba(&cells, Colormap::Grayscale);
        assert_eq!((w, h), (2, 2));
        assert_eq!(rgba[3], 0, "empty cell");
        assert_eq!(rgba[11], 235, "peak cell is 92 % opaque");
        assert_eq!(&rgba[8..11], &[255, 255, 255], "peak uses the top of the colormap");
        let two = CellData::Rows(vec![vec![vec![1.0]], vec![vec![1.0]]]).decode().unwrap();
        let (_, _, rgba) = creature_rgba(&two, Colormap::Grayscale);
        assert_eq!(&rgba[..3], &[255, 255, 0], "channels 0 and 1 add red and green");
    }

    #[test]
    fn plane_labels_read_well_next_to_the_layer() {
        assert_eq!(plane_display_label("potential (kernel 0)"), "kernel 1");
        assert_eq!(plane_display_label("potential (kernel 11)"), "kernel 12");
        assert_eq!(plane_display_label("growth 2"), "channel 2");
        assert_eq!(plane_display_label("affinity 0"), "channel 0");
        assert_eq!(plane_display_label("channel 0"), "channel 0", "channels keep the model's numbering");
        assert_eq!(plane_display_label("inner filling m"), "inner filling m");
        assert_eq!(plane_display_label("growth rate"), "growth rate");
        assert_eq!(plane_display_label("potential (kernel x)"), "potential (kernel x)");
        assert_eq!(format_cell_value(-7.99e-9), "0", "FFT rounding noise");
        assert_eq!(format_cell_value(0.5), "0.500");
        assert_eq!(format_cell_value(-2.0e-5), "-2.00e-5");
    }

    #[test]
    fn values_format_compactly() {
        assert_eq!(format_value(0.0), "0");
        assert_eq!(format_value(0.1234), "0.123");
        assert_eq!(format_value(-0.5), "-0.500");
        assert_eq!(format_value(12.34), "12.3");
        assert_eq!(format_value(73_000.0), "73000");
        assert_eq!(format_value(0.000_12), "1.20e-4");
        assert_eq!(format_value(f64::NAN), "n/a");
    }
}
