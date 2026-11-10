//! Genealogy tree of the living population ("lineage through time").
//!
//! The model keeps a pruned genealogy: living organisms and every ancestor
//! that still has living descendants. The tree follows **first parents**
//! (the line the model uses for lineages/founders); second parents of
//! sexual offspring are drawn as faint links. Time runs left to right: each
//! organism is a horizontal life line from birth to death (or to "now"),
//! offspring branch off at their birth tick. Any subtree can be collapsed
//! into a wedge showing how many living descendants it holds.
//!
//! Layout is linear in the number of nodes: parents always have smaller ids
//! than their children (enforced by the organism engine), so a reverse pass
//! over ids aggregates subtrees and a forward pass assigns rows.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy_egui::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use evolution::Species;
use interface::Palette;
use interface::theme::with_alpha;
use organism_engine::AgentId;

use crate::session::{GenealogyData, GenealogyNodeData};

/// One drawn node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeNode {
    /// The organism.
    pub id: AgentId,
    /// Vertical position (row units).
    pub row: f32,
    /// Birth tick.
    pub birth: u64,
    /// Death tick, or the current tick while alive.
    pub end: u64,
    /// Alive now.
    pub alive: bool,
    /// Index of the first parent in [`TreeLayout::nodes`].
    pub parent: Option<usize>,
    /// Collapsed: number of living descendants hidden in the wedge.
    pub collapsed: Option<u32>,
    /// Index of the root (founder line) the node belongs to.
    pub root: usize,
    /// Species (of the organism or of its living descendants).
    pub species: Option<Species>,
    /// Generation.
    pub generation: u32,
    /// Has visible children.
    pub internal: bool,
}

/// Laid-out tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeLayout {
    /// Visible nodes in id order.
    pub nodes: Vec<TreeNode>,
    /// Second-parent links `(parent index, child index)` between visible nodes.
    pub second_links: Vec<(usize, usize)>,
    /// Rows used.
    pub rows: usize,
    /// Earliest birth tick.
    pub first_tick: u64,
    /// Current tick.
    pub now: u64,
    /// Node index by id.
    pub index: BTreeMap<AgentId, usize>,
    /// Living organisms drawn (including those inside collapsed wedges).
    pub living: u32,
}

