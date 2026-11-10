//! The relationship graph view: a Fruchterman–Reingold force-directed
//! layout seeded and biased by trophic level (higher levels drawn higher,
//! resources at the bottom), drawn with the egui painter as typed,
//! coloured edges with arrow heads and a legend.

use std::collections::BTreeSet;

use bevy_egui::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use interface::Palette;
use interface::theme::{color32_from_rgba, mix, with_alpha};
use rule_engine::def::{LifeSystem, RelationKind};
use rule_engine::{GraphEdge, GraphNode, RelationshipGraph};

use crate::colors::relation_color;

/// Relationship graph built directly from a definition (used while the
/// draft does not compile; the compiled graph is preferred when available).
/// Mirrors the rule engine: types then resources as nodes, trophic levels
/// from `consumes` edges (`1 + mean level of the food`, capped at the node
/// count), one edge per relationship whose endpoints exist.
pub fn graph_of(sys: &LifeSystem) -> RelationshipGraph {
    let mut nodes: Vec<GraphNode> = sys
        .entity_types
        .iter()
        .map(|t| GraphNode {
            name: t.name.clone(),
            resource: false,
            color: t.color,
            icon: t.icon.clone(),
            trophic_level: 1.0,
        })
        .collect();
    nodes.extend(sys.world.resources.iter().map(|r| GraphNode {
        name: r.name.clone(),
        resource: true,
        color: r.color,
        icon: String::new(),
        trophic_level: 1.0,
    }));
    let names: Vec<String> = nodes.iter().map(|n| n.name.clone()).collect();
    let index = |name: &str| names.iter().position(|n| n == name);
    let foods: Vec<Vec<usize>> = (0..nodes.len())
        .map(|i| {
            sys.relationships
                .iter()
                .filter(|r| matches!(r.kind, RelationKind::Consumes { .. }) && r.subject == names[i])
                .filter_map(|r| index(&r.object))
                .collect()
        })
        .collect();
    let cap = nodes.len().max(1) as f64;
    let mut level = vec![1.0f64; nodes.len()];
    for _ in 0..nodes.len() + 1 {
        let next: Vec<f64> = foods
            .iter()
            .map(|f| {
                if f.is_empty() {
                    1.0
                } else {
                    (1.0 + f.iter().map(|&j| level[j]).sum::<f64>() / f.len() as f64).min(cap)
                }
            })
            .collect();
        if next == level {
            break;
        }
        level = next;
    }
    for (n, l) in nodes.iter_mut().zip(level) {
        n.trophic_level = l;
    }
    let edges = sys
        .relationships
        .iter()
        .enumerate()
        .filter_map(|(i, rel)| {
            Some(GraphEdge {
                from: index(&rel.subject)?,
                to: index(&rel.object)?,
                relation: rel.kind.label().to_string(),
                relationship: i,
                rules: Vec::new(),
                summary: rel.kind.summary(),
                symmetric: rel.kind.is_symmetric(),
            })
        })
        .collect();
    RelationshipGraph { nodes, edges }
}

/// Target height (0 = top, 1 = bottom) of a node: resources at the bottom,
/// entity types spread by trophic level (the highest level at the top).
pub fn target_y(level: f64, max_level: f64, resource: bool) -> f32 {
    if resource {
        return 0.9;
    }
    let span = (max_level - 1.0).max(1.0);
    let t = ((level - 1.0) / span).clamp(0.0, 1.0) as f32;
    0.72 - 0.6 * t
}

/// State of a force-directed layout in normalised coordinates (`[0, 1]²`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForceLayout {
    /// Node positions.
    pub pos: Vec<[f32; 2]>,
    /// Nodes the user dragged (excluded from the simulation).
    pub pinned: Vec<bool>,
    /// Maximum displacement per step (cools down).
    pub temperature: f32,
    names: Vec<String>,
}

/// Temperature below which the layout counts as settled.
pub const SETTLED: f32 = 0.002;

