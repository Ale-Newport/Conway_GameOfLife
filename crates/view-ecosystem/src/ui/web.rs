//! The Food web tab: nodes in rows by trophic level, diet links weighted by
//! the flows the run has observed.

use bevy_egui::egui;
use egui::{Color32, Pos2, Shape, Stroke, pos2, vec2};
use interface::Palette;

use super::{UiCache, color32, energy, note, num};
use crate::foodweb::{Flow, FoodWeb, NodeKind, build};
use crate::palette::category_color;
use crate::session::Frame;
use crate::state::{EcoUi, PropertiesTab};

/// Radius (points) of the most populous species' node.
const MAX_NODE_RADIUS: f32 = 13.0;

/// Seconds between two rebuilds of the web while the run goes on.
pub const WEB_REFRESH_SECONDS: f64 = 1.0;

fn node_color(kind: NodeKind) -> Color32 {
    match kind {
        NodeKind::Species(s) => color32(category_color(usize::from(s))),
        NodeKind::Basal(crate::foodweb::Basal::Plants) => Color32::from_rgb(90, 165, 75),
        NodeKind::Basal(crate::foodweb::Basal::Nectar) => Color32::from_rgb(235, 200, 70),
        NodeKind::Basal(crate::foodweb::Basal::Detritus) => Color32::from_rgb(140, 110, 70),
        NodeKind::Basal(crate::foodweb::Basal::Fungi) => Color32::from_rgb(170, 130, 200),
        NodeKind::Basal(crate::foodweb::Basal::Carrion) => Color32::from_rgb(150, 70, 70),
        NodeKind::Basal(crate::foodweb::Basal::Eggs) => Color32::from_rgb(230, 225, 205),
    }
}

fn flow_text(flow: Flow) -> String {
    match flow {
        Flow::Kills { count, kg } => format!("{count} killed ({})", super::mass(kg)),
        Flow::Eaten(kj) => format!("{} eaten grazing or browsing", energy(kj)),
        Flow::NotExposed => "flow not separated by the model".into(),
    }
}

/// `label` cut (with an ellipsis) to the longest prefix whose width,
/// measured by `width`, fits `max` (the hover card has the full name).
pub fn fit_label(label: &str, max: f32, width: impl Fn(&str) -> f32) -> String {
    if width(label) <= max {
        return label.to_string();
    }
    let chars: Vec<char> = label.chars().collect();
    for n in (1..chars.len()).rev() {
        let cut: String = chars[..n].iter().collect();
        let text = format!("{}…", cut.trim_end());
        if width(&text) <= max {
            return text;
        }
    }
    String::new()
}

/// Label lanes of a row of `n` nodes over `width` points: labels alternate
/// above and below the nodes, and crowded rows use two more lanes farther
/// out, so that neighbours never share a lane.
pub fn label_lanes(n: usize, width: f32) -> usize {
    if n as f32 * 34.0 > width { 4 } else { 2 }
}