/// Lays out the genealogy. `filter` keeps one species; `collapsed` lists
/// nodes whose subtree is folded.
pub fn layout(
    nodes: &[GenealogyNodeData],
    living: &BTreeMap<AgentId, Species>,
    filter: Option<Species>,
    collapsed: &BTreeSet<AgentId>,
    now: u64,
) -> TreeLayout {
    let n = nodes.len();
    let mut by_id: BTreeMap<AgentId, usize> = BTreeMap::new();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| nodes[i].id);
    for &i in &order {
        by_id.insert(nodes[i].id, i);
    }
    // First parent within the recorded set (a parent always has a smaller id).
    let parent: Vec<Option<usize>> = (0..n)
        .map(|i| nodes[i].parents[0].and_then(|p| by_id.get(&p).copied()).filter(|&p| nodes[p].id < nodes[i].id))
        .collect();

    // Reverse pass: living descendants and species per node.
    let mut living_below = vec![0u32; n];
    let mut species: Vec<Option<Species>> = vec![None; n];
    for &i in order.iter().rev() {
        if let Some(s) = living.get(&nodes[i].id) {
            living_below[i] += 1;
            species[i] = Some(*s);
        }
        if let Some(p) = parent[i] {
            living_below[p] += living_below[i];
            if species[p].is_none() {
                species[p] = species[i];
            }
        }
    }

    // Forward pass: visibility and roots.
    let mut visible = vec![false; n];
    let mut root_of = vec![0usize; n];
    for &i in &order {
        match parent[i] {
            None => {
                visible[i] = filter.is_none() || species[i] == filter;
                root_of[i] = i;
            }
            Some(p) => {
                visible[i] = visible[p] && !collapsed.contains(&nodes[p].id);
                root_of[i] = root_of[p];
            }
        }
    }
    let shows_children = |i: usize| !collapsed.contains(&nodes[i].id);

    // Reverse pass: rows spanned by each subtree.
    let mut span = vec![0usize; n];
    let mut has_children = vec![false; n];
    for &i in order.iter().rev() {
        if !visible[i] {
            continue;
        }
        if span[i] == 0 {
            span[i] = 1;
        }
        if let Some(p) = parent[i]
            && visible[p]
            && shows_children(p)
        {
            if !has_children[p] {
                has_children[p] = true;
                span[p] = 0;
            }
            span[p] += span[i];
        }
    }

    // Forward pass: first row of each subtree.
    let mut start = vec![0usize; n];
    let mut cursor = vec![0usize; n];
    let mut rows = 0usize;
    for &i in &order {
        if !visible[i] {
            continue;
        }
        match parent[i] {
            Some(p) if visible[p] => {
                start[i] = cursor[p];
                cursor[p] += span[i];
            }
            _ => {
                start[i] = rows;
                rows += span[i];
            }
        }
        cursor[i] = start[i];
    }

    // Reverse pass: row of each node (leaves at their start, parents centred
    // between their first and last child).
    let mut row = vec![0.0f32; n];
    let mut first_child: Vec<Option<f32>> = vec![None; n];
    let mut last_child: Vec<Option<f32>> = vec![None; n];
    for &i in order.iter().rev() {
        if !visible[i] {
            continue;
        }
        row[i] = match (first_child[i], last_child[i]) {
            (Some(a), Some(b)) if has_children[i] => 0.5 * (a + b),
            _ => start[i] as f32,
        };
        if let Some(p) = parent[i]
            && visible[p]
        {
            if last_child[p].is_none() {
                last_child[p] = Some(row[i]);
            }
            first_child[p] = Some(row[i]);
        }
    }

    let mut out = TreeLayout { rows, now, ..TreeLayout::default() };
    let mut index_of = vec![usize::MAX; n];
    for &i in &order {
        if !visible[i] {
            continue;
        }
        let node = &nodes[i];
        let alive = living.contains_key(&node.id);
        index_of[i] = out.nodes.len();
        out.index.insert(node.id, out.nodes.len());
        out.nodes.push(TreeNode {
            id: node.id,
            row: row[i],
            birth: node.birth_tick,
            end: if alive { now } else { node.death_tick.unwrap_or(now) },
            alive,
            parent: parent[i].filter(|&p| visible[p]).map(|p| index_of[p]),
            collapsed: (collapsed.contains(&node.id) && living_below[i] > 0).then_some(living_below[i]),
            root: root_of[i],
            species: species[i],
            generation: node.generation,
            internal: has_children[i],
        });
        if parent[i].is_none() {
            out.living += living_below[i];
        }
    }
    for &i in &order {
        if !visible[i] {
            continue;
        }
        if let Some(second) = nodes[i].parents[1].filter(|s| Some(*s) != nodes[i].parents[0])
            && let Some(&s) = by_id.get(&second)
            && visible[s]
        {
            out.second_links.push((index_of[s], index_of[i]));
        }
    }
    out.first_tick = out.nodes.iter().map(|n| n.birth).min().unwrap_or(0);
    out
}

/// The last laid-out tree, reused while its genealogy, species filter and
/// folded nodes are unchanged (the layout of 20 000 nodes is too costly to
/// redo every frame).
#[derive(Default)]
pub struct TreeCache {
    source: Option<Arc<GenealogyData>>,
    filter: Option<Species>,
    collapsed: BTreeSet<AgentId>,
    layout: TreeLayout,
}

impl TreeCache {
    /// Layout of `data` with the given filter and folds (recomputed only
    /// when one of them changed).
    pub fn layout(
        &mut self,
        data: &Arc<GenealogyData>,
        filter: Option<Species>,
        collapsed: &BTreeSet<AgentId>,
    ) -> &TreeLayout {
        let fresh = self.source.as_ref().is_some_and(|s| Arc::ptr_eq(s, data))
            && self.filter == filter
            && self.collapsed == *collapsed;
        if !fresh {
            self.layout = layout(&data.nodes, &data.living, filter, collapsed, data.tick);
            self.source = Some(data.clone());
            self.filter = filter;
            self.collapsed.clone_from(collapsed);
        }
        &self.layout
    }