impl ForceLayout {
    /// Adapts the layout to `graph`: nodes that kept their name keep their
    /// position; new nodes are seeded on their trophic band. Returns whether
    /// anything changed (the layout then warms up again).
    pub fn sync(&mut self, graph: &RelationshipGraph) -> bool {
        let names: Vec<String> = graph.nodes.iter().map(|n| n.name.clone()).collect();
        if names == self.names && self.pos.len() == names.len() {
            return false;
        }
        let max_level = graph.nodes.iter().filter(|n| !n.resource).map(|n| n.trophic_level).fold(1.0, f64::max);
        let mut band_counts = std::collections::BTreeMap::<i64, usize>::new();
        let bands: Vec<i64> = graph
            .nodes
            .iter()
            .map(|n| (target_y(n.trophic_level, max_level, n.resource) * 100.0).round() as i64)
            .collect();
        for b in &bands {
            *band_counts.entry(*b).or_default() += 1;
        }
        let mut band_seen = std::collections::BTreeMap::<i64, usize>::new();
        let mut pos = Vec::with_capacity(names.len());
        let mut pinned = Vec::with_capacity(names.len());
        for (i, n) in graph.nodes.iter().enumerate() {
            let old = self.names.iter().position(|o| *o == n.name);
            let seen = band_seen.entry(bands[i]).or_default();
            let count = band_counts[&bands[i]];
            let seeded = [
                (*seen as f32 + 1.0) / (count as f32 + 1.0),
                target_y(n.trophic_level, max_level, n.resource) + 0.03 * ((*seen % 2) as f32 - 0.5),
            ];
            *seen += 1;
            pos.push(old.and_then(|o| self.pos.get(o).copied()).unwrap_or(seeded));
            pinned.push(old.and_then(|o| self.pinned.get(o).copied()).unwrap_or(false));
        }
        self.pos = pos;
        self.pinned = pinned;
        self.names = names;
        self.temperature = 0.08;
        true
    }

    /// Restarts the simulation (e.g. after unpinning).
    pub fn warm(&mut self) {
        self.temperature = self.temperature.max(0.05);
    }

    /// Whether the layout has cooled down.
    pub fn settled(&self) -> bool {
        self.temperature <= SETTLED
    }

    /// One Fruchterman–Reingold step with a vertical pull towards each
    /// node's trophic band and a weak pull towards the centre line. Returns
    /// the largest displacement.
    #[allow(clippy::needless_range_loop)]
    pub fn step(&mut self, graph: &RelationshipGraph) -> f32 {
        let n = self.pos.len().min(graph.nodes.len());
        if n == 0 {
            self.temperature = 0.0;
            return 0.0;
        }
        let k = (0.45 / (n as f32).sqrt()).clamp(0.12, 0.4);
        let max_level = graph.nodes.iter().filter(|x| !x.resource).map(|x| x.trophic_level).fold(1.0, f64::max);
        let mut disp = vec![[0.0f32; 2]; n];
        for i in 0..n {
            for j in (i + 1)..n {
                let mut d = [self.pos[i][0] - self.pos[j][0], self.pos[i][1] - self.pos[j][1]];
                let mut dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
                if dist < 1e-4 {
                    // Coincident nodes: separate them deterministically.
                    d = [((i * 7 + j * 3) % 5) as f32 * 0.01 + 0.01, 0.005];
                    dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
                }
                let f = k * k / dist;
                for a in 0..2 {
                    disp[i][a] += d[a] / dist * f;
                    disp[j][a] -= d[a] / dist * f;
                }
            }
        }
        let pairs: BTreeSet<(usize, usize)> = graph
            .edges
            .iter()
            .filter(|e| e.from != e.to && e.from < n && e.to < n)
            .map(|e| (e.from.min(e.to), e.from.max(e.to)))
            .collect();
        for (i, j) in pairs {
            let d = [self.pos[i][0] - self.pos[j][0], self.pos[i][1] - self.pos[j][1]];
            let dist = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-4);
            let f = dist * dist / k;
            for a in 0..2 {
                disp[i][a] -= d[a] / dist * f;
                disp[j][a] += d[a] / dist * f;
            }
        }
        let mut largest = 0.0f32;
        for i in 0..n {
            if self.pinned.get(i).copied().unwrap_or(false) {
                continue;
            }
            let node = &graph.nodes[i];
            disp[i][1] += (target_y(node.trophic_level, max_level, node.resource) - self.pos[i][1]) * 2.5;
            disp[i][0] += (0.5 - self.pos[i][0]) * 0.15;
            let len = (disp[i][0] * disp[i][0] + disp[i][1] * disp[i][1]).sqrt();
            if !len.is_finite() || len < 1e-9 {
                continue;
            }
            let moved = len.min(self.temperature);
            self.pos[i][0] = (self.pos[i][0] + disp[i][0] / len * moved).clamp(0.05, 0.95);
            self.pos[i][1] = (self.pos[i][1] + disp[i][1] / len * moved).clamp(0.06, 0.94);
            largest = largest.max(moved);
        }
        self.temperature = (self.temperature * 0.95).max(0.0);
        if self.temperature < SETTLED * 0.5 {
            self.temperature = 0.0;
        }
        largest
    }
}

