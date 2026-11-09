//! Small animated previews painted on the dashboard cards.
//!
//! The Classic Conway and Cellular Automata previews are real computations
//! (a glider evolving under B3/S23 on a small torus; elementary rule 30).
//! The others are illustrative drawings of each laboratory's subject.

use std::f32::consts::{PI, TAU};

use bevy_egui::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::mode::AppMode;
use crate::theme::{Palette, mix, with_alpha};

/// One synchronous B3/S23 step on a `w × h` torus (row-major cells).
pub fn life_step(cells: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut next = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut n = 0;
            for dy in [h - 1, 0, 1] {
                for dx in [w - 1, 0, 1] {
                    if (dx, dy) != (0, 0) && cells[((y + dy) % h) * w + (x + dx) % w] {
                        n += 1;
                    }
                }
            }
            let alive = cells[y * w + x];
            next[y * w + x] = n == 3 || (alive && n == 2);
        }
    }
    next
}

/// Rows of an elementary cellular automaton (Wolfram numbering) on a ring
/// of `width` cells, starting from one live cell in the middle.
pub fn elementary_rows(rule: u8, width: usize, rows: usize) -> Vec<Vec<bool>> {
    let mut row = vec![false; width];
    if width > 0 {
        row[width / 2] = true;
    }
    let mut out = Vec::with_capacity(rows);
    for _ in 0..rows {
        let next = (0..width)
            .map(|x| {
                let l = row[(x + width - 1) % width] as u8;
                let c = row[x] as u8;
                let r = row[(x + 1) % width] as u8;
                (rule >> ((l << 2) | (c << 1) | r)) & 1 == 1
            })
            .collect();
        out.push(std::mem::replace(&mut row, next));
    }
    out
}

/// Radial glow: concentric discs whose opacities add up to `strength` at
/// the centre and fade linearly to zero at `radius`.
pub fn soft_glow(painter: &Painter, center: Pos2, radius: f32, color: Color32, strength: f32) {
    const LAYERS: usize = 14;
    for i in 0..LAYERS {
        let r = radius * (1.0 - i as f32 / LAYERS as f32);
        painter.circle_filled(center, r, with_alpha(color, strength / LAYERS as f32));
    }
}

/// Accent of a card that cannot be opened yet.
pub fn muted_accent(accent: Color32, palette: &Palette) -> Color32 {
    mix(accent, palette.text_muted, 0.65)
}

const LIFE_W: usize = 16;
const LIFE_H: usize = 10;

fn glider_start() -> Vec<bool> {
    let mut cells = vec![false; LIFE_W * LIFE_H];
    for (x, y) in [(2, 1), (3, 2), (1, 3), (2, 3), (3, 3)] {
        cells[y * LIFE_W + x] = true;
    }
    cells
}

/// Paints the preview of `mode` into `rect` at time `t` (seconds).
/// `active` is false for modes that cannot be opened yet (muted colours).
pub fn paint_preview(
    painter: &Painter,
    rect: Rect,
    mode: AppMode,
    t: f64,
    accent: Color32,
    palette: &Palette,
    active: bool,
) {
    let accent = if active { accent } else { muted_accent(accent, palette) };
    let painter = painter.with_clip_rect(rect);
    let bg = mix(palette.surface, Color32::BLACK, if palette.dark { 0.25 } else { 0.0 });
    painter.rect_filled(rect, 8.0, bg);
    soft_glow(&painter, rect.center(), rect.height() * 0.7, accent, if palette.dark { 0.16 } else { 0.10 });
    let ctx = PreviewCtx { painter: &painter, rect, t: t as f32, accent, palette };
    match mode {
        AppMode::Dashboard => {}
        AppMode::ClassicConway => life_preview(&ctx, t),
        AppMode::EcosystemSimulator => ecosystem_preview(&ctx),
        AppMode::CustomLifeCreator => rules_preview(&ctx),
        AppMode::ConwayWithinConway => metapixel_preview(&ctx),
        AppMode::CellularAutomataLab => elementary_preview(&ctx, t),
        AppMode::EvolutionLab => evolution_preview(&ctx),
        AppMode::ContinuousLife => lenia_preview(&ctx),
        AppMode::ThreeDimensionalLife => voxel_preview(&ctx),
        AppMode::EmergentCivilizations => civilization_preview(&ctx),
    }
}