    /// Whether the cached layout was computed from `data`.
    pub fn is_from(&self, data: &Arc<GenealogyData>) -> bool {
        self.source.as_ref().is_some_and(|s| Arc::ptr_eq(s, data))
    }
}

/// First-parent chain of `id` within the layout (the node itself first).
pub fn ancestry(layout: &TreeLayout, id: AgentId) -> Vec<usize> {
    let mut out = Vec::new();
    let mut next = layout.index.get(&id).copied();
    while let Some(i) = next {
        out.push(i);
        next = layout.nodes[i].parent;
        if out.len() > layout.nodes.len() {
            break;
        }
    }
    out
}

/// Categorical colour of a founder line.
pub fn lineage_color(root: usize) -> Color32 {
    // Golden-angle hues, fixed saturation/lightness readable on both themes.
    let h = (root as f32 * 0.618_034).fract();
    let (r, g, b) = hsl_to_rgb(h, 0.55, 0.58);
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r + m, g + m, b + m)
}

/// Pan/zoom state of the tree canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeView {
    /// Collapsed nodes.
    pub collapsed: BTreeSet<AgentId>,
    /// Horizontal zoom (1 = fit).
    pub zoom_x: f32,
    /// Vertical zoom (1 = fit).
    pub zoom_y: f32,
    /// Pan offset in pixels.
    pub pan: Vec2,
    /// Species shown.
    pub filter: Option<Species>,
}

impl Default for TreeView {
    fn default() -> Self {
        Self { collapsed: BTreeSet::new(), zoom_x: 1.0, zoom_y: 1.0, pan: Vec2::ZERO, filter: Some(Species::Prey) }
    }
}

impl TreeView {
    /// Back to the fitted view.
    pub fn fit(&mut self) {
        self.zoom_x = 1.0;
        self.zoom_y = 1.0;
        self.pan = Vec2::ZERO;
    }
}

/// What the user did in the tree this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeAction {
    /// Select this (living) organism.
    Select(AgentId),
}

/// Maps ticks and rows to canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreeTransform {
    origin: Pos2,
    first_tick: f64,
    px_per_tick: f32,
    px_per_row: f32,
}

impl TreeTransform {
    /// Transform fitting `layout` into `rect` with the view's zoom and pan.
    pub fn new(layout: &TreeLayout, rect: Rect, view: &TreeView) -> Self {
        let ticks = (layout.now.saturating_sub(layout.first_tick)).max(1) as f32;
        let rows = layout.rows.max(1) as f32;
        let inner = rect.shrink2(vec2(10.0, 8.0));
        Self {
            origin: inner.min + view.pan,
            first_tick: layout.first_tick as f64,
            px_per_tick: inner.width() / ticks * view.zoom_x,
            px_per_row: (inner.height() / rows).min(14.0) * view.zoom_y,
        }
    }

    /// Canvas position of `tick` on `row`.
    pub fn point(&self, tick: u64, row: f32) -> Pos2 {
        pos2(
            self.origin.x + ((tick as f64 - self.first_tick) as f32) * self.px_per_tick,
            self.origin.y + (row + 0.5) * self.px_per_row,
        )
    }

    /// Tick under an x coordinate.
    pub fn tick_at(&self, x: f32) -> f64 {
        self.first_tick + f64::from((x - self.origin.x) / self.px_per_tick.max(1e-6))
    }
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}