/// What the user did in the graph this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphAction {
    /// A node was clicked (index into `graph.nodes`).
    pub node: Option<usize>,
    /// An edge was clicked (relationship index).
    pub relationship: Option<usize>,
}

fn quad_point(a: Pos2, c: Pos2, b: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    pos2(u * u * a.x + 2.0 * u * t * c.x + t * t * b.x, u * u * a.y + 2.0 * u * t * c.y + t * t * b.y)
}

fn distance_to_curve(p: Pos2, a: Pos2, c: Pos2, b: Pos2) -> f32 {
    (0..=24).map(|i| quad_point(a, c, b, i as f32 / 24.0).distance(p)).fold(f32::INFINITY, f32::min)
}

/// Draws the graph into a `height`-tall area of the available width and
/// advances the layout. `selected_edge` (relationship index) and
/// `selected_node` are highlighted; nodes can be dragged (pinned) and
/// double-clicked to release.
pub fn draw_graph(
    ui: &mut Ui,
    graph: &RelationshipGraph,
    layout: &mut ForceLayout,
    palette: &Palette,
    height: f32,
    selected_edge: Option<usize>,
    selected_node: Option<usize>,
) -> GraphAction {
    let mut action = GraphAction::default();
    if layout.sync(graph) {
        for _ in 0..60 {
            layout.step(graph);
        }
    }
    for _ in 0..4 {
        if layout.settled() {
            break;
        }
        layout.step(graph);
    }
    if !layout.settled() {
        ui.ctx().request_repaint();
    }
    let width = ui.available_width().max(200.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, mix(palette.panel, palette.background, 0.5));
    painter.rect_stroke(rect, 8.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
    if graph.nodes.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No entity types or resources yet",
            FontId::proportional(13.0),
            palette.text_muted,
        );
        return action;
    }
    let inner = rect.shrink2(vec2(36.0, 26.0));
    let to_screen = |p: [f32; 2]| pos2(inner.min.x + p[0] * inner.width(), inner.min.y + p[1] * inner.height());
    let node_radius = 17.0;
    let pointer = response.hover_pos();

    // Dragging a node pins it.
    let drag_id = ui.id().with("graph_drag_node");
    if response.drag_started()
        && let Some(p) = response.interact_pointer_pos()
    {
        let hit = layout.pos.iter().position(|q| to_screen(*q).distance(p) <= node_radius + 3.0);
        ui.data_mut(|d| d.insert_temp(drag_id, hit));
    }
    let dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id)).flatten();
    if let (Some(i), true) = (dragging, response.dragged())
        && let Some(p) = response.interact_pointer_pos()
        && i < layout.pos.len()
    {
        layout.pos[i] = [
            ((p.x - inner.min.x) / inner.width()).clamp(0.0, 1.0),
            ((p.y - inner.min.y) / inner.height()).clamp(0.0, 1.0),
        ];
        if let Some(pin) = layout.pinned.get_mut(i) {
            *pin = true;
        }
        layout.warm();
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<Option<usize>>(drag_id));
    }

    // Parallel edges between the same pair bend apart.
    let mut pair_count = std::collections::BTreeMap::<(usize, usize), usize>::new();
    let mut hovered_edge: Option<(usize, f32)> = None;
    let mut edge_shapes = Vec::new();
    for (ei, e) in graph.edges.iter().enumerate() {
        if e.from >= layout.pos.len() || e.to >= layout.pos.len() {
            continue;
        }
        let key = (e.from.min(e.to), e.from.max(e.to));
        let nth = *pair_count.entry(key).and_modify(|c| *c += 1).or_insert(0);
        let a = to_screen(layout.pos[e.from]);
        let b = to_screen(layout.pos[e.to]);
        let color = relation_color(&e.relation);
        let selected = selected_edge == Some(e.relationship);
        if e.from == e.to {
            // Self loop.
            let c = a + vec2(0.0, -node_radius - 12.0 - 8.0 * nth as f32);
            let r = 10.0 + 3.0 * nth as f32;
            if selected {
                edge_shapes.push((Shape::circle_stroke(c, r, Stroke::new(8.0, with_alpha(palette.accent, 0.35))), ei));
            }
            edge_shapes.push((Shape::circle_stroke(c, r, Stroke::new(if selected { 3.0 } else { 1.6 }, color)), ei));
            if let Some(p) = pointer
                && (p.distance(c) - 10.0).abs() < 5.0
            {
                hovered_edge = Some((ei, 0.0));
            }
            continue;
        }
        let dir = (b - a).normalized();
        let normal = vec2(-dir.y, dir.x);
        // The sign keeps opposite directions on opposite sides.
        let side = if e.from < e.to { 1.0 } else { -1.0 };
        let bend = if nth == 0 && !graph.edges.iter().any(|o| o.from == e.to && o.to == e.from && !o.symmetric) {
            0.0
        } else {
            22.0 * (1.0 + nth as f32) * side
        };
        let mid = a + (b - a) * 0.5 + normal * bend;
        let start = a + (mid - a).normalized() * node_radius;
        let end = b + (mid - b).normalized() * (node_radius + 2.0);
        let width = if selected { 3.2 } else { 1.8 };
        let points: Vec<Pos2> = (0..=20).map(|i| quad_point(start, mid, end, i as f32 / 20.0)).collect();
        if selected {
            // A soft halo in the accent colour marks the edited relationship.
            edge_shapes.push((Shape::line(points.clone(), Stroke::new(9.0, with_alpha(palette.accent, 0.35))), ei));
        }
        edge_shapes.push((Shape::line(points, Stroke::new(width, color)), ei));
        if !e.symmetric {
            let tip = end;
            let back = (quad_point(start, mid, end, 0.93) - tip).normalized();
            let n = vec2(-back.y, back.x);
            let base = tip + back * 10.0;
            edge_shapes
                .push((Shape::convex_polygon(vec![tip, base + n * 5.0, base - n * 5.0], color, Stroke::NONE), ei));
        }
        if let Some(p) = pointer {
            let d = distance_to_curve(p, start, mid, end);
            if d < 6.0 && hovered_edge.is_none_or(|(_, best)| d < best) {
                hovered_edge = Some((ei, d));
            }
        }
    }
    for (shape, _) in edge_shapes {
        painter.add(shape);
    }

    // Nodes (labels are placed after all nodes so they can avoid them).
    let node_boxes: Vec<Rect> = (0..graph.nodes.len())
        .filter_map(|i| {
            layout.pos.get(i).map(|q| Rect::from_center_size(to_screen(*q), Vec2::splat(node_radius * 2.0 + 4.0)))
        })
        .collect();
    let mut placed: Vec<Rect> = Vec::new();
    let mut hovered_node = None;
    for (i, node) in graph.nodes.iter().enumerate() {
        let Some(p) = layout.pos.get(i).map(|q| to_screen(*q)) else { continue };
        let fill = color32_from_rgba(node.color);
        let selected = selected_node == Some(i);
        let hovered = pointer.is_some_and(|q| q.distance(p) <= node_radius);
        if hovered {
            hovered_node = Some(i);
        }
        let ring = if selected { palette.accent } else { mix(fill, Color32::BLACK, 0.45) };
        if node.resource {
            let r = Rect::from_center_size(p, Vec2::splat(node_radius * 1.7));
            painter.rect_filled(r, 6.0, with_alpha(fill, 0.85));
            painter.rect_stroke(r, 6.0, Stroke::new(if selected { 2.5 } else { 1.5 }, ring), egui::StrokeKind::Outside);
        } else {
            if selected || hovered {
                painter.circle_filled(p, node_radius + 5.0, with_alpha(palette.accent, 0.18));
            }
            painter.circle_filled(p, node_radius, fill);
            painter.circle_stroke(p, node_radius, Stroke::new(if selected { 2.5 } else { 1.5 }, ring));
            let glyph = crate::glyphs::shown_icon(ui.ctx(), &node.icon, &node.name);
            if !glyph.is_empty() {
                painter.text(
                    p,
                    Align2::CENTER_CENTER,
                    glyph,
                    FontId::proportional(15.0),
                    interface::theme::on_color(fill),
                );
            }
        }
        let label = if node.resource { format!("{} (resource)", node.name) } else { node.name.clone() };
        let galley = painter.layout_no_wrap(label, FontId::proportional(12.0), palette.text);
        let size = galley.size() + vec2(8.0, 2.0);
        let obstacles: Vec<Rect> = node_boxes
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != i)
            .map(|(_, r)| *r)
            .chain(placed.iter().copied())
            .collect();
        let bg = place_label(p, node_radius, size, rect, layout.pos.get(i).map_or(0.5, |q| q[0]), &obstacles);
        placed.push(bg);
        painter.rect_filled(bg, 4.0, with_alpha(palette.panel, 0.8));
        painter.galley(bg.min + vec2(4.0, 1.0), galley, palette.text);
    }

    if let Some(i) = hovered_node {
        let node = &graph.nodes[i];
        let text = if node.resource {
            format!("{} — resource field", node.name)
        } else {
            format!("{} — trophic level {:.2}", node.name, node.trophic_level)
        };
        response.clone().on_hover_text_at_pointer(text);
    } else if let Some((ei, _)) = hovered_edge {
        let e = &graph.edges[ei];
        let from = &graph.nodes[e.from].name;
        let to = &graph.nodes[e.to].name;
        let mut text = format!("{from} {} {to}\n{}", e.relation, e.summary);
        if !e.rules.is_empty() {
            text.push_str(&format!("\nGenerated rules: {}", e.rules.join(", ")));
        }
        response.clone().on_hover_text_at_pointer(text);
    }
    if response.clicked() {
        action.node = hovered_node;
        if hovered_node.is_none() {
            action.relationship = hovered_edge.map(|(ei, _)| graph.edges[ei].relationship);
        }
    }
    if response.double_clicked() {
        layout.pinned.iter_mut().for_each(|p| *p = false);
        layout.warm();
    }
    action
}