struct PreviewCtx<'a> {
    painter: &'a Painter,
    rect: Rect,
    t: f32,
    accent: Color32,
    palette: &'a Palette,
}

impl PreviewCtx<'_> {
    fn at(&self, fx: f32, fy: f32) -> Pos2 {
        pos2(self.rect.min.x + fx * self.rect.width(), self.rect.min.y + fy * self.rect.height())
    }
    fn faint(&self) -> Color32 {
        with_alpha(self.palette.text_muted, 0.18)
    }
}

fn cell_grid(ctx: &PreviewCtx, w: usize, h: usize) -> (f32, Pos2) {
    let cell = (ctx.rect.width() / w as f32).min(ctx.rect.height() / h as f32) * 0.92;
    let origin = ctx.rect.center() - vec2(cell * w as f32, cell * h as f32) / 2.0;
    (cell, origin)
}

fn life_preview(ctx: &PreviewCtx, t: f64) {
    // The glider moves (1, 1) every 4 generations; on a 16 x 10 torus it is
    // back at its start after 4 * lcm(16, 10) = 320 generations.
    let generation = (t * 6.0) as usize % 320;
    let (cell, origin) = cell_grid(ctx, LIFE_W, LIFE_H);
    let mut history: Vec<Vec<bool>> = Vec::with_capacity(4);
    let mut cells = glider_start();
    for g in 0..=generation {
        if g + 3 >= generation {
            history.push(cells.clone());
        }
        if g < generation {
            cells = life_step(&cells, LIFE_W, LIFE_H);
        }
    }
    for y in 0..LIFE_H {
        for x in 0..LIFE_W {
            let r = Rect::from_min_size(origin + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell))
                .shrink(cell * 0.1);
            ctx.painter.rect_filled(r, cell * 0.18, ctx.faint());
        }
    }
    let n = history.len();
    for (age, state) in history.iter().enumerate() {
        let alpha = if age + 1 == n { 1.0 } else { 0.12 + 0.1 * age as f32 };
        for (i, _) in state.iter().enumerate().filter(|(_, on)| **on) {
            let (x, y) = (i % LIFE_W, i / LIFE_W);
            let r = Rect::from_min_size(origin + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell))
                .shrink(cell * 0.1);
            ctx.painter.rect_filled(r, cell * 0.18, with_alpha(ctx.accent, alpha));
        }
    }
}

fn elementary_preview(ctx: &PreviewCtx, t: f64) {
    let width = 61;
    let visible = 22;
    let offset = (t * 5.0) as usize % 120;
    let rows = elementary_rows(30, width, offset + visible);
    let cell = (ctx.rect.width() / width as f32).min(ctx.rect.height() / visible as f32);
    let origin = ctx.rect.center() - vec2(cell * width as f32, cell * visible as f32) / 2.0;
    for (r, row) in rows[offset..].iter().enumerate() {
        let fade = 0.35 + 0.65 * (r as f32 / visible as f32);
        for (x, _) in row.iter().enumerate().filter(|(_, on)| **on) {
            let min = origin + vec2(x as f32 * cell, r as f32 * cell);
            ctx.painter.rect_filled(
                Rect::from_min_size(min, vec2(cell, cell)).shrink(0.4),
                1.0,
                with_alpha(ctx.accent, fade),
            );
        }
    }
}