/// Draws the tree into a canvas of `height` pixels and handles pan, zoom,
/// hover, selection and collapsing.
pub fn tree_canvas(
    ui: &mut egui::Ui,
    layout: &TreeLayout,
    view: &mut TreeView,
    selected: Option<AgentId>,
    palette: &Palette,
    height: f32,
) -> Option<TreeAction> {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, palette.surface);
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, palette.stroke), egui::StrokeKind::Inside);
    if layout.nodes.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No genealogy yet",
            egui::FontId::proportional(13.0),
            palette.text_muted,
        );
        return None;
    }
    if response.dragged() {
        view.pan += response.drag_delta();
    }
    if response.hovered() {
        let zoom = ui.input(|i| i.zoom_delta());
        if (zoom - 1.0).abs() > 1e-4
            && let Some(pointer) = response.hover_pos()
        {
            let before = TreeTransform::new(layout, rect, view);
            let anchor = pointer - before.origin;
            view.zoom_x = (view.zoom_x * zoom).clamp(0.5, 200.0);
            view.zoom_y = (view.zoom_y * zoom).clamp(0.5, 200.0);
            let after = TreeTransform::new(layout, rect, view);
            // Keep the point under the cursor in place.
            let sx = after.px_per_tick / before.px_per_tick.max(1e-6);
            let sy = after.px_per_row / before.px_per_row.max(1e-6);
            view.pan += anchor - vec2(anchor.x * sx, anchor.y * sy);
        }
    }
    let tf = TreeTransform::new(layout, rect, view);
    let ancestry_path: BTreeSet<usize> =
        selected.map(|id| ancestry(layout, id).into_iter().collect()).unwrap_or_default();
    let line_w = (tf.px_per_row * 0.35).clamp(0.6, 2.2);
    let visible = rect.expand(4.0);

    // Second-parent links first (behind).
    for &(s, c) in &layout.second_links {
        let (sn, cn) = (&layout.nodes[s], &layout.nodes[c]);
        let a = tf.point(cn.birth, sn.row);
        let b = tf.point(cn.birth, cn.row);
        if visible.intersects(Rect::from_two_pos(a, b)) {
            painter.line_segment([a, b], Stroke::new(0.6, with_alpha(palette.text_muted, 0.25)));
        }
    }
    let mut hovered: Option<(usize, f32)> = None;
    let pointer = response.hover_pos();
    for (i, node) in layout.nodes.iter().enumerate() {
        let start = tf.point(node.birth, node.row);
        let end = tf.point(node.end, node.row);
        let mut bounds = Rect::from_two_pos(start, end);
        if let Some(p) = node.parent {
            bounds = bounds.union(Rect::from_two_pos(start, tf.point(node.birth, layout.nodes[p].row)));
        }
        if !visible.intersects(bounds.expand(2.0)) {
            continue;
        }
        let on_path = ancestry_path.contains(&i);
        let base = lineage_color(node.root);
        let color = if on_path {
            palette.accent
        } else if node.alive {
            base
        } else {
            with_alpha(base, 0.7)
        };
        let width = if on_path { line_w + 1.2 } else { line_w };
        if let Some(p) = node.parent {
            let from = tf.point(node.birth, layout.nodes[p].row);
            painter.line_segment([from, start], Stroke::new(width * 0.8, color));
        }
        painter.line_segment([start, end], Stroke::new(width, color));
        if let Some(count) = node.collapsed {
            let h = (tf.px_per_row * 0.9).clamp(4.0, 12.0);
            let tip = end + vec2(14.0, 0.0);
            painter.add(egui::Shape::convex_polygon(
                vec![end, tip + vec2(0.0, -h), tip + vec2(0.0, h)],
                with_alpha(color, 0.45),
                Stroke::new(1.0, color),
            ));
            painter.text(
                tip + vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{count}"),
                egui::FontId::proportional(10.0),
                palette.text_muted,
            );
        } else if node.alive && !node.internal {
            painter.circle_filled(end, (line_w + 0.8).min(3.0), color);
        }
        if let Some(p) = pointer {
            let d = distance_to_segment(p, start, end);
            if d < 5.0 && hovered.is_none_or(|(_, best)| d < best) {
                hovered = Some((i, d));
            }
        }
    }
    // "Now" line.
    let now_x = tf.point(layout.now, 0.0).x;
    if (rect.left()..=rect.right()).contains(&now_x) {
        painter.line_segment(
            [pos2(now_x, rect.top() + 4.0), pos2(now_x, rect.bottom() - 4.0)],
            Stroke::new(1.0, with_alpha(palette.text_muted, 0.6)),
        );
    }
    // Tick axis labels.
    let ticks = nice_ticks(tf.tick_at(rect.left()), tf.tick_at(rect.right()), 5);
    for t in ticks {
        let x = tf.point(t.max(0.0) as u64, 0.0).x;
        painter.text(
            pos2(x, rect.bottom() - 3.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{t:.0}"),
            egui::FontId::proportional(9.5),
            palette.text_muted,
        );
    }

    let mut action = None;
    if let Some((i, _)) = hovered {
        let node = layout.nodes[i];
        let start = tf.point(node.birth, node.row);
        let end = tf.point(node.end, node.row);
        painter.line_segment([start, end], Stroke::new(line_w + 2.0, with_alpha(palette.text, 0.35)));
        let species = node.species.map_or("?", Species::label);
        let state = if node.alive { "alive".to_string() } else { format!("died at tick {}", node.end) };
        let mut text =
            format!("{} {species} · generation {}\nborn at tick {} · {state}", node.id, node.generation, node.birth);
        if let Some(c) = node.collapsed {
            text.push_str(&format!("\n{c} living descendants (collapsed)"));
        }
        text.push_str(if node.alive {
            "\nclick: inspect · right-click: fold/unfold"
        } else {
            "\nclick or right-click: fold/unfold"
        });
        let response = response.clone().on_hover_text_at_pointer(text);
        if response.clicked() {
            if node.alive {
                action = Some(TreeAction::Select(node.id));
            } else if node.internal || node.collapsed.is_some() {
                toggle(&mut view.collapsed, node.id);
            }
        }
        if response.secondary_clicked() && (node.internal || node.collapsed.is_some()) {
            toggle(&mut view.collapsed, node.id);
        }
    }
    action
}