/// Where a node's label goes: to the right of the node (left of it in the
/// right half), vertically centred, kept inside `area`.
pub fn label_rect(node: Pos2, radius: f32, size: Vec2, area: Rect, x_fraction: f32) -> Rect {
    let gap = radius + 6.0;
    let left = if x_fraction <= 0.6 { node.x + gap } else { node.x - gap - size.x };
    let left = left.clamp(area.min.x + 2.0, (area.max.x - size.x - 2.0).max(area.min.x + 2.0));
    let top = (node.y - size.y / 2.0).clamp(area.min.y + 2.0, (area.max.y - size.y - 2.0).max(area.min.y + 2.0));
    Rect::from_min_size(pos2(left, top), size)
}

/// A label position that avoids `obstacles` (other nodes, labels placed
/// before): beside the node on its preferred side ([`label_rect`]), then
/// the other side, below, above; the candidate overlapping least wins.
pub fn place_label(node: Pos2, radius: f32, size: Vec2, area: Rect, x_fraction: f32, obstacles: &[Rect]) -> Rect {
    let clamp = |r: Rect| {
        let left = r.min.x.clamp(area.min.x + 2.0, (area.max.x - size.x - 2.0).max(area.min.x + 2.0));
        let top = r.min.y.clamp(area.min.y + 2.0, (area.max.y - size.y - 2.0).max(area.min.y + 2.0));
        Rect::from_min_size(pos2(left, top), size)
    };
    let preferred = label_rect(node, radius, size, area, x_fraction);
    let other = label_rect(node, radius, size, area, if x_fraction <= 0.6 { 1.0 } else { 0.0 });
    let below = clamp(Rect::from_min_size(pos2(node.x - size.x / 2.0, node.y + radius + 3.0), size));
    let above = clamp(Rect::from_min_size(pos2(node.x - size.x / 2.0, node.y - radius - 3.0 - size.y), size));
    let overlap = |r: &Rect| -> f32 {
        obstacles
            .iter()
            .map(|o| {
                let i = r.intersect(*o);
                if i.is_positive() { i.area() } else { 0.0 }
            })
            .sum()
    };
    let mut best = preferred;
    let mut best_overlap = overlap(&preferred);
    for candidate in [other, below, above] {
        if best_overlap <= 0.0 {
            break;
        }
        let o = overlap(&candidate);
        if o < best_overlap {
            best = candidate;
            best_overlap = o;
        }
    }
    best
}