fn ecosystem_preview(ctx: &PreviewCtx) {
    let nodes = [ctx.at(0.18, 0.62), ctx.at(0.5, 0.38), ctx.at(0.82, 0.62)];
    let green = mix(ctx.accent, Color32::from_rgb(90, 200, 90), 0.4);
    let stroke = Stroke::new(1.5, with_alpha(ctx.palette.text_muted, 0.5));
    for pair in nodes.windows(2) {
        ctx.painter.line_segment([pair[0], pair[1]], stroke);
        for k in 0..3 {
            let f = ((ctx.t * 0.35 + k as f32 / 3.0) % 1.0).clamp(0.0, 1.0);
            let p = pair[0] + (pair[1] - pair[0]) * f;
            ctx.painter.circle_filled(p, 2.5, with_alpha(ctx.accent, 0.9 - f * 0.5));
        }
    }
    // Decomposition returns nutrients to the producers.
    let back: Vec<Pos2> = (0..=24)
        .map(|i| {
            let f = i as f32 / 24.0;
            let x = nodes[2].x + (nodes[0].x - nodes[2].x) * f;
            pos2(x, nodes[0].y + 18.0 + (f * PI).sin() * 18.0)
        })
        .collect();
    ctx.painter.add(Shape::dashed_line(&back, Stroke::new(1.2, with_alpha(ctx.palette.text_muted, 0.45)), 4.0, 3.0));
    let s = ctx.rect.height() * 0.11;
    for k in 0..3 {
        let a = ctx.t * 0.6 + k as f32 * TAU / 3.0;
        ctx.painter.circle_filled(nodes[0] + vec2(a.cos(), a.sin()) * s * 0.45, s * 0.55, with_alpha(green, 0.85));
    }
    let bob = (ctx.t * 2.0).sin() * 2.0;
    ctx.painter.circle_filled(nodes[1] + vec2(0.0, bob), s * 0.75, ctx.accent);
    ctx.painter.circle_filled(nodes[1] + vec2(s * 0.5, bob - s * 0.55), s * 0.25, ctx.accent);
    let p = nodes[2];
    ctx.painter.add(Shape::convex_polygon(
        vec![p + vec2(0.0, -s), p + vec2(s * 0.9, s * 0.7), p + vec2(-s * 0.9, s * 0.7)],
        mix(ctx.accent, Color32::from_rgb(230, 110, 90), 0.5),
        Stroke::NONE,
    ));
}

fn rules_preview(ctx: &PreviewCtx) {
    let lines = [("IF", "energy > 8"), ("AND", "neighbours < 3"), ("THEN", "reproduce 0.4"), ("ELSE", "wander")];
    let active = (ctx.t * 0.8) as usize % lines.len();
    let h = ctx.rect.height() / (lines.len() as f32 + 1.0);
    for (i, (kw, text)) in lines.iter().enumerate() {
        let y = ctx.rect.min.y + h * (i as f32 + 0.6);
        let indent = if *kw == "IF" || *kw == "ELSE" { 0.12 } else { 0.18 };
        let block = Rect::from_min_size(
            ctx.at(indent, 0.0) + vec2(0.0, y - ctx.rect.min.y),
            vec2(ctx.rect.width() * 0.56, h * 0.78),
        );
        let on = i == active;
        let fill = if on { with_alpha(ctx.accent, 0.35) } else { with_alpha(ctx.palette.text_muted, 0.12) };
        ctx.painter.rect_filled(block, 5.0, fill);
        ctx.painter.rect_filled(Rect::from_min_size(block.min, vec2(4.0, block.height())), 2.0, ctx.accent);
        let font = bevy_egui::egui::FontId::monospace((h * 0.42).clamp(9.0, 13.0));
        let pos = block.left_center() + vec2(12.0, 0.0);
        let kw_rect = ctx.painter.text(
            pos,
            bevy_egui::egui::Align2::LEFT_CENTER,
            kw,
            font.clone(),
            mix(ctx.accent, ctx.palette.text, 0.2),
        );
        ctx.painter.text(
            kw_rect.right_center() + vec2(8.0, 0.0),
            bevy_egui::egui::Align2::LEFT_CENTER,
            text,
            font,
            ctx.palette.text,
        );
    }
}