/// Draws the food web.
pub fn food_web_tab(
    ui: &mut egui::Ui,
    frame: &Frame,
    seq: u64,
    now: f64,
    cache: &mut UiCache,
    eco: &mut EcoUi,
    palette: &Palette,
) {
    let stale = cache.web.as_ref().is_none_or(|(s, _)| *s != seq) && now - cache.web_time >= WEB_REFRESH_SECONDS;
    if cache.web.is_none() || stale {
        let pops: Vec<f64> = frame.view.species.iter().map(|p| f64::from(p.population)).collect();
        let ever: Vec<bool> =
            frame.counters.total_births.iter().zip(&frame.counters.total_deaths).map(|(b, d)| b + d > 0).collect();
        cache.web = Some((seq, build(&frame.info, &pops, &ever, &frame.observed)));
        cache.web_time = now;
    }
    let Some((_, web)) = cache.web.as_ref() else { return };
    let days = web.observed_ticks as f64 / f64::from(frame.info.ticks_per_day.max(1));
    note(
        ui,
        &format!(
            "Who eats whom, from the species' diets. Line width grows with the flow observed over the last \
             {days:.1} days (kills; energy eaten while grazing or browsing); dashed links are diet links whose \
             flow the model does not separate (foraging, scavenging, field prey). Rows: producers and other \
             foods, primary and secondary consumers, apex predators."
        ),
    );
    let height = 460.0f32.min(ui.available_height().max(340.0));
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, palette.surface);
    let rows = 4usize;
    let pad = vec2(52.0, 34.0);
    let pos = |web: &FoodWeb, i: usize| -> Pos2 {
        let n = &web.nodes[i];
        let x = rect.left() + pad.x + n.x * (rect.width() - 2.0 * pad.x);
        let y = rect.bottom() - pad.y - n.level as f32 / (rows - 1) as f32 * (rect.height() - 2.0 * pad.y);
        pos2(x, y)
    };
    for (row, label) in ["foods", "primary", "secondary", "apex"].iter().enumerate() {
        let y = rect.bottom() - pad.y - row as f32 / (rows - 1) as f32 * (rect.height() - 2.0 * pad.y);
        painter.text(
            pos2(rect.left() + 6.0, y),
            egui::Align2::LEFT_CENTER,
            *label,
            egui::FontId::proportional(10.0),
            palette.text_muted,
        );
    }
    let hovered = response.hover_pos().and_then(|p| {
        (0..web.nodes.len())
            .map(|i| (pos(web, i).distance(p), i))
            .filter(|(d, _)| *d < 14.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    });
    for e in &web.edges {
        let (a, b) = (pos(web, e.from), pos(web, e.to));
        let focus = hovered.is_none_or(|h| h == e.from || h == e.to);
        let alpha = if focus { 1.0 } else { 0.15 };
        let base = match e.flow {
            Flow::Kills { .. } => Color32::from_rgb(215, 75, 65),
            Flow::Eaten(_) => Color32::from_rgb(110, 175, 80),
            Flow::NotExposed => palette.text_muted,
        };
        let color = base.gamma_multiply(alpha * 0.85);
        let width = web.edge_width(e, 7.0);
        match e.flow {
            Flow::NotExposed => {
                for s in Shape::dashed_line(&[a, b], Stroke::new(1.0, color), 5.0, 4.0) {
                    painter.add(s);
                }
            }
            _ => {
                painter.line_segment([a, b], Stroke::new(width, color));
            }
        }
    }
    for (i, n) in web.nodes.iter().enumerate() {
        let p = pos(web, i);
        let r = match n.kind {
            NodeKind::Basal(_) => 7.0,
            NodeKind::Species(_) => (4.0 + 1.6 * (n.population + 1.0).log10() as f32).clamp(4.0, MAX_NODE_RADIUS),
        };
        let color = node_color(n.kind);
        if n.alive || matches!(n.kind, NodeKind::Basal(_)) {
            painter.circle_filled(p, r, color);
        } else {
            painter.circle_stroke(p, r, Stroke::new(1.5, color.gamma_multiply(0.6)));
        }
        if hovered == Some(i) {
            painter.circle_stroke(p, r + 3.0, Stroke::new(2.0, palette.accent));
        }
        // Labels go round lanes above and below the row; each is cut to the
        // room up to the next label in its lane.
        let row = web.nodes.iter().filter(|m| m.level == n.level).count();
        let rank = web.nodes.iter().filter(|m| m.level == n.level && m.x < n.x).count();
        let span = rect.width() - 2.0 * pad.x;
        let lanes = if n.level == 0 { 1 } else { label_lanes(row, span) };
        let lane = rank % lanes;
        let above = lane % 2 == 1;
        let out = (lane / 2) as f32 * 11.0;
        // Lanes sit at fixed heights (past the largest node) so that labels
        // of different lanes never share a line.
        let (anchor, offset) = if above {
            (egui::Align2::CENTER_BOTTOM, -MAX_NODE_RADIUS - 2.0 - out)
        } else {
            (egui::Align2::CENTER_TOP, MAX_NODE_RADIUS + 2.0 + out)
        };
        let room = (span / row.max(1) as f32) * lanes as f32 - 6.0;
        let font = egui::FontId::proportional(10.0);
        let color = if n.alive || matches!(n.kind, NodeKind::Basal(_)) { palette.text } else { palette.text_muted };
        let text = fit_label(&n.label, room, |t| painter.layout_no_wrap(t.to_string(), font.clone(), color).size().x);
        painter.text(p + vec2(0.0, offset), anchor, text, font, color);
    }
    if let Some(h) = hovered {
        let n = &web.nodes[h];
        response.clone().on_hover_ui_at_pointer(|ui| {
            ui.label(egui::RichText::new(&n.label).strong());
            if let NodeKind::Species(_) = n.kind {
                ui.label(format!("population {}", num(n.population)));
            }
            for e in web.edges.iter().filter(|e| e.to == h) {
                ui.label(format!(
                    "eats {} (preference {:.2}): {}",
                    web.nodes[e.from].label,
                    e.preference,
                    flow_text(e.flow)
                ));
            }
            for e in web.edges.iter().filter(|e| e.from == h) {
                ui.label(format!("eaten by {}: {}", web.nodes[e.to].label, flow_text(e.flow)));
            }
        });
        if response.clicked()
            && let NodeKind::Species(s) = n.kind
        {
            eco.species = Some(s);
            eco.tab = PropertiesTab::Species;
        }
    }
    ui.horizontal_wrapped(|ui| {
        let chip = |ui: &mut egui::Ui, c: Color32, t: &str| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 10.0), egui::Sense::hover());
            ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(3.0, c));
            ui.label(t);
        };
        chip(ui, Color32::from_rgb(215, 75, 65), "kills");
        chip(ui, Color32::from_rgb(110, 175, 80), "grazing / browsing");
        chip(ui, palette.text_muted, "diet link");
        ui.label(egui::RichText::new("· hollow: absent now · click a species for its sheet").weak());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_fit_their_room() {
        let width = |t: &str| t.chars().count() as f32 * 6.0;
        assert_eq!(fit_label("Red fox", 60.0, width), "Red fox");
        let cut = fit_label("European rabbit", 60.0, width);
        assert!(width(&cut) <= 60.0 && cut.ends_with('…'), "{cut}");
        assert_eq!(fit_label("Grey heron", 3.0, width), "");
        assert_eq!(label_lanes(4, 300.0), 2);
        assert_eq!(label_lanes(12, 300.0), 4, "crowded rows use more lanes");
    }
}