/// Legend of the relation kinds present in `graph`.
pub fn graph_legend(ui: &mut Ui, graph: &RelationshipGraph) {
    let present: BTreeSet<&str> = graph.edges.iter().map(|e| e.relation.as_str()).collect();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        for label in crate::colors::RELATION_LABELS.iter().filter(|l| present.contains(*l)) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 10.0), Sense::hover());
                let c = relation_color(label);
                ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(2.0, c));
                if *label != "competes_with" {
                    let tip = r.right_center();
                    ui.painter().add(Shape::convex_polygon(
                        vec![tip, tip + vec2(-5.0, -3.5), tip + vec2(-5.0, 3.5)],
                        c,
                        Stroke::NONE,
                    ));
                }
                ui.label(egui::RichText::new(label.replace('_', " ")).small());
            });
        }
        if present.is_empty() {
            ui.label(egui::RichText::new("No relationships yet").small().weak());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::{compile, scenarios};

    #[test]
    fn local_graph_matches_the_compiled_one() {
        for s in scenarios::builtin().unwrap() {
            let compiled = compile(&s.system).unwrap();
            let mine = graph_of(&s.system);
            let theirs = compiled.graph();
            assert_eq!(mine.nodes, theirs.nodes, "{}", s.id);
            assert_eq!(mine.edges.len(), theirs.edges.len());
            for (a, b) in mine.edges.iter().zip(&theirs.edges) {
                assert_eq!((a.from, a.to, &a.relation, a.relationship), (b.from, b.to, &b.relation, b.relationship));
            }
        }
    }

    #[test]
    fn layout_converges_without_overlaps_and_respects_trophic_order() {
        let s = scenarios::builtin_by_id("producers-consumers").unwrap();
        let graph = compile(&s.system).unwrap().graph().clone();
        let mut layout = ForceLayout::default();
        assert!(layout.sync(&graph));
        assert!(!layout.sync(&graph), "same nodes: no reseed");
        let mut steps = 0;
        while !layout.settled() && steps < 2000 {
            layout.step(&graph);
            steps += 1;
        }
        assert!(layout.settled(), "cools down");
        assert!(layout.pos.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        for i in 0..layout.pos.len() {
            for j in (i + 1)..layout.pos.len() {
                let d = ((layout.pos[i][0] - layout.pos[j][0]).powi(2) + (layout.pos[i][1] - layout.pos[j][1]).powi(2))
                    .sqrt();
                assert!(d > 0.08, "nodes {i} and {j} overlap: {d}");
            }
        }
        let y = |name: &str| layout.pos[graph.nodes.iter().position(|n| n.name == name).unwrap()][1];
        assert!(y("hunter") < y("grazer"), "hunters above grazers");
        assert!(y("grazer") < y("plant"), "grazers above plants");
        assert!(y("plant") < y("sunlight"), "resources at the bottom");
    }

    #[test]
    fn renamed_nodes_keep_the_others_in_place() {
        let s = scenarios::builtin_by_id("producers-consumers").unwrap();
        let graph = graph_of(&s.system);
        let mut layout = ForceLayout::default();
        layout.sync(&graph);
        for _ in 0..50 {
            layout.step(&graph);
        }
        let before = layout.pos.clone();
        let mut sys = s.system.clone();
        sys.entity_types[2].name = "wolf".into();
        let graph2 = graph_of(&sys);
        assert!(layout.sync(&graph2));
        assert_eq!(layout.pos[0], before[0]);
        assert_eq!(layout.pos[1], before[1]);
        assert_eq!(layout.pos.len(), before.len());
    }

    #[test]
    fn empty_graph_is_harmless() {
        let mut layout = ForceLayout::default();
        let empty = RelationshipGraph::default();
        layout.sync(&empty);
        assert_eq!(layout.step(&empty), 0.0);
        assert!(layout.settled());
        assert!(target_y(3.0, 3.0, false) < target_y(1.0, 3.0, false));
        assert_eq!(target_y(1.0, 1.0, true), 0.9);
    }

    #[test]
    fn labels_sit_beside_nodes_inside_the_area() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 200.0));
        let r = label_rect(pos2(100.0, 100.0), 17.0, vec2(60.0, 16.0), area, 0.3);
        assert_eq!(r.min, pos2(123.0, 92.0), "right of a node in the left part");
        let r = label_rect(pos2(250.0, 100.0), 17.0, vec2(60.0, 16.0), area, 0.8);
        assert_eq!(r.max.x, 250.0 - 23.0, "left of a node in the right part");
        let r = label_rect(pos2(290.0, 199.0), 17.0, vec2(60.0, 16.0), area, 0.2);
        assert!(area.contains_rect(r));
        // A neighbour on the right pushes the label to the other side.
        let neighbour = Rect::from_center_size(pos2(160.0, 100.0), vec2(38.0, 38.0));
        let r = place_label(pos2(100.0, 100.0), 17.0, vec2(60.0, 16.0), area, 0.3, &[neighbour]);
        assert!(!r.intersects(neighbour) && r.max.x <= 100.0, "{r:?}");
        let free = place_label(pos2(100.0, 100.0), 17.0, vec2(60.0, 16.0), area, 0.3, &[]);
        assert_eq!(free, label_rect(pos2(100.0, 100.0), 17.0, vec2(60.0, 16.0), area, 0.3));
    }
}