fn metapixel_preview(ctx: &PreviewCtx) {
    // A blinker made of metacells: the macro state flips every 1.5 s.
    let n = 5;
    let phase = (ctx.t / 1.5) as usize % 2;
    let (cell, origin) = cell_grid(ctx, n, n);
    for y in 0..n {
        for x in 0..n {
            let on = if phase == 0 { y == 2 && (1..=3).contains(&x) } else { x == 2 && (1..=3).contains(&y) };
            let r = Rect::from_min_size(origin + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell)).shrink(2.0);
            let fill = if on { with_alpha(ctx.accent, 0.85) } else { with_alpha(ctx.palette.text_muted, 0.10) };
            ctx.painter.rect_filled(r, 3.0, fill);
            // Inner texture: the metacell is itself a Life pattern.
            let k = 6;
            let inner = r.width() / k as f32;
            for j in 0..k {
                for i in 0..k {
                    let lit = (i * 7 + j * 3 + x * 5 + y * 11) % 5 == 0 || (on && (i + j) % 3 == 0);
                    if lit {
                        let c = r.min + vec2((i as f32 + 0.5) * inner, (j as f32 + 0.5) * inner);
                        let color =
                            if on { with_alpha(ctx.palette.background, 0.55) } else { with_alpha(ctx.accent, 0.35) };
                        ctx.painter.circle_filled(c, inner * 0.22, color);
                    }
                }
            }
        }
    }
}

fn evolution_preview(ctx: &PreviewCtx) {
    // A deterministic phylogeny revealed from left (ancestor) to right.
    // Grow for 5 s, then hold the full tree for 3 s.
    let progress = ((ctx.t % 8.0) / 5.0).min(1.0);
    let mut seed: u32 = 0x9e37_79b9;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 1000) as f32 / 1000.0
    };
    let mut tips = vec![(ctx.at(0.08, 0.5), 0.0f32)];
    let depth = 5;
    for level in 0..depth {
        let x = 0.08 + 0.84 * (level as f32 + 1.0) / depth as f32;
        let visible = progress * depth as f32 - level as f32;
        let spread = 0.42 / (level as f32 + 1.0).powf(0.8);
        let mut new_tips = Vec::new();
        for (from, hue) in &tips {
            let branches = if next() < 0.75 { 2 } else { 1 };
            for b in 0..branches {
                let dy = if branches == 1 {
                    0.0
                } else if b == 0 {
                    -spread / 2.0
                } else {
                    spread / 2.0
                };
                let jitter = (next() - 0.5) * 0.05;
                let to = pos2(
                    ctx.at(x, 0.0).x,
                    (from.y + (dy + jitter) * ctx.rect.height()).clamp(ctx.rect.min.y + 6.0, ctx.rect.max.y - 6.0),
                );
                let hue = hue + (next() - 0.5) * 0.5;
                let color = mix(ctx.accent, Color32::from_rgb(120, 170, 255), (hue.sin() * 0.5 + 0.5).clamp(0.0, 1.0));
                let mid = pos2(from.x, to.y);
                // The whole tree is always visible, faintly; the growing
                // front sweeps from the ancestor to the living tips.
                let faint = Stroke::new(1.2, with_alpha(color, 0.3));
                ctx.painter.line_segment([*from, mid], faint);
                ctx.painter.line_segment([mid, to], faint);
                if visible > 0.0 {
                    let f = visible.min(1.0);
                    let end = mid + (to - mid) * f;
                    ctx.painter.line_segment([*from, mid], Stroke::new(1.6, color));
                    ctx.painter.line_segment([mid, end], Stroke::new(1.6, color));
                    if f >= 1.0 && level + 1 == depth {
                        ctx.painter.circle_filled(to, 2.6, color);
                    }
                }
                new_tips.push((to, hue));
            }
        }
        tips = new_tips;
    }
}