fn toggle(set: &mut BTreeSet<AgentId>, id: AgentId) {
    if !set.remove(&id) {
        set.insert(id);
    }
}

/// About `count` round axis values covering `[lo, hi]`.
pub fn nice_ticks(lo: f64, hi: f64, count: usize) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite()) || hi <= lo || count == 0 {
        return Vec::new();
    }
    let raw = (hi - lo) / count as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
    let mut t = (lo / step).ceil() * step;
    let mut out = Vec::new();
    while t <= hi + 1e-9 && out.len() < 50 {
        if t >= 0.0 {
            // Avoid printing "-0".
            out.push(if t.abs() < step * 1e-9 { 0.0 } else { t });
        }
        t += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, parent: Option<u64>, birth: u64, death: Option<u64>) -> GenealogyNodeData {
        GenealogyNodeData {
            id: AgentId(id),
            parents: [parent.map(AgentId), None],
            generation: 0,
            birth_tick: birth,
            death_tick: death,
        }
    }

    /// 0 -+- 2 -+- 4 (alive)
    ///    |     `- 5 (alive)
    ///    `- 3 (alive)
    /// 1 --- 6 (alive)
    fn sample() -> (Vec<GenealogyNodeData>, BTreeMap<AgentId, Species>) {
        let nodes = vec![
            node(0, None, 0, Some(50)),
            node(1, None, 0, Some(40)),
            node(2, Some(0), 10, Some(60)),
            node(3, Some(0), 20, None),
            node(4, Some(2), 30, None),
            node(5, Some(2), 35, None),
            node(6, Some(1), 25, None),
        ];
        let living = [3, 4, 5, 6].into_iter().map(|i| (AgentId(i), Species::Prey)).collect();
        (nodes, living)
    }

    #[test]
    fn leaves_take_rows_and_parents_centre_on_children() {
        let (nodes, living) = sample();
        let l = layout(&nodes, &living, None, &BTreeSet::new(), 100);
        assert_eq!(l.rows, 4);
        assert_eq!(l.living, 4);
        let row = |id: u64| l.nodes[l.index[&AgentId(id)]].row;
        assert_eq!((row(4), row(5), row(3), row(6)), (0.0, 1.0, 2.0, 3.0));
        assert_eq!(row(2), 0.5);
        assert_eq!(row(0), 1.25);
        assert_eq!(row(1), 3.0);
        let n4 = l.nodes[l.index[&AgentId(4)]];
        assert!(n4.alive && n4.end == 100 && !n4.internal);
        assert_eq!(l.nodes[n4.parent.unwrap()].id, AgentId(2));
        assert_eq!(l.nodes[l.index[&AgentId(0)]].end, 50, "dead nodes end at death");
        assert_eq!(l.first_tick, 0);
        let roots: BTreeSet<usize> = l.nodes.iter().map(|n| n.root).collect();
        assert_eq!(roots.len(), 2, "two founder lines");
    }

    #[test]
    fn collapsing_folds_subtrees_into_counted_wedges() {
        let (nodes, living) = sample();
        let collapsed: BTreeSet<AgentId> = [AgentId(2)].into();
        let l = layout(&nodes, &living, None, &collapsed, 100);
        assert_eq!(l.rows, 3);
        assert!(!l.index.contains_key(&AgentId(4)), "children of a folded node are hidden");
        let n2 = l.nodes[l.index[&AgentId(2)]];
        assert_eq!(n2.collapsed, Some(2));
        assert!(!n2.internal);
        assert_eq!(l.living, 4, "living count includes folded descendants");
    }

    #[test]
    fn species_filter_and_second_parents() {
        let (mut nodes, mut living) = sample();
        living.insert(AgentId(6), Species::Predator);
        nodes[5].parents[1] = Some(AgentId(3));
        let l = layout(&nodes, &living, Some(Species::Prey), &BTreeSet::new(), 100);
        assert!(!l.index.contains_key(&AgentId(1)) && !l.index.contains_key(&AgentId(6)));
        assert_eq!(l.nodes[l.index[&AgentId(0)]].species, Some(Species::Prey), "species inherited from descendants");
        assert_eq!(l.second_links.len(), 1);
        let (s, c) = l.second_links[0];
        assert_eq!((l.nodes[s].id, l.nodes[c].id), (AgentId(3), AgentId(5)));
        let path: Vec<AgentId> = ancestry(&l, AgentId(5)).into_iter().map(|i| l.nodes[i].id).collect();
        assert_eq!(path, vec![AgentId(5), AgentId(2), AgentId(0)]);
    }

    #[test]
    fn missing_parents_make_roots_and_deep_chains_do_not_recurse() {
        let mut nodes = vec![node(10, Some(3), 5, None)];
        for i in 11..5000 {
            nodes.push(node(i, Some(i - 1), i, if i < 4999 { Some(i + 1) } else { None }));
        }
        let living: BTreeMap<AgentId, Species> = [(AgentId(10), Species::Prey), (AgentId(4999), Species::Prey)].into();
        let l = layout(&nodes, &living, None, &BTreeSet::new(), 6000);
        assert_eq!(l.rows, 1);
        assert_eq!(l.nodes.len(), 4990);
        assert!(l.nodes[0].parent.is_none(), "a pruned parent makes a root");
    }

    #[test]
    fn tree_cache_recomputes_only_on_change() {
        let (nodes, living) = sample();
        let data = Arc::new(GenealogyData { tick: 100, nodes, living, ..GenealogyData::default() });
        let mut cache = TreeCache::default();
        let none = BTreeSet::new();
        let first = cache.layout(&data, None, &none).clone();
        assert_eq!(first.rows, 4);
        assert!(cache.is_from(&data));
        let folded: BTreeSet<AgentId> = [AgentId(2)].into();
        assert_eq!(cache.layout(&data, None, &folded).rows, 3, "a fold is a new layout");
        assert_eq!(cache.layout(&data, None, &none), &first);
        let copy = Arc::new((*data).clone());
        assert!(!cache.is_from(&copy), "a new reading is a new layout");
        assert_eq!(cache.layout(&copy, Some(Species::Predator), &none).nodes.len(), 0);
    }

    #[test]
    fn transform_and_ticks() {
        let (nodes, living) = sample();
        let l = layout(&nodes, &living, None, &BTreeSet::new(), 100);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(220.0, 100.0));
        let tf = TreeTransform::new(&l, rect, &TreeView::default());
        let a = tf.point(0, 0.0);
        let b = tf.point(100, 0.0);
        assert!((a.x - 10.0).abs() < 1e-4 && (b.x - 210.0).abs() < 1e-3);
        assert!((tf.tick_at(110.0) - 50.0).abs() < 1e-3);
        assert_eq!(nice_ticks(0.0, 100.0, 5), vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
        assert!(nice_ticks(5.0, 5.0, 3).is_empty());
        assert_ne!(lineage_color(0), lineage_color(1));
    }
}