fn lenia_preview(ctx: &PreviewCtx) {
    let c = ctx.rect.center() + vec2((ctx.t * 0.5).sin() * ctx.rect.width() * 0.12, 0.0);
    let r0 = ctx.rect.height() * 0.32;
    for layer in 0..14 {
        let f = layer as f32 / 14.0;
        let radius = r0 * (1.15 - f);
        let points: Vec<Pos2> = (0..64)
            .map(|i| {
                let a = i as f32 / 64.0 * TAU;
                let wobble = 1.0 + 0.10 * (3.0 * a + ctx.t * 1.3).sin() + 0.05 * (5.0 * a - ctx.t * 0.7).cos();
                c + vec2(a.cos(), a.sin()) * radius * wobble
            })
            .collect();
        // Ring-shaped density (as in Orbium): low at the core and rim.
        let density = (f * PI).sin().powi(2);
        let color = with_alpha(mix(ctx.accent, Color32::WHITE, density * 0.35), 0.07 + 0.16 * density);
        ctx.painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    }
    ctx.painter.circle_filled(c, r0 * 0.18, with_alpha(ctx.palette.background, 0.5));
}

fn voxel_preview(ctx: &PreviewCtx) {
    let a = ctx.t * 0.6;
    let tilt = 0.55f32;
    let scale = ctx.rect.height() * 0.21;
    let center = ctx.rect.center();
    let project = |x: f32, y: f32, z: f32| -> (Pos2, f32) {
        let (s, c) = a.sin_cos();
        let (x1, z1) = (x * c - z * s, x * s + z * c);
        let (st, ct) = tilt.sin_cos();
        let (y2, z2) = (y * ct - z1 * st, y * st + z1 * ct);
        (center + vec2(x1, -y2) * scale, z2)
    };
    let corners: Vec<(f32, f32, f32)> = (0..8)
        .map(|i| {
            (
                if i & 1 == 0 { -1.5 } else { 1.5 },
                if i & 2 == 0 { -1.5 } else { 1.5 },
                if i & 4 == 0 { -1.5 } else { 1.5 },
            )
        })
        .collect();
    for i in 0..8usize {
        for bit in [1usize, 2, 4] {
            let j = i | bit;
            if j != i {
                let (p, _) = project(corners[i].0, corners[i].1, corners[i].2);
                let (q, _) = project(corners[j].0, corners[j].1, corners[j].2);
                ctx.painter.line_segment([p, q], Stroke::new(1.2, with_alpha(ctx.palette.text_muted, 0.55)));
            }
        }
    }
    // A 3D "glider-like" cluster of live voxels, drawn back to front.
    let live = [(-1, -1, 0), (0, -1, 0), (1, -1, 0), (1, 0, 0), (0, 1, 0), (0, 0, 1), (-1, 0, -1)];
    let mut voxels: Vec<(Pos2, f32)> = live.iter().map(|&(x, y, z)| project(x as f32, y as f32, z as f32)).collect();
    voxels.sort_by(|a, b| a.1.total_cmp(&b.1));
    for (p, depth) in voxels {
        let shade = 0.55 + 0.45 * ((depth + 2.0) / 4.0).clamp(0.0, 1.0);
        let half = scale * 0.36;
        ctx.painter.rect_filled(
            Rect::from_center_size(p, vec2(half, half) * 2.0),
            3.0,
            mix(ctx.palette.background, ctx.accent, shade),
        );
    }
}

fn civilization_preview(ctx: &PreviewCtx) {
    let towns = [ctx.at(0.2, 0.3), ctx.at(0.55, 0.22), ctx.at(0.8, 0.55), ctx.at(0.35, 0.72), ctx.at(0.62, 0.8)];
    let sizes = [1.0f32, 0.7, 0.85, 0.6, 0.5];
    let roads = [(0, 1), (1, 2), (0, 3), (3, 4), (4, 2), (1, 3)];
    for (i, town) in towns.iter().enumerate() {
        ctx.painter.circle_filled(*town, ctx.rect.height() * 0.17 * sizes[i], with_alpha(ctx.accent, 0.08));
    }
    for (k, &(a, b)) in roads.iter().enumerate() {
        ctx.painter.line_segment([towns[a], towns[b]], Stroke::new(2.0, with_alpha(ctx.palette.text_muted, 0.4)));
        let f = ((ctx.t * 0.25 + k as f32 * 0.37) % 1.0).clamp(0.0, 1.0);
        let f = if k % 2 == 0 { f } else { 1.0 - f };
        ctx.painter.circle_filled(towns[a] + (towns[b] - towns[a]) * f, 2.4, mix(ctx.accent, ctx.palette.text, 0.3));
    }
    for (i, town) in towns.iter().enumerate() {
        let s = 4.0 + 6.0 * sizes[i];
        let roof = [*town + vec2(-s, 0.0), *town + vec2(0.0, -s * 0.9), *town + vec2(s, 0.0)];
        ctx.painter.rect_filled(
            Rect::from_min_max(*town + vec2(-s * 0.75, 0.0), *town + vec2(s * 0.75, s)),
            1.5,
            ctx.accent,
        );
        ctx.painter.add(Shape::convex_polygon(roof.to_vec(), mix(ctx.accent, ctx.palette.text, 0.35), Stroke::NONE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glider_translates_by_one_cell_diagonally_every_four_generations() {
        let mut cells = glider_start();
        for _ in 0..4 {
            cells = life_step(&cells, LIFE_W, LIFE_H);
        }
        let expected: Vec<bool> = (0..LIFE_W * LIFE_H)
            .map(|i| {
                let (x, y) = (i % LIFE_W, i / LIFE_W);
                glider_start()[((y + LIFE_H - 1) % LIFE_H) * LIFE_W + (x + LIFE_W - 1) % LIFE_W]
            })
            .collect();
        assert_eq!(cells, expected);
        assert_eq!(cells.iter().filter(|c| **c).count(), 5);
    }

    #[test]
    fn glider_returns_home_after_320_generations_on_the_torus() {
        let mut cells = glider_start();
        for _ in 0..320 {
            cells = life_step(&cells, LIFE_W, LIFE_H);
        }
        assert_eq!(cells, glider_start());
    }

    #[test]
    fn blinker_oscillates() {
        let mut cells = vec![false; 25];
        for x in 1..4 {
            cells[2 * 5 + x] = true;
        }
        let once = life_step(&cells, 5, 5);
        assert!(once[5 + 2] && once[2 * 5 + 2] && once[3 * 5 + 2] && !once[2 * 5 + 1]);
        assert_eq!(life_step(&once, 5, 5), cells);
    }

    #[test]
    fn rule_30_matches_known_rows() {
        let rows = elementary_rows(30, 11, 4);
        let text: Vec<String> = rows.iter().map(|r| r.iter().map(|&c| if c { '#' } else { '.' }).collect()).collect();
        assert_eq!(text, vec![".....#.....", "....###....", "...##..#...", "..##.####.."]);
    }

    #[test]
    fn disabled_accents_remain_visible() {
        use crate::theme::{ThemeKind, contrast_ratio};
        for kind in ThemeKind::ALL {
            for mode in AppMode::SIMULATIONS {
                let p = Palette::new(kind, mode.default_accent());
                assert!(contrast_ratio(muted_accent(mode.default_accent(), &p), p.surface) >= 2.0, "{kind:?} {mode:?}");
            }
        }
    }

    #[test]
    fn every_preview_paints() {
        let ctx = bevy_egui::egui::Context::default();
        let palette = Palette::new(crate::theme::ThemeKind::Dark, Color32::from_rgb(90, 169, 255));
        let output = crate::run_headless_ui(&ctx, |ui| {
            for (i, mode) in AppMode::SIMULATIONS.iter().enumerate() {
                let rect = Rect::from_min_size(pos2(0.0, i as f32 * 130.0), vec2(300.0, 120.0));
                paint_preview(ui.painter(), rect, *mode, 3.7, mode.default_accent(), &palette, i % 2 == 0);
            }
        });
        assert!(output.shapes.len() > 9 * 3);
    }
}
